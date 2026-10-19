//! The engagement governs the turn: what `vak-intent` decides reaches the
//! runtime knobs it was designed for (docs/design/47-commitment-kernel.md,
//! *What the review changed*). Each test here pins one seam that was
//! computed and never consumed before resolver version 2.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

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
        applied: None,
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
    core.set_shared_scope(vak_config::scope::SharedScope::new(dir.path().join("home")));
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
        "[stop_policy]\nenabled = false\n",
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
        "[route]\nmodality_hints = [\"vision\"]\n",
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
    let (core, _cwd) = core_with("", vec![text("I see a cat")]);
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
    let (core, _cwd) = core_with("", vec![text("done"), text("done again")]);
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
    // The parts are recorded for audit; the model is never told about parts
    // it was not shown (docs/design/86-model-plans-runtime-checks.md §1).
    assert!(
        record
            .model_visible
            .as_deref()
            .is_none_or(|note| !note.to_lowercase().contains("part")),
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

/// One turn, one id (docs/design/85-turn-graph.md, G0): the directive's
/// entry id is the turn id the intent minted its strands from, every record
/// of the turn names it, and a side-ledger row written under the run joins
/// back to it through its trace key.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn one_turn_one_id_from_intent_to_side_ledgers() {
    let (core, cwd) = core_with("", vec![text("two files")]);
    let session = core.start_session().await.unwrap();
    let (tx, mut rx) = tokio::sync::mpsc::channel(64);
    tokio::spawn(async move { while rx.recv().await.is_some() {} });
    let (_, session) = core
        .run_turn_with(
            session,
            "list the config files",
            CancellationToken::new(),
            None,
            None,
            None,
            tx,
        )
        .await
        .unwrap();
    let chain = session.chain_to_root();
    let directive = chain
        .iter()
        .find(|entry| entry.is_directive())
        .expect("a directive");
    let turn_id = directive.id.clone();
    let intent = chain
        .iter()
        .find(|entry| matches!(entry.payload, vak_session::EntryPayload::Intent(_)))
        .expect("an intent");
    let vak_session::EntryPayload::Intent(record) = &intent.payload else {
        unreachable!()
    };
    assert!(!record.strands.is_empty());
    for strand in &record.strands {
        assert!(
            strand.strand_id.starts_with(&format!("{turn_id}.")),
            "{} is not of turn {turn_id}",
            strand.strand_id
        );
    }
    let first_of_turn = chain
        .iter()
        .position(|entry| entry.id == intent.id)
        .unwrap();
    for entry in &chain[first_of_turn..] {
        assert_eq!(
            entry.at_turn.as_deref(),
            Some(turn_id.as_str()),
            "{:?} does not name its turn",
            entry.payload
        );
    }
    assert!(
        chain[first_of_turn..]
            .iter()
            .any(|entry| matches!(entry.payload, vak_session::EntryPayload::Receipt(_)))
    );

    let evidence = walk(&cwd.join("home"))
        .into_iter()
        .find_map(|path| {
            let dir = path.parent()?;
            dir.ends_with("routing-evidence").then(|| dir.to_path_buf())
        })
        .expect("a routing evidence ledger");
    let row: serde_json::Value = vak_session::chain::RecordChain::at(evidence)
        .read::<serde_json::Value>()
        .into_iter()
        .next()
        .expect("an evidence row");
    let trace: vak_session::trace::TraceKey = serde_json::from_value(row["trace"].clone()).unwrap();
    assert_eq!(
        trace.turn.map(|turn| turn.to_string()),
        Some(format!("trn_{turn_id}"))
    );
    assert_eq!(
        trace.session.map(|session| session.to_string()),
        session
            .header()
            .map(|header| format!("ses_{}", header.session_id))
    );
}

