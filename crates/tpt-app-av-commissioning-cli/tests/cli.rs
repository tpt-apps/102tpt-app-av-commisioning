//! Integration tests for the CLI subcommands (§34).
//!
//! Exit code contract: 0 = all valid / no test failed, 1 = validation
//! failure / a test failed, 2 = usage error / unreadable input.

use assert_cmd::prelude::*;
use predicates::prelude::*;
use std::process::Command;

fn fixture(name: &str) -> String {
    format!(
        "{}/../../tests/fixtures/{}",
        env!("CARGO_MANIFEST_DIR"),
        name
    )
}

fn bin() -> Command {
    Command::cargo_bin("tpt-app-av-commissioning-cli").unwrap()
}

#[test]
fn version_prints_ok() {
    bin()
        .arg("--version")
        .assert()
        .success()
        .stdout(predicate::str::contains("tpt-app-av-commissioning"));
}

#[test]
fn no_command_is_usage_error() {
    bin()
        .assert()
        .failure()
        .code(2)
        .stderr(predicate::str::contains("no command given"));
}

#[test]
fn unknown_subcommand_is_usage_error() {
    bin()
        .arg("deploy")
        .assert()
        .failure()
        .code(2)
        .stderr(predicate::str::contains("unknown command `deploy`"));
}

#[test]
fn validate_requires_an_input() {
    bin()
        .arg("validate")
        .assert()
        .failure()
        .code(2)
        .stderr(predicate::str::contains(
            "requires at least one of --project <file> or --suite <path>",
        ));
}

#[test]
fn validate_project_ok() {
    let project = fixture("valid-project.yaml");
    bin()
        .args(["validate", "--project"])
        .arg(&project)
        .assert()
        .success()
        .stdout(predicate::str::contains(
            "valid project manifest `Client Boardroom`",
        ))
        .stdout(predicate::str::contains("2 devices"));
}

#[test]
fn validate_suite_ok() {
    let suite = fixture("valid-suite.yaml");
    bin()
        .args(["validate", "--suite"])
        .arg(&suite)
        .assert()
        .success()
        .stdout(predicate::str::contains(
            "valid test procedure `display.identity`",
        ));
}

#[test]
fn validate_suite_invalid_fails() {
    let suite = fixture("invalid-suite.yaml");
    bin()
        .args(["validate", "--suite"])
        .arg(&suite)
        .assert()
        .failure()
        .code(1)
        .stderr(predicate::str::contains("invalid:"))
        .stderr(predicate::str::contains("assert must specify `equals`"));
}

#[test]
fn validate_suite_executable_content_rejected() {
    let suite = fixture("executable-suite.yaml");
    bin()
        .args(["validate", "--suite"])
        .arg(&suite)
        .assert()
        .failure()
        .code(1)
        .stderr(predicate::str::contains("executable content"));
}

#[test]
fn validate_suite_directory_processes_all_yaml() {
    let dir = fixture("."); // contains both valid and invalid suites
    let cmd = bin()
        .args(["validate", "--suite"])
        .arg(&dir)
        .assert()
        .failure()
        .code(1)
        .stdout(predicate::str::contains(
            "valid test procedure `display.identity`",
        ));
    #[allow(unused)]
    let _ = cmd;
}

#[test]
fn validate_project_and_suite_together() {
    let project = fixture("valid-project.yaml");
    let suite = fixture("valid-suite.yaml");
    bin()
        .args(["validate", "--project"])
        .arg(&project)
        .arg("--suite")
        .arg(&suite)
        .assert()
        .success()
        .stdout(predicate::str::contains("valid project manifest"))
        .stdout(predicate::str::contains("valid test procedure"));
}

#[test]
fn missing_project_value_is_usage_error() {
    bin()
        .args(["validate", "--project"])
        .assert()
        .failure()
        .code(2)
        .stderr(predicate::str::contains("`--project` requires a value"));
}

#[test]
fn unreadable_project_is_usage_error() {
    bin()
        .args(["validate", "--project", "does-not-exist.yaml"])
        .assert()
        .failure()
        .code(2)
        .stderr(predicate::str::contains("could not read"));
}

// -- test ---------------------------------------------------------------

