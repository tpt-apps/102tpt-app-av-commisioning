//! Generic protocol drivers (§40).
//!
//! Drivers for common transports that need no vendor code: a device is
//! described by a [`tpt_app_av_commissioning_profile::DeviceProfile`] and an
//! address. Text protocols share one core ([`text`]); transports supply the
//! connection ([`tcp`], [`udp`], [`serial`], [`websocket`]).
//!
//! Licensed under either of MIT OR Apache-2.0, at your option.

pub mod serial;
pub mod tcp;
pub mod text;
pub mod udp;
pub mod websocket;

pub use serial::{SerialDriver, SerialDriverConfig};
pub use tcp::{TcpDriver, TcpDriverConfig};
pub use text::{TextCommand, TextDriver, TextQuery, TextSpec, MAX_LINE_BYTES};
pub use udp::{UdpDriver, UdpDriverConfig};
pub use websocket::{WebSocketDriver, WebSocketDriverConfig};
