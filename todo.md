# tpt-app-av-commissioning — Project Todo

TPT AV Commissioning — professional desktop application for commissioning, testing, diagnosing, documenting, and handing over audiovisual installations. Product family: TPT Apps. Publisher: TPT Solutions.

Tracking checklist for the whole project, organized by phase. License: dual MIT OR Apache-2.0. Spec section references (§) point back to `spec.txt`.

## Phase 0 — Project & Repo Setup

- [x] Create GitHub repo `tpt-solutions/tpt-app-av-commissioning`
- [x] Initialize Cargo workspace (`Cargo.toml`: `[workspace]`, `resolver = "2"`, `[workspace.package]`, `[workspace.dependencies]`)
- [x] Dual licensing (MIT OR Apache-2.0)
  - [x] Add `LICENSE-MIT`
  - [x] Add `LICENSE-APACHE`
  - [x] Set `license = "MIT OR Apache-2.0"` in `[workspace.package]`
- [x] Add `deny.toml` (cargo-deny license allow/deny lists; permit MIT + Apache-2.0)
- [x] Add root `README.md`
- [x] Add `CHANGELOG.md`
- [x] Add `docs/` (§6)
  - [x] `docs/architecture.md`
  - [x] `docs/domain-model.md`
  - [x] `docs/device-drivers.md`
  - [x] `docs/test-model.md`
  - [x] `docs/report-format.md`
  - [x] `docs/project-format.md`
  - [x] `docs/security.md`
- [x] Set up CI (GitHub Actions)
  - [x] Build
  - [x] Test
  - [x] `cargo fmt --check`
  - [x] `cargo clippy`
  - [x] `cargo deny check`
- [x] Scaffold empty crates under `crates/` (§6, each dual-licensed with own `Cargo.toml` + `src/lib.rs`)
  - [x] `tpt-app-av-commissioning-core`
  - [x] `tpt-app-av-commissioning-model`
  - [x] `tpt-app-av-commissioning-device`
  - [x] `tpt-app-av-commissioning-driver`
  - [x] `tpt-app-av-commissioning-test`
  - [x] `tpt-app-av-commissioning-runner`
  - [x] `tpt-app-av-commissioning-report`
  - [x] `tpt-app-av-commissioning-cli`
  - [x] `tpt-app-av-commissioning-tauri`
  - [x] `tpt-app-av-commissioning-testkit`
  - [ ] Revisit crate count/boundaries once core model + driver layer exist (§6 allows reduction; domain separation matters more than crate count)
- [x] Scaffold `drivers/` directory (§6)
  - [x] `drivers/generic/`
  - [x] `drivers/osc/`
  - [x] `drivers/midi/`
  - [x] `drivers/network/`
  - [x] `drivers/examples/`
- [x] Scaffold `test-suites/` directory (§6)
  - [x] `test-suites/generic/`
  - [x] `test-suites/examples/`
- [x] Scaffold `tests/` directory (§6)
  - [x] `tests/integration/`
  - [x] `tests/fixtures/`
  - [x] `tests/golden/`
  - [x] `tests/mock/`
- [x] External TPT dependencies (`tpt-kinetix`, `tpt-av-control`, `tpt-av-asset`, `tpt-audio`, `tpt-dsp`, `tpt-cadence`, `tpt-av-ui`, `tpt-av-sync`, `tpt-av-test`) are vendored as git dependencies pinned to exact commits in `[workspace.dependencies]`, allow-listed in `deny.toml`, and verified to resolve and compile together; crates opt in per phase (§5.6, §55)

## Phase 1 — Core Domain Model (§7)

- [x] `tpt-app-av-commissioning-model`
  - [x] `Project` struct (id, name, client, site, rooms, devices, connections, test_suites)
  - [x] `Room` struct (id, name, description, devices, connections)
  - [x] `Device` struct (id, name, manufacturer, model, serial_number, firmware, device_type, endpoints, addresses)
  - [x] `DeviceType` enum (Display, Projector, Camera, Microphone, Speaker, Amplifier, DSP, Switcher, Matrix, Scaler, Encoder, Decoder, MediaServer, ControlProcessor, TouchPanel, LightingController, NetworkDevice, Computer, Other) — extensible
  - [x] `Endpoint` struct + `EndpointKind` enum (VideoInput, VideoOutput, AudioInput, AudioOutput, Network, Control, Gpio, Usb, Serial, Lighting, Clock)
  - [x] `DeviceAddress` type
  - [x] Unit tests: device models, endpoint models (§46.1)

