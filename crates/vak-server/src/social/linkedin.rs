//! LinkedIn's owner-only native PKCE/OIDC account link.
//!
//! This provides connection identity for Settings only. It does not create a
//! model tool or grant access to posts, member profiles, or organization data.

use crate::{AppState, AuthenticatedPrincipal};
use axum::{
    Json,
    extract::{ConnectInfo, Query, State},
    http::{HeaderMap, StatusCode, header},
    response::{Html, IntoResponse, Response},
};
use chrono::{DateTime, Duration as ChronoDuration, Utc};
use serde::{Deserialize, Serialize};
use sha2::Digest;
use std::{
    collections::HashMap,
    net::SocketAddr,
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};
use tokio::net::TcpListener;
use tokio_util::sync::CancellationToken;
use url::Url;
use zeroize::{Zeroize, Zeroizing};

const CLIENT_ID_KEY: &str = "VAK_SOCIAL_LINKEDIN_CLIENT_ID";
const CONNECTION_KEY: &str = "VAK_SOCIAL_LINKEDIN_CONNECTION";
const CALLBACK_PATH: &str = "/social/linkedin/oauth/callback";
const AUTHORIZATION_ENDPOINT: &str = "https://www.linkedin.com/oauth/native-pkce/authorization";
const TOKEN_ENDPOINT: &str = "https://www.linkedin.com/oauth/v2/accessToken";
const USERINFO_ENDPOINT: &str = "https://api.linkedin.com/v2/userinfo";
const AUTH_TTL: Duration = Duration::from_secs(10 * 60);
const MAX_PENDING: usize = 8;
const MAX_TOKEN_RESPONSE: usize = 64 * 1024;
const MAX_USERINFO_RESPONSE: usize = 64 * 1024;
const SCOPES: &str = "openid profile";

#[derive(Default)]
struct PendingState {
    entries: HashMap<String, Arc<PendingAuthorization>>,
    agent_generations: HashMap<String, u64>,
}

#[derive(Clone, Default)]
pub(crate) struct PendingAuthorizations(Arc<Mutex<PendingState>>);

struct PendingAuthorization {
    verifier: Zeroizing<String>,
    client_id: String,
    agent_id: String,
    callback_uri: String,
    session_id: Option<String>,
    generation: u64,
    expires_at: Instant,
    shutdown: CancellationToken,
}

impl PendingAuthorizations {
    fn insert(&self, state: String, authorization: PendingAuthorization) -> Result<(), ()> {
        let mut pending = self
            .0
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        pending
            .entries
            .retain(|_, flow| flow.expires_at > Instant::now());
        if pending.entries.len() >= MAX_PENDING {
            return Err(());
        }
        let mut authorization = authorization;
        authorization.generation = *pending
            .agent_generations
            .entry(authorization.agent_id.clone())
            .or_default();
        pending.entries.insert(state, Arc::new(authorization));
        Ok(())
    }

    fn consume(&self, state: &str) -> Option<Arc<PendingAuthorization>> {
        let authorization = self
            .0
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .entries
            .remove(state)?;
        if authorization.expires_at <= Instant::now() {
            return None;
        }
        Some(authorization)
    }

    fn cancel_agent(&self, agent_id: &str) {
        let mut pending = self
            .0
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let generation = pending
            .agent_generations
            .entry(agent_id.to_owned())
            .or_default();
        *generation = generation.wrapping_add(1);
        pending.entries.retain(|_, flow| {
            if flow.agent_id == agent_id {
                flow.shutdown.cancel();
                false
            } else {
                true
            }
        });
    }

    fn is_current(&self, agent_id: &str, generation: u64) -> bool {
        let pending = self
            .0
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        pending
            .agent_generations
            .get(agent_id)
            .copied()
            .unwrap_or(0)
            == generation
    }

    fn with_current<T>(
        &self,
        agent_id: &str,
        generation: u64,
        action: impl FnOnce() -> T,
    ) -> Option<T> {
        let pending = self
            .0
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if pending
            .agent_generations
            .get(agent_id)
            .copied()
            .unwrap_or(0)
            != generation
        {
            return None;
        }
        Some(action())
    }
}

#[derive(Deserialize)]
pub(crate) struct AgentQuery {
    agent: Option<String>,
}

#[derive(Deserialize)]
pub(crate) struct ClientIdBody {
    client_id: String,
}

