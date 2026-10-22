//! Explicit Topic focus is admitted as a whole-turn human choice, independent
//! of automatic placement, and its message metadata remains replayable.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::collections::VecDeque;
use std::sync::{Arc, Mutex};

use tokio_util::sync::CancellationToken;
use vak_core::Core;
use vak_llm::stream;
use vak_llm::types::{AssistantMessage, ChatRequest, ContentBlock, StopReason, Usage};
use vak_llm::{EventStream, LlmError, Provider};
use vak_session::work_threads::{PlacementSource, TopicStore};

struct Scripted {
    capacity_key: crate::support::CapacityKey,
    responses: Mutex<VecDeque<AssistantMessage>>,
    requests: Mutex<Vec<ChatRequest>>,
}

impl Scripted {
    fn new(responses: Vec<AssistantMessage>) -> Self {
        Self {
            capacity_key: crate::support::CapacityKey::default(),
            responses: Mutex::new(VecDeque::from(responses)),
            requests: Mutex::new(Vec::new()),
        }
    }
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
            Some(message) => {
                sink.push(stream::StreamEvent::Start {
                    partial: message.clone(),
                });
                sink.close_message(message).await;
            }
            None => {
                sink.close_error(LlmError::Parse("no scripted response".into()))
                    .await
            }
        }
        Ok(rx)
    }
}

fn answer(text: &str) -> AssistantMessage {
    AssistantMessage {
        content: vec![ContentBlock::text(text)],
        stop_reason: StopReason::EndTurn,
        usage: Usage::default(),
        model: "test-model".into(),
        response_id: None,
        applied: None,
    }
}

fn core_with_provider(
    config: &str,
    responses: Vec<AssistantMessage>,
) -> (Core, tempfile::TempDir, Arc<Scripted>) {
    let dir = tempfile::tempdir().unwrap();
    let cwd = dir.path().to_path_buf();
    std::fs::create_dir_all(cwd.join(".vak")).unwrap();
    std::fs::write(cwd.join(".vak/config.toml"), config).unwrap();
    vak_config::paths::isolate_home_for_tests();
    let core = Core::new_with_trust(cwd, true).unwrap();
    core.set_shared_scope(vak_config::scope::SharedScope::new(dir.path().join("home")));
    let provider = Arc::new(Scripted::new(responses));
    core.set_provider_instance(provider.clone());
    (core, dir, provider)
}

fn topic_trace(session_id: &str, turn_id: &str) -> vak_session::trace::TraceKey {
    use vak_session::trace::{self, Cause};
    vak_session::trace::TraceKey::root(
        trace::local::tenant(),
        trace::local::space(std::path::Path::new("/tmp/topics-core-test")),
        trace::local::agent("vak"),
        Cause::User {
            request_id: uuid::Uuid::now_v7().hyphenated().to_string(),
        },
    )
    .in_turn(session_id, turn_id)
    .acting(trace::local::local_owner(), None)
}

