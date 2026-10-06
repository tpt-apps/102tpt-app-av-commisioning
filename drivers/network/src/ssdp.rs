//! SSDP (UPnP) search on one selected interface.
//!
//! Sends one `M-SEARCH` (twice) to 239.255.255.250:1900 from the chosen
//! interface and collects the unicast replies. Replies are untrusted: only
//! `HTTP/1.1 200` responses are read, at most 2 KiB each and 256 per pass,
//! header values are reduced to printable ASCII and length-capped, and the
//! `LOCATION` URL is recorded but never fetched.
//!
//! A device often answers once per service it offers, so replies are grouped
//! by sender: one device per source address, with a `st` detail for each
//! search target it answered.

use std::collections::BTreeMap;
use std::net::{IpAddr, Ipv4Addr, SocketAddr, SocketAddrV4, UdpSocket};
use std::time::{Duration, Instant};

use tpt_app_av_commissioning_driver::{DiscoveredDevice, DriverError};

use crate::util::{sanitize, Ctx};

/// The SSDP multicast group.
pub(crate) const GROUP: SocketAddrV4 = SocketAddrV4::new(Ipv4Addr::new(239, 255, 255, 250), 1900);
pub(crate) const DEFAULT_TARGET: &str = "ssdp:all";
const MAX_DATAGRAMS: usize = 256;
const MAX_DATAGRAM: usize = 2048;
const MAX_HEADERS: usize = 32;
const MAX_LISTED: usize = 16;

/// A search target may not carry anything that could end the header line.
fn validate_target(target: &str) -> Result<(), DriverError> {
    if target.is_empty()
        || target.len() > 128
        || !target
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b":._-".contains(&b))
    {
        return Err(DriverError::Config(format!(
            "invalid SSDP search target {target:?} (letters, digits and : . _ - only)"
        )));
    }
    Ok(())
}

fn search_request(target: &str) -> Vec<u8> {
    format!(
        "M-SEARCH * HTTP/1.1\r\nHOST: 239.255.255.250:1900\r\nMAN: \"ssdp:discover\"\r\nMX: 1\r\nST: {target}\r\nUSER-AGENT: tpt-av-commissioning\r\n\r\n"
    )
    .into_bytes()
}

/// The headers of a `200 OK` search response, lowercased names.
pub(crate) fn parse_response(data: &[u8]) -> Option<BTreeMap<String, String>> {
    let text = std::str::from_utf8(data).ok()?;
    let mut lines = text.split("\r\n");
    let status = lines.next()?;
    if !(status.starts_with("HTTP/1.1 200") || status.starts_with("HTTP/1.0 200")) {
        return None;
    }
    let mut headers = BTreeMap::new();
    for line in lines.take(MAX_HEADERS) {
        if line.is_empty() {
            break;
        }
        if let Some((k, v)) = line.split_once(':') {
            headers
                .entry(k.trim().to_ascii_lowercase())
                .or_insert_with(|| sanitize(v, 256));
        }
    }
    Some(headers)
}

