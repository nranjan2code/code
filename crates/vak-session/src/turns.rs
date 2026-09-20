//! `TurnIndex`: the ledger reorganized into turns (docs/design/68-context-
//! engine.md principle 3, §2, §10).
//!
//! A turn is one user directive plus every assistant step and tool exchange
//! until the final answer; it is the unit the request assembler works with
//! instead of individual messages. `TurnIndex::from_log` walks the ledger's
//! active chain once and never splits a turn. Everything here is rebuilt
//! from the ledger on demand — nothing here is itself persisted except
//! `TurnCard`, which is written once at turn close (`EntryPayload::TurnCard`)
//! and read back by a later `TurnIndex::from_log` rather than recomputed.

use std::collections::{HashMap, HashSet};

use serde::{Deserialize, Serialize};
use serde_json::Value;
use vak_llm::{ContentBlock, Message, Role};

use crate::log::SessionLog;
use crate::types::{EntryPayload, PresentationRecord, PresentationSource};

/// One assistant step within a turn: the assistant message (text and/or
/// `ToolUse` blocks) and the tool results that answered it, in arrival order.
#[derive(Debug, Clone)]
pub struct Step {
    pub assistant: Message,
    /// `(tool_use_id, content, is_error)`.
    pub results: Vec<(String, String, bool)>,
}

/// One user directive plus every assistant step and tool exchange until the
/// final answer (principle 3: "the turn is the unit; a turn is never
/// split").
#[derive(Debug, Clone)]
pub struct Turn {
    /// The directive entry id — stable identity for `recall({ turn })` and
    /// `PresentationRecord::turn_id`.
    pub id: String,
    pub directive: Message,
    pub steps: Vec<Step>,
    /// The turn's final text-only assistant message, once it has one.
    pub final_answer: Option<Message>,
    /// Every tool_use_id with a recorded result in this turn, in the order
    /// results arrived. Includes presentation-tool acks; `TurnCard::did`
    /// filters those out via `presentation_records`.
    pub evidence: Vec<String>,
    /// Presentation ledger-entry ids answered by this turn, in emit order.
    pub presentations: Vec<String>,
    /// The turn's closing card, once written (`EntryPayload::TurnCard`).
    pub card: Option<TurnCard>,
    /// `false` only for the last turn when the chain ends without a final
    /// assistant text after the directive.
    pub closed: bool,
    /// `true` when a reset-with-handoff entry (docs/design/42) follows this
    /// turn on the chain: the turn is invisible to the model, so it is
    /// never planned, never packeted and never listed as covered — though
    /// it stays in the index for `recall({ turn })` and turn numbering.
    pub behind_reset: bool,
    /// Full presentation records parallel to `presentations`, kept alongside
    /// the id list so `full_record`/`build_card` never re-walk the ledger.
    presentation_records: Vec<PresentationRecord>,
    /// The turn's resolved intent reading, when an `Intent` entry exists.
    reading: Option<ReadingKey>,
    /// Every message after the directive, in ledger order, INCLUDING
    /// control ones (nudges) — the raw material for `current_verbatim`.
    /// Control messages are still never a `Step` and never end up in
    /// `full_record`: they are mid-turn runtime scaffolding for the model
    /// still working the turn, not part of what a later turn should see
    /// (docs/design/68-context-engine.md §10's "within a turn" rules), but
    /// they must stay verbatim in the CURRENT turn's own request or a
    /// repair nudge would never reach the model at all.
    raw_tail: Vec<Message>,
}

/// A stored compaction packet: the summary of one inclusive range of closed
/// turns (docs/design/68 §2: "Compaction packets become
/// `TurnIndex.packets`"), keyed by that range and reused only by a plan
/// asking for exactly it. Reset-with-handoff entries are not packets and
/// never appear here.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Packet {
    pub first_turn_id: String,
    pub last_turn_id: String,
    pub model: String,
    pub summary: String,
}

/// The fidelity at which a closed turn rides along in one request
/// (docs/design/68-context-engine.md §4/§10). Defined here (not in
/// `vak-context`, where `WorkingSetPlanner::plan` actually computes it) so
/// `SessionLog::derive_with_plan` can consume a `WorkingSetPlan` without
/// vak-session depending on vak-context; `vak_context::planner` re-exports both
/// types for callers.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Fidelity {
    /// Two-message record (directive + trace/presentations/narration).
    Full,
    /// One `<turns>` line (`TurnCard::line`).
    Card,
    /// Represented only by a compaction packet covering a range of turns.
    Packet,
}

/// The plan for one request: which turns ride at which fidelity, which
/// range (if any) is represented only by a packet, which older turns were
/// promoted by relevance, and the budget accounting that produced it.
/// Computed by `vak_context::planner::plan`; consumed by
/// `SessionLog::derive_with_plan`.
#[derive(Debug, Clone, Default)]
pub struct WorkingSetPlan {
    /// Chronological order (oldest first), one entry per closed turn that
    /// is `Full` or `Card`. A turn absent from this list and not covered by
    /// `packet_range` simply has no card yet — `derive_with_plan` treats
    /// that as already covered by an existing `Compaction` entry.
    pub per_turn: Vec<(String, Fidelity)>,
    /// The contiguous, oldest-to-newest range of turns represented only by
    /// a packet, inclusive — `(first_turn_id, last_turn_id)`.
    pub packet_range: Option<(String, String)>,
    /// Turn ids promoted to `Full` by relevance (search, reading overlap,
    /// or anaphora) rather than by the recency fill.
    pub retrieved: Vec<String>,
    pub budget: u64,
    pub spent: u64,
}

/// The ledger reorganized into turns, built once per request.
#[derive(Debug, Clone, Default)]
pub struct TurnIndex {
    pub turns: Vec<Turn>,
    pub packets: Vec<Packet>,
}

