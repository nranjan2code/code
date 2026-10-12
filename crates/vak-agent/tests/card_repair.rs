#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

//! The repair ladder for a card tool (docs/design/15-reliability.md,
//! docs/design/87-composed-answers.md §2). Seen live on 2026-10-11: a list
//! argument arrived as text that was not JSON, and the refusal gave a
//! column in a 2,700-character line. Here the refusal says where the text
//! breaks, the tool's schema is shown, then it is withdrawn, a step that
//! calls something else does not start the ladder again, and a planned
//! card that can no longer be shown is checked as text.

mod support;

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

/// What each request offered and required.
type Seen = Arc<Mutex<Vec<(Vec<String>, Option<String>)>>>;

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

/// The call seen live: the table's `rows` list is never closed, so the
/// provider could not read it and passed its text on.
fn broken_card(id: &str) -> AssistantMessage {
    call(
        id,
        "emit_table_card",
        json!({"semantic_type": "table", "answers": "comparison", "columns": ["App", "Sync"],
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
                // A step that calls something else does not start the
                // ladder again.
                plan("p2"),
                broken_card("a2"),
                broken_card("a3"),
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
    assert_eq!(seen.len(), 6, "{seen:?}");
    assert!(
        seen[..5]
            .iter()
            .all(|(tools, _)| tools.iter().any(|name| name == "emit_table_card")),
        "{seen:?}"
    );
    assert!(
        !seen[5].0.iter().any(|name| name == "emit_table_card")
            && seen[5].0.iter().any(|name| name == "plan"),
        "after the withdrawal the card tool is not offered: {seen:?}"
    );
    assert!(
        seen.iter().all(|(_, required)| required.is_none()),
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
