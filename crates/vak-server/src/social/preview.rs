//! Owner-only Reddit and X search previews.
//!
//! Like the YouTube preview, results go to Settings only: they are not saved,
//! not recorded in a session and not shown to a model. That is what keeps them
//! clear of Reddit's deleted-content rule and the append-only ledger.
//! Reddit uses the installed-app grant, so only a public Client ID is held.
//! Every X search is counted against a monthly ceiling the owner sets, before
//! the request is sent, and searching stops when it is reached.

use crate::{AppState, AuthenticatedPrincipal, social_plugin_enabled};
use axum::{
    Json,
    extract::{Query, State},
    http::StatusCode,
    response::{IntoResponse, Response},
};
use futures::StreamExt;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::{path::Path, sync::Mutex, time::Duration};
use vak_plugin::native_adapter::{
    AdapterAuth, AdapterAvailability, CompiledExecutor, NativeAdapterRegistration,
    native_adapter_for_plugin,
};

const REDDIT_CLIENT_ID: &str = "VAK_SOCIAL_REDDIT_CLIENT_ID";
const X_TOKEN: &str = "VAK_SOCIAL_X_BEARER_TOKEN";
const REDDIT_TOKEN_URL: &str = "https://www.reddit.com/api/v1/access_token";
const USER_AGENT: &str = "vakyartha-owner-preview/1.0";
const DEFAULT_X_LIMIT: u32 = 20;
const MAX_X_LIMIT: u32 = 1000;
const NOTICE: &str = "Human-only preview. Results are returned to this screen and are not sent to the agent or saved to conversation history.";

#[derive(Deserialize)]
pub(crate) struct Query2 {
    agent: Option<String>,
    scope: Option<String>,
}

fn fail(status: StatusCode, message: &str) -> Response {
    (status, Json(json!({ "error": message }))).into_response()
}

fn adapter(
    plugin: &str,
    executor: CompiledExecutor,
    auth: AdapterAuth,
) -> Option<&'static NativeAdapterRegistration> {
    native_adapter_for_plugin(plugin).filter(|a| {
        a.availability == AdapterAvailability::OwnerPreview
            && a.executor == executor
            && a.auth == auth
    })
}

pub(crate) fn reddit_registered() -> bool {
    reddit_adapter().is_some()
}

pub(crate) fn x_registered() -> bool {
    x_adapter().is_some()
}

fn reddit_adapter() -> Option<&'static NativeAdapterRegistration> {
    adapter(
        "social-reddit",
        CompiledExecutor::RedditOwnerSearchPreview,
        AdapterAuth::OAuthInstalledApp,
    )
    .filter(|a| a.api_host == "oauth.reddit.com" && a.credential_binding == REDDIT_CLIENT_ID)
}

fn x_adapter() -> Option<&'static NativeAdapterRegistration> {
    adapter(
        "social-x",
        CompiledExecutor::XOwnerSearchPreview,
        AdapterAuth::BearerToken,
    )
    .filter(|a| a.api_host == "api.x.com" && a.credential_binding == X_TOKEN)
}

macro_rules! guard {
    ($state:expr, $principal:expr, $agent:expr) => {{
        if let Err(status) = crate::operator_only(&$principal) {
            return status.into_response();
        }
        match crate::resolve_scoped_core($state, None, $agent) {
            Ok(core) => core,
            Err(response) => return response,
        }
    }};
}

fn valid_secret(value: &str, max: usize) -> bool {
    !value.is_empty() && value.len() <= max && value.bytes().all(|b| b.is_ascii_graphic())
}

async fn credential_status(
    state: AppState,
    principal: AuthenticatedPrincipal,
    query: Query2,
    key: &str,
) -> Response {
    let core = guard!(&state, principal, query.agent.as_deref());
    Json(super::credential_state(&core, key, query.scope.as_deref())).into_response()
}

async fn credential_save(
    state: AppState,
    principal: AuthenticatedPrincipal,
    query: Query2,
    key: &str,
    value: &str,
    max: usize,
    what: &str,
) -> Response {
    let core = guard!(&state, principal, query.agent.as_deref());
    let value = value.trim();
    if !valid_secret(value, max) {
        return fail(StatusCode::BAD_REQUEST, &format!("Enter a valid {what}."));
    }
    match vak_config::upsert_env_file(
        &super::credential_file(&core, query.scope.as_deref()),
        key,
        value,
    ) {
        Ok(()) => Json(json!({ "configured": true })).into_response(),
        Err(_) => fail(
            StatusCode::INTERNAL_SERVER_ERROR,
            "Could not store the credential in secure credential storage.",
        ),
    }
}

