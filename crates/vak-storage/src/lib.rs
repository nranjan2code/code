//! Greenfield persistence primitives for vakcoder.
//!
//! SQLite is the authority for mutable control-plane state; the audit log and
//! blob store are append-only/content-addressed respectively.

use rusqlite::{Connection, OptionalExtension, params};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};
use thiserror::Error;

#[derive(Debug, Error)]
pub enum StorageError {
    #[error("sqlite: {0}")]
    Sqlite(#[from] rusqlite::Error),
    #[error("io: {0}")]
    Io(#[from] std::io::Error),
    #[error("json: {0}")]
    Json(#[from] serde_json::Error),
    #[error("invalid blob digest: {0}")]
    InvalidDigest(String),
    #[error("unsupported storage operation: {0}")]
    Unsupported(String),
    #[error("unsupported state schema version: {0}")]
    SchemaVersion(i64),
    #[error("state conflict: {0}")]
    Conflict(String),
}

pub type Result<T> = std::result::Result<T, StorageError>;

/// Opens the authoritative transactional state database and initializes its schema.
pub struct StateStore {
    conn: Connection,
}

impl StateStore {
    pub fn open(path: impl AsRef<Path>) -> Result<Self> {
        let conn = Connection::open(path)?;
        conn.pragma_update(None, "foreign_keys", "ON")?;
        conn.pragma_update(None, "journal_mode", "WAL")?;
        conn.pragma_update(None, "busy_timeout", 5_000i64)?;
        let store = Self { conn };
        store.initialize_schema()?;
        Ok(store)
    }

    pub fn open_memory() -> Result<Self> {
        let conn = Connection::open_in_memory()?;
        conn.pragma_update(None, "foreign_keys", "ON")?;
        conn.pragma_update(None, "busy_timeout", 5_000i64)?;
        let store = Self { conn };
        store.initialize_schema()?;
        Ok(store)
    }

    fn initialize_schema(&self) -> Result<()> {
        let version: i64 = self
            .conn
            .pragma_query_value(None, "user_version", |row| row.get(0))?;
        if version == 1 {
            return Ok(());
        }
        if version != 0 {
            return Err(StorageError::SchemaVersion(version));
        }
        self.conn.execute_batch(
            "CREATE TABLE projects (
                 id TEXT PRIMARY KEY, root TEXT NOT NULL UNIQUE, created_at TEXT NOT NULL,
                 metadata_json TEXT NOT NULL DEFAULT '{}'
             );
             CREATE TABLE sessions (
                 id TEXT PRIMARY KEY, project_id TEXT NOT NULL REFERENCES projects(id),
                 created_at TEXT NOT NULL, status TEXT NOT NULL, contract_json TEXT NOT NULL
             );
             CREATE INDEX sessions_project_idx ON sessions(project_id);
             CREATE TABLE runs (
                 id TEXT PRIMARY KEY, session_id TEXT NOT NULL REFERENCES sessions(id),
                 status TEXT NOT NULL, created_at TEXT NOT NULL, finished_at TEXT,
                 capability_epoch INTEGER NOT NULL, outcome_json TEXT
             );
             CREATE INDEX runs_session_idx ON runs(session_id);
             CREATE UNIQUE INDEX runs_one_active_per_session ON runs(session_id)
                 WHERE status IN ('queued','admitted','running','waiting_approval','cancelling');
             CREATE TABLE tasks (
                 id TEXT PRIMARY KEY, project_id TEXT REFERENCES projects(id),
                 spec_json TEXT NOT NULL, status TEXT NOT NULL, updated_at TEXT NOT NULL
             );
             CREATE TABLE approvals (
                 id TEXT PRIMARY KEY, run_id TEXT NOT NULL REFERENCES runs(id),
                 status TEXT NOT NULL, request_json TEXT NOT NULL, response_json TEXT,
                 created_at TEXT NOT NULL, resolved_at TEXT
             );
             CREATE TABLE bindings (
                 id TEXT PRIMARY KEY, channel TEXT NOT NULL, external_key TEXT NOT NULL,
                 target_json TEXT NOT NULL, created_at TEXT NOT NULL,
                 UNIQUE(channel, external_key)
             );
             CREATE TABLE inbox (
                 id TEXT PRIMARY KEY, channel TEXT NOT NULL, external_id TEXT,
                 payload_json TEXT NOT NULL, created_at TEXT NOT NULL, acknowledged_at TEXT
             );
             CREATE TABLE delivery_jobs (
                 id TEXT PRIMARY KEY, channel TEXT NOT NULL, payload_json TEXT NOT NULL,
                 status TEXT NOT NULL, attempts INTEGER NOT NULL DEFAULT 0,
                 next_attempt_at TEXT, updated_at TEXT NOT NULL, error TEXT
             );
             CREATE TABLE config_revisions (
                 revision INTEGER PRIMARY KEY AUTOINCREMENT, scope TEXT NOT NULL,
                 config_json TEXT NOT NULL, created_at TEXT NOT NULL, actor TEXT NOT NULL
             );
             CREATE INDEX config_scope_idx ON config_revisions(scope, revision);
             CREATE TABLE blobs (
                 digest TEXT PRIMARY KEY, size INTEGER NOT NULL, created_at TEXT NOT NULL
             );
             PRAGMA user_version = 1;",
        )?;
        self.initialize_aux_schema()?;
        Ok(())
    }

    fn initialize_aux_schema(&self) -> Result<()> {
        self.conn.execute_batch(
            "CREATE TABLE IF NOT EXISTS memory (
                id TEXT PRIMARY KEY, project_id TEXT, scope TEXT NOT NULL,
                kind TEXT NOT NULL, tag TEXT NOT NULL, text TEXT NOT NULL,
                created_at TEXT NOT NULL, updated_at TEXT NOT NULL, deleted_at TEXT
             );
             CREATE INDEX IF NOT EXISTS memory_scope_idx ON memory(project_id,scope,deleted_at);
             CREATE TABLE IF NOT EXISTS skills (
                id TEXT PRIMARY KEY, project_id TEXT, name TEXT NOT NULL,
                description TEXT NOT NULL, body_digest TEXT, status TEXT NOT NULL,
                created_at TEXT NOT NULL, updated_at TEXT NOT NULL,
                UNIQUE(project_id,name)
             );
             CREATE TABLE IF NOT EXISTS checkpoints (
                id TEXT PRIMARY KEY, session_id TEXT NOT NULL REFERENCES sessions(id),
                manifest_digest TEXT NOT NULL, created_at TEXT NOT NULL, label TEXT NOT NULL
             );
             CREATE INDEX IF NOT EXISTS checkpoints_session_idx ON checkpoints(session_id,created_at);
             CREATE TABLE IF NOT EXISTS documents (
                id TEXT PRIMARY KEY, project_id TEXT, source TEXT NOT NULL,
                title TEXT NOT NULL, body_digest TEXT, body TEXT NOT NULL,
                created_at TEXT NOT NULL, updated_at TEXT NOT NULL
             );
             CREATE INDEX IF NOT EXISTS documents_project_idx ON documents(project_id,updated_at);
             CREATE TABLE IF NOT EXISTS backup_manifests (
                id TEXT PRIMARY KEY, manifest_json TEXT NOT NULL, created_at TEXT NOT NULL,
                status TEXT NOT NULL
             );
             CREATE TABLE IF NOT EXISTS usage_ledger (
                id TEXT PRIMARY KEY, run_id TEXT, provider TEXT NOT NULL, model TEXT NOT NULL,
                input_tokens INTEGER NOT NULL, output_tokens INTEGER NOT NULL,
                cost_cents INTEGER, created_at TEXT NOT NULL
             );
             CREATE INDEX IF NOT EXISTS usage_created_idx ON usage_ledger(created_at);
             CREATE TABLE IF NOT EXISTS budgets (
                scope TEXT PRIMARY KEY, cap_cents INTEGER, used_cents INTEGER NOT NULL DEFAULT 0,
                updated_at TEXT NOT NULL
             );
             CREATE TABLE IF NOT EXISTS diagnostics (
                id TEXT PRIMARY KEY, kind TEXT NOT NULL, status TEXT NOT NULL,
                detail_json TEXT NOT NULL, created_at TEXT NOT NULL
             );"
        )?;
        Ok(())
    }

    pub fn connection(&self) -> &Connection {
        &self.conn
    }

    pub fn put_project(
        &self,
        id: &str,
        root: &str,
        created_at: &str,
        metadata_json: &str,
    ) -> Result<()> {
        self.conn.execute(
            "INSERT INTO projects(id,root,created_at,metadata_json) VALUES(?1,?2,?3,?4)
             ON CONFLICT(id) DO UPDATE SET root=excluded.root, metadata_json=excluded.metadata_json",
            params![id, root, created_at, metadata_json],
        )?;
        Ok(())
    }

    pub fn create_project(
        &self,
        id: &str,
        root: &str,
        created_at: &str,
        metadata_json: &str,
    ) -> Result<()> {
        match self.conn.execute(
            "INSERT INTO projects(id,root,created_at,metadata_json) VALUES(?1,?2,?3,?4)",
            params![id, root, created_at, metadata_json],
        ) {
            Ok(_) => Ok(()),
            Err(rusqlite::Error::SqliteFailure(_, Some(message)))
                if message.contains("UNIQUE") || message.contains("PRIMARY KEY") =>
            {
                Err(StorageError::Conflict(message))
            }
            Err(error) => Err(error.into()),
        }
    }

    pub fn put_session(
        &self,
        id: &str,
        project_id: &str,
        created_at: &str,
        status: &str,
        contract_json: &str,
    ) -> Result<()> {
        self.conn.execute(
            "INSERT INTO sessions(id,project_id,created_at,status,contract_json) VALUES(?1,?2,?3,?4,?5)
             ON CONFLICT(id) DO UPDATE SET status=excluded.status, contract_json=excluded.contract_json",
            params![id, project_id, created_at, status, contract_json],
        )?;
        Ok(())
    }

    pub fn create_session(
        &self,
        id: &str,
        project_id: &str,
        created_at: &str,
        status: &str,
        contract_json: &str,
    ) -> Result<()> {
        match self.conn.execute("INSERT INTO sessions(id,project_id,created_at,status,contract_json) VALUES(?1,?2,?3,?4,?5)", params![id, project_id, created_at, status, contract_json]) {
            Ok(_) => Ok(()),
            Err(rusqlite::Error::SqliteFailure(_, Some(message))) if message.contains("UNIQUE") || message.contains("PRIMARY KEY") || message.contains("FOREIGN KEY") => Err(StorageError::Conflict(message)),
            Err(error) => Err(error.into()),
        }
    }

    pub fn put_run(
        &self,
        id: &str,
        session_id: &str,
        status: &str,
        created_at: &str,
        capability_epoch: i64,
    ) -> Result<()> {
        self.conn.execute(
            "INSERT INTO runs(id,session_id,status,created_at,capability_epoch) VALUES(?1,?2,?3,?4,?5)
             ON CONFLICT(id) DO UPDATE SET status=excluded.status, capability_epoch=excluded.capability_epoch",
            params![id, session_id, status, created_at, capability_epoch],
        )?;
        Ok(())
    }

    /// Inserts a run without replacing an existing run or active sibling.
    pub fn create_run(
        &self,
        id: &str,
        session_id: &str,
        status: &str,
        created_at: &str,
        capability_epoch: i64,
    ) -> Result<()> {
        match self.conn.execute("INSERT INTO runs(id,session_id,status,created_at,capability_epoch) VALUES(?1,?2,?3,?4,?5)", params![id, session_id, status, created_at, capability_epoch]) {
            Ok(_) => Ok(()),
            Err(rusqlite::Error::SqliteFailure(_, Some(message))) if message.contains("UNIQUE") || message.contains("PRIMARY KEY") => Err(StorageError::Conflict(message)),
            Err(error) => Err(error.into()),
        }
    }

    pub fn finish_run(
        &self,
        id: &str,
        status: &str,
        finished_at: &str,
        outcome_json: &str,
    ) -> Result<bool> {
        let changed = self.conn.execute(
            "UPDATE runs SET status=?2, finished_at=?3, outcome_json=?4 WHERE id=?1 AND finished_at IS NULL",
            params![id, status, finished_at, outcome_json],
        )?;
        Ok(changed == 1)
    }

    pub fn get_run_status(&self, id: &str) -> Result<Option<String>> {
        Ok(self
            .conn
            .query_row("SELECT status FROM runs WHERE id=?1", [id], |row| {
                row.get(0)
            })
            .optional()?)
    }

    pub fn get_run(&self, id: &str) -> Result<Option<RunRecord>> {
        Ok(self.conn.query_row("SELECT id,session_id,status,created_at,finished_at,capability_epoch,outcome_json FROM runs WHERE id=?1", [id], |row| {
            Ok(RunRecord {
                id: row.get(0)?, session_id: row.get(1)?, status: row.get(2)?, created_at: row.get(3)?,
                finished_at: row.get(4)?, capability_epoch: row.get(5)?, outcome_json: row.get(6)?,
            })
        }).optional()?)
    }

    /// Converts runs left active by a crashed process into terminal recovery records.
    pub fn recover_active_runs(&self, finished_at: &str) -> Result<usize> {
        Ok(self.conn.execute("UPDATE runs SET status='interrupted',finished_at=?1,outcome_json=?2 WHERE finished_at IS NULL AND status IN ('queued','admitted','running','waiting_approval','cancelling')", params![finished_at, r#"{"reason":"runtime_restarted"}"#])?)
    }

    pub fn list_projects(&self) -> Result<Vec<(String, String, String)>> {
        let mut stmt = self
            .conn
            .prepare("SELECT id, root, metadata_json FROM projects ORDER BY created_at")?;
        let rows = stmt.query_map([], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, String>(2)?,
            ))
        })?;
        Ok(rows.collect::<std::result::Result<Vec<_>, _>>()?)
    }

    pub fn get_session(&self, id: &str) -> Result<Option<(String, String, String)>> {
        Ok(self
            .conn
            .query_row(
                "SELECT project_id, created_at, contract_json FROM sessions WHERE id=?1",
                [id],
                |row| {
                    Ok((
                        row.get::<_, String>(0)?,
                        row.get::<_, String>(1)?,
                        row.get::<_, String>(2)?,
                    ))
                },
            )
            .optional()?)
    }

    pub fn list_sessions(
        &self,
        project_id: Option<&str>,
    ) -> Result<Vec<(String, String, String, String)>> {
        let mut stmt = self.conn.prepare("SELECT id,project_id,created_at,status FROM sessions WHERE (?1 IS NULL OR project_id=?1) ORDER BY created_at")?;
        let rows = stmt.query_map([project_id], |row| {
            Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?))
        })?;
        Ok(rows.collect::<std::result::Result<Vec<_>, _>>()?)
    }

    pub fn list_runs(
        &self,
        session_id: Option<&str>,
    ) -> Result<Vec<(String, String, String, i64)>> {
        let mut stmt = self.conn.prepare("SELECT id,session_id,status,capability_epoch FROM runs WHERE (?1 IS NULL OR session_id=?1) ORDER BY created_at")?;
        let rows = stmt.query_map([session_id], |row| {
            Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?))
        })?;
        Ok(rows.collect::<std::result::Result<Vec<_>, _>>()?)
    }

    pub fn session_project(&self, id: &str) -> Result<Option<String>> {
        Ok(self
            .conn
            .query_row("SELECT project_id FROM sessions WHERE id=?1", [id], |row| {
                row.get(0)
            })
            .optional()?)
    }
    pub fn session_root(&self, id: &str) -> Result<Option<String>> {
        Ok(self
            .conn
            .query_row(
                "SELECT p.root FROM projects p JOIN sessions s ON s.project_id=p.id WHERE s.id=?1",
                [id],
                |row| row.get(0),
            )
            .optional()?)
    }

    pub fn upsert_json(
        &self,
        table: &str,
        id: &str,
        payload_json: &str,
        status: &str,
        updated_at: &str,
    ) -> Result<()> {
        let sql = match table {
            "tasks" => {
                "INSERT INTO tasks(id,spec_json,status,updated_at) VALUES(?1,?2,?3,?4) ON CONFLICT(id) DO UPDATE SET spec_json=excluded.spec_json,status=excluded.status,updated_at=excluded.updated_at"
            }
            "delivery_jobs" => {
                "INSERT INTO delivery_jobs(id,channel,payload_json,status,updated_at) VALUES(?1,'',?2,?3,?4) ON CONFLICT(id) DO UPDATE SET payload_json=excluded.payload_json,status=excluded.status,updated_at=excluded.updated_at"
            }
            _ => return Err(StorageError::Unsupported(format!("table {table}"))),
        };
        self.conn
            .execute(sql, params![id, payload_json, status, updated_at])?;
        Ok(())
    }

    pub fn record_config_revision(
        &self,
        scope: &str,
        config_json: &str,
        created_at: &str,
        actor: &str,
    ) -> Result<i64> {
        self.conn.execute(
            "INSERT INTO config_revisions(scope,config_json,created_at,actor) VALUES(?1,?2,?3,?4)",
            params![scope, config_json, created_at, actor],
        )?;
        Ok(self.conn.last_insert_rowid())
    }

    pub fn register_blob(&self, digest: &str, size: i64, created_at: &str) -> Result<()> {
        self.conn.execute("INSERT INTO blobs(digest,size,created_at) VALUES(?1,?2,?3) ON CONFLICT(digest) DO NOTHING", params![digest, size, created_at])?;
        Ok(())
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct RunRecord {
    pub id: String,
    pub session_id: String,
    pub status: String,
    pub created_at: String,
    pub finished_at: Option<String>,
    pub capability_epoch: i64,
    pub outcome_json: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AuditEvent {
    pub event: String,
    pub timestamp: String,
    pub actor: String,
    #[serde(default)]
    pub data: serde_json::Value,
}

pub struct AuditWriter {
    path: PathBuf,
}

impl AuditWriter {
    pub fn open(path: impl Into<PathBuf>) -> Self {
        Self { path: path.into() }
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    pub fn append(&self, event: &AuditEvent) -> Result<()> {
        if let Some(parent) = self.path.parent() {
            fs::create_dir_all(parent)?;
        }
        let line = serde_json::to_vec(event)?;
        let mut file = OpenOptions::new()
            .create(true)
            .append(true)
            .open(&self.path)?;
        file.write_all(&line)?;
        file.write_all(b"\n")?;
        file.sync_data()?;
        Ok(())
    }
}

pub struct BlobStore {
    root: PathBuf,
}

impl BlobStore {
    pub fn open(root: impl Into<PathBuf>) -> Result<Self> {
        let root = root.into();
        fs::create_dir_all(&root)?;
        Ok(Self { root })
    }

    pub fn put(&self, bytes: &[u8]) -> Result<String> {
        let digest = format!("sha256:{:x}", Sha256::digest(bytes));
        let path = self.path_for(&digest)?;
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }
        if !path.exists() {
            let tmp = path.with_extension("tmp");
            fs::write(&tmp, bytes)?;
            fs::rename(tmp, &path)?;
        }
        Ok(digest)
    }

    pub fn get(&self, digest: &str) -> Result<Option<Vec<u8>>> {
        let path = self.path_for(digest)?;
        match fs::read(path) {
            Ok(bytes) => Ok(Some(bytes)),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
            Err(error) => Err(error.into()),
        }
    }

    pub fn contains(&self, digest: &str) -> Result<bool> {
        Ok(self.path_for(digest)?.exists())
    }

    fn path_for(&self, digest: &str) -> Result<PathBuf> {
        let hex = digest
            .strip_prefix("sha256:")
            .ok_or_else(|| StorageError::InvalidDigest(digest.to_owned()))?;
        if hex.len() != 64 || !hex.bytes().all(|b| b.is_ascii_hexdigit()) {
            return Err(StorageError::InvalidDigest(digest.to_owned()));
        }
        let dir = self.root.join(&hex[..2]);
        Ok(dir.join(hex))
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn state_schema_and_run_terminal_are_transactional() -> Result<()> {
        let store = StateStore::open_memory()?;
        store.put_project("p", "/tmp/p", "now", "{}")?;
        store.put_session("s", "p", "now", "active", "{}")?;
        store.put_run("r", "s", "running", "now", 2)?;
        assert!(store.finish_run("r", "cancelled", "later", "{}")?);
        assert!(!store.finish_run("r", "completed", "later", "{}")?);
        assert_eq!(store.get_run_status("r")?.as_deref(), Some("cancelled"));
        Ok(())
    }

    #[test]
    fn audit_appends_and_blobs_deduplicate() -> Result<()> {
        let dir = tempdir()?;
        let audit = AuditWriter::open(dir.path().join("audit.jsonl"));
        audit.append(&AuditEvent {
            event: "x".into(),
            timestamp: "now".into(),
            actor: "t".into(),
            data: serde_json::json!({"a":1}),
        })?;
        assert_eq!(
            fs::read_to_string(dir.path().join("audit.jsonl"))?
                .lines()
                .count(),
            1
        );
        let blobs = BlobStore::open(dir.path().join("blobs"))?;
        let a = blobs.put(b"hello")?;
        let b = blobs.put(b"hello")?;
        assert_eq!(a, b);
        assert_eq!(blobs.get(&a)?.as_deref(), Some(b"hello".as_slice()));
        assert_eq!(
            blobs.get("sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa")?,
            None
        );
        assert!(matches!(
            blobs.get("bad"),
            Err(StorageError::InvalidDigest(_))
        ));
        Ok(())
    }

    #[test]
    fn creation_is_conflict_safe_and_only_one_active_run_exists() -> Result<()> {
        let store = StateStore::open_memory()?;
        store.create_project("p", "/tmp/p", "now", "{}")?;
        assert!(matches!(
            store.create_project("p", "/tmp/p", "now", "{}"),
            Err(StorageError::Conflict(_))
        ));
        store.create_session("s", "p", "now", "active", "{}")?;
        assert!(matches!(
            store.create_session("s", "p", "now", "active", "{}"),
            Err(StorageError::Conflict(_))
        ));
        store.create_run("r1", "s", "running", "now", 1)?;
        assert!(matches!(
            store.create_run("r2", "s", "running", "now", 1),
            Err(StorageError::Conflict(_))
        ));
        assert_eq!(
            store.get_run("r1")?.map(|run| run.status),
            Some("running".into())
        );
        Ok(())
    }

    #[test]
    fn active_runs_are_recovered_after_restart() -> Result<()> {
        let dir = tempdir()?;
        let path = dir.path().join("state.db");
        {
            let store = StateStore::open(&path)?;
            store.create_project("p", "/tmp/p", "now", "{}")?;
            store.create_session("s", "p", "now", "active", "{}")?;
            store.create_run("r", "s", "running", "now", 1)?;
        }
        let store = StateStore::open(&path)?;
        assert_eq!(store.recover_active_runs("restart")?, 1);
        let run = store.get_run("r")?.expect("run persisted");
        assert_eq!(run.status, "interrupted");
        assert_eq!(run.finished_at.as_deref(), Some("restart"));
        assert_eq!(store.recover_active_runs("restart2")?, 0);
        Ok(())
    }

    #[test]
    fn blob_metadata_is_idempotent() -> Result<()> {
        let store = StateStore::open_memory()?;
        store.register_blob("sha256:abc", 3, "now")?;
        store.register_blob("sha256:abc", 99, "later")?;
        let count: i64 = store
            .connection()
            .query_row("SELECT count(*) FROM blobs", [], |row| row.get(0))?;
        assert_eq!(count, 1);
        Ok(())
    }
}