impl TurnIndex {
    /// Walks `log.chain_to_root()` once, in order. Control messages (nudges,
    /// intent notes — `MessageRecord::control_kind().is_some()`) are not
    /// turns and not steps: they never start a turn and never become a step.
    pub fn from_log(log: &SessionLog) -> TurnIndex {
        let chain = log.chain_to_root();
        let mut turns: Vec<Turn> = Vec::new();
        let mut packets: Vec<Packet> = Vec::new();

        for entry in &chain {
            match &entry.payload {
                EntryPayload::Message(record) => {
                    let msg = &record.message;
                    if record.control_kind().is_some() {
                        // Not a turn, not a step — but still mid-turn
                        // scaffolding for the OPEN turn's own request; kept
                        // verbatim there via `raw_tail` (see its doc
                        // comment) and dropped entirely once the turn
                        // closes and is rendered as `full_record`.
                        if let Some(turn) = turns.last_mut() {
                            turn.raw_tail.push(msg.clone());
                        }
                        continue;
                    }
                    match msg.role {
                        Role::User => {
                            let has_text = msg
                                .content
                                .iter()
                                .any(|b| matches!(b, ContentBlock::Text { .. }));
                            let has_tool_result = msg
                                .content
                                .iter()
                                .any(|b| matches!(b, ContentBlock::ToolResult { .. }));
                            if has_text && !has_tool_result {
                                turns.push(Turn {
                                    id: entry.id.clone(),
                                    directive: msg.clone(),
                                    steps: Vec::new(),
                                    final_answer: None,
                                    evidence: Vec::new(),
                                    presentations: Vec::new(),
                                    card: None,
                                    closed: true,
                                    behind_reset: false,
                                    presentation_records: Vec::new(),
                                    reading: None,
                                    raw_tail: Vec::new(),
                                });
                            } else if has_tool_result && let Some(turn) = turns.last_mut() {
                                turn.raw_tail.push(msg.clone());
                                if let Some(step) = turn.steps.last_mut() {
                                    for block in &msg.content {
                                        if let ContentBlock::ToolResult {
                                            tool_use_id,
                                            content,
                                            is_error,
                                        } = block
                                        {
                                            step.results.push((
                                                tool_use_id.clone(),
                                                content.clone(),
                                                *is_error,
                                            ));
                                            turn.evidence.push(tool_use_id.clone());
                                        }
                                    }
                                }
                            }
                        }
                        Role::Assistant => {
                            let has_tool_use = msg
                                .content
                                .iter()
                                .any(|b| matches!(b, ContentBlock::ToolUse { .. }));
                            if let Some(turn) = turns.last_mut() {
                                turn.raw_tail.push(msg.clone());
                                if has_tool_use {
                                    turn.steps.push(Step {
                                        assistant: msg.clone(),
                                        results: Vec::new(),
                                    });
                                } else {
                                    turn.final_answer = Some(msg.clone());
                                }
                            }
                        }
                    }
                }
                EntryPayload::Intent(record) => {
                    if let Some(turn) = turns.last_mut() {
                        turn.reading = Some(ReadingKey::from_reading(&record.reading));
                    }
                }
                EntryPayload::Compaction(c) if c.reset_all => {
                    for turn in &mut turns {
                        turn.behind_reset = true;
                    }
                }
                EntryPayload::Compaction(c) => {
                    packets.push(Packet {
                        first_turn_id: c.first_turn_id.clone(),
                        last_turn_id: c.last_turn_id.clone(),
                        model: c.model.clone(),
                        summary: c.summary.clone(),
                    });
                }
                _ => {}
            }
        }

        // Only the last turn can be open: every earlier turn already saw a
        // later directive, which cannot happen unless the run ended it one
        // way or another. The last turn is open exactly when it never
        // produced a final text-only answer.
        if let Some(last) = turns.last_mut() {
            last.closed = last.final_answer.is_some();
        }

        let by_id: HashMap<String, usize> = turns
            .iter()
            .enumerate()
            .map(|(i, t)| (t.id.clone(), i))
            .collect();
        for (id, record) in log.presentations() {
            if let Some(&idx) = by_id.get(&record.turn_id) {
                turns[idx].presentations.push(id);
                turns[idx].presentation_records.push(record.clone());
            }
        }
        for (turn_id, card) in log.turn_cards() {
            if let Some(&idx) = by_id.get(&turn_id) {
                turns[idx].card = Some(card);
            }
        }

        TurnIndex { turns, packets }
    }

    /// 1-based, chronological — the numbering `TurnCard::line` and
    /// `recall({ turn })` share.
    pub fn turn_by_number(&self, n: usize) -> Option<&Turn> {
        n.checked_sub(1).and_then(|i| self.turns.get(i))
    }

    pub fn turn_by_id(&self, id: &str) -> Option<&Turn> {
        self.turns.iter().find(|t| t.id == id)
    }

    /// In-memory BM25 over each closed turn's card index text
    /// (docs/design/68 §10). A turn with no card yet (never closed, or
    /// closed before this workstream landed) never matches. Highest score
    /// first; ties break by turn id for determinism.
    /// Gives every closed turn that has no written `TurnCard` a provisional
    /// one, in memory only, so a ledger written before cards existed (or a
    /// run that ended before its card was appended) is still costed,
    /// searchable and plannable — never silently projected at `Full` around
    /// the budget. The provisional card's narration is the final answer's
    /// opening sentence; nothing is written to the ledger.
    pub fn ensure_cards(&mut self, estimate_tokens: &dyn Fn(&str) -> u64) {
        for turn in self.turns.iter_mut() {
            if turn.closed && turn.card.is_none() {
                let narration = turn
                    .final_answer
                    .as_ref()
                    .map(|answer| first_sentence_by_words(&answer.text_content(), 60))
                    .unwrap_or_default();
                turn.card = Some(turn.build_card("unrecorded", narration, estimate_tokens));
            }
        }
    }

