//! TPT AV Commissioning — test kit.
//!
//! Mock devices, mock networks, and fault injection for integration tests.
//! See §46.2, §47 of `spec.txt`.
//!
//! Licensed under either of MIT OR Apache-2.0, at your option.

pub mod fault;
pub mod mocks;

pub use fault::{Fault, FaultProfile};
pub use mocks::MockDevice;