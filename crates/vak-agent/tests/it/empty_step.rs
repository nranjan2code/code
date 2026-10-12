#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

//! Empty-step enforcement (docs/design/68-context-engine.md §7): a response
//! with neither text nor a tool call is told once to act or answer; after a
//! tool call a later empty step is a new one.

use std::collections::VecDeque;
use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use serde_json::Value;
use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;

use tempfile::tempdir;

use vak_agent::{Agent, AgentConfig, TurnOutcome};
use vak_llm::stream;
use vak_llm::types::{AssistantMessage, ChatRequest, ContentBlock, StopReason, Usage};
use vak_llm::{EventStream, LlmError, Provider};
use vak_permission::PermissionEngine;
use vak_session::types::{FrozenContract, IntentRecord, SessionHeader};
use vak_session::{SessionLog, SessionPath};
use vak_tools::context::ToolContext;
use vak_tools::{Tool, ToolOutput};

struct Scripted {
    capacity_key: String,
    responses: Mutex<VecDeque<AssistantMessage>>,
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
        _request: ChatRequest,
        _cancel: CancellationToken,
    ) -> Result<EventStream, LlmError> {
        let next = self.responses.lock().unwrap().pop_front();
        let (mut sink, rx) = stream::channel(16);
        match next {
            Some(m) => sink.close_message(m).await,
            None => sink.close_error(LlmError::Parse("exhausted".into())).await,
        }
        Ok(rx)
    }
}

struct FakeSearchTool;

#[async_trait]
impl Tool for FakeSearchTool {
    fn name(&self) -> &str {
        "search"
    }
    fn description(&self) -> &str {
        "fake search tool for tests"
    }
    fn schema(&self) -> Value {
        serde_json::json!({"type": "object", "properties": {"query": {"type": "string"}}})
    }
    async fn execute(&self, _args: &Value, _ctx: &ToolContext) -> ToolOutput {
        ToolOutput::ok(
            "Title: Delhi observations 05:30\nURL: https://example-met.in/delhi\nTemperature: 26.4 C\n",
        )
    }
}

fn search_call(id: &str) -> AssistantMessage {
    AssistantMessage {
        content: vec![ContentBlock::ToolUse {
            id: id.into(),
            name: "search".into(),
            input: serde_json::json!({"query": "delhi observations now"}),
        }],
        stop_reason: StopReason::ToolUse,
        usage: Usage::default(),
        model: "test-model".into(),
        response_id: None,
        applied: None,
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
        applied: None,
    }
}

fn thinking_only() -> AssistantMessage {
    AssistantMessage {
        content: vec![ContentBlock::Thinking {
            text: "Final Plan: 1. Use search to get the current reading. 2. Answer.".into(),
            signature: None,
        }],
        stop_reason: StopReason::EndTurn,
        usage: Usage::default(),
        model: "test-model".into(),
        response_id: None,
        applied: None,
    }
}

/// Seeds the reading vak-core would have written at turn start for a
/// directive with temporal deixis.
fn live_data_intent() -> IntentRecord {
    let mut reading = vak_intent::Reading::general();
    reading.domains.insert("live-data".into());
    IntentRecord {
        reading,
        engagement: vak_intent::Engagement::general(),
        provenance: vak_intent::Provenance::new(vak_intent::Tier::Signals, 1, Vec::new()),
        outcome: None,
        model_visible: None,
        commitment_id: None,
        strands: Vec::new(),
        strand_commitments: Default::default(),
    }
}

