#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

//! Card gates: a card that cites a page nothing in the conversation
//! retrieved is refused, naming the URL, and never recorded.

mod support;

use std::collections::VecDeque;
use std::sync::Arc;
use std::sync::Mutex;
use vak_tools::Fault;

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
/// A retrieval that fails, as a fetch answering 403 now does.
struct FailingFetch;

#[async_trait::async_trait]
impl Tool for FailingFetch {
    fn name(&self) -> &str {
        "fetch"
    }
    fn description(&self) -> &str {
        "fetch a url"
    }
    fn schema(&self) -> serde_json::Value {
        serde_json::json!({"type": "object"})
    }
    async fn execute(&self, _args: &serde_json::Value, _ctx: &ToolContext) -> ToolOutput {
        ToolOutput::error(
            Fault::Tool,
            "[webfetch] GET https://example.com -> 403 Forbidden",
        )
    }
}

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
        serde_json::json!({"type": "object", "properties": {"sources": {"type": "array"}}})
    }

    fn presents_cards(&self) -> bool {
        true
    }

    fn card_kinds(&self) -> Vec<String> {
        vec!["chart".into()]
    }

    async fn execute(&self, args: &serde_json::Value, _ctx: &ToolContext) -> ToolOutput {
        let envelope = args.clone();
        ToolOutput::ok(envelope.to_string())
    }
}

struct FakeEmitTableCard;

#[async_trait]
impl Tool for FakeEmitTableCard {
    fn name(&self) -> &str {
        "emit_table_card"
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
        ToolOutput::ok(args.to_string())
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

async fn build_agent(
    dir: &tempfile::TempDir,
    session_id: &str,
    responses: Vec<AssistantMessage>,
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
            cfg.tools = vec![
                Arc::new(FakeEmitChartCard),
                Arc::new(FakeEmitTableCard),
                Arc::new(FailingFetch),
            ];
            // As in production: a card tool the request did not predict is
            // deferred, so it is not declared until something loads it.
            cfg.tool_definitions = Some(vec![
                vak_llm::ToolDefinition::new(
                    "emit_chart_card",
                    "test stand-in",
                    serde_json::json!({"type": "object"}),
                )
                .deferred(),
                vak_llm::ToolDefinition::new(
                    "emit_table_card",
                    "test stand-in",
                    serde_json::json!({"type": "object"}),
                )
                .deferred(),
                vak_llm::ToolDefinition::new(
                    "fetch",
                    "test stand-in",
                    serde_json::json!({"type": "object"}),
                ),
            ]);
            cfg
        },
    );
    (agent, requests)
}

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

/// A card citing a page nothing here retrieved is refused each time it is
/// offered, with the URLs named; no card is recorded, and the answer the
/// model then writes is the turn's answer.
#[tokio::test]
async fn a_card_citing_an_unretrieved_page_is_never_shown() {
    let dir = tempdir().unwrap();
    let cited = serde_json::json!({"sources": [{"url": "https://made-up.example/report"}]});
    let (mut agent, _) = build_agent(
        &dir,
        "pc-uncited",
        vec![
            tool_call_msg("c1", "emit_chart_card", cited.clone()),
            tool_call_msg("c2", "emit_chart_card", cited),
            text_msg("I could not open that report, so I have no figures to chart."),
        ],
    )
    .await;
    let outcome = run(&mut agent).await;
    let TurnOutcome::Completed { response } = &outcome else {
        panic!("{outcome:?}");
    };
    assert_eq!(
        response.text_content(),
        "I could not open that report, so I have no figures to chart."
    );
    let session = agent.session.lock().await;
    assert!(session.presentations().is_empty(), "no card is recorded");
    let refused = session
        .message_chain()
        .iter()
        .flat_map(|(_, message)| message.content.iter())
        .filter(|block| matches!(block, ContentBlock::ToolResult { content, is_error: true, .. } if content.contains("[uncited-source]") && content.contains("made-up.example")))
        .count();
    assert_eq!(refused, 2, "each card is refused, naming the URL");
}
