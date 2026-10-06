//! Durable channel delivery and the transport adapter boundary.

use async_trait::async_trait;
use serde_json::Value;
use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::Duration;
use vak_core::Core;
use vak_delivery::client::WorkerClient;
use vak_delivery::templates::{ChannelPreference, load_layers};
use vak_delivery::{
    AnswerDraft, DeliveryAction, DeliveryContent, DeliveryJob, DeliveryKind, DeliveryPacket,
    DeliveryProfile, Markup,
};
use vak_session::effects::{Dispatch, EffectKind, EffectRecord, Prepare, Receipt};
use vak_session::ids::EffectId;

const WORKER_TIMEOUT: Duration = Duration::from_secs(5);
const MAX_CHANNEL_CHARS: usize = 100_000;

/// Build a plugin-merged `PresentationPlanner` (skills + recipes) from all
/// enabled plugins across the Core's capability roots. Starts with built-in
/// skills and recipes and layers in any `PresentationSkillManifest` and
/// `PresentationRecipe` files that plugins declare in their
/// `components.presentation` list.
pub(crate) fn merged_presentation_planner(core: &Core) -> vak_delivery::PresentationPlanner {
    let mut skills = vak_delivery::built_in_skill_registry();
    let mut recipes = vak_delivery::built_in_recipes();
    let mut revoked_skills = BTreeSet::new();
    for root in core.capability_roots() {
        let Ok(plugins) = vak_plugin::PluginStore::new(&root.path).enabled() else {
            continue;
        };
        for plugin in plugins {
            for relative in &plugin.capabilities.presentation {
                let path = plugin.package_path.join(relative);
                let Ok(bytes) = std::fs::read(&path) else {
                    continue;
                };
                if let Ok(json) = std::str::from_utf8(&bytes) {
                    if let Ok(manifest) =
                        serde_json::from_str::<vak_delivery::PresentationSkillManifest>(json)
                    {
                        if !core.capability_revoked(
                            vak_session::types::CapabilityKind::Skill,
                            &manifest.id,
                        ) {
                            let _ = skills.register(manifest);
                        } else {
                            revoked_skills.insert(manifest.id);
                        }
                    } else if let Ok(recipe) =
                        serde_json::from_str::<vak_delivery::PresentationRecipe>(json)
                    {
                        let _ = recipes.register(recipe);
                    }
                }
            }
        }
    }
    recipes.remove_revoked_skills(&revoked_skills);
    vak_delivery::PresentationPlanner { skills, recipes }
}

/// Build a plugin-merged `SkillRegistry` for the worker's structured-fence
/// projection. Equivalent to `merged_presentation_planner(core).skills`
/// but avoids constructing a full planner.
fn merged_presentation_skills(core: &Core) -> vak_delivery::SkillRegistry {
    merged_presentation_planner(core).skills
}

#[derive(Debug, Clone, Default, serde::Deserialize, serde::Serialize)]
#[serde(default, deny_unknown_fields)]
pub struct RequestedCapabilities {
    pub markup: Option<Markup>,
    pub max_chars: Option<usize>,
    pub supports_tables: Option<bool>,
    pub supports_code_blocks: Option<bool>,
    pub supports_links: Option<bool>,
    pub supports_actions: Option<bool>,
    /// The bridge sends files back to the chat: a turn's Office drafts are
    /// returned as documents (docs/design/72, P5).
    pub accepts_files: Option<bool>,
}

/// Whether a failed send left anything with the provider.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Landed {
    /// The provider answered that it took nothing.
    No,
    /// Part of it landed before the provider refused the rest.
    Partly,
    /// Nobody can say: it timed out, or the provider failed after taking it.
    Unknown,
}

#[derive(Debug, Clone)]
pub(crate) struct SendFailure {
    pub(crate) reason: String,
    pub(crate) landed: Landed,
}

impl SendFailure {
    pub(crate) fn not_sent(reason: impl Into<String>) -> Self {
        Self {
            reason: reason.into(),
            landed: Landed::No,
        }
    }

    /// A transport error: refused before a connection was made means
    /// nothing was sent; any other means nobody knows.
    pub(crate) fn transport(operation: &str, error: &reqwest::Error, sent_before: bool) -> Self {
        let landed = if error.is_connect() || error.is_builder() {
            if sent_before {
                Landed::Partly
            } else {
                Landed::No
            }
        } else {
            Landed::Unknown
        };
        Self {
            reason: format!("{operation}: {error}"),
            landed,
        }
    }

    /// A provider's answer: a 4xx (429 included) took nothing; a 5xx may
    /// have taken it.
    pub(crate) fn status(operation: &str, status: reqwest::StatusCode, sent_before: bool) -> Self {
        let landed = if status.is_client_error() {
            if sent_before {
                Landed::Partly
            } else {
                Landed::No
            }
        } else {
            Landed::Unknown
        };
        Self {
            reason: format!("{operation} returned {status}"),
            landed,
        }
    }
}