async fn build_agent(
    dir: &tempfile::TempDir,
    session_id: &str,
    responses: Vec<AssistantMessage>,
    live_data: bool,
) -> Agent {
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
    let mut log = SessionLog::create(
        SessionPath::new_session_file(&home, dir.path(), session_id),
        header,
    )
    .unwrap();
    if live_data {
        log.append_intent(live_data_intent()).unwrap();
    }

    Agent::new(
        Arc::new(Scripted {
            capacity_key: crate::support::capacity_key(),
            responses: Mutex::new(VecDeque::from(responses)),
        }),
        log,
        {
            let mut cfg = AgentConfig::new("sys");
            cfg.model = "test-model".into();
            cfg.tools = vec![Arc::new(FakeSearchTool)];
            cfg.mode = vak_permission::Mode::FullAccess;
            cfg.permission = Some(Arc::new(PermissionEngine::default()));
            cfg
        },
    )
}

fn user_texts(agent: &Agent) -> Vec<String> {
    futures::executor::block_on(async {
        agent
            .session
            .lock()
            .await
            .message_chain()
            .iter()
            .filter(|(_, m)| m.role == vak_llm::types::Role::User)
            .flat_map(|(_, m)| m.content.iter())
            .filter_map(|b| match b {
                ContentBlock::Text { text } => Some(text.clone()),
                _ => None,
            })
            .collect()
    })
}

#[tokio::test]
async fn a_thinking_only_step_gets_one_redo_to_act() {
    let dir = tempdir().unwrap();
    let mut agent = build_agent(
        &dir,
        "empty-step-redo",
        vec![
            thinking_only(),
            text_msg("Copper is refined by electrolysis."),
        ],
        false,
    )
    .await;
    let outcome = agent
        .run(
            "explain how copper is refined",
            &Default::default(),
            CancellationToken::new(),
            mpsc::channel(64).0,
        )
        .await;
    assert!(
        matches!(&outcome, TurnOutcome::Completed { response } if response.text_content().contains("electrolysis")),
        "got {outcome:?}"
    );
    let nudges = user_texts(&agent);
    assert_eq!(
        nudges
            .iter()
            .filter(|t| t.starts_with("[empty-step]"))
            .count(),
        1,
        "exactly one empty-step nudge: {nudges:?}"
    );
    assert!(
        nudges.iter().any(|text| {
            text.starts_with("[empty-step]") && !text.contains("explain how copper is refined")
        }),
        "empty-step repair must not quote the request back: {nudges:?}"
    );
}

#[tokio::test]
async fn a_tool_call_starts_a_fresh_bounded_empty_step_boundary() {
    let dir = tempdir().unwrap();
    let mut agent = build_agent(
        &dir,
        "empty-step-after-tool",
        vec![
            thinking_only(),
            search_call("s1"),
            thinking_only(),
            text_msg("The retrieved observation reports 26.4 C in Delhi."),
        ],
        false,
    )
    .await;
    let outcome = agent
        .run(
            "find the Delhi observation and answer",
            &Default::default(),
            CancellationToken::new(),
            mpsc::channel(64).0,
        )
        .await;
    assert!(
        matches!(&outcome, TurnOutcome::Completed { response } if response.text_content().contains("26.4")),
        "got {outcome:?}"
    );
    assert_eq!(
        user_texts(&agent)
            .iter()
            .filter(|text| text.starts_with("[empty-step]"))
            .count(),
        2,
        "each action boundary gets one bounded repair"
    );
}

#[tokio::test]
async fn two_thinking_only_steps_fail_instead_of_completing_without_work() {
    let dir = tempdir().unwrap();
    let mut agent = build_agent(
        &dir,
        "empty-step-failure",
        vec![thinking_only(), thinking_only()],
        false,
    )
    .await;
    let outcome = agent
        .run(
            "create a draft",
            &Default::default(),
            CancellationToken::new(),
            mpsc::channel(64).0,
        )
        .await;
    assert!(
        matches!(&outcome, TurnOutcome::Failed { error: vak_llm::LlmError::Parse(message) }
            if message.contains("no visible answer")),
        "got {outcome:?}"
    );
    assert_eq!(
        user_texts(&agent)
            .iter()
            .filter(|text| text.starts_with("[empty-step]"))
            .count(),
        1
    );
}
