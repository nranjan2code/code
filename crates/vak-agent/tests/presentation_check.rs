#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

//! The presentation check: an answer the app's own signal/recipe detection
//! says should be a card, written as prose with no card emitted, gets exactly
//! one nudge. Real trigger: a gpt-5.6-luna answer to "how did the Indian stock
//! market perform last week" that was a markdown table plus bullets, with the
//! `emit_*_card` tools offered and unused.

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
    with_check: bool,
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
            if with_check {
                cfg.presentation_check = Some(Arc::new(|text, offered| {
                    (text.contains("TABLE") && offered.iter().any(|t| t == "emit_chart_card")).then(
                        || {
                            "[presentation-check]: this reads as a card; call emit_chart_card."
                                .to_string()
                        },
                    )
                }));
            }
            cfg
        },
    )
}

// Raw ledger, not the model-visible projection: these tests are about the
// repair loop's mechanics, which a closed turn's full record deliberately
// no longer preserves (docs/design/68-context-engine.md §10) — a rejected
// draft and a mid-turn nudge are dropped once the turn closes.
fn assistant_texts(agent: &Agent) -> Vec<String> {
    futures::executor::block_on(async {
        agent
            .session
            .lock()
            .await
            .message_chain()
            .iter()
            .filter(|(_, m)| m.role == vak_llm::types::Role::Assistant)
            .flat_map(|(_, m)| m.content.iter())
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

const PROSE: &str = "Weekly moves as a TABLE:\n\n| Index | Change |\n|---|---|\n| Nifty | -0.22% |";
const NARRATION: &str = "The chart is shown above.";
const CHART_INPUT: fn() -> serde_json::Value = || serde_json::json!({"semantic_type": "chart", "payload": {"chart_type": "line", "series": []}});

async fn run(agent: &mut Agent) -> TurnOutcome {
    agent
        .run(
            "how did the market do",
            &Default::default(),
            CancellationToken::new(),
            mpsc::channel(64).0,
        )
        .await
}

#[tokio::test]
async fn prose_that_reads_as_a_card_gets_one_nudge_and_the_model_can_then_emit_it() {
    let dir = tempdir().unwrap();
    let mut agent = build_agent(
        &dir,
        "pc-nudge",
        vec![
            text_msg(PROSE),
            tool_call_msg("c1", "emit_chart_card", CHART_INPUT()),
            text_msg(NARRATION),
        ],
        true,
    )
    .await;
    assert!(matches!(
        run(&mut agent).await,
        TurnOutcome::Completed { .. }
    ));
    let users = user_texts(&agent);
    assert_eq!(
        users
            .iter()
            .filter(|t| t.contains("[presentation-check]"))
            .count(),
        1,
        "{users:?}"
    );
    assert!(assistant_texts(&agent).iter().any(|t| t == NARRATION));
}

#[tokio::test]
async fn the_nudge_is_one_shot_and_the_model_may_decline_by_resending() {
    let dir = tempdir().unwrap();
    let mut agent = build_agent(
        &dir,
        "pc-once",
        vec![text_msg(PROSE), text_msg(PROSE)],
        true,
    )
    .await;
    assert!(matches!(
        run(&mut agent).await,
        TurnOutcome::Completed { .. }
    ));
    let users = user_texts(&agent);
    assert_eq!(
        users
            .iter()
            .filter(|t| t.contains("[presentation-check]"))
            .count(),
        1,
        "{users:?}"
    );
    assert_eq!(
        assistant_texts(&agent)
            .iter()
            .filter(|t| *t == PROSE)
            .count(),
        2
    );
}

#[tokio::test]
async fn no_nudge_when_a_card_was_already_emitted_this_run() {
    let dir = tempdir().unwrap();
    let mut agent = build_agent(
        &dir,
        "pc-emitted",
        vec![
            tool_call_msg("c1", "emit_chart_card", CHART_INPUT()),
            text_msg(PROSE),
        ],
        true,
    )
    .await;
    assert!(matches!(
        run(&mut agent).await,
        TurnOutcome::Completed { .. }
    ));
    assert!(
        !user_texts(&agent)
            .iter()
            .any(|t| t.contains("[presentation-check]"))
    );
}

#[tokio::test]
async fn no_nudge_when_the_answer_already_carries_an_inline_card() {
    let dir = tempdir().unwrap();
    let with_fence = format!(
        "{PROSE}\n\n```vak\n{{\"semantic_type\":\"metric\",\"payload\":{{\"label\":\"x\",\"value\":1}}}}\n```"
    );
    let mut agent = build_agent(&dir, "pc-fence", vec![text_msg(&with_fence)], true).await;
    assert!(matches!(
        run(&mut agent).await,
        TurnOutcome::Completed { .. }
    ));
    assert!(
        !user_texts(&agent)
            .iter()
            .any(|t| t.contains("[presentation-check]"))
    );
}

#[tokio::test]
async fn with_no_check_configured_the_loop_is_untouched() {
    let dir = tempdir().unwrap();
    let mut agent = build_agent(&dir, "pc-off", vec![text_msg(PROSE)], false).await;
    assert!(matches!(
        run(&mut agent).await,
        TurnOutcome::Completed { .. }
    ));
    assert!(
        !user_texts(&agent)
            .iter()
            .any(|t| t.contains("[presentation-check]"))
    );
}
