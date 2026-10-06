//! TPT AV Commissioning — versioned device profiles (§41).
//!
//! A profile describes how to talk to a family of devices as *data*: which
//! manufacturer/model it applies to, the protocol and port, the message each
//! typed command sends, and the queries that build device state. Integrators
//! and vendors can add devices without writing code; drivers turn a profile
//! plus a host address into a working driver (see `drivers/osc`).
//!
//! ```yaml
//! schema_version: 1
//! device:
//!   id: example-projector
//!   match:
//!     manufacturer: Example
//!     model: Beam-1          # or a list of models
//!   protocol:
//!     type: osc
//!     port: 9000
//!     timeout_ms: 500
//!   commands:
//!     power_on:  { send: /power, args: [true] }
//!     set_input: { send: /input, args: ["$input"] }
//!   state:
//!     power: { query: /power }
//! ```
//!
//! Rules (§36, §41):
//!
//! * `schema_version` is mandatory and must be one this build understands.
//! * Profiles are declarative. Custom YAML tags, unknown fields and
//!   unparseable input are rejected; nothing in a profile is ever executed.
//!   Placeholders (`"$input"`) only name values taken from the command being
//!   run, and each applies to one command only.
//! * A profile is only applied to a device whose identity matches its
//!   `match` section ([`DeviceProfile::matches`]).
//! * Sizes are bounded, and credentials have no place in a profile.
//!
//! Licensed under either of MIT OR Apache-2.0, at your option.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use tpt_app_av_commissioning_device::{DeviceIdentity, StateValue};

/// The schema version this build reads and writes.
pub const SCHEMA_VERSION: u32 = 1;
/// Largest profile document accepted.
pub const MAX_PROFILE_BYTES: usize = 256 * 1024;
/// Most commands, state fields, or arguments per command accepted.
pub const MAX_ENTRIES: usize = 64;
/// Most arguments one command may carry.
pub const MAX_ARGS: usize = 16;
/// Longest accepted timeout.
pub const MAX_TIMEOUT_MS: u64 = 30_000;

/// Command names a profile may bind, matching the typed `DeviceCommand` kinds.
pub const COMMAND_KINDS: [&str; 11] = [
    "power_on",
    "power_off",
    "power_cycle",
    "set_input",
    "freeze",
    "generate_test_pattern",
    "set_audio_volume",
    "set_audio_mute",
    "set_route",
    "read_edid",
    "measure_latency",
];

/// Why a profile was rejected.
#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum ProfileError {
    #[error("profile is larger than {MAX_PROFILE_BYTES} bytes")]
    TooLarge,
    #[error("profile contains executable content and was rejected")]
    ExecutableContent,
    #[error("profile failed to parse: {0}")]
    Parse(String),
    #[error("unsupported schema_version {found}; this build reads version {SCHEMA_VERSION}")]
    UnsupportedVersion { found: u32 },
    #[error("profile failed validation: {0}")]
    Validation(String),
}

/// A parsed, validated profile document.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DeviceProfile {
    pub schema_version: u32,
    pub device: DeviceSpec,
}

/// The device a profile describes.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DeviceSpec {
    /// Stable profile id (lowercase letters, digits, `-`, `_`).
    pub id: String,
    #[serde(rename = "match")]
    pub matcher: MatchSpec,
    pub protocol: ProtocolSpec,
    /// Command name (see [`COMMAND_KINDS`]) → message.
    #[serde(default)]
    pub commands: BTreeMap<String, CommandSpec>,
    /// State field name → query.
    #[serde(default)]
    pub state: BTreeMap<String, StateSpec>,
}

/// Which devices a profile applies to.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MatchSpec {
    pub manufacturer: String,
    pub model: OneOrMany,
}

/// A single string, or a list of them.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum OneOrMany {
    One(String),
    Many(Vec<String>),
}

impl OneOrMany {
    pub fn as_slice(&self) -> &[String] {
        match self {
            OneOrMany::One(s) => std::slice::from_ref(s),
            OneOrMany::Many(v) => v,
        }
    }
}

/// Transport a profile's device speaks.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProtocolKind {
    Osc,
    Midi,
    Tcp,
    Udp,
    Http,
    Websocket,
    Serial,
    Snmp,
}

/// Protocol settings.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ProtocolSpec {
    #[serde(rename = "type")]
    pub kind: ProtocolKind,
    /// Network port (network protocols).
    #[serde(default)]
    pub port: Option<u16>,
    /// Per-request timeout in milliseconds (default 1000).
    #[serde(default)]
    pub timeout_ms: Option<u64>,
    /// Line terminator for text protocols (default `crlf`).
    #[serde(default)]
    pub terminator: Option<Terminator>,
    /// WebSocket request path (default `/`).
    #[serde(default)]
    pub path: Option<String>,
    /// Serial line settings (required for, and only valid with, `serial`).
    #[serde(default)]
    pub serial: Option<SerialSpec>,
}

/// Serial line settings.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SerialSpec {
    pub baud: u32,
    /// 5, 6, 7 or 8 (default 8).
    #[serde(default)]
    pub data_bits: Option<u8>,
    /// Default `none`.
    #[serde(default)]
    pub parity: Option<Parity>,
    /// 1 or 2 (default 1).
    #[serde(default)]
    pub stop_bits: Option<u8>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Parity {
    None,
    Even,
    Odd,
}

/// How lines end on a text protocol.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Terminator {
    Lf,
    Crlf,
    Cr,
    /// No terminator: each message stands alone (datagram and message
    /// transports such as UDP and WebSocket).
    None,
}

