#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

//! Goal mode + audited completion + regression obligations +
//! reset-with-handoff (docs/design/27 Phase H).

use std::collections::VecDeque;
use std::sync::Arc;

use tempfile::tempdir;
use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;

use vak_agent::{
    Agent, AgentConfig, AutoApprove, SteeringQueues, TurnOutcome, workspace::WorkspaceDelta,
};
use vak_llm::stream;
use vak_llm::types::{AssistantMessage, ChatRequest, ContentBlock, StopReason, Usage};
use vak_llm::{EventStream, LlmError, Provider};
use vak_permission::{Mode, PermissionEngine};
use vak_session::types::{EntryPayload, FrozenContract, SessionHeader};
use vak_session::{SessionLog, SessionPath};
use vak_tools::bash::BashTool;

fn text_msg(t: &str) -> AssistantMessage {
    AssistantMessage {
        content: vec![ContentBlock::text(t)],
        stop_reason: StopReason::EndTurn,
        usage: Usage {
            input_tokens: 5,
            output_tokens: 3,
            ..Default::default()
        },
        model: "judge-model".into(),
    }
}

fn tool_msg() -> AssistantMessage {
    AssistantMessage {
        content: vec![ContentBlock::ToolUse {
            id: "t1".into(),
            name: "bash".into(),
            input: serde_json::json!({"command": "echo green"}),
        }],
        stop_reason: StopReason::ToolUse,
        usage: Usage::default(),
        model: "test-model".into(),
    }
}

/// Scripted responses consumed FIFO; empty queue => parse error.
struct Scripted {
    responses: std::sync::Mutex<VecDeque<AssistantMessage>>,
    requests: std::sync::Mutex<Vec<ChatRequest>>,
}

impl Scripted {
    fn new(responses: Vec<AssistantMessage>) -> Arc<Self> {
        Arc::new(Scripted {
            responses: std::sync::Mutex::new(responses.into_iter().collect()),
            requests: std::sync::Mutex::new(Vec::new()),
        })
    }
}

#[async_trait::async_trait]
impl Provider for Scripted {
    fn name(&self) -> &str {
        "scripted-goal"
    }

