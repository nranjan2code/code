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
    capacity_key: String,
    responses: Mutex<VecDeque<AssistantMessage>>,
}

/// Resolves `$c1`/`$c2` in queued tool arguments from the acknowledgement
/// the Agent actually returned for that earlier card call.
struct RequestScripted {
    capacity_key: String,
    responses: Mutex<VecDeque<AssistantMessage>>,
}

fn presentation_id(request: &ChatRequest, call_id: &str) -> Option<String> {
    request
        .messages
        .iter()
        .flat_map(|message| &message.content)
        .find_map(|block| {
            let ContentBlock::ToolResult {
                tool_use_id,
                content,
                ..
            } = block
            else {
                return None;
            };
            if tool_use_id != call_id {
                return None;
            }
            let (_, tail) = content.rsplit_once("(presentation ")?;
            Some(tail.trim_end_matches(')').to_string())
        })
}

fn replace_presentation_placeholders(value: &mut serde_json::Value, request: &ChatRequest) {
    match value {
        serde_json::Value::String(text) => {
            for key in ["c1", "c2"] {
                if text.as_str() == format!("${key}")
                    && let Some(id) = presentation_id(request, key)
                {
                    *text = id;
                }
            }
        }
        serde_json::Value::Array(items) => {
            for item in items {
                replace_presentation_placeholders(item, request);
            }
        }
        serde_json::Value::Object(fields) => {
            for item in fields.values_mut() {
                replace_presentation_placeholders(item, request);
            }
        }
        _ => {}
    }
}

#[async_trait]
impl Provider for RequestScripted {
    fn name(&self) -> &str {
        "request-scripted"
    }
    fn rate_limit_key(&self) -> String {
        self.capacity_key.clone()
    }
    async fn stream(
        &self,
        request: ChatRequest,
        _cancel: CancellationToken,
    ) -> Result<EventStream, LlmError> {
        let mut next = self.responses.lock().unwrap().pop_front();
        if let Some(message) = &mut next {
            for block in &mut message.content {
                if let ContentBlock::ToolUse { input, .. } = block {
                    replace_presentation_placeholders(input, &request);
                }
            }
        }
        let (mut sink, rx) = stream::channel(64);
        match next {
            Some(message) => {
                sink.push(stream::StreamEvent::Start {
                    partial: message.clone(),
                });
                sink.close_message(message).await;
            }
            None => sink.close_error(LlmError::Parse("exhausted".into())).await,
        }
        Ok(rx)
    }
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
        applied: None,
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
        applied: None,
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
        serde_json::json!({"type": "object", "properties": {
            "semantic_type": {"type": "string", "enum": ["chart", "controls", "stats"]},
            "title": {"type": "string"},
            "points": {"type": "array"}
        }})
    }
    fn presents_cards(&self) -> bool {
        true
    }
    fn followable_fields(&self) -> Vec<Vec<String>> {
        vec![vec!["readings".into(), "*".into(), "value".into()]]
    }
    async fn execute(&self, args: &serde_json::Value, _ctx: &ToolContext) -> ToolOutput {
        let envelope = args.clone();
        ToolOutput::ok(envelope.to_string())
    }
}

struct KeepGoing;
#[async_trait]
impl Tool for KeepGoing {
    fn name(&self) -> &str {
        "keep_going"
    }
    fn description(&self) -> &str {
        "test turn continuation"
    }
    fn schema(&self) -> serde_json::Value {
        serde_json::json!({"type":"object"})
    }
    async fn execute(&self, _args: &serde_json::Value, _ctx: &ToolContext) -> ToolOutput {
        ToolOutput::ok("continued")
    }
}

