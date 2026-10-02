#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

//! The agent loop's use of the context engine (docs/design/68-context-engine.md
//! §4, §10): history gives way as the open turn grows, a failed summariser does
//! not fail the turn, the plan behind each request is recorded and replayable,
//! and a turn that ended early still gets its card.

use std::collections::VecDeque;
use std::sync::{Arc, Mutex};

use tempfile::tempdir;
use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;
use vak_agent::{Agent, AgentConfig, ApprovalMode, AutoApprove, SteeringQueues, TurnOutcome};
use vak_context::capacity::{CacheBehaviour, CapacityProfile, Horizon, ProbeProvenance};
use vak_llm::stream;
use vak_llm::types::{AssistantMessage, ChatRequest, ContentBlock, StopReason, Usage};
use vak_llm::{EventStream, LlmError, Provider};
use vak_session::types::{FrozenContract, MessageRecord, SessionHeader, TurnCardRecord};
use vak_session::{SessionLog, TurnIndex};
use vak_tools::bash::BashTool;

struct Script {
    replies: Mutex<VecDeque<Result<AssistantMessage, LlmError>>>,
    requests: Arc<Mutex<Vec<ChatRequest>>>,
}

#[async_trait::async_trait]
impl Provider for Script {
    fn name(&self) -> &str {
        "scripted"
    }
    async fn stream(&self, r: ChatRequest, _c: CancellationToken) -> Result<EventStream, LlmError> {
        self.requests.lock().unwrap().push(r);
        let next = self.replies.lock().unwrap().pop_front();
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

fn msg(content: Vec<ContentBlock>, stop: StopReason) -> Result<AssistantMessage, LlmError> {
    Ok(AssistantMessage {
        content,
        stop_reason: stop,
        usage: Usage::default(),
        model: "m".into(),
        response_id: None,
    })
}

fn say(text: &str) -> Result<AssistantMessage, LlmError> {
    msg(vec![ContentBlock::text(text)], StopReason::EndTurn)
}

fn bash(id: &str, command: &str) -> Result<AssistantMessage, LlmError> {
    msg(
        vec![ContentBlock::ToolUse {
            id: id.into(),
            name: "bash".into(),
            input: serde_json::json!({"command": command}),
        }],
        StopReason::ToolUse,
    )
}

fn profile(horizon: u64) -> CapacityProfile {
    CapacityProfile::from_probe(
        32_768,
        None,
        Horizon {
            tokens: horizon,
            confidence: 0.9,
            last_confirmed: std::time::SystemTime::now(),
        },
        CacheBehaviour::Unknown,
        1_024,
        ProbeProvenance {
            probed_at: std::time::SystemTime::now(),
            rungs: Vec::new(),
            signals: Vec::new(),
            metadata_digest: "d".into(),
            quantisation: None,
        },
    )
}

fn open(dir: &std::path::Path) -> SessionLog {
    let header = SessionHeader {
        space: None,
        run: None,
        cause: None,
        agent: None,
        session_id: "s-loop".into(),
        created_at: chrono::Utc::now(),
        cwd: dir.to_path_buf(),
        parent_session_id: None,
        contract_id: None,
        work_item_id: None,
        conversation: None,
        contract: FrozenContract {
            app_version: "0.1.0".into(),
            provider: "scripted".into(),
            model: "m".into(),
            route_ladder: Vec::new(),
            route_objective: String::new(),
            route_annotations: Vec::new(),
            system_prompt: "sys".into(),
            permission_mode: "full-access".into(),
            capabilities: Vec::new(),
            prompt_layers: Vec::new(),
        },
    };
    SessionLog::create(dir.join("s.jsonl"), header).unwrap()
}

fn past_turn(log: &mut SessionLog, question: &str, answer: &str) {
    let id = log
        .append_message(MessageRecord {
            message: vak_llm::Message::user_text(question),
            meta: None,
        })
        .unwrap()
        .id;
    log.append_message(MessageRecord {
        message: vak_llm::Message::assistant(vec![ContentBlock::text(answer)]),
        meta: None,
    })
    .unwrap();
    let card = TurnIndex::from_log(log)
        .turn_by_id(&id)
        .unwrap()
        .build_card("completed", "ok".into(), &|s| s.len() as u64 / 4);
    log.append_turn_card(TurnCardRecord { turn_id: id, card })
        .unwrap();
}

struct Run {
    outcome: TurnOutcome,
    requests: Vec<ChatRequest>,
    path: std::path::PathBuf,
}

async fn run(
    log: SessionLog,
    horizon: u64,
    prompt: &str,
    replies: Vec<Result<AssistantMessage, LlmError>>,
    cancelled: bool,
) -> Run {
    let path = log.path().to_path_buf();
    let mut cfg = AgentConfig::new("sys");
    cfg.tools = vec![Arc::new(BashTool)];
    cfg.capacity = Some(profile(horizon));
    cfg.max_retries = 0;
    let requests = Arc::new(Mutex::new(Vec::new()));
    let provider = Arc::new(Script {
        replies: Mutex::new(replies.into()),
        requests: requests.clone(),
    });
    let mut agent = Agent::new(provider, log, cfg);
    agent.config.mode = vak_permission::Mode::FullAccess;
    agent.config.approval_mode = ApprovalMode::AutoApprove;
    agent.config.approver = Some(Arc::new(AutoApprove));
    let cancel = CancellationToken::new();
    if cancelled {
        cancel.cancel();
    }
    let (tx, _rx) = mpsc::channel(4096);
    let outcome = agent.run(prompt, &SteeringQueues::new(), cancel, tx).await;
    drop(agent);
    let requests = requests.lock().unwrap().clone();
    Run {
        outcome,
        requests,
        path,
    }
}

fn carrying(request: &ChatRequest, needle: &str) -> usize {
    request
        .messages
        .iter()
        .filter(|m| {
            m.content
                .iter()
                .any(|b| matches!(b, ContentBlock::Text { text } if text.contains(needle)))
        })
        .count()
}

#[tokio::test]
async fn history_gives_way_when_the_open_turn_outgrows_the_plan() {
    let dir = tempdir().unwrap();
    let mut log = open(dir.path());
    for n in 1..=3 {
        past_turn(
            &mut log,
            &format!("alpha budget review {n}"),
            &"a".repeat(4_000),
        );
    }
    let r = run(
        log,
        6_000,
        "alpha budget review again",
        vec![
            bash("t1", "head -c 16000 /dev/zero | tr '\\0' x"),
            say("done"),
        ],
        false,
    )
    .await;
    assert!(
        matches!(r.outcome, TurnOutcome::Completed { .. }),
        "{:?}",
        r.outcome
    );
    let marker = "a".repeat(100);
    let (first, second) = (
        carrying(&r.requests[0], &marker),
        carrying(&r.requests[1], &marker),
    );
    assert!(first >= 2, "relevant history rides at Full first: {first}");
    assert!(second < first, "history gave way: {first} -> {second}");
    assert!(
        r.requests[1].messages.iter().any(|m| m.content.iter().any(
            |b| matches!(b, ContentBlock::ToolResult { content, .. } if content.len() >= 16_000)
        )),
        "the open turn stays verbatim"
    );
}

#[tokio::test]
async fn a_failed_summariser_does_not_fail_the_turn() {
    let dir = tempdir().unwrap();
    let mut log = open(dir.path());
    for n in 0..40 {
        past_turn(&mut log, &format!("unrelated filler number {n}"), "ok");
    }
    let r = run(
        log,
        600,
        "something else entirely",
        vec![
            Err(LlmError::Network("summariser down".into())),
            say("answer"),
        ],
        false,
    )
    .await;
    assert!(
        matches!(r.outcome, TurnOutcome::Completed { .. }),
        "{:?}",
        r.outcome
    );
    let log = SessionLog::open_read_only(r.path).unwrap();
    assert!(
        log.activities()
            .iter()
            .any(|(_, _, a)| a.label == "compaction-failed"),
        "the failure is on the record"
    );
}

#[tokio::test]
async fn the_plan_behind_a_request_is_recorded_and_replays_exactly() {
    let dir = tempdir().unwrap();
    let mut log = open(dir.path());
    past_turn(&mut log, "alpha budget review one", "first answer body");
    past_turn(&mut log, "bravo unrelated thing", "second answer body");
    let r = run(
        log,
        20_000,
        "alpha budget review again",
        vec![say("done")],
        false,
    )
    .await;
    assert!(matches!(r.outcome, TurnOutcome::Completed { .. }));
    let log = SessionLog::open_read_only(r.path.clone()).unwrap();
    let (entry_id, (plan, leaf)) = log
        .chain_to_root()
        .into_iter()
        .find_map(|e| match &e.payload {
            vak_session::EntryPayload::Activity(a) => {
                vak_session::WorkingSetPlan::from_activity_data(&a.data).map(|p| (e.id.clone(), p))
            }
            _ => None,
        })
        .expect("a context-plan activity");
    assert!(!entry_id.is_empty());
    let mut replay = SessionLog::open_read_only(r.path).unwrap();
    replay.branch_at(&leaf).unwrap();
    let (messages, _) = replay.derive_with_plan_and_directive(&plan);
    assert_eq!(
        serde_json::to_value(&messages).unwrap(),
        serde_json::to_value(&r.requests[0].messages).unwrap(),
        "the recorded plan reproduces the request"
    );
}

#[tokio::test]
async fn a_cancelled_turn_still_gets_its_card() {
    let dir = tempdir().unwrap();
    let log = open(dir.path());
    let r = run(log, 20_000, "start something", vec![say("never")], true).await;
    assert!(
        matches!(r.outcome, TurnOutcome::Aborted { .. }),
        "{:?}",
        r.outcome
    );
    let log = SessionLog::open_read_only(r.path).unwrap();
    let cards = log.turn_cards();
    assert_eq!(cards.len(), 1);
    assert_eq!(cards[0].1.outcome, "cancelled");
}

#[tokio::test]
async fn recall_reaches_inside_a_line_a_window_cut() {
    let dir = tempdir().unwrap();
    let log = open(dir.path());
    let recall = msg(
        vec![ContentBlock::ToolUse {
            id: "r1".into(),
            name: "recall".into(),
            input: serde_json::json!({"id": "t1", "chars": {"start": 29_980, "end": 30_020}}),
        }],
        StopReason::ToolUse,
    );
    let r = run(
        log,
        50_000,
        "dump it",
        vec![
            bash(
                "t1",
                "head -c 29990 /dev/zero | tr '\\0' a; printf NEEDLE; head -c 10000 /dev/zero | tr '\\0' b",
            ),
            recall,
            say("done"),
        ],
        false,
    )
    .await;
    assert!(
        matches!(r.outcome, TurnOutcome::Completed { .. }),
        "{:?}",
        r.outcome
    );
    let shown = |request: &ChatRequest| {
        request
            .messages
            .iter()
            .flat_map(|m| m.content.iter())
            .filter_map(|b| match b {
                ContentBlock::ToolResult { content, .. } => Some(content.clone()),
                _ => None,
            })
            .collect::<Vec<_>>()
            .join("\n")
    };
    assert!(
        !shown(&r.requests[1]).contains("NEEDLE"),
        "the window cut the line"
    );
    assert!(
        shown(&r.requests[1]).contains("\"chars\": {\"start\": 20"),
        "and says how to reach the rest"
    );
    assert!(
        shown(&r.requests[2]).contains("NEEDLE"),
        "recall by characters returns it"
    );
}
