//! Discord surface adapter (docs/design/34-channel-onboarding.md Phase 3),
//! built to the same contract as `telegram.rs`: poll the remote API, route
//! every message through `POST /gateway/inbound`, deliver the reply back.
//!
//! **Transport choice.** Discord's real-time surface is a gateway
//! websocket, which would mean a new websocket dependency for the whole
//! workspace (there is none today). This bridge instead polls
//! `GET /channels/{id}/messages?after=<last>` for an explicitly configured
//! set of channel ids and replies with `POST /channels/{id}/messages` —
//! the same shape as Telegram's long poll, no new dependency, and correct
//! for the "bot watches a few channels" case this phase is for. The
//! trade-off is that channels must be named up front (`DISCORD_CHANNEL_IDS`)
//! rather than DMs being auto-discovered; the gateway websocket is the
//! follow-up that removes that limit.
//!
//! Launched via `vak discord --server URL --token GATEWAY_TOKEN` with
//! `DISCORD_BOT_TOKEN` in the environment (.env included).

use serde_json::Value;

use crate::gateway::{InboundChannel, InboundRequest};

const API_BASE_DEFAULT: &str = "https://discord.com/api/v10";

pub struct DiscordBridge {
    /// API base, e.g. `https://discord.com/api/v10`. Overridable for
    /// tests and proxies via `DISCORD_API_BASE`.
    pub api_base: String,
    pub bot_token: String,
    /// Channel ids this bot watches. Empty is a configuration error, not
    /// a silent no-op: a bridge that polls nothing looks identical to a
    /// broken one.
    pub channel_ids: Vec<String>,
    pub gateway_url: String,
    pub gateway_token: String,
    /// Seconds between polls. Discord's REST rate limits are generous at
    /// this cadence for a handful of channels.
    pub poll_secs: u64,
    /// See `TelegramBridge::bot_id`.
    pub bot_id: Option<String>,
}

impl InboundChannel for DiscordBridge {
    fn surface(&self) -> &'static str {
        "discord"
    }
}

/// One routable message: Discord's own snowflake ids, never a placeholder
/// (see `InboundRequest::new` for why that distinction is enforced).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DiscordMessage {
    pub id: String,
    pub channel_id: String,
    pub author_id: String,
    pub text: String,
}