fn focused_message(id: &str, title: &str) -> vak_session::MessageRecord {
    vak_session::MessageRecord {
        message: vak_llm::Message {
            role: vak_llm::Role::User,
            content: vec![
                ContentBlock::text("Please continue the current request."),
                ContentBlock::text(format!("Continue in Topic: {title}")),
                ContentBlock::text("Preserve this last request detail."),
            ],
        },
        meta: Some(vak_session::types::MessageMeta {
            topic: Some(vak_session::types::TopicFocus {
                id: id.into(),
                title: title.into(),
                block: 1,
            }),
            ..Default::default()
        }),
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn explicit_focus_is_a_replayable_human_assignment_even_when_automatic_is_off() {
    let (core, _dir, provider) = core_with_provider(
        "[stop_policy]\nenabled = false\n",
        vec![answer("I will continue the request.")],
    );
    let session = core.start_session().await.unwrap();
    let session_id = session.header().unwrap().session_id.clone();
    let store = TopicStore::new(&core.scope(), &session_id).unwrap();
    let setup_turn = uuid::Uuid::now_v7().hyphenated().to_string();
    let setup_trace = topic_trace(&session_id, &setup_turn);
    let title = "Field review";
    let topic_id = store.create(title, false, &setup_trace).unwrap();
    store.set_automatic(false, &setup_trace).unwrap();

    let (events, _rx) = tokio::sync::mpsc::channel(64);
    let (_, session) = core
        .run_turn_with_message(
            session,
            focused_message(topic_id.as_str(), title),
            CancellationToken::new(),
            None,
            None,
            None,
            events,
        )
        .await
        .unwrap();

    let snapshot = store.snapshot().unwrap();
    assert!(!snapshot.automatic);
    assert_eq!(snapshot.placements.len(), 1);
    let placement = &snapshot.placements[0];
    assert_eq!(placement.topic_ids, vec![topic_id.clone()]);
    assert_eq!(placement.source, PlacementSource::Human);
    assert!(placement.whole_turn);
    assert_eq!(placement.outcome, "This request");
    assert_eq!(placement.actor, vak_session::trace::local::local_owner());
    assert_eq!(
        placement.trace.session_id().as_deref(),
        Some(session_id.as_str())
    );
    assert_eq!(
        placement
            .trace
            .turn
            .unwrap()
            .uuid()
            .hyphenated()
            .to_string(),
        placement.turn_id
    );
    let directive = session
        .chain_to_root()
        .into_iter()
        .find(|entry| entry.is_directive())
        .unwrap();
    assert_eq!(directive.id, placement.turn_id);
    let vak_session::EntryPayload::Message(record) = &directive.payload else {
        panic!("directive is a message record");
    };
    let metadata = record.meta.as_ref().unwrap();
    assert_eq!(metadata.topic.as_ref().unwrap().id, topic_id.as_str());

    let replayed = session.derive_messages();
    assert!(replayed.iter().any(|message| {
        message.role == vak_llm::Role::User
            && message
                .text_content()
                .contains("Please continue the current request.")
            && message
                .text_content()
                .contains("Continue in Topic: Field review")
            && message
                .text_content()
                .contains("Preserve this last request detail.")
    }));
    let intent = session
        .chain_to_root()
        .into_iter()
        .find_map(|entry| match &entry.payload {
            vak_session::EntryPayload::Intent(record) => Some(record.as_ref()),
            _ => None,
        })
        .expect("the admitted turn records its request reading");
    let reading_text = intent
        .strands
        .iter()
        .map(|strand| strand.text.as_str())
        .collect::<Vec<_>>()
        .join("\n");
    assert!(
        reading_text.contains("continue the current request"),
        "intent reading should preserve the typed request text: {reading_text:?}"
    );
    assert!(
        reading_text.contains("Preserve this last request detail"),
        "intent reading should preserve the final typed detail: {reading_text:?}"
    );
    assert!(
        !reading_text.contains("Continue in Topic: Field review"),
        "the runtime note is excluded from the request reading: {reading_text}"
    );
    let requests = provider.requests.lock().unwrap();
    assert!(!requests.is_empty());
    assert!(requests.iter().any(|request| {
        request.messages.iter().any(|message| {
            message.role == vak_llm::Role::User
                && message
                    .text_content()
                    .contains("Continue in Topic: Field review")
        })
    }));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn archived_focus_is_refused_before_provider_dispatch() {
    let (core, _dir, provider) = core_with_provider(
        "[stop_policy]\nenabled = false\n",
        vec![answer("This must not be used.")],
    );
    let session = core.start_session().await.unwrap();
    let session_id = session.header().unwrap().session_id.clone();
    let store = TopicStore::new(&core.scope(), &session_id).unwrap();
    let setup_turn = uuid::Uuid::now_v7().hyphenated().to_string();
    let setup_trace = topic_trace(&session_id, &setup_turn);
    let title = "Closed topic";
    let topic_id = store.create(title, false, &setup_trace).unwrap();
    store.archive(&topic_id, &setup_trace).unwrap();

    let (events, _rx) = tokio::sync::mpsc::channel(64);
    let result = core
        .run_turn_with_message(
            session,
            focused_message(topic_id.as_str(), title),
            CancellationToken::new(),
            None,
            None,
            None,
            events,
        )
        .await;

    assert!(matches!(result, Err(vak_core::CoreError::InvalidConfig(_))));
    assert!(provider.requests.lock().unwrap().is_empty());
    assert!(store.snapshot().unwrap().placements.is_empty());
}

/// The default-on contract reaches the real Core loop, not only standalone
/// Agent fixtures. Unrelated scripts explicitly disable automatic Topics.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn automatic_organisation_is_checked_before_core_accepts_an_answer() {
    let mut placement = answer("");
    placement.stop_reason = StopReason::ToolUse;
    placement.content = vec![ContentBlock::ToolUse {
        id: "organize".into(),
        name: "topics".into(),
        input: serde_json::json!({
            "action":"create", "title":"Market research", "outcomes":["Market summary"]
        }),
    }];
    let (core, _dir, provider) = core_with_provider(
        "[stop_policy]\nenabled = false\n",
        vec![answer("The summary."), placement, answer("The summary.")],
    );
    let session = core.start_session().await.unwrap();
    let session_id = session.header().unwrap().session_id.clone();
    let (tx, mut rx) = tokio::sync::mpsc::channel(128);
    let drain = tokio::spawn(async move { while rx.recv().await.is_some() {} });
    let (outcome, session) = core
        .run_turn_with(
            session,
            "Continue the research",
            CancellationToken::new(),
            None,
            None,
            None,
            tx,
        )
        .await
        .unwrap();
    drain.await.unwrap();
    assert!(
        matches!(outcome, vak_agent::TurnOutcome::Completed { .. }),
        "{outcome:?}"
    );
    let snapshot = TopicStore::new(&core.scope(), &session_id)
        .unwrap()
        .snapshot()
        .unwrap();
    assert!(snapshot.automatic);
    assert_eq!(snapshot.topics.len(), 1);
    assert_eq!(snapshot.placements.len(), 1);
    assert_eq!(provider.requests.lock().unwrap().len(), 3);
    assert!(session.explanation().iter().any(|(_, step)| matches!(
        step,
        vak_session::log::TurnStep::Decision(decision)
            if matches!(&decision.choice, vak_control::Choice::Correct(c)
                if c.cause == vak_control::Cause::TopicsPlacement)
    )));
}
