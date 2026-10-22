#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

//! A small window carries the newest exchanges whole and one summary of the
//! rest, written once each time the window fills, never once per turn.

use std::collections::VecDeque;
use std::sync::{Arc, Mutex};

use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;

use tempfile::tempdir;

use vak_agent::{Agent, AgentConfig, TurnOutcome};
use vak_llm::stream;
use vak_llm::types::{AssistantMessage, ChatRequest, ContentBlock, StopReason, Usage};
use vak_llm::{EventStream, LlmError, Provider};
use vak_session::SessionLog;
use vak_session::types::{EntryPayload, FrozenContract, SessionHeader};

/// Routes by system-prompt marker: a compaction call carries
/// `COMPACTION_SYSTEM`; every other request gets the next scripted step
/// answer, cycling once exhausted (each turn here is a single no-tool-call
/// step, so the same short pool of answers covers many turns).
struct TaggedScripted {
    capacity_key: String,
    steps: Mutex<VecDeque<AssistantMessage>>,
    seen_systems: Arc<Mutex<Vec<String>>>,
    seen_requests: Arc<Mutex<Vec<ChatRequest>>>,
}

impl TaggedScripted {
    fn is_compaction(request: &ChatRequest) -> bool {
        request
            .system
            .as_deref()
            .is_some_and(|s| s.contains("context compactor"))
    }
}

#[async_trait::async_trait]
impl Provider for TaggedScripted {
    fn name(&self) -> &str {
        "scripted"
    }

    fn rate_limit_key(&self) -> String {
        self.capacity_key.clone()
    }

    async fn stream(
        &self,
        request: ChatRequest,
        _cancel: CancellationToken,
    ) -> Result<EventStream, LlmError> {
        self.seen_systems
            .lock()
            .unwrap()
            .push(request.system.clone().unwrap_or_default());
        self.seen_requests.lock().unwrap().push(request.clone());
        let msg = if Self::is_compaction(&request) {
            Some(text(
                "Summary: prior turns condensed; open items carried forward.",
            ))
        } else {
            let mut steps = self.steps.lock().unwrap();
            let next = steps.pop_front();
            if let Some(m) = &next {
                steps.push_back(m.clone());
            }
            next
        };
        let (mut sink, rx) = stream::channel(64);
        match msg {
            Some(m) => {
                sink.push(stream::StreamEvent::Start { partial: m.clone() });
                sink.close_message(m).await;
            }
            None => sink.close_error(LlmError::Parse("exhausted".into())).await,
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
        applied: None,
    }
}

fn header(session_id: &str, dir: &std::path::Path) -> SessionHeader {
    SessionHeader {
        space: None,
        run: None,
        cause: None,
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

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_small_window_summarises_once_per_fill_and_keeps_the_newest_whole() {
    let dir = tempdir().unwrap();
    let log =
        SessionLog::create(dir.path().join("s.jsonl"), header("compact", dir.path())).unwrap();

    let seen = Arc::new(Mutex::new(Vec::new()));
    let requests = Arc::new(Mutex::new(Vec::new()));
    let provider = Arc::new(TaggedScripted {
        capacity_key: crate::support::capacity_key(),
        steps: Mutex::new(VecDeque::from(vec![text(
            "a padded no-tool-call answer with enough filler text to accumulate real tokens over several turns of conversation",
        )])),
        seen_systems: seen.clone(),
        seen_requests: requests.clone(),
    });

    let mut cfg = AgentConfig::new("sys");
    // No live CapacityProfile is wired in, so the loop falls back to a
    // metadata-only one built from these two fields (docs/design/68 §4).
    // Small enough that recency alone cannot hold every turn at Full, and
    // eventually not even at Card, forcing a packet.
    cfg.declared_window = Some(600);
    cfg.max_output = 20;
    let mut agent = Agent::new(provider, log, cfg);

    for i in 0..20 {
        let outcome = agent
            .run(
                &format!(
                    "padded question number {i} with enough filler text to accumulate real tokens across turns"
                ),
                &Default::default(),
                CancellationToken::new(),
                mpsc::channel(256).0,
            )
            .await;
        assert!(
            matches!(outcome, TurnOutcome::Completed { .. }),
            "turn {i} did not complete: {outcome:?}"
        );
    }
    let summaries = seen
        .lock()
        .unwrap()
        .iter()
        .filter(|s| s.contains("context compactor"))
        .count();
    assert!(summaries >= 1, "a full window is summarised");
    assert!(
        summaries < 19,
        "a summary is reused until the window fills again, not rewritten every turn ({summaries})"
    );

    let session = agent.session.lock().await;
    let first_turn = session.turn_cards().first().unwrap().0.clone();
    let packets: Vec<_> = session
        .chain_to_root()
        .into_iter()
        .filter_map(|e| match &e.payload {
            EntryPayload::Compaction(c) if !c.reset_all => Some(c.first_turn_id.clone()),
            _ => None,
        })
        .collect();
    assert_eq!(packets.len(), summaries);
    assert!(
        packets.iter().all(|first| *first == first_turn),
        "every summary starts at the conversation's first turn"
    );
    let last_request = requests.lock().unwrap().last().cloned().unwrap();
    let current_projection: String = last_request
        .messages
        .iter()
        .map(|m| m.text_content())
        .collect::<Vec<_>>()
        .join("\n");
    assert!(
        current_projection.contains("padded question number 19")
            && current_projection.contains("padded question number 18")
            && current_projection.contains("<context_summary>")
            && !current_projection.contains("padded question number 0 "),
        "the newest exchanges ride whole beside one summary: {current_projection}"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn no_usable_horizon_fails_closed_when_handoff_is_disabled() {
    let dir = tempdir().unwrap();
    let log = SessionLog::create(dir.path().join("s.jsonl"), header("over", dir.path())).unwrap();

    let provider = Arc::new(TaggedScripted {
        capacity_key: crate::support::capacity_key(),
        steps: Mutex::new(VecDeque::from(vec![text("never reached")])),
        seen_systems: Arc::new(Mutex::new(Vec::new())),
        seen_requests: Arc::new(Mutex::new(Vec::new())),
    });

    let mut cfg = AgentConfig::new("sys");
    // A horizon far too small to hold even the current directive plus
    // output reserve: budget saturates to 0 (no usable horizon).
    cfg.declared_window = Some(5);
    cfg.max_output = 5;
    cfg.handoff_reset = false;
    cfg.run_retry_attempts = 0;
    let mut agent = Agent::new(provider, log, cfg);

    let outcome = agent
        .run(
            &"x".repeat(2_000),
            &Default::default(),
            CancellationToken::new(),
            mpsc::channel(64).0,
        )
        .await;
    match outcome {
        TurnOutcome::Failed { error } => {
            assert!(
                error.to_string().contains("no usable horizon"),
                "got: {error}"
            );
        }
        other => panic!("expected fail-closed, got {other:?}"),
    }
}
