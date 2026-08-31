#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::sync::{Arc, Mutex};

use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;

use tempfile::tempdir;

use vak_agent::{Agent, AgentConfig, AgentEvent, TurnOutcome};
use vak_llm::stream;
use vak_llm::types::{AssistantMessage, ChatRequest, ContentBlock, StopReason, Usage};
use vak_llm::{EventStream, LlmError, Provider};
use vak_session::SessionLog;
use vak_session::types::{FrozenContract, SessionHeader};

/// Fails the first `failures` calls with `error`, then succeeds.
struct FlakyThenGood {
    calls: Arc<Mutex<u32>>,
    failures: u32,
    error: LlmError,
}

#[async_trait::async_trait]
impl Provider for FlakyThenGood {
    fn name(&self) -> &str {
        "flaky-then-good"
    }

    async fn stream(
        &self,
        _request: ChatRequest,
        _cancel: CancellationToken,
    ) -> Result<EventStream, LlmError> {
        let n = {
            let mut c = self.calls.lock().unwrap();
            *c += 1;
            *c
        };
        let (mut sink, rx) = stream::channel(8);
        if n <= self.failures {
            sink.close_error(self.error.clone()).await;
        } else {
            let msg = AssistantMessage {
                content: vec![ContentBlock::text("recovered")],
                stop_reason: StopReason::EndTurn,
                usage: Usage::default(),
                model: "test-model".into(),
            };
            sink.push(stream::StreamEvent::Start {
                partial: msg.clone(),
            });
            sink.close_message(msg).await;
        }
        Ok(rx)
    }
}

fn build_agent(provider: Arc<dyn Provider>, session_id: &str, attempts: u32) -> Agent {
    let dir = tempdir().unwrap();
    let header = SessionHeader {
        session_id: session_id.into(),
        created_at: chrono::Utc::now(),
        cwd: dir.path().to_path_buf(),
        parent_session_id: None,
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
        },
    };
    let log = SessionLog::create(dir.path().join("s.jsonl"), header).unwrap();
    let mut cfg = AgentConfig::new("sys");
    cfg.max_retries = 0;
    cfg.retry_base_backoff_ms = 1;
    cfg.run_retry_attempts = attempts;
    cfg.run_retry_base_backoff_ms = 1;
    std::mem::forget(dir);
    Agent::new(provider, log, cfg)
}

async fn run_agent(agent: &mut Agent) -> (TurnOutcome, Vec<AgentEvent>) {
    let (tx, mut rx) = mpsc::channel(256);
    let cancel = CancellationToken::new();
    let outcome = agent
        .run("go", &Default::default(), cancel, tx.clone())
        .await;
    drop(tx);
    let mut events = Vec::new();
    while let Ok(ev) = rx.try_recv() {
        events.push(ev);
    }
    (outcome, events)
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn endurance_bridges_transient_windows_then_completes() {
    for err in [
        LlmError::Overloaded("storm".into()),
        LlmError::RateLimit {
            message: "429".into(),
            retry_after_secs: None,
        },
        LlmError::Network("connection reset".into()),
        LlmError::Parse("stream closed before finish_reason".into()),
    ] {
        let calls = Arc::new(Mutex::new(0u32));
        let provider = Arc::new(FlakyThenGood {
            calls: calls.clone(),
            failures: 3,
            error: err.clone(),
        });
        let mut agent = build_agent(provider, "endure", 5);
        let (outcome, events) = run_agent(&mut agent).await;
        assert!(
            matches!(outcome, TurnOutcome::Completed { .. }),
            "expected completion after endurance, got {outcome:?} ({err:?})"
        );
        assert_eq!(
            *calls.lock().unwrap(),
            4,
            "3 failures + 1 success ({err:?})"
        );
        let retries = events
            .iter()
            .filter(|e| matches!(e, AgentEvent::RetryScheduled { .. }))
            .count();
        assert_eq!(
            retries, 3,
            "run-level re-attempts surfaced as events ({err:?})"
        );
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn endurance_budget_exhaustion_fails_cleanly() {
    let calls = Arc::new(Mutex::new(0u32));
    let provider = Arc::new(FlakyThenGood {
        calls: calls.clone(),
        failures: 10,
        error: LlmError::Overloaded("long storm".into()),
    });
    let mut agent = build_agent(provider, "exhaust", 2);
    let (outcome, _) = run_agent(&mut agent).await;
    match outcome {
        TurnOutcome::Failed { error } => {
            assert!(error.to_string().contains("long storm"));
        }
        other => panic!("expected failed after budget exhausted, got {other:?}"),
    }
    assert_eq!(*calls.lock().unwrap(), 3, "initial + 2 run-level attempts");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn permanent_errors_are_not_endured() {
    for err in [
        LlmError::Auth("bad key".into()),
        LlmError::InvalidRequest("bad body".into()),
        LlmError::Api {
            status: 500,
            message: "server".into(),
        },
    ] {
        let calls = Arc::new(Mutex::new(0u32));
        let provider = Arc::new(FlakyThenGood {
            calls: calls.clone(),
            failures: 99,
            error: err.clone(),
        });
        let mut agent = build_agent(provider, "permanent", 5);
        let (outcome, events) = run_agent(&mut agent).await;
        assert!(matches!(outcome, TurnOutcome::Failed { .. }), "{err:?}");
        assert_eq!(*calls.lock().unwrap(), 1, "no run-level retry for {err:?}");
        assert!(
            !events
                .iter()
                .any(|e| matches!(e, AgentEvent::RetryScheduled { .. })),
            "no RetryScheduled for {err:?}"
        );
    }
}
