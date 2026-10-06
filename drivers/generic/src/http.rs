//! HTTP/1.1 driver for devices with a REST-style control API.
//!
//! A deliberately small client: plain `http://` (no TLS yet, so for devices on
//! the commissioning LAN), one request per connection (`Connection: close`),
//! no redirects, no cookies. In exchange every limit is explicit:
//!
//! * the target is an explicit unicast address;
//! * connect, send and every read share the configured timeout;
//! * response headers are capped at [`MAX_HEAD_BYTES`] and the body at
//!   [`MAX_BODY_BYTES`], whatever the device claims in `Content-Length`;
//! * values substituted into a request path are percent-encoded and values
//!   substituted into a JSON body are JSON-escaped, so a value cannot add a
//!   path segment, a query string, or another JSON field;
//! * credentials come from the caller ([`HttpAuth`]), never from a profile,
//!   and are redacted from `Debug` output.
//!
//! State is read with GET requests; a JSON reply is mined with `extract` (a
//! JSON Pointer). Several state fields that share one GET are served by one
//! request per `get_state` call.
//!
//! Status handling: 2xx is success; 401/403 is an error (`Refused`, "check the
//! credentials"); any other status makes the command a *rejected command*
//! (`ok: false`) so a test can report it.

use std::collections::{BTreeMap, HashMap};
use std::io::{Read, Write};
use std::net::{IpAddr, SocketAddr, TcpStream};
use std::time::{Duration, Instant};

use tpt_app_av_commissioning_device::{
    DeviceCapabilities, DeviceCommand, DeviceIdentity, DeviceResponse, DeviceState,
};
use tpt_app_av_commissioning_driver::net::{check_target, check_timeout, map_io_error};
use tpt_app_av_commissioning_driver::{DeviceDriver, DriverError};
use tpt_app_av_commissioning_profile::{
    http_request_line, placeholders_in, DeviceProfile, ParserKind, ProtocolKind,
};

use crate::text::{kind_name, render, reply_to_value, Escape};

/// Default HTTP port.
pub const DEFAULT_PORT: u16 = 80;
/// Largest response head (status line + headers) accepted.
pub const MAX_HEAD_BYTES: usize = 16 * 1024;
/// Largest response body accepted.
pub const MAX_BODY_BYTES: usize = 64 * 1024;
const MAX_REQUEST_BODY: usize = 4096;

/// Credentials for a device that wants them. Never part of a profile.
#[derive(Clone, PartialEq, Eq)]
pub enum HttpAuth {
    Basic { username: String, password: String },
    Bearer(String),
}

impl std::fmt::Debug for HttpAuth {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            HttpAuth::Basic { username, .. } => write!(f, "Basic({username}, <redacted>)"),
            HttpAuth::Bearer(_) => write!(f, "Bearer(<redacted>)"),
        }
    }
}

impl HttpAuth {
    fn header_value(&self) -> String {
        match self {
            HttpAuth::Basic { username, password } => {
                format!(
                    "Basic {}",
                    base64(format!("{username}:{password}").as_bytes())
                )
            }
            HttpAuth::Bearer(token) => format!("Bearer {token}"),
        }
    }

    fn validate(&self) -> Result<(), DriverError> {
        let ok = |s: &str| !s.is_empty() && s.len() <= 256 && !s.chars().any(char::is_control);
        let valid = match self {
            HttpAuth::Basic { username, password } => {
                ok(username) && !username.contains(':') && !password.chars().any(char::is_control)
            }
            HttpAuth::Bearer(t) => ok(t) && !t.contains(' '),
        };
        if valid {
            Ok(())
        } else {
            Err(DriverError::Config(
                "HTTP credentials contain characters that cannot be sent safely".to_owned(),
            ))
        }
    }
}

fn base64(input: &[u8]) -> String {
    const T: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::with_capacity(input.len().div_ceil(3) * 4);
    for chunk in input.chunks(3) {
        let n =
            chunk.iter().fold(0u32, |acc, b| (acc << 8) | u32::from(*b)) << (8 * (3 - chunk.len()));
        for i in 0..4 {
            if i <= chunk.len() {
                out.push(T[((n >> (18 - 6 * i)) & 63) as usize] as char);
            } else {
                out.push('=');
            }
        }
    }
    out
}

