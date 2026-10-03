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
use crate::types::{Entry, EntryPayload, PresentationRecord, PresentationSource};

/// Subject words shared by disk and in-memory history retrieval. Function
/// words and temporal qualifiers do not establish topical relevance.
pub fn history_query_terms(query: &str) -> Vec<String> {
    let mut terms = std::collections::BTreeSet::new();
    // Keep full Unicode words, including combining marks. The intent kernel's
    // ASCII keyword helper is useful for English stop words, but cannot supply
    // multilingual retrieval terms.
    static WORDS: std::sync::OnceLock<Result<regex::Regex, regex::Error>> =
        std::sync::OnceLock::new();
    if let Ok(words) = WORDS.get_or_init(|| regex::Regex::new(r"[\p{L}\p{M}\p{N}]+")) {
        for word in words.find_iter(query).map(|word| word.as_str()) {
            if !word.is_ascii() {
                terms.insert(word.to_lowercase());
            } else {
                terms.extend(vak_intent::strand::keywords(word));
            }
        }
    }
    terms
        .into_iter()
        .filter(|word| {
            !matches!(
                word.as_str(),
                "what"
                    | "which"
                    | "who"
                    | "when"
                    | "where"
                    | "how"
                    | "current"
                    | "latest"
                    | "today"
                    | "yesterday"
                    | "tomorrow"
                    | "number"
                    | "numbers"
                    | "times"
                    | "reply"
                    | "define"
                    | "explain"
                    | "calculate"
                    | "compute"
                    | "describe"
                    | "summarize"
                    | "tell"
                    | "show"
                    | "give"
                    | "make"
                    | "create"
                    | "good"
                    | "detailed"
                    | "sentence"
                    | "sentences"
                    | "one"
                    | "word"
                    | "words"
            )
        })
        .collect()
}

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
    /// What this turn is linked to besides its words: the threads its
    /// strands belong to, the files its calls read or wrote, the commitments
    /// it served (docs/design/85-turn-graph.md, G1). Built from entries that
    /// name this turn in `Entry::at_turn`.
    pub links: Vec<TurnLink>,
    /// The turn's closing card, once written (`EntryPayload::TurnCard`).
    pub card: Option<TurnCard>,
    /// Canonical closing-card entry address for selected projection.
    pub closing_entry_id: Option<String>,
    /// `false` only for the last turn when the chain's raw tail does not
    /// end in an assistant message without `tool_use` — a text-only draft
    /// is not enough once a control nudge or tool result follows it; only
    /// a fresh final answer with nothing after it closes the turn.
    pub closed: bool,
    /// `true` when a reset-with-handoff entry (docs/design/42) follows this
    /// turn on the chain and the turn had already finished: the turn is
    /// invisible to the model, so it is
    /// never planned, never packeted and never listed as covered — though
    /// it stays in the index for `recall({ turn })` and turn numbering.
    pub behind_reset: bool,
    /// Full presentation records parallel to `presentations`, kept alongside
    /// the id list so `full_record`/`build_card` never re-walk the ledger.
    presentation_records: Vec<PresentationRecord>,
    /// The whole result behind each of this turn's windowed tool results
    /// (`EntryPayload::EvidenceBody`), keyed by tool_use_id.
    evidence_bodies: HashMap<String, String>,
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
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
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
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct WorkingSetPlan {
    /// Chronological order (oldest first), one entry per closed turn that
    /// is `Full` or `Card`. A turn absent from this list and not covered by
    /// `packet_range` simply has no card yet — `derive_with_plan` treats
    /// that as already covered by an existing `Compaction` entry.
    pub per_turn: Vec<(String, Fidelity)>,
    /// Exact closing-record addresses when history is selectively retrieved.
    /// Some(empty) deliberately excludes all closed history. None is used by
    /// explicit historical/manual compaction projections.
    #[serde(default)]
    pub selected_records: Option<Vec<(String, String)>>,
    /// The contiguous, oldest-to-newest range of turns represented only by
    /// a packet, inclusive — `(first_turn_id, last_turn_id)`.
    pub packet_range: Option<(String, String)>,
    /// Turn ids promoted to `Full` by relevance (search, reading overlap,
    /// or anaphora) rather than by the recency fill.
    pub retrieved: Vec<String>,
    /// For each turn promoted by a link, the nodes it shares with the open
    /// turn (`thread:…`, `file:…`, `commitment:…`), so a transcript can say
    /// why the turn was in context (docs/design/85-turn-graph.md, G1).
    #[serde(default)]
    pub links: std::collections::BTreeMap<String, Vec<String>>,
    pub budget: u64,
    pub spent: u64,
}