#[async_trait]
trait ChannelAdapter: Send + Sync {
    fn scheme(&self) -> &'static str;
    fn profile(&self) -> DeliveryProfile;
    /// Sends `packet`, telling the provider `key` where it takes one.
    /// Returns the provider's id for what it created, when it gives one:
    /// an id the provider returned proves the message landed.
    async fn send(
        &self,
        core: &Core,
        packet: &DeliveryPacket,
        key: &str,
    ) -> Result<Option<String>, SendFailure>;
    /// Whether a success proves the message landed rather than that the
    /// provider took it.
    fn confirms(&self) -> bool {
        true
    }
    /// Whether the provider drops a repeat of a key it has seen, so an
    /// unknown send may be sent again under the same key.
    fn dedupes(&self) -> bool {
        false
    }
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
    /// Every configured bot on `surface` with a token actually set, as
    /// (bot id, token) pairs. Each gets its own adapter: collapsing them
    /// into one per surface is how a reply goes out under the wrong bot's
    /// identity (AGENTS.md invariant 24).
    fn all_bot_tokens(sessions_home: &std::path::Path, surface: &str) -> Vec<(String, String)> {
        let Ok(raw) = std::fs::read_to_string(
            vak_config::scope::SharedScope::new(sessions_home).gateway_bots(),
        ) else {
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
        // No per-surface chat adapter is registered. A chat credential
        // belongs to a bot (invariant 23), so every chat surface resolves
        // through the (surface, bot_id) map below. Log and webhook stay
        // surface-level because neither is a bot.
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

    fn register_bot(
        &mut self,
        surface: &str,
        bot_id: String,
        adapter: impl ChannelAdapter + 'static,
    ) {
        self.bot_adapters
            .insert((surface.to_string(), bot_id), Arc::new(adapter));
    }

    /// A delivery target is `surface:address:bot_id` for a chat surface,
    /// or `surface:address` for `log` and `webhook`, which are not bots.
    ///
    /// A chat target with no bot id is refused rather than resolved: there
    /// is no per-surface fallback any more, because picking "some bot on
    /// this surface" replies under an identity the chat was never bound to
    /// (AGENTS.md invariant 24).
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
        if vak_core::Core::is_surface(scheme) {
            return Err(format!(
                "delivery target '{target}' names no bot; a {scheme} target must be \
                 '<surface>:<address>:<bot_id>'"
            ));
        }
        self.adapters
            .get(scheme)
            .cloned()
            .map(|adapter| (adapter, address.to_string()))
            .ok_or_else(|| format!("unsupported gateway surface '{scheme}'"))
    }
}

struct DeliveryRuntime {
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
            worker,
            adapters: AdapterRegistry::built_in(&core.shared_scope().into_root()),
            serial: tokio::sync::Mutex::new(()),
        }
    }

    async fn render(&self, job: &DeliveryJob) -> Result<DeliveryPacket, String> {
        if let Some(worker) = &self.worker {
            match worker.render(job).await {
                Ok(packet) => return Ok(packet),
                Err(error) => {
                    tracing::warn!(
                        effect = %job.job_id,
                        error_kind = %vak_telemetry::error_kind(&error),
                        "the isolated renderer was unavailable; used the safe fallback"
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

    /// Sends the effect `id` if this process takes it (`how`), and
    /// records how it went. Only a taken effect is sent.
    async fn dispatch(
        &self,
        core: &Core,
        id: EffectId,
        how: Dispatch,
    ) -> Result<DeliveryPacket, String> {
        let effects = core.effects();
        let record = effects
            .begin_dispatch(id, how)
            .map_err(|error| error.to_string())?
            .ok_or_else(|| format!("effect {id} is not this server's to send now"))?;
        let span = vak_session::effects::delivery_span(&record);
        match tracing::Instrument::instrument(self.send(core, &record), span).await {
            Ok((packet, receipt, confirmed)) => {
                let recorded = if confirmed {
                    effects.confirmed(id, receipt)
                } else {
                    effects.accepted(id, receipt)
                };
                if let Err(error) = recorded {
                    tracing::error!(effect = %id, error_kind = %vak_telemetry::error_kind(&error), "an effect was sent but not recorded");
                }
                Ok(packet)
            }
            Err(failure) => {
                let recorded = match failure.landed {
                    Landed::No => effects.failed(id, &failure.reason, true),
                    Landed::Partly => effects.failed(id, &failure.reason, false),
                    Landed::Unknown => effects.unknown(id, &failure.reason),
                };
                if let Err(error) = recorded {
                    tracing::error!(effect = %id, error_kind = %vak_telemetry::error_kind(&error), "an effect's outcome was not recorded");
                }
                Err(failure.reason)
            }
        }
    }

    async fn send(
        &self,
        core: &Core,
        record: &EffectRecord,
    ) -> Result<(DeliveryPacket, Receipt, bool), SendFailure> {
        let payload = core
            .effects()
            .payload(record)
            .map_err(|error| SendFailure::not_sent(format!("effect payload: {error}")))?;
        let mut job: DeliveryJob = serde_json::from_slice(&payload)
            .map_err(|error| SendFailure::not_sent(format!("effect payload: {error}")))?;
        // A job sent again carries the id of the effect that sends it.
        job.job_id = record.id.to_string();
        let mut packet = self.render(&job).await.map_err(SendFailure::not_sent)?;
        let (adapter, address) = self
            .adapters
            .resolve(&record.target)
            .map_err(SendFailure::not_sent)?;
        // Each adapter's own `send` re-derives its address from
        // `packet.target` via a plain `surface:address` split — normalize
        // away a three-part bot-scoped target (`surface:address:bot_id`)
        // here, once, rather than teaching every adapter about the bot id
        // segment it has no use for once the right adapter is already
        // picked.
        packet.target = format!("{}:{address}", adapter.scheme());
        let provider_id = adapter.send(core, &packet, &record.idempotency_key).await?;
        let receipt = Receipt {
            provider: adapter.scheme().to_string(),
            provider_id,
            at: chrono::Utc::now(),
        };
        Ok((packet, receipt, adapter.confirms()))
    }

    /// Whether the provider behind `target` drops a repeat of a key.
    fn dedupes(&self, target: &str) -> bool {
        self.adapters
            .resolve(target)
            .is_ok_and(|(adapter, _)| adapter.dedupes())
    }
}

fn runtime(core: &Core) -> Arc<DeliveryRuntime> {
    static RUNTIMES: OnceLock<Mutex<BTreeMap<String, Arc<DeliveryRuntime>>>> = OnceLock::new();
    let key = core.scope().into_root().to_string_lossy().into_owned();
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
            posture: vak_delivery::DeliveryPosture::default(),
        },
        "discord" => DeliveryProfile {
            surface: surface.into(),
            markup: Markup::DiscordMarkdown,
            max_chars: Some(1900),
            supports_tables: false,
            supports_code_blocks: true,
            supports_links: true,
            supports_actions: false,
            template: None,
            posture: vak_delivery::DeliveryPosture::default(),
        },
        "slack" => DeliveryProfile {
            surface: surface.into(),
            markup: Markup::SlackMrkdwn,
            max_chars: Some(3900),
            supports_tables: false,
            supports_code_blocks: true,
            supports_links: true,
            supports_actions: false,
            template: None,
            posture: vak_delivery::DeliveryPosture::default(),
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
            posture: vak_delivery::DeliveryPosture::default(),
        },
        "background" => DeliveryProfile {
            surface: surface.into(),
            markup: Markup::Plain,
            max_chars: Some(4000),
            supports_tables: false,
            supports_code_blocks: false,
            supports_links: true,
            supports_actions: false,
            template: None,
            posture: vak_delivery::DeliveryPosture {
                cadence: vak_delivery::Cadence::Digest,
                urgency: vak_delivery::Urgency::Quiet,
            },
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
            posture: vak_delivery::DeliveryPosture::default(),
        },
    }
}

fn apply_preferences(core: &Core, mut profile: DeliveryProfile) -> DeliveryProfile {
    let loaded = load_layers(
        &core.scope().output_prefs(),
        &core.workspace_scope().output_prefs(),
        core.project_config_trusted(),
    );
    for warning in loaded.warnings {
        let _ = warning;
        tracing::warn!(
            kind = "output_preferences",
            "an output preference was not applied"
        );
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

#[allow(clippy::too_many_arguments)]
pub(crate) async fn render_response(
    core: &Core,
    surface: &str,
    chat: &str,
    markdown: String,
    requested: Option<&RequestedCapabilities>,
    outcome_metadata: Option<std::collections::BTreeMap<String, String>>,
    provenance: Option<std::collections::BTreeMap<String, String>>,
    bot_id: Option<&str>,
    session_id: Option<&str>,
    intent_posture: Option<vak_intent::DeliveryPosture>,
    cards: Vec<vak_delivery::StructuredOutput>,
) -> Result<DeliveryPacket, String> {
    let runtime = runtime(core);
    let mut profile = profile_for_surface(core, surface, requested);
    // The engagement decides *when* a packet goes out (docs/design/47,
    // delivery posture): an unattended run rolls up into the digest, an
    // irreversible step's confirmation breaks through.
    if let Some(posture) = intent_posture {
        profile.posture = vak_delivery::DeliveryPosture::from_intent_labels(
            posture.cadence.as_str(),
            posture.urgency.as_str(),
        );
    }
    let job = DeliveryJob {
        job_id: uuid::Uuid::now_v7().to_string(),
        target: bot_id
            .filter(|id| !id.trim().is_empty())
            .map(|id| format!("{surface}:{chat}:{id}"))
            .unwrap_or_else(|| format!("{surface}:{chat}")),
        kind: DeliveryKind::Assistant,
        content: DeliveryContent::Answer({
            let artifact_suffix = session_id
                .and_then(|id| {
                    crate::projection::sandbox_artifact_markdown(&core.scope().into_root(), id)
                })
                .unwrap_or_default();
            let cleaned_markdown = crate::projection::clean_scaffolding(&markdown);
            let mut answer =
                AnswerDraft::from_markdown(format!("{cleaned_markdown}{artifact_suffix}"));
            if let Some(metadata) = outcome_metadata {
                answer.metadata.extend(metadata.clone());
                answer.document.metadata.extend(metadata);
            }
            if let Some(provenance) = provenance {
                answer.metadata.extend(provenance.clone());
                answer.document.metadata.extend(provenance);
            }
            answer
        }),
        profile,
        skill_registry: Some(merged_presentation_skills(core)),
        trace: core.admitted_trace().cloned(),
        actor: core.admitted_trace().and_then(|trace| trace.actor),
    };
    let mut packet = runtime.render(&job).await?;
    packet.attach_structured_cards(cards);
    Ok(packet)
}

/// Delivers `content` to `target` as an effect of the run `trace` names:
/// prepared before anything is sent, then sent now unless its posture
/// holds it.
pub(crate) async fn deliver(
    core: &Core,
    trace: Option<&vak_session::trace::TraceKey>,
    target: &str,
    kind: DeliveryKind,
    content: DeliveryContent,
) -> Result<DeliveryPacket, String> {
    vak_session::fence::check().map_err(|error| error.to_string())?;
    let runtime = runtime(core);
    let _serial = runtime.serial.lock().await;
    let (adapter, _) = runtime.adapters.resolve(target)?;
    let profile = apply_preferences(core, adapter.profile());
    let content = enrich_provenance(core, content);
    // Posture decides WHEN a packet goes out, never what it says. A held
    // packet is a prepared effect with its hold.
    let disposition = profile.posture.disposition(kind);
    let job = DeliveryJob {
        job_id: uuid::Uuid::now_v7().to_string(),
        target: target.into(),
        kind,
        content,
        profile: profile.clone(),
        skill_registry: Some(merged_presentation_skills(core)),
        trace: trace.cloned(),
        actor: trace.and_then(|trace| trace.actor),
    };
    let hold = match disposition {
        vak_delivery::Disposition::Send => None,
        vak_delivery::Disposition::HoldUntilComplete => Some("until_complete".to_string()),
        vak_delivery::Disposition::HoldForDigest => Some("digest".to_string()),
    };
    let payload = serde_json::to_vec(&job).map_err(|error| error.to_string())?;
    let record = core
        .effects()
        .prepare(Prepare {
            kind: EffectKind::delivery(target),
            target: target.into(),
            trace: trace.cloned(),
            payload,
            hold: hold.clone(),
            supersedes: None,
        })
        .map_err(|error| error.to_string())?;
    match hold {
        None => runtime.dispatch(core, record.id, Dispatch::Fresh).await,
        Some(hold) => Ok(vak_delivery::DeliveryPacket {
            schema_version: vak_delivery::DELIVERY_SCHEMA_VERSION,
            job_id: record.id.to_string(),
            target: job.target.clone(),
            surface: job.profile.surface.clone(),
            kind: job.kind,
            payload: vak_delivery::DeliveryPayload::Text(String::new()),
            fallback_markdown: String::new(),
            chunks: Vec::new(),
            actions: Vec::new(),
            coverage: Vec::new(),
            diagnostics: vec![format!("delivery held: {hold}")],
            presentation: None,
            trace: job.trace.clone(),
            actor: job.actor,
        }),
    }
}

/// Attach the immutable ownership envelope before a generic task, schedule, or
/// approval packet becomes an effect. Gateway rendering supplies a
/// request id as well; this common path guarantees that packets emitted by
/// internal machinery still identify the Agent and authorized audience.
fn enrich_provenance(core: &Core, mut content: DeliveryContent) -> DeliveryContent {
    let DeliveryContent::Answer(answer) = &mut content else {
        return content;
    };
    if let Some(agent) = core.agent_identity() {
        for (key, value) in [
            ("agent_id", agent.id.clone()),
            ("agent_name", agent.name.clone()),
            ("agent_revision", agent.revision.to_string()),
            ("agent_character", agent.character.clone()),
            ("agent_animation", agent.animation.clone()),
            ("agent_voice", agent.voice.clone()),
        ] {
            answer.metadata.insert(key.into(), value.clone());
            answer.document.metadata.insert(key.into(), value);
        }
    }
    if let Some(context) = core.conversation_context() {
        for (key, value) in [
            ("audience_id", context.audience_id.clone()),
            ("conversation_id", context.conversation_id.clone()),
        ] {
            answer.metadata.insert(key.into(), value.clone());
            answer.document.metadata.insert(key.into(), value);
        }
        if let Some(origin) = &context.origin {
            answer
                .metadata
                .insert("origin_surface".into(), origin.surface.clone());
            answer
                .document
                .metadata
                .insert("origin_surface".into(), origin.surface.clone());
            answer
                .metadata
                .insert("origin_address".into(), origin.address.clone());
            answer
                .document
                .metadata
                .insert("origin_address".into(), origin.address.clone());
            if let Some(bot_id) = &origin.bot_id {
                answer.metadata.insert("bot_id".into(), bot_id.clone());
                answer
                    .document
                    .metadata
                    .insert("bot_id".into(), bot_id.clone());
            }
        }
    }
    content
}

pub(crate) async fn deliver_feed_intents(
    core: &Core,
    intents: &[serde_json::Value],
) -> Result<usize, String> {
    let mut delivered = 0;
    for intent in intents {
        let target = intent
            .get("deliver_to")
            .and_then(serde_json::Value::as_str)
            .filter(|value| !value.is_empty())
            .ok_or_else(|| "feed alert delivery intent has no target".to_string())?;
        let alert_name = intent
            .get("alert_name")
            .and_then(serde_json::Value::as_str)
            .unwrap_or("Feed alert");
        let item = intent.get("item").cloned().unwrap_or_default();
        let title = item.get("title").and_then(Value::as_str).unwrap_or("");
        let source = item
            .get("source_name")
            .and_then(Value::as_str)
            .unwrap_or("");
        let url = item.get("url").and_then(Value::as_str).unwrap_or("");
        let reasons = intent
            .get("match")
            .and_then(|value| value.get("reasons"))
            .and_then(Value::as_array)
            .map(|values| {
                values
                    .iter()
                    .filter_map(Value::as_str)
                    .collect::<Vec<_>>()
                    .join(", ")
            })
            .unwrap_or_default();
        let markdown = format!(
            "**Feed Alert: {alert_name}**\n\n**{title}**\nSource: {source}\nURL: {url}\nReasons: {reasons}"
        );
        deliver(
            core,
            core.admitted_trace(),
            target,
            DeliveryKind::Alert,
            DeliveryContent::Text { markdown },
        )
        .await?;
        delivered += 1;
    }
    Ok(delivered)
}

/// Sends what is waiting, every 30 seconds: first records as unknown
/// what a stopped process was sending (never sent again by itself) and as
/// failed an approved mail or calendar change its request never sent,
/// then takes each queued delivery and each one proven not sent with
/// attempts left. Held effects wait.
pub(crate) fn start_replay(core: &Core) {
    let core = core.clone();
    tokio::spawn(async move {
        let runtime = runtime(&core);
        let mut interval = tokio::time::interval(Duration::from_secs(30));
        loop {
            interval.tick().await;
            // A fenced process dispatches nothing: the restored store's
            // writer owns the effects now.
            if vak_session::fence::is_fenced() {
                continue;
            }
            let _serial = runtime.serial.lock().await;
            let effects = core.effects();
            let now = chrono::Utc::now();
            if let Err(error) = effects.recover(now) {
                tracing::warn!(error_kind = %vak_telemetry::error_kind(&error), "effect recovery failed");
                continue;
            }
            // A mail or calendar change is approved for five minutes; one
            // its request never sent is failed, not left waiting.
            if let Err(error) = effects.fail_unsent(now, chrono::Duration::minutes(5)) {
                tracing::warn!(error_kind = %vak_telemetry::error_kind(&error), "unsent approved changes were not settled");
            }
            let waiting = match effects.dispatchable() {
                Ok(waiting) => waiting,
                Err(error) => {
                    tracing::warn!(error_kind = %vak_telemetry::error_kind(&error), "the effect scan failed");
                    continue;
                }
            };
            for record in waiting.into_iter().take(100) {
                if let Err(error) = runtime.dispatch(&core, record.id, Dispatch::Fresh).await {
                    let _ = error;
                    tracing::warn!(effect = %record.id, "an effect was not sent");
                }
            }
        }
    });
}

/// What the owner asked for when they chose Send again.
pub(crate) enum Resent {
    /// The same effect, sent again under its key to a provider that drops
    /// repeats, or one proven not sent, tried now.
    Same(EffectId),
    /// A new effect that supersedes it.
    New(EffectId),
}

/// Sends an effect again for the owner. An unknown effect goes again
/// under the same key only where the provider drops a repeat of it
/// (Discord's nonce); anywhere else it is superseded by a new effect,
/// because sending it again under its key could deliver it twice. An
/// effect proven not sent, or still queued, is tried now.
pub(crate) async fn resend(core: &Core, id: EffectId) -> Result<Resent, String> {
    vak_session::fence::check().map_err(|error| error.to_string())?;
    runtime(core).resend(core, id).await
}

impl DeliveryRuntime {
    async fn resend(&self, core: &Core, id: EffectId) -> Result<Resent, String> {
        let _serial = self.serial.lock().await;
        let effects = core.effects();
        let record = effects
            .get(id)
            .map_err(|error| error.to_string())?
            .ok_or_else(|| format!("no effect {id}"))?;
        use vak_session::effects::EffectStatus as S;
        if !record.kind.retries() {
            return Err("a mail or calendar change is sent again only from a new review".into());
        }
        let (sent, outcome) = match record.status {
            S::Queued | S::Retrying => (
                self.dispatch(core, id, Dispatch::Fresh).await,
                Resent::Same(id),
            ),
            S::Unknown if self.dedupes(&record.target) => (
                self.dispatch(core, id, Dispatch::Dedupe).await,
                Resent::Same(id),
            ),
            S::Unknown | S::Failed => {
                let next = effects.send_again(id).map_err(|error| error.to_string())?;
                (
                    self.dispatch(core, next.id, Dispatch::Fresh).await,
                    Resent::New(next.id),
                )
            }
            S::Held => return Err("it is waiting for its digest".into()),
            S::Sending => return Err("it is being sent now".into()),
            S::Sent => return Err("it was sent".into()),
            S::Superseded => return Err("it was already sent again".into()),
        };
        sent.map(|_| outcome)
    }
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
            posture: vak_delivery::DeliveryPosture::default(),
        }
    }

    async fn send(
        &self,
        core: &Core,
        packet: &DeliveryPacket,
        _key: &str,
    ) -> Result<Option<String>, SendFailure> {
        let path = core.shared_scope().gateway_deliveries();
        let mut line = serde_json::json!({
            "ts": chrono::Utc::now().to_rfc3339(),
            "target": packet.target,
            "text": packet.fallback_markdown,
            "job_id": packet.job_id,
            "delivery": packet,
        });
        if let Some(trace) = &packet.trace {
            line["trace"] = serde_json::json!(trace);
        }
        if let Some(actor) = &packet.actor {
            line["actor"] = serde_json::json!(actor);
        }
        vak_session::chain::RecordChain::at(path)
            .append(&line)
            .map(|()| None)
            .map_err(|error| SendFailure::not_sent(format!("append deliveries log: {error}")))
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
            posture: vak_delivery::DeliveryPosture::default(),
        }
    }

    async fn send(
        &self,
        core: &Core,
        packet: &DeliveryPacket,
        key: &str,
    ) -> Result<Option<String>, SendFailure> {
        let (_, name) = packet
            .target
            .split_once(':')
            .ok_or_else(|| SendFailure::not_sent("webhook target has no name"))?;
        super::gateway::deliver_webhook_packet(core, name, packet, key)
            .await
            .map(|()| None)
    }

    /// A 2xx says the receiver took it, not what it did with it.
    fn confirms(&self) -> bool {
        false
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
            posture: vak_delivery::DeliveryPosture::default(),
        }
    }

    async fn send(
        &self,
        _core: &Core,
        packet: &DeliveryPacket,
        _key: &str,
    ) -> Result<Option<String>, SendFailure> {
        let (_, chat_id) = packet
            .target
            .split_once(':')
            .ok_or_else(|| SendFailure::not_sent("telegram target has no chat id"))?;
        let chunks: Vec<&str> = if packet.chunks.is_empty() {
            vec![packet.fallback_markdown.as_str()]
        } else {
            packet.chunks.iter().map(String::as_str).collect()
        };
        let client = reqwest::Client::new();
        let last = chunks.len().saturating_sub(1);
        let mut message_id = None;
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
                .map_err(|error| SendFailure::transport("telegram sendMessage", &error, i > 0))?;
            if !resp.status().is_success() {
                return Err(SendFailure::status(
                    "telegram sendMessage",
                    resp.status(),
                    i > 0,
                ));
            }
            let answer: serde_json::Value = resp.json().await.unwrap_or_default();
            message_id = answer["result"]["message_id"]
                .as_i64()
                .map(|id| id.to_string())
                .or(message_id);
        }
        Ok(message_id)
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
        built_in_surface_profile("discord")
    }

    /// Every message carries a nonce made from the effect's key and its
    /// index, with `enforce_nonce`, so Discord returns the message it
    /// already made rather than making a second one when the same effect
    /// is sent again.
    async fn send(
        &self,
        _core: &Core,
        packet: &DeliveryPacket,
        key: &str,
    ) -> Result<Option<String>, SendFailure> {
        let (_, channel_id) = packet
            .target
            .split_once(':')
            .ok_or_else(|| SendFailure::not_sent("discord target has no channel id"))?;
        let url = format!("{}/channels/{channel_id}/messages", self.api_base);
        let mut message_id = post_chunks(
            packet,
            |chunk, index, last| {
                let mut content = chunk.to_string();
                if last && let Some(prompt) = typed_verdict_prompt(&packet.actions) {
                    content.push_str(&prompt);
                }
                (
                    url.clone(),
                    serde_json::json!({
                        "content": content,
                        "allowed_mentions": {"parse": []},
                        "nonce": discord_nonce(key, index),
                        "enforce_nonce": true,
                    }),
                )
            },
            |request| request.header("Authorization", format!("Bot {}", self.bot_token)),
            "discord createMessage",
        )
        .await?;
        let cards = packet
            .structured_cards()
            .into_iter()
            .cloned()
            .collect::<Vec<_>>();
        if !cards.is_empty() {
            let index = packet.chunks.len().max(1);
            let response = reqwest::Client::new()
                .post(&url)
                .header("Authorization", format!("Bot {}", self.bot_token))
                .json(&serde_json::json!({
                    "embeds": vak_delivery::discord::structured_card_embeds(&cards),
                    "allowed_mentions": {"parse": []},
                    "nonce": discord_nonce(key, index),
                    "enforce_nonce": true,
                }))
                .send()
                .await
                .map_err(|error| SendFailure::transport("discord createMessage", &error, true))?;
            if !response.status().is_success() {
                return Err(SendFailure::status(
                    "discord createMessage",
                    response.status(),
                    true,
                ));
            }
            let answer: serde_json::Value = response.json().await.unwrap_or_default();
            message_id = answer["id"].as_str().map(str::to_string).or(message_id);
        }
        Ok(message_id)
    }

    fn dedupes(&self) -> bool {
        true
    }
}

