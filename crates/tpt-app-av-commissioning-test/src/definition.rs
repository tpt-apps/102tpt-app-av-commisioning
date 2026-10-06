//! Test definitions (§12) and the `CommissioningTest` trait.

use serde::{Deserialize, Serialize};
use std::time::Duration;

use tpt_app_av_commissioning_driver::DriverError;
use tpt_app_av_commissioning_model::{DeviceId, TestSuiteId};

use crate::result::TestResult;
use crate::test_kind::TestKind;

/// Identifies a test definition.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(transparent)]
pub struct TestId(pub String);

impl TestId {
    pub fn new<S: Into<String>>(value: S) -> Self {
        Self(value.into())
    }
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl std::fmt::Display for TestId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

/// How a test executes (§15, §4.5).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ExecutionMode {
    /// Software runs the test end to end.
    Automated,
    /// Software prepares and measures; an engineer confirms.
    SemiAutomated,
    /// The engineer is the instrument (checklist with evidence).
    Manual,
}

impl ExecutionMode {
    /// Stable lowercase name.
    pub fn as_str(&self) -> &'static str {
        match self {
            ExecutionMode::Automated => "automated",
            ExecutionMode::SemiAutomated => "semi_automated",
            ExecutionMode::Manual => "manual",
        }
    }
}

/// Declares what a test needs to run.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TestRequirements {
    /// Devices this test touches (read or write).
    pub devices: Vec<DeviceId>,
    /// Devices the test mutates — these require a `DeviceLock` (§18).
    pub mutate_devices: Vec<DeviceId>,
    /// How the test executes.
    pub mode: ExecutionMode,
    /// Tests that must complete before this one may run.
    pub depends_on: Vec<TestId>,
    /// Maximum run time before forced cancellation.
    pub max_duration: Option<Duration>,
    /// How many times to retry a transient failure.
    pub retries: u32,
    /// Driver capabilities the test relies on.
    pub requires_capabilities: Vec<DeviceCapabilityRef>,
}

/// A capability requirement, keyed to a device.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DeviceCapabilityRef {
    pub device: DeviceId,
    pub capability: String,
}

impl Default for TestRequirements {
    fn default() -> Self {
        Self {
            devices: Vec::new(),
            mutate_devices: Vec::new(),
            mode: ExecutionMode::Automated,
            depends_on: Vec::new(),
            max_duration: Some(Duration::from_secs(60)),
            retries: 0,
            requires_capabilities: Vec::new(),
        }
    }
}

impl TestRequirements {
    /// A read-only automated test touching `devices`.
    pub fn reads(devices: impl IntoIterator<Item = DeviceId>) -> Self {
        let devices = devices.into_iter().collect::<Vec<_>>();
        Self {
            mutate_devices: Vec::new(),
            devices,
            ..Self::default()
        }
    }

    /// A manual test with no device interaction (e.g. visual inspection).
    pub fn manual() -> Self {
        Self {
            mode: ExecutionMode::Manual,
            ..Self::default()
        }
    }

    /// Declared mutating devices (need locks).
    pub fn mutating<'a>(&'a self, device: &'a DeviceId) -> bool {
        self.mutate_devices.contains(device)
    }
}

/// Errors a test can produce while running.
#[derive(Debug, thiserror::Error)]
pub enum TestError {
    #[error("driver error: {0}")]
    Driver(#[from] DriverError),
    #[error("test timed out")]
    Timeout,
    #[error("test was cancelled")]
    Cancelled,
    #[error("test fixture error: {0}")]
    Fixture(String),
    #[error("test bug: {0}")]
    Internal(String),
}

/// A single commissioning test.
///
/// Tests are dispatched onto worker threads by the runner, so every test is
/// `Send + Sync`: share device access through `Arc<Mutex<…>>` handles, never
/// through `RefCell`/`Cell`.
pub trait CommissioningTest: Send + Sync {
    /// Stable, human-readable id (e.g. `display-identity`).
    fn id(&self) -> &TestId;

    /// Human-readable name.
    fn name(&self) -> &str;

    /// The test suite this test belongs to, if any.
    fn suite(&self) -> Option<TestSuiteId> {
        None
    }

    /// Which of the nine commissioning categories (§13) this test is in, if
    /// categorised. Informational only: it never affects scheduling.
    fn kind(&self) -> Option<TestKind> {
        None
    }

    /// What this test needs to run.
    fn requirements(&self) -> &TestRequirements;

    /// The checklist the engineer works through when this test runs manually
    /// (§15). `None` for automated and most semi-automated tests.
    fn checklist(&self) -> Option<crate::manual::ManualChecklist> {
        None
    }

    /// Execute the test, producing a result.
    ///
    /// Implementations must not catch infrastructure failures silently; an
    /// `Err` from a driver is folded into the result by the runner.
    fn execute(&self) -> Result<TestResult, TestError>;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn manual_requirements_are_non_mutating() {
        let req = TestRequirements::manual();
        assert_eq!(req.mode, ExecutionMode::Manual);
        assert!(req.mutate_devices.is_empty());
    }

    #[test]
    fn mutating_detection() {
        let d = DeviceId::new("p1");
        let mut req = TestRequirements::reads([d.clone()]);
        req.mutate_devices.push(d.clone());
        assert!(req.mutating(&d));
        let other = DeviceId::new("p2");
        assert!(!req.mutating(&other));
    }
}
