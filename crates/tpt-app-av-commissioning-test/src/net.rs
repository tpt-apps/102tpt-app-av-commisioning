//! Driver-free network tests (§13.1, §13.9).
//!
//! These open plain TCP connections to targets the engineer named explicitly.
//! They are deliberately non-aggressive (§36): the target is a literal
//! [`SocketAddr`]/[`IpAddr`] (no name resolution, no ranges), every connect is
//! bounded by a timeout, the number of ports is capped, and nothing is sent
//! after the handshake — the connection is closed immediately.

use std::net::{IpAddr, SocketAddr, TcpStream};
use std::time::{Duration, Instant};

use chrono::Utc;

use tpt_app_av_commissioning_model::{Measurement, MeasurementSource, MeasurementValue, Unit};

use crate::definition::{CommissioningTest, ExecutionMode, TestError, TestId, TestRequirements};
use crate::result::TestResult;
use crate::status::TestStatus;
use crate::test_kind::TestKind;

/// Default per-connection timeout.
pub const DEFAULT_TIMEOUT: Duration = Duration::from_secs(3);
/// Longest per-connection timeout accepted.
pub const MAX_TIMEOUT: Duration = Duration::from_secs(30);
/// Most ports one [`RequiredPortsTest`] will probe.
pub const MAX_PORTS: usize = 64;

fn check_timeout(timeout: Duration) -> Result<Duration, TestError> {
    if timeout.is_zero() || timeout > MAX_TIMEOUT {
        return Err(TestError::Fixture(format!(
            "connect timeout must be between 1 ms and {} s",
            MAX_TIMEOUT.as_secs()
        )));
    }
    Ok(timeout)
}

/// Outcome of one bounded connect attempt.
fn probe(addr: SocketAddr, timeout: Duration) -> (bool, Duration, Option<String>) {
    let timer = Instant::now();
    match TcpStream::connect_timeout(&addr, timeout) {
        Ok(_) => (true, timer.elapsed(), None),
        Err(e) => (false, timer.elapsed(), Some(e.to_string())),
    }
}

/// A TCP port on the device accepts connections (§13.1), and how long the
/// handshake took (a coarse latency figure, §13.9).
pub struct TcpReachableTest {
    id: TestId,
    name: String,
    addr: SocketAddr,
    timeout: Duration,
    requirements: TestRequirements,
}

impl TcpReachableTest {
    pub fn new(
        id: impl Into<String>,
        device: tpt_app_av_commissioning_model::DeviceId,
        addr: SocketAddr,
        timeout: Duration,
    ) -> Result<Self, TestError> {
        let timeout = check_timeout(timeout)?;
        let mut requirements = TestRequirements::reads([device.clone()]);
        requirements.max_duration = Some(timeout + Duration::from_secs(2));
        Ok(Self {
            id: TestId::new(id),
            name: format!("TCP reachable {addr}: {device}"),
            addr,
            timeout,
            requirements,
        })
    }
}

fn millis(name: &str, d: Duration) -> Measurement {
    let mut m = Measurement::new(name, MeasurementValue::Float(d.as_secs_f64() * 1000.0));
    m.unit = Some(Unit::MS);
    m.source = MeasurementSource::Computed;
    m
}

impl CommissioningTest for TcpReachableTest {
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
        let (open, took, err) = probe(self.addr, self.timeout);
        let status = if open {
            TestStatus::Pass
        } else {
            TestStatus::Fail
        };
        let mut r = TestResult::new(self.id.clone(), status, ExecutionMode::Automated);
        r.started_at = started;
        r.measurements.push(millis("connect_time_ms", took));
        match err {
            None => r
                .messages
                .push(format!("{} accepted the connection", self.addr)),
            Some(e) => {
                r.messages
                    .push(format!("{} did not accept: {e}", self.addr));
                r.error = Some(e);
            }
        }
        r.completed_at = Utc::now();
        Ok(r)
    }
}

/// Every required port on a host accepts connections (§13.9 required ports).
pub struct RequiredPortsTest {
    id: TestId,
    name: String,
    host: IpAddr,
    ports: Vec<u16>,
    timeout: Duration,
    requirements: TestRequirements,
}

impl RequiredPortsTest {
    pub fn new(
        id: impl Into<String>,
        device: tpt_app_av_commissioning_model::DeviceId,
        host: IpAddr,
        ports: impl IntoIterator<Item = u16>,
        timeout: Duration,
    ) -> Result<Self, TestError> {
        let timeout = check_timeout(timeout)?;
        let mut ports: Vec<u16> = ports.into_iter().collect();
        ports.sort_unstable();
        ports.dedup();
        if ports.is_empty() {
            return Err(TestError::Fixture(
                "at least one port is required".to_owned(),
            ));
        }
        if ports.len() > MAX_PORTS {
            return Err(TestError::Fixture(format!(
                "at most {MAX_PORTS} ports may be probed per test"
            )));
        }
        if ports.contains(&0) {
            return Err(TestError::Fixture(
                "port 0 is not a valid target".to_owned(),
            ));
        }
        let mut requirements = TestRequirements::reads([device.clone()]);
        requirements.max_duration =
            Some(timeout * u32::try_from(ports.len()).unwrap_or(u32::MAX) + Duration::from_secs(2));
        Ok(Self {
            id: TestId::new(id),
            name: format!("Required ports on {host}: {device}"),
            host,
            ports,
            timeout,
            requirements,
        })
    }
}

