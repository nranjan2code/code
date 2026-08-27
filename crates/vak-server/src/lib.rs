//! Greenfield HTTP/SSE adapter.
//!
//! This crate is deliberately a transport adapter. It owns no sessions,
//! runs, configuration or filesystem state; those operations are delegated
//! to [`vak_runtime::Runtime`].

use axum::http::header::{CACHE_CONTROL, CONTENT_TYPE, SET_COOKIE};
use axum::{
    Json, Router,
    extract::{Path, Query, State},
    http::StatusCode,
    middleware,
    response::IntoResponse,
    response::sse::{Event as SseEvent, KeepAlive, Sse},
    routing::{get, post},
};
use futures::{Stream, StreamExt};
use include_dir::{Dir, include_dir};
use serde::{Deserialize, Serialize};
use std::{
    collections::HashMap,
    convert::Infallible,
    sync::{
        Arc,
        atomic::{AtomicU64, Ordering},
    },
};
use tokio::sync::Mutex;
use tokio::task::JoinHandle;
use tokio_stream::wrappers::BroadcastStream;
use vak_domain::{Event, ProjectContext, ProjectId, RunId, SessionContract, SessionId, TaskId};
use vak_runtime::{RunSnapshot, Runtime, RuntimeError};

#[derive(Clone)]
pub struct AppState {
    pub runtime: Arc<Runtime>,
    auth_token: Arc<String>,
    tasks: Arc<Mutex<HashMap<RunId, JoinHandle<()>>>>,
    event_ids: Arc<AtomicU64>,
}

#[derive(Debug, thiserror::Error)]
pub enum RouterError {
    #[error("Runtime HTTP authentication token must not be empty")]
    EmptyAuthToken,
}

impl AppState {
    pub fn new(runtime: Arc<Runtime>) -> Self {
        Self::with_token(runtime, String::new())
    }

    fn with_token(runtime: Arc<Runtime>, token: String) -> Self {
        Self {
            runtime,
            auth_token: Arc::new(token),
            tasks: Arc::new(Mutex::new(HashMap::new())),
            event_ids: Arc::new(AtomicU64::new(0)),
        }
    }

    pub async fn shutdown(&self) {
        let mut tasks = self.tasks.lock().await;
        for (_, task) in tasks.drain() {
            task.abort();
        }
    }
}

pub fn router(runtime: Arc<Runtime>) -> Result<Router, RouterError> {
    let token = std::env::var("VAKCODER_GATEWAY_TOKEN")
        .ok()
        .filter(|value| !value.is_empty())
        .ok_or(RouterError::EmptyAuthToken)?;
    router_with_token(runtime, token)
}

pub fn router_with_token(
    runtime: Arc<Runtime>,
    token: impl Into<String>,
) -> Result<Router, RouterError> {
    let token = token.into();
    if token.is_empty() {
        return Err(RouterError::EmptyAuthToken);
    }
    Ok(build_router(
        AppState::with_token(runtime, token.clone()),
        token,
    ))
}

fn build_router(state: AppState, token: String) -> Router {
    let api = Router::new()
        .route("/health", get(health))
        .route("/version", get(version))
        .route("/projects", get(list_projects).post(register_project))
        .route("/sessions", post(create_session))
        .route("/sessions", get(list_sessions))
        .route("/sessions/{session_id}/transcript", get(transcript))
        .route("/runs", post(start_run))
        .route("/runs", get(list_runs))
        .route("/runs/{run_id}", get(get_run))
        .route("/runs/{run_id}/cancel", post(cancel_run))
        .route("/events", get(events))
        .route("/config", get(get_config).patch(update_config))
        .route("/config/permission-mode", post(update_permission_mode))
        .route("/tasks", get(list_tasks).post(create_task))
        .route(
            "/tasks/{task_id}",
            axum::routing::patch(update_task).delete(delete_task),
        )
        .route("/inbox", get(list_inbox))
        .route("/inbox/{id}/ack", post(ack_inbox))
        .route("/approvals", get(list_approvals))
        .route("/approvals/{id}/resolve", post(resolve_approval))
        .route("/memory", get(list_memory).post(create_memory))
        .route(
            "/memory/{id}",
            axum::routing::patch(amend_memory).delete(forget_memory),
        )
        .route("/skills", get(list_skills))
        .route("/skills/{id}/promote", post(promote_skill))
        .route("/skills/{id}/reject", post(reject_skill))
        .route(
            "/sessions/{session_id}/checkpoints",
            get(list_checkpoints).post(create_checkpoint),
        )
        .route(
            "/sessions/{session_id}/checkpoints/{checkpoint_id}",
            get(get_checkpoint),
        )
        .route(
            "/sessions/{session_id}/checkpoints/{checkpoint_id}/restore",
            post(restore_checkpoint),
        )
        .route("/backup/export", post(backup_export))
        .route("/backup/import", post(backup_import))
        .route("/flows", get(list_flows))
        .route("/flows/{name}/check", get(check_flow).post(check_flow))
        .route("/flows/{name}/run", post(run_flow))
        .route("/eval", post(run_eval))
        .route("/exec", post(exec_run))
        .route("/diagnostics", get(diagnostics));
    let api = api
        .route("/providers", get(list_providers))
        .route("/providers/{provider}/models", get(discover_models))
        .route(
            "/providers/{provider}/key",
            post(set_provider_key).delete(remove_provider_key),
        )
        .route("/fs/tree", get(fs_tree))
        .route("/fs/file", get(read_file).put(write_file));
    let public = Router::new()
        .route("/auth/login", post(login))
        .route("/admin", get(admin_index))
        .route("/admin/", get(admin_index))
        .route("/admin/{*path}", get(admin_asset));
    Router::new()
        .merge(public)
        .merge(api)
        .with_state(state)
        .layer(middleware::from_fn_with_state(token, require_bearer))
}

