//! Test results (§12.2).

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use tpt_app_av_commissioning_model::{EvidenceRef, Measurement};

use crate::definition::{ExecutionMode, TestId};
use crate::status::TestStatus;

/// The outcome of one test execution.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TestResult {
    pub test_id: TestId,
    pub status: TestStatus,
    pub mode: ExecutionMode,
    pub started_at: DateTime<Utc>,
    pub completed_at: DateTime<Utc>,
    /// References to immutable evidence assets captured by this test.
    pub evidence: Vec<EvidenceRef>,
    /// Raw measurements from this test.
    pub measurements: Vec<Measurement>,
    /// Human-readable outcome messages (chronological order).
    pub messages: Vec<String>,
    /// Primary error message when the test failed infra-level.
    pub error: Option<String>,
}

impl TestResult {
    /// Create a result with the given status, timestamps set to now.
    pub fn new(test_id: TestId, status: TestStatus, mode: ExecutionMode) -> Self {
        let now = Utc::now();
        Self {
            test_id,
            status,
            mode,
            started_at: now,
            completed_at: now,
            evidence: Vec::new(),
            measurements: Vec::new(),
            messages: Vec::new(),
            error: None,
        }
    }

    /// Attach a measurement.
    pub fn with_measurement(mut self, measurement: Measurement) -> Self {
        self.measurements.push(measurement);
        self
    }

    /// Attach an evidence reference.
    pub fn with_evidence(mut self, evidence: EvidenceRef) -> Self {
        self.evidence.push(evidence);
        self
    }

    /// Append a human-readable message.
    pub fn message(mut self, message: impl Into<String>) -> Self {
        self.messages.push(message.into());
        self
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn result_builder() {
        let r = TestResult::new(
            TestId::new("power-on"),
            TestStatus::Pass,
            ExecutionMode::Automated,
        )
        .message("device reported on")
        .with_measurement(tpt_app_av_commissioning_model::Measurement::new(
            "response_time_ms",
            18,
        ));
        assert_eq!(r.messages, vec!["device reported on"]);
        assert_eq!(r.measurements.len(), 1);
        assert!(r.status.ran());
    }
}