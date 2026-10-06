//! End-to-end: the WebSocket driver against a fake WebSocket device.

use std::io::{Read, Write};
use std::net::{SocketAddr, TcpListener, TcpStream};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::thread::{self, JoinHandle};
use std::time::Duration;

use tungstenite::Message;

use tpt_app_av_commissioning_device::{DeviceCommand, StateValue};
use tpt_app_av_commissioning_driver::{DeviceDriver, DriverError};
use tpt_app_av_commissioning_driver_generic::{
    TextCommand, TextQuery, WebSocketDriver, WebSocketDriverConfig,
};
use tpt_app_av_commissioning_model::DeviceId;
use tpt_app_av_commissioning_profile::{DeviceProfile, ParserKind};
use tpt_app_av_commissioning_test::kinds;
use tpt_app_av_commissioning_test::{CommissioningTest, TestStatus};

#[derive(Clone, Copy, PartialEq)]
enum Behaviour {
    Normal,
    /// Completes the handshake and never answers.
    Silent,
    Binary,
    Oversize,
    /// Answers the upgrade request with HTTP 403.
    RejectHandshake,
}

struct FakeDevice {
    addr: SocketAddr,
    stop: Arc<AtomicBool>,
    handle: Option<JoinHandle<()>>,
    received: Arc<Mutex<Vec<String>>>,
    paths: Arc<Mutex<Vec<String>>>,
}