## Phase 2 — Connection & Signal Path Model (§8–9)

- [x] `Connection` struct (id, source, destination, signal_type, transport, expected)
- [x] `SignalType` enum (Video, Audio, AudioVideo, Network, Control, Lighting, Clock, Unknown)
- [x] `Transport` enum (HDMI, SDI, DisplayPort, USB, AES3, Analog, Dante, NDI, RTP, RTSP, OSC, MIDI, ArtNet, SACN, Ethernet, Serial, Other) — extensible for proprietary/emerging protocols
- [x] `ConnectionExpectation` type
- [x] `SignalGraph` struct (nodes, edges) — graph, not a simple list
- [x] Path requirement schema (video resolution/frame_rate/hdr, audio channels, latency max_ms)
- [x] Unit tests: topology construction and traversal (§46.1)

## Phase 3 — Persistence (§25–27)

- [x] SQLite schema
  - [x] project metadata
  - [x] rooms
  - [x] devices
  - [x] endpoints
  - [x] connections
  - [x] test suites
  - [x] test definitions
  - [x] test executions
  - [x] results
  - [x] measurements
  - [x] defects
  - [x] evidence metadata
  - [x] configuration baselines
- [x] Managed project asset directory layout (`assets/screenshots`, `assets/recordings`, `assets/configurations`, `assets/evidence`, `reports/`, `exports/`)
- [x] Human-readable project manifest format (YAML, `schema_version`, rooms/devices with human-readable ids) — version-controllable, portable
- [x] Configuration snapshot storage
  - [x] Label snapshots (Before commissioning / After commissioning / Before maintenance / After maintenance)
  - [x] Never silently overwrite previous snapshot

## Phase 4 — Device Driver Architecture (§10, §39)

- [x] `tpt-app-av-commissioning-driver`
  - [x] `DeviceDriver` trait (identity, discover, get_state, execute, capabilities)
  - [x] `DeviceDiscovery` trait
  - [x] `DeviceCommand` enum (typed command list, incl. `Arbitrary` for vendor protocols) — see note below
  - [x] `DeviceState` struct
  - [x] `DeviceCapability` enum
  - [x] `DeviceCapabilities` struct (can_power_on, can_power_off, can_read_state, can_select_input, can_generate_test_pattern, can_read_signal_status, can_read_edid, can_measure_latency)
  - [x] Async driver support where protocol behaviour requires it (`AsyncDeviceDriver`)
- [ ] Integrate `tpt-av-control` as device-control foundation (OSC/MIDI/MIDI 2.0/DMX/Art-Net/sACN/WebRTC/control surfaces) — pinned in the workspace; OSC is wired in (`drivers/osc`), MIDI/MIDI 2.0/DMX/Art-Net/sACN/WebRTC/surfaces are not yet
- [ ] Contribute generic improvements back to `tpt-av-control`/`tpt-av-test` where applicable

> Note: `DeviceCommand`/`DeviceState`/`DeviceCapability` implemented as typed enums/structs rather
> than traits — matches the later driver SDK phase and keeps drivers data-driven.

## Phase 5 — Generic Protocol Drivers (§40–41)

- [ ] Generic drivers under `drivers/generic/`
  - [ ] TCP
  - [ ] UDP
  - [ ] HTTP
  - [ ] WebSocket
  - [x] OSC (`drivers/osc/`) — `tpt-app-av-commissioning-driver-osc`: command bindings + state queries as data, bounded UDP, built on the `tpt-av-control-osc` codec; tested end to end against a fake OSC device through the Phase 9 tests
  - [ ] MIDI (`drivers/midi/`)
  - [ ] SNMP (where appropriate)
  - [ ] Serial
- [ ] Versioned device profile format (`drivers/examples/`: match manufacturer/model, protocol config, commands, state query/parser)
- [ ] Profile format validation (versioned, no arbitrary code execution)

## Phase 6 — Device Discovery (§11)

- [ ] Discovery mechanisms (network scan, mDNS, SSDP, SNMP, vendor protocol discovery, OSC discovery, MIDI enumeration, local audio-device enumeration)
- [ ] Explicit, user-controlled discovery configuration
  - [ ] Network interface selection
  - [ ] IP range selection
  - [ ] Discovery protocol selection
  - [ ] Timeout configuration
- [ ] No aggressive/arbitrary scanning by default (§36 cross-reference)

## Phase 7 — Test Model & Runner (§12, §17–18)

