//! Owner-facing account linking for the mail and calendar package.
//!
//! These routes expose only connection metadata. Provider content operations
//! remain unavailable until lifecycle erasure can account for their lineage.

use crate::{AppState, AuthenticatedPrincipal, agents};
use axum::{
    Json,
    body::Bytes,
    extract::{ConnectInfo, Path, Query, State},
    http::{HeaderMap, StatusCode, header},
    response::{Html, IntoResponse, Response},
};
use chrono::Utc;
use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;
use std::net::SocketAddr;
use uuid::Uuid;
use vak_mail_calendar::{
    AccountStatus, Capability, ConnectedAccount, Provider,
    connection_ledger::ConnectionLedger,
    vault::{AccountSecretMaterial, AccountVault},
};
use zeroize::{Zeroize, Zeroizing};

const OAUTH_CALLBACK_COOKIE: &str = "vak_mail_calendar_oauth";
const OAUTH_CALLBACK_COOKIE_MAX_AGE_SECONDS: u64 = 10 * 60;

#[derive(Deserialize)]
pub(super) struct AccountQuery {
    agent_id: String,
}

#[derive(Serialize)]
struct AccountView {
    id: String,
    provider: Provider,
    status: AccountStatus,
    identity_masked: Option<String>,
    credential_available: bool,
    capabilities: Vec<Capability>,
    connected_at: chrono::DateTime<chrono::Utc>,
    access_token_expires_at: Option<chrono::DateTime<chrono::Utc>>,
    refresh_token_available: bool,
    revoked_at: Option<chrono::DateTime<chrono::Utc>>,
}

enum RefreshCommitError {
    AgentInactive,
    Credential(vak_mail_calendar::oauth::OAuthRefreshError),
}

pub(super) async fn list_accounts(
    State(state): State<AppState>,
    axum::Extension(principal): axum::Extension<AuthenticatedPrincipal>,
    Query(query): Query<AccountQuery>,
) -> Response {
    if !operator(&principal) {
        return StatusCode::FORBIDDEN.into_response();
    }
    if !registered_agent(&state, &query.agent_id) {
        return StatusCode::NOT_FOUND.into_response();
    }
    let ledger = match ConnectionLedger::for_agent(&query.agent_id) {
        Ok(ledger) => ledger,
        Err(_) => return StatusCode::BAD_REQUEST.into_response(),
    };
    match ledger.read_all() {
        Ok(accounts) => {
            let vault = match AccountVault::for_agent(&query.agent_id) {
                Ok(vault) => vault,
                Err(_) => return StatusCode::SERVICE_UNAVAILABLE.into_response(),
            };
            Json(serde_json::json!({
                    "accounts": accounts.into_iter().map(|account| {
                    let credential = vault.load(&account.id);
                    let identity_masked = credential.as_ref().ok()
                        .and_then(|secret| secret.masked_display_identity());
                    AccountView {
                        id: account.id,
                        provider: account.provider,
                        status: account.status,
                        identity_masked,
                        credential_available: credential.is_ok(),
                        capabilities: account.capabilities.into_iter().collect(),
                        connected_at: account.connected_at,
                        access_token_expires_at: account.access_token_expires_at,
                        refresh_token_available: account.refresh_token_available,
                        revoked_at: account.revoked_at,
                    }
                }).collect::<Vec<_>>()
            }))
            .into_response()
        }
        Err(_) => StatusCode::SERVICE_UNAVAILABLE.into_response(),
    }
}

#[derive(Deserialize)]
pub(super) struct BeginRequest {
    provider: Provider,
    capabilities: Vec<Capability>,
}

fn oauth_callback_uri(
    provider: Provider,
    request_host: &str,
    listener_port: u16,
) -> Result<url::Url, StatusCode> {
    if provider == Provider::AppleIcloud {
        return Err(StatusCode::BAD_REQUEST);
    }
    let mut callback =
        url::Url::parse(&format!("http://{request_host}/")).map_err(|_| StatusCode::BAD_REQUEST)?;
    if !matches!(callback.host_str(), Some("127.0.0.1" | "[::1]" | "::1")) {
        return Err(StatusCode::FORBIDDEN);
    }
    callback
        .set_port(Some(listener_port))
        .map_err(|_| StatusCode::BAD_REQUEST)?;
    // Entra ignores an ephemeral port for a native localhost redirect.
    // Google's desktop installed-app flow uses the loopback IP form.
    if provider == Provider::Microsoft {
        callback
            .set_host(Some("localhost"))
            .map_err(|_| StatusCode::BAD_REQUEST)?;
    }
    callback.set_path("/mail-calendar/oauth/callback");
    Ok(callback)
}