#[derive(Debug, thiserror::Error)]
pub enum ServeError {
    #[error(transparent)]
    Router(#[from] RouterError),
    #[error("cannot bind Runtime HTTP listener: {0}")]
    Bind(std::io::Error),
    #[error("Runtime HTTP server failed: {0}")]
    Serve(std::io::Error),
}

pub async fn serve(
    runtime: Arc<Runtime>,
    address: std::net::SocketAddr,
    token: impl Into<String>,
    shutdown: impl std::future::Future<Output = ()> + Send + 'static,
) -> Result<(), ServeError> {
    let token = token.into();
    if token.is_empty() {
        return Err(RouterError::EmptyAuthToken.into());
    }
    let state = AppState::with_token(runtime, token.clone());
    let app = build_router(state.clone(), token.clone());
    let listener = tokio::net::TcpListener::bind(address)
        .await
        .map_err(ServeError::Bind)?;
    let runtime_file = runtime_file_path(state.runtime.data_home());
    write_runtime_file(
        &runtime_file,
        listener.local_addr().map_err(ServeError::Bind)?,
        &token,
    )
    .map_err(ServeError::Bind)?;
    axum::serve(listener, app)
        .with_graceful_shutdown(shutdown)
        .await
        .map_err(ServeError::Serve)?;
    state.shutdown().await;
    let _ = std::fs::remove_file(runtime_file);
    Ok(())
}

fn runtime_file_path(data_home: &std::path::Path) -> std::path::PathBuf {
    data_home.join("runtime/gateway.json")
}

fn write_runtime_file(
    path: &std::path::Path,
    address: std::net::SocketAddr,
    token: &str,
) -> Result<(), std::io::Error> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let temp = path.with_extension(format!("tmp.{}", std::process::id()));
    let value =
        serde_json::json!({"pid": std::process::id(), "addr": address.to_string(), "token": token});
    std::fs::write(
        &temp,
        serde_json::to_vec(&value).map_err(std::io::Error::other)?,
    )?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&temp, std::fs::Permissions::from_mode(0o600))?;
    }
    std::fs::rename(temp, path)
}

async fn require_bearer(
    axum::extract::State(token): axum::extract::State<String>,
    request: axum::http::Request<axum::body::Body>,
    next: middleware::Next,
) -> axum::response::Response {
    let path = request.uri().path();
    if path == "/auth/login" || path == "/admin" || path == "/admin/" || path.starts_with("/admin/")
    {
        return next.run(request).await;
    }
    let valid = request
        .headers()
        .get(axum::http::header::AUTHORIZATION)
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.strip_prefix("Bearer "))
        .is_some_and(|provided| provided == token)
        || request
            .headers()
            .get(axum::http::header::COOKIE)
            .and_then(|value| value.to_str().ok())
            .and_then(|cookies| {
                cookies
                    .split(';')
                    .find_map(|cookie| cookie.trim().strip_prefix("vakcoder_session="))
            })
            .is_some_and(|provided| provided == token);
    if valid {
        next.run(request).await
    } else {
        (
            StatusCode::UNAUTHORIZED,
            Json(serde_json::json!({"error":"authentication required"})),
        )
            .into_response()
    }
}

#[derive(Deserialize)]
struct LoginRequest {
    token: String,
}

async fn login(
    axum::extract::State(state): axum::extract::State<AppState>,
    Json(request): Json<LoginRequest>,
) -> impl IntoResponse {
    if request.token != *state.auth_token {
        return (
            StatusCode::UNAUTHORIZED,
            Json(serde_json::json!({"error":"invalid token"})),
        )
            .into_response();
    }
    let cookie = format!(
        "vakcoder_session={}; HttpOnly; SameSite=Strict; Path=/; Max-Age=86400",
        state.auth_token
    );
    (
        StatusCode::OK,
        [(SET_COOKIE, cookie)],
        Json(serde_json::json!({"authenticated":true})),
    )
        .into_response()
}

static ADMIN_UI: Dir<'_> = include_dir!("$CARGO_MANIFEST_DIR/../vak-admin-ui/dist");

fn admin_file(path: &str) -> axum::response::Response {
    let path = if path.is_empty() { "index.html" } else { path };
    let Some(file) = ADMIN_UI.get_file(path) else {
        return (StatusCode::NOT_FOUND, "not found").into_response();
    };
    let mime = match path.rsplit('.').next() {
        Some("html") => "text/html; charset=utf-8",
        Some("js") => "text/javascript; charset=utf-8",
        Some("css") => "text/css; charset=utf-8",
        Some("svg") => "image/svg+xml",
        _ => "application/octet-stream",
    };
    let cache = if path.starts_with("assets/") {
        "public, max-age=31536000, immutable"
    } else {
        "no-cache"
    };
    (
        [(CONTENT_TYPE, mime), (CACHE_CONTROL, cache)],
        file.contents(),
    )
        .into_response()
}

async fn admin_index() -> axum::response::Response {
    admin_file("index.html")
}

async fn admin_asset(Path(path): Path<String>) -> axum::response::Response {
    admin_file(&path)
}

#[derive(Serialize)]
struct Health {
    status: &'static str,
    protocol: u32,
    runtime_id: &'static str,
}

async fn health() -> Json<Health> {
    Json(Health {
        status: "ok",
        protocol: 1,
        runtime_id: env!("CARGO_PKG_VERSION"),
    })
}

#[derive(Serialize)]
struct Version {
    version: &'static str,
}

async fn version() -> Json<Version> {
    Json(Version {
        version: env!("CARGO_PKG_VERSION"),
    })
}

#[derive(Serialize)]
struct EmptyList<T> {
    items: Vec<T>,
}

async fn list_projects(
    State(state): State<AppState>,
) -> Result<Json<EmptyList<ProjectContext>>, ApiError> {
    Ok(Json(EmptyList {
        items: state.runtime.list_projects().await?,
    }))
}

async fn register_project(
    State(state): State<AppState>,
    Json(project): Json<ProjectContext>,
) -> Result<(StatusCode, Json<ProjectContext>), ApiError> {
    state.runtime.register_project(project.clone()).await?;
    Ok((StatusCode::CREATED, Json(project)))
}

#[derive(Deserialize, Serialize)]
pub struct CreateSessionRequest {
    pub session_id: Option<SessionId>,
    pub project_id: ProjectId,
    pub contract: SessionContract,
}

#[derive(Serialize)]
pub struct SessionCreated {
    pub session_id: SessionId,
    pub project_id: ProjectId,
}

async fn create_session(
    State(state): State<AppState>,
    Json(request): Json<CreateSessionRequest>,
) -> Result<(StatusCode, Json<SessionCreated>), ApiError> {
    let session_id = request.session_id.unwrap_or_default();
    state
        .runtime
        .create_session(
            session_id.clone(),
            request.project_id.clone(),
            request.contract,
        )
        .await?;
    Ok((
        StatusCode::CREATED,
        Json(SessionCreated {
            session_id,
            project_id: request.project_id,
        }),
    ))
}

