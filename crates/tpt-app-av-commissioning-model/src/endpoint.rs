//! Endpoint model: `Endpoint` and `EndpointKind`.
//!
//! An endpoint represents something that can send, receive, control, or
//! measure a signal. A device may have many independently testable endpoints.

use serde::{Deserialize, Serialize};

use crate::id::{DeviceId, EndpointId};

/// What an endpoint can do with a signal.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EndpointKind {
    VideoInput,
    VideoOutput,
    AudioInput,
    AudioOutput,
    Network,
    Control,
    Gpio,
    Usb,
    Serial,
    Lighting,
    Clock,
}

impl EndpointKind {
    /// Stable lowercase name (mirrors the YAML form).
    pub fn as_str(&self) -> &'static str {
        match self {
            EndpointKind::VideoInput => "video_input",
            EndpointKind::VideoOutput => "video_output",
            EndpointKind::AudioInput => "audio_input",
            EndpointKind::AudioOutput => "audio_output",
            EndpointKind::Network => "network",
            EndpointKind::Control => "control",
            EndpointKind::Gpio => "gpio",
            EndpointKind::Usb => "usb",
            EndpointKind::Serial => "serial",
            EndpointKind::Lighting => "lighting",
            EndpointKind::Clock => "clock",
        }
    }
}

impl std::fmt::Display for EndpointKind {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

/// A send/receive/control/measure point on a device.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Endpoint {
    pub id: EndpointId,
    pub name: String,
    pub kind: EndpointKind,
    pub device: DeviceId,
}

impl Endpoint {
    /// Create a new endpoint belonging to `device`.
    pub fn new(
        id: EndpointId,
        name: impl Into<String>,
        kind: EndpointKind,
        device: DeviceId,
    ) -> Self {
        Self {
            id,
            name: name.into(),
            kind,
            device,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn endpoint_kind_string_forms() {
        assert_eq!(EndpointKind::VideoInput.as_str(), "video_input");
        assert_eq!(EndpointKind::AudioOutput.as_str(), "audio_output");
        assert_eq!(
            serde_json::to_string(&EndpointKind::Gpio).unwrap(),
            "\"gpio\""
        );
    }

    #[test]
    fn endpoint_belongs_to_a_device() {
        let dev = DeviceId::new("matrix-01");
        let e = Endpoint::new(
            EndpointId::new("matrix-01-in-3"),
            "Input 3",
            EndpointKind::VideoInput,
            dev.clone(),
        );
        assert_eq!(e.device, dev);
        assert_eq!(e.kind, EndpointKind::VideoInput);
    }
}
