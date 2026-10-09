//! Owner-facing account linking for the mail and calendar package.
//!
//! Owner-facing account, bounded read-preview, and local-candidate routes. All
//! data operations remain Agent-scoped; provider writes are a separate gate.

use crate::{AppState, AuthenticatedPrincipal, agent_chats, agents};
use axum::{
    Json,
    body::Bytes,
    extract::{ConnectInfo, OriginalUri, Path, Query, State},
    http::{HeaderMap, StatusCode, header},
    response::{Html, IntoResponse, Response},
};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;
use std::net::SocketAddr;
use uuid::Uuid;
use vak_mail_calendar::{
    AccountStatus, ActionCandidate, ActionState, CandidateApproval, Capability, ConnectedAccount,
    LOCAL_DRAFT_ACCOUNT_ID, ProposedAction, Provider, RoutineOperation, RoutineScope, SourceRef,
    connection_ledger::ConnectionLedger,
    effect::{ProviderEffectClient, ProviderEffectError},
    provider::ProviderReadClient,
    vault::{AccountSecretMaterial, AccountVault},
};
use zeroize::{Zeroize, Zeroizing};

const OAUTH_CALLBACK_COOKIE: &str = "vak_mail_calendar_oauth";
const OAUTH_CALLBACK_COOKIE_MAX_AGE_SECONDS: u64 = 10 * 60;

pub(crate) async fn validate_routine_scope(
    agent_id: &str,
    scope: &RoutineScope,
    worker_exe: &std::path::Path,
) -> Result<(), String> {
    scope.validate().map_err(|error| error.to_string())?;
    let ledger = ConnectionLedger::for_agent(agent_id)
        .map_err(|_| "mail/calendar connection state is unavailable for this Agent".to_string())?;
    let accounts = ledger
        .read_all()
        .map_err(|_| "mail/calendar connection state is unavailable for this Agent".to_string())?;
    let audience = format!("agent:{agent_id}");
    let Some(account) = accounts
        .into_iter()
        .find(|account| account.id == scope.account_id)
    else {
        return Err("the selected mail/calendar account is not linked to this Agent".into());
    };
    for operation in &scope.operations {
        let capability = match operation {
            RoutineOperation::RecentMail => Capability::MailRead,
            RoutineOperation::MailThread => Capability::MailRead,
            RoutineOperation::CalendarEvents => Capability::CalendarRead,
            RoutineOperation::FreeBusy => Capability::CalendarFreeBusy,
        };
        if !account.admits(agent_id, &audience, capability) {
            return Err("the selected account does not grant every requested routine read".into());
        }
        if operation == &RoutineOperation::MailThread && account.provider == Provider::AppleIcloud {
            return Err("conversation reads are not available for Apple accounts".into());
        }
    }
    let vault = AccountVault::for_agent(agent_id)
        .map_err(|_| "mail/calendar credentials are unavailable for this Agent".to_string())?;
    if !vault.credential_available(&scope.account_id) {
        return Err("the selected account's saved credential is unavailable".into());
    }
    if matches!(account.provider, Provider::Google | Provider::Microsoft)
        && vault.has_app_password(&account.id)
        && scope.operations.contains(&RoutineOperation::MailThread)
    {
        return Err(
            "App Password accounts support selected-message reads, not conversation reads".into(),
        );
    }
    if let Some(folder_id) = scope.mail_folder_id.as_deref() {
        let folders = ProviderReadClient::new()
            .list_mail_folders(&account, &vault, agent_id, &audience)
            .await
            .map_err(|_| "the selected mail folder could not be verified".to_string())?;
        if !folders.iter().any(|folder| folder.provider_id == folder_id) {
            return Err("the selected mail folder does not belong to this account".into());
        }
    }
    if let Some(source_id) = scope.calendar_source_id.as_deref() {
        let sources = vak_core::mail_calendar::calendar_sources_with_worker(
            &ProviderReadClient::new(),
            &account,
            &vault,
            agent_id,
            &audience,
            worker_exe,
        )
        .await
        .map_err(|_| "the selected calendar could not be verified".to_string())?;
        if !sources.iter().any(|source| source.provider_id == source_id) {
            return Err("the selected calendar does not belong to this account".into());
        }
    }
    // Folder or calendar discovery can take a provider round-trip. Do not persist a
    // routine against an account that was revoked or narrowed during it.
    let still_admitted = ledger.read_all().ok().is_some_and(|latest| {
        latest.iter().any(|current| {
            current.id == account.id
                && current.revision == account.revision
                && scope.operations.iter().all(|operation| {
                    let capability = match operation {
                        RoutineOperation::RecentMail | RoutineOperation::MailThread => {
                            Capability::MailRead
                        }
                        RoutineOperation::CalendarEvents => Capability::CalendarRead,
                        RoutineOperation::FreeBusy => Capability::CalendarFreeBusy,
                    };
                    current.admits(agent_id, &audience, capability)
                })
        })
    });
    if !still_admitted || !vault.credential_available(&scope.account_id) {
        return Err("the selected account changed while the routine was being checked".into());
    }
    Ok(())
}

/// Poll only the configured account's bounded recent-mail window. The
/// scheduler uses this content-free result to avoid invoking a model when a
/// watch has no unseen message IDs; the actual content is read again by the
/// brokered tool after a run starts.
pub(crate) async fn mail_watch_has_unseen(
    agent_id: &str,
    scope: &RoutineScope,
    previous_run_succeeded: bool,
) -> Result<Option<bool>, String> {
    scope.validate().map_err(|error| error.to_string())?;
    if !scope.watch_new_mail || !scope.operations.contains(&RoutineOperation::RecentMail) {
        return Err("the scheduled routine is not a mail watch".into());
    }
    let ledger = ConnectionLedger::for_agent(agent_id)
        .map_err(|_| "mail/calendar connection state is unavailable".to_string())?;
    let accounts = ledger
        .read_all()
        .map_err(|_| "mail/calendar connection state is unavailable".to_string())?;
    let audience = format!("agent:{agent_id}");
    let account = accounts
        .into_iter()
        .find(|account| {
            account.id == scope.account_id
                && account.admits(agent_id, &audience, Capability::MailRead)
        })
        .ok_or_else(|| "the selected mail account is no longer authorized".to_string())?;
    let vault = AccountVault::for_agent(agent_id)
        .map_err(|_| "mail/calendar credentials are unavailable".to_string())?;
    vault
        .resolve_delivered_mail_ids(&scope.routine_id, &scope.account_id, previous_run_succeeded)
        .map_err(|_| "the private mail watch cursor is unavailable".to_string())?;
    if vault
        .has_unresolved_mail_ids(&scope.routine_id, &scope.account_id)
        .map_err(|_| "the private mail watch cursor is unavailable".to_string())?
    {
        // Work is already durably queued. Let the run consume it without
        // claiming that this tick contacted or refreshed the provider.
        return Ok(None);
    }
    let cursor = vault
        .routine_provider_cursor(&scope.routine_id, &scope.account_id)
        .map_err(|_| "the private mail watch cursor is unavailable".to_string())?;
    let (item_ids, next_cursor) = ProviderReadClient::new()
        .mail_watch_page(
            &account,
            &vault,
            agent_id,
            &audience,
            cursor.as_deref(),
            vak_mail_calendar::MAX_ROUTINE_MAIL_BACKLOG,
        )
        .await
        .map_err(|error| match error {
            vak_mail_calendar::provider::ProviderReadError::WatchCursorReset => {
                "the provider watch cursor expired or reset; delete and recreate this watch to establish a fresh cursor, which may leave a gap".to_string()
            }
            _ => "the mail watch could not check its bounded provider window".to_string(),
        })?;
    let still_authorized = ledger.read_all().ok().is_some_and(|latest| {
        latest.iter().any(|current| {
            current.id == account.id
                && current.revision == account.revision
                && current.admits(agent_id, &audience, Capability::MailRead)
        })
    });
    if !still_authorized {
        return Err("the mail account changed during the watch check".into());
    }
    vault
        .queue_mail_ids_with_cursor(
            &scope.routine_id,
            &scope.account_id,
            &item_ids,
            next_cursor.as_deref(),
        )
        .map(Some)
        .map_err(|_| "the private mail watch cursor is unavailable".to_string())
}

/// Poll the configured calendar window and durably queue only event
/// occurrences whose configured boundary has become due. Provider content is
/// parsed by the same isolated worker used by the brokered calendar tool.
pub(crate) async fn calendar_event_has_due(
    agent_id: &str,
    scope: &RoutineScope,
    last_check_at: Option<DateTime<Utc>>,
    previous_run_succeeded: bool,
    worker_exe: &std::path::Path,
) -> Result<Option<bool>, String> {
    scope.validate().map_err(|error| error.to_string())?;
    let Some(trigger) = scope.calendar_event_trigger else {
        return Err("the scheduled routine has no calendar event trigger".into());
    };
    let ledger = ConnectionLedger::for_agent(agent_id)
        .map_err(|_| "mail/calendar connection state is unavailable".to_string())?;
    let accounts = ledger
        .read_all()
        .map_err(|_| "mail/calendar connection state is unavailable".to_string())?;
    let audience = format!("agent:{agent_id}");
    let account = accounts
        .into_iter()
        .find(|account| {
            account.id == scope.account_id
                && account.admits(agent_id, &audience, Capability::CalendarRead)
        })
        .ok_or_else(|| "the selected calendar account is no longer authorized".to_string())?;
    let vault = AccountVault::for_agent(agent_id)
        .map_err(|_| "calendar credentials are unavailable".to_string())?;
    vault
        .resolve_delivered_calendar_occurrences(
            &scope.routine_id,
            &scope.account_id,
            previous_run_succeeded,
        )
        .map_err(|_| "the private calendar occurrence queue is unavailable".to_string())?;
    if vault
        .has_unresolved_calendar_occurrences(&scope.routine_id, &scope.account_id)
        .map_err(|_| "the private calendar occurrence queue is unavailable".to_string())?
    {
        // Keep retry candidates durable until the brokered run can fetch and
        // stage the exact event. Do not advance its polling window here.
        return Ok(None);
    }
    let now = Utc::now();
    let catch_up_floor = now - chrono::Duration::minutes(i64::from(trigger.max_lateness_minutes));
    let checked_after = last_check_at.unwrap_or(catch_up_floor).max(catch_up_floor);
    let offset = chrono::Duration::minutes(i64::from(trigger.offset_minutes));
    let range = vak_mail_calendar::provider::CalendarRange {
        from: checked_after - offset - chrono::Duration::minutes(1),
        to: now - offset + chrono::Duration::minutes(1),
        // Scan to the durable occurrence queue's bound independently of the
        // Agent run's smaller output budget. Otherwise a full first page can
        // hide due events permanently when each later poll starts at page one.
        limit: vak_mail_calendar::MAX_ROUTINE_MAIL_BACKLOG,
    };
    if range.from >= range.to {
        return Ok(Some(false));
    }
    let client = ProviderReadClient::new();
    let page = if let Some(source_id) = scope.calendar_source_id.as_deref() {
        vak_core::mail_calendar::calendar_event_page_in_source_with_worker(
            &client, &account, &vault, agent_id, &audience, source_id, range, worker_exe,
        )
        .await
    } else {
        vak_core::mail_calendar::calendar_event_page_with_worker(
            &client, &account, &vault, agent_id, &audience, range, worker_exe,
        )
        .await
    }
    .map_err(|_| {
        "the calendar event trigger could not check its bounded provider window".to_string()
    })?;
    if page.has_more {
        return Err(
            "the calendar event trigger found more events than its bounded scan can safely reconcile; narrow its catch-up window".into(),
        );
    }
    let still_authorized = ledger.read_all().ok().is_some_and(|latest| {
        latest.iter().any(|current| {
            current.id == account.id
                && current.revision == account.revision
                && current.admits(agent_id, &audience, Capability::CalendarRead)
        })
    });
    if !still_authorized {
        return Err("the calendar account changed during the trigger check".into());
    }
    let keys = due_calendar_occurrence_keys(&page.events, trigger, checked_after, now);
    vault
        .reconcile_calendar_occurrences(&scope.routine_id, &scope.account_id, &keys)
        .map(Some)
        .map_err(|_| "the private calendar occurrence queue is unavailable".to_string())
}

/// Keep OAuth-backed unattended routines usable across access-token expiry.
/// The account lock is shared with explicit owner refresh and disconnect so a
/// refresh-token rotation cannot race a second refresh or a credential purge.
/// Apple app passwords have no access-token expiry and pass through untouched.
async fn refresh_routine_account_if_needed(
    state: &AppState,
    agent_id: &str,
    account_id: &str,
) -> Result<ConnectedAccount, String> {
    const REFRESH_AHEAD: chrono::Duration = chrono::Duration::seconds(300);

    let lock = state.mail_calendar_account_lock(agent_id, account_id);
    let _guard = lock.lock().await;
    if !valid_agent(state, agent_id) {
        return Err("the owning Agent is unavailable".into());
    }
    let ledger = ConnectionLedger::for_agent(agent_id)
        .map_err(|_| "mail/calendar connection state is unavailable".to_string())?;
    let vault = AccountVault::for_agent(agent_id)
        .map_err(|_| "mail/calendar credentials are unavailable".to_string())?;
    let Some(_refresh_lease) = vault
        .try_acquire_account_refresh_lease(account_id)
        .map_err(|_| "mail/calendar credentials are unavailable".to_string())?
    else {
        return Err("another local service is refreshing this account; retry shortly".into());
    };
    let account = ledger
        .read_all()
        .map_err(|_| "mail/calendar connection state is unavailable".to_string())?
        .into_iter()
        .find(|account| {
            account.id == account_id
                && account.owner_agent_id == agent_id
                && account.status == AccountStatus::Connected
                && account.revoked_at.is_none()
        })
        .ok_or_else(|| "the selected account is no longer connected".to_string())?;
    let Some(expires_at) = account.access_token_expires_at else {
        return Ok(account);
    };
    if expires_at > Utc::now() + REFRESH_AHEAD {
        return Ok(account);
    }

    let require_reconnection = || {
        if ledger
            .append_reauthentication_required(account_id, Utc::now())
            .is_ok()
        {
            pause_routines_for_account(state, agent_id, account_id);
            record_account_event(
                state,
                "account_reauthentication_required",
                agent_id,
                account_id,
                account.provider,
                &account.capabilities,
                "routine_token_refresh_rejected",
            );
        }
    };
    if !account.refresh_token_available || account.provider == Provider::AppleIcloud {
        require_reconnection();
        return Err("this account needs to be reconnected before its routine can continue".into());
    }
    #[cfg(feature = "test-support")]
    let refresh_result = {
        let test_endpoint = state
            .mail_calendar_test_refresh_endpoints
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .get(&account.provider)
            .cloned();
        match test_endpoint {
            Some(endpoint) => {
                vak_mail_calendar::oauth::refresh_account_tokens_from_loopback_test_endpoint(
                    &vault, &account, &endpoint,
                )
                .await
            }
            None => vak_mail_calendar::oauth::refresh_account_tokens(&vault, &account).await,
        }
    };
    #[cfg(not(feature = "test-support"))]
    let refresh_result = vak_mail_calendar::oauth::refresh_account_tokens(&vault, &account).await;
    let rotated = match refresh_result {
        Ok(tokens) => tokens,
        Err(vak_mail_calendar::oauth::OAuthRefreshError::ReconnectRequired) => {
            require_reconnection();
            return Err(
                "this account needs to be reconnected before its routine can continue".into(),
            );
        }
        Err(vak_mail_calendar::oauth::OAuthRefreshError::Vault(_)) => {
            return Err("mail/calendar credentials are unavailable".into());
        }
        Err(_) => return Err("the provider could not refresh this routine's sign-in".into()),
    };
    if !valid_agent(state, agent_id) {
        return Err("the owning Agent is unavailable".into());
    }
    let Some(revision) = account.revision.checked_add(1) else {
        return Err("the account changed while its sign-in was refreshing".into());
    };
    let mut refreshed = account.clone();
    refreshed.revision = revision;
    refreshed.access_token_expires_at = Some(rotated.expires_at());
    let commit = ledger.append_connected_if_current(refreshed.clone(), account.revision, || {
        if !valid_agent(state, agent_id) {
            return Err(RefreshCommitError::AgentInactive);
        }
        rotated
            .persist(&vault, &account)
            .map(|_| ())
            .map_err(RefreshCommitError::Credential)
    });
    if commit.is_err() {
        return Err("the account changed while its sign-in was refreshing".into());
    }
    record_account_event(
        state,
        "account_refreshed",
        agent_id,
        account_id,
        refreshed.provider,
        &refreshed.capabilities,
        "routine_token_refresh",
    );
    Ok(refreshed)
}

