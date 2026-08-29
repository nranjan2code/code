//! Durable channel delivery and the transport adapter boundary.

use async_trait::async_trait;
use std::collections::{BTreeMap, HashMap};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::Duration;
use vak_core::Core;
use vak_delivery::client::WorkerClient;
use vak_delivery::outbox::{Outbox, OutboxRecord};
use vak_delivery::templates::{ChannelPreference, load_layers};
use vak_delivery::{
    AnswerDraft, DeliveryAction, DeliveryContent, DeliveryJob, DeliveryKind, DeliveryPacket,
    DeliveryProfile, Markup,
};

const WORKER_TIMEOUT: Duration = Duration::from_secs(5);
const MAX_CHANNEL_CHARS: usize = 100_000;

#[derive(Debug, Clone, Default, serde::Deserialize)]
#[serde(default, deny_unknown_fields)]
pub(crate) struct RequestedCapabilities {
    pub markup: Option<Markup>,
    pub max_chars: Option<usize>,
    pub supports_tables: Option<bool>,
    pub supports_code_blocks: Option<bool>,
    pub supports_links: Option<bool>,
    pub supports_actions: Option<bool>,
}

#[async_trait]
trait ChannelAdapter: Send + Sync {
    fn scheme(&self) -> &'static str;
    fn profile(&self) -> DeliveryProfile;
    async fn send(&self, core: &Core, packet: &DeliveryPacket) -> Result<(), String>;
}

struct AdapterRegistry {
    /// Legacy per-surface fallback, used for a two-part target
    /// (`surface:address`, no bot id) exactly as before multi-bot-per-
    /// channel existed.
    adapters: HashMap<&'static str, Arc<dyn ChannelAdapter>>,
    /// One adapter per configured bot, keyed by (surface, bot id) — used
    /// for a three-part target (`surface:address:bot_id`), so a reply goes
    /// out with *that* bot's own token rather than whichever token happens
    /// to be registered first for the surface (docs/design/34 Phase 5
    /// "known limitation", now fixed: the delivery target carries the bot
    /// id, so this map can pick the exact adapter instead of guessing).
    bot_adapters: HashMap<(String, String), Arc<dyn ChannelAdapter>>,
}

impl AdapterRegistry {
    /// Legacy single-slot token for `surface`, falling back to *some*
    /// configured multi-bot (docs/design/34) row on that surface when the
    /// legacy env var is unset — so a deployment that only ever went
    /// through the new "Add another bot" flow (no `TELEGRAM_BOT_TOKEN` set
    /// directly) can still deliver outbound replies, not just receive
    /// inbound ones. When more than one bot is configured for a surface
    /// this necessarily picks one arbitrarily; a target naming a specific
    /// bot never goes through this path (see `all_bot_tokens`/
    /// `bot_adapters`) — this fallback exists only for a target with no
    /// bot id at all. The legacy slot, when set, always wins so existing
    /// single-bot behavior is unchanged.
    fn resolve_surface_token(
        sessions_home: &std::path::Path,
        legacy_env_var: &str,
        surface: &str,
    ) -> Option<String> {
        if let Some(token) = vak_config::get_var(legacy_env_var) {
            return Some(token);
        }
        Self::all_bot_tokens(sessions_home, surface)
            .into_iter()
            .next()
            .map(|(_, token)| token)
    }

    /// Every configured bot on `surface` with a token actually set, as
    /// (bot id, token) pairs — the multi-bot counterpart of
    /// `resolve_surface_token`'s single arbitrary pick, used to give each
    /// bot its own adapter instead of collapsing them into one.
    fn all_bot_tokens(sessions_home: &std::path::Path, surface: &str) -> Vec<(String, String)> {
        let Ok(raw) = std::fs::read_to_string(sessions_home.join("gateway").join("bots.json"))
        else {
            return Vec::new();
        };
        let Ok(file) = serde_json::from_str::<serde_json::Value>(&raw) else {
            return Vec::new();
        };
        file.get("bots")
            .and_then(|b| b.as_array())
            .map(|bots| {
                bots.iter()
                    .filter_map(|b| {
                        if b.get("surface")?.as_str()? != surface {
                            return None;
                        }
                        let id = b.get("id")?.as_str()?.to_string();
                        let env_var = b.get("token_env")?.as_str()?;
                        let token = vak_config::get_var(env_var)?;
                        Some((id, token))
                    })
                    .collect()
            })
            .unwrap_or_default()
    }

