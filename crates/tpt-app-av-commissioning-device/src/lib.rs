//! TPT AV Commissioning — device-level primitives.
//!
//! Value types shared by drivers and the runner: device identity, state,
//! commands, responses, and capabilities. See `docs/device-drivers.md` and
//! §10, §39 of `spec.txt`.
//!
//! Licensed under either of MIT OR Apache-2.0, at your option.

pub mod capability;
pub mod command;
pub mod identity;
pub mod response;
pub mod state;

pub use capability::{DeviceCapabilities, DeviceCapability};
pub use command::{DeviceCommand, PreState};
pub use identity::DeviceIdentity;
pub use response::DeviceResponse;
pub use state::{DeviceState, StateValue};
