#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

//! Model drift (docs/design/68-context-engine.md §7b) and the over-length
//! replan (§5): both are runtime-enforcement paths with no live model, so
//! they are covered here the same way the other repair-nudge regressions
//! are — a scripted provider driving `Agent::run` end to end.

use std::collections::VecDeque;
use std::sync::{Arc, Mutex};

use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;

use tempfile::tempdir;

use vak_agent::{Agent, AgentConfig, TurnOutcome};
use vak_llm::stream;
use vak_llm::types::{AssistantMessage, ChatRequest, ContentBlock, StopReason, Usage};
use vak_llm::{EventStream, LlmError, Provider};
use vak_session::types::{
    EntryPayload, FrozenContract, IntentRecord, MessageRecord, SessionHeader, TurnCardRecord,
};
use vak_session::{SessionLog, TurnIndex};

struct Scripted {
    responses: Mutex<VecDeque<AssistantMessage>>,
}

#[async_trait::async_trait]
impl Provider for Scripted {
    fn name(&self) -> &str {
        "scripted"
    }

    async fn stream(
        &self,
        _request: ChatRequest,
        _cancel: CancellationToken,
    ) -> Result<EventStream, LlmError> {
        let next = self.responses.lock().unwrap().pop_front();
        let (mut sink, rx) = stream::channel(64);
        match next {
            Some(m) => {
                sink.push(stream::StreamEvent::Start { partial: m.clone() });
                sink.close_message(m).await;
            }
            None => sink.close_error(LlmError::Parse("exhausted".into())).await,
        }
        Ok(rx)
    }
}

/// A provider whose first N calls reject with `LlmError::Context`, then
/// succeeds — for the over-length replan (§5).
struct OverLengthThenOk {
    context_errors_remaining: Mutex<u32>,
    final_answer: Mutex<Option<AssistantMessage>>,
}

#[async_trait::async_trait]
impl Provider for OverLengthThenOk {
    fn name(&self) -> &str {
        "scripted"
    }

    async fn stream(
        &self,
        _request: ChatRequest,
        _cancel: CancellationToken,
    ) -> Result<EventStream, LlmError> {
        let (mut sink, rx) = stream::channel(64);
        let should_reject = {
            let mut remaining = self.context_errors_remaining.lock().unwrap();
            if *remaining > 0 {
                *remaining -= 1;
                true
            } else {
                false
            }
        };
        if should_reject {
            sink.close_error(LlmError::Context("request too long for this model".into()))
                .await;
        } else {
            let msg = self
                .final_answer
                .lock()
                .unwrap()
                .take()
                .unwrap_or_else(|| text("done"));
            sink.push(stream::StreamEvent::Start {
                partial: msg.clone(),
            });
            sink.close_message(msg).await;
        }
        Ok(rx)
    }
}

fn text(t: &str) -> AssistantMessage {
    AssistantMessage {
        content: vec![ContentBlock::text(t)],
        stop_reason: StopReason::EndTurn,
        usage: Usage::default(),
        model: "test-model".into(),
        response_id: None,
    }
}

fn tool_call(id: &str, name: &str, input: serde_json::Value) -> AssistantMessage {
    AssistantMessage {
        content: vec![ContentBlock::ToolUse {
            id: id.into(),
            name: name.into(),
            input,
        }],
        stop_reason: StopReason::ToolUse,
        usage: Usage::default(),
        model: "test-model".into(),
        response_id: None,
    }
}

