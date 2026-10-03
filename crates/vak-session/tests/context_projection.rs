#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

//! Projection properties of the context engine
//! (docs/design/68-context-engine.md §4, §6, §10).

use tempfile::tempdir;
use vak_llm::{ContentBlock, Message, Role};
use vak_session::types::{
    FrozenContract, IntentRecord, MessageRecord, SessionHeader, TurnCardRecord,
};
use vak_session::{Fidelity, SessionLog, TurnIndex, WorkingSetPlan};

fn open(dir: &std::path::Path) -> SessionLog {
    let header = SessionHeader {
        space: None,
        run: None,
        cause: None,
        agent: None,
        session_id: "s-proj".into(),
        created_at: chrono::Utc::now(),
        cwd: dir.to_path_buf(),
        parent_session_id: None,
        contract_id: None,
        work_item_id: None,
        conversation: None,
        contract: FrozenContract {
            app_version: "0.1.0".into(),
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
    };
    SessionLog::create(dir.join("s.jsonl"), header).unwrap()
}

fn rec(m: Message) -> MessageRecord {
    MessageRecord {
        message: m,
        meta: None,
    }
}

fn text(m: &Message) -> String {
    m.content
        .iter()
        .map(|b| match b {
            ContentBlock::Text { text } => text.clone(),
            ContentBlock::ToolResult { content, .. } => content.clone(),
            ContentBlock::ToolUse { input, .. } => input.to_string(),
            _ => String::new(),
        })
        .collect::<Vec<_>>()
        .join(" ")
}

fn close(log: &mut SessionLog, id: &str) {
    let narration = log
        .derive_messages()
        .last()
        .map(|m| m.text_content())
        .unwrap_or_default();
    let card =
        TurnIndex::from_log(log)
            .turn_by_id(id)
            .unwrap()
            .build_card("completed", narration, &|s| s.len() as u64 / 4);
    log.append_turn_card(TurnCardRecord {
        turn_id: id.into(),
        card,
    })
    .unwrap();
}

fn turn(log: &mut SessionLog, q: &str, a: &str) -> String {
    let id = log.append_message(rec(Message::user_text(q))).unwrap().id;
    log.append_message(rec(Message::assistant(vec![ContentBlock::text(a)])))
        .unwrap();
    close(log, &id);
    id
}

fn plan(full: &[&String]) -> WorkingSetPlan {
    WorkingSetPlan {
        per_turn: full
            .iter()
            .map(|id| ((*id).clone(), Fidelity::Full))
            .collect(),
        ..Default::default()
    }
}

#[test]
fn full_turns_stay_byte_identical_as_card_lines_accumulate() {
    let dir = tempdir().unwrap();
    let mut log = open(dir.path());
    let full = turn(&mut log, "alpha budget numbers", "alpha answer body");
    turn(&mut log, "bravo one", "b1");
    log.append_message(rec(Message::user_text("charlie two")))
        .unwrap();
    let before = log.derive_with_plan(&plan(&[&full]));
    log.append_message(rec(Message::assistant(vec![ContentBlock::text("c2")])))
        .unwrap();
    let id = TurnIndex::from_log(&log).turns.last().unwrap().id.clone();
    close(&mut log, &id);
    log.append_message(rec(Message::user_text("delta three")))
        .unwrap();
    let after = log.derive_with_plan(&plan(&[&full]));
    assert_eq!(
        text(&before[0]),
        text(&after[0]),
        "the Full record leads both requests"
    );
    assert_eq!(text(&before[1]), text(&after[1]));
    assert!(
        text(&after[after.len() - 2]).starts_with("<turns>"),
        "card block sits just before the open turn"
    );
    assert!(
        text(&after[after.len() - 2]).contains("charlie"),
        "and now carries the new card"
    );
}

#[test]
fn a_cancelled_turn_does_not_take_the_next_turns_reading() {
    let dir = tempdir().unwrap();
    let mut log = open(dir.path());
    let intent = |marker: &str| {
        let mut reading = vak_intent::Reading::general();
        reading.domains.insert(marker.to_string());
        let outcome = vak_intent::OutcomeSpec {
            schema_version: 1,
            revision: 0,
            objective: "task".into(),
            assumptions: Vec::new(),
            requirements: Vec::new(),
            resolver_version: 1,
            evidence_max_age_secs: None,
            max_turns: None,
            acts: Default::default(),
            stop: Default::default(),
        };
        IntentRecord {
            reading,
            engagement: vak_intent::Engagement::general(),
            provenance: vak_intent::Provenance::new(
                vak_intent::Tier::Signals,
                vak_intent::RESOLVER_VERSION,
                Vec::new(),
            ),
            outcome: Some(outcome),
            model_visible: None,
            commitment_id: None,
            strands: Vec::new(),
            strand_commitments: Default::default(),
        }
    };
    log.begin_turn(&uuid::Uuid::now_v7().to_string());
    log.append_intent(intent("one")).unwrap();
    let t1 = log
        .append_message(rec(Message::user_text("first task")))
        .unwrap()
        .id;
    log.append_message(rec(Message::assistant(vec![ContentBlock::ToolUse {
        id: "a".into(),
        name: "bash".into(),
        input: serde_json::json!({}),
    }])))
    .unwrap();
    log.append_message(rec(Message {
        role: Role::User,
        content: vec![ContentBlock::tool_result("a", "x")],
    }))
    .unwrap();
    log.begin_turn(&uuid::Uuid::now_v7().to_string());
    log.append_intent(intent("two")).unwrap();
    let t2 = log
        .append_message(rec(Message::user_text("second task")))
        .unwrap()
        .id;
    log.append_message(rec(Message::assistant(vec![ContentBlock::text("done")])))
        .unwrap();
    let index = TurnIndex::from_log(&log);
    let domains = |id: &str| {
        index
            .turn_by_id(id)
            .unwrap()
            .build_card("x", String::new(), &|_| 0)
            .reading
            .domains
    };
    assert_eq!(domains(&t1), vec!["one".to_string()]);
    assert_eq!(domains(&t2), vec!["two".to_string()]);
}

#[test]
fn a_closed_record_drops_the_assistants_narration_between_calls() {
    let dir = tempdir().unwrap();
    let mut log = open(dir.path());
    let id = log
        .append_message(rec(Message::user_text("look it up")))
        .unwrap()
        .id;
    log.append_message(rec(Message::assistant(vec![
        ContentBlock::text("Let me search for that."),
        ContentBlock::ToolUse {
            id: "c1".into(),
            name: "webfetch".into(),
            input: serde_json::json!({"url": "u"}),
        },
    ])))
    .unwrap();
    log.append_message(rec(Message {
        role: Role::User,
        content: vec![ContentBlock::tool_result("c1", "page body")],
    }))
    .unwrap();
    log.append_message(rec(Message::assistant(vec![ContentBlock::text(
        "The answer.",
    )])))
    .unwrap();
    let record = TurnIndex::from_log(&log)
        .turn_by_id(&id)
        .unwrap()
        .full_record();
    let all = record.iter().map(text).collect::<Vec<_>>().join("\n");
    assert!(!all.contains("Let me search"), "{all}");
    assert!(all.contains("The answer.") && all.contains("\"url\""));
}

#[test]
fn search_needs_half_of_the_subject_and_weighs_rare_words() {
    let dir = tempdir().unwrap();
    let mut log = open(dir.path());
    let weather = turn(&mut log, "Noida weather forecast for the week", "sunny");
    let report = turn(&mut log, "quarterly report weather impact on sales", "down");
    turn(&mut log, "list the report items", "ok");
    log.append_message(rec(Message::user_text("x"))).unwrap();
    let index = TurnIndex::from_log(&log);
    let hits: Vec<String> = index
        .search("Noida weather")
        .into_iter()
        .map(|(id, _)| id)
        .collect();
    assert_eq!(
        hits.first(),
        Some(&weather),
        "the rare word leads: {hits:?}"
    );
    assert!(
        index.search("noida budget quarterly migration").is_empty(),
        "one word of four is not relevance"
    );
    assert!(!index.search("report").iter().any(|(id, _)| id == &weather));
    let _ = report;
}

#[test]
fn a_packet_names_the_turns_it_stands_in_for() {
    let dir = tempdir().unwrap();
    let mut log = open(dir.path());
    let a = turn(&mut log, "one", "1");
    let b = turn(&mut log, "two", "2");
    turn(&mut log, "three", "3");
    log.append_packet(&a, &b, "m", "SUMMARY".into(), 10)
        .unwrap();
    let plan = WorkingSetPlan {
        packet_range: Some((a, b)),
        ..Default::default()
    };
    let first = text(&log.derive_with_plan(&plan)[0]);
    assert!(
        first.contains("turns 1-2") && first.contains("SUMMARY"),
        "{first}"
    );
}
