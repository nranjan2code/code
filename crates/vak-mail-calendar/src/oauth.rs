//! Bounded OAuth 2.0 authorization-code + PKCE state for Google and Microsoft.
//!
//! Attempts live in memory for ten minutes, are bound to one Agent and a
//! reviewed capability/audience set, and are consumed once. Provider URLs are
//! fixed constants; arbitrary authorization or token hosts are not accepted.
//! The initial redirect contract is loopback only for installed/public
//! clients. A web callback needs a separately configured confidential-client
//! contract and must never come from a browser request.
//! Apple iCloud is intentionally excluded because its supported setup uses an
//! app-specific password rather than OAuth.

use crate::vault::{AccountSecretMaterial, AccountVault};
use crate::{
    AccountStatus, AgentId, AudienceId, Capability, ConnectedAccount, Provider,
    connection_ledger::ConnectionLedger,
};
use base64::Engine as _;
use chrono::{DateTime, Utc};
use ring::rand::{SecureRandom, SystemRandom};
use ring::signature::{RSA_PKCS1_2048_8192_SHA256, RsaPublicKeyComponents};
use serde::Deserialize;
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;
use std::sync::Mutex;
use std::time::{Duration, Instant};
use subtle::ConstantTimeEq;
use url::Url;
use zeroize::{Zeroize, Zeroizing};

const ATTEMPT_LIFETIME: Duration = Duration::from_secs(10 * 60);
const MAX_PENDING_ATTEMPTS: usize = 128;
const MAX_PENDING_PER_AGENT: usize = 4;
const MAX_SCOPES: usize = 16;
const MAX_AUTHORIZATION_FENCES: usize = 4096;

#[derive(Debug, thiserror::Error)]
pub enum OAuthError {
    #[error("provider authorization request is invalid")]
    InvalidRequest,
    #[error("provider authorization is not supported for this account type")]
    UnsupportedProvider,
    #[error("provider authorization could not create secure random state")]
    RandomUnavailable,
    #[error("provider authorization is temporarily full; retry shortly")]
    AtCapacity,
}

#[derive(Debug, thiserror::Error)]
pub enum OAuthExchangeError {
    #[error("provider authorization exchange failed")]
    Exchange,
    #[error("provider returned an invalid authorization response")]
    InvalidResponse,
    #[error("the provider did not grant all requested capabilities")]
    InsufficientScopes,
    #[error("the Agent vault and connection ledger could not be updated")]
    Persistence,
    #[error("connected account does not match the Agent vault")]
    AgentMismatch,
    #[error("this provider identity is already connected to the Agent")]
    AccountAlreadyConnected,
    #[error("another account connection for this provider is still in progress")]
    AccountLinkInProgress,
    #[error("provider authorization setup is unavailable")]
    Setup(#[from] OAuthError),
}

#[derive(Debug, thiserror::Error)]
pub enum OAuthRefreshError {
    #[error("provider account must be reconnected")]
    ReconnectRequired,
    #[error("provider token refresh failed")]
    ProviderUnavailable,
    #[error("provider returned invalid refreshed authorization")]
    InvalidResponse,
    #[error("refreshed authorization did not include the connected capabilities")]
    InsufficientScopes,
    #[error("updated credentials could not be saved to the Agent vault")]
    Vault(#[from] crate::vault::VaultError),
}

/// Refreshed token material held only until the server rechecks Agent state.
/// The server must call `persist` only after that admission check succeeds;
/// dropping this value zeroizes both tokens.
pub struct RefreshedOAuthTokens {
    expires_at: DateTime<Utc>,
    access_token: Zeroizing<String>,
    refresh_token: Option<Zeroizing<String>>,
}

impl RefreshedOAuthTokens {
    pub fn expires_at(&self) -> DateTime<Utc> {
        self.expires_at
    }

    pub fn persist(
        self,
        vault: &AccountVault,
        account: &ConnectedAccount,
    ) -> Result<DateTime<Utc>, OAuthRefreshError> {
        if vault.agent_id() != account.owner_agent_id
            || account.status != AccountStatus::Connected
            || account.revoked_at.is_some()
        {
            return Err(OAuthRefreshError::ReconnectRequired);
        }
        vault.rotate_oauth_tokens(&account.id, self.access_token, self.refresh_token)?;
        Ok(self.expires_at)
    }
}

pub struct RedeemedAuthorization {
    pub account_id: String,
    pub provider: Provider,
    pub agent_id: AgentId,
    pub audiences: Vec<AudienceId>,
    pub capabilities: Vec<Capability>,
    pub granted_scopes: Vec<String>,
    pub access_token_expires_at: DateTime<Utc>,
    pub refresh_token_available: bool,
    secret_material: AccountSecretMaterial,
}

impl RedeemedAuthorization {
    /// Record a pending link before writing its secret, then activate it only
    /// after both durable writes succeed. A crash leaves an owner-visible row
    /// that can be cleaned up instead of an unlisted vault credential.
    pub fn persist(
        self,
        vault: &AccountVault,
        ledger: &ConnectionLedger,
    ) -> Result<ConnectedAccount, OAuthExchangeError> {
        if vault.agent_id() != self.agent_id {
            return Err(OAuthExchangeError::AgentMismatch);
        }
        let credential_ref = AccountVault::credential_ref(&self.account_id)
            .map_err(|_| OAuthExchangeError::Persistence)?;
        let mut account = ConnectedAccount {
            id: self.account_id,
            provider: self.provider,
            status: AccountStatus::Pending,
            owner_agent_id: self.agent_id,
            allowed_audiences: self.audiences.into_iter().collect(),
            capabilities: self.capabilities.into_iter().collect(),
            provider_scopes: self.granted_scopes.into_iter().collect(),
            credential_ref: credential_ref.clone(),
            principal_ref: credential_ref,
            revision: 1,
            connected_at: Utc::now(),
            access_token_expires_at: Some(self.access_token_expires_at),
            refresh_token_available: self.refresh_token_available,
            revoked_at: None,
        };
        ledger
            .append_pending_if(account.clone(), |existing| {
                for linked in existing.iter().filter(|linked| {
                    linked.provider == self.provider && linked.revoked_at.is_none()
                }) {
                    if linked.status == AccountStatus::Pending {
                        return Err(OAuthExchangeError::AccountLinkInProgress);
                    }
                    if !matches!(
                        linked.status,
                        AccountStatus::Connected | AccountStatus::ReauthenticationRequired
                    ) {
                        continue;
                    }
                    let stored = vault
                        .load(&linked.id)
                        .map_err(|_| OAuthExchangeError::Persistence)?;
                    if stored.has_same_principal(&self.secret_material) {
                        return Err(OAuthExchangeError::AccountAlreadyConnected);
                    }
                }
                Ok(())
            })
            .map_err(|error| match error {
                crate::connection_ledger::ConditionalAppendError::Ledger(_) => {
                    OAuthExchangeError::Persistence
                }
                crate::connection_ledger::ConditionalAppendError::Check(error) => error,
            })?;
        account.status = AccountStatus::Connected;
        account.revision = 2;
        let account_id = account.id.clone();
        if ledger
            .append_connected_if_pending(account.clone(), || {
                vault.store(&account_id, self.secret_material)
            })
            .is_err()
        {
            let _ = ledger.append_revoked(&account.id, Utc::now());
            let _ = vault.remove(&account.id);
            return Err(OAuthExchangeError::Persistence);
        }
        Ok(account)
    }
}

/// In-memory, bounded, single-use callback state. Losing this store on restart
/// invalidates outstanding authorizations, which fail closed and can be
/// restarted by the owner.
#[derive(Default)]
struct AuthorizationState {
    attempts: BTreeMap<String, PendingAuthorization>,
    fences: BTreeMap<(String, Provider), u128>,
}

#[derive(Default)]
pub struct AuthorizationStore {
    state: Mutex<AuthorizationState>,
}

struct PendingAuthorization {
    provider: Provider,
    account_id: String,
    agent_id: AgentId,
    initiating_session_id: Option<Zeroizing<String>>,
    callback_binding: Option<Zeroizing<String>>,
    audiences: Vec<AudienceId>,
    capabilities: Vec<Capability>,
    requested_scopes: Vec<String>,
    client_id: String,
    redirect_uri: String,
    verifier: Zeroizing<String>,
    nonce: Zeroizing<String>,
    state: Zeroizing<String>,
    fence: u128,
    expires_at: Instant,
}

pub struct AuthorizationGrant {
    pub provider: Provider,
    pub account_id: String,
    pub agent_id: AgentId,
    pub audiences: Vec<AudienceId>,
    pub capabilities: Vec<Capability>,
    requested_scopes: Vec<String>,
    pub client_id: String,
    pub redirect_uri: String,
    initiating_session_id: Option<Zeroizing<String>>,
    verifier: Zeroizing<String>,
    nonce: Zeroizing<String>,
    fence: u128,
}

impl AuthorizationGrant {
    /// The verifier must be sent only to the fixed provider token endpoint
    /// associated with `provider`; never return it to a browser or log it.
    pub fn code_verifier(&self) -> &str {
        &self.verifier
    }

    /// Compare against the signed OIDC `nonce` claim before accepting identity
    /// claims from the token response.
    pub fn oidc_nonce(&self) -> &str {
        &self.nonce
    }

    /// Recheck that the initiating browser session was not revoked while the
    /// person was on the provider's sign-in page.
    pub fn initiating_session_id(&self) -> Option<&str> {
        self.initiating_session_id.as_deref().map(String::as_str)
    }

    /// Exchanges an authorization code only with the fixed token endpoint for
    /// this provider. The returned identity is accepted only after PKCE, scope,
    /// OIDC signature, audience, issuer, expiry and nonce checks pass.
    pub async fn redeem(&self, code: &str) -> Result<RedeemedAuthorization, OAuthExchangeError> {
        redeem(self, code).await
    }
}

impl AuthorizationStore {
    #[allow(clippy::too_many_arguments)]
    pub fn begin(
        &self,
        provider: Provider,
        agent_id: &str,
        initiating_session_id: Option<&str>,
        audiences: &[AudienceId],
        capabilities: &[Capability],
        client_id: &str,
        redirect_uri: &str,
        existing_callback_binding: Option<&str>,
    ) -> Result<(String, String, String), OAuthError> {
        self.begin_inner(
            provider,
            agent_id,
            initiating_session_id,
            audiences,
            capabilities,
            client_id,
            redirect_uri,
            existing_callback_binding,
            false,
        )
    }

    /// Native installed clients use the RFC 8252 loopback + PKCE flow. The
    /// single-use 256-bit state binds the system-browser return to the
    /// owner-authenticated initiation; there is no shared webview cookie.
    pub fn begin_native(
        &self,
        provider: Provider,
        agent_id: &str,
        audiences: &[AudienceId],
        capabilities: &[Capability],
        client_id: &str,
        redirect_uri: &str,
    ) -> Result<(String, String), OAuthError> {
        let (url, account, _) = self.begin_inner(
            provider,
            agent_id,
            None,
            audiences,
            capabilities,
            client_id,
            redirect_uri,
            None,
            true,
        )?;
        Ok((url, account))
    }

