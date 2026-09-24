#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

//! A call a tool refuses from its arguments alone is refused before
//! permission is evaluated: nobody is asked to approve a text edit of a
//! Word file, which could only fail, and the model gets the repair hint.

use std::collections::VecDeque;
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
);

#[async_trait]
impl Provider for Scripted {
    fn name(&self) -> &str {
        "scripted"
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
        Arc::new(Scripted(Mutex::new(script), requests.clone())),
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
