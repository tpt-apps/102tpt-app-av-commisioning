# Changelog

All notable changes to TPT AV Commissioning are documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

### Added

- Phase 0 — Project & repository setup
  - Cargo workspace with `resolver = "2"`, shared workspace metadata and dependencies
  - Dual licensing (`MIT OR Apache-2.0`) with `LICENSE-MIT` and `LICENSE-APACHE`
  - `deny.toml` license/advisory/ban policy for `cargo-deny`
  - Root `README.md` and this `CHANGELOG.md`
  - `docs/` — architecture, domain model, device drivers, test model, report format, project format, security
  - GitHub Actions CI (build, test, fmt, clippy, cargo-deny)
  - Scaffolded application crates under `crates/`
  - Scaffolded `drivers/`, `test-suites/`, and `tests/` directories

- Phase 9 — Test types (§13)
  - Expectation engine (`check`), concrete connectivity / identity / power / input-output / video / audio / control / synchronisation / network tests
  - `CommissioningTest::kind()` and DSL `kind:` pin/inference
  - Power-cycle tests require explicit confirmation; network tests are bounded and non-intrusive

- Phase 5 — OSC driver (`drivers/osc`)
  - Config-driven `DeviceDriver` over UDP using the `tpt-av-control-osc` codec; explicit unicast targets only, bounded reads, wildcard-free addresses

- Phase 5 — Device profiles
  - New `tpt-app-av-commissioning-profile` crate: versioned YAML profile format with validation, executable-content guard, manufacturer/model matching
  - `OscDriverConfig::from_profile`; sample `drivers/examples/osc-projector.yaml`

### Notes

- TPT ecosystem crates are pinned as git dependencies by exact commit (see `[workspace.dependencies]`); bump revs deliberately.
- GitHub repository `tpt-solutions/tpt-app-av-commissioning` is not yet created/connected remotely.