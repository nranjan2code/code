#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::collections::VecDeque;
use std::sync::{Arc, Mutex};

use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;

use tempfile::tempdir;

use vak_agent::{Agent, AgentConfig, TurnOutcome};
use vak_hooks::HookDef;
use vak_llm::stream;
use vak_llm::types::{AssistantMessage, ChatRequest, ContentBlock, StopReason, Usage};
use vak_llm::{EventStream, LlmError, Provider};
use vak_session::SessionLog;
use vak_session::types::{FrozenContract, SessionHeader};
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

fn build(responses: Vec<AssistantMessage>, hooks: Option<Vec<HookDef>>) -> Agent {
    let dir = tempdir().unwrap();
    let header = SessionHeader {
        session_id: "s-hooks".into(),
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
    let log = SessionLog::create(dir.path().join("s.jsonl"), header).unwrap();
    let mut cfg = AgentConfig::new("sys");
    cfg.tools = vec![Arc::new(BashTool)];
    cfg.mode = vak_permission::Mode::FullAccess;
    cfg.hooks = hooks.map(Arc::new);
    std::mem::forget(dir);
    Agent::new(
        Arc::new(Scripted {
            responses: Mutex::new(responses.into_iter().collect()),
        }),
        log,
        cfg,
    )
}

#[tokio::test]
async fn pre_tool_use_hook_blocks_execution() {
    let marker = tempdir().unwrap();
    let marker_path = marker.path().join("ran");
    let hooks = vec![HookDef {
        event: vak_hooks::HookEvent::PreToolUse,
        matcher: Some(vak_permission::Rule::parse("Bash(touch *)").unwrap()),
        command: r#"echo '{"decision":"block","reason":"no touching"}'"#.to_string(),
        timeout_ms: 5000,
    }];
    let marker_path = marker_path.display().to_string();
    let mut agent = build(
        vec![
            bash_call("t1", &format!("touch {marker_path}")),
            text_msg("adapted"),
        ],
        Some(hooks),
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
        !std::path::Path::new(&marker_path).exists(),
        "blocked command must never have executed"
    );
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
    assert!(result.1);
    assert!(result.0.contains("no touching"));
}

#[tokio::test]
async fn stop_hook_forces_continuation_once() {
    let dir = tempdir().unwrap();
    let flag = dir.path().join("block-once");
    let flag_display = flag.display().to_string();
    // Block the first Stop; after the flag exists, allow it.
    let cmd = format!(
        "if [ -f {flag_display} ]; then exit 0; else touch {flag_display}; echo '{{\"decision\":\"block\",\"reason\":\"say goodbye first\"}}'; fi"
    );
    let hooks = vec![HookDef {
        event: vak_hooks::HookEvent::Stop,
        matcher: None,
        command: cmd,
        timeout_ms: 5000,
    }];
    let mut agent = build(
        vec![text_msg("first attempt"), text_msg("goodbye")],
        Some(hooks),
    );

    let outcome = agent
        .run(
            "go",
            &Default::default(),
            CancellationToken::new(),
            mpsc::channel(64).0,
        )
        .await;
    match outcome {
        TurnOutcome::Completed { response } => {
            assert_eq!(response.text_content(), "goodbye");
        }
        other => panic!("expected completed after continuation, got {other:?}"),
    }
    let session = agent.session.lock().await;
    let texts: Vec<String> = session
        .derive_messages()
        .iter()
        .filter(|m| m.text_content().contains("[stop-hook]"))
        .map(|m| m.text_content())
        .collect();
    assert_eq!(texts.len(), 1, "continuation message must be logged once");
}