fn walk(dir: &std::path::Path) -> Vec<std::path::PathBuf> {
    let mut out = Vec::new();
    let mut stack = vec![dir.to_path_buf()];
    while let Some(dir) = stack.pop() {
        for entry in std::fs::read_dir(&dir).into_iter().flatten().flatten() {
            let path = entry.path();
            if path.is_dir() {
                stack.push(path);
            } else {
                out.push(path);
            }
        }
    }
    out
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
        async fn approve(&self, _: &str, _: &str, _: &str, _: Option<&str>) -> bool {
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
        .approve(
            "bash",
            "{\"command\":\"rotate-keys\"}",
            "irreversible",
            None,
        )
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
    let inbox = vak_core::inbox::unread(&vak_config::scope::AgentScope::new(&shared_home));
    assert_eq!(inbox.len(), 1);
    assert_eq!(inbox[0].kind, vak_core::inbox::Kind::ApprovalPending);
    assert!(inbox[0].body.contains("rotate-keys"));
}

/// The commitment's context is a fact about the turn, never a reading of its
/// words (docs/design/88-control-plane.md): a request that only sounds
/// durable gets no commitment context, and one asked for with `/until-done`
/// works on a commitment and records `full`.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn the_commitment_context_follows_the_commitment_not_the_words() {
    let (core, _cwd) = core_with(
        "[stop_policy]\nenabled = false\n",
        (0..8).map(|_| text("noted")).collect(),
    );
    let contexts = |session: &vak_session::SessionLog| -> Vec<vak_intent::ContextProfile> {
        session
            .chain_to_root()
            .iter()
            .filter_map(|entry| match &entry.payload {
                vak_session::EntryPayload::Intent(record) => {
                    Some(record.engagement.posture.context)
                }
                _ => None,
            })
            .collect()
    };
    let mut session = core.start_session().await.unwrap();
    for prompt in [
        "check the cloud bill every day and alert me",
        "/until-done check the cloud bill every day and alert me",
    ] {
        let (tx, _rx) = tokio::sync::mpsc::channel(64);
        session = core
            .run_turn_with(
                session,
                prompt,
                CancellationToken::new(),
                None,
                None,
                None,
                tx,
            )
            .await
            .unwrap()
            .1;
    }
    let contexts = contexts(&session);
    assert_eq!(contexts.len(), 2, "{contexts:?}");
    assert_ne!(contexts[0], vak_intent::ContextProfile::Full);
    assert_eq!(contexts[1], vak_intent::ContextProfile::Full);
}

/// C7 (docs/design/88-control-plane.md): a finished turn's facts are read
/// from its decisions, a call is named by the tool the model called, and a
/// refused call is one refusal, never also a failure.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_turns_facts_come_from_its_decisions() {
    let call = AssistantMessage {
        content: vec![ContentBlock::ToolUse {
            id: "toolu_ghost".into(),
            name: "no_such_tool".into(),
            input: serde_json::json!({}),
        }],
        stop_reason: vak_llm::types::StopReason::ToolUse,
        usage: Usage::default(),
        model: "test-model".into(),
        response_id: None,
        applied: None,
    };
    let (core, _cwd) = core_with(
        "[stop_policy]\nenabled = false\n",
        vec![call, text("done"), text("done")],
    );
    let session = core.start_session().await.unwrap();
    let (tx, _rx) = tokio::sync::mpsc::channel(64);
    let (_, session) = core
        .run_turn_with(
            session,
            "do the thing",
            CancellationToken::new(),
            None,
            None,
            None,
            tx,
        )
        .await
        .unwrap();
    let turns = vak_core::measure::turn_facts(&session);
    assert_eq!(turns.len(), 1, "{turns:?}");
    let turn = &turns[0];
    assert!(turn.steps >= 2, "{turn:?}");
    assert_eq!(turn.ending, Some(vak_control::End::Answered));
    assert!(turn.seconds.is_some(), "the admission was read: {turn:?}");
    assert_eq!(
        turn.calls,
        vec![(
            "no_such_tool".to_string(),
            vak_control::CallVerdict::Refused(vak_control::Refusal::Unknown)
        )]
    );
    assert!(turn.failures.is_empty(), "{turn:?}");
    let proposals = vak_judge::measure::promote(&turns);
    assert!(
        proposals
            .iter()
            .any(|p| p.key.ends_with("|tool_refused:no_such_tool:unknown")),
        "{proposals:?}"
    );
}

