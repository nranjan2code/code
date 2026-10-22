// A test's output is for the person running it.
#![allow(clippy::disallowed_macros)]
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

//! General-purpose (non-coding) scenario coverage. The same harness
//! machinery — steering, abort, stop gate, compaction, MCP, permissions —
//! exercised through research, writing, planning, and data-analysis flows
//! instead of code tasks.

use std::collections::{HashMap, VecDeque};
use std::path::Path;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;

use tempfile::tempdir;

use vak_agent::{Agent, AgentConfig, AgentEvent, SteeringQueues, TurnOutcome};
use vak_llm::stream;
use vak_llm::types::{AssistantMessage, ChatRequest, ContentBlock, StopReason, Usage};
use vak_llm::{EventStream, LlmError, Provider};
use vak_mcp::{McpManager, McpTool, ServerConfig};
use vak_permission::{Mode, PermissionEngine};
use vak_session::types::{FrozenContract, SessionHeader};
use vak_session::{SessionLog, SessionPath};
use vak_tools::bash::BashTool;
use vak_tools::read::ReadTool;
use vak_tools::write::WriteTool;

struct Scripted {
    capacity_key: String,
    responses: std::sync::Mutex<VecDeque<AssistantMessage>>,
    requests: std::sync::Mutex<Vec<ChatRequest>>,
}

#[async_trait::async_trait]
impl Provider for Scripted {
    fn name(&self) -> &str {
        "scripted-general"
    }

    fn rate_limit_key(&self) -> String {
        self.capacity_key.clone()
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
            None => {
                sink.close_error(LlmError::Parse("script exhausted".into()))
                    .await
            }
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
        applied: None,
    }
}

fn tool_call(id: &str, name: &str, input: serde_json::Value) -> AssistantMessage {
    AssistantMessage {
        content: vec![ContentBlock::ToolUse {
            id: id.into(),
            name: name.into(),
            input,
        }],
        stop_reason: StopReason::ToolUse,
        usage: Usage::default(),
        model: "test-model".into(),
        response_id: None,
        applied: None,
    }
}

fn setup(
    responses: Vec<AssistantMessage>,
    tools: Vec<Arc<dyn vak_tools::Tool>>,
    customize: impl FnOnce(&mut AgentConfig),
) -> (Agent, Arc<Scripted>, tempfile::TempDir) {
    let dir = tempdir().unwrap();
    let cwd = dir.path().to_path_buf();
    let header = SessionHeader {
        space: None,
        run: None,
        cause: None,
        agent: None,
        session_id: "general-flow".into(),
        created_at: chrono::Utc::now(),
        cwd: cwd.clone(),
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
    let home = cwd.join(".vak-home");
    std::fs::create_dir_all(&home).unwrap();
    let log = SessionLog::create(
        SessionPath::new_session_file(&home, &cwd, "general-flow"),
        header,
    )
    .unwrap();
    let provider = Arc::new(Scripted {
        capacity_key: crate::support::capacity_key(),
        responses: std::sync::Mutex::new(responses.into_iter().collect()),
        requests: std::sync::Mutex::new(Vec::new()),
    });
    let mut cfg = AgentConfig::new("sys");
    cfg.model = "test-model".into();
    cfg.tools = tools;
    cfg.mode = Mode::FullAccess;
    cfg.permission = Some(Arc::new(PermissionEngine::default()));
    cfg.approver = Some(Arc::new(vak_agent::AutoApprove));
    customize(&mut cfg);
    let agent = Agent::new(provider.clone(), log, cfg);
    (agent, provider, dir)
}

fn spawn_collector(mut rx: mpsc::Receiver<AgentEvent>) -> tokio::task::JoinHandle<Vec<AgentEvent>> {
    tokio::spawn(async move {
        let mut out = Vec::new();
        while let Some(ev) = rx.recv().await {
            out.push(ev);
        }
        out
    })
}

/// The user steers mid-run while the first step is still executing; the
/// steer must enter the ledger before the next model call and shape the
/// final artifact.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn steering_redirects_trip_plan_mid_run() {
    let (mut agent, _provider, dir) = setup(
        vec![
            tool_call("t1", "bash", serde_json::json!({"command": "sleep 0.6"})),
            tool_call(
                "t2",
                "write",
                serde_json::json!({
                    "path": "trip-plan.md",
                    "content": "# Trip Plan\n\nTotal budget stays under $1500, with two full days in Kyoto.\n"
                }),
            ),
            text_msg("plan finalized"),
        ],
        vec![Arc::new(BashTool), Arc::new(WriteTool)],
        |_| {},
    );

    let steering = Arc::new(SteeringQueues::new());
    let pusher = {
        let steering = steering.clone();
        tokio::spawn(async move {
            tokio::time::sleep(Duration::from_millis(150)).await;
            steering.push_steering("Budget is under $1500 total, and include two days in Kyoto.");
        })
    };

    let (ev_tx, ev_rx) = mpsc::channel(256);
    drop(spawn_collector(ev_rx));

    let outcome = agent
        .run(
            "Plan our Japan trip and save it to trip-plan.md.",
            &steering,
            CancellationToken::new(),
            ev_tx,
        )
        .await;
    pusher.await.unwrap();

    assert!(
        matches!(outcome, TurnOutcome::Completed { .. }),
        "got {outcome:?}"
    );

    // Model-visible input must be reconstructable from the ledger.
    let steered = agent
        .session
        .lock()
        .await
        .derive_messages()
        .iter()
        .any(|m| m.text_content().contains("under $1500"));
    assert!(steered, "steering message must be logged in the session");

    let draft = std::fs::read_to_string(dir.path().join("trip-plan.md")).unwrap();
    assert!(
        draft.contains("$1500") && draft.to_lowercase().contains("kyoto"),
        "final artifact must reflect the steering"
    );
}

