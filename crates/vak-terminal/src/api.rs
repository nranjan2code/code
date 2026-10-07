//! Real API client for the vak server, replacing the previous hardcoded
//! mock surface. Every piece of data the terminal renders is fetched live
//! from the running server over HTTP/SSE, authenticated with a bearer token.
//!
//! (docs/design/55-rich-terminal-surface.md §6 — the real-time data plane.)

use std::collections::HashMap;

use futures::StreamExt;
use reqwest::header::{AUTHORIZATION, CONTENT_TYPE};
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
use thiserror::Error;
use tokio::sync::mpsc;

// ---- Error type -------------------------------------------------------------

#[derive(Debug, Error)]
pub enum ApiError {
    #[error("network error: {0}")]
    Network(String),

    #[error("server error: {status} {body}")]
    Server { status: u16, body: String },

    #[error("parse error: {0}")]
    Parse(String),

    #[error("connection refused: {0}")]
    Unreachable(String),
}

// ---- Terminal events emitted by the SSE watcher ----

/// Events the terminal's main loop receives from background SSE tasks.
#[derive(Debug, Clone)]
pub enum TerminalEvent {
    /// A real agent event (TurnStart, ToolCall, ApprovalRequested, etc.).
    Agent { event: serde_json::Value, seq: u64 },
    /// A real presentation timeline frame.
    Presentation { frame: serde_json::Value, seq: u64 },
    /// Periodic health refresh.
    Health(HealthReport),
    /// Connection established to the server.
    Connected,
    /// An error occurred (stream dropped, parse failure, etc.).
    Error(String),
}

// ---- API client -------------------------------------------------------------

/// A bearer-token-authenticated HTTP+SSE client to the vak server.
pub struct ApiClient {
    client: reqwest::Client,
    base_url: String,
    token: String,
}