pub(crate) fn search(
    ctx: &Ctx<'_>,
    target: &str,
    group: SocketAddrV4,
    multicast: bool,
) -> Result<Vec<DiscoveredDevice>, DriverError> {
    let interface = ctx.interface.ok_or_else(|| {
        DriverError::Config("SSDP needs an explicit network interface".to_owned())
    })?;
    validate_target(target)?;
    let request = search_request(target);

    let io = |e: std::io::Error| DriverError::Protocol(format!("SSDP socket error: {e}"));
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
        socket.set_multicast_ttl_v4(2).map_err(io)?;
        socket.set_multicast_loop_v4(false).map_err(io)?;
    }
    let socket: UdpSocket = socket.into();

    let mut by_source: BTreeMap<IpAddr, DiscoveredDevice> = BTreeMap::new();
    let started = Instant::now();
    let deadline = started + ctx.timeout;
    let mut sent = 0;
    let mut datagrams = 0;
    let mut buf = vec![0u8; MAX_DATAGRAM];
    while Instant::now() < deadline && datagrams < MAX_DATAGRAMS && !ctx.cancel.is_cancelled() {
        if sent < 2 && started.elapsed() >= ctx.timeout / 3 * sent {
            socket.send_to(&request, group).map_err(io)?;
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
                let Some(headers) = parse_response(&buf[..n]) else {
                    continue;
                };
                let device = by_source
                    .entry(from.ip())
                    .or_insert_with(|| DiscoveredDevice::new(from.ip().to_string(), "ssdp"));
                if let Some(st) = headers.get("st") {
                    let listed = device.details.iter().filter(|(k, _)| k == "st").count();
                    if listed < MAX_LISTED
                        && !device.details.iter().any(|(k, v)| k == "st" && v == st)
                    {
                        device.details.push(("st".into(), st.clone()));
                    }
                }
                for key in ["server", "location", "usn"] {
                    if let Some(v) = headers.get(key) {
                        if device.get(key).is_none() {
                            device.details.push((key.into(), v.clone()));
                        }
                    }
                }
            }
            Err(e)
                if matches!(
                    e.kind(),
                    std::io::ErrorKind::WouldBlock
                        | std::io::ErrorKind::TimedOut
                        | std::io::ErrorKind::ConnectionReset
                ) => {}
            Err(e) => return Err(io(e)),
        }
    }
    Ok(by_source.into_values().collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_search_responses_defensively() {
        let ok = b"HTTP/1.1 200 OK\r\nCACHE-CONTROL: max-age=1800\r\nST: upnp:rootdevice\r\nSERVER: Linux/5 UPnP/1.1 Lux/3\x00\x1b[0m\r\nLOCATION: http://10.0.0.5:80/desc.xml\r\nUSN: uuid:abc::upnp:rootdevice\r\n\r\n";
        let h = parse_response(ok).unwrap();
        assert_eq!(h["st"], "upnp:rootdevice");
        assert_eq!(h["location"], "http://10.0.0.5:80/desc.xml");
        assert!(!h["server"].contains('\u{1b}') && !h["server"].contains('\0'));
        // Notifications, errors and junk are not search responses.
        assert!(parse_response(b"NOTIFY * HTTP/1.1\r\nST: x\r\n\r\n").is_none());
        assert!(parse_response(b"HTTP/1.1 404 Not Found\r\n\r\n").is_none());
        assert!(parse_response(&[0xff, 0xfe]).is_none());
        assert!(parse_response(b"").is_none());
    }

    #[test]
    fn header_count_and_length_are_bounded() {
        let mut big = String::from("HTTP/1.1 200 OK\r\n");
        for i in 0..200 {
            big.push_str(&format!("X-{i}: v\r\n"));
        }
        big.push_str("\r\n");
        assert!(parse_response(big.as_bytes()).unwrap().len() <= MAX_HEADERS);
        let long = format!("HTTP/1.1 200 OK\r\nSERVER: {}\r\n\r\n", "A".repeat(1000));
        assert_eq!(
            parse_response(long.as_bytes()).unwrap()["server"].len(),
            256
        );
    }

    #[test]
    fn the_search_target_cannot_inject_headers() {
        for ok in [
            "ssdp:all",
            "upnp:rootdevice",
            "urn:schemas-upnp-org:device:MediaRenderer:1",
        ] {
            assert!(validate_target(ok).is_ok(), "{ok}");
        }
        for bad in ["", "a b", "x\r\nHOST: evil", "a\"b", &"a".repeat(129)] {
            assert!(validate_target(bad).is_err(), "{bad:?}");
        }
        let req = String::from_utf8(search_request("ssdp:all")).unwrap();
        assert!(req.starts_with("M-SEARCH * HTTP/1.1\r\n") && req.contains("ST: ssdp:all\r\n"));
    }

    #[test]
    fn needs_an_interface() {
        let cancel = crate::CancelFlag::new();
        let ctx = Ctx {
            hosts: &[],
            interface: None,
            timeout: Duration::from_millis(50),
            concurrency: 1,
            cancel: &cancel,
        };
        assert!(matches!(
            search(&ctx, DEFAULT_TARGET, GROUP, true),
            Err(DriverError::Config(_))
        ));
    }

    #[test]
    fn groups_replies_by_sender_over_loopback() {
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
        let requests = Arc::new(std::sync::Mutex::new(Vec::<String>::new()));
        let seen = requests.clone();
        let handle = std::thread::spawn(move || {
            let mut buf = [0u8; 2048];
            while !flag.load(Ordering::SeqCst) {
                let Ok((n, from)) = responder.recv_from(&mut buf) else {
                    continue;
                };
                seen.lock()
                    .unwrap()
                    .push(String::from_utf8_lossy(&buf[..n]).to_string());
                for st in ["upnp:rootdevice", "urn:x:service:Control:1"] {
                    let reply = format!(
                        "HTTP/1.1 200 OK\r\nST: {st}\r\nSERVER: Lux/3\r\nLOCATION: http://127.0.0.1:80/d.xml\r\n\r\n"
                    );
                    let _ = responder.send_to(reply.as_bytes(), from);
                }
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
        let devices = search(&ctx, "upnp:rootdevice", addr, false).unwrap();
        stop.store(true, Ordering::SeqCst);
        handle.join().unwrap();

        // Two services, one device.
        assert_eq!(devices.len(), 1, "{devices:?}");
        let d = &devices[0];
        assert_eq!(d.address, "127.0.0.1");
        let sts: Vec<_> = d.details.iter().filter(|(k, _)| k == "st").collect();
        assert_eq!(sts.len(), 2);
        assert_eq!(d.get("server"), Some("Lux/3"));
        assert_eq!(d.get("location"), Some("http://127.0.0.1:80/d.xml"));
        let first = requests.lock().unwrap()[0].clone();
        assert!(first.contains("ST: upnp:rootdevice\r\n"));
    }
}
