#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

//! An over-length rejection belongs to the route leg that raised it
//! (docs/design/68-context-engine.md §1): only the primary's lowers the
//! primary's capacity profile, and it does so however the later legs fared.

use std::collections::VecDeque;
use std::sync::{Arc, Mutex};

use tempfile::tempdir;
use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;
use vak_agent::{Agent, AgentConfig, SteeringQueues, TurnOutcome};
use vak_context::capacity::CapacityProfile;
use vak_llm::stream;
use vak_llm::types::{AssistantMessage, ChatRequest, ContentBlock, StopReason, Usage};
use vak_llm::{EventStream, LlmError, Provider};
use vak_session::SessionLog;
use vak_session::types::{FrozenContract, SessionHeader};

struct Leg {
    name: &'static str,
    script: Mutex<VecDeque<Result<AssistantMessage, LlmError>>>,
    calls: Arc<Mutex<usize>>,
}

#[async_trait::async_trait]
impl Provider for Leg {
    fn name(&self) -> &str {
        self.name
    }
    async fn stream(
        &self,
        _r: ChatRequest,
        _c: CancellationToken,
    ) -> Result<EventStream, LlmError> {
        *self.calls.lock().unwrap() += 1;
        let next = self.script.lock().unwrap().pop_front();
        let (mut sink, rx) = stream::channel(64);
        match next {
            Some(Ok(m)) => {
                sink.push(stream::StreamEvent::Start { partial: m.clone() });
                sink.close_message(m).await;
            }
            Some(Err(e)) => sink.close_error(e).await,
            None => {
                sink.close_error(LlmError::Parse("script exhausted".into()))
                    .await
            }
        }
        Ok(rx)
    }
}

fn answer() -> Result<AssistantMessage, LlmError> {
    Ok(AssistantMessage {
        content: vec![ContentBlock::text("answer")],
        stop_reason: StopReason::EndTurn,
        usage: Usage::default(),
        model: "m".into(),
        response_id: None,
    })
}

fn too_long() -> Result<AssistantMessage, LlmError> {
    Err(LlmError::Context("prompt too long for this window".into()))
}

fn down() -> Result<AssistantMessage, LlmError> {
    Err(LlmError::Network("connection reset".into()))
}

struct Outcome {
    outcome: TurnOutcome,
    horizon_before: u64,
    horizon_after: u64,
    primary_calls: usize,
    fallback_calls: usize,
}

async fn run(
    primary: Vec<Result<AssistantMessage, LlmError>>,
    fallback: Vec<Result<AssistantMessage, LlmError>>,
) -> Outcome {
    let dir = tempdir().unwrap();
    let header = SessionHeader {
        space: None,
        run: None,
        cause: None,
        agent: None,
        session_id: "s-leg".into(),
        created_at: chrono::Utc::now(),
        cwd: dir.path().to_path_buf(),
        parent_session_id: None,
        contract_id: None,
        work_item_id: None,
        conversation: None,
        contract: FrozenContract {
            app_version: "0.1.0".into(),
            provider: "primary".into(),
            model: "big".into(),
            route_ladder: Vec::new(),
            route_objective: String::new(),
            route_annotations: Vec::new(),
            system_prompt: "sys".into(),
            permission_mode: "full-access".into(),
            capabilities: Vec::new(),
            prompt_layers: Vec::new(),
        },
    };
    let log = SessionLog::create(dir.path().join("s.jsonl"), header).unwrap();
    let mut cfg = AgentConfig::new("sys");
    cfg.capacity = Some(CapacityProfile::from_metadata_only(
        200_000,
        8_192,
        "d".into(),
        std::time::SystemTime::now(),
    ));
    cfg.max_retries = 0;
    let primary_calls = Arc::new(Mutex::new(0));
    let fallback_calls = Arc::new(Mutex::new(0));
    let primary_leg = Arc::new(Leg {
        name: "primary",
        script: Mutex::new(primary.into()),
        calls: primary_calls.clone(),
    });
    let fallback_leg = Arc::new(Leg {
        name: "small-fallback",
        script: Mutex::new(fallback.into()),
        calls: fallback_calls.clone(),
    });
    cfg.ladder = vec![(fallback_leg as Arc<dyn Provider>, "tiny".to_string())];
    cfg.ladder_provider_names = vec!["small-fallback".into()];
    let mut agent = Agent::new(primary_leg, log, cfg);
    let horizon_before = agent
        .config
        .capacity
        .as_ref()
        .unwrap()
        .instruction_horizon
        .tokens;
    let (tx, _rx) = mpsc::channel(4096);
    let outcome = agent
        .run(
            "hello there",
            &SteeringQueues::new(),
            CancellationToken::new(),
            tx,
        )
        .await;
    let horizon_after = agent
        .config
        .capacity
        .as_ref()
        .unwrap()
        .instruction_horizon
        .tokens;
    std::mem::forget(dir);
    let primary_calls = *primary_calls.lock().unwrap();
    let fallback_calls = *fallback_calls.lock().unwrap();
    Outcome {
        outcome,
        horizon_before,
        horizon_after,
        primary_calls,
        fallback_calls,
    }
}

