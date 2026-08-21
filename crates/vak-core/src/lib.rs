//! vak-core: the SDK facade. Composes config, provider, tools, session and
//! the agent loop behind one entry point. TUI, server, and exec mode are
//! thin consumers of this crate.

pub mod skills;

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

    pub fn tool_names(&self) -> Vec<String> {
        let mut names: Vec<String> = vak_tools::default_tools()
            .iter()
            .map(|t| t.name().to_string())
            .collect();
        if self.inner.config.subagents {
            names.push("task".into());
        }
        names
    }

    fn provider_auth(&self) -> Result<ProviderAuth, CoreError> {
        let provider = self.effective_provider();
        match provider.as_str() {
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
                let api_key = std::env::var(env).map_err(|_| CoreError::MissingAuth {
                    env: env.into(),
                    provider,
                })?;
                Ok(ProviderAuth {
                    api_key,
                    base_url: std::env::var(override_env)
                        .ok()
                        .or_else(|| Some(default_base.into())),
                })
            }
            "ollama" => Ok(ProviderAuth {
                api_key: "ollama".into(),
                base_url: std::env::var("VAKCODER_OLLAMA_BASE_URL")
                    .ok()
                    .or_else(|| Some("http://localhost:11434/v1".into())),
            }),
            other => Err(CoreError::MissingAuth {
                env: format!("(no auth wiring for '{other}' yet)"),
                provider: other.into(),
            }),
        }
    }

    pub fn provider(&self) -> Result<Arc<dyn Provider>, CoreError> {
        let auth = self.provider_auth()?;
        Ok(self.inner.registry.get(&self.effective_provider(), &auth)?)
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
        cfg.approver = approver.clone();
        cfg.mode = match self.effective_permission_mode() {
            vak_config::PermissionMode::ReadOnly => vak_permission::Mode::ReadOnly,
            vak_config::PermissionMode::WorkspaceWrite => vak_permission::Mode::WorkspaceWrite,
            vak_config::PermissionMode::FullAccess => vak_permission::Mode::FullAccess,
        };
        cfg.permission = Some(match permission {
            Some(p) => p,
            None => std::sync::Arc::new(build_engine(&self.inner.config)?),
        });
        let Some(engine) = cfg.permission.clone() else {
            return Err(CoreError::MissingEngine);
        };
        cfg.sandbox = self.build_sandbox();

        let mut tools = vak_tools::default_tools();
        if self.inner.config.subagents
            && let Some(parent_id) = session.header().map(|h| h.session_id.clone())
        {
            tools.push(Arc::new(vak_agent::TaskTool::new(vak_agent::TaskDeps {
                provider: provider.clone(),
                system_prompt: self.system_prompt(),
                model: self.effective_model(),
                tools: vak_tools::default_tools(),
                max_turns: self.effective_max_turns(),
                permission: Some(engine),
                mode: cfg.mode,
                approver,
                sandbox: self.build_sandbox(),
                cwd: self.inner.cwd.clone(),
                sessions_home: self.inner.sessions_home.clone(),
                parent_session_id: parent_id,
            })));
        }
        cfg.tools = tools;
        cfg.hooks = Some(Arc::new(build_hooks(&self.inner.config)?));

        let steering = vak_agent::SteeringQueues::new();
        let mut agent = Agent::new(provider, session, cfg);
        Ok(agent.run(prompt, &steering, cancel, events).await)
    }

    fn build_sandbox(&self) -> Option<std::sync::Arc<dyn vak_tools::sandbox::Sandbox>> {
        #[cfg(target_os = "macos")]
        {
            use vak_tools::sandbox::{SandboxMode, Seatbelt};
            let mode = match self.effective_permission_mode() {
                vak_config::PermissionMode::ReadOnly => SandboxMode::ReadOnly,
                vak_config::PermissionMode::WorkspaceWrite => SandboxMode::WorkspaceWrite,
                vak_config::PermissionMode::FullAccess => return None,
            };
            Some(std::sync::Arc::new(Seatbelt::new(
                mode,
                self.inner.cwd.as_path(),
            )))
        }
        #[cfg(not(target_os = "macos"))]
        {
            None
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

fn self_path() -> std::path::PathBuf {
    std::path::PathBuf::from(".vakcoder/config.toml")
}
