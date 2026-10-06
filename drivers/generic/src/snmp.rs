//! SNMP (v1 / v2c) read-only driver.
//!
//! For network AV gear (switches, amplifiers, projectors with an SNMP agent)
//! the useful commissioning facts are readings: firmware string, link state,
//! temperature, lamp hours. This driver issues SNMP GET requests and maps each
//! OID to a state field. It never sends SET: SNMP profiles are read-only, and
//! commands are unsupported.
//!
//! Wire encoding and decoding are done by the `snmp2` crate (v1/v2c only; its
//! v3 and async features are disabled). This module adds the commissioning
//! rules:
//!
//! * the target is an explicit unicast address and every request is bounded by
//!   the timeout;
//! * the community string is a credential: it is supplied by the caller (it
//!   defaults to the conventional read community `public`), never taken from a
//!   profile, and redacted from `Debug` output. v1/v2c send it in clear text,
//!   so use it only on a trusted network;
//! * a value is typed: integers, counters, gauges and timeticks become
//!   integers; octet strings become text (or hex if they are not UTF-8); an
//!   OID the agent does not have is simply *not reported*, so a test sees
//!   "device did not report this field" rather than an invented value.

use std::net::{IpAddr, SocketAddr};
use std::time::Duration;

use snmp2::{Oid, SyncSession, Value};

use tpt_app_av_commissioning_device::{
    DeviceCapabilities, DeviceCommand, DeviceIdentity, DeviceResponse, DeviceState, StateValue,
};
use tpt_app_av_commissioning_driver::net::{check_target, check_timeout};
use tpt_app_av_commissioning_driver::{DeviceDriver, DriverError};
use tpt_app_av_commissioning_profile::{parse_oid, DeviceProfile, ParserKind, ProtocolKind};

/// Default SNMP agent port.
pub const DEFAULT_PORT: u16 = 161;
/// Community used when the caller does not supply one.
pub const DEFAULT_COMMUNITY: &str = "public";

/// Protocol version.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SnmpVersion {
    V1,
    V2c,
}

/// A state field read from one OID.
#[derive(Debug, Clone, PartialEq)]
pub struct SnmpQuery {
    pub field: String,
    /// Dotted-decimal OID, e.g. `1.3.6.1.2.1.1.1.0`.
    pub oid: Vec<u32>,
    /// Applied to octet-string values when set (e.g. `power_state`); without
    /// it text stays text.
    pub parser: Option<ParserKind>,
}

impl SnmpQuery {
    /// Parse `oid` (`1.3.6.1.2.1.1.1.0`).
    pub fn new(field: impl Into<String>, oid: &str) -> Result<Self, DriverError> {
        Ok(Self {
            field: field.into(),
            oid: parse_oid(oid).map_err(DriverError::Config)?,
            parser: None,
        })
    }

    pub fn parser(mut self, parser: ParserKind) -> Self {
        self.parser = Some(parser);
        self
    }
}

/// Everything the SNMP driver needs to know about one device.
#[derive(Clone)]
pub struct SnmpDriverConfig {
    pub target: SocketAddr,
    pub timeout: Duration,
    pub version: SnmpVersion,
    pub community: String,
    pub identity: DeviceIdentity,
    pub queries: Vec<SnmpQuery>,
}

impl std::fmt::Debug for SnmpDriverConfig {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SnmpDriverConfig")
            .field("target", &self.target)
            .field("timeout", &self.timeout)
            .field("version", &self.version)
            .field("community", &"<redacted>")
            .field("identity", &self.identity)
            .field("queries", &self.queries)
            .finish()
    }
}

impl SnmpDriverConfig {
    /// A config for `target`: SNMPv2c, community `public`, 1 s timeout.
    pub fn new(target: SocketAddr) -> Self {
        Self {
            target,
            timeout: Duration::from_secs(1),
            version: SnmpVersion::V2c,
            community: DEFAULT_COMMUNITY.to_owned(),
            identity: DeviceIdentity::new(),
            queries: Vec::new(),
        }
    }

    pub fn timeout(mut self, timeout: Duration) -> Self {
        self.timeout = timeout;
        self
    }

    pub fn version(mut self, version: SnmpVersion) -> Self {
        self.version = version;
        self
    }

    pub fn community(mut self, community: impl Into<String>) -> Self {
        self.community = community.into();
        self
    }

    pub fn identity(mut self, identity: DeviceIdentity) -> Self {
        self.identity = identity;
        self
    }

    pub fn query(mut self, query: SnmpQuery) -> Self {
        self.queries.push(query);
        self
    }

