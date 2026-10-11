//! `WorkingSetPlanner` (docs/design/68-context-engine.md §4, §10): keeps the
//! current turn verbatim and admits only the compact card of the turn it
//! follows. Historical records are reopened through `recall`.
//! Pure: no I/O, no locks, no network. The caller
//! (`vak-agent`'s turn loop) supplies the `CapacityProfile`, the
//! `TurnIndex`, and the incoming directive; `plan()` returns a `WorkingSetPlan`
//! that `SessionLog::derive_with_plan` turns into messages.

pub use vak_session::{Fidelity, WorkingSetPlan};
use vak_session::{LinkKind, ReadingKey, SessionLog, TurnIndex};

use crate::assemble::messages_chars;
use crate::capacity::CapacityProfile;

/// Inputs to one planning pass. `reading` is the current directive's own
/// reading, already resolved by the intent tier before planning runs.
/// `current_turn_tokens` is the OPEN turn's measured size so far (directive
/// plus any steps already taken this turn); it never itself appears in the
/// plan (the open turn is always verbatim), but it floors the reserve
/// subtracted from the budget for it.
pub struct PlanInput<'a> {
    pub profile: &'a CapacityProfile,
    pub index: &'a TurnIndex,
    pub directive: &'a str,
    pub reading: Option<&'a ReadingKey>,
    pub prefix_tokens: u64,
    pub tail_tokens: u64,
    pub current_turn_tokens: u64,
}

/// How much a link of each kind says two turns belong together
/// (docs/design/85-turn-graph.md §6.1): an observed link (a file a call
/// touched, a thread a strand opened) weighs more than one the resolver
/// inferred. Pinned by `link_weights_are_pinned`.
fn link_weight(kind: LinkKind) -> f64 {
    match kind {
        LinkKind::OwnThread | LinkKind::WroteFile | LinkKind::ThisTurn | LinkKind::RecalledTurn => {
            1.0
        }
        LinkKind::ReadFile | LinkKind::ServesCommitment | LinkKind::NamedFile => 0.8,
        LinkKind::ContinuesThread => 0.7,
    }
}

/// At most this many turns, the most recent, are reached through any one
/// node, so a node every turn touches cannot pull the whole history in.
const MAX_TURNS_PER_NODE: usize = 8;

/// How many shared-node steps a walk takes from the open turn: a turn that
/// shares a node with a turn that shares one with the open turn is reached
/// at the second step.
const MAX_LINK_STEPS: usize = 2;

/// What one more step costs: a turn reached at step two is worth at most
/// half of one reached directly.
const STEP_DECAY: f64 = 0.5;

/// The workspace files a directive names: tokens shaped like a path or a
/// file name with an extension (`notes/plan.md`, `` `src/parser.rs` ``,
/// `README.md`). URLs, e-mail addresses and numbers like `3.5` are not
/// files. Matched against recorded file nodes by full path or last segment.
fn named_files(directive: &str) -> Vec<String> {
    let mut names = Vec::new();
    for token in directive.split(|c: char| {
        c.is_whitespace()
            || matches!(
                c,
                '`' | '"' | '\'' | ',' | '(' | ')' | '[' | ']' | '<' | '>'
            )
    }) {
        let token = token
            .trim_end_matches(['.', ':', ';', '!', '?'])
            .trim_start_matches("./");
        if token.contains("://") || token.contains('@') || token.is_empty() {
            continue;
        }
        let Some((stem, extension)) = token.rsplit_once('.') else {
            continue;
        };
        let file = stem.rsplit('/').next().unwrap_or(stem);
        let is_name = !file.is_empty()
            && (1..=8).contains(&extension.len())
            && extension.chars().all(|c| c.is_ascii_alphanumeric())
            && extension.chars().any(|c| c.is_ascii_alphabetic());
        if is_name && !names.iter().any(|n| n == token) {
            names.push(token.to_string());
        }
    }
    names
}

/// One place a walk continues from: the links to follow, the value they
/// carry, the path so far, and the turn they belong to (`None` for the open
/// turn).
type Frontier<'a> = (&'a [vak_session::TurnLink], f64, Vec<String>, Option<usize>);

