//! Device discovery (§11).
//!
//! Discovery is always explicit and user-controlled (§36): the caller names a
//! scope, the protocols to use, a timeout, and — for multicast — the network
//! interface. There is no default scope, no default protocol, and no way to
//! ask for "everything". [`DiscoveryConfig::validate`] enforces the limits
//! before a single packet is sent:
//!
//! * at least one protocol, each compatible with the scope;
//! * unicast scans cover a bounded number of hosts ([`MAX_HOSTS_LIMIT`]
//!   absolute, `max_hosts` by default 256), IPv4 only for CIDR ranges;
//! * only private, link-local, CGNAT and loopback ranges unless the caller
//!   sets `allow_public` — pointing a commissioning tool at someone else's
//!   network must be a deliberate act;
//! * timeouts, port lists and concurrency are capped.

use std::net::{IpAddr, Ipv4Addr, Ipv6Addr};

use serde::{Deserialize, Serialize};

use crate::error::DriverError;
use tpt_app_av_commissioning_device::{DeviceIdentity, DeviceState};

/// Absolute ceiling on hosts in one unicast scan.
pub const MAX_HOSTS_LIMIT: usize = 4096;
/// Default `max_hosts`.
pub const DEFAULT_MAX_HOSTS: usize = 256;
/// Most ports one probe may try per host.
pub const MAX_PORTS: usize = 16;
/// Longest per-target timeout.
pub const MAX_TIMEOUT_MS: u64 = 10_000;
/// Most parallel probes.
pub const MAX_CONCURRENCY: usize = 64;

/// The scope a discovery pass may search.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", content = "value", rename_all = "snake_case")]
pub enum DiscoveryScope {
    /// A single host.
    Host(IpAddr),
    /// An IPv4 CIDR range, e.g. `192.168.1.0/24`.
    Cidr(String),
    /// Enumerate local bus/OS devices (MIDI ports, serial, audio).
    Local,
    /// Link-local multicast on the selected interface (mDNS, SSDP).
    Multicast,
    /// Vendor/protocol-specific discovery (e.g. OSC groups).
    Protocol(String),
}

/// A discovery mechanism, with its own parameters. Selecting one is explicit:
/// the list in [`DiscoveryConfig::protocols`] is the whole of what runs.
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum DiscoveryProtocol {
    /// Network scan: try a TCP connection to each listed port on each host and
    /// close it. Nothing is sent.
    TcpProbe {
        ports: Vec<u16>,
        /// Wait briefly after connecting and record any banner the device
        /// volunteers (some text protocols send one).
        #[serde(default)]
        read_banner: bool,
    },
    /// mDNS service browse on the selected interface.
    Mdns {
        /// Service type, e.g. `_http._tcp.local`. Defaults to the service
        /// directory `_services._dns-sd._udp.local`.
        #[serde(default)]
        service: Option<String>,
    },
    /// SSDP M-SEARCH on the selected interface.
    Ssdp {
        /// Search target, e.g. `upnp:rootdevice`. Defaults to `ssdp:all`.
        #[serde(default)]
        search_target: Option<String>,
    },
    /// SNMP GET of sysDescr / sysObjectID / sysName on each host.
    Snmp {
        /// Read community. A credential: never serialized, redacted in
        /// `Debug`. Defaults to `public`.
        #[serde(default, skip_serializing)]
        community: Option<String>,
        /// Agent port (default 161).
        #[serde(default)]
        port: Option<u16>,
    },
    /// OSC probe: send an argument-less message to each host:port and accept
    /// any OSC reply.
    Osc {
        ports: Vec<u16>,
        /// Address to query (default `/info`).
        #[serde(default)]
        address: Option<String>,
    },
    /// Enumerate MIDI ports on this machine.
    Midi,
    /// Enumerate serial ports on this machine.
    Serial,
    /// Enumerate audio devices on this machine.
    Audio,
}

