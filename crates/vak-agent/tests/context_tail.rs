#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

//! Request tail assembly (docs/design/68-context-engine.md §6/§10): the
//! per-turn control block lands as the final text block of the last user
//! message, stays byte-identical across every step of one turn, carries
//! cache breakpoints at the three documented positions, and a change in the
//! stable prefix is surfaced as a `prefix-changed` activity exactly when the
//! digest actually changes.

use std::collections::VecDeque;
use std::sync::{Arc, Mutex};

use tempfile::tempdir;
use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;

use vak_agent::{Agent, AgentConfig, AutoApprove, SteeringQueues, TailInput, TurnOutcome};
use vak_llm::stream;
use vak_llm::types::{
    AssistantMessage, ChatRequest, ContentBlock, Message, Role, StopReason, Usage,
};
use vak_llm::{EventStream, LlmError, Provider};
use vak_permission::{Mode, PermissionEngine};
use vak_session::types::{FrozenContract, MessageRecord, SessionHeader};
use vak_session::{SessionLog, SessionPath};
use vak_tools::bash::BashTool;

/// Records every request it receives (for tail/cache inspection) and
/// replays a fixed queue of responses.
struct Recording {
    requests: Mutex<Vec<ChatRequest>>,
    responses: Mutex<VecDeque<AssistantMessage>>,
}

impl Recording {
    fn new(responses: Vec<AssistantMessage>) -> Self {
        Recording {
            requests: Mutex::new(Vec::new()),
            responses: Mutex::new(responses.into_iter().collect()),
        }
    }

    fn requests(&self) -> Vec<ChatRequest> {
        self.requests.lock().unwrap().clone()
    }
}

#[async_trait::async_trait]
impl Provider for Recording {
    fn name(&self) -> &str {
        "recording"
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
        response_id: None,
    }
}

fn text_msg(t: &str) -> AssistantMessage {
    AssistantMessage {
        content: vec![ContentBlock::text(t)],
        stop_reason: StopReason::EndTurn,
        usage: Usage {
            input_tokens: 50,
            output_tokens: 3,
            ..Default::default()
        },
        model: "test-model".into(),
        response_id: None,
    }
}

fn header(cwd: &std::path::Path) -> SessionHeader {
    SessionHeader {
        agent: None,
        session_id: "tail-test".into(),
        created_at: chrono::Utc::now(),
        cwd: cwd.to_path_buf(),
        parent_session_id: None,
        contract_id: None,
        work_item_id: None,
        conversation: None,
        contract: FrozenContract {
            app_version: "0".into(),
            provider: "recording".into(),
            model: "test-model".into(),
            route_ladder: Vec::new(),
            route_objective: String::new(),
            route_annotations: Vec::new(),
            system_prompt: "sys".into(),
            permission_mode: "workspace-write".into(),
            capabilities: Vec::new(),
            prompt_layers: Vec::new(),
        },
    }
}

fn build_agent(provider: Arc<Recording>, dir: &std::path::Path) -> Agent {
    let home = dir.join(".vak-home");
    std::fs::create_dir_all(&home).unwrap();
    let path = SessionPath::new_session_file(&home, dir, "tail-test");
    let log = SessionLog::create(path, header(dir)).unwrap();
    let mut cfg = AgentConfig::new("sys");
    cfg.model = "test-model".into();
    cfg.mode = Mode::FullAccess;
    cfg.permission = Some(Arc::new(PermissionEngine::default()));
    cfg.approver = Some(Arc::new(AutoApprove));
    cfg.tools = vec![Arc::new(BashTool)];
    cfg.tool_definitions = Some(vak_tools::definitions(&cfg.tools));
    cfg.retry_base_backoff_ms = 1;
    cfg.run_retry_base_backoff_ms = 1;
    cfg.tail = TailInput {
        temporal: "current UTC instant 2026-09-19T00:00:00Z".into(),
        stance: "Provide a clear, direct answer.".into(),
    };
    Agent::new(provider, log, cfg)
}

async fn run(agent: &mut Agent, prompt: &str) -> TurnOutcome {
    let (ev_tx, mut ev_rx) = mpsc::channel(256);
    tokio::spawn(async move { while ev_rx.recv().await.is_some() {} });
    let cancel = CancellationToken::new();
    let steering = SteeringQueues::new();
    agent.run(prompt, &steering, cancel, ev_tx).await
}

