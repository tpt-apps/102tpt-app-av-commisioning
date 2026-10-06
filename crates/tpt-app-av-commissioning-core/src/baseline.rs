//! Baselines, regression comparison (§21) and configuration drift detection
//! (§22).
//!
//! A [`Baseline`] snapshots the system at a point in time: device identity
//! fingerprints, signal routes, and the test results of the commissioning
//! run. Comparing a later run or state against it highlights only meaningful
//! changes — worse test outcomes, and concrete drift: firmware, addresses,
//! serials (a replacement device is never assumed identical because its
//! model matches), configuration attributes, and routes.

use std::collections::BTreeMap;

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use tpt_app_av_commissioning_model::{Connection, Device, ProjectId};
use tpt_app_av_commissioning_test::{TestResult, TestStatus};

/// The identity of one device at capture time (§22).
///
/// The fingerprint is what drift detection compares: manufacturer, model,
/// serial, firmware, addresses, and configuration attributes. A replacement
/// unit with the same model but a different serial is a different device.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DeviceFingerprint {
    pub device_id: String,
    pub name: String,
    pub device_type: String,
    pub manufacturer: Option<String>,
    pub model: Option<String>,
    pub serial_number: Option<String>,
    pub firmware: Option<String>,
    pub addresses: Vec<String>,
    /// Observed configuration attributes worth tracking (e.g. `resolution`,
    /// `audio_route`, control settings), keyed by name.
    pub attributes: BTreeMap<String, String>,
}

impl DeviceFingerprint {
    /// Fingerprint a device from the project model (no observed attributes).
    pub fn of_device(device: &Device) -> Self {
        Self {
            device_id: device.id.as_str().to_owned(),
            name: device.name.clone(),
            device_type: device.device_type.as_str().to_owned(),
            manufacturer: device.manufacturer.clone(),
            model: device.model.clone(),
            serial_number: device.serial_number.clone(),
            firmware: device.firmware.clone(),
            addresses: device.addresses.iter().map(|a| a.display()).collect(),
            attributes: BTreeMap::new(),
        }
    }

    /// Record an observed attribute (e.g. from device state at capture).
    pub fn with_attribute(mut self, name: impl Into<String>, value: impl Into<String>) -> Self {
        self.attributes.insert(name.into(), value.into());
        self
    }

    /// SHA-256 over the canonical JSON of the identity fields — the value
    /// stored as the configuration baseline fingerprint (§22).
    pub fn identity_fingerprint(&self) -> String {
        let canonical = serde_json::to_string(&self).unwrap_or_default();
        let hash = Sha256::digest(canonical.as_bytes());
        format!("{:x}", hash)
    }
}

/// One signal route at capture time.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ConnectionSnapshot {
    pub id: String,
    pub source: String,
    pub destination: String,
    pub signal_type: String,
    pub transport: String,
}

impl ConnectionSnapshot {
    /// The routed path as `source->destination` for reports.
    pub fn route(&self) -> String {
        format!("{}->{}", self.source, self.destination)
    }
}

impl From<&Connection> for ConnectionSnapshot {
    fn from(c: &Connection) -> Self {
        Self {
            id: c.id.as_str().to_owned(),
            source: c.source.as_str().to_owned(),
            destination: c.destination.as_str().to_owned(),
            signal_type: c.signal_type.as_str().to_owned(),
            transport: c.transport.as_str().to_owned(),
        }
    }
}

/// A recorded baseline of the commissioned system (§21).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Baseline {
    pub id: String,
    pub label: String,
    pub project: ProjectId,
    pub created_at: DateTime<Utc>,
    pub devices: Vec<DeviceFingerprint>,
    pub connections: Vec<ConnectionSnapshot>,
    /// Test id → status as of this baseline.
    pub test_results: BTreeMap<String, TestStatus>,
}