    #[allow(clippy::too_many_arguments)]
    fn begin_inner(
        &self,
        provider: Provider,
        agent_id: &str,
        initiating_session_id: Option<&str>,
        audiences: &[AudienceId],
        capabilities: &[Capability],
        client_id: &str,
        redirect_uri: &str,
        existing_callback_binding: Option<&str>,
        native: bool,
    ) -> Result<(String, String, String), OAuthError> {
        if !valid_agent_id(agent_id)
            || (!native
                && initiating_session_id.is_none_or(|session| {
                    session.trim().is_empty()
                        || session.len() > 256
                        || session.chars().any(char::is_control)
                }))
            || (native && (initiating_session_id.is_some() || existing_callback_binding.is_some()))
            || audiences.is_empty()
            || audiences.len() > 64
            || audiences.iter().any(|value| !valid_scope_id(value))
            || capabilities.is_empty()
            || capabilities.len() > 6
            || client_id.trim().is_empty()
            || client_id.len() > 512
            || client_id.contains(char::is_whitespace)
        {
            return Err(OAuthError::InvalidRequest);
        }
        let redirect = Url::parse(redirect_uri).map_err(|_| OAuthError::InvalidRequest)?;
        if !valid_redirect(&redirect) {
            return Err(OAuthError::InvalidRequest);
        }
        let provider_scopes = scopes_for(provider, capabilities)?;
        let requested_scopes = provider_scopes
            .iter()
            .map(|scope| (*scope).to_owned())
            .collect::<Vec<_>>();
        let client_id = client_id.to_owned();
        let mut random_state = Zeroizing::new([0_u8; 32]);
        let mut random_verifier = Zeroizing::new([0_u8; 32]);
        let mut random_nonce = Zeroizing::new([0_u8; 32]);
        let mut random_callback_binding = Zeroizing::new([0_u8; 32]);
        let rng = SystemRandom::new();
        rng.fill(&mut random_state[..])
            .map_err(|_| OAuthError::RandomUnavailable)?;
        rng.fill(&mut random_verifier[..])
            .map_err(|_| OAuthError::RandomUnavailable)?;
        rng.fill(&mut random_nonce[..])
            .map_err(|_| OAuthError::RandomUnavailable)?;
        rng.fill(&mut random_callback_binding[..])
            .map_err(|_| OAuthError::RandomUnavailable)?;
        let state = Zeroizing::new(
            base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(&random_state[..]),
        );
        let verifier = Zeroizing::new(
            base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(&random_verifier[..]),
        );
        let nonce = Zeroizing::new(
            base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(&random_nonce[..]),
        );
        let generated_callback_binding = Zeroizing::new(
            base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(&random_callback_binding[..]),
        );
        let challenge = base64::engine::general_purpose::URL_SAFE_NO_PAD
            .encode(Sha256::digest(verifier.as_bytes()));
        let state_key = state_key(&state);
        let account_id = uuid::Uuid::now_v7().to_string();
        let auth_url = authorization_url(
            provider,
            &client_id,
            redirect.as_str(),
            &provider_scopes,
            &state,
            &challenge,
            &nonce,
        )?;

        let mut authorization_state = self
            .state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let fence = current_or_create_fence(&mut authorization_state, agent_id, provider);
        let attempts = &mut authorization_state.attempts;
        attempts.retain(|_, attempt| attempt.expires_at > Instant::now());
        let session_id = initiating_session_id;
        let reuse_callback_binding = existing_callback_binding.filter(|candidate| {
            valid_callback_binding(candidate)
                && attempts.values().any(|attempt| {
                    attempt.initiating_session_id.as_deref().map(String::as_str) == session_id
                        && attempt.callback_binding.as_ref().is_some_and(|binding| {
                            bool::from(binding.as_bytes().ct_eq(candidate.as_bytes()))
                        })
                })
        });
        if !native && reuse_callback_binding.is_none() {
            // One HttpOnly callback cookie is shared by this browser session's
            // concurrent pending flows. If it was lost or replaced, retire its
            // old attempts before issuing a fresh binding.
            attempts.retain(|_, attempt| {
                attempt.initiating_session_id.as_deref().map(String::as_str) != session_id
            });
        }
        let callback_binding = if native {
            None
        } else {
            Some(
                reuse_callback_binding
                    .map(str::to_owned)
                    .unwrap_or_else(|| generated_callback_binding.to_string()),
            )
        };
        if attempts.len() >= MAX_PENDING_ATTEMPTS
            || attempts
                .values()
                .filter(|attempt| attempt.agent_id == agent_id)
                .count()
                >= MAX_PENDING_PER_AGENT
        {
            return Err(OAuthError::AtCapacity);
        }
        attempts.insert(
            state_key,
            PendingAuthorization {
                provider,
                account_id: account_id.clone(),
                agent_id: agent_id.to_owned(),
                initiating_session_id: session_id.map(|id| Zeroizing::new(id.to_owned())),
                callback_binding: callback_binding.clone().map(Zeroizing::new),
                audiences: sorted_unique(audiences),
                capabilities: sorted_unique(capabilities),
                requested_scopes,
                client_id,
                redirect_uri: redirect.to_string(),
                verifier,
                nonce,
                state,
                fence,
                expires_at: Instant::now() + ATTEMPT_LIFETIME,
            },
        );
        Ok((auth_url, account_id, callback_binding.unwrap_or_default()))
    }

    /// Consumes callback state before token exchange. A callback replay,
    /// expired flow, process restart, or unknown state has no grant to redeem.
    pub fn consume(
        &self,
        returned_state: &str,
        authenticated_session_id: Option<&str>,
        callback_binding: Option<&str>,
    ) -> Option<AuthorizationGrant> {
        if returned_state.len() != 43
            || callback_binding.is_some_and(|value| !valid_callback_binding(value))
        {
            return None;
        }
        let key = state_key(returned_state);
        let mut authorization_state = self
            .state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let attempts = &mut authorization_state.attempts;
        let attempt = attempts.get(&key)?;
        if attempt.expires_at <= Instant::now()
            || !bool::from(attempt.state.as_bytes().ct_eq(returned_state.as_bytes()))
            || match (&attempt.callback_binding, callback_binding) {
                (Some(expected), Some(actual)) => {
                    !bool::from(expected.as_bytes().ct_eq(actual.as_bytes()))
                }
                (None, None) => false,
                _ => true,
            }
            || match (&attempt.initiating_session_id, authenticated_session_id) {
                (Some(expected), Some(actual)) => {
                    !bool::from(expected.as_bytes().ct_eq(actual.as_bytes()))
                }
                (Some(_), None) => false,
                (None, None) => false,
                (None, Some(_)) => true,
            }
        {
            return None;
        }
        let attempt = attempts.remove(&key)?;
        Some(AuthorizationGrant {
            provider: attempt.provider,
            account_id: attempt.account_id,
            agent_id: attempt.agent_id,
            audiences: attempt.audiences,
            capabilities: attempt.capabilities,
            requested_scopes: attempt.requested_scopes,
            client_id: attempt.client_id,
            redirect_uri: attempt.redirect_uri,
            initiating_session_id: attempt.initiating_session_id,
            verifier: attempt.verifier,
            nonce: attempt.nonce,
            fence: attempt.fence,
        })
    }

    /// Cancel every pending or in-flight account link for this Agent/provider.
    /// In-flight callbacks compare their captured fence before persisting.
    pub fn cancel_provider(&self, agent_id: &str, provider: Provider) {
        let mut authorization_state = self
            .state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        rotate_fence(&mut authorization_state, agent_id, provider);
        authorization_state
            .attempts
            .retain(|_, attempt| attempt.agent_id != agent_id || attempt.provider != provider);
    }

    /// Recheck authorization freshness after asynchronous callback work.
    pub fn is_current(&self, grant: &AuthorizationGrant) -> bool {
        let authorization_state = self
            .state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        authorization_state
            .fences
            .get(&(grant.agent_id.clone(), grant.provider))
            .is_some_and(|fence| *fence == grant.fence)
    }

