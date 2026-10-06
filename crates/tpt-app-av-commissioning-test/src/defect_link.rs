//! Defect workflow glue (§23): converting failed test results into defects.
//!
//! Lives here rather than in the model crate so it can see [`TestResult`]
//! without the model depending on the test crate.

use tpt_app_av_commissioning_model::{Defect, ProjectId, Severity};

use crate::result::TestResult;
use crate::status::TestStatus;

/// Convert a failed test result into a defect (§23).
///
/// Returns `None` for results that did not fail — a failed *test* is
/// convertible, not a warning (that is a finding to accept, not a defect by
/// default) or a blocked/skipped one (nothing was measured). The result's
/// evidence references carry over so the defect is provable, and the
/// messages become the description. `title` overrides the default
/// (`Test failed: <test id>`).
pub fn defect_from_result(
    id: impl Into<String>,
    project: ProjectId,
    severity: Severity,
    result: &TestResult,
    title: Option<String>,
) -> Option<Defect> {
    if result.status != TestStatus::Fail {
        return None;
    }
    let mut defect = Defect::new(
        id,
        project,
        severity,
        title.unwrap_or_else(|| format!("Test failed: {}", result.test_id)),
        result.messages.join("\n"),
    );
    defect.relate_test(result.test_id.as_str());
    for evidence in &result.evidence {
        defect.with_evidence(evidence.clone());
    }
    Some(defect)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::definition::{ExecutionMode, TestId};

    fn failed_result() -> TestResult {
        TestResult::new(
            TestId::new("display.signal"),
            TestStatus::Fail,
            ExecutionMode::Automated,
        )
        .message("no signal on hdmi2")
        .message("resolution read 0x0")
    }

    #[test]
    fn failed_results_convert_to_defects() {
        let defect = defect_from_result(
            "DEF-001",
            ProjectId::new("prj-1"),
            Severity::Major,
            &failed_result(),
            None,
        )
        .unwrap();
        assert_eq!(defect.id.as_str(), "DEF-001");
        assert_eq!(
            defect.status,
            tpt_app_av_commissioning_model::DefectStatus::Open
        );
        assert_eq!(defect.severity, Severity::Major);
        assert_eq!(defect.title, "Test failed: display.signal");
        assert!(defect.description.contains("no signal on hdmi2"));
        assert_eq!(defect.related_tests, vec!["display.signal"]);
    }

    #[test]
    fn non_failures_do_not_convert() {
        let pass = TestResult::new(TestId::new("t"), TestStatus::Pass, ExecutionMode::Automated);
        assert!(
            defect_from_result("D", ProjectId::new("p"), Severity::Minor, &pass, None).is_none()
        );
        let warning = TestResult::new(
            TestId::new("t"),
            TestStatus::Warning,
            ExecutionMode::Automated,
        );
        assert!(
            defect_from_result("D", ProjectId::new("p"), Severity::Minor, &warning, None).is_none()
        );
        let blocked = TestResult::new(
            TestId::new("t"),
            TestStatus::Blocked,
            ExecutionMode::Automated,
        );
        assert!(
            defect_from_result("D", ProjectId::new("p"), Severity::Minor, &blocked, None).is_none()
        );
    }

    #[test]
    fn evidence_carries_over_and_title_can_be_overridden() {
        let mut result = failed_result();
        result
            .evidence
            .push(tpt_app_av_commissioning_model::EvidenceRef {
                id: tpt_app_av_commissioning_model::EvidenceId::new("ev-1"),
                project: ProjectId::new("prj-1"),
                kind: tpt_app_av_commissioning_model::EvidenceKind::Screenshot,
                relative_path: "assets/evidence/r1/screenshot.png".into(),
                description: None,
                captured_at: chrono::Utc::now(),
                sha256: None,
            });
        let defect = defect_from_result(
            "DEF-002",
            ProjectId::new("prj-1"),
            Severity::Critical,
            &result,
            Some("HDMI 2 has no signal".into()),
        )
        .unwrap();
        assert_eq!(defect.title, "HDMI 2 has no signal");
        assert_eq!(defect.evidence.len(), 1);
        assert_eq!(defect.evidence[0].id.as_str(), "ev-1");
    }
}