/// The nonce of one message of an effect: its key and the message's
/// index, within Discord's 25 characters.
fn discord_nonce(key: &str, index: usize) -> String {
    format!("{key}{index}").chars().take(25).collect()
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
        built_in_surface_profile("slack")
    }

    async fn send(
        &self,
        _core: &Core,
        packet: &DeliveryPacket,
        _key: &str,
    ) -> Result<Option<String>, SendFailure> {
        let (_, channel_id) = packet
            .target
            .split_once(':')
            .ok_or_else(|| SendFailure::not_sent("slack target has no channel id"))?;
        let url = format!("{}/chat.postMessage", self.api_base);
        let mut message_ts = post_chunks(
            packet,
            |chunk, _, last| {
                let mut text = chunk.to_string();
                if last && let Some(prompt) = typed_verdict_prompt(&packet.actions) {
                    text.push_str(&prompt);
                }
                (
                    url.clone(),
                    serde_json::json!({ "channel": channel_id, "text": text }),
                )
            },
            |request| request.bearer_auth(&self.bot_token),
            "slack chat.postMessage",
        )
        .await?;
        let cards = packet
            .structured_cards()
            .into_iter()
            .cloned()
            .collect::<Vec<_>>();
        if !cards.is_empty() {
            let fallback = cards
                .iter()
                .map(|card| {
                    let source = vak_delivery::structured_markdown(card);
                    source
                        .split_once("\n\n```json")
                        .map_or(source.as_str(), |(text, _)| text)
                        .to_owned()
                })
                .collect::<Vec<_>>()
                .join("\n\n");
            let operation = "slack chat.postMessage";
            let response = reqwest::Client::new()
                .post(&url)
                .bearer_auth(&self.bot_token)
                .json(&serde_json::json!({
                    "channel": channel_id,
                    "text": fallback.chars().take(2500).collect::<String>(),
                    "blocks": vak_delivery::slack::structured_card_blocks(&cards),
                    "parse": "none", "link_names": false,
                }))
                .send()
                .await
                .map_err(|error| SendFailure::transport(operation, &error, true))?;
            if !response.status().is_success() {
                return Err(SendFailure::status(operation, response.status(), true));
            }
            message_ts = slack_answer(operation, response, true)
                .await?
                .or(message_ts);
        }
        Ok(message_ts)
    }
}

