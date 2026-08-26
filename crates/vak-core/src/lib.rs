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
pub mod learning;
pub mod memory;
pub mod reflection;
pub mod routing;
pub mod sandbox_docker;
pub mod security_events;
pub mod session_search;
pub mod skills;
pub mod tasks;
pub mod transcript_md;
pub mod worktree;

use std::collections::HashMap;
use std::path::{Path, PathBuf};
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
    model_override: std::sync::Mutex<Option<String>>,
    provider_override: std::sync::Mutex<Option<String>>,
    max_turns_override: std::sync::Mutex<Option<usize>>,
    mode_override: std::sync::Mutex<Option<vak_config::PermissionMode>>,
    theme_override: std::sync::Mutex<Option<String>>,
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
    /// Cached MCP capability section appended to the system prompt; None
    /// until a run with servers configured populates it.
    mcp_inventory: std::sync::Mutex<Option<String>>,
    /// Runtime MCP table override (desktop/TUI management surface).
    mcp_override: std::sync::Mutex<Option<vak_config::McpConfig>>,
    /// Runtime hook override (desktop/TUI management surface).
    hooks_override: std::sync::Mutex<Option<Vec<vak_config::HookConfig>>>,
    /// Session-scoped domain-weighted doubt per (provider, model) leg
    /// (Phase R). Fed from work receipts at run end; read at ladder
    /// admission.
    beliefs: Arc<routing::BeliefState>,
}

/// Learned permission rules live outside the main config so they can be
/// written at runtime without touching (possibly committed) project config.
pub const PERMISSIONS_LOCAL_FILE: &str = ".vakcoder/permissions.local.toml";

#[derive(serde::Deserialize, Default)]
struct PermissionsLocal {
    #[serde(default)]
    allow: Vec<String>,
}