    fn built_in(sessions_home: &std::path::Path) -> Self {
        let mut registry = Self {
            adapters: HashMap::new(),
            bot_adapters: HashMap::new(),
        };
        registry.register(LogAdapter);
        registry.register(WebhookAdapter);
        // `[gateway] approver = "telegram:<chat>"` needs a real push path,
        // not just the reply to whichever message happens to be in flight
        // — an approval gate can open while the approver isn't the one
        // currently talking. Registered only when a bot token is
        // configured so an unconfigured deployment fails with the same
        // clear "unsupported gateway surface" error as before.
        if let Some(bot_token) =
            Self::resolve_surface_token(sessions_home, "TELEGRAM_BOT_TOKEN", "telegram")
        {
            let api_base = vak_config::get_var("TELEGRAM_API_BASE")
                .unwrap_or_else(|| "https://api.telegram.org".into());
            registry.register(TelegramAdapter {
                bot_token,
                api_base,
            });
        }
        // Same rule for the Phase 3 surfaces (docs/design/34): registered
        // only when a bot token exists, so an unconfigured deployment
        // still fails with the clear "unsupported gateway surface" error.
        if let Some(bot_token) =
            Self::resolve_surface_token(sessions_home, "DISCORD_BOT_TOKEN", "discord")
        {
            registry.register(DiscordAdapter {
                bot_token,
                api_base: vak_config::get_var("DISCORD_API_BASE")
                    .unwrap_or_else(|| "https://discord.com/api/v10".into()),
            });
        }
        if let Some(bot_token) =
            Self::resolve_surface_token(sessions_home, "SLACK_BOT_TOKEN", "slack")
        {
            registry.register(SlackAdapter {
                bot_token,
                api_base: vak_config::get_var("SLACK_API_BASE")
                    .unwrap_or_else(|| "https://slack.com/api".into()),
            });
        }
        let telegram_api_base = vak_config::get_var("TELEGRAM_API_BASE")
            .unwrap_or_else(|| "https://api.telegram.org".into());
        for (id, bot_token) in Self::all_bot_tokens(sessions_home, "telegram") {
            registry.register_bot(
                "telegram",
                id,
                TelegramAdapter {
                    bot_token,
                    api_base: telegram_api_base.clone(),
                },
            );
        }
        let discord_api_base = vak_config::get_var("DISCORD_API_BASE")
            .unwrap_or_else(|| "https://discord.com/api/v10".into());
        for (id, bot_token) in Self::all_bot_tokens(sessions_home, "discord") {
            registry.register_bot(
                "discord",
                id,
                DiscordAdapter {
                    bot_token,
                    api_base: discord_api_base.clone(),
                },
            );
        }
        let slack_api_base =
            vak_config::get_var("SLACK_API_BASE").unwrap_or_else(|| "https://slack.com/api".into());
        for (id, bot_token) in Self::all_bot_tokens(sessions_home, "slack") {
            registry.register_bot(
                "slack",
                id,
                SlackAdapter {
                    bot_token,
                    api_base: slack_api_base.clone(),
                },
            );
        }
        registry
    }

    fn register(&mut self, adapter: impl ChannelAdapter + 'static) {
        self.adapters.insert(adapter.scheme(), Arc::new(adapter));
    }

    fn register_bot(&mut self, surface: &str, bot_id: String, adapter: impl ChannelAdapter + 'static) {
        self.bot_adapters
            .insert((surface.to_string(), bot_id), Arc::new(adapter));
    }

