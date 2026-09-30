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
use std::{
    io,
    pin::Pin,
    task::{Context, Poll},
};
use tokio::io::{AsyncRead, AsyncWrite, ReadBuf};

const MAX_MAIL_ITEMS: usize = 20;
const MAX_WATCH_SCAN_ITEMS: usize = crate::MAX_ROUTINE_MAIL_BACKLOG;
const MAX_EVENT_ITEMS: usize = 100;
const MAX_RESPONSE_BYTES: usize = 512 * 1024;
const MAX_SELECTED_MESSAGE_RESPONSE_BYTES: usize = 256 * 1024;
const MAX_MESSAGE_BYTES: usize = 128 * 1024;
const MAX_TEXT_BYTES: usize = 16 * 1024;
const ICLOUD_IMAP_HOST: &str = "imap.mail.me.com";
const ICLOUD_IMAP_PORT: u16 = 993;
const ICLOUD_CALDAV_URL: &str = "https://caldav.icloud.com/.well-known/caldav";
const ICLOUD_CALDAV_ORIGIN: &str = "https://caldav.icloud.com";
const MAX_CALDAV_RESPONSE_BYTES: usize = 256 * 1024;
const MAX_IMAP_SESSION_BYTES: usize = 512 * 1024;
type IcloudSession =
    async_imap::Session<BudgetIo<async_native_tls::TlsStream<tokio::net::TcpStream>>>;

/// A per-connection byte budget around the IMAP parser. The provider library
/// does not expose a response-size limit; stopping the underlying stream keeps
/// hostile literals and oversized protocol responses from growing without a
/// bound before our own item limits can run.
#[derive(Debug)]
struct BudgetIo<T> {
    inner: T,
    read_remaining: usize,
    write_remaining: usize,
}

impl<T> BudgetIo<T> {
    fn new(inner: T, read_limit: usize, write_limit: usize) -> Self {
        Self {
            inner,
            read_remaining: read_limit,
            write_remaining: write_limit,
        }
    }
}

impl<T: AsyncRead + Unpin> AsyncRead for BudgetIo<T> {
    fn poll_read(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        output: &mut ReadBuf<'_>,
    ) -> Poll<io::Result<()>> {
        let this = self.get_mut();
        if this.read_remaining == 0 {
            return Poll::Ready(Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "IMAP read budget exceeded",
            )));
        }
        let allowed = this.read_remaining.min(output.remaining()).min(8192);
        if allowed == 0 {
            return Poll::Ready(Ok(()));
        }
        let mut chunk = [0_u8; 8192];
        let mut bounded = ReadBuf::new(&mut chunk[..allowed]);
        match Pin::new(&mut this.inner).poll_read(cx, &mut bounded) {
            Poll::Ready(Ok(())) => {
                let filled = bounded.filled();
                output.put_slice(filled);
                this.read_remaining -= filled.len();
                Poll::Ready(Ok(()))
            }
            other => other,
        }
    }
}

impl<T: AsyncWrite + Unpin> AsyncWrite for BudgetIo<T> {
    fn poll_write(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        input: &[u8],
    ) -> Poll<io::Result<usize>> {
        let this = self.get_mut();
        if this.write_remaining == 0 {
            return Poll::Ready(Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "IMAP write budget exceeded",
            )));
        }
        let allowed = input.len().min(this.write_remaining);
        match Pin::new(&mut this.inner).poll_write(cx, &input[..allowed]) {
            Poll::Ready(Ok(written)) => {
                this.write_remaining -= written;
                Poll::Ready(Ok(written))
            }
            other => other,
        }
    }

    fn poll_flush(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        Pin::new(&mut self.get_mut().inner).poll_flush(cx)
    }

    fn poll_shutdown(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        Pin::new(&mut self.get_mut().inner).poll_shutdown(cx)
    }
}

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
    #[error(
        "the provider watch cursor expired or reset; delete and recreate the routine to establish a new cursor, which may leave a gap"
    )]
    WatchCursorReset,
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

/// An Apple CalDAV URL validated to remain on the fixed HTTPS service host.
/// Construct paths from provider-returned hrefs with `from_href`; arbitrary
/// hosts and URL components fail closed before the credential is loaded.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IcloudCalDavPath(url::Url);

impl IcloudCalDavPath {
    pub fn well_known() -> Result<Self, ProviderReadError> {
        Self::validate(
            url::Url::parse(ICLOUD_CALDAV_URL).map_err(|_| ProviderReadError::Unavailable)?,
        )
    }

    pub fn from_href(base: &Self, href: &str) -> Result<Self, ProviderReadError> {
        let url = base
            .0
            .join(href)
            .map_err(|_| ProviderReadError::InvalidResponse)?;
        Self::validate(url)
    }

