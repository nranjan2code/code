//! vak-store: SQLite FTS5 rebuildable index over JSONL session ledgers.
//!
//! JSONL files remain the source of truth. This crate builds a persistent
//! query index (FTS5 for full-text search, normalized table for structured
//! queries) that can be fully reconstructed from the JSONL at any time.
//!
//! Design: `docs/design/23-memory.md` — rebuildable index layer.

use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use rusqlite::Connection;
use serde::{Deserialize, Serialize};

pub mod history;
pub mod index;
pub mod presentation;
pub mod query;

/// The database file name within the sessions home directory.
pub const DB_NAME: &str = "store.db";

/// Version tag stored in the `meta` table; bump when the schema changes.
/// Bumped whenever a table's shape changes; an index stamped with another
/// version is dropped and rebuilt, never adapted (it is derived data).
const SCHEMA_VERSION: u32 = 2;

/// A derived locator, not an authorization decision. A caller must verify
/// the canonical session's scope/lifecycle before loading this byte range.
#[derive(Debug, Clone)]
pub struct EntryLocator {
    pub entry_id: String,
    pub session_id: String,
    pub path: PathBuf,
    pub sequence: u64,
    pub offset: u64,
    pub length: u64,
    pub parent_id: Option<String>,
    pub digest: String,
}

// ---------------------------------------------------------------------------
// Entry metadata (written to the `entries` table alongside FTS rows)
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub enum EntryKind {
    Header,
    Message,
    Compaction,
    Receipt,
    Goal,
    Activity,
    Work,
    Intent,
    TurnCard,
}

