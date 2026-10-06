//! SQLite project store and managed project assets (§25).
//!
//! The store holds execution history and current system state; binary
//! evidence is stored as immutable assets under `assets/` and only referenced
//! here (never embedded).

use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};

use chrono::{DateTime, Utc};
use rusqlite::{params, Connection, OptionalExtension};
use sha2::{Digest, Sha256};

use tpt_app_av_commissioning_model::{
    Actor, AuditEvent, AuditEventType, Connection as ModelConnection, Defect, DefectStatus, Device,
    Endpoint, EvidenceKind, EvidenceRef, ProjectId, Room,
};
use tpt_app_av_commissioning_test::TestResult;

use crate::baseline::Baseline;

/// Errors produced by the project store.
#[derive(Debug, thiserror::Error)]
pub enum ProjectStoreError {
    #[error("sqlite error: {0}")]
    Sqlite(#[from] rusqlite::Error),
    #[error("io error: {0}")]
    Io(#[from] std::io::Error),
    #[error("project directory `{0}` does not exist")]
    MissingProject(PathBuf),
    #[error("json error: {0}")]
    Json(#[from] serde_json::Error),
    #[error("evidence file `{0}` already exists; will not overwrite")]
    EvidenceExists(PathBuf),
    #[error("baseline label `{0}` already exists; snapshot preserved")]
    BaselineExists(String),
    #[error("no defect with id `{0}`")]
    MissingDefect(String),
    #[error("project meta not set")]
    NoProjectMeta,
}

const SCHEMA: &str = r#"
CREATE TABLE IF NOT EXISTS project_meta (
    id INTEGER PRIMARY KEY CHECK (id = 1),
    project_id TEXT NOT NULL,
    name TEXT NOT NULL,
    client TEXT,
    site TEXT,
    created_at TEXT NOT NULL
);

CREATE TABLE IF NOT EXISTS rooms (
    id TEXT PRIMARY KEY,
    project_id TEXT NOT NULL,
    name TEXT NOT NULL,
    description TEXT
);

CREATE TABLE IF NOT EXISTS devices (
    id TEXT PRIMARY KEY,
    project_id TEXT NOT NULL,
    name TEXT NOT NULL,
    device_type TEXT NOT NULL,
    manufacturer TEXT,
    model TEXT,
    serial_number TEXT,
    firmware TEXT,
    addresses TEXT NOT NULL DEFAULT '[]'
);

CREATE TABLE IF NOT EXISTS endpoints (
    id TEXT PRIMARY KEY,
    device_id TEXT NOT NULL,
    name TEXT NOT NULL,
    kind TEXT NOT NULL
);

CREATE TABLE IF NOT EXISTS connections (
    id TEXT PRIMARY KEY,
    project_id TEXT NOT NULL,
    source TEXT NOT NULL,
    destination TEXT NOT NULL,
    signal_type TEXT NOT NULL,
    transport TEXT NOT NULL,
    expected TEXT NOT NULL DEFAULT '{}'
);

CREATE TABLE IF NOT EXISTS test_suites (
    id TEXT PRIMARY KEY,
    project_id TEXT NOT NULL,
    name TEXT NOT NULL,
    description TEXT
);

CREATE TABLE IF NOT EXISTS test_definitions (
    id TEXT PRIMARY KEY,
    suite_id TEXT,
    name TEXT NOT NULL,
    definition TEXT NOT NULL
);

CREATE TABLE IF NOT EXISTS test_executions (
    id TEXT PRIMARY KEY,
    run_id TEXT NOT NULL,
    project_id TEXT NOT NULL,
    test_id TEXT NOT NULL,
    mode TEXT NOT NULL,
    status TEXT NOT NULL,
    started_at TEXT NOT NULL,
    completed_at TEXT NOT NULL,
    error TEXT
);

CREATE TABLE IF NOT EXISTS results (
    id TEXT PRIMARY KEY,
    run_id TEXT NOT NULL,
    test_id TEXT NOT NULL,
    status TEXT NOT NULL,
    mode TEXT NOT NULL,
    started_at TEXT NOT NULL,
    completed_at TEXT NOT NULL,
    error TEXT,
    payload TEXT NOT NULL
);

CREATE TABLE IF NOT EXISTS measurements (
    id TEXT PRIMARY KEY,
    result_id TEXT NOT NULL,
    name TEXT NOT NULL,
    payload TEXT NOT NULL
);

CREATE TABLE IF NOT EXISTS defects (
    id TEXT PRIMARY KEY,
    project_id TEXT NOT NULL,
    severity TEXT NOT NULL,
    title TEXT NOT NULL,
    description TEXT,
    related_tests TEXT NOT NULL DEFAULT '[]',
    status TEXT NOT NULL,
    payload TEXT,
    created_at TEXT NOT NULL
);

CREATE TABLE IF NOT EXISTS evidence_meta (
    id TEXT PRIMARY KEY,
    project_id TEXT NOT NULL,
    result_id TEXT,
    kind TEXT NOT NULL,
    relative_path TEXT NOT NULL,
    description TEXT,
    captured_at TEXT NOT NULL,
    sha256 TEXT
);

CREATE TABLE IF NOT EXISTS configuration_baselines (
    id TEXT PRIMARY KEY,
    project_id TEXT NOT NULL,
    label TEXT NOT NULL,
    fingerprint TEXT NOT NULL,
    payload TEXT,
    created_at TEXT NOT NULL,
    UNIQUE (project_id, label)
);

CREATE TABLE IF NOT EXISTS audit_log (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    project_id TEXT NOT NULL,
    timestamp TEXT NOT NULL,
    event_type TEXT NOT NULL,
    actor TEXT NOT NULL,
    object_id TEXT,
    details TEXT NOT NULL DEFAULT '{}'
);

CREATE INDEX IF NOT EXISTS idx_devices_project ON devices(project_id);
CREATE INDEX IF NOT EXISTS idx_endpoints_device ON endpoints(device_id);
CREATE INDEX IF NOT EXISTS idx_results_run ON results(run_id);
CREATE INDEX IF NOT EXISTS idx_evidence_project ON evidence_meta(project_id);
"#;

/// Bring older project databases up to the current schema. Idempotent:
/// columns are only added when the `PRAGMA table_info` probe does not find
/// them yet.
fn migrate(conn: &Connection) -> Result<(), ProjectStoreError> {
    for (table, column) in [
        ("configuration_baselines", "payload"),
        ("defects", "payload"),
    ] {
        if !column_exists(conn, table, column)? {
            conn.execute(&format!("ALTER TABLE {table} ADD COLUMN {column} TEXT"), [])?;
        }
    }
    Ok(())
}

/// Whether `table` has a column named `column`.
fn column_exists(conn: &Connection, table: &str, column: &str) -> Result<bool, ProjectStoreError> {
    let mut stmt = conn.prepare(&format!("PRAGMA table_info({table})"))?;
    let mut rows = stmt.query([])?;
    while let Some(row) = rows.next()? {
        let name: String = row.get(1)?;
        if name == column {
            return Ok(true);
        }
    }
    Ok(false)
}

/// Parse a stored audit event type name. Only names this version writes can
/// occur; anything else is treated as a conversion failure, not silently
/// remapped.
fn audit_event_type(name: &str) -> rusqlite::Result<AuditEventType> {
    let known = [
        ("project_created", AuditEventType::ProjectCreated),
        ("device_added", AuditEventType::DeviceAdded),
        ("device_modified", AuditEventType::DeviceModified),
        ("test_run", AuditEventType::TestRun),
        ("result_changed", AuditEventType::ResultChanged),
        ("defect_created", AuditEventType::DefectCreated),
        ("defect_status_changed", AuditEventType::DefectStatusChanged),
        ("defect_closed", AuditEventType::DefectClosed),
        ("baseline_created", AuditEventType::BaselineCreated),
        (
            "configuration_imported",
            AuditEventType::ConfigurationImported,
        ),
        ("report_generated", AuditEventType::ReportGenerated),
        ("report_signed", AuditEventType::ReportSigned),
    ];
    known
        .iter()
        .find(|(n, _)| *n == name)
        .map(|(_, t)| *t)
        .ok_or_else(|| {
            rusqlite::Error::FromSqlConversionFailure(
                1,
                rusqlite::types::Type::Text,
                Box::new(std::io::Error::new(
                    std::io::ErrorKind::InvalidData,
                    format!("unknown audit event type `{name}`"),
                )),
            )
        })
}

/// Project metadata row.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProjectMeta {
    pub id: ProjectId,
    pub name: String,
    pub client: Option<String>,
    pub site: Option<String>,
}

/// A stored configuration baseline (summary row; the full snapshot is in
/// `payload`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StoredBaseline {
    pub id: String,
    pub label: String,
    pub fingerprint: String,
    pub created_at: String,
    /// The serialized [`crate::baseline::Baseline`] snapshot, when present.
    pub payload: Option<String>,
}

/// SQLite-backed store for one project.
#[derive(Debug)]
pub struct ProjectStore {
    conn: Connection,
    root: PathBuf,
}

impl ProjectStore {
    /// Create a new project directory and store. Fails if `project_dir`
    /// already contains a `project.sqlite` (to avoid clobbering).
    pub fn create(project_dir: &Path, meta: &ProjectMeta) -> Result<Self, ProjectStoreError> {
        let db = project_dir.join("project.sqlite");
        if db.exists() {
            return Err(ProjectStoreError::EvidenceExists(db));
        }
        fs::create_dir_all(project_dir)?;
        let assets = ProjectAssets::new(project_dir);
        assets.ensure_layout()?;

        let conn = Connection::open(&db)?;
        conn.execute_batch(SCHEMA)?;
        migrate(&conn)?;
        let mut store = Self {
            conn,
            root: project_dir.to_path_buf(),
        };
        store.insert_project_meta(meta)?;
        Ok(store)
    }

