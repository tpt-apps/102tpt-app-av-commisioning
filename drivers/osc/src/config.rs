//! Declarative OSC driver configuration.

use std::collections::BTreeMap;
use std::net::{IpAddr, SocketAddr};
use std::time::Duration;

use tpt_app_av_commissioning_device::{DeviceCapabilities, DeviceIdentity};
use tpt_app_av_commissioning_driver::net::{check_target, check_timeout};
use tpt_app_av_commissioning_driver::DriverError;
use tpt_app_av_commissioning_profile::{
    placeholders_in, DeviceProfile, Placeholder, ProfileArg, ProtocolKind,
};

pub use tpt_app_av_commissioning_driver::net::MAX_TIMEOUT;

/// One argument of a bound OSC message: a literal, or a value taken from the
/// `DeviceCommand` being executed.
#[derive(Debug, Clone, PartialEq)]
pub enum BindingArg {
    Int(i32),
    Float(f32),
    Text(String),
    Bool(bool),
    /// `SetInput { input }` — sent as a string.
    Input,
    /// `GenerateTestPattern { pattern }` — sent as a string.
    Pattern,
    /// `SetAudioVolume { level_db }` — sent as a float, unscaled.
    LevelDb,
    /// `SetAudioMute { muted }` — sent as a bool.
    Muted,
    /// `Freeze { frozen }` — sent as a bool.
    Frozen,
    /// `SetRoute { source }` — sent as a string.
    Source,
    /// `SetRoute { destination }` — sent as a string.
    Destination,
}

/// An OSC message template for one command.
#[derive(Debug, Clone, PartialEq)]
pub struct Binding {
    pub address: String,
    pub args: Vec<BindingArg>,
}

impl Binding {
    pub fn new(address: impl Into<String>, args: Vec<BindingArg>) -> Self {
        Self {
            address: address.into(),
            args,
        }
    }
}

/// A state field read by querying an OSC address.
///
/// The query is an argument-less message to `address`; the device is expected
/// to reply from the same address, and the reply's first argument becomes the
/// value of `field`.
#[derive(Debug, Clone, PartialEq)]
pub struct StateQuery {
    pub field: String,
    pub address: String,
}

impl StateQuery {
    pub fn new(field: impl Into<String>, address: impl Into<String>) -> Self {
        Self {
            field: field.into(),
            address: address.into(),
        }
    }
}

/// Everything the OSC driver needs to know about one device.
#[derive(Debug, Clone)]
pub struct OscDriverConfig {
    /// The device's OSC endpoint (unicast, explicit).
    pub target: SocketAddr,
    /// How long to wait for each reply.
    pub timeout: Duration,
    /// Identity the device is declared to have (from the profile).
    pub identity: DeviceIdentity,
    /// Command bindings, keyed by the `DeviceCommand` kind
    /// (`power_on`, `power_off`, `power_cycle`, `set_input`, `freeze`,
    /// `generate_test_pattern`, `set_audio_volume`, `set_audio_mute`,
    /// `set_route`, `measure_latency`, `read_edid`).
    pub commands: BTreeMap<String, Binding>,
    /// State fields to read.
    pub state_queries: Vec<StateQuery>,
}

impl OscDriverConfig {
    /// A config for `target` with a 1 s reply timeout and nothing bound.
    pub fn new(target: SocketAddr) -> Self {
        Self {
            target,
            timeout: Duration::from_secs(1),
            identity: DeviceIdentity::new(),
            commands: BTreeMap::new(),
            state_queries: Vec::new(),
        }
    }

    pub fn timeout(mut self, timeout: Duration) -> Self {
        self.timeout = timeout;
        self
    }

    pub fn identity(mut self, identity: DeviceIdentity) -> Self {
        self.identity = identity;
        self
    }

    /// Build a config from a device profile, for the device at `host`.
    ///
    /// The profile must be an `osc` profile (any other protocol is refused
    /// rather than half-applied). The declared identity comes from the
    /// profile's `match` section (the first listed model).
    pub fn from_profile(profile: &DeviceProfile, host: IpAddr) -> Result<Self, DriverError> {
        let spec = &profile.device;
        if spec.protocol.kind != ProtocolKind::Osc {
            return Err(DriverError::Config(format!(
                "profile `{}` is a {:?} profile; the OSC driver needs `type: osc`",
                spec.id, spec.protocol.kind
            )));
        }
        let port = spec.protocol.port.ok_or_else(|| {
            DriverError::Config(format!("profile `{}` has no `protocol.port`", spec.id))
        })?;
        let mut config = Self::new(SocketAddr::new(host, port))
            .timeout(Duration::from_millis(spec.protocol.timeout_ms_or_default()))
            .identity(DeviceIdentity {
                manufacturer: Some(spec.matcher.manufacturer.clone()),
                model: spec.matcher.model.as_slice().first().cloned(),
                ..DeviceIdentity::default()
            });
        for (name, cmd) in &spec.commands {
            if !placeholders_in(&cmd.send).is_empty() {
                return Err(DriverError::Config(format!(
                    "commands.{name}: OSC addresses cannot embed placeholders; pass them as args"
                )));
            }
            let args = cmd.args.iter().map(binding_arg).collect();
            config = config.bind(name.clone(), Binding::new(cmd.send.clone(), args));
        }
        for (field, q) in &spec.state {
            config = config.query(StateQuery::new(field.clone(), q.query.clone()));
        }
        config.validate()?;
        Ok(config)
    }

