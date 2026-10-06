//! Mock devices (§46.2): in-memory devices implementing `DeviceDriver`,
//! with optional fault injection. No real hardware, network, or protocol is
//! involved — integration tests run fully offline against these.

use std::time::Duration;

use tpt_app_av_commissioning_device::{
    DeviceCapabilities, DeviceCommand, DeviceIdentity, DeviceResponse, DeviceState, StateValue,
};
use tpt_app_av_commissioning_driver::{DeviceDriver, DriverError};

use crate::fault::Fault;

/// Default manufacturer used by the mock presets.
pub const MOCK_MANUFACTURER: &str = "MockDevices Inc.";

/// In-memory AV device.
#[derive(Debug, Clone)]
pub struct MockDevice {
    pub identity: DeviceIdentity,
    pub state: DeviceState,
    pub capabilities: DeviceCapabilities,
    pub fault: Fault,
    /// Call counter used by intermittent-fault simulation.
    call_count: u64,
}

impl Default for MockDevice {
    fn default() -> Self {
        Self {
            identity: DeviceIdentity::new(),
            state: DeviceState::new(),
            capabilities: DeviceCapabilities::read_only(),
            fault: Fault::None,
            call_count: 0,
        }
    }
}

impl MockDevice {
    /// A projector: power, input, signal, resolution, frame rate.
    ///
    /// Presets intentionally start from defaults and override identity,
    /// capabilities, and state — the reassignment pattern is clearer here
    /// than a monolithic initializer.
    #[allow(clippy::field_reassign_with_default)]
    pub fn projector() -> Self {
        let mut d = Self::default();
        d.identity = DeviceIdentity {
            manufacturer: Some(MOCK_MANUFACTURER.to_owned()),
            model: Some("Beamer 2000".to_owned()),
            serial_number: Some("MTL-SN-1001".to_owned()),
            firmware: Some("3.2.1".to_owned()),
        };
        d.capabilities = DeviceCapabilities::full();
        d.state.set("power", false);
        d.state.set("input", "hdmi1");
        d.state.set("signal_present", false);
        d.state.set("resolution", "1920x1080");
        d.state.set("frame_rate", 60);
        d
    }

    /// A display.
    #[allow(clippy::field_reassign_with_default)]
    pub fn display() -> Self {
        let mut d = Self::default();
        d.identity = DeviceIdentity {
            manufacturer: Some(MOCK_MANUFACTURER.to_owned()),
            model: Some("Panel 4K".to_owned()),
            serial_number: Some("DIS-SN-2002".to_owned()),
            firmware: Some("1.8.0".to_owned()),
        };
        d.capabilities = DeviceCapabilities::full();
        d.state.set("power", true);
        d.state.set("input", "hdmi1");
        d.state.set("signal_present", true);
        d.state.set("resolution", "3840x2160");
        d.state.set("frame_rate", 60);
        d.state.set("edid", "EDID-4K60-blob");
        d
    }

    /// A matrix.
    #[allow(clippy::field_reassign_with_default)]
    pub fn matrix() -> Self {
        let mut d = Self::default();
        d.identity = DeviceIdentity {
            manufacturer: Some(MOCK_MANUFACTURER.to_owned()),
            model: Some("Switch-8x8".to_owned()),
            serial_number: Some("MTX-SN-3003".to_owned()),
            firmware: Some("2.0.0".to_owned()),
        };
        d.capabilities = DeviceCapabilities::full();
        d.state.set("power", true);
        d.state.set("route", "in3->out1");
        d
    }

    /// A DSP.
    #[allow(clippy::field_reassign_with_default)]
    pub fn dsp() -> Self {
        let mut d = Self::default();
        d.identity = DeviceIdentity {
            manufacturer: Some(MOCK_MANUFACTURER.to_owned()),
            model: Some("Core-16".to_owned()),
            serial_number: Some("DSP-SN-4004".to_owned()),
            firmware: Some("5.4.2".to_owned()),
        };
        d.capabilities = DeviceCapabilities::full();
        d.state.set("power", true);
        d.state.set("input_level_dbfs", -18.0);
        d.state.set("output_level_dbfs", -3.0);
        d
    }

    /// A networked audio endpoint.
    #[allow(clippy::field_reassign_with_default)]
    pub fn audio_endpoint() -> Self {
        let mut d = Self::default();
        d.identity = DeviceIdentity {
            manufacturer: Some(MOCK_MANUFACTURER.to_owned()),
            model: Some("Node-2x2".to_owned()),
            serial_number: Some("AUD-SN-5005".to_owned()),
            firmware: Some("1.1.0".to_owned()),
        };
        d.capabilities = DeviceCapabilities::full();
        d.state.set("power", true);
        d.state.set("channel_1", -20.0);
        d.state.set("channel_2", -20.0);
        d
    }

