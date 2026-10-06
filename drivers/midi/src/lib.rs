//! MIDI device driver (§10, §40).
//!
//! Drives MIDI-controllable AV gear (mixers, lighting desks, show controllers)
//! with the MIDI 1.0 codec and port handling from `tpt-av-control-midi`.
//!
//! MIDI is not a request/response protocol, which shapes the driver:
//!
//! * **Commands are fixed messages.** A profile binds a command (`power_on`,
//!   `set_input`, …) to one channel message: `note_on`, `note_off`, `cc` or
//!   `program` with literal numbers. There is no acknowledgement; the driver
//!   reports the message as sent.
//! * **State is what the device last said.** A query names a controller or
//!   program change (`cc 1 7`, `program 1`); its value is the most recent
//!   such message *received* from the device. A device that echoes state
//!   changes can be verified; one that stays silent leaves the field
//!   unreported, and a test sees "device did not report this field" rather
//!   than a guess.
//! * **No stale readings.** The observed values are cleared (and pending
//!   input discarded) just before every command is sent, so a read-back after
//!   a command can only reflect messages the device sent *after* it.
//! * **Presence is the port.** `discover` succeeds when the configured ports
//!   are open; that is what "MIDI device present" means (§13.1).
//!
//! Deliberately not supported: raw (`Arbitrary`) MIDI and System Exclusive.
//! SysEx can carry firmware or configuration changes, so it is not something
//! test data should be able to inject.
//!
//! Safety: at most [`MAX_MESSAGES_PER_READ`] inbound messages are processed
//! per state read, so a device flooding clock or sensing bytes cannot stall a
//! test, and every wait is bounded by the timeout.
//!
//! Licensed under either of MIT OR Apache-2.0, at your option.

use std::collections::{BTreeMap, HashMap};
use std::time::{Duration, Instant};

use tpt_av_control_midi::{
    enumerate_devices, open_input, open_output, ControlError, Midi1Message, MidiSink, MidiSource,
};

use tpt_app_av_commissioning_device::{
    DeviceCapabilities, DeviceCommand, DeviceIdentity, DeviceResponse, DeviceState, StateValue,
};
use tpt_app_av_commissioning_driver::net::check_timeout;
use tpt_app_av_commissioning_driver::{DeviceDriver, DriverError};
use tpt_app_av_commissioning_profile::{DeviceProfile, MidiMessage, MidiQuery, ProtocolKind};

/// Inbound messages processed per state read.
pub const MAX_MESSAGES_PER_READ: usize = 4096;

/// A state field mapped to a MIDI controller or program change.
#[derive(Debug, Clone, PartialEq)]
pub struct MidiStateQuery {
    pub field: String,
    pub query: MidiQuery,
}

/// What a MIDI device is bound to.
#[derive(Debug, Clone, Default)]
pub struct MidiSpec {
    pub identity: DeviceIdentity,
    /// Keyed by command kind (`power_on`, `set_input`, …).
    pub commands: BTreeMap<String, MidiMessage>,
    pub queries: Vec<MidiStateQuery>,
}

impl MidiSpec {
    /// Lift commands, queries and identity out of a `midi` profile.
    pub fn from_profile(profile: &DeviceProfile) -> Result<Self, DriverError> {
        let p = &profile.device;
        if p.protocol.kind != ProtocolKind::Midi {
            return Err(DriverError::Config(format!(
                "profile `{}` is a {:?} profile; the MIDI driver needs `type: midi`",
                p.id, p.protocol.kind
            )));
        }
        let bad = |what: String, e: String| DriverError::Config(format!("{what}: {e}"));
        let mut spec = Self {
            identity: DeviceIdentity {
                manufacturer: Some(p.matcher.manufacturer.clone()),
                model: p.matcher.model.as_slice().first().cloned(),
                ..DeviceIdentity::default()
            },
            ..Self::default()
        };
        for (name, cmd) in &p.commands {
            if !cmd.args.is_empty() || cmd.ack.is_some() || cmd.body.is_some() {
                return Err(DriverError::Config(format!(
                    "commands.{name}: MIDI commands are a single `send` message"
                )));
            }
            let message =
                MidiMessage::parse(&cmd.send).map_err(|e| bad(format!("commands.{name}"), e))?;
            spec.commands.insert(name.clone(), message);
        }
        for (field, q) in &p.state {
            let query = MidiQuery::parse(&q.query).map_err(|e| bad(format!("state.{field}"), e))?;
            spec.queries.push(MidiStateQuery {
                field: field.clone(),
                query,
            });
        }
        Ok(spec)
    }
}