/// Label of the audit activity that records the plan a request was built from.
pub const CONTEXT_PLAN_LABEL: &str = "context-plan";

/// The planner's rules version recorded with each plan. Bump it with a change
/// to how a plan is computed, so an old record is read for what it was.
pub const CONTEXT_PLAN_POLICY_VERSION: u32 = 3;

impl WorkingSetPlan {
    /// The plan as the `data` of its audit activity: the `Full` turns, the
    /// packet range, the budget accounting and the ledger leaf it was planned
    /// at. Every other closed turn is a `Card` by construction, so this is all
    /// a replay needs (invariant 1); it stays bounded by the `Full` turns, not
    /// by the length of the conversation.
    pub fn to_activity_data(
        &self,
        leaf: Option<&str>,
    ) -> std::collections::BTreeMap<String, String> {
        let full: Vec<&str> = self
            .per_turn
            .iter()
            .filter(|(_, f)| *f == Fidelity::Full)
            .map(|(id, _)| id.as_str())
            .collect();
        let mut data = std::collections::BTreeMap::new();
        data.insert("section".into(), CONTEXT_PLAN_LABEL.into());
        data.insert("policy".into(), CONTEXT_PLAN_POLICY_VERSION.to_string());
        data.insert("leaf".into(), leaf.unwrap_or_default().to_string());
        data.insert(
            "full".into(),
            serde_json::to_string(&full).unwrap_or_default(),
        );
        data.insert(
            "retrieved".into(),
            serde_json::to_string(&self.retrieved).unwrap_or_default(),
        );
        if !self.links.is_empty() {
            data.insert(
                "links".into(),
                serde_json::to_string(&self.links).unwrap_or_default(),
            );
        }
        if let Some((first, last)) = &self.packet_range {
            data.insert("packet_first".into(), first.clone());
            data.insert("packet_last".into(), last.clone());
        }
        data.insert("budget".into(), self.budget.to_string());
        data.insert("spent".into(), self.spent.to_string());
        data
    }

    /// The plan and the leaf it was planned at, from a recorded activity.
    pub fn from_activity_data(
        data: &std::collections::BTreeMap<String, String>,
    ) -> Option<(WorkingSetPlan, String)> {
        if data.get("section").map(String::as_str) != Some(CONTEXT_PLAN_LABEL) {
            return None;
        }
        let full: Vec<String> = serde_json::from_str(data.get("full")?).ok()?;
        let retrieved: Vec<String> = serde_json::from_str(data.get("retrieved")?).ok()?;
        let packet_range = data
            .get("packet_first")
            .zip(data.get("packet_last"))
            .map(|(first, last)| (first.clone(), last.clone()));
        let plan = WorkingSetPlan {
            per_turn: full.into_iter().map(|id| (id, Fidelity::Full)).collect(),
            selected_records: None,
            packet_range,
            retrieved,
            links: data
                .get("links")
                .and_then(|links| serde_json::from_str(links).ok())
                .unwrap_or_default(),
            budget: data.get("budget")?.parse().ok()?,
            spent: data.get("spent")?.parse().ok()?,
        };
        Some((plan, data.get("leaf")?.clone()))
    }
}

/// How a turn is linked to a node (docs/design/85-turn-graph.md §4.2).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum LinkKind {
    /// A strand of the turn opened this thread.
    OwnThread,
    /// A strand of the turn continues or corrects this thread (inferred by
    /// the resolver unless an explicit command set it).
    ContinuesThread,
    /// A call read this workspace file.
    ReadFile,
    /// A call wrote this workspace file.
    WroteFile,
    /// The turn served this commitment.
    ServesCommitment,
    /// The turn's own node (`turn:<its id>`), which a recall reaches.
    ThisTurn,
    /// A `recall` call in the turn reopened this turn, or a card or result
    /// of it: the model itself found the two related.
    RecalledTurn,
    /// The open turn's directive names this file. Only the planner makes
    /// these, at plan time, before any call of the turn has run.
    NamedFile,
}

/// One link from a turn to a node, keyed `thread:…`, `file:…` or
/// `commitment:…`.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize)]
pub struct TurnLink {
    pub node: String,
    pub kind: LinkKind,
}

