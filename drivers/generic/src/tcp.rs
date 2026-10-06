//! Line-oriented TCP text-protocol driver.
//!
//! Many AV devices (projectors, matrices, DSPs) speak short ASCII commands
//! over TCP: send `POWR 1`, optionally read an acknowledgement; send `POWR?`,
//! read `POWR=ON`. This driver does exactly that, configured as data.
//!
//! Behaviour worth knowing:
//!
//! * **One connection per operation.** `get_state` opens a connection, asks
//!   every query on it, and closes it; `execute` does the same for one
//!   command. This is stateless and robust to devices that drop idle
//!   connections, at the cost of a reconnect per operation.
//! * **Bounded and time-limited.** Targets must be explicit unicast
//!   addresses; connect, write and every read share the configured timeout;
//!   a received line may not exceed [`MAX_LINE_BYTES`]; blank lines and
//!   other chatter are skipped only up to a fixed count.
//! * **No command injection.** Values substituted into a command template
//!   (`$input`, `$pattern`, …) must be printable, single-line and short. A
//!   value carrying a line break would otherwise smuggle a second command to
//!   the device, so it is rejected before anything is sent.
//! * **Typed replies.** A reply is trimmed, an optional prefix is stripped,
//!   and a built-in parser turns it into a typed value. A reply that does not
//!   fit is a `MalformedResponse`, never a guess.

use std::collections::BTreeMap;
use std::io::{Read, Write};
use std::net::{IpAddr, SocketAddr, TcpStream};
use std::time::{Duration, Instant};

use tpt_app_av_commissioning_device::{
    DeviceCapabilities, DeviceCommand, DeviceIdentity, DeviceResponse, DeviceState,
};
use tpt_app_av_commissioning_driver::net::{check_target, check_timeout, map_io_error};
use tpt_app_av_commissioning_driver::{DeviceDriver, DriverError};
use tpt_app_av_commissioning_profile::{
    placeholders_in, substitute, DeviceProfile, ParserKind, Placeholder, ProtocolKind, Terminator,
};

/// Longest line accepted from a device.
pub const MAX_LINE_BYTES: usize = 4096;
/// Blank lines skipped while waiting for one reply.
const MAX_BLANK_LINES: usize = 16;
/// Longest substituted value, and longest arbitrary command.
const MAX_VALUE_LEN: usize = 128;
const MAX_ARBITRARY_LEN: usize = 256;

/// A command's text and its optional acknowledgement.
#[derive(Debug, Clone, PartialEq)]
pub struct TextCommand {
    /// Command text, possibly with `$placeholders`.
    pub send: String,
    /// Reply line that acknowledges it; `None` means fire-and-forget.
    pub ack: Option<String>,
}

/// A state field read by sending `query` and parsing the reply.
#[derive(Debug, Clone, PartialEq)]
pub struct TextQuery {
    pub field: String,
    pub query: String,
    pub parser: ParserKind,
    pub strip_prefix: Option<String>,
}

/// Everything the TCP driver needs to know about one device.
#[derive(Debug, Clone)]
pub struct TcpDriverConfig {
    pub target: SocketAddr,
    pub timeout: Duration,
    pub terminator: Terminator,
    pub identity: DeviceIdentity,
    /// Keyed by command kind (`power_on`, `set_input`, …).
    pub commands: BTreeMap<String, TextCommand>,
    pub queries: Vec<TextQuery>,
}

impl TcpDriverConfig {
    /// A config for `target`: 1 s timeout, CRLF lines, nothing bound.
    pub fn new(target: SocketAddr) -> Self {
        Self {
            target,
            timeout: Duration::from_secs(1),
            terminator: Terminator::Crlf,
            identity: DeviceIdentity::new(),
            commands: BTreeMap::new(),
            queries: Vec::new(),
        }
    }

    pub fn timeout(mut self, timeout: Duration) -> Self {
        self.timeout = timeout;
        self
    }

    pub fn terminator(mut self, terminator: Terminator) -> Self {
        self.terminator = terminator;
        self
    }