    /// A delivery target is `surface:address` for a legacy/single-bot
    /// chat, or `surface:address:bot_id` for one scoped to a specific bot
    /// (multi-bot-per-channel) — the third segment, when present, always
    /// wins over the legacy per-surface fallback so a reply is never sent
    /// under the wrong bot's identity.
    fn resolve(&self, target: &str) -> Result<(Arc<dyn ChannelAdapter>, String), String> {
        let mut parts = target.splitn(3, ':');
        let scheme = parts.next().filter(|s| !s.is_empty()).ok_or_else(|| {
            format!("invalid delivery target '{target}': expected '<surface>:<address>'")
        })?;
        let address = parts.next().ok_or_else(|| {
            format!("invalid delivery target '{target}': expected '<surface>:<address>'")
        })?;
        if address.trim().is_empty() {
            return Err(format!("delivery target '{target}' has an empty address"));
        }
        if let Some(bot_id) = parts.next().filter(|b| !b.trim().is_empty()) {
            return self
                .bot_adapters
                .get(&(scheme.to_string(), bot_id.to_string()))
                .cloned()
                .map(|adapter| (adapter, address.to_string()))
                // Named but not (yet) configured with a token: fail loudly
                // rather than silently falling back to a different bot's
                // token, which would reply under the wrong identity.
                .ok_or_else(|| {
                    format!("delivery target '{target}' names bot '{bot_id}', which has no token configured")
                });
        }
        self.adapters
            .get(scheme)
            .cloned()
            .map(|adapter| (adapter, address.to_string()))
            .ok_or_else(|| format!("unsupported gateway surface '{scheme}'"))
    }
}

struct DeliveryRuntime {
    outbox: Outbox,
    worker: Option<WorkerClient>,
    adapters: AdapterRegistry,
    serial: tokio::sync::Mutex<()>,
}

impl DeliveryRuntime {
    fn new(core: &Core) -> Self {
        let worker = std::env::current_exe()
            .ok()
            .filter(|path| {
                path.parent()
                    .and_then(|parent| parent.file_name())
                    .is_none_or(|name| name != "deps")
            })
            .map(|path| WorkerClient::new(path, WORKER_TIMEOUT));
        Self {
            outbox: Outbox::new(core.sessions_home().join("delivery").join("jobs")),
            worker,
            adapters: AdapterRegistry::built_in(&core.sessions_home()),
            serial: tokio::sync::Mutex::new(()),
        }
    }

    async fn render(&self, job: &DeliveryJob) -> Result<DeliveryPacket, String> {
        if let Some(worker) = &self.worker {
            match worker.render(job).await {
                Ok(packet) => return Ok(packet),
                Err(error) => {
                    eprintln!(
                        "[delivery] isolated renderer unavailable for {}: {error}; using safe fallback",
                        job.job_id
                    );
                }
            }
        }
        let mut packet = vak_delivery::render(job).map_err(|error| error.to_string())?;
        packet
            .diagnostics
            .push("isolated renderer unavailable; used deterministic in-process fallback".into());
        Ok(packet)
    }

    async fn deliver_record(
        &self,
        core: &Core,
        record: OutboxRecord,
    ) -> Result<DeliveryPacket, String> {
        let mut packet = self.render(&record.job).await?;
        let (adapter, address) = self.adapters.resolve(&record.job.target)?;
        // Each adapter's own `send` re-derives its address from
        // `packet.target` via a plain `surface:address` split — normalize
        // away a three-part bot-scoped target (`surface:address:bot_id`)
        // here, once, rather than teaching every adapter about the bot id
        // segment it has no use for once the right adapter is already
        // picked.
        packet.target = format!("{}:{address}", adapter.scheme());
        adapter.send(core, &packet).await?;
        self.outbox
            .mark_delivered(&record.job.job_id, packet.clone())
            .map_err(|error| error.to_string())?;
        Ok(packet)
    }
}

fn runtime(core: &Core) -> Arc<DeliveryRuntime> {
    static RUNTIMES: OnceLock<Mutex<BTreeMap<String, Arc<DeliveryRuntime>>>> = OnceLock::new();
    let key = core.sessions_home().to_string_lossy().into_owned();
    let runtimes = RUNTIMES.get_or_init(|| Mutex::new(BTreeMap::new()));
    let mut runtimes = runtimes
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    if let Some(existing) = runtimes.get(&key) {
        return existing.clone();
    }
    let created = Arc::new(DeliveryRuntime::new(core));
    runtimes.insert(key, created.clone());
    created
}

pub(crate) fn profile_for_surface(
    core: &Core,
    surface: &str,
    requested: Option<&RequestedCapabilities>,
) -> DeliveryProfile {
    let mut profile = built_in_surface_profile(surface);
    if let Some(requested) = requested {
        if let Some(markup) = requested.markup {
            profile.markup = markup;
        }
        if let Some(max_chars) = requested.max_chars.filter(|value| *value > 0) {
            profile.max_chars = Some(max_chars.min(MAX_CHANNEL_CHARS));
        }
        profile.supports_tables = requested.supports_tables.unwrap_or(profile.supports_tables);
        profile.supports_code_blocks = requested
            .supports_code_blocks
            .unwrap_or(profile.supports_code_blocks);
        profile.supports_links = requested.supports_links.unwrap_or(profile.supports_links);
        profile.supports_actions = requested
            .supports_actions
            .unwrap_or(profile.supports_actions);
    }
    apply_preferences(core, profile)
}

