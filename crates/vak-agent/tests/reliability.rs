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
        space: None,
        run: None,
        cause: None,
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
            let (mut sink, rx) = stream::channel(8);
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
        space: None,
        run: None,
        cause: None,
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

/// Hangs (holds its stream open until cancelled) for the first
/// `hangs_remaining` calls, then serves `good` (or a plain "done"). Counts
/// every call and every attempt-cancellation it actually observed, so a
/// test can tell a per-attempt child token from the run's own.
struct HungThenGood {
    hangs_remaining: Mutex<u32>,
    attempts: Arc<std::sync::atomic::AtomicU32>,
    attempt_cancellations: Arc<std::sync::atomic::AtomicU32>,
    good: Mutex<Option<AssistantMessage>>,
}

#[async_trait::async_trait]
impl Provider for HungThenGood {
    fn name(&self) -> &str {
        "hung"
    }
    async fn stream(&self, _r: ChatRequest, c: CancellationToken) -> Result<EventStream, LlmError> {
        self.attempts
            .fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        let should_hang = {
            let mut remaining = self.hangs_remaining.lock().unwrap();
            if *remaining > 0 {
                *remaining -= 1;
                true
            } else {
                false
            }
        };
        let (mut sink, rx) = stream::channel(8);
        if should_hang {
            sink.push(stream::StreamEvent::Start {
                partial: AssistantMessage::empty("m"),
            });
            let attempt_cancellations = self.attempt_cancellations.clone();
            tokio::spawn(async move {
                let mut sink = sink;
                c.cancelled().await;
                attempt_cancellations.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                // A well-behaved provider always closes with a terminal
                // event on cancellation (every real adapter does); a mock
                // that just dropped the sink here would surface as
                // "stream ended without a terminal event" instead of the
                // Aborted this simulates.
                sink.close_error(LlmError::Aborted { partial: None }).await;
            });
        } else {
            let msg = self
                .good
                .lock()
                .unwrap()
                .take()
                .unwrap_or_else(|| text_msg_for_hung("done"));
            sink.push(stream::StreamEvent::Start {
                partial: msg.clone(),
            });
            sink.close_message(msg).await;
        }
        Ok(rx)
    }
}

fn text_msg_for_hung(t: &str) -> AssistantMessage {
    AssistantMessage {
        content: vec![ContentBlock::text(t)],
        stop_reason: StopReason::EndTurn,
        usage: Usage::default(),
        model: "m".into(),
        response_id: None,
    }
}

fn hung_header(provider: &str, session_id: &str, dir: &std::path::Path) -> SessionHeader {
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
            provider: provider.into(),
            model: "m".into(),
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

/// A per-attempt timeout must never poison the run's own cancellation
/// token: with retries enabled, every attempt independently hangs and
/// times out, and the run still gets a full, bounded set of attempts
/// (initial + `max_retries`) rather than the first timeout silently
/// aborting the rest.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn hung_provider_with_retries_gets_bounded_attempts_then_fails_with_deadline() {
    let dir = tempdir().unwrap();
    let log = SessionLog::create(
        dir.path().join("s.jsonl"),
        hung_header("hung", "hung-retries", dir.path()),
    )
    .unwrap();
    let attempts = Arc::new(std::sync::atomic::AtomicU32::new(0));
    let provider = Arc::new(HungThenGood {
        hangs_remaining: Mutex::new(u32::MAX),
        attempts: attempts.clone(),
        attempt_cancellations: Arc::new(std::sync::atomic::AtomicU32::new(0)),
        good: Mutex::new(None),
    });
    let mut cfg = AgentConfig::new("sys");
    cfg.max_retries = 2;
    cfg.retry_base_backoff_ms = 1;
    cfg.request_timeout = Some(std::time::Duration::from_millis(150));
    cfg.run_retry_attempts = 0;
    std::mem::forget(dir);
    let mut agent = Agent::new(provider, log, cfg);

    let outcome = agent
        .run(
            "go",
            &Default::default(),
            CancellationToken::new(),
            mpsc::channel(64).0,
        )
        .await;
    match outcome {
        TurnOutcome::Failed { error } => assert!(error.to_string().contains("deadline")),
        other => panic!("expected deadline failure, got {other:?}"),
    }
    assert_eq!(
        attempts.load(std::sync::atomic::Ordering::SeqCst),
        3,
        "initial attempt + 2 retries, each independently timing out"
    );
}