    /// Bind a command kind (see [`OscDriverConfig::commands`]) to a message.
    pub fn bind(mut self, command_kind: impl Into<String>, binding: Binding) -> Self {
        self.commands.insert(command_kind.into(), binding);
        self
    }

    /// Add a state query.
    pub fn query(mut self, query: StateQuery) -> Self {
        self.state_queries.push(query);
        self
    }

    /// Check the config is safe and coherent. [`crate::OscDriver::new`] calls
    /// this.
    pub fn validate(&self) -> Result<(), DriverError> {
        check_target(self.target)?;
        check_timeout(self.timeout)?;
        for binding in self.commands.values() {
            check_address(&binding.address)?;
        }
        for q in &self.state_queries {
            check_address(&q.address)?;
            if q.field.trim().is_empty() {
                return Err(DriverError::Config(
                    "state query field must not be empty".to_owned(),
                ));
            }
        }
        Ok(())
    }

    /// Capabilities implied by what is bound and queried.
    pub fn capabilities(&self) -> DeviceCapabilities {
        let has = |k: &str| self.commands.contains_key(k);
        let has_field = |f: &str| self.state_queries.iter().any(|q| q.field == f);
        DeviceCapabilities {
            can_power_on: has("power_on"),
            can_power_off: has("power_off"),
            can_read_state: !self.state_queries.is_empty(),
            can_select_input: has("set_input"),
            can_generate_test_pattern: has("generate_test_pattern"),
            can_read_signal_status: has_field("signal_present") || has_field("signal_lock"),
            can_read_edid: has("read_edid"),
            can_measure_latency: has("measure_latency"),
            can_restore_state: false,
        }
    }
}

fn binding_arg(arg: &ProfileArg) -> BindingArg {
    if let Some(ph) = arg.placeholder() {
        return match ph {
            Placeholder::Input => BindingArg::Input,
            Placeholder::Pattern => BindingArg::Pattern,
            Placeholder::LevelDb => BindingArg::LevelDb,
            Placeholder::Muted => BindingArg::Muted,
            Placeholder::Frozen => BindingArg::Frozen,
            Placeholder::Source => BindingArg::Source,
            Placeholder::Destination => BindingArg::Destination,
        };
    }
    match arg {
        ProfileArg::Bool(v) => BindingArg::Bool(*v),
        ProfileArg::Int(v) => BindingArg::Int(*v),
        ProfileArg::Float(v) => BindingArg::Float(*v),
        ProfileArg::Text(v) => BindingArg::Text(v.clone()),
    }
}

/// An address must be a plain OSC path. Wildcards and pattern characters are
/// rejected: this driver addresses one parameter, not a pattern.
pub(crate) fn check_address(address: &str) -> Result<(), DriverError> {
    if !address.starts_with('/')
        || address.len() > 256
        || address
            .chars()
            .any(|c| c.is_control() || "*?[]{} ,#".contains(c))
    {
        return Err(DriverError::Config(format!(
            "invalid OSC address {address:?}"
        )));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn addr(s: &str) -> SocketAddr {
        s.parse().unwrap()
    }

    #[test]
    fn rejects_unsafe_targets() {
        for bad in [
            "0.0.0.0:9000",
            "224.0.0.1:9000",
            "255.255.255.255:9000",
            "10.0.0.1:0",
        ] {
            assert!(OscDriverConfig::new(addr(bad)).validate().is_err(), "{bad}");
        }
        assert!(OscDriverConfig::new(addr("10.0.0.9:9000"))
            .validate()
            .is_ok());
    }

    #[test]
    fn rejects_bad_timeouts_and_addresses() {
        let t = addr("10.0.0.9:9000");
        assert!(OscDriverConfig::new(t)
            .timeout(Duration::ZERO)
            .validate()
            .is_err());
        assert!(OscDriverConfig::new(t)
            .timeout(MAX_TIMEOUT + Duration::from_secs(1))
            .validate()
            .is_err());
        let bad = OscDriverConfig::new(t).bind("power_on", Binding::new("power", vec![]));
        assert!(bad.validate().is_err());
        let wild = OscDriverConfig::new(t).query(StateQuery::new("power", "/ch/*/on"));
        assert!(wild.validate().is_err());
    }

    #[test]
    fn capabilities_follow_bindings() {
        let c = OscDriverConfig::new(addr("10.0.0.9:9000"))
            .bind(
                "power_on",
                Binding::new("/power", vec![BindingArg::Bool(true)]),
            )
            .query(StateQuery::new("signal_present", "/signal"));
        let caps = c.capabilities();
        assert!(caps.can_power_on && caps.can_read_state && caps.can_read_signal_status);
        assert!(!caps.can_power_off && !caps.can_select_input);
    }
}
