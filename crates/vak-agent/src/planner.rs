//! `WorkingSetPlanner` (docs/design/68-context-engine.md §4, §10): decides,
//! for one request, which closed turns ride along at `Full` fidelity, which
//! collapse to a one-line `Card`, and which are pushed into a `Packet` —
//! from measured costs against a measured budget, never a fixed
//! `keep_recent` count. Pure: no I/O, no locks, no network. The caller
//! (`vak-agent`'s turn loop) supplies the `CapacityProfile`, the
//! `TurnIndex`, and the incoming directive; `plan()` returns a `WorkingSetPlan`
//! that `SessionLog::derive_with_plan` turns into messages.

pub use vak_session::{Fidelity, WorkingSetPlan};
use vak_session::{ReadingKey, TurnIndex};

/// Share of the budget reserved for relevance-promoted older turns (§4):
/// "a reserved slice of the budget (`horizon × 0.15`, measured not guessed:
/// it is the share the vakyartha simulation found recovers 2.5–2.8×) —
/// re-validate with the probe harness." Not a cap on the whole plan, only
/// on how much of the budget retrieval promotion may spend beyond the
/// recency fill.
pub const RETRIEVAL_SHARE: f64 = 0.15;

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
    pub profile: &'a crate::capacity::CapacityProfile,
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

/// Whether two readings share an act or at least one domain — the "reading
/// overlap (same act/domains)" relevance signal (§10).
fn reading_overlaps(current: &ReadingKey, candidate: &ReadingKey) -> bool {
    current.act == candidate.act
        || current
            .domains
            .iter()
            .any(|domain| candidate.domains.contains(domain))
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
    let closed: Vec<&vak_session::Turn> = input
        .index
        .turns
        .iter()
        .filter(|turn| turn.closed && turn.card.is_some())
        .collect();

    let mut spent: u64 = 0;
    let mut full_ids: std::collections::HashSet<String> = std::collections::HashSet::new();
    let mut retrieved: Vec<String> = Vec::new();

    // The retrieval slice is carved out of the budget UP FRONT for the
    // *recency* fill, so a relevant or anaphoric older turn is never
    // permanently shut out just because recency alone already exhausted the
    // budget on newer turns — that would defeat the purpose of a reserved
    // slice. Anaphora gets a stronger guarantee still: it is checked against
    // the full remaining budget (main-budget leftover plus the whole
    // reserve), because "always promotes" (§4/§10) is a harder promise than
    // ordinary relevance's "top-k within the reserved slice".
    let retrieval_reserve = (budget as f64 * RETRIEVAL_SHARE) as u64;
    let main_budget = budget.saturating_sub(retrieval_reserve);

    // 1) Recency fill, newest -> oldest, at Full, against `main_budget`.
    // Stops at the first turn that would overflow it; every older turn is
    // left for anaphora/relevance/card/packet — never skip-scanned past a
    // gap.
    for turn in closed.iter().rev() {
        let cost = turn.card.as_ref().map(|c| c.tokens_full).unwrap_or(0);
        if spent.saturating_add(cost) <= main_budget {
            spent += cost;
            full_ids.insert(turn.id.clone());
        } else {
            break;
        }
    }

    // 2) Anaphora: the immediately preceding closed turn, regardless of
    // topic overlap, against the FULL remaining budget.
    if is_anaphoric(input.directive)
        && let Some(preceding) = closed.last()
        && !full_ids.contains(&preceding.id)
    {
        let cost = preceding.card.as_ref().map(|c| c.tokens_full).unwrap_or(0);
        if spent.saturating_add(cost) <= budget {
            spent += cost;
            full_ids.insert(preceding.id.clone());
            retrieved.push(preceding.id.clone());
        }
    }

    // 3) General relevance (lexical + reading overlap) for everything else
    // not yet promoted, inside what remains of the reserved slice.
    let mut retrieval_left = retrieval_reserve.min(budget.saturating_sub(spent));
    let mut candidates: Vec<String> = Vec::new();
    for (id, score) in input.index.search(input.directive) {
        if score > 0.0 && !full_ids.contains(&id) && !candidates.contains(&id) {
            candidates.push(id);
        }
    }
    if let Some(reading) = input.reading {
        for turn in &closed {
            let Some(card) = &turn.card else { continue };
            if !full_ids.contains(&turn.id)
                && reading_overlaps(reading, &card.reading)
                && !candidates.contains(&turn.id)
            {
                candidates.push(turn.id.clone());
            }
        }
    }
    for id in candidates {
        if full_ids.contains(&id) {
            continue;
        }
        let Some(turn) = closed.iter().find(|t| t.id == id) else {
            continue;
        };
        let cost = turn.card.as_ref().map(|c| c.tokens_full).unwrap_or(0);
        if cost <= retrieval_left && spent.saturating_add(cost) <= budget {
            spent += cost;
            retrieval_left -= cost;
            full_ids.insert(id.clone());
            retrieved.push(id);
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
        // budget = 200; retrieval_reserve = 30, main_budget = 170. The AAPL
        // turn is the immediately preceding (newest) turn but its cost
        // (180) exceeds main_budget (170), so plain recency would skip it
        // entirely; anaphora rescues it against the full remaining budget.
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
            result.retrieved.contains(&ids[1]),
            "anaphora must promote the immediately preceding turn: {:?}",
            result
        );
        assert!(result.spent <= result.budget);
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
