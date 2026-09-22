//! Device model: `Device` and `DeviceType`.

use serde::{Deserialize, Serialize};

use crate::id::{DeviceId, EndpointId};

/// The category of a physical device.
///
/// Extensible by design: `Other` covers categories not yet enumerated.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DeviceType {
    Display,
    Projector,
    Camera,
    Microphone,
    Speaker,
    Amplifier,
    Dsp,
    Switcher,
    Matrix,
    Scaler,
    Encoder,
    Decoder,
    MediaServer,
    ControlProcessor,
    TouchPanel,
    LightingController,
    NetworkDevice,
    Computer,
    Other,
}

impl DeviceType {
    /// A stable, lowercase, human-readable name (mirrors the YAML form).
    pub fn as_str(&self) -> &'static str {
        match self {
            DeviceType::Display => "display",
            DeviceType::Projector => "projector",
            DeviceType::Camera => "camera",
            DeviceType::Microphone => "microphone",
            DeviceType::Speaker => "speaker",
            DeviceType::Amplifier => "amplifier",
            DeviceType::Dsp => "dsp",
            DeviceType::Switcher => "switcher",
            DeviceType::Matrix => "matrix",
            DeviceType::Scaler => "scaler",
            DeviceType::Encoder => "encoder",
            DeviceType::Decoder => "decoder",
            DeviceType::MediaServer => "media_server",
            DeviceType::ControlProcessor => "control_processor",
            DeviceType::TouchPanel => "touch_panel",
            DeviceType::LightingController => "lighting_controller",
            DeviceType::NetworkDevice => "network_device",
            DeviceType::Computer => "computer",
            DeviceType::Other => "other",
        }
    }
}

impl std::fmt::Display for DeviceType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

/// A physical piece of AV equipment.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Device {
    pub id: DeviceId,
    pub name: String,
    pub manufacturer: Option<String>,
    pub model: Option<String>,
    pub serial_number: Option<String>,
    pub firmware: Option<String>,
    pub device_type: DeviceType,
    pub endpoints: Vec<EndpointId>,
    pub addresses: Vec<crate::DeviceAddress>,
}

impl Device {
    /// Create a new device with the given id, name, and type.
    pub fn new(id: DeviceId, name: impl Into<String>, device_type: DeviceType) -> Self {
        Self {
            id,
            name: name.into(),
            manufacturer: None,
            model: None,
            serial_number: None,
            firmware: None,
            device_type,
            endpoints: Vec::new(),
            addresses: Vec::new(),
        }
    }

    /// Attach an endpoint to this device.
    pub fn add_endpoint(&mut self, endpoint: EndpointId) {
        if !self.endpoints.contains(&endpoint) {
            self.endpoints.push(endpoint);
        }
    }

    /// Attach an address to this device.
    pub fn add_address(&mut self, address: crate::DeviceAddress) {
        if !self.addresses.contains(&address) {
            self.addresses.push(address);
        }
    }

    /// True if this device has any network-reachable address.
    pub fn is_reachable(&self) -> bool {
        self.addresses.iter().any(|a| a.is_network())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::DeviceAddress;

    #[test]
    fn device_tracks_endpoints_uniquely() {
        let mut d = Device::new(DeviceId::new("p1"), "Projector 1", DeviceType::Projector);
        let e = EndpointId::new("p1-hdmi-in");
        d.add_endpoint(e.clone());
        d.add_endpoint(e.clone());
        assert_eq!(d.endpoints, vec![e]);
    }

    #[test]
    fn device_is_reachable_when_it_has_an_ip() {
        let mut d = Device::new(DeviceId::new("p1"), "Projector 1", DeviceType::Projector);
        assert!(!d.is_reachable());
        d.add_address(DeviceAddress::Ip("192.168.1.40".parse().unwrap()));
        assert!(d.is_reachable());
    }

    #[test]
    fn device_type_string_forms() {
        assert_eq!(DeviceType::Dsp.as_str(), "dsp");
        assert_eq!(DeviceType::ControlProcessor.as_str(), "control_processor");
        assert_eq!(serde_json::to_string(&DeviceType::Matrix).unwrap(), "\"matrix\"");
    }
}