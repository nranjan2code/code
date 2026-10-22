#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

//! Regression: automatic Topics cannot silently be skipped at answer completion.

use std::collections::VecDeque;
use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use serde_json::Value;
use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;

use vak_agent::{Agent, AgentConfig, TurnOutcome};
use vak_llm::stream;
use vak_llm::types::{AssistantMessage, ChatRequest, ContentBlock, StopReason, Usage};
use vak_llm::{EventStream, LlmError, Provider};
use vak_permission::PermissionEngine;
use vak_session::SessionLog;
use vak_session::types::{FrozenContract, SessionHeader};
use vak_session::work_threads::TopicStore;

type RequiredCalls = Arc<Mutex<Vec<Option<String>>>>;
type Fixture = (
    Agent,
    Arc<TopicStore>,
    vak_session::trace::TraceKey,
    RequiredCalls,
);

struct Scripted {
    capacity_key: String,
    responses: Mutex<VecDeque<AssistantMessage>>,
    required: RequiredCalls,
}

#[async_trait]
impl Provider for Scripted {
    fn name(&self) -> &str {
        "scripted"
    }

    fn rate_limit_key(&self) -> String {
        self.capacity_key.clone()
    }

    async fn stream(
        &self,
        request: ChatRequest,
        _cancel: CancellationToken,
    ) -> Result<EventStream, LlmError> {
        self.required.lock().unwrap().push(request.must_call);
        let next = self.responses.lock().unwrap().pop_front();
        let (mut sink, rx) = stream::channel(16);
        match next {
            Some(m) => sink.close_message(m).await,
            // A finite fixture has no next response. This is terminal test
            // input, not a transient parse failure for endurance to retry.
            None => {
                sink.close_error(LlmError::InvalidRequest("exhausted".into()))
                    .await
            }
        }
        Ok(rx)
    }
}

fn message(content: Vec<ContentBlock>, stop_reason: StopReason) -> AssistantMessage {
    AssistantMessage {
        content,
        stop_reason,
        usage: Usage {
            input_tokens: 1,
            output_tokens: 1,
            ..Default::default()
        },
        model: "test-model".into(),
        response_id: None,
        applied: None,
    }
}

fn text(t: &str) -> AssistantMessage {
    message(vec![ContentBlock::text(t)], StopReason::EndTurn)
}

fn call(id: &str, name: &str, input: Value) -> AssistantMessage {
    message(
        vec![ContentBlock::ToolUse {
            id: id.into(),
            name: name.into(),
            input,
        }],
        StopReason::ToolUse,
    )
}

