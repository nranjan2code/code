#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::collections::{HashMap, VecDeque};
use std::sync::{Arc, Mutex};

use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;

use vak_agent::AutoApprove;
use vak_flow::{Executor, ExecutorDeps, FlowOutcome, FlowState};
use vak_llm::stream;
use vak_llm::types::{AssistantMessage, ChatRequest, ContentBlock, StopReason, Usage};
use vak_llm::{EventStream, LlmError, Provider};
use vak_permission::{Mode, PermissionEngine};
use vak_tools::bash::BashTool;

/// Serves a fixed sequence per routing key (the first user text).
struct TaggedScripted {
    routes: Mutex<HashMap<String, VecDeque<AssistantMessage>>>,
}

impl TaggedScripted {
    fn route_for(request: &ChatRequest) -> String {
        request
            .messages
            .iter()
            .find(|m| m.role == vak_llm::types::Role::User)
            .map(|m| m.text_content())
            .unwrap_or_default()
    }
}

#[async_trait::async_trait]
impl Provider for TaggedScripted {
    fn name(&self) -> &str {
        "scripted"
    }

    async fn stream(
        &self,
        request: ChatRequest,
        _cancel: CancellationToken,
    ) -> Result<EventStream, LlmError> {
        let key = Self::route_for(&request);
        let next = self
            .routes
            .lock()
            .unwrap()
            .get_mut(&key)
            .and_then(|d| d.pop_front());
        let (mut sink, rx) = stream::channel(64);
        match next {
            Some(m) => {
                sink.push(stream::StreamEvent::Start { partial: m.clone() });
                sink.close_message(m).await;
            }
            None => {
                sink.close_error(LlmError::Parse(format!("exhausted: {key}")))
                    .await
            }
        }
        Ok(rx)
    }
}

fn text(t: &str) -> AssistantMessage {
    AssistantMessage {
        content: vec![ContentBlock::text(t)],
        stop_reason: StopReason::EndTurn,
        usage: Usage::default(),
        model: "test-model".into(),
    }
}

fn make_executor(provider: Arc<TaggedScripted>, state_path: std::path::PathBuf) -> Executor {
    make_executor_with_policy(
        provider,
        state_path,
        Mode::FullAccess,
        Some(Arc::new(AutoApprove)),
        std::env::temp_dir(),
    )
}

fn make_executor_with_policy(
    provider: Arc<TaggedScripted>,
    state_path: std::path::PathBuf,
    mode: Mode,
    approver: Option<Arc<dyn vak_agent::Approver>>,
    cwd: std::path::PathBuf,
) -> Executor {
    make_executor_with_outcome(provider, state_path, mode, approver, cwd, None)
}

