//! Generic protocol drivers (§40).
//!
//! Drivers for common transports that need no vendor code: a device is
//! described by a [`tpt_app_av_commissioning_profile::DeviceProfile`] and a
//! host address. Only TCP text protocols exist so far (see [`tcp`]); UDP,
//! HTTP, WebSocket, serial and SNMP follow the same pattern.
//!
//! Licensed under either of MIT OR Apache-2.0, at your option.

pub mod tcp;

pub use tcp::{TcpDriver, TcpDriverConfig, TextCommand, TextQuery, MAX_LINE_BYTES};
