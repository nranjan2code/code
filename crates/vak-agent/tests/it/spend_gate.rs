#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

//! Budget admission (docs/design/15-reliability.md): denial fails the run
//! permanently with a typed budget message unless the approver accepts
//! the one-time raise Ask; unattended (no approver) always aborts.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU8, AtomicU32, Ordering};

use tempfile::tempdir;
use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;

use vak_agent::{Agent, AgentConfig, Approver, SpendCheck, SpendGate, SteeringQueues, TurnOutcome};
use vak_llm::stream;
use vak_llm::types::{AssistantMessage, ChatRequest, ContentBlock, StopReason, Usage};
use vak_llm::{EventStream, LlmError, Provider};
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
        model: "test-model".into(),
        response_id: None,
        applied: None,
    }
}

struct Success {
    capacity_key: String,
    dispatches: Arc<AtomicU32>,
}

#[async_trait::async_trait]
impl Provider for Success {
    fn name(&self) -> &str {
        "ok"
    }

    fn rate_limit_key(&self) -> String {
        self.capacity_key.clone()
    }

    async fn stream(
        &self,
        _request: ChatRequest,
        _cancel: CancellationToken,
    ) -> Result<EventStream, LlmError> {
        self.dispatches.fetch_add(1, Ordering::SeqCst);
        let (mut sink, rx) = stream::channel(64);
        let m = text_msg("done");
        sink.push(stream::StreamEvent::Start { partial: m.clone() });
        sink.close_message(m).await;
        Ok(rx)
    }
}

/// Denies the first authorize call N times.
struct FlakyGate {
    remaining_denials: AtomicU8,
}

struct AlwaysDenyGate;

#[async_trait::async_trait]
impl SpendGate for AlwaysDenyGate {
    async fn authorize(&self, _check: &SpendCheck<'_>) -> Result<(), String> {
        Err("daily budget remains exceeded".into())
    }

    fn record_settled(&self, _provider: &str, _model: &str, _session_id: &str, _usage: &Usage) {}
}

#[async_trait::async_trait]
impl SpendGate for FlakyGate {
    async fn authorize(&self, _check: &SpendCheck<'_>) -> Result<(), String> {
        if self.remaining_denials.fetch_sub(1, Ordering::SeqCst) > 0 {
            Err("run budget $1.00 would be exceeded".into())
        } else {
            Ok(())
        }
    }

    fn record_settled(&self, _provider: &str, _model: &str, _session_id: &str, _usage: &Usage) {}
}

struct AutoApproveBudget;

#[async_trait::async_trait]
impl Approver for AutoApproveBudget {
    async fn approve(
        &self,
        tool: &str,
        _args_json: &str,
        _reason: &str,
        _call_id: Option<&str>,
    ) -> bool {
        tool == "finops-budget"
    }
}

