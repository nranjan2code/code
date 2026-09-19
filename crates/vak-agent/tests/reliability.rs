#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::collections::VecDeque;
use std::sync::{Arc, Mutex};
use std::time::Instant;

use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;

use tempfile::tempdir;

use vak_agent::{Agent, AgentConfig, TurnOutcome};
use vak_llm::stream;
use vak_llm::types::{AssistantMessage, ChatRequest, ContentBlock, StopReason, Usage};
use vak_llm::{EventStream, LlmError, Provider};
use vak_session::SessionLog;
use vak_session::types::{FrozenContract, SessionHeader};

enum Step {
    Err(LlmError),
    Text(String),
}

struct Scripted {
    steps: Mutex<VecDeque<Step>>,
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
        let next = self.steps.lock().unwrap().pop_front();
        let (mut sink, rx) = stream::channel(64);
        match next {
            Some(Step::Text(t)) => {
                let msg = AssistantMessage {
                    content: vec![ContentBlock::text(t)],
                    stop_reason: StopReason::EndTurn,
                    usage: Usage::default(),
                    model: "test-model".into(),
                    response_id: None,
                };
                sink.push(stream::StreamEvent::Start {
                    partial: msg.clone(),
                });
                sink.close_message(msg).await;
            }
            Some(Step::Err(e)) => sink.close_error(e).await,
            None => sink.close_error(LlmError::Parse("exhausted".into())).await,
        }
        Ok(rx)
    }
}

fn build(
    steps: Vec<Step>,
    max_retries: u32,
    base_ms: u64,
    timeout: Option<std::time::Duration>,
) -> Agent {
    let dir = tempdir().unwrap();
    let header = SessionHeader {
        agent: None,
        session_id: "rel".into(),
        created_at: chrono::Utc::now(),
        cwd: dir.path().to_path_buf(),
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
    };
    let log = SessionLog::create(dir.path().join("s.jsonl"), header).unwrap();
    let mut cfg = AgentConfig::new("sys");
    cfg.max_retries = max_retries;
    cfg.retry_base_backoff_ms = base_ms;
    cfg.request_timeout = timeout;
    // Step-level machinery is the system under test here; run-level
    // endurance has its own tests (tests/run_endurance.rs).
    cfg.run_retry_attempts = 0;
    std::mem::forget(dir);
    Agent::new(
        Arc::new(Scripted {
            steps: Mutex::new(steps.into_iter().collect()),
        }),
        log,
        cfg,
    )
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn transient_errors_are_retried_until_success() {
    let mut agent = build(
        vec![
            Step::Err(LlmError::RateLimit {
                message: "slow down".into(),
                retry_after_secs: None,
            }),
            Step::Err(LlmError::Overloaded("529".into())),
            Step::Err(LlmError::Network("flap".into())),
            Step::Text("finally".into()),
        ],
        3,
        1, // near-zero backoff for the test
        None,
    );

    let outcome = agent
        .run(
            "go",
            &Default::default(),
            CancellationToken::new(),
            mpsc::channel(64).0,
        )
        .await;

    match outcome {
        TurnOutcome::Completed { response } => assert_eq!(response.text_content(), "finally"),
        other => panic!("expected completed after retries, got {other:?}"),
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn non_retryable_errors_fail_immediately() {
    let mut agent = build(
        vec![
            Step::Err(LlmError::Auth("bad key".into())),
            Step::Text("never".into()),
        ],
        3,
        1,
        None,
    );

    let start = Instant::now();
    let outcome = agent
        .run(
            "go",
            &Default::default(),
            CancellationToken::new(),
            mpsc::channel(64).0,
        )
        .await;
    assert!(
        start.elapsed() < std::time::Duration::from_millis(500),
        "auth errors must not be retried"
    );
    match outcome {
        TurnOutcome::Failed { error } => assert!(error.to_string().contains("bad key")),
        other => panic!("expected failed, got {other:?}"),
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn retry_budget_exhaustion_fails_with_last_error() {
    let err_fn = || {
        Step::Err(LlmError::RateLimit {
            message: "still limited".into(),
            retry_after_secs: None,
        })
    };
    let mut agent = build(
        vec![err_fn(), err_fn(), err_fn(), err_fn(), err_fn()],
        2, // budget smaller than failure count
        1,
        None,
    );

    let outcome = agent
        .run(
            "go",
            &Default::default(),
            CancellationToken::new(),
            mpsc::channel(64).0,
        )
        .await;
    match outcome {
        TurnOutcome::Failed { error } => assert!(error.to_string().contains("still limited")),
        other => panic!("expected failed after budget, got {other:?}"),
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn watchdog_deadline_converts_hung_step_into_retryable_failure() {
    // A provider whose stream never completes: the deadline must fire.
    let cancelled = Arc::new(std::sync::atomic::AtomicBool::new(false));
    struct Hung(Arc<std::sync::atomic::AtomicBool>);
    #[async_trait::async_trait]
    impl Provider for Hung {
        fn name(&self) -> &str {
            "hung"
        }
        async fn stream(
            &self,
            _r: ChatRequest,
            c: CancellationToken,
        ) -> Result<EventStream, LlmError> {
            let cancelled = self.0.clone();
            let (sink, rx) = stream::channel(8);
            sink.push(stream::StreamEvent::Start {
                partial: AssistantMessage::empty("m"),
            });
            // Hold the sink open forever — simulates a stalled stream.
            tokio::spawn(async move {
                let _keep_alive = sink;
                c.cancelled().await;
                cancelled.store(true, std::sync::atomic::Ordering::SeqCst);
            });
            Ok(rx)
        }
    }

    let dir = tempdir().unwrap();
    let header = SessionHeader {
        agent: None,
        session_id: "hung".into(),
        created_at: chrono::Utc::now(),
        cwd: dir.path().to_path_buf(),
        parent_session_id: None,
        contract_id: None,
        work_item_id: None,
        conversation: None,
        contract: FrozenContract {
            app_version: "0".into(),
            provider: "hung".into(),
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
    let mut cfg = AgentConfig::new("sys");
    cfg.max_retries = 0;
    cfg.request_timeout = Some(std::time::Duration::from_millis(300));
    cfg.run_retry_attempts = 0;
    std::mem::forget(dir);
    let mut agent = Agent::new(Arc::new(Hung(cancelled.clone())), log, cfg);

    let start = Instant::now();
    let outcome = agent
        .run(
            "go",
            &Default::default(),
            CancellationToken::new(),
            mpsc::channel(64).0,
        )
        .await;
    let elapsed = start.elapsed();

    eprintln!("OUTCOME: {outcome:?}");
    match outcome {
        TurnOutcome::Failed { error } => {
            assert!(error.to_string().contains("deadline"), "got: {error}");
        }
        other => panic!("expected deadline failure, got {other:?}"),
    }
    assert!(
        elapsed < std::time::Duration::from_secs(3),
        "watchdog must fire promptly, took {elapsed:?}"
    );
    tokio::time::sleep(std::time::Duration::from_millis(20)).await;
    assert!(cancelled.load(std::sync::atomic::Ordering::SeqCst));
}