    /// A control processor (read-only capability set plus test pattern).
    #[allow(clippy::field_reassign_with_default)]
    pub fn control_processor() -> Self {
        let mut d = Self::default();
        d.identity = DeviceIdentity {
            manufacturer: Some(MOCK_MANUFACTURER.to_owned()),
            model: Some("Brain-0".to_owned()),
            serial_number: Some("CTL-SN-6006".to_owned()),
            firmware: Some("9.0.1".to_owned()),
        };
        d.capabilities = DeviceCapabilities::read_only();
        d.capabilities.can_generate_test_pattern = true;
        d.state.set("power", true);
        d.state.set("last_command", "none");
        d
    }

    /// Apply a fault to this device.
    pub fn set_fault(&mut self, fault: Fault) {
        self.fault = fault;
        self.call_count = 0;
    }

    /// Increment call counter and return an error for fault scenarios that
    /// fail before any state is touched.
    fn probe_fault(&mut self) -> Result<(), DriverError> {
        // The inter-mutating-state fault is applied per call below.
        match &self.fault {
            Fault::None => Ok(()),
            Fault::Delay(ms) => {
                std::thread::sleep(Duration::from_millis(*ms));
                Ok(())
            }
            Fault::Timeout(ms) => Err(DriverError::Timeout(*ms)),
            Fault::MalformedResponse(payload) => {
                Err(DriverError::MalformedResponse(payload.clone()))
            }
            Fault::Unreachable => Err(DriverError::Unreachable(
                "simulated network drop".to_owned(),
            )),
            Fault::IncorrectField { field, value } => {
                self.state.set(field.clone(), value.clone());
                Ok(())
            }
            Fault::IntermittentFailure(n) => {
                self.call_count += 1;
                if *n == 0 || self.call_count.is_multiple_of(u64::from(*n)) {
                    Err(DriverError::Refused(
                        "simulated intermittent failure".to_owned(),
                    ))
                } else {
                    Ok(())
                }
            }
            Fault::IgnorePower => Ok(()),
        }
    }

    /// Apply command-specific fault behaviour (`IgnorePower`).
    fn apply_command_fault(&mut self, command: &DeviceCommand) -> bool {
        match &self.fault {
            Fault::IgnorePower => matches!(
                command,
                DeviceCommand::PowerOn | DeviceCommand::PowerOff | DeviceCommand::PowerCycle
            ),
            _ => false,
        }
    }
}

impl DeviceDriver for MockDevice {
    fn identity(&self) -> DeviceIdentity {
        self.identity.clone()
    }

    fn discover(&mut self) -> Result<DeviceState, DriverError> {
        self.probe_fault()?;
        Ok(self.state.clone())
    }

    fn get_state(&mut self) -> Result<DeviceState, DriverError> {
        self.probe_fault()?;
        Ok(self.state.clone())
    }

    fn execute(&mut self, command: DeviceCommand) -> Result<DeviceResponse, DriverError> {
        self.probe_fault()?;
        if self.apply_command_fault(&command) {
            // Device accepts the command but state does not change.
            return Ok(DeviceResponse {
                ok: true,
                state: Some(self.state.clone()),
                message: Some("command accepted, state unchanged (fault: IgnorePower)".to_owned()),
                response_time_ms: Some(1),
            });
        }

        let can = self.capabilities;
        let gate = |supported: bool| {
            if !supported {
                return Err(DriverError::UnsupportedOperation);
            }
            Ok(())
        };

        match &command {
            DeviceCommand::PowerOn => {
                gate(can.can_power_on)?;
                self.state.set("power", true);
            }
            DeviceCommand::PowerOff => {
                gate(can.can_power_off)?;
                self.state.set("power", false);
            }
            DeviceCommand::PowerCycle => {
                gate(can.can_power_off & can.can_power_on)?;
                self.state.set("power", false);
                self.state.set("power", true);
            }
            DeviceCommand::SetInput { input } => {
                gate(can.can_select_input)?;
                self.state.set("input", input.clone());
                self.state.set("signal_present", true);
            }
            DeviceCommand::GenerateTestPattern { pattern } => {
                gate(can.can_generate_test_pattern)?;
                self.state.set("test_pattern", pattern.clone());
            }
            DeviceCommand::SetAudioVolume { level_db } => {
                gate(can.can_select_input)?;
                self.state.set("volume_dbfs", *level_db);
            }
            DeviceCommand::SetAudioMute { muted } => {
                gate(can.can_select_input)?;
                self.state.set("muted", *muted);
            }
            DeviceCommand::SetRoute {
                source,
                destination,
            } => {
                gate(can.can_select_input)?;
                self.state.set("route", format!("{source}->{destination}"));
            }
            DeviceCommand::ReadState => {
                gate(can.can_read_state)?;
            }
            DeviceCommand::ReadEdid => {
                gate(can.can_read_edid)?;
            }
            DeviceCommand::MeasureLatency => {
                gate(can.can_measure_latency)?;
                self.state.set(
                    "latency_ms",
                    match self.state.get("power") {
                        Some(StateValue::Boolean(true)) => 18,
                        _ => 0,
                    },
                );
            }
            DeviceCommand::Arbitrary { command } => {
                self.state.set("last_command", command.clone());
            }
            DeviceCommand::Freeze { frozen } => {
                self.state.set("frozen", *frozen);
            }
        }

        Ok(DeviceResponse {
            ok: true,
            state: Some(self.state.clone()),
            message: Some(format!("executed {command:?}")),
            response_time_ms: Some(8),
        })
    }

