//! Device discovery (§11).
//!
//! Discovery is always explicit and user-controlled: an interface, scope,
//! protocol, and timeout must be supplied. There is no aggressive or
//! arbitrary scanning by default (§36).

use std::net::IpAddr;

use serde::{Deserialize, Serialize};

use crate::error::DriverError;
use tpt_app_av_commissioning_device::{DeviceIdentity, DeviceState};

/// The scope a discovery pass may search.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum DiscoveryScope {
    /// A single host.
    Host(IpAddr),
    /// A CIDR range, e.g. `192.168.1.0/24`.
    Cidr(String),
    /// Enumerate local bus/OS devices (MIDI ports, serial, audio).
    Local,
    /// Vendor/protocol-specific discovery (e.g. OSC groups).
    Protocol(String),
}

/// Configuration for a discovery pass.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DiscoveryConfig {
    /// The scope to search (explicit — never all interfaces).
    pub scope: DiscoveryScope,
    /// Interface to bind discovery to, if applicable.
    pub interface: Option<String>,
    /// Milliseconds to wait per target.
    pub timeout_ms: u64,
    /// Maximum number of parallel probes.
    pub concurrency: usize,
}

impl DiscoveryConfig {
    pub fn new(scope: DiscoveryScope) -> Self {
        Self {
            scope,
            interface: None,
            timeout_ms: 500,
            concurrency: 16,
        }
    }
}

/// A discovered device, ready to be matched to a driver.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DiscoveredDevice {
    /// Address endpoint matched (host/port/osc/midi…) as display text.
    pub address: String,
    pub identity: DeviceIdentity,
    /// Optional state captured during discovery (e.g. power state).
    pub state: Option<DeviceState>,
}

/// Trait implemented by drivers that can find devices.
pub trait DeviceDiscovery {
    /// Discovery method name (for config/UI display).
    fn method_name(&self) -> &'static str;

    /// Run one discovery pass under an explicit configuration.
    fn discover(&self, config: &DiscoveryConfig) -> Result<Vec<DiscoveredDevice>, DriverError>;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn discovery_config_defaults() {
        let c = DiscoveryConfig::new(DiscoveryScope::Cidr("192.168.1.0/24".into()));
        assert_eq!(c.timeout_ms, 500);
        assert_eq!(c.concurrency, 16);
        assert!(c.interface.is_none());
    }

    #[test]
    fn discovery_scope_serde() {
        let scope = DiscoveryScope::Cidr("10.0.0.0/8".into());
        let json = serde_json::to_string(&scope).unwrap();
        let back: DiscoveryScope = serde_json::from_str(&json).unwrap();
        assert_eq!(back, scope);
    }
}