//! Discovery end to end on loopback: the unicast mechanisms against fake
//! devices, plus the orchestrator's safety and reporting behaviour.

use std::io::Write;
use std::net::{SocketAddr, TcpListener, UdpSocket};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::thread::{self, JoinHandle};
use std::time::Duration;

use tpt_app_av_commissioning_discovery::{
    list_interfaces, match_profiles, run, CancelFlag, NetworkDiscovery,
};
use tpt_app_av_commissioning_driver::{
    DeviceDiscovery, DiscoveredDevice, DiscoveryConfig, DiscoveryProtocol, DiscoveryScope,
    DriverError,
};
use tpt_app_av_commissioning_profile::DeviceProfile;
use tpt_av_control_osc::{OscArg, OscMessage};

fn host(ip: &str) -> DiscoveryScope {
    DiscoveryScope::Host(ip.parse().unwrap())
}

fn tcp(ports: &[u16], banner: bool) -> DiscoveryProtocol {
    DiscoveryProtocol::TcpProbe {
        ports: ports.to_vec(),
        read_banner: banner,
    }
}

fn closed_port() -> u16 {
    TcpListener::bind("127.0.0.1:0")
        .unwrap()
        .local_addr()
        .unwrap()
        .port()
}

// ---- TCP probe ---------------------------------------------------------------

#[test]
fn tcp_probe_reports_only_open_ports() {
    let open = TcpListener::bind("127.0.0.1:0").unwrap();
    let open_port = open.local_addr().unwrap().port();
    let shut = closed_port();
    let config = DiscoveryConfig::new(host("127.0.0.1"))
        .protocol(tcp(&[open_port, shut], false))
        .timeout_ms(400);
    let report = run(&config, &CancelFlag::new()).unwrap();
    assert_eq!(report.devices.len(), 1, "{:?}", report.devices);
    let d = &report.devices[0];
    assert_eq!(d.address, format!("127.0.0.1:{open_port}"));
    assert_eq!(d.source, "tcp_probe");
    assert_eq!(d.get("port"), Some(open_port.to_string().as_str()));
    assert!(d.get("connect_ms").is_some());
    assert!(d.get("banner").is_none(), "no banner unless asked");
    assert_eq!(report.hosts_in_scope, 1);
    assert!(report.errors.is_empty() && !report.cancelled);
}

#[test]
fn tcp_probe_sends_nothing_and_banners_are_sanitised() {
    // The listener records anything the prober sends.
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    let handle = thread::spawn(move || {
        let (mut stream, _) = listener.accept().unwrap();
        stream.write_all(b"PJLINK 0\x1b[31m\r\n").unwrap();
        stream
            .set_read_timeout(Some(Duration::from_millis(300)))
            .unwrap();
        let mut buf = [0u8; 64];
        // Nothing may arrive; EOF (0) or a timeout are both fine.
        std::io::Read::read(&mut stream, &mut buf).unwrap_or(0)
    });
    let config = DiscoveryConfig::new(host("127.0.0.1"))
        .protocol(tcp(&[port], true))
        .timeout_ms(500);
    let report = run(&config, &CancelFlag::new()).unwrap();
    assert_eq!(handle.join().unwrap(), 0, "the probe must not send data");
    let banner = report.devices[0].get("banner").unwrap();
    assert!(banner.starts_with("PJLINK 0"));
    assert!(!banner.contains('\u{1b}'));
}

#[test]
fn a_cidr_scope_expands_to_its_hosts() {
    let open = TcpListener::bind("127.0.0.1:0").unwrap();
    let port = open.local_addr().unwrap().port();
    let config = DiscoveryConfig::new(DiscoveryScope::Cidr("127.0.0.0/30".into()))
        .protocol(tcp(&[port], false))
        .timeout_ms(300);
    let report = run(&config, &CancelFlag::new()).unwrap();
    assert_eq!(report.hosts_in_scope, 2); // .1 and .2
    assert!(report
        .devices
        .iter()
        .any(|d| d.address == format!("127.0.0.1:{port}")));
}

// ---- OSC probe ------------------------------------------------------------------

struct Thread(Arc<AtomicBool>, Option<JoinHandle<()>>);

impl Drop for Thread {
    fn drop(&mut self) {
        self.0.store(true, Ordering::SeqCst);
        if let Some(h) = self.1.take() {
            let _ = h.join();
        }
    }
}

