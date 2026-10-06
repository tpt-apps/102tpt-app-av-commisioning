//! TPT AV Commissioning — application core.
//!
//! Application services shared by the CLI and the desktop UI: persistence
//! (SQLite + YAML manifest), project assets, configuration snapshots,
//! baselines, regression comparison (§21) and drift detection (§22). See
//! `docs/project-format.md` and §21–27 of `spec.txt`.
//!
//! Licensed under either of MIT OR Apache-2.0, at your option.

pub mod baseline;
pub mod manifest;
pub mod snapshot;
pub mod store;

pub use baseline::{
    aggregate, compare_results, detect_drift, Baseline, BaselineSummary, ConnectionSnapshot,
    DeviceFingerprint, Drift, Regression, RunComparison,
};
pub use manifest::{Manifest, ManifestError};
pub use snapshot::{ConfigSnapshot, SnapshotLabel, SnapshotStore};
pub use store::{ProjectAssets, ProjectMeta, ProjectStore, ProjectStoreError, StoredBaseline};
