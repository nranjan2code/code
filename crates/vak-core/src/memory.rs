//! Durable memory notes (docs/design/26-learning.md): one plain-markdown
//! file per workspace under `<home>/memory/<hash>/MEMORY.md`, plus a global
//! user-profile tier at `<home>/memory/user/USER.md`
//! (docs/design/29-personal-os.md P1). The model appends through the
//! `remember` tool; humans edit or prune the file directly — the parser
//! tolerates hand-edits rather than fighting them. Forget/amend rewrite the
//! markdown file in place; sessions stay append-only.

use std::path::{Path, PathBuf};

use chrono::{DateTime, Utc};

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
    /// The conversation and turn this note was derived from, when the writer
    /// knew the turn. The conversation is `session_id`.
    pub derived_from: Option<vak_session::trace::DerivedFrom>,
}

fn memory_path(home: &Path, cwd: &Path) -> PathBuf {
    vak_config::scope::AgentScope::new(home).memory_notes(cwd)
}

/// [`memory_path`] for a write: noting something about `cwd` treats it as a
/// workspace, so it is bound to its space first; if that fails the store
/// refuses the write under its `unbound-` key and says why.
fn memory_write_path(home: &Path, cwd: &Path) -> PathBuf {
    let _ = vak_config::spaces::bind(cwd);
    memory_path(home, cwd)
}

/// Global profile-tier store, keyed to the user rather than a workspace.
pub fn profile_path(home: &Path) -> PathBuf {
    vak_config::scope::AgentScope::new(home).user_memory()
}

pub fn append_note(
    home: &Path,
    cwd: &Path,
    kind: &str,
    tag: &str,
    session_id: &str,
    text: &str,
) -> Result<NoteBlock, String> {
    append_block(
        &memory_write_path(home, cwd),
        kind,
        tag,
        session_id,
        None,
        text,
    )
}

/// `append_note` that also records the turn the note was derived from.
pub fn append_note_from_turn(
    home: &Path,
    cwd: &Path,
    kind: &str,
    tag: &str,
    session_id: &str,
    turn: Option<&str>,
    text: &str,
) -> Result<NoteBlock, String> {
    append_block(
        &memory_write_path(home, cwd),
        kind,
        tag,
        session_id,
        turn,
        text,
    )
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
    append_block(&profile_path(home), kind, tag, session, None, text)
}

