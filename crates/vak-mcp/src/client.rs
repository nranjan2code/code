use std::collections::HashMap;
use std::process::Stdio;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::time::Duration;

use serde_json::Value;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::process::{Child, ChildStdin, Command};
use tokio::sync::{Mutex, mpsc, oneshot};
use vak_tools::sandbox::Sandbox;

/// A server-initiated message that changes what the server offers.
///
/// MCP servers announce catalog changes rather than expecting the client to
/// poll (`notifications/tools/list_changed`). Dropping these at the transport
/// is what forced the old design to warm a catalog once and never notice a
/// change; routing them out gives the capability registry a hint channel and
/// costs one `match`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum McpNotification {
    ToolsListChanged,
    PromptsListChanged,
    ResourcesListChanged,
    /// Anything else the server sent. Kept rather than discarded so an
    /// unknown-but-present signal is visible in diagnostics.
    Other(String),
}

impl McpNotification {
    fn from_method(method: &str) -> Self {
        match method {
            "notifications/tools/list_changed" => McpNotification::ToolsListChanged,
            "notifications/prompts/list_changed" => McpNotification::PromptsListChanged,
            "notifications/resources/list_changed" => McpNotification::ResourcesListChanged,
            other => McpNotification::Other(other.to_string()),
        }
    }

    /// Whether this invalidates the tool catalog the registry holds.
    pub fn invalidates_tools(&self) -> bool {
        matches!(self, McpNotification::ToolsListChanged)
    }
}

/// Where a client posts server-initiated notifications, tagged with the
/// server they came from. Unbounded because the producer is a server we do
/// not control and blocking its reader task would stall responses too.
pub type NotificationSink = mpsc::UnboundedSender<(String, McpNotification)>;

#[derive(Debug, thiserror::Error)]
pub enum McpError {
    #[error("mcp spawn failed: {0}")]
    Spawn(String),
    #[error("mcp protocol error: {0}")]
    Protocol(String),
    #[error("mcp server error: {0}")]
    Server(String),
    #[error("mcp timed out")]
    Timeout,
    #[error("mcp connection closed")]
    Closed,
}

/// Deserialized from TOML, so defaults live on the config side; this
/// struct carries the resolved values.
#[derive(Debug, Clone)]
pub struct ServerConfig {
    pub command: String,
    pub args: Vec<String>,
    pub env: Vec<(String, String)>,
    /// Allow outbound network for this server (e.g. remote-API MCP tools
    /// like web search). Opt-in per server via privileged config; when
    /// false the platform sandbox applies as usual.
    pub network: bool,
}

use std::path::PathBuf;

type Pending = Arc<Mutex<HashMap<u64, oneshot::Sender<Result<Value, McpError>>>>>;

pub struct McpClient {
    server_name: String,
    stdin: Mutex<ChildStdin>,
    child: Mutex<Child>,
    pending: Pending,
    next_id: AtomicU64,
    /// Set by the reader task when stdout ends, which is the earliest and
    /// most reliable signal that this connection is dead. Checked before a
    /// pooled client is handed out, so a caller never dispatches into a
    /// corpse and gets a timeout instead of an actionable error.
    closed: Arc<AtomicBool>,
    /// What the server said it supports in its `initialize` result. Used to
    /// report whether a stale catalog is the server's fault (no
    /// `listChanged`, so we must poll) or ours.
    server_capabilities: Value,
}

impl McpClient {
    pub async fn connect(
        server_name: &str,
        config: &ServerConfig,
        cwd: &std::path::Path,
        sandbox: Option<&Arc<dyn Sandbox>>,
    ) -> Result<Self, McpError> {
        Self::connect_with_notifications(server_name, config, cwd, sandbox, None).await
    }