/// Source retrieval, output delivery and semantic sufficiency are independent.
/// These turns all finish normally, including one with an irrelevant source;
/// none may acquire semantic completion from transport or output existence.
#[tokio::test]
async fn research_requirements_survive_the_real_turn_and_do_not_self_certify() {
    fn call(id: &str, name: &str, input: serde_json::Value) -> AssistantMessage {
        AssistantMessage {
            content: vec![ContentBlock::ToolUse {
                id: id.into(),
                name: name.into(),
                input,
            }],
            stop_reason: vak_llm::types::StopReason::ToolUse,
            ..text("")
        }
    }
    let cases = [
        (
            vec![
                call(
                    "plan-a",
                    "plan",
                    serde_json::json!({"deliverables": [{"name": "analysis", "form": "text", "questions": ["What changed within the requested period?"]}]}),
                ),
                text("There is insufficient evidence to answer."),
            ],
            false,
            "plan-evidence-1-1",
        ),
        // A successful local read is evidence of retrieval, not relevance.
        (
            vec![
                call("read-a", "read", serde_json::json!({"path": "source.txt"})),
                text("A claim about an unrelated subject."),
            ],
            true,
            "retrieved-source-support",
        ),
    ];
    for (responses, retrieved, requirement) in cases {
        let (core, cwd) = core_with("[stop_policy]\nenabled = false\n", responses);
        // Cargo may place artifacts outside the checkout (release staging or
        // CARGO_TARGET_DIR). The real worker is beside this test's deps dir.
        let executable = std::env::current_exe().unwrap();
        let profile = executable.parent().unwrap().parent().unwrap();
        core.set_tool_worker_exe(profile.join(format!("vak{}", std::env::consts::EXE_SUFFIX)));
        std::fs::write(
            cwd.join("source.txt"),
            "This source is about the weather, not the requested analysis.",
        )
        .unwrap();
        let session = core.start_session().await.unwrap();
        let (tx, mut rx) = tokio::sync::mpsc::channel(256);
        let drain = tokio::spawn(async move { while rx.recv().await.is_some() {} });
        let (outcome, session) = core
            .run_turn_with(
                session,
                "Investigate the question using evidence.",
                CancellationToken::new(),
                None,
                None,
                None,
                tx,
            )
            .await
            .unwrap();
        assert!(
            matches!(outcome, vak_agent::TurnOutcome::Completed { .. }),
            "{outcome:?}"
        );
        let evaluation = session
            .chain_to_root()
            .into_iter()
            .rev()
            .find_map(|entry| match &entry.payload {
                vak_session::EntryPayload::Activity(activity)
                    if activity.label == "Outcome evaluation" =>
                {
                    Some(activity.clone())
                }
                _ => None,
            })
            .unwrap();
        assert_ne!(
            evaluation.data.get("completion").map(String::as_str),
            Some("complete"),
            "{evaluation:?}"
        );
        assert_eq!(
            evaluation.data.get("evidence_state").map(String::as_str),
            Some(if retrieved { "fresh" } else { "none" }),
            "{:?}",
            session.derive_messages()
        );
        let evaluations: Vec<vak_intent::RequirementEvaluation> =
            serde_json::from_str(&evaluation.data["evaluation"]).unwrap();
        assert!(
            evaluations
                .iter()
                .any(|row| row.requirement_id == requirement
                    && row.status != vak_intent::RequirementStatus::Met),
            "{evaluations:?}"
        );
        drain.await.unwrap();
    }
}

/// A planned change that read the file it fixes rests on the runtime's own
/// receipts, not on retrieved material: no claim-support requirement is
/// inferred for it (found live 2026-10-19: such a coding turn was marked a
/// partial result).
#[tokio::test]
async fn a_planned_change_that_read_its_file_needs_no_source_support() {
    fn call(id: &str, name: &str, input: serde_json::Value) -> AssistantMessage {
        AssistantMessage {
            content: vec![ContentBlock::ToolUse {
                id: id.into(),
                name: name.into(),
                input,
            }],
            stop_reason: vak_llm::types::StopReason::ToolUse,
            ..text("")
        }
    }
    let (core, cwd) = core_with(
        "[stop_policy]\nenabled = false\n",
        vec![
            call(
                "plan-a",
                "plan",
                serde_json::json!({"deliverables": [{"name": "fix add", "form": "change"}]}),
            ),
            call("read-a", "read", serde_json::json!({"path": "calc.py"})),
            call(
                "edit-a",
                "edit",
                serde_json::json!({"path": "calc.py", "edits": [{"old_text": "a - b", "new_text": "a + b"}]}),
            ),
            text("Fixed add."),
        ],
    );
    core.set_tool_worker_exe(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../target/debug/vak"),
    );
    std::fs::write(cwd.join("calc.py"), "def add(a, b):\n    return a - b\n").unwrap();
    let session = core.start_session().await.unwrap();
    let (tx, mut rx) = tokio::sync::mpsc::channel(256);
    let drain = tokio::spawn(async move { while rx.recv().await.is_some() {} });
    let (outcome, session) = core
        .run_turn_with(
            session,
            "Fix the bug in calc.py.",
            CancellationToken::new(),
            None,
            None,
            None,
            tx,
        )
        .await
        .unwrap();
    assert!(
        matches!(outcome, vak_agent::TurnOutcome::Completed { .. }),
        "{outcome:?}"
    );
    assert!(
        std::fs::read_to_string(cwd.join("calc.py"))
            .unwrap()
            .contains("a + b"),
        "{:?}",
        session.derive_messages()
    );
    let evaluation = session
        .chain_to_root()
        .into_iter()
        .rev()
        .find_map(|entry| match &entry.payload {
            vak_session::EntryPayload::Activity(activity)
                if activity.label == "Outcome evaluation" =>
            {
                Some(activity.clone())
            }
            _ => None,
        })
        .unwrap();
    let evaluations: Vec<vak_intent::RequirementEvaluation> =
        serde_json::from_str(&evaluation.data["evaluation"]).unwrap();
    assert!(
        !evaluations
            .iter()
            .any(|row| row.requirement_id == "retrieved-source-support"),
        "{evaluations:?}"
    );
    assert_eq!(
        evaluation.data.get("evidence_state").map(String::as_str),
        Some("fresh"),
        "the read is still a retrieval: {evaluation:?}"
    );
    drain.await.unwrap();
}
