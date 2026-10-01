#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::collections::VecDeque;
use std::sync::{Arc, Mutex};
use std::time::Instant;

use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;

use tempfile::tempdir;

use vak_agent::{Agent, AgentConfig, TaskDeps, TaskTool, TurnOutcome, WorkerRegistry};
use vak_llm::stream;
use vak_llm::types::{AssistantMessage, ChatRequest, ContentBlock, StopReason, Usage};
use vak_llm::{EventStream, LlmError, Provider};
use vak_permission::PermissionEngine;
use vak_session::types::{FrozenContract, SessionHeader};
use vak_session::{SessionLog, SessionPath};
use vak_tools::bash::BashTool;

/// Routes scripted responses by the trailing user-message tag so parallel
/// children are deterministic regardless of request interleaving.
struct TaggedScripted {
    routes: Mutex<HashMap<String, VecDeque<AssistantMessage>>>,
}

use std::collections::HashMap;

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
        eprintln!(
            "[route {key}] t={:?} -> {}",
            Instant::now(),
            match &next {
                Some(m) => format!("{:?}", m.stop_reason),
                None => "EXHAUSTED".into(),
            }
        );
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
        response_id: None,
    }
}

fn bash_script(command: String) -> AssistantMessage {
    AssistantMessage {
        content: vec![ContentBlock::ToolUse {
            id: format!("s{}", uuid_like()),
            name: "bash".into(),
            input: serde_json::json!({ "command": command }),
        }],
        stop_reason: StopReason::ToolUse,
        usage: Usage::default(),
        model: "test-model".into(),
        response_id: None,
    }
}

/// What a worker's shell does. The test proves ordering with marker files, not
/// with a stopwatch: elapsed time is a proxy that fails whenever the machine
/// is busy, and the claim is about who ran alongside whom.
///
/// * `Rendezvous` — announce start, then wait (bounded) for the peer's start
///   marker. It can only succeed if the two workers really ran at the same
///   time; if the scheduler had serialized them the wait times out.
/// * `AfterWave` — record whether both wave-1 workers had already finished when
///   this one began.
///
/// Every script ends in `true`: recording an observation must never make the
/// command itself fail, or the stop gate would add a turn and the test would
/// be measuring that instead.
enum Worker<'a> {
    Rendezvous { me: &'a str, peer: &'a str },
    AfterWave { me: &'a str },
}

fn worker_script(dir: &std::path::Path, worker: Worker<'_>) -> String {
    let d = dir.display();
    let body = match worker {
        Worker::Rendezvous { me, peer } => format!(
            "touch {d}/{me}.start; \
             for i in $(seq 1 400); do [ -f {d}/{peer}.start ] && break; sleep 0.05; done; \
             [ -f {d}/{peer}.start ] && touch {d}/{me}.saw_peer; \
             sleep 0.3; touch {d}/{me}.end; true"
        ),
        Worker::AfterWave { me } => format!(
            "touch {d}/{me}.start; \
             [ -f {d}/a.end ] && [ -f {d}/b.end ] && touch {d}/{me}.saw_wave_done; true"
        ),
    };
    format!("sh -c '{body}'")
}

fn task_call(id: &str, paths: &[&str], tag: &str) -> AssistantMessage {
    AssistantMessage {
        content: vec![ContentBlock::ToolUse {
            id: id.into(),
            name: "task".into(),
            input: serde_json::json!({
                "prompt": format!("do the thing for {tag}"),
                "paths": paths,
            }),
        }],
        stop_reason: StopReason::ToolUse,
        usage: Usage::default(),
        model: "test-model".into(),
        response_id: None,
    }
}

fn multi_task_msg(calls: Vec<AssistantMessage>) -> AssistantMessage {
    let merged = calls
        .into_iter()
        .filter_map(|m| m.content.into_iter().next())
        .collect();
    AssistantMessage {
        content: merged,
        stop_reason: StopReason::ToolUse,
        usage: Usage::default(),
        model: "test-model".into(),
        response_id: None,
    }
}

fn uuid_like() -> String {
    use std::sync::atomic::{AtomicU32, Ordering};
    static C: AtomicU32 = AtomicU32::new(0);
    format!("u{}", C.fetch_add(1, Ordering::Relaxed))
}