pub(crate) async fn client_id_status(
    State(state): State<AppState>,
    axum::Extension(principal): axum::Extension<AuthenticatedPrincipal>,
    Query(query): Query<AgentQuery>,
) -> Response {
    if let Err(status) = crate::operator_only(&principal) {
        return status.into_response();
    }
    let core = match crate::resolve_scoped_core(&state, None, query.agent.as_deref()) {
        Ok(core) => core,
        Err(response) => return response,
    };
    Json(serde_json::json!({
        "configured": vak_config::read_env_file_var(&core.scope().env_file(), CLIENT_ID_KEY).is_some()
    }))
    .into_response()
}

pub(crate) async fn save_client_id(
    State(state): State<AppState>,
    axum::Extension(principal): axum::Extension<AuthenticatedPrincipal>,
    Query(query): Query<AgentQuery>,
    Json(body): Json<ClientIdBody>,
) -> Response {
    if let Err(status) = crate::operator_only(&principal) {
        return status.into_response();
    }
    let core = match crate::resolve_scoped_core(&state, None, query.agent.as_deref()) {
        Ok(core) => core,
        Err(response) => return response,
    };
    let client_id = body.client_id.trim();
    if client_id.is_empty()
        || client_id.len() > 256
        || !client_id.bytes().all(|byte| byte.is_ascii_graphic())
    {
        return (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({"error":"Enter the Client ID from your LinkedIn app."})),
        )
            .into_response();
    }
    match vak_config::upsert_env_file(&core.scope().env_file(), CLIENT_ID_KEY, client_id) {
        Ok(()) => Json(serde_json::json!({ "configured": true })).into_response(),
        Err(_) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({"error":"Could not store the LinkedIn app configuration securely."})),
        )
            .into_response(),
    }
}

pub(crate) async fn remove_client_id(
    State(state): State<AppState>,
    axum::Extension(principal): axum::Extension<AuthenticatedPrincipal>,
    Query(query): Query<AgentQuery>,
) -> Response {
    if let Err(status) = crate::operator_only(&principal) {
        return status.into_response();
    }
    let core = match crate::resolve_scoped_core(&state, None, query.agent.as_deref()) {
        Ok(core) => core,
        Err(response) => return response,
    };
    state
        .linkedin_oauth
        .cancel_agent(query.agent.as_deref().unwrap_or("vak"));
    match vak_config::remove_env_file_key(&core.scope().env_file(), CLIENT_ID_KEY) {
        Ok(()) => Json(serde_json::json!({ "configured": false })).into_response(),
        Err(_) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({"error":"Could not remove the LinkedIn app configuration."})),
        )
            .into_response(),
    }
}

#[derive(Serialize, Deserialize)]
struct StoredConnection {
    access_token: String,
    display_name: String,
    scopes_requested: Vec<String>,
    scopes_returned: Option<Vec<String>>,
    expires_at: DateTime<Utc>,
}

impl Drop for StoredConnection {
    fn drop(&mut self) {
        self.access_token.zeroize();
    }
}

pub(crate) async fn account_status(
    State(state): State<AppState>,
    axum::Extension(principal): axum::Extension<AuthenticatedPrincipal>,
    Query(query): Query<AgentQuery>,
) -> Response {
    if let Err(status) = crate::operator_only(&principal) {
        return status.into_response();
    }
    let core = match crate::resolve_scoped_core(&state, None, query.agent.as_deref()) {
        Ok(core) => core,
        Err(response) => return response,
    };
    let connection = load_connection(&core);
    match connection {
        Ok(Some(connection)) => Json(serde_json::json!({
            "connected": true,
            "display_name": &connection.display_name,
            "expires_at": connection.expires_at,
            "scopes_requested": &connection.scopes_requested,
            "scopes_returned": &connection.scopes_returned,
            "expired": connection.expires_at <= Utc::now(),
        }))
        .into_response(),
        Ok(None) => Json(serde_json::json!({ "connected": false })).into_response(),
        Err(_) => (
            StatusCode::SERVICE_UNAVAILABLE,
            Json(serde_json::json!({"error":"LinkedIn connection storage is unavailable."})),
        )
            .into_response(),
    }
}

