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

pub mod index;
pub mod query;

/// The database file name within the sessions home directory.
pub const DB_NAME: &str = "store.db";

/// Version tag stored in the `meta` table; bump when the schema changes.
const SCHEMA_VERSION: u32 = 1;

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
            "work" => Some(Self::Work),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct IndexedEntry {
    pub entry_id: String,
    pub session_id: String,
    pub project_hash: String,
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
        conn.execute_batch("DELETE FROM entries; DELETE FROM entries_fts;")?;
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
}

// ---------------------------------------------------------------------------
// Schema
// ---------------------------------------------------------------------------

impl Store {
    fn ensure_schema(conn: &Connection) -> Result<(), StoreError> {
        conn.execute_batch(
            "CREATE TABLE IF NOT EXISTS meta (
                key   TEXT PRIMARY KEY,
                value TEXT NOT NULL
            );

            CREATE TABLE IF NOT EXISTS entries (
                entry_id      TEXT PRIMARY KEY,
                session_id    TEXT NOT NULL,
                project_hash  TEXT NOT NULL,
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
                ON entries(project_hash);
            CREATE INDEX IF NOT EXISTS idx_entries_kind
                ON entries(kind);
            CREATE INDEX IF NOT EXISTS idx_entries_ts
                ON entries(ts);
            CREATE INDEX IF NOT EXISTS idx_entries_role
                ON entries(role);

            CREATE VIRTUAL TABLE IF NOT EXISTS entries_fts USING fts5(
                entry_id UNINDEXED,
                session_id UNINDEXED,
                project_hash UNINDEXED,
                ts UNINDEXED,
                kind UNINDEXED,
                role UNINDEXED,
                content,
                tokenize='porter unicode61'
            );",
        )?;
        Ok(())
    }
}

// ---------------------------------------------------------------------------
// Extraction logic: JSONL Entry → IndexedEntry
// ---------------------------------------------------------------------------

impl Store {
    /// Extract search metadata from a single JSONL entry.
    fn extract_meta(&self, session_id: &str, entry: &vak_session::Entry) -> Option<IndexedEntry> {
        use vak_session::EntryPayload;

        let kind = match &entry.payload {
            EntryPayload::Header(_) => EntryKind::Header,
            EntryPayload::Message(_) => EntryKind::Message,
            EntryPayload::Compaction(_) => EntryKind::Compaction,
            EntryPayload::Receipt(_) => EntryKind::Receipt,
            EntryPayload::Goal(_) => EntryKind::Goal,
            EntryPayload::Activity(_) => EntryKind::Activity,
            EntryPayload::Work(_) => EntryKind::Work,
        };

        match &entry.payload {
            EntryPayload::Message(record) => {
                let role = match record.message.role {
                    vak_llm::Role::User => "user",
                    vak_llm::Role::Assistant => "assistant",
                };
                let (text, tool_name, is_error) = extract_message_text(&record.message.content);
                let model = record.meta.as_ref().and_then(|m| m.model.clone());
                Some(IndexedEntry {
                    entry_id: entry.id.clone(),
                    session_id: session_id.to_string(),
                    project_hash: String::new(),
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
                project_hash: String::new(),
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
                project_hash: String::new(),
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
                project_hash: String::new(),
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
                project_hash: String::new(),
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
            EntryPayload::Activity(activity) => Some(IndexedEntry {
                entry_id: entry.id.clone(),
                session_id: session_id.to_string(),
                project_hash: String::new(),
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
                project_hash: String::new(),
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
        }
    }

    fn insert_meta(conn: &Connection, meta: &IndexedEntry) -> Result<(), StoreError> {
        conn.execute(
            "INSERT OR IGNORE INTO entries
             (entry_id, session_id, project_hash, parent_id, ts, kind, role,
              provider, model, tool_name, content_text, is_error)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12)",
            rusqlite::params![
                meta.entry_id,
                meta.session_id,
                meta.project_hash,
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
        // FTS row — only if there is searchable content.
        if !meta.content_text.trim().is_empty() {
            conn.execute(
                "INSERT INTO entries_fts(entry_id, session_id, project_hash, ts, kind, role, content)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
                rusqlite::params![
                    meta.entry_id,
                    meta.session_id,
                    meta.project_hash,
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
            session_id: id.to_string(),
            created_at: chrono::Utc::now(),
            cwd: std::env::current_dir().unwrap(),
            parent_session_id: None,
            contract_id: None,
            work_item_id: None,
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

    fn write_session(home: &Path, cwd: &Path, id: &str, msgs: &[MessageRecord]) {
        let path = SessionPath::new_session_file(home, cwd, id);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        let file = std::fs::File::create(&path).unwrap();
        let mut w = std::io::BufWriter::new(file);
        let header = Entry::new(None, EntryPayload::Header(test_header(id)));
        serde_json::to_writer(&mut w, &header).unwrap();
        w.write_all(b"\n").unwrap();
        let mut parent = Some(header.id.clone());
        for m in msgs {
            let entry = Entry {
                id: uuid::Uuid::now_v7().to_string(),
                parent_id: parent.clone(),
                ts: chrono::Utc::now(),
                payload: EntryPayload::Message(m.clone()),
            };
            parent = Some(entry.id.clone());
            serde_json::to_writer(&mut w, &entry).unwrap();
            w.write_all(b"\n").unwrap();
        }
        w.flush().unwrap();
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
        assert_eq!(s2.skipped, 2, "second import skips all");
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