/// How to reach a MIDI device on this machine.
#[derive(Debug, Clone)]
pub struct MidiDriverConfig {
    /// The device's port name, exactly as the system lists it.
    pub port_name: String,
    /// How long to wait for the device to report state.
    pub timeout: Duration,
    pub spec: MidiSpec,
}

impl MidiDriverConfig {
    pub fn new(port_name: impl Into<String>, spec: MidiSpec) -> Self {
        Self {
            port_name: port_name.into(),
            timeout: Duration::from_millis(500),
            spec,
        }
    }

    pub fn timeout(mut self, timeout: Duration) -> Self {
        self.timeout = timeout;
        self
    }

    /// Build a config from a `midi` profile, for the port named `port_name`.
    pub fn from_profile(profile: &DeviceProfile, port_name: &str) -> Result<Self, DriverError> {
        let config = Self {
            port_name: port_name.to_owned(),
            timeout: Duration::from_millis(profile.device.protocol.timeout_ms_or_default()),
            spec: MidiSpec::from_profile(profile)?,
        };
        config.validate()?;
        Ok(config)
    }

    /// Check the config is coherent.
    pub fn validate(&self) -> Result<(), DriverError> {
        check_timeout(self.timeout)?;
        if self.port_name.trim().is_empty()
            || self.port_name.len() > 256
            || self.port_name.chars().any(char::is_control)
        {
            return Err(DriverError::Config(
                "port name must be 1-256 printable characters".to_owned(),
            ));
        }
        Ok(())
    }
}

/// A MIDI device.
pub struct MidiDriver {
    sink: Option<Box<dyn MidiSink + Send>>,
    source: Option<Box<dyn MidiSource + Send>>,
    spec: MidiSpec,
    timeout: Duration,
    capabilities: DeviceCapabilities,
    observed: HashMap<MidiQuery, u8>,
}

fn control_error(e: ControlError, port: &str) -> DriverError {
    match e {
        ControlError::DeviceNotFound(_) => {
            DriverError::Unreachable(format!("MIDI port {port:?} not found"))
        }
        ControlError::PortBusy(m) => DriverError::Unreachable(format!("MIDI port busy: {m}")),
        ControlError::Closed => DriverError::Unreachable(format!("MIDI port {port:?} closed")),
        other => DriverError::Protocol(format!("{port}: {other}")),
    }
}

impl MidiDriver {
    /// Open the named system ports. An output port is opened when commands are
    /// bound and an input port when state queries are; the device must expose
    /// the ports it needs.
    pub fn open(config: MidiDriverConfig) -> Result<Self, DriverError> {
        config.validate()?;
        let name = &config.port_name;
        let devices = enumerate_devices().map_err(|e| control_error(e, name))?;
        let device = devices
            .iter()
            .find(|d| &d.name == name)
            .ok_or_else(|| DriverError::Unreachable(format!("MIDI port {name:?} not found")))?;
        let sink: Option<Box<dyn MidiSink + Send>> = if config.spec.commands.is_empty() {
            None
        } else {
            let port = device.outputs.first().ok_or_else(|| {
                DriverError::Unreachable(format!("{name:?} has no MIDI output port"))
            })?;
            Some(Box::new(
                open_output(port).map_err(|e| control_error(e, name))?,
            ))
        };
        let source: Option<Box<dyn MidiSource + Send>> = if config.spec.queries.is_empty() {
            None
        } else {
            let port = device.inputs.first().ok_or_else(|| {
                DriverError::Unreachable(format!("{name:?} has no MIDI input port"))
            })?;
            Some(Box::new(
                open_input(port).map_err(|e| control_error(e, name))?,
            ))
        };
        Self::with_ports(sink, source, config.spec, config.timeout)
    }

