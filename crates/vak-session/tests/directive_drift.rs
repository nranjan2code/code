#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

//! Directive drift (docs/design/68-context-engine.md §7): when the thread
//! lists an earlier directive whose reading shares no domain with the
//! current one, it is marked "(earlier, now paused)" — a re-weighting, not
//! a cut. The directive itself still appears; nothing is removed.

use std::path::PathBuf;

use tempfile::tempdir;

use vak_llm::{ContentBlock, Message};
use vak_session::types::{
    FrozenContract, IntentRecord, MessageRecord, SessionHeader, TurnCardRecord,
};
use vak_session::{SessionLog, TurnIndex, WorkingSetPlan};

/// A plan that packets exactly `turn_id` (and nothing else), with the
/// packet already written — the state in which the thread has an earlier
/// directive to list.
fn packet_only(log: &mut SessionLog, turn_id: &str, summary: &str) -> WorkingSetPlan {
    log.append_packet(turn_id, turn_id, "fixture-model", summary.into(), 999)
        .unwrap();
    WorkingSetPlan {
        packet_range: Some((turn_id.to_string(), turn_id.to_string())),
        ..WorkingSetPlan::default()
    }
}

fn header() -> SessionHeader {
    SessionHeader {
        agent: None,
        session_id: "s-drift".into(),
        created_at: chrono::Utc::now(),
        cwd: PathBuf::from("/tmp/proj"),
        parent_session_id: None,
        contract_id: None,
        work_item_id: None,
        conversation: None,
        contract: FrozenContract {
            app_version: "0.1.0".into(),
            provider: "anthropic".into(),
            model: "claude-sonnet-4-5".into(),
            route_ladder: Vec::new(),
            route_objective: String::new(),
            route_annotations: Vec::new(),
            system_prompt: "system prompt v1".into(),
            permission_mode: "workspace-write".into(),
            capabilities: Vec::new(),
            prompt_layers: Vec::new(),
        },
    }
}

fn reading_with_domains(domains: &[&str]) -> vak_intent::Reading {
    vak_intent::Reading {
        domains: domains.iter().map(|d| d.to_string()).collect(),
        ..vak_intent::Reading::general()
    }
}

fn user_msg(text: &str) -> MessageRecord {
    MessageRecord {
        message: Message::user_text(text),
        meta: None,
    }
}

fn assistant_msg(text: &str) -> MessageRecord {
    MessageRecord {
        message: Message::assistant(vec![ContentBlock::text(text)]),
        meta: None,
    }
}

fn intent_record(reading: vak_intent::Reading) -> IntentRecord {
    IntentRecord {
        reading,
        engagement: vak_intent::Engagement::general(),
        provenance: vak_intent::Provenance::new(vak_intent::Tier::General, 1, Vec::new()),
        outcome: None,
        model_visible: None,
        commitment_id: None,
    }
}

#[test]
fn a_domain_disjoint_directive_is_marked_paused_not_removed() {
    let dir = tempdir().unwrap();
    let mut log = SessionLog::create(dir.path().join("s.jsonl"), header()).unwrap();

    // Turn 1: finance topic, closed with a card carrying that reading.
    log.append_goal_update(vak_intent::GoalUpdate {
        revision: 1,
        relation: vak_intent::GoalRelation::New,
        request: "look up the sensex".into(),
        supersedes_revision: None,
    })
    .unwrap();
    let t1 = log
        .append_message(user_msg("look up the sensex"))
        .unwrap()
        .id;
    log.append_intent(intent_record(reading_with_domains(&["finance"])))
        .unwrap();
    log.append_message(assistant_msg("sensex is up")).unwrap();
    let card1 = TurnIndex::from_log(&log)
        .turn_by_id(&t1)
        .unwrap()
        .build_card("completed", "sensex is up".to_string(), &|s| {
            s.len() as u64 / 4
        });
    log.append_turn_card(TurnCardRecord {
        turn_id: t1.clone(),
        card: card1,
    })
    .unwrap();

    // Turn 2: unrelated cooking topic — the current directive.
    log.append_goal_update(vak_intent::GoalUpdate {
        revision: 2,
        relation: vak_intent::GoalRelation::AddsTo,
        request: "how do I poach an egg".into(),
        supersedes_revision: None,
    })
    .unwrap();
    log.append_message(user_msg("how do I poach an egg"))
        .unwrap();
    log.append_intent(intent_record(reading_with_domains(&["cooking"])))
        .unwrap();

    // Packet turn 1 away under the plan so the thread has something to
    // list.
    let plan = packet_only(&mut log, &t1, "summary of turn 1");

    let thread = log
        .tail_sections(Some(&plan))
        .thread
        .expect("thread must list the dropped finance directive");
    assert!(thread.contains("look up the sensex"), "{thread}");
    assert!(
        thread.contains("look up the sensex (earlier, now paused)"),
        "a domain-disjoint earlier directive must be marked paused: {thread}"
    );
}

#[test]
fn a_domain_overlapping_directive_is_never_marked_paused() {
    let dir = tempdir().unwrap();
    let mut log = SessionLog::create(dir.path().join("s.jsonl"), header()).unwrap();

    log.append_goal_update(vak_intent::GoalUpdate {
        revision: 1,
        relation: vak_intent::GoalRelation::New,
        request: "look up the sensex".into(),
        supersedes_revision: None,
    })
    .unwrap();
    let t1 = log
        .append_message(user_msg("look up the sensex"))
        .unwrap()
        .id;
    log.append_intent(intent_record(reading_with_domains(&["finance"])))
        .unwrap();
    log.append_message(assistant_msg("sensex is up")).unwrap();
    let card1 = TurnIndex::from_log(&log)
        .turn_by_id(&t1)
        .unwrap()
        .build_card("completed", "sensex is up".to_string(), &|s| {
            s.len() as u64 / 4
        });
    log.append_turn_card(TurnCardRecord {
        turn_id: t1.clone(),
        card: card1,
    })
    .unwrap();

    // Turn 2: same domain (finance) — still active, never paused.
    log.append_goal_update(vak_intent::GoalUpdate {
        revision: 2,
        relation: vak_intent::GoalRelation::AddsTo,
        request: "and the nifty too".into(),
        supersedes_revision: None,
    })
    .unwrap();
    log.append_message(user_msg("and the nifty too")).unwrap();
    log.append_intent(intent_record(reading_with_domains(&["finance"])))
        .unwrap();

    let plan = packet_only(&mut log, &t1, "summary of turn 1");

    let thread = log
        .tail_sections(Some(&plan))
        .thread
        .expect("thread present");
    assert!(
        !thread.contains("(earlier, now paused)"),
        "same-domain directives must not be marked paused: {thread}"
    );
}
