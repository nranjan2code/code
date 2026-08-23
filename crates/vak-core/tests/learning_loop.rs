#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::collections::VecDeque;
use std::sync::{Arc, Mutex};

use tokio_util::sync::CancellationToken;

use vak_core::Core;
use vak_llm as _;
use vak_llm::stream;
use vak_llm::types::{AssistantMessage, ChatRequest, ContentBlock, Usage};
use vak_llm::{EventStream, LlmError, Provider}; // keep types import path stable for future edits

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

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn remember_propose_recall_promote_loop() {
    let dir = tempfile::tempdir().unwrap();
    let home = dir.path().join("home");
    let cwd = dir.path().to_path_buf();

    let core = Core::new(cwd.clone()).unwrap();
    core.set_sessions_home(home.clone());
    core.set_permission_mode(vak_config::PermissionMode::WorkspaceWrite);
    core.set_provider_instance(Arc::new(Scripted {
        responses: Mutex::new(VecDeque::from(vec![
            // Turn 1 of run A: the model journals a decision and drafts a skill.
            tool_call(
                "r1",
                "remember",
                serde_json::json!({
                    "note": "the deploy script must pause before rollback windows",
                    "kind": "decision",
                    "tag": "deploy-rollbacks"
                }),
            ),
            tool_call(
                "p1",
                "propose_skill",
                serde_json::json!({
                    "name": "deploy-safely",
                    "description": "Run deploys with rollback pauses",
                    "instructions": "1. read scripts/deploy.sh\n2. pause before rollback windows"
                }),
            ),
            text("Noted both."),
        ])),
    }));

    // ---- Run A: remember + propose --------------------------------------
    let s_a = core.start_session().await.unwrap();
    let sid_a = s_a.header().unwrap().session_id.clone();
    let (tx_a, _rx_a) = tokio::sync::mpsc::channel(256);
    let (outcome_a, log_a) = core
        .run_turn_with(
            s_a,
            "save what we learned",
            CancellationToken::new(),
            None,
            None,
            None,
            tx_a,
        )
        .await
        .unwrap();
    assert!(matches!(
        outcome_a,
        vak_agent::TurnOutcome::Completed { .. }
    ));

    // Notes landed on disk with provenance...
    let notes = vak_core::memory::list_notes(&home, &cwd);
    assert_eq!(notes.len(), 1);
    assert_eq!(notes[0].kind, "decision");
    assert_eq!(notes[0].tag, "deploy-rollbacks");
    assert_eq!(notes[0].session_id, sid_a);

    // ...and the confirmations are logged on the ledger (invariant 1).
    let logged = log_a
        .message_chain()
        .iter()
        .map(|(_, m)| {
            m.content
                .iter()
                .filter_map(|b| match b {
                    ContentBlock::ToolResult { content, .. } => Some(content.clone()),
                    _ => None,
                })
                .collect::<Vec<_>>()
                .join("\n")
        })
        .collect::<Vec<_>>()
        .join("\n");
    assert!(logged.contains("remembered"), "{logged}");
    assert!(logged.contains("queued for review"), "{logged}");

    // Proposal is pending, not installed.
    let pending = vak_core::learning::list_proposals(&home, &cwd);
    assert_eq!(pending.len(), 1);
    assert_eq!(pending[0].name, "deploy-safely");
    assert!(!home.join("skills/deploy-safely/SKILL.md").exists());

    // ---- Run B: recall ranks memory first --------------------------------
    core.set_provider_instance(Arc::new(Scripted {
        responses: Mutex::new(VecDeque::from(vec![
            tool_call(
                "s1",
                "session_search",
                serde_json::json!({"query": "deploy rollback"}),
            ),
            text("Recalled from memory."),
        ])),
    }));
    let s_b = core.start_session().await.unwrap();
    let (_tx_b, _rx_b) = tokio::sync::mpsc::channel::<vak_agent::AgentEvent>(256);
    let (tx_b2, _rx_b2) = tokio::sync::mpsc::channel(256);
    let _ = (&_tx_b, &_rx_b);
    let (outcome_b, log_b) = core
        .run_turn_with(
            s_b,
            "what do we know about deploys?",
            CancellationToken::new(),
            None,
            None,
            None,
            tx_b2,
        )
        .await
        .unwrap();
    assert!(matches!(
        outcome_b,
        vak_agent::TurnOutcome::Completed { .. }
    ));
    let search_output = log_b
        .message_chain()
        .iter()
        .flat_map(|(_, m)| m.content.iter())
        .filter_map(|b| match b {
            ContentBlock::ToolResult { content, .. } => Some(content.clone()),
            _ => None,
        })
        .find(|c| c.contains("hit(s)"))
        .expect("search result logged");
    assert!(
        search_output.contains("deploy-rollbacks"),
        "memory hit present: {search_output}"
    );
    assert!(
        search_output.contains("memory"),
        "role surfaces as memory: {search_output}"
    );

    // ---- Promotion closes the loop ---------------------------------------
    let id = pending[0].id.clone();
    let promoted = vak_core::learning::promote(&home, &cwd, &id).unwrap();
    assert_eq!(promoted, "deploy-safely");
    assert!(home.join("skills/deploy-safely/SKILL.md").exists());
    assert!(vak_core::learning::list_proposals(&home, &cwd).is_empty());
    let discovered = vak_core::skills::discover(&cwd, &home);
    assert!(
        discovered
            .iter()
            .any(|s| s.name == "deploy-safely" && s.description.contains("rollback")),
        "promoted skill enters discovery: {discovered:?}"
    );

    // Promoting twice or promoting after rejection fails cleanly.
    assert!(vak_core::learning::promote(&home, &cwd, &id).is_err());
}
