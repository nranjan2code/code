//! Fixed-host, non-retrying provider effects.
//!
//! The caller owns the durable single-use claim and must record `Unknown`
//! after an ambiguous result. This client never follows redirects or retries.

use crate::{
    CalendarDraft, Capability, ConnectedAccount, MailAddress, MailDraft, ProposedAction, Provider,
    provider::ProviderReadError, vault::AccountVault,
};
use base64::Engine;
use reqwest::{Response, StatusCode};
use serde_json::{Value, json};
use std::time::Duration;

const MAX_RESPONSE_BYTES: usize = 64 * 1024;
const GOOGLE_ACTION_PROPERTY: &str = "vak_action_id";
const GRAPH_ACTION_PROPERTY_ID: &str =
    "String {66f5a359-4659-4830-9070-00040ec6ac6e} Name vakActionId";
const MAIL_ACTION_HEADER: &str = "X-Vak-Action-ID";

/// Provider/action support is checked before candidates are persisted or a
/// single-use effect claim is written. Keep this matrix aligned with the
/// adapters below: Microsoft Graph event mutation stays disabled until a
/// conditional-write contract suitable for stale-review protection is
/// verified.
pub fn supports_action(provider: Provider, action: &ProposedAction) -> bool {
    match action {
        ProposedAction::SendMail { .. } | ProposedAction::CreateEvent { .. } => {
            matches!(provider, Provider::Google | Provider::Microsoft)
        }
        ProposedAction::UpdateEvent { .. } | ProposedAction::CancelEvent { .. } => {
            provider == Provider::Google
        }
        ProposedAction::RespondToEvent { .. } => false,
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProviderAcceptance {
    pub provider_item_id: Option<String>,
}

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum ProviderEffectError {
    #[error("the connected account does not grant this effect")]
    NotAdmitted,
    #[error("this provider does not support this effect")]
    Unsupported,
    #[error("the provider needs sign-in again")]
    ReauthorizationRequired,
    #[error("the provider rejected the effect")]
    Rejected,
    #[error("the provider event changed since it was reviewed")]
    Conflict,
    #[error("the provider outcome is unknown; this action must not be retried")]
    Unknown,
}

impl From<ProviderReadError> for ProviderEffectError {
    fn from(error: ProviderReadError) -> Self {
        match error {
            ProviderReadError::NotAdmitted => Self::NotAdmitted,
            ProviderReadError::Unsupported => Self::Unsupported,
            ProviderReadError::ReauthenticationRequired => Self::ReauthorizationRequired,
            ProviderReadError::Vault => Self::ReauthorizationRequired,
            ProviderReadError::Unavailable | ProviderReadError::InvalidResponse => Self::Unknown,
            ProviderReadError::InvalidRange
            | ProviderReadError::InvalidSearch
            | ProviderReadError::WatchCursorReset => Self::Rejected,
        }
    }
}

#[derive(Clone)]
pub struct ProviderEffectClient {
    http: reqwest::Client,
    google_gmail_base: String,
    google_calendar_base: String,
    microsoft_graph_base: String,
}

impl ProviderEffectClient {
    pub fn new() -> Result<Self, reqwest::Error> {
        let http = reqwest::Client::builder()
            .no_proxy()
            .redirect(reqwest::redirect::Policy::none())
            .timeout(Duration::from_secs(20))
            .build()?;
        Ok(Self {
            http,
            google_gmail_base: "https://gmail.googleapis.com/gmail/v1".into(),
            google_calendar_base: "https://www.googleapis.com/calendar/v3".into(),
            microsoft_graph_base: "https://graph.microsoft.com/v1.0".into(),
        })
    }

    /// Send a plain-text email, or reply to a selected provider message.
    pub async fn send_mail(
        &self,
        account: &ConnectedAccount,
        vault: &AccountVault,
        agent_id: &str,
        audience: &str,
        attempt_id: &str,
        draft: &MailDraft,
    ) -> Result<ProviderAcceptance, ProviderEffectError> {
        let is_reply = draft.reply_to_message_id.is_some();
        if vault.agent_id() != account.owner_agent_id
            || !account.admits(agent_id, audience, Capability::MailSend)
            || (is_reply && !account.admits(agent_id, audience, Capability::MailRead))
        {
            return Err(ProviderEffectError::NotAdmitted);
        }
        if account.provider == Provider::AppleIcloud {
            return Err(ProviderEffectError::Unsupported);
        }
        validate_mail_draft(draft)?;
        if !valid_attempt_id(attempt_id) {
            return Err(ProviderEffectError::Rejected);
        }
        let token = vault
            .access_token(&account.id)
            .map_err(|_| ProviderEffectError::ReauthorizationRequired)?;
        match account.provider {
            Provider::Google => {
                let body = if is_reply {
                    let source_id = draft
                        .reply_to_message_id
                        .as_deref()
                        .ok_or(ProviderEffectError::Rejected)?;
                    let thread_id = draft
                        .reply_to_thread_id
                        .as_deref()
                        .ok_or(ProviderEffectError::Rejected)?;
                    let source_url = path_url(
                        &self.google_gmail_base,
                        &["users", "me", "messages", source_id],
                    )?;
                    let source_response = self
                        .http
                        .get(source_url)
                        .bearer_auth(token.as_str())
                        .query(&[
                            ("format", "metadata"),
                            ("metadataHeaders", "Subject"),
                            ("metadataHeaders", "Message-ID"),
                            ("metadataHeaders", "References"),
                        ])
                        .send()
                        .await
                        .map_err(|_| ProviderEffectError::Unknown)?;
                    let source = checked_json(source_response).await?;
                    let headers = validate_google_reply_source(
                        &source,
                        source_id,
                        thread_id,
                        &draft.subject,
                    )?;
                    let raw = google_raw_reply(draft, &headers, attempt_id);
                    json!({"threadId": thread_id, "raw": base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(raw.as_bytes())})
                } else {
                    let raw = google_raw_message(draft, attempt_id);
                    json!({"raw": base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(raw.as_bytes())})
                };
                let response = self
                    .http
                    .post(format!("{}/users/me/messages/send", self.google_gmail_base))
                    .bearer_auth(token.as_str())
                    .json(&body)
                    .send()
                    .await
                    .map_err(|_| ProviderEffectError::Unknown)?;
                classify_google_response(response).await
            }
            Provider::Microsoft => {
                let response = if is_reply {
                    let source_id = draft
                        .reply_to_message_id
                        .as_deref()
                        .ok_or(ProviderEffectError::Rejected)?;
                    let thread_id = draft
                        .reply_to_thread_id
                        .as_deref()
                        .ok_or(ProviderEffectError::Rejected)?;
                    let source_url =
                        path_url(&self.microsoft_graph_base, &["me", "messages", source_id])?;
                    let source_response = self
                        .http
                        .get(source_url)
                        .bearer_auth(token.as_str())
                        .query(&[("$select", "id,conversationId,subject")])
                        .send()
                        .await
                        .map_err(|_| ProviderEffectError::Unknown)?;
                    let source = checked_json(source_response).await?;
                    validate_graph_reply_source(&source, source_id, thread_id, &draft.subject)?;
                    let payload = graph_reply_payload(draft, attempt_id);
                    self.http
                        .post(path_url(
                            &self.microsoft_graph_base,
                            &["me", "messages", source_id, "reply"],
                        )?)
                        .bearer_auth(token.as_str())
                        .json(&payload)
                        .send()
                        .await
                        .map_err(|_| ProviderEffectError::Unknown)?
                } else {
                    self.http
                        .post(format!("{}/me/sendMail", self.microsoft_graph_base))
                        .bearer_auth(token.as_str())
                        .json(&graph_send_payload(draft, attempt_id))
                        .send()
                        .await
                        .map_err(|_| ProviderEffectError::Unknown)?
                };
                classify_graph_response(response).await
            }
            Provider::AppleIcloud => Err(ProviderEffectError::Unsupported),
        }
    }

    /// Search the owner's sent mail for the exact opaque attempt marker. A
    /// missing marker is inconclusive: eventual consistency and bounded scans
    /// must never turn into permission to retry an ambiguous send.
    pub async fn reconcile_sent_mail(
        &self,
        account: &ConnectedAccount,
        vault: &AccountVault,
        agent_id: &str,
        audience: &str,
        attempt_id: &str,
    ) -> Result<Option<String>, ProviderEffectError> {
        if vault.agent_id() != account.owner_agent_id
            || !account.admits(agent_id, audience, Capability::MailRead)
            || !valid_attempt_id(attempt_id)
        {
            return Err(ProviderEffectError::NotAdmitted);
        }
        let token = vault
            .access_token(&account.id)
            .map_err(|_| ProviderEffectError::ReauthorizationRequired)?;
        match account.provider {
            Provider::Google => {
                let message_id = format!("<{attempt_id}@vak.invalid>");
                let response = self
                    .http
                    .get(format!("{}/users/me/messages", self.google_gmail_base))
                    .bearer_auth(token.as_str())
                    .query(&[
                        ("q", format!("in:sent rfc822msgid:{message_id}")),
                        ("maxResults", "2".into()),
                    ])
                    .send()
                    .await
                    .map_err(|_| ProviderEffectError::Unknown)?;
                let value = checked_json(response).await?;
                let Some(messages) = value.get("messages").and_then(Value::as_array) else {
                    return Ok(None);
                };
                if messages.len() != 1 || value.get("nextPageToken").is_some() {
                    return Ok(None);
                }
                let Some(id) = messages[0].get("id").and_then(Value::as_str) else {
                    return Ok(None);
                };
                let message_url =
                    path_url(&self.google_gmail_base, &["users", "me", "messages", id])?;
                let response = self
                    .http
                    .get(message_url)
                    .bearer_auth(token.as_str())
                    .query(&[("format", "metadata"), ("metadataHeaders", "Message-ID")])
                    .send()
                    .await
                    .map_err(|_| ProviderEffectError::Unknown)?;
                let message = checked_json(response).await?;
                let matches = message
                    .pointer("/payload/headers")
                    .and_then(Value::as_array)
                    .is_some_and(|headers| {
                        headers.iter().any(|header| {
                            header
                                .get("name")
                                .and_then(Value::as_str)
                                .is_some_and(|name| name.eq_ignore_ascii_case("Message-ID"))
                                && header.get("value").and_then(Value::as_str)
                                    == Some(message_id.as_str())
                        })
                    });
                Ok(matches.then(|| id.to_owned()))
            }
            Provider::Microsoft => {
                let Some(attempt_time) = attempt_timestamp(attempt_id) else {
                    return Err(ProviderEffectError::Rejected);
                };
                let filter = format!("sentDateTime ge {}", attempt_time.to_rfc3339());
                let url = format!(
                    "{}/me/mailFolders/sentitems/messages",
                    self.microsoft_graph_base
                );
                let response = self
                    .http
                    .get(url)
                    .bearer_auth(token.as_str())
                    .query(&[
                        ("$filter", filter),
                        ("$select", "id,internetMessageHeaders".into()),
                        ("$top", "10".into()),
                        ("$orderby", "sentDateTime desc".into()),
                    ])
                    .send()
                    .await
                    .map_err(|_| ProviderEffectError::Unknown)?;
                let value = checked_json(response).await?;
                let Some(messages) = value.get("value").and_then(Value::as_array) else {
                    return Ok(None);
                };
                let matching: Vec<&Value> =
                    messages
                        .iter()
                        .filter(|message| {
                            message
                                .get("internetMessageHeaders")
                                .and_then(Value::as_array)
                                .is_some_and(|headers| {
                                    headers.iter().any(|header| {
                                        header.get("name").and_then(Value::as_str).is_some_and(
                                            |name| name.eq_ignore_ascii_case(MAIL_ACTION_HEADER),
                                        ) && header.get("value").and_then(Value::as_str)
                                            == Some(attempt_id)
                                    })
                                })
                        })
                        .collect();
                if matching.len() != 1 || value.get("@odata.nextLink").is_some() {
                    return Ok(None);
                }
                Ok(matching[0]
                    .get("id")
                    .and_then(Value::as_str)
                    .map(str::to_owned))
            }
            Provider::AppleIcloud => Err(ProviderEffectError::Unsupported),
        }
    }

    /// Create one timed event without attendees, recurrence, or reminders.
    /// Times are the exact instants in the reviewed draft and are sent as UTC.
    /// This deliberately cannot invite people or mutate an existing event.
    pub async fn create_event(
        &self,
        account: &ConnectedAccount,
        vault: &AccountVault,
        agent_id: &str,
        audience: &str,
        attempt_id: &str,
        draft: &CalendarDraft,
    ) -> Result<ProviderAcceptance, ProviderEffectError> {
        if vault.agent_id() != account.owner_agent_id
            || !account.admits(agent_id, audience, Capability::CalendarWrite)
        {
            return Err(ProviderEffectError::NotAdmitted);
        }
        if account.provider == Provider::AppleIcloud {
            return Err(ProviderEffectError::Unsupported);
        }
        validate_event_create(draft)?;
        if !valid_attempt_id(attempt_id) {
            return Err(ProviderEffectError::Rejected);
        }
        let token = vault
            .access_token(&account.id)
            .map_err(|_| ProviderEffectError::ReauthorizationRequired)?;
        let response = match account.provider {
            Provider::Google => self
                .http
                .post(format!(
                    "{}/calendars/primary/events?sendUpdates=none",
                    self.google_calendar_base
                ))
                .bearer_auth(token.as_str())
                .json(&google_create_event_payload(draft, attempt_id))
                .send()
                .await
                .map_err(|_| ProviderEffectError::Unknown)?,
            Provider::Microsoft => self
                .http
                .post(format!("{}/me/events", self.microsoft_graph_base))
                .bearer_auth(token.as_str())
                .json(&graph_create_event_payload(draft, attempt_id))
                .send()
                .await
                .map_err(|_| ProviderEffectError::Unknown)?,
            Provider::AppleIcloud => return Err(ProviderEffectError::Unsupported),
        };
        classify_created_event_response(response).await
    }

    /// Look up an event created by a prior ambiguous dispatch. A provider-side
    /// private extension carries the durable attempt id; an absent result is
    /// deliberately inconclusive because provider indexing can lag.
    pub async fn reconcile_created_event(
        &self,
        account: &ConnectedAccount,
        vault: &AccountVault,
        agent_id: &str,
        audience: &str,
        attempt_id: &str,
    ) -> Result<Option<String>, ProviderEffectError> {
        if vault.agent_id() != account.owner_agent_id
            || !account.admits(agent_id, audience, Capability::CalendarWrite)
        {
            return Err(ProviderEffectError::NotAdmitted);
        }
        if !valid_attempt_id(attempt_id) {
            return Err(ProviderEffectError::Rejected);
        }
        let token = vault
            .access_token(&account.id)
            .map_err(|_| ProviderEffectError::ReauthorizationRequired)?;
        let response = match account.provider {
            Provider::Google => self
                .http
                .get(format!(
                    "{}/calendars/primary/events",
                    self.google_calendar_base
                ))
                .bearer_auth(token.as_str())
                .query(&[
                    (
                        "privateExtendedProperty",
                        format!("{GOOGLE_ACTION_PROPERTY}={attempt_id}"),
                    ),
                    ("maxResults", "2".into()),
                ])
                .send()
                .await
                .map_err(|_| ProviderEffectError::Unknown)?,
            Provider::Microsoft => {
                let filter = format!(
                    "singleValueExtendedProperties/Any(ep: ep/id eq '{GRAPH_ACTION_PROPERTY_ID}' and ep/value eq '{attempt_id}')"
                );
                self.http.get(format!("{}/me/events", self.microsoft_graph_base))
                    .bearer_auth(token.as_str())
                    .query(&[("$filter", filter), ("$top", "2".into()), ("$select", "id,singleValueExtendedProperties".into()), ("$expand", format!("singleValueExtendedProperties($filter=id eq '{GRAPH_ACTION_PROPERTY_ID}')"))])
                    .send().await.map_err(|_| ProviderEffectError::Unknown)?
            }
            Provider::AppleIcloud => return Err(ProviderEffectError::Unsupported),
        };
        match response.status() {
            StatusCode::UNAUTHORIZED | StatusCode::FORBIDDEN => {
                return Err(ProviderEffectError::ReauthorizationRequired);
            }
            status if status.is_redirection() || status.is_server_error() => {
                return Err(ProviderEffectError::Unknown);
            }
            status if !status.is_success() => return Err(ProviderEffectError::Rejected),
            _ => {}
        }
        let body = response_json(response).await?;
        let items = body
            .get("items")
            .or_else(|| body.get("value"))
            .and_then(Value::as_array)
            .ok_or(ProviderEffectError::Unknown)?;
        if items.len() != 1
            || body.get("nextPageToken").is_some()
            || body.get("@odata.nextLink").is_some()
        {
            return Ok(None);
        }
        let item = &items[0];
        let id = item
            .get("id")
            .and_then(Value::as_str)
            .filter(|id| !id.is_empty() && id.len() <= 512 && !id.chars().any(char::is_control))
            .ok_or(ProviderEffectError::Unknown)?;
        let marker_matches = match account.provider {
            Provider::Google => {
                item.get("extendedProperties")
                    .and_then(|v| v.get("private"))
                    .and_then(|v| v.get(GOOGLE_ACTION_PROPERTY))
                    .and_then(Value::as_str)
                    == Some(attempt_id)
            }
            Provider::Microsoft => item
                .get("singleValueExtendedProperties")
                .and_then(Value::as_array)
                .is_some_and(|props| {
                    props.iter().any(|prop| {
                        prop.get("id").and_then(Value::as_str) == Some(GRAPH_ACTION_PROPERTY_ID)
                            && prop.get("value").and_then(Value::as_str) == Some(attempt_id)
                    })
                }),
            Provider::AppleIcloud => false,
        };
        if marker_matches {
            Ok(Some(id.to_owned()))
        } else {
            Ok(None)
        }
    }

    /// Update one unchanged, standalone Google event with no attendees.
    /// The reviewed ETag is rechecked before dispatch and sent as If-Match to
    /// close the race between the source check and the mutation.
    pub async fn update_event(
        &self,
        account: &ConnectedAccount,
        vault: &AccountVault,
        agent_id: &str,
        audience: &str,
        attempt_id: &str,
        event_id: &str,
        source_version: &str,
        draft: &CalendarDraft,
    ) -> Result<ProviderAcceptance, ProviderEffectError> {
        if vault.agent_id() != account.owner_agent_id
            || !account.admits(agent_id, audience, Capability::CalendarWrite)
        {
            return Err(ProviderEffectError::NotAdmitted);
        }
        if account.provider != Provider::Google {
            // Graph's event update contract does not document a conditional
            // If-Match for events, so never offer a racy overwrite there.
            return Err(ProviderEffectError::Unsupported);
        }
        validate_event_update(event_id, source_version, draft)?;
        if !valid_attempt_id(attempt_id) {
            return Err(ProviderEffectError::Rejected);
        }
        let token = vault
            .access_token(&account.id)
            .map_err(|_| ProviderEffectError::ReauthorizationRequired)?;
        let url = format!(
            "{}/calendars/primary/events/{event_id}",
            self.google_calendar_base
        );
        let source_response = self
            .http
            .get(&url)
            .bearer_auth(token.as_str())
            .send()
            .await
            .map_err(|_| ProviderEffectError::Unknown)?;
        match source_response.status() {
            StatusCode::UNAUTHORIZED | StatusCode::FORBIDDEN => {
                return Err(ProviderEffectError::ReauthorizationRequired);
            }
            StatusCode::NOT_FOUND | StatusCode::PRECONDITION_FAILED => {
                return Err(ProviderEffectError::Conflict);
            }
            status if status.is_redirection() || status.is_server_error() => {
                return Err(ProviderEffectError::Unknown);
            }
            status if !status.is_success() => return Err(ProviderEffectError::Rejected),
            _ => {}
        }
        let source = response_json(source_response).await?;
        validate_google_update_source(&source, event_id, source_version)?;
        let if_match = reqwest::header::HeaderValue::from_str(source_version)
            .map_err(|_| ProviderEffectError::Rejected)?;
        let response = self
            .http
            .patch(format!("{url}?sendUpdates=none"))
            .bearer_auth(token.as_str())
            .header(reqwest::header::IF_MATCH, if_match)
            .json(&google_update_event_payload(draft, attempt_id))
            .send()
            .await
            .map_err(|_| ProviderEffectError::Unknown)?;
        classify_updated_event_response(response).await
    }

    /// Confirm an ambiguous conditional update only when the event now carries
    /// the exact private marker from that dispatch attempt.
    pub async fn reconcile_updated_event(
        &self,
        account: &ConnectedAccount,
        vault: &AccountVault,
        agent_id: &str,
        audience: &str,
        attempt_id: &str,
        event_id: &str,
    ) -> Result<Option<String>, ProviderEffectError> {
        if vault.agent_id() != account.owner_agent_id
            || !account.admits(agent_id, audience, Capability::CalendarWrite)
        {
            return Err(ProviderEffectError::NotAdmitted);
        }
        if account.provider != Provider::Google {
            return Err(ProviderEffectError::Unsupported);
        }
        if !valid_attempt_id(attempt_id)
            || event_id.is_empty()
            || event_id.len() > 512
            || event_id.chars().any(char::is_control)
        {
            return Err(ProviderEffectError::Rejected);
        }
        let token = vault
            .access_token(&account.id)
            .map_err(|_| ProviderEffectError::ReauthorizationRequired)?;
        let url = path_url(
            &self.google_calendar_base,
            &["calendars", "primary", "events", event_id],
        )?;
        let response = self
            .http
            .get(url)
            .bearer_auth(token.as_str())
            .send()
            .await
            .map_err(|_| ProviderEffectError::Unknown)?;
        match response.status() {
            StatusCode::UNAUTHORIZED | StatusCode::FORBIDDEN => {
                return Err(ProviderEffectError::ReauthorizationRequired);
            }
            StatusCode::NOT_FOUND => return Ok(None),
            status if status.is_redirection() || status.is_server_error() => {
                return Err(ProviderEffectError::Unknown);
            }
            status if !status.is_success() => return Err(ProviderEffectError::Rejected),
            _ => {}
        }
        let body = response_json(response).await?;
        let returned_id = body
            .get("id")
            .and_then(Value::as_str)
            .filter(|id| *id == event_id)
            .ok_or(ProviderEffectError::Unknown)?;
        let marker = body
            .get("extendedProperties")
            .and_then(|properties| properties.get("private"))
            .and_then(|private| private.get(GOOGLE_ACTION_PROPERTY))
            .and_then(Value::as_str);
        if marker == Some(attempt_id) {
            Ok(Some(returned_id.to_owned()))
        } else {
            Ok(None)
        }
    }

    /// Cancel only an unchanged public, standalone, timed Google event with
    /// no guests, when the connected user is its organizer. Store the attempt
    /// marker in the same conditional PATCH as `status=cancelled`, so an
    /// ambiguous result can be reconciled by reading the cancellation tombstone.
    pub async fn cancel_event(
        &self,
        account: &ConnectedAccount,
        vault: &AccountVault,
        agent_id: &str,
        audience: &str,
        attempt_id: &str,
        event_id: &str,
        source_version: &str,
        occurrence_id: Option<&str>,
        whole_series: bool,
    ) -> Result<ProviderAcceptance, ProviderEffectError> {
        if vault.agent_id() != account.owner_agent_id
            || !account.admits(agent_id, audience, Capability::CalendarWrite)
        {
            return Err(ProviderEffectError::NotAdmitted);
        }
        if account.provider != Provider::Google {
            return Err(ProviderEffectError::Unsupported);
        }
        validate_event_cancel(event_id, source_version, occurrence_id, whole_series)?;
        if !valid_attempt_id(attempt_id) {
            return Err(ProviderEffectError::Rejected);
        }
        let token = vault
            .access_token(&account.id)
            .map_err(|_| ProviderEffectError::ReauthorizationRequired)?;
        let url = format!(
            "{}/calendars/primary/events/{event_id}",
            self.google_calendar_base
        );
        let source_response = self
            .http
            .get(&url)
            .bearer_auth(token.as_str())
            .send()
            .await
            .map_err(|_| ProviderEffectError::Unknown)?;
        match source_response.status() {
            StatusCode::UNAUTHORIZED | StatusCode::FORBIDDEN => {
                return Err(ProviderEffectError::ReauthorizationRequired);
            }
            StatusCode::NOT_FOUND | StatusCode::PRECONDITION_FAILED => {
                return Err(ProviderEffectError::Conflict);
            }
            status if status.is_redirection() || status.is_server_error() => {
                return Err(ProviderEffectError::Unknown);
            }
            status if !status.is_success() => return Err(ProviderEffectError::Rejected),
            _ => {}
        }
        let source = response_json(source_response).await?;
        validate_google_cancel_source(&source, event_id, source_version)?;
        let if_match = reqwest::header::HeaderValue::from_str(source_version)
            .map_err(|_| ProviderEffectError::Rejected)?;
        let response = self
            .http
            .patch(format!("{url}?sendUpdates=none"))
            .bearer_auth(token.as_str())
            .header(reqwest::header::IF_MATCH, if_match)
            .json(&json!({
                "status": "cancelled",
                "extendedProperties": {"private": {GOOGLE_ACTION_PROPERTY: attempt_id}}
            }))
            .send()
            .await
            .map_err(|_| ProviderEffectError::Unknown)?;
        classify_updated_event_response(response).await
    }

    /// Confirm an ambiguous cancellation only when the same event is returned
    /// as cancelled and carries this exact attempt's private marker.
    pub async fn reconcile_cancelled_event(
        &self,
        account: &ConnectedAccount,
        vault: &AccountVault,
        agent_id: &str,
        audience: &str,
        attempt_id: &str,
        event_id: &str,
    ) -> Result<Option<String>, ProviderEffectError> {
        if vault.agent_id() != account.owner_agent_id
            || !account.admits(agent_id, audience, Capability::CalendarWrite)
        {
            return Err(ProviderEffectError::NotAdmitted);
        }
        if account.provider != Provider::Google {
            return Err(ProviderEffectError::Unsupported);
        }
        if !valid_attempt_id(attempt_id)
            || event_id.is_empty()
            || event_id.len() > 512
            || event_id.chars().any(char::is_control)
        {
            return Err(ProviderEffectError::Rejected);
        }
        let token = vault
            .access_token(&account.id)
            .map_err(|_| ProviderEffectError::ReauthorizationRequired)?;
        let url = path_url(
            &self.google_calendar_base,
            &["calendars", "primary", "events", event_id],
        )?;
        let response = self
            .http
            .get(url)
            .bearer_auth(token.as_str())
            .send()
            .await
            .map_err(|_| ProviderEffectError::Unknown)?;
        match response.status() {
            StatusCode::UNAUTHORIZED | StatusCode::FORBIDDEN => {
                return Err(ProviderEffectError::ReauthorizationRequired);
            }
            StatusCode::NOT_FOUND => return Ok(None),
            status if status.is_redirection() || status.is_server_error() => {
                return Err(ProviderEffectError::Unknown);
            }
            status if !status.is_success() => return Err(ProviderEffectError::Rejected),
            _ => {}
        }
        let body = response_json(response).await?;
        let is_cancelled = body.get("id").and_then(Value::as_str) == Some(event_id)
            && body.get("status").and_then(Value::as_str) == Some("cancelled");
        let marker = body
            .pointer("/extendedProperties/private/vak_action_id")
            .and_then(Value::as_str);
        if is_cancelled && marker == Some(attempt_id) {
            Ok(Some(event_id.to_owned()))
        } else {
            Ok(None)
        }
    }
}

/// Only create the semantics shown by the current event Review: one timed
/// event, no attendees/invitations, no recurrence, and no existing instance.
pub fn validate_event_create(draft: &CalendarDraft) -> Result<(), ProviderEffectError> {
    draft
        .validate()
        .map_err(|_| ProviderEffectError::Rejected)?;
    if draft.all_day
        || !draft.attendee_addresses.is_empty()
        || draft.recurrence.is_some()
        || draft.occurrence_id.is_some()
    {
        return Err(ProviderEffectError::Unsupported);
    }
    Ok(())
}

pub fn validate_event_update(
    event_id: &str,
    source_version: &str,
    draft: &CalendarDraft,
) -> Result<(), ProviderEffectError> {
    validate_event_create(draft)?;
    if event_id.len() < 5
        || event_id.len() > 1024
        || !event_id
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'v').contains(&byte))
        || source_version.trim().is_empty()
        || source_version.len() > 512
        || source_version.chars().any(char::is_control)
        || reqwest::header::HeaderValue::from_str(source_version).is_err()
    {
        return Err(ProviderEffectError::Rejected);
    }
    Ok(())
}

