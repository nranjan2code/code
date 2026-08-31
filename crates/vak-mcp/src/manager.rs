use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;

use tokio::sync::Mutex;

use serde_json::Value;

use crate::client::{McpClient, McpError, ServerConfig};

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
    pub async fn inventory(&self) -> Vec<(String, Vec<(String, String)>)> {
        let mut out = Vec::new();
        for name in self.server_names() {
            match self.get(&name).await {
                Ok(client) => match client.list_tools().await {
                    Ok(tools) => out.push((
                        name,
                        tools
                            .iter()
                            .map(|t| (t.name.clone(), t.description.chars().take(90).collect()))
                            .collect(),
                    )),
                    Err(e) => out.push((name, vec![("error".into(), e.to_string())])),
                },
                Err(e) => out.push((name, vec![("error".into(), e.to_string())])),
            }
        }
        out
    }
}