fn udp_responder(
    handler: impl Fn(&[u8]) -> Option<Vec<u8>> + Send + 'static,
) -> (SocketAddr, Thread) {
    let socket = UdpSocket::bind("127.0.0.1:0").unwrap();
    socket
        .set_read_timeout(Some(Duration::from_millis(20)))
        .unwrap();
    let addr = socket.local_addr().unwrap();
    let stop = Arc::new(AtomicBool::new(false));
    let flag = stop.clone();
    let handle = thread::spawn(move || {
        let mut buf = vec![0u8; 65_507];
        while !flag.load(Ordering::SeqCst) {
            let Ok((n, from)) = socket.recv_from(&mut buf) else {
                continue;
            };
            if let Some(reply) = handler(&buf[..n]) {
                let _ = socket.send_to(&reply, from);
            }
        }
    });
    (addr, Thread(stop, Some(handle)))
}

#[test]
fn osc_probe_finds_devices_that_answer_an_osc_query() {
    let (addr, _t) = udp_responder(|data| {
        let q = OscMessage::decode(data).ok()?;
        assert!(q.arguments.is_empty(), "the query carries no arguments");
        Some(
            OscMessage::new_unchecked(q.address, vec![OscArg::String("Beam-1 v3".into())]).encode(),
        )
    });
    let config = DiscoveryConfig::new(host("127.0.0.1"))
        .protocol(DiscoveryProtocol::Osc {
            ports: vec![addr.port()],
            address: None,
        })
        .timeout_ms(400);
    let report = run(&config, &CancelFlag::new()).unwrap();
    assert_eq!(report.devices.len(), 1);
    let d = &report.devices[0];
    assert_eq!(d.source, "osc");
    assert_eq!(d.get("osc_reply_address"), Some("/info"));
    assert_eq!(d.get("osc_reply_value"), Some("Beam-1 v3"));
}

#[test]
fn osc_probe_ignores_non_osc_replies_and_silence() {
    let (junk, _a) = udp_responder(|_| Some(vec![0xff; 20]));
    let silent = UdpSocket::bind("127.0.0.1:0").unwrap();
    let config = DiscoveryConfig::new(host("127.0.0.1"))
        .protocol(DiscoveryProtocol::Osc {
            ports: vec![junk.port(), silent.local_addr().unwrap().port()],
            address: None,
        })
        .timeout_ms(250);
    assert!(run(&config, &CancelFlag::new()).unwrap().devices.is_empty());
}

#[test]
fn osc_probe_rejects_wildcard_addresses() {
    let config = DiscoveryConfig::new(host("127.0.0.1")).protocol(DiscoveryProtocol::Osc {
        ports: vec![9000],
        address: Some("/*".into()),
    });
    let report = run(&config, &CancelFlag::new()).unwrap();
    assert!(report.devices.is_empty());
    assert!(matches!(report.errors[0].1, DriverError::Config(_)));
}

// ---- SNMP probe -----------------------------------------------------------------

fn tlv(tag: u8, content: &[u8]) -> Vec<u8> {
    let mut out = vec![tag];
    out.push(content.len() as u8); // all test packets are < 128 bytes
    out.extend_from_slice(content);
    out
}

fn oid_bytes(oid: &str) -> Vec<u8> {
    let arcs: Vec<u32> = oid.split('.').map(|a| a.parse().unwrap()).collect();
    let mut out = vec![(arcs[0] * 40 + arcs[1]) as u8];
    out.extend(arcs[2..].iter().map(|a| *a as u8)); // all arcs < 128 here
    out
}

fn snmp_agent(community: &'static str) -> (SocketAddr, Thread) {
    udp_responder(move |data| {
        let mut pdu = snmp2::Pdu::from_bytes(data).ok()?;
        if pdu.community != community.as_bytes() {
            return None;
        }
        let req_id = pdu.req_id;
        let (oid, _) = pdu.varbinds.next()?;
        let oid = oid.to_string();
        let value = match oid.as_str() {
            "1.3.6.1.2.1.1.1.0" => tlv(0x04, b"Lux Projector fw 3.2.1"),
            "1.3.6.1.2.1.1.5.0" => tlv(0x04, b"lux-boardroom"),
            "1.3.6.1.2.1.1.2.0" => tlv(0x06, &oid_bytes("1.3.6.1.4.1.99")),
            _ => vec![0x80, 0x00],
        };
        let varbind = tlv(0x30, &[tlv(0x06, &oid_bytes(&oid)), value].concat());
        let body = tlv(
            0xA2,
            &[
                tlv(0x02, &req_id.to_be_bytes()),
                tlv(0x02, &[0]),
                tlv(0x02, &[0]),
                tlv(0x30, &varbind),
            ]
            .concat(),
        );
        Some(tlv(
            0x30,
            &[tlv(0x02, &[1]), tlv(0x04, community.as_bytes()), body].concat(),
        ))
    })
}

