//! Blocking worker delegation. A task spawns a child agent with its own
//! JSONL session (linked via parent_session_id), a narrowed tool set that
//! excludes the task tool itself (depth-1 by construction), and returns the
//! child's final text as this tool's output. Live children register in a
//! shared [`WorkerRegistry`] so UIs can list, steer, and stop them.

use std::collections::BTreeMap;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use serde_json::Value;
use sha2::{Digest, Sha256};
use tokio_util::sync::CancellationToken;
use vak_llm::Provider;
use vak_permission::{Mode, PermissionEngine};
use vak_session::types::{CapabilityDescriptor, CapabilityKind, FrozenContract, SessionHeader};
use vak_session::{SessionLog, SessionPath};
use vak_tools::sandbox::Sandbox;
use vak_tools::{Tool, ToolContext, ToolOutput};

use crate::{
    Agent, AgentConfig, ApprovalMode, Approver, InputNormalizer, McpToolIndex, SteeringQueues,
};

fn vak_core_identity() -> vak_session::types::AgentIdentity {
    vak_session::types::AgentIdentity {
        id: "vak".into(),
        revision: 1,
        name: "Vakyartha".into(),
        character: "vak".into(),
        personality: String::new(),
        animation: "subtle".into(),
        voice: "default".into(),
        behaviour: String::new(),
        responsibilities: String::new(),
        instructions: String::new(),
    }
}

pub struct TaskDeps {
    /// The tenant object store child ledgers keep their large payloads in.
    pub objects: Arc<dyn vak_session::objects::Objects>,
    /// Resolved parent Agent identity; inherited by default-delegated children.
    pub parent_agent_identity: Option<vak_session::types::AgentIdentity>,
    /// Parent outcome context carried into the child for alignment only.
    pub outcome_objective: Option<String>,
    /// The parent's admitted outcome, narrowed for this child at dispatch.
    pub outcome: Option<vak_intent::OutcomeSpec>,
    /// Prompts for named roles, admitted up front by the host exactly like
    /// capabilities are. A child can only ever run under a role that was
    /// resolvable when the parent session was admitted, so an unknown or
    /// injected role name cannot conjure new instructions mid-run.
    pub role_prompts: std::collections::BTreeMap<String, String>,
    pub provider: Arc<dyn Provider>,
    pub system_prompt: String,
    /// Composes a child's system prompt through the host's one prompt
    /// resolver, for the child's own role, Agent and final capability set.
    /// `None` (deterministic fixtures) runs the child under `system_prompt`
    /// or the named entry in `role_prompts` unchanged.
    pub child_prompt: Option<ChildPrompt>,
    /// Whether this workspace's project config is trusted: a project's saved
    /// Agents are selectable only then, as for a top-level Agent.
    pub trust_project: bool,
    /// The parent turn's tail (clock instant + epistemic stance,
    /// docs/design/68-context-engine.md §6/§10). Children get the same
    /// turn context block as the parent rather than an empty one, since a
    /// worker dispatched mid-turn is still answering as of that turn's
    /// instant and stance.
    pub tail: crate::TailInput,
    pub model: String,
    pub tools: Vec<Arc<dyn Tool>>,
    /// Parent's progressive tool surface, already split into loaded and
    /// deferred definitions for this turn.
    pub tool_definitions: Vec<vak_llm::ToolDefinition>,
    pub capabilities: Vec<CapabilityDescriptor>,
    pub hooks: Option<Arc<Vec<vak_hooks::HookDef>>>,
    pub revocation_check: Option<crate::RevocationCheck>,
    /// Records a worker's cards in its own ledger as they validate, so they
    /// can be handed to the delegating conversation when the worker ends.
    pub presentation_rebuild: Option<crate::PresentationRebuild>,
    pub mcp_tool_index: Option<McpToolIndex>,
    pub input_normalizer: Option<InputNormalizer>,
    /// Read-only subset (read/glob/grep) used when a task declares
    /// `readonly: true`; children get these plus ReadOnly permission mode.
    pub read_only_tools: Vec<Arc<dyn Tool>>,
    pub max_turns: usize,
    /// The parent's measured context budget, inherited by every worker.
    pub capacity: Option<vak_context::capacity::CapacityProfile>,
    pub capacity_key: Option<vak_context::capacity::ProfileKey>,
    pub max_output: u64,
    pub declared_window: u64,
    pub ladder: Vec<(Arc<dyn Provider>, String)>,
    pub ladder_provider_names: Vec<String>,
    pub provider_name: Option<String>,
    pub max_retries: u32,
    pub retry_base_backoff_ms: u64,
    pub request_timeout: Option<std::time::Duration>,
    pub circuit_breaker: Option<Arc<crate::CircuitBreaker>>,
    pub run_retry_attempts: u32,
    pub run_retry_base_backoff_ms: u64,
    pub dispatch_ceiling: u32,
    pub spend_gate: Option<Arc<dyn crate::SpendGate>>,
    pub permission: Option<Arc<PermissionEngine>>,
    pub mode: Mode,
    pub approval_mode: ApprovalMode,
    pub approver: Option<Arc<dyn Approver>>,
    pub sandbox: Option<Arc<dyn Sandbox>>,
    pub cwd: PathBuf,
    pub sessions_home: PathBuf,
    pub parent_session_id: String,
    pub contract_id: Option<String>,
    pub work_item_id: Option<String>,
    pub work_item_ids: Vec<String>,
    /// Parent-loop event channel so worker lifecycles surface in the UI.
    pub events: Option<tokio::sync::mpsc::Sender<crate::AgentEvent>>,
    /// Shared registry of live children. None disables attach/steer (the
    /// child still runs normally).
    pub registry: Option<Arc<WorkerRegistry>>,
}

pub struct TaskTool {
    deps: Arc<TaskDeps>,
}

/// `(role, agent, capabilities) -> system prompt` for a child run.
pub type ChildPrompt = Arc<
    dyn Fn(
            Option<&str>,
            Option<&vak_session::types::AgentIdentity>,
            &[CapabilityDescriptor],
        ) -> String
        + Send
        + Sync,
>;

#[derive(serde::Deserialize)]
struct AgentDefinition {
    id: String,
    #[serde(default = "default_agent_revision")]
    revision: u64,
    #[serde(default = "default_agent_lifecycle")]
    lifecycle: String,
    name: String,
    character: String,
    personality: String,
    behaviour: String,
    #[serde(default)]
    responsibilities: String,
    #[serde(default)]
    instructions: String,
}

fn default_agent_revision() -> u64 {
    1
}

fn default_agent_lifecycle() -> String {
    "active".into()
}