fn built_in_surface_profile(surface: &str) -> DeliveryProfile {
    match surface {
        "telegram" => DeliveryProfile {
            surface: surface.into(),
            markup: Markup::TelegramHtml,
            max_chars: Some(4000),
            supports_tables: false,
            supports_code_blocks: true,
            supports_links: true,
            supports_actions: false,
            template: None,
        },
        // docs/design/34 Phase 3: both take Markdown, differ only in the
        // per-message cap each API enforces.
        "discord" | "slack" => DeliveryProfile {
            surface: surface.into(),
            markup: Markup::Markdown,
            max_chars: Some(if surface == "discord" { 1900 } else { 3900 }),
            supports_tables: false,
            supports_code_blocks: true,
            supports_links: true,
            supports_actions: false,
            template: None,
        },
        "desktop" | "tui" => DeliveryProfile {
            surface: surface.into(),
            markup: Markup::Markdown,
            max_chars: None,
            supports_tables: true,
            supports_code_blocks: true,
            supports_links: true,
            supports_actions: true,
            template: None,
        },
        _ => DeliveryProfile {
            surface: surface.into(),
            markup: Markup::Plain,
            max_chars: Some(4000),
            supports_tables: false,
            supports_code_blocks: false,
            supports_links: false,
            supports_actions: false,
            template: None,
        },
    }
}

fn apply_preferences(core: &Core, mut profile: DeliveryProfile) -> DeliveryProfile {
    let loaded = load_layers(
        &core.sessions_home().join("output.toml"),
        &core.cwd().join(".vak").join("output.toml"),
        core.project_config_trusted(),
    );
    for warning in loaded.warnings {
        eprintln!("[delivery] {warning}");
    }
    if let Some(preference) = loaded.channels.get(&profile.surface) {
        apply_preference(&mut profile, preference, &loaded.registry);
    }
    profile
}

fn apply_preference(
    profile: &mut DeliveryProfile,
    preference: &ChannelPreference,
    templates: &vak_delivery::TemplateRegistry,
) {
    if let Some(max_chars) = preference.max_chars.filter(|value| *value > 0) {
        profile.max_chars = Some(
            profile
                .max_chars
                .map_or(max_chars, |hard_limit| hard_limit.min(max_chars)),
        );
    }
    if let Some(markup) = preference.markup
        && markup == profile.markup
    {
        profile.markup = markup;
    }
    if !matches!(profile.markup, Markup::Json)
        && let Some(template) = preference
            .template
            .as_deref()
            .and_then(|id| templates.resolve(id))
    {
        profile.template = Some(template.clone());
    }
}

pub(crate) async fn render_response(
    core: &Core,
    surface: &str,
    chat: &str,
    markdown: String,
    requested: Option<&RequestedCapabilities>,
) -> Result<DeliveryPacket, String> {
    let runtime = runtime(core);
    let job = DeliveryJob {
        job_id: uuid::Uuid::now_v7().to_string(),
        target: format!("{surface}:{chat}"),
        kind: DeliveryKind::Assistant,
        content: DeliveryContent::Answer(AnswerDraft::from_markdown(markdown)),
        profile: profile_for_surface(core, surface, requested),
    };
    runtime.render(&job).await
}

pub(crate) async fn deliver(
    core: &Core,
    target: &str,
    kind: DeliveryKind,
    content: DeliveryContent,
) -> Result<DeliveryPacket, String> {
    let runtime = runtime(core);
    let _serial = runtime.serial.lock().await;
    let (adapter, _) = runtime.adapters.resolve(target)?;
    let profile = apply_preferences(core, adapter.profile());
    let job = DeliveryJob {
        job_id: uuid::Uuid::now_v7().to_string(),
        target: target.into(),
        kind,
        content,
        profile,
    };
    let record = runtime
        .outbox
        .enqueue(job)
        .map_err(|error| error.to_string())?;
    match runtime.deliver_record(core, record.clone()).await {
        Ok(packet) => Ok(packet),
        Err(error) => {
            let _ = runtime.outbox.mark_failed(&record.job.job_id, &error);
            Err(error)
        }
    }
}