async fn credential_remove(
    state: AppState,
    principal: AuthenticatedPrincipal,
    query: Query2,
    key: &str,
) -> Response {
    let core = guard!(&state, principal, query.agent.as_deref());
    match vak_config::remove_env_file_key(
        &super::credential_file(&core, query.scope.as_deref()),
        key,
    ) {
        Ok(()) => Json(json!({ "configured": false })).into_response(),
        Err(_) => fail(
            StatusCode::INTERNAL_SERVER_ERROR,
            "Could not remove the credential.",
        ),
    }
}

#[derive(Deserialize)]
pub(crate) struct ClientIdBody {
    client_id: String,
}
#[derive(Deserialize)]
pub(crate) struct TokenBody {
    token: String,
}

pub(crate) async fn reddit_client_id_status(
    State(s): State<AppState>,
    axum::Extension(p): axum::Extension<AuthenticatedPrincipal>,
    Query(q): Query<Query2>,
) -> Response {
    credential_status(s, p, q, REDDIT_CLIENT_ID).await
}
pub(crate) async fn reddit_client_id_save(
    State(s): State<AppState>,
    axum::Extension(p): axum::Extension<AuthenticatedPrincipal>,
    Query(q): Query<Query2>,
    Json(b): Json<ClientIdBody>,
) -> Response {
    credential_save(
        s,
        p,
        q,
        REDDIT_CLIENT_ID,
        &b.client_id,
        128,
        "Reddit app Client ID",
    )
    .await
}
pub(crate) async fn reddit_client_id_remove(
    State(s): State<AppState>,
    axum::Extension(p): axum::Extension<AuthenticatedPrincipal>,
    Query(q): Query<Query2>,
) -> Response {
    credential_remove(s, p, q, REDDIT_CLIENT_ID).await
}
pub(crate) async fn x_token_status(
    State(s): State<AppState>,
    axum::Extension(p): axum::Extension<AuthenticatedPrincipal>,
    Query(q): Query<Query2>,
) -> Response {
    credential_status(s, p, q, X_TOKEN).await
}
pub(crate) async fn x_token_save(
    State(s): State<AppState>,
    axum::Extension(p): axum::Extension<AuthenticatedPrincipal>,
    Query(q): Query<Query2>,
    Json(b): Json<TokenBody>,
) -> Response {
    credential_save(s, p, q, X_TOKEN, &b.token, 1024, "X bearer token").await
}
pub(crate) async fn x_token_remove(
    State(s): State<AppState>,
    axum::Extension(p): axum::Extension<AuthenticatedPrincipal>,
    Query(q): Query<Query2>,
) -> Response {
    credential_remove(s, p, q, X_TOKEN).await
}

#[derive(Serialize, Deserialize, Default, Clone)]
struct Usage {
    month: String,
    limit: u32,
    used: u32,
}

static USAGE_LOCK: Mutex<()> = Mutex::new(());

fn load_usage(path: &Path) -> Usage {
    let mut usage: Usage = std::fs::read(path)
        .ok()
        .and_then(|b| serde_json::from_slice(&b).ok())
        .unwrap_or_default();
    let month = chrono::Utc::now().format("%Y-%m").to_string();
    if usage.month != month {
        usage.month = month;
        usage.used = 0;
    }
    if usage.limit == 0 {
        usage.limit = DEFAULT_X_LIMIT;
    }
    usage
}

fn store_usage(path: &Path, usage: &Usage) -> std::io::Result<()> {
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    let tmp = path.with_extension("json.tmp");
    std::fs::write(&tmp, serde_json::to_vec(usage)?)?;
    std::fs::rename(tmp, path)
}

/// Counts one search before it is sent, so a failed or timed-out request still
/// spends from the ceiling: the ceiling bounds attempts, which can only
/// overstate what the provider billed.
fn reserve_x_search(path: &Path) -> Result<Usage, Usage> {
    let _guard = USAGE_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let mut usage = load_usage(path);
    if usage.used >= usage.limit {
        return Err(usage);
    }
    usage.used += 1;
    if store_usage(path, &usage).is_err() {
        usage.used = usage.limit;
        return Err(usage);
    }
    Ok(usage)
}

