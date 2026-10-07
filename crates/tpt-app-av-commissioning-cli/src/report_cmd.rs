//! The `report` command (§34): render a stored run as a report.
//!
//! `--project` points at a project directory (containing `project.sqlite`);
//! the latest run is reported unless `--run <id>` names one. The rendered
//! report goes to stdout, so it composes with shell redirection.

use std::path::Path;

use tpt_app_av_commissioning_core::ProjectStore;
use tpt_app_av_commissioning_report::{Report, ReportFormat};

const REPORT_USAGE: &str = "\
report --project <dir> [--format <markdown|html|csv|json>] [--run <id>]

  Renders a stored test run. `--project` is a project directory (containing
  `project.sqlite`); the latest run is reported unless `--run <id>` names
  one. The report is written to stdout.
";

pub fn run(args: &[String]) -> Result<bool, String> {
    let mut project: Option<String> = None;
    let mut format = ReportFormat::Markdown;
    let mut run: Option<String> = None;

    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "--project" => {
                i += 1;
                project = Some(value(args, i, "--project")?);
            }
            "--format" => {
                i += 1;
                format = parse_format(value(args, i, "--format")?)?;
            }
            "--run" => {
                i += 1;
                run = Some(value(args, i, "--run")?);
            }
            "--help" | "-h" => {
                println!("{REPORT_USAGE}");
                return Ok(true);
            }
            other => return Err(format!("unexpected argument `{other}` for `report`")),
        }
        i += 1;
    }
    let project = project.ok_or("`report` requires --project <dir>")?;

    let path = Path::new(&project);
    if !path.is_dir() {
        return Err(format!(
            "`{project}` is not a project directory (expected a directory containing project.sqlite)"
        ));
    }
    let store = ProjectStore::open(path).map_err(|e| format!("could not open project: {e}"))?;

    let runs = store.list_runs().map_err(|e| format!("could not list runs: {e}"))?;
    let run_id = match run {
        Some(id) => {
            if !runs.contains(&id) {
                return Err(format!("run `{id}` is not recorded in this project"));
            }
            id
        }
        None => runs.first().cloned().ok_or_else(|| {
            "this project has no recorded test runs; run `test` first".to_owned()
        })?,
    };

    let meta = store
        .project_meta()
        .map_err(|e| format!("could not read project meta: {e}"))?
        .ok_or("project meta missing")?;
    let results = store
        .results_for_run(&run_id)
        .map_err(|e| format!("could not read results: {e}"))?;

    let mut report = Report::new(meta.name.clone(), meta.id.as_str());
    report.client = meta.client.clone();
    report.site = meta.site.clone();
    report.results = results;
    report.refresh_summary();

    let rendered = report.render(format).map_err(|e| format!("{e}"))?;
    println!("{rendered}");
    Ok(true)
}

fn parse_format(value: String) -> Result<ReportFormat, String> {
    match value.as_str() {
        "markdown" | "md" => Ok(ReportFormat::Markdown),
        "html" => Ok(ReportFormat::Html),
        "csv" => Ok(ReportFormat::Csv),
        "json" => Ok(ReportFormat::Json),
        other => Err(format!(
            "unknown format `{other}` (expected markdown, html, csv, or json)"
        )),
    }
}

fn value(args: &[String], i: usize, flag: &str) -> Result<String, String> {
    args.get(i)
        .cloned()
        .ok_or_else(|| format!("`{flag}` requires a value"))
}
