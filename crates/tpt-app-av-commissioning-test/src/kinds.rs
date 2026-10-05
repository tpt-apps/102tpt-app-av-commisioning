//! Concrete commissioning tests for the nine test kinds (§13.1–13.9).
//!
//! Three building blocks cover the device-facing kinds:
//!
//! * [`ConnectivityTest`] — can the device be reached through its driver?
//! * [`StateCheckTest`] — read-only: read device state, verify expectations
//!   (identity, power state, video, audio, network, synchronisation, routes).
//! * [`CommandTest`] — mutating: send commands, wait, read the state back
//!   independently of the command response, verify (power on/off/recovery,
//!   input select, test pattern, routing, control feedback).
//!
//! Network-level checks that need no driver live in [`crate::net`].
//!
//! Drivers are shared as [`SharedDriver`]; the runner's `DeviceLock` (driven
//! by `requirements().mutate_devices`) serialises mutating tests, the mutex
//! only guards the handle itself.
//!
//! Driver-error policy: a device being unreachable is a *finding* for
//! connectivity tests (→ `Fail`); for every other test it is an
//! infrastructure error returned as `Err` so the runner's retry logic applies.
//! `UnsupportedOperation` is never a failure — the test is not applicable to
//! that device and reports `Skipped`.

use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use chrono::Utc;

use tpt_app_av_commissioning_device::{DeviceCommand, DeviceIdentity, DeviceState, StateValue};
use tpt_app_av_commissioning_driver::{DeviceDriver, DriverError};
use tpt_app_av_commissioning_model::{
    DeviceId, Measurement, MeasurementSource, MeasurementValue, Tolerance, Unit,
};

use crate::check::{evaluate, Expectation};
use crate::definition::{CommissioningTest, ExecutionMode, TestError, TestId, TestRequirements};
use crate::result::TestResult;
use crate::status::TestStatus;
use crate::test_kind::TestKind;

/// A driver shared between the tests that exercise one device.
pub type SharedDriver = Arc<Mutex<dyn DeviceDriver + Send>>;

/// Wrap a driver for sharing between tests.
pub fn share<D: DeviceDriver + Send + 'static>(driver: D) -> SharedDriver {
    Arc::new(Mutex::new(driver))
}

fn lock(
    driver: &SharedDriver,
) -> Result<std::sync::MutexGuard<'_, dyn DeviceDriver + Send + 'static>, TestError> {
    driver
        .lock()
        .map_err(|_| TestError::Internal("driver mutex poisoned".to_owned()))
}

fn result(id: &TestId, status: TestStatus, started: chrono::DateTime<Utc>) -> TestResult {
    let mut r = TestResult::new(id.clone(), status, ExecutionMode::Automated);
    r.started_at = started;
    r.completed_at = Utc::now();
    r
}

fn not_applicable(id: &TestId, started: chrono::DateTime<Utc>) -> TestResult {
    let mut r = result(id, TestStatus::Skipped, started);
    r.messages
        .push("not applicable: the driver does not support this operation".to_owned());
    r
}

fn millis(ms: u128) -> Measurement {
    let mut m = Measurement::new("response_time_ms", MeasurementValue::Integer(ms as i64));
    m.unit = Some(Unit::MS);
    m.source = MeasurementSource::Computed;
    m
}

// ---------------------------------------------------------------------------
// Connectivity (§13.1)
// ---------------------------------------------------------------------------

/// Which protocol a [`ConnectivityTest`] is probing (label only; the driver
/// owns the protocol).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ConnectivityProbe {
    Tcp,
    Udp,
    Http,
    Osc,
    Midi,
    Serial,
    Other(String),
}

impl ConnectivityProbe {
    fn label(&self) -> &str {
        match self {
            ConnectivityProbe::Tcp => "TCP",
            ConnectivityProbe::Udp => "UDP",
            ConnectivityProbe::Http => "HTTP",
            ConnectivityProbe::Osc => "OSC",
            ConnectivityProbe::Midi => "MIDI",
            ConnectivityProbe::Serial => "serial",
            ConnectivityProbe::Other(s) => s,
        }
    }
}

/// The driver can reach the device and get a well-formed reply.
pub struct ConnectivityTest {
    id: TestId,
    name: String,
    probe: ConnectivityProbe,
    driver: SharedDriver,
    requirements: TestRequirements,
}

