//! Cross-session recall (docs/design/23-memory.md): deterministic scan+score
//! over the per-cwd JSONL ledgers. No index to maintain, no new deps —
//! relevance comes from saturating term frequency plus a whole-phrase
//! bonus, with a small recency nudge so fresher context wins ties.

use std::collections::{HashSet, VecDeque};
use std::io::BufRead;
use std::path::Path;

use chrono::{DateTime, Utc};
use vak_llm::Role;

use crate::SessionPath;
use crate::types::{Entry, EntryPayload};

pub const DEFAULT_LIMIT: usize = 8;
/// Bounded work: at most the trailing N message lines of a ledger are
/// scanned. Long sessions still recall their recent past.
const MAX_SCAN_LINES: usize = 4000;
const SNIPPET_CHARS: usize = 240;
const SNIPPET_CONTEXT: usize = 60;
const PHRASE_BONUS: f32 = 3.0;
const RECENCY_NUDGE: f32 = 0.01;
/// Curated memory outranks equally-relevant transcript lines
/// (docs/design/26-learning.md).
pub const MEMORY_BONUS: f32 = 2.5;

#[derive(Debug, Clone, serde::Serialize)]
pub struct SessionHit {
    pub session_id: String,
    pub entry_id: String,
    pub ts: DateTime<Utc>,
    pub role: String,
    pub score: f32,
    pub snippet: String,
}

/// A curated document fed into recall alongside raw transcripts — today,
/// parsed MEMORY.md blocks (docs/design/26-learning.md).
#[derive(Debug, Clone)]
pub struct ExternalDoc {
    /// Stable identifier surfaced in `session_id` of the hit (e.g. the tag).
    pub id: String,
    pub text: String,
}

#[derive(Debug, thiserror::Error)]
pub enum SearchError {
    #[error("io error: {0}")]
    Io(#[from] std::io::Error),
}
/// Search every ledger under `home` for the given workspace `cwd`.
/// `exclude_session` (usually the current session) never matches — its
/// content is already in the caller's context.
pub fn search(
    sessions_home: &Path,
    cwd: &Path,
    query: &str,
    limit: usize,
    exclude_session: Option<&str>,
) -> Result<Vec<SessionHit>, SearchError> {
    search_extended(sessions_home, cwd, query, limit, exclude_session, &[])
}

/// Same scan with curated documents (memory blocks) folded into ranking.
/// Extras carry a bonus so hand-curated knowledge outranks raw history.
pub fn search_extended(
    sessions_home: &Path,
    cwd: &Path,
    query: &str,
    limit: usize,
    exclude_session: Option<&str>,
    extras: &[ExternalDoc],
) -> Result<Vec<SessionHit>, SearchError> {
    let terms = tokenize(query);
    let phrase = normalize(query);
    if terms.is_empty() || phrase.is_empty() {
        return Ok(Vec::new());
    }
    let limit = limit.clamp(1, 50);

    let mut hits: Vec<SessionHit> = Vec::new();
    for doc in extras {
        let base = score_text(&doc.text, &terms, &phrase);
        if base <= 0.0 {
            continue;
        }
        hits.push(SessionHit {
            session_id: doc.id.clone(),
            entry_id: String::new(),
            ts: Utc::now(),
            role: "memory".into(),
            score: base + MEMORY_BONUS,
            snippet: snippet_for(&doc.text, &terms),
        });
    }

    let dir = SessionPath::sessions_dir(sessions_home, cwd);
    let Ok(read) = std::fs::read_dir(&dir) else {
        finalize(&mut hits, limit);
        return Ok(hits);
    };
    for file_entry in read.flatten() {
        let path = file_entry.path();
        if path.extension().and_then(|e| e.to_str()) != Some("jsonl") {
            continue;
        }
        let Some(session_id) = path.file_stem().and_then(|s| s.to_str()).map(String::from) else {
            continue;
        };
        if exclude_session == Some(session_id.as_str()) {
            continue;
        }
        collect_hits(&path, &session_id, &terms, &phrase, &mut hits)?;
    }

    finalize(&mut hits, limit);
    Ok(hits)
}

fn finalize(hits: &mut Vec<SessionHit>, limit: usize) {
    // Score = relevance + tiny recency preference; newest wins exact ties.
    let now = Utc::now();
    for h in hits.iter_mut() {
        let hours = (now - h.ts).num_hours().max(0) as f32;
        h.score += RECENCY_NUDGE / (1.0 + hours);
    }
    hits.sort_by(|a, b| b.score.total_cmp(&a.score).then(b.ts.cmp(&a.ts)));
    hits.truncate(limit);
}

fn collect_hits(
    path: &Path,
    session_id: &str,
    terms: &[String],
    phrase: &str,
    hits: &mut Vec<SessionHit>,
) -> Result<(), SearchError> {
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
        let base = score_text(&text, terms, phrase);
        if base <= 0.0 {
            continue;
        }
        hits.push(SessionHit {
            session_id: session_id.to_string(),
            entry_id: entry.id,
            ts: entry.ts,
            role: role.to_string(),
            score: base,
            snippet: snippet_for(&text, terms),
        });
    }
    Ok(())
}

