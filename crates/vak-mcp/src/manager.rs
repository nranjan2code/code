use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;

use tokio::sync::Mutex;

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
