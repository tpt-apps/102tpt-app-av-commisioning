//! TPT AV Commissioning — driver SDK.
//!
//! Public traits (`DeviceDriver`, `DeviceDiscovery`, `DeviceCommand`,
//! `DeviceState`, `DeviceCapability`) and driver plumbing. Device-specific
//! communication must never leak into test logic. See `docs/device-drivers.md`
//! and §10, §39 of `spec.txt`.
//!
//! Licensed under either of MIT OR Apache-2.0, at your option.

pub mod discovery;
pub mod driver;
pub mod error;
pub mod net;

pub use discovery::{
    DeviceDiscovery, DiscoveredDevice, DiscoveryConfig, DiscoveryProtocol, DiscoveryScope,
    ScopeKind,
};
pub use driver::DeviceDriver;
pub use error::DriverError;