/// A command: an HTTP request.
#[derive(Debug, Clone, PartialEq)]
pub struct HttpCommand {
    /// `GET`, `POST`, `PUT`, `PATCH` or `DELETE`.
    pub method: String,
    /// Path (and query), possibly with `$placeholders`.
    pub path: String,
    /// JSON body, possibly with `$placeholders` inside string values.
    pub body: Option<String>,
}

impl HttpCommand {
    pub fn new(method: impl Into<String>, path: impl Into<String>) -> Self {
        Self {
            method: method.into(),
            path: path.into(),
            body: None,
        }
    }

    pub fn body(mut self, body: impl Into<String>) -> Self {
        self.body = Some(body.into());
        self
    }
}

/// A state field read with a GET.
#[derive(Debug, Clone, PartialEq)]
pub struct HttpQuery {
    pub field: String,
    pub path: String,
    pub parser: ParserKind,
    pub strip_prefix: Option<String>,
    /// JSON Pointer into a JSON response body.
    pub extract: Option<String>,
}

impl HttpQuery {
    pub fn new(field: impl Into<String>, path: impl Into<String>) -> Self {
        Self {
            field: field.into(),
            path: path.into(),
            parser: ParserKind::Auto,
            strip_prefix: None,
            extract: None,
        }
    }

    pub fn parser(mut self, parser: ParserKind) -> Self {
        self.parser = parser;
        self
    }

    pub fn extract(mut self, pointer: impl Into<String>) -> Self {
        self.extract = Some(pointer.into());
        self
    }
}

/// Everything the HTTP driver needs to know about one device.
#[derive(Debug, Clone)]
pub struct HttpDriverConfig {
    pub target: SocketAddr,
    pub timeout: Duration,
    pub identity: DeviceIdentity,
    /// Keyed by command kind (`power_on`, `set_input`, ...).
    pub commands: BTreeMap<String, HttpCommand>,
    pub queries: Vec<HttpQuery>,
    pub auth: Option<HttpAuth>,
}

impl HttpDriverConfig {
    /// A config for `target`: 1 s timeout, nothing bound, no credentials.
    pub fn new(target: SocketAddr) -> Self {
        Self {
            target,
            timeout: Duration::from_secs(1),
            identity: DeviceIdentity::new(),
            commands: BTreeMap::new(),
            queries: Vec::new(),
            auth: None,
        }
    }

    pub fn timeout(mut self, timeout: Duration) -> Self {
        self.timeout = timeout;
        self
    }

    pub fn identity(mut self, identity: DeviceIdentity) -> Self {
        self.identity = identity;
        self
    }

    pub fn auth(mut self, auth: HttpAuth) -> Self {
        self.auth = Some(auth);
        self
    }

    pub fn bind(mut self, command_kind: impl Into<String>, command: HttpCommand) -> Self {
        self.commands.insert(command_kind.into(), command);
        self
    }

    pub fn query(mut self, query: HttpQuery) -> Self {
        self.queries.push(query);
        self
    }

    /// Build a config from an `http` device profile, for the device at `host`.
    pub fn from_profile(profile: &DeviceProfile, host: IpAddr) -> Result<Self, DriverError> {
        let p = &profile.device;
        if p.protocol.kind != ProtocolKind::Http {
            return Err(DriverError::Config(format!(
                "profile `{}` is a {:?} profile; the HTTP driver needs `type: http`",
                p.id, p.protocol.kind
            )));
        }
        let mut config = Self::new(SocketAddr::new(
            host,
            p.protocol.port.unwrap_or(DEFAULT_PORT),
        ))
        .timeout(Duration::from_millis(p.protocol.timeout_ms_or_default()))
        .identity(DeviceIdentity {
            manufacturer: Some(p.matcher.manufacturer.clone()),
            model: p.matcher.model.as_slice().first().cloned(),
            ..DeviceIdentity::default()
        });
        let invalid =
            |e: tpt_app_av_commissioning_profile::ProfileError| DriverError::Config(e.to_string());
        for (name, cmd) in &p.commands {
            if !cmd.args.is_empty() || cmd.ack.is_some() {
                return Err(DriverError::Config(format!(
                    "commands.{name}: `args` and `ack` do not apply to HTTP"
                )));
            }
            let (method, path) =
                http_request_line(&format!("commands.{name}.send"), &cmd.send).map_err(invalid)?;
            config = config.bind(
                name.clone(),
                HttpCommand {
                    method: method.to_owned(),
                    path: path.to_owned(),
                    body: cmd.body.clone(),
                },
            );
        }
        for (field, q) in &p.state {
            let (_, path) =
                http_request_line(&format!("state.{field}.query"), &q.query).map_err(invalid)?;
            config = config.query(HttpQuery {
                field: field.clone(),
                path: path.to_owned(),
                parser: q.parser.unwrap_or_default(),
                strip_prefix: q.strip_prefix.clone(),
                extract: q.extract.clone(),
            });
        }
        config.validate()?;
        Ok(config)
    }