#[test]
fn snmp_probe_identifies_agents() {
    let (addr, _t) = snmp_agent("public");
    let config = DiscoveryConfig::new(host("127.0.0.1"))
        .protocol(DiscoveryProtocol::Snmp {
            community: None,
            port: Some(addr.port()),
        })
        .timeout_ms(400);
    let report = run(&config, &CancelFlag::new()).unwrap();
    assert_eq!(report.devices.len(), 1, "{:?}", report);
    let d = &report.devices[0];
    assert_eq!(d.source, "snmp");
    assert_eq!(d.get("sys_descr"), Some("Lux Projector fw 3.2.1"));
    assert_eq!(d.get("sys_name"), Some("lux-boardroom"));
    assert_eq!(d.get("sys_object_id"), Some("1.3.6.1.4.1.99"));
}

#[test]
fn snmp_probe_with_the_wrong_community_finds_nothing() {
    let (addr, _t) = snmp_agent("secret");
    let config = DiscoveryConfig::new(host("127.0.0.1"))
        .protocol(DiscoveryProtocol::Snmp {
            community: Some("public".into()),
            port: Some(addr.port()),
        })
        .timeout_ms(250);
    assert!(run(&config, &CancelFlag::new()).unwrap().devices.is_empty());
    let right = DiscoveryConfig::new(host("127.0.0.1"))
        .protocol(DiscoveryProtocol::Snmp {
            community: Some("secret".into()),
            port: Some(addr.port()),
        })
        .timeout_ms(400);
    assert_eq!(run(&right, &CancelFlag::new()).unwrap().devices.len(), 1);
}

// ---- orchestrator -----------------------------------------------------------------

#[test]
fn invalid_configs_send_nothing() {
    // A packet-counting listener on the target port proves nothing was sent.
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    listener.set_nonblocking(true).unwrap();
    let port = listener.local_addr().unwrap().port();
    let bad = [
        // no protocol selected
        DiscoveryConfig::new(host("127.0.0.1")),
        // public address without opt-in
        DiscoveryConfig::new(host("8.8.8.8")).protocol(tcp(&[port], false)),
        // protocol/scope mismatch
        DiscoveryConfig::new(host("127.0.0.1")).protocol(DiscoveryProtocol::Midi),
        // multicast without an interface
        DiscoveryConfig::new(DiscoveryScope::Multicast)
            .protocol(DiscoveryProtocol::Mdns { service: None }),
        // too many hosts
        DiscoveryConfig::new(DiscoveryScope::Cidr("10.0.0.0/16".into()))
            .protocol(tcp(&[80], false)),
    ];
    for config in bad {
        assert!(
            matches!(
                run(&config, &CancelFlag::new()),
                Err(DriverError::Config(_))
            ),
            "{config:?}"
        );
    }
    assert!(
        listener.accept().is_err(),
        "no connection may have been made"
    );
}

#[test]
fn an_interface_that_is_not_on_this_machine_is_refused() {
    let config = DiscoveryConfig::new(DiscoveryScope::Multicast)
        .protocol(DiscoveryProtocol::Mdns { service: None })
        .interface("203.0.113.77");
    assert!(matches!(
        run(&config, &CancelFlag::new()),
        Err(DriverError::Config(_))
    ));
}

#[test]
fn a_real_interface_is_accepted_for_multicast_and_a_quiet_network_finds_nothing() {
    let lo = list_interfaces()
        .unwrap()
        .into_iter()
        .find(|i| i.is_loopback)
        .unwrap();
    let config = DiscoveryConfig::new(DiscoveryScope::Multicast)
        .protocol(DiscoveryProtocol::Mdns { service: None })
        .protocol(DiscoveryProtocol::Ssdp {
            search_target: None,
        })
        .interface(lo.address.to_string())
        .timeout_ms(200);
    let report = run(&config, &CancelFlag::new()).unwrap();
    // Loopback has no multicast peers: nothing found. A platform that cannot
    // multicast on loopback reports per-mechanism errors rather than failing
    // the whole pass.
    assert!(report.devices.is_empty());
    for (name, _) in &report.errors {
        assert!(name == "mdns" || name == "ssdp");
    }
}

