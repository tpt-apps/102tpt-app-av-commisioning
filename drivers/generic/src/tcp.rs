//! Line-oriented TCP text-protocol driver.
//!
//! Many AV devices (projectors, matrices, DSPs) speak short ASCII commands
//! over TCP: send `POWR 1`, optionally read an acknowledgement; send `POWR?`,
//! read `POWR=ON`. The shared text core ([`crate::text`]) does the work; this
//! module supplies the connection.
//!
//! One connection per operation: `get_state` connects, asks every query on
//! that connection and closes it; `execute` does the same for one command.
//! This is stateless and tolerant of devices that drop idle connections, at
//! the cost of a reconnect per operation. Targets must be explicit unicast
//! addresses, and connect, write and every read share the configured timeout.

use std::net::{IpAddr, SocketAddr, TcpStream};
use std::time::Duration;

use tpt_app_av_commissioning_device::DeviceIdentity;
use tpt_app_av_commissioning_driver::net::{check_target, check_timeout, map_io_error};
use tpt_app_av_commissioning_driver::DriverError;
use tpt_app_av_commissioning_profile::{DeviceProfile, ProtocolKind, Terminator};

use crate::text::{
    text_config_builders, Exchange, Opener, StreamExchange, TextCommand, TextDriver, TextQuery,
    TextSpec,
};

/// Everything the TCP driver needs to know about one device.
#[derive(Debug, Clone)]
pub struct TcpDriverConfig {
    pub target: SocketAddr,
    pub timeout: Duration,
    pub terminator: Terminator,
    pub spec: TextSpec,
}

impl TcpDriverConfig {
    /// A config for `target`: 1 s timeout, CRLF lines, nothing bound.
    pub fn new(target: SocketAddr) -> Self {
        Self {
            target,
            timeout: Duration::from_secs(1),
            terminator: Terminator::Crlf,
            spec: TextSpec::default(),
        }
    }

    text_config_builders!();

    /// Build a config from a `tcp` device profile, for the device at `host`.
    pub fn from_profile(profile: &DeviceProfile, host: IpAddr) -> Result<Self, DriverError> {
        let p = &profile.device;
        if p.protocol.kind != ProtocolKind::Tcp {
            return Err(DriverError::Config(format!(
                "profile `{}` is a {:?} profile; the TCP driver needs `type: tcp`",
                p.id, p.protocol.kind
            )));
        }
        let port = p.protocol.port.ok_or_else(|| {
            DriverError::Config(format!("profile `{}` has no `protocol.port`", p.id))
        })?;
        let config = Self {
            target: SocketAddr::new(host, port),
            timeout: Duration::from_millis(p.protocol.timeout_ms_or_default()),
            terminator: p.protocol.terminator_or_default(),
            spec: TextSpec::from_profile(profile)?,
        };
        config.validate()?;
        Ok(config)
    }

    /// Check the config is safe and coherent. [`TcpDriver::new`] calls this.
    pub fn validate(&self) -> Result<(), DriverError> {
        check_target(self.target)?;
        check_timeout(self.timeout)?;
        self.spec.validate()
    }
}

/// Opens a TCP connection per operation.
pub struct TcpOpener {
    target: SocketAddr,
    timeout: Duration,
    terminator: Terminator,
}

impl Opener for TcpOpener {
    fn open(&self) -> Result<Box<dyn Exchange + '_>, DriverError> {
        let io = |e: &std::io::Error| map_io_error(e, self.target, self.timeout);
        let stream = TcpStream::connect_timeout(&self.target, self.timeout).map_err(|e| io(&e))?;
        stream
            .set_write_timeout(Some(self.timeout))
            .map_err(|e| io(&e))?;
        // Small request/response lines: do not wait to coalesce them.
        let _ = stream.set_nodelay(true);
        Ok(Box::new(StreamExchange::new(
            stream,
            self.terminator,
            self.timeout,
            self.target.to_string(),
        )))
    }
}

/// A text-protocol device reachable over TCP.
pub type TcpDriver = TextDriver<TcpOpener>;

impl TextDriver<TcpOpener> {
    /// Validate `config`. No connection is made until an operation runs.
    pub fn new(config: TcpDriverConfig) -> Result<Self, DriverError> {
        config.validate()?;
        TextDriver::with_opener(
            TcpOpener {
                target: config.target,
                timeout: config.timeout,
                terminator: config.terminator,
            },
            config.spec,
        )
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
        let bad = cfg().bind("power_on", TextCommand::new("POWR 1\r\nPOWR 0"));
        assert!(bad.validate().is_err());
        let ph_query = cfg().query(TextQuery::new("x", "X? $input"));
        assert!(ph_query.validate().is_err());
    }

    #[test]
    fn identity_is_carried_through() {
        let c = cfg().identity(DeviceIdentity {
            model: Some("M".into()),
            ..DeviceIdentity::default()
        });
        assert_eq!(c.spec.identity.model.as_deref(), Some("M"));
    }
}
