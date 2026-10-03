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
    /// The broker to every external integration. Claiming `live-data` is
    /// what lets a live-data reading reach a configured search server;
    /// servers refine it by declaring their own `serves`.
    pub const SERVES: &'static [&'static str] = &["live-data", "web", "documents", "messaging"];

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

    fn serves(&self) -> &'static [&'static str] {
        Self::SERVES
    }

    fn always_loaded(&self) -> bool {
        true
    }

    fn description(&self) -> &str {
        "Discover and call tools exposed by configured MCP servers. For a current fact or an unknown source URL, check configured servers for a search tool before fetching a page by URL. Always use this broker (never call an MCP tool name directly). Use action \"list\" with a server to get its exact tool names and inputSchemas, then action \"call\" with server, tool, and arguments matching that schema exactly."
    }

    fn schema(&self) -> Value {
        // OpenAI function declarations require a top-level object without
        // oneOf/anyOf. Keep the two actions explicit in descriptions, and
        // enforce call-only fields again in execute before touching a server.
        // The configured server names are declaration data, not a catalog
        // probe, so exposing them here preserves lazy startup while making a
        // `find_tools` result actionable for models that missed the separate
        // prompt inventory.
        let servers = self.reachable_servers();
        let server_schema = if servers.is_empty() {
            serde_json::json!({
                "type": "string",
                "description": "The configured server to list or call. Required for call."
            })
        } else {
            serde_json::json!({
                "type": "string",
                "enum": servers,
                "description": "A configured, policy-reachable server to list or call. Required for call."
            })
        };
        serde_json::json!({
            "type": "object",
            "properties": {
                "action": {"type": "string", "enum": ["list", "call"], "description": "list: a server's exact tool names and schemas (without a server, just the server names); call: invoke one tool."},
                "server": server_schema,
                "tool": {"type": "string", "description": "Required for call: a tool name returned by list for that server."},
                "arguments": {"type": "object", "description": "For call: arguments matching the discovered tool's inputSchema."}
            },
            "required": ["action"],
            "additionalProperties": false
        })
    }

    async fn execute(&self, args: &Value, _ctx: &ToolContext) -> ToolOutput {
        match args.get("action").and_then(|a| a.as_str()) {
            Some("list") => self.list(args).await,
            Some("call") => self.call(args).await,
            Some(other) => ToolOutput::error(format!("unknown mcp action '{other}'")),
            None => ToolOutput::error("missing required parameter: action"),
        }
    }
}

impl McpTool {
    /// Servers this turn may reach: configured, and not wholly denied.
    fn reachable_servers(&self) -> Vec<String> {
        self.manager
            .server_names()
            .into_iter()
            .filter(|server| {
                let may_be_allowed = self.allow.as_ref().is_none_or(|allow| {
                    allow.iter().any(|pattern| {
                        let server_pattern = pattern.split('/').next().unwrap_or_default();
                        globset::Glob::new(server_pattern)
                            .ok()
                            .is_some_and(|glob| glob.compile_matcher().is_match(server))
                    })
                });
                may_be_allowed && !Self::matches(&self.deny, &format!("{server}/*"))
            })
            .collect()
    }

    /// `list` without a server connects to nothing: it answers from what the
    /// pool already knows. With a server it is demand for exactly that one.
    async fn list(&self, args: &Value) -> ToolOutput {
        let servers = self.reachable_servers();
        if servers.is_empty() {
            return ToolOutput::ok("no MCP servers configured");
        }
        let Some(server) = args.get("server").and_then(|s| s.as_str()) else {
            let observed = self.manager.observations();
            let mut out =
                String::from("MCP servers (call list with a server to get its tool schemas):\n");
            for server in &servers {
                let known = observed.get(server);
                match known.and_then(|o| o.tools.as_ref()) {
                    Some(tools) => {
                        let names: Vec<&str> = tools
                            .iter()
                            .filter(|t| self.allowed(server, &t.name))
                            .map(|t| t.name.as_str())
                            .collect();
                        out.push_str(&format!("- {server}: {}\n", names.join(", ")));
                    }
                    None => out.push_str(&format!("- {server}\n")),
                }
                if let Some(failure) = known.and_then(|o| o.failure.as_deref()) {
                    out.push_str(&format!(
                        "  last attempt failed: {failure}. {}\n",
                        unavailable_guidance(server)
                    ));
                }
            }
            return ToolOutput::ok(out);
        };
        if !servers.iter().any(|name| name == server) {
            return ToolOutput::error(format!(
                "unknown mcp server '{server}'; available: {}",
                servers.join(", ")
            ));
        }
        let tools = match self.manager.list_tools(server).await {
            Ok(tools) => tools,
            Err(reason) => {
                return ToolOutput::error(format!(
                    "mcp list failed: {reason}. {}",
                    unavailable_guidance(server)
                ));
            }
        };
        let visible: Vec<_> = tools
            .into_iter()
            .filter(|t| self.allowed(server, &t.name))
            .collect();
        if let Some(observer) = &self.catalog_observer {
            observer(&[(server.to_string(), visible.clone())]);
        }
        if visible.is_empty() {
            return ToolOutput::ok(format!("{server}: (no tools)"));
        }
        let mut out = format!("{server}:\n");
        for t in &visible {
            let schema =
                serde_json::to_string(&t.input_schema).unwrap_or_else(|_| "{}".to_string());
            out.push_str(&format!(
                "  {} — {}\n    inputSchema: {schema}\n",
                t.name, t.description
            ));
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
            Ok((text, source)) => {
                if let Some(record) = &self.invocation_recorder {
                    record(server, tool, true, started.elapsed().as_millis() as u64);
                }
                let mut output = if text.is_empty() {
                    ToolOutput::ok("(empty result)")
                } else {
                    ToolOutput::ok(self.manager.redact(text))
                };
                output.mcp_source = Some(source);
                output
            }
            Err(e) => {
                if let Some(record) = &self.invocation_recorder {
                    record(server, tool, false, started.elapsed().as_millis() as u64);
                }
                ToolOutput::error(format!(
                    "mcp call failed: {}. {}",
                    self.manager.redact(e.to_string()),
                    unavailable_guidance(server)
                ))
            }
        }
    }
}

/// The operator's fix for a server that cannot be reached. One copy, used by
/// this tool's errors and by the capability report.
pub fn remedy(server: &str) -> String {
    format!("check the `{server}` entry under [mcp.servers] — command, args, and any required env")
}

/// What the model is told when a server fails: it is still callable (the
/// pool retries on the next demand after its backoff), so one retry is
/// reasonable, and otherwise the person hears that it is unavailable and
/// how to fix it. Told here, where the failure is observed, rather than in
/// the cached system prompt, which a changing failure text would churn.
fn unavailable_guidance(server: &str) -> String {
    format!(
        "If the request needs this server, try once more; otherwise tell the person it is \
         unavailable and that the fix is to {}",
        remedy(server)
    )
}
