#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

//! The agent loop's use of the context engine (docs/design/68-context-engine.md
//! §4, §10): the newest exchanges ride whole while they fit, what no longer
//! fits is summarised once and then reused, the plan behind each request is
//! replayable, and an early-ended turn still receives its durable card.

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
        applied: None,
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
        Some(32_768),
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
    SessionLog::create(dir.join("s.jsonl"), header)
        .unwrap()
        .with_objects(std::sync::Arc::new(
            vak_session::objects::MemoryObjects::default(),
        ))
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

fn compactions(path: &std::path::Path) -> Vec<(String, String, String)> {
    SessionLog::open_read_only(path.to_path_buf())
        .unwrap()
        .chain_to_root()
        .into_iter()
        .filter_map(|entry| match &entry.payload {
            vak_session::EntryPayload::Compaction(record) if !record.reset_all => Some((
                record.first_turn_id.clone(),
                record.last_turn_id.clone(),
                record.summary.clone(),
            )),
            _ => None,
        })
        .collect()
}

fn recorded_plans(path: &std::path::Path) -> Vec<vak_session::WorkingSetPlan> {
    SessionLog::open_read_only(path.to_path_buf())
        .unwrap()
        .chain_to_root()
        .into_iter()
        .filter_map(|entry| match &entry.payload {
            vak_session::EntryPayload::ContextSelection(record) => Some(record.plan.clone()),
            _ => None,
        })
        .collect()
}

#[tokio::test]
async fn open_turn_growth_makes_older_history_give_way_to_a_summary() {
    let dir = tempdir().unwrap();
    let mut log = open(dir.path());
    for n in 1..=3 {
        past_turn(
            &mut log,
            &format!("alpha budget review {n}"),
            &"a".repeat(4_000),
        );
    }
    let ids: Vec<String> = log
        .visible_turn_cards_rev()
        .map(|(_, card)| card.turn_id.clone())
        .collect();
    let r = run(
        log,
        6_000,
        "alpha budget review again",
        vec![
            bash("t1", "head -c 16000 /dev/zero | tr '\\0' x"),
            say("Reviews 1 and 2 covered the alpha budget."),
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
    assert_eq!(r.requests.len(), 3, "a step, the summary, the next step");
    assert_eq!(
        carrying(&r.requests[0], &marker),
        3,
        "every exchange fits whole at first"
    );
    assert_eq!(
        compactions(&r.path),
        vec![(
            ids[2].clone(),
            ids[1].clone(),
            "Reviews 1 and 2 covered the alpha budget.".to_string()
        )]
    );
    let next = &r.requests[2];
    assert_eq!(carrying(next, "<context_summary>"), 1);
    assert_eq!(
        carrying(next, &marker),
        1,
        "the newest exchange stays whole"
    );
    assert!(
        next.messages.iter().any(|m| m.content.iter().any(
            |b| matches!(b, ContentBlock::ToolResult { content, .. } if content.len() >= 16_000)
        )),
        "the open turn stays verbatim"
    );
}

#[tokio::test]
async fn a_long_conversation_is_summarised_once_and_its_newest_exchange_rides_whole() {
    let dir = tempdir().unwrap();
    let mut log = open(dir.path());
    for n in 0..40 {
        past_turn(&mut log, &format!("unrelated filler number {n}"), "ok");
    }
    let ids: Vec<String> = log
        .visible_turn_cards_rev()
        .map(|(_, card)| card.turn_id.clone())
        .collect();
    let r = run(
        log,
        600,
        "something else entirely",
        vec![
            say("Forty filler questions were answered ok."),
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
    let summaries = compactions(&r.path);
    assert_eq!(summaries.len(), 1, "{summaries:?}");
    assert_eq!((&summaries[0].0, &summaries[0].1), (&ids[39], &ids[1]));
    let plans = recorded_plans(&r.path);
    let last = plans.last().unwrap();
    assert_eq!(last.packet_range, Some((ids[39].clone(), ids[1].clone())));
    assert_eq!(
        last.per_turn,
        vec![(ids[0].clone(), vak_session::Fidelity::Full)]
    );
    let request = r.requests.last().unwrap();
    assert_eq!(carrying(request, "<context_summary>"), 1);
    assert_eq!(carrying(request, "unrelated filler number 39"), 1);
    assert_eq!(carrying(request, "unrelated filler number 5"), 0);
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
    let entry_id = log
        .chain_to_root()
        .into_iter()
        .find_map(|entry| {
            matches!(
                entry.payload,
                vak_session::EntryPayload::ContextSelection(_)
            )
            .then_some(entry.id.clone())
        })
        .expect("a structured context selection");
    let messages = log.replay_context_selection(&entry_id).unwrap();

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
            input: serde_json::json!({"open": "ev:t1", "chars": {"start": 29_980, "end": 30_020}}),
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

/// A recall search of this conversation holds what its own turns held, not
/// someone else's words (invariant 43): a turn that only read private data
/// stays private after looking back.
#[tokio::test]
async fn a_recall_search_of_this_conversation_holds_what_its_turns_held() {
    let dir = tempdir().unwrap();
    let log = open(dir.path());
    let recall = msg(
        vec![ContentBlock::ToolUse {
            id: "r1".into(),
            name: "recall".into(),
            input: serde_json::json!({"query": "splitting the bill"}),
        }],
        StopReason::ToolUse,
    );
    let r = run(log, 50_000, "look back", vec![recall, say("done")], false).await;
    assert!(
        matches!(r.outcome, TurnOutcome::Completed { .. }),
        "{:?}",
        r.outcome
    );
    let held = SessionLog::open_read_only(r.path)
        .unwrap()
        .call_provenance("r1")
        .expect("labelled");
    assert!(held.private && !held.untrusted, "{held:?}");
}
