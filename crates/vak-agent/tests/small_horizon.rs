#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

//! A small measured horizon leaves little or no room for history, but a turn
//! that fits still runs (docs/design/68-context-engine.md §4). Only a turn
//! that cannot fit at all takes the handoff rescue.

use std::collections::VecDeque;
use std::sync::{Arc, Mutex};

use tempfile::tempdir;
use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;
use vak_agent::{Agent, AgentConfig, SteeringQueues, TurnOutcome};
use vak_context::capacity::{CacheBehaviour, CapacityProfile, Horizon, ProbeProvenance};
use vak_llm::stream;
use vak_llm::types::{AssistantMessage, ChatRequest, ContentBlock, StopReason, Usage};
use vak_llm::{EventStream, LlmError, Provider};
use vak_session::SessionLog;
use vak_session::types::{FrozenContract, SessionHeader};

struct Script {
    replies: Mutex<VecDeque<AssistantMessage>>,
    requests: Arc<Mutex<Vec<ChatRequest>>>,
}

#[async_trait::async_trait]
impl Provider for Script {
    fn name(&self) -> &str {
        "scripted"
    }
    async fn stream(&self, r: ChatRequest, _c: CancellationToken) -> Result<EventStream, LlmError> {
        self.requests.lock().unwrap().push(r);
        let next = self.replies.lock().unwrap().pop_front();
        let (mut sink, rx) = stream::channel(64);
        match next {
            Some(m) => {
                sink.push(stream::StreamEvent::Start { partial: m.clone() });
                sink.close_message(m).await;
            }
            None => {
                sink.close_error(LlmError::Parse("script exhausted".into()))
                    .await
            }
        }
        Ok(rx)
    }
}

fn profile(declared: u64, horizon: u64, reserve: u64) -> CapacityProfile {
    CapacityProfile::from_probe(
        Some(declared),
        None,
        Horizon {
            tokens: horizon,
            confidence: 0.9,
            last_confirmed: std::time::SystemTime::now(),
        },
        CacheBehaviour::Unknown,
        reserve,
        ProbeProvenance {
            probed_at: std::time::SystemTime::now(),
            rungs: Vec::new(),
            signals: Vec::new(),
            metadata_digest: "d".into(),
            quantisation: None,
        },
    )
}

/// Runs "hi" against a prefix of `prefix_chars` characters (0.25 tokens/char).
async fn run(profile: CapacityProfile, prefix_chars: usize) -> (TurnOutcome, usize, SessionLog) {
    let (outcome, sent, log, _) = run_with_budget(profile, prefix_chars, 0).await;
    (outcome, sent, log)
}

/// [`run`] with a request output budget (`max_output`), also returning the
/// `max_tokens` the first request carried.
async fn run_with_budget(
    profile: CapacityProfile,
    prefix_chars: usize,
    max_output: u64,
) -> (TurnOutcome, usize, SessionLog, Option<u32>) {
    let dir = tempdir().unwrap();
    let header = SessionHeader {
        space: None,
        run: None,
        cause: None,
        agent: None,
        session_id: "s-small".into(),
        created_at: chrono::Utc::now(),
        cwd: dir.path().to_path_buf(),
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
            system_prompt: "sys".into(),
            permission_mode: "full-access".into(),
            capabilities: Vec::new(),
            prompt_layers: Vec::new(),
        },
    };
    let log = SessionLog::create(dir.path().join("s.jsonl"), header).unwrap();
    let mut cfg = AgentConfig::new("x".repeat(prefix_chars));
    cfg.capacity = Some(profile);
    cfg.max_output = max_output;
    cfg.handoff_reset = true;
    let requests = Arc::new(Mutex::new(Vec::new()));
    let reply = |text: &str| AssistantMessage {
        content: vec![ContentBlock::text(text)],
        stop_reason: StopReason::EndTurn,
        usage: Usage::default(),
        model: "m".into(),
        response_id: None,
    };
    let provider = Arc::new(Script {
        replies: Mutex::new(vec![reply("hello!"), reply("hello!")].into()),
        requests: requests.clone(),
    });
    let mut agent = Agent::new(provider, log, cfg);
    let (tx, _rx) = mpsc::channel(4096);
    let outcome = agent
        .run("hi", &SteeringQueues::new(), CancellationToken::new(), tx)
        .await;
    let sent = requests.lock().unwrap().len();
    let first_budget = requests
        .lock()
        .unwrap()
        .first()
        .and_then(|r: &ChatRequest| r.max_tokens);
    std::mem::forget(dir);
    let session = agent.session.lock().await;
    let log = SessionLog::open_read_only(session.path().to_path_buf()).unwrap();
    (outcome, sent, log, first_budget)
}

#[tokio::test]
async fn a_turn_that_fits_runs_when_the_prefix_leaves_no_room_for_history() {
    // 13k horizon, 12k-token prefix: the doc's own small-model case.
    let (outcome, sent, log) = run(profile(32_768, 13_000, 4_096), 48_000).await;
    assert!(
        matches!(outcome, TurnOutcome::Completed { .. }),
        "{outcome:?}"
    );
    assert_eq!(sent, 1, "no summariser call before the turn's own request");
    assert!(
        !vak_session::TurnIndex::from_log(&log)
            .turns
            .iter()
            .any(|t| t.behind_reset),
        "no reset may be written for a turn that fits"
    );
}

#[tokio::test]
async fn the_listed_output_maximum_does_not_zero_the_budget() {
    // A 32k window whose listing reports a 29k maximum completion. The
    // listed maximum bounds a request's max_tokens, never the room planning
    // leaves (that is what replies are observed to take), and the request's
    // budget is clamped to what the window leaves beside its prompt.
    let (outcome, _, _, budget) = run_with_budget(profile(32_768, 32_768, 0), 20_000, 29_491).await;
    assert!(
        matches!(outcome, TurnOutcome::Completed { .. }),
        "{outcome:?}"
    );
    let budget = u64::from(budget.expect("a budget was set"));
    assert!(
        budget < 29_491,
        "clamped to the room beside the prompt: {budget}"
    );
    assert!(
        budget + 5_000 <= 32_768,
        "prompt and reply fit the window: {budget}"
    );
}

#[tokio::test]
async fn a_turn_that_cannot_fit_still_takes_the_rescue_and_fails_loudly() {
    // 14k-token prefix against a 13k horizon: the turn itself cannot be sent.
    let (outcome, _, _) = run(profile(32_768, 13_000, 4_096), 56_000).await;
    assert!(
        matches!(
            outcome,
            TurnOutcome::Failed {
                error: LlmError::Context(_)
            }
        ),
        "{outcome:?}"
    );
}