impl Terminator {
    /// The bytes appended to every sent line.
    pub fn bytes(&self) -> &'static [u8] {
        match self {
            Terminator::Lf => b"\n",
            Terminator::Crlf => b"\r\n",
            Terminator::Cr => b"\r",
            Terminator::None => b"",
        }
    }

    /// The byte that ends a received line.
    pub fn end_byte(&self) -> u8 {
        match self {
            Terminator::Lf | Terminator::Crlf => b'\n',
            Terminator::Cr => b'\r',
            // Never used to split a stream; message transports are not split.
            Terminator::None => b'\n',
        }
    }
}

impl ProtocolSpec {
    /// The timeout in milliseconds, defaulted.
    pub fn timeout_ms_or_default(&self) -> u64 {
        self.timeout_ms.unwrap_or(1000)
    }

    /// The line terminator, defaulted.
    pub fn terminator_or_default(&self) -> Terminator {
        self.terminator.unwrap_or(Terminator::Crlf)
    }
}

/// A command's message.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CommandSpec {
    /// What to send: an OSC address, or protocol text. Text protocols may
    /// embed `$placeholders` (e.g. `INPUT $input`).
    pub send: String,
    #[serde(default)]
    pub args: Vec<ProfileArg>,
    /// Text protocols: the reply line that acknowledges the command. When
    /// absent the command is fire-and-forget.
    #[serde(default)]
    pub ack: Option<String>,
    /// HTTP: the request body. Text placeholders go inside JSON quotes and
    /// are JSON-escaped; numeric and boolean ones are inserted bare.
    #[serde(default)]
    pub body: Option<String>,
}

/// A state field's query.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct StateSpec {
    /// What to ask: an OSC address, or protocol text.
    pub query: String,
    /// How a text reply becomes a typed value (default `auto`). Unused by
    /// OSC, which returns typed values.
    #[serde(default)]
    pub parser: Option<ParserKind>,
    /// Text protocols: a prefix to strip from the reply before parsing
    /// (e.g. `POWR=`). A reply without it is a malformed response.
    #[serde(default)]
    pub strip_prefix: Option<String>,
    /// JSON replies: a JSON Pointer (`/power/state`) selecting the value.
    /// Strings then go through `parser`; numbers and booleans keep their type.
    #[serde(default)]
    pub extract: Option<String>,
}

/// Built-in reply parsers. Closed on purpose: a profile selects behaviour, it
/// never supplies code.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ParserKind {
    /// Boolean for on/off/true/false, else integer, else float, else text.
    #[default]
    Auto,
    Bool,
    Int,
    Float,
    Text,
    /// `on`/`power on`/`1` is true; `off`/`power off`/`standby`/`0` is false;
    /// transitional states such as `warming` and `cooling` stay text so a test
    /// expecting a boolean fails visibly rather than guessing.
    PowerState,
}

impl ParserKind {
    /// Parse one reply (prefix already stripped).
    pub fn parse(&self, reply: &str) -> Result<StateValue, String> {
        let t = reply.trim();
        let lower = t.to_ascii_lowercase();
        let as_bool = || match lower.as_str() {
            "on" | "true" | "1" | "yes" => Some(true),
            "off" | "false" | "0" | "no" => Some(false),
            _ => None,
        };
        match self {
            ParserKind::Text => Ok(StateValue::Text(t.to_owned())),
            ParserKind::Bool => as_bool()
                .map(StateValue::Boolean)
                .ok_or_else(|| format!("{t:?} is not a boolean")),
            ParserKind::Int => t
                .parse::<i64>()
                .map(StateValue::Integer)
                .map_err(|_| format!("{t:?} is not an integer")),
            ParserKind::Float => t
                .parse::<f64>()
                .ok()
                .filter(|f| f.is_finite())
                .map(StateValue::Float)
                .ok_or_else(|| format!("{t:?} is not a number")),
            ParserKind::PowerState => Ok(match lower.as_str() {
                "on" | "power on" | "1" => StateValue::Boolean(true),
                "off" | "power off" | "standby" | "0" => StateValue::Boolean(false),
                _ => StateValue::Text(t.to_owned()),
            }),
            ParserKind::Auto => {
                // "0"/"1" are numbers under `auto`; use `bool` to read them as flags.
                if let Some(b) = as_bool().filter(|_| !matches!(lower.as_str(), "0" | "1")) {
                    Ok(StateValue::Boolean(b))
                } else if let Ok(i) = t.parse::<i64>() {
                    Ok(StateValue::Integer(i))
                } else if let Some(f) = t.parse::<f64>().ok().filter(|f| f.is_finite()) {
                    Ok(StateValue::Float(f))
                } else {
                    Ok(StateValue::Text(t.to_owned()))
                }
            }
        }
    }
}

/// A command argument: a literal, or a `$placeholder` filled from the command.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum ProfileArg {
    Bool(bool),
    Int(i32),
    Float(f32),
    Text(String),
}

/// A value taken from the command being executed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Placeholder {
    Input,
    Pattern,
    LevelDb,
    Muted,
    Frozen,
    Source,
    Destination,
}

impl Placeholder {
    /// Parse `$name`; `None` when it is not a placeholder at all.
    pub fn parse(text: &str) -> Option<Result<Self, String>> {
        let name = text.strip_prefix('$')?;
        Some(match name {
            "input" => Ok(Self::Input),
            "pattern" => Ok(Self::Pattern),
            "level_db" => Ok(Self::LevelDb),
            "muted" => Ok(Self::Muted),
            "frozen" => Ok(Self::Frozen),
            "source" => Ok(Self::Source),
            "destination" => Ok(Self::Destination),
            other => Err(format!("unknown placeholder ${other}")),
        })
    }

