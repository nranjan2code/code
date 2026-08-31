use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;

use tokio::sync::Mutex;

use serde_json::Value;

use crate::client::{McpClient, McpError, McpToolInfo, ServerConfig};

/// Lazily spawns and caches one client per configured server.
pub struct McpManager {
    servers: HashMap<String, ServerConfig>,
    clients: Mutex<HashMap<String, Arc<McpClient>>>,
    cwd: PathBuf,
    sandbox: Option<Arc<dyn vak_tools::sandbox::Sandbox>>,
}

impl McpManager {
    pub fn new(servers: HashMap<String, ServerConfig>, cwd: PathBuf) -> Self {
        Self::new_sandboxed(servers, cwd, None)
    }

    pub fn new_sandboxed(
        servers: HashMap<String, ServerConfig>,
        cwd: PathBuf,
        sandbox: Option<Arc<dyn vak_tools::sandbox::Sandbox>>,
    ) -> Self {
        McpManager {
            servers,
            clients: Mutex::new(HashMap::new()),
            cwd,
            sandbox,
        }
    }

    pub fn server_names(&self) -> Vec<String> {
        let mut names: Vec<String> = self.servers.keys().cloned().collect();
        names.sort();
        names
    }

    pub async fn get(&self, server: &str) -> Result<Arc<McpClient>, McpError> {
        if let Some(cached) = self.clients.lock().await.get(server) {
            return Ok(cached.clone());
        }
        let config = self
            .servers
            .get(server)
            .ok_or_else(|| McpError::Protocol(format!("unknown mcp server '{server}'")))?;
        let config = Self::resolve(config, &self.cwd);
        let client =
            Arc::new(McpClient::connect(server, &config, &self.cwd, self.sandbox.as_ref()).await?);
        self.clients
            .lock()
            .await
            .insert(server.to_string(), client.clone());
        Ok(client)
    }

    /// Discover a server's current tool catalog immediately before dispatch.
    /// MCP tool names are server-defined; validating them here keeps every
    /// caller behind the same protocol boundary and turns model-invented
    /// names into actionable errors before `tools/call` is sent.
    pub async fn call_tool(
        &self,
        server: &str,
        tool: &str,
        arguments: Value,
    ) -> Result<String, McpError> {
        let client = self.get(server).await?;
        let tools = client.list_tools().await?;
        if !tools.iter().any(|candidate| candidate.name == tool) {
            let available = tools
                .iter()
                .map(|candidate| candidate.name.as_str())
                .collect::<Vec<_>>();
            return Err(McpError::Protocol(format!(
                "unknown tool '{tool}' on server '{server}'; discovered tools: {}. Use action \\\"list\\\" and call one of those exact names",
                if available.is_empty() {
                    "(none)".to_string()
                } else {
                    available.join(", ")
                }
            )));
        }
        client.call_tool(tool, arguments).await
    }

    /// Remove all resolved MCP environment values from text that can cross
    /// the MCP process boundary. MCP servers frequently include upstream
    /// request details in errors, so this applies equally to results and
    /// failures before either can enter a model transcript or UI timeline.
    pub fn redact(&self, text: impl AsRef<str>) -> String {
        let mut values = self
            .servers
            .values()
            .flat_map(|config| config.env.iter().map(|(_, value)| value))
            .filter(|value| value.len() >= 4)
            .collect::<Vec<_>>();
        values.sort_unstable_by_key(|value| std::cmp::Reverse(value.len()));
        values.dedup();
        let mut redacted = text.as_ref().to_string();
        for value in values {
            redacted = redacted.replace(value, "[REDACTED]");
        }
        redacted
    }

    /// The catalog is also model-visible, so schemas supplied by an MCP
    /// server must cross the same secret boundary as its text output.
    pub fn redact_json(&self, value: &Value) -> Value {
        serde_json::from_str(&self.redact(value.to_string())).unwrap_or(Value::Null)
    }

    /// Relative commands resolve against the workspace cwd.
    fn resolve(config: &ServerConfig, cwd: &std::path::Path) -> ServerConfig {
        let mut c = config.clone();
        let p = std::path::Path::new(&c.command);
        if !p.is_absolute()
            && p.components().count() > 1
            && let joined = cwd.join(p)
            && joined.is_file()
        {
            c.command = joined.display().to_string();
        }
        c
    }

    pub async fn shutdown_all(&self) {
        let clients: Vec<_> = self.clients.lock().await.values().cloned().collect();
        for c in clients {
            c.shutdown().await;
        }
    }
}

impl McpManager {
    /// Connect to every configured server and collect name + one-line tool
    /// summaries. Failures degrade to a per-server error line — the goal is
    /// prompt visibility, not perfection. Used by Core to advertise
    /// capabilities in the system prompt (docs/design/26-learning.md-style
    /// progressive disclosure, but for tools).
    pub async fn inventory(&self) -> Vec<(String, Vec<McpToolInfo>)> {
        let mut out = Vec::new();
        for name in self.server_names() {
            match self.get(&name).await {
                Ok(client) => match client.list_tools().await {
                    Ok(tools) => out.push((
                        name,
                        tools
                            .iter()
                            .map(|t| McpToolInfo {
                                name: t.name.clone(),
                                description: self
                                    .redact(t.description.chars().take(90).collect::<String>()),
                                input_schema: self.redact_json(&t.input_schema),
                            })
                            .collect(),
                    )),
                    Err(e) => out.push((
                        name,
                        vec![McpToolInfo {
                            name: "error".into(),
                            description: self.redact(e.to_string()),
                            input_schema: Value::Null,
                        }],
                    )),
                },
                Err(e) => out.push((
                    name,
                    vec![McpToolInfo {
                        name: "error".into(),
                        description: self.redact(e.to_string()),
                        input_schema: Value::Null,
                    }],
                )),
            }
        }
        out
    }
}
