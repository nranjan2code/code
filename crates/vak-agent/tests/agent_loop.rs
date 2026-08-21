#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::collections::VecDeque;
use std::sync::{Arc, Mutex};

use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;

use tempfile::tempdir;

use vak_agent::{Agent, AgentConfig, TurnOutcome};
use vak_llm::stream;
use vak_llm::types::{AssistantMessage, ChatRequest, ContentBlock, StopReason, Usage};
use vak_llm::{EventStream, LlmError, Provider};
use vak_session::SessionLog;
use vak_session::types::{FrozenContract, SessionHeader};
use vak_tools::Tool;
use vak_tools::bash::BashTool;
use vak_tools::write::WriteTool;

enum ScriptedResponse {
    Message(AssistantMessage),
    Error(LlmError),
}

struct Scripted {
    responses: Mutex<VecDeque<ScriptedResponse>>,
    requests: Arc<Mutex<Vec<ChatRequest>>>,
}

#[async_trait::async_trait]
impl Provider for Scripted {
    fn name(&self) -> &str {
        "scripted"
    }

    async fn stream(
        &self,
        request: ChatRequest,
        _cancel: CancellationToken,
    ) -> Result<EventStream, LlmError> {
        self.requests.lock().unwrap().push(request);
        let next = self.responses.lock().unwrap().pop_front();
        let (sink, rx) = stream::channel(64);
        match next {
            Some(ScriptedResponse::Message(m)) => {
                sink.push(stream::StreamEvent::Start { partial: m.clone() });
                sink.close_message(m);
            }
            Some(ScriptedResponse::Error(e)) => sink.close_error(e),
            None => sink.close_error(LlmError::Parse("script exhausted".into())),
        }
        Ok(rx)
    }
}
fn assistant_text(text: &str) -> AssistantMessage {
    AssistantMessage {
        content: vec![ContentBlock::text(text)],
        stop_reason: StopReason::EndTurn,
        usage: Usage {
            input_tokens: 10,
            output_tokens: 5,
            ..Default::default()
        },
        model: "test-model".into(),
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
    }
}

struct Harness {
    agent: Agent,
    requests: Arc<Mutex<Vec<ChatRequest>>>,
    events_tx: mpsc::Sender<vak_agent::AgentEvent>,
    _events_rx: mpsc::Receiver<vak_agent::AgentEvent>,
}

fn harness(responses: Vec<ScriptedResponse>, tools: Vec<Arc<dyn Tool>>) -> Harness {
    let dir = tempdir().unwrap();
    let header = SessionHeader {
        session_id: "s-test".into(),
        created_at: chrono::Utc::now(),
        cwd: dir.path().to_path_buf(),
        parent_session_id: None,
        contract: FrozenContract {
            app_version: "0.1.0".into(),
            provider: "scripted".into(),
            model: "test-model".into(),
            system_prompt: "sys".into(),
            tools: tools.iter().map(|t| t.name().to_string()).collect(),
            permission_mode: "full-access".into(),
            skills: Vec::new(),
        },
    };
    let log = SessionLog::create(dir.path().join("s.jsonl"), header).unwrap();
    let mut cfg = AgentConfig::new("sys");
    cfg.tools = tools;
    let (tx, rx) = mpsc::channel(4096);
    let requests: Arc<Mutex<Vec<ChatRequest>>> = Arc::new(Mutex::new(Vec::new()));
    let provider = Arc::new(Scripted {
        responses: Mutex::new(responses.into_iter().collect()),
        requests: requests.clone(),
    });
    std::mem::forget(dir);
    Harness {
        agent: Agent::new(provider, log, cfg),
        requests,
        events_tx: tx,
        _events_rx: rx,
    }
}

fn next_tool_id() -> String {
    use std::sync::atomic::{AtomicU32, Ordering};
    static C: AtomicU32 = AtomicU32::new(0);
    format!("t{}", C.fetch_add(1, Ordering::Relaxed))
}

#[tokio::test]
async fn single_turn_no_tools_completes() {
    let mut h = harness(
        vec![ScriptedResponse::Message(assistant_text("done"))],
        vec![],
    );
    let outcome = h
        .agent
        .run(
            "hello",
            &Default::default(),
            CancellationToken::new(),
            h.events_tx.clone(),
        )
        .await;
    match outcome {
        TurnOutcome::Completed { response } => assert_eq!(response.text_content(), "done"),
        other => panic!("expected completed, got {other:?}"),
    }
    let session = h.agent.session.lock().await;
    assert_eq!(session.derive_messages().len(), 2);
}

#[tokio::test]
async fn tool_roundtrip_executes_and_feeds_result_back() {
    let h = harness(
        vec![
            ScriptedResponse::Message(tool_call_msg(
                &next_tool_id(),
                "bash",
                serde_json::json!({"command": "echo ran"}),
            )),
            ScriptedResponse::Message(assistant_text("finished")),
        ],
        vec![Arc::new(BashTool)],
    );
    let mut agent = h.agent;
    let outcome = agent
        .run(
            "run it",
            &Default::default(),
            CancellationToken::new(),
            h.events_tx.clone(),
        )
        .await;
    assert!(matches!(outcome, TurnOutcome::Completed { .. }));

    let (req_count, has_tool_result) = {
        let reqs = h.requests.lock().unwrap();
        let hit = reqs.get(1).map(|r| {
            r.messages.iter().any(|m| {
                m.content
                    .iter()
                    .any(|b| matches!(b, ContentBlock::ToolResult { content, is_error: false, .. } if content.contains("ran")))
            })
        }).unwrap_or(false);
        (reqs.len(), hit)
    };
    assert_eq!(req_count, 2);
    assert!(
        has_tool_result,
        "second request must contain the tool result"
    );

    let session = agent.session.lock().await;
    assert_eq!(session.derive_messages().len(), 4);
}

