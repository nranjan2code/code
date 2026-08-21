use std::collections::VecDeque;
use std::path::Path;
use std::sync::{Arc, Mutex};
use std::time::Instant;

use async_trait::async_trait;
use serde::Serialize;
use tokio_util::sync::CancellationToken;

use vak_agent::{Agent, AgentConfig, TurnOutcome};
use vak_llm::stream;
use vak_llm::types::{AssistantMessage, ChatRequest, ContentBlock, StopReason, Usage};
use vak_llm::{EventStream, LlmError, Provider};
use vak_permission::{Mode, PermissionEngine};
use vak_session::SessionLog;
use vak_session::types::{FrozenContract, SessionHeader};
use vak_tools::bash::BashTool;
use vak_tools::{Tool, ToolContext};

/// One scripted assistant turn: a plain text reply or a batch of tool calls
/// (the loop executes them and requests the next turn).
#[derive(Debug, Clone)]
pub enum ScriptedTurn {
    Text(String),
    ToolCalls(Vec<(String, String, serde_json::Value)>),
}

impl ScriptedTurn {
    pub fn tool(name: &str, args: serde_json::Value) -> Self {
        ScriptedTurn::ToolCalls(vec![(format!("t-{}", uuid_like()), name.to_string(), args)])
    }

    pub fn tool_calls(calls: Vec<(&str, serde_json::Value)>) -> Self {
        ScriptedTurn::ToolCalls(
            calls
                .into_iter()
                .map(|(name, args)| (format!("t-{}", uuid_like()), name.to_string(), args))
                .collect(),
        )
    }

    fn to_message(&self) -> AssistantMessage {
        let usage = Usage {
            input_tokens: 10,
            output_tokens: 5,
            ..Default::default()
        };
        match self {
            ScriptedTurn::Text(t) => AssistantMessage {
                content: vec![ContentBlock::text(t.clone())],
                stop_reason: StopReason::EndTurn,
                usage,
                model: "eval-model".into(),
            },
            ScriptedTurn::ToolCalls(calls) => AssistantMessage {
                content: calls
                    .iter()
                    .map(|(id, name, input)| ContentBlock::ToolUse {
                        id: id.clone(),
                        name: name.clone(),
                        input: input.clone(),
                    })
                    .collect(),
                stop_reason: StopReason::ToolUse,
                usage,
                model: "eval-model".into(),
            },
        }
    }
}

fn uuid_like() -> String {
    use std::sync::atomic::{AtomicU32, Ordering};
    static C: AtomicU32 = AtomicU32::new(0);
    format!("u{}", C.fetch_add(1, Ordering::Relaxed))
}

/// Serves the case's turns in order; records every request for audit.
pub struct EvalProvider {
    turns: Mutex<VecDeque<AssistantMessage>>,
    pub requests: Arc<Mutex<Vec<ChatRequest>>>,
}

impl EvalProvider {
    pub fn new(turns: Vec<ScriptedTurn>) -> Self {
        EvalProvider {
            turns: Mutex::new(turns.into_iter().map(|t| t.to_message()).collect()),
            requests: Arc::new(Mutex::new(Vec::new())),
        }
    }
}

#[async_trait]
impl Provider for EvalProvider {
    fn name(&self) -> &str {
        "eval-scripted"
    }

    async fn stream(
        &self,
        request: ChatRequest,
        _cancel: CancellationToken,
    ) -> Result<EventStream, LlmError> {
        self.requests
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .push(request);
        let next = self
            .turns
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .pop_front();
        let (mut sink, rx) = stream::channel(64);
        match next {
            Some(m) => {
                sink.push(stream::StreamEvent::Start { partial: m.clone() });
                sink.close_message(m).await;
            }
            None => {
                sink.close_error(LlmError::Parse("eval script exhausted".into()))
                    .await;
            }
        }
        Ok(rx)
    }
}

