//! Fixed-host, non-retrying provider effects.
//!
//! The caller owns the durable single-use claim and must record `Unknown`
//! after an ambiguous result. This client never follows redirects or retries.

use crate::{
    Capability, ConnectedAccount, MailAddress, MailDraft, Provider, provider::ProviderReadError,
    vault::AccountVault,
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
            ProviderReadError::InvalidRange => Self::Rejected,
        }
    }
}

#[derive(Clone)]
pub struct ProviderEffectClient {
    http: reqwest::Client,
    google_gmail_base: String,
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
            microsoft_graph_base: "https://graph.microsoft.com/v1.0".into(),
        })
    }

    /// Send a plain-text email through the provider's fixed send endpoint.
    /// Attachments, sender aliases, and provider-thread reply semantics remain
    /// unsupported until they have explicit review and serialization rules.
    pub async fn send_mail(
        &self,
        account: &ConnectedAccount,
        vault: &AccountVault,
        agent_id: &str,
        audience: &str,
        draft: &MailDraft,
    ) -> Result<ProviderAcceptance, ProviderEffectError> {
        if vault.agent_id() != account.owner_agent_id
            || !account.admits(agent_id, audience, Capability::MailSend)
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
                let raw = google_raw_message(draft);
                let body = json!({
                    "raw": base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(raw.as_bytes())
                });
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
                let response = self
                    .http
                    .post(format!("{}/me/sendMail", self.microsoft_graph_base))
                    .bearer_auth(token.as_str())
                    .json(&graph_send_payload(draft))
                    .send()
                    .await
                    .map_err(|_| ProviderEffectError::Unknown)?;
                classify_graph_response(response).await
            }
            Provider::AppleIcloud => Err(ProviderEffectError::Unsupported),
        }
    }
}

/// Validate the currently supported plain-text send profile before a durable
/// single-use dispatch claim is created.
pub fn validate_mail_draft(draft: &MailDraft) -> Result<(), ProviderEffectError> {
    draft
        .validate()
        .map_err(|_| ProviderEffectError::Rejected)?;
    if !draft.attachment_refs.is_empty()
        || draft.from_alias.is_some()
        || draft.reply_to_message_id.is_some()
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
        };
        let raw = google_raw_message(&draft);
        assert!(raw.contains("To: to@example.com\r\n"));
        assert!(raw.contains("Cc: cc@example.com\r\n"));
        assert!(raw.contains("Bcc: bcc@example.com\r\n"));
        assert!(raw.contains("Subject: =?UTF-8?B?"));
    }
}