/// Lowercase alphanumeric runs of length >= 2. Cheap and language-tolerant:
/// CJK text yields long runs that behave like phrases, which is fine for
/// substring matching below.
fn tokenize(text: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut seen = HashSet::new();
    let normalized = normalize(text);
    let mut current = String::new();
    for ch in normalized.chars() {
        if ch.is_alphanumeric() {
            current.push(ch);
        } else if !current.is_empty() {
            push_token(&mut out, &mut seen, &current);
            current.clear();
        }
    }
    if !current.is_empty() {
        push_token(&mut out, &mut seen, &current);
    }
    out
}

fn push_token(out: &mut Vec<String>, seen: &mut HashSet<String>, token: &str) {
    if token.chars().count() >= 2 && seen.insert(token.to_string()) {
        out.push(token.to_string());
    }
}

fn normalize(text: &str) -> String {
    text.to_lowercase()
}

fn score_text(text: &str, terms: &[String], phrase: &str) -> f32 {
    let hay = normalize(text);
    let mut score = 0.0f32;
    let mut matched_any = false;
    for term in terms {
        let mut count = 0usize;
        let mut from = 0usize;
        while let Some(pos) = hay[from..].find(term.as_str()) {
            count += 1;
            let abs = from + pos + term.len();
            if abs >= hay.len() {
                break;
            }
            from = abs;
            if count >= 16 {
                break;
            }
        }
        if count > 0 {
            matched_any = true;
            score += 1.0 + (count as f32).ln();
        }
    }
    if !matched_any {
        return 0.0;
    }
    if hay.contains(phrase) {
        score += PHRASE_BONUS;
    }
    score
}