fn load_agent(
    cwd: &std::path::Path,
    trust_project: bool,
    requested: &str,
) -> Result<Option<AgentDefinition>, String> {
    // Delegated Agents resolve the same effective Shared → trusted project
    // layers as a top-level Agent. Previously this delegated helper read only the
    // project file, so a user-level Agent worked from the sidebar and
    // scheduler but was invisible to `task(agent=...)`.
    let shared = vak_config::paths::default_workspace();
    let mut profiles = read_agents(&shared)?;
    if cwd != shared && trust_project {
        for profile in read_agents(cwd)? {
            profiles.retain(|candidate| candidate.id != profile.id);
            profiles.push(profile);
        }
    }
    if let Some(profile) = profiles.iter().find(|profile| profile.id == requested) {
        if profile.lifecycle != "active" {
            return Err(format!(
                "Agent '{}' is {} and cannot be selected for delegated work",
                profile.id, profile.lifecycle
            ));
        }
        return Ok(Some(AgentDefinition {
            id: profile.id.clone(),
            revision: profile.revision,
            lifecycle: profile.lifecycle.clone(),
            name: profile.name.clone(),
            character: profile.character.clone(),
            personality: profile.personality.clone(),
            behaviour: profile.behaviour.clone(),
            responsibilities: profile.responsibilities.clone(),
            instructions: profile.instructions.clone(),
        }));
    }
    let matches = profiles
        .into_iter()
        .filter(|profile| profile.name.eq_ignore_ascii_case(requested))
        .collect::<Vec<_>>();
    match matches.len() {
        0 => Ok(None),
        1 => {
            let agent = matches.into_iter().next();
            if let Some(agent) = &agent
                && agent.lifecycle != "active"
            {
                return Err(format!(
                    "Agent '{}' is {} and cannot be selected for delegated work",
                    agent.id, agent.lifecycle
                ));
            }
            Ok(agent)
        }
        _ => Err(format!(
            "Agent name '{}' is ambiguous; choose an Agent by its exact name or id",
            requested
        )),
    }
}

fn read_agents(cwd: &std::path::Path) -> Result<Vec<AgentDefinition>, String> {
    let path = vak_config::scope::WorkspaceScope::new(cwd).agents_file();
    let Ok(raw) = std::fs::read_to_string(path) else {
        return Ok(Vec::new());
    };
    serde_json::from_str::<Vec<AgentDefinition>>(&raw)
        .map_err(|_| "saved Agent definitions are invalid".to_string())
}

struct RegistryGuard {
    registry: Arc<WorkerRegistry>,
    id: String,
}

impl Drop for RegistryGuard {
    fn drop(&mut self) {
        self.registry.unregister(&self.id);
    }
}

/// A live child agent: its steering queues and cancellation token, so an
/// attached UI can push steering or stop it while the parent's task tool
/// call is still blocking.
#[derive(Debug)]
pub struct WorkerHandle {
    pub label: String,
    pub agent_id: Option<String>,
    pub agent_revision: Option<u64>,
    pub started_at: std::time::Instant,
    pub steering: Arc<SteeringQueues>,
    pub cancel: CancellationToken,
    /// Session that spawned this child; routes registry lookups to the
    /// owning surface's endpoint scope.
    pub parent_session_id: String,
    /// Started with `task { background: true }`: the parent did not wait.
    pub background: bool,
    /// The paths a background writer may change, held until it ends. Empty
    /// for a worker that holds no lease.
    pub write_scopes: Vec<String>,
    pub progress: Arc<Mutex<WorkerProgress>>,
}

/// What a live worker has done so far, for `workers status`
/// (docs/design/84-worker-questions-and-control.md §5.3). Updated by the
/// pump that already drains the child's event stream.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct WorkerProgress {
    pub steps: u32,
    /// The last few tool names, oldest first.
    pub last_tools: std::collections::VecDeque<String>,
    pub input_tokens: u64,
    pub output_tokens: u64,
}

/// The most background workers one parent may have running at once.
pub const MAX_WORKERS: usize = 8;

const PROGRESS_TOOLS_KEPT: usize = 5;

impl WorkerProgress {
    fn tool_finished(&mut self, name: &str) {
        self.last_tools.push_back(name.to_string());
        while self.last_tools.len() > PROGRESS_TOOLS_KEPT {
            self.last_tools.pop_front();
        }
    }

    fn step_finished(&mut self, input_tokens: u64, output_tokens: u64) {
        self.steps += 1;
        self.input_tokens += input_tokens;
        self.output_tokens += output_tokens;
    }
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct ActiveWorker {
    pub id: String,
    pub label: String,
    pub agent_id: Option<String>,
    pub agent_revision: Option<u64>,
    pub elapsed_secs: u64,
    pub parent_session_id: String,
    pub background: bool,
    pub steps: u32,
    pub last_tools: Vec<String>,
    pub input_tokens: u64,
    pub output_tokens: u64,
}

impl ActiveWorker {
    fn of(id: &str, handle: &WorkerHandle) -> Self {
        let progress = handle
            .progress
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .clone();
        ActiveWorker {
            id: id.to_string(),
            label: handle.label.clone(),
            agent_id: handle.agent_id.clone(),
            agent_revision: handle.agent_revision,
            elapsed_secs: handle.started_at.elapsed().as_secs(),
            parent_session_id: handle.parent_session_id.clone(),
            background: handle.background,
            steps: progress.steps,
            last_tools: progress.last_tools.into_iter().collect(),
            input_tokens: progress.input_tokens,
            output_tokens: progress.output_tokens,
        }
    }
}

/// How a background worker ended, kept for its parent turn to read.
#[derive(Debug, Clone, PartialEq)]
pub struct FinishedWorker {
    pub id: String,
    pub label: String,
    pub parent_session_id: String,
    pub is_error: bool,
    pub text: String,
    pub elapsed_secs: u64,
}

/// Registry of currently-running workers, keyed by unique child session
/// id. Interior-mutable: the UI holds a shared reference across runs.
#[derive(Debug, Default)]
pub struct WorkerRegistry {
    inner: Mutex<BTreeMap<String, WorkerHandle>>,
    /// Background workers that have ended, until their parent turn does.
    finished: Mutex<BTreeMap<String, FinishedWorker>>,
    changed: tokio::sync::Notify,
    questions: crate::questions::QuestionBoard,
}

impl WorkerRegistry {
    pub fn new() -> Self {
        Self::default()
    }

    /// Questions the workers of this registry are waiting on.
    pub fn questions(&self) -> &crate::questions::QuestionBoard {
        &self.questions
    }

    #[cfg(test)]
    pub(crate) fn register(&self, id: String, handle: WorkerHandle) {
        if let Ok(mut map) = self.inner.lock() {
            map.insert(id, handle);
        }
    }

    fn try_register(&self, id: String, handle: WorkerHandle) -> Result<(), String> {
        let mut map = self
            .inner
            .lock()
            .map_err(|_| "worker registry is unavailable".to_string())?;
        if map
            .values()
            .filter(|worker| worker.parent_session_id == handle.parent_session_id)
            .count()
            >= MAX_WORKERS
        {
            return Err(format!(
                "{MAX_WORKERS} workers are already running; wait for one with the workers tool first"
            ));
        }
        map.insert(id, handle);
        Ok(())
    }

