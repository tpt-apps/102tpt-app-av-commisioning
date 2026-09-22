//! Driver SDK `DeviceDriver` trait and error type.

use async_trait::async_trait;
use tpt_app_av_commissioning_device::{
    DeviceCapabilities, DeviceCommand, DeviceIdentity, DeviceResponse, DeviceState,
};

use crate::error::DriverError;

/// The primary interface between the test runner and a physical device.
///
/// Concrete drivers know protocol- and vendor-specific details; callers only
/// ever depend on this trait. Device-specific communication must not leak
/// into test logic.
///
/// Methods take `&mut self` because executing commands advances driver
/// state. The runner serialises access per device via `DeviceLock`, so this
/// is both safe and race-free.
pub trait DeviceDriver {
    /// Static identity (driver version / protocol) plus the identity the
    /// device reported, when known.
    fn identity(&self) -> DeviceIdentity;

    /// Probe for the device and report a fresh identity + state.
    fn discover(&mut self) -> Result<DeviceState, DriverError>;

    /// Read current device state.
    fn get_state(&mut self) -> Result<DeviceState, DriverError>;

    /// Execute a typed command against the device.
    fn execute(&mut self, command: DeviceCommand) -> Result<DeviceResponse, DriverError>;

    /// The set of commands this driver supports.
    fn capabilities(&self) -> DeviceCapabilities;
}

/// A driver that is capable of async communication (OSC, HTTP, websocket…).
///
/// Blocking drivers may be wrapped in the async facade; this is the hook the
/// runner uses to avoid blocking the UI thread (see §44).
#[async_trait]
pub trait AsyncDeviceDriver {
    async fn discover_async(&mut self) -> Result<DeviceState, DriverError>;
    async fn get_state_async(&mut self) -> Result<DeviceState, DriverError>;
    async fn execute_async(
        &mut self,
        command: DeviceCommand,
    ) -> Result<DeviceResponse, DriverError>;
}
