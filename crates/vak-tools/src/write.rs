use async_trait::async_trait;
use serde_json::Value;

use crate::{Tool, ToolContext, ToolOutput};

pub struct WriteTool;

#[async_trait]
impl Tool for WriteTool {
    fn name(&self) -> &str {
        "write"
    }

    fn description(&self) -> &str {
        "Write content to a file, creating parent directories as needed. Overwrites the file if it exists."
    }

    fn schema(&self) -> Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "path": {"type": "string", "description": "File path (relative to cwd or absolute)"},
                "content": {"type": "string", "description": "Full file content to write"}
            },
            "required": ["path", "content"]
        })
    }

    async fn execute(&self, args: &Value, ctx: &ToolContext) -> ToolOutput {
        let Some(path_str) = args.get("path").and_then(|p| p.as_str()) else {
            return ToolOutput::error("missing required parameter: path");
        };
        let Some(content) = args.get("content").and_then(|c| c.as_str()) else {
            return ToolOutput::error("missing required parameter: content");
        };
        let path = ctx.resolve(std::path::Path::new(path_str));

        if let Some(parent) = path.parent()
            && let Err(e) = tokio::fs::create_dir_all(parent).await
        {
            return ToolOutput::error(format!("cannot create directory {}: {e}", parent.display()));
        }
        match tokio::fs::write(&path, content).await {
            Ok(_) => ToolOutput::ok(format!(
                "wrote {} bytes to {}",
                content.len(),
                path.display()
            )),
            Err(e) => ToolOutput::error(format!("cannot write {}: {e}", path.display())),
        }
    }
}
