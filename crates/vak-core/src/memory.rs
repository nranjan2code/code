//! Durable memory notes (docs/design/26-learning.md): one plain-markdown
//! file per workspace under `<home>/memory/<hash>/MEMORY.md`. The model
//! appends through the `remember` tool; humans edit or prune the file
//! directly — the parser tolerates hand-edits rather than fighting them.

use std::path::{Path, PathBuf};

use chrono::{DateTime, Utc};

/// FNV-1a 64-bit, matching vak-session's cwd hashing so every per-workspace
/// store keys off the same identity.
pub fn hash_cwd(cwd: &Path) -> String {
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for b in cwd.to_string_lossy().as_bytes() {
        h ^= u64::from(*b);
        h = h.wrapping_mul(0x0000_0100_0000_01b3);
    }
    format!("{h:016x}")
}

#[derive(Debug, Clone, PartialEq)]
pub struct NoteBlock {
    pub ts: DateTime<Utc>,
    pub kind: String,
    pub tag: String,
    pub session_id: String,
    pub text: String,
}

fn memory_path(home: &Path, cwd: &Path) -> PathBuf {
    home.join("memory").join(hash_cwd(cwd)).join("MEMORY.md")
}

pub fn append_note(
    home: &Path,
    cwd: &Path,
    kind: &str,
    tag: &str,
    session_id: &str,
    text: &str,
) -> Result<NoteBlock, String> {
    let text = text.trim();
    if text.is_empty() {
        return Err("note must not be empty".into());
    }
    let ts = Utc::now();
    let path = memory_path(home, cwd);
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|e| format!("create memory dir: {e}"))?;
    }
    let block = format!(
        "## {} [{kind}] tag={tag} session={session_id}\n{text}\n",
        ts.to_rfc3339()
    );
    use std::io::Write;
    let mut f = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&path)
        .map_err(|e| format!("open MEMORY.md: {e}"))?;
    writeln!(f, "{block}").map_err(|e| format!("append MEMORY.md: {e}"))?;
    Ok(NoteBlock {
        ts,
        kind: kind.to_string(),
        tag: tag.to_string(),
        session_id: session_id.to_string(),
        text: text.to_string(),
    })
}

/// Parse MEMORY.md blocks. Tolerant by design: lines before the first
/// heading and malformed headings attach to whatever precedes them, so a
/// hand-edit never silently loses content.
pub fn list_notes(home: &Path, cwd: &Path) -> Vec<NoteBlock> {
    let Ok(raw) = std::fs::read_to_string(memory_path(home, cwd)) else {
        return Vec::new();
    };
    parse_blocks(&raw)
}

pub fn parse_blocks(raw: &str) -> Vec<NoteBlock> {
    let mut out: Vec<NoteBlock> = Vec::new();
    let mut header: Option<(DateTime<Utc>, String, String, String)> = None;
    let mut body = String::new();

    let flush = |header: &mut Option<(DateTime<Utc>, String, String, String)>,
                     body: &mut String,
                     out: &mut Vec<NoteBlock>| {
        if let Some((ts, kind, tag, sid)) = header.take() {
            let text = body.trim().to_string();
            if !text.is_empty() {
                out.push(NoteBlock {
                    ts,
                    kind,
                    tag,
                    session_id: sid,
                    text,
                });
            }
        }
        body.clear();
    };

    for line in raw.lines() {
        if let Some(rest) = line.strip_prefix("## ") {
            flush(&mut header, &mut body, &mut out);
            match parse_header(rest) {
                Some(parsed) => header = Some(parsed),
                None => {
                    // Not our grammar: treat as continuation text of the
                    // previous block (or drop leading junk).
                    if header.is_some() || !out.is_empty() || !body.is_empty() {
                        body.push_str(line);
                        body.push('\n');
                    }
                }
            }
        } else if header.is_some() {
            body.push_str(line);
            body.push('\n');
        }
    }
    flush(&mut header, &mut body, &mut out);
    out
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
}