async fn list_sessions(
    State(state): State<AppState>,
    Query(query): Query<SessionListQuery>,
) -> Result<Json<EmptyList<SessionSummary>>, ApiError> {
    let project_id = query
        .project_id
        .map(|value| value.parse())
        .transpose()
        .map_err(ApiError::domain)?;
    let items = state
        .runtime
        .list_sessions(project_id)
        .await?
        .into_iter()
        .map(|(id, project_id, created_at, status)| SessionSummary {
            id,
            project_id,
            created_at,
            status,
        })
        .collect();
    Ok(Json(EmptyList { items }))
}

#[derive(Deserialize)]
struct SessionListQuery {
    project_id: Option<String>,
}

#[derive(Serialize)]
struct SessionSummary {
    id: SessionId,
    project_id: ProjectId,
    created_at: String,
    status: String,
}

#[derive(Serialize)]
struct TranscriptResponse {
    messages: Vec<vak_llm::Message>,
}

async fn transcript(
    State(state): State<AppState>,
    Path(session_id): Path<String>,
) -> Result<Json<TranscriptResponse>, ApiError> {
    let session_id = session_id.parse::<SessionId>().map_err(ApiError::domain)?;
    Ok(Json(TranscriptResponse {
        messages: state.runtime.session_messages(&session_id).await?,
    }))
}

#[derive(Deserialize)]
pub struct StartRunRequest {
    pub run_id: Option<RunId>,
    pub session_id: SessionId,
    pub project_id: ProjectId,
    pub input: String,
}

#[derive(Serialize)]
pub struct StartRunResponse {
    pub run: RunSnapshot,
    pub input: String,
}

async fn start_run(
    State(state): State<AppState>,
    Json(request): Json<StartRunRequest>,
) -> Result<(StatusCode, Json<StartRunResponse>), ApiError> {
    let run_id = request.run_id.unwrap_or_default();
    let input = request.input.clone();
    let session_id = request.session_id.clone();
    let handle = state
        .runtime
        .start_run(run_id, request.session_id.clone(), request.project_id)
        .await?;
    let runtime = state.runtime.clone();
    let run_handle = handle.clone_for_task();
    let tasks = state.tasks.clone();
    let task_run_id = handle.run_id().clone();
    let task = tokio::spawn(async move {
        runtime.execute_run(run_handle, session_id, input).await;
        tasks.lock().await.remove(&task_run_id);
    });
    state
        .tasks
        .lock()
        .await
        .insert(handle.run_id().clone(), task);
    Ok((
        StatusCode::ACCEPTED,
        Json(StartRunResponse {
            run: handle.snapshot().clone(),
            input: request.input,
        }),
    ))
}

async fn list_runs(
    State(state): State<AppState>,
    Query(query): Query<RunListQuery>,
) -> Result<Json<EmptyList<RunSnapshot>>, ApiError> {
    let session_id = query
        .session_id
        .map(|value| value.parse())
        .transpose()
        .map_err(ApiError::domain)?;
    Ok(Json(EmptyList {
        items: state.runtime.list_runs(session_id).await?,
    }))
}

#[derive(Deserialize)]
struct RunListQuery {
    session_id: Option<String>,
}

async fn get_run(
    State(state): State<AppState>,
    Path(run_id): Path<String>,
) -> Result<Json<RunSnapshot>, ApiError> {
    let run_id = run_id.parse::<RunId>().map_err(ApiError::domain)?;
    if let Some(snapshot) = state.runtime.runs.snapshot(&run_id).await {
        return Ok(Json(snapshot));
    }
    state
        .runtime
        .list_runs(None)
        .await?
        .into_iter()
        .find(|run| run.run_id == run_id)
        .map(Json)
        .ok_or_else(|| ApiError::not_found("run", &run_id.to_string()))
}

#[derive(Serialize)]
struct CancelResponse {
    cancelled: bool,
}

async fn cancel_run(
    State(state): State<AppState>,
    Path(run_id): Path<String>,
) -> Result<Json<CancelResponse>, ApiError> {
    let run_id = run_id.parse::<RunId>().map_err(ApiError::domain)?;
    let cancelled = state.runtime.cancel_run(&run_id).await?;
    Ok(Json(CancelResponse { cancelled }))
}

async fn events(
    State(state): State<AppState>,
    Query(query): Query<EventQuery>,
) -> Sse<impl Stream<Item = Result<SseEvent, Infallible>>> {
    let ids = state.event_ids.clone();
    let stream = BroadcastStream::new(state.runtime.subscribe()).filter_map(move |event| {
        let run_id = query.run_id.clone();
        let ids = ids.clone();
        async move {
            match event {
                Ok(event)
                    if run_id
                        .as_ref()
                        .is_none_or(|id| event_run_id(&event).as_ref() == Some(id)) =>
                {
                    serde_json::to_string(&event).ok().map(|data| {
                        Ok(SseEvent::default()
                            .id((ids.fetch_add(1, Ordering::Relaxed).saturating_add(1)).to_string())
                            .event(event_name(&event))
                            .data(data))
                    })
                }
                Err(_) => Some(Ok(SseEvent::default().event("resync").data(
                    r#"{"reason":"event stream lagged; refetch authoritative state"}"#,
                ))),
                _ => None,
            }
        }
    });
    Sse::new(stream).keep_alive(KeepAlive::default())
}

#[derive(Deserialize)]
struct EventQuery {
    run_id: Option<RunId>,
}

fn event_run_id(event: &Event) -> Option<RunId> {
    match event {
        Event::RunStatusChanged { run_id, .. }
        | Event::RunOutput { run_id, .. }
        | Event::RunFinished { run_id, .. } => Some(run_id.clone()),
        _ => None,
    }
}

fn event_name(event: &Event) -> &'static str {
    match event {
        Event::ProjectRegistered { .. } => "project.registered",
        Event::SessionCreated { .. } => "session.created",
        Event::RunStatusChanged { .. } => "run.status_changed",
        Event::RunOutput { .. } => "run.output",
        Event::ApprovalRequested { .. } => "approval.requested",
        Event::RunFinished { .. } => "run.finished",
        Event::ConfigChanged { .. } => "config.changed",
        Event::DeliveryUpdated { .. } => "delivery.updated",
    }
}

#[derive(Deserialize)]
pub struct ConfigQuery {
    pub project_id: ProjectId,
}

