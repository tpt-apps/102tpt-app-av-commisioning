//! The `test` command (§34): run commissioning tests for one room against
//! the devices in the project manifest.
//!
//! Devices bind to drivers through versioned device profiles (`--profiles`,
//! matched by manufacturer and model, never by trust in the manifest); a
//! device without a matching profile that carries a `host:port` address
//! falls back to a bounded TCP reachability probe, and anything else is
//! reported as skipped — never silently passed.

use std::net::{IpAddr, SocketAddr};
use std::path::Path;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use tpt_app_av_commissioning_core::Manifest;
use tpt_app_av_commissioning_device::DeviceIdentity;
use tpt_app_av_commissioning_driver::DriverError;
use tpt_app_av_commissioning_driver_generic::http::{HttpDriver, HttpDriverConfig};
use tpt_app_av_commissioning_driver_generic::snmp::{SnmpDriver, SnmpDriverConfig};
use tpt_app_av_commissioning_driver_generic::{
    SerialDriver, SerialDriverConfig, TcpDriver, TcpDriverConfig, UdpDriver, UdpDriverConfig,
    WebSocketDriver, WebSocketDriverConfig,
};
use tpt_app_av_commissioning_driver_osc::{OscDriver, OscDriverConfig};
use tpt_app_av_commissioning_model::DeviceId;
use tpt_app_av_commissioning_profile::{DeviceProfile, ProtocolKind};
use tpt_app_av_commissioning_runner::{RunObserver, RunOptions, RunOutcome, TestExecutor};
use tpt_app_av_commissioning_test::{
    share, CommissioningTest, ConnectivityProbe, ConnectivityTest, ExecutionMode,
    TcpReachableTest, TestError, TestId, TestRequirements, TestResult, TestStatus,
};

const TEST_USAGE: &str = "\
test --project <file> --room <name|id> [--profiles <path>] [--dry-run]

  Runs commissioning tests for the devices assigned to a room. Devices bind
  to protocol drivers through device profiles (--profiles, a YAML file or a
  directory of them, matched by manufacturer and model); a device with only
  a `host:port` address gets a bounded TCP reachability probe, and anything
  unbindable is reported as skipped.

  --dry-run schedules and reports without touching devices.
";

/// A device the CLI could not bind to a driver: reported as skipped with
/// the reason, never passed.
struct UntestedDevice {
    id: TestId,
    name: String,
    reason: String,
}

impl CommissioningTest for UntestedDevice {
    fn id(&self) -> &TestId {
        &self.id
    }
    fn name(&self) -> &str {
        &self.name
    }
    fn requirements(&self) -> &TestRequirements {
        static REQ: std::sync::OnceLock<TestRequirements> = std::sync::OnceLock::new();
        REQ.get_or_init(TestRequirements::default)
    }
    fn execute(&self) -> Result<TestResult, TestError> {
        Ok(TestResult::new(self.id.clone(), TestStatus::Skipped, ExecutionMode::Automated)
            .message(self.reason.clone()))
    }
}

/// How a device was bound for the run.
enum Binding {
    /// Profile-bound protocol driver.
    Driver(Arc<Mutex<dyn tpt_app_av_commissioning_driver::DeviceDriver + Send>>, ConnectivityProbe),
    /// Fallback probe of a spelled-out `host:port`.
    TcpProbe(SocketAddr),
    /// Cannot be tested; the reason is reported as a skip.
    Unbound(String),
}

/// Progress printed per test, and counts for the exit code.
#[derive(Default)]
struct PrintObserver {
    failures: usize,
}

impl RunObserver for PrintObserver {
    fn on_test_completed(&mut self, result: &TestResult) {
        if result.status == TestStatus::Fail {
            self.failures += 1;
        }
        let first = result.messages.first().map(String::as_str).unwrap_or("");
        println!(
            "{:<8} {}{}",
            result.status.as_str(),
            result.test_id,
            if first.is_empty() {
                String::new()
            } else {
                format!(" — {first}")
            }
        );
    }
}