fn child_script(command: String) -> VecDeque<AssistantMessage> {
    VecDeque::from(vec![bash_script(command), text("child done")])
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn disjoint_writers_run_parallel_conflicting_writer_serializes() {
    let dir = tempdir().unwrap();
    let home = dir.path().join("home");
    std::fs::create_dir_all(&home).unwrap();
    let header = SessionHeader {
        agent: None,
        session_id: "fanout-parent".into(),
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
    let log = SessionLog::create(
        SessionPath::new_session_file(&home, dir.path(), "fanout-parent"),
        header,
    )
    .unwrap();

    let mut routes: HashMap<String, VecDeque<AssistantMessage>> = HashMap::new();
    routes.insert(
        "fan out".into(),
        VecDeque::from(vec![
            multi_task_msg(vec![
                task_call("a", &["src/a/**"], "A"),
                task_call("b", &["src/b/**"], "B"),
                task_call("c", &["src/a/**"], "C"),
            ]),
            text("all done"),
        ]),
    );
    let work = dir.path().to_path_buf();
    routes.insert(
        "do the thing for A".into(),
        child_script(worker_script(
            &work,
            Worker::Rendezvous { me: "a", peer: "b" },
        )),
    );
    routes.insert(
        "do the thing for B".into(),
        child_script(worker_script(
            &work,
            Worker::Rendezvous { me: "b", peer: "a" },
        )),
    );
    routes.insert(
        "do the thing for C".into(),
        child_script(worker_script(&work, Worker::AfterWave { me: "c" })),
    );

    let provider = Arc::new(TaggedScripted {
        routes: Mutex::new(routes),
    });

    let mut cfg = AgentConfig::new("sys");
    cfg.model = "test-model".into();
    cfg.tools = vec![Arc::new(TaskTool::new(TaskDeps {
        parent_agent_identity: None,
        role_prompts: Default::default(),
        provider: provider.clone(),
        system_prompt: "child-sys".into(),
        child_prompt: None,
        trust_project: false,
        tail: Default::default(),
        model: "test-model".into(),
        tools: vec![Arc::new(BashTool)],
        capabilities: Vec::new(),
        hooks: None,
        revocation_check: None,
        presentation_rebuild: None,
        mcp_tool_index: None,
        input_normalizer: None,
        read_only_tools: vec![],
        max_turns: 4,
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
        mode: vak_permission::Mode::FullAccess,
        approval_mode: vak_agent::ApprovalMode::Ask,
        approver: Some(Arc::new(vak_agent::AutoApprove)),
        sandbox: None,
        cwd: dir.path().to_path_buf(),
        sessions_home: home.clone(),
        parent_session_id: "fanout-parent".into(),
        contract_id: None,
        work_item_id: None,
        work_item_ids: vec![],
        events: None,
        registry: Some(Arc::new(WorkerRegistry::new())),
    }))];
    cfg.permission = Some(Arc::new(
        PermissionEngine::from_rule_strings(&["+task".to_string(), "+Bash(sh *)".to_string()])
            .unwrap(),
    ));
    cfg.approver = Some(Arc::new(vak_agent::AutoApprove));
    let mut agent = Agent::new(provider, log, cfg);
    std::mem::forget(dir);

    let (ev_tx, mut ev_rx) = mpsc::channel(4096);
    let drainer = tokio::spawn(async move { while ev_rx.recv().await.is_some() {} });

    let outcome = agent
        .run(
            "fan out",
            &Default::default(),
            CancellationToken::new(),
            ev_tx,
        )
        .await;
    drop(drainer);

    assert!(
        matches!(outcome, TurnOutcome::Completed { .. }),
        "got {outcome:?}"
    );

    // Wave 1: A and B have disjoint paths, so they must have run at the same
    // time — each saw the other's start marker while it was still running.
    assert!(
        work.join("a.saw_peer").exists() && work.join("b.saw_peer").exists(),
        "disjoint writers A and B must run in parallel (each should have seen the other start)"
    );
    // Wave 2: C's paths conflict with A's, so it must wait for the wave to
    // finish — both wave-1 workers were already done when C began.
    assert!(
        work.join("c.saw_wave_done").exists(),
        "the conflicting writer C must start only after wave 1 (A and B) has finished"
    );

    let session = agent.session.lock().await;
    // Raw ledger: the closed turn's results are trace lines in the
    // projection now (docs/design/68-context-engine.md §10); source order
    // is a property of what was recorded.
    let msgs = session.message_chain();
    let results: Vec<&str> = msgs
        .iter()
        .flat_map(|(_, m)| m.content.iter())
        .filter_map(|b| match b {
            ContentBlock::ToolResult { tool_use_id, .. } => Some(tool_use_id.as_str()),
            _ => None,
        })
        .collect();
    assert_eq!(
        results,
        vec!["a", "b", "c"],
        "source order preserved across waves"
    );
}
