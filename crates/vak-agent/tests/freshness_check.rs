#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

//! Freshness and empty-step enforcement (docs/design/68-context-engine.md
//! §7). Freshness: a directive
//! whose reading carries the `live-data` domain asks for a value as it
//! stands now. If the model answers — in prose or with a card — without any
//! retrieval-shaped call succeeding in the run, the answer can only repeat
//! what an earlier turn found, so it gets exactly one `[freshness-check]`
//! redo. Observed live: six replays of "what is the current weather in new
//! delhi" on gemma4:e2b-mlx emitted a metric card five times without
//! searching, carrying a temperature from a previous turn's answer.

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
    responses: Mutex<VecDeque<AssistantMessage>>,
}

#[async_trait]
impl Provider for Scripted {
    fn name(&self) -> &str {
        "scripted"
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
            responses: Mutex::new(VecDeque::from(responses)),
        }),
        log,
        {
            let mut cfg = AgentConfig::new("sys");
            cfg.model = "test-model".into();
            cfg.tools = vec![Arc::new(FakeSearchTool)];
            cfg.retrieval_check = Some(Arc::new(|name: &str, _: &Value| name == "search"));
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
async fn a_live_data_answer_without_retrieval_gets_one_freshness_redo() {
    let dir = tempdir().unwrap();
    let mut agent = build_agent(
        &dir,
        "freshness-redo",
        vec![
            // Answers from memory of an earlier turn — no retrieval.
            text_msg("It is 29.1°C in New Delhi."),
            // After the nudge: retrieves, then answers from the result as a
            // cited card (the grounding check then has nothing to add).
            search_call("s1"),
            text_msg(
                "```vak\n{\"semantic_type\":\"metric\",\"payload\":{\"label\":\"Delhi 05:30\",\"value\":26.4,\"unit\":\"C\",\"source\":\"https://example-met.in/delhi\"}}\n```",
            ),
        ],
        true,
    )
    .await;

    let outcome = agent
        .run(
            "what is the current weather in new delhi",
            &Default::default(),
            CancellationToken::new(),
            mpsc::channel(64).0,
        )
        .await;
    assert!(
        matches!(&outcome, TurnOutcome::Completed { response } if response.text_content().contains("26.4")),
        "got {outcome:?}"
    );

    let nudges = user_texts(&agent);
    assert_eq!(
        nudges
            .iter()
            .filter(|t| t.starts_with("[freshness-check]"))
            .count(),
        1,
        "exactly one freshness nudge: {nudges:?}"
    );
}

#[tokio::test]
async fn admitting_no_live_data_is_accepted_without_a_redo() {
    let dir = tempdir().unwrap();
    let mut agent = build_agent(
        &dir,
        "freshness-admit",
        vec![text_msg(
            "I don't have live data for New Delhi right now; the last reading I saw was from an earlier turn.",
        )],
        true,
    )
    .await;
    let outcome = agent
        .run(
            "what is the current weather in new delhi",
            &Default::default(),
            CancellationToken::new(),
            mpsc::channel(64).0,
        )
        .await;
    assert!(
        matches!(outcome, TurnOutcome::Completed { .. }),
        "got {outcome:?}"
    );
    assert!(
        !user_texts(&agent)
            .iter()
            .any(|t| t.starts_with("[freshness-check]"))
    );
}

#[tokio::test]
async fn a_timeless_directive_is_never_nudged() {
    let dir = tempdir().unwrap();
    let mut agent = build_agent(
        &dir,
        "freshness-timeless",
        vec![text_msg("Copper is refined by electrolysis.")],
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
        matches!(outcome, TurnOutcome::Completed { .. }),
        "got {outcome:?}"
    );
    assert!(
        !user_texts(&agent)
            .iter()
            .any(|t| t.starts_with("[freshness-check]"))
    );
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
            text.starts_with("[empty-step]") && text.contains("explain how copper is refined")
        }),
        "empty-step repair must retain the already-admitted target: {nudges:?}"
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

