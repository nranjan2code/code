//! Slack surface adapter (docs/design/34-channel-onboarding.md Phase 3),
//! same contract as `telegram.rs`/`discord.rs`.
//!
//! **Transport choice.** Socket Mode (the option that needs no public
//! URL) is a websocket, and the workspace has no websocket dependency
//! today; the Events API alternative needs an internet-reachable webhook
//! endpoint, which a laptop bridge does not have. This bridge therefore
//! polls `conversations.history` for an explicitly configured set of
//! channel ids and replies with `chat.postMessage` — no new dependency, no
//! public URL, and the same "watch a few channels" shape as the Discord
//! bridge. Socket Mode is the follow-up for real-time delivery and
//! interactive Block Kit buttons.
//!
//! Launched via `vak slack --server URL --token GATEWAY_TOKEN` with
//! `SLACK_BOT_TOKEN` in the environment (.env included).

use serde_json::Value;

use crate::gateway::{InboundChannel, InboundRequest};

const API_BASE_DEFAULT: &str = "https://slack.com/api";

pub struct SlackBridge {
    /// API base, e.g. `https://slack.com/api`. Overridable for tests via
    /// `SLACK_API_BASE`.
    pub api_base: String,
    pub bot_token: String,
    /// Channel/DM ids this bot watches (`SLACK_CHANNEL_IDS`).
    pub channel_ids: Vec<String>,
    pub gateway_url: String,
    pub gateway_token: String,
    pub poll_secs: u64,
    /// See `TelegramBridge::bot_id`.
    pub bot_id: Option<String>,
}

impl InboundChannel for SlackBridge {
    fn surface(&self) -> &'static str {
        "slack"
    }
}

/// One routable Slack message. `ts` is Slack's own message timestamp,
/// which doubles as its id and as the `oldest` cursor.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SlackMessage {
    pub ts: String,
    pub channel_id: String,
    pub user_id: String,
    pub text: String,
}

/// Parse a `conversations.history` response into routable messages,
/// oldest first (Slack returns newest first). Bot messages and message
/// subtypes (joins, edits, thread broadcasts) are dropped: only a real
/// human message should open or continue a session.
pub fn parse_history(channel_id: &str, body: &Value) -> Vec<SlackMessage> {
    let mut out: Vec<SlackMessage> = body["messages"]
        .as_array()
        .map(|items| {
            items
                .iter()
                .filter(|m| m["bot_id"].is_null() && m["subtype"].is_null())
                .filter_map(|m| {
                    let text = m["text"].as_str().unwrap_or_default().to_string();
                    if text.trim().is_empty() {
                        return None;
                    }
                    Some(SlackMessage {
                        ts: m["ts"].as_str()?.to_string(),
                        channel_id: channel_id.to_string(),
                        user_id: m["user"].as_str()?.to_string(),
                        text,
                    })
                })
                .collect()
        })
        .unwrap_or_default();
    // Slack timestamps are "<seconds>.<microseconds>" — compare
    // numerically so a shorter second-part never sorts wrong.
    out.sort_by(|a, b| {
        ts_value(&a.ts)
            .partial_cmp(&ts_value(&b.ts))
            .unwrap_or(std::cmp::Ordering::Equal)
    });
    out
}

fn ts_value(ts: &str) -> f64 {
    ts.parse::<f64>().unwrap_or(0.0)
}

fn http() -> reqwest::Client {
    static CLIENT: std::sync::OnceLock<reqwest::Client> = std::sync::OnceLock::new();
    CLIENT
        .get_or_init(|| {
            reqwest::Client::builder()
                .timeout(std::time::Duration::from_secs(300))
                .build()
                .unwrap_or_default()
        })
        .clone()
}

