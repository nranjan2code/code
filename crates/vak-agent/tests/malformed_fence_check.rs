#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

//! Regression coverage for a verified real bug: the model emits a `vak`
//! card fence whose JSON body doesn't parse (mismatched brackets, an
//! unquoted key — real failure shapes seen from a small local model in
//! production), and the answer reaches the user with a broken/invisible
//! card instead of a working one. `Agent::run` must give the model one
//! bounded repair turn naming the exact parse error, reusing the same
//! turn-loop `continue` mechanism the grounding-check repair uses — this
//! is a new trigger condition on existing infrastructure, not a new system.

mod support;

use std::collections::VecDeque;
use std::sync::Arc;
use std::sync::Mutex;

use async_trait::async_trait;
use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;

use tempfile::tempdir;

use vak_agent::{Agent, AgentConfig, TurnOutcome};
use vak_llm::stream;
use vak_llm::types::{AssistantMessage, ChatRequest, ContentBlock, StopReason, Usage};
use vak_llm::{EventStream, LlmError, Provider};
use vak_permission::PermissionEngine;
use vak_session::types::{FrozenContract, SessionHeader};
use vak_session::{SessionLog, SessionPath};

struct Scripted {
    capacity_key: String,
    responses: Mutex<VecDeque<AssistantMessage>>,
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

fn text_msg(t: &str) -> AssistantMessage {
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

async fn build_agent(
    dir: &tempfile::TempDir,
    session_id: &str,
    responses: Vec<AssistantMessage>,
) -> Agent {
    let header = SessionHeader {
        space: None,
        run: None,
        cause: None,
        agent: None,
        session_id: session_id.into(),
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
    let home = dir.path().join("home");
    std::fs::create_dir_all(&home).unwrap();
    let log = SessionLog::create(
        SessionPath::new_session_file(&home, dir.path(), session_id),
        header,
    )
    .unwrap();

    Agent::new(
        Arc::new(Scripted {
            capacity_key: crate::support::capacity_key(),
            responses: Mutex::new(VecDeque::from(responses)),
        }),
        log,
        {
            let mut cfg = AgentConfig::new("sys");
            cfg.model = "test-model".into();
            cfg.mode = vak_permission::Mode::FullAccess;
            cfg.permission = Some(Arc::new(PermissionEngine::default()));
            cfg
        },
    )
}

// Raw ledger, not the model-visible projection: these tests are about the
// repair loop's mechanics, which a closed turn's full record deliberately
// no longer preserves (docs/design/68-context-engine.md §10) — a rejected
// draft and a mid-turn nudge are dropped once the turn closes.
fn assistant_texts(agent: &Agent) -> Vec<String> {
    futures::executor::block_on(async {
        agent
            .session
            .lock()
            .await
            .message_chain()
            .iter()
            .filter(|(_, m)| m.role == vak_llm::types::Role::Assistant)
            .flat_map(|(_, m)| m.content.iter())
            .filter_map(|b| match b {
                ContentBlock::Text { text } => Some(text.clone()),
                _ => None,
            })
            .collect()
    })
}

fn user_texts(agent: &Agent) -> Vec<String> {
    futures::executor::block_on(async {
        agent
            .session
            .lock()
            .await
            .message_chain()
            .iter()
            .filter(|(_, m)| m.role == vak_llm::types::Role::User)
            .flat_map(|(_, m)| m.content.iter())
            .filter_map(|b| match b {
                ContentBlock::Text { text } => Some(text.clone()),
                _ => None,
            })
            .collect()
    })
}

// The exact malformed shape from the live bug: a `decision` fence whose
// fence has a bare (unquoted) key, `precision":1` instead of `"precision":1`.
const REAL_MALFORMED_FENCE: &str = "```vak\n{\"semantic_type\":\"decision\",\"title\":\"Capability Confirmation\",\"items\":[{\"label\":\"Yes\",\"detail\":\"ok\"}],precision\":1}\n```\n\n### Explanation\n\nYes, I can do that.";

const VALID_FENCE: &str = "```vak\n{\"semantic_type\":\"metric\",\"label\":\"Uptime\",\"value\":\"99.9\",\"unit\":\"%\"}\n```";

#[tokio::test]
async fn malformed_fence_gets_one_repair_turn_naming_the_parse_error() {
    let dir = tempdir().unwrap();
    let mut agent = build_agent(
        &dir,
        "fence-repair",
        vec![text_msg(REAL_MALFORMED_FENCE), text_msg(VALID_FENCE)],
    )
    .await;

    let outcome = agent
        .run(
            "can you make charts",
            &Default::default(),
            CancellationToken::new(),
            mpsc::channel(64).0,
        )
        .await;
    assert!(
        matches!(outcome, TurnOutcome::Completed { .. }),
        "got {outcome:?}"
    );

    let texts = assistant_texts(&agent);
    assert!(
        texts
            .iter()
            .any(|t| t.contains("\"semantic_type\":\"metric\"")),
        "final state must contain the repaired, valid fence: {texts:?}"
    );

    let users = user_texts(&agent);
    assert!(
        users
            .iter()
            .any(|t| t.contains("[fence-check]") && t.contains("invalid JSON")),
        "expected a fence-check repair nudge naming the parse failure: {users:?}"
    );
}

#[tokio::test]
async fn repair_is_bounded_to_one_attempt_not_a_loop() {
    let dir = tempdir().unwrap();
    let mut agent = build_agent(
        &dir,
        "fence-repair-bounded",
        vec![
            text_msg(REAL_MALFORMED_FENCE),
            text_msg(REAL_MALFORMED_FENCE), // still broken after the nudge
        ],
    )
    .await;

    let outcome = agent
        .run(
            "can you make charts",
            &Default::default(),
            CancellationToken::new(),
            mpsc::channel(64).0,
        )
        .await;
    assert!(
        matches!(outcome, TurnOutcome::Completed { .. }),
        "got {outcome:?}"
    );

    let texts = assistant_texts(&agent);
    assert_eq!(
        texts
            .iter()
            .filter(|t| t.contains("Capability Confirmation"))
            .count(),
        2,
        "exactly one retry — not zero, not an infinite loop: {texts:?}"
    );
}

#[tokio::test]
async fn a_valid_fence_is_never_touched() {
    let dir = tempdir().unwrap();
    let mut agent = build_agent(&dir, "fence-valid", vec![text_msg(VALID_FENCE)]).await;

    let outcome = agent
        .run(
            "what's our uptime",
            &Default::default(),
            CancellationToken::new(),
            mpsc::channel(64).0,
        )
        .await;
    assert!(
        matches!(outcome, TurnOutcome::Completed { .. }),
        "got {outcome:?}"
    );

    let users = user_texts(&agent);
    assert!(
        !users.iter().any(|t| t.contains("[fence-check]")),
        "a syntactically valid fence must never trigger a repair nudge: {users:?}"
    );
}

#[tokio::test]
async fn plain_prose_with_no_fence_is_never_touched() {
    let dir = tempdir().unwrap();
    let mut agent = build_agent(
        &dir,
        "fence-none",
        vec![text_msg("Paris is the capital of France.")],
    )
    .await;

    let outcome = agent
        .run(
            "what's the capital of france",
            &Default::default(),
            CancellationToken::new(),
            mpsc::channel(64).0,
        )
        .await;
    assert!(
        matches!(outcome, TurnOutcome::Completed { .. }),
        "got {outcome:?}"
    );

    let users = user_texts(&agent);
    assert!(
        !users.iter().any(|t| t.contains("[fence-check]")),
        "plain prose with no fence must never trigger a repair nudge: {users:?}"
    );
}