    pub fn search(&self, query: &str) -> Vec<(String, f64)> {
        let terms = crate::search::tokenize_impl(query);
        let phrase = crate::search::normalize_impl(query);
        if terms.is_empty() || phrase.is_empty() {
            return Vec::new();
        }
        let mut scored: Vec<(String, f64)> = self
            .turns
            .iter()
            .filter_map(|turn| {
                let card = turn.card.as_ref()?;
                let text = card.index_text();
                let normalized = crate::search::normalize_impl(&text);
                let entities = crate::search::extract_entities(&text);
                let score =
                    crate::search::score_normalized(&normalized, &terms, &phrase, &entities);
                (score > 0.0).then(|| (turn.id.clone(), score as f64))
            })
            .collect();
        scored.sort_by(|a, b| b.1.total_cmp(&a.1).then(a.0.cmp(&b.0)));
        scored
    }
}

impl Turn {
    fn tool_use(&self, id: &str) -> Option<(&str, &Value)> {
        self.steps
            .iter()
            .flat_map(|step| step.assistant.content.iter())
            .find_map(|block| match block {
                ContentBlock::ToolUse {
                    id: call_id,
                    name,
                    input,
                } if call_id == id => Some((name.as_str(), input)),
                _ => None,
            })
    }

    fn tool_result(&self, id: &str) -> Option<(&str, bool)> {
        self.steps
            .iter()
            .flat_map(|step| step.results.iter())
            .find_map(|(result_id, content, is_error)| {
                (result_id == id).then_some((content.as_str(), *is_error))
            })
    }

    /// The resolved evidence for one of this turn's tool_use_ids, or `None`
    /// if the id belongs to a different turn or has no result yet.
    pub fn evidence_for(&self, id: &str) -> Option<Evidence> {
        let (tool, input) = self.tool_use(id)?;
        let (content, is_error) = self.tool_result(id)?;
        Some(Evidence {
            tool: tool.to_string(),
            input: input.clone(),
            content: content.to_string(),
            is_error,
        })
    }

    fn presentation_tool_use_ids(&self) -> HashSet<&str> {
        self.presentation_records
            .iter()
            .filter_map(|record| match &record.source {
                PresentationSource::ToolCall { tool_use_id } => Some(tool_use_id.as_str()),
                PresentationSource::Fence { .. } => None,
            })
            .collect()
    }

    /// One `TraceLine` per non-presentation tool call, in call order
    /// (docs/design/68 §10's "did").
    fn trace_lines(&self) -> Vec<TraceLine> {
        let card_ids = self.presentation_tool_use_ids();
        self.evidence
            .iter()
            .filter(|id| !card_ids.contains(id.as_str()))
            .filter_map(|id| {
                let evidence = self.evidence_for(id)?;
                Some(TraceLine {
                    tool: evidence.tool.clone(),
                    args_digest: args_digest(&evidence.tool, &evidence.input),
                    evidence_id: id.clone(),
                    shape: evidence_shape(&evidence),
                })
            })
            .collect()
    }

    /// The follow-up projection of a closed turn (docs/design/68 §3, §10):
    /// the turn as it was recorded, with thinking dropped, every evidence
    /// result replaced by its schema-driven digest (recallable by id), every
    /// card result replaced by the short ack, and a duplicate `vak` fence in
    /// the narration omitted. The tool calls themselves stay real
    /// `tool_use`/`tool_result` pairs: a later turn on a small model imitates
    /// what the assistant role did, so the record must show the call pattern
    /// (search → card), not a prose transcript of it. Pairs are always
    /// complete (a call whose result never arrived is dropped), so the
    /// projection is API-valid on every provider and byte-stable once the
    /// turn closes.
    pub fn full_record(&self) -> Vec<Message> {
        let card_ids = self.presentation_tool_use_ids();
        let presentation_for_call: HashMap<&str, &str> = self
            .presentations
            .iter()
            .zip(self.presentation_records.iter())
            .filter_map(|(id, record)| match &record.source {
                PresentationSource::ToolCall { tool_use_id } => {
                    Some((tool_use_id.as_str(), id.as_str()))
                }
                PresentationSource::Fence { .. } => None,
            })
            .collect();
        let mut out = vec![self.directive.clone()];
        for step in &self.steps {
            let answered: HashSet<&str> =
                step.results.iter().map(|(id, _, _)| id.as_str()).collect();
            let assistant: Vec<ContentBlock> = step
                .assistant
                .content
                .iter()
                .filter(|block| match block {
                    ContentBlock::Thinking { .. } => false,
                    ContentBlock::ToolUse { id, .. } => answered.contains(id.as_str()),
                    _ => true,
                })
                .cloned()
                .collect();
            if assistant.is_empty() {
                continue;
            }
            out.push(Message {
                role: Role::Assistant,
                content: assistant,
            });
            let results: Vec<ContentBlock> = step
                .results
                .iter()
                .map(|(id, content, is_error)| {
                    let rendered = if card_ids.contains(id.as_str()) {
                        presentation_for_call
                            .get(id.as_str())
                            .map(|pres| format!("{{\"presentation\":\"{pres}\",\"ok\":true}}"))
                            .unwrap_or_else(|| content.clone())
                    } else {
                        match self.evidence_for(id) {
                            Some(evidence) if !evidence.is_error => evidence_digest(id, &evidence),
                            _ => content.clone(),
                        }
                    };
                    ContentBlock::ToolResult {
                        tool_use_id: id.clone(),
                        content: rendered,
                        is_error: *is_error,
                    }
                })
                .collect();
            if !results.is_empty() {
                out.push(Message {
                    role: Role::User,
                    content: results,
                });
            }
        }
        if let Some(answer) = &self.final_answer {
            let narration =
                strip_duplicate_fences(&answer.text_content(), &self.presentation_records);
            if !narration.trim().is_empty() {
                out.push(Message {
                    role: Role::Assistant,
                    content: vec![ContentBlock::text(narration.trim().to_string())],
                });
            }
        }
        out
    }

    /// The open turn exactly as recorded: directive, then every message
    /// since (assistant steps, tool results, and any control nudge),
    /// verbatim and in order. Used for the still-open turn, which stays
    /// verbatim because it is still being worked from within this run — a
    /// nudge must reach the model on its next request, so it stays here
    /// even though it is dropped once the turn closes and is rendered as
    /// `full_record`.
    pub fn current_verbatim(&self) -> Vec<Message> {
        let mut out = vec![self.directive.clone()];
        out.extend(self.raw_tail.iter().cloned());
        out
    }

