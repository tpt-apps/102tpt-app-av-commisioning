# tpt-app-av-commissioning — Project Todo

TPT AV Commissioning — professional desktop application for commissioning, testing, diagnosing, documenting, and handing over audiovisual installations. Product family: TPT Apps. Publisher: TPT Solutions.

Tracking checklist for the whole project, organized by phase. License: dual MIT OR Apache-2.0. Spec section references (§) point back to `spec.txt`.

## Phase 0 — Project & Repo Setup

- [ ] Create GitHub repo `tpt-solutions/tpt-app-av-commissioning`
- [ ] Initialize Cargo workspace (`Cargo.toml`: `[workspace]`, `resolver = "2"`, `[workspace.package]`, `[workspace.dependencies]`)
- [ ] Dual licensing (MIT OR Apache-2.0)
  - [ ] Add `LICENSE-MIT`
  - [ ] Add `LICENSE-APACHE`
  - [ ] Set `license = "MIT OR Apache-2.0"` in `[workspace.package]`
- [ ] Add `deny.toml` (cargo-deny license allow/deny lists; permit MIT + Apache-2.0)
- [ ] Add root `README.md`
- [ ] Add `CHANGELOG.md`
- [ ] Add `docs/` (§6)
  - [ ] `docs/architecture.md`
  - [ ] `docs/domain-model.md`
  - [ ] `docs/device-drivers.md`
  - [ ] `docs/test-model.md`
  - [ ] `docs/report-format.md`
  - [ ] `docs/project-format.md`
  - [ ] `docs/security.md`
- [ ] Set up CI (GitHub Actions)
  - [ ] Build
  - [ ] Test
  - [ ] `cargo fmt --check`
  - [ ] `cargo clippy`
  - [ ] `cargo deny check`
- [ ] Scaffold empty crates under `crates/` (§6, each dual-licensed with own `Cargo.toml` + `src/lib.rs`)
  - [ ] `tpt-app-av-commissioning-core`
  - [ ] `tpt-app-av-commissioning-model`
  - [ ] `tpt-app-av-commissioning-device`
  - [ ] `tpt-app-av-commissioning-driver`
  - [ ] `tpt-app-av-commissioning-test`
  - [ ] `tpt-app-av-commissioning-runner`
  - [ ] `tpt-app-av-commissioning-report`
  - [ ] `tpt-app-av-commissioning-cli`
  - [ ] `tpt-app-av-commissioning-tauri`
  - [ ] `tpt-app-av-commissioning-testkit`
  - [ ] Revisit crate count/boundaries once core model + driver layer exist (§6 allows reduction; domain separation matters more than crate count)
- [ ] Scaffold `drivers/` directory (§6)
  - [ ] `drivers/generic/`
  - [ ] `drivers/osc/`
  - [ ] `drivers/midi/`
  - [ ] `drivers/network/`
  - [ ] `drivers/examples/`
- [ ] Scaffold `test-suites/` directory (§6)
  - [ ] `test-suites/generic/`
  - [ ] `test-suites/examples/`
- [ ] Scaffold `tests/` directory (§6)
  - [ ] `tests/integration/`
  - [ ] `tests/fixtures/`
  - [ ] `tests/golden/`
  - [ ] `tests/mock/`
- [ ] Note external dependency `tpt-kinetix` (`https://github.com/tpt-solutions/tpt-kinetix`) is not yet vendored locally — revisit integration once it exists/is cloned (§5.6, §55)

## Phase 1 — Core Domain Model (§7)

- [ ] `tpt-app-av-commissioning-model`
  - [ ] `Project` struct (id, name, client, site, rooms, devices, connections, test_suites)
  - [ ] `Room` struct (id, name, description, devices, connections)
  - [ ] `Device` struct (id, name, manufacturer, model, serial_number, firmware, device_type, endpoints, addresses)
  - [ ] `DeviceType` enum (Display, Projector, Camera, Microphone, Speaker, Amplifier, DSP, Switcher, Matrix, Scaler, Encoder, Decoder, MediaServer, ControlProcessor, TouchPanel, LightingController, NetworkDevice, Computer, Other) — extensible
  - [ ] `Endpoint` struct + `EndpointKind` enum (VideoInput, VideoOutput, AudioInput, AudioOutput, Network, Control, Gpio, Usb, Serial, Lighting, Clock)
  - [ ] `DeviceAddress` type
  - [ ] Unit tests: device models, endpoint models (§46.1)