    fn unregister(&self, id: &str) {
        if let Ok(mut map) = self.inner.lock() {
            map.remove(id);
        }
        self.changed.notify_waiters();
    }

    pub fn active(&self) -> Vec<ActiveWorker> {
        let Ok(map) = self.inner.lock() else {
            return Vec::new();
        };
        map.iter().map(|(id, h)| ActiveWorker::of(id, h)).collect()
    }

    /// Live children spawned by `parent`, oldest first.
    pub fn active_for(&self, parent: &str) -> Vec<ActiveWorker> {
        let Ok(map) = self.inner.lock() else {
            return Vec::new();
        };
        map.iter()
            .filter(|(_, h)| h.parent_session_id == parent)
            .map(|(id, h)| ActiveWorker::of(id, h))
            .collect()
    }

    /// One live child of `parent`; `None` for an id that is not live or
    /// belongs to another session, which are indistinguishable by design.
    pub fn live_child(&self, parent: &str, id: &str) -> Option<ActiveWorker> {
        let map = self
            .inner
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        map.get(id)
            .filter(|h| h.parent_session_id == parent)
            .map(|h| ActiveWorker::of(id, h))
    }

    /// Live background children of `parent`.
    pub fn background_live(&self, parent: &str) -> usize {
        self.inner.lock().map_or(0, |map| {
            map.values()
                .filter(|h| h.parent_session_id == parent && h.background)
                .count()
        })
    }

    /// The live background writer of `parent` whose lease `claims` would
    /// touch, if any. A lease lasts as long as its worker.
    pub fn lease_conflict(
        &self,
        parent: &str,
        claims: &vak_tools::ResourceClaims,
    ) -> Option<String> {
        let map = self
            .inner
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        map.iter()
            .filter(|(_, h)| h.parent_session_id == parent && !h.write_scopes.is_empty())
            .find(|(_, h)| {
                let lease = crate::write_lease::lease_claims(&h.write_scopes);
                lease.conflicts(claims) || claims.conflicts(&lease)
            })
            .map(|(id, _)| id.clone())
    }

    /// Record how a background worker ended and wake anyone waiting.
    fn finish(&self, worker: FinishedWorker) {
        if let Ok(mut map) = self.finished.lock() {
            map.insert(worker.id.clone(), worker);
        }
        self.changed.notify_waiters();
    }

    /// A finished background worker of `parent`.
    pub fn finished_child(&self, parent: &str, id: &str) -> Option<FinishedWorker> {
        let map = self.finished.lock().ok()?;
        map.get(id)
            .filter(|w| w.parent_session_id == parent)
            .cloned()
    }

    /// Finished background workers of `parent`, in the order they were given.
    pub fn finished_for(&self, parent: &str) -> Vec<FinishedWorker> {
        let Ok(map) = self.finished.lock() else {
            return Vec::new();
        };
        let mut out: Vec<FinishedWorker> = map
            .values()
            .filter(|w| w.parent_session_id == parent)
            .cloned()
            .collect();
        out.sort_by(|a, b| a.id.cmp(&b.id));
        out
    }

    /// Forget a parent's finished workers once its turn has ended.
    pub fn forget_finished_for(&self, parent: &str) {
        if let Ok(mut map) = self.finished.lock() {
            map.retain(|_, w| w.parent_session_id != parent);
        }
    }

    /// Resolves when any worker finishes or leaves, for `workers wait`.
    pub async fn changed(&self) {
        self.changed.notified().await;
    }

    /// Cancel every live child of `parent`; returns how many were live.
    pub fn stop_all_for(&self, parent: &str) -> usize {
        let Ok(map) = self.inner.lock() else {
            return 0;
        };
        let mut stopped = 0;
        for handle in map.values().filter(|h| h.parent_session_id == parent) {
            handle.cancel.cancel();
            stopped += 1;
        }
        stopped
    }

    /// Owning session of a live child, for endpoint-scope checks.
    pub fn parent_of(&self, id: &str) -> Option<String> {
        let map = self.inner.lock().ok()?;
        map.get(id).map(|h| h.parent_session_id.clone())
    }

    /// Queues steering text for the child. Returns false when no such
    /// child is live.
    pub fn steer(&self, id: &str, text: &str) -> bool {
        let Ok(map) = self.inner.lock() else {
            return false;
        };
        match map.get(id) {
            Some(h) => {
                h.steering.push_steering(text.to_string());
                true
            }
            None => false,
        }
    }

    /// Queues a follow-up turn for the child (runs after its natural stop).
    pub fn queue_follow_up(&self, id: &str, text: &str) -> bool {
        let Ok(map) = self.inner.lock() else {
            return false;
        };
        match map.get(id) {
            Some(h) => {
                h.steering.push_follow_up(text.to_string());
                true
            }
            None => false,
        }
    }

    /// Cancels the child. Returns false when no such child is live.
    pub fn stop(&self, id: &str) -> bool {
        let Ok(map) = self.inner.lock() else {
            return false;
        };
        match map.get(id) {
            Some(h) => {
                h.cancel.cancel();
                true
            }
            None => false,
        }
    }
}

impl TaskTool {
    /// Declared as a constant so capability declarations can read it
    /// without constructing the tool's dependencies.
    pub const SERVES: &'static [&'static str] = &["orchestration"];

    /// Constant so the prompt's tool catalogue can list the tool before its
    /// turn-bound dependencies exist.
    pub const DESCRIPTION: &'static str = "Delegate a self-contained subtask to a worker with its own context window and transcript. Use for focused research or exploration whose details you do not need in your own context. Optionally select a saved Agent so its identity and working style are applied; this never changes permissions. The worker cannot spawn further workers.";

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

