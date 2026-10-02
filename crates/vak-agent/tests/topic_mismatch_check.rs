#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

//! Topic-mismatch enforcement (docs/design/68-context-engine.md §7).
//! Regression coverage for a real defect found live on the shipped 3.5.0
//! build: asked "what is the current top news in AI", the model correctly
//! called `tavily_search`, got real AI-news results back, and then wrote an
//! `emit_metric_card` for "Noida Weather, 28°C" — a payload copied from an
//! unrelated, much older turn still sitting in its own context — instead of
//! answering from the evidence it had just retrieved. The freshness check
//! did not catch this: a real retrieval HAD succeeded this run, so freshness
//! had nothing to say; the card's own content was simply unrelated to both
//! the question and the evidence.

mod support;

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
use vak_session::types::{FrozenContract, SessionHeader};
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
        "tavily_search"
    }
    fn description(&self) -> &str {
        "fake search tool for tests"
    }
    fn schema(&self) -> Value {
        serde_json::json!({"type": "object", "properties": {"query": {"type": "string"}}})
    }
    async fn execute(&self, _args: &Value, _ctx: &ToolContext) -> ToolOutput {
        ToolOutput::ok(
            "Title: AI News | Latest News | Insights Powering AI-Driven Business\n\
             URL: https://www.artificialintelligence-news.example/\n\
             OpenAI today unveiled GPT-6, its newest model, at a launch event.\n",
        )
    }
}

struct FakeCardTool;

#[async_trait]
impl Tool for FakeCardTool {
    fn name(&self) -> &str {
        "emit_metric_card"
    }
    fn description(&self) -> &str {
        "fake card tool for tests"
    }
    fn schema(&self) -> Value {
        serde_json::json!({"type": "object"})
    }
    fn presents_cards(&self) -> bool {
        true
    }
    async fn execute(&self, _args: &Value, _ctx: &ToolContext) -> ToolOutput {
        ToolOutput::ok("{\"ok\":true}")
    }
}

struct FakeEntityTool;

#[async_trait]
impl Tool for FakeEntityTool {
    fn name(&self) -> &str {
        "emit_entity_card"
    }
    fn description(&self) -> &str {
        "fake entity card tool for tests"
    }
    fn schema(&self) -> Value {
        serde_json::json!({"type": "object"})
    }
    fn presents_cards(&self) -> bool {
        true
    }
    async fn execute(&self, _args: &Value, _ctx: &ToolContext) -> ToolOutput {
        ToolOutput::ok("{\"ok\":true}")
    }
}

fn search_call(id: &str) -> AssistantMessage {
    AssistantMessage {
        content: vec![ContentBlock::ToolUse {
            id: id.into(),
            name: "tavily_search".into(),
            input: serde_json::json!({"query": "current top news in AI"}),
        }],
        stop_reason: StopReason::ToolUse,
        usage: Usage::default(),
        model: "test-model".into(),
        response_id: None,
    }
}

fn weather_card_call(id: &str) -> AssistantMessage {
    AssistantMessage {
        content: vec![ContentBlock::ToolUse {
            id: id.into(),
            name: "emit_metric_card".into(),
            input: serde_json::json!({
                "semantic_type": "weather",
                "payload": {"label": "Noida Weather", "unit": "Celsius", "value": "28°C"}
            }),
        }],
        stop_reason: StopReason::ToolUse,
        usage: Usage::default(),
        model: "test-model".into(),
        response_id: None,
    }
}

