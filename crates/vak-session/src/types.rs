use std::path::PathBuf;

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
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
    pub provider: String,
    pub model: String,
    /// Frozen route ladder (docs/design/15-reliability.md): ordered candidate
    /// legs committed at admission; dispatch walks it top-down on typed
    /// failures. Walking the ladder IS contract execution — never a
    /// mid-contract switch. Empty/missing ⇒ a single-model header, which
    /// includes every header written before ladders existed.
    #[serde(default)]
    pub route_ladder: Vec<vak_llm::RouteLeg>,
    /// Objective the ladder was ordered for (Phase R): "utility" |
    /// "balanced" | "quality-critical". Empty on headers written before
    /// objectives existed.
    #[serde(default)]
    pub route_objective: String,
    /// Freeze-time routing warnings (thin chain, dominant failure
    /// domain, unreachable cross-model fallbacks). Audit-only context,
    /// never model-visible input. Empty on headers written before this
    /// field existed.
    #[serde(default)]
    pub route_annotations: Vec<String>,
    pub system_prompt: String,
    pub permission_mode: String,
    /// Authoritative capability packet admitted for this session. Prompt
    /// advertisement, model tool schemas, dispatch, and audit projections
    /// must all derive from this exact list.
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
    pub personality: String,
    pub behaviour: String,
    #[serde(default)]
    pub responsibilities: String,
    /// User-authored instructions added to vak's universal foundation.
    #[serde(default)]
    pub instructions: String,
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
    /// Packet accounting (docs/design/17-context.md): which visible message entries
    /// stayed verbatim vs became summary material. `None` for entries
    /// written before accounting existed.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub partition: Option<ContextPartition>,
    /// Full-context reset (docs/design/42-managed-work-contracts.md): when true, the projection
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
    Subagent,
    Flow { name: String },
    Tool { name: String },
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
    Subagent,
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
    pub reading: vak_intent::Reading,
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
    /// The durable commitment this turn serves, when one is open.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub commitment_id: Option<String>,
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

/// Projection-based compaction plan (see SessionLog::plan_compaction).
#[derive(Debug, Clone)]
pub struct CompactionPlan {
    pub older: Vec<vak_llm::Message>,
    /// Id of the FIRST KEPT projected entry — the new compaction's
    /// first_kept_entry_id anchor.
    pub first_kept_entry_id: String,
    /// Where each visible message entry lands (docs/design/17-context.md).
    pub partition: ContextPartition,
}
