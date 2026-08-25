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

pub use log::{SessionLog, SessionPath};
pub use search::{
    DEFAULT_LIMIT, ExternalDoc, MEMORY_BONUS, ProjectHit, SearchError, SessionHit, search,
    search_all, search_extended,
};
pub use types::{
    CompactionEntry, Entry, EntryPayload, FrozenContract, MessageMeta, MessageRecord, SessionError,
    SessionHeader,
};
