//! Durable memory notes (docs/design/26-learning.md): one plain-markdown
//! file per workspace under `<home>/memory/<hash>/MEMORY.md`, plus a global
//! user-profile tier at `<home>/memory/user/USER.md`
//! (docs/design/29-personal-os.md P1). The model appends through the
//! `remember` tool; humans edit or prune the file directly — the parser
//! tolerates hand-edits rather than fighting them. Forget/amend rewrite the
//! markdown file in place; sessions stay append-only.

use std::path::{Path, PathBuf};
use std::time::{Duration, Instant, SystemTime};

use chrono::{DateTime, Utc};

/// FNV-1a 64-bit, matching vak-session's cwd hashing so every per-workspace
/// store keys off the same identity.
pub fn hash_cwd(cwd: &Path) -> String {
    fnv1a(cwd.to_string_lossy().as_bytes())
}

fn fnv1a(data: &[u8]) -> String {
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for b in data {
        h ^= u64::from(*b);
        h = h.wrapping_mul(0x0000_0100_0000_01b3);
    }
    format!("{h:016x}")
}

/// Stable note id: FNV-1a 64 of the full header line (without any trailing
/// newline/carriage-return). Deterministic across processes and toolchains,
/// so list/forget/amend agree without extra state; hand-editing a header
/// deliberately changes its id.
fn note_id(header_line: &str) -> String {
    fnv1a(header_line.as_bytes())
}

#[derive(Debug, Clone, PartialEq)]
pub struct NoteBlock {
    pub id: String,
    pub ts: DateTime<Utc>,
    pub kind: String,
    pub tag: String,
    pub session_id: String,
    pub text: String,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct CleanupReport {
    pub removed_locks: usize,
    pub removed_temps: usize,
    pub removed_empty_dirs: usize,
}

/// Remove only abandoned write artifacts and empty workspace directories.
/// Durable note files are never age-pruned: retention is an explicit
/// forget/amend decision so an old fact cannot disappear silently.
pub fn cleanup_artifacts(home: &Path, older_than: Duration) -> CleanupReport {
    let root = home.join("memory");
    let Ok(entries) = std::fs::read_dir(&root) else {
        return CleanupReport::default();
    };
    let cutoff = SystemTime::now().checked_sub(older_than);
    let mut report = CleanupReport::default();
    for entry in entries.flatten() {
        let path = entry.path();
        if !path.is_dir() || path.file_name().is_some_and(|n| n == "user") {
            continue;
        }
        let Ok(files) = std::fs::read_dir(&path) else {
            continue;
        };
        for file in files.flatten() {
            let file_path = file.path();
            let name = file_path
                .file_name()
                .and_then(|n| n.to_str())
                .unwrap_or_default();
            let artifact = name.ends_with(".lock") || name.contains(".tmp.");
            let old = cutoff
                .zip(
                    std::fs::metadata(&file_path)
                        .and_then(|m| m.modified())
                        .ok(),
                )
                .is_some_and(|(cutoff, modified)| modified <= cutoff);
            if artifact && old && std::fs::remove_file(&file_path).is_ok() {
                if name.ends_with(".lock") {
                    report.removed_locks += 1;
                } else {
                    report.removed_temps += 1;
                }
            }
        }
        if std::fs::read_dir(&path)
            .map(|mut d| d.next().is_none())
            .unwrap_or(false)
            && std::fs::remove_dir(&path).is_ok()
        {
            report.removed_empty_dirs += 1;
        }
    }
    report
}

fn memory_path(home: &Path, cwd: &Path) -> PathBuf {
    home.join("memory").join(hash_cwd(cwd)).join("MEMORY.md")
}

/// Global profile-tier store, keyed to the user rather than a workspace.
pub fn profile_path(home: &Path) -> PathBuf {
    home.join("memory").join("user").join("USER.md")
}

pub fn append_note(
    home: &Path,
    cwd: &Path,
    kind: &str,
    tag: &str,
    session_id: &str,
    text: &str,
) -> Result<NoteBlock, String> {
    append_block(&memory_path(home, cwd), kind, tag, session_id, text)
}

/// Append to the global USER.md profile tier (same grammar, same validation,
/// lazy directory creation) — memories that follow the user across projects.
pub fn append_profile_note(
    home: &Path,
    kind: &str,
    tag: &str,
    text: &str,
    session: &str,
) -> Result<NoteBlock, String> {
    append_block(&profile_path(home), kind, tag, session, text)
}

fn append_block(
    path: &Path,
    kind: &str,
    tag: &str,
    session_id: &str,
    text: &str,
) -> Result<NoteBlock, String> {
    if !matches!(
        kind,
        "fact" | "decision" | "preference" | "reference" | "note" | "invariant" | "procedural"
    ) {
        return Err("unknown note kind; expected fact, decision, preference, reference, invariant, or procedural".into());
    }
    validate_field("kind", kind, 32)?;
    validate_field("tag", tag, 96)?;
    validate_field("session", session_id, 160)?;
    let text = text.trim();
    if text.is_empty() {
        return Err("note must not be empty".into());
    }
    if text.len() > 128 * 1024 {
        return Err("note exceeds the 128 KiB limit".into());
    }
    let ts = Utc::now();
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|e| format!("create memory dir: {e}"))?;
    }
    let _lock = StoreLock::acquire(path)?;
    let header = format!(
        "## {} [{kind}] tag={tag} session={session_id}",
        ts.to_rfc3339()
    );
    let block = format!("{header}\n{text}\n");
    use std::io::Write;
    let mut f = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)
        .map_err(|e| format!("open {}: {e}", path.display()))?;
    f.write_all(format!("{block}\n").as_bytes())
        .map_err(|e| format!("append {}: {e}", path.display()))?;
    f.sync_data()
        .map_err(|e| format!("flush {}: {e}", path.display()))?;
    Ok(NoteBlock {
        id: note_id(&header),
        ts,
        kind: kind.to_string(),
        tag: tag.to_string(),
        session_id: session_id.to_string(),
        text: text.to_string(),
    })
}

