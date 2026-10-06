//! Reach-back (plan M8.3b, docs/design/82-library.md §6): `recall` reads
//! another conversation only when an artifact attached in this one names
//! it; any other conversation is refused before Core is asked.

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

fn recall_call(id: &str, conversation: &str) -> AssistantMessage {
    AssistantMessage {
        content: vec![ContentBlock::ToolUse {
            id: id.into(),
            name: "recall".into(),
            input: serde_json::json!({"query": "harbour", "conversation": conversation}),
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
async fn recall_reaches_only_conversations_attached_artifacts_name() {
    let dir = tempfile::tempdir().unwrap();
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
    let mut log = SessionLog::create(dir.path().join("s.jsonl"), header).unwrap();
    // An earlier message attached an artifact made in conversation "made-it".
    log.append_message(vak_session::MessageRecord {
        message: vak_llm::Message::user_text("Continue the poem."),
        meta: Some(vak_session::MessageMeta {
            artifacts: vec![vak_session::AttachedArtifact {
                block: 0,
                artifact: "art_x".into(),
                name: "Poem".into(),
                path: "poem.md".into(),
                version: "ver_x".into(),
                digest: "d".into(),
                mode: Default::default(),
                conversations: vec!["made-it".into()],
            }],
            ..Default::default()
        }),
    })
    .unwrap();
    let asked = Arc::new(std::sync::Mutex::new(Vec::<String>::new()));
    let seen = asked.clone();
    let mut cfg = AgentConfig::new("sys");
    cfg.tools = vec![Arc::new(vak_tools::recall::RecallTool)];
    cfg.mode = Mode::FullAccess;
    cfg.permission = Some(Arc::new(PermissionEngine::default()));
    cfg.approver = Some(Arc::new(AutoApprove));
    cfg.stop_policy = None;
    cfg.history_recall = Some(Arc::new(move |request, _leaf, _cancel| {
        let seen = seen.clone();
        Box::pin(async move {
            if let vak_tools::RecallRequest::Elsewhere { conversation, .. } = request {
                seen.lock().unwrap().push(conversation.clone());
                return Ok(format!("turns of {conversation}"));
            }
            Err("unexpected".into())
        })
    }));
    let provider = Scripted {
        capacity_key: crate::support::capacity_key(),
        msgs: std::sync::Mutex::new(
            vec![
                recall_call("t1", "someone-elses"),
                recall_call("t2", "made-it"),
                text("Done."),
            ]
            .into(),
        ),
    };
    let mut agent = Agent::new(Arc::new(provider), log, cfg);
    let outcome = agent
        .run(
            "Keep going.",
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
        *asked.lock().unwrap(),
        vec!["made-it".to_string()],
        "only the named one reached Core"
    );
    let session = agent.session.lock().await;
    let result = |id: &str| {
        session
            .message_chain()
            .iter()
            .flat_map(|(_, m)| m.content.iter())
            .find_map(|b| match b {
                ContentBlock::ToolResult {
                    tool_use_id,
                    content,
                    is_error,
                } if tool_use_id == id => Some((content.clone(), *is_error)),
                _ => None,
            })
            .unwrap()
    };
    assert!(result("t1").1, "an unnamed conversation is refused");
    assert_eq!(result("t2"), ("turns of made-it".to_string(), false));
}
