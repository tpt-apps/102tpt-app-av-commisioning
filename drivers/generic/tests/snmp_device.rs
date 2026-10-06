//! End-to-end: the SNMP driver against a fake agent on loopback.
//!
//! The agent decodes requests with `snmp2`'s public parser and hand-encodes
//! GetResponse packets, so the client's decoder is exercised on bytes it did
//! not produce.

use std::collections::HashMap;
use std::net::{SocketAddr, UdpSocket};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::thread::{self, JoinHandle};
use std::time::Duration;

use snmp2::Pdu;
use tpt_app_av_commissioning_device::{DeviceCommand, StateValue};
use tpt_app_av_commissioning_driver::{DeviceDriver, DriverError};
use tpt_app_av_commissioning_driver_generic::snmp::{
    SnmpDriver, SnmpDriverConfig, SnmpQuery, SnmpVersion,
};
use tpt_app_av_commissioning_model::DeviceId;
use tpt_app_av_commissioning_profile::{DeviceProfile, ParserKind};
use tpt_app_av_commissioning_test::kinds;
use tpt_app_av_commissioning_test::{CommissioningTest, TestStatus};

// ---- minimal BER encoder for the fake agent --------------------------------

fn tlv(tag: u8, content: &[u8]) -> Vec<u8> {
    let mut out = vec![tag];
    if content.len() < 128 {
        out.push(content.len() as u8);
    } else {
        let len = content.len().to_be_bytes();
        let len: Vec<u8> = len.iter().copied().skip_while(|b| *b == 0).collect();
        out.push(0x80 | len.len() as u8);
        out.extend(len);
    }
    out.extend_from_slice(content);
    out
}

fn int_bytes(v: i64) -> Vec<u8> {
    let bytes = v.to_be_bytes();
    let mut start = 0;
    while start < 7
        && ((bytes[start] == 0 && bytes[start + 1] & 0x80 == 0)
            || (bytes[start] == 0xff && bytes[start + 1] & 0x80 != 0))
    {
        start += 1;
    }
    bytes[start..].to_vec()
}

fn oid_bytes(oid: &str) -> Vec<u8> {
    let arcs: Vec<u32> = oid.split('.').map(|a| a.parse().unwrap()).collect();
    let mut out = vec![(arcs[0] * 40 + arcs[1]) as u8];
    for &arc in &arcs[2..] {
        let mut groups = vec![(arc & 0x7f) as u8];
        let mut rest = arc >> 7;
        while rest > 0 {
            groups.push((rest & 0x7f) as u8 | 0x80);
            rest >>= 7;
        }
        groups.reverse();
        out.extend(groups);
    }
    out
}

#[derive(Clone)]
enum Val {
    Int(i64),
    Str(&'static [u8]),
    Gauge(u32),
    Ticks(u32),
    Missing,
}

fn encode_value(v: &Val) -> Vec<u8> {
    match v {
        Val::Int(i) => tlv(0x02, &int_bytes(*i)),
        Val::Str(s) => tlv(0x04, s),
        Val::Gauge(g) => tlv(0x42, &int_bytes(i64::from(*g))),
        Val::Ticks(t) => tlv(0x43, &int_bytes(i64::from(*t))),
        Val::Missing => vec![0x80, 0x00], // noSuchObject
    }
}

fn response(community: &[u8], req_id: i32, oid: &str, value: &Val) -> Vec<u8> {
    let varbind = tlv(
        0x30,
        &[tlv(0x06, &oid_bytes(oid)), encode_value(value)].concat(),
    );
    let pdu = tlv(
        0xA2,
        &[
            tlv(0x02, &req_id.to_be_bytes()),
            tlv(0x02, &[0]),
            tlv(0x02, &[0]),
            tlv(0x30, &varbind),
        ]
        .concat(),
    );
    tlv(0x30, &[tlv(0x02, &[1]), tlv(0x04, community), pdu].concat())
}

// ---- the fake agent ---------------------------------------------------------

struct FakeAgent {
    addr: SocketAddr,
    stop: Arc<AtomicBool>,
    handle: Option<JoinHandle<()>>,
}

impl FakeAgent {
    /// `community` is the one the agent accepts; others get no reply, as real
    /// agents do.
    fn start(community: &'static str, mib: Vec<(&'static str, Val)>, garbage: bool) -> Self {
        let socket = UdpSocket::bind("127.0.0.1:0").unwrap();
        socket
            .set_read_timeout(Some(Duration::from_millis(20)))
            .unwrap();
        let addr = socket.local_addr().unwrap();
        let stop = Arc::new(AtomicBool::new(false));
        let flag = stop.clone();
        let mib: HashMap<&str, Val> = mib.into_iter().collect();
        let handle = thread::spawn(move || {
            let mut buf = vec![0u8; 65_507];
            while !flag.load(Ordering::SeqCst) {
                let Ok((n, from)) = socket.recv_from(&mut buf) else {
                    continue;
                };
                let Ok(pdu) = Pdu::from_bytes(&buf[..n]) else {
                    continue;
                };
                if pdu.community != community.as_bytes() {
                    continue;
                }
                if garbage {
                    let _ = socket.send_to(&[0x30, 0x03, 0xff, 0xff, 0xff], from);
                    continue;
                }
                let req_id = pdu.req_id;
                let mut varbinds = pdu.varbinds;
                let Some((oid, _)) = varbinds.next() else {
                    continue;
                };
                let oid = oid.to_string();
                let value = mib.get(oid.as_str()).cloned().unwrap_or(Val::Missing);
                let _ = socket.send_to(&response(community.as_bytes(), req_id, &oid, &value), from);
            }
        });
        Self {
            addr,
            stop,
            handle: Some(handle),
        }
    }
}

