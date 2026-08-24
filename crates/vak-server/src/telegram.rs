//! Telegram surface adapter (docs/design/22-gateway.md G1): long-polls the
//! Bot API and bridges messages through `POST /gateway/inbound`, then
//! delivers the final assistant text back via `sendMessage`.
//!
//! The gateway stays transport-agnostic; this client runs anywhere it can
//! reach both Telegram and a vak-server — laptop, VPS, sidecar. Launched via
//! `vakcoder telegram --server URL --token GATEWAY_TOKEN` with
//! `TELEGRAM_BOT_TOKEN` in the environment (.env included).

use serde_json::Value;
use std::path::PathBuf;

pub struct TelegramBridge {
    /// Bot API base, e.g. `https://api.telegram.org`. Overridable for
    /// self-hosted relays and tests via `TELEGRAM_API_BASE`.
    pub api_base: String,
    pub bot_token: String,
    pub gateway_url: String,
    pub gateway_token: String,
    /// Directory for the per-token single-instance lock
    /// (`$VAKCODER_HOME/locks`). None skips locking (tests only).
    pub locks_dir: Option<PathBuf>,
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
                "another vakcoder telegram bridge already owns this bot\n  \
                 lock: {}\n  \
                 holder pid: {p}\n  \
                 stop it first (launchctl kickstart -k gui/$(id -u)/com.vakcoder.telegram,\n  \
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
    text: String,
    /// Largest-photo file id when the message carries an image.
    photo_file_id: Option<String>,
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
    /// Long-poll once, route every text through the gateway, deliver each
    /// reply. Returns the next offset even when nothing arrived.
    pub async fn tick(&self, offset: i64) -> Result<i64, String> {
        let updates = self.get_updates(offset).await?;
        let mut next = offset;
        for u in updates {
            if u.text.trim().is_empty() && u.photo_file_id.is_none() {
                continue;
            }
            let mut attachments = Vec::new();
            if let Some(file_id) = &u.photo_file_id {
                match self.fetch_photo_base64(file_id).await {
                    Ok((mime, data)) => attachments.push(serde_json::json!({
                        "mime": mime, "data": data
                    })),
                    Err(e) => eprintln!("[telegram] photo download failed: {e}"),
                }
            }
            let reply = self.process(u.chat_id, &u.text, &attachments).await;
            // Deliver whatever we got — an error notice beats silence, but a
            // failed send must not lose our offset progress either way.
            if let Err(e) = self.send_message(u.chat_id, &reply).await {
                eprintln!("[telegram] send to {chat}: {e}", chat = u.chat_id);
            }
            next = next.max(u.update_id + 1);
        }
        Ok(next)
    }

