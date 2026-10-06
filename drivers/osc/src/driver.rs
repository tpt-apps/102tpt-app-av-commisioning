//! The OSC `DeviceDriver`.

use std::net::{SocketAddr, UdpSocket};
use std::time::Instant;

use tpt_app_av_commissioning_device::{
    DeviceCapabilities, DeviceCommand, DeviceIdentity, DeviceResponse, DeviceState, StateValue,
};
use tpt_app_av_commissioning_driver::net::map_io_error;
use tpt_app_av_commissioning_driver::{DeviceDriver, DriverError};
use tpt_av_control_osc::{OscArg, OscMessage, OscServer};

use crate::config::{check_address, Binding, BindingArg, OscDriverConfig};

/// Largest datagram read (the practical UDP limit).
const MAX_DATAGRAM: usize = 65_507;
/// Unrelated or unparseable datagrams skipped while waiting for one reply.
pub const MAX_DATAGRAMS_PER_READ: usize = 16;
/// Arguments accepted in an `Arbitrary` command.
const MAX_ARBITRARY_ARGS: usize = 16;

/// An OSC device reachable over UDP.
pub struct OscDriver {
    config: OscDriverConfig,
    socket: UdpSocket,
    capabilities: DeviceCapabilities,
}

impl OscDriver {
    /// Validate `config` and open a socket connected to its target. No packet
    /// is sent until a command or query is issued.
    pub fn new(config: OscDriverConfig) -> Result<Self, DriverError> {
        config.validate()?;
        let local: SocketAddr = if config.target.is_ipv4() {
            ([0, 0, 0, 0], 0).into()
        } else {
            (std::net::Ipv6Addr::UNSPECIFIED, 0).into()
        };
        let socket = UdpSocket::bind(local).map_err(|e| map_io(&e, &config))?;
        // Connecting filters incoming datagrams to the target and surfaces
        // ICMP "port unreachable" as an error on the next call.
        socket
            .connect(config.target)
            .map_err(|e| map_io(&e, &config))?;
        let capabilities = config.capabilities();
        Ok(Self {
            config,
            socket,
            capabilities,
        })
    }

    pub fn config(&self) -> &OscDriverConfig {
        &self.config
    }

    fn send(&self, message: &OscMessage) -> Result<usize, DriverError> {
        let bytes = message.encode();
        self.socket
            .send(&bytes)
            .map_err(|e| map_io(&e, &self.config))
    }

    /// Wait for a reply from `address`, skipping at most
    /// [`MAX_DATAGRAMS_PER_READ`] other datagrams.
    fn read_reply(&self, address: &str) -> Result<OscMessage, DriverError> {
        let deadline = Instant::now() + self.config.timeout;
        let mut buf = vec![0u8; MAX_DATAGRAM];
        let mut malformed = 0usize;
        for _ in 0..MAX_DATAGRAMS_PER_READ {
            let remaining = deadline.saturating_duration_since(Instant::now());
            let received = if remaining.is_zero() {
                None
            } else {
                self.socket
                    .set_read_timeout(Some(remaining))
                    .map_err(|e| map_io(&e, &self.config))?;
                match self.socket.recv(&mut buf) {
                    Ok(len) => Some(len),
                    Err(e) => match map_io(&e, &self.config) {
                        DriverError::Timeout(_) => None,
                        other => return Err(other),
                    },
                }
            };
            let Some(len) = received else {
                // Out of time. If the device was talking, but not OSC, say so.
                return Err(if malformed > 0 {
                    DriverError::MalformedResponse(format!(
                        "{malformed} unparseable datagram(s) while waiting for {address}"
                    ))
                } else {
                    self.timeout_error()
                });
            };
            match OscServer::parse_bytes(&buf[..len]) {
                Ok(messages) => {
                    if let Some(m) = messages.into_iter().find(|m| m.address == address) {
                        return Ok(m);
                    }
                }
                Err(_) => malformed += 1,
            }
        }
        if malformed > 0 {
            Err(DriverError::MalformedResponse(format!(
                "{malformed} unparseable datagram(s) while waiting for {address}"
            )))
        } else {
            Err(DriverError::Protocol(format!(
                "no {address} reply within {MAX_DATAGRAMS_PER_READ} datagrams"
            )))
        }
    }

    fn timeout_error(&self) -> DriverError {
        DriverError::Timeout(self.config.timeout.as_millis() as u64)
    }

