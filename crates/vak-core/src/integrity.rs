//! Whether what the data home holds is intact (plan M7a-i,
//! docs/design/74 §6.2 A9): every ledger and record chain verified from
//! its stored bytes, with no key; the scope keys counted; the erasure
//! receipts' signatures checked; and whether search is behind the
//! records. It reads and reports, and changes nothing.

use crate::Core;
use chrono::{DateTime, Utc};
use serde::Serialize;
use std::path::{Path, PathBuf};

/// A ledger or chain that did not verify. `at` is its place in the data
/// home, which is made of ids, never a title or a file's name.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Broken {
    pub at: String,
    pub segment: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Integrity {
    pub at: DateTime<Utc>,
    /// Conversation ledgers and other record chains verified.
    pub conversations: u64,
    pub chains: u64,
    pub segments: u64,
    pub records: u64,
    /// Open segments ending in an unfinished record: a crash mid-write
    /// leaves one, and the next writer trims it. Not damage.
    pub torn_tails: u64,
    pub broken: Vec<Broken>,
    pub keys: u64,
    pub keys_destroyed: u64,
    pub keys_held: u64,
    pub receipts: u64,
    /// Receipts whose signature does not verify.
    pub receipts_unverified: u64,
    /// `current`, `behind` (rebuilding search fixes it) or `unreadable`.
    pub search: &'static str,
    /// Whether this process may still write (false after a restore until
    /// it is started again).
    pub fenced: bool,
}

impl Integrity {
    /// Nothing is damaged. Search being behind is not damage.
    pub fn sound(&self) -> bool {
        self.broken.is_empty() && self.receipts_unverified == 0
    }
}

/// Every directory of record segments under `root`, not looking inside
/// one, the object store or anything deeper than a ledger lies.
fn segment_dirs(root: &Path) -> Vec<PathBuf> {
    let mut found = Vec::new();
    let mut stack = vec![(root.to_path_buf(), 0u8)];
    while let Some((dir, depth)) = stack.pop() {
        if vak_session::verify::is_segment_dir(&dir) {
            found.push(dir);
            continue;
        }
        if depth >= 8 {
            continue;
        }
        for entry in std::fs::read_dir(&dir).into_iter().flatten().flatten() {
            let path = entry.path();
            let plain_dir = entry.file_type().is_ok_and(|kind| kind.is_dir());
            if plain_dir && entry.file_name() != "store" {
                stack.push((path, depth + 1));
            }
        }
    }
    found.sort();
    found
}

impl Core {
    /// Verifies the data home. Reads every stored record once.
    pub fn data_integrity(&self) -> Integrity {
        let home = self.shared_scope().into_root();
        let mut report = Integrity {
            at: Utc::now(),
            conversations: 0,
            chains: 0,
            segments: 0,
            records: 0,
            torn_tails: 0,
            broken: Vec::new(),
            keys: 0,
            keys_destroyed: 0,
            keys_held: 0,
            receipts: 0,
            receipts_unverified: 0,
            search: "unreadable",
            fenced: vak_session::fence::is_fenced(),
        };
        for dir in segment_dirs(&home) {
            let relative = dir.strip_prefix(&home).unwrap_or(&dir);
            if relative
                .components()
                .any(|part| part.as_os_str() == "sessions")
            {
                report.conversations += 1;
            } else {
                report.chains += 1;
            }
            let checked = vak_session::verify::verify(&dir);
            report.segments += checked.segments;
            report.records += checked.frames;
            report.torn_tails += u64::from(checked.torn_tail);
            if let Some(segment) = checked.broken_segment {
                report.broken.push(Broken {
                    at: relative.to_string_lossy().into_owned(),
                    segment,
                });
            }
        }
        if let Ok(tenant) = self.tenant_objects() {
            let (keys, destroyed) = tenant.scope_counts();
            report.keys = keys as u64;
            report.keys_destroyed = destroyed as u64;
            report.keys_held = tenant.held_count() as u64;
        }
        let receipts = self.erasure_receipts();
        report.receipts = receipts.len() as u64;
        report.receipts_unverified = receipts
            .iter()
            .filter(|receipt| !receipt.verifies())
            .count() as u64;
        if let Ok(catalog) = self.catalog() {
            report.search = match catalog.stale() {
                Ok(false) => "current",
                Ok(true) => "behind",
                Err(_) => "unreadable",
            };
        }
        report
    }
}
