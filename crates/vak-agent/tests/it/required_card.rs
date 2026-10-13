#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

//! The plan check's reframe rung (docs/design/88-control-plane.md §5): a
//! planned card still missing after the model was told requires one step
//! to call the card tool of the kind the plan named, once.

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
    fn card_kinds(&self) -> Vec<String> {
        vec!["steps".into()]
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
async fn a_planned_card_still_missing_requires_its_card_tool_once() {
    let dir = tempfile::tempdir().unwrap();
    let (mut agent, required) = agent(
        &dir,
        "required-card",
        vec![
            plan("steps"),
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
        vec![None, None, None, Some("emit_steps_card".to_string())],
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

/// A card whose kind the plan did not name has no tool to require: the
/// model is told once and its answer then stands.
#[tokio::test]
async fn a_planned_card_of_no_named_kind_is_told_once_and_never_required() {
    let dir = tempfile::tempdir().unwrap();
    let (mut agent, required) = agent(
        &dir,
        "card-no-kind",
        vec![
            plan("card"),
            text("Water on Monday and Thursday."),
            text("As I said: Monday and Thursday."),
        ],
    );
    let outcome = run(&mut agent).await;
    assert!(
        matches!(&outcome, TurnOutcome::Completed { response } if response.text_content() == "As I said: Monday and Thursday."),
        "{outcome:?}"
    );
    assert!(required.lock().unwrap().iter().all(Option::is_none));
}

#[tokio::test]
async fn a_plan_with_no_card_never_requires_a_card_tool() {
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

struct StepsBlock;

#[async_trait]
impl Tool for StepsBlock {
    fn name(&self) -> &str {
        "steps_block"
    }
    fn description(&self) -> &str {
        "Steps in order."
    }
    fn schema(&self) -> Value {
        serde_json::json!({"type": "object", "properties": {
            "semantic_type": {"type": "string"},
            "deliverable": {"type": "string"},
            "steps": {"type": "array", "items": {"type": "string"}}
        }, "required": ["steps"]})
    }
    fn presents_cards(&self) -> bool {
        true
    }
    fn card_kinds(&self) -> Vec<String> {
        vec!["steps".into()]
    }
    async fn execute(&self, _args: &Value, _ctx: &ToolContext) -> ToolOutput {
        ToolOutput::ok("Shown to the user.")
    }
}

/// A card that names a deliverable the plan does not have is refused with
/// the plan's names; one that names nothing is refused with the planned
/// cards not shown yet, so the model links them while it works rather than
/// at the plan check; the card that names its deliverable is shown, and
/// once every planned card is shown an unnamed one is too
/// (docs/audits/cards-in-answers-2026-10-11.md, F4).
#[tokio::test]
async fn each_planned_card_is_named_by_the_card_that_delivers_it() {
    let dir = tempfile::tempdir().unwrap();
    let (mut agent, _) = agent(
        &dir,
        "named-cards",
        vec![
            plan("card"),
            call(
                "a1",
                "steps_block",
                serde_json::json!({"steps": ["Water Monday"]}),
            ),
            call(
                "a2",
                "steps_block",
                serde_json::json!({"deliverable": "morning routine", "steps": ["Wake"]}),
            ),
            call(
                "a3",
                "steps_block",
                serde_json::json!({"deliverable": "Watering", "steps": ["Water Monday"]}),
            ),
            call(
                "a4",
                "steps_block",
                serde_json::json!({"steps": ["Feed Friday"]}),
            ),
            text("Water on Monday."),
        ],
    );
    agent.config.tools = vec![Arc::new(vak_tools::PlanTool), Arc::new(StepsBlock)];
    let outcome = run(&mut agent).await;
    assert!(
        matches!(&outcome, TurnOutcome::Completed { .. }),
        "{outcome:?}"
    );
    let session = agent.session.lock().await;
    let results: Vec<(String, bool, String)> = session
        .derive_messages()
        .iter()
        .flat_map(|message| message.content.clone())
        .filter_map(|block| match block {
            ContentBlock::ToolResult {
                tool_use_id,
                content,
                is_error,
            } if tool_use_id.starts_with('a') => Some((tool_use_id, is_error, content)),
            _ => None,
        })
        .collect();
    assert_eq!(results.len(), 4, "{results:?}");
    assert!(
        results[0].1 && results[0].2.contains("not shown yet are \"watering\""),
        "an unnamed card waits for the plan's: {results:?}"
    );
    assert!(
        results[1].1
            && results[1]
                .2
                .contains("\"morning routine\", which is not a card in your plan"),
        "{results:?}"
    );
    assert!(!results[2].1, "{results:?}");
    assert!(
        !results[3].1,
        "an unnamed card after the plan's: {results:?}"
    );
}

struct CalloutBlock;

#[async_trait]
impl Tool for CalloutBlock {
    fn name(&self) -> &str {
        "callout_block"
    }
    fn description(&self) -> &str {
        "A note."
    }
    fn schema(&self) -> Value {
        serde_json::json!({"type": "object", "properties": {
            "semantic_type": {"type": "string", "enum": ["callout"]},
            "text": {"type": "string"}
        }, "required": ["text"]})
    }
    fn presents_cards(&self) -> bool {
        true
    }
    fn card_kinds(&self) -> Vec<String> {
        vec!["callout".into()]
    }
    async fn execute(&self, _args: &Value, _ctx: &ToolContext) -> ToolOutput {
        ToolOutput::ok("Shown to the user.")
    }
}

/// A card sent to a tool that does not show its kind is told which tool
/// does, from the tools' own declared kinds.
#[tokio::test]
async fn a_kind_sent_to_the_wrong_card_tool_is_told_its_owner() {
    let dir = tempfile::tempdir().unwrap();
    let (mut agent, _) = agent(
        &dir,
        "wrong-kind",
        vec![
            call(
                "k1",
                "callout_block",
                serde_json::json!({"semantic_type": "steps", "text": "Water"}),
            ),
            text("Water on Monday."),
        ],
    );
    agent.config.tools = vec![Arc::new(StepsBlock), Arc::new(CalloutBlock)];
    let outcome = run(&mut agent).await;
    assert!(
        matches!(&outcome, TurnOutcome::Completed { .. }),
        "{outcome:?}"
    );
    let session = agent.session.lock().await;
    let refusal = session
        .derive_messages()
        .iter()
        .flat_map(|message| message.content.clone())
        .find_map(|block| match block {
            ContentBlock::ToolResult {
                tool_use_id,
                content,
                is_error: true,
            } if tool_use_id == "k1" => Some(content),
            _ => None,
        })
        .expect("refused");
    assert!(
        refusal.contains("`steps` is a kind of `steps_block`"),
        "{refusal}"
    );
}
