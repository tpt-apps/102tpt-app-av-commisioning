//! End-to-end: the HTTP driver against a fake REST device on loopback.

use std::io::{Read, Write};
use std::net::{SocketAddr, TcpListener, TcpStream};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::thread::{self, JoinHandle};
use std::time::Duration;

use tpt_app_av_commissioning_device::{DeviceCommand, StateValue};
use tpt_app_av_commissioning_driver::{DeviceDriver, DriverError};
use tpt_app_av_commissioning_driver_generic::http::{
    HttpAuth, HttpCommand, HttpDriver, HttpDriverConfig, HttpQuery, MAX_BODY_BYTES,
};
use tpt_app_av_commissioning_model::DeviceId;
use tpt_app_av_commissioning_profile::{DeviceProfile, ParserKind};
use tpt_app_av_commissioning_test::kinds;
use tpt_app_av_commissioning_test::{CommissioningTest, TestStatus};

#[derive(Debug, Clone)]
struct Seen {
    method: String,
    path: String,
    headers: Vec<(String, String)>,
    body: String,
}

impl Seen {
    fn header(&self, name: &str) -> Option<&str> {
        self.headers
            .iter()
            .find(|(k, _)| k.eq_ignore_ascii_case(name))
            .map(|(_, v)| v.as_str())
    }
}

#[derive(Clone, Copy, PartialEq)]
enum Behaviour {
    Normal,
    /// Responds 401 to everything.
    NeedsAuth,
    /// Responds with a chunked body.
    Chunked,
    /// Claims an enormous Content-Length.
    HugeLength,
    /// Streams a body with no length until it exceeds the cap.
    Endless,
    /// Sends both Content-Length and Transfer-Encoding.
    Smuggle,
    /// Responds with a 302.
    Redirect,
    /// Accepts and never answers.
    Silent,
    /// Sends something that is not HTTP.
    NotHttp,
}

struct FakeDevice {
    addr: SocketAddr,
    stop: Arc<AtomicBool>,
    handle: Option<JoinHandle<()>>,
    seen: Arc<Mutex<Vec<Seen>>>,
}