/// The tail is appended as a final text block on the turn's DIRECTIVE
/// message — never re-homed onto whatever message a later step happens to
/// end with — so the directive (tail included) stays byte-identical across
/// every step of the same turn: an append-only request
/// (docs/design/68-context-engine.md §6/§7).
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn tail_precedes_the_users_words_and_is_stable_within_a_turn() {
    let dir = tempdir().unwrap();
    let provider = Arc::new(Recording::new(vec![
        bash_call("call-1", "echo hi"),
        text_msg("done"),
    ]));
    let mut agent = build_agent(provider.clone(), dir.path());
    let outcome = run(&mut agent, "run the check").await;
    assert!(matches!(outcome, TurnOutcome::Completed { .. }));

    let requests = provider.requests();
    assert_eq!(requests.len(), 2, "one step per response");

    // Step 1: only the directive exists yet, so it is also the last (and
    // only) message; the tail sits BEFORE it so the user's own words are
    // the last thing the model reads (docs/design/68-context-engine.md
    // §6), and there is no echo because the directive itself follows.
    let step1_directive = requests[0].messages.last().expect("a message").clone();
    assert_eq!(step1_directive.role, Role::User);
    let texts: Vec<&str> = step1_directive
        .content
        .iter()
        .filter_map(|b| match b {
            ContentBlock::Text { text } => Some(text.as_str()),
            _ => None,
        })
        .collect();
    assert_eq!(texts.len(), 2, "tail block then directive: {texts:?}");
    let tail_step_1 = texts[0].to_string();
    assert!(tail_step_1.contains("<turn_context>"));
    assert!(tail_step_1.contains("current UTC instant 2026-09-19T00:00:00Z"));
    assert!(tail_step_1.contains("<stance>"));
    assert!(tail_step_1.contains("Provide a clear, direct answer."));
    assert!(!tail_step_1.contains("<directive>"));
    assert_eq!(texts[1], "run the check");

    // Step 2: the tool call and its result are appended AFTER the
    // directive. The directive -- tail included -- is byte-identical to
    // step 1's own first message, and the tail is never re-attached to the
    // trailing tool-result message.
    assert_eq!(
        requests[1].messages[0], step1_directive,
        "the directive (with its tail) must be byte-identical across steps"
    );
    let step2_last = requests[1].messages.last().unwrap();
    assert!(matches!(
        step2_last.content.first(),
        Some(ContentBlock::ToolResult { .. })
    ));
    assert!(
        !step2_last
            .content
            .iter()
            .any(|b| matches!(b, ContentBlock::Text { .. })),
        "the tail must not be re-homed onto the tool-result message: {:?}",
        step2_last.content
    );
}

/// Append-only requests within a turn (docs/design/68-context-engine.md
/// §7): across every step of one multi-step tool turn, a later request's
/// `system`, `tools`, and its messages up to the length of an earlier
/// request are byte-identical to that earlier request — nothing already
/// sent ever silently changes shape underneath a replayed thinking block.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn later_requests_in_a_turn_are_byte_identical_prefixes_of_earlier_ones() {
    let dir = tempdir().unwrap();
    let provider = Arc::new(Recording::new(vec![
        bash_call("call-1", "echo one"),
        bash_call("call-2", "echo two"),
        bash_call("call-3", "echo three"),
        text_msg("done"),
    ]));
    let mut agent = build_agent(provider.clone(), dir.path());
    let outcome = run(&mut agent, "run three checks").await;
    assert!(matches!(outcome, TurnOutcome::Completed { .. }));

    let requests = provider.requests();
    assert_eq!(requests.len(), 4, "one step per response");
    for k in 0..requests.len() - 1 {
        assert_eq!(
            requests[k + 1].system,
            requests[k].system,
            "system prompt must not change mid-turn (step {k} -> {})",
            k + 1
        );
        assert_eq!(
            requests[k + 1].tools,
            requests[k].tools,
            "tools array must not change mid-turn (step {k} -> {})",
            k + 1
        );
        assert_eq!(
            requests[k + 1].messages[..requests[k].messages.len()],
            requests[k].messages[..],
            "request {}'s messages must carry request {k}'s as a byte-identical prefix",
            k + 1
        );
    }
}