async fn registered_root(
    runtime: &Runtime,
    project_id: &ProjectId,
) -> Result<std::path::PathBuf, ApiError> {
    runtime
        .list_projects()
        .await?
        .into_iter()
        .find(|project| &project.id == project_id)
        .map(|project| std::path::PathBuf::from(project.root))
        .ok_or_else(|| ApiError::not_found("project", &project_id.to_string()))
}

async fn get_config(
    State(state): State<AppState>,
    Query(query): Query<ConfigQuery>,
) -> Result<Json<ConfigResponse>, ApiError> {
    state
        .runtime
        .config_for(registered_root(&state.runtime, &query.project_id).await?)
        .load()
        .map(|snapshot| Json(ConfigResponse::from(snapshot)))
        .map_err(ApiError::config)
}

#[derive(Deserialize)]
pub struct ConfigPatch {
    pub project_id: ProjectId,
    pub revision: Option<u64>,
    #[serde(alias = "patch")]
    pub config: serde_json::Value,
}

async fn update_config(
    State(state): State<AppState>,
    Json(patch): Json<ConfigPatch>,
) -> Result<Json<ConfigResponse>, ApiError> {
    let config: vak_config::Config = serde_json::from_value(patch.config)
        .map_err(|error| ApiError::message(StatusCode::BAD_REQUEST, error.to_string()))?;
    let root = registered_root(&state.runtime, &patch.project_id).await?;
    let revision = patch
        .revision
        .ok_or_else(|| ApiError::message(StatusCode::BAD_REQUEST, "revision is required"))?;
    let current = state
        .runtime
        .config_for(root.clone())
        .load()
        .map_err(ApiError::config)?;
    if current.revision != revision {
        return Err(ApiError::config(
            vak_config::ConfigError::RevisionConflict {
                expected: revision,
                actual: current.revision,
            },
        ));
    }
    let permission_changed = current.config.permission != config.permission;
    if permission_changed {
        state
            .runtime
            .revoke_capabilities_barrier(std::time::Duration::from_secs(5))
            .await?;
    }
    state
        .runtime
        .config_for(root.clone())
        .update_project(root, revision, |current| *current = config)
        .map(|snapshot| {
            let revision = snapshot.revision;
            state
                .runtime
                .publish_event(vak_domain::Event::ConfigChanged { revision });
            Json(ConfigResponse::from(snapshot))
        })
        .map_err(ApiError::config)
}

async fn update_permission_mode(
    State(state): State<AppState>,
    Json(request): Json<PermissionModeRequest>,
) -> Result<Json<ConfigResponse>, ApiError> {
    let root = registered_root(&state.runtime, &request.project_id).await?;
    let current = state
        .runtime
        .config_for(root.clone())
        .load()
        .map_err(ApiError::config)?;
    if current.revision != request.revision {
        return Err(ApiError::config(
            vak_config::ConfigError::RevisionConflict {
                expected: request.revision,
                actual: current.revision,
            },
        ));
    }
    state
        .runtime
        .revoke_capabilities_barrier(std::time::Duration::from_secs(5))
        .await?;
    state
        .runtime
        .config_for(root.clone())
        .update_project(root, request.revision, |config| {
            config.permission.mode = match request.mode {
                vak_domain::PermissionMode::ReadOnly => vak_config::PermissionMode::ReadOnly,
                vak_domain::PermissionMode::WorkspaceWrite => {
                    vak_config::PermissionMode::WorkspaceWrite
                }
                vak_domain::PermissionMode::FullAccess => vak_config::PermissionMode::FullAccess,
            }
        })
        .map(|snapshot| {
            let revision = snapshot.revision;
            state
                .runtime
                .publish_event(vak_domain::Event::ConfigChanged { revision });
            Json(ConfigResponse::from(snapshot))
        })
        .map_err(ApiError::config)
}

#[derive(Deserialize)]
struct PermissionModeRequest {
    project_id: ProjectId,
    revision: u64,
    mode: vak_domain::PermissionMode,
}

#[derive(Serialize)]
struct ConfigResponse {
    revision: u64,
    config: vak_config::Config,
    warnings: Vec<String>,
}

impl From<vak_config::ConfigSnapshot> for ConfigResponse {
    fn from(snapshot: vak_config::ConfigSnapshot) -> Self {
        Self {
            revision: snapshot.revision,
            config: snapshot.config,
            warnings: snapshot.warnings,
        }
    }
}

#[derive(Serialize)]
struct Diagnostics {
    status: &'static str,
    details: serde_json::Value,
}

#[derive(Serialize)]
struct ProviderInfo {
    name: String,
    env_var: String,
    requires_key: bool,
    configured: bool,
}

async fn list_providers(
    State(state): State<AppState>,
) -> Result<Json<serde_json::Value>, ApiError> {
    let projects = state.runtime.list_projects().await?;
    let config = projects
        .first()
        .map(|p| state.runtime.config_for(&p.root).load())
        .transpose()
        .map_err(ApiError::config)?;
    let current = config
        .as_ref()
        .and_then(|s| s.config.provider.name.clone())
        .unwrap_or_else(|| "anthropic".into());
    let current_model = config
        .as_ref()
        .and_then(|s| s.config.model.name.clone())
        .unwrap_or_default();
    let providers = ["anthropic", "openai", "google", "ollama"]
        .into_iter()
        .map(|name| {
            let env = match name {
                "anthropic" => "ANTHROPIC_API_KEY",
                "google" => "GOOGLE_API_KEY",
                "ollama" => "OLLAMA_API_KEY",
                _ => "OPENAI_API_KEY",
            };
            let configured = state.runtime.secrets().get(env).ok().flatten().is_some()
                || std::env::var(env).is_ok();
            ProviderInfo {
                name: name.into(),
                env_var: env.into(),
                requires_key: name != "ollama",
                configured,
            }
        })
        .collect::<Vec<_>>();
    Ok(Json(
        serde_json::json!({"current": current, "current_model": current_model, "current_configured": providers.iter().any(|p| p.name == current && (p.configured || !p.requires_key)), "providers": providers}),
    ))
}

async fn discover_models(
    State(state): State<AppState>,
    Path(provider): Path<String>,
) -> Result<Json<serde_json::Value>, ApiError> {
    Ok(Json(
        serde_json::json!({"provider": provider, "models": state.runtime.discover_models(&provider).await?}),
    ))
}

#[derive(Deserialize)]
struct ProviderKeyRequest {
    key: String,
}