    /// Builds this turn's closing `TurnCard`. `narration` is the caller's
    /// already-resolved narration (verbatim when short, or a side-call gist
    /// when long — docs/design/68 §10); `outcome` is the turn's terminal
    /// state; `estimate_tokens` measures a rendered text (the host's
    /// `CapacityProfile::estimate_tokens`).
    pub fn build_card(
        &self,
        outcome: impl Into<String>,
        narration: String,
        estimate_tokens: &dyn Fn(&str) -> u64,
    ) -> TurnCard {
        let reading = self.reading.clone().unwrap_or_default();
        let asked = first_sentence_by_words(&self.directive.text_content(), 60);
        let did = self.trace_lines();
        let presentations: Vec<PresentationRef> = self
            .presentations
            .iter()
            .zip(self.presentation_records.iter())
            .map(|(id, record)| PresentationRef {
                id: id.clone(),
                semantic_type: record.semantic_type.clone(),
                title: record.title.clone(),
                digest: record.identity_digest.clone(),
                derived_from: record.derived_from.clone(),
            })
            .collect();
        let answered = Answer {
            presentations,
            narration,
        };
        let full_text: String = self
            .full_record()
            .iter()
            .map(Message::text_content)
            .collect::<Vec<_>>()
            .join("\n");
        let tokens_full = estimate_tokens(&full_text);
        let mut card = TurnCard {
            turn_id: self.id.clone(),
            asked,
            did,
            answered,
            outcome: outcome.into(),
            reading,
            tokens_full,
            tokens_card: 0,
        };
        let index_text = card.index_text();
        card.tokens_card = estimate_tokens(&index_text);
        card
    }
}

/// What the request assembler and `recall` see of one past tool result:
/// the tool that produced it, its call arguments, its content, and whether
/// it failed. Resolved from the ledger by `SessionLog::evidence`.
#[derive(Debug, Clone)]
pub struct Evidence {
    pub tool: String,
    pub input: Value,
    pub content: String,
    pub is_error: bool,
}

/// Drops every ```` ```vak ```` fence from `narration` whose `payload`
/// digest matches one of this turn's presentation entries: the card is
/// already shown through the entry, and replaying the fence would teach the
/// model the duplicate it is told never to produce (docs/design/68 §10).
fn strip_duplicate_fences(narration: &str, presentations: &[PresentationRecord]) -> String {
    let known: HashSet<&str> = presentations
        .iter()
        .map(|record| record.payload_digest.as_str())
        .collect();
    let mut out = String::with_capacity(narration.len());
    let mut lines = narration.lines().peekable();
    while let Some(line) = lines.next() {
        let trimmed = line.trim_start();
        let opens_fence = trimmed
            .strip_prefix("```")
            .or_else(|| trimmed.strip_prefix("~~~"))
            .map(|rest| rest.trim() == "vak")
            .unwrap_or(false);
        if !opens_fence {
            out.push_str(line);
            out.push('\n');
            continue;
        }
        let fence_marker = &trimmed[..3];
        let mut body = String::new();
        let mut closed = false;
        for inner in lines.by_ref() {
            if inner.trim_start().starts_with(fence_marker) {
                closed = true;
                break;
            }
            body.push_str(inner);
            body.push('\n');
        }
        let duplicate = serde_json::from_str::<Value>(&body)
            .ok()
            .and_then(|value| value.get("payload").cloned())
            .map(|payload| crate::types::payload_digest(&payload))
            .map(|digest| known.contains(digest.as_str()))
            .unwrap_or(false);
        if !duplicate {
            out.push_str(line);
            out.push('\n');
            out.push_str(&body);
            if closed {
                out.push_str(fence_marker);
                out.push('\n');
            }
        }
    }
    out
}

/// A content-aware summary of one result, ending with the fixed
/// `[evidence:<id> — <n> chars; call recall to expand]` tag (docs/design/68
/// §3). Never a character-count cut: the body is shape-driven, and only the
/// bash/text digests select by *line* count.
pub fn evidence_digest(id: &str, evidence: &Evidence) -> String {
    let chars = evidence.content.chars().count();
    let tag = format!("[evidence:{id} \u{2014} {chars} chars; call recall to expand]");
    if evidence.is_error {
        return format!("{}\n{tag}", evidence.content);
    }
    let body = if evidence.tool == "bash" {
        bash_digest(&evidence.content)
    } else if let Ok(value) = serde_json::from_str::<Value>(&evidence.content) {
        json_digest(&value)
    } else {
        text_digest(&evidence.content)
    };
    format!("{body}\n{tag}")
}

/// The short parenthetical a `TraceLine` carries (`"8 results, 14.2k
/// chars"`) — a summary, not the full digest.
pub fn evidence_shape(evidence: &Evidence) -> String {
    let chars = evidence.content.chars().count();
    if evidence.is_error {
        return "error".to_string();
    }
    if evidence.tool == "bash" {
        let exit = evidence.content.lines().find_map(|line| {
            line.to_ascii_lowercase()
                .contains("exit code")
                .then(|| line.trim().to_string())
        });
        return match exit {
            Some(exit) => format!("{exit}, {chars} chars"),
            None => format!("{chars} chars"),
        };
    }
    if let Ok(value) = serde_json::from_str::<Value>(&evidence.content) {
        return match &value {
            Value::Array(items) if !items.is_empty() && items.iter().all(is_link_like) => {
                format!("{} results, {chars} chars", items.len())
            }
            Value::Array(items) => format!("array[{}], {chars} chars", items.len()),
            Value::Object(map) => format!("{} keys, {chars} chars", map.len()),
            _ => format!("{chars} chars"),
        };
    }
    let lines = evidence.content.lines().count();
    format!("{lines} lines, {chars} chars")
}

fn is_link_like(value: &Value) -> bool {
    value.is_object() && (value.get("url").is_some() || value.get("title").is_some())
}

fn json_digest(value: &Value) -> String {
    match value {
        Value::Array(items) => array_digest(items),
        Value::Object(map) => object_digest(map),
        other => other.to_string(),
    }
}