/// Cache breakpoints land after the stable prefix, after the last message of
/// any previous turn, and on the last message of the request being built.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn cache_breakpoints_mark_prefix_previous_turn_and_current_step() {
    let dir = tempdir().unwrap();
    let provider = Arc::new(Recording::new(vec![text_msg("first answer")]));
    let mut agent = build_agent(provider.clone(), dir.path());
    let outcome = run(&mut agent, "first question").await;
    assert!(matches!(outcome, TurnOutcome::Completed { .. }));

    // First-ever turn: no earlier turn exists, so only two positions apply
    // (after the prefix, and on the last — and only — message).
    let first_turn_requests = provider.requests();
    let breakpoints = first_turn_requests[0]
        .cache
        .as_ref()
        .expect("cache hints present")
        .breakpoints
        .clone();
    assert_eq!(
        breakpoints
            .iter()
            .map(|b| b.after_message)
            .collect::<Vec<_>>(),
        vec![None, Some(0)]
    );

    provider
        .responses
        .lock()
        .unwrap()
        .push_back(text_msg("second answer"));
    let outcome = run(&mut agent, "second question").await;
    assert!(matches!(outcome, TurnOutcome::Completed { .. }));

    let all_requests = provider.requests();
    let second_turn_request = all_requests.last().expect("second turn request");
    // messages: [user1, assistant1, user2] — three positions now apply.
    assert_eq!(second_turn_request.messages.len(), 3);
    let hints = second_turn_request
        .cache
        .as_ref()
        .expect("cache hints present");
    assert!(!hints.session_key.is_empty());
    assert_eq!(
        hints
            .breakpoints
            .iter()
            .map(|b| b.after_message)
            .collect::<Vec<_>>(),
        vec![None, Some(1), Some(2)],
        "prefix, end of previous turn, and the current step's last message"
    );
}

/// A digest that differs from the previous receipt's is surfaced as a
/// `prefix-changed` Activity; an unchanged digest across turns writes none.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn prefix_changed_activity_recorded_only_when_digest_changes() {
    let dir = tempdir().unwrap();
    let provider = Arc::new(Recording::new(vec![
        text_msg("first"),
        text_msg("second"),
        text_msg("third"),
    ]));
    let mut agent = build_agent(provider.clone(), dir.path());

    // Turn 1: first digest ever seen — nothing to compare against yet.
    assert!(matches!(
        run(&mut agent, "one").await,
        TurnOutcome::Completed { .. }
    ));
    // Turn 2: same prefix and tools — digest unchanged.
    assert!(matches!(
        run(&mut agent, "two").await,
        TurnOutcome::Completed { .. }
    ));
    {
        let session = agent.session.lock().await;
        assert!(
            session
                .activities()
                .iter()
                .all(|(_, _, activity)| activity.label != "prefix-changed"),
            "an unchanged prefix must never write a prefix-changed activity"
        );
    }

    // Turn 3: the prefix changes — this must be the one and only regression
    // surfaced in the ledger.
    agent.config.system_prefix = "a different system prefix".into();
    assert!(matches!(
        run(&mut agent, "three").await,
        TurnOutcome::Completed { .. }
    ));

    let session = agent.into_session().await;
    let prefix_changed: Vec<_> = session
        .activities()
        .into_iter()
        .filter(|(_, _, activity)| activity.label == "prefix-changed")
        .collect();
    assert_eq!(
        prefix_changed.len(),
        1,
        "exactly one prefix change happened"
    );
    let (_, _, activity) = &prefix_changed[0];
    assert!(activity.data.contains_key("previous"));
    assert!(activity.data.contains_key("current"));
    assert_ne!(activity.data["previous"], activity.data["current"]);

    // Every receipt still carries a digest, and only the third run's usage
    // measured a fresh `prefix_tokens` (the first two share one digest).
    let receipts = session.receipts();
    assert_eq!(receipts.len(), 3);
    assert!(receipts.iter().all(|r| !r.prefix_digest.is_empty()));
    assert!(
        receipts[0].prefix_tokens.is_some(),
        "the first request with a digest measures prefix_tokens"
    );
    assert!(
        receipts[1].prefix_tokens.is_none(),
        "the digest repeats, so it is not re-measured"
    );
    assert!(
        receipts[2].prefix_tokens.is_some(),
        "a new digest is measured again"
    );
}