impl FakeDevice {
    fn start(behaviour: Behaviour) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        listener.set_nonblocking(true).unwrap();
        let addr = listener.local_addr().unwrap();
        let stop = Arc::new(AtomicBool::new(false));
        let received = Arc::new(Mutex::new(Vec::new()));
        let paths = Arc::new(Mutex::new(Vec::new()));
        let power = Arc::new(Mutex::new(false));
        let (flag, log, seen_paths) = (stop.clone(), received.clone(), paths.clone());
        let handle = thread::spawn(move || {
            let mut workers = Vec::new();
            while !flag.load(Ordering::SeqCst) {
                match listener.accept() {
                    Ok((stream, _)) => {
                        let (flag, log, seen_paths, power) =
                            (flag.clone(), log.clone(), seen_paths.clone(), power.clone());
                        workers.push(thread::spawn(move || {
                            serve(stream, behaviour, flag, log, seen_paths, power)
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
            paths,
        }
    }

    fn messages(&self) -> Vec<String> {
        self.received.lock().unwrap().clone()
    }
}

fn serve(
    mut stream: TcpStream,
    behaviour: Behaviour,
    stop: Arc<AtomicBool>,
    log: Arc<Mutex<Vec<String>>>,
    seen_paths: Arc<Mutex<Vec<String>>>,
    power: Arc<Mutex<bool>>,
) {
    stream.set_nonblocking(false).unwrap();
    if behaviour == Behaviour::RejectHandshake {
        let mut buf = [0u8; 1024];
        let _ = stream.read(&mut buf);
        let _ = stream.write_all(b"HTTP/1.1 403 Forbidden\r\nContent-Length: 0\r\n\r\n");
        return;
    }
    #[allow(clippy::result_large_err)]
    let callback = |req: &tungstenite::handshake::server::Request,
                    resp: tungstenite::handshake::server::Response| {
        seen_paths.lock().unwrap().push(req.uri().path().to_owned());
        Ok(resp)
    };
    let Ok(mut ws) = tungstenite::accept_hdr(stream, callback) else {
        return;
    };
    ws.get_mut()
        .set_read_timeout(Some(Duration::from_millis(20)))
        .unwrap();
    while !stop.load(Ordering::SeqCst) {
        let msg = match ws.read() {
            Ok(m) => m,
            Err(tungstenite::Error::Io(e))
                if matches!(
                    e.kind(),
                    std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut
                ) =>
            {
                continue
            }
            Err(_) => return,
        };
        let Message::Text(text) = msg else { continue };
        let text = text.as_str().to_owned();
        log.lock().unwrap().push(text.clone());
        let reply = match behaviour {
            Behaviour::Silent => continue,
            Behaviour::Binary => Message::binary(vec![1u8, 2, 3]),
            Behaviour::Oversize => Message::text("A".repeat(10_000)),
            _ => match text.as_str() {
                "get" => {
                    let on = *power.lock().unwrap();
                    Message::text(format!(
                        r#"{{"power":{{"state":"{}"}},"vol":-12.5,"ch":3}}"#,
                        if on { "on" } else { "off" }
                    ))
                }
                "power on" => {
                    *power.lock().unwrap() = true;
                    Message::text("OK")
                }
                _ => Message::text("ERR"),
            },
        };
        if ws.send(reply).is_err() {
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

fn config(target: SocketAddr) -> WebSocketDriverConfig {
    WebSocketDriverConfig::new(target)
        .path("/control")
        .timeout(Duration::from_millis(400))
        .bind("power_on", TextCommand::new("power on").ack("OK"))
        .query(
            TextQuery::new("power", "get")
                .parser(ParserKind::Bool)
                .extract("/power/state"),
        )
        .query(TextQuery::new("volume_db", "get").extract("/vol"))
        .query(TextQuery::new("channel", "get").extract("/ch"))
}

#[test]
fn json_replies_become_typed_state() {
    let dev = FakeDevice::start(Behaviour::Normal);
    let mut d = WebSocketDriver::new(config(dev.addr)).unwrap();
    let s = d.get_state().unwrap();
    assert_eq!(s.get("power"), Some(&StateValue::Boolean(false)));
    assert_eq!(s.get("volume_db"), Some(&StateValue::Float(-12.5)));
    assert_eq!(s.get("channel"), Some(&StateValue::Integer(3)));
    // The handshake used the configured path.
    assert_eq!(dev.paths.lock().unwrap()[0], "/control");
}

#[test]
fn acknowledged_command_changes_state() {
    let dev = FakeDevice::start(Behaviour::Normal);
    let mut d = WebSocketDriver::new(config(dev.addr)).unwrap();
    let r = d.execute(DeviceCommand::PowerOn).unwrap();
    assert!(r.ok);
    assert_eq!(
        d.get_state().unwrap().get("power"),
        Some(&StateValue::Boolean(true))
    );
}

#[test]
fn injection_is_refused_before_anything_is_sent() {
    let dev = FakeDevice::start(Behaviour::Normal);
    let mut d =
        WebSocketDriver::new(config(dev.addr).bind("set_input", TextCommand::new("input $input")))
            .unwrap();
    assert!(matches!(
        d.execute(DeviceCommand::SetInput {
            input: "a\nb".into()
        }),
        Err(DriverError::Config(_))
    ));
    assert!(dev.messages().is_empty());
}

#[test]
fn silent_device_times_out() {
    let dev = FakeDevice::start(Behaviour::Silent);
    let mut d = WebSocketDriver::new(config(dev.addr)).unwrap();
    assert!(matches!(d.get_state(), Err(DriverError::Timeout(_))));
}

#[test]
fn hostile_replies_are_errors_not_crashes() {
    for behaviour in [Behaviour::Binary, Behaviour::Oversize] {
        let dev = FakeDevice::start(behaviour);
        let mut d = WebSocketDriver::new(config(dev.addr)).unwrap();
        assert!(matches!(
            d.get_state(),
            Err(DriverError::MalformedResponse(_))
        ));
    }
}

#[test]
fn rejected_handshake_is_refused() {
    let dev = FakeDevice::start(Behaviour::RejectHandshake);
    let mut d = WebSocketDriver::new(config(dev.addr)).unwrap();
    assert!(matches!(
        d.get_state(),
        Err(DriverError::Refused(_)) | Err(DriverError::Protocol(_))
    ));
}

#[test]
fn nothing_listening_is_unreachable_or_times_out() {
    let addr = {
        let l = TcpListener::bind("127.0.0.1:0").unwrap();
        l.local_addr().unwrap()
    };
    let mut d = WebSocketDriver::new(config(addr)).unwrap();
    assert!(matches!(
        d.get_state(),
        Err(DriverError::Unreachable(_)) | Err(DriverError::Timeout(_))
    ));
}

#[test]
fn profile_built_driver_and_phase9_test() {
    let doc = r#"
schema_version: 1
device:
  id: ws-proj
  match: { manufacturer: WsCo, model: W-1 }
  protocol: { type: websocket, port: 8080, path: /control, timeout_ms: 400 }
  commands:
    power_on: { send: "power on", ack: "OK" }
  state:
    power: { query: "get", parser: bool, extract: "/power/state" }
"#;
    // `bool` does not read "on"; the profile author picks `power_state`.
    let doc = doc.replace("parser: bool", "parser: power_state");
    let profile = DeviceProfile::from_yaml_str(&doc).unwrap();
    let dev = FakeDevice::start(Behaviour::Normal);
    let mut cfg = WebSocketDriverConfig::from_profile(&profile, dev.addr.ip()).unwrap();
    assert_eq!(cfg.target.port(), 8080);
    assert_eq!(cfg.path, "/control");
    cfg.target = dev.addr;
    let drv = kinds::share(WebSocketDriver::new(cfg).unwrap());
    let t = kinds::power_on("p", DeviceId::new("w"), drv);
    assert_eq!(t.execute().unwrap().status, TestStatus::Pass);
}
