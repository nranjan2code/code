use serde::{Deserialize, Serialize};
use serde_json::Value;
use vak_domain::{
    CapabilityEpoch, Event, PermissionMode, ProjectContext, ProjectId, RunId, RunStatus,
    SessionContract, SessionId,
};

#[derive(Clone, Debug, Deserialize, PartialEq, Eq)]
pub struct Health {
    pub status: String,
    #[serde(default)]
    pub protocol: u32,
    #[serde(default)]
    pub runtime_id: Option<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct Project {
    pub context: ProjectContext,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Eq)]
pub struct ProjectList {
    pub items: Vec<ProjectContext>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct Session {
    pub id: SessionId,
    pub project_id: ProjectId,
    #[serde(default)]
    pub created_at: Option<String>,
    #[serde(default)]
    pub status: Option<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct CreateSession {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub session_id: Option<SessionId>,
    pub project_id: ProjectId,
    pub contract: SessionContract,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct StartRun {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub run_id: Option<RunId>,
    pub session_id: SessionId,
    pub project_id: ProjectId,
    pub input: String,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct Run {
    pub run_id: RunId,
    pub session_id: SessionId,
    pub project_id: ProjectId,
    pub status: RunStatus,
    pub capability_epoch: CapabilityEpoch,
    #[serde(default)]
    pub output: String,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct Config {
    pub revision: u64,
    #[serde(alias = "config")]
    pub values: Value,
    #[serde(default)]
    pub warnings: Vec<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct ConfigPatch {
    pub project_id: ProjectId,
    pub revision: u64,
    #[serde(rename = "config")]
    pub patch: Value,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct Diagnostics {
    pub status: String,
    #[serde(default)]
    pub details: Value,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct CommandAccepted {
    pub request_id: String,
    #[serde(default)]
    pub run_id: Option<RunId>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct CancelResponse {
    pub cancelled: bool,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct SessionCreated {
    pub session_id: SessionId,
    pub project_id: ProjectId,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct Transcript {
    pub messages: Vec<vak_llm::Message>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct RunSnapshot {
    pub run_id: RunId,
    pub project_id: ProjectId,
    pub capability_epoch: CapabilityEpoch,
    pub status: RunStatus,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct StartRunResponse {
    pub run: RunSnapshot,
    pub input: String,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct ApiResponse<T> {
    pub request_id: String,
    pub result: T,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(untagged)]
pub enum ServerEvent {
    Domain(Event),
    Json(Value),
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct PermissionModeRequest {
    pub project_id: ProjectId,
    pub revision: u64,
    pub mode: PermissionMode,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Eq)]
pub struct Version {
    pub version: String,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct Task {
    pub id: String,
    pub project_id: Option<String>,
    pub spec: Value,
    pub status: String,
    pub updated_at: String,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct Memory {
    pub id: String,
    pub project_id: Option<String>,
    pub scope: String,
    pub kind: String,
    pub tag: String,
    pub text: String,
    pub created_at: String,
    pub updated_at: String,
    pub deleted_at: Option<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct Approval {
    pub id: String,
    pub run_id: String,
    pub status: String,
    pub request: Value,
    pub response: Option<Value>,
    pub created_at: String,
    pub resolved_at: Option<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct Skill {
    pub id: String,
    pub project_id: Option<String>,
    pub name: String,
    pub description: String,
    pub body_digest: Option<String>,
    pub status: String,
    pub created_at: String,
    pub updated_at: String,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct MemoryCreate {
    pub project_id: Option<ProjectId>,
    pub scope: String,
    pub kind: String,
    pub tag: String,
    pub text: String,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct MemoryPatch {
    pub text: String,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct Checkpoint {
    pub id: String,
    pub session_id: SessionId,
    pub manifest_digest: String,
    pub created_at: String,
    pub label: String,
    #[serde(default)]
    pub manifest: Option<Value>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct CheckpointCreate {
    pub label: String,
    pub manifest: Value,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct RestoreResponse {
    pub restored: bool,
    pub checkpoint_id: String,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct BackupReport {
    pub path: String,
    pub file_count: u64,
    pub total_bytes: u64,
    pub renamed: u64,
    pub skipped: u64,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct BackupExportRequest {
    pub directory: String,
    #[serde(default)]
    pub include_secrets: bool,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct BackupImportRequest {
    pub directory: String,
    #[serde(default = "default_conflict")]
    pub conflict: String,
}

fn default_conflict() -> String {
    "skip".to_owned()
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct FlowDefinition {
    pub name: String,
    pub path: String,
    pub valid: bool,
    #[serde(default)]
    pub nodes: usize,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct FlowRunRequest {
    pub project_id: ProjectId,
    pub session_id: SessionId,
    pub input: String,
    #[serde(default)]
    pub resume: bool,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct EvalRequest {
    #[serde(default)]
    pub cases: Vec<Value>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct EvalReport {
    pub total: usize,
    pub passed: usize,
    pub failed: usize,
}