impl ApiClient {
    pub fn new(base_url: impl Into<String>, token: impl Into<String>) -> Self {
        let client = reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(10))
            .build()
            .unwrap_or_default();
        Self {
            client,
            base_url: base_url.into(),
            token: token.into(),
        }
    }

    pub fn base_url(&self) -> &str {
        &self.base_url
    }

    fn auth(&self) -> String {
        format!("Bearer {}", self.token)
    }

    fn url(&self, path: &str) -> String {
        let base = self.base_url.trim_end_matches('/');
        let path = path.trim_start_matches('/');
        format!("{base}/{path}")
    }

    async fn get_json<T: DeserializeOwned>(&self, path: &str) -> Result<T, ApiError> {
        let url = self.url(path);
        let res = self
            .client
            .get(&url)
            .header(AUTHORIZATION, self.auth())
            .send()
            .await
            .map_err(|e| ApiError::Network(e.to_string()))?;
        if !res.status().is_success() {
            return Err(ApiError::Server {
                status: res.status().as_u16(),
                body: res.text().await.unwrap_or_default(),
            });
        }
        let json = res
            .json::<T>()
            .await
            .map_err(|e| ApiError::Parse(e.to_string()))?;
        Ok(json)
    }

    async fn post_json<T: Serialize>(
        &self,
        path: &str,
        body: &T,
    ) -> Result<serde_json::Value, ApiError> {
        let url = self.url(path);
        let res = self
            .client
            .post(&url)
            .header(AUTHORIZATION, self.auth())
            .header(CONTENT_TYPE, "application/json")
            .json(body)
            .send()
            .await
            .map_err(|e| ApiError::Network(e.to_string()))?;
        if !res.status().is_success() {
            return Err(ApiError::Server {
                status: res.status().as_u16(),
                body: res.text().await.unwrap_or_default(),
            });
        }
        let json = res
            .json::<serde_json::Value>()
            .await
            .map_err(|e| ApiError::Parse(e.to_string()))?;
        Ok(json)
    }

    pub async fn health(&self) -> Result<HealthReport, ApiError> {
        let json = self.get_json::<serde_json::Value>("/health").await?;
        Ok(HealthReport::from_json(json))
    }

    pub async fn list_sessions(&self) -> Result<Vec<SessionInfo>, ApiError> {
        let json = self.get_json::<serde_json::Value>("/sessions").await?;
        let arr = json
            .get("sessions")
            .and_then(|v| v.as_array())
            .ok_or_else(|| ApiError::Parse("missing sessions array".into()))?;
        arr.iter()
            .map(|v| serde_json::from_value(v.clone()).map_err(|e| ApiError::Parse(e.to_string())))
            .collect()
    }

    pub async fn create_session(&self) -> Result<String, ApiError> {
        let json = self.post_json("/sessions", &serde_json::json!({})).await?;
        json.get("session_id")
            .and_then(|v| v.as_str())
            .map(String::from)
            .ok_or_else(|| ApiError::Parse("missing session_id".into()))
    }

    pub async fn attach_session(&self, id: &str) -> Result<AttachResponse, ApiError> {
        let json = self
            .post_json(
                &format!("/sessions/{}/attach", id),
                &serde_json::json!({ "session_id": id }),
            )
            .await?;
        serde_json::from_value(json).map_err(|e| ApiError::Parse(e.to_string()))
    }

    /// Custom commands and skills for the quick-action palette. A skill is
    /// listed as `/skill:name`, the form the server expands deterministically.
    pub async fn list_palette_extras(&self) -> Vec<crate::repl::PaletteEntry> {
        let mut entries = Vec::new();
        if let Ok(json) = self.get_json::<serde_json::Value>("/commands").await {
            for item in json["commands"].as_array().into_iter().flatten() {
                if let Some(name) = item["name"].as_str() {
                    entries.push(crate::repl::PaletteEntry {
                        name: format!("/{name}"),
                        description: item["description"].as_str().unwrap_or_default().to_string(),
                    });
                }
            }
        }
        if let Ok(json) = self.get_json::<serde_json::Value>("/skills").await {
            for item in json["skills"].as_array().into_iter().flatten() {
                if let Some(name) = item["name"].as_str() {
                    entries.push(crate::repl::PaletteEntry {
                        name: format!("/skill:{name}"),
                        description: item["description"].as_str().unwrap_or_default().to_string(),
                    });
                }
            }
        }
        entries
    }

    pub async fn list_providers(&self) -> Result<ProviderList, ApiError> {
        let json = self.get_json::<serde_json::Value>("/providers").await?;
        serde_json::from_value(json).map_err(|e| ApiError::Parse(e.to_string()))
    }

    pub async fn discover_models(&self, provider: &str) -> Result<Vec<String>, ApiError> {
        let json = self
            .get_json::<serde_json::Value>(&format!("/providers/{}/models", provider))
            .await?;
        json.get("models")
            .and_then(|v| v.as_array())
            .map(|arr| {
                arr.iter()
                    .filter_map(|v| v.as_str().map(String::from))
                    .collect()
            })
            .ok_or_else(|| ApiError::Parse("missing models array".into()))
    }

    pub async fn get_mcp_servers(&self) -> Result<HashMap<String, McpServerDef>, ApiError> {
        let json = self.get_json::<serde_json::Value>("/config/mcp").await?;
        let servers = json
            .get("servers")
            .and_then(|v| v.as_object())
            .ok_or_else(|| ApiError::Parse("missing servers map".into()))?;
        let mut result = HashMap::new();
        for (name, def) in servers {
            if let Ok(mcp) = serde_json::from_value::<McpServerDef>(def.clone()) {
                result.insert(name.clone(), mcp);
            }
        }
        Ok(result)
    }

    pub async fn get_gateway_approvals(&self) -> Result<GatewayApprovals, ApiError> {
        let json = self
            .get_json::<serde_json::Value>("/gateway/approvals")
            .await?;
        serde_json::from_value(json).map_err(|e| ApiError::Parse(e.to_string()))
    }

    pub async fn list_bots(&self) -> Result<Vec<BotInfo>, ApiError> {
        let json = self.get_json::<serde_json::Value>("/gateway/bots").await?;
        json.get("bots")
            .and_then(|v| v.as_array())
            .map(|arr| {
                arr.iter()
                    .filter_map(|v| serde_json::from_value(v.clone()).ok())
                    .collect()
            })
            .ok_or_else(|| ApiError::Parse("missing bots array".into()))
    }

    pub async fn get_config(&self) -> Result<ConfigSnapshot, ApiError> {
        let json = self.get_json::<serde_json::Value>("/config").await?;
        serde_json::from_value(json).map_err(|e| ApiError::Parse(e.to_string()))
    }

    pub async fn get_control_state(&self, session_id: &str) -> Result<ControlState, ApiError> {
        let json = self
            .get_json::<serde_json::Value>(&format!("/sessions/{}/control-state", session_id))
            .await?;
        Ok(ControlState {
            running: json
                .get("running")
                .and_then(|v| v.as_bool())
                .unwrap_or(false),
            paused: json
                .get("paused")
                .and_then(|v| v.as_bool())
                .unwrap_or(false),
            revision: json.get("revision").and_then(|v| v.as_i64()).unwrap_or(0) as u64,
        })
    }

    pub async fn get_launch_servers(
        &self,
        session_id: &str,
    ) -> Result<Vec<LaunchServer>, ApiError> {
        let json = self
            .get_json::<serde_json::Value>(&format!("/sessions/{}/launch", session_id))
            .await?;
        let arr = json
            .get("servers")
            .and_then(|v| v.as_array())
            .ok_or_else(|| ApiError::Parse("missing servers array".into()))?;
        arr.iter()
            .map(|v| serde_json::from_value(v.clone()).map_err(|e| ApiError::Parse(e.to_string())))
            .collect()
    }

    pub async fn answer_approval(
        &self,
        session_id: &str,
        req_id: &str,
        approve: bool,
        remember: bool,
    ) -> Result<ApprovalAnswer, ApiError> {
        let json = self
            .post_json(
                &format!("/sessions/{}/approvals/{}", session_id, req_id),
                &serde_json::json!({ "approve": approve, "remember": remember }),
            )
            .await?;
        serde_json::from_value(json).map_err(|e| ApiError::Parse(e.to_string()))
    }

    pub async fn run_prompt(&self, session_id: &str, prompt: &str) -> Result<(), ApiError> {
        let _ = self
            .post_json(
                &format!("/sessions/{}/run", session_id),
                &serde_json::json!({ "prompt": prompt }),
            )
            .await?;
        Ok(())
    }

    pub async fn compact_session(&self, session_id: &str) -> Result<serde_json::Value, ApiError> {
        self.post_json(
            &format!("/sessions/{session_id}/compact"),
            &serde_json::json!({}),
        )
        .await
    }

    pub async fn send_steering(&self, session_id: &str, text: &str) -> Result<(), ApiError> {
        let _ = self
            .post_json(
                &format!("/sessions/{}/steering", session_id),
                &serde_json::json!({ "text": text }),
            )
            .await?;
        Ok(())
    }

    pub async fn set_permission_mode(&self, mode: &str) -> Result<(), ApiError> {
        let _ = self
            .post_json("/config/mode", &serde_json::json!({ "mode": mode }))
            .await?;
        Ok(())
    }

    pub async fn patch_config(&self, patch: &ConfigPatch) -> Result<(), ApiError> {
        let body = serde_json::to_value(patch).unwrap_or_default();
        let _ = self.post_json("/config", &body).await?;
        Ok(())
    }

    pub async fn start_launch(&self, session_id: &str, name: &str) -> Result<bool, ApiError> {
        let json = self
            .post_json(
                &format!("/sessions/{}/launch/start", session_id),
                &serde_json::json!({ "name": name }),
            )
            .await?;
        Ok(json
            .get("started")
            .and_then(|v| v.as_bool())
            .unwrap_or(false))
    }

    // ---- SSE streaming ----

    /// Connect to an SSE endpoint and drain ALL events from it through the
    /// channel, converting each parsed SSE frame into the appropriate
    /// `TerminalEvent` variant.  Returns `Ok(())` when the remote closes
    /// the stream; the caller is expected to reconnect.
    async fn drain_sse(
        client: &reqwest::Client,
        url: &str,
        token: &str,
        tx: &mpsc::UnboundedSender<TerminalEvent>,
        kind: SseKind,
    ) -> Result<(), ApiError> {
        let res = client
            .get(url)
            .header(AUTHORIZATION, token)
            .send()
            .await
            .map_err(|e| ApiError::Network(e.to_string()))?;

        if !res.status().is_success() {
            return Err(ApiError::Server {
                status: res.status().as_u16(),
                body: res.text().await.unwrap_or_default(),
            });
        }

        let mut buf: Vec<u8> = Vec::new();
        let mut stream = res.bytes_stream();
        while let Some(chunk) = stream.next().await {
            match chunk {
                Ok(bytes) => {
                    buf.extend_from_slice(&bytes);
                    for ev in parse_sse_buffer(&mut buf) {
                        let seq = ev
                            .id
                            .as_deref()
                            .and_then(|s| s.parse::<u64>().ok())
                            .unwrap_or(0);
                        let terminal_ev = match kind {
                            SseKind::Agent => TerminalEvent::Agent {
                                event: ev.event,
                                seq,
                            },
                            SseKind::Presentation => TerminalEvent::Presentation {
                                frame: ev.event,
                                seq,
                            },
                        };
                        if tx.send(terminal_ev).is_err() {
                            return Ok(()); // receiver dropped
                        }
                    }
                }
                Err(e) => return Err(ApiError::Network(e.to_string())),
            }
        }
        Ok(())
    }

    /// Spawn a background task that keeps the agent-event SSE stream
    /// open (reconnecting on disconnect) and forwards `AgentEvent` JSON
    /// objects through the channel.
    pub fn spawn_agent_event_watcher(
        &self,
        session_id: &str,
        tx: mpsc::UnboundedSender<TerminalEvent>,
    ) -> tokio::task::JoinHandle<()> {
        let url = self.url(&format!("/sessions/{}/events", session_id));
        let token = self.auth();
        let client = self.client.clone();

        tokio::spawn(async move {
            let _ = tx.send(TerminalEvent::Connected);
            loop {
                match Self::drain_sse(&client, &url, &token, &tx, SseKind::Agent).await {
                    Ok(()) => {
                        // Stream closed normally — wait briefly and reconnect.
                        tokio::time::sleep(std::time::Duration::from_secs(2)).await;
                    }
                    Err(e) => {
                        let _ = tx.send(TerminalEvent::Error(e.to_string()));
                        tokio::time::sleep(std::time::Duration::from_secs(2)).await;
                    }
                }
            }
        })
    }

    /// Spawn a background task that keeps the presentation SSE stream
    /// open and forwards timeline frames through the channel.
    pub fn spawn_presentation_watcher(
        &self,
        session_id: &str,
        tx: mpsc::UnboundedSender<TerminalEvent>,
    ) -> tokio::task::JoinHandle<()> {
        let url = self.url(&format!("/sessions/{}/presentation/events", session_id));
        let token = self.auth();
        let client = self.client.clone();

        tokio::spawn(async move {
            loop {
                match Self::drain_sse(&client, &url, &token, &tx, SseKind::Presentation).await {
                    Ok(()) => {
                        tokio::time::sleep(std::time::Duration::from_secs(2)).await;
                    }
                    Err(e) => {
                        let _ = tx.send(TerminalEvent::Error(e.to_string()));
                        tokio::time::sleep(std::time::Duration::from_secs(2)).await;
                    }
                }
            }
        })
    }

    /// Spawn a background task that polls `/health` periodically and
    /// forwards `HealthReport` through the channel.
    pub fn spawn_health_watcher(
        self: std::sync::Arc<Self>,
        tx: mpsc::UnboundedSender<TerminalEvent>,
    ) -> tokio::task::JoinHandle<()> {
        tokio::spawn(async move {
            let mut interval =
                tokio::time::interval(std::time::Duration::from_secs(HEALTH_POLL_INTERVAL_SECS));
            loop {
                interval.tick().await;
                match self.health().await {
                    Ok(report) => {
                        let _ = tx.send(TerminalEvent::Health(report));
                    }
                    Err(e) => {
                        let _ = tx.send(TerminalEvent::Error(e.to_string()));
                    }
                }
            }
        })
    }
}

