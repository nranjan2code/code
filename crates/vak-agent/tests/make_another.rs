//! Make another (plan M8.3b, docs/design/82-library.md §6): in a turn that
//! asks for a new artifact like an existing one, the loop refuses a write
//! to the source with a message the model can act on, so the result is a
//! new file and the source stays as it was.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod support;

use std::sync::Arc;
use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;
use vak_agent::{Agent, AgentConfig, AutoApprove, TurnOutcome};
use vak_llm::Provider;
use vak_llm::types::{AssistantMessage, ContentBlock, StopReason, Usage};
use vak_permission::{Mode, PermissionEngine};
use vak_session::SessionLog;
use vak_session::types::{FrozenContract, SessionHeader};
use vak_tools::write::WriteTool;

fn write_call(id: &str, path: &str, content: &str) -> AssistantMessage {
    AssistantMessage {
        content: vec![ContentBlock::ToolUse {
            id: id.into(),
            name: "write".into(),
            input: serde_json::json!({"path": path, "content": content}),
        }],
        stop_reason: StopReason::ToolUse,
        usage: Usage::default(),
        model: "test-model".into(),
        response_id: None,
    }
}

fn text(t: &str) -> AssistantMessage {
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
    }
}

struct Scripted {
    capacity_key: String,
    msgs: std::sync::Mutex<std::collections::VecDeque<AssistantMessage>>,
}

#[async_trait::async_trait]
impl Provider for Scripted {
    fn name(&self) -> &str {
        "scripted"
    }
    fn rate_limit_key(&self) -> String {
        self.capacity_key.clone()
    }
    async fn stream(
        &self,
        _request: vak_llm::types::ChatRequest,
        _cancel: CancellationToken,
    ) -> Result<vak_llm::EventStream, vak_llm::LlmError> {
        let next = self.msgs.lock().unwrap().pop_front();
        let (mut sink, rx) = vak_llm::stream::channel(8);
        match next {
            Some(m) => {
                sink.push(vak_llm::stream::StreamEvent::Start { partial: m.clone() });
                sink.close_message(m).await;
            }
            None => {
                sink.close_error(vak_llm::LlmError::Parse("exhausted".into()))
                    .await
            }
        }
        Ok(rx)
    }
}

#[tokio::test]
async fn make_another_keeps_its_source() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("poem.md"), "the original").unwrap();
    let header = SessionHeader {
        space: None,
        run: None,
        cause: None,
        agent: None,
        session_id: "s".into(),
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
    let log = SessionLog::create(dir.path().join("s.jsonl"), header).unwrap();
    let mut cfg = AgentConfig::new("sys");
    cfg.tools = vec![Arc::new(WriteTool)];
    cfg.mode = Mode::FullAccess;
    cfg.permission = Some(Arc::new(PermissionEngine::default()));
    cfg.approver = Some(Arc::new(AutoApprove));
    cfg.stop_policy = None;
    cfg.protected_paths = vec!["poem.md".into()];
    let provider = Scripted {
        capacity_key: crate::support::capacity_key(),
        msgs: std::sync::Mutex::new(
            vec![
                write_call("t1", "./poem.md", "a rewrite"),
                write_call("t2", "poem-2.md", "a new one"),
                text("Wrote poem-2.md."),
            ]
            .into(),
        ),
    };
    let mut agent = Agent::new(Arc::new(provider), log, cfg);
    let outcome = agent
        .run(
            "Make another one like poem.md, as a new file.",
            &Default::default(),
            CancellationToken::new(),
            mpsc::channel(64).0,
        )
        .await;
    assert!(
        matches!(outcome, TurnOutcome::Completed { .. }),
        "{outcome:?}"
    );
    assert_eq!(
        std::fs::read_to_string(dir.path().join("poem.md")).unwrap(),
        "the original"
    );
    assert_eq!(
        std::fs::read_to_string(dir.path().join("poem-2.md")).unwrap(),
        "a new one"
    );
    let session = agent.session.lock().await;
    let refused = session
        .message_chain()
        .iter()
        .flat_map(|(_, m)| m.content.iter())
        .find_map(|b| match b {
            ContentBlock::ToolResult {
                tool_use_id,
                content,
                is_error,
            } if tool_use_id == "t1" => Some((content.clone(), *is_error)),
            _ => None,
        })
        .expect("the refused call has a result");
    assert!(refused.1, "the refusal is an error result");
    assert!(refused.0.contains("Not changed: poem.md"), "{}", refused.0);
}