    pub async fn connect_with_notifications(
        server_name: &str,
        config: &ServerConfig,
        cwd: &std::path::Path,
        sandbox: Option<&Arc<dyn Sandbox>>,
        notifications: Option<NotificationSink>,
    ) -> Result<Self, McpError> {
        // A network-egress server is a deliberate trust decision from
        // privileged config; the OS command wrapper would deny its sockets,
        // so it spawns directly (env still scrubbed to the explicit set).
        let command_path = resolve_command(&config.command);
        let mut cmd = if let (Some(sandbox), false) = (sandbox, config.network) {
            let command = std::iter::once(command_path.to_string_lossy().to_string())
                .chain(config.args.iter().cloned())
                .map(|part| shell_quote(&part))
                .collect::<Vec<_>>()
                .join(" ");
            let mut cmd = Command::new("sh");
            cmd.arg("-c").arg(sandbox.wrap(&command));
            cmd
        } else {
            let mut cmd = Command::new(&command_path);
            cmd.args(&config.args);
            cmd
        };
        cmd.current_dir(cwd)
            .env_clear()
            .kill_on_drop(true)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null());
        for (k, v) in &config.env {
            cmd.env(k, v);
        }
        // Operational basics every runtime needs (PATH for interpreters,
        // HOME/TMPDIR for package caches). Secrets never ride along: the
        // environment was cleared above and recipients are explicit.
        // Prepend the server executable's own directory to PATH: under
        // service managers PATH is minimal, and interpreters resolved via
        // `#!/usr/bin/env` (npx→node) need the same prefix that worked for
        // the command itself.
        let mut path_parts: Vec<PathBuf> = Vec::new();
        if let Some(dir) = command_path.parent() {
            path_parts.push(dir.to_path_buf());
        }
        for toolchain_path in vak_config::paths::canonical_toolchain_paths() {
            if !path_parts.contains(&toolchain_path) {
                path_parts.push(toolchain_path);
            }
        }
        if let Some(path) = std::env::var_os("PATH") {
            for p in std::env::split_paths(&path).filter(|p| !p.as_os_str().is_empty()) {
                if !path_parts.contains(&p) {
                    path_parts.push(p);
                }
            }
        }
        cmd.env(
            "PATH",
            std::env::join_paths(&path_parts).unwrap_or_default(),
        );
        for var in ["HOME", "TMPDIR"] {
            if let Some(v) = std::env::var_os(var) {
                cmd.env(var, v);
            }
        }
        if std::env::var_os("VAK_MCP_DEBUG").is_some() {
            cmd.stderr(Stdio::inherit());
        }
        vak_tools::bash::isolate_process_group(&mut cmd);

        let mut child = cmd.spawn().map_err(|e| McpError::Spawn(e.to_string()))?;
        let stdin = child
            .stdin
            .take()
            .ok_or_else(|| McpError::Spawn("no stdin".into()))?;
        let stdout = child
            .stdout
            .take()
            .ok_or_else(|| McpError::Spawn("no stdout".into()))?;

        let pending: Pending = Arc::new(Mutex::new(HashMap::new()));
        let closed = Arc::new(AtomicBool::new(false));

        // Reader: route responses by id and notifications to the sink.
        // Server-initiated *requests* (those carry both an id and a method)
        // are still unanswered — v1 does not serve sampling/roots — but a
        // notification is a catalog-change signal the registry needs, so it
        // is forwarded rather than dropped.
        {
            let pending = pending.clone();
            let closed = closed.clone();
            let server = server_name.to_string();
            tokio::spawn(async move {
                let mut lines = BufReader::new(stdout).lines();
                loop {
                    match lines.next_line().await {
                        Ok(Some(line)) if line.trim().is_empty() => continue,
                        Ok(Some(line)) => {
                            let Ok(v) = serde_json::from_str::<Value>(&line) else {
                                continue;
                            };
                            let method = v.get("method").and_then(|m| m.as_str());
                            let Some(id) = v.get("id").and_then(|i| i.as_u64()) else {
                                // No id: a notification. Forward it.
                                if let (Some(method), Some(sink)) = (method, notifications.as_ref())
                                {
                                    let _ = sink.send((
                                        server.clone(),
                                        McpNotification::from_method(method),
                                    ));
                                }
                                continue;
                            };
                            if method.is_some() {
                                // id + method: a server-initiated request.
                                // Not served in v1; never matched to pending.
                                continue;
                            }
                            let responder = pending.lock().await.remove(&id);
                            if let Some(tx) = responder {
                                let result = match v.get("error") {
                                    Some(err) => Err(McpError::Server(
                                        err.get("message")
                                            .and_then(|m| m.as_str())
                                            .unwrap_or("unknown")
                                            .to_string(),
                                    )),
                                    None => Ok(v.get("result").cloned().unwrap_or(Value::Null)),
                                };
                                let _ = tx.send(result);
                            }
                        }
                        _ => {
                            // stdout ended: this connection is dead. Mark it
                            // before waking waiters so a racing `get()` sees
                            // the flag rather than handing out this client.
                            closed.store(true, Ordering::Release);
                            let mut map = pending.lock().await;
                            for (_, tx) in map.drain() {
                                let _ = tx.send(Err(McpError::Closed));
                            }
                            return;
                        }
                    }
                }
            });
        }