    /// Open an existing project store. The directory must exist and contain a
    /// `project.sqlite`.
    pub fn open(project_dir: &Path) -> Result<Self, ProjectStoreError> {
        let db = project_dir.join("project.sqlite");
        if !project_dir.exists() {
            return Err(ProjectStoreError::MissingProject(project_dir.to_path_buf()));
        }
        if !db.exists() {
            return Err(ProjectStoreError::MissingProject(db));
        }
        let conn = Connection::open(&db)?;
        conn.execute_batch(SCHEMA)?;
        migrate(&conn)?;
        Ok(Self {
            conn,
            root: project_dir.to_path_buf(),
        })
    }

    /// Project directory on disk.
    pub fn root(&self) -> &Path {
        &self.root
    }

    /// Managed asset layout for this project.
    pub fn assets(&self) -> ProjectAssets {
        ProjectAssets::new(&self.root)
    }

    // -- project meta ------------------------------------------------------

    fn insert_project_meta(&mut self, meta: &ProjectMeta) -> Result<(), ProjectStoreError> {
        let tx = self.conn.transaction()?;
        tx.execute(
            "INSERT INTO project_meta (id, project_id, name, client, site, created_at)
             VALUES (1, ?1, ?2, ?3, ?4, ?5)",
            params![
                meta.id.as_str(),
                meta.name,
                meta.client,
                meta.site,
                Utc::now().to_rfc3339()
            ],
        )?;
        tx.commit()?;
        Ok(())
    }

