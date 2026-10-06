//! WebSocket text-protocol driver.
//!
//! Same command/query model as TCP ([`crate::text`]); each text message is one
//! command, query or reply. JSON replies are the norm, so queries usually use
//! `extract` (a JSON Pointer) to pick the value.
//!
//! A connection is opened per operation. Plain `ws://` only: there is no TLS
//! yet, so this is for devices on the commissioning LAN. Incoming messages are
//! capped at [`MAX_LINE_BYTES`], binary messages are rejected, and every read
//! (including the handshake) is bounded by the timeout.

use std::net::{IpAddr, SocketAddr, TcpStream};
use std::time::{Duration, Instant};

use tungstenite::client::IntoClientRequest;
use tungstenite::protocol::WebSocketConfig;
use tungstenite::{Message, WebSocket};

use tpt_app_av_commissioning_device::DeviceIdentity;
use tpt_app_av_commissioning_driver::net::{check_target, check_timeout, map_io_error};
use tpt_app_av_commissioning_driver::DriverError;
use tpt_app_av_commissioning_profile::{DeviceProfile, ProtocolKind, Terminator};

use crate::text::{
    text_config_builders, Exchange, Opener, TextCommand, TextDriver, TextQuery, TextSpec,
    MAX_BLANK_LINES, MAX_LINE_BYTES,
};

/// Default WebSocket port.
pub const DEFAULT_PORT: u16 = 80;

/// Everything the WebSocket driver needs to know about one device.
#[derive(Debug, Clone)]
pub struct WebSocketDriverConfig {
    pub target: SocketAddr,
    /// Request path, e.g. `/ws`.
    pub path: String,
    pub timeout: Duration,
    /// Unused (messages are self-delimiting); present so the shared builders
    /// apply.
    pub terminator: Terminator,
    pub spec: TextSpec,
}

impl WebSocketDriverConfig {
    /// A config for `target` at path `/`: 1 s timeout, nothing bound.
    pub fn new(target: SocketAddr) -> Self {
        Self {
            target,
            path: "/".to_owned(),
            timeout: Duration::from_secs(1),
            terminator: Terminator::None,
            spec: TextSpec::default(),
        }
    }

    pub fn path(mut self, path: impl Into<String>) -> Self {
        self.path = path.into();
        self
    }

    text_config_builders!();

    /// Build a config from a `websocket` device profile, for the device at
    /// `host`.
    pub fn from_profile(profile: &DeviceProfile, host: IpAddr) -> Result<Self, DriverError> {
        let p = &profile.device;
        if p.protocol.kind != ProtocolKind::Websocket {
            return Err(DriverError::Config(format!(
                "profile `{}` is a {:?} profile; the WebSocket driver needs `type: websocket`",
                p.id, p.protocol.kind
            )));
        }
        let config = Self {
            target: SocketAddr::new(host, p.protocol.port.unwrap_or(DEFAULT_PORT)),
            path: p.protocol.path.clone().unwrap_or_else(|| "/".to_owned()),
            timeout: Duration::from_millis(p.protocol.timeout_ms_or_default()),
            terminator: Terminator::None,
            spec: TextSpec::from_profile(profile)?,
        };
        config.validate()?;
        Ok(config)
    }

    /// Check the config is safe and coherent. [`WebSocketDriver::new`] calls
    /// this.
    pub fn validate(&self) -> Result<(), DriverError> {
        check_target(self.target)?;
        check_timeout(self.timeout)?;
        if !self.path.starts_with('/')
            || self.path.len() > 256
            || self.path.chars().any(|c| c.is_control() || c == ' ')
        {
            return Err(DriverError::Config(
                "path must start with '/' and contain no spaces".to_owned(),
            ));
        }
        self.spec.validate()
    }
}

/// Opens a WebSocket connection per operation.
pub struct WebSocketOpener {
    target: SocketAddr,
    path: String,
    timeout: Duration,
}

fn map_ws_error(e: tungstenite::Error, target: SocketAddr, timeout: Duration) -> DriverError {
    match e {
        tungstenite::Error::Io(io) => map_io_error(&io, target, timeout),
        tungstenite::Error::ConnectionClosed | tungstenite::Error::AlreadyClosed => {
            DriverError::Unreachable(format!("{target}: connection closed"))
        }
        tungstenite::Error::Http(resp) => DriverError::Refused(format!(
            "{target}: WebSocket handshake rejected with HTTP {}",
            resp.status()
        )),
        tungstenite::Error::Capacity(_) => DriverError::MalformedResponse(format!(
            "{target}: message exceeds {MAX_LINE_BYTES} bytes"
        )),
        other => DriverError::Protocol(format!("{target}: {other}")),
    }
}