    pub fn identity(mut self, identity: DeviceIdentity) -> Self {
        self.identity = identity;
        self
    }

    pub fn bind(mut self, command_kind: impl Into<String>, command: TextCommand) -> Self {
        self.commands.insert(command_kind.into(), command);
        self
    }

    pub fn query(mut self, query: TextQuery) -> Self {
        self.queries.push(query);
        self
    }

    /// Build a config from a `tcp` device profile, for the device at `host`.
    pub fn from_profile(profile: &DeviceProfile, host: IpAddr) -> Result<Self, DriverError> {
        let spec = &profile.device;
        if spec.protocol.kind != ProtocolKind::Tcp {
            return Err(DriverError::Config(format!(
                "profile `{}` is a {:?} profile; the TCP driver needs `type: tcp`",
                spec.id, spec.protocol.kind
            )));
        }
        let port = spec.protocol.port.ok_or_else(|| {
            DriverError::Config(format!("profile `{}` has no `protocol.port`", spec.id))
        })?;
        let mut config = Self::new(SocketAddr::new(host, port))
            .timeout(Duration::from_millis(spec.protocol.timeout_ms_or_default()))
            .terminator(spec.protocol.terminator_or_default())
            .identity(DeviceIdentity {
                manufacturer: Some(spec.matcher.manufacturer.clone()),
                model: spec.matcher.model.as_slice().first().cloned(),
                ..DeviceIdentity::default()
            });
        for (name, cmd) in &spec.commands {
            if !cmd.args.is_empty() {
                return Err(DriverError::Config(format!(
                    "commands.{name}: text protocols embed placeholders in `send`; `args` is for OSC"
                )));
            }
            config = config.bind(
                name.clone(),
                TextCommand {
                    send: cmd.send.clone(),
                    ack: cmd.ack.clone(),
                },
            );
        }
        for (field, q) in &spec.state {
            config = config.query(TextQuery {
                field: field.clone(),
                query: q.query.clone(),
                parser: q.parser.unwrap_or_default(),
                strip_prefix: q.strip_prefix.clone(),
            });
        }
        config.validate()?;
        Ok(config)
    }

    /// Check the config is safe and coherent. [`TcpDriver::new`] calls this.
    pub fn validate(&self) -> Result<(), DriverError> {
        check_target(self.target)?;
        check_timeout(self.timeout)?;
        let printable = |what: &str, text: &str| {
            if text.trim().is_empty()
                || text.len() > MAX_ARBITRARY_LEN
                || text.chars().any(char::is_control)
            {
                Err(DriverError::Config(format!(
                    "{what} must be 1-{MAX_ARBITRARY_LEN} printable characters"
                )))
            } else {
                Ok(())
            }
        };
        for (name, cmd) in &self.commands {
            printable(&format!("commands.{name}.send"), &cmd.send)?;
            if let Some(ack) = &cmd.ack {
                printable(&format!("commands.{name}.ack"), ack)?;
            }
        }
        for q in &self.queries {
            printable(&format!("state.{}.query", q.field), &q.query)?;
            if !placeholders_in(&q.query).is_empty() {
                return Err(DriverError::Config(format!(
                    "state.{}.query may not contain placeholders",
                    q.field
                )));
            }
        }
        Ok(())
    }

    fn capabilities(&self) -> DeviceCapabilities {
        let has = |k: &str| self.commands.contains_key(k);
        let has_field = |f: &str| self.queries.iter().any(|q| q.field == f);
        DeviceCapabilities {
            can_power_on: has("power_on"),
            can_power_off: has("power_off"),
            can_read_state: !self.queries.is_empty(),
            can_select_input: has("set_input"),
            can_generate_test_pattern: has("generate_test_pattern"),
            can_read_signal_status: has_field("signal_present") || has_field("signal_lock"),
            can_read_edid: has("read_edid"),
            can_measure_latency: has("measure_latency"),
        }
    }
}

/// A text-protocol device reachable over TCP.
pub struct TcpDriver {
    config: TcpDriverConfig,
    capabilities: DeviceCapabilities,
}

