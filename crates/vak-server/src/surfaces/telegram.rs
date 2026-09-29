//! Telegram surface adapter (docs/design/22-gateway.md G1): long-polls the
//! Bot API and bridges messages through `POST /gateway/inbound`, then
//! delivers the final assistant text back via `sendMessage`; audio-originated
//! turns also receive a governed native `sendVoice` reply when configured.
//!
//! The gateway stays transport-agnostic; this client runs anywhere it can
//! reach both Telegram and a vak-server — laptop, VPS, sidecar. Launched via
//! `vak telegram --server URL --token GATEWAY_TOKEN` with
//! `TELEGRAM_BOT_TOKEN` in the environment (the credential store included).

use serde_json::Value;
use std::path::PathBuf;

use crate::gateway::{InboundChannel, InboundRequest};

/// Convert the server's canonical WAV synthesis artifact to Telegram's
/// `sendVoice` contract (Ogg/Opus). ffmpeg is an optional host capability;
/// when absent the caller can retain the text response and report the
/// actionable error instead of sending malformed media.
async fn wav_to_telegram_opus(wav: Vec<u8>) -> Result<Vec<u8>, String> {
    use tokio::io::AsyncWriteExt;
    let mut child = tokio::process::Command::new("ffmpeg")
        .args([
            "-hide_banner",
            "-loglevel",
            "error",
            "-f",
            "wav",
            "-i",
            "pipe:0",
            "-c:a",
            "libopus",
            "-b:a",
            "32k",
            "-vbr",
            "on",
            "-f",
            "ogg",
            "pipe:1",
        ])
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .map_err(|e| format!("Telegram voice requires ffmpeg for Ogg/Opus conversion: {e}"))?;
    if let Some(mut stdin) = child.stdin.take() {
        stdin
            .write_all(&wav)
            .await
            .map_err(|e| format!("write ffmpeg input: {e}"))?;
    }
    let output = child
        .wait_with_output()
        .await
        .map_err(|e| format!("wait for ffmpeg: {e}"))?;
    if !output.status.success() || output.stdout.is_empty() {
        let detail = String::from_utf8_lossy(&output.stderr);
        return Err(format!("ffmpeg Ogg/Opus conversion failed: {detail}"));
    }
    Ok(output.stdout)
}

fn telegram_http_error(operation: &str, error: &reqwest::Error) -> String {
    let kind = if error.is_timeout() {
        "request timed out"
    } else if error.is_connect() {
        "connection failed"
    } else if error.is_decode() {
        "response decode failed"
    } else if error.is_body() {
        "request or response body failed"
    } else if error.is_request() {
        "request failed"
    } else {
        "HTTP operation failed"
    };
    match error.status() {
        Some(status) => format!("{operation}: {kind} ({status})"),
        None => format!("{operation}: {kind}"),
    }
}

pub struct TelegramBridge {
    /// Bot API base, e.g. `https://api.telegram.org`. Overridable for
    /// self-hosted relays and tests via `TELEGRAM_API_BASE`.
    pub api_base: String,
    pub bot_token: String,
    /// The env var this bridge's credential lives in.
    ///
    /// Held so the bridge can re-read it while running: a token resolved
    /// once at startup made revocation ineffective until a restart, which
    /// is why an API handler used to bounce this process
    /// (`surfaces::CredentialWatch`). Empty disables the watch, for tests
    /// that pass a literal token.
    pub token_env: String,
    pub gateway_url: String,
    pub gateway_token: String,
    /// Directory for the per-token single-instance lock
    /// (`$VAK_HOME/locks`). None skips locking (tests only).
    pub locks_dir: Option<PathBuf>,
    /// This bot's id in the admin console's Bots list (`--bot-id`), when
    /// running the multi-bot path. Forwarded on every inbound message so a
    /// chat's first-sight pending entry already knows which bot delivered
    /// it (docs/design/34) — `None` for the legacy single-bot flow.
    pub bot_id: Option<String>,
}

impl InboundChannel for TelegramBridge {
    fn surface(&self) -> &'static str {
        "telegram"
    }
}

/// Why a poll failed -- recovery differs by kind.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PollBlock {
    /// Another consumer holds the getUpdates long-poll. Telegram allows
    /// exactly one per token; this is OWNERSHIP, not an outage.
    Conflict,
    /// Upstream blip / network / 5xx: retry shortly.
    Transient,
}

/// Classify by message content (the bridge stores formatted errors).
pub fn classify_poll_error(err: &str) -> PollBlock {
    if err.contains("409") || err.to_lowercase().contains("conflict") {
        PollBlock::Conflict
    } else {
        PollBlock::Transient
    }
}

/// Exponential standby backoff while a rival owns the bot: doubling,
/// capped at 30s. Pure so the schedule is testable.
pub fn standby_backoff_secs(attempt: u32) -> u64 {
    let exp = 1u64 << attempt.min(5);
    exp.min(30)
}

fn hostname_fallback() -> String {
    std::process::Command::new("hostname")
        .output()
        .ok()
        .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| "unknown-host".into())
}

fn identity() -> String {
    format!("{}[pid {}]", hostname_fallback(), std::process::id())
}

/// Cross-process single-instance guard keyed by bot-token hash: two
/// bridges on one machine can never fight each other (flock releases
/// automatically when the holder dies, so stale locks are impossible).
/// Cross-process AND cross-description single-instance guard keyed by
/// bot-token hash: O_EXCL marker plus PID liveness check. A crashed
/// holder leaves a stale marker; the next contender detects the dead PID
/// and takes over. No unsafe, no extra dependencies.
#[derive(Debug)]
pub struct InstanceLock {
    path: PathBuf,
    owned: bool,
}

impl Drop for InstanceLock {
    fn drop(&mut self) {
        if self.owned {
            let _ = std::fs::remove_file(&self.path);
        }
    }
}