    /// Check the config is safe and coherent. [`HttpDriver::new`] calls this.
    pub fn validate(&self) -> Result<(), DriverError> {
        check_target(self.target)?;
        check_timeout(self.timeout)?;
        if let Some(auth) = &self.auth {
            auth.validate()?;
        }
        let path_ok = |what: &str, path: &str| {
            if !path.starts_with('/')
                || path.len() > 256
                || path.chars().any(|c| c.is_control() || c == ' ')
            {
                Err(DriverError::Config(format!(
                    "{what} must start with '/' and contain no spaces"
                )))
            } else {
                Ok(())
            }
        };
        for (name, cmd) in &self.commands {
            if !matches!(
                cmd.method.as_str(),
                "GET" | "POST" | "PUT" | "PATCH" | "DELETE"
            ) {
                return Err(DriverError::Config(format!(
                    "commands.{name}: unsupported method {:?}",
                    cmd.method
                )));
            }
            path_ok(&format!("commands.{name} path"), &cmd.path)?;
            if let Some(body) = &cmd.body {
                if body.len() > MAX_REQUEST_BODY {
                    return Err(DriverError::Config(format!(
                        "commands.{name}: body is longer than {MAX_REQUEST_BODY} bytes"
                    )));
                }
            }
        }
        for q in &self.queries {
            path_ok(&format!("state.{} path", q.field), &q.path)?;
            if !placeholders_in(&q.path).is_empty() {
                return Err(DriverError::Config(format!(
                    "state.{} path may not contain placeholders",
                    q.field
                )));
            }
        }
        Ok(())
    }

    fn capabilities(&self) -> DeviceCapabilities {
        let has = |k: &str| self.commands.contains_key(k);
        let has_field = |f: &str| self.queries.iter().any(|q| q.field == f);
        DeviceCapabilities {
            can_power_on: has("power_on"),
            can_power_off: has("power_off"),
            can_read_state: !self.queries.is_empty(),
            can_select_input: has("set_input"),
            can_generate_test_pattern: has("generate_test_pattern"),
            can_read_signal_status: has_field("signal_present") || has_field("signal_lock"),
            can_read_edid: has("read_edid"),
            can_measure_latency: has("measure_latency"),
            can_restore_state: false,
        }
    }
}

/// An HTTP device.
pub struct HttpDriver {
    config: HttpDriverConfig,
    capabilities: DeviceCapabilities,
}

struct Response {
    status: u16,
    body: String,
}

impl HttpDriver {
    /// Validate `config`. No connection is made until an operation runs.
    pub fn new(config: HttpDriverConfig) -> Result<Self, DriverError> {
        config.validate()?;
        let capabilities = config.capabilities();
        Ok(Self {
            config,
            capabilities,
        })
    }

    pub fn config(&self) -> &HttpDriverConfig {
        &self.config
    }