pub(crate) fn start_replay(core: &Core) {
    let core = core.clone();
    tokio::spawn(async move {
        let runtime = runtime(&core);
        let mut interval = tokio::time::interval(Duration::from_secs(30));
        loop {
            interval.tick().await;
            let _serial = runtime.serial.lock().await;
            let records = match runtime.outbox.pending() {
                Ok(records) => records,
                Err(error) => {
                    eprintln!("[delivery] outbox replay scan failed: {error}");
                    continue;
                }
            };
            for record in records.into_iter().take(100) {
                if record.attempts >= 10 {
                    let _ = runtime
                        .outbox
                        .mark_dead_letter(&record.job.job_id, "delivery retry budget exhausted");
                    eprintln!(
                        "[delivery] {} moved to dead letter after {} attempts",
                        record.job.job_id, record.attempts
                    );
                    continue;
                }
                if let Err(error) = runtime.deliver_record(&core, record.clone()).await {
                    let _ = runtime.outbox.mark_failed(&record.job.job_id, &error);
                    eprintln!("[delivery] replay {} failed: {error}", record.job.job_id);
                }
            }
        }
    });
}

struct LogAdapter;

#[async_trait]
impl ChannelAdapter for LogAdapter {
    fn scheme(&self) -> &'static str {
        "log"
    }

    fn profile(&self) -> DeliveryProfile {
        DeliveryProfile {
            surface: "log".into(),
            markup: Markup::Markdown,
            max_chars: None,
            supports_tables: true,
            supports_code_blocks: true,
            supports_links: true,
            supports_actions: false,
            template: None,
        }
    }

    async fn send(&self, core: &Core, packet: &DeliveryPacket) -> Result<(), String> {
        let path = core
            .sessions_home()
            .join("gateway")
            .join("deliveries.jsonl");
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)
                .map_err(|error| format!("create deliveries directory: {error}"))?;
        }
        let line = serde_json::json!({
            "ts": chrono::Utc::now().to_rfc3339(),
            "target": packet.target,
            "text": packet.fallback_markdown,
            "job_id": packet.job_id,
            "delivery": packet,
        });
        let mut buffer = line.to_string();
        buffer.push('\n');
        use std::io::Write;
        let mut file = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(path)
            .map_err(|error| format!("open deliveries log: {error}"))?;
        file.write_all(buffer.as_bytes())
            .map_err(|error| format!("append deliveries log: {error}"))
    }
}

struct WebhookAdapter;

#[async_trait]
impl ChannelAdapter for WebhookAdapter {
    fn scheme(&self) -> &'static str {
        "webhook"
    }

    fn profile(&self) -> DeliveryProfile {
        DeliveryProfile {
            surface: "webhook".into(),
            markup: Markup::Json,
            max_chars: None,
            supports_tables: true,
            supports_code_blocks: true,
            supports_links: true,
            supports_actions: true,
            template: None,
        }
    }

    async fn send(&self, core: &Core, packet: &DeliveryPacket) -> Result<(), String> {
        let (_, name) = packet
            .target
            .split_once(':')
            .ok_or_else(|| "webhook target has no name".to_string())?;
        super::gateway::deliver_webhook_packet(core, name, packet).await
    }
}

/// Telegram's `reply_markup.inline_keyboard`: one row, one button per
/// action. `callback_data` is `"<verb>:<request_id>"` — the bridge's
/// `handle_callback` splits on the first `:` and maps `verb` straight onto
/// the same "yes"/"no" verdict text a typed chat reply would produce.
fn inline_keyboard_markup(actions: &[DeliveryAction]) -> serde_json::Value {
    let buttons: Vec<serde_json::Value> = actions
        .iter()
        .map(|action| {
            let request_id = action
                .data
                .get("request_id")
                .map(String::as_str)
                .unwrap_or_default();
            serde_json::json!({
                "text": action.label,
                "callback_data": format!("{}:{request_id}", action.verb),
            })
        })
        .collect();
    serde_json::json!({ "inline_keyboard": [buttons] })
}

/// Proactive push to a Telegram chat: the async counterpart to the
/// bridge's own `sendMessage` reply. Used for anything that isn't a direct
/// reply to the message currently in flight — chiefly forwarded approval
/// gates (`[gateway] approver = "telegram:<chat>"`), which can open while
/// the approver chat isn't the one that triggered the turn.
struct TelegramAdapter {
    bot_token: String,
    api_base: String,
}

