//! vak-core: the SDK facade. Composes config, provider, tools, session and
//! the agent loop behind one entry point. TUI, server, and exec mode are
//! thin consumers of this crate.

pub mod checkpoints;
pub mod custom_commands;
pub mod files;
pub mod learning;
pub mod memory;
pub mod sandbox_docker;
pub mod session_search;
pub mod skills;
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
        let sessions_home = vak_config::get_var("VAKCODER_HOME")
            .map(PathBuf::from)
            .unwrap_or_else(|| {
                std::env::var_os("HOME")
                    .map(|h| PathBuf::from(h).join(".vakcoder"))
                    .unwrap_or_else(|| cwd.join(".vakcoder"))
            });
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
            }),
        })
    }

    pub fn config(&self) -> &vak_config::Config {
        &self.inner.config
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

    pub fn set_theme(&self, theme: String) {
        if let Ok(mut t) = self.inner.theme_override.lock() {
            *t = Some(theme);
        }
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
        if !self.inner.config.mcp.servers.is_empty() {
            names.push("mcp".into());
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
    /// `~/.vakcoder/.env` (0600, shared by every surface) and registers it
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

    /// Revoke `provider`'s key: strip it from `~/.vakcoder/.env`, drop the
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

    pub async fn start_session(&self) -> Result<SessionLog, CoreError> {
        let session_id = uuid_like();
        let path = vak_session::SessionPath::new_session_file(
            &self.sessions_home(),
            &self.inner.cwd,
            &session_id,
        );
        let header = SessionHeader {
            session_id,
            created_at: chrono::Utc::now(),
            cwd: self.inner.cwd.clone(),
            parent_session_id: None,
            contract: FrozenContract {
                app_version: APP_VERSION.into(),
                provider: self.effective_provider(),
                model: self.effective_model(),
                system_prompt: self.system_prompt(),
                tools: self.tool_names(),
                permission_mode: format!("{:?}", self.inner.config.permission_mode)
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
        cfg.approver = approver.clone();
        cfg.mode = match self.effective_permission_mode() {
            vak_config::PermissionMode::ReadOnly => vak_permission::Mode::ReadOnly,
            vak_config::PermissionMode::WorkspaceWrite => vak_permission::Mode::WorkspaceWrite,
            vak_config::PermissionMode::FullAccess => vak_permission::Mode::FullAccess,
        };
        cfg.permission = Some(match permission {
            Some(p) => p,
            None => std::sync::Arc::new(build_engine_with(
                &self.inner.config,
                &self.extra_allow_snapshot(),
            )?),
        });
        let Some(engine) = cfg.permission.clone() else {
            return Err(CoreError::MissingEngine);
        };
        cfg.sandbox = self.build_sandbox();

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
        if !self.inner.config.mcp.servers.is_empty() {
            let servers = self
                .inner
                .config
                .mcp
                .servers
                .iter()
                .map(|(name, s)| {
                    (
                        name.clone(),
                        vak_mcp::ServerConfig {
                            command: s.command.clone(),
                            args: s.args.clone(),
                            env: s.env.iter().map(|(k, v)| (k.clone(), v.clone())).collect(),
                        },
                    )
                })
                .collect();
            let manager = Arc::new(vak_mcp::McpManager::new_sandboxed(
                servers,
                self.inner.cwd.clone(),
                self.build_sandbox(),
            ));
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
        cfg.tools = tools;
        let hooks: Option<std::sync::Arc<Vec<vak_hooks::HookDef>>> =
            Some(std::sync::Arc::new(build_hooks(&self.inner.config)?));
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
                &format!("turn: {prompt}"),
            ) {
                let _ = checkpoints::store(&self.sessions_home(), &cp);
            }
        }

        let steering = match steering {
            Some(s) => s,
            None => std::sync::Arc::new(vak_agent::SteeringQueues::new()),
        };
        let mut agent = Agent::new(provider, session, cfg);
        let outcome = agent.run(prompt, &steering, cancel, events).await;
        let session = agent.into_session().await;
        Ok((outcome, session))
    }

    fn next_checkpoint_seq(&self, session_id: &str) -> u32 {
        checkpoints::list(&self.sessions_home(), session_id)
            .map(|list| list.last().map(|c| c.seq + 1).unwrap_or(0))
            .unwrap_or(0)
    }

    fn build_sandbox(&self) -> Option<std::sync::Arc<dyn vak_tools::sandbox::Sandbox>> {
        let mode = match self.effective_permission_mode() {
            vak_config::PermissionMode::ReadOnly => SandboxMode::ReadOnly,
            vak_config::PermissionMode::WorkspaceWrite => SandboxMode::WorkspaceWrite,
            vak_config::PermissionMode::FullAccess => return None,
        };
        let backend = self.inner.config.sandbox.backend.as_str();
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
    Ok(vak_permission::PermissionEngine::from_rule_strings(&specs)?)
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
    let mut out = Vec::with_capacity(config.hooks.len());
    for h in &config.hooks {
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
