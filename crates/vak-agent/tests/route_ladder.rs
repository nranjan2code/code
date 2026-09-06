#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

//! Frozen-ladder routing (docs/design/15-reliability.md): dispatch walks the
//! frozen candidate legs on typed failures; ceiling/receipts/endurance
//! are shared across legs; walking the ladder is contract execution.

use std::collections::VecDeque;
use std::sync::Arc;
use std::sync::atomic::{AtomicU32, Ordering};

use tempfile::tempdir;
use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;

use vak_agent::{Agent, AgentConfig, AutoApprove, SteeringQueues, TurnOutcome};
use vak_llm::stream;
use vak_llm::types::{AssistantMessage, ChatRequest, ContentBlock, StopReason, Usage};
use vak_llm::{AttemptReason, EventStream, LlmError, Provider};
use vak_permission::{Mode, PermissionEngine};
use vak_session::types::{FrozenContract, SessionHeader};
use vak_session::{SessionLog, SessionPath};

fn text_msg(t: &str) -> AssistantMessage {
    AssistantMessage {
        content: vec![ContentBlock::text(t)],
        stop_reason: StopReason::EndTurn,
        usage: Usage {
            input_tokens: 5,
            output_tokens: 3,
            ..Default::default()
        },
        model: "fallback-model".into(),
    }
}

/// Always fails with a network error (blind failure domain).
struct AlwaysNetwork {
    calls: AtomicU32,
}

struct PanickingProvider;

struct TerminalQuota;

#[async_trait::async_trait]
impl Provider for TerminalQuota {
    fn name(&self) -> &str {
        "quota-limited"
    }

    async fn stream(
        &self,
        _request: ChatRequest,
        _cancel: CancellationToken,
    ) -> Result<EventStream, LlmError> {
        let (mut sink, rx) = stream::channel(64);
        sink.close_error(LlmError::RateLimit {
            message: "free-models-per-day exhausted".into(),
            retry_after_secs: None,
        })
        .await;
        Ok(rx)
    }
}

#[async_trait::async_trait]
impl Provider for PanickingProvider {
    fn name(&self) -> &str {
        "primary-panics"
    }

    async fn stream(
        &self,
        _request: ChatRequest,
        _cancel: CancellationToken,
    ) -> Result<EventStream, LlmError> {
        panic!("provider adapter panic");
    }
}

#[async_trait::async_trait]
impl Provider for AlwaysNetwork {
    fn name(&self) -> &str {
        "primary-net-dead"
    }

    async fn stream(
        &self,
        _request: ChatRequest,
        _cancel: CancellationToken,
    ) -> Result<EventStream, LlmError> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        let (mut sink, rx) = stream::channel(64);
        sink.close_error(LlmError::Network("conn reset".into()))
            .await;
        Ok(rx)
    }
}

struct Scripted {
    responses: std::sync::Mutex<VecDeque<AssistantMessage>>,
}

#[async_trait::async_trait]
impl Provider for Scripted {
    fn name(&self) -> &str {
        "fallback-ok"
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
            None => {
                sink.close_error(LlmError::Parse("script exhausted".into()))
                    .await
            }
        }
        Ok(rx)
    }
}

