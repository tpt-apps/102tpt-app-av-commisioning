//! Test results (§12.2).

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use tpt_app_av_commissioning_model::{EvidenceRef, Measurement};

use crate::definition::{ExecutionMode, TestId};
use crate::manual::{Confirmation, ConfirmationError, ManualChecklist, PendingConfirmation};
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
    /// The checklist a manual test asks the engineer to work through (§15),
    /// with verdicts, notes and evidence as answered.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub checklist: Option<ManualChecklist>,
    /// Set while a semi-automated result awaits the engineer's confirmation
    /// (§15): the software part is done, the status is provisional.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pending: Option<PendingConfirmation>,
    /// The test changed device state and could not restore it afterwards
    /// (§37). Reporting is enforced by [`TestResult::apply_restoration_failure`].
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub restoration_failed: bool,
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
            checklist: None,
            pending: None,
            restoration_failed: false,
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

    /// Attach a manual checklist.
    pub fn with_checklist(mut self, checklist: ManualChecklist) -> Self {
        self.checklist = Some(checklist);
        self
    }

    /// Mark the result as awaiting the engineer's confirmation.
    pub fn awaiting_confirmation(mut self, pending: PendingConfirmation) -> Self {
        self.pending = Some(pending);
        self
    }

    /// Record that state restoration failed after the test ran (§37) and
    /// apply the explicit reporting contract: the message "Test passed, but
    /// device state restoration failed." is added, and a `Pass` outcome
    /// becomes `Warning` — it needs human attention, not celebration.
    /// Failure outcomes keep their status; the message is still added.
    pub fn apply_restoration_failure(&mut self) {
        if !self.restoration_failed {
            return;
        }
        let message = "Test passed, but device state restoration failed.";
        if !self.messages.iter().any(|m| m == message) {
            self.messages.push(message.to_owned());
        }
        if self.status == TestStatus::Pass {
            self.status = TestStatus::Warning;
        }
    }

    /// Append a human-readable message.
    pub fn message(mut self, message: impl Into<String>) -> Self {
        self.messages.push(message.into());
        self
    }

    /// Whether this result still awaits an engineer (§15): it is pending
    /// confirmation, or it is a manual test not yet worked through.
    pub fn needs_confirmation(&self) -> bool {
        self.pending.is_some()
            || (self.mode == ExecutionMode::Manual && self.status == TestStatus::Manual)
    }

    /// Record the engineer's verdict on a manual or semi-automated result.
    ///
    /// * Semi-automated: approval restores the software part's status;
    ///   rejection fails the test.
    /// * Manual: the checklist (if present) must be complete; approval takes
    ///   the checklist's overall status (any failed item fails the test),
    ///   rejection fails it.
    /// * Automated results are final — an error is returned; re-run instead.
    pub fn confirm(
        mut self,
        confirmation: Confirmation,
        engineer: &str,
        note: Option<&str>,
    ) -> Result<TestResult, ConfirmationError> {
        if self.mode == ExecutionMode::Automated {
            return Err(ConfirmationError::NotConfirmable(
                "automated results are final; re-run the test instead".to_owned(),
            ));
        }

        let checklist_status = match &self.checklist {
            Some(checklist) => {
                let outstanding = checklist.outstanding();
                if !outstanding.is_empty() {
                    return Err(ConfirmationError::NotConfirmable(format!(
                        "checklist has unanswered items: {outstanding:?}"
                    )));
                }
                checklist.overall()
            }
            None => None,
        };

        self.status = match (&self.pending, checklist_status, confirmation) {
            // A rejected result always fails, whatever was measured.
            (_, _, Confirmation::Reject) => TestStatus::Fail,
            // Checklist failures stand even when the engineer approves.
            (_, Some(TestStatus::Fail), Confirmation::Approve) => TestStatus::Fail,
            (Some(pending), _, Confirmation::Approve) => {
                pending.software_status.unwrap_or(TestStatus::Pass)
            }
            (None, Some(status), Confirmation::Approve) => status,
            (None, None, Confirmation::Approve) => TestStatus::Pass,
        };
        self.pending = None;
        let verb = match confirmation {
            Confirmation::Approve => "approved",
            Confirmation::Reject => "rejected",
        };
        self.messages.push(format!("{verb} by {engineer}"));
        if let Some(note) = note {
            self.messages.push(note.to_owned());
        }
        self.completed_at = Utc::now();
        Ok(self)
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
        assert!(!r.needs_confirmation());
    }

    #[test]
    fn semi_automated_approval_restores_software_status() {
        let r = TestResult::new(
            TestId::new("s1"),
            TestStatus::Manual,
            ExecutionMode::SemiAutomated,
        )
        .awaiting_confirmation(PendingConfirmation {
            software_status: Some(TestStatus::Warning),
            prompt: "Is the measured level acceptable?".into(),
        })
        .message("measured level -18 dBFS");
        assert!(r.needs_confirmation());

        let confirmed = r.confirm(Confirmation::Approve, "J. Doe", None).unwrap();
        assert_eq!(confirmed.status, TestStatus::Warning);
        assert!(!confirmed.needs_confirmation());
        assert!(confirmed.messages.iter().any(|m| m.contains("J. Doe")));
    }

    #[test]
    fn semi_automated_rejection_fails() {
        let r = TestResult::new(
            TestId::new("s2"),
            TestStatus::Manual,
            ExecutionMode::SemiAutomated,
        )
        .awaiting_confirmation(PendingConfirmation {
            software_status: Some(TestStatus::Pass),
            prompt: "Confirm".into(),
        });
        let confirmed = r
            .confirm(Confirmation::Reject, "J. Doe", Some("too dim"))
            .unwrap();
        assert_eq!(confirmed.status, TestStatus::Fail);
        assert!(confirmed.messages.iter().any(|m| m == "too dim"));
    }

    #[test]
    fn bare_manual_result_confirms_by_verdict() {
        let r = TestResult::new(TestId::new("m1"), TestStatus::Manual, ExecutionMode::Manual);
        let confirmed = r.confirm(Confirmation::Approve, "J. Doe", None).unwrap();
        assert_eq!(confirmed.status, TestStatus::Pass);
    }

    #[test]
    fn results_round_trip_through_json_without_new_fields() {
        // Older persisted results (no checklist / pending) still deserialize.
        let legacy = r#"{"test_id":"t","status":"pass","mode":"automated",
            "started_at":"2026-01-01T00:00:00Z","completed_at":"2026-01-01T00:00:01Z",
            "evidence":[],"measurements":[],"messages":[],"error":null}"#;
        let r: TestResult = serde_json::from_str(legacy).unwrap();
        assert_eq!(r.status, TestStatus::Pass);
        assert_eq!(r.checklist, None);
        assert_eq!(r.pending, None);
    }
}