- [x] `tpt-app-av-commissioning-test`
  - [x] `CommissioningTest` trait (id, name, requirements, execute)
  - [x] `TestStatus` enum (Pass, Fail, Warning, Blocked, Skipped, Manual, Inconclusive)
  - [x] `TestResult` struct (test_id, status, started_at, completed_at, evidence, measurements, messages)
  - [x] `TestRequirements` type
- [x] `tpt-app-av-commissioning-runner`
  - [x] Dependency-aware scheduling (blocked-not-failed propagation, §14) — `plan.rs`
  - [x] Parallel execution (concurrency-slot worker pool, `execute.rs`) + 10 executor tests
  - [x] Serial execution (concurrency = 1)
  - [x] Timeouts (per-test max_duration, timeout override; abandoned-thread guard)
  - [x] Retries (`effective_retries`; transient-error + timeout retry loops)
  - [x] Cancellation (`CancelToken`, `RunObserver`/`CancelToken` wiring)
  - [x] Rate limits (`min_start_interval` gap between scheduled starts)
  - [x] `DeviceLock` struct (device_id, owner) + `LockRegistry` — prevent concurrent device mutation races (`lock.rs`)
  - [x] Non-blocking `can_acquire` + dependency/active-mutation gates before dispatch
  - [ ] Result persistence wiring (store persistence exists; executor hands results to `RunObserver`; app-layer persistence not yet wired)
  - [ ] Signal-path tests declare devices they mutate (DSL `mutate_devices` exists; no executable-content risk — see §14)

## Phase 8 — Test Procedure DSL (§16)

- [x] Declarative YAML test format (id, name, steps: command/wait/measure/assert)
- [x] Schema validation before execution (`TestProcedure::validate`, `from_yaml_str`)
- [x] Explicitly disallow arbitrary executable code in project/test files (executable-content guard mirrors the manifest guard; `docs/security.md`)

## Phase 9 — Test Types (§13)

- [x] `TestKind` taxonomy enum (Connectivity, Identity, Power, InputOutput, Video, Audio, Control, Synchronization, Network — §13.1–13.9); wired into `CommissioningTest::kind()` and the DSL (`kind:` pin or inferred from measured/asserted fields)
- [x] Connectivity: TCP/UDP reachable, HTTP response, OSC response, MIDI device present, serial connection available — `TcpReachableTest` (real bounded TCP connect) + driver-backed `ConnectivityTest` (UDP/HTTP/OSC/MIDI/serial probes go through the driver; real protocol drivers are Phase 5)
- [x] Device identity: manufacturer, model, serial, firmware, expected address
- [x] Power: power state, power on, power off, state feedback, power recovery (power-cycle blocked until explicitly confirmed, §36)
- [x] Input/output: select input, verify signal, verify output, verify expected route
- [x] Video: resolution, frame rate, colour format, HDR state, signal lock, timing, black level, test pattern response (`VideoSpec` covers resolution/frame rate/colour/HDR/lock; timing and black level via raw `Expectation`s; pixel-level measurement is Phase 34)
- [x] Audio: channel presence, routing, level, silence, frequency response, polarity, phase, clipping, noise (device-reported values via `AudioSpec`/`Expectation`; signal-analysis measurement of frequency response etc. awaits `tpt-audio`/`tpt-dsp`, Phase 12)
- [x] Control: command → device → expected state → feedback verification
- [x] Synchronisation: audio/video offset, device timing, clock drift, multi-device sync (device-reported; measured sync awaits `tpt-av-sync`, Phase 35)
- [x] Network: IP, gateway, DNS, latency, packet loss, link state, required ports, device reachability (non-intrusive by default)

## Phase 10 — Manual & Semi-Automated Workflow (§15, §4.5)

- [ ] `ExecutionMode` enum (Automated, SemiAutomated, Manual)
- [ ] Automated test execution path
- [ ] Semi-automated path (software prepares/measures, engineer confirms)
- [ ] Manual test path (checklist: Pass / Fail / N/A / Note / Evidence) as first-class citizen
- [ ] Manual test authoring support (step lists, e.g. projector image-quality inspection checklist)

## Phase 11 — Evidence System (§24)

- [x] `EvidenceKind` enum (Screenshot, Photo, AudioRecording, VideoRecording, NetworkResult, DeviceResponse, Measurement, Configuration, OperatorNote)
- [x] Evidence stored as immutable project assets (referenced from results, not embedded in DB)
- [ ] Integrate `tpt-av-asset` for test media assets, cached screenshots, recordings, generated evidence, project asset management