impl FakeDevice {
    fn start(behaviour: Behaviour) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        listener.set_nonblocking(true).unwrap();
        let addr = listener.local_addr().unwrap();
        let stop = Arc::new(AtomicBool::new(false));
        let seen = Arc::new(Mutex::new(Vec::new()));
        let power = Arc::new(Mutex::new(false));
        let (flag, log) = (stop.clone(), seen.clone());
        let handle = thread::spawn(move || {
            let mut workers = Vec::new();
            while !flag.load(Ordering::SeqCst) {
                match listener.accept() {
                    Ok((stream, _)) => {
                        let (log, power) = (log.clone(), power.clone());
                        workers.push(thread::spawn(move || serve(stream, behaviour, log, power)));
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
            seen,
        }
    }

    fn requests(&self) -> Vec<Seen> {
        self.seen.lock().unwrap().clone()
    }
}

fn read_request(stream: &mut TcpStream) -> Option<Seen> {
    stream.set_nonblocking(false).ok()?;
    stream
        .set_read_timeout(Some(Duration::from_millis(500)))
        .ok()?;
    let mut buf = Vec::new();
    let mut chunk = [0u8; 1024];
    let head_end = loop {
        if let Some(p) = buf.windows(4).position(|w| w == b"\r\n\r\n") {
            break p;
        }
        let n = stream.read(&mut chunk).ok()?;
        if n == 0 {
            return None;
        }
        buf.extend_from_slice(&chunk[..n]);
    };
    let head = String::from_utf8_lossy(&buf[..head_end]).to_string();
    let mut lines = head.split("\r\n");
    let mut first = lines.next()?.split(' ');
    let method = first.next()?.to_owned();
    let path = first.next()?.to_owned();
    let headers: Vec<(String, String)> = lines
        .filter_map(|l| l.split_once(':'))
        .map(|(k, v)| (k.trim().to_owned(), v.trim().to_owned()))
        .collect();
    let len: usize = headers
        .iter()
        .find(|(k, _)| k.eq_ignore_ascii_case("content-length"))
        .and_then(|(_, v)| v.parse().ok())
        .unwrap_or(0);
    let mut body = buf[head_end + 4..].to_vec();
    while body.len() < len {
        let n = stream.read(&mut chunk).ok()?;
        if n == 0 {
            break;
        }
        body.extend_from_slice(&chunk[..n]);
    }
    Some(Seen {
        method,
        path,
        headers,
        body: String::from_utf8_lossy(&body).to_string(),
    })
}

fn respond(stream: &mut TcpStream, status: &str, extra: &str, body: &str) {
    let _ = write!(
        stream,
        "HTTP/1.1 {status}\r\nContent-Length: {}\r\nContent-Type: application/json\r\nConnection: close\r\n{extra}\r\n{body}",
        body.len()
    );
}

fn serve(
    mut stream: TcpStream,
    behaviour: Behaviour,
    log: Arc<Mutex<Vec<Seen>>>,
    power: Arc<Mutex<bool>>,
) {
    let Some(req) = read_request(&mut stream) else {
        return;
    };
    log.lock().unwrap().push(req.clone());
    match behaviour {
        Behaviour::Silent => thread::sleep(Duration::from_millis(900)),
        Behaviour::NotHttp => {
            let _ = stream.write_all(b"SSH-2.0-nope\r\n\r\n");
        }
        Behaviour::NeedsAuth => respond(&mut stream, "401 Unauthorized", "", "{}"),
        Behaviour::Redirect => respond(&mut stream, "302 Found", "Location: /elsewhere\r\n", ""),
        Behaviour::HugeLength => {
            let _ = stream.write_all(
                b"HTTP/1.1 200 OK\r\nContent-Length: 99999999\r\nConnection: close\r\n\r\n{",
            );
        }
        Behaviour::Endless => {
            let _ = stream.write_all(b"HTTP/1.1 200 OK\r\nConnection: close\r\n\r\n");
            let junk = vec![b'x'; 8192];
            for _ in 0..(MAX_BODY_BYTES / 8192 + 4) {
                if stream.write_all(&junk).is_err() {
                    break;
                }
            }
        }
        Behaviour::Smuggle => {
            let _ = stream.write_all(
                b"HTTP/1.1 200 OK\r\nContent-Length: 2\r\nTransfer-Encoding: chunked\r\n\r\n2\r\n{}\r\n0\r\n\r\n",
            );
        }
        Behaviour::Chunked => {
            let _ = stream.write_all(
                b"HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\nConnection: close\r\n\r\n",
            );
            let body = r#"{"power":{"state":"off"},"vol":-6.5,"input":"x"}"#;
            let (a, b) = body.split_at(10);
            let _ = write!(
                stream,
                "{:x}\r\n{a}\r\n{:x}\r\n{b}\r\n0\r\n\r\n",
                a.len(),
                b.len()
            );
        }
        Behaviour::Normal => match (req.method.as_str(), req.path.as_str()) {
            ("GET", "/api/state") => {
                let on = *power.lock().unwrap();
                respond(
                    &mut stream,
                    "200 OK",
                    "",
                    &format!(
                        r#"{{"power":{{"state":"{}"}},"vol":-6.5,"input":"hdmi1"}}"#,
                        if on { "on" } else { "off" }
                    ),
                );
            }
            ("POST", "/api/power") => {
                *power.lock().unwrap() = req.body.contains("true");
                respond(&mut stream, "200 OK", "", r#"{"ok":true}"#);
            }
            ("PUT", _) if req.path.starts_with("/api/input/") => {
                respond(&mut stream, "204 No Content", "", "");
            }
            _ => respond(&mut stream, "404 Not Found", "", "{}"),
        },
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

fn config(target: SocketAddr) -> HttpDriverConfig {
    HttpDriverConfig::new(target)
        .timeout(Duration::from_millis(500))
        .bind(
            "power_on",
            HttpCommand::new("POST", "/api/power").body(r#"{"on":true}"#),
        )
        .bind(
            "power_off",
            HttpCommand::new("POST", "/api/power").body(r#"{"on":false}"#),
        )
        .bind(
            "set_input",
            HttpCommand::new("PUT", "/api/input/$input").body(r#"{"name":"$input"}"#),
        )
        .query(
            HttpQuery::new("power", "/api/state")
                .parser(ParserKind::PowerState)
                .extract("/power/state"),
        )
        .query(HttpQuery::new("volume_db", "/api/state").extract("/vol"))
        .query(HttpQuery::new("input", "/api/state").extract("/input"))
}

fn id() -> DeviceId {
    DeviceId::new("rest-dev")
}

#[test]
fn json_state_is_typed_and_shared_gets_are_fetched_once() {
    let dev = FakeDevice::start(Behaviour::Normal);
    let mut d = HttpDriver::new(config(dev.addr)).unwrap();
    let s = d.get_state().unwrap();
    assert_eq!(s.get("power"), Some(&StateValue::Boolean(false)));
    assert_eq!(s.get("volume_db"), Some(&StateValue::Float(-6.5)));
    assert_eq!(s.get("input"), Some(&StateValue::Text("hdmi1".into())));
    // Three fields, one GET.
    assert_eq!(dev.requests().len(), 1);
    let r = &dev.requests()[0];
    assert_eq!((r.method.as_str(), r.path.as_str()), ("GET", "/api/state"));
    assert_eq!(r.header("connection"), Some("close"));
}

#[test]
fn commands_send_json_and_state_confirms_them() {
    let dev = FakeDevice::start(Behaviour::Normal);
    let mut d = HttpDriver::new(config(dev.addr)).unwrap();
    let r = d.execute(DeviceCommand::PowerOn).unwrap();
    assert!(r.ok, "{:?}", r.message);
    let sent = &dev.requests()[0];
    assert_eq!(sent.header("content-type"), Some("application/json"));
    assert_eq!(sent.body, r#"{"on":true}"#);
    assert_eq!(
        d.get_state().unwrap().get("power"),
        Some(&StateValue::Boolean(true))
    );
}

#[test]
fn values_are_escaped_for_path_and_body() {
    let dev = FakeDevice::start(Behaviour::Normal);
    let mut d = HttpDriver::new(config(dev.addr)).unwrap();
    // A value that tries to add a path segment, a query and a JSON field.
    let r = d
        .execute(DeviceCommand::SetInput {
            input: r#"a/b?x=1","admin":true,"z":"#.into(),
        })
        .unwrap();
    assert!(r.ok);
    let sent = &dev.requests()[0];
    assert_eq!(
        sent.path,
        "/api/input/a%2Fb%3Fx%3D1%22%2C%22admin%22%3Atrue%2C%22z%22%3A"
    );
    let body: serde_json::Value = serde_json::from_str(&sent.body).unwrap();
    assert_eq!(body["name"].as_str(), Some(r#"a/b?x=1","admin":true,"z":"#));
    assert!(body.get("admin").is_none(), "no injected field");
}

#[test]
fn control_characters_in_values_are_refused_before_sending() {
    let dev = FakeDevice::start(Behaviour::Normal);
    let mut d = HttpDriver::new(config(dev.addr)).unwrap();
    assert!(matches!(
        d.execute(DeviceCommand::SetInput {
            input: "a\r\nX-Evil: 1".into()
        }),
        Err(DriverError::Config(_))
    ));
    assert!(dev.requests().is_empty());
}

#[test]
fn non_2xx_commands_are_rejected_not_errors() {
    let dev = FakeDevice::start(Behaviour::Normal);
    let mut d = HttpDriver::new(
        config(dev.addr).bind("power_cycle", HttpCommand::new("POST", "/api/nope")),
    )
    .unwrap();
    let r = d.execute(DeviceCommand::PowerCycle).unwrap();
    assert!(!r.ok);
    assert!(r.message.unwrap().contains("404"));
}

#[test]
fn redirects_are_not_followed() {
    let dev = FakeDevice::start(Behaviour::Redirect);
    let mut d = HttpDriver::new(config(dev.addr)).unwrap();
    let r = d.execute(DeviceCommand::PowerOn).unwrap();
    assert!(!r.ok && r.message.unwrap().contains("302"));
    assert_eq!(dev.requests().len(), 1);
}

#[test]
fn credentials_are_sent_and_refusal_is_reported() {
    let dev = FakeDevice::start(Behaviour::NeedsAuth);
    let mut d = HttpDriver::new(config(dev.addr).auth(HttpAuth::Bearer("tok123".into()))).unwrap();
    assert!(matches!(d.get_state(), Err(DriverError::Refused(_))));
    assert!(matches!(
        d.execute(DeviceCommand::PowerOn),
        Err(DriverError::Refused(_))
    ));
    assert_eq!(
        dev.requests()[0].header("authorization"),
        Some("Bearer tok123")
    );
}

#[test]
fn chunked_bodies_are_decoded() {
    let dev = FakeDevice::start(Behaviour::Chunked);
    let mut d = HttpDriver::new(config(dev.addr)).unwrap();
    let s = d.get_state().unwrap();
    assert_eq!(s.get("power"), Some(&StateValue::Boolean(false)));
    assert_eq!(s.get("volume_db"), Some(&StateValue::Float(-6.5)));
}

#[test]
fn hostile_responses_are_errors_not_crashes() {
    for (behaviour, label) in [
        (Behaviour::HugeLength, "huge content-length"),
        (Behaviour::Endless, "endless body"),
        (Behaviour::Smuggle, "smuggling headers"),
        (Behaviour::NotHttp, "not http"),
    ] {
        let dev = FakeDevice::start(behaviour);
        let mut d = HttpDriver::new(config(dev.addr)).unwrap();
        assert!(
            matches!(d.get_state(), Err(DriverError::MalformedResponse(_))),
            "{label}"
        );
    }
}

#[test]
fn silent_device_times_out() {
    let dev = FakeDevice::start(Behaviour::Silent);
    let mut d = HttpDriver::new(config(dev.addr)).unwrap();
    assert!(matches!(d.get_state(), Err(DriverError::Timeout(_))));
}

#[test]
fn arbitrary_requests_are_not_accepted() {
    let dev = FakeDevice::start(Behaviour::Normal);
    let mut d = HttpDriver::new(config(dev.addr)).unwrap();
    assert_eq!(
        d.execute(DeviceCommand::Arbitrary {
            command: "GET /admin".into()
        }),
        Err(DriverError::UnsupportedOperation)
    );
    assert!(dev.requests().is_empty());
}

#[test]
fn nothing_listening_is_unreachable_or_times_out() {
    let addr = {
        let l = TcpListener::bind("127.0.0.1:0").unwrap();
        l.local_addr().unwrap()
    };
    let mut d = HttpDriver::new(config(addr)).unwrap();
    assert!(matches!(
        d.get_state(),
        Err(DriverError::Unreachable(_)) | Err(DriverError::Timeout(_))
    ));
}

const PROFILE: &str = r#"
schema_version: 1
device:
  id: rest-projector
  match: { manufacturer: RestCo, model: R-1 }
  protocol: { type: http, port: 8080, timeout_ms: 500 }
  commands:
    power_on:  { send: "POST /api/power", body: '{"on":true}' }
    power_off: { send: "POST /api/power", body: '{"on":false}' }
    set_input: { send: "PUT /api/input/$input", body: '{"name":"$input"}' }
  state:
    power: { query: "GET /api/state", parser: power_state, extract: "/power/state" }
    input: { query: "GET /api/state", extract: "/input" }
"#;

#[test]
fn profile_built_driver_runs_phase9_tests() {
    let profile = DeviceProfile::from_yaml_str(PROFILE).unwrap();
    let dev = FakeDevice::start(Behaviour::Normal);
    let mut cfg = HttpDriverConfig::from_profile(&profile, dev.addr.ip()).unwrap();
    assert_eq!(cfg.target.port(), 8080);
    cfg.target = dev.addr;
    let drv = kinds::share(HttpDriver::new(cfg).unwrap());
    let on = kinds::power_on("p", id(), drv.clone());
    assert_eq!(on.execute().unwrap().status, TestStatus::Pass);
    // The fake reports no signal field, so only the input read-back counts.
    let input = kinds::select_input("i", id(), drv, "hdmi2");
    assert_ne!(input.execute().unwrap().status, TestStatus::Pass);
}

#[test]
fn profile_validation_covers_http_rules() {
    let bad_method = PROFILE.replace(
        "POST /api/power\", body: '{\"on\":true}'",
        "TRACE /api/power\"",
    );
    assert!(DeviceProfile::from_yaml_str(&bad_method).is_err());
    let post_query = PROFILE.replace(
        "GET /api/state\", extract: \"/input\"",
        "POST /api/state\", extract: \"/input\"",
    );
    assert!(DeviceProfile::from_yaml_str(&post_query).is_err());
    let body_in_tcp = PROFILE.replace("type: http", "type: tcp");
    assert!(DeviceProfile::from_yaml_str(&body_in_tcp).is_err());
    let wrong_ph = PROFILE.replace(r#"{"on":true}"#, r#"{"on":"$input"}"#);
    assert!(DeviceProfile::from_yaml_str(&wrong_ph).is_err());
    let osc = PROFILE.replace("type: http", "type: osc");
    let _ = osc;
}