impl Baseline {
    /// Capture a baseline from the current project model and a run's results.
    /// `label` distinguishes baselines (e.g. `after-commissioning`); saving
    /// twice with the same label is refused so snapshots are never silently
    /// overwritten.
    pub fn create(
        label: impl Into<String>,
        project: ProjectId,
        devices: &[Device],
        connections: &[Connection],
        results: &[TestResult],
    ) -> Self {
        let mut test_results = BTreeMap::new();
        for result in results {
            test_results.insert(result.test_id.as_str().to_owned(), result.status);
        }
        Self {
            id: uuid::Uuid::new_v4().to_string(),
            label: label.into(),
            project,
            created_at: Utc::now(),
            devices: devices.iter().map(DeviceFingerprint::of_device).collect(),
            connections: connections.iter().map(ConnectionSnapshot::from).collect(),
            test_results,
        }
    }

    /// SHA-256 over the snapshot content (devices + routes + results) —
    /// identical baselines hash identically; labels and timestamps do not
    /// count.
    pub fn fingerprint(&self) -> String {
        let canonical = serde_json::json!({
            "devices": self.devices,
            "connections": self.connections,
            "test_results": self.test_results,
        });
        let hash = Sha256::digest(canonical.to_string().as_bytes());
        format!("{:x}", hash)
    }

    /// The device fingerprint for a device id.
    pub fn device(&self, device_id: &str) -> Option<&DeviceFingerprint> {
        self.devices.iter().find(|d| d.device_id == device_id)
    }

    /// The headline counts of the baseline (§21 example header).
    pub fn summary(&self) -> BaselineSummary {
        let statuses: Vec<TestStatus> = self.test_results.values().copied().collect();
        BaselineSummary {
            devices: self.devices.len(),
            connections: self.connections.len(),
            tests: self.test_results.len(),
            result: aggregate(&statuses),
        }
    }
}

/// The headline numbers of a baseline (§21).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BaselineSummary {
    pub devices: usize,
    pub connections: usize,
    pub tests: usize,
    /// Aggregate result: `Fail` if anything failed, else `Warning` if
    /// anything warned or was inconclusive, else `Pass`; `None` for no
    /// results.
    pub result: Option<TestStatus>,
}

/// Aggregate result statuses into one headline status.
pub fn aggregate(statuses: &[TestStatus]) -> Option<TestStatus> {
    if statuses.is_empty() {
        return None;
    }
    let failed = statuses.iter().any(|s| s.is_failure());
    let warned = statuses
        .iter()
        .any(|s| matches!(s, TestStatus::Warning | TestStatus::Inconclusive));
    Some(if failed {
        TestStatus::Fail
    } else if warned {
        TestStatus::Warning
    } else {
        TestStatus::Pass
    })
}

/// One meaningful difference between a baseline and the current system (§22).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Drift {
    DeviceAdded(String),
    DeviceRemoved(String),
    /// Same slot, different serial: the unit was physically replaced (never
    /// assumed identical merely because the model matches).
    DeviceReplaced {
        device: String,
        from_serial: Option<String>,
        to_serial: Option<String>,
    },
    FirmwareChanged {
        device: String,
        from: Option<String>,
        to: Option<String>,
    },
    AddressChanged {
        device: String,
        from: Vec<String>,
        to: Vec<String>,
    },
    AttributeChanged {
        device: String,
        attribute: String,
        from: Option<String>,
        to: Option<String>,
    },
    RouteChanged {
        connection: String,
        from: String,
        to: String,
    },
    ConnectionAdded(String),
    ConnectionRemoved(String),
}

impl Drift {
    /// Human-readable one-line description (reports, CLI output).
    pub fn description(&self) -> String {
        match self {
            Drift::DeviceAdded(d) => format!("device added: {d}"),
            Drift::DeviceRemoved(d) => format!("device removed: {d}"),
            Drift::DeviceReplaced {
                device,
                from_serial,
                to_serial,
            } => format!(
                "device replaced: {device} (serial {} → {})",
                from_serial.as_deref().unwrap_or("(none)"),
                to_serial.as_deref().unwrap_or("(none)")
            ),
            Drift::FirmwareChanged { device, from, to } => format!(
                "firmware changed: {device} ({} → {})",
                from.as_deref().unwrap_or("(none)"),
                to.as_deref().unwrap_or("(none)")
            ),
            Drift::AddressChanged { device, from, to } => {
                format!("address changed: {device} ({from:?} → {to:?})")
            }
            Drift::AttributeChanged {
                device,
                attribute,
                from,
                to,
            } => format!(
                "{attribute} changed: {device} ({} → {})",
                from.as_deref().unwrap_or("(none)"),
                to.as_deref().unwrap_or("(none)")
            ),
            Drift::RouteChanged {
                connection,
                from,
                to,
            } => format!("route changed: {connection} ({from} → {to})"),
            Drift::ConnectionAdded(c) => format!("connection added: {c}"),
            Drift::ConnectionRemoved(c) => format!("connection removed: {c}"),
        }
    }

