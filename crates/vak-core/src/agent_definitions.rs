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
    vak_config::scope::WorkspaceScope::new(cwd).agents_file()
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

/// The workspace whose project layer defines `agent_id` for a turn running in
/// `cwd`. A custom Agent's `Core` runs in its own workspace
/// (`<base>/.vak/agents/<id>/workspace`, `vak_config::paths::agent_workspace`)
/// but is defined in the project layer of `<base>`, so the base is where its
/// definition is. Any other `cwd` is its own base.
fn definition_base(cwd: &Path, agent_id: &str) -> PathBuf {
    let tail = Path::new(vak_config::scope::PROJECT_DIR)
        .join("agents")
        .join(agent_id)
        .join("workspace");
    if cwd.ends_with(&tail) {
        cwd.ancestors().nth(4).unwrap_or(cwd).to_path_buf()
    } else {
        cwd.to_path_buf()
    }
}

/// The saved definition of one Agent as a turn running in `core` sees it:
/// the Shared layer, then the project layer of the Agent's base workspace
/// when that project is trusted, the project layer winning like
/// [`effective`]. `None` means no layer this `Core` can read defines it,
/// which is not proof the Agent was deleted.
pub fn definition(core: &crate::Core, agent_id: &str) -> Result<Option<AgentDefinition>, String> {
    let shared = vak_config::paths::default_workspace();
    let mut found = load(&shared)?.into_iter().find(|d| d.id == agent_id);
    let base = definition_base(core.cwd(), agent_id);
    // An Agent's own workspace is trusted only when the base it lives under
    // was trusted when the Agent was opened (`pinned_core_for_workspace`), so
    // a trusted Agent `Core` vouches for its base even where the base's trust
    // lives in memory (`serve --trust`) rather than in the trust store.
    let trusted =
        core.project_config_trusted() || (&base != core.cwd() && crate::trust::is_trusted(&base));
    if base != shared
        && trusted
        && let Some(project) = load(&base)?.into_iter().find(|d| d.id == agent_id)
    {
        found = Some(project);
    }
    Ok(found)
}