    fn serves(&self) -> &'static [&'static str] {
        Self::SERVES
    }

    fn description(&self) -> &str {
        Self::DESCRIPTION
    }

    fn schema(&self) -> Value {
        // The admitted roles go in the schema as an `enum` rather than in
        // prose: it is the only form the provider will actually constrain
        // the model against, and an unadmitted name is refused at call time
        // anyway.
        let roles: Vec<&str> = self.deps.role_prompts.keys().map(String::as_str).collect();
        let mut role_property = serde_json::json!({
            "type": "string",
            "description": "Named role whose instructions this child runs under. Omit to use the default worker instructions."
        });
        if !roles.is_empty() {
            role_property["enum"] = serde_json::json!(roles);
        }
        serde_json::json!({
            "type": "object",
            "properties": {
                "prompt": {"type": "string", "description": "Complete, self-contained instructions for the worker"},
                "role": role_property,
                "agent": {"type": "string", "description": "Optional saved Agent name or id. Applies its identity and working style without changing permissions."},
                "label": {"type": "string", "description": "Short label shown in the UI"},
                "readonly": {"type": "boolean", "description": "If true, the worker gets only read/glob/grep and may run concurrently with other tasks", "default": false},
                "background": {"type": "boolean", "description": "Return at once with the worker's id and keep it running while you continue; use the workers tool to check, message, wait for or stop it. It is cancelled if you finish your turn without waiting for it. A background worker that is not readonly must name paths: it may change only those, and you may not touch them until it ends.", "default": false},
                "paths": {"type": "array", "items": {"type": "string"}, "description": "Path scopes (globs) this task will write to; tasks with disjoint scopes run in parallel, overlapping scopes are serialized"},
                "contract_id": {"type": "string", "description": "Managed contract this child is executing"},
                "work_item_id": {"type": "string", "description": "Managed work item assigned to this child"}
            },
            "required": ["prompt"]
        })
    }

    fn claims(&self, args: &Value) -> vak_tools::ResourceClaims {
        let readonly = args
            .get("readonly")
            .and_then(|r| r.as_bool())
            .unwrap_or(false);
        // A background task only starts the worker, and it must be read-only.
        if readonly {
            return vak_tools::ResourceClaims {
                exclusive: false,
                read_only: true,
                paths: Vec::new(),
            };
        }
        let paths: Vec<String> = args
            .get("paths")
            .and_then(|p| p.as_array())
            .map(|a| {
                a.iter()
                    .filter_map(|v| v.as_str().map(String::from))
                    .collect()
            })
            .unwrap_or_default();
        vak_tools::ResourceClaims {
            exclusive: paths.is_empty(),
            read_only: false,
            paths,
        }
    }

    async fn execute(&self, args: &Value, ctx: &ToolContext) -> ToolOutput {
        self.execute_inner(args, ctx).await
    }
}

