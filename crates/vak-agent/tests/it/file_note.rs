#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

//! A call that touches a workspace path gets the record's note about that
//! path attached to its result, whether it worked or not (doc 90 §3.2). A
//! call whose tool declares no file access gets nothing, and the loop names
//! no tool.

use std::collections::VecDeque;
use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use serde_json::{Value, json};
use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;

use vak_agent::{Agent, AgentConfig, TurnOutcome};
use vak_llm::stream;
use vak_llm::types::{AssistantMessage, ChatRequest, ContentBlock, StopReason, Usage};
use vak_llm::{EventStream, LlmError, Provider};
use vak_permission::Mode;
use vak_session::types::{FrozenContract, SessionHeader};
use vak_session::{SessionLog, SessionPath};
use vak_tools::{Fault, FileAccess, Tool, ToolContext, ToolOutput};

struct Scripted(
    Mutex<VecDeque<AssistantMessage>>,
    Arc<Mutex<Vec<ChatRequest>>>,
    String,
);

#[async_trait]
impl Provider for Scripted {
    fn name(&self) -> &str {
        "scripted"
    }

    fn rate_limit_key(&self) -> String {
        self.2.clone()
    }

    async fn stream(
        &self,
        request: ChatRequest,
        _cancel: CancellationToken,
    ) -> Result<EventStream, LlmError> {
        self.1.lock().unwrap().push(request);
        let next = self.0.lock().unwrap().pop_front();
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

fn msg(content: Vec<ContentBlock>, stop_reason: StopReason) -> AssistantMessage {
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

/// A tool that names the workspace path it writes, as `write` does.
struct StoreFile {
    fail: bool,
}

#[async_trait]
impl Tool for StoreFile {
    fn name(&self) -> &str {
        "store_file"
    }
    fn description(&self) -> &str {
        "store a file"
    }
    fn schema(&self) -> Value {
        json!({"type": "object", "properties": {"path": {"type": "string"}}})
    }
    fn file_access(&self, args: &Value) -> Option<(FileAccess, String)> {
        args.get("path")
            .and_then(Value::as_str)
            .map(|path| (FileAccess::Write, path.to_string()))
    }
    async fn execute(&self, _args: &Value, _ctx: &ToolContext) -> ToolOutput {
        if self.fail {
            ToolOutput::error(Fault::Tool, "disk full")
        } else {
            ToolOutput::ok("stored")
        }
    }
}

/// A tool that lists paths by a pattern.
struct ListFiles;

#[async_trait]
impl Tool for ListFiles {
    fn name(&self) -> &str {
        "list_files"
    }
    fn description(&self) -> &str {
        "list files"
    }
    fn schema(&self) -> Value {
        json!({"type": "object", "properties": {"pattern": {"type": "string"}}})
    }
    fn lists_paths(&self, args: &Value) -> Option<String> {
        args.get("pattern")
            .and_then(Value::as_str)
            .map(str::to_string)
    }
    async fn execute(&self, _args: &Value, _ctx: &ToolContext) -> ToolOutput {
        ToolOutput::ok("no files matched")
    }
}

/// A tool that touches no workspace path.
struct Ping;

#[async_trait]
impl Tool for Ping {
    fn name(&self) -> &str {
        "ping"
    }
    fn description(&self) -> &str {
        "ping"
    }
    fn schema(&self) -> Value {
        json!({"type": "object"})
    }
    async fn execute(&self, _args: &Value, _ctx: &ToolContext) -> ToolOutput {
        ToolOutput::ok("pong")
    }
}

fn header(dir: &std::path::Path, session: &str) -> SessionHeader {
    SessionHeader {
        space: None,
        run: None,
        cause: None,
        agent: None,
        session_id: session.into(),
        created_at: chrono::Utc::now(),
        cwd: dir.to_path_buf(),
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
            permission_mode: "workspace-write".into(),
            capabilities: Vec::new(),
            prompt_layers: Vec::new(),
        },
    }
}

/// Runs one turn in which the model calls `call_name` with `input`, then
/// answers, and returns the text the call's result carried.
async fn result_of_call(
    tools: Vec<Arc<dyn Tool>>,
    call_name: &str,
    input: Value,
    note: Option<vak_agent::FileNote>,
) -> String {
    let dir = tempfile::tempdir().unwrap();
    let home = dir.path().join("home");
    std::fs::create_dir_all(&home).unwrap();
    let log = SessionLog::create(
        SessionPath::new_session_file(&home, dir.path(), "file-note"),
        header(dir.path(), "file-note"),
    )
    .unwrap();
    let mut cfg = AgentConfig::new("sys");
    cfg.model = "test-model".into();
    cfg.mode = Mode::WorkspaceWrite;
    cfg.retry_base_backoff_ms = 1;
    cfg.run_retry_base_backoff_ms = 1;
    cfg.tools = tools;
    cfg.file_note = note;
    let mut script = VecDeque::from([msg(
        vec![ContentBlock::ToolUse {
            id: "c1".into(),
            name: call_name.into(),
            input,
        }],
        StopReason::ToolUse,
    )]);
    for _ in 0..4 {
        script.push_back(msg(vec![ContentBlock::text("done")], StopReason::EndTurn));
    }
    let requests = Arc::new(Mutex::new(Vec::new()));
    let mut agent = Agent::new(
        Arc::new(Scripted(
            Mutex::new(script),
            requests.clone(),
            crate::support::capacity_key(),
        )),
        log,
        cfg,
    );
    let (tx, _rx) = mpsc::channel(256);
    let outcome = agent
        .run(
            "store it",
            &Default::default(),
            CancellationToken::new(),
            tx,
        )
        .await;
    assert!(
        matches!(outcome, TurnOutcome::Completed { .. }),
        "{outcome:?}"
    );
    let requests = requests.lock().unwrap();
    requests
        .iter()
        .flat_map(|request| request.messages.iter())
        .flat_map(|message| message.content.iter())
        .find_map(|block| match block {
            ContentBlock::ToolResult {
                tool_use_id,
                content,
                ..
            } if tool_use_id == "c1" => Some(content.clone()),
            _ => None,
        })
        .expect("the call's result reaches the model")
}

fn fixed_note() -> vak_agent::FileNote {
    Arc::new(|query: vak_agent::FileQuery<'_>| {
        matches!(query, vak_agent::FileQuery::Path("porto-notes.docx"))
            .then(|| "[file] `porto-notes.docx` is not in the workspace.".to_string())
    })
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn the_note_is_attached_to_a_call_that_touches_the_file() {
    let result = result_of_call(
        vec![Arc::new(StoreFile { fail: false })],
        "store_file",
        json!({"path": "porto-notes.docx"}),
        Some(fixed_note()),
    )
    .await;
    assert_eq!(
        result,
        "stored\n[file] `porto-notes.docx` is not in the workspace."
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn the_note_is_attached_when_the_call_fails_too() {
    let result = result_of_call(
        vec![Arc::new(StoreFile { fail: true })],
        "store_file",
        json!({"path": "porto-notes.docx"}),
        Some(fixed_note()),
    )
    .await;
    // After a failure the record's fact leads, before the tool's error.
    assert!(
        result.starts_with("[file] `porto-notes.docx` is not in the workspace.\ndisk full"),
        "{result}"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_tool_that_declares_no_file_access_gets_no_note() {
    let result = result_of_call(
        vec![Arc::new(Ping)],
        "ping",
        json!({"path": "porto-notes.docx"}),
        Some(fixed_note()),
    )
    .await;
    assert_eq!(result, "pong");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_path_the_record_says_nothing_about_gets_no_note() {
    let result = result_of_call(
        vec![Arc::new(StoreFile { fail: false })],
        "store_file",
        json!({"path": "other.txt"}),
        Some(fixed_note()),
    )
    .await;
    assert_eq!(result, "stored");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn no_hook_adds_nothing() {
    let result = result_of_call(
        vec![Arc::new(StoreFile { fail: false })],
        "store_file",
        json!({"path": "porto-notes.docx"}),
        None,
    )
    .await;
    assert_eq!(result, "stored");
}

/// A call that lists paths by pattern asks the record for that pattern, by
/// what the tool declares (doc 90 §3.3).
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_listing_is_told_of_matching_files_the_folder_lacks() {
    let note: vak_agent::FileNote = Arc::new(|query: vak_agent::FileQuery<'_>| {
        matches!(query, vak_agent::FileQuery::Pattern("**/*lisbon*"))
            .then(|| "[file] Matching `**/*lisbon*` and not in the workspace yet:".to_string())
    });
    let result = result_of_call(
        vec![Arc::new(ListFiles)],
        "list_files",
        json!({"pattern": "**/*lisbon*"}),
        Some(note),
    )
    .await;
    assert_eq!(
        result,
        "no files matched\n[file] Matching `**/*lisbon*` and not in the workspace yet:"
    );
}

/// A tool that reads the workspace file it names.
struct ReadFile;

#[async_trait]
impl Tool for ReadFile {
    fn name(&self) -> &str {
        "load_notes"
    }
    fn description(&self) -> &str {
        "load notes"
    }
    fn schema(&self) -> Value {
        json!({"type": "object", "properties": {"path": {"type": "string"}}})
    }
    fn file_access(&self, args: &Value) -> Option<(FileAccess, String)> {
        args.get("path")
            .and_then(Value::as_str)
            .map(|path| (FileAccess::Read, path.to_string()))
    }
    async fn execute(&self, _args: &Value, _ctx: &ToolContext) -> ToolOutput {
        ToolOutput::ok("the sources")
    }
}

/// A tool that fetches the page it names.
struct FetchPage;

#[async_trait]
impl Tool for FetchPage {
    fn name(&self) -> &str {
        "fetch_page"
    }
    fn description(&self) -> &str {
        "fetch a page"
    }
    fn schema(&self) -> Value {
        json!({"type": "object", "properties": {"url": {"type": "string"}}})
    }
    fn fetched(&self, args: &Value) -> Option<String> {
        args.get("url").and_then(Value::as_str).map(str::to_string)
    }
    async fn execute(&self, _args: &Value, _ctx: &ToolContext) -> ToolOutput {
        ToolOutput::ok("the page")
    }
}

#[derive(Default)]
struct Claims(Mutex<Vec<vak_tools::ArtifactClaim>>);

impl vak_tools::ArtifactSink for Claims {
    fn declared(
        &self,
        claim: &vak_tools::ArtifactClaim,
        _session: Option<&str>,
        _call: &str,
        _trace: Option<&vak_session::trace::TraceKey>,
    ) {
        self.0.lock().unwrap().push(claim.clone());
    }
}

/// A version made after the turn read a file and fetched a page names both,
/// first read first (doc 90 §5.1): the loop takes them from the turn's
/// recorded reads, never from the tools' names.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_version_names_what_its_turn_read_first() {
    let dir = tempfile::tempdir().unwrap();
    let home = dir.path().join("home");
    std::fs::create_dir_all(&home).unwrap();
    let log = SessionLog::create(
        SessionPath::new_session_file(&home, dir.path(), "read-first"),
        header(dir.path(), "read-first"),
    )
    .unwrap();
    let claims = Arc::new(Claims::default());
    let mut cfg = AgentConfig::new("sys");
    cfg.model = "test-model".into();
    cfg.mode = Mode::WorkspaceWrite;
    cfg.retry_base_backoff_ms = 1;
    cfg.run_retry_base_backoff_ms = 1;
    cfg.tools = vec![
        Arc::new(ReadFile),
        Arc::new(FetchPage),
        Arc::new(StoreFile { fail: false }),
    ];
    cfg.artifacts = Some(claims.clone());
    let call = |id: &str, name: &str, input: Value| ContentBlock::ToolUse {
        id: id.into(),
        name: name.into(),
        input,
    };
    let mut script = VecDeque::from([
        msg(
            vec![
                call("r1", "load_notes", json!({"path": "sources.md"})),
                call(
                    "f1",
                    "fetch_page",
                    json!({"url": "https://example.com/tram"}),
                ),
            ],
            StopReason::ToolUse,
        ),
        msg(
            vec![call("w1", "store_file", json!({"path": "lisbon.md"}))],
            StopReason::ToolUse,
        ),
    ]);
    for _ in 0..4 {
        script.push_back(msg(vec![ContentBlock::text("done")], StopReason::EndTurn));
    }
    let mut agent = Agent::new(
        Arc::new(Scripted(
            Mutex::new(script),
            Arc::new(Mutex::new(Vec::new())),
            crate::support::capacity_key(),
        )),
        log,
        cfg,
    );
    let (tx, _rx) = mpsc::channel(256);
    let outcome = agent
        .run(
            "write it",
            &Default::default(),
            CancellationToken::new(),
            tx,
        )
        .await;

    assert!(
        matches!(outcome, TurnOutcome::Completed { .. }),
        "{outcome:?}"
    );
    let claims = claims.0.lock().unwrap();
    let written = claims
        .iter()
        .find(|claim| claim.path == "lisbon.md")
        .expect("the write is claimed");
    assert_eq!(
        written.read_first,
        vec![
            vak_tools::ReadInput::File {
                path: "sources.md".into(),
                digest: None,
            },
            vak_tools::ReadInput::Page {
                url: "https://example.com/tram".into(),
            },
        ]
    );
}