impl TurnLink {
    fn from_intent(record: &crate::types::IntentRecord) -> Vec<TurnLink> {
        let mut links = Vec::new();
        for strand in &record.strands {
            let continues = strand.lineage.continued_thread().is_some();
            links.push(TurnLink {
                node: format!("thread:{}", strand.thread_id),
                kind: if continues {
                    LinkKind::ContinuesThread
                } else {
                    LinkKind::OwnThread
                },
            });
            links.extend(
                strand
                    .lineage
                    .continued_threads()
                    .into_iter()
                    .filter(|thread| *thread != strand.thread_id)
                    .map(|thread| TurnLink {
                        node: format!("thread:{thread}"),
                        kind: LinkKind::ContinuesThread,
                    }),
            );
        }
        links.extend(
            record
                .commitment_id
                .iter()
                .chain(record.strand_commitments.values())
                .map(|commitment| TurnLink {
                    node: format!("commitment:{commitment}"),
                    kind: LinkKind::ServesCommitment,
                }),
        );
        links
    }

    fn from_effect(effect: &crate::types::CallEffect) -> Option<TurnLink> {
        let (path, kind) = match effect {
            crate::types::CallEffect::FileRead { path, .. } => (path, LinkKind::ReadFile),
            crate::types::CallEffect::FileWrite { path, .. } => (path, LinkKind::WroteFile),
            crate::types::CallEffect::Mcp(_) => return None,
        };
        let path = path.trim_start_matches("./");
        Some(TurnLink {
            node: format!("file:{path}"),
            kind,
        })
    }
}

/// Links each turn that called `recall` to the turn it reopened, whether by
/// number, turn id, presentation id or evidence id, and gives the reopened
/// turn its own `turn:` node so the two share it. The loop answers `recall`
/// itself, so its name is the runtime's own primitive, not a tool table.
fn link_recalls(turns: &mut [Turn]) {
    let mut recalled: Vec<(usize, String)> = Vec::new();
    for (position, turn) in turns.iter().enumerate() {
        for step in &turn.steps {
            for block in &step.assistant.content {
                let ContentBlock::ToolUse { name, input, .. } = block else {
                    continue;
                };
                if name != "recall" {
                    continue;
                }
                let target = if let Some(n) = input.get("turn").and_then(Value::as_u64) {
                    usize::try_from(n)
                        .ok()
                        .and_then(|n| n.checked_sub(1))
                        .and_then(|i| turns.get(i))
                } else if let Some(id) = input.get("turn_id").and_then(Value::as_str) {
                    turns.iter().find(|t| t.id == id)
                } else if let Some(id) = input.get("presentation").and_then(Value::as_str) {
                    turns
                        .iter()
                        .find(|t| t.presentations.iter().any(|p| p == id))
                } else if let Some(id) = input.get("id").and_then(Value::as_str) {
                    turns
                        .iter()
                        .find(|t| t.evidence.iter().chain(&t.presentations).any(|e| e == id))
                } else {
                    None
                };
                if let Some(target) = target.filter(|target| target.id != turn.id) {
                    recalled.push((position, target.id.clone()));
                }
            }
        }
    }
    for (position, target) in recalled {
        let node = format!("turn:{target}");
        if let Some(target_turn) = turns.iter_mut().find(|t| t.id == target) {
            add_links(
                &mut target_turn.links,
                vec![TurnLink {
                    node: node.clone(),
                    kind: LinkKind::ThisTurn,
                }],
            );
        }
        add_links(
            &mut turns[position].links,
            vec![TurnLink {
                node,
                kind: LinkKind::RecalledTurn,
            }],
        );
    }
}

