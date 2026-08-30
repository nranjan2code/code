//! vak-core: the SDK facade. Composes config, provider, tools, session and
//! the agent loop behind one entry point. TUI, server, and exec mode are
//! thin consumers of this crate.

pub mod backup;
pub mod checkpoints;
pub mod custom_commands;
pub mod digest;
pub mod files;
pub mod finops;
pub mod health;
pub mod inbox;
pub mod install;
pub mod learning;
pub mod memory;
pub mod reflection;
pub mod routing;
pub mod sandbox_docker;
pub mod security_events;
pub mod session_search;
pub mod skills;
pub mod tasks;
pub mod tools_tasks;
pub mod transcript_md;
pub mod worktree;

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;

use tokio_util::sync::CancellationToken;
use vak_agent::{Agent, AgentConfig, AgentEvent, TurnOutcome};
use vak_llm::Provider;
use vak_llm::registry::{ProviderAuth, ProviderRegistry, default_registry};
use vak_session::SessionLog;
use vak_session::types::{FrozenContract, SessionHeader};
use vak_tools::sandbox::SandboxMode;

pub const APP_VERSION: &str = env!("CARGO_PKG_VERSION");
pub const DEFAULT_SYSTEM_PROMPT: &str = include_str!("system-prompt.md");

/// Re-exported so consumers (and tests) can name config types via vak_core.
pub use vak_config;

/// Outcome of revoking a provider key.
#[derive(Debug, Clone)]
pub struct RemovedKey {
    pub env_var: String,
    /// True when the variable is still set in the real process environment,
    /// so the provider stays authenticated despite the stored key going away.
    pub shadowed_by_env: bool,
}