impl TaskTool {
    async fn execute_inner(&self, args: &Value, ctx: &ToolContext) -> ToolOutput {
        let Some(prompt) = args.get("prompt").and_then(|p| p.as_str()) else {
            return ToolOutput::error("missing required parameter: prompt");
        };
        let requested_contract = args.get("contract_id").and_then(|value| value.as_str());
        let requested_item = args.get("work_item_id").and_then(|value| value.as_str());
        if requested_contract.is_some() != requested_item.is_some() {
            return ToolOutput::error("contract_id and work_item_id must be supplied together");
        }
        if let Some(contract_id) = requested_contract
            && self
                .deps
                .contract_id
                .as_deref()
                .is_some_and(|known| known != contract_id)
        {
            return ToolOutput::error(
                "child contract does not match the parent's managed contract",
            );
        }
        if let Some(item_id) = requested_item
            && !self.deps.work_item_ids.is_empty()
            && !self.deps.work_item_ids.iter().any(|known| known == item_id)
        {
            return ToolOutput::error(
                "child work item does not exist in the parent's managed contract",
            );
        }
        let session_id = next_child_session_id();
        let readonly = args
            .get("readonly")
            .and_then(|r| r.as_bool())
            .unwrap_or(false);
        let mut write_scopes: Vec<String> = Vec::new();
        if args
            .get("background")
            .and_then(|b| b.as_bool())
            .unwrap_or(false)
        {
            let Some(registry) = self.deps.registry.as_ref() else {
                return ToolOutput::error("background workers are not available here");
            };
            // A background writer keeps a lease on the paths it may change
            // for as long as it runs (docs/design/84 §5.1).
            if !readonly {
                let named: Vec<String> = args
                    .get("paths")
                    .and_then(|p| p.as_array())
                    .map(|a| {
                        a.iter()
                            .filter_map(|v| v.as_str().map(String::from))
                            .collect()
                    })
                    .unwrap_or_default();
                write_scopes = match crate::write_lease::normalize_scopes(&named) {
                    Ok(scopes) => scopes,
                    Err(reason) => return ToolOutput::error(reason),
                };
                if let Some(holder) = registry.lease_conflict(
                    &self.deps.parent_session_id,
                    &crate::write_lease::lease_claims(&write_scopes),
                ) {
                    return ToolOutput::error(format!(
                        "those paths are already leased to background worker '{holder}'; wait for it or stop it first"
                    ));
                }
            }
        }
        let child_outcome = child_outcome(prompt, readonly, self.deps.outcome.as_ref());
        let child_tools: Vec<Arc<dyn Tool>> = if !write_scopes.is_empty() {
            // File tools fenced to the lease; nothing that cannot be fenced.
            let mut tools = self.deps.read_only_tools.clone();
            for name in crate::write_lease::WRITER_TOOLS {
                if let Some(tool) = self.deps.tools.iter().find(|tool| tool.name() == *name) {
                    tools.push(Arc::new(crate::write_lease::ScopedWriteTool::new(
                        tool.clone(),
                        write_scopes.clone(),
                    )));
                }
            }
            tools
        } else if readonly {
            self.deps.read_only_tools.clone()
        } else {
            self.deps.tools.clone()
        }
        .into_iter()
        .filter(|tool| tool.name() != "flow")
        .collect();
        let child_mode = if readonly {
            Mode::ReadOnly
        } else {
            self.deps.mode
        };
        // An unknown role is refused rather than quietly ignored: a child
        // that silently ran under the default prompt when a role was asked
        // for would be the hardest kind of misconfiguration to notice.
        let mut child_system_prompt = match args.get("role").and_then(|r| r.as_str()) {
            Some(role) if !role.trim().is_empty() => {
                match self.deps.role_prompts.get(role.trim()) {
                    Some(prompt) => prompt.clone(),
                    None => {
                        let known = self
                            .deps
                            .role_prompts
                            .keys()
                            .cloned()
                            .collect::<Vec<_>>()
                            .join(", ");
                        return ToolOutput::error(if known.is_empty() {
                            format!("unknown role '{role}': no roles are defined")
                        } else {
                            format!("unknown role '{role}'; defined roles: {known}")
                        });
                    }
                }
            }
            _ => self.deps.system_prompt.clone(),
        };
        let explicit_agent = args
            .get("agent")
            .and_then(|value| value.as_str())
            .map(str::trim)
            .filter(|value| !value.is_empty());
        // Agent selection is typed tool input. Never infer ownership from
        // model-authored child prose.
        let selected_agent = explicit_agent;
        let profile = match selected_agent
            .as_ref()
            .map(|name| load_agent(&self.deps.cwd, self.deps.trust_project, name))
            .transpose()
        {
            Ok(profile) => profile.flatten(),
            Err(error) => return ToolOutput::error(error),
        };
        if profile.is_none() && selected_agent.is_some() {
            return ToolOutput::error(format!(
                "unknown Agent '{}'",
                selected_agent.unwrap_or_default()
            ));
        }
        let child_tool_names = child_tools
            .iter()
            .map(|tool| tool.name())
            .collect::<Vec<_>>();
        let child_capabilities: Vec<CapabilityDescriptor> = self
            .deps
            .capabilities
            .iter()
            .filter(|capability| match capability.kind {
                CapabilityKind::Tool => child_tool_names.contains(&capability.name.as_str()),
                CapabilityKind::Skill | CapabilityKind::McpServer => true,
                CapabilityKind::Hook | CapabilityKind::Command => false,
            })
            .cloned()
            .collect();
        let path =
            SessionPath::new_session_file(&self.deps.sessions_home, &self.deps.cwd, &session_id);
        // One identity, composed through the host's resolver for exactly the
        // tools this child has — never the parent's prompt with a second
        // identity appended, and never a prompt naming tools it lacks.
        let child_identity = profile
            .as_ref()
            .map(|profile| vak_session::types::AgentIdentity {
                id: profile.id.clone(),
                revision: profile.revision,
                name: profile.name.clone(),
                character: profile.character.clone(),
                personality: profile.personality.clone(),
                animation: "subtle".into(),
                voice: "default".into(),
                behaviour: profile.behaviour.clone(),
                responsibilities: profile.responsibilities.clone(),
                instructions: profile.instructions.clone(),
            })
            .or_else(|| self.deps.parent_agent_identity.clone());
        if let Some(compose) = &self.deps.child_prompt {
            let role = args
                .get("role")
                .and_then(|r| r.as_str())
                .map(str::trim)
                .filter(|role| !role.is_empty());
            child_system_prompt = compose(role, child_identity.as_ref(), &child_capabilities);
        }
        if let Some(objective) = self.deps.outcome_objective.as_deref()
            && !objective.trim().is_empty()
        {
            child_system_prompt.push_str("\n\nParent outcome objective: ");
            child_system_prompt.push_str(objective.trim());
            child_system_prompt.push_str(
                "\nTreat this as alignment context; permissions and completion remain runtime decisions.",
            );
        }
        let prompt_layers = child_identity
            .as_ref()
            .map(|identity| {
                let text = format!(
                    "{}\n{}\n{}\n{}\n{}",
                    identity.name,
                    identity.personality,
                    identity.behaviour,
                    identity.responsibilities,
                    identity.instructions
                );
                let digest = format!("{:x}", Sha256::digest(text.as_bytes()));
                vec![vak_session::types::PromptLayerDescriptor {
                    block: "identity".into(),
                    layer: "agent".into(),
                    source: Some(identity.id.clone()),
                    digest,
                    bytes: text.len(),
                }]
            })
            .unwrap_or_default();
        // The child is its own run, caused by this call of the parent's.
        let child_trace = ctx.trace.as_ref().map(|parent| {
            let tool_use_id = ctx
                .sandbox_sink
                .as_ref()
                .map_or("", |sink| sink.execution_id());
            let child_agent = child_identity.as_ref().map_or("vak", |a| a.id.as_str());
            parent.delegate(child_agent, tool_use_id)
        });
        let header = SessionHeader {
            space: None,
            run: child_trace.as_ref().map(|t| t.run),
            cause: child_trace.as_ref().map(|t| t.cause.clone()),
            agent: child_identity.clone().or_else(|| Some(vak_core_identity())),
            session_id: session_id.clone(),
            created_at: chrono::Utc::now(),
            cwd: self.deps.cwd.clone(),
            parent_session_id: Some(self.deps.parent_session_id.clone()),
            contract_id: args
                .get("contract_id")
                .and_then(|value| value.as_str())
                .map(str::to_string)
                .or_else(|| self.deps.contract_id.clone()),
            work_item_id: args
                .get("work_item_id")
                .and_then(|value| value.as_str())
                .map(str::to_string)
                .or_else(|| self.deps.work_item_id.clone()),
            conversation: Some(vak_session::ConversationContext::local(
                &session_id,
                "worker",
            )),
            contract: FrozenContract {
                app_version: env!("CARGO_PKG_VERSION").into(),
                provider: self.deps.provider.name().into(),
                model: self.deps.model.clone(),
                route_ladder: Vec::new(),
                route_objective: String::new(),
                route_annotations: Vec::new(),
                system_prompt: child_system_prompt.clone(),
                permission_mode: match child_mode {
                    Mode::ReadOnly => "read-only",
                    Mode::WorkspaceWrite => "workspace-write",
                    Mode::FullAccess => "full-access",
                }
                .into(),
                capabilities: child_capabilities,
                prompt_layers,
            },
        };
        let log = match SessionLog::create(path, header) {
            Ok(l) => l.with_objects(self.deps.objects.clone()),
            Err(e) => return ToolOutput::error(format!("cannot create child session: {e}")),
        };

        let label = args
            .get("label")
            .and_then(|l| l.as_str())
            .map(String::from)
            .or_else(|| profile.as_ref().map(|profile| profile.name.clone()))
            .unwrap_or_else(|| prompt.chars().take(48).collect());
        // A worker can always ask its parent one question
        // (docs/design/84-worker-questions-and-control.md). It is added here
        // and never to the parent's own tool set, so no turn that cannot use
        // it is told about it. Without a registry (deterministic fixtures)
        // there is no board to ask on.
        let mut child_tools = child_tools;
        let mut child_only_definitions = Vec::new();
        if let Some(registry) = &self.deps.registry {
            let ask_parent = Arc::new(crate::questions::AskParentTool::new(
                registry.clone(),
                session_id.clone(),
                label.clone(),
                self.deps.parent_session_id.clone(),
                self.deps
                    .approver
                    .as_ref()
                    .is_some_and(|a| a.answers_questions()),
                self.deps.events.clone(),
                self.deps.approver.clone(),
            ));
            child_only_definitions.push(vak_llm::ToolDefinition::new(
                ask_parent.name(),
                ask_parent.description(),
                ask_parent.schema(),
            ));
            child_tools.push(ask_parent);
        }

        let mut cfg = AgentConfig::new(child_system_prompt.clone());
        cfg.model = self.deps.model.clone();
        cfg.tail = self.deps.tail.clone();
        let child_tool_names: std::collections::BTreeSet<_> =
            child_tools.iter().map(|tool| tool.name()).collect();
        let mut definitions: Vec<_> = self
            .deps
            .tool_definitions
            .iter()
            .filter(|definition| child_tool_names.contains(definition.name.as_str()))
            .cloned()
            .collect();
        definitions.extend(child_only_definitions);
        cfg.tool_definitions = Some(definitions);
        cfg.tools = child_tools;
        cfg.hooks = self.deps.hooks.clone();
        cfg.revocation_check = self.deps.revocation_check.clone();
        cfg.presentation_rebuild = self.deps.presentation_rebuild.clone();
        cfg.mcp_tool_index = self.deps.mcp_tool_index.clone().unwrap_or_default();
        cfg.input_normalizer = self.deps.input_normalizer.clone();
        cfg.max_turns = self.deps.max_turns;
        cfg.capacity = self.deps.capacity.clone();
        cfg.capacity_key = self.deps.capacity_key.clone();
        cfg.max_output = self.deps.max_output;
        cfg.declared_window = self.deps.declared_window;
        cfg.max_retries = self.deps.max_retries;
        cfg.retry_base_backoff_ms = self.deps.retry_base_backoff_ms;
        cfg.request_timeout = self.deps.request_timeout;
        cfg.circuit_breaker = self.deps.circuit_breaker.clone();
        cfg.run_retry_attempts = self.deps.run_retry_attempts;
        cfg.run_retry_base_backoff_ms = self.deps.run_retry_base_backoff_ms;
        cfg.dispatch_ceiling = self.deps.dispatch_ceiling;
        cfg.ladder = self.deps.ladder.clone();
        cfg.ladder_provider_names = self.deps.ladder_provider_names.clone();
        cfg.provider_name = self.deps.provider_name.clone();
        cfg.spend_gate = self.deps.spend_gate.clone();
        cfg.outcome = child_outcome.clone();
        cfg.parallel_tools = true;
        cfg.permission = self.deps.permission.clone();
        cfg.mode = child_mode;
        cfg.approval_mode = self.deps.approval_mode;
        cfg.approver = self.deps.approver.clone();
        cfg.sandbox = self.deps.sandbox.clone();
        cfg.trace = child_trace;

        let agent = Agent::new(self.deps.provider.clone(), log, cfg);
        let steering = Arc::new(SteeringQueues::new());
        let cancel = ctx.cancel.child_token();
        let background = args
            .get("background")
            .and_then(|b| b.as_bool())
            .unwrap_or(false);
        let progress = Arc::new(Mutex::new(WorkerProgress::default()));
        let registry_guard = if let Some(registry) = &self.deps.registry {
            if let Err(reason) = registry.try_register(
                session_id.clone(),
                WorkerHandle {
                    label: label.clone(),
                    agent_id: profile.as_ref().map(|profile| profile.id.clone()),
                    agent_revision: profile.as_ref().map(|profile| profile.revision),
                    started_at: std::time::Instant::now(),
                    steering: steering.clone(),
                    cancel: cancel.clone(),
                    parent_session_id: self.deps.parent_session_id.clone(),
                    background,
                    write_scopes: write_scopes.clone(),
                    progress: progress.clone(),
                },
            ) {
                return ToolOutput::error(reason);
            }
            Some(RegistryGuard {
                registry: registry.clone(),
                id: session_id.clone(),
            })
        } else {
            None
        };
        let drive = DriveChild {
            agent,
            steering,
            cancel,
            prompt: prompt.to_string(),
            label: label.clone(),
            session_id: session_id.clone(),
            contracted: requested_contract.is_some(),
            child_outcome,
            parent_events: self.deps.events.clone(),
            progress,
            registry_guard,
            finish_into: background
                .then(|| self.deps.registry.clone())
                .flatten()
                .map(|registry| (registry, self.deps.parent_session_id.clone())),
        };
        if !background {
            return drive.run().await;
        }
        tokio::spawn(drive.run());
        ToolOutput::ok(format!(
            "worker '{session_id}' ({label}) started in the background. Use the workers tool to list it, check its status, message it, wait for it, or stop it. It is cancelled if you finish your turn without waiting for it."
        ))
    }
}