impl Opener for WebSocketOpener {
    fn open(&self) -> Result<Box<dyn Exchange + '_>, DriverError> {
        let io = |e: &std::io::Error| map_io_error(e, self.target, self.timeout);
        let stream = TcpStream::connect_timeout(&self.target, self.timeout).map_err(|e| io(&e))?;
        stream
            .set_write_timeout(Some(self.timeout))
            .map_err(|e| io(&e))?;
        // The handshake reads too: bound it.
        stream
            .set_read_timeout(Some(self.timeout))
            .map_err(|e| io(&e))?;
        let _ = stream.set_nodelay(true);
        let request = format!("ws://{}{}", self.target, self.path)
            .into_client_request()
            .map_err(|e| DriverError::Config(format!("bad WebSocket request: {e}")))?;
        let config = WebSocketConfig::default()
            .max_message_size(Some(MAX_LINE_BYTES))
            .max_frame_size(Some(MAX_LINE_BYTES));
        let (socket, _response) =
            tungstenite::client::client_with_config(request, stream, Some(config)).map_err(
                |e| match e {
                    tungstenite::HandshakeError::Failure(err) => {
                        map_ws_error(err, self.target, self.timeout)
                    }
                    tungstenite::HandshakeError::Interrupted(_) => {
                        DriverError::Timeout(self.timeout.as_millis() as u64)
                    }
                },
            )?;
        Ok(Box::new(WsExchange {
            socket,
            target: self.target,
            timeout: self.timeout,
        }))
    }
}

struct WsExchange {
    socket: WebSocket<TcpStream>,
    target: SocketAddr,
    timeout: Duration,
}

impl Exchange for WsExchange {
    fn send_line(&mut self, text: &str) -> Result<(), DriverError> {
        self.socket
            .send(Message::text(text))
            .map_err(|e| map_ws_error(e, self.target, self.timeout))
    }

    fn read_line(&mut self) -> Result<String, DriverError> {
        let deadline = Instant::now() + self.timeout;
        for _ in 0..=MAX_BLANK_LINES {
            let remaining = deadline.saturating_duration_since(Instant::now());
            if remaining.is_zero() {
                return Err(DriverError::Timeout(self.timeout.as_millis() as u64));
            }
            self.socket
                .get_mut()
                .set_read_timeout(Some(remaining))
                .map_err(|e| map_io_error(&e, self.target, self.timeout))?;
            match self.socket.read() {
                Ok(Message::Text(t)) => {
                    let text = t.as_str().trim();
                    if !text.is_empty() {
                        return Ok(text.to_owned());
                    }
                }
                Ok(Message::Binary(_)) => {
                    return Err(DriverError::MalformedResponse(
                        "binary message from a text device".to_owned(),
                    ))
                }
                Ok(Message::Close(_)) => {
                    return Err(DriverError::Unreachable(format!(
                        "{}: device closed the connection",
                        self.target
                    )))
                }
                // Control frames are answered by the library; keep waiting.
                Ok(_) => {}
                Err(e) => return Err(map_ws_error(e, self.target, self.timeout)),
            }
        }
        Err(DriverError::Protocol(
            "too many empty messages while waiting for a reply".to_owned(),
        ))
    }
}

impl Drop for WsExchange {
    fn drop(&mut self) {
        // Say goodbye politely; the device may already be gone.
        let _ = self.socket.close(None);
        let _ = self.socket.flush();
    }
}

/// A text-protocol device reachable over WebSocket.
pub type WebSocketDriver = TextDriver<WebSocketOpener>;

impl TextDriver<WebSocketOpener> {
    /// Validate `config`. No connection is made until an operation runs.
    pub fn new(config: WebSocketDriverConfig) -> Result<Self, DriverError> {
        config.validate()?;
        TextDriver::with_opener(
            WebSocketOpener {
                target: config.target,
                path: config.path,
                timeout: config.timeout,
            },
            config.spec,
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejects_unsafe_config() {
        let c = WebSocketDriverConfig::new("10.0.0.9:80".parse().unwrap());
        assert!(c.clone().validate().is_ok());
        assert!(c.clone().path("ws").validate().is_err());
        assert!(c.clone().path("/a b").validate().is_err());
        assert!(c.clone().timeout(Duration::ZERO).validate().is_err());
        assert!(WebSocketDriverConfig::new("0.0.0.0:80".parse().unwrap())
            .validate()
            .is_err());
        assert!(c
            .bind("power_on", TextCommand::new("a\nb"))
            .validate()
            .is_err());
        let _ = (TextQuery::new("a", "b"), DeviceIdentity::default());
    }
}