async fn set_provider_key(
    State(state): State<AppState>,
    Path(provider): Path<String>,
    Json(request): Json<ProviderKeyRequest>,
) -> Result<Json<serde_json::Value>, ApiError> {
    if request.key.trim().is_empty() {
        return Err(ApiError::message(
            StatusCode::BAD_REQUEST,
            "provider key must not be empty",
        ));
    }
    let env_var = state.runtime.set_provider_key(&provider, &request.key)?;
    Ok(Json(
        serde_json::json!({"provider": provider, "env_var": env_var, "configured": true}),
    ))
}

async fn remove_provider_key(
    State(state): State<AppState>,
    Path(provider): Path<String>,
) -> Result<Json<serde_json::Value>, ApiError> {
    let (env_var, removed) = state.runtime.remove_provider_key(&provider)?;
    Ok(Json(
        serde_json::json!({"provider": provider, "env_var": env_var, "configured": false, "shadowed_by_env": !removed && std::env::var(&env_var).is_ok()}),
    ))
}

async fn diagnostics(State(state): State<AppState>) -> Json<Diagnostics> {
    Json(Diagnostics {
        status: "ok",
        details: serde_json::json!({"data_home": state.runtime.data_home(), "capability_epoch": state.runtime.runs.current_epoch().0}),
    })
}

#[derive(Deserialize)]
struct FsQuery {
    project_id: ProjectId,
    path: String,
    limit: Option<usize>,
}

fn safe_project_path(
    root: &std::path::Path,
    relative: &str,
) -> Result<std::path::PathBuf, ApiError> {
    let candidate = root.join(relative);
    let canonical_root = std::fs::canonicalize(root)
        .map_err(|e| ApiError::message(StatusCode::BAD_REQUEST, e.to_string()))?;
    let canonical = if candidate.exists() {
        std::fs::canonicalize(&candidate)
            .map_err(|e| ApiError::message(StatusCode::BAD_REQUEST, e.to_string()))?
    } else {
        let parent = candidate
            .parent()
            .ok_or_else(|| ApiError::message(StatusCode::BAD_REQUEST, "invalid path"))?;
        let parent = std::fs::canonicalize(parent)
            .map_err(|e| ApiError::message(StatusCode::BAD_REQUEST, e.to_string()))?;
        parent.join(
            candidate
                .file_name()
                .ok_or_else(|| ApiError::message(StatusCode::BAD_REQUEST, "invalid path"))?,
        )
    };
    if !canonical.starts_with(&canonical_root) {
        return Err(ApiError::message(
            StatusCode::BAD_REQUEST,
            "path escapes project root",
        ));
    }
    Ok(canonical)
}

async fn fs_tree(
    State(state): State<AppState>,
    Query(query): Query<FsQuery>,
) -> Result<Json<serde_json::Value>, ApiError> {
    let root = registered_root(&state.runtime, &query.project_id).await?;
    let limit = query.limit.unwrap_or(400).min(10_000);
    let mut files = Vec::new();
    let mut stack = vec![root.clone()];
    while let Some(dir) = stack.pop() {
        for entry in std::fs::read_dir(&dir)
            .map_err(|e| ApiError::message(StatusCode::BAD_REQUEST, e.to_string()))?
        {
            let entry =
                entry.map_err(|e| ApiError::message(StatusCode::BAD_REQUEST, e.to_string()))?;
            let path = entry.path();
            if path
                .file_name()
                .and_then(|n| n.to_str())
                .is_some_and(|n| n == ".git" || n == "target")
            {
                continue;
            }
            if path.is_dir() {
                stack.push(path);
            } else if let Ok(rel) = path.strip_prefix(&root) {
                files.push(rel.to_string_lossy().into_owned());
                if files.len() >= limit {
                    return Ok(Json(serde_json::json!({"files": files, "truncated": true})));
                }
            }
        }
    }
    files.sort();
    Ok(Json(
        serde_json::json!({"files": files, "truncated": false}),
    ))
}

async fn read_file(
    State(state): State<AppState>,
    Query(query): Query<FsQuery>,
) -> Result<Json<serde_json::Value>, ApiError> {
    let root = registered_root(&state.runtime, &query.project_id).await?;
    let path = safe_project_path(&root, &query.path)?;
    let bytes = std::fs::read(&path)
        .map_err(|e| ApiError::message(StatusCode::NOT_FOUND, e.to_string()))?;
    let text = std::str::from_utf8(&bytes).ok().map(str::to_owned);
    let kind = if text.is_some() { "text" } else { "binary" };
    Ok(Json(
        serde_json::json!({"path": query.path, "kind": kind, "bytes": bytes.len(), "editable": kind == "text", "content": text}),
    ))
}

#[derive(Deserialize)]
struct FsWrite {
    project_id: ProjectId,
    path: String,
    content: String,
}

async fn write_file(
    State(state): State<AppState>,
    Json(request): Json<FsWrite>,
) -> Result<Json<serde_json::Value>, ApiError> {
    let root = registered_root(&state.runtime, &request.project_id).await?;
    let path = safe_project_path(&root, &request.path)?;
    let config = state
        .runtime
        .config_for(&root)
        .load()
        .map_err(ApiError::config)?;
    if config.config.permission.mode == vak_config::PermissionMode::ReadOnly {
        return Err(ApiError::message(
            StatusCode::FORBIDDEN,
            "workspace is read-only",
        ));
    }
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)
            .map_err(|e| ApiError::message(StatusCode::BAD_REQUEST, e.to_string()))?;
    }
    std::fs::write(&path, request.content.as_bytes())
        .map_err(|e| ApiError::message(StatusCode::BAD_REQUEST, e.to_string()))?;
    Ok(Json(
        serde_json::json!({"path": request.path, "bytes": request.content.len()}),
    ))
}

#[derive(Deserialize)]
struct TaskQuery {
    project_id: Option<ProjectId>,
}

async fn list_tasks(
    State(state): State<AppState>,
    Query(query): Query<TaskQuery>,
) -> Result<Json<EmptyList<vak_runtime::TaskRecord>>, ApiError> {
    Ok(Json(EmptyList {
        items: state.runtime.tasks(query.project_id.as_ref()).await?,
    }))
}

#[derive(Deserialize)]
struct CreateTaskRequest {
    id: Option<TaskId>,
    project_id: Option<ProjectId>,
    #[serde(default)]
    spec: serde_json::Value,
}

