//! vak-agent: the agent loop.
//!
//! Errors are values: run() never panics; every failure mode is a typed
//! TurnOutcome. Model context is always projected from the session log
//! (model-visible means logged).

pub mod circuit;
pub mod context;
pub mod goal;
pub mod spend;
pub mod steering;
pub mod stop_policy;
pub mod task;
pub mod workspace;

pub use circuit::{CircuitBreaker, CircuitBreakerConfig, CircuitOpen};
pub use goal::GoalState;
pub use spend::{SpendCheck, SpendGate};
pub use stop_policy::{BlockReason, StopPolicy};
pub use task::{ActiveSubagent, SubagentHandle, SubagentRegistry, TaskDeps, TaskTool};
pub use workspace::WorkspaceDelta;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum WorkMode {
    #[default]
    Direct,
    Managed,
    Auto,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ApprovalMode {
    #[default]
    Ask,
    ApproveSafe,
    AutoApprove,
}

pub fn auto_approve(
    approval_mode: ApprovalMode,
    source: AskSource,
    tool: &str,
    input: &Value,
    mode: Mode,
    sandboxed: bool,
    cwd: &std::path::Path,
) -> bool {
    if matches!(source, AskSource::Rule | AskSource::CircuitBreaker) {
        return false;
    }
    if matches!(approval_mode, ApprovalMode::AutoApprove) {
        return true;
    }
    if !matches!(approval_mode, ApprovalMode::ApproveSafe)
        || !matches!(source, AskSource::ModeDefault | AskSource::Scope)
    {
        return false;
    }
    if matches!(tool, "read" | "glob" | "grep" | "ls" | "search") {
        return true;
    }
    if matches!(tool, "write" | "edit") {
        return input
            .get("path")
            .and_then(Value::as_str)
            .is_some_and(|path| {
                vak_permission::path_in_workspace(std::path::Path::new(path), cwd)
            });
    }
    tool == "bash" && sandboxed && !matches!(mode, Mode::FullAccess)
}

use std::collections::HashMap;
use std::sync::{Arc, Mutex as StdMutex};

use futures::FutureExt;
use serde_json::Value;
use tokio::sync::{Mutex, mpsc};
use tokio_util::sync::CancellationToken;
use vak_llm::{
    AssistantMessage, ChatRequest, ContentBlock, LlmError, Message, Provider, Role, StopReason,
    StreamEvent, Usage,
    work::{AttemptReason, FailureDomain, Settlement, StepLedger, WorkPurpose},
};
use vak_permission::{AskSource, Decision, Mode, PermissionEngine};
use vak_session::{MessageMeta, MessageRecord, SessionLog};
use vak_tools::{Tool, ToolContext, ToolErrorKind, ToolOutput};

pub use steering::{DrainMode, SteeringQueues};

pub use async_trait;

/// Host-owned transformation applied to every user message admitted to a
/// running agent, including messages queued as steering while a turn is busy.
/// It is intentionally outside model control and receives the frozen session
/// capability packet through its closure.
pub type InputNormalizer = Arc<dyn Fn(Message) -> Result<Message, String> + Send + Sync>;

/// Host-owned executor for a managed static flow. The agent owns admission
/// and the live ledger; the host owns the flow engine to avoid a crate cycle.
#[async_trait::async_trait]
pub trait FlowDispatcher: Send + Sync {
    async fn dispatch(
        &self,
        args: &Value,
        session: Arc<Mutex<SessionLog>>,
        ctx: &ToolContext,
        approver: Option<Arc<dyn Approver>>,
    ) -> ToolOutput;
}

struct ManagedFlowTool {
    dispatcher: Arc<dyn FlowDispatcher>,
    session: Arc<Mutex<SessionLog>>,
    approver: Option<Arc<dyn Approver>>,
}

#[async_trait::async_trait]
impl Tool for ManagedFlowTool {
    fn name(&self) -> &str {
        "flow"
    }

    fn description(&self) -> &str {
        "Run a named static flow assigned by the active managed-work contract."
    }

    fn schema(&self) -> Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "flow": {"type": "string", "description": "Managed flow name"},
                "contract_id": {"type": "string", "description": "Active managed contract"},
                "work_item_id": {"type": "string", "description": "Flow-owned work item"}
            },
            "required": ["flow", "contract_id", "work_item_id"]
        })
    }

    fn claims(&self, _args: &Value) -> vak_tools::ResourceClaims {
        vak_tools::ResourceClaims {
            exclusive: true,
            read_only: false,
            paths: Vec::new(),
        }
    }

    async fn execute(&self, args: &Value, ctx: &ToolContext) -> ToolOutput {
        self.dispatcher
            .dispatch(args, self.session.clone(), ctx, self.approver.clone())
            .await
    }
}

#[derive(Debug, Clone, serde::Serialize)]
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
    /// First dispatch of the NEXT frozen-ladder leg after a typed failure
    /// of the previous one (Phase B). Walking the frozen ladder is contract
    /// execution; this event surfaces each leg change to every consumer.
    RouteFallback {
        to_provider: String,
        to_model: String,
    },
    ContextCompacting {
        estimated_tokens: u64,
    },
    /// Proactive retrieval surfaced N relevant older turns before compaction.
    ContextRetrieved {
        retrieved_count: usize,
        cap: usize,
    },
    ContextCompacted {
        before_tokens: u64,
        after_tokens: u64,
        summarized_messages: usize,
        /// Packet accounting (docs/design/17-context.md): how many visible message
        /// entries stayed verbatim vs became summary material.
        selected_messages: usize,
        dropped_messages: usize,
    },
    /// Reset-with-handoff fired (Phase H): the whole projection was
    /// replaced by a structured handoff summary.
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
    /// Latest durable managed-work projection. This is a live projection only;
    /// the session ledger remains the source of truth and can rebuild it.
    WorkState {
        projection: vak_session::work::WorkProjection,
    },
    RunFinished {
        summary: String,
        /// Whether the run ended badly. Consumers must not have to sniff
        /// `summary` for a "failed:" prefix to know something broke —
        /// errors are values (see SubagentFinished above).
        is_error: bool,
    },
    /// Live execution events from sandbox or bash commands.
    Sandbox(vak_tools::SandboxEvent),
}

#[derive(Debug)]
pub enum TurnOutcome {
    Completed { response: AssistantMessage },
    Aborted { partial: Option<AssistantMessage> },
    Failed { error: LlmError },
    MaxTurnsReached,
}

pub struct AgentConfig {
    /// The admitted result contract for this run. It is execution context,
    /// not model-authored authority; permission and broker checks remain the
    /// enforcement boundary.
    pub outcome: Option<vak_intent::OutcomeSpec>,
    pub work_mode: WorkMode,
    pub work_enabled: bool,
    pub max_work_items: usize,
    pub max_work_revisions: u32,
    pub system_prompt: String,
    pub model: String,
    pub tools: Vec<Arc<dyn Tool>>,
    /// Discovered MCP tool names accepted as compatibility aliases. Calls
    /// are rewritten to the `mcp` broker before authorization and dispatch.
    pub mcp_aliases: Arc<StdMutex<std::collections::HashMap<String, McpToolAlias>>>,
    /// Optional host dispatcher exposed only as the managed `flow` tool.
    pub flow_dispatcher: Option<Arc<dyn FlowDispatcher>>,
    /// Exact tool schemas admitted with the session. When absent, standalone
    /// agent users derive schemas from their runtime tools.
    pub tool_definitions: Option<Vec<vak_llm::ToolDefinition>>,
    /// Applies host commands and capability-bound prompt expansion before a
    /// message reaches the ledger or provider.
    pub input_normalizer: Option<InputNormalizer>,
    pub max_turns: usize,
    pub parallel_tools: bool,
    pub permission: Option<Arc<PermissionEngine>>,
    pub mode: Mode,
    pub approver: Option<Arc<dyn Approver>>,
    pub approval_mode: ApprovalMode,
    pub sandbox: Option<Arc<dyn vak_tools::sandbox::Sandbox>>,
    pub hooks: Option<Arc<Vec<vak_hooks::HookDef>>>,
    pub revocation_check: Option<RevocationCheck>,
    pub hook_recorder: Option<HookRecorder>,
    pub tool_activity_recorder: Option<ToolActivityRecorder>,
    /// Retries for transient provider errors (429/529/network) per step.
    pub max_retries: u32,
    /// Exponential backoff base: delay = base * 2^(attempt-1), jittered.
    pub retry_base_backoff_ms: u64,
    /// Whole-step deadline (connect + stream + collect). None disables.
    pub request_timeout: Option<std::time::Duration>,
    /// Shared cross-run provider-health breaker. None disables.
    pub circuit_breaker: Option<Arc<CircuitBreaker>>,
    /// Run-level endurance: when a model step exhausts its retry budget with
    /// a transient error (rate limit / overload / network / truncated
    /// stream), back off and re-attempt the same turn this many times
    /// before failing the run. Nothing has been committed to the ledger at
    /// that point, so the re-attempt is exact. 0 disables (fail on first
    /// step exhaustion).
    pub run_retry_attempts: u32,
    /// Exponential backoff base for run-level endurance, capped at 30s.
    pub run_retry_base_backoff_ms: u64,
    /// Hard cap on provider dispatches for one unit of work (docs/design/42-managed-work-contracts.mdPhase
    /// A). Exhaustion fails closed before another paid call goes out. The
    /// single-ladder default codifies today's worst case:
    /// `(max_retries + 1) * (run_retry_attempts + 1)`; the frozen ladder
    /// (Phase B) tightens this to `ladder + repair allowance`.
    pub dispatch_ceiling: u32,
    /// Long-horizon context policy (window, reserve, compaction trigger).
    pub context_policy: context::ContextPolicy,
    /// Built-in premature-completion gate. None disables entirely.
    pub stop_policy: Option<StopPolicy>,
    /// Pre-dispatch budget admission (docs/design/15-reliability.md). None
    /// disables spend gating entirely.
    pub spend_gate: Option<Arc<dyn SpendGate>>,
    /// Frozen route ladder (Phase B): primary-first candidate legs beyond
    /// the configured provider/model. Empty ⇒ single-model legacy.
    pub ladder: Vec<(Arc<dyn Provider>, String)>,
    /// Canonical configured provider names parallel to `ladder`. Adapter
    /// names are implementation details and must not enter routing evidence.
    pub ladder_provider_names: Vec<String>,
    /// Workspace-delta provider (Phase H MEA): supplies a bounded summary
    /// of what changed since the run-start checkpoint, feeding goal-mode
    /// auditors environment facts instead of transcript-only claims.
    pub workspace_delta: Option<Arc<dyn WorkspaceDelta>>,
    /// Reset-with-handoff rescue on still-over contexts (Phase H).
    pub handoff_reset: bool,
    /// Audit blocks per goal before degrading to Unverified.
    pub max_audit_blocks: u32,
}

pub type HookRecorder = Arc<dyn Fn(&vak_hooks::HookDef, bool, u64) + Send + Sync>;
pub type ToolActivityRecorder = Arc<dyn Fn(&str, &serde_json::Value, bool, u64) + Send + Sync>;
pub type RevocationCheck = Arc<dyn Fn(&str, &serde_json::Value) -> bool + Send + Sync>;

