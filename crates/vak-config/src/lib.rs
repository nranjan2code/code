//! vak-config: layered configuration. Defaults < global file < project file
//! < environment. Unknown keys are ignored with a warning, never fatal.

use std::path::{Path, PathBuf};

use serde::Deserialize;

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, serde::Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum PermissionMode {
    ReadOnly,
    #[default]
    WorkspaceWrite,
    FullAccess,
}

impl PermissionMode {
    pub fn deserialize_str(s: &str) -> Option<PermissionMode> {
        match s {
            "read-only" | "readonly" => Some(PermissionMode::ReadOnly),
            "workspace-write" => Some(PermissionMode::WorkspaceWrite),
            "full-access" | "fullaccess" => Some(PermissionMode::FullAccess),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default)]
pub struct Profile {
    pub model: Option<String>,
    pub provider: Option<String>,
    pub permission_mode: Option<PermissionMode>,
    pub max_turns: Option<usize>,
}

#[derive(Debug, Clone, Deserialize, Default)]
#[serde(default)]
pub struct FileConfig {
    pub provider: Option<String>,
    pub model: Option<String>,
    pub max_tokens: Option<u32>,
    pub max_turns: Option<usize>,
    pub permission_mode: Option<PermissionMode>,
    pub profile: Option<String>,
    pub profiles: std::collections::BTreeMap<String, Profile>,
    pub anthropic_base_url: Option<String>,
    #[serde(default)]
    pub allow: Vec<String>,
    #[serde(default)]
    pub ask: Vec<String>,
    #[serde(default)]
    pub deny: Vec<String>,
    pub subagents: Option<bool>,
    #[serde(default)]
    pub hooks: Vec<HookConfig>,
    #[serde(default)]
    pub mcp: McpConfig,
}

#[derive(Debug, Clone, Deserialize, Default)]
pub struct McpConfig {
    #[serde(default)]
    pub servers: std::collections::BTreeMap<String, McpServerConfig>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct McpServerConfig {
    pub command: String,
    #[serde(default)]
    pub args: Vec<String>,
    #[serde(default)]
    pub env: std::collections::BTreeMap<String, String>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct HookConfig {
    pub event: String,
    #[serde(rename = "match")]
    pub matcher: Option<String>,
    pub command: String,
    pub timeout_ms: Option<u64>,
}

#[derive(Debug, Clone)]
pub struct Config {
    pub provider: String,
    pub model: String,
    pub max_tokens: u32,
    pub max_turns: usize,
    pub permission_mode: PermissionMode,
    pub anthropic_base_url: Option<String>,
    pub allow: Vec<String>,
    pub ask: Vec<String>,
    pub deny: Vec<String>,
    pub subagents: bool,
    pub hooks: Vec<HookConfig>,
    pub mcp: McpConfig,
    pub warnings: Vec<String>,
}

impl Default for Config {
    fn default() -> Self {
        Config {
            provider: "anthropic".into(),
            model: "claude-sonnet-4-5".into(),
            max_tokens: 8192,
            max_turns: 40,
            permission_mode: PermissionMode::WorkspaceWrite,
            anthropic_base_url: None,
            allow: Vec::new(),
            ask: Vec::new(),
            deny: Vec::new(),
            subagents: true,
            hooks: Vec::new(),
            mcp: McpConfig::default(),
            warnings: Vec::new(),
        }
    }
}

#[derive(Debug, thiserror::Error)]
pub enum ConfigError {
    #[error("cannot read config file {path}: {source}")]
    Read {
        path: PathBuf,
        source: std::io::Error,
    },
    #[error("cannot parse config file {path}: {source}")]
    Parse {
        path: PathBuf,
        source: toml::de::Error,
    },
}

pub fn global_path() -> Option<PathBuf> {
    dirs_home().map(|h| h.join(".config/vakcoder/config.toml"))
}

pub fn project_path(cwd: &Path) -> PathBuf {
    cwd.join(".vakcoder/config.toml")
}

fn dirs_home() -> Option<PathBuf> {
    std::env::var_os("HOME").map(PathBuf::from)
}

pub fn load(cwd: &Path) -> Result<Config, ConfigError> {
    let warnings = Vec::new();
    let mut layers: Vec<FileConfig> = vec![FileConfig::default()];

    if let Some(gp) = global_path().filter(|gp| gp.is_file()) {
        layers.push(parse_file(&gp)?);
    }
    let pp = project_path(cwd);
    if pp.is_file() {
        layers.push(parse_file(&pp)?);
    }

    let mut merged = FileConfig::default();
    for layer in layers {
        merge_into(&mut merged, layer);
    }

    if let Ok(m) = std::env::var("VAKCODER_MODEL") {
        merged.model = Some(m);
    }
    if let Ok(p) = std::env::var("VAKCODER_PROVIDER") {
        merged.provider = Some(p);
    }

    let mut cfg = Config {
        warnings,
        ..Default::default()
    };
    if let Some(provider) = merged.provider {
        cfg.provider = provider;
    }
    if let Some(model) = merged.model {
        cfg.model = model;
    }
    if let Some(mt) = merged.max_tokens {
        cfg.max_tokens = mt;
    }
    if let Some(turns) = merged.max_turns {
        cfg.max_turns = turns;
    }
    if let Some(mode) = merged.permission_mode {
        cfg.permission_mode = mode;
    }
    cfg.anthropic_base_url = merged.anthropic_base_url;
    cfg.allow = merged.allow;
    cfg.ask = merged.ask;
    cfg.deny = merged.deny;
    if let Some(sa) = merged.subagents {
        cfg.subagents = sa;
    }
    cfg.hooks = merged.hooks;
    for (name, srv) in merged.mcp.servers {
        cfg.mcp.servers.insert(name, srv);
    }

    if let Some(name) = &merged.profile {
        if let Some(profile) = merged.profiles.get(name) {
            if let Some(model) = &profile.model {
                cfg.model = model.clone();
            }
            if let Some(provider) = &profile.provider {
                cfg.provider = provider.clone();
            }
            if let Some(mode) = profile.permission_mode {
                cfg.permission_mode = mode;
            }
            if let Some(turns) = profile.max_turns {
                cfg.max_turns = turns;
            }
        } else {
            cfg.warnings.push(format!("profile '{name}' not defined"));
        }
    }

    Ok(cfg)
}

fn parse_file(path: &Path) -> Result<FileConfig, ConfigError> {
    let text = std::fs::read_to_string(path).map_err(|source| ConfigError::Read {
        path: path.to_path_buf(),
        source,
    })?;
    toml::from_str(&text).map_err(|source| ConfigError::Parse {
        path: path.to_path_buf(),
        source,
    })
}

fn merge_into(base: &mut FileConfig, over: FileConfig) {
    if over.provider.is_some() {
        base.provider = over.provider;
    }
    if over.model.is_some() {
        base.model = over.model;
    }
    if over.max_tokens.is_some() {
        base.max_tokens = over.max_tokens;
    }
    if over.max_turns.is_some() {
        base.max_turns = over.max_turns;
    }
    if over.permission_mode.is_some() {
        base.permission_mode = over.permission_mode;
    }
    if over.profile.is_some() {
        base.profile = over.profile;
    }
    if over.anthropic_base_url.is_some() {
        base.anthropic_base_url = over.anthropic_base_url;
    }
    for r in over.allow {
        if !base.allow.contains(&r) {
            base.allow.push(r);
        }
    }
    for r in over.ask {
        if !base.ask.contains(&r) {
            base.ask.push(r);
        }
    }
    for r in over.deny {
        if !base.deny.contains(&r) {
            base.deny.push(r);
        }
    }
    if over.subagents.is_some() {
        base.subagents = over.subagents;
    }
    base.hooks.extend(over.hooks);
    for (name, srv) in over.mcp.servers {
        base.mcp.servers.insert(name, srv);
    }
    for (k, v) in over.profiles {
        base.profiles.insert(k, v);
    }
}
