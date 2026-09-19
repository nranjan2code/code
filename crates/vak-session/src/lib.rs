//! vak-session: append-only JSONL session trees.
//!
//! Invariant: model context is always derived by projecting the log
//! (derive_messages); nothing reaches a request that is not reconstructable
//! from the file. Branching is in-place via parent ids; compaction is an
//! entry, never a deletion.

mod index;
pub mod log;
pub mod search;
pub mod types;
pub mod work;

pub use log::{SessionLog, SessionPath, TailSections};
pub use search::{
    DEFAULT_LIMIT, ExternalDoc, MEMORY_BONUS, ProjectHit, SearchError, SessionHit, search,
    search_all, search_all_extended, search_extended,
};
pub use types::{
    ActivityKind, ActivityRecord, ActivityStatus, CapabilityDescriptor, CapabilityInvocation,
    CapabilityKind, CompactionEntry, ConversationContext, ConversationOrigin, Entry, EntryPayload,
    FrozenContract, MessageMeta, MessageRecord, PromptLayerDescriptor, SessionError, SessionHeader,
    TranscriptMessage, TurnCapabilitiesBound,
};
pub use work::{
    WorkError, WorkProjection, project_work, validate_contract, validate_contract_for_admission,
};
