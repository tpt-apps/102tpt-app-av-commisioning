//! How a device is reached (`DeviceAddress`).

use serde::{Deserialize, Serialize};
use std::net::IpAddr;
use std::str::FromStr;

/// An extensible description of how to reach a device.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", content = "value", rename_all = "snake_case")]
pub enum DeviceAddress {
    /// Plain IP address (optionally a bare host).
    Ip(IpAddr),
    /// Host plus port (TCP/UDP services).
    HostPort { host: String, port: u16 },
    /// Serial line.
    Serial { port: String, baud: u32 },
    /// OSC endpoint.
    Osc { host: String, port: u16 },
    /// MIDI port.
    Midi { port: String },
    /// Other URI-style address (RTSP, NDI, WebSocket …).
    Uri(String),
    /// Arbitrary vendor addressing, described by a driver.
    Other(String),
}

impl DeviceAddress {
    /// True if this address is network-reachable (IP or host-based).
    pub fn is_network(&self) -> bool {
        matches!(
            self,
            DeviceAddress::Ip(_)
                | DeviceAddress::HostPort { .. }
                | DeviceAddress::Osc { .. }
                | DeviceAddress::Uri(_)
        )
    }

    /// A short display string.
    pub fn display(&self) -> String {
        match self {
            DeviceAddress::Ip(ip) => ip.to_string(),
            DeviceAddress::HostPort { host, port } => format!("{host}:{port}"),
            DeviceAddress::Serial { port, baud } => format!("{port}@{baud}"),
            DeviceAddress::Osc { host, port } => format!("osc://{host}:{port}"),
            DeviceAddress::Midi { port } => format!("midi:{port}"),
            DeviceAddress::Uri(uri) => uri.clone(),
            DeviceAddress::Other(value) => value.clone(),
        }
    }
}

impl FromStr for DeviceAddress {
    type Err = AddressParseError;

    /// Lenient parsing: bare IP, `host:port`, `ip:port`.
    fn from_str(value: &str) -> Result<Self, Self::Err> {
        if let Ok(ip) = value.parse::<IpAddr>() {
            return Ok(DeviceAddress::Ip(ip));
        }
        if let Some((host, port)) = value.rsplit_once(':') {
            if let Ok(port) = port.parse::<u16>() {
                if host.parse::<IpAddr>().is_ok() || !host.is_empty() {
                    return Ok(DeviceAddress::HostPort {
                        host: host.to_owned(),
                        port,
                    });
                }
            }
        }
        Err(AddressParseError(value.to_owned()))
    }
}

/// Error returned when an address string cannot be parsed.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("cannot parse device address from `{0}`")]
pub struct AddressParseError(pub String);

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_bare_ip() {
        let a: DeviceAddress = "192.168.1.40".parse().unwrap();
        assert_eq!(a, DeviceAddress::Ip("192.168.1.40".parse().unwrap()));
    }

    #[test]
    fn parses_host_port() {
        let a: DeviceAddress = "projector.local:4352".parse().unwrap();
        assert_eq!(
            a,
            DeviceAddress::HostPort {
                host: "projector.local".to_owned(),
                port: 4352
            }
        );
    }

    #[test]
    fn rejects_garbage() {
        let r: Result<DeviceAddress, _> = "not an address at all !!".parse();
        assert!(r.is_err());
    }

    #[test]
    fn network_detection() {
        assert!(DeviceAddress::Ip("10.0.0.5".parse().unwrap()).is_network());
        let serial = DeviceAddress::Serial {
            port: "COM3".to_owned(),
            baud: 9600,
        };
        assert!(!serial.is_network());
    }
}
