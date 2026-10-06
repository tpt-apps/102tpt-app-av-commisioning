//! Shared core for text-protocol drivers (TCP, UDP, serial, WebSocket).
//!
//! A text device is driven the same way whatever carries the bytes: send a
//! short message, optionally read an acknowledgement; send a query, read and
//! parse the reply. [`TextDriver`] implements that once, over an [`Opener`]
//! that supplies a fresh [`Exchange`] (one open conversation) per operation.
//! Per-transport modules only say how to open and move lines.
//!
//! Safety rules, common to every text transport:
//!
//! * **No command injection.** Values substituted into a command template
//!   (`$input`, `$pattern`, …) must be short, printable and single-line; a
//!   line break would otherwise smuggle a second command to the device. Bad
//!   values are rejected before anything is sent.
//! * **Bounded.** A received line may not exceed [`MAX_LINE_BYTES`]; blank
//!   lines are skipped only up to a fixed count; every wait uses the
//!   configured timeout.
//! * **Typed replies.** A reply is trimmed, an optional prefix stripped, an
//!   optional JSON Pointer applied, and a built-in parser produces a typed
//!   value. Anything that does not fit is a `MalformedResponse`, never a guess.

use std::collections::BTreeMap;
use std::io::{Read, Write};
use std::net::UdpSocket;
use std::time::{Duration, Instant};

use tpt_app_av_commissioning_device::{
    DeviceCapabilities, DeviceCommand, DeviceIdentity, DeviceResponse, DeviceState, StateValue,
};
use tpt_app_av_commissioning_driver::net::map_io_error;
use tpt_app_av_commissioning_driver::{DeviceDriver, DriverError};
use tpt_app_av_commissioning_profile::{
    placeholders_in, substitute, DeviceProfile, ParserKind, Placeholder, Terminator,
};

/// Longest line (or message) accepted from a device.
pub const MAX_LINE_BYTES: usize = 4096;
/// Blank lines or datagrams skipped while waiting for one reply.
pub const MAX_BLANK_LINES: usize = 16;
/// Longest substituted value.
pub const MAX_VALUE_LEN: usize = 128;
/// Longest command text.
pub const MAX_COMMAND_LEN: usize = 256;

/// A command's text and its optional acknowledgement.
#[derive(Debug, Clone, PartialEq)]
pub struct TextCommand {
    /// Command text, possibly with `$placeholders`.
    pub send: String,
    /// Reply line that acknowledges it; `None` means fire-and-forget.
    pub ack: Option<String>,
}

impl TextCommand {
    pub fn new(send: impl Into<String>) -> Self {
        Self {
            send: send.into(),
            ack: None,
        }
    }

    pub fn ack(mut self, ack: impl Into<String>) -> Self {
        self.ack = Some(ack.into());
        self
    }
}

/// A state field read by sending `query` and parsing the reply.
#[derive(Debug, Clone, PartialEq)]
pub struct TextQuery {
    pub field: String,
    pub query: String,
    pub parser: ParserKind,
    pub strip_prefix: Option<String>,
    /// JSON Pointer into a JSON reply.
    pub extract: Option<String>,
}

impl TextQuery {
    pub fn new(field: impl Into<String>, query: impl Into<String>) -> Self {
        Self {
            field: field.into(),
            query: query.into(),
            parser: ParserKind::Auto,
            strip_prefix: None,
            extract: None,
        }
    }

    pub fn parser(mut self, parser: ParserKind) -> Self {
        self.parser = parser;
        self
    }

    pub fn strip_prefix(mut self, prefix: impl Into<String>) -> Self {
        self.strip_prefix = Some(prefix.into());
        self
    }

    pub fn extract(mut self, pointer: impl Into<String>) -> Self {
        self.extract = Some(pointer.into());
        self
    }
}

/// One open conversation with a device.
pub trait Exchange {
    /// Send one message (the transport adds its framing).
    fn send_line(&mut self, text: &str) -> Result<(), DriverError>;
    /// Receive the next non-blank message, within the configured timeout.
    fn read_line(&mut self) -> Result<String, DriverError>;
}

/// Opens a fresh [`Exchange`] for each operation.
pub trait Opener {
    fn open(&self) -> Result<Box<dyn Exchange + '_>, DriverError>;
}

