#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

//! Cards are Vak's own display channel: a card call is checked by the tool
//! that `presents_cards()` in every permission mode,
//! without an approval and without a denial in read-only. Real trigger: "Vak wants to use emit_metric_card — this needs
//! your approval" shown under a card that had already rendered.

use std::collections::VecDeque;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;

use vak_agent::{Agent, AgentConfig, AgentEvent, TurnOutcome};
use vak_llm::stream;
use vak_llm::types::{AssistantMessage, ChatRequest, ContentBlock, StopReason, Usage};
use vak_llm::{EventStream, LlmError, Provider};
use vak_permission::{Mode, PermissionEngine};
use vak_session::types::{FrozenContract, SessionHeader};
use vak_session::{SessionLog, SessionPath};
use vak_tools::context::ToolContext;
use vak_tools::{Tool, ToolOutput};

struct Scripted(Mutex<VecDeque<AssistantMessage>>, String);

#[async_trait]
impl Provider for Scripted {
    fn name(&self) -> &str {
        "scripted"
    }

    fn rate_limit_key(&self) -> String {
        self.1.clone()
    }

    async fn stream(
        &self,
        _request: ChatRequest,
        _cancel: CancellationToken,
    ) -> Result<EventStream, LlmError> {
        let next = self.0.lock().unwrap().pop_front();
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

struct CardTool {
    runs: Arc<AtomicUsize>,
}

#[async_trait]
impl Tool for CardTool {
    fn name(&self) -> &str {
        "emit_metric_card"
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
    fn card_kinds(&self) -> Vec<String> {
        vec!["metric".into()]
    }
    async fn execute(&self, _args: &serde_json::Value, _ctx: &ToolContext) -> ToolOutput {
        self.runs.fetch_add(1, Ordering::SeqCst);
        ToolOutput::ok("Card displayed to the user.")
    }
}

fn msg(content: Vec<ContentBlock>, stop_reason: StopReason) -> AssistantMessage {
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

/// Runs one turn whose answer is one metric card; returns (executions,
/// approvals asked).
async fn answer_with_a_card(mode: Mode) -> (usize, usize) {
    card_call(mode, serde_json::json!({}), &[]).await
}

/// Runs one turn whose answer is one card call with `input`, in a workspace
/// holding `files`; returns (executions, approvals asked).
async fn card_call(mode: Mode, input: serde_json::Value, files: &[&str]) -> (usize, usize) {
    let dir = tempfile::tempdir().unwrap();
    for file in files {
        std::fs::write(dir.path().join(file), "<p>x</p>").unwrap();
    }
    let home = dir.path().join("home");
    std::fs::create_dir_all(&home).unwrap();
    let header = SessionHeader {
        space: None,
        run: None,
        cause: None,
        agent: None,
        session_id: "card-perm".into(),
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
            permission_mode: "workspace-write".into(),
            capabilities: Vec::new(),
            prompt_layers: Vec::new(),
        },
    };
    let log = SessionLog::create(
        SessionPath::new_session_file(&home, dir.path(), "card-perm"),
        header,
    )
    .unwrap();
    let runs = Arc::new(AtomicUsize::new(0));
    let mut cfg = AgentConfig::new("sys");
    cfg.model = "test-model".into();
    cfg.mode = mode;
    cfg.permission = Some(Arc::new(PermissionEngine::default().with_tools([(
        "emit_metric_card".to_string(),
        vak_permission::Declared::reach(vak_permission::Reach::Display),
    )])));
    cfg.tools = vec![Arc::new(CardTool { runs: runs.clone() })];
    let mut agent = Agent::new(
        Arc::new(Scripted(
            Mutex::new(VecDeque::from([
                msg(
                    vec![ContentBlock::ToolUse {
                        id: "a0".into(),
                        name: "emit_metric_card".into(),
                        input,
                    }],
                    StopReason::ToolUse,
                ),
                msg(
                    vec![ContentBlock::text("Shown above.")],
                    StopReason::EndTurn,
                ),
            ])),
            crate::support::capacity_key(),
        )),
        log,
        cfg,
    );
    let (tx, mut rx) = mpsc::channel(256);
    let outcome = agent
        .run("show it", &Default::default(), CancellationToken::new(), tx)
        .await;
    assert!(
        matches!(outcome, TurnOutcome::Completed { .. }),
        "{outcome:?}"
    );
    let mut approvals = 0;
    while let Ok(event) = rx.try_recv() {
        if matches!(event, AgentEvent::ApprovalRequested { .. }) {
            approvals += 1;
        }
    }
    (runs.load(Ordering::SeqCst), approvals)
}

#[tokio::test]
async fn card_tools_run_without_approval_in_every_mode() {
    for mode in [Mode::WorkspaceWrite, Mode::ReadOnly, Mode::FullAccess] {
        assert_eq!(answer_with_a_card(mode).await, (1, 0), "{mode:?}");
    }
}

/// Found live 2026-10-13: asked to preview markup with no file, a model
/// gave a file preview a path to nothing. A card naming a file that exists
/// neither in the workspace nor among the Agent's drafts is not shown.
#[tokio::test]
async fn a_card_that_previews_a_missing_file_is_not_shown() {
    let missing = serde_json::json!({"artifact_path": "save-button.html"});
    assert_eq!(card_call(Mode::WorkspaceWrite, missing, &[]).await.0, 0);
    let folder = serde_json::json!({"artifact_path": "."});
    assert_eq!(card_call(Mode::WorkspaceWrite, folder, &[]).await.0, 0);
    let present = serde_json::json!({"artifact_path": "page.html"});
    assert_eq!(
        card_call(Mode::WorkspaceWrite, present, &["page.html"])
            .await
            .0,
        1
    );
}

#[test]
fn a_tool_the_host_did_not_declare_as_presenting_still_needs_approval() {
    let engine = PermissionEngine::default().with_tools([(
        "emit_metric_card".to_string(),
        vak_permission::Declared::reach(vak_permission::Reach::Display),
    )]);
    let cwd = std::env::temp_dir();
    let decision = engine.evaluate(
        "emit_lookalike_card",
        &serde_json::json!({}),
        Mode::WorkspaceWrite,
        &cwd,
    );
    assert!(
        matches!(decision, vak_permission::Decision::Ask { .. }),
        "only tools the host declared as presenting are exempt: {decision:?}"
    );
}

/// A card whose schema the model cannot satisfy.
struct StrictCardTool;

#[async_trait]
impl Tool for StrictCardTool {
    fn name(&self) -> &str {
        "emit_metric_card"
    }
    fn description(&self) -> &str {
        "test stand-in with a required label"
    }
    fn schema(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "object",
            "properties": {"label": {"type": "string"}},
            "required": ["label"]
        })
    }
    fn presents_cards(&self) -> bool {
        true
    }
    async fn execute(&self, _args: &serde_json::Value, _ctx: &ToolContext) -> ToolOutput {
        ToolOutput::ok("Card displayed to the user.")
    }
}

/// A card is a presentation of the answer, not part of the work. One that
/// fails validation is simply not shown; a complete prose answer after it
/// ends the turn instead of being sent back to repair the card. Measured
/// live: a small model could not build the card's arguments, and the stop
/// gate's demand to repair it turned a correct, sourced answer into a
/// failed turn.
#[tokio::test]
async fn a_failed_card_does_not_hold_back_a_complete_answer() {
    let dir = tempfile::tempdir().unwrap();
    let home = dir.path().join("home");
    std::fs::create_dir_all(&home).unwrap();
    let header = SessionHeader {
        space: None,
        run: None,
        cause: None,
        agent: None,
        session_id: "card-failed".into(),
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
            permission_mode: "workspace-write".into(),
            capabilities: Vec::new(),
            prompt_layers: Vec::new(),
        },
    };
    let log = SessionLog::create(
        SessionPath::new_session_file(&home, dir.path(), "card-failed"),
        header,
    )
    .unwrap();
    let mut cfg = AgentConfig::new("sys");
    cfg.model = "test-model".into();
    cfg.mode = Mode::WorkspaceWrite;
    cfg.permission = Some(Arc::new(PermissionEngine::default().with_tools([(
        "emit_metric_card".to_string(),
        vak_permission::Declared::reach(vak_permission::Reach::Display),
    )])));
    cfg.tools = vec![Arc::new(StrictCardTool)];
    let answer = "Copper trades at about $6.70 per pound, according to the exchange quote.";
    let mut agent = Agent::new(
        Arc::new(Scripted(
            Mutex::new(VecDeque::from([
                msg(
                    vec![ContentBlock::ToolUse {
                        id: "c0".into(),
                        name: "emit_metric_card".into(),
                        input: serde_json::json!({"metric_data": [1, 2]}),
                    }],
                    StopReason::ToolUse,
                ),
                // Its regeneration under the schema is refused too, so the
                // call runs as written and fails.
                msg(
                    vec![ContentBlock::text("{\"metric_data\": [1, 2]}")],
                    StopReason::EndTurn,
                ),
                msg(vec![ContentBlock::text(answer)], StopReason::EndTurn),
            ])),
            crate::support::capacity_key(),
        )),
        log,
        cfg,
    );
    let outcome = agent
        .run(
            "what is copper trading at",
            &Default::default(),
            CancellationToken::new(),
            mpsc::channel(256).0,
        )
        .await;
    assert!(
        matches!(&outcome, TurnOutcome::Completed { response } if response.text_content() == answer),
        "{outcome:?}"
    );
    let nudged = agent
        .session
        .lock()
        .await
        .message_chain()
        .iter()
        .flat_map(|(_, message)| message.content.iter())
        .any(|block| matches!(block, ContentBlock::Text { text } if text.contains("[stop-guard]")));
    assert!(!nudged, "the answer was sent back to repair a card");
}
