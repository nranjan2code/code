//! A run is one trace (docs/design/73 §4): the loop, each tool call it
//! makes and the worker it delegates to all carry the key minted at
//! admission, and a delegated child is its own run caused by the call.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

mod support;

use std::collections::VecDeque;
use std::sync::{Arc, Mutex};

use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;

use vak_agent::{Agent, AgentConfig, TaskDeps, TaskTool, TurnOutcome, WorkerRegistry};
use vak_llm::stream;
use vak_llm::types::{AssistantMessage, ChatRequest, ContentBlock, StopReason, Usage};
use vak_llm::{EventStream, LlmError, Provider};
use vak_permission::PermissionEngine;
use vak_session::ids::{AgentId, SpaceId, TenantId};
use vak_session::trace::{Cause, TraceKey};
use vak_session::types::{FrozenContract, SessionHeader};
use vak_session::{SessionLog, SessionPath};
use vak_tools::{Tool, ToolContext, ToolOutput};

struct Scripted {
    capacity_key: String,
    responses: Mutex<VecDeque<AssistantMessage>>,
}

#[async_trait::async_trait]
impl Provider for Scripted {
    fn name(&self) -> &str {
        "scripted"
    }

    fn rate_limit_key(&self) -> String {
        self.capacity_key.clone()
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

fn text(t: &str) -> AssistantMessage {
    message(vec![ContentBlock::text(t)], StopReason::EndTurn)
}

/// Records the key each call of it ran under.
struct Probe(Arc<Mutex<Vec<Option<TraceKey>>>>);

#[async_trait::async_trait]
impl Tool for Probe {
    fn name(&self) -> &str {
        "probe"
    }

    fn description(&self) -> &str {
        "records its trace"
    }

    fn schema(&self) -> serde_json::Value {
        serde_json::json!({"type": "object", "properties": {}})
    }

    async fn execute(&self, _args: &serde_json::Value, ctx: &ToolContext) -> ToolOutput {
        self.0.lock().unwrap().push(ctx.trace.clone());
        ToolOutput::ok("probed")
    }
}

fn header(id: &str, cwd: &std::path::Path) -> SessionHeader {
    SessionHeader {
        space: None,
        run: None,
        cause: None,
        agent: None,
        session_id: id.into(),
        created_at: chrono::Utc::now(),
        cwd: cwd.to_path_buf(),
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
            permission_mode: "workspace-write".into(),
            capabilities: Vec::new(),
            prompt_layers: Vec::new(),
        },
    }
}

#[tokio::test]
async fn one_run_one_trace_id() {
    let dir = tempfile::tempdir().unwrap();
    let home = dir.path().join("home");
    std::fs::create_dir_all(&home).unwrap();
    let parent_id = "parent-trace".to_string();
    let log = SessionLog::create(
        SessionPath::new_session_file(&home, dir.path(), &parent_id),
        header(&parent_id, dir.path()),
    )
    .unwrap();

    let seen = Arc::new(Mutex::new(Vec::new()));
    let provider = Arc::new(Scripted {
        capacity_key: crate::support::capacity_key(),
        responses: Mutex::new(VecDeque::from(vec![
            call("p1", "probe", serde_json::json!({})),
            call(
                "t1",
                "task",
                serde_json::json!({"prompt": "look", "label": "explore"}),
            ),
            call("c1", "probe", serde_json::json!({})),
            text("child done"),
            text("parent done"),
        ])),
    });

    let root = TraceKey::root(
        TenantId::new(),
        SpaceId::new(),
        AgentId::new(),
        Cause::User {
            request_id: "req-1".into(),
        },
    );
    let probe: Arc<dyn Tool> = Arc::new(Probe(seen.clone()));

    let mut cfg = AgentConfig::new("sys");
    cfg.model = "test-model".into();
    cfg.trace = Some(root.clone());
    cfg.tools = vec![
        probe.clone(),
        Arc::new(TaskTool::new(TaskDeps {
            parent_agent_identity: None,
            role_prompts: Default::default(),
            provider: provider.clone(),
            system_prompt: "child-sys".into(),
            child_prompt: None,
            trust_project: false,
            tail: Default::default(),
            model: "test-model".into(),
            tools: vec![probe.clone()],
            capabilities: Vec::new(),
            hooks: None,
            revocation_check: None,
            presentation_rebuild: None,
            mcp_tool_index: None,
            input_normalizer: None,
            read_only_tools: vec![probe.clone()],
            max_turns: 5,
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
            permission: Some(Arc::new(
                PermissionEngine::from_rule_strings(&["+probe".to_string()]).unwrap(),
            )),
            mode: vak_permission::Mode::WorkspaceWrite,
            approval_mode: vak_agent::ApprovalMode::Ask,
            approver: Some(Arc::new(vak_agent::AutoApprove)),
            sandbox: None,
            cwd: dir.path().to_path_buf(),
            sessions_home: home.clone(),
            parent_session_id: parent_id.clone(),
            contract_id: None,
            work_item_id: None,
            work_item_ids: vec![],
            events: None,
            registry: Some(Arc::new(WorkerRegistry::new())),
        })),
    ];
    cfg.permission = Some(Arc::new(
        PermissionEngine::from_rule_strings(&["+task".to_string(), "+probe".to_string()]).unwrap(),
    ));
    cfg.approver = Some(Arc::new(vak_agent::AutoApprove));

    let mut agent = Agent::new(provider, log, cfg);
    let outcome = agent
        .run(
            "go",
            &Default::default(),
            CancellationToken::new(),
            mpsc::channel(64).0,
        )
        .await;
    assert!(
        matches!(outcome, TurnOutcome::Completed { .. }),
        "{outcome:?}"
    );

    let seen = seen.lock().unwrap();
    assert_eq!(seen.len(), 2, "the parent's probe and the child's probe");
    let parent_call = seen[0].as_ref().expect("the parent's tool call has a key");
    assert_eq!(
        parent_call.run, root.run,
        "a tool call is in the run's trace"
    );
    assert_eq!(parent_call.parent_span, Some(root.span));
    assert_eq!(parent_call.cause, root.cause);

    let child_call = seen[1].as_ref().expect("the child's tool call has a key");
    assert_ne!(child_call.run, root.run, "a delegated child is its own run");
    assert_eq!(
        child_call.cause,
        Cause::Delegation {
            parent_run: root.run,
            tool_use_id: "t1".into()
        },
        "caused by the parent's task call"
    );
    assert_eq!(child_call.tenant, root.tenant);
    assert_eq!(child_call.space, root.space);

    let mut headers = Vec::new();
    for entry in std::fs::read_dir(SessionPath::sessions_dir(&home, dir.path()))
        .unwrap()
        .flatten()
    {
        if entry.file_name().to_string_lossy().starts_with("child-") {
            let log = SessionLog::open_read_only(entry.path()).unwrap();
            headers.push(log.header().unwrap().clone());
        }
    }
    assert_eq!(headers.len(), 1);
    assert_eq!(headers[0].run, Some(child_call.run));
    assert_eq!(headers[0].cause.as_ref(), Some(&child_call.cause));
}