/// Adds links not already present: a link is a set member, so a file read
/// ten times in one turn is one link.
fn add_links(into: &mut Vec<TurnLink>, links: Vec<TurnLink>) {
    for link in links {
        if !into.contains(&link) {
            into.push(link);
        }
    }
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
        Self::from_entries(log.chain_to_root())
    }

    /// Reconstruct only an already-authorized, chronologically ordered range.
    /// The caller owns scope, branch membership and byte/count limits.
    pub fn from_entries<'a>(entries: impl IntoIterator<Item = &'a Entry>) -> TurnIndex {
        let chain: Vec<&Entry> = entries.into_iter().collect();
        let mut turns: Vec<Turn> = Vec::new();
        let mut packets: Vec<Packet> = Vec::new();
        let mut pending_readings: HashMap<String, ReadingKey> = HashMap::new();
        let mut pending_links: HashMap<String, Vec<TurnLink>> = HashMap::new();

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
                                    closing_entry_id: None,
                                    closed: true,
                                    behind_reset: false,
                                    presentation_records: Vec::new(),
                                    evidence_bodies: HashMap::new(),
                                    reading: pending_readings.remove(&entry.id),
                                    links: pending_links.remove(&entry.id).unwrap_or_default(),
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
                    // An intent names its turn by id (`Entry::at_turn`): the
                    // admission record is written before the directive it
                    // serves, so it waits for that directive; a revision
                    // belongs to the turn it was written in.
                    let Some(turn_id) = &entry.at_turn else {
                        continue;
                    };
                    let reading = ReadingKey::from_record(record);
                    let links = TurnLink::from_intent(record);
                    match turns.iter_mut().rev().find(|turn| &turn.id == turn_id) {
                        Some(turn) => {
                            turn.reading = Some(reading);
                            add_links(&mut turn.links, links);
                        }
                        None => {
                            pending_readings.insert(turn_id.clone(), reading);
                            add_links(pending_links.entry(turn_id.clone()).or_default(), links);
                        }
                    }
                }
                EntryPayload::CallEffect(record) => {
                    let (Some(turn_id), Some(link)) =
                        (&entry.at_turn, TurnLink::from_effect(&record.effect))
                    else {
                        continue;
                    };
                    match turns.iter_mut().rev().find(|turn| &turn.id == turn_id) {
                        Some(turn) => add_links(&mut turn.links, vec![link]),
                        None => add_links(
                            pending_links.entry(turn_id.clone()).or_default(),
                            vec![link],
                        ),
                    }
                }
                EntryPayload::EvidenceBody(body) => {
                    if let Some(turn) = turns.last_mut() {
                        turn.evidence_bodies
                            .insert(body.tool_use_id.clone(), body.content.clone());
                    }
                }
                EntryPayload::Compaction(c) if c.reset_all => {
                    // The turn still being worked survives its own reset: the
                    // handoff summary stands in for what it did so far, and
                    // its directive and every later step stay visible.
                    // Hiding it would leave the model with a summary and no
                    // task, and every step after the reset invisible.
                    let open = c.keeps_open_turn
                        && turns.last().is_some_and(|turn| {
                            !turn.raw_tail.last().is_some_and(|message| {
                                message.role == Role::Assistant
                                    && !message
                                        .content
                                        .iter()
                                        .any(|block| matches!(block, ContentBlock::ToolUse { .. }))
                            })
                        });
                    let keep = usize::from(open);
                    let settled = turns.len() - keep;
                    for turn in &mut turns[..settled] {
                        turn.behind_reset = true;
                    }
                    if open && let Some(turn) = turns.last_mut() {
                        turn.raw_tail.clear();
                        turn.behind_reset = false;
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
        // way or another. The last turn is open exactly when its RAW
        // ledger tail — not merely whether a text-only answer was ever
        // produced — does not end in an assistant message without
        // `tool_use`. `final_answer` records the latest such message the
        // turn has seen, but a gate can append a control nudge (or a tool
        // result) after it while asking for a redo; until a fresh final
        // answer follows, the turn is still open, or the nudge itself
        // would be dropped from the next request the moment `full_record`
        // replaced `current_verbatim` (a text-only draft the ledger holds
        // is not the same thing as an accepted answer).
        if let Some(last) = turns.last_mut() {
            let last_message = last.raw_tail.last().unwrap_or(&last.directive);
            last.closed = last_message.role == Role::Assistant
                && !last_message
                    .content
                    .iter()
                    .any(|b| matches!(b, ContentBlock::ToolUse { .. }));
        }

        let by_id: HashMap<String, usize> = turns
            .iter()
            .enumerate()
            .map(|(i, t)| (t.id.clone(), i))
            .collect();
        for entry in &chain {
            match &entry.payload {
                EntryPayload::Presentation(record) => {
                    if let Some(&idx) = by_id.get(&record.turn_id) {
                        turns[idx].presentations.push(entry.id.clone());
                        turns[idx].presentation_records.push(record.clone());
                    }
                }
                EntryPayload::TurnCard(record) => {
                    if let Some(&idx) = by_id.get(&record.turn_id) {
                        turns[idx].card = Some(record.card.clone());
                        turns[idx].closing_entry_id = Some(entry.id.clone());
                    }
                }
                _ => {}
            }
        }

        link_recalls(&mut turns);
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

    /// Ranks closed turns by how much of the query's subject their card
    /// carries. A term counts by how rare it is across the conversation's own
    /// cards (inverse document frequency), so a word every card shares says
    /// nothing, and a turn must carry at least half of the query's subject
    /// terms to be a candidate at all: one coincidental word out of several is
    /// not topical relevance. Highest score first; ties break by turn id.
    pub fn search(&self, query: &str) -> Vec<(String, f64)> {
        let terms = history_query_terms(query);
        let phrase = crate::search::normalize_impl(query);
        if terms.is_empty() || phrase.is_empty() {
            return Vec::new();
        }
        let documents: Vec<(&Turn, String, HashSet<String>)> = self
            .turns
            .iter()
            .filter_map(|turn| {
                let text = turn.card.as_ref()?.index_text();
                let words: HashSet<String> = history_query_terms(&text).into_iter().collect();
                Some((turn, text, words))
            })
            .collect();
        let total = documents.len() as f64;
        let idf = |term: &String| {
            let containing = documents
                .iter()
                .filter(|(_, _, words)| words.contains(term))
                .count() as f64;
            (1.0 + (total - containing + 0.5) / (containing + 0.5)).ln()
        };
        let mut scored: Vec<(String, f64)> = documents
            .iter()
            .filter_map(|(turn, text, words)| {
                let matched: Vec<&String> =
                    terms.iter().filter(|term| words.contains(*term)).collect();
                if matched.is_empty() || matched.len() * 2 < terms.len() {
                    return None;
                }
                let mut score: f64 = matched.iter().map(|term| idf(term)).sum();
                if crate::search::normalize_impl(text).contains(&phrase) {
                    score += 1.0;
                }
                (score > 0.0).then(|| (turn.id.clone(), score))
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

    /// A result's whole content: its evidence body when the request carried
    /// only a window of it, else the recorded result itself.
    fn tool_result(&self, id: &str) -> Option<(&str, bool)> {
        self.steps
            .iter()
            .flat_map(|step| step.results.iter())
            .find_map(|(result_id, content, is_error)| {
                (result_id == id).then(|| {
                    let content = self.evidence_bodies.get(id).unwrap_or(content);
                    (content.as_str(), *is_error)
                })
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
                PresentationSource::Fence { .. } | PresentationSource::Delegated { .. } => None,
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
                PresentationSource::Fence { .. } | PresentationSource::Delegated { .. } => None,
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
                    // Process, not information ("Let me search..."): on a small
                    // model it is the prose pattern the next turn imitates.
                    ContentBlock::Thinking { .. } | ContentBlock::Text { .. } => false,
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
        // Account for call arguments and provider blocks as well as prose.
        // text_content() intentionally omits those and is not a wire cost.
        let full_text: String = self
            .full_record()
            .iter()
            .flat_map(|message| {
                message.content.iter().map(|block| match block {
                    ContentBlock::Text { text } => text.clone(),
                    ContentBlock::ToolUse { input, .. } => input.to_string(),
                    ContentBlock::ToolResult { content, .. } => content.clone(),
                    ContentBlock::Provider { raw, .. } => raw.to_string(),
                    _ => String::new(),
                })
            })
            .collect();
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
        // Rendered card cost is unrelated to the richer search projection.
        // Planning remeasures with the actual ordinal and current profile.
        card.tokens_card = estimate_tokens(&card.line(1));
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

/// A tool result as a summariser's transcript shows it (compaction,
/// handoff): its schema-driven digest, tagged with the evidence id `recall`
/// reopens, when the call that produced it is in `messages`; else the result
/// itself. Never a character cut.
pub fn transcript_result(
    messages: &[Message],
    tool_use_id: &str,
    content: &str,
    is_error: bool,
) -> String {
    let call = messages
        .iter()
        .flat_map(|message| message.content.iter())
        .find_map(|block| match block {
            ContentBlock::ToolUse { id, name, input } if id == tool_use_id => Some((name, input)),
            _ => None,
        });
    match call {
        Some((tool, input)) => evidence_digest(
            tool_use_id,
            &Evidence {
                tool: tool.clone(),
                input: input.clone(),
                content: content.to_string(),
                is_error,
            },
        ),
        None => content.to_string(),
    }
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

/// Command output as its exit line and first and last ten lines, saying how
/// many lines between them it leaves out.
pub fn bash_digest(content: &str) -> String {
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
        out.push_str(&format!("\n[{} lines omitted]\n", lines.len() - 20));
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
    truncate_words(first_sentence(trimmed), max_words)
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
    /// The engagement's context profile (`minimal` | `recall` | `working`
    /// | `full`), which steers the working-set planner
    /// (docs/design/47-commitment-kernel.md). Empty means `recall`.
    #[serde(default)]
    pub context: String,
}

impl ReadingKey {
    /// From a full intent entry: the reading plus the context profile.
    pub fn from_record(record: &crate::types::IntentRecord) -> Self {
        let mut key = Self::from_reading(&record.reading);
        key.context = record.engagement.posture.context.as_str().to_string();
        key
    }

    /// Whether the planner should keep history to the bare minimum.
    pub fn is_minimal(&self) -> bool {
        self.context == "minimal"
    }

    /// Whether the turn wants the workspace delta since the session began.
    pub fn wants_workspace_delta(&self) -> bool {
        matches!(self.context.as_str(), "working" | "full")
    }

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
            context: String::new(),
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
    /// Addressed history uses a stable id, not an ordinal that changes with
    /// the candidate set. This is also the exact string the planner costs.
    pub fn addressed_message(&self) -> String {
        let line = self.line(0);
        let description = line.strip_prefix("#0 ").unwrap_or(&line);
        format!(
            "<turns>\n[turn_id:{}] {description}\n</turns>",
            self.turn_id
        )
    }

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
            // No card was emitted, but the calls it made are still reachable:
            // a card line names every result a later turn can recall.
            None => {
                let evidence: Vec<&str> = self
                    .did
                    .iter()
                    .map(|trace| trace.evidence_id.as_str())
                    .collect();
                let narration = truncate_words(&self.answered.narration, 12);
                if evidence.is_empty() {
                    format!("\"{narration}\"")
                } else {
                    format!("\"{narration}\" [ev:{}]", evidence.join(","))
                }
            }
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
            space: None,
            run: None,
            cause: None,
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
    fn history_query_terms_keep_subjects_and_unicode_combining_marks() {
        assert_eq!(
            history_query_terms("what is the current weather in Noida"),
            vec!["noida", "weather"]
        );
        assert_eq!(
            history_query_terms("दिल्ली का मौसम"),
            vec!["का", "दिल्ली", "मौसम"]
        );
        assert_eq!(history_query_terms("北京 天气"), vec!["北京", "天气"]);
        assert_eq!(history_query_terms("CAFÉ latest"), vec!["café"]);
        assert!(history_query_terms("what is the current").is_empty());
        assert!(history_query_terms("What is 17 times 23? Reply with just the number.").is_empty());
        assert_eq!(
            history_query_terms("Define photosynthesis in one sentence."),
            vec!["photosynthesis"]
        );
    }

    #[test]
    fn in_memory_history_matches_unicode_subject_words() {
        let dir = tempfile::tempdir().unwrap();
        let (mut log, _, _, _) = two_closed_one_open(dir.path());
        log.append_message(user_text("दिल्ली मौसम 北京 天气 café"))
            .unwrap();
        log.append_message(assistant_text("historical observation"))
            .unwrap();
        let mut index = TurnIndex::from_log(&log);
        index.ensure_cards(&|_| 1);
        let wanted = index.turns.last().unwrap().id.clone();
        for query in ["दिल्ली मौसम", "北京 天气", "CAFÉ"] {
            assert_eq!(index.search(query)[0].0, wanted, "{query}");
        }
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
    fn current_turn_index_matches_the_full_projection_without_older_turns() {
        let dir = tempfile::tempdir().unwrap();
        let (mut log, _, _, t3) = two_closed_one_open(dir.path());
        let check = |log: &SessionLog| {
            let full = TurnIndex::from_log(log);
            let current = log.current_turn_index();
            assert_eq!(current.turns.len(), 1);
            let selected = &current.turns[0];
            let expected = full.turns.last().unwrap();
            assert_eq!(selected.id, expected.id);
            assert_eq!(selected.closed, expected.closed);
            assert_eq!(selected.reading, expected.reading);
            assert_eq!(selected.behind_reset, expected.behind_reset);
            assert_eq!(
                log.latest_message().map(Message::text_content),
                log.message_chain()
                    .last()
                    .map(|(_, message)| message.text_content())
            );
            let expected_open = if expected.closed || expected.behind_reset {
                vec![]
            } else {
                expected.current_verbatim()
            };
            assert_eq!(log.open_turn_verbatim(), expected_open);
            assert_eq!(selected.current_verbatim(), expected.current_verbatim());
            assert_eq!(selected.full_record(), expected.full_record());
            assert_eq!(
                selected.card.as_ref().map(|card| card.line(1)),
                expected.card.as_ref().map(|card| card.line(1))
            );
        };
        check(&log);
        log.append_message(tool_result("call-2", "changelog evidence"))
            .unwrap();
        log.append_message(assistant_text("The changelog confirms the release."))
            .unwrap();
        check(&log);
        let current = log.current_turn_index();
        let card = current.turns[0].build_card("completed", "release confirmed".into(), &|_| 1);
        log.append_turn_card(crate::types::TurnCardRecord {
            turn_id: t3.clone(),
            card,
        })
        .unwrap();
        check(&log);
        log.branch_at(&t3).unwrap();
        check(&log);
        log.append_handoff_reset("selected handoff".into(), 1, false)
            .unwrap();
        check(&log);
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

    /// A gate that redoes a text-only draft appends a control nudge AFTER
    /// it without any further assistant reply yet: the turn must stay open
    /// (never rendered as a closed `full_record`, which would drop the
    /// nudge entirely) and `current_verbatim` must end with the nudge, so
    /// the next request's last message is a real user-role message rather
    /// than the assistant's own draft (invariant: current Claude models
    /// reject a request with no trailing user turn).
    #[test]
    fn a_draft_followed_by_a_control_nudge_with_no_reply_yet_stays_open() {
        let dir = tempfile::tempdir().unwrap();
        let mut log = open_log(dir.path());
        let t1 = log.append_message(user_text("do a thing")).unwrap().id;
        log.append_message(assistant_text("draft answer")).unwrap();
        log.append_message(MessageRecord::control(
            vak_intent::control::ControlKind::StopHook,
            "[stop-hook]: keep going",
        ))
        .unwrap();
        let index = TurnIndex::from_log(&log);
        assert_eq!(index.turns.len(), 1);
        let turn = index.turn_by_id(&t1).unwrap();
        assert!(
            !turn.closed,
            "a nudge with no reply after it must keep the turn open"
        );
        let verbatim = turn.current_verbatim();
        let last = verbatim.last().expect("at least the directive");
        assert_eq!(
            last.text_content(),
            "[stop-hook]: keep going",
            "the nudge must be the last message so it reaches the model"
        );
        assert_eq!(last.role, R::User);
    }

    /// The same shape, but a tool result (rather than a nudge) is the last
    /// thing appended after the draft: also open, for the same reason.
    #[test]
    fn a_draft_followed_by_a_tool_result_with_no_reply_yet_stays_open() {
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
        let index = TurnIndex::from_log(&log);
        let turn = index.turn_by_id(&t1).unwrap();
        assert!(!turn.closed, "a fresh tool result must keep the turn open");
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
            strands: Vec::new(),
            strand_commitments: Default::default(),
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
    fn admission_before_directive_does_not_retag_the_previous_turn() {
        let dir = tempfile::tempdir().unwrap();
        let mut log = open_log(dir.path());
        log.append_message(user_text("old spreadsheet")).unwrap();
        log.append_message(assistant_text("created")).unwrap();
        log.begin_turn(&uuid::Uuid::now_v7().to_string());
        log.append_intent(IntentRecord {
            reading: vak_intent::Reading::general(),
            engagement: vak_intent::Engagement::general(),
            provenance: vak_intent::Provenance::new(vak_intent::Tier::General, 1, Vec::new()),
            outcome: None,
            model_visible: None,
            commitment_id: None,
            strands: Vec::new(),
            strand_commitments: Default::default(),
        })
        .unwrap();
        log.append_message(user_text("new weather")).unwrap();
        log.append_message(assistant_text("sunny")).unwrap();
        let index = TurnIndex::from_log(&log);
        assert!(index.turns[0].reading.is_none());
        assert_eq!(index.turns[1].reading.as_ref().unwrap().act, "answer");
    }

    fn general_intent() -> IntentRecord {
        IntentRecord {
            reading: vak_intent::Reading::general(),
            engagement: vak_intent::Engagement::general(),
            provenance: vak_intent::Provenance::new(vak_intent::Tier::General, 1, Vec::new()),
            outcome: None,
            model_visible: None,
            commitment_id: None,
            strands: Vec::new(),
            strand_commitments: Default::default(),
        }
    }

    #[test]
    fn one_turn_one_id() {
        let dir = tempfile::tempdir().unwrap();
        let mut log = open_log(dir.path());
        let turn_id = uuid::Uuid::now_v7().to_string();
        log.begin_turn(&turn_id);
        let intent = log.append_intent(general_intent()).unwrap();
        let directive = log.append_message(user_text("weather")).unwrap();
        assert_eq!(directive.id, turn_id, "the directive takes the reserved id");
        assert_eq!(intent.at_turn.as_deref(), Some(turn_id.as_str()));
        assert_eq!(directive.at_turn.as_deref(), Some(turn_id.as_str()));
        let index = TurnIndex::from_log(&log);
        assert_eq!(index.turns[0].id, turn_id);
        assert!(index.turns[0].reading.is_some());
    }

    #[test]
    fn every_turn_record_names_its_turn() {
        let dir = tempfile::tempdir().unwrap();
        let mut log = open_log(dir.path());
        let first = log.append_message(user_text("one")).unwrap();
        log.append_message(assistant_text("a")).unwrap();
        let second_id = uuid::Uuid::now_v7().to_string();
        log.begin_turn(&second_id);
        log.append_intent(general_intent()).unwrap();
        log.append_message(user_text("two")).unwrap();
        log.append_message(assistant_text("b")).unwrap();
        let steering = log.append_message(user_text("and three")).unwrap();
        log.append_message(assistant_text("c")).unwrap();
        let turns: Vec<Option<String>> = log
            .chain_to_root()
            .into_iter()
            .filter(|entry| !matches!(entry.payload, EntryPayload::Header(_)))
            .map(|entry| entry.at_turn.clone())
            .collect();
        let expected = [
            &first.id,
            &first.id,
            &second_id,
            &second_id,
            &second_id,
            &steering.id,
            &steering.id,
        ];
        assert_eq!(
            turns,
            expected
                .iter()
                .map(|id| Some((*id).clone()))
                .collect::<Vec<_>>()
        );
    }

    #[test]
    fn a_recall_links_the_two_turns() {
        let dir = tempfile::tempdir().unwrap();
        let mut log = open_log(dir.path());
        let first = log.append_message(user_text("sensex today")).unwrap();
        log.append_message(assistant_text("up 1%")).unwrap();
        let second = log
            .append_message(user_text("compare with before"))
            .unwrap();
        log.append_message(MessageRecord {
            message: M::assistant(vec![ContentBlock::ToolUse {
                id: "call-r".into(),
                name: "recall".into(),
                input: serde_json::json!({"turn_id": first.id}),
            }]),
            meta: None,
        })
        .unwrap();
        log.append_message(MessageRecord {
            message: M {
                role: Role::User,
                content: vec![ContentBlock::tool_result("call-r", "up 1%")],
            },
            meta: None,
        })
        .unwrap();
        log.append_message(assistant_text("same as before"))
            .unwrap();
        let index = TurnIndex::from_log(&log);
        let node = format!("turn:{}", first.id);
        let by_id = |id: &str| index.turns.iter().find(|t| t.id == id).unwrap();
        assert!(by_id(&first.id).links.contains(&TurnLink {
            node: node.clone(),
            kind: LinkKind::ThisTurn
        }));
        assert!(by_id(&second.id).links.contains(&TurnLink {
            node,
            kind: LinkKind::RecalledTurn
        }));
    }

    #[test]
    fn a_reserved_turn_id_already_used_is_refused() {
        let dir = tempfile::tempdir().unwrap();
        let mut log = open_log(dir.path());
        let first = log.append_message(user_text("one")).unwrap();
        log.begin_turn(&first.id);
        assert!(log.append_message(user_text("two")).is_err());
    }

    #[test]
    fn a_reopened_ledger_keeps_its_current_turn() {
        let dir = tempfile::tempdir().unwrap();
        let path = {
            let mut log = open_log(dir.path());
            let directive = log.append_message(user_text("one")).unwrap();
            (log.path().to_path_buf(), directive.id)
        };
        let mut log = SessionLog::open(path.0).unwrap();
        let reply = log.append_message(assistant_text("a")).unwrap();
        assert_eq!(reply.at_turn, Some(path.1));
    }

    #[test]
    fn every_ledger_activity_entry_is_ignored_by_turn_structure() {
        let dir = tempfile::tempdir().unwrap();
        let mut log = open_log(dir.path());
        log.append_message(user_text("hi")).unwrap();
        log.append_activity(ActivityRecord {
            activity_id: "a1".into(),
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
                keeps_open_turn: false,
            }),
        ))
        .unwrap();
        let index = TurnIndex::from_log(&log);
        assert_eq!(index.packets.len(), 1);
        assert_eq!(index.packets[0].summary, "summary text");
    }
}