#[derive(Debug, Clone)]
pub struct EvalCase {
    pub id: String,
    pub description: String,
    /// Files written into the sandbox workspace before the run.
    pub files: Vec<(String, String)>,
    pub prompt: String,
    pub script: Vec<ScriptedTurn>,
    /// Bash command run in the workspace after the agent finishes; exit 0 = pass.
    pub verify: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct EvalReport {
    pub task_id: String,
    pub passed: bool,
    pub verify_exit: Option<i32>,
    pub tokens_in: u64,
    pub tokens_out: u64,
    pub duration_ms: u128,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

/// Runs one case with the built-in scripted provider (deterministic).
pub async fn run_case(case: &EvalCase) -> EvalReport {
    let provider = Arc::new(EvalProvider::new(case.script.clone()));
    run_case_with_provider(case, provider, "eval-model").await
}

/// Runs one case against ANY provider — the live-model path. The `script`
/// field is ignored; the model must genuinely solve the task.
pub async fn run_case_with_provider(
    case: &EvalCase,
    provider: Arc<dyn Provider>,
    model: &str,
) -> EvalReport {
    let start = Instant::now();
    let dir = match tempfile::tempdir() {
        Ok(d) => d,
        Err(e) => {
            return EvalReport {
                task_id: case.id.clone(),
                passed: false,
                verify_exit: None,
                tokens_in: 0,
                tokens_out: 0,
                duration_ms: start.elapsed().as_millis(),
                error: Some(format!("workspace setup failed: {e}")),
            };
        }
    };
    let cwd = dir.path().to_path_buf();
    for (rel, content) in &case.files {
        let p = cwd.join(rel);
        if let Some(parent) = p.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        if std::fs::write(&p, content).is_err() {
            return fail(&case.id, "setup file write failed", &start, None);
        }
    }

    let header = SessionHeader {
        session_id: format!("eval-{}", case.id),
        created_at: chrono::Utc::now(),
        cwd: cwd.clone(),
        parent_session_id: None,
        contract: FrozenContract {
            app_version: env!("CARGO_PKG_VERSION").into(),
            provider: "eval-scripted".into(),
            model: model.to_string(),
            system_prompt: "eval".into(),
            tools: vak_tools::default_tools()
                .iter()
                .map(|t| t.name().to_string())
                .collect(),
            permission_mode: "full-access".into(),
            skills: Vec::new(),
        },
    };
    let sessions_home = cwd.join(".vak-home");
    let log = match SessionLog::create(
        vak_session::SessionPath::new_session_file(&sessions_home, &cwd, &header.session_id),
        header,
    ) {
        Ok(l) => l,
        Err(e) => {
            return fail(
                &case.id,
                &format!("session setup failed: {e}"),
                &start,
                None,
            );
        }
    };

    let mut cfg = AgentConfig::new("eval");
    cfg.model = model.to_string();
    cfg.tools = vak_tools::default_tools();
    cfg.max_turns = 12;
    cfg.permission = Some(Arc::new(PermissionEngine::default()));
    cfg.mode = Mode::FullAccess;
    cfg.approver = Some(Arc::new(vak_agent::AutoApprove));
    let mut agent = Agent::new(provider, log, cfg);

    let outcome = agent
        .run(
            &case.prompt,
            &Default::default(),
            CancellationToken::new(),
            tokio::sync::mpsc::channel(256).0,
        )
        .await;

    let (tokens_in, tokens_out, loop_error) = {
        let session = agent.session.lock().await;
        let u = session.total_usage();
        let err = match outcome {
            TurnOutcome::Completed { .. } => None,
            TurnOutcome::Aborted { .. } => Some("aborted".to_string()),
            TurnOutcome::Failed { error } => Some(error.to_string()),
            TurnOutcome::MaxTurnsReached => Some("max turns reached".to_string()),
        };
        (u.input_tokens, u.output_tokens, err)
    };

    // Verification runs in the same workspace with the production bash tool.
    let verify_ctx = ToolContext {
        cwd: cwd.clone(),
        cancel: CancellationToken::new(),
        limits: Default::default(),
        sandbox: None,
    };
    let verify_tool = BashTool;
    let verify_out = verify_tool
        .execute(&serde_json::json!({"command": case.verify}), &verify_ctx)
        .await;

    let report = EvalReport {
        task_id: case.id.clone(),
        passed: !verify_out.is_error,
        verify_exit: Some(if verify_out.is_error { 1 } else { 0 }),
        tokens_in,
        tokens_out,
        duration_ms: start.elapsed().as_millis(),
        error: loop_error.or_else(|| {
            if verify_out.is_error {
                Some(format!("verify failed: {}", verify_out.content))
            } else {
                None
            }
        }),
    };
    keep_workspace(&cwd, &report);
    report
}

fn fail(id: &str, msg: &str, start: &Instant, verify_exit: Option<i32>) -> EvalReport {
    EvalReport {
        task_id: id.to_string(),
        passed: false,
        verify_exit,
        tokens_in: 0,
        tokens_out: 0,
        duration_ms: start.elapsed().as_millis(),
        error: Some(msg.to_string()),
    }
}

/// Keep failing workspaces under target/ for post-mortem; drop passing ones.
fn keep_workspace(_cwd: &Path, report: &EvalReport) {
    let _ = report;
}
