//! Execution policy (§36): a project-level statement of what the software
//! may do to the installation. Disruptive actions are opt-in, never
//! on-by-default: a project can be commissioned read-only until the
//! engineer explicitly allows power cycles, configuration changes, or
//! network changes.

use serde::{Deserialize, Serialize};

/// What kind of device state a test changes, for execution-policy gating
/// (§36): a project's `execution_policy` decides which kinds may run.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MutationKind {
    /// The test changes nothing (read-only or manual).
    #[default]
    None,
    /// The test changes device or system configuration (input select,
    /// routing, audio settings).
    Configuration,
    /// The test changes network configuration (IP, VLAN, discovery-affecting
    /// settings).
    Network,
    /// The test power-cycles a device (disruptive; needs explicit permission
    /// and confirmation, §36).
    PowerCycle,
}

impl MutationKind {
    /// Stable lowercase name.
    pub fn as_str(&self) -> &'static str {
        match self {
            MutationKind::None => "none",
            MutationKind::Configuration => "configuration",
            MutationKind::Network => "network",
            MutationKind::PowerCycle => "power_cycle",
        }
    }
}

/// What the software is allowed to do on this project (§36).
///
/// Stored with the project; the runner receives it as part of the run
/// options and refuses (blocks, never fails) tests whose mutations are not
/// permitted.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct ExecutionPolicy {
    /// Power-cycle tests may run (disruptive; additionally requires the
    /// per-test explicit confirmation, §36).
    pub allow_power_cycle: bool,
    /// Tests may change device or system configuration (input select,
    /// routing, audio settings).
    pub allow_configuration_changes: bool,
    /// Tests may change network configuration.
    pub allow_network_changes: bool,
}

impl Default for ExecutionPolicy {
    fn default() -> Self {
        Self {
            allow_power_cycle: false,
            allow_configuration_changes: true,
            allow_network_changes: true,
        }
    }
}

impl ExecutionPolicy {
    /// The policy that permits everything — the engine's default so a run
    /// only ever narrows what a project opted into, never silently widens.
    pub fn permissive() -> Self {
        Self {
            allow_power_cycle: true,
            allow_configuration_changes: true,
            allow_network_changes: true,
        }
    }

    /// A policy that changes nothing anywhere (read-only commissioning).
    pub fn read_only() -> Self {
        Self {
            allow_power_cycle: false,
            allow_configuration_changes: false,
            allow_network_changes: false,
        }
    }

    /// Whether a mutation of the given kind is permitted. `None` (read-only
    /// tests) is always permitted.
    pub fn allows(&self, kind: MutationKind) -> bool {
        match kind {
            MutationKind::None => true,
            MutationKind::Configuration => self.allow_configuration_changes,
            MutationKind::Network => self.allow_network_changes,
            MutationKind::PowerCycle => self.allow_power_cycle,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn power_cycles_are_denied_by_default() {
        let policy = ExecutionPolicy::default();
        assert!(!policy.allow_power_cycle);
        assert!(policy.allow_configuration_changes);
        assert!(policy.allows(MutationKind::None));
        assert!(policy.allows(MutationKind::Configuration));
        assert!(!policy.allows(MutationKind::PowerCycle));
    }

    #[test]
    fn read_only_permits_nothing() {
        let policy = ExecutionPolicy::read_only();
        for kind in [
            MutationKind::None,
            MutationKind::Configuration,
            MutationKind::Network,
            MutationKind::PowerCycle,
        ] {
            assert_eq!(policy.allows(kind), kind == MutationKind::None);
        }
    }

    #[test]
    fn permissive_permits_everything_and_round_trips() {
        let policy = ExecutionPolicy::permissive();
        let json = serde_json::to_string(&policy).unwrap();
        let back: ExecutionPolicy = serde_json::from_str(&json).unwrap();
        assert_eq!(back, policy);
        assert!(policy.allows(MutationKind::PowerCycle));
    }
}