pub(crate) async fn disconnect(
    State(state): State<AppState>,
    axum::Extension(principal): axum::Extension<AuthenticatedPrincipal>,
    Query(query): Query<AgentQuery>,
) -> Response {
    if let Err(status) = crate::operator_only(&principal) {
        return status.into_response();
    }
    let core = match crate::resolve_scoped_core(&state, None, query.agent.as_deref()) {
        Ok(core) => core,
        Err(response) => return response,
    };
    state
        .linkedin_oauth
        .cancel_agent(query.agent.as_deref().unwrap_or("vak"));
    match vak_config::remove_env_file_key(&core.scope().env_file(), CONNECTION_KEY) {
        Ok(()) => Json(serde_json::json!({ "connected": false })).into_response(),
        Err(_) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({"error":"Could not disconnect the LinkedIn account."})),
        )
            .into_response(),
    }
}

pub(crate) async fn begin(
    State(state): State<AppState>,
    axum::Extension(principal): axum::Extension<AuthenticatedPrincipal>,
    ConnectInfo(peer): ConnectInfo<SocketAddr>,
    headers: HeaderMap,
    Query(query): Query<AgentQuery>,
) -> Response {
    if let Err(status) = crate::operator_only(&principal) {
        return status.into_response();
    }
    if state.core.config().server.public_url.is_some()
        || !crate::mail_calendar::is_loopback_request(&headers, peer)
    {
        return (
            StatusCode::FORBIDDEN,
            Json(serde_json::json!({"error":"LinkedIn account linking requires a direct local Vakyartha connection."})),
        )
            .into_response();
    }
    let core = match crate::resolve_scoped_core(&state, None, query.agent.as_deref()) {
        Ok(core) => core,
        Err(response) => return response,
    };
    if !crate::social_plugin_enabled(&core, "social-linkedin") {
        return (
            StatusCode::FORBIDDEN,
            Json(serde_json::json!({"error":"Install and enable the LinkedIn add-on before connecting an account."})),
        )
            .into_response();
    }
    let Some(client_id) = vak_config::read_env_file_var(&core.scope().env_file(), CLIENT_ID_KEY)
        .filter(|client_id| !client_id.trim().is_empty())
    else {
        return (
            StatusCode::PRECONDITION_FAILED,
            Json(serde_json::json!({"error":"Save your LinkedIn app Client ID first."})),
        )
            .into_response();
    };
    let listener = match TcpListener::bind(("127.0.0.1", 0)).await {
        Ok(listener) => listener,
        Err(_) => {
            return (
                StatusCode::SERVICE_UNAVAILABLE,
                Json(serde_json::json!({"error":"Could not open a temporary local LinkedIn callback."})),
            )
                .into_response();
        }
    };
    let Ok(local_addr) = listener.local_addr() else {
        return StatusCode::SERVICE_UNAVAILABLE.into_response();
    };
    let callback_uri = format!("http://127.0.0.1:{}{CALLBACK_PATH}", local_addr.port());
    let (state_value, verifier, challenge) = match pkce_material() {
        Ok(material) => material,
        Err(_) => return StatusCode::SERVICE_UNAVAILABLE.into_response(),
    };
    let mut authorization = match Url::parse(AUTHORIZATION_ENDPOINT) {
        Ok(url) => url,
        Err(_) => return StatusCode::INTERNAL_SERVER_ERROR.into_response(),
    };
    authorization.query_pairs_mut().extend_pairs([
        ("response_type", "code"),
        ("client_id", client_id.as_str()),
        ("redirect_uri", callback_uri.as_str()),
        ("state", state_value.as_str()),
        ("scope", SCOPES),
        ("code_challenge", challenge.as_str()),
        ("code_challenge_method", "S256"),
    ]);
    let shutdown = CancellationToken::new();
    let pending = PendingAuthorization {
        verifier: Zeroizing::new(verifier),
        client_id,
        agent_id: query.agent.unwrap_or_else(|| "vak".into()),
        callback_uri,
        session_id: crate::web::session_cookie(&headers)
            .filter(|session| state.browser_sessions.valid(session))
            .map(str::to_owned),
        generation: 0,
        expires_at: Instant::now() + AUTH_TTL,
        shutdown: shutdown.clone(),
    };
    if state
        .linkedin_oauth
        .insert(state_value.clone(), pending)
        .is_err()
    {
        shutdown.cancel();
        return (
            StatusCode::TOO_MANY_REQUESTS,
            Json(serde_json::json!({"error":"Too many LinkedIn connection attempts are pending. Wait a few minutes and try again."})),
        )
            .into_response();
    }
    tokio::spawn(serve_callback_listener(state, listener, shutdown));
    Json(serde_json::json!({
        "authorization_url": authorization.as_str(),
        "expires_in_seconds": AUTH_TTL.as_secs(),
    }))
    .into_response()
}

