#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

//! Budget admission (docs/design/27 Phase D): denial fails the run
//! permanently with a typed budget message unless the approver accepts
//! the one-time raise Ask; unattended (no approver) always aborts.

use std::sync::Arc;
use std::sync::atomic::{AtomicU8, Ordering};

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
    }
}

struct Success;

#[async_trait::async_trait]
impl Provider for Success {
    fn name(&self) -> &str {
        "ok"
    }

    async fn stream(
        &self,
        _request: ChatRequest,
        _cancel: CancellationToken,
    ) -> Result<EventStream, LlmError> {
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

#[async_trait::async_trait]
impl SpendGate for FlakyGate {
    async fn authorize(&self, _check: &SpendCheck<'_>) -> Result<(), String> {
        if self.remaining_denials.fetch_sub(1, Ordering::SeqCst) > 0 {
            Err("run budget $1.00 would be exceeded".into())
        } else {
            Ok(())
        }
    }

    fn record_settled(&self, _model: &str, _session_id: &str, _usage: &Usage) {}
}

struct AutoApproveBudget;

#[async_trait::async_trait]
impl Approver for AutoApproveBudget {
    async fn approve(&self, tool: &str, _args_json: &str, _reason: &str) -> bool {
        tool == "finops-budget"
    }
}

fn setup(
    approver: Option<Arc<dyn Approver>>,
    gate: Arc<dyn SpendGate>,
) -> (Agent, tempfile::TempDir) {
    let dir = tempdir().unwrap();
    let cwd = dir.path().to_path_buf();
    let header = SessionHeader {
        session_id: "spend".into(),
        created_at: chrono::Utc::now(),
        cwd: cwd.clone(),
        parent_session_id: None,
        contract: FrozenContract {
            app_version: "0".into(),
            provider: "ok".into(),
            model: "test-model".into(),
            system_prompt: "sys".into(),
            tools: vec![],
            permission_mode: "workspace-write".into(),
            skills: Vec::new(),
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
    (Agent::new(Arc::new(Success), log, cfg), dir)
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
    let (mut agent, _dir) = setup(
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
    let (mut agent, _dir) = setup(
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