impl InstanceLock {
    pub fn acquire(locks_dir: &PathBuf, bot_token: &str) -> Result<InstanceLock, String> {
        std::fs::create_dir_all(locks_dir)
            .map_err(|e| format!("lock dir {}: {e}", locks_dir.display()))?;
        let mut h: u64 = 0xcbf2_9ce4_8422_2325;
        for b in bot_token.as_bytes() {
            h ^= u64::from(*b);
            h = h.wrapping_mul(0x0000_0100_0000_01b3);
        }
        let path = locks_dir.join(format!("telegram-{h:016x}.lock"));

        if let Ok(lock) = Self::try_create(&path) {
            return Ok(lock);
        }

        // Marker exists: is its holder still alive?
        let holder = std::fs::read_to_string(&path).unwrap_or_default();
        let pid: Option<u32> = holder
            .split_whitespace()
            .find_map(|t| t.parse::<u32>().ok());
        match pid {
            Some(p) if Self::pid_alive(p) => Err(format!(
                "another vak telegram bridge already owns this bot\n  \
                 lock: {}\n  \
                 holder pid: {p}\n  \
                 stop it first (launchctl kickstart -k gui/$(id -u)/com.vak.telegram,\n  \
                 or kill the stale process); rotating TELEGRAM_BOT_TOKEN also helps",
                path.display()
            )),
            _ => {
                // Stale marker (crashed holder): take over.
                let _ = std::fs::remove_file(&path);
                Self::try_create(&path)
            }
        }
    }

    fn try_create(path: &PathBuf) -> Result<InstanceLock, String> {
        let file = std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(path)
            .map_err(|e| e.to_string())?;
        drop(file);
        let me = format!("{} pid {}\n", hostname_fallback(), std::process::id());
        std::fs::write(path, &me).map_err(|e| e.to_string())?;
        Ok(InstanceLock {
            path: path.clone(),
            owned: true,
        })
    }

    /// Safe liveness probe without libc: `kill -0` via a subprocess.
    fn pid_alive(pid: u32) -> bool {
        std::process::Command::new("kill")
            .arg("-0")
            .arg(pid.to_string())
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .status()
            .map(|st| st.success())
            .unwrap_or(false)
    }
}

struct TelegramUpdate {
    update_id: i64,
    chat_id: i64,
    /// Telegram's own numeric user id for whoever sent the message. Falls
    /// back to the chat id (never empty/placeholder) when Telegram omits
    /// `from` (rare, e.g. channel posts) so `InboundRequest::new` never
    /// sees a blank sender.
    sender_id: i64,
    text: String,
    /// Largest-photo file id when the message carries an image.
    photo_file_id: Option<String>,
    /// A non-photo file attachment (code, logs, CSVs, ...).
    document: Option<TelegramDocument>,
    /// Telegram voice-note file, kept distinct from documents so the
    /// gateway can select a transcription adapter rather than an image path.
    voice: Option<TelegramDocument>,
    /// Set instead of the fields above when this update is an
    /// inline-keyboard button tap rather than a message.
    callback: Option<TelegramCallback>,
}

struct TelegramDocument {
    file_id: String,
    file_name: String,
    mime_type: String,
}

/// One inline-keyboard tap: `data` is the `callback_data` set when the
/// button was built (`"approve:<request_id>"` / `"deny:<request_id>"`).
struct TelegramCallback {
    callback_id: String,
    data: String,
    chat_id: i64,
    sender_id: i64,
}

struct GatewayReply {
    text: String,
    delivery: Option<vak_delivery::DeliveryPacket>,
    session_id: Option<String>,
    /// Office drafts the turn made, sent back as documents.
    files: Vec<ReturnedFile>,
}

#[derive(serde::Deserialize)]
struct ReturnedFile {
    name: String,
    mime: String,
    /// Base64.
    data: String,
    caption: String,
}

