//! The engagement governs the turn: what `vak-intent` decides reaches the
//! runtime knobs it was designed for (docs/design/47-commitment-kernel.md,
//! *What the review changed*). Each test here pins one seam that was
//! computed and never consumed before resolver version 2.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

mod support;

use std::collections::VecDeque;
use std::sync::{Arc, Mutex};

use tokio_util::sync::CancellationToken;

use vak_core::Core;
use vak_llm::stream;
use vak_llm::types::{AssistantMessage, ChatRequest, ContentBlock, Usage};
use vak_llm::{EventStream, LlmError, Provider};

struct Scripted {
    capacity_key: crate::support::CapacityKey,
    responses: Mutex<VecDeque<AssistantMessage>>,
    /// Every request this provider was sent, so a test can read what the
    /// model actually saw.
    requests: Mutex<Vec<ChatRequest>>,
}

impl Scripted {
    fn new(responses: Vec<AssistantMessage>) -> Self {
        Scripted {
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
        usage: Usage::default(),
        model: "test-model".into(),
        response_id: None,
    }
}

fn core_with(config: &str, responses: Vec<AssistantMessage>) -> (Core, std::path::PathBuf) {
    let (core, cwd, _) = core_with_provider(config, responses);
    (core, cwd)
}

fn core_with_provider(
    config: &str,
    responses: Vec<AssistantMessage>,
) -> (Core, std::path::PathBuf, Arc<Scripted>) {
    let dir = tempfile::tempdir().unwrap();
    let cwd = dir.path().to_path_buf();
    let _ = std::fs::create_dir_all(cwd.join(".vak"));
    std::fs::write(cwd.join(".vak/config.toml"), config).unwrap();
    vak_config::paths::isolate_home_for_tests();
    let core = Core::new_with_trust(cwd.clone(), true).unwrap();
    core.set_sessions_home(dir.path().join("home"));
    core.set_permission_mode(vak_config::PermissionMode::WorkspaceWrite);
    let provider = Arc::new(Scripted::new(responses));
    core.set_provider_instance(provider.clone());
    std::mem::forget(dir);
    (core, cwd, provider)
}

/// `ContextProfile::Working`: a verifying (or modifying) turn sees what
/// changed in the workspace since the session began, from the ledger, in
/// its tail; a question does not pay for it.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_working_turn_sees_the_workspace_delta_and_a_question_does_not() {
    // The stop gate is off: this test is about the tail, and a scripted
    // provider cannot produce the execution receipt a verify turn owes.
    let (core, cwd, provider) = core_with_provider(
        "[memory]\nreflection = false\n\n[stop_policy]\nenabled = false\n",
        vec![text("hello"), text("it is a note"), text("checked")],
    );
    // The model steps of one turn: every request whose last message is the
    // user's (the judge's transcript request is not one).
    let step_texts = |from: usize| -> Vec<String> {
        provider
            .requests
            .lock()
            .unwrap()
            .iter()
            .skip(from)
            .filter(|request| {
                request
                    .messages
                    .last()
                    .is_some_and(|m| m.role == vak_llm::Role::User)
            })
            .map(|request| {
                request
                    .messages
                    .iter()
                    .map(|m| m.text_content())
                    .collect::<Vec<_>>()
                    .join("\n")
            })
            .collect()
    };
    std::fs::write(cwd.join("notes.md"), "seed\n").unwrap();
    let session = core.start_session().await.unwrap();
    let (tx, _rx) = tokio::sync::mpsc::channel(64);
    let (_, session) = core
        .run_turn_with(
            session,
            "hi",
            CancellationToken::new(),
            None,
            None,
            None,
            tx,
        )
        .await
        .unwrap();

    // The workspace changes between turns (the session's own tools would do
    // this; the change is what matters, not who made it).
    std::fs::write(cwd.join("notes.md"), "seed\nedited\n").unwrap();

