//! UDP text-protocol driver: one message per datagram.
//!
//! Same command/query model as TCP ([`crate::text`]), but each message is a
//! single datagram and each reply a single datagram. The socket is connected
//! to the device, so datagrams from anywhere else are ignored by the OS and a
//! closed port surfaces as an error on the next call. UDP gives no delivery
//! guarantee: a lost datagram is a timeout, not a retry.

use std::net::{IpAddr, SocketAddr, UdpSocket};
use std::time::Duration;

use tpt_app_av_commissioning_device::DeviceIdentity;
use tpt_app_av_commissioning_driver::net::{check_target, check_timeout, map_io_error};
use tpt_app_av_commissioning_driver::DriverError;
use tpt_app_av_commissioning_profile::{DeviceProfile, ProtocolKind, Terminator};

use crate::text::{
    text_config_builders, DatagramExchange, Exchange, Opener, TextCommand, TextDriver, TextQuery,
    TextSpec,
};

/// Everything the UDP driver needs to know about one device.
#[derive(Debug, Clone)]
pub struct UdpDriverConfig {
    pub target: SocketAddr,
    pub timeout: Duration,
    /// Appended to every sent message (`none` for pure datagram protocols).
    pub terminator: Terminator,
    pub spec: TextSpec,
}

impl UdpDriverConfig {
    /// A config for `target`: 1 s timeout, no terminator, nothing bound.
    pub fn new(target: SocketAddr) -> Self {
        Self {
            target,
            timeout: Duration::from_secs(1),
            terminator: Terminator::None,
            spec: TextSpec::default(),
        }
    }

    text_config_builders!();

    /// Build a config from a `udp` device profile, for the device at `host`.
    pub fn from_profile(profile: &DeviceProfile, host: IpAddr) -> Result<Self, DriverError> {
        let p = &profile.device;
        if p.protocol.kind != ProtocolKind::Udp {
            return Err(DriverError::Config(format!(
                "profile `{}` is a {:?} profile; the UDP driver needs `type: udp`",
                p.id, p.protocol.kind
            )));
        }
        let port = p.protocol.port.ok_or_else(|| {
            DriverError::Config(format!("profile `{}` has no `protocol.port`", p.id))
        })?;
        let config = Self {
            target: SocketAddr::new(host, port),
            timeout: Duration::from_millis(p.protocol.timeout_ms_or_default()),
            // Datagrams are self-delimiting: only add a terminator if asked.
            terminator: p.protocol.terminator.unwrap_or(Terminator::None),
            spec: TextSpec::from_profile(profile)?,
        };
        config.validate()?;
        Ok(config)
    }

    /// Check the config is safe and coherent. [`UdpDriver::new`] calls this.
    pub fn validate(&self) -> Result<(), DriverError> {
        check_target(self.target)?;
        check_timeout(self.timeout)?;
        self.spec.validate()
    }
}

/// Opens a connected UDP socket per operation.
pub struct UdpOpener {
    target: SocketAddr,
    timeout: Duration,
    terminator: Terminator,
}

impl Opener for UdpOpener {
    fn open(&self) -> Result<Box<dyn Exchange + '_>, DriverError> {
        let io = |e: &std::io::Error| map_io_error(e, self.target, self.timeout);
        let local: SocketAddr = if self.target.is_ipv4() {
            ([0, 0, 0, 0], 0).into()
        } else {
            (std::net::Ipv6Addr::UNSPECIFIED, 0).into()
        };
        let socket = UdpSocket::bind(local).map_err(|e| io(&e))?;
        socket.connect(self.target).map_err(|e| io(&e))?;
        Ok(Box::new(DatagramExchange::new(
            socket,
            self.terminator,
            self.timeout,
            self.target.to_string(),
        )))
    }
}

/// A text-protocol device reachable over UDP.
pub type UdpDriver = TextDriver<UdpOpener>;

impl TextDriver<UdpOpener> {
    /// Validate `config`. No socket is opened until an operation runs.
    pub fn new(config: UdpDriverConfig) -> Result<Self, DriverError> {
        config.validate()?;
        TextDriver::with_opener(
            UdpOpener {
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

    #[test]
    fn rejects_unsafe_config() {
        for bad in [
            "0.0.0.0:1",
            "224.0.0.1:1",
            "255.255.255.255:1",
            "10.0.0.1:0",
        ] {
            assert!(UdpDriverConfig::new(bad.parse().unwrap())
                .validate()
                .is_err());
        }
        let c = UdpDriverConfig::new("10.0.0.9:7000".parse().unwrap());
        assert!(c.clone().timeout(Duration::ZERO).validate().is_err());
        assert!(c
            .clone()
            .bind("power_on", TextCommand::new("a\nb"))
            .validate()
            .is_err());
        assert!(c
            .query(TextQuery::new("x", "X? $input"))
            .validate()
            .is_err());
    }

    #[test]
    fn identity_is_carried_through() {
        let c = UdpDriverConfig::new("10.0.0.9:7000".parse().unwrap()).identity(DeviceIdentity {
            model: Some("M".into()),
            ..DeviceIdentity::default()
        });
        assert_eq!(c.spec.identity.model.as_deref(), Some("M"));
    }
}
