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
    capacity_key: String,
    responses: Mutex<VecDeque<AssistantMessage>>,
    requests: Mutex<Vec<ChatRequest>>,
}

#[async_trait]
impl Provider for Scripted {
    fn name(&self) -> &str {
        "scripted"
    }

    fn rate_limit_key(&self) -> String {
        self.capacity_key.clone()
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
        capacity_key: crate::support::capacity_key(),
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
        applied: None,
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
        applied: None,
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
    fn card_kinds(&self) -> Vec<String> {
        vec!["chart".into()]
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
        let mut card = input.as_object()?.clone();
        let semantic_type = card.remove("semantic_type")?.as_str()?.to_string();
        let payload = vak_session::types::canonicalize_json(&serde_json::Value::Object(card));
        Some(PresentationCard {
            presentation_id: None,
            block: Some(semantic_type.clone()),
            data_call_id: None,
            semantic_type,
            skill_id: "test".into(),
            skill_version: "1".into(),
            schema_version: 1,
            title: payload.get("title")?.as_str()?.to_string(),
            payload,
            identity_digest: "digest".into(),
        })
    })
}

struct FakeFormCard(&'static str);

#[async_trait]
impl Tool for FakeFormCard {
    fn name(&self) -> &str {
        match self.0 {
            "controls" => "emit_controls_card",
            "stats" => "emit_stats_card",
            _ => "emit_table_card",
        }
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
    fn card_kinds(&self) -> Vec<String> {
        vec![self.0.into()]
    }
    fn followable_fields(&self) -> Vec<Vec<String>> {
        match self.0 {
            "stats" => vec![vec!["readings".into(), "*".into(), "value".into()]],
            "table" => vec![vec!["rows".into(), "*".into(), "*".into()]],
            _ => Vec::new(),
        }
    }
    async fn execute(&self, _args: &serde_json::Value, _ctx: &ToolContext) -> ToolOutput {
        ToolOutput::ok("shown")
    }
}

fn header(dir: &tempfile::TempDir, session_id: &str) -> SessionHeader {
    SessionHeader {
        space: None,
        run: None,
        cause: None,
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
    .unwrap()
    .with_objects(Arc::new(vak_session::objects::MemoryObjects::default()));
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
        runs: None,
        objects: std::sync::Arc::new(vak_session::objects::MemoryObjects::default()),
        parent_agent_identity: None,
        role_prompts: Default::default(),
        provider,
        system_prompt: "child-sys".into(),
        child_prompt: None,
        trust_project: false,
        tail: Default::default(),
        model: "test-model".into(),
        tools: vec![
            Arc::new(FakeEmitChartCard),
            Arc::new(FakeFormCard("controls")),
            Arc::new(FakeFormCard("stats")),
            Arc::new(FakeFormCard("table")),
        ],
        tool_definitions: Vec::new(),
        capabilities: Vec::new(),
        hooks: None,
        revocation_check: None,
        presentation_rebuild: Some(fake_rebuild()),
        child_services: None,
        mcp_tool_index: None,
        input_normalizer: None,
        read_only_tools: Vec::new(),
        max_turns: 5,
        capacity: None,
        capacity_key: None,
        max_output: 8192,
        declared_window: Some(128_000),
        ladder: Vec::new(),
        ladder_provider_names: Vec::new(),
        provider_name: None,
        outcome_objective: None,
        outcome: None,
        max_retries: 0,
        retry_base_backoff_ms: 0,
        request_timeout: None,
        control_limits: vak_agent::ControlLimits::shipped(),
        circuit_breaker: None,
        run_retry_attempts: 0,
        run_retry_base_backoff_ms: 0,
        dispatch_ceiling: 1,
        steps: vak_control::StepDefaults::shipped(),
        protected_paths: Vec::new(),
        stop_policy: Some(vak_agent::StopPolicy::default()),
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

fn tool_result_is_error(session: &SessionLog, id: &str) -> bool {
    session
        .message_chain()
        .iter()
        .flat_map(|(_, message)| message.content.iter())
        .find_map(|block| match block {
            ContentBlock::ToolResult {
                tool_use_id,
                is_error,
                ..
            } if tool_use_id == id => Some(*is_error),
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
            serde_json::json!({"semantic_type": "chart", "title": "Sales by month", "points": [{"series": "s", "x": "Jan", "y": 1}]}),
        ),
        text_msg("charted"),
        text_msg("The worker charted the sales by month."),
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
            serde_json::json!({"open": format!("pres:{presentation_id}")}),
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
    let reopened: serde_json::Value = serde_json::from_str(&tool_result(&session, "r1")).unwrap();
    assert_eq!(reopened["card"], payload, "the card, as it was recorded");
    assert!(
        reopened["from_turn"]
            .as_str()
            .is_some_and(|line| line.contains("asked:")),
        "with the record of the turn that showed it: {reopened}"
    );
}

#[tokio::test]
async fn a_workers_control_bindings_are_rebased_and_duplicate_imports_reuse_parent_ids() {
    let dir = tempdir().unwrap();
    let controls = serde_json::json!({
        "semantic_type": "controls", "title": "Inputs",
        "inputs": [{"label": "Base", "name": "base", "kind": "number", "value": 4}],
        "outputs": []
    });
    let stats = serde_json::json!({
        "semantic_type": "stats", "title": "Totals",
        "readings": [{"label": "Doubled", "value": "= base"}]
    });
    let mut responses = vec![
        call(
            "t1",
            "task",
            serde_json::json!({"prompt": "make the cards"}),
        ),
        call("c1", "emit_controls_card", controls.clone()),
        call("s1", "emit_stats_card", stats.clone()),
        text_msg("finished first child"),
        call(
            "t2",
            "task",
            serde_json::json!({"prompt": "make the cards again"}),
        ),
        call("c1", "emit_controls_card", controls),
        call("s1", "emit_stats_card", stats),
        text_msg("finished second child"),
        text_msg("Both workers finished."),
    ];
    let provider = scripted(std::mem::take(&mut responses));
    let task = task_tool(&dir, provider.clone(), "parent-bindings");
    let mut parent = agent(&dir, "parent-bindings", provider, vec![task]);
    let outcome = run(&mut parent, "make the cards twice").await;
    assert!(
        matches!(outcome, TurnOutcome::Completed { .. }),
        "{outcome:?}"
    );

    let session = parent.session.lock().await;
    let delegated: Vec<_> = session
        .presentations()
        .into_iter()
        .filter(|(_, record)| matches!(record.source, PresentationSource::Delegated { .. }))
        .collect();
    assert_eq!(delegated.len(), 2, "the repeated cards are deduplicated");
    let control = delegated
        .iter()
        .find(|(_, record)| record.block.as_deref() == Some("controls"))
        .unwrap();
    let stats = delegated
        .iter()
        .find(|(_, record)| record.block.as_deref() == Some("stats"))
        .unwrap();
    assert_eq!(stats.1.payload["readings"][0]["value"], 4);
    assert_eq!(
        stats.1.payload["bound"][0]["from"],
        serde_json::json!([control.0])
    );
    assert_eq!(
        stats.1.identity_digest, "digest",
        "identity excludes rebased bindings"
    );
    assert!(stats.1.payload["bound"][0]["from"][0] != "c1");

    let first_result = tool_result(&session, "t1");
    let second_result = tool_result(&session, "t2");
    assert!(first_result.contains(&format!("pres:{} controls \"Inputs\"", control.0)));
    assert!(first_result.contains(&format!("pres:{} stats \"Totals\"", stats.0)));
    assert!(second_result.contains(&format!("pres:{} controls \"Inputs\"", control.0)));
    assert!(second_result.contains(&format!("pres:{} stats \"Totals\"", stats.0)));
}

#[tokio::test]
async fn a_delegated_card_with_an_unmapped_source_is_reported_as_a_transfer_error() {
    let dir = tempdir().unwrap();
    let provider = scripted(vec![
        call(
            "t1",
            "task",
            serde_json::json!({"prompt": "make the cards"}),
        ),
        call(
            "c1",
            "emit_controls_card",
            serde_json::json!({
                "semantic_type": "controls", "title": "Inputs",
                "inputs": [{"label": "Base", "name": "base", "kind": "number", "value": 4}],
                "outputs": []
            }),
        ),
        call(
            "s1",
            "emit_stats_card",
            serde_json::json!({
                "semantic_type": "stats", "title": "Broken totals",
                "readings": [{"label": "Total", "value": 4}],
                "bound": [{"path": ["readings", 0, "value"], "expression": "base", "from": ["child-card-not-transferred"]}]
            }),
        ),
        text_msg("finished child"),
        text_msg("Done."),
    ]);
    let task = task_tool(&dir, provider.clone(), "parent-unmapped");
    let mut parent = agent(&dir, "parent-unmapped", provider, vec![task]);
    let outcome = run(&mut parent, "make those cards").await;
    assert!(
        matches!(outcome, TurnOutcome::Completed { .. }),
        "{outcome:?}"
    );

    let session = parent.session.lock().await;
    let delegated: Vec<_> = session
        .presentations()
        .into_iter()
        .filter(|(_, record)| matches!(record.source, PresentationSource::Delegated { .. }))
        .collect();
    assert_eq!(
        delegated.len(),
        1,
        "only the mapped controls card transfers"
    );
    let result = tool_result(&session, "t1");
    assert!(tool_result_is_error(&session, "t1"));
    assert!(
        result.contains("some cards were not transferred"),
        "{result}"
    );
    assert!(result.contains("child-card-not-transferred"), "{result}");
    assert!(!result.contains("stats \"Broken totals\""), "{result}");
}

#[tokio::test]
async fn an_over_long_result_is_windowed_in_the_request_and_whole_in_the_ledger() {
    let dir = tempdir().unwrap();
    let provider = scripted(vec![
        call("b1", "big_output", serde_json::json!({})),
        call(
            "r1",
            "recall",
            serde_json::json!({"open": "ev:b1", "lines": {"start": 3000, "end": 3002}}),
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
        carried.contains("omitted") && carried.contains("recall {\"open\": \"ev:b1\""),
        "the window says what it left out and how to read it"
    );
    assert!(!carried.contains("record 03000"));

    let body = session
        .chain_to_root()
        .into_iter()
        .find_map(|entry| match &entry.payload {
            EntryPayload::EvidenceBody(body) if body.tool_use_id == "b1" => {
                session.object_text(&body.body)
            }
            _ => None,
        })
        .expect("the whole result is an object the ledger names");
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
