//! vak-tools: built-in agent tools behind one trait.
//!
//! Contract: tools never panic and never return Err; failures are
//! ToolOutput::error values fed back to the model for self-correction.

pub mod bash;
pub mod context;
pub mod edit;
pub mod glob;
pub mod grep;
pub mod read;
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

pub fn definitions(tools: &[std::sync::Arc<dyn Tool>]) -> Vec<vak_llm::ToolDefinition> {
    tools
        .iter()
        .map(|t| vak_llm::ToolDefinition::new(t.name(), t.description(), t.schema()))
        .collect()
}