/// Parse a `GET /channels/{id}/messages` page into routable messages,
/// oldest first (Discord returns newest first). Bot-authored messages are
/// dropped: echoing our own replies back into the gateway would loop.
pub fn parse_messages(channel_id: &str, body: &Value) -> Vec<DiscordMessage> {
    let mut out: Vec<DiscordMessage> = body
        .as_array()
        .map(|items| {
            items
                .iter()
                .filter(|m| m["author"]["bot"].as_bool() != Some(true))
                .filter_map(|m| {
                    let text = m["content"].as_str().unwrap_or_default().to_string();
                    if text.trim().is_empty() {
                        return None;
                    }
                    Some(DiscordMessage {
                        id: m["id"].as_str()?.to_string(),
                        channel_id: channel_id.to_string(),
                        author_id: m["author"]["id"].as_str()?.to_string(),
                        text,
                    })
                })
                .collect()
        })
        .unwrap_or_default();
    // Snowflakes are monotonic and fixed-width enough that lexical order
    // matches time order for any ids of the same length; sort by (len, id)
    // so a rollover to a longer snowflake still orders correctly.
    out.sort_by(|a, b| (a.id.len(), &a.id).cmp(&(b.id.len(), &b.id)));
    out
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

/// Backoff for consecutive poll failures: doubling, capped at 30s — the
/// same ceiling the Telegram bridge uses.
pub fn backoff_secs(attempt: u32) -> u64 {
    (1u64 << attempt.min(5)).min(30)
}

impl DiscordBridge {
    pub fn from_env(
        gateway_url: String,
        gateway_token: String,
        bot_token: String,
        bot_id: Option<String>,
    ) -> Self {
        let channel_ids = vak_config::get_var("DISCORD_CHANNEL_IDS")
            .unwrap_or_default()
            .split(',')
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty())
            .collect();
        DiscordBridge {
            api_base: vak_config::get_var("DISCORD_API_BASE")
                .unwrap_or_else(|| API_BASE_DEFAULT.to_string()),
            bot_token,
            channel_ids,
            gateway_url: gateway_url.trim_end_matches('/').to_string(),
            gateway_token,
            poll_secs: 3,
            bot_id,
        }
    }

    /// One poll pass over every watched channel. `cursors` maps channel id
    /// to the last message id already routed, so a restart never replays
    /// and a transient failure never skips.
    pub async fn tick(
        &self,
        cursors: &mut std::collections::HashMap<String, String>,
    ) -> Result<(), String> {
        for channel_id in &self.channel_ids {
            let messages = self.fetch(channel_id, cursors.get(channel_id)).await?;
            for message in messages {
                // First sight of a channel: adopt the cursor without
                // replaying its backlog into the agent.
                let cold_start = !cursors.contains_key(channel_id);
                cursors.insert(channel_id.clone(), message.id.clone());
                if cold_start {
                    continue;
                }
                let reply = self.process(&message).await;
                if let Err(e) = self.send_message(channel_id, &reply).await {
                    eprintln!("[discord] send to {channel_id} failed: {e}");
                }
            }
        }
        Ok(())
    }

    async fn fetch(
        &self,
        channel_id: &str,
        after: Option<&String>,
    ) -> Result<Vec<DiscordMessage>, String> {
        let mut query: Vec<(&str, String)> = vec![("limit", "25".to_string())];
        match after {
            Some(id) => query.push(("after", id.clone())),
            // Cold start: one message is enough to seed the cursor.
            None => query[0].1 = "1".to_string(),
        }
        let resp = http()
            .get(format!("{}/channels/{channel_id}/messages", self.api_base))
            .header("Authorization", format!("Bot {}", self.bot_token))
            .query(&query)
            .send()
            .await
            .map_err(|e| format!("discord getMessages: {e}"))?;
        if !resp.status().is_success() {
            return Err(format!("discord getMessages returned {}", resp.status()));
        }
        let body: Value = resp
            .json()
            .await
            .map_err(|e| format!("discord getMessages body: {e}"))?;
        Ok(parse_messages(channel_id, &body))
    }

    /// One message through the gateway contract; wait for the final text.
    async fn process(&self, message: &DiscordMessage) -> String {
        // 0c-03: real per-user chat/sender, never a fixed placeholder.
        let req = match InboundRequest::new(
            self,
            message.channel_id.clone(),
            message.author_id.clone(),
            message.text.clone(),
        ) {
            Ok(req) => req.waiting().with_bot_id(self.bot_id.clone()),
            Err(e) => return format!("(bridge refused to send: {e})"),
        };
        let res = http()
            .post(format!("{}/gateway/inbound", self.gateway_url))
            .bearer_auth(&self.gateway_token)
            .json(&req)
            .send()
            .await;
        match res {
            Ok(r) if r.status().as_u16() == 202 => {
                "(queued: I'm still working on your previous message)".into()
            }
            Ok(r) if r.status().is_success() => match r.json::<Value>().await {
                Ok(v) => v["text"].as_str().unwrap_or("(empty reply)").to_string(),
                Err(e) => format!("(bad gateway reply: {e})"),
            },
            Ok(r) => format!("(gateway error: {})", r.status()),
            Err(e) => format!("(gateway unreachable: {e})"),
        }
    }

    async fn send_message(&self, channel_id: &str, text: &str) -> Result<(), String> {
        for chunk in chunk_text(text, 1900) {
            let resp = http()
                .post(format!("{}/channels/{channel_id}/messages", self.api_base))
                .header("Authorization", format!("Bot {}", self.bot_token))
                .json(&serde_json::json!({ "content": chunk }))
                .send()
                .await
                .map_err(|e| format!("discord createMessage: {e}"))?;
            if !resp.status().is_success() {
                return Err(format!("discord createMessage returned {}", resp.status()));
            }
        }
        Ok(())
    }

    /// Run until the process is killed. Transient failures back off and
    /// retry; the per-channel cursor makes every recovery gap-free.
    pub async fn run(&self) -> Result<(), String> {
        if self.channel_ids.is_empty() {
            return Err(
                "no channels to watch — set DISCORD_CHANNEL_IDS to a comma-separated \
                 list of Discord channel ids the bot can read"
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
                        eprintln!("[discord] poll failed ({failures} consecutive): {e}");
                    }
                    tokio::time::sleep(std::time::Duration::from_secs(backoff_secs(failures)))
                        .await;
                }
            }
        }
    }
}