/// A leg that always times out falls back to the next frozen-ladder leg
/// instead of the run-level `cancel` token being poisoned by the first
/// timeout (which used to make the very next retry/fallback wait observe
/// an already-cancelled token and return Aborted instead of falling back).
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_hung_leg_falls_back_to_a_working_ladder_leg() {
    let dir = tempdir().unwrap();
    let log = SessionLog::create(
        dir.path().join("s.jsonl"),
        hung_header("hung", "hung-fallback", dir.path()),
    )
    .unwrap();
    let hung_provider = Arc::new(HungThenGood {
        hangs_remaining: Mutex::new(u32::MAX),
        attempts: Arc::new(std::sync::atomic::AtomicU32::new(0)),
        attempt_cancellations: Arc::new(std::sync::atomic::AtomicU32::new(0)),
        good: Mutex::new(None),
    });
    let good_provider: Arc<dyn Provider> = Arc::new(Scripted {
        steps: Mutex::new(VecDeque::from(vec![Step::Text("from the fallback".into())])),
    });
    let mut cfg = AgentConfig::new("sys");
    cfg.max_retries = 0;
    cfg.request_timeout = Some(std::time::Duration::from_millis(150));
    cfg.run_retry_attempts = 0;
    cfg.ladder = vec![(good_provider, "good-model".into())];
    cfg.ladder_provider_names = vec!["good".into()];
    std::mem::forget(dir);
    let mut agent = Agent::new(hung_provider, log, cfg);

    let outcome = agent
        .run(
            "go",
            &Default::default(),
            CancellationToken::new(),
            mpsc::channel(64).0,
        )
        .await;
    match outcome {
        TurnOutcome::Completed { response } => {
            assert_eq!(response.text_content(), "from the fallback");
        }
        other => panic!("expected the fallback leg to complete, got {other:?}"),
    }
}

/// A step that exhausts its retry budget by timing out is a transient
/// (`LlmError::Network`) failure at the run level too, since the run's own
/// `cancel` was never touched by the per-attempt timeout: run-level
/// endurance gets a real re-attempt instead of seeing an Aborted turn.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn run_level_endurance_re_attempts_after_a_hung_step() {
    let dir = tempdir().unwrap();
    let log = SessionLog::create(
        dir.path().join("s.jsonl"),
        hung_header("hung", "hung-endurance", dir.path()),
    )
    .unwrap();
    let attempts = Arc::new(std::sync::atomic::AtomicU32::new(0));
    let provider = Arc::new(HungThenGood {
        // Hangs on the first step-level attempt only; the run-level
        // re-attempt's own first (and only) provider call succeeds.
        hangs_remaining: Mutex::new(1),
        attempts: attempts.clone(),
        attempt_cancellations: Arc::new(std::sync::atomic::AtomicU32::new(0)),
        good: Mutex::new(Some(text_msg_for_hung("recovered"))),
    });
    let mut cfg = AgentConfig::new("sys");
    cfg.max_retries = 0;
    cfg.request_timeout = Some(std::time::Duration::from_millis(150));
    cfg.run_retry_attempts = 1;
    cfg.run_retry_base_backoff_ms = 1;
    std::mem::forget(dir);
    let mut agent = Agent::new(provider, log, cfg);

    let outcome = agent
        .run(
            "go",
            &Default::default(),
            CancellationToken::new(),
            mpsc::channel(64).0,
        )
        .await;
    match outcome {
        TurnOutcome::Completed { response } => {
            assert_eq!(response.text_content(), "recovered");
        }
        other => panic!("expected run-level endurance to recover, got {other:?}"),
    }
    assert_eq!(attempts.load(std::sync::atomic::Ordering::SeqCst), 2);
}

/// A real user cancellation during a hung attempt still aborts immediately
/// -- the per-attempt child token derived from `cancel` must observe the
/// parent's cancellation (propagation is one-directional: parent cancels
/// child, never the reverse).
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn user_cancel_during_a_hung_attempt_gives_aborted() {
    let dir = tempdir().unwrap();
    let log = SessionLog::create(
        dir.path().join("s.jsonl"),
        hung_header("hung", "hung-cancel", dir.path()),
    )
    .unwrap();
    let provider = Arc::new(HungThenGood {
        hangs_remaining: Mutex::new(u32::MAX),
        attempts: Arc::new(std::sync::atomic::AtomicU32::new(0)),
        attempt_cancellations: Arc::new(std::sync::atomic::AtomicU32::new(0)),
        good: Mutex::new(None),
    });
    let mut cfg = AgentConfig::new("sys");
    cfg.max_retries = 3;
    // No request_timeout: only the user's own cancellation can end this.
    cfg.run_retry_attempts = 0;
    std::mem::forget(dir);
    let mut agent = Agent::new(provider, log, cfg);

    let cancel = CancellationToken::new();
    let canceller = cancel.clone();
    tokio::spawn(async move {
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
        canceller.cancel();
    });

    let outcome = tokio::time::timeout(
        std::time::Duration::from_secs(5),
        agent.run("go", &Default::default(), cancel, mpsc::channel(64).0),
    )
    .await
    .expect("a user cancel must not hang");
    assert!(
        matches!(outcome, TurnOutcome::Aborted { .. }),
        "expected Aborted, got {outcome:?}"
    );
}
