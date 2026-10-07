//! TPT AV Commissioning — command-line interface (§34).
//!
//! Shares the core engine with the desktop UI. Automation-friendly: exits 0
//! when everything validates (or no test failed), 1 on failures, 2 on usage
//! errors.
//!
//! Licensed under either of MIT OR Apache-2.0, at your option.

#![forbid(unsafe_code)]

mod report_cmd;
mod test_cmd;

use std::fs;
use std::path::Path;
use std::process::ExitCode;

use tpt_app_av_commissioning_core::Manifest;
use tpt_app_av_commissioning_test::TestProcedure;

const USAGE: &str = "\
TPT AV Commissioning — executable commissioning for professional AV systems.

Usage:
  tpt-app-av-commissioning-cli validate --project <file> [--suite <path>]
  tpt-app-av-commissioning-cli test --project <file> --room <name> [--profiles <path>] [--dry-run]
  tpt-app-av-commissioning-cli report --project <dir> [--format <markdown|html|csv|json>] [--run <id>]
  tpt-app-av-commissioning-cli --version
  tpt-app-av-commissioning-cli --help

Commands:
  validate   Validate a project manifest and/or test procedure suite file(s).
  test       Run commissioning tests for the devices in one room.
  report     Render a stored test run from a project directory.

Options:
  --project <path>   For `validate`/`test`: a project manifest (YAML). For
                     `report`: a project directory (with project.sqlite).
  --suite <path>     Path to a test procedure YAML file, or a directory of
                     such files. Each document is validated against the test
                     procedure DSL.
  --version          Print the version.
  --help             Print this help.

Exit codes:
  0  everything validated / no test failed
  1  one or more files failed validation / a test failed
  2  usage error or an input could not be read
";

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    match run(&args) {
        Ok(true) => ExitCode::SUCCESS,
        Ok(false) => ExitCode::from(1),
        Err(message) => {
            eprintln!("error: {message}");
            eprintln!();
            eprint!("{USAGE}");
            ExitCode::from(2)
        }
    }
}

fn run(args: &[String]) -> Result<bool, String> {
    if args.is_empty() {
        return Err("no command given (implemented: validate, test, report)".to_owned());
    }
    match args[0].as_str() {
        "--help" | "-h" => {
            print!("{USAGE}");
            Ok(true)
        }
        "--version" | "-V" => {
            println!("tpt-app-av-commissioning {}", env!("CARGO_PKG_VERSION"));
            Ok(true)
        }
        "validate" => run_validate(&args[1..]),
        "test" => test_cmd::run(&args[1..]),
        "report" => report_cmd::run(&args[1..]),
        other => Err(format!(
            "unknown command `{other}` (implemented: validate, test, report)"
        )),
    }
}

fn run_validate(args: &[String]) -> Result<bool, String> {
    let mut project: Option<String> = None;
    let mut suite: Option<String> = None;

    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "--project" => {
                i += 1;
                project = Some(take_value(args, i, "--project")?);
            }
            "--suite" => {
                i += 1;
                suite = Some(take_value(args, i, "--suite")?);
            }
            "--help" | "-h" => {
                print!("{USAGE}");
                return Ok(true);
            }
            other => return Err(format!("unexpected argument `{other}` for `validate`")),
        }
        i += 1;
    }

    if project.is_none() && suite.is_none() {
        return Err(
            "`validate` requires at least one of --project <file> or --suite <path>".to_owned(),
        );
    }

    let mut all_ok = true;

    if let Some(path) = &project {
        all_ok &= validate_project_file(path)?;
    }

    if let Some(path) = &suite {
        if Path::new(path).is_dir() {
            let mut files = collect_suite_yaml(path)?;
            files.sort();
            for file in files {
                let ok = validate_suite_file(&file)?;
                if !ok {
                    all_ok = false;
                }
            }
        } else {
            all_ok &= validate_suite_file(path)?;
        }
    }

    Ok(all_ok)
}

fn take_value(args: &[String], i: usize, flag: &str) -> Result<String, String> {
    args.get(i)
        .cloned()
        .ok_or_else(|| format!("`{flag}` requires a value"))
}

fn read_input(path: &str) -> Result<String, String> {
    fs::read_to_string(path).map_err(|e| format!("could not read `{path}`: {e}"))
}

fn validate_project_file(path: &str) -> Result<bool, String> {
    let text = read_input(path)?;

    if Manifest::contains_executable_content(&text) {
        eprintln!("{path}: rejected: project manifest contains executable content");
        return Ok(false);
    }

    match Manifest::parse(&text) {
        Ok(manifest) => {
            let n_devices = manifest.devices.len();
            println!(
                "OK  {path}: valid project manifest `{}` (schema {}, {} devices)",
                manifest.project.name, manifest.project.schema_version, n_devices
            );
            Ok(true)
        }
        Err(error) => {
            eprintln!("{path}: invalid: {error}");
            Ok(false)
        }
    }
}

fn validate_suite_file(path: &str) -> Result<bool, String> {
    let text = read_input(path)?;
    match TestProcedure::from_yaml_str(&text) {
        Ok(procedure) => {
            println!(
                "OK  {path}: valid test procedure `{}` ({} steps, {})",
                procedure.id,
                procedure.steps.len(),
                procedure.mode.as_str()
            );
            Ok(true)
        }
        Err(error) => {
            eprintln!("{path}: invalid: {error}");
            Ok(false)
        }
    }
}

fn collect_suite_yaml(path: &str) -> Result<Vec<String>, String> {
    let mut out = Vec::new();
    for entry in
        fs::read_dir(path).map_err(|e| format!("could not read suite directory `{path}`: {e}"))?
    {
        let entry = entry.map_err(|e| format!("could not read suite directory entry: {e}"))?;
        let name = entry.file_name().to_string_lossy().into_owned();
        if name.ends_with(".yaml") || name.ends_with(".yml") {
            out.push(entry.path().to_string_lossy().into_owned());
        }
    }
    Ok(out)
}
