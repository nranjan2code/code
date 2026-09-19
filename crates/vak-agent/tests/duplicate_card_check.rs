#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

//! Regression coverage for a verified real bug, observed live against the
//! real local model this app ships with (gemma4:e2b-mlx via Ollama): the
//! model calls `emit_chart_card` successfully — a card the user already
//! sees, pushed from the tool result — and then its own next answer *also*
//! writes out a `vak` fence repeating the same semantic_type, which renders
//! as a second, duplicate card (vak-server's tool-result projection and the
//! client's own fence-parsing are independent paths; nothing dedupes across
//! them). `Agent::run` must give the model one bounded repair turn asking
//! it to drop the redundant fence, reusing the same turn-loop `continue`
//! mechanism the grounding-check and malformed-fence repairs use.

use std::collections::VecDeque;
use std::sync::Arc;
use std::sync::Mutex;

use async_trait::async_trait;
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

/// Stand-in for `vak_core::presentation_tools::EmitCardTool` — returns the
/// same `{"semantic_type","payload"}` envelope shape the real tool wraps
/// its arguments in, without pulling in the vak-core dependency.
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

    async fn execute(&self, args: &serde_json::Value, _ctx: &ToolContext) -> ToolOutput {
        let envelope = serde_json::json!({
            "semantic_type": "chart",
            "payload": args.get("payload").cloned().unwrap_or(serde_json::json!({})),
        });
        ToolOutput::ok(envelope.to_string())
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

fn tool_call_msg(id: &str, name: &str, input: serde_json::Value) -> AssistantMessage {
    AssistantMessage {
        content: vec![ContentBlock::ToolUse {
            id: id.into(),
            name: name.into(),
            input,
        }],
        stop_reason: StopReason::ToolUse,
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
            responses: Mutex::new(VecDeque::from(responses)),
        }),
        log,
        {
            let mut cfg = AgentConfig::new("sys");
            cfg.model = "test-model".into();
            cfg.mode = vak_permission::Mode::FullAccess;
            cfg.permission = Some(Arc::new(PermissionEngine::default()));
            cfg.tools = vec![Arc::new(FakeEmitChartCard)];
            cfg
        },
    )
}

fn assistant_texts(agent: &Agent) -> Vec<String> {
    futures::executor::block_on(async {
        agent
            .session
            .lock()
            .await
            .derive_messages()
            .iter()
            .filter(|m| m.role == vak_llm::types::Role::Assistant)
            .flat_map(|m| m.content.iter())
            .filter_map(|b| match b {
                ContentBlock::Text { text } => Some(text.clone()),
                _ => None,
            })
            .collect()
    })
}

fn user_texts(agent: &Agent) -> Vec<String> {
    futures::executor::block_on(async {
        agent
            .session
            .lock()
            .await
            .derive_messages()
            .iter()
            .filter(|m| m.role == vak_llm::types::Role::User)
            .flat_map(|m| m.content.iter())
            .filter_map(|b| match b {
                ContentBlock::Text { text } => Some(text.clone()),
                _ => None,
            })
            .collect()
    })
}

const DUPLICATE_FENCE: &str = "Here's the chart you asked for.\n\n```vak\n{\"semantic_type\":\"chart\",\"payload\":{\"chart_type\":\"line\",\"series\":[]}}\n```";
const NARRATION_ONLY: &str = "Here's the chart you asked for.";

#[tokio::test]
async fn a_fence_repeating_a_just_emitted_card_gets_one_repair_turn() {
    let dir = tempdir().unwrap();
    let mut agent = build_agent(
        &dir,
        "dup-card-repair",
        vec![
            tool_call_msg(
                "call_1",
                "emit_chart_card",
                serde_json::json!({"semantic_type": "chart", "payload": {"chart_type": "line", "series": []}}),
            ),
            text_msg(DUPLICATE_FENCE),
            text_msg(NARRATION_ONLY),
        ],
    )
    .await;

    let outcome = agent
        .run(
            "show me a chart",
            &Default::default(),
            CancellationToken::new(),
            mpsc::channel(64).0,
        )
        .await;
    assert!(
        matches!(outcome, TurnOutcome::Completed { .. }),
        "got {outcome:?}"
    );

    let texts = assistant_texts(&agent);
    assert!(
        texts.iter().any(|t| t == NARRATION_ONLY),
        "final state must contain the de-duplicated narration-only answer: {texts:?}"
    );

    let users = user_texts(&agent);
    assert!(
        users
            .iter()
            .any(|t| t.contains("[duplicate-card-check]") && t.contains("chart")),
        "expected a duplicate-card-check repair nudge naming the semantic_type: {users:?}"
    );
}

#[tokio::test]
async fn repair_is_bounded_to_one_attempt_not_a_loop() {
    let dir = tempdir().unwrap();
    let mut agent = build_agent(
        &dir,
        "dup-card-bounded",
        vec![
            tool_call_msg(
                "call_1",
                "emit_chart_card",
                serde_json::json!({"semantic_type": "chart", "payload": {"chart_type": "line", "series": []}}),
            ),
            text_msg(DUPLICATE_FENCE),
            text_msg(DUPLICATE_FENCE), // still duplicated after the nudge
        ],
    )
    .await;

    let outcome = agent
        .run(
            "show me a chart",
            &Default::default(),
            CancellationToken::new(),
            mpsc::channel(64).0,
        )
        .await;
    assert!(
        matches!(outcome, TurnOutcome::Completed { .. }),
        "got {outcome:?}"
    );

    let texts = assistant_texts(&agent);
    assert_eq!(
        texts.iter().filter(|t| t.contains("```vak")).count(),
        2,
        "exactly one retry — not zero, not an infinite loop: {texts:?}"
    );
}

#[tokio::test]
async fn narration_without_a_duplicate_fence_is_never_touched() {
    let dir = tempdir().unwrap();
    let mut agent = build_agent(
        &dir,
        "dup-card-none",
        vec![
            tool_call_msg(
                "call_1",
                "emit_chart_card",
                serde_json::json!({"semantic_type": "chart", "payload": {"chart_type": "line", "series": []}}),
            ),
            text_msg(NARRATION_ONLY),
        ],
    )
    .await;

    let outcome = agent
        .run(
            "show me a chart",
            &Default::default(),
            CancellationToken::new(),
            mpsc::channel(64).0,
        )
        .await;
    assert!(
        matches!(outcome, TurnOutcome::Completed { .. }),
        "got {outcome:?}"
    );

    let users = user_texts(&agent);
    assert!(
        !users.iter().any(|t| t.contains("[duplicate-card-check]")),
        "an answer with no repeated fence must never trigger a repair nudge: {users:?}"
    );
}

#[tokio::test]
async fn a_fence_with_no_preceding_tool_call_is_never_touched() {
    let dir = tempdir().unwrap();
    let mut agent = build_agent(&dir, "dup-card-no-tool", vec![text_msg(DUPLICATE_FENCE)]).await;

    let outcome = agent
        .run(
            "show me a chart",
            &Default::default(),
            CancellationToken::new(),
            mpsc::channel(64).0,
        )
        .await;
    assert!(
        matches!(outcome, TurnOutcome::Completed { .. }),
        "got {outcome:?}"
    );

    let users = user_texts(&agent);
    assert!(
        !users.iter().any(|t| t.contains("[duplicate-card-check]")),
        "a fence with no matching tool call this turn is a normal fence-only card, not a duplicate: {users:?}"
    );
}