impl SlackBridge {
    pub fn from_env(
        gateway_url: String,
        gateway_token: String,
        bot_token: String,
        bot_id: Option<String>,
    ) -> Self {
        let channel_ids = vak_config::get_var("SLACK_CHANNEL_IDS")
            .unwrap_or_default()
            .split(',')
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty())
            .collect();
        SlackBridge {
            api_base: vak_config::get_var("SLACK_API_BASE")
                .unwrap_or_else(|| API_BASE_DEFAULT.to_string()),
            bot_token,
            channel_ids,
            gateway_url: gateway_url.trim_end_matches('/').to_string(),
            gateway_token,
            poll_secs: 3,
            bot_id,
        }
    }

    pub async fn tick(
        &self,
        cursors: &mut std::collections::HashMap<String, String>,
    ) -> Result<(), String> {
        for channel_id in &self.channel_ids {
            let messages = self.fetch(channel_id, cursors.get(channel_id)).await?;
            for message in messages {
                let cold_start = !cursors.contains_key(channel_id);
                cursors.insert(channel_id.clone(), message.ts.clone());
                if cold_start {
                    continue;
                }
                let reply = self.process(&message).await;
                if let Err(e) = self.send_message(channel_id, &reply).await {
                    eprintln!("[slack] send to {channel_id} failed: {e}");
                }
            }
        }
        Ok(())
    }

    async fn fetch(
        &self,
        channel_id: &str,
        oldest: Option<&String>,
    ) -> Result<Vec<SlackMessage>, String> {
        let mut query: Vec<(&str, String)> = vec![
            ("channel", channel_id.to_string()),
            (
                "limit",
                if oldest.is_some() { "25" } else { "1" }.to_string(),
            ),
        ];
        if let Some(ts) = oldest {
            query.push(("oldest", ts.clone()));
            // `oldest` is inclusive by default; excluding it means the
            // cursor message is never re-routed.
            query.push(("inclusive", "false".to_string()));
        }
        let resp = http()
            .get(format!("{}/conversations.history", self.api_base))
            .bearer_auth(&self.bot_token)
            .query(&query)
            .send()
            .await
            .map_err(|e| format!("slack conversations.history: {e}"))?;
        if !resp.status().is_success() {
            return Err(format!(
                "slack conversations.history returned {}",
                resp.status()
            ));
        }
        let body: Value = resp
            .json()
            .await
            .map_err(|e| format!("slack conversations.history body: {e}"))?;
        // Slack answers 200 with `{"ok": false, "error": ...}`; treating
        // that as success would silently poll forever against a bad token.
        if body["ok"].as_bool() != Some(true) {
            return Err(format!(
                "slack conversations.history not ok: {}",
                body["error"].as_str().unwrap_or("?")
            ));
        }
        Ok(parse_history(channel_id, &body))
    }

    async fn process(&self, message: &SlackMessage) -> Vec<String> {
        // 0c-03: real per-user chat/sender, never a fixed placeholder.
        let req = match InboundRequest::new(
            self,
            message.channel_id.clone(),
            message.user_id.clone(),
            message.text.clone(),
        ) {
            Ok(req) => req.waiting().with_bot_id(self.bot_id.clone()),
            Err(e) => return vec![format!("(bridge refused to send: {e})")],
        };
        let res = http()
            .post(format!("{}/gateway/inbound", self.gateway_url))
            .bearer_auth(&self.gateway_token)
            .json(&req)
            .send()
            .await;
        match res {
            Ok(r) if r.status().as_u16() == 202 => {
                vec!["(queued: I'm still working on your previous message)".into()]
            }
            Ok(r) if r.status().is_success() => match r.json::<Value>().await {
                Ok(v) => super::prepared_chunks(v, "slack")
                    .unwrap_or_else(|e| vec![format!("(delivery failed: {e})")]),
                Err(e) => vec![format!("(bad gateway reply: {e})")],
            },
            Ok(r) => vec![format!("(gateway error: {})", r.status())],
            Err(e) => vec![format!("(gateway unreachable: {e})")],
        }
    }

    async fn send_message(&self, channel_id: &str, chunks: &[String]) -> Result<(), String> {
        for chunk in chunks {
            let resp = http()
                .post(format!("{}/chat.postMessage", self.api_base))
                .bearer_auth(&self.bot_token)
                .json(&serde_json::json!({ "channel": channel_id, "text": chunk }))
                .send()
                .await
                .map_err(|e| format!("slack chat.postMessage: {e}"))?;
            if !resp.status().is_success() {
                return Err(format!("slack chat.postMessage returned {}", resp.status()));
            }
            let body: Value = resp
                .json()
                .await
                .map_err(|e| format!("slack chat.postMessage body: {e}"))?;
            if body["ok"].as_bool() != Some(true) {
                return Err(format!(
                    "slack chat.postMessage not ok: {}",
                    body["error"].as_str().unwrap_or("?")
                ));
            }
        }
        Ok(())
    }

    pub async fn run(&self) -> Result<(), String> {
        if self.channel_ids.is_empty() {
            return Err(
                "no channels to watch — set SLACK_CHANNEL_IDS to a comma-separated \
                 list of Slack channel or DM ids the bot has been invited to"
                    .into(),
            );
        }
        let mut cursors = std::collections::HashMap::new();
        let mut failures: u32 = 0;
        loop {
            match self.tick(&mut cursors).await {
                Ok(()) => {
                    failures = 0;
                    tokio::time::sleep(std::time::Duration::from_secs(self.poll_secs)).await;
                }
                Err(e) => {
                    failures += 1;
                    if failures == 1 || failures.is_multiple_of(10) {
                        eprintln!("[slack] poll failed ({failures} consecutive): {e}");
                    }
                    tokio::time::sleep(std::time::Duration::from_secs(
                        crate::surfaces::discord::backoff_secs(failures),
                    ))
                    .await;
                }
            }
        }
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;

    fn bridge() -> SlackBridge {
        SlackBridge {
            api_base: "http://localhost".into(),
            bot_token: "t".into(),
            channel_ids: vec!["C1".into()],
            gateway_url: "http://localhost".into(),
            gateway_token: "g".into(),
            poll_secs: 1,
            bot_id: None,
        }
    }

    #[test]
    fn surface_is_the_allowlist_key_prefix() {
        assert_eq!(bridge().surface(), "slack");
    }

    #[test]
    fn identity_maps_channel_to_chat_and_user_to_sender() {
        let req = InboundRequest::new(&bridge(), "C1", "U9", "hi").unwrap();
        assert_eq!(req.surface, "slack");
        assert_eq!(req.chat, "C1");
        assert_eq!(req.sender, "U9");
    }

    #[test]
    fn empty_or_placeholder_identity_is_refused() {
        assert!(InboundRequest::new(&bridge(), "", "U9", "hi").is_err());
        assert!(InboundRequest::new(&bridge(), "C1", "", "hi").is_err());
        assert!(InboundRequest::new(&bridge(), "slack", "U9", "hi").is_err());
        assert!(InboundRequest::new(&bridge(), "C1", "slack", "hi").is_err());
    }

    #[test]
    fn parses_oldest_first_and_drops_bot_and_subtype_messages() {
        let body = serde_json::json!({
            "ok": true,
            "messages": [
                { "ts": "1000.000200", "text": "second", "user": "U9" },
                { "ts": "1000.000300", "text": "beep", "user": "U1", "bot_id": "B1" },
                { "ts": "1000.000100", "text": "first", "user": "U9" },
                { "ts": "1000.000400", "text": "joined", "user": "U9", "subtype": "channel_join" },
            ]
        });
        let parsed = parse_history("C1", &body);
        assert_eq!(parsed.len(), 2);
        assert_eq!(parsed[0].text, "first");
        assert_eq!(parsed[0].user_id, "U9");
        assert_eq!(parsed[0].channel_id, "C1");
        assert_eq!(parsed[1].text, "second");
    }

    #[test]
    fn timestamps_order_numerically_not_lexically() {
        let body = serde_json::json!({
            "ok": true,
            "messages": [
                { "ts": "1000.5", "text": "later", "user": "U9" },
                { "ts": "999.9", "text": "earlier", "user": "U9" },
            ]
        });
        let parsed = parse_history("C1", &body);
        assert_eq!(parsed[0].text, "earlier");
        assert_eq!(parsed[1].text, "later");
    }
}
