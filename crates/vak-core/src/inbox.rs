//! Durable attention layer (docs/design/29-personal-os.md P6): every
//! `gateway::deliver` push also records an append-only entry at
//! `<home>/inbox.jsonl`, so unattended signals survive with no chat
//! channel configured. Read state is ack tombstone lines — nothing is
//! ever deleted or rewritten (invariant-2 adjacent). Retention note:
//! compaction of aged entries comes later; until then the ledger grows
//! monotonically and reads stay bounded by [`MAX_SCAN`].

use std::collections::VecDeque;
use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

/// Upper bound on raw lines examined per read. The ledger never shrinks
/// today, so unbounded scans would eventually dominate surface latency;
/// the newest window wins.
pub const MAX_SCAN: usize = 10_000;

#[derive(Debug, thiserror::Error)]
pub enum InboxError {
    #[error("io error on {path}: {source}")]
    Io {
        path: PathBuf,
        source: std::io::Error,
    },
    #[error("serialize inbox entry: {0}")]
    Serialize(String),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Kind {
    TaskSummary,
    ApprovalPending,
    ApprovalDenied,
    BudgetAlert,
    Digest,
    Heartbeat,
    ProposalOpened,
    /// A scheduled task was due and could not start; the body says why and
    /// what to do about it.
    RoutineFailed,
}

impl Kind {
    /// The serde snake_case tag, also used in id derivation so ids agree
    /// with whatever the JSONL on disk shows.
    fn tag(self) -> String {
        // Unit-variant serialization cannot fail; the error path exists
        // only to satisfy the no-unwrap rule.
        serde_json::to_string(&self)
            .map(|s| s.trim_matches('"').to_string())
            .unwrap_or_else(|_| format!("{self:?}"))
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Entry {
    pub id: String,
    pub ts: DateTime<Utc>,
    pub kind: Kind,
    pub title: String,
    pub body: String,
    #[serde(default)]
    pub session_id: Option<String>,
    #[serde(default)]
    pub task_id: Option<String>,
    #[serde(default)]
    pub result_id: Option<String>,
    #[serde(default)]
    pub dedupe_key: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub trace: Option<vak_session::trace::TraceKey>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub actor: Option<vak_session::ids::PrincipalId>,
}

vak_session::impl_traced!(Entry, "inbox_entry");

#[derive(Serialize, Deserialize)]
struct AckLine {
    ack_of: String,
    ts: DateTime<Utc>,
}

/// Entries plus a count of corrupt lines skipped while scanning — silent
/// for the caller's eye, but never invisible.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Scan {
    pub entries: Vec<Entry>,
    pub corrupt: usize,
}

#[derive(Default)]
struct Window {
    entries: Vec<Entry>,
    acked: Vec<String>,
    corrupt: usize,
}

/// FNV-1a 64-bit over `{ts}|{kind}|{title}`: fixed across toolchains like
/// every other house id. Hash collisions are tolerated as house style —
/// an id colliding merely merges two rows' read state.
fn entry_id(ts: DateTime<Utc>, kind: Kind, title: &str) -> String {
    fnv1a(format!("{}|{}|{}", ts.to_rfc3339(), kind.tag(), title).as_bytes())
}

fn fnv1a(data: &[u8]) -> String {
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for b in data {
        h ^= u64::from(*b);
        h = h.wrapping_mul(0x0000_0100_0000_01b3);
    }
    format!("{h:016x}")
}

pub fn inbox_path(home: &Path) -> PathBuf {
    home.join("inbox.jsonl")
}

/// Append one notification and return it. One formatted buffer + ONE
/// write_all: O_APPEND makes a single write atomic, whereas `writeln!`
/// emits several syscalls that concurrent deliverers could interleave
/// mid-line (same discipline as gateway.rs deliver_log).
pub fn record(
    home: &Path,
    kind: Kind,
    title: &str,
    body: &str,
    session_id: Option<&str>,
    task_id: Option<&str>,
) -> Result<Entry, InboxError> {
    record_with_result_and_key(home, kind, title, body, session_id, task_id, None, None)
}

/// Append a notification linked to one immutable presentation result.
pub fn record_with_result(
    home: &Path,
    kind: Kind,
    title: &str,
    body: &str,
    session_id: Option<&str>,
    task_id: Option<&str>,
    result_id: Option<&str>,
) -> Result<Entry, InboxError> {
    record_with_result_and_key(
        home, kind, title, body, session_id, task_id, result_id, None,
    )
}

/// Append a notification only once for a stable source/destination identity.
/// The check and append are serialized by the inbox file lock, so delivery
/// retries cannot create duplicate unread entries.
#[allow(clippy::too_many_arguments)]
pub fn record_with_result_and_key(
    home: &Path,
    kind: Kind,
    title: &str,
    body: &str,
    session_id: Option<&str>,
    task_id: Option<&str>,
    result_id: Option<&str>,
    dedupe_key: Option<&str>,
) -> Result<Entry, InboxError> {
    let _dedupe_lock = if dedupe_key.is_some() {
        Some(acquire_dedupe_lock(home)?)
    } else {
        None
    };
    if let Some(key) = dedupe_key
        && let Some(existing) = list(home, MAX_SCAN)
            .into_iter()
            .find(|entry| entry.dedupe_key.as_deref() == Some(key))
    {
        return Ok(existing);
    }
    let path = inbox_path(home);
    let ts = Utc::now();
    let entry = Entry {
        id: entry_id(ts, kind, title),
        ts,
        kind,
        title: title.to_string(),
        body: body.to_string(),
        session_id: session_id.map(str::to_string),
        task_id: task_id.map(str::to_string),
        result_id: result_id.map(str::to_string),
        dedupe_key: dedupe_key.map(str::to_string),
        trace: None,
        actor: None,
    };
    let line = serde_json::to_string(&entry).map_err(|e| InboxError::Serialize(e.to_string()))?;
    append_line(&path, &line)?;
    Ok(entry)
}

struct DedupeLock {
    path: PathBuf,
}

impl Drop for DedupeLock {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.path);
    }
}