pub fn validate_event_cancel(
    event_id: &str,
    source_version: &str,
    occurrence_id: Option<&str>,
    whole_series: bool,
) -> Result<(), ProviderEffectError> {
    if event_id.len() < 5
        || event_id.len() > 1024
        || !event_id
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'v').contains(&byte))
        || source_version.trim().is_empty()
        || source_version.len() > 512
        || source_version.chars().any(char::is_control)
        || reqwest::header::HeaderValue::from_str(source_version).is_err()
        || occurrence_id.is_some()
        || whole_series
    {
        return Err(ProviderEffectError::Rejected);
    }
    Ok(())
}

fn validate_google_cancel_source(
    source: &Value,
    event_id: &str,
    source_version: &str,
) -> Result<(), ProviderEffectError> {
    validate_google_update_source(source, event_id, source_version)?;
    if source.pointer("/organizer/self").and_then(Value::as_bool) != Some(true) {
        return Err(ProviderEffectError::Unsupported);
    }
    Ok(())
}

fn validate_google_update_source(
    source: &Value,
    event_id: &str,
    source_version: &str,
) -> Result<(), ProviderEffectError> {
    let id = source
        .get("id")
        .and_then(Value::as_str)
        .ok_or(ProviderEffectError::Conflict)?;
    let version = source
        .get("etag")
        .and_then(Value::as_str)
        .ok_or(ProviderEffectError::Conflict)?;
    let default_event = source
        .get("eventType")
        .and_then(Value::as_str)
        .unwrap_or("default")
        == "default";
    let has_attendees = source
        .get("attendees")
        .and_then(Value::as_array)
        .is_some_and(|attendees| !attendees.is_empty());
    let recurring = source.get("recurringEventId").is_some()
        || source
            .get("recurrence")
            .and_then(Value::as_array)
            .is_some_and(|rules| !rules.is_empty());
    let all_day = source.pointer("/start/date").is_some() || source.pointer("/end/date").is_some();
    if id != event_id || version != source_version {
        return Err(ProviderEffectError::Conflict);
    }
    if source.get("visibility").and_then(Value::as_str) == Some("private")
        || has_attendees
        || recurring
        || all_day
        || !default_event
    {
        return Err(ProviderEffectError::Unsupported);
    }
    Ok(())
}

