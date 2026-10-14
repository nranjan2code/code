//! The provenance gate (docs/design/08-permissions.md "Provenance"): a call
//! that can leave the machine goes to a person once the turn holds the
//! owner's data and words someone else wrote together.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;

use vak_agent::{Agent, AgentConfig, ApprovalMode, Approver, TurnOutcome};
use vak_llm::Provider;
use vak_llm::types::{AssistantMessage, ContentBlock, StopReason, Usage};
use vak_permission::{Declared, Mode, PermissionEngine, Reach};
use vak_session::SessionLog;
use vak_session::types::{CallEffect, EntryPayload, FrozenContract, SessionHeader};
use vak_tools::{Tool, ToolContext, ToolOutput};

/// A tool of the given reach that counts its runs.
struct Fake {
    name: &'static str,
    reach: Reach,
    runs: Arc<AtomicUsize>,
}

#[async_trait::async_trait]
impl Tool for Fake {
    fn name(&self) -> &str {
        self.name
    }
    fn description(&self) -> &str {
        "a fake"
    }
    fn schema(&self) -> serde_json::Value {
        serde_json::json!({"type": "object", "properties": {}})
    }
    fn permission(&self) -> Declared {
        Declared::reach(self.reach)
    }
    async fn execute(&self, _args: &serde_json::Value, _ctx: &ToolContext) -> ToolOutput {
        self.runs.fetch_add(1, Ordering::SeqCst);
        ToolOutput::ok(format!("{} result", self.name))
    }
}

/// Records every gate it is asked, answering `answer`.
struct Recording {
    asked: std::sync::Mutex<Vec<(String, String)>>,
    answer: bool,
    answerable: bool,
}

#[async_trait::async_trait]
impl Approver for Recording {
    async fn approve(
        &self,
        tool: &str,
        _args_json: &str,
        reason: &str,
        _call_id: Option<&str>,
    ) -> bool {
        self.asked
            .lock()
            .unwrap()
            .push((tool.to_string(), reason.to_string()));
        self.answer
    }
    fn answerable(&self) -> bool {
        self.answerable
    }
}

fn recording(answer: bool, answerable: bool) -> Arc<Recording> {
    Arc::new(Recording {
        asked: std::sync::Mutex::new(Vec::new()),
        answer,
        answerable,
    })
}

fn call(id: &str, name: &str) -> AssistantMessage {
    AssistantMessage {
        content: vec![ContentBlock::ToolUse {
            id: id.into(),
            name: name.into(),
            input: serde_json::json!({}),
        }],
        stop_reason: StopReason::ToolUse,
        usage: Usage::default(),
        model: "test-model".into(),
        response_id: None,
        applied: None,
    }
}

fn text(t: &str) -> AssistantMessage {
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
        applied: None,
    }
}

struct Scripted {
    capacity_key: String,
    msgs: std::sync::Mutex<std::collections::VecDeque<AssistantMessage>>,
}

#[async_trait::async_trait]
impl Provider for Scripted {
    fn name(&self) -> &str {
        "scripted"
    }
    fn rate_limit_key(&self) -> String {
        self.capacity_key.clone()
    }
    async fn stream(
        &self,
        _request: vak_llm::types::ChatRequest,
        _cancel: CancellationToken,
    ) -> Result<vak_llm::EventStream, vak_llm::LlmError> {
        let next = self.msgs.lock().unwrap().pop_front();
        let (mut sink, rx) = vak_llm::stream::channel(8);
        match next {
            Some(m) => {
                sink.push(vak_llm::stream::StreamEvent::Start { partial: m.clone() });
                sink.close_message(m).await;
            }
            None => {
                sink.close_error(vak_llm::LlmError::Parse("exhausted".into()))
                    .await;
            }
        }
        Ok(rx)
    }
}

struct Setup {
    agent: Agent,
    reads: Arc<AtomicUsize>,
    fetches: Arc<AtomicUsize>,
}