/// One child run, owned so it can be awaited inline or run on its own task
/// (`task { background: true }`).
struct DriveChild {
    agent: Agent,
    steering: Arc<SteeringQueues>,
    cancel: CancellationToken,
    prompt: String,
    label: String,
    session_id: String,
    contracted: bool,
    child_outcome: Option<vak_intent::OutcomeSpec>,
    parent_events: Option<tokio::sync::mpsc::Sender<crate::AgentEvent>>,
    progress: Arc<Mutex<WorkerProgress>>,
    registry_guard: Option<RegistryGuard>,
    /// Where a background worker records how it ended, with its parent.
    finish_into: Option<(Arc<WorkerRegistry>, String)>,
}

impl DriveChild {
    async fn run(self) -> ToolOutput {
        let DriveChild {
            mut agent,
            steering,
            cancel,
            prompt,
            label,
            session_id,
            contracted,
            child_outcome,
            parent_events,
            progress,
            registry_guard,
            finish_into,
        } = self;
        let _registry_guard = registry_guard;
        let (ev_tx, mut ev_rx) = tokio::sync::mpsc::channel::<crate::AgentEvent>(256);
        // Always drain the child stream (a full channel would deadlock the
        // child loop); tool calls are additionally forwarded to the parent
        // event stream so parallel workers are visible in the UI.
        let parent = parent_events.clone();
        let fwd_label = label.clone();
        let pump = tokio::spawn(async move {
            while let Some(ev) = ev_rx.recv().await {
                match ev {
                    crate::AgentEvent::ToolCallEnd { name, is_error, .. } => {
                        progress
                            .lock()
                            .unwrap_or_else(std::sync::PoisonError::into_inner)
                            .tool_finished(&name);
                        if let Some(parent) = &parent {
                            let _ = parent
                                .send(crate::AgentEvent::WorkerToolCall {
                                    label: fwd_label.clone(),
                                    name,
                                    is_error,
                                })
                                .await;
                        }
                    }
                    crate::AgentEvent::TurnEnd { usage } => {
                        progress
                            .lock()
                            .unwrap_or_else(std::sync::PoisonError::into_inner)
                            .step_finished(usage.input_tokens, usage.output_tokens);
                        if let Some(parent) = &parent {
                            let _ = parent
                                .send(crate::AgentEvent::WorkerUsage {
                                    label: fwd_label.clone(),
                                    input_tokens: usage.input_tokens,
                                    output_tokens: usage.output_tokens,
                                })
                                .await;
                        }
                    }
                    crate::AgentEvent::Sandbox(event) => {
                        // Sandbox output belongs to the parent surface too:
                        // worker tool calls execute through the same
                        // broker and must remain visible and rehydratable in
                        // Workbench with their own execution identity.
                        if let Some(parent) = &parent {
                            let _ = parent.send(crate::AgentEvent::Sandbox(event)).await;
                        }
                    }
                    _ => {}
                }
            }
        });
        if let Some(events) = &parent_events {
            let _ = events
                .send(crate::AgentEvent::WorkerStarted {
                    label: label.clone(),
                })
                .await;
        }
        let started = std::time::Instant::now();
        let outcome = agent.run(&prompt, &steering, cancel, ev_tx).await;
        let child_status = match &outcome {
            crate::TurnOutcome::Completed { .. } => vak_session::types::ChildRunStatus::Completed,
            crate::TurnOutcome::Failed { .. } => vak_session::types::ChildRunStatus::Failed,
            crate::TurnOutcome::Aborted { .. } => vak_session::types::ChildRunStatus::Aborted,
            crate::TurnOutcome::MaxTurnsReached => vak_session::types::ChildRunStatus::MaxTurns,
        };
        // Persist the terminal marker before notifying the parent. Recovery
        // must never observe a finished child without a durable status.
        let _ = agent
            .session
            .lock()
            .await
            .append_child_run_result(child_status, child_outcome);
        if let Some(events) = &parent_events {
            let _ = events
                .send(crate::AgentEvent::WorkerFinished {
                    label: label.clone(),
                    is_error: !matches!(outcome, crate::TurnOutcome::Completed { .. }),
                    elapsed_ms: started.elapsed().as_millis() as u64,
                })
                .await;
        }
        let _ = pump.await;
        let cards: Vec<vak_tools::PresentationCard> = agent
            .session
            .lock()
            .await
            .presentations()
            .into_iter()
            .map(|(_, record)| vak_tools::PresentationCard {
                semantic_type: record.semantic_type.clone(),
                skill_id: record.skill_id.clone(),
                skill_version: record.skill_version.clone(),
                schema_version: record.schema_version,
                payload: record.payload.clone(),
                title: record.title.clone(),
                identity_digest: record.identity_digest.clone(),
            })
            .collect();
        let mut output = worker_output(&session_id, outcome, contracted);
        if !cards.is_empty() {
            output.delegated = Some(vak_tools::DelegatedCards {
                session_id: session_id.clone(),
                cards,
            });
        }
        // A background worker has no `task` call to return its result to, so
        // it is kept for the parent's `workers` tool, recorded before the
        // registry guard drops so a waiter never sees neither.
        if let Some((registry, parent_session_id)) = finish_into {
            registry.finish(FinishedWorker {
                id: session_id,
                label,
                parent_session_id,
                is_error: output.is_error,
                text: output.content.clone(),
                elapsed_secs: started.elapsed().as_secs(),
            });
        }
        output
    }
}