/// Each closed turn's link value to the open turn, and the nodes on the path
/// that reached it. One step: the sum over the nodes a turn shares with the
/// source of both ends' link weights times the node's inverse-frequency
/// discount, capped at the source's own value. A node most turns touch (a
/// manifest, a README) is discounted toward zero, the graph form of a common
/// word. At most `MAX_TURNS_PER_NODE` turns, the most recent, are reached
/// through one node, and only the best `MAX_TURNS_PER_NODE` turns of a step
/// go on to the next, so the walk stays bounded however dense the history.
fn link_values(
    closed: &[&vak_session::Turn],
    open: Option<&vak_session::Turn>,
    directive: &str,
) -> std::collections::HashMap<String, (f64, Vec<String>)> {
    let mut reached: std::collections::HashMap<usize, (f64, Vec<String>)> =
        std::collections::HashMap::new();
    let mut holders: std::collections::HashMap<&str, std::collections::BTreeMap<usize, f64>> =
        std::collections::HashMap::new();
    for (position, turn) in closed.iter().enumerate() {
        for link in &turn.links {
            let weight = holders
                .entry(link.node.as_str())
                .or_default()
                .entry(position)
                .or_insert(0.0);
            *weight = weight.max(link_weight(link.kind));
        }
    }
    // The open turn's own links, and the files its directive names: the
    // only file anchors there are at plan time, because the plan is fixed
    // before the turn's first call (invariant 36).
    let mut anchors: Vec<vak_session::TurnLink> =
        open.map(|turn| turn.links.clone()).unwrap_or_default();
    for name in named_files(directive) {
        for node in holders.keys() {
            let Some(path) = node.strip_prefix("file:") else {
                continue;
            };
            let named = path == name || path.ends_with(&format!("/{name}"));
            if named && !anchors.iter().any(|anchor| anchor.node == *node) {
                anchors.push(vak_session::TurnLink {
                    node: (*node).to_string(),
                    kind: LinkKind::NamedFile,
                });
            }
        }
    }
    if anchors.is_empty() {
        return std::collections::HashMap::new();
    }
    let total = closed.len() as f64;
    let mut frontier: Vec<Frontier> = vec![(anchors.as_slice(), 1.0, Vec::new(), None)];
    for _ in 0..MAX_LINK_STEPS {
        let mut step: std::collections::HashMap<usize, (f64, f64, Vec<String>)> =
            std::collections::HashMap::new();
        for (links, carried, path, source) in &frontier {
            for anchor in *links {
                let Some(turns) = holders.get(anchor.node.as_str()) else {
                    continue;
                };
                let discount = (1.0 + total / turns.len() as f64).ln() / (1.0 + total).ln();
                for (&position, &weight) in turns.iter().rev().take(MAX_TURNS_PER_NODE) {
                    if Some(position) == *source || reached.contains_key(&position) {
                        continue;
                    }
                    let entry = step
                        .entry(position)
                        .or_insert((0.0, *carried, path.clone()));
                    entry.0 += carried * link_weight(anchor.kind) * weight * discount;
                    entry.1 = entry.1.max(*carried);
                    if !entry.2.contains(&anchor.node) {
                        entry.2.push(anchor.node.clone());
                    }
                }
            }
        }
        let mut next: Vec<(usize, f64, Vec<String>)> = step
            .into_iter()
            .map(|(position, (value, cap, path))| (position, value.min(cap), path))
            .collect();
        next.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));
        for (position, value, path) in &next {
            reached.insert(*position, (*value, path.clone()));
        }
        frontier = next
            .into_iter()
            .take(MAX_TURNS_PER_NODE)
            .map(|(position, value, path)| {
                (
                    closed[position].links.as_slice(),
                    value * STEP_DECAY,
                    path,
                    Some(position),
                )
            })
            .collect();
    }
    reached
        .into_iter()
        .map(|(position, value)| (closed[position].id.clone(), value))
        .collect()
}

/// Builds the working-set plan for one request (§4, §10). Never splits a
/// turn: the current turn is verbatim; at most the compact card of the
/// turn it follows is included. Historical detail is on demand.
pub fn plan(input: PlanInput) -> WorkingSetPlan {
    let reserve = input
        .profile
        .current_turn_reserve(input.current_turn_tokens);
    let budget = input
        .profile
        .budget(input.prefix_tokens, input.tail_tokens, reserve);

    // Only closed turns with a written card can supply a compact referent.
    // The open turn is always sent verbatim by the caller; turns behind a
    // reset-with-handoff are invisible to the model.
    let closed: Vec<&vak_session::Turn> = input
        .index
        .turns
        .iter()
        .filter(|turn| turn.closed && turn.card.is_some() && !turn.behind_reset)
        .collect();

    let mut spent: u64 = 0;
    let retrieved: Vec<String> = Vec::new();

    let open = input.index.turns.last().filter(|turn| !turn.closed);
    // The open turn follows the last turn of the thread it continues, when
    // it continues one; otherwise the last turn.
    let continued: Vec<&str> = open
        .map(|turn| {
            turn.links
                .iter()
                .filter(|link| link.kind == LinkKind::ContinuesThread)
                .map(|link| link.node.as_str())
                .collect()
        })
        .unwrap_or_default();
    let preceding = closed
        .iter()
        .rev()
        .find(|turn| {
            turn.links
                .iter()
                .any(|link| continued.contains(&link.node.as_str()))
        })
        .or(closed.last())
        .map(|turn| turn.id.clone());
    // `ContextProfile::Minimal` suppresses link expansion. The preceding
    // turn's compact card is carried at any context level.
    let minimal = input
        .reading
        .is_some_and(vak_session::ReadingKey::is_minimal);
    let linked = if minimal {
        std::collections::HashMap::new()
    } else {
        link_values(&closed, open, input.directive)
    };
    // Whether a request refers back is not read from its words: the turn it
    // follows is always present as one compact card, and nothing older is.
    // The always-loaded `recall` tool searches older turns when the model
    // finds that the current request depends on them.
    let candidate_id = preceding.as_deref();
    let candidates: Vec<&vak_session::Turn> = closed
        .iter()
        .filter(|turn| candidate_id == Some(turn.id.as_str()))
        .copied()
        .collect();
    let mut per_turn: Vec<(String, Fidelity)> = Vec::new();
    for turn in candidates {
        let cost = turn.card.as_ref().map(|card| card.tokens_card).unwrap_or(0);
        if spent.saturating_add(cost) <= budget {
            spent += cost;
            per_turn.push((turn.id.clone(), Fidelity::Card));
        }
    }

    // Deliberately do not fill spare capacity with other history or build a
    // packet from unselected turns. Omitted turns remain in the ledger and
    // are searchable through `recall`.
    let packet_range = None;

    // Render in chronological order (oldest first), matching `index.turns`.
    let order: std::collections::HashMap<&str, usize> = closed
        .iter()
        .enumerate()
        .map(|(i, t)| (t.id.as_str(), i))
        .collect();
    per_turn.sort_by_key(|(id, _)| order.get(id.as_str()).copied().unwrap_or(usize::MAX));

    let links = per_turn
        .iter()
        .filter_map(|(id, _)| linked.get(id).map(|(_, nodes)| (id.clone(), nodes.clone())))
        .collect();
    WorkingSetPlan {
        selected_records: None,
        per_turn,
        packet_range,
        retrieved,
        links,
        budget,
        spent,
    }
}