    /// The mock "device" always restores exactly: the snapshot becomes the
    /// state again (§37).
    fn restore_state(&mut self, pre_state: &DeviceState) -> Result<DeviceResponse, DriverError> {
        self.probe_fault()?;
        if !self.capabilities.can_restore_state {
            return Err(DriverError::UnsupportedOperation);
        }
        self.state = pre_state.clone();
        Ok(DeviceResponse {
            ok: true,
            state: Some(self.state.clone()),
            message: Some("state restored".to_owned()),
            response_time_ms: Some(2),
        })
    }

    fn capabilities(&self) -> DeviceCapabilities {
        self.capabilities
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fault::FaultProfile;

    #[test]
    fn mock_projector_reports_identity_and_state() {
        let mut proj = MockDevice::projector();
        assert_eq!(proj.identity().model.as_deref(), Some("Beamer 2000"));
        assert!(proj.capabilities().can_power_on);
        let state = proj.get_state().unwrap();
        assert_eq!(state.get("power"), Some(&StateValue::Boolean(false)));
    }

    #[test]
    fn power_on_mutates_state() {
        let mut proj = MockDevice::projector();
        let response = proj.execute(DeviceCommand::PowerOn).unwrap();
        assert!(response.ok);
        let state = proj.get_state().unwrap();
        assert_eq!(state.get("power"), Some(&StateValue::Boolean(true)));
    }

    #[test]
    fn unsupported_operation_is_rejected() {
        let mut proj = MockDevice::projector();
        proj.capabilities = DeviceCapabilities::read_only();
        let err = proj.execute(DeviceCommand::PowerOn).unwrap_err();
        assert!(matches!(err, DriverError::UnsupportedOperation));
    }

    #[test]
    fn unreachable_fault_surfaces_as_driver_error() {
        let mut proj = MockDevice::projector();
        proj.set_fault(Fault::Unreachable);
        assert!(matches!(proj.get_state(), Err(DriverError::Unreachable(_))));
    }

    #[test]
    fn malformed_response_fault_surfaces() {
        let mut disp = MockDevice::display();
        disp.set_fault(Fault::MalformedResponse("garbage garbled".to_owned()));
        let err = disp.discover().unwrap_err();
        assert!(matches!(err, DriverError::MalformedResponse(_)));
    }

    #[test]
    fn ignore_power_fault_keeps_state() {
        let mut proj = MockDevice::projector();
        proj.set_fault(Fault::IgnorePower);
        let _ = proj.execute(DeviceCommand::PowerOn).unwrap();
        let state = proj.get_state().unwrap();
        assert_eq!(state.get("power"), Some(&StateValue::Boolean(false)));
    }

    #[test]
    fn incorrect_field_fault_reports_wrong_state() {
        let mut proj = MockDevice::projector();
        proj.set_fault(Fault::IncorrectField {
            field: "input".to_owned(),
            value: "displayport".to_owned(),
        });
        let state = proj.get_state().unwrap();
        assert_eq!(
            state.get("input"),
            Some(&StateValue::Text("displayport".to_owned()))
        );
    }

    #[test]
    fn fault_profile_targets_devices() {
        let mut profile = FaultProfile::new();
        profile.set("matrix-01", Fault::IntermittentFailure(2));
        assert_eq!(
            profile.fault_for("matrix-01"),
            Some(&Fault::IntermittentFailure(2))
        );
        assert!(profile.fault_for("display-01").is_none());
    }
}
