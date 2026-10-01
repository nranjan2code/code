//! vak-session: append-only JSONL session trees.
//!
//! Invariant: model context is always derived by projecting the log
//! (derive_messages); nothing reaches a request that is not reconstructable
//! from the file. Branching is in-place via parent ids; compaction is an
//! entry, never a deletion.

pub mod ids;
mod index;
pub mod log;
pub mod search;
pub mod trace;
pub mod turns;
pub mod types;
pub mod work;

pub use log::{SessionLog, SessionPath, TailSections};
pub use search::{
    DEFAULT_LIMIT, ExternalDoc, MEMORY_BONUS, ProjectHit, SearchError, SessionHit, search,
    search_all, search_all_extended, search_extended,
};
pub use turns::{
    Answer, Evidence, Fidelity, Packet, PresentationRef, ReadingKey, Step, TraceLine, Turn,
    TurnCard, TurnIndex, WorkingSetPlan, bash_digest, evidence_digest, evidence_shape,
    transcript_result,
};
pub use types::{
    ActivityKind, ActivityRecord, ActivityStatus, AttachedFile, CapabilityDescriptor,
    CapabilityInvocation, CapabilityKind, CompactionEntry, ConversationContext, ConversationOrigin,
    Entry, EntryPayload, FrozenContract, MessageMeta, MessageRecord, PromptLayerDescriptor,
    SessionError, SessionHeader, TranscriptMessage, TurnCapabilitiesBound, TurnCapabilitiesRef,
    TurnCardRecord,
};
pub use work::{
    WorkError, WorkProjection, project_work, validate_contract, validate_contract_for_admission,
};