fn append_block(
    path: &Path,
    kind: &str,
    tag: &str,
    session_id: &str,
    turn: Option<&str>,
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
    if let Some(t) = turn {
        validate_field("turn", t, 160)?;
        if t.is_empty() || t.contains(char::is_whitespace) {
            return Err("turn contains invalid characters or is too long".into());
        }
    }
    let text = text.trim();
    if text.is_empty() {
        return Err("note must not be empty".into());
    }
    if text.len() > 128 * 1024 {
        return Err("note exceeds the 128 KiB limit".into());
    }
    let ts = Utc::now();
    let turn_part = turn.map(|t| format!(" turn={t}")).unwrap_or_default();
    let header = format!(
        "## {} [{kind}] tag={tag} session={session_id}{turn_part}",
        ts.to_rfc3339()
    );
    vak_session::documents::create(&path.join(note_id(&header)), &format!("{header}\n{text}\n"))?;
    Ok(NoteBlock {
        id: note_id(&header),
        ts,
        kind: kind.to_string(),
        tag: tag.to_string(),
        session_id: session_id.to_string(),
        text: text.to_string(),
        derived_from: turn.map(|t| vak_session::trace::DerivedFrom {
            conversation: session_id.to_string(),
            turn: Some(t.to_string()),
        }),
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

/// Forget one note in the Agent's workspace tier without exposing the
/// backing path to callers.
pub fn forget_workspace_note(home: &Path, cwd: &Path, note_id: &str) -> Result<usize, String> {
    forget_note(&memory_path(home, cwd), note_id)
}

/// Forget one note in the Agent's profile tier without exposing the path.
pub fn forget_profile_note(home: &Path, note_id: &str) -> Result<usize, String> {
    forget_note(&profile_path(home), note_id)
}

/// The notes of one tier, oldest first. Each note is its own Document
/// under the tier's name, so appending one never rewrites the others.
fn blocks_at(path: &Path) -> Vec<NoteBlock> {
    let mut notes: Vec<NoteBlock> = vak_session::documents::under(path)
        .iter()
        .filter_map(|note| vak_session::documents::read(note).ok().flatten())
        .flat_map(|raw| parse_blocks(&raw))
        .collect();
    notes.sort_by(|a, b| a.ts.cmp(&b.ts).then_with(|| a.id.cmp(&b.id)));
    notes
}

pub fn parse_blocks(raw: &str) -> Vec<NoteBlock> {
    struct Open {
        header_line: String,
        ts: DateTime<Utc>,
        kind: String,
        tag: String,
        session_id: String,
        turn: Option<String>,
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
                    derived_from: o.turn.map(|t| vak_session::trace::DerivedFrom {
                        conversation: o.session_id.clone(),
                        turn: Some(t),
                    }),
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
                Some((ts, kind, tag, sid, turn)) => {
                    flush(&mut open, &mut body, &mut out);
                    open = Some(Open {
                        header_line: line.to_string(),
                        ts,
                        kind,
                        tag,
                        session_id: sid,
                        turn,
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

/// Forgets one note and returns the size of what was removed. Its
/// versions are released for collection; the other notes are untouched.
pub fn forget_note(path: &Path, note_id: &str) -> Result<usize, String> {
    let note = path.join(note_id);
    let size = vak_session::documents::read(&note)?
        .ok_or_else(|| format!("no note '{note_id}' in {}", path.display()))?
        .len();
    vak_session::documents::forget(&note)?;
    Ok(size)
}

/// Replace one note's body as a new version, keeping its provenance header
/// line verbatim.
pub fn amend_note(path: &Path, note_id: &str, new_text: &str) -> Result<(), String> {
    let text = new_text.trim();
    if text.is_empty() {
        return Err("note must not be empty".into());
    }
    if text.len() > 128 * 1024 {
        return Err("note exceeds the 128 KiB limit".into());
    }
    vak_session::documents::update(&path.join(note_id), |old| {
        let old = old.ok_or_else(|| format!("no note '{note_id}' in {}", path.display()))?;
        let header_end = old.find('\n').unwrap_or(old.len());
        Ok(Some((format!("{}\n{text}\n", &old[..header_end]), ())))
    })?
    .ok_or_else(|| format!("no note '{note_id}' in {}", path.display()))
}

/// `2026-08-23T12:00:00+00:00 [decision] tag=x session=abc` → parts.
type ParsedHeader = (DateTime<Utc>, String, String, String, Option<String>);

fn parse_header(rest: &str) -> Option<ParsedHeader> {
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
    let mut turn = None;
    let mut leftover = String::new();
    for token in remainder.split_whitespace() {
        if let Some(v) = token.strip_prefix("tag=") {
            tag = v.to_string();
        } else if let Some(v) = token.strip_prefix("session=") {
            session = v.to_string();
        } else if let Some(v) = token.strip_prefix("turn=") {
            turn = Some(v.to_string());
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
    Some((ok_ts.with_timezone(&Utc), kind, tag, session, turn))
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]
    use super::*;

    #[test]
    fn append_then_parse_roundtrips() {
        vak_config::paths::isolate_home_for_tests();
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
        vak_config::paths::isolate_home_for_tests();
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
        vak_config::paths::isolate_home_for_tests();
        let dir = tempfile::tempdir().unwrap();
        let err = append_note(dir.path(), dir.path(), "fact", "", "s", "   ");
        assert!(err.is_err());
    }

    #[test]
    fn header_metadata_rejects_control_characters() {
        vak_config::paths::isolate_home_for_tests();
        let dir = tempfile::tempdir().unwrap();
        assert!(append_note(dir.path(), dir.path(), "fact", "bad\ntag", "s", "x").is_err());
        assert!(append_note(dir.path(), dir.path(), "fact", "ok", "s\n2", "x").is_err());
    }

    #[test]
    fn malformed_heading_remains_in_preceding_note() {
        vak_config::paths::isolate_home_for_tests();
        let raw = "## 2026-08-23T10:00:00+00:00 [fact] session=s\nfirst\n## not metadata\nmore\n";
        let notes = parse_blocks(raw);
        assert_eq!(notes.len(), 1);
        assert!(notes[0].text.contains("## not metadata"));
        assert!(notes[0].text.contains("more"));
    }

    #[test]
    fn concurrent_appends_keep_every_block_intact() {
        vak_config::paths::isolate_home_for_tests();
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
        vak_config::paths::isolate_home_for_tests();
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
        let path = memory_path(home, &cwd);
        assert_eq!(
            vak_session::documents::read(&path.join(&b.id))
                .unwrap()
                .as_deref(),
            Some(block_b.as_str())
        );

        let removed = forget_note(&path, &a.id).unwrap();
        assert_eq!(removed, block_a.len());

        let notes = list_notes(home, &cwd);
        assert_eq!(notes.len(), 1);
        assert_eq!(notes[0].id, b.id);

        assert!(forget_note(&path, &a.id).is_err());
        assert_eq!(list_notes(home, &cwd).len(), 1);
    }

    #[test]
    fn amend_swaps_body_and_preserves_provenance_header() {
        vak_config::paths::isolate_home_for_tests();
        let dir = tempfile::tempdir().unwrap();
        let home = dir.path();
        let cwd = dir.path().join("ws");

        let n = append_note(home, &cwd, "fact", "", "s3", "original body").unwrap();
        let header = format!("## {} [fact] tag= session=s3", n.ts.to_rfc3339());
        let path = memory_path(home, &cwd);

        amend_note(&path, &n.id, "revised body with more detail").unwrap();

        assert_eq!(
            vak_session::documents::read(&path.join(&n.id)).unwrap(),
            Some(format!("{header}\nrevised body with more detail\n"))
        );
        assert_eq!(vak_session::documents::version_count(&path.join(&n.id)), 2);
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
    fn profile_tier_roundtrips() {
        vak_config::paths::isolate_home_for_tests();
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
        vak_config::paths::isolate_home_for_tests();
        let dir = tempfile::tempdir().unwrap();
        let home = dir.path();
        let cwd = home.join("load-workspace");
        std::fs::create_dir_all(&cwd).unwrap();
        let mut expected = 0;
        for (label, count) in [("low", 10usize), ("medium", 500), ("high", 1_024)] {
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
    }

    #[test]
    fn old_notes_are_retained_until_explicitly_forgotten() {
        vak_config::paths::isolate_home_for_tests();
        let dir = tempfile::tempdir().unwrap();
        let home = dir.path();
        let cwd = home.join("retention-workspace");
        let path = memory_write_path(home, &cwd);
        let raw = "## 2020-01-01T00:00:00+00:00 [fact] tag=old session=historical\nlong-lived knowledge\n";
        vak_session::documents::create(&path.join(&parse_blocks(raw)[0].id), raw).unwrap();
        let notes = list_notes(home, &cwd);
        assert_eq!(notes.len(), 1);
        assert_eq!(notes[0].text, "long-lived knowledge");
        forget_note(&path, &notes[0].id).unwrap();
        assert!(list_notes(home, &cwd).is_empty());
    }
}