fn setup(
    responses: Vec<AssistantMessage>,
    approver: Arc<Recording>,
    tune: impl FnOnce(&mut AgentConfig),
) -> Setup {
    let dir = tempfile::tempdir().unwrap();
    let header = SessionHeader {
        space: None,
        run: None,
        cause: None,
        agent: None,
        session_id: "s".into(),
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
    let log = SessionLog::create(dir.path().join("s.jsonl"), header).unwrap();
    let reads = Arc::new(AtomicUsize::new(0));
    let fetches = Arc::new(AtomicUsize::new(0));
    let mut cfg = AgentConfig::new("sys");
    cfg.tools = vec![
        Arc::new(Fake {
            name: "readfile",
            reach: Reach::ReadPath,
            runs: reads.clone(),
        }),
        Arc::new(Fake {
            name: "fetch",
            reach: Reach::Network,
            runs: fetches.clone(),
        }),
    ];
    // The gate holds in the sandboxed modes; Full access skips it
    // (`full_access_never_raises_the_gate`). Rules allow both tools here,
    // so any ask is the gate's.
    cfg.mode = Mode::WorkspaceWrite;
    cfg.permission = Some(Arc::new(PermissionEngine::new(vec![
        vak_permission::Rule::parse("readfile").unwrap(),
        vak_permission::Rule::parse("fetch").unwrap(),
    ])));
    cfg.approver = Some(approver);
    cfg.stop_policy = None;
    tune(&mut cfg);
    std::mem::forget(dir);
    let provider = Scripted {
        capacity_key: crate::support::capacity_key(),
        msgs: std::sync::Mutex::new(responses.into_iter().collect()),
    };
    Setup {
        agent: Agent::new(Arc::new(provider), log, cfg),
        reads,
        fetches,
    }
}

async fn run(agent: &mut Agent, prompt: &str) -> TurnOutcome {
    agent
        .run(
            prompt,
            &Default::default(),
            CancellationToken::new(),
            mpsc::channel(64).0,
        )
        .await
}

#[tokio::test]
async fn reading_only_the_web_never_asks() {
    let approver = recording(false, true);
    let mut s = setup(
        vec![call("c1", "fetch"), call("c2", "fetch"), text("done")],
        approver.clone(),
        |_| {},
    );
    assert!(matches!(
        run(&mut s.agent, "go").await,
        TurnOutcome::Completed { .. }
    ));
    assert_eq!(s.fetches.load(Ordering::SeqCst), 2);
    assert!(approver.asked.lock().unwrap().is_empty());
}

#[tokio::test]
async fn owner_data_and_the_web_together_put_the_next_exit_to_a_person() {
    let approver = recording(false, true);
    let mut s = setup(
        vec![
            call("c1", "readfile"),
            call("c2", "fetch"),
            call("c3", "fetch"),
            text("done"),
        ],
        approver.clone(),
        |_| {},
    );
    assert!(matches!(
        run(&mut s.agent, "go").await,
        TurnOutcome::Completed { .. }
    ));
    assert_eq!(s.reads.load(Ordering::SeqCst), 1);
    // The first fetch runs (the turn held only the owner's data); the
    // second is asked and declined, so it never runs.
    assert_eq!(s.fetches.load(Ordering::SeqCst), 1);
    let asked = approver.asked.lock().unwrap().clone();
    assert_eq!(asked.len(), 1, "{asked:?}");
    assert_eq!(asked[0].0, "fetch");
    assert!(asked[0].1.contains("from readfile"), "{}", asked[0].1);
    assert!(asked[0].1.contains("from fetch"), "{}", asked[0].1);

    let session = s.agent.session.lock().await;
    let labels: Vec<_> = session
        .chain_to_root()
        .into_iter()
        .filter_map(|entry| match &entry.payload {
            EntryPayload::CallEffect(record) => match record.effect {
                CallEffect::Provenance(labels) => Some((record.tool_use_id.clone(), labels)),
                _ => None,
            },
            _ => None,
        })
        .collect();
    assert!(labels.contains(&("c1".into(), vak_control::Provenance::PRIVATE)));
    assert!(labels.contains(&("c2".into(), vak_control::Provenance::UNTRUSTED)));
    let card = session.turn_cards().pop().expect("the turn has its card").1;
    assert_eq!(card.provenance, vak_control::Provenance::BOTH);
}

/// Full access is the owner's explicit trust decision (invariant 13): the
/// same turn that asks above runs without asking there (maintainer,
/// 2026-10-14).
#[tokio::test]
async fn full_access_never_raises_the_gate() {
    let approver = recording(false, true);
    let mut s = setup(
        vec![
            call("c1", "readfile"),
            call("c2", "fetch"),
            call("c3", "fetch"),
            text("done"),
        ],
        approver.clone(),
        |cfg| cfg.mode = Mode::FullAccess,
    );
    assert!(matches!(
        run(&mut s.agent, "go").await,
        TurnOutcome::Completed { .. }
    ));
    assert_eq!(s.fetches.load(Ordering::SeqCst), 2);
    assert!(approver.asked.lock().unwrap().is_empty());
}

#[tokio::test]
async fn nothing_pre_authorizes_the_gate() {
    let approver = recording(false, true);
    let mut s = setup(
        vec![
            call("c1", "readfile"),
            call("c2", "fetch"),
            call("c3", "fetch"),
            text("done"),
        ],
        approver.clone(),
        |cfg| cfg.approval_mode = ApprovalMode::AutoApprove,
    );
    run(&mut s.agent, "go").await;
    assert_eq!(approver.asked.lock().unwrap().len(), 1);
    assert_eq!(s.fetches.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn an_unattended_surface_refuses_the_exit() {
    let approver = recording(true, false);
    let mut s = setup(
        vec![
            call("c1", "readfile"),
            call("c2", "fetch"),
            call("c3", "fetch"),
            text("done"),
        ],
        approver.clone(),
        |_| {},
    );
    run(&mut s.agent, "go").await;
    assert_eq!(s.fetches.load(Ordering::SeqCst), 1);
    assert!(approver.asked.lock().unwrap().is_empty());
}

#[tokio::test]
async fn an_approved_exit_runs() {
    let approver = recording(true, true);
    let mut s = setup(
        vec![
            call("c1", "readfile"),
            call("c2", "fetch"),
            call("c3", "fetch"),
            text("done"),
        ],
        approver.clone(),
        |_| {},
    );
    run(&mut s.agent, "go").await;
    assert_eq!(s.fetches.load(Ordering::SeqCst), 2);
    assert_eq!(approver.asked.lock().unwrap().len(), 1);
}

#[tokio::test]
async fn the_next_turn_carries_what_the_last_one_held() {
    let approver = recording(false, true);
    let mut s = setup(
        vec![
            call("c1", "readfile"),
            call("c2", "fetch"),
            text("first answer"),
            call("c3", "fetch"),
            text("second answer"),
        ],
        approver.clone(),
        |_| {},
    );
    run(&mut s.agent, "first").await;
    assert!(approver.asked.lock().unwrap().is_empty());
    run(&mut s.agent, "second").await;
    let asked = approver.asked.lock().unwrap().clone();
    assert_eq!(asked.len(), 1, "{asked:?}");
    assert!(
        asked[0].1.contains(vak_control::provenance::EARLIER_TURNS),
        "{}",
        asked[0].1
    );
    assert_eq!(s.fetches.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn a_delegated_worker_starts_with_what_its_parent_held() {
    let approver = recording(false, true);
    let mut s = setup(
        vec![call("c1", "fetch"), text("done")],
        approver.clone(),
        |cfg| cfg.inherited_provenance = vak_control::Provenance::BOTH,
    );
    run(&mut s.agent, "go").await;
    assert_eq!(s.fetches.load(Ordering::SeqCst), 0);
    let asked = approver.asked.lock().unwrap().clone();
    assert_eq!(asked.len(), 1);
    assert!(asked[0].1.contains(vak_control::provenance::DELEGATED_FROM));
}

#[tokio::test]
async fn a_declined_tool_leaves_the_turn() {
    let approver = recording(false, true);
    let mut s = setup(
        vec![
            call("c1", "readfile"),
            call("c2", "fetch"),
            call("c3", "fetch"),
            call("c4", "fetch"),
            text("done"),
            call("c5", "fetch"),
            text("done again"),
        ],
        approver.clone(),
        |_| {},
    );
    run(&mut s.agent, "go").await;
    // The person declined c3, so `fetch` is out of the turn: c4 never
    // reaches the person and never runs.
    assert_eq!(approver.asked.lock().unwrap().len(), 1);
    assert_eq!(s.fetches.load(Ordering::SeqCst), 1);
    let session = s.agent.session.lock().await;
    let blocks: Vec<ContentBlock> = session
        .message_chain()
        .iter()
        .flat_map(|(_, m)| m.content.clone())
        .collect();
    assert!(blocks.iter().any(|block| matches!(block,
        ContentBlock::Text { text } if text.contains("`fetch` is withdrawn for the rest of this turn"))));
    assert!(blocks.iter().any(|block| matches!(block,
        ContentBlock::ToolResult { tool_use_id, is_error: true, .. } if tool_use_id == "c4")));
    drop(session);
    // The next turn offers it again, and its call reaches the person.
    run(&mut s.agent, "again").await;
    assert_eq!(approver.asked.lock().unwrap().len(), 2);
}
