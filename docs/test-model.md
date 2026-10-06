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

`TestResult` carries: test id, status, mode, started/completed timestamps, evidence references, measurements, and human-readable messages. Manual and semi-automated results additionally carry the checklist (with verdicts, notes and evidence) and, while a semi-automated run awaits the engineer, the recorded software status.

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

- `Automated` — software runs the test end to end. Results are final; they cannot be "confirmed" (re-run instead).
- `SemiAutomated` — software prepares the test and captures measurements; the runner marks the result pending confirmation (`status: Manual`, software status recorded) and the engineer approves or rejects it via `TestResult::confirm`.
- `Manual` — the engineer is the instrument. The runner never executes software steps and touches no device: the result is produced with a `ManualChecklist` of items the engineer answers with Pass / Fail / N/A, each optionally carrying a note and evidence references. Confirming an incomplete checklist is an error; a failed item fails the test even when the engineer approves overall.

## Test procedures (§16)

Declarative YAML test format: `id`, `name`, and — depending on `mode` — `steps` composed of `command` / `wait` / `measure` / `assert`, a `checklist` of `label` (optionally `id`) entries, or both:

- `automated`: `steps` only.
- `manual`: `checklist` only; authored instructions become the engineer's checklist (`TestProcedure::manual_test`).
- `semi_automated`: `steps` (what software prepares/measures) and a `checklist` (what the engineer confirms).

Validation is mode-aware (a manual test with software steps, or a semi-automated test without a checklist, is rejected before execution), and arbitrary executable code in project/test files is explicitly disallowed.

## Runner (§17, §18)

- Dependency-aware scheduling with blocked-not-failed propagation.
- Parallel execution of independent tests with configurable concurrency; serial execution available.
- Timeouts, retries, cancellation, and rate limits.
- Result persistence after each test.
- `DeviceLock` (device_id, owner) prevents concurrent device mutation races. Signal-path tests declare the devices they mutate.

## Measurements (§19)

A `Measurement` has name, value, unit, tolerance, and source. Measurements preserve raw values — no internal rounding before tolerance evaluation.

## Concrete test types (§13)

`tpt-app-av-commissioning-test` ships ready-made tests for all nine kinds. Each reports its category through `CommissioningTest::kind()` (informational; scheduling never depends on it). DSL procedures carry the same category via an optional `kind:` pin, otherwise inferred from the fields they measure/assert (`TestProcedure::kind()`; unknown fields stay uncategorised).

| Building block | Mutates? | Kinds |
|---|---|---|
| `ConnectivityTest` (driver `discover`) | no | connectivity (UDP/HTTP/OSC/MIDI/serial via the driver) |
| `TcpReachableTest`, `RequiredPortsTest` (`net`) | no | connectivity, network |
| `StateCheckTest` + `identity`, `power_state`, `expected_route`, `signal_present`, `video`, `audio`, `network`, `synchronization` | no | identity, power, input/output, video, audio, network, synchronisation |
| `CommandTest` + `power_on`, `power_off`, `power_recovery`, `select_input`, `set_route`, `test_pattern_response`, `control_feedback` | yes (declares `mutate_devices`) | power, input/output, video, control |

Semantics worth knowing:

- **Feedback is independent.** `CommandTest` re-reads device state after the command instead of trusting the command response, so a device that acknowledges but doesn't act (e.g. a projector ignoring power) fails.
- **Status precedence** (`check::evaluate`): a missed required expectation is `Fail`; otherwise a field the device did not report is `Inconclusive`; otherwise a missed `advisory` expectation is `Warning`; otherwise `Pass`. A test with no expectations is `Inconclusive` — it proves nothing.
- **Driver errors:** unreachable/timeouts are a `Fail` finding only for connectivity tests; elsewhere they are returned as `Err` so the runner's retry rules apply. `UnsupportedOperation` is `Skipped` (not applicable), never a failure.
- **Safety (§36):** power-cycle tests are `Blocked` until `.confirm()` is called and send nothing. Network tests take literal addresses only (no name resolution or ranges), bound every connect by a timeout (max 30 s), cap ports per test (64), and send no data after the handshake.
- **State-field vocabulary.** Drivers expose readings under the names `classify_field` knows, e.g. `power`, `input`, `route`, `signal_present`, `resolution`, `frame_rate`, `colour_format`, `hdr_state`, `signal_lock`, `level`, `silence`, `polarity`, `phase`, `clipping`, `noise`, `channel_presence`, `ip_address`, `gateway`, `dns`, `latency_ms`, `packet_loss`, `link_state`, `av_offset`, `sync_offset`, `clock_drift`. Anything else can still be checked with a raw `Expectation`.