## Phase 12 — Measurement Model (§19)

- [x] `Measurement` struct (name, value, unit, tolerance, source)
- [x] `MeasurementValue`, `Unit`, `Tolerance`, `MeasurementSource` types
- [x] Preserve raw values; no internal rounding before tolerance evaluation
- [ ] Integrate `tpt-audio` (routing, level monitoring, channel testing, signal generation/monitoring)
- [ ] Integrate `tpt-dsp` (frequency response, signal level, noise, THD, phase, latency, spectral analysis, test-tone analysis)
- [ ] Integrate `tpt-cadence` where audio codec handling of test media is required

## Phase 13 — End-to-End Tests (§20)

- [ ] Video signal-path E2E test (source → switcher → scaler → display: configure, wait for stabilisation, detect signal, validate resolution/frame rate, capture evidence, restore previous state)
- [ ] Audio signal-path E2E test (mic → DSP → amp → speaker: generate signal, route, measure output, verify level/channel/phase/polarity, record result)
- [ ] Integrate `tpt-kinetix` for test video generation/playback/media decoding/test signals once available (external dependency, §5.6)

## Phase 14 — Baseline / Regression Mode (§21)

- [ ] Create baseline (version, device/connection/test counts, aggregate result)
- [ ] Run against baseline
- [ ] Highlight only meaningful changes between runs

## Phase 15 — Configuration Drift Detection (§22)

- [ ] Compare current system state vs recorded baseline
- [ ] Detect firmware change, IP change, device replacement, configuration change, signal-route change, resolution change, audio-routing change, control configuration change
- [ ] Configuration fingerprinting (manufacturer + model + serial + firmware + config fingerprint) so replacement devices aren't assumed identical

## Phase 16 — Defect Tracking (§23)

- [ ] `Defect` struct (id, severity, title, description, related_tests, evidence, status)
- [ ] `DefectStatus` enum (Open, Investigating, Fixed, Retest Required, Verified, Accepted, Deferred)
- [ ] Convert failed test → defect workflow
- [ ] Defect ↔ test/evidence linkage

## Phase 17 — Audit Log (§33)

- [ ] `AuditEvent` struct (timestamp, event_type, actor, object_id, details)
- [ ] `AuditEventType` coverage: project created, device added/modified, test run, result changed, defect created/closed, baseline created, configuration imported, report generated, report signed

## Phase 18 — Security & Network Safety (§36, §38)

- [ ] `execution_policy` project setting (allow_power_cycle, allow_configuration_changes, allow_network_changes)
- [ ] Explicit interface/target selection for all network operations
- [ ] Configurable timeouts and scan ranges
- [ ] Confirmation required before reboot/power-cycle tests
- [ ] Command logging
- [ ] Dry-run mode
- [ ] No arbitrary command execution from project files or test DSL
- [ ] Strict protocol parsing; bounded network reads
- [ ] Authentication support for device protocols
- [ ] Encrypted credential storage; OS credential store integration where practical
- [ ] Sensitive values excluded from reports by default
- [ ] Project access permissions handled by the OS
- [ ] Safe parsing of imported configuration files
- [ ] Credentials never committed into project manifests

## Phase 19 — Restore-State Mechanism (§37)

- [ ] Pre-test state capture for mutating tests
- [ ] Post-test state restoration
- [ ] Explicit "test passed, but device state restoration failed" reporting when restore fails
- [ ] Tests declare whether state restoration is supported

## Phase 20 — Reporting (§31–32)

- [x] `tpt-app-av-commissioning-report`
  - [ ] PDF output (cover page, project details, system summary, device inventory, topology, test summary, detailed results, defects, evidence, engineer sign-off, software/profile version, date/time)
  - [ ] HTML output
  - [x] CSV output
  - [x] JSON output
  - [x] Markdown output (used for the summary/headline formats)
