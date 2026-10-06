//! vak-session: append-only JSONL session trees.
//!
//! Invariant: model context is always derived by projecting the log
//! (derive_messages); nothing reaches a request that is not reconstructable
//! from the file. Branching is in-place via parent ids; compaction is an
//! entry, never a deletion.

pub mod chain;
pub mod cursors;
pub mod documents;
pub mod effects;
pub mod fence;
pub mod ids;
pub mod log;
pub mod objects;
pub mod rollup;
pub mod runs;
pub mod tail;
pub mod text;
pub mod trace;
pub mod turns;
pub mod types;
pub mod work;

pub use log::{SessionLog, SessionPath, TailSections};
pub use turns::{
    Answer, CONTEXT_PLAN_LABEL, CONTEXT_PLAN_POLICY_VERSION, Evidence, Fidelity, LinkKind, Packet,
    PresentationRef, ReadingKey, Step, TraceLine, Turn, TurnCard, TurnIndex, TurnLink,
    WorkingSetPlan, bash_digest, evidence_digest, evidence_shape, transcript_result,
};
pub use types::{
    ActivityKind, ActivityRecord, ActivityStatus, AttachedFile, CapabilityDescriptor,
    CapabilityInvocation, CapabilityKind, CompactionEntry, ConversationContext, ConversationOrigin,
    Entry, EntryPayload, FrozenContract, MessageMeta, MessageRecord, PromptLayerDescriptor,
    SessionError, SessionHeader, TranscriptMessage, TurnBinding, TurnCapabilitiesBound,
    TurnCapabilitiesRef, TurnCardRecord,
};
pub use work::{
    WorkError, WorkProjection, project_work, validate_contract, validate_contract_for_admission,
};