fn acquire_dedupe_lock(home: &Path) -> Result<DedupeLock, InboxError> {
    let path = home.join("inbox.dedupe.lock");
    std::fs::create_dir_all(home).map_err(|source| InboxError::Io {
        path: home.to_path_buf(),
        source,
    })?;
    for _ in 0..200 {
        match std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&path)
        {
            Ok(_) => return Ok(DedupeLock { path }),
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {
                std::thread::sleep(std::time::Duration::from_millis(5));
            }
            Err(source) => return Err(InboxError::Io { path, source }),
        }
    }
    Err(InboxError::Io {
        path,
        source: std::io::Error::new(std::io::ErrorKind::TimedOut, "inbox dedupe lock timed out"),
    })
}

/// Mark `id` read by appending a tombstone. Idempotent via
/// read-before-write; returns false when already acked.
pub fn ack(home: &Path, id: &str) -> Result<bool, InboxError> {
    let path = inbox_path(home);
    if scan_window(&path).acked.iter().any(|a| a == id) {
        return Ok(false);
    }
    let line = serde_json::to_string(&AckLine {
        ack_of: id.to_string(),
        ts: Utc::now(),
    })
    .map_err(|e| InboxError::Serialize(e.to_string()))?;
    append_line(&path, &line)?;
    Ok(true)
}

pub fn unread(home: &Path) -> Vec<Entry> {
    unread_scanned(home).entries
}

/// Same as [`unread`] but reports how many corrupt lines were skipped.
pub fn unread_scanned(home: &Path) -> Scan {
    let window = scan_window(&inbox_path(home));
    let mut entries = window.entries;
    entries.retain(|e| !window.acked.contains(&e.id));
    entries.reverse();
    Scan {
        entries,
        corrupt: window.corrupt,
    }
}

/// All entries (acked included), newest first, capped at `limit`.
pub fn list(home: &Path, limit: usize) -> Vec<Entry> {
    list_scanned(home, limit).entries
}

pub fn list_scanned(home: &Path, limit: usize) -> Scan {
    let mut window = scan_window(&inbox_path(home));
    window.entries.reverse();
    window.entries.truncate(limit);
    Scan {
        entries: window.entries,
        corrupt: window.corrupt,
    }
}

pub fn unread_count(home: &Path) -> usize {
    unread(home).len()
}

