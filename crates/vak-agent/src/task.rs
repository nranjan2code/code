//! Blocking subagent delegation. A task spawns a child agent with its own
//! JSONL session (linked via parent_session_id), a narrowed tool set that
//! excludes the task tool itself (depth-1 by construction), and returns the
//! child's final text as this tool's output.

use std::path::PathBuf;
use std::sync::Arc;

use async_trait::async_trait;
use serde_json::Value;
use vak_llm::Provider;
use vak_permission::{Mode, PermissionEngine};
use vak_session::types::{FrozenContract, SessionHeader};
use vak_session::{SessionLog, SessionPath};
use vak_tools::sandbox::Sandbox;
use vak_tools::{Tool, ToolContext, ToolOutput};

use crate::{Agent, AgentConfig, Approver};

pub struct TaskDeps {
    pub provider: Arc<dyn Provider>,
    pub system_prompt: String,
    pub model: String,
    pub tools: Vec<Arc<dyn Tool>>,
    pub max_turns: usize,
    pub permission: Option<Arc<PermissionEngine>>,
    pub mode: Mode,
    pub approver: Option<Arc<dyn Approver>>,
    pub sandbox: Option<Arc<dyn Sandbox>>,
    pub cwd: PathBuf,
    pub sessions_home: PathBuf,
    pub parent_session_id: String,
}

pub struct TaskTool {
    deps: Arc<TaskDeps>,
}

impl TaskTool {
    pub fn new(deps: TaskDeps) -> Self {
        TaskTool {
            deps: Arc::new(deps),
        }
    }
}

#[async_trait]
impl Tool for TaskTool {
    fn name(&self) -> &str {
        "task"
    }

    fn description(&self) -> &str {
        "Delegate a self-contained subtask to a subagent with its own context window and transcript. Use for focused research or exploration whose details you do not need in your own context. The subagent cannot spawn further subagents."
    }

    fn schema(&self) -> Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "prompt": {"type": "string", "description": "Complete, self-contained instructions for the subagent"},
                "label": {"type": "string", "description": "Short label shown in the UI"}
            },
            "required": ["prompt"]
        })
    }

    async fn execute(&self, args: &Value, ctx: &ToolContext) -> ToolOutput {
        let Some(prompt) = args.get("prompt").and_then(|p| p.as_str()) else {
            return ToolOutput::error("missing required parameter: prompt");
        };

        let session_id = format!(
            "child-{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_nanos()
        );
        let path =
            SessionPath::new_session_file(&self.deps.sessions_home, &self.deps.cwd, &session_id);
        let header = SessionHeader {
            session_id: session_id.clone(),
            created_at: chrono::Utc::now(),
            cwd: self.deps.cwd.clone(),
            parent_session_id: Some(self.deps.parent_session_id.clone()),
            contract: FrozenContract {
                app_version: env!("CARGO_PKG_VERSION").into(),
                provider: self.deps.provider.name().into(),
                model: self.deps.model.clone(),
                system_prompt: self.deps.system_prompt.clone(),
                tools: self
                    .deps
                    .tools
                    .iter()
                    .map(|t| t.name().to_string())
                    .collect(),
                permission_mode: match self.deps.mode {
                    Mode::ReadOnly => "read-only",
                    Mode::WorkspaceWrite => "workspace-write",
                    Mode::FullAccess => "full-access",
                }
                .into(),
                skills: Vec::new(),
            },
        };
        let log = match SessionLog::create(path, header) {
            Ok(l) => l,
            Err(e) => return ToolOutput::error(format!("cannot create child session: {e}")),
        };

        let mut cfg = AgentConfig::new(self.deps.system_prompt.clone());
        cfg.model = self.deps.model.clone();
        cfg.tools = self.deps.tools.clone();
        cfg.max_turns = self.deps.max_turns;
        cfg.parallel_tools = true;
        cfg.permission = self.deps.permission.clone();
        cfg.mode = self.deps.mode;
        cfg.approver = self.deps.approver.clone();
        cfg.sandbox = self.deps.sandbox.clone();

        let mut agent = Agent::new(self.deps.provider.clone(), log, cfg);
        let steering = Default::default();
        let cancel = ctx.cancel.child_token();
        let (ev_tx, mut ev_rx) = tokio::sync::mpsc::channel::<crate::AgentEvent>(256);
        let pump = tokio::spawn(async move { while ev_rx.recv().await.is_some() {} });
        let outcome = agent.run(prompt, &steering, cancel, ev_tx).await;
        let _ = pump.await;

        match outcome {
            crate::TurnOutcome::Completed { response } => {
                let text = response.text_content();
                if text.is_empty() {
                    ToolOutput::ok(format!("subagent '{session_id}' completed without output"))
                } else {
                    ToolOutput::ok(text)
                }
            }
            crate::TurnOutcome::Aborted { partial } => {
                let text = partial.map(|p| p.text_content()).unwrap_or_default();
                ToolOutput::error(format!(
                    "subagent '{session_id}' was cancelled. Partial output:\n{text}"
                ))
            }
            crate::TurnOutcome::Failed { error } => {
                ToolOutput::error(format!("subagent '{session_id}' failed: {error}"))
            }
            crate::TurnOutcome::MaxTurnsReached => ToolOutput::error(format!(
                "subagent '{session_id}' hit its turn limit before finishing"
            )),
        }
    }
}
