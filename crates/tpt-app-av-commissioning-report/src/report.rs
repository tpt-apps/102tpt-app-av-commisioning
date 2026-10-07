//! Report model and renderers (§31–32).
//!
//! Implemented outputs: JSON, CSV, Markdown, HTML. PDF rendering is
//! deferred (see `todo.md` Phase 20). The HTML output is a standalone
//! document (inline CSS, no external assets) and marks every status with a
//! labelled badge — the word and a symbol, never colour alone (§28).

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
            ReportFormat::Html => Ok(self.to_html()),
            ReportFormat::Pdf => Err(ReportError::Unsupported(format)),
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

    /// Standalone HTML document (§31): cover details, system summary,
    /// per-status summary, results with labelled status badges, defects, and
    /// sign-off. Inline CSS only — no external assets, safe to attach.
    fn to_html(&self) -> String {
        let mut s = String::with_capacity(8192);
        s.push_str("<!DOCTYPE html>\n<html lang=\"en\">\n<head>\n<meta charset=\"utf-8\">\n");
        s.push_str("<meta name=\"viewport\" content=\"width=device-width, initial-scale=1\">\n");
        s.push_str(&format!(
            "<title>Commissioning Report — {}</title>\n",
            html_escape(&self.project_name)
        ));
        s.push_str("<style>\n");
        s.push_str(STD_CSS);
        s.push_str("</style>\n</head>\n<body>\n");

        // Cover.
        s.push_str("<header>\n<h1>TPT AV Commissioning Report</h1>\n<dl class=\"cover\">\n");
        s.push_str(&format!(
            "<dt>Project</dt><dd>{} (<code>{}</code>)</dd>\n",
            html_escape(&self.project_name),
            html_escape(&self.project_id)
        ));
        if let Some(client) = &self.client {
            s.push_str(&format!(
                "<dt>Client</dt><dd>{}</dd>\n",
                html_escape(client)
            ));
        }
        if let Some(site) = &self.site {
            s.push_str(&format!("<dt>Site</dt><dd>{}</dd>\n", html_escape(site)));
        }
        s.push_str(&format!(
            "<dt>Generated</dt><dd><time>{}</time></dd>\n<dt>Software</dt><dd>{}</dd>\n",
            self.generated_at.to_rfc3339(),
            html_escape(&self.software_version)
        ));
        s.push_str(&format!(
            "<dt>System</dt><dd>{} rooms, {} devices, {} connections</dd>\n",
            self.room_count, self.device_count, self.connection_count
        ));
        s.push_str("</dl>\n</header>\n");

        // Summary.
        let sum = &self.summary;
        s.push_str("<section id=\"summary\">\n<h2>Summary</h2>\n<table>\n");
        s.push_str("<thead><tr><th>Total</th>");
        for label in [
            "Pass",
            "Fail",
            "Warning",
            "Blocked",
            "Skipped",
            "Manual",
            "Inconclusive",
        ] {
            s.push_str(&format!("<th>{label}</th>"));
        }
        s.push_str("</tr></thead>\n<tbody><tr>");
        s.push_str(&format!("<td>{}</td>", sum.total));
        for count in [
            sum.pass,
            sum.fail,
            sum.warning,
            sum.blocked,
            sum.skipped,
            sum.manual,
            sum.inconclusive,
        ] {
            s.push_str(&format!("<td>{count}</td>"));
        }
        s.push_str("</tr></tbody>\n</table>\n</section>\n");

        // Results.
        s.push_str("<section id=\"results\">\n<h2>Results</h2>\n<table>\n");
        s.push_str("<thead><tr><th>Test</th><th>Mode</th><th>Status</th><th>Messages</th></tr></thead>\n<tbody>\n");
        for r in &self.results {
            let messages = r.messages.join("; ");
            s.push_str(&format!(
                "<tr><td><code>{}</code></td><td>{}</td><td>{}</td><td>{}</td></tr>\n",
                html_escape(r.test_id.as_str()),
                html_escape(r.mode.as_str()),
                status_badge(r.status),
                html_escape(&messages)
            ));
        }
        s.push_str("</tbody>\n</table>\n</section>\n");

        // Defects.
        if !self.defects.is_empty() {
            s.push_str("<section id=\"defects\">\n<h2>Defects</h2>\n<ul>\n");
            for d in &self.defects {
                s.push_str(&format!(
                    "<li><code>{}</code> [{}] {}</li>\n",
                    html_escape(d.id.as_str()),
                    html_escape(&d.status),
                    html_escape(&d.title)
                ));
            }
            s.push_str("</ul>\n</section>\n");
        }

        // Sign-off.
        if let Some(off) = &self.sign_off {
            s.push_str("<section id=\"sign-off\">\n<h2>Sign-off</h2>\n");
            let signed = off
                .signatory
                .signed_at
                .map(|t| t.to_rfc3339())
                .unwrap_or_else(|| "not dated".to_owned());
            s.push_str(&format!(
                "<p><strong>{}</strong>{}{} — <span class=\"verdict\">{}</span> (signed {})</p>\n",
                html_escape(&off.signatory.name),
                off.signatory
                    .title
                    .as_deref()
                    .map(|t| format!(", {}", html_escape(t)))
                    .unwrap_or_default(),
                off.signatory
                    .company
                    .as_deref()
                    .map(|c| format!(", {}", html_escape(c)))
                    .unwrap_or_default(),
                html_escape(&off.result.wording()),
                signed
            ));
            if let SignOffResult::ApprovedWithExceptions { exceptions } = &off.result {
                if !exceptions.is_empty() {
                    s.push_str("<h3>Accepted exceptions</h3>\n<ul>\n");
                    for e in exceptions {
                        s.push_str(&format!("<li>{}</li>\n", html_escape(e)));
                    }
                    s.push_str("</ul>\n");
                }
            }
            if let Some(notes) = &off.notes {
                s.push_str(&format!("<p class=\"notes\">{}</p>\n", html_escape(notes)));
            }
            s.push_str("</section>\n");
        }

        s.push_str("</body>\n</html>\n");
        s
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

/// Escape text for safe interpolation into HTML content and attribute
/// positions.
fn html_escape(value: &str) -> String {
    let mut out = String::with_capacity(value.len());
    for ch in value.chars() {
        match ch {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&#39;"),
            _ => out.push(ch),
        }
    }
    out
}

/// A labelled status badge: word + symbol, never colour alone (§28).
fn status_badge(status: TestStatus) -> String {
    let (symbol, class, label) = match status {
        TestStatus::Pass => ("&#10003;", "pass", "pass"),
        TestStatus::Fail => ("&#10007;", "fail", "fail"),
        TestStatus::Warning => ("&#9888;", "warning", "warning"),
        TestStatus::Blocked => ("&#8856;", "blocked", "blocked"),
        TestStatus::Skipped => ("&#9675;", "skipped", "skipped"),
        TestStatus::Manual => ("&#9998;", "manual", "manual"),
        TestStatus::Inconclusive => ("&#63;", "inconclusive", "inconclusive"),
    };
    format!("<span class=\"status status-{class}\" aria-label=\"{label}\">{symbol} {label}</span>")
}

/// Inline stylesheet for the HTML report.
const STD_CSS: &str = r#"
:root { color-scheme: light; }
body { font-family: "Segoe UI", system-ui, sans-serif; margin: 2rem auto; max-width: 60rem;
       padding: 0 1rem; color: #1a1a2e; line-height: 1.45; }
h1 { font-size: 1.5rem; border-bottom: 3px solid #1a1a2e; padding-bottom: .4rem; }
h2 { font-size: 1.15rem; margin-top: 2rem; }
dl.cover { display: grid; grid-template-columns: max-content 1fr; gap: .15rem 1rem; }
dl.cover dt { font-weight: 600; color: #444; }
dl.cover dd { margin: 0; }
table { border-collapse: collapse; width: 100%; margin-top: .5rem; }
th, td { border: 1px solid #c9c9d4; padding: .35rem .55rem; text-align: left;
         vertical-align: top; font-size: .92rem; }
thead th { background: #eceef4; }
code { font-family: ui-monospace, Consolas, monospace; font-size: .9em;
       background: #f2f2f6; padding: 0 .25rem; border-radius: 3px; }
.status { display: inline-block; padding: .05rem .5rem; border-radius: 999px;
          font-weight: 600; white-space: nowrap; border: 1px solid; font-size: .85rem; }
.status-pass { background: #e3f6e8; color: #116629; border-color: #74c69d; }
.status-fail { background: #fdeaea; color: #a11212; border-color: #e08a8a; }
.status-warning { background: #fff4dc; color: #7a4d00; border-color: #d9a441; }
.status-blocked { background: #e8ecf7; color: #2d3f76; border-color: #93a4d6; }
.status-skipped { background: #f0f0f2; color: #555; border-color: #bcbccb; }
.status-manual { background: #f3e9fb; color: #5d2576; border-color: #bb8fd6; }
.status-inconclusive { background: #fff0e4; color: #8a3d00; border-color: #dca678; }
.verdict { font-weight: 700; }
.notes { color: #444; font-style: italic; }
"#;

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

    #[test]
    fn html_is_standalone_escapes_and_labels_statuses() {
        let mut report = Report::new("Board <room>", "prj-1");
        let mut failing = result("edid.<check>", TestStatus::Fail);
        failing.messages.push("expected & got <nothing>".to_owned());
        report.results = vec![result("power", TestStatus::Pass), failing];
        report.refresh_summary();
        let html = report.render(ReportFormat::Html).unwrap();

        assert!(html.starts_with("<!DOCTYPE html>"));
        assert!(html.contains("Board &lt;room&gt;"));
        assert!(html.contains("<code>edid.&lt;check&gt;</code>"));
        assert!(html.contains("expected &amp; got &lt;nothing&gt;"));
        // Statuses are word-labelled, never colour alone.
        assert!(html.contains("&#10003; pass"));
        assert!(html.contains("&#10007; fail"));
        assert!(html.contains("aria-label=\"fail\""));
    }
}