impl std::fmt::Debug for DiscoveryProtocol {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            DiscoveryProtocol::Snmp { port, .. } => f
                .debug_struct("Snmp")
                .field("community", &"<redacted>")
                .field("port", port)
                .finish(),
            DiscoveryProtocol::TcpProbe { ports, read_banner } => f
                .debug_struct("TcpProbe")
                .field("ports", ports)
                .field("read_banner", read_banner)
                .finish(),
            DiscoveryProtocol::Mdns { service } => {
                f.debug_struct("Mdns").field("service", service).finish()
            }
            DiscoveryProtocol::Ssdp { search_target } => f
                .debug_struct("Ssdp")
                .field("search_target", search_target)
                .finish(),
            DiscoveryProtocol::Osc { ports, address } => f
                .debug_struct("Osc")
                .field("ports", ports)
                .field("address", address)
                .finish(),
            DiscoveryProtocol::Midi => f.write_str("Midi"),
            DiscoveryProtocol::Serial => f.write_str("Serial"),
            DiscoveryProtocol::Audio => f.write_str("Audio"),
        }
    }
}

/// The kind of scope a protocol needs.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ScopeKind {
    /// `Host` or `Cidr`.
    Unicast,
    /// `Multicast`.
    Multicast,
    /// `Local`.
    Local,
}

impl DiscoveryProtocol {
    /// Stable lowercase name.
    pub fn name(&self) -> &'static str {
        match self {
            DiscoveryProtocol::TcpProbe { .. } => "tcp_probe",
            DiscoveryProtocol::Mdns { .. } => "mdns",
            DiscoveryProtocol::Ssdp { .. } => "ssdp",
            DiscoveryProtocol::Snmp { .. } => "snmp",
            DiscoveryProtocol::Osc { .. } => "osc",
            DiscoveryProtocol::Midi => "midi",
            DiscoveryProtocol::Serial => "serial",
            DiscoveryProtocol::Audio => "audio",
        }
    }

    /// The scope kind this protocol works with.
    pub fn scope_kind(&self) -> ScopeKind {
        match self {
            DiscoveryProtocol::TcpProbe { .. }
            | DiscoveryProtocol::Snmp { .. }
            | DiscoveryProtocol::Osc { .. } => ScopeKind::Unicast,
            DiscoveryProtocol::Mdns { .. } | DiscoveryProtocol::Ssdp { .. } => ScopeKind::Multicast,
            DiscoveryProtocol::Midi | DiscoveryProtocol::Serial | DiscoveryProtocol::Audio => {
                ScopeKind::Local
            }
        }
    }
}

/// Configuration for a discovery pass.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DiscoveryConfig {
    /// The scope to search (explicit — never all interfaces).
    pub scope: DiscoveryScope,
    /// The mechanisms to run. Empty is invalid.
    #[serde(default)]
    pub protocols: Vec<DiscoveryProtocol>,
    /// Network interface to use, by name or IPv4 address. Required for
    /// multicast; for unicast scans it selects the source address.
    pub interface: Option<String>,
    /// Milliseconds to wait per target.
    pub timeout_ms: u64,
    /// Maximum number of parallel probes.
    pub concurrency: usize,
    /// Most hosts a unicast scan may cover (capped at [`MAX_HOSTS_LIMIT`]).
    #[serde(default = "default_max_hosts")]
    pub max_hosts: usize,
    /// Allow scanning outside private/link-local/loopback ranges.
    #[serde(default)]
    pub allow_public: bool,
}

fn default_max_hosts() -> usize {
    DEFAULT_MAX_HOSTS
}

impl DiscoveryConfig {
    /// A config for `scope` with no protocols selected yet.
    pub fn new(scope: DiscoveryScope) -> Self {
        Self {
            scope,
            protocols: Vec::new(),
            interface: None,
            timeout_ms: 500,
            concurrency: 16,
            max_hosts: DEFAULT_MAX_HOSTS,
            allow_public: false,
        }
    }