fn google_create_event_payload(draft: &CalendarDraft, attempt_id: &str) -> Value {
    json!({
        "summary": draft.title,
        "description": draft.description,
        "location": draft.location,
        "start": {"dateTime": draft.starts_at.to_rfc3339(), "timeZone": "UTC"},
        "end": {"dateTime": draft.ends_at.to_rfc3339(), "timeZone": "UTC"},
        "attendees": [],
        "reminders": {"useDefault": false, "overrides": []},
        "extendedProperties": {"private": {GOOGLE_ACTION_PROPERTY: attempt_id}}
    })
}

fn google_update_event_payload(draft: &CalendarDraft, attempt_id: &str) -> Value {
    json!({
        "summary": draft.title,
        "description": draft.description,
        "location": draft.location,
        "start": {"dateTime": draft.starts_at.to_rfc3339(), "timeZone": "UTC"},
        "end": {"dateTime": draft.ends_at.to_rfc3339(), "timeZone": "UTC"},
        "extendedProperties": {"private": {GOOGLE_ACTION_PROPERTY: attempt_id}}
    })
}

fn graph_create_event_payload(draft: &CalendarDraft, attempt_id: &str) -> Value {
    json!({
        "subject": draft.title,
        "body": {"contentType": "Text", "content": draft.description},
        "location": {"displayName": draft.location.as_deref().unwrap_or("")},
        "start": {"dateTime": draft.starts_at.format("%Y-%m-%dT%H:%M:%S").to_string(), "timeZone": "UTC"},
        "end": {"dateTime": draft.ends_at.format("%Y-%m-%dT%H:%M:%S").to_string(), "timeZone": "UTC"},
        "attendees": [],
        "isReminderOn": false,
        "singleValueExtendedProperties": [{"id": GRAPH_ACTION_PROPERTY_ID, "value": attempt_id}]
    })
}