    /// One message through the gateway contract; wait for the final text.
    /// `attachments` are base64 image payloads posted alongside the text.
    async fn process(&self, chat_id: i64, text: &str, attachments: &[serde_json::Value]) -> String {
        let res = http()
            .post(format!("{}/gateway/inbound", self.gateway_url))
            .bearer_auth(&self.gateway_token)
            .json(&serde_json::json!({
                "surface": "telegram",
                "chat": chat_id.to_string(),
                "text": text,
                "wait": true,
                "sender": "telegram",
                "attachments": attachments,
            }))
            .send()
            .await;
        match res {
            Ok(r) if r.status().as_u16() == 202 => {
                // Queued behind a running turn; poll the transcript later —
                // for v1 tell the user the work is acknowledged.
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

    async fn get_updates(&self, offset: i64) -> Result<Vec<TelegramUpdate>, String> {
        let url = format!("{}/bot{}/getUpdates", self.api_base, self.bot_token);
        let resp = http()
            .get(&url)
            .query(&[("timeout", "25"), ("offset", &offset.to_string())])
            .send()
            .await
            .map_err(|e| format!("getUpdates: {e}"))?;
        let status = resp.status();
        if !status.is_success() {
            return Err(format!("getUpdates returned {status}"));
        }
        let body: Value = resp
            .json()
            .await
            .map_err(|e| format!("getUpdates body: {e}"))?;
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
                // Text and photo messages are routed; edits, callbacks and
                // other media advance the offset so they are never replayed.
                let msg = &item["message"];
                let chat_id = msg["chat"]["id"].as_i64();
                let text = msg["text"].as_str().map(String::from);
                let photo_file_id = msg["photo"]
                    .as_array()
                    .and_then(|sizes| sizes.last())
                    .and_then(|largest| largest["file_id"].as_str())
                    .map(String::from);
                let routable = chat_id.is_some() && (text.is_some() || photo_file_id.is_some());
                match (chat_id, routable) {
                    (Some(chat_id), true) => out.push(TelegramUpdate {
                        update_id,
                        chat_id,
                        text: text.unwrap_or_default(),
                        photo_file_id,
                    }),
                    _ => out.push(TelegramUpdate {
                        update_id,
                        chat_id: 0,
                        text: String::new(),
                        photo_file_id: None,
                    }),
                }
            }
        }
        Ok(out)
    }

    async fn send_message(&self, chat_id: i64, text: &str) -> Result<(), String> {
        // Markdown in, Telegram HTML out; chunks never cut inside a tag.
        let html = crate::channels::markdown_to_telegram_html(text);
        for chunk in crate::channels::split_html_chunks(&html, 4000) {
            let url = format!("{}/bot{}/sendMessage", self.api_base, self.bot_token);
            let body = serde_json::json!({
                "chat_id": chat_id,
                "text": chunk,
                "parse_mode": "HTML",
                "link_preview_options": { "is_disabled": true },
            });
            let resp = http().post(&url).json(&body).send().await;
            match resp {
                Ok(r) if r.status().is_success() => {}
                Ok(r) => {
                    let status = r.status();
                    // Converter edge-case guard: resend that chunk as plain
                    // text so a formatting bug degrades to ugly, not lost.
                    if status.as_u16() == 400 {
                        eprintln!(
                            "[telegram] HTML rejected ({status}); falling back to plain text — \
                             chunk head: {}",
                            &chunk[..chunk.chars().count().min(80)]
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
                            Err(e) => return Err(format!("sendMessage retry: {e}")),
                        }
                    }
                    return Err(format!("sendMessage returned {status}"));
                }
                Err(e) => return Err(format!("sendMessage: {e}")),
            }
        }
        Ok(())
    }

    /// getFile → two-step download of the largest photo variant, returned
    /// as (mime, base64). Telegram serves files at
    /// `{api_base}/file/bot{token}/{path}` with a 1 MiB bot-API cap — well
    /// inside vision budgets after downscale on the sender side.
    async fn fetch_photo_base64(&self, file_id: &str) -> Result<(String, String), String> {
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
            .map_err(|e| format!("getFile: {e}"))?
            .json()
            .await
            .map_err(|e| format!("getFile body: {e}"))?;
        if !meta.ok {
            return Err("getFile not ok".into());
        }
        let path = meta.result.file_path.ok_or("getFile missing file_path")?;
        let mime = if path.ends_with(".jpg") || path.ends_with(".jpeg") {
            "image/jpeg"
        } else if path.ends_with(".webp") {
            "image/webp"
        } else {
            "image/png"
        };
        let bytes = http()
            .get(format!(
                "{}/file/bot{}/{}",
                self.api_base, self.bot_token, path
            ))
            .send()
            .await
            .map_err(|e| format!("download: {e}"))?
            .error_for_status()
            .map_err(|e| format!("download status: {e}"))?
            .bytes()
            .await
            .map_err(|e| format!("download body: {e}"))?;
        use base64::Engine as _;
        Ok((
            mime.to_string(),
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
        loop {
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
                            eprintln!("[telegram] poll failed ({failures} consecutive): {e}");
                            if failures >= 10 {
                                return Err(format!(
                                    "giving up after {failures} consecutive failures"
                                ));
                            }
                            tokio::time::sleep(std::time::Duration::from_secs(3)).await;
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
}
