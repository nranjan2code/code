//! vak-agent: the agent loop.
//!
//! Errors are values: run() never panics; every failure mode is a typed
//! TurnOutcome. Model context is always projected from the session log
//! (model-visible means logged).

mod fences;
use fences::{find_duplicate_card_fence, find_malformed_vak_fence, vak_fence_bodies};
pub mod circuit;
pub mod goal;
pub mod spend;
pub mod steering;
pub mod stop_policy;
pub mod task;
pub mod workspace;

// The context engine (docs/design/68-context-engine.md) is its own crate;
// the loop here only orchestrates it: probe, plan, assemble, dispatch,
// write back.
pub use circuit::{CircuitBreaker, CircuitBreakerConfig, CircuitOpen};
pub use goal::GoalState;
pub use spend::{SpendCheck, SpendGate};
pub use stop_policy::{BlockReason, ReceiptSummary, StopPolicy, is_code_path};
pub use task::{ActiveWorker, TaskDeps, TaskTool, WorkerHandle, WorkerRegistry};
use vak_context::assemble::{
    attach_tail, cache_breakpoints, capacity_feedback_delta, chat_request_chars, compose_tail,
    messages_chars, prefix_chars,
};
pub use vak_context::{CapacityProfile, TailInput};
use vak_context::{assemble, capacity, planner};
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
use vak_session::{MessageMeta, MessageRecord, SessionLog, TurnIndex};
use vak_tools::{RecallRequest, Tool, ToolContext, ToolErrorKind, ToolOutput};

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
    /// An incremental compaction (docs/design/68-context-engine.md §4) ran:
    /// the plan's packet range collapsed into a new `Compaction` entry.
    /// `after_tokens` is the re-planned budget spend once the packet is
    /// covered.
    ContextCompacted {
        before_tokens: u64,
        after_tokens: u64,
        summarized_turns: usize,
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
    WorkerStarted {
        label: String,
    },
    WorkerToolCall {
        label: String,
        name: String,
        is_error: bool,
    },
    WorkerUsage {
        label: String,
        input_tokens: u64,
        output_tokens: u64,
    },
    WorkerFinished {
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
        /// errors are values (see WorkerFinished above).
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
    /// The stable prefix: identity, contract, guardrails, tool surface. Byte-
    /// identical across steps of a turn and across turns for an unchanged
    /// capability packet, so a provider's prefix cache can key on it
    /// (docs/design/68-context-engine.md §4/§6). Per-turn content never
    /// belongs here — see `tail`.
    pub system_prefix: String,
    /// Per-turn content rendered into the moving tail instead of the
    /// prefix: the clock instant and the epistemic stance. Session-derived
    /// tail content (intent, work contract, conversation thread) is read
    /// from `SessionLog::tail_sections()` at request-assembly time instead,
    /// since it is not host-supplied configuration.
    pub tail: TailInput,
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
    pub presentation_check: Option<PresentationCheck>,
    pub retrieval_check: Option<RetrievalCheck>,
    /// Writes a `Presentation` ledger entry at the moment a card validates.
    /// See `PresentationRebuild`. `None` disables the write (the ack stays
    /// the tool's own generic text — no id to embed).
    pub presentation_rebuild: Option<PresentationRebuild>,
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
    /// Reserve for the completion (`max_tokens`), subtracted from the
    /// horizon by `CapacityProfile::budget` (docs/design/68-context-engine.md
    /// §4).
    pub max_output: u64,
    /// Provider-declared context window, used only to build a
    /// metadata-only `CapacityProfile` (`CapacityProfile::from_metadata_only`)
    /// when the host has not wired real capacity measurement in (e.g.
    /// standalone agent use, or a test) — so an unmeasured window is still a
    /// real number from configuration, never a hardcoded magic default
    /// baked into the planning math itself.
    pub declared_window: u64,
    /// Each admitted tool's declared domains, mirroring
    /// `TurnCapabilitiesBound.tool_domains` (docs/design/68-context-engine.md
    /// §7 "model drift"): a tool call whose domains are disjoint from the
    /// current reading's is drift evidence. Empty for a tool with no
    /// declared domain (never treated as a mismatch by itself).
    pub tool_domains: std::collections::BTreeMap<String, Vec<String>>,
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
    /// Measured capacity for this turn's model (docs/design/68-context-engine.md
    /// §1), supplied by `Core::capacity_profile_for`. `None` when the host
    /// has not wired capacity measurement in (e.g. standalone agent use);
    /// the feedback calls in the turn loop are then no-ops.
    pub capacity: Option<CapacityProfile>,
    /// The `ProfileKey` `capacity` was probed under, so feedback activities
    /// land on the same key (docs/design/68 §1).
    pub capacity_key: Option<capacity::ProfileKey>,
    /// Full schemas `find_tools` has surfaced so far this run
    /// (docs/design/68-context-engine.md §5). `tool_definitions()` appends
    /// these after the core set on every call, in discovery order, never
    /// reordered — so a tool the model asked for by name stays reachable
    /// for the rest of the turn without re-entering the stable prefix.
    pub discovered_tools: Arc<StdMutex<Vec<vak_llm::ToolDefinition>>>,
}

pub type HookRecorder = Arc<dyn Fn(&vak_hooks::HookDef, bool, u64) + Send + Sync>;
pub type ToolActivityRecorder = Arc<dyn Fn(&str, &serde_json::Value, bool, u64) + Send + Sync>;
pub type RevocationCheck = Arc<dyn Fn(&str, &serde_json::Value) -> bool + Send + Sync>;

/// Given a final answer's text and the names of the tools offered this turn,
/// returns a nudge when the answer reads as something that should have been
/// presented as a card. Supplied by `Core` (which owns the presentation
/// vocabulary) so the agent loop stays free of any card knowledge.
pub type PresentationCheck = Arc<dyn Fn(&str, &[String]) -> Option<String> + Send + Sync>;

/// Whether a tool call reaches information from outside the machine and the
/// conversation (the kind an answer should cite), given the tool's name and
/// the call's input. Supplied by `Core`, which knows what each capability
/// *declares it serves*; the agent loop never guesses from a tool's name or
/// its output. Absent, no call counts as retrieval and the grounding check is
/// inert.
pub type RetrievalCheck = Arc<dyn Fn(&str, &serde_json::Value) -> bool + Send + Sync>;

/// Everything the agent loop needs to write a `Presentation` ledger entry
/// for a validated `emit_*_card` call (docs/design/68-context-engine.md
/// §10), supplied by the card-shape knowledge that lives in
/// `vak-core::presentation_tools` — this crate has no skill-registry or
/// card-shape knowledge of its own, only the mechanics of writing the entry.
#[derive(Debug, Clone)]
pub struct PresentationCardInfo {
    pub semantic_type: String,
    pub skill_id: String,
    pub skill_version: String,
    pub schema_version: u32,
    /// Canonical (key-sorted) payload — see
    /// `vak_session::types::canonicalize_json`.
    pub payload: serde_json::Value,
    pub title: String,
    pub identity_digest: String,
}

/// Re-validates a card call from its own arguments and returns the info
/// needed to write its `Presentation` entry, or `None` if it no longer
/// validates (unreachable in practice: `execute()` already validated it
/// before this is ever consulted). Supplied by `Core`
/// (`presentation_tools::presentation_info`) because a card tool executes
/// across the worker/broker boundary (AGENTS.md invariant 14) and has no
/// session-log access itself, so the agent loop writes the entry here, at
/// the point the tool result is appended to the session.
pub type PresentationRebuild =
    Arc<dyn Fn(&str, &serde_json::Value) -> Option<PresentationCardInfo> + Send + Sync>;

impl AgentConfig {
    pub fn new(system_prefix: impl Into<String>) -> Self {
        AgentConfig {
            outcome: None,
            work_mode: WorkMode::Direct,
            work_enabled: true,
            max_work_items: 20,
            max_work_revisions: 8,
            system_prefix: system_prefix.into(),
            tail: TailInput::default(),
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
            presentation_check: None,
            retrieval_check: None,
            presentation_rebuild: None,
            hook_recorder: None,
            tool_activity_recorder: None,
            max_retries: 3,
            retry_base_backoff_ms: 500,
            request_timeout: Some(std::time::Duration::from_secs(600)),
            circuit_breaker: None,
            run_retry_attempts: 6,
            run_retry_base_backoff_ms: 2_000,
            dispatch_ceiling: (3 + 1) * (6 + 1),
            max_output: 8_192,
            declared_window: 128_000,
            tool_domains: std::collections::BTreeMap::new(),
            stop_policy: Some(StopPolicy::default()),
            spend_gate: None,
            ladder: Vec::new(),
            ladder_provider_names: Vec::new(),
            workspace_delta: None,
            handoff_reset: true,
            max_audit_blocks: 2,
            capacity: None,
            capacity_key: None,
            discovered_tools: Arc::new(StdMutex::new(Vec::new())),
        }
    }
}

/// Renders the one control block appended to the last user message of a
/// request (docs/design/68-context-engine.md §6/§10): the host-supplied
/// per-turn content under its own tag, followed by whichever
/// session-derived sections `SessionLog::tail_sections()` returned.
/// Sections absent from `sections` are omitted entirely, never emitted as an
/// empty tag pair.
/// The no-op result for an `emit_*_card` call identical to one already shown
/// this run (cards are Vak's own display channel; a repeat is not an error).
const CARD_REPEAT_ACK: &str = "Card already displayed to the user. Do not call it again: add at most one short sentence and finish.";

/// Consecutive all-repeat card batches after which the turn closes on the
/// card as its answer. Three: one repeat is a slip the ack corrects, two is
/// a model that did not read it, three is one that will not.
const CARD_REPEAT_EXHAUSTION_THRESHOLD: u32 = 3;

/// The most recent Execute-purpose receipt's prefix digest recorded in this
/// session, or `None` when no receipt has recorded one yet.
fn last_prefix_digest(session: &SessionLog) -> Option<String> {
    session
        .chain_to_root()
        .into_iter()
        .rev()
        .find_map(|entry| match &entry.payload {
            vak_session::EntryPayload::Receipt(receipt) if !receipt.prefix_digest.is_empty() => {
                Some(receipt.prefix_digest.clone())
            }
            _ => None,
        })
}

/// Whether any earlier receipt in this session already carries `digest` —
/// used to measure `prefix_tokens` only on the first request seen with a
/// given digest.
fn prefix_digest_seen(session: &SessionLog, digest: &str) -> bool {
    session.chain_to_root().into_iter().any(|entry| {
        matches!(&entry.payload, vak_session::EntryPayload::Receipt(receipt) if receipt.prefix_digest == digest)
    })
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

/// Per-leg tool inclusion (docs/design/68-context-engine.md §5): Anthropic
/// legs get the full core+deferred set (deferred schemas withheld from the
/// prefix there via `defer_loading`, discoverable through the server-side
/// tool-search tool); every other provider gets core only — its adapter
/// ignores `ToolDefinition::defer` and would otherwise send the deferred
/// schema in full, defeating the point of deferring it.
fn tools_for_leg(
    tools: &[vak_llm::ToolDefinition],
    provider_name: &str,
) -> Vec<vak_llm::ToolDefinition> {
    if provider_name == "anthropic" {
        return tools.to_vec();
    }
    tools.iter().filter(|t| !t.defer).cloned().collect()
}

/// Renders a turn's full record (docs/design/68-context-engine.md §10) as
/// plain text for a `recall({ turn })` result: one `role: text` line per
/// message, in order.
fn render_full_record(messages: &[Message]) -> String {
    messages
        .iter()
        .map(|message| {
            let role = match message.role {
                vak_llm::Role::User => "user",
                vak_llm::Role::Assistant => "assistant",
            };
            format!("{role}: {}", message.text_content())
        })
        .collect::<Vec<_>>()
        .join("\n")
}

/// The text up to and including its first sentence-ending punctuation,
/// used as the deterministic fallback when the narration-gist side call
/// (docs/design/68-context-engine.md §10) errors. A semantic boundary, never
/// a character count.
fn first_sentence_fallback(text: &str) -> String {
    let trimmed = text.trim();
    let bytes = trimmed.as_bytes();
    for (i, b) in bytes.iter().enumerate() {
        if matches!(b, b'.' | b'!' | b'?') {
            let after = i + 1;
            if after >= bytes.len() || matches!(bytes[after], b' ' | b'\n') {
                return trimmed[..after].to_string();
            }
        }
    }
    trimmed.to_string()
}

/// Words too generic to establish that a card is *about* the same thing as
/// the directive: recency deixis ("current", "now") appears in both a
/// legitimate live-data directive and an unrelated one, and would make any
/// two such directives look related if left in; ordinary function words and
/// a few request-shaped verbs are excluded for the same reason. Deliberately
/// small and topic-neutral — this never grows into a per-domain keyword
/// list, it only strips words that carry no topic of their own.
const TOPIC_STOPWORDS: &[&str] = &[
    "a",
    "an",
    "the",
    "is",
    "are",
    "was",
    "were",
    "what",
    "who",
    "when",
    "where",
    "how",
    "why",
    "current",
    "currently",
    "now",
    "today",
    "tonight",
    "latest",
    "right",
    "this",
    "that",
    "of",
    "in",
    "on",
    "at",
    "to",
    "for",
    "and",
    "or",
    "with",
    "me",
    "please",
    "give",
    "tell",
    "show",
    "get",
    "find",
    "you",
];

/// Lower-cased, stopword-stripped word set of `text`, for a cheap topic
/// overlap check — not a search index, just "do these two strings share a
/// real word".
fn topic_tokens(text: &str) -> std::collections::HashSet<String> {
    text.to_ascii_lowercase()
        .split(|c: char| !c.is_alphanumeric())
        .filter(|word| word.len() > 2 && !TOPIC_STOPWORDS.contains(word))
        .map(str::to_string)
        .collect()
}

/// Whether an `emit_*_card` call's own text (its title/payload, serialized)
/// shares at least one real word with the directive it is supposed to
/// answer. Found live: a model that had just run a correct, on-topic search
/// still wrote a card from an unrelated older turn's payload ("Noida
/// Weather", "28°C") in answer to "what is the current top news in AI" — the
/// call executed, `derived_from` correctly pointed at the real search
/// result, and nothing else in the loop notices the payload itself has
/// nothing to do with either the question or that evidence. A directive with
/// no topic words of its own (every word is a stopword) is never gated —
/// there is nothing to compare against, and refusing everything would be a
/// worse failure than missing this one.
fn card_shares_a_topic_with(directive: &str, name: &str, input: &serde_json::Value) -> bool {
    let directive_words = topic_tokens(directive);
    if directive_words.is_empty() {
        return true;
    }
    let card_text = serde_json::to_string(input).unwrap_or_default() + " " + name;
    let card_words = topic_tokens(&card_text);
    directive_words.iter().any(|word| card_words.contains(word))
}

#[cfg(test)]
mod topic_gate_tests {
    use super::card_shares_a_topic_with;

    #[test]
    fn a_weather_card_matches_a_weather_directive() {
        let input = serde_json::json!({
            "semantic_type": "weather",
            "payload": {"label": "Noida Weather", "unit": "Celsius", "value": "28°C"}
        });
        assert!(card_shares_a_topic_with(
            "what is the current weather in noida",
            "emit_metric_card",
            &input
        ));
    }

    #[test]
    fn the_real_regression_a_weather_card_does_not_match_an_ai_news_directive() {
        // Exactly what shipped and broke live: a correct on-topic search for
        // "current top news in AI" ran, then the model wrote this weather
        // card from an unrelated older turn anyway.
        let input = serde_json::json!({
            "semantic_type": "weather",
            "payload": {"label": "Noida Weather", "unit": "Celsius", "value": "28°C"}
        });
        assert!(!card_shares_a_topic_with(
            "what is the current top news in AI",
            "emit_metric_card",
            &input
        ));
    }

    #[test]
    fn an_on_topic_ai_card_matches() {
        let input = serde_json::json!({
            "semantic_type": "research.synthesis",
            "payload": {"title": "AI news roundup", "sources": [{"title": "Latest AI breakthroughs"}]}
        });
        assert!(card_shares_a_topic_with(
            "what is the current top news in AI",
            "emit_research_card",
            &input
        ));
    }

    #[test]
    fn a_directive_with_only_stopwords_is_never_gated() {
        let input = serde_json::json!({"payload": {"label": "anything at all"}});
        assert!(card_shares_a_topic_with(
            "what is this now",
            "emit_metric_card",
            &input
        ));
    }

    #[test]
    fn a_correctly_derived_card_that_renames_the_topic_still_passes() {
        // The false-positive risk of a literal word check: a card that is
        // genuinely right but titled from what the search actually returned
        // ("OpenAI announces GPT-6"), not from the user's own words ("AI
        // news"), must not be gated just because it paraphrased. Checked
        // against the directive PLUS this turn's evidence together — the
        // call site does this by widening the context string before calling
        // this function, which is what this test exercises directly.
        let input = serde_json::json!({
            "payload": {"title": "OpenAI announces GPT-6", "summary": "a major model release"}
        });
        let directive_plus_evidence =
            "what is the current top news in AI OpenAI today unveiled GPT-6, its newest model";
        assert!(card_shares_a_topic_with(
            directive_plus_evidence,
            "emit_entity_card",
            &input
        ));
    }

    #[test]
    fn recency_words_alone_never_establish_a_shared_topic() {
        // Both directives use "current"/"now"/"right"; without stripping
        // them as stopwords, this unrelated pair would look related.
        let input = serde_json::json!({"payload": {"label": "Noida Weather"}});
        assert!(!card_shares_a_topic_with(
            "what is currently happening right now in politics",
            "emit_metric_card",
            &input
        ));
    }
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
    /// `name + input` of every card call that displayed successfully this run.
    presented_cards: std::sync::Mutex<std::collections::HashSet<String>>,
    /// Active goal (Phase H): set via `set_goal`, consumed by the audit
    /// gate on completion claims.
    active_goal: Option<goal::GoalState>,
    /// Bash commands proven green this run — re-run before any done claim.
    obligations: Vec<String>,
    /// The reset-with-handoff rescue fires at most once per run.
    handoff_used: bool,
    /// Per-run bookkeeping for model-guided tool recovery (see RepairState).
    repair: RepairState,
    /// Consecutive model-drift events this run (docs/design/68-context-
    /// engine.md §7): reset to 0 on any non-drifting step; three in a row
    /// end the turn with the degraded outcome.
    drift_streak: u32,
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
            presented_cards: std::sync::Mutex::new(std::collections::HashSet::new()),
            active_goal: None,
            obligations: Vec::new(),
            handoff_used: false,
            repair: RepairState::default(),
            drift_streak: 0,
        }
    }

    /// One `WorkingSetPlan` for the ledger as it stands
    /// (`vak_context::plan_for_session`). Called fresh per step (and again
    /// after an incremental compaction) so it always reflects the latest
    /// chain; the lock is held only for the pure planning pass.
    async fn build_working_set_plan(
        &self,
        profile: &CapacityProfile,
        prefix_tokens: u64,
        tail_tokens: u64,
    ) -> vak_session::WorkingSetPlan {
        let session = self.session.lock().await;
        planner::plan_for_session(&session, profile, prefix_tokens, tail_tokens)
    }

    /// The `CapacityProfile` to plan this turn against: the host-wired one
    /// (`Core::capacity_profile_for`) when present, or a metadata-only
    /// profile built from `declared_window`/`max_output` — the "construct
    /// `CapacityProfile::from_metadata_only` from the discovered window"
    /// fallback (docs/design/68-context-engine.md §4), never a magic
    /// number baked into the planner itself.
    fn effective_capacity_profile(&self) -> CapacityProfile {
        self.config.capacity.clone().unwrap_or_else(|| {
            CapacityProfile::from_metadata_only(
                self.config.declared_window,
                self.config.max_output,
                "no-capacity-wired".to_string(),
                std::time::SystemTime::now(),
            )
        })
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
        // MCP tools are reached only through the `mcp` broker
        // (docs/design/68-context-engine.md §5): `mcp_aliases` still
        // resolves a call the model addresses by the bare tool name
        // (`normalize_mcp_alias` in `execute_batch_calls`), but the alias
        // schema is never advertised directly — advertising it here was the
        // triplication (inline catalogue, direct tool, broker) the design
        // deletes. `mcp list` / `find_tools` are how a model discovers one.
        let discovered = self
            .config
            .discovered_tools
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .clone();
        for definition in discovered {
            if !definitions
                .iter()
                .any(|existing| existing.name == definition.name)
            {
                definitions.push(definition);
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
        let outcome = self
            .run_message_inner(prompt, steering, cancel.clone(), events.clone())
            .await;
        // A system-authored completion (drift, card-repeat and stale-data
        // outcomes) is the turn's answer as much as a model-authored one:
        // it must be on the ledger, or the turn never closes and the next
        // request would fold this one into it (invariant 1: model-visible
        // means logged).
        if let TurnOutcome::Completed { response } = &outcome {
            let already_logged = {
                let session = self.session.lock().await;
                session.message_chain().last().is_some_and(|(_, last)| {
                    last.role == Role::Assistant && last.content == response.content
                })
            };
            if !already_logged {
                self.append_assistant(response).await;
            }
        }
        // Turn-close hook (docs/design/68-context-engine.md §10): builds and
        // appends the TurnCard once the turn has actually closed. A turn
        // that never got a final assistant text (most `Failed`/aborted
        // exits) stays open and this is a no-op — there is nothing to card
        // yet, and the next call to `run_message` will pick it up once it
        // does close.
        self.close_turn(&outcome, &cancel, &events).await;
        outcome
    }

    async fn run_message_inner(
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
        let mut receipts = stop_policy::ReceiptSummary::default();
        let mut verification_stale = false;
        let mut user_completion_released = false;
        self.obligations.clear();
        self.handoff_used = false;
        self.run_call_counts
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .clear();
        self.presented_cards
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

        // Captured once for the whole turn (docs/design/68-context-engine.md
        // §6/§10): every step's request must carry byte-identical tail bytes
        // so the provider's prefix cache serves the growing middle instead
        // of re-billing it on every step. Session-derived sections (intent,
        // work contract, conversation thread) are read from the ledger here
        // rather than supplied by the caller, since they are derived state,
        // not host configuration.
        //
        // The conversation thread section depends on the working-set plan
        // (it lists only directives the projection leaves out), and the
        // plan's budget depends on the tail's size — so the tail is sized
        // from a preliminary plan built against the thread-less sections,
        // then composed from the plan-aware ones. The per-step plans below
        // may differ from this one by at most the thread's own few lines;
        // a turn that demotes to `Card` as a result still carries its
        // directive on its card line.
        let turn_tail = {
            let profile = self.effective_capacity_profile();
            let tool_defs = self.tool_definitions();
            let prefix_tokens =
                profile.estimate_tokens(prefix_chars(&self.config.system_prefix, &tool_defs));
            let base_tail = {
                let session = self.session.lock().await;
                compose_tail(&self.config.tail, &session.tail_sections(None))
            };
            let base_tail_tokens = profile.estimate_tokens(base_tail.chars().count() as u64);
            let preliminary_plan = self
                .build_working_set_plan(&profile, prefix_tokens, base_tail_tokens)
                .await;
            let session = self.session.lock().await;
            let sections = session.tail_sections(Some(&preliminary_plan));
            compose_tail(&self.config.tail, &sections)
        };

        let mut turn = 0usize;
        let mut outcome_turns = 0usize;
        self.repair.reset();
        self.run_call_counts
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .clear();
        self.presented_cards
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .clear();
        let mut stop_blocks_left = self
            .config
            .stop_policy
            .as_ref()
            .map(|p| p.max_blocks)
            .unwrap_or(0);
        // Names of retrieval-shaped tools that succeeded on the immediately
        // preceding turn (see `AgentConfig::retrieval_check`), cleared and
        // recomputed every time a tool-call batch runs. Consulted the very
        // next time the model produces a final text-only answer, to catch
        // the case where a search/fetch tool call succeeded and the model's
        // very next turn ignored the result instead of grounding on it.
        let mut pending_grounding_check: Option<Vec<String>> = None;
        let mut grounding_repair_attempted = false;
        // Whether any retrieval-shaped call succeeded at any point in this
        // run, for the freshness check: a directive whose reading carries
        // the `live-data` domain (temporal deixis — "current", "right now")
        // asks for a value retrieved this turn, and an answer or card that
        // arrives without one is repeating what an earlier turn found.
        let mut retrieval_succeeded_this_run = false;
        let mut freshness_repair_attempted = false;
        let mut empty_step_repair_attempted = false;
        let mut card_repeat_streak: u32 = 0;
        let mut topic_repair_attempted = false;
        // The most recent non-card tool result's own text this run, for the
        // fail-closed fallback below: when a topic-mismatched card has to be
        // refused twice, the evidence that WAS gathered is worth showing
        // rather than nothing (found live: a real `tavily_search` for "AI
        // news" succeeded, then the model wrote an unrelated "Noida Weather"
        // card twice in a row — the search result was sitting right there
        // and never got refused nothing).
        let mut last_evidence_snippet: Option<String> = None;
        let wants_live_data = self
            .session
            .lock()
            .await
            .latest_reading()
            .is_some_and(|reading| reading.domains.iter().any(|d| d == "live-data"));
        let mut malformed_fence_repair_attempted = false;
        // `semantic_type`s successfully emitted via an `emit_*_card` tool
        // call in the immediately preceding batch. A tool-emitted card is
        // already shown to the user (vak-server's projection pushes it from
        // the tool result independently of the assistant's own text), so if
        // the model's very next answer *also* writes a `vak` fence
        // repeating the same semantic_type, that's a second, duplicate card
        // — observed live against the real local model (gemma4:e2b-mlx):
        // it called `emit_chart_card` successfully, then still wrote out
        // the identical chart as a trailing fence. Same bounded-repair
        // pattern as `pending_grounding_check`/`malformed_fence_repair_attempted`.
        let mut pending_duplicate_card_check: Option<Vec<String>> = None;
        let mut duplicate_card_repair_attempted = false;
        // Whether any `emit_*_card` call succeeded so far in this run (unlike
        // `pending_duplicate_card_check`, which only covers the last batch),
        // and the one-shot flag for the presentation check below.
        let mut cards_emitted_this_run = false;
        let mut presentation_repair_attempted = false;
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
                    let raw = message.text_content();
                    let active_revision = session.active_goal_revision();
                    // Only an explicit command changes the goal's shape; free
                    // text adds to it (docs/design/47, control plane).
                    let command = vak_intent::parse_command(&raw);
                    let relation = vak_intent::goal_relation(command.as_ref(), active_revision);
                    let request = command
                        .as_ref()
                        .and_then(|c| c.text())
                        .map(str::to_string)
                        .unwrap_or(raw);
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

            // config.model is set by run_turn_inner from effective_model() on
            // every turn, so it always reflects the current live provider route.
            let model = self.config.model.clone();

            // Working-set planning (docs/design/68-context-engine.md
            // §4/§10): build the TurnIndex and the plan fresh every step —
            // not once per turn — so a just-closed turn's card, a mid-turn
            // over-length replan (§5), or an incremental compaction below
            // all see the freshest chain. The lock is taken only for short
            // read/plan/apply phases and is NEVER held across the
            // summarizer network call below.
            let profile = self.effective_capacity_profile();
            let tool_defs = self.tool_definitions();
            let prefix_tokens =
                profile.estimate_tokens(prefix_chars(&self.config.system_prefix, &tool_defs));
            let tail_tokens = profile.estimate_tokens(turn_tail.chars().count() as u64);

            let mut plan = self
                .build_working_set_plan(&profile, prefix_tokens, tail_tokens)
                .await;

            // No usable horizon at all: the open turn alone (plus prefix,
            // tail, output reserve) already exceeds the horizon. Nothing is
            // plannable, so the reset-with-handoff rescue is the surviving
            // recovery (§4's "the handoff reset stays as the recovery when
            // a profile has no usable horizon").
            if plan.budget == 0 {
                let est_tokens = prefix_tokens
                    .saturating_add(tail_tokens)
                    .saturating_add(profile.output_reserve);
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
                    error: LlmError::Context("context over budget: no usable horizon".into()),
                };
            }

            // Incremental compaction (docs/design/68-context-engine.md §4):
            // the plan collapsed some turns into a packet that no existing
            // `Compaction` entry covers yet. Summarize their CARDS (never
            // raw history) and append one, then re-plan — the packet
            // disappears from the new plan once it is covered.
            if let Some((first_turn_id, last_turn_id)) = plan.packet_range.clone() {
                let needs_compaction = {
                    let session = self.session.lock().await;
                    session.packet_needs_compaction(&first_turn_id, &last_turn_id)
                };
                if needs_compaction {
                    let (transcript, transcript_chars) = {
                        let session = self.session.lock().await;
                        session.packet_transcript(&first_turn_id, &last_turn_id)
                    };
                    let tokens_before = profile.estimate_tokens(transcript_chars);
                    let _ = events
                        .send(AgentEvent::ContextCompacting {
                            estimated_tokens: tokens_before,
                        })
                        .await;
                    let req = assemble::compaction_request(&model, &transcript);
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
                        if let Err(e) = session.append_incremental_compaction(
                            &first_turn_id,
                            &last_turn_id,
                            &model,
                            summary,
                            tokens_before,
                        ) {
                            return TurnOutcome::Failed {
                                error: LlmError::Network(format!("compaction write failed: {e}")),
                            };
                        }
                    }
                    let after_plan = self
                        .build_working_set_plan(&profile, prefix_tokens, tail_tokens)
                        .await;
                    let _ = events
                        .send(AgentEvent::ContextCompacted {
                            before_tokens: tokens_before,
                            after_tokens: after_plan.spent,
                            summarized_turns: 0,
                        })
                        .await;
                    plan = after_plan;
                }
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
            // Recorded on every exit path below, success or failure: the
            // digest describes what was SENT, not what came back
            // (docs/design/68-context-engine.md §6/§7).
            ledger.receipt.prefix_digest =
                assemble::prefix_digest(&self.config.system_prefix, &tool_defs);
            let base_request = {
                let session = self.session.lock().await;
                // `messages` is already the fidelity-selected projection
                // (docs/design/68-context-engine.md §4/§10): retrieved-by-
                // relevance turns ride at Full inside it, so there is no
                // separate proactive-retrieval prepend step any more.
                let mut messages = session.derive_with_plan(&plan);
                // The tail is one final text block on the last user message
                // (after any tool_result blocks), never a separate consecutive
                // user message (docs/design/68-context-engine.md §6).
                attach_tail(&mut messages, &turn_tail);
                let session_key = session
                    .header()
                    .map(|header| header.session_id.clone())
                    .unwrap_or_default();
                let cache = (!session_key.is_empty()).then(|| vak_llm::CacheHints {
                    session_key,
                    breakpoints: cache_breakpoints(&messages),
                });
                ChatRequest {
                    model,
                    system: Some(self.config.system_prefix.clone()),
                    messages,
                    tools: tool_defs.clone(),
                    max_tokens: self.config.max_output as u32,
                    temperature: None,
                    cache,
                    previous_response_id: None,
                    think: None,
                }
            };
            let mut request = base_request.clone();

            let response = {
                // Run-level endurance: a sustained fault window (rate-limit
                // burst, slow/hung upstream, truncating proxy) can outlast
                // one step's retry budget. The ledger has not been touched,
                // so re-attempting the whole turn is exact. Aborts, permanent
                // errors, and ceiling exhaustion still fail/abort immediately.
                let mut run_attempt: u32 = 0;
                let mut backoff_ms = self.config.run_retry_base_backoff_ms.max(1);
                // Over-length replan (docs/design/68-context-engine.md §5):
                // a provider context-length rejection is a CapacityProfile
                // contradiction, not a transient fault — retried once, with
                // the horizon lowered and the request replanned smaller. A
                // second rejection on the retry is the turn's failure.
                let mut context_replan_used = false;
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
                            let partial = partial.map(|boxed| *boxed);
                            if let Some(p) = &partial {
                                let _ = self.append_assistant(p).await;
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
                        Err(LlmError::Context(reason)) if !context_replan_used => {
                            context_replan_used = true;
                            let request_tokens =
                                profile.estimate_tokens(chat_request_chars(&request));
                            let mut lowered = profile.clone();
                            lowered.observe_over_length(request_tokens);
                            self.config.capacity = Some(lowered.clone());
                            let mut data = self.capacity_activity_data(&lowered);
                            data.insert("reason".into(), reason.clone());
                            data.insert("request_tokens".into(), request_tokens.to_string());
                            self.record_activity(
                                vak_session::ActivityKind::CapacityFeedback,
                                vak_session::ActivityStatus::Succeeded,
                                "Capacity horizon lowered by an over-length rejection".into(),
                                Some(reason),
                                data,
                            )
                            .await;
                            let new_prefix_tokens = lowered.estimate_tokens(prefix_chars(
                                &self.config.system_prefix,
                                &tool_defs,
                            ));
                            let new_tail_tokens =
                                lowered.estimate_tokens(turn_tail.chars().count() as u64);
                            plan = self
                                .build_working_set_plan(
                                    &lowered,
                                    new_prefix_tokens,
                                    new_tail_tokens,
                                )
                                .await;
                            request = {
                                let session = self.session.lock().await;
                                let mut messages = session.derive_with_plan(&plan);
                                attach_tail(&mut messages, &turn_tail);
                                let session_key = session
                                    .header()
                                    .map(|header| header.session_id.clone())
                                    .unwrap_or_default();
                                let cache =
                                    (!session_key.is_empty()).then(|| vak_llm::CacheHints {
                                        session_key,
                                        breakpoints: cache_breakpoints(&messages),
                                    });
                                ChatRequest {
                                    model: self.config.model.clone(),
                                    system: Some(self.config.system_prefix.clone()),
                                    messages,
                                    tools: tool_defs.clone(),
                                    max_tokens: self.config.max_output as u32,
                                    temperature: None,
                                    cache,
                                    previous_response_id: None,
                                    think: None,
                                }
                            };
                            continue;
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
                let mut receipt = ledger.take_receipt();
                let settled_provider = receipt.provider.clone();
                if !receipt.prefix_digest.is_empty() {
                    // A digest that differs from the immediately preceding
                    // receipt's is a cache-breaking event, surfaced so a
                    // regression is visible in the ledger rather than only in
                    // the bill (docs/design/68-context-engine.md §7).
                    if let Some(previous) = last_prefix_digest(&session)
                        .filter(|previous| previous != &receipt.prefix_digest)
                    {
                        let now = chrono::Utc::now();
                        let activity = vak_session::ActivityRecord {
                            activity_id: format!(
                                "activity-{}",
                                now.timestamp_nanos_opt()
                                    .unwrap_or_else(|| now.timestamp_micros() * 1_000)
                            ),
                            turn: None,
                            kind: vak_session::ActivityKind::Diagnostic,
                            status: vak_session::ActivityStatus::Succeeded,
                            label: "prefix-changed".to_string(),
                            detail: None,
                            data: [
                                ("previous".to_string(), previous),
                                ("current".to_string(), receipt.prefix_digest.clone()),
                            ]
                            .into_iter()
                            .collect(),
                        };
                        let _ = session.append_activity(activity);
                    }
                    // Measured once per digest: the provider's reported input
                    // tokens minus an estimate of the messages alone. A later
                    // request with the same digest reuses this measurement
                    // rather than re-deriving it from a cache-served step.
                    if !prefix_digest_seen(&session, &receipt.prefix_digest) {
                        let profile = self.effective_capacity_profile();
                        let messages_tokens =
                            profile.estimate_tokens(messages_chars(&request.messages));
                        receipt.prefix_tokens =
                            Some(usage.input_tokens.saturating_sub(messages_tokens));
                    }
                }
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
            self.record_capacity_usage_feedback(&request, &usage, ledger.last_first_token_ms)
                .await;
            let response_entry_id = self.append_assistant(&response).await;
            let _ = events.send(AgentEvent::TurnEnd { usage }).await;

            let calls = extract_tool_calls(&response)
                .into_iter()
                .map(normalize_tool_call)
                .collect::<Vec<_>>();

            // Model drift (docs/design/68-context-engine.md §7): the step
            // served a different directive than the current one. Never a
            // cut — the steering nudge is appended and the turn continues;
            // only three CONSECUTIVE drift events end it.
            if let Some(drift_reason) = self.detect_model_drift(&response, &calls).await {
                self.drift_streak += 1;
                if self.drift_streak >= MODEL_DRIFT_EXHAUSTION_THRESHOLD {
                    return self.degraded_drift_outcome(&drift_reason).await;
                }
                // A request near the horizon that also drifted is evidence
                // the horizon itself is optimistic (§1, §6).
                self.record_capacity_instruction_failure(response.usage.input_tokens)
                    .await;
                let directive_quote = first_sentence_fallback(&prompt_owned);
                let _ = self.session.lock().await.append_message(MessageRecord::control(
                    vak_intent::control::ControlKind::SteeringDrift,
                    format!(
                        "[steering-drift]: {drift_reason}. The directive you should be serving right now is: \"{directive_quote}\". Refocus your next step on it."
                    ),
                ));
                if calls.is_empty() {
                    // A drifted final answer is not accepted as the turn's
                    // answer: redo it, same as the other repair nudges.
                    if turn + 1 >= self.config.max_turns {
                        return TurnOutcome::MaxTurnsReached;
                    }
                    turn += 1;
                    continue;
                }
                // A drifted tool call still needs its result dispatched
                // (API validity requires the pair); the nudge above steers
                // the NEXT step instead of interrupting this one.
            } else {
                self.drift_streak = 0;
            }

            if calls.is_empty() {
                // Empty-step enforcement: the response carried neither text
                // nor a tool call — a thinking-only completion, which a
                // model with a reasoning channel produces when it plans an
                // action and then stops (observed live: "Final Plan: 1. Use
                // tavily_search…" followed by end of turn, four runs out of
                // six). That is not an answer; one bounded redo asks it to
                // act on the plan it already made. A card emitted earlier in
                // the run IS the answer, so a card-only turn is left alone.
                if response.text_content().trim().is_empty()
                    && !cards_emitted_this_run
                    && !empty_step_repair_attempted
                {
                    empty_step_repair_attempted = true;
                    if turn + 1 >= self.config.max_turns {
                        return TurnOutcome::MaxTurnsReached;
                    }
                    let _ = self
                        .session
                        .lock()
                        .await
                        .append_message(MessageRecord::control(
                        vak_intent::control::ControlKind::EmptyStep,
                        "[empty-step]: Your last response had no visible answer and no tool call. \
                         Act now: make the tool call you planned, or write the answer as text."
                            .to_string(),
                    ));
                    turn += 1;
                    continue;
                }
                // Freshness enforcement (docs/design/68 §7): the directive
                // asked for a current value and nothing was retrieved in
                // this run, so the answer — prose or card — can only be a
                // repeat of an earlier turn's data. One bounded redo naming
                // the gap; the model may decline by saying it has no live
                // data, which the grounding phrases below already accept.
                if wants_live_data && !retrieval_succeeded_this_run {
                    let text = response.text_content();
                    let lower = text.to_ascii_lowercase();
                    let admits_no_data = [
                        "don't have",
                        "do not have",
                        "no access to",
                        "couldn't find",
                        "could not find",
                        "no live data",
                    ]
                    .iter()
                    .any(|phrase| lower.contains(phrase));
                    if !admits_no_data && freshness_repair_attempted {
                        // Repaired once already and still nothing retrieved
                        // (a thinking-only end, or the same figure again):
                        // fail closed rather than accept a stale answer.
                        return self.stale_data_outcome().await;
                    }
                    if !admits_no_data {
                        freshness_repair_attempted = true;
                        if turn + 1 >= self.config.max_turns {
                            return TurnOutcome::MaxTurnsReached;
                        }
                        let _ = self.session.lock().await.append_message(MessageRecord::control(
                            vak_intent::control::ControlKind::FreshnessCheck,
                            "[freshness-check]: This asks for a value as it stands now, but nothing was \
                             retrieved on this turn — a number carried over from an earlier answer is \
                             stale. Call a retrieval tool for a current reading and answer from what it \
                             returns (a card is fine), or say plainly that you have no live data."
                                .to_string(),
                        ));
                        turn += 1;
                        continue;
                    }
                }
                // Grounding enforcement: the previous turn ran one or more
                // retrieval-shaped tool calls, but this final answer neither
                // emitted a structured (cited) card nor admitted it has no
                // data. Give the model exactly one bounded repair turn
                // instead of letting an ungrounded answer reach the user —
                // this is the runtime-enforcement half of the fix; the
                // system prompt's own wording is the other half, since a
                // small/local model can't be trusted to self-police this
                // from prompt text alone.
                if !grounding_repair_attempted
                    && let Some(tool_names) = pending_grounding_check.take()
                    && !tool_names.is_empty()
                {
                    let text = response.text_content();
                    let admits_no_data = {
                        let lower = text.to_ascii_lowercase();
                        [
                            "don't have",
                            "do not have",
                            "no access to",
                            "couldn't find",
                            "could not find",
                        ]
                        .iter()
                        .any(|phrase| lower.contains(phrase))
                    };
                    if !text.contains("\"semantic_type\"") && !admits_no_data {
                        grounding_repair_attempted = true;
                        if turn + 1 >= self.config.max_turns {
                            return TurnOutcome::MaxTurnsReached;
                        }
                        let tool_list = tool_names.join(", ");
                        let _ = self.session.lock().await.append_message(MessageRecord::control(vak_intent::control::ControlKind::GroundingCheck, format!(
                                "[grounding-check]: Your last answer didn't cite the results from {tool_list}, which you just called. \
                                 Either synthesize those results into a structured card that cites them (e.g. a \
                                 `research.synthesis` vak-fence with real sources/URLs from the tool output), or, if the \
                                 results genuinely don't answer the question, say so explicitly instead of writing a vague \
                                 unsourced summary. Please redo your answer now."
                            )));
                        turn += 1;
                        continue;
                    }
                }
                // Malformed-fence enforcement: the model emitted an explicit
                // vak-tagged card fence, but its JSON body doesn't parse
                // (mismatched brackets, an unquoted key, etc.) — a real,
                // observed failure mode from small/local models. Rather
                // than letting a broken card reach the user (where it
                // degrades to a "could not be rendered" notice at best),
                // give the model one bounded repair turn naming the exact
                // parse error, mirroring the grounding-check pattern above.
                if !malformed_fence_repair_attempted {
                    let text = response.text_content();
                    if let Some(parse_error) = find_malformed_vak_fence(&text) {
                        malformed_fence_repair_attempted = true;
                        if turn + 1 >= self.config.max_turns {
                            return TurnOutcome::MaxTurnsReached;
                        }
                        let _ = self.session.lock().await.append_message(MessageRecord::control(vak_intent::control::ControlKind::FenceCheck, format!(
                                "[fence-check]: The vak-fence in your last answer has invalid JSON and failed to parse \
                                 ({parse_error}). Resend the same answer with a syntactically valid JSON body this time — \
                                 double-check every object/array is closed and every key is quoted. If you can't produce \
                                 valid JSON for it, drop the fence and answer in plain prose instead."
                            )));
                        turn += 1;
                        continue;
                    }
                }
                // Duplicate-card enforcement: the model already emitted a
                // card via `emit_*_card` this turn, then its own trailing
                // text repeats the same semantic_type as a `vak` fence —
                // that fence renders as a SECOND card (vak-server's
                // projection and the client's own fence-parsing are
                // independent paths; nothing dedupes across them). One
                // bounded repair turn asking the model to drop the
                // redundant fence, mirroring the two checks above.
                if !duplicate_card_repair_attempted
                    && let Some(emitted_types) = pending_duplicate_card_check.take()
                    && !emitted_types.is_empty()
                {
                    let text = response.text_content();
                    if let Some(dup_type) = find_duplicate_card_fence(&text, &emitted_types) {
                        duplicate_card_repair_attempted = true;
                        if turn + 1 >= self.config.max_turns {
                            return TurnOutcome::MaxTurnsReached;
                        }
                        let _ = self.session.lock().await.append_message(MessageRecord::control(vak_intent::control::ControlKind::DuplicateCardCheck, format!(
                                "[duplicate-card-check]: You already emitted a `{dup_type}` card via the matching \
                                 emit_*_card tool call above, and the user already sees it. Resend your answer \
                                 WITHOUT the ```vak fence that repeats it — just the short narration around the \
                                 card is needed, no restated JSON."
                            )));
                        turn += 1;
                        continue;
                    }
                }
                // Presentation check: the answer reads as something the app
                // presents as a card (the app's own signal/recipe detection,
                // supplied by Core), yet no card was emitted and none is
                // written inline — the model answered in prose. One bounded
                // nudge; the model may decline by resending unchanged.
                if !presentation_repair_attempted
                    && !cards_emitted_this_run
                    && let Some(check) = &self.config.presentation_check
                {
                    let text = response.text_content();
                    if !text.trim().is_empty() && !text.contains("```vak") {
                        let offered: Vec<String> = self
                            .config
                            .tools
                            .iter()
                            .map(|t| t.name().to_string())
                            .collect();
                        if let Some(nudge) = check(&text, &offered) {
                            presentation_repair_attempted = true;
                            if turn + 1 >= self.config.max_turns {
                                return TurnOutcome::MaxTurnsReached;
                            }
                            // Required card not emitted: evidence the request
                            // may already be past this model's real
                            // instruction-following horizon (§1, §6).
                            self.record_capacity_instruction_failure(response.usage.input_tokens)
                                .await;
                            let _ =
                                self.session
                                    .lock()
                                    .await
                                    .append_message(MessageRecord::control(
                                        vak_intent::control::ControlKind::PresentationCheck,
                                        nudge,
                                    ));
                            turn += 1;
                            continue;
                        }
                    }
                }
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
                        let _ = self
                            .session
                            .lock()
                            .await
                            .append_message(MessageRecord::control(
                                vak_intent::control::ControlKind::StopHook,
                                format!("[stop-hook]: {reason}\nPlease continue."),
                            ));
                        turn += 1;
                        continue;
                    }
                }
                if let Some(reason) = self
                    .stop_gate(
                        &prompt_owned,
                        &response,
                        &receipts,
                        verification_stale,
                        &mut stop_blocks_left,
                        user_completion_released,
                    )
                    .await
                {
                    // Stop-policy block: the model tried to end the turn
                    // prematurely against an explicit completion
                    // requirement (§1, §6).
                    self.record_capacity_instruction_failure(response.usage.input_tokens)
                        .await;
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
                // Fence-path presentations (docs/design/68-context-engine.md
                // §10): only for the answer actually being accepted — every
                // gate above has already passed, so this text will not be
                // redone. A repair-nudged draft never reaches here.
                if let Some(entry_id) = &response_entry_id {
                    self.write_fence_presentations(&response.text_content(), entry_id)
                        .await;
                }
                return TurnOutcome::Completed { response };
            }

            for call in &calls {
                receipts.total_tool_calls += 1;
                if call.name == "bash" {
                    let is_subst = call
                        .input
                        .get("command")
                        .and_then(|v| v.as_str())
                        .map(stop_policy::is_substantive_command)
                        .unwrap_or(true);
                    if is_subst {
                        receipts.substantive_bash_calls += 1;
                    }
                } else if matches!(
                    call.name.as_str(),
                    "write" | "edit" | "patch" | "remember" | "propose_skill"
                ) {
                    receipts.files_modified += 1;
                    let path = call.input.get("path").and_then(|v| v.as_str());
                    if let Some(p) = path {
                        if stop_policy::is_code_path(p) {
                            receipts.code_files_modified += 1;
                        } else {
                            receipts.doc_files_modified += 1;
                        }
                    } else {
                        receipts.doc_files_modified += 1;
                    }
                } else if matches!(
                    call.name.as_str(),
                    "read"
                        | "read_file"
                        | "glob"
                        | "grep"
                        | "inspect"
                        | "browse"
                        | "webfetch"
                        | "session_search"
                        | "search"
                        | "session_list"
                ) {
                    receipts.read_or_inspected += 1;
                }
            }
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
            let code_mutation_ids: Vec<String> = calls
                .iter()
                .filter(|call| matches!(call.name.as_str(), "edit" | "write" | "patch"))
                .filter(|call| {
                    call.input
                        .get("path")
                        .and_then(|v| v.as_str())
                        .map(stop_policy::is_code_path)
                        .unwrap_or(false)
                })
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
            let call_inputs: HashMap<String, serde_json::Value> = calls
                .iter()
                .map(|c| (c.id.clone(), c.input.clone()))
                .collect();
            // The order the model actually issued these calls in, captured
            // before `execute_batch` (which may run calls concurrently and
            // return `results` in completion order, not issue order).
            // `verification_stale` below needs issue order specifically:
            // "ran bash after editing code" and "edited code after running
            // bash" are different situations even if both calls land in the
            // same batch and finish in the opposite order.
            let call_issue_order: Vec<String> = calls.iter().map(|c| c.id.clone()).collect();
            // Two card gates at the earliest point, before execution
            // (docs/design/68 §7). Freshness: a card in a live-data turn
            // with nothing retrieved yet would show a carried-over figure.
            // Topic mismatch: a card whose own payload shares no topic word
            // with the directive is unrelated to what was asked, whatever
            // it claims to be derived from — found live, a card correctly
            // linked to a real, on-topic search result still carried an
            // entirely different topic's payload, copied from an older
            // turn's own tool call still sitting in context. Each gated
            // call gets an error value naming which check refused it;
            // either one gets exactly one repair before failing closed.
            enum CardGate {
                Fresh,
                Topic,
            }
            let mut gated: Vec<(PendingToolCall, CardGate)> = Vec::new();
            let calls: Vec<PendingToolCall> = calls
                .into_iter()
                .filter(|call| {
                    if !self.tool_presents_cards(&call.name) {
                        return true;
                    }
                    if wants_live_data && !retrieval_succeeded_this_run {
                        gated.push((call.clone(), CardGate::Fresh));
                        return false;
                    }
                    // Scoped to "a retrieval actually succeeded this run":
                    // that is the one circumstance the real bug needs and
                    // the only one this check can safely judge. Many
                    // legitimate cards have sparse, structural payloads with
                    // no vocabulary of their own at all (an empty chart
                    // skeleton, a bare numeric metric) and share no word
                    // with any directive whether they are right or wrong —
                    // checked live, this exact shape broke a real,
                    // previously-passing test. Only when the model has just
                    // retrieved something is "the card is unrelated to
                    // both the question and what was found" a signal worth
                    // trusting; a card built from the model's own reasoning
                    // or from data already in the directive gets no such
                    // check; the evidence text (far richer than the terse
                    // question) is what lets a correctly-derived card that
                    // renames or paraphrases what was found still pass even
                    // with zero overlap against the directive alone.
                    if let Some(evidence) = &last_evidence_snippet {
                        let topic_context = format!("{prompt_owned} {evidence}");
                        if !card_shares_a_topic_with(&topic_context, &call.name, &call.input) {
                            gated.push((call.clone(), CardGate::Topic));
                            return false;
                        }
                    }
                    true
                })
                .collect();
            if gated
                .iter()
                .any(|(_, gate)| matches!(gate, CardGate::Fresh))
            {
                if freshness_repair_attempted {
                    // The repair was another carried-over card (measured
                    // live: "New Delhi 29.1°C" gated, then "Noida 28°C"
                    // from an older turn offered instead). Fail closed:
                    // no stale figure is presented as current.
                    return self.stale_data_outcome().await;
                }
                freshness_repair_attempted = true;
            }
            if gated
                .iter()
                .any(|(_, gate)| matches!(gate, CardGate::Topic))
            {
                if topic_repair_attempted {
                    return self
                        .topic_mismatch_outcome(last_evidence_snippet.clone())
                        .await;
                }
                topic_repair_attempted = true;
            }
            let mut results = self.execute_batch(calls, &cancel, &events).await;
            results.extend(gated.into_iter().map(|(call, gate)| {
                let message = match gate {
                    CardGate::Fresh => {
                        "[freshness-check]: not shown — this asks for a value as it stands now and \
                         nothing has been retrieved on this turn, so the card would carry a figure \
                         from an earlier answer. Call a retrieval tool first and build the card from \
                         what it returns, or say plainly that you have no live data."
                    }
                    CardGate::Topic => {
                        "[topic-mismatch]: not shown — this card's own content has nothing to do \
                         with what was asked. Build the card from what the current directive and \
                         this turn's own tool results actually say, not from an earlier turn's data."
                    }
                };
                (call.id, ToolRunOutput::Err(message.into()))
            }));
            self.record_worker_work(&task_assignments, &results).await;
            // Identical-card repeat breaker: the no-op ack ("already
            // displayed") is enough for a model that reads it; one that
            // re-emits the same card anyway (measured live: up to nineteen
            // times in one turn) would otherwise spend the whole turn budget
            // on acks. The card it keeps re-emitting IS its answer, so after
            // CARD_REPEAT_EXHAUSTION_THRESHOLD consecutive all-repeat batches
            // the turn closes on that answer.
            let all_repeats = !results.is_empty()
                && results.iter().all(|(_, output)| {
                    matches!(output, ToolRunOutput::Ok(text) if text.starts_with(CARD_REPEAT_ACK))
                });
            card_repeat_streak = if all_repeats {
                card_repeat_streak + 1
            } else {
                0
            };
            if card_repeat_streak >= CARD_REPEAT_EXHAUSTION_THRESHOLD {
                return self.card_repeat_outcome().await;
            }
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
            // Recomputed every batch (not accumulated) so the grounding
            // check below only ever looks at the IMMEDIATELY preceding
            // turn's retrieval calls, matching the observed bug shape
            // (search succeeds, the very next answer ignores it).
            let retrieval_tool_names: Vec<String> = results
                .iter()
                .filter_map(|(id, out)| match out {
                    ToolRunOutput::Ok(_) => {
                        let name = call_names.get(id).map(|s| s.as_str()).unwrap_or("tool");
                        let input = call_inputs.get(id)?;
                        self.config
                            .retrieval_check
                            .as_ref()
                            .is_some_and(|check| check(name, input))
                            .then(|| name.to_string())
                    }
                    ToolRunOutput::Err(_) => None,
                })
                .collect();
            if !retrieval_tool_names.is_empty() {
                retrieval_succeeded_this_run = true;
                if let Some((_, ToolRunOutput::Ok(content))) = results.iter().find(|(id, out)| {
                    matches!(out, ToolRunOutput::Ok(_))
                        && call_names
                            .get(id)
                            .is_some_and(|name| retrieval_tool_names.contains(name))
                }) {
                    last_evidence_snippet = Some(truncate_chars(content, RESULT_PREVIEW_LIMIT));
                }
            }
            pending_grounding_check = if retrieval_tool_names.is_empty() {
                None
            } else {
                Some(retrieval_tool_names)
            };
            let emitted_card_types: Vec<String> = results
                .iter()
                .filter_map(|(id, out)| match out {
                    ToolRunOutput::Ok(_) => {
                        let name = call_names.get(id).map(|s| s.as_str()).unwrap_or("");
                        self.tool_presents_cards(name)
                            .then(|| call_inputs.get(id))
                            .flatten()
                            .and_then(|input| input.get("semantic_type"))
                            .and_then(|s| s.as_str())
                            .map(str::to_string)
                    }
                    ToolRunOutput::Err(_) => None,
                })
                .collect();
            cards_emitted_this_run |= !emitted_card_types.is_empty();
            pending_duplicate_card_check = if emitted_card_types.is_empty() {
                None
            } else {
                Some(emitted_card_types)
            };
            // Presentations are ledger entries
            // (docs/design/68-context-engine.md §10): a validated
            // `emit_*_card` call gets its own hash-linked entry, written
            // HERE rather than inside the tool itself — the tool executes
            // across the worker/broker boundary (AGENTS.md invariant 14)
            // and has no session-log access. `execute()`'s generic ack is
            // replaced with a short one carrying the new entry's id.
            if let Some(rebuild) = self.config.presentation_rebuild.clone() {
                let mut session = self.session.lock().await;
                if let Some(turn_id) = session.latest_directive_entry_id() {
                    let prior_evidence = session
                        .non_card_evidence_since(&turn_id, |name| self.tool_presents_cards(name));
                    let mut in_batch_evidence: Vec<String> = Vec::new();
                    for id in &call_issue_order {
                        let Some(name) = call_names.get(id) else {
                            continue;
                        };
                        let succeeded_here = results
                            .iter()
                            .any(|(rid, out)| rid == id && matches!(out, ToolRunOutput::Ok(_)));
                        if !succeeded_here {
                            continue;
                        }
                        if !self.tool_presents_cards(name) {
                            in_batch_evidence.push(id.clone());
                            continue;
                        }
                        let Some(input) = call_inputs.get(id) else {
                            continue;
                        };
                        let Some(info) = rebuild(name.as_str(), input) else {
                            continue;
                        };
                        let digest = vak_session::types::payload_digest(&info.payload);
                        if session.has_presentation(&turn_id, &digest) {
                            // Already recorded — a repeated identical call
                            // (`execute_batch`'s own short-circuit reuses the
                            // exact same arguments) or a fence that beat this
                            // write to it. Nothing new to append; the tool's
                            // own result text (ack or "already displayed")
                            // stands.
                            continue;
                        }
                        let mut derived_from = prior_evidence.clone();
                        derived_from.extend(in_batch_evidence.iter().cloned());
                        let record = vak_session::types::PresentationRecord {
                            turn_id: turn_id.clone(),
                            source: vak_session::types::PresentationSource::ToolCall {
                                tool_use_id: id.clone(),
                            },
                            semantic_type: info.semantic_type,
                            skill_id: info.skill_id,
                            skill_version: info.skill_version,
                            schema_version: info.schema_version,
                            payload: info.payload,
                            payload_digest: digest,
                            derived_from,
                            title: info.title,
                            identity_digest: info.identity_digest,
                        };
                        if let Ok(entry) = session.append_presentation(record)
                            && let Some(slot) = results.iter_mut().find(|(rid, _)| rid == id)
                        {
                            slot.1 = ToolRunOutput::Ok(
                                serde_json::json!({"presentation": entry.id, "ok": true})
                                    .to_string(),
                            );
                        }
                    }
                }
            }
            for (id, out) in &results {
                match out {
                    ToolRunOutput::Ok(_) => {
                        receipts.successful_tool_calls += 1;
                        if let Some((_, cmd)) = bash_pairs.iter().find(|(bid, _)| bid == id)
                            && !self.obligations.iter().any(|o| o == cmd)
                        {
                            self.obligations.push(cmd.clone());
                        }
                    }
                    ToolRunOutput::Err(_) => {
                        receipts.failed_tool_calls += 1;
                    }
                }
            }
            // `verification_stale` used to be flipped inline in the loop
            // above, which made it depend on `results`' iteration order —
            // the order tools finished, not the order the model issued
            // them in (this agent does run tool calls within a batch
            // concurrently when `config.parallel_tools` is set, so this was
            // reachable, not just theoretical). See
            // `resolve_verification_stale` for the order-correct logic,
            // tested in isolation below.
            let succeeded: std::collections::HashSet<&str> = results
                .iter()
                .filter(|(_, out)| matches!(out, ToolRunOutput::Ok(_)))
                .map(|(id, _)| id.as_str())
                .collect();
            let bash_ids: Vec<&str> = bash_pairs.iter().map(|(id, _)| id.as_str()).collect();
            let mutation_ids: Vec<&str> = code_mutation_ids.iter().map(|id| id.as_str()).collect();
            verification_stale = resolve_verification_stale(
                &call_issue_order,
                &bash_ids,
                &mutation_ids,
                &succeeded,
                verification_stale,
            );
            // `unresolved_error` used to be set/cleared per-result inside the
            // loop above, which meant a later call in the SAME batch that
            // happened to succeed would silently erase an earlier call's
            // failure (order-dependent on `results`, not on whether the
            // failure was actually resolved). A model that fails one call
            // and succeeds at an unrelated trailing call in the same turn
            // could then claim total success next turn with `stop_gate`
            // never seeing the failure at all. Decide this once, after the
            // whole batch, from the batch's own outcome: any error in this
            // batch wins (first one, in issued order) over any success in
            // the same batch; only a batch with NO errors clears a
            // previous batch's still-unresolved failure.
            let batch_error = results.iter().find_map(|(id, out)| match out {
                ToolRunOutput::Err(err) => Some((
                    call_names.get(id).cloned().unwrap_or_else(|| id.clone()),
                    err.clone(),
                )),
                ToolRunOutput::Ok(_) => None,
            });
            receipts.unresolved_error = batch_error.or_else(|| {
                if results
                    .iter()
                    .all(|(_, out)| matches!(out, ToolRunOutput::Ok(_)))
                {
                    None
                } else {
                    receipts.unresolved_error.clone()
                }
            });
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

    /// Detects ```` ```vak ```` fences in the accepted final answer and
    /// writes a `Presentation` entry for each one that validates through
    /// `AgentConfig::presentation_rebuild` and is not a duplicate of a card
    /// already recorded for this turn — the inline-fence fallback for
    /// models without tool calling (docs/design/68-context-engine.md §10).
    /// `message_entry_id` is the ledger entry id of the assistant message
    /// that carried the fence text. A fence whose `semantic_type` cannot be
    /// mapped to a known card, or that fails validation, is silently
    /// skipped — the malformed-fence repair nudge (above, in the caller)
    /// already handles the "unparseable JSON" case separately.
    async fn write_fence_presentations(&self, text: &str, message_entry_id: &str) {
        let Some(rebuild) = self.config.presentation_rebuild.clone() else {
            return;
        };
        for body in vak_fence_bodies(text) {
            let Ok(fence_json) = serde_json::from_str::<Value>(body.trim()) else {
                continue;
            };
            let Some(semantic_type) = fence_json.get("semantic_type").and_then(Value::as_str)
            else {
                continue;
            };
            // `name` is a best-effort hint: the hook's own implementation
            // (vak-core) knows how to map `semantic_type` to the matching
            // `emit_*_card` tool via `presentation_tools::emit_tool_for`
            // when that mapping is reachable; passing `semantic_type` here
            // keeps this call meaningful even when it is not.
            let Some(info) = rebuild(semantic_type, &fence_json) else {
                continue;
            };
            let digest = vak_session::types::payload_digest(&info.payload);
            let mut session = self.session.lock().await;
            let Some(turn_id) = session.latest_directive_entry_id() else {
                continue;
            };
            if session.has_presentation(&turn_id, &digest) {
                // Duplicate of a card already recorded this turn (by tool
                // call or an earlier fence) — dropped from the projection.
                continue;
            }
            let derived_from =
                session.non_card_evidence_since(&turn_id, |name| self.tool_presents_cards(name));
            let record = vak_session::types::PresentationRecord {
                turn_id,
                source: vak_session::types::PresentationSource::Fence {
                    message_entry_id: message_entry_id.to_string(),
                },
                semantic_type: info.semantic_type,
                skill_id: info.skill_id,
                skill_version: info.skill_version,
                schema_version: info.schema_version,
                payload: info.payload,
                payload_digest: digest,
                derived_from,
                title: info.title,
                identity_digest: info.identity_digest,
            };
            let _ = session.append_presentation(record);
        }
    }

    /// Builds and appends this turn's `TurnCard` (docs/design/68-context-
    /// engine.md §10) once it has actually closed. Idempotent: a turn
    /// already carrying a card (`TurnIndex` rebuilds `turn.card` from any
    /// existing `TurnCard` entry) is left alone, since a card is written
    /// once and never rewritten.
    async fn close_turn(
        &mut self,
        outcome: &TurnOutcome,
        cancel: &CancellationToken,
        events: &mpsc::Sender<AgentEvent>,
    ) {
        let outcome_label = match outcome {
            TurnOutcome::Completed { .. } => "completed",
            TurnOutcome::Aborted { .. } => "cancelled",
            TurnOutcome::Failed { .. } => "failed",
            TurnOutcome::MaxTurnsReached => "degraded",
        };
        let Some((turn_id, raw_narration)) = ({
            let session = self.session.lock().await;
            let index = TurnIndex::from_log(&session);
            index.turns.last().and_then(|turn| {
                (turn.closed && turn.card.is_none()).then(|| {
                    let narration = turn
                        .final_answer
                        .as_ref()
                        .map(Message::text_content)
                        .unwrap_or_default();
                    (turn.id.clone(), narration)
                })
            })
        }) else {
            return;
        };
        let narration = self.resolve_narration(&raw_narration, cancel, events).await;
        // No profile wired in ⇒ a metadata-only one
        // (docs/design/68-context-engine.md §4).
        let profile = self.effective_capacity_profile();
        let estimate = move |s: &str| -> u64 { profile.estimate_tokens(s.chars().count() as u64) };
        let tokens_full = {
            let mut session = self.session.lock().await;
            let index = TurnIndex::from_log(&session);
            let Some(turn) = index.turn_by_id(&turn_id) else {
                return;
            };
            if turn.card.is_some() {
                return; // written concurrently between the two locks above
            }
            let card = turn.build_card(outcome_label, narration, &estimate);
            let tokens_full = card.tokens_full;
            let _ = session.append_turn_card(vak_session::types::TurnCardRecord { turn_id, card });
            tokens_full
        };
        // Feeds the planner's reserve for the NEXT open turn (§4); a no-op
        // when no live profile is wired in (nothing to persist it on).
        if let Some(profile) = self.config.capacity.as_mut() {
            profile.observe_current_turn_tokens(tokens_full);
        }
    }

    /// Resolves the caller-visible narration for a `TurnCard`: verbatim when
    /// short (≤ 60 words), otherwise one side call on the configured model
    /// with ONLY the narration as input — never the turn's history — asking
    /// for a one-sentence gist (docs/design/68-context-engine.md §10). On a
    /// dispatch error, falls back to the narration's own first sentence
    /// rather than failing turn close over a summarizer hiccup.
    async fn resolve_narration(
        &self,
        narration: &str,
        cancel: &CancellationToken,
        events: &mpsc::Sender<AgentEvent>,
    ) -> String {
        if narration.split_whitespace().count() <= 60 {
            return narration.to_string();
        }
        let model = self.config.model.clone();
        let mut request = ChatRequest::new(model.clone());
        request.system = Some(
            "Give a one-sentence gist of the following text. Output only that sentence, \
             nothing else."
                .to_string(),
        );
        request.messages = vec![Message::user_text(narration.to_string())];
        request.max_tokens = 128;
        let mut ledger = StepLedger::new(
            WorkPurpose::Summarize,
            self.provider.name(),
            &model,
            self.config.dispatch_ceiling,
        );
        let result = self
            .complete_with_reliability(&request, cancel, events, false, &mut ledger)
            .await;
        {
            let mut session = self.session.lock().await;
            let _ = session.append_receipt(ledger.take_receipt());
        }
        match result {
            Ok(message) => {
                let gist = message.text_content();
                let gist = gist.trim();
                if gist.is_empty() {
                    first_sentence_fallback(narration)
                } else {
                    gist.to_string()
                }
            }
            Err(_) => first_sentence_fallback(narration),
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
        // config.model reflects the per-turn effective route set by run_turn_inner.
        let model = self.config.model.clone();
        let authoring_request = ChatRequest {
            model: model.clone(),
            system: Some("You author durable work contracts. Return only one strict JSON object with keys objective, constraints, assumptions, criteria, and items. Each item must have item_id, title, instructions, dependencies, owner, required, readonly, path_claims, and criterion_ids. Owner must be one of parent_agent, worker, flow, tool, or human. Criterion kind must be one of shell, file_exists, file_contains, tool_succeeded, flow_completed, external_receipt, or semantic. Do not include markdown or commentary.".into()),
            messages: vec![Message::user_text(prompt)],
            tools: Vec::new(),
            max_tokens: self.config.max_output.min(8_000) as u32,
            temperature: None,
            cache: None,
            previous_response_id: None,
            think: None,
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
            agent_id: None,
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
        // config.model reflects the per-turn effective route set by run_turn_inner.
        let model = self.config.model.clone();
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
        // config.model reflects the per-turn effective route set by run_turn_inner.
        let model = self.config.model.clone();
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
        receipts: &stop_policy::ReceiptSummary,
        verification_stale: bool,
        blocks_left: &mut u32,
        user_completion_released: bool,
    ) -> Option<String> {
        let policy = self.config.stop_policy.as_ref()?;
        if user_completion_released {
            return None;
        }
        let reason = policy.evaluate_receipts(
            prompt,
            &response.text_content(),
            self.config.outcome.as_ref(),
            receipts,
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
        let _ = self
            .session
            .lock()
            .await
            .append_message(MessageRecord::control(
                vak_intent::control::ControlKind::StopGuard,
                format!("[stop-guard]: {reason}\nPlease continue."),
            ));
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

    /// Whether `name` is one of this agent's tools and it declares that a
    /// successful call presents a card (`Tool::presents_cards`).
    fn tool_presents_cards(&self, name: &str) -> bool {
        self.config
            .tools
            .iter()
            .any(|tool| tool.name() == name && tool.presents_cards())
    }

    /// Appends the model's response and returns its ledger entry id, so
    /// callers that need to attribute something back to this exact message
    /// (a fence-path `Presentation`, docs/design/68-context-engine.md §10)
    /// don't have to re-derive it. `None` only on a session write failure.
    async fn append_assistant(&self, response: &AssistantMessage) -> Option<String> {
        let mut session = self.session.lock().await;
        session
            .append_message(MessageRecord {
                message: response.clone().into_message(),
                meta: Some(MessageMeta {
                    model: Some(response.model.clone()),
                    stop_reason: Some(format!("{:?}", response.stop_reason).to_lowercase()),
                    usage: Some(response.usage.clone()),
                    control: None,
                    ..Default::default()
                }),
            })
            .ok()
            .map(|entry| entry.id)
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

    /// Serializes a `CapacityProfile` and its key into an `Activity`'s
    /// `data` map, the shape `SessionLog::latest_capacity_profile` reads
    /// back (docs/design/68-context-engine.md §1, §4).
    fn capacity_activity_data(
        &self,
        profile: &CapacityProfile,
    ) -> std::collections::BTreeMap<String, String> {
        let mut data = std::collections::BTreeMap::new();
        let key = self
            .config
            .capacity_key
            .clone()
            .unwrap_or_else(|| capacity::ProfileKey {
                provider: self.provider.name().to_string(),
                model: self.config.model.clone(),
                quantisation: profile.provenance.quantisation.clone(),
            });
        if let Ok(key_json) = serde_json::to_string(&key) {
            data.insert("key".into(), key_json);
        }
        if let Ok(profile_json) = serde_json::to_string(profile) {
            data.insert("profile".into(), profile_json);
        }
        data
    }

    /// Folds one turn's real usage into `self.config.capacity` (§1
    /// "Feedback") and records a `capacity-feedback` activity when a field
    /// moved by more than `CAPACITY_FEEDBACK_CHANGE_THRESHOLD`. A no-op
    /// when no profile was wired in for this run.
    async fn record_capacity_usage_feedback(
        &mut self,
        request: &ChatRequest,
        usage: &Usage,
        first_token_latency_ms: Option<u64>,
    ) {
        let Some(profile) = self.config.capacity.as_mut() else {
            return;
        };
        let before = profile.clone();
        let chars_sent = chat_request_chars(request);
        let cache_miss = usage.cache_read_input_tokens.unwrap_or(0) == 0;
        profile.observe_usage(chars_sent, usage, first_token_latency_ms, cache_miss);
        let after = profile.clone();
        let delta = capacity_feedback_delta(&before, &after);
        if delta.is_empty() {
            return;
        }
        let mut data = self.capacity_activity_data(&after);
        data.extend(delta);
        self.record_activity(
            vak_session::ActivityKind::CapacityFeedback,
            vak_session::ActivityStatus::Succeeded,
            "Capacity profile updated from usage".into(),
            None,
            data,
        )
        .await;
    }

    /// Folds an explicit-instruction failure (required card not emitted,
    /// required tool not called, a stop-policy block — the runtime already
    /// classifies each) into `self.config.capacity` (§1 "Horizon
    /// tightening") and records the change. A no-op when no profile was
    /// wired in, or when the request was far from the current horizon.
    async fn record_capacity_instruction_failure(&mut self, request_tokens: u64) {
        let Some(profile) = self.config.capacity.as_mut() else {
            return;
        };
        let before = profile.clone();
        profile.observe_instruction_failure(request_tokens);
        let after = profile.clone();
        if before.instruction_horizon.tokens == after.instruction_horizon.tokens {
            return;
        }
        let mut data = self.capacity_activity_data(&after);
        data.extend(capacity_feedback_delta(&before, &after));
        self.record_activity(
            vak_session::ActivityKind::CapacityFeedback,
            vak_session::ActivityStatus::Succeeded,
            "Capacity horizon lowered by instruction-following feedback".into(),
            Some(format!("request was {request_tokens} tokens")),
            data,
        )
        .await;
    }

    /// Model drift (docs/design/68-context-engine.md §7): the step's own
    /// evidence, not a guess. Checks, in order: (a) a called tool whose
    /// declared domains are disjoint from the current reading's — only when
    /// the reading actually HAS domains, so a general/undeclared reading
    /// never flags every call; (b) for a final answer, an exact match
    /// against a prior turn's card narration — a verbatim repeat of a past
    /// answer instead of addressing the current one. Returns the drift
    /// reason for the nudge, or `None`.
    async fn detect_model_drift(
        &self,
        response: &AssistantMessage,
        calls: &[PendingToolCall],
    ) -> Option<String> {
        let (reading, past_narrations) = {
            let session = self.session.lock().await;
            let reading = session.latest_reading();
            let index = TurnIndex::from_log(&session);
            let narrations: Vec<String> = index
                .turns
                .iter()
                .filter_map(|turn| turn.card.as_ref())
                .map(|card| card.answered.narration.trim().to_string())
                .filter(|narration| !narration.is_empty())
                .collect();
            (reading, narrations)
        };
        if let Some(reading) = &reading
            && !reading.domains.is_empty()
        {
            for call in calls {
                let Some(domains) = self.config.tool_domains.get(&call.name) else {
                    continue;
                };
                if domains.is_empty() {
                    continue;
                }
                if domains.iter().all(|d| !reading.domains.contains(d)) {
                    return Some(format!(
                        "called `{}` (domains: {}), which serves none of the current directive's domains ({})",
                        call.name,
                        domains.join(", "),
                        reading.domains.join(", ")
                    ));
                }
            }
        }
        if calls.is_empty() {
            let text = response.text_content();
            let trimmed = text.trim();
            if !trimmed.is_empty() && past_narrations.iter().any(|n| n == trimmed) {
                return Some(
                    "repeated a previous turn's answer verbatim instead of addressing the current directive"
                        .to_string(),
                );
            }
        }
        None
    }

    /// The degraded, honest completion returned when model drift exhausts
    /// its steering budget (docs/design/68-context-engine.md §7) — same
    /// shape as `degraded_outcome`'s tool-repair exhaustion
    /// (docs/design/15-reliability.md), a different diagnostic label.
    async fn degraded_drift_outcome(&self, last_reason: &str) -> TurnOutcome {
        self.record_activity(
            vak_session::ActivityKind::Diagnostic,
            vak_session::ActivityStatus::Failed,
            "model-drift-exhausted".into(),
            Some(format!(
                "three consecutive steps served a different directive than the current one; \
                 run stopped rather than continuing to answer the wrong request. Last: {last_reason}"
            )),
            std::collections::BTreeMap::new(),
        )
        .await;
        let response = AssistantMessage {
            content: vec![ContentBlock::text(
                "I kept drifting away from your current request across several steps and \
                 could not stay on it within this turn's steering budget. Please restate what \
                 you need now, or narrow the request, and I will address it directly."
                    .to_string(),
            )],
            stop_reason: StopReason::EndTurn,
            usage: Usage::default(),
            model: self.config.model.clone(),
            response_id: None,
        };
        TurnOutcome::Completed { response }
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
            // Per-leg tool inclusion (docs/design/68-context-engine.md §5):
            // only Anthropic legs get the deferred schemas (withheld from
            // the prefix there via `defer_loading`); every other provider
            // sees core only, since its tool index is already in the
            // prefix and `find_tools` is how it reaches the rest.
            leg_req.tools = tools_for_leg(&request.tools, &route_provider);
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
                    let profile = self.effective_capacity_profile();
                    let est_input = profile.estimate_tokens(
                        messages_chars(&leg_req.messages)
                            + prefix_chars(leg_req.system.as_deref().unwrap_or(""), &leg_req.tools),
                    );
                    let check = SpendCheck {
                        model,
                        provider: provider_arc.name(),
                        session_id: &session_id,
                        est_input_tokens: est_input,
                        planned_output_tokens: self.config.max_output,
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
                    // First-token latency, wall clock from just before
                    // `.stream()` was called to the first event off the
                    // wire. Ollama fills `usage.prefill_ms` itself
                    // (preferred when present); this is what lets every
                    // other provider feed `prefill_tps` too
                    // (docs/design/68-context-engine.md §1 "Feedback").
                    let mut first_token_ms: Option<u64> = None;
                    while let Some(ev) = futures::StreamExt::next(&mut stream).await {
                        if first_token_ms.is_none() {
                            first_token_ms = Some(started.elapsed().as_millis() as u64);
                        }
                        if forward && events.send(AgentEvent::Stream(ev)).await.is_err() {
                            cancel.cancel();
                        }
                    }
                    stream
                        .result()
                        .await
                        .map(|message| (message, first_token_ms))
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
                    Ok((r, first_token_ms)) => {
                        if let Some(breaker) = &self.config.circuit_breaker {
                            breaker.record_success_key(&breaker_key);
                        }
                        ledger.last_first_token_ms = first_token_ms;
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

    /// Runs a batch, except that a card the user already sees is not shown
    /// again. A card call is on screen after its first success, so an identical
    /// one (a small model repeats it until a breaker fires) is answered with a
    /// plain ack instead of being run — never an error, which the stop guard
    /// would count as an unresolved failure and answer with another model turn.
    async fn execute_batch(
        &self,
        calls: Vec<PendingToolCall>,
        cancel: &CancellationToken,
        events: &mpsc::Sender<AgentEvent>,
    ) -> Vec<(String, ToolRunOutput)> {
        // `recall` is answered from the session here, before dispatch —
        // never sent to a worker (docs/design/68-context-engine.md §3/§7).
        // Intercepting by name mirrors how `emit_*_card` results are
        // rewritten after `execute_batch` below, just earlier: `recall` has
        // no side effects to execute, only session state to read, and
        // `ToolContext` carries no session handle for `RecallTool::execute`
        // to use (AGENTS.md invariant 14).
        let (recall_calls, calls): (Vec<_>, Vec<_>) = calls
            .into_iter()
            .partition(|call| vak_tools::canonical_tool_name(&call.name) == "recall");
        let mut results = Vec::with_capacity(recall_calls.len());
        for call in recall_calls {
            let output = self.resolve_recall(&call.input).await;
            results.push((call.id, output));
        }
        results.extend(self.execute_batch_inner(calls, cancel, events).await);
        results
    }

    /// Resolves one `recall` call's arguments against the current session:
    /// `turn` → the resolved turn's full record as text; `presentation` →
    /// the canonical payload; `id` → the evidence content, optionally
    /// sliced by line range. The result is a current-turn tool result and
    /// is verbatim for the rest of that turn like any other result.
    async fn resolve_recall(&self, input: &Value) -> ToolRunOutput {
        let request = match vak_tools::parse_recall_args(input) {
            Ok(request) => request,
            Err(message) => {
                return ToolRunOutput::Err(format!(
                    r#"{{"type":"invalid_arguments","message":"{message}"}}"#
                ));
            }
        };
        let session = self.session.lock().await;
        match request {
            RecallRequest::Turn(n) => {
                let index = TurnIndex::from_log(&session);
                match index.turn_by_number(n as usize) {
                    Some(turn) => ToolRunOutput::Ok(render_full_record(&turn.full_record())),
                    None => ToolRunOutput::Err(format!(
                        r#"{{"type":"invalid_arguments","message":"no turn numbered {n}"}}"#
                    )),
                }
            }
            RecallRequest::Presentation(id) => {
                match session
                    .presentations()
                    .into_iter()
                    .find(|(pid, _)| *pid == id)
                {
                    Some((_, record)) => ToolRunOutput::Ok(record.payload.to_string()),
                    None => ToolRunOutput::Err(format!(
                        r#"{{"type":"invalid_arguments","message":"no presentation {id}"}}"#
                    )),
                }
            }
            RecallRequest::Id { id, range } => match session.evidence(&id) {
                Some(evidence) => {
                    ToolRunOutput::Ok(vak_tools::apply_range(&evidence.content, range))
                }
                None => ToolRunOutput::Err(format!(
                    r#"{{"type":"invalid_arguments","message":"no evidence {id}"}}"#
                )),
            },
        }
    }

    async fn execute_batch_inner(
        &self,
        calls: Vec<PendingToolCall>,
        cancel: &CancellationToken,
        events: &mpsc::Sender<AgentEvent>,
    ) -> Vec<(String, ToolRunOutput)> {
        let key = |call: &PendingToolCall| {
            format!(
                "{}\u{0}{}",
                call.name,
                serde_json::to_string(&call.input).unwrap_or_default()
            )
        };
        let mut shown = self
            .presented_cards
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .clone();
        let mut repeats: Vec<String> = Vec::new();
        let mut live_cards: HashMap<String, String> = HashMap::new();
        let mut live = Vec::with_capacity(calls.len());
        for call in calls {
            if self.tool_presents_cards(&call.name) {
                let card_key = key(&call);
                if !shown.insert(card_key.clone()) {
                    repeats.push(call.id.clone());
                    continue;
                }
                live_cards.insert(call.id.clone(), card_key);
            }
            live.push(call);
        }
        let mut results = self.execute_batch_calls(live, cancel, events).await;
        {
            let mut presented = self
                .presented_cards
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            for (id, output) in &results {
                if let (Some(card_key), ToolRunOutput::Ok(_)) = (live_cards.get(id), output) {
                    presented.insert(card_key.clone());
                }
            }
        }
        results.extend(
            repeats
                .into_iter()
                .map(|id| (id, ToolRunOutput::Ok(CARD_REPEAT_ACK.into()))),
        );
        results
    }

    /// The turn's answer when a current value was asked for and nothing
    /// was retrieved after the one repair (docs/design/68 §7): an honest
    /// statement naming the last figure this conversation recorded and
    /// when, never that figure presented as current.
    async fn stale_data_outcome(&self) -> TurnOutcome {
        let last_known = {
            let session = self.session.lock().await;
            let turn_id = session.latest_directive_entry_id();
            let entries = session.chain_to_root();
            session
                .presentations()
                .into_iter()
                .rev()
                .find(|(_, record)| Some(record.turn_id.as_str()) != turn_id.as_deref())
                .map(|(id, record)| {
                    let when = entries
                        .iter()
                        .find(|entry| entry.id == id)
                        .map(|entry| entry.ts.format("%Y-%m-%d %H:%M UTC").to_string())
                        .unwrap_or_else(|| "an earlier turn".to_string());
                    format!(
                        " The most recent figure in this conversation was recorded at {when}: {}.",
                        record.identity_digest
                    )
                })
                .unwrap_or_default()
        };
        self.record_activity(
            vak_session::ActivityKind::Diagnostic,
            vak_session::ActivityStatus::Failed,
            "stale-data-refused".into(),
            Some(
                "a current value was asked for, nothing was retrieved this turn after one repair, \
                 and no carried-over figure was presented as current"
                    .into(),
            ),
            std::collections::BTreeMap::new(),
        )
        .await;
        TurnOutcome::Completed {
            response: AssistantMessage {
                content: vec![ContentBlock::text(format!(
                    "I could not retrieve a current value on this turn, so I am not presenting a \
                     carried-over figure as current.{last_known} Ask again when a retrieval tool \
                     is available, or ask for the last known figure explicitly."
                ))],
                stop_reason: StopReason::EndTurn,
                usage: Usage::default(),
                model: self.config.model.clone(),
                response_id: None,
            },
        }
    }

    /// The turn's answer when a card repeatedly refuses to be about what was
    /// asked (docs/design/68 §7): honest text instead of a wrong card. When
    /// this turn's own retrieval actually succeeded, its raw result is
    /// quoted rather than left out — the evidence exists, only the card
    /// built from it did not.
    async fn topic_mismatch_outcome(&self, evidence: Option<String>) -> TurnOutcome {
        self.record_activity(
            vak_session::ActivityKind::Diagnostic,
            vak_session::ActivityStatus::Failed,
            "topic-mismatch-refused".into(),
            Some(
                "a card was offered twice whose content did not match the directive; the turn \
                 closed on an honest statement instead of showing a wrong card"
                    .into(),
            ),
            std::collections::BTreeMap::new(),
        )
        .await;
        let text = match evidence {
            Some(found) => format!(
                "I could not turn what I found into a card that actually answers this, so here is \
                 the raw result instead:\n\n{found}"
            ),
            None => "I could not produce a card that matches what was asked, and had nothing else \
                     to fall back on this turn. Could you rephrase the question?"
                .to_string(),
        };
        TurnOutcome::Completed {
            response: AssistantMessage {
                content: vec![ContentBlock::text(text)],
                stop_reason: StopReason::EndTurn,
                usage: Usage::default(),
                model: self.config.model.clone(),
                response_id: None,
            },
        }
    }

    /// The turn's answer when the model kept re-emitting a card it had
    /// already shown: the card stands, the loop stops paying for acks.
    async fn card_repeat_outcome(&self) -> TurnOutcome {
        self.record_activity(
            vak_session::ActivityKind::Diagnostic,
            vak_session::ActivityStatus::Succeeded,
            "card-repeat-exhausted".into(),
            Some(format!(
                "{CARD_REPEAT_EXHAUSTION_THRESHOLD} consecutive steps re-emitted an already-shown card; \
                 the turn closed on that card as its answer"
            )),
            std::collections::BTreeMap::new(),
        )
        .await;
        TurnOutcome::Completed {
            response: AssistantMessage {
                content: Vec::new(),
                stop_reason: StopReason::EndTurn,
                usage: Usage::default(),
                model: self.config.model.clone(),
                response_id: None,
            },
        }
    }

    async fn execute_batch_calls(
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
        let agent_id = self
            .session
            .lock()
            .await
            .header()
            .and_then(|h| h.agent.as_ref().map(|a| a.id.clone()));
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
                                agent_id.as_deref(),
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
                let agent_id = agent_id.clone();
                join.spawn(async move {
                    let r = execute_one(
                        call,
                        &tools,
                        &cwd,
                        &session_id,
                        agent_id.as_deref(),
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
                                            (vak_session::types::WorkOwner::Worker, "task", _) => true,
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
                    vak_session::types::WorkOwner::Worker
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

    async fn record_worker_work(
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
                ToolRunOutput::Ok(text) | ToolRunOutput::Err(text) => extract_worker_id(text),
            };
            let revision = projection.contract.revision;
            let assigned = vak_session::types::WorkEvent {
                contract_id: contract_id.clone(),
                revision,
                kind: vak_session::types::WorkEventKind::ItemAssigned {
                    item_id: item_id.clone(),
                    owner: vak_session::types::WorkOwner::Worker,
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
                        reason: "worker returned".into(),
                    },
                });
            }
        }
    }
}

/// Decides `verification_stale` from the model's actual call-issue order,
/// not from whatever order the tool results happened to come back in.
/// `code_mutation_ids` codepaths turn the flag on (an edit to a code path
/// just landed and hasn't been re-verified); `bash_ids` turn it off (a
/// bash run just re-verified, or is at least the most recent evidence).
/// Only the LAST succeeded call, in issue order, that matches either list
/// decides the outcome — everything else in the batch is irrelevant to it.
/// A call that never succeeded doesn't count as either kind of evidence.
fn resolve_verification_stale(
    call_issue_order: &[String],
    bash_ids: &[&str],
    code_mutation_ids: &[&str],
    succeeded: &std::collections::HashSet<&str>,
    current: bool,
) -> bool {
    let mut stale = current;
    for id in call_issue_order {
        if !succeeded.contains(id.as_str()) {
            continue;
        }
        if bash_ids.contains(&id.as_str()) {
            stale = false;
        } else if code_mutation_ids.contains(&id.as_str()) {
            stale = true;
        }
    }
    stale
}

#[cfg(test)]
mod verification_stale_tests {
    use super::resolve_verification_stale;
    use std::collections::HashSet;

    #[test]
    fn edit_issued_after_bash_stays_stale_regardless_of_result_order() {
        // Model issues bash first, then edits a code file — the bash
        // verification is now stale, no matter which result comes back
        // first from a concurrent batch.
        let order = vec!["b1".to_string(), "e1".to_string()];
        let succeeded: HashSet<&str> = ["b1", "e1"].into_iter().collect();
        assert!(resolve_verification_stale(
            &order,
            &["b1"],
            &["e1"],
            &succeeded,
            false
        ));
    }

    #[test]
    fn bash_issued_after_edit_clears_stale_regardless_of_result_order() {
        // Model edits a code file, then runs bash to verify it — no
        // longer stale, no matter which result comes back first.
        let order = vec!["e1".to_string(), "b1".to_string()];
        let succeeded: HashSet<&str> = ["b1", "e1"].into_iter().collect();
        assert!(!resolve_verification_stale(
            &order,
            &["b1"],
            &["e1"],
            &succeeded,
            false
        ));
    }

    #[test]
    fn a_failed_call_is_not_evidence_either_way() {
        // Bash issued after the edit, but the bash call FAILED — the edit
        // is still unverified, so staleness must not clear.
        let order = vec!["e1".to_string(), "b1".to_string()];
        let succeeded: HashSet<&str> = ["e1"].into_iter().collect(); // b1 not in succeeded
        assert!(resolve_verification_stale(
            &order,
            &["b1"],
            &["e1"],
            &succeeded,
            false
        ));
    }

    #[test]
    fn irrelevant_calls_in_the_batch_do_not_affect_the_flag() {
        let order = vec!["e1".to_string(), "r1".to_string()];
        let succeeded: HashSet<&str> = ["e1", "r1"].into_iter().collect();
        assert!(resolve_verification_stale(
            &order,
            &["b1"],
            &["e1"],
            &succeeded,
            false
        ));
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
        && let Some(cmd) = obj
            .get("cmd")
            .or_else(|| obj.get("script"))
            .or_else(|| obj.get("code"))
            .or_else(|| obj.get("input"))
            .cloned()
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
        // Some providers ignore the `oneOf` discriminator and omit `action`.
        // Recover the unambiguous shapes at the broker boundary so a malformed
        // call does not strand an otherwise valid turn: server+tool means call;
        // anything else is the harmless catalog request.
        if let Some(obj) = call.input.as_object_mut()
            && !obj.contains_key("action")
        {
            let has_server = obj
                .get("server")
                .and_then(|v| v.as_str())
                .is_some_and(|s| !s.is_empty());
            let has_tool = obj
                .get("tool")
                .and_then(|v| v.as_str())
                .is_some_and(|s| !s.is_empty());
            obj.insert(
                "action".into(),
                serde_json::Value::String(
                    if has_server && has_tool {
                        "call"
                    } else {
                        "list"
                    }
                    .into(),
                ),
            );
        }
        // Dynamic broker auto-resolution: if the model called `mcp` with `action: "call"`
        // and specified `tool`, but omitted or left `server` empty, resolve `server`
        // dynamically if the tool name uniquely maps to an admitted server in `aliases`.
        if let Some(obj) = call.input.as_object_mut() {
            let is_call = obj.get("action").and_then(|a| a.as_str()) == Some("call");
            let server_missing = obj
                .get("server")
                .is_none_or(|s| s.is_null() || s.as_str() == Some(""));
            if is_call
                && server_missing
                && let Some(tool_name) = obj.get("tool").and_then(|t| t.as_str())
                && let Some(alias) = aliases.get(tool_name)
            {
                obj.insert(
                    "server".into(),
                    serde_json::Value::String(alias.server.clone()),
                );
            }
            if is_call
                && obj
                    .get("server")
                    .and_then(|s| s.as_str())
                    .is_none_or(str::is_empty)
            {
                obj.insert("action".into(), serde_json::Value::String("list".into()));
                obj.remove("server");
                obj.remove("tool");
                obj.remove("arguments");
            }
        }
    }
    call
}

fn extract_worker_id(text: &str) -> Option<String> {
    let prefix = "worker '";
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
    agent_id: Option<&str>,
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
            } else if tools.iter().any(|t| t.name() == "mcp") {
                ToolRunOutput::Err(format!(
                    r#"{{"type":"unknown_capability","requested_kind":"tool","name":{},"available_tools":{},"recovery_advice":"Tool '{}' is an external MCP capability. Invoke it via the 'mcp' tool: mcp(action: \"call\", server: \"<server_name>\", tool: \"{}\", arguments: {{ ... }})"}}"#,
                    serde_json::to_string(&call.name).unwrap_or_else(|_| "\"invalid\"".into()),
                    serde_json::to_string(&tools.iter().map(|t| t.name()).collect::<Vec<_>>())
                        .unwrap_or_else(|_| "[]".into()),
                    call.name,
                    call.name
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
                agent_id: agent_id.map(|s| s.to_string()),
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
        response_id: None,
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
/// Three consecutive model-drift events (docs/design/68-context-engine.md
/// §7) end the turn with the degraded outcome, mirroring `MAX_REPAIR_TURNS`
/// for tool repair: enough room for one bad step to self-correct after a
/// steering nudge, not enough to spend the whole turn serving the wrong
/// directive.
const MODEL_DRIFT_EXHAUSTION_THRESHOLD: u32 = 3;

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
        if let Ok(val) = serde_json::from_str::<serde_json::Value>(&block)
            && let Some(call) = json_to_tool_call(&val)
        {
            calls.push(call);
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
    } else {
        let idx = text.find("write ")?;
        (idx, "write ".len())
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
            let after = if let Some(stripped) = after.strip_prefix(',') {
                stripped.trim_start()
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
            && !p.is_empty()
            && !c.is_empty()
        {
            return Some(PendingToolCall {
                id: format!("call_txt_{:08x}", rand_jitter(u64::MAX)),
                name: "write".into(),
                input: serde_json::json!({"path": p, "content": c}),
            });
        }
    }
    None
}

fn extract_raw_content(sub: &str) -> Option<String> {
    let sub = sub.trim_start();
    for triple in ["\"\"\"", "'''"] {
        if let Some(rest) = sub.strip_prefix(triple) {
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
        if let Some(rest) = after.strip_prefix(triple) {
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
            let Some(quote) = needle.chars().last() else {
                continue;
            };
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
            && !cmd.trim().is_empty()
        {
            return Some(PendingToolCall {
                id: format!("call_txt_{:08x}", rand_jitter(u64::MAX)),
                name: "bash".into(),
                input: serde_json::json!({"command": cmd.trim()}),
            });
        }
    }
    // Fenced shell code blocks: ```bash ... ``` or ```sh ... ```
    for tag in [
        "```bash\n",
        "```sh\n",
        "```shell\n",
        "```zsh\n",
        "```bash\r\n",
        "```sh\r\n",
    ] {
        if let Some(idx) = text.find(tag) {
            let start = idx + tag.len();
            let rest = &text[start..];
            let end_idx = rest
                .find("\n```")
                .or_else(|| rest.find("\r\n```"))
                .or_else(|| rest.find("```"));
            let raw_cmd = match end_idx {
                Some(e) => &rest[..e],
                None => rest,
            }
            .trim();
            if !raw_cmd.is_empty()
                && !raw_cmd.starts_with("write(")
                && !raw_cmd.starts_with("write ")
                && !raw_cmd.starts_with("write\t")
            {
                return Some(PendingToolCall {
                    id: format!("call_txt_{:08x}", rand_jitter(u64::MAX)),
                    name: "bash".into(),
                    input: serde_json::json!({"command": raw_cmd}),
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

    #[test]
    fn mcp_missing_action_is_recovered_from_shape() {
        use super::{McpToolAlias, PendingToolCall, normalize_mcp_alias};
        let aliases = std::collections::HashMap::<String, McpToolAlias>::new();
        let call = normalize_mcp_alias(
            PendingToolCall {
                id: "1".into(),
                name: "mcp".into(),
                input: serde_json::json!({"server":"weather","tool":"forecast"}),
            },
            &aliases,
        );
        assert_eq!(
            call.input.get("action").and_then(|v| v.as_str()),
            Some("call")
        );
        let list = normalize_mcp_alias(
            PendingToolCall {
                id: "2".into(),
                name: "mcp".into(),
                input: serde_json::json!({}),
            },
            &aliases,
        );
        assert_eq!(
            list.input.get("action").and_then(|v| v.as_str()),
            Some("list")
        );
        let repair = normalize_mcp_alias(
            PendingToolCall {
                id: "3".into(),
                name: "mcp".into(),
                input: serde_json::json!({"action":"call","tool":"forecast"}),
            },
            &aliases,
        );
        assert_eq!(
            repair.input.get("action").and_then(|v| v.as_str()),
            Some("list")
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
                .unwrap_or_default()
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
                .unwrap_or_default()
                .contains("What?")
        );
        assert!(
            calls[0].input["content"]
                .as_str()
                .unwrap_or_default()
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
                .unwrap_or_default()
                .contains("React App")
        );
    }

    #[test]
    fn parses_fenced_bash_script() {
        let text = r#"I will run the Python script to generate the dashboard:

```bash
mkdir -p .vak/scratch
cat <<EOF > .vak/scratch/markov_dashboard.py
import numpy as np
print("Markov Chain Dashboard")
EOF

python3 .vak/scratch/markov_dashboard.py
```

Execution finished."#;
        let calls = parse_text_tool_calls(text);
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].name, "bash");
        let cmd = calls[0].input["command"].as_str().unwrap_or_default();
        assert!(cmd.contains("mkdir -p .vak/scratch"));
        assert!(cmd.contains("python3 .vak/scratch/markov_dashboard.py"));
    }
}