#[derive(Deserialize)]
struct CallbackQuery {
    state: String,
    code: Option<String>,
    error: Option<String>,
    error_description: Option<String>,
}

impl Drop for CallbackQuery {
    fn drop(&mut self) {
        self.state.zeroize();
        self.code.zeroize();
        self.error.zeroize();
        self.error_description.zeroize();
    }
}

async fn serve_callback_listener(
    state: AppState,
    listener: TcpListener,
    shutdown: CancellationToken,
) {
    let app = axum::Router::new()
        .route(CALLBACK_PATH, axum::routing::get(callback))
        .with_state(state);
    let shutdown_signal = async move {
        tokio::select! {
            _ = shutdown.cancelled() => {},
            _ = tokio::time::sleep(AUTH_TTL) => {},
        }
    };
    let server = axum::serve(
        listener,
        app.into_make_service_with_connect_info::<SocketAddr>(),
    )
    .with_graceful_shutdown(shutdown_signal);
    let _ = server.await;
}

async fn callback(
    State(state): State<AppState>,
    ConnectInfo(peer): ConnectInfo<SocketAddr>,
    headers: HeaderMap,
    Query(query): Query<CallbackQuery>,
) -> Response {
    if !peer.ip().is_loopback() {
        return callback_page(
            StatusCode::FORBIDDEN,
            "Local connection required",
            "Return to Vakyartha and start the LinkedIn connection again.",
            false,
        );
    }
    let Some(pending) = state.linkedin_oauth.consume(&query.state) else {
        return callback_page(
            StatusCode::BAD_REQUEST,
            "Connection expired",
            "This LinkedIn sign-in expired or was already used. Return to Vakyartha and try again.",
            false,
        );
    };
    pending.shutdown.cancel();
    if Instant::now() >= pending.expires_at {
        return callback_page(
            StatusCode::BAD_REQUEST,
            "Connection expired",
            "This LinkedIn sign-in expired. Return to Vakyartha and try again.",
            false,
        );
    }
    let expected_host = Url::parse(&pending.callback_uri)
        .ok()
        .and_then(|url| Some(format!("{}:{}", url.host_str()?, url.port()?)));
    if headers
        .get(header::HOST)
        .and_then(|value| value.to_str().ok())
        != expected_host.as_deref()
    {
        return callback_page(
            StatusCode::BAD_REQUEST,
            "Invalid callback",
            "This LinkedIn callback did not match the local sign-in request.",
            false,
        );
    }
    if let Some(session_id) = pending.session_id.as_deref()
        && !state.browser_sessions.valid(session_id)
    {
        return callback_page(
            StatusCode::BAD_REQUEST,
            "Sign in again",
            "The Vakyartha session that started this connection has expired. Return to Settings and try again.",
            false,
        );
    }
    if query.error.is_some() {
        return callback_page(
            StatusCode::BAD_REQUEST,
            "Connection cancelled",
            "LinkedIn sign-in was cancelled. You can close this window.",
            false,
        );
    }
    let Some(code) = query.code.as_deref().filter(|code| valid_code(code)) else {
        return callback_page(
            StatusCode::BAD_REQUEST,
            "Connection incomplete",
            "LinkedIn did not return a valid authorization code. Return to Vakyartha and try again.",
            false,
        );
    };
    let Some(core) = linked_agent_core(&state, &pending.agent_id) else {
        return callback_page(
            StatusCode::NOT_FOUND,
            "Agent unavailable",
            "The Vakyartha Agent that started this connection is no longer available.",
            false,
        );
    };
    if !crate::social_plugin_enabled(&core, "social-linkedin") {
        return callback_page(
            StatusCode::CONFLICT,
            "Add-on disabled",
            "The LinkedIn add-on was disabled while sign-in was in progress. No account was connected.",
            false,
        );
    }
    if !state
        .linkedin_oauth
        .is_current(&pending.agent_id, pending.generation)
    {
        return callback_page(
            StatusCode::CONFLICT,
            "Connection cancelled",
            "The LinkedIn connection was cancelled before it could finish.",
            false,
        );
    }
    let redeemed = match exchange_and_read_profile(code, &pending).await {
        Ok(connection) => connection,
        Err(_) => {
            return callback_page(
                StatusCode::BAD_GATEWAY,
                "Connection failed",
                "LinkedIn could not complete sign-in. Check the app's OIDC and native PKCE setup, then try again.",
                false,
            );
        }
    };
    if pending
        .session_id
        .as_deref()
        .is_some_and(|session| !state.browser_sessions.valid(session))
    {
        return callback_page(
            StatusCode::BAD_REQUEST,
            "Sign in again",
            "The Vakyartha session expired before the account could be saved. Start again from Settings.",
            false,
        );
    }
    let Some(core) = linked_agent_core(&state, &pending.agent_id) else {
        return callback_page(
            StatusCode::NOT_FOUND,
            "Agent unavailable",
            "The Vakyartha Agent is no longer available. No account was saved.",
            false,
        );
    };
    if !crate::social_plugin_enabled(&core, "social-linkedin") {
        return callback_page(
            StatusCode::CONFLICT,
            "Add-on disabled",
            "The LinkedIn add-on was disabled while sign-in was in progress. No account was connected.",
            false,
        );
    }
    let mut serialized = match serde_json::to_string(&redeemed) {
        Ok(value) => Zeroizing::new(value),
        Err(_) => {
            return callback_page(
                StatusCode::INTERNAL_SERVER_ERROR,
                "Connection failed",
                "Vakyartha could not prepare the secure account record.",
                false,
            );
        }
    };
    let stored = state
        .linkedin_oauth
        .with_current(&pending.agent_id, pending.generation, || {
            vak_config::upsert_env_file(&core.scope().env_file(), CONNECTION_KEY, &serialized)
        });
    serialized.zeroize();
    match stored {
        Some(Ok(())) => callback_page(
            StatusCode::OK,
            "LinkedIn connected",
            "The LinkedIn profile is connected for owner-visible identity details only. You can close this window and return to Settings.",
            true,
        ),
        Some(Err(_)) => callback_page(
            StatusCode::INTERNAL_SERVER_ERROR,
            "Connection could not be saved",
            "Vakyartha could not save the account securely. Return to Settings and try again.",
            false,
        ),
        None => callback_page(
            StatusCode::CONFLICT,
            "Connection cancelled",
            "The LinkedIn connection was cancelled before Vakyartha could save it.",
            false,
        ),
    }
}