pub(super) async fn begin_oauth(
    State(state): State<AppState>,
    axum::Extension(principal): axum::Extension<AuthenticatedPrincipal>,
    ConnectInfo(peer): ConnectInfo<SocketAddr>,
    Path(agent_id): Path<String>,
    headers: HeaderMap,
    body: Bytes,
) -> Response {
    if !operator(&principal) {
        return StatusCode::FORBIDDEN.into_response();
    }
    if !valid_agent(&state, &agent_id) {
        return StatusCode::NOT_FOUND.into_response();
    }
    if state.core.config().server.public_url.is_some() || !is_loopback_request(&headers, peer) {
        return (
            StatusCode::FORBIDDEN,
            "Account linking is currently limited to a local server; hosted callbacks are not configured.",
        )
            .into_response();
    }
    // Bound malformed JSON before deserialization and never accept client IDs,
    // scopes, redirect URIs, audience IDs, or vault paths from the browser.
    if body.len() > 4096 {
        return StatusCode::PAYLOAD_TOO_LARGE.into_response();
    }
    let request: BeginRequest = match serde_json::from_slice(&body) {
        Ok(request) => request,
        Err(_) => return StatusCode::BAD_REQUEST.into_response(),
    };
    // Serialize initiation with disconnect so a new flow cannot slip between
    // cancellation and the durable revocation tombstone.
    let Some(_provider_guard) =
        active_provider_link_guard(&state, &agent_id, request.provider).await
    else {
        return StatusCode::NOT_FOUND.into_response();
    };
    let host = match headers
        .get(axum::http::header::HOST)
        .and_then(|v| v.to_str().ok())
    {
        Some(host) if host.len() <= 255 => host,
        _ => return StatusCode::BAD_REQUEST.into_response(),
    };
    let callback_uri = match oauth_callback_uri(request.provider, host, state.ops_port) {
        Ok(uri) => uri,
        Err(status) => return status.into_response(),
    };
    let client_id_name = match request.provider {
        Provider::Google => "VAK_GOOGLE_OAUTH_CLIENT_ID",
        Provider::Microsoft => "VAK_MICROSOFT_OAUTH_CLIENT_ID",
        Provider::AppleIcloud => return StatusCode::BAD_REQUEST.into_response(),
    };
    let Some(client_id) =
        vak_config::get_var(client_id_name).filter(|value| !value.trim().is_empty())
    else {
        return (
            StatusCode::SERVICE_UNAVAILABLE,
            "This provider is not configured on this installation.",
        )
            .into_response();
    };
    let session =
        crate::web::session_cookie(&headers).filter(|token| state.browser_sessions.valid(token));
    let native = session.is_none();
    if native
        && !headers
            .get(header::AUTHORIZATION)
            .and_then(|value| value.to_str().ok())
            .is_some_and(|value| value.starts_with("Bearer "))
    {
        return StatusCode::UNAUTHORIZED.into_response();
    }
    let existing_callback_binding = if native {
        None
    } else {
        oauth_callback_cookie(&headers)
    };
    let audience = format!("agent:{agent_id}");
    let result = if native {
        state
            .mail_calendar_oauth
            .begin_native(
                request.provider,
                &agent_id,
                &[audience],
                &request.capabilities,
                &client_id,
                callback_uri.as_str(),
            )
            .map(|(url, _account_id)| (url, None))
    } else {
        state
            .mail_calendar_oauth
            .begin(
                request.provider,
                &agent_id,
                session,
                &[audience],
                &request.capabilities,
                &client_id,
                callback_uri.as_str(),
                existing_callback_binding,
            )
            .map(|(url, _account_id, binding)| (url, Some(binding)))
    };
    match result {
        Ok((authorization_url, binding)) => {
            let mut response =
                Json(serde_json::json!({ "authorization_url": authorization_url })).into_response();
            if let Some(binding) = binding {
                let cookie = oauth_callback_set_cookie(
                    &binding,
                    state.core.config().server.cookie_is_secure()
                        || crate::web::forwarded_proto(&headers) == Some("https"),
                );
                if let Ok(value) = cookie.parse() {
                    response.headers_mut().insert(header::SET_COOKIE, value);
                }
            }
            response
        }
        Err(_) => (
            StatusCode::BAD_REQUEST,
            "The requested account connection is invalid or unavailable.",
        )
            .into_response(),
    }
}

#[derive(Deserialize)]
struct IcloudConnectRequest {
    email: SensitiveInput,
    app_specific_password: SensitiveInput,
    capabilities: Vec<Capability>,
}

impl Drop for IcloudConnectRequest {
    fn drop(&mut self) {
        self.email.0.zeroize();
        self.app_specific_password.0.zeroize();
    }
}

/// Wrap each sensitive JSON field as soon as serde has decoded it, so a later
/// parse error also drops the field through Zeroizing.
struct SensitiveInput(Zeroizing<String>);

impl<'de> Deserialize<'de> for SensitiveInput {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        String::deserialize(deserializer).map(|value| Self(Zeroizing::new(value)))
    }
}