    /// Load project metadata.
    pub fn project_meta(&self) -> Result<Option<ProjectMeta>, ProjectStoreError> {
        let row = self
            .conn
            .query_row(
                "SELECT project_id, name, client, site FROM project_meta WHERE id = 1",
                [],
                |r| {
                    Ok(ProjectMeta {
                        id: ProjectId::new(r.get::<_, String>(0)?),
                        name: r.get(1)?,
                        client: r.get(2)?,
                        site: r.get(3)?,
                    })
                },
            )
            .optional()?;
        Ok(row)
    }

    // -- rooms -------------------------------------------------------------

    /// Upsert a room row.
    pub fn upsert_room(
        &mut self,
        project: &ProjectId,
        room: &Room,
    ) -> Result<(), ProjectStoreError> {
        self.conn.execute(
            "INSERT INTO rooms (id, project_id, name, description)
             VALUES (?1, ?2, ?3, ?4)
             ON CONFLICT(id) DO UPDATE SET name = excluded.name, description = excluded.description",
            params![room.id.as_str(), project.as_str(), room.name, room.description],
        )?;
        Ok(())
    }

    /// List all rooms.
    pub fn list_rooms(&self) -> Result<Vec<Room>, ProjectStoreError> {
        let mut stmt = self
            .conn
            .prepare("SELECT id, name, description FROM rooms")?;
        let rows = stmt.query_map([], |r| {
            Ok(Room {
                id: tpt_app_av_commissioning_model::RoomId::new(r.get::<_, String>(0)?),
                name: r.get(1)?,
                description: r.get(2)?,
                devices: Vec::new(),
                connections: Vec::new(),
            })
        })?;
        Ok(rows.collect::<Result<Vec<_>, _>>()?)
    }

    // -- devices -----------------------------------------------------------

    /// Upsert a device row (addresses serialised to JSON).
    pub fn upsert_device(
        &mut self,
        project: &ProjectId,
        device: &Device,
    ) -> Result<(), ProjectStoreError> {
        let addresses = serde_json::to_string(&device.addresses)?;
        self.conn.execute(
            "INSERT INTO devices (id, project_id, name, device_type, manufacturer, model, serial_number, firmware, addresses)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)
             ON CONFLICT(id) DO UPDATE SET
               name = excluded.name,
               device_type = excluded.device_type,
               manufacturer = excluded.manufacturer,
               model = excluded.model,
               serial_number = excluded.serial_number,
               firmware = excluded.firmware,
               addresses = excluded.addresses",
            params![
                device.id.as_str(),
                project.as_str(),
                device.name,
                serde_json::to_string(&device.device_type)?,
                device.manufacturer,
                device.model,
                device.serial_number,
                device.firmware,
                addresses
            ],
        )?;
        Ok(())
    }

    /// List all devices.
    pub fn list_devices(&self) -> Result<Vec<Device>, ProjectStoreError> {
        let mut stmt = self
            .conn
            .prepare("SELECT id, name, device_type, manufacturer, model, serial_number, firmware, addresses FROM devices")?;
        let rows = stmt.query_map([], |r| {
            let addresses: String = r.get(7)?;
            Ok(Device {
                id: tpt_app_av_commissioning_model::DeviceId::new(r.get::<_, String>(0)?),
                name: r.get(1)?,
                device_type: serde_json::from_str(&r.get::<_, String>(2)?).map_err(
                    |e: serde_json::Error| rusqlite::Error::ToSqlConversionFailure(Box::new(e)),
                )?,
                manufacturer: r.get(3)?,
                model: r.get(4)?,
                serial_number: r.get(5)?,
                firmware: r.get(6)?,
                endpoints: Vec::new(),
                addresses: serde_json::from_str(&addresses).map_err(|e: serde_json::Error| {
                    rusqlite::Error::ToSqlConversionFailure(Box::new(e))
                })?,
            })
        })?;
        Ok(rows.collect::<Result<Vec<_>, _>>()?)
    }

    // -- endpoints ---------------------------------------------------------

    /// Upsert an endpoint row.
    pub fn upsert_endpoint(&mut self, endpoint: &Endpoint) -> Result<(), ProjectStoreError> {
        self.conn.execute(
            "INSERT INTO endpoints (id, device_id, name, kind)
             VALUES (?1, ?2, ?3, ?4)
             ON CONFLICT(id) DO UPDATE SET device_id = excluded.device_id, name = excluded.name, kind = excluded.kind",
            params![
                endpoint.id.as_str(),
                endpoint.device.as_str(),
                endpoint.name,
                serde_json::to_string(&endpoint.kind)?
            ],
        )?;
        Ok(())
    }