#[derive(Debug, thiserror::Error)]
pub enum CoreError {
    #[error("provider auth missing: set {env} for provider '{provider}'")]
    MissingAuth { env: String, provider: String },
    #[error("config error: {0}")]
    Config(#[from] vak_config::ConfigError),
    #[error("session error: {0}")]
    Session(#[from] vak_session::SessionError),
    #[error("provider error: {0}")]
    Llm(#[from] vak_llm::LlmError),
    #[error("permission rule error: {0}")]
    Rule(#[from] vak_permission::RuleError),
    #[error("blocked by hook: {0}")]
    HookBlocked(String),
    #[error("invalid configuration: {0}")]
    InvalidConfig(String),
    #[error("internal: permission engine missing")]
    MissingEngine,
}

/// Stats reported by a successful manual compaction.
#[derive(Debug, Clone, Copy)]
pub struct CompactReport {
    pub before_tokens: u64,
    pub after_tokens: u64,
    pub summarized_messages: usize,
}

/// Result envelope for manual compaction: the caller keeps ownership of the
/// session either way; `report` is `Some` exactly when `error` is `None`.
#[derive(Debug, Clone)]
pub struct CompactOutcome {
    pub report: Option<CompactReport>,
    pub error: Option<String>,
}

impl CompactOutcome {
    fn failed(error: String) -> Self {
        CompactOutcome {
            report: None,
            error: Some(error),
        }
    }
}

impl Core {
    /// Today's estimated spend (local midnight window), USD 0.0 when the
    /// ledger is absent or unpriced rows dominate — absent is zero here
    /// because the ledger itself is the source being displayed.
    pub fn spend_day_usd(&self) -> f64 {
        vak_core_ledger(self).day_total_usd(chrono::Utc::now())
    }

    /// Estimated spend over the trailing `days`, USD.
    pub fn spend_trailing_usd(&self, days: u64) -> f64 {
        vak_core_ledger(self).total_usd_since(
            chrono::Utc::now() - chrono::Duration::hours(days.saturating_mul(24) as i64),
        )
    }
}

fn vak_core_ledger(core: &Core) -> finops::FinOpsLedger {
    finops::FinOpsLedger::new(&core.inner.sessions_home)
}

struct CoreInner {
    config: vak_config::Config,
    cwd: PathBuf,
    sessions_home: PathBuf,
    registry: ProviderRegistry,
    route: std::sync::Mutex<RouteSelection>,
    max_turns_override: std::sync::Mutex<Option<usize>>,
    max_turns_runtime_pinned: std::sync::atomic::AtomicBool,
    mode_override: std::sync::Mutex<Option<vak_config::PermissionMode>>,
    mode_runtime_pinned: std::sync::atomic::AtomicBool,
    theme_override: std::sync::Mutex<Option<String>>,
    theme_runtime_pinned: std::sync::atomic::AtomicBool,
    /// Live overrides for `[memory]` toggles (docs/design/23-memory.md).
    /// No CLI flag pins these today, so unlike route/theme/max_turns there
    /// is no `*_runtime_pinned` counterpart — `refresh_persisted_preferences`
    /// always takes the latest persisted value.
    memory_search_enabled_override: std::sync::Mutex<Option<bool>>,
    memory_write_enabled_override: std::sync::Mutex<Option<bool>>,
    memory_reflection_override: std::sync::Mutex<Option<bool>>,
    memory_skill_proposals_override: std::sync::Mutex<Option<bool>>,
    /// Same no-pin, always-take-latest shape as the memory overrides above.
    subagents_override: std::sync::Mutex<Option<bool>>,
    /// Live overrides for `[finops]` budget caps (docs/design/27 Phase D).
    /// `None` = follow the persisted value; `Some(None)` = explicitly
    /// cleared (no cap); `Some(Some(v))` = pinned to `v`. Distinct from
    /// the other overrides here because "no cap" is a real, settable
    /// value, not merely "unset" — a plain `Mutex<Option<f64>>` couldn't
    /// tell "never overridden" from "overridden to no cap" apart.
    finops_max_run_usd_override: std::sync::Mutex<Option<Option<f64>>>,
    finops_max_day_usd_override: std::sync::Mutex<Option<Option<f64>>>,
    sandbox_backend_override: std::sync::Mutex<Option<String>>,
    provider_instance: std::sync::Mutex<Option<Arc<dyn Provider>>>,
    sessions_home_override: std::sync::Mutex<Option<PathBuf>>,
    breaker: Arc<vak_agent::CircuitBreaker>,
    subagents: Arc<vak_agent::SubagentRegistry>,
    trust_project_config: bool,
    extra_allow: std::sync::Mutex<Vec<String>>,
    user_env_override: std::sync::Mutex<Option<PathBuf>>,
    tool_worker_exe: std::sync::Mutex<PathBuf>,
    /// provider -> (fetched_at, model ids). Discovery is a network call;
    /// pickers re-read it constantly, so results are memoised briefly.
    models_cache: std::sync::Mutex<HashMap<String, (std::time::Instant, Vec<String>)>>,
    /// Runtime MCP table override (desktop/TUI management surface).
    mcp_override: std::sync::Mutex<Option<vak_config::McpConfig>>,
    mcp_runtime_pinned: std::sync::atomic::AtomicBool,
    /// One `McpManager` per distinct server set, reused across turns so
    /// spawned server processes (e.g. `npx tavily-mcp`) and their live
    /// connections survive a whole session instead of respawning every
    /// turn. Keyed by a fingerprint of the resolved server set so a
    /// runtime `set_mcp_servers` call or a plugin enable/disable — both of
    /// which change what `effective_mcp()` returns — transparently swaps
    /// in a fresh manager instead of serving a stale one. The inventory
    /// (server -> tool name/description pairs) is filled in by a
    /// best-effort background warm-up and read by `system_prompt()`; a
    /// turn never blocks on it — see `mcp_manager()` / `cached_mcp_inventory()`.
    mcp_cache: std::sync::Mutex<Option<McpCache>>,
    /// Runtime hook override (desktop/TUI management surface).
    hooks_override: std::sync::Mutex<Option<Vec<vak_config::HookConfig>>>,
    hooks_runtime_pinned: std::sync::atomic::AtomicBool,
    /// Restrictive overlay applied only to a gateway channel Core.
    channel_policy: std::sync::Mutex<Option<vak_config::ChannelPolicy>>,
    /// Session-scoped domain-weighted doubt per (provider, model) leg
    /// (Phase R). Fed from work receipts at run end; read at ladder
    /// admission.
    beliefs: Arc<routing::BeliefState>,
    /// Per-session FinOps spend gates (docs/design/27 Phase D), keyed by
    /// session id. Built once per session and reused for every turn: a
    /// fresh gate per turn used to zero out `max_run_usd`'s accounting on
    /// every message, so a multi-turn conversation could blow past the
    /// run cap by an arbitrary multiple. Sessions are evicted explicitly
    /// (see `Core::forget_spend_gate`) rather than left to grow forever.
    spend_gates: std::sync::Mutex<HashMap<String, Arc<finops::CoreSpendGate>>>,
    /// Shared cross-session/cross-turn day-cap admission state (see
    /// [`finops::CoreSpendGate`]'s `DayBudget` doc) — one tracker per
    /// `Core`, handed to every spend gate it builds so concurrent
    /// dispatches from different sessions can't jointly race past the
    /// day cap before any of them settles.
    day_budget: Arc<std::sync::Mutex<finops::DayBudget>>,
}

/// Learned permission rules live outside the main config so they can be
/// written at runtime without touching (possibly committed) project config.
pub const PERMISSIONS_LOCAL_FILE: &str = ".vak/permissions.local.toml";

#[derive(serde::Deserialize, Default)]
struct PermissionsLocal {
    #[serde(default)]
    allow: Vec<String>,
}

#[derive(Clone)]
pub struct Core {
    inner: Arc<CoreInner>,
    /// `<surface>:<chat>` for the conversation this turn is running
    /// inside, when known (set by the gateway per inbound message; unset
    /// for the CLI and desktop app, which have no chat to reply into).
    /// Read once, at tool-build time, as [`tasks::TasksTool`]'s default
    /// `deliver_to` — so a task created by a prompt in that chat ("remind
    /// me every Monday at 9am") reports back into the same chat without
    /// the model having to know or guess its own channel address.
    default_deliver_to: Option<String>,
}

/// One indivisible provider/model selection. A route is always read and
/// written as a pair so session admission cannot observe a torn update.
#[derive(Debug, Clone, serde::Serialize, PartialEq, Eq)]
pub struct RouteSelection {
    pub provider: String,
    pub model: String,
    pub provider_source: String,
    pub model_source: String,
    pub revision: String,
    #[serde(skip)]
    runtime_pinned: bool,
}

fn route_selection(
    provider: String,
    model: String,
    provider_source: &str,
    model_source: &str,
    runtime_pinned: bool,
) -> RouteSelection {
    let revision = route_revision(&provider, &model, provider_source, model_source);
    RouteSelection {
        provider,
        model,
        provider_source: provider_source.to_string(),
        model_source: model_source.to_string(),
        revision,
        runtime_pinned,
    }
}

fn route_from_config(
    cwd: &std::path::Path,
    config: &vak_config::Config,
    pinned: bool,
) -> RouteSelection {
    route_selection(
        config.provider.clone(),
        config.model.clone(),
        &route_source(cwd, "provider"),
        &route_source(cwd, "model"),
        pinned,
    )
}

fn route_revision(
    provider: &str,
    model: &str,
    provider_source: &str,
    model_source: &str,
) -> String {
    let mut hash = 0xcbf29ce484222325_u64;
    for byte in [provider, model, provider_source, model_source]
        .join("\0")
        .bytes()
    {
        hash ^= u64::from(byte);
        hash = hash.wrapping_mul(0x100000001b3);
    }
    format!("r{hash:016x}")
}

fn route_source(cwd: &std::path::Path, key: &str) -> String {
    let env_set = match key {
        "provider" => std::env::var("VAK_PROVIDER").is_ok(),
        "model" => std::env::var("VAK_MODEL").is_ok(),
        _ => false,
    };
    if env_set {
        "environment"
    } else if project_profile_has_key(cwd, key) {
        "project_profile"
    } else if vak_config::project_path(cwd).is_file() && project_config_has_key(cwd, key) {
        "project_config"
    } else if global_profile_has_key(key) {
        "global_profile"
    } else if vak_config::global_path().is_some_and(|path| path.is_file())
        && global_config_has_key(key)
    {
        "global_config"
    } else {
        "built_in_default"
    }
    .to_string()
}

impl Core {
    pub fn new(cwd: PathBuf) -> Result<Self, CoreError> {
        Self::new_with_trust(cwd, true)
    }

    /// `trust_project_config == false` demotes privileged project-layer
    /// keys (permission_mode, allow, hooks, base URLs, mcp servers) so a
    /// cloned repository cannot grant itself full access, auto-approvals,
    /// or hook/base-URL redirection on first run.
    pub fn new_with_trust(cwd: PathBuf, trust_project_config: bool) -> Result<Self, CoreError> {
        let config = vak_config::load_with_trust(&cwd, trust_project_config)?;
        let route = route_from_config(&cwd, &config, false);
        // Canonical layout (doc 32): one resolver for the whole workspace.
        // The cwd fallback covers exotic environments with no HOME.
        let sessions_home = vak_config::paths::data_home();
        let sessions_home = if std::env::var_os("VAK_HOME").is_none()
            && std::env::var_os("HOME").is_none()
            && std::env::var_os("USERPROFILE").is_none()
        {
            cwd.join(".vak")
        } else {
            sessions_home
        };
        let breaker = Arc::new(vak_agent::CircuitBreaker::new(
            vak_agent::CircuitBreakerConfig {
                threshold: config.circuit_breaker_threshold,
                cooldown: std::time::Duration::from_secs(config.circuit_breaker_cooldown_secs),
            },
        ));
        let extra_allow = if trust_project_config {
            load_permissions_local(&cwd)
        } else {
            Vec::new()
        };
        Ok(Core {
            default_deliver_to: None,
            inner: Arc::new(CoreInner {
                config,
                cwd,
                sessions_home,
                registry: default_registry(),
                route: std::sync::Mutex::new(route),
                max_turns_override: std::sync::Mutex::new(None),
                max_turns_runtime_pinned: std::sync::atomic::AtomicBool::new(false),
                mode_override: std::sync::Mutex::new(None),
                mode_runtime_pinned: std::sync::atomic::AtomicBool::new(false),
                sandbox_backend_override: std::sync::Mutex::new(None),
                theme_override: std::sync::Mutex::new(None),
                theme_runtime_pinned: std::sync::atomic::AtomicBool::new(false),
                memory_search_enabled_override: std::sync::Mutex::new(None),
                memory_write_enabled_override: std::sync::Mutex::new(None),
                memory_reflection_override: std::sync::Mutex::new(None),
                memory_skill_proposals_override: std::sync::Mutex::new(None),
                subagents_override: std::sync::Mutex::new(None),
                finops_max_run_usd_override: std::sync::Mutex::new(None),
                finops_max_day_usd_override: std::sync::Mutex::new(None),
                provider_instance: std::sync::Mutex::new(None),
                sessions_home_override: std::sync::Mutex::new(None),
                breaker,
                subagents: Arc::new(vak_agent::SubagentRegistry::new()),
                trust_project_config,
                extra_allow: std::sync::Mutex::new(extra_allow),
                user_env_override: std::sync::Mutex::new(None),
                tool_worker_exe: std::sync::Mutex::new(
                    std::env::current_exe()
                        .unwrap_or_else(|_| PathBuf::from("__vak_tool_worker_unavailable__")),
                ),
                models_cache: std::sync::Mutex::new(HashMap::new()),
                mcp_override: std::sync::Mutex::new(None),
                mcp_runtime_pinned: std::sync::atomic::AtomicBool::new(false),
                hooks_override: std::sync::Mutex::new(None),
                hooks_runtime_pinned: std::sync::atomic::AtomicBool::new(false),
                channel_policy: std::sync::Mutex::new(None),
                beliefs: Arc::new(routing::BeliefState::new()),
                spend_gates: std::sync::Mutex::new(HashMap::new()),
                day_budget: Arc::new(std::sync::Mutex::new(finops::DayBudget::new())),
                mcp_cache: std::sync::Mutex::new(None),
            }),
        })
    }

    pub fn config(&self) -> &vak_config::Config {
        &self.inner.config
    }

    /// Session-scoped routing beliefs (Phase R): domain-weighted doubt
    /// that demotes flaky legs until one success clears them.
    pub fn beliefs(&self) -> &Arc<routing::BeliefState> {
        &self.inner.beliefs
    }

    /// Read a session-scoped override, releasing the lock before returning.
    ///
    /// Every `Option`-shaped override on `Inner` is read through here. The
    /// idiom this replaces — `if let Ok(g) = self.inner.slot.lock() && …`
    /// — keeps the guard alive for the whole body, so a `self.` call
    /// inside that body which locks the same slot deadlocks the thread:
    /// `std::sync::Mutex` is not reentrant. `cache_home` did exactly that
    /// against `sessions_home`, wedging any process that set the override.
    /// Cloning out under a minimal scope makes the hazard unreachable.
    fn read_override<T: Clone>(slot: &std::sync::Mutex<Option<T>>) -> Option<T> {
        slot.lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .clone()
    }

    /// Set a session-scoped override. Poisoning is recovered rather than
    /// propagated: an override is a preference, and losing one to an
    /// unrelated panic elsewhere should not take down this call.
    fn write_override<T>(slot: &std::sync::Mutex<Option<T>>, value: Option<T>) {
        *slot
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner) = value;
    }

    pub fn set_model(&self, model: String) {
        let route = self.effective_route();
        self.set_route(route.provider, model);
    }

    pub fn effective_model(&self) -> String {
        self.effective_route().model
    }

    pub fn model_source(&self) -> String {
        self.effective_route().model_source
    }

    pub fn set_provider(&self, provider: String) {
        let route = self.effective_route();
        self.set_route(provider, route.model);
    }

    /// Apply an explicit scoped route override atomically. CLI flags, task
    /// pins, heartbeat pins, and test seams use this path; persisted admin
    /// changes use `apply_persisted_route` instead.
    pub fn set_route(&self, provider: String, model: String) {
        self.replace_route(route_selection(
            provider,
            model,
            "runtime_override",
            "runtime_override",
            true,
        ));
    }

    /// Re-read the layered provider/model defaults. Runtime-pinned cores are
    /// deliberately excluded so a global admin edit cannot rewrite a scoped
    /// CLI, task, heartbeat, or subagent contract.
    pub fn refresh_persisted_route(&self) -> Result<RouteSelection, CoreError> {
        let current = self.effective_route();
        if current.runtime_pinned {
            return Ok(current);
        }
        let config = vak_config::load_with_trust(&self.inner.cwd, self.inner.trust_project_config)?;
        let route = route_from_config(&self.inner.cwd, &config, false);
        if route != current {
            self.replace_route(route.clone());
        }
        Ok(route)
    }

    /// Hot-apply a route that has already been committed atomically to the
    /// workspace config by the authenticated administration surface.
    pub fn apply_persisted_route(&self, provider: String, model: String) {
        let provider_source = route_source(&self.inner.cwd, "provider");
        let model_source = route_source(&self.inner.cwd, "model");
        self.replace_route(route_selection(
            provider,
            model,
            &provider_source,
            &model_source,
            false,
        ));
    }

    pub fn effective_route(&self) -> RouteSelection {
        self.inner
            .route
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .clone()
    }

    fn replace_route(&self, route: RouteSelection) {
        let provider = route.provider.clone();
        *self
            .inner
            .route
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner) = route;
        if let Ok(mut injected) = self.inner.provider_instance.lock()
            && injected
                .as_ref()
                .is_some_and(|current| current.name() != provider)
        {
            *injected = None;
        }
    }

    pub fn effective_provider(&self) -> String {
        self.effective_route().provider
    }

    pub fn provider_source(&self) -> String {
        self.effective_route().provider_source
    }

    /// Whether project-owned privileged configuration was admitted when this
    /// core was created. Presentation files use the same trust boundary.
    pub fn project_config_trusted(&self) -> bool {
        self.inner.trust_project_config
    }

    pub fn provider_names(&self) -> Vec<String> {
        self.inner.registry.names()
    }

    pub fn set_max_turns(&self, max_turns: usize) {
        self.inner
            .max_turns_runtime_pinned
            .store(true, std::sync::atomic::Ordering::Release);
        if let Ok(mut c) = self.inner.max_turns_override.lock() {
            *c = Some(max_turns);
        }
    }

    pub fn apply_persisted_max_turns(&self, max_turns: usize) {
        Self::write_override(&self.inner.max_turns_override, Some(max_turns));
        self.inner
            .max_turns_runtime_pinned
            .store(false, std::sync::atomic::Ordering::Release);
    }

    pub fn set_tool_worker_exe(&self, executable: PathBuf) {
        if let Ok(mut worker) = self.inner.tool_worker_exe.lock() {
            *worker = executable;
        }
    }

    pub fn agent_tools(&self) -> Vec<Arc<dyn vak_tools::Tool>> {
        let worker = self
            .inner
            .tool_worker_exe
            .lock()
            .ok()
            .map(|worker| worker.clone())
            .unwrap_or_else(|| PathBuf::from("__vak_tool_worker_unavailable__"));
        let tools = vak_tools::brokered_default_tools(worker);
        self.filter_builtin_tools(tools)
    }

    pub fn agent_read_only_tools(&self) -> Vec<Arc<dyn vak_tools::Tool>> {
        let worker = self
            .inner
            .tool_worker_exe
            .lock()
            .ok()
            .map(|worker| worker.clone())
            .unwrap_or_else(|| PathBuf::from("__vak_tool_worker_unavailable__"));
        let tools = vak_tools::brokered_read_only_tools(worker);
        self.filter_builtin_tools(tools)
    }

    fn filter_builtin_tools(
        &self,
        tools: Vec<Arc<dyn vak_tools::Tool>>,
    ) -> Vec<Arc<dyn vak_tools::Tool>> {
        let Some(policy) = self.channel_policy() else {
            return tools;
        };
        tools
            .into_iter()
            .filter(|tool| Self::allowed_by(&policy.tools_allow, &policy.tools_deny, tool.name()))
            .collect()
    }

    pub fn agent_sandbox(&self) -> Option<Arc<dyn vak_tools::sandbox::Sandbox>> {
        self.build_sandbox()
    }

    pub fn effective_max_turns(&self) -> usize {
        Self::read_override(&self.inner.max_turns_override).unwrap_or(self.inner.config.max_turns)
    }

    pub fn set_permission_mode(&self, mode: vak_config::PermissionMode) {
        self.inner
            .mode_runtime_pinned
            .store(true, std::sync::atomic::Ordering::Release);
        Self::write_override(&self.inner.mode_override, Some(mode));
    }

    pub fn apply_persisted_permission_mode(&self, mode: vak_config::PermissionMode) {
        Self::write_override(&self.inner.mode_override, Some(mode));
        self.inner
            .mode_runtime_pinned
            .store(false, std::sync::atomic::Ordering::Release);
    }

    pub fn permission_mode_runtime_pinned(&self) -> bool {
        self.inner
            .mode_runtime_pinned
            .load(std::sync::atomic::Ordering::Acquire)
    }

    /// Runtime sandbox-backend selection ("os", "docker", or config default
    /// via None). Session-scoped like every other override; never persisted.
    pub fn set_sandbox_backend(&self, backend: Option<String>) {
        Self::write_override(&self.inner.sandbox_backend_override, backend);
    }

    pub fn effective_sandbox_backend(&self) -> String {
        Self::read_override(&self.inner.sandbox_backend_override)
            .unwrap_or_else(|| self.inner.config.sandbox.backend.clone())
    }

    pub fn breaker(&self) -> Arc<vak_agent::CircuitBreaker> {
        self.inner.breaker.clone()
    }

    /// Runtime MCP server table replacement (trusted surfaces only). Takes
    /// effect on the next turn; the cached capability section is dropped so
    /// the next run re-discovers the new inventory.
    pub fn set_mcp_servers(&self, config: vak_config::McpConfig) {
        self.inner
            .mcp_runtime_pinned
            .store(true, std::sync::atomic::Ordering::Release);
        self.replace_mcp(config);
    }

    pub fn apply_persisted_mcp_servers(&self, config: vak_config::McpConfig) {
        self.replace_mcp(config);
        self.inner
            .mcp_runtime_pinned
            .store(false, std::sync::atomic::Ordering::Release);
    }

    fn replace_mcp(&self, config: vak_config::McpConfig) {
        if let Ok(mut c) = self.inner.mcp_override.lock() {
            *c = Some(config);
        }
    }

    pub fn effective_mcp(&self) -> vak_config::McpConfig {
        let mut config = self
            .inner
            .mcp_override
            .lock()
            .ok()
            .and_then(|c| c.clone())
            .unwrap_or_else(|| self.inner.config.mcp.clone());
        self.extend_enabled_plugin_mcp(&mut config);
        self.filter_mcp(config)
    }

    fn extend_enabled_plugin_mcp(&self, config: &mut vak_config::McpConfig) {
        for root in [
            self.inner.cwd.join(".vak"),
            self.inner.sessions_home.clone(),
        ] {
            let Ok(plugins) = vak_plugin::PluginStore::new(root).enabled() else {
                continue;
            };
            for plugin in plugins {
                for relative in plugin.capabilities.mcp_manifests {
                    let path = plugin.package_path.join(&relative);
                    let Ok(bytes) = std::fs::read(&path) else {
                        continue;
                    };
                    let Ok(value) = serde_json::from_slice::<serde_json::Value>(&bytes) else {
                        continue;
                    };
                    let Some(servers) = value
                        .get("mcpServers")
                        .and_then(serde_json::Value::as_object)
                    else {
                        continue;
                    };
                    for (name, raw) in servers {
                        let Some(command) = raw.get("command").and_then(serde_json::Value::as_str)
                        else {
                            continue;
                        };
                        let args = raw
                            .get("args")
                            .and_then(serde_json::Value::as_array)
                            .map(|values| {
                                values
                                    .iter()
                                    .filter_map(serde_json::Value::as_str)
                                    .map(str::to_string)
                                    .collect()
                            })
                            .unwrap_or_default();
                        let env = raw
                            .get("env")
                            .and_then(serde_json::Value::as_object)
                            .map(|values| {
                                values
                                    .iter()
                                    .filter_map(|(key, value)| {
                                        value.as_str().map(|v| (key.clone(), v.to_string()))
                                    })
                                    .collect()
                            })
                            .unwrap_or_default();
                        let key = format!("plugin.{}.{}", plugin.name, name);
                        config
                            .servers
                            .entry(key)
                            .or_insert(vak_config::McpServerConfig {
                                command: command.to_string(),
                                args,
                                env,
                                network: false,
                            });
                    }
                }
            }
        }
    }

    fn plugin_mcp_invocation_context(&self) -> Vec<(vak_plugin::PluginStore, String, String)> {
        let mut context = Vec::new();
        for root in [
            self.inner.cwd.join(".vak"),
            self.inner.sessions_home.clone(),
        ] {
            let store = vak_plugin::PluginStore::new(root);
            let Ok(plugins) = store.enabled() else {
                continue;
            };
            context.extend(
                plugins
                    .into_iter()
                    .map(|plugin| (store.clone(), plugin.name, plugin.trace_id)),
            );
        }
        context
    }

    pub fn apply_channel_policy(&self, policy: vak_config::ChannelPolicy) {
        if let Ok(mut current) = self.inner.channel_policy.lock() {
            *current = Some(policy);
        }
    }

    pub fn channel_policy(&self) -> Option<vak_config::ChannelPolicy> {
        self.inner
            .channel_policy
            .lock()
            .ok()
            .and_then(|p| p.clone())
    }

    /// Whether a channel overlay permits a named capability. Inheritance is
    /// represented by no policy and therefore permits the capability here;
    /// the ordinary permission engine still decides whether execution is
    /// allowed for the current mode.
    pub fn channel_tool_allowed(&self, tool: &str) -> bool {
        self.channel_policy()
            .is_none_or(|policy| Self::allowed_by(&policy.tools_allow, &policy.tools_deny, tool))
    }

    pub fn memory_write_allowed(&self) -> bool {
        if !self.channel_tool_allowed("remember") {
            return false;
        }
        let mut rules = self.extra_allow_snapshot();
        if let Some(policy) = self.channel_policy() {
            if let Some(allow) = policy.tools_allow {
                if allow.is_empty() {
                    rules.extend(
                        [
                            "read",
                            "write",
                            "edit",
                            "bash",
                            "glob",
                            "grep",
                            "remember",
                            "propose_skill",
                        ]
                        .into_iter()
                        .map(|tool| format!("-{tool}")),
                    );
                } else {
                    rules.extend(allow.into_iter().map(|pattern| format!("+{pattern}")));
                }
            }
            rules.extend(
                policy
                    .tools_deny
                    .into_iter()
                    .map(|pattern| format!("-{pattern}")),
            );
        }
        let Ok(engine) =
            build_engine_for_mode(&self.inner.config, &rules, self.effective_permission_mode())
        else {
            return false;
        };
        let mode = match self.effective_permission_mode() {
            vak_config::PermissionMode::ReadOnly => vak_permission::Mode::ReadOnly,
            vak_config::PermissionMode::WorkspaceWrite => vak_permission::Mode::WorkspaceWrite,
            vak_config::PermissionMode::FullAccess => vak_permission::Mode::FullAccess,
        };
        matches!(
            engine.evaluate("remember", &serde_json::json!({}), mode, &self.inner.cwd),
            vak_permission::Decision::Allow
        )
    }

    fn policy_matches(patterns: &[String], value: &str) -> bool {
        patterns.iter().any(|pattern| {
            globset::Glob::new(pattern)
                .ok()
                .is_some_and(|glob| glob.compile_matcher().is_match(value))
        })
    }

    fn allowed_by(allow: &Option<Vec<String>>, deny: &[String], value: &str) -> bool {
        !Self::policy_matches(deny, value)
            && allow
                .as_ref()
                .is_none_or(|patterns| Self::policy_matches(patterns, value))
    }

    fn filter_mcp(&self, mut config: vak_config::McpConfig) -> vak_config::McpConfig {
        let Some(policy) = self.channel_policy() else {
            return config;
        };
        config.servers.retain(|name, _| {
            let server_pattern = format!("{name}/*");
            if Self::policy_matches(&policy.mcp_deny, &server_pattern) {
                return false;
            }
            policy.mcp_allow.as_ref().is_none_or(|allow| {
                Self::policy_matches(allow, &server_pattern)
                    || allow
                        .iter()
                        .any(|pattern| pattern.starts_with(&format!("{name}/")))
            })
        });
        // Restrictive only: a channel can force a server's network off, but
        // there is no matching grant — a server the config itself denies
        // network to stays denied no matter what a channel policy says.
        // Same pattern shape as mcp_allow/mcp_deny above (`name/*`).
        for (name, server) in config.servers.iter_mut() {
            let server_pattern = format!("{name}/*");
            if server.network && Self::policy_matches(&policy.mcp_network_deny, &server_pattern) {
                server.network = false;
            }
        }
        config
    }

    /// Resolve `effective_mcp()` into the shape `vak_mcp::McpManager` wants:
    /// `${VAR}` in env values expanded through the standard secret path,
    /// fail-closed per server (an unresolved reference drops that server
    /// rather than starting it half-configured). Shared by the per-turn
    /// tool list and `mcp_manager()` so the two never resolve servers two
    /// different ways.
    fn resolved_mcp_servers(&self) -> Vec<(String, vak_mcp::ServerConfig)> {
        self.effective_mcp()
            .servers
            .into_iter()
            .filter_map(|(name, s)| {
                let mut env = Vec::with_capacity(s.env.len());
                for (k, v) in &s.env {
                    match interpolate_env_var(v) {
                        Some(resolved) => env.push((k.clone(), resolved)),
                        None => {
                            eprintln!("[mcp] server '{name}' skipped: unresolved environment variable in '{v}' (define it in .env)");
                            return None;
                        }
                    }
                }
                Some((
                    name,
                    vak_mcp::ServerConfig {
                        command: s.command,
                        args: s.args,
                        env,
                        network: s.network,
                    },
                ))
            })
            .collect()
    }

    /// Long-lived `McpManager` for this Core, reused across turns instead
    /// of rebuilt per turn — `McpManager::get` caches one live connection
    /// per server, so a manager rebuilt every turn meant every turn that
    /// touched MCP respawned every configured server's process from
    /// scratch. Returns `None` when no servers are configured.
    ///
    /// Keyed by a fingerprint of the resolved server set: a runtime
    /// `set_mcp_servers` call or a plugin being enabled/disabled changes
    /// what `effective_mcp()` returns, and the fingerprint mismatch swaps
    /// in a fresh manager (dropping the old one, which shuts its clients
    /// down on drop) rather than serving stale servers indefinitely.
    ///
    /// Turn admission still never blocks on an optional integration: this
    /// only constructs the manager (no I/O — `ServerConfig` is inert until
    /// something calls `.get()` on it) and fires a best-effort background
    /// warm-up of the tool inventory that `system_prompt()` picks up once
    /// it lands.
    fn mcp_manager(&self) -> Option<Arc<vak_mcp::McpManager>> {
        let servers = self.resolved_mcp_servers();
        if servers.is_empty() {
            *self
                .inner
                .mcp_cache
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner) = None;
            return None;
        }
        let fp = mcp_fingerprint(&servers);
        {
            let cache = self
                .inner
                .mcp_cache
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            if let Some(c) = cache.as_ref()
                && c.fingerprint == fp
            {
                return Some(c.manager.clone());
            }
        }
        let manager = Arc::new(vak_mcp::McpManager::new_sandboxed(
            servers.into_iter().collect(),
            self.inner.cwd.clone(),
            self.build_sandbox(),
        ));
        {
            let mut cache = self
                .inner
                .mcp_cache
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            *cache = Some(McpCache {
                fingerprint: fp,
                manager: manager.clone(),
                inventory: None,
                warming: false,
            });
        }
        self.spawn_mcp_inventory_warm(fp, manager.clone());
        Some(manager)
    }

    /// Kick a background discovery pass for `system_prompt()`'s rich MCP
    /// section, if one for this exact server set (`fp`) isn't already
    /// running or done. Best-effort: a slow or failing server degrades to
    /// a per-server error line inside the inventory (see
    /// `McpManager::inventory`), never to a panic or a blocked turn.
    fn spawn_mcp_inventory_warm(&self, fp: u64, manager: Arc<vak_mcp::McpManager>) {
        {
            let mut cache = self
                .inner
                .mcp_cache
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            match cache.as_mut() {
                Some(c) if c.fingerprint == fp && !c.warming && c.inventory.is_none() => {
                    c.warming = true;
                }
                _ => return,
            }
        }
        let core = self.clone();
        tokio::spawn(async move {
            let inventory = manager.inventory().await;
            let mut cache = core
                .inner
                .mcp_cache
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            if let Some(c) = cache.as_mut()
                && c.fingerprint == fp
            {
                c.inventory = Some(inventory);
                c.warming = false;
            }
        });
    }

    /// Whatever the background warm-up has landed so far, for
    /// `system_prompt()`. `None` means "not ready yet" (or discovery
    /// hasn't been triggered this session) — callers fall back to the
    /// name-only `mcp_config_section`, never block waiting for this.
    fn cached_mcp_inventory(&self) -> Option<McpInventory> {
        self.inner
            .mcp_cache
            .lock()
            .ok()?
            .as_ref()
            .and_then(|c| c.inventory.clone())
    }

    /// Replace lifecycle hooks for subsequent turns without restarting the
    /// desktop/server process. Persistence is owned by the server surface.
    pub fn set_hooks(&self, hooks: Vec<vak_config::HookConfig>) {
        self.inner
            .hooks_runtime_pinned
            .store(true, std::sync::atomic::Ordering::Release);
        self.replace_hooks(hooks);
    }

    pub fn apply_persisted_hooks(&self, hooks: Vec<vak_config::HookConfig>) {
        self.replace_hooks(hooks);
        self.inner
            .hooks_runtime_pinned
            .store(false, std::sync::atomic::Ordering::Release);
    }

    fn replace_hooks(&self, hooks: Vec<vak_config::HookConfig>) {
        if let Ok(mut current) = self.inner.hooks_override.lock() {
            *current = Some(hooks);
        }
    }

    pub fn effective_hooks(&self) -> Vec<vak_config::HookConfig> {
        let hooks = if let Ok(current) = self.inner.hooks_override.lock()
            && let Some(hooks) = current.as_ref()
        {
            hooks.clone()
        } else {
            self.inner.config.hooks.clone()
        };
        let mut hooks = hooks;
        for store_home in [self.sessions_home(), self.inner.cwd.join(".vak")] {
            let store = vak_plugin::PluginStore::new(store_home);
            if let Ok(plugin_hooks) = store.enabled_hooks() {
                hooks.extend(plugin_hooks.into_iter().map(|(_plugin, hook)| {
                    vak_config::HookConfig {
                        event: hook.event,
                        matcher: hook.matcher,
                        command: hook.command,
                        timeout_ms: hook.timeout_ms,
                        enabled: true,
                    }
                }));
            }
        }
        let Some(policy) = self.channel_policy() else {
            return hooks;
        };
        hooks
            .into_iter()
            .filter(|hook| {
                let identity = format!("{}/{}", hook.event, hook.command);
                Self::allowed_by(&policy.hooks_allow, &policy.hooks_deny, &identity)
            })
            .collect()
    }

    pub fn set_theme(&self, theme: String) {
        self.inner
            .theme_runtime_pinned
            .store(true, std::sync::atomic::Ordering::Release);
        if let Ok(mut t) = self.inner.theme_override.lock() {
            *t = Some(theme);
        }
    }

    pub fn apply_persisted_theme(&self, theme: String) {
        Self::write_override(&self.inner.theme_override, Some(theme));
        self.inner
            .theme_runtime_pinned
            .store(false, std::sync::atomic::Ordering::Release);
    }

    /// Live-effective `[memory]` toggles (docs/design/23-memory.md): a
    /// PATCH-applied override when one has been set this process's
    /// lifetime, else whatever was persisted at construction/last refresh.
    /// Every tool-registration and reflection call site must read through
    /// these, never `self.inner.config.memory.*` directly, or a live PATCH
    /// would silently do nothing until the process restarts.
    pub fn effective_memory_search_enabled(&self) -> bool {
        Self::read_override(&self.inner.memory_search_enabled_override)
            .unwrap_or(self.inner.config.memory.search_enabled)
    }

    pub fn effective_memory_write_enabled(&self) -> bool {
        Self::read_override(&self.inner.memory_write_enabled_override)
            .unwrap_or(self.inner.config.memory.write_enabled)
    }

    pub fn effective_memory_reflection(&self) -> bool {
        Self::read_override(&self.inner.memory_reflection_override)
            .unwrap_or(self.inner.config.memory.reflection)
    }

    pub fn effective_memory_skill_proposals(&self) -> bool {
        Self::read_override(&self.inner.memory_skill_proposals_override)
            .unwrap_or(self.inner.config.memory.skill_proposals)
    }

    /// Set the live `[memory]` overrides all at once — used both by the
    /// admin/desktop PATCH handler applying an explicit change and by
    /// `refresh_persisted_preferences` picking up a value another process
    /// wrote to disk.
    pub fn apply_persisted_memory(
        &self,
        search_enabled: bool,
        write_enabled: bool,
        reflection: bool,
        skill_proposals: bool,
    ) {
        Self::write_override(
            &self.inner.memory_search_enabled_override,
            Some(search_enabled),
        );
        Self::write_override(
            &self.inner.memory_write_enabled_override,
            Some(write_enabled),
        );
        Self::write_override(&self.inner.memory_reflection_override, Some(reflection));
        Self::write_override(
            &self.inner.memory_skill_proposals_override,
            Some(skill_proposals),
        );
    }

    /// Whether sub-agent delegation (the `task` tool) is available right
    /// now — live-effective, same shape as the memory accessors above.
    pub fn effective_subagents(&self) -> bool {
        Self::read_override(&self.inner.subagents_override).unwrap_or(self.inner.config.subagents)
    }

    pub fn apply_persisted_subagents(&self, enabled: bool) {
        Self::write_override(&self.inner.subagents_override, Some(enabled));
    }

    /// The `[finops]` config, with any live cap override substituted in —
    /// pass this to [`finops::CoreSpendGate::new`] instead of
    /// `self.config().finops` directly, or a PATCH-set cap would never
    /// actually bind.
    pub fn effective_finops(&self) -> vak_config::FinopsResolved {
        let mut finops = self.inner.config.finops.clone();
        if let Some(run) = Self::read_override(&self.inner.finops_max_run_usd_override) {
            finops.max_run_usd = run;
        }
        if let Some(day) = Self::read_override(&self.inner.finops_max_day_usd_override) {
            finops.max_day_usd = day;
        }
        finops
    }

    /// The spend gate for `session_id`, built once and reused for every
    /// subsequent turn of that session (docs/design/27 Phase D). Rebuilding
    /// a fresh gate per turn used to reset `max_run_usd`'s in-memory spend
    /// counter to zero on every message — this cache is what makes the run
    /// cap actually span the whole run rather than a single turn. Live cap
    /// edits (`PATCH /finops`) are picked up on the next call via
    /// `refresh_caps`, without disturbing spend already tallied.
    fn spend_gate_for(&self, session_id: &str) -> Arc<finops::CoreSpendGate> {
        let finops = self.effective_finops();
        let mut gates = self
            .inner
            .spend_gates
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if let Some(gate) = gates.get(session_id) {
            gate.refresh_caps(&finops);
            return gate.clone();
        }
        let gate = Arc::new(finops::CoreSpendGate::with_shared_day_budget(
            // `self.sessions_home()`, not the raw `inner.sessions_home`
            // field — the latter ignores `set_sessions_home` (the SDK
            // seam tests/embedded runtimes use to relocate storage), so
            // the ledger would silently keep writing to the original
            // location. The reflection call site already got this right;
            // the per-turn call site this replaces did not.
            &self.sessions_home(),
            &finops,
            self.inner.day_budget.clone(),
        ));
        gates.insert(session_id.to_string(), gate.clone());
        gate
    }

    /// Drop a session's spend gate (and the run-spend it was tracking)
    /// once the session is done, so `spend_gates` doesn't grow forever
    /// across the lifetime of a long-running process. Safe to call even
    /// when no gate was ever built for `session_id`.
    pub fn forget_spend_gate(&self, session_id: &str) {
        self.inner
            .spend_gates
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .remove(session_id);
    }

    pub fn effective_finops_max_run_usd(&self) -> Option<f64> {
        Self::read_override(&self.inner.finops_max_run_usd_override)
            .unwrap_or(self.inner.config.finops.max_run_usd)
    }

    pub fn effective_finops_max_day_usd(&self) -> Option<f64> {
        Self::read_override(&self.inner.finops_max_day_usd_override)
            .unwrap_or(self.inner.config.finops.max_day_usd)
    }

    /// `None` for either cap leaves it at its current effective value —
    /// same "absent means don't touch" convention `apply_persisted_memory`
    /// uses, except here the value being set/kept is itself an
    /// `Option<f64>` (a cap can legitimately be cleared to "none").
    pub fn apply_persisted_finops_caps(
        &self,
        max_run_usd: Option<Option<f64>>,
        max_day_usd: Option<Option<f64>>,
    ) {
        if let Some(run) = max_run_usd {
            Self::write_override(&self.inner.finops_max_run_usd_override, Some(run));
        }
        if let Some(day) = max_day_usd {
            Self::write_override(&self.inner.finops_max_day_usd_override, Some(day));
        }
    }

    /// Refresh every non-security persisted preference. Permission mode is
    /// returned to the server control plane so it can revoke in-flight
    /// capabilities before applying a changed value.
    pub fn refresh_persisted_preferences(&self) -> Result<vak_config::PermissionMode, CoreError> {
        let config = vak_config::load_with_trust(&self.inner.cwd, self.inner.trust_project_config)?;
        let current_route = self.effective_route();
        if !current_route.runtime_pinned {
            let route = route_from_config(&self.inner.cwd, &config, false);
            if route != current_route {
                self.replace_route(route);
            }
        }
        if !self
            .inner
            .max_turns_runtime_pinned
            .load(std::sync::atomic::Ordering::Acquire)
        {
            self.apply_persisted_max_turns(config.max_turns);
        }
        if !self
            .inner
            .theme_runtime_pinned
            .load(std::sync::atomic::Ordering::Acquire)
        {
            self.apply_persisted_theme(config.ui.theme.clone());
        }
        if !self
            .inner
            .mcp_runtime_pinned
            .load(std::sync::atomic::Ordering::Acquire)
        {
            self.apply_persisted_mcp_servers(config.mcp.clone());
        }
        if !self
            .inner
            .hooks_runtime_pinned
            .load(std::sync::atomic::Ordering::Acquire)
        {
            self.apply_persisted_hooks(config.hooks.clone());
        }
        self.apply_persisted_memory(
            config.memory.search_enabled,
            config.memory.write_enabled,
            config.memory.reflection,
            config.memory.skill_proposals,
        );
        self.apply_persisted_subagents(config.subagents);
        self.apply_persisted_finops_caps(
            Some(config.finops.max_run_usd),
            Some(config.finops.max_day_usd),
        );
        Ok(config.permission_mode)
    }

    /// Persists a learned allow rule to `.vak/permissions.local.toml`
    /// (and this process's in-memory engine inputs). Trusted workspaces only:
    /// an untrusted session must not be able to write grant files. Rules are
    /// severity-aggregated by the engine, so a learned Allow can never
    /// shadow an explicit Deny from any config layer.
    pub fn learn_allow_rule(&self, spec: &str) -> Result<(), CoreError> {
        vak_permission::Rule::parse(spec).map_err(CoreError::Rule)?;
        if !self.inner.trust_project_config {
            return Err(CoreError::Config(vak_config::ConfigError::Read {
                path: std::path::PathBuf::from(PERMISSIONS_LOCAL_FILE),
                source: std::io::Error::other(
                    "untrusted workspace: refusing to persist permission rules",
                ),
            }));
        }
        let path = self.inner.cwd.join(PERMISSIONS_LOCAL_FILE);
        let mut rules = load_permissions_local(&self.inner.cwd);
        if !rules.iter().any(|r| r == spec) {
            rules.push(spec.to_string());
        }
        self.write_permissions_local(&path, &rules)?;
        if let Ok(mut extra) = self.inner.extra_allow.lock() {
            *extra = rules;
        }
        Ok(())
    }

    fn write_permissions_local(
        &self,
        path: &std::path::Path,
        rules: &[String],
    ) -> Result<(), CoreError> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)
                .map_err(|e| CoreError::Session(vak_session::SessionError::Io(e)))?;
        }
        let mut body = String::from(
            "# Learned 'always allow' rules — written when you press [p] on an approval.\nallow = [\n",
        );
        for r in rules {
            body.push_str(&format!("  \"{r}\",\n"));
        }
        body.push_str("]\n");
        let tmp = path.with_extension("toml.tmp");
        std::fs::write(&tmp, body)
            .and_then(|_| std::fs::rename(&tmp, path))
            .map_err(|e| CoreError::Session(vak_session::SessionError::Io(e)))?;
        Ok(())
    }

    pub fn extra_allow_snapshot(&self) -> Vec<String> {
        self.inner
            .extra_allow
            .lock()
            .ok()
            .map(|e| e.clone())
            .unwrap_or_default()
    }

    pub fn effective_theme(&self) -> String {
        Self::read_override(&self.inner.theme_override)
            .unwrap_or_else(|| self.inner.config.ui.theme.clone())
    }

    pub fn effective_permission_mode(&self) -> vak_config::PermissionMode {
        Self::read_override(&self.inner.mode_override).unwrap_or(self.inner.config.permission_mode)
    }

    pub fn cwd(&self) -> &PathBuf {
        &self.inner.cwd
    }

    /// Bind this turn to the chat it's replying into, as `<surface>:<chat>`
    /// (the same shape `deliver_to` already uses everywhere). Cheap: an
    /// `Arc` bump plus one `String`, so callers can clone-and-set per
    /// inbound message without touching the shared workspace state the
    /// `Arc<CoreInner>` carries.
    pub fn with_default_deliver_to(mut self, target: Option<String>) -> Self {
        self.default_deliver_to = target;
        self
    }

    /// Reopens an existing session ledger for resumed runs.
    pub async fn open_session(&self, session_id: &str) -> Result<SessionLog, CoreError> {
        let path = vak_session::SessionPath::new_session_file(
            &self.sessions_home(),
            &self.inner.cwd,
            session_id,
        );
        Ok(SessionLog::open(path)?)
    }

    /// SDK seam: relocate session storage (tests, embedded runtimes).
    pub fn set_sessions_home(&self, path: PathBuf) {
        Self::write_override(&self.inner.sessions_home_override, Some(path));
    }

    /// Redirects the user-level secret store (tests, portable installs).
    pub fn set_user_env_path(&self, path: PathBuf) {
        Self::write_override(&self.inner.user_env_override, Some(path));
    }

    fn user_env_file(&self) -> PathBuf {
        Self::read_override(&self.inner.user_env_override)
            .or_else(vak_config::user_env_path)
            .unwrap_or_else(|| self.sessions_home().join(".env"))
    }

    /// SDK seam: inject a provider directly (tests, embedded runtimes).
    pub fn set_provider_instance(&self, provider: Arc<dyn Provider>) {
        if let Ok(mut p) = self.inner.provider_instance.lock() {
            *p = Some(provider);
        }
    }

    pub fn sessions_home(&self) -> PathBuf {
        Self::read_override(&self.inner.sessions_home_override)
            .unwrap_or_else(|| self.inner.sessions_home.clone())
    }

    /// Rebuildable-artifact directory (SQLite FTS index + WAL sidecars).
    /// Canonical layout (doc 32): Library/Caches on macOS, XDG cache on
    /// Linux — deleting it must always be safe.
    pub fn cache_home(&self) -> PathBuf {
        // Overridden homes are self-contained sandboxes, so the cache
        // lives inside them. Resolved from the cloned override rather
        // than by calling `sessions_home()` under the guard, which
        // re-locked the same non-reentrant mutex and hung the thread.
        match Self::read_override(&self.inner.sessions_home_override) {
            Some(home) => home.join("cache"),
            None => vak_config::paths::cache_home(),
        }
    }

    pub fn system_prompt(&self) -> String {
        let project_prompt = self.inner.cwd.join(".vak/SYSTEM.md");
        let base = if project_prompt.is_file()
            && let Ok(custom) = std::fs::read_to_string(&project_prompt)
        {
            custom
        } else {
            DEFAULT_SYSTEM_PROMPT.replace("{{version}}", APP_VERSION)
        };
        let discovered = self.skills();
        // Ensure a manager exists and a background discovery pass is (or
        // becomes) in flight for the current server set, then use whatever
        // it has produced so far. A cold session or a slow/failing server
        // means `cached_mcp_inventory()` is still `None` here — that's the
        // expected steady state for the very first turn, and this call
        // never blocks waiting for it to fill in.
        self.mcp_manager();
        let mcp_section_text = match self.cached_mcp_inventory() {
            Some(inventory) if !inventory.is_empty() => mcp_section(&inventory),
            _ => {
                let servers = self.effective_mcp().servers.into_keys().collect::<Vec<_>>();
                mcp_config_section(&servers)
            }
        };
        format!(
            "{}{}{}",
            base,
            skills::prompt_section(&discovered),
            mcp_section_text
        )
    }

    pub fn skills(&self) -> Vec<skills::Skill> {
        let mut plugin_roots = Vec::new();
        for (root, label) in [
            (self.inner.cwd.join(".vak"), "workspace".to_string()),
            (self.inner.sessions_home.clone(), "user".to_string()),
        ] {
            if let Ok(enabled) = vak_plugin::PluginStore::new(&root).enabled() {
                plugin_roots.extend(enabled.into_iter().map(|plugin| {
                    (
                        plugin.package_path,
                        format!("plugin:{}:{}:{}", label, plugin.name, plugin.trace_id),
                    )
                }));
            }
        }
        let skills = skills::discover_with_plugins(
            &self.inner.cwd,
            &self.inner.sessions_home,
            &plugin_roots,
        );
        let Some(policy) = self.channel_policy() else {
            return skills;
        };
        skills
            .into_iter()
            .filter(|skill| {
                Self::allowed_by(&policy.skills_allow, &policy.skills_deny, &skill.name)
            })
            .collect()
    }

    /// Live subagents spawned by this Core's runs, for attach/steer UIs.
    pub fn subagents(&self) -> Arc<vak_agent::SubagentRegistry> {
        self.inner.subagents.clone()
    }

    pub fn custom_commands(&self) -> Vec<custom_commands::CustomCommand> {
        let mut plugin_roots = Vec::new();
        for (root, label) in [
            (self.inner.cwd.join(".vak"), "workspace".to_string()),
            (self.inner.sessions_home.clone(), "user".to_string()),
        ] {
            if let Ok(enabled) = vak_plugin::PluginStore::new(&root).enabled() {
                plugin_roots.extend(enabled.into_iter().map(|plugin| {
                    (
                        plugin.package_path,
                        format!("plugin:{}:{}:{}", label, plugin.name, plugin.trace_id),
                    )
                }));
            }
        }
        custom_commands::discover_with_plugins(
            &self.inner.cwd,
            &self.inner.sessions_home,
            &plugin_roots,
        )
    }

    pub fn tool_names(&self) -> Vec<String> {
        let mut names: Vec<String> = vak_tools::default_tools()
            .iter()
            .map(|t| t.name().to_string())
            .collect();
        if self.effective_subagents() {
            names.push("task".into());
        }
        if !self.effective_mcp().servers.is_empty() {
            names.push("mcp".into());
        }
        if self.inner.config.tools.web_fetch {
            names.push("webfetch".into());
        }
        if self.inner.config.tools.browse {
            names.push("browse".into());
        }

        if self.effective_memory_search_enabled() {
            names.push("session_search".into());
        }
        if self.effective_memory_write_enabled() {
            names.push("remember".into());
        }
        if self.effective_memory_skill_proposals() {
            names.push("propose_skill".into());
        }
        names
            .into_iter()
            .filter(|name| self.channel_tool_allowed(name))
            .collect()
    }

    fn provider_auth(&self) -> Result<ProviderAuth, CoreError> {
        self.provider_auth_for(&self.effective_provider())
    }

    /// Resolve credentials for an arbitrary provider, not just the active
    /// one — model discovery needs to authenticate against whichever
    /// provider the user is inspecting.
    fn provider_auth_for(&self, provider: &str) -> Result<ProviderAuth, CoreError> {
        let provider = provider.to_string();
        let required_key = |env: &str, provider: &str| {
            vak_config::get_var(env)
                .filter(|key| !key.trim().is_empty())
                .map(|key| key.trim().to_string())
                .ok_or_else(|| CoreError::MissingAuth {
                    env: env.into(),
                    provider: provider.into(),
                })
        };
        match provider.as_str() {
            "anthropic" => {
                let api_key = required_key("ANTHROPIC_API_KEY", "anthropic")?;
                Ok(ProviderAuth {
                    api_key,
                    base_url: self
                        .inner
                        .config
                        .anthropic_base_url
                        .clone()
                        .or_else(|| vak_config::get_var("VAK_ANTHROPIC_BASE_URL")),
                })
            }
            "google" => {
                let api_key = vak_config::get_var("GEMINI_API_KEY")
                    .or_else(|| vak_config::get_var("GOOGLE_API_KEY"))
                    .filter(|key| !key.trim().is_empty())
                    .map(|key| key.trim().to_string())
                    .ok_or_else(|| CoreError::MissingAuth {
                        env: "GEMINI_API_KEY".into(),
                        provider: provider.clone(),
                    })?;
                Ok(ProviderAuth {
                    api_key,
                    base_url: vak_config::get_var("VAK_GOOGLE_BASE_URL").or_else(|| {
                        Some("https://generativelanguage.googleapis.com/v1beta".into())
                    }),
                })
            }
            "openai-responses" => {
                let api_key = required_key("OPENAI_API_KEY", "openai-responses")?;
                Ok(ProviderAuth {
                    api_key,
                    base_url: vak_config::get_var("VAK_OPENAI_BASE_URL")
                        .or_else(|| Some("https://api.openai.com/v1".into())),
                })
            }
            // get_var (not raw env) so user-level and project .env files
            // authenticate these providers exactly like every other one.
            "openai" | "openrouter" => {
                let (env, default_base, override_env) = if provider == "openai" {
                    (
                        "OPENAI_API_KEY",
                        "https://api.openai.com/v1",
                        "VAK_OPENAI_BASE_URL",
                    )
                } else {
                    (
                        "OPENROUTER_API_KEY",
                        "https://openrouter.ai/api/v1",
                        "VAK_OPENROUTER_BASE_URL",
                    )
                };
                let api_key = required_key(env, &provider)?;
                Ok(ProviderAuth {
                    api_key,
                    base_url: vak_config::get_var(override_env)
                        .or_else(|| Some(default_base.into())),
                })
            }
            "opencode-zen" => {
                let api_key = required_key("OPENCODE_API_KEY", "opencode-zen")?;
                Ok(ProviderAuth {
                    api_key,
                    base_url: vak_config::get_var("VAK_OPENCODE_ZEN_BASE_URL")
                        .or_else(|| Some("https://opencode.ai/zen/v1".into())),
                })
            }
            "ollama" => Ok(ProviderAuth {
                api_key: "ollama".into(),
                base_url: vak_config::get_var("VAK_OLLAMA_BASE_URL")
                    .or_else(|| Some("http://localhost:11434/v1".into())),
            }),
            other => Err(CoreError::MissingAuth {
                env: format!("(no auth wiring for '{other}' yet)"),
                provider: other.into(),
            }),
        }
    }

    pub fn provider(&self) -> Result<Arc<dyn Provider>, CoreError> {
        if let Ok(p) = self.inner.provider_instance.lock()
            && let Some(provider) = p.as_ref()
        {
            return Ok(provider.clone());
        }
        let auth = self.provider_auth()?;
        Ok(self.inner.registry.get(&self.effective_provider(), &auth)?)
    }

    /// The env var that authenticates `provider`, or None for keyless
    /// providers (ollama). Unknown providers yield None as well — callers
    /// distinguish via `provider_known`.
    pub fn provider_env_var(provider: &str) -> Option<&'static str> {
        match provider {
            "anthropic" => Some("ANTHROPIC_API_KEY"),
            "google" => Some("GEMINI_API_KEY"),
            "openai" | "openai-responses" => Some("OPENAI_API_KEY"),
            "openrouter" => Some("OPENROUTER_API_KEY"),
            "opencode-zen" => Some("OPENCODE_API_KEY"),
            _ => None,
        }
    }

    pub fn provider_known(provider: &str) -> bool {
        matches!(
            provider,
            "anthropic"
                | "google"
                | "openai"
                | "openai-responses"
                | "openrouter"
                | "opencode-zen"
                | "ollama"
        )
    }

    /// True when a run on `provider` would find credentials right now.
    pub fn provider_configured(&self, provider: &str) -> bool {
        match provider {
            "ollama" => true,
            "google" => ["GEMINI_API_KEY", "GOOGLE_API_KEY"]
                .iter()
                .any(|env| vak_config::get_var(env).is_some_and(|key| !key.trim().is_empty())),
            other => vak_config::get_var(Self::provider_env_var(other).unwrap_or(""))
                .is_some_and(|key| !key.trim().is_empty()),
        }
    }

    /// Persists the key for `provider` into the user-level
    /// `.env` at `data_home()/.env` (0600, shared by every surface) and registers it
    /// as a runtime override so the next request uses it immediately —
    /// no restart. Returns the env var that was written. The key itself
    /// never re-enters any response.
    pub fn set_provider_key(&self, provider: &str, key: &str) -> Result<String, CoreError> {
        let key = key.trim();
        if key.is_empty() {
            return Err(CoreError::InvalidConfig("empty api key".into()));
        }
        let env = Self::provider_env_var(provider).ok_or_else(|| {
            CoreError::InvalidConfig(format!(
                "unknown provider '{provider}' (or it needs no key)"
            ))
        })?;
        let path = self.user_env_file();
        vak_config::upsert_env_file(&path, env, key)
            .map_err(|e| CoreError::InvalidConfig(format!("writing {path:?}: {e}")))?;
        vak_config::set_override(env, key);
        // A different key reaches a different set of models, and any cached
        // client still holds the old credential.
        self.invalidate_models_cache(Some(provider));
        if let Ok(mut p) = self.inner.provider_instance.lock() {
            *p = None;
        }
        Ok(env.to_string())
    }

    /// Revoke `provider`'s key: strip it from the user `.env`, drop the
    /// runtime override and the loaded-dotenv copy, and forget any
    /// discovered models. A key exported in the real environment cannot be
    /// unset from here — the caller is told so it can say as much.
    pub fn remove_provider_key(&self, provider: &str) -> Result<RemovedKey, CoreError> {
        let env = Self::provider_env_var(provider).ok_or_else(|| {
            CoreError::InvalidConfig(format!(
                "unknown provider '{provider}' (or it needs no key)"
            ))
        })?;
        let path = self.user_env_file();
        vak_config::remove_env_file_key(&path, env)
            .map_err(|e| CoreError::InvalidConfig(format!("writing {path:?}: {e}")))?;
        vak_config::clear_override(env);
        vak_config::forget_dotenv_var(env);
        self.invalidate_models_cache(Some(provider));
        // Any cached client was built with the old key.
        if let Ok(mut p) = self.inner.provider_instance.lock() {
            *p = None;
        }
        Ok(RemovedKey {
            env_var: env.to_string(),
            // If it still resolves, it comes from the process environment.
            shadowed_by_env: vak_config::get_var(env).is_some(),
        })
    }

    /// Store an MCP credential in the shared user secret file and register it
    /// as a runtime override. The MCP config should contain a `${VAR}`
    /// reference, never the credential itself.
    pub fn set_mcp_secret(&self, env_var: &str, key: &str) -> Result<(), CoreError> {
        let env_var = env_var.trim();
        let key = key.trim();
        if env_var.is_empty()
            || !env_var
                .chars()
                .all(|c| c.is_ascii_uppercase() || c.is_ascii_digit() || c == '_')
            || key.is_empty()
        {
            return Err(CoreError::InvalidConfig("invalid MCP secret".into()));
        }
        let path = self.user_env_file();
        vak_config::upsert_env_file(&path, env_var, key)
            .map_err(|e| CoreError::InvalidConfig(format!("writing {path:?}: {e}")))?;
        vak_config::set_override(env_var, key);
        Ok(())
    }

    /// The env var that authenticates the Telegram bridge. Not a "provider"
    /// in the LLM sense, but stored the same way: the shared user `.env`
    /// that every surface and service loads.
    pub const TELEGRAM_TOKEN_ENV: &'static str = "TELEGRAM_BOT_TOKEN";

    /// True when a Telegram bridge launched right now would find a token.
    pub fn telegram_configured(&self) -> bool {
        vak_config::get_var(Self::TELEGRAM_TOKEN_ENV).is_some()
    }

    /// Persists the Telegram bot token into the user-level `.env`
    /// (0600, shared by every surface) and registers it as a runtime
    /// override so it's visible immediately. The launchd/systemd unit for
    /// the bridge still reads `.env` itself on (re)start — this only makes
    /// `telegram_configured()` and any in-process check correct right away.
    /// The token itself never re-enters any response.
    pub fn set_telegram_token(&self, token: &str) -> Result<String, CoreError> {
        self.set_bot_token(Self::TELEGRAM_TOKEN_ENV, token)
    }

    /// The env var that authenticates each chat bridge, by surface name
    /// (docs/design/34 Phase 3). All three are stored identically: the
    /// shared user `.env`, owner-only. `None` for an unknown surface, so a
    /// typo'd path segment is a 400 rather than a new env var nothing
    /// reads.
    pub fn bot_token_env(surface: &str) -> Option<&'static str> {
        match surface {
            "telegram" => Some(Self::TELEGRAM_TOKEN_ENV),
            "discord" => Some("DISCORD_BOT_TOKEN"),
            "slack" => Some("SLACK_BOT_TOKEN"),
            _ => None,
        }
    }

    /// True when a bridge for `surface` launched right now would find a
    /// token.
    pub fn bot_token_configured(surface: &str) -> bool {
        Self::bot_token_env(surface).is_some_and(|env| vak_config::get_var(env).is_some())
    }

    /// Generic form of [`Core::set_telegram_token`], shared by every chat
    /// surface so one storage convention covers all of them.
    pub fn set_bot_token(&self, env: &str, token: &str) -> Result<String, CoreError> {
        let token = token.trim();
        if token.is_empty() {
            return Err(CoreError::InvalidConfig(format!("empty token for {env}")));
        }
        let path = self.user_env_file();
        vak_config::upsert_env_file(&path, env, token)
            .map_err(|e| CoreError::InvalidConfig(format!("writing {path:?}: {e}")))?;
        vak_config::set_override(env, token);
        Ok(env.to_string())
    }

    /// Revoke the stored Telegram bot token: strip it from the user `.env`
    /// and drop the runtime override. A token exported in the real
    /// environment cannot be unset from here — the caller is told so it can
    /// say as much.
    pub fn remove_telegram_token(&self) -> Result<RemovedKey, CoreError> {
        self.remove_bot_token(Self::TELEGRAM_TOKEN_ENV)
    }

    /// Generic form of [`Core::remove_telegram_token`].
    pub fn remove_bot_token(&self, env: &str) -> Result<RemovedKey, CoreError> {
        let path = self.user_env_file();
        vak_config::remove_env_file_key(&path, env)
            .map_err(|e| CoreError::InvalidConfig(format!("writing {path:?}: {e}")))?;
        vak_config::clear_override(env);
        vak_config::forget_dotenv_var(env);
        Ok(RemovedKey {
            env_var: env.to_string(),
            shadowed_by_env: vak_config::get_var(env).is_some(),
        })
    }

    /// Ask `provider` which models its configured key can actually reach.
    ///
    /// There is no baked-in catalogue: an out-of-date table silently hides
    /// models a provider shipped yesterday and offers ones the key cannot
    /// use. Results are cached briefly because pickers poll this.
    pub async fn discover_models(&self, provider: &str) -> Result<Vec<String>, CoreError> {
        const TTL: std::time::Duration = std::time::Duration::from_secs(300);
        if let Ok(cache) = self.inner.models_cache.lock()
            && let Some((at, models)) = cache.get(provider)
            && at.elapsed() < TTL
        {
            return Ok(models.clone());
        }
        let auth = self.provider_auth_for(provider)?;
        let models = vak_llm::models::list_models(provider, &auth).await?;
        if let Ok(mut cache) = self.inner.models_cache.lock() {
            cache.insert(
                provider.to_string(),
                (std::time::Instant::now(), models.clone()),
            );
        }
        Ok(models)
    }

    /// Drop memoised discovery for `provider` (or all of it) so the next
    /// read reflects a key that just changed.
    pub fn invalidate_models_cache(&self, provider: Option<&str>) {
        if let Ok(mut cache) = self.inner.models_cache.lock() {
            match provider {
                Some(p) => {
                    cache.remove(p);
                }
                None => cache.clear(),
            }
        }
    }

    /// Frozen-ladder admission (docs/design/27 Phase B + Phase R).
    ///
    /// Pure with respect to its inputs: warm discovery caches, the
    /// evidence ledger, session beliefs, config, and tool count. No
    /// network, no invented model ids. The operator-selected primary is
    /// pinned to the head; v2 ordering decides only the FALLBACK order.
    fn plan_route_ladder(&self, primary: vak_llm::RouteLeg) -> routing::RoutePlan {
        let mut candidates = vec![primary.clone()];

        // Same-model legs on other keyed providers (legacy Phase B set).
        if let Ok(cache) = self.inner.models_cache.lock() {
            for (p, (_, models)) in cache.iter() {
                if *p != primary.provider
                    && models.contains(&primary.model)
                    && self.provider_auth_for(p).is_ok()
                    && !candidates.iter().any(|c| c.provider == *p)
                {
                    candidates.push(vak_llm::RouteLeg {
                        provider: p.clone(),
                        model: primary.model.clone(),
                    });
                }
            }
        }

        // Phase R cross-model legs: ONLY exact ids from the explicit
        // `[route].fallback_models` allowlist, admitted when warm
        // discovery shows a configured key reaches them.
        let route_cfg = &self.inner.config.route;
        if !route_cfg.fallback_models.is_empty()
            && let Ok(cache) = self.inner.models_cache.lock()
        {
            for (p, (_, models)) in cache.iter() {
                if self.provider_auth_for(p).is_err() {
                    continue;
                }
                for m in models {
                    if route_cfg.fallback_models.contains(m)
                        && m != &primary.model
                        && !candidates.iter().any(|c| c.provider == *p && c.model == *m)
                    {
                        candidates.push(vak_llm::RouteLeg {
                            provider: p.clone(),
                            model: m.clone(),
                        });
                    }
                }
            }
        }
        candidates.sort();
        candidates.dedup();

        // Demand scoring from facts available at admission. Unknown
        // context reads as moderate -- never zero, never fabricated.
        let demand = vak_llm::score_demand(vak_llm::DemandInput {
            estimated_input_tokens: 0,
            output_budget_tokens: u64::from(self.inner.config.max_tokens),
            tool_count: self.tool_names().len(),
            structured_output: false,
            reasoning_required: false,
            evidence_required: false,
        });
        let objective = vak_llm::QualityObjective::resolve(
            (route_cfg.objective != "auto").then_some(route_cfg.objective.as_str()),
            demand.band,
        );

        let belief_map = vak_llm::BeliefMap {
            multipliers: self.inner.beliefs.snapshot().multipliers,
        };
        let finops_cfg = self.inner.config.finops.clone();
        let home = self.sessions_home();
        let hints = route_cfg.quality_hints.clone();
        let ranked = vak_llm::order_ladder_v2(
            candidates,
            &routing::EvidenceLedger::new(&home).snapshot(),
            &belief_map,
            objective,
            &hints,
            move |m: &str| {
                vak_config::finops::resolve_usd_per_mtok(m, &finops_cfg.price_overrides)
                    .map(|(_, out)| out)
            },
        );

        let (ladder, annotations) = routing::assemble_ladder(
            &primary,
            ranked,
            route_cfg.max_fallbacks,
            !route_cfg.fallback_models.is_empty(),
        );
        routing::RoutePlan {
            ladder,
            objective: objective.as_str().to_string(),
            annotations,
        }
    }

    pub async fn start_session(&self) -> Result<SessionLog, CoreError> {
        let route = self.refresh_persisted_route()?;
        self.start_session_with_route(route.provider, route.model)
            .await
    }

    /// Create a frozen session from an explicit provider/model pair without
    /// mutating the Core default. Gateway/channel overrides use this path so
    /// concurrent surfaces cannot overwrite one another's admission route.
    pub async fn start_session_with_route(
        &self,
        provider: String,
        model: String,
    ) -> Result<SessionLog, CoreError> {
        let session_id = uuid_like();
        let path = vak_session::SessionPath::new_session_file(
            &self.sessions_home(),
            &self.inner.cwd,
            &session_id,
        );
        let plan = self.plan_route_ladder(vak_llm::RouteLeg {
            provider: provider.clone(),
            model: model.clone(),
        });
        let header = SessionHeader {
            session_id,
            created_at: chrono::Utc::now(),
            cwd: self.inner.cwd.clone(),
            parent_session_id: None,
            contract: FrozenContract {
                app_version: APP_VERSION.into(),
                provider,
                model,
                // Frozen-ladder admission (docs/design/27 Phase B +
                // Phase R): primary leg always first; additional legs
                // ONLY from warm discovery caches -- the same model on
                // other keyed providers, plus explicit `[route]`
                // fallback_models when warm discovery reaches them.
                // No invented ids, no network at admission. Ordered by
                // demand-scored v2 over TTL-filtered evidence and
                // session beliefs, diversity-capped, then frozen.
                route_ladder: plan.ladder,
                route_objective: plan.objective,
                route_annotations: plan.annotations,
                system_prompt: self.system_prompt(),
                tools: self.tool_names(),
                permission_mode: format!("{:?}", self.effective_permission_mode())
                    .to_kebab_lowercase(),
                skills: self.skills().iter().map(|s| s.name.clone()).collect(),
            },
        };
        Ok(SessionLog::create(path, header)?)
    }

    pub async fn run_turn(
        &self,
        session: SessionLog,
        prompt: &str,
        cancel: CancellationToken,
        events: tokio::sync::mpsc::Sender<AgentEvent>,
    ) -> Result<TurnOutcome, CoreError> {
        let (outcome, _session) = self
            .run_turn_with(session, prompt, cancel, None, None, None, events)
            .await?;
        Ok(outcome)
    }

    #[allow(clippy::too_many_arguments)]
    pub async fn run_turn_with_message(
        &self,
        session: SessionLog,
        prompt: vak_llm::Message,
        cancel: CancellationToken,
        approver: Option<std::sync::Arc<dyn vak_agent::Approver>>,
        permission: Option<std::sync::Arc<vak_permission::PermissionEngine>>,
        steering: Option<std::sync::Arc<vak_agent::SteeringQueues>>,
        events: tokio::sync::mpsc::Sender<AgentEvent>,
    ) -> Result<(TurnOutcome, SessionLog), CoreError> {
        self.run_turn_inner(
            session, prompt, cancel, approver, permission, steering, events, None,
        )
        .await
    }

    #[allow(clippy::too_many_arguments)]
    pub async fn run_turn_with(
        &self,
        session: SessionLog,
        prompt: &str,
        cancel: CancellationToken,
        approver: Option<std::sync::Arc<dyn vak_agent::Approver>>,
        permission: Option<std::sync::Arc<vak_permission::PermissionEngine>>,
        steering: Option<std::sync::Arc<vak_agent::SteeringQueues>>,
        events: tokio::sync::mpsc::Sender<AgentEvent>,
    ) -> Result<(TurnOutcome, SessionLog), CoreError> {
        self.run_turn_inner(
            session,
            vak_llm::Message::user_text(prompt),
            cancel,
            approver,
            permission,
            steering,
            events,
            None,
        )
        .await
    }

    /// Goal-mode turn (docs/design/27 Phase H): the run may only end when
    /// the objective's acceptance criteria pass an independent audit.
    #[allow(clippy::too_many_arguments)]
    pub async fn run_goal_turn_with(
        &self,
        session: SessionLog,
        prompt: &str,
        objective: &str,
        criteria: Vec<String>,
        cancel: CancellationToken,
        approver: Option<std::sync::Arc<dyn vak_agent::Approver>>,
        permission: Option<std::sync::Arc<vak_permission::PermissionEngine>>,
        steering: Option<std::sync::Arc<vak_agent::SteeringQueues>>,
        events: tokio::sync::mpsc::Sender<AgentEvent>,
    ) -> Result<(TurnOutcome, SessionLog), CoreError> {
        self.run_turn_inner(
            session,
            vak_llm::Message::user_text(prompt),
            cancel,
            approver,
            permission,
            steering,
            events,
            Some((objective.to_string(), criteria)),
        )
        .await
    }

    #[allow(clippy::too_many_arguments)]
    async fn run_turn_inner(
        &self,
        session: SessionLog,
        prompt: vak_llm::Message,
        cancel: CancellationToken,
        approver: Option<std::sync::Arc<dyn vak_agent::Approver>>,
        permission: Option<std::sync::Arc<vak_permission::PermissionEngine>>,
        steering: Option<std::sync::Arc<vak_agent::SteeringQueues>>,
        events: tokio::sync::mpsc::Sender<AgentEvent>,
        goal: Option<(String, Vec<String>)>,
    ) -> Result<(TurnOutcome, SessionLog), CoreError> {
        let session_contract = session.header().map(|header| header.contract.clone());
        let (provider, model) = match session_contract.as_ref() {
            Some(contract) => {
                let injected = self
                    .inner
                    .provider_instance
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner)
                    .clone();
                let provider = if let Some(provider) = injected {
                    provider
                } else {
                    let auth = self.provider_auth_for(&contract.provider)?;
                    self.inner.registry.get(&contract.provider, &auth)?
                };
                (provider, contract.model.clone())
            }
            None => (self.provider()?, self.effective_model()),
        };
        let mut cfg = AgentConfig::new(self.system_prompt());
        cfg.model = model.clone();
        cfg.tools = self.agent_tools();
        cfg.max_turns = self.effective_max_turns();
        cfg.parallel_tools = true;
        cfg.max_retries = self.inner.config.max_retries;
        cfg.retry_base_backoff_ms = self.inner.config.retry_base_backoff_ms;
        cfg.run_retry_attempts = self.inner.config.run_retry_attempts;
        cfg.run_retry_base_backoff_ms = self.inner.config.run_retry_base_backoff_ms;
        cfg.request_timeout = if self.inner.config.request_timeout_secs == 0 {
            None
        } else {
            Some(std::time::Duration::from_secs(
                self.inner.config.request_timeout_secs,
            ))
        };
        cfg.context_policy.context_window = self.inner.config.context_window;
        cfg.context_policy.max_output = u64::from(self.inner.config.max_tokens);
        let sp = &self.inner.config.stop_policy;
        cfg.stop_policy = if sp.enabled {
            Some(vak_agent::StopPolicy {
                marker_gate: sp.marker_gate,
                verify_gate: sp.verify_gate,
                max_blocks: sp.max_blocks,
            })
        } else {
            None
        };
        cfg.circuit_breaker = Some(self.inner.breaker.clone());
        cfg.handoff_reset = self.inner.config.goal.handoff_reset;
        cfg.max_audit_blocks = self.inner.config.goal.max_audit_blocks;
        cfg.approver = approver.clone();
        // Pre-dispatch budget admission (docs/design/27 Phase D): active
        // whenever any finops knob is configured.
        let f = self.effective_finops();
        if f.max_run_usd.is_some() || f.max_day_usd.is_some() || !f.price_overrides.is_empty() {
            let sid = session
                .header()
                .map(|h| h.session_id.clone())
                .unwrap_or_default();
            // Reused for every turn of this session (not rebuilt per
            // turn) so `max_run_usd`'s spend counter actually spans the
            // whole run instead of resetting on each message.
            cfg.spend_gate = Some(self.spend_gate_for(&sid));

            // MEA substrate (Phase H): auditor sees the workspace delta between
            // this run's start checkpoint and the live tree.
            {
                let home = self.sessions_home();
                let seq = self.next_checkpoint_seq(&sid);
                let cwd = self.inner.cwd.clone();
                cfg.workspace_delta = Some(Arc::new(CheckpointDelta {
                    home: home.clone(),
                    sid: sid.clone(),
                    seq,
                    cwd,
                }));
            }
        }
        cfg.mode = match self.effective_permission_mode() {
            vak_config::PermissionMode::ReadOnly => vak_permission::Mode::ReadOnly,
            vak_config::PermissionMode::WorkspaceWrite => vak_permission::Mode::WorkspaceWrite,
            vak_config::PermissionMode::FullAccess => vak_permission::Mode::FullAccess,
        };
        let mut permission_rules = self.extra_allow_snapshot();
        if let Some(policy) = self.channel_policy() {
            if let Some(allow) = policy.tools_allow {
                if allow.is_empty() {
                    permission_rules.extend(
                        ["read", "write", "edit", "bash", "glob", "grep"]
                            .into_iter()
                            .map(|tool| format!("-{tool}")),
                    );
                } else {
                    permission_rules.extend(allow.into_iter().map(|pattern| format!("+{pattern}")));
                }
            }
            permission_rules.extend(
                policy
                    .tools_deny
                    .into_iter()
                    .map(|pattern| format!("-{pattern}")),
            );
            if let Some(allow) = policy.mcp_allow {
                if allow.is_empty() {
                    permission_rules.push("-mcp".into());
                } else {
                    permission_rules.push("+mcp".into());
                }
            }
            permission_rules.extend(
                policy
                    .mcp_deny
                    .into_iter()
                    .map(|pattern| format!("-mcp({pattern})")),
            );
        }
        cfg.permission = Some(match permission {
            Some(p) => p,
            None => std::sync::Arc::new(build_engine_for_mode(
                &self.inner.config,
                &permission_rules,
                self.effective_permission_mode(),
            )?),
        });
        let Some(engine) = cfg.permission.clone() else {
            return Err(CoreError::MissingEngine);
        };
        cfg.sandbox = self.build_sandbox();

        // Phase B: materialize fallback legs beyond the primary provider.
        // Unresolvable legs (missing key/registry) skip silently --
        // receipts record whatever actually walked.
        if let Some(contract) = session_contract.as_ref()
            && contract.route_ladder.len() > 1
        {
            for leg in contract.route_ladder.iter().skip(1) {
                if leg.provider == contract.provider {
                    continue;
                }
                if let Ok(auth) = self.provider_auth_for(&leg.provider)
                    && let Ok(p) = self.inner.registry.get(&leg.provider, &auth)
                {
                    cfg.ladder.push((p, leg.model.clone()));
                }
            }
        }

        let mut tools = self.agent_tools();
        if self.effective_subagents()
            && let Some(parent_id) = session.header().map(|h| h.session_id.clone())
        {
            tools.push(Arc::new(vak_agent::TaskTool::new(vak_agent::TaskDeps {
                provider: provider.clone(),
                system_prompt: self.system_prompt(),
                model: model.clone(),
                tools: self.agent_tools(),
                read_only_tools: self.agent_read_only_tools(),
                max_turns: self.effective_max_turns(),
                permission: Some(engine.clone()),
                mode: cfg.mode,
                approver: approver.clone(),
                sandbox: self.build_sandbox(),
                cwd: self.inner.cwd.clone(),
                sessions_home: self.inner.sessions_home.clone(),
                parent_session_id: parent_id,
                events: Some(events.clone()),
                registry: Some(self.inner.subagents.clone()),
            })));
        }
        if let Some(manager) = self.mcp_manager() {
            // Turn admission never blocks on an optional integration: the
            // manager above is reused across turns (built once per
            // resolved server set — see `mcp_manager()`) and this lazy
            // meta-tool only actually connects when the model calls it.
            // `system_prompt()` advertises what's configured — richly,
            // once `spawn_mcp_inventory_warm` has a result, or by name
            // only until then.
            let policy = self.channel_policy().unwrap_or_default();
            let context = self.plugin_mcp_invocation_context();
            let recorder = Arc::new(move |server: &str, tool: &str, success: bool| {
                for (store, plugin, trace_id) in &context {
                    if server.starts_with(&format!("plugin.{plugin}.")) {
                        let _ = store.record_invocation(
                            trace_id,
                            plugin,
                            &format!("mcp:{server}/{tool}"),
                            success,
                        );
                        break;
                    }
                }
            });
            tools.push(Arc::new(vak_mcp::McpTool::with_policy_and_recorder(
                manager,
                policy.mcp_allow,
                policy.mcp_deny,
                recorder,
            )));
        }
        if self.effective_memory_search_enabled() {
            let exclude = session
                .header()
                .map(|h| h.session_id.clone())
                .unwrap_or_default();
            tools.push(Arc::new(session_search::SessionSearchTool {
                // The accessor honors the sessions-home override; the raw
                // field does not.
                sessions_home: self.sessions_home(),
                cwd: self.inner.cwd.clone(),
                exclude_session_id: exclude,
            }));
        }
        // Scheduling from a plain-language request ("remind me every
        // morning at 8"), from any surface — CLI, desktop, or a bound chat
        // — since they all run this same turn assembly. Always on: unlike
        // memory search/write there is no cost or drift risk to gate it
        // behind, and a user who never asks for a schedule never triggers
        // it. `default_deliver_to` (set per inbound message by the
        // gateway) routes a task's result back into this conversation
        // unless the model names a different one explicitly.
        tools.push(Arc::new(tools_tasks::TasksTool {
            sessions_home: self.sessions_home(),
            cwd: self.inner.cwd.clone(),
            default_deliver_to: self.default_deliver_to.clone(),
        }));
        // Learning loop (docs/design/26-learning.md): journaling tools are
        // ordinary model-visible tools; promotion stays human-only.
        let current_session = session
            .header()
            .map(|h| h.session_id.clone())
            .unwrap_or_default();
        if self.effective_memory_write_enabled() {
            tools.push(Arc::new(learning::RememberTool {
                sessions_home: self.sessions_home(),
                cwd: self.inner.cwd.clone(),
                session_id: current_session.clone(),
            }));
        }
        if self.effective_memory_skill_proposals() {
            tools.push(Arc::new(learning::ProposeSkillTool {
                sessions_home: self.sessions_home(),
                cwd: self.inner.cwd.clone(),
                session_id: current_session,
            }));
        }
        // Bounded web fetch (docs/design/29-personal-os.md P4): registered
        // like the other broker-owned narrow tools; every dispatch crosses
        // the permission engine, where it is classified network-capable.
        if self.inner.config.tools.web_fetch {
            tools.push(Arc::new(vak_tools::WebFetchTool));
        }
        // Headless-browser DOM render: same narrow broker-owned shape as
        // webfetch — the child Chromium process is spawned from wherever the
        // tool executes, never holding policy or credentials.
        if self.inner.config.tools.browse {
            tools.push(Arc::new(vak_tools::WebBrowseTool));
        }
        // Apply channel visibility after every tool has been assembled. Memory
        // and optional tools are added below the base registry, so filtering
        // only the initial built-in vector would leak capabilities through an
        // explicit channel allowlist.
        tools.retain(|tool| self.channel_tool_allowed(tool.name()));
        cfg.tools = tools;
        let hooks: Option<std::sync::Arc<Vec<vak_hooks::HookDef>>> = Some(std::sync::Arc::new(
            build_hooks_from(&self.effective_hooks())?,
        ));
        cfg.hooks = hooks.clone();
        let plugin_hooks: Vec<_> = [self.sessions_home(), self.inner.cwd.join(".vak")]
            .into_iter()
            .flat_map(|home| {
                vak_plugin::PluginStore::new(home)
                    .enabled_hooks()
                    .unwrap_or_default()
            })
            .collect();
        let user_home = self.sessions_home();
        let workspace_home = self.inner.cwd.join(".vak");
        cfg.hook_recorder = Some(Arc::new(move |hook: &vak_hooks::HookDef, success: bool| {
            if let Some((plugin, _)) = plugin_hooks
                .iter()
                .find(|(_, candidate)| candidate.command == hook.command)
            {
                let store = vak_plugin::PluginStore::new(match plugin.scope {
                    vak_plugin::InstallScope::User => &user_home,
                    vak_plugin::InstallScope::Workspace => &workspace_home,
                });
                let _ = store.record_invocation(
                    &plugin.trace_id,
                    &plugin.name,
                    &format!("hook:{}", hook.event.as_str()),
                    success,
                );
            }
        }));

        // session-start hooks fire once per run, before any tool or
        // checkpoint activity. A block aborts the run before it starts.
        if let Some(hook_defs) = &hooks
            && hook_defs
                .iter()
                .any(|h| h.event == vak_hooks::HookEvent::SessionStart)
        {
            let session_id = session
                .header()
                .map(|h| h.session_id.clone())
                .unwrap_or_default();
            let outcome = vak_hooks::run_hooks_with_recorder(
                hook_defs.clone(),
                vak_hooks::HookEvent::SessionStart,
                &session_id,
                &self.inner.cwd,
                None,
                None,
                &cancel,
                cfg.hook_recorder.as_deref(),
            )
            .await;
            if outcome.blocked {
                let reason = outcome.reason.unwrap_or_else(|| "blocked by hook".into());
                return Err(CoreError::HookBlocked(format!("session-start: {reason}")));
            }
        }

        // Checkpoint the workspace before any mutation of this run.
        if let Some(h) = session.header() {
            let seq = self.next_checkpoint_seq(&h.session_id);
            if let Ok(cp) = checkpoints::capture(
                &self.inner.cwd,
                &h.session_id,
                seq,
                &format!("turn: {}", prompt.text_content()),
            ) {
                let _ = checkpoints::store(&self.sessions_home(), &cp);
            }
        }

        let steering = match steering {
            Some(s) => s,
            None => std::sync::Arc::new(vak_agent::SteeringQueues::new()),
        };
        let mut agent = Agent::new(provider, session, cfg);
        if let Some((objective, criteria)) = goal {
            agent.set_goal(objective, criteria);
        }
        let receipts_before = {
            let s = agent.session.lock().await;
            s.receipts().len()
        };
        let outcome = agent.run_message(prompt, &steering, cancel, events).await;
        let session = agent.into_session().await;

        // Phase B: fold this run's dispatches into the routing evidence
        // ledger (success / failure / unknown by settlement).
        let new_receipts: Vec<vak_llm::WorkReceipt> = session
            .receipts()
            .into_iter()
            .skip(receipts_before)
            .cloned()
            .collect();
        if !new_receipts.is_empty() {
            routing::EvidenceLedger::new(&self.sessions_home()).record_receipts(&new_receipts);
            // Phase R: fold the same dispatches into session beliefs.
            // Domain-weighted doubt accumulates per leg; one success
            // clears it. Cancelled attempts say nothing.
            for r in &new_receipts {
                for a in &r.attempts {
                    let (provider, model) = r.attempt_leg(a);
                    if provider.is_empty() || model.is_empty() {
                        continue;
                    }
                    match a.settlement {
                        vak_llm::Settlement::Ok => {
                            self.inner
                                .beliefs
                                .record_outcome(provider, model, a.domain, true);
                        }
                        vak_llm::Settlement::Failed => {
                            self.inner
                                .beliefs
                                .record_outcome(provider, model, a.domain, false);
                        }
                        _ => {}
                    }
                }
            }
        }

        Ok((outcome, session))
    }

    fn next_checkpoint_seq(&self, session_id: &str) -> u32 {
        checkpoints::list(&self.sessions_home(), session_id)
            .map(|list| list.last().map(|c| c.seq + 1).unwrap_or(0))
            .unwrap_or(0)
    }

    /// User-invoked compaction (`/compact`): summarize older turns into a
    /// compaction entry now, regardless of the automatic trigger threshold.
    /// Append-only; a receipt entry audits the summarizer dispatch. The
    /// session always returns; failures land in `CompactOutcome.error`.
    pub async fn compact_session_now(
        &self,
        mut session: SessionLog,
        cancel: tokio_util::sync::CancellationToken,
    ) -> (SessionLog, CompactOutcome) {
        let policy = vak_agent::context::ContextPolicy {
            context_window: self.inner.config.context_window,
            max_output: u64::from(self.inner.config.max_tokens),
            ..Default::default()
        };
        let system = self.system_prompt();
        let tool_defs = vak_tools::definitions(&self.agent_tools());
        let before = vak_agent::context::estimate_tokens(
            &session.derive_messages(),
            Some(&system),
            &tool_defs,
        );
        let Some(plan) = session.plan_compaction(policy.keep_recent) else {
            return (
                session,
                CompactOutcome {
                    report: None,
                    error: None,
                },
            );
        };
        let provider = match self.provider() {
            Ok(p) => p,
            Err(e) => return (session, CompactOutcome::failed(e.to_string())),
        };
        let model = self.effective_model();
        let summarized = plan.older.len();
        let transcript = vak_agent::context::render_transcript(&plan.older);
        let req = vak_agent::context::compaction_request(&model, &transcript);

        let started = std::time::Instant::now();
        let mut receipt =
            vak_llm::WorkReceipt::new(vak_llm::WorkPurpose::Summarize, provider.name(), &model);
        let summary = match provider.stream(req, cancel).await {
            Ok(stream) => match stream.result().await {
                Ok(msg) => {
                    receipt.record(
                        vak_llm::AttemptReason::Initial,
                        vak_llm::FailureDomain::Unknown,
                        vak_llm::Settlement::Ok,
                        started.elapsed().as_millis() as u64,
                        Some(msg.usage.clone()),
                        None,
                    );
                    msg.text_content()
                }
                Err(e) => {
                    receipt.record(
                        vak_llm::AttemptReason::Initial,
                        vak_llm::FailureDomain::Unknown,
                        vak_llm::Settlement::Failed,
                        started.elapsed().as_millis() as u64,
                        None,
                        Some(e.to_string()),
                    );
                    let _ = session.append_receipt(receipt);
                    return (session, CompactOutcome::failed(e.to_string()));
                }
            },
            Err(e) => {
                receipt.record(
                    vak_llm::AttemptReason::Initial,
                    vak_llm::FailureDomain::Unknown,
                    vak_llm::Settlement::Cancelled,
                    started.elapsed().as_millis() as u64,
                    None,
                    Some(e.to_string()),
                );
                let _ = session.append_receipt(receipt);
                return (session, CompactOutcome::failed(e.to_string()));
            }
        };
        if summary.trim().is_empty() {
            let _ = session.append_receipt(receipt);
            return (
                session,
                CompactOutcome::failed("compaction produced an empty summary".into()),
            );
        }
        if let Err(e) = session.apply_compaction(&plan, summary, before) {
            return (
                session,
                CompactOutcome::failed(format!("compaction write failed: {e}")),
            );
        }
        let _ = session.append_receipt(receipt);
        let after = vak_agent::context::estimate_tokens(
            &session.derive_messages(),
            Some(&system),
            &tool_defs,
        );
        (
            session,
            CompactOutcome {
                report: Some(CompactReport {
                    before_tokens: before,
                    after_tokens: after,
                    summarized_messages: summarized,
                }),
                error: None,
            },
        )
    }

    fn build_sandbox(&self) -> Option<std::sync::Arc<dyn vak_tools::sandbox::Sandbox>> {
        let mode = match self.effective_permission_mode() {
            vak_config::PermissionMode::ReadOnly => SandboxMode::ReadOnly,
            vak_config::PermissionMode::WorkspaceWrite => SandboxMode::WorkspaceWrite,
            vak_config::PermissionMode::FullAccess => return None,
        };
        let backend = self.effective_sandbox_backend();
        let backend = backend.as_str();
        if backend == "docker" {
            // Fail closed at call time if the daemon is unreachable:
            // BashTool surfaces the wrapped command's error verbatim, and
            // the probe keeps startup cheap.
            return Some(std::sync::Arc::new(sandbox_docker::DockerSandbox::new(
                mode,
                self.inner.config.sandbox.image.clone(),
                self.inner.cwd.as_path(),
            )));
        }
        #[cfg(target_os = "macos")]
        {
            use vak_tools::sandbox::Seatbelt;
            let _ = backend;
            Some(std::sync::Arc::new(Seatbelt::new(
                mode,
                self.inner.cwd.as_path(),
            )))
        }
        #[cfg(target_os = "linux")]
        {
            if backend == "seatbelt" {
                // Explicit cross-platform pin that cannot apply here: fall
                // through to Landlock rather than weakening.
                return Some(std::sync::Arc::new(vak_tools::landlock::Landlock::new(
                    mode,
                    self.inner.cwd.as_path(),
                )));
            }
            Some(std::sync::Arc::new(vak_tools::landlock::Landlock::new(
                mode,
                self.inner.cwd.as_path(),
            )))
        }
        #[cfg(not(any(target_os = "macos", target_os = "linux")))]
        {
            let _ = backend;
            Some(std::sync::Arc::new(vak_tools::sandbox::DenySandbox::new(
                "restricted execution is unsupported on this platform",
            )))
        }
    }

    pub fn effective_sandbox_name(&self) -> String {
        match self.build_sandbox() {
            Some(sb) => sb.name().to_string(),
            None => "off".to_string(),
        }
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod channel_mcp_network_tests {
    use super::Core;
    use std::collections::BTreeMap;

    fn core_with_servers(dir: &std::path::Path, servers: &[(&str, bool)]) -> Core {
        let mut servers_toml = String::new();
        for (name, network) in servers {
            servers_toml.push_str(&format!(
                "[mcp.servers.{name}]\ncommand = \"echo\"\nnetwork = {network}\n"
            ));
        }
        let vak = dir.join(".vak");
        std::fs::create_dir_all(&vak).unwrap();
        std::fs::write(vak.join("config.toml"), servers_toml).unwrap();
        Core::new_with_trust(dir.to_path_buf(), true).unwrap()
    }

    fn network_map(core: &Core) -> BTreeMap<String, bool> {
        core.effective_mcp()
            .servers
            .into_iter()
            .map(|(name, server)| (name, server.network))
            .collect()
    }

    /// A channel policy that only *removes* network from a server the
    /// config already grants it to — never adds it to one the config
    /// denies. That asymmetry is the whole point (AGENTS.md rule 20:
    /// overlays are restrictive-only).
    #[test]
    fn channel_policy_can_only_take_network_away_never_grant_it() {
        let dir = tempfile::tempdir().unwrap();
        let core = core_with_servers(dir.path(), &[("tavily", true), ("sandboxed", false)]);
        assert_eq!(
            network_map(&core),
            BTreeMap::from([("tavily".into(), true), ("sandboxed".into(), false)]),
            "sanity: both servers report their own configured network setting with no policy"
        );

        core.apply_channel_policy(vak_config::ChannelPolicy {
            mcp_network_deny: vec!["tavily/*".into(), "sandboxed/*".into()],
            ..Default::default()
        });
        assert_eq!(
            network_map(&core),
            BTreeMap::from([("tavily".into(), false), ("sandboxed".into(), false)]),
            "tavily's network must be forced off; sandboxed already was and stays off"
        );
    }

    /// An un-matched server keeps its own configured value; the deny list
    /// is per-server, not a channel-wide network kill switch.
    #[test]
    fn network_deny_pattern_only_affects_matching_servers() {
        let dir = tempfile::tempdir().unwrap();
        let core = core_with_servers(dir.path(), &[("tavily", true), ("github", true)]);
        core.apply_channel_policy(vak_config::ChannelPolicy {
            mcp_network_deny: vec!["tavily/*".into()],
            ..Default::default()
        });
        let map = network_map(&core);
        assert!(!map["tavily"]);
        assert!(
            map["github"],
            "github did not match the pattern; must be untouched"
        );
    }

    #[test]
    fn channel_allowlist_removes_memory_capabilities_from_advertised_tools() {
        let dir = tempfile::tempdir().unwrap();
        let core = Core::new_with_trust(dir.path().to_path_buf(), true).unwrap();
        core.apply_channel_policy(vak_config::ChannelPolicy {
            tools_allow: Some(vec!["read".into()]),
            ..Default::default()
        });
        let names = core.tool_names();
        assert!(
            !names
                .iter()
                .any(|n| matches!(n.as_str(), "remember" | "propose_skill" | "session_search"))
        );
        assert!(!core.channel_tool_allowed("remember"));
    }
}

pub fn build_engine(
    config: &vak_config::Config,
) -> Result<vak_permission::PermissionEngine, CoreError> {
    build_engine_with(config, &[])
}

/// `extra` carries learned rules from permissions.local.toml; the engine
/// aggregates by severity, so they can never shadow explicit denies.
pub fn build_engine_with(
    config: &vak_config::Config,
    extra: &[String],
) -> Result<vak_permission::PermissionEngine, CoreError> {
    vak_permission::PermissionEngine::from_rule_strings(&rule_specs(config, extra))
        .map_err(CoreError::Rule)
}

fn rule_specs(config: &vak_config::Config, extra: &[String]) -> Vec<String> {
    let mut specs: Vec<String> = Vec::new();
    for (list, prefix) in [
        (&config.deny, "-"),
        (&config.ask, "?"),
        (&config.allow, "+"),
    ] {
        for s in list {
            let spec = if s.starts_with(['+', '-', '?']) {
                s.clone()
            } else {
                format!("{prefix}{s}")
            };
            specs.push(spec);
        }
    }
    specs.extend(extra.iter().cloned());
    specs
}

/// Tools whose reach exceeds the workspace: network-capable capabilities
/// registered next to built-ins (docs/design/29-personal-os.md P4).
pub const NETWORK_TOOLS: [&str; 2] = ["webfetch", "browse"];

/// True when a BLANKET (patternless) rule spec targets `tool`. Only
/// blanket rules govern the injection decision: patterned rules cannot
/// match webfetch requests today (`arg_candidates` has no webfetch family),
/// so suppressing the Ask default on their behalf would widen access on
/// arguments the rule can never see — restricted modes keep asking.
fn has_blanket_rule_for(specs: &[String], tool: &str) -> bool {
    specs.iter().any(|spec| {
        let rest = spec.trim();
        let rest = rest.strip_prefix(['+', '-', '?']).unwrap_or(rest);
        let rest = rest.trim();
        !rest.contains('(') && rest.eq_ignore_ascii_case(tool)
    })
}

/// Mode-aware engine construction — the seam where network-capable tools
/// are permission-classified (docs/design/29-personal-os.md P4): outside
/// FullAccess every network tool gains an implicit Ask default; under
/// FullAccess the mode's allow-by-default applies untouched. Injection is
/// skipped when a blanket webfetch rule exists in any layer, because
/// severity aggregation would otherwise rank an injected Ask over a
/// deliberate Allow/Deny; deny always outranks ask regardless of layer.
pub fn build_engine_for_mode(
    config: &vak_config::Config,
    extra: &[String],
    mode: vak_config::PermissionMode,
) -> Result<vak_permission::PermissionEngine, CoreError> {
    let mut specs = rule_specs(config, extra);
    if mode != vak_config::PermissionMode::FullAccess {
        for tool in NETWORK_TOOLS {
            if !has_blanket_rule_for(&specs, tool) {
                specs.push(format!("?{tool}"));
            }
        }
    }
    vak_permission::PermissionEngine::from_rule_strings(&specs).map_err(CoreError::Rule)
}

trait KebabLower {
    fn to_kebab_lowercase(&self) -> String;
}

impl KebabLower for str {
    fn to_kebab_lowercase(&self) -> String {
        self.chars()
            .map(|c| {
                if c.is_uppercase() {
                    c.to_ascii_lowercase()
                } else {
                    c
                }
            })
            .map(|c| if c == '_' { '-' } else { c })
            .collect()
    }
}

/// Session ids are UUIDv7, matching entry ids in the same tree: time-
/// ordered and collision-safe even across clock rewinds (the previous
/// nanosecond-hex scheme appended a second header onto an existing file
/// on collision).
fn uuid_like() -> String {
    uuid::Uuid::now_v7().to_string()
}

pub fn build_hooks(config: &vak_config::Config) -> Result<Vec<vak_hooks::HookDef>, CoreError> {
    build_hooks_from(&config.hooks)
}

fn build_hooks_from(
    config_hooks: &[vak_config::HookConfig],
) -> Result<Vec<vak_hooks::HookDef>, CoreError> {
    let mut out = Vec::with_capacity(config_hooks.len());
    for h in config_hooks {
        if !h.enabled {
            continue;
        }
        let event = match h.event.as_str() {
            "session-start" | "session_start" | "start" => vak_hooks::HookEvent::SessionStart,
            "pre-tool-use" | "pre_tool_use" => vak_hooks::HookEvent::PreToolUse,
            "post-tool-use" | "post_tool_use" => vak_hooks::HookEvent::PostToolUse,
            "stop" => vak_hooks::HookEvent::Stop,
            other => {
                return Err(CoreError::Config(vak_config::ConfigError::Read {
                    path: self_path(),
                    source: std::io::Error::other(format!("unknown hook event '{other}'")),
                }));
            }
        };
        let matcher = match &h.matcher {
            Some(m) if !m.trim().is_empty() => Some(vak_permission::Rule::parse(m)?),
            _ => None,
        };
        out.push(vak_hooks::HookDef {
            event,
            matcher,
            command: h.command.clone(),
            timeout_ms: h.timeout_ms.unwrap_or(vak_hooks::DEFAULT_TIMEOUT_MS),
        });
    }
    Ok(out)
}

impl Core {
    /// Background reflection seam (docs/design/29 P1): after a completed
    /// turn on ANY surface, offer one auxiliary-model reflection pass.
    ///
    /// Contract: background reflection is best-effort by design. It never
    /// blocks or fails a completed turn — every unhappy path collapses into
    /// [`reflection::ReflectionOutcome::Skipped`] with a static reason,
    /// never `Err`. Dispatch is budget-admitted BEFORE any provider call
    /// through the same gate path runs use, so reflection can never bypass
    /// a day/run cap; concurrent turns over the same session tail collapse
    /// to a single pass via an in-flight marker; and all reflection logic
    /// is the shared implementation in [`crate::reflection`] (no fork).
    pub async fn reflect_after_turn(
        &self,
        session: &SessionLog,
        final_text: &str,
    ) -> reflection::ReflectionOutcome {
        if !self.effective_memory_reflection() {
            return reflection::ReflectionOutcome::Skipped {
                reason: "reflection-disabled",
            };
        }
        if !self.effective_memory_write_enabled() {
            return reflection::ReflectionOutcome::Skipped {
                reason: "memory-writes-disabled",
            };
        }
        if matches!(
            self.effective_permission_mode(),
            vak_config::PermissionMode::ReadOnly
        ) {
            return reflection::ReflectionOutcome::Skipped {
                reason: "permission-mode-read-only",
            };
        }
        if !self.memory_write_allowed() {
            return reflection::ReflectionOutcome::Skipped {
                reason: "memory-write-not-authorized",
            };
        }
        let sid = session
            .header()
            .map(|h| h.session_id.clone())
            .unwrap_or_default();
        let Ok(provider) = self.provider() else {
            return reflection::ReflectionOutcome::Skipped {
                reason: "provider-unready",
            };
        };
        // One pass per session tail at a time, across every surface in
        // this process.
        let Some(_in_flight) = reflection::InFlightGuard::acquire(&sid) else {
            return reflection::ReflectionOutcome::Skipped {
                reason: "already-in-flight",
            };
        };

        let mut tail = String::new();
        for (_, m) in session.message_chain() {
            tail.push_str(&reflection::render_message(m.role, &m.content));
        }
        if !final_text.is_empty() {
            let block = vak_llm::ContentBlock::text(final_text.to_string());
            tail.push_str(&reflection::render_message(
                vak_llm::Role::Assistant,
                &[block],
            ));
        }

        // Budget admission before any dispatch — the same CoreSpendGate
        // path that admits run turns, so caps bind identically here.
        let f = self.effective_finops();
        if f.max_run_usd.is_some() || f.max_day_usd.is_some() || !f.price_overrides.is_empty() {
            // The session's own gate, not a fresh one — so a reflection
            // pass is admitted against the SAME run/day budget the
            // session's turns have already been spending from.
            let gate = self.spend_gate_for(&sid);
            let probe = vak_llm::Message {
                role: vak_llm::Role::User,
                content: vec![vak_llm::ContentBlock::text(tail.clone())],
            };
            let est = vak_agent::context::estimate_tokens(
                &[probe],
                Some(&reflection::system_prompt()),
                &[],
            );
            let model = self.effective_model();
            let provider_name = self.effective_provider();
            let check = vak_agent::SpendCheck {
                model: &model,
                provider: &provider_name,
                session_id: &sid,
                est_input_tokens: est,
                planned_output_tokens: u64::from(reflection::MAX_TOKENS),
            };
            if vak_agent::SpendGate::authorize(gate.as_ref(), &check)
                .await
                .is_err()
            {
                return reflection::ReflectionOutcome::Skipped { reason: "budget" };
            }
        }

        let model = self.effective_model();
        let proposals =
            match reflection::propose(provider, &model, &tail, CancellationToken::new()).await {
                Ok(p) => p,
                Err(_) => {
                    return reflection::ReflectionOutcome::Skipped {
                        reason: "reflect-call-failed",
                    };
                }
            };
        if proposals.notes.is_empty() && proposals.skill.is_none() {
            return reflection::ReflectionOutcome::Reflected {
                notes_added: 0,
                skills_proposed: false,
            };
        }
        let home = self.sessions_home();
        let mut proposals = proposals;
        if !self.channel_tool_allowed("propose_skill") {
            proposals.skill = None;
        }
        match reflection::apply(home.as_path(), self.cwd(), &sid, &proposals) {
            Ok((notes_added, skills_proposed)) => reflection::ReflectionOutcome::Reflected {
                notes_added,
                skills_proposed,
            },
            Err(_) => reflection::ReflectionOutcome::Skipped {
                reason: "apply-failed",
            },
        }
    }
}

fn load_permissions_local(cwd: &std::path::Path) -> Vec<String> {
    let Ok(text) = std::fs::read_to_string(cwd.join(PERMISSIONS_LOCAL_FILE)) else {
        return Vec::new();
    };
    match toml::from_str::<PermissionsLocal>(&text) {
        Ok(p) => p.allow,
        Err(_) => Vec::new(),
    }
}

fn project_config_has_key(cwd: &std::path::Path, key: &str) -> bool {
    std::fs::read_to_string(vak_config::project_path(cwd))
        .ok()
        .and_then(|raw| toml::from_str::<toml::Value>(&raw).ok())
        .and_then(|value| value.as_table().map(|table| table.contains_key(key)))
        .unwrap_or(false)
}

fn global_config_has_key(key: &str) -> bool {
    vak_config::global_path()
        .and_then(|path| std::fs::read_to_string(path).ok())
        .and_then(|raw| toml::from_str::<toml::Value>(&raw).ok())
        .and_then(|value| value.as_table().map(|table| table.contains_key(key)))
        .unwrap_or(false)
}

fn project_profile_has_key(cwd: &std::path::Path, key: &str) -> bool {
    config_profile_has_key(&vak_config::project_path(cwd), key)
}

fn global_profile_has_key(key: &str) -> bool {
    vak_config::global_path().is_some_and(|path| config_profile_has_key(&path, key))
}

fn config_profile_has_key(path: &std::path::Path, key: &str) -> bool {
    let Some(value) = std::fs::read_to_string(path)
        .ok()
        .and_then(|raw| toml::from_str::<toml::Value>(&raw).ok())
    else {
        return false;
    };
    let Some(profile) = value.get("profile").and_then(toml::Value::as_str) else {
        return false;
    };
    value
        .get("profiles")
        .and_then(|profiles| profiles.get(profile))
        .and_then(|profile| profile.get(key))
        .is_some()
}

fn self_path() -> std::path::PathBuf {
    std::path::PathBuf::from(".vak/config.toml")
}

/// Resolve `${NAME}` references in an MCP server env value through
/// `vak_config::get_var` (override → process env → dotenv). Returns None
/// when any reference is unresolved so callers can drop the pair instead of
/// leaking a literal placeholder into a child environment.
pub fn interpolate_env_var(value: &str) -> Option<String> {
    if !value.contains("${") {
        return Some(value.to_string());
    }
    let mut out = String::with_capacity(value.len());
    let mut rest = value;
    while let Some(start) = rest.find("${") {
        out.push_str(&rest[..start]);
        let after = &rest[start + 2..];
        let end = after.find('}')?;
        let name = &after[..end];
        if name.is_empty() || !name.chars().all(|c| c.is_ascii_alphanumeric() || c == '_') {
            return None;
        }
        out.push_str(&vak_config::get_var(name)?);
        rest = &after[end + 1..];
    }
    out.push_str(rest);
    Some(out)
}

/// `CoreInner::mcp_cache`'s payload: the manager currently live for
/// `fingerprint`'s server set, plus whatever the background inventory
/// warm-up has produced for it so far.
/// server name -> (tool name, description) pairs, as returned by
/// `McpManager::inventory()`.
type McpInventory = Vec<(String, Vec<(String, String)>)>;

struct McpCache {
    fingerprint: u64,
    manager: Arc<vak_mcp::McpManager>,
    inventory: Option<McpInventory>,
    /// True while a `spawn_mcp_inventory_warm` task for this fingerprint
    /// is in flight, so a burst of turns doesn't each fire their own
    /// discovery pass against the same server set.
    warming: bool,
}

/// Identifies one resolved MCP server set for cache-invalidation purposes:
/// two calls that resolve to the same names/commands/args/env/network
/// settings get the same fingerprint and reuse one manager; anything that
/// changes what `effective_mcp()` returns (a runtime `set_mcp_servers`, a
/// plugin enabled/disabled) changes the fingerprint and rebuilds. Not a
/// security boundary — only used to decide "same manager or not".
fn mcp_fingerprint(servers: &[(String, vak_mcp::ServerConfig)]) -> u64 {
    use std::hash::{Hash, Hasher};
    let mut sorted: Vec<&(String, vak_mcp::ServerConfig)> = servers.iter().collect();
    sorted.sort_by(|a, b| a.0.cmp(&b.0));
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    for (name, cfg) in sorted {
        name.hash(&mut hasher);
        cfg.command.hash(&mut hasher);
        cfg.args.hash(&mut hasher);
        cfg.network.hash(&mut hasher);
        for (k, v) in &cfg.env {
            k.hash(&mut hasher);
            v.hash(&mut hasher);
        }
    }
    hasher.finish()
}

/// Compact capability section from an MCP inventory snapshot
/// (`McpManager::inventory`). Used by `system_prompt()` once the
/// background warm-up in `Core::spawn_mcp_inventory_warm` has a result;
/// `mcp_config_section` below is the name-only fallback for before that
/// (or when discovery failed for every server).
fn mcp_section(inventory: &[(String, Vec<(String, String)>)]) -> String {
    if inventory.is_empty() {
        return String::new();
    }
    let mut out = String::from(
        "\nMCP tool servers available now — use the `mcp` \
tool (action \"call\", server, tool, arguments). Prefer these over guessing \
when the task matches:\n",
    );
    for (server, tools) in inventory {
        // `McpManager::inventory` degrades a connect/list failure to a
        // single ("error", <message>) pseudo-tool per server; render it as
        // an error, not as a callable tool named "error".
        if let [(name, msg)] = tools.as_slice()
            && name.as_str() == "error"
        {
            out.push_str(&format!("- {server}: unavailable ({msg})\n"));
            continue;
        }
        if tools.is_empty() {
            out.push_str(&format!("- {server}: (no tools)\n"));
            continue;
        }
        for (name, desc) in tools.iter().take(12) {
            out.push_str(&format!(
                "- mcp call server=\"{server}\" tool=\"{name}\" — {}\n",
                desc.trim_end()
            ));
        }
    }
    out
}

/// Model-visible fallback for configured MCP servers when live discovery is
/// unavailable. This belongs in the frozen session contract as well as the
/// live prompt so a transient launcher failure cannot hide a capability.
fn mcp_config_section(servers: &[String]) -> String {
    if servers.is_empty() {
        return String::new();
    }
    format!(
        "\nConfigured MCP servers: {}. Use the `mcp` tool with action \"list\" to inspect their tools, then action \"call\" with server, tool, and arguments.\n",
        servers.join(", ")
    )
}

/// Phase H MEA provider: diff the run-start checkpoint against disk.
struct CheckpointDelta {
    home: PathBuf,
    sid: String,
    seq: u32,
    cwd: PathBuf,
}

impl vak_agent::WorkspaceDelta for CheckpointDelta {
    fn summary(&self) -> Result<String, String> {
        checkpoints::delta_summary(
            &self.cwd.clone(),
            &self.home.clone(),
            self.sid.as_str(),
            self.seq,
            8192,
        )
        .map_err(|e| e.to_string())
    }
}

#[cfg(test)]
mod mcp_section_tests {
    use super::{mcp_config_section, mcp_section};

    #[test]
    fn section_lists_server_tools_with_call_hint() {
        let inv = vec![(
            "tavily".to_string(),
            vec![
                ("tavily-search".to_string(), "Web search".to_string()),
                (
                    "tavily-extract".to_string(),
                    "Extract page content".to_string(),
                ),
            ],
        )];
        let s = mcp_section(&inv);
        assert!(s.contains("tavily-search"));
        assert!(s.contains("Web search"));
        assert!(s.contains("action \"call\""));
        assert!(s.contains("server=\"tavily\""));
    }

    #[test]
    fn configured_servers_remain_visible_without_inventory() {
        let section = mcp_config_section(&["tavily".to_string()]);
        assert!(section.contains("tavily"));
        assert!(section.contains("`mcp`"));
        assert!(section.contains("action \"list\""));
    }

    #[test]
    fn empty_inventory_is_silent_but_named_servers_listed() {
        assert_eq!(mcp_section(&[]), "");
        let s = mcp_section(&[("x".into(), vec![])]);
        assert!(s.contains("- x: (no tools)"));
    }
}

#[cfg(test)]
mod plugin_runtime_tests {
    #![allow(clippy::unwrap_used)]

    use super::*;

    #[test]
    fn enabled_plugin_mcp_is_namespaced_and_disabled_plugin_is_invisible() {
        let dir = tempfile::tempdir().unwrap();
        let package = dir.path().join("plugin");
        std::fs::create_dir_all(&package).unwrap();
        std::fs::write(
            package.join("vak-plugin.json"),
            r#"{"schema":1,"name":"tools-pack","version":"1.0.0","description":"Tools","license":"MIT","components":{"mcp":["mcp.json"]}}"#,
        )
        .unwrap();
        std::fs::write(
            package.join("mcp.json"),
            r#"{"mcpServers":{"lookup":{"command":"lookup-bin","args":["--safe"]}}}"#,
        )
        .unwrap();
        let core = Core::new_with_trust(dir.path().to_path_buf(), true).unwrap();
        core.set_sessions_home(dir.path().join("home"));
        let store = vak_plugin::PluginStore::new(dir.path().join(".vak"));
        store
            .install_local(package.as_path(), vak_plugin::InstallOptions::default())
            .unwrap();
        assert!(core.effective_mcp().servers.is_empty());
        store.enable("tools-pack").unwrap();
        let servers = core.effective_mcp().servers;
        let server = servers.get("plugin.tools-pack.lookup").unwrap();
        assert_eq!(server.command, "lookup-bin");
        assert_eq!(server.args, ["--safe"]);
        assert!(!server.network);
    }

    /// The whole point of caching `McpManager` on `Core` (see
    /// `mcp_manager()`): a hot plugin enable/disable — the same kind of
    /// on-the-fly change `set_mcp_servers` makes at runtime — must be
    /// picked up on the very next call, not require a restart, while an
    /// unrelated repeat call in between reuses the same manager instance
    /// rather than respawning server connections for no reason.
    #[tokio::test]
    async fn mcp_manager_reuses_instance_and_picks_up_hot_plugin_toggle() {
        let dir = tempfile::tempdir().unwrap();
        let package = dir.path().join("plugin");
        std::fs::create_dir_all(&package).unwrap();
        std::fs::write(
            package.join("vak-plugin.json"),
            r#"{"schema":1,"name":"tools-pack","version":"1.0.0","description":"Tools","license":"MIT","components":{"mcp":["mcp.json"]}}"#,
        )
        .unwrap();
        std::fs::write(
            package.join("mcp.json"),
            r#"{"mcpServers":{"lookup":{"command":"lookup-bin","args":["--safe"]}}}"#,
        )
        .unwrap();
        let core = Core::new_with_trust(dir.path().to_path_buf(), true).unwrap();
        core.set_sessions_home(dir.path().join("home"));
        let store = vak_plugin::PluginStore::new(dir.path().join(".vak"));
        store
            .install_local(package.as_path(), vak_plugin::InstallOptions::default())
            .unwrap();

        // Nothing configured yet: no manager.
        assert!(core.mcp_manager().is_none());

        // Enable on the fly: next call sees the new server immediately.
        store.enable("tools-pack").unwrap();
        let first = core.mcp_manager().unwrap();
        assert_eq!(first.server_names(), vec!["plugin.tools-pack.lookup"]);

        // Same server set: same manager instance (no respawn/reconnect).
        let second = core.mcp_manager().unwrap();
        assert!(
            Arc::ptr_eq(&first, &second),
            "unchanged server set must reuse the cached manager"
        );

        // Disable on the fly: next call sees it's gone immediately, no
        // restart required.
        store.disable("tools-pack").unwrap();
        assert!(core.mcp_manager().is_none());
    }
}

#[cfg(test)]
mod webfetch_classification_tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]
    use super::*;

    use vak_config::PermissionMode;
    use vak_permission::{Decision, Mode, PermissionEngine};

    fn decide(
        cfg: &vak_config::Config,
        extra: &[String],
        mode: PermissionMode,
        tool: &str,
    ) -> Decision {
        let dir = tempfile::tempdir().unwrap();
        let engine: PermissionEngine =
            build_engine_for_mode(cfg, extra, mode).expect("engine builds");
        engine.evaluate(tool, &serde_json::json!({}), to_mode(mode), dir.path())
    }

    fn to_mode(mode: PermissionMode) -> Mode {
        match mode {
            PermissionMode::ReadOnly => Mode::ReadOnly,
            PermissionMode::WorkspaceWrite => Mode::WorkspaceWrite,
            PermissionMode::FullAccess => Mode::FullAccess,
        }
    }

    #[test]
    fn no_rule_outside_fullaccess_asks_fullaccess_allows() {
        let cfg = vak_config::Config::default();
        for mode in [PermissionMode::ReadOnly, PermissionMode::WorkspaceWrite] {
            for tool in NETWORK_TOOLS {
                let d = decide(&cfg, &[], mode, tool);
                assert!(
                    matches!(d, Decision::Ask { .. }),
                    "{tool} in {mode:?} must Ask, got {d:?}"
                );
            }
        }
        for tool in NETWORK_TOOLS {
            let d = decide(&cfg, &[], PermissionMode::FullAccess, tool);
            assert!(
                matches!(d, Decision::Allow),
                "{tool} under FullAccess must Allow, got {d:?}"
            );
        }
    }

    #[test]
    fn explicit_allow_rule_wins_in_every_restricted_mode() {
        for spec in ["webfetch", "+webfetch"] {
            let mut cfg = vak_config::Config::default();
            cfg.allow.push(spec.into());
            for mode in [PermissionMode::ReadOnly, PermissionMode::WorkspaceWrite] {
                let d = decide(&cfg, &[], mode, "webfetch");
                assert!(matches!(d, Decision::Allow), "{spec:?} in {mode:?}: {d:?}");
            }
        }
        // Learned rules ride in via `extra`.
        let cfg = vak_config::Config::default();
        for mode in [PermissionMode::ReadOnly, PermissionMode::WorkspaceWrite] {
            let d = decide(&cfg, &["+webfetch".to_string()], mode, "webfetch");
            assert!(matches!(d, Decision::Allow), "learned in {mode:?}: {d:?}");
        }
    }

    #[test]
    fn explicit_deny_rule_wins_everywhere() {
        let mut cfg = vak_config::Config::default();
        cfg.deny.push("-webfetch".into());
        for mode in [
            PermissionMode::ReadOnly,
            PermissionMode::WorkspaceWrite,
            PermissionMode::FullAccess,
        ] {
            let d = decide(&cfg, &[], mode, "webfetch");
            assert!(matches!(d, Decision::Deny { .. }), "{mode:?}: {d:?}");
        }
    }

    #[test]
    fn explicit_ask_rule_is_honored_even_under_fullaccess() {
        let mut cfg = vak_config::Config::default();
        cfg.ask.push("?webfetch".into());
        for mode in [
            PermissionMode::ReadOnly,
            PermissionMode::WorkspaceWrite,
            PermissionMode::FullAccess,
        ] {
            let d = decide(&cfg, &[], mode, "webfetch");
            assert!(matches!(d, Decision::Ask { .. }), "{mode:?}: {d:?}");
        }
    }

    #[test]
    fn browse_shares_the_full_webfetch_mode_matrix() {
        // Same family, same posture: restricted modes Ask by default,
        // FullAccess allows, blanket allow lifts the Ask, blanket deny
        // denies even under FullAccess, and an explicit ?ask is honored
        // everywhere.
        let base = vak_config::Config::default();
        for mode in [PermissionMode::ReadOnly, PermissionMode::WorkspaceWrite] {
            let d = decide(&base, &[], mode, "browse");
            assert!(matches!(d, Decision::Ask { .. }), "{mode:?}: {d:?}");
        }
        let d = decide(&base, &[], PermissionMode::FullAccess, "browse");
        assert!(matches!(d, Decision::Allow), "FullAccess: {d:?}");

        let mut allowed = vak_config::Config::default();
        allowed.allow.push("browse".into());
        for mode in [PermissionMode::ReadOnly, PermissionMode::WorkspaceWrite] {
            let d = decide(&allowed, &[], mode, "browse");
            assert!(
                matches!(d, Decision::Allow),
                "allow rule in {mode:?}: {d:?}"
            );
        }

        let mut denied = vak_config::Config::default();
        denied.deny.push("-browse".into());
        for mode in [
            PermissionMode::ReadOnly,
            PermissionMode::WorkspaceWrite,
            PermissionMode::FullAccess,
        ] {
            let d = decide(&denied, &[], mode, "browse");
            assert!(
                matches!(d, Decision::Deny { .. }),
                "-browse in {mode:?}: {d:?}"
            );
        }

        let mut asked = vak_config::Config::default();
        asked.ask.push("?browse".into());
        let d = decide(&asked, &[], PermissionMode::FullAccess, "browse");
        assert!(
            matches!(d, Decision::Ask { .. }),
            "?browse under FullAccess: {d:?}"
        );
    }

    #[test]
    fn deny_outranks_allow_regardless_of_layer_order() {
        let mut cfg = vak_config::Config::default();
        cfg.allow.push("webfetch".into());
        cfg.deny.push("-webfetch".into());
        let dir = tempfile::tempdir().unwrap();
        let e = build_engine_for_mode(&cfg, &[], PermissionMode::WorkspaceWrite).unwrap();
        let d = e.evaluate(
            "webfetch",
            &serde_json::json!({}),
            Mode::WorkspaceWrite,
            dir.path(),
        );
        assert!(matches!(d, Decision::Deny { .. }));
    }

    #[test]
    fn patterned_allow_cannot_lift_the_ask_default() {
        // Patterned rules cannot see webfetch args today, so a patterned
        // allow must NOT suppress the injected Ask: matching URLs still ask
        // (severity Ask > Allow) and non-matching ones fall through to the
        // same Ask. Failing toward asking is the safe direction.
        let mut cfg = vak_config::Config::default();
        cfg.allow.push("webfetch(example.com/*)".into());
        let dir = tempfile::tempdir().unwrap();
        let e = build_engine_for_mode(&cfg, &[], PermissionMode::WorkspaceWrite).unwrap();
        for url in ["other.org/x", "example.com/x"] {
            let d = e.evaluate(
                "webfetch",
                &serde_json::json!({"url": url}),
                Mode::WorkspaceWrite,
                dir.path(),
            );
            assert!(matches!(d, Decision::Ask { .. }), "{url}: {d:?}");
        }
    }

    #[test]
    fn other_tools_keep_their_existing_defaults() {
        // The seam must not widen: bash still asks under workspace-write,
        // session_search stays a read tool, remember stays sanctioned.
        let dir = tempfile::tempdir().unwrap();
        let e = build_engine_for_mode(
            &vak_config::Config::default(),
            &[],
            PermissionMode::WorkspaceWrite,
        )
        .unwrap();
        let bash = e.evaluate(
            "bash",
            &serde_json::json!({"command": "ls"}),
            Mode::WorkspaceWrite,
            dir.path(),
        );
        assert!(matches!(bash, Decision::Ask { .. }));
        for tool in ["session_search", "remember", "propose_skill"] {
            let d = e.evaluate(
                tool,
                &serde_json::json!({}),
                Mode::WorkspaceWrite,
                dir.path(),
            );
            assert!(matches!(d, Decision::Allow), "{tool}: {d:?}");
        }
    }

    #[test]
    fn has_rule_for_matches_only_blanket_specs() {
        assert!(has_blanket_rule_for(&["webfetch".into()], "webfetch"));
        assert!(has_blanket_rule_for(&["+webfetch".into()], "webfetch"));
        assert!(has_blanket_rule_for(&["?WEBFETCH".into()], "webfetch"));
        assert!(!has_blanket_rule_for(&["webfetch(x)".into()], "webfetch"));
        assert!(!has_blanket_rule_for(&["webfetchy".into()], "webfetch"));
        assert!(!has_blanket_rule_for(&["bash".into()], "webfetch"));
        assert!(has_blanket_rule_for(&["browse".into()], "browse"));
        assert!(has_blanket_rule_for(&["+BROWSE".into()], "browse"));
        assert!(!has_blanket_rule_for(&["browse(x)".into()], "browse"));
        assert!(!has_blanket_rule_for(&["browser".into()], "browse"));
    }
}