fn ai_entity_card_call(id: &str) -> AssistantMessage {
    AssistantMessage {
        content: vec![ContentBlock::ToolUse {
            id: id.into(),
            name: "emit_entity_card".into(),
            input: serde_json::json!({
                "semantic_type": "entity",
                "payload": {"title": "OpenAI announces GPT-6", "summary": "newest model launch"}
            }),
        }],
        stop_reason: StopReason::ToolUse,
        usage: Usage::default(),
        model: "test-model".into(),
        response_id: None,
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

async fn build_agent(
    dir: &tempfile::TempDir,
    session_id: &str,
    responses: Vec<AssistantMessage>,
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
    let log = SessionLog::create(
        SessionPath::new_session_file(&home, dir.path(), session_id),
        header,
    )
    .unwrap();

    Agent::new(
        Arc::new(Scripted {
            capacity_key: crate::support::capacity_key(),
            responses: Mutex::new(VecDeque::from(responses)),
        }),
        log,
        {
            let mut cfg = AgentConfig::new("sys");
            cfg.model = "test-model".into();
            cfg.tools = vec![
                Arc::new(FakeSearchTool),
                Arc::new(FakeCardTool),
                Arc::new(FakeEntityTool),
            ];
            cfg.retrieval_check = Some(Arc::new(|name: &str, _: &Value| name == "tavily_search"));
            cfg.mode = vak_permission::Mode::FullAccess;
            cfg.permission = Some(Arc::new(PermissionEngine::default()));
            cfg
        },
    )
}

/// The tool_result content for a given `tool_use_id`, raw from the ledger.
/// This harness never wires `presentation_rebuild` (that hook lives in
/// `Core`, not `Agent`), so a card call's success or gating is only visible
/// here — never in `session.presentations()`, which stays empty regardless.
fn tool_result_for(agent: &Agent, tool_use_id: &str) -> Option<(String, bool)> {
    futures::executor::block_on(async {
        agent
            .session
            .lock()
            .await
            .message_chain()
            .iter()
            .flat_map(|(_, m)| m.content.clone())
            .find_map(|b| match b {
                ContentBlock::ToolResult {
                    tool_use_id: id,
                    content,
                    is_error,
                } if id == tool_use_id => Some((content, is_error)),
                _ => None,
            })
    })
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
async fn the_real_regression_is_gated_and_evidence_is_shown_instead() {
    let dir = tempdir().unwrap();
    let mut agent = build_agent(
        &dir,
        "topic-mismatch-real-bug",
        vec![
            // Exactly what really happened: correct search, then an
            // unrelated card written from an older turn, twice.
            search_call("s1"),
            weather_card_call("c1"),
            weather_card_call("c2"),
        ],
    )
    .await;

    let outcome = agent
        .run(
            "what is the current top news in AI",
            &Default::default(),
            CancellationToken::new(),
            mpsc::channel(64).0,
        )
        .await;

    // The first weather-card attempt is refused, not executed, by the
    // topic gate specifically.
    let (content, is_error) = tool_result_for(&agent, "c1").expect("a result for c1");
    assert!(is_error, "c1 must be refused, not executed: {content}");
    assert!(
        content.starts_with("[topic-mismatch]"),
        "c1 must be refused by the topic gate specifically: {content}"
    );
    // The second attempt is the same mismatched card again: the turn ends
    // there (matching the freshness gate's own precedent) before a second
    // tool_result is ever recorded for it.
    assert!(tool_result_for(&agent, "c2").is_none());

    match &outcome {
        TurnOutcome::Completed { response } => {
            let text = response.text_content();
            assert!(
                text.contains("OpenAI") || text.contains("GPT-6") || text.contains("AI News"),
                "the real evidence gathered this run must surface instead of nothing: {text}"
            );
            assert!(
                !text.contains("Noida"),
                "the wrong card's content must not leak through: {text}"
            );
        }
        other => panic!("expected a completed turn, got {other:?}"),
    }

    let nudges = user_texts(&agent);
    assert_eq!(
        nudges
            .iter()
            .filter(|t| t.starts_with("[topic-mismatch]"))
            .count(),
        0,
        "the gate answers with a tool-result error, not a session-visible nudge in this path: {nudges:?}"
    );
}

#[tokio::test]
async fn a_correctly_retrieved_card_that_renames_the_topic_is_not_gated() {
    let dir = tempdir().unwrap();
    let mut agent = build_agent(
        &dir,
        "topic-mismatch-paraphrase-safe",
        vec![
            search_call("s1"),
            // Titled from what the search actually said, not from the
            // user's own words — must NOT be gated.
            ai_entity_card_call("c1"),
            text_msg("OpenAI just announced GPT-6."),
        ],
    )
    .await;

    let outcome = agent
        .run(
            "what is the current top news in AI",
            &Default::default(),
            CancellationToken::new(),
            mpsc::channel(64).0,
        )
        .await;

    assert!(
        matches!(&outcome, TurnOutcome::Completed { .. }),
        "got {outcome:?}"
    );
    let (content, is_error) =
        tool_result_for(&agent, "c1").expect("a result for the entity card call");
    assert!(
        !is_error,
        "the correctly-derived, differently-titled card must not be gated: {content}"
    );
    assert_eq!(content, "{\"ok\":true}");
}
