//! The working set (docs/design/68-context-engine.md §4): the open turn
//! verbatim, then as many of the newest visible settled exchanges as the
//! measured budget holds, each whole under a historical header with its
//! address and how it ended. What no longer fits is one stored summary of
//! everything older. Recency and measured size choose, never words, threads,
//! file names or graph scores. Pure functions; Core and the broker own
//! retrieval and access checks.

use vak_session::SessionLog;
pub use vak_session::{Fidelity, WorkingSetPlan};

use crate::assemble::messages_chars;
use crate::capacity::CapacityProfile;

/// One visible settled exchange the window may carry.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Exchange {
    pub turn_id: String,
    /// Its closing `TurnCard` record, the address a recorded selection keeps.
    pub closing_id: String,
}

/// A summary stored in the ledger, with its measured size.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StoredSummary {
    pub first_turn_id: String,
    pub last_turn_id: String,
    pub tokens: u64,
}

/// Plans one request over `exchanges`, newest first.
///
/// 1. Every exchange whole, newest first, while it fits.
/// 2. If some do not fit, the stored summary that starts at the oldest
///    exchange and leaves the most exchanges whole, when everything newer
///    than it fits whole beside it.
/// 3. Otherwise a summary of everything but the newest exchange, which rides
///    whole when it fits, else as its compact card, else not at all. Until
///    that summary is written the plan names its range and the loop writes
///    it; it then stays the boundary until the window fills again, so the
///    request between two summaries only grows at its end.
///
/// `full_cost` measures an exchange as historical dialogue (`None` when it
/// cannot be rebuilt, which ends the run of whole exchanges); `card_cost`
/// measures its compact card. Both are called only as far as needed.
pub fn fill_window(
    budget: u64,
    exchanges: &[Exchange],
    summaries: &[StoredSummary],
    full_cost: &mut dyn FnMut(usize) -> Option<u64>,
    card_cost: &mut dyn FnMut(usize) -> u64,
) -> WorkingSetPlan {
    let mut plan = WorkingSetPlan {
        budget,
        selected_records: Some(Vec::new()),
        ..Default::default()
    };
    let Some(oldest) = exchanges.last() else {
        return plan;
    };
    // Running totals of the newest exchanges taken whole.
    let mut whole: Vec<u64> = Vec::new();
    let mut spent = 0u64;
    while whole.len() < exchanges.len() {
        match full_cost(whole.len()) {
            Some(cost) if spent.saturating_add(cost) <= budget => {
                spent = spent.saturating_add(cost);
                whole.push(spent);
            }
            _ => break,
        }
    }
    let total = |count: usize| count.checked_sub(1).map_or(0, |last| whole[last]);
    if whole.len() == exchanges.len() {
        take(&mut plan, exchanges, exchanges.len(), None);
        plan.spent = total(exchanges.len());
        return plan;
    }
    // A stored summary ending at exchange `k` covers `k..` and leaves the
    // `k` newer exchanges whole; the fewest covered wins.
    let reusable = summaries
        .iter()
        .filter(|summary| summary.first_turn_id == oldest.turn_id)
        .filter_map(|summary| {
            let k = exchanges
                .iter()
                .position(|exchange| exchange.turn_id == summary.last_turn_id)?;
            (k <= whole.len() && total(k).saturating_add(summary.tokens) <= budget)
                .then_some((k, summary))
        })
        .max_by_key(|(k, _)| *k);
    if let Some((k, summary)) = reusable {
        take(&mut plan, exchanges, k, None);
        plan.packet_range = Some((summary.first_turn_id.clone(), summary.last_turn_id.clone()));
        plan.spent = total(k).saturating_add(summary.tokens);
        return plan;
    }
    let newest_room = if exchanges.len() >= 2 {
        let range = (oldest.turn_id.clone(), exchanges[1].turn_id.clone());
        let stored = summaries
            .iter()
            .find(|summary| summary.first_turn_id == range.0 && summary.last_turn_id == range.1)
            .map_or(0, |summary| summary.tokens);
        plan.packet_range = Some(range);
        plan.spent = stored;
        budget.saturating_sub(stored)
    } else {
        budget
    };
    if let Some(cost) = full_cost(0).filter(|cost| *cost <= newest_room) {
        take(&mut plan, exchanges, 1, None);
        plan.spent = plan.spent.saturating_add(cost);
    } else {
        let cost = card_cost(0);
        if cost <= newest_room {
            take(&mut plan, exchanges, 0, Some(&exchanges[0]));
            plan.spent = plan.spent.saturating_add(cost);
        }
    }
    plan
}

