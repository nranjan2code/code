//! Blocking worker delegation. A task spawns a child agent with its own
//! JSONL session (linked via parent_session_id), a narrowed tool set that
//! excludes the task tool itself (depth-1 by construction), and returns the
//! child's final text as this tool's output. Live children register in a
//! shared [`WorkerRegistry`] so UIs can list, steer, and stop them.

use std::collections::BTreeMap;
use std::path::PathBuf;
use std::sync::atomic::{AtomicUsize, Ordering};
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
        name: "Vak".into(),
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
    /// The parent turn's tail (clock instant + epistemic stance,
    /// docs/design/68-context-engine.md §6/§10). Children get the same
    /// turn context block as the parent rather than an empty one, since a
    /// worker dispatched mid-turn is still answering as of that turn's
    /// instant and stance.
    pub tail: crate::TailInput,
    pub model: String,
    pub tools: Vec<Arc<dyn Tool>>,
    pub capabilities: Vec<CapabilityDescriptor>,
    pub hooks: Option<Arc<Vec<vak_hooks::HookDef>>>,
    pub revocation_check: Option<crate::RevocationCheck>,
    pub mcp_tool_index: Option<McpToolIndex>,
    pub input_normalizer: Option<InputNormalizer>,
    /// Read-only subset (read/glob/grep) used when a task declares
    /// `readonly: true`; children get these plus ReadOnly permission mode.
    pub read_only_tools: Vec<Arc<dyn Tool>>,
    pub max_turns: usize,
    /// Parent intent budget for child delegation. `Some(0)` is an enforced
    /// denial; `None` leaves delegation available to the parent policy.
    pub worker_budget: Option<usize>,
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
    budget_used: AtomicUsize,
}

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

