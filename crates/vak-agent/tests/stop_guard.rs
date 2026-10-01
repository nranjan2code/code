#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::collections::VecDeque;
use std::sync::{Arc, Mutex};

use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;

use tempfile::tempdir;

use vak_agent::{Agent, AgentConfig, AgentEvent, StopPolicy, TurnOutcome};
use vak_llm::stream;
use vak_llm::types::{AssistantMessage, ChatRequest, ContentBlock, StopReason, Usage};
use vak_llm::{EventStream, LlmError, Provider};
use vak_session::SessionLog;
use vak_session::types::{FrozenContract, SessionHeader};

enum ScriptedResponse {
    Message(AssistantMessage),
    #[allow(dead_code)]
    Error(LlmError),
}

struct Scripted {
    responses: Mutex<VecDeque<ScriptedResponse>>,
    requests: Arc<Mutex<Vec<ChatRequest>>>,
}

#[async_trait::async_trait]
impl Provider for Scripted {
    fn name(&self) -> &str {
        "scripted"
    }

    async fn stream(
        &self,
        request: ChatRequest,
        _cancel: CancellationToken,
    ) -> Result<EventStream, LlmError> {
        self.requests.lock().unwrap().push(request);
        let next = self.responses.lock().unwrap().pop_front();
        let (mut sink, rx) = stream::channel(64);
        match next {
            Some(ScriptedResponse::Message(m)) => {
                sink.push(stream::StreamEvent::Start { partial: m.clone() });
                sink.close_message(m).await;
            }
            Some(ScriptedResponse::Error(e)) => sink.close_error(e).await,
            None => {
                sink.close_error(LlmError::Parse("script exhausted".into()))
                    .await
            }
        }
        Ok(rx)
    }
}

fn text_msg(text: &str) -> AssistantMessage {
    AssistantMessage {
        content: vec![ContentBlock::text(text)],
        stop_reason: StopReason::EndTurn,
        usage: Usage {
            input_tokens: 10,
            output_tokens: 5,
            ..Default::default()
        },
        model: "test-model".into(),
        response_id: None,
    }
}

struct Harness {
    agent: Option<Agent>,
    requests: Arc<Mutex<Vec<ChatRequest>>>,
    events_tx: mpsc::Sender<AgentEvent>,
    events_rx: mpsc::Receiver<AgentEvent>,
}