/// What a profile contributes to every text driver.
#[derive(Debug, Clone, Default)]
pub struct TextSpec {
    pub identity: DeviceIdentity,
    /// Keyed by command kind (`power_on`, `set_input`, …).
    pub commands: BTreeMap<String, TextCommand>,
    pub queries: Vec<TextQuery>,
}

impl TextSpec {
    /// Lift commands, state queries and identity out of a profile. `args` is
    /// an OSC concept and is refused here.
    pub fn from_profile(profile: &DeviceProfile) -> Result<Self, DriverError> {
        let spec = &profile.device;
        let mut out = Self {
            identity: DeviceIdentity {
                manufacturer: Some(spec.matcher.manufacturer.clone()),
                model: spec.matcher.model.as_slice().first().cloned(),
                ..DeviceIdentity::default()
            },
            ..Self::default()
        };
        for (name, cmd) in &spec.commands {
            if !cmd.args.is_empty() {
                return Err(DriverError::Config(format!(
                    "commands.{name}: text protocols embed placeholders in `send`; `args` is for OSC"
                )));
            }
            if cmd.body.is_some() {
                return Err(DriverError::Config(format!(
                    "commands.{name}: `body` is only for HTTP"
                )));
            }
            out.commands.insert(
                name.clone(),
                TextCommand {
                    send: cmd.send.clone(),
                    ack: cmd.ack.clone(),
                },
            );
        }
        for (field, q) in &spec.state {
            out.queries.push(TextQuery {
                field: field.clone(),
                query: q.query.clone(),
                parser: q.parser.unwrap_or_default(),
                strip_prefix: q.strip_prefix.clone(),
                extract: q.extract.clone(),
            });
        }
        Ok(out)
    }

    /// Check commands and queries are printable, single-line and coherent.
    pub fn validate(&self) -> Result<(), DriverError> {
        for (name, cmd) in &self.commands {
            printable(&format!("commands.{name}.send"), &cmd.send)?;
            if let Some(ack) = &cmd.ack {
                printable(&format!("commands.{name}.ack"), ack)?;
            }
        }
        for q in &self.queries {
            printable(&format!("state.{}.query", q.field), &q.query)?;
            if !placeholders_in(&q.query).is_empty() {
                return Err(DriverError::Config(format!(
                    "state.{}.query may not contain placeholders",
                    q.field
                )));
            }
        }
        Ok(())
    }