    /// The only command this placeholder may appear in.
    pub fn command(&self) -> &'static str {
        match self {
            Self::Input => "set_input",
            Self::Pattern => "generate_test_pattern",
            Self::LevelDb => "set_audio_volume",
            Self::Muted => "set_audio_mute",
            Self::Frozen => "freeze",
            Self::Source | Self::Destination => "set_route",
        }
    }
}

/// Placeholders embedded in `text` with their byte ranges. A `$` not followed
/// by a lowercase letter is ordinary text.
fn scan_placeholders(text: &str) -> Vec<(std::ops::Range<usize>, Result<Placeholder, String>)> {
    let bytes = text.as_bytes();
    let mut out = Vec::new();
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'$' && bytes.get(i + 1).is_some_and(u8::is_ascii_lowercase) {
            let start = i;
            i += 1;
            while i < bytes.len() && (bytes[i].is_ascii_lowercase() || bytes[i] == b'_') {
                i += 1;
            }
            if let Some(found) = Placeholder::parse(&text[start..i]) {
                out.push((start..i, found));
            }
        } else {
            i += 1;
        }
    }
    out
}

/// The `$placeholders` embedded in `text`, in order. `Err` carries an unknown
/// name.
pub fn placeholders_in(text: &str) -> Vec<Result<Placeholder, String>> {
    scan_placeholders(text)
        .into_iter()
        .map(|(_, r)| r)
        .collect()
}

/// Replace every embedded placeholder in `text` with `value(placeholder)`.
/// Unknown placeholders and values the callback rejects are errors.
pub fn substitute(
    text: &str,
    mut value: impl FnMut(Placeholder) -> Result<String, String>,
) -> Result<String, String> {
    let mut out = String::with_capacity(text.len());
    let mut last = 0;
    for (range, found) in scan_placeholders(text) {
        out.push_str(&text[last..range.start]);
        out.push_str(&value(found?)?);
        last = range.end;
    }
    out.push_str(&text[last..]);
    Ok(out)
}

impl ProfileArg {
    /// The placeholder this argument names, if it is one.
    pub fn placeholder(&self) -> Option<Placeholder> {
        match self {
            ProfileArg::Text(t) => Placeholder::parse(t).and_then(Result::ok),
            _ => None,
        }
    }
}

impl DeviceProfile {
    /// Parse and validate a profile document.
    pub fn from_yaml_str(input: &str) -> Result<Self, ProfileError> {
        if input.len() > MAX_PROFILE_BYTES {
            return Err(ProfileError::TooLarge);
        }
        if contains_executable_content(input) {
            return Err(ProfileError::ExecutableContent);
        }
        // Read the version first so a future-format document reports "wrong
        // version" rather than an unrelated unknown-field error.
        #[derive(Deserialize)]
        struct Probe {
            schema_version: Option<u32>,
        }
        let probe: Probe =
            serde_yaml_ng::from_str(input).map_err(|e| ProfileError::Parse(e.to_string()))?;
        match probe.schema_version {
            None => {
                return Err(ProfileError::Validation(
                    "`schema_version` is required".to_owned(),
                ))
            }
            Some(v) if v != SCHEMA_VERSION => {
                return Err(ProfileError::UnsupportedVersion { found: v })
            }
            Some(_) => {}
        }
        let profile: DeviceProfile =
            serde_yaml_ng::from_str(input).map_err(|e| ProfileError::Parse(e.to_string()))?;
        profile.validate()?;
        Ok(profile)
    }