        let mut client = McpClient {
            server_name: server_name.to_string(),
            stdin: Mutex::new(stdin),
            child: Mutex::new(child),
            pending,
            next_id: AtomicU64::new(1),
            closed,
            server_capabilities: Value::Null,
        };

        let initialized = client
            .request(
                "initialize",
                serde_json::json!({
                    "protocolVersion": "2025-06-18",
                    "capabilities": {},
                    "clientInfo": {"name": "vak", "version": env!("CARGO_PKG_VERSION")},
                }),
            )
            .await?;
        client.server_capabilities = initialized
            .get("capabilities")
            .cloned()
            .unwrap_or(Value::Null);
        client
            .notify("notifications/initialized", serde_json::json!({}))
            .await;
        Ok(client)
    }

    pub fn server_name(&self) -> &str {
        &self.server_name
    }

    /// False once stdout has ended or the child has exited. Cheap and
    /// non-blocking: the pool checks this before reusing a client so a dead
    /// server is replaced rather than dispatched into.
    pub fn is_alive(&self) -> bool {
        if self.closed.load(Ordering::Acquire) {
            return false;
        }
        // `try_lock` because this runs on the hot path of every dispatch and
        // a contended child lock means someone is mid-shutdown anyway.
        match self.child.try_lock() {
            Ok(mut child) => !matches!(child.try_wait(), Ok(Some(_))),
            Err(_) => true,
        }
    }

    /// Whether the server promised to announce tool-catalog changes. When
    /// false the registry must fall back to periodic re-probing for this
    /// server rather than trusting that silence means unchanged.
    pub fn announces_tool_changes(&self) -> bool {
        self.server_capabilities
            .get("tools")
            .and_then(|t| t.get("listChanged"))
            .and_then(|l| l.as_bool())
            .unwrap_or(false)
    }

    pub async fn shutdown(&self) {
        self.cancel_pending().await;
        let mut child = self.child.lock().await;
        vak_tools::bash::kill_process_group(&child.id());
        let _ = child.wait().await;
    }

    async fn cancel_pending(&self) {
        let mut map = self.pending.lock().await;
        for (_, tx) in map.drain() {
            let _ = tx.send(Err(McpError::Closed));
        }
    }

    async fn send_raw(&self, msg: &Value) -> Result<(), McpError> {
        let mut line = serde_json::to_string(msg).map_err(|e| McpError::Protocol(e.to_string()))?;
        line.push('\n');
        let mut stdin = self.stdin.lock().await;
        stdin
            .write_all(line.as_bytes())
            .await
            .map_err(|_| McpError::Closed)?;
        stdin.flush().await.map_err(|_| McpError::Closed)
    }

    async fn notify(&self, method: &str, params: Value) {
        let msg = serde_json::json!({"jsonrpc": "2.0", "method": method, "params": params});
        let _ = self.send_raw(&msg).await;
    }

    async fn request(&self, method: &str, params: Value) -> Result<Value, McpError> {
        let id = self.next_id.fetch_add(1, Ordering::Relaxed);
        let msg = serde_json::json!({
            "jsonrpc": "2.0",
            "id": id,
            "method": method,
            "params": params,
        });

        let (tx, rx) = oneshot::channel();
        self.pending.lock().await.insert(id, tx);

        if let Err(e) = self.send_raw(&msg).await {
            self.pending.lock().await.remove(&id);
            return Err(e);
        }

        match tokio::time::timeout(Duration::from_secs(60), rx).await {
            Ok(Ok(result)) => result,
            Ok(Err(_)) => Err(McpError::Closed),
            Err(_) => {
                self.pending.lock().await.remove(&id);
                Err(McpError::Timeout)
            }
        }
    }

    pub async fn list_tools(&self) -> Result<Vec<McpToolInfo>, McpError> {
        let result = self.request("tools/list", serde_json::json!({})).await?;
        let Some(tools) = result.get("tools").and_then(|t| t.as_array()) else {
            return Ok(Vec::new());
        };
        Ok(tools
            .iter()
            .map(|t| McpToolInfo {
                name: t
                    .get("name")
                    .and_then(|n| n.as_str())
                    .unwrap_or_default()
                    .to_string(),
                description: t
                    .get("description")
                    .and_then(|d| d.as_str())
                    .unwrap_or_default()
                    .to_string(),
                input_schema: t
                    .get("inputSchema")
                    .cloned()
                    .unwrap_or_else(|| serde_json::json!({"type": "object"})),
            })
            .collect())
    }

    pub async fn call_tool(&self, tool: &str, arguments: Value) -> Result<String, McpError> {
        let result = self
            .request(
                "tools/call",
                serde_json::json!({"name": tool, "arguments": arguments}),
            )
            .await?;

        let is_error = result
            .get("isError")
            .and_then(|e| e.as_bool())
            .unwrap_or(false);
        let text = text_tool_result(&result)?;

        if is_error {
            Err(McpError::Server(if text.is_empty() {
                "tool reported an error".into()
            } else {
                text
            }))
        } else {
            Ok(text)
        }
    }
}