/// Plan only scoped, indexed candidates. Unselected history remains on disk;
/// it contributes neither arbitrary cards nor automatic compaction packets.
/// A candidate too large for Full can ride as its bounded discovery card.
pub fn plan_selected(input: PlanInput) -> WorkingSetPlan {
    let reserve = input
        .profile
        .current_turn_reserve(input.current_turn_tokens);
    let budget = input
        .profile
        .budget(input.prefix_tokens, input.tail_tokens, reserve);
    let mut plan = WorkingSetPlan {
        budget,
        selected_records: Some(vec![]),
        ..Default::default()
    };
    for turn in &input.index.turns {
        if !turn.closed || turn.behind_reset {
            continue;
        }
        let (Some(card), Some(closing)) = (&turn.card, &turn.closing_entry_id) else {
            continue;
        };
        let full = input
            .profile
            .estimate_tokens(messages_chars(&turn.full_record()));
        let compact = input
            .profile
            .estimate_tokens(card.addressed_message().len() as u64);
        let (fidelity, cost) = if plan.spent.saturating_add(full) <= budget {
            (Fidelity::Full, full)
        } else if plan.spent.saturating_add(compact) <= budget {
            (Fidelity::Card, compact)
        } else {
            continue;
        };
        plan.spent = plan.spent.saturating_add(cost);
        plan.per_turn.push((turn.id.clone(), fidelity));
        if let Some(sources) = plan.selected_records.as_mut() {
            sources.push((turn.id.clone(), closing.clone()));
        }
        plan.retrieved.push(turn.id.clone());
    }
    plan
}

