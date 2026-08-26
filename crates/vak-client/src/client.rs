use crate::types::*;
use futures::stream::{Stream, StreamExt};
use reqwest::StatusCode;
use std::time::Duration;

#[derive(Debug, thiserror::Error)]
pub enum ClientError {
    #[error("HTTP {status}: {body}")]
    Http { status: StatusCode, body: String },

    #[error("request failed: {0}")]
    Reqwest(#[from] reqwest::Error),

    #[error("JSON: {0}")]
    Json(#[from] serde_json::Error),

    #[error("SSE stream ended unexpectedly")]
    StreamEnded,

    #[error("connection refused — is the base running?")]
    ConnectionRefused,
}

pub type Result<T> = std::result::Result<T, ClientError>;

/// Typed HTTP + SSE client for a vakcoder base.
///
/// Construct via `Client::new(url, token)`. All methods are async and
/// return typed responses. SSE streaming is exposed as a `futures::Stream`.
#[derive(Debug, Clone)]
pub struct Client {
    http: reqwest::Client,
    base_url: String,
    token: String,
}

impl Client {
    pub fn new(base_url: impl Into<String>, token: impl Into<String>) -> Self {
        Self {
            http: reqwest::Client::builder()
                .timeout(Duration::from_secs(30))
                .build()
                .expect("reqwest client"),
            base_url: base_url.into(),
            token: token.into(),
        }
    }

    fn url(&self, path: &str) -> String {
        format!("{}{path}", self.base_url)
    }

    fn auth(&self) -> reqwest::header::HeaderValue {
        reqwest::header::HeaderValue::from_str(&format!("Bearer {}", self.token))
            .expect("valid header value")
    }

    async fn get<T: serde::de::DeserializeOwned>(&self, path: &str) -> Result<T> {
        let resp = self
            .http
            .get(self.url(path))
            .header("authorization", self.auth())
            .send()
            .await
            .map_err(|e| {
                if e.is_connect() {
                    ClientError::ConnectionRefused
                } else {
                    ClientError::Reqwest(e)
                }
            })?;
        let status = resp.status();
        let text = resp.text().await?;
        if !status.is_success() {
            return Err(ClientError::Http { status, body: text });
        }
        Ok(serde_json::from_str(&text)?)
    }

    pub async fn get_raw(&self, path: &str) -> Result<(StatusCode, String)> {
        let resp = self
            .http
            .get(self.url(path))
            .header("authorization", self.auth())
            .send()
            .await
            .map_err(|e| {
                if e.is_connect() {
                    ClientError::ConnectionRefused
                } else {
                    ClientError::Reqwest(e)
                }
            })?;
        let status = resp.status();
        let text = resp.text().await?;
        Ok((status, text))
    }

    pub async fn post_empty(&self, path: &str) -> Result<StatusCode> {
        let resp = self
            .http
            .post(self.url(path))
            .header("authorization", self.auth())
            .send()
            .await
            .map_err(|e| {
                if e.is_connect() {
                    ClientError::ConnectionRefused
                } else {
                    ClientError::Reqwest(e)
                }
            })?;
        Ok(resp.status())
    }

    pub async fn post_json<B: serde::Serialize, T: serde::de::DeserializeOwned>(
        &self,
        path: &str,
        body: &B,
    ) -> Result<T> {
        let resp = self
            .http
            .post(self.url(path))
            .header("authorization", self.auth())
            .json(body)
            .send()
            .await
            .map_err(|e| {
                if e.is_connect() {
                    ClientError::ConnectionRefused
                } else {
                    ClientError::Reqwest(e)
                }
            })?;
        let status = resp.status();
        let text = resp.text().await?;
        if !status.is_success() {
            return Err(ClientError::Http { status, body: text });
        }
        Ok(serde_json::from_str(&text)?)
    }

    pub async fn post_json_status<B: serde::Serialize>(
        &self,
        path: &str,
        body: &B,
    ) -> Result<StatusCode> {
        let resp = self
            .http
            .post(self.url(path))
            .header("authorization", self.auth())
            .json(body)
            .send()
            .await
            .map_err(|e| {
                if e.is_connect() {
                    ClientError::ConnectionRefused
                } else {
                    ClientError::Reqwest(e)
                }
            })?;
        Ok(resp.status())
    }

    pub async fn patch_json<B: serde::Serialize>(
        &self,
        path: &str,
        body: &B,
    ) -> Result<StatusCode> {
        let resp = self
            .http
            .patch(self.url(path))
            .header("authorization", self.auth())
            .json(body)
            .send()
            .await
            .map_err(|e| {
                if e.is_connect() {
                    ClientError::ConnectionRefused
                } else {
                    ClientError::Reqwest(e)
                }
            })?;
        Ok(resp.status())
    }

    pub async fn delete(&self, path: &str) -> Result<StatusCode> {
        let resp = self
            .http
            .delete(self.url(path))
            .header("authorization", self.auth())
            .send()
            .await
            .map_err(|e| {
                if e.is_connect() {
                    ClientError::ConnectionRefused
                } else {
                    ClientError::Reqwest(e)
                }
            })?;
        Ok(resp.status())
    }

    pub async fn put_json<B: serde::Serialize, T: serde::de::DeserializeOwned>(
        &self,
        path: &str,
        body: &B,
    ) -> Result<T> {
        let resp = self
            .http
            .put(self.url(path))
            .header("authorization", self.auth())
            .json(body)
            .send()
            .await
            .map_err(|e| {
                if e.is_connect() {
                    ClientError::ConnectionRefused
                } else {
                    ClientError::Reqwest(e)
                }
            })?;
        let status = resp.status();
        let text = resp.text().await?;
        if !status.is_success() {
            return Err(ClientError::Http { status, body: text });
        }
        Ok(serde_json::from_str(&text)?)
    }

    pub async fn put_json_status<B: serde::Serialize>(
        &self,
        path: &str,
        body: &B,
    ) -> Result<StatusCode> {
        let resp = self
            .http
            .put(self.url(path))
            .header("authorization", self.auth())
            .json(body)
            .send()
            .await
            .map_err(|e| {
                if e.is_connect() {
                    ClientError::ConnectionRefused
                } else {
                    ClientError::Reqwest(e)
                }
            })?;
        Ok(resp.status())
    }

    pub async fn delete_json<B: serde::Serialize, T: serde::de::DeserializeOwned>(
        &self,
        path: &str,
        body: &B,
    ) -> Result<T> {
        let resp = self
            .http
            .delete(self.url(path))
            .header("authorization", self.auth())
            .json(body)
            .send()
            .await
            .map_err(|e| {
                if e.is_connect() {
                    ClientError::ConnectionRefused
                } else {
                    ClientError::Reqwest(e)
                }
            })?;
        let status = resp.status();
        let text = resp.text().await?;
        if !status.is_success() {
            return Err(ClientError::Http { status, body: text });
        }
        Ok(serde_json::from_str(&text)?)
    }

    // ── Health ───────────────────────────────────────────────────

    pub async fn health(&self) -> Result<HealthResponse> {
        self.get("/health").await
    }

    // ── Sessions ─────────────────────────────────────────────────

    pub async fn list_sessions(&self) -> Result<SessionListResponse> {
        self.get("/sessions").await
    }

    pub async fn create_session(&self, cwd: Option<&str>) -> Result<CreateSessionResponse> {
        self.post_json(
            "/sessions",
            &CreateSessionRequest {
                cwd: cwd.map(String::from),
            },
        )
        .await
    }

    pub async fn delete_session(&self, id: &str) -> Result<StatusCode> {
        self.delete(&format!("/sessions/{id}")).await
    }

    /// Resume a persisted session into the base's live-handle map. Returns
    /// the canonical session id (the header id can differ from the request).
    pub async fn attach_session(&self, id: &str) -> Result<String> {
        let resp: CreateSessionResponse = self
            .post_json(&format!("/sessions/{id}/attach"), &serde_json::json!({}))
            .await?;
        Ok(resp.session_id)
    }

    // ── Run / Steering ───────────────────────────────────────────

    pub async fn run_prompt(&self, session_id: &str, req: &RunRequest) -> Result<StatusCode> {
        self.post_json_status(&format!("/sessions/{session_id}/run"), req)
            .await
    }

    pub async fn steer(&self, session_id: &str, text: &str) -> Result<StatusCode> {
        self.post_json_status(
            &format!("/sessions/{session_id}/steering"),
            &SteeringRequest {
                text: text.to_string(),
            },
        )
        .await
    }

    pub async fn cancel_run(&self, session_id: &str) -> Result<StatusCode> {
        self.post_empty(&format!("/sessions/{session_id}/cancel"))
            .await
    }

    // ── SSE streaming ────────────────────────────────────────────

    pub fn events(&self, session_id: &str) -> impl Stream<Item = Result<AgentEvent>> + '_ {
        let url = self.url(&format!("/sessions/{session_id}/events"));
        let token = self.token.clone();
        let client = self.http.clone();

        async_stream::stream! {
            let resp = client
                .get(&url)
                .header("authorization", format!("Bearer {token}"))
                .send()
                .await;

            let resp = match resp {
                Ok(r) => r,
                Err(e) => {
                    if e.is_connect() {
                        yield Err(ClientError::ConnectionRefused);
                    } else {
                        yield Err(ClientError::Reqwest(e));
                    }
                    return;
                }
            };

            let mut buffer = String::new();
            let mut bytes_stream = resp.bytes_stream();

            while let Some(chunk) = bytes_stream.next().await {
                let chunk = match chunk {
                    Ok(c) => c,
                    Err(e) => {
                        yield Err(ClientError::Reqwest(e));
                        return;
                    }
                };

                buffer.push_str(&String::from_utf8_lossy(&chunk));

                while let Some(newline_pos) = buffer.find('\n') {
                    let line = buffer[..newline_pos].trim().to_string();
                    buffer = buffer[newline_pos + 1..].to_string();

                    if let Some(data) = line.strip_prefix("data: ") {
                        if data == "[DONE]" {
                            return;
                        }
                        match serde_json::from_str::<AgentEvent>(data) {
                            Ok(event) => yield Ok(event),
                            Err(e) => {
                                yield Err(ClientError::Json(e));
                                return;
                            }
                        }
                    }
                }
            }
        }
    }

    // ── Transcript ───────────────────────────────────────────────

    pub async fn transcript(&self, session_id: &str) -> Result<TranscriptResponse> {
        self.get(&format!("/sessions/{session_id}/transcript"))
            .await
    }

    // ── Config ───────────────────────────────────────────────────

    pub async fn config(&self) -> Result<ConfigResponse> {
        self.get("/config").await
    }

    pub async fn patch_config(&self, patch: &PatchConfigRequest) -> Result<StatusCode> {
        self.patch_json("/config", patch).await
    }

    // ── Search ───────────────────────────────────────────────────

    pub async fn search(
        &self,
        query: &str,
        limit: u32,
        all: bool,
        exclude: Option<&str>,
    ) -> Result<SearchResponse> {
        let mut url = format!("/search?q={query}&limit={limit}&all={all}");
        if let Some(ex) = exclude {
            url.push_str(&format!("&exclude={ex}"));
        }
        self.get(&url).await
    }

    // ── FinOps ───────────────────────────────────────────────────

    pub async fn finops(&self) -> Result<FinopsResponse> {
        self.get("/finops").await
    }

    // ── Checkpoints ──────────────────────────────────────────────

    pub async fn checkpoints(&self, session_id: &str) -> Result<Vec<Checkpoint>> {
        self.get(&format!("/sessions/{session_id}/checkpoints"))
            .await
    }

    // ── Skills ───────────────────────────────────────────────────

    pub async fn skills(&self) -> Result<Vec<Skill>> {
        self.get("/skills").await
    }

    // ── Custom commands ──────────────────────────────────────────

    pub async fn custom_commands(&self) -> Result<Vec<CustomCommandInfo>> {
        let resp: CustomCommandsResponse = self.get("/config/commands").await?;
        Ok(resp.commands)
    }

    // ── Tools / breaker introspection ────────────────────────────

    pub async fn tools(&self) -> Result<Vec<String>> {
        let resp: serde_json::Value = self.get("/tools").await?;
        Ok(resp
            .get("tools")
            .and_then(|t| t.as_array())
            .map(|a| {
                a.iter()
                    .filter_map(|v| v.as_str().map(String::from))
                    .collect()
            })
            .unwrap_or_default())
    }

    pub async fn breaker(&self) -> Result<serde_json::Value> {
        self.get("/breaker").await
    }

    // ── Memory ───────────────────────────────────────────────────

    pub async fn memory(&self) -> Result<Vec<MemoryNote>> {
        self.get("/memory").await
    }

    // ── Tasks ────────────────────────────────────────────────────

    pub async fn tasks(&self) -> Result<Vec<Task>> {
        self.get("/tasks").await
    }

    // ── Providers ────────────────────────────────────────────────

    pub async fn providers(&self) -> Result<Vec<ProviderInfo>> {
        self.get("/providers").await
    }

    pub async fn discover_models(&self, provider: &str) -> Result<Vec<ModelInfo>> {
        self.get(&format!("/providers/{provider}/models")).await
    }

    // ── Approvals ────────────────────────────────────────────────

    pub async fn answer_approval(
        &self,
        session_id: &str,
        approval_id: &str,
        approve: bool,
    ) -> Result<StatusCode> {
        let path = format!("/sessions/{session_id}/approvals/{approval_id}");
        let body = serde_json::json!({ "approve": approve });
        let resp = self
            .http
            .post(self.url(&path))
            .header("authorization", self.auth())
            .json(&body)
            .send()
            .await
            .map_err(|e| {
                if e.is_connect() {
                    ClientError::ConnectionRefused
                } else {
                    ClientError::Reqwest(e)
                }
            })?;
        Ok(resp.status())
    }

    // ── Doctor ───────────────────────────────────────────────────

    pub async fn doctor(&self) -> Result<serde_json::Value> {
        self.get("/doctor").await
    }

    // ── Ops ──────────────────────────────────────────────────────

    pub async fn ops_status(&self) -> Result<serde_json::Value> {
        self.get("/ops/status").await
    }

    // ── Backup ───────────────────────────────────────────────────

    pub async fn backup_export(&self) -> Result<serde_json::Value> {
        self.post_json("/backup/export", &serde_json::json!({}))
            .await
    }

    // ── Inbox ────────────────────────────────────────────────────

    pub async fn inbox_list(&self, limit: u32, unread_only: bool) -> Result<InboxResponse> {
        let mut url = format!("/inbox?limit={limit}");
        if unread_only {
            url.push_str("&unread=true");
        }
        self.get(&url).await
    }

    pub async fn inbox_ack(&self, id: &str) -> Result<InboxAckResponse> {
        self.post_json(&format!("/inbox/{id}/ack"), &serde_json::json!({}))
            .await
    }

    pub async fn inbox_unread_count(&self) -> Result<UnreadCountResponse> {
        self.get("/inbox/unread_count").await
    }

    // ── Task management ──────────────────────────────────────────

    pub async fn task_enable(&self, id: &str) -> Result<StatusCode> {
        self.patch_json(
            &format!("/tasks/{id}"),
            &serde_json::json!({ "enabled": true }),
        )
        .await
    }

    pub async fn task_disable(&self, id: &str) -> Result<StatusCode> {
        self.patch_json(
            &format!("/tasks/{id}"),
            &serde_json::json!({ "enabled": false }),
        )
        .await
    }

    pub async fn task_run_now(&self, id: &str) -> Result<StatusCode> {
        self.post_empty(&format!("/tasks/{id}/run-now")).await
    }

    pub async fn proposals_list(&self) -> Result<ProposalsResponse> {
        self.get("/skills/proposals").await
    }

    pub async fn proposal_promote(&self, id: &str) -> Result<serde_json::Value> {
        self.post_json(
            &format!("/skills/proposals/{id}/promote"),
            &serde_json::json!({}),
        )
        .await
    }

    pub async fn proposal_reject(&self, id: &str) -> Result<serde_json::Value> {
        self.post_json(
            &format!("/skills/proposals/{id}/reject"),
            &serde_json::json!({}),
        )
        .await
    }

    // ── Provider key management ──────────────────────────────────

    pub async fn set_provider_key(&self, provider: &str, key: &str) -> Result<ProviderKeyResponse> {
        self.put_json(
            "/config/key",
            &ProviderKeyRequest {
                provider: provider.to_string(),
                key: key.to_string(),
            },
        )
        .await
    }

    pub async fn remove_provider_key(&self, provider: &str) -> Result<ProviderRemoveResponse> {
        self.delete_json(
            "/config/key",
            &ProviderRef {
                provider: provider.to_string(),
            },
        )
        .await
    }

    // ── Subagents ────────────────────────────────────────────────

    pub async fn subagents_list(&self, session_id: &str) -> Result<SubagentsResponse> {
        self.get(&format!("/sessions/{session_id}/subagents")).await
    }

    pub async fn subagent_steer(
        &self,
        session_id: &str,
        child: &str,
        text: &str,
    ) -> Result<StatusCode> {
        self.post_json_status(
            &format!("/sessions/{session_id}/subagents/{child}/steer"),
            &SteeringRequest {
                text: text.to_string(),
            },
        )
        .await
    }

    pub async fn subagent_stop(&self, session_id: &str, child: &str) -> Result<StatusCode> {
        self.post_empty(&format!("/sessions/{session_id}/subagents/{child}/stop"))
            .await
    }

    // ── Checkpoint restore ───────────────────────────────────────

    pub async fn checkpoint_restore(
        &self,
        session_id: &str,
        seq: u32,
    ) -> Result<CheckpointRestoreResponse> {
        self.post_json(
            &format!("/sessions/{session_id}/checkpoints/{seq}/restore"),
            &serde_json::json!({}),
        )
        .await
    }

    // ── Compaction ───────────────────────────────────────────────

    pub async fn compact(&self, session_id: &str) -> Result<serde_json::Value> {
        self.post_json(
            &format!("/sessions/{session_id}/compact"),
            &serde_json::json!({}),
        )
        .await
    }

    pub async fn set_permission_mode(&self, mode: &str) -> Result<StatusCode> {
        self.post_json_status("/config/mode", &serde_json::json!({ "mode": mode }))
            .await
    }

    // ── Sandbox backend ──────────────────────────────────────────

    pub async fn sandbox_info(&self) -> Result<serde_json::Value> {
        self.get("/config/sandbox").await
    }

    pub async fn set_sandbox_backend(&self, backend: Option<&str>) -> Result<StatusCode> {
        self.post_json_status(
            "/config/sandbox",
            &serde_json::json!({ "backend": backend }),
        )
        .await
    }

    // ── MCP hot-apply ────────────────────────────────────────────

    pub async fn mcp_apply(&self, servers: &serde_json::Value) -> Result<StatusCode> {
        self.put_json_status("/config/mcp", servers).await
    }
}
