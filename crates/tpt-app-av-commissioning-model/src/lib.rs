//! TPT AV Commissioning — core domain model.
//!
//! Pure domain types: `Project`, `Room`, `Device`, `Endpoint`, `Connection`,
//! signal paths, measurements, and evidence. See `docs/domain-model.md` and
//! §7–9, §19, §24 of `spec.txt`.
//!
//! Licensed under either of MIT OR Apache-2.0, at your option.

#[macro_use]
mod id;

pub mod address;
pub mod audit;
pub mod connection;
pub mod defect;
pub mod device;
pub mod endpoint;
pub mod evidence;
pub mod measurement;
pub mod project;
pub mod room;
pub mod signal_path;

pub use address::{AddressParseError, DeviceAddress};
pub use audit::{Actor, AuditEvent, AuditEventType};
pub use connection::{Connection, ConnectionExpectation, SignalType, Transport, VideoRequirement};
pub use defect::{Defect, DefectId, DefectStatus, Severity};
pub use device::{Device, DeviceType};
pub use endpoint::{Endpoint, EndpointKind};
pub use evidence::{EvidenceId, EvidenceKind, EvidenceRef};
pub use id::{ConnectionId, DeviceId, EndpointId, ProjectId, RoomId, TestSuiteId};
pub use measurement::{Measurement, MeasurementSource, MeasurementValue, Tolerance, Unit};
pub use project::Project;
pub use room::Room;
pub use signal_path::{
    AudioRequirement, LatencyRequirement, PathRequirement, SignalGraph, SignalPath,
};
