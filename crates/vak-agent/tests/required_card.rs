#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

//! The plan check's reframe rung (docs/design/88-control-plane.md §5): a
//! planned card still missing after the model was told requires the
//! `answer` document of one step, once.

mod support;

use std::collections::VecDeque;
use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use serde_json::Value;
use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;

use vak_agent::{Agent, AgentConfig, TurnOutcome};
use vak_llm::stream;
use vak_llm::types::{AssistantMessage, ChatRequest, ContentBlock, StopReason, Usage};
use vak_llm::{EventStream, LlmError, Provider};
use vak_permission::PermissionEngine;
use vak_session::types::{FrozenContract, SessionHeader};
use vak_session::{SessionLog, SessionPath};
use vak_tools::context::ToolContext;
use vak_tools::{Tool, ToolOutput};

struct Scripted {
    capacity_key: String,
    responses: Mutex<VecDeque<AssistantMessage>>,
    required: Arc<Mutex<Vec<Option<String>>>>,
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
        self.required.lock().unwrap().push(request.must_call);
        let next = self.responses.lock().unwrap().pop_front();
        let (mut sink, rx) = stream::channel(16);
        match next {
            Some(m) => sink.close_message(m).await,
            None => sink.close_error(LlmError::Parse("exhausted".into())).await,
        }
        Ok(rx)
    }
}

struct StepsCard;

#[async_trait]
impl Tool for StepsCard {
    fn name(&self) -> &str {
        "emit_steps_card"
    }
    fn description(&self) -> &str {
        "a card tool for tests"
    }
    fn schema(&self) -> Value {
        serde_json::json!({"type": "object"})
    }
    fn presents_cards(&self) -> bool {
        true
    }
    async fn execute(&self, _args: &Value, _ctx: &ToolContext) -> ToolOutput {
        ToolOutput::ok("Shown to the user.")
    }
}

fn message(content: Vec<ContentBlock>, stop_reason: StopReason) -> AssistantMessage {
    AssistantMessage {
        content,
        stop_reason,
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

fn text(t: &str) -> AssistantMessage {
    message(vec![ContentBlock::text(t)], StopReason::EndTurn)
}

fn call(id: &str, name: &str, input: Value) -> AssistantMessage {
    message(
        vec![ContentBlock::ToolUse {
            id: id.into(),
            name: name.into(),
            input,
        }],
        StopReason::ToolUse,
    )
}

fn agent(
    dir: &tempfile::TempDir,
    session_id: &str,
    responses: Vec<AssistantMessage>,
) -> (Agent, Arc<Mutex<Vec<Option<String>>>>) {
    let header = SessionHeader {
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
    };
    let home = dir.path().join("home");
    std::fs::create_dir_all(&home).unwrap();
    let log = SessionLog::create(
        SessionPath::new_session_file(&home, dir.path(), session_id),
        header,
    )
    .unwrap();
    let required = Arc::new(Mutex::new(Vec::new()));
    let agent = Agent::new(
        Arc::new(Scripted {
            capacity_key: crate::support::capacity_key(),
            responses: Mutex::new(VecDeque::from(responses)),
            required: required.clone(),
        }),
        log,
        {
            let mut cfg = AgentConfig::new("sys");
            cfg.model = "test-model".into();
            cfg.tools = vec![Arc::new(vak_tools::PlanTool), Arc::new(StepsCard)];
            cfg.mode = vak_permission::Mode::FullAccess;
            cfg.permission = Some(Arc::new(PermissionEngine::default()));
            cfg.stop_policy = None;
            cfg
        },
    );
    (agent, required)
}

async fn run(agent: &mut Agent) -> TurnOutcome {
    agent
        .run(
            "plan my week of watering",
            &Default::default(),
            CancellationToken::new(),
            mpsc::channel(64).0,
        )
        .await
}

fn plan(form: &str) -> AssistantMessage {
    let deliverable = serde_json::json!({"name": "watering", "form": form});
    call(
        "p1",
        "plan",
        serde_json::json!({ "deliverables": [deliverable] }),
    )
}

#[tokio::test]
async fn planned_cards_still_missing_require_the_answer_of_one_step() {
    let dir = tempfile::tempdir().unwrap();
    let (mut agent, required) = agent(
        &dir,
        "required-card",
        vec![
            plan("card"),
            text("Water on Monday and Thursday."),
            text("As I said: Monday and Thursday."),
            text("Still only words."),
        ],
    );
    let outcome = run(&mut agent).await;
    assert!(
        matches!(&outcome, TurnOutcome::Completed { response } if response.text_content() == "Still only words."),
        "{outcome:?}"
    );
    assert_eq!(
        *required.lock().unwrap(),
        vec![None, None, None, Some("answer".to_string())],
        "told once, then one step is required to call the tool, then the answer stands"
    );
    let session = agent.session.lock().await;
    let rungs: Vec<vak_control::Rung> = session
        .explanation()
        .into_iter()
        .filter_map(|(_, step)| match step {
            vak_session::log::TurnStep::Decision(decision) => match &decision.choice {
                vak_control::Choice::Correct(correction) => Some(correction.rung),
                _ => None,
            },
            _ => None,
        })
        .collect();
    assert_eq!(
        rungs,
        vec![
            vak_control::Rung::Tell,
            vak_control::Rung::Reframe,
            vak_control::Rung::End
        ]
    );
}

#[tokio::test]
async fn a_plan_with_no_card_never_requires_the_answer() {
    let dir = tempfile::tempdir().unwrap();
    let (mut agent, required) = agent(
        &dir,
        "no-named-tool",
        vec![
            plan("text"),
            text("Water on Monday and Thursday."),
            text("As I said: Monday and Thursday."),
        ],
    );
    let outcome = run(&mut agent).await;
    assert!(
        matches!(&outcome, TurnOutcome::Completed { .. }),
        "{outcome:?}"
    );
    assert!(required.lock().unwrap().iter().all(Option::is_none));
}