    /// List all endpoints.
    pub fn list_endpoints(&self) -> Result<Vec<Endpoint>, ProjectStoreError> {
        let mut stmt = self
            .conn
            .prepare("SELECT id, device_id, name, kind FROM endpoints")?;
        let rows = stmt.query_map([], |r| {
            Ok(Endpoint {
                id: tpt_app_av_commissioning_model::EndpointId::new(r.get::<_, String>(0)?),
                device: tpt_app_av_commissioning_model::DeviceId::new(r.get::<_, String>(1)?),
                name: r.get(2)?,
                kind: serde_json::from_str(&r.get::<_, String>(3)?).map_err(
                    |e: serde_json::Error| rusqlite::Error::ToSqlConversionFailure(Box::new(e)),
                )?,
            })
        })?;
        Ok(rows.collect::<Result<Vec<_>, _>>()?)
    }

    // -- connections -------------------------------------------------------

    /// Upsert a connection row.
    pub fn upsert_connection(
        &mut self,
        project: &ProjectId,
        connection: &ModelConnection,
    ) -> Result<(), ProjectStoreError> {
        self.conn.execute(
            "INSERT INTO connections (id, project_id, source, destination, signal_type, transport, expected)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)
             ON CONFLICT(id) DO UPDATE SET
               source = excluded.source,
               destination = excluded.destination,
               signal_type = excluded.signal_type,
               transport = excluded.transport,
               expected = excluded.expected",
            params![
                connection.id.as_str(),
                project.as_str(),
                connection.source.as_str(),
                connection.destination.as_str(),
                serde_json::to_string(&connection.signal_type)?,
                serde_json::to_string(&connection.transport)?,
                serde_json::to_string(&connection.expected)?
            ],
        )?;
        Ok(())
    }

    /// List all connections.
    pub fn list_connections(&self) -> Result<Vec<ModelConnection>, ProjectStoreError> {
        let mut stmt = self.conn.prepare(
            "SELECT id, source, destination, signal_type, transport, expected FROM connections",
        )?;
        let rows = stmt.query_map([], |r| {
            let signal: String = r.get(3)?;
            let transport: String = r.get(4)?;
            let expected: String = r.get(5)?;
            Ok(ModelConnection {
                id: tpt_app_av_commissioning_model::ConnectionId::new(r.get::<_, String>(0)?),
                source: tpt_app_av_commissioning_model::EndpointId::new(r.get::<_, String>(1)?),
                destination: tpt_app_av_commissioning_model::EndpointId::new(
                    r.get::<_, String>(2)?,
                ),
                signal_type: serde_json::from_str(&signal).map_err(|e: serde_json::Error| {
                    rusqlite::Error::ToSqlConversionFailure(Box::new(e))
                })?,
                transport: serde_json::from_str(&transport).map_err(|e: serde_json::Error| {
                    rusqlite::Error::ToSqlConversionFailure(Box::new(e))
                })?,
                expected: serde_json::from_str(&expected).map_err(|e: serde_json::Error| {
                    rusqlite::Error::ToSqlConversionFailure(Box::new(e))
                })?,
            })
        })?;
        Ok(rows.collect::<Result<Vec<_>, _>>()?)
    }

    // -- results -----------------------------------------------------------

    /// Persist one test result: execution row, result row, measurements,
    /// and evidence metadata. Run-scoped so a full run can be reconstructed.
    pub fn save_result(
        &mut self,
        run_id: &str,
        project: &ProjectId,
        result: &TestResult,
    ) -> Result<(), ProjectStoreError> {
        let payload = serde_json::to_string(result)?;
        let tx = self.conn.transaction()?;
        tx.execute(
            "INSERT INTO results (id, run_id, test_id, status, mode, started_at, completed_at, error, payload)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
            params![
                uuid::Uuid::new_v4().to_string(),
                run_id,
                result.test_id.as_str(),
                result.status.as_str(),
                result.mode.as_str(),
                result.started_at.to_rfc3339(),
                result.completed_at.to_rfc3339(),
                result.error,
                payload
            ],
        )?;
        tx.execute(
            "INSERT INTO test_executions (id, run_id, project_id, test_id, mode, status, started_at, completed_at, error)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
            params![
                uuid::Uuid::new_v4().to_string(),
                run_id,
                project.as_str(),
                result.test_id.as_str(),
                result.mode.as_str(),
                result.status.as_str(),
                result.started_at.to_rfc3339(),
                result.completed_at.to_rfc3339(),
                result.error,
            ],
        )?;
        for measurement in &result.measurements {
            tx.execute(
                "INSERT INTO measurements (id, result_id, name, payload) VALUES (?1, ?2, ?3, ?4)",
                params![
                    uuid::Uuid::new_v4().to_string(),
                    run_id,
                    measurement.name,
                    serde_json::to_string(measurement)?
                ],
            )?;
        }
        for evidence in &result.evidence {
            tx.execute(
                "INSERT INTO evidence_meta (id, project_id, result_id, kind, relative_path, description, captured_at, sha256)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
                params![
                    evidence.id.as_str(),
                    project.as_str(),
                    run_id,
                    serde_json::to_string(&evidence.kind)?,
                    evidence.relative_path,
                    evidence.description,
                    evidence.captured_at.to_rfc3339(),
                    evidence.sha256,
                ],
            )?;
        }
        tx.commit()?;
        Ok(())
    }