#[test]
fn test_dry_run_schedules_without_touching_devices() {
    let project = fixture("valid-project.yaml");
    bin()
        .args([
            "test",
            "--project",
            &project,
            "--room",
            "Boardroom",
            "--dry-run",
        ])
        .assert()
        .success()
        .stdout(predicate::str::contains(
            "room `Boardroom`: 2 device(s), run (dry run)",
        ))
        .stdout(predicate::str::contains("skipped  connectivity.matrix-01"))
        .stdout(predicate::str::contains("2 skipped"));
}

#[test]
fn test_profiles_bind_in_dry_run() {
    let project = fixture("valid-project.yaml");
    let profiles = fixture("osc-projector-profile.yaml");
    bin()
        .args([
            "test",
            "--project",
            &project,
            "--room",
            "room-01",
            "--profiles",
            &profiles,
            "--dry-run",
        ])
        .assert()
        .success()
        .stdout(predicate::str::contains(
            "room `room-01`: 2 device(s), run (dry run)",
        ));
}

#[test]
fn test_unknown_room_is_usage_error() {
    let project = fixture("valid-project.yaml");
    bin()
        .args(["test", "--project", &project, "--room", "Nowhere"])
        .assert()
        .failure()
        .code(2)
        .stderr(predicate::str::contains("room `Nowhere` does not exist"));
}

#[test]
fn test_requires_room() {
    let project = fixture("valid-project.yaml");
    bin()
        .args(["test", "--project", &project])
        .assert()
        .failure()
        .code(2)
        .stderr(predicate::str::contains("`test` requires --room <name>"));
}

#[test]
fn test_unbound_devices_skip_and_unreachable_probe_fails() {
    // Real run: the matrix (bare IP, no profile) cannot bind and is
    // reported as skipped; the projector's host:port gets a bounded TCP
    // probe against an address that is not listening — a genuine failure,
    // and the run exits 1.
    let project = fixture("valid-project.yaml");
    bin()
        .args(["test", "--project", &project, "--room", "Boardroom"])
        .assert()
        .failure()
        .code(1)
        .stdout(predicate::str::contains(
            "skipped  connectivity.matrix-01",
        ))
        .stdout(predicate::str::contains(
            "fail     connectivity.projector-01",
        ));
}

// -- report -------------------------------------------------------------

#[test]
fn report_requires_a_project_directory() {
    let project = fixture("valid-project.yaml");
    bin()
        .args(["report", "--project", &project])
        .assert()
        .failure()
        .code(2)
        .stderr(predicate::str::contains(
            "is not a project directory",
        ));
}

#[test]
fn report_renders_the_latest_stored_run() {
    use tpt_app_av_commissioning_core::{ProjectMeta, ProjectStore};
    use tpt_app_av_commissioning_model::ProjectId;
    use tpt_app_av_commissioning_test::{ExecutionMode, TestId, TestResult, TestStatus};

    let dir = std::env::temp_dir().join(format!(
        "tpt-cli-report-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_millis()
    ));
    let mut store = ProjectStore::create(
        &dir,
        &ProjectMeta {
            id: ProjectId::new("prj-1"),
            name: "CLI Report Project".to_owned(),
            client: Some("Example Corp".to_owned()),
            site: None,
        },
    )
    .unwrap();
    let project = ProjectId::new("prj-1");
    let mut pass =
        TestResult::new(TestId::new("display.identity"), TestStatus::Pass, ExecutionMode::Automated);
    pass.messages.push("identity matches".to_owned());
    let fail = TestResult::new(
        TestId::new("display.signal"),
        TestStatus::Fail,
        ExecutionMode::Automated,
    );
    store.save_result("run-1", &project, &pass).unwrap();
    store.save_result("run-1", &project, &fail).unwrap();
    drop(store);

    bin()
        .args([
            "report",
            "--project",
            dir.to_str().unwrap(),
            "--format",
            "markdown",
        ])
        .assert()
        .success()
        .stdout(predicate::str::contains("# TPT AV Commissioning Report"))
        .stdout(predicate::str::contains("CLI Report Project"))
        .stdout(predicate::str::contains("1 fail"));

    // A named run works; an unknown run is a usage error.
    bin()
        .args([
            "report",
            "--project",
            dir.to_str().unwrap(),
            "--run",
            "missing",
        ])
        .assert()
        .failure()
        .code(2)
        .stderr(predicate::str::contains("not recorded"));

    std::fs::remove_dir_all(&dir).unwrap();
}
