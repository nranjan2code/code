//! Strands: the parts of a request that are separate pieces of work.
//!
//! A request is rarely one thing. "Explain the parser, then refactor it, and
//! also check whether the nightly job ran" is three pieces of work with three
//! different readings, and the runtime has to know that: the tool surface must
//! cover all three, the stop rule must be the strictest of the three, and the
//! nightly-job question has nothing to do with the parser — it may well be a
//! thread the user opened two turns ago.
//!
//! So a turn resolves to a list of [`Strand`]s. Each carries its own
//! [`Reading`] and [`Engagement`], its relation to the strands beside it, and
//! its lineage to threads from earlier turns. The turn's engagement is
//! [`Engagement::compose`] over the strands, and the turn's composite reading
//! (kept on [`crate::Intent::reading`] for everything that wants one answer)
//! is the most consequential strand widened by the others.
//!
//! Segmentation is deterministic, like the rest of tier 1: sentence
//! boundaries, enumerated items, and a short list of sequencing and addition
//! markers. Plain "and" is deliberately not a boundary — "explain what this
//! and that mean" is one question — and a clause that carries no act signal of
//! its own is folded back into its neighbour rather than becoming a strand
//! that says nothing.

use std::collections::BTreeSet;

use serde::{Deserialize, Serialize};

use crate::axes::Act;
use crate::engage::Engagement;
use crate::reading::Reading;

/// How a strand relates to the strands beside it in the same request.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "kebab-case")]
pub enum StrandRelation {
    /// Stands on its own; order does not matter.
    Independent,
    /// Must happen after `after` ("then", "after that", "finally").
    Sequential { after: String },
    /// Refers to `on`'s result ("… and summarise it").
    Dependent { on: String },
}

/// How a strand relates to work from earlier turns.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "kebab-case")]
pub enum Lineage {
    /// A thread of its own.
    New,
    /// Carries on an open thread, and joins `merges` into it: other open
    /// threads the clause shares at least `MERGE_MIN_OVERLAP` content words
    /// with ("combine the parser work with the test cleanup").
    Continues {
        thread_id: String,
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        merges: Vec<String>,
    },
    /// Amends an open thread without discarding it. Only ever set from an
    /// explicit human command, never inferred (docs/design/47, control plane).
    Corrects { thread_id: String },
    /// Supersedes an open thread. Same rule. The replacing strand starts a
    /// thread of its own; `thread_id` names the one it replaces.
    Replaces { thread_id: String },
}

impl Lineage {
    /// The thread this strand carries on, if it carries one on. A
    /// replacement does not: it supersedes its thread and starts its own.
    pub fn continued_thread(&self) -> Option<&str> {
        match self {
            Lineage::New | Lineage::Replaces { .. } => None,
            Lineage::Continues { thread_id, .. } | Lineage::Corrects { thread_id } => {
                Some(thread_id)
            }
        }
    }

    /// Every thread this strand carries on: the one it continues and the
    /// ones it merges into it.
    pub fn continued_threads(&self) -> Vec<&str> {
        let mut threads: Vec<&str> = self.continued_thread().into_iter().collect();
        if let Lineage::Continues { merges, .. } = self {
            threads.extend(merges.iter().map(String::as_str));
        }
        threads
    }

    /// The thread this strand supersedes, if any.
    pub fn replaced_thread(&self) -> Option<&str> {
        match self {
            Lineage::Replaces { thread_id } => Some(thread_id),
            _ => None,
        }
    }
}

/// One piece of work inside a request.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Strand {
    /// `{turn_id}.{index}`: unique across turns, sessions and workspaces,
    /// because the host mints the turn id (a UUIDv7).
    pub strand_id: String,
    /// The thread this strand belongs to across turns. Equal to `strand_id`
    /// for a new thread and for a replacement; inherited for a continuation
    /// or a correction.
    pub thread_id: String,
    /// The clause, verbatim after scaffolding removal.
    pub text: String,
    pub reading: Reading,
    pub relation: StrandRelation,
    pub lineage: Lineage,
    pub engagement: Engagement,
}