impl AgentConfig {
    pub fn new(system_prompt: impl Into<String>) -> Self {
        AgentConfig {
            outcome: None,
            work_mode: WorkMode::Direct,
            work_enabled: true,
            max_work_items: 20,
            max_work_revisions: 8,
            system_prompt: system_prompt.into(),
            model: String::new(),
            tools: Vec::new(),
            mcp_aliases: Arc::new(StdMutex::new(std::collections::HashMap::new())),
            flow_dispatcher: None,
            tool_definitions: None,
            input_normalizer: None,
            max_turns: 40,
            parallel_tools: true,
            permission: None,
            mode: Mode::WorkspaceWrite,
            approver: None,
            approval_mode: ApprovalMode::Ask,
            sandbox: None,
            hooks: None,
            revocation_check: None,
            hook_recorder: None,
            tool_activity_recorder: None,
            max_retries: 3,
            retry_base_backoff_ms: 500,
            request_timeout: Some(std::time::Duration::from_secs(600)),
            circuit_breaker: None,
            run_retry_attempts: 6,
            run_retry_base_backoff_ms: 2_000,
            dispatch_ceiling: (3 + 1) * (6 + 1),
            context_policy: Default::default(),
            stop_policy: Some(StopPolicy::default()),
            spend_gate: None,
            ladder: Vec::new(),
            ladder_provider_names: Vec::new(),
            workspace_delta: None,
            handoff_reset: true,
            max_audit_blocks: 2,
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct McpToolAlias {
    pub server: String,
    pub tool: String,
    pub description: String,
    pub schema: serde_json::Value,
}

#[async_trait::async_trait]
pub trait Approver: Send + Sync {
    async fn approve(&self, tool: &str, args_json: &str, reason: &str) -> bool;

    /// Whether a gate raised here reaches somebody who can answer it.
    ///
    /// `false` means every `Ask` on this surface is a foregone denial —
    /// nobody is listening, and `approve` will return `false` without
    /// having asked anyone. That is correct behaviour (AGENTS.md invariant
    /// 15: unattended surfaces fail closed) but it is also a *fact about
    /// this turn's callable interface*, and it has to be knowable before
    /// dispatch rather than only discoverable by burning a tool call on it.
    /// `vak_core::reach` reads this to decide what the system prompt may
    /// honestly advertise as usable.
    ///
    /// Defaults to `true`: an approver that does not say otherwise is one
    /// that resolves gates.
    fn answerable(&self) -> bool {
        true
    }
}

/// Errors worth surviving at run level: sustained fault windows, hung or
/// truncated streams. Permanent errors (auth, bad request, non-2xx api,
/// aborts) are excluded — retrying them cannot help.
fn is_transient_step_error(e: &LlmError) -> bool {
    match e {
        LlmError::RateLimit { .. } => e.is_retryable(),
        LlmError::Overloaded(_) | LlmError::Network(_) | LlmError::Parse(_) => true,
        _ => false,
    }
}

/// The breaker protects against a DEAD provider: blind failures with no
/// server guidance (network loss, watchdog deadlines, truncated or malformed
/// streams). Informed transience — 429 with Retry-After, explicit 503/529
/// overload — is the server saying "try again later"; endurance handles it
/// by waiting, and it must not open the circuit mid-window (found in the
/// live chaos campaign: an opened breaker killed runs the window would have
/// released seconds later).
fn trips_breaker(e: &LlmError) -> bool {
    matches!(e, LlmError::Network(_) | LlmError::Parse(_))
}

pub struct AutoApprove;

#[async_trait::async_trait]
impl Approver for AutoApprove {
    async fn approve(&self, _tool: &str, _args_json: &str, _reason: &str) -> bool {
        true
    }
}

pub struct AutoDeny;

#[async_trait::async_trait]
impl Approver for AutoDeny {
    async fn approve(&self, _tool: &str, _args_json: &str, _reason: &str) -> bool {
        false
    }

    fn answerable(&self) -> bool {
        false
    }
}

/// Compact JSON preview of a tool input; capped so UI surfaces never
/// absorb unbounded payloads.
fn args_preview(input: &Value) -> String {
    let json = serde_json::to_string(input).unwrap_or_else(|_| "{}".into());
    truncate_chars(&json, ARGS_PREVIEW_LIMIT)
}

fn result_preview(content: &str) -> Option<String> {
    if content.is_empty() {
        None
    } else {
        Some(truncate_chars(content, RESULT_PREVIEW_LIMIT))
    }
}

fn truncate_chars(s: &str, max: usize) -> String {
    if s.chars().count() <= max {
        return s.to_string();
    }
    let mut out: String = s.chars().take(max).collect();
    out.push('…');
    out
}

const ARGS_PREVIEW_LIMIT: usize = 400;
const RESULT_PREVIEW_LIMIT: usize = 2000;

#[derive(Clone)]
struct PendingToolCall {
    id: String,
    name: String,
    input: Value,
}

pub struct Agent {
    provider: Arc<dyn Provider>,
    pub session: Arc<Mutex<SessionLog>>,
    pub config: AgentConfig,
    /// Identical-call detector for the doom-loop guard, reset per run.
    run_call_counts: std::sync::Mutex<HashMap<String, u32>>,
    /// Active goal (Phase H): set via `set_goal`, consumed by the audit
    /// gate on completion claims.
    active_goal: Option<goal::GoalState>,
    /// Bash commands proven green this run — re-run before any done claim.
    obligations: Vec<String>,
    /// The reset-with-handoff rescue fires at most once per run.
    handoff_used: bool,
    /// Per-run bookkeeping for model-guided tool recovery (see RepairState).
    repair: RepairState,
}

impl Agent {
    pub fn new(provider: Arc<dyn Provider>, session: SessionLog, mut config: AgentConfig) -> Self {
        let session = Arc::new(Mutex::new(session));
        if let Some(dispatcher) = config.flow_dispatcher.clone() {
            config.tools.push(Arc::new(ManagedFlowTool {
                dispatcher,
                session: session.clone(),
                approver: config.approver.clone(),
            }));
            config.tool_definitions = Some(vak_tools::definitions(&config.tools));
        }
        Agent {
            provider,
            session,
            config,
            run_call_counts: std::sync::Mutex::new(HashMap::new()),
            active_goal: None,
            obligations: Vec::new(),
            handoff_used: false,
            repair: RepairState::default(),
        }
    }

    /// Arms goal mode for the next run: durable objective + acceptance
    /// criteria; completion becomes audited, never self-reported.
    pub fn set_goal(&mut self, objective: impl Into<String>, criteria: Vec<String>) {
        self.active_goal = Some(goal::GoalState {
            objective: objective.into(),
            criteria,
            audits_left: self.config.max_audit_blocks,
        });
    }

    fn normalize_input(&self, message: Message) -> Result<Message, String> {
        match &self.config.input_normalizer {
            Some(normalizer) => normalizer(message),
            None => Ok(message),
        }
    }

    fn tool_definitions(&self) -> Vec<vak_llm::ToolDefinition> {
        let mut definitions = self
            .config
            .tool_definitions
            .clone()
            .unwrap_or_else(|| vak_tools::definitions(&self.config.tools));
        // The `work` tool definition is admitted iff the `flow` capability
        // survived the full four-stage pipeline in vak-core (channel →
        // reach → contract → domain slice). `flow_dispatcher` is None when
        // the pipeline rejected `flow` — so we check that, not just
        // `work_mode == Managed`, which would append `work` even when the
        // domain slice correctly withheld it.
        if self.config.flow_dispatcher.is_some()
            && !definitions
                .iter()
                .any(|definition| definition.name == "work")
        {
            definitions.push(vak_llm::ToolDefinition::new(
                "work",
                "Inspect and update the durable managed work contract. Model transitions may only move ready to running, running to blocked, or running to ready_for_verification.",
                serde_json::json!({
                    "type": "object",
                    "properties": {
                        "operation": {"type": "string", "enum": ["get", "transition", "attach_evidence"]},
                        "item_id": {"type": "string"},
                        "to": {"type": "string", "enum": ["running", "blocked", "ready_for_verification"]},
                        "reason": {"type": "string"},
                        "evidence": {"type": "object"}
                    },
                    "required": ["operation"]
                }),
            ));
        }
        let aliases = self
            .config
            .mcp_aliases
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        for (name, alias) in aliases.iter() {
            if !definitions
                .iter()
                .any(|definition| definition.name == *name)
            {
                definitions.push(vak_llm::ToolDefinition::new(
                    name,
                    format!(
                        "{} (MCP compatibility alias; routed through mcp/{}/{})",
                        alias.description, alias.server, alias.tool
                    ),
                    alias.schema.clone(),
                ));
            }
        }
        definitions
    }

    pub async fn run(
        &mut self,
        prompt: &str,
        steering: &SteeringQueues,
        cancel: CancellationToken,
        events: mpsc::Sender<AgentEvent>,
    ) -> TurnOutcome {
        self.run_message(Message::user_text(prompt), steering, cancel, events)
            .await
    }

    /// Run with a prebuilt prompt `Message` — the seam for multimodal
    /// (image) input; the ledger stores exactly what the model sees.
    pub async fn run_message(
        &mut self,
        prompt: Message,
        steering: &SteeringQueues,
        cancel: CancellationToken,
        events: mpsc::Sender<AgentEvent>,
    ) -> TurnOutcome {
        let prompt = match self.normalize_input(prompt) {
            Ok(prompt) => prompt,
            Err(error) => {
                return TurnOutcome::Failed {
                    error: LlmError::InvalidRequest(error),
                };
            }
        };
        let prompt_owned = prompt.text_content();
        let mut bash_calls_this_run: u32 = 0;
        let mut verification_stale = false;
        let mut user_completion_released = false;
        self.obligations.clear();
        self.handoff_used = false;
        self.run_call_counts
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .clear();
        if self.active_goal.is_some() {
            // Goal lifecycle opens the run (audit-only entry).
            let mut session = self.session.lock().await;
            if let Some(g) = &self.active_goal {
                let _ = session.append_goal(vak_session::types::GoalEntry {
                    goal_id: format!("goal-{}", chrono::Utc::now().timestamp_millis()),
                    objective: g.objective.clone(),
                    criteria: g.criteria.clone(),
                    status: vak_session::types::GoalStatus::Active,
                });
            }
        }
        let prompt_entry = self.session.lock().await.append_message(MessageRecord {
            message: prompt,
            meta: None,
        });
        let prompt_entry = match prompt_entry {
            Ok(entry) => entry,
            Err(error) => {
                return TurnOutcome::Failed {
                    error: LlmError::Network(format!("session write failed: {error}")),
                };
            }
        };
        if self.config.work_mode == WorkMode::Managed && !self.config.work_enabled {
            return TurnOutcome::Failed {
                error: LlmError::InvalidRequest("managed work is disabled by configuration".into()),
            };
        }
        if self.config.work_enabled && self.config.work_mode == WorkMode::Managed {
            let entry_id = prompt_entry.id.clone();
            if let Err(error) = self.resolve_managed_input(&prompt_owned).await {
                return TurnOutcome::Failed {
                    error: LlmError::InvalidRequest(error),
                };
            }
            if let Err(error) = self
                .start_managed_contract(&prompt_owned, entry_id, &cancel, &events)
                .await
            {
                return TurnOutcome::Failed {
                    error: LlmError::InvalidRequest(error),
                };
            }
            self.emit_work_state(&events).await;
            if self
                .session
                .lock()
                .await
                .work_projection()
                .ok()
                .flatten()
                .is_some_and(|work| {
                    work.status == vak_session::types::WorkContractStatus::AwaitingInput
                })
            {
                let mut response = AssistantMessage::empty(self.config.model.clone());
                let pending = self
                    .session
                    .lock()
                    .await
                    .work_projection()
                    .ok()
                    .flatten()
                    .map(|work| {
                        work.contract
                            .assumptions
                            .iter()
                            .filter(|assumption| {
                                assumption.requires_confirmation && assumption.resolution.is_none()
                            })
                            .map(|assumption| {
                                format!("- {}: {}", assumption.assumption_id, assumption.text)
                            })
                            .collect::<Vec<_>>()
                            .join("\n")
                    })
                    .unwrap_or_default();
                response.content.push(ContentBlock::text(
                    format!(
                        "I created the managed work contract, but need these assumptions confirmed before starting:\n{pending}\nReply with: answer: <your answer>.",
                    ),
                ));
                let _ = self.session.lock().await.append_message(MessageRecord {
                    message: response.clone().into_message(),
                    meta: None,
                });
                return TurnOutcome::Completed { response };
            }
        }

        let mut turn = 0usize;
        let mut outcome_turns = 0usize;
        self.repair.reset();
        self.run_call_counts
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .clear();
        let mut stop_blocks_left = self
            .config
            .stop_policy
            .as_ref()
            .map(|p| p.max_blocks)
            .unwrap_or(0);
        loop {
            if cancel.is_cancelled() {
                return TurnOutcome::Aborted { partial: None };
            }
            if !steering.wait_if_paused(&cancel).await {
                return TurnOutcome::Aborted { partial: None };
            }
            if turn >= self.config.max_turns {
                return TurnOutcome::MaxTurnsReached;
            }
            if self
                .config
                .outcome
                .as_ref()
                .and_then(|outcome| outcome.max_turns)
                .is_some_and(|max| outcome_turns >= max)
            {
                return TurnOutcome::MaxTurnsReached;
            }

            {
                let mut session = self.session.lock().await;
                while let Some(update) = steering.take_outcome_update() {
                    self.config.outcome = update.outcome.clone();
                    if let Err(error) = session.append_intent(update) {
                        return TurnOutcome::Failed {
                            error: LlmError::Network(format!(
                                "outcome revision write failed: {error}"
                            )),
                        };
                    }
                }
                for message in steering.drain(DrainMode::OneAtATime) {
                    let message = match self.normalize_input(message) {
                        Ok(message) => message,
                        Err(error) => {
                            return TurnOutcome::Failed {
                                error: LlmError::InvalidRequest(error),
                            };
                        }
                    };
                    let request = message.text_content();
                    let active_revision = session.active_goal_revision();
                    let relation = vak_intent::classify_goal_update(&request, active_revision);
                    let update = vak_intent::GoalUpdate {
                        revision: session
                            .latest_goal_update()
                            .map(|update| update.revision)
                            .unwrap_or(0)
                            .saturating_add(1),
                        relation,
                        request: request.clone(),
                        supersedes_revision: matches!(
                            relation,
                            vak_intent::GoalRelation::Corrects | vak_intent::GoalRelation::Replaces
                        )
                        .then_some(active_revision)
                        .flatten(),
                    };
                    if let Err(error) = session.append_goal_update(update) {
                        return TurnOutcome::Failed {
                            error: LlmError::Network(format!("goal update write failed: {error}")),
                        };
                    }
                    if StopPolicy::is_done_message(&message.text_content()) {
                        user_completion_released = true;
                    }
                    let _ = session.append_message(MessageRecord {
                        message,
                        meta: None,
                    });
                }
            }

            let _ = events.send(AgentEvent::TurnStart { turn }).await;

            let model = {
                let session = self.session.lock().await;
                if self.config.model.is_empty() {
                    session
                        .header()
                        .map(|h| h.contract.model.clone())
                        .unwrap_or_default()
                } else {
                    self.config.model.clone()
                }
            };

            // Long-horizon guard: estimate the projection; on overflow,
            // summarize older turns into a compaction entry and retry
            // the same contract. Still over afterwards, or no progress,
            // => fail closed. The lock is taken only for short read /
            // plan / apply phases and is NEVER held across the summarizer
            // network call below.
            //
            // `retrieved_messages` holds proactive retrieval results across
            // compaction iterations — they get prepended to the final request
            // after compaction shrinks the projection.
            let mut retrieved_messages: Vec<vak_llm::Message> = Vec::new();
            enum CompactionNeed {
                None,
                Plan(vak_session::types::CompactionPlan, u64),
                TooShortToCompact(u64),
            }
            let need = {
                let session = self.session.lock().await;
                let policy = &self.config.context_policy;
                let system = self.config.system_prompt.as_str();
                let tool_defs = self.tool_definitions();
                let est =
                    context::estimate_tokens(&session.derive_messages(), Some(system), &tool_defs);
                if est <= policy.trigger_at() {
                    CompactionNeed::None
                } else {
                    match session.plan_compaction(policy.keep_recent) {
                        Some(plan) => CompactionNeed::Plan(plan, est),
                        None => CompactionNeed::TooShortToCompact(est),
                    }
                }
            };
            match need {
                CompactionNeed::TooShortToCompact(est_tokens) => {
                    // Reset-with-handoff rescue (Phase H): one structured
                    // summary replaces the entire projection.
                    if !self.handoff_used && self.config.handoff_reset {
                        self.handoff_used = true;
                        if let Ok(handoff) = self
                            .write_handoff(est_tokens, &prompt_owned, &cancel, &events)
                            .await
                        {
                            let mut session = self.session.lock().await;
                            match session.append_handoff_reset(handoff, est_tokens) {
                                Ok(_) => {
                                    let _ = events
                                        .send(AgentEvent::HandoffReset {
                                            before_tokens: est_tokens,
                                        })
                                        .await;
                                    drop(session);
                                    continue;
                                }
                                Err(e) => {
                                    return TurnOutcome::Failed {
                                        error: LlmError::Context(format!(
                                            "context over budget and handoff write failed: {e}"
                                        )),
                                    };
                                }
                            }
                        }
                    }
                    return TurnOutcome::Failed {
                        error: LlmError::Context(
                            "context over budget but too short to compact".into(),
                        ),
                    };
                }
                CompactionNeed::Plan(plan, tokens_before) => {
                    // Proactive retrieval (from vakyartha simulation):
                    // Before compaction summarizes older turns, retrieve the
                    // most relevant ones and keep them verbatim in the
                    // retained region. Only the non-retrieved older turns get
                    // summarized.
                    let history_budget = self
                        .config
                        .context_policy
                        .input_budget()
                        .saturating_sub(tokens_before);
                    let retrieval_cap = self
                        .config
                        .context_policy
                        .dynamic_retrieval_cap(history_budget);
                    let retrieved = {
                        let session = self.session.lock().await;
                        session.retrieve_relevant_entries(
                            &prompt_owned,
                            retrieval_cap,
                            self.config.context_policy.keep_recent,
                        )
                    };
                    // Store messages for later re-insertion into the final
                    // request after compaction.
                    for (_, msg) in &retrieved {
                        retrieved_messages.push(msg.clone());
                    }
                    if !retrieved.is_empty() {
                        let _ = events
                            .send(AgentEvent::ContextRetrieved {
                                retrieved_count: retrieved.len(),
                                cap: retrieval_cap,
                            })
                            .await;
                    }

                    // Build the summary transcript EXCLUDING retrieved
                    // entries — they will be kept verbatim after compaction.
                    // We match by text content since plan.older carries no
                    // entry IDs.
                    let filtered_older: Vec<vak_llm::Message> = plan
                        .older
                        .iter()
                        .filter(|m| {
                            let text = m.text_content();
                            !retrieved.iter().any(|(_, rm)| rm.text_content() == text)
                        })
                        .cloned()
                        .collect();
                    let transcript = context::render_transcript(&filtered_older);
                    let _ = events
                        .send(AgentEvent::ContextCompacting {
                            estimated_tokens: tokens_before,
                        })
                        .await;
                    let req = context::compaction_request(&model, &transcript);
                    let mut ledger = StepLedger::new(
                        WorkPurpose::Summarize,
                        self.provider.name(),
                        &model,
                        self.config.dispatch_ceiling,
                    );
                    let summary_msg = match self
                        .complete_with_reliability(&req, &cancel, &events, false, &mut ledger)
                        .await
                    {
                        Ok(m) => {
                            let sid = {
                                let mut session = self.session.lock().await;
                                let sid = session
                                    .header()
                                    .map(|h| h.session_id.clone())
                                    .unwrap_or_default();
                                let _ = session.append_receipt(ledger.take_receipt());
                                sid
                            };
                            if let Some(gate) = &self.config.spend_gate {
                                gate.record_settled_with_latency(
                                    self.provider.name(),
                                    &m.model,
                                    &sid,
                                    &m.usage,
                                    ledger.receipt.attempts.iter().map(|a| a.latency_ms).sum(),
                                );
                            }
                            m
                        }
                        Err(LlmError::Aborted { .. }) => {
                            let _ = self
                                .session
                                .lock()
                                .await
                                .append_receipt(ledger.take_receipt());
                            return TurnOutcome::Aborted { partial: None };
                        }
                        Err(e) => {
                            let _ = self
                                .session
                                .lock()
                                .await
                                .append_receipt(ledger.take_receipt());
                            return TurnOutcome::Failed {
                                error: LlmError::Network(format!("compaction call failed: {e}")),
                            };
                        }
                    };
                    let summary = summary_msg.text_content();
                    if summary.trim().is_empty() {
                        return TurnOutcome::Failed {
                            error: LlmError::Network("compaction produced an empty summary".into()),
                        };
                    }

                    {
                        let mut session = self.session.lock().await;
                        if let Err(e) = session.apply_compaction(&plan, summary, tokens_before) {
                            return TurnOutcome::Failed {
                                error: LlmError::Network(format!("compaction write failed: {e}")),
                            };
                        }
                    }

                    let est = {
                        let session = self.session.lock().await;
                        let system = self.config.system_prompt.as_str();
                        let tool_defs = self.tool_definitions();
                        context::estimate_tokens(
                            &session.derive_messages(),
                            Some(system),
                            &tool_defs,
                        )
                    };
                    if est >= tokens_before {
                        return TurnOutcome::Failed {
                            error: LlmError::Context(format!(
                                "compaction made no progress (~{tokens_before} -> ~{est} tokens)"
                            )),
                        };
                    }
                    let _ = events
                        .send(AgentEvent::ContextCompacted {
                            before_tokens: tokens_before,
                            after_tokens: est,
                            summarized_messages: filtered_older.len(),
                            selected_messages: plan.partition.selected_entry_ids.len(),
                            dropped_messages: plan.partition.dropped_entry_ids.len(),
                        })
                        .await;
                    if est > self.config.context_policy.input_budget() {
                        // Reset-with-handoff rescue (Phase H), once per run.
                        if !self.handoff_used && self.config.handoff_reset {
                            self.handoff_used = true;
                            if let Ok(handoff) = self
                                .write_handoff(est, &prompt_owned, &cancel, &events)
                                .await
                            {
                                let mut session = self.session.lock().await;
                                match session.append_handoff_reset(handoff, est) {
                                    Ok(_) => {
                                        let _ = events
                                            .send(AgentEvent::HandoffReset { before_tokens: est })
                                            .await;
                                        drop(session);
                                        continue;
                                    }
                                    Err(e) => {
                                        return TurnOutcome::Failed {
                                            error: LlmError::Context(format!(
                                                "context still over budget and handoff write failed: {e}"
                                            )),
                                        };
                                    }
                                }
                            }
                        }
                        return TurnOutcome::Failed {
                            error: LlmError::Context(format!(
                                "context still over budget after compaction (~{est} > {} tokens)",
                                self.config.context_policy.input_budget()
                            )),
                        };
                    }
                }
                CompactionNeed::None => {}
            }

            // One model step = connect + stream + collect, wrapped with the
            // full reliability machinery (watchdog, retries+backoff,
            // circuit breaker, dispatch ceiling). Every dispatch is recorded
            // into the work receipt, which lands in the ledger on every
            // exit path. User aborts and partial-output aborts are never
            // retried; they propagate for caller handling.
            let mut ledger = StepLedger::new(
                WorkPurpose::Execute,
                self.provider.name(),
                &model,
                self.config.dispatch_ceiling,
            );
            let base_request = {
                let session = self.session.lock().await;
                // Proactive retrieval results: relevant older turns that were
                // dropped during compaction are re-inserted here as verbatim
                // context, prepended before the projected (compacted) messages.
                let mut messages = retrieved_messages.clone();
                messages.extend(session.derive_messages());
                ChatRequest {
                    model,
                    system: Some(self.config.system_prompt.clone()),
                    messages,
                    tools: self.tool_definitions(),
                    max_tokens: self.config.context_policy.max_output as u32,
                    temperature: None,
                }
            };
            let request = base_request.clone();

            let response = {
                // Run-level endurance: a sustained fault window (rate-limit
                // burst, slow/hung upstream, truncating proxy) can outlast
                // one step's retry budget. The ledger has not been touched,
                // so re-attempting the whole turn is exact. Aborts, permanent
                // errors, and ceiling exhaustion still fail/abort immediately.
                let mut run_attempt: u32 = 0;
                let mut backoff_ms = self.config.run_retry_base_backoff_ms.max(1);
                loop {
                    match self
                        .complete_with_reliability(&request, &cancel, &events, true, &mut ledger)
                        .await
                    {
                        Ok(r) => break r,
                        Err(LlmError::Aborted { partial }) => {
                            let _ = self
                                .session
                                .lock()
                                .await
                                .append_receipt(ledger.take_receipt());
                            if let Some(p) = &partial {
                                self.append_assistant(p).await;
                            }
                            return TurnOutcome::Aborted { partial };
                        }
                        Err(e) if ledger.budget.remaining() == 0 => {
                            let _ = self
                                .session
                                .lock()
                                .await
                                .append_receipt(ledger.take_receipt());
                            return TurnOutcome::Failed {
                                error: LlmError::Network(format!(
                                    "dispatch ceiling of {} exhausted for this step; last error: {e}",
                                    self.config.dispatch_ceiling
                                )),
                            };
                        }
                        Err(e)
                            if run_attempt < self.config.run_retry_attempts
                                && is_transient_step_error(&e) =>
                        {
                            run_attempt += 1;
                            let delay = backoff_ms.min(30_000);
                            let reason = format!(
                                "step exhausted ({e}); run-level re-attempt {run_attempt}/{}",
                                self.config.run_retry_attempts
                            );
                            self.record_activity(
                                vak_session::ActivityKind::Retry,
                                vak_session::ActivityStatus::Running,
                                format!("Retry attempt {run_attempt}"),
                                Some(reason.clone()),
                                [
                                    ("attempt".into(), run_attempt.to_string()),
                                    ("delay_ms".into(), delay.to_string()),
                                ]
                                .into(),
                            )
                            .await;
                            let _ = events
                                .send(AgentEvent::RetryScheduled {
                                    attempt: run_attempt,
                                    delay_ms: delay,
                                    reason,
                                })
                                .await;
                            if tokio::select! {
                                _ = cancel.cancelled() => false,
                                _ = tokio::time::sleep(std::time::Duration::from_millis(delay)) => true,
                            } {
                                backoff_ms = backoff_ms.saturating_mul(2);
                                continue;
                            }
                            let _ = self
                                .session
                                .lock()
                                .await
                                .append_receipt(ledger.take_receipt());
                            return TurnOutcome::Aborted { partial: None };
                        }
                        Err(e) => {
                            let _ = self
                                .session
                                .lock()
                                .await
                                .append_receipt(ledger.take_receipt());
                            return TurnOutcome::Failed { error: e };
                        }
                    }
                }
            };

            outcome_turns += 1;

            let usage = response.usage.clone();
            let mut settled_provider_slot: Option<String> = None;
            let settled_session_id = {
                let mut session = self.session.lock().await;
                let sid = session
                    .header()
                    .map(|h| h.session_id.clone())
                    .unwrap_or_default();
                let receipt = ledger.take_receipt();
                let settled_provider = receipt.provider.clone();
                let _ = session.append_receipt(receipt);
                settled_provider_slot.replace(settled_provider);
                sid
            };
            if let Some(gate) = &self.config.spend_gate {
                let provider = settled_provider_slot.as_deref().unwrap_or_default();
                gate.record_settled_with_latency(
                    provider,
                    &response.model,
                    &settled_session_id,
                    &usage,
                    ledger.receipt.attempts.iter().map(|a| a.latency_ms).sum(),
                );
            }
            self.append_assistant(&response).await;
            let _ = events.send(AgentEvent::TurnEnd { usage }).await;

            let calls = extract_tool_calls(&response)
                .into_iter()
                .map(normalize_tool_call)
                .collect::<Vec<_>>();

            if calls.is_empty() {
                if let Some(hooks) = &self.config.hooks {
                    let session_id = self
                        .session
                        .lock()
                        .await
                        .header()
                        .map(|h| h.session_id.clone())
                        .unwrap_or_default();
                    let cwd = self
                        .session
                        .lock()
                        .await
                        .header()
                        .map(|h| h.contract_cwd())
                        .unwrap_or_else(|| ".".into());
                    let stop = vak_hooks::run_hooks_with_recorder(
                        hooks.clone(),
                        vak_hooks::HookEvent::Stop,
                        &session_id,
                        &cwd,
                        None,
                        Some(&response.text_content()),
                        &cancel,
                        self.config.hook_recorder.as_deref(),
                    )
                    .await;
                    if stop.blocked {
                        if turn + 1 >= self.config.max_turns {
                            return TurnOutcome::MaxTurnsReached;
                        }
                        let reason = stop
                            .reason
                            .unwrap_or_else(|| "continue required by hook".into());
                        let _ = events
                            .send(AgentEvent::StopHookContinuation {
                                reason: reason.clone(),
                            })
                            .await;
                        let _ = self.session.lock().await.append_message(MessageRecord {
                            message: Message::user_text(format!(
                                "[stop-hook]: {reason}\nPlease continue."
                            )),
                            meta: None,
                        });
                        turn += 1;
                        continue;
                    }
                }
                if let Some(reason) = self
                    .stop_gate(
                        &prompt_owned,
                        &response,
                        bash_calls_this_run,
                        verification_stale,
                        &mut stop_blocks_left,
                        user_completion_released,
                    )
                    .await
                {
                    if self.guard_continue(reason, &events, turn).await {
                        turn += 1;
                        continue;
                    }
                    return TurnOutcome::MaxTurnsReached;
                }
                if let Some(rejection) = self.goal_gate(&response, &cancel, &events).await {
                    if self.guard_continue(rejection, &events, turn).await {
                        turn += 1;
                        continue;
                    }
                    return TurnOutcome::MaxTurnsReached;
                }
                if let Some(rejection) = self.managed_work_gate(&cancel, &events).await {
                    if self.guard_continue(rejection, &events, turn).await {
                        turn += 1;
                        continue;
                    }
                    return TurnOutcome::MaxTurnsReached;
                }
                return TurnOutcome::Completed { response };
            }

            bash_calls_this_run += calls.iter().filter(|c| c.name == "bash").count() as u32;
            // Regression obligations (Phase H): commands proven GREEN this
            // run must stay green before any completion claim.
            let bash_pairs: Vec<(String, String)> = calls
                .iter()
                .filter(|c| c.name == "bash")
                .filter_map(|c| {
                    c.input
                        .get("command")
                        .and_then(|v| v.as_str())
                        .map(|cmd| (c.id.clone(), cmd.to_string()))
                })
                .collect();
            let mutation_ids: Vec<String> = calls
                .iter()
                .filter(|call| matches!(call.name.as_str(), "edit" | "write"))
                .map(|call| call.id.clone())
                .collect();
            let task_assignments: Vec<(String, String, String)> = calls
                .iter()
                .filter(|call| call.name == "task")
                .filter_map(|call| {
                    Some((
                        call.id.clone(),
                        call.input.get("contract_id")?.as_str()?.to_string(),
                        call.input.get("work_item_id")?.as_str()?.to_string(),
                    ))
                })
                .collect();
            let call_names: HashMap<String, String> = calls
                .iter()
                .map(|c| (c.id.clone(), c.name.clone()))
                .collect();
            let results = self.execute_batch(calls, &cancel, &events).await;
            self.record_subagent_work(&task_assignments, &results).await;
            // Classify unresolved correctable tool failures this turn for the
            // repair budget (see `reconcile_repair_budget`). Computed before
            // `results` is consumed into tool-result blocks below, and keyed
            // by admitted tool name so the controller can resurface the exact
            // schema that was rejected.
            let failed_correctable: Vec<(String, ToolErrorKind)> = results
                .iter()
                .filter_map(|(id, out)| match out {
                    ToolRunOutput::Err(content) => {
                        let kind = ToolErrorKind::classify(content);
                        if kind.is_correctable() {
                            Some((
                                call_names.get(id).cloned().unwrap_or_else(|| id.clone()),
                                kind,
                            ))
                        } else {
                            None
                        }
                    }
                    _ => None,
                })
                .collect();
            for (id, out) in &results {
                if matches!(out, ToolRunOutput::Ok(_)) {
                    if bash_pairs.iter().any(|(bash_id, _)| bash_id == id) {
                        verification_stale = false;
                    } else if mutation_ids.iter().any(|mutation_id| mutation_id == id) {
                        verification_stale = true;
                    }
                    if let Some((_, cmd)) = bash_pairs.iter().find(|(bid, _)| bid == id)
                        && !self.obligations.iter().any(|o| o == cmd)
                    {
                        self.obligations.push(cmd.clone());
                    }
                }
            }
            let blocks = results
                .into_iter()
                .map(|(id, out)| match out {
                    ToolRunOutput::Ok(content) => ContentBlock::tool_result(id, content),
                    ToolRunOutput::Err(content) => ContentBlock::tool_error(id, content),
                })
                .collect();

            if let Err(e) = self
                .session
                .lock()
                .await
                .append_message(MessageRecord {
                    message: Message {
                        role: Role::User,
                        content: blocks,
                    },
                    meta: None,
                })
                .map_err(|e| LlmError::Network(format!("session write failed: {e}")))
            {
                return TurnOutcome::Failed { error: e };
            }

            if let Some(outcome) = reconcile_repair_budget(self, &failed_correctable).await {
                return outcome;
            }

            turn += 1;
        }
    }

    async fn resolve_managed_input(&self, answer: &str) -> Result<(), String> {
        let mut session = self.session.lock().await;
        let Some(projection) = session
            .work_projection()
            .map_err(|error| format!("managed work projection is invalid: {error}"))?
        else {
            return Ok(());
        };
        if projection.status != vak_session::types::WorkContractStatus::AwaitingInput {
            return Ok(());
        }
        let Some(answer) = answer.trim().strip_prefix("answer:").map(str::trim) else {
            return Ok(());
        };
        let Some(assumption) =
            projection.contract.assumptions.iter().find(|assumption| {
                assumption.requires_confirmation && assumption.resolution.is_none()
            })
        else {
            return Ok(());
        };
        if answer.is_empty() {
            return Err(format!(
                "cannot resolve assumption '{}' with an empty answer",
                assumption.assumption_id
            ));
        }
        session
            .append_work(vak_session::types::WorkEvent {
                contract_id: projection.contract.contract_id.clone(),
                revision: projection.contract.revision,
                kind: vak_session::types::WorkEventKind::AssumptionResolved {
                    assumption_id: assumption.assumption_id.clone(),
                    resolution: answer.into(),
                },
            })
            .map_err(|error| format!("managed assumption resolution failed: {error}"))?;
        let Some(updated) = session
            .work_projection()
            .map_err(|error| format!("managed work projection is invalid: {error}"))?
        else {
            return Ok(());
        };
        if updated.status == vak_session::types::WorkContractStatus::AwaitingInput
            && updated
                .contract
                .assumptions
                .iter()
                .filter(|assumption| assumption.requires_confirmation)
                .all(|assumption| assumption.resolution.is_some())
        {
            session
                .append_work(vak_session::types::WorkEvent {
                    contract_id: updated.contract.contract_id,
                    revision: updated.contract.revision,
                    kind: vak_session::types::WorkEventKind::ContractStatusChanged {
                        from: vak_session::types::WorkContractStatus::AwaitingInput,
                        to: vak_session::types::WorkContractStatus::Active,
                        reason: "all required assumptions resolved from chat".into(),
                    },
                })
                .map_err(|error| format!("managed work activation failed: {error}"))?;
        }
        Ok(())
    }

    async fn start_managed_contract(
        &self,
        prompt: &str,
        source_entry_id: String,
        cancel: &CancellationToken,
        events: &mpsc::Sender<AgentEvent>,
    ) -> Result<(), String> {
        if self
            .session
            .lock()
            .await
            .work_projection()
            .map_err(|error| format!("managed work projection is invalid: {error}"))?
            .is_some_and(|work| {
                !matches!(
                    work.status,
                    vak_session::types::WorkContractStatus::Completed
                        | vak_session::types::WorkContractStatus::Failed
                        | vak_session::types::WorkContractStatus::Cancelled
                        | vak_session::types::WorkContractStatus::Unverified
                )
            })
        {
            return Ok(());
        }
        let session_id = self
            .session
            .lock()
            .await
            .header()
            .map(|header| header.session_id.clone())
            .ok_or_else(|| "managed work requires a session header".to_string())?;
        let model = self
            .session
            .lock()
            .await
            .header()
            .map(|header| header.contract.model.clone())
            .unwrap_or_else(|| self.config.model.clone());
        let authoring_request = ChatRequest {
            model: model.clone(),
            system: Some("You author durable work contracts. Return only one strict JSON object with keys objective, constraints, assumptions, criteria, and items. Each item must have item_id, title, instructions, dependencies, owner, required, readonly, path_claims, and criterion_ids. Owner must be one of parent_agent, subagent, flow, tool, or human. Criterion kind must be one of shell, file_exists, file_contains, tool_succeeded, flow_completed, external_receipt, or semantic. Do not include markdown or commentary.".into()),
            messages: vec![Message::user_text(prompt)],
            tools: Vec::new(),
            max_tokens: self.config.context_policy.max_output.min(8_000) as u32,
            temperature: None,
        };
        let mut ledger = StepLedger::new(
            WorkPurpose::Plan,
            self.provider.name(),
            &model,
            self.config.dispatch_ceiling.min(4),
        );
        let response = self
            .complete_with_reliability(&authoring_request, cancel, events, false, &mut ledger)
            .await
            .map_err(|error| format!("managed contract authoring failed: {error}"))?;
        self.session
            .lock()
            .await
            .append_receipt(ledger.take_receipt())
            .map_err(|error| format!("managed contract receipt failed: {error}"))?;
        let authored: AuthoredContract =
            serde_json::from_str(&response.text_content()).map_err(|error| {
                format!("managed contract authoring returned invalid JSON: {error}")
            })?;
        if authored.items.is_empty() || authored.items.len() > self.config.max_work_items {
            return Err(format!(
                "managed contract must contain between one and {} items",
                self.config.max_work_items
            ));
        }
        if authored.objective.trim().is_empty() || authored.objective.chars().count() > 16_000 {
            return Err("managed contract objective is empty or too long".into());
        }
        let contract_id = format!(
            "work-{session_id}-{}",
            chrono::Utc::now().timestamp_millis()
        );
        let contract = vak_session::types::WorkContract {
            contract_id: contract_id.clone(),
            revision: 0,
            source_entry_id,
            objective: authored.objective,
            constraints: authored.constraints,
            assumptions: authored.assumptions,
            criteria: authored.criteria,
            items: authored.items,
        };
        vak_session::validate_contract_for_admission(&contract)
            .map_err(|error| format!("managed contract validation failed: {error}"))?;
        validate_work_paths(&contract)?;
        let mut session = self.session.lock().await;
        session
            .append_work(vak_session::types::WorkEvent {
                contract_id: contract_id.clone(),
                revision: 0,
                kind: vak_session::types::WorkEventKind::ContractCreated { contract },
            })
            .map_err(|error| format!("managed contract write failed: {error}"))?;
        let awaiting_input = session
            .work_projection()
            .ok()
            .flatten()
            .is_some_and(|work| {
                work.contract.assumptions.iter().any(|assumption| {
                    assumption.requires_confirmation && assumption.resolution.is_none()
                })
            });
        session
            .append_work(vak_session::types::WorkEvent {
                contract_id: contract_id.clone(),
                revision: 0,
                kind: vak_session::types::WorkEventKind::ContractStatusChanged {
                    from: vak_session::types::WorkContractStatus::Draft,
                    to: if awaiting_input {
                        vak_session::types::WorkContractStatus::AwaitingInput
                    } else {
                        vak_session::types::WorkContractStatus::Active
                    },
                    reason: if awaiting_input {
                        "required assumptions need confirmation".into()
                    } else {
                        "managed execution admitted".into()
                    },
                },
            })
            .map_err(|error| format!("managed contract activation failed: {error}"))?;
        Ok(())
    }

    async fn emit_work_state(&self, events: &mpsc::Sender<AgentEvent>) {
        let projection = self.session.lock().await.work_projection().ok().flatten();
        if let Some(projection) = projection {
            let _ = events.send(AgentEvent::WorkState { projection }).await;
        }
    }

    async fn managed_work_gate(
        &mut self,
        cancel: &CancellationToken,
        events: &mpsc::Sender<AgentEvent>,
    ) -> Option<String> {
        if !self.config.work_enabled || self.config.work_mode != WorkMode::Managed {
            return None;
        }
        let session = self.session.lock().await;
        let projection = match session.work_projection() {
            Ok(Some(projection)) => projection,
            Ok(None) => return Some("managed run has no durable work contract".into()),
            Err(error) => return Some(format!("managed work projection is invalid: {error}")),
        };
        if matches!(
            projection.status,
            vak_session::types::WorkContractStatus::Completed
                | vak_session::types::WorkContractStatus::Failed
                | vak_session::types::WorkContractStatus::Cancelled
                | vak_session::types::WorkContractStatus::Unverified
        ) {
            return None;
        }
        let criteria = projection.contract.criteria.clone();
        drop(session);
        self.verify_managed_criteria(&criteria, &projection, cancel, events)
            .await;
        let mut session = self.session.lock().await;
        let projection = match session.work_projection() {
            Ok(Some(projection)) => projection,
            Ok(None) => return Some("managed run lost its work contract".into()),
            Err(error) => return Some(format!("managed work projection is invalid: {error}")),
        };
        for item in &projection.contract.items {
            let Some(state) = projection.items.get(&item.item_id) else {
                return Some(format!("managed work item '{}' has no state", item.item_id));
            };
            if state.status == vak_session::types::WorkItemStatus::ReadyForVerification {
                let event = vak_session::types::WorkEvent {
                    contract_id: projection.contract.contract_id.clone(),
                    revision: projection.contract.revision,
                    kind: vak_session::types::WorkEventKind::ItemVerified {
                        item_id: item.item_id.clone(),
                        attempt: state.attempt,
                    },
                };
                if let Err(error) = session.append_work(event) {
                    return Some(format!(
                        "managed verification pending for '{}': {error}",
                        item.item_id
                    ));
                }
            }
        }
        let projection = match session.work_projection() {
            Ok(Some(projection)) => projection,
            Ok(None) => return Some("managed run lost its work contract".into()),
            Err(error) => return Some(format!("managed work projection is invalid: {error}")),
        };
        let incomplete: Vec<&str> = projection
            .contract
            .items
            .iter()
            .filter(|item| item.required)
            .filter_map(|item| {
                let state = projection.items.get(&item.item_id)?;
                (!matches!(state.status, vak_session::types::WorkItemStatus::Succeeded))
                    .then_some(item.item_id.as_str())
            })
            .collect();
        if !incomplete.is_empty() {
            return Some(format!(
                "managed work is not complete; required items pending: {}. Use the work tool and do not claim completion.",
                incomplete.join(", ")
            ));
        }
        let missing_criteria: Vec<&str> = projection
            .contract
            .criteria
            .iter()
            .filter(|criterion| criterion.required)
            .filter_map(|criterion| {
                (!matches!(
                    projection.criteria.get(&criterion.criterion_id),
                    Some(vak_session::types::CriterionResult::Passed { .. })
                ))
                .then_some(criterion.criterion_id.as_str())
            })
            .collect();
        if !missing_criteria.is_empty() {
            return Some(format!(
                "managed work cannot complete; required criteria are not proven: {}",
                missing_criteria.join(", ")
            ));
        }
        if projection.status == vak_session::types::WorkContractStatus::Active {
            let contract_id = projection.contract.contract_id.clone();
            let revision = projection.contract.revision;
            if let Err(error) = session.append_work(vak_session::types::WorkEvent {
                contract_id: contract_id.clone(),
                revision,
                kind: vak_session::types::WorkEventKind::ContractStatusChanged {
                    from: vak_session::types::WorkContractStatus::Active,
                    to: vak_session::types::WorkContractStatus::Verifying,
                    reason: "required work items verified".into(),
                },
            }) {
                return Some(format!("managed verification status failed: {error}"));
            }
            if let Err(error) = session.append_work(vak_session::types::WorkEvent {
                contract_id,
                revision,
                kind: vak_session::types::WorkEventKind::ContractStatusChanged {
                    from: vak_session::types::WorkContractStatus::Verifying,
                    to: vak_session::types::WorkContractStatus::Completed,
                    reason: "all required work items verified".into(),
                },
            }) {
                return Some(format!("managed completion status failed: {error}"));
            }
        }
        drop(session);
        self.emit_work_state(events).await;
        None
    }

    async fn verify_managed_criteria(
        &mut self,
        criteria: &[vak_session::types::WorkCriterion],
        projection: &vak_session::work::WorkProjection,
        cancel: &CancellationToken,
        events: &mpsc::Sender<AgentEvent>,
    ) {
        let cwd = self
            .session
            .lock()
            .await
            .header()
            .map(|header| header.contract_cwd())
            .unwrap_or_else(|| std::env::current_dir().unwrap_or_else(|_| ".".into()));
        for criterion in criteria {
            let result = match &criterion.kind {
                vak_session::types::CriterionKind::Shell { command } => self
                    .run_audit_command(command, cancel)
                    .await
                    .map(|_| vak_session::types::CriterionResult::Passed {
                        evidence: format!("shell:{command}"),
                    })
                    .unwrap_or_else(|reason| vak_session::types::CriterionResult::Failed {
                        reason,
                    }),
                vak_session::types::CriterionKind::FileExists { path } => {
                    let Some(resolved) = workspace_criterion_path(&cwd, path) else {
                        self.record_managed_criterion(
                            criterion,
                            vak_session::types::CriterionResult::Failed {
                                reason: format!("path is outside the workspace: {}", path.display()),
                            },
                        )
                        .await;
                        continue;
                    };
                    if resolved.exists() {
                        vak_session::types::CriterionResult::Passed {
                            evidence: format!("file_exists:{}", path.display()),
                        }
                    } else {
                        vak_session::types::CriterionResult::Failed {
                            reason: format!("file does not exist: {}", path.display()),
                        }
                    }
                }
                vak_session::types::CriterionKind::FileContains { path, pattern } => {
                    let Some(resolved) = workspace_criterion_path(&cwd, path) else {
                        self.record_managed_criterion(
                            criterion,
                            vak_session::types::CriterionResult::Failed {
                                reason: format!("path is outside the workspace: {}", path.display()),
                            },
                        )
                        .await;
                        continue;
                    };
                    match std::fs::read_to_string(&resolved) {
                        Ok(content) if content.contains(pattern) => {
                            vak_session::types::CriterionResult::Passed {
                                evidence: format!("file_contains:{}", path.display()),
                            }
                        }
                        Ok(_) => vak_session::types::CriterionResult::Failed {
                            reason: format!("pattern not found in {}", path.display()),
                        },
                        Err(error) => vak_session::types::CriterionResult::Failed {
                            reason: format!("cannot read {}: {error}", path.display()),
                        },
                    }
                }
                vak_session::types::CriterionKind::Semantic => continue,
                vak_session::types::CriterionKind::ToolSucceeded { tool } => {
                    let mut succeeded = false;
                    for item_id in self.criterion_item_ids(projection, &criterion.criterion_id) {
                        if self
                            .tool_succeeded_for_item(projection, &item_id, tool)
                            .await
                        {
                            succeeded = true;
                            break;
                        }
                    }
                    if succeeded {
                        vak_session::types::CriterionResult::Passed {
                            evidence: format!("tool_succeeded:{tool}"),
                        }
                    } else {
                        vak_session::types::CriterionResult::Unknown {
                            reason: format!("no successful '{tool}' tool result exists yet"),
                        }
                    }
                }
                vak_session::types::CriterionKind::FlowCompleted { flow } => {
                    if self.criterion_item_ids(projection, &criterion.criterion_id).into_iter().any(
                        |item_id| {
                            projection.items.get(&item_id).is_some_and(|item| {
                                item.evidence.iter().any(|evidence| {
                                    matches!(evidence, vak_session::types::EvidenceRef::FlowNode { flow: name, node_id, .. } if name == flow && node_id == "__flow_completed__")
                                })
                            })
                        },
                    ) {
                        vak_session::types::CriterionResult::Passed {
                            evidence: format!("flow_completed:{flow}"),
                        }
                    } else {
                        vak_session::types::CriterionResult::Unknown {
                            reason: format!("flow '{flow}' has no linked completion evidence"),
                        }
                    }
                }
                vak_session::types::CriterionKind::ExternalReceipt { integration } => {
                    if self.criterion_item_ids(projection, &criterion.criterion_id).into_iter().any(
                        |item_id| {
                            projection.items.get(&item_id).is_some_and(|item| {
                                item.evidence.iter().any(|evidence| {
                                    matches!(evidence, vak_session::types::EvidenceRef::ExternalOperation { integration: name, .. } if name == integration)
                                })
                            })
                        },
                    ) {
                        vak_session::types::CriterionResult::Passed {
                            evidence: format!("external_receipt:{integration}"),
                        }
                    } else {
                        vak_session::types::CriterionResult::Unknown {
                            reason: format!("integration '{integration}' has no receipt evidence"),
                        }
                    }
                }
            };
            self.record_managed_criterion(criterion, result).await;
        }
        let semantic: Vec<(vak_session::types::WorkCriterion, String)> = criteria
            .iter()
            .filter_map(|criterion| match &criterion.kind {
                vak_session::types::CriterionKind::Semantic => Some((
                    criterion.clone(),
                    format!("[{}] {}", criterion.criterion_id, criterion.statement),
                )),
                _ => None,
            })
            .collect();
        if semantic.is_empty() || cancel.is_cancelled() {
            return;
        }
        let judge_criteria: Vec<String> = semantic.iter().map(|(_, text)| text.clone()).collect();
        match self.run_judge(&judge_criteria, cancel, events).await {
            Ok(verdicts) => {
                for (criterion, expected) in semantic {
                    let result = verdicts
                        .iter()
                        .find(|verdict| verdict.criterion == expected)
                        .map(|verdict| match verdict.verdict.as_str() {
                            "pass" => vak_session::types::CriterionResult::Passed {
                                evidence: verdict.evidence.clone(),
                            },
                            "fail" => vak_session::types::CriterionResult::Failed {
                                reason: verdict.evidence.clone(),
                            },
                            _ => vak_session::types::CriterionResult::Unknown {
                                reason: verdict.evidence.clone(),
                            },
                        })
                        .unwrap_or_else(|| vak_session::types::CriterionResult::Unknown {
                            reason: "judge returned no verdict for this criterion".into(),
                        });
                    self.record_managed_criterion(&criterion, result).await;
                }
            }
            Err(reason) => {
                for (criterion, _) in semantic {
                    self.record_managed_criterion(
                        &criterion,
                        vak_session::types::CriterionResult::Unknown {
                            reason: reason.clone(),
                        },
                    )
                    .await;
                }
            }
        }
    }

    async fn record_managed_criterion(
        &self,
        criterion: &vak_session::types::WorkCriterion,
        result: vak_session::types::CriterionResult,
    ) {
        let contract_id = self
            .session
            .lock()
            .await
            .work_projection()
            .ok()
            .flatten()
            .map(|projection| {
                (
                    projection.contract.contract_id,
                    projection.contract.revision,
                )
            });
        if let Some((contract_id, revision)) = contract_id {
            let _ = self
                .session
                .lock()
                .await
                .append_work(vak_session::types::WorkEvent {
                    contract_id,
                    revision,
                    kind: vak_session::types::WorkEventKind::VerificationRecorded {
                        criterion_id: criterion.criterion_id.clone(),
                        result,
                    },
                });
        }
    }

    fn criterion_item_ids(
        &self,
        projection: &vak_session::work::WorkProjection,
        criterion_id: &str,
    ) -> Vec<String> {
        projection
            .contract
            .items
            .iter()
            .filter(|item| item.criterion_ids.iter().any(|id| id == criterion_id))
            .map(|item| item.item_id.clone())
            .collect()
    }

    async fn tool_succeeded_for_item(
        &self,
        projection: &vak_session::work::WorkProjection,
        item_id: &str,
        tool: &str,
    ) -> bool {
        let Some(item) = projection.items.get(item_id) else {
            return false;
        };
        let tool_result_ids: std::collections::HashSet<&str> = item
            .evidence
            .iter()
            .filter_map(|evidence| match evidence {
                vak_session::types::EvidenceRef::ToolResult { tool_use_id, .. } => {
                    Some(tool_use_id.as_str())
                }
                _ => None,
            })
            .collect();
        if tool_result_ids.is_empty() {
            return false;
        }
        let session = self.session.lock().await;
        let mut tool_names = std::collections::HashMap::new();
        for entry in session.chain_to_root() {
            if let vak_session::types::EntryPayload::Message(record) = &entry.payload {
                for block in &record.message.content {
                    if let vak_llm::ContentBlock::ToolUse { id, name, .. } = block {
                        tool_names.insert(id.as_str(), name.as_str());
                    }
                }
            }
        }
        session.chain_to_root().iter().any(|entry| {
            let vak_session::types::EntryPayload::Message(record) = &entry.payload else {
                return false;
            };
            record.message.content.iter().any(|block| {
                matches!(
                    block,
                    vak_llm::ContentBlock::ToolResult {
                        tool_use_id,
                        is_error: false,
                        ..
                    } if tool_result_ids.contains(tool_use_id.as_str())
                        && tool_names.get(tool_use_id.as_str()).copied() == Some(tool)
                )
            })
        })
    }

    /// Goal audit gate (Phase H): runs when the model claims completion.
    /// Some(reason) rejects the claim and continues the run; None lets it
    /// end. Completion is recorded from audit, never self-report — and the
    /// audit budget is capped so this can never trap a run.
    async fn goal_gate(
        &mut self,
        _response: &AssistantMessage,
        cancel: &CancellationToken,
        events: &mpsc::Sender<AgentEvent>,
    ) -> Option<String> {
        let goal_active = self.active_goal.is_some();
        if !goal_active {
            return None;
        }

        let mut findings = String::new();

        // 1) Regression obligations: everything proven green must stay so.
        for cmd in self.obligations.clone() {
            if cancel.is_cancelled() {
                return None;
            }
            match self.run_audit_command(&cmd, cancel).await {
                Ok(()) => {}
                Err(err) => {
                    findings.push_str(&format!(
                        "REGRESSION: previously-green command failed now:\n  $ {cmd}\n  {err}\n"
                    ));
                }
            }
        }

        let criteria = self
            .active_goal
            .as_ref()
            .map(|g| g.criteria.clone())
            .unwrap_or_default();

        // 2) Deterministic shell criteria.
        let mut judged_criteria: Vec<String> = Vec::new();
        for criterion in &criteria {
            if goal::is_shell_criterion(criterion) {
                let cmd = goal::shell_command(criterion);
                match self.run_audit_command(cmd, cancel).await {
                    Ok(()) => {}
                    Err(err) => {
                        findings.push_str(&format!("CRITERION FAILED: {criterion}\n  {err}\n"));
                    }
                }
                judged_criteria.push(criterion.clone());
            }
        }

        // 3) Judge call for remaining free-text criteria — only worth a
        // model dispatch when deterministic checks already passed.
        let text_criteria: Vec<String> = criteria
            .iter()
            .filter(|c| !goal::is_shell_criterion(c))
            .cloned()
            .collect();
        if !text_criteria.is_empty() && findings.is_empty() {
            match self.run_judge(&text_criteria, cancel, events).await {
                Ok(verdicts) => {
                    for v in verdicts {
                        if v.verdict != "pass" {
                            findings.push_str(&format!(
                                "JUDGE {}: evidence: {}\n",
                                v.verdict.to_uppercase(),
                                v.evidence
                            ));
                        }
                    }
                }
                Err(e) => {
                    // Fail closed: an unavailable auditor cannot confirm done.
                    findings.push_str(&format!("AUDIT UNAVAILABLE: {e}\n"));
                }
            }
        }

        if findings.is_empty() {
            let mut session = self.session.lock().await;
            let goal_snapshot = self.active_goal.clone();
            let sid = session
                .header()
                .map(|h| h.session_id.clone())
                .unwrap_or_default();
            if let Some(g) = goal_snapshot.as_ref() {
                let _ = session.append_goal(vak_session::types::GoalEntry {
                    goal_id: format!("goal-{sid}"),
                    objective: g.objective.clone(),
                    criteria: g.criteria.clone(),
                    status: vak_session::types::GoalStatus::Done { audited: true },
                });
            }
            drop(session);
            self.active_goal = None;
            return None;
        }

        let audits_left = self
            .active_goal
            .as_mut()
            .map(|g| {
                g.audits_left = g.audits_left.saturating_sub(1);
                g.audits_left
            })
            .unwrap_or(0);
        if audits_left > 0 {
            Some(format!(
                "[goal-audit] not verified yet:\n{findings}\nAddress these and finish again."
            ))
        } else {
            // Budget exhausted: degrade to Unverified rather than trapping.
            let mut session = self.session.lock().await;
            let goal_snapshot = self.active_goal.clone();
            let sid = session
                .header()
                .map(|h| h.session_id.clone())
                .unwrap_or_default();
            if let Some(g) = goal_snapshot.as_ref() {
                let _ = session.append_goal(vak_session::types::GoalEntry {
                    goal_id: format!("goal-{sid}"),
                    objective: g.objective.clone(),
                    criteria: g.criteria.clone(),
                    status: vak_session::types::GoalStatus::Unverified {
                        reason: truncate_chars(&findings, 800),
                    },
                });
            }
            drop(session);
            self.active_goal = None;
            None
        }
    }

    /// Runs one brokered bash command for auditing; Err = failure text.
    async fn run_audit_command(&self, cmd: &str, cancel: &CancellationToken) -> Result<(), String> {
        let tool = self
            .config
            .tools
            .iter()
            .find(|t| t.name() == "bash")
            .ok_or_else(|| "no bash tool available for verification".to_string())?;
        let cwd = self
            .session
            .lock()
            .await
            .header()
            .map(|h| h.contract_cwd())
            .unwrap_or_else(|| std::env::current_dir().unwrap_or_else(|_| ".".into()));
        authorize(
            &self.config,
            &PendingToolCall {
                id: "managed-verification".into(),
                name: "bash".into(),
                input: serde_json::json!({"command": cmd}),
            },
            &cwd,
            &self.run_call_counts,
            &self.config.tools,
        )
        .await?;
        let Some(sandbox) = self
            .config
            .sandbox
            .as_ref()
            .and_then(|sandbox| sandbox.read_only_variant())
        else {
            return Err("shell verification requires a read-only sandbox".into());
        };
        let ctx = vak_tools::ToolContext {
            cwd,
            cancel: cancel.child_token(),
            limits: Default::default(),
            sandbox: Some(sandbox),
            sandbox_sink: None,
        };
        let input = serde_json::json!({ "command": cmd });
        let out = match tokio::time::timeout(
            std::time::Duration::from_secs(120),
            tool.execute(&input, &ctx),
        )
        .await
        {
            Ok(o) => o,
            Err(_) => return Err("timed out after 120s".to_string()),
        };
        if out.is_error {
            let tail: String = out.content.chars().rev().take(400).collect::<String>();
            Err(tail.chars().rev().collect())
        } else {
            Ok(())
        }
    }

    /// One skeptical judge dispatch over the transcript digest.
    async fn run_judge(
        &mut self,
        text_criteria: &[String],
        cancel: &CancellationToken,
        events: &mpsc::Sender<AgentEvent>,
    ) -> Result<Vec<goal::CriterionVerdict>, String> {
        let model = {
            let session = self.session.lock().await;
            session
                .header()
                .map(|h| h.contract.model.clone())
                .unwrap_or_else(|| self.config.model.clone())
        };
        let digest = {
            let session = self.session.lock().await;
            let msgs = session.derive_messages();
            goal::transcript_digest(&msgs, 24_000)
        };
        let objective = self
            .active_goal
            .as_ref()
            .map(|g| g.objective.clone())
            .unwrap_or_default();
        // MEA: environment facts over transcript claims. Unavailable delta
        // is stated as such to the judge (UNKNOWN, never fabricated).
        let workspace_delta = match &self.config.workspace_delta {
            Some(p) => p.summary().ok(),
            None => None,
        };
        let req = goal::audit_request(
            &model,
            goal::audit_prompt(
                &objective,
                text_criteria,
                &digest,
                workspace_delta.as_deref(),
            ),
        );
        let mut ledger = StepLedger::new(
            WorkPurpose::Verify,
            self.provider.name(),
            &model,
            self.config.dispatch_ceiling.min(4),
        );
        let reply = self
            .complete_with_reliability(&req, cancel, events, false, &mut ledger)
            .await
            .map_err(|e| e.to_string())?;
        let _ = self
            .session
            .lock()
            .await
            .append_receipt(ledger.take_receipt());
        goal::parse_verdicts(&reply.text_content())
    }

    /// Reset-with-handoff rescue: one structured summary replaces the whole
    /// projection; returns the handoff markdown.
    async fn write_handoff(
        &self,
        est_tokens: u64,
        original_prompt: &str,
        cancel: &CancellationToken,
        events: &mpsc::Sender<AgentEvent>,
    ) -> Result<String, LlmError> {
        let model = {
            let session = self.session.lock().await;
            session
                .header()
                .map(|h| h.contract.model.clone())
                .unwrap_or_else(|| self.config.model.clone())
        };
        let digest = {
            let session = self.session.lock().await;
            let msgs = session.derive_messages();
            goal::transcript_digest(&msgs, 20_000)
        };
        let objective_line = if self.active_goal.is_some() || !original_prompt.is_empty() {
            format!("Original task: {original_prompt}\n\n")
        } else {
            String::new()
        };
        let req = goal::handoff_request(&model, format!("{objective_line}{digest}"));
        let mut ledger = StepLedger::new(
            WorkPurpose::Summarize,
            self.provider.name(),
            &model,
            self.config.dispatch_ceiling.min(3),
        );
        let _ = events
            .send(AgentEvent::ContextCompacting {
                estimated_tokens: est_tokens,
            })
            .await;
        let reply = self
            .complete_with_reliability(&req, cancel, events, false, &mut ledger)
            .await?;
        let _ = self
            .session
            .lock()
            .await
            .append_receipt(ledger.take_receipt());
        Ok(reply.text_content())
    }

    /// Internal premature-completion gate. Returns a continuation reason
    /// when the stop policy fires and budget remains.
    async fn stop_gate(
        &self,
        prompt: &str,
        response: &AssistantMessage,
        bash_calls_this_run: u32,
        verification_stale: bool,
        blocks_left: &mut u32,
        user_completion_released: bool,
    ) -> Option<String> {
        let policy = self.config.stop_policy.as_ref()?;
        if user_completion_released {
            return None;
        }
        let reason = policy.evaluate_with_state(
            prompt,
            &response.text_content(),
            bash_calls_this_run,
            verification_stale,
        )?;
        if !matches!(reason, BlockReason::UserCompletionRequired) && *blocks_left == 0 {
            return None;
        }
        if matches!(reason, BlockReason::UserCompletionRequired) {
            return Some(reason.message());
        }
        *blocks_left -= 1;
        Some(reason.message())
    }

    /// Appends the continue nudge (model-visible => logged) and reports
    /// whether the loop may continue within max_turns.
    async fn guard_continue(
        &mut self,
        reason: String,
        events: &mpsc::Sender<AgentEvent>,
        turn: usize,
    ) -> bool {
        if turn + 1 >= self.config.max_turns {
            return false;
        }
        let _ = events
            .send(AgentEvent::StopHookContinuation {
                reason: reason.clone(),
            })
            .await;
        let _ = self.session.lock().await.append_message(MessageRecord {
            message: Message::user_text(format!("[stop-guard]: {reason}\nPlease continue.")),
            meta: None,
        });
        true
    }

    /// Recovers the session ledger after a run (server/API consumers).
    #[allow(clippy::panic)]
    pub async fn into_session(self) -> SessionLog {
        let mut config = self.config;
        config.tools.clear();
        config.flow_dispatcher = None;
        drop(config);
        match Arc::try_unwrap(self.session) {
            Ok(session) => session.into_inner(),
            Err(_) => {
                panic!("managed flow dispatcher retained the session after the agent stopped")
            }
        }
    }

    async fn append_assistant(&self, response: &AssistantMessage) {
        let mut session = self.session.lock().await;
        let _ = session.append_message(MessageRecord {
            message: response.clone().into_message(),
            meta: Some(MessageMeta {
                model: Some(response.model.clone()),
                stop_reason: Some(format!("{:?}", response.stop_reason).to_lowercase()),
                usage: Some(response.usage.clone()),
            }),
        });
    }

    async fn record_activity(
        &self,
        kind: vak_session::ActivityKind,
        status: vak_session::ActivityStatus,
        label: String,
        detail: Option<String>,
        data: std::collections::BTreeMap<String, String>,
    ) {
        let now = chrono::Utc::now();
        let activity = vak_session::ActivityRecord {
            activity_id: format!(
                "activity-{}",
                now.timestamp_nanos_opt()
                    .unwrap_or_else(|| now.timestamp_micros() * 1_000)
            ),
            turn: None,
            kind,
            status,
            label,
            detail,
            data,
        };
        let _ = self.session.lock().await.append_activity(activity);
    }

    /// One provider completion with watchdog, retry/backoff (honoring
    /// Retry-After), circuit breaker, dispatch-ceiling enforcement, and
    /// per-attempt receipt recording. When `forward` is true, stream
    /// deltas are forwarded to `events`; otherwise they are drained.
    async fn complete_with_reliability(
        &self,
        request: &ChatRequest,
        cancel: &CancellationToken,
        events: &mpsc::Sender<AgentEvent>,
        forward: bool,
        ledger: &mut StepLedger,
    ) -> Result<AssistantMessage, LlmError> {
        // Frozen-ladder walk (Phase B): `ladder` holds FALLBACK legs;
        // the primary provider/model always walks first. Ceiling,
        // receipt, and endurance budget are shared across ALL legs --
        // walking the ladder is contract execution, never a switch.
        let mut legs: Vec<(Arc<dyn Provider>, String)> =
            Vec::with_capacity(1 + self.config.ladder.len());
        legs.push((self.provider.clone(), request.model.clone()));
        legs.extend(self.config.ladder.iter().cloned());
        let mut leg_req = request.clone();
        let mut last_err: Option<LlmError> = None;

        'legs: for (li, (provider_arc, model)) in legs.iter().enumerate() {
            leg_req.model = model.clone();
            let breaker_key = provider_arc.circuit_key();
            if let Some(breaker) = &self.config.circuit_breaker
                && let Err(open) = breaker.check_key(&breaker_key)
            {
                last_err = Some(LlmError::Network(open.to_string()));
                continue 'legs;
            }
            let route_provider = if li == 0 {
                provider_arc.name().to_string()
            } else {
                self.config
                    .ladder_provider_names
                    .get(li - 1)
                    .cloned()
                    .unwrap_or_else(|| provider_arc.name().to_string())
            };
            ledger.receipt.stamp_leg(&route_provider, model);
            if li > 0 && forward {
                self.record_activity(
                    vak_session::ActivityKind::RouteFallback,
                    vak_session::ActivityStatus::Running,
                    "Route fallback".into(),
                    Some(format!("{}/{}", route_provider, model)),
                    [
                        ("provider".into(), route_provider.clone()),
                        ("model".into(), model.clone()),
                    ]
                    .into(),
                )
                .await;
                let _ = events
                    .send(AgentEvent::RouteFallback {
                        to_provider: route_provider,
                        to_model: model.clone(),
                    })
                    .await;
            }
            let mut attempt: u32 = 0;
            loop {
                if cancel.is_cancelled() {
                    return Err(LlmError::Aborted { partial: None });
                }
                // Budget admission precedes every paid dispatch (Phase D). A
                // denial becomes one bounded budget Ask; refusal -- or no
                // approver, which is the unattended case -- fails the step
                // permanently (never retried, never breaker-tripping).
                if let Some(gate) = &self.config.spend_gate {
                    let session_id = self
                        .session
                        .lock()
                        .await
                        .header()
                        .map(|h| h.session_id.clone())
                        .unwrap_or_default();
                    let est_input = context::estimate_tokens(
                        &leg_req.messages,
                        leg_req.system.as_deref(),
                        &leg_req.tools,
                    );
                    let check = SpendCheck {
                        model,
                        provider: provider_arc.name(),
                        session_id: &session_id,
                        est_input_tokens: est_input,
                        planned_output_tokens: self.config.context_policy.max_output,
                    };
                    if let Err(reason) = gate.authorize(&check).await {
                        let approved = match &self.config.approver {
                            Some(a) => {
                                a.approve(
                                    "finops-budget",
                                    &args_preview(&serde_json::json!({
                                        "model": model,
                                        "reason": reason,
                                    })),
                                    &reason,
                                )
                                .await
                            }
                            None => false,
                        };
                        if !approved {
                            return Err(LlmError::InvalidRequest(format!(
                                "budget admission denied: {reason}"
                            )));
                        }
                        // Raise-cap-once: the rest of THIS run is admitted.
                        gate.on_budget_approved();
                    }
                }
                // Ceiling check happens before every paid dispatch; exhaustion
                // surfaces as a plain error that the endurance loop treats as
                // fail-closed (never transient).
                if let Err(c) = ledger.budget.consume() {
                    return Err(LlmError::Network(c.to_string()));
                }
                let reason = if li > 0 && attempt == 0 {
                    AttemptReason::RouteFallback
                } else if attempt == 0 {
                    AttemptReason::Initial
                } else {
                    AttemptReason::Retry
                };
                let started = std::time::Instant::now();

                let provider_for_stream = provider_arc.clone();
                let req_for_stream = leg_req.clone();
                let step = async move {
                    let mut stream = provider_for_stream
                        .stream(req_for_stream, cancel.clone())
                        .await?;
                    while let Some(ev) = futures::StreamExt::next(&mut stream).await {
                        if forward && events.send(AgentEvent::Stream(ev)).await.is_err() {
                            cancel.cancel();
                        }
                    }
                    stream.result().await
                };
                let step = std::panic::AssertUnwindSafe(step).catch_unwind();

                let mut domain_override: Option<FailureDomain> = None;
                let outcome = match self.config.request_timeout {
                    Some(t) => match tokio::time::timeout(t, step).await {
                        Ok(Ok(r)) => r,
                        Ok(Err(_)) => Err(LlmError::Network(
                            "provider dispatch panicked and was contained".into(),
                        )),
                        Err(_) => {
                            // The provider owns a spawned producer keyed by
                            // this token. A watchdog timeout must revoke the
                            // attempt before the retry/fallback path can
                            // release capacity and dispatch again.
                            cancel.cancel();
                            domain_override = Some(FailureDomain::Deadline);
                            Err(LlmError::Network(format!(
                                "model step exceeded deadline of {}s",
                                t.as_secs()
                            )))
                        }
                    },
                    None => match step.await {
                        Ok(r) => r,
                        Err(_) => Err(LlmError::Network(
                            "provider dispatch panicked and was contained".into(),
                        )),
                    },
                };
                let elapsed_ms = started.elapsed().as_millis() as u64;

                match outcome {
                    Ok(r) => {
                        if let Some(breaker) = &self.config.circuit_breaker {
                            breaker.record_success_key(&breaker_key);
                        }
                        ledger.receipt.record(
                            reason,
                            FailureDomain::Unknown,
                            Settlement::Ok,
                            elapsed_ms,
                            Some(r.usage.clone()),
                            None,
                        );
                        return Ok(r);
                    }
                    Err(e @ LlmError::Aborted { .. }) => {
                        ledger.receipt.record(
                            reason,
                            FailureDomain::Unknown,
                            Settlement::Cancelled,
                            elapsed_ms,
                            None,
                            None,
                        );
                        return Err(e);
                    }
                    Err(e) => {
                        let (domain, settlement) = vak_llm::work::classify_error(&e);
                        ledger.receipt.record(
                            reason,
                            domain_override.unwrap_or(domain),
                            settlement,
                            elapsed_ms,
                            None,
                            Some(e.to_string()),
                        );
                        if e.is_retryable() && attempt < self.config.max_retries {
                            if trips_breaker(&e)
                                && let Some(breaker) = &self.config.circuit_breaker
                            {
                                breaker.record_failure_key(&breaker_key);
                            }
                            attempt += 1;
                            let delay = backoff_delay(
                                attempt,
                                e.retry_after_secs(),
                                self.config.retry_base_backoff_ms,
                            );
                            self.record_activity(
                                vak_session::ActivityKind::Retry,
                                vak_session::ActivityStatus::Running,
                                format!("Retry attempt {attempt}"),
                                Some(e.to_string()),
                                [
                                    ("attempt".into(), attempt.to_string()),
                                    ("delay_ms".into(), delay.as_millis().to_string()),
                                ]
                                .into(),
                            )
                            .await;
                            let _ = events
                                .send(AgentEvent::RetryScheduled {
                                    attempt,
                                    delay_ms: delay.as_millis() as u64,
                                    reason: e.to_string(),
                                })
                                .await;
                            tokio::select! {
                                _ = cancel.cancelled() => {
                                    return Err(LlmError::Aborted { partial: None });
                                }
                                _ = tokio::time::sleep(delay) => {}
                            }
                        } else {
                            // Leg exhausted (retries burned or permanent error):
                            // defer to the next frozen candidate.
                            if trips_breaker(&e)
                                && let Some(breaker) = &self.config.circuit_breaker
                            {
                                breaker.record_failure_key(&breaker_key);
                            }
                            last_err = Some(e);
                            continue 'legs;
                        }
                    }
                }
            }
        }
        Err(last_err.unwrap_or_else(|| LlmError::Network("route ladder exhausted".into())))
    }

    async fn execute_batch(
        &self,
        calls: Vec<PendingToolCall>,
        cancel: &CancellationToken,
        events: &mpsc::Sender<AgentEvent>,
    ) -> Vec<(String, ToolRunOutput)> {
        let aliases = self
            .config
            .mcp_aliases
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .clone();
        let calls = calls
            .into_iter()
            .map(normalize_tool_call)
            .map(|call| normalize_mcp_alias(call, &aliases))
            .collect::<Vec<_>>();
        let n = calls.len();
        let cwd = self
            .session
            .lock()
            .await
            .header()
            .map(|h| h.contract_cwd())
            .unwrap_or_else(|| std::env::current_dir().unwrap_or_else(|_| ".".into()));
        let sandbox = self.config.sandbox.clone();
        let hooks = self.config.hooks.clone();
        let hook_recorder = self.config.hook_recorder.clone();
        let tool_activity_recorder = self.config.tool_activity_recorder.clone();
        let session_id = self
            .session
            .lock()
            .await
            .header()
            .map(|h| h.session_id.clone())
            .unwrap_or_default();
        let skill_names = self
            .session
            .lock()
            .await
            .header()
            .map(|h| {
                h.contract
                    .capabilities
                    .iter()
                    .filter(|capability| {
                        capability.kind == vak_session::types::CapabilityKind::Skill
                    })
                    .map(|capability| capability.name.clone())
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default();

        let mut authz: Vec<Result<(), String>> = Vec::with_capacity(n);
        for call in &calls {
            authz.push(
                authorize(
                    &self.config,
                    call,
                    &cwd,
                    &self.run_call_counts,
                    &self.config.tools,
                )
                .await,
            );
        }
        let ids: Vec<String> = calls.iter().map(|c| c.id.clone()).collect();

        if !self.config.parallel_tools
            || n == 1
            || self.config.work_mode == WorkMode::Managed
            || calls.iter().any(|call| call.name == "work")
        {
            let mut out = Vec::with_capacity(n);
            for (call, verdict) in calls.into_iter().zip(authz) {
                match verdict {
                    Err(reason) => out.push((call.id, ToolRunOutput::Err(reason))),
                    Ok(()) => {
                        if cancel.is_cancelled() {
                            out.push((call.id, ToolRunOutput::Err("cancelled".into())));
                            continue;
                        }
                        let managed_item = (self.config.work_mode == WorkMode::Managed
                            && call.name != "work")
                            .then(|| call.name.clone());
                        let managed_item = match managed_item {
                            Some(tool) => {
                                self.begin_managed_tool_item(
                                    &tool,
                                    (call.name == "task" || call.name == "flow")
                                        .then(|| {
                                            Some((
                                                call.input.get("contract_id")?.as_str()?,
                                                call.input.get("work_item_id")?.as_str()?,
                                            ))
                                        })
                                        .flatten(),
                                    call.input.get("flow").and_then(|value| value.as_str()),
                                )
                                .await
                            }
                            None => None,
                        };
                        if self.config.work_mode == WorkMode::Managed
                            && call.name != "work"
                            && managed_item.is_none()
                        {
                            out.push((
                                call.id,
                                ToolRunOutput::Err(
                                    "managed execution requires a compatible Ready or Running work item"
                                        .into(),
                                ),
                            ));
                            continue;
                        }
                        out.push(if call.name == "work" {
                            let result = self.execute_work_call(&call.input).await;
                            self.emit_work_state(events).await;
                            (call.id.clone(), result)
                        } else {
                            let owns_lifecycle = call.name == "task" || call.name == "flow";
                            let (returned_id, result) = execute_one(
                                call,
                                &self.config.tools,
                                &cwd,
                                &session_id,
                                &skill_names,
                                hooks.as_ref(),
                                hook_recorder.as_ref(),
                                tool_activity_recorder.as_ref(),
                                sandbox.as_ref(),
                                cancel,
                                events,
                            )
                            .await;
                            if !owns_lifecycle
                                && let Some((contract_id, item_id, attempt)) = managed_item
                            {
                                self.finish_managed_tool_item(
                                    &contract_id,
                                    &item_id,
                                    attempt,
                                    &returned_id,
                                    &result,
                                )
                                .await;
                                self.emit_work_state(events).await;
                            }
                            (returned_id, result)
                        });
                    }
                }
            }
            return out;
        }

        // Greedy wave scheduling: claimed calls that conflict are placed in
        // separate waves; unclaimed and read-only calls share wave 0.
        let claims_for = |call: &PendingToolCall| -> vak_tools::ResourceClaims {
            self.config
                .tools
                .iter()
                .find(|t| t.name() == call.name)
                .map(|t| t.claims(&call.input))
                .unwrap_or_default()
        };
        let mut waves: Vec<Vec<usize>> = vec![Vec::new()];
        for (idx, call) in calls.iter().enumerate() {
            let claims = claims_for(call);
            if claims.is_unclaimed() {
                waves[0].push(idx);
                continue;
            }
            let mut placed = false;
            for wave in waves.iter_mut() {
                let compatible = wave.iter().all(|j| {
                    let other = claims_for(&calls[*j]);
                    !other.conflicts(&claims) && !claims.conflicts(&other)
                });
                if compatible {
                    wave.push(idx);
                    placed = true;
                    break;
                }
            }
            if !placed {
                waves.push(vec![idx]);
            }
        }

        let mut ordered: Vec<Option<(String, ToolRunOutput)>> = (0..n).map(|_| None).collect();
        for wave in waves {
            let mut join = tokio::task::JoinSet::new();
            for &idx in &wave {
                if authz[idx].is_err() {
                    continue;
                }
                let call = match calls.get(idx) {
                    Some(c) => c.clone(),
                    None => continue,
                };
                let tools = self.config.tools.clone();
                let cancel = cancel.clone();
                let events = events.clone();
                let cwd = cwd.clone();
                let sandbox = sandbox.clone();
                let session_id = session_id.clone();
                let skill_names = skill_names.clone();
                let hooks = hooks.clone();
                let hook_recorder = hook_recorder.clone();
                let tool_activity_recorder = tool_activity_recorder.clone();
                join.spawn(async move {
                    let r = execute_one(
                        call,
                        &tools,
                        &cwd,
                        &session_id,
                        &skill_names,
                        hooks.as_ref(),
                        hook_recorder.as_ref(),
                        tool_activity_recorder.as_ref(),
                        sandbox.as_ref(),
                        &cancel,
                        &events,
                    )
                    .await;
                    (idx, r)
                });
            }
            while let Some(res) = join.join_next().await {
                if let Ok((idx, pair)) = res {
                    ordered[idx] = Some(pair);
                }
            }
        }
        for (idx, verdict) in authz.into_iter().enumerate() {
            if let Err(reason) = verdict {
                ordered[idx] = Some((ids[idx].clone(), ToolRunOutput::Err(reason)));
            }
        }
        ordered.into_iter().flatten().collect()
    }

    async fn execute_work_call(&self, args: &serde_json::Value) -> ToolRunOutput {
        let operation = args.get("operation").and_then(|value| value.as_str());
        let mut session = self.session.lock().await;
        let projection = match session.work_projection() {
            Ok(Some(projection)) => projection,
            Ok(None) => return ToolRunOutput::Err("no active managed work contract".into()),
            Err(error) => return ToolRunOutput::Err(format!("invalid work ledger: {error}")),
        };
        match operation {
            Some("get") => ToolRunOutput::Ok(
                serde_json::to_string(&projection).unwrap_or_else(|_| "{}".into()),
            ),
            Some("transition") => {
                let Some(item_id) = args.get("item_id").and_then(|value| value.as_str()) else {
                    return ToolRunOutput::Err("work transition requires item_id".into());
                };
                let Some(to) = args.get("to").and_then(|value| value.as_str()) else {
                    return ToolRunOutput::Err("work transition requires to".into());
                };
                let Some(state) = projection.items.get(item_id) else {
                    return ToolRunOutput::Err(format!("unknown work item '{item_id}'"));
                };
                let Some(target) = parse_model_item_status(to) else {
                    return ToolRunOutput::Err(format!("unsupported model transition '{to}'"));
                };
                let event = vak_session::types::WorkEvent {
                    contract_id: projection.contract.contract_id.clone(),
                    revision: projection.contract.revision,
                    kind: vak_session::types::WorkEventKind::ItemStatusChanged {
                        item_id: item_id.into(),
                        from: state.status.clone(),
                        to: target,
                        attempt: state.attempt,
                        reason: args
                            .get("reason")
                            .and_then(|value| value.as_str())
                            .unwrap_or_default()
                            .into(),
                    },
                };
                match session.append_work(event) {
                    Ok(_) => {
                        ToolRunOutput::Ok(format!("work item '{item_id}' transitioned to {to}"))
                    }
                    Err(error) => ToolRunOutput::Err(error.to_string()),
                }
            }
            Some("attach_evidence") => {
                ToolRunOutput::Err(
                    "model evidence attachment is disabled; successful tool results are attached automatically"
                        .into(),
                )
            }
            _ => ToolRunOutput::Err(
                "work operation must be get, transition, or attach_evidence".into(),
            ),
        }
    }

    async fn begin_managed_tool_item(
        &self,
        tool: &str,
        requested: Option<(&str, &str)>,
        flow_name: Option<&str>,
    ) -> Option<(String, String, u32)> {
        let mut session = self.session.lock().await;
        let projection = session.work_projection().ok().flatten()?;
        let item = projection.items.values().find(|state| {
            (state.status == vak_session::types::WorkItemStatus::Ready
                || state.status == vak_session::types::WorkItemStatus::Running)
                && projection
                    .contract
                    .items
                    .iter()
                    .find(|definition| definition.item_id == state.item_id)
                    .is_some_and(|definition| {
                        let dependencies_ready = definition.dependencies.iter().all(|dependency| {
                            matches!(
                                projection.items.get(dependency).map(|item| &item.status),
                                Some(vak_session::types::WorkItemStatus::Succeeded)
                                    | Some(vak_session::types::WorkItemStatus::Skipped)
                            )
                        });
                        dependencies_ready
                            && match requested {
                                Some((contract_id, item_id)) => {
                                    projection.contract.contract_id == contract_id
                                        && state.item_id == item_id
                                        && match (&definition.owner, tool, flow_name) {
                                            (vak_session::types::WorkOwner::Subagent, "task", _) => true,
                                            (
                                                vak_session::types::WorkOwner::Flow { name },
                                                "flow",
                                                Some(requested_flow),
                                            ) => name == requested_flow,
                                            _ => false,
                                        }
                                }
                                None => {
                                    matches!(definition.owner, vak_session::types::WorkOwner::ParentAgent)
                                        || matches!(&definition.owner, vak_session::types::WorkOwner::Tool { name } if name == tool)
                                }
                            }
                    })
        })?;
        let contract_id = projection.contract.contract_id.clone();
        let item_id = item.item_id.clone();
        let attempt = item.attempt.saturating_add(1);
        if item.status == vak_session::types::WorkItemStatus::Ready {
            if requested.is_some() {
                let owner = if tool == "flow" {
                    vak_session::types::WorkOwner::Flow {
                        name: flow_name.unwrap_or_default().into(),
                    }
                } else {
                    vak_session::types::WorkOwner::Subagent
                };
                session
                    .append_work(vak_session::types::WorkEvent {
                        contract_id: contract_id.clone(),
                        revision: projection.contract.revision,
                        kind: vak_session::types::WorkEventKind::ItemAssigned {
                            item_id: item_id.clone(),
                            owner,
                            child_session_id: None,
                        },
                    })
                    .ok()?;
            }
            session
                .append_work(vak_session::types::WorkEvent {
                    contract_id: contract_id.clone(),
                    revision: projection.contract.revision,
                    kind: vak_session::types::WorkEventKind::ItemStatusChanged {
                        item_id: item_id.clone(),
                        from: vak_session::types::WorkItemStatus::Ready,
                        to: vak_session::types::WorkItemStatus::Running,
                        attempt,
                        reason: format!("executing {tool}"),
                    },
                })
                .ok()?;
        }
        Some((contract_id, item_id, attempt))
    }

    async fn finish_managed_tool_item(
        &self,
        contract_id: &str,
        item_id: &str,
        attempt: u32,
        tool_use_id: &str,
        result: &ToolRunOutput,
    ) {
        let mut session = self.session.lock().await;
        let Ok(Some(projection)) = session.work_projection() else {
            return;
        };
        if projection.contract.contract_id != contract_id
            || projection
                .items
                .get(item_id)
                .is_none_or(|state| state.status != vak_session::types::WorkItemStatus::Running)
        {
            return;
        }
        let next = match result {
            ToolRunOutput::Ok(_) => vak_session::types::WorkItemStatus::ReadyForVerification,
            ToolRunOutput::Err(_) => vak_session::types::WorkItemStatus::Failed,
        };
        let session_id = session
            .header()
            .map(|header| header.session_id.clone())
            .unwrap_or_default();
        if matches!(result, ToolRunOutput::Ok(_))
            && session
                .append_work(vak_session::types::WorkEvent {
                    contract_id: contract_id.into(),
                    revision: projection.contract.revision,
                    kind: vak_session::types::WorkEventKind::EvidenceAttached {
                        item_id: item_id.into(),
                        evidence: vak_session::types::EvidenceRef::ToolResult {
                            session_id,
                            tool_use_id: tool_use_id.into(),
                        },
                    },
                })
                .is_err()
        {
            return;
        }
        let _ = session.append_work(vak_session::types::WorkEvent {
            contract_id: contract_id.into(),
            revision: projection.contract.revision,
            kind: vak_session::types::WorkEventKind::ItemStatusChanged {
                item_id: item_id.into(),
                from: vak_session::types::WorkItemStatus::Running,
                to: next,
                attempt,
                reason: "parent tool execution returned".into(),
            },
        });
    }

    async fn record_subagent_work(
        &self,
        assignments: &[(String, String, String)],
        results: &[(String, ToolRunOutput)],
    ) {
        if self.config.work_mode != WorkMode::Managed {
            return;
        }
        let mut session = self.session.lock().await;
        for (call_id, contract_id, item_id) in assignments {
            let Ok(Some(projection)) = session.work_projection() else {
                continue;
            };
            if projection.contract.contract_id != *contract_id {
                continue;
            }
            let Some(state) = projection.items.get(item_id) else {
                continue;
            };
            if state.status != vak_session::types::WorkItemStatus::Running {
                continue;
            }
            let Some((_, output)) = results.iter().find(|(id, _)| id == call_id) else {
                continue;
            };
            let child_id = match output {
                ToolRunOutput::Ok(text) | ToolRunOutput::Err(text) => extract_subagent_id(text),
            };
            let revision = projection.contract.revision;
            let assigned = vak_session::types::WorkEvent {
                contract_id: contract_id.clone(),
                revision,
                kind: vak_session::types::WorkEventKind::ItemAssigned {
                    item_id: item_id.clone(),
                    owner: vak_session::types::WorkOwner::Subagent,
                    child_session_id: child_id.clone(),
                },
            };
            if session.append_work(assigned).is_err() {
                continue;
            }
            let outcome = if matches!(output, ToolRunOutput::Ok(_)) {
                vak_session::types::WorkItemStatus::ReadyForVerification
            } else {
                vak_session::types::WorkItemStatus::Failed
            };
            if let Some(child_id) = child_id
                && matches!(output, ToolRunOutput::Ok(_))
                && session
                    .append_work(vak_session::types::WorkEvent {
                        contract_id: contract_id.clone(),
                        revision,
                        kind: vak_session::types::WorkEventKind::EvidenceAttached {
                            item_id: item_id.clone(),
                            evidence: vak_session::types::EvidenceRef::ChildSession {
                                session_id: child_id,
                            },
                        },
                    })
                    .is_err()
            {
                continue;
            }
            if let Ok(Some(after_running)) = session.work_projection() {
                let _ = session.append_work(vak_session::types::WorkEvent {
                    contract_id: contract_id.clone(),
                    revision,
                    kind: vak_session::types::WorkEventKind::ItemStatusChanged {
                        item_id: item_id.clone(),
                        from: vak_session::types::WorkItemStatus::Running,
                        to: outcome,
                        attempt: after_running.items[item_id].attempt,
                        reason: "subagent returned".into(),
                    },
                });
            }
        }
    }
}

fn normalize_tool_call(mut call: PendingToolCall) -> PendingToolCall {
    let canonical = vak_tools::canonical_tool_name(&call.name);
    if canonical != call.name {
        call.name = canonical.to_string();
    }
    if (call.name == "read" || call.name == "write" || call.name == "edit")
        && call.input.is_object()
        && let Some(obj) = call.input.as_object_mut()
        && !obj.contains_key("path")
        && let Some(file_path) = obj.get("file_path").or_else(|| obj.get("file")).cloned()
    {
        obj.insert("path".into(), file_path);
    }
    if call.name == "bash"
        && call.input.is_object()
        && let Some(obj) = call.input.as_object_mut()
        && !obj.contains_key("command")
        && let Some(cmd) = obj.get("cmd").or_else(|| obj.get("script")).cloned()
    {
        obj.insert("command".into(), cmd);
    }
    call
}

fn normalize_mcp_alias(
    mut call: PendingToolCall,
    aliases: &std::collections::HashMap<String, McpToolAlias>,
) -> PendingToolCall {
    if let Some(alias) = aliases.get(&call.name) {
        call.name = "mcp".into();
        call.input = serde_json::json!({
            "action": "call",
            "server": alias.server,
            "tool": alias.tool,
            "arguments": call.input,
        });
    } else if call.name == "mcp" {
        // Dynamic broker auto-resolution: if the model called `mcp` with `action: "call"`
        // and specified `tool`, but omitted or left `server` empty, resolve `server`
        // dynamically if the tool name uniquely maps to an admitted server in `aliases`.
        if let Some(obj) = call.input.as_object_mut() {
            let is_call = obj.get("action").and_then(|a| a.as_str()) == Some("call");
            let server_missing = obj
                .get("server")
                .is_none_or(|s| s.is_null() || s.as_str() == Some(""));
            if is_call && server_missing {
                if let Some(tool_name) = obj.get("tool").and_then(|t| t.as_str()) {
                    if let Some(alias) = aliases.get(tool_name) {
                        obj.insert(
                            "server".into(),
                            serde_json::Value::String(alias.server.clone()),
                        );
                    }
                }
            }
        }
    }
    call
}

fn extract_subagent_id(text: &str) -> Option<String> {
    let prefix = "subagent '";
    let start = text.find(prefix)? + prefix.len();
    let rest = &text[start..];
    Some(rest.split('\'').next()?.to_string())
}

#[derive(serde::Deserialize)]
struct AuthoredContract {
    objective: String,
    #[serde(default)]
    constraints: Vec<vak_session::types::WorkConstraint>,
    #[serde(default)]
    assumptions: Vec<vak_session::types::WorkAssumption>,
    #[serde(default)]
    criteria: Vec<vak_session::types::WorkCriterion>,
    items: Vec<vak_session::types::WorkItemDefinition>,
}

pub fn validate_work_paths(contract: &vak_session::types::WorkContract) -> Result<(), String> {
    for item in &contract.items {
        for claim in &item.path_claims {
            let path = std::path::Path::new(claim);
            if path.is_absolute()
                || path
                    .components()
                    .any(|component| component == std::path::Component::ParentDir)
            {
                return Err(format!(
                    "work item '{}' claims a path outside the workspace",
                    item.item_id
                ));
            }
        }
    }
    for criterion in &contract.criteria {
        let path = match &criterion.kind {
            vak_session::types::CriterionKind::FileExists { path }
            | vak_session::types::CriterionKind::FileContains { path, .. } => Some(path),
            _ => None,
        };
        if let Some(path) = path
            && (path.is_absolute()
                || path
                    .components()
                    .any(|component| component == std::path::Component::ParentDir))
        {
            return Err(format!(
                "criterion '{}' claims a path outside the workspace",
                criterion.criterion_id
            ));
        }
    }
    Ok(())
}

fn parse_model_item_status(value: &str) -> Option<vak_session::types::WorkItemStatus> {
    match value {
        "running" => Some(vak_session::types::WorkItemStatus::Running),
        "blocked" => Some(vak_session::types::WorkItemStatus::Blocked),
        "ready_for_verification" => Some(vak_session::types::WorkItemStatus::ReadyForVerification),
        _ => None,
    }
}

fn workspace_criterion_path(
    cwd: &std::path::Path,
    path: &std::path::Path,
) -> Option<std::path::PathBuf> {
    let workspace = std::fs::canonicalize(cwd).ok()?;
    let candidate = if path.is_absolute() {
        path.to_path_buf()
    } else {
        cwd.join(path)
    };
    let resolved = std::fs::canonicalize(candidate).ok()?;
    resolved.starts_with(workspace).then_some(resolved)
}
#[allow(clippy::too_many_arguments)]
async fn execute_one(
    call: PendingToolCall,
    tools: &[Arc<dyn Tool>],
    cwd: &std::path::Path,
    session_id: &str,
    skill_names: &[String],
    hooks: Option<&Arc<Vec<vak_hooks::HookDef>>>,
    hook_recorder: Option<&HookRecorder>,
    tool_activity_recorder: Option<&ToolActivityRecorder>,
    sandbox: Option<&Arc<dyn vak_tools::sandbox::Sandbox>>,
    cancel: &CancellationToken,
    events: &mpsc::Sender<AgentEvent>,
) -> (String, ToolRunOutput) {
    let started = std::time::Instant::now();
    let activity_input = call.input.clone();
    let _ = events
        .send(AgentEvent::ToolCallStart {
            id: call.id.clone(),
            name: call.name.clone(),
            args_json: args_preview(&call.input),
        })
        .await;

    if let Some(hooks) = hooks {
        let pre = vak_hooks::run_hooks_with_recorder(
            hooks.clone(),
            vak_hooks::HookEvent::PreToolUse,
            session_id,
            cwd,
            Some((&call.name, &call.input)),
            None,
            cancel,
            hook_recorder.map(|recorder| &**recorder),
        )
        .await;
        if pre.blocked {
            let reason = pre.reason.unwrap_or_else(|| "blocked by hook".into());
            let content = format!("blocked by hook: {reason}");
            let _ = events
                .send(AgentEvent::ToolCallEnd {
                    id: call.id.clone(),
                    name: call.name.clone(),
                    is_error: true,
                    result_preview: result_preview(&content),
                })
                .await;
            return (call.id, ToolRunOutput::Err(content));
        }
    }

    let tool = tools.iter().find(|t| t.name() == call.name);
    let hook_name = call.name.clone();
    let hook_input = call.input.clone();

    let matched_skill = skill_names.iter().find(|name| {
        name.as_str() == call.name
            || name.replace('-', "_") == call.name
            || name.as_str() == call.name.replace('_', "-")
    });

    let mut output = match tool {
        None => {
            if let Some(actual_skill) = matched_skill {
                ToolRunOutput::Err(format!(
                    r#"{{"type":"capability_kind_mismatch","name":{},"actual_kind":"skill","invocation":{{"tool":"skill","arguments":{{"name":{}}}}}}}"#,
                    serde_json::to_string(&call.name).unwrap_or_else(|_| "\"invalid\"".into()),
                    serde_json::to_string(actual_skill).unwrap_or_else(|_| "\"invalid\"".into())
                ))
            } else {
                ToolRunOutput::Err(format!(
                    r#"{{"type":"unknown_capability","requested_kind":"tool","name":{},"available_tools":{}}}"#,
                    serde_json::to_string(&call.name).unwrap_or_else(|_| "\"invalid\"".into()),
                    serde_json::to_string(&tools.iter().map(|t| t.name()).collect::<Vec<_>>())
                        .unwrap_or_else(|_| "[]".into())
                ))
            }
        }
        Some(tool) => {
            let (sandbox_sink, mut sandbox_rx) =
                vak_tools::SandboxEventSink::new_with_id(call.id.clone());
            // Agent executions are always quarantined.  The Workbench contract
            // promises that intermediate files stay under `.vak/scratch/`;
            // leaving this opt-in made the normal production path run in the
            // workspace root while only tests exercised containment.
            let sandbox_sink = sandbox_sink
                .with_owner_session(session_id.to_string())
                .with_quarantine(true);
            let events_tx = events.clone();
            let forwarder = tokio::spawn(async move {
                while let Some(sb_ev) = sandbox_rx.recv().await {
                    let _ = events_tx.send(AgentEvent::Sandbox(sb_ev)).await;
                }
            });

            let ctx = vak_tools::ToolContext {
                cwd: cwd.to_path_buf(),
                cancel: cancel.child_token(),
                limits: Default::default(),
                sandbox: sandbox.cloned(),
                sandbox_sink: Some(sandbox_sink),
            };
            let result_ctx = ctx.clone();
            let tool = tool.clone();
            let res = tokio::spawn(async move { tool.execute(&call.input, &ctx).await }).await;
            // Close the event sender before joining the forwarder. Keeping a
            // cloned ToolContext alive while awaiting the receiver makes the
            // channel wait on itself forever, which hides cancellation and
            // leaves partial-output runs stuck.
            let output = match res {
                Ok(out) if out.is_error => {
                    ToolRunOutput::Err(result_ctx.truncate_output(out.content))
                }
                Ok(out) => ToolRunOutput::Ok(result_ctx.truncate_output(out.content)),
                Err(join_err) => ToolRunOutput::Err(format!("tool task failed: {join_err}")),
            };
            drop(result_ctx);
            let _ = forwarder.await;
            output
        }
    };

    if let Some(hooks) = hooks {
        let post_reason = match &output {
            ToolRunOutput::Ok(content) => content.as_str(),
            ToolRunOutput::Err(content) => content.as_str(),
        };
        let post = vak_hooks::run_hooks_with_recorder(
            hooks.clone(),
            vak_hooks::HookEvent::PostToolUse,
            session_id,
            cwd,
            Some((&hook_name, &hook_input)),
            Some(post_reason),
            cancel,
            hook_recorder.map(|recorder| &**recorder),
        )
        .await;
        if post.blocked && matches!(output, ToolRunOutput::Ok(_)) {
            let reason = post.reason.unwrap_or_else(|| "flagged by hook".into());
            output = ToolRunOutput::Err(format!("{}\n[post-tool-use hook]: {reason}", post_reason));
        } else if post.blocked {
            let reason = post.reason.unwrap_or_else(|| "flagged by hook".into());
            output = ToolRunOutput::Err(format!("{post_reason}\n[post-tool-use hook]: {reason}"));
        }
    }

    // Tool failures are values, but a terse error alone makes weaker models
    // stop instead of repairing the call. Keep the original error intact and
    // attach a bounded, non-authorizing recovery contract. The next model
    // turn is the retry loop; permission, cancellation, and policy failures
    // deliberately do not receive a retry suggestion.
    if let ToolRunOutput::Err(content) = &mut output
        && let Some(hint) = tool_recovery_hint(content)
    {
        content.push_str(hint);
    }

    let result_content = match &output {
        ToolRunOutput::Ok(content) | ToolRunOutput::Err(content) => content.as_str(),
    };
    let _ = events
        .send(AgentEvent::ToolCallEnd {
            id: call.id.clone(),
            name: call.name.clone(),
            is_error: matches!(output, ToolRunOutput::Err(_)),
            result_preview: result_preview(result_content),
        })
        .await;
    if let Some(recorder) = tool_activity_recorder {
        recorder(
            &call.name,
            &activity_input,
            matches!(output, ToolRunOutput::Ok(_)),
            started.elapsed().as_millis() as u64,
        );
    }

    (call.id, output)
}

/// Returns the model recovery contract for a correctable tool error. The
/// classification that decides this is owned by `vak_tools::ToolErrorKind` —
/// this function is intentionally a thin shim so the hint and the repair
/// budget in the turn loop can never drift onto a different keyword list.
fn tool_recovery_hint(error: &str) -> Option<&'static str> {
    if ToolErrorKind::classify(error).is_correctable() {
        Some(
            "\n[recovery] Treat this as a failed attempt. Inspect the error and the admitted tool/schema inventory, then make at most one corrected or alternative call. Do not repeat identical arguments. If the failure is environmental or the corrected call is unsafe, explain the blocker instead.",
        )
    } else {
        None
    }
}

/// Reconcile the run repair budget against this turn's correctable tool
/// failures. Returns `Some(TurnOutcome)` only when the budget is exhausted
/// and the loop must stop repairing; otherwise `None` (continue to the next
/// model dispatch). This is the "loop where these don't happen" for tool
/// failures of any class: the recovery hint is the first nudge; a
/// system-authored, schema-resurfacing directive is the second; a bounded
/// degraded stop is the third.
async fn reconcile_repair_budget(
    agent: &mut Agent,
    failed_correctable: &[(String, ToolErrorKind)],
) -> Option<TurnOutcome> {
    if failed_correctable.is_empty() {
        agent.repair.consecutive_failed_turns = 0;
        return None;
    }
    if agent.repair.exhausted {
        return None;
    }
    agent.repair.consecutive_failed_turns += 1;

    if agent.repair.consecutive_failed_turns == REPAIR_DIRECTIVE_TURN {
        // The model has now failed to self-repair the same fault across two
        // turns: a text hint is no longer enough. The loop takes over and
        // resurfaces the exact admitted schema for the rejected tools so the
        // repair is no longer a guess.
        inject_repair_directive(agent, failed_correctable).await;
    }

    if agent.repair.consecutive_failed_turns > MAX_REPAIR_TURNS {
        agent.repair.exhausted = true;
        let outcome = degraded_outcome(agent, failed_correctable).await;
        return Some(outcome);
    }
    None
}

/// Append an authoritative repair directive that resurfaces the admitted
/// schema for each tool the model could not get right. Unlike the per-call
/// `[recovery]` hint, this is issued by the loop itself (not the model)
/// when the model has demonstrated it cannot repair the failure unprompted.
async fn inject_repair_directive(agent: &Agent, failed: &[(String, ToolErrorKind)]) {
    let remaining = MAX_REPAIR_TURNS.saturating_sub(agent.repair.consecutive_failed_turns - 1);
    let mut parts: Vec<String> = vec![format!(
        "[repair directive] The run is stuck on correctable tool failures that          were not repaired across turns. Do not repeat the failing call shape;          re-issue with the exact arguments this tool requires. The run will          stop retrying after {} more failed repair turn(s).",
        remaining.max(1)
    )];
    let mut seen = std::collections::HashSet::new();
    for (name, _kind) in failed {
        if !seen.insert(name.clone()) {
            continue;
        }
        let mut found = false;
        for tool in &agent.config.tools {
            if tool.name() != *name {
                continue;
            }
            found = true;
            let schema_str =
                serde_json::to_string(&tool.schema()).unwrap_or_else(|_| "{}".to_string());
            parts.push(format!(
                "\nAdmitted tool `{}` (re-surfaced verbatim):\ndescription: {}\ninput_schema: {}",
                tool.name(),
                tool.description(),
                schema_str
            ));
            break;
        }
        if !found {
            // Unknown tool name: list what IS admitted so the model can map
            // the rejected call onto an admitted one (e.g. use the `mcp`
            // broker instead of a raw capability name).
            let admitted: Vec<String> = agent
                .config
                .tools
                .iter()
                .map(|t| t.name().to_string())
                .collect();
            parts.push(format!(
                "\nTool `{}` is not in the admitted set for this turn.                  Admitted tools: {}. Re-issue using an admitted tool.",
                name,
                admitted.join(", ")
            ));
        }
    }
    let _ = agent.session.lock().await.append_message(MessageRecord {
        message: Message {
            role: Role::User,
            content: vec![ContentBlock::text(parts.join("\n\n"))],
        },
        meta: None,
    });
}

/// Build the degraded, honest completion returned when the repair budget is
/// exhausted: record a diagnostic (append-only, model-visible) and return a
/// system-authored answer that states the failure instead of inventing one.
async fn degraded_outcome(agent: &Agent, failed: &[(String, ToolErrorKind)]) -> TurnOutcome {
    let summary = failed
        .iter()
        .map(|(name, kind)| format!("- `{name}`: correctable fault ({kind:?})"))
        .collect::<Vec<_>>()
        .join("\n");
    agent
        .record_activity(
            vak_session::ActivityKind::Diagnostic,
            vak_session::ActivityStatus::Failed,
            "Tool repair exhausted".into(),
            Some(format!(
                "correctable tool failures were not repaired within the run                  repair budget ({} repair turns); run stopped rather than                  signing a false complete",
                MAX_REPAIR_TURNS
            )),
            std::collections::BTreeMap::from([
                ("repair_turns".into(), agent.repair.consecutive_failed_turns.to_string()),
                (
                    "failed_tools".into(),
                    failed.iter().map(|(n, _)| n.as_str()).collect::<Vec<_>>().join(","),
                ),
            ]),
        )
        .await;
    let response = AssistantMessage {
        content: vec![ContentBlock::text(format!(
            "I attempted the requested work, but the supporting tool calls              failed and could not be repaired within the run's recovery              budget. I will not sign off a fabricated answer. What failed:
             {summary}

To continue, either correct the inputs above and              re-run, or widen the workspace capabilities / permissions if the              failure is an admission gate."
        ))],
        stop_reason: StopReason::EndTurn,
        usage: Usage::default(),
        model: agent.config.model.clone(),
    };
    TurnOutcome::Completed { response }
}

/// Doom-loop threshold: the Nth identical (tool, args) call in one run is
/// re-routed through approval instead of silently repeating.
const DOOM_LOOP_THRESHOLD: u32 = 3;
const MAX_TOOL_INPUT_CHARS: usize = 32_000;

/// After this many **consecutive** turns that end with an unresolved
/// correctable tool failure (`RepairState::consecutive_failed_turns` exceeds
/// it), the run stops re-dispatching and degrades the outcome instead of
/// spinning on failing tool calls. This bounds model-guided repair so a weak
/// model that ignores the recovery hint cannot burn the whole turn budget
/// on the same fault class.
const MAX_REPAIR_TURNS: u32 = 2;
/// On the Nth consecutive correctable-failure turn the system stops relying
/// on a text hint alone: it injects an authoritative, schema-resurfacing
/// directive so the model is no longer guessing what shape was rejected.
const REPAIR_DIRECTIVE_TURN: u32 = 2;

/// Per-run account of model-guided tool recovery. The loop is: hint on the
/// first failure, a system-authored directive on the second, and a bounded
/// degraded stop on the third — instead of unlimited spin or a silent
/// false "complete". Reset at the start of every `run`.
#[derive(Debug, Default)]
struct RepairState {
    /// Consecutive turns ending with one or more unresolved correctable tool
    /// failures. Resets to 0 when a turn produces no correctable failures.
    consecutive_failed_turns: u32,
    /// Set once the run repair budget is exhausted; the loop must not keep
    /// dispatching for repair after this.
    exhausted: bool,
}

impl RepairState {
    fn reset(&mut self) {
        *self = Self::default();
    }
}

async fn authorize(
    config: &AgentConfig,
    call: &PendingToolCall,
    cwd: &std::path::Path,
    run_call_counts: &std::sync::Mutex<HashMap<String, u32>>,
    tools: &[Arc<dyn Tool>],
) -> Result<(), String> {
    let input_chars = serde_json::to_string(&call.input)
        .map(|input| input.chars().count())
        .unwrap_or(MAX_TOOL_INPUT_CHARS.saturating_add(1));
    if input_chars > MAX_TOOL_INPUT_CHARS {
        return Err(format!(
            "tool call arguments exceed the {}-character safety limit; reduce the arguments and retry",
            MAX_TOOL_INPUT_CHARS
        ));
    }
    if let Some(tool) = tools.iter().find(|tool| tool.name() == call.name) {
        vak_tools::validate_input(&tool.schema(), &call.input)?;
    }
    if config
        .revocation_check
        .as_ref()
        .is_some_and(|check| check(&call.name, &call.input))
    {
        return Err(format!(
            "capability `{}` was revoked during this turn",
            call.name
        ));
    }
    let key = format!(
        "{}\u{0}{}",
        call.name,
        serde_json::to_string(&call.input).unwrap_or_default()
    );
    let n = {
        let mut counts = run_call_counts
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let entry = counts.entry(key).or_insert(0);
        *entry += 1;
        *entry
    };
    let decision = if n >= DOOM_LOOP_THRESHOLD {
        Decision::Ask {
            reason: format!("identical {} call repeated ×{n} this run", call.name),
            source: AskSource::CircuitBreaker,
        }
    } else if let Some(engine) = &config.permission {
        engine.evaluate(&call.name, &call.input, config.mode, cwd)
    } else {
        Decision::Allow
    };
    match decision {
        Decision::Allow => Ok(()),
        Decision::Deny { reason } => Err(reason),
        Decision::Ask { reason, source } => {
            if auto_approve(
                config.approval_mode,
                source,
                &call.name,
                &call.input,
                config.mode,
                config.sandbox.is_some(),
                cwd,
            ) {
                if config
                    .revocation_check
                    .as_ref()
                    .is_some_and(|check| check(&call.name, &call.input))
                {
                    return Err(format!(
                        "capability `{}` was revoked during this turn",
                        call.name
                    ));
                }
                return Ok(());
            }
            match &config.approver {
                Some(a)
                    if a.approve(&call.name, &args_preview(&call.input), &reason)
                        .await =>
                {
                    if config
                        .revocation_check
                        .as_ref()
                        .is_some_and(|check| check(&call.name, &call.input))
                    {
                        Err(format!(
                            "capability `{}` was revoked while approval was pending",
                            call.name
                        ))
                    } else {
                        Ok(())
                    }
                }
                // Say who actually refused. On an unattended surface the
                // gate was never put to a person, and reporting it as
                // "denied by user" sent the model looking for a different
                // tool — and the operator looking for a user who had done
                // nothing — instead of naming the policy that decided.
                Some(a) if !a.answerable() => Err(format!(
                    "unattended surface: {reason}. No approver is configured to \
                     answer it, so this capability cannot be used on this turn"
                )),
                Some(_) => Err(format!("denied by user: {reason}")),
                None => Err(format!("{reason} (no approver available)")),
            }
        }
    }
}

enum ToolRunOutput {
    Ok(String),
    Err(String),
}

fn extract_tool_calls(response: &AssistantMessage) -> Vec<PendingToolCall> {
    let calls: Vec<PendingToolCall> = response
        .content
        .iter()
        .filter_map(|b| match b {
            ContentBlock::ToolUse { id, name, input } => Some(PendingToolCall {
                id: id.clone(),
                name: name.clone(),
                input: input.clone(),
            }),
            _ => None,
        })
        .collect();

    if !calls.is_empty() {
        return calls;
    }

    let text = response.text_content();
    if text.trim().is_empty() {
        return calls;
    }

    parse_text_tool_calls(&text)
}

fn parse_text_tool_calls(text: &str) -> Vec<PendingToolCall> {
    let mut calls = Vec::new();

    // 1. Structured JSON blocks: ```tool_call ... ``` or <tool_call> ... </tool_call>
    for block in extract_tool_call_blocks(text) {
        if let Ok(val) = serde_json::from_str::<serde_json::Value>(&block) {
            if let Some(call) = json_to_tool_call(&val) {
                calls.push(call);
            }
        }
    }
    if !calls.is_empty() {
        return calls;
    }

    // 2. Functional write calls: write(path="...", content="...")
    let mut cursor = 0;
    while cursor < text.len() {
        let sub = &text[cursor..];
        let found = sub
            .find("write(")
            .map(|i| (i, "write(".len()))
            .or_else(|| sub.find("write ").map(|i| (i, "write ".len())));
        let Some((rel_idx, offset)) = found else {
            break;
        };
        let call_start = cursor + rel_idx;
        let call_sub = &text[call_start + offset..];

        let next_delim = [
            call_sub.find("\nwrite("),
            call_sub.find("\nwrite "),
            call_sub.find("\nbash("),
            call_sub.find("\nbash "),
            call_sub.find("\n```"),
        ]
        .into_iter()
        .flatten()
        .min()
        .unwrap_or(call_sub.len());
        let this_call_text = &call_sub[..next_delim];

        if let Some(path) = extract_named_param(this_call_text, "path=")
            .or_else(|| extract_named_param(this_call_text, "file="))
            && let Some(content) = extract_content_param(this_call_text)
        {
            calls.push(PendingToolCall {
                id: format!("call_txt_{:08x}", rand_jitter(u64::MAX)),
                name: "write".into(),
                input: serde_json::json!({"path": path, "content": content}),
            });
            cursor = call_start + offset + next_delim;
        } else if let Some(call) =
            parse_write_call_from_text(&text[call_start..call_start + offset + next_delim])
        {
            calls.push(call);
            cursor = call_start + offset + next_delim;
        } else {
            cursor = call_start + offset;
        }
    }

    // 3. Command execution: bash -c "..." or bash("...")
    let mut cursor = 0;
    while cursor < text.len() {
        let sub = &text[cursor..];
        if let Some(call) = parse_bash_call_from_text(sub) {
            let cmd_str = call
                .input
                .get("command")
                .and_then(|c| c.as_str())
                .unwrap_or("")
                .to_string();
            calls.push(call);
            if !cmd_str.is_empty()
                && let Some(idx) = sub.find(&cmd_str)
            {
                cursor += idx + cmd_str.len();
            } else {
                cursor += 10;
            }
        } else {
            break;
        }
    }

    calls
}

fn extract_tool_call_blocks(text: &str) -> Vec<String> {
    let mut blocks = Vec::new();
    for tag in ["<tool_call>", "```tool_call", "```tool_use", "```json"] {
        let mut cursor = 0;
        while let Some(start_idx) = text[cursor..].find(tag) {
            let abs_start = cursor + start_idx + tag.len();
            let close_tag = if tag.starts_with('<') {
                "</tool_call>"
            } else {
                "```"
            };
            if let Some(end_idx) = text[abs_start..].find(close_tag) {
                let block = text[abs_start..abs_start + end_idx].trim().to_string();
                if block.contains("\"name\"") || block.contains("\"tool\"") {
                    blocks.push(block);
                }
                cursor = abs_start + end_idx + close_tag.len();
            } else {
                break;
            }
        }
    }
    blocks
}

fn json_to_tool_call(val: &serde_json::Value) -> Option<PendingToolCall> {
    let name = val
        .get("name")
        .or_else(|| val.get("tool"))
        .and_then(|v| v.as_str())?;
    let input = val
        .get("arguments")
        .or_else(|| val.get("input"))
        .or_else(|| val.get("parameters"))
        .cloned()
        .unwrap_or(serde_json::json!({}));
    Some(PendingToolCall {
        id: format!("call_txt_{:08x}", rand_jitter(u64::MAX)),
        name: name.to_string(),
        input,
    })
}

fn parse_write_call_from_text(text: &str) -> Option<PendingToolCall> {
    let (write_idx, offset) = if let Some(idx) = text.find("write(") {
        (idx, "write(".len())
    } else if let Some(idx) = text.find("write ") {
        (idx, "write ".len())
    } else {
        return None;
    };
    let sub = text[write_idx + offset..].trim_start();

    // 1. Try explicit named parameters
    let path = extract_named_param(sub, "path=").or_else(|| extract_named_param(sub, "file="));
    let content = extract_content_param(sub);

    if let (Some(p), Some(c)) = (path, content) {
        return Some(PendingToolCall {
            id: format!("call_txt_{:08x}", rand_jitter(u64::MAX)),
            name: "write".into(),
            input: serde_json::json!({"path": p, "content": c}),
        });
    }

    // 2. Try positional or command-style: write [path] [content]
    for quote_str in ["\"\"\"", "'''", "\"", "'"] {
        let (p, content_sub) = if sub.starts_with('"') || sub.starts_with('\'') {
            let Some(q) = sub.chars().next() else {
                continue;
            };
            let rest = &sub[q.len_utf8()..];
            let Some(end_p) = rest.find(q) else {
                continue;
            };
            let p = rest[..end_p].to_string();
            let after = rest[end_p + q.len_utf8()..].trim_start();
            let after = if after.starts_with(',') {
                after[1..].trim_start()
            } else {
                after
            };
            (p, after)
        } else {
            let Some(q_idx) = sub.find(quote_str) else {
                continue;
            };
            let prefix = sub[..q_idx].trim();
            let Some(p) = prefix
                .split(|c: char| c.is_whitespace() || c == '=' || c == ',' || c == '\\' || c == '(')
                .find(|token| !token.is_empty() && (token.contains('/') || token.contains('.')))
            else {
                continue;
            };
            let p = p.trim_matches('"').trim_matches('\'').to_string();
            (p, &sub[q_idx..])
        };

        if let Some(c) =
            extract_content_param(content_sub).or_else(|| extract_raw_content(content_sub))
        {
            if !p.is_empty() && !c.is_empty() {
                return Some(PendingToolCall {
                    id: format!("call_txt_{:08x}", rand_jitter(u64::MAX)),
                    name: "write".into(),
                    input: serde_json::json!({"path": p, "content": c}),
                });
            }
        }
    }
    None
}

fn extract_raw_content(sub: &str) -> Option<String> {
    let sub = sub.trim_start();
    for triple in ["\"\"\"", "'''"] {
        if sub.starts_with(triple) {
            let rest = &sub[triple.len()..];
            let end = rest.find(triple)?;
            return Some(rest[..end].trim().to_string());
        }
    }
    if sub.starts_with('"') || sub.starts_with('\'') {
        let quote = sub.chars().next()?;
        let rest = &sub[quote.len_utf8()..];
        let mut end_idx = None;
        let mut escaped = false;
        for (i, ch) in rest.char_indices() {
            if escaped {
                escaped = false;
                continue;
            }
            if ch == '\\' {
                escaped = true;
                continue;
            }
            if ch == quote {
                let rem = rest[i + 1..].trim_start();
                if rem.starts_with(')') || rem.starts_with("```") || rem.is_empty() {
                    end_idx = Some(i);
                    break;
                }
            }
        }
        if let Some(end) = end_idx {
            return Some(unescape_string(&rest[..end]));
        }
    }
    None
}

fn extract_named_param(sub: &str, prefix: &str) -> Option<String> {
    let idx = sub.find(prefix)?;
    let after = &sub[idx + prefix.len()..];
    let quote = after.chars().next()?;
    if quote != '"' && quote != '\'' {
        return None;
    }
    let val_start = &after[quote.len_utf8()..];
    let end_idx = val_start.find(quote)?;
    Some(val_start[..end_idx].to_string())
}

fn extract_content_param(sub: &str) -> Option<String> {
    let idx = sub.find("content=")?;
    let after = sub[idx + "content=".len()..].trim_start();
    for triple in ["\"\"\"", "'''"] {
        if after.starts_with(triple) {
            let rest = &after[triple.len()..];
            let end = rest.find(triple)?;
            return Some(rest[..end].to_string());
        }
    }
    let quote = after.chars().next()?;
    if quote != '"' && quote != '\'' {
        return None;
    }
    let rest = &after[quote.len_utf8()..];
    let mut end_idx = None;
    let mut escaped = false;
    for (i, ch) in rest.char_indices() {
        if escaped {
            escaped = false;
            continue;
        }
        if ch == '\\' {
            escaped = true;
            continue;
        }
        if ch == quote {
            let rem = rest[i + 1..].trim_start();
            if rem.starts_with(')')
                || rem.starts_with("-->")
                || rem.starts_with("```")
                || rem.starts_with("\nbash")
                || rem.starts_with("\nwrite")
                || rem.is_empty()
            {
                end_idx = Some(i);
                break;
            }
        }
    }
    if let Some(end) = end_idx {
        let raw = &rest[..end];
        return Some(unescape_string(raw));
    }
    None
}

fn unescape_string(raw: &str) -> String {
    if !raw.contains('\\') {
        return raw.to_string();
    }
    if let Ok(val) = serde_json::from_str::<String>(&format!("\"{raw}\"")) {
        return val;
    }
    raw.replace("\\n", "\n")
        .replace("\\t", "\t")
        .replace("\\\"", "\"")
        .replace("\\'", "'")
        .replace("\\\\", "\\")
}

fn parse_bash_call_from_text(text: &str) -> Option<PendingToolCall> {
    for needle in ["bash -c \"", "bash -c '"] {
        if let Some(idx) = text.find(needle) {
            let quote = needle.chars().last().unwrap();
            let rest = &text[idx + needle.len()..];
            if let Some(end) = rest.find(quote) {
                let cmd = &rest[..end];
                if !cmd.trim().is_empty() {
                    return Some(PendingToolCall {
                        id: format!("call_txt_{:08x}", rand_jitter(u64::MAX)),
                        name: "bash".into(),
                        input: serde_json::json!({"command": cmd.trim()}),
                    });
                }
            }
        }
    }
    if let Some(idx) = text.find("bash(") {
        let sub = &text[idx + 5..];
        if let Some(cmd) =
            extract_named_param(sub, "command=").or_else(|| extract_named_param(sub, "cmd="))
        {
            if !cmd.trim().is_empty() {
                return Some(PendingToolCall {
                    id: format!("call_txt_{:08x}", rand_jitter(u64::MAX)),
                    name: "bash".into(),
                    input: serde_json::json!({"command": cmd.trim()}),
                });
            }
        }
    }
    None
}

fn backoff_delay(attempt: u32, retry_after_secs: Option<u64>, base_ms: u64) -> std::time::Duration {
    if let Some(secs) = retry_after_secs {
        return std::time::Duration::from_secs(secs.max(1));
    }
    let exp = base_ms.saturating_mul(1u64 << (attempt - 1).min(6));
    let jitter = rand_jitter(exp);
    std::time::Duration::from_millis((exp / 2).max(1).saturating_add(jitter).min(30_000))
}

fn rand_jitter(ms: u64) -> u64 {
    use std::sync::atomic::{AtomicU64, Ordering};
    static STATE: AtomicU64 = AtomicU64::new(0);
    let x = STATE
        .fetch_add(0x9E3779B97F4A7C15, Ordering::Relaxed)
        .wrapping_add(0x9E3779B97F4A7C15);
    (x >> 33) % ms.max(2)
}

#[cfg(test)]
mod tool_recovery_tests {
    use super::tool_recovery_hint;

    #[test]
    fn repairable_failures_get_a_model_recovery_contract() {
        assert!(
            tool_recovery_hint(r#"{"type":"unknown_capability","name":"tavily_search"}"#).is_some()
        );
        assert!(tool_recovery_hint("mcp protocol error: invalid arguments").is_some());
    }

    #[test]
    fn authorization_and_user_control_failures_never_get_retry_advice() {
        assert!(tool_recovery_hint("capability denied by channel policy").is_none());
        assert!(tool_recovery_hint("cancelled").is_none());
        assert!(tool_recovery_hint("429 rate limit").is_none());
    }

    #[test]
    fn test_normalize_tool_call_aliases() {
        use super::{PendingToolCall, normalize_tool_call};

        // Read file alias normalization
        let read_call = PendingToolCall {
            id: "call_3".into(),
            name: "read_file".into(),
            input: serde_json::json!({
                "file_path": "src/main.rs"
            }),
        };
        let normalized_read = normalize_tool_call(read_call);
        assert_eq!(normalized_read.name, "read");
        assert_eq!(
            normalized_read.input.get("path").and_then(|v| v.as_str()),
            Some("src/main.rs")
        );
    }
}

#[cfg(test)]
mod auto_approve_tests {
    #![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
    use super::auto_approve;
    use super::{ApprovalMode, Mode};
    use vak_permission::AskSource;

    fn ws() -> tempfile::TempDir {
        tempfile::tempdir().unwrap()
    }

    #[test]
    fn rule_and_circuit_breaker_sources_never_auto_approve() {
        let cwd = ws();
        for source in [AskSource::Rule, AskSource::CircuitBreaker] {
            assert!(
                !auto_approve(
                    ApprovalMode::AutoApprove,
                    source,
                    "bash",
                    &serde_json::json!({"command": "ls"}),
                    Mode::WorkspaceWrite,
                    true,
                    cwd.path(),
                ),
                "AutoApprove must not override {source:?}"
            );
        }
    }

    #[test]
    fn auto_approve_mode_accepts_everything_safe_and_unsafe() {
        let cwd = ws();
        let json = serde_json::json!({"command": "rm -rf /"});
        assert!(auto_approve(
            ApprovalMode::AutoApprove,
            AskSource::ModeDefault,
            "bash",
            &json,
            Mode::WorkspaceWrite,
            true,
            cwd.path(),
        ));
    }

    #[test]
    fn approve_safe_auto_approves_sandboxed_bash() {
        // The security contract: bash in a restricted mode with a sandbox
        // is "safe" because the sandbox confines it — so ApproveSafe trusts it.
        let cwd = ws();
        assert!(
            auto_approve(
                ApprovalMode::ApproveSafe,
                AskSource::ModeDefault,
                "bash",
                &serde_json::json!({"command": "cargo test"}),
                Mode::WorkspaceWrite,
                true,
                cwd.path(),
            ),
            "sandboxed bash in WorkspaceWrite should be auto-approved under ApproveSafe"
        );
        assert!(
            auto_approve(
                ApprovalMode::ApproveSafe,
                AskSource::ModeDefault,
                "bash",
                &serde_json::json!({"command": "cargo test"}),
                Mode::ReadOnly,
                true,
                cwd.path(),
            ),
            "sandboxed bash in ReadOnly should be auto-approved under ApproveSafe"
        );
    }

    #[test]
    fn approve_safe_does_not_auto_approve_unsandboxed_bash() {
        // No sandbox => FullAccess-equivalent reach => never auto-approved.
        // This is the guardrail that stops ApproveSafe from silently
        // granting host-shell access.
        let cwd = ws();
        assert!(
            !auto_approve(
                ApprovalMode::ApproveSafe,
                AskSource::ModeDefault,
                "bash",
                &serde_json::json!({"command": "rm -rf /"}),
                Mode::WorkspaceWrite,
                false,
                cwd.path(),
            ),
            "un-sandboxed bash must not be auto-approved"
        );
        assert!(
            !auto_approve(
                ApprovalMode::ApproveSafe,
                AskSource::ModeDefault,
                "bash",
                &serde_json::json!({"command": "ls"}),
                Mode::FullAccess,
                true,
                cwd.path(),
            ),
            "bash under FullAccess must not be auto-approved even with a sandbox"
        );
    }

    #[test]
    fn approve_safe_auto_approves_read_tools() {
        let cwd = ws();
        for tool in ["read", "glob", "grep", "ls", "search"] {
            assert!(
                auto_approve(
                    ApprovalMode::ApproveSafe,
                    AskSource::ModeDefault,
                    tool,
                    &serde_json::json!({}),
                    Mode::WorkspaceWrite,
                    false,
                    cwd.path(),
                ),
                "{tool} should be auto-approved under ApproveSafe"
            );
        }
    }

    #[test]
    fn approve_safe_approves_workspace_write_only_when_in_workspace() {
        let cwd = ws();
        // Use a relative path so the workspace-rooting check resolves
        // against cwd without symlink-interpolation ambiguity on macOS
        // (/var → /private/var).
        assert!(
            auto_approve(
                ApprovalMode::ApproveSafe,
                AskSource::ModeDefault,
                "write",
                &serde_json::json!({"path": "notes.txt", "content": "x"}),
                Mode::WorkspaceWrite,
                false,
                cwd.path(),
            ),
            "writing inside the workspace should be auto-approved"
        );
        let outside = std::env::temp_dir().join("vak-outside.txt");
        assert!(
            !auto_approve(
                ApprovalMode::ApproveSafe,
                AskSource::ModeDefault,
                "write",
                &serde_json::json!({"path": outside.to_string_lossy(), "content": "x"}),
                Mode::WorkspaceWrite,
                false,
                cwd.path(),
            ),
            "writing outside the workspace must not be auto-approved"
        );
    }

    #[test]
    fn approve_safe_denies_non_mode_default_sources() {
        let cwd = ws();
        // Scope source is auto-approved for workspace-scoped writes.
        assert!(auto_approve(
            ApprovalMode::ApproveSafe,
            AskSource::Scope,
            "write",
            &serde_json::json!({"path": "ok.txt", "content": "x"}),
            Mode::WorkspaceWrite,
            false,
            cwd.path(),
        ));
    }

    #[test]
    fn ask_mode_never_auto_approves_anything() {
        let cwd = ws();
        assert!(!auto_approve(
            ApprovalMode::Ask,
            AskSource::ModeDefault,
            "read",
            &serde_json::json!({}),
            Mode::WorkspaceWrite,
            true,
            cwd.path(),
        ));
    }

    #[test]
    fn bash_without_command_arg_still_approved_when_sandboxed() {
        // auto_approve for bash keys only on (sandboxed && not FullAccess),
        // not on the presence of a command arg — the command arg is
        // validated separately by authorize(). This documents that boundary.
        let cwd = ws();
        assert!(
            !auto_approve(
                ApprovalMode::ApproveSafe,
                AskSource::ModeDefault,
                "bash",
                &serde_json::json!({}),
                Mode::WorkspaceWrite,
                false,
                cwd.path(),
            ),
            "un-sandboxed bash without command arg must not be auto-approved"
        );
        assert!(
            auto_approve(
                ApprovalMode::ApproveSafe,
                AskSource::ModeDefault,
                "bash",
                &serde_json::json!({}),
                Mode::WorkspaceWrite,
                true,
                cwd.path(),
            ),
            "sandboxed bash is auto-approved by the sand-boxing contract"
        );
    }
}

#[cfg(test)]
mod parse_text_tool_calls_tests {
    use super::parse_text_tool_calls;

    #[test]
    fn parses_write_functional_syntax() {
        let text = r#"Surface: terminal CLI.
```bash
write(path=".vak/scratch/test.html", content="<!DOCTYPE html>\n<html><body>Hi</body></html>")
```
"#;
        let calls = parse_text_tool_calls(text);
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].name, "write");
        assert_eq!(calls[0].input["path"], ".vak/scratch/test.html");
        assert_eq!(
            calls[0].input["content"],
            "<!DOCTYPE html>\n<html><body>Hi</body></html>"
        );
    }

    #[test]
    fn parses_bash_c_syntax() {
        let text = r#"bash -c "ls -l .vak/scratch/test.html && head -n 10 .vak/scratch/test.html""#;
        let calls = parse_text_tool_calls(text);
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].name, "bash");
        assert_eq!(
            calls[0].input["command"],
            "ls -l .vak/scratch/test.html && head -n 10 .vak/scratch/test.html"
        );
    }

