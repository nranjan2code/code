#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

//! The presentation check: an answer the app's own signal/recipe detection
//! says should be a card, written as prose with no card emitted, gets exactly
//! one nudge. Real trigger: a gpt-5.6-luna answer to "how did the Indian stock
//! market perform last week" that was a markdown table plus bullets, with the
//! `emit_*_card` tools offered and unused.

mod support;

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

type Requests = Arc<Mutex<Vec<Vec<String>>>>;

struct Scripted {
    capacity_key: String,
    responses: Mutex<VecDeque<AssistantMessage>>,
    /// The tool names each request actually declared.
    requests: Requests,
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
        self.requests
            .lock()
            .unwrap()
            .push(request.tools.iter().map(|t| t.name.clone()).collect());
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
/// card's flat arguments as the real tool takes them, without pulling in
/// the vak-core dependency.
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
        let envelope = args.clone();
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
) -> (Agent, Requests) {
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

    let requests = Requests::default();
    let agent = Agent::new(
        Arc::new(Scripted {
            capacity_key: crate::support::capacity_key(),
            responses: Mutex::new(VecDeque::from(responses)),
            requests: requests.clone(),
        }),
        log,
        {
            let mut cfg = AgentConfig::new("sys");
            cfg.model = "test-model".into();
            cfg.mode = vak_permission::Mode::FullAccess;
            cfg.permission = Some(Arc::new(PermissionEngine::default()));
            cfg.tools = vec![Arc::new(FakeEmitChartCard)];
            // As in production: a card tool the request did not predict is
            // deferred, so it is not declared until something loads it.
            cfg.tool_definitions = Some(vec![
                vak_llm::ToolDefinition::new(
                    "emit_chart_card",
                    "test stand-in",
                    serde_json::json!({"type": "object"}),
                )
                .deferred(),
            ]);
            if with_check {
                cfg.presentation_check = Some(Arc::new(|text, offered| {
                    (text.contains("TABLE") && offered.iter().any(|t| t == "emit_chart_card")).then(
                        || vak_agent::PresentationNudge {
                            tool: "emit_chart_card".into(),
                            text:
                                "[presentation-check]: this reads as a card; call emit_chart_card."
                                    .into(),
                        },
                    )
                }));
            }
            cfg
        },
    );
    (agent, requests)
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
const CHART_INPUT: fn() -> serde_json::Value =
    || serde_json::json!({"semantic_type": "chart", "chart_type": "line", "points": []});

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
    let (mut agent, requests) = build_agent(
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
    let requests = requests.lock().unwrap();
    assert!(
        !requests[0].contains(&"emit_chart_card".to_string()),
        "the deferred card tool is not declared before the nudge"
    );
    assert!(
        requests[1].contains(&"emit_chart_card".to_string()),
        "the nudge loads the card tool it asks for, on a non-Anthropic leg"
    );
}

#[tokio::test]
async fn the_nudge_is_one_shot_and_the_model_may_decline_by_resending() {
    let dir = tempdir().unwrap();
    let (mut agent, _) = build_agent(
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
    let (mut agent, _) = build_agent(
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
        "{PROSE}\n\n```vak\n{{\"semantic_type\":\"metric\",\"label\":\"x\",\"value\":\"1\"}}\n```"
    );
    let (mut agent, _) = build_agent(&dir, "pc-fence", vec![text_msg(&with_fence)], true).await;
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
    let (mut agent, _) = build_agent(&dir, "pc-off", vec![text_msg(PROSE)], false).await;
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

/// Found live: a model shown a card tool only by name called it with
/// arguments it had guessed, three times. Calling a deferred tool loads it,
/// so the request after the call declares its schema.
#[tokio::test]
async fn calling_a_deferred_tool_loads_its_schema_for_the_next_step() {
    let dir = tempdir().unwrap();
    let (mut agent, requests) = build_agent(
        &dir,
        "pc-direct",
        vec![
            tool_call_msg("c1", "emit_chart_card", CHART_INPUT()),
            text_msg(NARRATION),
        ],
        false,
    )
    .await;
    assert!(matches!(
        run(&mut agent).await,
        TurnOutcome::Completed { .. }
    ));
    let requests = requests.lock().unwrap();
    assert!(!requests[0].contains(&"emit_chart_card".to_string()));
    assert!(
        requests[1].contains(&"emit_chart_card".to_string()),
        "{requests:?}"
    );
}