#[test]
fn local_mechanisms_return_devices_or_errors_never_panics() {
    let config = DiscoveryConfig::new(DiscoveryScope::Local)
        .protocol(DiscoveryProtocol::Midi)
        .protocol(DiscoveryProtocol::Serial)
        .protocol(DiscoveryProtocol::Audio);
    let report = run(&config, &CancelFlag::new()).unwrap();
    for d in &report.devices {
        assert!(["midi", "serial", "audio"].contains(&d.source.as_str()));
        assert!(!d.address.is_empty());
        assert!(!d.address.chars().any(char::is_control));
    }
    for (name, _) in &report.errors {
        assert!(["midi", "serial", "audio"].contains(&name.as_str()));
    }
    assert_eq!(report.hosts_in_scope, 0);
}

#[test]
fn one_failing_mechanism_does_not_hide_the_others() {
    let open = TcpListener::bind("127.0.0.1:0").unwrap();
    let port = open.local_addr().unwrap().port();
    let config = DiscoveryConfig::new(host("127.0.0.1"))
        .protocol(DiscoveryProtocol::Snmp {
            community: Some(String::new()), // invalid: fails
            port: None,
        })
        .protocol(tcp(&[port], false))
        .timeout_ms(300);
    let report = run(&config, &CancelFlag::new()).unwrap();
    assert_eq!(report.devices.len(), 1);
    assert_eq!(report.errors.len(), 1);
    assert_eq!(report.errors[0].0, "snmp");
    // Through the trait the partial result is returned, not an error.
    assert_eq!(NetworkDiscovery.discover(&config).unwrap().len(), 1);
}

#[test]
fn if_every_mechanism_fails_the_trait_reports_the_error() {
    let config = DiscoveryConfig::new(host("127.0.0.1")).protocol(DiscoveryProtocol::Snmp {
        community: Some(String::new()),
        port: None,
    });
    assert!(matches!(
        NetworkDiscovery.discover(&config),
        Err(DriverError::Config(_))
    ));
    assert_eq!(NetworkDiscovery.method_name(), "network");
}

#[test]
fn results_are_deduplicated_and_ordered() {
    let a = TcpListener::bind("127.0.0.1:0").unwrap();
    let b = TcpListener::bind("127.0.0.1:0").unwrap();
    let (pa, pb) = (
        a.local_addr().unwrap().port(),
        b.local_addr().unwrap().port(),
    );
    // The same mechanism listed twice must not double-report.
    let config = DiscoveryConfig::new(host("127.0.0.1"))
        .protocol(tcp(&[pb, pa], false))
        .protocol(tcp(&[pa], false))
        .timeout_ms(300);
    let report = run(&config, &CancelFlag::new()).unwrap();
    assert_eq!(report.devices.len(), 2);
    let addrs: Vec<&str> = report.devices.iter().map(|d| d.address.as_str()).collect();
    let mut sorted = addrs.clone();
    sorted.sort();
    assert_eq!(addrs, sorted);
}

#[test]
fn cancellation_stops_a_pass() {
    let cancel = CancelFlag::new();
    cancel.cancel();
    let config = DiscoveryConfig::new(host("127.0.0.1")).protocol(tcp(&[1], false));
    let report = run(&config, &cancel).unwrap();
    assert!(report.cancelled && report.devices.is_empty());
}

// ---- profile matching ---------------------------------------------------------------

#[test]
fn candidates_match_profiles_only_on_reported_identity() {
    let profile = DeviceProfile::from_yaml_str(
        "schema_version: 1\ndevice:\n  id: p\n  match: { manufacturer: Acme, model: Beam-1 }\n  protocol: { type: osc, port: 9000 }\n  state:\n    power: { query: /power }\n",
    )
    .unwrap();
    let profiles = [profile];

    let mut device = DiscoveredDevice::new("10.0.0.5:9000", "osc");
    assert!(
        match_profiles(&device, &profiles).is_empty(),
        "no identity, no match"
    );

    device.identity.manufacturer = Some("ACME".into());
    device.identity.model = Some("beam-1".into());
    assert_eq!(match_profiles(&device, &profiles).len(), 1);

    device.identity.model = Some("Beam-2".into());
    assert!(match_profiles(&device, &profiles).is_empty());
}