    fn request(
        &self,
        method: &str,
        path: &str,
        body: Option<&str>,
    ) -> Result<Response, DriverError> {
        let cfg = &self.config;
        let io = |e: &std::io::Error| map_io_error(e, cfg.target, cfg.timeout);
        let deadline = Instant::now() + cfg.timeout;
        let mut stream =
            TcpStream::connect_timeout(&cfg.target, cfg.timeout).map_err(|e| io(&e))?;
        stream
            .set_write_timeout(Some(cfg.timeout))
            .map_err(|e| io(&e))?;
        let _ = stream.set_nodelay(true);

        let mut head = format!(
            "{method} {path} HTTP/1.1\r\nHost: {}\r\nConnection: close\r\nAccept: application/json, text/plain, */*\r\nUser-Agent: tpt-av-commissioning\r\n",
            cfg.target
        );
        if let Some(auth) = &cfg.auth {
            head.push_str(&format!("Authorization: {}\r\n", auth.header_value()));
        }
        if let Some(body) = body {
            head.push_str(&format!(
                "Content-Type: application/json\r\nContent-Length: {}\r\n",
                body.len()
            ));
        }
        head.push_str("\r\n");
        let mut bytes = head.into_bytes();
        if let Some(body) = body {
            bytes.extend_from_slice(body.as_bytes());
        }
        stream.write_all(&bytes).map_err(|e| io(&e))?;

        let mut reader = Reader {
            stream,
            buf: Vec::new(),
            deadline,
            target: cfg.target,
            timeout: cfg.timeout,
        };
        read_response(&mut reader, method)
    }

    fn read_state(&self) -> Result<DeviceState, DriverError> {
        if self.config.queries.is_empty() {
            return Err(DriverError::UnsupportedOperation);
        }
        // One GET per distinct path.
        let mut bodies: HashMap<&str, String> = HashMap::new();
        let mut state = DeviceState::new();
        for q in &self.config.queries {
            if !bodies.contains_key(q.path.as_str()) {
                let resp = self.request("GET", &q.path, None)?;
                check_auth(&resp)?;
                if !(200..300).contains(&resp.status) {
                    return Err(DriverError::Protocol(format!(
                        "GET {} returned HTTP {}",
                        q.path, resp.status
                    )));
                }
                bodies.insert(q.path.as_str(), resp.body);
            }
            let body = &bodies[q.path.as_str()];
            let value = reply_to_value(
                body,
                q.parser,
                q.strip_prefix.as_deref(),
                q.extract.as_deref(),
            )
            .map_err(|e| DriverError::MalformedResponse(format!("{}: {e}", q.field)))?;
            state.set(q.field.clone(), value);
        }
        Ok(state)
    }
}

fn check_auth(resp: &Response) -> Result<(), DriverError> {
    if matches!(resp.status, 401 | 403) {
        return Err(DriverError::Refused(format!(
            "HTTP {}: the device refused the request; check the credentials",
            resp.status
        )));
    }
    Ok(())
}

impl DeviceDriver for HttpDriver {
    fn identity(&self) -> DeviceIdentity {
        self.config.identity.clone()
    }

    /// Discovery is a state read: a device that answers its queries is there.
    fn discover(&mut self) -> Result<DeviceState, DriverError> {
        if self.config.queries.is_empty() {
            return Err(DriverError::Config(
                "no state queries configured; cannot verify the device is present".to_owned(),
            ));
        }
        self.read_state()
    }

    fn get_state(&mut self) -> Result<DeviceState, DriverError> {
        self.read_state()
    }

    fn execute(&mut self, command: DeviceCommand) -> Result<DeviceResponse, DriverError> {
        if matches!(command, DeviceCommand::ReadState) {
            let state = self.read_state()?;
            return Ok(DeviceResponse {
                ok: true,
                state: Some(state),
                message: None,
                response_time_ms: None,
            });
        }
        if matches!(command, DeviceCommand::Arbitrary { .. }) {
            // A raw request line is the one thing an HTTP profile must not
            // accept from test data: bind a command instead.
            return Err(DriverError::UnsupportedOperation);
        }
        let bound = self
            .config
            .commands
            .get(kind_name(&command))
            .ok_or(DriverError::UnsupportedOperation)?;
        let path = render(&bound.path, &command, Escape::Url)?;
        let body = bound
            .body
            .as_deref()
            .map(|b| render(b, &command, Escape::Json))
            .transpose()?;
        let started = Instant::now();
        let resp = self.request(&bound.method, &path, body.as_deref())?;
        check_auth(&resp)?;
        let ok = (200..300).contains(&resp.status);
        Ok(DeviceResponse {
            ok,
            state: None,
            message: Some(format!(
                "{} {path} -> HTTP {}{}",
                bound.method,
                resp.status,
                if ok {
                    "; verify by reading state"
                } else {
                    " (command rejected)"
                }
            )),
            response_time_ms: Some(started.elapsed().as_millis() as u64),
        })
    }