    /// Run the final durable link commit while holding the same fence lock
    /// used by disconnect. Either the commit completes before cancellation,
    /// or cancellation wins and the operation is not called.
    pub fn with_current<T>(
        &self,
        grant: &AuthorizationGrant,
        operation: impl FnOnce() -> T,
    ) -> Option<T> {
        let authorization_state = self
            .state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        authorization_state
            .fences
            .get(&(grant.agent_id.clone(), grant.provider))
            .is_some_and(|fence| *fence == grant.fence)
            .then(operation)
    }
}

/// Generations exist only while the bounded authorization store remembers
/// them. Evicting a generation makes every grant that captured it stale.
/// That can require the owner to restart a rare concurrent authorization,
/// but it can never turn eviction into permission to commit.
fn current_or_create_fence(
    state: &mut AuthorizationState,
    agent_id: &str,
    provider: Provider,
) -> u128 {
    let key = (agent_id.to_owned(), provider);
    if let Some(fence) = state.fences.get(&key) {
        return *fence;
    }
    let fence = uuid::Uuid::now_v7().as_u128();
    bounded_fence_insert(state, key, fence);
    fence
}

fn rotate_fence(state: &mut AuthorizationState, agent_id: &str, provider: Provider) {
    bounded_fence_insert(
        state,
        (agent_id.to_owned(), provider),
        uuid::Uuid::now_v7().as_u128(),
    );
}

fn bounded_fence_insert(state: &mut AuthorizationState, key: (String, Provider), fence: u128) {
    if !state.fences.contains_key(&key) && state.fences.len() >= MAX_AUTHORIZATION_FENCES {
        state.fences.pop_first();
    }
    state.fences.insert(key, fence);
}

fn valid_callback_binding(value: &str) -> bool {
    value.len() == 43
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-' || byte == b'_')
}

fn authorization_url(
    provider: Provider,
    client_id: &str,
    redirect_uri: &str,
    scopes: &[&str],
    state: &str,
    challenge: &str,
    nonce: &str,
) -> Result<String, OAuthError> {
    let (endpoint, provider_scopes) = match provider {
        Provider::Google => (
            "https://accounts.google.com/o/oauth2/v2/auth",
            scopes.join(" "),
        ),
        Provider::Microsoft => (
            "https://login.microsoftonline.com/common/oauth2/v2.0/authorize",
            scopes.join(" "),
        ),
        Provider::AppleIcloud => return Err(OAuthError::UnsupportedProvider),
    };
    let mut url = Url::parse(endpoint).map_err(|_| OAuthError::InvalidRequest)?;
    {
        let mut query = url.query_pairs_mut();
        query
            .append_pair("client_id", client_id)
            .append_pair("redirect_uri", redirect_uri)
            .append_pair("response_type", "code")
            .append_pair("scope", &provider_scopes)
            .append_pair("state", state)
            .append_pair("nonce", nonce)
            .append_pair("code_challenge", challenge)
            .append_pair("code_challenge_method", "S256");
        if provider == Provider::Google {
            query
                .append_pair("access_type", "offline")
                // Prevent previously approved scopes from being unioned into
                // a token for this selected capability set.
                .append_pair("include_granted_scopes", "false");
        } else {
            query.append_pair("response_mode", "query");
        }
    }
    Ok(url.to_string())
}

pub(crate) fn scopes_for(
    provider: Provider,
    capabilities: &[Capability],
) -> Result<Vec<&'static str>, OAuthError> {
    // Stage 1 only links accounts for bounded reads. Do not let a direct API
    // caller acquire send or calendar-write grants before their reviewed
    // provider-effect path exists, even if the UI does not offer those boxes.
    if capabilities.is_empty()
        || capabilities.iter().any(|capability| {
            !matches!(
                capability,
                Capability::MailRead | Capability::CalendarFreeBusy | Capability::CalendarRead
            )
        })
    {
        return Err(OAuthError::InvalidRequest);
    }
    let mut scopes = Vec::new();
    match provider {
        Provider::Google => {
            scopes.extend(["openid", "email"]);
            for capability in capabilities {
                match capability {
                    Capability::MailRead => {
                        scopes.push("https://www.googleapis.com/auth/gmail.readonly")
                    }
                    Capability::MailSend => {
                        scopes.push("https://www.googleapis.com/auth/gmail.send")
                    }
                    Capability::CalendarFreeBusy => {
                        scopes.push("https://www.googleapis.com/auth/calendar.freebusy")
                    }
                    Capability::CalendarRead => {
                        scopes.push("https://www.googleapis.com/auth/calendar.readonly")
                    }
                    Capability::CalendarWrite => {
                        scopes.push("https://www.googleapis.com/auth/calendar.events")
                    }
                    Capability::MailPrepare => {}
                }
            }
        }
        Provider::Microsoft => {
            scopes.extend(["openid", "profile", "offline_access", "User.Read"]);
            for capability in capabilities {
                match capability {
                    Capability::MailRead => scopes.push("Mail.Read"),
                    Capability::MailSend => scopes.push("Mail.Send"),
                    // Graph's getSchedule endpoint accepts Calendars.ReadBasic
                    // as its least delegated permission. Keep full event
                    // details behind the separate calendar.read capability.
                    Capability::CalendarFreeBusy => scopes.push("Calendars.ReadBasic"),
                    Capability::CalendarRead => scopes.push("Calendars.Read"),
                    Capability::CalendarWrite => scopes.push("Calendars.ReadWrite"),
                    Capability::MailPrepare => {}
                }
            }
        }
        Provider::AppleIcloud => return Err(OAuthError::UnsupportedProvider),
    }
    scopes.sort_unstable();
    scopes.dedup();
    if scopes.len() > MAX_SCOPES
        || scopes.iter().all(|scope| {
            matches!(
                *scope,
                "openid" | "email" | "profile" | "offline_access" | "User.Read"
            )
        })
    {
        return Err(OAuthError::InvalidRequest);
    }
    Ok(scopes)
}

fn valid_redirect(url: &Url) -> bool {
    if url.username() != ""
        || url.password().is_some()
        || url.query().is_some()
        || url.fragment().is_some()
    {
        return false;
    }
    url.scheme() == "http"
        && url.host_str().is_some_and(|host| {
            host == "127.0.0.1" || host == "localhost" || host == "[::1]" || host == "::1"
        })
        && url.port().is_some_and(|port| port != 0)
}

fn state_key(state: &str) -> String {
    let digest = Sha256::digest(state.as_bytes());
    base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(digest)
}

fn valid_agent_id(value: &str) -> bool {
    !value.is_empty()
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-' || byte == b'_')
}

fn valid_scope_id(value: &str) -> bool {
    !value.trim().is_empty() && value.len() <= 256 && !value.chars().any(char::is_control)
}

fn sorted_unique<T: Clone + Ord>(values: &[T]) -> Vec<T> {
    let mut sorted = values.to_vec();
    sorted.sort_unstable();
    sorted.dedup();
    sorted
}

const MAX_HTTP_BODY: usize = 64 * 1024;

#[derive(Deserialize)]
struct TokenResponse {
    access_token: String,
    refresh_token: Option<String>,
    token_type: String,
    expires_in: u64,
    scope: Option<String>,
    id_token: Option<String>,
}

impl Drop for TokenResponse {
    fn drop(&mut self) {
        self.access_token.zeroize();
        self.refresh_token.zeroize();
        self.id_token.zeroize();
        self.scope.zeroize();
    }
}

#[derive(Deserialize)]
struct Jwks {
    keys: Vec<JsonWebKey>,
}

#[derive(Deserialize)]
struct JsonWebKey {
    kid: String,
    kty: String,
    n: String,
    e: String,
    alg: Option<String>,
    #[serde(rename = "use")]
    use_: Option<String>,
}

#[derive(Deserialize)]
struct JwtHeader {
    alg: String,
    kid: String,
    jku: Option<String>,
    crit: Option<Vec<String>>,
}

#[derive(Deserialize)]
struct OidcClaims {
    iss: String,
    sub: String,
    aud: AudienceClaim,
    exp: u64,
    iat: u64,
    nbf: Option<u64>,
    nonce: String,
    tid: Option<String>,
    azp: Option<String>,
    #[serde(default)]
    email: Option<String>,
    #[serde(default)]
    email_verified: Option<bool>,
    #[serde(default)]
    preferred_username: Option<String>,
}

impl Drop for OidcClaims {
    fn drop(&mut self) {
        self.iss.zeroize();
        self.sub.zeroize();
        self.nonce.zeroize();
        self.tid.zeroize();
        self.azp.zeroize();
        self.email.zeroize();
        self.preferred_username.zeroize();
        match &mut self.aud {
            AudienceClaim::One(value) => value.zeroize(),
            AudienceClaim::Many(values) => values.zeroize(),
        }
    }
}

#[derive(Deserialize)]
#[serde(untagged)]
enum AudienceClaim {
    One(String),
    Many(Vec<String>),
}

impl AudienceClaim {
    fn contains(&self, expected: &str) -> bool {
        match self {
            Self::One(value) => value == expected,
            Self::Many(values) => values.iter().any(|value| value == expected),
        }
    }

    fn len(&self) -> usize {
        match self {
            Self::One(_) => 1,
            Self::Many(values) => values.len(),
        }
    }
}

pub async fn redeem(
    grant: &AuthorizationGrant,
    code: &str,
) -> Result<RedeemedAuthorization, OAuthExchangeError> {
    let endpoints = OAuthEndpoints::for_provider(grant.provider)?;
    redeem_with_endpoints(grant, code, &endpoints).await
}

#[derive(Clone, Copy)]
struct OAuthEndpoints<'a> {
    token: &'a str,
    signing_keys: &'a str,
}

impl<'a> OAuthEndpoints<'a> {
    fn for_provider(provider: Provider) -> Result<Self, OAuthExchangeError> {
        match provider {
            Provider::Google => Ok(Self {
                token: "https://oauth2.googleapis.com/token",
                signing_keys: "https://www.googleapis.com/oauth2/v3/certs",
            }),
            Provider::Microsoft => Ok(Self {
                token: "https://login.microsoftonline.com/common/oauth2/v2.0/token",
                signing_keys: "https://login.microsoftonline.com/common/discovery/v2.0/keys",
            }),
            Provider::AppleIcloud => {
                Err(OAuthExchangeError::Setup(OAuthError::UnsupportedProvider))
            }
        }
    }
}

async fn redeem_with_endpoints(
    grant: &AuthorizationGrant,
    code: &str,
    endpoints: &OAuthEndpoints<'_>,
) -> Result<RedeemedAuthorization, OAuthExchangeError> {
    if code.trim().is_empty() || code.len() > 4096 || code.chars().any(char::is_control) {
        return Err(OAuthExchangeError::InvalidResponse);
    }
    let client = reqwest::Client::builder()
        .no_proxy()
        .redirect(reqwest::redirect::Policy::none())
        .timeout(Duration::from_secs(15))
        .build()
        .map_err(|_| OAuthExchangeError::Exchange)?;
    let requested_scope = grant.requested_scopes.join(" ");
    let mut form = vec![
        ("client_id", grant.client_id.as_str()),
        ("code", code),
        ("code_verifier", grant.code_verifier()),
        ("grant_type", "authorization_code"),
        ("redirect_uri", grant.redirect_uri.as_str()),
    ];
    if grant.provider == Provider::Microsoft {
        form.push(("scope", requested_scope.as_str()));
    }
    let mut token_response = client
        .post(endpoints.token)
        .form(&form)
        .send()
        .await
        .map_err(|_| OAuthExchangeError::Exchange)?;
    if !token_response.status().is_success() {
        return Err(OAuthExchangeError::Exchange);
    }
    let token_bytes = read_limited(&mut token_response).await?;
    let mut tokens: TokenResponse =
        serde_json::from_slice(&token_bytes).map_err(|_| OAuthExchangeError::InvalidResponse)?;
    if tokens.access_token.is_empty()
        || tokens.access_token.len() > 32 * 1024
        || !tokens.token_type.eq_ignore_ascii_case("bearer")
        || !(1..=86_400).contains(&tokens.expires_in)
        || tokens
            .id_token
            .as_ref()
            .is_none_or(|token| token.is_empty() || token.len() > 32 * 1024)
        || tokens
            .refresh_token
            .as_ref()
            .is_some_and(|token| token.is_empty() || token.len() > 32 * 1024)
    {
        return Err(OAuthExchangeError::InvalidResponse);
    }
    let granted_scopes = tokens
        .scope
        .as_deref()
        .ok_or(OAuthExchangeError::InsufficientScopes)?
        .split_ascii_whitespace()
        .map(str::to_owned)
        .collect::<Vec<_>>();
    if granted_scopes.len() > MAX_SCOPES
        || granted_scopes
            .iter()
            .any(|scope| scope.len() > 512 || scope.chars().any(char::is_control))
    {
        return Err(OAuthExchangeError::InvalidResponse);
    }
    if !requested_scopes_granted(grant.provider, &grant.requested_scopes, &granted_scopes)
        || !granted_scopes_are_bounded(grant.provider, &grant.requested_scopes, &granted_scopes)
    {
        return Err(OAuthExchangeError::InsufficientScopes);
    }
    let id_token = tokens
        .id_token
        .as_deref()
        .ok_or(OAuthExchangeError::InvalidResponse)?;
    let claims = verify_id_token(
        &client,
        grant.provider,
        id_token,
        &grant.client_id,
        grant.oidc_nonce(),
        endpoints.signing_keys,
    )
    .await?;
    let principal = match grant.provider {
        Provider::Google => format!("google:{}", claims.sub),
        Provider::Microsoft => format!(
            "microsoft:{}:{}",
            claims.tid.as_deref().unwrap_or("common"),
            claims.sub
        ),
        Provider::AppleIcloud => {
            return Err(OAuthExchangeError::Setup(OAuthError::UnsupportedProvider));
        }
    };
    let display_identity = claims
        .email
        .as_ref()
        .filter(|_| claims.email_verified == Some(true))
        .or(claims.preferred_username.as_ref())
        .cloned();
    let now = Utc::now();
    let access_token_expires_at =
        now + chrono::Duration::seconds(i64::try_from(tokens.expires_in).unwrap_or(86_400));
    let refresh_token_available = tokens.refresh_token.is_some();
    let access_token = std::mem::take(&mut tokens.access_token);
    let refresh_token = tokens.refresh_token.take();
    let secret_material = AccountSecretMaterial::new(
        principal,
        display_identity,
        Some(grant.client_id.clone()),
        Some(access_token),
        refresh_token,
        None,
        None,
    )
    .map_err(|_| OAuthExchangeError::InvalidResponse)?;
    Ok(RedeemedAuthorization {
        account_id: grant.account_id.clone(),
        provider: grant.provider,
        agent_id: grant.agent_id.clone(),
        audiences: grant.audiences.clone(),
        capabilities: grant.capabilities.clone(),
        granted_scopes,
        access_token_expires_at,
        refresh_token_available,
        secret_material,
    })
}

/// Refresh an account's access token using only the fixed provider token
/// endpoint. A rotated refresh token replaces the old one in the Agent vault;
/// callers update ledger expiry metadata after this succeeds.
pub async fn refresh_account_tokens(
    vault: &AccountVault,
    account: &ConnectedAccount,
) -> Result<RefreshedOAuthTokens, OAuthRefreshError> {
    let endpoints = OAuthEndpoints::for_provider(account.provider)
        .map_err(|_| OAuthRefreshError::ReconnectRequired)?;
    refresh_account_tokens_with_endpoint(vault, account, endpoints.token).await
}