#[derive(Debug, Clone, Copy)]
enum SseKind {
    Agent,
    Presentation,
}

const HEALTH_POLL_INTERVAL_SECS: u64 = 3;

// ---- Typed response structures ----------------------------------------------

/// Decoded from `GET /health`.
#[derive(Debug, Clone, Default)]
pub struct HealthReport {
    pub provider: String,
    pub model: String,
    pub permission_mode: String,
    pub sandbox: String,
    /// `None` when neither the model nor a setting states one.
    pub context_window: Option<u64>,
    pub cwd: String,
    pub healthy: bool,
    pub circuit_breaker_healthy: bool,
    pub failures: u32,
    pub warnings: Vec<String>,
}

impl HealthReport {
    pub fn from_json(json: serde_json::Value) -> Self {
        Self {
            provider: json
                .get("provider")
                .and_then(|v| v.as_str())
                .unwrap_or("unknown")
                .to_string(),
            model: json
                .get("model")
                .and_then(|v| v.as_str())
                .unwrap_or("unknown")
                .to_string(),
            permission_mode: json
                .get("permission_mode")
                .and_then(|v| v.as_str())
                .unwrap_or("unknown")
                .to_string(),
            sandbox: json
                .get("sandbox")
                .and_then(|v| v.as_str())
                .unwrap_or("unknown")
                .to_string(),
            context_window: json.get("context_window").and_then(|v| v.as_u64()),
            cwd: json
                .get("cwd")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string(),
            healthy: json.get("posture").and_then(|v| v.as_str()) == Some("healthy"),
            circuit_breaker_healthy: json.get("failures").and_then(|v| v.as_u64()).unwrap_or(0)
                == 0,
            failures: json.get("failures").and_then(|v| v.as_u64()).unwrap_or(0) as u32,
            warnings: json
                .get("warnings")
                .and_then(|v| v.as_array())
                .map(|arr| {
                    arr.iter()
                        .filter_map(|v| v.as_str().map(String::from))
                        .collect()
                })
                .unwrap_or_default(),
        }
    }
}