/// Store an iCloud app-specific password without exposing it to a provider
/// request here. Content reads remain disabled until the lifecycle erasure
/// gate is implemented.
pub(super) async fn connect_icloud(
    State(state): State<AppState>,
    axum::Extension(principal): axum::Extension<AuthenticatedPrincipal>,
    ConnectInfo(peer): ConnectInfo<SocketAddr>,
    Path(agent_id): Path<String>,
    headers: HeaderMap,
    body: Bytes,
) -> Response {
    if !operator(&principal) {
        return StatusCode::FORBIDDEN.into_response();
    }
    if !valid_agent(&state, &agent_id) {
        return StatusCode::NOT_FOUND.into_response();
    }
    let Some(_provider_guard) =
        active_provider_link_guard(&state, &agent_id, Provider::AppleIcloud).await
    else {
        return StatusCode::NOT_FOUND.into_response();
    };
    if state.core.config().server.public_url.is_some() || !is_loopback_request(&headers, peer) {
        return (
            StatusCode::FORBIDDEN,
            "iCloud app-specific passwords can only be added to a local server.",
        )
            .into_response();
    }
    if body.len() > 4096 {
        wipe_request_bytes(body);
        return StatusCode::PAYLOAD_TOO_LARGE.into_response();
    }
    let parsed = serde_json::from_slice::<IcloudConnectRequest>(&body);
    wipe_request_bytes(body);
    let mut request = match parsed {
        Ok(request) => request,
        Err(_) => return StatusCode::BAD_REQUEST.into_response(),
    };
    let email = Zeroizing::new(request.email.0.trim().to_owned());
    request.email.0.zeroize();
    let valid_email = valid_icloud_email(&email);
    let valid_password = (16..=32).contains(&request.app_specific_password.0.len())
        && request
            .app_specific_password
            .0
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-');
    let capabilities = request
        .capabilities
        .iter()
        .copied()
        .collect::<BTreeSet<_>>();
    let apple_read_only = capabilities.len() == request.capabilities.len()
        && !capabilities.is_empty()
        && capabilities.iter().all(|capability| {
            matches!(
                capability,
                Capability::MailRead | Capability::CalendarFreeBusy | Capability::CalendarRead
            )
        });
    let audit_capabilities = capabilities.clone();
    if !valid_email || !valid_password || !apple_read_only {
        request.app_specific_password.0.zeroize();
        return (
            StatusCode::BAD_REQUEST,
            "Enter a valid iCloud email, app-specific password, and read-only access selection.",
        )
            .into_response();
    }
    let account_id = Uuid::now_v7().to_string();
    let vault = match AccountVault::for_agent(&agent_id) {
        Ok(vault) => vault,
        Err(_) => {
            request.app_specific_password.0.zeroize();
            return StatusCode::INTERNAL_SERVER_ERROR.into_response();
        }
    };
    let ledger = match ConnectionLedger::for_agent(&agent_id) {
        Ok(ledger) => ledger,
        Err(_) => {
            request.app_specific_password.0.zeroize();
            return StatusCode::INTERNAL_SERVER_ERROR.into_response();
        }
    };
    let credential_ref = match AccountVault::credential_ref(&account_id) {
        Ok(reference) => reference,
        Err(_) => {
            request.app_specific_password.0.zeroize();
            return StatusCode::INTERNAL_SERVER_ERROR.into_response();
        }
    };
    let material = match AccountSecretMaterial::new(
        email.to_string(),
        Some(email.to_string()),
        None,
        None,
        None,
        Some(email.to_string()),
        Some(std::mem::take(&mut *request.app_specific_password.0)),
    ) {
        Ok(material) => material,
        Err(_) => {
            request.app_specific_password.0.zeroize();
            return StatusCode::BAD_REQUEST.into_response();
        }
    };
    let now = Utc::now();
    let mut account = ConnectedAccount {
        id: account_id.clone(),
        provider: Provider::AppleIcloud,
        status: AccountStatus::Pending,
        owner_agent_id: agent_id.clone(),
        allowed_audiences: [format!("agent:{agent_id}")].into_iter().collect(),
        capabilities,
        provider_scopes: BTreeSet::new(),
        credential_ref: credential_ref.clone(),
        principal_ref: credential_ref,
        revision: 1,
        connected_at: now,
        access_token_expires_at: None,
        refresh_token_available: false,
        revoked_at: None,
    };
    match ledger.append_pending_if(account.clone(), |existing| {
        for linked in existing.iter().filter(|linked| {
            linked.provider == Provider::AppleIcloud && linked.revoked_at.is_none()
        }) {
            if linked.status == AccountStatus::Pending {
                return Err(IcloudPendingCheckError::LinkInProgress);
            }
            if !matches!(
                linked.status,
                AccountStatus::Connected | AccountStatus::ReauthenticationRequired
            ) {
                continue;
            }
            let stored = vault
                .load(&linked.id)
                .map_err(|_| IcloudPendingCheckError::VaultUnavailable)?;
            if stored.has_same_principal_ignoring_ascii_case(&material) {
                return Err(IcloudPendingCheckError::AlreadyConnected);
            }
        }
        Ok(())
    }) {
        Ok(()) => {}
        Err(vak_mail_calendar::connection_ledger::ConditionalAppendError::Check(
            IcloudPendingCheckError::AlreadyConnected,
        )) => {
            request.app_specific_password.0.zeroize();
            return (
                StatusCode::CONFLICT,
                "This iCloud account is already connected to this Agent. Disconnect it before changing its access selection.",
            )
                .into_response();
        }
        Err(vak_mail_calendar::connection_ledger::ConditionalAppendError::Check(
            IcloudPendingCheckError::LinkInProgress,
        )) => {
            request.app_specific_password.0.zeroize();
            return (
                StatusCode::CONFLICT,
                "Another iCloud account connection for this Agent is still pending. Finish its cleanup before trying again.",
            )
                .into_response();
        }
        Err(_) => {
            request.app_specific_password.0.zeroize();
            return StatusCode::SERVICE_UNAVAILABLE.into_response();
        }
    }
    account.status = AccountStatus::Connected;
    account.revision = 2;
    let linked_account_id = account_id.clone();
    if ledger
        .append_connected_if_pending(account, || vault.store(&linked_account_id, material))
        .is_err()
    {
        let _ = ledger.append_revoked(&account_id, Utc::now());
        let _ = vault.remove(&account_id);
        return StatusCode::SERVICE_UNAVAILABLE.into_response();
    }
    record_account_event(
        &state,
        "account_connected",
        &agent_id,
        &account_id,
        Provider::AppleIcloud,
        &audit_capabilities,
        "connected_unverified",
    );
    Json(serde_json::json!({ "connected": true })).into_response()
}

fn is_loopback_request(headers: &HeaderMap, peer: SocketAddr) -> bool {
    if !peer.ip().is_loopback() {
        return false;
    }
    // A local reverse proxy can hide a remote browser behind a loopback peer
    // and rewrite Host. Account credentials must travel directly from the
    // local browser to Vak, never through a proxy hop.
    if [
        "forwarded",
        "via",
        "x-forwarded-for",
        "x-forwarded-host",
        "x-forwarded-proto",
        "x-real-ip",
    ]
    .iter()
    .any(|name| headers.contains_key(*name))
    {
        return false;
    }
    headers
        .get(axum::http::header::HOST)
        .and_then(|value| value.to_str().ok())
        .and_then(|host| url::Url::parse(&format!("http://{host}/")).ok())
        .is_some_and(|url| {
            matches!(
                url.host_str(),
                Some("localhost" | "127.0.0.1" | "::1" | "[::1]")
            )
        })
}

#[derive(Debug)]
enum IcloudPendingCheckError {
    AlreadyConnected,
    LinkInProgress,
    VaultUnavailable,
}

fn wipe_request_bytes(body: Bytes) {
    if let Ok(mut bytes) = body.try_into_mut() {
        bytes.as_mut().zeroize();
    }
}

fn valid_icloud_email(email: &str) -> bool {
    if email.len() > 320 || !email.is_ascii() {
        return false;
    }
    let Some((local, domain)) = email.split_once('@') else {
        return false;
    };
    let valid_local = !local.is_empty()
        && local.len() <= 64
        && !local.starts_with('.')
        && !local.ends_with('.')
        && !local.contains("..")
        && local.bytes().all(|byte| {
            byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'_' | b'%' | b'+' | b'-')
        });
    let labels = domain.split('.').collect::<Vec<_>>();
    let valid_domain = labels.len() >= 2
        && labels.iter().all(|label| {
            !label.is_empty()
                && label.len() <= 63
                && label.as_bytes()[0].is_ascii_alphanumeric()
                && label.as_bytes()[label.len() - 1].is_ascii_alphanumeric()
                && label
                    .bytes()
                    .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-')
        });
    valid_local && valid_domain
}

fn record_account_event(
    state: &AppState,
    label: &str,
    agent_id: &str,
    account_id: &str,
    provider: Provider,
    capabilities: &BTreeSet<Capability>,
    outcome: &str,
) {
    let detail = serde_json::json!({
        "agent_id": agent_id,
        "account_id": account_id,
        "provider": provider,
        "capabilities": capabilities,
        "outcome": outcome,
    })
    .to_string();
    vak_core::security_events::record(
        &state.core.sessions_home(),
        vak_core::security_events::EventKind::MailCalendarAccount,
        label,
        &detail,
        None,
    );
}