impl TcpDriver {
    /// Validate `config`. No connection is made until an operation runs.
    pub fn new(config: TcpDriverConfig) -> Result<Self, DriverError> {
        config.validate()?;
        let capabilities = config.capabilities();
        Ok(Self {
            config,
            capabilities,
        })
    }

    pub fn config(&self) -> &TcpDriverConfig {
        &self.config
    }

    fn connect(&self) -> Result<Connection<'_>, DriverError> {
        let cfg = &self.config;
        let stream = TcpStream::connect_timeout(&cfg.target, cfg.timeout)
            .map_err(|e| map_io_error(&e, cfg.target, cfg.timeout))?;
        stream
            .set_write_timeout(Some(cfg.timeout))
            .map_err(|e| map_io_error(&e, cfg.target, cfg.timeout))?;
        // Small request/response lines: do not wait to coalesce them.
        let _ = stream.set_nodelay(true);
        Ok(Connection {
            stream,
            buffer: Vec::new(),
            config: cfg,
        })
    }

    fn read_state(&self) -> Result<DeviceState, DriverError> {
        if self.config.queries.is_empty() {
            return Err(DriverError::UnsupportedOperation);
        }
        let mut conn = self.connect()?;
        let mut state = DeviceState::new();
        for q in &self.config.queries {
            conn.send_line(&q.query)?;
            let mut reply = conn.read_line()?;
            if let Some(prefix) = &q.strip_prefix {
                reply = reply
                    .strip_prefix(prefix.as_str())
                    .ok_or_else(|| {
                        DriverError::MalformedResponse(format!(
                            "{}: reply {reply:?} does not start with {prefix:?}",
                            q.field
                        ))
                    })?
                    .to_owned();
            }
            let value = q
                .parser
                .parse(&reply)
                .map_err(|e| DriverError::MalformedResponse(format!("{}: {e}", q.field)))?;
            state.set(q.field.clone(), value);
        }
        Ok(state)
    }

    fn send_command(&self, text: &str, ack: Option<&str>) -> Result<DeviceResponse, DriverError> {
        let started = Instant::now();
        let mut conn = self.connect()?;
        conn.send_line(text)?;
        let (ok, message) = match ack {
            None => (
                true,
                format!("sent {text:?}; no acknowledgement expected, verify by reading state"),
            ),
            Some(expected) => {
                let reply = conn.read_line()?;
                if reply.trim().eq_ignore_ascii_case(expected.trim()) {
                    (true, format!("sent {text:?}; acknowledged"))
                } else {
                    (
                        false,
                        format!(
                            "sent {text:?}; expected acknowledgement {expected:?}, got {reply:?}"
                        ),
                    )
                }
            }
        };
        Ok(DeviceResponse {
            ok,
            state: None,
            message: Some(message),
            response_time_ms: Some(started.elapsed().as_millis() as u64),
        })
    }
}

/// One open connection plus its receive buffer.
struct Connection<'a> {
    stream: TcpStream,
    buffer: Vec<u8>,
    config: &'a TcpDriverConfig,
}

