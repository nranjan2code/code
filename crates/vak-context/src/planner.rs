//! `WorkingSetPlanner` (docs/design/68-context-engine.md §4, §10): decides,
//! for one request, which closed turns ride along at `Full` fidelity, which
//! collapse to a one-line `Card`, and which are pushed into a `Packet` —
//! from measured costs against a measured budget, never a fixed count.
//! Pure: no I/O, no locks, no network. The caller
//! (`vak-agent`'s turn loop) supplies the `CapacityProfile`, the
//! `TurnIndex`, and the incoming directive; `plan()` returns a `WorkingSetPlan`
//! that `SessionLog::derive_with_plan` turns into messages.

pub use vak_session::{Fidelity, WorkingSetPlan};
use vak_session::{ReadingKey, SessionLog, TurnIndex};

use crate::assemble::messages_chars;
use crate::capacity::CapacityProfile;

/// Directive fragments that refer back to the immediately preceding turn
/// without repeating its subject (§4/§10: "anaphora ... always promotes the
/// immediately preceding turn"). Matched as a case-insensitive substring of
/// the directive text — deliberately loose, since a false positive costs
/// one extra `Full` turn and a false negative costs nothing the relevance
/// search wouldn't otherwise catch.
pub const ANAPHORA_PHRASES: [&str; 7] = [
    "that", "it", "again", "the same", "previous", "above", "this one",
];

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

/// Whether `directive` contains an anaphoric reference to "the last thing"
/// (§4/§10's anaphora word list).
fn is_anaphoric(directive: &str) -> bool {
    let lower = directive.to_lowercase();
    let words: Vec<&str> = lower
        .split(|c: char| !c.is_alphanumeric())
        .filter(|w| !w.is_empty())
        .collect();
    ANAPHORA_PHRASES.iter().any(|phrase| {
        if phrase.contains(' ') {
            lower.contains(phrase)
        } else {
            words.contains(phrase)
        }
    })
}

/// How many of the most recent closed turns a `minimal` reading may still
/// carry at `Full`. Two: the exchange just before this one, and the one
/// before that, which is what a greeting or a one-line answer plausibly
/// refers to.
const MINIMAL_FULL_TURNS: usize = 2;

/// A turn's value from recency alone: `1 / (1 + age)` where `age` is how
/// many closed turns came after it. The most recent closed turn is worth
/// 1.0, the one before it 0.5, and so on — a parameter-free decay that a
/// perfectly relevant older turn (normalised lexical score 1.0) ties with
/// rather than loses to.
fn recency_value(age: usize) -> f64 {
    1.0 / (1.0 + age as f64)
}

/// The relevance query: the directive plus the current reading's act and
/// domain words, so reading overlap is scored by the same BM25 as the text
/// instead of being a separate boolean bonus.
fn relevance_query(directive: &str, reading: Option<&ReadingKey>) -> String {
    let mut query = directive.to_string();
    if let Some(reading) = reading {
        query.push(' ');
        query.push_str(&reading.act);
        for domain in &reading.domains {
            query.push(' ');
            query.push_str(domain);
        }
    }
    query
}