fn valid_attempt_id(value: &str) -> bool {
    uuid::Uuid::parse_str(value).is_ok_and(|id| id.get_version_num() == 7)
}

fn attempt_timestamp(value: &str) -> Option<chrono::DateTime<chrono::Utc>> {
    let uuid = uuid::Uuid::parse_str(value).ok()?;
    if uuid.get_version_num() != 7 {
        return None;
    }
    let bytes = uuid.as_bytes();
    let millis = u64::from_be_bytes([
        0, 0, bytes[0], bytes[1], bytes[2], bytes[3], bytes[4], bytes[5],
    ]);
    chrono::DateTime::from_timestamp_millis(i64::try_from(millis).ok()?)
}

/// Validate the currently supported plain-text send profile before a durable
/// single-use dispatch claim is created.
pub fn validate_mail_draft(draft: &MailDraft) -> Result<(), ProviderEffectError> {
    draft
        .validate()
        .map_err(|_| ProviderEffectError::Rejected)?;
    if !draft.attachment_refs.is_empty()
        || draft.from_alias.is_some()
        || draft
            .to
            .iter()
            .chain(&draft.cc)
            .chain(&draft.bcc)
            .any(|address| {
                address
                    .display_name
                    .as_ref()
                    .is_some_and(|name| name.chars().any(char::is_control))
            })
    {
        return Err(ProviderEffectError::Unsupported);
    }
    Ok(())
}

fn graph_recipients(addresses: &[MailAddress]) -> Vec<Value> {
    addresses
        .iter()
        .map(|address| {
            json!({"emailAddress": {
                "address": address.address,
                "name": address.display_name
            }})
        })
        .collect()
}

fn graph_send_payload(draft: &MailDraft, attempt_id: &str) -> Value {
    json!({
        "message": {
            "subject": draft.subject,
            "body": {"contentType": "Text", "content": draft.body_text},
            "toRecipients": graph_recipients(&draft.to),
            "ccRecipients": graph_recipients(&draft.cc),
            "bccRecipients": graph_recipients(&draft.bcc),
            "internetMessageHeaders": [{"name": MAIL_ACTION_HEADER, "value": attempt_id}]
        },
        "saveToSentItems": true
    })
}

fn graph_reply_payload(draft: &MailDraft, attempt_id: &str) -> Value {
    json!({"message": {
        "body": {"contentType": "Text", "content": draft.body_text},
        "toRecipients": graph_recipients(&draft.to),
        "ccRecipients": graph_recipients(&draft.cc),
        "bccRecipients": graph_recipients(&draft.bcc),
        "internetMessageHeaders": [{"name": MAIL_ACTION_HEADER, "value": attempt_id}]
    }})
}

fn path_url(base: &str, segments: &[&str]) -> Result<url::Url, ProviderEffectError> {
    let mut url = url::Url::parse(base).map_err(|_| ProviderEffectError::Rejected)?;
    url.path_segments_mut()
        .map_err(|_| ProviderEffectError::Rejected)?
        .extend(segments.iter().copied());
    Ok(url)
}

async fn checked_json(response: Response) -> Result<Value, ProviderEffectError> {
    match response.status() {
        StatusCode::UNAUTHORIZED | StatusCode::FORBIDDEN => {
            return Err(ProviderEffectError::ReauthorizationRequired);
        }
        StatusCode::NOT_FOUND | StatusCode::PRECONDITION_FAILED => {
            return Err(ProviderEffectError::Conflict);
        }
        status if status.is_redirection() || status.is_server_error() => {
            return Err(ProviderEffectError::Unknown);
        }
        status if !status.is_success() => return Err(ProviderEffectError::Rejected),
        _ => {}
    }
    response_json(response).await
}

fn validate_graph_reply_source(
    source: &Value,
    message_id: &str,
    thread_id: &str,
    subject: &str,
) -> Result<(), ProviderEffectError> {
    if source.get("id").and_then(Value::as_str) != Some(message_id)
        || source.get("conversationId").and_then(Value::as_str) != Some(thread_id)
        || source
            .get("subject")
            .and_then(Value::as_str)
            .is_none_or(|actual| !reply_subjects_match(actual, subject))
    {
        return Err(ProviderEffectError::Conflict);
    }
    Ok(())
}

#[derive(Debug, PartialEq, Eq)]
struct GoogleReplyHeaders {
    message_id: String,
    references: String,
}

fn validate_google_reply_source(
    source: &Value,
    message_id: &str,
    thread_id: &str,
    subject: &str,
) -> Result<GoogleReplyHeaders, ProviderEffectError> {
    if source.get("id").and_then(Value::as_str) != Some(message_id)
        || source.get("threadId").and_then(Value::as_str) != Some(thread_id)
    {
        return Err(ProviderEffectError::Conflict);
    }
    let headers = source
        .pointer("/payload/headers")
        .and_then(Value::as_array)
        .ok_or(ProviderEffectError::Conflict)?;
    let header = |name: &str| {
        headers
            .iter()
            .find(|h| {
                h.get("name")
                    .and_then(Value::as_str)
                    .is_some_and(|n| n.eq_ignore_ascii_case(name))
            })
            .and_then(|h| h.get("value"))
            .and_then(Value::as_str)
    };
    let actual_subject =
        decode_subject_header(header("Subject").ok_or(ProviderEffectError::Conflict)?)
            .ok_or(ProviderEffectError::Conflict)?;
    if !reply_subjects_match(&actual_subject, subject) {
        return Err(ProviderEffectError::Conflict);
    }
    let message_id = header("Message-ID")
        .filter(|v| valid_msg_id(v))
        .ok_or(ProviderEffectError::Conflict)?
        .to_owned();
    let prior_refs = header("References").unwrap_or("");
    if prior_refs.chars().any(char::is_control) || prior_refs.len() > 8192 {
        return Err(ProviderEffectError::Conflict);
    }
    let refs = prior_refs.split_ascii_whitespace().collect::<Vec<_>>();
    if refs.iter().any(|v| !valid_msg_id(v)) {
        return Err(ProviderEffectError::Conflict);
    }
    let mut references = refs.join(" ");
    if !references.is_empty() {
        references.push(' ');
    }
    references.push_str(&message_id);
    Ok(GoogleReplyHeaders {
        message_id,
        references,
    })
}

fn valid_msg_id(value: &str) -> bool {
    value.len() >= 5
        && value.len() <= 998
        && value.starts_with('<')
        && value.ends_with('>')
        && value.is_ascii()
        && value
            .bytes()
            .all(|c| c == b'<' || c == b'>' || (b'!'..=b'~').contains(&c))
        && value[1..value.len() - 1].contains('@')
}

fn decode_subject_header(value: &str) -> Option<String> {
    let Some(encoded) = value.strip_prefix("=?UTF-8?") else {
        if value.chars().any(char::is_control) {
            return None;
        }
        return Some(value.to_owned());
    };
    let (encoding, rest) = encoded.split_once('?')?;
    let encoded = rest.strip_suffix("?=")?;
    let bytes = match encoding.to_ascii_uppercase().as_str() {
        "B" => base64::engine::general_purpose::STANDARD
            .decode(encoded)
            .ok()?,
        "Q" => {
            let input = encoded.as_bytes();
            let mut output = Vec::with_capacity(input.len());
            let mut index = 0;
            while index < input.len() {
                match input[index] {
                    b'_' => {
                        output.push(b' ');
                        index += 1;
                    }
                    b'=' if index + 2 < input.len() => {
                        let hex = std::str::from_utf8(&input[index + 1..index + 3]).ok()?;
                        output.push(u8::from_str_radix(hex, 16).ok()?);
                        index += 3;
                    }
                    byte if byte.is_ascii() && !byte.is_ascii_control() => {
                        output.push(byte);
                        index += 1;
                    }
                    _ => return None,
                }
            }
            output
        }
        _ => return None,
    };
    let decoded = String::from_utf8(bytes).ok()?;
    (!decoded.chars().any(char::is_control)).then_some(decoded)
}

fn reply_subjects_match(source: &str, reply: &str) -> bool {
    fn base(value: &str) -> &str {
        let mut value = value.trim();
        loop {
            let lower = value.to_ascii_lowercase();
            if lower.starts_with("re:") {
                value = value[3..].trim_start();
            } else {
                return value;
            }
        }
    }
    base(source).eq_ignore_ascii_case(base(reply))
}

fn google_raw_reply(draft: &MailDraft, headers: &GoogleReplyHeaders, attempt_id: &str) -> String {
    let mut raw = google_raw_message(draft, attempt_id);
    // Insert before MIME headers; values are restricted to validated RFC message-id tokens.
    let mime = "MIME-Version: 1.0\r\n";
    raw = raw.replacen(
        mime,
        &format!(
            "In-Reply-To: {}\r\nReferences: {}\r\n{mime}",
            headers.message_id, headers.references
        ),
        1,
    );
    raw
}

