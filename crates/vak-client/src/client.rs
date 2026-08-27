use crate::types::*;
use async_stream::try_stream;
use futures::{Stream, StreamExt};
use reqwest::{StatusCode, Url, header};
use serde::{Deserialize, Serialize, de::DeserializeOwned};
use serde_json::Value;
use std::{pin::Pin, time::Duration};
use vak_domain::{ProjectContext, ProjectId, RunId, SessionId};

#[derive(Debug, thiserror::Error)]
pub enum ClientError {
    #[error("invalid base URL")]
    Url,
    #[error("invalid authorization token")]
    InvalidToken,
    #[error("HTTP {status}: {body}")]
    Http { status: StatusCode, body: String },
    #[error("request failed: {0}")]
    Request(#[from] reqwest::Error),
    #[error("invalid JSON: {0}")]
    Json(#[from] serde_json::Error),
    #[error("SSE stream ended unexpectedly")]
    StreamEnded,
}

pub type Result<T> = std::result::Result<T, ClientError>;
pub type EventStream = Pin<Box<dyn Stream<Item = Result<ServerEvent>> + Send>>;

#[derive(Clone)]
pub struct Client {
    http: reqwest::Client,
    base: Url,
    token: String,
}

impl std::fmt::Debug for Client {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("Client")
            .field("base", &self.base)
            .field("token", &"[REDACTED]")
            .finish()
    }
}

#[derive(Debug, Default)]
pub struct ClientBuilder {
    base: Option<String>,
    token: Option<String>,
    timeout: Option<Duration>,
}

impl ClientBuilder {
    pub fn new() -> Self {
        Self::default()
    }
    pub fn base_url(mut self, url: impl Into<String>) -> Self {
        self.base = Some(url.into());
        self
    }
    pub fn token(mut self, token: impl Into<String>) -> Self {
        self.token = Some(token.into());
        self
    }
    pub fn timeout(mut self, timeout: Duration) -> Self {
        self.timeout = Some(timeout);
        self
    }
    pub fn build(self) -> Result<Client> {
        let raw = self.base.unwrap_or_else(|| "http://127.0.0.1:7420".into());
        let base = Url::parse(&raw).map_err(|_| ClientError::Url)?;
        let token = self.token.unwrap_or_default();
        if token.is_empty() {
            return Err(ClientError::InvalidToken);
        }
        let mut builder = reqwest::Client::builder();
        if let Some(timeout) = self.timeout {
            builder = builder.timeout(timeout);
        }
        let http = builder.build()?;
        Ok(Client { http, base, token })
    }
}

impl Client {
    pub fn new(base: impl Into<String>, token: impl Into<String>) -> Result<Self> {
        ClientBuilder::new().base_url(base).token(token).build()
    }

    fn endpoint(&self, path: &str) -> Result<Url> {
        self.base
            .join(path.trim_start_matches('/'))
            .map_err(|_| ClientError::Url)
    }

    fn request(&self, method: reqwest::Method, path: &str) -> Result<reqwest::RequestBuilder> {
        let mut req = self.http.request(method, self.endpoint(path)?);
        if !self.token.is_empty() {
            let value = header::HeaderValue::from_str(&format!("Bearer {}", self.token))
                .map_err(|_| ClientError::InvalidToken)?;
            req = req.header(header::AUTHORIZATION, value);
        }
        Ok(req)
    }

    async fn decode<T: DeserializeOwned>(&self, response: reqwest::Response) -> Result<T> {
        let status = response.status();
        let body = response.text().await?;
        if !status.is_success() {
            return Err(ClientError::Http { status, body });
        }
        Ok(serde_json::from_str(&body)?)
    }

    async fn get<T: DeserializeOwned>(&self, path: &str) -> Result<T> {
        self.decode(self.request(reqwest::Method::GET, path)?.send().await?)
            .await
    }