fn array_digest(items: &[Value]) -> String {
    if !items.is_empty() && items.iter().all(is_link_like) {
        let mut out = format!("{} results:\n", items.len());
        for item in items {
            let title = item.get("title").and_then(Value::as_str).unwrap_or("");
            let url = item.get("url").and_then(Value::as_str).unwrap_or("");
            out.push_str(&format!("- {title} ({url})\n"));
        }
        return out.trim_end().to_string();
    }
    let mut out = format!("array, {} items", items.len());
    if let Some(first) = items.first() {
        out.push_str(&format!(
            "; first: {}",
            serde_json::to_string(first).unwrap_or_default()
        ));
    }
    out
}

fn object_digest(map: &serde_json::Map<String, Value>) -> String {
    let keys: Vec<&str> = map.keys().map(String::as_str).collect();
    let mut lines = vec![format!("object, keys: {}", keys.join(", "))];
    for (key, value) in map {
        if let Value::Array(items) = value {
            if !items.is_empty() && items.iter().all(is_link_like) {
                lines.push(format!("{key}: {}", array_digest(items)));
            } else {
                lines.push(format!("{key}: array[{}]", items.len()));
            }
        }
    }
    lines.join("\n")
}

fn text_digest(content: &str) -> String {
    let headings: Vec<&str> = content
        .lines()
        .map(str::trim_start)
        .filter(|line| line.starts_with('#'))
        .collect();
    // The opening of the first paragraph: its first sentence, bounded by
    // words. A paragraph with no whitespace at all is not prose (a blob, a
    // token, base64) and has no opening worth quoting — it would "digest" to
    // itself — so only its counts are reported.
    let first_paragraph = content
        .split("\n\n")
        .map(str::trim)
        .find(|paragraph| !paragraph.is_empty() && !paragraph.starts_with('#'))
        .filter(|paragraph| paragraph.contains(char::is_whitespace))
        .map(|paragraph| truncate_words(first_sentence(paragraph), 60))
        .unwrap_or_default();
    let lines = content.lines().count();
    let bytes = content.len();
    let mut out = String::new();
    if !headings.is_empty() {
        out.push_str("headings: ");
        out.push_str(&headings.join(" | "));
        out.push('\n');
    }
    if !first_paragraph.is_empty() {
        out.push_str(&first_paragraph);
        out.push('\n');
    }
    out.push_str(&format!("({lines} lines, {bytes} bytes)"));
    out
}

fn bash_digest(content: &str) -> String {
    let lines: Vec<&str> = content.lines().collect();
    let mut out = String::new();
    if let Some(exit) = lines
        .iter()
        .find(|line| line.to_ascii_lowercase().contains("exit code"))
    {
        out.push_str(exit);
        out.push('\n');
    }
    if lines.len() <= 20 {
        out.push_str(&lines.join("\n"));
    } else {
        out.push_str(&lines[..10].join("\n"));
        out.push_str("\n...\n");
        out.push_str(&lines[lines.len() - 10..].join("\n"));
    }
    out
}

/// The input JSON with string values longer than one sentence replaced by
/// their first sentence — never for `bash`/`edit`/`write`, whose inputs stay
/// whole (docs/design/68 §10's `TraceLine.args_digest`).
fn args_digest(tool: &str, input: &Value) -> Value {
    if matches!(tool, "bash" | "edit" | "write") {
        return input.clone();
    }
    shorten_strings(input)
}

fn shorten_strings(value: &Value) -> Value {
    match value {
        Value::String(s) => Value::String(first_sentence(s).to_string()),
        Value::Array(items) => Value::Array(items.iter().map(shorten_strings).collect()),
        Value::Object(map) => Value::Object(
            map.iter()
                .map(|(key, value)| (key.clone(), shorten_strings(value)))
                .collect(),
        ),
        other => other.clone(),
    }
}

/// The text up to and including its first sentence-ending punctuation
/// (`.`, `!`, `?`) followed by a space, newline, or end of string. Returns
/// the whole trimmed text unchanged when it has none — never a character
/// count, always a semantic boundary.
pub(crate) fn first_sentence(text: &str) -> &str {
    let trimmed = text.trim();
    let bytes = trimmed.as_bytes();
    for (i, b) in bytes.iter().enumerate() {
        if matches!(b, b'.' | b'!' | b'?') {
            let after = i + 1;
            if after >= bytes.len() || matches!(bytes[after], b' ' | b'\n') {
                return &trimmed[..after];
            }
        }
    }
    trimmed
}

fn first_sentence_by_words(text: &str, max_words: usize) -> String {
    let trimmed = text.trim();
    if trimmed.split_whitespace().count() <= max_words {
        return trimmed.to_string();
    }
    first_sentence(trimmed).to_string()
}

fn truncate_words(text: &str, max_words: usize) -> String {
    let words: Vec<&str> = text.split_whitespace().collect();
    if words.len() <= max_words {
        return text.trim().to_string();
    }
    format!("{}\u{2026}", words[..max_words].join(" "))
}

/// Act/domains/modalities distilled from a turn's `vak_intent::Reading` —
/// enough to group and filter turns without carrying the full reading (which
/// includes provenance and confidence irrelevant to a card).
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct ReadingKey {
    pub act: String,
    pub domains: Vec<String>,
    pub modalities: Vec<String>,
}

impl ReadingKey {
    pub fn from_reading(reading: &vak_intent::Reading) -> Self {
        let mut modalities: Vec<String> = reading
            .input_modalities
            .iter()
            .chain(reading.output_modalities.iter())
            .map(|m| m.as_str().to_string())
            .collect();
        modalities.sort();
        modalities.dedup();
        ReadingKey {
            act: reading.act.as_str().to_string(),
            domains: reading.domains.iter().cloned().collect(),
            modalities,
        }
    }
}

/// One non-presentation tool call in a turn's trace (docs/design/68 §10).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TraceLine {
    pub tool: String,
    pub args_digest: Value,
    pub evidence_id: String,
    pub shape: String,
}