    /// Check the profile against the schema rules.
    pub fn validate(&self) -> Result<(), ProfileError> {
        let bad = |m: String| Err(ProfileError::Validation(m));
        if self.schema_version != SCHEMA_VERSION {
            return Err(ProfileError::UnsupportedVersion {
                found: self.schema_version,
            });
        }
        let d = &self.device;
        if d.id.is_empty()
            || d.id.len() > 64
            || !d
                .id
                .chars()
                .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-' || c == '_')
        {
            return bad(format!(
                "`id` {:?} must be 1-64 chars of lowercase letters, digits, '-' or '_'",
                d.id
            ));
        }
        if d.matcher.manufacturer.trim().is_empty() {
            return bad("`match.manufacturer` must not be empty".to_owned());
        }
        let models = d.matcher.model.as_slice();
        if models.is_empty() || models.iter().any(|m| m.trim().is_empty()) {
            return bad("`match.model` must name at least one non-empty model".to_owned());
        }
        if let Some(port) = d.protocol.port {
            if port == 0 {
                return bad("`protocol.port` must not be 0".to_owned());
            }
        }
        if let Some(t) = d.protocol.timeout_ms {
            if t == 0 || t > MAX_TIMEOUT_MS {
                return bad(format!(
                    "`protocol.timeout_ms` must be between 1 and {MAX_TIMEOUT_MS}"
                ));
            }
        }
        validate_protocol(d)?;
        if d.commands.is_empty() && d.state.is_empty() {
            return bad("a profile needs at least one command or state query".to_owned());
        }
        if d.commands.len() > MAX_ENTRIES || d.state.len() > MAX_ENTRIES {
            return bad(format!("at most {MAX_ENTRIES} commands and state fields"));
        }
        for (name, cmd) in &d.commands {
            if !COMMAND_KINDS.contains(&name.as_str()) {
                return bad(format!(
                    "unknown command `{name}` (known: {})",
                    COMMAND_KINDS.join(", ")
                ));
            }
            check_text(&format!("commands.{name}.send"), &cmd.send)?;
            for found in placeholders_in(&cmd.send) {
                let ph = found.map_err(ProfileError::Validation)?;
                if ph.command() != name {
                    return bad(format!(
                        "commands.{name}: {ph:?} placeholder may only be used in `{}`",
                        ph.command()
                    ));
                }
            }
            if let Some(ack) = &cmd.ack {
                check_text(&format!("commands.{name}.ack"), ack)?;
            }
            if let Some(body) = &cmd.body {
                if d.protocol.kind != ProtocolKind::Http {
                    return bad(format!("commands.{name}.body is only valid for `http`"));
                }
                if body.len() > 4096 {
                    return bad(format!("commands.{name}.body is longer than 4096 bytes"));
                }
                for found in placeholders_in(body) {
                    let ph = found.map_err(ProfileError::Validation)?;
                    if ph.command() != name {
                        return bad(format!(
                            "commands.{name}: {ph:?} placeholder may only be used in `{}`",
                            ph.command()
                        ));
                    }
                }
            }
            if d.protocol.kind == ProtocolKind::Http {
                http_request_line(&format!("commands.{name}.send"), &cmd.send)?;
            }
            if d.protocol.kind == ProtocolKind::Midi {
                MidiMessage::parse(&cmd.send)
                    .map_err(|e| ProfileError::Validation(format!("commands.{name}.send: {e}")))?;
            }
            if d.protocol.kind == ProtocolKind::Snmp {
                return bad(format!(
                    "commands.{name}: SNMP profiles are read-only; remove `commands`"
                ));
            }
            if cmd.args.len() > MAX_ARGS {
                return bad(format!("commands.{name}: at most {MAX_ARGS} args"));
            }
            for arg in &cmd.args {
                if let ProfileArg::Text(t) = arg {
                    if let Some(parsed) = Placeholder::parse(t) {
                        let ph = parsed.map_err(ProfileError::Validation)?;
                        if ph.command() != name {
                            return bad(format!(
                                "commands.{name}: ${} may only be used in `{}`",
                                t.trim_start_matches('$'),
                                ph.command()
                            ));
                        }
                    }
                }
            }
        }
        for (field, spec) in &d.state {
            if field.trim().is_empty() {
                return bad("state field names must not be empty".to_owned());
            }
            check_text(&format!("state.{field}.query"), &spec.query)?;
            if !placeholders_in(&spec.query).is_empty() {
                return bad(format!("state.{field}.query may not contain placeholders"));
            }
            if let Some(prefix) = &spec.strip_prefix {
                check_text(&format!("state.{field}.strip_prefix"), prefix)?;
            }
            if let Some(pointer) = &spec.extract {
                if !pointer.starts_with('/') || pointer.len() > 256 {
                    return bad(format!(
                        "state.{field}.extract must be a JSON Pointer starting with '/'"
                    ));
                }
            }
            match d.protocol.kind {
                ProtocolKind::Http => {
                    let (method, _) =
                        http_request_line(&format!("state.{field}.query"), &spec.query)?;
                    if method != "GET" {
                        return bad(format!("state.{field}.query must be a GET request"));
                    }
                }
                ProtocolKind::Snmp => {
                    parse_oid(&spec.query).map_err(|e| {
                        ProfileError::Validation(format!("state.{field}.query: {e}"))
                    })?;
                }
                ProtocolKind::Midi => {
                    MidiQuery::parse(&spec.query).map_err(|e| {
                        ProfileError::Validation(format!("state.{field}.query: {e}"))
                    })?;
                }
                _ => {}
            }
        }
        Ok(())
    }

    /// Whether this profile applies to a device with `identity`. Both
    /// manufacturer and model must be reported and match (trimmed,
    /// case-insensitive); a device that reports neither never matches.
    pub fn matches(&self, identity: &DeviceIdentity) -> bool {
        let same = |a: &str, b: &str| a.trim().eq_ignore_ascii_case(b.trim());
        let (Some(maker), Some(model)) = (&identity.manufacturer, &identity.model) else {
            return false;
        };
        same(&self.device.matcher.manufacturer, maker)
            && self
                .device
                .matcher
                .model
                .as_slice()
                .iter()
                .any(|m| same(m, model))
    }
}

fn validate_protocol(d: &DeviceSpec) -> Result<(), ProfileError> {
    let bad = |m: String| Err(ProfileError::Validation(m));
    let p = &d.protocol;
    match p.kind {
        ProtocolKind::Serial => {
            let Some(serial) = &p.serial else {
                return bad("`protocol.serial` is required for `serial`".to_owned());
            };
            if p.port.is_some() {
                return bad("`protocol.port` does not apply to `serial`".to_owned());
            }
            if !(300..=4_000_000).contains(&serial.baud) {
                return bad("`protocol.serial.baud` must be between 300 and 4000000".to_owned());
            }
            if !matches!(serial.data_bits.unwrap_or(8), 5..=8) {
                return bad("`protocol.serial.data_bits` must be 5, 6, 7 or 8".to_owned());
            }
            if !matches!(serial.stop_bits.unwrap_or(1), 1 | 2) {
                return bad("`protocol.serial.stop_bits` must be 1 or 2".to_owned());
            }
        }
        _ => {
            if p.serial.is_some() {
                return bad("`protocol.serial` is only valid for `serial`".to_owned());
            }
        }
    }
    if p.kind == ProtocolKind::Midi && p.port.is_some() {
        return bad("`protocol.port` does not apply to `midi`".to_owned());
    }
    if matches!(
        p.kind,
        ProtocolKind::Osc | ProtocolKind::Tcp | ProtocolKind::Udp
    ) && p.port.is_none()
    {
        return bad(format!("`protocol.port` is required for `{:?}`", p.kind));
    }
    if let Some(path) = &p.path {
        if p.kind != ProtocolKind::Websocket {
            return bad("`protocol.path` is only valid for `websocket`".to_owned());
        }
        if !path.starts_with('/')
            || path.len() > 256
            || path.chars().any(|c| c.is_control() || c == ' ')
        {
            return bad("`protocol.path` must start with '/' and contain no spaces".to_owned());
        }
    }
    Ok(())
}