    /// Build a config from an `snmp` device profile, for the agent at `host`.
    pub fn from_profile(profile: &DeviceProfile, host: IpAddr) -> Result<Self, DriverError> {
        let p = &profile.device;
        if p.protocol.kind != ProtocolKind::Snmp {
            return Err(DriverError::Config(format!(
                "profile `{}` is a {:?} profile; the SNMP driver needs `type: snmp`",
                p.id, p.protocol.kind
            )));
        }
        if !p.commands.is_empty() {
            return Err(DriverError::Config(
                "SNMP profiles are read-only; remove `commands`".to_owned(),
            ));
        }
        let mut config = Self::new(SocketAddr::new(
            host,
            p.protocol.port.unwrap_or(DEFAULT_PORT),
        ))
        .timeout(Duration::from_millis(p.protocol.timeout_ms_or_default()))
        .identity(DeviceIdentity {
            manufacturer: Some(p.matcher.manufacturer.clone()),
            model: p.matcher.model.as_slice().first().cloned(),
            ..DeviceIdentity::default()
        });
        for (field, q) in &p.state {
            let mut query = SnmpQuery::new(field.clone(), &q.query)?;
            query.parser = q.parser;
            config = config.query(query);
        }
        config.validate()?;
        Ok(config)
    }

    /// Check the config is safe and coherent. [`SnmpDriver::new`] calls this.
    pub fn validate(&self) -> Result<(), DriverError> {
        check_target(self.target)?;
        check_timeout(self.timeout)?;
        if self.community.is_empty()
            || self.community.len() > 64
            || self.community.chars().any(char::is_control)
        {
            return Err(DriverError::Config(
                "community must be 1-64 printable characters".to_owned(),
            ));
        }
        for q in &self.queries {
            if q.field.trim().is_empty() {
                return Err(DriverError::Config(
                    "state field names must not be empty".to_owned(),
                ));
            }
            if q.oid.len() < 2 {
                return Err(DriverError::Config(format!("{}: invalid OID", q.field)));
            }
        }
        Ok(())
    }
}

/// An SNMP agent, read-only.
pub struct SnmpDriver {
    config: SnmpDriverConfig,
}

impl SnmpDriver {
    /// Validate `config`. No packet is sent until an operation runs.
    pub fn new(config: SnmpDriverConfig) -> Result<Self, DriverError> {
        config.validate()?;
        Ok(Self { config })
    }

    pub fn config(&self) -> &SnmpDriverConfig {
        &self.config
    }

    fn session(&self) -> Result<SyncSession, DriverError> {
        let cfg = &self.config;
        // Spread request ids so a stale reply from an earlier call is rejected.
        let req_id = (std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.subsec_nanos())
            .unwrap_or(1)
            & 0x3fff_ffff) as i32;
        let timeout = Some(cfg.timeout);
        let session = match cfg.version {
            SnmpVersion::V1 => {
                SyncSession::new_v1(cfg.target, cfg.community.as_bytes(), timeout, req_id)
            }
            SnmpVersion::V2c => {
                SyncSession::new_v2c(cfg.target, cfg.community.as_bytes(), timeout, req_id)
            }
        };
        session.map_err(|e| {
            tpt_app_av_commissioning_driver::net::map_io_error(&e, cfg.target, cfg.timeout)
        })
    }

    fn read_state(&self) -> Result<DeviceState, DriverError> {
        if self.config.queries.is_empty() {
            return Err(DriverError::UnsupportedOperation);
        }
        let mut session = self.session()?;
        let mut state = DeviceState::new();
        for q in &self.config.queries {
            let arcs: Vec<u64> = q.oid.iter().map(|a| u64::from(*a)).collect();
            let oid = Oid::from(&arcs)
                .map_err(|_| DriverError::Config(format!("{}: invalid OID", q.field)))?;
            let pdu = session
                .get(&oid)
                .map_err(|e| map_snmp_error(e, &self.config))?;
            // v1 reports a missing OID as noSuchName (status 2).
            if pdu.error_status == 2 {
                continue;
            }
            if pdu.error_status != 0 {
                return Err(DriverError::Refused(format!(
                    "{}: agent returned SNMP error status {}",
                    q.field, pdu.error_status
                )));
            }
            let mut found = false;
            for (_, value) in pdu.varbinds {
                found = true;
                if let Some(v) = to_state_value(&q.field, &value, q.parser)? {
                    state.set(q.field.clone(), v);
                }
            }
            if !found {
                return Err(DriverError::MalformedResponse(format!(
                    "{}: response carried no value",
                    q.field
                )));
            }
        }
        Ok(state)
    }
}