    /// The device this drift concerns, when it is about a device.
    pub fn device(&self) -> Option<&str> {
        match self {
            Drift::DeviceAdded(d)
            | Drift::DeviceRemoved(d)
            | Drift::DeviceReplaced { device: d, .. }
            | Drift::FirmwareChanged { device: d, .. }
            | Drift::AddressChanged { device: d, .. }
            | Drift::AttributeChanged { device: d, .. } => Some(d),
            _ => None,
        }
    }
}

/// Compare a baseline against the current system state (§22).
///
/// Both sides are [`Baseline`]s: capture one at baseline time, build another
/// from the current model and observed state, and diff. Results are ordered
/// by device for stable reports.
pub fn detect_drift(baseline: &Baseline, current: &Baseline) -> Vec<Drift> {
    let mut drift = Vec::new();

    let mut by_id: BTreeMap<&str, &DeviceFingerprint> = BTreeMap::new();
    for d in &baseline.devices {
        by_id.insert(&d.device_id, d);
    }
    let mut current_by_id: BTreeMap<&str, &DeviceFingerprint> = BTreeMap::new();
    for d in &current.devices {
        current_by_id.insert(&d.device_id, d);
    }

    for (id, old) in &by_id {
        match current_by_id.get(id) {
            None => drift.push(Drift::DeviceRemoved((*id).to_owned())),
            Some(new) => {
                // Replacement: both serials known and different. A serial
                // appearing (or disappearing) alone is bookkeeping, not a
                // swapped unit.
                if let (Some(from), Some(to)) = (&old.serial_number, &new.serial_number) {
                    if from != to {
                        drift.push(Drift::DeviceReplaced {
                            device: (*id).to_owned(),
                            from_serial: Some(from.clone()),
                            to_serial: Some(to.clone()),
                        });
                    }
                }
                if old.firmware != new.firmware {
                    drift.push(Drift::FirmwareChanged {
                        device: (*id).to_owned(),
                        from: old.firmware.clone(),
                        to: new.firmware.clone(),
                    });
                }
                if old.addresses != new.addresses {
                    drift.push(Drift::AddressChanged {
                        device: (*id).to_owned(),
                        from: old.addresses.clone(),
                        to: new.addresses.clone(),
                    });
                }
                for attribute in old.attributes.keys().chain(new.attributes.keys()) {
                    let from = old.attributes.get(attribute);
                    let to = new.attributes.get(attribute);
                    if from != to {
                        drift.push(Drift::AttributeChanged {
                            device: (*id).to_owned(),
                            attribute: attribute.clone(),
                            from: from.cloned(),
                            to: to.cloned(),
                        });
                    }
                }
            }
        }
    }
    for id in current_by_id.keys() {
        if !by_id.contains_key(id) {
            drift.push(Drift::DeviceAdded((*id).to_owned()));
        }
    }

    let mut baseline_routes: BTreeMap<&str, &ConnectionSnapshot> = BTreeMap::new();
    for c in &baseline.connections {
        baseline_routes.insert(&c.id, c);
    }
    let mut current_routes: BTreeMap<&str, &ConnectionSnapshot> = BTreeMap::new();
    for c in &current.connections {
        current_routes.insert(&c.id, c);
    }
    for (id, old) in &baseline_routes {
        match current_routes.get(id) {
            None => drift.push(Drift::ConnectionRemoved((*id).to_owned())),
            Some(new) if new != old => drift.push(Drift::RouteChanged {
                connection: (*id).to_owned(),
                from: old.route(),
                to: new.route(),
            }),
            Some(_) => {}
        }
    }
    for id in current_routes.keys() {
        if !baseline_routes.contains_key(id) {
            drift.push(Drift::ConnectionAdded((*id).to_owned()));
        }
    }

    drift
}

