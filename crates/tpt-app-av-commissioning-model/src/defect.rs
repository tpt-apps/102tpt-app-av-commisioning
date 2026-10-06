//! Defect tracking (§23).
//!
//! A failed test is convertible into a defect so commissioning and defect
//! resolution stay in one workflow. Defects reference the tests that found
//! them and the evidence captured; the report lists them in the handover.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use crate::evidence::EvidenceRef;
use crate::id::ProjectId;

id_type! {
    /// Identifies a defect.
    DefectId
}

/// How severe a defect is. Ordered from most to least severe so reports can
/// sort and summarise.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Severity {
    /// The installation does not meet its purpose; must be fixed.
    Critical,
    /// A significant function is impaired; must be fixed or accepted.
    Major,
    /// An annoyance or degradation with a workaround.
    Minor,
    /// Cosmetic only.
    Cosmetic,
}

impl Severity {
    /// Stable lowercase name.
    pub fn as_str(&self) -> &'static str {
        match self {
            Severity::Critical => "critical",
            Severity::Major => "major",
            Severity::Minor => "minor",
            Severity::Cosmetic => "cosmetic",
        }
    }
}

/// Where a defect is in its life (§23).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DefectStatus {
    /// Recorded, not yet looked at.
    Open,
    /// Being investigated.
    Investigating,
    /// A fix is in place awaiting a retest.
    Fixed,
    /// The retest failed or was inconclusive; more work is needed.
    RetestRequired,
    /// The retest passed; resolved.
    Verified,
    /// Knowingly accepted (e.g. "PASS WITH ACCEPTED EXCEPTIONS").
    Accepted,
    /// Deliberately not addressed in this commissioning round.
    Deferred,
}

impl DefectStatus {
    /// Stable lowercase name.
    pub fn as_str(&self) -> &'static str {
        match self {
            DefectStatus::Open => "open",
            DefectStatus::Investigating => "investigating",
            DefectStatus::Fixed => "fixed",
            DefectStatus::RetestRequired => "retest_required",
            DefectStatus::Verified => "verified",
            DefectStatus::Accepted => "accepted",
            DefectStatus::Deferred => "deferred",
        }
    }

    /// Whether the defect still needs work or a decision — i.e. it is not in
    /// a terminal state (`Verified`, `Accepted`, `Deferred`).
    pub fn is_open(&self) -> bool {
        !matches!(
            self,
            DefectStatus::Verified | DefectStatus::Accepted | DefectStatus::Deferred
        )
    }
}

/// A defect found during commissioning (§23).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Defect {
    pub id: DefectId,
    pub project: ProjectId,
    pub severity: Severity,
    pub title: String,
    pub description: String,
    /// The tests that found (or retest) this defect, by test id. Plain
    /// strings: the model crate does not depend on the test crate's
    /// `TestId` newtype.
    pub related_tests: Vec<String>,
    /// Evidence captured for this defect.
    pub evidence: Vec<EvidenceRef>,
    pub status: DefectStatus,
    pub created_at: DateTime<Utc>,
}

impl Defect {
    /// Create an open defect.
    pub fn new(
        id: impl Into<String>,
        project: ProjectId,
        severity: Severity,
        title: impl Into<String>,
        description: impl Into<String>,
    ) -> Self {
        Self {
            id: DefectId::new(id),
            project,
            severity,
            title: title.into(),
            description: description.into(),
            related_tests: Vec::new(),
            evidence: Vec::new(),
            status: DefectStatus::Open,
            created_at: Utc::now(),
        }
    }

    /// Link a test to this defect (e.g. after a retest).
    pub fn relate_test(&mut self, test_id: impl Into<String>) {
        self.related_tests.push(test_id.into());
    }

    /// Attach evidence.
    pub fn with_evidence(&mut self, evidence: EvidenceRef) {
        self.evidence.push(evidence);
    }

    /// Move to another status. Transitions are free — the audit log records
    /// who changed what — but `RetestRequired` exists so a failed retest is
    /// never silently reported as fixed.
    pub fn transition(&mut self, status: DefectStatus) {
        self.status = status;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::id::ProjectId;

    #[test]
    fn open_statuses_and_terminals() {
        assert!(DefectStatus::Open.is_open());
        assert!(DefectStatus::Investigating.is_open());
        assert!(DefectStatus::Fixed.is_open());
        assert!(DefectStatus::RetestRequired.is_open());
        assert!(!DefectStatus::Verified.is_open());
        assert!(!DefectStatus::Accepted.is_open());
        assert!(!DefectStatus::Deferred.is_open());
    }

    #[test]
    fn severity_orders_from_critical_to_cosmetic() {
        let mut severities = [Severity::Minor, Severity::Critical, Severity::Cosmetic];
        severities.sort();
        assert_eq!(
            severities,
            [Severity::Critical, Severity::Minor, Severity::Cosmetic]
        );
    }

    #[test]
    fn transitions_and_linkage() {
        let mut defect = Defect::new("D", ProjectId::new("p"), Severity::Minor, "t", "d");
        defect.transition(DefectStatus::Fixed);
        assert_eq!(defect.status.as_str(), "fixed");
        defect.transition(DefectStatus::RetestRequired);
        assert!(defect.status.is_open());
        defect.relate_test("display.signal-retest");
        assert_eq!(defect.related_tests, vec!["display.signal-retest"]);

        let json = serde_json::to_string(&defect).unwrap();
        assert!(json.contains("\"retest_required\""));
        let back: Defect = serde_json::from_str(&json).unwrap();
        assert_eq!(back, defect);
    }
}