fn linked_agent_core(state: &AppState, agent_id: &str) -> Option<vak_core::Core> {
    if !crate::mail_calendar::valid_agent(state, agent_id) {
        return None;
    }
    crate::resolve_scoped_core(state, None, Some(agent_id)).ok()
}

fn load_connection(core: &vak_core::Core) -> Result<Option<StoredConnection>, ()> {
    let Some(mut contents) =
        vak_config::read_env_file_var(&core.scope().env_file(), CONNECTION_KEY)
    else {
        return Ok(None);
    };
    let parsed: Result<StoredConnection, ()> = serde_json::from_str(&contents).map_err(|_| ());
    contents.zeroize();
    parsed.map(Some)
}

async fn exchange_and_read_profile(
    code: &str,
    pending: &PendingAuthorization,
) -> Result<StoredConnection, ()> {
    let client = reqwest::Client::builder()
        .redirect(reqwest::redirect::Policy::none())
        .timeout(Duration::from_secs(12))
        .build()
        .map_err(|_| ())?;
    let response = client
        .post(TOKEN_ENDPOINT)
        .form(&[
            ("grant_type", "authorization_code"),
            ("code", code),
            ("redirect_uri", pending.callback_uri.as_str()),
            ("client_id", pending.client_id.as_str()),
            ("code_verifier", pending.verifier.as_str()),
        ])
        .send()
        .await
        .map_err(|_| ())?;
    if !response.status().is_success() {
        return Err(());
    }
    let mut token_body = bounded_body(response, MAX_TOKEN_RESPONSE).await?;
    let parsed_token: Result<TokenResponse, ()> =
        serde_json::from_slice(&token_body).map_err(|_| ());
    token_body.zeroize();
    let mut token_response = parsed_token?;
    if token_response.access_token.is_empty()
        || token_response.access_token.len() > 16 * 1024
        || token_response.access_token.chars().any(char::is_control)
        || token_response.expires_in == 0
        || token_response.expires_in > 365 * 24 * 60 * 60
        || token_response
            .token_type
            .as_deref()
            .is_some_and(|kind| !kind.eq_ignore_ascii_case("bearer"))
    {
        token_response.access_token.zeroize();
        return Err(());
    }
    let profile_response = client
        .get(USERINFO_ENDPOINT)
        .bearer_auth(&token_response.access_token)
        .send()
        .await
        .map_err(|_| {
            token_response.access_token.zeroize();
        })?;
    if !profile_response.status().is_success() {
        token_response.access_token.zeroize();
        return Err(());
    }
    let mut profile_body = bounded_body(profile_response, MAX_USERINFO_RESPONSE).await?;
    let parsed_profile: Result<UserInfo, ()> =
        serde_json::from_slice(&profile_body).map_err(|_| {
            token_response.access_token.zeroize();
        });
    profile_body.zeroize();
    let mut profile = parsed_profile?;
    if profile.sub.is_empty()
        || profile.sub.len() > 512
        || profile.sub.chars().any(char::is_control)
    {
        token_response.access_token.zeroize();
        return Err(());
    }
    let display_name = profile
        .name
        .filter(|name| {
            !name.trim().is_empty() && name.len() <= 256 && !name.chars().any(char::is_control)
        })
        .unwrap_or_else(|| "LinkedIn member".into());
    profile.sub.zeroize();
    let expires_at = Utc::now()
        .checked_add_signed(ChronoDuration::seconds(
            i64::try_from(token_response.expires_in).map_err(|_| ())?,
        ))
        .ok_or_else(|| {
            token_response.access_token.zeroize();
        })?;
    let scopes_returned = token_response.scope.take().map(|scope| {
        scope
            .split_ascii_whitespace()
            .filter(|scope| {
                scope.len() <= 128
                    && scope
                        .bytes()
                        .all(|byte| byte.is_ascii_alphanumeric() || byte == b'_')
            })
            .take(32)
            .map(str::to_owned)
            .collect()
    });
    let access_token = std::mem::take(&mut token_response.access_token);
    Ok(StoredConnection {
        access_token,
        display_name,
        scopes_requested: SCOPES.split_ascii_whitespace().map(str::to_owned).collect(),
        scopes_returned,
        expires_at,
    })
}