/// Plans one request against the ledger as it stands: builds the
/// `TurnIndex`, gives every closed turn a provisional card so it can be
/// costed, reads the current directive and its resolved reading, sizes the
/// open turn's reserve from what will actually be sent
/// (`open_turn_verbatim`, reset-aware), and delegates to [`plan`]. The one
/// entry point every caller — the agent loop per step, `/compact`, the
/// eval gate — plans through, so they can never disagree on what a plan
/// is computed from.
pub fn plan_for_session(
    log: &SessionLog,
    profile: &CapacityProfile,
    prefix_tokens: u64,
    tail_tokens: u64,
) -> WorkingSetPlan {
    let mut index = TurnIndex::from_log(log);
    index.ensure_cards(&|text| profile.estimate_tokens(text.len() as u64));
    // Stored costs were measured when the card was written, possibly with
    // another model/profile. Older cards also counted text_content() alone,
    // omitting tool-call arguments, and costed index_text() as the card even
    // though the wire carries line(). Cost the actual two projections now.
    // This changes no ledger data and sends no new model-visible content.
    for (number, turn) in index.turns.iter_mut().enumerate() {
        let full_cost = profile.estimate_tokens(messages_chars(&turn.full_record()));
        if let Some(card) = &mut turn.card {
            card.tokens_full = full_cost;
            card.tokens_card = profile.estimate_tokens(card.line(number + 1).len() as u64);
        }
    }
    let directive = index
        .turns
        .last()
        .map(|turn| turn.directive.text_content())
        .unwrap_or_default();
    let current_turn_tokens = profile.estimate_tokens(messages_chars(&log.open_turn_verbatim()));
    let reading = log.latest_reading();
    plan(PlanInput {
        profile,
        index: &index,
        directive: &directive,
        reading: reading.as_ref(),
        prefix_tokens,
        tail_tokens,
        current_turn_tokens,
    })
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::*;
    use crate::capacity::{CacheBehaviour, CapacityProfile, Horizon, ProbeProvenance};
    use vak_llm::Message as M;
    use vak_session::types::{FrozenContract, MessageRecord, SessionHeader, TurnCardRecord};
    use vak_session::{Answer, SessionLog, SessionPath, TraceLine, TurnCard};

    fn profile(horizon_tokens: u64) -> CapacityProfile {
        CapacityProfile::from_probe(
            Some(horizon_tokens),
            None,
            Horizon {
                tokens: horizon_tokens,
                confidence: 0.9,
                last_confirmed: std::time::SystemTime::now(),
            },
            CacheBehaviour::Unknown,
            0,
            ProbeProvenance {
                probed_at: std::time::SystemTime::now(),
                rungs: Vec::new(),
                signals: Vec::new(),
                metadata_digest: "d".into(),
                quantisation: None,
            },
        )
    }

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

    fn card(
        turn_id: &str,
        text: &str,
        tokens_full: u64,
        tokens_card: u64,
        domains: &[&str],
    ) -> TurnCard {
        TurnCard {
            turn_id: turn_id.to_string(),
            asked: text.to_string(),
            did: Vec::<TraceLine>::new(),
            answered: Answer {
                presentations: Vec::new(),
                narration: format!("answer for {turn_id}"),
            },
            outcome: "completed".into(),
            reading: ReadingKey {
                act: "answer".into(),
                domains: domains.iter().map(|d| d.to_string()).collect(),
                modalities: Vec::new(),
                context: String::new(),
            },
            tokens_full,
            tokens_card,
        }
    }

    /// Builds `n` closed turns, each with a directly-constructed `TurnCard`
    /// (so `tokens_full`/`tokens_card` are exact, not derived from rendered
    /// text) and returns the log plus the turn ids in chronological order.
    /// The contract for history: nothing but the compact card of the turn
    /// the open one follows.
    fn carries_only_the_preceding_card(index: &TurnIndex, plan: &WorkingSetPlan) -> bool {
        let preceding = index
            .turns
            .iter()
            .rev()
            .find(|turn| turn.closed && turn.card.is_some() && !turn.behind_reset)
            .map(|turn| turn.id.as_str());
        plan.per_turn.len() <= 1
            && plan
                .per_turn
                .iter()
                .all(|(id, fidelity)| Some(id.as_str()) == preceding && *fidelity == Fidelity::Card)
    }

    fn fixture(
        dir: &std::path::Path,
        specs: &[(&str, u64, u64, &[&str])],
    ) -> (SessionLog, Vec<String>) {
        let path = SessionPath::new_session_file(dir, dir, "s1");
        let mut log = SessionLog::create(path, header(dir)).unwrap();
        let mut ids = Vec::new();
        for (text, tokens_full, tokens_card, domains) in specs {
            let id = log
                .append_message(MessageRecord {
                    message: M::user_text(*text),
                    meta: None,
                })
                .unwrap()
                .id;
            log.append_message(MessageRecord {
                message: M::assistant(vec![vak_llm::ContentBlock::text(format!(
                    "answer for {text}"
                ))]),
                meta: None,
            })
            .unwrap();
            log.append_turn_card(TurnCardRecord {
                turn_id: id.clone(),
                card: card(&id, text, *tokens_full, *tokens_card, domains),
            })
            .unwrap();
            ids.push(id);
        }
        (log, ids)
    }

    /// One turn of a linked fixture: its words, its full cost, the files its
    /// calls touched (`true` = wrote), and the thread its strand belongs to
    /// with whether it continues it.
    struct LinkedTurn<'a> {
        text: &'a str,
        tokens_full: u64,
        files: &'a [(&'a str, bool)],
        thread: Option<(&'a str, bool)>,
    }

    fn strand(turn_id: &str, thread: &str, continues: bool) -> vak_intent::Strand {
        vak_intent::Strand {
            strand_id: format!("{turn_id}.0"),
            thread_id: thread.to_string(),
            text: String::new(),
            reading: vak_intent::Reading::general(),
            relation: vak_intent::StrandRelation::Independent,
            lineage: if continues {
                vak_intent::Lineage::Continues {
                    thread_id: thread.to_string(),
                    merges: Vec::new(),
                }
            } else {
                vak_intent::Lineage::New
            },
            engagement: vak_intent::Engagement::general(),
        }
    }

    fn open_linked_turn(log: &mut SessionLog, spec: &LinkedTurn) -> String {
        let turn_id = uuid::Uuid::now_v7().to_string();
        log.begin_turn(&turn_id).unwrap();
        if let Some((thread, continues)) = spec.thread {
            log.append_intent(vak_session::types::IntentRecord {
                reading: vak_intent::Reading::general(),
                strands: vec![strand(&turn_id, thread, continues)],
                engagement: vak_intent::Engagement::general(),
                provenance: vak_intent::Provenance::new(vak_intent::Tier::General, 1, Vec::new()),
                outcome: None,
                model_visible: None,
                commitment_id: None,
                strand_commitments: Default::default(),
            })
            .unwrap();
        }
        log.append_message(MessageRecord {
            message: M::user_text(spec.text),
            meta: None,
        })
        .unwrap();
        for (path, wrote) in spec.files {
            let path = path.to_string();
            let effect = if *wrote {
                vak_session::types::CallEffect::FileWrite {
                    path,
                    digest: None,
                    bytes: None,
                }
            } else {
                vak_session::types::CallEffect::FileRead {
                    path,
                    digest: None,
                    bytes: None,
                }
            };
            log.append_call_effect(vak_session::types::CallEffectRecord {
                tool_use_id: uuid::Uuid::now_v7().to_string(),
                effect,
            })
            .unwrap();
        }
        turn_id
    }

    /// Closed turns from `specs`, then the open turn `open` (no reply yet).
    fn linked_fixture(
        dir: &std::path::Path,
        specs: &[LinkedTurn],
        open: &LinkedTurn,
    ) -> (SessionLog, Vec<String>) {
        let path = SessionPath::new_session_file(dir, dir, "s1");
        let mut log = SessionLog::create(path, header(dir)).unwrap();
        let mut ids = Vec::new();
        for spec in specs {
            let id = open_linked_turn(&mut log, spec);
            log.append_message(MessageRecord {
                message: M::assistant(vec![vak_llm::ContentBlock::text("done")]),
                meta: None,
            })
            .unwrap();
            log.append_turn_card(TurnCardRecord {
                turn_id: id.clone(),
                card: card(&id, spec.text, spec.tokens_full, 10, &[]),
            })
            .unwrap();
            ids.push(id);
        }
        open_linked_turn(&mut log, open);
        (log, ids)
    }

    fn plan_for(index: &TurnIndex, horizon: u64, directive: &str) -> WorkingSetPlan {
        let profile = profile(horizon);
        plan(PlanInput {
            profile: &profile,
            index,
            directive,
            reading: None,
            prefix_tokens: 0,
            tail_tokens: 0,
            current_turn_tokens: 0,
        })
    }

    #[test]
    fn linked_turn_promoted_without_shared_words() {
        let dir = tempfile::tempdir().unwrap();
        let unrelated = LinkedTurn {
            text: "weather in paris",
            tokens_full: 100,
            files: &[],
            thread: None,
        };
        let (log, _ids) = linked_fixture(
            dir.path(),
            &[
                LinkedTurn {
                    text: "refactor the tokenizer",
                    tokens_full: 300,
                    files: &[("src/parser.rs", true)],
                    thread: None,
                },
                LinkedTurn { ..unrelated },
                LinkedTurn { ..unrelated },
                LinkedTurn { ..unrelated },
            ],
            &LinkedTurn {
                text: "fix the failing tests",
                tokens_full: 0,
                files: &[("./src/parser.rs", false)],
                thread: None,
            },
        );
        let index = TurnIndex::from_log(&log);
        let result = plan_for(&index, 2_000, "fix the failing tests");
        assert!(
            carries_only_the_preceding_card(&index, &result),
            "only the preceding card: {result:?}"
        );
        assert!(result.links.is_empty());
        assert!(result.retrieved.is_empty());
        let (replayed, _) =
            WorkingSetPlan::from_activity_data(&result.to_activity_data(Some("leaf"))).unwrap();
        assert_eq!(
            replayed.links, result.links,
            "the plan records why a turn was linked"
        );
    }

    #[test]
    fn hub_artifact_does_not_link_everything() {
        let dir = tempfile::tempdir().unwrap();
        let hub = LinkedTurn {
            text: "bump a dependency",
            tokens_full: 50,
            files: &[("Cargo.toml", false)],
            thread: None,
        };
        let mut specs: Vec<LinkedTurn> = (0..20).map(|_| LinkedTurn { ..hub }).collect();
        specs.insert(
            3,
            LinkedTurn {
                text: "rewrite the lexer",
                tokens_full: 50,
                files: &[("src/parser.rs", true)],
                thread: None,
            },
        );
        let (log, ids) = linked_fixture(
            dir.path(),
            &specs,
            &LinkedTurn {
                text: "fix the build",
                tokens_full: 0,
                files: &[("Cargo.toml", false), ("src/parser.rs", false)],
                thread: None,
            },
        );
        let index = TurnIndex::from_log(&log);
        let closed: Vec<&vak_session::Turn> = index.turns.iter().filter(|t| t.closed).collect();
        let values = link_values(&closed, index.turns.last(), "");
        let special = values.get(&ids[3]).unwrap().0;
        assert!(
            special > 0.75,
            "a rare shared file links strongly: {special}"
        );
        let hubs: Vec<f64> = ids
            .iter()
            .enumerate()
            .filter(|(i, _)| *i != 3)
            .filter_map(|(_, id)| values.get(id).map(|(v, _)| *v))
            .collect();
        assert_eq!(
            hubs.len(),
            MAX_TURNS_PER_NODE,
            "fan-out through one node is capped"
        );
        assert!(
            hubs.iter().all(|v| *v < 0.2),
            "a file every turn reads barely links: {hubs:?}"
        );
    }

    #[test]
    fn a_second_step_reaches_through_a_shared_turn() {
        let dir = tempfile::tempdir().unwrap();
        let unrelated = LinkedTurn {
            text: "weather in paris",
            tokens_full: 100,
            files: &[],
            thread: None,
        };
        let (log, ids) = linked_fixture(
            dir.path(),
            &[
                LinkedTurn {
                    text: "rewrite the lexer",
                    tokens_full: 100,
                    files: &[("src/parser.rs", true)],
                    thread: None,
                },
                LinkedTurn {
                    text: "cover the lexer",
                    tokens_full: 100,
                    files: &[("src/parser.rs", false), ("tests/parser.rs", true)],
                    thread: None,
                },
                LinkedTurn { ..unrelated },
                LinkedTurn { ..unrelated },
            ],
            &LinkedTurn {
                text: "the tests fail",
                tokens_full: 0,
                files: &[("tests/parser.rs", false)],
                thread: None,
            },
        );
        let index = TurnIndex::from_log(&log);
        let closed: Vec<&vak_session::Turn> = index.turns.iter().filter(|t| t.closed).collect();
        let values = link_values(&closed, index.turns.last(), "");
        let direct = values.get(&ids[1]).unwrap();
        let second = values.get(&ids[0]).unwrap();
        assert!(
            second.0 > 0.0 && second.0 <= STEP_DECAY && second.0 < direct.0,
            "{values:?}"
        );
        assert_eq!(
            second.1,
            vec![
                "file:tests/parser.rs".to_string(),
                "file:src/parser.rs".to_string()
            ]
        );
        assert!(!values.contains_key(&ids[2]) && !values.contains_key(&ids[3]));
    }

    #[test]
    fn named_files_are_paths_not_urls_or_numbers() {
        assert_eq!(
            named_files(
                "fix `src/parser.rs` and ./README.md, bump to 3.5, see https://x.io/a.html, mail a@b.com, then plan.md."
            ),
            vec!["src/parser.rs", "README.md", "plan.md"]
        );
        assert!(named_files("what is the capital of Australia?").is_empty());
    }

    #[test]
    fn a_named_file_links_at_plan_time() {
        let dir = tempfile::tempdir().unwrap();
        let unrelated = LinkedTurn {
            text: "weather in paris",
            tokens_full: 100,
            files: &[],
            thread: None,
        };
        let (log, _ids) = linked_fixture(
            dir.path(),
            &[
                LinkedTurn {
                    text: "draft the bake sale",
                    tokens_full: 300,
                    files: &[("notes/plan.md", true)],
                    thread: None,
                },
                LinkedTurn { ..unrelated },
                LinkedTurn { ..unrelated },
            ],
            // The open turn has run no call yet: only its words can anchor.
            &LinkedTurn {
                text: "shorten plan.md to two lines",
                tokens_full: 0,
                files: &[],
                thread: None,
            },
        );
        let index = TurnIndex::from_log(&log);
        let result = plan_for(&index, 2_000, "shorten plan.md to two lines");
        assert!(
            carries_only_the_preceding_card(&index, &result),
            "file-name match alone uses recall: {result:?}"
        );
    }

    #[test]
    fn the_preceding_turn_follows_the_thread() {
        let dir = tempfile::tempdir().unwrap();
        let (log, ids) = linked_fixture(
            dir.path(),
            &[
                LinkedTurn {
                    text: "draft the release notes",
                    tokens_full: 150,
                    files: &[],
                    thread: Some(("t-notes", false)),
                },
                LinkedTurn {
                    text: "what time is it in tokyo",
                    tokens_full: 150,
                    files: &[],
                    thread: Some(("t-time", false)),
                },
            ],
            &LinkedTurn {
                text: "do that again, shorter",
                tokens_full: 0,
                files: &[],
                thread: Some(("t-notes", true)),
            },
        );
        let index = TurnIndex::from_log(&log);
        // "that" points to the release notes in the continued thread; send
        // its compact card and leave the full history to `recall`.
        let result = plan_for(&index, 200, "do that again, shorter");
        assert_eq!(result.per_turn, vec![(ids[0].clone(), Fidelity::Card)]);
    }

    #[test]
    fn link_weights_are_pinned() {
        assert_eq!(link_weight(LinkKind::OwnThread), 1.0);
        assert_eq!(link_weight(LinkKind::WroteFile), 1.0);
        assert_eq!(link_weight(LinkKind::ReadFile), 0.8);
        assert_eq!(link_weight(LinkKind::ServesCommitment), 0.8);
        assert_eq!(link_weight(LinkKind::ContinuesThread), 0.7);
        assert_eq!(link_weight(LinkKind::ThisTurn), 1.0);
        assert_eq!(link_weight(LinkKind::RecalledTurn), 1.0);
        assert_eq!(link_weight(LinkKind::NamedFile), 0.8);
        assert_eq!(MAX_TURNS_PER_NODE, 8);
        assert_eq!(MAX_LINK_STEPS, 2);
        assert_eq!(STEP_DECAY, 0.5);
    }

    #[test]
    fn a_fresh_request_leaves_spare_capacity_unused() {
        let dir = tempfile::tempdir().unwrap();
        // Costs chosen so exactly the newest 2 fit; the 3rd would overflow.
        let (log, _ids) = fixture(
            dir.path(),
            &[
                ("turn one", 400, 40, &[]),
                ("turn two", 400, 40, &[]),
                ("turn three", 400, 40, &[]),
            ],
        );
        let index = TurnIndex::from_log(&log);
        let profile = profile(1_100);
        let result = plan(PlanInput {
            profile: &profile,
            index: &index,
            directive: "turn",
            reading: None,
            prefix_tokens: 100,
            tail_tokens: 0,
            current_turn_tokens: 0,
        });
        // Spare capacity stays unused when the request does not refer to a
        // prior turn.
        assert!(
            carries_only_the_preceding_card(&index, &result),
            "{result:?}"
        );
        assert!(result.spent <= result.budget);
    }

    #[test]
    fn planning_remeasures_tool_arguments_instead_of_trusting_old_card_costs() {
        let dir = tempfile::tempdir().unwrap();
        let path = SessionPath::new_session_file(dir.path(), dir.path(), "s1");
        let mut log = SessionLog::create(path, header(dir.path())).unwrap();
        let id = log
            .append_message(MessageRecord {
                message: M::user_text("create spreadsheet"),
                meta: None,
            })
            .unwrap()
            .id;
        log.append_message(MessageRecord {
            message: M::assistant(vec![vak_llm::ContentBlock::ToolUse {
                id: "call".into(),
                name: "write".into(),
                input: serde_json::json!({"content": "spreadsheet data ".repeat(10_000)}),
            }]),
            meta: None,
        })
        .unwrap();
        log.append_message(MessageRecord {
            message: M {
                role: vak_llm::Role::User,
                content: vec![vak_llm::ContentBlock::ToolResult {
                    tool_use_id: "call".into(),
                    content: "written".into(),
                    is_error: false,
                }],
            },
            meta: None,
        })
        .unwrap();
        log.append_message(MessageRecord {
            message: M::assistant(vec![vak_llm::ContentBlock::text("Created spreadsheet")]),
            meta: None,
        })
        .unwrap();
        log.append_turn_card(TurnCardRecord {
            turn_id: id.clone(),
            card: card(&id, "create spreadsheet", 1, 1, &[]),
        })
        .unwrap();
        log.append_message(MessageRecord {
            message: M::user_text("update that spreadsheet"),
            meta: None,
        })
        .unwrap();
        let result = plan_for_session(&log, &profile(1_000), 0, 0);
        assert!(
            result.per_turn.contains(&(id, Fidelity::Card)),
            "a follow-up carries the compact turn card; oversized arguments require recall: {result:?}"
        );
        assert!(result.spent <= result.budget);
    }

    #[test]
    fn fresh_topic_leaves_spare_capacity_empty() {
        let dir = tempfile::tempdir().unwrap();
        let (log, _) = fixture(
            dir.path(),
            &[
                (
                    "what is the current Indian stock market",
                    50_000,
                    40,
                    &["information"],
                ),
                (
                    "make the latest Excel document",
                    50_000,
                    40,
                    &["information"],
                ),
            ],
        );
        let index = TurnIndex::from_log(&log);
        let reading = ReadingKey {
            act: "answer".into(),
            domains: vec!["information".into()],
            modalities: vec![],
            context: "recall".into(),
        };
        let profile = profile(200_000);
        let result = plan(PlanInput {
            profile: &profile,
            index: &index,
            directive: "what is the current weather in noida",
            reading: Some(&reading),
            prefix_tokens: 0,
            tail_tokens: 0,
            current_turn_tokens: 0,
        });
        assert!(
            carries_only_the_preceding_card(&index, &result),
            "{result:?}"
        );
        assert!(result.retrieved.is_empty());
    }

    #[test]
    fn topic_overlap_does_not_preload_a_turn_one_hundred_back() {
        let dir = tempfile::tempdir().unwrap();
        let mut specs = vec![("Noida weather forecast", 100, 5, &[][..])];
        specs.extend((0..100).map(|_| ("Indian stock market dashboard", 100, 5, &[][..])));
        let (log, _ids) = fixture(dir.path(), &specs);
        let index = TurnIndex::from_log(&log);
        let profile = profile(100_000);
        let result = plan(PlanInput {
            profile: &profile,
            index: &index,
            directive: "Noida weather update",
            reading: None,
            prefix_tokens: 0,
            tail_tokens: 0,
            current_turn_tokens: 0,
        });
        assert!(
            carries_only_the_preceding_card(&index, &result),
            "topical return uses recall: {result:?}"
        );
        assert!(result.retrieved.is_empty());
    }

    #[test]
    fn matching_history_cannot_fill_the_model_horizon_with_full_records() {
        let dir = tempfile::tempdir().unwrap();
        let specs = (0..20)
            .map(|n| {
                let text = if n == 0 {
                    "Noida weather from earlier"
                } else {
                    "Noida weather details and measurements"
                };
                (text, 6_000, 40, &[][..])
            })
            .collect::<Vec<_>>();
        let (log, _) = fixture(dir.path(), &specs);
        let index = TurnIndex::from_log(&log);
        let result = plan(PlanInput {
            profile: &profile(200_000),
            index: &index,
            directive: "What temperature did you report in the latest weather answer?",
            reading: None,
            prefix_tokens: 0,
            tail_tokens: 0,
            current_turn_tokens: 0,
        });
        assert!(
            carries_only_the_preceding_card(&index, &result),
            "historical search is on demand: {result:?}"
        );
        assert!(result.retrieved.is_empty());
        assert!(result.spent <= result.budget);
    }

    #[test]
    fn a_relevant_older_turn_waits_for_on_demand_recall() {
        let dir = tempfile::tempdir().unwrap();
        let (log, _ids) = fixture(
            dir.path(),
            &[
                ("what is the sensex today", 15, 5, &["finance"]),
                ("unrelated small talk", 60, 10, &[]),
                ("another unrelated turn", 60, 10, &[]),
            ],
        );
        let index = TurnIndex::from_log(&log);
        // budget = 130; retrieval_reserve = 19, main_budget = 111. Recency
        // (newest -> oldest) fits the newest unrelated turn (60 <= 111) but
        // the next one would make 120 > 111 -> stops there, leaving the
        // sensex turn (oldest) reachable only through relevance.
        let profile = profile(130);
        let result = plan(PlanInput {
            profile: &profile,
            index: &index,
            directive: "sensex",
            reading: None,
            prefix_tokens: 0,
            tail_tokens: 0,
            current_turn_tokens: 0,
        });
        assert!(
            carries_only_the_preceding_card(&index, &result),
            "topical recall is on demand: {result:?}"
        );
        assert!(result.retrieved.is_empty());
        assert!(result.spent <= result.budget);
    }

    /// `ContextProfile::Minimal` (docs/design/47-commitment-kernel.md): a
    /// greeting does not retrieve or carry older turns, however much spare
    /// budget there is.
    #[test]
    fn a_minimal_reading_neither_retrieves_nor_carries_old_turns_at_full() {
        let dir = tempfile::tempdir().unwrap();
        let (log, _ids) = fixture(
            dir.path(),
            &[
                ("what is the sensex today", 15, 5, &["finance"]),
                ("unrelated small talk", 60, 10, &[]),
                ("another unrelated turn", 60, 10, &[]),
                ("and one more", 60, 10, &[]),
            ],
        );
        let index = TurnIndex::from_log(&log);
        // Plenty of budget: every turn would be `Full` for a recall reading.
        let profile = profile(10_000);
        let minimal = ReadingKey {
            act: "converse".into(),
            domains: Vec::new(),
            modalities: Vec::new(),
            context: "minimal".into(),
        };
        let result = plan(PlanInput {
            profile: &profile,
            index: &index,
            directive: "sensex",
            reading: Some(&minimal),
            prefix_tokens: 0,
            tail_tokens: 0,
            current_turn_tokens: 0,
        });
        let full: Vec<&str> = result
            .per_turn
            .iter()
            .filter(|(_, fidelity)| *fidelity == Fidelity::Full)
            .map(|(id, _)| id.as_str())
            .collect();
        assert!(result.retrieved.is_empty(), "{:?}", result.retrieved);
        assert!(full.is_empty(), "{full:?}");
        assert!(
            carries_only_the_preceding_card(&index, &result),
            "{result:?}"
        );

        // A recall reading retrieves the matching subject, not every turn.
        let recall = ReadingKey {
            context: "recall".into(),
            ..minimal.clone()
        };
        let result = plan(PlanInput {
            profile: &profile,
            index: &index,
            directive: "sensex",
            reading: Some(&recall),
            prefix_tokens: 0,
            tail_tokens: 0,
            current_turn_tokens: 0,
        });
        assert!(
            carries_only_the_preceding_card(&index, &result),
            "a recall reading adds no history by itself"
        );
    }

    #[test]
    fn a_request_carries_only_the_preceding_turn_card() {
        let dir = tempfile::tempdir().unwrap();
        let (log, ids) = fixture(
            dir.path(),
            &[
                ("filler turn from earlier", 160, 10, &[]),
                ("show me the chart for AAPL", 180, 10, &[]),
            ],
        );
        let index = TurnIndex::from_log(&log);
        // The AAPL turn is the immediately preceding referent. Its compact
        // card is enough to identify it; `recall` can reopen detail.
        let profile = profile(200);
        let result = plan(PlanInput {
            profile: &profile,
            index: &index,
            directive: "do that again",
            reading: None,
            prefix_tokens: 0,
            tail_tokens: 0,
            current_turn_tokens: 0,
        });
        assert!(
            result
                .per_turn
                .iter()
                .any(|(id, fidelity)| id == &ids[1] && *fidelity == Fidelity::Card),
            "only the immediately preceding card is carried: {:?}",
            result
        );
        assert!(!result.per_turn.iter().any(|(id, _)| id == &ids[0]));
        assert!(result.spent <= result.budget);
        let data = result.to_activity_data(Some("leaf"));
        let recorded_cards: Vec<String> = serde_json::from_str(data.get("cards").unwrap()).unwrap();
        assert_eq!(recorded_cards, vec![ids[1].clone()]);
        let (replayed, _) = WorkingSetPlan::from_activity_data(&data).unwrap();
        assert_eq!(replayed.per_turn, result.per_turn);
    }

    /// A turn behind a reset-with-handoff is invisible to the model and
    /// therefore never a candidate: not Full, not Card, and never the
    /// start of a packet range (which would otherwise trigger a
    /// summariser call over turns the model cannot see).
    #[test]
    fn turns_behind_a_reset_are_never_planned() {
        let dir = tempfile::tempdir().unwrap();
        let (mut log, ids) = fixture(
            dir.path(),
            &[("turn one", 400, 40, &[]), ("turn two", 400, 40, &[])],
        );
        log.append_handoff_reset("handoff".into(), 1_000, false)
            .unwrap();
        let id3 = log
            .append_message(MessageRecord {
                message: M::user_text("turn three"),
                meta: None,
            })
            .unwrap()
            .id;
        log.append_message(MessageRecord {
            message: M::assistant(vec![vak_llm::ContentBlock::text("answer for turn three")]),
            meta: None,
        })
        .unwrap();
        log.append_turn_card(TurnCardRecord {
            turn_id: id3.clone(),
            card: card(&id3, "turn three", 400, 40, &[]),
        })
        .unwrap();

        let index = TurnIndex::from_log(&log);
        assert!(index.turns[0].behind_reset && index.turns[1].behind_reset);
        assert!(!index.turns[2].behind_reset);

        // Tiny budget: only a packet could hold the pre-reset turns, and
        // even that must not happen.
        let profile = profile(200);
        let result = plan(PlanInput {
            profile: &profile,
            index: &index,
            directive: "turn one again",
            reading: None,
            prefix_tokens: 100,
            tail_tokens: 0,
            current_turn_tokens: 0,
        });
        let planned: Vec<&str> = result.per_turn.iter().map(|(id, _)| id.as_str()).collect();
        assert!(!planned.contains(&ids[0].as_str()) && !planned.contains(&ids[1].as_str()));
        assert!(
            result
                .packet_range
                .as_ref()
                .is_none_or(|(first, last)| first != &ids[0] && last != &ids[1]),
            "{:?}",
            result.packet_range
        );
    }

    #[test]
    fn topical_history_is_not_added_to_a_packet_automatically() {
        let dir = tempfile::tempdir().unwrap();
        let (log, _ids) = fixture(
            dir.path(),
            &[
                ("ancient turn", 5_000, 5_000, &[]),
                ("old turn", 10, 10, &[]),
                ("recent turn", 10, 10, &[]),
            ],
        );
        let index = TurnIndex::from_log(&log);
        // A broad word shared by old turns does not cause the planner to
        // packet the entire intervening history.
        let profile = profile(50);
        let result = plan(PlanInput {
            profile: &profile,
            index: &index,
            directive: "turn",
            reading: None,
            prefix_tokens: 0,
            tail_tokens: 0,
            current_turn_tokens: 0,
        });
        assert!(result.packet_range.is_none(), "{result:?}");
        assert!(
            carries_only_the_preceding_card(&index, &result),
            "{result:?}"
        );
        assert!(result.spent <= result.budget);
    }

    #[test]
    fn existing_packets_are_ignored_when_the_request_does_not_need_history() {
        let dir = tempfile::tempdir().unwrap();
        let (mut log, _) = fixture(
            dir.path(),
            &[
                ("Noida weather", 100, 10, &[]),
                ("sales dashboard", 100, 10, &[]),
            ],
        );
        let index = TurnIndex::from_log(&log);
        log.append_packet(
            &index.turns[0].id,
            &index.turns[1].id,
            "m",
            "stale packet".into(),
            1,
        )
        .unwrap();
        let index = TurnIndex::from_log(&log);
        let profile = profile(10_000);
        let result = plan(PlanInput {
            profile: &profile,
            index: &index,
            directive: "what is 2 plus 2",
            reading: None,
            prefix_tokens: 0,
            tail_tokens: 0,
            current_turn_tokens: 0,
        });
        assert!(result.packet_range.is_none());
        assert!(carries_only_the_preceding_card(&index, &result));
    }

    #[test]
    fn the_preceding_turn_outranks_a_lexical_hit_on_an_older_turn() {
        let dir = tempfile::tempdir().unwrap();
        let (log, ids) = fixture(
            dir.path(),
            &[
                ("Translate the welcome greeting into Hindi", 160, 10, &[]),
                ("Summarize the quarterly sales report", 180, 10, &[]),
            ],
        );
        let index = TurnIndex::from_log(&log);
        let profile = profile(200);
        let result = plan(PlanInput {
            profile: &profile,
            index: &index,
            directive: "Now translate that into Hindi",
            reading: None,
            prefix_tokens: 0,
            tail_tokens: 0,
            current_turn_tokens: 0,
        });
        assert!(
            result.per_turn.contains(&(ids[1].clone(), Fidelity::Card)),
            "the referent of \"that\" is the preceding turn: {result:?}"
        );
    }

    #[test]
    fn tiny_horizon_yields_no_planned_turns() {
        let dir = tempfile::tempdir().unwrap();
        let (log, _ids) = fixture(
            dir.path(),
            &[("only turn", 500, 100, &[]), ("second turn", 500, 100, &[])],
        );
        let index = TurnIndex::from_log(&log);
        // Horizon entirely consumed by prefix/tail/reserve: budget saturates
        // to zero, so the open turn is the only thing left (planned turns
        // are empty; the caller sends only the open turn verbatim).
        let profile = profile(100);
        let result = plan(PlanInput {
            profile: &profile,
            index: &index,
            directive: "hello",
            reading: None,
            prefix_tokens: 60,
            tail_tokens: 40,
            current_turn_tokens: 50,
        });
        assert_eq!(result.budget, 0);
        assert!(
            result
                .per_turn
                .iter()
                .all(|(_, f)| !matches!(f, Fidelity::Full | Fidelity::Card))
        );
        assert_eq!(result.spent, 0);
    }

    #[test]
    fn selected_plan_uses_only_scoped_candidates_and_costs_actual_card_markup() {
        let dir = tempfile::tempdir().unwrap();
        let (log, ids) = fixture(
            dir.path(),
            &[
                ("weather in Noida", 400, 40, &["weather"]),
                ("OpenAI release notes", 450, 45, &["release"]),
            ],
        );
        let mut index = TurnIndex::from_log(&log);
        index.turns.retain(|turn| turn.id == ids[1]);
        let profile = profile(20_000);
        let plan = plan_selected(PlanInput {
            profile: &profile,
            index: &index,
            directive: "continue the release notes",
            reading: None,
            prefix_tokens: 0,
            tail_tokens: 0,
            current_turn_tokens: 0,
        });
        assert_eq!(plan.selected_records.as_ref().unwrap().len(), 1);
        assert_eq!(plan.selected_records.as_ref().unwrap()[0].0, ids[1]);
        assert!(
            !plan
                .selected_records
                .as_ref()
                .unwrap()
                .iter()
                .any(|(id, _)| id == &ids[0])
        );
        let full_chars = messages_chars(&index.turns[0].full_record());
        assert_eq!(plan.per_turn[0].1, Fidelity::Full);
        assert_eq!(plan.spent, profile.estimate_tokens(full_chars));
    }
}
