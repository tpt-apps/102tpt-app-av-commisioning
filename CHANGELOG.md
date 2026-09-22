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

### Notes

- External dependency `tpt-kinetix` is not yet vendored locally; integration will be revisited once the repository exists/clones (§5.6, §55).
- GitHub repository `tpt-solutions/tpt-app-av-commissioning` is not yet created/connected remotely.