use std::collections::HashMap;
use std::process::Stdio;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

use serde_json::Value;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::process::{Child, ChildStdin, Command};
use tokio::sync::{Mutex, oneshot};
use vak_tools::sandbox::Sandbox;

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
}

impl McpClient {
    pub async fn connect(
        server_name: &str,
        config: &ServerConfig,
        cwd: &std::path::Path,
        sandbox: Option<&Arc<dyn Sandbox>>,
    ) -> Result<Self, McpError> {
        // A network-egress server is a deliberate trust decision from
        // privileged config; the OS command wrapper would deny its sockets,
        // so it spawns directly (env still scrubbed to the explicit set).
        let mut cmd = if let (Some(sandbox), false) = (sandbox, config.network) {
            let command = std::iter::once(config.command.as_str())
                .chain(config.args.iter().map(String::as_str))
                .map(shell_quote)
                .collect::<Vec<_>>()
                .join(" ");
            let mut cmd = Command::new("sh");
            cmd.arg("-c").arg(sandbox.wrap(&command));
            cmd
        } else {
            let mut cmd = Command::new(&config.command);
            cmd.args(&config.args);
            cmd
        };
        cmd.current_dir(cwd)
            .env_clear()
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
        if let Some(dir) = std::path::Path::new(&config.command).parent() {
            path_parts.push(dir.to_path_buf());
        }
        if let Some(path) = std::env::var_os("PATH") {
            path_parts.extend(std::env::split_paths(&path).filter(|p| !p.as_os_str().is_empty()));
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

        // Reader: route responses by id; drop notifications and
        // server-initiated requests (v1 does not serve sampling/roots).
        {
            let pending = pending.clone();
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
                            let Some(id) = v.get("id").and_then(|i| i.as_u64()) else {
                                continue;
                            };
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
                            let mut map = pending.lock().await;
                            for (_, tx) in map.drain() {
                                let _ = tx.send(Err(McpError::Closed));
                            }
                            let _ = server;
                            return;
                        }
                    }
                }
            });
        }

        let client = McpClient {
            server_name: server_name.to_string(),
            stdin: Mutex::new(stdin),
            child: Mutex::new(child),
            pending,
            next_id: AtomicU64::new(1),
        };

        client
            .request(
                "initialize",
                serde_json::json!({
                    "protocolVersion": "2025-06-18",
                    "capabilities": {},
                    "clientInfo": {"name": "vakcoder", "version": env!("CARGO_PKG_VERSION")},
                }),
            )
            .await?;
        client
            .notify("notifications/initialized", serde_json::json!({}))
            .await;
        Ok(client)
    }

    pub fn server_name(&self) -> &str {
        &self.server_name
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
        let text = result
            .get("content")
            .and_then(|c| c.as_array())
            .map(|blocks| {
                blocks
                    .iter()
                    .filter_map(|b| b.get("text").and_then(|t| t.as_str()))
                    .collect::<Vec<_>>()
                    .join("\n")
            })
            .unwrap_or_default();

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

#[derive(Debug, Clone)]
pub struct McpToolInfo {
    pub name: String,
    pub description: String,
}