async fn refresh_account_tokens_with_endpoint(
    vault: &AccountVault,
    account: &ConnectedAccount,
    token_endpoint: &str,
) -> Result<RefreshedOAuthTokens, OAuthRefreshError> {
    if vault.agent_id() != account.owner_agent_id
        || account.status != AccountStatus::Connected
        || account.revoked_at.is_some()
        || !account.refresh_token_available
        || account.provider == Provider::AppleIcloud
    {
        return Err(OAuthRefreshError::ReconnectRequired);
    }
    let (client_id, refresh_token) = vault
        .oauth_refresh_credentials(&account.id)
        .map_err(|_| OAuthRefreshError::ReconnectRequired)?;
    let client = reqwest::Client::builder()
        .no_proxy()
        .redirect(reqwest::redirect::Policy::none())
        .timeout(Duration::from_secs(15))
        .build()
        .map_err(|_| OAuthRefreshError::ProviderUnavailable)?;
    if account.provider == Provider::AppleIcloud {
        return Err(OAuthRefreshError::ReconnectRequired);
    }
    let client_id_value = client_id.as_str();
    let refresh_token_value = refresh_token.as_str();
    let mut form = vec![
        ("client_id", client_id_value),
        ("refresh_token", refresh_token_value),
        ("grant_type", "refresh_token"),
    ];
    let requested_scope;
    if account.provider == Provider::Microsoft {
        requested_scope = account
            .provider_scopes
            .iter()
            .cloned()
            .collect::<Vec<_>>()
            .join(" ");
        form.push(("scope", requested_scope.as_str()));
    }
    let mut response = client
        .post(token_endpoint)
        .form(&form)
        .send()
        .await
        .map_err(|_| OAuthRefreshError::ProviderUnavailable)?;
    if !response.status().is_success() {
        return Err(OAuthRefreshError::ReconnectRequired);
    }
    let token_bytes = read_limited(&mut response)
        .await
        .map_err(|_| OAuthRefreshError::ProviderUnavailable)?;
    let mut tokens: TokenResponse =
        serde_json::from_slice(&token_bytes).map_err(|_| OAuthRefreshError::InvalidResponse)?;
    if tokens.access_token.is_empty()
        || tokens.access_token.len() > 32 * 1024
        || !tokens.token_type.eq_ignore_ascii_case("bearer")
        || !(1..=86_400).contains(&tokens.expires_in)
        || tokens
            .refresh_token
            .as_ref()
            .is_some_and(|token| token.is_empty() || token.len() > 32 * 1024)
    {
        return Err(OAuthRefreshError::InvalidResponse);
    }
    if let Some(scope_text) = tokens.scope.as_deref() {
        let granted = scope_text.split_ascii_whitespace().collect::<Vec<_>>();
        if granted.len() > MAX_SCOPES
            || granted
                .iter()
                .any(|scope| scope.len() > 512 || scope.chars().any(char::is_control))
        {
            return Err(OAuthRefreshError::InvalidResponse);
        }
        let expected_scopes = account.provider_scopes.iter().cloned().collect::<Vec<_>>();
        let granted_scopes = granted.into_iter().map(str::to_owned).collect::<Vec<_>>();
        if !requested_scopes_granted(account.provider, &expected_scopes, &granted_scopes)
            || !granted_scopes_are_bounded(account.provider, &expected_scopes, &granted_scopes)
        {
            return Err(OAuthRefreshError::InsufficientScopes);
        }
    }
    let expires_at =
        Utc::now() + chrono::Duration::seconds(i64::try_from(tokens.expires_in).unwrap_or(86_400));
    let access_token = Zeroizing::new(std::mem::take(&mut tokens.access_token));
    let rotated_refresh_token = tokens.refresh_token.take().map(Zeroizing::new);
    Ok(RefreshedOAuthTokens {
        expires_at,
        access_token,
        refresh_token: rotated_refresh_token,
    })
}

/// Best-effort remote grant revocation before a disconnect removes local
/// secrets. Google supports an access/refresh-token revocation endpoint;
/// Microsoft does not expose per-application delegated-token revocation for
/// personal accounts, so that provider returns `false` and the UI must not
/// claim its grant was revoked.
pub async fn revoke_provider_grant(vault: &AccountVault, account: &ConnectedAccount) -> bool {
    revoke_provider_grant_with_endpoint(vault, account, "https://oauth2.googleapis.com/revoke")
        .await
}

async fn revoke_provider_grant_with_endpoint(
    vault: &AccountVault,
    account: &ConnectedAccount,
    revoke_endpoint: &str,
) -> bool {
    if vault.agent_id() != account.owner_agent_id || account.provider != Provider::Google {
        return false;
    }
    let Ok(token) = vault.oauth_revoke_token(&account.id) else {
        return false;
    };
    let Ok(client) = reqwest::Client::builder()
        .no_proxy()
        .redirect(reqwest::redirect::Policy::none())
        .timeout(Duration::from_secs(15))
        .build()
    else {
        return false;
    };
    let Ok(mut response) = client
        .post(revoke_endpoint)
        .form(&[("token", token.as_str())])
        .send()
        .await
    else {
        return false;
    };
    if response.status() != reqwest::StatusCode::OK {
        return false;
    }
    read_limited(&mut response).await.is_ok()
}

fn scope_matches(provider: Provider, expected: &str, actual: &str) -> bool {
    if expected == actual {
        return true;
    }
    provider == Provider::Microsoft
        && actual
            .strip_prefix("https://graph.microsoft.com/")
            .is_some_and(|scope| scope.eq_ignore_ascii_case(expected))
}

fn requested_scopes_granted(provider: Provider, requested: &[String], granted: &[String]) -> bool {
    requested
        .iter()
        .filter(|scope| {
            !matches!(
                scope.as_str(),
                "openid" | "email" | "profile" | "offline_access" | "User.Read"
            )
        })
        .all(|expected| {
            granted
                .iter()
                .any(|actual| scope_matches(provider, expected, actual))
        })
}

/// Reject provider-returned grants beyond the selected capability set. The
/// broker must not treat a union of prior consent as authority for this link.
fn granted_scopes_are_bounded(
    provider: Provider,
    requested: &[String],
    granted: &[String],
) -> bool {
    granted.iter().all(|actual| {
        requested
            .iter()
            .any(|expected| scope_matches(provider, expected, actual))
            || (provider == Provider::Google
                && ((actual == "https://www.googleapis.com/auth/userinfo.email"
                    && requested.iter().any(|scope| scope == "email"))
                    || (actual == "https://www.googleapis.com/auth/userinfo.profile"
                        && requested.iter().any(|scope| scope == "profile"))))
    })
}

pub(crate) fn account_grants_are_valid(
    provider: Provider,
    capabilities: &[Capability],
    granted: &std::collections::BTreeSet<String>,
) -> bool {
    if capabilities.is_empty()
        || capabilities.iter().any(|capability| {
            !matches!(
                capability,
                Capability::MailRead | Capability::CalendarFreeBusy | Capability::CalendarRead
            )
        })
    {
        return false;
    }
    if provider == Provider::AppleIcloud {
        return granted.is_empty();
    }
    let Ok(requested) = scopes_for(provider, capabilities) else {
        return false;
    };
    let requested = requested.into_iter().map(str::to_owned).collect::<Vec<_>>();
    let granted = granted.iter().cloned().collect::<Vec<_>>();
    requested_scopes_granted(provider, &requested, &granted)
        && granted_scopes_are_bounded(provider, &requested, &granted)
}

async fn verify_id_token(
    client: &reqwest::Client,
    provider: Provider,
    token: &str,
    client_id: &str,
    expected_nonce: &str,
    keys_url: &str,
) -> Result<OidcClaims, OAuthExchangeError> {
    let parts = token.split('.').collect::<Vec<_>>();
    if parts.len() != 3 || parts.iter().any(|part| part.len() > 32 * 1024) {
        return Err(OAuthExchangeError::InvalidResponse);
    }
    let header_bytes = Zeroizing::new(
        base64::engine::general_purpose::URL_SAFE_NO_PAD
            .decode(parts[0])
            .map_err(|_| OAuthExchangeError::InvalidResponse)?,
    );
    let header: JwtHeader =
        serde_json::from_slice(&header_bytes).map_err(|_| OAuthExchangeError::InvalidResponse)?;
    if header.alg != "RS256"
        || header.kid.is_empty()
        || header.jku.is_some()
        || header.crit.as_ref().is_some_and(|crit| !crit.is_empty())
    {
        return Err(OAuthExchangeError::InvalidResponse);
    }
    let claims_bytes = Zeroizing::new(
        base64::engine::general_purpose::URL_SAFE_NO_PAD
            .decode(parts[1])
            .map_err(|_| OAuthExchangeError::InvalidResponse)?,
    );
    let claims: OidcClaims =
        serde_json::from_slice(&claims_bytes).map_err(|_| OAuthExchangeError::InvalidResponse)?;
    let mut key_response = client
        .get(keys_url)
        .send()
        .await
        .map_err(|_| OAuthExchangeError::Exchange)?;
    if !key_response.status().is_success() {
        return Err(OAuthExchangeError::Exchange);
    }
    let key_bytes = read_limited(&mut key_response).await?;
    let jwks: Jwks =
        serde_json::from_slice(&key_bytes).map_err(|_| OAuthExchangeError::InvalidResponse)?;
    let key = jwks
        .keys
        .iter()
        .find(|key| {
            key.kid == header.kid
                && key.kty == "RSA"
                && key.alg.as_deref().is_none_or(|alg| alg == "RS256")
                && key.use_.as_deref().is_none_or(|usage| usage == "sig")
        })
        .ok_or(OAuthExchangeError::InvalidResponse)?;
    if key.n.len() > 4096 || key.e.len() > 16 {
        return Err(OAuthExchangeError::InvalidResponse);
    }
    let modulus = base64::engine::general_purpose::URL_SAFE_NO_PAD
        .decode(&key.n)
        .map_err(|_| OAuthExchangeError::InvalidResponse)?;
    let exponent = base64::engine::general_purpose::URL_SAFE_NO_PAD
        .decode(&key.e)
        .map_err(|_| OAuthExchangeError::InvalidResponse)?;
    let signature = base64::engine::general_purpose::URL_SAFE_NO_PAD
        .decode(parts[2])
        .map_err(|_| OAuthExchangeError::InvalidResponse)?;
    let signed = format!("{}.{}", parts[0], parts[1]);
    RsaPublicKeyComponents {
        n: &modulus,
        e: &exponent,
    }
    .verify(&RSA_PKCS1_2048_8192_SHA256, signed.as_bytes(), &signature)
    .map_err(|_| OAuthExchangeError::InvalidResponse)?;

    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_err(|_| OAuthExchangeError::InvalidResponse)?
        .as_secs();
    validate_oidc_claims(provider, &claims, client_id, expected_nonce, now)?;
    Ok(claims)
}

fn validate_oidc_claims(
    provider: Provider,
    claims: &OidcClaims,
    client_id: &str,
    expected_nonce: &str,
    now: u64,
) -> Result<(), OAuthExchangeError> {
    if claims.sub.is_empty()
        || claims.sub.len() > 512
        || !bool::from(claims.nonce.as_bytes().ct_eq(expected_nonce.as_bytes()))
        || !claims.aud.contains(client_id)
        || claims.azp.as_deref().is_some_and(|azp| azp != client_id)
        || (claims.aud.len() > 1 && claims.azp.as_deref() != Some(client_id))
        || claims.exp <= now
        || claims.exp > now.saturating_add(86_400)
        || claims.iat > now.saturating_add(300)
        || claims.iat < now.saturating_sub(86_400)
        || claims.nbf.is_some_and(|nbf| nbf > now.saturating_add(300))
        || !valid_issuer(provider, claims)
    {
        return Err(OAuthExchangeError::InvalidResponse);
    }
    Ok(())
}

fn valid_issuer(provider: Provider, claims: &OidcClaims) -> bool {
    match provider {
        Provider::Google => {
            claims.iss == "https://accounts.google.com" || claims.iss == "accounts.google.com"
        }
        Provider::Microsoft => claims.tid.as_deref().is_some_and(|tenant| {
            uuid::Uuid::parse_str(tenant).is_ok()
                && claims.iss == format!("https://login.microsoftonline.com/{tenant}/v2.0")
        }),
        Provider::AppleIcloud => false,
    }
}

