#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

//! Work receipts + dispatch ceiling (docs/design/42-managed-work-contracts.md): every provider
//! dispatch lands as a typed ledger entry on every exit path; the ceiling
//! fails closed without another paid call.

use std::path::Path;
use std::sync::Arc;
use std::sync::atomic::{AtomicU32, Ordering};

use tempfile::tempdir;
use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;

use vak_agent::{Agent, AgentConfig, AutoApprove, SteeringQueues, TurnOutcome};
use vak_llm::stream;
use vak_llm::types::{AssistantMessage, ChatRequest, ContentBlock, StopReason, Usage};
use vak_llm::{
    AttemptReason, EventStream, FailureDomain, LlmError, Provider, Settlement, WorkPurpose,
};
use vak_permission::{Mode, PermissionEngine};
use vak_session::types::{EntryPayload, FrozenContract, SessionHeader};
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
        model: "test-model".into(),
        response_id: None,
    }
}

/// Always fails with a rate-limit error: informed transience feeding
/// retries/endurance but never tripping the breaker.
struct AlwaysRateLimited;

#[async_trait::async_trait]
impl Provider for AlwaysRateLimited {
    fn name(&self) -> &str {
        "always-429"
    }

    async fn stream(
        &self,
        _request: ChatRequest,
        _cancel: CancellationToken,
    ) -> Result<EventStream, LlmError> {
        let (mut sink, rx) = stream::channel(64);
        sink.close_error(LlmError::RateLimit {
            message: "slow down".into(),
            retry_after_secs: None,
        })
        .await;
        Ok(rx)
    }
}

/// Succeeds after N failed dispatches (network resets).
struct FailThenSucceed {
    remaining_failures: AtomicU32,
}

#[async_trait::async_trait]
impl Provider for FailThenSucceed {
    fn name(&self) -> &str {
        "flaky"
    }

    async fn stream(
        &self,
        _request: ChatRequest,
        _cancel: CancellationToken,
    ) -> Result<EventStream, LlmError> {
        let (mut sink, rx) = stream::channel(64);
        if self.remaining_failures.fetch_sub(1, Ordering::SeqCst) > 0 {
            sink.close_error(LlmError::Network("conn reset".into()))
                .await;
        } else {
            let m = text_msg("done");
            sink.push(stream::StreamEvent::Start { partial: m.clone() });
            sink.close_message(m).await;
        }
        Ok(rx)
    }
}

/// Aborts mid-stream with partial output.
struct AbortMidStream;

#[async_trait::async_trait]
impl Provider for AbortMidStream {
    fn name(&self) -> &str {
        "aborting"
    }

    async fn stream(
        &self,
        _request: ChatRequest,
        _cancel: CancellationToken,
    ) -> Result<EventStream, LlmError> {
        let (mut sink, rx) = stream::channel(64);
        sink.close_error(LlmError::Aborted {
            partial: Some(Box::new(text_msg("partial answer"))),
        })
        .await;
        Ok(rx)
    }
}

fn session_paths(dir: &Path) -> (std::path::PathBuf, std::path::PathBuf) {
    let cwd = dir.to_path_buf();
    let home = cwd.join(".vak-home");
    (
        home.clone(),
        SessionPath::new_session_file(&home, &cwd, "receipts"),
    )
}

fn setup_with(
    provider: Arc<dyn Provider>,
    customize: impl FnOnce(&mut AgentConfig),
) -> (Agent, tempfile::TempDir, std::path::PathBuf) {
    let dir = tempdir().unwrap();
    let (home, path) = session_paths(dir.path());
    std::fs::create_dir_all(&home).unwrap();
    let header = SessionHeader {
        agent: None,
        session_id: "receipts".into(),
        created_at: chrono::Utc::now(),
        cwd: dir.path().to_path_buf(),
        parent_session_id: None,
        contract_id: None,
        work_item_id: None,
        conversation: None,
        contract: FrozenContract {
            app_version: "0".into(),
            provider: provider.name().to_string(),
            model: "test-model".into(),
            route_ladder: Vec::new(),
            route_objective: String::new(),
            route_annotations: Vec::new(),
            system_prompt: "sys".into(),
            permission_mode: "workspace-write".into(),
            capabilities: Vec::new(),
            prompt_layers: Vec::new(),
        },
    };
    let log = SessionLog::create(path, header).unwrap();
    let mut cfg = AgentConfig::new("sys");
    cfg.model = "test-model".into();
    cfg.mode = Mode::FullAccess;
    cfg.permission = Some(Arc::new(PermissionEngine::default()));
    cfg.approver = Some(Arc::new(AutoApprove));
    // Keep tests fast regardless of backoff math.
    cfg.retry_base_backoff_ms = 1;
    cfg.run_retry_base_backoff_ms = 1;
    customize(&mut cfg);
    (Agent::new(provider, log, cfg), dir, home)
}