/// Decoded from `GET /sessions`.
#[derive(Debug, Clone, Deserialize)]
pub struct SessionInfo {
    pub session_id: String,
    pub cwd: Option<String>,
    pub created_at: Option<String>,
    pub updated_at: Option<String>,
    pub entries: Option<u64>,
    pub title: Option<String>,
    pub running: Option<bool>,
    pub archived: Option<bool>,
}

impl SessionInfo {
    /// A human-readable status string derived from the real server state.
    pub fn status(&self) -> &str {
        if self.running == Some(true) {
            "active"
        } else if self.archived == Some(true) {
            "archived"
        } else {
            "idle"
        }
    }
}

#[derive(Debug, Clone, Deserialize)]
pub struct AttachResponse {
    pub session_id: String,
}

/// Decoded from `GET /providers`.
#[derive(Debug, Clone, Deserialize, Default)]
pub struct ProviderList {
    pub current: String,
    pub current_model: String,
    pub current_configured: bool,
    pub providers: Vec<ProviderInfo>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct ProviderInfo {
    pub name: String,
    pub env_var: Option<String>,
    pub pool_env_var: Option<String>,
    pub pool_size: Option<usize>,
    pub credential_ids: Vec<String>,
    pub requires_key: bool,
    pub configured: bool,
}

/// Decoded from `GET /config`.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct ConfigSnapshot {
    #[serde(default)]
    pub provider: Option<String>,
    #[serde(default)]
    pub model: Option<String>,
    #[serde(default)]
    pub max_turns: Option<u32>,
    #[serde(default)]
    pub permission_mode: Option<String>,
    #[serde(default)]
    pub approval_mode: Option<String>,
    #[serde(default)]
    pub theme: Option<String>,
}