impl CommissioningTest for RequiredPortsTest {
    fn id(&self) -> &TestId {
        &self.id
    }
    fn name(&self) -> &str {
        &self.name
    }
    fn kind(&self) -> Option<TestKind> {
        Some(TestKind::Network)
    }
    fn requirements(&self) -> &TestRequirements {
        &self.requirements
    }

    fn execute(&self) -> Result<TestResult, TestError> {
        let started = Utc::now();
        let mut closed = Vec::new();
        let mut r = TestResult::new(self.id.clone(), TestStatus::Pass, ExecutionMode::Automated);
        r.started_at = started;
        for &port in &self.ports {
            let (open, _, _) = probe(SocketAddr::new(self.host, port), self.timeout);
            let mut m = Measurement::new(format!("port_{port}_open"), open);
            m.expected = Some(MeasurementValue::Boolean(true));
            m.source = MeasurementSource::Computed;
            r.measurements.push(m);
            if !open {
                closed.push(port);
            }
        }
        if closed.is_empty() {
            r.messages.push(format!(
                "all {} required ports are open on {}",
                self.ports.len(),
                self.host
            ));
        } else {
            r.status = TestStatus::Fail;
            r.messages.push(format!(
                "ports not accepting connections on {}: {}",
                self.host,
                closed
                    .iter()
                    .map(u16::to_string)
                    .collect::<Vec<_>>()
                    .join(", ")
            ));
        }
        r.completed_at = Utc::now();
        Ok(r)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::net::{Ipv4Addr, TcpListener};
    use tpt_app_av_commissioning_model::DeviceId;

    const LOOPBACK: IpAddr = IpAddr::V4(Ipv4Addr::LOCALHOST);

    fn closed_port() -> u16 {
        let l = TcpListener::bind((LOOPBACK, 0)).unwrap();
        l.local_addr().unwrap().port()
    }

    #[test]
    fn open_port_passes_with_connect_time() {
        let listener = TcpListener::bind((LOOPBACK, 0)).unwrap();
        let addr = listener.local_addr().unwrap();
        let t = TcpReachableTest::new("tcp", DeviceId::new("d"), addr, DEFAULT_TIMEOUT).unwrap();
        let r = t.execute().unwrap();
        assert_eq!(r.status, TestStatus::Pass);
        assert_eq!(r.measurements[0].name, "connect_time_ms");
        assert_eq!(t.kind(), Some(TestKind::Connectivity));
    }

    #[test]
    fn closed_port_fails() {
        let addr = SocketAddr::new(LOOPBACK, closed_port());
        let t = TcpReachableTest::new("tcp", DeviceId::new("d"), addr, DEFAULT_TIMEOUT).unwrap();
        let r = t.execute().unwrap();
        assert_eq!(r.status, TestStatus::Fail);
        assert!(r.error.is_some());
    }

    #[test]
    fn required_ports_reports_each_port() {
        let open = TcpListener::bind((LOOPBACK, 0)).unwrap();
        let open_port = open.local_addr().unwrap().port();
        let t = RequiredPortsTest::new(
            "ports",
            DeviceId::new("d"),
            LOOPBACK,
            [open_port],
            DEFAULT_TIMEOUT,
        )
        .unwrap();
        assert_eq!(t.execute().unwrap().status, TestStatus::Pass);

        let shut = closed_port();
        let t = RequiredPortsTest::new(
            "ports",
            DeviceId::new("d"),
            LOOPBACK,
            [open_port, shut],
            DEFAULT_TIMEOUT,
        )
        .unwrap();
        let r = t.execute().unwrap();
        assert_eq!(r.status, TestStatus::Fail);
        assert_eq!(r.measurements.len(), 2);
        assert_eq!(t.kind(), Some(TestKind::Network));
    }

    #[test]
    fn probing_is_bounded() {
        let d = DeviceId::new("d");
        assert!(RequiredPortsTest::new("p", d.clone(), LOOPBACK, [], DEFAULT_TIMEOUT).is_err());
        assert!(RequiredPortsTest::new("p", d.clone(), LOOPBACK, [0], DEFAULT_TIMEOUT).is_err());
        assert!(RequiredPortsTest::new(
            "p",
            d.clone(),
            LOOPBACK,
            1..=(MAX_PORTS as u16 + 1),
            DEFAULT_TIMEOUT
        )
        .is_err());
        assert!(RequiredPortsTest::new("p", d.clone(), LOOPBACK, [80], Duration::ZERO).is_err());
        assert!(RequiredPortsTest::new(
            "p",
            d,
            LOOPBACK,
            [80],
            MAX_TIMEOUT + Duration::from_secs(1)
        )
        .is_err());
    }
}