async fn read_limited(
    response: &mut reqwest::Response,
) -> Result<Zeroizing<Vec<u8>>, OAuthExchangeError> {
    if response
        .content_length()
        .is_some_and(|length| length > MAX_HTTP_BODY as u64)
    {
        return Err(OAuthExchangeError::InvalidResponse);
    }
    let mut body = Zeroizing::new(Vec::new());
    while let Some(chunk) = response
        .chunk()
        .await
        .map_err(|_| OAuthExchangeError::Exchange)?
    {
        if body.len().saturating_add(chunk.len()) > MAX_HTTP_BODY {
            return Err(OAuthExchangeError::InvalidResponse);
        }
        body.extend_from_slice(&chunk);
    }
    Ok(body)
}

#[cfg(test)]
#[allow(clippy::expect_used, clippy::unwrap_used)]
mod tests {
    use super::*;

    fn make_store() -> (AuthorizationStore, String, String) {
        let store = AuthorizationStore::default();
        let (url, _, callback_binding) = store
            .begin(
                Provider::Google,
                "agent-7",
                Some("session-42"),
                &["conversation-1".to_owned()],
                &[Capability::MailRead],
                "vak-test-client",
                "http://127.0.0.1:43127/oauth/callback",
                None,
            )
            .expect("valid Google authorization request");
        (store, url, callback_binding)
    }

    #[test]
    fn oidc_identity_claims_reject_wrong_audience_nonce_issuer_and_times() {
        fn valid_claims(provider: Provider, now: u64) -> OidcClaims {
            let tenant = "11111111-2222-4333-8444-555555555555";
            let (iss, tid) = match provider {
                Provider::Google => ("https://accounts.google.com".to_owned(), None),
                Provider::Microsoft => (
                    format!("https://login.microsoftonline.com/{tenant}/v2.0"),
                    Some(tenant.to_owned()),
                ),
                Provider::AppleIcloud => (String::new(), None),
            };
            OidcClaims {
                iss,
                sub: "fixture-subject".to_owned(),
                aud: AudienceClaim::One("fixture-client".to_owned()),
                exp: now + 3600,
                iat: now,
                nbf: None,
                nonce: "fixture-nonce".to_owned(),
                tid,
                azp: None,
                email: None,
                email_verified: None,
                preferred_username: None,
            }
        }

        let now = 1_800_000_000;
        for provider in [Provider::Google, Provider::Microsoft] {
            assert!(
                validate_oidc_claims(
                    provider,
                    &valid_claims(provider, now),
                    "fixture-client",
                    "fixture-nonce",
                    now
                )
                .is_ok()
            );

            let mut wrong_audience = valid_claims(provider, now);
            wrong_audience.aud = AudienceClaim::One("other-client".to_owned());
            assert!(
                validate_oidc_claims(
                    provider,
                    &wrong_audience,
                    "fixture-client",
                    "fixture-nonce",
                    now
                )
                .is_err()
            );

            let mut wrong_nonce = valid_claims(provider, now);
            wrong_nonce.nonce = "another-flow".to_owned();
            assert!(
                validate_oidc_claims(
                    provider,
                    &wrong_nonce,
                    "fixture-client",
                    "fixture-nonce",
                    now
                )
                .is_err()
            );

            let mut wrong_issuer = valid_claims(provider, now);
            wrong_issuer.iss = "https://attacker.example/issuer".to_owned();
            assert!(
                validate_oidc_claims(
                    provider,
                    &wrong_issuer,
                    "fixture-client",
                    "fixture-nonce",
                    now
                )
                .is_err()
            );

            let mut expired = valid_claims(provider, now);
            expired.exp = now;
            assert!(
                validate_oidc_claims(provider, &expired, "fixture-client", "fixture-nonce", now)
                    .is_err()
            );

            let mut future_issued = valid_claims(provider, now);
            future_issued.iat = now + 301;
            assert!(
                validate_oidc_claims(
                    provider,
                    &future_issued,
                    "fixture-client",
                    "fixture-nonce",
                    now
                )
                .is_err()
            );
        }

        let mut wrong_tenant = valid_claims(Provider::Microsoft, now);
        wrong_tenant.tid = Some("not-a-tenant-uuid".to_owned());
        assert!(
            validate_oidc_claims(
                Provider::Microsoft,
                &wrong_tenant,
                "fixture-client",
                "fixture-nonce",
                now
            )
            .is_err()
        );
    }

    #[tokio::test]
    async fn google_and_microsoft_provider_test_doubles_exercise_token_exchange() {
        use axum::{Json, Router, body::Bytes, extract::State, routing::get, routing::post};
        use std::sync::{Arc, Mutex};

        type FixtureState = Arc<Mutex<(Vec<u8>, String)>>;

        async fn token(
            State(fixture): State<FixtureState>,
            body: Bytes,
        ) -> Json<serde_json::Value> {
            let scope = {
                let mut fixture = fixture.lock().unwrap();
                fixture.0 = body.to_vec();
                fixture.1.clone()
            };
            Json(serde_json::json!({
                "access_token": "fixture-access-token",
                "token_type": "Bearer",
                "expires_in": 3600,
                "id_token": "eyJhbGciOiJSUzI1NiIsImtpZCI6ImZpeHR1cmUta2V5In0.eyJzdWIiOiJmaXh0dXJlLXN1YiJ9.AA",
                "scope": scope
            }))
        }

        async fn keys() -> Json<serde_json::Value> {
            Json(serde_json::json!({ "keys": [] }))
        }

        for provider in [Provider::Google, Provider::Microsoft] {
            let production_endpoints = OAuthEndpoints::for_provider(provider).unwrap();
            match provider {
                Provider::Google => {
                    assert_eq!(
                        production_endpoints.token,
                        "https://oauth2.googleapis.com/token"
                    );
                    assert_eq!(
                        production_endpoints.signing_keys,
                        "https://www.googleapis.com/oauth2/v3/certs"
                    );
                }
                Provider::Microsoft => {
                    assert_eq!(
                        production_endpoints.token,
                        "https://login.microsoftonline.com/common/oauth2/v2.0/token"
                    );
                    assert_eq!(
                        production_endpoints.signing_keys,
                        "https://login.microsoftonline.com/common/discovery/v2.0/keys"
                    );
                }
                Provider::AppleIcloud => unreachable!(),
            }

            let store = AuthorizationStore::default();
            let redirect_uri = "http://127.0.0.1:43127/mail-calendar/oauth/callback";
            let (authorization_url, _) = store
                .begin_native(
                    provider,
                    "agent-test-double",
                    &["agent:agent-test-double".to_owned()],
                    &[Capability::MailRead],
                    "fixture-client",
                    redirect_uri,
                )
                .unwrap();
            let state = Url::parse(&authorization_url)
                .unwrap()
                .query_pairs()
                .find(|(name, _)| name == "state")
                .unwrap()
                .1
                .into_owned();
            let grant = store.consume(&state, None, None).unwrap();
            let requested_scopes = scopes_for(provider, &[Capability::MailRead])
                .unwrap()
                .join(" ");

            let fixture = Arc::new(Mutex::new((Vec::new(), requested_scopes.clone())));
            let app = Router::new()
                .route("/token", post(token))
                .route("/keys", get(keys))
                .with_state(fixture.clone());
            let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
            let address = listener.local_addr().unwrap();
            let server = tokio::spawn(async move {
                axum::serve(listener, app).await.unwrap();
            });

            let token_url = format!("http://{address}/token");
            let signing_keys_url = format!("http://{address}/keys");
            let endpoints = OAuthEndpoints {
                token: &token_url,
                signing_keys: &signing_keys_url,
            };
            let result = redeem_with_endpoints(&grant, "fixture-code", &endpoints).await;
            assert!(matches!(result, Err(OAuthExchangeError::InvalidResponse)));
            let body = fixture.lock().unwrap().0.clone();
            let form = url::form_urlencoded::parse(&body)
                .into_owned()
                .collect::<BTreeMap<_, _>>();
            assert_eq!(
                form.get("client_id").map(String::as_str),
                Some("fixture-client")
            );
            assert_eq!(form.get("code").map(String::as_str), Some("fixture-code"));
            assert_eq!(
                form.get("code_verifier").map(String::as_str),
                Some(grant.code_verifier())
            );
            assert_eq!(
                form.get("redirect_uri").map(String::as_str),
                Some(redirect_uri)
            );
            assert!(!form.contains_key("client_secret"));
            if provider == Provider::Microsoft {
                assert_eq!(form.get("scope"), Some(&requested_scopes));
            } else {
                assert!(!form.contains_key("scope"));
            }
            server.abort();
        }
    }

    #[tokio::test]
    async fn oversized_oauth_provider_responses_fail_before_body_parsing() {
        use axum::{
            Router, body::Body, http::header::CONTENT_LENGTH, response::Response, routing::get,
        };

        async fn oversized() -> Response<Body> {
            Response::builder()
                .header(CONTENT_LENGTH, (MAX_HTTP_BODY + 1).to_string())
                .body(Body::from(vec![b'x'; MAX_HTTP_BODY + 1]))
                .unwrap()
        }

        let app = Router::new().route("/oversized", get(oversized));
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let server = tokio::spawn(async move {
            axum::serve(listener, app).await.unwrap();
        });
        let client = reqwest::Client::builder().no_proxy().build().unwrap();
        let mut response = client
            .get(format!("http://{address}/oversized"))
            .send()
            .await
            .unwrap();
        assert!(matches!(
            read_limited(&mut response).await,
            Err(OAuthExchangeError::InvalidResponse)
        ));
        server.abort();
    }

    #[tokio::test]
    async fn valid_google_and_microsoft_test_doubles_persist_verified_accounts() {
        use axum::{Json, Router, extract::State, routing::get, routing::post};
        use ring::rand::SystemRandom;
        use ring::signature::{KeyPair, RSA_PKCS1_SHA256, RsaKeyPair};
        use std::time::{SystemTime, UNIX_EPOCH};

        #[derive(Clone)]
        struct Fixture {
            id_token: String,
            scope: String,
            keys: serde_json::Value,
        }

        async fn token(State(fixture): State<Fixture>) -> Json<serde_json::Value> {
            Json(serde_json::json!({
                "access_token": "verified-fixture-access-token",
                "refresh_token": "verified-fixture-refresh-token",
                "token_type": "Bearer",
                "expires_in": 3600,
                "id_token": fixture.id_token,
                "scope": fixture.scope
            }))
        }

        async fn keys(State(fixture): State<Fixture>) -> Json<serde_json::Value> {
            Json(fixture.keys)
        }

        fn der_value<'a>(bytes: &'a [u8], offset: &mut usize) -> (u8, &'a [u8]) {
            let tag = bytes[*offset];
            *offset += 1;
            let first_length = bytes[*offset];
            *offset += 1;
            let length = if first_length & 0x80 == 0 {
                usize::from(first_length)
            } else {
                let octets = usize::from(first_length & 0x7f);
                let mut length = 0_usize;
                for _ in 0..octets {
                    length = (length << 8) | usize::from(bytes[*offset]);
                    *offset += 1;
                }
                length
            };
            let end = *offset + length;
            let value = &bytes[*offset..end];
            *offset = end;
            (tag, value)
        }

        let rng = SystemRandom::new();
        let key_pair =
            RsaKeyPair::from_pkcs8(include_bytes!("../tests/fixtures/oauth-test-key.pk8")).unwrap();
        let public_key = key_pair.public_key().as_ref();
        let mut offset = 0;
        let (0x30, rsa_sequence) = der_value(public_key, &mut offset) else {
            panic!("expected an RSA public-key sequence");
        };
        let mut offset = 0;
        let (0x02, modulus) = der_value(rsa_sequence, &mut offset) else {
            panic!("expected an RSA modulus");
        };
        let (0x02, exponent) = der_value(rsa_sequence, &mut offset) else {
            panic!("expected an RSA exponent");
        };
        let modulus = modulus.strip_prefix(&[0]).unwrap_or(modulus);
        let encode = |bytes: &[u8]| base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(bytes);

