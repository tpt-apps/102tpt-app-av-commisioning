//! Report model and renderers (§31–32).
//!
//! Implemented outputs: JSON, CSV, Markdown. PDF and HTML rendering are
//! deferred (see `todo.md` Phase 20).

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use tpt_app_av_commissioning_model::DefectId;
use tpt_app_av_commissioning_test::{TestResult, TestStatus};

use crate::signoff::{SignOff, SignOffResult};

/// Supported report outputs. Not all variants render yet (see
/// [`Report::render`]).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ReportFormat {
    Pdf,
    Html,
    Csv,
    Json,
    Markdown,
}

/// Errors produced while generating a report.
#[derive(Debug, thiserror::Error)]
pub enum ReportError {
    #[error("JSON serialization failed: {0}")]
    Json(#[from] serde_json::Error),
    #[error("rendering to `{0:?}` is not implemented yet")]
    Unsupported(ReportFormat),
}

/// A roll-up of result statuses used for the summary page.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct ReportSummary {
    pub total: usize,
    pub pass: usize,
    pub fail: usize,
    pub warning: usize,
    pub blocked: usize,
    pub skipped: usize,
    pub manual: usize,
    pub inconclusive: usize,
}

impl ReportSummary {
    /// Summarise a set of results.
    pub fn from_results(results: &[TestResult]) -> Self {
        use TestStatus::*;
        let mut s = Self {
            total: results.len(),
            ..Self::default()
        };
        for r in results {
            match r.status {
                Pass => s.pass += 1,
                Fail => s.fail += 1,
                Warning => s.warning += 1,
                Blocked => s.blocked += 1,
                Skipped => s.skipped += 1,
                Manual => s.manual += 1,
                Inconclusive => s.inconclusive += 1,
            }
        }
        s
    }
}

/// A defect line shown on the report (full defect tracking is §23).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DefectLine {
    pub id: DefectId,
    pub title: String,
    pub status: String,
}

/// A complete commissioning report.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Report {
    // Cover / project details.
    pub project_name: String,
    pub project_id: String,
    pub client: Option<String>,
    pub site: Option<String>,
    pub generated_at: DateTime<Utc>,
    pub software_version: String,
    pub room_count: usize,
    pub device_count: usize,
    pub connection_count: usize,

    pub summary: ReportSummary,
    pub results: Vec<TestResult>,
    pub defects: Vec<DefectLine>,
    /// Names of evidence assets referenced by the results.
    pub evidence: Vec<String>,

    pub sign_off: Option<SignOff>,
}

impl Report {
    /// Create a report. `sign_off` is attached by the caller after signing.
    pub fn new(project_name: impl Into<String>, project_id: impl Into<String>) -> Self {
        Self {
            project_name: project_name.into(),
            project_id: project_id.into(),
            client: None,
            site: None,
            generated_at: Utc::now(),
            software_version: option_env!("CARGO_PKG_VERSION").unwrap_or("dev").to_owned(),
            room_count: 0,
            device_count: 0,
            connection_count: 0,
            summary: ReportSummary::default(),
            results: Vec::new(),
            defects: Vec::new(),
            evidence: Vec::new(),
            sign_off: None,
        }
    }

    /// Recompute the summary from current results.
    pub fn refresh_summary(&mut self) {
        self.summary = ReportSummary::from_results(&self.results);
        self.evidence = self
            .results
            .iter()
            .flat_map(|r| r.evidence.iter().map(|e| e.relative_path.clone()))
            .collect();
    }

    /// Render this report to the requested format.
    pub fn render(&self, format: ReportFormat) -> Result<String, ReportError> {
        match format {
            ReportFormat::Json => Ok(serde_json::to_string_pretty(self)?),
            ReportFormat::Csv => Ok(self.to_csv()?),
            ReportFormat::Markdown => Ok(self.to_markdown()),
            ReportFormat::Pdf | ReportFormat::Html => Err(ReportError::Unsupported(format)),
        }
    }

    fn to_csv(&self) -> Result<String, ReportError> {
        let mut out = String::from("test_id,status,mode,started_at,completed_at,error\n");
        for r in &self.results {
            let error = r.error.as_deref().unwrap_or("");
            out.push_str(&format!(
                "{},{},{},{},{},{}\n",
                r.test_id,
                r.status.as_str(),
                r.mode.as_str(),
                r.started_at.to_rfc3339(),
                r.completed_at.to_rfc3339(),
                error.replace(',', " ")
            ));
        }
        Ok(out)
    }