/// A card emitted in this turn, as referenced from its `TurnCard`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PresentationRef {
    pub id: String,
    pub semantic_type: String,
    pub title: String,
    /// Schema-driven digest (`PresentationRecord::identity_digest`).
    pub digest: String,
    pub derived_from: Vec<String>,
}

/// The turn's answer: the presentations it emitted (the answer itself) and
/// the narration around them.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Answer {
    pub presentations: Vec<PresentationRef>,
    pub narration: String,
}

/// A turn's closing card (docs/design/68-context-engine.md §10): the
/// three-layer answer (evidence trace, presentations, narration) plus enough
/// bookkeeping to render it as a `<turns>` line or promote it to a full
/// record. Written once at turn close (`EntryPayload::TurnCard`) and never
/// rewritten.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TurnCard {
    pub turn_id: String,
    pub asked: String,
    pub did: Vec<TraceLine>,
    pub answered: Answer,
    pub outcome: String,
    pub reading: ReadingKey,
    pub tokens_full: u64,
    pub tokens_card: u64,
}

impl TurnCard {
    /// The text BM25 indexes this card by: `asked` + every presentation's
    /// title/digest + narration + trace tool names and args (docs/design/68
    /// §10).
    pub fn index_text(&self) -> String {
        let mut parts = vec![self.asked.clone()];
        for presentation in &self.answered.presentations {
            parts.push(presentation.title.clone());
            parts.push(presentation.digest.clone());
        }
        parts.push(self.answered.narration.clone());
        for trace in &self.did {
            parts.push(trace.tool.clone());
            parts.push(trace.args_digest.to_string());
        }
        parts.push(self.reading.act.clone());
        parts.extend(self.reading.domains.iter().cloned());
        parts.join(" ")
    }

    /// The one-line `<turns>` rendering: `#<n> asked: … → did: search×2 →
    /// research.synthesis "Sensex 15 Sep" [pres:a1; ev:9f2,9f3]`.
    pub fn line(&self, n: usize) -> String {
        let did_summary = summarize_trace(&self.did);
        let outcome_part = match self.answered.presentations.first() {
            Some(presentation) => {
                let evidence_ids: Vec<&str> = self
                    .did
                    .iter()
                    .map(|trace| trace.evidence_id.as_str())
                    .collect();
                let evidence_part = if evidence_ids.is_empty() {
                    String::new()
                } else {
                    format!("; ev:{}", evidence_ids.join(","))
                };
                format!(
                    "{} \"{}\" [pres:{}{evidence_part}]",
                    presentation.semantic_type, presentation.title, presentation.id
                )
            }
            None => format!("\"{}\"", truncate_words(&self.answered.narration, 12)),
        };
        format!(
            "#{n} asked: {} \u{2192} did: {did_summary} \u{2192} {outcome_part}",
            self.asked
        )
    }
}

