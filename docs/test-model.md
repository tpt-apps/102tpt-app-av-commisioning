# Test Model

This document describes the test model and runner (§12–18 of `spec.txt`). Implemented in the `test` and `runner` crates.

## Test definition

A test is identified by id and name, declares its requirements, and knows how to execute against a set of drivers:

- `CommissioningTest` trait: `id`, `name`, `requirements`, `execute`
- `TestRequirements`: what the test needs (devices, mutating vs read-only, execution mode, timeout budget)

## Status

`TestStatus` is: `Pass`, `Fail`, `Warning`, `Blocked`, `Skipped`, `Manual`, `Inconclusive`.

`Blocked` (not `Fail`) is used when a test cannot run because a dependency failed — a blocked test has not been executed, so it must not be reported as a failure (§14).

## Result

`TestResult` carries: test id, status, started/completed timestamps, evidence references, measurements, and human-readable messages.

## Test types (§13)

| Category | Checks |
|---|---|
| Connectivity | TCP/UDP reachable, HTTP response, OSC response, MIDI present, serial available |
| Device identity | Manufacturer, model, serial, firmware, expected address |
| Power | Power state, on/off, state feedback, power recovery |
| Input/output | Select input, verify signal/output, verify expected route |
| Video | Resolution, frame rate, colour format, HDR, signal lock, timing, black level, test pattern response |
| Audio | Channel presence, routing, level, silence, frequency response, polarity, phase, clipping, noise |
| Control | Command → device → expected state → feedback verification |
| Synchronisation | A/V offset, device timing, clock drift, multi-device sync |
| Network | IP, gateway, DNS, latency, packet loss, link state, required ports, reachability (non-intrusive by default) |

## Execution modes (§15)

- `Automated` — software runs the test end to end.
- `SemiAutomated` — software prepares/measures; an engineer confirms.
- `Manual` — a first-class checklist (Pass / Fail / N/A / Note / Evidence) where the engineer is the instrument (e.g. projector image-quality inspection).

## Test procedures (§16)

Declarative YAML test format: `id`, `name`, `steps` composed of `command` / `wait` / `measure` / `assert`.

- Schema-validated before execution.
- Arbitrary executable code in project/test files is explicitly disallowed.

## Runner (§17, §18)

- Dependency-aware scheduling with blocked-not-failed propagation.
- Parallel execution of independent tests with configurable concurrency; serial execution available.
- Timeouts, retries, cancellation, and rate limits.
- Result persistence after each test.
- `DeviceLock` (device_id, owner) prevents concurrent device mutation races. Signal-path tests declare the devices they mutate.

## Measurements (§19)

A `Measurement` has name, value, unit, tolerance, and source. Measurements preserve raw values — no internal rounding before tolerance evaluation.