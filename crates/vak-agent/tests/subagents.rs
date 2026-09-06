#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::collections::VecDeque;
use std::sync::{Arc, Mutex};

use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;

use tempfile::tempdir;

use vak_agent::{Agent, AgentConfig, SubagentRegistry, TaskDeps, TaskTool, TurnOutcome};
use vak_llm::stream;
use vak_llm::types::{AssistantMessage, ChatRequest, ContentBlock, StopReason, Usage};
use vak_llm::{EventStream, LlmError, Provider};
use vak_session::types::{FrozenContract, SessionHeader};
use vak_session::{SessionLog, SessionPath};
use vak_tools::read::ReadTool;

struct Scripted {
    responses: Mutex<VecDeque<AssistantMessage>>,
    requests: Arc<Mutex<Vec<ChatRequest>>>,
}

#[async_trait::async_trait]
impl Provider for Scripted {
    fn name(&self) -> &str {
        "scripted"
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

fn text_msg(t: &str) -> AssistantMessage {
    AssistantMessage {
        content: vec![ContentBlock::text(t)],
        stop_reason: StopReason::EndTurn,
        usage: Usage {
            input_tokens: 1,
            output_tokens: 1,
            ..Default::default()
        },
        model: "test-model".into(),
    }
}

fn task_call(id: &str, prompt: &str) -> AssistantMessage {
    AssistantMessage {
        content: vec![ContentBlock::ToolUse {
            id: id.into(),
            name: "task".into(),
            input: serde_json::json!({"prompt": prompt, "label": "explore"}),
        }],
        stop_reason: StopReason::ToolUse,
        usage: Usage::default(),
        model: "test-model".into(),
    }
}

use vak_permission::PermissionEngine;

#[tokio::test]
async fn subagent_roundtrip_with_shared_scripted_provider() {
    let dir = tempdir().unwrap();
    let home = dir.path().join("home");
    std::fs::create_dir_all(&home).unwrap();
    let parent_id = "parent-x".to_string();

    let header = SessionHeader {
        session_id: parent_id.clone(),
        created_at: chrono::Utc::now(),
        cwd: dir.path().to_path_buf(),
        parent_session_id: None,
        contract_id: None,
        work_item_id: None,
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
    };
    let log = SessionLog::create(
        SessionPath::new_session_file(&home, dir.path(), &parent_id),
        header,
    )
    .unwrap();

    let requests = Arc::new(Mutex::new(Vec::new()));
    let scripted = Arc::new(Scripted {
        responses: Mutex::new(VecDeque::from(vec![
            task_call("t1", "find the answer"),
            text_msg("child final answer"),
            text_msg("parent done"),
        ])),
        requests: requests.clone(),
    });

    let mut cfg = AgentConfig::new("sys");
    cfg.model = "test-model".into();
    cfg.tools = vec![Arc::new(TaskTool::new(TaskDeps {
        role_prompts: Default::default(),
        provider: scripted.clone(),
        system_prompt: "child-sys".into(),
        model: "test-model".into(),
        tools: vec![Arc::new(ReadTool)],
        capabilities: Vec::new(),
        hooks: None,
        revocation_check: None,
        mcp_aliases: None,
        input_normalizer: None,
        read_only_tools: vec![Arc::new(ReadTool)],
        max_turns: 5,
        outcome_objective: None,
        outcome: None,
        subagent_budget: None,
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
        approver: None,
        sandbox: None,
        cwd: dir.path().to_path_buf(),
        sessions_home: home.clone(),
        parent_session_id: parent_id.clone(),
        contract_id: None,
        work_item_id: None,
        work_item_ids: vec![],
        events: None,
        registry: Some(Arc::new(SubagentRegistry::new())),
    }))];
    cfg.permission = Some(Arc::new(
        PermissionEngine::from_rule_strings(&["+task".to_string()]).unwrap(),
    ));
    cfg.approver = Some(Arc::new(vak_agent::AutoApprove));

    let mut agent = Agent::new(scripted, log, cfg);
    let outcome = agent
        .run(
            "delegate please",
            &Default::default(),
            CancellationToken::new(),
            mpsc::channel(64).0,
        )
        .await;

    match outcome {
        TurnOutcome::Completed { response } => {
            assert_eq!(response.text_content(), "parent done");
        }
        other => panic!("expected completed, got {other:?}"),
    }

    let session = agent.session.lock().await;
    let tool_result = session
        .derive_messages()
        .iter()
        .flat_map(|m| m.content.iter())
        .find_map(|b| match b {
            ContentBlock::ToolResult {
                content, is_error, ..
            } => Some((content.clone(), *is_error)),
            _ => None,
        })
        .expect("tool result from task must exist");
    assert!(!tool_result.1);
    assert_eq!(tool_result.0, "child final answer");

    let reqs = requests.lock().unwrap();
    assert!(
        reqs.len() >= 3,
        "parent call + child call(s) + parent continuation expected, got {}",
        reqs.len()
    );
    let child_req = &reqs[1];
    assert_eq!(
        child_req.system.as_deref(),
        Some("child-sys"),
        "child must run with its own narrowed system prompt"
    );
    assert!(
        child_req.tools.iter().all(|t| t.name != "task"),
        "children must not be able to spawn further subagents"
    );

    let mut found_child = false;
    for entry in std::fs::read_dir(SessionPath::sessions_dir(&home, dir.path()))
        .unwrap()
        .flatten()
    {
        let name = entry.file_name().to_string_lossy().into_owned();
        if name.starts_with("child-") {
            found_child = true;
            let content = std::fs::read_to_string(entry.path()).unwrap();
            let first: serde_json::Value =
                serde_json::from_str(content.lines().next().unwrap()).unwrap();
            assert_eq!(
                first["parent_session_id"].as_str(),
                Some(parent_id.as_str()),
                "child session must link to its parent"
            );
        }
    }
    assert!(found_child, "a child session file must exist");
}