    /// Capabilities implied by what is bound and queried.
    pub fn capabilities(&self) -> DeviceCapabilities {
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

pub(crate) fn printable(what: &str, text: &str) -> Result<(), DriverError> {
    if text.trim().is_empty() || text.len() > MAX_COMMAND_LEN || text.chars().any(char::is_control)
    {
        return Err(DriverError::Config(format!(
            "{what} must be 1-{MAX_COMMAND_LEN} printable characters on one line"
        )));
    }
    Ok(())
}

/// A text-protocol device: a [`TextSpec`] plus a way to open conversations.
pub struct TextDriver<O: Opener> {
    opener: O,
    spec: TextSpec,
    capabilities: DeviceCapabilities,
}

impl<O: Opener> TextDriver<O> {
    /// Validate `spec`. Nothing is opened until an operation runs.
    pub fn with_opener(opener: O, spec: TextSpec) -> Result<Self, DriverError> {
        spec.validate()?;
        let capabilities = spec.capabilities();
        Ok(Self {
            opener,
            spec,
            capabilities,
        })
    }

    pub fn opener(&self) -> &O {
        &self.opener
    }

    pub fn spec(&self) -> &TextSpec {
        &self.spec
    }

    fn read_state(&self) -> Result<DeviceState, DriverError> {
        if self.spec.queries.is_empty() {
            return Err(DriverError::UnsupportedOperation);
        }
        let mut conn = self.opener.open()?;
        let mut state = DeviceState::new();
        for q in &self.spec.queries {
            conn.send_line(&q.query)?;
            let reply = conn.read_line()?;
            let value = reply_to_value(
                &reply,
                q.parser,
                q.strip_prefix.as_deref(),
                q.extract.as_deref(),
            )
            .map_err(|e| DriverError::MalformedResponse(format!("{}: {e}", q.field)))?;
            state.set(q.field.clone(), value);
        }
        Ok(state)
    }

    fn send_command(&self, text: &str, ack: Option<&str>) -> Result<DeviceResponse, DriverError> {
        let started = Instant::now();
        let mut conn = self.opener.open()?;
        conn.send_line(text)?;
        let (ok, message) = match ack {
            None => (
                true,
                format!("sent {text:?}; no acknowledgement expected, verify by reading state"),
            ),
            Some(expected) => {
                let reply = conn.read_line()?;
                if reply.trim().eq_ignore_ascii_case(expected.trim()) {
                    (true, format!("sent {text:?}; acknowledged"))
                } else {
                    (
                        false,
                        format!(
                            "sent {text:?}; expected acknowledgement {expected:?}, got {reply:?}"
                        ),
                    )
                }
            }
        };
        Ok(DeviceResponse {
            ok,
            state: None,
            message: Some(message),
            response_time_ms: Some(started.elapsed().as_millis() as u64),
        })
    }
}

impl<O: Opener> DeviceDriver for TextDriver<O> {
    fn identity(&self) -> DeviceIdentity {
        self.spec.identity.clone()
    }

    /// Discovery is a state read: a device that answers its queries is there.
    fn discover(&mut self) -> Result<DeviceState, DriverError> {
        if self.spec.queries.is_empty() {
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
        match &command {
            DeviceCommand::ReadState => {
                let state = self.read_state()?;
                Ok(DeviceResponse {
                    ok: true,
                    state: Some(state),
                    message: None,
                    response_time_ms: None,
                })
            }
            DeviceCommand::Arbitrary { command: text } => {
                if text.trim().is_empty()
                    || text.len() > MAX_COMMAND_LEN
                    || text.chars().any(char::is_control)
                {
                    return Err(DriverError::Config(format!(
                        "arbitrary command must be 1-{MAX_COMMAND_LEN} printable characters on one line"
                    )));
                }
                self.send_command(text, None)
            }
            other => {
                let bound = self
                    .spec
                    .commands
                    .get(kind_name(other))
                    .ok_or(DriverError::UnsupportedOperation)?;
                let text = render(&bound.send, other, Escape::None)?;
                self.send_command(&text, bound.ack.as_deref())
            }
        }
    }

    fn capabilities(&self) -> DeviceCapabilities {
        self.capabilities
    }
}

/// The command kind name used by profiles.
pub fn kind_name(command: &DeviceCommand) -> &'static str {
    match command {
        DeviceCommand::PowerOn => "power_on",
        DeviceCommand::PowerOff => "power_off",
        DeviceCommand::PowerCycle => "power_cycle",
        DeviceCommand::SetInput { .. } => "set_input",
        DeviceCommand::Freeze { .. } => "freeze",
        DeviceCommand::GenerateTestPattern { .. } => "generate_test_pattern",
        DeviceCommand::SetAudioVolume { .. } => "set_audio_volume",
        DeviceCommand::SetAudioMute { .. } => "set_audio_mute",
        DeviceCommand::SetRoute { .. } => "set_route",
        DeviceCommand::ReadState => "read_state",
        DeviceCommand::ReadEdid => "read_edid",
        DeviceCommand::MeasureLatency => "measure_latency",
        DeviceCommand::Arbitrary { .. } => "arbitrary",
    }
}

/// How a substituted text value is escaped for where it lands.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Escape {
    /// Plain text (line protocols).
    None,
    /// Percent-encoded (URL paths).
    Url,
    /// JSON string content, without the surrounding quotes (request bodies).
    Json,
}

fn safe_value(what: &str, value: &str) -> Result<String, String> {
    if value.is_empty() || value.len() > MAX_VALUE_LEN || value.chars().any(char::is_control) {
        return Err(format!(
            "{what} must be 1-{MAX_VALUE_LEN} printable characters on one line"
        ));
    }
    Ok(value.to_owned())
}

fn escape(value: &str, how: Escape) -> String {
    match how {
        Escape::None => value.to_owned(),
        Escape::Url => value
            .bytes()
            .map(|b| match b {
                b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                    (b as char).to_string()
                }
                _ => format!("%{b:02X}"),
            })
            .collect(),
        Escape::Json => {
            let quoted = serde_json::Value::String(value.to_owned()).to_string();
            quoted[1..quoted.len() - 1].to_owned()
        }
    }
}

