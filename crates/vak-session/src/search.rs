//! Cross-session recall (docs/design/23-memory.md): deterministic scan+score
//! over the per-cwd JSONL ledgers. Relevance comes from saturating term
//! frequency plus a whole-phrase bonus, with a small recency nudge so
//! fresher context wins ties. An mtime-keyed per-ledger cache
//! (crate::index) makes warm queries skip the rescan (M1); `search_all`
//! extends the same ranking across every project hash dir (personal-os P1).

use std::collections::HashSet;
use std::path::{Path, PathBuf};

use chrono::{DateTime, Utc};

use crate::SessionPath;
use crate::index;

pub const DEFAULT_LIMIT: usize = 8;
const SNIPPET_CHARS: usize = 240;
const SNIPPET_CONTEXT: usize = 60;
const PHRASE_BONUS: f32 = 3.0;
const RECENCY_NUDGE: f32 = 0.01;
/// Curated memory outranks equally-relevant transcript lines
/// (docs/design/26-learning.md).
pub const MEMORY_BONUS: f32 = 2.5;

#[derive(Debug, Clone, PartialEq, serde::Serialize)]
pub struct SessionHit {
    pub session_id: String,
    pub entry_id: String,
    pub ts: DateTime<Utc>,
    pub role: String,
    pub score: f32,
    pub snippet: String,
}

/// A hit from cross-project recall: the session hit plus the project hash
/// directory (`<home>/sessions/<hash>/`) it was found in (personal-os P1).
#[derive(Debug, Clone, PartialEq, serde::Serialize)]
pub struct ProjectHit {
    pub project_hash: String,
    #[serde(flatten)]
    pub hit: SessionHit,
}

#[derive(Debug, Clone)]
struct RankedHit {
    hit: SessionHit,
    project_hash: Option<String>,
}

/// A curated document fed into recall alongside raw transcripts — today,
/// parsed MEMORY.md blocks (docs/design/26-learning.md).
#[derive(Debug, Clone)]
pub struct ExternalDoc {
    /// Stable identifier surfaced in `session_id` of the hit (e.g. the tag).
    pub id: String,
    pub text: String,
    /// Original document timestamp. `None` preserves the legacy caller
    /// contract; durable memory callers always provide it.
    pub ts: Option<DateTime<Utc>>,
    /// Result role (`memory` or `profile`) shown to surfaces.
    pub role: Option<String>,
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

    let mut ranked: Vec<RankedHit> = Vec::new();
    for doc in extras {
        let base = score_text(&doc.text, &terms, &phrase);
        if base <= 0.0 {
            continue;
        }
        ranked.push(RankedHit {
            hit: SessionHit {
                session_id: doc.id.clone(),
                entry_id: String::new(),
                ts: doc.ts.unwrap_or_else(Utc::now),
                role: doc.role.clone().unwrap_or_else(|| "memory".into()),
                score: base + MEMORY_BONUS,
                snippet: snippet_for(&doc.text, &terms),
            },
            project_hash: None,
        });
    }

    let dir = SessionPath::sessions_dir(sessions_home, cwd);
    if let Ok(read) = std::fs::read_dir(&dir) {
        for file_entry in read.flatten() {
            let path = file_entry.path();
            if path.extension().and_then(|e| e.to_str()) != Some("jsonl") {
                continue;
            }
            let Some(session_id) = path.file_stem().and_then(|s| s.to_str()).map(String::from)
            else {
                continue;
            };
            if exclude_session == Some(session_id.as_str()) {
                continue;
            }
            collect_ranked(&path, &session_id, &terms, &phrase, None, &mut ranked)?;
        }
    }

    finalize(&mut ranked, limit);
    Ok(ranked.into_iter().map(|r| r.hit).collect())
}

/// Search every ledger of every project hash dir under `<home>/sessions/`
/// with the identical scoring, exclusions, and snippet rules as `search`.
/// Hits are annotated with the hash dir they came from; the current-session
/// exclusion applies across all dirs (personal-os P1).
pub fn search_all(
    home: &Path,
    query: &str,
    limit: usize,
    exclude_session: Option<&str>,
) -> Result<Vec<ProjectHit>, SearchError> {
    search_all_extended(home, query, limit, exclude_session, &[])
}