#[cfg(test)]
mod tests {
    use super::{
        OAUTH_CALLBACK_COOKIE, is_loopback_request, oauth_callback_cookie, oauth_callback_page,
        oauth_callback_set_cookie, oauth_callback_uri, registered_agent, valid_agent,
        valid_icloud_email,
    };
    use axum::http::{HeaderMap, HeaderValue, StatusCode, header};
    use vak_mail_calendar::Provider;

    #[tokio::test]
    async fn callback_page_clears_the_query_and_only_closes_on_success() {
        let success = oauth_callback_page(
            StatusCode::OK,
            "Account connected",
            "You can close this window and return to Settings.",
            true,
        );
        let success_body = axum::body::to_bytes(success.into_body(), 4096)
            .await
            .unwrap();
        let success_html = String::from_utf8(success_body.to_vec()).unwrap();
        assert!(success_html.contains("history.replaceState(null,\"\",location.pathname)"));
        assert!(success_html.contains("window.close()"));

        let failure = oauth_callback_page(
            StatusCode::BAD_REQUEST,
            "Account connection cancelled",
            "Account connection was cancelled.",
            false,
        );
        let failure_body = axum::body::to_bytes(failure.into_body(), 4096)
            .await
            .unwrap();
        let failure_html = String::from_utf8(failure_body.to_vec()).unwrap();
        assert!(failure_html.contains("history.replaceState(null,\"\",location.pathname)"));
        assert!(!failure_html.contains("window.close()"));
    }

    #[test]
    fn oauth_redirect_uris_use_provider_native_loopback_forms() {
        let google = oauth_callback_uri(Provider::Google, "127.0.0.1:41783", 41783)
            .expect("Google desktop loopback URI");
        assert_eq!(
            google.as_str(),
            "http://127.0.0.1:41783/mail-calendar/oauth/callback"
        );

        let microsoft = oauth_callback_uri(Provider::Microsoft, "127.0.0.1:41783", 41783)
            .expect("Microsoft desktop loopback URI");
        assert_eq!(
            microsoft.as_str(),
            "http://localhost:41783/mail-calendar/oauth/callback"
        );

        assert_eq!(
            oauth_callback_uri(Provider::Google, "attacker.example", 41783),
            Err(axum::http::StatusCode::FORBIDDEN)
        );
        assert_eq!(
            oauth_callback_uri(Provider::AppleIcloud, "127.0.0.1:41783", 41783),
            Err(axum::http::StatusCode::BAD_REQUEST)
        );
    }

    #[test]
    fn accepts_icloud_style_addresses_and_rejects_malformed_authorities() {
        assert!(valid_icloud_email("owner+vak@icloud.com"));
        assert!(valid_icloud_email("name@me.com"));
        assert!(!valid_icloud_email("name@@icloud.com"));
        assert!(!valid_icloud_email("name@-icloud.com"));
        assert!(!valid_icloud_email("name@icloud..com"));
        assert!(!valid_icloud_email("nämé@icloud.com"));
    }

    #[test]
    fn local_account_setup_requires_loopback_peer_even_with_spoofed_host() {
        let mut headers = HeaderMap::new();
        headers.insert(axum::http::header::HOST, "127.0.0.1:43127".parse().unwrap());
        let remote_peer = "192.0.2.8:54321".parse().unwrap();
        let local_peer = "127.0.0.1:54321".parse().unwrap();
        assert!(!is_loopback_request(&headers, remote_peer));
        assert!(is_loopback_request(&headers, local_peer));

        headers.insert(axum::http::header::HOST, "localhost:43127".parse().unwrap());
        assert!(is_loopback_request(&headers, local_peer));

        headers.insert(axum::http::header::HOST, "example.test".parse().unwrap());
        assert!(!is_loopback_request(&headers, local_peer));

        headers.insert(axum::http::header::HOST, "127.0.0.1:43127".parse().unwrap());
        headers.insert("x-forwarded-for", "198.51.100.7".parse().unwrap());
        assert!(!is_loopback_request(&headers, local_peer));
        headers.remove("x-forwarded-for");
        headers.insert("forwarded", "for=198.51.100.7".parse().unwrap());
        assert!(!is_loopback_request(&headers, local_peer));
    }

    #[test]
    fn oauth_callback_cookie_requires_one_bounded_random_value() {
        let mut headers = HeaderMap::new();
        let binding = "x".repeat(43);
        headers.insert(
            header::COOKIE,
            HeaderValue::from_str(&format!(
                "session=unrelated; {OAUTH_CALLBACK_COOKIE}={binding}"
            ))
            .unwrap(),
        );
        assert_eq!(oauth_callback_cookie(&headers), Some(binding.as_str()));

        headers.insert(
            header::COOKIE,
            HeaderValue::from_str(&format!(
                "{OAUTH_CALLBACK_COOKIE}={binding}; {OAUTH_CALLBACK_COOKIE}={binding}"
            ))
            .unwrap(),
        );
        assert_eq!(oauth_callback_cookie(&headers), None);

        headers.insert(
            header::COOKIE,
            HeaderValue::from_str(&format!("{OAUTH_CALLBACK_COOKIE}=short")).unwrap(),
        );
        assert_eq!(oauth_callback_cookie(&headers), None);
    }

    #[test]
    fn oauth_callback_cookie_is_short_lived_path_scoped_and_lax() {
        let cookie = oauth_callback_set_cookie(&"x".repeat(43), false);
        assert!(cookie.contains("HttpOnly"));
        assert!(cookie.contains("SameSite=Lax"));
        assert!(cookie.contains("Path=/mail-calendar;"));
        assert!(cookie.contains("Max-Age=600"));
        assert!(!cookie.contains("; Secure"));
        assert!(!cookie.contains("Domain="));

        let secure_cookie = oauth_callback_set_cookie(&"x".repeat(43), true);
        assert!(secure_cookie.contains("; Secure"));
    }