    /// Load all results for a given run.
    pub fn results_for_run(&self, run_id: &str) -> Result<Vec<TestResult>, ProjectStoreError> {
        let mut stmt = self
            .conn
            .prepare("SELECT payload FROM results WHERE run_id = ?1 ORDER BY started_at")?;
        let rows = stmt.query_map(params![run_id], |r| {
            let payload: String = r.get(0)?;
            serde_json::from_str(&payload).map_err(|e: serde_json::Error| {
                rusqlite::Error::FromSqlConversionFailure(
                    0,
                    rusqlite::types::Type::Text,
                    Box::new(e),
                )
            })
        })?;
        Ok(rows.collect::<Result<Vec<_>, _>>()?)
    }

    /// Distinct run ids, most recent first.
    pub fn list_runs(&self) -> Result<Vec<String>, ProjectStoreError> {
        let mut stmt = self
            .conn
            .prepare("SELECT run_id FROM results GROUP BY run_id ORDER BY MAX(started_at) DESC")?;
        let rows = stmt.query_map([], |r| r.get::<_, String>(0))?;
        Ok(rows.collect::<Result<Vec<_>, _>>()?)
    }

    // -- defects -----------------------------------------------------------

    /// Save a defect (§23). Inserting a second defect with the same id fails
    /// so recorded defects are never silently overwritten.
    pub fn save_defect(&mut self, defect: &Defect) -> Result<(), ProjectStoreError> {
        let payload = serde_json::to_string(defect)?;
        self.conn.execute(
            "INSERT INTO defects (id, project_id, severity, title, description, related_tests, status, created_at, payload)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
            params![
                defect.id.as_str(),
                defect.project.as_str(),
                defect.severity.as_str(),
                defect.title,
                defect.description,
                serde_json::to_string(&defect.related_tests)?,
                defect.status.as_str(),
                defect.created_at.to_rfc3339(),
                payload,
            ],
        )?;
        Ok(())
    }

    /// Update a saved defect's status (§23 defect workflow). The payload is
    /// updated alongside the column so reloaded defects agree.
    pub fn update_defect_status(
        &mut self,
        id: &str,
        status: DefectStatus,
    ) -> Result<(), ProjectStoreError> {
        let changed = self.conn.execute(
            "UPDATE defects SET status = ?1, payload = json_set(payload, '$.status', ?2) WHERE id = ?3",
            params![status.as_str(), status.as_str(), id],
        )?;
        if changed == 0 {
            return Err(ProjectStoreError::MissingDefect(id.to_owned()));
        }
        Ok(())
    }

    /// All defects for a project, oldest first.
    pub fn list_defects(&self, project: &ProjectId) -> Result<Vec<Defect>, ProjectStoreError> {
        let mut stmt = self
            .conn
            .prepare("SELECT payload FROM defects WHERE project_id = ?1 ORDER BY created_at")?;
        let rows = stmt.query_map(params![project.as_str()], |r| {
            let payload: String = r.get(0)?;
            serde_json::from_str(&payload).map_err(|e| {
                rusqlite::Error::FromSqlConversionFailure(
                    0,
                    rusqlite::types::Type::Text,
                    Box::new(e),
                )
            })
        })?;
        Ok(rows.collect::<Result<Vec<_>, _>>()?)
    }

    // -- configuration baselines -------------------------------------------

    /// Save a baseline snapshot. `(project, label)` is unique: saving a
    /// second time with the same label fails so the previous snapshot is
    /// preserved (§27).
    pub fn save_baseline(&mut self, baseline: &Baseline) -> Result<(), ProjectStoreError> {
        let payload = serde_json::to_string(baseline)?;
        let result = self.conn.execute(
            "INSERT OR IGNORE INTO configuration_baselines (id, project_id, label, fingerprint, payload, created_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            params![
                baseline.id,
                baseline.project.as_str(),
                baseline.label,
                baseline.fingerprint(),
                payload,
                baseline.created_at.to_rfc3339()
            ],
        )?;
        if result == 0 {
            return Err(ProjectStoreError::BaselineExists(baseline.label.clone()));
        }
        Ok(())
    }

    /// Summary rows for saved baselines, oldest first.
    pub fn list_baselines(
        &self,
        project: &ProjectId,
    ) -> Result<Vec<StoredBaseline>, ProjectStoreError> {
        let mut stmt = self.conn.prepare(
            "SELECT id, label, fingerprint, payload, created_at FROM configuration_baselines WHERE project_id = ?1 ORDER BY created_at",
        )?;
        let rows = stmt.query_map(params![project.as_str()], |r| {
            Ok(StoredBaseline {
                id: r.get(0)?,
                label: r.get(1)?,
                fingerprint: r.get(2)?,
                payload: r.get(3)?,
                created_at: r.get(4)?,
            })
        })?;
        Ok(rows.collect::<Result<Vec<_>, _>>()?)
    }

    /// Load a full baseline snapshot by label.
    pub fn load_baseline(
        &self,
        project: &ProjectId,
        label: &str,
    ) -> Result<Option<Baseline>, ProjectStoreError> {
        let payload: Option<String> = self
            .conn
            .query_row(
                "SELECT payload FROM configuration_baselines WHERE project_id = ?1 AND label = ?2",
                params![project.as_str(), label],
                |r| r.get(0),
            )
            .optional()?;
        payload
            .map(|p| serde_json::from_str(&p).map_err(ProjectStoreError::Json))
            .transpose()
    }

    // -- audit log -----------------------------------------------------------