- [x] Report distinguishes automated test / manual inspection / engineer acceptance (summary counts by `TestStatus`, incl. Manual/Inconclusive — never implies measurements that didn't occur)
- [x] Electronic sign-off (engineer name, date, result incl. "PASS WITH ACCEPTED EXCEPTIONS", exception list)
- [ ] Audit event recorded when a report is signed (sign-off types exist; audit wiring is Phase 17)
- [ ] Unit tests: report generation, golden reports (§46.1, §46.4)

## Phase 21 — Desktop UI (Tauri) (§28)

- [ ] `tpt-app-av-commissioning-tauri`
  - [ ] Project Dashboard (project/client/site/rooms/devices/tests/latest result/open defects/last baseline)
  - [ ] System Graph (interactive topology; status via more than colour alone: healthy/warning/failed/offline/unknown)
  - [ ] Device View (identity, address, firmware, endpoints, capabilities, current state, recent tests, configuration snapshots)
  - [ ] Test Runner view (live per-test status: PASS/RUN/WAIT/BLOCK)
  - [ ] Results view (filter by room/device/test/severity/status/date)
  - [ ] Defects view (dedicated list + workflow)
  - [ ] Use `tpt-av-ui` primitives (signal-path diagrams, timeline/evidence display, meters, waveform display, device state)

## Phase 22 — Commissioning Wizard (§29)

- [ ] Step 1 — Create project
- [ ] Step 2 — Add rooms
- [ ] Step 3 — Discover/add devices
- [ ] Step 4 — Confirm device identities
- [ ] Step 5 — Build topology
- [ ] Step 6 — Select commissioning profile
- [ ] Step 7 — Run automated preflight
- [ ] Step 8 — Run signal-path tests
- [ ] Step 9 — Perform manual tests
- [ ] Step 10 — Resolve defects
- [ ] Step 11 — Retest
- [ ] Step 12 — Generate handover package

## Phase 23 — Handover Package (§30)

- [ ] Package generation: commissioning report, device inventory, network inventory, signal topology, test results, failed/resolved tests, defect register, configuration snapshots, evidence, test profile, software version, project baseline

## Phase 24 — CLI (§34)

- [x] `tpt-app-av-commissioning-cli` (shares core engine with GUI)
  - [x] `validate --project <file> [--suite <path>]` (suite = file or directory of DSL files)
  - [ ] `test --project <file> --room <name>` (needs runner executor, Phase 7)
  - [ ] `report --project <file> --format <pdf|html|csv|json>` (needs run data + report wiring)
  - [x] Automation-friendly exit codes/output (`0` valid / `1` invalid / `2` usage) for integrator deployment pipelines
  - [x] Integration tests (`crates/tpt-app-av-commissioning-cli/tests/cli.rs`; exit-code contract, `--version`/`--help`, valid/invalid project + suite fixtures)

## Phase 25 — Local API (§35)

- [ ] Optional localhost-only API (default bind `127.0.0.1`, never all interfaces by default)
  - [ ] `POST /projects/:id/runs`
  - [ ] `GET /runs/:id`
  - [ ] `GET /runs/:id/results`
  - [ ] `POST /runs/:id/cancel`
  - [ ] `GET /devices`
  - [ ] `GET /health`
- [ ] Authentication required if external binding is ever enabled

## Phase 26 — Driver SDK & Device Profiles (§39, §41)

- [ ] Public SDK traits (`DeviceDriver`, `DeviceDiscovery`, `DeviceCommand`, `DeviceState`, `DeviceCapability`) documented for third-party driver authors
- [ ] Driver declaration surface: protocol, manufacturer, model matching, discovery method, capabilities, commands, responses, state parsing, health checks
- [ ] Drivers compiled into the application initially (no dynamic plugin system yet)
- [ ] Versioned device profile format (`docs/device-drivers.md`)
- [ ] Evaluate dynamic plugin system only after static architecture is proven

## Phase 27 — Test & Project Templates (§42–43)

- [ ] Test templates
  - [ ] Display commissioning (identity, network, power, input, resolution, frame rate, signal lock, visual inspection)
  - [ ] Projector commissioning (identity, power, input, signal, resolution, geometry, focus, image uniformity, lamp/laser status)
  - [ ] Audio commissioning (device identity, channel mapping, routing, level, polarity, phase, noise, latency)
  - [ ] Control commissioning (panel connectivity, command execution, feedback, state consistency, error handling)
- [ ] Project templates (Boardroom, Meeting Room, Training Room, Lecture Theatre, Auditorium, Digital Signage, Video Conference Room, Control Room, Theatre) — editable, vendor-neutral

## Phase 28 — Testkit, Mock Devices & Fault Injection (§46.2, §47)

- [x] `tpt-app-av-commissioning-testkit`
  - [x] `MockProjector` (via `MockDevice::projector()` preset)
  - [x] `MockDisplay` (via `MockDevice::display()` preset)
  - [x] `MockMatrix` (via `MockDevice::matrix()` preset)
  - [x] `MockDSP` (via `MockDevice::dsp()` preset)
  - [x] `MockAudioEndpoint` (via `MockDevice::audio_endpoint()` preset)
  - [x] `MockControlProcessor` (via `MockDevice::control_processor()` preset)
  - [x] Simulate: normal operation, latency, timeout, malformed response, incorrect state, intermittent failure, unreachable device
- [x] Fault injection scenarios (display offline, matrix output stuck, wrong EDID, audio channel missing, projector ignores power command, network latency, packet loss, incorrect feedback, device reboot) — `Fault` enum + workflow-state faults
- [ ] Verify no malformed device response can crash the application (§54) — mocks return errors safely; full fuzz pass + HIL validation pending

## Phase 29 — Testing Strategy (§46)

- [x] Unit tests: device/endpoint models, topology, test definitions, DSL parsing, tolerance logic, result aggregation, report generation
- [ ] Integration tests against mock networks (e.g. Mock PC → Mock Matrix → Mock Scaler → Mock Display)
- [ ] Golden reports (project + suite + mock device state → stable expected report)
- [ ] Hardware-in-the-loop lab (small real-device set) — validate protocol implementations, drivers, timing, state restoration, firmware variations (ongoing/eventual, §46.5)

## Phase 30 — Performance (§44)

- [ ] Asynchronous device communication throughout
- [ ] Bounded queues
- [ ] Cancellation support
- [ ] Parallel independent test execution
- [ ] Persistent progress reporting
- [ ] No blocking UI thread
- [ ] Efficient SQLite transactions
- [ ] Configurable network test concurrency
- [ ] Use TPT's real-time-safe infrastructure for media/signal analysis where applicable

## Phase 31 — Offline Operation Verification (§45)

- [ ] Verify fully offline install supports: project creation, device configuration, manual tests, automated local/network tests, reports, baselines, defect tracking, evidence, sign-off — no internet required

## Phase 32 — Packaging & Release

- [ ] Package Windows application
- [ ] Run private beta with AV integrators
- [ ] Validate against real AV hardware (§54)
- [ ] Commission a real AV installation with the software (Definition of Done, §54)
- [ ] Use beta feedback to determine which vendor drivers/advanced measurements justify the next release (§55)
- [ ] Confirm Definition of Done checklist (§54) fully satisfied before release

## Phase 33 — Commercial Setup

- [ ] Validate pricing hypothesis with real integrators (§2.1: Engineer $799 / Integrator $1,499 / Professional $2,999+)
- [ ] Perpetual desktop licence model (no subscription dependency); optional major-version upgrade pricing
- [ ] Optional support contracts (post-launch)
- [ ] Positioning materials: "Executable commissioning for professional AV systems" (§53) — avoid "pings AV devices" framing

## Phase 34 — Post-MVP Phase 2 (§49)

- [ ] Richer video measurements
- [ ] Audio measurement suite
- [ ] Latency measurement
- [ ] Advanced topology visualisation
- [ ] More generic protocols
- [ ] More device profiles
- [ ] Richer configuration snapshots

## Phase 35 — Post-MVP Phase 3 (§49)

- [ ] Vendor driver ecosystem
- [ ] Automatic device discovery
- [ ] Advanced AV sync (integrate `tpt-av-sync`: synchronisation measurements, multi-device timing)
- [ ] Signal-path recording
- [ ] Automated end-to-end test generation
- [ ] Regression testing

## Phase 36 — Post-MVP Phase 4 (§49)

- [ ] Team workflows
- [ ] LAN test workers
- [ ] Shared project repositories (integrate `tpt-av-sync` collaborative project editing)
- [ ] Remote site diagnostics
- [ ] Service/maintenance mode
- [ ] Explicitly no cloud dependency introduced merely to implement team features

## Phase 37 — Long-Term Differentiators (§50–52)

- [ ] Topology-driven automatic test generation (infer path tests + requirements from the system graph)
- [ ] Digital twin model (physical system ↔ TPT model ↔ tests ↔ observed state)
- [ ] Full lifecycle mode: Commission → Baseline → Maintenance → Run regression → Detect drift → Repair → Retest → Update baseline

## Strategic Boundary (§56)

- [ ] Keep `tpt-app-av-commissioning` scoped to Model / Discover / Test / Measure / Compare / Diagnose / Document / Prove
- [ ] Do not absorb Operate / Route / Trigger / Automate / Synchronise responsibilities — those belong to `tpt-av-control` and the future AV Automation product