    /// Build a driver over already-open ports (also how tests inject an
    /// in-memory device). Commands need a sink; queries need a source.
    pub fn with_ports(
        sink: Option<Box<dyn MidiSink + Send>>,
        source: Option<Box<dyn MidiSource + Send>>,
        spec: MidiSpec,
        timeout: Duration,
    ) -> Result<Self, DriverError> {
        check_timeout(timeout)?;
        if !spec.commands.is_empty() && sink.is_none() {
            return Err(DriverError::Config(
                "commands are bound but there is no MIDI output".to_owned(),
            ));
        }
        if !spec.queries.is_empty() && source.is_none() {
            return Err(DriverError::Config(
                "state queries are bound but there is no MIDI input".to_owned(),
            ));
        }
        let has = |k: &str| spec.commands.contains_key(k);
        let capabilities = DeviceCapabilities {
            can_power_on: has("power_on"),
            can_power_off: has("power_off"),
            can_read_state: !spec.queries.is_empty(),
            can_select_input: has("set_input"),
            can_generate_test_pattern: has("generate_test_pattern"),
            ..DeviceCapabilities::none()
        };
        Ok(Self {
            sink,
            source,
            spec,
            timeout,
            capabilities,
            observed: HashMap::new(),
        })
    }

    fn record(&mut self, message: &Midi1Message) {
        match *message {
            Midi1Message::ControlChange {
                channel,
                controller,
                value,
            } => {
                self.observed.insert(
                    MidiQuery::ControlChange {
                        channel,
                        controller,
                    },
                    value,
                );
            }
            Midi1Message::ProgramChange { channel, program } => {
                self.observed
                    .insert(MidiQuery::ProgramChange { channel }, program);
            }
            _ => {}
        }
    }

    fn missing(&self) -> bool {
        self.spec
            .queries
            .iter()
            .any(|q| !self.observed.contains_key(&q.query))
    }

    /// Process inbound messages: everything already queued, then, while some
    /// queried value is still unseen, whatever arrives before the deadline.
    fn pump(&mut self, wait: bool) -> Result<(), DriverError> {
        let Some(mut source) = self.source.take() else {
            return Ok(());
        };
        let deadline = Instant::now() + self.timeout;
        let mut processed = 0;
        let result = (|| {
            while processed < MAX_MESSAGES_PER_READ {
                let next = match source.try_recv().map_err(|e| control_error(e, "input"))? {
                    Some(m) => Some(m),
                    None if wait && self.missing() => {
                        let remaining = deadline.saturating_duration_since(Instant::now());
                        if remaining.is_zero() {
                            return Ok(());
                        }
                        source
                            .recv_timeout(remaining)
                            .map_err(|e| control_error(e, "input"))?
                    }
                    None => return Ok(()),
                };
                match next {
                    Some(inbound) => {
                        processed += 1;
                        if let Some(message) = inbound.message {
                            self.record(&message);
                        }
                    }
                    None => return Ok(()), // timed out
                }
            }
            Ok(())
        })();
        self.source = Some(source);
        result
    }

    fn read_state(&mut self) -> Result<DeviceState, DriverError> {
        if self.spec.queries.is_empty() {
            return Err(DriverError::UnsupportedOperation);
        }
        self.pump(true)?;
        let mut state = DeviceState::new();
        for q in &self.spec.queries {
            if let Some(v) = self.observed.get(&q.query) {
                state.set(q.field.clone(), StateValue::Integer(i64::from(*v)));
            }
        }
        Ok(state)
    }
}

fn to_midi1(message: MidiMessage) -> Midi1Message {
    match message {
        MidiMessage::NoteOn {
            channel,
            note,
            velocity,
        } => Midi1Message::NoteOn {
            channel,
            note,
            velocity,
        },
        MidiMessage::NoteOff {
            channel,
            note,
            velocity,
        } => Midi1Message::NoteOff {
            channel,
            note,
            velocity,
        },
        MidiMessage::ControlChange {
            channel,
            controller,
            value,
        } => Midi1Message::ControlChange {
            channel,
            controller,
            value,
        },
        MidiMessage::ProgramChange { channel, program } => {
            Midi1Message::ProgramChange { channel, program }
        }
    }
}