        let redirect_uri = "http://127.0.0.1:43127/mail-calendar/oauth/callback";
        let tenant_id = "11111111-2222-4333-8444-555555555555";
        for provider in [Provider::Google, Provider::Microsoft] {
            let store = AuthorizationStore::default();
            let provider_name = match provider {
                Provider::Google => "google",
                Provider::Microsoft => "microsoft",
                Provider::AppleIcloud => continue,
            };
            let agent_id = format!("oauth-positive-{provider_name}-{}", uuid::Uuid::now_v7());
            let (authorization_url, account_id) = store
                .begin_native(
                    provider,
                    &agent_id,
                    &[format!("agent:{agent_id}")],
                    &[Capability::MailRead],
                    "fixture-client",
                    redirect_uri,
                )
                .unwrap();
            let state = Url::parse(&authorization_url)
                .unwrap()
                .query_pairs()
                .find(|(name, _)| name == "state")
                .unwrap()
                .1
                .into_owned();
            let grant = store.consume(&state, None, None).unwrap();
            let now = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_secs();
            let issuer = match provider {
                Provider::Google => "https://accounts.google.com".to_owned(),
                Provider::Microsoft => {
                    format!("https://login.microsoftonline.com/{tenant_id}/v2.0")
                }
                Provider::AppleIcloud => continue,
            };
            let header = encode(br#"{"alg":"RS256","kid":"fixture-key"}"#);
            let claims = encode(
                serde_json::to_string(&serde_json::json!({
                    "iss": issuer,
                    "sub": "fixture-subject",
                    "tid": (provider == Provider::Microsoft).then_some(tenant_id),
                    "aud": "fixture-client",
                    "exp": now + 3600,
                    "iat": now,
                    "nonce": grant.oidc_nonce(),
                    "email": "person@example.com",
                    "email_verified": true
                }))
                .unwrap()
                .as_bytes(),
            );
            let signing_input = format!("{header}.{claims}");
            let mut signature = vec![0; key_pair.public().modulus_len()];
            key_pair
                .sign(
                    &RSA_PKCS1_SHA256,
                    &rng,
                    signing_input.as_bytes(),
                    &mut signature,
                )
                .unwrap();
            let id_token = format!("{signing_input}.{}", encode(&signature));
            let requested_scopes = scopes_for(provider, &[Capability::MailRead])
                .unwrap()
                .join(" ");
            let fixture = Fixture {
                id_token,
                scope: requested_scopes,
                keys: serde_json::json!({
                    "keys": [{
                        "kid": "fixture-key",
                        "kty": "RSA",
                        "alg": "RS256",
                        "use": "sig",
                        "n": encode(modulus),
                        "e": encode(exponent)
                    }]
                }),
            };
            let app = Router::new()
                .route("/token", post(token))
                .route("/keys", get(keys))
                .with_state(fixture);
            let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
            let address = listener.local_addr().unwrap();
            let server = tokio::spawn(async move {
                axum::serve(listener, app).await.unwrap();
            });
            let token_url = format!("http://{address}/token");
            let signing_keys_url = format!("http://{address}/keys");
            let endpoints = OAuthEndpoints {
                token: &token_url,
                signing_keys: &signing_keys_url,
            };
            let redeemed = redeem_with_endpoints(&grant, "fixture-code", &endpoints)
                .await
                .unwrap();
            assert_eq!(redeemed.account_id, account_id);
            assert_eq!(redeemed.provider, provider);
            assert_eq!(redeemed.capabilities, vec![Capability::MailRead]);
            assert!(redeemed.refresh_token_available);

            vak_config::paths::isolate_home_for_tests();
            let vault = AccountVault::for_agent(&agent_id).unwrap();
            let ledger = ConnectionLedger::for_agent(&agent_id).unwrap();
            let account = redeemed.persist(&vault, &ledger).unwrap();
            assert_eq!(account.status, AccountStatus::Connected);
            assert!(account.admits(
                &agent_id,
                &format!("agent:{agent_id}"),
                Capability::MailRead
            ));
            assert_eq!(ledger.read_all().unwrap(), vec![account]);
            assert_eq!(
                vault
                    .load(&account_id)
                    .unwrap()
                    .masked_display_identity()
                    .as_deref(),
                Some("p***@example.com")
            );
            server.abort();
        }
    }

    #[tokio::test]
    async fn google_and_microsoft_refresh_test_doubles_stage_only_bounded_rotations() {
        use crate::vault::{AccountSecretMaterial, AccountVault};
        use axum::{Json, Router, body::Bytes, extract::State, routing::post};
        use std::sync::{Arc, Mutex};

        type FixtureState = Arc<Mutex<(Vec<u8>, String)>>;

        async fn refresh(
            State(fixture): State<FixtureState>,
            body: Bytes,
        ) -> Json<serde_json::Value> {
            let scope = {
                let mut fixture = fixture.lock().unwrap();
                fixture.0 = body.to_vec();
                fixture.1.clone()
            };
            Json(serde_json::json!({
                "access_token": "rotated-access-token",
                "refresh_token": "rotated-refresh-token",
                "token_type": "Bearer",
                "expires_in": 3600,
                "scope": scope
            }))
        }

        vak_config::paths::isolate_home_for_tests();
        for provider in [Provider::Google, Provider::Microsoft] {
            let agent_id = format!(
                "refresh-double-{}-{}",
                provider_name(provider),
                uuid::Uuid::now_v7()
            );
            let account_id = uuid::Uuid::now_v7().to_string();
            let vault = AccountVault::for_agent(&agent_id).unwrap();
            let provider_scopes = scopes_for(provider, &[Capability::MailRead])
                .unwrap()
                .into_iter()
                .map(str::to_owned)
                .collect::<std::collections::BTreeSet<_>>();
            let account = ConnectedAccount {
                id: account_id.clone(),
                provider,
                status: AccountStatus::Connected,
                owner_agent_id: agent_id.clone(),
                allowed_audiences: [format!("agent:{agent_id}")].into_iter().collect(),
                capabilities: [Capability::MailRead].into_iter().collect(),
                provider_scopes: provider_scopes.clone(),
                credential_ref: AccountVault::credential_ref(&account_id).unwrap(),
                principal_ref: AccountVault::credential_ref(&account_id).unwrap(),
                revision: 1,
                connected_at: Utc::now(),
                access_token_expires_at: Some(Utc::now() + chrono::Duration::minutes(5)),
                refresh_token_available: true,
                revoked_at: None,
            };
            vault
                .store(
                    &account_id,
                    AccountSecretMaterial::new(
                        format!("{}:fixture-subject", provider_name(provider)),
                        Some("person@example.com".to_owned()),
                        Some("fixture-client".to_owned()),
                        Some("old-access-token".to_owned()),
                        Some("old-refresh-token".to_owned()),
                        None,
                        None,
                    )
                    .unwrap(),
                )
                .unwrap();

            let scope = provider_scopes
                .iter()
                .cloned()
                .collect::<Vec<_>>()
                .join(" ");
            let extra_scope = match provider {
                Provider::Google => "https://www.googleapis.com/auth/gmail.modify",
                Provider::Microsoft => "Mail.Send",
                Provider::AppleIcloud => continue,
            };
            for broaden_grant in [true, false] {
                let response_scope = if broaden_grant {
                    format!("{scope} {extra_scope}")
                } else {
                    scope.clone()
                };
                let fixture = Arc::new(Mutex::new((Vec::new(), response_scope)));
                let app = Router::new()
                    .route("/token", post(refresh))
                    .with_state(fixture.clone());
                let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
                let address = listener.local_addr().unwrap();
                let server = tokio::spawn(async move {
                    axum::serve(listener, app).await.unwrap();
                });
                let token_url = format!("http://{address}/token");
                let rotated_result =
                    refresh_account_tokens_with_endpoint(&vault, &account, &token_url).await;
                let body = fixture.lock().unwrap().0.clone();
                let form = url::form_urlencoded::parse(&body)
                    .into_owned()
                    .collect::<BTreeMap<_, _>>();
                assert_eq!(
                    form.get("client_id").map(String::as_str),
                    Some("fixture-client")
                );
                assert_eq!(
                    form.get("refresh_token").map(String::as_str),
                    Some("old-refresh-token")
                );
                assert_eq!(
                    form.get("grant_type").map(String::as_str),
                    Some("refresh_token")
                );
                if provider == Provider::Microsoft {
                    assert_eq!(form.get("scope"), Some(&scope));
                } else {
                    assert!(!form.contains_key("scope"));
                }
                assert_eq!(
                    vault
                        .oauth_refresh_credentials(&account_id)
                        .unwrap()
                        .1
                        .as_str(),
                    "old-refresh-token"
                );

                if broaden_grant {
                    assert!(matches!(
                        rotated_result,
                        Err(OAuthRefreshError::InsufficientScopes)
                    ));
                    assert_eq!(
                        vault
                            .oauth_refresh_credentials(&account_id)
                            .unwrap()
                            .1
                            .as_str(),
                        "old-refresh-token"
                    );
                } else {
                    rotated_result.unwrap().persist(&vault, &account).unwrap();
                    assert_eq!(
                        vault
                            .oauth_refresh_credentials(&account_id)
                            .unwrap()
                            .1
                            .as_str(),
                        "rotated-refresh-token"
                    );
                }
                server.abort();
            }
        }
    }

    #[tokio::test]
    async fn google_revocation_test_double_receives_only_the_vaulted_token() {
        use crate::vault::{AccountSecretMaterial, AccountVault};
        use axum::{Router, body::Bytes, extract::State, http::StatusCode, routing::post};
        use std::sync::{Arc, Mutex};

        async fn revoke(State(captured): State<Arc<Mutex<Vec<u8>>>>, body: Bytes) -> StatusCode {
            *captured.lock().unwrap() = body.to_vec();
            StatusCode::OK
        }

        vak_config::paths::isolate_home_for_tests();
        let agent_id = format!("revoke-double-{}", uuid::Uuid::now_v7());
        let account_id = uuid::Uuid::now_v7().to_string();
        let vault = AccountVault::for_agent(&agent_id).unwrap();
        vault
            .store(
                &account_id,
                AccountSecretMaterial::new(
                    "google:fixture-subject".to_owned(),
                    Some("person@example.com".to_owned()),
                    Some("fixture-client".to_owned()),
                    Some("fixture-access-token".to_owned()),
                    Some("fixture-refresh-token".to_owned()),
                    None,
                    None,
                )
                .unwrap(),
            )
            .unwrap();
        let mut account = ConnectedAccount {
            id: account_id.clone(),
            provider: Provider::Google,
            status: AccountStatus::Connected,
            owner_agent_id: agent_id.clone(),
            allowed_audiences: [format!("agent:{agent_id}")].into_iter().collect(),
            capabilities: [Capability::MailRead].into_iter().collect(),
            provider_scopes: scopes_for(Provider::Google, &[Capability::MailRead])
                .unwrap()
                .into_iter()
                .map(str::to_owned)
                .collect(),
            credential_ref: AccountVault::credential_ref(&account_id).unwrap(),
            principal_ref: AccountVault::credential_ref(&account_id).unwrap(),
            revision: 1,
            connected_at: Utc::now(),
            access_token_expires_at: Some(Utc::now() + chrono::Duration::minutes(5)),
            refresh_token_available: true,
            revoked_at: None,
        };

        let captured = Arc::new(Mutex::new(Vec::new()));
        let app = Router::new()
            .route("/revoke", post(revoke))
            .with_state(captured.clone());
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let server = tokio::spawn(async move {
            axum::serve(listener, app).await.unwrap();
        });
        let endpoint = format!("http://{address}/revoke");
        assert!(revoke_provider_grant_with_endpoint(&vault, &account, &endpoint).await);
        let form = url::form_urlencoded::parse(&captured.lock().unwrap())
            .into_owned()
            .collect::<BTreeMap<_, _>>();
        assert_eq!(form.len(), 1);
        assert_eq!(
            form.get("token").map(String::as_str),
            Some("fixture-refresh-token")
        );

        captured.lock().unwrap().clear();
        account.provider = Provider::Microsoft;
        assert!(!revoke_provider_grant_with_endpoint(&vault, &account, &endpoint).await);
        assert!(captured.lock().unwrap().is_empty());
        server.abort();
    }