    #[test]
    fn account_linking_requires_an_active_agent_at_callback_time() {
        vak_config::paths::isolate_home_for_tests();
        let dir = tempfile::tempdir().unwrap();
        let core = vak_core::Core::new(dir.path().to_path_buf()).unwrap();
        let state = crate::AppState::new(core.clone());
        let mut agent = crate::agents::find_template("writer")
            .unwrap()
            .to_agent_definition("mail-owner", None);

        crate::agents::save(core.cwd(), std::slice::from_ref(&agent), true).unwrap();
        assert!(valid_agent(&state, "mail-owner"));

        agent.lifecycle = crate::agents::AgentLifecycle::Paused;
        crate::agents::save(core.cwd(), std::slice::from_ref(&agent), true).unwrap();
        assert!(!valid_agent(&state, "mail-owner"));
        assert!(registered_agent(&state, "mail-owner"));

        agent.lifecycle = crate::agents::AgentLifecycle::Archived;
        crate::agents::save(core.cwd(), std::slice::from_ref(&agent), true).unwrap();
        assert!(!valid_agent(&state, "mail-owner"));
        assert!(registered_agent(&state, "mail-owner"));
    }

    #[tokio::test]
    async fn same_account_operations_share_a_serialization_lock() {
        let state = crate::test_support::state();
        let first = state.mail_calendar_account_lock("agent-a", "account-1");
        let cloned_state = state.clone();
        let same_account = cloned_state.mail_calendar_account_lock("agent-a", "account-1");
        let other_account = state.mail_calendar_account_lock("agent-a", "account-2");
        let other_agent = state.mail_calendar_account_lock("agent-b", "account-1");
        let provider = state.mail_calendar_provider_lock("agent-a", Provider::Google);
        let same_provider = cloned_state.mail_calendar_provider_lock("agent-a", Provider::Google);
        let other_provider = state.mail_calendar_provider_lock("agent-a", Provider::Microsoft);
        let other_provider_agent = state.mail_calendar_provider_lock("agent-b", Provider::Google);
        assert!(std::sync::Arc::ptr_eq(&first, &same_account));
        assert!(!std::sync::Arc::ptr_eq(&first, &other_account));
        assert!(!std::sync::Arc::ptr_eq(&first, &other_agent));
        assert!(std::sync::Arc::ptr_eq(&provider, &same_provider));
        assert!(!std::sync::Arc::ptr_eq(&provider, &other_provider));
        assert!(!std::sync::Arc::ptr_eq(&provider, &other_provider_agent));

        let guard = first.lock().await;
        let (started_tx, started_rx) = tokio::sync::oneshot::channel();
        let (acquired_tx, mut acquired_rx) = tokio::sync::oneshot::channel();
        tokio::spawn(async move {
            let _ = started_tx.send(());
            let _guard = same_account.lock().await;
            let _ = acquired_tx.send(());
        });
        started_rx.await.unwrap();
        assert!(
            tokio::time::timeout(std::time::Duration::from_millis(10), &mut acquired_rx)
                .await
                .is_err()
        );

        drop(guard);
        tokio::time::timeout(std::time::Duration::from_secs(1), acquired_rx)
            .await
            .unwrap()
            .unwrap();
    }

    #[test]
    fn idle_account_operation_locks_are_reclaimed_after_account_churn() {
        let state = crate::test_support::state();
        for index in 0..128 {
            let account_id = format!("account-{index}");
            drop(state.mail_calendar_account_lock("agent-a", &account_id));
        }

        // Acquiring the next lock prunes every prior weak entry whose last
        // operation has finished. A long-lived server therefore does not
        // retain one map entry per account it has ever seen.
        drop(state.mail_calendar_account_lock("agent-a", "account-final"));
        let locks = state
            .mail_calendar_account_locks
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        assert_eq!(locks.len(), 1);
    }

    #[tokio::test]
    async fn account_link_admission_rechecks_agent_after_provider_lock_wait() {
        vak_config::paths::isolate_home_for_tests();
        let dir = tempfile::tempdir().unwrap();
        let core = vak_core::Core::new(dir.path().to_path_buf()).unwrap();
        let state = crate::AppState::new(core.clone());
        let mut agent = crate::agents::find_template("writer")
            .unwrap()
            .to_agent_definition("mail-lock-owner", None);
        crate::agents::save(core.cwd(), std::slice::from_ref(&agent), true).unwrap();

        let provider_lock = state.mail_calendar_provider_lock("mail-lock-owner", Provider::Google);
        let blocker = provider_lock.lock_owned().await;
        let waiting_state = state.clone();
        let waiting = tokio::spawn(async move {
            super::active_provider_link_guard(&waiting_state, "mail-lock-owner", Provider::Google)
                .await
                .is_some()
        });
        tokio::task::yield_now().await;

        agent.lifecycle = crate::agents::AgentLifecycle::Paused;
        crate::agents::save(core.cwd(), std::slice::from_ref(&agent), true).unwrap();
        drop(blocker);
        assert!(
            !tokio::time::timeout(std::time::Duration::from_secs(1), waiting)
                .await
                .unwrap()
                .unwrap()
        );
    }
}

#[derive(Deserialize)]
pub(super) struct CallbackQuery {
    state: String,
    code: Option<String>,
    error: Option<String>,
}

impl Drop for CallbackQuery {
    fn drop(&mut self) {
        self.state.zeroize();
        self.code.zeroize();
        self.error.zeroize();
    }
}