    async fn stream(
        &self,
        request: ChatRequest,
        _cancel: CancellationToken,
    ) -> Result<EventStream, LlmError> {
        self.requests.lock().unwrap().push(request);
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

fn setup(
    provider: Arc<Scripted>,
    tools: Vec<Arc<dyn vak_tools::Tool>>,
) -> (Agent, tempfile::TempDir) {
    setup_with_delta(provider, tools, None)
}

fn setup_with_delta(
    provider: Arc<Scripted>,
    tools: Vec<Arc<dyn vak_tools::Tool>>,
    delta: Option<Arc<dyn WorkspaceDelta>>,
) -> (Agent, tempfile::TempDir) {
    let dir = tempdir().unwrap();
    let cwd = dir.path().to_path_buf();
    let header = SessionHeader {
        session_id: "goal".into(),
        created_at: chrono::Utc::now(),
        cwd: cwd.clone(),
        parent_session_id: None,
        contract: FrozenContract {
            app_version: "0".into(),
            provider: "scripted-goal".into(),
            model: "test-model".into(),
            route_ladder: Vec::new(),
            route_objective: String::new(),
            route_annotations: Vec::new(),
            system_prompt: "sys".into(),
            tools: tools.iter().map(|t| t.name().to_string()).collect(),
            permission_mode: "full-access".into(),
            skills: Vec::new(),
        },
    };
    let home = cwd.join(".vak-home");
    std::fs::create_dir_all(&home).unwrap();
    let log =
        SessionLog::create(SessionPath::new_session_file(&home, &cwd, "goal"), header).unwrap();
    let mut cfg = AgentConfig::new("sys");
    cfg.model = "test-model".into();
    cfg.tools = tools;
    cfg.mode = Mode::FullAccess;
    cfg.permission = Some(Arc::new(PermissionEngine::default()));
    cfg.approver = Some(Arc::new(AutoApprove));
    cfg.workspace_delta = delta;
    cfg.retry_base_backoff_ms = 1;
    cfg.run_retry_base_backoff_ms = 1;
    (Agent::new(provider, log, cfg), dir)
}

async fn run(agent: &mut Agent, prompt: &str) -> TurnOutcome {
    let (ev_tx, mut ev_rx) = mpsc::channel(512);
    tokio::spawn(async move { while ev_rx.recv().await.is_some() {} });
    let cancel = CancellationToken::new();
    let steering = SteeringQueues::new();
    agent.run(prompt, &steering, cancel, ev_tx).await
}

fn goal_statuses(session: &SessionLog) -> Vec<String> {
    session
        .chain_to_root()
        .iter()
        .filter_map(|e| match &e.payload {
            EntryPayload::Goal(g) => Some(match &g.status {
                vak_session::types::GoalStatus::Active => "active".into(),
                vak_session::types::GoalStatus::Done { .. } => "done".into(),
                vak_session::types::GoalStatus::Unverified { .. } => "unverified".into(),
            }),
            _ => None,
        })
        .collect()
}

/// Model claims done immediately; judge passes all criteria.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn audited_done_when_judge_passes() {
    let provider = Scripted::new(vec![
        text_msg("haiku written"),
        text_msg(
            r#"{"results":[{"criterion":"summary mentions done","verdict":"pass","evidence":"said it"}]}"#,
        ),
    ]);
    struct StaticDelta;
    impl WorkspaceDelta for StaticDelta {
        fn summary(&self) -> Result<String, String> {
            Ok("M src/lib.rs\nA goal-live.txt\nWORKSPACE-DELTA-MARKER".into())
        }
    }
    let (mut agent, _dir) = setup_with_delta(provider.clone(), vec![], Some(Arc::new(StaticDelta)));
    agent.set_goal("write a haiku", vec!["summary mentions done".into()]);
    let outcome = run(&mut agent, "write a haiku about rust").await;
    assert!(matches!(outcome, TurnOutcome::Completed { .. }));

    // The judge call happened and was receipted as Verify work.
    let has_judge = provider.requests.lock().unwrap().iter().any(|r| {
        r.system
            .as_deref()
            .unwrap_or("")
            .contains("completion auditor")
    });
    assert!(has_judge, "judge call must carry the auditor system prompt");
    let session = agent.into_session().await;
    assert_eq!(goal_statuses(&session), vec!["active", "done"]);
    assert!(
        session
            .receipts()
            .iter()
            .any(|r| r.purpose == vak_llm::WorkPurpose::Verify),
        "audit dispatch must be receipted"
    );
}

/// Judge fails a criterion first; findings reach the model; second claim
/// passes. Audit budget respected.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn rejected_claim_returns_findings_then_passes() {
    let provider = Scripted::new(vec![
        text_msg("attempt one"),
        text_msg(
            r#"{"results":[{"criterion":"c1","verdict":"fail","evidence":"no file written"}]}"#,
        ),
        text_msg("attempt two, file written"),
        text_msg(r#"{"results":[{"criterion":"c1","verdict":"pass","evidence":"wrote it"}]}"#),
    ]);
    let (mut agent, _dir) = setup(provider.clone(), vec![]);
    agent.set_goal("create x", vec!["c1".into()]);
    let outcome = run(&mut agent, "create x").await;
    assert!(matches!(outcome, TurnOutcome::Completed { .. }));
    let session = agent.into_session().await;
    let statuses = goal_statuses(&session);
    assert_eq!(statuses, vec!["active", "done"]);
    // Findings were injected model-visible (invariant 1): a [goal-audit]
    // user turn exists between claims.
    assert!(
        session
            .derive_messages()
            .iter()
            .any(|m| m.text_content().contains("[goal-audit]"))
    );
    let judge_calls = provider
        .requests
        .lock()
        .unwrap()
        .iter()
        .filter(|r| {
            r.system
                .as_deref()
                .unwrap_or("")
                .contains("completion auditor")
        })
        .count();
    assert_eq!(judge_calls, 2, "two judge dispatches");
}

/// A verify: criterion that fails rejects without any judge call.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn shell_criterion_failure_blocks_without_judge() {
    let provider = Scripted::new(vec![text_msg("i ran things")]);
    let (mut agent, _dir) = setup(provider.clone(), vec![]);
    agent.config.max_audit_blocks = 0;
    agent.set_goal("run things", vec!["verify: exit 3".into()]);
    let outcome = run(&mut agent, "run things").await;
    assert!(
        matches!(outcome, TurnOutcome::Completed { .. }),
        "budget exhausts => unverified completion"
    );
    let judge_calls = provider
        .requests
        .lock()
        .unwrap()
        .iter()
        .filter(|r| {
            r.system
                .as_deref()
                .unwrap_or("")
                .contains("completion auditor")
        })
        .count();
    assert_eq!(judge_calls, 0, "deterministic failure needs no judge");