/// Patch body for `PATCH /config`.
#[derive(Debug, Clone, Serialize, Default)]
pub struct ConfigPatch {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub provider: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub permission_mode: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub approval_mode: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max_turns: Option<u32>,
}

/// MCP server definition from `GET /config/mcp`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct McpServerDef {
    pub command: String,
    #[serde(default)]
    pub args: Vec<String>,
    #[serde(default)]
    pub env: HashMap<String, String>,
    #[serde(default)]
    pub network: bool,
}

/// Decoded from `GET /gateway/approvals`.
#[derive(Debug, Clone, Deserialize, Default)]
pub struct GatewayApprovals {
    pub mode: String,
    pub approver: Option<String>,
    pub timeout_secs: u64,
    pub enabled: bool,
    pub forwarding: bool,
    pub candidates: Vec<String>,
}

/// Bot info from `GET /gateway/bots`.
#[derive(Debug, Clone, Deserialize)]
pub struct BotInfo {
    pub id: String,
    pub surface: String,
    pub name: String,
    pub configured: bool,
    #[serde(default)]
    pub policy_inherited: bool,
}

/// Decoded from `POST /sessions/{id}/approvals/{req_id}`.
#[derive(Debug, Clone, Deserialize)]
pub struct ApprovalAnswer {
    pub approved: bool,
    pub learned_rule: Option<String>,
    pub learn_error: Option<String>,
}

