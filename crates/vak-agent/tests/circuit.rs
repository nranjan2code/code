#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::sync::{Arc, Mutex};

use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;

use tempfile::tempdir;

use vak_agent::{Agent, AgentConfig, CircuitBreaker, CircuitBreakerConfig, TurnOutcome};
use vak_llm::stream;
use vak_llm::types::{AssistantMessage, ChatRequest, ContentBlock, StopReason, Usage};
use vak_llm::{EventStream, LlmError, Provider};
use vak_session::SessionLog;
use vak_session::types::{FrozenContract, SessionHeader};

#[test]
fn breaker_opens_after_threshold_and_half_closes_after_cooldown() {
    let br = CircuitBreaker::new(CircuitBreakerConfig {
        threshold: 3,
        cooldown: std::time::Duration::from_millis(120),
    });

    assert!(br.check().is_ok());
    br.record_failure();
    br.record_failure();
    assert!(br.check().is_ok(), "below threshold");

    br.record_failure();
    assert!(br.check().is_err(), "open at threshold");

    std::thread::sleep(std::time::Duration::from_millis(150));
    assert!(br.check().is_ok(), "cooldown half-closes");

    // A success resets the failure count.
    br.record_success();
    br.record_failure();
    br.record_failure();
    assert!(br.check().is_ok(), "success reset the counter");
}

#[test]
fn non_retryable_failures_do_not_trip_the_breaker() {
    let br = CircuitBreaker::new(CircuitBreakerConfig {
        threshold: 2,
        cooldown: std::time::Duration::from_secs(60),
    });
    // Simulate auth failures: they never call record_failure.
    for _ in 0..10 {
        assert!(br.check().is_ok());
    }
    assert!(br.check().is_ok());
}

struct Scripted {
    calls: Arc<Mutex<u32>>,
    fail_with: LlmError,
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
        *self.calls.lock().unwrap() += 1;
        let (mut sink, rx) = stream::channel(8);
        sink.close_error(self.fail_with.clone()).await;
        Ok(rx)
    }
}

fn text_msg(t: &str) -> AssistantMessage {
    AssistantMessage {
        content: vec![ContentBlock::text(t)],
        stop_reason: StopReason::EndTurn,
        usage: Usage::default(),
        model: "test-model".into(),
    }
}

fn build_agent(
    provider: Arc<dyn Provider>,
    breaker: Arc<CircuitBreaker>,
    session_id: &str,
) -> Agent {
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
            system_prompt: "sys".into(),
            tools: vec![],
            permission_mode: "full-access".into(),
            skills: Vec::new(),
        },
    };
    let log = SessionLog::create(dir.path().join("s.jsonl"), header).unwrap();
    let mut cfg = AgentConfig::new("sys");
    cfg.max_retries = 1; // one retry per run => 2 failures per run
    cfg.retry_base_backoff_ms = 1;
    cfg.circuit_breaker = Some(breaker);
    std::mem::forget(dir);
    Agent::new(provider, log, cfg)
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn second_run_fails_fast_when_circuit_is_open() {
    let calls = Arc::new(Mutex::new(0u32));
    let provider = Arc::new(Scripted {
        calls: calls.clone(),
        fail_with: LlmError::Overloaded("provider down".into()),
    });
    let breaker = Arc::new(CircuitBreaker::new(CircuitBreakerConfig {
        threshold: 4,
        cooldown: std::time::Duration::from_millis(200),
    }));

    // Run 1: 2 attempts (1 + 1 retry) => 2 calls, breaker counts 2 failures.
    let mut a1 = build_agent(provider.clone(), breaker.clone(), "run-1");
    let outcome = a1
        .run(
            "go",
            &Default::default(),
            CancellationToken::new(),
            mpsc::channel(64).0,
        )
        .await;
    assert!(matches!(outcome, TurnOutcome::Failed { .. }));
    assert_eq!(*calls.lock().unwrap(), 2);

    // Run 2 (fresh agent, SAME breaker): 2 more failures => 4 total >= 4 → open.
    let mut a2 = build_agent(provider.clone(), breaker.clone(), "run-2");
    let outcome = a2
        .run(
            "go",
            &Default::default(),
            CancellationToken::new(),
            mpsc::channel(64).0,
        )
        .await;
    assert!(matches!(outcome, TurnOutcome::Failed { .. }));
    assert_eq!(*calls.lock().unwrap(), 4);

    // Run 3: circuit is open — fails fast WITHOUT touching the provider.
    let mut a3 = build_agent(provider.clone(), breaker.clone(), "run-3");
    let outcome = a3
        .run(
            "go",
            &Default::default(),
            CancellationToken::new(),
            mpsc::channel(64).0,
        )
        .await;
    match outcome {
        TurnOutcome::Failed { error } => {
            assert!(error.to_string().contains("circuit open"), "got: {error}");
        }
        other => panic!("expected fast failure, got {other:?}"),
    }
    assert_eq!(
        *calls.lock().unwrap(),
        4,
        "open circuit must not reach the provider"
    );

    // A healthy provider sharing the breaker heals it on success.
    struct Healthy;
    #[async_trait::async_trait]
    impl Provider for Healthy {
        fn name(&self) -> &str {
            "healthy"
        }
        async fn stream(
            &self,
            _r: ChatRequest,
            _c: CancellationToken,
        ) -> Result<EventStream, LlmError> {
            let (mut sink, rx) = stream::channel(8);
            let msg = text_msg("recovered");
            sink.push(stream::StreamEvent::Start {
                partial: msg.clone(),
            });
            sink.close_message(msg).await;
            Ok(rx)
        }
    }
    // After the cooldown the circuit half-closes: one probe gets through,
    // and a healthy provider heals it on success.
    std::thread::sleep(std::time::Duration::from_millis(250));
    let mut a4 = build_agent(Arc::new(Healthy), breaker.clone(), "run-4");
    let outcome = a4
        .run(
            "go",
            &Default::default(),
            CancellationToken::new(),
            mpsc::channel(64).0,
        )
        .await;
    assert!(matches!(outcome, TurnOutcome::Completed { .. }));
    assert!(breaker.check().is_ok(), "success must close the circuit");
}
