//! A saved Agent's definition is read every turn: an edit reaches the next
//! turn of a conversation that already exists, while the session header
//! keeps the identity as admitted (docs/design/45-prompt-layers.md,
//! *Per-turn resolution*).

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

mod support;

use std::collections::VecDeque;
use std::sync::{Arc, Mutex};

use tokio_util::sync::CancellationToken;

use vak_core::Core;
use vak_llm::stream;
use vak_llm::types::{AssistantMessage, ChatRequest, ContentBlock, StopReason, Usage};
use vak_llm::{EventStream, LlmError, Provider};

struct Scripted {
    capacity_key: crate::support::CapacityKey,
    responses: Mutex<VecDeque<AssistantMessage>>,
    requests: Mutex<Vec<ChatRequest>>,
}

#[async_trait::async_trait]
impl Provider for Scripted {
    fn name(&self) -> &str {
        "scripted"
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
        capacity_key: crate::support::CapacityKey::default(),
        responses: Mutex::new(VecDeque::from([text("one"), text("two")])),
        requests: Mutex::new(Vec::new()),
    });
    let core = Core::new_with_trust(cwd.clone(), true)
        .unwrap()
        .with_agent_identity(Some(identity("Audit carefully", 1)));
    core.set_shared_scope(vak_config::scope::SharedScope::new(dir.path().join("home")));
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

fn write_lifecycle(path: &std::path::Path, lifecycle: &str) {
    let definition = serde_json::json!([{
        "id": "auditor", "revision": 3, "lifecycle": lifecycle,
        "name": "Auditor", "character": "vak", "personality": "Focused",
        "behaviour": "Analytical", "responsibilities": "Auditing",
        "instructions": "Audit carefully", "animation": "subtle", "voice": "default"
    }]);
    std::fs::create_dir_all(path.join(".vak")).unwrap();
    std::fs::write(path.join(".vak/agents.json"), definition.to_string()).unwrap();
}

/// A custom Agent's `Core` runs in its own workspace under the base, and its
/// definition lives in the base's project layer. Pausing it refuses the next
/// turn before any model call, and setting it active again resumes the same
/// conversation.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_paused_agent_refuses_its_next_turn_and_resumes_when_active() {
    let dir = tempfile::tempdir().unwrap();
    let base = dir.path().to_path_buf();
    std::fs::create_dir_all(base.join(".vak")).unwrap();
    std::fs::write(
        base.join(".vak/config.toml"),
        "[memory]\nreflection = false\n\n[stop_policy]\nenabled = false\n",
    )
    .unwrap();
    vak_config::paths::isolate_home_for_tests();
    vak_core::trust::mark_trusted(&base).unwrap();
    write_lifecycle(&base, "active");
    let workspace = vak_config::paths::agent_workspace(&base, "auditor");
    std::fs::create_dir_all(workspace.join(".vak")).unwrap();

    let provider = Arc::new(Scripted {
        capacity_key: crate::support::CapacityKey::default(),
        responses: Mutex::new(VecDeque::from([text("one"), text("two")])),
        requests: Mutex::new(Vec::new()),
    });
    let core = Core::new_with_trust(workspace.clone(), true)
        .unwrap()
        .with_agent_identity(Some(identity("Audit carefully", 3)));
    core.set_shared_scope(vak_config::scope::SharedScope::new(dir.path().join("home")));
    core.set_provider_instance(provider.clone());

    let session = core.start_session().await.unwrap();
    let (tx, _rx) = tokio::sync::mpsc::channel(64);
    let (_, session) = core
        .run_turn_with(
            session,
            "one",
            CancellationToken::new(),
            None,
            None,
            None,
            tx,
        )
        .await
        .unwrap();
    let sent = provider.requests.lock().unwrap().len();
    let session_id = session.header().unwrap().session_id.clone();

    write_lifecycle(&base, "paused");
    let (tx, _rx) = tokio::sync::mpsc::channel(64);
    let refused = core
        .run_turn_with(
            session,
            "two",
            CancellationToken::new(),
            None,
            None,
            None,
            tx,
        )
        .await;
    let error = refused
        .err()
        .expect("a paused Agent must refuse the turn")
        .to_string();
    assert!(
        error.contains("paused") && error.contains("settings"),
        "{error}"
    );
    assert_eq!(
        provider.requests.lock().unwrap().len(),
        sent,
        "a refused turn makes no model call"
    );

    write_lifecycle(&base, "active");
    let session = core.open_session(&session_id).await.unwrap();
    let (tx, _rx) = tokio::sync::mpsc::channel(64);
    core.run_turn_with(
        session,
        "two",
        CancellationToken::new(),
        None,
        None,
        None,
        tx,
    )
    .await
    .expect("an active Agent takes the turn again");
}
