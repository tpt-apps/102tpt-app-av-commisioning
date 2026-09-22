//! TPT AV Commissioning — test runner.
//!
//! Dependency-aware scheduling (§14), device locking (§18), and execution
//! options. See `docs/test-model.md` and §17–18 of `spec.txt`.
//!
//! Licensed under either of MIT OR Apache-2.0, at your option.

pub mod lock;
pub mod plan;

pub use lock::{DeviceLock, DeviceLockError, LockRegistry};
pub use plan::{PlanError, PlannedTest, RunOptions, RunPlan, RunPlanner};