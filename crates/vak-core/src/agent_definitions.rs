//! Saved Agent definitions: the durable, user-edited description of a custom
//! Agent, read from `.vak/agents.json` in the Shared layer and, when the
//! project is trusted, the project layer.
//!
//! A definition describes presentation and prompt preferences. It never
//! grants tools, permissions, credentials, budget or a wider execution
//! scope. A turn resolves the Agent's identity from here every time, so an
//! edit reaches the next turn of every conversation that Agent owns
//! (docs/design/45-prompt-layers.md, *Per-turn resolution*); the session
//! header keeps the identity as admitted, for display and audit.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum AgentLifecycle {
    Active,
    Paused,
    Archived,
}

fn default_lifecycle() -> AgentLifecycle {
    AgentLifecycle::Active
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentDefinition {
    pub id: String,
    #[serde(default = "default_revision")]
    pub revision: u64,
    #[serde(default = "default_lifecycle")]
    pub lifecycle: AgentLifecycle,
    pub name: String,
    pub character: String,
    pub personality: String,
    pub behaviour: String,
    #[serde(default)]
    pub responsibilities: String,
    #[serde(default)]
    pub instructions: String,
    pub animation: String,
    pub voice: String,
}

fn default_revision() -> u64 {
    1
}

impl AgentDefinition {
    pub fn is_admissible(&self) -> bool {
        self.lifecycle == AgentLifecycle::Active
    }

    pub fn identity(&self) -> vak_session::types::AgentIdentity {
        vak_session::types::AgentIdentity {
            id: self.id.clone(),
            revision: self.revision,
            name: self.name.clone(),
            character: self.character.clone(),
            personality: self.personality.clone(),
            animation: self.animation.clone(),
            voice: self.voice.clone(),
            behaviour: self.behaviour.clone(),
            responsibilities: self.responsibilities.clone(),
            instructions: self.instructions.clone(),
        }
    }
}

pub fn effective(core: &crate::Core) -> Result<Vec<AgentDefinition>, String> {
    let shared = vak_config::paths::default_workspace();
    let mut profiles = load(&shared)?;
    if core.cwd() != &shared && core.project_config_trusted() {
        for profile in load(core.cwd())? {
            profiles.retain(|p| p.id != profile.id);
            profiles.push(profile);
        }
    }
    Ok(profiles)
}

pub fn path(cwd: &Path) -> PathBuf {
    cwd.join(".vak").join("agents.json")
}

pub fn load(cwd: &Path) -> Result<Vec<AgentDefinition>, String> {
    let file = path(cwd);
    let raw = match std::fs::read_to_string(&file) {
        Ok(raw) => raw,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(e) => return Err(format!("cannot read {}: {e}", file.display())),
    };
    serde_json::from_str(&raw).map_err(|e| format!("invalid agents: {e}"))
}