impl Connection<'_> {
    fn io_error(&self, e: &std::io::Error) -> DriverError {
        map_io_error(e, self.config.target, self.config.timeout)
    }

    fn send_line(&mut self, text: &str) -> Result<(), DriverError> {
        let mut bytes = Vec::with_capacity(text.len() + 2);
        bytes.extend_from_slice(text.as_bytes());
        bytes.extend_from_slice(self.config.terminator.bytes());
        self.stream.write_all(&bytes).map_err(|e| self.io_error(&e))
    }

    /// Read one non-blank line, within the configured timeout in total.
    fn read_line(&mut self) -> Result<String, DriverError> {
        let deadline = Instant::now() + self.config.timeout;
        let end = self.config.terminator.end_byte();
        let mut chunk = [0u8; 512];
        let mut blanks = 0;
        loop {
            if let Some(pos) = self.buffer.iter().position(|b| *b == end) {
                let raw: Vec<u8> = self.buffer.drain(..=pos).collect();
                let line = String::from_utf8(raw).map_err(|_| {
                    DriverError::MalformedResponse("reply is not valid UTF-8".to_owned())
                })?;
                let line = line.trim_matches(|c| c == '\r' || c == '\n').trim();
                if !line.is_empty() {
                    return Ok(line.to_owned());
                }
                blanks += 1;
                if blanks > MAX_BLANK_LINES {
                    return Err(DriverError::Protocol(
                        "too many blank lines while waiting for a reply".to_owned(),
                    ));
                }
                continue;
            }
            if self.buffer.len() > MAX_LINE_BYTES {
                return Err(DriverError::MalformedResponse(format!(
                    "reply line exceeds {MAX_LINE_BYTES} bytes"
                )));
            }
            let remaining = deadline.saturating_duration_since(Instant::now());
            if remaining.is_zero() {
                return Err(DriverError::Timeout(self.config.timeout.as_millis() as u64));
            }
            self.stream
                .set_read_timeout(Some(remaining))
                .map_err(|e| self.io_error(&e))?;
            match self.stream.read(&mut chunk) {
                Ok(0) => {
                    return Err(if self.buffer.is_empty() {
                        DriverError::Unreachable(format!(
                            "{}: connection closed before a reply",
                            self.config.target
                        ))
                    } else {
                        DriverError::MalformedResponse(
                            "connection closed in the middle of a line".to_owned(),
                        )
                    })
                }
                Ok(n) => self.buffer.extend_from_slice(&chunk[..n]),
                Err(e) => return Err(self.io_error(&e)),
            }
        }
    }
}

fn kind_name(command: &DeviceCommand) -> &'static str {
    match command {
        DeviceCommand::PowerOn => "power_on",
        DeviceCommand::PowerOff => "power_off",
        DeviceCommand::PowerCycle => "power_cycle",
        DeviceCommand::SetInput { .. } => "set_input",
        DeviceCommand::Freeze { .. } => "freeze",
        DeviceCommand::GenerateTestPattern { .. } => "generate_test_pattern",
        DeviceCommand::SetAudioVolume { .. } => "set_audio_volume",
        DeviceCommand::SetAudioMute { .. } => "set_audio_mute",
        DeviceCommand::SetRoute { .. } => "set_route",
        DeviceCommand::ReadState => "read_state",
        DeviceCommand::ReadEdid => "read_edid",
        DeviceCommand::MeasureLatency => "measure_latency",
        DeviceCommand::Arbitrary { .. } => "arbitrary",
    }
}

/// A substituted value must not be able to end the line or add a command.
fn safe_value(what: &str, value: &str) -> Result<String, String> {
    if value.is_empty() || value.len() > MAX_VALUE_LEN || value.chars().any(char::is_control) {
        return Err(format!(
            "{what} must be 1-{MAX_VALUE_LEN} printable characters on one line"
        ));
    }
    Ok(value.to_owned())
}

/// Fill a command template from the command being executed.
fn render(template: &str, command: &DeviceCommand) -> Result<String, DriverError> {
    substitute(template, |ph| match (ph, command) {
        (Placeholder::Input, DeviceCommand::SetInput { input }) => safe_value("input", input),
        (Placeholder::Pattern, DeviceCommand::GenerateTestPattern { pattern }) => {
            safe_value("pattern", pattern)
        }
        (Placeholder::Source, DeviceCommand::SetRoute { source, .. }) => {
            safe_value("source", source)
        }
        (Placeholder::Destination, DeviceCommand::SetRoute { destination, .. }) => {
            safe_value("destination", destination)
        }
        (Placeholder::LevelDb, DeviceCommand::SetAudioVolume { level_db }) => {
            if level_db.is_finite() {
                Ok(level_db.to_string())
            } else {
                Err("level_db must be a finite number".to_owned())
            }
        }
        (Placeholder::Muted, DeviceCommand::SetAudioMute { muted }) => {
            Ok(if *muted { "1" } else { "0" }.to_owned())
        }
        (Placeholder::Frozen, DeviceCommand::Freeze { frozen }) => {
            Ok(if *frozen { "1" } else { "0" }.to_owned())
        }
        (ph, _) => Err(format!(
            "{ph:?} placeholder does not apply to {}",
            kind_name(command)
        )),
    })
    .map_err(DriverError::Config)
}