/// Decoded from `GET /sessions/{id}/control-state`.
#[derive(Debug, Clone, Default)]
pub struct ControlState {
    pub running: bool,
    pub paused: bool,
    pub revision: u64,
}

/// Decoded from `GET /sessions/{id}/launch`.
#[derive(Debug, Clone, Deserialize)]
pub struct LaunchServer {
    pub name: String,
    pub cmd: String,
    #[serde(default)]
    pub args: Vec<String>,
    pub port: Option<u16>,
    pub running: bool,
}

// ---- SSE frame parsing ------------------------------------------------------

/// Parsed SSE frame from the server's event streams.
#[derive(Debug, Clone)]
pub struct SseEvent {
    pub event: serde_json::Value,
    pub id: Option<String>,
}

/// Parsed SSE line: a field name and its value.
#[derive(Debug, Clone)]
struct SseLine {
    field: String,
    value: String,
}

impl SseLine {
    /// Parse a single SSE line (without the trailing newline).
    /// Returns `None` for comment lines (starting with `:`).
    fn parse(line: &str) -> Option<Self> {
        if line.is_empty() {
            return Some(SseLine {
                field: String::new(),
                value: String::new(),
            });
        }
        if line.starts_with(':') {
            return None;
        }
        if let Some(colon_pos) = line.find(':') {
            let field = &line[..colon_pos];
            let value = if line.as_bytes().get(colon_pos + 1) == Some(&b' ') {
                &line[colon_pos + 2..]
            } else {
                &line[colon_pos + 1..]
            };
            Some(SseLine {
                field: field.to_string(),
                value: value.to_string(),
            })
        } else {
            Some(SseLine {
                field: line.to_string(),
                value: String::new(),
            })
        }
    }
}

