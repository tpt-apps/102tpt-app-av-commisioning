//! Declarative test procedure DSL (§16).
//!
//! Procedures are YAML data describing a sequence of `command` / `wait` /
//! `measure` / `assert` steps. The DSL is data-only: arbitrary executable
//! content (custom YAML tags, directives) is rejected before parsing (§36,
//! `docs/security.md`).
//!
//! A procedure derives the [`TestRequirements`] the runner needs (devices it
//! reads, devices it mutates, mode, dependencies, timeout, retries), so a
//! charted DSL test slots into the existing scheduling / locking pipeline.

use std::fmt;
use std::time::Duration;

use serde::de::{self, Deserializer, MapAccess, Visitor};
use serde::{Deserialize, Serialize};
use serde_json::Value as JsonValue;

use tpt_app_av_commissioning_model::DeviceId;

use crate::definition::{ExecutionMode, TestId, TestRequirements};
use crate::test_kind::{classify_field, TestKind};

/// Errors raised while parsing or validating a procedure.
#[derive(Debug, thiserror::Error)]
pub enum ProcedureError {
    #[error("test procedure contains executable content and was rejected")]
    ExecutableContent,
    #[error("test procedure failed to parse: {0}")]
    Parse(String),
    #[error("test procedure failed validation: {0}")]
    Validation(String),
}

/// A single step in a [`TestProcedure`].
///
/// The YAML spelling of each step is its variant name:
///
/// ```yaml
/// steps:
///   - command:
///       device: matrix-01
///       action: select_output
///       args:
///         output: 4
///   - wait:
///       milliseconds: 1500
///   - measure:
///       type: video_signal
///       device: projector-01
///   - assert:
///       field: resolution
///       equals: "3840x2160"
/// ```
///
/// Serialization is the spec's single-key map form (`- command: {...}`);
/// deserialization is implemented by hand because serde_yaml_ng's default
/// enum handling expects YAML tags rather than map keys.
#[derive(Debug, Clone, PartialEq)]
pub enum ProcedureStep {
    Command(CommandStep),
    Wait(WaitStep),
    Measure(MeasureStep),
    Assert(AssertStep),
}

impl Serialize for ProcedureStep {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        use serde::ser::SerializeMap;
        let mut map = serializer.serialize_map(Some(1))?;
        match self {
            ProcedureStep::Command(step) => map.serialize_entry("command", step)?,
            ProcedureStep::Wait(step) => map.serialize_entry("wait", step)?,
            ProcedureStep::Measure(step) => map.serialize_entry("measure", step)?,
            ProcedureStep::Assert(step) => map.serialize_entry("assert", step)?,
        }
        map.end()
    }
}

impl<'de> Deserialize<'de> for ProcedureStep {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        deserializer.deserialize_map(StepVisitor)
    }
}

struct StepVisitor;

impl<'de> Visitor<'de> for StepVisitor {
    type Value = ProcedureStep;

    fn expecting(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("a single-key map whose key is one of: command, wait, measure, assert")
    }

    fn visit_map<A>(self, mut map: A) -> Result<Self::Value, A::Error>
    where
        A: MapAccess<'de>,
    {
        let kind: Option<String> = map.next_key()?;
        let kind = kind.ok_or_else(|| de::Error::custom("step must not be empty"))?;
        let step = match kind.as_str() {
            "command" => ProcedureStep::Command(map.next_value()?),
            "wait" => ProcedureStep::Wait(map.next_value()?),
            "measure" => ProcedureStep::Measure(map.next_value()?),
            "assert" => ProcedureStep::Assert(map.next_value()?),
            other => {
                return Err(de::Error::custom(format!(
                    "unknown step type `{other}` (expected command, wait, measure, or assert)"
                )));
            }
        };
        if map.next_key::<de::IgnoredAny>()?.is_some() {
            return Err(de::Error::custom(
                "step must have exactly one key (command, wait, measure, or assert)",
            ));
        }
        Ok(step)
    }
}

/// A driver command sent to a device. `action` is driver-specific (for
/// example `select_input`, `power_on`); `args` carries its parameters.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CommandStep {
    pub device: DeviceId,
    pub action: String,
    #[serde(default)]
    pub args: JsonValue,
}

impl CommandStep {
    /// The device this step mutates.
    pub fn device(&self) -> &DeviceId {
        &self.device
    }
}

/// Wait for the system to stabilise (§13 stabilisation delays).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct WaitStep {
    pub milliseconds: u64,
}

impl WaitStep {
    /// The wait as a [`Duration`].
    pub fn duration(&self) -> Duration {
        Duration::from_millis(self.milliseconds)
    }
}