pub(crate) async fn x_usage_status(
    State(state): State<AppState>,
    axum::Extension(p): axum::Extension<AuthenticatedPrincipal>,
    Query(q): Query<Query2>,
) -> Response {
    let core = guard!(&state, p, q.agent.as_deref());
    let _guard = USAGE_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    Json(load_usage(&core.shared_scope().social_x_usage())).into_response()
}

#[derive(Deserialize)]
pub(crate) struct LimitBody {
    limit: u32,
}

pub(crate) async fn x_usage_limit(
    State(state): State<AppState>,
    axum::Extension(p): axum::Extension<AuthenticatedPrincipal>,
    Query(q): Query<Query2>,
    Json(b): Json<LimitBody>,
) -> Response {
    let core = guard!(&state, p, q.agent.as_deref());
    if !(1..=MAX_X_LIMIT).contains(&b.limit) {
        return fail(
            StatusCode::BAD_REQUEST,
            &format!("Choose a monthly limit from 1 to {MAX_X_LIMIT} searches."),
        );
    }
    let path = core.shared_scope().social_x_usage();
    let _guard = USAGE_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let mut usage = load_usage(&path);
    usage.limit = b.limit;
    match store_usage(&path, &usage) {
        Ok(()) => Json(usage).into_response(),
        Err(_) => fail(
            StatusCode::INTERNAL_SERVER_ERROR,
            "Could not save the limit.",
        ),
    }
}

#[derive(Deserialize)]
pub(crate) struct SearchBody {
    query: String,
    #[serde(default = "default_results")]
    max_results: u8,
}
fn default_results() -> u8 {
    5
}

fn client(a: &NativeAdapterRegistration) -> Option<reqwest::Client> {
    reqwest::Client::builder()
        .timeout(Duration::from_secs(u64::from(a.timeout_seconds)))
        .redirect(reqwest::redirect::Policy::none())
        .user_agent(USER_AGENT)
        .build()
        .ok()
}

async fn bounded_json(response: reqwest::Response, max: usize) -> Result<Value, &'static str> {
    if !response.status().is_success() {
        return Err("The platform rejected the credential or the request.");
    }
    if response.content_length().is_some_and(|n| n as usize > max) {
        return Err("The response exceeded the size limit.");
    }
    let mut stream = response.bytes_stream();
    let mut bytes = Vec::new();
    while let Some(chunk) = stream.next().await {
        match chunk {
            Ok(chunk) if bytes.len().saturating_add(chunk.len()) <= max => {
                bytes.extend_from_slice(&chunk)
            }
            _ => return Err("The response exceeded the size limit or could not be read."),
        }
    }
    serde_json::from_slice(&bytes).map_err(|_| "The platform returned an invalid response.")
}

fn snippet(text: &str, max_chars: usize) -> String {
    let text = text.split_whitespace().collect::<Vec<_>>().join(" ");
    if text.chars().count() <= max_chars {
        return text;
    }
    let mut cut: String = text.chars().take(max_chars).collect();
    cut.push('…');
    cut
}

fn valid_query(body: &SearchBody, max_results: u8) -> bool {
    let q = body.query.trim();
    !q.is_empty()
        && q.len() <= 200
        && !q.chars().any(char::is_control)
        && (1..=max_results).contains(&body.max_results)
}

fn str_of<'a>(v: &'a Value, key: &str) -> &'a str {
    v.get(key).and_then(Value::as_str).unwrap_or("")
}

fn safe_token(s: &str, max: usize) -> bool {
    !s.is_empty()
        && s.len() <= max
        && s.chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-')
}

