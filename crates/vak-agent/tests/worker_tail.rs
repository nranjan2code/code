#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

//! Worker tail (docs/design/68-context-engine.md §6/§10): a spawned child
//! agent must carry the same turn context block (clock instant + epistemic
//! stance) as its parent turn, not an empty one — see `TaskDeps::tail` and
//! `Core`'s `TaskDeps` construction, which now thread the parent's
//! `AgentConfig::tail` through instead of leaving the child's default.
//! Assembly and single-turn stability of the tail itself are covered by
//! `context_tail.rs`; this test only checks that a WORKER's own request
//! carries it too.

use std::collections::VecDeque;
use std::sync::{Arc, Mutex};

use tempfile::tempdir;
use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;

use vak_agent::{Agent, AgentConfig, AutoApprove, TailInput, TaskDeps, TaskTool, TurnOutcome};
use vak_llm::stream;
use vak_llm::types::{AssistantMessage, ChatRequest, ContentBlock, Role, StopReason, Usage};
use vak_llm::{EventStream, LlmError, Provider};
use vak_permission::{Mode, PermissionEngine};
use vak_session::types::{FrozenContract, SessionHeader};
use vak_session::{SessionLog, SessionPath};
use vak_tools::read::ReadTool;

/// Records every request it sees and replays a scripted queue of
/// responses — shared between the parent agent and its spawned child, so
/// both turns' requests land in one inspectable list, in dispatch order.
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
        response_id: None,
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
        response_id: None,
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn spawned_worker_request_carries_the_parent_turns_tail() {
    let dir = tempdir().unwrap();
    let home = dir.path().join("home");
    std::fs::create_dir_all(&home).unwrap();
    let parent_id = "parent-tail".to_string();

    let header = SessionHeader {
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

    let parent_tail = TailInput {
        temporal: "current UTC instant 2026-09-19T00:00:00Z".into(),
        stance: "Provide a clear, direct answer.".into(),
    };

    let mut cfg = AgentConfig::new("sys");
    cfg.model = "test-model".into();
    cfg.tail = parent_tail.clone();
    cfg.tools = vec![Arc::new(TaskTool::new(TaskDeps {
        parent_agent_identity: None,
        role_prompts: Default::default(),
        provider: scripted.clone(),
        system_prompt: "child-sys".into(),
        // The field under test: `Core` threads `cfg.tail.clone()` through
        // here (crates/vak-core/src/lib.rs, TaskDeps construction) rather
        // than leaving the child with an empty `TailInput::default()`.
        tail: parent_tail.clone(),
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
        mode: Mode::WorkspaceWrite,
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
        registry: Some(Arc::new(vak_agent::WorkerRegistry::new())),
    }))];
    cfg.permission = Some(Arc::new(
        PermissionEngine::from_rule_strings(&["+task".to_string()]).unwrap(),
    ));
    cfg.approver = Some(Arc::new(AutoApprove));

    let mut agent = Agent::new(scripted, log, cfg);
    let outcome = agent
        .run(
            "delegate please",
            &Default::default(),
            CancellationToken::new(),
            mpsc::channel(64).0,
        )
        .await;
    assert!(matches!(outcome, TurnOutcome::Completed { .. }));

    let reqs = requests.lock().unwrap();
    assert!(
        reqs.len() >= 2,
        "parent call + child call expected, got {}",
        reqs.len()
    );
    let child_request = &reqs[1];
    assert_eq!(
        child_request.system.as_deref(),
        Some("child-sys"),
        "reqs[1] must be the child's own request"
    );

    let last = child_request
        .messages
        .last()
        .expect("child request has at least one message");
    assert_eq!(
        last.role,
        Role::User,
        "the tail rides the child's last USER message"
    );
    let tail_text = match last.content.last().expect("at least one content block") {
        ContentBlock::Text { text } => text.clone(),
        other => panic!("expected the tail as a trailing text block, got {other:?}"),
    };
    assert!(
        tail_text.contains("<turn_context>"),
        "worker request tail: {tail_text}"
    );
    assert!(
        tail_text.contains("current UTC instant 2026-09-19T00:00:00Z"),
        "worker must carry the PARENT turn's temporal context, not an empty one: {tail_text}"
    );
    assert!(tail_text.contains("<stance>"));
    assert!(
        tail_text.contains("Provide a clear, direct answer."),
        "worker must carry the parent turn's epistemic stance: {tail_text}"
    );
}
