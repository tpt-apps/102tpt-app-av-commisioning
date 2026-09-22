//! Evidence model (§24): immutable project assets referenced from results.
//!
//! Binary evidence lives in the managed project asset directory; the database
//! (and reports) only reference it by relative path.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use crate::id::ProjectId;

/// What kind of evidence was captured.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EvidenceKind {
    Screenshot,
    Photo,
    AudioRecording,
    VideoRecording,
    NetworkResult,
    DeviceResponse,
    Measurement,
    Configuration,
    OperatorNote,
}

/// A reference to an immutable evidence asset for a project.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EvidenceRef {
    pub id: EvidenceId,
    pub project: ProjectId,
    pub kind: EvidenceKind,
    /// Path relative to the project assets root (e.g.
    /// `assets/evidence/run-42/001-screenshot.png`).
    pub relative_path: String,
    pub description: Option<String>,
    pub captured_at: DateTime<Utc>,
    /// Content hash (e.g. SHA-256) so results can prove the asset was not
    /// altered after capture.
    pub sha256: Option<String>,
}

id_type! {
    /// Identifies an evidence asset.
    EvidenceId
}

/// Identifies a defect (targeted tracking linkage).
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(transparent)]
pub struct DefectId(String);

impl DefectId {
    pub fn new<S: Into<String>>(value: S) -> Self {
        Self(value.into())
    }
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl std::fmt::Display for DefectId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn evidence_serde_round_trip() {
        let ev = EvidenceRef {
            id: EvidenceId::new("ev-1"),
            project: ProjectId::new("prj-1"),
            kind: EvidenceKind::Screenshot,
            relative_path: "assets/evidence/run-1/a.png".to_owned(),
            description: Some("Signal present on display".to_owned()),
            captured_at: DateTime::parse_from_rfc3339("2026-09-21T10:00:00Z")
                .unwrap()
                .with_timezone(&Utc),
            sha256: None,
        };
        let json = serde_json::to_string(&ev).unwrap();
        let back: EvidenceRef = serde_json::from_str(&json).unwrap();
        assert_eq!(back, ev);
        assert_eq!(ev.relative_path, back.relative_path);
    }
}