/// A measurement to take from a device.
///
/// `kind` is driver/measurement-specific (for example `video_signal`,
/// `audio_level`, `latency`) and may carry options in `args`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct MeasureStep {
    #[serde(rename = "type")]
    pub kind: String,
    pub device: DeviceId,
    #[serde(default)]
    pub args: JsonValue,
}

/// An assertion against a measured field.
///
/// `field` names the measured value (for example `resolution`,
/// `frame_rate`) and `equals` the expected value.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AssertStep {
    pub field: String,
    #[serde(default)]
    pub equals: Option<JsonValue>,
}

/// A declarative commissioning test (§16).
///
/// The top-level YAML document is wrapped in a `test:` key:
///
/// ```yaml
/// test:
///   id: projector.signal-path
///   name: "Projector Signal Path"
///   mode: automated
///   steps: [ ... ]
/// ```
///
/// Optional scheduling fields map directly onto [`TestRequirements`]:
/// `depends_on`, `max_duration_ms`, `retries`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TestProcedure {
    pub id: TestId,
    pub name: String,
    /// Pins the test's category (§13); when absent it is inferred from the
    /// fields the steps measure and assert (see [`TestProcedure::kind`]).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub kind: Option<TestKind>,
    #[serde(default = "default_mode")]
    pub mode: ExecutionMode,
    #[serde(default)]
    pub depends_on: Vec<TestId>,
    #[serde(default)]
    pub max_duration_ms: Option<u64>,
    #[serde(default)]
    pub retries: u32,
    #[serde(default)]
    pub steps: Vec<ProcedureStep>,
}

fn default_mode() -> ExecutionMode {
    ExecutionMode::Automated
}

/// The on-disk envelope: `test: <procedure>`.
#[derive(Debug, Serialize, Deserialize)]
struct ProcedureFile {
    test: TestProcedure,
}

impl TestProcedure {
    /// Parse, check for executable content, and validate a DSL document.
    pub fn from_yaml_str(input: &str) -> Result<Self, ProcedureError> {
        if contains_executable_content(input) {
            return Err(ProcedureError::ExecutableContent);
        }
        let file: ProcedureFile = serde_yaml_ng::from_str(input).map_err(|e| {
            ProcedureError::Parse(format!("invalid YAML or missing `test:` key: {e}"))
        })?;
        file.test.validate()?;
        Ok(file.test)
    }

    /// Validate the procedure against the schema rules.
    pub fn validate(&self) -> Result<(), ProcedureError> {
        let err = |msg: String| ProcedureError::Validation(msg);
        if self.id.as_str().is_empty() {
            return Err(err("`id` must not be empty".to_owned()));
        }
        if self.name.trim().is_empty() {
            return Err(err("`name` must not be empty".to_owned()));
        }
        if self.steps.is_empty() {
            return Err(err("`steps` must contain at least one step".to_owned()));
        }
        if self.max_duration_ms == Some(0) {
            return Err(err("`max_duration_ms` must be greater than zero".to_owned()));
        }
        for (i, step) in self.steps.iter().enumerate() {
            let at = |detail: String| err(format!("step {}: {detail}", i + 1));
            match step {
                ProcedureStep::Command(c) => {
                    if c.device.as_str().is_empty() {
                        return Err(at("command `device` must not be empty".to_owned()));
                    }
                    if c.action.trim().is_empty() {
                        return Err(at("command `action` must not be empty".to_owned()));
                    }
                }
                ProcedureStep::Wait(_) => {}
                ProcedureStep::Measure(m) => {
                    if m.kind.trim().is_empty() {
                        return Err(at("measure `type` must not be empty".to_owned()));
                    }
                    if m.device.as_str().is_empty() {
                        return Err(at("measure `device` must not be empty".to_owned()));
                    }
                }
                ProcedureStep::Assert(a) => {
                    if a.field.trim().is_empty() {
                        return Err(at("assert `field` must not be empty".to_owned()));
                    }
                    if a.equals.is_none() {
                        return Err(at("assert must specify `equals`".to_owned()));
                    }
                }
            }
        }
        Ok(())
    }

    /// The test's category (§13): the pinned `kind:` if set, otherwise the
    /// first category any measure `type` or assert `field` classifies to.
    /// `None` when nothing classifies — never guessed.
    pub fn kind(&self) -> Option<TestKind> {
        if self.kind.is_some() {
            return self.kind;
        }
        self.steps.iter().find_map(|step| match step {
            ProcedureStep::Measure(m) => classify_field(&m.kind),
            ProcedureStep::Assert(a) => classify_field(&a.field),
            ProcedureStep::Command(_) | ProcedureStep::Wait(_) => None,
        })
    }