fn card_call(id: &str, value: &str) -> AssistantMessage {
    AssistantMessage {
        content: vec![ContentBlock::ToolUse {
            id: id.into(),
            name: "emit_metric_card".into(),
            input: serde_json::json!({"semantic_type":"metric","payload":{"label":"Delhi","value":value,"unit":"C"}}),
        }],
        stop_reason: StopReason::ToolUse,
        usage: Usage::default(),
        model: "test-model".into(),
        response_id: None,
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

#[tokio::test]
async fn a_card_in_a_live_data_turn_is_gated_until_something_is_retrieved() {
    let dir = tempdir().unwrap();
    let mut agent = build_agent(
        &dir,
        "freshness-card-gate",
        vec![
            // Card straight from memory: not executed, error value back.
            card_call("c1", "29.1"),
            // Repairs: retrieves, then the card from the result is shown.
            search_call("s1"),
            card_call("c2", "26.4"),
            text_msg("Delhi is at 26.4 C (example-met.in, 05:30)."),
        ],
        true,
    )
    .await;
    agent.config.tools.push(Arc::new(FakeCardTool));
    let outcome = agent
        .run(
            "what is the current weather in new delhi",
            &Default::default(),
            CancellationToken::new(),
            mpsc::channel(64).0,
        )
        .await;
    assert!(
        matches!(&outcome, TurnOutcome::Completed { .. }),
        "got {outcome:?}"
    );

    let results: Vec<(String, bool)> = futures::executor::block_on(async {
        agent
            .session
            .lock()
            .await
            .message_chain()
            .iter()
            .flat_map(|(_, m)| m.content.clone())
            .filter_map(|b| match b {
                ContentBlock::ToolResult {
                    content, is_error, ..
                } => Some((content, is_error)),
                _ => None,
            })
            .collect()
    });
    assert!(
        results
            .iter()
            .any(|(c, e)| *e && c.starts_with("[freshness-check]")),
        "the first card must come back as a freshness error: {results:?}"
    );
    assert!(
        !user_texts(&agent)
            .iter()
            .any(|t| t.starts_with("[freshness-check]")),
        "the gate fired at the card, so no second nudge on the final text"
    );
}

#[tokio::test]
async fn a_second_stale_card_fails_closed_with_an_honest_answer() {
    let dir = tempdir().unwrap();
    let mut agent = build_agent(
        &dir,
        "freshness-fail-closed",
        vec![
            card_call("c1", "29.1"),
            // The "repair" is another carried-over figure, still no retrieval.
            card_call("c2", "28"),
            text_msg("unreachable"),
        ],
        true,
    )
    .await;
    agent.config.tools.push(Arc::new(FakeCardTool));
    let outcome = agent
        .run(
            "what is the current weather in new delhi",
            &Default::default(),
            CancellationToken::new(),
            mpsc::channel(64).0,
        )
        .await;
    match &outcome {
        TurnOutcome::Completed { response } => {
            let text = response.text_content();
            assert!(
                text.contains("not presenting a carried-over figure as current"),
                "fail-closed answer expected, got: {text}"
            );
            assert!(!text.contains("unreachable"));
        }
        other => panic!("expected a completed turn, got {other:?}"),
    }
    let presented =
        futures::executor::block_on(async { agent.session.lock().await.presentations().len() });
    assert_eq!(
        presented, 0,
        "no stale card may reach the ledger as a presentation"
    );
    // The system-authored answer is on the ledger, so the turn is closed
    // and carded like any other.
    let (last_is_answer, carded) = futures::executor::block_on(async {
        let session = agent.session.lock().await;
        let last = session
            .message_chain()
            .last()
            .map(|(_, m)| m.text_content().contains("carried-over figure"))
            .unwrap_or(false);
        (last, session.turn_cards().len())
    });
    assert!(last_is_answer, "the fail-closed answer must be logged");
    assert_eq!(carded, 1, "the turn closes with a TurnCard");
}