fn validate_field(name: &str, value: &str, max: usize) -> Result<(), String> {
    if value.len() > max
        || value
            .chars()
            .any(|c| c.is_control() || c == '\n' || c == '\r')
    {
        return Err(format!("{name} contains invalid characters or is too long"));
    }
    Ok(())
}

struct StoreLock {
    path: PathBuf,
}

impl StoreLock {
    fn acquire(store: &Path) -> Result<Self, String> {
        let path = store.with_extension("lock");
        let deadline = Instant::now() + Duration::from_secs(10);
        loop {
            match std::fs::OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(&path)
            {
                Ok(mut file) => {
                    use std::io::Write;
                    let _ = writeln!(file, "pid={}", std::process::id());
                    return Ok(Self { path });
                }
                Err(e)
                    if e.kind() == std::io::ErrorKind::AlreadyExists
                        && Instant::now() < deadline =>
                {
                    if std::fs::metadata(&path)
                        .and_then(|m| m.modified())
                        .ok()
                        .and_then(|t| t.elapsed().ok())
                        .is_some_and(|age| age > Duration::from_secs(60))
                    {
                        let _ = std::fs::remove_file(&path);
                        continue;
                    }
                    std::thread::sleep(Duration::from_millis(10));
                }
                Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {
                    return Err(format!("timed out waiting for {}", path.display()));
                }
                Err(e) => return Err(format!("lock {}: {e}", path.display())),
            }
        }
    }
}

impl Drop for StoreLock {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.path);
    }
}

/// Parse MEMORY.md blocks. Tolerant by design: lines before the first
/// heading and malformed headings attach to whatever precedes them, so a
/// hand-edit never silently loses content.
///
/// Reads exactly `home`'s memory. An Agent's memory is private
/// (invariant 37); an empty home is empty, never a reason to read another
/// Agent's notes.
pub fn list_notes(home: &Path, cwd: &Path) -> Vec<NoteBlock> {
    blocks_at(&memory_path(home, cwd))
}

