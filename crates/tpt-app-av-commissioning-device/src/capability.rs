//! Device capabilities: what a driver can do with a device.

use serde::{Deserialize, Serialize};

/// A single capability flag.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DeviceCapability {
    CanPowerOn,
    CanPowerOff,
    CanReadState,
    CanSelectInput,
    CanGenerateTestPattern,
    CanReadSignalStatus,
    CanReadEdid,
    CanMeasureLatency,
    CanRestoreState,
}

/// The set of capabilities a driver supports.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
pub struct DeviceCapabilities {
    pub can_power_on: bool,
    pub can_power_off: bool,
    pub can_read_state: bool,
    pub can_select_input: bool,
    pub can_generate_test_pattern: bool,
    pub can_read_signal_status: bool,
    pub can_read_edid: bool,
    pub can_measure_latency: bool,
    pub can_restore_state: bool,
}

impl DeviceCapabilities {
    /// A driver with no capabilities (do-nothing driver).
    pub fn none() -> Self {
        Self::default()
    }

    /// A read-only driver.
    pub fn read_only() -> Self {
        Self {
            can_read_state: true,
            ..Self::default()
        }
    }

    /// A fully capable driver.
    pub fn full() -> Self {
        Self {
            can_power_on: true,
            can_power_off: true,
            can_read_state: true,
            can_select_input: true,
            can_generate_test_pattern: true,
            can_read_signal_status: true,
            can_read_edid: true,
            can_measure_latency: true,
            can_restore_state: true,
        }
    }
}

impl From<&[DeviceCapability]> for DeviceCapabilities {
    fn from(caps: &[DeviceCapability]) -> Self {
        let mut out = DeviceCapabilities::none();
        for c in caps {
            out |= *c;
        }
        out
    }
}

impl std::ops::BitOrAssign<DeviceCapability> for DeviceCapabilities {
    fn bitor_assign(&mut self, cap: DeviceCapability) {
        let flag = match cap {
            DeviceCapability::CanPowerOn => &mut self.can_power_on,
            DeviceCapability::CanPowerOff => &mut self.can_power_off,
            DeviceCapability::CanReadState => &mut self.can_read_state,
            DeviceCapability::CanSelectInput => &mut self.can_select_input,
            DeviceCapability::CanGenerateTestPattern => &mut self.can_generate_test_pattern,
            DeviceCapability::CanReadSignalStatus => &mut self.can_read_signal_status,
            DeviceCapability::CanReadEdid => &mut self.can_read_edid,
            DeviceCapability::CanMeasureLatency => &mut self.can_measure_latency,
            DeviceCapability::CanRestoreState => &mut self.can_restore_state,
        };
        *flag = true;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn capability_set_builder() {
        let caps = DeviceCapabilities::from(
            &[DeviceCapability::CanPowerOn, DeviceCapability::CanReadState][..],
        );
        assert!(caps.can_power_on);
        assert!(caps.can_read_state);
        assert!(!caps.can_select_input);
    }

    #[test]
    fn full_and_read_only() {
        let full = DeviceCapabilities::full();
        assert!(full.can_measure_latency && full.can_generate_test_pattern);
        let ro = DeviceCapabilities::read_only();
        assert!(ro.can_read_state);
        assert!(!ro.can_power_on);
    }

    #[test]
    fn capabilities_serde() {
        let caps = DeviceCapabilities::full();
        let json = serde_json::to_string(&caps).unwrap();
        let back: DeviceCapabilities = serde_json::from_str(&json).unwrap();
        assert_eq!(back, caps);
    }
}
