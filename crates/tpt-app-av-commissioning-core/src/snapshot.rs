//! Configuration snapshots (§27): labelled device configuration captures.
//!
//! Snapshots are always labelled and are never silently overwritten — each
//! write produces a uniquely named, timestamped file.

use std::fs;

use chrono::Utc;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::store::{ProjectAssets, ProjectStoreError};

/// The allowed snapshot labels (§27).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SnapshotLabel {
    BeforeCommissioning,
    AfterCommissioning,
    BeforeMaintenance,
    AfterMaintenance,
}

impl SnapshotLabel {
    /// Directory/segment name on disk.
    pub fn as_str(&self) -> &'static str {
        match self {
            SnapshotLabel::BeforeCommissioning => "before_commissioning",
            SnapshotLabel::AfterCommissioning => "after_commissioning",
            SnapshotLabel::BeforeMaintenance => "before_maintenance",
            SnapshotLabel::AfterMaintenance => "after_maintenance",
        }
    }
}

impl std::fmt::Display for SnapshotLabel {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

/// One stored configuration snapshot.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ConfigSnapshot {
    pub label: SnapshotLabel,
    pub captured_at: String,
    /// Device the snapshot belongs to, when applicable.
    pub device: Option<String>,
    /// The captured configuration as structured JSON.
    pub content: Value,
    /// The relative path of the stored file.
    pub relative_path: String,
}

/// File-based snapshot store under `assets/configurations/`.
#[derive(Debug, Clone)]
pub struct SnapshotStore {
    assets: ProjectAssets,
}

impl SnapshotStore {
    /// Build a snapshot store rooted at the project's asset layout.
    pub fn new(assets: ProjectAssets) -> Self {
        Self { assets }
    }

    /// Save a snapshot under `assets/configurations/<label>/<timestamp>.json`.
    ///
    /// File names embed a timestamp and a random suffix, so a previous
    /// snapshot is never overwritten.
    pub fn save(
        &self,
        label: SnapshotLabel,
        device: Option<String>,
        content: Value,
    ) -> Result<ConfigSnapshot, ProjectStoreError> {
        let dir = self.assets.configurations_dir().join(label.as_str());
        fs::create_dir_all(&dir)?;

        let timestamp = Utc::now().timestamp_micros();
        let random = uuid::Uuid::new_v4().simple().to_string();
        let path = dir.join(format!("{timestamp}-{random}.json"));
        if path.exists() {
            // Extremely unlikely; fail rather than overwrite.
            return Err(ProjectStoreError::EvidenceExists(path));
        }

        let snapshot = ConfigSnapshot {
            label,
            captured_at: Utc::now().to_rfc3339(),
            device: device.clone(),
            content: content.clone(),
            relative_path: path
                .strip_prefix(&self.assets.root())
                .unwrap_or(&path)
                .to_string_lossy()
                .replace('\\', "/"),
        };

        fs::write(&path, serde_json::to_string_pretty(&snapshot)?)?;
        Ok(snapshot)
    }

    /// List snapshots under a label, oldest first.
    pub fn list(&self, label: SnapshotLabel) -> Result<Vec<ConfigSnapshot>, ProjectStoreError> {
        let dir = self.assets.configurations_dir().join(label.as_str());
        if !dir.exists() {
            return Ok(Vec::new());
        }
        let mut entries: Vec<_> = fs::read_dir(&dir)?
            .filter_map(|e| e.ok())
            .filter(|e| e.path().extension().map_or(false, |x| x == "json"))
            .collect();
        entries.sort_by_key(|e| e.file_name());

        let mut out = Vec::new();
        for entry in entries {
            let raw = fs::read_to_string(entry.path())?;
            let snap: ConfigSnapshot = serde_json::from_str(&raw)?;
            out.push(snap);
        }
        Ok(out)
    }

    /// The raw content of a stored snapshot by relative path.
    pub fn read(&self, relative_path: &str) -> Result<Value, ProjectStoreError> {
        let path = self.assets.root().join(relative_path);
        let raw = fs::read_to_string(path)?;
        Ok(serde_json::from_str(&raw)?)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn tmp_dir() -> PathBuf {
        let dir = std::env::temp_dir().join(format!("tpt-av-snap-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn saves_and_lists_under_label() {
        let dir = tmp_dir();
        let assets = ProjectAssets::new(&dir);
        assets.ensure_layout().unwrap();
        let store = SnapshotStore::new(assets);

        let content = serde_json::json!({ "input": "hdmi3", "brightness": 50 });
        let snap = store
            .save(SnapshotLabel::BeforeMaintenance, Some("projector-1".to_owned()), content)
            .unwrap();
        assert_eq!(snap.label, SnapshotLabel::BeforeMaintenance);

        let listed = store.list(SnapshotLabel::BeforeMaintenance).unwrap();
        assert_eq!(listed.len(), 1);

        store
            .save(SnapshotLabel::AfterMaintenance, Some("projector-1".to_owned()), serde_json::json!({}))
            .unwrap();
        assert_eq!(store.list(SnapshotLabel::BeforeMaintenance).unwrap().len(), 1);
        assert_eq!(store.list(SnapshotLabel::AfterMaintenance).unwrap().len(), 1);

        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn never_overwrites_previous_snapshot() {
        let dir = tmp_dir();
        let assets = ProjectAssets::new(&dir);
        assets.ensure_layout().unwrap();
        let store = SnapshotStore::new(assets);

        store
            .save(
                SnapshotLabel::AfterCommissioning,
                None,
                serde_json::json!({ "routing": "a" }),
            )
            .unwrap();
        store
            .save(
                SnapshotLabel::AfterCommissioning,
                None,
                serde_json::json!({ "routing": "b" }),
            )
            .unwrap();

        let listed = store.list(SnapshotLabel::AfterCommissioning).unwrap();
        assert_eq!(listed.len(), 2, "second save must not overwrite the first");
        assert_eq!(listed[0].content["routing"], "a");
        assert_eq!(listed[1].content["routing"], "b");

        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn empty_label_lists_empty() {
        let dir = tmp_dir();
        let assets = ProjectAssets::new(&dir);
        let store = SnapshotStore::new(assets);
        assert!(store.list(SnapshotLabel::BeforeCommissioning).unwrap().is_empty());
        std::fs::remove_dir_all(&dir).unwrap();
    }
}