/// Cross-project search with curated documents included in the same ranking.
pub fn search_all_extended(
    home: &Path,
    query: &str,
    limit: usize,
    exclude_session: Option<&str>,
    extras: &[ExternalDoc],
) -> Result<Vec<ProjectHit>, SearchError> {
    let terms = tokenize(query);
    let phrase = normalize(query);
    if terms.is_empty() || phrase.is_empty() {
        return Ok(Vec::new());
    }
    let limit = limit.clamp(1, 50);

    let mut ranked: Vec<RankedHit> = extras
        .iter()
        .filter_map(|doc| {
            let base = score_text(&doc.text, &terms, &phrase);
            (base > 0.0).then(|| RankedHit {
                hit: SessionHit {
                    session_id: doc.id.clone(),
                    entry_id: String::new(),
                    ts: doc.ts.unwrap_or_else(Utc::now),
                    role: doc.role.clone().unwrap_or_else(|| "memory".into()),
                    score: base + MEMORY_BONUS,
                    snippet: snippet_for(&doc.text, &terms),
                },
                project_hash: None,
            })
        })
        .collect();
    let root = home.join("sessions");
    let mut projects: Vec<PathBuf> = match std::fs::read_dir(&root) {
        Ok(read) => read
            .flatten()
            .map(|e| e.path())
            .filter(|p| p.is_dir())
            .collect(),
        Err(_) => Vec::new(),
    };
    projects.sort();
    for dir in projects {
        let Ok(read) = std::fs::read_dir(&dir) else {
            continue;
        };
        let Some(project_hash) = dir.file_name().and_then(|n| n.to_str()).map(String::from) else {
            continue;
        };
        let mut files: Vec<PathBuf> = read.flatten().map(|e| e.path()).collect();
        files.sort();
        for path in files {
            if path.extension().and_then(|e| e.to_str()) != Some("jsonl") {
                continue;
            }
            let Some(session_id) = path.file_stem().and_then(|s| s.to_str()).map(String::from)
            else {
                continue;
            };
            if exclude_session == Some(session_id.as_str()) {
                continue;
            }
            collect_ranked(
                &path,
                &session_id,
                &terms,
                &phrase,
                Some(project_hash.as_str()),
                &mut ranked,
            )?;
        }
    }

    finalize(&mut ranked, limit);
    Ok(ranked
        .into_iter()
        .map(|r| ProjectHit {
            project_hash: r.project_hash.unwrap_or_default(),
            hit: r.hit,
        })
        .collect())
}

fn finalize(ranked: &mut Vec<RankedHit>, limit: usize) {
    // Score = relevance + tiny recency preference; newest wins exact ties.
    let now = Utc::now();
    for r in ranked.iter_mut() {
        let hours = (now - r.hit.ts).num_hours().max(0) as f32;
        r.hit.score += RECENCY_NUDGE / (1.0 + hours);
    }
    // Total order so equal-score results are deterministic regardless of
    // directory iteration order.
    ranked.sort_by(|a, b| {
        b.hit
            .score
            .total_cmp(&a.hit.score)
            .then(b.hit.ts.cmp(&a.hit.ts))
            .then(a.hit.session_id.cmp(&b.hit.session_id))
            .then(a.hit.entry_id.cmp(&b.hit.entry_id))
            .then(a.project_hash.cmp(&b.project_hash))
    });
    ranked.truncate(limit);
}

