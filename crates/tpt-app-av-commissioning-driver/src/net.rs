//! Shared network safety rules for protocol drivers (§36).
//!
//! Every network driver names its target explicitly, bounds every wait, and
//! reports socket failures in the same terms, so tests see one error model
//! regardless of protocol.

use std::io::ErrorKind;
use std::net::{IpAddr, SocketAddr};
use std::time::Duration;

use crate::error::DriverError;

/// Longest per-request timeout a driver accepts.
pub const MAX_TIMEOUT: Duration = Duration::from_secs(30);

/// Reject targets a commissioning tool must never fire at: unspecified,
/// multicast, broadcast, or port 0. Drivers talk to one named device.
pub fn check_target(target: SocketAddr) -> Result<(), DriverError> {
    let ip = target.ip();
    if ip.is_unspecified() || ip.is_multicast() {
        return Err(DriverError::Config(format!(
            "target {ip} must be an explicit unicast address"
        )));
    }
    if matches!(ip, IpAddr::V4(v4) if v4.is_broadcast()) {
        return Err(DriverError::Config(
            "broadcast targets are not allowed".to_owned(),
        ));
    }
    if target.port() == 0 {
        return Err(DriverError::Config("target port must not be 0".to_owned()));
    }
    Ok(())
}

/// Reject zero or excessive timeouts.
pub fn check_timeout(timeout: Duration) -> Result<(), DriverError> {
    if timeout.is_zero() || timeout > MAX_TIMEOUT {
        return Err(DriverError::Config(format!(
            "timeout must be between 1 ms and {} s",
            MAX_TIMEOUT.as_secs()
        )));
    }
    Ok(())
}

/// Map a socket error onto the driver error model.
pub fn map_io_error(
    error: &std::io::Error,
    target: impl std::fmt::Display,
    timeout: Duration,
) -> DriverError {
    match error.kind() {
        // A timed-out read is WouldBlock on Unix and TimedOut on Windows.
        ErrorKind::WouldBlock | ErrorKind::TimedOut => {
            DriverError::Timeout(timeout.as_millis() as u64)
        }
        ErrorKind::ConnectionRefused
        | ErrorKind::ConnectionReset
        | ErrorKind::ConnectionAborted
        | ErrorKind::NotConnected
        | ErrorKind::BrokenPipe
        | ErrorKind::UnexpectedEof
        | ErrorKind::NetworkUnreachable
        | ErrorKind::HostUnreachable
        | ErrorKind::AddrNotAvailable => DriverError::Unreachable(format!("{target}: {error}")),
        _ => DriverError::Protocol(format!("{target}: {error}")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn targets_must_be_explicit_unicast() {
        for bad in [
            "0.0.0.0:1",
            "224.0.0.1:1",
            "255.255.255.255:1",
            "10.0.0.1:0",
            "[::]:1",
        ] {
            assert!(check_target(bad.parse().unwrap()).is_err(), "{bad}");
        }
        assert!(check_target("127.0.0.1:80".parse().unwrap()).is_ok());
        assert!(check_target("[::1]:80".parse().unwrap()).is_ok());
    }

    #[test]
    fn timeouts_are_bounded() {
        assert!(check_timeout(Duration::ZERO).is_err());
        assert!(check_timeout(MAX_TIMEOUT + Duration::from_secs(1)).is_err());
        assert!(check_timeout(Duration::from_millis(1)).is_ok());
        assert!(check_timeout(MAX_TIMEOUT).is_ok());
    }

    #[test]
    fn io_errors_map_to_the_driver_model() {
        let t: SocketAddr = "10.0.0.1:1".parse().unwrap();
        let d = Duration::from_millis(250);
        let e = |k| std::io::Error::from(k);
        assert_eq!(
            map_io_error(&e(ErrorKind::TimedOut), t, d),
            DriverError::Timeout(250)
        );
        assert!(matches!(
            map_io_error(&e(ErrorKind::ConnectionRefused), t, d),
            DriverError::Unreachable(_)
        ));
        assert!(matches!(
            map_io_error(&e(ErrorKind::InvalidData), t, d),
            DriverError::Protocol(_)
        ));
    }
}