impl ConnectivityTest {
    pub fn new(
        id: impl Into<String>,
        device: DeviceId,
        probe: ConnectivityProbe,
        driver: SharedDriver,
    ) -> Self {
        let name = format!("{} connectivity: {}", probe.label(), device);
        Self {
            id: TestId::new(id),
            name,
            probe,
            driver,
            requirements: TestRequirements::reads([device]),
        }
    }
}

impl CommissioningTest for ConnectivityTest {
    fn id(&self) -> &TestId {
        &self.id
    }
    fn name(&self) -> &str {
        &self.name
    }
    fn kind(&self) -> Option<TestKind> {
        Some(TestKind::Connectivity)
    }
    fn requirements(&self) -> &TestRequirements {
        &self.requirements
    }

    fn execute(&self) -> Result<TestResult, TestError> {
        let started = Utc::now();
        let timer = Instant::now();
        let probed = lock(&self.driver)?.discover();
        let elapsed = timer.elapsed().as_millis();
        match probed {
            Ok(_) => {
                let mut r = result(&self.id, TestStatus::Pass, started)
                    .message(format!("{} endpoint responded", self.probe.label()));
                r.measurements.push(millis(elapsed));
                Ok(r)
            }
            Err(DriverError::UnsupportedOperation) => Ok(not_applicable(&self.id, started)),
            // A configuration error is a bug in the project, not a finding.
            Err(e @ DriverError::Config(_)) => Err(e.into()),
            // Everything else means the device did not answer properly: the
            // finding this test exists to report.
            Err(e) => {
                let mut r = result(&self.id, TestStatus::Fail, started)
                    .message(format!("{} endpoint not usable: {e}", self.probe.label()));
                r.error = Some(e.to_string());
                Ok(r)
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Read-only state checks (identity, power state, video, audio, network, sync)
// ---------------------------------------------------------------------------

/// Read device state and verify it against expectations.
pub struct StateCheckTest {
    id: TestId,
    name: String,
    kind: TestKind,
    driver: SharedDriver,
    expectations: Vec<Expectation>,
    requirements: TestRequirements,
}

impl StateCheckTest {
    pub fn new(
        id: impl Into<String>,
        name: impl Into<String>,
        kind: TestKind,
        device: DeviceId,
        driver: SharedDriver,
    ) -> Self {
        Self {
            id: TestId::new(id),
            name: name.into(),
            kind,
            driver,
            expectations: Vec::new(),
            requirements: TestRequirements::reads([device]),
        }
    }

    /// Add an expectation.
    pub fn expect(mut self, expectation: Expectation) -> Self {
        self.expectations.push(expectation);
        self
    }

    /// Declare ordering dependencies (§14).
    pub fn depends_on(mut self, deps: impl IntoIterator<Item = TestId>) -> Self {
        self.requirements.depends_on.extend(deps);
        self
    }

    pub fn expectations(&self) -> &[Expectation] {
        &self.expectations
    }
}

impl CommissioningTest for StateCheckTest {
    fn id(&self) -> &TestId {
        &self.id
    }
    fn name(&self) -> &str {
        &self.name
    }
    fn kind(&self) -> Option<TestKind> {
        Some(self.kind)
    }
    fn requirements(&self) -> &TestRequirements {
        &self.requirements
    }

    fn execute(&self) -> Result<TestResult, TestError> {
        let started = Utc::now();
        let state = match lock(&self.driver)?.get_state() {
            Ok(s) => s,
            Err(DriverError::UnsupportedOperation) => return Ok(not_applicable(&self.id, started)),
            Err(e) => return Err(e.into()),
        };
        let eval = evaluate(&state, &self.expectations);
        let mut r = result(&self.id, eval.status, started);
        r.measurements = eval.measurements;
        r.messages = eval.messages;
        Ok(r)
    }
}

/// Identity check (§13.2): manufacturer, model, serial, firmware and,
/// optionally, the address the device must be found at. Detects the wrong
/// device having been installed. Comparison is trimmed and case-insensitive.
pub fn identity(
    id: impl Into<String>,
    device: DeviceId,
    driver: SharedDriver,
    expected: &DeviceIdentity,
    expected_address: Option<&str>,
) -> StateCheckTest {
    let mut t = StateCheckTest::new(
        id,
        format!("Device identity: {device}"),
        TestKind::Identity,
        device,
        Arc::new(Mutex::new(IdentityView { inner: driver })),
    );
    for (field, want) in [
        ("manufacturer", &expected.manufacturer),
        ("model", &expected.model),
        ("serial", &expected.serial_number),
        ("firmware", &expected.firmware),
    ] {
        if let Some(want) = want {
            t = t.expect(Expectation::text(field, want.clone()));
        }
    }
    if let Some(addr) = expected_address {
        t = t.expect(Expectation::text("device_address", addr));
    }
    t
}

/// Presents a driver's reported identity as state fields so identity checks
/// reuse the expectation engine.
struct IdentityView {
    inner: SharedDriver,
}

impl IdentityView {
    fn view(&self) -> Result<DeviceState, DriverError> {
        let mut drv = self
            .inner
            .lock()
            .map_err(|_| DriverError::Other("driver mutex poisoned".to_owned()))?;
        let mut state = drv.discover()?;
        let id = drv.identity();
        for (field, value) in [
            ("manufacturer", id.manufacturer),
            ("model", id.model),
            ("serial", id.serial_number),
            ("firmware", id.firmware),
        ] {
            if let Some(v) = value {
                state.set(field, v);
            }
        }
        // The address is whatever the device reports; an absent field leaves
        // the address check `Inconclusive` rather than a guess.
        if state.get("device_address").is_none() {
            if let Some(StateValue::Text(ip)) = state.get("ip_address").cloned() {
                state.set("device_address", ip);
            }
        }
        Ok(state)
    }
}

impl DeviceDriver for IdentityView {
    fn identity(&self) -> DeviceIdentity {
        self.inner.lock().map(|d| d.identity()).unwrap_or_default()
    }
    fn discover(&mut self) -> Result<DeviceState, DriverError> {
        self.view()
    }
    fn get_state(&mut self) -> Result<DeviceState, DriverError> {
        self.view()
    }
    fn execute(
        &mut self,
        _command: DeviceCommand,
    ) -> Result<tpt_app_av_commissioning_device::DeviceResponse, DriverError> {
        Err(DriverError::UnsupportedOperation)
    }
    fn capabilities(&self) -> tpt_app_av_commissioning_device::DeviceCapabilities {
        tpt_app_av_commissioning_device::DeviceCapabilities::read_only()
    }
}

/// Power state feedback (§13.3): the device reports it is on / off.
pub fn power_state(
    id: impl Into<String>,
    device: DeviceId,
    driver: SharedDriver,
    on: bool,
) -> StateCheckTest {
    StateCheckTest::new(
        id,
        format!("Power state is {}: {device}", if on { "on" } else { "off" }),
        TestKind::Power,
        device,
        driver,
    )
    .expect(Expectation::boolean("power", on))
}

/// Verify an expected route (§13.4), e.g. `in3->out1`.
pub fn expected_route(
    id: impl Into<String>,
    device: DeviceId,
    driver: SharedDriver,
    route: &str,
) -> StateCheckTest {
    StateCheckTest::new(
        id,
        format!("Expected route {route}: {device}"),
        TestKind::InputOutput,
        device,
        driver,
    )
    .expect(Expectation::text("route", route))
}

/// Verify a signal arrives at a device (§13.4).
pub fn signal_present(
    id: impl Into<String>,
    device: DeviceId,
    driver: SharedDriver,
) -> StateCheckTest {
    StateCheckTest::new(
        id,
        format!("Signal present: {device}"),
        TestKind::InputOutput,
        device,
        driver,
    )
    .expect(Expectation::boolean("signal_present", true))
}

/// Video expectations (§13.5). Unset fields are not checked.
#[derive(Debug, Clone, Default)]
pub struct VideoSpec {
    /// e.g. `3840x2160`.
    pub resolution: Option<String>,
    pub frame_rate: Option<(f64, Tolerance)>,
    pub colour_format: Option<String>,
    pub hdr_state: Option<String>,
    pub signal_lock: Option<bool>,
}

/// Video signal check against `spec`.
pub fn video(
    id: impl Into<String>,
    device: DeviceId,
    driver: SharedDriver,
    spec: &VideoSpec,
) -> StateCheckTest {
    let mut t = StateCheckTest::new(
        id,
        format!("Video signal: {device}"),
        TestKind::Video,
        device,
        driver,
    );
    if let Some(v) = &spec.resolution {
        t = t.expect(Expectation::text("resolution", v.clone()));
    }
    if let Some((v, tol)) = spec.frame_rate {
        t = t.expect(Expectation::number("frame_rate", v, tol).unit(Unit::new("fps")));
    }
    if let Some(v) = &spec.colour_format {
        t = t.expect(Expectation::text("colour_format", v.clone()));
    }
    if let Some(v) = &spec.hdr_state {
        t = t.expect(Expectation::text("hdr_state", v.clone()));
    }
    if let Some(v) = spec.signal_lock {
        t = t.expect(Expectation::boolean("signal_lock", v));
    }
    t
}

/// Audio expectations (§13.6). Unset fields are not checked. Level fields use
/// dBFS; phase is degrees; `max_noise_dbfs` is the noise-floor ceiling.
#[derive(Debug, Clone, Default)]
pub struct AudioSpec {
    pub channel_presence: Option<bool>,
    pub level_dbfs: Option<(f64, f64)>,
    pub max_silence_dbfs: Option<f64>,
    pub polarity: Option<String>,
    pub phase_degrees: Option<(f64, f64)>,
    pub clipping: Option<bool>,
    pub max_noise_dbfs: Option<f64>,
}

/// Audio check against `spec`.
pub fn audio(
    id: impl Into<String>,
    device: DeviceId,
    driver: SharedDriver,
    spec: &AudioSpec,
) -> StateCheckTest {
    let mut t = StateCheckTest::new(
        id,
        format!("Audio: {device}"),
        TestKind::Audio,
        device,
        driver,
    );
    if let Some(v) = spec.channel_presence {
        t = t.expect(Expectation::boolean("channel_presence", v));
    }
    if let Some((lo, hi)) = spec.level_dbfs {
        t = t.expect(Expectation::between("level", lo, hi).unit(Unit::DBFS));
    }
    if let Some(v) = spec.max_silence_dbfs {
        t = t.expect(Expectation::at_most("silence", v).unit(Unit::DBFS));
    }
    if let Some(v) = &spec.polarity {
        t = t.expect(Expectation::text("polarity", v.clone()));
    }
    if let Some((expected, tol)) = spec.phase_degrees {
        t = t.expect(
            Expectation::number("phase", expected, Tolerance::Absolute { delta: tol })
                .unit(Unit::new("deg")),
        );
    }
    if let Some(v) = spec.clipping {
        t = t.expect(Expectation::boolean("clipping", v));
    }
    if let Some(v) = spec.max_noise_dbfs {
        t = t.expect(Expectation::at_most("noise", v).unit(Unit::DBFS));
    }
    t
}

/// Network expectations (§13.9). Unset fields are not checked.
#[derive(Debug, Clone, Default)]
pub struct NetworkSpec {
    pub ip_address: Option<String>,
    pub gateway: Option<String>,
    pub dns: Option<String>,
    pub max_latency_ms: Option<f64>,
    pub max_packet_loss_percent: Option<f64>,
    pub link_up: Option<bool>,
}

/// Device-reported network configuration check against `spec`. Read-only and
/// non-intrusive: it asks the device what it sees, it sends no probes.
pub fn network(
    id: impl Into<String>,
    device: DeviceId,
    driver: SharedDriver,
    spec: &NetworkSpec,
) -> StateCheckTest {
    let mut t = StateCheckTest::new(
        id,
        format!("Network configuration: {device}"),
        TestKind::Network,
        device,
        driver,
    );
    if let Some(v) = &spec.ip_address {
        t = t.expect(Expectation::text("ip_address", v.clone()));
    }
    if let Some(v) = &spec.gateway {
        t = t.expect(Expectation::text("gateway", v.clone()));
    }
    if let Some(v) = &spec.dns {
        t = t.expect(Expectation::text("dns", v.clone()));
    }
    if let Some(v) = spec.max_latency_ms {
        t = t.expect(Expectation::at_most("latency_ms", v).unit(Unit::MS));
    }
    if let Some(v) = spec.max_packet_loss_percent {
        t = t.expect(Expectation::at_most("packet_loss", v).unit(Unit::PERCENT));
    }
    if let Some(v) = spec.link_up {
        t = t.expect(Expectation::boolean("link_state", v));
    }
    t
}

/// Synchronisation expectations (§13.8). Offsets are milliseconds; clock
/// drift is parts per million. Unset fields are not checked.
#[derive(Debug, Clone, Default)]
pub struct SyncSpec {
    /// Maximum absolute audio/video offset.
    pub max_av_offset_ms: Option<f64>,
    /// Maximum absolute cross-device timing offset.
    pub max_device_offset_ms: Option<f64>,
    /// Maximum absolute clock drift.
    pub max_clock_drift_ppm: Option<f64>,
}

/// Synchronisation check against `spec`.
pub fn synchronization(
    id: impl Into<String>,
    device: DeviceId,
    driver: SharedDriver,
    spec: &SyncSpec,
) -> StateCheckTest {
    let mut t = StateCheckTest::new(
        id,
        format!("Synchronisation: {device}"),
        TestKind::Synchronization,
        device,
        driver,
    );
    if let Some(v) = spec.max_av_offset_ms {
        t = t.expect(Expectation::between("av_offset", -v.abs(), v.abs()).unit(Unit::MS));
    }
    if let Some(v) = spec.max_device_offset_ms {
        t = t.expect(Expectation::between("sync_offset", -v.abs(), v.abs()).unit(Unit::MS));
    }
    if let Some(v) = spec.max_clock_drift_ppm {
        t = t.expect(Expectation::between("clock_drift", -v.abs(), v.abs()).unit(Unit::new("ppm")));
    }
    t
}

// ---------------------------------------------------------------------------
// Mutating command tests (power, input/output, control feedback)
// ---------------------------------------------------------------------------

/// Send commands, wait for the device to settle, then read the state back and
/// verify it — the command → device → expected state → feedback chain of
/// §13.7. The state is re-read independently of the command response, so a
/// device that acknowledges a command but does not act on it fails.
pub struct CommandTest {
    id: TestId,
    name: String,
    kind: TestKind,
    driver: SharedDriver,
    commands: Vec<DeviceCommand>,
    settle: Duration,
    expectations: Vec<Expectation>,
    requirements: TestRequirements,
    confirmed: bool,
}

impl CommandTest {
    pub fn new(
        id: impl Into<String>,
        name: impl Into<String>,
        kind: TestKind,
        device: DeviceId,
        driver: SharedDriver,
    ) -> Self {
        let mut requirements = TestRequirements::reads([device.clone()]);
        requirements.mutate_devices.push(device);
        Self {
            id: TestId::new(id),
            name: name.into(),
            kind,
            driver,
            commands: Vec::new(),
            settle: Duration::ZERO,
            expectations: Vec::new(),
            requirements,
            confirmed: false,
        }
    }

    /// Append a command to send.
    pub fn command(mut self, command: DeviceCommand) -> Self {
        self.commands.push(command);
        self
    }

    /// Add a state expectation checked after the commands.
    pub fn expect(mut self, expectation: Expectation) -> Self {
        self.expectations.push(expectation);
        self
    }

    /// Wait this long after the last command before reading state back.
    pub fn settle(mut self, settle: Duration) -> Self {
        self.settle = settle;
        self.requirements.max_duration = Some(settle + Duration::from_secs(30));
        self
    }

    /// Declare ordering dependencies (§14).
    pub fn depends_on(mut self, deps: impl IntoIterator<Item = TestId>) -> Self {
        self.requirements.depends_on.extend(deps);
        self
    }

    /// Confirm a disruptive (power-cycle) test may run (§36).
    pub fn confirm(mut self) -> Self {
        self.confirmed = true;
        self
    }

    /// Whether this test must be explicitly confirmed before it runs.
    pub fn needs_confirmation(&self) -> bool {
        self.commands
            .iter()
            .any(|c| matches!(c, DeviceCommand::PowerCycle))
    }
}

impl CommissioningTest for CommandTest {
    fn id(&self) -> &TestId {
        &self.id
    }
    fn name(&self) -> &str {
        &self.name
    }
    fn kind(&self) -> Option<TestKind> {
        Some(self.kind)
    }
    fn requirements(&self) -> &TestRequirements {
        &self.requirements
    }

    fn execute(&self) -> Result<TestResult, TestError> {
        let started = Utc::now();
        if self.needs_confirmation() && !self.confirmed {
            return Ok(result(&self.id, TestStatus::Blocked, started)
                .message("power-cycle tests require explicit confirmation; no command was sent"));
        }

        let mut drv = lock(&self.driver)?;
        let mut measurements = Vec::new();
        let mut messages = Vec::new();
        for command in &self.commands {
            match drv.execute(command.clone()) {
                Ok(resp) if resp.ok => {
                    if let Some(ms) = resp.response_time_ms {
                        measurements.push(millis(u128::from(ms)));
                    }
                    messages.push(format!("{command:?} acknowledged"));
                }
                Ok(resp) => {
                    let why = resp.message.unwrap_or_else(|| "no detail".to_owned());
                    let mut r = result(&self.id, TestStatus::Fail, started)
                        .message(format!("device rejected {command:?}: {why}"));
                    r.measurements = measurements;
                    return Ok(r);
                }
                Err(DriverError::UnsupportedOperation) => {
                    return Ok(not_applicable(&self.id, started))
                }
                Err(e) => return Err(e.into()),
            }
        }

        if !self.settle.is_zero() {
            std::thread::sleep(self.settle);
        }

        let state = match drv.get_state() {
            Ok(s) => s,
            Err(DriverError::UnsupportedOperation) => return Ok(not_applicable(&self.id, started)),
            Err(e) => return Err(e.into()),
        };
        let eval = evaluate(&state, &self.expectations);
        measurements.extend(eval.measurements);
        messages.extend(eval.messages);
        let mut r = result(&self.id, eval.status, started);
        r.measurements = measurements;
        r.messages = messages;
        Ok(r)
    }
}

/// Power on and verify the device reports it (§13.3).
pub fn power_on(id: impl Into<String>, device: DeviceId, driver: SharedDriver) -> CommandTest {
    CommandTest::new(
        id,
        format!("Power on: {device}"),
        TestKind::Power,
        device,
        driver,
    )
    .command(DeviceCommand::PowerOn)
    .expect(Expectation::boolean("power", true))
}

/// Power off and verify the device reports it (§13.3).
pub fn power_off(id: impl Into<String>, device: DeviceId, driver: SharedDriver) -> CommandTest {
    CommandTest::new(
        id,
        format!("Power off: {device}"),
        TestKind::Power,
        device,
        driver,
    )
    .command(DeviceCommand::PowerOff)
    .expect(Expectation::boolean("power", false))
}

/// Power-cycle and verify the device recovers to on (§13.3 power recovery).
/// Disruptive: blocked until [`CommandTest::confirm`] is called (§36).
pub fn power_recovery(
    id: impl Into<String>,
    device: DeviceId,
    driver: SharedDriver,
) -> CommandTest {
    CommandTest::new(
        id,
        format!("Power recovery after cycle: {device}"),
        TestKind::Power,
        device,
        driver,
    )
    .command(DeviceCommand::PowerCycle)
    .expect(Expectation::boolean("power", true))
}

/// Select an input and verify it took, with a signal present (§13.4).
pub fn select_input(
    id: impl Into<String>,
    device: DeviceId,
    driver: SharedDriver,
    input: &str,
) -> CommandTest {
    CommandTest::new(
        id,
        format!("Select input {input}: {device}"),
        TestKind::InputOutput,
        device,
        driver,
    )
    .command(DeviceCommand::SetInput {
        input: input.to_owned(),
    })
    .expect(Expectation::text("input", input))
    .expect(Expectation::boolean("signal_present", true))
}

/// Set a route and verify it (§13.4).
pub fn set_route(
    id: impl Into<String>,
    device: DeviceId,
    driver: SharedDriver,
    source: &str,
    destination: &str,
) -> CommandTest {
    CommandTest::new(
        id,
        format!("Route {source}->{destination}: {device}"),
        TestKind::InputOutput,
        device,
        driver,
    )
    .command(DeviceCommand::SetRoute {
        source: source.to_owned(),
        destination: destination.to_owned(),
    })
    .expect(Expectation::text(
        "route",
        format!("{source}->{destination}"),
    ))
}

/// Show a test pattern and verify the device acknowledges it (§13.5).
pub fn test_pattern_response(
    id: impl Into<String>,
    device: DeviceId,
    driver: SharedDriver,
    pattern: &str,
) -> CommandTest {
    CommandTest::new(
        id,
        format!("Test pattern {pattern}: {device}"),
        TestKind::Video,
        device,
        driver,
    )
    .command(DeviceCommand::GenerateTestPattern {
        pattern: pattern.to_owned(),
    })
    .expect(Expectation::text("test_pattern", pattern))
}

/// Control feedback (§13.7): send `command`, verify `expectation` on the
/// device's own state.
pub fn control_feedback(
    id: impl Into<String>,
    device: DeviceId,
    driver: SharedDriver,
    command: DeviceCommand,
    expectation: Expectation,
) -> CommandTest {
    CommandTest::new(
        id,
        format!("Control feedback: {device}"),
        TestKind::Control,
        device,
        driver,
    )
    .command(command)
    .expect(expectation)
}

#[cfg(test)]
mod tests {
    use super::*;
    use tpt_app_av_commissioning_testkit::{Fault, MockDevice};

    fn dev(name: &str) -> DeviceId {
        DeviceId::new(name)
    }

    #[test]
    fn connectivity_passes_against_reachable_device() {
        let t = ConnectivityTest::new(
            "c1",
            dev("p1"),
            ConnectivityProbe::Osc,
            share(MockDevice::projector()),
        );
        let r = t.execute().unwrap();
        assert_eq!(r.status, TestStatus::Pass);
        assert_eq!(t.kind(), Some(TestKind::Connectivity));
    }

    #[test]
    fn connectivity_unreachable_is_a_fail_finding() {
        let mut m = MockDevice::projector();
        m.set_fault(Fault::Unreachable);
        let t = ConnectivityTest::new("c1", dev("p1"), ConnectivityProbe::Tcp, share(m));
        let r = t.execute().unwrap();
        assert_eq!(r.status, TestStatus::Fail);
        assert!(r.error.is_some());
    }

    #[test]
    fn identity_matches_and_detects_wrong_device() {
        let expected = MockDevice::projector().identity;
        let t = identity(
            "i1",
            dev("p1"),
            share(MockDevice::projector()),
            &expected,
            None,
        );
        assert_eq!(t.execute().unwrap().status, TestStatus::Pass);

        let wrong = DeviceIdentity {
            model: Some("Beamer 9999".into()),
            ..expected
        };
        let t = identity(
            "i2",
            dev("p1"),
            share(MockDevice::projector()),
            &wrong,
            None,
        );
        let r = t.execute().unwrap();
        assert_eq!(r.status, TestStatus::Fail);
        assert_eq!(t.kind(), Some(TestKind::Identity));
    }

    #[test]
    fn identity_address_absent_is_inconclusive() {
        let expected = MockDevice::projector().identity;
        let t = identity(
            "i1",
            dev("p1"),
            share(MockDevice::projector()),
            &expected,
            Some("10.0.0.5"),
        );
        assert_eq!(t.execute().unwrap().status, TestStatus::Inconclusive);
    }

    #[test]
    fn power_on_verifies_state_feedback() {
        let t = power_on("pw1", dev("p1"), share(MockDevice::projector()));
        assert_eq!(t.execute().unwrap().status, TestStatus::Pass);
        assert!(t.requirements().mutating(&dev("p1")));
    }

    #[test]
    fn projector_ignoring_power_fails_despite_ack() {
        let mut m = MockDevice::projector();
        m.set_fault(Fault::IgnorePower);
        let t = power_on("pw1", dev("p1"), share(m));
        let r = t.execute().unwrap();
        assert_eq!(r.status, TestStatus::Fail);
    }

    #[test]
    fn power_cycle_blocked_until_confirmed() {
        let drv = share(MockDevice::projector());
        let t = power_recovery("pw2", dev("p1"), drv.clone());
        let r = t.execute().unwrap();
        assert_eq!(r.status, TestStatus::Blocked);
        // Nothing was sent: the projector is still off.
        assert_eq!(
            drv.lock().unwrap().get_state().unwrap().get("power"),
            Some(&tpt_app_av_commissioning_device::StateValue::Boolean(false))
        );
        let r = power_recovery("pw2", dev("p1"), drv)
            .confirm()
            .execute()
            .unwrap();
        assert_eq!(r.status, TestStatus::Pass);
    }

    #[test]
    fn power_state_read_only() {
        let t = power_state("ps", dev("d"), share(MockDevice::display()), true);
        assert_eq!(t.execute().unwrap().status, TestStatus::Pass);
        assert!(t.requirements().mutate_devices.is_empty());
        let t = power_state("ps", dev("d"), share(MockDevice::display()), false);
        assert_eq!(t.execute().unwrap().status, TestStatus::Fail);
    }

    #[test]
    fn select_input_and_route() {
        let t = select_input("io1", dev("d"), share(MockDevice::display()), "hdmi2");
        assert_eq!(t.execute().unwrap().status, TestStatus::Pass);
        let t = set_route("io2", dev("m"), share(MockDevice::matrix()), "in2", "out4");
        assert_eq!(t.execute().unwrap().status, TestStatus::Pass);
        let t = expected_route("io3", dev("m"), share(MockDevice::matrix()), "in3->out1");
        assert_eq!(t.execute().unwrap().status, TestStatus::Pass);
        let t = expected_route("io4", dev("m"), share(MockDevice::matrix()), "in1->out1");
        assert_eq!(t.execute().unwrap().status, TestStatus::Fail);
        let t = signal_present("io5", dev("p"), share(MockDevice::projector()));
        assert_eq!(t.execute().unwrap().status, TestStatus::Fail);
    }

    #[test]
    fn video_check_catches_wrong_resolution() {
        let spec = VideoSpec {
            resolution: Some("3840x2160".into()),
            frame_rate: Some((60.0, Tolerance::Exact)),
            ..VideoSpec::default()
        };
        let t = video("v1", dev("d"), share(MockDevice::display()), &spec);
        assert_eq!(t.execute().unwrap().status, TestStatus::Pass);
        let t = video("v2", dev("p"), share(MockDevice::projector()), &spec);
        assert_eq!(t.execute().unwrap().status, TestStatus::Fail);
        assert_eq!(t.kind(), Some(TestKind::Video));
    }

    #[test]
    fn test_pattern_response_verified() {
        let t = test_pattern_response("v3", dev("d"), share(MockDevice::display()), "colour-bars");
        assert_eq!(t.execute().unwrap().status, TestStatus::Pass);
    }

    #[test]
    fn audio_levels_checked_against_range() {
        let spec = AudioSpec {
            level_dbfs: Some((-6.0, 0.0)),
            ..AudioSpec::default()
        };
        let mut m = MockDevice::dsp();
        m.state.set("level", -3.0);
        let t = audio("a1", dev("dsp"), share(m), &spec);
        assert_eq!(t.execute().unwrap().status, TestStatus::Pass);

        let mut m = MockDevice::dsp();
        m.state.set("level", -30.0);
        let t = audio("a2", dev("dsp"), share(m), &spec);
        assert_eq!(t.execute().unwrap().status, TestStatus::Fail);
        assert_eq!(t.kind(), Some(TestKind::Audio));
    }

    #[test]
    fn network_and_sync_specs() {
        let mut m = MockDevice::display();
        m.state.set("ip_address", "10.0.0.20");
        m.state.set("latency_ms", 4.0);
        m.state.set("packet_loss", 0.0);
        m.state.set("link_state", true);
        let spec = NetworkSpec {
            ip_address: Some("10.0.0.20".into()),
            max_latency_ms: Some(10.0),
            max_packet_loss_percent: Some(0.1),
            link_up: Some(true),
            ..NetworkSpec::default()
        };
        let t = network("n1", dev("d"), share(m), &spec);
        assert_eq!(t.execute().unwrap().status, TestStatus::Pass);

        let mut m = MockDevice::display();
        m.state.set("av_offset", 80.0);
        let spec = SyncSpec {
            max_av_offset_ms: Some(40.0),
            ..SyncSpec::default()
        };
        let t = synchronization("s1", dev("d"), share(m), &spec);
        assert_eq!(t.execute().unwrap().status, TestStatus::Fail);
        assert_eq!(t.kind(), Some(TestKind::Synchronization));
    }

    #[test]
    fn control_feedback_detects_incorrect_feedback() {
        let t = control_feedback(
            "ct1",
            dev("p"),
            share(MockDevice::projector()),
            DeviceCommand::PowerOn,
            Expectation::boolean("power", true),
        );
        assert_eq!(t.execute().unwrap().status, TestStatus::Pass);

        let mut m = MockDevice::projector();
        m.set_fault(Fault::IncorrectField {
            field: "power".into(),
            value: "off".into(),
        });
        let t = control_feedback(
            "ct2",
            dev("p"),
            share(m),
            DeviceCommand::PowerOn,
            Expectation::boolean("power", true),
        );
        assert_eq!(t.execute().unwrap().status, TestStatus::Fail);
        assert_eq!(t.kind(), Some(TestKind::Control));
    }

    #[test]
    fn unsupported_operation_skips_instead_of_failing() {
        let mut m = MockDevice::projector();
        m.capabilities = tpt_app_av_commissioning_device::DeviceCapabilities::read_only();
        let t = power_on("pw", dev("p"), share(m));
        assert_eq!(t.execute().unwrap().status, TestStatus::Skipped);
    }

    #[test]
    fn driver_errors_other_than_unsupported_propagate() {
        let mut m = MockDevice::display();
        m.set_fault(Fault::Timeout(500));
        let t = power_state("ps", dev("d"), share(m), true);
        assert!(matches!(t.execute(), Err(TestError::Driver(_))));
    }

    #[test]
    fn no_expectations_is_inconclusive() {
        let t = CommandTest::new(
            "x",
            "x",
            TestKind::Control,
            dev("d"),
            share(MockDevice::display()),
        )
        .command(DeviceCommand::ReadState);
        assert_eq!(t.execute().unwrap().status, TestStatus::Inconclusive);
    }

    #[test]
    fn every_result_carries_the_test_id_and_timing() {
        let t = power_state("ps", dev("d"), share(MockDevice::display()), true);
        let r = t.execute().unwrap();
        assert_eq!(r.test_id.as_str(), "ps");
        assert!(r.completed_at >= r.started_at);
    }
}