/// Entry point for `test`. `Ok(false)` maps to exit code 1 (a test failed).
pub fn run(args: &[String]) -> Result<bool, String> {
    let mut project: Option<String> = None;
    let mut room: Option<String> = None;
    let mut profiles: Option<String> = None;
    let mut dry_run = false;

    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "--project" => {
                i += 1;
                project = Some(value(args, i, "--project")?);
            }
            "--room" => {
                i += 1;
                room = Some(value(args, i, "--room")?);
            }
            "--profiles" => {
                i += 1;
                profiles = Some(value(args, i, "--profiles")?);
            }
            "--dry-run" => dry_run = true,
            "--help" | "-h" => {
                println!("{TEST_USAGE}");
                return Ok(true);
            }
            other => return Err(format!("unexpected argument `{other}` for `test`")),
        }
        i += 1;
    }
    let project = project.ok_or("`test` requires --project <file>")?;
    let room = room.ok_or("`test` requires --room <name>")?;

    let manifest = load_manifest(&project)?;
    let room_ids = rooms_matching(&manifest, &room)?;
    let devices: Vec<_> = manifest
        .devices
        .iter()
        .filter(|d| {
            d.room
                .as_deref()
                .map(|r| room_ids.iter().any(|id| id == r))
                .unwrap_or(false)
        })
        .collect();
    if devices.is_empty() {
        println!("no devices are assigned to room `{room}` in the manifest; nothing to test");
        return Ok(true);
    }

    let profiles = match &profiles {
        Some(path) => load_profiles(path)?,
        None => Vec::new(),
    };

    let tests: Vec<Box<dyn CommissioningTest + Send>> = devices
        .iter()
        .map(|device| bind_device(device, &profiles))
        .collect::<Result<_, _>>()?;

    println!(
        "room `{room}`: {} device(s), run{}:",
        tests.len(),
        if dry_run { " (dry run)" } else { "" }
    );
    let mut observer = PrintObserver::default();
    let options = RunOptions {
        dry_run,
        ..RunOptions::default()
    };
    let outcome = TestExecutor::new(options, tests, &mut observer)
        .execute()
        .map_err(|e| format!("run failed: {e}"))?;
    print_summary(&outcome);

    // A run fails when something actually failed; blocked and skipped tests
    // are visible in the summary but do not fail automation pipelines.
    Ok(observer.failures == 0)
}

fn value(args: &[String], i: usize, flag: &str) -> Result<String, String> {
    args.get(i)
        .cloned()
        .ok_or_else(|| format!("`{flag}` requires a value"))
}

fn load_manifest(path: &str) -> Result<Manifest, String> {
    let text =
        std::fs::read_to_string(path).map_err(|e| format!("could not read `{path}`: {e}"))?;
    if Manifest::contains_executable_content(&text) {
        return Err(format!("`{path}`: manifest contains executable content"));
    }
    Manifest::parse(&text).map_err(|e| format!("`{path}`: invalid manifest: {e}"))
}

/// Ids of the rooms matching `--room` (by room id or name).
fn rooms_matching(manifest: &Manifest, room: &str) -> Result<Vec<String>, String> {
    let ids: Vec<String> = manifest
        .rooms
        .iter()
        .filter(|r| r.id == room || r.name == room)
        .map(|r| r.id.clone())
        .collect();
    if ids.is_empty() {
        return Err(format!("room `{room}` does not exist in the manifest"));
    }
    Ok(ids)
}

fn load_profiles(path: &str) -> Result<Vec<DeviceProfile>, String> {
    let files: Vec<std::path::PathBuf> = if Path::new(path).is_dir() {
        let mut files: Vec<_> = std::fs::read_dir(path)
            .map_err(|e| format!("could not read profiles directory `{path}`: {e}"))?
            .filter_map(|e| e.ok())
            .map(|e| e.path())
            .filter(|p| p.extension().map(|x| x == "yaml" || x == "yml").unwrap_or(false))
            .collect();
        files.sort();
        files
    } else {
        vec![path.into()]
    };
    let mut out = Vec::new();
    for file in files {
        let text = std::fs::read_to_string(&file)
            .map_err(|e| format!("could not read `{}`: {e}", file.display()))?;
        let profile = DeviceProfile::from_yaml_str(&text)
            .map_err(|e| format!("`{}`: invalid device profile: {e}", file.display()))?;
        out.push(profile);
    }
    Ok(out)
}

/// The host portion of a manifest address (bare IP or `host:port`).
fn host_of(address: &str) -> Option<IpAddr> {
    if let Ok(ip) = address.parse::<IpAddr>() {
        return Some(ip);
    }
    address.parse::<SocketAddr>().ok().map(|a| a.ip())
}

fn host_arg(address: &str) -> Result<IpAddr, DriverError> {
    host_of(address).ok_or_else(|| {
        DriverError::Config(format!("address `{address}` is not an IP address"))
    })
}