pub(crate) async fn reddit_search(
    State(state): State<AppState>,
    axum::Extension(p): axum::Extension<AuthenticatedPrincipal>,
    Query(q): Query<Query2>,
    Json(body): Json<SearchBody>,
) -> Response {
    let core = guard!(&state, p, q.agent.as_deref());
    if !social_plugin_enabled(&core, "social-reddit") {
        return fail(
            StatusCode::FORBIDDEN,
            "Enable the Reddit add-on before searching.",
        );
    }
    let Some(a) = reddit_adapter() else {
        return fail(
            StatusCode::SERVICE_UNAVAILABLE,
            "The compiled Reddit preview adapter is unavailable.",
        );
    };
    if !valid_query(&body, a.max_results) {
        return fail(
            StatusCode::BAD_REQUEST,
            "Search text must be 1–200 characters and result count must be 1–10.",
        );
    }
    let Some(client_id) = super::resolve_credential(&core, REDDIT_CLIENT_ID) else {
        return fail(
            StatusCode::PRECONDITION_FAILED,
            "Add your Reddit app Client ID first.",
        );
    };
    let Some(http) = client(a) else {
        return StatusCode::INTERNAL_SERVER_ERROR.into_response();
    };
    let token = match http
        .post(REDDIT_TOKEN_URL)
        .basic_auth(&client_id, Some(""))
        .form(&[
            (
                "grant_type",
                "https://oauth.reddit.com/grants/installed_client",
            ),
            ("device_id", "DO_NOT_TRACK_THIS_DEVICE"),
        ])
        .send()
        .await
    {
        Ok(r) => match bounded_json(r, 16 * 1024).await {
            Ok(v) => v
                .get("access_token")
                .and_then(Value::as_str)
                .map(str::to_owned),
            Err(e) => {
                return fail(
                    StatusCode::BAD_GATEWAY,
                    &format!(
                        "Reddit sign-in failed. {e} Check that the app is an installed app and the Client ID is right."
                    ),
                );
            }
        },
        Err(_) => {
            return fail(
                StatusCode::BAD_GATEWAY,
                "Reddit sign-in failed or timed out.",
            );
        }
    };
    let Some(token) = token else {
        return fail(
            StatusCode::BAD_GATEWAY,
            "Reddit did not issue an access token for this Client ID.",
        );
    };
    let limit = body.max_results.to_string();
    let response = match http
        .get(format!("https://{}/search", a.api_host))
        .bearer_auth(&token)
        .query(&[
            ("q", body.query.trim()),
            ("limit", limit.as_str()),
            ("type", "link"),
            ("raw_json", "1"),
            ("include_over_18", "off"),
        ])
        .send()
        .await
    {
        Ok(r) => r,
        Err(_) => {
            return fail(
                StatusCode::BAD_GATEWAY,
                "Reddit search failed or timed out.",
            );
        }
    };
    let data = match bounded_json(response, a.max_response_bytes).await {
        Ok(v) => v,
        Err(e) => return fail(StatusCode::BAD_GATEWAY, e),
    };
    let items = data["data"]["children"]
        .as_array()
        .map(|c| c.as_slice())
        .unwrap_or_default()
        .iter()
        .filter_map(|child| {
            let d = &child["data"];
            let permalink = str_of(d, "permalink");
            if !permalink.starts_with("/r/")
                || permalink.chars().any(char::is_control)
                || !safe_token(str_of(d, "id"), 32)
            {
                return None;
            }
            let created = d["created_utc"]
                .as_f64()
                .and_then(|t| chrono::DateTime::from_timestamp(t as i64, 0))
                .map(|t| t.to_rfc3339())
                .unwrap_or_default();
            Some(json!({
                "id": str_of(d, "id"),
                "url": format!("https://www.reddit.com{permalink}"),
                "title": snippet(str_of(d, "title"), 300),
                "subreddit": str_of(d, "subreddit_name_prefixed"),
                "snippet": snippet(str_of(d, "selftext"), 280),
                "score": d["score"].as_i64().unwrap_or(0),
                "comments": d["num_comments"].as_u64().unwrap_or(0),
                "created_at": created,
            }))
        })
        .collect::<Vec<_>>();
    Json(json!({ "items": items, "notice": NOTICE })).into_response()
}