/// Builds the working-set plan for one request (§4, §10). Never splits a
/// turn: every cost check is turn-whole, and a turn that does not fit is
/// deferred to the next tier down (Full → Card → Packet) rather than
/// truncated.
pub fn plan(input: PlanInput) -> WorkingSetPlan {
    let reserve = input
        .profile
        .current_turn_reserve(input.current_turn_tokens);
    let budget = input
        .profile
        .budget(input.prefix_tokens, input.tail_tokens, reserve);

    // Only closed turns with a written card are plannable at all — the
    // still-open turn (last, uncarded) is never planned; it is always sent
    // verbatim by the caller.
    // Turns behind a reset-with-handoff are invisible to the model: not
    // candidates for any tier, and never the start of a packet range.
    let closed: Vec<&vak_session::Turn> = input
        .index
        .turns
        .iter()
        .filter(|turn| turn.closed && turn.card.is_some() && !turn.behind_reset)
        .collect();

    let mut spent: u64 = 0;
    let mut full_ids: std::collections::HashSet<String> = std::collections::HashSet::new();
    let mut retrieved: Vec<String> = Vec::new();

    // Every closed turn is scored once and the budget is filled in
    // descending value: the most recent turn and the most relevant turn are
    // both worth 1.0, an older or less relevant one proportionally less, so
    // no share of the budget is reserved for either signal — recency and
    // relevance compete for the same tokens on equal terms, and a turn that
    // does not fit is skipped for a cheaper one further down the ranking
    // rather than blocking everything behind it.
    let anaphoric = is_anaphoric(input.directive);
    let preceding = closed.last().map(|turn| turn.id.clone());
    // `ContextProfile::Minimal` (docs/design/47-commitment-kernel.md): just
    // the conversation. No relevance retrieval promotes an older turn, and
    // only the most recent turns are candidates for `Full`; a greeting does
    // not pay for last Tuesday. Anaphora still promotes the preceding turn —
    // "thanks, do that again" points at it.
    let minimal = input
        .reading
        .is_some_and(vak_session::ReadingKey::is_minimal);
    let query = relevance_query(input.directive, input.reading);
    let lexical: std::collections::HashMap<String, f64> = if minimal {
        std::collections::HashMap::new()
    } else {
        input.index.search(&query).into_iter().collect()
    };
    let best_lexical = lexical.values().copied().fold(0.0_f64, f64::max);
    let mut ranked: Vec<(f64, f64, usize, &vak_session::Turn)> = closed
        .iter()
        .enumerate()
        .map(|(position, turn)| {
            let age = closed.len() - 1 - position;
            let recency = if minimal && age >= MINIMAL_FULL_TURNS {
                0.0
            } else {
                recency_value(age)
            };
            let relevance = if best_lexical > 0.0 {
                lexical.get(&turn.id).copied().unwrap_or(0.0) / best_lexical
            } else {
                0.0
            };
            let anaphora = if anaphoric && preceding.as_deref() == Some(turn.id.as_str()) {
                1.0
            } else {
                0.0
            };
            (
                recency.max(relevance).max(anaphora),
                recency,
                position,
                *turn,
            )
        })
        .collect();
    ranked.sort_by(|a, b| {
        b.0.partial_cmp(&a.0)
            .unwrap_or(std::cmp::Ordering::Equal)
            .then(b.2.cmp(&a.2))
    });
    for (value, recency, _, turn) in &ranked {
        // A turn worth nothing is not promoted to `Full`, whatever the
        // budget; it still gets a card below.
        if *value <= 0.0 {
            continue;
        }
        let cost = turn.card.as_ref().map(|c| c.tokens_full).unwrap_or(0);
        if spent.saturating_add(cost) <= budget {
            spent += cost;
            full_ids.insert(turn.id.clone());
            if *value > *recency {
                retrieved.push(turn.id.clone());
            }
        }
    }

    // 3) Card tier over everything still not Full, newest -> oldest, while
    // it fits in what remains of `budget`. The first one that does not fit
    // — and everything older than it — collapses into a single packet
    // range instead (never split, never scattered).
    let mut per_turn: Vec<(String, Fidelity)> = Vec::new();
    let mut packet_ids: Vec<String> = Vec::new();
    let mut packet_started = false;
    for turn in closed.iter().rev() {
        if full_ids.contains(&turn.id) {
            continue;
        }
        if packet_started {
            packet_ids.push(turn.id.clone());
            continue;
        }
        let cost = turn.card.as_ref().map(|c| c.tokens_card).unwrap_or(0);
        if spent.saturating_add(cost) <= budget {
            spent += cost;
            per_turn.push((turn.id.clone(), Fidelity::Card));
        } else {
            packet_started = true;
            packet_ids.push(turn.id.clone());
        }
    }
    for id in full_ids {
        per_turn.push((id, Fidelity::Full));
    }

    // Render in chronological order (oldest first), matching `index.turns`.
    let order: std::collections::HashMap<&str, usize> = closed
        .iter()
        .enumerate()
        .map(|(i, t)| (t.id.as_str(), i))
        .collect();
    per_turn.sort_by_key(|(id, _)| order.get(id.as_str()).copied().unwrap_or(usize::MAX));

    let packet_range = if packet_ids.is_empty() {
        None
    } else {
        // `packet_ids` was collected oldest-appended-last while walking
        // newest -> oldest, so the range is (last pushed, first pushed).
        let first = packet_ids.last().cloned();
        let last = packet_ids.first().cloned();
        first.zip(last)
    };

    WorkingSetPlan {
        per_turn,
        packet_range,
        retrieved,
        budget,
        spent,
    }
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
    index.ensure_cards(&|text| profile.estimate_tokens(text.chars().count() as u64));
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
            horizon_tokens,
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

    #[test]
    fn fills_newest_first_and_never_splits() {
        let dir = tempfile::tempdir().unwrap();
        // Costs chosen so exactly the newest 2 fit; the 3rd would overflow.
        let (log, ids) = fixture(
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
            directive: "unrelated question",
            reading: None,
            prefix_tokens: 100,
            tail_tokens: 0,
            current_turn_tokens: 0,
        });
        // budget = 1100 - 100 = 1000; retrieval_reserve = 150, so recency
        // fills against main_budget = 850: turn3(400) fits(400<=850),
        // turn2(400) fits(800<=850), turn1(400) would make 1200 > 850 ->
        // stops, turn1 stays out of Full.
        let full: Vec<&str> = result
            .per_turn
            .iter()
            .filter(|(_, f)| matches!(f, Fidelity::Full))
            .map(|(id, _)| id.as_str())
            .collect();
        assert!(full.contains(&ids[2].as_str()));
        assert!(full.contains(&ids[1].as_str()));
        assert!(
            !full.contains(&ids[0].as_str()),
            "turn one must not be Full"
        );
        assert!(result.spent <= result.budget);
    }

    #[test]
    fn retrieval_promotes_a_relevant_older_turn() {
        let dir = tempfile::tempdir().unwrap();
        let (log, ids) = fixture(
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
            result.retrieved.contains(&ids[0]),
            "the sensex turn should be promoted by BM25 relevance: {:?}",
            result.retrieved
        );
        assert!(result.spent <= result.budget);
    }

    /// `ContextProfile::Minimal` (docs/design/47-commitment-kernel.md): a
    /// greeting does not retrieve an older turn on relevance, and only the
    /// most recent turns are candidates for `Full`, however much budget
    /// there is.
    #[test]
    fn a_minimal_reading_neither_retrieves_nor_carries_old_turns_at_full() {
        let dir = tempfile::tempdir().unwrap();
        let (log, ids) = fixture(
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
        assert!(full.contains(&ids[3].as_str()));
        assert!(full.contains(&ids[2].as_str()));
        assert!(!full.contains(&ids[1].as_str()), "{full:?}");
        assert!(!full.contains(&ids[0].as_str()), "{full:?}");

        // The same request with a recall reading carries everything.
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
        let full = result
            .per_turn
            .iter()
            .filter(|(_, fidelity)| *fidelity == Fidelity::Full)
            .count();
        assert_eq!(full, 4);
    }

    #[test]
    fn anaphora_always_promotes_the_immediately_preceding_turn() {
        let dir = tempfile::tempdir().unwrap();
        let (log, ids) = fixture(
            dir.path(),
            &[
                ("filler turn from earlier", 160, 10, &[]),
                ("show me the chart for AAPL", 180, 10, &[]),
            ],
        );
        let index = TurnIndex::from_log(&log);
        // budget = 200: only one of the two turns can be Full. The AAPL turn
        // is the immediately preceding one, so it is worth 1.0 by recency
        // and by anaphora alike and must be the one that rides Full; the
        // filler turn falls to a card.
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
                .any(|(id, fidelity)| id == &ids[1] && *fidelity == Fidelity::Full),
            "anaphora must promote the immediately preceding turn: {:?}",
            result
        );
        assert!(result.spent <= result.budget);
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
        log.append_handoff_reset("handoff".into(), 1_000).unwrap();
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
    fn card_overflow_collapses_the_oldest_into_a_packet() {
        let dir = tempfile::tempdir().unwrap();
        let (log, ids) = fixture(
            dir.path(),
            &[
                ("ancient turn", 5_000, 5_000, &[]),
                ("old turn", 10, 10, &[]),
                ("recent turn", 10, 10, &[]),
            ],
        );
        let index = TurnIndex::from_log(&log);
        // Budget fits the two small turns as Full (recency) with nothing
        // left for the ancient turn's oversized card -> it becomes a packet.
        let profile = profile(50);
        let result = plan(PlanInput {
            profile: &profile,
            index: &index,
            directive: "unrelated",
            reading: None,
            prefix_tokens: 0,
            tail_tokens: 0,
            current_turn_tokens: 0,
        });
        assert!(result.packet_range.is_some(), "{:?}", result);
        let (first, last) = result.packet_range.unwrap();
        assert_eq!(first, ids[0]);
        assert_eq!(last, ids[0]);
        assert!(result.spent <= result.budget);
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
}
