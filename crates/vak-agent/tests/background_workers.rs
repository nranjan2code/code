//! Background workers and the `workers` tool
//! (docs/design/84-worker-questions-and-control.md §5): a parent starts a
//! read-only worker without waiting, looks at it, waits for it and reads its
//! result; a worker is never visible to another session; and a parent that
//! tries to finish with a worker still running is sent back, then the worker
//! is cancelled.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

mod support;

use std::collections::VecDeque;
use std::sync::{Arc, Mutex};

use tempfile::tempdir;
use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;

use vak_agent::{Agent, AgentConfig, TaskDeps, TaskTool, TurnOutcome, WorkerRegistry, WorkersTool};
use vak_llm::stream;
use vak_llm::types::{AssistantMessage, ChatRequest, ContentBlock, StopReason, Usage};
use vak_llm::{EventStream, LlmError, Provider};
use vak_permission::PermissionEngine;
use vak_session::types::{FrozenContract, SessionHeader};
use vak_session::{SessionLog, SessionPath};
use vak_tools::read::ReadTool;
use vak_tools::write::WriteTool;
use vak_tools::{Tool, ToolContext};

/// What a worker's model does.
enum Child {
    /// Answers with these messages, in order.
    Script(VecDeque<AssistantMessage>),
    /// Never answers until cancelled.
    Hangs,
}

/// Parent and worker run concurrently, so one shared script would interleave
/// unpredictably: requests are routed by system prompt instead.
struct Routed {
    capacity_key: String,
    parent: Mutex<VecDeque<AssistantMessage>>,
    child: Mutex<Child>,
    requests: Arc<Mutex<Vec<ChatRequest>>>,
}

#[async_trait::async_trait]
impl Provider for Routed {
    fn name(&self) -> &str {
        "routed"
    }

    fn rate_limit_key(&self) -> String {
        self.capacity_key.clone()
    }

    async fn stream(
        &self,
        request: ChatRequest,
        cancel: CancellationToken,
    ) -> Result<EventStream, LlmError> {
        let is_child = request.system.as_deref() == Some("child-sys");
        self.requests.lock().unwrap().push(request);
        let (mut sink, rx) = stream::channel(64);
        let next = if is_child {
            match &mut *self.child.lock().unwrap() {
                Child::Script(queue) => queue.pop_front(),
                Child::Hangs => {
                    sink.push(stream::StreamEvent::Start {
                        partial: AssistantMessage::empty("m"),
                    });
                    tokio::spawn(async move {
                        let _keep = sink;
                        cancel.cancelled().await;
                    });
                    return Ok(rx);
                }
            }
        } else {
            self.parent.lock().unwrap().pop_front()
        };
        match next {
            Some(m) => {
                sink.push(stream::StreamEvent::Start { partial: m.clone() });
                sink.close_message(m).await;
            }
            None => sink.close_error(LlmError::Parse("exhausted".into())).await,
        }
        Ok(rx)
    }
}

fn message(content: Vec<ContentBlock>, stop: StopReason) -> AssistantMessage {
    AssistantMessage {
        content,
        stop_reason: stop,
        usage: Usage {
            input_tokens: 1,
            output_tokens: 1,
            ..Default::default()
        },
        model: "test-model".into(),
        response_id: None,
    }
}

fn text(t: &str) -> AssistantMessage {
    message(vec![ContentBlock::text(t)], StopReason::EndTurn)
}

fn call(id: &str, name: &str, input: serde_json::Value) -> AssistantMessage {
    message(
        vec![ContentBlock::ToolUse {
            id: id.into(),
            name: name.into(),
            input,
        }],
        StopReason::ToolUse,
    )
}

struct Watched;

#[async_trait::async_trait]
impl vak_agent::Approver for Watched {
    async fn approve(&self, _: &str, _: &str, _: &str, _: Option<&str>) -> bool {
        true
    }
    fn answers_questions(&self) -> bool {
        true
    }
}

struct Harness {
    task: Arc<TaskTool>,
    agent: Agent,
    registry: Arc<WorkerRegistry>,
    requests: Arc<Mutex<Vec<ChatRequest>>>,
    cwd: std::path::PathBuf,
}