    #[test]
    fn parses_json_fenced_tool_call() {
        let text = r##"Here is the call:
```tool_call
{"name": "write", "arguments": {"path": "notes.md", "content": "# Notes"}}
```
"##;
        let calls = parse_text_tool_calls(text);
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].name, "write");
        assert_eq!(calls[0].input["path"], "notes.md");
    }

    #[test]
    fn parses_positional_triple_quoted_write() {
        let text = r#"Surface: terminal CLI.
```bash
write(".vak/scratch/bloomberg.html", """
<!DOCTYPE html>
<html><body>Bloomberg</body></html>
""")
```
"#;
        let calls = parse_text_tool_calls(text);
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].name, "write");
        assert_eq!(calls[0].input["path"], ".vak/scratch/bloomberg.html");
        assert!(
            calls[0].input["content"]
                .as_str()
                .unwrap()
                .contains("Bloomberg")
        );
    }

    #[test]
    fn parses_named_write_with_nested_quoted_commas() {
        let text = r#"```bash
write(path="app.js", content="const data = [{ q: \"What?\", a: \"Answer\" }];")
```"#;
        let calls = parse_text_tool_calls(text);
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].name, "write");
        assert_eq!(calls[0].input["path"], "app.js");
        assert!(
            calls[0].input["content"]
                .as_str()
                .unwrap()
                .contains("What?")
        );
        assert!(
            calls[0].input["content"]
                .as_str()
                .unwrap()
                .contains("Answer")
        );
    }

    #[test]
    fn parses_cli_style_write_with_backslash() {
        let text = r#"Surface: terminal CLI.
```bash
write content=.vak/scratch/react_app.html \
"<!DOCTYPE html><html><body>React App</body></html>"
```"#;
        let calls = parse_text_tool_calls(text);
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].name, "write");
        assert_eq!(calls[0].input["path"], ".vak/scratch/react_app.html");
        assert!(
            calls[0].input["content"]
                .as_str()
                .unwrap()
                .contains("React App")
        );
    }
}