pub(crate) async fn x_search(
    State(state): State<AppState>,
    axum::Extension(p): axum::Extension<AuthenticatedPrincipal>,
    Query(q): Query<Query2>,
    Json(body): Json<SearchBody>,
) -> Response {
    let core = guard!(&state, p, q.agent.as_deref());
    if !social_plugin_enabled(&core, "social-x") {
        return fail(
            StatusCode::FORBIDDEN,
            "Enable the X add-on before searching.",
        );
    }
    let Some(a) = x_adapter() else {
        return fail(
            StatusCode::SERVICE_UNAVAILABLE,
            "The compiled X preview adapter is unavailable.",
        );
    };
    if !valid_query(&body, a.max_results) {
        return fail(
            StatusCode::BAD_REQUEST,
            "Search text must be 1–200 characters and result count must be 1–10.",
        );
    }
    let Some(token) = super::resolve_credential(&core, X_TOKEN) else {
        return fail(
            StatusCode::PRECONDITION_FAILED,
            "Add your X bearer token first.",
        );
    };
    let usage = match reserve_x_search(&core.shared_scope().social_x_usage()) {
        Ok(usage) => usage,
        Err(usage) => {
            return fail(
                StatusCode::TOO_MANY_REQUESTS,
                &format!(
                    "The monthly X search limit ({}) is reached. Raise it in Settings to continue.",
                    usage.limit
                ),
            );
        }
    };
    let Some(http) = client(a) else {
        return StatusCode::INTERNAL_SERVER_ERROR.into_response();
    };
    // X accepts 10 to 100 results per search; the first ten are shown.
    let response = match http
        .get(format!("https://{}/2/tweets/search/recent", a.api_host))
        .bearer_auth(&token)
        .query(&[
            ("query", body.query.trim()),
            ("max_results", "10"),
            ("tweet.fields", "created_at,public_metrics"),
            ("expansions", "author_id"),
            ("user.fields", "username"),
        ])
        .send()
        .await
    {
        Ok(r) => r,
        Err(_) => {
            return fail(
                StatusCode::BAD_GATEWAY,
                "X search failed or timed out. The attempt still counts toward your monthly limit.",
            );
        }
    };
    let data = match bounded_json(response, a.max_response_bytes).await {
        Ok(v) => v,
        Err(e) => return fail(StatusCode::BAD_GATEWAY, e),
    };
    let users: std::collections::HashMap<&str, &str> = data["includes"]["users"]
        .as_array()
        .map(|u| u.as_slice())
        .unwrap_or_default()
        .iter()
        .map(|u| (str_of(u, "id"), str_of(u, "username")))
        .collect();
    let items = data["data"]
        .as_array()
        .map(|d| d.as_slice())
        .unwrap_or_default()
        .iter()
        .take(usize::from(body.max_results))
        .filter_map(|post| {
            let id = str_of(post, "id");
            let handle = users.get(str_of(post, "author_id")).copied().unwrap_or("");
            if !id.bytes().all(|b| b.is_ascii_digit())
                || id.is_empty()
                || id.len() > 32
                || !safe_token(handle, 15)
            {
                return None;
            }
            Some(json!({
                "id": id,
                "url": format!("https://x.com/{handle}/status/{id}"),
                "handle": handle,
                "text": snippet(str_of(post, "text"), 400),
                "likes": post["public_metrics"]["like_count"].as_u64().unwrap_or(0),
                "reposts": post["public_metrics"]["retweet_count"].as_u64().unwrap_or(0),
                "created_at": str_of(post, "created_at"),
            }))
        })
        .collect::<Vec<_>>();
    Json(json!({ "items": items, "notice": NOTICE, "usage": usage })).into_response()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn x_ceiling_blocks_the_search_after_the_limit_and_counts_attempts() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("x-usage.json");
        store_usage(
            &path,
            &Usage {
                month: chrono::Utc::now().format("%Y-%m").to_string(),
                limit: 2,
                used: 0,
            },
        )
        .unwrap();
        assert_eq!(reserve_x_search(&path).unwrap().used, 1);
        assert_eq!(reserve_x_search(&path).unwrap().used, 2);
        assert!(reserve_x_search(&path).is_err());
        assert_eq!(load_usage(&path).used, 2);
    }

    #[test]
    fn x_usage_rolls_over_a_new_month_and_defaults_the_limit() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("x-usage.json");
        store_usage(
            &path,
            &Usage {
                month: "2000-01".into(),
                limit: 0,
                used: 9,
            },
        )
        .unwrap();
        let usage = load_usage(&path);
        assert_eq!((usage.used, usage.limit), (0, DEFAULT_X_LIMIT));
    }

    #[test]
    fn snippets_cut_on_characters() {
        assert_eq!(snippet("héllo wörld", 5), "héllo…");
    }

    #[test]
    fn both_previews_are_registered_to_their_fixed_hosts() {
        assert!(reddit_adapter().is_some());
        assert!(x_adapter().is_some());
    }
}
