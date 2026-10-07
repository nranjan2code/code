#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

//! A call a tool refuses from its arguments alone is refused before
//! permission is evaluated: nobody is asked to approve a text edit of a
//! Word file, which could only fail, and the model gets the repair hint.
//! And when correctable failures persist, the loop's repair directive is
//! runtime-authored control traffic, never a message from the person.
//! And an answer after a tool delivered a reviewable file (an Office
//! draft) is not sent back to be re-presented as a card; a repeat of the
//! delivering call writes no second draft, and a card previewing the
//! delivered file is not shown.

mod support;

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

struct Scripted(
    Mutex<VecDeque<AssistantMessage>>,
    Arc<Mutex<Vec<ChatRequest>>>,
    String,
);

#[async_trait]
impl Provider for Scripted {
    fn name(&self) -> &str {
        "scripted"
    }

    fn rate_limit_key(&self) -> String {
        self.2.clone()
    }

    async fn stream(
        &self,
        request: ChatRequest,
        _cancel: CancellationToken,
    ) -> Result<EventStream, LlmError> {
        self.1.lock().unwrap().push(request);
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
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_text_edit_of_a_word_file_is_refused_without_asking_anyone() {
    let dir = tempfile::tempdir().unwrap();
    let home = dir.path().join("home");
    std::fs::create_dir_all(&home).unwrap();
    std::fs::write(
        dir.path().join("q3.docx"),
        b"PK\x03\x04 not really a package",
    )
    .unwrap();
    let header = SessionHeader {
        space: None,
        run: None,
        cause: None,
        agent: None,
        session_id: "refusal".into(),
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
        SessionPath::new_session_file(&home, dir.path(), "refusal"),
        header,
    )
    .unwrap();
    let mut cfg = AgentConfig::new("sys");
    cfg.model = "test-model".into();
    cfg.mode = Mode::WorkspaceWrite;
    cfg.retry_base_backoff_ms = 1;
    cfg.run_retry_base_backoff_ms = 1;
    // Every edit would ask for approval, so an approval request is what the
    // refusal must pre-empt.
    cfg.permission = Some(Arc::new(
        PermissionEngine::from_rule_strings(&["?edit".to_string()]).unwrap(),
    ));
    cfg.tools = vec![Arc::new(vak_tools::edit::EditTool)];
    let script = VecDeque::from([msg(
        vec![ContentBlock::ToolUse {
            id: "c1".into(),
            name: "edit".into(),
            input: serde_json::json!({
                "path": "q3.docx",
                "edits": [{"old_text": "Steady.", "new_text": "Growing."}]
            }),
        }],
        StopReason::ToolUse,
    )]);
    // Spare answers: a text answer after a failed call may be sent back for
    // a redo, and this test is about what happened before it.
    let mut script = script;
    for _ in 0..6 {
        script.push_back(msg(
            vec![ContentBlock::text(
                "The file is a Word document, so I will use office_apply.",
            )],
            StopReason::EndTurn,
        ));
    }
    let requests = Arc::new(Mutex::new(Vec::new()));
    let mut agent = Agent::new(
        Arc::new(Scripted(
            Mutex::new(script),
            requests.clone(),
            crate::support::capacity_key(),
        )),
        log,
        cfg,
    );
    let (tx, mut rx) = mpsc::channel(256);
    let outcome = agent
        .run("edit it", &Default::default(), CancellationToken::new(), tx)
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
    assert_eq!(
        approvals, 0,
        "a call that can only be refused is never put to a person"
    );
    let requests = requests.lock().unwrap();
    let refusal = requests
        .iter()
        .flat_map(|request| request.messages.iter())
        .flat_map(|message| message.content.iter())
        .find_map(|block| match block {
            ContentBlock::ToolResult {
                tool_use_id,
                content,
                is_error: true,
            } if tool_use_id == "c1" => Some(content.clone()),
            _ => None,
        })
        .expect("the model is told why");
    assert!(
        refusal.contains("office_apply") && refusal.contains("doc_read"),
        "{refusal}"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn the_repair_directive_is_recorded_as_control_not_as_the_persons_words() {
    let dir = tempfile::tempdir().unwrap();
    let home = dir.path().join("home");
    std::fs::create_dir_all(&home).unwrap();
    std::fs::write(dir.path().join("notes.txt"), "plain").unwrap();
    let header = SessionHeader {
        space: None,
        run: None,
        cause: None,
        agent: None,
        session_id: "directive".into(),
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
    let ledger = SessionPath::new_session_file(&home, dir.path(), "directive");
    let log = SessionLog::create(ledger.clone(), header).unwrap();
    let mut cfg = AgentConfig::new("sys");
    cfg.model = "test-model".into();
    cfg.mode = Mode::WorkspaceWrite;
    cfg.retry_base_backoff_ms = 1;
    cfg.run_retry_base_backoff_ms = 1;
    cfg.tools = vec![Arc::new(vak_tools::edit::EditTool)];
    // The same malformed call, step after step: `edits` is missing.
    let mut script: VecDeque<AssistantMessage> = (0..8)
        .map(|n| {
            msg(
                vec![ContentBlock::ToolUse {
                    id: format!("c{n}"),
                    name: "edit".into(),
                    input: serde_json::json!({"path": "notes.txt"}),
                }],
                StopReason::ToolUse,
            )
        })
        .collect();
    for _ in 0..4 {
        script.push_back(msg(
            vec![ContentBlock::text("Stopped.")],
            StopReason::EndTurn,
        ));
    }
    let requests = Arc::new(Mutex::new(Vec::new()));
    let mut agent = Agent::new(
        Arc::new(Scripted(
            Mutex::new(script),
            requests.clone(),
            crate::support::capacity_key(),
        )),
        log,
        cfg,
    );
    let (tx, mut rx) = mpsc::channel(512);
    tokio::spawn(async move { while rx.recv().await.is_some() {} });
    agent
        .run(
            "fix the notes",
            &Default::default(),
            CancellationToken::new(),
            tx,
        )
        .await;
    let ledger = vak_session::SessionLog::text(&ledger);
    let directives: Vec<&str> = ledger
        .lines()
        .filter(|line| line.contains("[repair-directive]"))
        .collect();
    assert!(!directives.is_empty(), "the loop issued a repair directive");
    for line in directives {
        assert!(
            line.contains("\"control\":\"repair_directive\""),
            "tagged as runtime control: {line}"
        );
    }
    assert!(
        !ledger.contains("[repair directive]"),
        "the retired inline marker is not written"
    );
}

#[derive(Default)]
struct Deliverer {
    delivers: bool,
    runs: AtomicUsize,
}

#[async_trait]
impl vak_tools::Tool for Deliverer {
    fn name(&self) -> &str {
        "make_draft"
    }
    fn description(&self) -> &str {
        "test stand-in"
    }
    fn schema(&self) -> serde_json::Value {
        serde_json::json!({"type": "object"})
    }
    fn delivered_file(&self, _args: &serde_json::Value) -> Option<String> {
        self.delivers.then(|| "deck.pptx".to_string())
    }
    async fn execute(
        &self,
        _args: &serde_json::Value,
        _ctx: &vak_tools::context::ToolContext,
    ) -> vak_tools::ToolOutput {
        self.runs.fetch_add(1, Ordering::SeqCst);
        vak_tools::ToolOutput::ok("Draft for deck.pptx written; the person reviews it.")
    }
}

#[derive(Default)]
struct Shell {
    runs: AtomicUsize,
}

#[async_trait]
impl vak_tools::Tool for Shell {
    fn name(&self) -> &str {
        "bash"
    }
    fn description(&self) -> &str {
        "test stand-in"
    }
    fn schema(&self) -> serde_json::Value {
        serde_json::json!({"type": "object"})
    }
    async fn execute(
        &self,
        _args: &serde_json::Value,
        _ctx: &vak_tools::context::ToolContext,
    ) -> vak_tools::ToolOutput {
        self.runs.fetch_add(1, Ordering::SeqCst);
        vak_tools::ToolOutput::ok("ran")
    }
}

#[derive(Default)]
struct PreviewCard {
    runs: AtomicUsize,
}

#[async_trait]
impl vak_tools::Tool for PreviewCard {
    fn name(&self) -> &str {
        "emit_ui_preview_card"
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
    async fn execute(
        &self,
        _args: &serde_json::Value,
        _ctx: &vak_tools::context::ToolContext,
    ) -> vak_tools::ToolOutput {
        self.runs.fetch_add(1, Ordering::SeqCst);
        vak_tools::ToolOutput::ok(r#"{"ok":true}"#)
    }
}

/// How many model requests one turn takes when the presentation check
/// always wants a card.
async fn requests_after_a_draft(delivers: bool, final_text: &str) -> usize {
    let dir = tempfile::tempdir().unwrap();
    let home = dir.path().join("home");
    std::fs::create_dir_all(&home).unwrap();
    let header = SessionHeader {
        space: None,
        run: None,
        cause: None,
        agent: None,
        session_id: "deliver".into(),
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
        SessionPath::new_session_file(&home, dir.path(), "deliver"),
        header,
    )
    .unwrap();
    let mut cfg = AgentConfig::new("sys");
    cfg.model = "test-model".into();
    cfg.mode = Mode::WorkspaceWrite;
    cfg.retry_base_backoff_ms = 1;
    cfg.run_retry_base_backoff_ms = 1;
    cfg.tools = vec![Arc::new(Deliverer {
        delivers,
        ..Default::default()
    })];
    cfg.presentation_check = Some(Arc::new(|_text: &str, _offered: &[String]| {
        Some(vak_agent::PresentationNudge {
            tool: "make_draft".into(),
            text: "[presentation-check] show it as a card".into(),
        })
    }));
    let mut script = VecDeque::from([msg(
        vec![ContentBlock::ToolUse {
            id: "c1".into(),
            name: "make_draft".into(),
            input: serde_json::json!({}),
        }],
        StopReason::ToolUse,
    )]);
    for _ in 0..3 {
        script.push_back(msg(
            vec![ContentBlock::text(final_text)],
            StopReason::EndTurn,
        ));
    }
    let requests = Arc::new(Mutex::new(Vec::new()));
    let mut agent = Agent::new(
        Arc::new(Scripted(
            Mutex::new(script),
            requests.clone(),
            crate::support::capacity_key(),
        )),
        log,
        cfg,
    );
    let (tx, mut rx) = mpsc::channel(256);
    tokio::spawn(async move { while rx.recv().await.is_some() {} });
    let outcome = agent
        .run(
            "add a slide",
            &Default::default(),
            CancellationToken::new(),
            tx,
        )
        .await;
    assert!(
        matches!(outcome, TurnOutcome::Completed { .. }),
        "{outcome:?}"
    );
    requests.lock().unwrap().len()
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn an_answer_after_a_delivered_draft_is_not_sent_back_to_become_a_card() {
    assert_eq!(
        requests_after_a_draft(true, "Added the slide; the draft is ready for review.").await,
        2,
        "the tool call, then the answer, which ends the turn"
    );
    assert_eq!(
        requests_after_a_draft(false, "Added the slide; the draft is ready for review.").await,
        3,
        "without a delivered file, the check still asks once for a card"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_delivered_file_can_finish_without_repeating_it_in_text() {
    assert_eq!(requests_after_a_draft(true, "").await, 2);
}

fn tool_call(id: &str, name: &str, input: serde_json::Value) -> AssistantMessage {
    msg(
        vec![ContentBlock::ToolUse {
            id: id.into(),
            name: name.into(),
            input,
        }],
        StopReason::ToolUse,
    )
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_repeated_draft_writes_nothing_and_a_preview_of_the_draft_is_not_shown() {
    let dir = tempfile::tempdir().unwrap();
    let home = dir.path().join("home");
    std::fs::create_dir_all(&home).unwrap();
    let header = SessionHeader {
        space: None,
        run: None,
        cause: None,
        agent: None,
        session_id: "repeat".into(),
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
        SessionPath::new_session_file(&home, dir.path(), "repeat"),
        header,
    )
    .unwrap();
    let drafts = Arc::new(Deliverer {
        delivers: true,
        ..Default::default()
    });
    let cards = Arc::new(PreviewCard::default());
    let shell = Arc::new(Shell::default());
    let mut cfg = AgentConfig::new("sys");
    cfg.model = "test-model".into();
    cfg.mode = Mode::FullAccess;
    cfg.retry_base_backoff_ms = 1;
    cfg.run_retry_base_backoff_ms = 1;
    cfg.tools = vec![drafts.clone(), cards.clone(), shell.clone()];
    let slide = serde_json::json!({"path": "deck.pptx", "ops": [{"op": "add_slide_from_layout"}]});
    let script = VecDeque::from([
        tool_call("c1", "make_draft", slide.clone()),
        tool_call(
            "c2",
            "emit_ui_preview_card",
            serde_json::json!({"artifact_path": "./deck.pptx", "html": "<h1>Next steps</h1>"}),
        ),
        tool_call(
            "c4",
            "bash",
            serde_json::json!({"command": "cp .vak/scratch/vak/c1/deck.pptx ./deck.pptx"}),
        ),
        tool_call(
            "c5",
            "bash",
            serde_json::json!({"command": "ls .vak/scratch"}),
        ),
        tool_call("c3", "make_draft", slide),
        msg(
            vec![ContentBlock::text("Added the Next steps slide.")],
            StopReason::EndTurn,
        ),
    ]);
    let requests = Arc::new(Mutex::new(Vec::new()));
    let mut agent = Agent::new(
        Arc::new(Scripted(
            Mutex::new(script),
            requests.clone(),
            crate::support::capacity_key(),
        )),
        log,
        cfg,
    );
    let (tx, mut rx) = mpsc::channel(256);
    tokio::spawn(async move { while rx.recv().await.is_some() {} });
    let outcome = agent
        .run(
            "add a slide",
            &Default::default(),
            CancellationToken::new(),
            tx,
        )
        .await;
    assert!(
        matches!(outcome, TurnOutcome::Completed { .. }),
        "{outcome:?}"
    );
    assert_eq!(drafts.runs.load(Ordering::SeqCst), 1, "one draft written");
    assert_eq!(
        cards.runs.load(Ordering::SeqCst),
        0,
        "the preview never ran"
    );
    let requests = requests.lock().unwrap();
    let results: Vec<(String, String)> = requests
        .last()
        .unwrap()
        .messages
        .iter()
        .flat_map(|message| message.content.iter())
        .filter_map(|block| match block {
            ContentBlock::ToolResult {
                tool_use_id,
                content,
                ..
            } => Some((tool_use_id.clone(), format!("{content:?}"))),
            _ => None,
        })
        .collect();
    let result = |id: &str| {
        results
            .iter()
            .find(|(call, _)| call == id)
            .map(|(_, text)| text.clone())
            .unwrap_or_default()
    };
    assert!(result("c2").contains("Not shown: deck.pptx"), "{results:?}");
    assert!(
        result("c3").contains("Already drafted") && result("c3").contains("Draft for deck.pptx"),
        "{results:?}"
    );
    assert!(
        result("c4").contains("Not run: deck.pptx is a draft waiting for the person's review"),
        "copying the draft out of scratch is refused even in FullAccess: {results:?}"
    );
    assert_eq!(
        shell.runs.load(Ordering::SeqCst),
        1,
        "only the command that does not name the draft ran"
    );
}