fn setup_with_primary(
    primary: Arc<dyn Provider>,
    ladder: Vec<(Arc<dyn Provider>, String)>,
) -> (Agent, tempfile::TempDir) {
    let dir = tempdir().unwrap();
    let cwd = dir.path().to_path_buf();
    let header = SessionHeader {
        session_id: "ladder".into(),
        created_at: chrono::Utc::now(),
        cwd: cwd.clone(),
        parent_session_id: None,
        contract_id: None,
        work_item_id: None,
        contract: FrozenContract {
            app_version: "0".into(),
            provider: "primary-net-dead".into(),
            model: "primary-model".into(),
            route_ladder: Vec::new(),
            route_objective: String::new(),
            route_annotations: Vec::new(),
            system_prompt: "sys".into(),
            permission_mode: "workspace-write".into(),
            capabilities: Vec::new(),
            prompt_layers: Vec::new(),
        },
    };
    let home = cwd.join(".vak-home");
    std::fs::create_dir_all(&home).unwrap();
    let log =
        SessionLog::create(SessionPath::new_session_file(&home, &cwd, "ladder"), header).unwrap();
    let mut cfg = AgentConfig::new("sys");
    cfg.model = "primary-model".into();
    cfg.mode = Mode::FullAccess;
    cfg.permission = Some(Arc::new(PermissionEngine::default()));
    cfg.approver = Some(Arc::new(AutoApprove));
    cfg.retry_base_backoff_ms = 1;
    cfg.run_retry_base_backoff_ms = 1;
    cfg.max_retries = 0; // one dispatch per leg keeps counting exact
    cfg.run_retry_attempts = 0;
    cfg.dispatch_ceiling = 3;
    cfg.ladder = ladder;
    cfg.ladder_provider_names = vec!["openrouter".into(); cfg.ladder.len()];
    (Agent::new(primary, log, cfg), dir)
}

fn setup(ladder: Vec<(Arc<dyn Provider>, String)>) -> (Agent, tempfile::TempDir) {
    setup_with_primary(
        Arc::new(AlwaysNetwork {
            calls: AtomicU32::new(0),
        }),
        ladder,
    )
}

async fn run(agent: &mut Agent) -> TurnOutcome {
    let (ev_tx, mut ev_rx) = mpsc::channel(512);
    tokio::spawn(async move { while ev_rx.recv().await.is_some() {} });
    let cancel = CancellationToken::new();
    let steering = SteeringQueues::new();
    agent.run("hello", &steering, cancel, ev_tx).await
}

