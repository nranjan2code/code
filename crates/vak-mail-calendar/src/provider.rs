//! Bounded, read-only provider adapters for the first mail/calendar slice.
//!
//! URLs are fixed per provider, redirects are disabled, responses are capped,
//! and operations require the exact account capability before a vault token is
//! loaded. Provider content is returned only to the caller; this module does
//! not create a local content cache.

use crate::{Capability, ConnectedAccount, Provider, vault::AccountVault};
use base64::Engine;
use chrono::{DateTime, Duration, Utc};
use reqwest::{Response, StatusCode};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::time::Duration as StdDuration;

const MAX_MAIL_ITEMS: usize = 20;
const MAX_EVENT_ITEMS: usize = 100;
const MAX_RESPONSE_BYTES: usize = 512 * 1024;
const MAX_MESSAGE_BYTES: usize = 128 * 1024;
const MAX_TEXT_BYTES: usize = 16 * 1024;

#[derive(Debug, thiserror::Error)]
pub enum ProviderReadError {
    #[error("the connected account does not grant this read")]
    NotAdmitted,
    #[error("this provider does not support the requested operation")]
    Unsupported,
    #[error("the provider needs sign-in again")]
    ReauthenticationRequired,
    #[error("provider service is unavailable")]
    Unavailable,
    #[error("provider response was invalid or exceeded its size limit")]
    InvalidResponse,
    #[error("requested time range is outside the allowed window")]
    InvalidRange,
    #[error("credential vault is unavailable")]
    Vault,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MailItem {
    pub provider_id: String,
    pub thread_id: Option<String>,
    pub from: Option<String>,
    pub subject: String,
    pub received_at: Option<DateTime<Utc>>,
    pub preview: String,
    pub body_text: Option<String>,
    pub has_attachments: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CalendarItem {
    pub provider_id: String,
    /// Opaque provider version token; never display it in the everyday UI.
    pub version: Option<String>,
    pub title: String,
    pub starts_at: Option<DateTime<Utc>>,
    pub ends_at: Option<DateTime<Utc>>,
    pub starts_on: Option<String>,
    pub ends_on: Option<String>,
    pub all_day: bool,
    pub location: Option<String>,
    pub description: Option<String>,
    pub attendee_count: usize,
    pub recurring: bool,
    pub private: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BusySlot {
    pub starts_at: DateTime<Utc>,
    pub ends_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CalendarRange {
    pub from: DateTime<Utc>,
    pub to: DateTime<Utc>,
    pub limit: usize,
}

#[derive(Clone)]
pub struct ProviderReadClient {
    http: reqwest::Client,
    google_gmail_base: String,
    google_calendar_base: String,
    microsoft_graph_base: String,
}

impl Default for ProviderReadClient {
    fn default() -> Self {
        Self::new()
    }
}

impl ProviderReadClient {
    pub fn new() -> Self {
        let http = reqwest::Client::builder()
            .no_proxy()
            .redirect(reqwest::redirect::Policy::none())
            .timeout(StdDuration::from_secs(20))
            .build()
            .unwrap_or_default();
        Self {
            http,
            google_gmail_base: "https://gmail.googleapis.com/gmail/v1".into(),
            google_calendar_base: "https://www.googleapis.com/calendar/v3".into(),
            microsoft_graph_base: "https://graph.microsoft.com/v1.0".into(),
        }
    }

    pub async fn recent_mail(
        &self,
        account: &ConnectedAccount,
        vault: &AccountVault,
        agent_id: &str,
        audience: &str,
        limit: usize,
    ) -> Result<Vec<MailItem>, ProviderReadError> {
        admit(account, vault, agent_id, audience, Capability::MailRead)?;
        let limit = limit.clamp(1, MAX_MAIL_ITEMS);
        let token = vault
            .access_token(&account.id)
            .map_err(|_| ProviderReadError::Vault)?;
        match account.provider {
            Provider::Google => self.google_mail(token.as_str(), limit).await,
            Provider::Microsoft => self.microsoft_mail(token.as_str(), limit).await,
            Provider::AppleIcloud => Err(ProviderReadError::Unsupported),
        }
    }

    /// Return only stable provider message IDs for a bounded recent window.
    /// Used by scheduled watches before model admission so a quiet poll never
    /// downloads message bodies merely to decide that there is no new work.
    pub async fn recent_mail_ids(
        &self,
        account: &ConnectedAccount,
        vault: &AccountVault,
        agent_id: &str,
        audience: &str,
        limit: usize,
    ) -> Result<Vec<String>, ProviderReadError> {
        admit(account, vault, agent_id, audience, Capability::MailRead)?;
        let token = vault
            .access_token(&account.id)
            .map_err(|_| ProviderReadError::Vault)?;
        let limit = limit.clamp(1, MAX_MAIL_ITEMS).to_string();
        let response = match account.provider {
            Provider::Google => {
                self.http
                    .get(format!("{}/users/me/messages", self.google_gmail_base))
                    .bearer_auth(token.as_str())
                    .query(&[("labelIds", "INBOX"), ("maxResults", limit.as_str())])
                    .send()
                    .await
            }
            Provider::Microsoft => {
                self.http
                    .get(format!(
                        "{}/me/mailFolders/inbox/messages",
                        self.microsoft_graph_base
                    ))
                    .bearer_auth(token.as_str())
                    .query(&[
                        ("$top", limit.as_str()),
                        ("$orderby", "receivedDateTime desc"),
                    ])
                    .query(&[("$select", "id")])
                    .send()
                    .await
            }
            Provider::AppleIcloud => return Err(ProviderReadError::Unsupported),
        }
        .map_err(|_| ProviderReadError::Unavailable)?;
        let value = parse_response(response, MAX_RESPONSE_BYTES).await?;
        let items = match account.provider {
            Provider::Google => value.get("messages").and_then(Value::as_array),
            Provider::Microsoft => value.get("value").and_then(Value::as_array),
            Provider::AppleIcloud => None,
        }
        .ok_or(ProviderReadError::InvalidResponse)?;
        Ok(items
            .iter()
            .take(limit.parse().unwrap_or(1))
            .filter_map(|item| item.get("id").and_then(Value::as_str))
            .filter(|id| !id.is_empty() && id.len() <= 512 && !id.chars().any(char::is_control))
            .map(str::to_owned)
            .collect())
    }

    pub async fn calendar_events(
        &self,
        account: &ConnectedAccount,
        vault: &AccountVault,
        agent_id: &str,
        audience: &str,
        range: CalendarRange,
    ) -> Result<Vec<CalendarItem>, ProviderReadError> {
        admit(account, vault, agent_id, audience, Capability::CalendarRead)?;
        validate_range(range.from, range.to)?;
        let limit = range.limit.clamp(1, MAX_EVENT_ITEMS);
        let token = vault
            .access_token(&account.id)
            .map_err(|_| ProviderReadError::Vault)?;
        match account.provider {
            Provider::Google => {
                self.google_events(token.as_str(), range.from, range.to, limit)
                    .await
            }
            Provider::Microsoft => {
                self.microsoft_events(token.as_str(), range.from, range.to, limit)
                    .await
            }
            Provider::AppleIcloud => Err(ProviderReadError::Unsupported),
        }
    }

    pub async fn free_busy(
        &self,
        account: &ConnectedAccount,
        vault: &AccountVault,
        agent_id: &str,
        audience: &str,
        from: DateTime<Utc>,
        to: DateTime<Utc>,
    ) -> Result<Vec<BusySlot>, ProviderReadError> {
        admit(
            account,
            vault,
            agent_id,
            audience,
            Capability::CalendarFreeBusy,
        )?;
        validate_range(from, to)?;
        let token = vault
            .access_token(&account.id)
            .map_err(|_| ProviderReadError::Vault)?;
        match account.provider {
            Provider::Google => self.google_free_busy(token.as_str(), from, to).await,
            Provider::Microsoft => {
                let identity = vault
                    .display_identity(&account.id)
                    .map_err(|_| ProviderReadError::Vault)?
                    .ok_or(ProviderReadError::Unsupported)?;
                if !identity.contains('@') || identity.chars().any(char::is_control) {
                    return Err(ProviderReadError::Unsupported);
                }
                self.microsoft_free_busy(token.as_str(), identity.as_str(), from, to)
                    .await
            }
            Provider::AppleIcloud => Err(ProviderReadError::Unsupported),
        }
    }

    async fn google_mail(
        &self,
        token: &str,
        limit: usize,
    ) -> Result<Vec<MailItem>, ProviderReadError> {
        let response = self
            .http
            .get(format!("{}/users/me/messages", self.google_gmail_base))
            .bearer_auth(token)
            .query(&[("labelIds", "INBOX"), ("maxResults", &limit.to_string())])
            .send()
            .await
            .map_err(|_| ProviderReadError::Unavailable)?;
        let list = parse_response(response, MAX_RESPONSE_BYTES).await?;
        let ids = list
            .get("messages")
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default();
        let mut output = Vec::with_capacity(ids.len().min(limit));
        for entry in ids.into_iter().take(limit) {
            let Some(id) = entry.get("id").and_then(Value::as_str) else {
                continue;
            };
            if id.is_empty()
                || id.len() > 256
                || id
                    .bytes()
                    .any(|b| !b.is_ascii_alphanumeric() && b != b'_' && b != b'-')
            {
                continue;
            }
            let url = format!("{}/users/me/messages/{id}", self.google_gmail_base);
            let response = self
                .http
                .get(url)
                .bearer_auth(token)
                .query(&[("format", "full")])
                .send()
                .await
                .map_err(|_| ProviderReadError::Unavailable)?;
            let value = parse_response(response, MAX_MESSAGE_BYTES).await?;
            output.push(parse_google_message(&value));
        }
        Ok(output)
    }

    async fn microsoft_mail(
        &self,
        token: &str,
        limit: usize,
    ) -> Result<Vec<MailItem>, ProviderReadError> {
        let limit = limit.to_string();
        let response = self
            .http
            .get(format!(
                "{}/me/mailFolders/inbox/messages",
                self.microsoft_graph_base
            ))
            .bearer_auth(token)
            .header(
                "Prefer",
                "outlook.timezone=\"UTC\", outlook.body-content-type=\"text\"",
            )
            .query(&[
                ("$top", limit.as_str()),
                ("$orderby", "receivedDateTime desc"),
            ])
            .query(&[(
                "$select",
                "id,conversationId,from,subject,receivedDateTime,bodyPreview,body,hasAttachments",
            )])
            .send()
            .await
            .map_err(|_| ProviderReadError::Unavailable)?;
        let value = parse_response(response, MAX_RESPONSE_BYTES).await?;
        let items = value
            .get("value")
            .and_then(Value::as_array)
            .ok_or(ProviderReadError::InvalidResponse)?;
        Ok(items
            .iter()
            .take(limit.parse().unwrap_or(1))
            .filter_map(parse_graph_message)
            .collect())
    }

    async fn google_events(
        &self,
        token: &str,
        from: DateTime<Utc>,
        to: DateTime<Utc>,
        limit: usize,
    ) -> Result<Vec<CalendarItem>, ProviderReadError> {
        let max_items = limit.min(MAX_EVENT_ITEMS);
        let query_limit = max_items.to_string();
        let response = self
            .http
            .get(format!(
                "{}/calendars/primary/events",
                self.google_calendar_base
            ))
            .bearer_auth(token)
            .query(&[("timeMin", from.to_rfc3339()), ("timeMax", to.to_rfc3339())])
            .query(&[
                ("singleEvents", "true".to_string()),
                ("orderBy", "startTime".to_string()),
                ("maxResults", query_limit),
            ])
            .send()
            .await
            .map_err(|_| ProviderReadError::Unavailable)?;
        let value = parse_response(response, MAX_RESPONSE_BYTES).await?;
        let items = value
            .get("items")
            .and_then(Value::as_array)
            .ok_or(ProviderReadError::InvalidResponse)?;
        Ok(items
            .iter()
            .take(max_items)
            .filter_map(parse_google_event)
            .collect())
    }

    async fn microsoft_events(
        &self,
        token: &str,
        from: DateTime<Utc>,
        to: DateTime<Utc>,
        limit: usize,
    ) -> Result<Vec<CalendarItem>, ProviderReadError> {
        let limit = limit.to_string();
        let response = self
            .http
            .get(format!("{}/me/calendarView", self.microsoft_graph_base))
            .bearer_auth(token)
            .query(&[
                ("startDateTime", from.to_rfc3339()),
                ("endDateTime", to.to_rfc3339()),
            ])
            .query(&[("$top", limit.as_str()), ("$orderby", "start/dateTime")])
            .query(&[(
                "$select",
                "id,subject,start,end,isAllDay,location,body,attendees,sensitivity",
            )])
            .header(
                "Prefer",
                "outlook.timezone=\"UTC\", outlook.body-content-type=\"text\"",
            )
            .send()
            .await
            .map_err(|_| ProviderReadError::Unavailable)?;
        let value = parse_response(response, MAX_RESPONSE_BYTES).await?;
        let items = value
            .get("value")
            .and_then(Value::as_array)
            .ok_or(ProviderReadError::InvalidResponse)?;
        Ok(items
            .iter()
            .take(limit.parse().unwrap_or(1))
            .filter_map(parse_graph_event)
            .collect())
    }

    async fn google_free_busy(
        &self,
        token: &str,
        from: DateTime<Utc>,
        to: DateTime<Utc>,
    ) -> Result<Vec<BusySlot>, ProviderReadError> {
        let response = self
            .http
            .post(format!("{}/freeBusy", self.google_calendar_base))
            .bearer_auth(token)
            .json(&json!({"timeMin": from.to_rfc3339(), "timeMax": to.to_rfc3339(), "items": [{"id": "primary"}]}))
            .send()
            .await
            .map_err(|_| ProviderReadError::Unavailable)?;
        let value = parse_response(response, MAX_RESPONSE_BYTES).await?;
        let calendars = value
            .get("calendars")
            .and_then(Value::as_object)
            .ok_or(ProviderReadError::InvalidResponse)?;
        let busy = calendars
            .get("primary")
            .and_then(|v| v.get("busy"))
            .and_then(Value::as_array)
            .ok_or(ProviderReadError::InvalidResponse)?;
        Ok(busy
            .iter()
            .filter_map(parse_busy)
            .take(MAX_EVENT_ITEMS)
            .collect())
    }

    async fn microsoft_free_busy(
        &self,
        token: &str,
        account_email: &str,
        from: DateTime<Utc>,
        to: DateTime<Utc>,
    ) -> Result<Vec<BusySlot>, ProviderReadError> {
        let response = self
            .http
            .post(format!(
                "{}/me/calendar/getSchedule",
                self.microsoft_graph_base
            ))
            .bearer_auth(token)
            .header("Prefer", "outlook.timezone=\"UTC\"")
            .json(&json!({
                "schedules": [account_email],
                "startTime": {"dateTime": from.format("%Y-%m-%dT%H:%M:%S").to_string(), "timeZone": "UTC"},
                "endTime": {"dateTime": to.format("%Y-%m-%dT%H:%M:%S").to_string(), "timeZone": "UTC"},
                "availabilityViewInterval": 30
            }))
            .send()
            .await
            .map_err(|_| ProviderReadError::Unavailable)?;
        if response.status() == StatusCode::BAD_REQUEST {
            // Graph's getSchedule is explicitly unsupported for delegated
            // personal Microsoft accounts. Requests here are fixed and
            // locally range-validated, so a 400 is surfaced as unsupported.
            return Err(ProviderReadError::Unsupported);
        }
        let value = parse_response(response, MAX_RESPONSE_BYTES).await?;
        let schedules = value
            .get("value")
            .and_then(Value::as_array)
            .ok_or(ProviderReadError::Unsupported)?;
        let slots = schedules
            .first()
            .and_then(|s| s.get("scheduleItems"))
            .and_then(Value::as_array)
            .ok_or(ProviderReadError::InvalidResponse)?;
        Ok(slots
            .iter()
            .filter_map(|slot| {
                if slot.get("status").and_then(Value::as_str) == Some("free") {
                    return None;
                }
                let start = slot
                    .get("start")
                    .and_then(|v| v.get("dateTime"))
                    .and_then(Value::as_str)
                    .and_then(parse_provider_datetime)?;
                let end = slot
                    .get("end")
                    .and_then(|v| v.get("dateTime"))
                    .and_then(Value::as_str)
                    .and_then(parse_provider_datetime)?;
                Some(BusySlot {
                    starts_at: start,
                    ends_at: end,
                })
            })
            .take(MAX_EVENT_ITEMS)
            .collect())
    }
}

fn admit(
    account: &ConnectedAccount,
    vault: &AccountVault,
    agent: &str,
    audience: &str,
    capability: Capability,
) -> Result<(), ProviderReadError> {
    if vault.agent_id() != account.owner_agent_id || !account.admits(agent, audience, capability) {
        return Err(ProviderReadError::NotAdmitted);
    }
    if account.provider == Provider::AppleIcloud {
        return Err(ProviderReadError::Unsupported);
    }
    Ok(())
}

fn validate_range(from: DateTime<Utc>, to: DateTime<Utc>) -> Result<(), ProviderReadError> {
    if to <= from
        || to - from > Duration::days(31)
        || from < Utc::now() - Duration::days(365)
        || to > Utc::now() + Duration::days(366)
    {
        return Err(ProviderReadError::InvalidRange);
    }
    Ok(())
}

async fn parse_response(mut response: Response, limit: usize) -> Result<Value, ProviderReadError> {
    if response.status() == StatusCode::UNAUTHORIZED || response.status() == StatusCode::FORBIDDEN {
        return Err(ProviderReadError::ReauthenticationRequired);
    }
    if !response.status().is_success() {
        return Err(ProviderReadError::Unavailable);
    }
    if response
        .content_length()
        .is_some_and(|size| size > limit as u64)
    {
        return Err(ProviderReadError::InvalidResponse);
    }
    let mut body = Vec::new();
    while let Some(chunk) = response
        .chunk()
        .await
        .map_err(|_| ProviderReadError::Unavailable)?
    {
        if body.len().saturating_add(chunk.len()) > limit {
            return Err(ProviderReadError::InvalidResponse);
        }
        body.extend_from_slice(&chunk);
    }
    serde_json::from_slice(&body).map_err(|_| ProviderReadError::InvalidResponse)
}

fn parse_google_message(value: &Value) -> MailItem {
    let headers = value.pointer("/payload/headers").and_then(Value::as_array);
    let header = |name: &str| {
        headers?
            .iter()
            .find(|h| {
                h.get("name")
                    .and_then(Value::as_str)
                    .is_some_and(|n| n.eq_ignore_ascii_case(name))
            })
            .and_then(|h| h.get("value"))
            .and_then(Value::as_str)
            .map(bounded_text)
    };
    let body = google_plain_text(value.get("payload").unwrap_or(&Value::Null));
    MailItem {
        provider_id: value
            .get("id")
            .and_then(Value::as_str)
            .map(bounded_text)
            .unwrap_or_default(),
        thread_id: value
            .get("threadId")
            .and_then(Value::as_str)
            .map(bounded_text),
        from: header("From"),
        subject: header("Subject").unwrap_or_default(),
        received_at: value
            .get("internalDate")
            .and_then(Value::as_str)
            .and_then(|v| v.parse::<i64>().ok())
            .and_then(DateTime::from_timestamp_millis),
        preview: value
            .get("snippet")
            .and_then(Value::as_str)
            .map(bounded_text)
            .unwrap_or_default(),
        body_text: body,
        has_attachments: has_gmail_attachment(value.get("payload").unwrap_or(&Value::Null)),
    }
}

fn google_plain_text(payload: &Value) -> Option<String> {
    if payload.get("mimeType").and_then(Value::as_str) == Some("text/plain") {
        let encoded = payload.pointer("/body/data").and_then(Value::as_str)?;
        let decoded = base64::engine::general_purpose::URL_SAFE_NO_PAD
            .decode(encoded)
            .or_else(|_| base64::engine::general_purpose::URL_SAFE.decode(encoded))
            .ok()?;
        return String::from_utf8(decoded)
            .ok()
            .map(|text| bounded_text(&text));
    }
    payload
        .get("parts")
        .and_then(Value::as_array)?
        .iter()
        .find_map(google_plain_text)
}

fn has_gmail_attachment(payload: &Value) -> bool {
    payload
        .get("filename")
        .and_then(Value::as_str)
        .is_some_and(|name| !name.is_empty())
        || payload
            .get("parts")
            .and_then(Value::as_array)
            .is_some_and(|parts| parts.iter().any(has_gmail_attachment))
}

fn parse_graph_message(value: &Value) -> Option<MailItem> {
    let body = value
        .pointer("/body/content")
        .and_then(Value::as_str)
        .map(bounded_text);
    Some(MailItem {
        provider_id: bounded_text(value.get("id")?.as_str()?),
        thread_id: value
            .get("conversationId")
            .and_then(Value::as_str)
            .map(bounded_text),
        from: value
            .pointer("/from/emailAddress/address")
            .and_then(Value::as_str)
            .map(bounded_text),
        subject: value
            .get("subject")
            .and_then(Value::as_str)
            .map(bounded_text)
            .unwrap_or_default(),
        received_at: value
            .get("receivedDateTime")
            .and_then(Value::as_str)
            .and_then(parse_datetime),
        preview: value
            .get("bodyPreview")
            .and_then(Value::as_str)
            .map(bounded_text)
            .unwrap_or_default(),
        body_text: body,
        has_attachments: value
            .get("hasAttachments")
            .and_then(Value::as_bool)
            .unwrap_or(false),
    })
}

fn parse_google_event(value: &Value) -> Option<CalendarItem> {
    let start = value.get("start")?;
    let end = value.get("end")?;
    let private = value.get("visibility").and_then(Value::as_str) == Some("private");
    let starts_at = start
        .get("dateTime")
        .and_then(Value::as_str)
        .and_then(parse_datetime);
    let ends_at = end
        .get("dateTime")
        .and_then(Value::as_str)
        .and_then(parse_datetime);
    Some(CalendarItem {
        provider_id: bounded_text(value.get("id")?.as_str()?),
        version: value.get("etag").and_then(Value::as_str).map(bounded_text),
        title: if private {
            "Private event".into()
        } else {
            value
                .get("summary")
                .and_then(Value::as_str)
                .map(bounded_text)
                .unwrap_or_else(|| "(untitled event)".into())
        },
        starts_at,
        ends_at,
        starts_on: start.get("date").and_then(Value::as_str).map(bounded_text),
        ends_on: end.get("date").and_then(Value::as_str).map(bounded_text),
        all_day: start.get("date").is_some(),
        location: if private {
            None
        } else {
            value
                .get("location")
                .and_then(Value::as_str)
                .map(bounded_text)
        },
        description: if private {
            None
        } else {
            value
                .get("description")
                .and_then(Value::as_str)
                .map(bounded_text)
        },
        attendee_count: if private {
            0
        } else {
            value
                .get("attendees")
                .and_then(Value::as_array)
                .map_or(0, Vec::len)
        },
        recurring: value.get("recurringEventId").is_some()
            || value
                .get("recurrence")
                .and_then(Value::as_array)
                .is_some_and(|rules| !rules.is_empty()),
        private,
    })
}

fn parse_graph_event(value: &Value) -> Option<CalendarItem> {
    let start = value.get("start")?;
    let end = value.get("end")?;
    let private = value.get("sensitivity").and_then(Value::as_str) == Some("private");
    let all_day = value
        .get("isAllDay")
        .and_then(Value::as_bool)
        .unwrap_or(false);
    Some(CalendarItem {
        provider_id: bounded_text(value.get("id")?.as_str()?),
        version: value
            .get("@odata.etag")
            .or_else(|| value.get("changeKey"))
            .and_then(Value::as_str)
            .map(bounded_text),
        title: if private {
            "Private event".into()
        } else {
            value
                .get("subject")
                .and_then(Value::as_str)
                .map(bounded_text)
                .unwrap_or_else(|| "(untitled event)".into())
        },
        starts_at: start
            .get("dateTime")
            .and_then(Value::as_str)
            .and_then(parse_provider_datetime),
        ends_at: end
            .get("dateTime")
            .and_then(Value::as_str)
            .and_then(parse_provider_datetime),
        starts_on: if all_day { date_part(start) } else { None },
        ends_on: if all_day { date_part(end) } else { None },
        all_day,
        location: if private {
            None
        } else {
            value
                .pointer("/location/displayName")
                .and_then(Value::as_str)
                .map(bounded_text)
        },
        description: if private {
            None
        } else {
            value
                .pointer("/body/content")
                .and_then(Value::as_str)
                .map(bounded_text)
        },
        attendee_count: value
            .get("attendees")
            .and_then(Value::as_array)
            .map_or(0, Vec::len),
        recurring: value
            .get("type")
            .and_then(Value::as_str)
            .is_some_and(|kind| kind != "singleInstance"),
        private,
    })
}

fn date_part(value: &Value) -> Option<String> {
    value
        .get("dateTime")
        .and_then(Value::as_str)
        .and_then(|date_time| date_time.get(..10))
        .map(bounded_text)
}

fn parse_busy(value: &Value) -> Option<BusySlot> {
    Some(BusySlot {
        starts_at: value
            .get("start")
            .and_then(Value::as_str)
            .and_then(parse_datetime)?,
        ends_at: value
            .get("end")
            .and_then(Value::as_str)
            .and_then(parse_datetime)?,
    })
}

fn parse_datetime(value: &str) -> Option<DateTime<Utc>> {
    DateTime::parse_from_rfc3339(value)
        .ok()
        .map(|date| date.with_timezone(&Utc))
}

fn parse_provider_datetime(value: &str) -> Option<DateTime<Utc>> {
    parse_datetime(value).or_else(|| {
        chrono::NaiveDateTime::parse_from_str(value, "%Y-%m-%dT%H:%M:%S")
            .ok()
            .map(|date_time| date_time.and_utc())
    })
}

fn bounded_text(value: &str) -> String {
    let mut text = value
        .char_indices()
        .take_while(|(offset, character)| offset + character.len_utf8() <= MAX_TEXT_BYTES)
        .map(|(_, character)| character)
        .collect::<String>();
    text.retain(|ch| ch == '\n' || ch == '\t' || !ch.is_control());
    text
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{AccountStatus, vault::AccountSecretMaterial};
    use std::collections::BTreeSet;
    use std::sync::Arc;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use uuid::Uuid;

    #[test]
    fn private_google_event_suppresses_details_but_keeps_busy_time() {
        let value = json!({
            "id": "e1", "etag": "\"version-1\"", "summary": "Private title", "visibility": "private",
            "start": {"dateTime": "2026-09-30T10:00:00Z"},
            "end": {"dateTime": "2026-09-30T11:00:00Z"},
            "attendees": [{"email": "secret@example.test"}]
        });
        let parsed = parse_google_event(&value).unwrap();
        assert!(parsed.private);
        assert_eq!(parsed.version.as_deref(), Some("\"version-1\""));
        assert!(!parsed.recurring);
        assert_eq!(parsed.title, "Private event");
        assert_eq!(parsed.location, None);
        assert_eq!(parsed.description, None);
        assert_eq!(parsed.attendee_count, 0);
        assert_eq!(
            parsed.starts_at,
            Some(parse_datetime("2026-09-30T10:00:00Z").unwrap())
        );
    }

    #[test]
    fn private_graph_event_suppresses_title_location_and_body() {
        let value = json!({
            "id": "e1", "subject": "Private title", "sensitivity": "private",
            "start": {"dateTime": "2026-09-30T10:00:00Z"},
            "end": {"dateTime": "2026-09-30T11:00:00Z"},
            "location": {"displayName": "Secret room"}, "body": {"content": "Secret"}
        });
        let parsed = parse_graph_event(&value).unwrap();
        assert_eq!(parsed.title, "Private event");
        assert_eq!(parsed.location, None);
        assert_eq!(parsed.description, None);
    }

    #[test]
    fn mailbox_body_parser_uses_text_plain_only_and_caps_output() {
        let text = "hello";
        let encoded = base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(text);
        let payload = json!({"mimeType":"text/plain", "body":{"data":encoded}});
        assert_eq!(google_plain_text(&payload).as_deref(), Some("hello"));
        let html = json!({"mimeType":"text/html", "body":{"data":encoded}});
        assert_eq!(google_plain_text(&html), None);
    }

    #[test]
    fn read_time_window_is_limited_to_one_month() {
        let now = Utc::now();
        assert!(validate_range(now, now + Duration::days(32)).is_err());
        assert!(validate_range(now, now + Duration::days(30)).is_ok());
    }

    #[test]
    fn graph_calendar_values_without_offsets_are_interpreted_as_utc() {
        assert_eq!(
            parse_provider_datetime("2026-09-30T10:30:00").unwrap(),
            parse_datetime("2026-09-30T10:30:00Z").unwrap()
        );
        let value = json!({
            "id": "event", "isAllDay": true, "start": {"dateTime":"2026-09-30T00:00:00"},
            "end": {"dateTime":"2026-10-01T00:00:00"}
        });
        assert_eq!(
            parse_graph_event(&value).unwrap().starts_on.as_deref(),
            Some("2026-09-30")
        );
    }

    #[tokio::test]
    async fn provider_responses_are_size_limited_and_redirects_are_not_followed() {
        let app = axum::Router::new()
            .route(
                "/oversized",
                axum::routing::get(|| async {
                    axum::http::Response::builder()
                        .status(StatusCode::OK)
                        .body(axum::body::Body::from(vec![b'x'; 128]))
                        .unwrap()
                }),
            )
            .route(
                "/redirect",
                axum::routing::get(|| async {
                    axum::http::Response::builder()
                        .status(StatusCode::FOUND)
                        .header("Location", "/oversized")
                        .body(axum::body::Body::empty())
                        .unwrap()
                }),
            );
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let task = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
        let http = reqwest::Client::builder()
            .no_proxy()
            .redirect(reqwest::redirect::Policy::none())
            .build()
            .unwrap();
        let oversized = http
            .get(format!("http://{address}/oversized"))
            .send()
            .await
            .unwrap();
        assert!(matches!(
            parse_response(oversized, 64).await,
            Err(ProviderReadError::InvalidResponse)
        ));
        let redirect = http
            .get(format!("http://{address}/redirect"))
            .send()
            .await
            .unwrap();
        assert_eq!(redirect.status(), StatusCode::FOUND);
        assert!(matches!(
            parse_response(redirect, 64).await,
            Err(ProviderReadError::Unavailable)
        ));
        task.abort();
    }

    #[tokio::test]
    async fn google_mail_read_uses_only_the_linked_agent_credential_and_inbox() {
        vak_config::paths::isolate_home_for_tests();
        let agent_id = format!("mailcal-read-{}", Uuid::now_v7());
        let account_id = Uuid::now_v7().to_string();
        let vault = AccountVault::for_agent(&agent_id).unwrap();
        vault
            .store(
                &account_id,
                AccountSecretMaterial::new(
                    "google:subject".into(),
                    Some("owner@example.test".into()),
                    Some("client".into()),
                    Some("token-secret".into()),
                    Some("refresh-secret".into()),
                    None,
                    None,
                )
                .unwrap(),
            )
            .unwrap();

        let message_reads = Arc::new(AtomicUsize::new(0));
        let message_reads_for_handler = message_reads.clone();
        let app = axum::Router::new()
            .route("/gmail/v1/users/me/messages", axum::routing::get(|| async {
                axum::Json(json!({"messages":[{"id":"message-1","threadId":"thread-1"}]}))
            }))
            .route("/gmail/v1/users/me/messages/message-1", axum::routing::get(move || {
                let reads = message_reads_for_handler.clone();
                async move {
                reads.fetch_add(1, Ordering::SeqCst);
                let encoded = base64::engine::general_purpose::URL_SAFE_NO_PAD.encode("hello from inbox");
                axum::Json(json!({"id":"message-1","threadId":"thread-1","snippet":"hello","internalDate":"1790784000000","payload":{"mimeType":"text/plain","headers":[{"name":"Subject","value":"Hello"},{"name":"From","value":"sender@example.test"}],"body":{"data":encoded}}}))
            }}));
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let task = tokio::spawn(async move {
            axum::serve(listener, app).await.unwrap();
        });
        let client = ProviderReadClient {
            http: reqwest::Client::builder()
                .no_proxy()
                .redirect(reqwest::redirect::Policy::none())
                .build()
                .unwrap(),
            google_gmail_base: format!("http://{address}/gmail/v1"),
            google_calendar_base: format!("http://{address}/calendar/v3"),
            microsoft_graph_base: format!("http://{address}/graph/v1.0"),
        };
        let account = ConnectedAccount {
            id: account_id.clone(),
            provider: Provider::Google,
            status: AccountStatus::Connected,
            owner_agent_id: agent_id.clone(),
            allowed_audiences: [format!("agent:{agent_id}")]
                .into_iter()
                .collect::<BTreeSet<_>>(),
            capabilities: [Capability::MailRead].into_iter().collect(),
            provider_scopes: BTreeSet::new(),
            credential_ref: AccountVault::credential_ref(&account_id).unwrap(),
            principal_ref: "opaque".into(),
            revision: 1,
            connected_at: Utc::now(),
            access_token_expires_at: None,
            refresh_token_available: true,
            revoked_at: None,
        };

        let messages = client
            .recent_mail(&account, &vault, &agent_id, &format!("agent:{agent_id}"), 1)
            .await
            .unwrap();
        assert_eq!(messages.len(), 1);
        assert_eq!(messages[0].subject, "Hello");
        assert_eq!(messages[0].body_text.as_deref(), Some("hello from inbox"));
        assert_eq!(message_reads.load(Ordering::SeqCst), 1);
        let ids = client
            .recent_mail_ids(&account, &vault, &agent_id, &format!("agent:{agent_id}"), 1)
            .await
            .unwrap();
        assert_eq!(ids, ["message-1"]);
        assert_eq!(message_reads.load(Ordering::SeqCst), 1);
        assert!(matches!(
            client
                .recent_mail(&account, &vault, "another-agent", "agent:elsewhere", 1)
                .await,
            Err(ProviderReadError::NotAdmitted)
        ));
        assert!(matches!(
            client
                .calendar_events(
                    &account,
                    &vault,
                    &agent_id,
                    &format!("agent:{agent_id}"),
                    CalendarRange {
                        from: Utc::now(),
                        to: Utc::now() + Duration::days(1),
                        limit: 1,
                    },
                )
                .await,
            Err(ProviderReadError::NotAdmitted)
        ));
        vault.remove(&account_id).unwrap();
        task.abort();
    }
}