/// Split on message boundaries the remote surface enforces, preferring a
/// line break so a code block or list is not cut mid-token.
pub fn chunk_text(text: &str, max: usize) -> Vec<String> {
    if text.chars().count() <= max {
        return vec![text.to_string()];
    }
    let mut out = Vec::new();
    let mut current = String::new();
    for line in text.split_inclusive('\n') {
        if current.chars().count() + line.chars().count() > max && !current.is_empty() {
            out.push(std::mem::take(&mut current));
        }
        // A single line longer than the cap is hard-split; nothing else can
        // be done without dropping content.
        if line.chars().count() > max {
            let mut buf = String::new();
            for ch in line.chars() {
                buf.push(ch);
                if buf.chars().count() == max {
                    out.push(std::mem::take(&mut buf));
                }
            }
            current = buf;
            continue;
        }
        current.push_str(line);
    }
    if !current.is_empty() {
        out.push(current);
    }
    out
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;

    fn bridge() -> DiscordBridge {
        DiscordBridge {
            api_base: "http://localhost".into(),
            bot_token: "t".into(),
            channel_ids: vec!["555".into()],
            gateway_url: "http://localhost".into(),
            gateway_token: "g".into(),
            poll_secs: 1,
            bot_id: None,
        }
    }

    #[test]
    fn surface_is_the_allowlist_key_prefix() {
        assert_eq!(bridge().surface(), "discord");
    }

    #[test]
    fn identity_maps_channel_to_chat_and_author_to_sender() {
        let req = InboundRequest::new(&bridge(), "555", "42", "hi").unwrap();
        assert_eq!(req.surface, "discord");
        assert_eq!(req.chat, "555");
        assert_eq!(req.sender, "42");
    }

    #[test]
    fn empty_or_placeholder_identity_is_refused() {
        // 0c-03: a bridge that has not wired up real identity must fail
        // loudly, not merge every stranger into one session.
        assert!(InboundRequest::new(&bridge(), "", "42", "hi").is_err());
        assert!(InboundRequest::new(&bridge(), "555", "  ", "hi").is_err());
        assert!(InboundRequest::new(&bridge(), "discord", "42", "hi").is_err());
        assert!(InboundRequest::new(&bridge(), "555", "discord", "hi").is_err());
    }

    #[test]
    fn parses_oldest_first_and_drops_bot_and_empty_messages() {
        let body = serde_json::json!([
            { "id": "30", "content": "third", "author": { "id": "7" } },
            { "id": "20", "content": "from me", "author": { "id": "1", "bot": true } },
            { "id": "10", "content": "first", "author": { "id": "7" } },
            { "id": "40", "content": "   ", "author": { "id": "7" } },
        ]);
        let parsed = parse_messages("555", &body);
        assert_eq!(parsed.len(), 2);
        assert_eq!(parsed[0].id, "10");
        assert_eq!(parsed[0].text, "first");
        assert_eq!(parsed[0].channel_id, "555");
        assert_eq!(parsed[0].author_id, "7");
        assert_eq!(parsed[1].id, "30");
    }

    #[test]
    fn longer_snowflakes_sort_after_shorter_ones() {
        let body = serde_json::json!([
            { "id": "1000000000000000000", "content": "new", "author": { "id": "7" } },
            { "id": "999999999999999999", "content": "old", "author": { "id": "7" } },
        ]);
        let parsed = parse_messages("555", &body);
        assert_eq!(parsed[0].text, "old");
        assert_eq!(parsed[1].text, "new");
    }

    #[test]
    fn chunking_respects_the_cap_and_keeps_every_character() {
        let text = "abcd\n".repeat(100);
        let chunks = chunk_text(&text, 40);
        assert!(chunks.iter().all(|c| c.chars().count() <= 40));
        assert_eq!(chunks.concat(), text);
    }

    #[test]
    fn short_text_is_one_chunk() {
        assert_eq!(chunk_text("hello", 100), vec!["hello".to_string()]);
    }

    #[test]
    fn backoff_doubles_then_caps() {
        assert_eq!(backoff_secs(0), 1);
        assert_eq!(backoff_secs(3), 8);
        assert_eq!(backoff_secs(99), 30);
    }
}