    /// Add a mechanism.
    pub fn protocol(mut self, protocol: DiscoveryProtocol) -> Self {
        self.protocols.push(protocol);
        self
    }

    pub fn interface(mut self, interface: impl Into<String>) -> Self {
        self.interface = Some(interface.into());
        self
    }

    pub fn timeout_ms(mut self, timeout_ms: u64) -> Self {
        self.timeout_ms = timeout_ms;
        self
    }

    pub fn concurrency(mut self, concurrency: usize) -> Self {
        self.concurrency = concurrency;
        self
    }

    pub fn max_hosts(mut self, max_hosts: usize) -> Self {
        self.max_hosts = max_hosts;
        self
    }

    pub fn allow_public(mut self, allow: bool) -> Self {
        self.allow_public = allow;
        self
    }

    /// The scope kind this config's scope belongs to, if it is one a protocol
    /// can use (`Protocol(_)` is reserved for vendor discovery).
    pub fn scope_kind(&self) -> Option<ScopeKind> {
        match self.scope {
            DiscoveryScope::Host(_) | DiscoveryScope::Cidr(_) => Some(ScopeKind::Unicast),
            DiscoveryScope::Multicast => Some(ScopeKind::Multicast),
            DiscoveryScope::Local => Some(ScopeKind::Local),
            DiscoveryScope::Protocol(_) => None,
        }
    }

    /// Check every limit. Nothing is sent before this passes.
    pub fn validate(&self) -> Result<(), DriverError> {
        let bad = |m: String| Err(DriverError::Config(m));
        if self.protocols.is_empty() {
            return bad("select at least one discovery protocol".to_owned());
        }
        if self.timeout_ms == 0 || self.timeout_ms > MAX_TIMEOUT_MS {
            return bad(format!("timeout_ms must be between 1 and {MAX_TIMEOUT_MS}"));
        }
        if self.concurrency == 0 || self.concurrency > MAX_CONCURRENCY {
            return bad(format!(
                "concurrency must be between 1 and {MAX_CONCURRENCY}"
            ));
        }
        if self.max_hosts == 0 || self.max_hosts > MAX_HOSTS_LIMIT {
            return bad(format!("max_hosts must be between 1 and {MAX_HOSTS_LIMIT}"));
        }
        let Some(kind) = self.scope_kind() else {
            return bad(
                "`protocol` scopes are for vendor discovery; use host, cidr, local or multicast"
                    .to_owned(),
            );
        };
        for p in &self.protocols {
            if p.scope_kind() != kind {
                return bad(format!(
                    "{} needs a {:?} scope, not {:?}",
                    p.name(),
                    p.scope_kind(),
                    self.scope
                ));
            }
            match p {
                DiscoveryProtocol::TcpProbe { ports, .. }
                | DiscoveryProtocol::Osc { ports, .. } => {
                    if ports.is_empty() || ports.len() > MAX_PORTS || ports.contains(&0) {
                        return bad(format!(
                            "{}: list between 1 and {MAX_PORTS} ports (not 0)",
                            p.name()
                        ));
                    }
                }
                DiscoveryProtocol::Snmp { port: Some(0), .. } => {
                    return bad("snmp: port must not be 0".to_owned());
                }
                _ => {}
            }
        }
        if kind == ScopeKind::Multicast
            && self
                .interface
                .as_deref()
                .is_none_or(|i| i.trim().is_empty())
        {
            return bad("multicast discovery needs an explicit network interface".to_owned());
        }
        if kind == ScopeKind::Unicast {
            self.hosts()?;
        }
        Ok(())
    }