fn header(session_id: &str, dir: &std::path::Path) -> SessionHeader {
    SessionHeader {
        agent: None,
        session_id: session_id.into(),
        created_at: chrono::Utc::now(),
        cwd: dir.to_path_buf(),
        parent_session_id: None,
        contract_id: None,
        work_item_id: None,
        conversation: None,
        contract: FrozenContract {
            app_version: "0".into(),
            provider: "scripted".into(),
            model: "test-model".into(),
            route_ladder: Vec::new(),
            route_objective: String::new(),
            route_annotations: Vec::new(),
            system_prompt: "sys".into(),
            permission_mode: "full-access".into(),
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

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_domain_mismatched_tool_call_fires_a_steering_nudge_and_still_completes() {
    let dir = tempdir().unwrap();
    let mut log =
        SessionLog::create(dir.path().join("s.jsonl"), header("drift", dir.path())).unwrap();
    log.append_intent(intent_record(reading_with_domains(&["cooking"])))
        .unwrap();

    let provider = Arc::new(Scripted {
        responses: Mutex::new(VecDeque::from(vec![
            tool_call("c1", "bash", serde_json::json!({"command": "ls"})),
            text("done"),
        ])),
    });
    let mut cfg = AgentConfig::new("sys");
    cfg.tools = vec![Arc::new(vak_tools::bash::BashTool)];
    cfg.tool_domains = [("bash".to_string(), vec!["code-exec".to_string()])].into();
    let mut agent = Agent::new(provider, log, cfg);

    let outcome = agent
        .run(
            "please poach an egg",
            &Default::default(),
            CancellationToken::new(),
            mpsc::channel(64).0,
        )
        .await;
    assert!(
        matches!(outcome, TurnOutcome::Completed { .. }),
        "got {outcome:?}"
    );

    let session = agent.session.lock().await;
    let saw_drift_nudge = session.chain_to_root().iter().any(|e| {
        matches!(&e.payload, EntryPayload::Message(record)
            if record.control_kind() == Some(vak_intent::control::ControlKind::SteeringDrift))
    });
    assert!(
        saw_drift_nudge,
        "expected a [steering-drift] control message"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn three_consecutive_drift_events_end_the_turn_degraded() {
    let dir = tempdir().unwrap();
    let mut log =
        SessionLog::create(dir.path().join("s.jsonl"), header("drift3", dir.path())).unwrap();
    log.append_intent(intent_record(reading_with_domains(&["cooking"])))
        .unwrap();

    // Every scripted step calls the same domain-mismatched tool; the loop
    // must end the turn at the third drift event without even needing a
    // fourth scripted response.
    let provider = Arc::new(Scripted {
        responses: Mutex::new(VecDeque::from(vec![
            tool_call("c1", "bash", serde_json::json!({"command": "ls"})),
            tool_call("c2", "bash", serde_json::json!({"command": "ls"})),
            tool_call("c3", "bash", serde_json::json!({"command": "ls"})),
        ])),
    });
    let mut cfg = AgentConfig::new("sys");
    cfg.tools = vec![Arc::new(vak_tools::bash::BashTool)];
    cfg.tool_domains = [("bash".to_string(), vec!["code-exec".to_string()])].into();
    let mut agent = Agent::new(provider, log, cfg);

    let outcome = agent
        .run(
            "please poach an egg",
            &Default::default(),
            CancellationToken::new(),
            mpsc::channel(64).0,
        )
        .await;
    match outcome {
        TurnOutcome::Completed { response } => {
            assert!(
                response.text_content().contains("drifting"),
                "expected the degraded drift message, got: {}",
                response.text_content()
            );
        }
        other => panic!("expected a degraded Completed outcome, got {other:?}"),
    }

    let session = agent.session.lock().await;
    let saw_exhaustion = session.chain_to_root().iter().any(
        |e| matches!(&e.payload, EntryPayload::Activity(a) if a.label == "model-drift-exhausted"),
    );
    assert!(
        saw_exhaustion,
        "expected a model-drift-exhausted diagnostic"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_verbatim_repeat_of_a_past_answer_is_treated_as_drift_and_redone() {
    let dir = tempdir().unwrap();
    let mut log = SessionLog::create(
        dir.path().join("s.jsonl"),
        header("drift-repeat", dir.path()),
    )
    .unwrap();

    // A prior closed turn whose narration is the exact string the model
    // will later repeat verbatim for an unrelated new directive.
    let t1 = log
        .append_message(MessageRecord {
            message: vak_llm::Message::user_text("what is the capital of France"),
            meta: None,
        })
        .unwrap()
        .id;
    log.append_message(MessageRecord {
        message: vak_llm::Message::assistant(vec![ContentBlock::text("Paris is the capital.")]),
        meta: None,
    })
    .unwrap();
    let card = TurnIndex::from_log(&log)
        .turn_by_id(&t1)
        .unwrap()
        .build_card("completed", "Paris is the capital.".to_string(), &|s| {
            s.len() as u64 / 4
        });
    log.append_turn_card(TurnCardRecord { turn_id: t1, card })
        .unwrap();

    let provider = Arc::new(Scripted {
        responses: Mutex::new(VecDeque::from(vec![
            text("Paris is the capital."),
            text("Tokyo has about 14 million people."),
        ])),
    });
    let cfg = AgentConfig::new("sys");
    let mut agent = Agent::new(provider, log, cfg);

    let outcome = agent
        .run(
            "how many people live in Tokyo",
            &Default::default(),
            CancellationToken::new(),
            mpsc::channel(64).0,
        )
        .await;
    match outcome {
        TurnOutcome::Completed { response } => {
            assert_eq!(
                response.text_content(),
                "Tokyo has about 14 million people."
            );
        }
        other => panic!("expected the redone answer to complete, got {other:?}"),
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn over_length_rejection_lowers_the_horizon_and_retries_once() {
    let dir = tempdir().unwrap();
    let log =
        SessionLog::create(dir.path().join("s.jsonl"), header("overlength", dir.path())).unwrap();

    let provider = Arc::new(OverLengthThenOk {
        context_errors_remaining: Mutex::new(1),
        final_answer: Mutex::new(Some(text("recovered after replan"))),
    });
    let cfg = AgentConfig::new("sys");
    let mut agent = Agent::new(provider, log, cfg);

    let outcome = agent
        .run(
            "hello",
            &Default::default(),
            CancellationToken::new(),
            mpsc::channel(64).0,
        )
        .await;
    match outcome {
        TurnOutcome::Completed { response } => {
            assert_eq!(response.text_content(), "recovered after replan");
        }
        other => panic!("expected recovery after one replanned retry, got {other:?}"),
    }

    assert!(
        agent.config.capacity.is_some(),
        "the over-length feedback must persist a CapacityProfile"
    );
    let profile = agent.config.capacity.as_ref().unwrap();
    assert!(
        profile.verified_window.is_some(),
        "verified_window must be set from the rejection"
    );

    let session = agent.session.lock().await;
    let saw_feedback = session.chain_to_root().iter().any(
        |e| matches!(&e.payload, EntryPayload::Activity(a) if a.label.contains("over-length")),
    );
    assert!(saw_feedback, "expected a capacity-feedback activity");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_second_consecutive_over_length_rejection_fails_the_turn() {
    let dir = tempdir().unwrap();
    let log = SessionLog::create(
        dir.path().join("s.jsonl"),
        header("overlength2", dir.path()),
    )
    .unwrap();

    let provider = Arc::new(OverLengthThenOk {
        // Never runs out: every call rejects.
        context_errors_remaining: Mutex::new(u32::MAX),
        final_answer: Mutex::new(None),
    });
    let mut cfg = AgentConfig::new("sys");
    cfg.run_retry_attempts = 0;
    let mut agent = Agent::new(provider, log, cfg);

    let outcome = agent
        .run(
            "hello",
            &Default::default(),
            CancellationToken::new(),
            mpsc::channel(64).0,
        )
        .await;
    match outcome {
        TurnOutcome::Failed { error } => {
            assert!(matches!(error, LlmError::Context(_)), "got: {error}");
        }
        other => panic!("expected the second Context error to fail the turn, got {other:?}"),
    }
}
