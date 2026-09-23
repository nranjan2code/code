#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

//! A worker's cards reach the conversation that delegated to it, and an
//! over-long tool result is recorded whole behind the window its request
//! carries (docs/design/68-context-engine.md §3, §10).

use std::collections::VecDeque;
use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;

use tempfile::tempdir;

use vak_agent::{Agent, AgentConfig, TaskDeps, TaskTool, TurnOutcome};
use vak_llm::stream;
use vak_llm::types::{AssistantMessage, ChatRequest, ContentBlock, StopReason, Usage};
use vak_llm::{EventStream, LlmError, Provider};
use vak_permission::{Mode, PermissionEngine};
use vak_session::types::{EntryPayload, FrozenContract, PresentationSource, SessionHeader};
use vak_session::{SessionLog, SessionPath};
use vak_tools::context::ToolContext;
use vak_tools::{PresentationCard, Tool, ToolOutput};

struct Scripted {
    responses: Mutex<VecDeque<AssistantMessage>>,
    requests: Mutex<Vec<ChatRequest>>,
}

#[async_trait]
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
            None => sink.close_error(LlmError::Parse("exhausted".into())).await,
        }
        Ok(rx)
    }
}

fn scripted(responses: Vec<AssistantMessage>) -> Arc<Scripted> {
    Arc::new(Scripted {
        responses: Mutex::new(VecDeque::from(responses)),
        requests: Mutex::new(Vec::new()),
    })
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
        response_id: None,
    }
}

fn call(id: &str, name: &str, input: serde_json::Value) -> AssistantMessage {
    AssistantMessage {
        content: vec![ContentBlock::ToolUse {
            id: id.into(),
            name: name.into(),
            input,
        }],
        stop_reason: StopReason::ToolUse,
        usage: Usage::default(),
        model: "test-model".into(),
        response_id: None,
    }
}

struct FakeEmitChartCard;

#[async_trait]
impl Tool for FakeEmitChartCard {
    fn name(&self) -> &str {
        "emit_chart_card"
    }
    fn description(&self) -> &str {
        "test stand-in"
    }
    fn schema(&self) -> serde_json::Value {
        serde_json::json!({"type": "object"})
    }
    fn presents_cards(&self) -> bool {
        true
    }
    async fn execute(&self, _args: &serde_json::Value, _ctx: &ToolContext) -> ToolOutput {
        ToolOutput::ok("shown")
    }
}

/// Returns 5,000 numbered lines — far past what one request carries whole.
struct BigOutput;

#[async_trait]
impl Tool for BigOutput {
    fn name(&self) -> &str {
        "big_output"
    }
    fn description(&self) -> &str {
        "test stand-in"
    }
    fn schema(&self) -> serde_json::Value {
        serde_json::json!({"type": "object"})
    }
    async fn execute(&self, _args: &serde_json::Value, _ctx: &ToolContext) -> ToolOutput {
        ToolOutput::ok(big_output())
    }
}

fn big_output() -> String {
    (1..=5000)
        .map(|n| format!("record {n:05}"))
        .collect::<Vec<_>>()
        .join("\n")
}

fn fake_rebuild() -> vak_agent::PresentationRebuild {
    Arc::new(|_name, input| {
        let payload = vak_session::types::canonicalize_json(input.get("payload")?);
        Some(PresentationCard {
            semantic_type: "chart".into(),
            skill_id: "test".into(),
            skill_version: "1".into(),
            schema_version: 1,
            title: payload.get("title")?.as_str()?.to_string(),
            payload,
            identity_digest: "digest".into(),
        })
    })
}

fn header(dir: &tempfile::TempDir, session_id: &str) -> SessionHeader {
    SessionHeader {
        agent: None,
        session_id: session_id.into(),
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
    }
}

fn agent(
    dir: &tempfile::TempDir,
    session_id: &str,
    provider: Arc<Scripted>,
    tools: Vec<Arc<dyn Tool>>,
) -> Agent {
    let home = dir.path().join("home");
    std::fs::create_dir_all(&home).unwrap();
    let log = SessionLog::create(
        SessionPath::new_session_file(&home, dir.path(), session_id),
        header(dir, session_id),
    )
    .unwrap();
    let mut cfg = AgentConfig::new("sys");
    cfg.model = "test-model".into();
    cfg.mode = Mode::FullAccess;
    cfg.permission = Some(Arc::new(PermissionEngine::default()));
    cfg.approver = Some(Arc::new(vak_agent::AutoApprove));
    cfg.tools = tools;
    Agent::new(provider, log, cfg)
}

fn task_tool(dir: &tempfile::TempDir, provider: Arc<Scripted>, parent: &str) -> Arc<dyn Tool> {
    Arc::new(TaskTool::new(TaskDeps {
        parent_agent_identity: None,
        role_prompts: Default::default(),
        provider,
        system_prompt: "child-sys".into(),
        tail: Default::default(),
        model: "test-model".into(),
        tools: vec![Arc::new(FakeEmitChartCard)],
        capabilities: Vec::new(),
        hooks: None,
        revocation_check: None,
        presentation_rebuild: Some(fake_rebuild()),
        mcp_tool_index: None,
        input_normalizer: None,
        read_only_tools: Vec::new(),
        max_turns: 5,
        outcome_objective: None,
        outcome: None,
        worker_budget: None,
        max_retries: 0,
        retry_base_backoff_ms: 0,
        request_timeout: None,
        circuit_breaker: None,
        run_retry_attempts: 0,
        run_retry_base_backoff_ms: 0,
        dispatch_ceiling: 1,
        spend_gate: None,
        permission: Some(Arc::new(PermissionEngine::default())),
        mode: Mode::FullAccess,
        approval_mode: vak_agent::ApprovalMode::Ask,
        approver: Some(Arc::new(vak_agent::AutoApprove)),
        sandbox: None,
        cwd: dir.path().to_path_buf(),
        sessions_home: dir.path().join("home"),
        parent_session_id: parent.into(),
        contract_id: None,
        work_item_id: None,
        work_item_ids: vec![],
        events: None,
        registry: None,
    }))
}

