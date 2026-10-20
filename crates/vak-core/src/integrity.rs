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
    /// Coverage failures prevent a healthy verdict; paths contain only state ids.
    pub unreadable: Vec<String>,
    /// Payload classes outside this keyless record/receipt check.
    pub unchecked_classes: Vec<&'static str>,
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
    /// The covered records and receipts passed, with no inventory/read failure.
    /// `unchecked_classes` remain outside this verdict; stale search is not damage.
    pub fn sound(&self) -> bool {
        self.broken.is_empty()
            && self.receipts_unverified == 0
            && self.unreadable.is_empty()
            && self.search != "unreadable"
    }
}

/// Every directory of record segments under `root`, not looking inside
/// one or state classes that the registry excludes from record verification.
fn segment_dirs(root: &Path) -> (Vec<PathBuf>, Vec<PathBuf>) {
    let mut found = Vec::new();
    let mut unreadable = Vec::new();
    let mut stack = vec![root.to_path_buf()];
    while let Some(dir) = stack.pop() {
        let relative = dir.strip_prefix(root).unwrap_or(&dir);
        // Only authoritative record trees belong in this inventory. In
        // particular, never walk an Agent's editable workspace or a cache.
        if crate::state::entries_for(crate::state::Root::Data)
            .any(|entry| entry.class != crate::state::Class::Record && entry.matches(relative))
        {
            continue;
        }
        let entries = match std::fs::read_dir(&dir) {
            Ok(entries) => entries,
            Err(_) => {
                unreadable.push(dir);
                continue;
            }
        };
        let mut children = Vec::new();
        let mut chain = false;
        for entry in entries {
            let Ok(entry) = entry else {
                unreadable.push(dir.clone());
                continue;
            };
            let name = entry.file_name();
            chain |=
                name == "seals.log" || name.to_str().is_some_and(|name| name.starts_with("seg-"));
            match entry.file_type() {
                Ok(kind) if kind.is_dir() && name != "store" => children.push(entry.path()),
                Ok(kind) if kind.is_symlink() => unreadable.push(entry.path()),
                Ok(_) => {}
                Err(_) => unreadable.push(entry.path()),
            }
        }
        if chain {
            found.push(dir);
        } else {
            stack.extend(children);
        }
    }
    found.sort();
    (found, unreadable)
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
            unreadable: Vec::new(),
            unchecked_classes: vec![
                "tenant_object_payloads",
                "scope_key_payloads",
                "derived_index_payloads",
            ],
            keys: 0,
            keys_destroyed: 0,
            keys_held: 0,
            receipts: 0,
            receipts_unverified: 0,
            search: "unreadable",
            fenced: vak_session::fence::is_fenced(),
        };
        let (directories, unreadable) = segment_dirs(&home);
        report.unreadable = unreadable
            .iter()
            .map(|path| {
                path.strip_prefix(&home)
                    .unwrap_or(path)
                    .to_string_lossy()
                    .into_owned()
            })
            .collect();
        for dir in directories {
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
        } else {
            report.unreadable.push("tenant_keys".into());
        }
        let receipts = match self.try_erasure_receipts() {
            Ok(receipts) => receipts,
            Err(_) => {
                report.unreadable.push("erasure_receipts".into());
                Vec::new()
            }
        };
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