fn command_kind(command: &DeviceCommand) -> Option<&'static str> {
    Some(match command {
        DeviceCommand::PowerOn => "power_on",
        DeviceCommand::PowerOff => "power_off",
        DeviceCommand::PowerCycle => "power_cycle",
        DeviceCommand::SetInput { .. } => "set_input",
        DeviceCommand::Freeze { .. } => "freeze",
        DeviceCommand::GenerateTestPattern { .. } => "generate_test_pattern",
        DeviceCommand::SetAudioVolume { .. } => "set_audio_volume",
        DeviceCommand::SetAudioMute { .. } => "set_audio_mute",
        DeviceCommand::SetRoute { .. } => "set_route",
        DeviceCommand::ReadEdid => "read_edid",
        DeviceCommand::MeasureLatency => "measure_latency",
        DeviceCommand::ReadState | DeviceCommand::Arbitrary { .. } => return None,
    })
}

impl DeviceDriver for MidiDriver {
    fn identity(&self) -> DeviceIdentity {
        self.spec.identity.clone()
    }

    /// The ports are open, so the device is present. State is returned if the
    /// device has reported any, but silence is not a failure here.
    fn discover(&mut self) -> Result<DeviceState, DriverError> {
        if self.spec.queries.is_empty() {
            return Ok(DeviceState::new());
        }
        self.pump(false)?;
        let mut state = DeviceState::new();
        for q in &self.spec.queries {
            if let Some(v) = self.observed.get(&q.query) {
                state.set(q.field.clone(), StateValue::Integer(i64::from(*v)));
            }
        }
        Ok(state)
    }

    fn get_state(&mut self) -> Result<DeviceState, DriverError> {
        self.read_state()
    }

    fn execute(&mut self, command: DeviceCommand) -> Result<DeviceResponse, DriverError> {
        if matches!(command, DeviceCommand::ReadState) {
            return Ok(DeviceResponse {
                ok: true,
                state: Some(self.read_state()?),
                message: None,
                response_time_ms: None,
            });
        }
        let kind = command_kind(&command).ok_or(DriverError::UnsupportedOperation)?;
        let message = *self
            .spec
            .commands
            .get(kind)
            .ok_or(DriverError::UnsupportedOperation)?;
        // Anything the device said before this command must not be mistaken
        // for its reaction to it.
        self.observed.clear();
        if let Some(mut source) = self.source.take() {
            let mut drained = 0;
            while drained < MAX_MESSAGES_PER_READ
                && source
                    .try_recv()
                    .map_err(|e| control_error(e, "input"))?
                    .is_some()
            {
                drained += 1;
            }
            self.source = Some(source);
        }
        let started = Instant::now();
        let sink = self
            .sink
            .as_mut()
            .ok_or(DriverError::UnsupportedOperation)?;
        sink.send_midi1(&to_midi1(message))
            .map_err(|e| control_error(e, "output"))?;
        Ok(DeviceResponse {
            ok: true,
            state: None,
            message: Some(format!(
                "sent {message:?}; MIDI has no acknowledgement, verify by reading state"
            )),
            response_time_ms: Some(started.elapsed().as_millis() as u64),
        })
    }

    fn capabilities(&self) -> DeviceCapabilities {
        self.capabilities
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn config_validation() {
        let c = MidiDriverConfig::new("Mixer", MidiSpec::default());
        assert!(c.validate().is_ok());
        assert!(MidiDriverConfig::new("", MidiSpec::default())
            .validate()
            .is_err());
        assert!(MidiDriverConfig::new("a\nb", MidiSpec::default())
            .validate()
            .is_err());
        assert!(c.timeout(Duration::ZERO).validate().is_err());
    }

    #[test]
    fn commands_and_queries_need_their_ports() {
        let mut spec = MidiSpec::default();
        spec.commands.insert(
            "power_on".into(),
            MidiMessage::ControlChange {
                channel: 0,
                controller: 1,
                value: 127,
            },
        );
        assert!(MidiDriver::with_ports(None, None, spec, Duration::from_millis(100)).is_err());
        let mut spec = MidiSpec::default();
        spec.queries.push(MidiStateQuery {
            field: "x".into(),
            query: MidiQuery::ProgramChange { channel: 0 },
        });
        assert!(MidiDriver::with_ports(None, None, spec, Duration::from_millis(100)).is_err());
    }

    #[test]
    fn missing_port_is_unreachable_not_a_panic() {
        let cfg = MidiDriverConfig::new("tpt-no-such-midi-port-12345", MidiSpec::default());
        // Enumeration can itself fail on a machine with no MIDI subsystem;
        // either way it is an error value.
        assert!(MidiDriver::open(cfg).is_err());
    }
}