/// The `task` result for a finished worker: its final text, or why there is
/// none. The worker's cards travel beside it (`ToolOutput::delegated`).
fn worker_output(session_id: &str, outcome: crate::TurnOutcome, contracted: bool) -> ToolOutput {
    match outcome {
        crate::TurnOutcome::Completed { response } => {
            let text = response.text_content();
            if text.is_empty() {
                ToolOutput::ok(format!("worker '{session_id}' completed without output"))
            } else {
                if contracted {
                    ToolOutput::ok(format!("worker '{session_id}' completed:\n{text}"))
                } else {
                    ToolOutput::ok(text)
                }
            }
        }
        crate::TurnOutcome::Aborted { partial } => {
            let text = partial.map(|p| p.text_content()).unwrap_or_default();
            ToolOutput::error(format!(
                "worker '{session_id}' was cancelled. Partial output:\n{text}"
            ))
        }
        crate::TurnOutcome::Failed { error } => {
            ToolOutput::error(format!("worker '{session_id}' failed: {error}"))
        }
        crate::TurnOutcome::MaxTurnsReached => ToolOutput::error(format!(
            "worker '{session_id}' hit its turn limit before finishing"
        )),
    }
}

/// A child session's id, unique by construction.
///
/// The contract a worker is held to: what its own prompt asks for, not the
/// parent's whole request. A research worker spawned from "fix the bug and
/// run the tests" was held to the parent's execution requirement and sent
/// back to act on files its task never asked it to touch. A read-only worker
/// is clamped further: its contract may not demand an effect or a file its
/// tools cannot produce. Read with the same tier-1 reader as every turn; the
/// parent's evidence freshness and turn budget carry over.
fn child_outcome(
    prompt: &str,
    readonly: bool,
    parent: Option<&vak_intent::OutcomeSpec>,
) -> Option<vak_intent::OutcomeSpec> {
    let parent = parent?;
    let request = vak_intent::Request {
        text: prompt,
        surface: vak_intent::Surface::Worker,
        ..vak_intent::Request::default()
    };
    let intent = vak_intent::resolve(
        &request,
        &vak_intent::Declared::default(),
        &vak_intent::Authority::default(),
        &vak_intent::ResolverConfig::default(),
    )
    .intent();
    let mut spec = vak_intent::OutcomeSpec::from_intent(prompt, &intent);
    spec.evidence_max_age_secs = parent.evidence_max_age_secs;
    spec.max_turns = parent.max_turns;
    if readonly {
        spec.acts
            .retain(|act| !act.requires_execution() && !matches!(act, vak_intent::Act::Author));
        if spec.acts.is_empty() {
            spec.acts.insert(vak_intent::Act::Analyze);
        }
        if spec.stop.rank() > vak_intent::StopProfile::Inspection.rank() {
            spec.stop = vak_intent::StopProfile::Inspection;
        }
    }
    Some(spec)
}