#[tokio::test]
async fn unknown_tool_becomes_error_value_not_crash() {
    let h = harness(
        vec![
            ScriptedResponse::Message(tool_call_msg("t1", "nonexistent", serde_json::json!({}))),
            ScriptedResponse::Message(assistant_text("recovered")),
        ],
        vec![Arc::new(BashTool)],
    );
    let mut agent = h.agent;
    let outcome = agent
        .run(
            "go",
            &Default::default(),
            CancellationToken::new(),
            h.events_tx.clone(),
        )
        .await;
    assert!(matches!(outcome, TurnOutcome::Completed { .. }));
    let session = agent.session.lock().await;
    let msgs = session.derive_messages();
    let results_block = msgs
        .iter()
        .rev()
        .flat_map(|m| m.content.iter())
        .find_map(|b| match b {
            ContentBlock::ToolResult { content, .. } => Some(content.clone()),
            _ => None,
        })
        .expect("a tool result must exist");
    assert!(results_block.contains("unknown tool"));
}

#[tokio::test]
async fn parallel_tools_preserve_source_order() {
    let msg = AssistantMessage {
        content: vec![
            ContentBlock::ToolUse {
                id: "a".into(),
                name: "write".into(),
                input: serde_json::json!({"path": "a.txt", "content": "A"}),
            },
            ContentBlock::ToolUse {
                id: "b".into(),
                name: "write".into(),
                input: serde_json::json!({"path": "b.txt", "content": "B"}),
            },
        ],
        stop_reason: StopReason::ToolUse,
        usage: Usage::default(),
        model: "test-model".into(),
    };
    let h = harness(
        vec![
            ScriptedResponse::Message(msg),
            ScriptedResponse::Message(assistant_text("both done")),
        ],
        vec![Arc::new(WriteTool), Arc::new(BashTool)],
    );
    let mut h = h;
    h.agent.config.parallel_tools = true;
    let mut agent = h.agent;
    let outcome = agent
        .run(
            "fan out",
            &Default::default(),
            CancellationToken::new(),
            h.events_tx.clone(),
        )
        .await;
    assert!(matches!(outcome, TurnOutcome::Completed { .. }));

    let session = agent.session.lock().await;
    let msgs = session.derive_messages();
    let results_msg = msgs
        .iter()
        .rev()
        .find(|m| matches!(m.content.first(), Some(ContentBlock::ToolResult { .. })))
        .expect("tool results message must exist");
    assert_eq!(results_msg.content.len(), 2);
    if let ContentBlock::ToolResult { tool_use_id, .. } = &results_msg.content[0] {
        assert_eq!(
            tool_use_id, "a",
            "results must follow assistant source order"
        );
    } else {
        panic!("expected tool result block");
    }
}

#[tokio::test]
async fn abort_mid_stream_returns_partial_and_persists_it() {
    let h = harness(
        vec![ScriptedResponse::Error(LlmError::Aborted {
            partial: Some(assistant_text("partial work")),
        })],
        vec![],
    );
    let mut agent = h.agent;
    let outcome = agent
        .run(
            "start",
            &Default::default(),
            CancellationToken::new(),
            h.events_tx.clone(),
        )
        .await;
    match outcome {
        TurnOutcome::Aborted { partial } => {
            assert_eq!(partial.unwrap().text_content(), "partial work");
        }
        other => panic!("expected aborted, got {other:?}"),
    }
    let session = agent.session.lock().await;
    assert_eq!(
        session.derive_messages().last().unwrap().text_content(),
        "partial work"
    );
}

#[tokio::test]
async fn max_turns_guard_stops_runaway_loops() {
    let responses: Vec<_> = (0..6)
        .map(|_| {
            ScriptedResponse::Message(tool_call_msg(
                &next_tool_id(),
                "bash",
                serde_json::json!({"command": "true"}),
            ))
        })
        .collect();
    let mut h = harness(responses, vec![Arc::new(BashTool)]);
    h.agent.config.max_turns = 3;
    let mut agent = h.agent;
    let outcome = agent
        .run(
            "loop forever",
            &Default::default(),
            CancellationToken::new(),
            h.events_tx.clone(),
        )
        .await;
    assert!(matches!(outcome, TurnOutcome::MaxTurnsReached));
}

#[tokio::test]
async fn projection_invariant_every_request_message_is_logged() {
    let h = harness(
        vec![
            ScriptedResponse::Message(tool_call_msg(
                &next_tool_id(),
                "bash",
                serde_json::json!({"command": "echo x"}),
            )),
            ScriptedResponse::Message(assistant_text("ok")),
        ],
        vec![Arc::new(BashTool)],
    );
    let mut agent = h.agent;
    let _ = agent
        .run(
            "invariant check",
            &Default::default(),
            CancellationToken::new(),
            h.events_tx.clone(),
        )
        .await;

    let final_msgs = agent.session.lock().await.derive_messages();
    for req in h.requests.lock().unwrap().iter() {
        for msg in &req.messages {
            assert!(
                final_msgs.contains(msg),
                "model-visible means logged: request message missing from log projection"
            );
        }
    }
}