/// What the host knows about a thread that is still open, for lineage.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ThreadFact {
    pub thread_id: String,
    pub act: Act,
    #[serde(default)]
    pub domains: BTreeSet<String>,
    /// A few content words from the thread's text, lowercase, for overlap.
    #[serde(default)]
    pub keywords: BTreeSet<String>,
}

/// A lineage the host already knows, from an explicit command.
///
/// `/goal fix …` and `/goal replace …` are the only way a strand becomes a
/// correction or a replacement; the resolver never infers either, because a
/// wrongly inferred replacement discards work.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum LineageHint {
    Corrects,
    Replaces,
}

// ------------------------------------------------------------ segmentation ---

/// Why a clause was split from the one before it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Boundary {
    /// The first clause.
    Start,
    /// A sequencing marker: the clause happens after the previous one.
    Sequence,
    /// An addition marker or a sentence boundary.
    Addition,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Clause {
    pub text: String,
    pub boundary: Boundary,
    /// The clause ended with a question mark.
    pub question: bool,
}

/// Markers that begin a new clause. Matched as whole words on the lowercase
/// token stream, longest first, so "and then" is not read as "and" + "then".
const SEQUENCE_MARKERS: &[&str] = &[
    "and then",
    "then",
    "after that",
    "afterwards",
    "and finally",
    "finally",
    "and after that",
    "once that is done",
    "once done",
];
// `plus` and `next` are not here: "plus-size", "the next release".
const ADDITION_MARKERS: &[&str] = &["and also", "also", "additionally", "as well as"];

/// The segmentation vocabulary, for the lexicon digest.
pub(crate) fn segmentation_fingerprint(out: &mut String) {
    for marker in SEQUENCE_MARKERS {
        out.push_str(&format!("seq:{marker}\n"));
    }
    for marker in ADDITION_MARKERS {
        out.push_str(&format!("add:{marker}\n"));
    }
}

/// Split a request into clauses.
///
/// Boundaries, in priority order: enumerated list items; newlines; sentence
/// terminators; `;`; sequencing markers; addition markers. Each marker must
/// be a whole word (or run of words), and the marker itself is dropped from
/// the clause that follows it.
pub(crate) fn segment(text: &str) -> Vec<Clause> {
    let mut clauses: Vec<Clause> = Vec::new();

    // Pass 1: hard boundaries — list items, newlines, sentence ends, `;`.
    let mut pieces: Vec<(String, Boundary, bool)> = Vec::new();
    let mut current = String::new();
    let mut chars = text.chars().peekable();
    while let Some(c) = chars.next() {
        match c {
            '\n' | ';' => {
                flush(&mut pieces, &mut current, Boundary::Addition, false);
            }
            '.' | '!' | '?' => {
                // A terminator followed by whitespace or end ends a sentence;
                // "v1.2" and "e.g." do not.
                let ends = chars.peek().is_none_or(|next| next.is_whitespace());
                // "1. run the tests": the dot belongs to the list marker.
                let list_marker = c == '.' && {
                    let head = current.trim();
                    !head.is_empty()
                        && head.len() <= 3
                        && head.chars().all(|ch| ch.is_ascii_digit())
                };
                // "e.g." / "i.e.": a dot after a single letter that itself
                // follows a dot.
                let abbreviation = c == '.' && {
                    let mut tail = current.chars().rev();
                    tail.next().is_some_and(|ch| ch.is_ascii_alphabetic())
                        && tail.next() == Some('.')
                };
                if ends && !list_marker && !abbreviation {
                    flush(&mut pieces, &mut current, Boundary::Addition, c == '?');
                } else {
                    current.push(c);
                }
            }
            _ => current.push(c),
        }
    }
    flush(&mut pieces, &mut current, Boundary::Addition, false);

    // Pass 2: soft boundaries inside each piece — sequencing and addition
    // markers, as whole words. A question mark belongs to the last clause of
    // its sentence.
    for (index, (piece, boundary, question)) in pieces.into_iter().enumerate() {
        let piece = strip_list_marker(&piece);
        let subs = split_on_markers(piece);
        let last = subs.len().saturating_sub(1);
        for (position, (sub, sub_boundary)) in subs.into_iter().enumerate() {
            let boundary = if position == 0 {
                if index == 0 {
                    Boundary::Start
                } else {
                    boundary
                }
            } else {
                sub_boundary
            };
            let trimmed = sub.trim().trim_matches(',').trim();
            if !trimmed.is_empty() {
                clauses.push(Clause {
                    text: trimmed.to_string(),
                    boundary,
                    question: question && position == last,
                });
            }
        }
    }
    if let Some(first) = clauses.first_mut() {
        first.boundary = Boundary::Start;
    }
    clauses
}

