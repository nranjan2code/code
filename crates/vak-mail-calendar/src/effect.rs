//! Fixed-host, non-retrying provider effects.
//!
//! The caller owns the durable single-use claim and must record `Unknown`
//! after an ambiguous result. This client never follows redirects or retries.

use crate::{
    CalendarDraft, Capability, ConnectedAccount, MailAddress, MailDraft, Provider,
    provider::ProviderReadError, vault::AccountVault,
};
use base64::Engine;
use reqwest::{Response, StatusCode};
use serde_json::{Value, json};
use std::time::Duration;

const MAX_RESPONSE_BYTES: usize = 64 * 1024;

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
                    let raw = google_raw_reply(draft, &headers);
                    json!({"threadId": thread_id, "raw": base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(raw.as_bytes())})
                } else {
                    let raw = google_raw_message(draft);
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
                    let payload = graph_reply_payload(draft);
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
                        .json(&graph_send_payload(draft))
                        .send()
                        .await
                        .map_err(|_| ProviderEffectError::Unknown)?
                };
                classify_graph_response(response).await
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
                .json(&google_create_event_payload(draft))
                .send()
                .await
                .map_err(|_| ProviderEffectError::Unknown)?,
            Provider::Microsoft => self
                .http
                .post(format!("{}/me/events", self.microsoft_graph_base))
                .bearer_auth(token.as_str())
                .json(&graph_create_event_payload(draft))
                .send()
                .await
                .map_err(|_| ProviderEffectError::Unknown)?,
            Provider::AppleIcloud => return Err(ProviderEffectError::Unsupported),
        };
        classify_created_event_response(response).await
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
            .json(&google_update_event_payload(draft))
            .send()
            .await
            .map_err(|_| ProviderEffectError::Unknown)?;
        classify_updated_event_response(response).await
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

fn google_create_event_payload(draft: &CalendarDraft) -> Value {
    json!({
        "summary": draft.title,
        "description": draft.description,
        "location": draft.location,
        "start": {"dateTime": draft.starts_at.to_rfc3339(), "timeZone": "UTC"},
        "end": {"dateTime": draft.ends_at.to_rfc3339(), "timeZone": "UTC"},
        "attendees": [],
        "reminders": {"useDefault": false, "overrides": []}
    })
}

fn google_update_event_payload(draft: &CalendarDraft) -> Value {
    json!({
        "summary": draft.title,
        "description": draft.description,
        "location": draft.location,
        "start": {"dateTime": draft.starts_at.to_rfc3339(), "timeZone": "UTC"},
        "end": {"dateTime": draft.ends_at.to_rfc3339(), "timeZone": "UTC"}
    })
}

fn graph_create_event_payload(draft: &CalendarDraft) -> Value {
    json!({
        "subject": draft.title,
        "body": {"contentType": "Text", "content": draft.description},
        "location": {"displayName": draft.location.as_deref().unwrap_or("")},
        "start": {"dateTime": draft.starts_at.format("%Y-%m-%dT%H:%M:%S").to_string(), "timeZone": "UTC"},
        "end": {"dateTime": draft.ends_at.format("%Y-%m-%dT%H:%M:%S").to_string(), "timeZone": "UTC"},
        "attendees": [],
        "isReminderOn": false
    })
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

fn graph_send_payload(draft: &MailDraft) -> Value {
    json!({
        "message": {
            "subject": draft.subject,
            "body": {"contentType": "Text", "content": draft.body_text},
            "toRecipients": graph_recipients(&draft.to),
            "ccRecipients": graph_recipients(&draft.cc),
            "bccRecipients": graph_recipients(&draft.bcc)
        },
        "saveToSentItems": true
    })
}

fn graph_reply_payload(draft: &MailDraft) -> Value {
    json!({"message": {
        "body": {"contentType": "Text", "content": draft.body_text},
        "toRecipients": graph_recipients(&draft.to),
        "ccRecipients": graph_recipients(&draft.cc),
        "bccRecipients": graph_recipients(&draft.bcc)
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

fn google_raw_reply(draft: &MailDraft, headers: &GoogleReplyHeaders) -> String {
    let mut raw = google_raw_message(draft);
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

fn google_raw_message(draft: &MailDraft) -> String {
    let mut raw = String::new();
    raw.push_str(&format!("To: {}\r\n", google_addresses(&draft.to)));
    if !draft.cc.is_empty() {
        raw.push_str(&format!("Cc: {}\r\n", google_addresses(&draft.cc)));
    }
    if !draft.bcc.is_empty() {
        raw.push_str(&format!("Bcc: {}\r\n", google_addresses(&draft.bcc)));
    }
    raw.push_str(&format!("Subject: {}\r\n", encoded_header(&draft.subject)));
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
    use axum::{Json, Router, routing::post};
    use serde_json::json;
    use std::sync::{Arc, Mutex};

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

    #[test]
    fn reviewed_event_profile_has_no_invites_or_reminders() {
        let draft = timed_event();
        validate_event_create(&draft).unwrap();
        let google = google_create_event_payload(&draft);
        assert_eq!(google["attendees"], json!([]));
        assert_eq!(
            google["reminders"],
            json!({"useDefault": false, "overrides": []})
        );
        assert_eq!(google["start"]["timeZone"], "UTC");
        assert_eq!(google["start"]["dateTime"], "2026-10-01T09:00:00+00:00");
        let graph = graph_create_event_payload(&draft);
        assert_eq!(graph["attendees"], json!([]));
        assert_eq!(graph["isReminderOn"], false);
        assert_eq!(graph["start"]["timeZone"], "UTC");
        assert_eq!(graph["start"]["dateTime"], "2026-10-01T09:00:00");
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
    fn google_update_payload_does_not_replace_attendees_or_reminders() {
        let payload = google_update_event_payload(&timed_event());
        assert!(payload.get("attendees").is_none());
        assert!(payload.get("reminders").is_none());
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
        let raw = google_raw_message(&draft);
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
        let raw = google_raw_reply(&draft, &headers);
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
