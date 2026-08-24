use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FlowDef {
    pub name: String,
    #[serde(default)]
    pub description: String,
    #[serde(default)]
    pub nodes: Vec<NodeDef>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct NodeDef {
    pub id: String,
    pub r#type: String,
    #[serde(default)]
    pub deps: Vec<String>,
    #[serde(default = "default_required")]
    pub required: bool,
    #[serde(default)]
    pub prompt: Option<String>,
    #[serde(default)]
    pub command: Option<String>,
    #[serde(default)]
    pub message: Option<String>,
    #[serde(default)]
    pub readonly: bool,
    #[serde(default)]
    pub paths: Vec<String>,
    #[serde(default)]
    pub timeout_ms: Option<u64>,
    /// Done-contract (docs/design/27 Phase H mechanism 5): shell checks
    /// that must pass AFTER the node succeeds. `verify:` prefix optional;
    /// every entry runs as a brokered bash command and must exit 0.
    #[serde(default)]
    pub accept: Vec<String>,
}

fn default_required() -> bool {
    true
}

impl NodeDef {
    /// The strings a `{{dep}}` substitution may pull from upstream outputs.
    pub fn uses_template(&self) -> bool {
        match self.r#type.as_str() {
            "agent" => self.prompt.as_deref().is_some_and(|p| p.contains("{{")),
            "bash" => self.command.as_deref().is_some_and(|c| c.contains("{{")),
            _ => false,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum NodeStatus {
    Pending,
    Running,
    Completed,
    Failed,
    Skipped,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NodeResult {
    pub status: NodeStatus,
    #[serde(default)]
    pub output: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FlowState {
    pub run_id: String,
    pub flow_name: String,
    /// Frozen definition (raw TOML) captured at first run.
    pub definition_toml: String,
    pub started_at: chrono::DateTime<chrono::Utc>,
    pub nodes: BTreeMap<String, NodeResult>,
}