## Phase 2 — Connection & Signal Path Model (§8–9)

- [ ] `Connection` struct (id, source, destination, signal_type, transport, expected)
- [ ] `SignalType` enum (Video, Audio, AudioVideo, Network, Control, Lighting, Clock, Unknown)
- [ ] `Transport` enum (HDMI, SDI, DisplayPort, USB, AES3, Analog, Dante, NDI, RTP, RTSP, OSC, MIDI, ArtNet, SACN, Ethernet, Serial, Other) — extensible for proprietary/emerging protocols
- [ ] `ConnectionExpectation` type
- [ ] `SignalGraph` struct (nodes, edges) — graph, not a simple list
- [ ] Path requirement schema (video resolution/frame_rate/hdr, audio channels, latency max_ms)
- [ ] Unit tests: topology construction and traversal (§46.1)

## Phase 3 — Persistence (§25–27)

- [ ] SQLite schema
  - [ ] project metadata
  - [ ] rooms
  - [ ] devices
  - [ ] endpoints
  - [ ] connections
  - [ ] test suites
  - [ ] test definitions
  - [ ] test executions
  - [ ] results
  - [ ] measurements
  - [ ] defects
  - [ ] evidence metadata
  - [ ] configuration baselines
- [ ] Managed project asset directory layout (`assets/screenshots`, `assets/recordings`, `assets/configurations`, `assets/evidence`, `reports/`, `exports/`)
- [ ] Human-readable project manifest format (YAML, `schema_version`, rooms/devices with human-readable ids) — version-controllable, portable
- [ ] Configuration snapshot storage
  - [ ] Label snapshots (Before commissioning / After commissioning / Before maintenance / After maintenance)
  - [ ] Never silently overwrite previous snapshot

## Phase 4 — Device Driver Architecture (§10, §39)

- [ ] `tpt-app-av-commissioning-driver`
  - [ ] `DeviceDriver` trait (identity, discover, get_state, execute, capabilities)
  - [ ] `DeviceDiscovery` trait
  - [ ] `DeviceCommand` trait
  - [ ] `DeviceState` trait
  - [ ] `DeviceCapability` trait
  - [ ] `DeviceCapabilities` struct (can_power_on, can_power_off, can_read_state, can_select_input, can_generate_test_pattern, can_read_signal_status, can_read_edid, can_measure_latency)
  - [ ] Async driver support where protocol behaviour requires it
- [ ] Integrate `tpt-av-control` as device-control foundation (OSC/MIDI/MIDI 2.0/DMX/Art-Net/sACN/WebRTC/control surfaces)
- [ ] Contribute generic improvements back to `tpt-av-control`/`tpt-av-test` where applicable

## Phase 5 — Generic Protocol Drivers (§40–41)

- [ ] Generic drivers under `drivers/generic/`
  - [ ] TCP
  - [ ] UDP
  - [ ] HTTP
  - [ ] WebSocket
  - [ ] OSC (`drivers/osc/`)
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

- [ ] `tpt-app-av-commissioning-test`
  - [ ] `CommissioningTest` trait (id, name, requirements, execute)
  - [ ] `TestStatus` enum (Pass, Fail, Warning, Blocked, Skipped, Manual, Inconclusive)
  - [ ] `TestResult` struct (test_id, status, started_at, completed_at, evidence, measurements, messages)
  - [ ] `TestRequirements` type
- [ ] `tpt-app-av-commissioning-runner`
  - [ ] Dependency-aware scheduling (blocked-not-failed propagation, §14)
  - [ ] Parallel execution
  - [ ] Serial execution
  - [ ] Timeouts
  - [ ] Retries
  - [ ] Cancellation
  - [ ] Rate limits
  - [ ] Result persistence
  - [ ] `DeviceLock` struct (device_id, owner) — prevent concurrent device mutation races
  - [ ] Signal-path tests declare devices they mutate