fn fallback_provider() -> Arc<Scripted> {
    Arc::new(Scripted {
        responses: std::sync::Mutex::new(VecDeque::from(vec![text_msg("answered via fallback")])),
    })
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn primary_failure_walks_to_fallback_and_receipt_records_it() {
    let fb = fallback_provider();
    let model_fb = "fallback-model".to_string();
    let (mut agent, _dir) = setup(vec![(fb.clone() as Arc<dyn Provider>, model_fb.clone())]);
    let outcome = run(&mut agent).await;
    assert!(
        matches!(outcome, TurnOutcome::Completed { .. }),
        "fallback must rescue the step"
    );

    let session = agent.into_session().await;
    assert_eq!(session.receipts().len(), 1);
    let r = &session.receipts()[0];
    assert_eq!(r.model, "fallback-model", "receipt names the winning leg");
    assert_eq!(
        r.provider, "openrouter",
        "receipt preserves configured route identity"
    );
    assert_eq!(r.winning_attempt, Some(1));
    assert_eq!(r.attempts[0].reason, AttemptReason::Initial);
    assert_eq!(r.attempts[0].domain, vak_llm::FailureDomain::Network);
    assert_eq!(
        r.attempts[1].reason,
        AttemptReason::RouteFallback,
        "first dispatch of the next leg is the fallback reason"
    );
    assert_eq!(r.attempts[1].settlement, vak_llm::Settlement::Ok);

    // Shared ceiling: exactly two paid dispatches consumed.
    // (asserted indirectly: a third dispatch would have exceeded ceiling=3? no)
    // Projection stays clean.
    assert_eq!(session.derive_messages().len(), 2);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn provider_panic_is_contained_and_fallback_completes() {
    let fb = fallback_provider();
    let (mut agent, _dir) = setup_with_primary(
        Arc::new(PanickingProvider),
        vec![(fb as Arc<dyn Provider>, "fallback-model".to_string())],
    );

    assert!(matches!(
        run(&mut agent).await,
        TurnOutcome::Completed { .. }
    ));
    let session = agent.into_session().await;
    let receipt = &session.receipts()[0];
    assert_eq!(receipt.attempts.len(), 2);
    assert_eq!(receipt.attempts[0].domain, vak_llm::FailureDomain::Network);
    assert_eq!(receipt.attempts[1].reason, AttemptReason::RouteFallback);
    assert_eq!(receipt.winning_attempt, Some(1));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn terminal_quota_walks_to_fallback_without_retry_storm() {
    let fb = fallback_provider();
    let (mut agent, _dir) = setup_with_primary(
        Arc::new(TerminalQuota),
        vec![(fb as Arc<dyn Provider>, "fallback-model".to_string())],
    );

    assert!(matches!(
        run(&mut agent).await,
        TurnOutcome::Completed { .. }
    ));
    let session = agent.into_session().await;
    let receipt = &session.receipts()[0];
    assert_eq!(receipt.attempts.len(), 2);
    assert_eq!(receipt.attempts[0].reason, AttemptReason::Initial);
    assert_eq!(receipt.attempts[1].reason, AttemptReason::RouteFallback);
    assert_eq!(receipt.winning_attempt, Some(1));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn all_legs_exhausted_fails_closed_within_ceiling() {
    // Fallback also fails (script empty => Parse error, non-retryable).
    let dead_fb = Arc::new(Scripted {
        responses: std::sync::Mutex::new(VecDeque::new()),
    });
    let (mut agent, _dir) = setup(vec![(
        dead_fb as Arc<dyn Provider>,
        "fallback-model".to_string(),
    )]);
    match run(&mut agent).await {
        TurnOutcome::Failed { error } => {
            assert!(!error.to_string().is_empty());
        }
        other => panic!("expected Failed after all legs, got {other:?}"),
    }
    let session = agent.into_session().await;
    let r = &session.receipts()[0];
    assert_eq!(r.attempts.len(), 2, "one dispatch per leg at max_retries=0");
    assert_eq!(r.attempts[1].reason, AttemptReason::RouteFallback);
    assert!(r.winning_attempt.is_none());
    assert!(
        r.attempts
            .iter()
            .all(|a| a.settlement == vak_llm::Settlement::Unknown),
        "transport ambiguity stays UNKNOWN"
    );
}

/// Phase R: per-attempt leg attribution survives a cross-model walk, and
/// consumers see one RouteFallback event per leg change.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn fallback_legs_carry_provider_attribution_and_events() {
    let fb = fallback_provider();
    let (mut agent, _dir) = setup(vec![(
        fb as Arc<dyn Provider>,
        "fallback-model".to_string(),
    )]);

    let (ev_tx, mut ev_rx) = mpsc::channel(512);
    let events = tokio::spawn(async move {
        let mut fallbacks = Vec::new();
        while let Some(ev) = ev_rx.recv().await {
            if let vak_agent::AgentEvent::RouteFallback {
                to_provider,
                to_model,
            } = ev
            {
                fallbacks.push((to_provider, to_model));
            }
        }
        fallbacks
    });

    let cancel = CancellationToken::new();
    let steering = SteeringQueues::new();
    let outcome = agent.run("hello", &steering, cancel, ev_tx).await;
    assert!(matches!(outcome, TurnOutcome::Completed { .. }));

    let session = agent.into_session().await;
    let r = &session.receipts()[0];
    // Receipt-level stamp names the WINNING leg.
    assert_eq!(
        (r.provider.as_str(), r.model.as_str()),
        ("openrouter", "fallback-model")
    );
    // Attempt 0 keeps its own (failed) leg attribution.
    assert_eq!(
        r.attempt_leg(&r.attempts[0]),
        ("primary-net-dead", "primary-model")
    );
    // Attempt 1 attributes to the serving fallback leg.
    assert_eq!(
        r.attempt_leg(&r.attempts[1]),
        ("openrouter", "fallback-model")
    );

    let fallbacks = events.await.unwrap();
    assert_eq!(
        fallbacks,
        vec![("openrouter".into(), "fallback-model".into())],
        "exactly one RouteFallback event for the single leg change"
    );
}