    async fn json<T: DeserializeOwned, B: Serialize>(
        &self,
        method: reqwest::Method,
        path: &str,
        body: &B,
    ) -> Result<T> {
        let response = self.request(method, path)?.json(body).send().await?;
        self.decode(response).await
    }

    pub async fn health(&self) -> Result<Health> {
        self.get("/health").await
    }
    pub async fn version(&self) -> Result<Version> {
        self.get("/version").await
    }
    pub async fn projects(&self) -> Result<Vec<ProjectContext>> {
        Ok(self.get::<ProjectList>("/projects").await?.items)
    }
    pub async fn register_project(&self, project: &ProjectContext) -> Result<ProjectContext> {
        self.json(reqwest::Method::POST, "/projects", project).await
    }
    pub async fn sessions(&self, project_id: Option<&ProjectId>) -> Result<Vec<Session>> {
        let path = project_id.map_or_else(
            || "/sessions".into(),
            |id| format!("/sessions?project_id={id}"),
        );
        self.get(&path).await
    }
    pub async fn create_session(&self, request: &CreateSession) -> Result<SessionCreated> {
        self.json(reqwest::Method::POST, "/sessions", request).await
    }
    pub async fn transcript(&self, session_id: &SessionId) -> Result<Transcript> {
        self.get(&format!("/sessions/{session_id}/transcript"))
            .await
    }
    pub async fn start_run(&self, request: &StartRun) -> Result<StartRunResponse> {
        self.json(reqwest::Method::POST, "/runs", request).await
    }
    pub async fn run(&self, id: &RunId) -> Result<RunSnapshot> {
        self.get(&format!("/runs/{id}")).await
    }
    pub async fn cancel_run(&self, id: &RunId, reason: Option<&str>) -> Result<CancelResponse> {
        self.json(
            reqwest::Method::POST,
            &format!("/runs/{id}/cancel"),
            &serde_json::json!({"reason": reason}),
        )
        .await
    }
    pub async fn events(&self, _id: &RunId) -> Result<EventStream> {
        let id = _id;
        let response = self
            .request(reqwest::Method::GET, &format!("/events?run_id={id}"))?
            .header(header::ACCEPT, "text/event-stream")
            .send()
            .await?;
        let status = response.status();
        if !status.is_success() {
            let body = response.text().await?;
            return Err(ClientError::Http { status, body });
        }
        let mut bytes = response.bytes_stream();
        let stream = try_stream! {
            let mut buffer = String::new();
            while let Some(chunk) = bytes.next().await {
                buffer.push_str(&String::from_utf8_lossy(&chunk?));
                while let Some(pos) = buffer.find("\n\n") {
                    let frame = buffer[..pos].replace('\r', "");
                    buffer.drain(..pos + 2);
                    let data = frame.lines().filter_map(|line| line.strip_prefix("data:")).map(str::trim).collect::<Vec<_>>().join("\n");
                    if data.is_empty() { continue; }
                    if data == "[DONE]" { return; }
                    yield serde_json::from_str::<ServerEvent>(&data)?;
                }
            }
        };
        Ok(Box::pin(stream))
    }
    pub async fn config(&self, project_id: &ProjectId) -> Result<Config> {
        self.get(&format!("/config?project_id={project_id}")).await
    }
    pub async fn patch_config(&self, patch: &ConfigPatch) -> Result<Config> {
        self.json(reqwest::Method::PATCH, "/config", patch).await
    }
    pub async fn diagnostics(&self) -> Result<Diagnostics> {
        self.get("/diagnostics").await
    }
    pub async fn tasks(&self, project_id: Option<&ProjectId>) -> Result<Vec<Task>> {
        let path = project_id.map_or_else(
            || "/tasks".to_owned(),
            |id| format!("/tasks?project_id={id}"),
        );
        Ok(self.get::<TaskList>(&path).await?.items)
    }
    pub async fn create_task(&self, task: &serde_json::Value) -> Result<Task> {
        self.json(reqwest::Method::POST, "/tasks", task).await
    }
    pub async fn update_task(&self, id: &str, patch: &serde_json::Value) -> Result<Task> {
        self.json(reqwest::Method::PATCH, &format!("/tasks/{id}"), patch)
            .await
    }
    pub async fn delete_task(&self, id: &str) -> Result<bool> {
        Ok(self
            .json::<serde_json::Value, _>(
                reqwest::Method::DELETE,
                &format!("/tasks/{id}"),
                &serde_json::json!({}),
            )
            .await?
            .get("deleted")
            .and_then(serde_json::Value::as_bool)
            .unwrap_or(false))
    }
    pub async fn memory(&self, project_id: Option<&ProjectId>, scope: &str) -> Result<Vec<Memory>> {
        let mut path = format!("/memory?scope={scope}");
        if let Some(id) = project_id {
            path.push_str(&format!("&project_id={id}"));
        }
        Ok(self.get::<MemoryList>(&path).await?.items)
    }
    pub async fn create_memory<B: Serialize>(&self, request: &B) -> Result<Memory> {
        self.json(reqwest::Method::POST, "/memory", request).await
    }
    pub async fn amend_memory(&self, id: &str, text: &str) -> Result<Memory> {
        self.json(
            reqwest::Method::PATCH,
            &format!("/memory/{id}"),
            &serde_json::json!({"text": text}),
        )
        .await
    }
    pub async fn forget_memory(&self, id: &str) -> Result<bool> {
        Ok(self
            .json::<serde_json::Value, _>(
                reqwest::Method::DELETE,
                &format!("/memory/{id}"),
                &serde_json::json!({}),
            )
            .await?
            .get("forgotten")
            .and_then(serde_json::Value::as_bool)
            .unwrap_or(false))
    }
    pub async fn approvals(&self) -> Result<Vec<Approval>> {
        Ok(self.get::<ApprovalList>("/approvals").await?.items)
    }
    pub async fn resolve_approval(
        &self,
        id: &str,
        allow: bool,
        response: Value,
    ) -> Result<Approval> {
        self.json(
            reqwest::Method::POST,
            &format!("/approvals/{id}/resolve"),
            &serde_json::json!({"allow": allow, "response": response}),
        )
        .await
    }
    pub async fn skills(
        &self,
        project_id: Option<&ProjectId>,
        status: Option<&str>,
    ) -> Result<Vec<Skill>> {
        let mut path = "/skills".to_owned();
        let mut query = Vec::new();
        if let Some(id) = project_id {
            query.push(format!("project_id={id}"));
        }
        if let Some(status) = status {
            query.push(format!("status={status}"));
        }
        if !query.is_empty() {
            path.push('?');
            path.push_str(&query.join("&"));
        }
        Ok(self.get::<SkillList>(&path).await?.items)
    }
    pub async fn promote_skill(&self, id: &str) -> Result<Skill> {
        self.json(
            reqwest::Method::POST,
            &format!("/skills/{id}/promote"),
            &serde_json::json!({}),
        )
        .await
    }
    pub async fn reject_skill(&self, id: &str) -> Result<Skill> {
        self.json(
            reqwest::Method::POST,
            &format!("/skills/{id}/reject"),
            &serde_json::json!({}),
        )
        .await
    }
    pub async fn change_permission_mode(&self, request: &PermissionModeRequest) -> Result<Config> {
        self.json(reqwest::Method::POST, "/config/permission-mode", request)
            .await
    }
    pub async fn checkpoints(&self, session_id: &SessionId) -> Result<Vec<Checkpoint>> {
        Ok(self
            .get::<CheckpointList>(&format!(
                "/sessions/{session_id}/checkpoints?include_manifest=true"
            ))
            .await?
            .items)
    }
    pub async fn create_checkpoint(
        &self,
        session_id: &SessionId,
        request: &CheckpointCreate,
    ) -> Result<Checkpoint> {
        self.json(
            reqwest::Method::POST,
            &format!("/sessions/{session_id}/checkpoints"),
            request,
        )
        .await
    }
    pub async fn checkpoint(&self, session_id: &SessionId, id: &str) -> Result<Checkpoint> {
        self.get(&format!("/sessions/{session_id}/checkpoints/{id}"))
            .await
    }
    pub async fn restore_checkpoint(
        &self,
        session_id: &SessionId,
        id: &str,
    ) -> Result<RestoreResponse> {
        self.json(
            reqwest::Method::POST,
            &format!("/sessions/{session_id}/checkpoints/{id}/restore"),
            &serde_json::json!({}),
        )
        .await
    }
    pub async fn backup_export(&self, request: &BackupExportRequest) -> Result<BackupReport> {
        self.json(reqwest::Method::POST, "/backup/export", request)
            .await
    }
    pub async fn backup_import(&self, request: &BackupImportRequest) -> Result<BackupReport> {
        self.json(reqwest::Method::POST, "/backup/import", request)
            .await
    }
    pub async fn flows(&self, project_id: &ProjectId) -> Result<Vec<FlowDefinition>> {
        Ok(self
            .get::<FlowList>(&format!("/flows?project_id={project_id}"))
            .await?
            .items)
    }
    pub async fn check_flow(&self, name: &str, project_id: &ProjectId) -> Result<FlowDefinition> {
        self.json(
            reqwest::Method::POST,
            &format!("/flows/{name}/check?project_id={project_id}"),
            &serde_json::json!({}),
        )
        .await
    }
    pub async fn run_flow(&self, name: &str, request: &FlowRunRequest) -> Result<StartRunResponse> {
        self.json(
            reqwest::Method::POST,
            &format!("/flows/{name}/run"),
            request,
        )
        .await
    }
    pub async fn exec(&self, request: &StartRun) -> Result<StartRunResponse> {
        self.json(reqwest::Method::POST, "/exec", request).await
    }
    pub async fn eval(&self, request: &EvalRequest) -> Result<EvalReport> {
        self.json(reqwest::Method::POST, "/eval", request).await
    }
}

#[derive(Deserialize)]
struct TaskList {
    items: Vec<Task>,
}

#[derive(Deserialize)]
struct MemoryList {
    items: Vec<Memory>,
}

#[derive(Deserialize)]
struct ApprovalList {
    items: Vec<Approval>,
}

#[derive(Deserialize)]
struct SkillList {
    items: Vec<Skill>,
}

#[derive(Deserialize)]
struct CheckpointList {
    items: Vec<Checkpoint>,
}

#[derive(Deserialize)]
struct FlowList {
    items: Vec<FlowDefinition>,
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::*;
    use axum::{Router, extract::State, http::StatusCode, response::IntoResponse, routing::get};
    use std::net::SocketAddr;

    #[tokio::test]
    async fn health_is_typed_and_authorized() {
        async fn route(
            State(_token): State<String>,
            headers: axum::http::HeaderMap,
        ) -> impl IntoResponse {
            assert_eq!(
                headers
                    .get(header::AUTHORIZATION)
                    .and_then(|v| v.to_str().ok()),
                Some("Bearer secret")
            );
            (
                StatusCode::OK,
                serde_json::json!({"status":"ok","protocol":1,"runtime_id":"r"}).to_string(),
            )
        }
        let app = Router::new()
            .route("/health", get(route))
            .with_state("secret".to_string());
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr: SocketAddr = listener.local_addr().unwrap();
        tokio::spawn(async move {
            axum::serve(listener, app).await.unwrap();
        });
        let health = Client::new(format!("http://{addr}"), "secret")
            .unwrap()
            .health()
            .await
            .unwrap();
        assert_eq!(health.protocol, 1);
    }

    #[test]
    fn sse_event_round_trips() {
        let event = ServerEvent::Domain(vak_domain::Event::ConfigChanged { revision: 7 });
        let json = serde_json::to_string(&event).unwrap();
        assert!(json.contains("ConfigChanged"));
    }
}