    fn read_state(&mut self) -> Result<DeviceState, DriverError> {
        if self.config.state_queries.is_empty() {
            return Err(DriverError::UnsupportedOperation);
        }
        let mut state = DeviceState::new();
        for q in &self.config.state_queries {
            let ask = OscMessage::new_unchecked(q.address.clone(), Vec::new());
            self.send(&ask)?;
            let reply = self.read_reply(&q.address)?;
            state.set(q.field.clone(), state_value(&q.address, &reply)?);
        }
        Ok(state)
    }

    fn bound_message(
        &self,
        binding: &Binding,
        command: &DeviceCommand,
    ) -> Result<OscMessage, DriverError> {
        let args = binding
            .args
            .iter()
            .map(|a| resolve(a, command))
            .collect::<Result<Vec<_>, _>>()?;
        Ok(OscMessage::new_unchecked(binding.address.clone(), args))
    }
}

fn kind_name(command: &DeviceCommand) -> &'static str {
    match command {
        DeviceCommand::PowerOn => "power_on",
        DeviceCommand::PowerOff => "power_off",
        DeviceCommand::PowerCycle => "power_cycle",
        DeviceCommand::SetInput { .. } => "set_input",
        DeviceCommand::Freeze { .. } => "freeze",
        DeviceCommand::GenerateTestPattern { .. } => "generate_test_pattern",
        DeviceCommand::SetAudioVolume { .. } => "set_audio_volume",
        DeviceCommand::SetAudioMute { .. } => "set_audio_mute",
        DeviceCommand::SetRoute { .. } => "set_route",
        DeviceCommand::ReadState => "read_state",
        DeviceCommand::ReadEdid => "read_edid",
        DeviceCommand::MeasureLatency => "measure_latency",
        DeviceCommand::Arbitrary { .. } => "arbitrary",
    }
}

/// Fill a template argument from the command being executed.
fn resolve(arg: &BindingArg, command: &DeviceCommand) -> Result<OscArg, DriverError> {
    let mismatch = || {
        DriverError::Config(format!(
            "binding argument {arg:?} does not apply to command {}",
            kind_name(command)
        ))
    };
    Ok(match (arg, command) {
        (BindingArg::Int(v), _) => OscArg::Int(*v),
        (BindingArg::Float(v), _) => OscArg::Float(*v),
        (BindingArg::Text(v), _) => OscArg::String(v.clone()),
        (BindingArg::Bool(v), _) => OscArg::Bool(*v),
        (BindingArg::Input, DeviceCommand::SetInput { input }) => OscArg::String(input.clone()),
        (BindingArg::Pattern, DeviceCommand::GenerateTestPattern { pattern }) => {
            OscArg::String(pattern.clone())
        }
        (BindingArg::LevelDb, DeviceCommand::SetAudioVolume { level_db }) => {
            OscArg::Float(*level_db as f32)
        }
        (BindingArg::Muted, DeviceCommand::SetAudioMute { muted }) => OscArg::Bool(*muted),
        (BindingArg::Frozen, DeviceCommand::Freeze { frozen }) => OscArg::Bool(*frozen),
        (BindingArg::Source, DeviceCommand::SetRoute { source, .. }) => {
            OscArg::String(source.clone())
        }
        (BindingArg::Destination, DeviceCommand::SetRoute { destination, .. }) => {
            OscArg::String(destination.clone())
        }
        _ => return Err(mismatch()),
    })
}

/// A reply's first argument as a state value (raw; no rounding of numbers).
fn state_value(address: &str, reply: &OscMessage) -> Result<StateValue, DriverError> {
    match reply.arguments.first() {
        Some(OscArg::Int(v)) => Ok(StateValue::Integer(i64::from(*v))),
        Some(OscArg::Long(v)) => Ok(StateValue::Integer(*v)),
        Some(OscArg::Float(v)) => Ok(StateValue::Float(f64::from(*v))),
        Some(OscArg::Double(v)) => Ok(StateValue::Float(*v)),
        Some(OscArg::Bool(v)) => Ok(StateValue::Boolean(*v)),
        Some(OscArg::String(v)) | Some(OscArg::Symbol(v)) => Ok(StateValue::Text(v.clone())),
        Some(other) => Err(DriverError::MalformedResponse(format!(
            "{address}: unsupported reply argument type '{}'",
            other.type_tag()
        ))),
        None => Err(DriverError::MalformedResponse(format!(
            "{address}: reply carried no argument"
        ))),
    }
}

