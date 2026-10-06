//! End-to-end: the TCP driver against a fake line-protocol device on loopback,
//! driven by the Phase 9 commissioning tests.

use std::io::{BufRead, BufReader, Write};
use std::net::{SocketAddr, TcpListener, TcpStream};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::thread::{self, JoinHandle};
use std::time::Duration;

use tpt_app_av_commissioning_device::{DeviceCommand, DeviceIdentity, StateValue};
use tpt_app_av_commissioning_driver::{DeviceDriver, DriverError};
use tpt_app_av_commissioning_driver_generic::{
    TcpDriver, TcpDriverConfig, TextCommand, TextQuery, MAX_LINE_BYTES,
};
use tpt_app_av_commissioning_model::DeviceId;
use tpt_app_av_commissioning_profile::{DeviceProfile, ParserKind};
use tpt_app_av_commissioning_test::kinds::{self, ConnectivityProbe, ConnectivityTest};
use tpt_app_av_commissioning_test::{CommissioningTest, TestStatus};

#[derive(Clone, Copy, PartialEq)]
enum Behaviour {
    Normal,
    /// Says OK but never changes state.
    IgnoreSets,
    /// Accepts connections and never answers.
    Silent,
    /// Replies with bytes that are not UTF-8.
    NotUtf8,
    /// Replies with an endless line.
    Flood,
    /// Replies with something the parser cannot read.
    Nonsense,
}

struct FakeDevice {
    addr: SocketAddr,
    stop: Arc<AtomicBool>,
    handle: Option<JoinHandle<()>>,
    /// Every line any client sent.
    received: Arc<Mutex<Vec<String>>>,
}

impl FakeDevice {
    fn start(behaviour: Behaviour) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        listener.set_nonblocking(true).unwrap();
        let addr = listener.local_addr().unwrap();
        let stop = Arc::new(AtomicBool::new(false));
        let received = Arc::new(Mutex::new(Vec::new()));
        let state = Arc::new(Mutex::new(("OFF".to_owned(), "hdmi1".to_owned())));
        let (flag, log) = (stop.clone(), received.clone());
        let handle = thread::spawn(move || {
            let mut workers = Vec::new();
            while !flag.load(Ordering::SeqCst) {
                match listener.accept() {
                    Ok((stream, _)) => {
                        let (flag, log, state) = (flag.clone(), log.clone(), state.clone());
                        workers.push(thread::spawn(move || {
                            serve(stream, behaviour, flag, log, state)
                        }));
                    }
                    Err(_) => thread::sleep(Duration::from_millis(5)),
                }
            }
            for w in workers {
                let _ = w.join();
            }
        });
        Self {
            addr,
            stop,
            handle: Some(handle),
            received,
        }
    }

    fn lines(&self) -> Vec<String> {
        self.received.lock().unwrap().clone()
    }
}