    /// Record a significant project action (§33).
    pub fn append_audit_event(
        &mut self,
        project: &ProjectId,
        event: &AuditEvent,
    ) -> Result<(), ProjectStoreError> {
        self.conn.execute(
            "INSERT INTO audit_log (project_id, timestamp, event_type, actor, object_id, details)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            params![
                project.as_str(),
                event.timestamp.to_rfc3339(),
                event.event_type.as_str(),
                event.actor.display(),
                event.object_id,
                serde_json::to_string(&event.details)?,
            ],
        )?;
        Ok(())
    }

    /// The project's audit trail, chronological.
    pub fn list_audit_events(
        &self,
        project: &ProjectId,
    ) -> Result<Vec<AuditEvent>, ProjectStoreError> {
        let mut stmt = self.conn.prepare(
            "SELECT timestamp, event_type, actor, object_id, details
             FROM audit_log WHERE project_id = ?1 ORDER BY id",
        )?;
        let rows = stmt.query_map(params![project.as_str()], |r| {
            Ok((
                r.get::<_, String>(0)?,
                r.get::<_, String>(1)?,
                r.get::<_, String>(2)?,
                r.get::<_, Option<String>>(3)?,
                r.get::<_, String>(4)?,
            ))
        })?;
        let mut events = Vec::new();
        for row in rows {
            let (timestamp, event_type, actor, object_id, details) = row?;
            events.push(AuditEvent {
                timestamp: DateTime::parse_from_rfc3339(&timestamp)
                    .map_err(|e| {
                        rusqlite::Error::FromSqlConversionFailure(
                            0,
                            rusqlite::types::Type::Text,
                            Box::new(e),
                        )
                    })?
                    .with_timezone(&Utc),
                event_type: audit_event_type(&event_type)?,
                actor: if actor == "system" {
                    Actor::System
                } else {
                    Actor::User(actor)
                },
                object_id,
                details: serde_json::from_str(&details)?,
            });
        }
        Ok(events)
    }

    /// Evidence metadata for a project.
    pub fn list_evidence(
        &self,
        project: &ProjectId,
    ) -> Result<Vec<EvidenceRef>, ProjectStoreError> {
        let mut stmt = self.conn.prepare(
            "SELECT id, kind, relative_path, description, captured_at, sha256
             FROM evidence_meta WHERE project_id = ?1 ORDER BY captured_at",
        )?;
        let rows = stmt.query_map(params![project.as_str()], |r| {
            let kind_str: String = r.get(1)?;
            let captured: String = r.get(4)?;
            Ok(EvidenceRef {
                id: tpt_app_av_commissioning_model::EvidenceId::new(r.get::<_, String>(0)?),
                project: project.clone(),
                kind: serde_json::from_str(&kind_str).unwrap_or(EvidenceKind::OperatorNote),
                relative_path: r.get(2)?,
                description: r.get(3)?,
                captured_at: chrono::DateTime::parse_from_rfc3339(&captured)
                    .map(|t| t.with_timezone(&Utc))
                    .unwrap_or_else(|_| Utc::now()),
                sha256: r.get(5)?,
            })
        })?;
        Ok(rows.collect::<Result<Vec<_>, _>>()?)
    }
}

/// Managed project asset layout (§25).
///
/// ```text
/// Project/
/// ├── project.sqlite
/// ├── assets/
/// │   ├── screenshots/
/// │   ├── recordings/
/// │   ├── configurations/
/// │   └── evidence/
/// ├── reports/
/// └── exports/
/// ```
#[derive(Debug, Clone)]
pub struct ProjectAssets {
    root: PathBuf,
}

impl ProjectAssets {
    /// Create a handle for the asset layout rooted under `project_dir`.
    pub fn new(project_dir: &Path) -> Self {
        Self {
            root: project_dir.to_path_buf(),
        }
    }

    /// The project directory that roots this asset layout.
    pub fn root(&self) -> &Path {
        &self.root
    }

    /// Create the full directory layout.
    pub fn ensure_layout(&self) -> Result<(), ProjectStoreError> {
        for path in self.dirs() {
            fs::create_dir_all(&path)?;
        }
        Ok(())
    }

    /// All managed directories.
    pub fn dirs(&self) -> Vec<PathBuf> {
        vec![
            self.root.join("assets/screenshots"),
            self.root.join("assets/recordings"),
            self.root.join("assets/configurations"),
            self.root.join("assets/evidence"),
            self.root.join("reports"),
            self.root.join("exports"),
        ]
    }

    /// `assets/evidence` directory.
    pub fn evidence_dir(&self) -> PathBuf {
        self.root.join("assets/evidence")
    }

    /// `assets/configurations` directory.
    pub fn configurations_dir(&self) -> PathBuf {
        self.root.join("assets/configurations")
    }

    /// `reports` directory.
    pub fn reports_dir(&self) -> PathBuf {
        self.root.join("reports")
    }

    /// `exports` directory.
    pub fn exports_dir(&self) -> PathBuf {
        self.root.join("exports")
    }

