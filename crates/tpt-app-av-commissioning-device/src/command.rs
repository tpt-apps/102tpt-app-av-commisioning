//! Typed commands a driver can execute against a device.

use serde::{Deserialize, Serialize};

use crate::DeviceState;

/// A command sent to a device. `Arbitrary` carries protocol-specific text for
/// vendor protocols; typed variants are preferred.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", content = "value", rename_all = "snake_case")]
pub enum DeviceCommand {
    PowerOn,
    PowerOff,
    PowerCycle,
    SetInput {
        input: String,
    },
    Freeze {
        frozen: bool,
    },
    GenerateTestPattern {
        pattern: String,
    },
    SetAudioVolume {
        level_db: f64,
    },
    SetAudioMute {
        muted: bool,
    },
    SetRoute {
        source: String,
        destination: String,
    },
    ReadState,
    ReadEdid,
    MeasureLatency,
    /// Vendor/protocol-specific command, e.g. a raw OSC address or command
    /// string. The driver is responsible for interpreting it.
    Arbitrary {
        command: String,
    },
}

/// A snapshot of state captured before executing a mutating command, so it
/// can be restored afterwards (§37).
pub type PreState = DeviceState;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn command_serde_round_trip() {
        let cmd = DeviceCommand::SetInput {
            input: "hdmi3".to_owned(),
        };
        let json = serde_json::to_string(&cmd).unwrap();
        let back: DeviceCommand = serde_json::from_str(&json).unwrap();
        assert_eq!(back, cmd);
    }

    #[test]
    fn arbitrary_command_is_tagged() {
        let cmd = DeviceCommand::Arbitrary {
            command: "/preset/recall 1".to_owned(),
        };
        let json = serde_json::to_string(&cmd).unwrap();
        assert!(json.contains("arbitrary"));
    }
}
