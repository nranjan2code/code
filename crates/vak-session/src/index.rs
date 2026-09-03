//! Per-ledger recall cache (docs/design/23-memory.md M1): keyed by the
//! canonical ledger path and invalidated when mtime or length differs, so
//! warm queries skip the JSONL rescan while appends stay visible on the
//! very next search. Purely an accelerator — cold behavior is identical to
//! a fresh scan.

use std::collections::{HashMap, VecDeque};
use std::io::BufRead;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, MutexGuard, OnceLock};
use std::time::{Instant, SystemTime};

use chrono::{DateTime, Utc};
use vak_llm::Role;

use crate::SearchError;
use crate::search::normalize_impl;
use crate::types::{Entry, EntryPayload};

/// Bounded work: at most the trailing N message lines of a ledger are
/// scanned. Long sessions still recall their recent past.
pub(crate) const MAX_SCAN_LINES: usize = 4000;
const MAX_CACHED_LEDGERS: usize = 128;

pub(crate) struct CachedMessage {
    pub entry_id: String,
    pub ts: DateTime<Utc>,
    pub role: &'static str,
    pub text: String,
    pub normalized: String,
}

#[derive(Clone, Copy, PartialEq, Eq)]
struct Fingerprint {
    mtime: SystemTime,
    len: u64,
}

struct CacheSlot {
    fingerprint: Fingerprint,
    last_used: Instant,
    ledger: Arc<Vec<CachedMessage>>,
}

fn cache() -> &'static Mutex<HashMap<PathBuf, CacheSlot>> {
    static CACHE: OnceLock<Mutex<HashMap<PathBuf, CacheSlot>>> = OnceLock::new();
    CACHE.get_or_init(|| Mutex::new(HashMap::new()))
}

fn lock(map: &Mutex<HashMap<PathBuf, CacheSlot>>) -> MutexGuard<'_, HashMap<PathBuf, CacheSlot>> {
    map.lock().unwrap_or_else(|poisoned| poisoned.into_inner())
}

fn evict_to_capacity(map: &mut HashMap<PathBuf, CacheSlot>) {
    while map.len() >= MAX_CACHED_LEDGERS {
        let oldest = map
            .iter()
            .min_by_key(|(_, slot)| slot.last_used)
            .map(|(k, _)| k.clone());
        match oldest {
            Some(k) => {
                map.remove(&k);
            }
            None => break,
        }
    }
}

/// Tokenized representation of `path`'s trailing window, served from cache
/// when the file's mtime and length are unchanged since the last scan.
pub(crate) fn ledger(path: &Path) -> Result<Arc<Vec<CachedMessage>>, SearchError> {
    let key = path.canonicalize()?;
    let meta = std::fs::metadata(path)?;
    let fingerprint = Fingerprint {
        mtime: meta.modified()?,
        len: meta.len(),
    };
    {
        let mut map = lock(cache());
        if let Some(slot) = map.get_mut(&key)
            && slot.fingerprint == fingerprint
        {
            slot.last_used = Instant::now();
            return Ok(Arc::clone(&slot.ledger));
        }
    }
    let scanned = Arc::new(scan(path)?);
    let mut map = lock(cache());
    // A concurrent query may have cached a snapshot of this exact state
    // while we scanned; prefer it over re-inserting an equal copy.
    if let Some(slot) = map.get(&key)
        && slot.fingerprint == fingerprint
    {
        return Ok(Arc::clone(&slot.ledger));
    }
    evict_to_capacity(&mut map);
    map.insert(
        key,
        CacheSlot {
            fingerprint,
            last_used: Instant::now(),
            ledger: Arc::clone(&scanned),
        },
    );
    Ok(scanned)
}

fn scan(path: &Path) -> Result<Vec<CachedMessage>, SearchError> {
    let file = std::fs::File::open(path)?;
    let reader = std::io::BufReader::new(file);
    // Ring buffer keeps memory bounded on huge ledgers while preserving
    // "trailing lines" semantics.
    let mut ring: VecDeque<String> = VecDeque::with_capacity(MAX_SCAN_LINES);
    for line in reader.lines() {
        let line = line?;
        if line.trim().is_empty() {
            continue;
        }
        if ring.len() == MAX_SCAN_LINES {
            ring.pop_front();
        }
        ring.push_back(line);
    }
    let mut messages = Vec::new();
    for line in ring {
        let Ok(entry) = serde_json::from_str::<Entry>(&line) else {
            continue;
        };
        let EntryPayload::Message(record) = entry.payload else {
            continue;
        };
        let role = match record.message.role {
            Role::User => "user",
            Role::Assistant => "assistant",
        };
        let text = record.message.text_content();
        if text.trim().is_empty() {
            continue;
        }
        messages.push(CachedMessage {
            normalized: normalize_impl(&text),
            text,
            role,
            entry_id: entry.id,
            ts: entry.ts,
        });
    }
    Ok(messages)
}
