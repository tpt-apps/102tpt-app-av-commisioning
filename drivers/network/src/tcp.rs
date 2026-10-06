//! TCP port probe (the "network scan").
//!
//! For every host in scope and every listed port, try to connect and close
//! again. Nothing is sent. With `read_banner`, wait briefly after connecting
//! and record whatever the device volunteers (some text protocols greet), cut
//! at 128 bytes and reduced to printable ASCII.

use std::io::Read;
use std::net::{IpAddr, SocketAddr};
use std::time::{Duration, Instant};

use tpt_app_av_commissioning_driver::DiscoveredDevice;

use crate::util::{connect, parallel_map, sanitize, Ctx};

/// Longest banner kept.
const BANNER_BYTES: usize = 128;
/// How long to wait for a banner.
const BANNER_WAIT: Duration = Duration::from_millis(300);

pub(crate) fn probe(ctx: &Ctx<'_>, ports: &[u16], read_banner: bool) -> Vec<DiscoveredDevice> {
    let targets: Vec<(IpAddr, u16)> = ctx
        .hosts
        .iter()
        .flat_map(|h| ports.iter().map(move |p| (*h, *p)))
        .collect();
    parallel_map(&targets, ctx.concurrency, ctx.cancel, |(host, port)| {
        let addr = SocketAddr::new(*host, *port);
        let started = Instant::now();
        let mut stream = connect(addr, ctx.timeout, ctx.interface).ok()?;
        let rtt = started.elapsed();
        let mut device = DiscoveredDevice::new(addr.to_string(), "tcp_probe")
            .detail("port", port.to_string())
            .detail("connect_ms", rtt.as_millis().to_string());
        if read_banner {
            let wait = BANNER_WAIT.min(ctx.timeout);
            if stream.set_read_timeout(Some(wait)).is_ok() {
                let mut buf = [0u8; BANNER_BYTES];
                if let Ok(n) = stream.read(&mut buf) {
                    let banner = sanitize(&String::from_utf8_lossy(&buf[..n]), BANNER_BYTES);
                    if !banner.is_empty() {
                        device = device.detail("banner", banner);
                    }
                }
            }
        }
        Some(device)
    })
}