fn setup(
    approver: Option<Arc<dyn Approver>>,
    gate: Arc<dyn SpendGate>,
) -> (Agent, tempfile::TempDir, Arc<AtomicU32>) {
    let dir = tempdir().unwrap();
    let cwd = dir.path().to_path_buf();
    let header = SessionHeader {
        space: None,
        run: None,
        cause: None,
        agent: None,
        session_id: "spend".into(),
        created_at: chrono::Utc::now(),
        cwd: cwd.clone(),
        parent_session_id: None,
        contract_id: None,
        work_item_id: None,
        conversation: None,
        contract: FrozenContract {
            app_version: "0".into(),
            provider: "ok".into(),
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
    let home = cwd.join(".vak-home");
    std::fs::create_dir_all(&home).unwrap();
    let log =
        SessionLog::create(SessionPath::new_session_file(&home, &cwd, "spend"), header).unwrap();
    let mut cfg = AgentConfig::new("sys");
    cfg.model = "test-model".into();
    cfg.mode = Mode::FullAccess;
    cfg.permission = Some(Arc::new(PermissionEngine::default()));
    cfg.approver = approver;
    cfg.spend_gate = Some(gate);
    cfg.retry_base_backoff_ms = 1;
    cfg.run_retry_base_backoff_ms = 1;
    let dispatches = Arc::new(AtomicU32::new(0));
    (
        Agent::new(
            Arc::new(Success {
                capacity_key: crate::support::capacity_key(),
                dispatches: dispatches.clone(),
            }),
            log,
            cfg,
        ),
        dir,
        dispatches,
    )
}

async fn run(agent: &mut Agent) -> TurnOutcome {
    let (ev_tx, mut ev_rx) = mpsc::channel(256);
    tokio::spawn(async move { while ev_rx.recv().await.is_some() {} });
    let cancel = CancellationToken::new();
    let steering = SteeringQueues::new();
    agent.run("hello", &steering, cancel, ev_tx).await
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn budget_denial_without_approver_fails_permanently() {
    let (mut agent, _dir, _) = setup(
        None,
        Arc::new(FlakyGate {
            remaining_denials: AtomicU8::new(9),
        }),
    );
    match run(&mut agent).await {
        TurnOutcome::Failed { error } => {
            assert!(
                error.to_string().contains("budget admission denied"),
                "{error}"
            );
        }
        other => panic!("expected Failed, got {other:?}"),
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn approved_budget_ask_lets_the_run_proceed() {
    let (mut agent, _dir, _) = setup(
        Some(Arc::new(AutoApproveBudget)),
        Arc::new(FlakyGate {
            remaining_denials: AtomicU8::new(1),
        }),
    );
    let outcome = run(&mut agent).await;
    assert!(
        matches!(outcome, TurnOutcome::Completed { .. }),
        "approved ask must proceed, got {outcome:?}"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn approved_budget_recheck_error_stops_before_provider_dispatch() {
    let (mut agent, _dir, dispatches) =
        setup(Some(Arc::new(AutoApproveBudget)), Arc::new(AlwaysDenyGate));

    match run(&mut agent).await {
        TurnOutcome::Failed { error } => assert!(
            error
                .to_string()
                .contains("budget admission recheck failed")
                && error.to_string().contains("daily budget remains exceeded"),
            "the failed recheck must be surfaced clearly, got {error}"
        ),
        other => panic!("expected failed budget recheck, got {other:?}"),
    }
    assert_eq!(
        dispatches.load(Ordering::SeqCst),
        0,
        "an approved ask cannot bypass a denied second admission"
    );
}

/// Always denies until on_budget_approved() flips it.
struct CapGate {
    raised: AtomicBool,
    denials: AtomicU32,
}

#[async_trait::async_trait]
impl SpendGate for CapGate {
    async fn authorize(&self, _check: &SpendCheck<'_>) -> Result<(), String> {
        if !self.raised.load(Ordering::SeqCst) {
            self.denials.fetch_add(1, Ordering::SeqCst);
            Err("run budget $1.00 would be exceeded".into())
        } else {
            Ok(())
        }
    }

    fn on_budget_approved(&self) {
        self.raised.store(true, Ordering::SeqCst);
    }

    fn record_settled(&self, _provider: &str, _model: &str, _session_id: &str, _usage: &Usage) {}
}

struct ApproveOnce {
    remaining: AtomicU32,
}

#[async_trait::async_trait]
impl Approver for ApproveOnce {
    async fn approve(
        &self,
        tool: &str,
        _args_json: &str,
        _reason: &str,
        _call_id: Option<&str>,
    ) -> bool {
        tool == "finops-budget" && self.remaining.fetch_sub(1, Ordering::SeqCst) > 0
    }
}

/// Approval raises the cap for the WHOLE run: later dispatches pass
/// without re-asking (raise-cap-once, docs/design/15-reliability.md).
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn budget_approval_raises_cap_for_rest_of_run() {
    let gate = Arc::new(CapGate {
        raised: AtomicBool::new(false),
        denials: AtomicU32::new(0),
    });
    let dir = tempdir().unwrap();
    let cwd = dir.path().to_path_buf();
    let header = SessionHeader {
        space: None,
        run: None,
        cause: None,
        agent: None,
        session_id: "raise".into(),
        created_at: chrono::Utc::now(),
        cwd: cwd.clone(),
        parent_session_id: None,
        contract_id: None,
        work_item_id: None,
        conversation: None,
        contract: FrozenContract {
            app_version: "0".into(),
            provider: "ok".into(),
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
    let home = cwd.join(".vak-home");
    std::fs::create_dir_all(&home).unwrap();
    let log =
        SessionLog::create(SessionPath::new_session_file(&home, &cwd, "raise"), header).unwrap();
    let mut cfg = AgentConfig::new("sys");
    cfg.model = "test-model".into();
    cfg.mode = Mode::FullAccess;
    cfg.permission = Some(Arc::new(PermissionEngine::default()));
    cfg.approver = Some(Arc::new(ApproveOnce {
        remaining: AtomicU32::new(1),
    }));
    cfg.spend_gate = Some(gate.clone() as Arc<dyn SpendGate>);
    cfg.retry_base_backoff_ms = 1;
    let mut agent = Agent::new(
        Arc::new(Success {
            capacity_key: crate::support::capacity_key(),
            dispatches: Arc::new(AtomicU32::new(0)),
        }),
        log,
        cfg,
    );

    let (ev_tx, mut ev_rx) = mpsc::channel(256);
    tokio::spawn(async move { while ev_rx.recv().await.is_some() {} });
    let outcome = agent
        .run(
            "hello",
            &SteeringQueues::new(),
            CancellationToken::new(),
            ev_tx,
        )
        .await;
    assert!(matches!(outcome, TurnOutcome::Completed { .. }));
    assert_eq!(gate.denials.load(Ordering::SeqCst), 1, "exactly one denial");
}

/// Never answers; says when it was asked.
struct Silent(Arc<tokio::sync::Notify>);

#[async_trait::async_trait]
impl Approver for Silent {
    async fn approve(
        &self,
        _tool: &str,
        _args_json: &str,
        _reason: &str,
        _call_id: Option<&str>,
    ) -> bool {
        self.0.notify_one();
        std::future::pending().await
    }
}

/// A run cancelled while a gate is open ends without an answer: a stop or a
/// mode change that lands before the gate is listed cannot deny it, so the
/// gate itself gives way to the run's cancel token (invariants 5 and 11).
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_cancelled_run_stops_waiting_on_an_open_gate() {
    let asked = Arc::new(tokio::sync::Notify::new());
    let (mut agent, _dir, _) = setup(
        Some(Arc::new(Silent(asked.clone()))),
        Arc::new(FlakyGate {
            remaining_denials: AtomicU8::new(9),
        }),
    );
    let (ev_tx, mut ev_rx) = mpsc::channel(256);
    tokio::spawn(async move { while ev_rx.recv().await.is_some() {} });
    let cancel = CancellationToken::new();
    let steering = SteeringQueues::new();
    let stop = cancel.clone();
    tokio::spawn(async move {
        asked.notified().await;
        stop.cancel();
    });
    let outcome = tokio::time::timeout(
        std::time::Duration::from_secs(30),
        agent.run("hello", &steering, cancel, ev_tx),
    )
    .await
    .expect("the run must not wait on a gate after it was cancelled");
    assert!(
        !matches!(outcome, TurnOutcome::Completed { .. }),
        "{outcome:?}"
    );
}