    fn capabilities(&self) -> DeviceCapabilities {
        self.capabilities
    }
}

// ---------------------------------------------------------------------------
// Response parsing
// ---------------------------------------------------------------------------

struct Reader {
    stream: TcpStream,
    buf: Vec<u8>,
    deadline: Instant,
    target: SocketAddr,
    timeout: Duration,
}

impl Reader {
    /// Pull more bytes into the buffer. `Ok(false)` means end of stream.
    fn fill(&mut self) -> Result<bool, DriverError> {
        let remaining = self.deadline.saturating_duration_since(Instant::now());
        if remaining.is_zero() {
            return Err(DriverError::Timeout(self.timeout.as_millis() as u64));
        }
        self.stream
            .set_read_timeout(Some(remaining))
            .map_err(|e| map_io_error(&e, self.target, self.timeout))?;
        let mut chunk = [0u8; 2048];
        match self.stream.read(&mut chunk) {
            Ok(0) => Ok(false),
            Ok(n) => {
                self.buf.extend_from_slice(&chunk[..n]);
                Ok(true)
            }
            Err(e) => Err(map_io_error(&e, self.target, self.timeout)),
        }
    }

    /// Read up to and including `needle`, returning the bytes before it.
    fn read_until(&mut self, needle: &[u8], limit: usize) -> Result<Vec<u8>, DriverError> {
        loop {
            if let Some(pos) = self.buf.windows(needle.len()).position(|w| w == needle) {
                let mut taken: Vec<u8> = self.buf.drain(..pos + needle.len()).collect();
                taken.truncate(pos);
                return Ok(taken);
            }
            if self.buf.len() > limit {
                return Err(DriverError::MalformedResponse(format!(
                    "response head exceeds {limit} bytes"
                )));
            }
            if !self.fill()? {
                return Err(DriverError::MalformedResponse(
                    "connection closed before the response was complete".to_owned(),
                ));
            }
        }
    }

    fn take_exact(&mut self, n: usize) -> Result<Vec<u8>, DriverError> {
        while self.buf.len() < n {
            if !self.fill()? {
                return Err(DriverError::MalformedResponse(
                    "connection closed before the body was complete".to_owned(),
                ));
            }
        }
        Ok(self.buf.drain(..n).collect())
    }

    fn take_to_end(&mut self, limit: usize) -> Result<Vec<u8>, DriverError> {
        while self.buf.len() <= limit {
            if !self.fill()? {
                return Ok(std::mem::take(&mut self.buf));
            }
        }
        Err(too_big())
    }
}

fn too_big() -> DriverError {
    DriverError::MalformedResponse(format!("response body exceeds {MAX_BODY_BYTES} bytes"))
}

fn malformed(what: &str) -> DriverError {
    DriverError::MalformedResponse(what.to_owned())
}

fn read_response(r: &mut Reader, method: &str) -> Result<Response, DriverError> {
    let head = r.read_until(b"\r\n\r\n", MAX_HEAD_BYTES)?;
    let head = String::from_utf8(head).map_err(|_| malformed("response head is not UTF-8"))?;
    let mut lines = head.split("\r\n");
    let status_line = lines.next().unwrap_or_default();
    let mut parts = status_line.splitn(3, ' ');
    let version = parts.next().unwrap_or_default();
    if !(version == "HTTP/1.1" || version == "HTTP/1.0") {
        return Err(malformed("not an HTTP/1.x response"));
    }
    let status: u16 = parts
        .next()
        .and_then(|s| s.parse().ok())
        .filter(|s| (100..600).contains(s))
        .ok_or_else(|| malformed("invalid HTTP status"))?;

    let mut content_length: Option<usize> = None;
    let mut chunked = false;
    for line in lines {
        let (name, value) = line
            .split_once(':')
            .ok_or_else(|| malformed("invalid header line"))?;
        let value = value.trim();
        match name.trim().to_ascii_lowercase().as_str() {
            "content-length" => {
                if content_length.is_some() {
                    return Err(malformed("duplicate Content-Length"));
                }
                content_length = Some(
                    value
                        .parse()
                        .map_err(|_| malformed("invalid Content-Length"))?,
                );
            }
            "transfer-encoding" => {
                if value.eq_ignore_ascii_case("chunked") {
                    chunked = true;
                } else {
                    return Err(malformed("unsupported Transfer-Encoding"));
                }
            }
            _ => {}
        }
    }
    if chunked && content_length.is_some() {
        // A classic request-smuggling shape; refuse rather than pick one.
        return Err(malformed("both Content-Length and Transfer-Encoding"));
    }

    let bodiless = method == "HEAD" || status < 200 || status == 204 || status == 304;
    let body = if bodiless {
        Vec::new()
    } else if chunked {
        read_chunked(r)?
    } else if let Some(n) = content_length {
        if n > MAX_BODY_BYTES {
            return Err(too_big());
        }
        r.take_exact(n)?
    } else {
        r.take_to_end(MAX_BODY_BYTES)?
    };
    let body = String::from_utf8(body).map_err(|_| malformed("response body is not UTF-8"))?;
    Ok(Response { status, body })
}

