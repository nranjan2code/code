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

use vak_agent::{Agent, AgentConfig, AgentEvent, McpToolAlias, SteeringQueues, TurnOutcome};
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
    responses: std::sync::Mutex<VecDeque<AssistantMessage>>,
    requests: std::sync::Mutex<Vec<ChatRequest>>,
}

#[async_trait::async_trait]
impl Provider for Scripted {
    fn name(&self) -> &str {
        "scripted-general"
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

/// A report task that demands verification cannot finish with zero executed
/// commands; the built-in stop gate forces one continuation, then the run
/// completes only after real verification ran.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn stop_gate_blocks_premature_report_until_verified() {
    let (mut agent, _provider, dir) = setup(
        vec![
            text_msg("Done. The report is ready."),
            tool_call(
                "t1",
                "bash",
                serde_json::json!({
                    "command": "printf 'Q1 total: 4200\\n' > report.md && grep -q 4200 report.md"
                }),
            ),
            text_msg("Verified. Q1 total recorded as 4200 in report.md."),
        ],
        vec![Arc::new(BashTool)],
        |_| {},
    );

    let (ev_tx, ev_rx) = mpsc::channel(256);
    let collector = spawn_collector(ev_rx);

    let outcome = agent
        .run(
            "Compute the totals into report.md and verify the number appears in the file before you finish.",
            &Default::default(),
            CancellationToken::new(),
            ev_tx,
        )
        .await;
    let events = collector.await.unwrap();

    match outcome {
        TurnOutcome::Completed { response } => {
            assert!(response.text_content().contains("Verified"));
        }
        other => panic!("expected completed after gate continuation, got {other:?}"),
    }

    let continuations = events
        .iter()
        .filter(|e| matches!(e, AgentEvent::StopHookContinuation { .. }))
        .count();
    assert_eq!(continuations, 1, "gate must block exactly once");

    let guard_msgs = agent
        .session
        .lock()
        .await
        .derive_messages()
        .iter()
        .filter(|m| m.text_content().contains("[stop-guard]"))
        .count();
    assert_eq!(guard_msgs, 1, "guard continuation must be logged once");

    // Agent bash runs in its per-call quarantine directory; the report must
    // survive there rather than contaminating the workspace root.
    let report_path = if dir.path().join(".vak/scratch/vak/t1/report.md").exists() {
        dir.path().join(".vak/scratch/vak/t1/report.md")
    } else {
        dir.path().join(".vak/scratch/t1/report.md")
    };
    let report = std::fs::read_to_string(report_path)
        .expect("verified report survives in the quarantined scratch directory");
    assert!(report.contains("4200"));
}

/// A long research session crosses the context trigger; compaction
/// summarizes older turns into a ledger entry (never deleting them), the
/// projection carries the summary forward, and the run completes.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn compaction_during_long_research_session() {
    let big_line = "Research note covering climate data points and energy market shifts.\n";
    let big_a = big_line.repeat(87);
    let big_b = big_line.repeat(87);

    let (mut agent, provider, dir) = setup(
        vec![
            tool_call("r1", "read", serde_json::json!({"path": "big-a.txt"})),
            tool_call("r2", "read", serde_json::json!({"path": "big-b.txt"})),
            text_msg(
                "Summary: both sources reviewed; key climate and energy findings retained for the brief.",
            ),
            tool_call(
                "w1",
                "write",
                serde_json::json!({
                    "path": "digest.txt",
                    "content": "Digest: sources A and B agree on the retained findings.\n"
                }),
            ),
            text_msg("digest written"),
        ],
        vec![Arc::new(ReadTool), Arc::new(WriteTool)],
        |cfg| {
            cfg.context_policy = vak_agent::context::ContextPolicy {
                context_window: 4000,
                max_output: 256,
                compact_threshold: 0.8,
                keep_recent: 2,
            };
        },
    );

    for (name, content) in [("big-a.txt", &big_a), ("big-b.txt", &big_b)] {
        std::fs::write(dir.path().join(name), content).unwrap();
    }

    let (ev_tx, ev_rx) = mpsc::channel(4096);
    let collector = spawn_collector(ev_rx);

    let outcome = agent
        .run(
            "Read both source files and write digest.txt summarizing their key facts.",
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
    assert_eq!(continuations, 1, "compaction must trigger exactly once");
    let compacted = events.iter().find_map(|e| match e {
        AgentEvent::ContextCompacted {
            before_tokens,
            after_tokens,
            ..
        } => Some((*before_tokens, *after_tokens)),
        _ => None,
    });
    let (before, after) = compacted.expect("compacted event");
    assert!(after < before, "compaction must shrink the estimate");

    // The summarizer ran as its own model call against the transcript, and
    // the whole run consumed exactly the scripted trajectory.
    {
        let reqs = provider.requests.lock().unwrap();
        assert_eq!(reqs.len(), 5, "two reads + compaction + write + final");
        assert!(
            reqs[2]
                .system
                .as_deref()
                .unwrap_or("")
                .contains("compactor"),
            "third request must be the compaction call"
        );
    }

    // Append-only invariant: raw ledger still holds pre-compaction history
    // plus exactly one compaction entry; projection carries the summary.
    let session_file =
        SessionPath::new_session_file(&dir.path().join(".vak-home"), dir.path(), "general-flow");
    let raw = std::fs::read_to_string(&session_file).unwrap();
    let compaction_lines = raw
        .lines()
        .filter(|l| l.contains("\"kind\":\"compaction\""))
        .count();
    assert_eq!(compaction_lines, 1, "exactly one compaction entry expected");
    assert!(
        raw.contains("Research note covering climate data points"),
        "original history must never be deleted from the ledger"
    );

    let projected = agent.session.lock().await.derive_messages();
    assert!(
        projected[0].text_content().starts_with("<context_summary>"),
        "projection must lead with the summary"
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
            network: false,
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
                    registered.insert(
                        tool.name.clone(),
                        McpToolAlias {
                            server: server.clone(),
                            tool: tool.name.clone(),
                            description: tool.description.clone(),
                            schema: tool.input_schema.clone(),
                        },
                    );
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
            config.mcp_aliases = aliases.clone();
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
            network: false,
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
                    registered.insert(
                        tool.name.clone(),
                        McpToolAlias {
                            server: server.clone(),
                            tool: tool.name.clone(),
                            description: tool.description.clone(),
                            schema: tool.input_schema.clone(),
                        },
                    );
                }
            }
        }),
    ));

    // Pre-populate alias as would be done from admitted capability inventory
    aliases.lock().unwrap().insert(
        "echo".into(),
        McpToolAlias {
            server: "notes".into(),
            tool: "echo".into(),
            description: "echo text back".into(),
            schema: serde_json::json!({"properties": {"text": {"type": "string"}}, "required": ["text"]}),
        },
    );

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
            config.mcp_aliases = aliases.clone();
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
    let denied = agent
        .session
        .lock()
        .await
        .derive_messages()
        .iter()
        .flat_map(|m| m.content.iter())
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
