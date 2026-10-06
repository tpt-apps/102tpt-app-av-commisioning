//! Device discovery (§11).
//!
//! Finds devices on the commissioning network and on this machine, under the
//! rules of §36: nothing runs unless the caller names a scope and the
//! protocols to use, and `DiscoveryConfig::validate` (in the driver SDK) caps
//! the number of hosts, refuses public address ranges without an explicit
//! opt-in, and requires a network interface for multicast.
//!
//! | Protocol | Scope | What it does |
//! |---|---|---|
//! | `tcp_probe` | host / CIDR | connect to each listed port and close; nothing is sent (optional banner read) |
//! | `snmp` | host / CIDR | SNMP GET of sysDescr, sysObjectID, sysName |
//! | `osc` | host / CIDR | send an argument-less OSC query, accept any OSC reply |
//! | `mdns` | multicast | one mDNS query on the chosen interface |
//! | `ssdp` | multicast | one SSDP M-SEARCH on the chosen interface |
//! | `midi` / `serial` / `audio` | local | enumerate this machine's ports and devices |
//!
//! One mechanism failing never hides another's results: [`run`] returns a
//! [`DiscoveryReport`] with the devices found *and* each mechanism's error.
//! Scans can be cancelled, and results are deduplicated and ordered.
//!
//! A discovered device is a *candidate*: [`match_profiles`] pairs it with the
//! device profiles whose `match` section fits what it reported, but adding it
//! to a project remains the engineer's decision.
//!
//! Licensed under either of MIT OR Apache-2.0, at your option.

mod dns;
mod local;
mod mdns;
mod osc;
mod snmp;
mod ssdp;
mod tcp;
mod util;

use std::net::{IpAddr, Ipv4Addr};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;

use tpt_app_av_commissioning_driver::{
    DeviceDiscovery, DiscoveredDevice, DiscoveryConfig, DiscoveryProtocol, DriverError,
};
use tpt_app_av_commissioning_profile::DeviceProfile;

pub use dns::{DnsMessage, DnsRecord, RData};
pub use util::{list_interfaces, resolve_interface, InterfaceInfo};

/// A cooperative cancellation handle for a discovery pass.
#[derive(Debug, Clone, Default)]
pub struct CancelFlag(Arc<AtomicBool>);

impl CancelFlag {
    pub fn new() -> Self {
        Self::default()
    }

    /// Ask the pass to stop at the next opportunity.
    pub fn cancel(&self) {
        self.0.store(true, Ordering::SeqCst);
    }

    pub fn is_cancelled(&self) -> bool {
        self.0.load(Ordering::SeqCst)
    }
}

/// The outcome of a discovery pass.
#[derive(Debug, Clone, PartialEq)]
pub struct DiscoveryReport {
    /// Devices found, deduplicated and ordered by address then source.
    pub devices: Vec<DiscoveredDevice>,
    /// Mechanisms that failed, with why. A failure here does not discard the
    /// devices other mechanisms found.
    pub errors: Vec<(String, DriverError)>,
    /// Hosts in the unicast scope (0 for local and multicast scopes).
    pub hosts_in_scope: usize,
    /// Whether the pass was cancelled before it finished.
    pub cancelled: bool,
}

/// Run one discovery pass. The config is validated first; if it is invalid
/// nothing is sent.
pub fn run(config: &DiscoveryConfig, cancel: &CancelFlag) -> Result<DiscoveryReport, DriverError> {
    config.validate()?;
    let interface: Option<Ipv4Addr> = config
        .interface
        .as_deref()
        .map(resolve_interface)
        .transpose()?;
    let timeout = Duration::from_millis(config.timeout_ms);
    let hosts: Vec<IpAddr> = match config.scope_kind() {
        Some(tpt_app_av_commissioning_driver::ScopeKind::Unicast) => config.hosts()?,
        _ => Vec::new(),
    };

    let mut devices = Vec::new();
    let mut errors = Vec::new();
    for protocol in &config.protocols {
        if cancel.is_cancelled() {
            break;
        }
        let ctx = util::Ctx {
            hosts: &hosts,
            interface,
            timeout,
            concurrency: config.concurrency,
            cancel,
        };
        let found = match protocol {
            DiscoveryProtocol::TcpProbe { ports, read_banner } => {
                Ok(tcp::probe(&ctx, ports, *read_banner))
            }
            DiscoveryProtocol::Snmp { community, port } => snmp::probe(
                &ctx,
                community.as_deref().unwrap_or(snmp::DEFAULT_COMMUNITY),
                port.unwrap_or(snmp::DEFAULT_PORT),
            ),
            DiscoveryProtocol::Osc { ports, address } => osc::probe(
                &ctx,
                ports,
                address.as_deref().unwrap_or(osc::DEFAULT_ADDRESS),
            ),
            DiscoveryProtocol::Mdns { service } => mdns::browse(
                &ctx,
                service.as_deref().unwrap_or(mdns::DEFAULT_SERVICE),
                mdns::GROUP,
                true,
            ),
            DiscoveryProtocol::Ssdp { search_target } => ssdp::search(
                &ctx,
                search_target.as_deref().unwrap_or(ssdp::DEFAULT_TARGET),
                ssdp::GROUP,
                true,
            ),
            DiscoveryProtocol::Midi => local::midi(),
            DiscoveryProtocol::Serial => local::serial(),
            DiscoveryProtocol::Audio => local::audio(),
        };
        match found {
            Ok(mut d) => devices.append(&mut d),
            Err(e) => errors.push((protocol.name().to_owned(), e)),
        }
    }

    // Deduplicate on (address, source) and give a stable order.
    devices.sort_by(|a, b| (&a.address, &a.source).cmp(&(&b.address, &b.source)));
    devices.dedup_by(|b, a| a.address == b.address && a.source == b.source);

    Ok(DiscoveryReport {
        devices,
        errors,
        hosts_in_scope: hosts.len(),
        cancelled: cancel.is_cancelled(),
    })
}

/// The device profiles whose `match` section fits what `device` reported.
/// Discovery mechanisms that learn no manufacturer and model (a bare open
/// port, say) match nothing: a profile is never applied on a guess.
pub fn match_profiles<'a>(
    device: &DiscoveredDevice,
    profiles: &'a [DeviceProfile],
) -> Vec<&'a DeviceProfile> {
    profiles
        .iter()
        .filter(|p| p.matches(&device.identity))
        .collect()
}

/// [`DeviceDiscovery`] over every mechanism in this crate.
#[derive(Debug, Clone, Copy, Default)]
pub struct NetworkDiscovery;

impl DeviceDiscovery for NetworkDiscovery {
    fn method_name(&self) -> &'static str {
        "network"
    }

    /// Runs the pass and returns what was found. If every selected mechanism
    /// failed, the first error is returned instead of an empty list that would
    /// read as "nothing there".
    fn discover(&self, config: &DiscoveryConfig) -> Result<Vec<DiscoveredDevice>, DriverError> {
        let report = run(config, &CancelFlag::new())?;
        if report.devices.is_empty() {
            if let Some((_, e)) = report.errors.first() {
                if report.errors.len() == config.protocols.len() {
                    return Err(e.clone());
                }
            }
        }
        Ok(report.devices)
    }
}
