use std::sync::Arc;

use async_trait::async_trait;
use serde_json::Value;

use vak_tools::{Tool, ToolContext, ToolOutput};

use crate::manager::McpManager;

type InvocationRecorder = Arc<dyn Fn(&str, &str, bool, u64) + Send + Sync>;
type CatalogObserver = Arc<dyn Fn(&[(String, Vec<crate::McpToolInfo>)]) + Send + Sync>;

/// One meta-tool exposing every configured MCP server without dumping tool
/// descriptions into context. The model lists on demand, then calls.
pub struct McpTool {
    manager: Arc<McpManager>,
    allow: Option<Vec<String>>,
    deny: Vec<String>,
    invocation_recorder: Option<InvocationRecorder>,
    catalog_observer: Option<CatalogObserver>,
}

impl McpTool {
    pub fn new(manager: Arc<McpManager>) -> Self {
        McpTool {
            manager,
            allow: None,
            deny: Vec::new(),
            invocation_recorder: None,
            catalog_observer: None,
        }
    }

    pub fn with_policy(
        manager: Arc<McpManager>,
        allow: Option<Vec<String>>,
        deny: Vec<String>,
    ) -> Self {
        McpTool {
            manager,
            allow,
            deny,
            invocation_recorder: None,
            catalog_observer: None,
        }
    }

    pub fn with_policy_and_recorder(
        manager: Arc<McpManager>,
        allow: Option<Vec<String>>,
        deny: Vec<String>,
        recorder: InvocationRecorder,
    ) -> Self {
        Self {
            manager,
            allow,
            deny,
            invocation_recorder: Some(recorder),
            catalog_observer: None,
        }
    }

    pub fn with_catalog_observer(mut self, observer: CatalogObserver) -> Self {
        self.catalog_observer = Some(observer);
        self
    }

    fn matches(patterns: &[String], value: &str) -> bool {
        patterns.iter().any(|pattern| {
            globset::Glob::new(pattern)
                .ok()
                .is_some_and(|glob| glob.compile_matcher().is_match(value))
        })
    }

    fn allowed(&self, server: &str, tool: &str) -> bool {
        let value = format!("{server}/{tool}");
        !Self::matches(&self.deny, &value)
            && self
                .allow
                .as_ref()
                .is_none_or(|allow| Self::matches(allow, &value))
    }
}

#[async_trait]
impl Tool for McpTool {
    fn name(&self) -> &str {
        "mcp"
    }

    fn description(&self) -> &str {
        "Call tools exposed by configured MCP servers. Always use this broker (never call an MCP tool name directly). First use action \"list\"; it returns each exact tool name and inputSchema. Then use action \"call\" with server, tool, and arguments matching that schema exactly."
    }

    fn schema(&self) -> Value {
        // The contract is intentionally `oneOf` (not a flat `required`):
        // `server`/`tool`/`arguments` are only meaningful for `action == "call"`,
        // and `additionalProperties: false` on each branch keeps the model from
        // inventing shapes it cannot reach. This is the schema the model sees
        // during tool selection, so it must be precise — a model that trusts a
        // flat `required: ["action"]` will omit `server`/`tool` and guess.
        serde_json::json!({
            "type": "object",
            "oneOf": [
                {
                    "description": "List every configured MCP server and the tools it exposes (exact names + inputSchemas). Always call this first.",
                    "properties": {
                        "action": {"type": "string", "enum": ["list"]}
                    },
                    "required": ["action"],
                    "additionalProperties": false
                },
                {
                    "description": "Invoke one discovered tool. `server` must be a name returned by `action: list`; `tool` one of that server's tools.",
                    "properties": {
                        "action": {"type": "string", "enum": ["call"]},
                        "server": {"type": "string", "description": "Discovered server name"},
                        "tool": {"type": "string", "description": "Discovered tool name"},
                        "arguments": {"type": "object", "description": "Arguments matching the tool's inputSchema (call only)"}
                    },
                    "required": ["action", "server", "tool"],
                    "additionalProperties": false
                }
            ]
        })
    }

    async fn execute(&self, args: &Value, ctx: &ToolContext) -> ToolOutput {
        match args.get("action").and_then(|a| a.as_str()) {
            Some("list") => self.list(ctx).await,
            Some("call") => self.call(args, ctx).await,
            Some(other) => ToolOutput::error(format!("unknown mcp action '{other}'")),
            None => ToolOutput::error("missing required parameter: action"),
        }
    }
}

impl McpTool {
    async fn list(&self, ctx: &ToolContext) -> ToolOutput {
        let mut out = String::new();
        let mut catalog = Vec::new();
        for server in self.manager.server_names() {
            out.push_str(&format!("{server}:\n"));
            match self.manager.get(&server).await {
                Ok(client) => match client.list_tools().await {
                    Ok(tools) if tools.is_empty() => out.push_str("  (no tools)\n"),
                    Ok(tools) => {
                        let mut visible = Vec::new();
                        for t in tools {
                            if !self.allowed(&server, &t.name) {
                                continue;
                            }
                            let schema = self.manager.redact(
                                serde_json::to_string(&t.input_schema)
                                    .unwrap_or_else(|_| "{}".to_string()),
                            );
                            out.push_str(&format!(
                                "  {} — {}\n    inputSchema: {schema}\n",
                                t.name, t.description
                            ));
                            visible.push(t);
                        }
                        catalog.push((server.clone(), visible));
                    }
                    Err(e) => out.push_str(&format!(
                        "  error: {}\n",
                        self.manager.redact(e.to_string())
                    )),
                },
                Err(e) => out.push_str(&format!(
                    "  connect failed: {}\n",
                    self.manager.redact(e.to_string())
                )),
            }
        }
        if out.is_empty() {
            return ToolOutput::ok("no MCP servers configured");
        }
        if let Some(observer) = &self.catalog_observer {
            observer(&catalog);
        }
        ToolOutput::ok(ctx.truncate_output(out))
    }

    async fn call(&self, args: &Value, ctx: &ToolContext) -> ToolOutput {
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

        if !self.allowed(server, tool) {
            if let Some(record) = &self.invocation_recorder {
                record(server, tool, false, 0);
            }
            return ToolOutput::error(format!(
                "MCP capability denied by channel policy: {server}/{tool}"
            ));
        }

        let started = std::time::Instant::now();
        match self.manager.call_tool(server, tool, arguments).await {
            Ok(text) => {
                if let Some(record) = &self.invocation_recorder {
                    record(server, tool, true, started.elapsed().as_millis() as u64);
                }
                if text.is_empty() {
                    ToolOutput::ok("(empty result)")
                } else {
                    let redacted = self.manager.redact(text);
                    let preview = ctx.truncate_output(redacted.clone());
                    if preview == redacted {
                        return ToolOutput::ok(preview);
                    }
                    let artifact = self.manager.store_artifact(server, tool, &redacted);
                    match artifact {
                        Some(path) => ToolOutput::ok(ctx.truncate_output(format!(
                            "[full MCP result stored at {}. Use the read tool with this path and offset/limit for exact retrieval.]\n{redacted}",
                            path.display()
                        ))),
                        None => ToolOutput::ok(preview),
                    }
                }
            }
            Err(e) => {
                if let Some(record) = &self.invocation_recorder {
                    record(server, tool, false, started.elapsed().as_millis() as u64);
                }
                ToolOutput::error(ctx.truncate_output(format!(
                    "mcp call failed: {}",
                    self.manager.redact(e.to_string())
                )))
            }
        }
    }
}
