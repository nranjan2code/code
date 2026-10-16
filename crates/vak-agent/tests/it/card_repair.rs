#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

//! The repair ladder for a card tool (docs/design/15-reliability.md,
//! docs/design/87-composed-answers.md §2). Seen live on 2026-10-11: a list
//! argument arrived as text that was not JSON, and the refusal gave a
//! column in a 2,700-character line. Here the refusal says where the text
//! breaks, the tool's schema is shown, then it is withdrawn, a step that
//! calls something else does not start the ladder again, and a planned
//! card that can no longer be shown is checked as text.

use std::collections::VecDeque;
use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use serde_json::{Value, json};
use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;

use vak_agent::{Agent, AgentConfig, TurnOutcome};
use vak_llm::stream;
use vak_llm::types::{AssistantMessage, ChatRequest, ContentBlock, StopReason, Usage};
use vak_llm::{EventStream, LlmError, Provider};
use vak_permission::PermissionEngine;
use vak_session::types::{FrozenContract, SessionHeader};
use vak_session::{SessionLog, SessionPath};
use vak_tools::context::ToolContext;
use vak_tools::{Tool, ToolOutput};

/// What each request offered and required, and the schema its output was
/// asked in.
type Seen = Arc<Mutex<Vec<(Vec<String>, Option<String>, Option<String>)>>>;

struct Scripted {
    capacity_key: String,
    responses: Mutex<VecDeque<AssistantMessage>>,
    seen: Seen,
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
        self.seen.lock().unwrap().push((
            request.tools.iter().map(|tool| tool.name.clone()).collect(),
            request.must_call,
            request.output_schema.map(|output| output.name),
        ));
        let next = self.responses.lock().unwrap().pop_front();
        let (mut sink, rx) = stream::channel(16);
        match next {
            Some(m) => sink.close_message(m).await,
            None => sink.close_error(LlmError::Parse("exhausted".into())).await,
        }
        Ok(rx)
    }
}

struct TableCard;

#[async_trait]
impl Tool for TableCard {
    fn name(&self) -> &str {
        "emit_table_card"
    }
    fn description(&self) -> &str {
        "A table."
    }
    fn schema(&self) -> Value {
        json!({"type": "object", "properties": {
            "semantic_type": {"type": "string", "enum": ["table"]},
            "columns": {"type": "array", "items": {"type": "string"}, "description": "The column names."},
            "rows": {"type": "array", "items": {"type": "string"}, "description": "The rows, each `A | B`."}
        }, "required": ["semantic_type", "columns", "rows"]})
    }
    fn presents_cards(&self) -> bool {
        true
    }
    fn card_kinds(&self) -> Vec<String> {
        vec!["table".into()]
    }
    async fn execute(&self, _args: &Value, _ctx: &ToolContext) -> ToolOutput {
        ToolOutput::ok("Shown to the user.")
    }
}