#[derive(Clone)]
pub struct Core {
    inner: Arc<CoreInner>,
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
        // Canonical layout (doc 32): one resolver for the whole workspace.
        // The cwd fallback covers exotic environments with no HOME.
        let sessions_home = vak_config::paths::data_home();
        let sessions_home = if std::env::var_os("VAKCODER_HOME").is_none()
            && std::env::var_os("HOME").is_none()
            && std::env::var_os("USERPROFILE").is_none()
        {
            cwd.join(".vakcoder")
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
            inner: Arc::new(CoreInner {
                config,
                cwd,
                sessions_home,
                registry: default_registry(),
                model_override: std::sync::Mutex::new(None),
                provider_override: std::sync::Mutex::new(None),
                max_turns_override: std::sync::Mutex::new(None),
                mode_override: std::sync::Mutex::new(None),
                sandbox_backend_override: std::sync::Mutex::new(None),
                theme_override: std::sync::Mutex::new(None),
                provider_instance: std::sync::Mutex::new(None),
                sessions_home_override: std::sync::Mutex::new(None),
                breaker,
                subagents: Arc::new(vak_agent::SubagentRegistry::new()),
                trust_project_config,
                extra_allow: std::sync::Mutex::new(extra_allow),
                user_env_override: std::sync::Mutex::new(None),
                tool_worker_exe: std::sync::Mutex::new(
                    std::env::current_exe()
                        .unwrap_or_else(|_| PathBuf::from("__vakcoder_tool_worker_unavailable__")),
                ),
                models_cache: std::sync::Mutex::new(HashMap::new()),
                mcp_inventory: std::sync::Mutex::new(None),
                mcp_override: std::sync::Mutex::new(None),
                hooks_override: std::sync::Mutex::new(None),
                beliefs: Arc::new(routing::BeliefState::new()),
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

    pub fn set_model(&self, model: String) {
        if let Ok(mut c) = self.inner.model_override.lock() {
            *c = Some(model);
        }
    }

    pub fn effective_model(&self) -> String {
        if let Ok(c) = self.inner.model_override.lock()
            && let Some(m) = c.as_ref()
        {
            return m.clone();
        }
        self.inner.config.model.clone()
    }

    pub fn set_provider(&self, provider: String) {
        if let Ok(mut c) = self.inner.provider_override.lock() {
            *c = Some(provider);
        }
    }

    pub fn effective_provider(&self) -> String {
        if let Ok(c) = self.inner.provider_override.lock()
            && let Some(p) = c.as_ref()
        {
            return p.clone();
        }
        self.inner.config.provider.clone()
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
        if let Ok(mut c) = self.inner.max_turns_override.lock() {
            *c = Some(max_turns);
        }
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
            .unwrap_or_else(|| PathBuf::from("__vakcoder_tool_worker_unavailable__"));
        vak_tools::brokered_default_tools(worker)
    }

    pub fn agent_read_only_tools(&self) -> Vec<Arc<dyn vak_tools::Tool>> {
        let worker = self
            .inner
            .tool_worker_exe
            .lock()
            .ok()
            .map(|worker| worker.clone())
            .unwrap_or_else(|| PathBuf::from("__vakcoder_tool_worker_unavailable__"));
        vak_tools::brokered_read_only_tools(worker)
    }

    pub fn agent_sandbox(&self) -> Option<Arc<dyn vak_tools::sandbox::Sandbox>> {
        self.build_sandbox()
    }

    pub fn effective_max_turns(&self) -> usize {
        if let Ok(c) = self.inner.max_turns_override.lock()
            && let Some(t) = *c
        {
            return t;
        }
        self.inner.config.max_turns
    }

    pub fn set_permission_mode(&self, mode: vak_config::PermissionMode) {
        if let Ok(mut c) = self.inner.mode_override.lock() {
            *c = Some(mode);
        }
    }

    /// Runtime sandbox-backend selection ("os", "docker", or config default
    /// via None). Session-scoped like every other override; never persisted.
    pub fn set_sandbox_backend(&self, backend: Option<String>) {
        if let Ok(mut c) = self.inner.sandbox_backend_override.lock() {
            *c = backend;
        }
    }

    pub fn effective_sandbox_backend(&self) -> String {
        if let Ok(c) = self.inner.sandbox_backend_override.lock()
            && let Some(b) = c.as_ref()
        {
            return b.clone();
        }
        self.inner.config.sandbox.backend.clone()
    }

    pub fn breaker(&self) -> Arc<vak_agent::CircuitBreaker> {
        self.inner.breaker.clone()
    }

    /// Runtime MCP server table replacement (trusted surfaces only). Takes
    /// effect on the next turn; the cached capability section is dropped so
    /// the next run re-discovers the new inventory.
    pub fn set_mcp_servers(&self, config: vak_config::McpConfig) {
        if let Ok(mut c) = self.inner.mcp_override.lock() {
            *c = Some(config);
        }
        if let Ok(mut inv) = self.inner.mcp_inventory.lock() {
            *inv = None;
        }
    }

    pub fn effective_mcp(&self) -> vak_config::McpConfig {
        if let Ok(c) = self.inner.mcp_override.lock()
            && let Some(cfg) = c.as_ref()
        {
            return cfg.clone();
        }
        self.inner.config.mcp.clone()
    }

    /// Replace lifecycle hooks for subsequent turns without restarting the
    /// desktop/server process. Persistence is owned by the server surface.
    pub fn set_hooks(&self, hooks: Vec<vak_config::HookConfig>) {
        if let Ok(mut current) = self.inner.hooks_override.lock() {
            *current = Some(hooks);
        }
    }

    pub fn effective_hooks(&self) -> Vec<vak_config::HookConfig> {
        if let Ok(current) = self.inner.hooks_override.lock()
            && let Some(hooks) = current.as_ref()
        {
            return hooks.clone();
        }
        self.inner.config.hooks.clone()
    }

    pub fn set_theme(&self, theme: String) {
        if let Ok(mut t) = self.inner.theme_override.lock() {
            *t = Some(theme);
        }
    }

    pub fn has_provider_override(&self) -> bool {
        self.inner
            .provider_override
            .lock()
            .map(|c| c.is_some())
            .unwrap_or(false)
    }

    pub fn has_model_override(&self) -> bool {
        self.inner
            .model_override
            .lock()
            .map(|c| c.is_some())
            .unwrap_or(false)
    }

    pub fn has_max_turns_override(&self) -> bool {
        self.inner
            .max_turns_override
            .lock()
            .map(|c| c.is_some())
            .unwrap_or(false)
    }

    pub fn has_theme_override(&self) -> bool {
        self.inner
            .theme_override
            .lock()
            .map(|c| c.is_some())
            .unwrap_or(false)
    }

    pub fn has_permission_mode_override(&self) -> bool {
        self.inner
            .mode_override
            .lock()
            .map(|c| c.is_some())
            .unwrap_or(false)
    }

    /// Persists a learned allow rule to `.vakcoder/permissions.local.toml`
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
        if let Ok(t) = self.inner.theme_override.lock()
            && let Some(name) = t.as_ref()
        {
            return name.clone();
        }
        self.inner.config.ui.theme.clone()
    }

    pub fn effective_permission_mode(&self) -> vak_config::PermissionMode {
        if let Ok(c) = self.inner.mode_override.lock()
            && let Some(m) = *c
        {
            return m;
        }
        self.inner.config.permission_mode
    }

    pub fn cwd(&self) -> &PathBuf {
        &self.inner.cwd
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
        if let Ok(mut c) = self.inner.sessions_home_override.lock() {
            *c = Some(path);
        }
    }

    /// Redirects the user-level secret store (tests, portable installs).
    pub fn set_user_env_path(&self, path: PathBuf) {
        if let Ok(mut c) = self.inner.user_env_override.lock() {
            *c = Some(path);
        }
    }

    fn user_env_file(&self) -> PathBuf {
        self.inner
            .user_env_override
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .clone()
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
        if let Ok(h) = self.inner.sessions_home_override.lock()
            && let Some(p) = h.as_ref()
        {
            return p.clone();
        }
        self.inner.sessions_home.clone()
    }

    /// Rebuildable-artifact directory (SQLite FTS index + WAL sidecars).
    /// Canonical layout (doc 32): Library/Caches on macOS, XDG cache on
    /// Linux — deleting it must always be safe.
    pub fn cache_home(&self) -> PathBuf {
        // Release the override guard before the recursive read below;
        // holding it across sessions_home() would self-deadlock.
        let overridden = self
            .inner
            .sessions_home_override
            .lock()
            .map(|h| h.is_some())
            .unwrap_or(false);
        if overridden {
            // Overridden homes are self-contained sandboxes.
            return self.sessions_home().join("cache");
        }
        vak_config::paths::cache_home()
    }

    pub fn system_prompt(&self) -> String {
        let project_prompt = self.inner.cwd.join(".vakcoder/SYSTEM.md");
        let base = if project_prompt.is_file()
            && let Ok(custom) = std::fs::read_to_string(&project_prompt)
        {
            custom
        } else {
            DEFAULT_SYSTEM_PROMPT.replace("{{version}}", APP_VERSION)
        };
        let discovered = self.skills();
        format!("{}{}", base, skills::prompt_section(&discovered))
    }

    pub fn skills(&self) -> Vec<skills::Skill> {
        skills::discover(&self.inner.cwd, &self.inner.sessions_home)
    }

    /// Live subagents spawned by this Core's runs, for attach/steer UIs.
    pub fn subagents(&self) -> Arc<vak_agent::SubagentRegistry> {
        self.inner.subagents.clone()
    }

    pub fn custom_commands(&self) -> Vec<custom_commands::CustomCommand> {
        custom_commands::discover(&self.inner.cwd, &self.inner.sessions_home)
    }

    pub fn tool_names(&self) -> Vec<String> {
        let mut names: Vec<String> = vak_tools::default_tools()
            .iter()
            .map(|t| t.name().to_string())
            .collect();
        if self.inner.config.subagents {
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

        if self.inner.config.memory.search_enabled {
            names.push("session_search".into());
        }
        if self.inner.config.memory.write_enabled {
            names.push("remember".into());
        }
        if self.inner.config.memory.skill_proposals {
            names.push("propose_skill".into());
        }
        names
    }

    fn provider_auth(&self) -> Result<ProviderAuth, CoreError> {
        self.provider_auth_for(&self.effective_provider())
    }

    /// Resolve credentials for an arbitrary provider, not just the active
    /// one — model discovery needs to authenticate against whichever
    /// provider the user is inspecting.
    fn provider_auth_for(&self, provider: &str) -> Result<ProviderAuth, CoreError> {
        let provider = provider.to_string();
        match provider.as_str() {
            "anthropic" => {
                let api_key = vak_config::get_var("ANTHROPIC_API_KEY").ok_or_else(|| {
                    CoreError::MissingAuth {
                        env: "ANTHROPIC_API_KEY".into(),
                        provider: "anthropic".into(),
                    }
                })?;
                Ok(ProviderAuth {
                    api_key,
                    base_url: self
                        .inner
                        .config
                        .anthropic_base_url
                        .clone()
                        .or_else(|| vak_config::get_var("VAKCODER_ANTHROPIC_BASE_URL")),
                })
            }
            "google" => {
                let api_key = vak_config::get_var("GEMINI_API_KEY")
                    .or_else(|| vak_config::get_var("GOOGLE_API_KEY"))
                    .ok_or_else(|| CoreError::MissingAuth {
                        env: "GEMINI_API_KEY".into(),
                        provider,
                    })?;
                Ok(ProviderAuth {
                    api_key,
                    base_url: vak_config::get_var("VAKCODER_GOOGLE_BASE_URL").or_else(|| {
                        Some("https://generativelanguage.googleapis.com/v1beta".into())
                    }),
                })
            }
            "openai-responses" => {
                let api_key = vak_config::get_var("OPENAI_API_KEY").ok_or_else(|| {
                    CoreError::MissingAuth {
                        env: "OPENAI_API_KEY".into(),
                        provider,
                    }
                })?;
                Ok(ProviderAuth {
                    api_key,
                    base_url: vak_config::get_var("VAKCODER_OPENAI_BASE_URL")
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
                        "VAKCODER_OPENAI_BASE_URL",
                    )
                } else {
                    (
                        "OPENROUTER_API_KEY",
                        "https://openrouter.ai/api/v1",
                        "VAKCODER_OPENROUTER_BASE_URL",
                    )
                };
                let api_key = vak_config::get_var(env).ok_or_else(|| CoreError::MissingAuth {
                    env: env.into(),
                    provider,
                })?;
                Ok(ProviderAuth {
                    api_key,
                    base_url: vak_config::get_var(override_env)
                        .or_else(|| Some(default_base.into())),
                })
            }
            "opencode-zen" => {
                let api_key = vak_config::get_var("OPENCODE_API_KEY").ok_or_else(|| {
                    CoreError::MissingAuth {
                        env: "OPENCODE_API_KEY".into(),
                        provider,
                    }
                })?;
                Ok(ProviderAuth {
                    api_key,
                    base_url: vak_config::get_var("VAKCODER_OPENCODE_ZEN_BASE_URL")
                        .or_else(|| Some("https://opencode.ai/zen/v1".into())),
                })
            }
            "ollama" => Ok(ProviderAuth {
                api_key: "ollama".into(),
                base_url: vak_config::get_var("VAKCODER_OLLAMA_BASE_URL")
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
            "google" => {
                vak_config::get_var("GEMINI_API_KEY").is_some()
                    || vak_config::get_var("GOOGLE_API_KEY").is_some()
            }
            other => vak_config::get_var(Self::provider_env_var(other).unwrap_or("")).is_some(),
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
    fn plan_route_ladder(&self) -> routing::RoutePlan {
        let primary = vak_llm::RouteLeg {
            provider: self.effective_provider(),
            model: self.effective_model(),
        };
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
        self.start_session_in(&self.inner.cwd.clone()).await
    }

    /// Start a session in a specific project directory. The session header
    /// freezes the provided cwd (not Core's own), so agent turns and tools
    /// execute in that workspace. Config is loaded from the project layer
    /// of the provided cwd if it exists.
    pub async fn start_session_in(&self, cwd: &Path) -> Result<SessionLog, CoreError> {
        let session_id = uuid_like();
        let path =
            vak_session::SessionPath::new_session_file(&self.sessions_home(), cwd, &session_id);
        let plan = self.plan_route_ladder();
        let header = SessionHeader {
            session_id,
            created_at: chrono::Utc::now(),
            cwd: cwd.to_path_buf(),
            parent_session_id: None,
            contract: FrozenContract {
                app_version: APP_VERSION.into(),
                provider: self.effective_provider(),
                model: self.effective_model(),
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
        let provider = self.provider()?;
        let mut cfg = AgentConfig::new(self.system_prompt());
        cfg.model = self.effective_model();
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
        let f = &self.inner.config.finops;
        if f.max_run_usd.is_some() || f.max_day_usd.is_some() || !f.price_overrides.is_empty() {
            cfg.spend_gate = Some(Arc::new(finops::CoreSpendGate::new(
                &self.inner.sessions_home,
                f,
            )));

            // MEA substrate (Phase H): auditor sees the workspace delta between
            // this run's start checkpoint and the live tree.
            {
                let home = self.sessions_home();
                let sid = session
                    .header()
                    .map(|h| h.session_id.clone())
                    .unwrap_or_default();
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
        cfg.permission = Some(match permission {
            Some(p) => p,
            None => std::sync::Arc::new(build_engine_for_mode(
                &self.inner.config,
                &self.extra_allow_snapshot(),
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
        if let Some(h) = session.header()
            && h.contract.route_ladder.len() > 1
        {
            for leg in h.contract.route_ladder.iter().skip(1) {
                if leg.provider == h.contract.provider {
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
        if self.inner.config.subagents
            && let Some(parent_id) = session.header().map(|h| h.session_id.clone())
        {
            tools.push(Arc::new(vak_agent::TaskTool::new(vak_agent::TaskDeps {
                provider: provider.clone(),
                system_prompt: self.system_prompt(),
                model: self.effective_model(),
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
        let mcp_cfg = self.effective_mcp();
        if !mcp_cfg.servers.is_empty() {
            let servers = mcp_cfg
                .servers
                .iter()
                .filter_map(|(name, s)| {
                    // ${VAR} in env values resolves through the standard
                    // secret path (runtime override → process env →
                    // .env files), so keys stay out of config.toml.
                    // Unresolved references skip the pair rather than
                    // handing the server a literal "${...}".
                    // Fail closed: one unresolved reference drops the whole
                    // server rather than starting it half-configured.
                    let mut env = Vec::with_capacity(s.env.len());
                    for (k, v) in &s.env {
                        match interpolate_env_var(v) {
                            Some(resolved) => env.push((k.clone(), resolved)),
                            None => return None,
                        }
                    }
                    Some((
                        name.clone(),
                        vak_mcp::ServerConfig {
                            command: s.command.clone(),
                            args: s.args.clone(),
                            env,
                            network: s.network,
                        },
                    ))
                })
                .collect();
            let manager = Arc::new(vak_mcp::McpManager::new_sandboxed(
                servers,
                self.inner.cwd.clone(),
                self.build_sandbox(),
            ));
            // Advertise MCP capabilities in the prompt so the model reaches
            // for them unprompted (first-turn usability). Inventory is
            // fetched once per process and cached; failures degrade to a
            // bare server-name hint.
            // A cold npx fetch can outlive the budget; an empty snapshot is
            // NOT cached so the next run retries.
            let fetched =
                tokio::time::timeout(std::time::Duration::from_secs(20), manager.inventory())
                    .await
                    .unwrap_or_default();
            let section = mcp_section(&fetched);
            if !fetched.is_empty() {
                let mut cache = self
                    .inner
                    .mcp_inventory
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner);
                if cache.is_none() {
                    *cache = Some(section.clone());
                }
            }
            let cached = self
                .inner
                .mcp_inventory
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .clone();
            if let Some(sec) = cached {
                cfg.system_prompt.push_str(&sec);
            }
            tools.push(Arc::new(vak_mcp::McpTool::new(manager)));
        }
        if self.inner.config.memory.search_enabled {
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
        // Learning loop (docs/design/26-learning.md): journaling tools are
        // ordinary model-visible tools; promotion stays human-only.
        let current_session = session
            .header()
            .map(|h| h.session_id.clone())
            .unwrap_or_default();
        if self.inner.config.memory.write_enabled {
            tools.push(Arc::new(learning::RememberTool {
                sessions_home: self.sessions_home(),
                cwd: self.inner.cwd.clone(),
                session_id: current_session.clone(),
            }));
        }
        if self.inner.config.memory.skill_proposals {
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
        cfg.tools = tools;
        let hooks: Option<std::sync::Arc<Vec<vak_hooks::HookDef>>> = Some(std::sync::Arc::new(
            build_hooks_from(&self.effective_hooks())?,
        ));
        cfg.hooks = hooks.clone();

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
            let outcome = vak_hooks::run_hooks(
                hook_defs.clone(),
                vak_hooks::HookEvent::SessionStart,
                &session_id,
                &self.inner.cwd,
                None,
                None,
                &cancel,
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
        if !self.config().memory.reflection {
            return reflection::ReflectionOutcome::Skipped {
                reason: "reflection-disabled",
            };
        }
        if !self.config().memory.write_enabled {
            return reflection::ReflectionOutcome::Skipped {
                reason: "memory-writes-disabled",
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
        let f = &self.inner.config.finops;
        if f.max_run_usd.is_some() || f.max_day_usd.is_some() || !f.price_overrides.is_empty() {
            let gate = finops::CoreSpendGate::new(&self.sessions_home(), f);
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
            if vak_agent::SpendGate::authorize(&gate, &check)
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

fn self_path() -> std::path::PathBuf {
    std::path::PathBuf::from(".vakcoder/config.toml")
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

/// Compact capability section from an MCP inventory snapshot.
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
    use super::mcp_section;

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
    fn empty_inventory_is_silent_but_named_servers_listed() {
        assert_eq!(mcp_section(&[]), "");
        let s = mcp_section(&[("x".into(), vec![])]);
        assert!(s.contains("- x: (no tools)"));
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