/// Parse the global profile tier of exactly `home`.
pub fn list_profile_notes(home: &Path) -> Vec<NoteBlock> {
    blocks_at(&profile_path(home))
}

fn blocks_at(path: &Path) -> Vec<NoteBlock> {
    let Ok(raw) = std::fs::read_to_string(path) else {
        return Vec::new();
    };
    parse_blocks(&raw)
}

pub fn parse_blocks(raw: &str) -> Vec<NoteBlock> {
    struct Open {
        header_line: String,
        ts: DateTime<Utc>,
        kind: String,
        tag: String,
        session_id: String,
    }

    fn flush(open: &mut Option<Open>, body: &mut String, out: &mut Vec<NoteBlock>) {
        if let Some(o) = open.take() {
            let text = body.trim().to_string();
            if !text.is_empty() {
                out.push(NoteBlock {
                    id: note_id(&o.header_line),
                    ts: o.ts,
                    kind: o.kind,
                    tag: o.tag,
                    session_id: o.session_id,
                    text,
                });
            }
        }
        body.clear();
    }

    let mut out: Vec<NoteBlock> = Vec::new();
    let mut open: Option<Open> = None;
    let mut body = String::new();

    for line in raw.lines() {
        if let Some(rest) = line.strip_prefix("## ") {
            match parse_header(rest) {
                Some((ts, kind, tag, sid)) => {
                    flush(&mut open, &mut body, &mut out);
                    open = Some(Open {
                        header_line: line.to_string(),
                        ts,
                        kind,
                        tag,
                        session_id: sid,
                    });
                }
                None => {
                    // Not our grammar: treat as continuation text of the
                    // previous block (or drop leading junk).
                    if open.is_some() {
                        body.push_str(line);
                        body.push('\n');
                    }
                }
            }
        } else if open.is_some() {
            body.push_str(line);
            body.push('\n');
        }
    }
    flush(&mut open, &mut body, &mut out);
    out
}

/// Byte span of each parsed note in `raw`, mirroring parse_blocks exactly:
/// a block runs from its valid `## ` header through the newline of its last
/// non-blank body line; malformed headings count as body text; headers with
/// empty bodies yield no span (they yield no note either).
struct Span {
    id: String,
    start: usize,
    end: usize,
}

fn note_spans(raw: &str) -> Vec<Span> {
    let pieces: Vec<&str> = raw.split('\n').collect();
    let mut starts = Vec::with_capacity(pieces.len());
    let mut off = 0usize;
    for p in &pieces {
        starts.push(off);
        off += p.len() + 1;
    }

    fn close(
        open: &mut Option<(String, usize)>,
        last_content: &mut Option<usize>,
        starts: &[usize],
        pieces: &[&str],
        raw_len: usize,
        out: &mut Vec<Span>,
    ) {
        if let Some((id, h)) = open.take()
            && let Some(l) = last_content.take()
        {
            let end = (starts[l] + pieces[l].len() + 1).min(raw_len);
            out.push(Span {
                id,
                start: starts[h],
                end,
            });
        }
    }

    let mut out: Vec<Span> = Vec::new();
    let mut open: Option<(String, usize)> = None;
    let mut last_content: Option<usize> = None;
    for (i, piece) in pieces.iter().enumerate() {
        let line = piece.strip_suffix('\r').unwrap_or(piece);
        if let Some(rest) = line.strip_prefix("## ")
            && parse_header(rest).is_some()
        {
            close(
                &mut open,
                &mut last_content,
                &starts,
                &pieces,
                raw.len(),
                &mut out,
            );
            open = Some((note_id(line), i));
        } else if open.is_some() && !line.trim().is_empty() {
            last_content = Some(i);
        }
    }
    close(
        &mut open,
        &mut last_content,
        &starts,
        &pieces,
        raw.len(),
        &mut out,
    );
    out
}

fn load_raw(path: &Path) -> Result<String, String> {
    std::fs::read_to_string(path).map_err(|e| format!("read {}: {e}", path.display()))
}