    /// The [`TestRequirements`] implied by this procedure.
    ///
    /// Command steps mutate their device (they need a [`DeviceLock`]); measure
    /// steps only read. `mode`, `depends_on`, `max_duration_ms` and `retries`
    /// pass through.
    pub fn requirements(&self) -> TestRequirements {
        let mut devices = Vec::new();
        let mut mutate_devices = Vec::new();
        for step in &self.steps {
            match step {
                ProcedureStep::Command(c) => {
                    devices.push(c.device.clone());
                    mutate_devices.push(c.device.clone());
                }
                ProcedureStep::Measure(m) => devices.push(m.device.clone()),
                ProcedureStep::Wait(_) | ProcedureStep::Assert(_) => {}
            }
        }
        devices.sort();
        devices.dedup();
        mutate_devices.sort();
        mutate_devices.dedup();
        TestRequirements {
            devices,
            mutate_devices,
            mode: self.mode,
            depends_on: self.depends_on.clone(),
            max_duration: self.max_duration_ms.map(Duration::from_millis),
            retries: self.retries,
            requires_capabilities: Vec::new(),
        }
    }
}

/// Reject DSL documents that smuggle executable content into the YAML.
///
/// Mirrors the guard on the project manifest (`core::manifest`): only standard
/// YAML tags are data; custom tags, directives, and unparseable input are
/// treated as unsafe (§36).
pub fn contains_executable_content(input: &str) -> bool {
    let value: serde_yaml_ng::Value = match serde_yaml_ng::from_str(input) {
        Ok(v) => v,
        Err(_) => return true, // unparseable is treated as unsafe
    };
    contains_executable(&value, 0)
}

fn contains_executable(value: &serde_yaml_ng::Value, depth: u32) -> bool {
    if depth > 32 {
        return true;
    }
    use serde_yaml_ng::Value;
    match value {
        Value::Tagged(tagged) => {
            if !is_safe_tag(&tagged.tag.to_string()) {
                return true;
            }
            contains_executable(&tagged.value, depth + 1)
        }
        Value::Sequence(seq) => seq.iter().any(|v| contains_executable(v, depth + 1)),
        Value::Mapping(map) => map.values().any(|v| contains_executable(v, depth + 1)),
        _ => false,
    }
}

fn is_safe_tag(tag: &str) -> bool {
    tag.is_empty() || tag == "!" || tag.starts_with("!!")
}

#[cfg(test)]
mod tests {
    use super::*;

    const SPEC_SAMPLE: &str = r##"
test:
  id: projector.signal-path
  name: "Projector Signal Path"

  steps:
    - command:
        device: matrix-01
        action: select_output
        args:
          output: 4

    - command:
        device: projector-01
        action: select_input
        args:
          input: hdmi1

    - wait:
        milliseconds: 1500

    - measure:
        type: video_signal
        device: projector-01

    - assert:
        field: resolution
        equals: "3840x2160"

    - assert:
        field: frame_rate
        equals: 60
"##;

    #[test]
    fn parses_spec_sample() {
        let proc = TestProcedure::from_yaml_str(SPEC_SAMPLE).unwrap();
        assert_eq!(proc.id, TestId::new("projector.signal-path"));
        assert_eq!(proc.name, "Projector Signal Path");
        assert_eq!(proc.mode, ExecutionMode::Automated);
        assert_eq!(proc.steps.len(), 6);
        assert!(matches!(proc.steps[0], ProcedureStep::Command(_)));
        assert!(matches!(proc.steps[2], ProcedureStep::Wait(_)));
        assert!(matches!(proc.steps[3], ProcedureStep::Measure(_)));
        assert!(matches!(proc.steps[4], ProcedureStep::Assert(_)));
    }

    #[test]
    fn wait_duration_is_milliseconds() {
        let proc = TestProcedure::from_yaml_str(SPEC_SAMPLE).unwrap();
        for step in proc.steps {
            if let ProcedureStep::Wait(step) = step {
                assert_eq!(step.duration(), Duration::from_millis(1500));
            }
        }
    }

    #[test]
    fn requirements_derive_devices_and_capabilities() {
        let proc = TestProcedure::from_yaml_str(SPEC_SAMPLE).unwrap();
        let req = proc.requirements();
        assert_eq!(req.mode, ExecutionMode::Automated);
        // Command steps mutate; measure steps only read.
        assert_eq!(
            req.devices,
            vec![DeviceId::new("matrix-01"), DeviceId::new("projector-01")]
        );
        assert_eq!(
            req.mutate_devices,
            vec![DeviceId::new("matrix-01"), DeviceId::new("projector-01")]
        );
        assert!(req.max_duration.is_none());
    }

