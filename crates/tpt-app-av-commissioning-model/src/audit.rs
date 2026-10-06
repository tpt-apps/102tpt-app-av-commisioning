//! Audit log (§33): significant project actions are recorded so the
//! commissioning story is provable — what ran, what changed, who decided.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use serde_json::Value as JsonValue;

/// What happened (§33).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AuditEventType {
    ProjectCreated,
    DeviceAdded,
    DeviceModified,
    TestRun,
    ResultChanged,
    DefectCreated,
    DefectStatusChanged,
    DefectClosed,
    BaselineCreated,
    ConfigurationImported,
    ReportGenerated,
    ReportSigned,
}

impl AuditEventType {
    /// Stable lowercase name.
    pub fn as_str(&self) -> &'static str {
        match self {
            AuditEventType::ProjectCreated => "project_created",
            AuditEventType::DeviceAdded => "device_added",
            AuditEventType::DeviceModified => "device_modified",
            AuditEventType::TestRun => "test_run",
            AuditEventType::ResultChanged => "result_changed",
            AuditEventType::DefectCreated => "defect_created",
            AuditEventType::DefectStatusChanged => "defect_status_changed",
            AuditEventType::DefectClosed => "defect_closed",
            AuditEventType::BaselineCreated => "baseline_created",
            AuditEventType::ConfigurationImported => "configuration_imported",
            AuditEventType::ReportGenerated => "report_generated",
            AuditEventType::ReportSigned => "report_signed",
        }
    }
}

/// Who performed an action.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Actor {
    /// A named person (engineer, operator).
    User(String),
    /// The software itself (automated runs, generated reports).
    System,
}

impl Actor {
    /// Display name for reports and logs.
    pub fn display(&self) -> &str {
        match self {
            Actor::User(name) => name,
            Actor::System => "system",
        }
    }
}

/// One recorded project action (§33).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AuditEvent {
    pub timestamp: DateTime<Utc>,
    pub event_type: AuditEventType,
    pub actor: Actor,
    /// The object the event is about (device id, run id, defect id, …),
    /// when there is one.
    pub object_id: Option<String>,
    /// Free-form structured detail (e.g. changed fields, statuses).
    pub details: JsonValue,
}

impl AuditEvent {
    /// Record an event that happened now.
    pub fn now(
        event_type: AuditEventType,
        actor: Actor,
        object_id: Option<String>,
        details: JsonValue,
    ) -> Self {
        Self {
            timestamp: Utc::now(),
            event_type,
            actor,
            object_id,
            details,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn events_round_trip_through_json() {
        let event = AuditEvent::now(
            AuditEventType::DefectCreated,
            Actor::User("J. Doe".into()),
            Some("DEF-001".into()),
            json!({ "severity": "major", "title": "No signal" }),
        );
        let json = serde_json::to_string(&event).unwrap();
        assert!(json.contains("\"defect_created\""));
        assert!(json.contains("\"user\""));
        let back: AuditEvent = serde_json::from_str(&json).unwrap();
        assert_eq!(back, event);
    }

    #[test]
    fn actor_display_names() {
        assert_eq!(Actor::User("J. Doe".into()).display(), "J. Doe");
        assert_eq!(Actor::System.display(), "system");
    }

    #[test]
    fn event_type_names_are_stable() {
        assert_eq!(AuditEventType::TestRun.as_str(), "test_run");
        assert_eq!(AuditEventType::ReportSigned.as_str(), "report_signed");
    }
}