pub(crate) async fn prepare_routine_account(
    state: &AppState,
    agent_id: &str,
    scope: &RoutineScope,
) -> Result<(), String> {
    scope.validate().map_err(|error| error.to_string())?;
    let account = refresh_routine_account_if_needed(state, agent_id, &scope.account_id).await?;
    let audience = format!("agent:{agent_id}");
    let authorized = scope.operations.iter().all(|operation| {
        let capability = match operation {
            RoutineOperation::RecentMail | RoutineOperation::MailThread => Capability::MailRead,
            RoutineOperation::CalendarEvents => Capability::CalendarRead,
            RoutineOperation::FreeBusy => Capability::CalendarFreeBusy,
        };
        account.admits(agent_id, &audience, capability)
    });
    if !authorized {
        return Err("the account no longer grants every operation in this routine".into());
    }
    let vault = AccountVault::for_agent(agent_id)
        .map_err(|_| "mail/calendar credentials are unavailable".to_string())?;
    if !vault.credential_available(&scope.account_id) {
        return Err("the linked account credential is unavailable".into());
    }
    Ok(())
}

fn due_calendar_occurrence_keys(
    events: &[vak_mail_calendar::provider::CalendarItem],
    trigger: vak_mail_calendar::CalendarEventTrigger,
    checked_after: DateTime<Utc>,
    now: DateTime<Utc>,
) -> Vec<String> {
    events
        .iter()
        .filter_map(|event| {
            let (Some(starts_at), Some(ends_at)) = (event.starts_at, event.ends_at) else {
                return None;
            };
            trigger
                .is_due_between(starts_at, ends_at, checked_after, now)
                .then(|| trigger.occurrence_key(&event.provider_id, starts_at, ends_at))
        })
        .collect()
}

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
    auth_method: Option<&'static str>,
    credential_available: bool,
    superseded_by_active_link: bool,
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
            let account_views = accounts
                .iter()
                .map(|account| {
                    // Do not decrypt every account credential into a retained
                    // collection just to render owner metadata. At most the
                    // candidate and one active replacement are live together.
                    let credential = if account.revoked_at.is_some()
                        || account.status == AccountStatus::Pending
                    {
                        Err(vak_mail_calendar::vault::VaultError::Unavailable)
                    } else {
                        vault.load(&account.id)
                    };
                    let superseded_by_active_link = account.status
                        == AccountStatus::ReauthenticationRequired
                        && credential.as_ref().is_ok_and(|candidate| {
                            accounts.iter().any(|other| {
                                if other.id == account.id
                                    || other.provider != account.provider
                                    || other.status != AccountStatus::Connected
                                    || other.revoked_at.is_some()
                                {
                                    return false;
                                }
                                vault.load(&other.id).is_ok_and(|other_secret| {
                                    same_provider_principal(
                                        account.provider,
                                        candidate,
                                        &other_secret,
                                    )
                                })
                            })
                        });
                    AccountView {
                        id: account.id.clone(),
                        provider: account.provider,
                        status: account.status,
                        identity_masked: credential
                            .as_ref()
                            .ok()
                            .and_then(AccountSecretMaterial::masked_display_identity),
                        auth_method: credential.as_ref().ok().map(|material| {
                            if material.uses_app_password() {
                                "app_password"
                            } else {
                                "oauth"
                            }
                        }),
                        credential_available: credential.is_ok(),
                        superseded_by_active_link,
                        capabilities: account.capabilities.iter().copied().collect(),
                        connected_at: account.connected_at,
                        access_token_expires_at: account.access_token_expires_at,
                        refresh_token_available: account.refresh_token_available,
                        revoked_at: account.revoked_at,
                    }
                })
                .collect::<Vec<_>>();
            Json(serde_json::json!({
                "accounts": account_views
            }))
            .into_response()
        }
        Err(_) => StatusCode::SERVICE_UNAVAILABLE.into_response(),
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct CandidateWriteRequest {
    account_id: String,
    candidate_id: Option<String>,
    expected_revision: Option<u64>,
    source_refs: Option<Vec<SourceRef>>,
    action: ProposedAction,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct CandidateDeleteRequest {
    expected_revision: u64,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct SendCandidateRequest {
    expected_revision: u64,
    candidate_digest: String,
    /// Set only by the owner's explicit exact-payload confirmation control.
    confirm: bool,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct CandidateReviewContextRequest {
    expected_revision: u64,
    candidate_digest: String,
}

pub(super) async fn list_candidates(
    State(state): State<AppState>,
    axum::Extension(principal): axum::Extension<AuthenticatedPrincipal>,
    Path(agent_id): Path<String>,
) -> Response {
    if !operator(&principal) {
        return StatusCode::FORBIDDEN.into_response();
    }
    if !registered_agent(&state, &agent_id) {
        return StatusCode::NOT_FOUND.into_response();
    }
    let vault = match AccountVault::for_agent(&agent_id) {
        Ok(vault) => vault,
        Err(_) => return StatusCode::SERVICE_UNAVAILABLE.into_response(),
    };
    let effects = match state.core.effects().list() {
        Ok(effects) => effects,
        Err(_) => return StatusCode::SERVICE_UNAVAILABLE.into_response(),
    };
    match vault.list_candidates() {
        Ok(candidates) => Json(serde_json::json!({
            "candidates": candidates.into_iter().filter_map(|candidate| {
                let target = vak_mail_calendar::effect::candidate_target(&candidate.id);
                let state = effects
                    .iter()
                    .find(|effect| {
                        effect.target == target
                            && effect.status != vak_session::effects::EffectStatus::Superseded
                    })
                    .map(vak_mail_calendar::effect::action_state);
                candidate_view(candidate, state)
            }).collect::<Vec<_>>()
        }))
        .into_response(),
        Err(_) => StatusCode::SERVICE_UNAVAILABLE.into_response(),
    }
}

/// Return exact sender and source-header context for the owner's explicit
/// Review. Full account identity is transient here; it is never stored with
/// the candidate or returned to a model-facing tool.
pub(super) async fn candidate_review_context(
    State(state): State<AppState>,
    axum::Extension(principal): axum::Extension<AuthenticatedPrincipal>,
    Path((agent_id, candidate_id)): Path<(String, String)>,
    Json(request): Json<CandidateReviewContextRequest>,
) -> Response {
    if !operator(&principal) {
        return StatusCode::FORBIDDEN.into_response();
    }
    if !registered_agent(&state, &agent_id) {
        return StatusCode::NOT_FOUND.into_response();
    }
    if request.candidate_digest.len() != 64
        || !request
            .candidate_digest
            .bytes()
            .all(|byte| byte.is_ascii_hexdigit())
    {
        return StatusCode::BAD_REQUEST.into_response();
    }
    let vault = match AccountVault::for_agent(&agent_id) {
        Ok(vault) => vault,
        Err(_) => return StatusCode::SERVICE_UNAVAILABLE.into_response(),
    };
    let initial = match vault.list_candidates() {
        Ok(candidates) => candidates
            .into_iter()
            .find(|candidate| candidate.id == candidate_id && candidate.agent_id == agent_id),
        Err(_) => return StatusCode::SERVICE_UNAVAILABLE.into_response(),
    };
    let Some(initial) = initial else {
        return StatusCode::NOT_FOUND.into_response();
    };
    let account_lock = state.mail_calendar_account_lock(&agent_id, &initial.account_id);
    let _account_guard = account_lock.lock().await;
    if !valid_agent(&state, &agent_id) {
        return StatusCode::CONFLICT.into_response();
    }
    let candidate = match vault.list_candidates() {
        Ok(candidates) => candidates
            .into_iter()
            .find(|candidate| candidate.id == candidate_id && candidate.agent_id == agent_id),
        Err(_) => return StatusCode::SERVICE_UNAVAILABLE.into_response(),
    };
    let Some(candidate) = candidate else {
        return StatusCode::CONFLICT.into_response();
    };
    if candidate.account_id != initial.account_id
        || candidate.audience_id != format!("agent:{agent_id}")
        || candidate.revision != request.expected_revision
        || !candidate
            .digest()
            .is_ok_and(|digest| digest == request.candidate_digest)
    {
        return (
            StatusCode::CONFLICT,
            "The draft changed. Reload and review it again.",
        )
            .into_response();
    }
    let ProposedAction::SendMail { draft } = &candidate.action else {
        return StatusCode::BAD_REQUEST.into_response();
    };
    if draft.from_alias.is_some() || vak_mail_calendar::effect::validate_mail_draft(draft).is_err()
    {
        return (
            StatusCode::BAD_REQUEST,
            "This sender profile is not supported.",
        )
            .into_response();
    }
    let attempted = match candidate_effect(&state, &candidate.id) {
        Ok(effect) => effect.is_some(),
        Err(status) => return status.into_response(),
    };
    if attempted {
        return (
            StatusCode::CONFLICT,
            "This draft already has a provider-action outcome.",
        )
            .into_response();
    }
    let ledger = match ConnectionLedger::for_agent(&agent_id) {
        Ok(ledger) => ledger,
        Err(_) => return StatusCode::SERVICE_UNAVAILABLE.into_response(),
    };
    let account = match ledger.read_all() {
        Ok(accounts) => accounts
            .into_iter()
            .find(|account| account.id == candidate.account_id),
        Err(_) => return StatusCode::SERVICE_UNAVAILABLE.into_response(),
    };
    let Some(account) = account else {
        return StatusCode::NOT_FOUND.into_response();
    };
    let audience = format!("agent:{agent_id}");
    if !matches!(account.provider, Provider::Google | Provider::Microsoft)
        || !account.admits(&agent_id, &audience, Capability::MailSend)
        || (draft.reply_to_message_id.is_some()
            && !account.admits(&agent_id, &audience, Capability::MailRead))
    {
        return StatusCode::FORBIDDEN.into_response();
    }
    if !valid_agent(&state, &agent_id) {
        return StatusCode::CONFLICT.into_response();
    }
    let sender = match vault.display_identity(&account.id) {
        Ok(Some(sender)) if valid_sender_identity(&sender) => sender,
        _ => {
            return (
                StatusCode::PRECONDITION_FAILED,
                "The connected account has no verified sender identity for Review.",
            )
                .into_response();
        }
    };

    let source = match (
        draft.reply_to_message_id.as_deref(),
        draft.reply_to_thread_id.as_deref(),
    ) {
        (None, None) => None,
        (Some(message_id), Some(thread_id)) => {
            if !candidate
                .source_refs
                .iter()
                .any(|source| source.item_id == message_id)
            {
                return (
                    StatusCode::BAD_REQUEST,
                    "The reply source does not match this draft.",
                )
                    .into_response();
            }
            record_account_event(
                &state,
                "mail_reply_review_context",
                &agent_id,
                &account.id,
                account.provider,
                &account.capabilities,
                "requested",
            );
            match ProviderReadClient::default()
                .mail_reply_headers(
                    &account, &vault, &agent_id, &audience, message_id, thread_id,
                )
                .await
            {
                Ok(headers) => {
                    record_account_event(
                        &state,
                        "mail_reply_review_context",
                        &agent_id,
                        &account.id,
                        account.provider,
                        &account.capabilities,
                        "succeeded",
                    );
                    Some(headers)
                }
                Err(error) => {
                    mark_preview_reauthentication(&state, &account, &error);
                    record_account_event(
                        &state,
                        "mail_reply_review_context",
                        &agent_id,
                        &account.id,
                        account.provider,
                        &account.capabilities,
                        "failed",
                    );
                    return provider_preview_error(error);
                }
            }
        }
        _ => return StatusCode::BAD_REQUEST.into_response(),
    };
    Json(serde_json::json!({
        "candidate_id": candidate.id,
        "revision": candidate.revision,
        "sender": sender.as_str(),
        "source_from": source.as_ref().and_then(|headers| headers.from.as_deref()),
        "source_reply_to": source.as_ref().and_then(|headers| headers.reply_to.as_deref()),
    }))
    .into_response()
}

fn valid_sender_identity(identity: &str) -> bool {
    identity.len() <= 320
        && !identity.is_empty()
        && !identity.chars().any(char::is_control)
        && identity.matches('@').count() == 1
        && !identity.chars().any(char::is_whitespace)
}

/// The routine's run records, newest first, as content-free history:
/// when it ran, whether someone asked for it, how it ended, its result's
/// session and how many items its account returned. Run output itself
/// stays in the Agent's session.
pub(super) async fn routine_history(
    State(state): State<AppState>,
    axum::Extension(principal): axum::Extension<AuthenticatedPrincipal>,
    Path((agent_id, routine_id)): Path<(String, String)>,
) -> Response {
    if !operator(&principal) {
        return StatusCode::FORBIDDEN.into_response();
    }
    if !registered_agent(&state, &agent_id) {
        return StatusCode::NOT_FOUND.into_response();
    }
    let Some(routine) = crate::automations::routine(&state, &agent_id, &routine_id) else {
        return StatusCode::NOT_FOUND.into_response();
    };
    match state.core.runs().of_trigger(&routine.id) {
        Ok(runs) => {
            let runs: Vec<_> = runs.iter().filter_map(routine_run_view).collect();
            Json(serde_json::json!({ "runs": runs })).into_response()
        }
        Err(_) => StatusCode::SERVICE_UNAVAILABLE.into_response(),
    }
}

/// One run of a routine as its history shows it; a skipped or coalesced
/// record is no run of its own. A completed run that started no session
/// found nothing new.
pub(crate) fn routine_run_view(run: &vak_session::runs::RunRecord) -> Option<serde_json::Value> {
    use vak_session::runs::{RunStatus, Slot};
    if run.coalesced_into.is_some() {
        return None;
    }
    let status = match run.status {
        RunStatus::Skipped => return None,
        RunStatus::Running => "running",
        RunStatus::Completed if run.sessions.is_empty() => "no_changes",
        RunStatus::Completed => "complete",
        RunStatus::Failed => "failed",
        RunStatus::Cancelled | RunStatus::Abandoned => "interrupted",
    };
    Some(serde_json::json!({
        "run_id": run.id.to_string(),
        "session_id": run.sessions.first(),
        "trigger": if matches!(run.slot, Some(Slot::Event { .. })) { "manual" } else { "scheduled" },
        "status": status,
        "started_at": run.opened_at,
        "finished_at": run.settled_at,
        "items_returned": run.items_used.unwrap_or(0),
    }))
}

pub(super) async fn save_candidate(
    State(state): State<AppState>,
    axum::Extension(principal): axum::Extension<AuthenticatedPrincipal>,
    Path(agent_id): Path<String>,
    Json(request): Json<CandidateWriteRequest>,
) -> Response {
    if !operator(&principal) {
        return StatusCode::FORBIDDEN.into_response();
    }
    if !valid_agent(&state, &agent_id) {
        return StatusCode::NOT_FOUND.into_response();
    }
    let account_lock = state.mail_calendar_account_lock(&agent_id, &request.account_id);
    let _account_guard = account_lock.lock().await;
    let ledger = match ConnectionLedger::for_agent(&agent_id) {
        Ok(ledger) => ledger,
        Err(_) => return StatusCode::BAD_REQUEST.into_response(),
    };
    let local_draft = request.account_id == LOCAL_DRAFT_ACCOUNT_ID;
    let account = if local_draft {
        None
    } else {
        match ledger.read_all() {
            Ok(accounts) => accounts.into_iter().find(|account| {
                account.id == request.account_id
                    && account.status == AccountStatus::Connected
                    && account.revoked_at.is_none()
                    && account.owner_agent_id == agent_id
                    && account.provider != Provider::AppleIcloud
                    && account
                        .allowed_audiences
                        .contains(&format!("agent:{agent_id}"))
            }),
            Err(_) => return StatusCode::SERVICE_UNAVAILABLE.into_response(),
        }
    };
    if !local_draft && account.is_none() {
        return (
            StatusCode::FORBIDDEN,
            "The selected account is unavailable for this Agent.",
        )
            .into_response();
    }
    if account.as_ref().is_some_and(|account| {
        !vak_mail_calendar::effect::supports_action(account.provider, &request.action)
    }) {
        return (
            StatusCode::BAD_REQUEST,
            "This provider does not support that reviewed action.",
        )
            .into_response();
    }
    let vault = match AccountVault::for_agent(&agent_id) {
        Ok(vault) => vault,
        Err(_) => return StatusCode::SERVICE_UNAVAILABLE.into_response(),
    };
    let candidate = match request.candidate_id {
        None if request.expected_revision.is_none() => {
            let Some(source_refs) = request.source_refs else {
                return (
                    StatusCode::BAD_REQUEST,
                    "New candidates need source references.",
                )
                    .into_response();
            };
            match ActionCandidate::new(
                account
                    .as_ref()
                    .map(|account| account.id.clone())
                    .unwrap_or_else(|| LOCAL_DRAFT_ACCOUNT_ID.to_owned()),
                agent_id.clone(),
                format!("agent:{agent_id}"),
                source_refs,
                request.action,
            ) {
                Ok(candidate) => candidate,
                Err(error) => return (StatusCode::BAD_REQUEST, error.to_string()).into_response(),
            }
        }
        Some(candidate_id)
            if request.expected_revision.is_some() && request.source_refs.is_none() =>
        {
            let mut candidates = match vault.list_candidates() {
                Ok(candidates) => candidates,
                Err(_) => return StatusCode::SERVICE_UNAVAILABLE.into_response(),
            };
            let Some(mut candidate) = candidates.drain(..).find(|item| item.id == candidate_id)
            else {
                return StatusCode::NOT_FOUND.into_response();
            };
            if candidate.account_id != request.account_id
                || candidate.agent_id != agent_id
                || candidate.audience_id != format!("agent:{agent_id}")
            {
                return StatusCode::FORBIDDEN.into_response();
            }
            if let Err(error) = candidate.revise(request.action) {
                return (StatusCode::BAD_REQUEST, error.to_string()).into_response();
            }
            candidate
        }
        _ => {
            return (
                StatusCode::BAD_REQUEST,
                "Candidate creation or revision fields are inconsistent.",
            )
                .into_response();
        }
    };
    let expected_revision = request.expected_revision;
    match vault.save_candidate(candidate.clone(), expected_revision) {
        Ok(()) => {
            // A second server process may disconnect after the initial read.
            // Recheck after the vault write; if that raced, remove the saved
            // version before exposing it to the owner.
            let account_still_connected = if let Some(account) = account.as_ref() {
                ledger.read_all().is_ok_and(|accounts| {
                    accounts.iter().any(|current| {
                        current.id == candidate.account_id
                            && current.revision == account.revision
                            && current.status == AccountStatus::Connected
                            && current.revoked_at.is_none()
                    })
                })
            } else {
                candidate.account_id == LOCAL_DRAFT_ACCOUNT_ID
            };
            if !account_still_connected {
                let _ = vault.delete_candidate(&candidate.id, candidate.revision);
                return (
                    StatusCode::CONFLICT,
                    Json(serde_json::json!({
                        "error": "The account changed while saving; reload its status and try again.",
                        "kind": "mail_calendar_account_changed",
                    })),
                )
                    .into_response();
            }
            let digest = match candidate.digest() {
                Ok(digest) => digest,
                Err(_) => return StatusCode::INTERNAL_SERVER_ERROR.into_response(),
            };
            match candidate_view(candidate, None) {
                Some(candidate) => {
                    Json(serde_json::json!({"candidate": candidate, "candidate_digest": digest}))
                        .into_response()
                }
                None => StatusCode::INTERNAL_SERVER_ERROR.into_response(),
            }
        }
        Err(vak_mail_calendar::vault::VaultError::Conflict) => (
            StatusCode::CONFLICT,
            Json(serde_json::json!({
                "error": "The candidate changed; reload it before saving.",
                "kind": "mail_calendar_candidate_conflict",
            })),
        )
            .into_response(),
        Err(vak_mail_calendar::vault::VaultError::TooLarge) => (
            StatusCode::PAYLOAD_TOO_LARGE,
            "The local work area is full.",
        )
            .into_response(),
        Err(_) => StatusCode::SERVICE_UNAVAILABLE.into_response(),
    }
}

fn candidate_view(
    candidate: ActionCandidate,
    action_state: Option<ActionState>,
) -> Option<serde_json::Value> {
    let digest = candidate.digest().ok()?;
    let mut value = serde_json::to_value(candidate).ok()?;
    value
        .as_object_mut()?
        .insert("candidate_digest".into(), digest.into());
    if let Some(state) = action_state {
        value
            .as_object_mut()?
            .insert("action_state".into(), serde_json::to_value(state).ok()?);
    }
    Some(value)
}

/// Commit only an exact, owner-confirmed supported provider candidate, as its
/// one effect (`effect::commit_candidate`). The account lock spans
/// revalidation, the effect's preparation and only attempt, and its outcome,
/// so local disconnect/revision paths cannot cross it.
pub(super) async fn send_mail_candidate(
    State(state): State<AppState>,
    axum::Extension(principal): axum::Extension<AuthenticatedPrincipal>,
    Path((agent_id, candidate_id)): Path<(String, String)>,
    OriginalUri(uri): OriginalUri,
    Json(request): Json<SendCandidateRequest>,
) -> Response {
    use axum::response::IntoResponse;
    if !operator(&principal) {
        return StatusCode::FORBIDDEN.into_response();
    }
    if !valid_agent(&state, &agent_id) {
        return StatusCode::NOT_FOUND.into_response();
    }
    if !request.confirm
        || request.candidate_digest.len() != 64
        || !request
            .candidate_digest
            .bytes()
            .all(|byte| byte.is_ascii_hexdigit())
    {
        return (
            StatusCode::BAD_REQUEST,
            "Review and confirm the exact provider action.",
        )
            .into_response();
    }
    let vault = match AccountVault::for_agent(&agent_id) {
        Ok(vault) => vault,
        Err(_) => return StatusCode::SERVICE_UNAVAILABLE.into_response(),
    };
    let initial = match vault.list_candidates() {
        Ok(candidates) => candidates
            .into_iter()
            .find(|candidate| candidate.id == candidate_id),
        Err(_) => return StatusCode::SERVICE_UNAVAILABLE.into_response(),
    };
    let Some(initial) = initial else {
        return StatusCode::NOT_FOUND.into_response();
    };
    if initial.agent_id != agent_id || initial.audience_id != format!("agent:{agent_id}") {
        return StatusCode::FORBIDDEN.into_response();
    }
    let account_lock = state.mail_calendar_account_lock(&agent_id, &initial.account_id);
    let _account_guard = account_lock.lock().await;
    if !valid_agent(&state, &agent_id) {
        return StatusCode::CONFLICT.into_response();
    }

    // Reload candidate and account after waiting for the same account lock
    // used by save, disconnect, and refresh handlers.
    let candidate = match vault.list_candidates() {
        Ok(candidates) => candidates
            .into_iter()
            .find(|candidate| candidate.id == candidate_id),
        Err(_) => return StatusCode::SERVICE_UNAVAILABLE.into_response(),
    };
    let Some(candidate) = candidate else {
        return StatusCode::CONFLICT.into_response();
    };
    if candidate.account_id != initial.account_id
        || candidate.agent_id != agent_id
        || candidate.audience_id != format!("agent:{agent_id}")
        || candidate.revision != request.expected_revision
        || !candidate
            .digest()
            .is_ok_and(|digest| digest == request.candidate_digest)
    {
        return (
            StatusCode::CONFLICT,
            "The draft changed. Reload and review it again before sending.",
        )
            .into_response();
    }
    let required_capability = candidate.required_capability();
    let requires_mail_read = matches!(
        &candidate.action,
        ProposedAction::SendMail { draft } if draft.reply_to_message_id.is_some()
    );
    let requires_calendar_read = matches!(&candidate.action, ProposedAction::RespondToEvent { .. });
    let send_route = uri.path().ends_with("/send");
    let create_route = uri.path().ends_with("/create-event");
    let update_route = uri.path().ends_with("/update-event");
    let cancel_route = uri.path().ends_with("/cancel-event");
    let respond_route = uri.path().ends_with("/respond-event");
    match &candidate.action {
        ProposedAction::SendMail { draft } if send_route && vak_mail_calendar::effect::validate_mail_draft(draft).is_ok() => {},
        ProposedAction::CreateEvent { draft } if create_route && vak_mail_calendar::effect::validate_event_create(draft).is_ok() => {},
        ProposedAction::UpdateEvent { event_id, source_version, draft } if update_route && vak_mail_calendar::effect::validate_event_update(event_id, source_version, draft).is_ok() => {},
        ProposedAction::CancelEvent { event_id, source_version, occurrence_id, whole_series } if cancel_route && vak_mail_calendar::effect::validate_event_cancel(event_id, source_version, occurrence_id.as_deref(), *whole_series).is_ok() => {},
        ProposedAction::RespondToEvent { event_id, source_version, .. } if respond_route && vak_mail_calendar::effect::validate_event_response(event_id, source_version).is_ok() => {},
        ProposedAction::SendMail { .. } => return (StatusCode::BAD_REQUEST, "This send profile supports plain text without attachments or sender aliases; replies require a selected message and conversation.").into_response(),
        ProposedAction::CreateEvent { .. } => return (StatusCode::BAD_REQUEST, "This event profile supports one timed event without attendees, recurrence, or reminders.").into_response(),
        ProposedAction::UpdateEvent { .. } => return (StatusCode::BAD_REQUEST, "This update profile supports one standalone timed Google event without attendees, recurrence, or reminders.").into_response(),
        ProposedAction::CancelEvent { .. } => return (StatusCode::BAD_REQUEST, "Cancellation is limited to one unchanged public standalone Google event with no attendees.").into_response(),
        ProposedAction::RespondToEvent { .. } => return (StatusCode::BAD_REQUEST, "RSVP is limited to a standalone Google invitation that still matches the version you reviewed.").into_response(),
    }
    if !candidate.source_refs.iter().all(|source| {
        source
            .item_id
            .chars()
            .all(|character| !character.is_control())
    }) {
        return StatusCode::BAD_REQUEST.into_response();
    }

    let ledger = match ConnectionLedger::for_agent(&agent_id) {
        Ok(ledger) => ledger,
        Err(_) => return StatusCode::SERVICE_UNAVAILABLE.into_response(),
    };
    let account = match ledger.read_all() {
        Ok(accounts) => accounts.into_iter().find(|account| {
            account.id == candidate.account_id
                && account.owner_agent_id == agent_id
                && account.admits(&agent_id, &candidate.audience_id, required_capability)
                && (!requires_mail_read
                    || account.admits(&agent_id, &candidate.audience_id, Capability::MailRead))
                && (!requires_calendar_read
                    || account.admits(&agent_id, &candidate.audience_id, Capability::CalendarRead))
        }),
        Err(_) => return StatusCode::SERVICE_UNAVAILABLE.into_response(),
    };
    let Some(account) = account else {
        return (
            StatusCode::FORBIDDEN,
            "This account does not grant this provider action.",
        )
            .into_response();
    };
    if !vak_mail_calendar::effect::supports_action(account.provider, &candidate.action) {
        return (
            StatusCode::BAD_REQUEST,
            "This provider does not support that reviewed action.",
        )
            .into_response();
    }
    if account
        .access_token_expires_at
        .is_some_and(|expires| expires <= Utc::now())
    {
        return (
            StatusCode::PRECONDITION_REQUIRED,
            "Refresh this account's sign-in before making this change.",
        )
            .into_response();
    }
    if !vault.credential_available(&account.id) {
        return (
            StatusCode::CONFLICT,
            "Sign in to this account again before making this change.",
        )
            .into_response();
    }

    // The Core permission engine remains an independent gate from OAuth
    // capability and explicit Review confirmation. Ask can be resolved only
    // by this owner-confirmed, digest-bound request; an explicit Deny wins.
    let core = match agent_chats::resolve_agent_core(&state, &agent_id) {
        Ok((_, core)) => core,
        Err(response) => return response,
    };
    let engine = match core.build_permission_engine(&[]) {
        Ok(engine) => engine,
        Err(_) => return StatusCode::SERVICE_UNAVAILABLE.into_response(),
    };
    let mode = match core.effective_permission_mode() {
        vak_config::PermissionMode::ReadOnly => vak_permission::Mode::ReadOnly,
        vak_config::PermissionMode::WorkspaceWrite => vak_permission::Mode::WorkspaceWrite,
        vak_config::PermissionMode::FullAccess => vak_permission::Mode::FullAccess,
    };
    let permission_args = serde_json::json!({
        "agent_id": agent_id,
        "account_id": account.id,
        "candidate_id": candidate.id,
        "candidate_digest": request.candidate_digest.clone(),
        "revision": candidate.revision,
    });
    if matches!(
        engine.evaluate(
            match &candidate.action {
                ProposedAction::SendMail { .. } => "mail_calendar_send",
                ProposedAction::CreateEvent { .. } => "mail_calendar_event_create",
                ProposedAction::UpdateEvent { .. } => "mail_calendar_event_update",
                ProposedAction::CancelEvent { .. } => "mail_calendar_event_cancel",
                ProposedAction::RespondToEvent { .. } => "mail_calendar_event_respond",
            },
            &permission_args,
            mode,
            core.cwd()
        ),
        vak_permission::Decision::Deny { .. }
    ) {
        return (
            StatusCode::FORBIDDEN,
            "The Agent's permission rules deny this provider change.",
        )
            .into_response();
    }
    let now = Utc::now();
    let approval = CandidateApproval {
        candidate_id: candidate.id.clone(),
        candidate_digest: request.candidate_digest,
        account_id: account.id.clone(),
        agent_id: agent_id.clone(),
        audience_id: candidate.audience_id.clone(),
        approved_at: now,
        expires_at: now + chrono::Duration::minutes(5),
        single_use: true,
    };
    if candidate
        .authorize_effect(&account, &approval, now)
        .is_err()
    {
        return StatusCode::FORBIDDEN.into_response();
    }
    // Fail before preparing if the client cannot be constructed. Once the
    // effect is taken, every result is single-use, including unknown.
    let client = match ProviderEffectClient::new() {
        Ok(client) => client,
        Err(_) => return StatusCode::SERVICE_UNAVAILABLE.into_response(),
    };
    let effects = state.core.effects();
    let committed = vak_mail_calendar::effect::commit_candidate(
        &client,
        &effects,
        Some(crate::request_trace(&state, "mail-calendar-change")),
        &account,
        &vault,
        &agent_id,
        &candidate,
    )
    .await;
    let (effect, outcome) = match committed {
        Ok(vak_mail_calendar::effect::Committed::Attempted { effect, outcome }) => {
            (effect, outcome)
        }
        Ok(vak_mail_calendar::effect::Committed::AlreadyAttempted(prior)) => {
            return (
                StatusCode::CONFLICT,
                Json(serde_json::json!({
                    "error": "This draft already has a provider action attempt; it will not be retried.",
                    "receipt": effect_receipt(&prior),
                })),
            )
                .into_response();
        }
        // The effect could not be recorded: if it was taken, the provider
        // may have been called, and the effect stays unsendable.
        Err(_) => {
            return (
                StatusCode::ACCEPTED,
                Json(serde_json::json!({
                    "state": "dispatching",
                    "message": "The provider request may have been made; do not retry this draft.",
                })),
            )
                .into_response();
        }
    };
    let detail_code = match &outcome {
        Ok(_) if matches!(candidate.action, ProposedAction::CancelEvent { .. }) => {
            "provider_accepted_event_cancelled"
        }
        Ok(_) => "provider_accepted_not_delivery",
        Err(error) => vak_mail_calendar::effect::error_code(error),
    };
    if outcome == Err(ProviderEffectError::ReauthorizationRequired) {
        mark_account_reauthentication_required(&state, &account, "provider_effect_rejected");
    }
    record_account_event(
        &state,
        if required_capability == Capability::MailSend {
            "mail_send_effect"
        } else if matches!(candidate.action, ProposedAction::UpdateEvent { .. }) {
            "calendar_event_update_effect"
        } else if matches!(candidate.action, ProposedAction::CancelEvent { .. }) {
            "calendar_event_cancel_effect"
        } else if matches!(candidate.action, ProposedAction::RespondToEvent { .. }) {
            "calendar_event_response_effect"
        } else {
            "calendar_event_create_effect"
        },
        &agent_id,
        &account.id,
        account.provider,
        &account.capabilities,
        detail_code,
    );
    let status = match &outcome {
        Ok(_) => StatusCode::ACCEPTED,
        Err(ProviderEffectError::Conflict) => StatusCode::CONFLICT,
        Err(_) => StatusCode::BAD_GATEWAY,
    };
    let mut receipt = effect_receipt(&effect);
    receipt["detail_code"] = serde_json::json!(detail_code);
    (status, Json(serde_json::json!({ "receipt": receipt }))).into_response()
}

/// Records that the provider shows the effect's change landed, and
/// answers with its receipt.
fn confirm_by_provider(
    state: &AppState,
    effect: &vak_session::effects::EffectRecord,
    provider: Provider,
    provider_item_id: String,
    detail_code: &str,
) -> Response {
    let effects = state.core.effects();
    let receipt = vak_session::effects::Receipt {
        provider: provider.as_str().to_owned(),
        provider_id: Some(provider_item_id),
        at: Utc::now(),
    };
    let confirmed = effects
        .confirmed(effect.id, receipt)
        .and_then(|()| effects.get(effect.id));
    match confirmed {
        Ok(Some(effect)) => {
            let mut receipt = effect_receipt(&effect);
            receipt["detail_code"] = serde_json::json!(detail_code);
            (
                StatusCode::OK,
                Json(serde_json::json!({"matched": true, "receipt": receipt})),
            )
                .into_response()
        }
        _ => StatusCode::SERVICE_UNAVAILABLE.into_response(),
    }
}

/// A candidate's effect as Review reads it: its state in the candidate's
/// words, the provider's id, and why it failed or is unknown.
fn effect_receipt(effect: &vak_session::effects::EffectRecord) -> serde_json::Value {
    serde_json::json!({
        "effect": effect.id.to_string(),
        "state": vak_mail_calendar::effect::action_state(effect),
        "provider_item_id": effect.receipt.as_ref().and_then(|receipt| receipt.provider_id.clone()),
        "detail_code": effect.reason,
        "observed_at": effect.updated_at,
    })
}

/// The candidate's effect, if any, read from the effects chain.
fn candidate_effect(
    state: &AppState,
    candidate_id: &str,
) -> Result<Option<vak_session::effects::EffectRecord>, StatusCode> {
    vak_mail_calendar::effect::candidate_effect(&state.core.effects(), candidate_id)
        .map_err(|_| StatusCode::SERVICE_UNAVAILABLE)
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct ReconcileEventRequest {
    expected_revision: u64,
    candidate_digest: String,
}

/// Reconcile a supported calendar effect after ambiguous dispatch.
/// Provider-private attempt markers can prove the exact attempt; absence is
/// inconclusive and never enables a retry.
pub(super) async fn reconcile_event_candidate(
    State(state): State<AppState>,
    axum::Extension(principal): axum::Extension<AuthenticatedPrincipal>,
    Path((agent_id, candidate_id)): Path<(String, String)>,
    Json(request): Json<ReconcileEventRequest>,
) -> Response {
    if !operator(&principal) {
        return StatusCode::FORBIDDEN.into_response();
    }
    if !valid_agent(&state, &agent_id) {
        return StatusCode::NOT_FOUND.into_response();
    }
    if request.candidate_digest.len() != 64
        || !request
            .candidate_digest
            .bytes()
            .all(|b| b.is_ascii_hexdigit())
    {
        return StatusCode::BAD_REQUEST.into_response();
    }
    let vault = match AccountVault::for_agent(&agent_id) {
        Ok(vault) => vault,
        Err(_) => return StatusCode::SERVICE_UNAVAILABLE.into_response(),
    };
    let Some(initial) = vault
        .list_candidates()
        .ok()
        .and_then(|rows| rows.into_iter().find(|row| row.id == candidate_id))
    else {
        return StatusCode::NOT_FOUND.into_response();
    };
    if initial.agent_id != agent_id || initial.audience_id != format!("agent:{agent_id}") {
        return StatusCode::FORBIDDEN.into_response();
    }
    let lock = state.mail_calendar_account_lock(&agent_id, &initial.account_id);
    let _guard = lock.lock().await;
    if !valid_agent(&state, &agent_id) {
        return StatusCode::CONFLICT.into_response();
    }
    let candidate = match vault
        .list_candidates()
        .ok()
        .and_then(|rows| rows.into_iter().find(|row| row.id == candidate_id))
    {
        Some(candidate) => candidate,
        None => return StatusCode::CONFLICT.into_response(),
    };
    if candidate.agent_id != agent_id
        || candidate.account_id != initial.account_id
        || candidate.revision != request.expected_revision
        || !candidate
            .digest()
            .is_ok_and(|digest| digest == request.candidate_digest)
    {
        return (
            StatusCode::CONFLICT,
            "The draft changed; reload its latest status before reconciling.",
        )
            .into_response();
    }
    let (reconcile_update_event, reconcile_cancel_event, reconcile_response_event) =
        match &candidate.action {
            ProposedAction::CreateEvent { .. } => (None, None, None),
            ProposedAction::UpdateEvent { event_id, .. } => (Some(event_id.as_str()), None, None),
            ProposedAction::CancelEvent {
                event_id,
                occurrence_id,
                whole_series,
                ..
            } if occurrence_id.is_none() && !whole_series => (None, Some(event_id.as_str()), None),
            ProposedAction::RespondToEvent {
                event_id, response, ..
            } => (None, None, Some((event_id.as_str(), *response))),
            _ => {
                return (
                    StatusCode::BAD_REQUEST,
                    "Only an ambiguous supported calendar event change can be reconciled here.",
                )
                    .into_response();
            }
        };
    let effects = state.core.effects();
    let Ok(Some(effect)) = candidate_effect(&state, &candidate.id) else {
        return StatusCode::CONFLICT.into_response();
    };
    if !matches!(
        effect.status,
        vak_session::effects::EffectStatus::Unknown | vak_session::effects::EffectStatus::Sending
    ) || !vak_mail_calendar::effect::is_effect_of(
        &effects,
        &effect,
        &candidate.id,
        &request.candidate_digest,
        &candidate.account_id,
    ) {
        return (
            StatusCode::CONFLICT,
            "This calendar action does not have a reconcilable ambiguous outcome.",
        )
            .into_response();
    }
    let ledger = match ConnectionLedger::for_agent(&agent_id) {
        Ok(ledger) => ledger,
        Err(_) => return StatusCode::SERVICE_UNAVAILABLE.into_response(),
    };
    let account = match ledger.read_all().ok().and_then(|accounts| {
        accounts.into_iter().find(|account| {
            account.id == candidate.account_id
                && account.owner_agent_id == agent_id
                && account.admits(&agent_id, &candidate.audience_id, Capability::CalendarWrite)
                && (!matches!(&candidate.action, ProposedAction::RespondToEvent { .. })
                    || account.admits(&agent_id, &candidate.audience_id, Capability::CalendarRead))
        })
    }) {
        Some(account) => account,
        None => return StatusCode::FORBIDDEN.into_response(),
    };
    let provider_supports_reconcile = match account.provider {
        Provider::Google => true,
        Provider::Microsoft => {
            reconcile_update_event.is_none()
                && reconcile_cancel_event.is_none()
                && reconcile_response_event.is_none()
        }
        Provider::AppleIcloud => false,
    };
    if !provider_supports_reconcile {
        return StatusCode::BAD_REQUEST.into_response();
    }
    if account
        .access_token_expires_at
        .is_some_and(|expires| expires <= Utc::now())
        || !vault.credential_available(&account.id)
    {
        return (
            StatusCode::PRECONDITION_REQUIRED,
            "Refresh this account's sign-in before checking the event result.",
        )
            .into_response();
    }
    let core = match agent_chats::resolve_agent_core(&state, &agent_id) {
        Ok((_, core)) => core,
        Err(response) => return response,
    };
    let engine = match core.build_permission_engine(&[]) {
        Ok(engine) => engine,
        Err(_) => return StatusCode::SERVICE_UNAVAILABLE.into_response(),
    };
    let mode = match core.effective_permission_mode() {
        vak_config::PermissionMode::ReadOnly => vak_permission::Mode::ReadOnly,
        vak_config::PermissionMode::WorkspaceWrite => vak_permission::Mode::WorkspaceWrite,
        vak_config::PermissionMode::FullAccess => vak_permission::Mode::FullAccess,
    };
    let args = serde_json::json!({"agent_id": agent_id, "account_id": account.id, "candidate_id": candidate.id, "candidate_digest": request.candidate_digest});
    if matches!(
        engine.evaluate("mail_calendar_event_reconcile", &args, mode, core.cwd()),
        vak_permission::Decision::Deny { .. }
    ) {
        return (
            StatusCode::FORBIDDEN,
            "The Agent's permission rules deny this provider read.",
        )
            .into_response();
    }
    let client = match ProviderEffectClient::new() {
        Ok(client) => client,
        Err(_) => return StatusCode::SERVICE_UNAVAILABLE.into_response(),
    };
    let provider_result = if let Some(event_id) = reconcile_update_event {
        client
            .reconcile_updated_event(
                &account,
                &vault,
                &agent_id,
                &candidate.audience_id,
                &vak_mail_calendar::effect::attempt_marker(&effect),
                event_id,
            )
            .await
    } else if let Some(event_id) = reconcile_cancel_event {
        client
            .reconcile_cancelled_event(
                &account,
                &vault,
                &agent_id,
                &candidate.audience_id,
                &vak_mail_calendar::effect::attempt_marker(&effect),
                event_id,
            )
            .await
    } else if let Some((event_id, response)) = reconcile_response_event {
        client
            .reconcile_event_response(
                &account,
                &vault,
                &agent_id,
                &candidate.audience_id,
                &vak_mail_calendar::effect::attempt_marker(&effect),
                event_id,
                response,
            )
            .await
    } else {
        client
            .reconcile_created_event(
                &account,
                &vault,
                &agent_id,
                &candidate.audience_id,
                &vak_mail_calendar::effect::attempt_marker(&effect),
            )
            .await
    };
    match provider_result {
        Ok(Some(provider_item_id)) => {
            let detail_code = if reconcile_cancel_event.is_some() {
                    "provider_event_cancel_confirmed_by_reconciliation"
                } else if reconcile_response_event.is_some() {
                    "provider_event_response_confirmed_by_reconciliation"
                } else if reconcile_update_event.is_some() {
                    "provider_event_update_confirmed_by_reconciliation"
                } else {
                    "provider_event_confirmed_by_reconciliation"
                };
            confirm_by_provider(&state, &effect, account.provider, provider_item_id, detail_code)
        }
        Ok(None) => (StatusCode::OK, Json(serde_json::json!({"matched": false, "state": vak_mail_calendar::effect::action_state(&effect), "message": "No matching event is visible yet. The outcome remains unknown and the action cannot be retried."}))).into_response(),
        Err(ProviderEffectError::ReauthorizationRequired) => (StatusCode::PRECONDITION_REQUIRED, "Refresh this account's sign-in before checking the event result.").into_response(),
        Err(ProviderEffectError::NotAdmitted) => StatusCode::FORBIDDEN.into_response(),
        Err(_) => StatusCode::BAD_GATEWAY.into_response(),
    }
}

/// Reconcile a sent-mail attempt by its exact provider-visible opaque marker.
/// A missing marker leaves the durable action unknown and non-retryable.
pub(super) async fn reconcile_mail_candidate(
    State(state): State<AppState>,
    axum::Extension(principal): axum::Extension<AuthenticatedPrincipal>,
    Path((agent_id, candidate_id)): Path<(String, String)>,
    Json(request): Json<ReconcileEventRequest>,
) -> Response {
    if !operator(&principal) {
        return StatusCode::FORBIDDEN.into_response();
    }
    if !valid_agent(&state, &agent_id) {
        return StatusCode::NOT_FOUND.into_response();
    }
    if request.candidate_digest.len() != 64
        || !request
            .candidate_digest
            .bytes()
            .all(|b| b.is_ascii_hexdigit())
    {
        return StatusCode::BAD_REQUEST.into_response();
    }
    let vault = match AccountVault::for_agent(&agent_id) {
        Ok(vault) => vault,
        Err(_) => return StatusCode::SERVICE_UNAVAILABLE.into_response(),
    };
    let Some(initial) = vault
        .list_candidates()
        .ok()
        .and_then(|rows| rows.into_iter().find(|row| row.id == candidate_id))
    else {
        return StatusCode::NOT_FOUND.into_response();
    };
    if initial.agent_id != agent_id || initial.audience_id != format!("agent:{agent_id}") {
        return StatusCode::FORBIDDEN.into_response();
    }
    let lock = state.mail_calendar_account_lock(&agent_id, &initial.account_id);
    let _guard = lock.lock().await;
    if !valid_agent(&state, &agent_id) {
        return StatusCode::CONFLICT.into_response();
    }
    let candidate = match vault
        .list_candidates()
        .ok()
        .and_then(|rows| rows.into_iter().find(|row| row.id == candidate_id))
    {
        Some(candidate) => candidate,
        None => return StatusCode::CONFLICT.into_response(),
    };
    if candidate.agent_id != agent_id
        || candidate.account_id != initial.account_id
        || candidate.revision != request.expected_revision
        || !candidate
            .digest()
            .is_ok_and(|digest| digest == request.candidate_digest)
    {
        return (
            StatusCode::CONFLICT,
            "The draft changed; reload its latest status before reconciling.",
        )
            .into_response();
    }
    if !matches!(candidate.action, ProposedAction::SendMail { .. }) {
        return (
            StatusCode::BAD_REQUEST,
            "Only an ambiguous sent-mail action can be reconciled here.",
        )
            .into_response();
    }
    let effects = state.core.effects();
    let Ok(Some(effect)) = candidate_effect(&state, &candidate.id) else {
        return StatusCode::CONFLICT.into_response();
    };
    if !matches!(
        effect.status,
        vak_session::effects::EffectStatus::Unknown | vak_session::effects::EffectStatus::Sending
    ) || !vak_mail_calendar::effect::is_effect_of(
        &effects,
        &effect,
        &candidate.id,
        &request.candidate_digest,
        &candidate.account_id,
    ) {
        return (
            StatusCode::CONFLICT,
            "This mail action does not have a reconcilable ambiguous outcome.",
        )
            .into_response();
    }
    let ledger = match ConnectionLedger::for_agent(&agent_id) {
        Ok(ledger) => ledger,
        Err(_) => return StatusCode::SERVICE_UNAVAILABLE.into_response(),
    };
    let account = match ledger.read_all().ok().and_then(|accounts| {
        accounts.into_iter().find(|account| {
            account.id == candidate.account_id
                && account.owner_agent_id == agent_id
                && account.admits(&agent_id, &candidate.audience_id, Capability::MailRead)
        })
    }) {
        Some(account) => account,
        None => return StatusCode::FORBIDDEN.into_response(),
    };
    if !matches!(account.provider, Provider::Google | Provider::Microsoft) {
        return (
            StatusCode::BAD_REQUEST,
            "This provider does not support sent-mail reconciliation.",
        )
            .into_response();
    }
    if account
        .access_token_expires_at
        .is_some_and(|expires| expires <= Utc::now())
        || !vault.credential_available(&account.id)
    {
        return (
            StatusCode::PRECONDITION_REQUIRED,
            "Refresh this account's sign-in before checking the send result.",
        )
            .into_response();
    }
    let core = match agent_chats::resolve_agent_core(&state, &agent_id) {
        Ok((_, core)) => core,
        Err(response) => return response,
    };
    let engine = match core.build_permission_engine(&[]) {
        Ok(engine) => engine,
        Err(_) => return StatusCode::SERVICE_UNAVAILABLE.into_response(),
    };
    let mode = match core.effective_permission_mode() {
        vak_config::PermissionMode::ReadOnly => vak_permission::Mode::ReadOnly,
        vak_config::PermissionMode::WorkspaceWrite => vak_permission::Mode::WorkspaceWrite,
        vak_config::PermissionMode::FullAccess => vak_permission::Mode::FullAccess,
    };
    let args = serde_json::json!({"agent_id": agent_id, "account_id": account.id, "candidate_id": candidate.id, "candidate_digest": request.candidate_digest});
    if matches!(
        engine.evaluate("mail_calendar_mail_reconcile", &args, mode, core.cwd()),
        vak_permission::Decision::Deny { .. }
    ) {
        return (
            StatusCode::FORBIDDEN,
            "The Agent's permission rules deny this provider read.",
        )
            .into_response();
    }
    let client = match ProviderEffectClient::new() {
        Ok(client) => client,
        Err(_) => return StatusCode::SERVICE_UNAVAILABLE.into_response(),
    };
    match client.reconcile_sent_mail(&account, &vault, &agent_id, &candidate.audience_id, &vak_mail_calendar::effect::attempt_marker(&effect)).await {
        Ok(Some(item_id)) => confirm_by_provider(
            &state,
            &effect,
            account.provider,
            item_id,
            "provider_mail_confirmed_by_reconciliation",
        ),
        Ok(None) => (StatusCode::OK, Json(serde_json::json!({"matched": false, "state": vak_mail_calendar::effect::action_state(&effect), "message": "No matching sent message is visible yet. The outcome remains unknown and the action cannot be retried."}))).into_response(),
        Err(ProviderEffectError::ReauthorizationRequired) => (StatusCode::PRECONDITION_REQUIRED, "Refresh this account's sign-in before checking the send result.").into_response(),
        Err(ProviderEffectError::NotAdmitted) => StatusCode::FORBIDDEN.into_response(),
        Err(_) => StatusCode::BAD_GATEWAY.into_response(),
    }
}

pub(super) async fn delete_candidate(
    State(state): State<AppState>,
    axum::Extension(principal): axum::Extension<AuthenticatedPrincipal>,
    Path((agent_id, candidate_id)): Path<(String, String)>,
    Json(request): Json<CandidateDeleteRequest>,
) -> Response {
    if !operator(&principal) {
        return StatusCode::FORBIDDEN.into_response();
    }
    if !registered_agent(&state, &agent_id) {
        return StatusCode::NOT_FOUND.into_response();
    }
    let vault = match AccountVault::for_agent(&agent_id) {
        Ok(vault) => vault,
        Err(_) => return StatusCode::SERVICE_UNAVAILABLE.into_response(),
    };
    let initial = match vault.list_candidates() {
        Ok(candidates) => candidates
            .into_iter()
            .find(|candidate| candidate.id == candidate_id),
        Err(_) => return StatusCode::SERVICE_UNAVAILABLE.into_response(),
    };
    let Some(initial) = initial else {
        return StatusCode::NOT_FOUND.into_response();
    };
    let account_lock = state.mail_calendar_account_lock(&agent_id, &initial.account_id);
    let _account_guard = account_lock.lock().await;
    if !registered_agent(&state, &agent_id) {
        return StatusCode::NOT_FOUND.into_response();
    }
    let still_exists = vault.list_candidates().is_ok_and(|candidates| {
        candidates.into_iter().any(|candidate| {
            candidate.id == candidate_id
                && candidate.account_id == initial.account_id
                && candidate.agent_id == agent_id
        })
    });
    if !still_exists {
        return StatusCode::CONFLICT.into_response();
    }
    match vault.delete_candidate(&candidate_id, request.expected_revision) {
        Ok(()) => Json(serde_json::json!({"deleted": true})).into_response(),
        Err(vak_mail_calendar::vault::VaultError::Conflict) => (
            StatusCode::CONFLICT,
            "The candidate changed; reload it before deleting.",
        )
            .into_response(),
        Err(_) => StatusCode::SERVICE_UNAVAILABLE.into_response(),
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct MailPreviewRequest {
    limit: Option<usize>,
    query: Option<String>,
    folder_id: Option<String>,
    #[serde(default)]
    cursor: Option<String>,
}

pub(super) async fn mail_folders(
    State(state): State<AppState>,
    axum::Extension(principal): axum::Extension<AuthenticatedPrincipal>,
    Path((agent_id, account_id)): Path<(String, String)>,
) -> Response {
    if !operator(&principal) {
        return StatusCode::FORBIDDEN.into_response();
    }
    let operation_lock = state.mail_calendar_account_lock(&agent_id, &account_id);
    let _operation_guard = operation_lock.lock().await;
    let Some((account, vault)) = preview_account(&state, &agent_id, &account_id) else {
        return StatusCode::NOT_FOUND.into_response();
    };
    record_account_event(
        &state,
        "mail_folders_preview",
        &agent_id,
        &account_id,
        account.provider,
        &account.capabilities,
        "requested",
    );
    let folders = vak_mail_calendar::provider::ProviderReadClient::default()
        .list_mail_folders(&account, &vault, &agent_id, &format!("agent:{agent_id}"))
        .await;
    match folders {
        Ok(folders) => {
            record_account_event(
                &state,
                "mail_folders_preview",
                &agent_id,
                &account_id,
                account.provider,
                &account.capabilities,
                "succeeded",
            );
            Json(serde_json::json!({"folders": folders})).into_response()
        }
        Err(error) => {
            mark_preview_reauthentication(&state, &account, &error);
            record_account_event(
                &state,
                "mail_folders_preview",
                &agent_id,
                &account_id,
                account.provider,
                &account.capabilities,
                "failed",
            );
            provider_preview_error(error)
        }
    }
}

pub(super) async fn calendar_sources(
    State(state): State<AppState>,
    axum::Extension(principal): axum::Extension<AuthenticatedPrincipal>,
    Path((agent_id, account_id)): Path<(String, String)>,
) -> Response {
    if !operator(&principal) {
        return StatusCode::FORBIDDEN.into_response();
    }
    let operation_lock = state.mail_calendar_account_lock(&agent_id, &account_id);
    let _operation_guard = operation_lock.lock().await;
    let Some((account, vault)) = preview_account(&state, &agent_id, &account_id) else {
        return StatusCode::NOT_FOUND.into_response();
    };
    record_account_event(
        &state,
        "calendar_sources_preview",
        &agent_id,
        &account_id,
        account.provider,
        &account.capabilities,
        "requested",
    );
    let sources = vak_core::mail_calendar::calendar_sources_with_worker(
        &vak_mail_calendar::provider::ProviderReadClient::default(),
        &account,
        &vault,
        &agent_id,
        &format!("agent:{agent_id}"),
        &state.core.tool_worker_exe(),
    )
    .await;
    match sources {
        Ok(sources) => {
            record_account_event(
                &state,
                "calendar_sources_preview",
                &agent_id,
                &account_id,
                account.provider,
                &account.capabilities,
                "succeeded",
            );
            Json(serde_json::json!({"sources": sources})).into_response()
        }
        Err(error) => {
            mark_preview_reauthentication(&state, &account, &error);
            record_account_event(
                &state,
                "calendar_sources_preview",
                &agent_id,
                &account_id,
                account.provider,
                &account.capabilities,
                "failed",
            );
            provider_preview_error(error)
        }
    }
}

#[derive(Deserialize)]
pub(super) struct MessagePreviewRequest {
    provider_id: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct ThreadPreviewRequest {
    thread_id: String,
    #[serde(default)]
    cursor: Option<String>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct AttachmentPreviewRequest {
    message_id: String,
    attachment_id: String,
}

#[derive(Deserialize)]
pub(super) struct CalendarPreviewRequest {
    from: DateTime<Utc>,
    to: DateTime<Utc>,
    limit: Option<usize>,
    #[serde(default)]
    calendar_id: Option<String>,
    #[serde(default)]
    cursor: Option<String>,
}

/// Owner-only interactive preview. The response is transient and is not
/// persisted; Agent/model access will use a separate broker-owned path.
pub(super) async fn mail_preview(
    State(state): State<AppState>,
    axum::Extension(principal): axum::Extension<AuthenticatedPrincipal>,
    Path((agent_id, account_id)): Path<(String, String)>,
    Json(request): Json<MailPreviewRequest>,
) -> Response {
    if !operator(&principal) {
        return StatusCode::FORBIDDEN.into_response();
    }
    let operation_lock = state.mail_calendar_account_lock(&agent_id, &account_id);
    let _operation_guard = operation_lock.lock().await;
    let Some((account, vault)) = preview_account(&state, &agent_id, &account_id) else {
        return StatusCode::NOT_FOUND.into_response();
    };
    if account
        .access_token_expires_at
        .is_some_and(|expires| expires <= Utc::now())
    {
        return (
            StatusCode::PRECONDITION_REQUIRED,
            "Refresh this account's sign-in before previewing content.",
        )
            .into_response();
    }
    record_account_event(
        &state,
        "mail_content_preview",
        &agent_id,
        &account_id,
        account.provider,
        &account.capabilities,
        "requested",
    );
    let client = vak_mail_calendar::provider::ProviderReadClient::default();
    let query = request
        .query
        .as_deref()
        .filter(|query| !query.trim().is_empty());
    let preview = client
        .mail_preview_page(
            &account,
            &vault,
            &agent_id,
            &format!("agent:{agent_id}"),
            request.folder_id.as_deref(),
            query,
            request.cursor.as_deref(),
            request.limit.unwrap_or(10),
        )
        .await;
    match preview {
        Ok(page) => {
            record_account_event(
                &state,
                "mail_content_preview",
                &agent_id,
                &account_id,
                account.provider,
                &account.capabilities,
                "succeeded",
            );
            Json(serde_json::json!({"messages": page.messages, "next_cursor": page.next_cursor}))
                .into_response()
        }
        Err(error) => {
            mark_preview_reauthentication(&state, &account, &error);
            record_account_event(
                &state,
                "mail_content_preview",
                &agent_id,
                &account_id,
                account.provider,
                &account.capabilities,
                "failed",
            );
            provider_preview_error(error)
        }
    }
}

/// Preview one selected Google or Microsoft document attachment. Provider
/// bytes are bounded and sent only to the network-denied document worker; the
/// response contains extracted text, never the original attachment bytes.
pub(super) async fn attachment_preview(
    State(state): State<AppState>,
    axum::Extension(principal): axum::Extension<AuthenticatedPrincipal>,
    Path((agent_id, account_id)): Path<(String, String)>,
    Json(request): Json<AttachmentPreviewRequest>,
) -> Response {
    if !operator(&principal) {
        return StatusCode::FORBIDDEN.into_response();
    }
    if request.message_id.is_empty()
        || request.message_id.len() > 512
        || request.attachment_id.is_empty()
        || request.attachment_id.len() > 2048
    {
        return StatusCode::BAD_REQUEST.into_response();
    }
    let operation_lock = state.mail_calendar_account_lock(&agent_id, &account_id);
    let _operation_guard = operation_lock.lock().await;
    let Some((account, vault)) = preview_account(&state, &agent_id, &account_id) else {
        return StatusCode::NOT_FOUND.into_response();
    };
    record_account_event(
        &state,
        "mail_attachment_preview",
        &agent_id,
        &account_id,
        account.provider,
        &account.capabilities,
        "requested",
    );
    let client = ProviderReadClient::default();
    let attachment = client
        .mail_attachment(
            &account,
            &vault,
            &agent_id,
            &format!("agent:{agent_id}"),
            &request.message_id,
            &request.attachment_id,
        )
        .await;
    let (metadata, bytes) = match attachment {
        Ok(attachment) => attachment,
        Err(error) => {
            mark_preview_reauthentication(&state, &account, &error);
            record_account_event(
                &state,
                "mail_attachment_preview",
                &agent_id,
                &account_id,
                account.provider,
                &account.capabilities,
                "failed",
            );
            return provider_preview_error(error);
        }
    };
    match vak_tools::broker::preview_mail_attachment(
        &state.core.tool_worker_exe(),
        &metadata.filename,
        &bytes,
    )
    .await
    {
        Ok(text) => {
            record_account_event(
                &state,
                "mail_attachment_preview",
                &agent_id,
                &account_id,
                account.provider,
                &account.capabilities,
                "succeeded",
            );
            Json(serde_json::json!({
                "filename": metadata.filename,
                "mime_type": metadata.mime_type,
                "size_bytes": metadata.size_bytes,
                "text": text,
            }))
            .into_response()
        }
        Err(_) => {
            record_account_event(
                &state,
                "mail_attachment_preview",
                &agent_id,
                &account_id,
                account.provider,
                &account.capabilities,
                "failed",
            );
            (
                StatusCode::UNPROCESSABLE_ENTITY,
                Json(serde_json::json!({
                    "kind": "attachment_unpreviewable",
                    "error": "This attachment could not be previewed by the supported document reader.",
                })),
            )
                .into_response()
        }
    }
}

/// Read one owner-selected Apple message without changing its unread state.
/// The IMAP response is parsed only by the network-denied tool worker.
pub(super) async fn message_preview(
    State(state): State<AppState>,
    axum::Extension(principal): axum::Extension<AuthenticatedPrincipal>,
    Path((agent_id, account_id)): Path<(String, String)>,
    Json(request): Json<MessagePreviewRequest>,
) -> Response {
    if !operator(&principal) {
        return StatusCode::FORBIDDEN.into_response();
    }
    if request.provider_id.len() > 64 {
        return StatusCode::BAD_REQUEST.into_response();
    }
    let operation_lock = state.mail_calendar_account_lock(&agent_id, &account_id);
    let _operation_guard = operation_lock.lock().await;
    let Some((account, vault)) = preview_account(&state, &agent_id, &account_id) else {
        return StatusCode::NOT_FOUND.into_response();
    };
    record_account_event(
        &state,
        "mail_message_preview",
        &agent_id,
        &account_id,
        account.provider,
        &account.capabilities,
        "requested",
    );
    let result = vak_core::mail_calendar::read_icloud_message_with_worker(
        &ProviderReadClient::default(),
        &account,
        &vault,
        &agent_id,
        &format!("agent:{agent_id}"),
        &request.provider_id,
        &state.core.tool_worker_exe(),
    )
    .await;
    match result {
        Ok(message) => {
            record_account_event(
                &state,
                "mail_message_preview",
                &agent_id,
                &account_id,
                account.provider,
                &account.capabilities,
                "succeeded",
            );
            Json(message).into_response()
        }
        Err(error) => {
            mark_preview_reauthentication(&state, &account, &error);
            record_account_event(
                &state,
                "mail_message_preview",
                &agent_id,
                &account_id,
                account.provider,
                &account.capabilities,
                "failed",
            );
            provider_preview_error(error)
        }
    }
}

/// Open one selected Google/Microsoft conversation for the owner. The
/// provider adapter bounds the response and verifies every returned message's
/// conversation membership before the content reaches this transient preview.
pub(super) async fn thread_preview(
    State(state): State<AppState>,
    axum::Extension(principal): axum::Extension<AuthenticatedPrincipal>,
    Path((agent_id, account_id)): Path<(String, String)>,
    Json(request): Json<ThreadPreviewRequest>,
) -> Response {
    if !operator(&principal) {
        return StatusCode::FORBIDDEN.into_response();
    }
    if request.thread_id.is_empty()
        || request.thread_id.len() > 512
        || request.thread_id.chars().any(char::is_control)
    {
        return StatusCode::BAD_REQUEST.into_response();
    }
    if request
        .cursor
        .as_ref()
        .is_some_and(|cursor| cursor.len() > 8192)
    {
        return StatusCode::BAD_REQUEST.into_response();
    }
    let operation_lock = state.mail_calendar_account_lock(&agent_id, &account_id);
    let _operation_guard = operation_lock.lock().await;
    let Some((account, vault)) = preview_account(&state, &agent_id, &account_id) else {
        return StatusCode::NOT_FOUND.into_response();
    };
    record_account_event(
        &state,
        "mail_thread_preview",
        &agent_id,
        &account_id,
        account.provider,
        &account.capabilities,
        "requested",
    );
    let result = ProviderReadClient::default()
        .mail_thread(
            &account,
            &vault,
            &agent_id,
            &format!("agent:{agent_id}"),
            &request.thread_id,
            request.cursor.as_deref(),
            vak_mail_calendar::MAX_MAIL_THREAD_MESSAGES,
        )
        .await;
    match result {
        Ok(thread) => {
            record_account_event(
                &state,
                "mail_thread_preview",
                &agent_id,
                &account_id,
                account.provider,
                &account.capabilities,
                "succeeded",
            );
            Json(thread).into_response()
        }
        Err(error) => {
            mark_preview_reauthentication(&state, &account, &error);
            record_account_event(
                &state,
                "mail_thread_preview",
                &agent_id,
                &account_id,
                account.provider,
                &account.capabilities,
                "failed",
            );
            provider_preview_error(error)
        }
    }
}

pub(super) async fn calendar_preview(
    State(state): State<AppState>,
    axum::Extension(principal): axum::Extension<AuthenticatedPrincipal>,
    Path((agent_id, account_id)): Path<(String, String)>,
    Json(request): Json<CalendarPreviewRequest>,
) -> Response {
    if !operator(&principal) {
        return StatusCode::FORBIDDEN.into_response();
    }
    let operation_lock = state.mail_calendar_account_lock(&agent_id, &account_id);
    let _operation_guard = operation_lock.lock().await;
    let Some((account, vault)) = preview_account(&state, &agent_id, &account_id) else {
        return StatusCode::NOT_FOUND.into_response();
    };
    if account
        .access_token_expires_at
        .is_some_and(|expires| expires <= Utc::now())
    {
        return (
            StatusCode::PRECONDITION_REQUIRED,
            "Refresh this account's sign-in before previewing content.",
        )
            .into_response();
    }
    record_account_event(
        &state,
        "calendar_content_preview",
        &agent_id,
        &account_id,
        account.provider,
        &account.capabilities,
        "requested",
    );
    let client = vak_mail_calendar::provider::ProviderReadClient::default();
    let audience = format!("agent:{agent_id}");
    let range = vak_mail_calendar::provider::CalendarRange {
        from: request.from,
        to: request.to,
        limit: request.limit.unwrap_or(50),
    };
    let worker_exe = state.core.tool_worker_exe();
    let result = if let Some(calendar_id) = request.calendar_id.as_deref() {
        vak_core::mail_calendar::calendar_preview_page_in_source_with_worker(
            &client,
            &account,
            &vault,
            &agent_id,
            &audience,
            calendar_id,
            range,
            request.cursor.as_deref(),
            &worker_exe,
        )
        .await
    } else {
        vak_core::mail_calendar::calendar_preview_page_with_worker(
            &client,
            &account,
            &vault,
            &agent_id,
            &audience,
            range,
            request.cursor.as_deref(),
            &worker_exe,
        )
        .await
    };
    match result {
        Ok(events) => {
            record_account_event(
                &state,
                "calendar_content_preview",
                &agent_id,
                &account_id,
                account.provider,
                &account.capabilities,
                "succeeded",
            );
            Json(serde_json::json!({"events": events.events, "next_cursor": events.next_cursor}))
                .into_response()
        }
        Err(error) => {
            mark_preview_reauthentication(&state, &account, &error);
            record_account_event(
                &state,
                "calendar_content_preview",
                &agent_id,
                &account_id,
                account.provider,
                &account.capabilities,
                "failed",
            );
            provider_preview_error(error)
        }
    }
}

pub(super) async fn free_busy_preview(
    State(state): State<AppState>,
    axum::Extension(principal): axum::Extension<AuthenticatedPrincipal>,
    Path((agent_id, account_id)): Path<(String, String)>,
    Json(request): Json<CalendarPreviewRequest>,
) -> Response {
    if !operator(&principal) {
        return StatusCode::FORBIDDEN.into_response();
    }
    let operation_lock = state.mail_calendar_account_lock(&agent_id, &account_id);
    let _operation_guard = operation_lock.lock().await;
    let Some((account, vault)) = preview_account(&state, &agent_id, &account_id) else {
        return StatusCode::NOT_FOUND.into_response();
    };
    if account
        .access_token_expires_at
        .is_some_and(|expires| expires <= Utc::now())
    {
        return (
            StatusCode::PRECONDITION_REQUIRED,
            "Refresh this account's sign-in before previewing content.",
        )
            .into_response();
    }
    record_account_event(
        &state,
        "calendar_freebusy_preview",
        &agent_id,
        &account_id,
        account.provider,
        &account.capabilities,
        "requested",
    );
    match vak_core::mail_calendar::free_busy_with_worker(
        &vak_mail_calendar::provider::ProviderReadClient::default(),
        &account,
        &vault,
        &agent_id,
        &format!("agent:{agent_id}"),
        vak_mail_calendar::provider::CalendarRange {
            from: request.from,
            to: request.to,
            limit: 100,
        },
        &state.core.tool_worker_exe(),
    )
    .await
    {
        Ok(busy) => {
            record_account_event(
                &state,
                "calendar_freebusy_preview",
                &agent_id,
                &account_id,
                account.provider,
                &account.capabilities,
                "succeeded",
            );
            Json(serde_json::json!({"busy": busy})).into_response()
        }
        Err(error) => {
            mark_preview_reauthentication(&state, &account, &error);
            record_account_event(
                &state,
                "calendar_freebusy_preview",
                &agent_id,
                &account_id,
                account.provider,
                &account.capabilities,
                "failed",
            );
            provider_preview_error(error)
        }
    }
}

fn mark_preview_reauthentication(
    state: &AppState,
    account: &ConnectedAccount,
    error: &vak_mail_calendar::provider::ProviderReadError,
) {
    if !matches!(
        error,
        vak_mail_calendar::provider::ProviderReadError::ReauthenticationRequired
    ) {
        return;
    }
    mark_account_reauthentication_required(state, account, "provider_read_rejected");
}

fn mark_account_reauthentication_required(
    state: &AppState,
    account: &ConnectedAccount,
    outcome: &str,
) {
    if let Ok(ledger) = ConnectionLedger::for_agent(&account.owner_agent_id)
        && ledger
            .append_reauthentication_required(&account.id, Utc::now())
            .is_ok()
    {
        pause_routines_for_account(state, &account.owner_agent_id, &account.id);
        record_account_event(
            state,
            "account_reauthentication_required",
            &account.owner_agent_id,
            &account.id,
            account.provider,
            &account.capabilities,
            outcome,
        );
    }
}

fn pause_routines_for_account(state: &AppState, agent_id: &str, account_id: &str) {
    crate::automations::pause_account_routines(state, agent_id, account_id, None);
}

fn preview_account(
    state: &AppState,
    agent_id: &str,
    account_id: &str,
) -> Option<(ConnectedAccount, AccountVault)> {
    if !registered_agent(state, agent_id)
        || !valid_agent(state, agent_id)
        || Uuid::parse_str(account_id).is_err()
    {
        return None;
    }
    let ledger = ConnectionLedger::for_agent(agent_id).ok()?;
    let account = ledger
        .read_all()
        .ok()?
        .into_iter()
        .find(|account| account.id == account_id)?;
    if account.owner_agent_id != agent_id
        || account.status != AccountStatus::Connected
        || account.revoked_at.is_some()
        || !account.capabilities.iter().any(|capability| {
            matches!(
                capability,
                Capability::MailRead | Capability::CalendarRead | Capability::CalendarFreeBusy
            )
        })
    {
        return None;
    }
    let vault = AccountVault::for_agent(agent_id).ok()?;
    Some((account, vault))
}

fn provider_preview_error(error: vak_mail_calendar::provider::ProviderReadError) -> Response {
    use vak_mail_calendar::provider::ProviderReadError as E;
    let (status, kind) = match error {
        E::NotAdmitted => (StatusCode::FORBIDDEN, "not_admitted"),
        E::Unsupported => (StatusCode::NOT_IMPLEMENTED, "unsupported"),
        E::ReauthenticationRequired => (
            StatusCode::PRECONDITION_REQUIRED,
            "reauthentication_required",
        ),
        E::InvalidRange | E::InvalidResponse | E::InvalidSearch => {
            (StatusCode::BAD_REQUEST, "invalid_request")
        }
        E::WatchCursorReset => (StatusCode::CONFLICT, "watch_cursor_reset"),
        E::Vault => (StatusCode::SERVICE_UNAVAILABLE, "credential_unavailable"),
        E::Unavailable => (StatusCode::BAD_GATEWAY, "provider_unavailable"),
    };
    (
        status,
        Json(serde_json::json!({"kind": kind, "error": error.to_string()})),
    )
        .into_response()
}

fn same_provider_principal(
    provider: Provider,
    left: &AccountSecretMaterial,
    right: &AccountSecretMaterial,
) -> bool {
    if provider == Provider::AppleIcloud
        || (matches!(provider, Provider::Google | Provider::Microsoft)
            && (left.uses_app_password() || right.uses_app_password()))
    {
        left.has_same_display_identity_ignoring_ascii_case(right)
    } else {
        left.has_same_principal(right)
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
    let Ok(ledger) = ConnectionLedger::for_agent(&agent_id) else {
        return StatusCode::BAD_REQUEST.into_response();
    };
    let durable_fence = match ledger.provider_fence(request.provider) {
        Ok(fence) => fence,
        Err(_) => return StatusCode::SERVICE_UNAVAILABLE.into_response(),
    };
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
            .map(|(url, account_id)| (url, account_id, None))
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
            .map(|(url, account_id, binding)| (url, account_id, Some(binding)))
    };
    match result {
        Ok((authorization_url, account_id, binding)) => {
            if !state
                .mail_calendar_oauth
                .set_durable_fence(&account_id, durable_fence)
            {
                return StatusCode::SERVICE_UNAVAILABLE.into_response();
            }
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
struct AppPasswordConnectRequest {
    email: SensitiveInput,
    app_specific_password: SensitiveInput,
    capabilities: Vec<Capability>,
}

impl Drop for AppPasswordConnectRequest {
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

/// Verify and store a provider app password for the supported, local-only
/// read-only profile. The credential is checked against the provider's fixed
/// TLS endpoint before it is written to the owning Agent vault. iCloud app-
/// specific passwords have broader provider authority than these local grants;
/// account disconnect removes the vault credential, but copied append-only
/// session content remains subject to the documented M7 erasure gate.
pub(super) async fn connect_app_password(
    State(state): State<AppState>,
    axum::Extension(principal): axum::Extension<AuthenticatedPrincipal>,
    ConnectInfo(peer): ConnectInfo<SocketAddr>,
    OriginalUri(uri): OriginalUri,
    Path(agent_id): Path<String>,
    headers: HeaderMap,
    body: Bytes,
) -> Response {
    let provider = if uri.path().ends_with("/google-app-password") {
        Provider::Google
    } else if uri.path().ends_with("/microsoft-app-password") {
        Provider::Microsoft
    } else {
        Provider::AppleIcloud
    };
    if !operator(&principal) {
        return StatusCode::FORBIDDEN.into_response();
    }
    if !valid_agent(&state, &agent_id) {
        return StatusCode::NOT_FOUND.into_response();
    }
    let Some(_provider_guard) = active_provider_link_guard(&state, &agent_id, provider).await
    else {
        return StatusCode::NOT_FOUND.into_response();
    };
    if state.core.config().server.public_url.is_some() || !is_loopback_request(&headers, peer) {
        return (
            StatusCode::FORBIDDEN,
            "App passwords can only be added to a local server.",
        )
            .into_response();
    }
    if body.len() > 4096 {
        wipe_request_bytes(body);
        return StatusCode::PAYLOAD_TOO_LARGE.into_response();
    }
    let parsed = serde_json::from_slice::<AppPasswordConnectRequest>(&body);
    wipe_request_bytes(body);
    let mut request = match parsed {
        Ok(request) => request,
        Err(_) => return StatusCode::BAD_REQUEST.into_response(),
    };
    let email = Zeroizing::new(request.email.0.trim().to_owned());
    request.email.0.zeroize();
    let valid_email = valid_provider_email(&email);
    let valid_password = valid_provider_app_password(provider, &request.app_specific_password.0);
    let capabilities = request
        .capabilities
        .iter()
        .copied()
        .collect::<BTreeSet<_>>();
    let supported_read_only = capabilities.len() == request.capabilities.len()
        && verifiable_app_password_selection(provider, &capabilities);
    let microsoft_personal_email =
        provider != Provider::Microsoft || valid_microsoft_personal_email(&email);
    let audit_capabilities = capabilities.clone();
    let verifies_mail = capabilities.contains(&Capability::MailRead);
    let verifies_icloud_calendar = provider == Provider::AppleIcloud
        && capabilities.iter().any(|capability| {
            matches!(
                capability,
                Capability::CalendarRead | Capability::CalendarFreeBusy
            )
        });
    if !valid_email || !valid_password || !supported_read_only || !microsoft_personal_email {
        request.app_specific_password.0.zeroize();
        return (
            StatusCode::BAD_REQUEST,
            "Enter a valid provider email, app password, and supported read-only access selection.",
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
    let mut app_password = std::mem::take(&mut *request.app_specific_password.0);
    if provider == Provider::Google {
        app_password.retain(|character| character != ' ');
    }
    let material = match AccountSecretMaterial::new(
        email.to_string(),
        Some(email.to_string()),
        None,
        None,
        None,
        Some(email.to_string()),
        Some(app_password),
    ) {
        Ok(material) => material,
        Err(_) => {
            request.app_specific_password.0.zeroize();
            return StatusCode::BAD_REQUEST.into_response();
        }
    };
    // Avoid sending credentials to the provider for a link that is already
    // known to be active. The conditional append below remains the
    // cross-process authority for races that occur after this read.
    let existing_provider_links = match ledger.read_all() {
        Ok(links) => links,
        Err(_) => return StatusCode::SERVICE_UNAVAILABLE.into_response(),
    };
    for linked in existing_provider_links
        .iter()
        .filter(|linked| linked.provider == provider && linked.revoked_at.is_none())
    {
        if linked.status == AccountStatus::Pending {
            return (
                StatusCode::CONFLICT,
                "Another account connection for this provider is still pending. Finish its cleanup before trying again.",
            )
                .into_response();
        }
        if matches!(
            linked.status,
            AccountStatus::Connected
                | AccountStatus::ConnectedUnverified
                | AccountStatus::ReauthenticationRequired
        ) {
            let stored = match vault.load(&linked.id) {
                Ok(stored) => stored,
                Err(_) => return StatusCode::SERVICE_UNAVAILABLE.into_response(),
            };
            if same_provider_principal(provider, &stored, &material) {
                return (
                    StatusCode::CONFLICT,
                    "This account is already connected to this Agent. Disconnect it before changing its access selection.",
                )
                    .into_response();
            }
        }
    }
    if verifies_mail && matches!(provider, Provider::Google | Provider::Microsoft) {
        let (imap_login, imap_password) = match material.icloud_imap_credentials() {
            Ok(credentials) => credentials,
            Err(_) => return StatusCode::BAD_REQUEST.into_response(),
        };
        let verification = if provider == Provider::Google {
            vak_mail_calendar::provider::verify_google_imap_credentials(
                imap_login.as_str(),
                imap_password.as_str(),
            )
            .await
        } else {
            vak_mail_calendar::provider::verify_microsoft_imap_credentials(
                imap_login.as_str(),
                imap_password.as_str(),
            )
            .await
        };
        if verification.is_err() {
            let message = if provider == Provider::Google {
                "Gmail App Password sign-in could not be verified. Check the email and app password."
            } else {
                "Outlook.com app-password sign-in could not be verified. Check that this is a personal Outlook.com, Live, Hotmail, or MSN account and that its provider still permits IMAP app-password sign-in."
            };
            return (StatusCode::UNAUTHORIZED, message).into_response();
        }
    } else if verifies_mail {
        let (imap_login, imap_password) = match material.icloud_imap_credentials() {
            Ok(credentials) => credentials,
            Err(_) => return StatusCode::BAD_REQUEST.into_response(),
        };
        if vak_mail_calendar::provider::verify_icloud_mail_credentials(
            imap_login.as_str(),
            imap_password.as_str(),
        )
        .await
        .is_err()
        {
            return (
                StatusCode::UNAUTHORIZED,
                "iCloud Mail sign-in could not be verified. Check the email and app-specific password.",
            )
                .into_response();
        }
    }
    if verifies_icloud_calendar {
        let (caldav_login, caldav_password) = match material.icloud_imap_credentials() {
            Ok(credentials) => credentials,
            Err(_) => return StatusCode::BAD_REQUEST.into_response(),
        };
        if vak_mail_calendar::provider::verify_icloud_calendar_credentials(
            caldav_login.as_str(),
            caldav_password.as_str(),
        )
        .await
        .is_err()
        {
            return (
                StatusCode::UNAUTHORIZED,
                "iCloud Calendar sign-in could not be verified. Check the email and app-specific password.",
            )
                .into_response();
        }
    }
    let now = Utc::now();
    let mut account = ConnectedAccount {
        id: account_id.clone(),
        provider,
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
        for linked in existing
            .iter()
            .filter(|linked| linked.provider == provider && linked.revoked_at.is_none())
        {
            if linked.status == AccountStatus::Pending {
                return Err(AppPasswordPendingCheckError::LinkInProgress);
            }
            if !matches!(
                linked.status,
                AccountStatus::Connected
                    | AccountStatus::ConnectedUnverified
                    | AccountStatus::ReauthenticationRequired
            ) {
                continue;
            }
            let stored = vault
                .load(&linked.id)
                .map_err(|_| AppPasswordPendingCheckError::VaultUnavailable)?;
            if same_provider_principal(provider, &stored, &material) {
                return Err(AppPasswordPendingCheckError::AlreadyConnected);
            }
        }
        Ok(())
    }) {
        Ok(()) => {}
        Err(vak_mail_calendar::connection_ledger::ConditionalAppendError::Check(
            AppPasswordPendingCheckError::AlreadyConnected,
        )) => {
            request.app_specific_password.0.zeroize();
            return (
                StatusCode::CONFLICT,
                "This account is already connected to this Agent. Disconnect it before changing its access selection.",
            )
                .into_response();
        }
        Err(vak_mail_calendar::connection_ledger::ConditionalAppendError::Check(
            AppPasswordPendingCheckError::LinkInProgress,
        )) => {
            request.app_specific_password.0.zeroize();
            return (
                StatusCode::CONFLICT,
                "Another account connection for this provider is still pending. Finish its cleanup before trying again.",
            )
                .into_response();
        }
        Err(_) => {
            request.app_specific_password.0.zeroize();
            return StatusCode::SERVICE_UNAVAILABLE.into_response();
        }
    }
    // This account status covers every selected capability. Mark it connected
    // only after each requested fixed-host protocol check has succeeded.
    account.status = if supported_read_only {
        AccountStatus::Connected
    } else {
        AccountStatus::ConnectedUnverified
    };
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
        provider,
        &audit_capabilities,
        if supported_read_only {
            "connected"
        } else {
            "connected_unverified"
        },
    );
    Json(serde_json::json!({ "connected": true })).into_response()
}

pub(crate) fn is_loopback_request(headers: &HeaderMap, peer: SocketAddr) -> bool {
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
enum AppPasswordPendingCheckError {
    AlreadyConnected,
    LinkInProgress,
    VaultUnavailable,
}

fn wipe_request_bytes(body: Bytes) {
    if let Ok(mut bytes) = body.try_into_mut() {
        bytes.as_mut().zeroize();
    }
}

fn valid_provider_email(email: &str) -> bool {
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

fn verifiable_app_password_selection(
    provider: Provider,
    capabilities: &BTreeSet<Capability>,
) -> bool {
    if capabilities.is_empty() {
        return false;
    }
    match provider {
        Provider::Google | Provider::Microsoft => {
            capabilities.len() == 1 && capabilities.contains(&Capability::MailRead)
        }
        Provider::AppleIcloud => capabilities.iter().all(|capability| {
            matches!(
                capability,
                Capability::MailRead | Capability::CalendarRead | Capability::CalendarFreeBusy
            )
        }),
    }
}

fn valid_provider_app_password(provider: Provider, password: &str) -> bool {
    match provider {
        Provider::Google => {
            password
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || byte == b' ')
                && password.bytes().filter(u8::is_ascii_alphanumeric).count() == 16
        }
        Provider::AppleIcloud => {
            (16..=32).contains(&password.len())
                && password
                    .bytes()
                    .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-')
        }
        Provider::Microsoft => {
            (8..=64).contains(&password.len())
                && password.bytes().all(|byte| byte.is_ascii_graphic())
        }
    }
}

fn valid_microsoft_personal_email(email: &str) -> bool {
    email.rsplit_once('@').is_some_and(|(_, domain)| {
        matches!(
            domain.to_ascii_lowercase().as_str(),
            "outlook.com" | "hotmail.com" | "live.com" | "msn.com"
        )
    })
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
        &state.core.scope(),
        vak_core::security_events::EventKind::MailCalendarAccount,
        label,
        &detail,
        None,
    );
}

#[cfg(test)]
#[cfg_attr(not(feature = "test-support"), allow(unused_imports))]
mod tests {
    use super::{
        OAUTH_CALLBACK_COOKIE, due_calendar_occurrence_keys, is_loopback_request,
        oauth_callback_cookie, oauth_callback_page, oauth_callback_set_cookie, oauth_callback_uri,
        pause_routines_for_account, prepare_routine_account, refresh_routine_account_if_needed,
        registered_agent, same_provider_principal, valid_agent, valid_microsoft_personal_email,
        valid_provider_app_password, valid_provider_email, verifiable_app_password_selection,
    };
    use axum::http::{HeaderMap, HeaderValue, StatusCode, header};
    use chrono::Utc;
    use std::collections::BTreeSet;
    use uuid::Uuid;
    use vak_mail_calendar::connection_ledger::ConnectionLedger;
    use vak_mail_calendar::vault::{AccountSecretMaterial, AccountVault};
    use vak_mail_calendar::{
        AccountStatus, CalendarEventBoundary, CalendarEventTrigger, Capability, ConnectedAccount,
        Provider, RoutineOperation, RoutineScope,
    };

    #[test]
    fn large_simulated_calendar_batch_filters_due_timed_occurrences_only() {
        use chrono::{Duration, TimeZone, Utc};
        use vak_mail_calendar::provider::CalendarItem;

        let now = Utc.with_ymd_and_hms(2026, 10, 1, 12, 0, 0).unwrap();
        let checked_after = now - Duration::minutes(2);
        let trigger = CalendarEventTrigger {
            boundary: CalendarEventBoundary::Start,
            offset_minutes: 5,
            max_lateness_minutes: 2,
        };
        let make_event = |id: String, starts_at: Option<chrono::DateTime<Utc>>| CalendarItem {
            provider_id: id,
            version: None,
            title: "private subject must not enter occurrence key".into(),
            starts_at,
            ends_at: starts_at.map(|start| start + Duration::minutes(30)),
            starts_on: None,
            ends_on: None,
            all_day: starts_at.is_none(),
            location: None,
            description: None,
            attendee_count: 0,
            recurring: false,
            private: false,
            can_cancel: false,
            can_respond: false,
        };
        let mut events = (0..1_500)
            .map(|index| make_event(format!("due-{index}"), Some(now + Duration::minutes(4))))
            .collect::<Vec<_>>();
        events.extend((0..200).map(|index| {
            make_event(
                format!("too-late-{index}"),
                Some(now + Duration::minutes(2)),
            )
        }));
        events.extend(
            (0..200).map(|index| {
                make_event(format!("future-{index}"), Some(now + Duration::minutes(6)))
            }),
        );
        events.extend((0..10).map(|index| make_event(format!("all-day-{index}"), None)));

        let keys = due_calendar_occurrence_keys(&events, trigger, checked_after, now);
        assert_eq!(keys.len(), 1_500);
        assert!(keys.iter().all(|key| key.starts_with("calendar:")));
        assert!(keys.iter().all(|key| !key.contains("private")));
        assert_eq!(keys.iter().collect::<BTreeSet<_>>().len(), 1_500);
    }

    #[test]
    fn app_password_validation_is_provider_specific_and_accepts_google_display_spacing() {
        assert!(valid_provider_app_password(
            Provider::Google,
            "abcd efgh ijkl mnop"
        ));
        assert!(valid_provider_app_password(
            Provider::Google,
            "abcdefghijklmnop"
        ));
        assert!(!valid_provider_app_password(
            Provider::Google,
            "ordinary-account-password"
        ));
        assert!(!valid_provider_app_password(
            Provider::Google,
            "abcd\nefghijklmnop"
        ));
        assert!(valid_provider_app_password(
            Provider::AppleIcloud,
            "abcd-efgh-ijkl-mnop"
        ));
        assert!(valid_provider_app_password(
            Provider::Microsoft,
            "a1b2c3d4e5f6g7h8"
        ));
        assert!(!valid_provider_app_password(Provider::Microsoft, "short"));
        assert!(!valid_provider_app_password(
            Provider::Microsoft,
            "not safe\npassword"
        ));
        assert!(valid_microsoft_personal_email("person@live.com"));
        assert!(valid_microsoft_personal_email("person@outlook.com"));
        assert!(!valid_microsoft_personal_email("person@company.com"));
    }

    #[test]
    fn app_password_selection_verifies_every_supported_icloud_read_independently() {
        for capability in [
            Capability::MailRead,
            Capability::CalendarRead,
            Capability::CalendarFreeBusy,
        ] {
            assert!(verifiable_app_password_selection(
                Provider::AppleIcloud,
                &BTreeSet::from([capability]),
            ));
        }
        assert!(verifiable_app_password_selection(
            Provider::AppleIcloud,
            &BTreeSet::from([Capability::MailRead, Capability::CalendarRead]),
        ));
        assert!(verifiable_app_password_selection(
            Provider::AppleIcloud,
            &BTreeSet::from([Capability::MailRead, Capability::CalendarFreeBusy]),
        ));
        assert!(verifiable_app_password_selection(
            Provider::AppleIcloud,
            &BTreeSet::from([Capability::CalendarRead, Capability::CalendarFreeBusy]),
        ));
        assert!(verifiable_app_password_selection(
            Provider::AppleIcloud,
            &BTreeSet::from([
                Capability::MailRead,
                Capability::CalendarRead,
                Capability::CalendarFreeBusy,
            ]),
        ));
        assert!(verifiable_app_password_selection(
            Provider::Google,
            &BTreeSet::from([Capability::MailRead]),
        ));
        assert!(!verifiable_app_password_selection(
            Provider::Google,
            &BTreeSet::from([Capability::MailRead, Capability::CalendarRead]),
        ));
        assert!(!verifiable_app_password_selection(
            Provider::Microsoft,
            &BTreeSet::from([Capability::MailRead, Capability::CalendarFreeBusy]),
        ));
        assert!(!verifiable_app_password_selection(
            Provider::AppleIcloud,
            &BTreeSet::from([Capability::MailRead, Capability::MailSend]),
        ));
        assert!(!verifiable_app_password_selection(
            Provider::AppleIcloud,
            &BTreeSet::new(),
        ));
    }

    #[test]
    fn oauth_and_app_password_links_for_same_email_are_duplicates() {
        let oauth = AccountSecretMaterial::new(
            "google:opaque-subject".into(),
            Some("owner@gmail.com".into()),
            None,
            Some("access-token".into()),
            Some("refresh-token".into()),
            None,
            None,
        )
        .unwrap();
        let app_password = AccountSecretMaterial::new(
            "OWNER@gmail.com".into(),
            Some("OWNER@gmail.com".into()),
            None,
            None,
            None,
            Some("OWNER@gmail.com".into()),
            Some("abcdefghijklmnop".into()),
        )
        .unwrap();
        assert!(same_provider_principal(
            Provider::Google,
            &oauth,
            &app_password
        ));
        assert!(same_provider_principal(
            Provider::Microsoft,
            &oauth,
            &app_password
        ));
    }

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
        assert!(valid_provider_email("owner+vak@icloud.com"));
        assert!(valid_provider_email("name@me.com"));
        assert!(valid_provider_email("user@gmail.com"));
        assert!(!valid_provider_email("name@@icloud.com"));
        assert!(!valid_provider_email("name@-icloud.com"));
        assert!(!valid_provider_email("name@icloud..com"));
        assert!(!valid_provider_email("nämé@icloud.com"));
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

        crate::agents::save(core.cwd(), std::slice::from_ref(&agent)).unwrap();
        assert!(valid_agent(&state, "mail-owner"));

        agent.lifecycle = crate::agents::AgentLifecycle::Paused;
        crate::agents::save(core.cwd(), std::slice::from_ref(&agent)).unwrap();
        assert!(!valid_agent(&state, "mail-owner"));
        assert!(registered_agent(&state, "mail-owner"));

        agent.lifecycle = crate::agents::AgentLifecycle::Archived;
        crate::agents::save(core.cwd(), std::slice::from_ref(&agent)).unwrap();
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
    fn reauthentication_pauses_only_routines_for_that_agent_account_pair() {
        let state = crate::test_support::state();
        let shared = state.core.shared_scope();
        let accounts = [
            uuid::Uuid::now_v7().to_string(),
            uuid::Uuid::now_v7().to_string(),
        ];
        let routine = |agent: &str, account: &str| {
            let id = vak_session::ids::TriggerId::new();
            let trigger = vak_core::triggers::Trigger {
                id,
                name: format!("{agent} {account}"),
                agent: agent.into(),
                agent_revision: Some(1),
                space: "spc_test".into(),
                enabled: true,
                kind: vak_core::triggers::TriggerKind::Manual,
                action: vak_core::triggers::TriggerAction::Prompt {
                    text: "summarise".into(),
                    model_pin: None,
                },
                deliver_to: None,
                on_crash: vak_core::triggers::OnCrash::Skip,
                scope: Some(
                    serde_json::from_value(serde_json::json!({
                        "routine_id": id.uuid().to_string(),
                        "account_id": account,
                        "operations": ["recent_mail"],
                        "max_items": 5,
                        "watch_new_mail": true
                    }))
                    .unwrap(),
                ),
                created_at: Utc::now(),
                created_by: None,
            };
            vak_core::triggers::create(&shared, &trigger).unwrap();
            id.to_string()
        };
        let same = routine("agent-a", &accounts[0]);
        let other_account = routine("agent-a", &accounts[1]);
        let other_agent = routine("agent-b", &accounts[0]);

        pause_routines_for_account(&state, "agent-a", &accounts[0]);

        let enabled = |id: &str| {
            vak_core::triggers::get(&shared, id)
                .unwrap()
                .unwrap()
                .enabled
        };
        assert!(!enabled(&same));
        assert!(enabled(&other_account));
        assert!(enabled(&other_agent));
    }

    // Needs the loopback OAuth endpoint, which exists only with `test-support`.
    #[cfg(feature = "test-support")]
    #[tokio::test]
    async fn expired_routine_tokens_refresh_or_pause_before_use() {
        vak_config::paths::isolate_home_for_tests();
        let state = crate::test_support::state();
        let agent_id = "mail-refresh-owner";
        let agent = crate::agents::find_template("writer")
            .unwrap()
            .to_agent_definition(agent_id, None);
        crate::agents::save(state.core.cwd(), std::slice::from_ref(&agent)).unwrap();

        let account_id = Uuid::now_v7().to_string();
        let credential_ref = AccountVault::credential_ref(&account_id).unwrap();
        let vault = AccountVault::for_agent(agent_id).unwrap();
        vault
            .store(
                &account_id,
                AccountSecretMaterial::new(
                    "google:synthetic-subject".into(),
                    Some("owner@example.test".into()),
                    Some("synthetic-client".into()),
                    Some("expired-access-token".into()),
                    None,
                    None,
                    None,
                )
                .unwrap(),
            )
            .unwrap();
        let ledger = ConnectionLedger::for_agent(agent_id).unwrap();
        let audience = format!("agent:{agent_id}");
        let mut account = ConnectedAccount {
            id: account_id.clone(),
            provider: Provider::Google,
            status: AccountStatus::Pending,
            owner_agent_id: agent_id.into(),
            allowed_audiences: [audience].into_iter().collect(),
            capabilities: [Capability::MailRead].into_iter().collect(),
            provider_scopes: ["https://www.googleapis.com/auth/gmail.readonly".into()]
                .into_iter()
                .collect(),
            credential_ref: credential_ref.clone(),
            principal_ref: credential_ref,
            revision: 1,
            connected_at: Utc::now() - chrono::Duration::hours(1),
            access_token_expires_at: Some(Utc::now() - chrono::Duration::seconds(1)),
            refresh_token_available: false,
            revoked_at: None,
        };
        ledger.append_pending(account.clone()).unwrap();
        account.status = AccountStatus::Connected;
        account.revision = 2;
        ledger.append_connected(account.clone()).unwrap();

        let refresh_account_id = Uuid::now_v7().to_string();
        let refresh_credential_ref = AccountVault::credential_ref(&refresh_account_id).unwrap();
        vault
            .store(
                &refresh_account_id,
                AccountSecretMaterial::new(
                    "google:synthetic-refresh-subject".into(),
                    Some("refresh@example.test".into()),
                    Some("synthetic-client".into()),
                    Some("old-access-token".into()),
                    Some("old-refresh-token".into()),
                    None,
                    None,
                )
                .unwrap(),
            )
            .unwrap();
        let mut refresh_account = account;
        refresh_account.id = refresh_account_id.clone();
        refresh_account.credential_ref = refresh_credential_ref.clone();
        refresh_account.principal_ref = refresh_credential_ref;
        refresh_account.revision = 1;
        refresh_account.status = AccountStatus::Pending;
        refresh_account.refresh_token_available = true;
        ledger.append_pending(refresh_account.clone()).unwrap();
        refresh_account.status = AccountStatus::Connected;
        refresh_account.revision = 2;
        ledger.append_connected(refresh_account).unwrap();

        async fn refresh_endpoint(
            axum::extract::State(calls): axum::extract::State<
                std::sync::Arc<std::sync::atomic::AtomicUsize>,
            >,
            axum::Form(form): axum::Form<std::collections::HashMap<String, String>>,
        ) -> axum::Json<serde_json::Value> {
            assert_eq!(
                form.get("client_id").map(String::as_str),
                Some("synthetic-client")
            );
            let expected_refresh_token =
                if calls.fetch_add(1, std::sync::atomic::Ordering::SeqCst) == 0 {
                    "old-refresh-token"
                } else {
                    "rotated-refresh-token"
                };
            assert_eq!(
                form.get("refresh_token").map(String::as_str),
                Some(expected_refresh_token)
            );
            assert_eq!(
                form.get("grant_type").map(String::as_str),
                Some("refresh_token")
            );
            axum::Json(serde_json::json!({
                "access_token": "rotated-access-token",
                "refresh_token": "rotated-refresh-token",
                "token_type": "Bearer",
                "expires_in": 3600,
                "scope": "https://www.googleapis.com/auth/gmail.readonly"
            }))
        }
        let calls = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let app = axum::Router::new()
            .route("/token", axum::routing::post(refresh_endpoint))
            .with_state(calls.clone());
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let token_url = format!("http://{}/token", listener.local_addr().unwrap());
        let server = tokio::spawn(async move {
            axum::serve(listener, app).await.unwrap();
        });
        state
            .mail_calendar_test_refresh_endpoints
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .insert(Provider::Google, token_url);

        let scope = RoutineScope {
            routine_id: Uuid::now_v7().to_string(),
            account_id: refresh_account_id.clone(),
            mail_folder_id: None,
            calendar_source_id: None,
            operations: [RoutineOperation::RecentMail].into_iter().collect(),
            max_items: 5,
            watch_new_mail: true,
            read_commitments: false,
            calendar_event_trigger: None,
        };
        prepare_routine_account(&state, agent_id, &scope)
            .await
            .unwrap();
        assert_eq!(
            vault.access_token(&refresh_account_id).unwrap().as_str(),
            "rotated-access-token"
        );
        let persisted_account = ledger
            .read_all()
            .unwrap()
            .into_iter()
            .find(|account| account.id == refresh_account_id)
            .unwrap();
        assert_eq!(persisted_account.revision, 3);
        assert!(persisted_account.access_token_expires_at.unwrap() > Utc::now());

        let mut expiring_again = persisted_account;
        expiring_again.revision += 1;
        expiring_again.access_token_expires_at = Some(Utc::now() + chrono::Duration::minutes(1));
        ledger
            .append_connected_if_current(expiring_again, 3, || {
                Ok::<(), std::convert::Infallible>(())
            })
            .unwrap();
        prepare_routine_account(&state, agent_id, &scope)
            .await
            .unwrap();
        let refreshed_again = ledger
            .read_all()
            .unwrap()
            .into_iter()
            .find(|account| account.id == refresh_account_id)
            .unwrap();
        assert_eq!(refreshed_again.revision, 5);
        assert_eq!(calls.load(std::sync::atomic::Ordering::SeqCst), 2);
        assert_eq!(
            vault.access_token(&refresh_account_id).unwrap().as_str(),
            "rotated-access-token"
        );
        server.abort();

        let routine_id = vak_session::ids::TriggerId::new();
        let routine = vak_core::triggers::Trigger {
            id: routine_id,
            name: "expired mail watch".into(),
            agent: agent_id.into(),
            agent_revision: Some(1),
            space: vak_config::spaces::key(state.core.cwd()),
            enabled: true,
            kind: vak_core::triggers::TriggerKind::Manual,
            action: vak_core::triggers::TriggerAction::Prompt {
                text: "watch".into(),
                model_pin: None,
            },
            deliver_to: None,
            on_crash: vak_core::triggers::OnCrash::Skip,
            scope: Some(
                serde_json::from_value(serde_json::json!({
                    "routine_id": routine_id.uuid().to_string(),
                    "account_id": account_id,
                    "operations": ["recent_mail"],
                    "max_items": 5,
                    "watch_new_mail": true
                }))
                .unwrap(),
            ),
            created_at: Utc::now(),
            created_by: None,
        };
        vak_core::triggers::create(&state.core.shared_scope(), &routine).unwrap();

        let result = refresh_routine_account_if_needed(&state, agent_id, &account_id).await;
        assert!(result.is_err());
        assert_eq!(
            ledger
                .read_all()
                .unwrap()
                .iter()
                .find(|account| account.id == account_id)
                .unwrap()
                .status,
            AccountStatus::ReauthenticationRequired
        );
        assert!(
            !vak_core::triggers::get(&state.core.shared_scope(), &routine_id.to_string())
                .unwrap()
                .unwrap()
                .enabled
        );
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
        crate::agents::save(core.cwd(), std::slice::from_ref(&agent)).unwrap();

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
        crate::agents::save(core.cwd(), std::slice::from_ref(&agent)).unwrap();
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
        state.mail_calendar_oauth.with_current(&grant, || {
            redeemed.persist_fenced(&vault, &ledger, grant.durable_fence())
        })
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
    let Ok(vault) = AccountVault::for_agent(&agent_id) else {
        return StatusCode::BAD_REQUEST.into_response();
    };
    let _refresh_lease = match vault.try_acquire_account_refresh_lease(&account_id) {
        Ok(Some(lease)) => lease,
        Ok(None) => {
            return (
                StatusCode::CONFLICT,
                "Another local service is refreshing this account. Retry shortly.",
            )
                .into_response();
        }
        Err(_) => return StatusCode::SERVICE_UNAVAILABLE.into_response(),
    };
    let account = match ledger.read_all() {
        Ok(accounts) => accounts.into_iter().find(|account| {
            account.id == account_id
                && matches!(
                    account.status,
                    AccountStatus::Connected | AccountStatus::ConnectedUnverified
                )
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
                pause_routines_for_account(&state, &agent_id, &account_id);
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
    let auth_method = vault.load(&refreshed.id).ok().map(|material| {
        if material.uses_app_password() {
            "app_password"
        } else {
            "oauth"
        }
    });
    Json(serde_json::json!({
        "account": AccountView {
            id: refreshed.id,
            provider: refreshed.provider,
            status: refreshed.status,
            identity_masked,
            auth_method,
            credential_available: true,
            superseded_by_active_link: false,
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
    disconnect(&state, &agent_id, &account_id).await
}

/// Disconnects one of an Agent's accounts: the tombstone that fences local
/// use, provider revocation where there is one, the credentials, and the
/// account's routines. Shared by the owner's Disconnect and by revoking the
/// Agent.
pub(crate) async fn disconnect(state: &AppState, agent_id: &str, account_id: &str) -> Response {
    let Ok(ledger) = ConnectionLedger::for_agent(agent_id) else {
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
    let provider_lock = state.mail_calendar_provider_lock(agent_id, account.provider);
    let _provider_guard = provider_lock.lock().await;
    let operation_lock = state.mail_calendar_account_lock(agent_id, account_id);
    let _operation_guard = operation_lock.lock().await;
    if !registered_agent(state, agent_id) {
        return StatusCode::NOT_FOUND.into_response();
    }
    let Ok(vault) = AccountVault::for_agent(agent_id) else {
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
        .cancel_provider(agent_id, account.provider);
    let mut already_disconnected = account.revoked_at.is_some();
    if !already_disconnected
        && ledger
            .append_disconnected(account_id, account.provider, chrono::Utc::now())
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
    if vault.remove(account_id).is_err() {
        record_account_event(
            state,
            "account_disconnect_cleanup_pending",
            agent_id,
            account_id,
            account.provider,
            &account.capabilities,
            "credential_removal_failed",
        );
        return StatusCode::SERVICE_UNAVAILABLE.into_response();
    }
    crate::automations::pause_account_routines(
        state,
        agent_id,
        account_id,
        Some("Paused because its linked account was disconnected."),
    );
    record_account_event(
        state,
        "account_disconnected",
        agent_id,
        account_id,
        account.provider,
        &account.capabilities,
        provider_revocation,
    );
    Json(serde_json::json!({ "disconnected": true, "already_disconnected": already_disconnected, "provider_grant_revoked": provider_grant_revoked, "provider_revocation": provider_revocation, "content_erased": false })).into_response()
}

fn operator(principal: &AuthenticatedPrincipal) -> bool {
    matches!(principal, AuthenticatedPrincipal::Operator)
}

pub(crate) fn valid_agent(state: &AppState, agent_id: &str) -> bool {
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