    fn validate(url: url::Url) -> Result<Self, ProviderReadError> {
        if url.scheme() != "https"
            || url.host_str() != Some("caldav.icloud.com")
            || url.port().is_some_and(|port| port != 443)
            || !url.username().is_empty()
            || url.password().is_some()
            || url.query().is_some()
            || url.fragment().is_some()
        {
            return Err(ProviderReadError::InvalidResponse);
        }
        Ok(Self(url))
    }
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
        if account.provider == Provider::AppleIcloud {
            return icloud_recent_mail(account, vault, limit).await;
        }
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
        if account.provider == Provider::AppleIcloud {
            return icloud_recent_mail_ids(account, vault, limit.clamp(1, MAX_WATCH_SCAN_ITEMS))
                .await;
        }
        let token = vault
            .access_token(&account.id)
            .map_err(|_| ProviderReadError::Vault)?;
        let limit = limit.clamp(1, MAX_WATCH_SCAN_ITEMS).to_string();
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
            // Gmail omits `messages` when the inbox has no matching items.
            Provider::Google => value
                .get("messages")
                .and_then(Value::as_array)
                .cloned()
                .unwrap_or_default(),
            Provider::Microsoft => value
                .get("value")
                .and_then(Value::as_array)
                .cloned()
                .ok_or(ProviderReadError::InvalidResponse)?,
            Provider::AppleIcloud => return Err(ProviderReadError::Unsupported),
        };
        Ok(items
            .iter()
            .take(limit.parse().unwrap_or(1))
            .filter_map(|item| item.get("id").and_then(Value::as_str))
            .filter(|id| !id.is_empty() && id.len() <= 512 && !id.chars().any(char::is_control))
            .map(str::to_owned)
            .collect())
    }

    /// Return one bounded watch page and its continuation position. Apple
    /// advances by UIDVALIDITY/UIDNEXT; providers without a native page
    /// adapter retain the bounded recent-window behavior for now.
    pub async fn mail_watch_page(
        &self,
        account: &ConnectedAccount,
        vault: &AccountVault,
        agent_id: &str,
        audience: &str,
        cursor: Option<&str>,
        limit: usize,
    ) -> Result<(Vec<String>, Option<String>), ProviderReadError> {
        admit(account, vault, agent_id, audience, Capability::MailRead)?;
        let limit = limit.clamp(1, MAX_WATCH_SCAN_ITEMS);
        if account.provider == Provider::AppleIcloud {
            return icloud_mail_watch_page(account, vault, cursor, limit).await;
        }
        if account.provider == Provider::Google {
            let token = vault
                .access_token(&account.id)
                .map_err(|_| ProviderReadError::Vault)?;
            return self
                .google_mail_watch_page(
                    token.as_str(),
                    account,
                    vault,
                    agent_id,
                    audience,
                    cursor,
                    limit,
                )
                .await;
        }
        self.recent_mail_ids(account, vault, agent_id, audience, limit)
            .await
            .map(|ids| (ids, None))
    }

    async fn google_mail_watch_page(
        &self,
        token: &str,
        account: &ConnectedAccount,
        vault: &AccountVault,
        agent_id: &str,
        audience: &str,
        cursor: Option<&str>,
        limit: usize,
    ) -> Result<(Vec<String>, Option<String>), ProviderReadError> {
        let Some(cursor) = cursor else {
            let profile = self
                .http
                .get(format!("{}/users/me/profile", self.google_gmail_base))
                .bearer_auth(token)
                .send()
                .await
                .map_err(|_| ProviderReadError::Unavailable)?;
            let profile = parse_response(profile, MAX_RESPONSE_BYTES).await?;
            let history_id = profile
                .get("historyId")
                .and_then(Value::as_str)
                .filter(|id| valid_google_history_id(id))
                .ok_or(ProviderReadError::InvalidResponse)?;
            // Take an initial bounded snapshot, then use the profile's history
            // ID to catch arrivals racing that snapshot on the next poll.
            let ids = self
                .recent_mail_ids(account, vault, agent_id, audience, limit)
                .await?;
            return Ok((
                ids,
                Some(encode_google_watch_cursor(&GoogleWatchCursor {
                    start_history_id: history_id.to_owned(),
                    page_token: None,
                })?),
            ));
        };
        let cursor = decode_google_watch_cursor(cursor)?;
        let mut request = self
            .http
            .get(format!("{}/users/me/history", self.google_gmail_base))
            .bearer_auth(token)
            .query(&[
                ("startHistoryId", cursor.start_history_id.as_str()),
                ("labelId", "INBOX"),
                ("maxResults", "100"),
            ])
            .query(&[
                ("historyTypes", "messageAdded"),
                ("historyTypes", "labelAdded"),
            ]);
        if let Some(page_token) = cursor.page_token.as_deref() {
            request = request.query(&[("pageToken", page_token)]);
        }
        let response = request
            .send()
            .await
            .map_err(|_| ProviderReadError::Unavailable)?;
        if response.status() == StatusCode::NOT_FOUND {
            return Err(ProviderReadError::WatchCursorReset);
        }
        let value = parse_response(response, MAX_RESPONSE_BYTES).await?;
        let ids = google_history_inbox_ids(&value, limit)?;
        let next_cursor =
            if let Some(page_token) = value.get("nextPageToken").and_then(Value::as_str) {
                if page_token.is_empty()
                    || page_token.len() > 1024
                    || page_token.chars().any(char::is_control)
                {
                    return Err(ProviderReadError::WatchCursorReset);
                }
                Some(encode_google_watch_cursor(&GoogleWatchCursor {
                    start_history_id: cursor.start_history_id,
                    page_token: Some(page_token.to_owned()),
                })?)
            } else {
                let history_id = value
                    .get("historyId")
                    .and_then(Value::as_str)
                    .filter(|id| valid_google_history_id(id))
                    .ok_or(ProviderReadError::InvalidResponse)?;
                Some(encode_google_watch_cursor(&GoogleWatchCursor {
                    start_history_id: history_id.to_owned(),
                    page_token: None,
                })?)
            };
        Ok((ids, next_cursor))
    }

    /// Fetch a small explicit set of message IDs. Scheduled watches use this
    /// to drain their encrypted backlog instead of repeatedly reading only
    /// the newest provider page.
    pub async fn mail_by_ids(
        &self,
        account: &ConnectedAccount,
        vault: &AccountVault,
        agent_id: &str,
        audience: &str,
        item_ids: &[String],
    ) -> Result<Vec<MailItem>, ProviderReadError> {
        admit(account, vault, agent_id, audience, Capability::MailRead)?;
        if item_ids.is_empty()
            || item_ids.len() > MAX_MAIL_ITEMS
            || item_ids
                .iter()
                .any(|id| id.is_empty() || id.len() > 512 || id.chars().any(char::is_control))
        {
            return Err(ProviderReadError::InvalidResponse);
        }
        if account.provider == Provider::AppleIcloud {
            return icloud_mail_by_ids(account, vault, item_ids).await;
        }
        let token = vault
            .access_token(&account.id)
            .map_err(|_| ProviderReadError::Vault)?;
        let mut output = Vec::with_capacity(item_ids.len());
        for id in item_ids {
            let (base, segments) = match account.provider {
                Provider::Google => (
                    self.google_gmail_base.as_str(),
                    vec!["users", "me", "messages", id.as_str()],
                ),
                Provider::Microsoft => (
                    self.microsoft_graph_base.as_str(),
                    vec!["me", "messages", id.as_str()],
                ),
                Provider::AppleIcloud => return Err(ProviderReadError::Unsupported),
            };
            let mut url = url::Url::parse(base).map_err(|_| ProviderReadError::Unavailable)?;
            url.path_segments_mut()
                .map_err(|_| ProviderReadError::Unavailable)?
                .extend(segments);
            let mut request = self.http.get(url).bearer_auth(token.as_str());
            match account.provider {
                Provider::Google => {
                    request = request.query(&[("format", "full")]);
                }
                Provider::Microsoft => {
                    request = request
                        .header("Prefer", "outlook.body-content-type=\"text\"")
                        .query(&[(
                            "$select",
                            "id,conversationId,from,subject,receivedDateTime,bodyPreview,body,hasAttachments",
                        )]);
                }
                Provider::AppleIcloud => return Err(ProviderReadError::Unsupported),
            }
            let response = request
                .send()
                .await
                .map_err(|_| ProviderReadError::Unavailable)?;
            let value = parse_response(response, MAX_SELECTED_MESSAGE_RESPONSE_BYTES).await?;
            let item = match account.provider {
                Provider::Google => parse_google_message(&value),
                Provider::Microsoft => {
                    parse_graph_message(&value).ok_or(ProviderReadError::InvalidResponse)?
                }
                Provider::AppleIcloud => return Err(ProviderReadError::Unsupported),
            };
            if item.provider_id != *id {
                return Err(ProviderReadError::InvalidResponse);
            }
            output.push(item);
        }
        Ok(output)
    }

    /// Fetch one selected Apple inbox message without setting `Seen`. Raw MIME
    /// is returned only to the Core layer for worker-isolated parsing.
    pub async fn icloud_message_mime(
        &self,
        account: &ConnectedAccount,
        vault: &AccountVault,
        agent_id: &str,
        audience: &str,
        provider_id: &str,
    ) -> Result<Vec<u8>, ProviderReadError> {
        admit(account, vault, agent_id, audience, Capability::MailRead)?;
        if account.provider != Provider::AppleIcloud {
            return Err(ProviderReadError::Unsupported);
        }
        icloud_message_mime(account, vault, provider_id).await
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

    pub async fn icloud_caldav_current_principal_xml(
        &self,
        account: &ConnectedAccount,
        vault: &AccountVault,
        agent_id: &str,
        audience: &str,
    ) -> Result<String, ProviderReadError> {
        self.icloud_caldav_propfind(
            account,
            vault,
            agent_id,
            audience,
            &IcloudCalDavPath::well_known()?,
            "0",
            "<d:propfind xmlns:d=\"DAV:\"><d:prop><d:current-user-principal/></d:prop></d:propfind>",
        )
        .await
    }

    pub async fn icloud_caldav_calendar_home_xml(
        &self,
        account: &ConnectedAccount,
        vault: &AccountVault,
        agent_id: &str,
        audience: &str,
        principal: &IcloudCalDavPath,
    ) -> Result<String, ProviderReadError> {
        self.icloud_caldav_propfind(
            account,
            vault,
            agent_id,
            audience,
            principal,
            "0",
            "<d:propfind xmlns:d=\"DAV:\" xmlns:c=\"urn:ietf:params:xml:ns:caldav\"><d:prop><c:calendar-home-set/></d:prop></d:propfind>",
        )
        .await
    }

    pub async fn icloud_caldav_calendar_collections_xml(
        &self,
        account: &ConnectedAccount,
        vault: &AccountVault,
        agent_id: &str,
        audience: &str,
        calendar_home: &IcloudCalDavPath,
    ) -> Result<String, ProviderReadError> {
        self.icloud_caldav_propfind(
            account,
            vault,
            agent_id,
            audience,
            calendar_home,
            "1",
            "<d:propfind xmlns:d=\"DAV:\" xmlns:c=\"urn:ietf:params:xml:ns:caldav\"><d:prop><d:displayname/><d:resourcetype/></d:prop></d:propfind>",
        )
        .await
    }

    pub async fn icloud_caldav_calendar_query_xml(
        &self,
        account: &ConnectedAccount,
        vault: &AccountVault,
        agent_id: &str,
        audience: &str,
        calendar: &IcloudCalDavPath,
        range: CalendarRange,
    ) -> Result<String, ProviderReadError> {
        admit(account, vault, agent_id, audience, Capability::CalendarRead)?;
        if account.provider != Provider::AppleIcloud {
            return Err(ProviderReadError::Unsupported);
        }
        validate_range(range.from, range.to)?;
        let (login, password) = vault
            .icloud_imap_credentials(&account.id)
            .map_err(|_| ProviderReadError::Vault)?;
        let body = format!(
            "<c:calendar-query xmlns:d=\"DAV:\" xmlns:c=\"urn:ietf:params:xml:ns:caldav\"><c:filter><c:comp-filter name=\"VCALENDAR\"><c:comp-filter name=\"VEVENT\"><c:time-range start=\"{}\" end=\"{}\"/></c:comp-filter></c:comp-filter></c:filter><d:prop><d:getetag/><c:calendar-data/></d:prop></c:calendar-query>",
            range.from.format("%Y%m%dT%H%M%SZ"),
            range.to.format("%Y%m%dT%H%M%SZ"),
        );
        self.icloud_caldav_request(
            "REPORT",
            calendar,
            Some("0"),
            &body,
            login.as_str(),
            password.as_str(),
        )
        .await
    }

    async fn icloud_caldav_propfind(
        &self,
        account: &ConnectedAccount,
        vault: &AccountVault,
        agent_id: &str,
        audience: &str,
        path: &IcloudCalDavPath,
        depth: &str,
        body: &str,
    ) -> Result<String, ProviderReadError> {
        admit(account, vault, agent_id, audience, Capability::CalendarRead)?;
        if account.provider != Provider::AppleIcloud {
            return Err(ProviderReadError::Unsupported);
        }
        let (login, password) = vault
            .icloud_imap_credentials(&account.id)
            .map_err(|_| ProviderReadError::Vault)?;
        self.icloud_caldav_request(
            "PROPFIND",
            path,
            Some(depth),
            body,
            login.as_str(),
            password.as_str(),
        )
        .await
    }

    async fn icloud_caldav_request(
        &self,
        method: &str,
        path: &IcloudCalDavPath,
        depth: Option<&str>,
        body: &str,
        login: &str,
        password: &str,
    ) -> Result<String, ProviderReadError> {
        if path.0.origin().ascii_serialization() != ICLOUD_CALDAV_ORIGIN {
            return Err(ProviderReadError::InvalidResponse);
        }
        let method = reqwest::Method::from_bytes(method.as_bytes())
            .map_err(|_| ProviderReadError::Unsupported)?;
        let mut request = self
            .http
            .request(method, path.0.clone())
            .basic_auth(login, Some(password))
            .header(
                reqwest::header::CONTENT_TYPE,
                "application/xml; charset=utf-8",
            )
            .body(body.to_owned());
        if let Some(depth) = depth {
            request = request.header("Depth", depth);
        }
        let response = request
            .send()
            .await
            .map_err(|_| ProviderReadError::Unavailable)?;
        read_caldav_multistatus(response).await
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

/// Verify an iCloud app-specific password against the fixed TLS IMAP endpoint.
/// This proves only Mail access; it does not verify or authorize Calendar.
pub async fn verify_icloud_mail_credentials(
    login: &str,
    password: &str,
) -> Result<(), ProviderReadError> {
    let mut session = icloud_imap_session(login, password).await?;
    let result = tokio::time::timeout(StdDuration::from_secs(10), session.examine("INBOX"))
        .await
        .map_err(|_| ProviderReadError::Unavailable)?
        .map_err(map_imap_error)
        .map(|_| ());
    let _ = tokio::time::timeout(StdDuration::from_secs(2), session.logout()).await;
    result
}

/// Verify an iCloud app-specific password against Apple's fixed CalDAV
/// endpoint without following redirects or parsing provider-controlled XML.
/// This is only an authentication probe: callers must not mark calendar
/// capabilities usable until discovery, bounded event reads, and worker-side
/// iCalendar parsing are implemented.
pub async fn verify_icloud_calendar_credentials(
    login: &str,
    password: &str,
) -> Result<(), ProviderReadError> {
    let client = reqwest::Client::builder()
        .no_proxy()
        .redirect(reqwest::redirect::Policy::none())
        .timeout(StdDuration::from_secs(15))
        .build()
        .map_err(|_| ProviderReadError::Unavailable)?;
    verify_icloud_calendar_credentials_at(&client, ICLOUD_CALDAV_URL, login, password).await
}

async fn verify_icloud_calendar_credentials_at(
    client: &reqwest::Client,
    endpoint: &str,
    login: &str,
    password: &str,
) -> Result<(), ProviderReadError> {
    let method =
        reqwest::Method::from_bytes(b"PROPFIND").map_err(|_| ProviderReadError::Unavailable)?;
    let response = client
        .request(method, endpoint)
        .basic_auth(login, Some(password))
        .header("Depth", "0")
        .header(reqwest::header::CONTENT_TYPE, "application/xml; charset=utf-8")
        .body("<?xml version=\"1.0\"?><d:propfind xmlns:d=\"DAV:\"><d:prop><d:current-user-principal/></d:prop></d:propfind>")
        .send()
        .await
        .map_err(|_| ProviderReadError::Unavailable)?;
    read_caldav_multistatus(response).await.map(|_| ())
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
    Ok(())
}

async fn icloud_recent_mail(
    account: &ConnectedAccount,
    vault: &AccountVault,
    limit: usize,
) -> Result<Vec<MailItem>, ProviderReadError> {
    let (login, password) = vault
        .icloud_imap_credentials(&account.id)
        .map_err(|_| ProviderReadError::Vault)?;
    let mut session = icloud_imap_session(login.as_str(), password.as_str()).await?;
    let result = icloud_fetch_recent_mail(&mut session, limit).await;
    let _ = tokio::time::timeout(StdDuration::from_secs(2), session.logout()).await;
    result
}

async fn icloud_fetch_recent_mail<T>(
    session: &mut async_imap::Session<BudgetIo<T>>,
    limit: usize,
) -> Result<Vec<MailItem>, ProviderReadError>
where
    T: AsyncRead + AsyncWrite + Unpin + std::fmt::Debug + Send,
{
    let mailbox = tokio::time::timeout(StdDuration::from_secs(10), session.examine("INBOX"))
        .await
        .map_err(|_| ProviderReadError::Unavailable)?
        .map_err(map_imap_error)?;
    if mailbox.exists == 0 {
        return Ok(Vec::new());
    }
    let uid_validity = mailbox
        .uid_validity
        .ok_or(ProviderReadError::InvalidResponse)?;
    let first = mailbox
        .exists
        .saturating_sub(limit.min(u32::MAX as usize) as u32 - 1)
        .max(1);
    let mut fetched = tokio::time::timeout(
        StdDuration::from_secs(10),
        session.fetch(
            format!("{first}:{}", mailbox.exists),
            "UID ENVELOPE INTERNALDATE RFC822.SIZE BODYSTRUCTURE",
        ),
    )
    .await
    .map_err(|_| ProviderReadError::Unavailable)?
    .map_err(map_imap_error)?;
    use futures::TryStreamExt;
    let mut output = Vec::with_capacity(limit.min(MAX_MAIL_ITEMS));
    while let Some(message) = tokio::time::timeout(StdDuration::from_secs(10), fetched.try_next())
        .await
        .map_err(|_| ProviderReadError::Unavailable)?
        .map_err(map_imap_error)?
    {
        let Some(uid) = message.uid else {
            return Err(ProviderReadError::InvalidResponse);
        };
        let Some(envelope) = message.envelope() else {
            continue;
        };
        let body_structure = message
            .bodystructure()
            .ok_or(ProviderReadError::InvalidResponse)?;
        let subject = envelope
            .subject
            .as_deref()
            .map(|value| bounded_mail_text(value, 512))
            .filter(|value| !value.is_empty())
            .unwrap_or_else(|| "(no subject)".into());
        let from = envelope
            .from
            .as_ref()
            .and_then(|addresses| addresses.first())
            .and_then(|address| {
                let mailbox = address.mailbox.as_deref()?;
                let host = address.host.as_deref()?;
                let address = format!(
                    "{}@{}",
                    bounded_mail_text(mailbox, 256),
                    bounded_mail_text(host, 256)
                );
                (!address.starts_with('@') && !address.ends_with('@') && address.len() <= 512)
                    .then_some(address)
            });
        let received_at = message.internal_date().map(|date| date.with_timezone(&Utc));
        let has_attachments = imap_body_has_attachments(body_structure);
        output.push(MailItem {
            provider_id: format!("{uid_validity}:{uid}"),
            thread_id: None,
            from,
            preview: subject.clone(),
            subject,
            received_at,
            body_text: None,
            has_attachments,
        });
    }
    output.sort_by(|left, right| right.received_at.cmp(&left.received_at));
    output.truncate(limit.min(MAX_MAIL_ITEMS));
    Ok(output)
}

async fn icloud_recent_mail_ids(
    account: &ConnectedAccount,
    vault: &AccountVault,
    limit: usize,
) -> Result<Vec<String>, ProviderReadError> {
    let (login, password) = vault
        .icloud_imap_credentials(&account.id)
        .map_err(|_| ProviderReadError::Vault)?;
    let mut session = icloud_imap_session(login.as_str(), password.as_str()).await?;
    let result = async {
        let mailbox = tokio::time::timeout(StdDuration::from_secs(10), session.examine("INBOX"))
            .await
            .map_err(|_| ProviderReadError::Unavailable)?
            .map_err(map_imap_error)?;
        if mailbox.exists == 0 {
            return Ok(Vec::new());
        }
        let uid_validity = mailbox
            .uid_validity
            .ok_or(ProviderReadError::InvalidResponse)?;
        let first = mailbox
            .exists
            .saturating_sub(limit.min(u32::MAX as usize) as u32 - 1)
            .max(1);
        let mut fetched = tokio::time::timeout(
            StdDuration::from_secs(10),
            session.fetch(format!("{first}:{}", mailbox.exists), "UID"),
        )
        .await
        .map_err(|_| ProviderReadError::Unavailable)?
        .map_err(map_imap_error)?;
        use futures::TryStreamExt;
        let mut ids = Vec::with_capacity(limit.min(MAX_MAIL_ITEMS));
        while let Some(message) =
            tokio::time::timeout(StdDuration::from_secs(10), fetched.try_next())
                .await
                .map_err(|_| ProviderReadError::Unavailable)?
                .map_err(map_imap_error)?
        {
            if let Some(uid) = message.uid {
                ids.push(format!("{uid_validity}:{uid}"));
            } else {
                return Err(ProviderReadError::InvalidResponse);
            }
        }
        Ok(ids)
    }
    .await;
    let _ = tokio::time::timeout(StdDuration::from_secs(2), session.logout()).await;
    result
}

async fn icloud_mail_watch_page(
    account: &ConnectedAccount,
    vault: &AccountVault,
    cursor: Option<&str>,
    limit: usize,
) -> Result<(Vec<String>, Option<String>), ProviderReadError> {
    let (login, password) = vault
        .icloud_imap_credentials(&account.id)
        .map_err(|_| ProviderReadError::Vault)?;
    let mut session = icloud_imap_session(login.as_str(), password.as_str()).await?;
    let result = async {
        let mailbox = tokio::time::timeout(StdDuration::from_secs(10), session.examine("INBOX"))
            .await
            .map_err(|_| ProviderReadError::Unavailable)?
            .map_err(map_imap_error)?;
        icloud_watch_ids_in_mailbox(&mut session, &mailbox, cursor, limit).await
    }
    .await;
    let _ = tokio::time::timeout(StdDuration::from_secs(2), session.logout()).await;
    result
}

async fn icloud_watch_ids_in_mailbox<T>(
    session: &mut async_imap::Session<BudgetIo<T>>,
    mailbox: &async_imap::types::Mailbox,
    cursor: Option<&str>,
    limit: usize,
) -> Result<(Vec<String>, Option<String>), ProviderReadError>
where
    T: AsyncRead + AsyncWrite + Unpin + std::fmt::Debug + Send,
{
    let uid_validity = mailbox
        .uid_validity
        .ok_or(ProviderReadError::InvalidResponse)?;
    let uid_next = mailbox.uid_next.ok_or(ProviderReadError::InvalidResponse)?;
    let highest_uid = uid_next.saturating_sub(1);
    let (first_uid, end_uid) = icloud_watch_uid_window(cursor, uid_validity, highest_uid, limit)?;
    let mut ids = Vec::new();
    if mailbox.exists > 0 && first_uid <= end_uid {
        let query = format!("UID {first_uid}:{end_uid}");
        let found = tokio::time::timeout(StdDuration::from_secs(10), session.uid_search(query))
            .await
            .map_err(|_| ProviderReadError::Unavailable)?
            .map_err(map_imap_error)?;
        if found.len() > limit {
            return Err(ProviderReadError::InvalidResponse);
        }
        let mut found = found.into_iter().collect::<Vec<_>>();
        found.sort_unstable();
        ids.extend(found.into_iter().map(|uid| format!("{uid_validity}:{uid}")));
    }
    let next_cursor = format!("apple-imap:{uid_validity}:{end_uid}");
    Ok((ids, Some(next_cursor)))
}

fn parse_icloud_watch_cursor(cursor: &str) -> Result<(u32, u32), ProviderReadError> {
    let mut parts = cursor.split(':');
    if parts.next() != Some("apple-imap") {
        return Err(ProviderReadError::InvalidResponse);
    }
    let validity = parts
        .next()
        .and_then(|part| part.parse::<u32>().ok())
        .filter(|value| *value > 0)
        .ok_or(ProviderReadError::InvalidResponse)?;
    let uid = parts
        .next()
        .and_then(|part| part.parse::<u32>().ok())
        .ok_or(ProviderReadError::InvalidResponse)?;
    if parts.next().is_some() {
        return Err(ProviderReadError::InvalidResponse);
    }
    Ok((validity, uid))
}

fn icloud_watch_uid_window(
    cursor: Option<&str>,
    uid_validity: u32,
    highest_uid: u32,
    limit: usize,
) -> Result<(u32, u32), ProviderReadError> {
    let width = limit.clamp(1, MAX_WATCH_SCAN_ITEMS).min(u32::MAX as usize) as u32;
    let first = if let Some(cursor) = cursor {
        let (stored_validity, stored_uid) = parse_icloud_watch_cursor(cursor)?;
        if stored_validity != uid_validity {
            // UID values cannot be compared across a UIDVALIDITY change.
            // Fail visibly; silently restarting could replay or skip mail.
            return Err(ProviderReadError::WatchCursorReset);
        }
        stored_uid.saturating_add(1)
    } else {
        highest_uid.saturating_sub(width - 1).max(1)
    };
    let end = if cursor.is_some() {
        highest_uid.min(first.saturating_add(width - 1))
    } else {
        highest_uid
    };
    Ok((first, end))
}

async fn icloud_mail_by_ids(
    account: &ConnectedAccount,
    vault: &AccountVault,
    item_ids: &[String],
) -> Result<Vec<MailItem>, ProviderReadError> {
    let (login, password) = vault
        .icloud_imap_credentials(&account.id)
        .map_err(|_| ProviderReadError::Vault)?;
    let mut session = icloud_imap_session(login.as_str(), password.as_str()).await?;
    let result = icloud_fetch_mail_by_ids(&mut session, item_ids).await;
    let _ = tokio::time::timeout(StdDuration::from_secs(2), session.logout()).await;
    result
}

async fn icloud_fetch_mail_by_ids<T>(
    session: &mut async_imap::Session<BudgetIo<T>>,
    item_ids: &[String],
) -> Result<Vec<MailItem>, ProviderReadError>
where
    T: AsyncRead + AsyncWrite + Unpin + std::fmt::Debug + Send,
{
    if item_ids.is_empty() || item_ids.len() > MAX_MAIL_ITEMS {
        return Err(ProviderReadError::InvalidResponse);
    }
    let parsed = item_ids
        .iter()
        .map(|id| parse_icloud_provider_id(id))
        .collect::<Result<Vec<_>, _>>()?;
    if parsed.iter().any(|(validity, _)| *validity != parsed[0].0) {
        return Err(ProviderReadError::InvalidResponse);
    }
    let uid_set = parsed
        .iter()
        .map(|(_, uid)| uid.to_string())
        .collect::<Vec<_>>()
        .join(",");
    let mailbox = tokio::time::timeout(StdDuration::from_secs(10), session.examine("INBOX"))
        .await
        .map_err(|_| ProviderReadError::Unavailable)?
        .map_err(map_imap_error)?;
    if mailbox.uid_validity != Some(parsed[0].0) {
        return Err(ProviderReadError::WatchCursorReset);
    }
    let mut fetched = tokio::time::timeout(
        StdDuration::from_secs(10),
        session.uid_fetch(
            uid_set,
            "UID ENVELOPE INTERNALDATE RFC822.SIZE BODYSTRUCTURE",
        ),
    )
    .await
    .map_err(|_| ProviderReadError::Unavailable)?
    .map_err(map_imap_error)?;
    use futures::TryStreamExt;
    let mut output = Vec::with_capacity(item_ids.len());
    while let Some(message) = tokio::time::timeout(StdDuration::from_secs(10), fetched.try_next())
        .await
        .map_err(|_| ProviderReadError::Unavailable)?
        .map_err(map_imap_error)?
    {
        let uid = message.uid.ok_or(ProviderReadError::InvalidResponse)?;
        if !parsed.iter().any(|(_, selected)| *selected == uid) {
            return Err(ProviderReadError::InvalidResponse);
        }
        let Some(envelope) = message.envelope() else {
            return Err(ProviderReadError::InvalidResponse);
        };
        let body_structure = message
            .bodystructure()
            .ok_or(ProviderReadError::InvalidResponse)?;
        let subject = envelope
            .subject
            .as_deref()
            .map(|value| bounded_mail_text(value, 512))
            .filter(|value| !value.is_empty())
            .unwrap_or_else(|| "(no subject)".into());
        let from = envelope
            .from
            .as_ref()
            .and_then(|addresses| addresses.first())
            .and_then(|address| {
                let mailbox = address.mailbox.as_deref()?;
                let host = address.host.as_deref()?;
                let address = format!(
                    "{}@{}",
                    bounded_mail_text(mailbox, 256),
                    bounded_mail_text(host, 256)
                );
                (!address.starts_with('@') && !address.ends_with('@') && address.len() <= 512)
                    .then_some(address)
            });
        output.push(MailItem {
            provider_id: format!("{}:{uid}", parsed[0].0),
            thread_id: None,
            from,
            subject: subject.clone(),
            received_at: message.internal_date().map(|date| date.with_timezone(&Utc)),
            preview: subject,
            body_text: None,
            has_attachments: imap_body_has_attachments(body_structure),
        });
    }
    if output.len() != item_ids.len() {
        return Err(ProviderReadError::InvalidResponse);
    }
    Ok(output)
}

async fn icloud_fetch_message_mime<T>(
    session: &mut async_imap::Session<BudgetIo<T>>,
    expected_validity: u32,
    uid: u32,
) -> Result<Vec<u8>, ProviderReadError>
where
    T: AsyncRead + AsyncWrite + Unpin + std::fmt::Debug + Send,
{
    let mailbox = tokio::time::timeout(StdDuration::from_secs(10), session.examine("INBOX"))
        .await
        .map_err(|_| ProviderReadError::Unavailable)?
        .map_err(map_imap_error)?;
    if mailbox.uid_validity != Some(expected_validity) {
        return Err(ProviderReadError::InvalidResponse);
    }
    let mut fetched = tokio::time::timeout(
        StdDuration::from_secs(10),
        session.uid_fetch(uid.to_string(), "UID RFC822.SIZE BODY.PEEK[]"),
    )
    .await
    .map_err(|_| ProviderReadError::Unavailable)?
    .map_err(map_imap_error)?;
    use futures::TryStreamExt;
    let mut raw = None;
    while let Some(message) = tokio::time::timeout(StdDuration::from_secs(10), fetched.try_next())
        .await
        .map_err(|_| ProviderReadError::Unavailable)?
        .map_err(map_imap_error)?
    {
        if message.uid != Some(uid) {
            return Err(ProviderReadError::InvalidResponse);
        }
        let body = message.body().ok_or(ProviderReadError::InvalidResponse)?;
        if body.is_empty() || body.len() > MAX_MESSAGE_BYTES {
            return Err(ProviderReadError::InvalidResponse);
        }
        raw = Some(body.to_vec());
    }
    raw.ok_or(ProviderReadError::InvalidResponse)
}

fn parse_icloud_provider_id(provider_id: &str) -> Result<(u32, u32), ProviderReadError> {
    let (validity, uid) = provider_id
        .split_once(':')
        .ok_or(ProviderReadError::InvalidResponse)?;
    if validity.is_empty()
        || uid.is_empty()
        || !validity.bytes().all(|byte| byte.is_ascii_digit())
        || !uid.bytes().all(|byte| byte.is_ascii_digit())
    {
        return Err(ProviderReadError::InvalidResponse);
    }
    let validity = validity
        .parse::<u32>()
        .map_err(|_| ProviderReadError::InvalidResponse)?;
    let uid = uid
        .parse::<u32>()
        .map_err(|_| ProviderReadError::InvalidResponse)?;
    if validity == 0 || uid == 0 {
        return Err(ProviderReadError::InvalidResponse);
    }
    Ok((validity, uid))
}

async fn icloud_message_mime(
    account: &ConnectedAccount,
    vault: &AccountVault,
    provider_id: &str,
) -> Result<Vec<u8>, ProviderReadError> {
    let (expected_validity, uid) = parse_icloud_provider_id(provider_id)?;
    let (login, password) = vault
        .icloud_imap_credentials(&account.id)
        .map_err(|_| ProviderReadError::Vault)?;
    let mut session = icloud_imap_session(login.as_str(), password.as_str()).await?;
    let result = icloud_fetch_message_mime(&mut session, expected_validity, uid).await;
    let _ = tokio::time::timeout(StdDuration::from_secs(2), session.logout()).await;
    result
}

async fn icloud_imap_session(
    login: &str,
    password: &str,
) -> Result<IcloudSession, ProviderReadError> {
    let tcp = tokio::time::timeout(
        StdDuration::from_secs(10),
        tokio::net::TcpStream::connect((ICLOUD_IMAP_HOST, ICLOUD_IMAP_PORT)),
    )
    .await
    .map_err(|_| ProviderReadError::Unavailable)?
    .map_err(|_| ProviderReadError::Unavailable)?;
    let tls = tokio::time::timeout(
        StdDuration::from_secs(10),
        async_native_tls::connect(ICLOUD_IMAP_HOST, tcp),
    )
    .await
    .map_err(|_| ProviderReadError::Unavailable)?
    .map_err(|_| ProviderReadError::Unavailable)?;
    let client = async_imap::Client::new(BudgetIo::new(tls, MAX_IMAP_SESSION_BYTES, 64 * 1024));
    tokio::time::timeout(StdDuration::from_secs(15), client.login(login, password))
        .await
        .map_err(|_| ProviderReadError::Unavailable)?
        .map_err(|(error, _client)| match error {
            async_imap::error::Error::No(_) => ProviderReadError::ReauthenticationRequired,
            _ => ProviderReadError::Unavailable,
        })
}

fn map_imap_error(error: async_imap::error::Error) -> ProviderReadError {
    match error {
        async_imap::error::Error::No(_) => ProviderReadError::ReauthenticationRequired,
        async_imap::error::Error::Io(ref error) if error.kind() == io::ErrorKind::InvalidData => {
            ProviderReadError::InvalidResponse
        }
        async_imap::error::Error::Parse(_) => ProviderReadError::InvalidResponse,
        _ => ProviderReadError::Unavailable,
    }
}

fn bounded_mail_text(bytes: &[u8], max_bytes: usize) -> String {
    String::from_utf8_lossy(&bytes[..bytes.len().min(max_bytes)])
        .chars()
        .filter(|ch| !ch.is_control() || *ch == ' ')
        .take(max_bytes)
        .collect()
}

fn imap_body_has_attachments(body: &async_imap::imap_proto::types::BodyStructure<'_>) -> bool {
    use async_imap::imap_proto::types::BodyStructure;
    let (common, nested) = match body {
        BodyStructure::Basic { common, .. } | BodyStructure::Text { common, .. } => (common, None),
        BodyStructure::Message { common, body, .. } => {
            (common, Some(std::slice::from_ref(body.as_ref())))
        }
        BodyStructure::Multipart { common, bodies, .. } => (common, Some(bodies.as_slice())),
    };
    let explicit_attachment = common
        .disposition
        .as_ref()
        .is_some_and(|disposition| disposition.ty.eq_ignore_ascii_case("attachment"));
    let named_part = common.ty.params.as_ref().is_some_and(|params| {
        params
            .iter()
            .any(|(name, value)| name.eq_ignore_ascii_case("name") && !value.trim().is_empty())
    }) || common.disposition.as_ref().is_some_and(|disposition| {
        disposition.params.as_ref().is_some_and(|params| {
            params.iter().any(|(name, value)| {
                name.eq_ignore_ascii_case("filename") && !value.trim().is_empty()
            })
        })
    });
    explicit_attachment
        || named_part
        || nested.is_some_and(|parts| parts.iter().any(imap_body_has_attachments))
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

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct GoogleWatchCursor {
    start_history_id: String,
    #[serde(default)]
    page_token: Option<String>,
}

fn valid_google_history_id(id: &str) -> bool {
    !id.is_empty() && id.len() <= 32 && id.bytes().all(|byte| byte.is_ascii_digit())
}

fn encode_google_watch_cursor(cursor: &GoogleWatchCursor) -> Result<String, ProviderReadError> {
    if !valid_google_history_id(&cursor.start_history_id)
        || cursor.page_token.as_ref().is_some_and(|token| {
            token.is_empty() || token.len() > 1024 || token.chars().any(char::is_control)
        })
    {
        return Err(ProviderReadError::WatchCursorReset);
    }
    let encoded = base64::engine::general_purpose::URL_SAFE_NO_PAD
        .encode(serde_json::to_vec(cursor).map_err(|_| ProviderReadError::InvalidResponse)?);
    let value = format!("gmail-v1:{encoded}");
    if value.len() > 2048 {
        return Err(ProviderReadError::WatchCursorReset);
    }
    Ok(value)
}

fn decode_google_watch_cursor(value: &str) -> Result<GoogleWatchCursor, ProviderReadError> {
    let encoded = value
        .strip_prefix("gmail-v1:")
        .filter(|encoded| !encoded.is_empty() && encoded.len() <= 2040)
        .ok_or(ProviderReadError::WatchCursorReset)?;
    let bytes = base64::engine::general_purpose::URL_SAFE_NO_PAD
        .decode(encoded)
        .map_err(|_| ProviderReadError::WatchCursorReset)?;
    let cursor: GoogleWatchCursor =
        serde_json::from_slice(&bytes).map_err(|_| ProviderReadError::WatchCursorReset)?;
    if !valid_google_history_id(&cursor.start_history_id)
        || cursor.page_token.as_ref().is_some_and(|token| {
            token.is_empty() || token.len() > 1024 || token.chars().any(char::is_control)
        })
    {
        return Err(ProviderReadError::WatchCursorReset);
    }
    Ok(cursor)
}

fn google_history_inbox_ids(value: &Value, limit: usize) -> Result<Vec<String>, ProviderReadError> {
    let mut ids = Vec::new();
    let mut seen = std::collections::HashSet::new();
    let history = value
        .get("history")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    for record in history {
        for change_kind in ["messagesAdded", "labelsAdded"] {
            let Some(changes) = record.get(change_kind).and_then(Value::as_array) else {
                continue;
            };
            for change in changes {
                let message = change.get("message").unwrap_or(&Value::Null);
                let in_inbox = message
                    .get("labelIds")
                    .and_then(Value::as_array)
                    .is_some_and(|labels| {
                        labels.iter().any(|label| label.as_str() == Some("INBOX"))
                    })
                    || change
                        .get("labelIds")
                        .and_then(Value::as_array)
                        .is_some_and(|labels| {
                            labels.iter().any(|label| label.as_str() == Some("INBOX"))
                        });
                if !in_inbox {
                    continue;
                }
                let Some(id) = message.get("id").and_then(Value::as_str) else {
                    continue;
                };
                if id.is_empty() || id.len() > 512 || id.chars().any(char::is_control) {
                    return Err(ProviderReadError::InvalidResponse);
                }
                if seen.insert(id.to_owned()) {
                    ids.push(id.to_owned());
                    if ids.len() > limit {
                        return Err(ProviderReadError::WatchCursorReset);
                    }
                }
            }
        }
    }
    Ok(ids)
}

async fn read_caldav_multistatus(mut response: Response) -> Result<String, ProviderReadError> {
    match response.status() {
        StatusCode::MULTI_STATUS => {}
        StatusCode::UNAUTHORIZED | StatusCode::FORBIDDEN => {
            return Err(ProviderReadError::ReauthenticationRequired);
        }
        status if status.is_redirection() => return Err(ProviderReadError::InvalidResponse),
        _ => return Err(ProviderReadError::Unavailable),
    }
    if response
        .content_length()
        .is_some_and(|size| size > MAX_CALDAV_RESPONSE_BYTES as u64)
    {
        return Err(ProviderReadError::InvalidResponse);
    }
    let mut body = Vec::new();
    while let Some(chunk) = response
        .chunk()
        .await
        .map_err(|_| ProviderReadError::Unavailable)?
    {
        if body.len().saturating_add(chunk.len()) > MAX_CALDAV_RESPONSE_BYTES {
            return Err(ProviderReadError::InvalidResponse);
        }
        body.extend_from_slice(&chunk);
    }
    String::from_utf8(body).map_err(|_| ProviderReadError::InvalidResponse)
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
    use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
    use uuid::Uuid;

    #[tokio::test]
    async fn icloud_recent_mail_reads_bounded_metadata_from_a_read_only_mailbox() {
        use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};

        let (client_stream, server_stream) = tokio::io::duplex(8192);
        let server = tokio::spawn(async move {
            let mut stream = BufReader::new(server_stream);
            stream
                .get_mut()
                .write_all(b"* OK iCloud test server\r\n")
                .await
                .unwrap();
            loop {
                let mut line = String::new();
                if stream.read_line(&mut line).await.unwrap_or(0) == 0 {
                    break;
                }
                let tag = line.split_whitespace().next().unwrap_or("A0000").to_owned();
                let command = line.split_whitespace().nth(1).unwrap_or("");
                let response = if line.contains(" LOGIN ") {
                    format!("{tag} OK authenticated\r\n")
                } else if command.eq_ignore_ascii_case("CAPABILITY") {
                    format!("* CAPABILITY IMAP4rev1\r\n{tag} OK capabilities\r\n")
                } else if line.contains(" EXAMINE ") {
                    format!(
                        "* 1 EXISTS\r\n* 0 RECENT\r\n* OK [UIDVALIDITY 7] valid\r\n{tag} OK [READ-ONLY] selected\r\n"
                    )
                } else if line.contains(" FETCH ") {
                    format!(
                        "* 1 FETCH (UID 31 ENVELOPE (NIL \"Hello\" ((NIL NIL \"sender\" \"example.test\")) NIL NIL NIL NIL NIL NIL \"<m31>\") INTERNALDATE \"30-Sep-2026 12:00:00 +0000\" RFC822.SIZE 20 BODYSTRUCTURE (\"TEXT\" \"PLAIN\" (\"CHARSET\" \"UTF-8\") NIL NIL \"7BIT\" 5 1))\r\n{tag} OK fetched\r\n"
                    )
                } else if command.eq_ignore_ascii_case("LOGOUT") {
                    format!("* BYE logging out\r\n{tag} OK logout\r\n")
                } else {
                    format!("{tag} BAD unsupported test command {command}\r\n")
                };
                if stream
                    .get_mut()
                    .write_all(response.as_bytes())
                    .await
                    .is_err()
                {
                    break;
                }
                if command.eq_ignore_ascii_case("LOGOUT") {
                    break;
                }
            }
        });
        let client = async_imap::Client::new(BudgetIo::new(client_stream, 8192, 8192));
        let mut session = client
            .login("owner@icloud.com", "test-secret")
            .await
            .unwrap();
        let messages = icloud_fetch_recent_mail(&mut session, 1).await.unwrap();
        assert_eq!(messages.len(), 1);
        assert_eq!(messages[0].provider_id, "7:31");
        assert_eq!(messages[0].subject, "Hello");
        assert_eq!(messages[0].from.as_deref(), Some("sender@example.test"));
        assert!(!messages[0].has_attachments);
        assert!(messages[0].body_text.is_none());
        session.logout().await.unwrap();
        server.await.unwrap();
    }

    #[tokio::test]
    async fn icloud_watch_fetches_only_selected_uids_without_setting_seen() {
        use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};

        let (client_stream, server_stream) = tokio::io::duplex(8192);
        let server = tokio::spawn(async move {
            let mut stream = BufReader::new(server_stream);
            stream
                .get_mut()
                .write_all(b"* OK iCloud test server\r\n")
                .await
                .unwrap();
            loop {
                let mut line = String::new();
                if stream.read_line(&mut line).await.unwrap_or(0) == 0 {
                    break;
                }
                let tag = line.split_whitespace().next().unwrap_or("A0000").to_owned();
                let command = line.split_whitespace().nth(1).unwrap_or("");
                let response = if line.contains(" LOGIN ") {
                    format!("{tag} OK authenticated\r\n")
                } else if command.eq_ignore_ascii_case("CAPABILITY") {
                    format!("* CAPABILITY IMAP4rev1\r\n{tag} OK capabilities\r\n")
                } else if line.contains(" EXAMINE ") {
                    format!(
                        "* 1 EXISTS\r\n* 0 RECENT\r\n* OK [UIDVALIDITY 7] valid\r\n{tag} OK [READ-ONLY] selected\r\n"
                    )
                } else if line.contains(" UID FETCH ") {
                    assert!(line.contains("UID FETCH 31 "), "{line}");
                    assert!(line.contains("ENVELOPE"), "{line}");
                    assert!(!line.contains("BODY[]"), "{line}");
                    format!(
                        "* 1 FETCH (UID 31 ENVELOPE (NIL \"Selected\" ((NIL NIL \"sender\" \"example.test\")) NIL NIL NIL NIL NIL NIL \"<m31>\") INTERNALDATE \"30-Sep-2026 12:00:00 +0000\" RFC822.SIZE 20 BODYSTRUCTURE (\"TEXT\" \"PLAIN\" (\"CHARSET\" \"UTF-8\") NIL NIL \"7BIT\" 5 1))\r\n{tag} OK fetched\r\n"
                    )
                } else if command.eq_ignore_ascii_case("LOGOUT") {
                    format!("* BYE logging out\r\n{tag} OK logout\r\n")
                } else {
                    format!("{tag} BAD unsupported test command {command}\r\n")
                };
                if stream
                    .get_mut()
                    .write_all(response.as_bytes())
                    .await
                    .is_err()
                {
                    break;
                }
                if command.eq_ignore_ascii_case("LOGOUT") {
                    break;
                }
            }
        });
        let client = async_imap::Client::new(BudgetIo::new(client_stream, 8192, 8192));
        let mut session = client
            .login("owner@icloud.com", "test-secret")
            .await
            .unwrap();
        let items = icloud_fetch_mail_by_ids(&mut session, &["7:31".into()])
            .await
            .unwrap();
        assert_eq!(items.len(), 1);
        assert_eq!(items[0].provider_id, "7:31");
        assert_eq!(items[0].subject, "Selected");
        assert!(items[0].body_text.is_none());
        session.logout().await.unwrap();
        server.await.unwrap();
    }

    #[test]
    fn icloud_watch_cursor_is_uidvalidity_scoped_and_strictly_parsed() {
        assert_eq!(
            parse_icloud_watch_cursor("apple-imap:17:204").unwrap(),
            (17, 204)
        );
        for invalid in [
            "google:17:204",
            "apple-imap:0:204",
            "apple-imap:17:-1",
            "apple-imap:17:204:extra",
            "apple-imap:17:204\n",
        ] {
            assert!(matches!(
                parse_icloud_watch_cursor(invalid),
                Err(ProviderReadError::InvalidResponse)
            ));
        }
        assert_eq!(
            icloud_watch_uid_window(None, 17, 250, 100).unwrap(),
            (151, 250)
        );
        assert_eq!(
            icloud_watch_uid_window(Some("apple-imap:17:250"), 17, 420, 100).unwrap(),
            (251, 350)
        );
        assert_eq!(
            icloud_watch_uid_window(Some("apple-imap:17:350"), 17, 420, 100).unwrap(),
            (351, 420)
        );
        assert!(matches!(
            icloud_watch_uid_window(Some("apple-imap:17:350"), 18, 420, 100),
            Err(ProviderReadError::WatchCursorReset)
        ));
    }

    #[tokio::test]
    async fn icloud_watch_page_queries_only_the_next_bounded_uid_range() {
        use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};

        let (client_stream, server_stream) = tokio::io::duplex(8192);
        let server = tokio::spawn(async move {
            let mut stream = BufReader::new(server_stream);
            stream
                .get_mut()
                .write_all(b"* OK iCloud test server\r\n")
                .await
                .unwrap();
            loop {
                let mut line = String::new();
                if stream.read_line(&mut line).await.unwrap_or(0) == 0 {
                    break;
                }
                let tag = line.split_whitespace().next().unwrap_or("A0000").to_owned();
                let command = line.split_whitespace().nth(1).unwrap_or("");
                let response = if line.contains(" LOGIN ") {
                    format!("{tag} OK authenticated\r\n")
                } else if command.eq_ignore_ascii_case("CAPABILITY") {
                    format!("* CAPABILITY IMAP4rev1\r\n{tag} OK capabilities\r\n")
                } else if line.to_ascii_uppercase().contains("UID SEARCH") {
                    assert!(line.contains("UID SEARCH UID 351:420"), "{line}");
                    format!("* SEARCH 351 355 420\r\n{tag} OK searched\r\n")
                } else if command.eq_ignore_ascii_case("LOGOUT") {
                    format!("* BYE logging out\r\n{tag} OK logout\r\n")
                } else {
                    format!("{tag} BAD unsupported test command {command}\r\n")
                };
                if stream
                    .get_mut()
                    .write_all(response.as_bytes())
                    .await
                    .is_err()
                {
                    break;
                }
                if command.eq_ignore_ascii_case("LOGOUT") {
                    break;
                }
            }
        });
        let client = async_imap::Client::new(BudgetIo::new(client_stream, 8192, 8192));
        let mut session = client
            .login("owner@icloud.com", "test-secret")
            .await
            .unwrap();
        let mailbox = async_imap::types::Mailbox {
            exists: 3,
            uid_validity: Some(7),
            uid_next: Some(421),
            ..Default::default()
        };
        let (ids, cursor) =
            icloud_watch_ids_in_mailbox(&mut session, &mailbox, Some("apple-imap:7:350"), 100)
                .await
                .unwrap();
        assert_eq!(ids, vec!["7:351", "7:355", "7:420"]);
        assert_eq!(cursor.as_deref(), Some("apple-imap:7:420"));
        session.logout().await.unwrap();
        server.await.unwrap();
    }

    #[tokio::test]
    async fn icloud_selected_message_uses_uidvalidity_and_body_peek() {
        use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};

        let (client_stream, server_stream) = tokio::io::duplex(8192);
        let raw = b"Subject: Selected\r\nContent-Type: text/plain\r\n\r\nBody".to_vec();
        let expected_raw = raw.clone();
        let server = tokio::spawn(async move {
            let mut stream = BufReader::new(server_stream);
            stream
                .get_mut()
                .write_all(b"* OK test server\r\n")
                .await
                .unwrap();
            loop {
                let mut line = String::new();
                if stream.read_line(&mut line).await.unwrap_or(0) == 0 {
                    break;
                }
                let tag = line.split_whitespace().next().unwrap_or("A0000").to_owned();
                let upper = line.to_ascii_uppercase();
                let command = if upper.contains(" LOGIN ") {
                    format!("{tag} OK authenticated\r\n")
                } else if upper.contains(" CAPABILITY") {
                    format!("* CAPABILITY IMAP4rev1\r\n{tag} OK capabilities\r\n")
                } else if upper.contains(" EXAMINE ") {
                    format!(
                        "* 1 EXISTS\r\n* OK [UIDVALIDITY 7] valid\r\n{tag} OK [READ-ONLY] selected\r\n"
                    )
                } else if upper.contains(" UID FETCH ") {
                    assert!(upper.contains("BODY.PEEK[]"), "{line}");
                    assert!(!upper.contains("BODY[]"), "{line}");
                    let mut response = format!(
                        "* 1 FETCH (UID 31 RFC822.SIZE {} BODY[] {{{}}}\r\n",
                        raw.len(),
                        raw.len()
                    )
                    .into_bytes();
                    response.extend_from_slice(&raw);
                    response.extend_from_slice(format!(")\r\n{tag} OK fetched\r\n").as_bytes());
                    if stream.get_mut().write_all(&response).await.is_err() {
                        break;
                    }
                    continue;
                } else if upper.contains(" LOGOUT") {
                    format!("* BYE logging out\r\n{tag} OK logout\r\n")
                } else {
                    format!("{tag} BAD unsupported\r\n")
                };
                if stream
                    .get_mut()
                    .write_all(command.as_bytes())
                    .await
                    .is_err()
                {
                    break;
                }
                if upper.contains(" LOGOUT") {
                    break;
                }
            }
        });
        let client = async_imap::Client::new(BudgetIo::new(client_stream, 8192, 8192));
        let mut session = client.login("owner@icloud.com", "secret").await.unwrap();
        let body = icloud_fetch_message_mime(&mut session, 7, 31)
            .await
            .unwrap();
        assert_eq!(body, expected_raw);
        session.logout().await.unwrap();
        server.await.unwrap();
    }

    #[tokio::test]
    async fn icloud_selected_message_refuses_stale_uidvalidity_before_fetch() {
        use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};

        let (client_stream, server_stream) = tokio::io::duplex(2048);
        let server = tokio::spawn(async move {
            let mut stream = BufReader::new(server_stream);
            stream
                .get_mut()
                .write_all(b"* OK test server\r\n")
                .await
                .unwrap();
            loop {
                let mut line = String::new();
                if stream.read_line(&mut line).await.unwrap_or(0) == 0 {
                    break;
                }
                let tag = line.split_whitespace().next().unwrap_or("A0000").to_owned();
                let upper = line.to_ascii_uppercase();
                let response = if upper.contains(" LOGIN ") {
                    format!("{tag} OK authenticated\r\n")
                } else if upper.contains(" CAPABILITY") {
                    format!("* CAPABILITY IMAP4rev1\r\n{tag} OK capabilities\r\n")
                } else if upper.contains(" EXAMINE ") {
                    format!(
                        "* 1 EXISTS\r\n* OK [UIDVALIDITY 8] valid\r\n{tag} OK [READ-ONLY] selected\r\n"
                    )
                } else if upper.contains(" LOGOUT") {
                    format!("* BYE logging out\r\n{tag} OK logout\r\n")
                } else {
                    panic!("stale UIDVALIDITY must prevent a fetch: {line}");
                };
                if stream
                    .get_mut()
                    .write_all(response.as_bytes())
                    .await
                    .is_err()
                {
                    break;
                }
                if upper.contains(" LOGOUT") {
                    break;
                }
            }
        });
        let client = async_imap::Client::new(BudgetIo::new(client_stream, 2048, 2048));
        let mut session = client.login("owner@icloud.com", "secret").await.unwrap();
        assert!(matches!(
            icloud_fetch_message_mime(&mut session, 7, 31).await,
            Err(ProviderReadError::InvalidResponse)
        ));
        session.logout().await.unwrap();
        server.await.unwrap();
    }

    #[tokio::test]
    async fn icloud_imap_transport_stops_at_the_configured_byte_budget() {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};

        let (mut writer, reader) = tokio::io::duplex(32);
        writer.write_all(b"abcdefgh").await.unwrap();
        drop(writer);
        let mut bounded = BudgetIo::new(reader, 4, 16);
        let mut bytes = [0; 8];
        assert_eq!(bounded.read(&mut bytes).await.unwrap(), 4);
        assert_eq!(&bytes[..4], b"abcd");
        assert_eq!(
            bounded.read(&mut bytes).await.unwrap_err().kind(),
            io::ErrorKind::InvalidData
        );

        let (reader, _peer) = tokio::io::duplex(32);
        let mut bounded = BudgetIo::new(reader, 16, 3);
        assert_eq!(bounded.write(b"abcd").await.unwrap(), 3);
        assert_eq!(
            bounded.write(b"e").await.unwrap_err().kind(),
            io::ErrorKind::InvalidData
        );
    }

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
    fn icloud_message_ids_are_uidvalidity_scoped_and_numeric() {
        assert_eq!(parse_icloud_provider_id("1234:56").unwrap(), (1234, 56));
        for invalid in ["", "1234", "1234:0", "0:1", "1:2:3", "1:%2f", "-1:5"] {
            assert!(parse_icloud_provider_id(invalid).is_err(), "{invalid}");
        }
    }

    #[test]
    fn icloud_caldav_href_validation_never_changes_the_authenticated_origin() {
        let base = IcloudCalDavPath::well_known().unwrap();
        let principal = IcloudCalDavPath::from_href(&base, "/principal/owner").unwrap();
        assert_eq!(
            principal.0.origin().ascii_serialization(),
            ICLOUD_CALDAV_ORIGIN
        );
        assert!(IcloudCalDavPath::from_href(&base, "//attacker.invalid/path").is_err());
        assert!(IcloudCalDavPath::from_href(&base, "https://attacker.invalid/path").is_err());
        assert!(IcloudCalDavPath::from_href(&base, "http://caldav.icloud.com/path").is_err());
        assert!(IcloudCalDavPath::from_href(&base, "/principal/owner?redirect=1").is_err());
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
        let app = app
            .route(
                "/multistatus",
                axum::routing::any(|| async {
                    axum::http::Response::builder()
                        .status(StatusCode::MULTI_STATUS)
                        .body(axum::body::Body::from("<d:multistatus/>"))
                        .unwrap()
                }),
            )
            .route(
                "/large-multistatus",
                axum::routing::any(|| async {
                    axum::http::Response::builder()
                        .status(StatusCode::MULTI_STATUS)
                        .body(axum::body::Body::from(vec![
                            b'x';
                            MAX_CALDAV_RESPONSE_BYTES + 1
                        ]))
                        .unwrap()
                }),
            )
            .route(
                "/unauthorized",
                axum::routing::any(|| async { StatusCode::UNAUTHORIZED }),
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
        let multistatus = http
            .request(
                reqwest::Method::from_bytes(b"PROPFIND").unwrap(),
                format!("http://{address}/multistatus"),
            )
            .send()
            .await
            .unwrap();
        assert_eq!(
            read_caldav_multistatus(multistatus).await.unwrap(),
            "<d:multistatus/>"
        );
        let large = http
            .request(
                reqwest::Method::from_bytes(b"PROPFIND").unwrap(),
                format!("http://{address}/large-multistatus"),
            )
            .send()
            .await
            .unwrap();
        assert!(matches!(
            read_caldav_multistatus(large).await,
            Err(ProviderReadError::InvalidResponse)
        ));
        let unauthorized = http
            .request(
                reqwest::Method::from_bytes(b"PROPFIND").unwrap(),
                format!("http://{address}/unauthorized"),
            )
            .send()
            .await
            .unwrap();
        assert!(matches!(
            read_caldav_multistatus(unauthorized).await,
            Err(ProviderReadError::ReauthenticationRequired)
        ));
        task.abort();
    }

    #[tokio::test]
    async fn icloud_caldav_probe_authenticates_only_to_fixed_endpoint_and_rejects_redirects() {
        use axum::http::Request;
        use std::sync::atomic::{AtomicBool, Ordering};

        let saw_authorization = Arc::new(AtomicBool::new(false));
        let captured = Arc::clone(&saw_authorization);
        let app = axum::Router::new()
            .route(
                "/caldav",
                axum::routing::any(move |request: Request<axum::body::Body>| {
                    let captured = Arc::clone(&captured);
                    async move {
                        captured.store(
                            request
                                .headers()
                                .contains_key(reqwest::header::AUTHORIZATION),
                            Ordering::SeqCst,
                        );
                        axum::http::StatusCode::MULTI_STATUS
                    }
                }),
            )
            .route(
                "/redirect",
                axum::routing::any(|| async {
                    axum::http::Response::builder()
                        .status(StatusCode::TEMPORARY_REDIRECT)
                        .header(reqwest::header::LOCATION, "/caldav")
                        .body(axum::body::Body::empty())
                        .unwrap()
                }),
            );
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let task = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
        let client = reqwest::Client::builder()
            .no_proxy()
            .redirect(reqwest::redirect::Policy::none())
            .build()
            .unwrap();

        assert!(
            verify_icloud_calendar_credentials_at(
                &client,
                &format!("http://{address}/caldav"),
                "owner@icloud.com",
                "test-app-password",
            )
            .await
            .is_ok()
        );
        assert!(saw_authorization.load(Ordering::SeqCst));
        assert!(matches!(
            verify_icloud_calendar_credentials_at(
                &client,
                &format!("http://{address}/redirect"),
                "owner@icloud.com",
                "test-app-password",
            )
            .await,
            Err(ProviderReadError::InvalidResponse)
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

        let empty_inbox = Arc::new(AtomicBool::new(false));
        let empty_inbox_for_handler = Arc::clone(&empty_inbox);
        let message_reads = Arc::new(AtomicUsize::new(0));
        let message_reads_for_handler = message_reads.clone();
        let app = axum::Router::new()
            .route(
                "/gmail/v1/users/me/messages",
                axum::routing::get(move || {
                    let empty = Arc::clone(&empty_inbox_for_handler);
                    async move {
                        if empty.load(Ordering::SeqCst) {
                            axum::Json(json!({"resultSizeEstimate":0}))
                        } else {
                            axum::Json(json!({"messages":[{"id":"message-1","threadId":"thread-1"}]}))
                        }
                    }
                }),
            )
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
        let selected = client
            .mail_by_ids(
                &account,
                &vault,
                &agent_id,
                &format!("agent:{agent_id}"),
                &ids,
            )
            .await
            .unwrap();
        assert_eq!(selected.len(), 1);
        assert_eq!(selected[0].provider_id, "message-1");
        assert_eq!(selected[0].body_text.as_deref(), Some("hello from inbox"));
        assert_eq!(message_reads.load(Ordering::SeqCst), 2);
        empty_inbox.store(true, Ordering::SeqCst);
        assert!(
            client
                .recent_mail_ids(
                    &account,
                    &vault,
                    &agent_id,
                    &format!("agent:{agent_id}"),
                    100
                )
                .await
                .unwrap()
                .is_empty()
        );
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

    #[tokio::test]
    async fn google_watch_uses_bounded_history_pages_and_recovers_expired_cursor() {
        use axum::http::Request;
        use axum::response::IntoResponse;

        vak_config::paths::isolate_home_for_tests();
        let agent_id = format!("mailcal-gmail-history-{}", Uuid::now_v7());
        let account_id = Uuid::now_v7().to_string();
        let vault = AccountVault::for_agent(&agent_id).unwrap();
        vault
            .store(
                &account_id,
                AccountSecretMaterial::new(
                    "google:history-subject".into(),
                    Some("owner@example.test".into()),
                    Some("client".into()),
                    Some("history-token".into()),
                    Some("refresh-token".into()),
                    None,
                    None,
                )
                .unwrap(),
            )
            .unwrap();

        let app = axum::Router::new()
            .route(
                "/gmail/v1/users/me/profile",
                axum::routing::get(|request: Request<axum::body::Body>| async move {
                    assert_eq!(
                        request
                            .headers()
                            .get(reqwest::header::AUTHORIZATION)
                            .and_then(|value| value.to_str().ok()),
                        Some("Bearer history-token")
                    );
                    axum::Json(json!({"historyId":"100"}))
                }),
            )
            .route(
                "/gmail/v1/users/me/messages",
                axum::routing::get(|request: Request<axum::body::Body>| async move {
                    assert!(request.uri().query().unwrap_or_default().contains("labelIds=INBOX"));
                    axum::Json(json!({"messages":[{"id":"initial-message"}]}))
                }),
            )
            .route(
                "/gmail/v1/users/me/history",
                axum::routing::get(|request: Request<axum::body::Body>| async move {
                    assert_eq!(
                        request
                            .headers()
                            .get(reqwest::header::AUTHORIZATION)
                            .and_then(|value| value.to_str().ok()),
                        Some("Bearer history-token")
                    );
                    let query = request.uri().query().unwrap_or_default();
                    if query
                        .split('&')
                        .any(|part| part == "startHistoryId=1")
                    {
                        return axum::http::StatusCode::NOT_FOUND.into_response();
                    }
                    if query.contains("pageToken=page-2") {
                        assert!(query.contains("startHistoryId=100"));
                        return axum::Json(json!({
                            "historyId":"110",
                            "history":[{"messagesAdded":[{"message":{"id":"message-3","labelIds":["INBOX"]}}]}]
                        })).into_response();
                    }
                    assert!(query.contains("startHistoryId=100"));
                    if query.contains("pageToken=page-1") {
                        return axum::Json(json!({
                            "historyId":"110",
                            "history":[{"messagesAdded":[{"message":{"id":"message-3","labelIds":["INBOX"]}}]}]
                        })).into_response();
                    }
                    axum::Json(json!({
                        "historyId":"105",
                        "nextPageToken":"page-2",
                        "history":[
                            {"messagesAdded":[
                                {"message":{"id":"message-1","labelIds":["INBOX"]}},
                                {"message":{"id":"sent-message","labelIds":["SENT"]}}
                            ]},
                            {"labelsAdded":[{"message":{"id":"message-2"},"labelIds":["INBOX"]}]}
                        ]
                    })).into_response()
                }),
            );
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let task = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
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
            allowed_audiences: [format!("agent:{agent_id}")].into_iter().collect(),
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
        let audience = format!("agent:{agent_id}");
        let (initial, cursor) = client
            .mail_watch_page(&account, &vault, &agent_id, &audience, None, 100)
            .await
            .unwrap();
        assert_eq!(initial, ["initial-message"]);
        let cursor = cursor.unwrap();
        let (first_page, cursor) = client
            .mail_watch_page(&account, &vault, &agent_id, &audience, Some(&cursor), 100)
            .await
            .unwrap();
        assert_eq!(first_page, ["message-1", "message-2"]);
        let cursor = cursor.unwrap();
        let (second_page, cursor) = client
            .mail_watch_page(&account, &vault, &agent_id, &audience, Some(&cursor), 100)
            .await
            .unwrap();
        assert_eq!(second_page, ["message-3"]);
        assert_eq!(
            decode_google_watch_cursor(cursor.as_deref().unwrap())
                .unwrap()
                .start_history_id,
            "110"
        );
        let expired = encode_google_watch_cursor(&GoogleWatchCursor {
            start_history_id: "1".into(),
            page_token: None,
        })
        .unwrap();
        assert!(matches!(
            client
                .mail_watch_page(&account, &vault, &agent_id, &audience, Some(&expired), 100)
                .await,
            Err(ProviderReadError::WatchCursorReset)
        ));
        vault.remove(&account_id).unwrap();
        task.abort();
    }

    #[tokio::test]
    async fn microsoft_watch_fetches_only_selected_message_ids() {
        use axum::http::Request;
        use std::sync::atomic::{AtomicBool, Ordering};

        vak_config::paths::isolate_home_for_tests();
        let agent_id = format!("mailcal-graph-watch-{}", Uuid::now_v7());
        let account_id = Uuid::now_v7().to_string();
        let vault = AccountVault::for_agent(&agent_id).unwrap();
        vault
            .store(
                &account_id,
                AccountSecretMaterial::new(
                    "microsoft:subject".into(),
                    Some("owner@example.test".into()),
                    Some("client".into()),
                    Some("graph-token".into()),
                    Some("refresh-token".into()),
                    None,
                    None,
                )
                .unwrap(),
            )
            .unwrap();

        let admitted = Arc::new(AtomicBool::new(false));
        let admitted_for_request = Arc::clone(&admitted);
        let app = axum::Router::new().route(
            "/graph/v1.0/me/messages/ms-message-1",
            axum::routing::get(move |request: Request<axum::body::Body>| {
                let admitted = Arc::clone(&admitted_for_request);
                async move {
                    admitted.store(
                        request
                            .headers()
                            .get(reqwest::header::AUTHORIZATION)
                            .and_then(|value| value.to_str().ok())
                            == Some("Bearer graph-token")
                            && request
                                .headers()
                                .get("Prefer")
                                .and_then(|value| value.to_str().ok())
                                == Some("outlook.body-content-type=\"text\""),
                        Ordering::SeqCst,
                    );
                    axum::Json(json!({
                        "id":"ms-message-1",
                        "conversationId":"conversation-1",
                        "subject":"Graph selected",
                        "from":{"emailAddress":{"address":"sender@example.test"}},
                        "receivedDateTime":"2026-09-30T12:00:00Z",
                        "bodyPreview":"selected preview",
                        "body":{"contentType":"text","content":"selected body"},
                        "hasAttachments":false
                    }))
                }
            }),
        );
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let task = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
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
            provider: Provider::Microsoft,
            status: AccountStatus::Connected,
            owner_agent_id: agent_id.clone(),
            allowed_audiences: [format!("agent:{agent_id}")].into_iter().collect(),
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
        let items = client
            .mail_by_ids(
                &account,
                &vault,
                &agent_id,
                &format!("agent:{agent_id}"),
                &["ms-message-1".into()],
            )
            .await
            .unwrap();
        assert!(admitted.load(Ordering::SeqCst));
        assert_eq!(items.len(), 1);
        assert_eq!(items[0].provider_id, "ms-message-1");
        assert_eq!(items[0].body_text.as_deref(), Some("selected body"));
        task.abort();
    }
}