async fn run(agent: &mut Agent) -> TurnOutcome {
    let (ev_tx, mut ev_rx) = mpsc::channel(256);
    tokio::spawn(async move { while ev_rx.recv().await.is_some() {} });
    let cancel = CancellationToken::new();
    let steering = SteeringQueues::new();
    agent.run("hello", &steering, cancel, ev_tx).await
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn success_step_writes_execute_receipt_and_projection_ignores_it() {
    let (mut agent, _dir, _home) = setup_with(
        Arc::new(FailThenSucceed {
            remaining_failures: AtomicU32::new(1),
        }),
        |_| {},
    );
    let outcome = run(&mut agent).await;
    assert!(matches!(outcome, TurnOutcome::Completed { .. }));

    let session = agent.into_session().await;

    // Model-visible means logged — and ONLY that: receipts never enter the
    // projection.
    assert_eq!(session.derive_messages().len(), 2, "prompt + reply only");

    let receipts = session.receipts();
    assert_eq!(receipts.len(), 1, "one work unit => one receipt");
    let r = receipts[0];
    assert_eq!(r.purpose, WorkPurpose::Execute);
    assert_eq!(r.attempts.len(), 2, "network failure, then success");
    assert_eq!(r.attempts[0].reason, AttemptReason::Initial);
    assert_eq!(r.attempts[0].domain, FailureDomain::Network);
    assert_eq!(r.attempts[0].settlement, Settlement::Unknown);
    assert_eq!(r.attempts[1].reason, AttemptReason::Retry);
    assert_eq!(r.attempts[1].settlement, Settlement::Ok);
    assert_eq!(r.winning_attempt, Some(1));
    assert_eq!(
        r.attempts[1].usage.as_ref().map(|u| u.input_tokens),
        Some(5)
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn ceiling_exhaustion_fails_closed_with_failure_receipt() {
    let (mut agent, _dir, _home) = setup_with(Arc::new(AlwaysRateLimited), |cfg| {
        cfg.max_retries = 1;
        cfg.run_retry_attempts = 0;
        cfg.dispatch_ceiling = 2;
    });
    let outcome = run(&mut agent).await;
    match outcome {
        TurnOutcome::Failed { error } => {
            assert!(
                error.to_string().contains("dispatch ceiling of 2"),
                "unexpected error: {error}"
            );
        }
        other => panic!("expected Failed at ceiling, got {other:?}"),
    }

    let session = agent.into_session().await;
    let receipts = session.receipts();
    assert_eq!(receipts.len(), 1);
    let r = receipts[0];
    assert_eq!(r.attempts.len(), 2, "initial + one retry, then closed");
    assert!(r.winning_attempt.is_none());
    assert!(
        r.attempts
            .iter()
            .all(|a| a.domain == FailureDomain::Account && a.settlement == Settlement::Failed)
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn mid_stream_abort_records_cancelled_settlement_and_keeps_partial() {
    let (mut agent, _dir, _home) = setup_with(Arc::new(AbortMidStream), |_| {});
    let outcome = run(&mut agent).await;
    assert!(matches!(outcome, TurnOutcome::Aborted { .. }));

    let session = agent.into_session().await;
    let messages = session.derive_messages();
    assert!(
        messages
            .iter()
            .any(|m| m.text_content().contains("partial answer")),
        "partial output survives abort (invariant 5)"
    );

    let receipts = session.receipts();
    assert_eq!(receipts.len(), 1);
    let attempts = &receipts[0].attempts;
    assert_eq!(attempts.len(), 1);
    assert_eq!(attempts[0].settlement, Settlement::Cancelled);
    assert!(receipts[0].winning_attempt.is_none());
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn receipts_round_trip_through_disk() {
    let (mut agent, dir, _home) = setup_with(
        Arc::new(FailThenSucceed {
            remaining_failures: AtomicU32::new(0),
        }),
        |_| {},
    );
    let outcome = run(&mut agent).await;
    assert!(matches!(outcome, TurnOutcome::Completed { .. }));

    let (_, path) = session_paths(dir.path());
    let session = agent.into_session().await;
    let kinds: Vec<&str> = session
        .chain_to_root()
        .iter()
        .map(|e| match e.payload {
            EntryPayload::Header(_) => "header",
            EntryPayload::Message(_) => "message",
            EntryPayload::Compaction(_) => "compaction",
            EntryPayload::Receipt(_) => "receipt",
            EntryPayload::Goal(_) => "goal",
            EntryPayload::GoalUpdate(_) => "goal-update",
            EntryPayload::Activity(_) => "activity",
            EntryPayload::Work(_) => "work",
            EntryPayload::Intent(_) => "intent",
            EntryPayload::TurnCapabilitiesBound(_) => "capabilities",
            EntryPayload::ChildRun { .. } => "child-run",
        })
        .collect();
    assert_eq!(
        kinds,
        vec!["header", "message", "receipt", "message"],
        "receipt sits between the prompt and the reply it produced"
    );
    drop(session);

    let reopened = SessionLog::open(path).unwrap();
    assert_eq!(reopened.receipts().len(), 1, "audit survives restart");
    assert_eq!(reopened.derive_messages().len(), 2, "projection unchanged");
    assert!(reopened.warnings().is_empty());
}
