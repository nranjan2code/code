#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

//! Card gates: a card that cites a page nothing in the conversation
//! retrieved is refused, naming the URL, and never recorded.

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
    fn fetched(&self, args: &serde_json::Value) -> Option<String> {
        args["url"].as_str().map(str::to_string)
    }
    async fn execute(&self, args: &serde_json::Value, _ctx: &ToolContext) -> ToolOutput {
        if args["succeed"] == true {
            return ToolOutput::ok("Retrieved body");
        }
        ToolOutput::error(
            Fault::Tool,
            "[webfetch] GET https://example.com -> 403 Forbidden",
        )
    }
}

/// Writes a workspace file, declaring it as a file tool does.
struct FakeWriter;

#[async_trait::async_trait]
impl Tool for FakeWriter {
    fn name(&self) -> &str {
        "put"
    }
    fn description(&self) -> &str {
        "write a file"
    }
    fn schema(&self) -> serde_json::Value {
        serde_json::json!({"type": "object"})
    }
    fn file_access(&self, args: &serde_json::Value) -> Option<(vak_tools::FileAccess, String)> {
        Some((
            vak_tools::FileAccess::Write,
            args["path"].as_str()?.to_string(),
        ))
    }
    async fn execute(&self, args: &serde_json::Value, ctx: &ToolContext) -> ToolOutput {
        let path = ctx.cwd.join(args["path"].as_str().unwrap_or_default());
        std::fs::write(&path, args["content"].as_str().unwrap_or_default()).unwrap();
        ToolOutput::ok("wrote")
    }
}

/// An ability whose program answers 300 records.
struct FakeAbility;

