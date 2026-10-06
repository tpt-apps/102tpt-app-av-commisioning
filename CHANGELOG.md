# Changelog

All notable changes to TPT AV Commissioning are documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

### Added

- Phase 12 — Audio test-tone measurement (§13.6, §19) on the TPT ecosystem
  - `test::audio`: tone generation (`tpt-dsp-audio` oscillator), 16-bit PCM
    WAV I/O (`tpt-av-cadence-wav`, stereo downmixed on read), and buffer
    analysis (`tpt-dsp-core` Hann FFT peak with parabolic interpolation,
    `tpt-dsp-analysis` RMS/ZCR): level, frequency, ZCR cross-check,
    clipped-sample count, silence detection, evaluated against
    `ToneExpectations` — inconclusive without expectations
  - Workspace now compiles against the `tpt-dsp` and `tpt-cadence` repos on
    GitHub (pinned revs unchanged); `num-complex` 0.4 joins the workspace

- Phase 18/19 — Execution policy and restore-state (§36, §37)
  - `ExecutionPolicy` project setting (power cycles denied by default); the
    runner blocks (never fails) tests whose `MutationKind` the policy does
    not permit, before dispatch; command tests classify their own mutation
    kind (a power cycle makes a test disruptive), the DSL classifies from
    command steps
  - Restore-state mechanism: `TestRequirements::restores_state`,
    `DeviceDriver::restore_state` (default `UnsupportedOperation`) with a
    `CanRestoreState` capability, pre-test capture and best-effort
    restoration in `CommandTest::restores_state()`, and the explicit §37
    report — "Test passed, but device state restoration failed." with the
    outcome downgraded to `Warning` — enforced by the runner even when a
    test implementation forgets
  - The mock device restores captured state exactly, for hardware-style
    tests of the whole flow

- Phase 14/15 — Baselines, regression and drift (§21–22)
  - `core::baseline`: fingerprinted `Baseline` snapshots (device identity
    incl. tracked configuration attributes, signal routes, per-test
    statuses), aggregate headline, `RunComparison` that highlights only
    meaningful outcome changes, and `detect_drift` for firmware / address /
    replacement (serial) / attribute / route / added-removed drift
  - Baseline snapshots persist as payload next to their fingerprint;
    `(project, label)` stays unique so existing snapshots are preserved

- Phase 16 — Defect tracking (§23)
  - `Defect`, `Severity` (Critical/Major/Minor/Cosmetic), `DefectStatus`
    (Open … Verified/Accepted/Deferred, terminal states explicit)
  - `defect_from_result` converts failed test results into defects with
    evidence and messages carried over; typed store round-trip and status
    updates

- Phase 17 — Audit log (§33)
  - `AuditEvent` / `AuditEventType` / `Actor` in the model, persisted
    chronologically per project (`append_audit_event` / `list_audit_events`)

- Phase 10 — Manual & semi-automated workflow (§15, §4.5)
  - `ManualChecklist` model: Pass / Fail / N/A verdicts per item with note and
    evidence; a failed item fails the checklist; confirming an incomplete
    checklist is an error
  - `ManualTest` and a `checklist()` hook on `CommissioningTest`; the runner
    executes no software and touches no device for manual tests — the result
    stays `Manual` (carrying the checklist) until the engineer confirms it
  - Semi-automated path: the runner marks finished software runs as pending
    confirmation with the measured status recorded; `TestResult::confirm`
    approves (measured status stands) or rejects (test fails); automated
    results are final and refuse confirmation
  - Mode-aware DSL authoring: `checklist:` entries for `manual` and
    `semi_automated` tests (composition per mode is validated); runnable
    manual tests from procedures via `TestProcedure::manual_test`
  - Example suites `test-suites/generic/projector-image-quality.yaml`
    (manual) and `test-suites/generic/audio-level-check.yaml` (semi-automated)

- Phase 7 — Runner executor wired and covered (§17)
  - The executor module is now compiled and exported (`TestExecutor`,
    `RunObserver`, `CancelToken`, `RunOutcome`); 14 executor tests cover
    dependency blocking, parallel and serial execution, timeouts, retries,
    cancellation (before and mid-run), rate limiting, device-lock
    serialisation, and dry runs
  - Live `test started` observer events at dispatch; synthesized
    blocked/skipped results are reported to observers so persisted runs are
    complete

### Changed

- `DeviceCapabilities` gained `can_restore_state` (in `full()`, not in
  `read_only()`); profile-driven drivers declare it `false`
- `TestRequirements` gained serde-defaulted `mutation` and `restores_state`
  fields; `TestResult` gained a serde-defaulted `restoration_failed` flag
- `MutationKind` lives in the model crate (re-exported by the test crate)
  so the execution policy can gate on it
- `SignalType` gained `as_str()` (matching `Transport`)
- Store: defects and baselines carry a `payload` column (migrated on open
  via `PRAGMA table_info` probe); `save_baseline`/`save_defect` take typed
  records, `list_defects` returns typed `Defect`s, `StoredDefect` removed
- `CommissioningTest` requires `Send + Sync` (tests are dispatched onto
  worker threads)
- `TestResult` gained optional serde-defaulted `checklist` and `pending`
  fields; older persisted results still deserialize

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

- Phase 5 — TCP driver (`drivers/generic`)
  - Line-based text-protocol driver configured from `type: tcp` profiles; reply parsers, ack lines, prefix stripping, injection-safe placeholder substitution
  - Shared network safety helpers (`driver::net`) now used by the OSC and TCP drivers

- Phase 5 — Generic drivers
  - UDP, serial, WebSocket, HTTP and read-only SNMP drivers (`drivers/generic`) and a MIDI driver (`drivers/midi`)
  - Shared text-protocol core with injection-safe substitution, bounded reads and typed reply parsing
  - Profile format: `terminator: none`, WebSocket `path`, serial line settings, HTTP `body`, JSON Pointer `extract`, MIDI and SNMP grammars

- Phase 6 — Device discovery (`drivers/network`)
  - TCP probe, mDNS, SSDP, SNMP, OSC, MIDI, serial and audio discovery behind one explicit, validated `DiscoveryConfig`
  - Host-count/port/timeout caps, private-range default, mandatory interface for multicast, cancellation, per-mechanism error reporting

### Notes

- TPT ecosystem crates are pinned as git dependencies by exact commit (see `[workspace.dependencies]`); bump revs deliberately.
- GitHub repository `tpt-solutions/tpt-app-av-commissioning` is not yet created/connected remotely.