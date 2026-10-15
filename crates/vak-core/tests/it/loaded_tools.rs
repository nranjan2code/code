//! A tool a conversation's turn called stays loaded for its later turns,
//! read from the ledger, so a restart changes nothing
//! (docs/design/88-control-plane.md §12.1).

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::collections::VecDeque;
use std::sync::{Arc, Mutex};

use tokio_util::sync::CancellationToken;

use vak_core::Core;
use vak_llm::stream;
use vak_llm::types::{AssistantMessage, ChatRequest, ContentBlock, StopReason, Usage};
use vak_llm::{EventStream, LlmError, Provider};

struct Recording {
    capacity_key: crate::support::CapacityKey,
    responses: Mutex<VecDeque<AssistantMessage>>,
    requests: Arc<Mutex<Vec<ChatRequest>>>,
}

#[async_trait::async_trait]
impl Provider for Recording {
    fn name(&self) -> &str {
        "recording"
    }

    fn rate_limit_key(&self) -> String {
        self.capacity_key.0.clone()
    }

    async fn stream(
        &self,
        request: ChatRequest,
        _cancel: CancellationToken,
    ) -> Result<EventStream, LlmError> {
        self.requests.lock().unwrap().push(request);
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

fn reply(content: ContentBlock, stop: StopReason) -> AssistantMessage {
    AssistantMessage {
        content: vec![content],
        stop_reason: stop,
        usage: Usage::default(),
        model: "test-model".into(),
        response_id: None,
        applied: None,
    }
}

fn core(cwd: &std::path::Path, home: &std::path::Path, provider: Arc<Recording>) -> Core {
    let core = Core::new_with_trust(cwd.to_path_buf(), true).unwrap();
    core.set_shared_scope(vak_config::scope::SharedScope::new(home.to_path_buf()));
    core.set_permission_mode(vak_config::PermissionMode::FullAccess);
    core.set_provider_instance(provider);
    core
}

/// Whether `request` offers `tool` loaded (its schema sent, not deferred).
fn loaded(request: &ChatRequest, tool: &str) -> bool {
    request
        .tools
        .iter()
        .any(|definition| definition.name == tool && !definition.defer)
}

#[tokio::test]
async fn a_tool_a_turn_called_is_loaded_after_a_restart() {
    vak_config::paths::isolate_home_for_tests();
    let dir = tempfile::tempdir().unwrap();
    let cwd = dir.path().join("workspace");
    std::fs::create_dir_all(&cwd).unwrap();
    let home = dir.path().join("home");
    let requests = Arc::new(Mutex::new(Vec::new()));
    let provider = |responses: Vec<AssistantMessage>| {
        Arc::new(Recording {
            capacity_key: crate::support::CapacityKey::default(),
            responses: Mutex::new(responses.into()),
            requests: requests.clone(),
        })
    };

    let first = core(
        &cwd,
        &home,
        provider(vec![
            reply(
                ContentBlock::ToolUse {
                    id: "a1".into(),
                    name: "automations".into(),
                    input: serde_json::json!({"action": "list"}),
                },
                StopReason::ToolUse,
            ),
            reply(ContentBlock::text("none yet"), StopReason::EndTurn),
        ]),
    );
    let session = first.start_session().await.unwrap();
    let id = session.header().unwrap().session_id.clone();
    let (events, _rx) = tokio::sync::mpsc::channel(256);
    first
        .run_turn_with(
            session,
            "what runs on its own?",
            CancellationToken::new(),
            None,
            None,
            None,
            events,
        )
        .await
        .unwrap();
    let offered_first = requests.lock().unwrap()[0].clone();
    assert!(
        !loaded(&offered_first, "automations"),
        "deferred until a turn reaches for it"
    );
    drop(first);

    let second = core(
        &cwd,
        &home,
        provider(vec![reply(ContentBlock::text("ok"), StopReason::EndTurn)]),
    );
    let session = second.open_session(&id).await.unwrap();
    let before = requests.lock().unwrap().len();
    let (events, _rx) = tokio::sync::mpsc::channel(256);
    second
        .run_turn_with(
            session,
            "pause it",
            CancellationToken::new(),
            None,
            None,
            None,
            events,
        )
        .await
        .unwrap();
    let request = requests.lock().unwrap()[before].clone();
    assert!(
        loaded(&request, "automations"),
        "a tool an earlier turn called is loaded after a restart: {:?}",
        request
            .tools
            .iter()
            .map(|t| (&t.name, t.defer))
            .collect::<Vec<_>>()
    );
}