impl DeviceDriver for TcpDriver {
    fn identity(&self) -> DeviceIdentity {
        self.config.identity.clone()
    }

    /// Discovery is a state read: a device that answers its queries is there.
    fn discover(&mut self) -> Result<DeviceState, DriverError> {
        if self.config.queries.is_empty() {
            return Err(DriverError::Config(
                "no state queries configured; cannot verify the device is present".to_owned(),
            ));
        }
        self.read_state()
    }

    fn get_state(&mut self) -> Result<DeviceState, DriverError> {
        self.read_state()
    }

    fn execute(&mut self, command: DeviceCommand) -> Result<DeviceResponse, DriverError> {
        match &command {
            DeviceCommand::ReadState => {
                let state = self.read_state()?;
                Ok(DeviceResponse {
                    ok: true,
                    state: Some(state),
                    message: None,
                    response_time_ms: None,
                })
            }
            DeviceCommand::Arbitrary { command: text } => {
                if text.trim().is_empty()
                    || text.len() > MAX_ARBITRARY_LEN
                    || text.chars().any(char::is_control)
                {
                    return Err(DriverError::Config(format!(
                        "arbitrary command must be 1-{MAX_ARBITRARY_LEN} printable characters on one line"
                    )));
                }
                self.send_command(text, None)
            }
            other => {
                let bound = self
                    .config
                    .commands
                    .get(kind_name(other))
                    .ok_or(DriverError::UnsupportedOperation)?;
                let text = render(&bound.send, other)?;
                self.send_command(&text, bound.ack.as_deref())
            }
        }
    }

    fn capabilities(&self) -> DeviceCapabilities {
        self.capabilities
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cfg() -> TcpDriverConfig {
        TcpDriverConfig::new("10.0.0.9:4352".parse().unwrap())
    }

    #[test]
    fn rejects_unsafe_config() {
        for bad in ["0.0.0.0:1", "224.0.0.1:1", "10.0.0.1:0"] {
            assert!(TcpDriverConfig::new(bad.parse().unwrap())
                .validate()
                .is_err());
        }
        assert!(cfg().timeout(Duration::ZERO).validate().is_err());
        let bad = cfg().bind(
            "power_on",
            TextCommand {
                send: "POWR 1\r\nPOWR 0".into(),
                ack: None,
            },
        );
        assert!(bad.validate().is_err());
        let ph_query = cfg().query(TextQuery {
            field: "x".into(),
            query: "X? $input".into(),
            parser: ParserKind::Auto,
            strip_prefix: None,
        });
        assert!(ph_query.validate().is_err());
    }

    #[test]
    fn renders_templates_and_refuses_injection() {
        let set = |i: &str| DeviceCommand::SetInput { input: i.into() };
        assert_eq!(render("INPT $input", &set("hdmi2")).unwrap(), "INPT hdmi2");
        for evil in [
            "hdmi1\r\nPOWR 0",
            "a\nb",
            "",
            &"x".repeat(MAX_VALUE_LEN + 1),
        ] {
            assert!(render("INPT $input", &set(evil)).is_err(), "{evil:?}");
        }
        assert_eq!(
            render(
                "VOL $level_db",
                &DeviceCommand::SetAudioVolume { level_db: -6.5 }
            )
            .unwrap(),
            "VOL -6.5"
        );
        assert!(render(
            "VOL $level_db",
            &DeviceCommand::SetAudioVolume { level_db: f64::NAN }
        )
        .is_err());
        assert_eq!(
            render("MUTE $muted", &DeviceCommand::SetAudioMute { muted: true }).unwrap(),
            "MUTE 1"
        );
        // A placeholder from another command never resolves.
        assert!(render("INPT $input", &DeviceCommand::PowerOn).is_err());
    }
}
