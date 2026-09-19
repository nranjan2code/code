#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

//! Presentations are ledger entries (docs/design/68-context-engine.md §10):
//! a validated card is written once, hash-linked like every other entry,
//! never model-visible raw, and is the single source both the display
//! channel and a later turn's history read.

use std::path::PathBuf;

use tempfile::tempdir;

use vak_llm::{ContentBlock, Message, Role};
use vak_session::SessionLog;
use vak_session::types::{
    FrozenContract, MessageRecord, PresentationRecord, PresentationSource, SessionHeader,
    canonicalize_json, payload_digest,
};

fn header() -> SessionHeader {
    SessionHeader {
        agent: None,
        session_id: "s-test".into(),
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

fn chart_payload(summary: &str) -> serde_json::Value {
    serde_json::json!({
        "chart_type": "line",
        "series": [{"name": "s1", "points": [{"x": 1, "y": 2.0}]}],
        "accessible_summary": summary,
    })
}

fn record_for(turn_id: &str, tool_use_id: &str, summary: &str) -> PresentationRecord {
    let payload = canonicalize_json(&chart_payload(summary));
    PresentationRecord {
        turn_id: turn_id.into(),
        source: PresentationSource::ToolCall {
            tool_use_id: tool_use_id.into(),
        },
        semantic_type: "chart".into(),
        skill_id: "core".into(),
        skill_version: "1.0.0".into(),
        schema_version: 1,
        payload_digest: payload_digest(&payload),
        payload,
        derived_from: Vec::new(),
        title: "Chart".into(),
        identity_digest: format!("summary: {summary}"),
    }
}

#[test]
fn a_presentation_entry_round_trips_through_jsonl() {
    let dir = tempdir().unwrap();
    let path = dir.path().join("presentation.jsonl");
    let mut log = SessionLog::create(path.clone(), header()).unwrap();
    let user = log
        .append_message(MessageRecord {
            message: Message::user_text("show me a chart"),
            meta: None,
        })
        .unwrap();
    let record = record_for(&user.id, "call-1", "rising");
    let appended = log.append_presentation(record.clone()).unwrap();
    drop(log);

    // Reopen from disk: the entry must parse back byte-identical in the
    // fields that matter, proving the JSONL round-trip.
    let reopened = SessionLog::open_read_only(path).unwrap();
    let (id, reloaded) = reopened
        .presentations()
        .into_iter()
        .next()
        .expect("presentation entry must survive a reopen");
    assert_eq!(id, appended.id);
    assert_eq!(reloaded.turn_id, record.turn_id);
    assert_eq!(reloaded.semantic_type, "chart");
    assert_eq!(reloaded.payload_digest, record.payload_digest);
    assert_eq!(reloaded.identity_digest, "summary: rising");
    match &reloaded.source {
        PresentationSource::ToolCall { tool_use_id } => assert_eq!(tool_use_id, "call-1"),
        other => panic!("expected ToolCall source, got {other:?}"),
    }
}

#[test]
fn presentations_are_returned_in_chain_order() {
    let dir = tempdir().unwrap();
    let path = dir.path().join("order.jsonl");
    let mut log = SessionLog::create(path, header()).unwrap();
    let user = log
        .append_message(MessageRecord {
            message: Message::user_text("go"),
            meta: None,
        })
        .unwrap();
    log.append_presentation(record_for(&user.id, "call-1", "first"))
        .unwrap();
    log.append_presentation(record_for(&user.id, "call-2", "second"))
        .unwrap();
    log.append_presentation(record_for(&user.id, "call-3", "third"))
        .unwrap();

    let ids: Vec<String> = log
        .presentations()
        .into_iter()
        .map(|(_, record)| match &record.source {
            PresentationSource::ToolCall { tool_use_id } => tool_use_id.clone(),
            other => panic!("expected ToolCall source, got {other:?}"),
        })
        .collect();
    assert_eq!(ids, vec!["call-1", "call-2", "call-3"]);
}

#[test]
fn has_presentation_dedupes_by_turn_and_payload_digest() {
    let dir = tempdir().unwrap();
    let path = dir.path().join("dedupe.jsonl");
    let mut log = SessionLog::create(path, header()).unwrap();
    let user = log
        .append_message(MessageRecord {
            message: Message::user_text("go"),
            meta: None,
        })
        .unwrap();
    let record = record_for(&user.id, "call-1", "same");
    let digest = record.payload_digest.clone();
    assert!(!log.has_presentation(&user.id, &digest));
    log.append_presentation(record).unwrap();
    assert!(log.has_presentation(&user.id, &digest));
    // A different turn with the same digest is not a duplicate.
    assert!(!log.has_presentation("some-other-turn", &digest));
    // A different digest in the same turn is not a duplicate either.
    assert!(!log.has_presentation(&user.id, "not-the-real-digest"));
}

#[test]
fn derive_messages_skips_presentation_entries_like_receipts() {
    let dir = tempdir().unwrap();
    let with_path = dir.path().join("with.jsonl");
    let without_path = dir.path().join("without.jsonl");

    let mut with_log = SessionLog::create(with_path, header()).unwrap();
    let mut without_log = SessionLog::create(without_path, header()).unwrap();

    for log in [&mut with_log, &mut without_log] {
        log.append_message(MessageRecord {
            message: Message::user_text("show me a chart"),
            meta: None,
        })
        .unwrap();
        log.append_message(MessageRecord {
            message: Message::assistant(vec![ContentBlock::ToolUse {
                id: "call-1".into(),
                name: "emit_chart_card".into(),
                input: serde_json::json!({"semantic_type": "chart", "payload": chart_payload("flat")}),
            }]),
            meta: None,
        })
        .unwrap();
        log.append_message(MessageRecord {
            message: Message {
                role: Role::User,
                content: vec![ContentBlock::tool_result(
                    "call-1",
                    "Card displayed to the user.",
                )],
            },
            meta: None,
        })
        .unwrap();
    }

    let user_entry = with_log.latest_directive_entry_id().unwrap();
    with_log
        .append_presentation(record_for(&user_entry, "call-1", "flat"))
        .unwrap();

    assert_eq!(
        with_log.derive_messages().len(),
        without_log.derive_messages().len(),
        "a Presentation entry must never change the model-visible projection"
    );
    // And it genuinely never appears verbatim anywhere in that projection.
    let serialized: String = with_log
        .derive_messages()
        .iter()
        .map(|m| m.text_content())
        .collect();
    assert!(!serialized.contains("identity_digest"));
}

#[test]
fn latest_directive_entry_id_ignores_control_messages() {
    let dir = tempdir().unwrap();
    let path = dir.path().join("directive.jsonl");
    let mut log = SessionLog::create(path, header()).unwrap();
    let first = log
        .append_message(MessageRecord {
            message: Message::user_text("first"),
            meta: None,
        })
        .unwrap();
    assert_eq!(log.latest_directive_entry_id(), Some(first.id.clone()));

    log.append_message(MessageRecord::control(
        vak_intent::control::ControlKind::GroundingCheck,
        "redo",
    ))
    .unwrap();
    // A runtime-authored nudge must never be mistaken for the directive.
    assert_eq!(log.latest_directive_entry_id(), Some(first.id));

    let second = log
        .append_message(MessageRecord {
            message: Message::user_text("second"),
            meta: None,
        })
        .unwrap();
    assert_eq!(log.latest_directive_entry_id(), Some(second.id.clone()));

    // A mid-turn tool-result message is `User` role and non-control, but it
    // is not a directive: it carries no `Text` block and it carries a
    // `ToolResult` block. Mistaking it for the directive breaks any
    // `turn_id` computed from it after the tool call — the fence-path
    // presentation write does exactly this
    // (docs/design/68-context-engine.md §10).
    log.append_message(MessageRecord {
        message: Message {
            role: vak_llm::Role::User,
            content: vec![vak_llm::ContentBlock::tool_result("call-1", "result")],
        },
        meta: None,
    })
    .unwrap();
    assert_eq!(log.latest_directive_entry_id(), Some(second.id));
}

#[test]
fn non_card_evidence_since_collects_prior_non_card_tool_results_only() {
    let dir = tempdir().unwrap();
    let path = dir.path().join("evidence.jsonl");
    let mut log = SessionLog::create(path, header()).unwrap();
    let user = log
        .append_message(MessageRecord {
            message: Message::user_text("what's the weather, with a chart"),
            meta: None,
        })
        .unwrap();
    // Evidence-gathering call.
    log.append_message(MessageRecord {
        message: Message::assistant(vec![ContentBlock::ToolUse {
            id: "search-1".into(),
            name: "web_search".into(),
            input: serde_json::json!({"query": "weather"}),
        }]),
        meta: None,
    })
    .unwrap();
    log.append_message(MessageRecord {
        message: Message {
            role: Role::User,
            content: vec![ContentBlock::tool_result("search-1", "72F and sunny")],
        },
        meta: None,
    })
    .unwrap();
    // Card call (not evidence).
    log.append_message(MessageRecord {
        message: Message::assistant(vec![ContentBlock::ToolUse {
            id: "call-1".into(),
            name: "emit_chart_card".into(),
            input: serde_json::json!({"semantic_type": "chart", "payload": chart_payload("flat")}),
        }]),
        meta: None,
    })
    .unwrap();
    log.append_message(MessageRecord {
        message: Message {
            role: Role::User,
            content: vec![ContentBlock::tool_result(
                "call-1",
                "Card displayed to the user.",
            )],
        },
        meta: None,
    })
    .unwrap();

    let is_card_tool = |name: &str| name == "emit_chart_card";
    let evidence = log.non_card_evidence_since(&user.id, is_card_tool);
    assert_eq!(evidence, vec!["search-1".to_string()]);
}