#[async_trait]
impl ChannelAdapter for TelegramAdapter {
    fn scheme(&self) -> &'static str {
        "telegram"
    }

    fn profile(&self) -> DeliveryProfile {
        DeliveryProfile {
            surface: "telegram".into(),
            markup: Markup::TelegramHtml,
            max_chars: Some(4000),
            supports_tables: false,
            supports_code_blocks: true,
            supports_links: true,
            // Unlike the synchronous per-turn reply profile, this adapter
            // renders `packet.actions` as inline-keyboard buttons below.
            supports_actions: true,
            template: None,
        }
    }

    async fn send(&self, _core: &Core, packet: &DeliveryPacket) -> Result<(), String> {
        let (_, chat_id) = packet
            .target
            .split_once(':')
            .ok_or_else(|| "telegram target has no chat id".to_string())?;
        let chunks: Vec<&str> = if packet.chunks.is_empty() {
            vec![packet.fallback_markdown.as_str()]
        } else {
            packet.chunks.iter().map(String::as_str).collect()
        };
        let client = reqwest::Client::new();
        let last = chunks.len().saturating_sub(1);
        for (i, chunk) in chunks.iter().enumerate() {
            let mut body = serde_json::json!({
                "chat_id": chat_id,
                "text": chunk,
                "parse_mode": "HTML",
                "link_preview_options": { "is_disabled": true },
            });
            // Buttons ride on the last chunk so they land under the final
            // line of text, matching where a human reader expects them.
            if i == last && !packet.actions.is_empty() {
                body["reply_markup"] = inline_keyboard_markup(&packet.actions);
            }
            let resp = client
                .post(format!(
                    "{}/bot{}/sendMessage",
                    self.api_base, self.bot_token
                ))
                .json(&body)
                .send()
                .await
                .map_err(|error| format!("telegram sendMessage: {error}"))?;
            if !resp.status().is_success() {
                return Err(format!("telegram sendMessage returned {}", resp.status()));
            }
        }
        Ok(())
    }
}

/// Forwarded-approval actions rendered as a typed-verdict prompt, for the
/// surfaces that do not (yet) get interactive components here. This is the
/// same fallback Telegram used before its inline keyboard: the reply text
/// it asks for is exactly what `parse_verdict` in `gateway.rs` already
/// understands, so approvals resolve through the one existing path.
fn typed_verdict_prompt(actions: &[DeliveryAction]) -> Option<String> {
    let request_id = actions
        .iter()
        .find_map(|action| action.data.get("request_id"))?;
    Some(format!(
        "\n\nReply `yes {request_id}` to approve or `no {request_id}` to deny."
    ))
}

/// Proactive push to a Discord channel (docs/design/34 Phase 3): the async
/// counterpart to the bridge's own reply, chiefly forwarded approval
/// gates, which can open while the approver channel is not the one that
/// triggered the turn.
struct DiscordAdapter {
    bot_token: String,
    api_base: String,
}

#[async_trait]
impl ChannelAdapter for DiscordAdapter {
    fn scheme(&self) -> &'static str {
        "discord"
    }

    fn profile(&self) -> DeliveryProfile {
        DeliveryProfile {
            surface: "discord".into(),
            markup: Markup::Markdown,
            max_chars: Some(1900),
            supports_tables: false,
            supports_code_blocks: true,
            supports_links: true,
            // Interactive components (Discord buttons) are a follow-up;
            // approvals ship as the typed yes/no prompt below.
            supports_actions: false,
            template: None,
        }
    }

    async fn send(&self, _core: &Core, packet: &DeliveryPacket) -> Result<(), String> {
        let (_, channel_id) = packet
            .target
            .split_once(':')
            .ok_or_else(|| "discord target has no channel id".to_string())?;
        post_chunks(
            packet,
            |chunk, last| {
                let mut content = chunk.to_string();
                if last && let Some(prompt) = typed_verdict_prompt(&packet.actions) {
                    content.push_str(&prompt);
                }
                (
                    format!("{}/channels/{channel_id}/messages", self.api_base),
                    serde_json::json!({ "content": content }),
                )
            },
            |request| request.header("Authorization", format!("Bot {}", self.bot_token)),
            "discord createMessage",
        )
        .await
    }
}