/// Cancelling mid-run keeps everything the agent already wrote to disk.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn abort_preserves_partial_research_notes() {
    let cancel = CancellationToken::new();
    let (mut agent, _provider, dir) = setup(
        vec![
            tool_call(
                "t1",
                "write",
                serde_json::json!({
                    "path": "notes-draft.md",
                    "content": "# Research Notes\n\n- source A reviewed\n"
                }),
            ),
            tool_call("t2", "bash", serde_json::json!({"command": "sleep 30"})),
            text_msg("never reached"),
        ],
        vec![Arc::new(WriteTool), Arc::new(BashTool)],
        |_| {},
    );

    let watcher_cancel = cancel.clone();
    let watch_path = dir.path().join("notes-draft.md");
    let watcher = tokio::spawn(async move {
        loop {
            if watch_path.exists() {
                tokio::time::sleep(Duration::from_millis(50)).await;
                watcher_cancel.cancel();
                break;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    });

    let outcome = agent
        .run(
            "Compile research notes into notes-draft.md.",
            &Default::default(),
            cancel,
            mpsc::channel(64).0,
        )
        .await;
    watcher.abort();

    assert!(
        matches!(outcome, TurnOutcome::Aborted { .. }),
        "expected abort, got {outcome:?}"
    );
    let partial =
        std::fs::read_to_string(dir.path().join("notes-draft.md")).expect("partial file survives");
    assert!(partial.contains("source A reviewed"));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn long_research_history_in_a_narrow_window_is_summarised_once() {
    let big_a = "Research note on climate data points.\n".repeat(6);
    let big_b = "Research note on energy market shifts.\n".repeat(6);

    let dir = tempdir().unwrap();
    let cwd = dir.path().to_path_buf();
    let header = SessionHeader {
        space: None,
        run: None,
        cause: None,
        agent: None,
        session_id: "general-flow".into(),
        created_at: chrono::Utc::now(),
        cwd: cwd.clone(),
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
    let home = cwd.join(".vak-home");
    std::fs::create_dir_all(&home).unwrap();
    let mut log = SessionLog::create(
        SessionPath::new_session_file(&home, &cwd, "general-flow"),
        header,
    )
    .unwrap();
    // These carded turns are deliberately large: the narrow window cannot
    // hold them whole, so the request carries one summary of them.
    let seed_filler = "prior research finding ".repeat(220);
    for i in 0..30 {
        let turn_id = log
            .append_message(vak_session::types::MessageRecord {
                message: vak_llm::types::Message::user_text(format!(
                    "Research climate data and energy market shifts, round {i}: {}",
                    "compare retained climate and energy evidence across the relevant earlier research. "
                        .repeat(12)
                )),
                meta: None,
            })
            .unwrap()
            .id;
        let answer = format!("acknowledged note {i}: {seed_filler}");
        log.append_message(vak_session::types::MessageRecord {
            message: vak_llm::types::Message::assistant(vec![ContentBlock::text(&answer)]),
            meta: None,
        })
        .unwrap();
        let card = vak_session::TurnIndex::from_log(&log)
            .turn_by_id(&turn_id)
            .unwrap()
            .build_card("completed", answer, &|s| s.len() as u64 / 4);
        log.append_turn_card(vak_session::types::TurnCardRecord { turn_id, card })
            .unwrap();
    }
    let provider = Arc::new(Scripted {
        capacity_key: crate::support::capacity_key(),
        responses: std::sync::Mutex::new(VecDeque::from(vec![
            text_msg("Summary: thirty earlier climate and energy research rounds."),
            tool_call("r1", "read", serde_json::json!({"path": "big-a.txt"})),
            tool_call("r2", "read", serde_json::json!({"path": "big-b.txt"})),
            tool_call(
                "w1",
                "write",
                serde_json::json!({
                    "path": "digest.txt",
                    "content": "Digest: sources A and B agree on the retained findings.\n"
                }),
            ),
            text_msg("digest written"),
        ])),
        requests: std::sync::Mutex::new(Vec::new()),
    });
    let mut cfg = AgentConfig::new("sys");
    cfg.model = "test-model".into();
    cfg.tools = vec![Arc::new(ReadTool), Arc::new(WriteTool)];
    cfg.mode = Mode::FullAccess;
    cfg.permission = Some(Arc::new(PermissionEngine::default()));
    cfg.approver = Some(Arc::new(vak_agent::AutoApprove));
    // A narrow discovered horizon: the old turns cannot ride whole.
    cfg.declared_window = Some(1700);
    cfg.max_output = 100;
    // Handoff reset is independent; this test isolates request projection.
    cfg.handoff_reset = false;
    let mut agent = Agent::new(provider.clone(), log, cfg);

    for (name, content) in [("big-a.txt", &big_a), ("big-b.txt", &big_b)] {
        std::fs::write(dir.path().join(name), content).unwrap();
    }

    let (ev_tx, ev_rx) = mpsc::channel(4096);
    let collector = spawn_collector(ev_rx);

    let outcome = agent
        .run(
            "Read both source files and write digest.txt summarizing their climate and energy facts.",
            &Default::default(),
            CancellationToken::new(),
            ev_tx,
        )
        .await;
    let events = collector.await.unwrap();

    match outcome {
        TurnOutcome::Completed { response } => {
            assert_eq!(response.text_content(), "digest written");
        }
        other => panic!("expected completed after compaction, got {other:?}"),
    }
    let continuations = events
        .iter()
        .filter(|e| matches!(e, AgentEvent::ContextCompacting { .. }))
        .count();
    assert_eq!(
        continuations, 1,
        "one summary, written before the first step and reused by the rest"
    );

    // The summary request, then the current task's steps, none of which
    // repeats a seeded research turn's record whole.
    {
        let reqs = provider.requests.lock().unwrap();
        assert_eq!(reqs.len(), 5, "summary + two reads + write + final");
        assert!(reqs[1..].iter().all(|request| {
            request
                .messages
                .iter()
                .any(|message| message.text_content().contains("<context_summary>"))
        }));
        assert!(reqs[1..].iter().all(|request| {
            !request
                .messages
                .iter()
                .any(|message| message.text_content().contains(seed_filler.as_str()))
        }));
    }

    // History remains append-only and recallable in the source ledger
    // beside the one compaction entry.
    let session_file =
        SessionPath::new_session_file(&dir.path().join(".vak-home"), dir.path(), "general-flow");
    let raw = vak_session::SessionLog::text(&session_file);
    let compaction_lines = raw
        .lines()
        .filter(|l| l.contains("\"kind\":\"compaction\""))
        .count();
    assert_eq!(compaction_lines, 1, "one compaction entry expected");
    assert!(
        raw.contains("prior research finding"),
        "original history must never be deleted from the ledger"
    );

    // The plan-free ledger projection still holds the old turns whole for
    // explicit recall.
    let projected = agent.session.lock().await.derive_messages();
    assert!(
        projected
            .iter()
            .any(|m| m.text_content().contains("prior research finding")),
        "omitting history from this request must not remove it from the ledger"
    );

    let digest = std::fs::read_to_string(dir.path().join("digest.txt")).unwrap();
    assert!(digest.contains("Digest:"));
}

/// A non-code lookup through the MCP meta-tool (list → call → artifact).
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn mcp_notes_lookup_informs_planning_answer() {
    let Ok(probe) = std::process::Command::new("python3")
        .arg("--version")
        .output()
    else {
        eprintln!("skipping: python3 unavailable");
        return;
    };
    assert!(probe.status.success(), "python3 must be runnable");

    let script = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../scripts/fake_mcp_server.py");
    let mut servers = HashMap::new();
    servers.insert(
        "notes".to_string(),
        ServerConfig {
            command: "python3".into(),
            args: vec![script.display().to_string()],
            env: Vec::new(),
            hosts: Vec::new(),
        },
    );
    let manager = Arc::new(McpManager::new(servers, dir_safe_cwd()));
    let aliases = Arc::new(Mutex::new(HashMap::new()));
    let aliases_after_list = aliases.clone();
    let mcp_tool: Arc<dyn vak_tools::Tool> = Arc::new(McpTool::new(manager).with_catalog_observer(
        Arc::new(move |catalog| {
            let mut registered = aliases_after_list.lock().unwrap();
            for (server, tools) in catalog {
                for tool in tools {
                    registered.insert(tool.name.clone(), server.clone());
                }
            }
        }),
    ));

    let (mut agent, _provider, dir) = setup(
        vec![
            tool_call("m1", "mcp", serde_json::json!({"action": "list"})),
            tool_call("m2", "echo", serde_json::json!({"text": "flights booked"})),
            tool_call(
                "w1",
                "write",
                serde_json::json!({
                    "path": "answer.md",
                    "content": "# Status\n\nMCP says: echo: flights booked\n"
                }),
            ),
            text_msg("answered via mcp"),
        ],
        vec![mcp_tool, Arc::new(WriteTool)],
        |config| {
            config.mcp_tool_index = aliases.clone();
        },
    );

    let outcome = agent
        .run(
            "Ask the notes service about our booking and record the reply in answer.md.",
            &Default::default(),
            CancellationToken::new(),
            mpsc::channel(256).0,
        )
        .await;

    assert!(
        matches!(outcome, TurnOutcome::Completed { .. }),
        "got {outcome:?}"
    );
    let answer = std::fs::read_to_string(dir.path().join("answer.md")).unwrap();
    assert!(
        answer.contains("echo: flights booked"),
        "artifact must carry the MCP result"
    );
}

#[tokio::test]
async fn mcp_call_omitting_server_auto_resolves_and_completes() {
    let Ok(probe) = std::process::Command::new("python3")
        .arg("--version")
        .output()
    else {
        eprintln!("skipping: python3 unavailable");
        return;
    };
    assert!(probe.status.success(), "python3 must be runnable");

    let script = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../scripts/fake_mcp_server.py");
    let mut servers = HashMap::new();
    servers.insert(
        "notes".to_string(),
        ServerConfig {
            command: "python3".into(),
            args: vec![script.display().to_string()],
            env: Vec::new(),
            hosts: Vec::new(),
        },
    );
    let manager = Arc::new(McpManager::new(servers, dir_safe_cwd()));
    let aliases = Arc::new(Mutex::new(HashMap::new()));
    let aliases_map = aliases.clone();
    let mcp_tool: Arc<dyn vak_tools::Tool> = Arc::new(McpTool::new(manager).with_catalog_observer(
        Arc::new(move |catalog| {
            let mut registered = aliases_map.lock().unwrap();
            for (server, tools) in catalog {
                for tool in tools {
                    registered.insert(tool.name.clone(), server.clone());
                }
            }
        }),
    ));

    // Pre-populate alias as would be done from admitted capability inventory
    aliases
        .lock()
        .unwrap()
        .insert("echo".into(), "notes".into());

    let (mut agent, _provider, dir) = setup(
        vec![
            // Model calls `mcp` with action=call and tool=echo, but completely OMITS `server`!
            tool_call(
                "m1",
                "mcp",
                serde_json::json!({
                    "action": "call",
                    "tool": "echo",
                    "arguments": {"text": "flights booked"}
                }),
            ),
            tool_call(
                "w1",
                "write",
                serde_json::json!({
                    "path": "auto_resolved.md",
                    "content": "# Result\n\nAuto-resolved echo succeeded\n"
                }),
            ),
            text_msg("auto-resolved and done"),
        ],
        vec![mcp_tool, Arc::new(WriteTool)],
        |config| {
            config.mcp_tool_index = aliases.clone();
        },
    );

    let outcome = agent
        .run(
            "Echo flights booked without specifying server",
            &Default::default(),
            CancellationToken::new(),
            mpsc::channel(256).0,
        )
        .await;

    assert!(
        matches!(outcome, TurnOutcome::Completed { .. }),
        "got {outcome:?}"
    );
    assert!(dir.path().join("auto_resolved.md").is_file());
}

fn dir_safe_cwd() -> std::path::PathBuf {
    std::env::current_dir().unwrap_or_else(|_| std::path::PathBuf::from("."))
}

/// In read-only mode a note-taking write is denied with a typed reason; the
/// model adapts by delivering the content inline and nothing hits disk.
#[tokio::test]
async fn read_only_mode_denies_note_writes_and_agent_reports_inline() {
    let (mut agent, _provider, dir) = setup(
        vec![
            tool_call(
                "t1",
                "write",
                serde_json::json!({
                    "path": "field-notes.md",
                    "content": "# Field Notes\n\nObservation one recorded on site.\n"
                }),
            ),
            text_msg(
                "Saving is unavailable in read-only mode. Field notes inline: Observation one recorded on site.",
            ),
        ],
        vec![Arc::new(WriteTool)],
        |cfg| {
            cfg.mode = Mode::ReadOnly;
        },
    );

    let outcome = agent
        .run(
            "Record today's field observation in field-notes.md.",
            &Default::default(),
            CancellationToken::new(),
            mpsc::channel(64).0,
        )
        .await;

    match outcome {
        TurnOutcome::Completed { response } => {
            assert!(response.text_content().contains("inline"));
        }
        other => panic!("denied write must not fail the run, got {other:?}"),
    }
    assert!(
        !dir.path().join("field-notes.md").exists(),
        "denied write must never touch disk"
    );
    // Raw ledger: the closed turn's result is a trace line in the
    // projection now (docs/design/68-context-engine.md §10); this checks
    // what actually got recorded.
    let denied = agent
        .session
        .lock()
        .await
        .message_chain()
        .iter()
        .flat_map(|(_, m)| m.content.iter())
        .find_map(|b| match b {
            ContentBlock::ToolResult {
                content, is_error, ..
            } => Some((content.clone(), *is_error)),
            _ => None,
        })
        .expect("denied call must produce a tool result");
    assert!(denied.1, "denied write is an error result");
    assert!(denied.0.contains("read-only mode denies"));
}
