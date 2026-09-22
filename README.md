# TPT AV Commissioning

Professional desktop application for commissioning, testing, diagnosing, documenting, and handing over audiovisual installations.

Product family: **TPT Apps** · Publisher: **TPT Solutions** · License: **MIT OR Apache-2.0**

## What it does

Commission an AV system once, prove that it works, and retain a repeatable record that it still works later.

TPT AV Commissioning turns a collection of AV devices and connections into a structured, testable system:

- a project/system inventory with rooms, devices, and endpoints
- a connection and signal-path model
- reusable commissioning test suites (automated, semi-automated, and manual)
- preflight, signal-path, and regression testing
- evidence capture, defect tracking, baselines, and configuration drift detection
- professional commissioning and handover reports with engineer sign-off

The product is intentionally narrower than a general AV control platform. Its job is to test and prove the installation — not to operate the venue during normal use.

## Repository layout

```
├── Cargo.toml                      # Cargo workspace (resolver = "2")
├── crates/                         # Application crates
│   ├── tpt-app-av-commissioning-core/
│   ├── tpt-app-av-commissioning-model/
│   ├── tpt-app-av-commissioning-device/
│   ├── tpt-app-av-commissioning-driver/
│   ├── tpt-app-av-commissioning-test/
│   ├── tpt-app-av-commissioning-runner/
│   ├── tpt-app-av-commissioning-report/
│   ├── tpt-app-av-commissioning-cli/
│   ├── tpt-app-av-commissioning-tauri/
│   └── tpt-app-av-commissioning-testkit/
├── drivers/                        # Protocol + device drivers
│   ├── generic/  ├── osc/  ├── midi/  ├── network/  └── examples/
├── test-suites/                    # Test suite definitions
│   ├── generic/  └── examples/
├── tests/                          # Integration, fixtures, golden, mock
├── docs/                           # Architecture + domain documentation
└── spec.txt                        # Product specification (§ references)
```

The crate count may be reduced as implementation proceeds; domain separation matters more than the exact number of crates.

## Getting started

Prerequisites: a recent stable Rust toolchain (edition 2021, resolver 2).

```sh
cargo build --workspace
cargo test --workspace
```

## Documentation

- `spec.txt` — the full product specification (primary reference)
- `docs/architecture.md` — architectural overview
- `docs/domain-model.md` — core domain model
- `docs/device-drivers.md` — driver SDK and device profile format
- `docs/test-model.md` — test model, runner, and DSL
- `docs/report-format.md` — reporting and sign-off
- `docs/project-format.md` — project persistence and manifest format
- `docs/security.md` — network safety and security posture

## Development workflow

- `cargo build` / `cargo test` / `cargo clippy` / `cargo fmt`
- `cargo deny check` for dependency licence and advisory checks
- Contribution checklist lives in `todo.md`, organised by phase

## Status

Early development. See `todo.md` for the phased checklist and `CHANGELOG.md` for current progress.