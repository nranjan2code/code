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
//! `SLACK_BOT_TOKEN` in the environment (the credential store included).

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
    pub audio_url: Option<String>,
    pub audio_mime: Option<String>,
}

#[derive(Debug, Default)]
struct GatewayReply {
    chunks: Vec<String>,
    session_id: Option<String>,
    delivery: Option<vak_delivery::DeliveryPacket>,
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
                    let audio = m["files"].as_array().and_then(|items| {
                        items.iter().find(|f| {
                            f["mimetype"]
                                .as_str()
                                .is_some_and(|mime| mime.starts_with("audio/"))
                                && f["url_private_download"].as_str().is_some()
                        })
                    });
                    if text.trim().is_empty() && audio.is_none() {
                        return None;
                    }
                    Some(SlackMessage {
                        ts: m["ts"].as_str()?.to_string(),
                        channel_id: channel_id.to_string(),
                        user_id: m["user"].as_str()?.to_string(),
                        text,
                        audio_url: audio
                            .and_then(|f| f["url_private_download"].as_str())
                            .map(str::to_string),
                        audio_mime: audio
                            .and_then(|f| f["mimetype"].as_str())
                            .map(str::to_string),
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
                if message.audio_url.is_some()
                    && let Err(e) = self
                        .send_voice(
                            channel_id,
                            &reply.chunks.join("\n"),
                            reply.session_id.as_deref(),
                        )
                        .await
                {
                    eprintln!("[slack] voice reply unavailable: {e}");
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

    async fn process(&self, message: &SlackMessage) -> GatewayReply {
        let text = message.text.clone();
        let mut attachments = Vec::new();
        if let Some(url) = &message.audio_url
            && let Ok(response) = http().get(url).bearer_auth(&self.bot_token).send().await
            && response.status().is_success()
            && let Ok(bytes) = response.bytes().await
            && bytes.len() <= 16 * 1024 * 1024
        {
            use base64::Engine as _;
            let encoded = base64::engine::general_purpose::STANDARD.encode(bytes);
            attachments.push(serde_json::json!({
                "data": encoded,
                "mime": message.audio_mime.as_deref().unwrap_or("audio/ogg"),
                "kind": "audio",
                "filename": "voice",
            }));
        }
        // 0c-03: real per-user chat/sender, never a fixed placeholder.
        let req = match InboundRequest::new(
            self,
            message.channel_id.clone(),
            message.user_id.clone(),
            text,
        ) {
            Ok(req) => req
                .with_attachments(attachments)
                .waiting()
                .with_bot_id(self.bot_id.clone()),
            Err(e) => {
                return GatewayReply {
                    chunks: vec![format!("(bridge refused to send: {e})")],
                    ..Default::default()
                };
            }
        };
        let res = http()
            .post(format!("{}/gateway/inbound", self.gateway_url))
            .bearer_auth(&self.gateway_token)
            .json(&req)
            .send()
            .await;
        match res {
            Ok(r) if r.status().as_u16() == 202 => GatewayReply {
                chunks: vec!["(queued: I'm still working on your previous message)".into()],
                ..Default::default()
            },
            Ok(r) if r.status().is_success() => match r.json::<Value>().await {
                Ok(v) => {
                    let session_id = v["session_id"].as_str().map(String::from);
                    let delivery = super::prepared_packet(v.clone(), "slack");
                    let chunks = delivery
                        .as_ref()
                        .map(|packet| packet.chunks.clone())
                        .unwrap_or_else(|e| vec![format!("(delivery failed: {e})")]);
                    GatewayReply {
                        chunks,
                        session_id,
                        delivery: delivery.ok(),
                    }
                }
                Err(e) => GatewayReply {
                    chunks: vec![format!("(bad gateway reply: {e})")],
                    ..Default::default()
                },
            },
            Ok(r) => GatewayReply {
                chunks: vec![format!("(gateway error: {})", r.status())],
                ..Default::default()
            },
            Err(e) => GatewayReply {
                chunks: vec![format!("(gateway unreachable: {e})")],
                ..Default::default()
            },
        }
    }

    async fn send_message(&self, channel_id: &str, reply: &GatewayReply) -> Result<(), String> {
        let cards = reply
            .delivery
            .as_ref()
            .map(vak_delivery::DeliveryPacket::structured_cards)
            .unwrap_or_default();
        if !cards.is_empty() {
            let cards = cards.into_iter().cloned().collect::<Vec<_>>();
            let chunks = if reply.chunks.is_empty() {
                vec![String::new()]
            } else {
                reply.chunks.clone()
            };
            let body = serde_json::json!({
                "channel": channel_id,
                "text": chunks[0],
                "blocks": vak_delivery::slack::structured_card_blocks(&cards),
                "parse": "none",
                "link_names": false,
            });
            let resp = http()
                .post(format!("{}/chat.postMessage", self.api_base))
                .bearer_auth(&self.bot_token)
                .json(&body)
                .send()
                .await
                .map_err(|e| format!("slack chat.postMessage: {e}"))?;
            if !resp.status().is_success() {
                return Err(format!("slack chat.postMessage returned {}", resp.status()));
            }
            let result: Value = resp
                .json()
                .await
                .map_err(|e| format!("slack chat.postMessage body: {e}"))?;
            if result["ok"].as_bool() != Some(true) {
                return Err(format!(
                    "slack chat.postMessage not ok: {}",
                    result["error"].as_str().unwrap_or("?")
                ));
            }
            for chunk in chunks.iter().skip(1) {
                let response = http()
                    .post(format!("{}/chat.postMessage", self.api_base))
                    .bearer_auth(&self.bot_token)
                    .json(&serde_json::json!({"channel":channel_id,"text":chunk,"parse":"none","link_names":false}))
                    .send().await.map_err(|e| format!("slack chat.postMessage: {e}"))?;
                if !response.status().is_success() {
                    return Err(format!(
                        "slack chat.postMessage returned {}",
                        response.status()
                    ));
                }
                let result: Value = response
                    .json()
                    .await
                    .map_err(|e| format!("slack chat.postMessage body: {e}"))?;
                if result["ok"].as_bool() != Some(true) {
                    return Err(format!(
                        "slack chat.postMessage not ok: {}",
                        result["error"].as_str().unwrap_or("?")
                    ));
                }
            }
            return Ok(());
        }
        for chunk in &reply.chunks {
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

    async fn send_voice(
        &self,
        channel_id: &str,
        text: &str,
        session_id: Option<&str>,
    ) -> Result<(), String> {
        let response = http()
            .post(format!("{}/voice/speak", self.gateway_url))
            .bearer_auth(&self.gateway_token)
            .json(&serde_json::json!({"text": text, "format": "wav", "session_id": session_id}))
            .send()
            .await
            .map_err(|e| e.to_string())?;
        if !response.status().is_success() {
            return Err(format!("voice speak returned {}", response.status()));
        }
        let part = reqwest::multipart::Part::bytes(
            response.bytes().await.map_err(|e| e.to_string())?.to_vec(),
        )
        .file_name("reply.wav")
        .mime_str("audio/wav")
        .map_err(|e| e.to_string())?;
        let sent = http()
            .post(format!("{}/files.uploadV2", self.api_base))
            .bearer_auth(&self.bot_token)
            .multipart(
                reqwest::multipart::Form::new()
                    .text("channel_id", channel_id.to_string())
                    .part("file", part),
            )
            .send()
            .await
            .map_err(|e| e.to_string())?;
        if !sent.status().is_success() {
            return Err(format!("slack voice upload returned {}", sent.status()));
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

    fn sample_card() -> vak_delivery::StructuredOutput {
        vak_delivery::StructuredOutput {
            semantic_type: "metric".into(),
            schema_version: vak_delivery::PRESENTATION_SCHEMA_VERSION,
            skill_id: "core".into(),
            skill_version: "1".into(),
            payload: serde_json::json!({"title":"Weather","Temperature":"23°C","Status":"Sunny"}),
        }
    }

    #[test]
    fn card_delivery_uses_native_block_kit_fields() {
        let blocks = vak_delivery::slack::structured_card_blocks(&[sample_card()]);
        assert_eq!(blocks[0]["type"], "header");
        assert_eq!(blocks[0]["text"]["text"], "Weather");
        assert_eq!(blocks[1]["fields"][0]["type"], "plain_text");
        assert_eq!(blocks[1]["fields"][0]["text"], "Status\nSunny");
        assert!(
            !blocks
                .iter()
                .any(|block| block.to_string().contains("\"semantic_type\""))
        );
    }

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
    fn parses_audio_files_without_text_for_governed_transcription() {
        let body = serde_json::json!({"messages":[{"ts":"1.1","text":"","user":"U9","files":[{"mimetype":"audio/ogg","url_private_download":"https://files.example/voice"}]}]});
        let parsed = parse_history("C1", &body);
        assert_eq!(parsed.len(), 1);
        assert_eq!(parsed[0].audio_mime.as_deref(), Some("audio/ogg"));
        assert_eq!(
            parsed[0].audio_url.as_deref(),
            Some("https://files.example/voice")
        );
    }

    #[test]
    fn audio_file_without_download_url_is_not_routable_as_voice() {
        let body = serde_json::json!({"messages":[{"ts":"1.2","text":"","user":"U9","files":[{"mimetype":"audio/ogg"}]}]});
        let parsed = parse_history("C1", &body);
        assert!(
            parsed.is_empty(),
            "unfetchable media must not enter the governed path"
        );
    }

    #[test]
    fn gateway_reply_session_id_is_preserved_for_playback() {
        let value = serde_json::json!({"chunks":["ok"],"session_id":"slack-session"});
        let reply = GatewayReply {
            chunks: value["chunks"]
                .as_array()
                .unwrap()
                .iter()
                .filter_map(|v| v.as_str().map(String::from))
                .collect(),
            session_id: value["session_id"].as_str().map(String::from),
            delivery: None,
        };
        assert_eq!(reply.session_id.as_deref(), Some("slack-session"));
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
