//! Test statuses (§12.1).

use serde::{Deserialize, Serialize};

/// The outcome of an executed test.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TestStatus {
    /// The test passed.
    Pass,
    /// The test failed.
    Fail,
    /// Passed with warnings / non-critical anomalies.
    Warning,
    /// Could not run because a dependency failed — not a failure of this test.
    Blocked,
    /// Not run (not applicable, deselected, or skipped).
    Skipped,
    /// A manual test awaiting an engineer's verdict.
    Manual,
    /// No conclusion could be reached.
    Inconclusive,
}

impl TestStatus {
    /// Stable lowercase name (mirrors the serialized form).
    pub fn as_str(&self) -> &'static str {
        match self {
            TestStatus::Pass => "pass",
            TestStatus::Fail => "fail",
            TestStatus::Warning => "warning",
            TestStatus::Blocked => "blocked",
            TestStatus::Skipped => "skipped",
            TestStatus::Manual => "manual",
            TestStatus::Inconclusive => "inconclusive",
        }
    }

    /// Whether this status means the test itself failed.
    pub fn is_failure(&self) -> bool {
        matches!(self, TestStatus::Fail)
    }

    /// Whether this status should be treated as needing human attention in a
    /// summary view.
    pub fn needs_attention(&self) -> bool {
        matches!(self, TestStatus::Fail | TestStatus::Warning | TestStatus::Inconclusive)
    }

    /// Whether the test actually ran (as opposed to skipped/blocked).
    pub fn ran(&self) -> bool {
        matches!(
            self,
            TestStatus::Pass | TestStatus::Fail | TestStatus::Warning | TestStatus::Inconclusive | TestStatus::Manual
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn blocked_is_not_a_failure() {
        assert!(!TestStatus::Blocked.is_failure());
        assert!(!TestStatus::Blocked.ran());
    }

    #[test]
    fn failure_statuses() {
        assert!(TestStatus::Fail.is_failure());
        assert!(TestStatus::Warning.needs_attention());
        assert!(TestStatus::Inconclusive.needs_attention());
    }

    #[test]
    fn status_string_forms() {
        assert_eq!(serde_json::to_string(&TestStatus::Blocked).unwrap(), "\"blocked\"");
    }
}