/// Proactive push to a Slack channel/DM via `chat.postMessage`.
struct SlackAdapter {
    bot_token: String,
    api_base: String,
}

#[async_trait]
impl ChannelAdapter for SlackAdapter {
    fn scheme(&self) -> &'static str {
        "slack"
    }

    fn profile(&self) -> DeliveryProfile {
        DeliveryProfile {
            surface: "slack".into(),
            markup: Markup::Markdown,
            max_chars: Some(3900),
            supports_tables: false,
            supports_code_blocks: true,
            supports_links: true,
            // Block Kit buttons are a follow-up; see typed_verdict_prompt.
            supports_actions: false,
            template: None,
        }
    }

    async fn send(&self, _core: &Core, packet: &DeliveryPacket) -> Result<(), String> {
        let (_, channel_id) = packet
            .target
            .split_once(':')
            .ok_or_else(|| "slack target has no channel id".to_string())?;
        post_chunks(
            packet,
            |chunk, last| {
                let mut text = chunk.to_string();
                if last && let Some(prompt) = typed_verdict_prompt(&packet.actions) {
                    text.push_str(&prompt);
                }
                (
                    format!("{}/chat.postMessage", self.api_base),
                    serde_json::json!({ "channel": channel_id, "text": text }),
                )
            },
            |request| request.bearer_auth(&self.bot_token),
            "slack chat.postMessage",
        )
        .await
    }
}