fn persist_locked(path: &Path, contents: String) -> Result<(), String> {
    let tmp = path.with_extension(format!("tmp.{}", std::process::id()));
    let result = (|| {
        let mut f =
            std::fs::File::create(&tmp).map_err(|e| format!("rewrite {}: {e}", path.display()))?;
        use std::io::Write;
        f.write_all(contents.as_bytes())
            .map_err(|e| format!("rewrite {}: {e}", path.display()))?;
        f.sync_all()
            .map_err(|e| format!("rewrite {}: {e}", path.display()))?;
        std::fs::rename(&tmp, path).map_err(|e| format!("replace {}: {e}", path.display()))
    })();
    if result.is_err() {
        let _ = std::fs::remove_file(&tmp);
    }
    result
}

/// Rewrite-not-tombstone forget: removes exactly one block (the first whose
/// id matches) and returns the number of bytes deleted. Everything outside
/// the removed span — preambles, blank separators, hand-edited prose — is
/// preserved byte-for-byte.
pub fn forget_note(path: &Path, note_id: &str) -> Result<usize, String> {
    let _lock = StoreLock::acquire(path)?;
    let raw = load_raw(path)?;
    for span in note_spans(&raw) {
        if span.id != note_id {
            continue;
        }
        let mut out = String::with_capacity(raw.len() - (span.end - span.start));
        out.push_str(&raw[..span.start]);
        out.push_str(&raw[span.end..]);
        persist_locked(path, out)?;
        return Ok(span.end - span.start);
    }
    Err(format!("no note '{note_id}' in {}", path.display()))
}

/// Replace one block's body while keeping its provenance header line
/// verbatim — including any unknown/garbled tokens a hand-edit introduced.
pub fn amend_note(path: &Path, note_id: &str, new_text: &str) -> Result<(), String> {
    let text = new_text.trim();
    if text.is_empty() {
        return Err("note must not be empty".into());
    }
    if text.len() > 128 * 1024 {
        return Err("note exceeds the 128 KiB limit".into());
    }
    let _lock = StoreLock::acquire(path)?;
    let raw = load_raw(path)?;
    for span in note_spans(&raw) {
        if span.id != note_id {
            continue;
        }
        let old = &raw[span.start..span.end];
        let header_end = old.find('\n').unwrap_or(old.len());
        let rebuilt = format!("{}\n{text}\n", &old[..header_end]);
        let mut out = String::with_capacity(raw.len() - old.len() + rebuilt.len());
        out.push_str(&raw[..span.start]);
        out.push_str(&rebuilt);
        out.push_str(&raw[span.end..]);
        return persist_locked(path, out);
    }
    Err(format!("no note '{note_id}' in {}", path.display()))
}