/// Fill a template from the command being executed. Text values are escaped
/// for their destination; numbers and flags are inserted as-is.
pub fn render(template: &str, command: &DeviceCommand, how: Escape) -> Result<String, DriverError> {
    let text = |what: &str, v: &str| safe_value(what, v).map(|v| escape(&v, how));
    substitute(template, |ph| match (ph, command) {
        (Placeholder::Input, DeviceCommand::SetInput { input }) => text("input", input),
        (Placeholder::Pattern, DeviceCommand::GenerateTestPattern { pattern }) => {
            text("pattern", pattern)
        }
        (Placeholder::Source, DeviceCommand::SetRoute { source, .. }) => text("source", source),
        (Placeholder::Destination, DeviceCommand::SetRoute { destination, .. }) => {
            text("destination", destination)
        }
        (Placeholder::LevelDb, DeviceCommand::SetAudioVolume { level_db }) => {
            if level_db.is_finite() {
                Ok(level_db.to_string())
            } else {
                Err("level_db must be a finite number".to_owned())
            }
        }
        (Placeholder::Muted, DeviceCommand::SetAudioMute { muted }) => {
            Ok(if *muted { "1" } else { "0" }.to_owned())
        }
        (Placeholder::Frozen, DeviceCommand::Freeze { frozen }) => {
            Ok(if *frozen { "1" } else { "0" }.to_owned())
        }
        (ph, _) => Err(format!(
            "{ph:?} placeholder does not apply to {}",
            kind_name(command)
        )),
    })
    .map_err(DriverError::Config)
}

/// Turn a reply into a typed value: strip the prefix, apply the JSON Pointer
/// if any, then parse.
pub fn reply_to_value(
    reply: &str,
    parser: ParserKind,
    strip_prefix: Option<&str>,
    extract: Option<&str>,
) -> Result<StateValue, String> {
    let mut text = reply.trim();
    if let Some(prefix) = strip_prefix {
        text = text
            .strip_prefix(prefix)
            .ok_or_else(|| format!("reply {text:?} does not start with {prefix:?}"))?;
    }
    let Some(pointer) = extract else {
        return parser.parse(text);
    };
    let json: serde_json::Value =
        serde_json::from_str(text).map_err(|e| format!("reply is not valid JSON: {e}"))?;
    match json.pointer(pointer) {
        None => Err(format!("JSON reply has nothing at {pointer}")),
        Some(serde_json::Value::Bool(b)) => Ok(StateValue::Boolean(*b)),
        Some(serde_json::Value::Number(n)) => {
            if let Some(i) = n.as_i64() {
                Ok(StateValue::Integer(i))
            } else {
                n.as_f64()
                    .filter(|f| f.is_finite())
                    .map(StateValue::Float)
                    .ok_or_else(|| format!("{pointer}: number out of range"))
            }
        }
        Some(serde_json::Value::String(s)) => parser.parse(s),
        Some(other) => Err(format!(
            "{pointer}: expected a scalar, found {}",
            match other {
                serde_json::Value::Null => "null",
                serde_json::Value::Array(_) => "an array",
                _ => "an object",
            }
        )),
    }
}

// ---------------------------------------------------------------------------
// Transports
// ---------------------------------------------------------------------------

/// A byte stream whose read timeout can be set between reads.
pub trait Transport: Read + Write {
    fn set_read_timeout(&mut self, timeout: Duration) -> std::io::Result<()>;
}

impl Transport for std::net::TcpStream {
    fn set_read_timeout(&mut self, timeout: Duration) -> std::io::Result<()> {
        std::net::TcpStream::set_read_timeout(self, Some(timeout))
    }
}

/// Line framing over a byte stream (TCP, serial).
pub struct StreamExchange<S: Transport> {
    stream: S,
    buffer: Vec<u8>,
    terminator: Terminator,
    timeout: Duration,
    label: String,
}

impl<S: Transport> StreamExchange<S> {
    pub fn new(stream: S, terminator: Terminator, timeout: Duration, label: String) -> Self {
        Self {
            stream,
            buffer: Vec::new(),
            terminator,
            timeout,
            label,
        }
    }

    fn io_error(&self, e: &std::io::Error) -> DriverError {
        map_io_error(e, &self.label, self.timeout)
    }
}

impl<S: Transport> Exchange for StreamExchange<S> {
    fn send_line(&mut self, text: &str) -> Result<(), DriverError> {
        let mut bytes = Vec::with_capacity(text.len() + 2);
        bytes.extend_from_slice(text.as_bytes());
        bytes.extend_from_slice(self.terminator.bytes());
        self.stream
            .write_all(&bytes)
            .and_then(|_| self.stream.flush())
            .map_err(|e| self.io_error(&e))
    }