impl Drop for FakeAgent {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::SeqCst);
        if let Some(h) = self.handle.take() {
            let _ = h.join();
        }
    }
}

const SYS_DESCR: &str = "1.3.6.1.2.1.1.1.0";
const SYS_UPTIME: &str = "1.3.6.1.2.1.1.3.0";
const IF_STATUS: &str = "1.3.6.1.2.1.2.2.1.8.1";
const LAMP_HOURS: &str = "1.3.6.1.4.1.99999.1.1.0";
const POWER: &str = "1.3.6.1.4.1.99999.1.2.0";
const BIG_ID: &str = "1.3.6.1.4.1.99999.300000.1";

fn mib() -> Vec<(&'static str, Val)> {
    vec![
        (SYS_DESCR, Val::Str(b"Lux Projector fw 3.2.1")),
        (SYS_UPTIME, Val::Ticks(123_456)),
        (IF_STATUS, Val::Int(1)),
        (LAMP_HOURS, Val::Gauge(4_000_000_000)),
        (POWER, Val::Str(b"standby")),
        (BIG_ID, Val::Int(-129)),
    ]
}

fn config(target: SocketAddr, community: &str) -> SnmpDriverConfig {
    SnmpDriverConfig::new(target)
        .timeout(Duration::from_millis(300))
        .community(community)
        .query(SnmpQuery::new("firmware", SYS_DESCR).unwrap())
        .query(SnmpQuery::new("uptime_ticks", SYS_UPTIME).unwrap())
        .query(SnmpQuery::new("link_state", IF_STATUS).unwrap())
        .query(SnmpQuery::new("lamp_hours", LAMP_HOURS).unwrap())
        .query(
            SnmpQuery::new("power", POWER)
                .unwrap()
                .parser(ParserKind::PowerState),
        )
        .query(SnmpQuery::new("signed_multi_arc", BIG_ID).unwrap())
}

#[test]
fn values_are_typed() {
    let agent = FakeAgent::start("public", mib(), false);
    let mut d = SnmpDriver::new(config(agent.addr, "public")).unwrap();
    let s = d.get_state().unwrap();
    assert_eq!(
        s.get("firmware"),
        Some(&StateValue::Text("Lux Projector fw 3.2.1".into()))
    );
    assert_eq!(s.get("uptime_ticks"), Some(&StateValue::Integer(123_456)));
    assert_eq!(s.get("link_state"), Some(&StateValue::Integer(1)));
    // A gauge above i32::MAX is still an integer.
    assert_eq!(
        s.get("lamp_hours"),
        Some(&StateValue::Integer(4_000_000_000))
    );
    assert_eq!(s.get("power"), Some(&StateValue::Boolean(false)));
    assert_eq!(s.get("signed_multi_arc"), Some(&StateValue::Integer(-129)));
    assert!(d.capabilities().can_read_state && !d.capabilities().can_power_on);
}