/// Stand-in for `vak_core::presentation_tools::presentation_info` /
/// `emit_tool_for`, minimal enough to exercise the fence path without
/// pulling in vak-core (vak-agent has no dependency on it).
fn fake_rebuild() -> vak_agent::PresentationRebuild {
    Arc::new(|_name, input| {
        let semantic_type = input.get("semantic_type")?.as_str()?.to_string();
        let mut card = input.as_object()?.clone();
        card.remove("semantic_type");
        let payload = vak_session::types::canonicalize_json(&serde_json::Value::Object(card));
        let title = payload
            .get("title")
            .and_then(|v| v.as_str())
            .unwrap_or("chart")
            .to_string();
        Some(PresentationCard {
            presentation_id: None,
            block: None,
            data_call_id: None,
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

fn fake_controls_rebuild() -> vak_agent::PresentationRebuild {
    Arc::new(|_name, input| {
        let semantic_type = input.get("semantic_type")?.as_str()?.to_string();
        let mut card = input.as_object()?.clone();
        card.remove("semantic_type");
        let payload = vak_session::types::canonicalize_json(&serde_json::Value::Object(card));
        Some(PresentationCard {
            presentation_id: None,
            block: (semantic_type == "controls").then(|| "controls".into()),
            data_call_id: None,
            title: payload["title"].as_str().unwrap_or("card").to_string(),
            semantic_type,
            skill_id: "test".into(),
            skill_version: "1".into(),
            schema_version: 1,
            payload,
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
    build_agent_with_provider(
        dir,
        session_id,
        tools,
        with_presentation_rebuild,
        Arc::new(Scripted {
            capacity_key: crate::support::capacity_key(),
            responses: Mutex::new(VecDeque::from(responses)),
        }),
    )
}

fn build_agent_with_provider(
    dir: &tempfile::TempDir,
    session_id: &str,
    tools: Vec<Arc<dyn Tool>>,
    with_presentation_rebuild: bool,
    provider: Arc<dyn Provider>,
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
    let mut cfg = AgentConfig::new("sys");
    cfg.model = "test-model".into();
    cfg.mode = vak_permission::Mode::FullAccess;
    cfg.permission = Some(Arc::new(PermissionEngine::default()));
    cfg.approver = Some(Arc::new(vak_agent::AutoApprove));
    cfg.tools = tools;
    if with_presentation_rebuild {
        cfg.presentation_rebuild = Some(fake_rebuild());
    }
    Agent::new(provider, log, cfg)
}

fn calls(items: Vec<(&str, &str, serde_json::Value)>) -> AssistantMessage {
    let mut message = tool_call_msg("unused", "keep_going", serde_json::json!({}));
    message.content = items
        .into_iter()
        .map(|(id, name, input)| ContentBlock::ToolUse {
            id: id.into(),
            name: name.into(),
            input,
        })
        .collect();
    message
}

fn card_controls(value: i64) -> serde_json::Value {
    serde_json::json!({"semantic_type":"controls", "title":"Inputs",
        "inputs":[{"label":"Base", "name":"base", "kind":"number", "value":value}],
        "outputs":[]})
}

fn make_dynamic_agent(
    dir: &tempfile::TempDir,
    session_id: &str,
    responses: Vec<AssistantMessage>,
) -> Agent {
    let mut agent = build_agent_with_provider(
        dir,
        session_id,
        vec![Arc::new(FakeEmitChartCard), Arc::new(KeepGoing)],
        true,
        Arc::new(RequestScripted {
            capacity_key: crate::support::capacity_key(),
            responses: Mutex::new(VecDeque::from(responses)),
        }),
    );
    agent.config.presentation_rebuild = Some(fake_controls_rebuild());
    agent.config.run_retry_attempts = 0;
    agent.config.max_retries = 0;
    agent
}

#[tokio::test]
async fn replacing_an_unused_same_name_control_succeeds_and_later_binding_uses_new_id() {
    let dir = tempdir().unwrap();
    let mut agent = make_dynamic_agent(
        &dir,
        "replace-unused-control",
        vec![
            calls(vec![
                ("c1", "emit_chart_card", card_controls(4)),
                ("n1", "keep_going", serde_json::json!({})),
            ]),
            calls(vec![
                ("c2", "emit_chart_card", {
                    let mut input = card_controls(9);
                    input["replaces"] = serde_json::json!("$c1");
                    input
                }),
                ("n2", "keep_going", serde_json::json!({})),
            ]),
            calls(vec![
                ("c3", "emit_chart_card", {
                    let mut input = card_controls(11);
                    input["replaces"] = serde_json::json!("$c1");
                    input
                }),
                ("n3", "keep_going", serde_json::json!({})),
            ]),
            calls(vec![
                (
                    "s1",
                    "emit_chart_card",
                    serde_json::json!({"semantic_type":"stats", "title":"Totals", "readings":[{"label":"Base","value":"= base"}]}),
                ),
                ("n4", "keep_going", serde_json::json!({})),
            ]),
            text_msg("Done."),
        ],
    );
    let outcome = agent
        .run(
            "replace the input and show its total",
            &Default::default(),
            CancellationToken::new(),
            mpsc::channel(64).0,
        )
        .await;
    assert!(
        matches!(outcome, TurnOutcome::Completed { .. }),
        "{outcome:?}"
    );
    let session = agent.session.lock().await;
    let presentations = session.presentations();
    assert_eq!(
        presentations.len(),
        3,
        "{presentations:?}; messages={:?}",
        session.derive_messages()
    );
    let controls = presentations
        .iter()
        .filter(|(_, record)| record.block.as_deref() == Some("controls"))
        .collect::<Vec<_>>();
    assert_eq!(controls.len(), 2);
    let active = session.active_presentations();
    assert_eq!(active.len(), 2, "{active:?}");
    assert_eq!(active[0].1.payload["inputs"][0]["value"], 9);
    assert!(active.iter().all(|(id, _)| id != &controls[0].0));
    let new_id = controls[1].0.clone();
    let stats = presentations
        .iter()
        .find(|(_, record)| record.semantic_type == "stats")
        .unwrap();
    assert_eq!(stats.1.payload["readings"][0]["value"], 9);
    assert_eq!(stats.1.payload["bound"][0]["from"][0], new_id);
    let stale_refusal = session
        .derive_messages()
        .iter()
        .flat_map(|message| &message.content)
        .find_map(|block| match block {
            ContentBlock::ToolResult {
                tool_use_id,
                content,
                is_error: true,
            } if tool_use_id == "c3" => Some(content.clone()),
            _ => None,
        })
        .expect("replacing the retired control ID should be refused");
    assert!(
        stale_refusal.contains("no card in this answer"),
        "{stale_refusal}"
    );
}

#[tokio::test]
async fn replacing_a_control_used_by_an_active_card_returns_typed_refusal() {
    let dir = tempdir().unwrap();
    let mut agent = make_dynamic_agent(
        &dir,
        "replace-bound-control",
        vec![
            calls(vec![
                ("c1", "emit_chart_card", card_controls(4)),
                ("n1", "keep_going", serde_json::json!({})),
            ]),
            calls(vec![
                (
                    "s1",
                    "emit_chart_card",
                    serde_json::json!({"semantic_type":"stats", "title":"Totals", "readings":[{"label":"Base","value":"= base"}]}),
                ),
                ("n2", "keep_going", serde_json::json!({})),
            ]),
            calls(vec![
                ("c2", "emit_chart_card", {
                    let mut input = card_controls(9);
                    input["replaces"] = serde_json::json!("$c1");
                    input
                }),
                ("n3", "keep_going", serde_json::json!({})),
            ]),
            text_msg("Done."),
        ],
    );
    let outcome = agent
        .run(
            "show an input and a stat, then replace it",
            &Default::default(),
            CancellationToken::new(),
            mpsc::channel(64).0,
        )
        .await;
    assert!(
        matches!(outcome, TurnOutcome::Completed { .. }),
        "{outcome:?}"
    );
    let session = agent.session.lock().await;
    let c1 = session
        .presentations()
        .into_iter()
        .find_map(|(id, record)| match &record.source {
            PresentationSource::ToolCall { tool_use_id } if tool_use_id == "c1" => Some(id),
            _ => None,
        })
        .expect("c1 presentation");
    let refusal = session
        .derive_messages()
        .iter()
        .flat_map(|message| &message.content)
        .find_map(|block| match block {
            ContentBlock::ToolResult {
                tool_use_id,
                content,
                is_error: true,
            } if tool_use_id == "c2" => Some(content.clone()),
            _ => None,
        })
        .expect("replacement should be refused");
    let error: serde_json::Value = serde_json::from_str(&refusal).expect("typed JSON refusal");
    assert_eq!(error["type"], "card_source_in_use", "{error}");
    assert_eq!(error["presentation_id"], c1);
    assert_eq!(
        error["dependent_presentation_ids"]
            .as_array()
            .unwrap()
            .len(),
        1,
        "{error}"
    );
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
    let fence = "Here's a fresh chart.\n\n```vak\n{\"semantic_type\":\"chart\",\"title\":\"Fresh\",\"points\":[{\"series\":\"s\",\"x\":\"1\",\"y\":1}]}\n```";
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
    // The reply repeats the card it showed as a `vak` fence with the
    // IDENTICAL payload: the card is recorded once.
    let duplicate_fence = "Here's the chart you asked for.\n\n```vak\n{\"semantic_type\":\"chart\",\"title\":\"Sales\",\"points\":[{\"series\":\"s\",\"x\":\"1\",\"y\":1}]}\n```";
    let mut agent = build_agent(
        &dir,
        "fence-dedup",
        vec![
            tool_call_msg(
                "call_1",
                "emit_chart_card",
                serde_json::json!({"semantic_type": "chart", "title": "Sales", "points": [{"series": "s", "x": "1", "y": 1}]}),
            ),
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
        &presentations[0].1.source,
        PresentationSource::ToolCall { tool_use_id } if tool_use_id == "call_1"
    ));
}

/// Found live 2026-10-14: an answer's cards came one per model step until
/// the prompt asked for ready cards together. Several card calls in one
/// step are each recorded, in the order the step made them, which is the
/// order the projection reads them in.
#[tokio::test]
async fn several_cards_in_one_step_are_recorded_in_call_order() {
    let dir = tempdir().unwrap();
    let card = |id: &str, title: &str| ContentBlock::ToolUse {
        id: id.into(),
        name: "emit_chart_card".into(),
        input: serde_json::json!({"semantic_type": "chart", "title": title, "points": [{"series": "s", "x": "1", "y": 1}]}),
    };
    let mut step = tool_call_msg("unused", "emit_chart_card", serde_json::json!({}));
    step.content = vec![
        card("c1", "Payment"),
        card("c2", "Schedule"),
        card("c3", "Balance"),
    ];
    let mut agent = build_agent(
        &dir,
        "cards-one-step",
        vec![step, text_msg("Above.")],
        vec![Arc::new(FakeEmitChartCard)],
        true,
    );
    let outcome = agent
        .run(
            "the loan in three cards",
            &Default::default(),
            CancellationToken::new(),
            mpsc::channel(64).0,
        )
        .await;
    assert!(
        matches!(outcome, TurnOutcome::Completed { .. }),
        "{outcome:?}"
    );

    let session = agent.session.lock().await;
    let recorded: Vec<(String, String)> = session
        .presentations()
        .iter()
        .map(|(_, record)| match &record.source {
            PresentationSource::ToolCall { tool_use_id } => {
                (tool_use_id.clone(), record.title.clone())
            }
            other => panic!("not a card call: {other:?}"),
        })
        .collect();
    assert_eq!(
        recorded,
        [
            ("c1".to_string(), "Payment".to_string()),
            ("c2".to_string(), "Schedule".to_string()),
            ("c3".to_string(), "Balance".to_string()),
        ]
    );
}

/// A card's `replaces` names a card of this answer by the presentation id
/// its acknowledgement gave; any other id is refused with the answer's
/// card ids, and nothing is shown for it.
#[tokio::test]
async fn a_card_replacing_no_card_of_this_answer_is_refused_with_its_ids() {
    let dir = tempdir().unwrap();
    let chart = |title: &str, replaces: Option<&str>| {
        let mut input = serde_json::json!({"semantic_type": "chart", "title": title, "points": [{"series": "s", "x": "1", "y": 1}]});
        if let Some(id) = replaces {
            input["replaces"] = serde_json::json!(id);
        }
        input
    };
    let mut agent = build_agent(
        &dir,
        "replaces-unknown",
        vec![
            // One step: c1 is shown before c2 is checked, so c2's refusal
            // names it (a step of accepted cards alone ends the turn).
            {
                let mut step = tool_call_msg("c1", "emit_chart_card", chart("First", None));
                step.content.push(ContentBlock::ToolUse {
                    id: "c2".into(),
                    name: "emit_chart_card".into(),
                    input: chart("Second", Some("not-a-card")),
                });
                step
            },
            text_msg("Above."),
        ],
        vec![Arc::new(FakeEmitChartCard)],
        true,
    );
    let outcome = agent
        .run(
            "a chart",
            &Default::default(),
            CancellationToken::new(),
            mpsc::channel(64).0,
        )
        .await;
    assert!(
        matches!(outcome, TurnOutcome::Completed { .. }),
        "{outcome:?}"
    );
    let session = agent.session.lock().await;
    let presentations = session.presentations();
    assert_eq!(presentations.len(), 1, "{presentations:?}");
    let shown = presentations[0].0.clone();
    let refusal = session
        .derive_messages()
        .iter()
        .flat_map(|message| message.content.clone())
        .find_map(|block| match block {
            ContentBlock::ToolResult {
                tool_use_id,
                content,
                is_error: true,
            } if tool_use_id == "c2" => Some(content),
            _ => None,
        })
        .expect("c2 refused");
    assert!(
        refusal.contains("`not-a-card`") && refusal.contains(&format!("`{shown}`")),
        "{refusal}"
    );
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
            tool_call_msg(
                "t2",
                "recall",
                serde_json::json!({
                    "open": "ev:t1", "query": "",
                    "lines": {"start": 0, "end": 0}
                }),
            ),
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

#[tokio::test]
async fn recall_query_discovers_old_work_and_logs_its_bounded_result() {
    let dir = tempdir().unwrap();
    let mut agent = build_agent(
        &dir,
        "recall-search",
        vec![
            text_msg("Noida weather observation from last month"),
            tool_call_msg(
                "search-old",
                "recall",
                serde_json::json!({"query": "Noida weather", "limit": 1}),
            ),
            text_msg("Found the older observation; current weather still needs a fresh check."),
        ],
        vec![],
        false,
    );
    for prompt in [
        "record Noida weather",
        "find my earlier weather observation",
    ] {
        let outcome = agent
            .run(
                prompt,
                &Default::default(),
                CancellationToken::new(),
                mpsc::channel(64).0,
            )
            .await;
        assert!(
            matches!(outcome, TurnOutcome::Completed { .. }),
            "{outcome:?}"
        );
    }
    let session = agent.session.lock().await;
    let result = session
        .message_chain()
        .iter()
        .flat_map(|(_, m)| &m.content)
        .find_map(|block| match block {
            ContentBlock::ToolResult {
                tool_use_id,
                content,
                is_error,
            } if tool_use_id == "search-old" => {
                assert!(!is_error);
                Some(serde_json::from_str::<serde_json::Value>(content).unwrap())
            }
            _ => None,
        })
        .unwrap();
    let matches = result["this_conversation"].as_array().unwrap();
    assert_eq!(matches.len(), 1);
    assert!(matches[0]["open"].as_str().unwrap().starts_with("turn:"));
    assert!(matches[0]["record"].as_str().unwrap().contains("Noida"));
    assert!(session.derive_messages().iter().flat_map(|m| &m.content).any(|block|
        matches!(block, ContentBlock::ToolResult { tool_use_id, .. } if tool_use_id == "search-old")));
}

#[tokio::test]
async fn host_history_lookup_is_logged_without_in_memory_fallback() {
    let dir = tempdir().unwrap();
    let mut agent = build_agent(
        &dir,
        "host-recall",
        vec![
            tool_call_msg(
                "host-history",
                "recall",
                serde_json::json!({"query":"old budget"}),
            ),
            text_msg("History is warming; could you share the missing budget detail?"),
        ],
        vec![],
        false,
    );
    agent.config.run_retry_attempts = 0;
    agent.config.max_retries = 0;
    let called = Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let count = called.clone();
    agent.config.history_recall = Some(Arc::new(move |request, leaf, _cancel| {
        assert!(matches!(request, vak_tools::RecallRequest::Search { .. }));
        assert!(!leaf.is_empty());
        count.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        Box::pin(async {
            Err(
                serde_json::json!({"type":"history_unavailable", "message":"History is warming"})
                    .to_string(),
            )
        })
    }));
    let outcome = agent
        .run(
            "find the old budget",
            &Default::default(),
            Default::default(),
            mpsc::channel(64).0,
        )
        .await;
    assert!(
        matches!(outcome, TurnOutcome::Completed { .. }),
        "{outcome:?}"
    );
    assert_eq!(called.load(std::sync::atomic::Ordering::Relaxed), 1);
    let session = agent.session.lock().await;
    assert!(session.derive_messages().iter().flat_map(|m| &m.content).any(|block|
        matches!(block, ContentBlock::ToolResult { tool_use_id, content, is_error: true }
            if tool_use_id == "host-history" && content.contains("history_unavailable"))));
}

#[tokio::test]
async fn a_card_the_host_cannot_record_is_never_reported_shown() {
    let dir = tempdir().unwrap();
    // The fake rebuild needs `semantic_type`; this call has none, so the
    // tool accepts it and the host cannot record it.
    let mut agent = build_agent(
        &dir,
        "card-unrecorded",
        vec![
            tool_call_msg(
                "call_1",
                "emit_chart_card",
                serde_json::json!({"title": "Sales", "points": []}),
            ),
            text_msg("done"),
        ],
        vec![Arc::new(FakeEmitChartCard)],
        true,
    );
    agent
        .run(
            "show me the sales chart",
            &Default::default(),
            CancellationToken::new(),
            mpsc::channel(64).0,
        )
        .await;
    let session = agent.session.lock().await;
    assert!(session.presentations().is_empty());
    assert!(session.derive_messages().iter().flat_map(|m| &m.content).any(|block|
        matches!(block, ContentBlock::ToolResult { tool_use_id, content, is_error: true }
            if tool_use_id == "call_1" && content.contains("not recorded"))));
}

/// Found live 2026-10-16: searching for a fact, a model also sent the turn
/// number it could see in `open`; both are answered, so the search runs.
#[tokio::test]
async fn open_and_query_together_are_both_answered() {
    let dir = tempdir().unwrap();
    let mut agent = build_agent(
        &dir,
        "recall-both",
        vec![
            text_msg("Noted: the cat is called Miso."),
            tool_call_msg(
                "both",
                "recall",
                serde_json::json!({"open": "#1", "query": "cat called"}),
            ),
            text_msg("Your cat is called Miso."),
        ],
        vec![],
        false,
    );
    for prompt in ["my cat is called Miso", "what is my cat called?"] {
        let outcome = agent
            .run(
                prompt,
                &Default::default(),
                CancellationToken::new(),
                mpsc::channel(64).0,
            )
            .await;
        assert!(
            matches!(outcome, TurnOutcome::Completed { .. }),
            "{outcome:?}"
        );
    }
    let session = agent.session.lock().await;
    let result: serde_json::Value = session
        .message_chain()
        .iter()
        .flat_map(|(_, m)| &m.content)
        .find_map(|block| match block {
            ContentBlock::ToolResult {
                tool_use_id,
                content,
                is_error,
            } if tool_use_id == "both" => {
                assert!(!is_error, "{content}");
                Some(serde_json::from_str(content).unwrap())
            }
            _ => None,
        })
        .unwrap();
    assert!(result["opened"]["messages"].is_array(), "{result}");
    let found = result["found"]["this_conversation"].as_array().unwrap();
    assert!(
        found[0]["record"].as_str().unwrap().contains("Miso"),
        "{result}"
    );
}

/// A turn a person set aside is not reopened or found by `recall`, nor is
/// what it received; once restored it is again
/// (docs/design/99-turns-that-do-not-poison.md §4 F).
#[tokio::test]
async fn recall_skips_a_turn_set_aside_until_it_is_restored() {
    let dir = tempdir().unwrap();
    let mut agent = build_agent(
        &dir,
        "recall-aside",
        vec![
            tool_call_msg(
                "t1",
                "bash",
                serde_json::json!({"command": "printf 'HELLO WORLD'"}),
            ),
            text_msg("Printed HELLO WORLD."),
            tool_call_msg(
                "r1",
                "recall",
                serde_json::json!({"open": "ev:t1", "query": "", "lines": {"start": 0, "end": 0}}),
            ),
            tool_call_msg(
                "r2",
                "recall",
                serde_json::json!({"open": "#1", "query": ""}),
            ),
            tool_call_msg(
                "r3",
                "recall",
                serde_json::json!({"query": "HELLO WORLD", "limit": 5}),
            ),
            text_msg("Nothing earlier."),
            tool_call_msg(
                "r4",
                "recall",
                serde_json::json!({"open": "ev:t1", "query": "", "lines": {"start": 0, "end": 0}}),
            ),
            tool_call_msg(
                "r5",
                "recall",
                serde_json::json!({"query": "HELLO WORLD", "limit": 5}),
            ),
            text_msg("Found it."),
        ],
        vec![Arc::new(BashTool)],
        false,
    );
    let mark = |agent: &Agent, aside: bool| {
        let session = agent.session.clone();
        async move {
            let mut session = session.lock().await;
            let turn = vak_session::TurnIndex::from_log(&session).turns[0]
                .id
                .clone();
            session
                .append_turn_mark(
                    aside,
                    vak_session::turn_marks::TurnMarkRecord {
                        turn,
                        trace: None,
                        actor: None,
                    },
                )
                .unwrap();
        }
    };
    for (prompt, aside) in [
        ("print hello", Some(true)),
        ("what did we print before?", Some(false)),
        ("and now?", None),
    ] {
        let outcome = agent
            .run(
                prompt,
                &Default::default(),
                CancellationToken::new(),
                mpsc::channel(64).0,
            )
            .await;
        assert!(
            matches!(outcome, TurnOutcome::Completed { .. }),
            "{outcome:?}"
        );
        if let Some(aside) = aside {
            mark(&agent, aside).await;
        }
    }

    let session = agent.session.lock().await;
    let result = |id: &str| -> String {
        session
            .message_chain()
            .iter()
            .flat_map(|(_, m)| m.content.clone())
            .find_map(|b| match b {
                ContentBlock::ToolResult {
                    tool_use_id,
                    content,
                    ..
                } if tool_use_id == id => Some(content),
                _ => None,
            })
            .unwrap_or_else(|| panic!("no tool result for {id}"))
    };
    assert!(result("r1").contains("set_aside"), "{}", result("r1"));
    assert!(result("r2").contains("set_aside"), "{}", result("r2"));
    assert!(!result("r3").contains("HELLO"), "{}", result("r3"));
    assert!(result("r4").contains("HELLO WORLD"), "{}", result("r4"));
    assert!(result("r5").contains("HELLO"), "{}", result("r5"));
}