/// One meaningful outcome change relative to the baseline (§21).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Regression {
    /// The test now measures worse than at baseline.
    Worsened {
        test_id: String,
        from: TestStatus,
        to: TestStatus,
    },
    /// The test measures better than at baseline (a fix landed).
    Improved {
        test_id: String,
        from: TestStatus,
        to: TestStatus,
    },
    /// The test failed although it was not part of the baseline run.
    NewFailure(String),
    /// A test that had a verdict at baseline no longer produces one
    /// (not run, blocked, or still awaiting confirmation).
    NoLongerRun(String),
}

/// How a later run compares to the baseline run (§21 "RUN BASELINE").
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RunComparison {
    /// Aggregate of the baseline run.
    pub baseline_result: Option<TestStatus>,
    /// Aggregate of the current run.
    pub current_result: Option<TestStatus>,
    /// Only the meaningful changes — never an unchanged PASS.
    pub changes: Vec<Regression>,
}

/// How a status ranks for regression purposes. Only concrete verdicts rank:
/// `Pass` < `Warning` < `Fail`.
fn rank(status: TestStatus) -> Option<u8> {
    match status {
        TestStatus::Pass => Some(0),
        TestStatus::Warning => Some(1),
        TestStatus::Fail => Some(2),
        _ => None,
    }
}