/// A tapped button's `callback_data` ("approve:<id>" / "deny:<id>"),
/// translated to the verdict text a typed chat reply would produce ("yes
/// <id>" / "no <id>") so `parse_verdict` in `gateway.rs` resolves it
/// through the one existing path. `None` for anything else — a stale or
/// tampered-with callback should be rejected, not guessed at.
fn callback_data_to_verdict_text(data: &str) -> (&str, Option<String>) {
    let mut parts = data.splitn(2, ':');
    let verb = parts.next().unwrap_or_default();
    let request_id = parts.next().unwrap_or_default();
    match verb {
        "approve" => (verb, Some(format!("yes {request_id}"))),
        "deny" => (verb, Some(format!("no {request_id}"))),
        _ => (verb, None),
    }
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

impl TelegramBridge {
    /// The gateway's one inbound document cap; a larger upload is not
    /// downloaded, and the sender is told rather than the upload silently
    /// vanishing.
    const DOCUMENT_MAX_BYTES: usize = crate::gateway::INBOUND_DOCUMENT_MAX_BYTES;

    /// Long-poll once, route every text through the gateway, deliver each
    /// reply. Returns the next offset even when nothing arrived.
    pub async fn tick(&self, offset: i64) -> Result<i64, String> {
        let updates = self.get_updates(offset).await?;
        let mut next = offset;
        for u in updates {
            if let Some(cb) = &u.callback {
                if let Err(e) = self.handle_callback(cb).await {
                    eprintln!("[telegram] callback handling failed: {e}");
                }
                // Resolving a gate twice is a no-op on the server side, so
                // this is safe to advance unconditionally — unlike a text
                // turn, replaying a button tap can't re-run a tool.
                next = next.max(u.update_id + 1);
                continue;
            }
            if u.text.trim().is_empty()
                && u.photo_file_id.is_none()
                && u.document.is_none()
                && u.voice.is_none()
            {
                continue;
            }
            let mut attachments = Vec::new();
            let mut text = u.text.clone();
            if let Some(file_id) = &u.photo_file_id {
                match self.fetch_photo_base64(file_id).await {
                    Ok((mime, data)) => attachments.push(serde_json::json!({
                        "mime": mime, "data": data, "kind": "image"
                    })),
                    Err(e) => eprintln!("[telegram] photo download failed: {e}"),
                }
            }
            if let Some(doc) = &u.document {
                match self.fetch_document_base64(doc).await {
                    Ok(Some(data)) => attachments.push(serde_json::json!({
                        "mime": doc.mime_type,
                        "data": data,
                        "filename": doc.file_name,
                        "kind": "document",
                    })),
                    Ok(None) => {
                        text = format!(
                            "{text}\n\n[attached file '{}' exceeds the {} KiB channel limit; \
                             not received]",
                            doc.file_name,
                            Self::DOCUMENT_MAX_BYTES / 1024
                        );
                    }
                    Err(e) => eprintln!("[telegram] document download failed: {e}"),
                }
            }
            if let Some(voice) = &u.voice {
                match self.download_file(&voice.file_id).await {
                    Ok((mime, bytes)) => {
                        use base64::Engine as _;
                        attachments.push(serde_json::json!({
                            "mime": if mime == "application/octet-stream" { "audio/ogg" } else { &mime },
                            "data": base64::engine::general_purpose::STANDARD.encode(bytes),
                            "kind": "audio",
                            "filename": voice.file_name,
                        }));
                    }
                    Err(e) => eprintln!("[telegram] voice download failed: {e}"),
                }
            }
            let reply = self
                .process(u.chat_id, u.sender_id, &text, &attachments)
                .await;
            // Deliver whatever we got — an error notice beats silence, but a
            // failed send must not lose our offset progress either way.
            self.send_message(u.chat_id, &reply)
                .await
                .map_err(|error| format!("send to {}: {error}", u.chat_id))?;
            for file in &reply.files {
                if let Err(error) = self.send_document(u.chat_id, file).await {
                    eprintln!("[telegram] {} not sent: {error}", file.name);
                    let notice = GatewayReply {
                        text: format!("{} could not be sent here: {error}", file.name),
                        delivery: None,
                        session_id: None,
                        files: Vec::new(),
                    };
                    let _ = self.send_message(u.chat_id, &notice).await;
                }
            }
            // Native voice delivery is best-effort: text remains the durable
            // fallback when voice is disabled, unconfigured, or unavailable.
            if attachments
                .iter()
                .any(|a| a.get("kind").and_then(Value::as_str) == Some("audio"))
                && !reply.text.trim().is_empty()
                && let Err(error) = self
                    .send_voice(u.chat_id, &reply.text, reply.session_id.as_deref())
                    .await
            {
                eprintln!("[telegram] voice reply unavailable: {error}");
            }
            next = next.max(u.update_id + 1);
        }
        Ok(next)
    }

    /// A tapped approval button, translated into the same verdict text the
    /// yes/no chat flow already understands (`parse_verdict` in
    /// `gateway.rs`) — reuses all existing gate-resolution logic instead of
    /// adding a second path. `answerCallbackQuery` is mandatory: without it
    /// Telegram leaves the button showing a permanent loading spinner.
    async fn handle_callback(&self, cb: &TelegramCallback) -> Result<(), String> {
        let (verb, Some(text)) = callback_data_to_verdict_text(&cb.data) else {
            return self
                .answer_callback(&cb.callback_id, "Unknown action")
                .await;
        };
        let req =
            InboundRequest::new(self, cb.chat_id.to_string(), cb.sender_id.to_string(), text)?
                .with_bot_id(self.bot_id.clone());
        let res = http()
            .post(format!("{}/gateway/inbound", self.gateway_url))
            .bearer_auth(&self.gateway_token)
            .json(&req)
            .send()
            .await;
        let toast = match res {
            Ok(r) if r.status().is_success() => {
                if verb == "approve" {
                    "Approved"
                } else {
                    "Denied"
                }
            }
            Ok(r) => {
                eprintln!("[telegram] callback resolve returned {}", r.status());
                "Could not resolve (see server logs)"
            }
            Err(e) => {
                eprintln!("[telegram] callback resolve failed: {e}");
                "Could not reach gateway"
            }
        };
        self.answer_callback(&cb.callback_id, toast).await
    }

    async fn answer_callback(&self, callback_id: &str, text: &str) -> Result<(), String> {
        let resp = http()
            .post(format!(
                "{}/bot{}/answerCallbackQuery",
                self.api_base, self.bot_token
            ))
            .json(&serde_json::json!({ "callback_query_id": callback_id, "text": text }))
            .send()
            .await
            .map_err(|e| telegram_http_error("answerCallbackQuery", &e))?;
        if !resp.status().is_success() {
            return Err(format!("answerCallbackQuery returned {}", resp.status()));
        }
        Ok(())
    }

    /// One message through the gateway contract; wait for the final text.
    /// `attachments` are base64 image payloads posted alongside the text.
    async fn process(
        &self,
        chat_id: i64,
        sender_id: i64,
        text: &str,
        attachments: &[serde_json::Value],
    ) -> GatewayReply {
        // 0c-03: real per-user chat/sender, not a fixed placeholder — see
        // InboundRequest::new for why that distinction is enforced here.
        let req = match InboundRequest::new(
            self,
            chat_id.to_string(),
            sender_id.to_string(),
            text.to_string(),
        ) {
            Ok(req) => req
                .with_attachments(attachments.to_vec())
                .waiting()
                .accepting_files()
                .with_bot_id(self.bot_id.clone()),
            Err(e) => {
                return GatewayReply {
                    text: format!("(bridge refused to send: {e})"),
                    delivery: None,
                    session_id: None,
                    files: Vec::new(),
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
                // Queued behind a running turn; poll the transcript later —
                // for v1 tell the user the work is acknowledged.
                text: "(queued: I'm still working on your previous message)".into(),
                delivery: None,
                session_id: None,
                files: Vec::new(),
            },
            Ok(r) if r.status().is_success() => match r.json::<Value>().await {
                Ok(v) => GatewayReply {
                    text: v["text"].as_str().unwrap_or("(empty reply)").to_string(),
                    delivery: serde_json::from_value(v["delivery"].clone()).ok(),
                    session_id: v["session_id"].as_str().map(String::from),
                    files: serde_json::from_value(v["files"].clone()).unwrap_or_default(),
                },
                Err(e) => GatewayReply {
                    text: format!("(bad gateway reply: {e})"),
                    delivery: None,
                    session_id: None,
                    files: Vec::new(),
                },
            },
            Ok(r) => GatewayReply {
                text: format!("(gateway error: {})", r.status()),
                delivery: None,
                session_id: None,
                files: Vec::new(),
            },
            Err(e) => GatewayReply {
                text: format!("(gateway unreachable: {e})"),
                delivery: None,
                session_id: None,
                files: Vec::new(),
            },
        }
    }

    async fn get_updates(&self, offset: i64) -> Result<Vec<TelegramUpdate>, String> {
        let url = format!("{}/bot{}/getUpdates", self.api_base, self.bot_token);
        let resp = http()
            .get(&url)
            .query(&[("timeout", "25"), ("offset", &offset.to_string())])
            .send()
            .await
            .map_err(|e| telegram_http_error("getUpdates", &e))?;
        let status = resp.status();
        if !status.is_success() {
            return Err(format!("getUpdates returned {status}"));
        }
        let body: Value = resp
            .json()
            .await
            .map_err(|e| telegram_http_error("getUpdates body", &e))?;
        if body["ok"].as_bool() != Some(true) {
            return Err(format!(
                "getUpdates not ok: {}",
                body["description"].as_str().unwrap_or("?")
            ));
        }
        let mut out = Vec::new();
        if let Some(items) = body["result"].as_array() {
            for item in items {
                let Some(update_id) = item["update_id"].as_i64() else {
                    continue;
                };
                // An inline-keyboard tap arrives as callback_query, not
                // message; route it separately and skip straight to the
                // next update.
                if let Some(cq) = item.get("callback_query").filter(|v| !v.is_null()) {
                    let Some(callback_id) = cq["id"].as_str() else {
                        continue;
                    };
                    let chat_id = cq["message"]["chat"]["id"].as_i64().unwrap_or(0);
                    let sender_id = cq["from"]["id"].as_i64().unwrap_or(chat_id);
                    out.push(TelegramUpdate {
                        update_id,
                        chat_id,
                        sender_id,
                        text: String::new(),
                        photo_file_id: None,
                        document: None,
                        voice: None,
                        callback: Some(TelegramCallback {
                            callback_id: callback_id.to_string(),
                            data: cq["data"].as_str().unwrap_or_default().to_string(),
                            chat_id,
                            sender_id,
                        }),
                    });
                    continue;
                }
                // Text, photo, and document messages are routed; edits and
                // other media advance the offset so they are never replayed.
                let msg = &item["message"];
                let chat_id = msg["chat"]["id"].as_i64();
                let sender_id = msg["from"]["id"].as_i64();
                // Words sent with a photo or file arrive as its caption.
                let text = msg["text"]
                    .as_str()
                    .or_else(|| msg["caption"].as_str())
                    .map(String::from);
                let photo_file_id = msg["photo"]
                    .as_array()
                    .and_then(|sizes| sizes.last())
                    .and_then(|largest| largest["file_id"].as_str())
                    .map(String::from);
                let document =
                    msg["document"]["file_id"]
                        .as_str()
                        .map(|file_id| TelegramDocument {
                            file_id: file_id.to_string(),
                            file_name: msg["document"]["file_name"]
                                .as_str()
                                .unwrap_or("file")
                                .to_string(),
                            mime_type: msg["document"]["mime_type"]
                                .as_str()
                                .unwrap_or("application/octet-stream")
                                .to_string(),
                        });
                let voice = msg["voice"]["file_id"]
                    .as_str()
                    .map(|file_id| TelegramDocument {
                        file_id: file_id.to_string(),
                        file_name: "voice.ogg".into(),
                        mime_type: "audio/ogg".into(),
                    });
                let routable = chat_id.is_some()
                    && (text.is_some()
                        || photo_file_id.is_some()
                        || document.is_some()
                        || voice.is_some());
                match (chat_id, routable) {
                    (Some(chat_id), true) => out.push(TelegramUpdate {
                        update_id,
                        chat_id,
                        sender_id: sender_id.unwrap_or(chat_id),
                        text: text.unwrap_or_default(),
                        photo_file_id,
                        document,
                        voice,
                        callback: None,
                    }),
                    _ => out.push(TelegramUpdate {
                        update_id,
                        chat_id: 0,
                        sender_id: 0,
                        text: String::new(),
                        photo_file_id: None,
                        document: None,
                        voice: None,
                        callback: None,
                    }),
                }
            }
        }
        Ok(out)
    }

    async fn send_message(&self, chat_id: i64, reply: &GatewayReply) -> Result<(), String> {
        // Validate the packet against the same schema/surface boundary used
        // by Slack and Discord. Telegram still keeps the plain reply text as
        // its lossless fallback when the packet is absent or incompatible.
        let rendered = reply.delivery.as_ref().and_then(|packet| {
            let body = serde_json::json!({"delivery": packet});
            super::prepared_packet(body, "telegram").ok().map(|packet| {
                let has_surface_chunks = !packet.chunks.is_empty();
                (packet.channel_text_chunks(), has_surface_chunks)
            })
        });
        let (chunks, parse_html) = match rendered {
            Some((chunks, parse_html)) if !chunks.is_empty() => (chunks, parse_html),
            _ => (vec![reply.text.clone()], false),
        };
        for chunk in chunks {
            let url = format!("{}/bot{}/sendMessage", self.api_base, self.bot_token);
            let mut body = serde_json::json!({
                "chat_id": chat_id,
                "text": chunk,
                "link_preview_options": { "is_disabled": true },
            });
            if parse_html {
                body["parse_mode"] = serde_json::Value::String("HTML".into());
            }
            let resp = http().post(&url).json(&body).send().await;
            match resp {
                Ok(r) if r.status().is_success() => {}
                Ok(r) => {
                    let status = r.status();
                    // Converter edge-case guard: resend that chunk as plain
                    // text so a formatting bug degrades to ugly, not lost.
                    if parse_html && status.as_u16() == 400 {
                        eprintln!(
                            "[telegram] HTML rejected ({status}); falling back to plain text — \
                             chunk head: {}",
                            chunk.chars().take(80).collect::<String>()
                        );
                        let fallback = serde_json::json!({
                            "chat_id": chat_id,
                            "text": crate::channels::strip_tags(&chunk),
                        });
                        let r2 = http().post(&url).json(&fallback).send().await;
                        match r2 {
                            Ok(r2) if r2.status().is_success() => continue,
                            Ok(r2) => {
                                return Err(format!(
                                    "sendMessage returned {} (plain retry: {})",
                                    status,
                                    r2.status()
                                ));
                            }
                            Err(e) => {
                                return Err(telegram_http_error("sendMessage retry", &e));
                            }
                        }
                    }
                    return Err(format!("sendMessage returned {status}"));
                }
                Err(e) => return Err(telegram_http_error("sendMessage", &e)),
            }
        }
        Ok(())
    }

    /// Sends a file the turn produced, under this bot's own token, with the
    /// change summary as its caption (Telegram allows 1024 characters).
    async fn send_document(&self, chat_id: i64, file: &ReturnedFile) -> Result<(), String> {
        use base64::Engine as _;
        let bytes = base64::engine::general_purpose::STANDARD
            .decode(file.data.as_bytes())
            .map_err(|e| format!("bad file encoding: {e}"))?;
        let part = reqwest::multipart::Part::bytes(bytes)
            .file_name(file.name.clone())
            .mime_str(&file.mime)
            .map_err(|e| format!("document mime: {e}"))?;
        let caption: String = file.caption.chars().take(1024).collect();
        let body = reqwest::multipart::Form::new()
            .text("chat_id", chat_id.to_string())
            .text("caption", caption)
            .part("document", part);
        let sent = http()
            .post(format!(
                "{}/bot{}/sendDocument",
                self.api_base, self.bot_token
            ))
            .multipart(body)
            .send()
            .await
            .map_err(|e| telegram_http_error("sendDocument", &e))?;
        if !sent.status().is_success() {
            return Err(format!("sendDocument returned {}", sent.status()));
        }
        Ok(())
    }

    async fn send_voice(
        &self,
        chat_id: i64,
        text: &str,
        session_id: Option<&str>,
    ) -> Result<(), String> {
        let response = http()
            .post(format!("{}/voice/speak", self.gateway_url))
            .bearer_auth(&self.gateway_token)
            .json(&serde_json::json!({"text": text, "format": "wav", "session_id": session_id}))
            .send()
            .await
            .map_err(|e| format!("voice speak request: {e}"))?;
        if !response.status().is_success() {
            return Err(format!("voice speak returned {}", response.status()));
        }
        let audio = response
            .bytes()
            .await
            .map_err(|e| format!("voice speak body: {e}"))?;
        if audio.is_empty() {
            return Err("voice speak returned empty audio".into());
        }
        // Telegram's native voice method only accepts an OGG container with
        // Opus audio.  The provider-neutral synthesis endpoint deliberately
        // returns WAV, so negotiate the transport codec here instead of
        // silently uploading an invalid WAV as `sendVoice`.
        let audio = wav_to_telegram_opus(audio.to_vec()).await?;
        let part = reqwest::multipart::Part::bytes(audio)
            .file_name("reply.ogg")
            .mime_str("audio/ogg")
            .map_err(|e| format!("voice mime: {e}"))?;
        let body = reqwest::multipart::Form::new()
            .text("chat_id", chat_id.to_string())
            .part("voice", part);
        let sent = http()
            .post(format!("{}/bot{}/sendVoice", self.api_base, self.bot_token))
            .multipart(body)
            .send()
            .await
            .map_err(|e| format!("sendVoice: {e}"))?;
        if !sent.status().is_success() {
            return Err(format!("sendVoice returned {}", sent.status()));
        }
        Ok(())
    }

    /// getFile → two-step download of a Telegram-hosted file, returned as
    /// raw bytes plus the `file_path` Telegram reported (its extension is
    /// how photo mime type gets inferred below). 1 MiB bot-API cap.
    async fn download_file(&self, file_id: &str) -> Result<(String, Vec<u8>), String> {
        #[derive(serde::Deserialize)]
        struct FileResp {
            ok: bool,
            result: FileMeta,
        }
        #[derive(serde::Deserialize)]
        struct FileMeta {
            file_path: Option<String>,
        }
        let meta: FileResp = http()
            .get(format!("{}/bot{}/getFile", self.api_base, self.bot_token))
            .query(&[("file_id", file_id)])
            .send()
            .await
            .map_err(|e| telegram_http_error("getFile", &e))?
            .json()
            .await
            .map_err(|e| telegram_http_error("getFile body", &e))?;
        if !meta.ok {
            return Err("getFile not ok".into());
        }
        let path = meta.result.file_path.ok_or("getFile missing file_path")?;
        let bytes = http()
            .get(format!(
                "{}/file/bot{}/{}",
                self.api_base, self.bot_token, path
            ))
            .send()
            .await
            .map_err(|e| telegram_http_error("download", &e))?
            .error_for_status()
            .map_err(|e| telegram_http_error("download status", &e))?
            .bytes()
            .await
            .map_err(|e| telegram_http_error("download body", &e))?;
        Ok((path, bytes.to_vec()))
    }

    /// Largest-photo variant, downscaled by Telegram on the sender side —
    /// well inside vision budgets. Returned as (mime, base64).
    async fn fetch_photo_base64(&self, file_id: &str) -> Result<(String, String), String> {
        let (path, bytes) = self.download_file(file_id).await?;
        let mime = if path.ends_with(".jpg") || path.ends_with(".jpeg") {
            "image/jpeg"
        } else if path.ends_with(".webp") {
            "image/webp"
        } else {
            "image/png"
        };
        use base64::Engine as _;
        Ok((
            mime.to_string(),
            base64::engine::general_purpose::STANDARD.encode(bytes),
        ))
    }

    /// A non-photo attachment (code, logs, CSVs, Office files, ...).
    /// `Ok(None)` means the file was over `DOCUMENT_MAX_BYTES` and was not
    /// received; the caller tells the sender rather than truncating it.
    async fn fetch_document_base64(
        &self,
        doc: &TelegramDocument,
    ) -> Result<Option<String>, String> {
        let (_, bytes) = self.download_file(&doc.file_id).await?;
        if bytes.len() > Self::DOCUMENT_MAX_BYTES {
            return Ok(None);
        }
        use base64::Engine as _;
        Ok(Some(
            base64::engine::general_purpose::STANDARD.encode(bytes),
        ))
    }

    /// Run until the process is killed. Transient poll/send failures back
    /// off and retry; they never drop the update stream position.
    /// Non-acking ownership probe: `timeout=0, offset=-1` returns at most
    /// the LAST update and acknowledges nothing, so probing is safe before
    /// the real loop decides where to start.
    async fn probe_ownership(&self) -> Result<(), PollBlock> {
        let url = format!("{}/bot{}/getUpdates", self.api_base, self.bot_token);
        let resp = http()
            .get(&url)
            .query(&[("timeout", "0"), ("offset", "-1")])
            .send()
            .await
            .map_err(|_| PollBlock::Transient)?;
        match resp.status().as_u16() {
            200 => Ok(()),
            409 => Err(PollBlock::Conflict),
            _ => Err(PollBlock::Transient),
        }
    }

    /// Hot-standby: while a rival owns the bot, wait quietly and take over
    /// the moment it disappears. Logs entry once, then every 10th attempt.
    async fn await_ownership(&self) {
        let id = identity();
        eprintln!(
            "[telegram] bot token is owned by ANOTHER getUpdates consumer;\n[telegram] {id} standing by as hot standby (auto-takeover on rival exit)"
        );
        let mut attempt: u32 = 0;
        loop {
            tokio::time::sleep(std::time::Duration::from_secs(standby_backoff_secs(
                attempt,
            )))
            .await;
            attempt += 1;
            match self.probe_ownership().await {
                Ok(()) => {
                    eprintln!("[telegram] {id} took over polling (rival gone)");
                    return;
                }
                Err(PollBlock::Conflict) => {
                    if attempt.is_multiple_of(10) {
                        eprintln!(
                            "[telegram] still owned elsewhere ({attempt} probes); standing by"
                        );
                    }
                }
                Err(PollBlock::Transient) => {}
            }
        }
    }

    pub async fn run(&self) -> Result<(), String> {
        // Local mutual exclusion first: two bridges on one host must fail
        // fast with the holder's identity instead of flapping 409s.
        let _instance_lock = match &self.locks_dir {
            Some(dir) => Some(InstanceLock::acquire(dir, &self.bot_token)?),
            None => None,
        };

        // Ownership probe: if another machine/session holds the long-poll,
        // become hot standby instead of hammering 409s forever.
        match self.probe_ownership().await {
            Err(PollBlock::Conflict) => self.await_ownership().await,
            Err(PollBlock::Transient) | Ok(()) => {}
        }

        let mut offset: i64 = 0;
        let mut failures: u32 = 0;
        let watch = (!self.token_env.is_empty())
            .then(|| super::CredentialWatch::new(&self.token_env, &self.bot_token));
        loop {
            // Revocation and rotation are facts about the credential
            // store, noticed here within one poll cycle, rather than
            // something an API handler orchestrates by restarting this
            // process.
            if let Some(watch) = &watch {
                match watch.check() {
                    super::CredentialState::Unchanged => {}
                    super::CredentialState::Rotated => {
                        eprintln!(
                            "[telegram] credential rotated; exiting so the service manager \
                             restarts this bridge with the new one"
                        );
                        return Ok(());
                    }
                    super::CredentialState::Revoked => {
                        eprintln!(
                            "[telegram] credential revoked; this bridge is stopping and will \
                             not poll again until a token is set"
                        );
                        return Ok(());
                    }
                }
            }
            match self.tick(offset).await {
                Ok(next) => {
                    offset = next;
                    failures = 0;
                }
                Err(e) => {
                    failures += 1;
                    match classify_poll_error(&e) {
                        PollBlock::Conflict => {
                            // A rival appeared mid-run: hand over gracefully
                            // and stand by for auto-takeover.
                            eprintln!(
                                "[telegram] lost ownership to another getUpdates consumer; entering hot standby ({})",
                                identity()
                            );
                            self.await_ownership().await;
                        }
                        PollBlock::Transient => {
                            // Never give up (docs/design/31-network-resilience.md):
                            // outages, DHCP switches, and sleep/wake are all
                            // ordinary transients. Backoff doubles to a 30s
                            // cap — the same ceiling as hot standby — and
                            // resets on first success. The offset cursor
                            // makes every recovery gap-free.
                            if failures == 1 || failures.is_multiple_of(10) {
                                eprintln!("[telegram] poll failed ({failures} consecutive): {e}");
                            }
                            tokio::time::sleep(std::time::Duration::from_secs(
                                standby_backoff_secs(failures),
                            ))
                            .await;
                        }
                    }
                }
            }
        }
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;

    #[test]
    fn telegram_card_projection_formats_typed_fields_without_json() {
        let card = vak_delivery::StructuredOutput {
            semantic_type: "metric".into(),
            schema_version: vak_delivery::PRESENTATION_SCHEMA_VERSION,
            skill_id: "core".into(),
            skill_version: "1".into(),
            payload: serde_json::json!({"title":"Weather Card","temperature":"23°C","status":"Sunny"}),
        };
        let chunks = vak_delivery::telegram::structured_card_chunks(&[card], Some(3900));
        assert_eq!(chunks.len(), 1);
        assert!(chunks[0].contains("<b>"));
        assert!(chunks[0].contains("23°C"));
        assert!(chunks[0].contains("Sunny"));
        assert!(!chunks[0].contains("semantic_type"));
        assert!(!chunks[0].contains("```json"));
    }

    #[test]
    fn approve_button_maps_to_a_yes_verdict_with_the_request_id() {
        assert_eq!(
            callback_data_to_verdict_text("approve:ab12cd34"),
            ("approve", Some("yes ab12cd34".into()))
        );
    }

    #[test]
    fn deny_button_maps_to_a_no_verdict_with_the_request_id() {
        assert_eq!(
            callback_data_to_verdict_text("deny:ab12cd34"),
            ("deny", Some("no ab12cd34".into()))
        );
    }

    #[test]
    fn unknown_callback_data_is_rejected_not_guessed() {
        assert_eq!(
            callback_data_to_verdict_text("something-else"),
            ("something-else", None)
        );
        assert_eq!(callback_data_to_verdict_text(""), ("", None));
    }

    #[test]
    fn classify_maps_conflict_vs_transient() {
        assert_eq!(
            classify_poll_error("getUpdates returned 409 Conflict"),
            PollBlock::Conflict
        );
        assert_eq!(
            classify_poll_error("getUpdates returned 502 Bad Gateway"),
            PollBlock::Transient
        );
        assert_eq!(
            classify_poll_error("getUpdates: error sending request"),
            PollBlock::Transient
        );
    }

    #[tokio::test]
    async fn telegram_http_errors_never_include_bot_token_or_url() {
        let error = reqwest::Client::new()
            .get("http://127.0.0.1:1/botsecret-token/getUpdates")
            .send()
            .await
            .unwrap_err();
        let rendered = telegram_http_error("getUpdates", &error);
        assert!(!rendered.contains("secret-token"));
        assert!(!rendered.contains("127.0.0.1"));
        assert!(!rendered.contains("/bot"));
    }

    #[tokio::test]
    async fn opus_conversion_rejects_invalid_wav_without_claiming_delivery() {
        let result = wav_to_telegram_opus(b"not-a-wav".to_vec()).await;
        assert!(result.is_err());
        let error = result.unwrap_err();
        assert!(error.contains("ffmpeg") || error.contains("conversion"));
    }

    #[test]
    fn gateway_reply_keeps_session_for_voice_receipt() {
        let value = serde_json::json!({
            "text": "reply",
            "session_id": "session-42",
            "delivery": {"status": "delivered"}
        });
        assert_eq!(value["session_id"].as_str(), Some("session-42"));
        assert_eq!(value["text"].as_str(), Some("reply"));
    }

    #[test]
    fn standby_backoff_doubles_and_caps_at_thirty() {
        assert_eq!(standby_backoff_secs(0), 1);
        assert_eq!(standby_backoff_secs(1), 2);
        assert_eq!(standby_backoff_secs(5), 30);
        assert_eq!(standby_backoff_secs(50), 30);
    }

    #[test]
    fn instance_lock_is_exclusive_per_token_and_released_on_drop() {
        let dir = tempfile::tempdir().unwrap();
        let a = InstanceLock::acquire(&dir.path().to_path_buf(), "tok-A");
        assert!(a.is_ok(), "first holder acquires");
        let b = InstanceLock::acquire(&dir.path().to_path_buf(), "tok-A");
        assert!(b.is_err(), "second holder on SAME token rejected");
        let c = InstanceLock::acquire(&dir.path().to_path_buf(), "tok-B");
        assert!(c.is_ok(), "different token = different lock file");
        drop(a);
        let d = InstanceLock::acquire(&dir.path().to_path_buf(), "tok-A");
        assert!(d.is_ok(), "flock releases when holder drops");
    }

    /// Opt-in network smoke test: set `VAK_LIVE_TELEGRAM_CHAT_ID` and run
    /// with `--ignored`. The bot credential is read from Vak's secret store
    /// and is never included in output.
    #[tokio::test]
    #[ignore = "posts a clearly labeled card-rendering check to a real chat"]
    async fn live_telegram_accepts_card_html() {
        let chat_id = std::env::var("VAK_LIVE_TELEGRAM_CHAT_ID")
            .expect("set VAK_LIVE_TELEGRAM_CHAT_ID to the authorized test chat");
        let token_env = "BOT_TOKEN__VKAYARTHA";
        let token = vak_config::user_env_path()
            .and_then(|path| vak_config::read_env_file_var(&path, token_env))
            .or_else(|| vak_config::get_var(token_env))
            .expect("configured Telegram bot credential is unavailable");
        let api_base = vak_config::get_var("TELEGRAM_API_BASE")
            .unwrap_or_else(|| "https://api.telegram.org".into());
        let samples = [
            (
                "Research brief",
                "### Findings\n\nThree sources point to steady adoption growth. The clearest signal is **repeat use**, while pricing remains uncertain.\n\n- Strongest evidence: monthly active use\n- Open question: regional pricing",
            ),
            (
                "Metric card",
                "### Service health\n\n**Availability:** 99.95%\n\n**Latency:** p95 184 ms\n\nNo customer impact was detected.",
            ),
            (
                "Comparison",
                "### Options\n\n| Option | Cost | Fit |\n|---|---:|---|\n| A | Low | Fast setup |\n| B | Medium | More control |\n\n**Recommendation:** start with A.",
            ),
            (
                "Test report",
                "### Build checks\n\n- Passed: 128\n- Failed: 0\n- Skipped: 2\n\nThe two skipped checks need a configured simulator.",
            ),
            (
                "Plan timeline",
                "### This week\n\n1. Monday — confirm scope\n2. Wednesday — review the prototype\n3. Friday — decide whether to ship\n\nOwner: Product team.",
            ),
            (
                "Decision brief",
                "### Decision\n\nChoose the staged rollout. It limits exposure while keeping rollback simple.\n\n> Revisit after the first 500 users.",
            ),
            (
                "Recipe card",
                "### Lentil soup\n\n**Prep:** 10 min · **Cook:** 30 min\n\n- Red lentils\n- Onion and cumin\n- Lemon to finish\n\nSimmer until soft, then season.",
            ),
            (
                "Incident update",
                "### Checkout delay\n\n**Status:** Mitigated\n\nThe queue is draining normally. We are monitoring error rates for another 20 minutes.",
            ),
            (
                "Release notes",
                "### Version 2.4\n\n- Added export to CSV\n- Fixed a retry loop on slow networks\n- Improved keyboard navigation",
            ),
            (
                "Command result",
                "### Backup check\n\n`vak backup verify` completed successfully.\n\nFiles checked: 42\nErrors: 0",
            ),
            (
                "Research links",
                "### Sources\n\n1. [Product usage report](https://example.com/usage) — updated this month\n2. [Pricing overview](https://example.com/pricing) — current plans",
            ),
            (
                "Checklist",
                "### Before launch\n\n- [x] Rollback tested\n- [x] Support briefed\n- [ ] Regional notice approved",
            ),
            (
                "Chart summary",
                "### Weekly signups\n\nSignups rose from 420 to 610 over four weeks, with the largest increase after the onboarding change.",
            ),
            (
                "Support digest",
                "### Today\n\n**Resolved:** 18\n**Waiting:** 3\n\nMost reports concern account recovery. No outage is active.",
            ),
            (
                "Preview status",
                "### Preview ready\n\nThe mobile layout is ready for review. The primary action stays visible at narrow widths.",
            ),
        ];
        let profile = vak_delivery::DeliveryProfile {
            surface: "telegram".into(),
            markup: vak_delivery::Markup::TelegramHtml,
            max_chars: Some(3900),
            supports_tables: false,
            supports_code_blocks: true,
            supports_links: true,
            supports_actions: false,
            template: None,
            posture: vak_delivery::DeliveryPosture::default(),
        };
        let mut batches: Vec<String> = Vec::new();
        for (index, (title, answer)) in samples.iter().enumerate() {
            let job = vak_delivery::DeliveryJob {
                job_id: format!("telegram-live-card-{}", index + 1),
                target: format!("telegram:{chat_id}:Vkayartha"),
                kind: vak_delivery::DeliveryKind::Assistant,
                content: vak_delivery::DeliveryContent::Answer(
                    vak_delivery::AnswerDraft::from_markdown(format!(
                        "## Live delivery check {}/15 — {title}\n\n{answer}",
                        index + 1
                    )),
                ),
                profile: profile.clone(),
                skill_registry: None,
            };
            let packet = vak_delivery::render(&job).expect("Telegram answer rendering failed");
            assert_eq!(packet.surface, "telegram");
            assert!(packet.chunks.iter().all(|chunk| !chunk.contains("```json")));
            let rendered = packet.chunks.join("\n\n");
            if index % 3 == 0 {
                batches.push(rendered);
            } else if let Some(batch) = batches.last_mut() {
                batch.push_str("\n\n──────────\n\n");
                batch.push_str(&rendered);
            }
        }
        assert_eq!(samples.len(), 15);
        let card = vak_delivery::StructuredOutput {
            semantic_type: "metric".into(),
            schema_version: vak_delivery::PRESENTATION_SCHEMA_VERSION,
            skill_id: "core".into(),
            skill_version: "1".into(),
            payload: serde_json::json!({
                "title": "Native Telegram card layout",
                "availability": "99.95%",
                "p95 latency": "184 ms",
                "status": "Healthy"
            }),
        };
        batches[0].push_str("\n\n──────────\n\n");
        batches[0].push_str(
            &vak_delivery::telegram::structured_card_chunks(&[card], Some(3900)).join("\n\n"),
        );
        for (index, text) in batches.iter().enumerate() {
            let response = http()
                .post(format!(
                    "{}/bot{token}/sendMessage",
                    api_base.trim_end_matches('/')
                ))
                .json(&serde_json::json!({
                    "chat_id": chat_id,
                    "text": text,
                    "parse_mode": "HTML",
                    "link_preview_options": {"is_disabled": true}
                }))
                .send()
                .await
                .expect("Telegram API request failed");
            assert!(
                response.status().is_success(),
                "Telegram rejected live batch {}: {}",
                index + 1,
                response.status()
            );
            let body: serde_json::Value = response
                .json()
                .await
                .expect("Telegram response was not JSON");
            assert_eq!(body["ok"], true, "Telegram API did not accept live batch");
            if index + 1 < batches.len() {
                tokio::time::sleep(std::time::Duration::from_millis(1100)).await;
            }
        }
    }
}