/// `2026-08-23T12:00:00+00:00 [decision] tag=x session=abc` → parts.
fn parse_header(rest: &str) -> Option<(DateTime<Utc>, String, String, String)> {
    let mut parts = rest.splitn(2, char::is_whitespace);
    let ts_raw = parts.next()?.trim();
    let ok_ts = DateTime::parse_from_rfc3339(ts_raw).ok()?;
    let tail = parts.next().unwrap_or("").trim();

    let mut kind = "note".to_string();
    let mut remainder = tail;
    if let Some(after) = tail.strip_prefix('[')
        && let Some((inner, r)) = after.split_once(']')
    {
        kind = inner.trim().to_string();
        remainder = r.trim();
    }

    let mut tag = String::new();
    let mut session = String::new();
    let mut leftover = String::new();
    for token in remainder.split_whitespace() {
        if let Some(v) = token.strip_prefix("tag=") {
            tag = v.to_string();
        } else if let Some(v) = token.strip_prefix("session=") {
            session = v.to_string();
        } else if !token.is_empty() {
            leftover.push_str(token);
            leftover.push(' ');
        }
    }
    // Unknown trailing tokens are preserved as a pseudo-tag so nothing a
    // human typed disappears.
    if tag.is_empty() && !leftover.trim().is_empty() {
        tag = leftover.trim().to_string();
    }
    Some((ok_ts.with_timezone(&Utc), kind, tag, session))
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]
    use super::*;

    #[test]
    fn append_then_parse_roundtrips() {
        let dir = tempfile::tempdir().unwrap();
        let home = dir.path();
        let cwd = dir.path().join("ws");
        std::fs::create_dir_all(&cwd).unwrap();

        let b = append_note(
            home,
            &cwd,
            "decision",
            "deploy",
            "sess-1",
            "pause before rollbacks",
        )
        .unwrap();
        let notes = list_notes(home, &cwd);
        assert_eq!(notes.len(), 1);
        assert_eq!(notes[0], b);
        assert_eq!(notes[0].kind, "decision");
        assert_eq!(notes[0].tag, "deploy");
    }

    #[test]
    fn parser_tolerates_hand_edits_and_junk() {
        let raw = "leading junk line\n\n## not a real header just text\nmore prose\n\n## 2026-08-23T10:00:00+00:00 [fact] session=abc\nthe deploy script lives in scripts/deploy.sh\ncustom user line\n";
        let blocks = parse_blocks(raw);
        // The malformed heading becomes part of the leading-junk block? No:
        // it precedes any valid header, so only the valid block survives.
        assert_eq!(blocks.len(), 1);
        assert_eq!(blocks[0].kind, "fact");
        assert_eq!(blocks[0].session_id, "abc");
        assert!(blocks[0].text.contains("deploy.sh"));
        assert!(blocks[0].text.contains("custom user line"));
    }

    #[test]
    fn empty_note_rejected() {
        let dir = tempfile::tempdir().unwrap();
        let err = append_note(dir.path(), dir.path(), "fact", "", "s", "   ");
        assert!(err.is_err());
    }

    #[test]
    fn header_metadata_rejects_control_characters() {
        let dir = tempfile::tempdir().unwrap();
        assert!(append_note(dir.path(), dir.path(), "fact", "bad\ntag", "s", "x").is_err());
        assert!(append_note(dir.path(), dir.path(), "fact", "ok", "s\n2", "x").is_err());
    }

    #[test]
    fn malformed_heading_remains_in_preceding_note() {
        let raw = "## 2026-08-23T10:00:00+00:00 [fact] session=s\nfirst\n## not metadata\nmore\n";
        let notes = parse_blocks(raw);
        assert_eq!(notes.len(), 1);
        assert!(notes[0].text.contains("## not metadata"));
        assert!(notes[0].text.contains("more"));
    }

    #[test]
    fn concurrent_appends_keep_every_block_intact() {
        let dir = tempfile::tempdir().unwrap();
        let home = dir.path().to_path_buf();
        let cwd = dir.path().join("ws");
        std::fs::create_dir_all(&cwd).unwrap();
        let mut joins = Vec::new();
        for i in 0..16 {
            let home = home.clone();
            let cwd = cwd.clone();
            joins.push(std::thread::spawn(move || {
                append_note(
                    &home,
                    &cwd,
                    "fact",
                    "parallel",
                    &format!("s{i}"),
                    &format!("note-{i}"),
                )
                .unwrap();
            }));
        }
        for join in joins {
            join.join().unwrap();
        }
        assert_eq!(list_notes(&home, &cwd).len(), 16);
    }

    #[test]
    fn forget_removes_exactly_one_block_preserving_rest_byte_for_byte() {
        let dir = tempfile::tempdir().unwrap();
        let home = dir.path();
        let cwd = dir.path().join("ws");

        let a = append_note(home, &cwd, "fact", "x", "s1", "alpha note").unwrap();
        let b = append_note(home, &cwd, "decision", "y", "s2", "beta note").unwrap();

        let block_a = format!(
            "## {} [fact] tag=x session=s1\nalpha note\n",
            a.ts.to_rfc3339()
        );
        let block_b = format!(
            "## {} [decision] tag=y session=s2\nbeta note\n",
            b.ts.to_rfc3339()
        );
        let path = home.join("memory").join(hash_cwd(&cwd)).join("MEMORY.md");
        assert_eq!(
            std::fs::read_to_string(&path).unwrap(),
            format!("{block_a}\n{block_b}\n")
        );

        let removed = forget_note(&path, &a.id).unwrap();
        assert_eq!(removed, block_a.len());
        assert_eq!(
            std::fs::read_to_string(&path).unwrap(),
            format!("\n{block_b}\n")
        );

        let notes = list_notes(home, &cwd);
        assert_eq!(notes.len(), 1);
        assert_eq!(notes[0].id, b.id);

        assert!(forget_note(&path, &a.id).is_err());
        assert_eq!(
            std::fs::read_to_string(&path).unwrap(),
            format!("\n{block_b}\n")
        );
    }

    #[test]
    fn amend_swaps_body_and_preserves_provenance_header() {
        let dir = tempfile::tempdir().unwrap();
        let home = dir.path();
        let cwd = dir.path().join("ws");

        let n = append_note(home, &cwd, "fact", "", "s3", "original body").unwrap();
        let header = format!("## {} [fact] tag= session=s3", n.ts.to_rfc3339());
        let path = home.join("memory").join(hash_cwd(&cwd)).join("MEMORY.md");

        amend_note(&path, &n.id, "revised body with more detail").unwrap();

        assert_eq!(
            std::fs::read_to_string(&path).unwrap(),
            format!("{header}\nrevised body with more detail\n\n")
        );
        let notes = list_notes(home, &cwd);
        assert_eq!(notes.len(), 1);
        assert_eq!(notes[0].id, n.id);
        assert_eq!(notes[0].ts, n.ts);
        assert_eq!(notes[0].kind, n.kind);
        assert_eq!(notes[0].tag, n.tag);
        assert_eq!(notes[0].session_id, n.session_id);
        assert_eq!(notes[0].text, "revised body with more detail");

        assert!(amend_note(&path, &n.id, "   ").is_err());
        assert!(amend_note(&path, "deadbeef", "x").is_err());
    }

    #[test]
    fn amend_preserves_hand_edited_header_tokens() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("USER.md");
        let raw = "## 2026-08-23T10:00:00+00:00 [fact] tag=custom=foo session=abc mystery-token\ncold body\n";
        std::fs::write(&path, raw).unwrap();
        let id = parse_blocks(raw)[0].id.clone();

        amend_note(&path, &id, "warm body").unwrap();
        let after = std::fs::read_to_string(&path).unwrap();
        assert!(after.starts_with(
            "## 2026-08-23T10:00:00+00:00 [fact] tag=custom=foo session=abc mystery-token\n"
        ));
        assert!(after.ends_with("warm body\n"));
    }

    #[test]
    fn forget_leaves_hand_edited_garbage_between_blocks_intact() {
        let dir = tempfile::tempdir().unwrap();
        let home = dir.path();
        let cwd = dir.path().join("ws");
        std::fs::create_dir_all(&cwd).unwrap();
        let path = home.join("memory").join(hash_cwd(&cwd)).join("MEMORY.md");
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        let raw = "prelude scribble\n\n## 2026-08-23T10:00:00+00:00 [fact] tag=a session=s1\nfirst note\nHUMAN GARBAGE ## fake header here\nmore scribble\n\n## 2026-08-23T11:00:00+00:00 [decision] tag=b session=s2\nsecond note";
        std::fs::write(&path, raw).unwrap();

        let notes = list_notes(home, &cwd);
        assert_eq!(notes.len(), 2);
        assert!(notes[0].text.contains("HUMAN GARBAGE"));

        let removed = forget_note(&path, &notes[1].id).unwrap();
        let after = std::fs::read_to_string(&path).unwrap();
        assert_eq!(removed, raw.len() - after.len());
        assert_eq!(after, &raw[..raw.len() - removed]);
        assert!(after.contains("HUMAN GARBAGE"));

        let removed2 = forget_note(&path, &notes[0].id).unwrap();
        assert_eq!(
            std::fs::read_to_string(&path).unwrap(),
            "prelude scribble\n\n\n"
        );
        assert!(list_notes(home, &cwd).is_empty());
        assert_eq!(
            removed + removed2 + "prelude scribble\n\n\n".len(),
            raw.len()
        );
    }

    #[test]
    fn profile_tier_roundtrips() {
        let dir = tempfile::tempdir().unwrap();
        let home = dir.path();

        assert_eq!(
            profile_path(home),
            home.join("memory").join("user").join("USER.md")
        );
        assert!(list_profile_notes(home).is_empty());

        let p = append_profile_note(
            home,
            "preference",
            "editor",
            "prefers vim bindings",
            "sess-u",
        )
        .unwrap();
        let q =
            append_profile_note(home, "fact", "timezone", "works in UTC+5:30", "sess-u").unwrap();

        let notes = list_profile_notes(home);
        assert_eq!(notes.len(), 2);
        assert_eq!(notes[0], p);
        assert_eq!(notes[1], q);
        assert_eq!(notes[0].tag, "editor");

        let again = list_profile_notes(home);
        assert_eq!(again[0].id, p.id);
        assert_eq!(again[1].id, q.id);

        let cwd = dir.path().join("ws");
        std::fs::create_dir_all(&cwd).unwrap();
        assert!(list_notes(home, &cwd).is_empty());

        assert!(append_profile_note(home, "fact", "", "  ", "sess-u").is_err());
    }

    #[test]
    fn load_matrix_scales_real_store_from_low_to_high() {
        let dir = tempfile::tempdir().unwrap();
        let home = dir.path();
        let cwd = home.join("load-workspace");
        std::fs::create_dir_all(&cwd).unwrap();
        let mut expected = 0;
        for (label, count) in [("low", 10usize), ("medium", 500), ("high", 4_096)] {
            for i in 0..count {
                append_note(
                    home,
                    &cwd,
                    "fact",
                    label,
                    "load-test",
                    &format!("{label} load item {i}: durable memory remains searchable"),
                )
                .unwrap();
            }
            expected += count;
            let notes = list_notes(home, &cwd);
            assert_eq!(notes.len(), expected, "{label} load phase");
            assert!(notes.iter().all(|n| n.session_id == "load-test"));
        }
        let raw = std::fs::read_to_string(memory_path(home, &cwd)).unwrap();
        assert!(raw.len() > 500_000, "high-load store unexpectedly small");
    }

    #[test]
    fn old_notes_are_retained_until_explicitly_forgotten() {
        let dir = tempfile::tempdir().unwrap();
        let home = dir.path();
        let cwd = home.join("retention-workspace");
        let path = memory_path(home, &cwd);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        let raw = "## 2020-01-01T00:00:00+00:00 [fact] tag=old session=historical\nlong-lived knowledge\n";
        std::fs::write(&path, raw).unwrap();
        let notes = list_notes(home, &cwd);
        assert_eq!(notes.len(), 1);
        assert_eq!(notes[0].text, "long-lived knowledge");
        forget_note(&path, &notes[0].id).unwrap();
        assert!(list_notes(home, &cwd).is_empty());
    }

    #[test]
    fn cleanup_removes_only_abandoned_artifacts() {
        let dir = tempfile::tempdir().unwrap();
        let home = dir.path();
        let cwd = home.join("cleanup-workspace");
        let path = memory_path(home, &cwd);
        append_note(home, &cwd, "fact", "keep", "s", "keep this note").unwrap();
        let lock = path.with_extension("lock");
        let tmp = path.with_file_name("MEMORY.tmp.crashed");
        std::fs::write(&lock, "pid=dead\n").unwrap();
        std::fs::write(&tmp, "partial\n").unwrap();

        let report = cleanup_artifacts(home, Duration::ZERO);
        assert_eq!(report.removed_locks, 1);
        assert_eq!(report.removed_temps, 1);
        assert!(path.is_file());
        assert_eq!(list_notes(home, &cwd).len(), 1);
        assert!(!lock.exists());
        assert!(!tmp.exists());
    }
}
