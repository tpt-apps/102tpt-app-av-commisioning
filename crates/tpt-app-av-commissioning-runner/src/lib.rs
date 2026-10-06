//! TPT AV Commissioning — test runner.
//!
//! Dependency-aware scheduling (§14), the concurrent executor (§17) with
//! device locking, timeouts, retries, cancellation and rate limiting, and
//! execution-mode handling (§15). See `docs/test-model.md` and §14–18 of
//! `spec.txt`.
//!
//! Licensed under either of MIT OR Apache-2.0, at your option.

pub mod execute;
pub mod lock;
pub mod plan;

pub use execute::{CancelToken, ExecutionError, RunObserver, RunOutcome, TestExecutor};
pub use lock::{DeviceLock, DeviceLockError, LockRegistry};
pub use plan::{PlanError, PlannedTest, RunOptions, RunPlan, RunPlanner};