async fn bounded_body(response: reqwest::Response, maximum: usize) -> Result<Vec<u8>, ()> {
    if response
        .content_length()
        .is_some_and(|length| length > maximum as u64)
    {
        return Err(());
    }
    let mut stream = response.bytes_stream();
    let mut body = Vec::new();
    use futures::StreamExt;
    while let Some(chunk) = stream.next().await {
        let chunk = match chunk {
            Ok(chunk) => chunk,
            Err(_) => {
                body.zeroize();
                return Err(());
            }
        };
        if body.len().saturating_add(chunk.len()) > maximum {
            body.zeroize();
            return Err(());
        }
        body.extend_from_slice(&chunk);
    }
    Ok(body)
}

#[derive(Deserialize)]
struct TokenResponse {
    access_token: String,
    expires_in: u64,
    token_type: Option<String>,
    scope: Option<String>,
}

impl Drop for TokenResponse {
    fn drop(&mut self) {
        self.access_token.zeroize();
    }
}

#[derive(Deserialize)]
struct UserInfo {
    sub: String,
    name: Option<String>,
}

fn pkce_material() -> Result<(String, String, String), getrandom::Error> {
    use base64::Engine;
    let mut state = [0_u8; 32];
    let mut verifier = [0_u8; 32];
    getrandom::fill(&mut state)?;
    getrandom::fill(&mut verifier)?;
    let state = base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(state);
    let verifier = base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(verifier);
    let digest = sha2::Sha256::digest(verifier.as_bytes());
    let challenge = base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(digest);
    Ok((state, verifier, challenge))
}

fn valid_code(code: &str) -> bool {
    !code.is_empty() && code.len() <= 2048 && !code.chars().any(char::is_control)
}