/// Slack answers 200 with `ok: false` when it refused a message, so the
/// body decides; an unreadable body after a 200 may have been taken.
async fn slack_answer(
    operation: &str,
    response: reqwest::Response,
    sent_before: bool,
) -> Result<Option<String>, SendFailure> {
    let result: serde_json::Value = response.json().await.map_err(|error| SendFailure {
        reason: format!("{operation} response: {error}"),
        landed: Landed::Unknown,
    })?;
    if result["ok"].as_bool() != Some(true) {
        return Err(SendFailure {
            reason: format!(
                "{operation} not ok: {}",
                result["error"].as_str().unwrap_or("?")
            ),
            landed: if sent_before {
                Landed::Partly
            } else {
                Landed::No
            },
        });
    }
    Ok(result["ts"].as_str().map(str::to_string))
}

/// Shared chunk-and-POST loop for Discord and Slack: `body` builds the
/// (url, json) for one chunk from its index and whether it is the last,
/// `auth` applies the surface's auth header. Returns the provider's id of
/// the last message.
async fn post_chunks(
    packet: &DeliveryPacket,
    body: impl Fn(&str, usize, bool) -> (String, serde_json::Value),
    auth: impl Fn(reqwest::RequestBuilder) -> reqwest::RequestBuilder,
    operation: &str,
) -> Result<Option<String>, SendFailure> {
    let chunks: Vec<&str> = if packet.chunks.is_empty() {
        vec![packet.fallback_markdown.as_str()]
    } else {
        packet.chunks.iter().map(String::as_str).collect()
    };
    let client = reqwest::Client::new();
    let last = chunks.len().saturating_sub(1);
    let mut id = None;
    for (i, chunk) in chunks.iter().enumerate() {
        let (url, json) = body(chunk, i, i == last);
        let resp = auth(client.post(url))
            .json(&json)
            .send()
            .await
            .map_err(|error| SendFailure::transport(operation, &error, i > 0))?;
        if !resp.status().is_success() {
            return Err(SendFailure::status(operation, resp.status(), i > 0));
        }
        if operation.starts_with("slack ") {
            id = slack_answer(operation, resp, i > 0).await?.or(id);
        } else {
            let answer: serde_json::Value = resp.json().await.unwrap_or_default();
            id = answer["id"].as_str().map(str::to_string).or(id);
        }
    }
    Ok(id)
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
    fn generic_answer_delivery_carries_agent_conversation_provenance() {
        let dir = tempfile::tempdir().unwrap();
        let core = Core::new(dir.path().to_path_buf())
            .unwrap()
            .with_agent_identity(Some(vak_session::types::AgentIdentity {
                id: "support".into(),
                revision: 2,
                name: "Support".into(),
                character: "vak".into(),
                personality: String::new(),
                animation: "subtle".into(),
                voice: "default".into(),
                behaviour: String::new(),
                responsibilities: String::new(),
                instructions: String::new(),
            }))
            .with_conversation_context(Some(vak_session::ConversationContext {
                conversation_id: "conv-1".into(),
                audience_id: "aud-1".into(),
                origin: Some(vak_session::ConversationOrigin {
                    surface: "telegram".into(),
                    address: "chat-1".into(),
                    bot_id: Some("support-bot".into()),
                }),
            }));
        let content = enrich_provenance(
            &core,
            DeliveryContent::Answer(AnswerDraft::from_markdown("result")),
        );
        let answer = match content {
            DeliveryContent::Answer(answer) => answer,
            other => {
                assert!(
                    matches!(other, DeliveryContent::Answer(_)),
                    "answer content expected"
                );
                return;
            }
        };
        assert_eq!(
            answer.metadata.get("agent_id").map(String::as_str),
            Some("support")
        );
        assert_eq!(
            answer.metadata.get("agent_name").map(String::as_str),
            Some("Support")
        );
        assert_eq!(
            answer.metadata.get("agent_character").map(String::as_str),
            Some("vak")
        );
        assert_eq!(
            answer.metadata.get("agent_revision").map(String::as_str),
            Some("2")
        );
        assert_eq!(
            answer
                .document
                .metadata
                .get("agent_voice")
                .map(String::as_str),
            Some("default")
        );
        assert_eq!(
            answer.metadata.get("audience_id").map(String::as_str),
            Some("aud-1")
        );
        assert_eq!(
            answer.metadata.get("conversation_id").map(String::as_str),
            Some("conv-1")
        );
        assert_eq!(
            answer.metadata.get("bot_id").map(String::as_str),
            Some("support-bot")
        );
        assert_eq!(
            answer
                .document
                .metadata
                .get("origin_surface")
                .map(String::as_str),
            Some("telegram")
        );
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
                (
                    ("telegram".to_string(), "VakBot".to_string()),
                    vakbot.clone(),
                ),
                (
                    ("telegram".to_string(), "Vakyartha".to_string()),
                    vakyartha.clone(),
                ),
            ]),
        };

        let (adapter_a, address_a) = registry.resolve("telegram:8846301562:VakBot").unwrap();
        assert_eq!(address_a, "8846301562");
        assert!(
            Arc::ptr_eq(&adapter_a, &vakbot),
            "must pick VakBot's own adapter"
        );
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

    /// A chat target with no bot id is refused, not resolved. Picking
    /// "some bot on this surface" replies under an identity the chat was
    /// never bound to (AGENTS.md invariant 24).
    #[test]
    fn a_chat_target_with_no_bot_id_is_refused() {
        let registry = AdapterRegistry {
            adapters: HashMap::new(),
            bot_adapters: HashMap::from([(
                ("telegram".to_string(), "VakBot".to_string()),
                telegram_adapter("vakbot-token"),
            )]),
        };
        let err = registry
            .resolve("telegram:8846301562")
            .map(|_| ())
            .unwrap_err();
        assert!(err.contains("names no bot"), "{err}");
        assert!(err.contains("<bot_id>"), "{err}");
    }

    /// `log` and `webhook` are not bots, so they keep the two-part shape.
    #[test]
    fn non_chat_surfaces_still_resolve_without_a_bot_id() {
        let mut registry = AdapterRegistry {
            adapters: HashMap::new(),
            bot_adapters: HashMap::new(),
        };
        registry.register(LogAdapter);
        let (_, address) = registry.resolve("log:anywhere").unwrap();
        assert_eq!(address, "anywhere");
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

    /// An unknown Discord send goes again as the same effect with the same
    /// nonce and `enforce_nonce`, so Discord returns the message it already
    /// made instead of posting a second one.
    #[tokio::test]
    async fn discord_resend_reuses_nonce() {
        use std::sync::atomic::{AtomicUsize, Ordering};
        use vak_session::effects::EffectStatus;
        let bodies: Arc<Mutex<Vec<serde_json::Value>>> = Arc::default();
        let calls = Arc::new(AtomicUsize::new(0));
        let app = axum::Router::new().route(
            "/channels/{channel}/messages",
            axum::routing::post({
                let bodies = bodies.clone();
                let calls = calls.clone();
                move |axum::Json(body): axum::Json<serde_json::Value>| {
                    let bodies = bodies.clone();
                    let calls = calls.clone();
                    async move {
                        bodies.lock().unwrap().push(body);
                        // The first post fails after Discord may have taken it.
                        if calls.fetch_add(1, Ordering::SeqCst) == 0 {
                            (
                                axum::http::StatusCode::BAD_GATEWAY,
                                axum::Json(serde_json::json!({})),
                            )
                        } else {
                            (
                                axum::http::StatusCode::OK,
                                axum::Json(serde_json::json!({ "id": "m-1" })),
                            )
                        }
                    }
                }
            }),
        );
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });

        let dir = tempfile::tempdir().unwrap();
        let core = Core::new(dir.path().to_path_buf()).unwrap();
        let mut adapters = AdapterRegistry {
            adapters: HashMap::new(),
            bot_adapters: HashMap::new(),
        };
        adapters.register_bot(
            "discord",
            "bot".into(),
            DiscordAdapter {
                bot_token: "t".into(),
                api_base: format!("http://{addr}"),
            },
        );
        let runtime = DeliveryRuntime {
            worker: None,
            adapters,
            serial: tokio::sync::Mutex::new(()),
        };
        let target = "discord:chan:bot";
        let job = DeliveryJob {
            job_id: "j".into(),
            target: target.into(),
            kind: DeliveryKind::Assistant,
            content: DeliveryContent::Answer(AnswerDraft::from_markdown("hello")),
            profile: built_in_surface_profile("discord"),
            skill_registry: None,
            trace: None,
            actor: None,
        };
        let effect = core
            .effects()
            .prepare(Prepare {
                kind: EffectKind::delivery(target),
                target: target.into(),
                trace: None,
                payload: serde_json::to_vec(&job).unwrap(),
                hold: None,
                supersedes: None,
            })
            .unwrap();
        assert!(
            runtime
                .dispatch(&core, effect.id, Dispatch::Fresh)
                .await
                .is_err()
        );
        let unknown = core.effects().get(effect.id).unwrap().unwrap();
        assert_eq!(unknown.status, EffectStatus::Unknown);

        let resent = runtime.resend(&core, effect.id).await.unwrap();
        assert!(matches!(resent, Resent::Same(id) if id == effect.id));
        let sent = core.effects().get(effect.id).unwrap().unwrap();
        assert_eq!(sent.status, EffectStatus::Sent);
        assert_eq!(sent.attempts, 2);
        assert_eq!(
            sent.receipt
                .and_then(|receipt| receipt.provider_id)
                .as_deref(),
            Some("m-1")
        );
        let bodies = bodies.lock().unwrap();
        assert_eq!(bodies.len(), 2);
        assert_eq!(bodies[0]["nonce"], bodies[1]["nonce"]);
        assert_eq!(bodies[1]["enforce_nonce"], true);
        let nonce = bodies[0]["nonce"].as_str().unwrap();
        assert!(nonce.starts_with(&effect.idempotency_key) && nonce.len() <= 25);
    }

    /// Anywhere a provider does not drop repeats, an unknown send is sent
    /// again only as a new effect that supersedes it.
    #[tokio::test]
    async fn unknown_send_elsewhere_is_superseded_not_resent() {
        use vak_session::effects::EffectStatus;
        let dir = tempfile::tempdir().unwrap();
        let core = Core::new(dir.path().to_path_buf()).unwrap();
        let mut adapters = AdapterRegistry {
            adapters: HashMap::new(),
            bot_adapters: HashMap::new(),
        };
        adapters.register(LogAdapter);
        let runtime = DeliveryRuntime {
            worker: None,
            adapters,
            serial: tokio::sync::Mutex::new(()),
        };
        let target = "log:main";
        let job = DeliveryJob {
            job_id: "j".into(),
            target: target.into(),
            kind: DeliveryKind::Assistant,
            content: DeliveryContent::Answer(AnswerDraft::from_markdown("hello")),
            profile: DeliveryProfile::plain("log"),
            skill_registry: None,
            trace: None,
            actor: None,
        };
        let effects = core.effects();
        let effect = effects
            .prepare(Prepare {
                kind: EffectKind::delivery(target),
                target: target.into(),
                trace: None,
                payload: serde_json::to_vec(&job).unwrap(),
                hold: None,
                supersedes: None,
            })
            .unwrap();
        effects
            .begin_dispatch(effect.id, Dispatch::Fresh)
            .unwrap()
            .unwrap();
        effects.unknown(effect.id, "timed out").unwrap();
        let Resent::New(next) = runtime.resend(&core, effect.id).await.unwrap() else {
            unreachable!("a log send is never resent under its key");
        };
        assert_eq!(
            effects.get(effect.id).unwrap().unwrap().status,
            EffectStatus::Superseded
        );
        let next = effects.get(next).unwrap().unwrap();
        assert_eq!(next.status, EffectStatus::Sent);
        assert_eq!(next.supersedes, Some(effect.id));
    }
}