fn agent(dir: &tempfile::TempDir, responses: Vec<AssistantMessage>) -> Fixture {
    use vak_session::trace::{self, TraceKey};
    let home = vak_config::paths::isolate_home_for_tests();
    let session_id = uuid::Uuid::now_v7().to_string();
    let turn_id = uuid::Uuid::now_v7().to_string();
    let scope = vak_config::scope::AgentScope::new(vak_config::paths::agent_home_at(&home, "vak"));
    let trace = TraceKey::root(
        trace::local::tenant(),
        trace::local::space(dir.path()),
        trace::local::agent("vak"),
        trace::Cause::User {
            request_id: uuid::Uuid::now_v7().to_string(),
        },
    )
    .in_turn(&session_id, &turn_id)
    .acting(trace::local::local_owner(), None);
    let store = Arc::new(TopicStore::new(&scope, &session_id).unwrap());
    let header = SessionHeader {
        space: None,
        run: None,
        cause: None,
        agent: None,
        session_id: session_id.clone(),
        created_at: chrono::Utc::now(),
        cwd: dir.path().to_path_buf(),
        parent_session_id: None,
        contract_id: None,
        work_item_id: None,
        conversation: None,
        contract: FrozenContract {
            app_version: "test".into(),
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
    let mut log = SessionLog::create(dir.path().join("session"), header).unwrap();
    log.begin_turn(&turn_id).unwrap();
    let required = Arc::new(Mutex::new(Vec::new()));
    let agent = Agent::new(
        Arc::new(Scripted {
            capacity_key: crate::support::capacity_key(),
            responses: Mutex::new(VecDeque::from(responses)),
            required: required.clone(),
        }),
        log,
        {
            let mut cfg = AgentConfig::new("sys");
            cfg.model = "test-model".into();
            cfg.tools = vec![Arc::new(vak_tools::TopicsTool)];
            cfg.topic_store = Some(store.clone());
            cfg.trace = Some(trace.clone());
            cfg.mode = vak_permission::Mode::FullAccess;
            cfg.permission = Some(Arc::new(PermissionEngine::default()));
            cfg.stop_policy = None;
            cfg.max_turns = 12;
            cfg
        },
    );
    (agent, store, trace, required)
}

async fn run(agent: &mut Agent) -> TurnOutcome {
    let (tx, mut rx) = mpsc::channel(64);
    let drain = tokio::spawn(async move { while rx.recv().await.is_some() {} });
    let result = agent
        .run(
            "Continue the work",
            &Default::default(),
            CancellationToken::new(),
            tx,
        )
        .await;
    drain.await.unwrap();
    result
}

fn create() -> AssistantMessage {
    call(
        "place",
        "topics",
        serde_json::json!({
            "action":"create", "title":"Market research", "outcomes":["Market summary", "Sources"]
        }),
    )
}

async fn rungs(agent: &Agent) -> Vec<vak_control::Rung> {
    agent
        .session
        .lock()
        .await
        .explanation()
        .into_iter()
        .filter_map(|(_, step)| match step {
            vak_session::log::TurnStep::Decision(decision) => match &decision.choice {
                vak_control::Choice::Correct(c)
                    if c.cause == vak_control::Cause::TopicsPlacement =>
                {
                    Some(c.rung)
                }
                _ => None,
            },
            _ => None,
        })
        .collect()
}

#[tokio::test]
async fn skipped_organisation_is_corrected_and_places_multiple_outcomes() {
    let dir = tempfile::tempdir().unwrap();
    let (mut agent, store, _, required) = agent(
        &dir,
        vec![text("The summary."), create(), text("The summary.")],
    );
    assert!(matches!(
        run(&mut agent).await,
        TurnOutcome::Completed { .. }
    ));
    let snapshot = store.snapshot().unwrap();
    assert_eq!(snapshot.topics.len(), 1);
    assert_eq!(snapshot.placements.len(), 2);
    assert_eq!(rungs(&agent).await, vec![vak_control::Rung::Tell]);
    assert_eq!(required.lock().unwrap().len(), 3);
    // Corrections and tool exchanges survive ordinary message derivation.
    let messages = agent.session.lock().await.derive_transcript();
    assert!(messages.iter().any(|m| {
        m.as_typed()
            .text_content()
            .contains("no recorded organisation")
    }));
}

#[tokio::test]
async fn list_alone_does_not_satisfy_placement_and_the_frame_changes() {
    let dir = tempfile::tempdir().unwrap();
    let (mut agent, store, _, required) = agent(
        &dir,
        vec![
            call("list", "topics", serde_json::json!({"action":"list"})),
            text("The summary."),
            text("Still skipped."),
            create(),
            text("The summary."),
        ],
    );
    assert!(matches!(
        run(&mut agent).await,
        TurnOutcome::Completed { .. }
    ));
    assert_eq!(store.snapshot().unwrap().placements.len(), 2);
    assert_eq!(required.lock().unwrap()[3], Some("topics".into()));
    assert_eq!(
        rungs(&agent).await,
        vec![vak_control::Rung::Tell, vak_control::Rung::Reframe]
    );
}

#[tokio::test]
async fn noncompliant_model_ends_with_its_answer_without_an_organisation_loop() {
    let dir = tempfile::tempdir().unwrap();
    let (mut agent, _, _, required) =
        agent(&dir, vec![text("First."), text("Second."), text("Final.")]);
    let outcome = run(&mut agent).await;
    assert!(
        matches!(outcome, TurnOutcome::Completed { response } if response.text_content() == "Final.")
    );
    assert_eq!(
        *required.lock().unwrap(),
        vec![None, None, Some("topics".into())]
    );
    assert_eq!(
        rungs(&agent).await,
        vec![
            vak_control::Rung::Tell,
            vak_control::Rung::Reframe,
            vak_control::Rung::End
        ]
    );
}

#[tokio::test]
async fn simple_answer_can_explicitly_remain_ungrouped() {
    let dir = tempfile::tempdir().unwrap();
    let (mut agent, store, _, _) = agent(
        &dir,
        vec![
            call(
                "skip",
                "topics",
                serde_json::json!({"action":"assign", "outcome":"Independent answer", "topic_ids":[]}),
            ),
            text("42."),
        ],
    );
    assert!(matches!(
        run(&mut agent).await,
        TurnOutcome::Completed { .. }
    ));
    let snapshot = store.snapshot().unwrap();
    assert!(snapshot.topics.is_empty());
    assert_eq!(snapshot.placements.len(), 1);
    assert!(snapshot.placements[0].topic_ids.is_empty());
    assert!(rungs(&agent).await.is_empty());
}

#[tokio::test]
async fn automatic_off_and_human_unassignment_are_not_overridden() {
    for automatic in [false, true] {
        let dir = tempfile::tempdir().unwrap();
        let (mut agent, store, trace, required) = agent(&dir, vec![text("Answer.")]);
        if automatic {
            store
                .assign_turn_human(&trace.turn.unwrap().uuid().to_string(), vec![], &trace)
                .unwrap();
        } else {
            store.set_automatic(false, &trace).unwrap();
        }
        assert!(matches!(
            run(&mut agent).await,
            TurnOutcome::Completed { .. }
        ));
        assert_eq!(required.lock().unwrap().len(), 1);
        assert!(rungs(&agent).await.is_empty());
    }
}

#[tokio::test]
async fn failed_organisation_does_not_hold_the_requested_answer() {
    let dir = tempfile::tempdir().unwrap();
    let (mut agent, _, _, required) = agent(
        &dir,
        vec![
            call(
                "bad",
                "topics",
                serde_json::json!({"action":"assign", "outcome":"Answer", "topic_ids":["invalid"]}),
            ),
            text("Answer; organisation unavailable."),
        ],
    );
    assert!(matches!(
        run(&mut agent).await,
        TurnOutcome::Completed { .. }
    ));
    assert_eq!(required.lock().unwrap().len(), 2);
    assert!(rungs(&agent).await.is_empty());
}

#[tokio::test]
async fn organisation_never_spends_the_last_step_or_requires_a_child_store() {
    for child in [false, true] {
        let dir = tempfile::tempdir().unwrap();
        let (mut agent, _, _, required) = agent(&dir, vec![text("Answer.")]);
        if child {
            agent.config.topic_store = None;
        } else {
            agent.config.max_turns = 1;
        }
        assert!(matches!(
            run(&mut agent).await,
            TurnOutcome::Completed { .. }
        ));
        assert_eq!(required.lock().unwrap().len(), 1);
    }
}

#[tokio::test]
async fn an_older_turns_placement_does_not_hide_the_current_gap() {
    let dir = tempfile::tempdir().unwrap();
    let (mut agent, store, trace, _) =
        agent(&dir, vec![text("Summary."), create(), text("Summary.")]);
    store
        .assign_model(
            &uuid::Uuid::now_v7().to_string(),
            "Earlier answer",
            vec![],
            &trace,
        )
        .unwrap();
    assert!(matches!(
        run(&mut agent).await,
        TurnOutcome::Completed { .. }
    ));
    assert_eq!(store.snapshot().unwrap().placements.len(), 3);
    assert_eq!(rungs(&agent).await, vec![vak_control::Rung::Tell]);
}

struct StepsBlock;

#[async_trait]
impl vak_tools::Tool for StepsBlock {
    fn name(&self) -> &str {
        "steps_block"
    }
    fn description(&self) -> &str {
        "Steps in order."
    }
    fn schema(&self) -> Value {
        serde_json::json!({"type":"object"})
    }
    fn presents_cards(&self) -> bool {
        true
    }
    fn card_kinds(&self) -> Vec<String> {
        vec!["steps".into()]
    }
    async fn execute(
        &self,
        _: &Value,
        _: &vak_tools::context::ToolContext,
    ) -> vak_tools::ToolOutput {
        vak_tools::ToolOutput::ok("Shown to the user.")
    }
}

#[tokio::test]
async fn card_only_answer_still_gets_organised_without_duplicating_the_card() {
    let dir = tempfile::tempdir().unwrap();
    let (mut agent, store, _, required) = agent(
        &dir,
        vec![
            call(
                "card",
                "steps_block",
                serde_json::json!({"steps":["Inspect the market data"],"then":"done"}),
            ),
            create(),
            text(""),
        ],
    );
    agent.config.tools.push(Arc::new(StepsBlock));
    agent.config.presentation_rebuild = Some(Arc::new(|_, input| {
        Some(vak_tools::PresentationCard {
            presentation_id: None,
            semantic_type: "steps".into(),
            skill_id: "test".into(),
            skill_version: "1".into(),
            schema_version: 1,
            payload: input.clone(),
            title: "Market summary".into(),
            identity_digest: "summary".into(),
            block: Some("steps".into()),
            data_call_id: None,
        })
    }));
    let outcome = run(&mut agent).await;
    assert!(
        matches!(outcome, TurnOutcome::Completed { .. }),
        "{outcome:?}"
    );
    assert_eq!(store.snapshot().unwrap().placements.len(), 2);
    assert_eq!(agent.session.lock().await.presentations().len(), 1);
    assert_eq!(required.lock().unwrap().len(), 3);
    assert_eq!(rungs(&agent).await, vec![vak_control::Rung::Tell]);
}

#[tokio::test]
async fn organisation_preserves_the_answer_at_outcome_and_request_call_limits() {
    for request_limit in [false, true] {
        let dir = tempfile::tempdir().unwrap();
        let (mut agent, _, trace, required) = agent(&dir, vec![text("Answer.")]);
        if request_limit {
            agent.config.model_call_budget =
                Some(Arc::new(vak_agent::ModelCallBudget::new(trace.run, 1)));
        } else {
            agent.config.outcome = Some(
                serde_json::from_value(serde_json::json!({
                    "schema_version":1,"objective":"Answer","resolver_version":1,"max_turns":1
                }))
                .unwrap(),
            );
        }
        let outcome = run(&mut agent).await;
        assert!(
            matches!(outcome, TurnOutcome::Completed { .. }),
            "{outcome:?}"
        );
        assert_eq!(required.lock().unwrap().len(), 1);
    }
}
