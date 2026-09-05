use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, Instant};

use tokio::sync::Mutex;

use serde_json::Value;

use crate::client::{McpClient, McpError, McpToolInfo, NotificationSink, ServerConfig};

/// How long a probe (connect + `tools/list`) may take before the server is
/// treated as unreachable for this pass. Deliberately far below the 60s
/// per-request ceiling: discovery runs on a loop and a slow server costs a
/// retry, not a stalled turn.
pub const PROBE_TIMEOUT: Duration = Duration::from_secs(10);

/// How long a pooled connection may sit unused before it is shut down. A
/// process that stays up for weeks must not hold a subprocess for every
/// server it has ever touched; the next call respawns on demand.
pub const IDLE_TTL: Duration = Duration::from_secs(15 * 60);

/// One pooled connection plus the bookkeeping the pool needs to decide
/// whether to keep it.
struct Pooled {
    client: Arc<McpClient>,
    last_used: Instant,
}

/// The outcome of probing one server's catalog.
///
/// A failure is a `Err(reason)`, never a synthesised tool. The old shape
/// returned a tool literally named `error`, which made a dead server look
/// like a working one to the prompt, to the alias table, and to the retry
/// guard that then refused to try again.
pub type ProbeOutcome = Result<Vec<McpToolInfo>, String>;

/// Lazily spawns and pools one client per configured server.
pub struct McpManager {
    servers: HashMap<String, ServerConfig>,
    clients: Mutex<HashMap<String, Pooled>>,
    cwd: PathBuf,
    sandbox: Option<Arc<dyn vak_tools::sandbox::Sandbox>>,
    notifications: Option<NotificationSink>,
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
            notifications: None,
        }
    }

    /// Route server-initiated notifications (catalog changes) to `sink`.
    /// Applies to connections opened after this call.
    pub fn with_notifications(mut self, sink: NotificationSink) -> Self {
        self.notifications = Some(sink);
        self
    }

    pub fn server_names(&self) -> Vec<String> {
        let mut names: Vec<String> = self.servers.keys().cloned().collect();
        names.sort();
        names
    }

    /// Persist a redacted MCP response as a workspace artifact. The model
    /// receives only a bounded preview plus this path and can use the normal
    /// `read` capability for exact, paged retrieval.
    pub fn store_artifact(&self, server: &str, tool: &str, content: &str) -> Option<PathBuf> {
        let dir = self.cwd.join(".vak").join("mcp-artifacts");
        std::fs::create_dir_all(&dir).ok()?;
        let safe_server: String = server
            .chars()
            .map(|c| if c.is_ascii_alphanumeric() { c } else { '_' })
            .collect();
        let safe_tool: String = tool
            .chars()
            .map(|c| if c.is_ascii_alphanumeric() { c } else { '_' })
            .collect();
        let path = dir.join(format!(
            "{}-{}-{}.txt",
            safe_server,
            safe_tool,
            uuid::Uuid::now_v7()
        ));
        let tmp = path.with_extension("tmp");
        std::fs::write(&tmp, content).ok()?;
        std::fs::rename(&tmp, &path).ok()?;
        Some(path)
    }

    pub async fn get(&self, server: &str) -> Result<Arc<McpClient>, McpError> {
        {
            let mut pool = self.clients.lock().await;
            if let Some(entry) = pool.get_mut(server) {
                if entry.client.is_alive() {
                    entry.last_used = Instant::now();
                    return Ok(entry.client.clone());
                }
                // Dead: drop it and fall through to a fresh spawn rather
                // than handing back a client whose child has exited.
                pool.remove(server);
            }
        }
        let config = self
            .servers
            .get(server)
            .ok_or_else(|| McpError::Protocol(format!("unknown mcp server '{server}'")))?;
        let config = Self::resolve(config, &self.cwd);
        let client = Arc::new(
            McpClient::connect_with_notifications(
                server,
                &config,
                &self.cwd,
                self.sandbox.as_ref(),
                self.notifications.clone(),
            )
            .await?,
        );
        self.clients.lock().await.insert(
            server.to_string(),
            Pooled {
                client: client.clone(),
                last_used: Instant::now(),
            },
        );
        Ok(client)
    }

    /// Shut down connections idle for longer than `ttl`, and drop any that
    /// have died. Called from the reconcile loop; safe to call at any time.
    /// Returns the names evicted, for the diagnostics surface.
    pub async fn evict_idle(&self, ttl: Duration) -> Vec<String> {
        let now = Instant::now();
        let mut evicted = Vec::new();
        let mut closing = Vec::new();
        {
            let mut pool = self.clients.lock().await;
            pool.retain(|name, entry| {
                let expired = now.duration_since(entry.last_used) > ttl;
                let dead = !entry.client.is_alive();
                if expired || dead {
                    evicted.push(name.clone());
                    if expired && !dead {
                        closing.push(entry.client.clone());
                    }
                    return false;
                }
                true
            });
        }
        // Shut down outside the pool lock: `shutdown` waits on the child.
        for client in closing {
            client.shutdown().await;
        }
        evicted
    }

    /// Whether a server currently has a live pooled connection.
    pub async fn is_connected(&self, server: &str) -> bool {
        self.clients
            .lock()
            .await
            .get(server)
            .is_some_and(|entry| entry.client.is_alive())
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
        let Some(info) = tools.iter().find(|candidate| candidate.name == tool) else {
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
        };
        crate::validate::validate_arguments(&arguments, &info.input_schema)
            .map_err(McpError::Protocol)?;
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
        let clients: Vec<_> = self
            .clients
            .lock()
            .await
            .values()
            .map(|entry| entry.client.clone())
            .collect();
        for c in clients {
            c.shutdown().await;
        }
    }
}

impl McpManager {
    /// Probe one server's catalog under `PROBE_TIMEOUT`.
    ///
    /// The timeout wraps connect *and* list, because a server that hangs on
    /// spawn is the same problem as one that hangs on `tools/list` and the
    /// caller cannot act differently on the two.
    pub async fn probe(&self, server: &str) -> ProbeOutcome {
        let probe = async {
            let client = self.get(server).await.map_err(|e| e.to_string())?;
            let tools = client.list_tools().await.map_err(|e| e.to_string())?;
            Ok::<_, String>(
                tools
                    .iter()
                    .map(|t| McpToolInfo {
                        name: t.name.clone(),
                        description: self
                            .redact(t.description.chars().take(90).collect::<String>()),
                        input_schema: self.redact_json(&t.input_schema),
                    })
                    .collect::<Vec<_>>(),
            )
        };
        match tokio::time::timeout(PROBE_TIMEOUT, probe).await {
            Ok(Ok(tools)) => Ok(tools),
            Ok(Err(e)) => Err(self.redact(e)),
            Err(_) => Err(format!(
                "did not respond within {}s",
                PROBE_TIMEOUT.as_secs()
            )),
        }
    }

    /// Probe every configured server concurrently.
    ///
    /// Concurrent because the cost of a dead server must be the *max* of the
    /// probe budget rather than the sum: three unreachable servers used to
    /// mean three minutes during which every new session froze a degraded
    /// prompt. Each server's outcome is independent, so one failure never
    /// hides another server's catalog.
    pub async fn inventory(&self) -> Vec<(String, ProbeOutcome)> {
        let names = self.server_names();
        let probes = names.iter().map(|name| async move {
            let outcome = self.probe(name).await;
            (name.clone(), outcome)
        });
        futures::future::join_all(probes).await
    }
}