fn google_raw_message(draft: &MailDraft, attempt_id: &str) -> String {
    let mut raw = String::new();
    raw.push_str(&format!("To: {}\r\n", google_addresses(&draft.to)));
    if !draft.cc.is_empty() {
        raw.push_str(&format!("Cc: {}\r\n", google_addresses(&draft.cc)));
    }
    if !draft.bcc.is_empty() {
        raw.push_str(&format!("Bcc: {}\r\n", google_addresses(&draft.bcc)));
    }
    raw.push_str(&format!("Subject: {}\r\n", encoded_header(&draft.subject)));
    raw.push_str(&format!("Message-ID: <{attempt_id}@vak.invalid>\r\n"));
    raw.push_str("MIME-Version: 1.0\r\n");
    raw.push_str("Content-Type: text/plain; charset=UTF-8\r\n");
    raw.push_str("Content-Transfer-Encoding: 8bit\r\n\r\n");
    raw.push_str(&draft.body_text);
    raw
}

fn google_addresses(addresses: &[MailAddress]) -> String {
    addresses
        .iter()
        .map(|address| match &address.display_name {
            Some(name) => format!(
                "\"{}\" <{}>",
                name.replace('\\', "\\\\").replace('"', "\\\""),
                address.address
            ),
            None => address.address.clone(),
        })
        .collect::<Vec<_>>()
        .join(", ")
}

fn encoded_header(value: &str) -> String {
    if value.is_ascii() {
        value.to_owned()
    } else {
        format!(
            "=?UTF-8?B?{}?=",
            base64::engine::general_purpose::STANDARD.encode(value.as_bytes())
        )
    }
}

async fn classify_google_response(
    response: Response,
) -> Result<ProviderAcceptance, ProviderEffectError> {
    match response.status() {
        StatusCode::UNAUTHORIZED | StatusCode::FORBIDDEN => {
            return Err(ProviderEffectError::ReauthorizationRequired);
        }
        status if status.is_redirection() || status.is_server_error() => {
            return Err(ProviderEffectError::Unknown);
        }
        status if !status.is_success() => return Err(ProviderEffectError::Rejected),
        _ => {}
    }
    let value = response_json(response).await?;
    let id = value
        .get("id")
        .and_then(Value::as_str)
        .filter(|id| !id.is_empty() && id.len() <= 512 && !id.chars().any(char::is_control))
        .ok_or(ProviderEffectError::Unknown)?;
    Ok(ProviderAcceptance {
        provider_item_id: Some(id.to_owned()),
    })
}

async fn classify_graph_response(
    response: Response,
) -> Result<ProviderAcceptance, ProviderEffectError> {
    match response.status() {
        StatusCode::UNAUTHORIZED | StatusCode::FORBIDDEN => {
            Err(ProviderEffectError::ReauthorizationRequired)
        }
        StatusCode::ACCEPTED | StatusCode::OK | StatusCode::CREATED => Ok(ProviderAcceptance {
            provider_item_id: None,
        }),
        status if status.is_redirection() || status.is_server_error() => {
            Err(ProviderEffectError::Unknown)
        }
        _ => Err(ProviderEffectError::Rejected),
    }
}

async fn classify_created_event_response(
    response: Response,
) -> Result<ProviderAcceptance, ProviderEffectError> {
    match response.status() {
        StatusCode::UNAUTHORIZED | StatusCode::FORBIDDEN => {
            return Err(ProviderEffectError::ReauthorizationRequired);
        }
        StatusCode::CREATED | StatusCode::OK => {}
        status if status.is_redirection() || status.is_server_error() => {
            return Err(ProviderEffectError::Unknown);
        }
        _ => return Err(ProviderEffectError::Rejected),
    }
    let value = response_json(response).await?;
    let id = value
        .get("id")
        .and_then(Value::as_str)
        .filter(|id| !id.is_empty() && id.len() <= 512 && !id.chars().any(char::is_control))
        .ok_or(ProviderEffectError::Unknown)?;
    Ok(ProviderAcceptance {
        provider_item_id: Some(id.to_owned()),
    })
}

async fn classify_updated_event_response(
    response: Response,
) -> Result<ProviderAcceptance, ProviderEffectError> {
    match response.status() {
        StatusCode::UNAUTHORIZED | StatusCode::FORBIDDEN => {
            return Err(ProviderEffectError::ReauthorizationRequired);
        }
        StatusCode::PRECONDITION_FAILED | StatusCode::NOT_FOUND => {
            return Err(ProviderEffectError::Conflict);
        }
        StatusCode::OK => {}
        status if status.is_redirection() || status.is_server_error() => {
            return Err(ProviderEffectError::Unknown);
        }
        _ => return Err(ProviderEffectError::Rejected),
    }
    let value = response_json(response).await?;
    let id = value
        .get("id")
        .and_then(Value::as_str)
        .filter(|id| !id.is_empty() && id.len() <= 512 && !id.chars().any(char::is_control))
        .ok_or(ProviderEffectError::Unknown)?;
    Ok(ProviderAcceptance {
        provider_item_id: Some(id.to_owned()),
    })
}