/// Split and validate an HTTP `METHOD /path` line.
pub fn http_request_line<'a>(
    what: &str,
    line: &'a str,
) -> Result<(&'a str, &'a str), ProfileError> {
    let bad = |m: String| Err(ProfileError::Validation(m));
    let Some((method, path)) = line.split_once(' ') else {
        return bad(format!("`{what}` must look like `GET /path`"));
    };
    if !matches!(method, "GET" | "POST" | "PUT" | "PATCH" | "DELETE") {
        return bad(format!(
            "`{what}`: method must be GET, POST, PUT, PATCH or DELETE"
        ));
    }
    if !path.starts_with('/')
        || path.len() > 256
        || path.chars().any(|c| c.is_control() || c == ' ')
    {
        return bad(format!(
            "`{what}`: path must start with '/' and contain no spaces"
        ));
    }
    Ok((method, path))
}

fn check_text(what: &str, text: &str) -> Result<(), ProfileError> {
    if text.trim().is_empty() || text.len() > 256 || text.chars().any(char::is_control) {
        return Err(ProfileError::Validation(format!(
            "`{what}` must be 1-256 printable characters"
        )));
    }
    Ok(())
}

/// Parse a dotted-decimal OID (`1.3.6.1.2.1.1.1.0`). At least two arcs, the
/// first 0-2, at most 64 arcs.
pub fn parse_oid(text: &str) -> Result<Vec<u32>, String> {
    let arcs: Vec<u32> = text
        .trim_start_matches('.')
        .split('.')
        .map(|a| {
            a.parse::<u32>()
                .map_err(|_| format!("{text:?} is not a dotted OID"))
        })
        .collect::<Result<_, _>>()?;
    if arcs.len() < 2 || arcs.len() > 64 || arcs[0] > 2 || (arcs[0] < 2 && arcs[1] > 39) {
        return Err(format!("{text:?} is not a valid OID"));
    }
    Ok(arcs)
}

/// A MIDI 1.0 channel message a profile can send, written
/// `note_on <ch> <note> <velocity>`, `note_off …`, `cc <ch> <controller> <value>`
/// or `program <ch> <program>`. Channels are 1-16; data bytes 0-127.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MidiMessage {
    NoteOn {
        channel: u8,
        note: u8,
        velocity: u8,
    },
    NoteOff {
        channel: u8,
        note: u8,
        velocity: u8,
    },
    ControlChange {
        channel: u8,
        controller: u8,
        value: u8,
    },
    ProgramChange {
        channel: u8,
        program: u8,
    },
}

fn midi_args(parts: &[&str], n: usize) -> Result<Vec<u8>, String> {
    if parts.len() != n {
        return Err(format!("expected {n} numbers, found {}", parts.len()));
    }
    parts
        .iter()
        .map(|p| {
            p.parse::<u8>()
                .map_err(|_| format!("{p:?} is not a number 0-255"))
        })
        .collect()
}

fn midi_channel(ch: u8) -> Result<u8, String> {
    if (1..=16).contains(&ch) {
        Ok(ch - 1)
    } else {
        Err(format!("channel {ch} is not 1-16"))
    }
}

fn midi_data(v: u8) -> Result<u8, String> {
    if v <= 127 {
        Ok(v)
    } else {
        Err(format!("{v} is not a 7-bit value (0-127)"))
    }
}

impl MidiMessage {
    /// Parse the textual form. The channel in the result is 0-based.
    pub fn parse(text: &str) -> Result<Self, String> {
        let parts: Vec<&str> = text.split_whitespace().collect();
        let (kind, rest) = parts
            .split_first()
            .ok_or_else(|| "empty MIDI message".to_owned())?;
        Ok(match *kind {
            "note_on" | "note_off" => {
                let a = midi_args(rest, 3)?;
                let (channel, note, velocity) =
                    (midi_channel(a[0])?, midi_data(a[1])?, midi_data(a[2])?);
                if *kind == "note_on" {
                    MidiMessage::NoteOn {
                        channel,
                        note,
                        velocity,
                    }
                } else {
                    MidiMessage::NoteOff {
                        channel,
                        note,
                        velocity,
                    }
                }
            }
            "cc" => {
                let a = midi_args(rest, 3)?;
                MidiMessage::ControlChange {
                    channel: midi_channel(a[0])?,
                    controller: midi_data(a[1])?,
                    value: midi_data(a[2])?,
                }
            }
            "program" => {
                let a = midi_args(rest, 2)?;
                MidiMessage::ProgramChange {
                    channel: midi_channel(a[0])?,
                    program: midi_data(a[1])?,
                }
            }
            other => return Err(format!("unknown MIDI message `{other}`")),
        })
    }
}

/// What a MIDI state query watches for: the latest `cc <ch> <controller>` or
/// `program <ch>` the device sent.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum MidiQuery {
    ControlChange { channel: u8, controller: u8 },
    ProgramChange { channel: u8 },
}

