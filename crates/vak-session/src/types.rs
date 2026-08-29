use std::path::PathBuf;

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

use vak_llm::{Message, Usage};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FrozenContract {
    pub app_version: String,
    pub provider: String,
    pub model: String,
    /// Frozen route ladder (docs/design/27 Phase B): ordered candidate
    /// legs committed at admission; dispatch walks it top-down on typed
    /// failures. Walking the ladder IS contract execution — never a
    /// mid-contract switch. Empty/missing ⇒ single-model legacy.
    #[serde(default)]
    pub route_ladder: Vec<vak_llm::RouteLeg>,
    /// Objective the ladder was ordered for (Phase R): "utility" |
    /// "balanced" | "quality-critical". Empty on legacy headers.
    #[serde(default)]
    pub route_objective: String,
    /// Freeze-time routing warnings (thin chain, dominant failure
    /// domain, unreachable cross-model fallbacks). Audit-only context,
    /// never model-visible input. Empty on legacy headers.
    #[serde(default)]
    pub route_annotations: Vec<String>,
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
    /// Full-context reset (doc 27 Phase H): when true, the projection
    /// replaces EVERYTHING with this summary — the reset-with-handoff
    /// rescue. Default false; older ledgers parse unchanged.
    #[serde(default)]
    pub reset_all: bool,
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

/// A durable objective with acceptance criteria (docs/design/27 Phase H).
/// Status transitions append new entries — the ledger never rewrites.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct GoalEntry {
    pub goal_id: String,
    pub objective: String,
    pub criteria: Vec<String>,
    pub status: GoalStatus,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum GoalStatus {
    Active,
    /// Completed AND independently audited (deterministic checks and/or
    /// judge) — never self-reported alone.
    Done {
        audited: bool,
    },
    /// Audit budget exhausted without verification; run proceeds so it
    /// can never trap the model.
    Unverified {
        reason: String,
    },
}

/// Durable, projection-neutral lifecycle fact used to rebuild native output
/// timelines. Activity never enters the model context and never replaces the
/// message/tool records that remain the source of conversational truth.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ActivityRecord {
    pub activity_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub turn: Option<usize>,
    #[serde(rename = "activity_kind")]
    pub kind: ActivityKind,
    pub status: ActivityStatus,
    pub label: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub detail: Option<String>,
    #[serde(default)]
    pub data: BTreeMap<String, String>,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ActivityKind {
    Approval,
    Retry,
    RouteFallback,
    Subagent,
    Diagnostic,
    Run,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ActivityStatus {
    Pending,
    Running,
    Succeeded,
    Failed,
    Denied,
    Cancelled,
    Partial,
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
    /// Goal lifecycle (doc 27 Phase H). Never model-visible.
    Goal(GoalEntry),
    /// UI/audit lifecycle facts; never model-visible.
    Activity(ActivityRecord),
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