fn flush(
    pieces: &mut Vec<(String, Boundary, bool)>,
    current: &mut String,
    boundary: Boundary,
    question: bool,
) {
    let trimmed = current.trim();
    if !trimmed.is_empty() {
        pieces.push((trimmed.to_string(), boundary, question));
    }
    current.clear();
}

/// `- item`, `* item`, `1. item`, `1) item` → `item`.
fn strip_list_marker(piece: &str) -> &str {
    let trimmed = piece.trim_start();
    if let Some(rest) = trimmed
        .strip_prefix("- ")
        .or_else(|| trimmed.strip_prefix("* "))
    {
        return rest;
    }
    let digits = trimmed.chars().take_while(char::is_ascii_digit).count();
    if digits > 0 && digits <= 3 {
        let rest = &trimmed[digits..];
        if let Some(rest) = rest.strip_prefix(". ").or_else(|| rest.strip_prefix(") ")) {
            return rest;
        }
    }
    trimmed
}

/// Split one sentence on sequencing / addition markers.
///
/// Works on the original text so the clauses keep their spelling; markers are
/// located by scanning words with their byte offsets.
fn split_on_markers(text: &str) -> Vec<(&str, Boundary)> {
    // Word spans: (start, end) byte offsets of alphanumeric runs.
    let mut spans: Vec<(usize, usize)> = Vec::new();
    let mut start: Option<usize> = None;
    for (i, c) in text.char_indices() {
        if c.is_ascii_alphanumeric() || c == '\'' {
            if start.is_none() {
                start = Some(i);
            }
        } else if let Some(s) = start.take() {
            spans.push((s, i));
        }
    }
    if let Some(s) = start {
        spans.push((s, text.len()));
    }
    let words: Vec<String> = spans
        .iter()
        .map(|(s, e)| text[*s..*e].to_ascii_lowercase())
        .collect();

    let mut out = Vec::new();
    let mut clause_start = 0usize;
    let mut i = 0usize;
    while i < words.len() {
        let mut matched: Option<(usize, Boundary)> = None;
        for (markers, boundary) in [
            (SEQUENCE_MARKERS, Boundary::Sequence),
            (ADDITION_MARKERS, Boundary::Addition),
        ] {
            for marker in markers {
                let needle: Vec<&str> = marker.split(' ').collect();
                if i + needle.len() <= words.len()
                    && needle
                        .iter()
                        .zip(&words[i..i + needle.len()])
                        .all(|(a, b)| *a == b.as_str())
                {
                    // Longest match wins.
                    if matched.is_none_or(|(len, _)| needle.len() > len) {
                        matched = Some((needle.len(), boundary));
                    }
                }
            }
        }
        match matched {
            Some((len, boundary)) => {
                let (marker_start, _) = spans[i];
                let (_, marker_end) = spans[i + len - 1];
                let head = &text[clause_start..marker_start];
                if head.trim().trim_matches(',').trim().is_empty() {
                    // "then" as the first word of this clause: just drop it.
                } else {
                    out.push((head, Boundary::Start));
                }
                // The boundary applies to what follows the marker.
                clause_start = marker_end;
                i += len;
                // Record which boundary the *next* clause gets by pushing a
                // placeholder we fix up below.
                out.push(("", boundary));
            }
            _ => i += 1,
        }
    }
    out.push((&text[clause_start..], Boundary::Start));

    // Fold placeholders: ("", b) followed by (clause, _) → (clause, b).
    let mut folded: Vec<(&str, Boundary)> = Vec::new();
    let mut pending: Option<Boundary> = None;
    for (clause, boundary) in out {
        if clause.is_empty() {
            pending = Some(boundary);
            continue;
        }
        let boundary = pending.take().unwrap_or(boundary);
        folded.push((clause, boundary));
    }
    folded
}