    /// Write an evidence asset as an immutable file (`next-index` naming).
    ///
    /// Drives the SHA-256 hash into the returned metadata. Refuses to
    /// overwrite any existing file.
    pub fn add_evidence(
        &self,
        run_id: &str,
        project: &ProjectId,
        kind: EvidenceKind,
        bytes: &[u8],
        extension: &str,
        description: Option<String>,
    ) -> Result<EvidenceRef, ProjectStoreError> {
        self.ensure_layout()?;
        let run_dir = self.evidence_dir().join(run_id);
        fs::create_dir_all(&run_dir)?;

        let mut index = 0usize;
        let path = loop {
            let candidate = run_dir.join(format!("{index:04}.{extension}"));
            if !candidate.exists() {
                break candidate;
            }
            index += 1;
        };

        if path.exists() {
            return Err(ProjectStoreError::EvidenceExists(path));
        }
        let mut file = fs::File::create(&path)?;
        file.write_all(bytes)?;
        file.flush()?;

        let sha256 = hex(&Sha256::digest(bytes));

        let relative = path
            .strip_prefix(&self.root)
            .unwrap_or(&path)
            .to_string_lossy()
            .replace('\\', "/");

        Ok(EvidenceRef {
            id: tpt_app_av_commissioning_model::EvidenceId::new(uuid::Uuid::new_v4().to_string()),
            project: project.clone(),
            kind,
            relative_path: relative,
            description,
            captured_at: Utc::now(),
            sha256: Some(sha256),
        })
    }

    /// Read an evidence asset back from its relative path.
    pub fn read_evidence(&self, relative_path: &str) -> Result<Vec<u8>, ProjectStoreError> {
        let path = self.root.join(relative_path);
        Ok(fs::read(path)?)
    }
}