fn bind_device(
    device: &tpt_app_av_commissioning_core::manifest::ManifestDevice,
    profiles: &[DeviceProfile],
) -> Result<Box<dyn CommissioningTest + Send>, String> {
    let identity = DeviceIdentity {
        manufacturer: device.manufacturer.clone(),
        model: device.model.clone(),
        serial_number: device.serial_number.clone(),
        firmware: device.firmware.clone(),
    };
    let profile = profiles.iter().find(|p| p.matches(&identity));
    let address = device.address.as_deref().unwrap_or("");

    let binding = match (profile, address) {
        (None, "") => Binding::Unbound("no matching profile and no address".to_owned()),
        (None, addr) => match addr.parse::<SocketAddr>() {
            Ok(sock) => Binding::TcpProbe(sock),
            Err(_) if host_of(addr).is_some() => Binding::Unbound(format!(
                "no matching profile and `{addr}` has no port to probe"
            )),
            Err(_) => Binding::Unbound(format!(
                "no matching profile and address `{addr}` is not an IP address"
            )),
        },
        (Some(profile), "") => Binding::Unbound(format!(
            "profile `{}` matches but the device has no address",
            profile.device.id
        )),
        (Some(profile), addr) => bind_with_profile(profile, addr),
    };

    let id = TestId::new(format!("connectivity.{}", device.id));
    match binding {
        Binding::Driver(driver, probe) => Ok(Box::new(ConnectivityTest::new(
            id.as_str(),
            DeviceId::new(&device.id),
            probe,
            driver,
        ))),
        Binding::TcpProbe(addr) => {
            Ok(match TcpReachableTest::new(
                id.as_str(),
                DeviceId::new(&device.id),
                addr,
                Duration::from_secs(2),
            ) {
                Ok(test) => Box::new(test),
                Err(e) => Box::new(UntestedDevice {
                    id,
                    name: format!("Connectivity: {}", device.name),
                    reason: format!("could not schedule probe: {e}"),
                }),
            })
        }
        Binding::Unbound(reason) => Ok(Box::new(UntestedDevice {
            id,
            name: format!("Connectivity: {}", device.name),
            reason,
        })),
    }
}

fn bind_with_profile(profile: &DeviceProfile, address: &str) -> Binding {
    let bind = |result: Result<Binding, DriverError>| match result {
        Ok(binding) => binding,
        Err(e) => Binding::Unbound(format!("profile `{}` could not bind: {e}", profile.device.id)),
    };
    match profile.device.protocol.kind {
        ProtocolKind::Osc => bind(host_arg(address).and_then(|host| {
            OscDriverConfig::from_profile(profile, host)
                .and_then(|cfg| OscDriver::new(cfg))
                .map(|d| Binding::Driver(share(d), ConnectivityProbe::Osc))
        })),
        ProtocolKind::Tcp => bind(host_arg(address).and_then(|host| {
            TcpDriverConfig::from_profile(profile, host)
                .and_then(|cfg| TcpDriver::new(cfg))
                .map(|d| Binding::Driver(share(d), ConnectivityProbe::Tcp))
        })),
        ProtocolKind::Udp => bind(host_arg(address).and_then(|host| {
            UdpDriverConfig::from_profile(profile, host)
                .and_then(|cfg| UdpDriver::new(cfg))
                .map(|d| Binding::Driver(share(d), ConnectivityProbe::Udp))
        })),
        ProtocolKind::Http => bind(host_arg(address).and_then(|host| {
            HttpDriverConfig::from_profile(profile, host)
                .and_then(|cfg| HttpDriver::new(cfg))
                .map(|d| Binding::Driver(share(d), ConnectivityProbe::Http))
        })),
        ProtocolKind::Websocket => bind(host_arg(address).and_then(|host| {
            WebSocketDriverConfig::from_profile(profile, host)
                .and_then(|cfg| WebSocketDriver::new(cfg))
                .map(|d| Binding::Driver(share(d), ConnectivityProbe::Other("websocket".to_owned())))
        })),
        ProtocolKind::Snmp => bind(host_arg(address).and_then(|host| {
            SnmpDriverConfig::from_profile(profile, host)
                .and_then(|cfg| SnmpDriver::new(cfg))
                .map(|d| Binding::Driver(share(d), ConnectivityProbe::Other("snmp".to_owned())))
        })),
        ProtocolKind::Serial => bind(
            // Serial binds by port path; the manifest address is the path.
            SerialDriverConfig::from_profile(profile, address)
                .and_then(|cfg| SerialDriver::new(cfg))
                .map(|d| Binding::Driver(share(d), ConnectivityProbe::Serial)),
        ),
        ProtocolKind::Midi => Binding::Unbound(
            "MIDI binds by enumerated port name; not yet supported by `test`".to_owned(),
        ),
    }
}

fn print_summary(outcome: &RunOutcome) {
    let mut counts: Vec<(TestStatus, usize)> = Vec::new();
    for result in &outcome.results {
        match counts.iter_mut().find(|(s, _)| *s == result.status) {
            Some((_, n)) => *n += 1,
            None => counts.push((result.status, 1)),
        }
    }
    counts.sort_by_key(|(s, _)| s.as_str());
    let parts: Vec<String> = counts
        .iter()
        .map(|(s, n)| format!("{n} {}", s.as_str()))
        .collect();
    println!("run {} complete: {}", outcome.run_id, parts.join(", "));
}
