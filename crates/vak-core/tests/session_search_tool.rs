#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::collections::VecDeque;
use std::path::Path;
use std::sync::{Arc, Mutex};

use tokio_util::sync::CancellationToken;

use vak_core::Core;
use vak_llm::stream;
use vak_llm::types::{AssistantMessage, ChatRequest, ContentBlock, Usage};
use vak_llm::{EventStream, LlmError, Provider};
use vak_session::types::{FrozenContract, MessageRecord, SessionHeader};
use vak_session::{SessionLog, SessionPath};

struct Scripted {
    responses: Mutex<VecDeque<AssistantMessage>>,
}

#[async_trait::async_trait]
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

fn text(t: &str) -> AssistantMessage {
    AssistantMessage {
        content: vec![ContentBlock::text(t)],
        stop_reason: vak_llm::types::StopReason::EndTurn,
        usage: Usage {
            input_tokens: 7,
            output_tokens: 3,
            ..Default::default()
        },
        model: "test-model".into(),
    }
}

fn tool_call(id: &str, name: &str, input: serde_json::Value) -> AssistantMessage {
    AssistantMessage {
        content: vec![ContentBlock::ToolUse {
            id: id.into(),
            name: name.into(),
            input,
        }],
        stop_reason: vak_llm::types::StopReason::ToolUse,
        usage: Usage::default(),
        model: "test-model".into(),
    }
}

fn header_for(id: &str, cwd: &Path) -> SessionHeader {
    SessionHeader {
        session_id: id.to_string(),
        created_at: chrono::Utc::now(),
        cwd: cwd.to_path_buf(),
        parent_session_id: None,
        contract_id: None,
        work_item_id: None,
        contract: FrozenContract {
            app_version: "test".into(),
            provider: "scripted".into(),
            model: "m".into(),
            route_ladder: Vec::new(),
            route_objective: String::new(),
            route_annotations: Vec::new(),
            system_prompt: String::new(),
            permission_mode: "workspace-write".into(),
            capabilities: Vec::new(),
            prompt_layers: Vec::new(),
        },
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn session_search_tool_is_available_and_logged() {
    let dir = tempfile::tempdir().unwrap();
    let home = dir.path().join("home");
    let cwd = dir.path().to_path_buf();

    // A past session with retrievable knowledge.
    let past_path = SessionPath::new_session_file(&home, &cwd, "11111111-past");
    let mut past = SessionLog::create(past_path, header_for("11111111-past", &cwd)).unwrap();
    past.append_message(MessageRecord {
        message: vak_llm::Message {
            role: vak_llm::Role::User,
            content: vec![ContentBlock::text(
                "we decided the deploy script must pause before rollback windows",
            )],
        },
        meta: None,
    })
    .unwrap();
    drop(past); // release the ledger lock

    // Live run: the scripted model reaches for memory, then answers.
    let core = Core::new(cwd.clone()).unwrap();
    core.set_sessions_home(home.clone());
    core.set_permission_mode(vak_config::PermissionMode::FullAccess);
    core.set_provider_instance(Arc::new(Scripted {
        responses: Mutex::new(VecDeque::from(vec![
            tool_call(
                "t1",
                "session_search",
                serde_json::json!({"query": "deploy script"}),
            ),
            text("Found it: the deploy script pauses before rollbacks."),
        ])),
    }));
    std::mem::forget(dir);

    // Direct probe of the search function on identical inputs.
    let probe = vak_session::search(&home, &cwd, "deploy script", 8, None).unwrap();
    eprintln!("PROBE hits={}", probe.len());
    for h in &probe {
        eprintln!(
            "  -> {} score={} snippet={}",
            h.session_id, h.score, h.snippet
        );
    }

    let live = core.start_session().await.unwrap();
    let live_id = live.header().unwrap().session_id.clone();
    let (events_tx, _events_rx) = tokio::sync::mpsc::channel(256);
    let result = core
        .run_turn_with(
            live,
            "what did we decide about deploys?",
            CancellationToken::new(),
            None,
            None,
            None,
            events_tx,
        )
        .await
        .unwrap();
    assert!(matches!(result.0, vak_agent::TurnOutcome::Completed { .. }));
    let log = result.1;

    // Invariant 1: the search output must be reconstructable from the
    // ledger — it lives in the ToolResult block's content.
    let chain = log.message_chain();
    let mut saw_search_output = false;
    for (_, m) in &chain {
        for b in &m.content {
            if let ContentBlock::ToolResult {
                content, is_error, ..
            } = b
            {
                eprintln!("TOOL_RESULT[{is_error}]: {content}");
                assert!(!is_error, "search must not error: {content}");
                if content.contains("hit(s)") {
                    saw_search_output = true;
                    assert!(
                        content.contains("11111111-past"),
                        "snippet cites the source session: {content}"
                    );
                    assert!(
                        !content.contains(&live_id),
                        "current session must be excluded from results"
                    );
                }
            }
        }
    }
    assert!(
        saw_search_output,
        "tool result with hits must be logged on the chain"
    );

    // And the model's answer reflects what memory returned.
    let answered = chain
        .iter()
        .any(|(_, m)| m.text_content().contains("pauses before rollbacks"));
    assert!(answered, "final answer on the chain");

    // The tool appears in the frozen contract.
    let capabilities = &log.header().unwrap().contract.capabilities;
    assert!(
        capabilities.iter().any(|capability| {
            capability.kind == vak_session::CapabilityKind::Tool
                && capability.name == "session_search"
        }),
        "session_search in frozen contract: {capabilities:?}"
    );
}