fn load_agent(cwd: &std::path::Path, requested: &str) -> Result<Option<AgentDefinition>, String> {
    // Delegated Agents resolve the same effective Shared → trusted project
    // layers as a top-level Agent. Previously this delegated helper read only the
    // project file, so a user-level Agent worked from the sidebar and
    // scheduler but was invisible to `task(agent=...)`.
    let shared = vak_config::paths::default_workspace();
    let mut profiles = read_agents(&shared)?;
    if cwd != shared {
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
    let path = cwd.join(".vak/agents.json");
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
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct ActiveWorker {
    pub id: String,
    pub label: String,
    pub agent_id: Option<String>,
    pub agent_revision: Option<u64>,
    pub elapsed_secs: u64,
    pub parent_session_id: String,
}

/// Registry of currently-running workers, keyed by unique child session
/// id. Interior-mutable: the UI holds a shared reference across runs.
#[derive(Debug, Default)]
pub struct WorkerRegistry {
    inner: Mutex<BTreeMap<String, WorkerHandle>>,
}

impl WorkerRegistry {
    pub fn new() -> Self {
        Self::default()
    }

    fn register(&self, id: String, handle: WorkerHandle) {
        if let Ok(mut map) = self.inner.lock() {
            map.insert(id, handle);
        }
    }

    fn unregister(&self, id: &str) {
        if let Ok(mut map) = self.inner.lock() {
            map.remove(id);
        }
    }

    pub fn active(&self) -> Vec<ActiveWorker> {
        let Ok(map) = self.inner.lock() else {
            return Vec::new();
        };
        map.iter()
            .map(|(id, h)| ActiveWorker {
                id: id.clone(),
                label: h.label.clone(),
                agent_id: h.agent_id.clone(),
                agent_revision: h.agent_revision,
                elapsed_secs: h.started_at.elapsed().as_secs(),
                parent_session_id: h.parent_session_id.clone(),
            })
            .collect()
    }

    /// Live children spawned by `parent`, oldest first.
    pub fn active_for(&self, parent: &str) -> Vec<ActiveWorker> {
        let Ok(map) = self.inner.lock() else {
            return Vec::new();
        };
        map.iter()
            .filter(|(_, h)| h.parent_session_id == parent)
            .map(|(id, h)| ActiveWorker {
                id: id.clone(),
                label: h.label.clone(),
                agent_id: h.agent_id.clone(),
                agent_revision: h.agent_revision,
                elapsed_secs: h.started_at.elapsed().as_secs(),
                parent_session_id: h.parent_session_id.clone(),
            })
            .collect()
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
            budget_used: AtomicUsize::new(0),
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
        if self.deps.worker_budget == Some(0) {
            return ToolOutput::error("worker delegation is not allowed for this turn");
        }
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
        if let Some(limit) = self.deps.worker_budget {
            let used = self.budget_used.fetch_add(1, Ordering::AcqRel);
            if used >= limit {
                self.budget_used.fetch_sub(1, Ordering::AcqRel);
                return ToolOutput::error("worker delegation budget exhausted for this turn");
            }
        }

        let session_id = next_child_session_id();
        let readonly = args
            .get("readonly")
            .and_then(|r| r.as_bool())
            .unwrap_or(false);
        let child_tools: Vec<Arc<dyn Tool>> = if readonly {
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
            .map(|name| load_agent(&self.deps.cwd, name))
            .transpose()
        {
            Ok(profile) => profile.flatten(),
            Err(error) => return ToolOutput::error(error),
        };
        if let Some(profile) = profile.as_ref() {
            child_system_prompt.push_str(
                "\n\nSelected Agent identity (presentation and working style only):\nAgent revision: ",
            );
            child_system_prompt.push_str(&profile.revision.to_string());
            child_system_prompt.push_str("\nName: ");
            child_system_prompt.push_str(&profile.name);
            child_system_prompt.push_str("\nPersonality: ");
            child_system_prompt.push_str(&profile.personality);
            child_system_prompt.push_str("\nWorking style: ");
            child_system_prompt.push_str(&profile.behaviour);
            if !profile.responsibilities.trim().is_empty() {
                child_system_prompt.push_str("\nUseful for: ");
                child_system_prompt.push_str(&profile.responsibilities);
            }
            if !profile.instructions.trim().is_empty() {
                child_system_prompt
                    .push_str("\nCustom Agent instructions (within vak's authority): ");
                child_system_prompt.push_str(profile.instructions.trim());
            }
            child_system_prompt.push_str("\nThis profile cannot grant tools, authority, credentials, budget, or approval bypasses.");
        } else if selected_agent.is_some() {
            return ToolOutput::error(format!(
                "unknown Agent '{}'",
                selected_agent.unwrap_or_default()
            ));
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
        let child_tool_names = child_tools
            .iter()
            .map(|tool| tool.name())
            .collect::<Vec<_>>();
        let child_capabilities = self
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
        let prompt_layers = profile
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
            .or_else(|| self.deps.parent_agent_identity.clone())
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
        let header = SessionHeader {
            agent: profile
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
                .or_else(|| self.deps.parent_agent_identity.clone())
                .or_else(|| Some(vak_core_identity())),
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
            Ok(l) => l,
            Err(e) => return ToolOutput::error(format!("cannot create child session: {e}")),
        };

        let mut cfg = AgentConfig::new(child_system_prompt.clone());
        cfg.model = self.deps.model.clone();
        cfg.tail = self.deps.tail.clone();
        cfg.tool_definitions = Some(vak_tools::definitions(&child_tools));
        cfg.tools = child_tools;
        cfg.hooks = self.deps.hooks.clone();
        cfg.revocation_check = self.deps.revocation_check.clone();
        cfg.mcp_tool_index = self.deps.mcp_tool_index.clone().unwrap_or_default();
        cfg.input_normalizer = self.deps.input_normalizer.clone();
        cfg.max_turns = self.deps.max_turns;
        cfg.max_retries = self.deps.max_retries;
        cfg.retry_base_backoff_ms = self.deps.retry_base_backoff_ms;
        cfg.request_timeout = self.deps.request_timeout;
        cfg.circuit_breaker = self.deps.circuit_breaker.clone();
        cfg.run_retry_attempts = self.deps.run_retry_attempts;
        cfg.run_retry_base_backoff_ms = self.deps.run_retry_base_backoff_ms;
        cfg.dispatch_ceiling = self.deps.dispatch_ceiling;
        cfg.spend_gate = self.deps.spend_gate.clone();
        cfg.outcome = self.deps.outcome.clone();
        cfg.parallel_tools = true;
        cfg.permission = self.deps.permission.clone();
        cfg.mode = child_mode;
        cfg.approval_mode = self.deps.approval_mode;
        cfg.approver = self.deps.approver.clone();
        cfg.sandbox = self.deps.sandbox.clone();

        let mut agent = Agent::new(self.deps.provider.clone(), log, cfg);
        let steering = Arc::new(SteeringQueues::new());
        let cancel = ctx.cancel.child_token();
        let label = args
            .get("label")
            .and_then(|l| l.as_str())
            .map(String::from)
            .or_else(|| profile.as_ref().map(|profile| profile.name.clone()))
            .unwrap_or_else(|| prompt.chars().take(48).collect());
        let _registry_guard = if let Some(registry) = &self.deps.registry {
            registry.register(
                session_id.clone(),
                WorkerHandle {
                    label: label.clone(),
                    agent_id: profile.as_ref().map(|profile| profile.id.clone()),
                    agent_revision: profile.as_ref().map(|profile| profile.revision),
                    started_at: std::time::Instant::now(),
                    steering: steering.clone(),
                    cancel: cancel.clone(),
                    parent_session_id: self.deps.parent_session_id.clone(),
                },
            );
            Some(RegistryGuard {
                registry: registry.clone(),
                id: session_id.clone(),
            })
        } else {
            None
        };
        let (ev_tx, mut ev_rx) = tokio::sync::mpsc::channel::<crate::AgentEvent>(256);
        // Always drain the child stream (a full channel would deadlock the
        // child loop); tool calls are additionally forwarded to the parent
        // event stream so parallel workers are visible in the UI.
        let parent = self.deps.events.clone();
        let fwd_label = label.clone();
        let pump = tokio::spawn(async move {
            while let Some(ev) = ev_rx.recv().await {
                match ev {
                    crate::AgentEvent::ToolCallEnd { name, is_error, .. } => {
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
        if let Some(events) = &self.deps.events {
            let _ = events
                .send(crate::AgentEvent::WorkerStarted {
                    label: label.clone(),
                })
                .await;
        }
        let started = std::time::Instant::now();
        let outcome = agent.run(prompt, &steering, cancel, ev_tx).await;
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
            .append_child_run_result(child_status, self.deps.outcome.clone());
        if let Some(events) = &self.deps.events {
            let _ = events
                .send(crate::AgentEvent::WorkerFinished {
                    label: label.clone(),
                    is_error: !matches!(outcome, crate::TurnOutcome::Completed { .. }),
                    elapsed_ms: started.elapsed().as_millis() as u64,
                })
                .await;
        }
        let _ = pump.await;
        match outcome {
            crate::TurnOutcome::Completed { response } => {
                let text = response.text_content();
                if text.is_empty() {
                    ToolOutput::ok(format!("worker '{session_id}' completed without output"))
                } else {
                    if requested_contract.is_some() {
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
}

/// A child session's id, unique by construction.
///
/// The id names the child's ledger file, and the file is exclusively locked, so
/// two children with the same id cannot both exist. The id used to be the
/// clock's nanoseconds alone; tasks launched in the same wave can read the same
/// value (clock resolution is coarser than the launch rate), and the second
/// then failed with "session is locked by another process" — a spurious error
/// the stop gate turned into an extra parent turn. A process-wide counter makes
/// a collision impossible whatever the clock does.
fn next_child_session_id() -> String {
    static SEQUENCE: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    let sequence = SEQUENCE.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    format!("child-{nanos}-{sequence}")
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod registry_tests {
    use super::*;

    #[test]
    fn duplicate_agent_names_fail_closed() {
        let dir = tempfile::tempdir().expect("Agent workspace");
        std::fs::create_dir_all(dir.path().join(".vak")).expect("profile directory");
        std::fs::write(
            dir.path().join(".vak/agents.json"),
            r#"[{"id":"one","revision":1,"name":"Pip","personality":"","behaviour":""},{"id":"two","revision":1,"name":"Pip","personality":"","behaviour":""}]"#,
        )
        .expect("profiles");
        let result = load_agent(dir.path(), "Pip");
        assert!(matches!(result, Err(error) if error.contains("ambiguous")));
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