impl MidiQuery {
    /// Parse the textual form. The channel in the result is 0-based.
    pub fn parse(text: &str) -> Result<Self, String> {
        let parts: Vec<&str> = text.split_whitespace().collect();
        let (kind, rest) = parts
            .split_first()
            .ok_or_else(|| "empty MIDI query".to_owned())?;
        Ok(match *kind {
            "cc" => {
                let a = midi_args(rest, 2)?;
                MidiQuery::ControlChange {
                    channel: midi_channel(a[0])?,
                    controller: midi_data(a[1])?,
                }
            }
            "program" => {
                let a = midi_args(rest, 1)?;
                MidiQuery::ProgramChange {
                    channel: midi_channel(a[0])?,
                }
            }
            other => return Err(format!("unknown MIDI query `{other}`")),
        })
    }
}

/// Reject documents that smuggle executable content into the YAML. Mirrors the
/// guards on the project manifest and the test DSL: only standard YAML tags
/// are data; custom tags, excessive nesting and unparseable input are unsafe.
pub fn contains_executable_content(input: &str) -> bool {
    match serde_yaml_ng::from_str::<serde_yaml_ng::Value>(input) {
        Ok(v) => contains_executable(&v, 0),
        Err(_) => true,
    }
}

fn contains_executable(value: &serde_yaml_ng::Value, depth: u32) -> bool {
    use serde_yaml_ng::Value;
    if depth > 32 {
        return true;
    }
    match value {
        Value::Tagged(t) => {
            let tag = t.tag.to_string();
            !(tag.is_empty() || tag == "!" || tag.starts_with("!!"))
                || contains_executable(&t.value, depth + 1)
        }
        Value::Sequence(s) => s.iter().any(|v| contains_executable(v, depth + 1)),
        Value::Mapping(m) => m.values().any(|v| contains_executable(v, depth + 1)),
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const GOOD: &str = r#"
schema_version: 1
device:
  id: example-projector
  match:
    manufacturer: Example
    model: [Beam-1, Beam-2]
  protocol:
    type: osc
    port: 9000
    timeout_ms: 500
  commands:
    power_on:  { send: /power, args: [true] }
    set_input: { send: /input, args: ["$input"] }
    set_audio_volume: { send: /vol, args: ["$level_db", 1] }
  state:
    power: { query: /power }
"#;

    fn err(doc: &str) -> ProfileError {
        DeviceProfile::from_yaml_str(doc).unwrap_err()
    }

    #[test]
    fn parses_a_valid_profile() {
        let p = DeviceProfile::from_yaml_str(GOOD).unwrap();
        assert_eq!(p.device.protocol.kind, ProtocolKind::Osc);
        assert_eq!(p.device.protocol.port, Some(9000));
        assert_eq!(
            p.device.commands["power_on"].args,
            vec![ProfileArg::Bool(true)]
        );
        assert_eq!(
            p.device.commands["set_input"].args[0].placeholder(),
            Some(Placeholder::Input)
        );
        assert_eq!(p.device.state["power"].query, "/power");
    }

    #[test]
    fn round_trips_through_yaml() {
        let p = DeviceProfile::from_yaml_str(GOOD).unwrap();
        let again = DeviceProfile::from_yaml_str(&serde_yaml_ng::to_string(&p).unwrap()).unwrap();
        assert_eq!(p, again);
    }

    #[test]
    fn schema_version_is_mandatory_and_checked() {
        let no_version = GOOD.replace("schema_version: 1\n", "");
        assert!(matches!(err(&no_version), ProfileError::Validation(_)));
        let future = GOOD.replace("schema_version: 1", "schema_version: 2");
        assert_eq!(err(&future), ProfileError::UnsupportedVersion { found: 2 });
    }

    #[test]
    fn future_version_reports_version_not_unknown_fields() {
        let doc = "schema_version: 9\ndevice:\n  brand_new_field: 1\n";
        assert_eq!(err(doc), ProfileError::UnsupportedVersion { found: 9 });
    }

    #[test]
    fn rejects_executable_content_and_junk() {
        let tagged = GOOD.replace("send: /power,", "send: !exec /power,");
        assert_eq!(err(&tagged), ProfileError::ExecutableContent);
        assert_eq!(err("{{{ not yaml"), ProfileError::ExecutableContent);
        assert!(matches!(
            err(&"x".repeat(MAX_PROFILE_BYTES + 1)),
            ProfileError::TooLarge
        ));
    }

    #[test]
    fn rejects_unknown_fields() {
        let doc = GOOD.replace("port: 9000", "port: 9000\n    password: hunter2");
        assert!(matches!(err(&doc), ProfileError::Parse(_)));
    }

    #[test]
    fn validates_commands_and_placeholders() {
        let unknown = GOOD.replace("power_on:", "launch_missiles:");
        assert!(
            matches!(err(&unknown), ProfileError::Validation(m) if m.contains("unknown command"))
        );
        let wrong_cmd = GOOD.replace(r#"args: ["$input"]"#, r#"args: ["$level_db"]"#);
        assert!(
            matches!(err(&wrong_cmd), ProfileError::Validation(m) if m.contains("may only be used"))
        );
        let bad_ph = GOOD.replace(r#"args: ["$input"]"#, r#"args: ["$home"]"#);
        assert!(
            matches!(err(&bad_ph), ProfileError::Validation(m) if m.contains("unknown placeholder"))
        );
    }

    #[test]
    fn validates_ids_ports_and_timeouts() {
        for (from, to) in [
            ("id: example-projector", "id: Bad Id"),
            ("port: 9000", "port: 0"),
            ("timeout_ms: 500", "timeout_ms: 0"),
            ("timeout_ms: 500", "timeout_ms: 99999"),
            ("manufacturer: Example", "manufacturer: ''"),
        ] {
            assert!(
                matches!(err(&GOOD.replace(from, to)), ProfileError::Validation(_)),
                "{to}"
            );
        }
    }

    #[test]
    fn needs_something_to_do() {
        let doc = "schema_version: 1\ndevice:\n  id: x\n  match: {manufacturer: A, model: B}\n  protocol: {type: osc}\n";
        assert!(matches!(err(doc), ProfileError::Validation(_)));
    }

    #[test]
    fn matching_requires_manufacturer_and_model() {
        let p = DeviceProfile::from_yaml_str(GOOD).unwrap();
        let id = |m: Option<&str>, n: Option<&str>| DeviceIdentity {
            manufacturer: m.map(Into::into),
            model: n.map(Into::into),
            ..DeviceIdentity::default()
        };
        assert!(p.matches(&id(Some("example"), Some(" BEAM-2 "))));
        assert!(!p.matches(&id(Some("Example"), Some("Beam-3"))));
        assert!(!p.matches(&id(Some("Other"), Some("Beam-1"))));
        assert!(!p.matches(&id(Some("Example"), None)));
        assert!(!p.matches(&DeviceIdentity::default()));
    }

    #[test]
    fn text_protocol_fields_validate() {
        let doc = r#"
schema_version: 1
device:
  id: tcp-proj
  match: { manufacturer: A, model: B }
  protocol: { type: tcp, port: 4352, terminator: cr }
  commands:
    power_on: { send: "POWR 1", ack: "OK" }
    set_input: { send: "INPT $input" }
  state:
    power: { query: "POWR?", parser: power_state, strip_prefix: "POWR=" }
"#;
        let p = DeviceProfile::from_yaml_str(doc).unwrap();
        assert_eq!(p.device.protocol.terminator_or_default(), Terminator::Cr);
        assert_eq!(p.device.commands["power_on"].ack.as_deref(), Some("OK"));
        assert_eq!(p.device.state["power"].parser, Some(ParserKind::PowerState));
        let wrong = doc.replace("INPT $input", "INPT $level_db");
        assert!(matches!(err(&wrong), ProfileError::Validation(_)));
        let unknown = doc.replace("INPT $input", "INPT $nope");
        assert!(matches!(err(&unknown), ProfileError::Validation(_)));
        let in_query = doc.replace("POWR?", "POWR? $input");
        assert!(matches!(err(&in_query), ProfileError::Validation(_)));
        let bad_parser = doc.replace("power_state", "python");
        assert!(matches!(err(&bad_parser), ProfileError::Parse(_)));
    }

    #[test]
    fn parsers_are_typed_and_conservative() {
        use StateValue::*;
        assert_eq!(ParserKind::Bool.parse(" ON ").unwrap(), Boolean(true));
        assert!(ParserKind::Bool.parse("maybe").is_err());
        assert_eq!(ParserKind::Int.parse("42").unwrap(), Integer(42));
        assert!(ParserKind::Int.parse("4.2").is_err());
        assert_eq!(ParserKind::Float.parse("-3.5").unwrap(), Float(-3.5));
        assert!(ParserKind::Float.parse("NaN").is_err());
        assert_eq!(
            ParserKind::PowerState.parse("Standby").unwrap(),
            Boolean(false)
        );
        assert_eq!(
            ParserKind::PowerState.parse("warming").unwrap(),
            Text("warming".into())
        );
        assert_eq!(ParserKind::Auto.parse("on").unwrap(), Boolean(true));
        assert_eq!(ParserKind::Auto.parse("1").unwrap(), Integer(1));
        assert_eq!(ParserKind::Auto.parse("2.5").unwrap(), Float(2.5));
        assert_eq!(ParserKind::Auto.parse("NaN").unwrap(), Text("NaN".into()));
        assert_eq!(
            ParserKind::Auto.parse("HDMI1").unwrap(),
            Text("HDMI1".into())
        );
    }

    #[test]
    fn finds_embedded_placeholders() {
        let found = placeholders_in("ROUTE $source TO $destination, cost $5 $Z $nope");
        assert_eq!(found.len(), 3);
        assert_eq!(found[0], Ok(Placeholder::Source));
        assert_eq!(found[1], Ok(Placeholder::Destination));
        assert!(found[2].is_err());
    }

    fn doc(protocol: &str, extra: &str) -> String {
        format!(
            "schema_version: 1\ndevice:\n  id: x\n  match: {{ manufacturer: A, model: B }}\n  protocol: {protocol}\n{extra}"
        )
    }

    #[test]
    fn serial_profiles_need_line_settings_and_no_port() {
        let state = "  state:\n    p: { query: \"P?\" }\n";
        assert!(DeviceProfile::from_yaml_str(&doc(
            "{ type: serial, serial: { baud: 9600 } }",
            state
        ))
        .is_ok());
        for protocol in [
            "{ type: serial }",
            "{ type: serial, port: 5, serial: { baud: 9600 } }",
            "{ type: serial, serial: { baud: 5 } }",
            "{ type: serial, serial: { baud: 9600, data_bits: 9 } }",
            "{ type: serial, serial: { baud: 9600, stop_bits: 3 } }",
            "{ type: tcp, port: 1, serial: { baud: 9600 } }",
        ] {
            assert!(
                matches!(err(&doc(protocol, state)), ProfileError::Validation(_)),
                "{protocol}"
            );
        }
    }

    #[test]
    fn network_protocols_require_a_port_where_there_is_no_default() {
        let state = "  state:\n    p: { query: \"P?\" }\n";
        for kind in ["osc", "tcp", "udp"] {
            let d = doc(&format!("{{ type: {kind} }}"), state);
            assert!(matches!(err(&d), ProfileError::Validation(_)), "{kind}");
        }
        // http / websocket / snmp default their port.
        assert!(DeviceProfile::from_yaml_str(&doc("{ type: websocket }", state)).is_ok());
        let ws_path = doc("{ type: websocket, path: /ws }", state);
        assert!(DeviceProfile::from_yaml_str(&ws_path).is_ok());
        let bad = doc("{ type: tcp, port: 1, path: /ws }", state);
        assert!(matches!(err(&bad), ProfileError::Validation(_)));
        let bad = doc("{ type: websocket, path: ws }", state);
        assert!(matches!(err(&bad), ProfileError::Validation(_)));
    }

    #[test]
    fn http_request_lines_are_validated() {
        assert_eq!(
            http_request_line("x", "POST /api/power").unwrap(),
            ("POST", "/api/power")
        );
        for bad in [
            "GET",
            "TRACE /x",
            "GET x",
            "GET /a b",
            "GET /a\nb",
            "get /x",
        ] {
            assert!(http_request_line("x", bad).is_err(), "{bad:?}");
        }
    }

    #[test]
    fn http_profiles_enforce_get_queries_and_body_rules() {
        let ok = doc(
            "{ type: http }",
            "  commands:\n    set_input: { send: \"PUT /in/$input\", body: '{\"n\":\"$input\"}' }\n  state:\n    p: { query: \"GET /s\", extract: /p }\n",
        );
        assert!(DeviceProfile::from_yaml_str(&ok).is_ok());
        let post_state = ok.replace("GET /s", "POST /s");
        assert!(matches!(err(&post_state), ProfileError::Validation(_)));
        let bad_extract = ok.replace("extract: /p", "extract: p");
        assert!(matches!(err(&bad_extract), ProfileError::Validation(_)));
    }

    #[test]
    fn snmp_profiles_are_read_only_with_valid_oids() {
        let ok = doc(
            "{ type: snmp }",
            "  state:\n    fw: { query: \"1.3.6.1.2.1.1.1.0\" }\n",
        );
        assert!(DeviceProfile::from_yaml_str(&ok).is_ok());
        let bad_oid = ok.replace("1.3.6.1.2.1.1.1.0", "sysDescr.0");
        assert!(matches!(err(&bad_oid), ProfileError::Validation(_)));
        let with_cmd =
            format!("{ok}  commands:\n    power_on: {{ send: \"1.3.6.1.2.1.1.5.0\" }}\n");
        assert!(matches!(err(&with_cmd), ProfileError::Validation(_)));
    }

    #[test]
    fn oids_parse_strictly() {
        assert_eq!(parse_oid("1.3.6.1").unwrap(), vec![1, 3, 6, 1]);
        assert_eq!(parse_oid(".1.3.6.1").unwrap(), vec![1, 3, 6, 1]);
        for bad in ["", "1", "1.", "a.b", "3.1", "1.40", "1.3.-1"] {
            assert!(parse_oid(bad).is_err(), "{bad:?}");
        }
        assert!(parse_oid(&vec!["1"; 65].join(".")).is_err());
    }

    #[test]
    fn midi_grammar_is_strict() {
        assert_eq!(
            MidiMessage::parse("cc 1 7 127").unwrap(),
            MidiMessage::ControlChange {
                channel: 0,
                controller: 7,
                value: 127
            }
        );
        assert_eq!(
            MidiMessage::parse("note_on 16 60 100").unwrap(),
            MidiMessage::NoteOn {
                channel: 15,
                note: 60,
                velocity: 100
            }
        );
        assert_eq!(
            MidiMessage::parse("program 2 5").unwrap(),
            MidiMessage::ProgramChange {
                channel: 1,
                program: 5
            }
        );
        for bad in [
            "",
            "cc 0 7 1",
            "cc 17 7 1",
            "cc 1 128 1",
            "cc 1 7 128",
            "cc 1 7",
            "cc 1 7 1 1",
            "sysex f0 f7",
            "cc a b c",
            "cc 1 7 $level_db",
        ] {
            assert!(MidiMessage::parse(bad).is_err(), "{bad:?}");
        }
        assert_eq!(
            MidiQuery::parse("cc 3 11").unwrap(),
            MidiQuery::ControlChange {
                channel: 2,
                controller: 11
            }
        );
        assert_eq!(
            MidiQuery::parse("program 1").unwrap(),
            MidiQuery::ProgramChange { channel: 0 }
        );
        assert!(MidiQuery::parse("note 1 2").is_err());
        assert!(MidiQuery::parse("cc 1").is_err());
    }

    #[test]
    fn terminator_bytes() {
        assert_eq!(Terminator::Crlf.bytes(), b"\r\n");
        assert_eq!(Terminator::Cr.bytes(), b"\r");
        assert_eq!(Terminator::None.bytes(), b"");
        assert_eq!(Terminator::Cr.end_byte(), b'\r');
        assert_eq!(Terminator::Lf.end_byte(), b'\n');
    }

    #[test]
    fn substitute_replaces_in_order_and_propagates_errors() {
        let out = substitute("A $source B $destination", |p| {
            Ok(match p {
                Placeholder::Source => "in1".to_owned(),
                _ => "out2".to_owned(),
            })
        })
        .unwrap();
        assert_eq!(out, "A in1 B out2");
        assert!(substitute("$nope", |_| Ok(String::new())).is_err());
        assert!(substitute("$input", |_| Err("no".into())).is_err());
        assert_eq!(
            substitute("no placeholders $5", |_| Ok(String::new())).unwrap(),
            "no placeholders $5"
        );
    }
}