## Phase 8 — Test Procedure DSL (§16)

- [ ] Declarative YAML test format (id, name, steps: command/wait/measure/assert)
- [ ] Schema validation before execution
- [ ] Explicitly disallow arbitrary executable code in project/test files

## Phase 9 — Test Types (§13)

- [ ] Connectivity: TCP/UDP reachable, HTTP response, OSC response, MIDI device present, serial connection available
- [ ] Device identity: manufacturer, model, serial, firmware, expected address
- [ ] Power: power state, power on, power off, state feedback, power recovery
- [ ] Input/output: select input, verify signal, verify output, verify expected route
- [ ] Video: resolution, frame rate, colour format, HDR state, signal lock, timing, black level, test pattern response
- [ ] Audio: channel presence, routing, level, silence, frequency response, polarity, phase, clipping, noise
- [ ] Control: command → device → expected state → feedback verification
- [ ] Synchronisation: audio/video offset, device timing, clock drift, multi-device sync
- [ ] Network: IP, gateway, DNS, latency, packet loss, link state, required ports, device reachability (non-intrusive by default)

## Phase 10 — Manual & Semi-Automated Workflow (§15, §4.5)

- [ ] `ExecutionMode` enum (Automated, SemiAutomated, Manual)
- [ ] Automated test execution path
- [ ] Semi-automated path (software prepares/measures, engineer confirms)
- [ ] Manual test path (checklist: Pass / Fail / N/A / Note / Evidence) as first-class citizen
- [ ] Manual test authoring support (step lists, e.g. projector image-quality inspection checklist)

## Phase 11 — Evidence System (§24)

- [ ] `EvidenceKind` enum (Screenshot, Photo, AudioRecording, VideoRecording, NetworkResult, DeviceResponse, Measurement, Configuration, OperatorNote)
- [ ] Evidence stored as immutable project assets (referenced from results, not embedded in DB)
- [ ] Integrate `tpt-av-asset` for test media assets, cached screenshots, recordings, generated evidence, project asset management

## Phase 12 — Measurement Model (§19)

- [ ] `Measurement` struct (name, value, unit, tolerance, source)
- [ ] `MeasurementValue`, `Unit`, `Tolerance`, `MeasurementSource` types
- [ ] Preserve raw values; no internal rounding before tolerance evaluation
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

- [ ] `tpt-app-av-commissioning-report`
  - [ ] PDF output (cover page, project details, system summary, device inventory, topology, test summary, detailed results, defects, evidence, engineer sign-off, software/profile version, date/time)
  - [ ] HTML output
  - [ ] CSV output
  - [ ] JSON output
- [ ] Report distinguishes automated test / manual inspection / engineer acceptance (never imply automated measurement that didn't occur)
- [ ] Electronic sign-off (engineer name, date, result incl. "PASS WITH ACCEPTED EXCEPTIONS", exception list)
- [ ] Audit event recorded when a report is signed
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

- [ ] `tpt-app-av-commissioning-cli` (shares core engine with GUI)
  - [ ] `validate --project <file> --suite <name>`
  - [ ] `test --project <file> --room <name>`
  - [ ] `report --project <file> --format <pdf|html|csv|json>`
  - [ ] Automation-friendly exit codes/output for integrator deployment pipelines

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

- [ ] `tpt-app-av-commissioning-testkit`
  - [ ] `MockProjector`
  - [ ] `MockDisplay`
  - [ ] `MockMatrix`
  - [ ] `MockDSP`
  - [ ] `MockAudioEndpoint`
  - [ ] `MockControlProcessor`
  - [ ] Simulate: normal operation, latency, timeout, malformed response, incorrect state, intermittent failure, unreachable device
- [ ] Fault injection scenarios (display offline, matrix output stuck, wrong EDID, audio channel missing, projector ignores power command, network latency, packet loss, incorrect feedback, device reboot)
- [ ] Verify no malformed device response can crash the application (§54)

## Phase 29 — Testing Strategy (§46)

- [ ] Unit tests: device/endpoint models, topology, test definitions, DSL parsing, tolerance logic, result aggregation, report generation
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
