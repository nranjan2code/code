//! vak-core: the SDK facade. Composes config, provider, tools, session and
//! the agent loop behind one entry point. TUI, server, and exec mode are
//! thin consumers of this crate.

use std::path::PathBuf;
use std::sync::Arc;

use tokio_util::sync::CancellationToken;
use vak_agent::{Agent, AgentConfig, AgentEvent, TurnOutcome};
use vak_llm::Provider;
use vak_llm::registry::{ProviderAuth, ProviderRegistry, default_registry};
use vak_session::SessionLog;
use vak_session::types::{FrozenContract, SessionHeader};

pub const APP_VERSION: &str = env!("CARGO_PKG_VERSION");
pub const DEFAULT_SYSTEM_PROMPT: &str = include_str!("system-prompt.md");

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
}

#[derive(Clone)]
pub struct Core {
    inner: Arc<CoreInner>,
}

impl Core {
    pub fn new(cwd: PathBuf) -> Result<Self, CoreError> {
        let config = vak_config::load(&cwd)?;
        let sessions_home = std::env::var("VAKCODER_HOME")
            .map(PathBuf::from)
            .unwrap_or_else(|_| {
                std::env::var_os("HOME")
                    .map(|h| PathBuf::from(h).join(".vakcoder"))
                    .unwrap_or_else(|| cwd.join(".vakcoder"))
            });
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

    pub fn set_max_turns(&self, max_turns: usize) {
        if let Ok(mut c) = self.inner.max_turns_override.lock() {
            *c = Some(max_turns);
        }
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

    pub fn sessions_home(&self) -> &PathBuf {
        &self.inner.sessions_home
    }

    pub fn system_prompt(&self) -> String {
        let project_prompt = self.inner.cwd.join(".vakcoder/SYSTEM.md");
        if project_prompt.is_file()
            && let Ok(custom) = std::fs::read_to_string(&project_prompt)
        {
            return custom;
        }
        DEFAULT_SYSTEM_PROMPT.replace("{{version}}", APP_VERSION)
    }

    pub fn tool_names(&self) -> Vec<String> {
        vak_tools::default_tools()
            .iter()
            .map(|t| t.name().to_string())
            .collect()
    }

    fn provider_auth(&self) -> Result<ProviderAuth, CoreError> {
        match self.inner.config.provider.as_str() {
            "anthropic" => {
                let api_key =
                    std::env::var("ANTHROPIC_API_KEY").map_err(|_| CoreError::MissingAuth {
                        env: "ANTHROPIC_API_KEY".into(),
                        provider: "anthropic".into(),
                    })?;
                Ok(ProviderAuth {
                    api_key,
                    base_url: self
                        .inner
                        .config
                        .anthropic_base_url
                        .clone()
                        .or_else(|| std::env::var("VAKCODER_ANTHROPIC_BASE_URL").ok()),
                })
            }
            other => Err(CoreError::MissingAuth {
                env: format!("(no auth wiring for '{other}' yet)"),
                provider: other.into(),
            }),
        }
    }

    pub fn provider(&self) -> Result<Arc<dyn Provider>, CoreError> {
        let auth = self.provider_auth()?;
        Ok(self
            .inner
            .registry
            .get(&self.inner.config.provider, &auth)?)
    }

    pub async fn start_session(&self) -> Result<SessionLog, CoreError> {
        let session_id = uuid_like();
        let path = vak_session::SessionPath::new_session_file(
            &self.inner.sessions_home,
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
        self.run_turn_with(session, prompt, cancel, None, None, events)
            .await
    }

    pub async fn run_turn_with(
        &self,
        session: SessionLog,
        prompt: &str,
        cancel: CancellationToken,
        approver: Option<std::sync::Arc<dyn vak_agent::Approver>>,
        permission: Option<std::sync::Arc<vak_permission::PermissionEngine>>,
        events: tokio::sync::mpsc::Sender<AgentEvent>,
    ) -> Result<TurnOutcome, CoreError> {
        let provider = self.provider()?;
        let mut cfg = AgentConfig::new(self.system_prompt());
        cfg.model = self.effective_model();
        cfg.tools = vak_tools::default_tools();
        cfg.max_turns = self.effective_max_turns();
        cfg.parallel_tools = true;
        cfg.approver = approver;
        cfg.mode = match self.effective_permission_mode() {
            vak_config::PermissionMode::ReadOnly => vak_permission::Mode::ReadOnly,
            vak_config::PermissionMode::WorkspaceWrite => vak_permission::Mode::WorkspaceWrite,
            vak_config::PermissionMode::FullAccess => vak_permission::Mode::FullAccess,
        };
        cfg.permission = Some(match permission {
            Some(p) => p,
            None => std::sync::Arc::new(build_engine(&self.inner.config)?),
        });
        let steering = vak_agent::SteeringQueues::new();
        let mut agent = Agent::new(provider, session, cfg);
        Ok(agent.run(prompt, &steering, cancel, events).await)
    }
}

pub fn build_engine(
    config: &vak_config::Config,
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

fn uuid_like() -> String {
    use std::time::{SystemTime, UNIX_EPOCH};
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    format!("{nanos:032x}")
}
