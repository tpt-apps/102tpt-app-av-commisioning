//! mDNS (Bonjour/Avahi) browse on one selected interface.
//!
//! Sends one PTR query (twice, to survive a lost datagram) to the mDNS group
//! 224.0.0.251:5353 from the chosen interface, with the "QU" bit set so
//! devices answer unicast to our port. We therefore never join the multicast
//! group or bind port 5353, and cannot disturb a resident mDNS responder; the
//! trade-off is that a device that insists on multicasting its reply is not
//! heard.
//!
//! Replies are parsed with the strict codec in [`crate::dns`]. A service
//! instance (SRV) becomes one device with its host, port and TXT entries; a
//! device that only lists service types (the default directory query) becomes
//! one device with a `service_type` detail per type.

use std::collections::{BTreeMap, HashMap};
use std::net::{IpAddr, Ipv4Addr, SocketAddr, SocketAddrV4, UdpSocket};
use std::time::{Duration, Instant};

use tpt_app_av_commissioning_driver::{DiscoveredDevice, DriverError};

use crate::dns::{self, RData};
use crate::util::{sanitize, Ctx};

/// The mDNS multicast group.
pub(crate) const GROUP: SocketAddrV4 = SocketAddrV4::new(Ipv4Addr::new(224, 0, 0, 251), 5353);
/// The service directory: asks "what services do you offer?".
pub(crate) const DEFAULT_SERVICE: &str = "_services._dns-sd._udp.local";
/// Datagrams processed per pass.
const MAX_DATAGRAMS: usize = 256;
/// Largest datagram read.
const MAX_DATAGRAM: usize = 9000;
/// TXT entries and service types kept per device.
const MAX_LISTED: usize = 16;

pub(crate) fn browse(
    ctx: &Ctx<'_>,
    service: &str,
    target: SocketAddrV4,
    multicast: bool,
) -> Result<Vec<DiscoveredDevice>, DriverError> {
    let interface = ctx.interface.ok_or_else(|| {
        DriverError::Config("mDNS needs an explicit network interface".to_owned())
    })?;
    let query = dns::encode_query(service, dns::TYPE_PTR, true)
        .map_err(|e| DriverError::Config(format!("invalid mDNS service {service:?}: {e}")))?;

    let socket = socket2::Socket::new(
        socket2::Domain::IPV4,
        socket2::Type::DGRAM,
        Some(socket2::Protocol::UDP),
    )
    .map_err(io)?;
    socket
        .bind(&SocketAddr::V4(SocketAddrV4::new(interface, 0)).into())
        .map_err(io)?;
    if multicast {
        socket.set_multicast_if_v4(&interface).map_err(io)?;
        socket.set_multicast_ttl_v4(255).map_err(io)?;
        socket.set_multicast_loop_v4(false).map_err(io)?;
    }
    let socket: UdpSocket = socket.into();

    let mut answers: Vec<(IpAddr, dns::DnsMessage)> = Vec::new();
    let started = Instant::now();
    let deadline = started + ctx.timeout;
    let mut sent = 0;
    let mut buf = vec![0u8; MAX_DATAGRAM];
    let mut datagrams = 0;
    while Instant::now() < deadline && datagrams < MAX_DATAGRAMS && !ctx.cancel.is_cancelled() {
        // Two queries: now, and a third of the way through the wait.
        let due = ctx.timeout / 3 * sent;
        if sent < 2 && started.elapsed() >= due {
            socket.send_to(&query, target).map_err(io)?;
            sent += 1;
        }
        let slice =
            Duration::from_millis(50).min(deadline.saturating_duration_since(Instant::now()));
        if slice.is_zero() {
            break;
        }
        socket.set_read_timeout(Some(slice)).map_err(io)?;
        match socket.recv_from(&mut buf) {
            Ok((n, from)) => {
                datagrams += 1;
                if let Ok(message) = dns::parse(&buf[..n]) {
                    if message.is_response {
                        answers.push((from.ip(), message));
                    }
                }
            }
            Err(e)
                if matches!(
                    e.kind(),
                    std::io::ErrorKind::WouldBlock
                        | std::io::ErrorKind::TimedOut
                        // Windows reports an ICMP error from an earlier send.
                        | std::io::ErrorKind::ConnectionReset
                ) => {}
            Err(e) => return Err(io(e)),
        }
    }
    Ok(devices_from(&answers))
}

fn io(e: std::io::Error) -> DriverError {
    DriverError::Protocol(format!("mDNS socket error: {e}"))
}