impl EntryKind {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Header => "header",
            Self::Message => "message",
            Self::Compaction => "compaction",
            Self::Receipt => "receipt",
            Self::Goal => "goal",
            Self::Activity => "activity",
            Self::Work => "work",
            Self::Intent => "intent",
            Self::TurnCard => "turn_card",
        }
    }

    pub fn parse_str(s: &str) -> Option<Self> {
        match s {
            "header" => Some(Self::Header),
            "message" => Some(Self::Message),
            "compaction" => Some(Self::Compaction),
            "receipt" => Some(Self::Receipt),
            "goal" => Some(Self::Goal),
            "activity" => Some(Self::Activity),
            "intent" => Some(Self::Intent),
            "turn_card" => Some(Self::TurnCard),
            "work" => Some(Self::Work),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct IndexedEntry {
    pub entry_id: String,
    pub session_id: String,
    pub space_id: String,
    pub parent_id: Option<String>,
    pub ts: String,
    pub kind: EntryKind,
    pub role: Option<String>,
    pub provider: Option<String>,
    pub model: Option<String>,
    pub tool_name: Option<String>,
    pub content_text: String,
    pub is_error: bool,
}

#[derive(Debug, thiserror::Error)]
pub enum StoreError {
    #[error("database error: {0}")]
    Db(#[from] rusqlite::Error),
    #[error("io error: {0}")]
    Io(#[from] std::io::Error),
    #[error("json error: {0}")]
    Json(#[from] serde_json::Error),
    #[error("schema mismatch: expected {expected}, found {found}")]
    SchemaMismatch { expected: u32, found: u32 },
}

// ---------------------------------------------------------------------------
// Store handle (thread-safe, cheap to clone)
// ---------------------------------------------------------------------------

#[derive(Clone)]
pub struct Store {
    inner: Arc<StoreInner>,
}

struct StoreInner {
    db_path: PathBuf,
    conn: Mutex<Connection>,
}

impl Store {
    /// Open or create the FTS5 index at `<sessions_home>/store.db`.
    /// WAL mode is enabled automatically. Idempotent.
    pub fn open(sessions_home: &Path) -> Result<Self, StoreError> {
        std::fs::create_dir_all(sessions_home)?;
        let db_path = sessions_home.join(DB_NAME);
        let conn = Connection::open(&db_path)?;
        conn.busy_timeout(std::time::Duration::from_millis(100))?;
        conn.execute_batch("PRAGMA journal_mode=WAL; PRAGMA synchronous=NORMAL;")?;
        Self::ensure_schema(&conn)?;
        Ok(Store {
            inner: Arc::new(StoreInner {
                db_path,
                conn: Mutex::new(conn),
            }),
        })
    }

    /// Path to the database file.
    pub fn db_path(&self) -> &Path {
        &self.inner.db_path
    }

    /// Rebuild the entire index from JSONL files under `sessions_home`.
    /// This drops all existing data and re-imports — safe because the
    /// index is fully derivable from JSONL.
    pub fn rebuild(&self, sessions_home: &Path) -> Result<RebuildStats, StoreError> {
        let conn = self.inner.conn.lock().unwrap_or_else(|p| p.into_inner());
        conn.execute_batch(
            "DELETE FROM entries; DELETE FROM entries_fts; DELETE FROM ledger_cursors; DELETE FROM entry_locators; DELETE FROM entry_lineage; DELETE FROM entry_jumps; DELETE FROM turn_descriptors;",
        )?;
        let stats = self.import_all(&conn, sessions_home)?;
        conn.execute(
            "INSERT OR REPLACE INTO meta(key, value) VALUES ('version', ?1)",
            [SCHEMA_VERSION.to_string()],
        )?;
        Ok(stats)
    }

    /// Import a single JSONL file into the index. Idempotent (skips
    /// already-indexed entry IDs).
    pub fn import_session(
        &self,
        sessions_home: &Path,
        jsonl_path: &Path,
    ) -> Result<ImportStats, StoreError> {
        let conn = self.inner.conn.lock().unwrap_or_else(|p| p.into_inner());
        self.import_file(&conn, sessions_home, jsonl_path)
    }

    /// Request-path refresh: never cold-rebuild or decode an unbounded append.
    pub fn import_session_bounded(
        &self,
        sessions_home: &Path,
        path: &Path,
        max_bytes: u64,
    ) -> Result<ImportStats, StoreError> {
        let conn = self.conn();
        self.import_file_bounded(&conn, sessions_home, path, Some(max_bytes))
    }

    /// One background replay transaction. A single complete record may exceed
    /// the target chunk size; individual records still have the global read cap.
    pub fn import_session_chunk(
        &self,
        sessions_home: &Path,
        path: &Path,
        target_bytes: u64,
    ) -> Result<ImportStats, StoreError> {
        let conn = self.conn();
        self.import_file_chunk(&conn, sessions_home, path, None, Some(target_bytes.max(1)))
    }

    /// Locate an exact entry within its requested session. No content is
    /// loaded here and an ID from another session cannot resolve.
    pub fn locate_entry(
        &self,
        session_id: &str,
        entry_id: &str,
    ) -> Result<Option<EntryLocator>, StoreError> {
        use rusqlite::OptionalExtension;
        let conn = self.conn();
        Ok(conn
            .query_row(
                "SELECT entry_id, session_id, path, sequence, offset, length, parent_id, digest
             FROM entry_locators WHERE session_id = ?1 AND entry_id = ?2",
                rusqlite::params![session_id, entry_id],
                |row| {
                    Ok(EntryLocator {
                        entry_id: row.get(0)?,
                        session_id: row.get(1)?,
                        path: PathBuf::from(row.get::<_, String>(2)?),
                        sequence: row.get(3)?,
                        offset: row.get(4)?,
                        length: row.get(5)?,
                        parent_id: row.get(6)?,
                        digest: row.get(7)?,
                    })
                },
            )
            .optional()?)
    }

    pub fn locate_sequence(
        &self,
        session_id: &str,
        sequence: u64,
    ) -> Result<Option<EntryLocator>, StoreError> {
        use rusqlite::OptionalExtension;
        let conn = self.conn();
        let id: Option<String> = conn
            .query_row(
                "SELECT entry_id FROM entry_locators WHERE session_id = ?1 AND sequence = ?2",
                rusqlite::params![session_id, sequence],
                |row| row.get(0),
            )
            .optional()?;
        drop(conn);
        match id {
            Some(id) => self.locate_entry(session_id, &id),
            None => Ok(None),
        }
    }

    /// Append a single entry (real-time update path). Called from
    /// `SessionLog::append` when the store is wired in.
    pub fn append_entry(
        &self,
        session_id: &str,
        entry: &vak_session::Entry,
    ) -> Result<(), StoreError> {
        let conn = self.inner.conn.lock().unwrap_or_else(|p| p.into_inner());
        if let Some(meta) = self.extract_meta(session_id, entry) {
            Self::insert_meta(&conn, &meta)?;
        }
        Ok(())
    }

    fn conn(&self) -> std::sync::MutexGuard<'_, Connection> {
        self.inner.conn.lock().unwrap_or_else(|p| p.into_inner())
    }
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct RebuildStats {
    pub files_scanned: usize,
    pub entries_indexed: usize,
    pub fts_rows: usize,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct ImportStats {
    pub entries_indexed: usize,
    pub fts_rows: usize,
    pub skipped: usize,
    /// Canonical ledger bytes read, including bounded cursor validation.
    pub bytes_read: u64,
    /// Completed canonical extent committed in the same transaction as rows.
    pub committed_offset: u64,
    pub observed_length: u64,
    /// False for an unchanged prefix or an incomplete final record. Background
    /// replay must not spin waiting for an unfinished append.
    pub made_progress: bool,
}

// ---------------------------------------------------------------------------
// Schema
// ---------------------------------------------------------------------------

impl Store {
    fn ensure_schema(conn: &Connection) -> Result<(), StoreError> {
        let stamped = conn
            .query_row("SELECT value FROM meta WHERE key = 'schema'", [], |row| {
                row.get::<_, String>(0)
            })
            .ok();
        if stamped.as_deref() != Some(SCHEMA_VERSION.to_string().as_str()) {
            Self::drop_all(conn)?;
        } else if conn
            // A warm reader must not contend for a write transaction on
            // every open.
            .query_row(
                "SELECT value FROM meta WHERE key = 'locator_version'",
                [],
                |row| row.get::<_, String>(0),
            )
            .is_ok_and(|version| version == "2")
        {
            return Ok(());
        }
        conn.execute_batch(
            "CREATE TABLE IF NOT EXISTS meta (
                key   TEXT PRIMARY KEY,
                value TEXT NOT NULL
            );

            CREATE TABLE IF NOT EXISTS entry_locators (
                entry_id TEXT PRIMARY KEY,
                session_id TEXT NOT NULL,
                path TEXT NOT NULL,
                sequence INTEGER NOT NULL,
                offset INTEGER NOT NULL,
                length INTEGER NOT NULL,
                parent_id TEXT,
                digest TEXT NOT NULL
            );
            CREATE INDEX IF NOT EXISTS idx_locators_session_sequence
                ON entry_locators(session_id, sequence);
            CREATE INDEX IF NOT EXISTS idx_locators_path_sequence
                ON entry_locators(path, sequence);

            CREATE TABLE IF NOT EXISTS entry_lineage (
                entry_id TEXT PRIMARY KEY,
                session_id TEXT NOT NULL,
                depth INTEGER NOT NULL,
                reset_id TEXT
            );
            CREATE TABLE IF NOT EXISTS entry_jumps (
                entry_id TEXT NOT NULL,
                level INTEGER NOT NULL,
                ancestor_id TEXT NOT NULL,
                PRIMARY KEY(entry_id, level)
            );
            CREATE TABLE IF NOT EXISTS turn_descriptors (
                entry_id TEXT PRIMARY KEY,
                session_id TEXT NOT NULL,
                turn_id TEXT NOT NULL,
                record TEXT NOT NULL
            );
            CREATE INDEX IF NOT EXISTS idx_turn_descriptors_scope
                ON turn_descriptors(session_id, turn_id);

            CREATE TABLE IF NOT EXISTS ledger_cursors (
                path TEXT PRIMARY KEY,
                session_id TEXT NOT NULL,
                offset INTEGER NOT NULL,
                anchor TEXT NOT NULL
            );

            CREATE TABLE IF NOT EXISTS entries (
                entry_id      TEXT PRIMARY KEY,
                session_id    TEXT NOT NULL,
                space_id  TEXT NOT NULL,
                parent_id     TEXT,
                ts            TEXT NOT NULL,
                kind          TEXT NOT NULL,
                role          TEXT,
                provider      TEXT,
                model         TEXT,
                tool_name     TEXT,
                content_text  TEXT NOT NULL DEFAULT '',
                is_error      INTEGER NOT NULL DEFAULT 0
            );

            CREATE INDEX IF NOT EXISTS idx_entries_session
                ON entries(session_id);
            CREATE INDEX IF NOT EXISTS idx_entries_project
                ON entries(space_id);
            CREATE INDEX IF NOT EXISTS idx_entries_kind
                ON entries(kind);
            CREATE INDEX IF NOT EXISTS idx_entries_ts
                ON entries(ts);
            CREATE INDEX IF NOT EXISTS idx_entries_role
                ON entries(role);

            CREATE VIRTUAL TABLE IF NOT EXISTS entries_fts USING fts5(
                entry_id UNINDEXED,
                session_id UNINDEXED,
                space_id UNINDEXED,
                ts UNINDEXED,
                kind UNINDEXED,
                role UNINDEXED,
                content,
                tokenize='porter unicode61'
            );",
        )?;
        // Existing cache generations predate locators. Invalidate only their
        // ingest watermark so a background replay can populate every locator.
        conn.execute_batch(
            "BEGIN IMMEDIATE;
            DELETE FROM ledger_cursors WHERE NOT EXISTS
                (SELECT 1 FROM meta WHERE key = 'locator_version' AND value = '2');
            DELETE FROM entry_jumps WHERE NOT EXISTS
                (SELECT 1 FROM meta WHERE key = 'locator_version' AND value = '2');
            DELETE FROM entry_lineage WHERE NOT EXISTS
                (SELECT 1 FROM meta WHERE key = 'locator_version' AND value = '2');
            DELETE FROM turn_descriptors WHERE NOT EXISTS
                (SELECT 1 FROM meta WHERE key = 'locator_version' AND value = '2');
            DELETE FROM entry_locators WHERE NOT EXISTS
                (SELECT 1 FROM meta WHERE key = 'locator_version' AND value = '2');
            INSERT OR REPLACE INTO meta(key, value) VALUES ('locator_version', '2');
            COMMIT;",
        )?;
        conn.execute(
            "INSERT OR REPLACE INTO meta(key, value) VALUES ('schema', ?1)",
            [SCHEMA_VERSION.to_string()],
        )?;
        Ok(())
    }
}

// ---------------------------------------------------------------------------
// Extraction logic: JSONL Entry → IndexedEntry
// ---------------------------------------------------------------------------

impl Store {
    /// Extract search metadata from a single JSONL entry.
    /// Drop every table of an index written under another schema. The index
    /// is derived from the ledgers, so it is rebuilt rather than adapted.
    fn drop_all(conn: &Connection) -> Result<(), StoreError> {
        let tables: Vec<String> = {
            let mut statement = conn.prepare(
                "SELECT name FROM sqlite_master WHERE type = 'table' AND name NOT LIKE 'sqlite_%'",
            )?;
            statement
                .query_map([], |row| row.get::<_, String>(0))?
                .collect::<Result<Vec<_>, _>>()?
        };
        for table in tables {
            if conn
                .query_row(
                    "SELECT 1 FROM sqlite_master WHERE type = 'table' AND name = ?1",
                    [&table],
                    |_| Ok(()),
                )
                .is_ok()
            {
                conn.execute_batch(&format!(
                    "DROP TABLE IF EXISTS \"{}\";",
                    table.replace('"', "\"\"")
                ))?;
            }
        }
        Ok(())
    }

    fn extract_meta(&self, session_id: &str, entry: &vak_session::Entry) -> Option<IndexedEntry> {
        use vak_session::EntryPayload;

        let kind = match &entry.payload {
            EntryPayload::Header(_) => EntryKind::Header,
            EntryPayload::Message(_) => EntryKind::Message,
            EntryPayload::Compaction(_) => EntryKind::Compaction,
            EntryPayload::Receipt(_) => EntryKind::Receipt,
            EntryPayload::Goal(_) => EntryKind::Goal,
            EntryPayload::GoalUpdate(_) => EntryKind::Goal,
            EntryPayload::Activity(_) => EntryKind::Activity,
            EntryPayload::Work(_) => EntryKind::Work,
            EntryPayload::Intent(_) => EntryKind::Intent,
            EntryPayload::TurnCard(_) => EntryKind::TurnCard,
            // A Presentation entry is display-channel/model-history
            // data (docs/design/68-context-engine.md §10), not free text to
            // full-text index today. TurnCards project compact subjects
            // and outcomes into this shared index; their canonical records
            // remain in the ledger.
            // An evidence body repeats a tool result the Message entry
            // already indexes the window of; `recall` reaches the rest.
            EntryPayload::ContextSelection(_)
            | EntryPayload::TurnCapabilitiesBound(_)
            | EntryPayload::TurnCapabilitiesRef(_)
            | EntryPayload::ChildRun { .. }
            | EntryPayload::Presentation(_)
            | EntryPayload::EvidenceBody(_)
            | EntryPayload::CallEffect(_) => return None,
        };

        match &entry.payload {
            EntryPayload::Message(record) => {
                // A runtime-authored nudge is indexed under its own role so
                // it is never returned as something the user said.
                let role = match record.message.role {
                    _ if record.control_kind().is_some() => "control",
                    vak_llm::Role::User => "user",
                    vak_llm::Role::Assistant => "assistant",
                };
                let (text, tool_name, is_error) = extract_message_text(&record.as_typed().content);
                let model = record.meta.as_ref().and_then(|m| m.model.clone());
                Some(IndexedEntry {
                    entry_id: entry.id.clone(),
                    session_id: session_id.to_string(),
                    space_id: String::new(),
                    parent_id: entry.parent_id.clone(),
                    ts: entry.ts.to_rfc3339(),
                    kind,
                    role: Some(role.to_string()),
                    provider: None,
                    model,
                    tool_name,
                    content_text: text,
                    is_error,
                })
            }
            EntryPayload::Header(header) => Some(IndexedEntry {
                entry_id: entry.id.clone(),
                session_id: session_id.to_string(),
                space_id: String::new(),
                parent_id: entry.parent_id.clone(),
                ts: entry.ts.to_rfc3339(),
                kind,
                role: None,
                provider: Some(header.contract.provider.clone()),
                model: Some(header.contract.model.clone()),
                tool_name: None,
                content_text: String::new(),
                is_error: false,
            }),
            EntryPayload::Compaction(c) => Some(IndexedEntry {
                entry_id: entry.id.clone(),
                session_id: session_id.to_string(),
                space_id: String::new(),
                parent_id: entry.parent_id.clone(),
                ts: entry.ts.to_rfc3339(),
                kind,
                role: None,
                provider: None,
                model: None,
                tool_name: None,
                content_text: c.summary.clone(),
                is_error: false,
            }),
            EntryPayload::Receipt(r) => Some(IndexedEntry {
                entry_id: entry.id.clone(),
                session_id: session_id.to_string(),
                space_id: String::new(),
                parent_id: entry.parent_id.clone(),
                ts: entry.ts.to_rfc3339(),
                kind,
                role: None,
                provider: Some(r.provider.clone()),
                model: Some(r.model.clone()),
                tool_name: None,
                content_text: String::new(),
                is_error: false,
            }),
            EntryPayload::Goal(g) => Some(IndexedEntry {
                entry_id: entry.id.clone(),
                session_id: session_id.to_string(),
                space_id: String::new(),
                parent_id: entry.parent_id.clone(),
                ts: entry.ts.to_rfc3339(),
                kind,
                role: None,
                provider: None,
                model: None,
                tool_name: None,
                content_text: format!("{} {}", g.objective, g.criteria.join(" ")),
                is_error: false,
            }),
            EntryPayload::GoalUpdate(update) => Some(IndexedEntry {
                entry_id: entry.id.clone(),
                session_id: session_id.to_string(),
                space_id: String::new(),
                parent_id: entry.parent_id.clone(),
                ts: entry.ts.to_rfc3339(),
                kind,
                role: Some("system".into()),
                provider: None,
                model: None,
                tool_name: None,
                content_text: format!("{:?} {}", update.relation, update.request),
                is_error: false,
            }),
            EntryPayload::Activity(activity) => Some(IndexedEntry {
                entry_id: entry.id.clone(),
                session_id: session_id.to_string(),
                space_id: String::new(),
                parent_id: entry.parent_id.clone(),
                ts: entry.ts.to_rfc3339(),
                kind,
                role: Some("system".into()),
                provider: None,
                model: None,
                tool_name: activity.data.get("tool").cloned(),
                content_text: format!(
                    "{} {}",
                    activity.label,
                    activity.detail.as_deref().unwrap_or_default()
                ),
                is_error: matches!(
                    activity.status,
                    vak_session::ActivityStatus::Failed | vak_session::ActivityStatus::Denied
                ),
            }),
            EntryPayload::Work(work) => Some(IndexedEntry {
                entry_id: entry.id.clone(),
                session_id: session_id.to_string(),
                space_id: String::new(),
                parent_id: entry.parent_id.clone(),
                ts: entry.ts.to_rfc3339(),
                kind,
                role: Some("system".into()),
                provider: None,
                model: None,
                tool_name: None,
                content_text: serde_json::to_string(&work.kind).unwrap_or_default(),
                is_error: false,
            }),
            // Indexed on the reading rather than the note, so "every
            // irreversible thing this agent did in March" is a search rather
            // than a ledger crawl. That query is the whole point of recording
            // the axes next to the decision.
            EntryPayload::TurnCard(record) => Some(IndexedEntry {
                entry_id: entry.id.clone(),
                session_id: session_id.to_string(),
                space_id: String::new(),
                parent_id: entry.parent_id.clone(),
                ts: entry.ts.to_rfc3339(),
                kind,
                role: Some("turn".into()),
                provider: None,
                model: None,
                tool_name: None,
                // Search the subject/outcome, not megabytes of call arguments.
                content_text: format!(
                    "{} {} {}",
                    record.card.asked,
                    record.card.answered.narration,
                    record
                        .card
                        .answered
                        .presentations
                        .iter()
                        .map(|p| p.title.as_str())
                        .collect::<Vec<_>>()
                        .join(" ")
                ),
                is_error: false,
            }),
            EntryPayload::Intent(record) => Some(IndexedEntry {
                entry_id: entry.id.clone(),
                session_id: session_id.to_string(),
                space_id: String::new(),
                parent_id: entry.parent_id.clone(),
                ts: entry.ts.to_rfc3339(),
                kind,
                role: Some("system".into()),
                provider: None,
                model: record.provenance.model.clone(),
                tool_name: None,
                content_text: format!(
                    "{} {} {} {} {} {} {}",
                    record.reading.act.as_str(),
                    record.reading.horizon.as_str(),
                    record.reading.stakes.as_str(),
                    record.reading.evidence.as_str(),
                    record.reading.attendance.as_str(),
                    record.provenance.tier.as_str(),
                    record.model_visible.as_deref().unwrap_or_default()
                ),
                is_error: false,
            }),
            EntryPayload::ContextSelection(_)
            | EntryPayload::TurnCapabilitiesBound(_)
            | EntryPayload::TurnCapabilitiesRef(_)
            | EntryPayload::ChildRun { .. }
            | EntryPayload::Presentation(_)
            | EntryPayload::EvidenceBody(_)
            | EntryPayload::CallEffect(_) => None,
        }
    }

    fn insert_meta(conn: &Connection, meta: &IndexedEntry) -> Result<(), StoreError> {
        let inserted = conn.execute(
            "INSERT OR IGNORE INTO entries
             (entry_id, session_id, space_id, parent_id, ts, kind, role,
              provider, model, tool_name, content_text, is_error)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12)",
            rusqlite::params![
                meta.entry_id,
                meta.session_id,
                meta.space_id,
                meta.parent_id,
                meta.ts,
                meta.kind.as_str(),
                meta.role,
                meta.provider,
                meta.model,
                meta.tool_name,
                meta.content_text,
                meta.is_error as i32,
            ],
        )?;
        // Entry and FTS identities are the same idempotent boundary.
        if inserted == 0 {
            return Ok(());
        }
        // FTS row — only if there is searchable content.
        if !meta.content_text.trim().is_empty() {
            conn.execute(
                "INSERT INTO entries_fts(entry_id, session_id, space_id, ts, kind, role, content)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
                rusqlite::params![
                    meta.entry_id,
                    meta.session_id,
                    meta.space_id,
                    meta.ts,
                    meta.kind.as_str(),
                    meta.role.as_deref().unwrap_or(""),
                    meta.content_text,
                ],
            )?;
        }
        Ok(())
    }
}

/// Extract searchable text and metadata from message content blocks.
/// Returns (text, tool_name, is_error).
fn extract_message_text(
    content: &[vak_llm::types::ContentBlock],
) -> (String, Option<String>, bool) {
    use vak_llm::types::ContentBlock;

    let mut text_parts = Vec::new();
    let mut tool_name: Option<String> = None;
    let mut is_error = false;

    for block in content {
        match block {
            ContentBlock::Text { text } => {
                text_parts.push(text.clone());
            }
            ContentBlock::Thinking { text, .. } => {
                text_parts.push(format!("[thinking] {text}"));
            }
            ContentBlock::ToolUse { name, input, .. } => {
                tool_name = Some(name.clone());
                if let Some(cmd) = input.get("command").and_then(|v| v.as_str()) {
                    text_parts.push(format!("[tool:{name}] {cmd}"));
                } else {
                    text_parts.push(format!("[tool:{name}]"));
                }
            }
            ContentBlock::ToolResult {
                content,
                is_error: err,
                ..
            } => {
                is_error = *err;
                if !content.trim().is_empty() {
                    text_parts.push(format!("[result] {content}"));
                }
            }
            ContentBlock::Image { .. } => {}
            ContentBlock::Provider { .. } => {}
        }
    }
    (text_parts.join("\n"), tool_name, is_error)
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]

    use super::*;
    use std::io::Write;
    use vak_llm::types::{ContentBlock, Message};
    use vak_session::log::SessionPath;
    use vak_session::types::{Entry, EntryPayload, FrozenContract, MessageRecord, SessionHeader};

    fn test_header(id: &str) -> SessionHeader {
        SessionHeader {
            space: None,
            run: None,
            cause: None,
            agent: None,
            session_id: id.to_string(),
            created_at: chrono::Utc::now(),
            cwd: std::env::current_dir().unwrap(),
            parent_session_id: None,
            contract_id: None,
            work_item_id: None,
            conversation: None,
            contract: FrozenContract {
                app_version: "test".into(),
                provider: "openai".into(),
                model: "gpt-4o".into(),
                route_ladder: vec![],
                route_objective: String::new(),
                route_annotations: vec![],
                system_prompt: String::new(),
                permission_mode: "workspace-write".into(),
                capabilities: Vec::new(),
                prompt_layers: Vec::new(),
            },
        }
    }

    fn user_msg(text: &str) -> MessageRecord {
        MessageRecord {
            message: Message {
                role: vak_llm::Role::User,
                content: vec![ContentBlock::text(text)],
            },
            meta: None,
        }
    }

    fn assistant_msg(text: &str) -> MessageRecord {
        MessageRecord {
            message: Message {
                role: vak_llm::Role::Assistant,
                content: vec![ContentBlock::text(text)],
            },
            meta: None,
        }
    }

    /// One record frame holding `entry`, for appending raw (torn-write tests).
    fn frame_of(entry: &[u8]) -> Vec<u8> {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("frame.log");
        let mut writer = vak_storage::records::RecordWriter::open(&path).unwrap();
        writer.append(entry, None).unwrap();
        std::fs::read(path).unwrap()
    }

    fn write_session(home: &Path, cwd: &Path, id: &str, msgs: &[MessageRecord]) {
        let path = SessionPath::new_session_file(home, cwd, id);
        // A fixture replaces any ledger already at this path.
        let _ = std::fs::remove_dir_all(&path);
        let mut w = vak_storage::segments::SegmentSet::open(&path)
            .unwrap()
            .writer(1)
            .unwrap();
        let header = Entry::new(None, EntryPayload::Header(test_header(id)));
        w.append(&serde_json::to_vec(&header).unwrap(), None)
            .unwrap();
        let mut parent = Some(header.id.clone());
        for m in msgs {
            let entry = Entry {
                at_turn: None,
                id: uuid::Uuid::now_v7().to_string(),
                parent_id: parent.clone(),
                ts: chrono::Utc::now(),
                payload: EntryPayload::Message(m.clone()),
            };
            parent = Some(entry.id.clone());
            w.append(&serde_json::to_vec(&entry).unwrap(), None)
                .unwrap();
        }
    }

    #[test]
    fn open_creates_db_and_schema() {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::open(dir.path()).unwrap();
        assert!(store.db_path().exists());
        // Second open is idempotent.
        let store2 = Store::open(dir.path()).unwrap();
        assert_eq!(store.db_path(), store2.db_path());
    }

    #[test]
    fn rebuild_indexes_entries_and_fts() {
        let dir = tempfile::tempdir().unwrap();
        let home = dir.path();
        let cwd = dir.path();
        write_session(
            home,
            cwd,
            "sess-001",
            &[
                user_msg("the deploy pipeline handles rollbacks gracefully"),
                assistant_msg("confirmed — rollback windows pause before deploy"),
            ],
        );
        write_session(
            home,
            cwd,
            "sess-002",
            &[user_msg("pizza toppings are irrelevant to this query")],
        );

        let store = Store::open(home).unwrap();
        let stats = store.rebuild(home).unwrap();
        assert_eq!(stats.files_scanned, 2);
        // sess-001: 1 header + 2 msgs, sess-002: 1 header + 1 msg = 5 entries
        assert_eq!(stats.entries_indexed, 5);
        // 3 messages with text → 3 FTS rows
        assert_eq!(stats.fts_rows, 3);
    }

    #[test]
    fn import_skips_already_indexed() {
        let dir = tempfile::tempdir().unwrap();
        let home = dir.path();
        let cwd = dir.path();
        write_session(
            home,
            cwd,
            "sess-aaa",
            &[user_msg("unique search term zanzibar")],
        );

        let store = Store::open(home).unwrap();
        let s1 = store
            .import_session(home, &SessionPath::new_session_file(home, cwd, "sess-aaa"))
            .unwrap();
        assert_eq!(s1.entries_indexed, 2); // header + message
        let s2 = store
            .import_session(home, &SessionPath::new_session_file(home, cwd, "sess-aaa"))
            .unwrap();
        assert_eq!(s2.entries_indexed, 0);
        assert_eq!(s2.skipped, 0, "committed prefix is not parsed again");
    }

    #[test]
    fn cursor_survives_restart_and_reads_only_appended_records() {
        let dir = tempfile::tempdir().unwrap();
        let path = SessionPath::new_session_file(dir.path(), dir.path(), "incremental");
        write_session(
            dir.path(),
            dir.path(),
            "incremental",
            &[user_msg(&"old evidence ".repeat(100_000))],
        );
        let store = Store::open(dir.path()).unwrap();
        store.import_session(dir.path(), &path).unwrap();
        drop(store);
        let store = Store::open(dir.path()).unwrap();
        let warm = store.import_session(dir.path(), &path).unwrap();
        assert_eq!(warm.entries_indexed, 0);
        assert!(warm.bytes_read <= 16384, "{:?}", warm);
        let entry = Entry::new(None, EntryPayload::Message(user_msg("new bounded subject")));
        let serialized = serde_json::to_vec(&entry).unwrap();
        let frame = frame_of(&serialized);
        let mut file = std::fs::OpenOptions::new()
            .append(true)
            .open(path.join("seg-00000001.log"))
            .unwrap();
        file.write_all(&frame[..frame.len() / 2]).unwrap();
        let partial = store.import_session(dir.path(), &path).unwrap();
        assert_eq!(partial.entries_indexed, 0);
        file.write_all(&frame[frame.len() / 2..]).unwrap();
        let appended = store.import_session(dir.path(), &path).unwrap();
        assert_eq!(appended.entries_indexed, 1);
        assert!(appended.bytes_read < 20000, "{:?}", appended);
        assert_eq!(
            store
                .search("bounded", 10, &crate::query::SearchFilter::default())
                .unwrap()
                .entries
                .len(),
            1
        );
    }

    #[test]
    fn locators_include_non_searchable_evidence_and_survive_restart() {
        let dir = tempfile::tempdir().unwrap();
        let path = SessionPath::new_session_file(dir.path(), dir.path(), "opaque");
        write_session(
            dir.path(),
            dir.path(),
            "opaque",
            &[user_msg("searchable subject")],
        );
        let evidence = Entry::new(
            None,
            EntryPayload::EvidenceBody(vak_session::types::EvidenceBodyRecord {
                tool_use_id: "evidence-call".into(),
                body: vak_session::objects::ObjectRef {
                    id: "opaque-evidence-body".into(),
                    len: 20,
                },
            }),
        );
        let mut file = vak_storage::segments::SegmentSet::open(&path)
            .unwrap()
            .writer(1)
            .unwrap();
        file.append(&serde_json::to_vec(&evidence).unwrap(), None)
            .unwrap();
        let store = Store::open(dir.path()).unwrap();
        store.import_session(dir.path(), &path).unwrap();
        let location = store.locate_entry("opaque", &evidence.id).unwrap().unwrap();
        assert_eq!(location.sequence, 2);
        assert!(
            store
                .search("opaque", 10, &crate::query::SearchFilter::default())
                .unwrap()
                .entries
                .is_empty()
        );
        let loaded = vak_session::SessionLog::read_record_at(
            &location.path,
            location.offset,
            location.length,
            &location.entry_id,
            &location.digest,
        )
        .unwrap();
        assert!(
            matches!(loaded.payload, EntryPayload::EvidenceBody(record) if record.body.id == "opaque-evidence-body")
        );
        drop(store);
        let reopened = Store::open(dir.path()).unwrap();
        assert_eq!(
            reopened
                .locate_sequence("opaque", 2)
                .unwrap()
                .unwrap()
                .entry_id,
            evidence.id
        );
    }

    #[test]
    fn repeated_incremental_append_does_not_duplicate_fts_results() {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::open(dir.path()).unwrap();
        let entry = Entry::new(None, EntryPayload::Message(user_msg("idempotent zanzibar")));
        for _ in 0..3 {
            store.append_entry("same-session", &entry).unwrap();
        }
        assert_eq!(
            store
                .search("zanzibar", 10, &crate::query::SearchFilter::default())
                .unwrap()
                .entries
                .len(),
            1
        );
    }

    #[test]
    fn older_cache_watermark_replays_locators_without_duplicate_search_rows() {
        let dir = tempfile::tempdir().unwrap();
        let path = SessionPath::new_session_file(dir.path(), dir.path(), "old-cache");
        write_session(
            dir.path(),
            dir.path(),
            "old-cache",
            &[user_msg("zanzibar legacy cache")],
        );
        let store = Store::open(dir.path()).unwrap();
        store.import_session(dir.path(), &path).unwrap();
        store
            .conn()
            .execute_batch(
                "DELETE FROM entry_locators; DELETE FROM meta WHERE key = 'locator_version';",
            )
            .unwrap();
        drop(store);
        let store = Store::open(dir.path()).unwrap();
        assert!(store.locate_sequence("old-cache", 0).unwrap().is_none());
        let replay = store.import_session(dir.path(), &path).unwrap();
        assert_eq!(replay.skipped, 2);
        assert!(store.locate_sequence("old-cache", 0).unwrap().is_some());
        assert!(store.locate_sequence("old-cache", 1).unwrap().is_some());
        assert_eq!(
            store
                .search("zanzibar", 10, &crate::query::SearchFilter::default())
                .unwrap()
                .entries
                .len(),
            1
        );
    }

    #[test]
    fn background_chunks_commit_restart_and_release_the_writer() {
        let dir = tempfile::tempdir().unwrap();
        let path = SessionPath::new_session_file(dir.path(), dir.path(), "chunks");
        write_session(dir.path(), dir.path(), "chunks", &[user_msg("initial")]);
        let log = vak_session::SessionLog::open(path.clone()).unwrap();
        let mut parent = log.tail_id().cloned();
        drop(log);
        let mut file = vak_storage::segments::SegmentSet::open(&path)
            .unwrap()
            .writer(1)
            .unwrap();
        let mut last_id = String::new();
        for number in 0..1000 {
            let entry = Entry::new(
                parent,
                EntryPayload::Message(user_msg(&format!("indexed subject number {number}"))),
            );
            file.append(&serde_json::to_vec(&entry).unwrap(), None)
                .unwrap();
            parent = Some(entry.id.clone());
            last_id = entry.id;
        }
        let mut previous_offset = 0;
        let mut transactions = 0;
        loop {
            // Reopen every chunk to exercise crash/restart at each watermark.
            let store = Store::open(dir.path()).unwrap();
            let stats = store.import_session_chunk(dir.path(), &path, 4096).unwrap();
            assert!(stats.made_progress);
            assert!(stats.committed_offset > previous_offset);
            assert!(stats.bytes_read <= 4096 + 16384 + 1024, "{stats:?}");
            previous_offset = stats.committed_offset;
            transactions += 1;
            // Another handle sees committed progress between writer chunks.
            let observer = Store::open(dir.path()).unwrap();
            assert!(observer.locate_sequence("chunks", 0).unwrap().is_some());
            if stats.committed_offset == stats.observed_length {
                assert!(observer.locate_entry("chunks", &last_id).unwrap().is_some());
                break;
            }
            assert!(transactions < 200);
        }
        assert!(transactions > 10);
        let store = Store::open(dir.path()).unwrap();
        let stable = store.import_session_chunk(dir.path(), &path, 4096).unwrap();
        assert!(!stable.made_progress);
        let torn = frame_of(b"{\"incomplete\":true}");
        std::fs::OpenOptions::new()
            .append(true)
            .open(path.join("seg-00000001.log"))
            .unwrap()
            .write_all(&torn[..torn.len() / 2])
            .unwrap();
        let partial = store.import_session_chunk(dir.path(), &path, 4096).unwrap();
        assert!(!partial.made_progress);
        assert_eq!(partial.committed_offset, previous_offset);
        assert!(partial.committed_offset < partial.observed_length);
    }

    #[test]
    fn failed_background_chunk_preserves_previously_committed_progress() {
        let dir = tempfile::tempdir().unwrap();
        let path = SessionPath::new_session_file(dir.path(), dir.path(), "chunk-error");
        write_session(
            dir.path(),
            dir.path(),
            "chunk-error",
            &[user_msg("committed subject")],
        );
        let store = Store::open(dir.path()).unwrap();
        let first = store.import_session_chunk(dir.path(), &path, 1).unwrap();
        assert_eq!(first.entries_indexed, 1); // header-only transaction
        let second = store.import_session_chunk(dir.path(), &path, 4096).unwrap();
        assert_eq!(second.entries_indexed, 1);
        let addition = Entry::new(
            None,
            EntryPayload::Message(user_msg("uncommitted zanzibar")),
        );
        let mut file = vak_storage::segments::SegmentSet::open(&path)
            .unwrap()
            .writer(1)
            .unwrap();
        file.append(&serde_json::to_vec(&addition).unwrap(), None)
            .unwrap();
        file.append(b"corrupt complete record", None).unwrap();
        assert!(store.import_session_chunk(dir.path(), &path, 4096).is_err());
        assert!(store.locate_sequence("chunk-error", 0).unwrap().is_some());
        assert!(store.locate_sequence("chunk-error", 1).unwrap().is_some());
        assert!(
            store
                .locate_entry("chunk-error", &addition.id)
                .unwrap()
                .is_none()
        );
        assert!(
            store
                .search("zanzibar", 10, &crate::query::SearchFilter::default())
                .unwrap()
                .entries
                .is_empty()
        );
        drop(store);
        let reopened = Store::open(dir.path()).unwrap();
        assert!(
            reopened
                .import_session_chunk(dir.path(), &path, 4096)
                .is_err()
        );
    }

    /// An index file from another schema (here, the column a session's
    /// space had before it was renamed) is dropped and rebuilt on open,
    /// never queried against a column it does not have.
    #[test]
    fn an_index_from_another_schema_is_rebuilt_not_misread() {
        let dir = tempfile::tempdir().unwrap();
        {
            let conn = Connection::open(dir.path().join(DB_NAME)).unwrap();
            conn.execute_batch(
                "CREATE TABLE meta (key TEXT PRIMARY KEY, value TEXT NOT NULL);
                 INSERT INTO meta VALUES ('locator_version', '2');
                 CREATE TABLE entries (entry_id TEXT PRIMARY KEY, session_id TEXT, project_hash TEXT, ts TEXT);",
            )
            .unwrap();
        }
        let store = Store::open(dir.path()).unwrap();
        let conn = store.conn();
        let columns: Vec<String> = conn
            .prepare("SELECT name FROM pragma_table_info('entries')")
            .unwrap()
            .query_map([], |row| row.get(0))
            .unwrap()
            .collect::<Result<_, _>>()
            .unwrap();
        assert!(columns.iter().any(|c| c == "space_id"), "{columns:?}");
        assert!(!columns.iter().any(|c| c == "project_hash"));
    }

    #[test]
    fn request_path_import_never_cold_rebuilds_or_accepts_large_appends() {
        let dir = tempfile::tempdir().unwrap();
        let path = SessionPath::new_session_file(dir.path(), dir.path(), "bounded");
        write_session(dir.path(), dir.path(), "bounded", &[user_msg("initial")]);
        let store = Store::open(dir.path()).unwrap();
        assert!(
            store
                .import_session_bounded(dir.path(), &path, 1024)
                .is_err()
        );
        assert!(store.locate_sequence("bounded", 0).unwrap().is_none());
        store.import_session(dir.path(), &path).unwrap();
        let entry = Entry::new(
            None,
            EntryPayload::Message(user_msg(
                &(0..2000)
                    .map(|_| uuid::Uuid::now_v7().to_string())
                    .collect::<Vec<_>>()
                    .join(" "),
            )),
        );
        let mut file = vak_storage::segments::SegmentSet::open(&path)
            .unwrap()
            .writer(1)
            .unwrap();
        file.append(&serde_json::to_vec(&entry).unwrap(), None)
            .unwrap();
        assert!(
            store
                .import_session_bounded(dir.path(), &path, 1024)
                .is_err()
        );
        assert!(store.locate_entry("bounded", &entry.id).unwrap().is_none());
        store.import_session(dir.path(), &path).unwrap();
        assert!(store.locate_entry("bounded", &entry.id).unwrap().is_some());
    }

    #[test]
    fn failed_import_rolls_back_rows_and_watermark() {
        let dir = tempfile::tempdir().unwrap();
        let path = SessionPath::new_session_file(dir.path(), dir.path(), "rollback");
        write_session(
            dir.path(),
            dir.path(),
            "rollback",
            &[user_msg("committed original")],
        );
        let store = Store::open(dir.path()).unwrap();
        store.import_session(dir.path(), &path).unwrap();
        let entry = Entry::new(
            None,
            EntryPayload::Message(user_msg("uncommitted addition")),
        );
        let mut file = vak_storage::segments::SegmentSet::open(&path)
            .unwrap()
            .writer(1)
            .unwrap();
        file.append(&serde_json::to_vec(&entry).unwrap(), None)
            .unwrap();
        file.append(b"corrupt completed record", None).unwrap();
        assert!(store.import_session(dir.path(), &path).is_err());
        assert!(
            store
                .search("uncommitted", 10, &crate::query::SearchFilter::default())
                .unwrap()
                .entries
                .is_empty()
        );
        assert!(
            store.import_session(dir.path(), &path).is_err(),
            "failed progress must not be committed"
        );
    }

    #[test]
    fn replaced_ledger_invalidates_prior_search_rows() {
        let dir = tempfile::tempdir().unwrap();
        let path = SessionPath::new_session_file(dir.path(), dir.path(), "replace");
        write_session(
            dir.path(),
            dir.path(),
            "replace",
            &[user_msg("zanzibar discarded")],
        );
        let store = Store::open(dir.path()).unwrap();
        store.import_session(dir.path(), &path).unwrap();
        write_session(
            dir.path(),
            dir.path(),
            "replace",
            &[user_msg("replacement subject")],
        );
        store.import_session(dir.path(), &path).unwrap();
        assert!(
            store
                .search("zanzibar", 10, &crate::query::SearchFilter::default())
                .unwrap()
                .entries
                .is_empty()
        );
        assert_eq!(
            store
                .search("replacement", 10, &crate::query::SearchFilter::default())
                .unwrap()
                .entries
                .len(),
            1
        );
    }

    #[test]
    fn extract_message_text_captures_tools() {
        use vak_llm::types::ContentBlock;

        let blocks = vec![
            ContentBlock::text("look at this"),
            ContentBlock::ToolUse {
                id: "tc-1".into(),
                name: "bash".into(),
                input: serde_json::json!({"command": "cargo test"}),
            },
            ContentBlock::ToolResult {
                tool_use_id: "tc-1".into(),
                content: "2 passed".into(),
                is_error: false,
            },
        ];
        let (text, tool, is_err) = extract_message_text(&blocks);
        assert!(text.contains("look at this"));
        assert!(text.contains("[tool:bash] cargo test"));
        assert!(text.contains("[result] 2 passed"));
        assert_eq!(tool.as_deref(), Some("bash"));
        assert!(!is_err);
    }
}
