//! TPT AV Commissioning — command-line interface.
//!
//! Shares the core engine with the desktop UI. See §34 of `spec.txt`.
//!
//! Licensed under either of MIT OR Apache-2.0, at your option.

pub fn cli_version() -> &'static str {
    env!("CARGO_PKG_VERSION")
}