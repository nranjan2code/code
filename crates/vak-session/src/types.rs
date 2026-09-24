use std::path::PathBuf;

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::BTreeMap;

use vak_llm::{Message, Usage};

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum CapabilityKind {
    Tool,
    Skill,
    McpServer,
    Hook,
    Command,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum CapabilityInvocation {
    ModelTool,
    SkillLoader,
    Automatic,
    UserCommand,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CapabilityDescriptor {
    pub name: String,
    pub kind: CapabilityKind,
    pub invocation: CapabilityInvocation,
    pub description: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source: Option<PathBuf>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub digest: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub provenance: Option<String>,
    /// Kind-specific, non-secret frozen configuration. Tool schemas,
    /// command templates, and hook lifecycle options live here so runtime
    /// dispatch never has to rediscover a second definition.
    #[serde(default)]
    pub configuration: serde_json::Value,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FrozenContract {
    pub app_version: String,
    /// Provider at session admission — **initial snapshot only**.
    ///
    /// Not authoritative for turn dispatch since the per-turn routing change
    /// (docs/design/15-reliability.md §Turn-Level Routing). Each turn's actual
    /// provider is resolved from `Core::effective_route()` and recorded in its
    /// `WorkReceipt`. This field is retained for audit history.
    pub provider: String,
    /// Model at session admission — **initial snapshot only**.
    ///
    /// Same caveat as `provider`: not dispatch authority post per-turn routing.
    /// Use `WorkReceipt.attempt_leg()` to reconstruct which model served a turn.
    pub model: String,
    /// Route ladder at session admission — **initial snapshot only**.
    ///
    /// Since per-turn routing, each turn calls `Core::plan_route_ladder()` fresh
    /// using the live evidence ledger, belief state, and `effective_route()`.
    /// This field records what the ladder looked like when the session was opened.
    /// Empty/missing ⇒ a single-model header written before ladders existed.
    #[serde(default)]
    pub route_ladder: Vec<vak_llm::RouteLeg>,
    /// Objective the ladder was ordered for (Phase R): "utility" |
    /// "balanced" | "quality-critical". Initial snapshot; see `route_ladder`.
    #[serde(default)]
    pub route_objective: String,
    /// Freeze-time routing warnings (thin chain, dominant failure
    /// domain, unreachable cross-model fallbacks). Audit-only context,
    /// never model-visible input. Empty on headers written before this
    /// field existed.
    #[serde(default)]
    pub route_annotations: Vec<String>,
    pub system_prompt: String,
    /// **Authoritative** for the session's lifetime: the permission mode
    /// admitted when this session was created. Never changed mid-session.
    pub permission_mode: String,
    /// **Authoritative** capability packet admitted for this session. Prompt
    /// advertisement, model tool schemas, dispatch, and audit projections
    /// must all derive from this exact list. Never changed mid-session.
    #[serde(default)]
    pub capabilities: Vec<CapabilityDescriptor>,
    /// Which prompt layer contributed each block, with a digest of the text
    /// it contributed (docs/design/45-prompt-layers.md). `system_prompt` says
    /// what the model was told; this says *who told it* and lets a resumed
    /// session notice that an editable layer changed underneath it. Empty on
    /// headers written before prompt layers existed.
    #[serde(default)]
    pub prompt_layers: Vec<PromptLayerDescriptor>,
}

/// The exact capability interface bound for one model turn. This is
/// append-only audit data: it is not projected into model messages, but it
/// makes the provider request reconstructable after live capabilities move.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TurnCapabilitiesBound {
    pub epoch: u64,
    pub capability_ids: Vec<String>,
    pub excluded_ids: Vec<String>,
    pub system_prompt: String,
    #[serde(default)]
    pub tool_schemas: Vec<serde_json::Value>,
    /// Tool names sent with full schemas in the stable prefix this turn
    /// (docs/design/68-context-engine.md §5). Empty on entries written
    /// before the tool surface split existed.
    #[serde(default)]
    pub core_tool_names: Vec<String>,
    /// Tool names withheld from the prefix this turn — reachable via
    /// `find_tools`, or via Anthropic `defer_loading` on legs that support
    /// it. Empty on entries written before the split existed.
    #[serde(default)]
    pub deferred_tool_names: Vec<String>,
    /// The rendered `tool_index` text sent this turn: one line per deferred
    /// tool, no schemas.
    #[serde(default)]
    pub tool_index: String,
    /// Declared domains (`vak_core::capability::domain::Domain::as_str`)
    /// per bound tool name, so a later projection can recover "what did this
    /// tool declare it serves" without re-touching the live capability
    /// registry (docs/design/68 §9's `SignalContext.domains`).
    #[serde(default)]
    pub tool_domains: std::collections::BTreeMap<String, Vec<String>>,
}

/// One layer's contribution to the assembled system prompt.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PromptLayerDescriptor {
    /// "identity" | "operating-rules" | "guardrails".
    pub block: String,
    /// "seed" | "shared" | "project" | "surface" | "bot" | "chat" | "agent".
    pub layer: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source: Option<String>,
    pub digest: String,
    pub bytes: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SessionHeader {
    /// Frozen user-facing owner. Absence denotes the built-in Vak agent.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub agent: Option<AgentIdentity>,
    pub session_id: String,
    pub created_at: DateTime<Utc>,
    pub cwd: PathBuf,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub parent_session_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub contract_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub work_item_id: Option<String>,
    /// Durable conversation ownership and ingress provenance. This is
    /// optional only while the baseline checker identifies pre-contract
    /// ledgers; every newly admitted session receives it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub conversation: Option<ConversationContext>,
    pub contract: FrozenContract,
}

/// The audience and conversation that are allowed to see a session. A
/// transport address is not itself a person: `audience_id` is the verified
/// principal/group identity, while `origin` records where this request arrived
/// for delivery and audit purposes.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ConversationContext {
    pub conversation_id: String,
    pub audience_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub origin: Option<ConversationOrigin>,
}

impl ConversationContext {
    /// The private local default used when an embedding surface has not
    /// supplied a remote audience. The conversation id is intentionally the
    /// newly admitted session id, so independent CLI/background admissions do
    /// not accidentally share history.
    pub fn local(conversation_id: impl Into<String>, surface: impl Into<String>) -> Self {
        let surface = surface.into();
        Self {
            conversation_id: conversation_id.into(),
            audience_id: "local".into(),
            origin: Some(ConversationOrigin {
                surface: if surface.trim().is_empty() {
                    "local".into()
                } else {
                    surface
                },
                address: "local".into(),
                bot_id: None,
            }),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ConversationOrigin {
    pub surface: String,
    pub address: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub bot_id: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AgentIdentity {
    pub id: String,
    pub revision: u64,
    pub name: String,
    // Older ledgers in the supported major line predate characters. An
    // empty historical snapshot means "use the Agent's current profile";
    // new headers always write the selected character explicitly.
    #[serde(default)]
    pub character: String,
    pub personality: String,
    #[serde(default = "default_agent_animation")]
    pub animation: String,
    #[serde(default = "default_agent_voice")]
    pub voice: String,
    pub behaviour: String,
    #[serde(default)]
    pub responsibilities: String,
    /// User-authored instructions added to vak's universal foundation.
    #[serde(default)]
    pub instructions: String,
}

fn default_agent_animation() -> String {
    "subtle".into()
}

fn default_agent_voice() -> String {
    "default".into()
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

/// One model-visible message with its ledger identity and class.
#[derive(Debug, Clone)]
pub struct TranscriptMessage {
    /// The ledger entry this message came from (stable, unique).
    pub entry_id: String,
    pub message: Message,
    /// Set when the runtime authored this user-role message (a nudge).
    pub control: Option<vak_intent::control::ControlKind>,
    /// Set when the runtime derived this message into the model's input
    /// (compaction summary, intent note, work contract, conversation thread).
    pub context: bool,
    pub author_id: Option<String>,
    pub author_name: Option<String>,
}

impl MessageRecord {
    /// A user-role message the runtime authored. The body still begins with
    /// the kind's marker (the model reads it); the tag is what every other
    /// layer reads instead of guessing from the text.
    pub fn control(kind: vak_intent::control::ControlKind, body: impl Into<String>) -> Self {
        Self {
            message: Message::user_text(body),
            meta: Some(MessageMeta {
                control: Some(kind),
                ..MessageMeta::default()
            }),
        }
    }

    /// Whether the runtime, not the user, authored this message: the
    /// structural tag, and nothing else. Text is never sniffed.
    pub fn control_kind(&self) -> Option<vak_intent::control::ControlKind> {
        self.meta.as_ref().and_then(|meta| meta.control)
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct MessageMeta {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub stop_reason: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub usage: Option<Usage>,
    /// Set on a user-role message the runtime authored (a repair nudge, a
    /// stop guard) rather than the user. See `vak_intent::control`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub control: Option<vak_intent::control::ControlKind>,
    /// Verified human principal for a shared-conversation message.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub author_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub author_name: Option<String>,
    /// Client idempotency key, scoped to `author_id`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub request_id: Option<String>,
}

/// A compaction packet (docs/design/68-context-engine.md §4): the summary
/// of one contiguous range of closed turns, keyed by that range. A packet
/// is a cache of summariser work, never a boundary. The `WorkingSetPlanner`
/// decides per request, from the bound model's measured profile, whether a
/// packet is needed at all and over which range; the projection reuses a
/// stored packet only when its range is exactly the one the plan asks for.
/// So a packet written while a small model was bound never hides those
/// turns from a larger model bound later: the ledger stays the one rich
/// original, and every projection is a function of (ledger, bound model).
///
/// `reset_all` is the one true boundary: the reset-with-handoff rescue
/// (docs/design/42-managed-work-contracts.md) for a profile with no usable
/// horizon replaces everything before the entry with `summary`; the range
/// fields are empty on such an entry.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CompactionEntry {
    pub summary: String,
    /// Oldest covered turn's directive entry id (inclusive).
    pub first_turn_id: String,
    /// Newest covered turn's directive entry id (inclusive).
    pub last_turn_id: String,
    /// The model whose plan asked for this packet and whose summariser
    /// wrote it — provenance for forensics, not a lookup key.
    pub model: String,
    pub tokens_before: u64,
    #[serde(default)]
    pub reset_all: bool,
}

/// A durable objective with acceptance criteria (docs/design/42-managed-work-contracts.md).
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

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct WorkContract {
    pub contract_id: String,
    pub revision: u32,
    pub source_entry_id: String,
    pub objective: String,
    #[serde(default)]
    pub constraints: Vec<WorkConstraint>,
    #[serde(default)]
    pub assumptions: Vec<WorkAssumption>,
    #[serde(default)]
    pub criteria: Vec<WorkCriterion>,
    pub items: Vec<WorkItemDefinition>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct WorkConstraint {
    pub constraint_id: String,
    pub text: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct WorkAssumption {
    pub assumption_id: String,
    pub text: String,
    #[serde(default)]
    pub requires_confirmation: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub resolution: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct WorkCriterion {
    pub criterion_id: String,
    pub statement: String,
    pub kind: CriterionKind,
    #[serde(default = "default_true")]
    pub required: bool,
}

fn default_true() -> bool {
    true
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum CriterionKind {
    Shell { command: String },
    FileExists { path: PathBuf },
    FileContains { path: PathBuf, pattern: String },
    ToolSucceeded { tool: String },
    FlowCompleted { flow: String },
    ExternalReceipt { integration: String },
    Semantic,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct WorkItemDefinition {
    pub item_id: String,
    pub title: String,
    pub instructions: String,
    #[serde(default)]
    pub dependencies: Vec<String>,
    pub owner: WorkOwner,
    #[serde(default = "default_true")]
    pub required: bool,
    #[serde(default)]
    pub readonly: bool,
    #[serde(default)]
    pub path_claims: Vec<String>,
    #[serde(default)]
    pub criterion_ids: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum WorkOwner {
    ParentAgent,
    /// A ledger entry written before the subagent->worker rename stored this
    /// as "subagent" — the alias keeps sessions logged before that upgrade
    /// from silently failing to parse (which would break the hash-chain
    /// integrity check on the next entry too).
    #[serde(alias = "subagent")]
    Worker,
    Flow {
        name: String,
    },
    Tool {
        name: String,
    },
    Human,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum WorkContractStatus {
    Draft,
    AwaitingInput,
    Active,
    Blocked,
    Verifying,
    Completed,
    Failed,
    Cancelled,
    Unverified,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum WorkItemStatus {
    Proposed,
    Ready,
    Running,
    WaitingApproval,
    Blocked,
    ReadyForVerification,
    Succeeded,
    Failed,
    Skipped,
    Cancelled,
    Interrupted,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct WorkItemState {
    pub item_id: String,
    pub owner: WorkOwner,
    pub status: WorkItemStatus,
    pub attempt: u32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub child_session_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub blocker: Option<String>,
    #[serde(default)]
    pub evidence: Vec<EvidenceRef>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum EvidenceRef {
    LedgerEntry {
        session_id: String,
        entry_id: String,
    },
    ToolResult {
        session_id: String,
        tool_use_id: String,
    },
    Receipt {
        session_id: String,
        entry_id: String,
    },
    CheckpointDiff {
        session_id: String,
        from_seq: u32,
        to_seq: u32,
    },
    FlowNode {
        flow: String,
        run_id: String,
        node_id: String,
    },
    ChildSession {
        session_id: String,
    },
    ExternalOperation {
        integration: String,
        operation_id: String,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum CriterionResult {
    Passed { evidence: String },
    Failed { reason: String },
    Unknown { reason: String },
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct WorkEvent {
    pub contract_id: String,
    pub revision: u32,
    pub kind: WorkEventKind,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum WorkEventKind {
    ContractCreated {
        contract: WorkContract,
    },
    ContractRevised {
        previous_revision: u32,
        contract: WorkContract,
        reason: String,
    },
    ContractStatusChanged {
        from: WorkContractStatus,
        to: WorkContractStatus,
        reason: String,
    },
    ItemStatusChanged {
        item_id: String,
        from: WorkItemStatus,
        to: WorkItemStatus,
        attempt: u32,
        reason: String,
    },
    ItemVerified {
        item_id: String,
        attempt: u32,
    },
    ItemAssigned {
        item_id: String,
        owner: WorkOwner,
        child_session_id: Option<String>,
    },
    EvidenceAttached {
        item_id: String,
        evidence: EvidenceRef,
    },
    AssumptionResolved {
        assumption_id: String,
        resolution: String,
    },
    VerificationRecorded {
        criterion_id: String,
        result: CriterionResult,
    },
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ActivityKind {
    Approval,
    Retry,
    RouteFallback,
    /// A ledger entry written before the subagent->worker rename stored this
    /// as "subagent" — the alias keeps sessions logged before that upgrade
    /// from silently failing to parse.
    #[serde(alias = "subagent")]
    Worker,
    Diagnostic,
    Run,
    /// A provisional or committed speech recognition segment.
    VoiceTranscript,
    /// Speech delivery and playback accounting for an assistant response.
    VoicePlayback,
    /// Which immutable presentation revision was selected for a result.
    PresentationSelection,
    /// A validated immutable presentation revision preview was proposed.
    PresentationProposal,
    /// User choice or feedback about a presentation projection.
    PresentationFeedback,
    /// Human feedback anchored to an immutable candidate result and file.
    CandidateComment,
    /// A human-requested isolated Agent revision of a saved candidate.
    CandidateRevision,
    /// A bind-time capacity probe ran and recorded a `CapacityProfile`
    /// (docs/design/68-context-engine.md §1). Never model-visible.
    CapacityProbe,
    /// A turn's usage or instruction-following outcome updated an existing
    /// `CapacityProfile` (§1 "Feedback"). Never model-visible.
    CapacityFeedback,
}

/// Where a validated presentation came from (docs/design/68-context-engine.md
/// §10 "Presentations are ledger entries"). Both paths converge on the same
/// `PresentationRecord` shape; only the provenance differs.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum PresentationSource {
    /// Emitted through an `emit_*_card` tool call.
    ToolCall { tool_use_id: String },
    /// Emitted as an inline ```` ```vak ```` fence in assistant text (models
    /// without tool calling).
    Fence { message_entry_id: String },
    /// Emitted by a worker this conversation delegated to through the
    /// `tool_use_id` call (`task`). The worker's own ledger holds the card's
    /// original call; this entry is how the delegating conversation shows it
    /// and recalls it. Unlike `ToolCall`, one call may carry several.
    Delegated {
        tool_use_id: String,
        worker_session_id: String,
    },
}

/// A validated `emit_*_card` (or fence) presentation, written once at the
/// moment it validates. Never model-visible raw (`derive_messages` skips it,
/// like `Receipt`): the current turn already sees the card through the
/// `tool_use` input it wrote; later turns see it through a `TurnCard` or a
/// full-record rendering, both of which read this entry. This is the single
/// source both the model-visible history and the display channel read —
/// nothing is rebuilt from tool arguments after this is written.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PresentationRecord {
    /// The entry id of the user directive this turn answers.
    pub turn_id: String,
    pub source: PresentationSource,
    pub semantic_type: String,
    pub skill_id: String,
    pub skill_version: String,
    pub schema_version: u32,
    /// Canonical (validated, key-sorted) form. See [`canonicalize_json`].
    pub payload: Value,
    /// Hash of the canonical payload. See [`payload_digest`].
    pub payload_digest: String,
    /// Evidence ids: the `tool_use_id`s of every non-card tool result that
    /// appears in the current turn before this card.
    pub derived_from: Vec<String>,
    pub title: String,
    /// Schema-driven summary of the fields that make this presentation
    /// distinguishable from another of the same `semantic_type`, used in
    /// `TurnCard` index lines. Never a character truncation.
    pub identity_digest: String,
}

/// Recursively sorts object keys so two payloads that differ only in field
/// insertion order canonicalize to identical bytes. Array order is
/// preserved — it is meaningful (e.g. chart series, table rows).
///
/// Uses an explicit `BTreeMap` pass (rather than relying on `serde_json`'s
/// own map ordering, which is only sorted when the `preserve_order` feature
/// is off) so the canonical form is deterministic regardless of that
/// feature flag.
pub fn canonicalize_json(value: &Value) -> Value {
    match value {
        Value::Object(map) => {
            let sorted: BTreeMap<&String, &Value> = map.iter().collect();
            let mut out = serde_json::Map::new();
            for (key, val) in sorted {
                out.insert(key.clone(), canonicalize_json(val));
            }
            Value::Object(out)
        }
        Value::Array(items) => Value::Array(items.iter().map(canonicalize_json).collect()),
        other => other.clone(),
    }
}

/// SHA-256 hex digest of a payload's canonical form (see
/// [`canonicalize_json`]). Used to detect a repeated presentation — the
/// same card validated twice in one turn (once via tool call, once via a
/// duplicate fence) hashes identically regardless of key order.
pub fn payload_digest(payload: &Value) -> String {
    use sha2::{Digest, Sha256};
    let canonical = canonicalize_json(payload);
    let bytes = serde_json::to_vec(&canonical).unwrap_or_default();
    let mut hasher = Sha256::new();
    hasher.update(&bytes);
    hasher
        .finalize()
        .iter()
        .fold(String::with_capacity(64), |mut acc, byte| {
            use std::fmt::Write;
            let _ = write!(acc, "{byte:02x}");
            acc
        })
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

/// One turn's resolved intent (docs/design/47-commitment-kernel.md).
///
/// Model-visible **and** audit in one entry, deliberately. `model_visible`
/// holds the exact text the engagement contributed to the model's context, so
/// invariant 1 holds by construction: replaying the ledger reproduces the
/// prompt byte-for-byte rather than regenerating it from a derivation that may
/// have changed in the meantime.
///
/// The `reading`/`engagement`/`provenance` fields alongside it are what make
/// the decision auditable — including whether it was reproducible at all.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct IntentRecord {
    /// The composite reading for the turn.
    pub reading: vak_intent::Reading,
    /// The parts of the request, each with its own reading, relation and
    /// thread lineage (docs/design/47-commitment-kernel.md, strands).
    #[serde(default)]
    pub strands: Vec<vak_intent::Strand>,
    pub engagement: vak_intent::Engagement,
    pub provenance: vak_intent::Provenance,
    /// The requested outcome captured for this turn, when the host could
    /// construct one without inventing requirements.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub outcome: Option<vak_intent::OutcomeSpec>,
    /// Exactly what the model was told, if anything. `None` when the
    /// engagement had nothing worth spending tokens to say.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model_visible: Option<String>,
    /// The durable commitment this turn serves, when one is open: the
    /// primary strand's.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub commitment_id: Option<String>,
    /// Every strand's commitment, by strand id, when more than one durable
    /// thread is served by this turn.
    #[serde(default, skip_serializing_if = "std::collections::BTreeMap::is_empty")]
    pub strand_commitments: std::collections::BTreeMap<String, String>,
}

/// Durable terminal marker for a child-agent run. Presence of a child ledger
/// alone does not prove that the child finished; recovery must consult this
/// marker before attaching evidence or retrying work.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ChildRunStatus {
    Completed,
    Failed,
    Aborted,
    MaxTurns,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
// SessionHeader intentionally carries the frozen agent contract and remains
// inline so the append-only JSONL representation and all existing pattern
// matches stay unchanged. Keep this representation decision explicit as the
// header grows; boxing it would be a wire/API refactor, not a lint-only fix.
#[allow(clippy::large_enum_variant)]
pub enum EntryPayload {
    Header(SessionHeader),
    Message(MessageRecord),
    Compaction(CompactionEntry),
    /// Audit record for one unit of provider work (docs/design/42-managed-work-contracts.md).
    /// Never model-visible: `derive_messages` skips it.
    Receipt(vak_llm::WorkReceipt),
    /// Goal lifecycle (docs/design/42-managed-work-contracts.md). Never model-visible.
    Goal(GoalEntry),
    /// Relationship between this request and the active collaborative goal.
    /// Never model-visible; the original request remains a Message entry.
    GoalUpdate(vak_intent::GoalUpdate),
    /// UI/audit lifecycle facts; never model-visible.
    Activity(ActivityRecord),
    /// Durable managed-work lifecycle event. The projector is the source of
    /// current work state; events are never rewritten.
    Work(WorkEvent),
    /// This turn's resolved intent. Model-visible via `model_visible`.
    Intent(Box<IntentRecord>),
    /// Terminal marker written by a child agent before its parent observes the
    /// result. Never model-visible.
    ChildRun {
        status: ChildRunStatus,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        outcome: Option<vak_intent::OutcomeSpec>,
    },
    /// Exact capability interface used by one provider turn.
    TurnCapabilitiesBound(TurnCapabilitiesBound),
    /// A validated presentation (docs/design/68-context-engine.md §10).
    /// Never model-visible raw: `derive_messages` skips it like `Receipt`.
    Presentation(PresentationRecord),
    /// A turn's closing card (docs/design/68-context-engine.md §10), written
    /// once when the turn closes and never rewritten. Never model-visible
    /// raw: a follow-up turn sees it through `TurnCard::line` in the
    /// `<turns>` tail block or, promoted, through `Turn::full_record`, never
    /// through this entry directly.
    TurnCard(TurnCardRecord),
    /// The whole result of a tool call whose request carried only a window
    /// of it (docs/design/68-context-engine.md §3). Never model-visible raw:
    /// the `ToolResult` block holds what the request carried, and `recall`
    /// and the closed-turn digests read this.
    EvidenceBody(EvidenceBodyRecord),
}

/// The whole result behind a windowed `ToolResult` block.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct EvidenceBodyRecord {
    pub tool_use_id: String,
    pub content: String,
}

/// The closing record for one turn (docs/design/68-context-engine.md §10).
/// `turn_id` is the directive entry id the card answers — the same id
/// `TurnIndex` uses to key turns and `recall({ turn })` resolves against.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TurnCardRecord {
    pub turn_id: String,
    pub card: crate::turns::TurnCard,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Entry {
    pub id: String,
    #[serde(default)]
    pub parent_id: Option<String>,
    pub ts: DateTime<Utc>,
    /// SHA-256 of the previous entry's serialized line, hex-encoded.
    ///
    /// `parent_id` links entries but binds nothing: an interior entry could be
    /// rewritten and re-linked, and reconstruction would accept the result.
    /// This makes any such edit detectable — changing an entry changes its
    /// line digest, which no longer matches its successor's `prev_hash`.
    ///
    /// `None` on the first entry, and on every entry written before the chain
    /// existed. Ledgers are a frozen, append-only contract, so an unchained
    /// entry is reported as a warning and never a read failure.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub prev_hash: Option<String>,
    #[serde(flatten)]
    pub payload: EntryPayload,
}

impl Entry {
    pub fn new(parent_id: Option<String>, payload: EntryPayload) -> Self {
        Entry {
            id: uuid::Uuid::now_v7().to_string(),
            parent_id,
            ts: Utc::now(),
            prev_hash: None,
            payload,
        }
    }
}

/// Chain digest of one serialized ledger line.
///
/// Taken over the exact bytes written rather than a re-serialization, so
/// verification cannot drift with serde field ordering or formatting.
pub fn line_digest(line: &str) -> String {
    use sha2::{Digest, Sha256};
    let mut hasher = Sha256::new();
    hasher.update(line.as_bytes());
    hasher
        .finalize()
        .iter()
        .fold(String::with_capacity(64), |mut acc, byte| {
            use std::fmt::Write;
            let _ = write!(acc, "{byte:02x}");
            acc
        })
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

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod agent_identity_tests {
    #[test]
    fn character_absent_from_existing_ledger_has_neutral_default() {
        let value = serde_json::json!({
            "id": "researcher",
            "revision": 1,
            "name": "Researcher",
            "personality": "curious",
            "behaviour": "careful"
        });
        let identity = serde_json::from_value::<super::AgentIdentity>(value).unwrap();
        assert!(identity.character.is_empty());
    }

    #[test]
    fn movement_and_voice_have_stable_defaults_for_existing_ledgers() {
        let value = serde_json::json!({
            "id": "researcher",
            "revision": 1,
            "name": "Researcher",
            "character": "moss",
            "personality": "curious",
            "behaviour": "careful"
        });
        let identity = serde_json::from_value::<super::AgentIdentity>(value).unwrap();
        assert_eq!(identity.animation, "subtle");
        assert_eq!(identity.voice, "default");
    }
}