/// The id (a full UUIDv7 plus the sequence) names the child's ledger file, and the file is exclusively locked, so
/// two children with the same id cannot both exist. The id used to be the
/// clock's nanoseconds alone; tasks launched in the same wave can read the same
/// value (clock resolution is coarser than the launch rate), and the second
/// then failed with "session is locked by another process" — a spurious error
/// the stop gate turned into an extra parent turn. A process-wide counter makes
/// a collision impossible whatever the clock does.
fn next_child_session_id() -> String {
    static SEQUENCE: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    let sequence = SEQUENCE.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    format!("child-{}-{sequence}", uuid::Uuid::now_v7())
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod registry_tests {
    use super::*;

    #[test]
    fn worker_cap_is_atomic_and_shared_by_foreground_and_background() {
        let registry = Arc::new(WorkerRegistry::new());
        let workers: Vec<_> = (0..MAX_WORKERS + 4)
            .map(|index| {
                let registry = registry.clone();
                std::thread::spawn(move || {
                    registry.try_register(
                        format!("child-{index}"),
                        WorkerHandle {
                            label: format!("worker-{index}"),
                            agent_id: None,
                            agent_revision: None,
                            started_at: std::time::Instant::now(),
                            steering: Arc::new(SteeringQueues::new()),
                            cancel: CancellationToken::new(),
                            parent_session_id: "parent".into(),
                            background: index % 2 == 0,
                            write_scopes: Vec::new(),
                            progress: Default::default(),
                        },
                    )
                })
            })
            .collect();
        let admitted = workers
            .into_iter()
            .map(|worker| worker.join().expect("worker registration thread"))
            .filter(Result::is_ok)
            .count();
        assert_eq!(admitted, MAX_WORKERS);
        assert_eq!(registry.active_for("parent").len(), MAX_WORKERS);
    }

    fn parent_contract(text: &str) -> vak_intent::OutcomeSpec {
        let request = vak_intent::Request {
            text,
            ..vak_intent::Request::default()
        };
        let intent = vak_intent::resolve(
            &request,
            &vak_intent::Declared::default(),
            &vak_intent::Authority::default(),
            &vak_intent::ResolverConfig::default(),
        )
        .intent();
        vak_intent::OutcomeSpec::from_intent(text, &intent)
    }

    /// A worker answers for its own task. A research child of an effectful
    /// request is not held to the parent's execution requirement.
    #[test]
    fn a_worker_is_held_to_its_own_task_not_the_parents() {
        let parent = parent_contract("fix the failing test and run the suite");
        assert!(parent.requires_execution());
        let child = child_outcome(
            "find where the parser handles empty input",
            false,
            Some(&parent),
        )
        .expect("a parent contract yields a child contract");
        assert!(!child.requires_execution(), "acts={:?}", child.acts);
        assert_eq!(child.objective, "find where the parser handles empty input");
    }

    /// A read-only worker cannot be asked for what its tools cannot do,
    /// whatever its prompt says.
    #[test]
    fn a_read_only_worker_is_never_owed_an_effect() {
        let parent = parent_contract("fix the failing test");
        let child = child_outcome(
            "update the parser and save the notes as notes.md",
            true,
            Some(&parent),
        )
        .expect("child contract");
        assert!(!child.requires_execution(), "acts={:?}", child.acts);
        assert!(child.stop.rank() <= vak_intent::StopProfile::Inspection.rank());
        // No parent contract, no child contract: nothing to be held to.
        assert!(child_outcome("anything", false, None).is_none());
    }

    #[test]
    fn duplicate_agent_names_fail_closed() {
        vak_config::paths::isolate_home_for_tests();
        let dir = tempfile::tempdir().expect("Agent workspace");
        std::fs::create_dir_all(dir.path().join(".vak")).expect("profile directory");
        std::fs::write(
            dir.path().join(".vak/agents.json"),
            r#"[{"id":"one","revision":1,"name":"Pip","character":"pip","personality":"","behaviour":""},{"id":"two","revision":1,"name":"Pip","character":"pip","personality":"","behaviour":""}]"#,
        )
        .expect("profiles");
        let result = load_agent(dir.path(), true, "Pip");
        assert!(matches!(result, Err(error) if error.contains("ambiguous")));
    }

    /// A project's saved Agents are selectable only in a trusted workspace,
    /// as for a top-level Agent (`vak-server/src/agents.rs::effective`).
    #[test]
    fn untrusted_project_agents_are_not_selectable() {
        vak_config::paths::isolate_home_for_tests();
        let dir = tempfile::tempdir().expect("Agent workspace");
        std::fs::create_dir_all(dir.path().join(".vak")).expect("profile directory");
        std::fs::write(
            dir.path().join(".vak/agents.json"),
            r#"[{"id":"repo-agent","revision":1,"name":"Repo","character":"pip","personality":"","behaviour":""}]"#,
        )
        .expect("profiles");
        assert!(matches!(
            load_agent(dir.path(), false, "repo-agent"),
            Ok(None)
        ));
        assert!(matches!(
            load_agent(dir.path(), true, "repo-agent"),
            Ok(Some(_))
        ));
    }

    #[test]
    fn register_steer_stop_lifecycle() {
        let reg = WorkerRegistry::new();
        assert!(reg.active().is_empty());
        assert!(!reg.steer("child-1", "go"));
        assert!(!reg.stop("child-1"));

        let cancel = CancellationToken::new();
        reg.register(
            "child-1".into(),
            WorkerHandle {
                label: "explore".into(),
                agent_id: None,
                agent_revision: None,
                started_at: std::time::Instant::now(),
                steering: Arc::new(SteeringQueues::new()),
                cancel: cancel.clone(),
                parent_session_id: "parent-a".into(),
                background: false,
                write_scopes: Vec::new(),
                progress: Default::default(),
            },
        );
        assert!(reg.steer("child-1", "look left"));
        assert!(reg.queue_follow_up("child-1", "then right"));
        let active = reg.active();
        assert_eq!(active.len(), 1);
        assert_eq!(active[0].id, "child-1");
        assert_eq!(active[0].label, "explore");

        // The queued items landed in the child's queues.
        assert!(!cancel.is_cancelled());
        assert!(reg.stop("child-1"));
        assert!(cancel.is_cancelled());

        reg.unregister("child-1");
        assert!(reg.active().is_empty());
        assert!(!reg.steer("child-1", "gone"));
    }

    #[test]
    fn scope_checks_route_children_to_their_own_parent() {
        let reg = WorkerRegistry::new();
        for (id, parent) in [("c1", "pA"), ("c2", "pB")] {
            reg.register(
                id.into(),
                WorkerHandle {
                    label: id.into(),
                    agent_id: None,
                    agent_revision: None,
                    started_at: std::time::Instant::now(),
                    steering: Arc::new(SteeringQueues::new()),
                    cancel: CancellationToken::new(),
                    parent_session_id: parent.into(),
                    background: false,
                    write_scopes: Vec::new(),
                    progress: Default::default(),
                },
            );
        }
        assert_eq!(
            reg.parent_of("c1").as_deref(),
            Some("pA"),
            "ownership is recorded"
        );
        assert_eq!(reg.parent_of("missing"), None);
        let a = reg.active_for("pA");
        assert_eq!(a.len(), 1);
        assert_eq!(a[0].id, "c1");
        assert_eq!(reg.active_for("pB")[0].id, "c2");
        assert!(reg.active_for("pC").is_empty());
    }

    #[test]
    fn a_lease_is_the_workers_for_as_long_as_it_is_registered() {
        let registry = WorkerRegistry::new();
        registry.register(
            "w1".into(),
            WorkerHandle {
                label: "scribe".into(),
                agent_id: None,
                agent_revision: None,
                started_at: std::time::Instant::now(),
                steering: Arc::new(SteeringQueues::new()),
                cancel: CancellationToken::new(),
                parent_session_id: "p".into(),
                background: true,
                write_scopes: vec!["out".into()],
                progress: Default::default(),
            },
        );
        let touching = |path: &str| vak_tools::ResourceClaims {
            exclusive: false,
            read_only: false,
            paths: vec![path.into()],
        };
        assert_eq!(
            registry.lease_conflict("p", &touching("out/x")).as_deref(),
            Some("w1")
        );
        assert_eq!(registry.lease_conflict("p", &touching("src/x")), None);
        assert_eq!(
            registry.lease_conflict("someone-else", &touching("out/x")),
            None,
            "another session's writer is not this session's business"
        );
        let shell = vak_tools::ResourceClaims {
            exclusive: true,
            read_only: false,
            paths: Vec::new(),
        };
        assert_eq!(
            registry.lease_conflict("p", &shell).as_deref(),
            Some("w1"),
            "a command that could touch anything waits"
        );
        let read = vak_tools::ResourceClaims {
            exclusive: false,
            read_only: true,
            paths: Vec::new(),
        };
        assert_eq!(registry.lease_conflict("p", &read), None);
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod child_id_tests {
    use super::next_child_session_id;

    #[test]
    fn child_session_ids_never_collide_even_when_generated_in_the_same_instant() {
        let ids: std::collections::HashSet<String> =
            (0..10_000).map(|_| next_child_session_id()).collect();
        assert_eq!(
            ids.len(),
            10_000,
            "ids must be unique regardless of clock resolution"
        );
    }

    #[test]
    fn child_session_ids_are_unique_across_threads() {
        let handles: Vec<_> = (0..8)
            .map(|_| {
                std::thread::spawn(|| {
                    (0..2_000)
                        .map(|_| next_child_session_id())
                        .collect::<Vec<_>>()
                })
            })
            .collect();
        let all: Vec<String> = handles
            .into_iter()
            .flat_map(|h| h.join().unwrap())
            .collect();
        let unique: std::collections::HashSet<_> = all.iter().collect();
        assert_eq!(unique.len(), all.len());
    }
}
