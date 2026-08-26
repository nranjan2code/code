use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use serde_json::Value;

// ── LLM types (wire-compatible with vak-llm) ─────────────────────────

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Role {
    User,
    Assistant,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ContentBlock {
    Text {
        text: String,
    },
    Thinking {
        text: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        signature: Option<String>,
    },
    ToolUse {
        id: String,
        name: String,
        input: Value,
    },
    ToolResult {
        tool_use_id: String,
        content: String,
        #[serde(default)]
        is_error: bool,
    },
    Image {
        source: ImageSource,
    },
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ImageSource {
    #[serde(rename = "type")]
    pub source_type: String,
    pub media_type: String,
    pub data: String,
}

impl ContentBlock {
    pub fn text(s: impl Into<String>) -> Self {
        ContentBlock::Text { text: s.into() }
    }
    pub fn tool_result(tool_use_id: impl Into<String>, content: impl Into<String>) -> Self {
        ContentBlock::ToolResult {
            tool_use_id: tool_use_id.into(),
            content: content.into(),
            is_error: false,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Message {
    pub role: Role,
    pub content: Vec<ContentBlock>,
}

impl Message {
    pub fn user_text(s: impl Into<String>) -> Self {
        Message {
            role: Role::User,
            content: vec![ContentBlock::text(s)],
        }
    }
    pub fn assistant(content: Vec<ContentBlock>) -> Self {
        Message {
            role: Role::Assistant,
            content,
        }
    }
    pub fn text_content(&self) -> String {
        self.content
            .iter()
            .filter_map(|b| match b {
                ContentBlock::Text { text } => Some(text.as_str()),
                _ => None,
            })
            .collect::<Vec<_>>()
            .join("\n")
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AssistantMessage {
    pub role: Role,
    pub content: Vec<ContentBlock>,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Usage {
    #[serde(default)]
    pub input_tokens: u64,
    #[serde(default)]
    pub output_tokens: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cache_read_input_tokens: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cache_creation_input_tokens: Option<u64>,
}

impl Usage {
    pub fn total_tokens(&self) -> u64 {
        self.input_tokens + self.output_tokens
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum StopReason {
    EndTurn,
    ToolUse,
    MaxTokens,
    Aborted,
}

// ── StreamEvent (wire-compatible with vak_llm::stream) ───────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum StreamEvent {
    Start {
        partial: AssistantMessage,
    },
    TextDelta {
        delta: String,
        partial: AssistantMessage,
    },
    ThinkingDelta {
        delta: String,
        partial: AssistantMessage,
    },
    ToolUseStart {
        index: usize,
        id: String,
        name: String,
        partial: AssistantMessage,
    },
    ToolInputDelta {
        index: usize,
        delta: String,
        partial: AssistantMessage,
    },
    End {
        message: AssistantMessage,
    },
}

impl StreamEvent {
    pub fn partial(&self) -> &AssistantMessage {
        match self {
            StreamEvent::Start { partial }
            | StreamEvent::TextDelta { partial, .. }
            | StreamEvent::ThinkingDelta { partial, .. }
            | StreamEvent::ToolUseStart { partial, .. }
            | StreamEvent::ToolInputDelta { partial, .. } => partial,
            StreamEvent::End { message } => message,
        }
    }
}

// ── AgentEvent (wire-compatible with vak_agent::AgentEvent) ──────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum AgentEvent {
    TurnStart {
        turn: usize,
    },
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
        estimated_tokens: u64,
    },
    ContextCompacted {
        before_tokens: u64,
        after_tokens: u64,
        summarized_messages: usize,
        selected_messages: usize,
        dropped_messages: usize,
    },
    HandoffReset {
        before_tokens: u64,
    },
    StreamOpened,
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
        input_tokens: u64,
        output_tokens: u64,
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
    #[serde(other)]
    Lagged,
}

// ── TurnOutcome ──────────────────────────────────────────────────────

#[derive(Debug, Clone)]
pub enum TurnOutcome {
    Completed { usage: Usage },
    Aborted,
    Failed { error: String },
}

// ── Approval ─────────────────────────────────────────────────────────

#[derive(Debug, Clone)]
pub struct ApprovalRequest {
    pub id: String,
    pub tool: String,
    pub args_json: String,
    pub reason: String,
}

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

// ── Config ───────────────────────────────────────────────────────────

#[derive(Debug, Clone, Deserialize)]
pub struct ConfigResponse {
    pub provider: String,
    pub model: String,
    pub max_tokens: u32,
    pub max_turns: u32,
    pub permission_mode: String,
    pub subagents: Value,
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

impl Default for ConfigResponse {
    fn default() -> Self {
        Self {
            provider: String::new(),
            model: String::new(),
            max_tokens: 8192,
            max_turns: 50,
            permission_mode: "WorkspaceWrite".into(),
            subagents: serde_json::json!(true),
            max_retries: 3,
            retry_base_backoff_ms: 500,
            request_timeout_secs: 120,
            run_retry_attempts: 2,
            run_retry_base_backoff_ms: 1000,
            circuit_breaker_threshold: 5,
            circuit_breaker_cooldown_secs: 60,
            context_window: None,
            theme: "dark".into(),
            bell: true,
            stop_policy: StopPolicy {
                enabled: true,
                marker_gate: serde_json::json!(true),
                verify_gate: serde_json::json!(true),
                max_blocks: serde_json::json!(20),
            },
            route: RouteConfig {
                objective: None,
                fallback_models: vec![],
                max_fallbacks: 2,
                quality_hints: serde_json::json!(true),
            },
            integrations: IntegrationsConfig {
                mcp_servers: vec![],
                hooks: 0,
                skills: vec![],
            },
            paths: PathsConfig {
                project_config: String::new(),
                global_config: String::new(),
                sessions_home: String::new(),
                cwd: String::new(),
            },
            warnings: vec![],
        }
    }
}

#[derive(Debug, Clone, Deserialize)]
pub struct StopPolicy {
    pub enabled: bool,
    pub marker_gate: Value,
    pub verify_gate: Value,
    pub max_blocks: Value,
}

#[derive(Debug, Clone, Deserialize)]
pub struct RouteConfig {
    pub objective: Option<String>,
    pub fallback_models: Vec<String>,
    pub max_fallbacks: u32,
    pub quality_hints: Value,
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

// ── Transcript ───────────────────────────────────────────────────────

#[derive(Debug, Clone, Deserialize)]
pub struct TranscriptResponse {
    pub count: usize,
    pub usage: Usage,
    pub messages: Vec<Message>,
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

// ── Custom commands ──────────────────────────────────────────────────

#[derive(Debug, Clone, Deserialize)]
pub struct CustomCommandInfo {
    pub name: String,
    pub description: String,
    #[serde(default)]
    pub source: String,
    /// Full markdown body; `$ARGUMENTS` is substituted at invocation.
    #[serde(default)]
    pub template: String,
}

#[derive(Debug, Clone, Deserialize)]
pub struct CustomCommandsResponse {
    pub commands: Vec<CustomCommandInfo>,
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

/// Wire shape mirrors vak_core::tasks::TaskDef (subset the TUI renders).
#[derive(Debug, Clone, Deserialize)]
pub struct Task {
    pub id: String,
    pub name: String,
    #[serde(default)]
    pub prompt: String,
    pub interval_secs: u64,
    pub enabled: bool,
    #[serde(default)]
    pub schedule: Option<String>,
    #[serde(default)]
    pub script: Option<String>,
    #[serde(default)]
    pub model_pin: Option<String>,
}

impl Task {
    pub fn is_watchdog(&self) -> bool {
        self.script.is_some()
    }
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

// ── Inbox ────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Deserialize)]
pub struct InboxEntry {
    pub id: String,
    pub ts: String,
    pub kind: String,
    pub title: String,
    pub body: String,
    #[serde(default)]
    pub session_id: Option<String>,
    #[serde(default)]
    pub task_id: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct InboxResponse {
    pub entries: Vec<InboxEntry>,
    pub unread_count: u32,
}

#[derive(Debug, Clone, Deserialize)]
pub struct InboxAckResponse {
    pub acked: bool,
}

#[derive(Debug, Clone, Deserialize)]
pub struct UnreadCountResponse {
    pub count: u32,
}

// ── Proposals ────────────────────────────────────────────────────────

#[derive(Debug, Clone, Deserialize)]
pub struct ProposalInfo {
    pub id: String,
    pub name: String,
    pub description: String,
}

#[derive(Debug, Clone, Deserialize)]
pub struct ProposalsResponse {
    pub proposals: Vec<ProposalInfo>,
}

// ── Subagents ────────────────────────────────────────────────────────

#[derive(Debug, Clone, Deserialize)]
pub struct SubagentInfo {
    pub label: String,
    pub session_id: String,
    #[serde(default)]
    pub is_error: bool,
    #[serde(default)]
    pub elapsed_ms: Option<u64>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct SubagentsResponse {
    pub subagents: Vec<SubagentInfo>,
}

// ── Checkpoint restore ───────────────────────────────────────────────

#[derive(Debug, Clone, Deserialize)]
pub struct CheckpointRestoreResponse {
    pub restored: u32,
    pub deleted: u32,
    pub seq: u32,
}

// ── Provider key management ──────────────────────────────────────────

#[derive(Debug, Clone, Serialize)]
pub struct ProviderKeyRequest {
    pub provider: String,
    pub key: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct ProviderRef {
    pub provider: String,
}

#[derive(Debug, Clone, Deserialize)]
pub struct ProviderKeyResponse {
    pub provider: String,
    pub env_var: String,
    pub configured: bool,
}

#[derive(Debug, Clone, Deserialize)]
pub struct ProviderRemoveResponse {
    pub provider: String,
    pub env_var: String,
    pub configured: bool,
    pub shadowed_by_env: bool,
}

// ── Errors ───────────────────────────────────────────────────────────

#[derive(Debug, Clone, Deserialize)]
pub struct ErrorResponse {
    pub error: String,
}
