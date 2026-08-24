#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

//! Doom-loop guard: the third identical (tool, args) call in one run is
//! re-routed through approval instead of executing silently.

use std::collections::VecDeque;
use std::sync::{Arc, Mutex};

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
use vak_tools::bash::BashTool;

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

#[tokio::test]
async fn third_identical_call_is_blocked_with_reason() {
    let dir = tempdir().unwrap();
    let header = SessionHeader {
        session_id: "doom-loop".into(),
        created_at: chrono::Utc::now(),
        cwd: dir.path().to_path_buf(),
        parent_session_id: None,
        contract: FrozenContract {
            app_version: "0".into(),
            provider: "scripted".into(),
            model: "test-model".into(),
            route_ladder: Vec::new(),
            system_prompt: "sys".into(),
            tools: vec!["bash".into()],
            permission_mode: "full-access".into(),
            skills: Vec::new(),
        },
    };
    let home = dir.path().join("home");
    std::fs::create_dir_all(&home).unwrap();
    let log = SessionLog::create(
        SessionPath::new_session_file(&home, dir.path(), "doom-loop"),
        header,
    )
    .unwrap();

    let mut agent = Agent::new(
        Arc::new(Scripted {
            responses: Mutex::new(VecDeque::from(vec![
                bash_call("a", "echo same"),
                bash_call("b", "echo same"),
                bash_call("c", "echo same"),
                text_msg("adapted after guard"),
            ])),
        }),
        log,
        {
            let mut cfg = AgentConfig::new("sys");
            cfg.model = "test-model".into();
            cfg.tools = vec![Arc::new(BashTool)];
            cfg.mode = vak_permission::Mode::FullAccess;
            cfg.permission = Some(Arc::new(PermissionEngine::default()));
            // AutoDeny makes the doom-loop Ask observable as a typed denial.
            cfg.approver = Some(Arc::new(vak_agent::AutoDeny));
            cfg
        },
    );

    let outcome = agent
        .run(
            "loop",
            &Default::default(),
            CancellationToken::new(),
            mpsc::channel(64).0,
        )
        .await;
    assert!(
        matches!(outcome, TurnOutcome::Completed { .. }),
        "got {outcome:?}"
    );

    let results: Vec<(bool, String)> = agent
        .session
        .lock()
        .await
        .derive_messages()
        .iter()
        .flat_map(|m| m.content.iter())
        .filter_map(|b| match b {
            ContentBlock::ToolResult {
                content, is_error, ..
            } => Some((*is_error, content.clone())),
            _ => None,
        })
        .collect();
    assert_eq!(results.len(), 3);
    assert!(!results[0].0 && !results[1].0, "first two run normally");
    assert!(results[2].0, "third identical call must be blocked");
    assert!(
        results[2].1.contains("repeated"),
        "guard reason must reach the model: {}",
        results[2].1
    );
}

#[tokio::test]
async fn different_args_are_not_counted_together() {
    let dir = tempdir().unwrap();
    let header = SessionHeader {
        session_id: "no-doom".into(),
        created_at: chrono::Utc::now(),
        cwd: dir.path().to_path_buf(),
        parent_session_id: None,
        contract: FrozenContract {
            app_version: "0".into(),
            provider: "scripted".into(),
            model: "test-model".into(),
            route_ladder: Vec::new(),
            system_prompt: "sys".into(),
            tools: vec!["bash".into()],
            permission_mode: "full-access".into(),
            skills: Vec::new(),
        },
    };
    let home = dir.path().join("home");
    std::fs::create_dir_all(&home).unwrap();
    let log = SessionLog::create(
        SessionPath::new_session_file(&home, dir.path(), "no-doom"),
        header,
    )
    .unwrap();

    let mut agent = Agent::new(
        Arc::new(Scripted {
            responses: Mutex::new(VecDeque::from(vec![
                bash_call("a", "echo one"),
                bash_call("b", "echo two"),
                bash_call("c", "echo three"),
                text_msg("done"),
            ])),
        }),
        log,
        {
            let mut cfg = AgentConfig::new("sys");
            cfg.model = "test-model".into();
            cfg.tools = vec![Arc::new(BashTool)];
            cfg.mode = vak_permission::Mode::FullAccess;
            cfg.permission = Some(Arc::new(PermissionEngine::default()));
            cfg.approver = Some(Arc::new(vak_agent::AutoDeny));
            cfg
        },
    );

    let outcome = agent
        .run(
            "distinct commands",
            &Default::default(),
            CancellationToken::new(),
            mpsc::channel(64).0,
        )
        .await;
    assert!(matches!(outcome, TurnOutcome::Completed { .. }));
    for is_error in agent
        .session
        .lock()
        .await
        .derive_messages()
        .iter()
        .flat_map(|m| m.content.iter())
        .filter_map(|b| match b {
            ContentBlock::ToolResult { is_error, .. } => Some(*is_error),
            _ => None,
        })
    {
        assert!(!is_error, "distinct calls must never trip the guard");
    }
}