fn harness(policy: Option<StopPolicy>, responses: Vec<ScriptedResponse>) -> Harness {
    let dir = tempdir().expect("tempdir");
    let header = SessionHeader {
        space: None,
        run: None,
        cause: None,
        agent: None,
        session_id: "s-stop".into(),
        created_at: chrono::Utc::now(),
        cwd: dir.path().to_path_buf(),
        parent_session_id: None,
        contract_id: None,
        work_item_id: None,
        conversation: None,
        contract: FrozenContract {
            app_version: "0.1.0".into(),
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
    };
    let log = SessionLog::create(dir.path().join("s.jsonl"), header).expect("ledger");
    let mut cfg = AgentConfig::new("sys");
    cfg.stop_policy = policy;
    let requests: Arc<Mutex<Vec<ChatRequest>>> = Arc::new(Mutex::new(Vec::new()));
    let provider = Arc::new(Scripted {
        responses: Mutex::new(responses.into_iter().collect()),
        requests: requests.clone(),
    });
    let (tx, rx) = mpsc::channel(4096);
    std::mem::forget(dir);
    Harness {
        agent: Some(Agent::new(provider, log, cfg)),
        requests,
        events_tx: tx,
        events_rx: rx,
    }
}

fn drain_events(h: &mut Harness) -> Vec<AgentEvent> {
    let mut out = Vec::new();
    while let Ok(ev) = h.events_rx.try_recv() {
        out.push(ev);
    }
    out
}

// Raw ledger, not the model-visible projection: a stop-guard nudge is
// mid-turn scaffolding, dropped once the turn actually closes
// (docs/design/68-context-engine.md §10). This only checks the nudge was
// RECORDED — being recorded does not by itself mean it reached the model;
// that depends on the still-open turn projecting verbatim rather than
// being (incorrectly) treated as already closed. See
// `TurnIndex::from_log`'s `closed` derivation and
// `a_draft_followed_by_a_control_nudge_with_no_reply_yet_stays_open` in
// vak-session/src/turns.rs for the actual delivery guarantee.
async fn guard_messages(h: &mut Harness) -> Vec<String> {
    let agent = h.agent.take().expect("guard_messages consumes the agent");
    let session = agent.into_session().await;
    session
        .message_chain()
        .into_iter()
        .filter(|(_, m)| m.role == vak_llm::Role::User)
        .filter_map(|(_, m)| match m.content.first() {
            Some(ContentBlock::Text { text }) => Some(text.clone()),
            _ => None,
        })
        .collect()
}

#[tokio::test]
async fn truncated_plan_gets_one_continuation_then_completes() {
    let mut h = harness(
        Some(StopPolicy {
            marker_gate: true,
            verify_gate: false,
            max_blocks: 2,
        }),
        vec![
            ScriptedResponse::Message(text_msg("Fixing both:")),
            ScriptedResponse::Message(text_msg("All done — both fixed, tests green.")),
        ],
    );
    let outcome = h
        .agent
        .as_mut()
        .expect("agent")
        .run(
            "fix the two bugs",
            &Default::default(),
            CancellationToken::new(),
            h.events_tx.clone(),
        )
        .await;
    let text = match outcome {
        TurnOutcome::Completed { response } => response.text_content(),
        other => panic!("expected completion, got {other:?}"),
    };
    assert_eq!(text, "All done — both fixed, tests green.");
    assert_eq!(
        h.requests.lock().unwrap().len(),
        2,
        "model must be re-called"
    );

    let events = drain_events(&mut h);
    let guards: Vec<&String> = events
        .iter()
        .filter_map(|e| match e {
            AgentEvent::StopHookContinuation { reason } => Some(reason),
            _ => None,
        })
        .collect();
    assert_eq!(guards.len(), 1);
    assert!(guards[0].contains("cut off"), "reason: {}", guards[0]);

    let users = guard_messages(&mut h).await;
    assert!(
        users.iter().any(|u| u.starts_with("[stop-guard]:")),
        "ledger must contain the guard nudge: {users:?}"
    );
}

/// The regression this file's `guard_messages` alone could not catch: being
/// recorded in the ledger is not the same as reaching the model. This
/// inspects the actual second `ChatRequest` the provider received and
/// checks both halves of the delivery guarantee — the nudge text is
/// present verbatim, and the request ends on a user-role message (current
/// Claude models reject a request with no trailing user turn — invariant
/// covered by `TurnIndex::from_log`'s `closed` derivation in
/// vak-session/src/turns.rs).
#[tokio::test]
async fn the_stop_guard_nudge_reaches_the_actual_next_request() {
    let mut h = harness(
        Some(StopPolicy {
            marker_gate: true,
            verify_gate: false,
            max_blocks: 2,
        }),
        vec![
            ScriptedResponse::Message(text_msg("Fixing both:")),
            ScriptedResponse::Message(text_msg("All done — both fixed, tests green.")),
        ],
    );
    let outcome = h
        .agent
        .as_mut()
        .expect("agent")
        .run(
            "fix the two bugs",
            &Default::default(),
            CancellationToken::new(),
            h.events_tx.clone(),
        )
        .await;
    assert!(matches!(outcome, TurnOutcome::Completed { .. }));

    let requests = h.requests.lock().unwrap().clone();
    assert_eq!(requests.len(), 2, "model must be re-called after the guard");
    let redo_request = &requests[1];

    let last = redo_request
        .messages
        .last()
        .expect("the redo request must carry at least one message");
    assert_eq!(
        last.role,
        vak_llm::Role::User,
        "the request must not end on the assistant's own draft: {:?}",
        redo_request.messages
    );

    let carries_nudge = redo_request.messages.iter().any(|m| {
        m.content.iter().any(|b| match b {
            ContentBlock::Text { text } => text.starts_with("[stop-guard]:"),
            _ => false,
        })
    });
    assert!(
        carries_nudge,
        "the redo request must carry the [stop-guard] nudge verbatim: {:?}",
        redo_request.messages
    );
}

#[tokio::test]
async fn verify_gate_blocks_when_the_reading_demands_proof_and_nothing_ran() {
    let mut h = harness(
        Some(StopPolicy {
            marker_gate: false,
            verify_gate: true,
            max_blocks: 1,
        }),
        vec![
            ScriptedResponse::Message(text_msg("Created fizzbuzz.py.")),
            ScriptedResponse::Message(text_msg("Ran it — output verified.")),
        ],
    );
    let mut reading = vak_intent::Reading::general();
    reading.act = vak_intent::Act::Author;
    reading.evidence = vak_intent::Evidence::Verified;
    let mut spec = vak_intent::OutcomeSpec::from_reading(
        "Create fizzbuzz.py and run it to prove that it works.",
        &reading,
        1,
    );
    spec.stop = vak_intent::StopProfile::Verification;
    h.agent.as_mut().expect("agent").config.outcome = Some(spec);
    let outcome = h
        .agent
        .as_mut()
        .expect("agent")
        .run(
            "Create fizzbuzz.py and run it to prove that it works.",
            &Default::default(),
            CancellationToken::new(),
            h.events_tx.clone(),
        )
        .await;
    assert!(matches!(outcome, TurnOutcome::Completed { .. }));
    assert_eq!(h.requests.lock().unwrap().len(), 2);

    let guards: Vec<String> = drain_events(&mut h)
        .into_iter()
        .filter_map(|e| match e {
            AgentEvent::StopHookContinuation { reason } => Some(reason),
            _ => None,
        })
        .collect();
    assert_eq!(guards.len(), 1);
    assert!(guards[0].contains("proves it"), "reason: {}", guards[0]);
}

#[tokio::test]
async fn max_blocks_cap_lets_second_bad_response_through() {
    let mut h = harness(
        Some(StopPolicy {
            marker_gate: true,
            verify_gate: false,
            max_blocks: 1,
        }),
        vec![
            ScriptedResponse::Message(text_msg("Now I'll fix it")),
            ScriptedResponse::Message(text_msg("Now I'll really fix it")),
        ],
    );
    let outcome = h
        .agent
        .as_mut()
        .expect("agent")
        .run(
            "fix",
            &Default::default(),
            CancellationToken::new(),
            h.events_tx.clone(),
        )
        .await;
    match outcome {
        TurnOutcome::Completed { response } => {
            assert_eq!(response.text_content(), "Now I'll really fix it")
        }
        other => panic!("expected completion at cap, got {other:?}"),
    }
    assert_eq!(h.requests.lock().unwrap().len(), 2, "cap stops the loop");
    let guards: Vec<_> = guard_messages(&mut h)
        .await
        .into_iter()
        .filter(|u| u.starts_with("[stop-guard]:"))
        .collect();
    assert_eq!(guards.len(), 1, "exactly one nudge persisted");
}

#[tokio::test]
async fn guard_at_turn_limit_is_not_reported_as_completed() {
    let mut h = harness(
        Some(StopPolicy {
            marker_gate: true,
            verify_gate: false,
            max_blocks: 1,
        }),
        vec![ScriptedResponse::Message(text_msg("Fixing both:"))],
    );
    h.agent.as_mut().expect("agent").config.max_turns = 1;
    let outcome = h
        .agent
        .as_mut()
        .expect("agent")
        .run(
            "fix the two bugs",
            &Default::default(),
            CancellationToken::new(),
            h.events_tx.clone(),
        )
        .await;
    assert!(matches!(outcome, TurnOutcome::MaxTurnsReached));
}

#[tokio::test]
async fn disabled_policy_never_blocks() {
    let mut h = harness(
        None,
        vec![ScriptedResponse::Message(text_msg("Fixing both:"))],
    );
    let outcome = h
        .agent
        .as_mut()
        .expect("agent")
        .run(
            "fix",
            &Default::default(),
            CancellationToken::new(),
            h.events_tx.clone(),
        )
        .await;
    assert!(matches!(outcome, TurnOutcome::Completed { .. }));
    assert_eq!(h.requests.lock().unwrap().len(), 1, "no continuation");
    assert!(
        drain_events(&mut h)
            .into_iter()
            .all(|e| !matches!(e, AgentEvent::StopHookContinuation { .. }))
    );
    assert!(
        !guard_messages(&mut h)
            .await
            .iter()
            .any(|u| u.starts_with("[stop-guard]:"))
    );
}

#[tokio::test]
async fn authored_prose_without_a_file_target_completes_without_a_guard() {
    let mut h = harness(
        Some(StopPolicy {
            marker_gate: false,
            verify_gate: true,
            max_blocks: 1,
        }),
        vec![ScriptedResponse::Message(text_msg(
            "Here is the component, explained step by step.",
        ))],
    );

    let mut reading = vak_intent::Reading::general();
    reading.act = vak_intent::Act::Author;
    let spec =
        vak_intent::OutcomeSpec::from_reading("Build a react visualization component", &reading, 1);
    h.agent.as_mut().expect("agent").config.outcome = Some(spec);

    let outcome = h
        .agent
        .as_mut()
        .expect("agent")
        .run(
            "Build a react visualization component",
            &Default::default(),
            CancellationToken::new(),
            h.events_tx.clone(),
        )
        .await;
    assert!(matches!(outcome, TurnOutcome::Completed { .. }));
    assert_eq!(
        h.requests.lock().unwrap().len(),
        1,
        "authoring is not an effect"
    );
    assert!(
        drain_events(&mut h)
            .iter()
            .all(|e| !matches!(e, AgentEvent::StopHookContinuation { .. }))
    );
}

#[tokio::test]
async fn universal_task_with_verify_completes_without_bash_guard() {
    let mut h = harness(
        Some(StopPolicy {
            marker_gate: true,
            verify_gate: true,
            max_blocks: 1,
        }),
        vec![ScriptedResponse::Message(text_msg(
            "Here is the brewing guide: grind size, water-to-coffee ratio, and brew temperature. I have verified that water temperature should be kept between 90°C and 96°C for optimal extraction.",
        ))],
    );

    let outcome = h
        .agent
        .as_mut()
        .expect("agent")
        .run(
            "Explain the 3 main coffee brewing variables and verify that water temperature recommendations are included.",
            &Default::default(),
            CancellationToken::new(),
            h.events_tx.clone(),
        )
        .await;

    assert!(matches!(outcome, TurnOutcome::Completed { .. }));
    // Must complete in 1 call without being falsely trapped by VerificationMissing demanding bash
    assert_eq!(h.requests.lock().unwrap().len(), 1);
    assert!(
        drain_events(&mut h)
            .into_iter()
            .all(|e| !matches!(e, AgentEvent::StopHookContinuation { .. }))
    );
}