/// Parse `"/address arg arg …"`: integers become `i`, other numbers `f`,
/// `true`/`false` become booleans, anything else a string (no spaces).
fn parse_arbitrary(text: &str) -> Result<OscMessage, DriverError> {
    let mut parts = text.split_whitespace();
    let address = parts
        .next()
        .ok_or_else(|| DriverError::Config("empty arbitrary OSC command".to_owned()))?;
    check_address(address)?;
    let args: Vec<OscArg> = parts
        .map(|t| {
            if let Ok(i) = t.parse::<i32>() {
                OscArg::Int(i)
            } else if let Ok(f) = t.parse::<f32>() {
                OscArg::Float(f)
            } else if let Ok(b) = t.parse::<bool>() {
                OscArg::Bool(b)
            } else {
                OscArg::String(t.to_owned())
            }
        })
        .collect();
    if args.len() > MAX_ARBITRARY_ARGS {
        return Err(DriverError::Config(format!(
            "at most {MAX_ARBITRARY_ARGS} arguments are allowed"
        )));
    }
    Ok(OscMessage::new_unchecked(address.to_owned(), args))
}

fn map_io(error: &std::io::Error, config: &OscDriverConfig) -> DriverError {
    map_io_error(error, config.target, config.timeout)
}

impl DeviceDriver for OscDriver {
    fn identity(&self) -> DeviceIdentity {
        self.config.identity.clone()
    }

    /// OSC has no handshake, so discovery is a state read: a device that
    /// answers its queries is there. Needs at least one state query.
    fn discover(&mut self) -> Result<DeviceState, DriverError> {
        if self.config.state_queries.is_empty() {
            return Err(DriverError::Config(
                "no state queries configured; cannot verify the device is present".to_owned(),
            ));
        }
        self.read_state()
    }

    fn get_state(&mut self) -> Result<DeviceState, DriverError> {
        self.read_state()
    }

    fn execute(&mut self, command: DeviceCommand) -> Result<DeviceResponse, DriverError> {
        if matches!(command, DeviceCommand::ReadState) {
            let state = self.read_state()?;
            return Ok(DeviceResponse {
                ok: true,
                state: Some(state),
                message: None,
                response_time_ms: None,
            });
        }
        let message = match &command {
            DeviceCommand::Arbitrary { command } => parse_arbitrary(command)?,
            other => {
                let binding = self
                    .config
                    .commands
                    .get(kind_name(other))
                    .ok_or(DriverError::UnsupportedOperation)?;
                self.bound_message(binding, other)?
            }
        };
        let sent = self.send(&message)?;
        Ok(DeviceResponse {
            ok: true,
            state: None,
            message: Some(format!(
                "sent {sent} bytes to {}; OSC has no acknowledgement, verify by reading state",
                message.address
            )),
            response_time_ms: None,
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
    fn arbitrary_parsing_types_arguments() {
        let m = parse_arbitrary("/preset/recall 3 0.5 true main").unwrap();
        assert_eq!(m.address, "/preset/recall");
        assert_eq!(
            m.arguments,
            vec![
                OscArg::Int(3),
                OscArg::Float(0.5),
                OscArg::Bool(true),
                OscArg::String("main".into())
            ]
        );
    }

    #[test]
    fn arbitrary_parsing_rejects_bad_input() {
        assert!(parse_arbitrary("").is_err());
        assert!(parse_arbitrary("no-slash 1").is_err());
        assert!(parse_arbitrary("/a/* 1").is_err());
        let many = format!("/x {}", "1 ".repeat(MAX_ARBITRARY_ARGS + 1));
        assert!(parse_arbitrary(&many).is_err());
    }

    #[test]
    fn placeholders_only_apply_to_their_command() {
        assert!(resolve(&BindingArg::LevelDb, &DeviceCommand::PowerOn).is_err());
        assert_eq!(
            resolve(
                &BindingArg::Input,
                &DeviceCommand::SetInput {
                    input: "hdmi2".into()
                }
            )
            .unwrap(),
            OscArg::String("hdmi2".into())
        );
    }

    #[test]
    fn reply_values_keep_their_type() {
        let m = |a| OscMessage::new_unchecked("/x", vec![a]);
        assert_eq!(
            state_value("/x", &m(OscArg::Bool(true))).unwrap(),
            StateValue::Boolean(true)
        );
        assert_eq!(
            state_value("/x", &m(OscArg::Int(7))).unwrap(),
            StateValue::Integer(7)
        );
        assert!(state_value("/x", &m(OscArg::Nil)).is_err());
        assert!(state_value("/x", &OscMessage::new_unchecked("/x", vec![])).is_err());
    }
}