async fn run(agent: &mut Agent, prompt: &str) -> TurnOutcome {
    agent
        .run(
            prompt,
            &Default::default(),
            CancellationToken::new(),
            mpsc::channel(256).0,
        )
        .await
}

fn tool_result(session: &SessionLog, id: &str) -> String {
    session
        .message_chain()
        .iter()
        .flat_map(|(_, m)| m.content.iter())
        .find_map(|block| match block {
            ContentBlock::ToolResult {
                tool_use_id,
                content,
                ..
            } if tool_use_id == id => Some(content.clone()),
            _ => None,
        })
        .expect("tool result recorded")
}

#[tokio::test]
async fn a_workers_card_is_shown_and_recallable_in_the_delegating_conversation() {
    let dir = tempdir().unwrap();
    let provider = scripted(vec![
        call(
            "t1",
            "task",
            serde_json::json!({"prompt": "chart the sales"}),
        ),
        call(
            "c1",
            "emit_chart_card",
            serde_json::json!({"payload": {"title": "Sales by month", "series": [1, 2, 3]}}),
        ),
        text_msg("charted"),
        text_msg("the worker charted it"),
    ]);
    let task = task_tool(&dir, provider.clone(), "parent-cards");
    let mut parent = agent(&dir, "parent-cards", provider.clone(), vec![task]);
    let outcome = run(&mut parent, "chart the sales by month").await;
    assert!(
        matches!(outcome, TurnOutcome::Completed { .. }),
        "{outcome:?}"
    );

    let session = parent.session.lock().await;
    let delegated: Vec<(String, _)> = session
        .presentations()
        .into_iter()
        .filter(|(_, record)| {
            matches!(&record.source, PresentationSource::Delegated { tool_use_id, .. } if tool_use_id == "t1")
        })
        .collect();
    assert_eq!(
        delegated.len(),
        1,
        "the worker's card is in the parent ledger"
    );
    let (presentation_id, record) = &delegated[0];
    assert_eq!(record.title, "Sales by month");
    assert!(record.derived_from.contains(&"t1".to_string()));

    let result = tool_result(&session, "t1");
    assert!(result.starts_with("charted"), "{result}");
    assert!(
        result.contains(&format!(
            "- pres:{presentation_id} chart \"Sales by month\""
        )),
        "the task result lists the card by id: {result}"
    );
    let payload = record.payload.clone();
    let presentation_id = presentation_id.clone();
    drop(session);

    // The delegating agent opens the worker's card to review it.
    provider.responses.lock().unwrap().extend([
        call(
            "r1",
            "recall",
            serde_json::json!({"presentation": presentation_id}),
        ),
        text_msg("reviewed"),
    ]);
    parent.config.tools.push(Arc::new(vak_tools::RecallTool));
    let outcome = run(&mut parent, "check the chart the worker made").await;
    assert!(
        matches!(outcome, TurnOutcome::Completed { .. }),
        "{outcome:?}"
    );
    let session = parent.session.lock().await;
    assert_eq!(tool_result(&session, "r1"), payload.to_string());
}

#[tokio::test]
async fn an_over_long_result_is_windowed_in_the_request_and_whole_in_the_ledger() {
    let dir = tempdir().unwrap();
    let provider = scripted(vec![
        call("b1", "big_output", serde_json::json!({})),
        call(
            "r1",
            "recall",
            serde_json::json!({"id": "b1", "range": {"start": 3000, "end": 3002}}),
        ),
        text_msg("found it"),
    ]);
    let mut agent = agent(
        &dir,
        "windowed",
        provider.clone(),
        vec![Arc::new(BigOutput), Arc::new(vak_tools::RecallTool)],
    );
    let outcome = run(&mut agent, "read the big output").await;
    assert!(
        matches!(outcome, TurnOutcome::Completed { .. }),
        "{outcome:?}"
    );

    let session = agent.session.lock().await;
    let carried = tool_result(&session, "b1");
    assert!(carried.chars().count() <= vak_tools::RESULT_WINDOW_CHARS + 200);
    assert!(carried.starts_with("record 00001\n") && carried.ends_with("record 05000"));
    assert!(
        carried.contains("omitted") && carried.contains("recall {\"id\": \"b1\""),
        "the window says what it left out and how to read it"
    );
    assert!(!carried.contains("record 03000"));

    let body = session
        .chain_to_root()
        .into_iter()
        .find_map(|entry| match &entry.payload {
            EntryPayload::EvidenceBody(body) if body.tool_use_id == "b1" => {
                Some(body.content.clone())
            }
            _ => None,
        })
        .expect("the whole result is in the ledger");
    assert_eq!(body, big_output());
    assert_eq!(
        session.evidence("b1").expect("evidence").content,
        big_output()
    );

    assert_eq!(
        tool_result(&session, "r1"),
        "record 03000\nrecord 03001\nrecord 03002",
        "recall returns the omitted lines exactly"
    );

    let last_request = provider.requests.lock().unwrap().last().cloned().unwrap();
    let carried_in_request = last_request
        .messages
        .iter()
        .flat_map(|m| m.content.iter())
        .find_map(|block| match block {
            ContentBlock::ToolResult {
                tool_use_id,
                content,
                ..
            } if tool_use_id == "b1" => Some(content.clone()),
            _ => None,
        })
        .expect("the result is in the next request");
    assert_eq!(
        carried_in_request, carried,
        "the request carries what the ledger logged"
    );
}