async fn create_task(
    State(state): State<AppState>,
    Json(request): Json<CreateTaskRequest>,
) -> Result<(StatusCode, Json<vak_runtime::TaskRecord>), ApiError> {
    let task = state
        .runtime
        .create_task(
            request.id.unwrap_or_default().to_string(),
            request.project_id,
            request.spec,
        )
        .await?;
    Ok((StatusCode::CREATED, Json(task)))
}

#[derive(Deserialize)]
struct TaskPatch {
    spec: Option<serde_json::Value>,
    status: Option<String>,
}

async fn update_task(
    State(state): State<AppState>,
    Path(task_id): Path<String>,
    Json(patch): Json<TaskPatch>,
) -> Result<Json<vak_runtime::TaskRecord>, ApiError> {
    let task_id = task_id.parse::<TaskId>().map_err(ApiError::domain)?;
    Ok(Json(
        state
            .runtime
            .update_task(&task_id, patch.spec, patch.status)
            .await?,
    ))
}

async fn delete_task(
    State(state): State<AppState>,
    Path(task_id): Path<String>,
) -> Result<Json<serde_json::Value>, ApiError> {
    let task_id = task_id.parse::<TaskId>().map_err(ApiError::domain)?;
    Ok(Json(
        serde_json::json!({"deleted": state.runtime.delete_task(&task_id).await?}),
    ))
}

#[derive(Deserialize)]
struct InboxQuery {
    unread: Option<bool>,
}

async fn list_inbox(
    State(state): State<AppState>,
    Query(query): Query<InboxQuery>,
) -> Result<Json<EmptyList<vak_runtime::InboxRecord>>, ApiError> {
    Ok(Json(EmptyList {
        items: state.runtime.inbox(query.unread.unwrap_or(false)).await?,
    }))
}

