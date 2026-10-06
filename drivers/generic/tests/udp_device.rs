//! End-to-end: the UDP driver against a fake datagram device on loopback.

use std::net::{SocketAddr, UdpSocket};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::thread::{self, JoinHandle};
use std::time::Duration;

use tpt_app_av_commissioning_device::{DeviceCommand, StateValue};
use tpt_app_av_commissioning_driver::{DeviceDriver, DriverError};
use tpt_app_av_commissioning_driver_generic::text::MAX_LINE_BYTES;
use tpt_app_av_commissioning_driver_generic::{TextCommand, TextQuery, UdpDriver, UdpDriverConfig};
use tpt_app_av_commissioning_model::DeviceId;
use tpt_app_av_commissioning_profile::{DeviceProfile, ParserKind};
use tpt_app_av_commissioning_test::kinds;
use tpt_app_av_commissioning_test::{CommissioningTest, TestStatus};

#[derive(Clone, Copy, PartialEq)]
enum Behaviour {
    Normal,
    NotUtf8,
    Oversize,
}

struct FakeDevice {
    addr: SocketAddr,
    stop: Arc<AtomicBool>,
    handle: Option<JoinHandle<()>>,
    /// Raw datagrams received.
    received: Arc<Mutex<Vec<Vec<u8>>>>,
}

impl FakeDevice {
    fn start(behaviour: Behaviour) -> Self {
        let socket = UdpSocket::bind("127.0.0.1:0").unwrap();
        socket
            .set_read_timeout(Some(Duration::from_millis(20)))
            .unwrap();
        let addr = socket.local_addr().unwrap();
        let stop = Arc::new(AtomicBool::new(false));
        let received = Arc::new(Mutex::new(Vec::new()));
        let (flag, log) = (stop.clone(), received.clone());
        let handle = thread::spawn(move || {
            let mut power = "OFF".to_owned();
            let mut buf = vec![0u8; 65_507];
            while !flag.load(Ordering::SeqCst) {
                let Ok((n, from)) = socket.recv_from(&mut buf) else {
                    continue;
                };
                log.lock().unwrap().push(buf[..n].to_vec());
                let text = String::from_utf8_lossy(&buf[..n]).trim().to_owned();
                let reply: Vec<u8> = match behaviour {
                    Behaviour::NotUtf8 => vec![0xff, 0xfe],
                    Behaviour::Oversize => vec![b'A'; MAX_LINE_BYTES + 10],
                    Behaviour::Normal => match text.as_str() {
                        "POWR?" => format!("POWR={power}").into_bytes(),
                        "POWR 1" => {
                            power = "ON".into();
                            b"OK".to_vec()
                        }
                        _ => b"ERR".to_vec(),
                    },
                };
                let _ = socket.send_to(&reply, from);
            }
        });
        Self {
            addr,
            stop,
            handle: Some(handle),
            received,
        }
    }

    fn datagrams(&self) -> Vec<Vec<u8>> {
        self.received.lock().unwrap().clone()
    }
}

impl Drop for FakeDevice {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::SeqCst);
        if let Some(h) = self.handle.take() {
            let _ = h.join();
        }
    }
}

fn config(target: SocketAddr) -> UdpDriverConfig {
    UdpDriverConfig::new(target)
        .timeout(Duration::from_millis(300))
        .bind("power_on", TextCommand::new("POWR 1").ack("OK"))
        .query(
            TextQuery::new("power", "POWR?")
                .parser(ParserKind::PowerState)
                .strip_prefix("POWR="),
        )
}

#[test]
fn queries_and_commands_round_trip_as_single_datagrams() {
    let dev = FakeDevice::start(Behaviour::Normal);
    let mut d = UdpDriver::new(config(dev.addr)).unwrap();
    assert_eq!(
        d.get_state().unwrap().get("power"),
        Some(&StateValue::Boolean(false))
    );
    let r = d.execute(DeviceCommand::PowerOn).unwrap();
    assert!(r.ok && r.message.unwrap().contains("acknowledged"));
    assert_eq!(
        d.get_state().unwrap().get("power"),
        Some(&StateValue::Boolean(true))
    );
    // No terminator is added by default: the datagram is exactly the text.
    assert_eq!(dev.datagrams()[0], b"POWR?");
}

#[test]
fn terminator_is_added_only_when_configured() {
    let dev = FakeDevice::start(Behaviour::Normal);
    let mut d = UdpDriver::new(
        config(dev.addr).terminator(tpt_app_av_commissioning_profile::Terminator::Crlf),
    )
    .unwrap();
    d.get_state().unwrap();
    assert_eq!(dev.datagrams()[0], b"POWR?\r\n");
}

#[test]
fn injection_is_refused_before_anything_is_sent() {
    let dev = FakeDevice::start(Behaviour::Normal);
    let mut d = UdpDriver::new(config(dev.addr).bind("set_input", TextCommand::new("INPT $input")))
        .unwrap();
    assert!(matches!(
        d.execute(DeviceCommand::SetInput {
            input: "a\r\nPOWR 0".into()
        }),
        Err(DriverError::Config(_))
    ));
    assert!(dev.datagrams().is_empty());
}

#[test]
fn silence_is_a_timeout() {
    let silent = UdpSocket::bind("127.0.0.1:0").unwrap();
    let mut d = UdpDriver::new(config(silent.local_addr().unwrap())).unwrap();
    assert!(matches!(d.get_state(), Err(DriverError::Timeout(_))));
}

#[test]
fn hostile_replies_are_errors_not_crashes() {
    for behaviour in [Behaviour::NotUtf8, Behaviour::Oversize] {
        let dev = FakeDevice::start(behaviour);
        let mut d = UdpDriver::new(config(dev.addr)).unwrap();
        assert!(matches!(
            d.get_state(),
            Err(DriverError::MalformedResponse(_))
        ));
    }
}

#[test]
fn profile_built_driver_and_phase9_test() {
    let doc = r#"
schema_version: 1
device:
  id: udp-proj
  match: { manufacturer: UdpCo, model: U-1 }
  protocol: { type: udp, port: 7000, timeout_ms: 300 }
  commands:
    power_on: { send: "POWR 1", ack: "OK" }
  state:
    power: { query: "POWR?", parser: power_state, strip_prefix: "POWR=" }
"#;
    let profile = DeviceProfile::from_yaml_str(doc).unwrap();
    let dev = FakeDevice::start(Behaviour::Normal);
    let mut cfg = UdpDriverConfig::from_profile(&profile, dev.addr.ip()).unwrap();
    assert_eq!(cfg.target.port(), 7000);
    cfg.target = dev.addr;
    let drv = kinds::share(UdpDriver::new(cfg).unwrap());
    let t = kinds::power_on("p", DeviceId::new("u"), drv);
    assert_eq!(t.execute().unwrap().status, TestStatus::Pass);

    let tcp = doc.replace("type: udp", "type: tcp");
    let p = DeviceProfile::from_yaml_str(&tcp).unwrap();
    assert!(UdpDriverConfig::from_profile(&p, "10.0.0.5".parse().unwrap()).is_err());
}
