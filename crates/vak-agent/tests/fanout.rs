#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::collections::VecDeque;
use std::sync::{Arc, Mutex};
use std::time::Instant;

use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;

use tempfile::tempdir;

use vak_agent::{Agent, AgentConfig, SubagentRegistry, TaskDeps, TaskTool, TurnOutcome};
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
    }
}

fn bash_sleep() -> AssistantMessage {
    AssistantMessage {
        content: vec![ContentBlock::ToolUse {
            id: format!("s{}", uuid_like()),
            name: "bash".into(),
            input: serde_json::json!({"command": "sleep 0.5"}),
        }],
        stop_reason: StopReason::ToolUse,
        usage: Usage::default(),
        model: "test-model".into(),
    }
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
    }
}

fn uuid_like() -> String {
    use std::sync::atomic::{AtomicU32, Ordering};
    static C: AtomicU32 = AtomicU32::new(0);
    format!("u{}", C.fetch_add(1, Ordering::Relaxed))
}

fn child_script() -> VecDeque<AssistantMessage> {
    VecDeque::from(vec![bash_sleep(), text("child done")])
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn disjoint_writers_run_parallel_conflicting_writer_serializes() {
    let dir = tempdir().unwrap();
    let home = dir.path().join("home");
    std::fs::create_dir_all(&home).unwrap();
    let header = SessionHeader {
        session_id: "fanout-parent".into(),
        created_at: chrono::Utc::now(),
        cwd: dir.path().to_path_buf(),
        parent_session_id: None,
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
    routes.insert("do the thing for A".into(), child_script());
    routes.insert("do the thing for B".into(), child_script());
    routes.insert("do the thing for C".into(), child_script());

    let provider = Arc::new(TaggedScripted {
        routes: Mutex::new(routes),
    });

    let mut cfg = AgentConfig::new("sys");
    cfg.model = "test-model".into();
    cfg.tools = vec![Arc::new(TaskTool::new(TaskDeps {
        provider: provider.clone(),
        system_prompt: "child-sys".into(),
        model: "test-model".into(),
        tools: vec![Arc::new(BashTool)],
        capabilities: Vec::new(),
        input_normalizer: None,
        read_only_tools: vec![],
        max_turns: 4,
        permission: Some(Arc::new(PermissionEngine::default())),
        mode: vak_permission::Mode::FullAccess,
        approver: Some(Arc::new(vak_agent::AutoApprove)),
        sandbox: None,
        cwd: dir.path().to_path_buf(),
        sessions_home: home.clone(),
        parent_session_id: "fanout-parent".into(),
        events: None,
        registry: Some(Arc::new(SubagentRegistry::new())),
    }))];
    cfg.permission = Some(Arc::new(
        PermissionEngine::from_rule_strings(&["+task".to_string(), "+Bash(sleep *)".to_string()])
            .unwrap(),
    ));
    cfg.approver = Some(Arc::new(vak_agent::AutoApprove));
    let mut agent = Agent::new(provider, log, cfg);
    std::mem::forget(dir);

    let (ev_tx, mut ev_rx) = mpsc::channel(4096);
    let drainer = tokio::spawn(async move { while ev_rx.recv().await.is_some() {} });

    let start = Instant::now();
    let outcome = agent
        .run(
            "fan out",
            &Default::default(),
            CancellationToken::new(),
            ev_tx,
        )
        .await;
    let elapsed = start.elapsed();
    drop(drainer);

    eprintln!("elapsed {elapsed:?}");
    assert!(
        matches!(outcome, TurnOutcome::Completed { .. }),
        "got {outcome:?}"
    );

    // Wave 1 (A+B concurrent, ~0.5s) then wave 2 (C, ~0.5s) => ~1.0s.
    // Fully serial would be ~1.5s.
    assert!(
        elapsed < std::time::Duration::from_millis(1350),
        "expected wave parallelism (~1.0s), took {elapsed:?}"
    );
    assert!(
        elapsed >= std::time::Duration::from_millis(900),
        "conflicting writer must serialize after its wave, took {elapsed:?}"
    );

    let session = agent.session.lock().await;
    let msgs = session.derive_messages();
    let results: Vec<&str> = msgs
        .iter()
        .flat_map(|m| m.content.iter())
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