    /// The hosts a unicast scope covers, after the size and range checks.
    /// Network and broadcast addresses are skipped for ranges of /30 or wider.
    pub fn hosts(&self) -> Result<Vec<IpAddr>, DriverError> {
        let bad = |m: String| Err(DriverError::Config(m));
        match &self.scope {
            DiscoveryScope::Host(ip) => {
                if !self.allow_public && !is_private(*ip) {
                    return bad(format!(
                        "{ip} is outside private, link-local and loopback ranges; set allow_public to scan it"
                    ));
                }
                Ok(vec![*ip])
            }
            DiscoveryScope::Cidr(text) => {
                let (base, prefix) = parse_cidr(text)?;
                let count: u64 = 1u64 << (32 - prefix);
                let (first, last) = (u32::from(base), u32::from(base) + (count - 1) as u32);
                if !self.allow_public && !(range_is_private(first, last)) {
                    return bad(format!(
                        "{text} is outside private, link-local and loopback ranges; set allow_public to scan it"
                    ));
                }
                let (start, end) = if prefix <= 30 {
                    (first + 1, last - 1)
                } else {
                    (first, last)
                };
                let n = (end - start) as u64 + 1;
                if n > self.max_hosts as u64 {
                    return bad(format!(
                        "{text} covers {n} hosts, more than max_hosts ({}); narrow the range",
                        self.max_hosts
                    ));
                }
                Ok((start..=end)
                    .map(|a| IpAddr::V4(Ipv4Addr::from(a)))
                    .collect())
            }
            other => bad(format!("{other:?} is not a unicast scope")),
        }
    }
}

/// Parse `a.b.c.d/n` (IPv4 only). The address must be the network address, so
/// a typo such as `192.168.1.77/24` is caught rather than silently widened.
pub fn parse_cidr(text: &str) -> Result<(Ipv4Addr, u32), DriverError> {
    let bad = || {
        DriverError::Config(format!(
            "{text:?} is not an IPv4 CIDR range like 192.168.1.0/24"
        ))
    };
    let (addr, prefix) = text.split_once('/').ok_or_else(bad)?;
    let base: Ipv4Addr = addr.parse().map_err(|_| bad())?;
    let prefix: u32 = prefix.parse().map_err(|_| bad())?;
    if prefix > 32 {
        return Err(bad());
    }
    let mask = if prefix == 0 {
        0
    } else {
        u32::MAX << (32 - prefix)
    };
    if u32::from(base) & !mask != 0 {
        return Err(DriverError::Config(format!(
            "{text:?} has host bits set; did you mean {}/{prefix}?",
            Ipv4Addr::from(u32::from(base) & mask)
        )));
    }
    Ok((base, prefix))
}

/// Blocks a commissioning scan may touch without `allow_public`: RFC 1918,
/// link-local, CGNAT and loopback.
const PRIVATE_V4: [(u32, u32); 6] = [
    (0x0A00_0000, 0x0AFF_FFFF), // 10.0.0.0/8
    (0xAC10_0000, 0xAC1F_FFFF), // 172.16.0.0/12
    (0xC0A8_0000, 0xC0A8_FFFF), // 192.168.0.0/16
    (0xA9FE_0000, 0xA9FE_FFFF), // 169.254.0.0/16
    (0x6440_0000, 0x647F_FFFF), // 100.64.0.0/10
    (0x7F00_0000, 0x7FFF_FFFF), // 127.0.0.0/8
];

fn range_is_private(first: u32, last: u32) -> bool {
    PRIVATE_V4
        .iter()
        .any(|(lo, hi)| first >= *lo && last <= *hi)
}

