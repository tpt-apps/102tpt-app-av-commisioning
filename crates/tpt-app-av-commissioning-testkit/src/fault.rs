//! Fault injection scenarios (§46.2, §47): simulate the failure modes a
//! commissioning engineer actually meets in the field.

use serde::{Deserialize, Serialize};

/// A single fault to inject into a mock device.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Fault {
    /// Normal operation.
    #[default]
    None,
    /// Every call responds after a delay (simulated in the mock).
    Delay(u64),
    /// Every call fails with a timeout after `ms`.
    Timeout(u64),
    /// The "device" replies with a malformed payload.
    MalformedResponse(String),
    /// The device reports a state field inconsistent with the simulation
    /// (e.g. it claims HDMI input while the route is DisplayPort).
    IncorrectField { field: String, value: String },
    /// Fail every Nth call (1 = always).
    IntermittentFailure(u32),
    /// The device is unreachable (network drop).
    Unreachable,
    /// The device silently ignores power commands (lamp still on, etc.).
    IgnorePower,
}

impl Fault {
    /// Whether any fault is active.
    pub fn enabled(&self) -> bool {
        !matches!(self, Fault::None)
    }
}

/// A set of faults to apply across multiple devices (per-device profiles).
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct FaultProfile {
    /// Scenarios keyed by device id.
    pub per_device: Vec<(String, Fault)>,
}

impl FaultProfile {
    pub fn new() -> Self {
        Self::default()
    }

    /// Set (or replace) the fault for a device.
    pub fn set(&mut self, device: impl Into<String>, fault: Fault) -> &mut Self {
        let device = device.into();
        if let Some(entry) = self.per_device.iter_mut().find(|(id, _)| id == &device) {
            entry.1 = fault;
        } else {
            self.per_device.push((device, fault));
        }
        self
    }

    /// The active fault for a device, if any.
    pub fn fault_for(&self, device: &str) -> Option<&Fault> {
        self.per_device
            .iter()
            .find(|(id, _)| id == device)
            .map(|(_, fault)| fault)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn profile_sets_and_replaces() {
        let mut profile = FaultProfile::new();
        profile.set("p1", Fault::Unreachable);
        profile.set("p1", Fault::Timeout(500));
        assert_eq!(profile.fault_for("p1"), Some(&Fault::Timeout(500)));
        assert!(profile.fault_for("p2").is_none());
    }

    #[test]
    fn fault_enabled_semantics() {
        assert!(!Fault::None.enabled());
        assert!(Fault::IgnorePower.enabled());
    }
}