/// Puts the `count` newest exchanges into `plan` whole, oldest first, then
/// `card` (the newest exchange) as its compact card when given.
fn take(plan: &mut WorkingSetPlan, exchanges: &[Exchange], count: usize, card: Option<&Exchange>) {
    let mut records = Vec::new();
    for exchange in exchanges[..count].iter().rev() {
        plan.per_turn
            .push((exchange.turn_id.clone(), Fidelity::Full));
        plan.recent_dialogue.push(exchange.turn_id.clone());
        records.push((exchange.turn_id.clone(), exchange.closing_id.clone()));
    }
    if let Some(exchange) = card {
        plan.per_turn
            .push((exchange.turn_id.clone(), Fidelity::Card));
        records.push((exchange.turn_id.clone(), exchange.closing_id.clone()));
    }
    plan.selected_records = Some(records);
}

/// The production entry point: the loop, `/compact` and the eval gate all
/// plan through it. Measures the open turn, then each visible exchange
/// newest first only until the window is full, so the work is bounded by
/// what the request carries, not by the conversation's length.
pub fn plan_for_session(
    log: &SessionLog,
    profile: &CapacityProfile,
    prefix_tokens: u64,
    tail_tokens: u64,
) -> WorkingSetPlan {
    let current = profile.estimate_tokens(messages_chars(&log.open_turn_verbatim()));
    let budget = profile.budget(
        prefix_tokens,
        tail_tokens,
        profile.current_turn_reserve(current),
    );
    let visible: Vec<(String, String, u64)> = log
        .visible_turn_cards_rev()
        .map(|(closing, card)| {
            (
                card.turn_id.clone(),
                closing.to_string(),
                profile.estimate_tokens(card.addressed_message().len() as u64),
            )
        })
        .collect();
    let exchanges: Vec<Exchange> = visible
        .iter()
        .map(|(turn_id, closing_id, _)| Exchange {
            turn_id: turn_id.clone(),
            closing_id: closing_id.clone(),
        })
        .collect();
    let summaries: Vec<StoredSummary> = log
        .stored_packets_rev()
        .into_iter()
        .map(|packet| StoredSummary {
            tokens: profile.estimate_tokens(packet.summary.len() as u64),
            first_turn_id: packet.first_turn_id,
            last_turn_id: packet.last_turn_id,
        })
        .collect();
    fill_window(
        budget,
        &exchanges,
        &summaries,
        &mut |i| {
            let exchange = &exchanges[i];
            log.turn_dialogue(&exchange.turn_id, &exchange.closing_id)
                .map(|messages| profile.estimate_tokens(messages_chars(&messages)))
        },
        &mut |i| visible[i].2,
    )
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::*;
    use crate::capacity::{CacheBehaviour, CapacityProfile, Horizon, ProbeProvenance};
    use vak_llm::Message as M;
    use vak_session::types::{FrozenContract, MessageRecord, SessionHeader, TurnCardRecord};
    use vak_session::{SessionPath, TurnIndex};

    fn exchanges(n: usize) -> Vec<Exchange> {
        (0..n)
            .map(|i| Exchange {
                turn_id: format!("t{i}"),
                closing_id: format!("c{i}"),
            })
            .collect()
    }

    /// `costs[i]` is the whole cost of exchange `i`, newest first; a card
    /// costs one.
    fn window(budget: u64, costs: &[u64], summaries: &[StoredSummary]) -> WorkingSetPlan {
        let all = exchanges(costs.len());
        fill_window(budget, &all, summaries, &mut |i| Some(costs[i]), &mut |_| 1)
    }

    fn summary(first: &str, last: &str, tokens: u64) -> StoredSummary {
        StoredSummary {
            first_turn_id: first.into(),
            last_turn_id: last.into(),
            tokens,
        }
    }

    fn whole(ids: &[&str]) -> Vec<(String, Fidelity)> {
        ids.iter()
            .map(|id| (id.to_string(), Fidelity::Full))
            .collect()
    }

    #[test]
    fn every_exchange_rides_whole_while_it_fits_whatever_its_subject() {
        let plan = window(100, &[10, 20, 30], &[]);
        assert_eq!(plan.per_turn, whole(&["t2", "t1", "t0"]));
        assert_eq!(plan.recent_dialogue, vec!["t2", "t1", "t0"]);
        assert_eq!(plan.packet_range, None);
        assert_eq!(plan.spent, 60);
        assert_eq!(
            plan.selected_records.unwrap(),
            vec![
                ("t2".to_string(), "c2".to_string()),
                ("t1".into(), "c1".into()),
                ("t0".into(), "c0".into())
            ]
        );
    }

    #[test]
    fn an_overflowing_window_asks_for_a_summary_of_all_but_the_newest() {
        let plan = window(50, &[10, 20, 30, 5], &[]);
        assert_eq!(plan.packet_range, Some(("t3".into(), "t1".into())));
        assert_eq!(plan.per_turn, whole(&["t0"]));
    }

    #[test]
    fn a_stored_summary_stays_the_boundary_while_newer_exchanges_fit() {
        // Written when t3..t1 overflowed; two newer exchanges have come since.
        let stored = [summary("t5", "t3", 8)];
        let plan = window(50, &[10, 12, 9, 30, 30, 30], &stored);
        assert_eq!(plan.packet_range, Some(("t5".into(), "t3".into())));
        assert_eq!(plan.per_turn, whole(&["t2", "t1", "t0"]));
        assert_eq!(plan.spent, 39);
    }

    #[test]
    fn the_stored_summary_that_keeps_the_most_exchanges_whole_wins() {
        let stored = [summary("t3", "t1", 5), summary("t3", "t2", 6)];
        let plan = window(40, &[10, 10, 30, 30], &stored);
        assert_eq!(plan.packet_range, Some(("t3".into(), "t2".into())));
        assert_eq!(plan.per_turn, whole(&["t1", "t0"]));
    }

    #[test]
    fn a_full_window_after_a_summary_asks_for_a_wider_one() {
        let stored = [summary("t4", "t3", 8)];
        let plan = window(40, &[20, 20, 20, 9, 9], &stored);
        assert_eq!(plan.packet_range, Some(("t4".into(), "t1".into())));
        assert_eq!(plan.per_turn, whole(&["t0"]));
    }

    #[test]
    fn the_newest_exchange_falls_to_its_card_then_to_nothing() {
        let stored = [summary("t1", "t1", 30)];
        let plan = window(35, &[20, 20], &stored);
        assert_eq!(plan.packet_range, Some(("t1".into(), "t1".into())));
        assert_eq!(plan.per_turn, vec![("t0".to_string(), Fidelity::Card)]);
        assert!(plan.recent_dialogue.is_empty());

        let plan = window(0, &[20], &[]);
        assert!(plan.per_turn.is_empty());
        assert_eq!(plan.packet_range, None);
        assert_eq!(plan.selected_records, Some(Vec::new()));
    }

    #[test]
    fn measuring_stops_at_the_first_exchange_that_does_not_fit() {
        let all = exchanges(1000);
        let mut measured = 0;
        let plan = fill_window(
            30,
            &all,
            &[],
            &mut |_| {
                measured += 1;
                Some(10)
            },
            &mut |_| 1,
        );
        assert_eq!(plan.packet_range, Some(("t999".into(), "t1".into())));
        assert!(measured <= 5, "measured {measured} exchanges");
    }

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

    fn closed_turn(log: &mut SessionLog, asked: &str, answered: &str) -> String {
        let id = log
            .append_message(MessageRecord {
                message: M::user_text(asked),
                meta: None,
            })
            .unwrap()
            .id;
        log.append_message(MessageRecord {
            message: M::assistant(vec![vak_llm::ContentBlock::text(answered)]),
            meta: None,
        })
        .unwrap();
        let card = TurnIndex::from_log(log)
            .turn_by_id(&id)
            .unwrap()
            .build_card("completed", answered.into(), &|s| s.len() as u64 / 4);
        log.append_turn_card(TurnCardRecord {
            turn_id: id.clone(),
            card,
        })
        .unwrap();
        id
    }

    #[test]
    fn a_subject_before_twelve_side_questions_stays_in_the_request() {
        let dir = tempfile::tempdir().unwrap();
        let path = SessionPath::new_session_file(dir.path(), dir.path(), "s1");
        let mut log = SessionLog::create(path, header(dir.path())).unwrap();
        let aster = closed_turn(
            &mut log,
            "Analyze the Aster observations: first=13, second=18, third=16.",
            "The biggest change is +5, from the first to the second.",
        );
        for n in 1..=12 {
            closed_turn(
                &mut log,
                &format!("Independent side question {n}: what is 2 plus 3?"),
                "5",
            );
        }
        log.append_message(MessageRecord {
            message: M::user_text("Return to what we were working on and make it clearer."),
            meta: None,
        })
        .unwrap();
        let plan = plan_for_session(&log, &profile(20_000), 0, 0);
        assert_eq!(plan.per_turn.len(), 13);
        assert_eq!(plan.per_turn[0], (aster.clone(), Fidelity::Full));
        let rendered = log.derive_selected_checked(&plan).unwrap();
        let text: Vec<String> = rendered.iter().map(M::text_content).collect();
        assert!(
            text.iter()
                .any(|t| t.contains("first=13, second=18, third=16"))
        );
        assert_eq!(
            text.iter()
                .filter(|t| t.contains("<historical_exchange>"))
                .count(),
            13
        );
        assert_eq!(
            text.last().unwrap(),
            "Return to what we were working on and make it clearer."
        );
    }
}