async fn ack_inbox(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> Result<Json<serde_json::Value>, ApiError> {
    Ok(Json(
        serde_json::json!({"acknowledged": state.runtime.acknowledge_inbox(&id).await?}),
    ))
}

async fn list_approvals(
    State(state): State<AppState>,
) -> Result<Json<EmptyList<vak_runtime::ApprovalRecord>>, ApiError> {
    Ok(Json(EmptyList {
        items: state.runtime.approvals().await?,
    }))
}

#[derive(Deserialize)]
struct ApprovalResolution {
    allow: bool,
    #[serde(default)]
    response: serde_json::Value,
}

async fn resolve_approval(
    State(state): State<AppState>,
    Path(id): Path<String>,
    Json(request): Json<ApprovalResolution>,
) -> Result<Json<vak_runtime::ApprovalRecord>, ApiError> {
    Ok(Json(
        state
            .runtime
            .resolve_approval(&id, request.allow, request.response)
            .await?,
    ))
}

#[derive(Deserialize)]
struct MemoryQuery {
    project_id: Option<ProjectId>,
    scope: Option<String>,
}

async fn list_memory(
    State(state): State<AppState>,
    Query(query): Query<MemoryQuery>,
) -> Result<Json<EmptyList<vak_services::MemoryRecord>>, ApiError> {
    let scope = query.scope.as_deref().unwrap_or("workspace");
    Ok(Json(EmptyList {
        items: state
            .runtime
            .memory(query.project_id.as_ref(), scope)
            .await?,
    }))
}

#[derive(Deserialize)]
struct CreateMemoryRequest {
    id: Option<String>,
    project_id: Option<ProjectId>,
    #[serde(default = "default_memory_scope")]
    scope: String,
    #[serde(default = "default_memory_kind")]
    kind: String,
    #[serde(default)]
    tag: String,
    text: String,
}

fn default_memory_scope() -> String {
    "workspace".to_owned()
}
fn default_memory_kind() -> String {
    "note".to_owned()
}

async fn create_memory(
    State(state): State<AppState>,
    Json(request): Json<CreateMemoryRequest>,
) -> Result<(StatusCode, Json<vak_services::MemoryRecord>), ApiError> {
    let id = request
        .id
        .unwrap_or_else(|| vak_domain::TaskId::new().to_string())
        .to_string();
    let memory = state
        .runtime
        .create_memory(
            id,
            request.project_id,
            request.scope,
            request.kind,
            request.tag,
            request.text,
        )
        .await?;
    Ok((StatusCode::CREATED, Json(memory)))
}

#[derive(Deserialize)]
struct AmendMemoryRequest {
    text: String,
}

async fn amend_memory(
    State(state): State<AppState>,
    Path(id): Path<String>,
    Json(request): Json<AmendMemoryRequest>,
) -> Result<Json<vak_services::MemoryRecord>, ApiError> {
    Ok(Json(state.runtime.amend_memory(&id, &request.text).await?))
}

async fn forget_memory(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> Result<Json<serde_json::Value>, ApiError> {
    Ok(Json(
        serde_json::json!({"forgotten": state.runtime.forget_memory(&id).await?}),
    ))
}

#[derive(Deserialize)]
struct SkillQuery {
    project_id: Option<ProjectId>,
    status: Option<String>,
}

async fn list_skills(
    State(state): State<AppState>,
    Query(query): Query<SkillQuery>,
) -> Result<Json<EmptyList<vak_runtime::SkillRecord>>, ApiError> {
    Ok(Json(EmptyList {
        items: state
            .runtime
            .skills(query.project_id.as_ref(), query.status.as_deref())
            .await?,
    }))
}

async fn promote_skill(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> Result<Json<vak_runtime::SkillRecord>, ApiError> {
    Ok(Json(state.runtime.promote_skill(&id).await?))
}

async fn reject_skill(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> Result<Json<vak_runtime::SkillRecord>, ApiError> {
    Ok(Json(state.runtime.reject_skill(&id).await?))
}

#[derive(Deserialize)]
struct CheckpointQuery {
    #[serde(default)]
    include_manifest: bool,
}

fn checkpoint_json(
    record: vak_runtime::CheckpointRecord,
    session_id: SessionId,
    manifest: Option<serde_json::Value>,
) -> vak_client::Checkpoint {
    vak_client::Checkpoint {
        id: record.id,
        session_id,
        manifest_digest: record.manifest_digest,
        created_at: record.created_at,
        label: record.label,
        manifest,
    }
}

async fn list_checkpoints(
    State(state): State<AppState>,
    Path(session_id): Path<String>,
    Query(query): Query<CheckpointQuery>,
) -> Result<Json<EmptyList<vak_client::Checkpoint>>, ApiError> {
    let session_id = session_id.parse::<SessionId>().map_err(ApiError::domain)?;
    let mut items = Vec::new();
    for record in state.runtime.checkpoints(&session_id).await? {
        let manifest = if query.include_manifest {
            state.runtime.checkpoint_manifest(&record.id).await?
        } else {
            None
        };
        items.push(checkpoint_json(record, session_id.clone(), manifest));
    }
    Ok(Json(EmptyList { items }))
}

async fn create_checkpoint(
    State(state): State<AppState>,
    Path(session_id): Path<String>,
    Json(request): Json<vak_client::CheckpointCreate>,
) -> Result<(StatusCode, Json<vak_client::Checkpoint>), ApiError> {
    let session_id = session_id.parse::<SessionId>().map_err(ApiError::domain)?;
    let record = state
        .runtime
        .create_checkpoint(&session_id, &request.label, &request.manifest)
        .await?;
    Ok((
        StatusCode::CREATED,
        Json(checkpoint_json(record, session_id, Some(request.manifest))),
    ))
}

async fn get_checkpoint(
    State(state): State<AppState>,
    Path((session_id, checkpoint_id)): Path<(String, String)>,
) -> Result<Json<vak_client::Checkpoint>, ApiError> {
    let session_id = session_id.parse::<SessionId>().map_err(ApiError::domain)?;
    let record = state
        .runtime
        .checkpoints(&session_id)
        .await?
        .into_iter()
        .find(|x| x.id == checkpoint_id)
        .ok_or_else(|| ApiError::not_found("checkpoint", &checkpoint_id))?;
    let manifest = state.runtime.checkpoint_manifest(&record.id).await?;
    Ok(Json(checkpoint_json(record, session_id, manifest)))
}

async fn restore_checkpoint(
    State(state): State<AppState>,
    Path((session_id, checkpoint_id)): Path<(String, String)>,
) -> Result<Json<vak_client::RestoreResponse>, ApiError> {
    let session_id = session_id.parse::<SessionId>().map_err(ApiError::domain)?;
    state
        .runtime
        .restore_checkpoint(&session_id, &checkpoint_id)
        .await
        .map_err(|error| {
            if error.to_string().contains("not found") {
                ApiError::not_found("checkpoint", &checkpoint_id)
            } else {
                ApiError::from(error)
            }
        })?;
    Ok(Json(vak_client::RestoreResponse {
        restored: true,
        checkpoint_id,
    }))
}

#[derive(Deserialize)]
struct BackupRequest {
    directory: String,
    #[serde(default)]
    include_secrets: bool,
    #[serde(default = "default_conflict")]
    conflict: String,
}
fn default_conflict() -> String {
    "skip".to_owned()
}

async fn backup_export(
    State(state): State<AppState>,
    Json(request): Json<BackupRequest>,
) -> Result<Json<vak_runtime::BackupReport>, ApiError> {
    Ok(Json(
        state
            .runtime
            .backup_export(request.directory, request.include_secrets)
            .await?,
    ))
}
async fn backup_import(
    State(state): State<AppState>,
    Json(request): Json<BackupRequest>,
) -> Result<Json<vak_runtime::BackupReport>, ApiError> {
    Ok(Json(
        state
            .runtime
            .backup_import(request.directory, &request.conflict)
            .await?,
    ))
}

#[derive(Deserialize)]
struct FlowQuery {
    project_id: ProjectId,
}
async fn list_flows(
    State(state): State<AppState>,
    Query(query): Query<FlowQuery>,
) -> Result<Json<EmptyList<vak_client::FlowDefinition>>, ApiError> {
    let items = state
        .runtime
        .flows(&query.project_id)
        .await?
        .into_iter()
        .map(|flow| vak_client::FlowDefinition {
            name: flow.name,
            path: flow.path.display().to_string(),
            valid: flow.valid,
            nodes: flow.nodes,
        })
        .collect();
    Ok(Json(EmptyList { items }))
}
async fn check_flow(
    State(state): State<AppState>,
    Path(name): Path<String>,
    Query(query): Query<FlowQuery>,
) -> Result<Json<vak_client::FlowDefinition>, ApiError> {
    let flow = state.runtime.check_flow(&query.project_id, &name).await?;
    Ok(Json(vak_client::FlowDefinition {
        name: flow.name,
        path: flow.path.display().to_string(),
        valid: flow.valid,
        nodes: flow.nodes,
    }))
}
async fn run_flow(
    State(state): State<AppState>,
    Path(name): Path<String>,
    Json(request): Json<vak_client::FlowRunRequest>,
) -> Result<(StatusCode, Json<StartRunResponse>), ApiError> {
    let flow = state.runtime.check_flow(&request.project_id, &name).await?;
    if !flow.valid {
        return Err(ApiError::message(
            StatusCode::BAD_REQUEST,
            "flow definition is invalid",
        ));
    }
    start_run(
        State(state),
        Json(StartRunRequest {
            run_id: None,
            session_id: request.session_id,
            project_id: request.project_id,
            input: request.input,
        }),
    )
    .await
}
async fn exec_run(
    State(state): State<AppState>,
    Json(request): Json<StartRunRequest>,
) -> Result<(StatusCode, Json<StartRunResponse>), ApiError> {
    start_run(State(state), Json(request)).await
}

async fn run_eval(
    State(state): State<AppState>,
    Json(request): Json<vak_client::EvalRequest>,
) -> Result<Json<vak_client::EvalReport>, ApiError> {
    let total = request.cases.len();
    let (passed, failed) = state.runtime.eval(&request.cases).await?;
    Ok(Json(vak_client::EvalReport {
        total,
        passed,
        failed,
    }))
}

pub struct ApiError {
    status: StatusCode,
    message: String,
}

impl ApiError {
    fn domain(error: vak_domain::DomainError) -> Self {
        Self {
            status: StatusCode::BAD_REQUEST,
            message: error.to_string(),
        }
    }
    fn not_found(resource: &str, id: &str) -> Self {
        Self {
            status: StatusCode::NOT_FOUND,
            message: format!("{resource} not found: {id}"),
        }
    }
    fn config(error: vak_config::ConfigError) -> Self {
        Self {
            status: StatusCode::BAD_REQUEST,
            message: error.to_string(),
        }
    }
    fn message(status: StatusCode, message: impl Into<String>) -> Self {
        Self {
            status,
            message: message.into(),
        }
    }
}

impl From<RuntimeError> for ApiError {
    fn from(error: RuntimeError) -> Self {
        let status = match &error {
            RuntimeError::Run(vak_runtime::RunError::NotFound(_)) => StatusCode::NOT_FOUND,
            RuntimeError::Run(vak_runtime::RunError::AlreadyExists(_))
            | RuntimeError::Run(vak_runtime::RunError::Terminal(_)) => StatusCode::CONFLICT,
            _ => StatusCode::BAD_REQUEST,
        };
        Self {
            status,
            message: error.to_string(),
        }
    }
}

impl axum::response::IntoResponse for ApiError {
    fn into_response(self) -> axum::response::Response {
        (
            self.status,
            Json(serde_json::json!({"error": self.message})),
        )
            .into_response()
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::*;
    use axum::{body::Body, http::Request};
    use http_body_util::BodyExt;
    use tempfile::tempdir;
    use tower::ServiceExt;

    #[tokio::test]
    async fn health_and_version_are_transport_only() {
        let home = tempdir().expect("tempdir");
        let app = router_with_token(
            Runtime::open(home.path()).expect("runtime"),
            "local-development",
        )
        .expect("router");
        let response = app
            .oneshot(
                Request::builder()
                    .uri("/health")
                    .header("authorization", "Bearer local-development")
                    .body(Body::empty())
                    .expect("request"),
            )
            .await
            .expect("response");
        assert_eq!(response.status(), StatusCode::OK);
        let body = response
            .into_body()
            .collect()
            .await
            .expect("body")
            .to_bytes();
        assert_eq!(
            &body[..],
            br#"{"status":"ok","protocol":1,"runtime_id":"0.9.2"}"#
        );
    }

    #[tokio::test]
    async fn register_project_and_create_session_delegate_to_runtime() {
        let home = tempdir().expect("tempdir");
        let project = tempdir().expect("project");
        let runtime = Runtime::open(home.path()).expect("runtime");
        let app = router_with_token(runtime.clone(), "local-development").expect("router");
        let project_id = ProjectId::new();
        let project_body = serde_json::to_vec(&ProjectContext {
            id: project_id.clone(),
            root: project.path().display().to_string(),
            display_name: None,
        })
        .expect("json");
        let response = app
            .clone()
            .oneshot(
                Request::post("/projects")
                    .header("content-type", "application/json")
                    .header("authorization", "Bearer local-development")
                    .body(Body::from(project_body))
                    .expect("request"),
            )
            .await
            .expect("response");
        assert_eq!(response.status(), StatusCode::CREATED);
        let contract = SessionContract {
            provider: "test".into(),
            model: "test".into(),
            route_ladder: vec![],
            system_prompt: "system".into(),
            permission_mode: vak_domain::PermissionMode::ReadOnly,
            sandbox: vak_domain::SandboxMode::None,
            tool_catalogue_revision: "test".into(),
            context_limit: 100,
            budget_ceiling: None,
        };
        let body = serde_json::to_vec(&CreateSessionRequest {
            session_id: None,
            project_id,
            contract,
        })
        .expect("json");
        let response = app
            .oneshot(
                Request::post("/sessions")
                    .header("content-type", "application/json")
                    .header("authorization", "Bearer local-development")
                    .body(Body::from(body))
                    .expect("request"),
            )
            .await
            .expect("response");
        assert_eq!(response.status(), StatusCode::CREATED);
    }

    #[tokio::test]
    async fn every_endpoint_requires_bearer_authentication() {
        let home = tempdir().expect("tempdir");
        let app = router_with_token(Runtime::open(home.path()).expect("runtime"), "secret")
            .expect("router");
        let response = app
            .oneshot(
                Request::get("/health")
                    .body(Body::empty())
                    .expect("request"),
            )
            .await
            .expect("response");
        assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
    }

    #[tokio::test]
    async fn admin_bootstrap_uses_http_only_cookie_without_exposing_data_routes() {
        let home = tempdir().expect("tempdir");
        let app = router_with_token(Runtime::open(home.path()).expect("runtime"), "secret")
            .expect("router");
        let response = app
            .clone()
            .oneshot(
                Request::post("/auth/login")
                    .header("content-type", "application/json")
                    .body(Body::from(r#"{"token":"secret"}"#))
                    .expect("request"),
            )
            .await
            .expect("response");
        assert_eq!(response.status(), StatusCode::OK);
        let cookie = response
            .headers()
            .get(axum::http::header::SET_COOKIE)
            .expect("cookie")
            .to_str()
            .expect("cookie text")
            .to_owned();
        assert!(cookie.contains("HttpOnly"));
        let response = app
            .oneshot(
                Request::get("/projects")
                    .header(axum::http::header::COOKIE, cookie)
                    .body(Body::empty())
                    .expect("request"),
            )
            .await
            .expect("response");
        assert_eq!(response.status(), StatusCode::OK);
    }

    #[tokio::test]
    async fn config_requires_a_registered_project_id() {
        let home = tempdir().expect("tempdir");
        let project = ProjectId::new();
        let app = router_with_token(Runtime::open(home.path()).expect("runtime"), "secret")
            .expect("router");
        let response = app
            .oneshot(
                Request::get(format!("/config?project_id={project}"))
                    .header("authorization", "Bearer secret")
                    .body(Body::empty())
                    .expect("request"),
            )
            .await
            .expect("response");
        assert_eq!(response.status(), StatusCode::NOT_FOUND);
    }

    #[tokio::test]
    async fn permission_change_revokes_the_current_capability_epoch() {
        let home = tempdir().expect("tempdir");
        let project_dir = tempdir().expect("project");
        let runtime = Runtime::open(home.path()).expect("runtime");
        let project_id = ProjectId::new();
        runtime
            .register_project(ProjectContext {
                id: project_id.clone(),
                root: project_dir.path().display().to_string(),
                display_name: None,
            })
            .await
            .expect("project");
        let before = runtime.runs.current_epoch();
        let app = router_with_token(runtime.clone(), "secret").expect("router");
        let body = serde_json::json!({
            "project_id": project_id,
            "revision": 0,
            "mode": "WorkspaceWrite"
        });
        let response = app
            .oneshot(
                Request::post("/config/permission-mode")
                    .header("authorization", "Bearer secret")
                    .header("content-type", "application/json")
                    .body(Body::from(body.to_string()))
                    .expect("request"),
            )
            .await
            .expect("response");
        assert_eq!(response.status(), StatusCode::OK);
        assert!(runtime.runs.current_epoch().0 > before.0);
    }
}
