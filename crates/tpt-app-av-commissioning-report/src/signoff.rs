//! Sign-off (§32): engineer name, date, and result including
//! "PASS WITH ACCEPTED EXCEPTIONS" with an exception list.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

/// Who signed the report.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Signatory {
    pub name: String,
    pub title: Option<String>,
    pub company: Option<String>,
    /// When the signature was actually placed (audit-relevant).
    pub signed_at: Option<DateTime<Utc>>,
}

impl Signatory {
    pub fn new(name: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            title: None,
            company: None,
            signed_at: None,
        }
    }
}

/// The engineer's verdict.
///
/// `ApprovedWithExceptions` maps to the report wording
/// "PASS WITH ACCEPTED EXCEPTIONS" and always carries the exception list.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum SignOffResult {
    Approved,
    ApprovedWithExceptions {
        exceptions: Vec<String>,
    },
    NotApproved {
        reason: Option<String>,
    },
}

impl SignOffResult {
    /// Whether the installation passed handover.
    pub fn is_approved(&self) -> bool {
        matches!(self, SignOffResult::Approved | SignOffResult::ApprovedWithExceptions { .. })
    }

    /// The display wording used on the report.
    pub fn wording(&self) -> String {
        match self {
            SignOffResult::Approved => "PASS".to_owned(),
            SignOffResult::ApprovedWithExceptions { .. } => "PASS WITH ACCEPTED EXCEPTIONS".to_owned(),
            SignOffResult::NotApproved { .. } => "NOT APPROVED".to_owned(),
        }
    }
}

/// A completed sign-off block for a report.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SignOff {
    pub signatory: Signatory,
    pub result: SignOffResult,
    pub notes: Option<String>,
}

impl SignOff {
    /// Sign off on behalf of `signatory`. Caller persists the audit event.
    pub fn sign(signatory: Signatory, result: SignOffResult) -> Self {
        Self {
            signatory,
            result,
            notes: None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn approved_wording() {
        let mut off = SignOff::sign(
            Signatory::new("A. Engineer"),
            SignOffResult::ApprovedWithExceptions {
                exceptions: vec!["no spare projector lamp on site".to_owned()],
            },
        );
        off.signatory.signed_at = Some(Utc::now());
        assert_eq!(off.result.wording(), "PASS WITH ACCEPTED EXCEPTIONS");
        assert!(off.result.is_approved());
    }

    #[test]
    fn not_approved_wording() {
        let off = SignOff::sign(
            Signatory::new("A. Engineer"),
            SignOffResult::NotApproved {
                reason: Some("audio path failing".to_owned()),
            },
        );
        assert_eq!(off.result.wording(), "NOT APPROVED");
        assert!(!off.result.is_approved());
    }
}