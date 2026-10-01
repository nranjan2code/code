//! Storage substrate: no vak dependencies, primitives from `ring` only.
//! Design: docs/design/73-data-architecture-and-lifecycle.md §5-§7.3.

pub mod keys;
pub mod objects;
pub mod records;
pub mod refs;

mod seal;

use thiserror::Error;

pub type Result<T> = std::result::Result<T, StorageError>;

#[derive(Debug, Error)]
pub enum StorageError {
    #[error("key authority unavailable: {0}")]
    AuthorityUnavailable(String),
    #[error("scope key revoked: {0}")]
    Revoked(String),
    #[error("cryptographic failure: {0}")]
    Crypto(&'static str),
    #[error("compression failure: {0}")]
    Compression(String),
    #[error("io: {0}")]
    Io(#[from] std::io::Error),
    #[error("sqlite: {0}")]
    Sqlite(#[from] rusqlite::Error),
    #[error("stale writer epoch {presented}, current {current}")]
    StaleEpoch { presented: u64, current: u64 },
    #[error("generation conflict: expected {expected:?}, current {current:?}")]
    Conflict {
        expected: Option<u64>,
        current: Option<u64>,
    },
    #[error("object not found")]
    NotFound,
    #[error("no grant for scope")]
    NoGrant,
    #[error("object integrity check failed")]
    Integrity,
    #[error("chain broken at entry {0}")]
    ChainBroken(u64),
    #[error("torn tail at byte {0}")]
    TornTail(u64),
    #[error("frame is sealed and cannot be opened with the given key")]
    Undecryptable,
    #[error("malformed data: {0}")]
    Malformed(&'static str),
}
