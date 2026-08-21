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
}

pub struct Core {
    pub config: vak_config::Config,
    pub cwd: PathBuf,
    pub sessions_home: PathBuf,
    registry: ProviderRegistry,
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
            config,
            cwd,
            sessions_home,
            registry: default_registry(),
        })
    }

    pub fn system_prompt(&self) -> String {
        let project_prompt = self.cwd.join(".vakcoder/SYSTEM.md");
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
        match self.config.provider.as_str() {
            "anthropic" => {
                let api_key =
                    std::env::var("ANTHROPIC_API_KEY").map_err(|_| CoreError::MissingAuth {
                        env: "ANTHROPIC_API_KEY".into(),
                        provider: "anthropic".into(),
                    })?;
                Ok(ProviderAuth {
                    api_key,
                    base_url: self
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
        Ok(self.registry.get(&self.config.provider, &auth)?)
    }

    pub async fn start_session(&self) -> Result<SessionLog, CoreError> {
        let session_id = uuid_like();
        let path =
            vak_session::SessionPath::new_session_file(&self.sessions_home, &self.cwd, &session_id);
        let header = SessionHeader {
            session_id,
            created_at: chrono::Utc::now(),
            cwd: self.cwd.clone(),
            parent_session_id: None,
            contract: FrozenContract {
                app_version: APP_VERSION.into(),
                provider: self.config.provider.clone(),
                model: self.config.model.clone(),
                system_prompt: self.system_prompt(),
                tools: self.tool_names(),
                permission_mode: format!("{:?}", self.config.permission_mode).to_kebab_lowercase(),
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
        let provider = self.provider()?;
        let mut cfg = AgentConfig::new(self.system_prompt());
        cfg.tools = vak_tools::default_tools();
        cfg.max_turns = self.config.max_turns;
        cfg.parallel_tools = true;
        let mut agent = Agent::new(provider, session, cfg);
        Ok(agent.run(prompt, cancel, events).await)
    }
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