    #[test]
    fn scheduling_fields_flow_into_requirements() {
        let yaml = r##"
test:
  id: t1
  name: "Scheduled"
  mode: manual
  depends_on: [boot.identity]
  max_duration_ms: 30000
  retries: 2
  steps:
    - wait:
        milliseconds: 100
    - assert:
        field: result
        equals: ok
"##;
        let proc = TestProcedure::from_yaml_str(yaml).unwrap();
        let req = proc.requirements();
        assert_eq!(req.mode, ExecutionMode::Manual);
        assert_eq!(req.depends_on, vec![TestId::new("boot.identity")]);
        assert_eq!(req.max_duration, Some(Duration::from_millis(30_000)));
        assert_eq!(req.retries, 2);
    }

    #[test]
    fn round_trips_through_yaml() {
        let proc = TestProcedure::from_yaml_str(SPEC_SAMPLE).unwrap();
        let doc = serde_yaml_ng::to_string(&ProcedureFile { test: proc.clone() }).unwrap();
        let back = TestProcedure::from_yaml_str(&doc).unwrap();
        assert_eq!(back, proc);
    }

    #[test]
    fn rejects_empty_steps() {
        let yaml = r##"
test:
  id: t1
  name: "Empty"
  steps: []
"##;
        let err = TestProcedure::from_yaml_str(yaml).unwrap_err();
        assert!(matches!(err, ProcedureError::Validation(_)));
    }

    #[test]
    fn rejects_empty_action() {
        let yaml = r##"
test:
  id: t1
  name: "No action"
  steps:
    - command:
        device: d1
        action: " "
"##;
        assert!(matches!(
            TestProcedure::from_yaml_str(yaml),
            Err(ProcedureError::Validation(_))
        ));
    }

    #[test]
    fn rejects_assert_without_equals() {
        let yaml = r##"
test:
  id: t1
  name: "No equals"
  steps:
    - assert:
        field: resolution
"##;
        assert!(matches!(
            TestProcedure::from_yaml_str(yaml),
            Err(ProcedureError::Validation(_))
        ));
    }

    #[test]
    fn rejects_missing_test_key() {
        let yaml = "steps: [{ wait: { milliseconds: 1 } }]";
        assert!(matches!(
            TestProcedure::from_yaml_str(yaml),
            Err(ProcedureError::Parse(_))
        ));
    }

    #[test]
    fn rejects_executable_content() {
        let yaml = r##"
test:
  id: t1
  name: "Smuggled code"
  steps:
    - command:
        device: d1
        action: exec
        args: !python/object/apply:os.system ["reboot"]
"##;
        assert_eq!(
            TestProcedure::from_yaml_str(yaml).unwrap_err().to_string(),
            "test procedure contains executable content and was rejected"
        );
        assert!(contains_executable_content(yaml));
    }

    #[test]
    fn accepts_standard_yaml_tags() {
        let yaml = r##"
test:
  id: t1
  name: "Tagged strings"
  steps:
    - assert:
        field: frame_rate
        equals: !!int 60
"##;
        assert!(!contains_executable_content(yaml));
        assert!(TestProcedure::from_yaml_str(yaml).is_ok());
    }

    #[test]
    fn serializes_externally_tagged_steps() {
        let proc = TestProcedure::from_yaml_str(SPEC_SAMPLE).unwrap();
        let yaml = serde_yaml_ng::to_string(&proc).unwrap();
        assert!(yaml.contains("- command:"));
        assert!(yaml.contains("- measure:"));
        assert!(yaml.contains("- assert:"));
    }

    #[test]
    fn parses_checked_in_sample_suite() {
        let yaml = include_str!("../../../test-suites/generic/projector-signal-path.yaml");
        let proc = TestProcedure::from_yaml_str(yaml).unwrap();
        assert_eq!(proc.id, TestId::new("projector.signal-path"));
        proc.validate().unwrap();
    }

    #[test]
    fn kind_is_pinned_or_inferred_never_guessed() {
        let pinned = TestProcedure::from_yaml_str(
            "test:
  id: t
  name: T
  kind: connectivity
  steps:
    - assert:
        field: resolution
        equals: x
",
        )
        .unwrap();
        assert_eq!(pinned.kind(), Some(TestKind::Connectivity));

        let inferred = TestProcedure::from_yaml_str(
            "test:
  id: t
  name: T
  steps:
    - assert:
        field: resolution
        equals: x
",
        )
        .unwrap();
        assert_eq!(inferred.kind(), Some(TestKind::Video));

        let unknown = TestProcedure::from_yaml_str(
            "test:
  id: t
  name: T
  steps:
    - assert:
        field: make
        equals: x
",
        )
        .unwrap();
        assert_eq!(unknown.kind(), None);
    }
}