fn make_executor_with_outcome(
    provider: Arc<TaggedScripted>,
    state_path: std::path::PathBuf,
    mode: Mode,
    approver: Option<Arc<dyn vak_agent::Approver>>,
    cwd: std::path::PathBuf,
    outcome: Option<vak_intent::OutcomeSpec>,
) -> Executor {
    let dir = tempfile::tempdir().unwrap();
    let home = dir.path().join("home");
    std::fs::create_dir_all(&home).unwrap();
    std::mem::forget(dir);
    Executor::new(ExecutorDeps {
        provider,
        system_prompt: "sys".into(),
        model: "test-model".into(),
        tools: vec![Arc::new(BashTool)],
        read_only_tools: vec![],
        max_turns: 4,
        outcome,
        max_retries: 0,
        retry_base_backoff_ms: 100,
        request_timeout: Some(std::time::Duration::from_secs(600)),
        circuit_breaker: None,
        run_retry_attempts: 0,
        run_retry_base_backoff_ms: 1000,
        dispatch_ceiling: 1,
        spend_gate: None,
        permission: Some(Arc::new(PermissionEngine::default())),
        mode,
        approval_mode: vak_agent::ApprovalMode::Ask,
        approver,
        sandbox: None,
        cwd,
        sessions_home: home,
        parent_session_id: "flow-parent".into(),
        state_path,
        work: None,
    })
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn admitted_outcome_is_persisted_into_flow_state() {
    let workspace = tempfile::tempdir().unwrap();
    let toml = r#"
[flow]
name = "outcome"

[[nodes]]
id = "report"
type = "bash"
command = "echo admitted"
"#;
    let flow = vak_flow::parse_flow(toml).unwrap();
    let expected = vak_intent::OutcomeSpec {
        schema_version: 1,
        revision: 4,
        objective: "preserve the admitted objective".into(),
        assumptions: vec!["the workspace is available".into()],
        requirements: Vec::new(),
        resolver_version: 1,
        evidence_max_age_secs: Some(3600),
        max_turns: Some(2),
    };
    let provider = Arc::new(TaggedScripted {
        routes: Mutex::new(HashMap::new()),
    });
    let state_path = workspace.path().join("state.json");
    let mut state = FlowState {
        run_id: "outcome-run".into(),
        flow_name: "outcome".into(),
        definition_toml: toml.into(),
        started_at: chrono::Utc::now(),
        outcome: None,
        nodes: Default::default(),
    };
    let executor = make_executor_with_outcome(
        provider,
        state_path.clone(),
        Mode::FullAccess,
        Some(Arc::new(AutoApprove)),
        workspace.path().to_path_buf(),
        Some(expected.clone()),
    );

    let _ = drain_run(&executor, &flow, &mut state).await;
    assert_eq!(state.outcome, Some(expected));
    let persisted = std::fs::read_to_string(state_path).unwrap();
    assert!(persisted.contains("preserve the admitted objective"));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn bash_node_obeys_permission_decision() {
    let workspace = tempfile::tempdir().unwrap();
    let toml = r#"
[flow]
name = "permission-gate"

[[nodes]]
id = "blocked"
type = "bash"
command = "echo escaped > should-not-exist.txt"
"#;
    let flow = vak_flow::parse_flow(toml).unwrap();
    let provider = Arc::new(TaggedScripted {
        routes: Mutex::new(HashMap::new()),
    });
    let state_path = workspace.path().join("state.json");
    let mut state = FlowState {
        run_id: "permission-run".into(),
        flow_name: "permission-gate".into(),
        definition_toml: toml.into(),
        started_at: chrono::Utc::now(),
        outcome: None,
        nodes: Default::default(),
    };
    let executor = make_executor_with_policy(
        provider,
        state_path,
        Mode::WorkspaceWrite,
        None,
        workspace.path().to_path_buf(),
    );

    let outcome = drain_run(&executor, &flow, &mut state).await;
    match outcome {
        FlowOutcome::Failed { node, reason, .. } => {
            assert_eq!(node, "blocked");
            assert!(reason.contains("no approver available"), "{reason}");
        }
        other => panic!("expected permission failure, got {other:?}"),
    }
    assert!(!workspace.path().join("should-not-exist.txt").exists());
}

async fn drain_run(
    executor: &Executor,
    flow: &vak_flow::FlowDef,
    state: &mut FlowState,
) -> FlowOutcome {
    let (tx, mut rx) = mpsc::channel(256);
    let drainer = tokio::spawn(async move { while rx.recv().await.is_some() {} });
    let outcome = executor
        .run(flow, state, CancellationToken::new(), tx)
        .await;
    drainer.await.unwrap();
    outcome
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn chain_runs_with_substitution_and_merge() {
    let toml = r#"
[flow]
name = "chain"

[[nodes]]
id = "greet"
type = "bash"
command = "echo hello"

[[nodes]]
id = "shout"
type = "agent"
prompt = "SHOUT {{greet}}"

[[nodes]]
id = "done"
type = "merge"
deps = ["shout"]
"#;
    let flow = vak_flow::parse_flow(toml).unwrap();

    let mut routes = HashMap::new();
    routes.insert(
        "SHOUT [stdout]\nhello\n\n".to_string(),
        VecDeque::from(vec![text("HELLO")]),
    );
    let provider = Arc::new(TaggedScripted {
        routes: Mutex::new(routes),
    });

    let state_path = std::env::temp_dir().join(format!("vak-flow-{}.json", uuid_like()));
    let mut state = FlowState {
        run_id: "r1".into(),
        flow_name: "chain".into(),
        definition_toml: toml.into(),
        started_at: chrono::Utc::now(),
        outcome: None,
        nodes: Default::default(),
    };

    let executor = make_executor(provider, state_path.clone());
    let outcome = drain_run(&executor, &flow, &mut state).await;

    match outcome {
        FlowOutcome::Completed { outputs } => {
            assert!(outputs.get("greet").is_some_and(|o| o.contains("hello")));
            assert_eq!(outputs.get("shout").map(String::as_str), Some("HELLO"));
            let merge = outputs.get("done").expect("merge output");
            assert!(merge.contains("[shout] ok"));
            assert!(merge.contains("HELLO"));
        }
        other => panic!("expected completed, got {other:?}"),
    }
    assert_eq!(
        state.nodes.get("greet").unwrap().status,
        vak_flow::NodeStatus::Completed
    );
    let _ = std::fs::remove_file(&state_path);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn required_failure_fails_flow_and_skips_downstream() {
    let toml = r#"
[flow]
name = "strict"

[[nodes]]
id = "boom"
type = "bash"
command = "exit 3"
required = true

[[nodes]]
id = "after"
type = "agent"
prompt = "never runs"
deps = ["boom"]
"#;
    let flow = vak_flow::parse_flow(toml).unwrap();
    let provider = Arc::new(TaggedScripted {
        routes: Mutex::new(HashMap::new()),
    });
    let state_path = std::env::temp_dir().join(format!("vak-flow-{}.json", uuid_like()));
    let mut state = FlowState {
        run_id: "r2".into(),
        flow_name: "strict".into(),
        definition_toml: toml.into(),
        started_at: chrono::Utc::now(),
        outcome: None,
        nodes: Default::default(),
    };
    let executor = make_executor(provider, state_path.clone());
    let outcome = drain_run(&executor, &flow, &mut state).await;

    match outcome {
        FlowOutcome::Failed { node, reason, .. } => {
            assert_eq!(node, "boom");
            assert!(reason.contains("exit code: 3"));
        }
        other => panic!("expected failed, got {other:?}"),
    }
    assert_eq!(
        state.nodes.get("after").unwrap().status,
        vak_flow::NodeStatus::Skipped
    );
    let _ = std::fs::remove_file(&state_path);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn optional_failure_lets_merge_report_partial_outcome() {
    let toml = r#"
[flow]
name = "lenient"

[[nodes]]
id = "flaky"
type = "bash"
command = "exit 9"
required = false

[[nodes]]
id = "report"
type = "merge"
deps = ["flaky"]
"#;
    let flow = vak_flow::parse_flow(toml).unwrap();
    let provider = Arc::new(TaggedScripted {
        routes: Mutex::new(HashMap::new()),
    });
    let state_path = std::env::temp_dir().join(format!("vak-flow-{}.json", uuid_like()));
    let mut state = FlowState {
        run_id: "r3".into(),
        flow_name: "lenient".into(),
        definition_toml: toml.into(),
        started_at: chrono::Utc::now(),
        outcome: None,
        nodes: Default::default(),
    };
    let executor = make_executor(provider, state_path.clone());
    let outcome = drain_run(&executor, &flow, &mut state).await;

    match outcome {
        FlowOutcome::Completed { outputs } => {
            let report = outputs.get("report").expect("merge ran despite failure");
            assert!(report.contains("[flaky] unavailable"));
        }
        other => panic!("expected completed with partial report, got {other:?}"),
    }
    assert_eq!(
        state.nodes.get("flaky").unwrap().status,
        vak_flow::NodeStatus::Failed
    );
    let _ = std::fs::remove_file(&state_path);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn resume_skips_completed_nodes() {
    let toml = r#"
[flow]
name = "resumable"

[[nodes]]
id = "first"
type = "bash"
command = "echo one"

[[nodes]]
id = "second"
type = "bash"
command = "echo two"
deps = ["first"]
"#;
    let flow = vak_flow::parse_flow(toml).unwrap();
    let provider = Arc::new(TaggedScripted {
        routes: Mutex::new(HashMap::new()),
    });
    let state_path = std::env::temp_dir().join(format!("vak-flow-{}.json", uuid_like()));

    // Pre-seed state as if "first" already completed in an earlier run.
    let mut state = FlowState {
        run_id: "r4".into(),
        flow_name: "resumable".into(),
        definition_toml: toml.into(),
        started_at: chrono::Utc::now(),
        outcome: None,
        nodes: [(
            "first".to_string(),
            vak_flow::NodeResult {
                status: vak_flow::NodeStatus::Completed,
                output: "one\n".into(),
            },
        )]
        .into_iter()
        .collect(),
    };
    std::fs::write(&state_path, serde_json::to_string_pretty(&state).unwrap()).unwrap();

    let executor = make_executor(provider.clone(), state_path.clone());
    let outcome = drain_run(&executor, &flow, &mut state).await;
    assert!(matches!(outcome, FlowOutcome::Completed { .. }));

    // "second" must have executed; "first" must not have re-executed.
    assert_eq!(
        state.nodes.get("second").unwrap().status,
        vak_flow::NodeStatus::Completed
    );
    assert!(state.nodes.get("second").unwrap().output.contains("two"));
    assert_eq!(state.nodes.get("first").unwrap().output.trim(), "one");

    // The persisted ledger reflects both.
    let persisted: FlowState =
        serde_json::from_str(&std::fs::read_to_string(&state_path).unwrap()).unwrap();
    assert_eq!(persisted.nodes.len(), 2);
    let _ = std::fs::remove_file(&state_path);
}

fn uuid_like() -> String {
    use std::sync::atomic::{AtomicU32, Ordering};
    static C: AtomicU32 = AtomicU32::new(0);
    format!("u{}", C.fetch_add(1, Ordering::Relaxed))
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn done_contract_failure_rejects_node() {
    let workspace = tempfile::tempdir().unwrap();
    let toml = r#"
[flow]
name = "contract"

[[nodes]]
id = "build"
type = "bash"
command = "echo built > built.txt"
accept = ["verify: test -f built.txt", "verify: test -f missing-artifact.txt"]

[[nodes]]
id = "after"
type = "merge"
deps = ["build"]
"#;
    let flow = vak_flow::parse_flow(toml).unwrap();
    let provider = Arc::new(TaggedScripted {
        routes: Mutex::new(HashMap::new()),
    });
    let state_path = workspace.path().join("state.json");
    let executor = make_executor_with_policy(
        provider,
        state_path.clone(),
        Mode::FullAccess,
        None,
        workspace.path().to_path_buf(),
    );

    let (tx, mut rx) = mpsc::channel(256);
    tokio::spawn(async move { while rx.recv().await.is_some() {} });
    let mut state = FlowState {
        run_id: "r".into(),
        flow_name: "contract".into(),
        definition_toml: toml.into(),
        started_at: chrono::Utc::now(),
        outcome: None,
        nodes: Default::default(),
    };
    let outcome = executor
        .run(&flow, &mut state, CancellationToken::new(), tx)
        .await;
    match outcome {
        FlowOutcome::Failed { node, reason, .. } => {
            assert_eq!(node, "build");
            assert!(reason.contains("done-contract failed"), "{reason}");
            assert!(reason.contains("missing-artifact.txt"), "{reason}");
        }
        other => panic!("expected contract failure, got {other:?}"),
    }

    // Ledger marks the node Failed despite the command itself succeeding.
    let st: FlowState =
        serde_json::from_str(&std::fs::read_to_string(&state_path).unwrap()).unwrap();
    assert_eq!(
        st.nodes.get("build").map(|r| r.status),
        Some(vak_flow::NodeStatus::Failed)
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn done_contract_pass_keeps_node_green() {
    let workspace = tempfile::tempdir().unwrap();
    let toml = r#"
[flow]
name = "contract-ok"

[[nodes]]
id = "make"
type = "bash"
command = "echo v1 > artifact.txt"
accept = ["verify: grep -q v1 artifact.txt"]
"#;
    let flow = vak_flow::parse_flow(toml).unwrap();
    let provider = Arc::new(TaggedScripted {
        routes: Mutex::new(HashMap::new()),
    });
    let state_path = workspace.path().join("state.json");
    let executor = make_executor_with_policy(
        provider,
        state_path.clone(),
        Mode::FullAccess,
        None,
        workspace.path().to_path_buf(),
    );
    let (tx, mut rx) = mpsc::channel(256);
    tokio::spawn(async move { while rx.recv().await.is_some() {} });
    let mut state = FlowState {
        run_id: "r".into(),
        flow_name: "contract-ok".into(),
        definition_toml: toml.into(),
        started_at: chrono::Utc::now(),
        outcome: None,
        nodes: Default::default(),
    };
    let outcome = executor
        .run(&flow, &mut state, CancellationToken::new(), tx)
        .await;
    assert!(matches!(outcome, FlowOutcome::Completed { .. }));
    let st: FlowState =
        serde_json::from_str(&std::fs::read_to_string(&state_path).unwrap()).unwrap();
    assert_eq!(
        st.nodes.get("make").map(|r| r.status),
        Some(vak_flow::NodeStatus::Completed)
    );
}