/// Format a digest as lowercase hex.
fn hex(digest: &[u8]) -> String {
    digest.iter().map(|b| format!("{b:02x}")).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use tpt_app_av_commissioning_model::{DeviceAddress, DeviceType, EndpointKind};
    use tpt_app_av_commissioning_test::{ExecutionMode, TestId, TestStatus};

    fn tmp_dir(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("tpt-av-comm-{name}-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn meta() -> ProjectMeta {
        ProjectMeta {
            id: ProjectId::new("prj-1"),
            name: "Boardroom".to_owned(),
            client: Some("Example Corp".to_owned()),
            site: None,
        }
    }

    #[test]
    fn create_opens_and_reads_meta() {
        let dir = tmp_dir("meta");
        let store = ProjectStore::create(&dir, &meta()).unwrap();
        drop(store);
        let store = ProjectStore::open(&dir).unwrap();
        assert_eq!(store.project_meta().unwrap().unwrap().name, "Boardroom");
        drop(store);
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn create_refuses_to_clobber() {
        let dir = tmp_dir("clobber");
        ProjectStore::create(&dir, &meta()).unwrap();
        let err = ProjectStore::create(&dir, &meta()).unwrap_err();
        assert!(err.to_string().contains("already exists"));
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn asset_layout_is_created() {
        let dir = tmp_dir("layout");
        ProjectStore::create(&dir, &meta()).unwrap();
        let assets = ProjectAssets::new(&dir);
        for d in assets.dirs() {
            assert!(d.exists(), "missing {}", d.display());
        }
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn device_round_trip() {
        let dir = tmp_dir("device");
        let mut store = ProjectStore::create(&dir, &meta()).unwrap();
        let mut device = Device::new(
            tpt_app_av_commissioning_model::DeviceId::new("p1"),
            "Projector 1",
            DeviceType::Projector,
        );
        device
            .addresses
            .push(DeviceAddress::Ip("192.168.1.40".parse().unwrap()));
        store
            .upsert_device(&ProjectId::new("prj-1"), &device)
            .unwrap();
        let devices = store.list_devices().unwrap();
        assert_eq!(devices.len(), 1);
        assert_eq!(devices[0].addresses.len(), 1);
        drop(store);
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn endpoint_and_connection_round_trip() {
        let dir = tmp_dir("conn");
        let mut store = ProjectStore::create(&dir, &meta()).unwrap();
        let project = ProjectId::new("prj-1");

        store
            .upsert_device(
                &project,
                &Device::new(
                    tpt_app_av_commissioning_model::DeviceId::new("m1"),
                    "Matrix",
                    DeviceType::Matrix,
                ),
            )
            .unwrap();
        store
            .upsert_endpoint(&Endpoint::new(
                tpt_app_av_commissioning_model::EndpointId::new("m1-in1"),
                "In 1",
                EndpointKind::VideoInput,
                tpt_app_av_commissioning_model::DeviceId::new("m1"),
            ))
            .unwrap();
        store
            .upsert_endpoint(&Endpoint::new(
                tpt_app_av_commissioning_model::EndpointId::new("m1-out1"),
                "Out 1",
                EndpointKind::VideoOutput,
                tpt_app_av_commissioning_model::DeviceId::new("m1"),
            ))
            .unwrap();

        let connection = ModelConnection::new(
            tpt_app_av_commissioning_model::ConnectionId::new("c1"),
            tpt_app_av_commissioning_model::EndpointId::new("m1-in1"),
            tpt_app_av_commissioning_model::EndpointId::new("m1-out1"),
            tpt_app_av_commissioning_model::SignalType::Video,
            tpt_app_av_commissioning_model::Transport::Hdmi,
        );
        store.upsert_connection(&project, &connection).unwrap();
        assert_eq!(store.list_connections().unwrap().len(), 1);
        drop(store);
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn result_and_evidence_persist_across_reopen() {
        let dir = tmp_dir("results");
        let project = ProjectId::new("prj-1");
        {
            let mut store = ProjectStore::create(&dir, &meta()).unwrap();
            let assets = store.assets();
            let evidence = assets
                .add_evidence(
                    "run-1",
                    &project,
                    EvidenceKind::Screenshot,
                    b"png-bytes",
                    "png",
                    Some("signal present".to_owned()),
                )
                .unwrap();
            assert!(evidence.sha256.is_some());
            // Indexed naming means a second asset never collides/overwrites.
            let evidence2 = assets
                .add_evidence(
                    "run-1",
                    &project,
                    EvidenceKind::DeviceResponse,
                    b"other",
                    "json",
                    None,
                )
                .unwrap();

            let result = TestResult::new(
                TestId::new("power-on"),
                TestStatus::Pass,
                ExecutionMode::Automated,
            )
            .with_evidence(evidence.clone())
            .with_evidence(evidence2);
            store.save_result("run-1", &project, &result).unwrap();
        }
        let store = ProjectStore::open(&dir).unwrap();
        assert_eq!(store.results_for_run("run-1").unwrap().len(), 1);
        assert_eq!(store.list_runs().unwrap(), vec!["run-1".to_owned()]);
        assert_eq!(store.list_evidence(&project).unwrap().len(), 2);
        drop(store);
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn baseline_label_is_unique() {
        let dir = tmp_dir("baseline");
        let project = ProjectId::new("prj-1");
        let mut store = ProjectStore::create(&dir, &meta()).unwrap();
        let make = |label: &str, serial: &str| {
            let mut device = Device::new(
                tpt_app_av_commissioning_model::DeviceId::new("p1"),
                "Projector",
                DeviceType::Projector,
            );
            device.serial_number = Some(serial.to_owned());
            Baseline::create(label, project.clone(), &[device], &[], &[])
        };
        let first = make("after-commissioning", "SN-1");
        let fingerprint = first.fingerprint();
        store.save_baseline(&first).unwrap();
        let second = make("after-commissioning", "SN-2");
        let err = store.save_baseline(&second).unwrap_err();
        assert!(matches!(err, ProjectStoreError::BaselineExists(_)));
        // The original snapshot is preserved.
        assert_eq!(
            store.list_baselines(&project).unwrap()[0].fingerprint,
            fingerprint
        );
        let loaded = store
            .load_baseline(&project, "after-commissioning")
            .unwrap()
            .unwrap();
        assert_eq!(loaded.fingerprint(), fingerprint);
        assert_eq!(loaded.devices[0].serial_number.as_deref(), Some("SN-1"));
        drop(store);
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn defect_round_trip() {
        let dir = tmp_dir("defect");
        let project = ProjectId::new("prj-1");
        let mut store = ProjectStore::create(&dir, &meta()).unwrap();
        let mut defect = Defect::new(
            "d-1",
            project.clone(),
            tpt_app_av_commissioning_model::Severity::Major,
            "Display does not lock signal on input 3",
            "only on warm reboots",
        );
        defect.relate_test("display-input");
        store.save_defect(&defect).unwrap();

        store
            .update_defect_status("d-1", DefectStatus::RetestRequired)
            .unwrap();
        assert!(matches!(
            store.update_defect_status("missing", DefectStatus::Open),
            Err(ProjectStoreError::MissingDefect(_))
        ));

        let defects = store.list_defects(&project).unwrap();
        assert_eq!(defects.len(), 1);
        assert_eq!(defects[0].severity.as_str(), "major");
        assert_eq!(defects[0].related_tests, vec!["display-input"]);
        assert_eq!(defects[0].status, DefectStatus::RetestRequired);
        drop(store);
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn audit_events_are_chronological_and_typed() {
        let dir = tmp_dir("audit");
        let project = ProjectId::new("prj-1");
        let mut store = ProjectStore::create(&dir, &meta()).unwrap();
        for (event_type, object) in [
            (AuditEventType::ProjectCreated, None),
            (AuditEventType::TestRun, Some("run-1".to_owned())),
            (AuditEventType::DefectCreated, Some("d-1".to_owned())),
        ] {
            store
                .append_audit_event(
                    &project,
                    &AuditEvent::now(
                        event_type,
                        if event_type == AuditEventType::TestRun {
                            tpt_app_av_commissioning_model::Actor::System
                        } else {
                            tpt_app_av_commissioning_model::Actor::User("J. Doe".into())
                        },
                        object,
                        serde_json::json!({ "note": "ok" }),
                    ),
                )
                .unwrap();
        }
        let events = store.list_audit_events(&project).unwrap();
        assert_eq!(events.len(), 3);
        assert_eq!(events[0].event_type, AuditEventType::ProjectCreated);
        assert_eq!(events[1].event_type, AuditEventType::TestRun);
        assert_eq!(
            events[1].actor,
            tpt_app_av_commissioning_model::Actor::System
        );
        assert_eq!(events[2].object_id.as_deref(), Some("d-1"));
        assert!(events.windows(2).all(|w| w[0].timestamp <= w[1].timestamp));
        drop(store);
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn evidence_content_reads_back_unmodified() {
        let dir = tmp_dir("evidence-read");
        let project = ProjectId::new("prj-1");
        let store = ProjectStore::create(&dir, &meta()).unwrap();
        let assets = store.assets();
        let evidence = assets
            .add_evidence(
                "run-1",
                &project,
                EvidenceKind::Screenshot,
                b"\x89PNG123",
                "png",
                None,
            )
            .unwrap();
        assert_eq!(
            assets.read_evidence(&evidence.relative_path).unwrap(),
            b"\x89PNG123"
        );
        drop(store);
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