    let before = provider.requests.lock().unwrap().len();
    let (tx, _rx) = tokio::sync::mpsc::channel(64);
    let (_, session) = core
        .run_turn_with(
            session,
            "what is in the notes file?",
            CancellationToken::new(),
            None,
            None,
            None,
            tx,
        )
        .await
        .unwrap();
    let question_steps = step_texts(before);
    assert!(!question_steps.is_empty());
    assert!(
        question_steps
            .iter()
            .all(|text| !text.contains("<workspace_delta>")),
        "a question does not carry the delta: {question_steps:?}"
    );

    let before = provider.requests.lock().unwrap().len();
    let (tx, _rx) = tokio::sync::mpsc::channel(64);
    let (_, session) = core
        .run_turn_with(
            session,
            "check the notes file",
            CancellationToken::new(),
            None,
            None,
            None,
            tx,
        )
        .await
        .unwrap();
    let working_steps = step_texts(before);
    assert!(
        working_steps
            .iter()
            .any(|text| text.contains("<workspace_delta>") && text.contains("notes.md")),
        "a working turn sees what the session changed: {working_steps:?}"
    );
    // And the bytes the model saw are in the ledger.
    let logged = session.chain_to_root().iter().any(|entry| {
        matches!(&entry.payload, vak_session::EntryPayload::Activity(activity)
            if activity.data.get("section").map(String::as_str) == Some("workspace_delta")
                && activity.detail.as_deref().is_some_and(|d| d.contains("notes.md")))
    });
    assert!(logged, "the delta is a ledger entry");
}

/// Invariant 10: an image turn is served only by a leg declared able to see
/// it. With hints declared and no capable leg, the turn fails typed rather
/// than quietly sending the image to a text-only model.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn an_unsupported_modality_fails_typed_instead_of_dropping_the_image() {
    let (core, _cwd) = core_with(
        "[memory]\nreflection = false\n\n[route]\nmodality_hints = [\"vision\"]\n",
        vec![text("I see nothing")],
    );
    let session = core.start_session().await.unwrap();
    let (tx, _rx) = tokio::sync::mpsc::channel(64);
    let prompt = vak_llm::Message {
        role: vak_llm::Role::User,
        content: vec![
            ContentBlock::text("what is in this picture"),
            ContentBlock::image_base64("image/png", "iVBORw0KGgo="),
        ],
    };
    let result = core
        .run_turn_with_message(
            session,
            vak_session::MessageRecord {
                message: prompt,
                meta: None,
            },
            CancellationToken::new(),
            None,
            None,
            None,
            tx,
        )
        .await;
    match result {
        Err(vak_core::CoreError::UnsupportedModality { modalities, .. }) => {
            assert!(modalities.contains("image"), "{modalities}");
        }
        other => panic!(
            "expected a typed modality error, got {:?}",
            other.map(|_| ())
        ),
    }

    // With no hints declared, every leg is assumed capable and the turn runs.
    let (core, _cwd) = core_with("[memory]\nreflection = false\n", vec![text("I see a cat")]);
    let session = core.start_session().await.unwrap();
    let (tx, _rx) = tokio::sync::mpsc::channel(64);
    let prompt = vak_llm::Message {
        role: vak_llm::Role::User,
        content: vec![
            ContentBlock::text("what is in this picture"),
            ContentBlock::image_base64("image/png", "iVBORw0KGgo="),
        ],
    };
    let (outcome, _) = core
        .run_turn_with_message(
            session,
            vak_session::MessageRecord {
                message: prompt,
                meta: None,
            },
            CancellationToken::new(),
            None,
            None,
            None,
            tx,
        )
        .await
        .unwrap();
    assert!(matches!(outcome, vak_agent::TurnOutcome::Completed { .. }));
}