/// Shared chunk-and-POST loop for the two Phase 3 adapters: `body` builds
/// the (url, json) for one chunk and is told whether it is the last one,
/// `auth` applies the surface's auth header.
async fn post_chunks(
    packet: &DeliveryPacket,
    body: impl Fn(&str, bool) -> (String, serde_json::Value),
    auth: impl Fn(reqwest::RequestBuilder) -> reqwest::RequestBuilder,
    operation: &str,
) -> Result<(), String> {
    let chunks: Vec<&str> = if packet.chunks.is_empty() {
        vec![packet.fallback_markdown.as_str()]
    } else {
        packet.chunks.iter().map(String::as_str).collect()
    };
    let client = reqwest::Client::new();
    let last = chunks.len().saturating_sub(1);
    for (i, chunk) in chunks.iter().enumerate() {
        let (url, json) = body(chunk, i == last);
        let resp = auth(client.post(url))
            .json(&json)
            .send()
            .await
            .map_err(|error| format!("{operation}: {error}"))?;
        if !resp.status().is_success() {
            return Err(format!("{operation} returned {}", resp.status()));
        }
    }
    Ok(())
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;

    fn approval_actions() -> Vec<DeliveryAction> {
        vec![
            DeliveryAction {
                id: "approve".into(),
                label: "Approve".into(),
                verb: "approve".into(),
                data: [("request_id".into(), "abc123".into())]
                    .into_iter()
                    .collect(),
            },
            DeliveryAction {
                id: "deny".into(),
                label: "Deny".into(),
                verb: "deny".into(),
                data: [("request_id".into(), "abc123".into())]
                    .into_iter()
                    .collect(),
            },
        ]
    }

    #[test]
    fn inline_keyboard_carries_verb_and_request_id_in_callback_data() {
        let markup = inline_keyboard_markup(&approval_actions());
        let row = markup["inline_keyboard"][0].as_array().unwrap();
        assert_eq!(row.len(), 2);
        assert_eq!(row[0]["text"], "Approve");
        assert_eq!(row[0]["callback_data"], "approve:abc123");
        assert_eq!(row[1]["text"], "Deny");
        assert_eq!(row[1]["callback_data"], "deny:abc123");
    }

    #[test]
    fn typed_verdict_prompt_asks_for_the_text_parse_verdict_accepts() {
        let prompt = typed_verdict_prompt(&approval_actions()).unwrap();
        assert!(prompt.contains("yes abc123"));
        assert!(prompt.contains("no abc123"));
    }

    #[test]
    fn typed_verdict_prompt_is_absent_without_actions() {
        assert!(typed_verdict_prompt(&[]).is_none());
    }

    #[test]
    fn phase_three_surfaces_declare_no_interactive_components_yet() {
        // Approvals ship as typed yes/no on these surfaces; the profile
        // must say so or the renderer would emit buttons nothing draws.
        let discord = DiscordAdapter {
            bot_token: "t".into(),
            api_base: "http://localhost".into(),
        };
        let slack = SlackAdapter {
            bot_token: "t".into(),
            api_base: "http://localhost".into(),
        };
        assert_eq!(discord.scheme(), "discord");
        assert_eq!(slack.scheme(), "slack");
        assert!(!discord.profile().supports_actions);
        assert!(!slack.profile().supports_actions);
    }

    #[test]
    fn inline_keyboard_is_empty_row_when_no_actions() {
        let markup = inline_keyboard_markup(&[]);
        let row = markup["inline_keyboard"][0].as_array().unwrap();
        assert!(row.is_empty());
    }

    fn telegram_adapter(token: &str) -> Arc<dyn ChannelAdapter> {
        Arc::new(TelegramAdapter {
            bot_token: token.into(),
            api_base: "http://localhost".into(),
        })
    }

    /// The whole point of a bot-scoped delivery target: two bots on the
    /// same surface must resolve to two distinct adapters (and therefore
    /// two distinct tokens), not whichever one happens to be registered
    /// first — the exact "known limitation" docs/design/34 Phase 5 called
    /// out and this change fixes. Compared by `Arc::ptr_eq` rather than by
    /// field, since `resolve` hands back a trait object.
    #[test]
    fn bot_scoped_target_resolves_to_that_bots_own_adapter() {
        let vakbot = telegram_adapter("vakbot-token");
        let vakyartha = telegram_adapter("vakyartha-token");
        let registry = AdapterRegistry {
            adapters: HashMap::new(),
            bot_adapters: HashMap::from([
                (("telegram".to_string(), "VakBot".to_string()), vakbot.clone()),
                (
                    ("telegram".to_string(), "Vakyartha".to_string()),
                    vakyartha.clone(),
                ),
            ]),
        };

        let (adapter_a, address_a) = registry.resolve("telegram:8846301562:VakBot").unwrap();
        assert_eq!(address_a, "8846301562");
        assert!(Arc::ptr_eq(&adapter_a, &vakbot), "must pick VakBot's own adapter");
        assert!(!Arc::ptr_eq(&adapter_a, &vakyartha));

        let (adapter_b, address_b) = registry.resolve("telegram:8846301562:Vakyartha").unwrap();
        assert_eq!(address_b, "8846301562");
        assert!(
            Arc::ptr_eq(&adapter_b, &vakyartha),
            "must pick Vakyartha's own adapter, not VakBot's"
        );
    }

    /// Naming a bot that has no token configured must fail loudly, never
    /// silently fall back to a different bot's token — that would reply
    /// under the wrong identity without anyone noticing.
    #[test]
    fn bot_scoped_target_naming_an_unknown_bot_is_a_clear_error() {
        let registry = AdapterRegistry {
            adapters: HashMap::new(),
            bot_adapters: HashMap::from([(
                ("telegram".to_string(), "VakBot".to_string()),
                telegram_adapter("vakbot-token"),
            )]),
        };
        let err = registry
            .resolve("telegram:8846301562:SomeOtherBot")
            .map(|_| ())
            .unwrap_err();
        assert!(err.contains("SomeOtherBot"), "{err}");
        assert!(err.contains("no token configured"), "{err}");
    }

    /// A legacy two-part target (no bot id) must still resolve through the
    /// per-surface fallback exactly as before — single-bot deployments see
    /// no behavior change even once other bots are registered.
    #[test]
    fn legacy_two_part_target_still_uses_the_surface_fallback() {
        let legacy = telegram_adapter("legacy-token");
        let registry = AdapterRegistry {
            adapters: HashMap::from([("telegram", legacy.clone())]),
            bot_adapters: HashMap::from([(
                ("telegram".to_string(), "VakBot".to_string()),
                telegram_adapter("vakbot-token"),
            )]),
        };
        let (adapter, address) = registry.resolve("telegram:8846301562").unwrap();
        assert_eq!(address, "8846301562");
        assert!(Arc::ptr_eq(&adapter, &legacy));
    }

    #[test]
    fn resolve_rejects_targets_with_no_address_or_empty_address() {
        let registry = AdapterRegistry {
            adapters: HashMap::from([("telegram", telegram_adapter("t"))]),
            bot_adapters: HashMap::new(),
        };
        assert!(registry.resolve("telegram").is_err());
        assert!(registry.resolve("telegram:").is_err());
    }
}
