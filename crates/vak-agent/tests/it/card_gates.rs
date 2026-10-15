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
    async fn execute(&self, _args: &serde_json::Value, _ctx: &ToolContext) -> ToolOutput {
        ToolOutput::error(
            Fault::Tool,
            "[webfetch] GET https://example.com -> 403 Forbidden",
        )
    }
}

/// A shell that ran a command and stopped at its limit, as a dev server
/// does. Named `shell`, it is called as itself: no name stands for another
/// tool.
struct FakeShell;

#[async_trait::async_trait]
impl Tool for FakeShell {
    fn name(&self) -> &str {
        "shell"
    }
    fn description(&self) -> &str {
        "run a command"
    }
    fn schema(&self) -> serde_json::Value {
        serde_json::json!({"type": "object"})
    }
    fn permission(&self) -> vak_permission::Declared {
        vak_permission::Declared::new(
            vak_permission::Reach::Command,
            vak_permission::Subject::Command("command"),
        )
    }
    async fn execute(&self, args: &serde_json::Value, _ctx: &ToolContext) -> ToolOutput {
        // One refused before anything ran, as a missing folder is.
        if args["command"] == "ls missing" {
            return ToolOutput::error(Fault::Target, "working directory does not exist");
        }
        ToolOutput::error(
            Fault::Limit,
            "starting\n[command stopped at its time limit of 1000ms]\n[limit]: a note for the model",
        )
        .ran(None, 1000)
    }
}

/// A card that shows what a command printed, as the terminal card does.
struct FakeTerminalCard;

