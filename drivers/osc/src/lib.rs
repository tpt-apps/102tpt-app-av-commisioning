//! OSC device driver (§10, §40).
//!
//! Talks to AV devices over UDP using the OSC codec from `tpt-av-control-osc`.
//! The codec encodes and parses; this crate owns the socket so that replies
//! arrive on the same port the request left from (how OSC devices reply) and
//! every read is bounded by a timeout.
//!
//! How commissioning concepts map onto OSC is data, not code: an
//! [`OscDriverConfig`] binds each typed `DeviceCommand` to an OSC address and
//! argument template, and lists the addresses to query to build `DeviceState`.
//! This is the shape the versioned device-profile format (Phase 5) will load.
//!
//! OSC has no acknowledgement. A command is reported `ok` once the datagram is
//! sent, and says so in its message; whether the device *acted* is established
//! by reading state back (`CommandTest` does exactly that).
//!
//! Safety (§36): the target must be a unicast address named explicitly (no
//! broadcast, multicast or unspecified), reads are time-bounded, and at most
//! [`MAX_DATAGRAMS_PER_READ`] unrelated datagrams are skipped while waiting
//! for a reply.
//!
//! Licensed under either of MIT OR Apache-2.0, at your option.

mod config;
mod driver;

pub use config::{Binding, BindingArg, OscDriverConfig, StateQuery, MAX_TIMEOUT};
pub use driver::{OscDriver, MAX_DATAGRAMS_PER_READ};
