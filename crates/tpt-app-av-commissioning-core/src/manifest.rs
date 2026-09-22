//! Portable human-readable project manifest (§26).
//!
//! The manifest captures the version-controllable system definition; the
//! SQLite store holds execution history. The format is declarative only —
//! arbitrary executable content is rejected by construction.

use serde::{Deserialize, Serialize};
use serde_yaml_ng::Value;

use tpt_app_av_commissioning_model::{
    Connection, ConnectionExpectation, Device, DeviceAddress, DeviceType, Endpoint, EndpointKind,
    Project, Room, SignalType, Transport,
};

/// The current manifest schema version.
pub const CURRENT_SCHEMA_VERSION: u32 = 1;

/// Errors produced while parsing or rendering a manifest.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum ManifestError {
    #[error("YAML error: {0}")]
    Yaml(String),
    #[error("unsupported schema_version `{found}` (expected `{expected}`)")]
    UnsupportedSchema { found: u32, expected: u32 },
    #[error("missing `schema_version` in `project` section")]
    MissingSchemaVersion,
    #[error("could not parse address `{0}`: {1}")]
    BadAddress(String, String),
}

impl From<serde_yaml_ng::Error> for ManifestError {
    fn from(e: serde_yaml_ng::Error) -> Self {
        ManifestError::Yaml(e.to_string())
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct Manifest {
    pub project: ManifestProject,
    #[serde(default)]
    pub rooms: Vec<ManifestRoom>,
    #[serde(default)]
    pub devices: Vec<ManifestDevice>,
    #[serde(default)]
    pub endpoints: Vec<ManifestEndpoint>,
    #[serde(default)]
    pub connections: Vec<ManifestConnection>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct ManifestProject {
    pub name: String,
    pub schema_version: u32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub client: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub site: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct ManifestRoom {
    pub id: String,
    pub name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct ManifestDevice {
    pub id: String,
    #[serde(rename = "type")]
    pub device_type: DeviceType,
    pub name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub manufacturer: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub serial_number: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub firmware: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub address: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct ManifestEndpoint {
    pub id: String,
    pub device: String,
    pub name: String,
    pub kind: EndpointKind,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct ManifestConnection {
    pub id: String,
    pub source: String,
    pub destination: String,
    pub signal_type: SignalType,
    pub transport: Transport,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub expected: Option<ConnectionExpectation>,
}

impl Manifest {
    /// Parse and validate a manifest string.
    pub fn parse(input: &str) -> Result<Self, ManifestError> {
        let manifest: Self = serde_yaml_ng::from_str(input)?;
        let found = manifest.project.schema_version;
        if found != CURRENT_SCHEMA_VERSION {
            return Err(ManifestError::UnsupportedSchema {
                found,
                expected: CURRENT_SCHEMA_VERSION,
            });
        }
        Ok(manifest)
    }

    /// Render to YAML.
    pub fn render(&self) -> Result<String, ManifestError> {
        Ok(serde_yaml_ng::to_string(self)?)
    }

    /// Build a `Project` plus its entities from the manifest.
    pub fn to_model(
        &self,
    ) -> (
        Project,
        Vec<Room>,
        Vec<Device>,
        Vec<Endpoint>,
        Vec<Connection>,
    ) {
        let id = self
            .project
            .id
            .clone()
            .unwrap_or_else(|| "project".to_owned());
        let mut project = Project::new(
            tpt_app_av_commissioning_model::ProjectId::new(id.clone()),
            &self.project.name,
        );
        project.client = self.project.client.clone();
        project.site = self.project.site.clone();

        let rooms: Vec<Room> = self
            .rooms
            .iter()
            .map(|r| {
                let mut room =
                    Room::new(tpt_app_av_commissioning_model::RoomId::new(&r.id), &r.name);
                room.description = r.description.clone();
                room
            })
            .collect();
        for room in &rooms {
            project.add_room(room.id.clone());
        }

        let devices: Vec<Device> = self
            .devices
            .iter()
            .map(|d| {
                let mut device = Device::new(
                    tpt_app_av_commissioning_model::DeviceId::new(&d.id),
                    &d.name,
                    d.device_type,
                );
                device.manufacturer = d.manufacturer.clone();
                device.model = d.model.clone();
                device.serial_number = d.serial_number.clone();
                device.firmware = d.firmware.clone();
                if let Some(address) = &d.address {
                    if let Ok(parsed) = address.parse::<DeviceAddress>() {
                        device.addresses.push(parsed);
                    }
                }
                device
            })
            .collect();
        for device in &devices {
            project.add_device(device.id.clone());
        }

        let endpoints: Vec<Endpoint> = self
            .endpoints
            .iter()
            .map(|e| {
                Endpoint::new(
                    tpt_app_av_commissioning_model::EndpointId::new(&e.id),
                    &e.name,
                    e.kind,
                    tpt_app_av_commissioning_model::DeviceId::new(&e.device),
                )
            })
            .collect();

        let connections: Vec<Connection> = self
            .connections
            .iter()
            .map(|c| {
                let id = tpt_app_av_commissioning_model::ConnectionId::new(&c.id);
                let mut conn = Connection::new(
                    id,
                    tpt_app_av_commissioning_model::EndpointId::new(&c.source),
                    tpt_app_av_commissioning_model::EndpointId::new(&c.destination),
                    c.signal_type,
                    c.transport,
                );
                if let Some(expected) = &c.expected {
                    conn.expected = expected.clone();
                }
                conn
            })
            .collect();
        for connection in &connections {
            project.add_connection(connection.id.clone());
        }

        (project, rooms, devices, endpoints, connections)
    }

    /// Reject manifests that smuggle executable content into the YAML.
    ///
    /// Our parser is strict, but this explicit guard documents the policy:
    /// the manifest is data only (no code tags, functions, or directives).
    pub fn contains_executable_content(input: &str) -> bool {
        let value: serde_yaml_ng::Value = match serde_yaml_ng::from_str::<Value>(input) {
            Ok(v) => v,
            Err(_) => return true, // unparseable is treated as unsafe
        };
        contains_executable(&value, 0)
    }
}

/// Recursively look for suspicious tags (e.g. `!python/object`, `!rust`) in
/// YAML. Standard YAML tags (`!!str`, `!!int`, …) are data; anything else is
/// treated as executable content.
fn contains_executable(value: &Value, depth: u32) -> bool {
    if depth > 32 {
        return true;
    }
    match value {
        Value::Tagged(tagged) => {
            if !is_safe_tag(&tagged.tag.to_string()) {
                return true;
            }
            contains_executable(&tagged.value, depth + 1)
        }
        Value::Sequence(seq) => seq.iter().any(|v| contains_executable(v, depth + 1)),
        Value::Mapping(map) => map.values().any(|v| contains_executable(v, depth + 1)),
        _ => false,
    }
}

fn is_safe_tag(tag: &str) -> bool {
    tag.is_empty() || tag == "!" || tag.starts_with("!!")
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE: &str = r##"
project:
  schema_version: 1
  name: "Client Boardroom"
  client: "Example Corp"
  site: "Level 2"

rooms:
  - id: boardroom
    name: "Boardroom"

devices:
  - id: projector-01
    type: projector
    name: "Projector 1"
    manufacturer: Example
    model: Example-5000
    address: 192.168.1.40

endpoints:
  - id: projector-01-hdmi
    device: projector-01
    name: "HDMI in"
    kind: video_input

connections:
  - id: matrix-to-projector
    source: scaler-out
    destination: projector-01-hdmi
    signal_type: video
    transport: hdmi
"##;

    #[test]
    fn parses_sample_manifest() {
        let m = Manifest::parse(SAMPLE).unwrap();
        assert_eq!(m.project.schema_version, 1);
        assert_eq!(m.rooms.len(), 1);
        assert_eq!(m.devices[0].model.as_deref(), Some("Example-5000"));
        assert_eq!(m.devices[0].device_type, DeviceType::Projector);
        assert_eq!(m.endpoints[0].kind, EndpointKind::VideoInput);
    }

    #[test]
    fn rejects_unsupported_schema_version() {
        let bad = SAMPLE.replace("schema_version: 1", "schema_version: 99");
        let err = Manifest::parse(&bad).unwrap_err();
        assert_eq!(
            err,
            ManifestError::UnsupportedSchema {
                found: 99,
                expected: 1
            }
        );
    }

    #[test]
    fn round_trips_through_yaml() {
        let m = Manifest::parse(SAMPLE).unwrap();
        let rendered = m.render().unwrap();
        let m2 = Manifest::parse(&rendered).unwrap();
        assert_eq!(m, m2);
    }

    #[test]
    fn converts_to_model() {
        let m = Manifest::parse(SAMPLE).unwrap();
        let (project, rooms, devices, endpoints, connections) = m.to_model();
        assert_eq!(project.name, "Client Boardroom");
        assert_eq!(rooms.len(), 1);
        assert_eq!(devices[0].addresses.len(), 1);
        assert_eq!(endpoints.len(), 1);
        assert_eq!(connections.len(), 1);
    }

    #[test]
    fn rejects_executable_content() {
        assert!(!Manifest::contains_executable_content(SAMPLE));
        assert!(Manifest::contains_executable_content(
            "!python/object:os.system {}\n"
        ));
    }
}