fn serve(
    stream: TcpStream,
    behaviour: Behaviour,
    stop: Arc<AtomicBool>,
    log: Arc<Mutex<Vec<String>>>,
    state: Arc<Mutex<(String, String)>>,
) {
    stream
        .set_read_timeout(Some(Duration::from_millis(20)))
        .unwrap();
    let mut out = stream.try_clone().unwrap();
    let mut reader = BufReader::new(stream);
    let mut line = String::new();
    while !stop.load(Ordering::SeqCst) {
        line.clear();
        match reader.read_line(&mut line) {
            Ok(0) => return,
            Ok(_) => {}
            Err(e)
                if matches!(
                    e.kind(),
                    std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut
                ) =>
            {
                continue
            }
            Err(_) => return,
        }
        let cmd = line.trim().to_owned();
        log.lock().unwrap().push(cmd.clone());
        let reply: Vec<u8> = match behaviour {
            Behaviour::Silent => continue,
            Behaviour::NotUtf8 => vec![0xff, 0xfe, b'\r', b'\n'],
            Behaviour::Flood => vec![b'A'; MAX_LINE_BYTES * 4],
            Behaviour::Nonsense => b"???\r\n".to_vec(),
            Behaviour::Normal | Behaviour::IgnoreSets => {
                let mut st = state.lock().unwrap();
                let set = behaviour == Behaviour::Normal;
                match cmd.as_str() {
                    "POWR?" => format!("POWR={}\r\n", st.0).into_bytes(),
                    "INPT?" => format!("INPT={}\r\n", st.1).into_bytes(),
                    "POWR 1" => {
                        if set {
                            st.0 = "ON".into();
                        }
                        b"OK\r\n".to_vec()
                    }
                    "POWR 0" => {
                        if set {
                            st.0 = "OFF".into();
                        }
                        b"OK\r\n".to_vec()
                    }
                    c if c.starts_with("INPT ") => {
                        if set {
                            st.1 = c[5..].to_owned();
                        }
                        b"OK\r\n".to_vec()
                    }
                    _ => b"ERR\r\n".to_vec(),
                }
            }
        };
        if out.write_all(&reply).is_err() {
            return;
        }
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

fn config(target: SocketAddr) -> TcpDriverConfig {
    TcpDriverConfig::new(target)
        .timeout(Duration::from_millis(400))
        .identity(DeviceIdentity {
            manufacturer: Some("TextCo".into()),
            model: Some("Lux-3".into()),
            ..DeviceIdentity::default()
        })
        .bind(
            "power_on",
            TextCommand {
                send: "POWR 1".into(),
                ack: Some("OK".into()),
            },
        )
        .bind(
            "power_off",
            TextCommand {
                send: "POWR 0".into(),
                ack: Some("OK".into()),
            },
        )
        .bind(
            "set_input",
            TextCommand {
                send: "INPT $input".into(),
                ack: Some("OK".into()),
            },
        )
        .query(
            TextQuery::new("power", "POWR?")
                .parser(ParserKind::PowerState)
                .strip_prefix("POWR="),
        )
        .query(
            TextQuery::new("input", "INPT?")
                .parser(ParserKind::Text)
                .strip_prefix("INPT="),
        )
}

fn driver(dev: &FakeDevice) -> TcpDriver {
    TcpDriver::new(config(dev.addr)).unwrap()
}

fn id() -> DeviceId {
    DeviceId::new("proj-tcp")
}

#[test]
fn reads_typed_state() {
    let dev = FakeDevice::start(Behaviour::Normal);
    let mut d = driver(&dev);
    let s = d.get_state().unwrap();
    assert_eq!(s.get("power"), Some(&StateValue::Boolean(false)));
    assert_eq!(s.get("input"), Some(&StateValue::Text("hdmi1".into())));
    assert!(d.capabilities().can_power_on && d.capabilities().can_select_input);
}

#[test]
fn acknowledged_command_changes_state() {
    let dev = FakeDevice::start(Behaviour::Normal);
    let mut d = driver(&dev);
    let r = d.execute(DeviceCommand::PowerOn).unwrap();
    assert!(r.ok && r.message.unwrap().contains("acknowledged"));
    assert_eq!(
        d.get_state().unwrap().get("power"),
        Some(&StateValue::Boolean(true))
    );
}

#[test]
fn wrong_acknowledgement_is_a_rejected_command() {
    // Device answers ERR to anything it does not know.
    let dev = FakeDevice::start(Behaviour::Normal);
    let mut d = TcpDriver::new(config(dev.addr).bind(
        "freeze",
        TextCommand {
            send: "FRZE".into(),
            ack: Some("OK".into()),
        },
    ))
    .unwrap();
    let r = d.execute(DeviceCommand::Freeze { frozen: true }).unwrap();
    assert!(!r.ok);
    assert!(r.message.unwrap().contains("ERR"));
}

#[test]
fn injection_through_a_value_is_refused_before_anything_is_sent() {
    let dev = FakeDevice::start(Behaviour::Normal);
    let mut d = driver(&dev);
    let err = d
        .execute(DeviceCommand::SetInput {
            input: "hdmi2\r\nPOWR 0".into(),
        })
        .unwrap_err();
    assert!(matches!(err, DriverError::Config(_)));
    assert!(dev.lines().is_empty(), "nothing may reach the device");
}

#[test]
fn arbitrary_commands_are_single_line() {
    let dev = FakeDevice::start(Behaviour::Normal);
    let mut d = driver(&dev);
    assert!(d
        .execute(DeviceCommand::Arbitrary {
            command: "POWR 1\nPOWR 0".into()
        })
        .is_err());
    assert!(dev.lines().is_empty());
    let r = d
        .execute(DeviceCommand::Arbitrary {
            command: "PING".into(),
        })
        .unwrap();
    assert!(r.ok);
    // Fire-and-forget: the device logs it a moment after we hang up.
    for _ in 0..100 {
        if !dev.lines().is_empty() {
            break;
        }
        thread::sleep(Duration::from_millis(10));
    }
    assert_eq!(dev.lines(), vec!["PING"]);
}

#[test]
fn unbound_command_is_unsupported() {
    let dev = FakeDevice::start(Behaviour::Normal);
    let mut d = driver(&dev);
    assert_eq!(
        d.execute(DeviceCommand::PowerCycle),
        Err(DriverError::UnsupportedOperation)
    );
}

#[test]
fn closed_port_is_unreachable() {
    let addr = {
        let l = TcpListener::bind("127.0.0.1:0").unwrap();
        l.local_addr().unwrap()
    };
    let mut d = TcpDriver::new(config(addr)).unwrap();
    // Windows retries a refused loopback connect, so within a short timeout
    // it can surface as a timeout instead of a refusal.
    assert!(matches!(
        d.get_state(),
        Err(DriverError::Unreachable(_)) | Err(DriverError::Timeout(_))
    ));
}

#[test]
fn silent_device_times_out() {
    let dev = FakeDevice::start(Behaviour::Silent);
    let mut d = driver(&dev);
    assert!(matches!(d.get_state(), Err(DriverError::Timeout(_))));
}

#[test]
fn hostile_replies_are_errors_not_crashes() {
    for (behaviour, label) in [
        (Behaviour::NotUtf8, "not utf-8"),
        (Behaviour::Flood, "endless line"),
        (Behaviour::Nonsense, "wrong prefix"),
    ] {
        let dev = FakeDevice::start(behaviour);
        let mut d = driver(&dev);
        assert!(
            matches!(d.get_state(), Err(DriverError::MalformedResponse(_))),
            "{label}"
        );
    }
}

// --- the Phase 9 tests, running over TCP ------------------------------------

#[test]
fn phase9_tests_pass_against_a_good_device() {
    let dev = FakeDevice::start(Behaviour::Normal);
    let drv = kinds::share(driver(&dev));
    let conn = ConnectivityTest::new("c", id(), ConnectivityProbe::Tcp, drv.clone());
    assert_eq!(conn.execute().unwrap().status, TestStatus::Pass);
    let on = kinds::power_on("p", id(), drv.clone());
    assert_eq!(on.execute().unwrap().status, TestStatus::Pass);
    let input = kinds::select_input("i", id(), drv, "hdmi3");
    // The fake has no signal field, so only the input read-back is checked:
    // that expectation passes while signal_present is unreported.
    assert_eq!(input.execute().unwrap().status, TestStatus::Inconclusive);
}

#[test]
fn phase9_power_on_fails_when_the_device_says_ok_but_does_nothing() {
    let dev = FakeDevice::start(Behaviour::IgnoreSets);
    let t = kinds::power_on("p", id(), kinds::share(driver(&dev)));
    assert_eq!(t.execute().unwrap().status, TestStatus::Fail);
}

#[test]
fn phase9_connectivity_fails_when_nothing_listens() {
    let addr = {
        let l = TcpListener::bind("127.0.0.1:0").unwrap();
        l.local_addr().unwrap()
    };
    let drv = kinds::share(TcpDriver::new(config(addr)).unwrap());
    let t = ConnectivityTest::new("c", id(), ConnectivityProbe::Tcp, drv);
    assert_eq!(t.execute().unwrap().status, TestStatus::Fail);
}

// --- profiles ----------------------------------------------------------------

const SAMPLE: &str = include_str!("../../examples/tcp-projector.yaml");

#[test]
fn sample_profile_builds_a_working_driver() {
    let profile = DeviceProfile::from_yaml_str(SAMPLE).unwrap();
    let dev = FakeDevice::start(Behaviour::Normal);
    let mut cfg = TcpDriverConfig::from_profile(&profile, dev.addr.ip()).unwrap();
    assert_eq!(cfg.target.port(), 4352);
    cfg.target = dev.addr;
    let mut d = TcpDriver::new(cfg).unwrap();
    assert_eq!(d.identity().model.as_deref(), Some("Lux-3"));
    d.execute(DeviceCommand::SetInput {
        input: "hdmi4".into(),
    })
    .unwrap();
    assert_eq!(
        d.get_state().unwrap().get("input"),
        Some(&StateValue::Text("hdmi4".into()))
    );
}

#[test]
fn non_tcp_and_argful_profiles_are_refused() {
    let osc = SAMPLE.replace("type: tcp", "type: osc");
    let p = DeviceProfile::from_yaml_str(&osc).unwrap();
    assert!(TcpDriverConfig::from_profile(&p, "10.0.0.5".parse().unwrap()).is_err());
    let args = SAMPLE.replace(
        r#"power_on:  { send: "POWR 1", ack: "OK" }"#,
        r#"power_on:  { send: "POWR 1", args: [1] }"#,
    );
    let p = DeviceProfile::from_yaml_str(&args).unwrap();
    assert!(TcpDriverConfig::from_profile(&p, "10.0.0.5".parse().unwrap()).is_err());
}