    fn provider_name(provider: Provider) -> &'static str {
        match provider {
            Provider::Google => "google",
            Provider::Microsoft => "microsoft",
            Provider::AppleIcloud => "apple",
        }
    }

    #[test]
    fn callback_requires_the_initiating_session_and_is_single_use() {
        let (store, url, callback_binding) = make_store();
        let parsed = Url::parse(&url).expect("authorization URL parses");
        let params = parsed.query_pairs().collect::<BTreeMap<_, _>>();
        let state = params.get("state").expect("state parameter").to_string();
        let verifier = store.consume(&state, Some("different-session"), Some(&callback_binding));
        assert!(verifier.is_none());
        assert!(store.consume(&state, None, Some(&"x".repeat(43))).is_none());
        assert!(store.consume(&state, Some("session-42"), None).is_none());

        let grant = store
            .consume(&state, None, Some(&callback_binding))
            .expect("matching HttpOnly callback cookie for the initiating browser");
        assert_eq!(grant.provider, Provider::Google);
        assert_eq!(grant.initiating_session_id(), Some("session-42"));
        assert_eq!(grant.code_verifier().len(), 43);
        assert_eq!(grant.oidc_nonce().len(), 43);
        assert!(
            store
                .consume(&state, Some("session-42"), Some(&callback_binding))
                .is_none()
        );
    }

    #[test]
    fn native_loopback_callback_is_single_use_and_rejects_browser_binding() {
        let store = AuthorizationStore::default();
        let (url, _) = store
            .begin_native(
                Provider::Microsoft,
                "agent-7",
                &["agent:agent-7".to_owned()],
                &[Capability::MailRead],
                "vak-test-client",
                "http://localhost:43127/mail-calendar/oauth/callback",
            )
            .expect("owner-authorized native flow can start");
        let state = Url::parse(&url)
            .expect("authorization URL parses")
            .query_pairs()
            .find(|(name, _)| name == "state")
            .map(|(_, value)| value.into_owned())
            .expect("state is present");
        assert!(store.consume(&state, None, Some(&"x".repeat(43))).is_none());
        let grant = store
            .consume(&state, None, None)
            .expect("native state returns once");
        assert_eq!(grant.initiating_session_id(), None);
        assert!(store.consume(&state, None, None).is_none());
    }

    #[test]
    fn provider_disconnect_cancels_pending_and_in_flight_authorizations() {
        let store = AuthorizationStore::default();
        let (url, _) = store
            .begin_native(
                Provider::Google,
                "agent-7",
                &["agent:agent-7".to_owned()],
                &[Capability::MailRead],
                "vak-test-client",
                "http://127.0.0.1:43127/mail-calendar/oauth/callback",
            )
            .unwrap();
        let state = Url::parse(&url)
            .unwrap()
            .query_pairs()
            .find(|(name, _)| name == "state")
            .map(|(_, value)| value.into_owned())
            .unwrap();
        let in_flight = store.consume(&state, None, None).unwrap();
        assert!(store.is_current(&in_flight));
        let mut committed = false;
        assert_eq!(
            store.with_current(&in_flight, || {
                committed = true;
            }),
            Some(())
        );
        assert!(committed);

        store.cancel_provider("agent-7", Provider::Google);
        assert!(!store.is_current(&in_flight));
        committed = false;
        assert!(
            store
                .with_current(&in_flight, || {
                    committed = true;
                })
                .is_none()
        );
        assert!(!committed);

        let (pending_url, _) = store
            .begin_native(
                Provider::Google,
                "agent-7",
                &["agent:agent-7".to_owned()],
                &[Capability::MailRead],
                "vak-test-client",
                "http://127.0.0.1:43127/mail-calendar/oauth/callback",
            )
            .unwrap();
        let pending_state = Url::parse(&pending_url)
            .unwrap()
            .query_pairs()
            .find(|(name, _)| name == "state")
            .map(|(_, value)| value.into_owned())
            .unwrap();
        store.cancel_provider("agent-7", Provider::Google);
        assert!(store.consume(&pending_state, None, None).is_none());

        let (fresh_url, _) = store
            .begin_native(
                Provider::Google,
                "agent-7",
                &["agent:agent-7".to_owned()],
                &[Capability::MailRead],
                "vak-test-client",
                "http://127.0.0.1:43127/mail-calendar/oauth/callback",
            )
            .unwrap();
        let fresh_state = Url::parse(&fresh_url)
            .unwrap()
            .query_pairs()
            .find(|(name, _)| name == "state")
            .map(|(_, value)| value.into_owned())
            .unwrap();
        let fresh = store.consume(&fresh_state, None, None).unwrap();
        assert!(store.is_current(&fresh));
    }

    #[test]
    fn redeemed_authorization_cannot_persist_into_another_agents_vault() {
        vak_config::paths::isolate_home_for_tests();
        let owner_agent = format!("mailcal-owner-{}", uuid::Uuid::now_v7());
        let other_agent = format!("mailcal-other-{}", uuid::Uuid::now_v7());
        let owner_vault = AccountVault::for_agent(&owner_agent).unwrap();
        let other_vault = AccountVault::for_agent(&other_agent).unwrap();
        let ledger = ConnectionLedger::for_agent(&owner_agent).unwrap();
        let account_id = uuid::Uuid::now_v7().to_string();
        let redeemed = RedeemedAuthorization {
            account_id: account_id.clone(),
            provider: Provider::Google,
            agent_id: owner_agent.clone(),
            audiences: vec![format!("agent:{owner_agent}")],
            capabilities: vec![Capability::MailRead],
            granted_scopes: vec!["https://www.googleapis.com/auth/gmail.readonly".into()],
            access_token_expires_at: Utc::now() + chrono::Duration::minutes(30),
            refresh_token_available: true,
            secret_material: AccountSecretMaterial::new(
                "google:subject".into(),
                Some("owner@example.com".into()),
                Some("public-client".into()),
                Some("access-secret".into()),
                Some("refresh-secret".into()),
                None,
                None,
            )
            .unwrap(),
        };

        assert!(matches!(
            redeemed.persist(&other_vault, &ledger),
            Err(OAuthExchangeError::AgentMismatch)
        ));
        assert!(ledger.read_all().unwrap().is_empty());
        assert!(matches!(
            owner_vault.load(&account_id),
            Err(crate::vault::VaultError::Unavailable)
        ));
        assert!(matches!(
            other_vault.load(&account_id),
            Err(crate::vault::VaultError::Unavailable)
        ));
    }

    #[test]
    fn provider_principal_can_have_only_one_active_account_link_per_agent() {
        vak_config::paths::isolate_home_for_tests();
        let agent_id = format!("mailcal-unique-principal-{}", uuid::Uuid::now_v7());
        let vault = AccountVault::for_agent(&agent_id).unwrap();
        let ledger = ConnectionLedger::for_agent(&agent_id).unwrap();
        let make_redeemed = |provider, principal: &str, capability| RedeemedAuthorization {
            account_id: uuid::Uuid::now_v7().to_string(),
            provider,
            agent_id: agent_id.clone(),
            audiences: vec![format!("agent:{agent_id}")],
            capabilities: vec![capability],
            granted_scopes: scopes_for(provider, &[capability])
                .unwrap()
                .into_iter()
                .map(str::to_owned)
                .collect(),
            access_token_expires_at: Utc::now() + chrono::Duration::minutes(30),
            refresh_token_available: true,
            secret_material: AccountSecretMaterial::new(
                principal.to_owned(),
                Some("person@example.com".into()),
                Some("public-client".into()),
                Some("access-secret".into()),
                Some("refresh-secret".into()),
                None,
                None,
            )
            .unwrap(),
        };

        let first = make_redeemed(
            Provider::Google,
            "google:stable-subject",
            Capability::MailRead,
        );
        let first_account_id = first.account_id.clone();
        first.persist(&vault, &ledger).unwrap();

        let duplicate = make_redeemed(
            Provider::Google,
            "google:stable-subject",
            Capability::CalendarRead,
        );
        let duplicate_account_id = duplicate.account_id.clone();
        assert!(matches!(
            duplicate.persist(&vault, &ledger),
            Err(OAuthExchangeError::AccountAlreadyConnected)
        ));
        assert!(matches!(
            vault.load(&duplicate_account_id),
            Err(crate::vault::VaultError::Unavailable)
        ));

        let pending_id = uuid::Uuid::now_v7().to_string();
        let pending = ConnectedAccount {
            id: pending_id.clone(),
            provider: Provider::Google,
            status: AccountStatus::Pending,
            owner_agent_id: agent_id.clone(),
            allowed_audiences: [format!("agent:{agent_id}")].into_iter().collect(),
            capabilities: [Capability::MailRead].into_iter().collect(),
            provider_scopes: scopes_for(Provider::Google, &[Capability::MailRead])
                .unwrap()
                .into_iter()
                .map(str::to_owned)
                .collect(),
            credential_ref: AccountVault::credential_ref(&pending_id).unwrap(),
            principal_ref: AccountVault::credential_ref(&pending_id).unwrap(),
            revision: 1,
            connected_at: Utc::now(),
            access_token_expires_at: Some(Utc::now() + chrono::Duration::minutes(30)),
            refresh_token_available: true,
            revoked_at: None,
        };
        ledger.append_pending(pending).unwrap();
        let distinct = make_redeemed(
            Provider::Google,
            "google:another-subject",
            Capability::CalendarRead,
        );
        assert!(matches!(
            distinct.persist(&vault, &ledger),
            Err(OAuthExchangeError::AccountLinkInProgress)
        ));
        ledger.append_revoked(&pending_id, Utc::now()).unwrap();

        let distinct = make_redeemed(
            Provider::Google,
            "google:another-subject",
            Capability::CalendarRead,
        );
        distinct.persist(&vault, &ledger).unwrap();
        let accounts = ledger.read_all().unwrap();
        assert_eq!(
            accounts
                .iter()
                .filter(|account| {
                    account.status == AccountStatus::Connected && account.revoked_at.is_none()
                })
                .count(),
            2
        );
        assert!(vault.load(&first_account_id).is_ok());
    }

    #[test]
    fn authorization_fence_memory_is_bounded_and_eviction_fails_closed() {
        let store = AuthorizationStore::default();
        let (url, _) = store
            .begin_native(
                Provider::Google,
                "agent-0000",
                &["agent:agent-0000".to_owned()],
                &[Capability::MailRead],
                "vak-test-client",
                "http://127.0.0.1:43127/mail-calendar/oauth/callback",
            )
            .unwrap();
        let state = Url::parse(&url)
            .unwrap()
            .query_pairs()
            .find(|(name, _)| name == "state")
            .map(|(_, value)| value.into_owned())
            .unwrap();
        let grant = store.consume(&state, None, None).unwrap();
        assert!(store.is_current(&grant));

        for index in 0..=MAX_AUTHORIZATION_FENCES {
            store.cancel_provider(&format!("agent-{index:04}"), Provider::Google);
        }

        let authorization_state = store
            .state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        assert_eq!(authorization_state.fences.len(), MAX_AUTHORIZATION_FENCES);
        drop(authorization_state);
        assert!(!store.is_current(&grant));
        assert!(store.with_current(&grant, || ()).is_none());
    }

    #[test]
    fn authorization_uses_pkce_and_rejects_non_loopback_redirects() {
        let (store, url, callback_binding) = make_store();
        let parsed = Url::parse(&url).expect("authorization URL parses");
        let params = parsed.query_pairs().collect::<BTreeMap<_, _>>();
        assert_eq!(
            params.get("code_challenge_method").map(|v| v.as_ref()),
            Some("S256")
        );
        assert_eq!(
            params.get("response_type").map(|v| v.as_ref()),
            Some("code")
        );
        assert_eq!(
            params.get("include_granted_scopes").map(|v| v.as_ref()),
            Some("false")
        );
        let state = params.get("state").expect("state parameter").to_string();
        let expected_challenge = params
            .get("code_challenge")
            .expect("PKCE challenge")
            .to_string();
        let grant = store
            .consume(&state, Some("session-42"), Some(&callback_binding))
            .expect("authorization grant is available");
        let actual_challenge = base64::engine::general_purpose::URL_SAFE_NO_PAD
            .encode(Sha256::digest(grant.code_verifier().as_bytes()));
        assert_eq!(actual_challenge, expected_challenge);

        let rejected = store.begin(
            Provider::Google,
            "agent-7",
            Some("session-42"),
            &["conversation-1".to_owned()],
            &[Capability::MailRead],
            "vak-test-client",
            "https://attacker.example/callback",
            None,
        );
        assert!(matches!(rejected, Err(OAuthError::InvalidRequest)));
    }

    #[test]
    fn pending_authorizations_are_bounded_per_agent() {
        let store = AuthorizationStore::default();
        let mut callback_binding: Option<String> = None;
        for _ in 0..MAX_PENDING_PER_AGENT {
            let (_, _, binding) = store
                .begin(
                    Provider::Microsoft,
                    "agent-7",
                    Some("session-42"),
                    &["conversation-1".to_owned()],
                    &[Capability::CalendarRead],
                    "vak-test-client",
                    "http://127.0.0.1:43127/oauth/callback",
                    callback_binding.as_deref(),
                )
                .expect("attempt fits per-Agent bound");
            callback_binding = Some(binding);
        }
        let rejected = store.begin(
            Provider::Microsoft,
            "agent-7",
            Some("session-42"),
            &["conversation-1".to_owned()],
            &[Capability::CalendarRead],
            "vak-test-client",
            "http://127.0.0.1:43127/oauth/callback",
            callback_binding.as_deref(),
        );
        assert!(matches!(rejected, Err(OAuthError::AtCapacity)));
    }

    #[test]
    fn callback_binding_can_be_reused_only_by_the_same_pending_browser_session() {
        let store = AuthorizationStore::default();
        let (first_url, _, binding) = store
            .begin(
                Provider::Google,
                "agent-7",
                Some("session-42"),
                &["conversation-1".to_owned()],
                &[Capability::MailRead],
                "vak-test-client",
                "http://127.0.0.1:43127/oauth/callback",
                None,
            )
            .expect("first flow gets a browser binding");
        let (second_url, _, reused_binding) = store
            .begin(
                Provider::Microsoft,
                "agent-7",
                Some("session-42"),
                &["conversation-1".to_owned()],
                &[Capability::CalendarRead],
                "vak-test-client",
                "http://127.0.0.1:43127/oauth/callback",
                Some(&binding),
            )
            .expect("same browser reuses its pending binding");
        assert_eq!(binding, reused_binding);
        for authorization_url in [first_url, second_url] {
            let state = Url::parse(&authorization_url)
                .expect("authorization URL parses")
                .query_pairs()
                .find(|(name, _)| name == "state")
                .map(|(_, value)| value.into_owned())
                .expect("state is present");
            assert!(store.consume(&state, None, Some(&binding)).is_some());
        }
    }

    #[test]
    fn provider_scopes_follow_the_requested_capability() {
        let freebusy = scopes_for(Provider::Google, &[Capability::CalendarFreeBusy])
            .expect("Google free-busy scope");
        assert!(freebusy.contains(&"https://www.googleapis.com/auth/calendar.freebusy"));
        assert!(!freebusy.contains(&"https://www.googleapis.com/auth/calendar.readonly"));
        let microsoft_freebusy = scopes_for(Provider::Microsoft, &[Capability::CalendarFreeBusy])
            .expect("Microsoft free-busy scope");
        assert!(microsoft_freebusy.contains(&"Calendars.ReadBasic"));
        assert!(!microsoft_freebusy.contains(&"Calendars.Read"));
        let microsoft_calendar = scopes_for(Provider::Microsoft, &[Capability::CalendarRead])
            .expect("Microsoft event-read scope");
        assert!(microsoft_calendar.contains(&"Calendars.Read"));
        assert!(!microsoft_calendar.contains(&"Calendars.ReadBasic"));
        let microsoft_both = scopes_for(
            Provider::Microsoft,
            &[Capability::CalendarFreeBusy, Capability::CalendarRead],
        )
        .expect("Microsoft free-busy and event-read scopes");
        assert!(microsoft_both.contains(&"Calendars.ReadBasic"));
        assert!(microsoft_both.contains(&"Calendars.Read"));
        let mail = scopes_for(Provider::Microsoft, &[Capability::MailRead])
            .expect("Microsoft delegated mail scope");
        assert!(mail.contains(&"Mail.Read"));
        assert!(!mail.contains(&"Mail.ReadWrite"));
    }

    #[test]
    fn account_linking_rejects_provider_write_and_prepare_capabilities() {
        for provider in [Provider::Google, Provider::Microsoft] {
            for capability in [
                Capability::MailSend,
                Capability::MailPrepare,
                Capability::CalendarWrite,
            ] {
                assert!(scopes_for(provider, &[Capability::MailRead, capability]).is_err());
            }
        }
    }

    #[test]
    fn refresh_scope_check_never_drops_a_requested_data_capability() {
        let requested = vec![
            "openid".to_owned(),
            "Mail.Read".to_owned(),
            "Calendars.Read".to_owned(),
        ];
        let complete = vec![
            "openid".to_owned(),
            "https://graph.microsoft.com/Mail.Read".to_owned(),
            "https://graph.microsoft.com/Calendars.Read".to_owned(),
        ];
        assert!(requested_scopes_granted(
            Provider::Microsoft,
            &requested,
            &complete
        ));
        let incomplete = vec!["openid".to_owned(), "Mail.Read".to_owned()];
        assert!(!requested_scopes_granted(
            Provider::Microsoft,
            &requested,
            &incomplete
        ));
        assert!(!requested_scopes_granted(
            Provider::Google,
            &["https://www.googleapis.com/auth/gmail.readonly".to_owned()],
            &[]
        ));
    }

    #[test]
    fn provider_grants_cannot_exceed_the_selected_capabilities() {
        let google_requested = vec![
            "openid".to_owned(),
            "email".to_owned(),
            "https://www.googleapis.com/auth/gmail.readonly".to_owned(),
        ];
        assert!(granted_scopes_are_bounded(
            Provider::Google,
            &google_requested,
            &[
                "openid".to_owned(),
                "email".to_owned(),
                "https://www.googleapis.com/auth/gmail.readonly".to_owned(),
            ],
        ));
        assert!(!granted_scopes_are_bounded(
            Provider::Google,
            &google_requested,
            &[
                "openid".to_owned(),
                "email".to_owned(),
                "https://www.googleapis.com/auth/gmail.readonly".to_owned(),
                "https://www.googleapis.com/auth/gmail.modify".to_owned(),
            ],
        ));

        let microsoft_requested = vec!["Calendars.ReadBasic".to_owned()];
        assert!(granted_scopes_are_bounded(
            Provider::Microsoft,
            &microsoft_requested,
            &["https://graph.microsoft.com/Calendars.ReadBasic".to_owned()],
        ));
        assert!(!granted_scopes_are_bounded(
            Provider::Microsoft,
            &microsoft_requested,
            &[
                "https://graph.microsoft.com/Calendars.ReadBasic".to_owned(),
                "Calendars.ReadWrite".to_owned(),
            ],
        ));
    }

    #[test]
    fn refreshed_tokens_change_the_vault_only_after_explicit_persist() {
        use crate::vault::{AccountSecretMaterial, AccountVault};
        use std::collections::BTreeSet;

        vak_config::paths::isolate_home_for_tests();
        let agent_id = format!("refresh-agent-{}", uuid::Uuid::now_v7());
        let account_id = uuid::Uuid::now_v7().to_string();
        let vault = AccountVault::for_agent(&agent_id).unwrap();
        vault
            .store(
                &account_id,
                AccountSecretMaterial::new(
                    "google:subject".to_owned(),
                    Some("person@example.com".to_owned()),
                    Some("desktop-client".to_owned()),
                    Some("old-access".to_owned()),
                    Some("old-refresh".to_owned()),
                    None,
                    None,
                )
                .unwrap(),
            )
            .unwrap();
        let account = ConnectedAccount {
            id: account_id.clone(),
            provider: Provider::Google,
            status: AccountStatus::Connected,
            owner_agent_id: agent_id.clone(),
            allowed_audiences: BTreeSet::from([format!("agent:{agent_id}")]),
            capabilities: BTreeSet::from([Capability::MailRead]),
            provider_scopes: BTreeSet::from([
                "https://www.googleapis.com/auth/gmail.readonly".to_owned()
            ]),
            credential_ref: AccountVault::credential_ref(&account_id).unwrap(),
            principal_ref: AccountVault::credential_ref(&account_id).unwrap(),
            revision: 1,
            connected_at: Utc::now(),
            access_token_expires_at: Some(Utc::now() + chrono::Duration::minutes(30)),
            refresh_token_available: true,
            revoked_at: None,
        };
        let encoded_secret = Zeroizing::new(
            vak_config::read_env_file_var(
                &vak_config::paths::agent_home(&agent_id).join(".env"),
                &AccountVault::credential_ref(&account_id).unwrap(),
            )
            .unwrap(),
        );
        let before: serde_json::Value = serde_json::from_str(&encoded_secret).unwrap();
        assert_eq!(before["access_token"], "old-access");
        assert_eq!(before["refresh_token"], "old-refresh");

        let rotated = RefreshedOAuthTokens {
            expires_at: Utc::now() + chrono::Duration::hours(1),
            access_token: Zeroizing::new("new-access".to_owned()),
            refresh_token: Some(Zeroizing::new("new-refresh".to_owned())),
        };
        drop(rotated);
        let still_old = Zeroizing::new(
            vak_config::read_env_file_var(
                &vak_config::paths::agent_home(&agent_id).join(".env"),
                &AccountVault::credential_ref(&account_id).unwrap(),
            )
            .unwrap(),
        );
        let still_old: serde_json::Value = serde_json::from_str(&still_old).unwrap();
        assert_eq!(still_old["access_token"], "old-access");
        assert_eq!(still_old["refresh_token"], "old-refresh");

        let rotated = RefreshedOAuthTokens {
            expires_at: Utc::now() + chrono::Duration::hours(1),
            access_token: Zeroizing::new("new-access".to_owned()),
            refresh_token: Some(Zeroizing::new("new-refresh".to_owned())),
        };
        rotated.persist(&vault, &account).unwrap();
        let saved = Zeroizing::new(
            vak_config::read_env_file_var(
                &vak_config::paths::agent_home(&agent_id).join(".env"),
                &AccountVault::credential_ref(&account_id).unwrap(),
            )
            .unwrap(),
        );
        let saved: serde_json::Value = serde_json::from_str(&saved).unwrap();
        assert_eq!(saved["access_token"], "new-access");
        assert_eq!(saved["refresh_token"], "new-refresh");
    }
}
