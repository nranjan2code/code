use std::path::PathBuf;

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use vak_llm::{Message, Usage};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FrozenContract {
    pub app_version: String,
    pub provider: String,
    pub model: String,
    pub system_prompt: String,
    pub tools: Vec<String>,
    pub permission_mode: String,
    #[serde(default)]
    pub skills: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SessionHeader {
    pub session_id: String,
    pub created_at: DateTime<Utc>,
    pub cwd: PathBuf,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub parent_session_id: Option<String>,
    pub contract: FrozenContract,
}

impl SessionHeader {
    pub fn contract_cwd(&self) -> PathBuf {
        self.cwd.clone()
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MessageRecord {
    pub message: Message,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub meta: Option<MessageMeta>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct MessageMeta {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub stop_reason: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub usage: Option<Usage>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CompactionEntry {
    pub summary: String,
    pub first_kept_entry_id: String,
    pub tokens_before: u64,
    /// Packet accounting (doc 27 Phase C): which visible message entries
    /// stayed verbatim vs became summary material. `None` for entries
    /// written before accounting existed.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub partition: Option<ContextPartition>,
}

/// The compaction-time packet partition: every message entry visible in
/// the current projection appears exactly once — verbatim (`selected`) or
/// summarized away (`dropped`). Prior compaction pseudo-entries appear in
/// neither list; they were settled by earlier compactions.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ContextPartition {
    pub selected_entry_ids: Vec<String>,
    pub dropped_entry_ids: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum EntryPayload {
    Header(SessionHeader),
    Message(MessageRecord),
    Compaction(CompactionEntry),
    /// Audit record for one unit of provider work (doc 27 Phase A).
    /// Never model-visible: `derive_messages` skips it.
    Receipt(vak_llm::WorkReceipt),
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Entry {
    pub id: String,
    #[serde(default)]
    pub parent_id: Option<String>,
    pub ts: DateTime<Utc>,
    #[serde(flatten)]
    pub payload: EntryPayload,
}

impl Entry {
    pub fn new(parent_id: Option<String>, payload: EntryPayload) -> Self {
        Entry {
            id: uuid::Uuid::now_v7().to_string(),
            parent_id,
            ts: Utc::now(),
            payload,
        }
    }
}

#[derive(Debug, thiserror::Error)]
pub enum SessionError {
    #[error("io error: {0}")]
    Io(#[from] std::io::Error),
    #[error("json error at line {line}: {message}")]
    Corrupt { line: usize, message: String },
    #[error("session file already exists: {0}")]
    Exists(std::path::PathBuf),
    #[error("session is locked by another process: {0}")]
    Locked(std::path::PathBuf),
}

/// Projection-based compaction plan (see SessionLog::plan_compaction).
#[derive(Debug, Clone)]
pub struct CompactionPlan {
    pub older: Vec<vak_llm::Message>,
    /// Id of the FIRST KEPT projected entry — the new compaction's
    /// first_kept_entry_id anchor.
    pub first_kept_entry_id: String,
    /// Where each visible message entry lands (doc 27 Phase C).
    pub partition: ContextPartition,
}
