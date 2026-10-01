//! A saved Agent's definition is read every turn: an edit reaches the next
//! turn of a conversation that already exists, while the session header
//! keeps the identity as admitted (docs/design/45-prompt-layers.md,
//! *Per-turn resolution*).

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::collections::VecDeque;
use std::sync::{Arc, Mutex};

use tokio_util::sync::CancellationToken;

use vak_core::Core;
use vak_llm::stream;
use vak_llm::types::{AssistantMessage, ChatRequest, ContentBlock, StopReason, Usage};
use vak_llm::{EventStream, LlmError, Provider};

struct Scripted {
    responses: Mutex<VecDeque<AssistantMessage>>,
    requests: Mutex<Vec<ChatRequest>>,
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
        stop_reason: StopReason::EndTurn,
        usage: Usage::default(),
        model: "test-model".into(),
        response_id: None,
    }
}

fn identity(instructions: &str, revision: u64) -> vak_session::types::AgentIdentity {
    vak_session::types::AgentIdentity {
        id: "auditor".into(),
        revision,
        name: "Auditor".into(),
        character: "vak".into(),
        personality: "Focused".into(),
        animation: "subtle".into(),
        voice: "default".into(),
        behaviour: "Analytical".into(),
        responsibilities: "Auditing".into(),
        instructions: instructions.into(),
    }
}

fn write_definition(cwd: &std::path::Path, instructions: &str, revision: u64) {
    let definition = serde_json::json!([{
        "id": "auditor", "revision": revision, "lifecycle": "active",
        "name": "Auditor", "character": "vak", "personality": "Focused",
        "behaviour": "Analytical", "responsibilities": "Auditing",
        "instructions": instructions, "animation": "subtle", "voice": "default"
    }]);
    std::fs::write(cwd.join(".vak/agents.json"), definition.to_string()).unwrap();
}

/// The system prompt of every main-turn step: a side dispatch has its own.
fn turn_systems(provider: &Scripted) -> Vec<String> {
    provider
        .requests
        .lock()
        .unwrap()
        .iter()
        .filter_map(|request| request.system.clone())
        .filter(|system| system.contains("Capability contract"))
        .collect()
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn editing_an_agent_changes_the_next_turn_of_its_open_conversation() {
    let dir = tempfile::tempdir().unwrap();
    let cwd = dir.path().to_path_buf();
    std::fs::create_dir_all(cwd.join(".vak")).unwrap();
    std::fs::write(
        cwd.join(".vak/config.toml"),
        "[memory]\nreflection = false\n\n[stop_policy]\nenabled = false\n",
    )
    .unwrap();
    vak_config::paths::isolate_home_for_tests();
    write_definition(&cwd, "Audit carefully", 1);

    let provider = Arc::new(Scripted {
        responses: Mutex::new(VecDeque::from([text("one"), text("two")])),
        requests: Mutex::new(Vec::new()),
    });
    let core = Core::new_with_trust(cwd.clone(), true)
        .unwrap()
        .with_agent_identity(Some(identity("Audit carefully", 1)));
    core.set_sessions_home(dir.path().join("home"));
    core.set_provider_instance(provider.clone());

    let session = core.start_session().await.unwrap();
    let (tx, _rx) = tokio::sync::mpsc::channel(64);
    let (_, session) = core
        .run_turn_with(
            session,
            "first question",
            CancellationToken::new(),
            None,
            None,
            None,
            tx,
        )
        .await
        .unwrap();

    write_definition(&cwd, "Answer in a table", 2);
    let (tx, _rx) = tokio::sync::mpsc::channel(64);
    let (_, session) = core
        .run_turn_with(
            session,
            "second question",
            CancellationToken::new(),
            None,
            None,
            None,
            tx,
        )
        .await
        .unwrap();

    let systems = turn_systems(&provider);
    assert_eq!(systems.len(), 2, "one step per turn: {systems:?}");
    assert!(systems[0].contains("Audit carefully"), "{}", systems[0]);
    assert!(systems[1].contains("Answer in a table"), "{}", systems[1]);
    assert!(!systems[1].contains("Audit carefully"), "{}", systems[1]);

    let admitted = session.header().and_then(|h| h.agent.clone()).unwrap();
    assert_eq!(
        (admitted.revision, admitted.instructions.as_str()),
        (1, "Audit carefully"),
        "the header keeps the identity as admitted"
    );
}
