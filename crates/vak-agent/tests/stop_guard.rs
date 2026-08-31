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
        session_id: "s-stop".into(),
        created_at: chrono::Utc::now(),
        cwd: dir.path().to_path_buf(),
        parent_session_id: None,
        contract_id: None,
        work_item_id: None,
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

async fn guard_messages(h: &mut Harness) -> Vec<String> {
    let agent = h.agent.take().expect("guard_messages consumes the agent");
    let session = agent.into_session().await;
    session
        .derive_messages()
        .into_iter()
        .filter(|m| m.role == vak_llm::Role::User)
        .filter_map(|m| match m.content.first() {
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

#[tokio::test]
async fn verify_gate_blocks_when_prompt_demands_and_no_bash_ran() {
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
    assert!(guards[0].contains("verification"), "reason: {}", guards[0]);
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
