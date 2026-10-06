//! Manual and semi-automated workflow (§15, §4.5).
//!
//! Three execution modes exist; this module provides the model for the two
//! that need a human:
//!
//! * **Manual** — the engineer is the instrument. A [`ManualChecklist`] lists
//!   what to inspect (Pass / Fail / N/A per item, plus note and evidence);
//!   the result stays `TestStatus::Manual` until the checklist is complete
//!   and [`TestResult::confirm`] is called.
//! * **Semi-automated** — software prepares the test and captures
//!   measurements; the runner marks the result pending confirmation and the
//!   engineer approves or rejects the measured outcome.
//!
//! Automated results are final: `confirm` on them is an error (re-run
//! instead).

use serde::{Deserialize, Serialize};

use tpt_app_av_commissioning_model::EvidenceRef;

use crate::definition::{CommissioningTest, ExecutionMode, TestError, TestId, TestRequirements};
use crate::result::TestResult;
use crate::status::TestStatus;
use crate::test_kind::TestKind;

/// An engineer's verdict on one checklist item (§15: Pass / Fail / N/A).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ChecklistVerdict {
    /// The item checked out.
    Pass,
    /// The item failed inspection.
    Fail,
    /// The item does not apply to this installation.
    NotApplicable,
}

impl ChecklistVerdict {
    /// Stable lowercase name.
    pub fn as_str(&self) -> &'static str {
        match self {
            ChecklistVerdict::Pass => "pass",
            ChecklistVerdict::Fail => "fail",
            ChecklistVerdict::NotApplicable => "not_applicable",
        }
    }
}

/// One inspection step of a [`ManualChecklist`].
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ChecklistItem {
    /// Stable id within the checklist (e.g. `item-1` or an authored id).
    pub id: String,
    /// What the engineer should check (e.g. "focus uniform across screen").
    pub label: String,
    /// The engineer's verdict; `None` until confirmed.
    pub verdict: Option<ChecklistVerdict>,
    /// Free-form observation attached to the verdict.
    pub note: Option<String>,
    /// Evidence captured for this item (photo, screenshot, …).
    pub evidence: Vec<EvidenceRef>,
}

/// Errors raised while working with a [`ManualChecklist`].
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum ChecklistError {
    #[error("no checklist item `{0}`")]
    UnknownItem(String),
    #[error("checklist item `{0}` already has a verdict")]
    AlreadyConfirmed(String),
    #[error("checklist has unanswered items: {0:?}")]
    Incomplete(Vec<String>),
}

/// The checklist a manual test asks the engineer to work through.
///
/// The checklist is part of the test definition (authored in the DSL or built
/// in code); verdicts, notes and evidence are filled in at execution time and
/// carried on the `TestResult` so reports can show what was asked and what
/// was answered — never an implied measurement that did not occur.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ManualChecklist {
    pub title: String,
    pub items: Vec<ChecklistItem>,
}

impl ManualChecklist {
    /// Build a checklist from item labels; ids are assigned positionally
    /// (`item-1`, `item-2`, …).
    pub fn new(
        title: impl Into<String>,
        labels: impl IntoIterator<Item = impl Into<String>>,
    ) -> Self {
        Self {
            title: title.into(),
            items: labels
                .into_iter()
                .enumerate()
                .map(|(i, label)| ChecklistItem {
                    id: format!("item-{}", i + 1),
                    label: label.into(),
                    verdict: None,
                    note: None,
                    evidence: Vec::new(),
                })
                .collect(),
        }
    }

    /// Record a verdict (with optional note and evidence) for one item.
    pub fn confirm_item(
        &mut self,
        id: &str,
        verdict: ChecklistVerdict,
        note: Option<String>,
        evidence: Option<EvidenceRef>,
    ) -> Result<(), ChecklistError> {
        let item = self
            .items
            .iter_mut()
            .find(|i| i.id == id)
            .ok_or_else(|| ChecklistError::UnknownItem(id.to_owned()))?;
        if item.verdict.is_some() {
            return Err(ChecklistError::AlreadyConfirmed(id.to_owned()));
        }
        item.verdict = Some(verdict);
        item.note = note;
        if let Some(evidence) = evidence {
            item.evidence.push(evidence);
        }
        Ok(())
    }