/// The conversation thread in the assembled request lists only directives
/// the projection leaves out (docs/design/68-context-engine.md §6/§10): a
/// directive hidden behind a reset-with-handoff resurfaces there, but one
/// still present — including the turn's own new directive — is never
/// repeated. (The plan-driven case, a packeted turn, is covered at the
/// session level: `tail_sections(Some(&plan))` in vak-session's tests.)
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn thread_in_the_assembled_request_lists_only_non_verbatim_directives() {
    let dir = tempdir().unwrap();
    let home = dir.path().join(".vak-home");
    std::fs::create_dir_all(&home).unwrap();
    let path = SessionPath::new_session_file(&home, dir.path(), "tail-test");
    let mut log = SessionLog::create(path, header(dir.path())).unwrap();

    log.append_goal_update(vak_intent::GoalUpdate {
        revision: 1,
        relation: vak_intent::GoalRelation::New,
        request: "research on WEF".into(),
        supersedes_revision: None,
        explicit: false,
    })
    .unwrap();
    log.append_message(MessageRecord {
        message: Message::user_text("research on WEF"),
        meta: None,
    })
    .unwrap();
    log.append_message(MessageRecord {
        message: Message::assistant(vec![ContentBlock::text("here are findings")]),
        meta: None,
    })
    .unwrap();
    // Reset-with-handoff after the first turn: everything before it is
    // invisible to the model; the second turn, appended after, stays
    // verbatim.
    log.append_handoff_reset("summary of the WEF research turn".into(), 999)
        .unwrap();
    log.append_goal_update(vak_intent::GoalUpdate {
        revision: 2,
        relation: vak_intent::GoalRelation::AddsTo,
        request: "use python sandbox".into(),
        supersedes_revision: None,
        explicit: false,
    })
    .unwrap();
    log.append_message(MessageRecord {
        message: Message::user_text("use python sandbox"),
        meta: None,
    })
    .unwrap();
    log.append_message(MessageRecord {
        message: Message::assistant(vec![ContentBlock::text("running in sandbox")]),
        meta: None,
    })
    .unwrap();

    let provider = Arc::new(Recording::new(vec![text_msg(
        "the global economy grew modestly",
    )]));
    let mut cfg = AgentConfig::new("sys");
    cfg.model = "test-model".into();
    cfg.mode = Mode::FullAccess;
    cfg.permission = Some(Arc::new(PermissionEngine::default()));
    cfg.approver = Some(Arc::new(AutoApprove));
    cfg.retry_base_backoff_ms = 1;
    cfg.run_retry_base_backoff_ms = 1;
    let mut agent = Agent::new(provider.clone(), log, cfg);

    let outcome = run(&mut agent, "now evaluate global GDP past 5 years").await;
    assert!(matches!(outcome, TurnOutcome::Completed { .. }));

    let requests = provider.requests();
    assert_eq!(requests.len(), 1);
    let last = requests[0].messages.last().unwrap();
    // The tail is the text block before the directive (§6).
    let tail = last
        .content
        .iter()
        .filter_map(|b| match b {
            ContentBlock::Text { text } => Some(text.clone()),
            _ => None,
        })
        .find(|text| text.contains("<conversation_thread"))
        .expect("thread section present for a reset-hidden directive");

    let thread_start = tail.find("<conversation_thread").unwrap_or(0);
    let thread_text = &tail[thread_start..];
    assert!(
        thread_text.contains("research on WEF"),
        "the reset-hidden directive must resurface: {thread_text}"
    );
    assert!(
        !thread_text.contains("use python sandbox"),
        "a directive still verbatim in the working set must not repeat: {thread_text}"
    );
    assert!(
        !thread_text.contains("now evaluate global GDP"),
        "the turn's own directive is already in `messages` and must not repeat: {thread_text}"
    );
}
