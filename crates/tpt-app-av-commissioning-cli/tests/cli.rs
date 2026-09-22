//! Integration tests for the `validate` CLI subcommand (§34).
//!
//! Exit code contract: 0 = all valid, 1 = validation failure,
//! 2 = usage error / unreadable input.

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