#[test]
fn a_missing_oid_is_not_reported_instead_of_invented() {
    let agent = FakeAgent::start("public", mib(), false);
    let mut d = SnmpDriver::new(
        config(agent.addr, "public")
            .query(SnmpQuery::new("temp", "1.3.6.1.4.1.99999.9.9.0").unwrap()),
    )
    .unwrap();
    let s = d.get_state().unwrap();
    assert!(s.get("temp").is_none());
    assert!(s.get("firmware").is_some());
}

#[test]
fn wrong_community_looks_like_a_timeout() {
    // Agents silently drop requests with a bad community; the driver can only
    // observe the silence.
    let agent = FakeAgent::start("secret", mib(), false);
    let mut d = SnmpDriver::new(config(agent.addr, "public")).unwrap();
    assert!(matches!(d.get_state(), Err(DriverError::Timeout(_))));
    let mut d = SnmpDriver::new(config(agent.addr, "secret")).unwrap();
    assert!(d.get_state().is_ok());
}

#[test]
fn garbage_replies_are_errors_not_crashes() {
    let agent = FakeAgent::start("public", mib(), true);
    let mut d = SnmpDriver::new(config(agent.addr, "public")).unwrap();
    assert!(d.get_state().is_err());
}

#[test]
fn commands_are_never_sent() {
    let agent = FakeAgent::start("public", mib(), false);
    let mut d = SnmpDriver::new(config(agent.addr, "public")).unwrap();
    for c in [
        DeviceCommand::PowerOn,
        DeviceCommand::SetInput { input: "x".into() },
        DeviceCommand::Arbitrary {
            command: "1.3.6.1.2.1.1.5.0 s evil".into(),
        },
    ] {
        assert_eq!(d.execute(c), Err(DriverError::UnsupportedOperation));
    }
}

#[test]
fn v1_agents_work_too() {
    let agent = FakeAgent::start("public", mib(), false);
    let mut d = SnmpDriver::new(config(agent.addr, "public").version(SnmpVersion::V1)).unwrap();
    assert!(d.get_state().unwrap().get("firmware").is_some());
}

#[test]
fn silent_agent_times_out() {
    let silent = UdpSocket::bind("127.0.0.1:0").unwrap();
    let mut d = SnmpDriver::new(config(silent.local_addr().unwrap(), "public")).unwrap();
    assert!(matches!(d.get_state(), Err(DriverError::Timeout(_))));
}

const PROFILE: &str = r#"
schema_version: 1
device:
  id: snmp-projector
  match: { manufacturer: SnmpCo, model: S-1 }
  protocol: { type: snmp, port: 161, timeout_ms: 300 }
  state:
    firmware: { query: "1.3.6.1.2.1.1.1.0" }
    link_state: { query: "1.3.6.1.2.1.2.2.1.8.1" }
    power: { query: "1.3.6.1.4.1.99999.1.2.0", parser: power_state }
"#;

#[test]
fn profile_built_driver_runs_phase9_tests() {
    let profile = DeviceProfile::from_yaml_str(PROFILE).unwrap();
    let agent = FakeAgent::start("public", mib(), false);
    let mut cfg = SnmpDriverConfig::from_profile(&profile, agent.addr.ip()).unwrap();
    assert_eq!(cfg.target.port(), 161);
    cfg.target = agent.addr;
    let drv = kinds::share(SnmpDriver::new(cfg).unwrap());
    let t = kinds::power_state("p", DeviceId::new("s"), drv.clone(), false);
    assert_eq!(t.execute().unwrap().status, TestStatus::Pass);
    let t = kinds::power_state("p2", DeviceId::new("s"), drv, true);
    assert_eq!(t.execute().unwrap().status, TestStatus::Fail);
}

#[test]
fn profiles_with_commands_or_bad_oids_are_refused() {
    let with_cmd = PROFILE.replace(
        "  state:",
        "  commands:\n    power_on: { send: \"1.3.6.1.2.1.1.5.0\" }\n  state:",
    );
    assert!(DeviceProfile::from_yaml_str(&with_cmd).is_err());
    let bad_oid = PROFILE.replace("1.3.6.1.2.1.1.1.0", "not-an-oid");
    assert!(DeviceProfile::from_yaml_str(&bad_oid).is_err());
    let tcp = PROFILE.replace("type: snmp", "type: tcp");
    let p = DeviceProfile::from_yaml_str(&tcp);
    // (a tcp profile with OID queries is legal text; the SNMP driver refuses it)
    if let Ok(p) = p {
        assert!(SnmpDriverConfig::from_profile(&p, "10.0.0.5".parse().unwrap()).is_err());
    }
}
