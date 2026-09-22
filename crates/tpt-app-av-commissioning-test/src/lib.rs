//! TPT AV Commissioning — test model and schema.
//!
//! The `CommissioningTest` trait, `TestStatus`, `TestResult`, requirement and
//! procedure types. See `docs/test-model.md` and §12–16 of `spec.txt`.
//!
//! Licensed under either of MIT OR Apache-2.0, at your option.

pub mod definition;
pub mod result;
pub mod status;

pub use definition::{CommissioningTest, ExecutionMode, TestId, TestRequirements};
pub use result::TestResult;
pub use status::TestStatus;