    /// Whether every item has a verdict.
    pub fn is_complete(&self) -> bool {
        self.items.iter().all(|i| i.verdict.is_some())
    }

    /// Ids of items still awaiting a verdict.
    pub fn outstanding(&self) -> Vec<String> {
        self.items
            .iter()
            .filter(|i| i.verdict.is_none())
            .map(|i| i.id.clone())
            .collect()
    }

    /// The checklist's overall status: `None` until complete; `Fail` if any
    /// item failed, `Pass` otherwise (`NotApplicable` items never fail a
    /// checklist).
    pub fn overall(&self) -> Option<TestStatus> {
        if !self.is_complete() {
            return None;
        }
        let failed = self
            .items
            .iter()
            .any(|i| i.verdict == Some(ChecklistVerdict::Fail));
        Some(if failed {
            TestStatus::Fail
        } else {
            TestStatus::Pass
        })
    }
}

/// What a semi-automated result is waiting for (§15).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PendingConfirmation {
    /// The status the software part concluded with (restored on approval).
    pub software_status: Option<TestStatus>,
    /// What the engineer is being asked to confirm.
    pub prompt: String,
}

/// The engineer's overall call on a pending manual / semi-automated result.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Confirmation {
    /// Accept the result (semi-automated: the measured outcome stands).
    Approve,
    /// Reject the result — the final status becomes `Fail`.
    Reject,
}

/// Why a result cannot be confirmed.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum ConfirmationError {
    #[error("{0}")]
    NotConfirmable(String),
}

/// A manual test (§15): the engineer performs the physical/visual test.
///
/// The runner never runs software steps for it — executing it produces a
/// `TestStatus::Manual` result carrying the checklist, awaiting the
/// engineer's verdicts and [`crate::result::TestResult::confirm`].
pub struct ManualTest {
    id: TestId,
    name: String,
    kind: Option<TestKind>,
    checklist: ManualChecklist,
    requirements: TestRequirements,
}

impl ManualTest {
    pub fn new(id: impl Into<String>, name: impl Into<String>, checklist: ManualChecklist) -> Self {
        Self {
            id: TestId::new(id),
            name: name.into(),
            kind: None,
            checklist,
            requirements: TestRequirements::manual(),
        }
    }

    /// Pin the test's category (§13). Informational only.
    pub fn with_kind(mut self, kind: TestKind) -> Self {
        self.kind = Some(kind);
        self
    }

    /// Devices the checklist is about (context only — a manual test touches
    /// no devices itself, so it never takes a `DeviceLock`).
    pub fn using_devices(
        mut self,
        devices: impl IntoIterator<Item = tpt_app_av_commissioning_model::DeviceId>,
    ) -> Self {
        self.requirements.devices.extend(devices);
        self
    }

    /// Declare ordering dependencies (§14).
    pub fn depends_on(mut self, deps: impl IntoIterator<Item = TestId>) -> Self {
        self.requirements.depends_on.extend(deps);
        self
    }

    /// The checklist this test asks the engineer to work through.
    pub fn checklist(&self) -> &ManualChecklist {
        &self.checklist
    }
}

impl CommissioningTest for ManualTest {
    fn id(&self) -> &TestId {
        &self.id
    }
    fn name(&self) -> &str {
        &self.name
    }
    fn kind(&self) -> Option<TestKind> {
        self.kind
    }
    fn requirements(&self) -> &TestRequirements {
        &self.requirements
    }
    fn checklist(&self) -> Option<ManualChecklist> {
        Some(self.checklist.clone())
    }