/// The current model tool-result contract is text. Never report success after
/// silently discarding an MCP image, audio, or resource block: that would
/// make the model reason over an incomplete result while the ledger says the
/// call succeeded. A richer result type can replace this boundary later.
fn text_tool_result(result: &Value) -> Result<String, McpError> {
    let blocks = result
        .get("content")
        .and_then(Value::as_array)
        .ok_or_else(|| McpError::Protocol("tools/call returned no content array".into()))?;
    let mut parts = Vec::with_capacity(blocks.len());
    for block in blocks {
        if block.get("type").and_then(Value::as_str) != Some("text") {
            let kind = block
                .get("type")
                .and_then(Value::as_str)
                .unwrap_or("unknown");
            return Err(McpError::Protocol(format!(
                "tools/call returned unsupported {kind} content; this tool boundary supports text only"
            )));
        }
        let text = block
            .get("text")
            .and_then(Value::as_str)
            .ok_or_else(|| McpError::Protocol("tools/call text block has no text".into()))?;
        parts.push(text);
    }
    Ok(parts.join("\n"))
}

fn resolve_command(command: &str) -> PathBuf {
    let path = PathBuf::from(command);
    if path.is_absolute() || command.contains(std::path::MAIN_SEPARATOR) {
        return path;
    }
    if let Some(paths) = std::env::var_os("PATH") {
        for dir in std::env::split_paths(&paths) {
            let candidate = dir.join(command);
            if candidate.is_file() {
                return candidate;
            }
        }
    }
    for dir in vak_config::paths::canonical_toolchain_paths() {
        let candidate = dir.join(command);
        if candidate.is_file() {
            return candidate;
        }
    }
    path
}

fn shell_quote(value: &str) -> String {
    let mut out = String::with_capacity(value.len() + 2);
    out.push('\'');
    for character in value.chars() {
        if character == '\'' {
            out.push_str("'\\''");
        } else {
            out.push(character);
        }
    }
    out.push('\'');
    out
}

#[derive(Debug, Clone, PartialEq)]
pub struct McpToolInfo {
    pub name: String,
    pub description: String,
    pub input_schema: Value,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn text_result_keeps_every_text_block() {
        let result = serde_json::json!({"content": [
            {"type": "text", "text": "first"},
            {"type": "text", "text": "second"}
        ]});
        assert_eq!(
            text_tool_result(&result).ok().as_deref(),
            Some("first\nsecond")
        );
    }

    #[test]
    fn non_text_result_fails_instead_of_disappearing() {
        for result in [
            serde_json::json!({"content": [{"type": "image", "data": "..."}]}),
            serde_json::json!({"content": [
                {"type": "text", "text": "caption"},
                {"type": "resource", "resource": {"uri": "file:///report"}}
            ]}),
        ] {
            assert!(matches!(
                text_tool_result(&result),
                Err(McpError::Protocol(_))
            ));
        }
    }
}