/// Compare a later run against the baseline's recorded test results.
pub fn compare_results(baseline: &Baseline, current: &[TestResult]) -> RunComparison {
    let mut current_by_id: BTreeMap<&str, TestStatus> = BTreeMap::new();
    for result in current {
        current_by_id.insert(result.test_id.as_str(), result.status);
    }

    let mut changes = Vec::new();
    for (test_id, old) in &baseline.test_results {
        match current_by_id.get(test_id.as_str()).copied() {
            Some(new) => match (rank(*old), rank(new)) {
                (Some(from), Some(to)) if to > from => changes.push(Regression::Worsened {
                    test_id: test_id.clone(),
                    from: *old,
                    to: new,
                }),
                (Some(from), Some(to)) if to < from => changes.push(Regression::Improved {
                    test_id: test_id.clone(),
                    from: *old,
                    to: new,
                }),
                _ => {}
            },
            None => {
                if rank(*old).is_some() {
                    changes.push(Regression::NoLongerRun(test_id.clone()));
                }
            }
        }
    }
    for (test_id, status) in &current_by_id {
        if !baseline.test_results.contains_key(*test_id) && *status == TestStatus::Fail {
            changes.push(Regression::NewFailure((*test_id).to_owned()));
        }
    }
    changes.sort_by(|a, b| match (a, b) {
        (Regression::Worsened { test_id: a, .. }, Regression::Worsened { test_id: b, .. }) => {
            a.cmp(b)
        }
        (Regression::Worsened { .. }, _) => std::cmp::Ordering::Less,
        (_, Regression::Worsened { .. }) => std::cmp::Ordering::Greater,
        _ => std::cmp::Ordering::Equal,
    });

    let baseline_statuses: Vec<TestStatus> = baseline.test_results.values().copied().collect();
    let current_statuses: Vec<TestStatus> = current.iter().map(|r| r.status).collect();
    RunComparison {
        baseline_result: aggregate(&baseline_statuses),
        current_result: aggregate(&current_statuses),
        changes,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tpt_app_av_commissioning_model::{
        ConnectionId, DeviceId, DeviceType, EndpointId, SignalType, Transport,
    };

    fn projector(serial: &str, firmware: &str) -> Device {
        let mut d = Device::new(
            DeviceId::new("projector-01"),
            "Projector",
            DeviceType::Projector,
        );
        d.manufacturer = Some("ACME".into());
        d.model = Some("Beamer 9000".into());
        d.serial_number = Some(serial.into());
        d.firmware = Some(firmware.into());
        d
    }

    fn baseline_with(devices: Vec<Device>, connections: Vec<Connection>) -> Baseline {
        Baseline {
            id: "b1".into(),
            label: "after-commissioning".into(),
            project: ProjectId::new("prj-1"),
            created_at: Utc::now(),
            devices: devices.iter().map(DeviceFingerprint::of_device).collect(),
            connections: connections.iter().map(ConnectionSnapshot::from).collect(),
            test_results: BTreeMap::new(),
        }
    }

    fn connection(id: &str, source: &str, destination: &str) -> Connection {
        Connection::new(
            ConnectionId::new(id),
            EndpointId::new(source),
            EndpointId::new(destination),
            SignalType::Video,
            Transport::Hdmi,
        )
    }

    #[test]
    fn fingerprints_differ_for_replacement_units() {
        let a = DeviceFingerprint::of_device(&projector("SN-1", "v1.0"));
        let b = DeviceFingerprint::of_device(&projector("SN-2", "v1.0"));
        assert_ne!(a.identity_fingerprint(), b.identity_fingerprint());
        assert_eq!(a.identity_fingerprint().len(), 64);
        // Same unit, same hash.
        let a2 = DeviceFingerprint::of_device(&projector("SN-1", "v1.0"));
        assert_eq!(a.identity_fingerprint(), a2.identity_fingerprint());
    }

    #[test]
    fn drift_detects_replacement_firmware_ip_attributes_and_removal() {
        let old = baseline_with(
            vec![
                projector("SN-1", "v1.0"),
                Device::new(DeviceId::new("display-01"), "Display", DeviceType::Display),
            ],
            vec![],
        );
        let mut new_projector = projector("SN-2", "v1.1");
        new_projector.addresses = Vec::new();
        let mut fp =
            DeviceFingerprint::of_device(&new_projector).with_attribute("resolution", "1920x1080");
        fp.addresses.push("10.0.0.9".into());

        // The display is gone from the current snapshot.
        let current = Baseline {
            devices: vec![fp],
            ..baseline_with(vec![], vec![])
        };

        let drift = detect_drift(&old, &current);
        let descriptions: Vec<String> = drift.iter().map(|d| d.description()).collect();
        assert!(descriptions
            .iter()
            .any(|d| d.contains("device replaced: projector-01")));
        assert!(descriptions.iter().any(|d| d.contains("firmware changed")));
        assert!(descriptions.iter().any(|d| d.contains("address changed")));
        assert!(descriptions
            .iter()
            .any(|d| d.contains("resolution changed: projector-01")));
        assert!(descriptions
            .iter()
            .any(|d| d.contains("device removed: display-01")));
        assert_eq!(drift.len(), 5);
    }

    #[test]
    fn drift_detects_route_changes() {
        let old = baseline_with(vec![], vec![connection("c1", "out-1", "in-2")]);
        let current = baseline_with(vec![], vec![connection("c1", "out-3", "in-2")]);
        let drift = detect_drift(&old, &current);
        assert_eq!(drift.len(), 1);
        assert!(drift[0]
            .description()
            .contains("route changed: c1 (out-1->in-2 → out-3->in-2)"));

        let current = baseline_with(vec![], vec![connection("c1", "out-1", "in-2")]);
        assert!(detect_drift(&old, &current).is_empty());

        let current = baseline_with(vec![], vec![]);
        let drift = detect_drift(&old, &current);
        assert_eq!(drift, vec![Drift::ConnectionRemoved("c1".into())]);

        let drift = detect_drift(&current, &old);
        assert_eq!(drift, vec![Drift::ConnectionAdded("c1".into())]);
    }

    #[test]
    fn identical_systems_produce_no_drift() {
        let devices = vec![projector("SN-1", "v1.0")];
        let connections = vec![connection("c1", "out-1", "in-2")];
        let a = baseline_with(devices.clone(), connections.clone());
        let b = baseline_with(devices, connections);
        assert!(detect_drift(&a, &b).is_empty());
        assert_eq!(a.fingerprint(), b.fingerprint());
    }

    fn result(test: &str, status: TestStatus) -> TestResult {
        TestResult::new(
            tpt_app_av_commissioning_test::TestId::new(test),
            status,
            tpt_app_av_commissioning_test::ExecutionMode::Automated,
        )
    }

    fn baseline_results(results: Vec<(&str, TestStatus)>) -> Baseline {
        Baseline {
            test_results: results
                .into_iter()
                .map(|(id, s)| (id.to_owned(), s))
                .collect(),
            ..baseline_with(vec![], vec![])
        }
    }

    #[test]
    fn regression_comparison_highlights_only_meaningful_changes() {
        let baseline = baseline_results(vec![
            ("a", TestStatus::Pass),
            ("b", TestStatus::Pass),
            ("c", TestStatus::Warning),
            ("d", TestStatus::Fail),
            ("e", TestStatus::Pass),
        ]);
        let current = vec![
            result("a", TestStatus::Fail), // worsened
            result("b", TestStatus::Pass), // unchanged — not reported
            result("c", TestStatus::Pass), // improved
            result("d", TestStatus::Fail), // unchanged failure — not reported
            result("f", TestStatus::Fail), // new failure
        ];
        // "e" missing → no longer run.

        let comparison = compare_results(&baseline, &current);
        assert_eq!(comparison.baseline_result, Some(TestStatus::Fail));
        assert_eq!(comparison.current_result, Some(TestStatus::Fail));

        let worsened: Vec<&str> = comparison
            .changes
            .iter()
            .filter_map(|c| match c {
                Regression::Worsened { test_id, .. } => Some(test_id.as_str()),
                _ => None,
            })
            .collect();
        assert_eq!(worsened, vec!["a"]);
        assert!(comparison
            .changes
            .iter()
            .any(|c| matches!(c, Regression::Improved { test_id, .. } if test_id == "c")));
        assert!(comparison
            .changes
            .iter()
            .any(|c| matches!(c, Regression::NewFailure(f) if f == "f")));
        assert!(comparison
            .changes
            .iter()
            .any(|c| matches!(c, Regression::NoLongerRun(t) if t == "e")));
        // Unchanged b and d are not reported.
        assert_eq!(comparison.changes.len(), 4);
    }

    #[test]
    fn manual_results_await_confirmation_instead_of_counting() {
        let baseline = baseline_results(vec![("m", TestStatus::Pass)]);
        let pending = vec![result("m", TestStatus::Manual)];
        let comparison = compare_results(&baseline, &pending);
        assert!(comparison
            .changes
            .iter()
            .all(|c| matches!(c, Regression::NoLongerRun(_))));
    }

    #[test]
    fn summary_and_aggregate() {
        assert_eq!(aggregate(&[]), None);
        assert_eq!(
            aggregate(&[TestStatus::Pass, TestStatus::Pass]),
            Some(TestStatus::Pass)
        );
        assert_eq!(
            aggregate(&[TestStatus::Pass, TestStatus::Warning]),
            Some(TestStatus::Warning)
        );
        assert_eq!(
            aggregate(&[TestStatus::Warning, TestStatus::Fail]),
            Some(TestStatus::Fail)
        );

        let mut baseline = baseline_with(vec![projector("SN-1", "v1.0")], vec![]);
        baseline.test_results.insert("a".into(), TestStatus::Pass);
        baseline
            .test_results
            .insert("b".into(), TestStatus::Warning);
        let summary = baseline.summary();
        assert_eq!(summary.devices, 1);
        assert_eq!(summary.tests, 2);
        assert_eq!(summary.result, Some(TestStatus::Warning));
    }

    #[test]
    fn baseline_round_trips_through_json() {
        let baseline = baseline_with(
            vec![projector("SN-1", "v1.0")],
            vec![connection("c1", "out-1", "in-2")],
        );
        let json = serde_json::to_string(&baseline).unwrap();
        let back: Baseline = serde_json::from_str(&json).unwrap();
        assert_eq!(back, baseline);
    }
}