pub(super) async fn oauth_callback(
    State(state): State<AppState>,
    ConnectInfo(peer): ConnectInfo<SocketAddr>,
    headers: HeaderMap,
    Query(query): Query<CallbackQuery>,
) -> Response {
    if !is_loopback_request(&headers, peer) {
        return oauth_callback_page(
            StatusCode::FORBIDDEN,
            "Local connection required",
            "Account connections can only return directly to the local Vakyartha service.",
            false,
        );
    }
    let session =
        crate::web::session_cookie(&headers).filter(|token| state.browser_sessions.valid(token));
    let callback_binding = oauth_callback_cookie(&headers);
    let Some(grant) = state
        .mail_calendar_oauth
        .consume(&query.state, session, callback_binding)
    else {
        return oauth_callback_page(
            StatusCode::BAD_REQUEST,
            "Account connection expired",
            "This account connection expired or was started in another browser session. Start again from Settings.",
            false,
        );
    };
    if grant
        .initiating_session_id()
        .is_some_and(|session| !state.browser_sessions.valid(session))
    {
        return oauth_callback_page(
            StatusCode::BAD_REQUEST,
            "Sign in again",
            "The browser session that started this account connection has expired. Start again from Settings.",
            false,
        );
    }
    if !valid_agent(&state, &grant.agent_id) {
        return oauth_callback_page(
            StatusCode::NOT_FOUND,
            "Agent unavailable",
            "The Agent that started this account connection is no longer active. Start again from an active Agent's Settings.",
            false,
        );
    }
    if query.error.is_some() {
        return oauth_callback_page(
            StatusCode::BAD_REQUEST,
            "Account connection cancelled",
            "Account connection was cancelled by the provider. You can close this window and return to Settings.",
            false,
        );
    }
    let Some(code) = query.code.as_deref() else {
        return oauth_callback_page(
            StatusCode::BAD_REQUEST,
            "Account connection incomplete",
            "The provider returned an incomplete account connection. Start again from Settings.",
            false,
        );
    };
    if !state.mail_calendar_oauth.is_current(&grant) {
        return oauth_callback_page(
            StatusCode::CONFLICT,
            "Account connection cancelled",
            "This account connection was cancelled. Start again from Settings if you still want to connect it.",
            false,
        );
    }
    let code = Zeroizing::new(code.to_owned());
    let agent_id = grant.agent_id.clone();
    let account_id = grant.account_id.clone();
    let redeemed = match grant.redeem(&code).await {
        Ok(redeemed) => redeemed,
        Err(_) => {
            return oauth_callback_page(
                StatusCode::BAD_GATEWAY,
                "Account connection failed",
                "The provider could not complete account linking. Start again from Settings.",
                false,
            );
        }
    };
    // Redemption crosses the network. Recheck the owner Agent and initiating
    // browser session before making the returned credentials durable.
    if !valid_agent(&state, &agent_id) {
        return oauth_callback_page(
            StatusCode::NOT_FOUND,
            "Agent unavailable",
            "The Agent that started this account connection is no longer active. Start again from an active Agent's Settings.",
            false,
        );
    }
    if !state.mail_calendar_oauth.is_current(&grant) {
        return oauth_callback_page(
            StatusCode::CONFLICT,
            "Account connection cancelled",
            "This account connection was cancelled while sign-in was finishing. Start again from Settings if you still want to connect it.",
            false,
        );
    }
    if grant
        .initiating_session_id()
        .is_some_and(|session| !state.browser_sessions.valid(session))
    {
        return oauth_callback_page(
            StatusCode::BAD_REQUEST,
            "Sign in again",
            "The browser session that started this account connection has expired. Start again from Settings.",
            false,
        );
    }
    let vault = match AccountVault::for_agent(&agent_id) {
        Ok(vault) => vault,
        Err(_) => {
            return oauth_callback_page(
                StatusCode::INTERNAL_SERVER_ERROR,
                "Account connection failed",
                "Vakyartha could not save the account securely. Start again from Settings.",
                false,
            );
        }
    };
    let ledger = match ConnectionLedger::for_agent(&agent_id) {
        Ok(ledger) => ledger,
        Err(_) => {
            return oauth_callback_page(
                StatusCode::INTERNAL_SERVER_ERROR,
                "Account connection failed",
                "Vakyartha could not save the account. Start again from Settings.",
                false,
            );
        }
    };
    let Some(_provider_guard) = active_provider_link_guard(&state, &agent_id, grant.provider).await
    else {
        return oauth_callback_page(
            StatusCode::NOT_FOUND,
            "Agent unavailable",
            "The Agent that started this account connection is no longer active. Start again from an active Agent's Settings.",
            false,
        );
    };
    if !state.mail_calendar_oauth.is_current(&grant) {
        return oauth_callback_page(
            StatusCode::CONFLICT,
            "Account connection cancelled",
            "This account connection was cancelled while sign-in was finishing. Start again from Settings if you still want to connect it.",
            false,
        );
    }
    if grant
        .initiating_session_id()
        .is_some_and(|session| !state.browser_sessions.valid(session))
    {
        return oauth_callback_page(
            StatusCode::BAD_REQUEST,
            "Sign in again",
            "The browser session that started this account connection has expired. Start again from Settings.",
            false,
        );
    }
    let persisted = if redeemed.account_id != account_id {
        None
    } else {
        state
            .mail_calendar_oauth
            .with_current(&grant, || redeemed.persist(&vault, &ledger))
    };
    match persisted {
        None => {
            return oauth_callback_page(
                StatusCode::CONFLICT,
                "Account connection cancelled",
                "This account connection was cancelled while sign-in was finishing. Start again from Settings if you still want to connect it.",
                false,
            );
        }
        Some(Err(vak_mail_calendar::oauth::OAuthExchangeError::AccountAlreadyConnected)) => {
            return oauth_callback_page(
                StatusCode::CONFLICT,
                "Account already connected",
                "This provider account is already connected to this Agent. Disconnect it before changing its access selection.",
                false,
            );
        }
        Some(Err(vak_mail_calendar::oauth::OAuthExchangeError::AccountLinkInProgress)) => {
            return oauth_callback_page(
                StatusCode::CONFLICT,
                "Account connection in progress",
                "Another account connection for this provider is still pending. Finish its cleanup before trying again.",
                false,
            );
        }
        Some(Err(_)) => {
            return oauth_callback_page(
                StatusCode::INTERNAL_SERVER_ERROR,
                "Account connection failed",
                "Vakyartha could not save the account. Start again from Settings.",
                false,
            );
        }
        Some(Ok(account)) => record_account_event(
            &state,
            "account_connected",
            &account.owner_agent_id,
            &account.id,
            account.provider,
            &account.capabilities,
            "connected",
        ),
    }
    oauth_callback_page(
        StatusCode::OK,
        "Account connected",
        "You can close this window and return to Settings.",
        true,
    )
}

/// Callback query values include a short-lived provider code. Keep result
/// pages entirely static and remove the query from browser history immediately
/// after rendering so code and state are not left visible in the address bar.
fn oauth_callback_page(
    status: StatusCode,
    title: &'static str,
    message: &'static str,
    close_window: bool,
) -> Response {
    let close_script = if close_window { "window.close();" } else { "" };
    let html = format!(
        "<!doctype html><html lang=\"en\"><head><meta charset=\"utf-8\"><meta name=\"referrer\" content=\"no-referrer\"><title>{title}</title><script>history.replaceState(null,\"\",location.pathname);{close_script}</script></head><body><main><h1>{title}</h1><p>{message}</p></main></body></html>"
    );
    (status, Html(html)).into_response()
}

