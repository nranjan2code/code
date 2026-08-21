use std::sync::Arc;

use async_trait::async_trait;
use serde_json::Value;

use vak_tools::{Tool, ToolContext, ToolOutput};

use crate::manager::McpManager;

/// One meta-tool exposing every configured MCP server without dumping tool
/// descriptions into context. The model lists on demand, then calls.
pub struct McpTool {
    manager: Arc<McpManager>,
}

impl McpTool {
    pub fn new(manager: Arc<McpManager>) -> Self {
        McpTool { manager }
    }
}

#[async_trait]
impl Tool for McpTool {
    fn name(&self) -> &str {
        "mcp"
    }

    fn description(&self) -> &str {
        "Call tools exposed by configured MCP servers. Use action \"list\" to discover servers and their tools (compact), then action \"call\" with server, tool, and arguments."
    }

    fn schema(&self) -> Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "action": {"type": "string", "enum": ["list", "call"], "description": "list servers+tools, or call one"},
                "server": {"type": "string", "description": "Server name (required for call)"},
                "tool": {"type": "string", "description": "Tool name (required for call)"},
                "arguments": {"type": "object", "description": "Arguments object for the tool (call only)"}
            },
            "required": ["action"]
        })
    }

    async fn execute(&self, args: &Value, _ctx: &ToolContext) -> ToolOutput {
        match args.get("action").and_then(|a| a.as_str()) {
            Some("list") => self.list().await,
            Some("call") => self.call(args).await,
            Some(other) => ToolOutput::error(format!("unknown mcp action '{other}'")),
            None => ToolOutput::error("missing required parameter: action"),
        }
    }
}

impl McpTool {
    async fn list(&self) -> ToolOutput {
        let mut out = String::new();
        for server in self.manager.server_names() {
            out.push_str(&format!("{server}:\n"));
            match self.manager.get(&server).await {
                Ok(client) => match client.list_tools().await {
                    Ok(tools) if tools.is_empty() => out.push_str("  (no tools)\n"),
                    Ok(tools) => {
                        for t in tools {
                            let desc: String = t.description.chars().take(100).collect();
                            out.push_str(&format!("  {} — {desc}\n", t.name));
                        }
                    }
                    Err(e) => out.push_str(&format!("  error: {e}\n")),
                },
                Err(e) => out.push_str(&format!("  connect failed: {e}\n")),
            }
        }
        if out.is_empty() {
            return ToolOutput::ok("no MCP servers configured");
        }
        ToolOutput::ok(out)
    }

    async fn call(&self, args: &Value) -> ToolOutput {
        let Some(server) = args.get("server").and_then(|s| s.as_str()) else {
            return ToolOutput::error("missing required parameter: server");
        };
        let Some(tool) = args.get("tool").and_then(|t| t.as_str()) else {
            return ToolOutput::error("missing required parameter: tool");
        };
        let arguments = args
            .get("arguments")
            .cloned()
            .unwrap_or(Value::Object(Default::default()));

        let client = match self.manager.get(server).await {
            Ok(c) => c,
            Err(e) => return ToolOutput::error(format!("cannot connect to '{server}': {e}")),
        };
        match client.call_tool(tool, arguments).await {
            Ok(text) => {
                if text.is_empty() {
                    ToolOutput::ok("(empty result)")
                } else {
                    ToolOutput::ok(text)
                }
            }
            Err(e) => ToolOutput::error(format!("mcp call failed: {e}")),
        }
    }
}
