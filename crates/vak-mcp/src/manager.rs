use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, Instant};

use tokio::sync::Mutex;

use serde_json::Value;

use crate::client::{McpClient, McpError, McpToolInfo, NotificationSink, ServerConfig};

/// How long listing a server (connect + `tools/list`) may take before the
/// attempt counts as a failure. Far below the 60s per-request ceiling: a
/// server that cannot even list its tools should cost the turn seconds, not
/// a minute.
pub const LIST_TIMEOUT: Duration = Duration::from_secs(10);

/// First wait after a failed attempt; doubles per consecutive failure up to
/// [`FAILURE_BACKOFF_CAP`]. The pool never retries on its own — this only
/// stops repeated demand from respawning a broken server on every call.
const FAILURE_BACKOFF_BASE: Duration = Duration::from_secs(5);
const FAILURE_BACKOFF_CAP: Duration = Duration::from_secs(5 * 60);

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

/// The outcome of listing one server's catalog.
///
/// A failure is a `Err(reason)`, never a synthesised tool. The old shape
/// returned a tool literally named `error`, which made a dead server look
/// like a working one to the prompt, to the alias table, and to the retry
/// guard that then refused to try again.
pub type ListOutcome = Result<Vec<McpToolInfo>, String>;

/// What demand has taught the pool about one server. Nothing here is ever
/// learned by spawning a server for its own sake: a server nobody has asked
/// for has an empty observation.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct ServerObservation {
    /// The catalog the last live connection returned, redacted and with
    /// descriptions trimmed. `None` until the server is first used.
    pub tools: Option<Vec<McpToolInfo>>,
    /// Why the last attempt failed, until an attempt succeeds.
    pub failure: Option<String>,
}

#[derive(Default)]
struct ServerState {
    observation: ServerObservation,
    attempts: u32,
    failed_at: Option<Instant>,
}

impl ServerState {
    fn retry_after(&self, now: Instant) -> Option<Duration> {
        let failed_at = self.failed_at?;
        let shift = self.attempts.saturating_sub(1).min(16);
        let wait = FAILURE_BACKOFF_BASE
            .saturating_mul(1u32 << shift)
            .min(FAILURE_BACKOFF_CAP);
        wait.checked_sub(now.duration_since(failed_at))
    }
}