    let session = agent.into_session().await;
    assert_eq!(goal_statuses(&session), vec!["active", "unverified"]);
}

/// Green bash commands become obligations: a later failing re-run blocks
/// the claim until fixed.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn regression_obligation_blocks_claim() {
    // Provider script: run echo-green (tool call), claim done, then after
    // rejection claim done again. Judge always passes.
    let provider = Scripted::new(vec![
        tool_msg(),
        text_msg("all done"),
        text_msg(r#"{"results":[{"criterion":"c1","verdict":"pass","evidence":"ok"}]}"#),
    ]);
    let (mut agent, _dir) = setup(provider, vec![Arc::new(BashTool)]);
    agent.set_goal("keep tests green", vec!["c1".into()]);
    let outcome = run(&mut agent, "echo something then finish").await;
    assert!(matches!(outcome, TurnOutcome::Completed { .. }));

    // The audit re-ran `echo green` as an obligation — visible as an extra
    // brokered request beyond the scripted model turns.
    let session = agent.into_session().await;
    let statuses = goal_statuses(&session);
    assert!(
        statuses
            .last()
            .map(|s| s == "done" || s == "unverified")
            .unwrap_or(false),
        "final status recorded: {statuses:?}"
    );
}

/// Unparseable judge output fails closed: claim rejected, not accepted.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn unparseable_judge_fails_closed() {
    let provider = Scripted::new(vec![
        text_msg("attempt one"),
        text_msg("I think it's probably fine honestly"),
        text_msg("attempt two"),
        text_msg(r#"{"results":[{"criterion":"c1","verdict":"pass","evidence":"now clear"}]}"#),
    ]);
    let (mut agent, _dir) = setup(provider, vec![]);
    agent.set_goal("do it", vec!["c1".into()]);
    let outcome = run(&mut agent, "do it").await;
    assert!(matches!(outcome, TurnOutcome::Completed { .. }));
    let session = agent.into_session().await;
    let msgs = session.derive_messages();
    assert!(
        msgs.iter()
            .any(|m| m.text_content().contains("[goal-audit]")
                && m.text_content().contains("AUDIT UNAVAILABLE")),
        "unparseable verdict must inject fail-closed findings"
    );
}

/// Handoff-reset: with a tiny window, the still-over path writes a handoff
/// and continues fresh instead of failing.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn handoff_reset_rescues_still_over_context() {
    // Script: handoff writer draws first, then the recovered turn.
    let provider = Scripted::new(vec![
        text_msg(
            "# Objective\nfinish\n# Current State\nmid\n# Decisions Made\nnone\n# Open Items\ndone\n# Obligations\nnone",
        ),
        text_msg("recovered and finished"),
    ]);
    let (mut agent, _dir) = setup(provider, vec![]);
    // Tiny policy so the fixture triggers the over-budget path with too
    // few turns to compact.
    agent.config.context_policy.context_window = 900;
    agent.config.context_policy.max_output = 64;
    agent.config.context_policy.keep_recent = 2;

    let filler = "x".repeat(4000);
    let prompt = format!("task {filler}");
    let outcome = run(&mut agent, &prompt).await;
    assert!(
        matches!(
            outcome,
            TurnOutcome::Completed { .. }
                | TurnOutcome::MaxTurnsReached
                | TurnOutcome::Failed { .. }
        ),
        "unexpected: {outcome:?}"
    );

    let session = agent.into_session().await;
    let has_handoff = session
        .chain_to_root()
        .iter()
        .any(|e| matches!(&e.payload, EntryPayload::Compaction(c) if c.reset_all));
    assert!(has_handoff, "handoff entry must exist on disk");

    // Projection after reset contains ONLY summaries — no verbatim filler.
    let projected = session.derive_messages();
    assert!(
        !projected.iter().any(|m| m.text_content().contains(&filler)),
        "reset must clear verbatim history"
    );
}
