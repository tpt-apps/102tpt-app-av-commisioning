//! TPT AV Commissioning — reporting and handover.
//!
//! Report rendering (PDF / HTML / CSV / JSON), engineer sign-off, and the
//! handover package. See `docs/report-format.md` and §30–32 of `spec.txt`.
//!
//! Licensed under either of MIT OR Apache-2.0, at your option.

pub mod report;
pub mod signoff;

pub use report::{Report, ReportFormat};
pub use signoff::{SignOff, SignOffResult, Signatory};