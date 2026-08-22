//! vak-tools: built-in agent tools behind one trait.
//!
//! Contract: tools never panic and never return Err; failures are
//! ToolOutput::error values fed back to the model for self-correction.

pub mod bash;
pub mod context;
pub mod edit;
pub mod glob;
pub mod grep;
#[cfg(target_os = "linux")]
pub mod landlock;
pub mod read;
pub mod sandbox;
pub mod write;

use async_trait::async_trait;
use serde_json::Value;

pub use context::{OutputLimits, ToolContext};

#[derive(Debug, Clone)]
pub struct ToolOutput {
    pub content: String,
    pub is_error: bool,
}

impl ToolOutput {
    pub fn ok(content: impl Into<String>) -> Self {
        ToolOutput {
            content: content.into(),
            is_error: false,
        }
    }

    pub fn error(content: impl Into<String>) -> Self {
        ToolOutput {
            content: content.into(),
            is_error: true,
        }
    }
}

#[async_trait]
pub trait Tool: Send + Sync {
    fn name(&self) -> &str;
    fn description(&self) -> &str;
    fn schema(&self) -> Value;

    async fn execute(&self, args: &Value, ctx: &ToolContext) -> ToolOutput;

    /// What this call needs from the world while it runs. The default is
    /// unclaimed: schedulers may run it alongside anything.
    fn claims(&self, _args: &Value) -> ResourceClaims {
        ResourceClaims::default()
    }
}

/// Declared concurrent-execution constraints for one tool invocation.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct ResourceClaims {
    /// Conflicts with every other claimed call (unknown or whole-world scope).
    pub exclusive: bool,
    /// Never conflicts; safe to fan out freely.
    pub read_only: bool,
    /// Path scopes the call will touch (globs; `src/**` style).
    pub paths: Vec<String>,
}

impl ResourceClaims {
    pub fn is_unclaimed(&self) -> bool {
        !self.exclusive && !self.read_only && self.paths.is_empty()
    }

    /// Conservative prefix test after normalization: two write scopes
    /// conflict when one contains the other. May over-serialize; never
    /// under-serializes.
    pub fn conflicts(&self, other: &ResourceClaims) -> bool {
        if self.is_unclaimed() || other.is_unclaimed() || self.read_only || other.read_only {
            return false;
        }
        if self.exclusive || other.exclusive {
            return true;
        }
        for a in &self.paths {
            let na = normalize_scope(a);
            for b in &other.paths {
                let nb = normalize_scope(b);
                if na.starts_with(&nb) || nb.starts_with(&na) {
                    return true;
                }
            }
        }
        false
    }
}

fn normalize_scope(scope: &str) -> String {
    let mut s = scope.trim().trim_end_matches(['/', '*']).to_string();
    while s.ends_with('/') {
        s.pop();
    }
    if !s.ends_with('/') {
        s.push('/');
    }
    s
}

pub fn default_tools() -> Vec<std::sync::Arc<dyn Tool>> {
    vec![
        std::sync::Arc::new(read::ReadTool),
        std::sync::Arc::new(write::WriteTool),
        std::sync::Arc::new(edit::EditTool),
        std::sync::Arc::new(bash::BashTool),
        std::sync::Arc::new(glob::GlobTool),
        std::sync::Arc::new(grep::GrepTool),
    ]
}

/// Read/glob/grep subset for explore-style subagents.
pub fn read_only_tools() -> Vec<std::sync::Arc<dyn Tool>> {
    vec![
        std::sync::Arc::new(read::ReadTool),
        std::sync::Arc::new(glob::GlobTool),
        std::sync::Arc::new(grep::GrepTool),
    ]
}

pub fn definitions(tools: &[std::sync::Arc<dyn Tool>]) -> Vec<vak_llm::ToolDefinition> {
    tools
        .iter()
        .map(|t| vak_llm::ToolDefinition::new(t.name(), t.description(), t.schema()))
        .collect()
}