/// Whether `ip` is private, link-local, CGNAT or loopback.
pub fn is_private(ip: IpAddr) -> bool {
    match ip {
        IpAddr::V4(v4) => {
            let n = u32::from(v4);
            range_is_private(n, n)
        }
        IpAddr::V6(v6) => {
            v6 == Ipv6Addr::LOCALHOST
                || (v6.segments()[0] & 0xffc0) == 0xfe80 // fe80::/10
                || (v6.segments()[0] & 0xfe00) == 0xfc00 // fc00::/7
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
    /// The mechanism that found it (`tcp_probe`, `mdns`, …).
    #[serde(default)]
    pub source: String,
    /// Mechanism-specific facts (service name, banner, server header, …),
    /// in a stable order.
    #[serde(default)]
    pub details: Vec<(String, String)>,
}

impl DiscoveredDevice {
    /// A device found at `address` by `source`.
    pub fn new(address: impl Into<String>, source: impl Into<String>) -> Self {
        Self {
            address: address.into(),
            identity: DeviceIdentity::new(),
            state: None,
            source: source.into(),
            details: Vec::new(),
        }
    }

    /// Record a fact about the device.
    pub fn detail(mut self, key: impl Into<String>, value: impl Into<String>) -> Self {
        self.details.push((key.into(), value.into()));
        self
    }

    /// Look up a recorded fact.
    pub fn get(&self, key: &str) -> Option<&str> {
        self.details
            .iter()
            .find(|(k, _)| k == key)
            .map(|(_, v)| v.as_str())
    }
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

    fn tcp(ports: &[u16]) -> DiscoveryProtocol {
        DiscoveryProtocol::TcpProbe {
            ports: ports.to_vec(),
            read_banner: false,
        }
    }

    fn cidr(range: &str) -> DiscoveryConfig {
        DiscoveryConfig::new(DiscoveryScope::Cidr(range.into())).protocol(tcp(&[80]))
    }

    #[test]
    fn discovery_config_defaults_are_inert() {
        let c = DiscoveryConfig::new(DiscoveryScope::Cidr("192.168.1.0/24".into()));
        assert_eq!(c.timeout_ms, 500);
        assert_eq!(c.concurrency, 16);
        assert!(c.interface.is_none());
        assert!(!c.allow_public);
        // Nothing runs until a protocol is chosen.
        assert!(c.validate().is_err());
    }

    #[test]
    fn serde_round_trip_never_writes_the_community() {
        let c = DiscoveryConfig::new(DiscoveryScope::Cidr("10.0.0.0/24".into())).protocol(
            DiscoveryProtocol::Snmp {
                community: Some("s3cret".into()),
                port: None,
            },
        );
        let json = serde_json::to_string(&c).unwrap();
        assert!(!json.contains("s3cret"));
        assert!(!format!("{c:?}").contains("s3cret"));
        let back: DiscoveryConfig = serde_json::from_str(&json).unwrap();
        assert_eq!(back.protocols.len(), 1);
    }

    #[test]
    fn scope_serde() {
        let scope = DiscoveryScope::Cidr("10.0.0.0/8".into());
        let json = serde_json::to_string(&scope).unwrap();
        assert_eq!(
            serde_json::from_str::<DiscoveryScope>(&json).unwrap(),
            scope
        );
    }

    #[test]
    fn cidr_expansion_skips_network_and_broadcast() {
        let hosts = cidr("192.168.1.0/30").hosts().unwrap();
        assert_eq!(
            hosts,
            vec![
                "192.168.1.1".parse::<IpAddr>().unwrap(),
                "192.168.1.2".parse().unwrap()
            ]
        );
        assert_eq!(cidr("192.168.1.0/31").hosts().unwrap().len(), 2);
        assert_eq!(cidr("192.168.1.5/32").hosts().unwrap().len(), 1);
        assert_eq!(cidr("192.168.1.0/24").hosts().unwrap().len(), 254);
    }

    #[test]
    fn cidr_must_be_a_network_address() {
        let err = cidr("192.168.1.77/24").hosts().unwrap_err();
        assert!(err.to_string().contains("192.168.1.0/24"));
        for bad in [
            "192.168.1.0",
            "192.168.1.0/33",
            "x/24",
            "192.168.1.0/",
            "::1/64",
        ] {
            assert!(cidr(bad).hosts().is_err(), "{bad}");
        }
    }

    #[test]
    fn scans_are_size_limited() {
        // 192.168.0.0/16 is private but far too big.
        assert!(cidr("192.168.0.0/16").validate().is_err());
        assert!(cidr("192.168.0.0/23").validate().is_err()); // 510 > 256
        assert!(cidr("192.168.0.0/23").max_hosts(512).validate().is_ok());
        // The absolute ceiling cannot be raised past.
        assert!(cidr("10.0.0.0/16").max_hosts(100_000).validate().is_err());
        assert!(cidr("10.0.0.0/8")
            .max_hosts(MAX_HOSTS_LIMIT)
            .validate()
            .is_err());
    }

    #[test]
    fn public_ranges_need_an_explicit_opt_in() {
        assert!(cidr("8.8.8.0/24").validate().is_err());
        assert!(cidr("8.8.8.0/24").allow_public(true).validate().is_ok());
        // A range that merely overlaps a private block is still public.
        assert!(cidr("172.0.0.0/8")
            .max_hosts(MAX_HOSTS_LIMIT)
            .validate()
            .is_err());
        for ok in [
            "10.1.2.0/24",
            "172.16.5.0/24",
            "169.254.1.0/24",
            "100.64.1.0/24",
            "127.0.0.0/30",
        ] {
            assert!(cidr(ok).validate().is_ok(), "{ok}");
        }
        let host = |ip: &str| {
            DiscoveryConfig::new(DiscoveryScope::Host(ip.parse().unwrap())).protocol(tcp(&[80]))
        };
        assert!(host("192.168.1.10").validate().is_ok());
        assert!(host("::1").validate().is_ok());
        assert!(host("fe80::1").validate().is_ok());
        assert!(host("1.1.1.1").validate().is_err());
        assert!(host("2001:db8::1").validate().is_err());
    }

    #[test]
    fn protocols_must_match_the_scope() {
        let local = DiscoveryConfig::new(DiscoveryScope::Local);
        assert!(local
            .clone()
            .protocol(DiscoveryProtocol::Midi)
            .validate()
            .is_ok());
        assert!(local.clone().protocol(tcp(&[80])).validate().is_err());
        assert!(cidr("10.0.0.0/24")
            .protocol(DiscoveryProtocol::Midi)
            .validate()
            .is_err());
        let multicast = DiscoveryConfig::new(DiscoveryScope::Multicast)
            .protocol(DiscoveryProtocol::Mdns { service: None });
        assert!(multicast.clone().validate().is_err(), "needs an interface");
        assert!(multicast.clone().interface("  ").validate().is_err());
        assert!(multicast.interface("192.168.1.10").validate().is_ok());
        assert!(DiscoveryConfig::new(DiscoveryScope::Protocol("x".into()))
            .protocol(DiscoveryProtocol::Midi)
            .validate()
            .is_err());
    }

    #[test]
    fn limits_are_enforced() {
        let base = cidr("10.0.0.0/24");
        assert!(base.clone().timeout_ms(0).validate().is_err());
        assert!(base
            .clone()
            .timeout_ms(MAX_TIMEOUT_MS + 1)
            .validate()
            .is_err());
        assert!(base.clone().concurrency(0).validate().is_err());
        assert!(base
            .clone()
            .concurrency(MAX_CONCURRENCY + 1)
            .validate()
            .is_err());
        assert!(base.clone().max_hosts(0).validate().is_err());
        let ports = |p: &[u16]| {
            DiscoveryConfig::new(DiscoveryScope::Host("10.0.0.5".parse().unwrap()))
                .protocol(tcp(p))
                .validate()
        };
        assert!(ports(&[]).is_err());
        assert!(ports(&[0]).is_err());
        assert!(ports(&(1..=17).collect::<Vec<u16>>()).is_err());
        assert!(ports(&(1..=16).collect::<Vec<u16>>()).is_ok());
    }

    #[test]
    fn discovered_devices_carry_source_and_details() {
        let d = DiscoveredDevice::new("10.0.0.5:4352", "tcp_probe").detail("banner", "PJLINK 0");
        assert_eq!(d.get("banner"), Some("PJLINK 0"));
        assert_eq!(d.get("nope"), None);
        assert_eq!(d.source, "tcp_probe");
    }
}