#[async_trait::async_trait]
impl Tool for FakeTerminalCard {
    fn name(&self) -> &str {
        "emit_terminal_card"
    }
    fn description(&self) -> &str {
        "show a command's output"
    }
    fn schema(&self) -> serde_json::Value {
        serde_json::json!({"type": "object"})
    }
    fn presents_cards(&self) -> bool {
        true
    }
    fn output_of_command(&self) -> Option<vak_tools::CommandOutput> {
        Some(vak_tools::CommandOutput {
            command: "command",
            output: Some("output"),
            exit_code: "exit_code",
            duration_ms: "duration_ms",
        })
    }
    async fn execute(&self, args: &serde_json::Value, _ctx: &ToolContext) -> ToolOutput {
        ToolOutput::ok(format!(
            "shown: {} exit={} ms={}",
            args["output"].as_str().unwrap_or(""),
            args["exit_code"],
            args["duration_ms"]
        ))
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
    async fn execute(&self, _args: &serde_json::Value, _ctx: &ToolContext) -> ToolOutput {
        let rows: Vec<serde_json::Value> = (0..300)
            .map(|at| serde_json::json!({"city": format!("City {at}"), "people": at * 7}))
            .collect();
        ToolOutput::ok(serde_json::json!({"rows": rows}).to_string())
    }
}

/// A table card filled from an ability's answer: it reports the rows the
/// loop gave it.
struct FakeAbilityTable;

#[async_trait::async_trait]
impl Tool for FakeAbilityTable {
    fn name(&self) -> &str {
        "emit_ability_table_card"
    }
    fn description(&self) -> &str {
        "a table of an ability's answer"
    }
    fn schema(&self) -> serde_json::Value {
        serde_json::json!({"type": "object"})
    }
    fn presents_cards(&self) -> bool {
        true
    }
    fn data_of_call(&self) -> Option<vak_tools::CallData> {
        Some(vak_tools::CallData {
            from: "from",
            list: "list",
            filled: &["rows"],
        })
    }
    fn fill_from_data(
        &self,
        args: &mut serde_json::Value,
        records: &[serde_json::Value],
    ) -> Result<(), String> {
        args["rows"] = serde_json::Value::Array(records.to_vec());
        Ok(())
    }
    async fn execute(&self, args: &serde_json::Value, _ctx: &ToolContext) -> ToolOutput {
        ToolOutput::ok(args["rows"].to_string())
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
                Arc::new(FakeShell),
                Arc::new(FakeTerminalCard),
                Arc::new(FakeAbility),
                Arc::new(FakeAbilityTable),
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
                vak_llm::ToolDefinition::new(
                    "shell",
                    "test stand-in",
                    serde_json::json!({"type": "object"}),
                ),
                vak_llm::ToolDefinition::new(
                    "emit_terminal_card",
                    "test stand-in",
                    serde_json::json!({"type": "object"}),
                ),
                vak_llm::ToolDefinition::new(
                    "census",
                    "test stand-in",
                    serde_json::json!({"type": "object"}),
                ),
                vak_llm::ToolDefinition::new(
                    "emit_ability_table_card",
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

/// A card showing what a command printed shows what the call that ran it
/// returned, whatever the model wrote; one naming a command no call ran is
/// refused with the command named.
#[tokio::test]
async fn a_terminal_card_shows_what_its_command_really_printed() {
    let dir = tempdir().unwrap();
    let (mut agent, _) = build_agent(
        &dir,
        "pc-terminal",
        vec![
            tool_call_msg("s1", "shell", serde_json::json!({"command": "npm start"})),
            tool_call_msg(
                "t1",
                "emit_terminal_card",
                serde_json::json!({"command": "npm start", "output": "ready"}),
            ),
            tool_call_msg(
                "t2",
                "emit_terminal_card",
                serde_json::json!({"command": "npm run dev"}),
            ),
            tool_call_msg("s2", "shell", serde_json::json!({"command": "ls missing"})),
            tool_call_msg(
                "t3",
                "emit_terminal_card",
                serde_json::json!({"command": "ls missing"}),
            ),
            text_msg("The server stopped at its limit."),
        ],
    )
    .await;
    let outcome = run(&mut agent).await;
    assert!(
        matches!(outcome, TurnOutcome::Completed { .. }),
        "{outcome:?}"
    );
    let session = agent.session.lock().await;
    let result = |id: &str| {
        session
            .message_chain()
            .iter()
            .flat_map(|(_, message)| message.content.iter())
            .find_map(|block| match block {
                ContentBlock::ToolResult {
                    tool_use_id,
                    content,
                    is_error,
                } if tool_use_id == id => Some((content.clone(), *is_error)),
                _ => None,
            })
            .unwrap()
    };
    let (shown, failed) = result("t1");
    assert!(!failed, "{shown}");
    assert!(shown.contains("stopped at its time limit"), "{shown}");
    assert!(!shown.contains("ready"), "{shown}");
    assert!(
        !shown.contains("[limit]"),
        "the runtime's note is not output: {shown}"
    );
    assert!(
        shown.contains("exit=null ms=1000"),
        "stopped, not exited: {shown}"
    );
    let (refused, failed) = result("t2");
    assert!(failed, "{refused}");
    assert!(
        refused.contains("no call in this conversation ran `npm run dev`"),
        "{refused}"
    );
    assert!(
        refused.contains("The commands that ran in this turn are `npm start`"),
        "the refusal names what did run: {refused}"
    );
    // A call refused before anything ran printed nothing.
    let (refused, failed) = result("t3");
    assert!(failed, "{refused}");
    assert!(refused.contains("ran `ls missing`"), "{refused}");
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

/// A card filled from an ability's answer shows every record the call
/// answered, whatever the model wrote; one naming an ability no call
/// answered is refused with the abilities that did (docs/design/93 R5).
#[tokio::test]
async fn an_ability_card_shows_every_record_its_call_answered() {
    let dir = tempdir().unwrap();
    let (mut agent, _) = build_agent(
        &dir,
        "pc-ability-card",
        vec![
            tool_call_msg("u0", "emit_ability_table_card", serde_json::json!({"from": "census", "list": "rows"})),
            tool_call_msg("a1", "census", serde_json::json!({})),
            tool_call_msg(
                "u1",
                "emit_ability_table_card",
                serde_json::json!({"from": "census", "list": "rows", "rows": [{"city": "Made up", "people": 1}]}),
            ),
            tool_call_msg("u2", "emit_ability_table_card", serde_json::json!({"from": "weather", "list": "rows"})),
            tool_call_msg("u3", "emit_ability_table_card", serde_json::json!({"from": "census", "list": "towns"})),
            text_msg("Here are the cities."),
        ],
    )
    .await;
    let outcome = run(&mut agent).await;
    assert!(
        matches!(outcome, TurnOutcome::Completed { .. }),
        "{outcome:?}"
    );
    let session = agent.session.lock().await;
    let result = |id: &str| {
        session
            .message_chain()
            .iter()
            .flat_map(|(_, message)| message.content.iter())
            .find_map(|block| match block {
                ContentBlock::ToolResult {
                    tool_use_id,
                    content,
                    is_error,
                } if tool_use_id == id => Some((content.clone(), *is_error)),
                _ => None,
            })
            .unwrap()
    };
    let (refused, failed) = result("u0");
    assert!(
        failed && refused.contains("No ability has answered"),
        "{refused}"
    );
    let (shown, failed) = result("u1");
    assert!(!failed, "{shown}");
    let rows: Vec<serde_json::Value> = serde_json::from_str(&shown).unwrap();
    assert_eq!(rows.len(), 300);
    assert_eq!(rows[0]["city"], "City 0");
    assert_eq!(rows[299]["people"], 299 * 7);
    let (refused, failed) = result("u2");
    assert!(failed, "{refused}");
    assert!(
        refused.contains("`weather`") && refused.contains("`census`"),
        "{refused}"
    );
    let (refused, failed) = result("u3");
    assert!(
        failed && refused.contains("`towns`") && refused.contains("`rows`"),
        "{refused}"
    );
}