fn append_line(path: &Path, line: &str) -> Result<(), InboxError> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|e| InboxError::Io {
            path: parent.to_path_buf(),
            source: e,
        })?;
    }
    let mut buf = String::with_capacity(line.len() + 1);
    buf.push_str(line);
    buf.push('\n');
    let mut f = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)
        .map_err(|e| InboxError::Io {
            path: path.to_path_buf(),
            source: e,
        })?;
    f.write_all(buf.as_bytes()).map_err(|e| InboxError::Io {
        path: path.to_path_buf(),
        source: e,
    })
}

/// Last MAX_SCAN raw lines (oldest→newest within the window); corrupt
/// lines are counted, not fatal. Missing file reads as empty. Entries and
/// ack tombstones come from the same window so read state stays
/// consistent with what a surface can see.
fn scan_window(path: &Path) -> Window {
    let mut window: VecDeque<String> = VecDeque::with_capacity(64);
    let mut torn_reads = 0usize;
    if let Ok(f) = std::fs::File::open(path) {
        for line in BufReader::new(f).lines() {
            match line {
                Ok(l) => {
                    if window.len() == MAX_SCAN {
                        window.pop_front();
                    }
                    window.push_back(l);
                }
                Err(_) => {
                    // A torn read mid-line cannot continue by contract of
                    // BufRead::lines; count it and stop.
                    torn_reads += 1;
                    break;
                }
            }
        }
    }
    let mut out = Window::default();
    out.corrupt += torn_reads;
    for line in &window {
        if line.trim().is_empty() {
            continue;
        }
        if let Ok(e) = serde_json::from_str::<Entry>(line) {
            out.entries.push(e);
        } else if let Ok(a) = serde_json::from_str::<AckLine>(line) {
            out.acked.push(a.ack_of);
        } else {
            out.corrupt += 1;
        }
    }
    out
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]

    use super::*;

    #[test]
    fn record_unread_roundtrip_newest_first() {
        let dir = tempfile::tempdir().unwrap();
        let home = dir.path();

        assert!(unread(home).is_empty());
        assert_eq!(unread_count(home), 0);

        let a = record(
            home,
            Kind::TaskSummary,
            "first",
            "body a",
            Some("sess-1"),
            None,
        )
        .unwrap();
        let b = record(
            home,
            Kind::BudgetAlert,
            "second",
            "body b",
            None,
            Some("task-9"),
        )
        .unwrap();

        assert_eq!(a.session_id.as_deref(), Some("sess-1"));
        assert_eq!(b.task_id.as_deref(), Some("task-9"));
        assert_ne!(a.id, b.id);

        let items = unread(home);
        assert_eq!(items.len(), 2);
        assert_eq!(items[0], b);
        assert_eq!(items[1], a);
        assert_eq!(unread_count(home), 2);
        assert!(list(home, 10).contains(&a));
    }

    #[test]
    fn ack_is_idempotent_and_shrinks_unread_once() {
        let dir = tempfile::tempdir().unwrap();
        let home = dir.path();
        let e = record(home, Kind::Digest, "daily", "", None, None).unwrap();
        let other = record(home, Kind::Heartbeat, "beat", "", None, None).unwrap();

        assert!(ack(home, &e.id).unwrap());
        assert!(!ack(home, &e.id).unwrap());

        let left = unread(home);
        assert_eq!(left, vec![other]);
        assert_eq!(unread_count(home), 1);

        assert!(!ack(home, &e.id).unwrap());
        assert_eq!(unread_count(home), 1);
    }

    #[test]
    fn acked_excluded_from_unread_but_present_in_list() {
        let dir = tempfile::tempdir().unwrap();
        let home = dir.path();
        let e = record(
            home,
            Kind::ApprovalPending,
            "gate",
            "wants write",
            None,
            None,
        )
        .unwrap();
        ack(home, &e.id).unwrap();

        assert!(unread(home).is_empty());
        let all = list(home, 10);
        assert_eq!(all.len(), 1);
        assert_eq!(all[0].id, e.id);
        assert_eq!(all[0].kind, Kind::ApprovalPending);
    }

    #[test]
    fn corrupt_middle_line_is_skipped_and_counted() {
        let dir = tempfile::tempdir().unwrap();
        let home = dir.path();
        let first = record(home, Kind::Digest, "one", "", None, None).unwrap();
        let mut raw = std::fs::read_to_string(inbox_path(home)).unwrap();
        raw.push_str("{\"id\": torn line no json\n");
        std::fs::write(inbox_path(home), &raw).unwrap();
        let last = record(home, Kind::BudgetAlert, "two", "", None, None).unwrap();

        let scan = unread_scanned(home);
        assert_eq!(scan.corrupt, 1);
        assert_eq!(scan.entries, vec![last.clone(), first.clone()]);
        let listed = list_scanned(home, 10);
        assert_eq!(listed.corrupt, 1);
        // Ack still works with garbage in the middle.
        assert!(ack(home, &first.id).unwrap());
        assert_eq!(unread_scanned(home).entries, vec![last]);
    }

    #[test]
    fn interleaved_writers_keep_line_integrity_and_order() {
        let dir = tempfile::tempdir().unwrap();
        let home = dir.path();
        let home_a = home.to_path_buf();
        let home_b = home.to_path_buf();

        let writer = |home: PathBuf, tag: &str| -> Vec<String> {
            (0..25)
                .map(|i| {
                    record(
                        home.as_path(),
                        Kind::TaskSummary,
                        &format!("{tag}-{i}"),
                        "x",
                        None,
                        None,
                    )
                    .unwrap()
                    .id
                })
                .collect()
        };
        let h1 = std::thread::spawn(move || writer(home_a, "a"));
        let h2 = std::thread::spawn(move || writer(home_b, "b"));
        let ids_a = h1.join().unwrap();
        let ids_b = h2.join().unwrap();

        let on_disk = unread_scanned(home);
        assert_eq!(
            on_disk.corrupt, 0,
            "no torn lines under O_APPEND single writes"
        );
        assert_eq!(on_disk.entries.len(), 50);

        // Each writer's own subsequence must appear in its recorded order.
        // `on_disk.entries` is newest-first, so later writes sit earlier.
        let position_of = |id: &str| {
            on_disk
                .entries
                .iter()
                .position(|e| e.id == id)
                .unwrap_or(usize::MAX)
        };
        for pair in ids_a.windows(2) {
            assert!(
                position_of(&pair[1]) < position_of(&pair[0]),
                "writer A reordered"
            );
        }
        for pair in ids_b.windows(2) {
            assert!(
                position_of(&pair[1]) < position_of(&pair[0]),
                "writer B reordered"
            );
        }
    }

    #[test]
    fn ids_are_deterministic_fnv_over_ts_kind_title() {
        let fixed: DateTime<Utc> = "2026-08-25T00:00:00Z".parse().unwrap();
        assert_eq!(
            entry_id(fixed, Kind::Digest, "weekly digest"),
            "76391567a848733e"
        );
        assert_eq!(
            entry_id(fixed, Kind::Digest, "weekly digest"),
            entry_id(fixed, Kind::Digest, "weekly digest")
        );
        assert_ne!(
            entry_id(fixed, Kind::Digest, "a"),
            entry_id(fixed, Kind::Digest, "b")
        );
        assert_ne!(
            entry_id(fixed, Kind::Heartbeat, "a"),
            entry_id(fixed, Kind::Digest, "a")
        );

        // The serde snake_case tags feed the id and the wire format.
        let line = serde_json::to_string(&Kind::ProposalOpened).unwrap();
        assert_eq!(line, "\"proposal_opened\"");
        let back: Kind = serde_json::from_str(&line).unwrap();
        assert_eq!(back, Kind::ProposalOpened);
    }

    #[test]
    fn result_delivery_deduplicates_by_source_key() {
        let dir = tempfile::tempdir().unwrap();
        let first = record_with_result_and_key(
            dir.path(),
            Kind::TaskSummary,
            "finished",
            "answer",
            Some("session"),
            Some("task"),
            Some("result-1"),
            Some("surface:chat|result-1|0"),
        )
        .unwrap();
        let second = record_with_result_and_key(
            dir.path(),
            Kind::TaskSummary,
            "finished",
            "answer changed",
            Some("session"),
            Some("task"),
            Some("result-1"),
            Some("surface:chat|result-1|0"),
        )
        .unwrap();
        assert_eq!(first.id, second.id);
        assert_eq!(list(dir.path(), 10).len(), 1);
    }
}