fn collect_ranked(
    path: &Path,
    session_id: &str,
    terms: &[String],
    phrase: &str,
    project_hash: Option<&str>,
    ranked: &mut Vec<RankedHit>,
) -> Result<(), SearchError> {
    let messages = index::ledger(path)?;
    for m in messages.iter() {
        let base = score_normalized(&m.normalized, terms, phrase);
        if base <= 0.0 {
            continue;
        }
        ranked.push(RankedHit {
            hit: SessionHit {
                session_id: session_id.to_string(),
                entry_id: m.entry_id.clone(),
                ts: m.ts,
                role: m.role.to_string(),
                score: base,
                snippet: snippet_for(&m.text, terms),
            },
            project_hash: project_hash.map(String::from),
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

pub(crate) fn normalize(text: &str) -> String {
    text.to_lowercase()
}

fn score_text(text: &str, terms: &[String], phrase: &str) -> f32 {
    score_normalized(&normalize(text), terms, phrase)
}

fn score_normalized(hay: &str, terms: &[String], phrase: &str) -> f32 {
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
    use crate::types::{Entry, EntryPayload, MessageRecord, SessionHeader};
    use std::io::Write as _;
    use vak_llm::Role;
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
                route_objective: String::new(),
                route_annotations: Vec::new(),
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
            ts: None,
            role: None,
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

    #[test]
    fn index_detects_appended_lines_without_restart() {
        let dir = tempfile::tempdir().unwrap();
        let home = dir.path().join("home");
        let cwd = dir.path().to_path_buf();
        seed(
            &home,
            &cwd,
            "eeeeeeee-append",
            &[user_msg("alpha bravo charlie baseline")],
        );

        assert!(search(&home, &cwd, "foxtrot", 5, None).unwrap().is_empty());

        let path = SessionPath::new_session_file(&home, &cwd, "eeeeeeee-append");
        let mut log = SessionLog::open(path).unwrap();
        log.append_message(user_msg("delta echo foxtrot followup"))
            .unwrap();
        drop(log);

        let hits = search(&home, &cwd, "foxtrot", 5, None).unwrap();
        assert_eq!(hits.len(), 1, "append must invalidate the cached ledger");
        assert_eq!(hits[0].session_id, "eeeeeeee-append");
        assert_eq!(hits[0].role, "user");

        // The warmed cache answers again without re-reading the file.
        let again = search(&home, &cwd, "foxtrot", 5, None).unwrap();
        assert_eq!(hits, again);
    }

    fn write_ledger(path: &Path, msgs: &[MessageRecord]) {
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        let file = std::fs::File::create(path).unwrap();
        let mut w = std::io::BufWriter::new(file);
        for m in msgs {
            let entry = Entry::new(None, EntryPayload::Message(m.clone()));
            serde_json::to_writer(&mut w, &entry).unwrap();
            w.write_all(b"\n").unwrap();
        }
        w.flush().unwrap();
    }

    #[test]
    fn warm_index_beats_cold_scan_on_large_store() {
        let dir = tempfile::tempdir().unwrap();
        let home = dir.path().join("home");
        let cwd = dir.path().to_path_buf();

        const LEDGERS: usize = 10;
        const LINES: usize = 1000;
        for k in 0..LEDGERS {
            let mut msgs = Vec::with_capacity(LINES);
            for i in 0..LINES {
                msgs.push(user_msg(&format!(
                    "note {i} about refactor planning and review cadence {k}"
                )));
            }
            if k == 0 {
                msgs[7] = user_msg("quantum tuning notes for ledger zero");
                msgs[9] = user_msg("xylophone quantum alignment strategy");
            } else if k < 3 {
                msgs[7] = user_msg(&format!("quantum tuning notes for ledger {k}"));
                msgs[9] = user_msg(&format!("quantum alignment strategy for ledger {k}"));
            }
            write_ledger(
                &SessionPath::new_session_file(&home, &cwd, &format!("{k:08}-bulk")),
                &msgs,
            );
        }

        let t0 = std::time::Instant::now();
        let cold_hits = search(&home, &cwd, "xylophone quantum", 20, None).unwrap();
        let cold = t0.elapsed();

        let mut warm_min = std::time::Duration::MAX;
        let mut last_hits = Vec::new();
        for _ in 0..3 {
            let t = std::time::Instant::now();
            last_hits = search(&home, &cwd, "xylophone quantum", 20, None).unwrap();
            warm_min = warm_min.min(t.elapsed());
        }

        assert_eq!(
            cold_hits.len(),
            6,
            "two planted messages in each of three ledgers"
        );
        assert_eq!(
            cold_hits[0].session_id, "00000000-bulk",
            "the only full-phrase match outranks scattered term matches"
        );
        assert!(cold_hits[0].snippet.contains("xylophone"));
        assert_eq!(
            cold_hits, last_hits,
            "same input must yield identical output"
        );
        assert!(
            warm_min < cold,
            "warm query ({warm_min:?}) should beat cold scan ({cold:?})"
        );
        assert!(
            warm_min < std::time::Duration::from_millis(500),
            "warm query must stay fast even on slow CI ({warm_min:?})"
        );
    }

    #[test]
    fn search_all_spans_project_dirs_and_excludes_everywhere() {
        let dir = tempfile::tempdir().unwrap();
        let home = dir.path().join("home");
        let cwd_a = dir.path().join("proj-a");
        let cwd_b = dir.path().join("proj-b");
        let cwd_c = dir.path().join("proj-c");

        seed(
            &home,
            &cwd_a,
            "aa-keeper",
            &[user_msg("needle in project a")],
        );
        seed(&home, &cwd_a, "zz-excluded", &[user_msg("needle hidden a")]);
        seed(
            &home,
            &cwd_b,
            "bb-keeper",
            &[user_msg("needle in project b")],
        );
        seed(&home, &cwd_b, "zz-excluded", &[user_msg("needle hidden b")]);
        seed(
            &home,
            &cwd_c,
            "cc-keeper",
            &[user_msg("needle in project c")],
        );

        let hash = |cwd: &Path| {
            SessionPath::sessions_dir(&home, cwd)
                .file_name()
                .unwrap()
                .to_string_lossy()
                .to_string()
        };
        let (ha, hb, hc) = (hash(&cwd_a), hash(&cwd_b), hash(&cwd_c));

        let first = search_all(&home, "needle", 20, Some("zz-excluded")).unwrap();
        let second = search_all(&home, "needle", 20, Some("zz-excluded")).unwrap();

        assert_eq!(first.len(), 3, "one keeper hit per project dir");
        assert_eq!(
            first, second,
            "cross-project ordering must be deterministic"
        );
        assert!(
            first.iter().all(|h| h.hit.session_id != "zz-excluded"),
            "current-session exclusion applies in every hash dir"
        );
        let sessions: HashSet<&str> = first.iter().map(|h| h.hit.session_id.as_str()).collect();
        assert_eq!(
            sessions,
            HashSet::from(["aa-keeper", "bb-keeper", "cc-keeper"])
        );
        let hashes: HashSet<&str> = first.iter().map(|h| h.project_hash.as_str()).collect();
        assert_eq!(
            hashes,
            HashSet::from([ha.as_str(), hb.as_str(), hc.as_str()])
        );
        for h in &first {
            let source = match h.hit.session_id.as_str() {
                "aa-keeper" => &ha,
                "bb-keeper" => &hb,
                _ => &hc,
            };
            assert_eq!(h.project_hash, *source);
        }
    }
}