/// Consume complete SSE frames from `buf` (a byte buffer of raw event data),
/// returning decoded `SseEvent` objects. Bytes that don't form a complete
/// frame (no trailing blank line) remain in `buf` for the next call.
pub fn parse_sse_buffer(buf: &mut Vec<u8>) -> Vec<SseEvent> {
    let mut events = Vec::new();
    let mut data_lines: Vec<String> = Vec::new();
    let mut event_type = String::new();
    let mut id = None::<String>;

    while let Some(nl) = buf.iter().position(|&b| b == b'\n') {
        let line_bytes: Vec<u8> = buf.drain(..=nl).collect();
        let line_str = String::from_utf8_lossy(&line_bytes);
        let trimmed = line_str.trim_end_matches(['\r', '\n']);

        match SseLine::parse(trimmed) {
            None => {} // comment line
            Some(SseLine {
                field: f,
                value: _v,
            }) if f.is_empty() => {
                // blank line — dispatch accumulated event
                if !data_lines.is_empty() || !event_type.is_empty() || id.is_some() {
                    let data = data_lines.join("\n");
                    let event = serde_json::from_str::<serde_json::Value>(&data).unwrap_or_else(
                        |_| serde_json::json!({ "_raw": data, "_type": event_type }),
                    );
                    events.push(SseEvent {
                        event,
                        id: id.clone(),
                    });
                    data_lines.clear();
                    event_type.clear();
                    id = None;
                }
            }
            Some(SseLine { field: f, value: v }) if f == "data" => {
                data_lines.push(v);
            }
            Some(SseLine { field: f, value: v }) if f == "event" => {
                event_type = v;
            }
            Some(SseLine { field: f, value: v }) if f == "id" => {
                id = Some(v);
            }
            _ => {}
        }
    }

    events
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_sse_single_event() {
        let mut buf = b"id:42\ndata:{\"test\":true}\n\n".to_vec();
        let events = parse_sse_buffer(&mut buf);
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].id, Some("42".to_string()));
        assert_eq!(
            events[0].event.get("test").and_then(|v| v.as_bool()),
            Some(true)
        );
        assert!(buf.is_empty());
    }

    #[test]
    fn test_parse_sse_multi_line_data() {
        let mut buf = b"data:{\"a\":1,\ndata:\"b\":2}\n\n".to_vec();
        let events = parse_sse_buffer(&mut buf);
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].event.get("a").and_then(|v| v.as_i64()), Some(1));
        assert_eq!(events[0].event.get("b").and_then(|v| v.as_i64()), Some(2));
    }

    #[test]
    fn test_parse_sse_incomplete_buffer() {
        let mut buf = b"id:42\ndata:{\"test\":".to_vec();
        let events = parse_sse_buffer(&mut buf);
        assert!(events.is_empty());
        // The incomplete data should still be in the buffer
        assert!(!buf.is_empty());
    }

    #[test]
    fn test_parse_sse_comment_lines() {
        let mut buf = b":keepalive\ndata:{\"ok\":true}\n\n".to_vec();
        let events = parse_sse_buffer(&mut buf);
        assert_eq!(events.len(), 1);
        assert_eq!(
            events[0].event.get("ok").and_then(|v| v.as_bool()),
            Some(true)
        );
    }

    #[test]
    fn test_parse_sse_two_events() {
        let mut buf = b"id:1\ndata:{\"n\":1}\n\nid:2\ndata:{\"n\":2}\n\n".to_vec();
        let events = parse_sse_buffer(&mut buf);
        assert_eq!(events.len(), 2);
        assert_eq!(events[0].event.get("n").and_then(|v| v.as_i64()), Some(1));
        assert_eq!(events[1].event.get("n").and_then(|v| v.as_i64()), Some(2));
    }

    #[test]
    fn test_url_construction() {
        let client = ApiClient::new("http://localhost:8901", "mytoken");
        assert_eq!(client.url("/health"), "http://localhost:8901/health");
        assert_eq!(
            client.url("/sessions/abc/events"),
            "http://localhost:8901/sessions/abc/events"
        );
    }
}