fn summarize_trace(did: &[TraceLine]) -> String {
    if did.is_empty() {
        return "nothing".to_string();
    }
    let mut counts: Vec<(String, usize)> = Vec::new();
    for trace in did {
        match counts.iter_mut().find(|(name, _)| *name == trace.tool) {
            Some(existing) => existing.1 += 1,
            None => counts.push((trace.tool.clone(), 1)),
        }
    }
    counts
        .into_iter()
        .map(|(name, n)| {
            if n > 1 {
                format!("{name}\u{d7}{n}")
            } else {
                name
            }
        })
        .collect::<Vec<_>>()
        .join(", ")
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::*;
    use crate::types::{
        ActivityKind, ActivityRecord, ActivityStatus, CompactionEntry, Entry, FrozenContract,
        IntentRecord, MessageRecord, PresentationRecord, PresentationSource, SessionHeader,
        TurnCardRecord,
    };
    use crate::{SessionLog, SessionPath};
    use vak_llm::{ContentBlock as CB, Message as M, Role as R};

    fn header(cwd: &std::path::Path) -> SessionHeader {
        SessionHeader {
            agent: None,
            session_id: "s1".into(),
            created_at: chrono::Utc::now(),
            cwd: cwd.to_path_buf(),
            parent_session_id: None,
            contract_id: None,
            work_item_id: None,
            conversation: None,
            contract: FrozenContract {
                app_version: "test".into(),
                provider: "scripted".into(),
                model: "m".into(),
                route_ladder: Vec::new(),
                route_objective: String::new(),
                route_annotations: Vec::new(),
                system_prompt: String::new(),
                permission_mode: "workspace-write".into(),
                capabilities: Vec::new(),
                prompt_layers: Vec::new(),
            },
        }
    }

    fn open_log(dir: &std::path::Path) -> SessionLog {
        let path = SessionPath::new_session_file(dir, dir, "s1");
        SessionLog::create(path, header(dir)).unwrap()
    }

    fn user_text(text: &str) -> MessageRecord {
        MessageRecord {
            message: M::user_text(text),
            meta: None,
        }
    }

    fn assistant_tool_call(id: &str, name: &str, input: Value) -> MessageRecord {
        MessageRecord {
            message: M::assistant(vec![CB::ToolUse {
                id: id.into(),
                name: name.into(),
                input,
            }]),
            meta: None,
        }
    }

    fn tool_result(id: &str, content: &str) -> MessageRecord {
        MessageRecord {
            message: M {
                role: R::User,
                content: vec![CB::tool_result(id, content)],
            },
            meta: None,
        }
    }

    fn assistant_text(text: &str) -> MessageRecord {
        MessageRecord {
            message: M::assistant(vec![CB::text(text)]),
            meta: None,
        }
    }

    /// Builds a fixture ledger with two closed turns (one plain, one with a
    /// tool call and a presentation) and a third turn left open (a tool call
    /// with no final answer yet).
    fn two_closed_one_open(dir: &std::path::Path) -> (SessionLog, String, String, String) {
        let mut log = open_log(dir);
        let t1 = log.append_message(user_text("hello there")).unwrap().id;
        log.append_message(assistant_text("hi, how can I help?"))
            .unwrap();

        let t2 = log
            .append_message(user_text("search for rust news"))
            .unwrap()
            .id;
        log.append_message(assistant_tool_call(
            "call-1",
            "search",
            serde_json::json!({"query": "rust news"}),
        ))
        .unwrap();
        log.append_message(tool_result(
            "call-1",
            r#"[{"title":"Rust 2.0","url":"https://example.com/a"}]"#,
        ))
        .unwrap();
        log.append_message(assistant_tool_call(
            "card-1",
            "emit_research_card",
            serde_json::json!({"semantic_type": "research.synthesis"}),
        ))
        .unwrap();
        log.append_message(tool_result(
            "card-1",
            r#"{"presentation":"pres-1","ok":true}"#,
        ))
        .unwrap();
        log.append_message(assistant_text("Here is what I found."))
            .unwrap();
        log.append_presentation(PresentationRecord {
            turn_id: t2.clone(),
            source: PresentationSource::ToolCall {
                tool_use_id: "card-1".into(),
            },
            semantic_type: "research.synthesis".into(),
            skill_id: "skill".into(),
            skill_version: "1".into(),
            schema_version: 1,
            payload: serde_json::json!({"takeaways": ["Rust 2.0 shipped"]}),
            payload_digest: "digest1".into(),
            derived_from: vec!["call-1".into()],
            title: "Rust news".into(),
            identity_digest: "Rust news: Rust 2.0 shipped".into(),
        })
        .unwrap();

        let t3 = log
            .append_message(user_text("now check the changelog"))
            .unwrap()
            .id;
        log.append_message(assistant_tool_call(
            "call-2",
            "webfetch",
            serde_json::json!({"url": "https://example.com/changelog"}),
        ))
        .unwrap();
        (log, t1, t2, t3)
    }

    #[test]
    fn index_builds_turns_from_a_fixture_ledger_with_two_closed_and_one_open() {
        let dir = tempfile::tempdir().unwrap();
        let (log, t1, t2, t3) = two_closed_one_open(dir.path());
        let index = TurnIndex::from_log(&log);
        assert_eq!(index.turns.len(), 3);
        assert_eq!(index.turns[0].id, t1);
        assert!(index.turns[0].closed);
        assert_eq!(index.turns[1].id, t2);
        assert!(index.turns[1].closed);
        assert_eq!(index.turns[2].id, t3);
        assert!(
            !index.turns[2].closed,
            "the last turn has no final answer yet"
        );
    }

    #[test]
    fn a_turn_is_never_split_across_the_index() {
        let dir = tempfile::tempdir().unwrap();
        let (log, _, t2, _) = two_closed_one_open(dir.path());
        let index = TurnIndex::from_log(&log);
        let turn2 = index.turn_by_id(&t2).unwrap();
        assert_eq!(turn2.steps.len(), 2);
        assert_eq!(
            turn2.evidence,
            vec!["call-1".to_string(), "card-1".to_string()]
        );
        assert_eq!(turn2.presentations.len(), 1);
    }

    #[test]
    fn full_record_keeps_the_call_pattern_with_digested_results() {
        let dir = tempfile::tempdir().unwrap();
        let (log, _, t2, _) = two_closed_one_open(dir.path());
        let index = TurnIndex::from_log(&log);
        let turn2 = index.turn_by_id(&t2).unwrap();
        let record = turn2.full_record();
        // directive, search call, digested result, card call, ack, narration
        let roles: Vec<R> = record.iter().map(|m| m.role).collect();
        assert_eq!(
            roles,
            vec![
                R::User,
                R::Assistant,
                R::User,
                R::Assistant,
                R::User,
                R::Assistant
            ]
        );
        let search_result = match &record[2].content[0] {
            CB::ToolResult { content, .. } => content.clone(),
            other => panic!("expected a tool result, got {other:?}"),
        };
        assert!(
            search_result.contains("[evidence:call-1"),
            "{search_result}"
        );
        assert!(search_result.contains("https://example.com/a"));
        assert!(
            !search_result.contains("Rust 2.0\",\"url"),
            "raw JSON must be digested, not replayed: {search_result}"
        );
        assert!(matches!(
            &record[3].content[0],
            CB::ToolUse { name, .. } if name == "emit_research_card"
        ));
        assert!(matches!(
            &record[4].content[0],
            CB::ToolResult { tool_use_id, content, .. }
                if tool_use_id == "card-1" && content.contains("\"ok\":true")
        ));
        assert!(
            !record
                .iter()
                .any(|m| m.content.iter().any(|b| matches!(b, CB::Thinking { .. })))
        );
        assert_eq!(record[5].text_content(), "Here is what I found.");
    }

    #[test]
    fn full_record_drops_a_fence_that_duplicates_an_emitted_card() {
        let payload = serde_json::json!({"label": "Temperature", "value": 29.1, "unit": "C"});
        let record = PresentationRecord {
            turn_id: "t".into(),
            source: PresentationSource::ToolCall {
                tool_use_id: "card-9".into(),
            },
            semantic_type: "metric".into(),
            skill_id: "core".into(),
            skill_version: "1.0.0".into(),
            schema_version: 2,
            payload: crate::types::canonicalize_json(&payload),
            payload_digest: crate::types::payload_digest(&payload),
            derived_from: vec![],
            title: "Temperature".into(),
            identity_digest: String::new(),
        };
        let narration = format!(
            "It is 29.1°C.\n```vak\n{}\n```\nStay hydrated.",
            serde_json::json!({"semantic_type": "metric", "payload": payload})
        );
        let kept = strip_duplicate_fences(&narration, std::slice::from_ref(&record));
        assert_eq!(kept.trim(), "It is 29.1°C.\nStay hydrated.");
        let other = "```vak\n{\"semantic_type\":\"metric\",\"payload\":{\"label\":\"Wind\"}}\n```";
        assert_eq!(
            strip_duplicate_fences(other, std::slice::from_ref(&record)).trim(),
            other
        );
    }

    #[test]
    fn card_line_format() {
        let dir = tempfile::tempdir().unwrap();
        let (log, _, t2, _) = two_closed_one_open(dir.path());
        let index = TurnIndex::from_log(&log);
        let turn2 = index.turn_by_id(&t2).unwrap();
        let card = turn2.build_card("completed", "Here is what I found.".to_string(), &|s| {
            s.len() as u64 / 4
        });
        let line = card.line(2);
        assert!(line.starts_with("#2 asked:"));
        assert!(line.contains("did: search"));
        assert!(line.contains("research.synthesis"));
        assert!(line.contains("Rust news"));
        assert!(line.contains("ev:call-1"));
    }

    #[test]
    fn digest_per_shape() {
        let search_ev = Evidence {
            tool: "search".into(),
            input: serde_json::json!({}),
            content: r#"[{"title":"A","url":"https://a"},{"title":"B","url":"https://b"}]"#.into(),
            is_error: false,
        };
        let digest = evidence_digest("ev1", &search_ev);
        assert!(digest.contains("https://a"));
        assert!(digest.contains("https://b"));
        assert!(digest.ends_with("[evidence:ev1 \u{2014} 65 chars; call recall to expand]"));

        let bash_ev = Evidence {
            tool: "bash".into(),
            input: serde_json::json!({"command": "echo hi"}),
            content: "hi\nexit code: 0".into(),
            is_error: false,
        };
        let bash_digest_text = evidence_digest("ev2", &bash_ev);
        assert!(bash_digest_text.contains("exit code: 0"));

        let text_ev = Evidence {
            tool: "read".into(),
            input: serde_json::json!({}),
            content: "# Heading\n\nFirst paragraph text.\n\nMore.".into(),
            is_error: false,
        };
        let text_digest_text = evidence_digest("ev3", &text_ev);
        assert!(text_digest_text.contains("# Heading"));
        assert!(text_digest_text.contains("First paragraph text."));

        let err_ev = Evidence {
            tool: "bash".into(),
            input: serde_json::json!({}),
            content: "boom: permission denied".into(),
            is_error: true,
        };
        let err_digest = evidence_digest("ev4", &err_ev);
        assert!(err_digest.starts_with("boom: permission denied"));
    }

    #[test]
    fn bm25_returns_the_matching_turn_first() {
        let dir = tempfile::tempdir().unwrap();
        let (mut log, _, t2, _) = two_closed_one_open(dir.path());
        let card = TurnIndex::from_log(&log)
            .turn_by_id(&t2)
            .unwrap()
            .build_card("completed", "Here is what I found.".to_string(), &|s| {
                s.len() as u64 / 4
            });
        log.append_turn_card(TurnCardRecord {
            turn_id: t2.clone(),
            card,
        })
        .unwrap();
        let index = TurnIndex::from_log(&log);
        let hits = index.search("rust news");
        assert!(!hits.is_empty());
        assert_eq!(hits[0].0, t2);
    }

    #[test]
    fn control_messages_are_not_turns_or_steps() {
        let dir = tempfile::tempdir().unwrap();
        let mut log = open_log(dir.path());
        let t1 = log.append_message(user_text("do a thing")).unwrap().id;
        log.append_message(assistant_tool_call(
            "c1",
            "bash",
            serde_json::json!({"command": "ls"}),
        ))
        .unwrap();
        log.append_message(tool_result("c1", "ok")).unwrap();
        log.append_message(MessageRecord::control(
            vak_intent::control::ControlKind::StopHook,
            "[stop-hook]: keep going",
        ))
        .unwrap();
        log.append_message(assistant_text("done")).unwrap();
        let index = TurnIndex::from_log(&log);
        assert_eq!(index.turns.len(), 1);
        let turn = index.turn_by_id(&t1).unwrap();
        assert_eq!(turn.steps.len(), 1, "the nudge must not become a step");
        assert!(turn.closed);
    }

    #[test]
    fn intent_reading_attaches_to_its_turn() {
        let dir = tempfile::tempdir().unwrap();
        let mut log = open_log(dir.path());
        let t1 = log
            .append_message(user_text("find the weather"))
            .unwrap()
            .id;
        let reading = vak_intent::Reading::general();
        log.append_intent(IntentRecord {
            reading,
            engagement: vak_intent::Engagement::general(),
            provenance: vak_intent::Provenance::new(vak_intent::Tier::General, 1, Vec::new()),
            outcome: None,
            model_visible: None,
            commitment_id: None,
        })
        .unwrap();
        log.append_message(assistant_text("it is sunny")).unwrap();
        let index = TurnIndex::from_log(&log);
        let turn = index.turn_by_id(&t1).unwrap();
        let card = turn.build_card("completed", "it is sunny".to_string(), &|s| {
            s.len() as u64 / 4
        });
        assert_eq!(card.reading.act, "answer");
    }

    #[test]
    fn every_ledger_activity_entry_is_ignored_by_turn_structure() {
        let dir = tempfile::tempdir().unwrap();
        let mut log = open_log(dir.path());
        log.append_message(user_text("hi")).unwrap();
        log.append_activity(ActivityRecord {
            activity_id: "a1".into(),
            turn: None,
            kind: ActivityKind::Diagnostic,
            status: ActivityStatus::Succeeded,
            label: "note".into(),
            detail: None,
            data: Default::default(),
        })
        .unwrap();
        log.append_message(assistant_text("hello")).unwrap();
        let index = TurnIndex::from_log(&log);
        assert_eq!(index.turns.len(), 1);
    }

    #[test]
    fn packets_collect_compaction_entries() {
        let dir = tempfile::tempdir().unwrap();
        let mut log = open_log(dir.path());
        let t1 = log.append_message(user_text("first")).unwrap().id;
        log.append_message(assistant_text("first answer")).unwrap();
        log.append(Entry::new(
            log.tail_id().cloned(),
            EntryPayload::Compaction(CompactionEntry {
                summary: "summary text".into(),
                first_turn_id: t1.clone(),
                last_turn_id: t1,
                model: "fixture-model".into(),
                tokens_before: 100,
                reset_all: false,
            }),
        ))
        .unwrap();
        let index = TurnIndex::from_log(&log);
        assert_eq!(index.packets.len(), 1);
        assert_eq!(index.packets[0].summary, "summary text");
    }
}