/// Every `Core` accessor must terminate while session-scoped overrides are
/// set. `cache_home` once locked `sessions_home_override` and then called
/// `sessions_home()`, which locks the same non-reentrant mutex — the
/// thread wedged forever. It surfaced only in tests, because production
/// leaves the override unset and never entered the branch, and it made
/// `cargo test --workspace` hang rather than fail.
///
/// These run each accessor on a worker thread with a deadline, so a
/// reintroduced deadlock fails the suite instead of hanging it. A test
/// that hangs reports nothing and blocks every gate behind it.
#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod override_deadlock {
    use super::*;

    /// Run `f` on its own thread, failing if it has not returned in time.
    fn within<T: Send + 'static>(label: &str, f: impl FnOnce() -> T + Send + 'static) -> T {
        let (tx, rx) = std::sync::mpsc::channel();
        let handle = std::thread::spawn(move || {
            let out = f();
            // Send may fail if the receiver already gave up; the timeout
            // below is what reports that, so ignore the error here.
            let _ = tx.send(());
            out
        });
        match rx.recv_timeout(std::time::Duration::from_secs(10)) {
            Ok(()) => handle.join().expect("accessor thread panicked"),
            Err(_) => panic!(
                "{label} did not return within 10s — an accessor is deadlocked \
                 (a guard held across a call that re-locks the same mutex)"
            ),
        }
    }

    /// One accessor to exercise, boxed so a heterogeneous set can share
    /// a list.
    type AccessorCheck = Box<dyn FnOnce(Arc<Core>) + Send>;

    fn core_with_override() -> (tempfile::TempDir, Core) {
        let dir = tempfile::tempdir().unwrap();
        let core = Core::new_with_trust(dir.path().to_path_buf(), true).unwrap();
        core.set_sessions_home(dir.path().join("home"));
        (dir, core)
    }

    #[test]
    fn cache_home_returns_while_the_sessions_home_override_is_set() {
        let (dir, core) = core_with_override();
        let expected = dir.path().join("home").join("cache");
        let got = within("cache_home", move || core.cache_home());
        assert_eq!(
            got, expected,
            "an overridden home is a self-contained sandbox"
        );
    }

    #[test]
    fn cache_home_falls_back_to_the_platform_directory_without_an_override() {
        let dir = tempfile::tempdir().unwrap();
        let core = Core::new_with_trust(dir.path().to_path_buf(), true).unwrap();
        let got = within("cache_home (no override)", move || core.cache_home());
        assert_eq!(got, vak_config::paths::cache_home());
    }

    #[test]
    fn every_override_backed_accessor_terminates() {
        // Breadth matters more than depth here: the hazard is the
        // locking idiom, so each accessor that reads an override is
        // exercised with one set.
        let (_dir, core) = core_with_override();
        let core = Arc::new(core);
        core.set_model("m".into());
        core.set_provider("p".into());
        core.set_permission_mode(vak_config::PermissionMode::FullAccess);
        core.set_sandbox_backend(Some("os".into()));

        let checks: Vec<(&str, AccessorCheck)> = vec![
            (
                "sessions_home",
                Box::new(|c: Arc<Core>| {
                    c.sessions_home();
                }),
            ),
            (
                "cache_home",
                Box::new(|c: Arc<Core>| {
                    c.cache_home();
                }),
            ),
            (
                "effective_model",
                Box::new(|c: Arc<Core>| {
                    c.effective_model();
                }),
            ),
            (
                "effective_provider",
                Box::new(|c: Arc<Core>| {
                    c.effective_provider();
                }),
            ),
            (
                "effective_max_turns",
                Box::new(|c: Arc<Core>| {
                    c.effective_max_turns();
                }),
            ),
            (
                "effective_theme",
                Box::new(|c: Arc<Core>| {
                    c.effective_theme();
                }),
            ),
            (
                "effective_permission_mode",
                Box::new(|c: Arc<Core>| {
                    c.effective_permission_mode();
                }),
            ),
            (
                "effective_sandbox_backend",
                Box::new(|c: Arc<Core>| {
                    c.effective_sandbox_backend();
                }),
            ),
        ];
        for (label, check) in checks {
            let c = Arc::clone(&core);
            within(label, move || check(c));
        }
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod route_control_tests {
    use super::*;

    #[test]
    fn provider_and_model_are_never_observed_as_a_torn_pair() {
        let dir = tempfile::tempdir().unwrap();
        let core = Arc::new(Core::new(dir.path().to_path_buf()).unwrap());
        core.set_route("provider-a".into(), "model-a".into());
        let writer = {
            let core = Arc::clone(&core);
            std::thread::spawn(move || {
                for _ in 0..10_000 {
                    core.set_route("provider-b".into(), "model-b".into());
                    core.set_route("provider-a".into(), "model-a".into());
                }
            })
        };
        for _ in 0..20_000 {
            let route = core.effective_route();
            assert!(
                (route.provider == "provider-a" && route.model == "model-a")
                    || (route.provider == "provider-b" && route.model == "model-b")
            );
        }
        writer.join().unwrap();
    }

    #[test]
    fn independent_cores_observe_one_persisted_route_change() {
        let dir = tempfile::tempdir().unwrap();
        let first = Core::new(dir.path().to_path_buf()).unwrap();
        let second = Core::new(dir.path().to_path_buf()).unwrap();
        vak_config::persist_project_preferences(
            dir.path(),
            Some("provider-new"),
            Some("model-new"),
            None,
            None,
            None,
        )
        .unwrap();
        first.refresh_persisted_route().unwrap();
        second.refresh_persisted_route().unwrap();
        assert_eq!(first.effective_provider(), "provider-new");
        assert_eq!(first.effective_model(), "model-new");
        assert_eq!(first.effective_route(), second.effective_route());
    }

    #[test]
    fn persisted_non_route_preferences_refresh_without_touching_runtime_pins() {
        let dir = tempfile::tempdir().unwrap();
        let core = Core::new(dir.path().to_path_buf()).unwrap();
        vak_config::persist_project_preferences(
            dir.path(),
            None,
            None,
            Some(17),
            None,
            Some("plain"),
        )
        .unwrap();
        core.refresh_persisted_preferences().unwrap();
        assert_eq!(core.effective_max_turns(), 17);
        assert_eq!(core.effective_theme(), "plain");

        core.set_max_turns(23);
        vak_config::persist_project_preferences(dir.path(), None, None, Some(31), None, None)
            .unwrap();
        core.refresh_persisted_preferences().unwrap();
        assert_eq!(core.effective_max_turns(), 23, "scoped runtime pin wins");
    }

    #[tokio::test]
    async fn explicit_session_route_does_not_mutate_the_shared_default() {
        let dir = tempfile::tempdir().unwrap();
        let core = Core::new(dir.path().to_path_buf()).unwrap();
        core.set_sessions_home(dir.path().join("home"));
        let default = core.effective_route();
        let session = core
            .start_session_with_route("channel-provider".into(), "channel-model".into())
            .await
            .unwrap();
        let contract = &session.header().unwrap().contract;
        assert_eq!(contract.provider, "channel-provider");
        assert_eq!(contract.model, "channel-model");
        assert_eq!(core.effective_route(), default);
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod spend_gate_persistence_tests {
    // Regression coverage for the FinOps audit fix: `spend_gate_for` used
    // to be `finops::CoreSpendGate::new(..)` called fresh inline on every
    // turn, so `max_run_usd`'s in-memory spend counter reset to zero each
    // message — a multi-turn conversation could blow past the run cap by
    // an arbitrary multiple. `spend_gate_for` now caches one gate per
    // session and reuses it turn over turn.
    use super::Core;
    use std::sync::Arc;
    use vak_agent::{SpendCheck, SpendGate};
    use vak_llm::Usage;

    fn core_with_run_cap(dir: &std::path::Path, cap: f64) -> Core {
        let core = Core::new(dir.join("cwd")).unwrap();
        core.set_sessions_home(dir.join("home"));
        core.apply_persisted_finops_caps(Some(Some(cap)), None);
        core
    }

    fn check(session_id: &'static str) -> SpendCheck<'static> {
        SpendCheck {
            model: "claude-sonnet",
            provider: "anthropic",
            session_id,
            est_input_tokens: 1_000_000,
            planned_output_tokens: 100_000,
        }
    }

    fn usage_1m_in_100k_out() -> Usage {
        Usage {
            input_tokens: 1_000_000,
            output_tokens: 100_000,
            ..Default::default()
        }
    }

    #[test]
    fn same_session_reuses_one_gate_across_calls() {
        let dir = tempfile::tempdir().unwrap();
        let core = core_with_run_cap(dir.path(), 5.0);

        let turn1 = core.spend_gate_for("s1");
        let turn2 = core.spend_gate_for("s1");
        assert!(
            Arc::ptr_eq(&turn1, &turn2),
            "the same session must get the SAME gate on its next turn, \
             not a freshly zeroed one"
        );

        let other_session = core.spend_gate_for("s2");
        assert!(
            !Arc::ptr_eq(&turn1, &other_session),
            "a different session must not share another session's run budget"
        );
    }

    #[tokio::test]
    async fn run_cap_spend_survives_across_simulated_turns() {
        // sonnet: $3/MTok in, $15/MTok out => 1M in + 100k out = $4.50.
        let dir = tempfile::tempdir().unwrap();
        let core = core_with_run_cap(dir.path(), 5.0);

        // Turn 1: `run_turn_inner` fetches the session's gate, admits the
        // dispatch, then records it settled.
        let gate = core.spend_gate_for("s1");
        gate.authorize(&check("s1")).await.unwrap();
        gate.record_settled("anthropic", "claude-sonnet", "s1", &usage_1m_in_100k_out());

        // Turn 2: a SEPARATE `run_turn_inner` call for the same session
        // re-fetches the gate. Before this fix, that call built a brand
        // new `CoreSpendGate` with `run_spent_usd` back at zero, so this
        // dispatch was wrongly admitted even though the run cap was
        // already exhausted by turn 1.
        let gate_next_turn = core.spend_gate_for("s1");
        let err = gate_next_turn
            .authorize(&check("s1"))
            .await
            .expect_err("run cap must still reflect turn 1's spend on turn 2");
        assert!(err.contains("run budget $5.00"), "{err}");
    }

    #[test]
    fn forget_spend_gate_drops_cached_state() {
        let dir = tempfile::tempdir().unwrap();
        let core = core_with_run_cap(dir.path(), 5.0);

        let before = core.spend_gate_for("s1");
        core.forget_spend_gate("s1");
        let after = core.spend_gate_for("s1");
        assert!(
            !Arc::ptr_eq(&before, &after),
            "forgetting a session's gate must not leave the old one cached"
        );

        // Forgetting a session with no cached gate must be a harmless no-op.
        core.forget_spend_gate("never-seen");
    }
}