fn callback_page(
    status: StatusCode,
    title: &'static str,
    message: &'static str,
    success: bool,
) -> Response {
    let close = if success { "window.close();" } else { "" };
    let html = format!(
        "<!doctype html><html lang=\"en\"><head><meta charset=\"utf-8\"><meta name=\"referrer\" content=\"no-referrer\"><meta http-equiv=\"Cache-Control\" content=\"no-store\"><title>{title}</title><script>history.replaceState(null,\"\",location.pathname);{close}</script></head><body><main><h1>{title}</h1><p>{message}</p></main></body></html>"
    );
    let mut response = (status, Html(html)).into_response();
    response.headers_mut().insert(
        header::CACHE_CONTROL,
        axum::http::HeaderValue::from_static("no-store"),
    );
    response.headers_mut().insert(
        header::REFERRER_POLICY,
        axum::http::HeaderValue::from_static("no-referrer"),
    );
    response.headers_mut().insert(
        header::X_CONTENT_TYPE_OPTIONS,
        axum::http::HeaderValue::from_static("nosniff"),
    );
    response.headers_mut().insert(
        header::CONTENT_SECURITY_POLICY,
        axum::http::HeaderValue::from_static(
            "default-src 'none'; script-src 'unsafe-inline'; style-src 'unsafe-inline'",
        ),
    );
    response
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pkce_values_are_url_safe_and_use_s256() {
        let (state, verifier, challenge) = pkce_material().expect("PKCE material");
        assert_eq!(state.len(), 43);
        assert_eq!(verifier.len(), 43);
        assert!(
            state
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'-'))
        );
        assert!(
            verifier
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'-'))
        );
        use base64::Engine;
        let expected = base64::engine::general_purpose::URL_SAFE_NO_PAD
            .encode(sha2::Sha256::digest(verifier.as_bytes()));
        assert_eq!(challenge, expected);
    }

    #[test]
    fn callback_state_is_single_use_and_pending_is_bounded() {
        let store = PendingAuthorizations::default();
        for index in 0..MAX_PENDING {
            store
                .insert(
                    format!("state-{index}"),
                    PendingAuthorization {
                        verifier: Zeroizing::new("x".into()),
                        client_id: "client".into(),
                        agent_id: "vak".into(),
                        callback_uri: "http://127.0.0.1:1234/social/linkedin/oauth/callback".into(),
                        session_id: None,
                        generation: 0,
                        expires_at: Instant::now() + AUTH_TTL,
                        shutdown: CancellationToken::new(),
                    },
                )
                .expect("pending flow");
        }
        assert!(
            store
                .insert(
                    "overflow".into(),
                    PendingAuthorization {
                        verifier: Zeroizing::new("x".into()),
                        client_id: "client".into(),
                        agent_id: "vak".into(),
                        callback_uri: "http://127.0.0.1:1234/social/linkedin/oauth/callback".into(),
                        session_id: None,
                        generation: 0,
                        expires_at: Instant::now() + AUTH_TTL,
                        shutdown: CancellationToken::new(),
                    }
                )
                .is_err()
        );
        assert!(store.consume("state-0").is_some());
        assert!(store.consume("state-0").is_none());
        assert!(store.consume("missing").is_none());
    }

    #[test]
    fn disconnect_fences_an_already_consumed_callback_before_it_can_persist() {
        let store = PendingAuthorizations::default();
        store
            .insert(
                "one-time-state".into(),
                PendingAuthorization {
                    verifier: Zeroizing::new("x".into()),
                    client_id: "client".into(),
                    agent_id: "agent-a".into(),
                    callback_uri: "http://127.0.0.1:1234/social/linkedin/oauth/callback".into(),
                    session_id: None,
                    generation: 0,
                    expires_at: Instant::now() + AUTH_TTL,
                    shutdown: CancellationToken::new(),
                },
            )
            .expect("pending flow");
        let flow = store.consume("one-time-state").expect("claimed flow");
        assert!(store.is_current("agent-a", flow.generation));
        store.cancel_agent("agent-a");
        assert!(!store.is_current("agent-a", flow.generation));
        assert!(
            store
                .with_current("agent-a", flow.generation, || true)
                .is_none()
        );
    }

    #[test]
    fn linkedin_client_ids_and_codes_have_bounds() {
        assert!(valid_code("authorization-code"));
        assert!(!valid_code("bad\ncode"));
        assert!(!valid_code(&"x".repeat(2049)));
    }
}
