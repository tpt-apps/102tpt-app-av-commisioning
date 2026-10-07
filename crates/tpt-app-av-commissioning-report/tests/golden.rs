//! Golden reports (§46.4): project + suite results + sign-off render to
//! stable expected documents.
//!
//! The fixture pins every timestamp and id, so the rendered bytes are
//! deterministic. Run with `UPDATE_GOLDEN=1` to regenerate the checked-in
//! files when the renderers change deliberately:
//!
//! ```text
//! UPDATE_GOLDEN=1 cargo test -p tpt-app-av-commissioning-report --test golden
//! ```

use chrono::{TimeZone, Utc};

use tpt_app_av_commissioning_model::{
    DefectId, EvidenceId, EvidenceKind, EvidenceRef, Measurement, ProjectId,
};
use tpt_app_av_commissioning_report::report::DefectLine;
use tpt_app_av_commissioning_report::{Report, ReportFormat};
use tpt_app_av_commissioning_test::{
    ChecklistVerdict, ExecutionMode, ManualChecklist, PendingConfirmation, TestId, TestResult,
    TestStatus,
};

/// The shared deterministic fixture.
fn fixture() -> Report {
    let at = |secs: i64| Utc.timestamp_opt(1_768_500_000 + secs, 0).unwrap();
    let project = ProjectId::new("prj-boardroom-01");

    let mut pass = TestResult::new(
        TestId::new("display.identity"),
        TestStatus::Pass,
        ExecutionMode::Automated,
    );
    pass.started_at = at(0);
    pass.completed_at = at(2);
    pass.messages
        .push("identity matches: ACME Beamer 9000".to_owned());
    pass.measurements.push(Measurement::new(
        "response_time_ms",
        tpt_app_av_commissioning_model::MeasurementValue::Integer(18),
    ));
    pass.evidence.push(EvidenceRef {
        id: EvidenceId::new("ev-1"),
        project: project.clone(),
        kind: EvidenceKind::Screenshot,
        relative_path: "assets/evidence/run-7/0000.png".to_owned(),
        description: Some("test pattern captured".to_owned()),
        captured_at: at(1),
        sha256: Some("9f2c0b".to_owned()),
    });

    let mut failing = TestResult::new(
        TestId::new("display.signal"),
        TestStatus::Fail,
        ExecutionMode::Automated,
    );
    failing.started_at = at(3);
    failing.completed_at = at(5);
    failing.messages.push("no signal on hdmi2".to_owned());
    failing.messages.push("resolution read 0x0".to_owned());
    failing.error = Some("device reported <no-signal> & 0x0".to_owned());

    let mut checklist = ManualChecklist::new(
        "Projector image quality",
        ["Focus uniform across the screen", "Geometry undistorted"],
    );
    checklist
        .confirm_item("item-1", ChecklistVerdict::Pass, None, None)
        .unwrap();
    let mut manual = TestResult::new(
        TestId::new("projector.image-quality"),
        TestStatus::Manual,
        ExecutionMode::Manual,
    );
    manual.started_at = at(6);
    manual.completed_at = at(60);
    manual.checklist = Some(checklist);

    let mut semi = TestResult::new(
        TestId::new("pa.speech-level"),
        TestStatus::Manual,
        ExecutionMode::SemiAutomated,
    );
    semi.started_at = at(61);
    semi.completed_at = at(70);
    semi.pending = Some(PendingConfirmation {
        software_status: Some(TestStatus::Warning),
        prompt: "Confirm the measured level".to_owned(),
    });
    semi.messages.push("measured level -18 dBFS".to_owned());

    let mut report = Report::new("Boardroom Fitout", "prj-boardroom-01");
    report.client = Some("Meridian Business <Group>".to_owned());
    report.site = Some("Level 12, 88 Collins St".to_owned());
    report.generated_at = at(120);
    report.room_count = 2;
    report.device_count = 7;
    report.connection_count = 11;
    report.results = vec![pass, failing, manual, semi];
    report.defects = vec![DefectLine {
        id: DefectId::new("DEF-001"),
        title: "Display does not lock signal on input 3".to_owned(),
        status: "investigating".to_owned(),
    }];
    report.refresh_summary();

    let mut signatory = tpt_app_av_commissioning_report::Signatory::new("A. Engineer");
    signatory.title = Some("Commissioning Engineer".to_owned());
    signatory.company = Some("TPT Solutions".to_owned());
    signatory.signed_at = Some(at(130));
    let mut off = tpt_app_av_commissioning_report::SignOff::sign(
        signatory,
        tpt_app_av_commissioning_report::SignOffResult::ApprovedWithExceptions {
            exceptions: vec!["DEF-001 accepted: input 3 unused in final layout".to_owned()],
        },
    );
    off.notes = Some("Handover walkthrough attended by the client.".to_owned());
    report.sign_off = Some(off);
    report
}

fn golden_path(name: &str) -> std::path::PathBuf {
    std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/golden")
        .join(name)
}

fn check_golden(name: &str, rendered: &str) {
    let path = golden_path(name);
    if std::env::var("UPDATE_GOLDEN").is_ok() {
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(&path, rendered).unwrap();
        eprintln!("updated golden {}", path.display());
        return;
    }
    let expected = std::fs::read_to_string(&path).unwrap_or_else(|e| {
        panic!(
            "cannot read golden {}: {e}; run with UPDATE_GOLDEN=1 to regenerate",
            path.display()
        )
    });
    if expected != rendered {
        // Show the first divergence for quick diagnosis.
        for (line, (a, b)) in (1..).zip(expected.lines().zip(rendered.lines())) {
            if a != b {
                panic!(
                    "golden {name} diverges at line {line}:\n  expected: {a}\n  actual:   {b}\n\
                     re-run with UPDATE_GOLDEN=1 if this change is deliberate"
                );
            }
        }
        panic!(
            "golden {name} differs in length (expected {} lines, got {}); \
             re-run with UPDATE_GOLDEN=1 if this change is deliberate",
            expected.lines().count(),
            rendered.lines().count()
        );
    }
}

#[test]
fn markdown_report_matches_golden() {
    let rendered = fixture().render(ReportFormat::Markdown).unwrap();
    check_golden("report.md", &rendered);
}

#[test]
fn html_report_matches_golden() {
    let rendered = fixture().render(ReportFormat::Html).unwrap();
    check_golden("report.html", &rendered);
}

#[test]
fn csv_report_matches_golden() {
    let rendered = fixture().render(ReportFormat::Csv).unwrap();
    check_golden("report.csv", &rendered);
}

#[test]
fn json_report_matches_golden() {
    let rendered = fixture().render(ReportFormat::Json).unwrap();
    check_golden("report.json", &rendered);
}