fn snippet_for(text: &str, terms: &[String]) -> String {
    let hay = normalize(text);
    let mut first_byte: Option<usize> = None;
    for term in terms {
        if let Some(pos) = hay.find(term.as_str()) {
            first_byte = Some(match first_byte {
                Some(existing) => existing.min(pos),
                None => pos,
            });
        }
    }
    let char_start = match first_byte {
        Some(byte) => text[..byte].chars().count(),
        None => 0,
    };
    let start = char_start.saturating_sub(SNIPPET_CONTEXT);
    let total_chars = text.chars().count();
    let end = (start + SNIPPET_CHARS).min(total_chars);

    let prefix = if start > 0 { "…" } else { "" };
    let suffix = if end < total_chars { "…" } else { "" };
    format!(
        "{prefix}{}{suffix}",
        text.chars()
            .skip(start)
            .take(end - start)
            .collect::<String>()
            .trim()
    )
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]
    use super::*;
    use crate::SessionLog;
    use crate::types::{MessageRecord, SessionHeader};
    use vak_llm::types::{ContentBlock, Message};

    fn header_for(id: &str, cwd: &Path) -> SessionHeader {
        SessionHeader {
            session_id: id.to_string(),
            created_at: Utc::now(),
            cwd: cwd.to_path_buf(),
            parent_session_id: None,
            contract: crate::types::FrozenContract {
                app_version: "test".into(),
                provider: "scripted".into(),
                model: "m".into(),
                route_ladder: Vec::new(),
                system_prompt: String::new(),
                tools: vec![],
                permission_mode: "workspace-write".into(),
                skills: vec![],
            },
        }
    }

    fn user_msg(t: &str) -> MessageRecord {
        MessageRecord {
            message: Message {
                role: Role::User,
                content: vec![ContentBlock::text(t)],
            },
            meta: None,
        }
    }

    fn assistant_msg(t: &str) -> MessageRecord {
        MessageRecord {
            message: Message {
                role: Role::Assistant,
                content: vec![ContentBlock::text(t)],
            },
            meta: None,
        }
    }

    fn seed(home: &Path, cwd: &Path, id: &str, msgs: &[MessageRecord]) {
        let path = SessionPath::new_session_file(home, cwd, id);
        let mut log = SessionLog::create(path, header_for(id, cwd)).unwrap();
        for m in msgs {
            log.append_message(m.clone()).unwrap();
        }
    }

    #[test]
    fn finds_and_ranks_relevant_sessions() {
        let dir = tempfile::tempdir().unwrap();
        let home = dir.path().join("home");
        let cwd = dir.path().to_path_buf();

        seed(
            &home,
            &cwd,
            "11111111-deploy-talks",
            &[
                user_msg("how does the deploy script handle rollbacks?"),
                assistant_msg("the deploy script pauses before rollback windows"),
            ],
        );
        seed(
            &home,
            &cwd,
            "22222222-unrelated",
            &[user_msg("favorite pizza toppings debate")],
        );
        seed(
            &home,
            &cwd,
            "33333333-phrase-match",
            &[user_msg("run the deploy script now")],
        );

        let hits = search(&home, &cwd, "deploy script", DEFAULT_LIMIT, None).unwrap();
        // Message-level hits: both messages of the first ledger match.
        assert_eq!(hits.len(), 3);
        assert_eq!(
            hits[0].session_id, "33333333-phrase-match",
            "verbatim phrase outranks scattered terms"
        );
        let sessions: HashSet<&str> = hits.iter().map(|h| h.session_id.as_str()).collect();
        assert_eq!(sessions.len(), 2, "pizza debate never matches");
        assert!(hits[0].snippet.contains("deploy script"));
        assert!(hits.iter().all(|h| h.score > 0.0));
        assert!(
            hits.iter().any(|h| h.role == "user") && hits.iter().any(|h| h.role == "assistant")
        );
    }

    #[test]
    fn excludes_current_session_and_empty_queries() {
        let dir = tempfile::tempdir().unwrap();
        let home = dir.path().join("home");
        let cwd = dir.path().to_path_buf();
        seed(
            &home,
            &cwd,
            "aaaaaaaa-current",
            &[user_msg("kubernetes ingress quirks")],
        );
        seed(
            &home,
            &cwd,
            "bbbbbbbb-other",
            &[user_msg("kubernetes ingress quirks from last week")],
        );

        let hits = search(
            &home,
            &cwd,
            "kubernetes ingress",
            DEFAULT_LIMIT,
            Some("aaaaaaaa-current"),
        )
        .unwrap();
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].session_id, "bbbbbbbb-other");

        assert!(
            search(&home, &cwd, "   ", DEFAULT_LIMIT, None)
                .unwrap()
                .is_empty()
        );
        assert!(
            search(&home, &cwd, "zzzqqq nonexistent", DEFAULT_LIMIT, None)
                .unwrap()
                .is_empty()
        );
    }

    #[test]
    fn memory_extras_outrank_equal_transcript_hits() {
        let dir = tempfile::tempdir().unwrap();
        let home = dir.path().join("home");
        let cwd = dir.path().to_path_buf();
        seed(
            &home,
            &cwd,
            "dddddddd-transcript",
            &[user_msg(
                "rollback windows are configured in the deploy pipeline",
            )],
        );
        let extras = vec![ExternalDoc {
            id: "deploy".into(),
            text: "decision: rollback windows pause the deploy pipeline".into(),
        }];
        let hits = search_extended(&home, &cwd, "deploy rollback", 5, None, &extras).unwrap();
        assert!(hits.len() >= 2);
        assert_eq!(hits[0].role, "memory");
        assert_eq!(hits[0].session_id, "deploy");
        assert!(hits.iter().skip(1).all(|h| h.role != "memory"));
    }

    #[test]
    fn snippets_are_bounded_and_centered() {
        let dir = tempfile::tempdir().unwrap();
        let home = dir.path().join("home");
        let cwd = dir.path().to_path_buf();
        let filler = "lorem ipsum ".repeat(500);
        seed(
            &home,
            &cwd,
            "cccccccc-long",
            &[assistant_msg(&format!(
                "{filler}NEEDLE-HAYSTACK trailing words"
            ))],
        );
        let hits = search(&home, &cwd, "needle", 5, None).unwrap();
        assert_eq!(hits.len(), 1);
        let snip = &hits[0].snippet;
        assert!(snip.starts_with('…'), "long text gets a leading ellipsis");
        assert!(snip.contains("NEEDLE"));
        assert!(snip.chars().count() <= SNIPPET_CHARS + 2);
    }
}