fn harness(parent_script: Vec<AssistantMessage>, child: Child) -> Harness {
    let dir = tempdir().unwrap();
    let home = dir.path().join("home");
    std::fs::create_dir_all(&home).unwrap();
    let parent_id = "parent-bg".to_string();
    let header = SessionHeader {
        space: None,
        run: None,
        cause: None,
        agent: None,
        session_id: parent_id.clone(),
        created_at: chrono::Utc::now(),
        cwd: dir.path().to_path_buf(),
        parent_session_id: None,
        contract_id: None,
        work_item_id: None,
        conversation: None,
        contract: FrozenContract {
            app_version: "0".into(),
            provider: "routed".into(),
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
    let log = SessionLog::create(
        SessionPath::new_session_file(&home, dir.path(), &parent_id),
        header,
    )
    .unwrap();
    let requests = Arc::new(Mutex::new(Vec::new()));
    let provider = Arc::new(Routed {
        capacity_key: crate::support::capacity_key(),
        parent: Mutex::new(VecDeque::from(parent_script)),
        child: Mutex::new(child),
        requests: requests.clone(),
    });
    let registry = Arc::new(WorkerRegistry::new());
    let task = TaskTool::new(TaskDeps {
        objects: std::sync::Arc::new(vak_session::objects::MemoryObjects::default()),
        parent_agent_identity: None,
        role_prompts: Default::default(),
        provider: provider.clone(),
        system_prompt: "child-sys".into(),
        child_prompt: None,
        trust_project: false,
        tail: Default::default(),
        model: "test-model".into(),
        tools: vec![Arc::new(ReadTool), Arc::new(WriteTool)],
        tool_definitions: Vec::new(),
        capabilities: Vec::new(),
        hooks: None,
        revocation_check: None,
        presentation_rebuild: None,
        mcp_tool_index: None,
        input_normalizer: None,
        read_only_tools: vec![Arc::new(ReadTool)],
        max_turns: 5,
        capacity: None,
        capacity_key: None,
        max_output: 8192,
        declared_window: 128_000,
        ladder: Vec::new(),
        ladder_provider_names: Vec::new(),
        provider_name: None,
        outcome_objective: None,
        outcome: None,
        max_retries: 0,
        retry_base_backoff_ms: 0,
        request_timeout: None,
        circuit_breaker: None,
        run_retry_attempts: 0,
        run_retry_base_backoff_ms: 0,
        dispatch_ceiling: 1,
        spend_gate: None,
        permission: Some(Arc::new(PermissionEngine::default())),
        mode: vak_permission::Mode::WorkspaceWrite,
        approval_mode: vak_agent::ApprovalMode::Ask,
        approver: Some(Arc::new(Watched)),
        sandbox: None,
        cwd: dir.path().to_path_buf(),
        sessions_home: home.clone(),
        parent_session_id: parent_id.clone(),
        contract_id: None,
        work_item_id: None,
        work_item_ids: vec![],
        events: None,
        registry: Some(registry.clone()),
    });
    let task = Arc::new(task);
    let mut cfg = AgentConfig::new("sys");
    cfg.model = "test-model".into();
    cfg.tools = vec![
        task.clone(),
        Arc::new(WorkersTool::new(registry.clone(), parent_id.clone())),
        Arc::new(WriteTool),
    ];
    cfg.workers = Some(registry.clone());
    cfg.permission = Some(Arc::new(
        PermissionEngine::from_rule_strings(&["+task".to_string(), "+write".to_string()]).unwrap(),
    ));
    cfg.approver = Some(Arc::new(vak_agent::AutoApprove));
    let cwd = dir.path().to_path_buf();
    std::mem::forget(dir);
    Harness {
        task,
        agent: Agent::new(provider, log, cfg),
        registry,
        requests,
        cwd,
    }
}

async fn run(agent: &mut Agent) -> TurnOutcome {
    tokio::time::timeout(
        std::time::Duration::from_secs(30),
        agent.run(
            "go",
            &Default::default(),
            CancellationToken::new(),
            mpsc::channel(256).0,
        ),
    )
    .await
    .expect("the run must not hang")
}

fn tool_results(requests: &[ChatRequest]) -> Vec<String> {
    requests
        .iter()
        .flat_map(|r| r.messages.iter())
        .flat_map(|m| m.content.iter())
        .filter_map(|b| match b {
            ContentBlock::ToolResult { content, .. } => Some(content.clone()),
            _ => None,
        })
        .collect()
}

fn background_task(label: &str) -> AssistantMessage {
    call(
        "t1",
        "task",
        serde_json::json!({"prompt": "research it", "label": label, "readonly": true, "background": true}),
    )
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_parent_starts_lists_waits_for_and_reads_a_background_worker() {
    let mut h = harness(
        vec![
            background_task("scout"),
            call("w1", "workers", serde_json::json!({"action": "list"})),
            call("w2", "workers", serde_json::json!({"action": "wait"})),
            text("parent done"),
        ],
        Child::Script(VecDeque::from(vec![text("the child's findings")])),
    );
    let outcome = run(&mut h.agent).await;
    assert!(
        matches!(outcome, TurnOutcome::Completed { .. }),
        "{outcome:?}"
    );
    let results = tool_results(&h.requests.lock().unwrap());
    assert!(
        results
            .iter()
            .any(|r| r.contains("started in the background") && r.contains("scout")),
        "task returns at once: {results:?}"
    );
    assert!(
        results.iter().any(|r| r.contains("scout")),
        "list names the worker: {results:?}"
    );
    assert!(
        results.iter().any(|r| r.contains("the child's findings")),
        "wait returns the result: {results:?}"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_background_writer_must_name_what_it_may_change() {
    let mut h = harness(
        vec![
            call(
                "t1",
                "task",
                serde_json::json!({"prompt": "edit it", "background": true}),
            ),
            call(
                "t2",
                "task",
                serde_json::json!({"prompt": "edit it", "background": true, "paths": ["."]}),
            ),
            call(
                "t3",
                "task",
                serde_json::json!({"prompt": "edit it", "background": true, "paths": ["../x"]}),
            ),
            text("it failed"),
            text("it failed"),
            text("it failed"),
        ],
        Child::Script(VecDeque::new()),
    );
    let _ = run(&mut h.agent).await;
    let results = tool_results(&h.requests.lock().unwrap());
    for expected in [
        "needs paths",
        "not the whole workspace",
        "not a path inside the workspace",
    ] {
        assert!(
            results.iter().any(|r| r.contains(expected)),
            "{expected}: {results:?}"
        );
    }
    assert_eq!(h.registry.background_live("parent-bg"), 0);
}

fn writer_task(label: &str, paths: serde_json::Value) -> AssistantMessage {
    call(
        "t1",
        "task",
        serde_json::json!({"prompt": "write it", "label": label, "background": true, "paths": paths}),
    )
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_background_writer_changes_only_what_it_leased() {
    let mut h = harness(
        vec![
            writer_task("scribe", serde_json::json!(["out"])),
            call("w1", "workers", serde_json::json!({"action": "wait"})),
            text("parent done"),
        ],
        Child::Script(VecDeque::from(vec![
            call(
                "c1",
                "write",
                serde_json::json!({"path": "out/a.txt", "content": "A"}),
            ),
            call(
                "c2",
                "write",
                serde_json::json!({"path": "other.txt", "content": "B"}),
            ),
            text("wrote it"),
        ])),
    );
    let outcome = run(&mut h.agent).await;
    assert!(
        matches!(outcome, TurnOutcome::Completed { .. }),
        "{outcome:?}"
    );
    assert_eq!(
        std::fs::read_to_string(h.cwd.join("out/a.txt")).unwrap(),
        "A",
        "a write inside the lease lands"
    );
    assert!(
        !h.cwd.join("other.txt").exists(),
        "a write outside the lease never happens"
    );
    let results = tool_results(&h.requests.lock().unwrap());
    assert!(
        results
            .iter()
            .any(|r| r.contains("outside this worker's write lease")),
        "the worker is told why: {results:?}"
    );
    assert_eq!(
        h.registry.background_live("parent-bg"),
        0,
        "the lease ends with the worker"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn the_parent_is_held_off_leased_paths_until_the_worker_ends() {
    let mut h = harness(
        vec![
            writer_task("scribe", serde_json::json!(["out"])),
            call(
                "p1",
                "write",
                serde_json::json!({"path": "out/mine.txt", "content": "P"}),
            ),
            call(
                "p2",
                "write",
                serde_json::json!({"path": "free.txt", "content": "F"}),
            ),
            call(
                "p3",
                "task",
                serde_json::json!({"prompt": "second", "background": true, "paths": ["out/sub"]}),
            ),
            call("w1", "workers", serde_json::json!({"action": "list"})),
            text("done"),
            text("done"),
            text("done"),
        ],
        Child::Hangs,
    );
    let _ = run(&mut h.agent).await;
    let results = tool_results(&h.requests.lock().unwrap());
    assert!(
        results.iter().any(|r| r.contains("holds a write lease")),
        "the parent's write to a leased path is refused: {results:?}"
    );
    assert!(!h.cwd.join("out/mine.txt").exists());
    assert_eq!(
        std::fs::read_to_string(h.cwd.join("free.txt")).unwrap(),
        "F",
        "a path nobody holds is still the parent's to write"
    );
    assert!(
        results
            .iter()
            .filter(|r| r.contains("holds a write lease"))
            .count()
            >= 2,
        "a second writer cannot take an overlapping lease either: {results:?}"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn another_sessions_worker_is_unknown_not_forbidden() {
    let mut h = harness(
        vec![
            background_task("scout"),
            call("w2", "workers", serde_json::json!({"action": "wait"})),
            text("done"),
        ],
        Child::Script(VecDeque::from(vec![text("findings")])),
    );
    let _ = run(&mut h.agent).await;
    let worker = h
        .registry
        .finished_for("parent-bg")
        .into_iter()
        .next()
        .expect("the worker finished and was kept for its parent");
    let stranger = WorkersTool::new(h.registry.clone(), "someone-else".into());
    let ctx = ToolContext::new(std::env::temp_dir());
    for action in ["status", "result", "stop", "message"] {
        let out = stranger
            .execute(
                &serde_json::json!({"action": action, "id": worker.id, "text": "hi"}),
                &ctx,
            )
            .await;
        assert!(out.is_error, "{action}: {}", out.content);
        assert_eq!(
            out.content,
            format!("unknown worker '{}'", worker.id),
            "{action} must not reveal that the worker exists"
        );
    }
    let list = stranger
        .execute(&serde_json::json!({"action": "list"}), &ctx)
        .await;
    assert_eq!(list.content, "No workers.");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn finishing_with_a_running_worker_is_blocked_twice_then_the_worker_is_cancelled() {
    let mut h = harness(
        vec![
            background_task("scout"),
            text("I am done"),
            text("still done"),
            text("final answer"),
        ],
        Child::Hangs,
    );
    let outcome = run(&mut h.agent).await;
    assert!(
        matches!(&outcome, TurnOutcome::Completed { response } if response.text_content() == "final answer"),
        "{outcome:?}"
    );
    let requests = h.requests.lock().unwrap().clone();
    let nudges = requests
        .iter()
        .flat_map(|r| r.messages.iter())
        .map(|m| m.text_content())
        .filter(|t| t.contains("workers you started are still running"))
        .count();
    assert!(nudges >= 1, "the parent was sent back at least once");
    for _ in 0..200 {
        if h.registry.active_for("parent-bg").is_empty() {
            let session = h.agent.session.lock().await;
            let cancelled = session.chain_to_root().iter().any(|entry| {
                matches!(
                    &entry.payload,
                    vak_session::EntryPayload::Activity(a)
                        if a.label.contains("Background workers cancelled")
                )
            });
            assert!(cancelled, "the cancellation is recorded");
            return;
        }
        tokio::time::sleep(std::time::Duration::from_millis(25)).await;
    }
    panic!("the worker was not cancelled when its parent's turn ended");
}

fn workers_tool(h: &Harness) -> WorkersTool {
    WorkersTool::new(h.registry.clone(), "parent-bg".into())
}

async fn act(tool: &WorkersTool, args: serde_json::Value) -> vak_tools::ToolOutput {
    tool.execute(&args, &ToolContext::new(std::env::temp_dir()))
        .await
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_parent_answers_its_workers_question_and_the_worker_uses_it() {
    let h = harness(
        Vec::new(),
        Child::Script(VecDeque::from(vec![
            call(
                "a1",
                "ask_parent",
                serde_json::json!({"question": "Which fiscal year?", "options": ["2025", "2026"]}),
            ),
            text("used the answer"),
        ])),
    );
    let ctx = ToolContext::new(std::env::temp_dir());
    let started = h
        .task
        .execute(
            &serde_json::json!({"prompt": "total it", "label": "scout", "readonly": true, "background": true}),
            &ctx,
        )
        .await;
    assert!(!started.is_error, "{}", started.content);
    let tool = workers_tool(&h);

    // The worker asks, so a wait stops early and says what it is waiting for.
    let waited = act(
        &tool,
        serde_json::json!({"action": "wait", "timeout_secs": 30}),
    )
    .await;
    assert!(
        waited.content.contains("WAITING FOR AN ANSWER")
            && waited.content.contains("Which fiscal year?")
            && waited.content.contains("2025 | 2026"),
        "{}",
        waited.content
    );
    let id = h.registry.active_for("parent-bg")[0].id.clone();

    let replied = act(
        &tool,
        serde_json::json!({"action": "reply", "id": id, "text": "fiscal 2026"}),
    )
    .await;
    assert!(!replied.is_error, "{}", replied.content);
    // A second reply has nothing to answer.
    let again = act(
        &tool,
        serde_json::json!({"action": "reply", "id": id, "text": "again"}),
    )
    .await;
    assert!(again.is_error);

    let finished = act(
        &tool,
        serde_json::json!({"action": "wait", "timeout_secs": 30}),
    )
    .await;
    assert!(
        finished.content.contains("used the answer"),
        "{}",
        finished.content
    );
    let seen: String = tool_results(&h.requests.lock().unwrap()).join("\n");
    assert!(
        seen.contains("Answer from the agent that started you: fiscal 2026"),
        "{seen}"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_message_reaches_a_live_worker_and_stop_cancels_it() {
    let h = harness(Vec::new(), Child::Hangs);
    let ctx = ToolContext::new(std::env::temp_dir());
    let started = h
        .task
        .execute(
            &serde_json::json!({"prompt": "wait around", "label": "scout", "readonly": true, "background": true}),
            &ctx,
        )
        .await;
    assert!(!started.is_error, "{}", started.content);
    let tool = workers_tool(&h);
    let id = h.registry.active_for("parent-bg")[0].id.clone();

    let status = act(&tool, serde_json::json!({"action": "status", "id": id})).await;
    assert!(status.content.contains("running") && status.content.contains("scout"));
    let empty = act(
        &tool,
        serde_json::json!({"action": "message", "id": id, "text": "  "}),
    )
    .await;
    assert!(empty.is_error);
    let sent = act(
        &tool,
        serde_json::json!({"action": "message", "id": id, "text": "focus on 2026"}),
    )
    .await;
    assert!(!sent.is_error, "{}", sent.content);

    let stopped = act(&tool, serde_json::json!({"action": "stop", "id": id})).await;
    assert!(!stopped.is_error, "{}", stopped.content);
    for _ in 0..200 {
        if h.registry.active_for("parent-bg").is_empty() {
            let result = act(&tool, serde_json::json!({"action": "result", "id": id})).await;
            assert!(
                result.is_error,
                "a stopped worker's result is an error: {}",
                result.content
            );
            assert!(result.content.contains("cancelled"), "{}", result.content);
            return;
        }
        tokio::time::sleep(std::time::Duration::from_millis(25)).await;
    }
    panic!("the worker did not stop");
}
