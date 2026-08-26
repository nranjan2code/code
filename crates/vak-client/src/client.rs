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
            return Err(ClientError::Http {
                status,
                body: text,
            });
        }
        Ok(serde_json::from_str(&text)?)
    }

    async fn post_empty(&self, path: &str) -> Result<StatusCode> {
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

    async fn post_json<B: serde::Serialize, T: serde::de::DeserializeOwned>(
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
            return Err(ClientError::Http {
                status,
                body: text,
            });
        }
        Ok(serde_json::from_str(&text)?)
    }

    async fn post_json_status<B: serde::Serialize>(
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

    async fn patch_json<B: serde::Serialize>(&self, path: &str, body: &B) -> Result<StatusCode> {
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

    async fn delete(&self, path: &str) -> Result<StatusCode> {
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

    pub fn events(
        &self,
        session_id: &str,
    ) -> impl Stream<Item = Result<AgentEvent>> + '_ {
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
        self.post_json("/backup/export", &serde_json::json!({})).await
    }
}