#[async_trait::async_trait]
impl Tool for FakeAbility {
    fn name(&self) -> &str {
        "census"
    }
    fn description(&self) -> &str {
        "people per city"
    }
    fn schema(&self) -> serde_json::Value {
        serde_json::json!({"type": "object"})
    }
    fn answers_data(&self) -> bool {
        true
    }
    async fn execute(&self, args: &serde_json::Value, _ctx: &ToolContext) -> ToolOutput {
        if args.get("fail").and_then(serde_json::Value::as_bool) == Some(true) {
            return ToolOutput::error(Fault::Tool, "test ability failure");
        }
        let rows: Vec<serde_json::Value> = (0..300)
            .map(|at| serde_json::json!({"city": format!("City {at}"), "people": at * 7}))
            .collect();
        ToolOutput::ok(serde_json::json!({"rows": rows}).to_string())
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

pub(super) fn text_msg(t: &str) -> AssistantMessage {
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

pub(super) fn tool_call_msg(id: &str, name: &str, input: serde_json::Value) -> AssistantMessage {
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

pub(super) async fn build_agent(
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
            cfg.presentation_rebuild = Some(Arc::new(|form, input| {
                let form = if form == "emit_chart_card" {
                    "chart"
                } else {
                    form
                };
                if !matches!(form, "sources" | "diff" | "chart") {
                    return None;
                }
                Some(vak_tools::PresentationCard {
                    presentation_id: None,
                    semantic_type: form.into(),
                    skill_id: "test".into(),
                    skill_version: "1".into(),
                    schema_version: 1,
                    payload: input.clone(),
                    title: form.into(),
                    identity_digest: form.into(),
                    block: Some(form.into()),
                    data_call_id: None,
                })
            }));
            cfg.model = "test-model".into();
            cfg.mode = vak_permission::Mode::FullAccess;
            cfg.permission = Some(Arc::new(PermissionEngine::default()));
            cfg.tools = vec![
                Arc::new(vak_tools::ProgressTool),
                Arc::new(FakeEmitChartCard),
                Arc::new(FakeEmitTableCard),
                Arc::new(FailingFetch),
                Arc::new(FakeWriter),
                Arc::new(FakeAbility),
            ];
            // As in production: a card tool the request did not predict is
            // deferred, so it is not declared until something loads it.
            cfg.tool_definitions = Some(vec![
                vak_llm::ToolDefinition::new(
                    "progress",
                    vak_tools::ProgressTool.description(),
                    vak_tools::ProgressTool.schema(),
                ),
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
                vak_llm::ToolDefinition::new(
                    "put",
                    "test stand-in",
                    serde_json::json!({"type": "object"}),
                ),
                vak_llm::ToolDefinition::new(
                    "census",
                    "test stand-in",
                    serde_json::json!({"type": "object"}),
                ),
            ]);
            cfg
        },
    );
    (agent, requests)
}

pub(super) async fn run(agent: &mut Agent) -> TurnOutcome {
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

/// A URL the model wrote into a call's arguments is not retrieved by
/// writing it: a call that succeeded without returning it leaves it
/// uncited (docs/design/99-turns-that-do-not-poison.md §4 A).
#[tokio::test]
async fn a_url_only_in_call_arguments_is_not_citable() {
    let dir = tempdir().unwrap();
    let url = "https://typed-by-model.example/directions";
    let cited = serde_json::json!({"sources": [{"url": url}]});
    let (mut agent, _) = build_agent(
        &dir,
        "pc-arguments",
        vec![
            tool_call_msg("a1", "census", serde_json::json!({"url": url})),
            tool_call_msg("c1", "emit_chart_card", cited),
            text_msg("No source to show."),
        ],
    )
    .await;
    let outcome = run(&mut agent).await;
    assert!(
        matches!(outcome, TurnOutcome::Completed { .. }),
        "{outcome:?}"
    );
    let session = agent.session.lock().await;
    assert!(session.presentations().is_empty(), "no card is recorded");
    let refused = session
        .message_chain()
        .iter()
        .flat_map(|(_, message)| message.content.iter())
        .any(|block| matches!(block, ContentBlock::ToolResult { content, is_error: true, .. } if content.contains("[uncited-source]") && content.contains("typed-by-model.example")));
    assert!(refused, "the card is refused, naming the URL");
}

/// A call by a name Vak retired reaches no other tool: it is told what
/// replaced it and which tools exist, and the turn goes on.
#[tokio::test]
async fn a_retired_name_is_told_what_replaced_it() {
    let dir = tempdir().unwrap();
    let (mut agent, _) = build_agent(
        &dir,
        "pc-retired",
        vec![
            tool_call_msg(
                "p1",
                "python_eval",
                serde_json::json!({"code": "print(42)"}),
            ),
            text_msg("Done."),
        ],
    )
    .await;
    let outcome = run(&mut agent).await;
    assert!(
        matches!(outcome, TurnOutcome::Completed { .. }),
        "{outcome:?}"
    );
    let session = agent.session.lock().await;
    let told = session
        .message_chain()
        .iter()
        .flat_map(|(_, message)| message.content.iter())
        .find_map(|block| match block {
            ContentBlock::ToolResult {
                tool_use_id,
                content,
                is_error: true,
            } if tool_use_id == "p1" => Some(content.clone()),
            _ => None,
        })
        .expect("the call is answered");
    assert!(told.contains(r#""name":"python_eval""#), "{told}");
    assert!(told.contains(r#""use_instead":"bash"#), "{told}");
    assert!(told.contains(r#""available_tools""#), "{told}");
}

#[tokio::test]
async fn runtime_cards_use_this_turns_receipts_without_card_calls() {
    let dir = tempdir().unwrap();
    std::fs::write(dir.path().join("a.txt"), "one\ntwo\n").unwrap();
    let (mut agent, requests) = build_agent(
        &dir,
        "runtime-cards",
        vec![
            tool_call_msg(
                "failed",
                "fetch",
                serde_json::json!({"url": "https://failed.example"}),
            ),
            tool_call_msg(
                "fetched",
                "fetch",
                serde_json::json!({"url": "https://read.example", "succeed": true}),
            ),
            tool_call_msg(
                "w1",
                "put",
                serde_json::json!({"path": "a.txt", "content": "one\nTWO\n"}),
            ),
            tool_call_msg(
                "w2",
                "put",
                serde_json::json!({"path": "new.txt", "content": "new\n"}),
            ),
            text_msg("Updated the files. The other source could not be retrieved."),
            text_msg("A fresh answer."),
        ],
    )
    .await;
    let outcome = run(&mut agent).await;
    assert!(
        matches!(outcome, TurnOutcome::Completed { .. }),
        "{outcome:?}"
    );
    assert_eq!(
        requests.lock().unwrap().len(),
        5,
        "no model call for runtime cards"
    );
    {
        let session = agent.session.lock().await;
        let cards = session.presentations();
        assert_eq!(cards.len(), 2);
        let sources = cards
            .iter()
            .find(|(_, card)| card.semantic_type == "sources")
            .unwrap()
            .1;
        assert_eq!(
            sources.source,
            vak_session::types::PresentationSource::Derived {
                from: vec!["fetched".into()]
            }
        );
        assert_eq!(sources.payload["sources"].as_array().unwrap().len(), 1);
        assert_eq!(sources.payload["sources"][0]["url"], "https://read.example");
        let diff = cards
            .iter()
            .find(|(_, card)| card.semantic_type == "diff")
            .unwrap()
            .1;
        assert_eq!(diff.derived_from, ["w1", "w2"]);
        let files = diff.payload["files"].as_array().unwrap();
        assert_eq!(files.len(), 2);
        let changed = files
            .iter()
            .find(|file| file["filename"] == "a.txt")
            .unwrap();
        assert!(changed["hunks"].as_str().unwrap().contains("-two"));
        assert!(changed["hunks"].as_str().unwrap().contains("+TWO"));
        assert_eq!(changed["additions"], 1);
        assert_eq!(changed["deletions"], 1);
        let chain = session.chain_to_root();
        let answer = chain.iter().position(|entry| matches!(&entry.payload, vak_session::EntryPayload::Message(record) if record.message.text_content().starts_with("Updated the files."))).unwrap();
        for (id, _) in cards {
            assert!(chain.iter().position(|entry| entry.id == id).unwrap() > answer);
        }
    }
    assert!(matches!(
        run(&mut agent).await,
        TurnOutcome::Completed { .. }
    ));
    let session = agent.session.lock().await;
    assert_eq!(
        session.presentations().len(),
        2,
        "no sources or diffs carried into the next turn"
    );
}