    fn read_line(&mut self) -> Result<String, DriverError> {
        let deadline = Instant::now() + self.timeout;
        // A stream with no terminator cannot be split into lines.
        let end = if self.terminator == Terminator::None {
            b'\n'
        } else {
            self.terminator.end_byte()
        };
        let mut chunk = [0u8; 512];
        let mut blanks = 0;
        loop {
            if let Some(pos) = self.buffer.iter().position(|b| *b == end) {
                let raw: Vec<u8> = self.buffer.drain(..=pos).collect();
                let line = String::from_utf8(raw).map_err(|_| {
                    DriverError::MalformedResponse("reply is not valid UTF-8".to_owned())
                })?;
                let line = line.trim_matches(|c| c == '\r' || c == '\n').trim();
                if !line.is_empty() {
                    return Ok(line.to_owned());
                }
                blanks += 1;
                if blanks > MAX_BLANK_LINES {
                    return Err(DriverError::Protocol(
                        "too many blank lines while waiting for a reply".to_owned(),
                    ));
                }
                continue;
            }
            if self.buffer.len() > MAX_LINE_BYTES {
                return Err(DriverError::MalformedResponse(format!(
                    "reply line exceeds {MAX_LINE_BYTES} bytes"
                )));
            }
            let remaining = deadline.saturating_duration_since(Instant::now());
            if remaining.is_zero() {
                return Err(DriverError::Timeout(self.timeout.as_millis() as u64));
            }
            self.stream
                .set_read_timeout(remaining)
                .map_err(|e| self.io_error(&e))?;
            match self.stream.read(&mut chunk) {
                Ok(0) => {
                    return Err(if self.buffer.is_empty() {
                        DriverError::Unreachable(format!(
                            "{}: connection closed before a reply",
                            self.label
                        ))
                    } else {
                        DriverError::MalformedResponse(
                            "connection closed in the middle of a line".to_owned(),
                        )
                    })
                }
                Ok(n) => self.buffer.extend_from_slice(&chunk[..n]),
                Err(e) => return Err(self.io_error(&e)),
            }
        }
    }
}

/// One message per datagram (UDP).
pub struct DatagramExchange {
    socket: UdpSocket,
    terminator: Terminator,
    timeout: Duration,
    label: String,
}

impl DatagramExchange {
    /// `socket` must already be connected to the device.
    pub fn new(
        socket: UdpSocket,
        terminator: Terminator,
        timeout: Duration,
        label: String,
    ) -> Self {
        Self {
            socket,
            terminator,
            timeout,
            label,
        }
    }

    fn io_error(&self, e: &std::io::Error) -> DriverError {
        map_io_error(e, &self.label, self.timeout)
    }
}

impl Exchange for DatagramExchange {
    fn send_line(&mut self, text: &str) -> Result<(), DriverError> {
        let mut bytes = Vec::with_capacity(text.len() + 2);
        bytes.extend_from_slice(text.as_bytes());
        bytes.extend_from_slice(self.terminator.bytes());
        self.socket
            .send(&bytes)
            .map(|_| ())
            .map_err(|e| self.io_error(&e))
    }

    fn read_line(&mut self) -> Result<String, DriverError> {
        let deadline = Instant::now() + self.timeout;
        // Read into a buffer big enough for any UDP datagram (Windows reports
        // an error rather than truncating), then enforce the line limit.
        let mut buf = vec![0u8; 65_507];
        let mut skipped = 0;
        loop {
            let remaining = deadline.saturating_duration_since(Instant::now());
            if remaining.is_zero() {
                return Err(DriverError::Timeout(self.timeout.as_millis() as u64));
            }
            self.socket
                .set_read_timeout(Some(remaining))
                .map_err(|e| self.io_error(&e))?;
            let len = self.socket.recv(&mut buf).map_err(|e| self.io_error(&e))?;
            if len > MAX_LINE_BYTES {
                return Err(DriverError::MalformedResponse(format!(
                    "datagram exceeds {MAX_LINE_BYTES} bytes"
                )));
            }
            let text = std::str::from_utf8(&buf[..len]).map_err(|_| {
                DriverError::MalformedResponse("reply is not valid UTF-8".to_owned())
            })?;
            let text = text.trim();
            if !text.is_empty() {
                return Ok(text.to_owned());
            }
            skipped += 1;
            if skipped > MAX_BLANK_LINES {
                return Err(DriverError::Protocol(
                    "too many blank datagrams while waiting for a reply".to_owned(),
                ));
            }
        }
    }
}

