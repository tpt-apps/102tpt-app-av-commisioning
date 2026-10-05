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
use tpt_app_av_commissioning_device::DeviceIdentity;

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
}

impl ProtocolSpec {
    /// The timeout in milliseconds, defaulted.
    pub fn timeout_ms_or_default(&self) -> u64 {
        self.timeout_ms.unwrap_or(1000)
    }
}

/// A command's message.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CommandSpec {
    /// What to send: an OSC address, or protocol text.
    pub send: String,
    #[serde(default)]
    pub args: Vec<ProfileArg>,
}

/// A state field's query.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct StateSpec {
    /// What to ask: an OSC address, or protocol text.
    pub query: String,
    /// Name of a built-in parser for text protocols (unused by OSC, which
    /// returns typed values).
    #[serde(default)]
    pub parser: Option<String>,
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

fn check_text(what: &str, text: &str) -> Result<(), ProfileError> {
    if text.trim().is_empty() || text.len() > 256 || text.chars().any(char::is_control) {
        return Err(ProfileError::Validation(format!(
            "`{what}` must be 1-256 printable characters"
        )));
    }
    Ok(())
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
}