fn oauth_callback_cookie(headers: &HeaderMap) -> Option<&str> {
    let cookie_header = headers.get(header::COOKIE)?.to_str().ok()?;
    let mut matches = cookie_header.split(';').filter_map(|pair| {
        let (name, value) = pair.trim().split_once('=')?;
        (name == OAUTH_CALLBACK_COOKIE).then_some(value.trim())
    });
    let value = matches.next()?;
    if matches.next().is_some()
        || value.len() != 43
        || !value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-' || byte == b'_')
    {
        return None;
    }
    Some(value)
}

fn oauth_callback_set_cookie(binding: &str, secure: bool) -> String {
    let secure_attribute = if secure { "; Secure" } else { "" };
    format!(
        "{OAUTH_CALLBACK_COOKIE}={binding}; HttpOnly; SameSite=Lax; Path=/mail-calendar; Max-Age={OAUTH_CALLBACK_COOKIE_MAX_AGE_SECONDS}{secure_attribute}"
    )
}

pub(super) async fn refresh_account(
    State(state): State<AppState>,
    axum::Extension(principal): axum::Extension<AuthenticatedPrincipal>,
    Path((agent_id, account_id)): Path<(String, String)>,
) -> Response {
    if !operator(&principal) {
        return StatusCode::FORBIDDEN.into_response();
    }
    if !valid_agent(&state, &agent_id) {
        return StatusCode::NOT_FOUND.into_response();
    }
    let operation_lock = state.mail_calendar_account_lock(&agent_id, &account_id);
    let _operation_guard = operation_lock.lock().await;
    if !valid_agent(&state, &agent_id) {
        return StatusCode::NOT_FOUND.into_response();
    }
    let Ok(ledger) = ConnectionLedger::for_agent(&agent_id) else {
        return StatusCode::BAD_REQUEST.into_response();
    };
    let account = match ledger.read_all() {
        Ok(accounts) => accounts.into_iter().find(|account| {
            account.id == account_id
                && account.status == AccountStatus::Connected
                && account.revoked_at.is_none()
        }),
        Err(_) => return StatusCode::SERVICE_UNAVAILABLE.into_response(),
    };
    let Some(account) = account else {
        return StatusCode::NOT_FOUND.into_response();
    };
    if account.provider == Provider::AppleIcloud {
        return (
            StatusCode::CONFLICT,
            "This account uses an app-specific password and does not support token refresh.",
        )
            .into_response();
    }
    let Ok(vault) = AccountVault::for_agent(&agent_id) else {
        return StatusCode::BAD_REQUEST.into_response();
    };
    let rotated_tokens =
        match vak_mail_calendar::oauth::refresh_account_tokens(&vault, &account).await {
            Ok(tokens) => tokens,
            Err(vak_mail_calendar::oauth::OAuthRefreshError::ReconnectRequired) => {
                if ledger
                    .append_reauthentication_required(&account_id, Utc::now())
                    .is_err()
                {
                    return StatusCode::SERVICE_UNAVAILABLE.into_response();
                }
                record_account_event(
                    &state,
                    "account_reauthentication_required",
                    &agent_id,
                    &account_id,
                    account.provider,
                    &account.capabilities,
                    "reauthentication_required",
                );
                return (
                    StatusCode::CONFLICT,
                    "This account needs to be connected again.",
                )
                    .into_response();
            }
            Err(vak_mail_calendar::oauth::OAuthRefreshError::Vault(_)) => {
                return StatusCode::SERVICE_UNAVAILABLE.into_response();
            }
            Err(_) => {
                return (
                    StatusCode::BAD_GATEWAY,
                    "The provider could not refresh this account.",
                )
                    .into_response();
            }
        };
    // The provider request may outlive an Agent pause/archive. Keep rotated
    // tokens in zeroizing memory until the Agent is rechecked at the final
    // credential commit, then discard them if the Agent is no longer active.
    if !valid_agent(&state, &agent_id) {
        return StatusCode::NOT_FOUND.into_response();
    }
    let expires_at = rotated_tokens.expires_at();
    let Some(revision) = account.revision.checked_add(1) else {
        return StatusCode::CONFLICT.into_response();
    };
    let mut refreshed = account.clone();
    refreshed.revision = revision;
    refreshed.access_token_expires_at = Some(expires_at);
    let commit = ledger.append_connected_if_current(refreshed.clone(), account.revision, || {
        if !valid_agent(&state, &agent_id) {
            return Err(RefreshCommitError::AgentInactive);
        }
        rotated_tokens
            .persist(&vault, &account)
            .map(|_| ())
            .map_err(RefreshCommitError::Credential)
    });
    match commit {
        Ok(()) => {}
        Err(vak_mail_calendar::connection_ledger::ConditionalUpdateError::Conflict) => {
            return (
                StatusCode::CONFLICT,
                "This account changed while its sign-in was refreshing. Retry from Settings.",
            )
                .into_response();
        }
        Err(vak_mail_calendar::connection_ledger::ConditionalUpdateError::Ledger(_)) => {
            return StatusCode::SERVICE_UNAVAILABLE.into_response();
        }
        Err(vak_mail_calendar::connection_ledger::ConditionalUpdateError::Operation(
            RefreshCommitError::AgentInactive,
        )) => return StatusCode::NOT_FOUND.into_response(),
        Err(vak_mail_calendar::connection_ledger::ConditionalUpdateError::Operation(
            RefreshCommitError::Credential(
                vak_mail_calendar::oauth::OAuthRefreshError::ReconnectRequired,
            ),
        )) => {
            return (
                StatusCode::CONFLICT,
                "This account needs to be connected again.",
            )
                .into_response();
        }
        Err(vak_mail_calendar::connection_ledger::ConditionalUpdateError::Operation(
            RefreshCommitError::Credential(vak_mail_calendar::oauth::OAuthRefreshError::Vault(_)),
        )) => return StatusCode::SERVICE_UNAVAILABLE.into_response(),
        Err(vak_mail_calendar::connection_ledger::ConditionalUpdateError::Operation(_)) => {
            return (
                StatusCode::BAD_GATEWAY,
                "The provider could not refresh this account.",
            )
                .into_response();
        }
    }
    record_account_event(
        &state,
        "account_refreshed",
        &agent_id,
        &account_id,
        refreshed.provider,
        &refreshed.capabilities,
        "refreshed",
    );
    let identity_masked = vault
        .load(&refreshed.id)
        .ok()
        .and_then(|secret| secret.masked_display_identity());
    Json(serde_json::json!({
        "account": AccountView {
            id: refreshed.id,
            provider: refreshed.provider,
            status: refreshed.status,
            identity_masked,
            credential_available: true,
            capabilities: refreshed.capabilities.into_iter().collect(),
            connected_at: refreshed.connected_at,
            access_token_expires_at: refreshed.access_token_expires_at,
            refresh_token_available: refreshed.refresh_token_available,
            revoked_at: refreshed.revoked_at,
        }
    }))
    .into_response()
}