fn map_snmp_error(e: snmp2::Error, cfg: &SnmpDriverConfig) -> DriverError {
    use snmp2::Error as E;
    match e {
        // The library folds socket timeouts into `Receive`.
        E::Receive => DriverError::Timeout(cfg.timeout.as_millis() as u64),
        E::Send => DriverError::Unreachable(format!("{}: could not send", cfg.target)),
        E::CommunityMismatch => DriverError::Refused(format!(
            "{}: the agent answered with a different community; check it",
            cfg.target
        )),
        E::RequestIdMismatch => {
            DriverError::MalformedResponse("reply does not match the request".to_owned())
        }
        other => DriverError::MalformedResponse(format!("{}: {other}", cfg.target)),
    }
}

/// Convert one SNMP value. `Ok(None)` means the agent does not have the OID.
fn to_state_value(
    field: &str,
    value: &Value<'_>,
    parser: Option<ParserKind>,
) -> Result<Option<StateValue>, DriverError> {
    Ok(Some(match value {
        Value::Integer(i) => StateValue::Integer(*i),
        Value::Counter32(v) | Value::Unsigned32(v) | Value::Timeticks(v) => {
            StateValue::Integer(i64::from(*v))
        }
        Value::Counter64(v) => match i64::try_from(*v) {
            Ok(i) => StateValue::Integer(i),
            Err(_) => StateValue::Text(v.to_string()),
        },
        Value::Boolean(b) => StateValue::Boolean(*b),
        Value::IpAddress(a) => StateValue::Text(format!("{}.{}.{}.{}", a[0], a[1], a[2], a[3])),
        Value::ObjectIdentifier(oid) => StateValue::Text(oid.to_string()),
        Value::OctetString(bytes) => match std::str::from_utf8(bytes) {
            Ok(text) => match parser {
                Some(p) => p
                    .parse(text)
                    .map_err(|e| DriverError::MalformedResponse(format!("{field}: {e}")))?,
                None => StateValue::Text(text.trim_end_matches('\0').to_owned()),
            },
            Err(_) => StateValue::Blob(bytes.iter().map(|b| format!("{b:02x}")).collect()),
        },
        Value::NoSuchObject | Value::NoSuchInstance | Value::EndOfMibView | Value::Null => {
            return Ok(None)
        }
        other => {
            return Err(DriverError::MalformedResponse(format!(
                "{field}: unsupported SNMP value {other:?}"
            )))
        }
    }))
}

impl DeviceDriver for SnmpDriver {
    fn identity(&self) -> DeviceIdentity {
        self.config.identity.clone()
    }

    /// Discovery is a state read: an agent that answers is there.
    fn discover(&mut self) -> Result<DeviceState, DriverError> {
        if self.config.queries.is_empty() {
            return Err(DriverError::Config(
                "no OIDs configured; cannot verify the agent is present".to_owned(),
            ));
        }
        self.read_state()
    }

    fn get_state(&mut self) -> Result<DeviceState, DriverError> {
        self.read_state()
    }

    fn execute(&mut self, command: DeviceCommand) -> Result<DeviceResponse, DriverError> {
        match command {
            DeviceCommand::ReadState => Ok(DeviceResponse {
                ok: true,
                state: Some(self.read_state()?),
                message: None,
                response_time_ms: None,
            }),
            // SNMP SET is intentionally not implemented.
            _ => Err(DriverError::UnsupportedOperation),
        }
    }

    fn capabilities(&self) -> DeviceCapabilities {
        DeviceCapabilities {
            can_read_state: !self.config.queries.is_empty(),
            ..DeviceCapabilities::none()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn community_is_redacted_and_validated() {
        let c = SnmpDriverConfig::new("10.0.0.9:161".parse().unwrap()).community("s3cret");
        assert!(!format!("{c:?}").contains("s3cret"));
        assert!(c.clone().validate().is_ok());
        assert!(c.clone().community("").validate().is_err());
        assert!(c.community("a\nb").validate().is_err());
    }

    #[test]
    fn rejects_unsafe_config() {
        for bad in ["0.0.0.0:161", "224.0.0.1:161", "10.0.0.1:0"] {
            assert!(SnmpDriverConfig::new(bad.parse().unwrap())
                .validate()
                .is_err());
        }
        assert!(SnmpDriverConfig::new("10.0.0.9:161".parse().unwrap())
            .timeout(Duration::ZERO)
            .validate()
            .is_err());
    }

    #[test]
    fn oids_are_validated() {
        assert!(SnmpQuery::new("d", "1.3.6.1.2.1.1.1.0").is_ok());
        assert!(SnmpQuery::new("d", ".1.3.6.1.2.1.1.1.0").is_ok());
        for bad in ["", "1", "x.y", "1.3.6.-1", "3.1.2", "1.40.3"] {
            assert!(SnmpQuery::new("d", bad).is_err(), "{bad}");
        }
    }
}