#[tokio::test]
async fn a_fallback_legs_rejection_leaves_the_primarys_horizon_alone() {
    let r = run(vec![down()], vec![too_long()]).await;
    assert_eq!(r.horizon_after, r.horizon_before, "{:?}", r.outcome);
    assert!(
        matches!(r.outcome, TurnOutcome::Failed { .. }),
        "{:?}",
        r.outcome
    );
}

#[tokio::test]
async fn the_primarys_rejection_is_acted_on_even_when_a_later_leg_fails_otherwise() {
    let r = run(vec![too_long(), answer()], vec![down()]).await;
    assert!(
        matches!(r.outcome, TurnOutcome::Completed { .. }),
        "{:?}",
        r.outcome
    );
    assert!(
        r.horizon_after < r.horizon_before,
        "the primary's horizon is lowered"
    );
    assert_eq!(r.primary_calls, 2, "replanned and retried on the primary");
}

#[tokio::test]
async fn a_fallback_that_carries_the_request_still_lowers_the_primarys_horizon() {
    let r = run(vec![too_long()], vec![answer()]).await;
    assert!(
        matches!(r.outcome, TurnOutcome::Completed { .. }),
        "{:?}",
        r.outcome
    );
    assert!(r.horizon_after < r.horizon_before);
    assert_eq!(
        (r.primary_calls, r.fallback_calls),
        (1, 1),
        "no replan once a leg answered"
    );
}

fn answer_with_usage() -> Result<AssistantMessage, LlmError> {
    Ok(AssistantMessage {
        content: vec![ContentBlock::text("answer")],
        stop_reason: StopReason::EndTurn,
        usage: Usage {
            input_tokens: 400,
            output_tokens: 5,
            ..Default::default()
        },
        model: "m".into(),
        response_id: None,
    })
}

async fn calibration_samples(
    primary: Vec<Result<AssistantMessage, LlmError>>,
    fallback: Vec<Result<AssistantMessage, LlmError>>,
) -> u32 {
    let r = run_profile(primary, fallback).await;
    r.0.config
        .capacity
        .as_ref()
        .unwrap()
        .tokens_per_char
        .samples
}

async fn run_profile(
    primary: Vec<Result<AssistantMessage, LlmError>>,
    fallback: Vec<Result<AssistantMessage, LlmError>>,
) -> (Agent,) {
    let dir = tempdir().unwrap();
    let header = SessionHeader {
        space: None,
        run: None,
        cause: None,
        agent: None,
        session_id: "s-cal".into(),
        created_at: chrono::Utc::now(),
        cwd: dir.path().to_path_buf(),
        parent_session_id: None,
        contract_id: None,
        work_item_id: None,
        conversation: None,
        contract: FrozenContract {
            app_version: "0.1.0".into(),
            provider: "primary".into(),
            model: "big".into(),
            route_ladder: Vec::new(),
            route_objective: String::new(),
            route_annotations: Vec::new(),
            system_prompt: "sys".into(),
            permission_mode: "full-access".into(),
            capabilities: Vec::new(),
            prompt_layers: Vec::new(),
        },
    };
    let log = SessionLog::create(dir.path().join("s.jsonl"), header).unwrap();
    let mut cfg = AgentConfig::new("sys");
    cfg.capacity = Some(CapacityProfile::from_metadata_only(
        200_000,
        8_192,
        "d".into(),
        std::time::SystemTime::now(),
    ));
    cfg.max_retries = 0;
    let primary_leg = Arc::new(Leg {
        name: "primary",
        script: Mutex::new(primary.into()),
        calls: Arc::new(Mutex::new(0)),
    });
    let fallback_leg = Arc::new(Leg {
        name: "small-fallback",
        script: Mutex::new(fallback.into()),
        calls: Arc::new(Mutex::new(0)),
    });
    cfg.ladder = vec![(fallback_leg as Arc<dyn Provider>, "tiny".to_string())];
    cfg.ladder_provider_names = vec!["small-fallback".into()];
    let mut agent = Agent::new(primary_leg, log, cfg);
    let (tx, _rx) = mpsc::channel(4096);
    let _ = agent
        .run(
            "hello there",
            &SteeringQueues::new(),
            CancellationToken::new(),
            tx,
        )
        .await;
    std::mem::forget(dir);
    (agent,)
}

#[tokio::test]
async fn usage_from_the_primary_calibrates_it_and_a_fallbacks_does_not() {
    assert_eq!(
        calibration_samples(vec![answer_with_usage()], vec![]).await,
        1
    );
    assert_eq!(
        calibration_samples(vec![down()], vec![answer_with_usage()]).await,
        0,
        "a fallback leg's tokenizer is not the primary's"
    );
}
