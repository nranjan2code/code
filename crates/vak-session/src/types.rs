use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use vak_llm::Message;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct FrozenContract {
    pub app_version: String,
    pub provider: String,
    pub model: String,
    pub system_prompt: String,
    pub tools: Vec<String>,
    pub permission_mode: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct SessionHeader {
    pub session_id: String,
    pub project_id: String,
    pub created_at: DateTime<Utc>,
    pub project_root: PathBuf,
    pub parent_session_id: Option<String>,
    pub contract: FrozenContract,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MessageRecord {
    pub message: Message,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum EntryPayload {
    Header(SessionHeader),
    Message(MessageRecord),
    Audit {
        name: String,
        data: serde_json::Value,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Entry {
    pub id: String,
    pub parent_id: Option<String>,
    pub ts: DateTime<Utc>,
    #[serde(flatten)]
    pub payload: EntryPayload,
}
impl Entry {
    pub fn new(parent_id: Option<String>, payload: EntryPayload) -> Self {
        Self {
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
    #[error("invalid identifier: {0}")]
    InvalidIdentifier(String),
    #[error("corrupt session at line {line}: {message}")]
    Corrupt { line: usize, message: String },
    #[error("session file already exists: {0}")]
    Exists(std::path::PathBuf),
    #[error("session is locked by another process: {0}")]
    Locked(std::path::PathBuf),
    #[error("session header is missing or invalid")]
    MissingHeader,
}