/// Content words of a clause, for cross-turn overlap. Short and common words
/// are dropped so "the" does not link every thread to every other.
pub fn keywords(text: &str) -> BTreeSet<String> {
    const STOP: &[&str] = &[
        "the", "a", "an", "and", "or", "to", "of", "in", "on", "for", "it", "this", "that", "is",
        "are", "be", "with", "as", "at", "by", "from", "me", "my", "we", "our", "you", "your",
        "please", "can", "could", "would", "should", "do", "does", "did", "then", "also", "just",
        "now", "so", "if", "but", "not", "no", "yes", "into", "about",
    ];
    text.to_ascii_lowercase()
        .split(|c: char| !c.is_ascii_alphanumeric())
        .filter(|w| w.len() >= 3 && !STOP.contains(w))
        .map(str::to_string)
        .collect()
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::*;

    fn texts(text: &str) -> Vec<(String, Boundary)> {
        segment(text)
            .into_iter()
            .map(|c| (c.text, c.boundary))
            .collect()
    }

    #[test]
    fn a_single_clause_is_one_strand() {
        assert_eq!(
            texts("refactor the parser"),
            vec![("refactor the parser".to_string(), Boundary::Start)]
        );
    }

    #[test]
    fn then_splits_into_a_sequence_and_drops_the_marker() {
        assert_eq!(
            texts("first explain the parser, then refactor it"),
            vec![
                ("first explain the parser".to_string(), Boundary::Start),
                ("refactor it".to_string(), Boundary::Sequence),
            ]
        );
        assert_eq!(
            texts("explain the parser and then refactor it"),
            vec![
                ("explain the parser".to_string(), Boundary::Start),
                ("refactor it".to_string(), Boundary::Sequence),
            ]
        );
    }

    #[test]
    fn also_and_sentences_are_additions() {
        assert_eq!(
            texts("fix the login bug. Also check whether the nightly job ran"),
            vec![
                ("fix the login bug".to_string(), Boundary::Start),
                (
                    "check whether the nightly job ran".to_string(),
                    Boundary::Addition
                ),
            ]
        );
    }

    #[test]
    fn markers_inside_words_do_not_split() {
        assert_eq!(
            texts("fix the authentication bug in the login handler").len(),
            1
        );
        assert_eq!(texts("strengthen the plus-size handling").len(), 1);
    }

    #[test]
    fn plain_and_is_not_a_boundary() {
        assert_eq!(texts("explain what this and that mean").len(), 1);
    }

    #[test]
    fn enumerated_items_split_and_lose_their_markers() {
        let clauses =
            texts("do these:\n1. run the tests\n2. update the changelog\n- tag the release");
        assert_eq!(
            clauses.iter().map(|(t, _)| t.as_str()).collect::<Vec<_>>(),
            vec![
                "do these:",
                "run the tests",
                "update the changelog",
                "tag the release"
            ]
        );
    }

    #[test]
    fn version_numbers_and_abbreviations_are_not_sentence_ends() {
        assert_eq!(texts("upgrade to v1.2.3 e.g. via the installer").len(), 1);
    }

    #[test]
    fn keywords_drop_stop_words() {
        let k = keywords("Explain the parser and then refactor it");
        assert!(k.contains("parser") && k.contains("refactor") && k.contains("explain"));
        assert!(!k.contains("the") && !k.contains("and") && !k.contains("it"));
    }
}
