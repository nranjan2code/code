//! Telegram surface adapter (docs/design/22-gateway.md G1): long-polls the
//! Bot API and bridges messages through `POST /gateway/inbound`, then
//! delivers the final assistant text back via `sendMessage`.
//!
//! The gateway stays transport-agnostic; this client runs anywhere it can
//! reach both Telegram and a vak-server — laptop, VPS, sidecar. Launched via
//! `vakcoder telegram --server URL --token GATEWAY_TOKEN` with
//! `TELEGRAM_BOT_TOKEN` in the environment (.env included).

use serde_json::Value;

pub struct TelegramBridge {
    /// Bot API base, e.g. `https://api.telegram.org`. Overridable for
    /// self-hosted relays and tests via `TELEGRAM_API_BASE`.
    pub api_base: String,
    pub bot_token: String,
    pub gateway_url: String,
    pub gateway_token: String,
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
        // Telegram hard-caps messages at 4096 chars; split on the last safe
        // newline under the limit rather than failing.
        for chunk in split_chunks(text, 4000) {
            let url = format!("{}/bot{}/sendMessage", self.api_base, self.bot_token);
            let resp = http()
                .post(&url)
                .json(&serde_json::json!({ "chat_id": chat_id, "text": chunk }))
                .send()
                .await
                .map_err(|e| format!("sendMessage: {e}"))?;
            let status = resp.status();
            if !status.is_success() {
                return Err(format!("sendMessage returned {status}"));
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
    pub async fn run(&self) -> Result<(), String> {
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
                    eprintln!("[telegram] poll failed ({failures} consecutive): {e}");
                    if failures >= 10 {
                        return Err(format!("giving up after {failures} consecutive failures"));
                    }
                    tokio::time::sleep(std::time::Duration::from_secs(3)).await;
                }
            }
        }
    }
}

fn split_chunks(text: &str, cap: usize) -> Vec<String> {
    if text.chars().count() <= cap {
        return vec![text.to_string()];
    }
    let mut chunks = Vec::new();
    let mut rest = text;
    while rest.chars().count() > cap {
        let cut = rest
            .char_indices()
            .nth(cap)
            .map(|(i, _)| i)
            .unwrap_or(rest.len());
        let slice = &rest[..cut];
        let boundary = slice.rfind('\n').map(|i| i + 1).unwrap_or(cut);
        chunks.push(rest[..boundary].trim_end().to_string());
        rest = &rest[boundary..];
    }
    if !rest.trim().is_empty() {
        chunks.push(rest.to_string());
    }
    chunks
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]
    #[test]
    fn chunks_respect_cap_and_newlines() {
        let small = super::split_chunks("hello", 10);
        assert_eq!(small.len(), 1);

        let long = "word ".repeat(3000);
        let chunks = super::split_chunks(long.trim(), 4000);
        assert!(chunks.len() >= 3);
        assert!(chunks.iter().all(|c| c.chars().count() <= 4000));

        let multiline = format!("{}\n{}", "a".repeat(4500), "tail");
        let chunks = super::split_chunks(&multiline, 4000);
        assert_eq!(chunks.len(), 2);
        // No newline inside the first cap window: hard cut, remainder keeps
        // the trailing segment.
        assert!(chunks[1].ends_with("tail"));
    }
}
