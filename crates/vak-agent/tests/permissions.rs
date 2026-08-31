#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::sync::Arc;

use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;

use tempfile::tempdir;

use vak_agent::{Agent, AgentConfig, Approver, AutoApprove, TurnOutcome};
use vak_llm::Provider;
use vak_llm::types::{AssistantMessage, ContentBlock, StopReason, Usage};
use vak_permission::PermissionEngine;
use vak_session::SessionLog;
use vak_session::types::{FrozenContract, SessionHeader};
use vak_tools::bash::BashTool;

fn bash_call(id: &str, cmd: &str) -> AssistantMessage {
    AssistantMessage {
        content: vec![ContentBlock::ToolUse {
            id: id.into(),
            name: "bash".into(),
            input: serde_json::json!({"command": cmd}),
        }],
        stop_reason: StopReason::ToolUse,
        usage: Usage::default(),
        model: "test-model".into(),
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
    }
}

struct MultiScripted {
    msgs: std::sync::Mutex<std::collections::VecDeque<AssistantMessage>>,
}

#[async_trait::async_trait]
impl Provider for MultiScripted {
    fn name(&self) -> &str {
        "scripted"
    }

    async fn stream(
        &self,
        _request: vak_llm::types::ChatRequest,
        _cancel: CancellationToken,
    ) -> Result<vak_llm::EventStream, vak_llm::LlmError> {
        let next = self.msgs.lock().unwrap().pop_front();
        let (mut sink, rx) = vak_llm::stream::channel(8);
        if let Some(m) = next {
            sink.push(vak_llm::stream::StreamEvent::Start { partial: m.clone() });
            sink.close_message(m).await;
        } else {
            sink.close_error(vak_llm::LlmError::Parse("exhausted".into()))
                .await;
        }
        Ok(rx)
    }
}

fn multi_setup(
    responses: Vec<AssistantMessage>,
    engine: Option<PermissionEngine>,
    approver: Option<Arc<dyn Approver>>,
) -> Agent {
    let dir = tempdir().unwrap();
    let header = SessionHeader {
        session_id: "s".into(),
        created_at: chrono::Utc::now(),
        cwd: dir.path().to_path_buf(),
        parent_session_id: None,
        contract_id: None,
        work_item_id: None,
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
        },
    };
    let log = SessionLog::create(dir.path().join("s.jsonl"), header).unwrap();
    let mut cfg = AgentConfig::new("sys");
    cfg.tools = vec![Arc::new(BashTool)];
    cfg.permission = engine.map(Arc::new);
    cfg.approver = approver;
    std::mem::forget(dir);
    let provider = MultiScripted {
        msgs: std::sync::Mutex::new(responses.into_iter().collect()),
    };
    Agent::new(Arc::new(provider), log, cfg)
}

#[tokio::test]
async fn deny_rule_blocks_execution_and_feeds_reason_back() {
    let eng = PermissionEngine::from_rule_strings(&["-Bash(rm *)".to_string()]).unwrap();
    let mut agent = multi_setup(
        vec![bash_call("t1", "rm -rf /"), text_msg("done")],
        Some(eng),
        Some(Arc::new(AutoApprove)),
    );
    let outcome = agent
        .run(
            "go",
            &Default::default(),
            CancellationToken::new(),
            mpsc::channel(64).0,
        )
        .await;
    assert!(matches!(outcome, TurnOutcome::Completed { .. }));
    let session = agent.session.lock().await;
    let result = session
        .derive_messages()
        .iter()
        .flat_map(|m| m.content.iter())
        .find_map(|b| match b {
            ContentBlock::ToolResult {
                content, is_error, ..
            } => Some((content.clone(), *is_error)),
            _ => None,
        })
        .expect("tool result exists");
    assert!(result.1, "denied call must be an error result");
    assert!(result.0.contains("denied by rule"));
}

#[tokio::test]
async fn ask_without_approver_is_denied_with_hint() {
    let eng = PermissionEngine::default();
    let mut agent = multi_setup(
        vec![bash_call("t1", "echo hi"), text_msg("ok")],
        Some(eng),
        None,
    );
    let outcome = agent
        .run(
            "go",
            &Default::default(),
            CancellationToken::new(),
            mpsc::channel(64).0,
        )
        .await;
    assert!(matches!(outcome, TurnOutcome::Completed { .. }));
    let session = agent.session.lock().await;
    let result = session
        .derive_messages()
        .iter()
        .flat_map(|m| m.content.iter())
        .find_map(|b| match b {
            ContentBlock::ToolResult {
                content, is_error, ..
            } => Some((content.clone(), *is_error)),
            _ => None,
        })
        .unwrap();
    assert!(result.0.contains("no approver available"));
}

#[tokio::test]
async fn approved_ask_executes_the_tool() {
    let eng = PermissionEngine::default();
    let marker = tempdir().unwrap();
    let cmd = format!("touch {}/marker", marker.path().display());
    let mut agent = multi_setup(
        vec![bash_call("t1", &cmd), text_msg("ok")],
        Some(eng),
        Some(Arc::new(AutoApprove)),
    );
    let outcome = agent
        .run(
            "go",
            &Default::default(),
            CancellationToken::new(),
            mpsc::channel(64).0,
        )
        .await;
    assert!(matches!(outcome, TurnOutcome::Completed { .. }));
    assert!(
        marker.path().join("marker").exists(),
        "approved command must have run"
    );
}
