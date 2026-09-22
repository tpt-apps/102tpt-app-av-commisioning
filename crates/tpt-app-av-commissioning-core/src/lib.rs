//! TPT AV Commissioning — application core.
//!
//! Application services shared by the CLI and the desktop UI: persistence
//! (SQLite + YAML manifest), project assets, configuration snapshots, and
//! baselines. See `docs/project-format.md` and §25–27 of `spec.txt`.
//!
//! Licensed under either of MIT OR Apache-2.0, at your option.

pub mod manifest;
pub mod snapshot;
pub mod store;

pub use manifest::{Manifest, ManifestError};
pub use snapshot::{ConfigSnapshot, SnapshotLabel, SnapshotStore};
pub use store::{ProjectAssets, ProjectMeta, ProjectStore, ProjectStoreError, StoredBaseline, StoredDefect};