/// The on-demand connection pool: one client per configured server, spawned
/// only when a call needs it, shared by every session of the owning `Core`,
/// and shut down after [`IDLE_TTL`] unused. It never connects on its own
/// initiative — no warm-up, no background probe, no respawn after eviction.
pub struct McpManager {
    servers: HashMap<String, ServerConfig>,
    clients: Mutex<HashMap<String, Pooled>>,
    state: std::sync::Mutex<HashMap<String, ServerState>>,
    cwd: PathBuf,
    sandbox: Option<Arc<dyn vak_tools::sandbox::Sandbox>>,
    notifications: Option<NotificationSink>,
    observer: Option<tokio::sync::mpsc::UnboundedSender<String>>,
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
            state: std::sync::Mutex::new(HashMap::new()),
            cwd,
            sandbox,
            notifications: None,
            observer: None,
        }
    }

    /// Receive a server's name whenever what the pool knows about it
    /// changes — a catalog learned or changed, a failure recorded or cleared.
    pub fn with_observer(mut self, sink: tokio::sync::mpsc::UnboundedSender<String>) -> Self {
        self.observer = Some(sink);
        self
    }

    /// What demand has observed about every server, for the capability
    /// declarations. Synchronous and free of I/O.
    pub fn observations(&self) -> HashMap<String, ServerObservation> {
        self.state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .iter()
            .map(|(name, state)| (name.clone(), state.observation.clone()))
            .collect()
    }

    /// Forget a server's catalog, e.g. after it announced
    /// `notifications/tools/list_changed`. The next use re-lists it.
    pub fn forget_catalog(&self, server: &str) {
        let changed = {
            let mut state = self
                .state
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            state
                .get_mut(server)
                .and_then(|entry| entry.observation.tools.take())
                .is_some()
        };
        if changed {
            self.announce(server);
        }
    }

    fn announce(&self, server: &str) {
        if let Some(observer) = &self.observer {
            let _ = observer.send(server.to_string());
        }
    }

    fn record_tools(&self, server: &str, tools: &[McpToolInfo]) {
        let observed: Vec<McpToolInfo> = tools
            .iter()
            .map(|tool| McpToolInfo {
                name: tool.name.clone(),
                description: self.redact(tool.description.chars().take(90).collect::<String>()),
                input_schema: self.redact_json(&tool.input_schema),
            })
            .collect();
        let changed = {
            let mut state = self
                .state
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            let entry = state.entry(server.to_string()).or_default();
            let changed = entry.observation.tools.as_ref() != Some(&observed)
                || entry.observation.failure.is_some();
            entry.observation.tools = Some(observed);
            entry.observation.failure = None;
            entry.attempts = 0;
            entry.failed_at = None;
            changed
        };
        if changed {
            self.announce(server);
        }
    }

    fn record_failure(&self, server: &str, reason: &str) {
        let reason = self.redact(reason);
        let changed = {
            let mut state = self
                .state
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            let entry = state.entry(server.to_string()).or_default();
            let changed = entry.observation.failure.as_deref() != Some(reason.as_str());
            entry.observation.failure = Some(reason);
            entry.attempts = entry.attempts.saturating_add(1);
            entry.failed_at = Some(Instant::now());
            changed
        };
        if changed {
            self.announce(server);
        }
    }

    /// The failure still inside its backoff window, if any: repeated demand
    /// gets the recorded reason back instead of a fresh spawn.
    fn backing_off(&self, server: &str) -> Option<String> {
        let state = self
            .state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let entry = state.get(server)?;
        let wait = entry.retry_after(Instant::now())?;
        let reason = entry.observation.failure.as_deref().unwrap_or("failed");
        Some(format!(
            "mcp server '{server}' is unavailable: {reason} (next attempt in {}s)",
            wait.as_secs().max(1)
        ))
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

    async fn get(&self, server: &str) -> Result<Arc<McpClient>, McpError> {
        if let Some(reason) = self.backing_off(server) {
            return Err(McpError::Protocol(reason));
        }
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
        let tools = self.list_live(server).await?;
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
        let client = self.get(server).await?;
        client.call_tool(tool, arguments).await
    }

    /// Connect if needed and list, recording what happened either way.
    async fn list_live(&self, server: &str) -> Result<Vec<McpToolInfo>, McpError> {
        let listed = async {
            let client = self.get(server).await?;
            client.list_tools().await
        };
        match listed.await {
            Ok(tools) => {
                self.record_tools(server, &tools);
                Ok(tools)
            }
            Err(error) => {
                if self.backing_off(server).is_none() {
                    self.record_failure(server, &error.to_string());
                }
                Err(error)
            }
        }
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
    /// List one server's catalog under [`LIST_TIMEOUT`] — the demand path
    /// behind the `mcp` tool's `list`. Connects on demand, records the
    /// outcome, and returns the redacted catalog.
    ///
    /// The timeout wraps connect *and* list, because a server that hangs on
    /// spawn is the same problem as one that hangs on `tools/list` and the
    /// caller cannot act differently on the two.
    pub async fn list_tools(&self, server: &str) -> ListOutcome {
        match tokio::time::timeout(LIST_TIMEOUT, self.list_live(server)).await {
            Ok(Ok(tools)) => Ok(tools
                .iter()
                .map(|t| McpToolInfo {
                    name: t.name.clone(),
                    description: self.redact(&t.description),
                    input_schema: self.redact_json(&t.input_schema),
                })
                .collect()),
            Ok(Err(e)) => Err(self.redact(e.to_string())),
            Err(_) => {
                let reason = format!("did not respond within {}s", LIST_TIMEOUT.as_secs());
                self.record_failure(server, &reason);
                Err(reason)
            }
        }
    }
}