fn read_chunked(r: &mut Reader) -> Result<Vec<u8>, DriverError> {
    let mut body = Vec::new();
    loop {
        let line = r.read_until(b"\r\n", 64)?;
        let line = String::from_utf8(line).map_err(|_| malformed("invalid chunk header"))?;
        let size_text = line.split(';').next().unwrap_or_default().trim();
        let size =
            usize::from_str_radix(size_text, 16).map_err(|_| malformed("invalid chunk size"))?;
        if size == 0 {
            // Trailers: skip lines until the blank one.
            for _ in 0..32 {
                if r.read_until(b"\r\n", MAX_HEAD_BYTES)?.is_empty() {
                    return Ok(body);
                }
            }
            return Err(malformed("too many trailer lines"));
        }
        if body.len() + size > MAX_BODY_BYTES {
            return Err(too_big());
        }
        body.extend_from_slice(&r.take_exact(size)?);
        if !r.take_exact(2)?.eq(b"\r\n") {
            return Err(malformed("chunk not terminated by CRLF"));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn base64_matches_rfc4648_vectors() {
        for (input, want) in [
            ("", ""),
            ("f", "Zg=="),
            ("fo", "Zm8="),
            ("foo", "Zm9v"),
            ("foob", "Zm9vYg=="),
            ("fooba", "Zm9vYmE="),
            ("foobar", "Zm9vYmFy"),
        ] {
            assert_eq!(base64(input.as_bytes()), want);
        }
    }

    #[test]
    fn credentials_are_redacted_and_validated() {
        let a = HttpAuth::Basic {
            username: "admin".into(),
            password: "hunter2".into(),
        };
        assert!(!format!("{a:?}").contains("hunter2"));
        assert!(!format!("{:?}", HttpAuth::Bearer("sekret".into())).contains("sekret"));
        assert_eq!(a.header_value(), "Basic YWRtaW46aHVudGVyMg==");
        let cfg = HttpDriverConfig::new("10.0.0.9:80".parse().unwrap());
        assert!(cfg
            .clone()
            .auth(HttpAuth::Bearer("a b".into()))
            .validate()
            .is_err());
        assert!(cfg
            .clone()
            .auth(HttpAuth::Basic {
                username: "a:b".into(),
                password: "x".into()
            })
            .validate()
            .is_err());
        assert!(cfg.auth(HttpAuth::Bearer("tok".into())).validate().is_ok());
    }

    #[test]
    fn rejects_unsafe_config() {
        let c = HttpDriverConfig::new("10.0.0.9:80".parse().unwrap());
        assert!(c
            .clone()
            .bind("power_on", HttpCommand::new("TRACE", "/x"))
            .validate()
            .is_err());
        assert!(c
            .clone()
            .bind("power_on", HttpCommand::new("POST", "no-slash"))
            .validate()
            .is_err());
        assert!(c
            .clone()
            .query(HttpQuery::new("p", "/state/$input"))
            .validate()
            .is_err());
        assert!(HttpDriverConfig::new("0.0.0.0:80".parse().unwrap())
            .validate()
            .is_err());
    }
}