/// Builder methods shared by every text-driver config that has `timeout`,
/// `terminator` and `spec: TextSpec` fields.
macro_rules! text_config_builders {
    () => {
        pub fn timeout(mut self, timeout: Duration) -> Self {
            self.timeout = timeout;
            self
        }

        pub fn terminator(mut self, terminator: Terminator) -> Self {
            self.terminator = terminator;
            self
        }

        pub fn identity(mut self, identity: DeviceIdentity) -> Self {
            self.spec.identity = identity;
            self
        }

        /// Bind a command kind (`power_on`, `set_input`, ...) to a message.
        pub fn bind(mut self, command_kind: impl Into<String>, command: TextCommand) -> Self {
            self.spec.commands.insert(command_kind.into(), command);
            self
        }

        pub fn query(mut self, query: TextQuery) -> Self {
            self.spec.queries.push(query);
            self
        }
    };
}
pub(crate) use text_config_builders;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn renders_templates_and_refuses_injection() {
        let set = |i: &str| DeviceCommand::SetInput { input: i.into() };
        assert_eq!(
            render("INPT $input", &set("hdmi2"), Escape::None).unwrap(),
            "INPT hdmi2"
        );
        for evil in [
            "hdmi1\r\nPOWR 0",
            "a\nb",
            "",
            &"x".repeat(MAX_VALUE_LEN + 1),
        ] {
            assert!(
                render("INPT $input", &set(evil), Escape::None).is_err(),
                "{evil:?}"
            );
        }
        assert_eq!(
            render(
                "VOL $level_db",
                &DeviceCommand::SetAudioVolume { level_db: -6.5 },
                Escape::None
            )
            .unwrap(),
            "VOL -6.5"
        );
        assert!(render(
            "VOL $level_db",
            &DeviceCommand::SetAudioVolume { level_db: f64::NAN },
            Escape::None
        )
        .is_err());
        assert_eq!(
            render(
                "MUTE $muted",
                &DeviceCommand::SetAudioMute { muted: true },
                Escape::None
            )
            .unwrap(),
            "MUTE 1"
        );
        assert!(render("INPT $input", &DeviceCommand::PowerOn, Escape::None).is_err());
    }

    #[test]
    fn escaping_is_per_destination() {
        let set = |i: &str| DeviceCommand::SetInput { input: i.into() };
        assert_eq!(
            render("/in/$input", &set("HDMI 1/a?b"), Escape::Url).unwrap(),
            "/in/HDMI%201%2Fa%3Fb"
        );
        // A quote cannot break out of the JSON string.
        assert_eq!(
            render(
                r#"{"input":"$input"}"#,
                &set(r#"x","admin":true,"y":"1"#),
                Escape::Json
            )
            .unwrap(),
            r#"{"input":"x\",\"admin\":true,\"y\":\"1"}"#
        );
    }

    #[test]
    fn replies_become_typed_values() {
        use ParserKind::*;
        let v = |r, p, pre, ex| reply_to_value(r, p, pre, ex);
        assert_eq!(
            v("POWR=ON", PowerState, Some("POWR="), None),
            Ok(StateValue::Boolean(true))
        );
        assert!(v("XXXX=ON", PowerState, Some("POWR="), None).is_err());
        let json = r#"{"power":{"state":"on"},"vol":-12.5,"n":3,"mute":false,"o":{}}"#;
        assert_eq!(
            v(json, Bool, None, Some("/power/state")),
            Ok(StateValue::Boolean(true))
        );
        assert_eq!(
            v(json, Auto, None, Some("/vol")),
            Ok(StateValue::Float(-12.5))
        );
        assert_eq!(v(json, Auto, None, Some("/n")), Ok(StateValue::Integer(3)));
        assert_eq!(
            v(json, Auto, None, Some("/mute")),
            Ok(StateValue::Boolean(false))
        );
        assert!(v(json, Auto, None, Some("/missing")).is_err());
        assert!(v(json, Auto, None, Some("/o")).is_err());
        assert!(v("not json", Auto, None, Some("/x")).is_err());
    }
}
