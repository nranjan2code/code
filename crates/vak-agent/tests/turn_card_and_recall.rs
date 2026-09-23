#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

//! docs/design/68-context-engine.md §10/§3: a TurnCard is written once a
//! turn closes; a fence-path Presentation is written once and deduped
//! against a tool-emitted card with the same payload; `recall({ id })`
//! returns evidence content verbatim.

use std::collections::VecDeque;
use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;

use tempfile::tempdir;

use vak_agent::{Agent, AgentConfig, TurnOutcome};
use vak_llm::stream;
use vak_llm::types::{AssistantMessage, ChatRequest, ContentBlock, StopReason, Usage};
use vak_llm::{EventStream, LlmError, Provider};
use vak_permission::PermissionEngine;
use vak_session::types::{FrozenContract, PresentationSource, SessionHeader};
use vak_session::{SessionLog, SessionPath};
use vak_tools::PresentationCard;
use vak_tools::bash::BashTool;
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
        usage: Usage::default(),
        model: "test-model".into(),
        response_id: None,
    }
}

/// Stand-in for `vak_core::presentation_tools::EmitCardTool`.
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

/// Stand-in for `vak_core::presentation_tools::presentation_info` /
/// `emit_tool_for`, minimal enough to exercise the fence path without
/// pulling in vak-core (vak-agent has no dependency on it).
fn fake_rebuild() -> vak_agent::PresentationRebuild {
    Arc::new(|_name, input| {
        let semantic_type = input.get("semantic_type")?.as_str()?.to_string();
        let payload = vak_session::types::canonicalize_json(input.get("payload")?);
        let title = payload
            .get("title")
            .and_then(|v| v.as_str())
            .unwrap_or("chart")
            .to_string();
        Some(PresentationCard {
            semantic_type,
            skill_id: "test".into(),
            skill_version: "1".into(),
            schema_version: 1,
            payload,
            title,
            identity_digest: "digest".into(),
        })
    })
}

fn build_agent(
    dir: &tempfile::TempDir,
    session_id: &str,
    responses: Vec<AssistantMessage>,
    tools: Vec<Arc<dyn Tool>>,
    with_presentation_rebuild: bool,
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
    let mut cfg = AgentConfig::new("sys");
    cfg.model = "test-model".into();
    cfg.mode = vak_permission::Mode::FullAccess;
    cfg.permission = Some(Arc::new(PermissionEngine::default()));
    cfg.approver = Some(Arc::new(vak_agent::AutoApprove));
    cfg.tools = tools;
    if with_presentation_rebuild {
        cfg.presentation_rebuild = Some(fake_rebuild());
    }
    Agent::new(
        Arc::new(Scripted {
            responses: Mutex::new(VecDeque::from(responses)),
        }),
        log,
        cfg,
    )
}

#[tokio::test]
async fn turn_card_is_written_at_close() {
    let dir = tempdir().unwrap();
    let mut agent = build_agent(&dir, "card-close", vec![text_msg("42")], vec![], false);
    let outcome = agent
        .run(
            "what is six times seven",
            &Default::default(),
            CancellationToken::new(),
            mpsc::channel(64).0,
        )
        .await;
    assert!(matches!(outcome, TurnOutcome::Completed { .. }));

    let session = agent.session.lock().await;
    let cards = session.turn_cards();
    assert_eq!(cards.len(), 1, "exactly one TurnCard written at close");
    assert_eq!(cards[0].1.outcome, "completed");
    assert_eq!(cards[0].1.answered.narration, "42");
    assert!(cards[0].1.answered.presentations.is_empty());
}

#[tokio::test]
async fn fence_presentation_is_written_once() {
    let dir = tempdir().unwrap();
    let fence = "Here's a fresh chart.\n\n```vak\n{\"semantic_type\":\"chart\",\"payload\":{\"title\":\"Fresh\",\"series\":[1,2,3]}}\n```";
    let mut agent = build_agent(&dir, "fence-write", vec![text_msg(fence)], vec![], true);
    let outcome = agent
        .run(
            "show me a chart",
            &Default::default(),
            CancellationToken::new(),
            mpsc::channel(64).0,
        )
        .await;
    assert!(matches!(outcome, TurnOutcome::Completed { .. }));

    let session = agent.session.lock().await;
    let presentations = session.presentations();
    assert_eq!(
        presentations.len(),
        1,
        "the fence must write one presentation"
    );
    assert!(matches!(
        presentations[0].1.source,
        PresentationSource::Fence { .. }
    ));
    assert_eq!(presentations[0].1.title, "Fresh");
}

#[tokio::test]
async fn fence_presentation_deduped_against_tool_emitted_card() {
    let dir = tempdir().unwrap();
    // The final accepted answer still repeats the just-emitted card as a
    // `vak` fence with the IDENTICAL payload (the observed real bug this
    // rule exists for) — the duplicate-card-check repair nudge fires once,
    // and the model ignores it (attempt two repeats it too), so the run
    // completes with the duplicate fence still present in the final text.
    let duplicate_fence = "Here's the chart you asked for.\n\n```vak\n{\"semantic_type\":\"chart\",\"payload\":{\"title\":\"Sales\",\"series\":[1,2,3]}}\n```";
    let mut agent = build_agent(
        &dir,
        "fence-dedup",
        vec![
            tool_call_msg(
                "call_1",
                "emit_chart_card",
                serde_json::json!({"semantic_type": "chart", "payload": {"title": "Sales", "series": [1, 2, 3]}}),
            ),
            text_msg(duplicate_fence),
            text_msg(duplicate_fence),
        ],
        vec![Arc::new(FakeEmitChartCard)],
        true,
    );
    let outcome = agent
        .run(
            "show me the sales chart",
            &Default::default(),
            CancellationToken::new(),
            mpsc::channel(64).0,
        )
        .await;
    assert!(matches!(outcome, TurnOutcome::Completed { .. }));

    let session = agent.session.lock().await;
    let presentations = session.presentations();
    assert_eq!(
        presentations.len(),
        1,
        "the duplicate fence must not add a second presentation: {presentations:?}"
    );
    assert!(matches!(
        presentations[0].1.source,
        PresentationSource::ToolCall { .. }
    ));
}

#[tokio::test]
async fn recall_by_id_returns_the_full_evidence_content() {
    let dir = tempdir().unwrap();
    let mut agent = build_agent(
        &dir,
        "recall-id",
        vec![
            tool_call_msg(
                "t1",
                "bash",
                serde_json::json!({"command": "printf 'HELLO WORLD'"}),
            ),
            tool_call_msg("t2", "recall", serde_json::json!({"id": "t1"})),
            text_msg("done"),
        ],
        vec![Arc::new(BashTool)],
        false,
    );
    let outcome = agent
        .run(
            "run the command then recall it",
            &Default::default(),
            CancellationToken::new(),
            mpsc::channel(64).0,
        )
        .await;
    assert!(matches!(outcome, TurnOutcome::Completed { .. }));

    let session = agent.session.lock().await;
    let messages = session.message_chain();
    let content_for = |id: &str| -> String {
        messages
            .iter()
            .flat_map(|(_, m)| m.content.iter())
            .find_map(|b| match b {
                ContentBlock::ToolResult {
                    tool_use_id,
                    content,
                    ..
                } if tool_use_id == id => Some(content.clone()),
                _ => None,
            })
            .unwrap_or_else(|| panic!("no tool result for {id}"))
    };
    let original = content_for("t1");
    let recalled = content_for("t2");
    assert!(original.contains("HELLO WORLD"));
    assert_eq!(
        recalled, original,
        "recall({{ id }}) must return the evidence content verbatim"
    );
}