    fn to_markdown(&self) -> String {
        let mut s = String::new();
        s.push_str("# TPT AV Commissioning Report\n\n");
        s.push_str(&format!(
            "**Project:** {} (`{}`)\n",
            self.project_name, self.project_id
        ));
        if let Some(client) = &self.client {
            s.push_str(&format!("**Client:** {}\n", client));
        }
        if let Some(site) = &self.site {
            s.push_str(&format!("**Site:** {}\n", site));
        }
        s.push_str(&format!(
            "**Generated:** {}\n",
            self.generated_at.to_rfc3339()
        ));
        s.push_str(&format!("**Software:** {}\n\n", self.software_version));

        s.push_str(&format!(
            "Rooms: {} · Devices: {} · Connections: {}\n\n",
            self.room_count, self.device_count, self.connection_count
        ));

        let summary = &self.summary;
        s.push_str("## Summary\n\n");
        s.push_str(
            "| Total | Pass | Fail | Warning | Blocked | Skipped | Manual | Inconclusive |\n",
        );
        s.push_str("|---|---|---|---|---|---|---|---|\n");
        s.push_str(&format!(
            "| {} | {} | {} | {} | {} | {} | {} | {} |\n\n",
            summary.total,
            summary.pass,
            summary.fail,
            summary.warning,
            summary.blocked,
            summary.skipped,
            summary.manual,
            summary.inconclusive
        ));

        s.push_str("## Results\n\n");
        s.push_str("| Test | Status | Messages |\n|---|---|---|\n");
        for r in &self.results {
            let messages = r.messages.join("; ");
            s.push_str(&format!(
                "| {} | {} | {} |\n",
                r.test_id,
                r.status.as_str(),
                messages
            ));
        }

        if !self.defects.is_empty() {
            s.push_str("\n## Defects\n\n");
            for d in &self.defects {
                s.push_str(&format!("- `{}` [{}] {}\n", d.id, d.status, d.title));
            }
        }

        if let Some(off) = &self.sign_off {
            s.push_str("\n## Sign-off\n\n");
            s.push_str(&format!(
                "**{}** — {}\n",
                off.signatory.name,
                off.result.wording()
            ));
            if let SignOffResult::ApprovedWithExceptions { exceptions } = &off.result {
                if !exceptions.is_empty() {
                    s.push_str("\nExceptions:\n");
                    for e in exceptions {
                        s.push_str(&format!("- {}\n", e));
                    }
                }
            }
        }

        s
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tpt_app_av_commissioning_test::{ExecutionMode, TestId, TestResult};

    fn result(id: &str, status: TestStatus) -> TestResult {
        TestResult::new(TestId::new(id), status, ExecutionMode::Automated)
    }

    #[test]
    fn summary_counts_results() {
        let results = vec![
            result("a", TestStatus::Pass),
            result("b", TestStatus::Pass),
            result("c", TestStatus::Fail),
            result("d", TestStatus::Blocked),
            result("e", TestStatus::Manual),
        ];
        let s = ReportSummary::from_results(&results);
        assert_eq!(s.total, 5);
        assert_eq!(s.pass, 2);
        assert_eq!(s.fail, 1);
        assert_eq!(s.blocked, 1);
        assert_eq!(s.manual, 1);
    }

    #[test]
    fn json_rendering_round_trips() {
        let mut report = Report::new("Boardroom", "prj-1");
        report.results = vec![result("power", TestStatus::Pass)];
        report.refresh_summary();
        let json = report.render(ReportFormat::Json).unwrap();
        let back: Report = serde_json::from_str(&json).unwrap();
        assert_eq!(back.results.len(), 1);
        assert_eq!(back.summary.pass, 1);
    }

    #[test]
    fn markdown_rendering_includes_sections() {
        let mut report = Report::new("Boardroom", "prj-1");
        report.results = vec![result("power", TestStatus::Pass)];
        report.refresh_summary();
        let md = report.render(ReportFormat::Markdown).unwrap();
        assert!(md.contains("# TPT AV Commissioning Report"));
        assert!(md.contains("## Summary"));
    }
}