    fn execute(&self) -> Result<TestResult, TestError> {
        let mut r = TestResult::new(self.id.clone(), TestStatus::Manual, ExecutionMode::Manual)
            .message("manual test: awaiting engineer verdict");
        r.checklist = Some(self.checklist.clone());
        Ok(r)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn checklist() -> ManualChecklist {
        ManualChecklist::new(
            "Projector image quality",
            ["Focus uniform across the screen", "Geometry undistorted"],
        )
    }

    #[test]
    fn items_get_positional_ids() {
        let cl = checklist();
        assert_eq!(cl.items[0].id, "item-1");
        assert_eq!(cl.items[1].id, "item-2");
        assert!(cl.items.iter().all(|i| i.verdict.is_none()));
    }

    #[test]
    fn confirm_item_then_overall() {
        let mut cl = checklist();
        assert_eq!(cl.overall(), None);
        cl.confirm_item("item-1", ChecklistVerdict::Pass, None, None)
            .unwrap();
        assert_eq!(cl.overall(), None); // still incomplete
        cl.confirm_item("item-2", ChecklistVerdict::NotApplicable, None, None)
            .unwrap();
        assert_eq!(cl.overall(), Some(TestStatus::Pass));
    }

    #[test]
    fn a_failed_item_fails_the_checklist() {
        let mut cl = checklist();
        cl.confirm_item("item-1", ChecklistVerdict::Pass, None, None)
            .unwrap();
        cl.confirm_item(
            "item-2",
            ChecklistVerdict::Fail,
            Some("keystone visible".into()),
            None,
        )
        .unwrap();
        assert_eq!(cl.overall(), Some(TestStatus::Fail));
    }

    #[test]
    fn double_confirmation_is_rejected() {
        let mut cl = checklist();
        cl.confirm_item("item-1", ChecklistVerdict::Pass, None, None)
            .unwrap();
        assert_eq!(
            cl.confirm_item("item-1", ChecklistVerdict::Fail, None, None),
            Err(ChecklistError::AlreadyConfirmed("item-1".into()))
        );
        assert_eq!(cl.items[0].verdict, Some(ChecklistVerdict::Pass));
    }

    #[test]
    fn unknown_item_is_rejected() {
        let mut cl = checklist();
        assert_eq!(
            cl.confirm_item("nope", ChecklistVerdict::Pass, None, None),
            Err(ChecklistError::UnknownItem("nope".into()))
        );
    }

    #[test]
    fn verdicts_round_trip_through_json() {
        let mut cl = checklist();
        cl.confirm_item("item-1", ChecklistVerdict::NotApplicable, None, None)
            .unwrap();
        let json = serde_json::to_string(&cl).unwrap();
        assert!(json.contains("\"not_applicable\""));
        let back: ManualChecklist = serde_json::from_str(&json).unwrap();
        assert_eq!(back, cl);
    }

    #[test]
    fn manual_test_executes_to_pending_manual_result() {
        let t = ManualTest::new("projector.image", "Projector image quality", checklist())
            .with_kind(TestKind::Video);
        assert_eq!(t.kind(), Some(TestKind::Video));
        assert!(t.requirements().mutate_devices.is_empty());

        let r = t.execute().unwrap();
        assert_eq!(r.status, TestStatus::Manual);
        assert_eq!(r.mode, ExecutionMode::Manual);
        let cl = r.checklist.as_ref().unwrap();
        assert_eq!(cl.items.len(), 2);
        assert!(r.needs_confirmation());
    }

    #[test]
    fn manual_confirmation_uses_the_checklist() {
        let t = ManualTest::new("m", "Manual", checklist());
        let result = t.execute().unwrap();

        // Confirming before the checklist is complete is an error.
        let incomplete = result
            .clone()
            .confirm(Confirmation::Approve, "J. Doe", None)
            .unwrap_err();
        assert!(incomplete.to_string().contains("unanswered"));

        let mut confirmed = result;
        confirmed
            .checklist
            .as_mut()
            .unwrap()
            .confirm_item("item-1", ChecklistVerdict::Pass, None, None)
            .unwrap();
        confirmed
            .checklist
            .as_mut()
            .unwrap()
            .confirm_item("item-2", ChecklistVerdict::Fail, None, None)
            .unwrap();
        let finalised = confirmed
            .confirm(Confirmation::Approve, "J. Doe", Some("seen it"))
            .unwrap();
        assert_eq!(finalised.status, TestStatus::Fail);
        assert!(!finalised.needs_confirmation());
        assert!(finalised
            .messages
            .iter()
            .any(|m| m.contains("approved by J. Doe")));
        assert!(finalised.messages.iter().any(|m| m.contains("seen it")));
    }

    #[test]
    fn automated_results_are_never_confirmable() {
        let r = TestResult::new(TestId::new("a"), TestStatus::Pass, ExecutionMode::Automated);
        assert_eq!(
            r.confirm(Confirmation::Approve, "J. Doe", None),
            Err(ConfirmationError::NotConfirmable(
                "automated results are final; re-run the test instead".to_owned()
            ))
        );
    }
}
