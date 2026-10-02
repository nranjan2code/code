//! A worker asking its parent a question
//! (docs/design/84-worker-questions-and-control.md §4): the question reaches
//! whoever can answer, the answer reaches the worker as information, and a
//! surface nobody is watching ends the call at once.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

mod support;

use std::collections::VecDeque;
use std::sync::{Arc, Mutex};

use tempfile::tempdir;
use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;

use vak_agent::{Agent, AgentConfig, Approver, TaskDeps, TaskTool, TurnOutcome, WorkerRegistry};
use vak_llm::stream;
use vak_llm::types::{AssistantMessage, ChatRequest, ContentBlock, StopReason, Usage};
use vak_llm::{EventStream, LlmError, Provider};
use vak_permission::PermissionEngine;
use vak_session::types::{FrozenContract, SessionHeader};
use vak_session::{SessionLog, SessionPath};
use vak_tools::read::ReadTool;

struct Scripted {
    capacity_key: String,
    responses: Mutex<VecDeque<AssistantMessage>>,
    requests: Arc<Mutex<Vec<ChatRequest>>>,
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

/// An approver whose surface can show a question and take an answer.
struct Watched;

#[async_trait::async_trait]
impl Approver for Watched {
    async fn approve(&self, _: &str, _: &str, _: &str) -> bool {
        true
    }
    fn answers_questions(&self) -> bool {
        true
    }
}

/// An approver that can answer a yes/no gate but cannot show a question.
struct GateOnly;

#[async_trait::async_trait]
impl Approver for GateOnly {
    async fn approve(&self, _: &str, _: &str, _: &str) -> bool {
        true
    }
}

/// An approver that says nobody is watching.
struct Unattended;

#[async_trait::async_trait]
impl Approver for Unattended {
    async fn approve(&self, _: &str, _: &str, _: &str) -> bool {
        false
    }
    fn answerable(&self) -> bool {
        false
    }
}

struct Run {
    outcome: TurnOutcome,
    requests: Vec<ChatRequest>,
}

async fn run_with_worker_question(
    approver: Arc<dyn Approver>,
    answer_with: Option<&'static str>,
) -> Run {
    let dir = tempdir().unwrap();
    let home = dir.path().join("home");
    std::fs::create_dir_all(&home).unwrap();
    let parent_id = "parent-q".to_string();
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
        capacity_key: crate::support::capacity_key(),
        responses: Mutex::new(VecDeque::from(vec![
            call(
                "t1",
                "task",
                serde_json::json!({"prompt": "total the year", "label": "totals"}),
            ),
            call(
                "a1",
                "ask_parent",
                serde_json::json!({"question": "Which fiscal year?", "options": ["2025", "2026"]}),
            ),
            text("child used the answer"),
            text("parent done"),
        ])),
        requests: requests.clone(),
    });
    let registry = Arc::new(WorkerRegistry::new());
    let mut cfg = AgentConfig::new("sys");
    cfg.model = "test-model".into();
    cfg.tools = vec![Arc::new(TaskTool::new(TaskDeps {
        parent_agent_identity: None,
        role_prompts: Default::default(),
        provider: scripted.clone(),
        system_prompt: "child-sys".into(),
        child_prompt: None,
        trust_project: false,
        tail: Default::default(),
        model: "test-model".into(),
        tools: vec![Arc::new(ReadTool)],
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
        max_output: 0,
        declared_window: 0,
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
        mode: vak_permission::Mode::ReadOnly,
        approval_mode: vak_agent::ApprovalMode::Ask,
        approver: Some(approver.clone()),
        sandbox: None,
        cwd: dir.path().to_path_buf(),
        sessions_home: home.clone(),
        parent_session_id: parent_id.clone(),
        contract_id: None,
        work_item_id: None,
        work_item_ids: vec![],
        events: None,
        registry: Some(registry.clone()),
    }))];
    cfg.permission = Some(Arc::new(
        PermissionEngine::from_rule_strings(&["+task".to_string()]).unwrap(),
    ));
    cfg.approver = Some(approver);

    if let Some(answer) = answer_with {
        let registry = registry.clone();
        tokio::spawn(async move {
            loop {
                if let Some(question) = registry.questions().pending("parent-q").into_iter().next()
                {
                    assert_eq!(question.question, "Which fiscal year?");
                    assert_eq!(question.options, vec!["2025", "2026"]);
                    registry
                        .questions()
                        .answer("parent-q", &question.id, answer, "person:test")
                        .unwrap();
                    return;
                }
                tokio::task::yield_now().await;
            }
        });
    }

    let mut agent = Agent::new(scripted, log, cfg);
    let outcome = tokio::time::timeout(
        std::time::Duration::from_secs(20),
        agent.run(
            "delegate please",
            &Default::default(),
            CancellationToken::new(),
            mpsc::channel(64).0,
        ),
    )
    .await
    .expect("the run must not hang on a question");
    let requests = requests.lock().unwrap().clone();
    drop(dir);
    Run { outcome, requests }
}

fn every_text(requests: &[ChatRequest]) -> String {
    requests
        .iter()
        .flat_map(|r| r.messages.iter())
        .flat_map(|m| m.content.iter())
        .filter_map(|b| match b {
            ContentBlock::ToolResult { content, .. } => Some(content.clone()),
            ContentBlock::Text { text } => Some(text.clone()),
            _ => None,
        })
        .collect::<Vec<_>>()
        .join("\n")
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_workers_question_is_answered_and_the_answer_reaches_it() {
    let run = run_with_worker_question(Arc::new(Watched), Some("2026")).await;
    assert!(
        matches!(run.outcome, TurnOutcome::Completed { .. }),
        "{:?}",
        run.outcome
    );
    let seen = every_text(&run.requests);
    assert!(
        seen.contains("Answer from person:test: 2026"),
        "the worker's next step must carry the answer: {seen}"
    );
    let child_requests: Vec<_> = run
        .requests
        .iter()
        .filter(|r| r.system.as_deref() == Some("child-sys"))
        .collect();
    assert!(
        child_requests
            .iter()
            .all(|r| r.tools.iter().all(|t| t.name != "task")),
        "a worker still cannot spawn workers"
    );
    assert!(
        child_requests
            .iter()
            .all(|r| r.tools.iter().any(|t| t.name == "ask_parent")),
        "every worker, read-only included, can ask"
    );
    assert!(
        run.requests
            .first()
            .unwrap()
            .tools
            .iter()
            .all(|t| t.name != "ask_parent"),
        "the parent's own tool set never carries ask_parent"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn an_unwatched_surface_ends_the_question_at_once() {
    let run = run_with_worker_question(Arc::new(Unattended), None).await;
    assert!(
        matches!(run.outcome, TurnOutcome::Completed { .. }),
        "{:?}",
        run.outcome
    );
    let seen = every_text(&run.requests);
    assert!(seen.contains("Nobody is available to answer"), "{seen}");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_surface_that_cannot_show_a_question_ends_it_at_once() {
    let run = run_with_worker_question(Arc::new(GateOnly), None).await;
    assert!(
        matches!(run.outcome, TurnOutcome::Completed { .. }),
        "{:?}",
        run.outcome
    );
    let seen = every_text(&run.requests);
    assert!(
        seen.contains("Nobody is available to answer"),
        "a surface that answers gates but not questions must not leave the worker blocked: {seen}"
    );
}