/// A turn's intent entry carries its strands, and the next turn's lineage
/// sees them as open threads.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn strands_are_recorded_and_become_open_threads() {
    let (core, _cwd) = core_with(
        "[memory]\nreflection = false\n",
        vec![text("done"), text("done again")],
    );
    let session = core.start_session().await.unwrap();
    let (tx, _rx) = tokio::sync::mpsc::channel(64);
    let (_, session) = core
        .run_turn_with(
            session,
            "explain the parser, then summarise the changelog",
            CancellationToken::new(),
            None,
            None,
            None,
            tx,
        )
        .await
        .unwrap();
    let record = session
        .chain_to_root()
        .into_iter()
        .rev()
        .find_map(|entry| match &entry.payload {
            vak_session::EntryPayload::Intent(record) => Some(record.as_ref().clone()),
            _ => None,
        })
        .expect("an intent entry");
    assert_eq!(record.strands.len(), 2, "{:?}", record.strands);
    assert!(
        record
            .model_visible
            .as_deref()
            .is_some_and(|note| note.contains("2 parts")),
        "{:?}",
        record.model_visible
    );

    let threads = vak_core::intent::open_threads(&session);
    assert_eq!(threads.len(), 2);
    assert!(threads.iter().any(|t| t.keywords.contains("parser")));

    // "now explain it in more detail" continues a thread rather than opening
    // a twin.
    let (tx, _rx) = tokio::sync::mpsc::channel(64);
    let (_, session) = core
        .run_turn_with(
            session,
            "now explain it in more detail",
            CancellationToken::new(),
            None,
            None,
            None,
            tx,
        )
        .await
        .unwrap();
    let latest = session
        .chain_to_root()
        .into_iter()
        .rev()
        .find_map(|entry| match &entry.payload {
            vak_session::EntryPayload::Intent(record) => Some(record.as_ref().clone()),
            _ => None,
        })
        .unwrap();
    assert!(
        matches!(
            latest.strands[0].lineage,
            vak_intent::Lineage::Continues { .. }
        ),
        "{:?}",
        latest.strands[0].lineage
    );
}

/// `Defer`: a gate nobody can answer is parked in the inbox and suspends the
/// commitment, and the turn still fails closed.
#[tokio::test]
async fn a_deferred_gate_reaches_the_inbox_and_suspends_the_commitment() {
    use vak_agent::Approver;
    let dir = tempfile::tempdir().unwrap();
    let sessions_home = dir.path().join("sessions");
    let shared_home = dir.path().join("shared");
    let ledger = vak_commit::CommitmentLedger::new(&sessions_home);
    let reading = vak_intent::Reading {
        horizon: vak_intent::Horizon::Durable,
        stakes: vak_intent::Stakes::Costly,
        ..vak_intent::Reading::general()
    };
    let commitment_id = ledger
        .open_commitment(vak_commit::spec_from_reading(
            "rotate the production keys",
            reading,
            Vec::new(),
            dir.path().to_path_buf(),
            vak_commit::Economics::default(),
        ))
        .unwrap();

    struct Nobody;
    #[async_trait::async_trait]
    impl Approver for Nobody {
        async fn approve(&self, _: &str, _: &str, _: &str) -> bool {
            panic!("an unanswerable approver must never be asked")
        }
        fn answerable(&self) -> bool {
            false
        }
    }
    let approver = vak_core::intent::DeferringApprover::new(
        Some(Arc::new(Nobody)),
        shared_home.clone(),
        sessions_home.clone(),
        "s1".into(),
        commitment_id.clone(),
        vak_intent::Escalation::WaitIndefinitely,
    );
    assert!(!approver.answerable());
    let allowed = approver
        .approve("bash", "{\"command\":\"rotate-keys\"}", "irreversible")
        .await;
    assert!(!allowed, "the turn fails closed");

    let commitment = ledger.get(&commitment_id).unwrap().unwrap();
    assert!(
        matches!(
            commitment.suspension,
            Some(vak_commit::Suspension::Human { .. })
        ),
        "{:?}",
        commitment.suspension
    );
    let inbox = vak_core::inbox::unread(&shared_home);
    assert_eq!(inbox.len(), 1);
    assert_eq!(inbox[0].kind, vak_core::inbox::Kind::ApprovalPending);
    assert!(inbox[0].body.contains("rotate-keys"));
}
