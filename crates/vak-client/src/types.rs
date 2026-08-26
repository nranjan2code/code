use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

// ── Health ───────────────────────────────────────────────────────────

#[derive(Debug, Clone, Deserialize)]
pub struct HealthResponse {
    pub status: String,
    pub provider: String,
    pub model: String,
    pub permission_mode: String,
    pub sandbox: String,
    pub context_window: Option<u32>,
    pub cwd: String,
    pub warnings: Vec<String>,
}

// ── Sessions ─────────────────────────────────────────────────────────

#[derive(Debug, Clone, Deserialize)]
pub struct SessionSummary {
    pub session_id: String,
    pub cwd: String,
    pub created_at: Option<DateTime<Utc>>,
    pub updated_at: Option<DateTime<Utc>>,
    pub entries: u64,
    pub title: Option<String>,
    pub running: bool,
    pub archived: bool,
}

#[derive(Debug, Clone, Deserialize)]
pub struct SessionListResponse {
    pub sessions: Vec<SessionSummary>,
}

#[derive(Debug, Clone, Serialize)]
pub struct CreateSessionRequest {
    pub cwd: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct CreateSessionResponse {
    pub session_id: String,
}

// ── Run ──────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize)]
pub struct RunRequest {
    pub prompt: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub attachments: Option<Vec<Attachment>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub goal: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub criteria: Option<Vec<String>>,
}

#[derive(Debug, Clone, Serialize, Default)]
pub struct Attachment {
    #[serde(default)]
    pub mime: String,
    pub data: String,
}

impl Attachment {
    pub fn new(data: impl Into<String>) -> Self {
        Self {
            mime: "image/png".into(),
            data: data.into(),
        }
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct SteeringRequest {
    pub text: String,
}

// ── Agent events (SSE stream) ───────────────────────────────────────

#[derive(Debug, Clone, Deserialize)]
#[serde(tag = "type")]
pub enum AgentEvent {
    StreamOpened,
    TurnStart { turn: u32 },
    Stream(StreamEvent),
    ToolCallStart {
        id: String,
        name: String,
        args_json: String,
    },
    ToolCallEnd {
        id: String,
        name: String,
        is_error: bool,
        result_preview: Option<String>,
    },
    TurnEnd {
        usage: Usage,
    },
    StopHookContinuation {
        reason: String,
    },
    RetryScheduled {
        attempt: u32,
        delay_ms: u64,
        reason: String,
    },
    RouteFallback {
        to_provider: String,
        to_model: String,
    },
    ContextCompacting {
        estimated_tokens: u32,
    },
    ContextCompacted {
        before_tokens: u32,
        after_tokens: u32,
        summarized_messages: u32,
        selected_messages: u32,
        dropped_messages: u32,
    },
    HandoffReset {
        before_tokens: u32,
    },
    ApprovalRequested {
        id: String,
        tool: String,
        args_json: String,
        reason: String,
    },
    SubagentStarted {
        label: String,
    },
    SubagentToolCall {
        label: String,
        name: String,
        is_error: bool,
    },
    SubagentUsage {
        label: String,
        input_tokens: u32,
        output_tokens: u32,
    },
    SubagentFinished {
        label: String,
        is_error: bool,
        elapsed_ms: u64,
    },
    RunFinished {
        summary: String,
        is_error: bool,
    },
    Lagged,
    Error {
        error: String,
    },
}

#[derive(Debug, Clone, Deserialize)]
#[serde(tag = "type")]
pub enum StreamEvent {
    TextDelta {
        delta: String,
        partial: Option<serde_json::Value>,
    },
    ThinkingDelta {
        delta: String,
        partial: Option<serde_json::Value>,
    },
    Start {
        partial: Option<serde_json::Value>,
    },
    ToolUseStart {
        index: u32,
        id: String,
        name: String,
        partial: Option<serde_json::Value>,
    },
    ToolInputDelta {
        index: u32,
        delta: String,
        partial: Option<serde_json::Value>,
    },
    End {
        message: Option<serde_json::Value>,
    },
}

// ── Usage ────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Default, Deserialize, Serialize)]
pub struct Usage {
    #[serde(default)]
    pub input_tokens: u32,
    #[serde(default)]
    pub output_tokens: u32,
    pub cache_read_input_tokens: Option<u32>,
    pub cache_creation_input_tokens: Option<u32>,
}

// ── Transcript ───────────────────────────────────────────────────────

#[derive(Debug, Clone, Deserialize)]
pub struct TranscriptResponse {
    pub count: usize,
    pub usage: Usage,
    pub messages: Vec<TranscriptMessage>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct TranscriptMessage {
    pub role: String,
    pub content: Vec<ContentBlock>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(tag = "type")]
pub enum ContentBlock {
    #[serde(rename = "text")]
    Text { text: String },
    #[serde(rename = "thinking")]
    Thinking {
        text: String,
        signature: Option<String>,
    },
    #[serde(rename = "tool_use")]
    ToolUse {
        id: String,
        name: String,
        input: serde_json::Value,
    },
    #[serde(rename = "tool_result")]
    ToolResult {
        tool_use_id: String,
        content: String,
        is_error: bool,
    },
    #[serde(rename = "image")]
    Image {
        source: ImageSource,
    },
}

#[derive(Debug, Clone, Deserialize)]
pub struct ImageSource {
    #[serde(rename = "type")]
    pub source_type: String,
    pub media_type: String,
    pub data: String,
}

// ── Config ───────────────────────────────────────────────────────────

#[derive(Debug, Clone, Deserialize)]
pub struct ConfigResponse {
    pub provider: String,
    pub model: String,
    pub max_tokens: u32,
    pub max_turns: u32,
    pub permission_mode: String,
    pub subagents: serde_json::Value,
    pub max_retries: u32,
    pub retry_base_backoff_ms: u64,
    pub request_timeout_secs: u32,
    pub run_retry_attempts: u32,
    pub run_retry_base_backoff_ms: u64,
    pub circuit_breaker_threshold: u32,
    pub circuit_breaker_cooldown_secs: u32,
    pub context_window: Option<u32>,
    pub theme: String,
    pub bell: bool,
    pub stop_policy: StopPolicy,
    pub route: RouteConfig,
    pub integrations: IntegrationsConfig,
    pub paths: PathsConfig,
    pub warnings: Vec<String>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct StopPolicy {
    pub enabled: bool,
    pub marker_gate: serde_json::Value,
    pub verify_gate: serde_json::Value,
    pub max_blocks: serde_json::Value,
}

#[derive(Debug, Clone, Deserialize)]
pub struct RouteConfig {
    pub objective: Option<String>,
    pub fallback_models: Vec<String>,
    pub max_fallbacks: u32,
    pub quality_hints: serde_json::Value,
}

#[derive(Debug, Clone, Deserialize)]
pub struct IntegrationsConfig {
    pub mcp_servers: Vec<String>,
    pub hooks: u32,
    pub skills: Vec<String>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct PathsConfig {
    pub project_config: String,
    pub global_config: String,
    pub sessions_home: String,
    pub cwd: String,
}

#[derive(Debug, Clone, Serialize, Default)]
pub struct PatchConfigRequest {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub provider: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max_turns: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub permission_mode: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub theme: Option<String>,
}

// ── Search ───────────────────────────────────────────────────────────

#[derive(Debug, Clone, Deserialize)]
pub struct SearchHit {
    #[serde(default)]
    pub project_hash: Option<String>,
    pub session_id: String,
    pub entry_id: String,
    pub ts: String,
    pub role: String,
    pub score: f32,
    pub snippet: String,
}

#[derive(Debug, Clone, Deserialize)]
pub struct SearchResponse {
    pub all: bool,
    pub hits: Vec<SearchHit>,
}

// ── FinOps ───────────────────────────────────────────────────────────

#[derive(Debug, Clone, Deserialize)]
pub struct FinopsResponse {
    pub day_usd: f64,
    pub run_cap_usd: Option<f64>,
    pub day_cap_usd: Option<f64>,
    pub unknown_rows: u32,
    pub total_rows: u32,
    pub by_provider: Vec<FinopsEntry>,
    pub by_model: Vec<FinopsEntry>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct FinopsEntry {
    pub name: String,
    pub usd: f64,
    pub calls: u32,
}

// ── Checkpoints ──────────────────────────────────────────────────────

#[derive(Debug, Clone, Deserialize)]
pub struct Checkpoint {
    pub sequence: u32,
    pub created_at: String,
    pub description: Option<String>,
}

// ── Skills ───────────────────────────────────────────────────────────

#[derive(Debug, Clone, Deserialize)]
pub struct Skill {
    pub name: String,
    pub description: String,
}

// ── Memory ───────────────────────────────────────────────────────────

#[derive(Debug, Clone, Deserialize)]
pub struct MemoryNote {
    pub id: String,
    pub text: String,
    pub created_at: String,
    pub tier: String,
}

// ── Tasks ────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Deserialize)]
pub struct Task {
    pub id: String,
    pub name: String,
    #[serde(rename = "type")]
    pub task_type: String,
    pub enabled: bool,
    pub schedule: Option<String>,
    pub prompt: String,
}

// ── Approvals ────────────────────────────────────────────────────────

#[derive(Debug, Clone, Deserialize)]
pub struct PendingApproval {
    pub id: String,
    pub session_id: String,
    pub tool: String,
    pub args_json: String,
    pub reason: String,
}

// ── Providers ────────────────────────────────────────────────────────

#[derive(Debug, Clone, Deserialize)]
pub struct ProviderInfo {
    pub name: String,
    pub configured: bool,
}

#[derive(Debug, Clone, Deserialize)]
pub struct ModelInfo {
    pub id: String,
    pub name: Option<String>,
}

// ── Errors ───────────────────────────────────────────────────────────

#[derive(Debug, Clone, Deserialize)]
pub struct ErrorResponse {
    pub error: String,
}