async fn response_json(response: Response) -> Result<Value, ProviderEffectError> {
    if response
        .content_length()
        .is_some_and(|length| length > MAX_RESPONSE_BYTES as u64)
    {
        return Err(ProviderEffectError::Unknown);
    }
    let mut response = response;
    let mut bytes = Vec::new();
    while let Some(chunk) = response
        .chunk()
        .await
        .map_err(|_| ProviderEffectError::Unknown)?
    {
        if bytes.len().saturating_add(chunk.len()) > MAX_RESPONSE_BYTES {
            return Err(ProviderEffectError::Unknown);
        }
        bytes.extend_from_slice(&chunk);
    }
    serde_json::from_slice(&bytes).map_err(|_| ProviderEffectError::Unknown)
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::{
        Json, Router,
        extract::Query,
        routing::{get, post},
    };
    use serde_json::json;
    use std::sync::{Arc, Mutex};

    #[test]
    fn provider_action_matrix_disables_unverified_mutations() {
        let create = ProposedAction::CreateEvent {
            draft: timed_event(),
        };
        let update = ProposedAction::UpdateEvent {
            event_id: "event-1".into(),
            source_version: "etag-1".into(),
            draft: timed_event(),
        };
        let cancel = ProposedAction::CancelEvent {
            event_id: "event-1".into(),
            source_version: "etag-1".into(),
            occurrence_id: None,
            whole_series: false,
        };
        let respond = ProposedAction::RespondToEvent {
            event_id: "event-1".into(),
            source_version: "etag-1".into(),
            response: crate::AttendeeResponse::Accept,
        };

        assert!(supports_action(Provider::Google, &create));
        assert!(supports_action(Provider::Microsoft, &create));
        assert!(supports_action(Provider::Google, &update));
        assert!(supports_action(Provider::Google, &cancel));
        assert!(!supports_action(Provider::Microsoft, &update));
        assert!(!supports_action(Provider::Microsoft, &cancel));
        assert!(!supports_action(Provider::AppleIcloud, &create));
        assert!(!supports_action(Provider::Google, &respond));
        assert!(!supports_action(Provider::Microsoft, &respond));
    }

    fn timed_event() -> CalendarDraft {
        CalendarDraft {
            title: "Review meeting".into(),
            description: "Draft description".into(),
            location: Some("Room 2".into()),
            starts_at: "2026-10-01T09:00:00Z".parse().unwrap(),
            ends_at: "2026-10-01T10:00:00Z".parse().unwrap(),
            time_zone: "Asia/Kolkata".into(),
            all_day: false,
            attendee_addresses: vec![],
            recurrence: None,
            occurrence_id: None,
        }
    }

    fn calendar_account(agent_id: &str, account_id: &str, provider: Provider) -> ConnectedAccount {
        let credential_ref = AccountVault::credential_ref(account_id).unwrap();
        ConnectedAccount {
            id: account_id.into(),
            provider,
            status: crate::AccountStatus::Connected,
            owner_agent_id: agent_id.into(),
            allowed_audiences: [format!("agent:{agent_id}")].into_iter().collect(),
            capabilities: [Capability::CalendarWrite].into_iter().collect(),
            provider_scopes: ["calendar.write".into()].into_iter().collect(),
            credential_ref: credential_ref.clone(),
            principal_ref: credential_ref,
            revision: 1,
            connected_at: chrono::Utc::now(),
            access_token_expires_at: None,
            refresh_token_available: false,
            revoked_at: None,
        }
    }

    #[tokio::test]
    async fn created_event_reconciliation_confirms_only_a_matching_private_attempt_marker() {
        vak_config::paths::isolate_home_for_tests();
        let agent_id = format!("mailcal-reconcile-{}", uuid::Uuid::now_v7());
        let vault = AccountVault::for_agent(&agent_id).unwrap();
        let attempt_id = uuid::Uuid::now_v7().to_string();
        let google_id = uuid::Uuid::now_v7().to_string();
        let graph_id = uuid::Uuid::now_v7().to_string();
        for account_id in [&google_id, &graph_id] {
            vault
                .store(
                    account_id,
                    crate::vault::AccountSecretMaterial::new(
                        format!("provider:{account_id}"),
                        Some("owner@example.test".into()),
                        None,
                        Some("mock-access-token".into()),
                        None,
                        None,
                        None,
                    )
                    .unwrap(),
                )
                .unwrap();
        }
        let google_visible = Arc::new(std::sync::atomic::AtomicBool::new(false));
        let google_flag = google_visible.clone();
        let graph_flag = google_visible.clone();
        let google_attempt = attempt_id.clone();
        let graph_attempt = attempt_id.clone();
        let app = Router::new()
            .route("/calendar/v3/calendars/primary/events", get(move |Query(query): Query<std::collections::HashMap<String, String>>| {
                let visible = google_flag.load(std::sync::atomic::Ordering::SeqCst);
                async move {
                    assert_eq!(query.get("privateExtendedProperty"), Some(&format!("{GOOGLE_ACTION_PROPERTY}={google_attempt}")));
                    assert_eq!(query.get("maxResults").map(String::as_str), Some("2"));
                    Json(json!({"items": if visible { vec![json!({"id":"google-event","extendedProperties":{"private":{(GOOGLE_ACTION_PROPERTY):google_attempt}}})] } else { vec![] }}))
                }
            }))
            .route("/graph/v1.0/me/events", get(move |Query(query): Query<std::collections::HashMap<String, String>>| {
                let visible = graph_flag.load(std::sync::atomic::Ordering::SeqCst);
                async move {
                    assert!(query.get("$filter").is_some_and(|value| value.contains(&graph_attempt)));
                    assert_eq!(query.get("$top").map(String::as_str), Some("2"));
                    Json(json!({"value": if visible { vec![json!({"id":"graph-event","singleValueExtendedProperties":[{"id":GRAPH_ACTION_PROPERTY_ID,"value":graph_attempt}]})] } else { vec![] }}))
                }
            }));
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let server = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
        let mut client = ProviderEffectClient::new().unwrap();
        client.google_calendar_base = format!("http://{address}/calendar/v3");
        client.microsoft_graph_base = format!("http://{address}/graph/v1.0");
        let google = calendar_account(&agent_id, &google_id, Provider::Google);
        let graph = calendar_account(&agent_id, &graph_id, Provider::Microsoft);
        assert_eq!(
            client
                .reconcile_created_event(
                    &google,
                    &vault,
                    &agent_id,
                    &format!("agent:{agent_id}"),
                    &attempt_id
                )
                .await
                .unwrap(),
            None
        );
        assert_eq!(
            client
                .reconcile_created_event(
                    &graph,
                    &vault,
                    &agent_id,
                    &format!("agent:{agent_id}"),
                    &attempt_id
                )
                .await
                .unwrap(),
            None
        );
        google_visible.store(true, std::sync::atomic::Ordering::SeqCst);
        assert_eq!(
            client
                .reconcile_created_event(
                    &google,
                    &vault,
                    &agent_id,
                    &format!("agent:{agent_id}"),
                    &attempt_id
                )
                .await
                .unwrap()
                .as_deref(),
            Some("google-event")
        );
        assert_eq!(
            client
                .reconcile_created_event(
                    &graph,
                    &vault,
                    &agent_id,
                    &format!("agent:{agent_id}"),
                    &attempt_id
                )
                .await
                .unwrap()
                .as_deref(),
            Some("graph-event")
        );
        vault.remove(&google_id).unwrap();
        vault.remove(&graph_id).unwrap();
        server.abort();
    }

    #[tokio::test]
    async fn ambiguous_google_event_update_reconciles_only_its_attempt_marker() {
        vak_config::paths::isolate_home_for_tests();
        let agent_id = format!("mailcal-update-reconcile-{}", uuid::Uuid::now_v7());
        let account_id = uuid::Uuid::now_v7().to_string();
        let attempt_id = uuid::Uuid::now_v7().to_string();
        let vault = AccountVault::for_agent(&agent_id).unwrap();
        vault
            .store(
                &account_id,
                crate::vault::AccountSecretMaterial::new(
                    format!("provider:{account_id}"),
                    Some("owner@example.test".into()),
                    None,
                    Some("mock-access-token".into()),
                    None,
                    None,
                    None,
                )
                .unwrap(),
            )
            .unwrap();
        let changed = Arc::new(std::sync::atomic::AtomicBool::new(false));
        let get_changed = Arc::clone(&changed);
        let patch_changed = Arc::clone(&changed);
        let stored_payload = Arc::new(Mutex::new(None::<Value>));
        let patch_payload = Arc::clone(&stored_payload);
        let attempt_for_read = attempt_id.clone();
        let app = Router::new().route(
            "/calendar/v3/calendars/primary/events/abcde",
            get(move || {
                let changed = get_changed.load(std::sync::atomic::Ordering::SeqCst);
                let marker = attempt_for_read.clone();
                async move {
                    let mut event = json!({
                            "id":"abcde",
                        "etag":"\"v1\"",
                        "eventType":"default",
                        "attendees":[],
                        "organizer":{"self":true}
                    });
                    if changed {
                        event["extendedProperties"] =
                            json!({"private":{(GOOGLE_ACTION_PROPERTY):marker}});
                    }
                    Json(event)
                }
            })
            .patch(move |Json(payload): Json<Value>| {
                let changed = Arc::clone(&patch_changed);
                let stored = Arc::clone(&patch_payload);
                async move {
                    *stored.lock().unwrap() = Some(payload);
                    changed.store(true, std::sync::atomic::Ordering::SeqCst);
                    StatusCode::INTERNAL_SERVER_ERROR
                }
            }),
        );
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let server = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
        let mut client = ProviderEffectClient::new().unwrap();
        client.google_calendar_base = format!("http://{address}/calendar/v3");
        let account = calendar_account(&agent_id, &account_id, Provider::Google);

        assert_eq!(
            client
                .reconcile_updated_event(
                    &account,
                    &vault,
                    &agent_id,
                    &format!("agent:{agent_id}"),
                    &attempt_id,
                    "abcde",
                )
                .await
                .unwrap(),
            None
        );
        assert_eq!(
            client
                .update_event(
                    &account,
                    &vault,
                    &agent_id,
                    &format!("agent:{agent_id}"),
                    &attempt_id,
                    "abcde",
                    "\"v1\"",
                    &timed_event(),
                )
                .await,
            Err(ProviderEffectError::Unknown)
        );
        assert_eq!(
            stored_payload.lock().unwrap().as_ref().unwrap()["extendedProperties"]["private"]
                [GOOGLE_ACTION_PROPERTY],
            attempt_id
        );
        assert_eq!(
            client
                .reconcile_updated_event(
                    &account,
                    &vault,
                    &agent_id,
                    &format!("agent:{agent_id}"),
                    &attempt_id,
                    "abcde",
                )
                .await
                .unwrap()
                .as_deref(),
            Some("abcde")
        );

        vault.remove(&account_id).unwrap();
        server.abort();
    }

    #[tokio::test]
    async fn sent_mail_reconciliation_confirms_only_unique_exact_markers() {
        vak_config::paths::isolate_home_for_tests();
        let agent_id = format!("mailcal-mail-reconcile-{}", uuid::Uuid::now_v7());
        let google_id = uuid::Uuid::now_v7().to_string();
        let graph_id = uuid::Uuid::now_v7().to_string();
        let attempt_id = uuid::Uuid::now_v7().to_string();
        let vault = AccountVault::for_agent(&agent_id).unwrap();
        for account_id in [&google_id, &graph_id] {
            vault
                .store(
                    account_id,
                    crate::vault::AccountSecretMaterial::new(
                        format!("provider:{account_id}"),
                        Some("owner@example.test".into()),
                        None,
                        Some("mock-access-token".into()),
                        None,
                        None,
                        None,
                    )
                    .unwrap(),
                )
                .unwrap();
        }
        let visible = Arc::new(std::sync::atomic::AtomicBool::new(false));
        let google_flag = Arc::clone(&visible);
        let graph_flag = Arc::clone(&visible);
        let attempt_for_google = attempt_id.clone();
        let attempt_for_google_get = attempt_id.clone();
        let attempt_for_graph = attempt_id.clone();
        let app = Router::new()
            .route("/gmail/v1/users/me/messages", get(move |Query(query): Query<std::collections::HashMap<String, String>>| {
                let visible = google_flag.load(std::sync::atomic::Ordering::SeqCst);
                let attempt = attempt_for_google.clone();
                async move {
                    assert_eq!(query.get("q"), Some(&format!("in:sent rfc822msgid:<{attempt}@vak.invalid>")));
                    assert_eq!(query.get("maxResults").map(String::as_str), Some("2"));
                    Json(if visible { json!({"messages":[{"id":"gmail-item"}]}) } else { json!({"messages":[]}) })
                }
            }))
            .route("/gmail/v1/users/me/messages/gmail-item", get(move || {
                let attempt = attempt_for_google_get.clone();
                async move { Json(json!({"id":"gmail-item", "payload":{"headers":[{"name":"Message-ID", "value":format!("<{attempt}@vak.invalid>")}]}})) }
            }))
            .route("/graph/v1.0/me/mailFolders/sentitems/messages", get(move |Query(query): Query<std::collections::HashMap<String, String>>| {
                let visible = graph_flag.load(std::sync::atomic::Ordering::SeqCst);
                let attempt = attempt_for_graph.clone();
                async move {
                    assert!(query.get("$filter").is_some_and(|filter| filter.starts_with("sentDateTime ge ")));
                    assert_eq!(query.get("$top").map(String::as_str), Some("10"));
                    Json(if visible { json!({"value":[{"id":"graph-item", "internetMessageHeaders":[{"name":MAIL_ACTION_HEADER,"value":attempt}]}]}) } else { json!({"value":[]}) })
                }
            }));
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let server = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
        let mut client = ProviderEffectClient::new().unwrap();
        client.google_gmail_base = format!("http://{address}/gmail/v1");
        client.microsoft_graph_base = format!("http://{address}/graph/v1.0");
        let mut google = calendar_account(&agent_id, &google_id, Provider::Google);
        let mut graph = calendar_account(&agent_id, &graph_id, Provider::Microsoft);
        for account in [&mut google, &mut graph] {
            account.capabilities.insert(Capability::MailRead);
            account.provider_scopes.insert("mail.read".into());
        }
        for (account, expected) in [(&google, None), (&graph, None)] {
            assert_eq!(
                client
                    .reconcile_sent_mail(
                        account,
                        &vault,
                        &agent_id,
                        &format!("agent:{agent_id}"),
                        &attempt_id
                    )
                    .await
                    .unwrap(),
                expected
            );
        }
        visible.store(true, std::sync::atomic::Ordering::SeqCst);
        assert_eq!(
            client
                .reconcile_sent_mail(
                    &google,
                    &vault,
                    &agent_id,
                    &format!("agent:{agent_id}"),
                    &attempt_id
                )
                .await
                .unwrap()
                .as_deref(),
            Some("gmail-item")
        );
        assert_eq!(
            client
                .reconcile_sent_mail(
                    &graph,
                    &vault,
                    &agent_id,
                    &format!("agent:{agent_id}"),
                    &attempt_id
                )
                .await
                .unwrap()
                .as_deref(),
            Some("graph-item")
        );
        vault.remove(&google_id).unwrap();
        vault.remove(&graph_id).unwrap();
        server.abort();
    }

    #[tokio::test]
    async fn graph_reply_rechecks_source_then_posts_only_the_reviewed_reply() {
        vak_config::paths::isolate_home_for_tests();
        let agent_id = format!("mailcal-reply-{}", uuid::Uuid::now_v7());
        let account_id = uuid::Uuid::now_v7().to_string();
        let vault = AccountVault::for_agent(&agent_id).unwrap();
        let credential_ref = AccountVault::credential_ref(&account_id).unwrap();
        vault
            .store(
                &account_id,
                crate::vault::AccountSecretMaterial::new(
                    "graph:subject".into(),
                    Some("owner@example.com".into()),
                    None,
                    Some("mock-access-token".into()),
                    None,
                    None,
                    None,
                )
                .unwrap(),
            )
            .unwrap();
        let account = ConnectedAccount {
            id: account_id.clone(),
            provider: Provider::Microsoft,
            status: crate::AccountStatus::Connected,
            owner_agent_id: agent_id.clone(),
            allowed_audiences: [format!("agent:{agent_id}")].into_iter().collect(),
            capabilities: [Capability::MailRead, Capability::MailSend]
                .into_iter()
                .collect(),
            provider_scopes: ["Mail.Read".into(), "Mail.Send".into()]
                .into_iter()
                .collect(),
            credential_ref: credential_ref.clone(),
            principal_ref: credential_ref,
            revision: 1,
            connected_at: chrono::Utc::now(),
            access_token_expires_at: None,
            refresh_token_available: false,
            revoked_at: None,
        };
        let source_reads = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let reply_payload = Arc::new(Mutex::new(None));
        let app = Router::new()
            .route("/v1.0/me/messages/source-1", get({
                let source_reads = Arc::clone(&source_reads);
                move || {
                    let source_reads = Arc::clone(&source_reads);
                    async move {
                        source_reads.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                        Json(json!({"id":"source-1", "conversationId":"conversation-1", "subject":"Project plan"}))
                    }
                }
            }))
            .route("/v1.0/me/messages/source-1/reply", post({
                let reply_payload = Arc::clone(&reply_payload);
                move |Json(payload): Json<Value>| {
                    let reply_payload = Arc::clone(&reply_payload);
                    async move {
                        *reply_payload.lock().unwrap() = Some(payload);
                        axum::http::StatusCode::ACCEPTED
                    }
                }
            }));
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let server = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
        let mut client = ProviderEffectClient::new().unwrap();
        client.microsoft_graph_base = format!("http://{address}/v1.0");
        let draft = MailDraft {
            from_alias: None,
            to: vec![MailAddress {
                address: "recipient@example.com".into(),
                display_name: None,
            }],
            cc: vec![],
            bcc: vec![],
            subject: "Project plan".into(),
            body_text: "Reviewed reply".into(),
            attachment_refs: vec![],
            reply_to_message_id: Some("source-1".into()),
            reply_to_thread_id: Some("conversation-1".into()),
        };

        let mut no_read = account.clone();
        no_read.capabilities.remove(&Capability::MailRead);
        let attempt_id = uuid::Uuid::now_v7().to_string();
        assert_eq!(
            client
                .send_mail(
                    &no_read,
                    &vault,
                    &agent_id,
                    &format!("agent:{agent_id}"),
                    &attempt_id,
                    &draft
                )
                .await,
            Err(ProviderEffectError::NotAdmitted)
        );
        assert_eq!(source_reads.load(std::sync::atomic::Ordering::SeqCst), 0);

        let accepted = client
            .send_mail(
                &account,
                &vault,
                &agent_id,
                &format!("agent:{agent_id}"),
                &attempt_id,
                &draft,
            )
            .await
            .unwrap();
        assert_eq!(accepted.provider_item_id, None);
        assert_eq!(source_reads.load(std::sync::atomic::Ordering::SeqCst), 1);
        let payload = reply_payload.lock().unwrap().clone().unwrap();
        assert_eq!(
            payload.pointer("/message/body/content"),
            Some(&json!("Reviewed reply"))
        );
        assert_eq!(
            payload.pointer("/message/toRecipients/0/emailAddress/address"),
            Some(&json!("recipient@example.com"))
        );
        assert_eq!(
            payload.pointer("/message/internetMessageHeaders/0/value"),
            Some(&json!(attempt_id))
        );
        assert!(payload.pointer("/message/subject").is_none());
        vault.remove(&account_id).unwrap();
        server.abort();
    }

    #[test]
    fn reviewed_event_profile_has_no_invites_or_reminders() {
        let draft = timed_event();
        validate_event_create(&draft).unwrap();
        let attempt_id = uuid::Uuid::now_v7().to_string();
        let google = google_create_event_payload(&draft, &attempt_id);
        assert_eq!(google["attendees"], json!([]));
        assert_eq!(
            google["reminders"],
            json!({"useDefault": false, "overrides": []})
        );
        assert_eq!(google["start"]["timeZone"], "UTC");
        assert_eq!(google["start"]["dateTime"], "2026-10-01T09:00:00+00:00");
        assert_eq!(
            google["extendedProperties"]["private"][GOOGLE_ACTION_PROPERTY],
            attempt_id
        );
        let graph = graph_create_event_payload(&draft, &attempt_id);
        assert_eq!(graph["attendees"], json!([]));
        assert_eq!(graph["isReminderOn"], false);
        assert_eq!(graph["start"]["timeZone"], "UTC");
        assert_eq!(graph["start"]["dateTime"], "2026-10-01T09:00:00");
        assert_eq!(
            graph["singleValueExtendedProperties"][0]["value"],
            attempt_id
        );
    }

    #[test]
    fn reviewed_event_profile_rejects_all_day_attendees_and_recurrence() {
        let mut draft = timed_event();
        draft.all_day = true;
        assert_eq!(
            validate_event_create(&draft),
            Err(ProviderEffectError::Unsupported)
        );
        let mut draft = timed_event();
        draft.attendee_addresses.push(MailAddress {
            address: "person@example.com".into(),
            display_name: None,
        });
        assert_eq!(
            validate_event_create(&draft),
            Err(ProviderEffectError::Unsupported)
        );
        let mut draft = timed_event();
        draft.recurrence = Some("FREQ=DAILY".into());
        assert_eq!(
            validate_event_create(&draft),
            Err(ProviderEffectError::Unsupported)
        );
    }

    #[test]
    fn google_update_requires_the_same_public_standalone_event_version() {
        let source = json!({
            "id":"abcde", "etag":"\"v1\"", "eventType":"default",
            "visibility":"default", "attendees":[],
            "start":{"dateTime":"2026-10-01T09:00:00Z"},
            "end":{"dateTime":"2026-10-01T10:00:00Z"}
        });
        validate_event_update("abcde", "\"v1\"", &timed_event()).unwrap();
        validate_google_update_source(&source, "abcde", "\"v1\"").unwrap();
        assert_eq!(
            validate_google_update_source(&source, "abcde", "\"v2\""),
            Err(ProviderEffectError::Conflict)
        );
        let mut private = source.clone();
        private["visibility"] = json!("private");
        assert_eq!(
            validate_google_update_source(&private, "abcde", "\"v1\""),
            Err(ProviderEffectError::Unsupported)
        );
        let mut invited = source.clone();
        invited["attendees"] = json!([{ "email": "guest@example.com" }]);
        assert_eq!(
            validate_google_update_source(&invited, "abcde", "\"v1\""),
            Err(ProviderEffectError::Unsupported)
        );
    }

    #[test]
    fn google_cancel_requires_one_valid_standalone_event_and_organizer() {
        let source = json!({
            "id":"abcde", "etag":"\"v1\"", "eventType":"default",
            "visibility":"default", "attendees":[], "organizer":{"self":true},
            "start":{"dateTime":"2026-10-01T09:00:00Z"},
            "end":{"dateTime":"2026-10-01T10:00:00Z"}
        });
        validate_event_cancel("abcde", "\"v1\"", None, false).unwrap();
        validate_google_cancel_source(&source, "abcde", "\"v1\"").unwrap();
        assert_eq!(
            validate_event_cancel("abcde", "\"v1\"", Some("instance"), false),
            Err(ProviderEffectError::Rejected)
        );
        assert_eq!(
            validate_event_cancel("abcde", "\"v1\"", None, true),
            Err(ProviderEffectError::Rejected)
        );
        let mut not_organizer = source.clone();
        not_organizer["organizer"]["self"] = json!(false);
        assert_eq!(
            validate_google_cancel_source(&not_organizer, "abcde", "\"v1\""),
            Err(ProviderEffectError::Unsupported)
        );
        let mut invited = source.clone();
        invited["attendees"] = json!([{ "email": "guest@example.com" }]);
        assert_eq!(
            validate_google_cancel_source(&invited, "abcde", "\"v1\""),
            Err(ProviderEffectError::Unsupported)
        );
        assert_eq!(
            validate_google_cancel_source(&source, "abcde", "\"stale\""),
            Err(ProviderEffectError::Conflict)
        );
    }

    #[tokio::test]
    async fn google_cancel_stores_attempt_marker_and_reconciles_ambiguous_cancellation() {
        vak_config::paths::isolate_home_for_tests();
        let agent_id = format!("mailcal-cancel-{}", uuid::Uuid::now_v7());
        let account_id = uuid::Uuid::now_v7().to_string();
        let vault = AccountVault::for_agent(&agent_id).unwrap();
        let credential_ref = AccountVault::credential_ref(&account_id).unwrap();
        vault
            .store(
                &account_id,
                crate::vault::AccountSecretMaterial::new(
                    "google:subject".into(),
                    Some("owner@example.com".into()),
                    None,
                    Some("mock-access-token".into()),
                    None,
                    None,
                    None,
                )
                .unwrap(),
            )
            .unwrap();
        let account = ConnectedAccount {
            id: account_id,
            provider: Provider::Google,
            status: crate::AccountStatus::Connected,
            owner_agent_id: agent_id.clone(),
            allowed_audiences: [format!("agent:{agent_id}")].into_iter().collect(),
            capabilities: [Capability::CalendarRead, Capability::CalendarWrite]
                .into_iter()
                .collect(),
            provider_scopes: ["https://www.googleapis.com/auth/calendar.events".into()]
                .into_iter()
                .collect(),
            credential_ref: credential_ref.clone(),
            principal_ref: credential_ref,
            revision: 1,
            connected_at: chrono::Utc::now(),
            access_token_expires_at: None,
            refresh_token_available: false,
            revoked_at: None,
        };
        let event_state = Arc::new(Mutex::new(json!({
            "id":"abcde", "etag":"\"v1\"", "eventType":"default",
            "visibility":"default", "attendees":[], "organizer":{"self":true},
            "start":{"dateTime":"2026-10-01T09:00:00Z"},
            "end":{"dateTime":"2026-10-01T10:00:00Z"}
        })));
        let state_for_get = event_state.clone();
        let state_for_patch = event_state.clone();
        let patch_observed = Arc::new(Mutex::new(None));
        let patch_capture = patch_observed.clone();
        let app = Router::new().route(
            "/calendar/v3/calendars/primary/events/abcde",
            get(move || {
                let state = state_for_get.clone();
                async move { Json(state.lock().unwrap().clone()) }
            })
            .patch(
                move |headers: axum::http::HeaderMap,
                      uri: axum::http::Uri,
                      Json(body): Json<Value>| {
                    let observed = patch_capture.clone();
                    let state = state_for_patch.clone();
                    async move {
                        *observed.lock().unwrap() = Some((
                            headers
                                .get(axum::http::header::IF_MATCH)
                                .and_then(|v| v.to_str().ok())
                                .map(str::to_owned),
                            uri.query().map(str::to_owned),
                            body.clone(),
                        ));
                        *state.lock().unwrap() = json!({
                            "id":"abcde", "etag":"\"v2\"", "status":"cancelled",
                            "extendedProperties":{"private":{"vak_action_id":body.pointer("/extendedProperties/private/vak_action_id").and_then(Value::as_str).unwrap_or_default()}},
                            "organizer":{"self":true}
                        });
                        // Simulate the provider applying the cancellation but
                        // losing the response after the effect committed.
                        axum::http::StatusCode::INTERNAL_SERVER_ERROR
                    }
                },
            ),
        );
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let server = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
        let mut client = ProviderEffectClient::new().unwrap();
        client.google_calendar_base = format!("http://{address}/calendar/v3");
        let attempt_id = uuid::Uuid::now_v7().to_string();
        let error = client
            .cancel_event(
                &account,
                &vault,
                &agent_id,
                &format!("agent:{agent_id}"),
                &attempt_id,
                "abcde",
                "\"v1\"",
                None,
                false,
            )
            .await
            .unwrap_err();
        assert_eq!(error, ProviderEffectError::Unknown);
        assert_eq!(
            patch_observed.lock().unwrap().as_ref(),
            Some(&(
                Some("\"v1\"".to_owned()),
                Some("sendUpdates=none".to_owned()),
                json!({
                    "status":"cancelled",
                    "extendedProperties":{"private":{"vak_action_id":attempt_id}}
                })
            ))
        );
        assert_eq!(
            client
                .reconcile_cancelled_event(
                    &account,
                    &vault,
                    &agent_id,
                    &format!("agent:{agent_id}"),
                    &attempt_id,
                    "abcde",
                )
                .await
                .unwrap()
                .as_deref(),
            Some("abcde")
        );
        assert_eq!(
            client
                .reconcile_cancelled_event(
                    &account,
                    &vault,
                    &agent_id,
                    &format!("agent:{agent_id}"),
                    &uuid::Uuid::now_v7().to_string(),
                    "abcde",
                )
                .await
                .unwrap(),
            None
        );
        server.abort();
    }

    #[test]
    fn google_update_payload_does_not_replace_attendees_or_reminders() {
        let attempt_id = uuid::Uuid::now_v7().to_string();
        let payload = google_update_event_payload(&timed_event(), &attempt_id);
        assert!(payload.get("attendees").is_none());
        assert!(payload.get("reminders").is_none());
        assert_eq!(
            payload["extendedProperties"]["private"][GOOGLE_ACTION_PROPERTY],
            attempt_id
        );
    }

    #[tokio::test]
    async fn created_event_response_requires_provider_id_and_classifies_unknown() {
        let app = Router::new().route(
            "/event",
            post(|| async { (axum::http::StatusCode::CREATED, r#"{"id":"event-1"}"#) }),
        );
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let server = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
        let response = reqwest::Client::new()
            .post(format!("http://{address}/event"))
            .send()
            .await
            .unwrap();
        let accepted = classify_created_event_response(response).await.unwrap();
        assert_eq!(accepted.provider_item_id.as_deref(), Some("event-1"));
        server.abort();
    }

    #[tokio::test]
    async fn graph_send_uses_fixed_payload_and_reports_only_provider_acceptance() {
        let observed = Arc::new(Mutex::new(Value::Null));
        let observed_handler = observed.clone();
        let app = Router::new().route(
            "/graph/v1.0/me/sendMail",
            post(move |Json(payload): Json<Value>| {
                let observed = observed_handler.clone();
                async move {
                    *observed.lock().unwrap() = payload;
                    axum::http::StatusCode::ACCEPTED
                }
            }),
        );
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let server = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
        let response = reqwest::Client::new()
            .post(format!("http://{address}/graph/v1.0/me/sendMail"))
            .json(&json!({"message":{"subject":"Hi"}}))
            .send()
            .await
            .unwrap();
        let accepted = classify_graph_response(response).await.unwrap();
        assert_eq!(accepted.provider_item_id, None);
        assert_eq!(observed.lock().unwrap()["message"]["subject"], "Hi");
        server.abort();
    }

    #[tokio::test]
    async fn google_send_response_requires_provider_item_id() {
        let app = Router::new().route(
            "/send",
            post(|| async { (axum::http::StatusCode::OK, r#"{"id":"sent-1"}"#) }),
        );
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let server = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
        let response = reqwest::Client::builder()
            .no_proxy()
            .build()
            .unwrap()
            .post(format!("http://{address}/send"))
            .send()
            .await
            .unwrap();
        let accepted = classify_google_response(response).await.unwrap();
        assert_eq!(accepted.provider_item_id.as_deref(), Some("sent-1"));
        server.abort();
    }

    #[tokio::test]
    async fn ambiguous_server_failure_is_unknown_not_retryable() {
        let app = Router::new().route(
            "/send",
            post(|| async { axum::http::StatusCode::SERVICE_UNAVAILABLE }),
        );
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let server = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
        let response = reqwest::Client::builder()
            .no_proxy()
            .build()
            .unwrap()
            .post(format!("http://{address}/send"))
            .send()
            .await
            .unwrap();
        assert_eq!(
            classify_google_response(response).await,
            Err(ProviderEffectError::Unknown)
        );
        server.abort();
    }

    #[test]
    fn google_mime_keeps_bcc_hidden_from_to_and_cc_and_encodes_subject() {
        let draft = MailDraft {
            from_alias: None,
            to: vec![MailAddress {
                address: "to@example.com".into(),
                display_name: None,
            }],
            cc: vec![MailAddress {
                address: "cc@example.com".into(),
                display_name: None,
            }],
            bcc: vec![MailAddress {
                address: "bcc@example.com".into(),
                display_name: None,
            }],
            subject: "Héllo".into(),
            body_text: "body".into(),
            attachment_refs: Vec::new(),
            reply_to_message_id: None,
            reply_to_thread_id: None,
        };
        let raw = google_raw_message(&draft, &uuid::Uuid::now_v7().to_string());
        assert!(raw.contains("To: to@example.com\r\n"));
        assert!(raw.contains("Cc: cc@example.com\r\n"));
        assert!(raw.contains("Bcc: bcc@example.com\r\n"));
        assert!(raw.contains("Subject: =?UTF-8?B?"));
    }

    #[test]
    fn reply_source_must_match_message_conversation_and_subject() {
        let graph = json!({"id":"m1", "conversationId":"c1", "subject":"Re: Project"});
        assert!(validate_graph_reply_source(&graph, "m1", "c1", "Project").is_ok());
        assert_eq!(
            validate_graph_reply_source(&graph, "m1", "other", "Project"),
            Err(ProviderEffectError::Conflict)
        );
        assert_eq!(
            validate_graph_reply_source(&graph, "m1", "c1", "Different"),
            Err(ProviderEffectError::Conflict)
        );
    }

    #[test]
    fn google_reply_headers_are_validated_before_mime_serialization() {
        let source = json!({"id":"m1", "threadId":"t1", "payload":{"headers":[
            {"name":"Subject", "value":"Project"},
            {"name":"Message-ID", "value":"<source@example.com>"},
            {"name":"References", "value":"<prior@example.com>"}
        ]}});
        let headers = validate_google_reply_source(&source, "m1", "t1", "Re: Project").unwrap();
        assert_eq!(
            headers.references,
            "<prior@example.com> <source@example.com>"
        );
        let mut draft = MailDraft {
            from_alias: None,
            to: vec![MailAddress {
                address: "to@example.com".into(),
                display_name: None,
            }],
            cc: vec![],
            bcc: vec![],
            subject: "Re: Project".into(),
            body_text: "reply".into(),
            attachment_refs: vec![],
            reply_to_message_id: Some("m1".into()),
            reply_to_thread_id: Some("t1".into()),
        };
        let raw = google_raw_reply(&draft, &headers, &uuid::Uuid::now_v7().to_string());
        assert!(raw.contains("In-Reply-To: <source@example.com>\r\n"));
        assert!(raw.contains("References: <prior@example.com> <source@example.com>\r\n"));
        draft.subject = "Other subject".into();
        assert_eq!(
            validate_google_reply_source(&source, "m1", "t1", &draft.subject),
            Err(ProviderEffectError::Conflict)
        );
        let injected = json!({"id":"m1", "threadId":"t1", "payload":{"headers":[
            {"name":"Subject", "value":"Project"}, {"name":"Message-ID", "value":"<bad>\r\nBcc: attacker@example.com"}
        ]}});
        assert_eq!(
            validate_google_reply_source(&injected, "m1", "t1", "Project"),
            Err(ProviderEffectError::Conflict)
        );
        assert_eq!(
            decode_subject_header("=?UTF-8?B?SMOpbGxv?="),
            Some("Héllo".into())
        );
        assert_eq!(
            decode_subject_header("=?UTF-8?Q?H=C3=A9llo?="),
            Some("Héllo".into())
        );
        assert_eq!(decode_subject_header("=?UTF-8?B?%%%?="), None);
    }

    #[test]
    fn reply_resource_ids_stay_inside_the_fixed_provider_path() {
        let url = path_url(
            "https://graph.microsoft.com/v1.0",
            &["me", "messages", "id/../../evil"],
        )
        .unwrap();
        assert_eq!(url.host_str(), Some("graph.microsoft.com"));
        assert!(url.path().contains("id%2F..%2F..%2Fevil"));
    }
}
