//! OSC discovery.
//!
//! OSC has no standard discovery, but many devices answer a query to an
//! info-style address (`/info` by default). For every host:port in scope this
//! sends one argument-less message and accepts any well-formed OSC reply, so a
//! hit means "something here speaks OSC", with the reply address and first
//! argument recorded.

use std::net::SocketAddr;

use tpt_av_control_osc::{OscArg, OscMessage, OscServer};

use tpt_app_av_commissioning_driver::{DiscoveredDevice, DriverError};

use crate::util::{parallel_map, sanitize, udp_to, Ctx};

pub(crate) const DEFAULT_ADDRESS: &str = "/info";
/// Largest reply read.
const MAX_REPLY: usize = 8192;

fn describe(arg: &OscArg) -> String {
    match arg {
        OscArg::Int(v) => v.to_string(),
        OscArg::Long(v) => v.to_string(),
        OscArg::Float(v) => v.to_string(),
        OscArg::Double(v) => v.to_string(),
        OscArg::Bool(v) => v.to_string(),
        OscArg::String(s) | OscArg::Symbol(s) => sanitize(s, 128),
        other => format!("<{}>", other.type_tag()),
    }
}

pub(crate) fn probe(
    ctx: &Ctx<'_>,
    ports: &[u16],
    address: &str,
) -> Result<Vec<DiscoveredDevice>, DriverError> {
    // Reject wildcard and malformed addresses up front.
    if address.chars().any(|c| "*?[]{} ,#".contains(c)) {
        return Err(DriverError::Config(format!(
            "OSC address {address:?} must be a plain path"
        )));
    }
    let query = OscMessage::new(address, &[])
        .map_err(|e| DriverError::Config(format!("invalid OSC address {address:?}: {e}")))?
        .encode();
    let targets: Vec<SocketAddr> = ctx
        .hosts
        .iter()
        .flat_map(|h| ports.iter().map(move |p| SocketAddr::new(*h, *p)))
        .collect();
    Ok(parallel_map(
        &targets,
        ctx.concurrency,
        ctx.cancel,
        |target| {
            let socket = udp_to(*target, ctx.interface).ok()?;
            socket.set_read_timeout(Some(ctx.timeout)).ok()?;
            socket.send(&query).ok()?;
            let mut buf = vec![0u8; MAX_REPLY];
            let n = socket.recv(&mut buf).ok()?;
            let messages = OscServer::parse_bytes(&buf[..n]).ok()?;
            let first = messages.first()?;
            let mut device = DiscoveredDevice::new(target.to_string(), "osc")
                .detail("osc_reply_address", sanitize(&first.address, 128));
            if let Some(arg) = first.arguments.first() {
                device = device.detail("osc_reply_value", describe(arg));
            }
            Some(device)
        },
    ))
}