/// Turn parsed responses into devices.
pub(crate) fn devices_from(answers: &[(IpAddr, dns::DnsMessage)]) -> Vec<DiscoveredDevice> {
    let mut hosts: HashMap<String, Ipv4Addr> = HashMap::new();
    let mut txts: HashMap<String, Vec<String>> = HashMap::new();
    let mut service_of: HashMap<String, String> = HashMap::new();
    // (source, instance name, host, port)
    let mut instances: Vec<(IpAddr, String, String, u16)> = Vec::new();
    // Service types a source listed without any instance.
    let mut types: BTreeMap<IpAddr, Vec<String>> = BTreeMap::new();

    for (src, message) in answers {
        for r in &message.records {
            match &r.data {
                RData::A(ip) => {
                    hosts.insert(r.name.to_ascii_lowercase(), *ip);
                }
                RData::Txt(t) => {
                    txts.entry(r.name.clone())
                        .or_default()
                        .extend(t.iter().cloned());
                }
                RData::Srv { port, target } => {
                    let instance = (*src, r.name.clone(), target.to_ascii_lowercase(), *port);
                    // Each query is answered separately; keep one of each.
                    if !instances.contains(&instance) {
                        instances.push(instance);
                    }
                }
                RData::Ptr(target) => {
                    // `_http._tcp.local -> Lux._http._tcp.local` is an
                    // instance of that service; a PTR to a service type is a
                    // directory entry.
                    service_of.insert(target.clone(), r.name.clone());
                    let entry = types.entry(*src).or_default();
                    let listed = sanitize(target, 128);
                    if entry.len() < MAX_LISTED && !entry.contains(&listed) {
                        entry.push(listed);
                    }
                }
                RData::Other => {}
            }
        }
    }

    let mut devices = Vec::new();
    let mut with_instance: Vec<IpAddr> = Vec::new();
    for (src, instance, host, port) in &instances {
        let ip = hosts.get(host).map(|a| IpAddr::V4(*a)).unwrap_or(*src);
        with_instance.push(*src);
        let mut d = DiscoveredDevice::new(SocketAddr::new(ip, *port).to_string(), "mdns")
            .detail("instance", sanitize(instance, 128))
            .detail("host", sanitize(host, 128));
        if let Some(service) = service_of.get(instance) {
            d = d.detail("service", sanitize(service, 128));
        }
        for entry in txts.get(instance).into_iter().flatten().take(MAX_LISTED) {
            d = d.detail("txt", sanitize(entry, 128));
        }
        devices.push(d);
    }
    for (src, listed) in types {
        if with_instance.contains(&src) {
            continue;
        }
        let mut d = DiscoveredDevice::new(src.to_string(), "mdns");
        for t in listed {
            d = d.detail("service_type", t);
        }
        devices.push(d);
    }
    devices
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::dns::build::*;
    use crate::dns::{TYPE_A, TYPE_PTR, TYPE_SRV, TYPE_TXT};

    fn parsed(records: &[Vec<u8>]) -> dns::DnsMessage {
        dns::parse(&message(records)).unwrap()
    }

    #[test]
    fn instances_become_devices_with_resolved_addresses() {
        let src: IpAddr = "192.168.1.50".parse().unwrap();
        let msg = parsed(&[
            record("_http._tcp.local", TYPE_PTR, &name("Lux._http._tcp.local")),
            record(
                "Lux._http._tcp.local",
                TYPE_SRV,
                &srv(8080, "Lux-Host.local"),
            ),
            record("lux-host.local", TYPE_A, &[192, 168, 1, 61]),
            record("Lux._http._tcp.local", TYPE_TXT, &txt(&["model=Lux-3"])),
        ]);
        let devices = devices_from(&[(src, msg)]);
        assert_eq!(devices.len(), 1);
        let d = &devices[0];
        assert_eq!(d.address, "192.168.1.61:8080");
        assert_eq!(d.source, "mdns");
        assert_eq!(d.get("service"), Some("_http._tcp.local"));
        assert_eq!(d.get("txt"), Some("model=Lux-3"));
        assert_eq!(d.get("instance"), Some("Lux._http._tcp.local"));
    }

    #[test]
    fn missing_a_record_falls_back_to_the_sender() {
        let src: IpAddr = "192.168.1.50".parse().unwrap();
        let msg = parsed(&[record("i._x._tcp.local", TYPE_SRV, &srv(9, "ghost.local"))]);
        assert_eq!(devices_from(&[(src, msg)])[0].address, "192.168.1.50:9");
    }

    #[test]
    fn a_directory_reply_lists_service_types_per_source() {
        let src: IpAddr = "192.168.1.9".parse().unwrap();
        let msg = parsed(&[
            record(
                "_services._dns-sd._udp.local",
                TYPE_PTR,
                &name("_http._tcp.local"),
            ),
            record(
                "_services._dns-sd._udp.local",
                TYPE_PTR,
                &name("_osc._udp.local"),
            ),
        ]);
        let devices = devices_from(&[(src, msg)]);
        assert_eq!(devices.len(), 1);
        assert_eq!(devices[0].address, "192.168.1.9");
        let types: Vec<_> = devices[0]
            .details
            .iter()
            .filter(|(k, _)| k == "service_type")
            .map(|(_, v)| v.as_str())
            .collect();
        assert_eq!(types, vec!["_http._tcp.local", "_osc._udp.local"]);
    }

    #[test]
    fn listings_are_bounded() {
        let src: IpAddr = "192.168.1.9".parse().unwrap();
        let many: Vec<Vec<u8>> = (0..40)
            .map(|i| {
                record(
                    "_services._dns-sd._udp.local",
                    TYPE_PTR,
                    &name(&format!("_s{i}._tcp.local")),
                )
            })
            .collect();
        let devices = devices_from(&[(src, parsed(&many))]);
        assert_eq!(devices[0].details.len(), MAX_LISTED);
    }

    #[test]
    fn needs_an_interface_and_a_valid_service_name() {
        let cancel = crate::CancelFlag::new();
        let ctx = |interface| Ctx {
            hosts: &[],
            interface,
            timeout: Duration::from_millis(50),
            concurrency: 1,
            cancel: &cancel,
        };
        assert!(matches!(
            browse(&ctx(None), DEFAULT_SERVICE, GROUP, true),
            Err(DriverError::Config(_))
        ));
        assert!(matches!(
            browse(&ctx(Some(Ipv4Addr::LOCALHOST)), "bad name!", GROUP, true),
            Err(DriverError::Config(_))
        ));
    }

    #[test]
    fn browses_a_responder_over_loopback() {
        use std::sync::atomic::{AtomicBool, Ordering};
        use std::sync::Arc;
        let responder = UdpSocket::bind("127.0.0.1:0").unwrap();
        responder
            .set_read_timeout(Some(Duration::from_millis(20)))
            .unwrap();
        let addr = match responder.local_addr().unwrap() {
            SocketAddr::V4(a) => a,
            _ => unreachable!(),
        };
        let stop = Arc::new(AtomicBool::new(false));
        let flag = stop.clone();
        let queries = Arc::new(std::sync::Mutex::new(Vec::<Vec<u8>>::new()));
        let seen = queries.clone();
        let handle = std::thread::spawn(move || {
            let mut buf = [0u8; 2048];
            while !flag.load(Ordering::SeqCst) {
                let Ok((n, from)) = responder.recv_from(&mut buf) else {
                    continue;
                };
                seen.lock().unwrap().push(buf[..n].to_vec());
                let reply = message(&[
                    record("_http._tcp.local", TYPE_PTR, &name("Lux._http._tcp.local")),
                    record("Lux._http._tcp.local", TYPE_SRV, &srv(8080, "lux.local")),
                    record("lux.local", TYPE_A, &[10, 1, 2, 3]),
                ]);
                let _ = responder.send_to(&reply, from);
            }
        });

        let cancel = crate::CancelFlag::new();
        let ctx = Ctx {
            hosts: &[],
            interface: Some(Ipv4Addr::LOCALHOST),
            timeout: Duration::from_millis(400),
            concurrency: 1,
            cancel: &cancel,
        };
        let devices = browse(&ctx, "_http._tcp.local", addr, false).unwrap();
        stop.store(true, Ordering::SeqCst);
        handle.join().unwrap();

        assert_eq!(devices.len(), 1, "{devices:?}");
        assert_eq!(devices[0].address, "10.1.2.3:8080");
        // The query asks for a PTR with the unicast-response bit set.
        let q = queries.lock().unwrap()[0].clone();
        assert_eq!(&q[q.len() - 4..], &[0, 12, 0x80, 1]);
        // And it was sent more than once to survive a lost datagram.
        assert!(queries.lock().unwrap().len() >= 2);
    }

    #[test]
    fn garbage_and_non_responses_are_ignored() {
        use std::sync::atomic::{AtomicBool, Ordering};
        use std::sync::Arc;
        let responder = UdpSocket::bind("127.0.0.1:0").unwrap();
        responder
            .set_read_timeout(Some(Duration::from_millis(20)))
            .unwrap();
        let addr = match responder.local_addr().unwrap() {
            SocketAddr::V4(a) => a,
            _ => unreachable!(),
        };
        let stop = Arc::new(AtomicBool::new(false));
        let flag = stop.clone();
        let handle = std::thread::spawn(move || {
            let mut buf = [0u8; 2048];
            while !flag.load(Ordering::SeqCst) {
                let Ok((_, from)) = responder.recv_from(&mut buf) else {
                    continue;
                };
                let _ = responder.send_to(&[0xff; 40], from);
                // A query echoed back is not a response.
                let _ = responder.send_to(&buf[..30], from);
            }
        });
        let cancel = crate::CancelFlag::new();
        let ctx = Ctx {
            hosts: &[],
            interface: Some(Ipv4Addr::LOCALHOST),
            timeout: Duration::from_millis(250),
            concurrency: 1,
            cancel: &cancel,
        };
        let devices = browse(&ctx, DEFAULT_SERVICE, addr, false).unwrap();
        stop.store(true, Ordering::SeqCst);
        handle.join().unwrap();
        assert!(devices.is_empty());
    }
}