fn message(content: Vec<ContentBlock>, stop_reason: StopReason) -> AssistantMessage {
    AssistantMessage {
        content,
        stop_reason,
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

fn call(id: &str, name: &str, input: Value) -> AssistantMessage {
    message(
        vec![ContentBlock::ToolUse {
            id: id.into(),
            name: name.into(),
            input,
        }],
        StopReason::ToolUse,
    )
}

fn text(t: &str) -> AssistantMessage {
    message(vec![ContentBlock::text(t)], StopReason::EndTurn)
}

/// The call seen live: the table's `rows` list is never closed, so the
/// provider could not read it and passed its text on.
fn broken_card(id: &str) -> AssistantMessage {
    call(
        id,
        "emit_table_card",
        json!({"semantic_type": "table", "deliverable": "comparison", "columns": ["App", "Sync"],
               "rows": "[\"Obsidian | Local\", \"Notion | Cloud\", \"type\": \"table\"}"}),
    )
}

#[tokio::test]
async fn a_card_that_keeps_failing_is_located_shown_its_schema_then_withdrawn() {
    let dir = tempfile::tempdir().unwrap();
    let header = SessionHeader {
        space: None,
        run: None,
        cause: None,
        agent: None,
        session_id: "card-repair".into(),
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
    let ledger = SessionPath::new_session_file(&home, dir.path(), "card-repair");
    let log = SessionLog::create(ledger.clone(), header).unwrap();
    let seen: Seen = Arc::new(Mutex::new(Vec::new()));
    let plan = |id: &str| {
        call(
            id,
            "plan",
            json!({"deliverables": [{"name": "comparison", "form": "table"}]}),
        )
    };
    let mut agent = Agent::new(
        Arc::new(Scripted {
            capacity_key: crate::support::capacity_key(),
            responses: Mutex::new(VecDeque::from(vec![
                plan("p1"),
                broken_card("a1"),
                // Each unreadable call is first written again under its
                // schema; here that fails too, so the ladder goes on.
                text("not json"),
                // A step that calls something else does not start the
                // ladder again.
                plan("p2"),
                broken_card("a2"),
                text("not json"),
                broken_card("a3"),
                text("not json"),
                message(
                    vec![ContentBlock::text(
                        "Obsidian keeps notes local; Notion syncs.",
                    )],
                    StopReason::EndTurn,
                ),
            ])),
            seen: seen.clone(),
        }),
        log,
        {
            let mut cfg = AgentConfig::new("sys");
            cfg.model = "test-model".into();
            cfg.tools = vec![Arc::new(vak_tools::PlanTool), Arc::new(TableCard)];
            cfg.mode = vak_permission::Mode::FullAccess;
            cfg.permission = Some(Arc::new(PermissionEngine::default()));
            cfg.stop_policy = None;
            cfg
        },
    );
    let outcome = agent
        .run(
            "compare two note apps",
            &Default::default(),
            CancellationToken::new(),
            mpsc::channel(64).0,
        )
        .await;
    // The model's own text is the answer, and the planned card, which
    // cannot be shown any more, is not asked for again.
    assert!(
        matches!(
            &outcome,
            TurnOutcome::Completed { response }
                if response.text_content() == "Obsidian keeps notes local; Notion syncs."
        ),
        "{outcome:?}"
    );
    let seen = seen.lock().unwrap();
    assert_eq!(seen.len(), 9, "{seen:?}");
    // Every regeneration offers no tool and asks for the card's schema.
    let (steps, rewrites): (Vec<_>, Vec<_>) =
        seen.iter().partition(|(_, _, output)| output.is_none());
    assert_eq!(rewrites.len(), 3, "{seen:?}");
    assert!(
        rewrites
            .iter()
            .all(|(tools, _, output)| tools.is_empty()
                && output.as_deref() == Some("emit_table_card")),
        "{seen:?}"
    );
    assert!(
        steps[..5]
            .iter()
            .all(|(tools, _, _)| tools.iter().any(|name| name == "emit_table_card")),
        "{seen:?}"
    );
    assert!(
        !steps[5].0.iter().any(|name| name == "emit_table_card")
            && steps[5].0.iter().any(|name| name == "plan"),
        "after the withdrawal the card tool is not offered: {seen:?}"
    );
    assert!(
        seen.iter().all(|(_, required, _)| required.is_none()),
        "{seen:?}"
    );

    let text = SessionLog::text(&ledger);
    // The refusal names the field and says where its text breaks, once.
    assert!(
        text.contains("expected `,` or `]` in arguments.rows"),
        "{text}"
    );
    assert!(
        !text.contains("invalid tool arguments: invalid tool arguments"),
        "{text}"
    );
    assert!(!text.contains("sent as text"), "{text}");
    let schema_at = text
        .find("The same tool call failed twice")
        .expect("schema shown");
    let withdrawn_at = text
        .find("`emit_table_card` is withdrawn for the rest of this turn")
        .expect("card tool withdrawn");
    assert!(schema_at < withdrawn_at);
    assert_eq!(
        text.matches("is withdrawn for the rest").count(),
        1,
        "{text}"
    );
}

/// An unreadable call is written again under its tool's schema before it is
/// ledgered: the call runs with the arguments that stand, and the ledger
/// keeps what the model first wrote (docs/design/89 §4.3).
#[tokio::test]
async fn an_unreadable_call_is_regenerated_under_its_schema_and_runs() {
    let dir = tempfile::tempdir().unwrap();
    let header = SessionHeader {
        space: None,
        run: None,
        cause: None,
        agent: None,
        session_id: "card-regenerated".into(),
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
    let ledger = SessionPath::new_session_file(&home, dir.path(), "card-regenerated");
    let log = SessionLog::create(ledger.clone(), header).unwrap();
    let seen: Seen = Arc::new(Mutex::new(Vec::new()));
    let rewritten = json!({"semantic_type": "table", "deliverable": "comparison",
        "columns": ["App", "Sync"], "rows": ["Obsidian | Local", "Notion | Cloud"]});
    let mut agent = Agent::new(
        Arc::new(Scripted {
            capacity_key: crate::support::capacity_key(),
            responses: Mutex::new(VecDeque::from(vec![
                broken_card("a1"),
                text(&rewritten.to_string()),
                text("Obsidian keeps notes local; Notion syncs."),
            ])),
            seen: seen.clone(),
        }),
        log,
        {
            let mut cfg = AgentConfig::new("sys");
            cfg.model = "test-model".into();
            cfg.tools = vec![Arc::new(TableCard)];
            cfg.mode = vak_permission::Mode::FullAccess;
            cfg.permission = Some(Arc::new(PermissionEngine::default()));
            cfg.stop_policy = None;
            cfg
        },
    );
    let outcome = agent
        .run(
            "compare two note apps",
            &Default::default(),
            CancellationToken::new(),
            mpsc::channel(64).0,
        )
        .await;
    assert!(
        matches!(&outcome, TurnOutcome::Completed { .. }),
        "{outcome:?}"
    );
    {
        let seen = seen.lock().unwrap();
        assert_eq!(seen.len(), 3, "{seen:?}");
        assert!(seen[1].0.is_empty(), "the rewrite offers no tool: {seen:?}");
        assert_eq!(seen[1].2.as_deref(), Some("emit_table_card"));
    }
    let session = agent.session.lock().await;
    let ran = session
        .derive_messages()
        .iter()
        .flat_map(|message| message.content.clone())
        .find_map(|block| match block {
            ContentBlock::ToolUse { id, input, .. } if id == "a1" => Some(input),
            _ => None,
        })
        .expect("the call is ledgered");
    assert_eq!(ran["rows"], json!(["Obsidian | Local", "Notion | Cloud"]));
    let refused = session
        .derive_messages()
        .iter()
        .flat_map(|message| message.content.clone())
        .any(|block| matches!(block, ContentBlock::ToolResult { tool_use_id, is_error: true, .. } if tool_use_id == "a1"));
    assert!(!refused, "the rewritten call ran");
    drop(session);
    let text = SessionLog::text(&ledger);
    assert!(text.contains("arguments-regenerated"), "{text}");
    assert!(
        text.contains("Notion | Cloud\\\", \\\"type\\\""),
        "what the model wrote is kept: {text}"
    );
}

/// A call that is valid JSON but the wrong shape is written again under its
/// schema at once, as an unreadable one is (doc 89 §4.4): telling the model
/// first left the rewrite to its calling the tool again, and on 2026-10-14
/// a steps card told it was missing `steps` was never called again.
#[tokio::test]
async fn a_call_of_the_wrong_shape_is_regenerated_under_its_schema_at_once() {
    let dir = tempfile::tempdir().unwrap();
    let header = SessionHeader {
        space: None,
        run: None,
        cause: None,
        agent: None,
        session_id: "card-shape-regenerated".into(),
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
    let ledger = SessionPath::new_session_file(&home, dir.path(), "card-shape-regenerated");
    let log = SessionLog::create(ledger.clone(), header).unwrap();
    let seen: Seen = Arc::new(Mutex::new(Vec::new()));
    let rewritten = json!({"semantic_type": "table", "deliverable": "comparison",
        "columns": ["App", "Sync"], "rows": ["Obsidian | Local", "Notion | Cloud"]});
    let mut agent = Agent::new(
        Arc::new(Scripted {
            capacity_key: crate::support::capacity_key(),
            responses: Mutex::new(VecDeque::from(vec![
                call(
                    "a0",
                    "emit_table_card",
                    json!({"semantic_type": "table", "rows": ["A | B"]}),
                ),
                text(&rewritten.to_string()),
                text("Obsidian keeps notes local; Notion syncs."),
            ])),
            seen: seen.clone(),
        }),
        log,
        {
            let mut cfg = AgentConfig::new("sys");
            cfg.model = "test-model".into();
            cfg.tools = vec![Arc::new(TableCard)];
            cfg.mode = vak_permission::Mode::FullAccess;
            cfg.permission = Some(Arc::new(PermissionEngine::default()));
            cfg.stop_policy = None;
            cfg
        },
    );
    let outcome = agent
        .run(
            "compare two note apps",
            &Default::default(),
            CancellationToken::new(),
            mpsc::channel(64).0,
        )
        .await;
    assert!(
        matches!(&outcome, TurnOutcome::Completed { .. }),
        "{outcome:?}"
    );
    {
        let seen = seen.lock().unwrap();
        assert_eq!(seen.len(), 3, "{seen:?}");
        assert_eq!(seen[1].2.as_deref(), Some("emit_table_card"), "{seen:?}");
    }
    let session = agent.session.lock().await;
    let ran = session
        .derive_messages()
        .iter()
        .flat_map(|message| message.content.clone())
        .find_map(|block| match block {
            ContentBlock::ToolUse { id, input, .. } if id == "a0" => Some(input),
            _ => None,
        })
        .expect("the call is ledgered");
    assert_eq!(ran["rows"], json!(["Obsidian | Local", "Notion | Cloud"]));
    let refused = session
        .derive_messages()
        .iter()
        .flat_map(|message| message.content.clone())
        .any(|block| matches!(block, ContentBlock::ToolResult { tool_use_id, is_error: true, .. } if tool_use_id == "a0"));
    assert!(!refused, "the rewritten call ran");
    drop(session);
    let text = SessionLog::text(&ledger);
    assert!(text.contains("arguments-regenerated"), "{text}");
}

/// A call of the wrong shape to a tool the request did not carry (deferred,
/// known to the model by name only) is written again under its schema at
/// once: the model never saw the schema it would be told it broke.
#[tokio::test]
async fn a_blind_call_to_a_deferred_tool_is_regenerated_at_once() {
    let dir = tempfile::tempdir().unwrap();
    let header = SessionHeader {
        space: None,
        run: None,
        cause: None,
        agent: None,
        session_id: "card-blind-regenerated".into(),
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
    let ledger = SessionPath::new_session_file(&home, dir.path(), "card-blind-regenerated");
    let log = SessionLog::create(ledger.clone(), header).unwrap();
    let seen: Seen = Arc::new(Mutex::new(Vec::new()));
    let rewritten = json!({"semantic_type": "table", "deliverable": "comparison",
        "columns": ["App", "Sync"], "rows": ["Obsidian | Local", "Notion | Cloud"]});
    let mut agent = Agent::new(
        Arc::new(Scripted {
            capacity_key: crate::support::capacity_key(),
            responses: Mutex::new(VecDeque::from(vec![
                call(
                    "a1",
                    "emit_table_card",
                    json!({"semantic_type": "table", "rows": ["A | B"]}),
                ),
                text(&rewritten.to_string()),
                text("Obsidian keeps notes local; Notion syncs."),
            ])),
            seen: seen.clone(),
        }),
        log,
        {
            let mut cfg = AgentConfig::new("sys");
            cfg.model = "test-model".into();
            cfg.tools = vec![Arc::new(TableCard)];
            cfg.tool_definitions = Some(
                vak_tools::definitions(&cfg.tools)
                    .into_iter()
                    .map(|definition| definition.deferred())
                    .collect(),
            );
            cfg.mode = vak_permission::Mode::FullAccess;
            cfg.permission = Some(Arc::new(PermissionEngine::default()));
            cfg.stop_policy = None;
            cfg
        },
    );
    let outcome = agent
        .run(
            "compare two note apps",
            &Default::default(),
            CancellationToken::new(),
            mpsc::channel(64).0,
        )
        .await;
    assert!(
        matches!(&outcome, TurnOutcome::Completed { .. }),
        "{outcome:?}"
    );
    {
        let seen = seen.lock().unwrap();
        assert_eq!(seen.len(), 3, "{seen:?}");
        assert_eq!(
            seen[1].2.as_deref(),
            Some("emit_table_card"),
            "rewritten at once, never refused first: {seen:?}"
        );
    }
    let session = agent.session.lock().await;
    let ran = session
        .derive_messages()
        .iter()
        .flat_map(|message| message.content.clone())
        .find_map(|block| match block {
            ContentBlock::ToolUse { id, input, .. } if id == "a1" => Some(input),
            _ => None,
        })
        .expect("the call is ledgered");
    assert_eq!(ran["rows"], json!(["Obsidian | Local", "Notion | Cloud"]));
    let refused = session
        .derive_messages()
        .iter()
        .flat_map(|message| message.content.clone())
        .any(|block| matches!(block, ContentBlock::ToolResult { tool_use_id, is_error: true, .. } if tool_use_id == "a1"));
    assert!(!refused, "the rewritten call ran");
    drop(session);
    let text = SessionLog::text(&ledger);
    assert!(text.contains("arguments-regenerated"), "{text}");
}

/// A card refused for what is not there, and a tool that is not a card
/// failing on its target, side by side (2026-10-15, the installed route: a
/// file preview of a file that did not exist was refused at every step
/// until the turn was stopped).
struct MissingFilePreview;

#[async_trait]
impl Tool for MissingFilePreview {
    fn name(&self) -> &str {
        "emit_file_preview_card"
    }
    fn description(&self) -> &str {
        "A preview of a file."
    }
    fn schema(&self) -> Value {
        json!({"type": "object", "properties": {
            "artifact_path": {"type": "string", "description": "The file."}
        }, "required": ["artifact_path"]})
    }
    fn presents_cards(&self) -> bool {
        true
    }
    fn card_kinds(&self) -> Vec<String> {
        vec!["file_preview".into()]
    }
    async fn execute(&self, _args: &Value, _ctx: &ToolContext) -> ToolOutput {
        ToolOutput::error(
            vak_control::Fault::Target,
            "Not shown: that file does not exist.",
        )
    }
}

struct GonePage;

#[async_trait]
impl Tool for GonePage {
    fn name(&self) -> &str {
        "fetch"
    }
    fn description(&self) -> &str {
        "Fetch a page."
    }
    fn schema(&self) -> Value {
        json!({"type": "object", "properties": {
            "url": {"type": "string", "description": "The page."}
        }, "required": ["url"]})
    }
    async fn execute(&self, _args: &Value, _ctx: &ToolContext) -> ToolOutput {
        ToolOutput::error(vak_control::Fault::Target, "404 Not Found")
    }
}

async fn run_with(tools: Vec<Arc<dyn Tool>>, script: Vec<AssistantMessage>) -> String {
    let dir = tempfile::tempdir().unwrap();
    let header = SessionHeader {
        space: None,
        run: None,
        cause: None,
        agent: None,
        session_id: "target-repair".into(),
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
    let ledger = SessionPath::new_session_file(&home, dir.path(), "target-repair");
    let log = SessionLog::create(ledger.clone(), header).unwrap();
    let mut agent = Agent::new(
        Arc::new(Scripted {
            capacity_key: crate::support::capacity_key(),
            responses: Mutex::new(VecDeque::from(script)),
            seen: Arc::new(Mutex::new(Vec::new())),
        }),
        log,
        {
            let mut cfg = AgentConfig::new("sys");
            cfg.model = "test-model".into();
            cfg.tools = tools;
            cfg.mode = vak_permission::Mode::FullAccess;
            cfg.permission = Some(Arc::new(PermissionEngine::default()));
            cfg.stop_policy = None;
            cfg
        },
    );
    let outcome = agent
        .run(
            "whats my name?",
            &Default::default(),
            CancellationToken::new(),
            mpsc::channel(64).0,
        )
        .await;
    assert!(
        matches!(outcome, TurnOutcome::Completed { .. }),
        "{outcome:?}"
    );
    SessionLog::text(&ledger)
}

#[tokio::test]
async fn a_card_refused_for_a_missing_file_is_withdrawn_and_a_gone_page_is_not() {
    let preview = |id: &str| {
        call(
            id,
            "emit_file_preview_card",
            json!({"artifact_path": format!("/workspace/placeholder-{id}")}),
        )
    };
    let written = run_with(
        vec![Arc::new(MissingFilePreview)],
        vec![
            preview("a1"),
            preview("a2"),
            preview("a3"),
            text("Your name is Nisheeth Ranjan."),
        ],
    )
    .await;
    assert!(
        written.contains("`emit_file_preview_card` is withdrawn for the rest of this turn"),
        "{written}"
    );

    let fetch = |id: &str| {
        call(
            id,
            "fetch",
            json!({"url": format!("https://example.invalid/{id}")}),
        )
    };
    let written = run_with(
        vec![Arc::new(GonePage)],
        vec![
            fetch("f1"),
            fetch("f2"),
            fetch("f3"),
            text("Those pages are gone."),
        ],
    )
    .await;
    assert!(!written.contains("is withdrawn for the rest"), "{written}");
    assert!(
        !written.contains("The same tool call failed twice"),
        "{written}"
    );
}

/// A card that shows what a command printed waits for a command
/// (`Tool::grounded_in`): deferred until a call runs one, then promoted in
/// place from the next step of the same turn, as a tool called by name is
/// (docs/design/68-context-engine.md §5).
struct Runs;

#[async_trait]
impl Tool for Runs {
    fn name(&self) -> &str {
        "run"
    }
    fn description(&self) -> &str {
        "Run a command."
    }
    fn schema(&self) -> Value {
        json!({"type": "object", "properties": {
            "command": {"type": "string", "description": "The command."}
        }, "required": ["command"]})
    }
    async fn execute(&self, _args: &Value, _ctx: &ToolContext) -> ToolOutput {
        ToolOutput::ok("hi").ran(Some(0), 3)
    }
}

struct Terminal;

#[async_trait]
impl Tool for Terminal {
    fn name(&self) -> &str {
        "emit_terminal_card"
    }
    fn description(&self) -> &str {
        "A command you ran."
    }
    fn schema(&self) -> Value {
        json!({"type": "object", "properties": {
            "command": {"type": "string", "description": "The command."}
        }, "required": ["command"]})
    }
    fn presents_cards(&self) -> bool {
        true
    }
    fn card_kinds(&self) -> Vec<String> {
        vec!["terminal".into()]
    }
    fn grounded_in(&self) -> Option<vak_tools::Grounds> {
        Some(vak_tools::Grounds::Ran)
    }
    async fn execute(&self, _args: &Value, _ctx: &ToolContext) -> ToolOutput {
        ToolOutput::ok("Shown to the user.")
    }
}

#[tokio::test]
async fn a_card_grounded_in_a_command_is_offered_from_the_step_after_one_ran() {
    let dir = tempfile::tempdir().unwrap();
    let header = SessionHeader {
        space: None,
        run: None,
        cause: None,
        agent: None,
        session_id: "grounded".into(),
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
    let ledger = SessionPath::new_session_file(&home, dir.path(), "grounded");
    let log = SessionLog::create(ledger.clone(), header).unwrap();
    let seen: Seen = Arc::new(Mutex::new(Vec::new()));
    let tools: Vec<Arc<dyn Tool>> = vec![Arc::new(Runs), Arc::new(Terminal)];
    let mut definitions = vak_tools::definitions(&tools);
    for definition in &mut definitions {
        if definition.name == "emit_terminal_card" {
            definition.defer = true;
        }
    }
    let mut agent = Agent::new(
        Arc::new(Scripted {
            capacity_key: crate::support::capacity_key(),
            responses: Mutex::new(VecDeque::from(vec![
                call("r1", "run", json!({"command": "echo hi"})),
                call("t1", "emit_terminal_card", json!({"command": "echo hi"})),
                text("Done."),
            ])),
            seen: seen.clone(),
        }),
        log,
        {
            let mut cfg = AgentConfig::new("sys");
            cfg.model = "test-model".into();
            cfg.tools = tools;
            cfg.tool_definitions = Some(definitions);
            cfg.mode = vak_permission::Mode::FullAccess;
            cfg.permission = Some(Arc::new(PermissionEngine::default()));
            cfg.stop_policy = None;
            cfg
        },
    );
    let outcome = agent
        .run(
            "run echo hi and show it",
            &Default::default(),
            CancellationToken::new(),
            mpsc::channel(64).0,
        )
        .await;
    assert!(
        matches!(outcome, TurnOutcome::Completed { .. }),
        "{outcome:?}"
    );
    let seen = seen.lock().unwrap();
    let offered = |step: usize| seen[step].0.iter().any(|name| name == "emit_terminal_card");
    assert!(!offered(0), "no command has run yet: {seen:?}");
    assert!(offered(1), "the command ran at step 1: {seen:?}");
    assert!(
        SessionLog::text(&ledger).contains("Shown to the user."),
        "the card was shown"
    );
}