pub(super) async fn disconnect_account(
    State(state): State<AppState>,
    axum::Extension(principal): axum::Extension<AuthenticatedPrincipal>,
    Path((agent_id, account_id)): Path<(String, String)>,
) -> Response {
    if !operator(&principal) {
        return StatusCode::FORBIDDEN.into_response();
    }
    if !registered_agent(&state, &agent_id) {
        return StatusCode::NOT_FOUND.into_response();
    }
    let Ok(ledger) = ConnectionLedger::for_agent(&agent_id) else {
        return StatusCode::BAD_REQUEST.into_response();
    };
    let account = match ledger.read_all() {
        Ok(accounts) => accounts
            .into_iter()
            .find(|account| account.id == account_id),
        Err(_) => return StatusCode::SERVICE_UNAVAILABLE.into_response(),
    };
    let Some(account) = account else {
        return StatusCode::NOT_FOUND.into_response();
    };
    // This lock orders new links behind disconnect completion. The account
    // lock then serializes refresh/disconnect for this specific credential.
    let provider_lock = state.mail_calendar_provider_lock(&agent_id, account.provider);
    let _provider_guard = provider_lock.lock().await;
    let operation_lock = state.mail_calendar_account_lock(&agent_id, &account_id);
    let _operation_guard = operation_lock.lock().await;
    if !registered_agent(&state, &agent_id) {
        return StatusCode::NOT_FOUND.into_response();
    }
    let Ok(vault) = AccountVault::for_agent(&agent_id) else {
        return StatusCode::BAD_REQUEST.into_response();
    };
    let account = match ledger.read_all() {
        Ok(accounts) => accounts
            .into_iter()
            .find(|account| account.id == account_id),
        Err(_) => return StatusCode::SERVICE_UNAVAILABLE.into_response(),
    };
    let Some(account) = account else {
        return StatusCode::NOT_FOUND.into_response();
    };
    // Serialize disconnect with a callback's final vault/ledger commit before
    // writing the tombstone. The callback either commits first or observes a
    // stale fence and refuses to persist credentials.
    state
        .mail_calendar_oauth
        .cancel_provider(&agent_id, account.provider);
    let mut already_disconnected = account.revoked_at.is_some();
    if !already_disconnected {
        if ledger
            .append_revoked(&account_id, chrono::Utc::now())
            .is_err()
        {
            // A concurrent disconnect may have written the tombstone after
            // our read. Accept that state so this request can finish cleanup.
            let already_fenced = ledger.read_all().is_ok_and(|accounts| {
                accounts
                    .iter()
                    .any(|saved| saved.id == account_id && saved.revoked_at.is_some())
            });
            if !already_fenced {
                return StatusCode::SERVICE_UNAVAILABLE.into_response();
            }
            already_disconnected = true;
        }
    }
    // The ledger tombstone fences local use before any provider request. A
    // retry of an existing tombstone finishes local cleanup without attempting
    // provider revocation again; its prior outcome is reported as unconfirmed.
    // Microsoft does not provide a narrow per-application revoker.
    let provider_grant_revoked = if already_disconnected {
        false
    } else {
        vak_mail_calendar::oauth::revoke_provider_grant(&vault, &account).await
    };
    let provider_revocation = if already_disconnected {
        "not_retried"
    } else if provider_grant_revoked {
        "confirmed"
    } else if account.provider == Provider::Google {
        "unconfirmed"
    } else {
        "unsupported"
    };
    if vault.remove(&account_id).is_err() {
        record_account_event(
            &state,
            "account_disconnect_cleanup_pending",
            &agent_id,
            &account_id,
            account.provider,
            &account.capabilities,
            "credential_removal_failed",
        );
        return StatusCode::SERVICE_UNAVAILABLE.into_response();
    }
    record_account_event(
        &state,
        "account_disconnected",
        &agent_id,
        &account_id,
        account.provider,
        &account.capabilities,
        provider_revocation,
    );
    Json(serde_json::json!({ "disconnected": true, "already_disconnected": already_disconnected, "provider_grant_revoked": provider_grant_revoked, "provider_revocation": provider_revocation, "content_erased": false })).into_response()
}

fn operator(principal: &AuthenticatedPrincipal) -> bool {
    matches!(principal, AuthenticatedPrincipal::Operator)
}

fn valid_agent(state: &AppState, agent_id: &str) -> bool {
    registered_agent(state, agent_id)
        && (agent_id == "vak"
            || agents::effective(&state.active_core()).is_ok_and(|agents| {
                agents
                    .iter()
                    .any(|agent| agent.id == agent_id && agent.is_admissible())
            }))
}

/// Account inventory and credential cleanup remain available to the owner
/// while an Agent is paused or archived. Creating or refreshing a connection
/// uses `valid_agent` and still requires an active Agent.
fn registered_agent(state: &AppState, agent_id: &str) -> bool {
    if agent_id == "vak" {
        return true;
    }
    agents::effective(&state.active_core())
        .is_ok_and(|agents| agents.iter().any(|agent| agent.id == agent_id))
}

/// Hold the Agent/provider admission lock and recheck lifecycle after waiting.
/// Account linking is allowed only for an active Agent, even if the Agent was
/// paused or archived while this request contended with disconnect cleanup.
async fn active_provider_link_guard(
    state: &AppState,
    agent_id: &str,
    provider: Provider,
) -> Option<tokio::sync::OwnedMutexGuard<()>> {
    let provider_lock = state.mail_calendar_provider_lock(agent_id, provider);
    let guard = provider_lock.lock_owned().await;
    valid_agent(state, agent_id).then_some(guard)
}
