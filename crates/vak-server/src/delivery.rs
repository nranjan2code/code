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
    AnswerDraft, DeliveryContent, DeliveryJob, DeliveryKind, DeliveryPacket, DeliveryProfile,
    Markup,
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
    adapters: HashMap<&'static str, Arc<dyn ChannelAdapter>>,
}

impl AdapterRegistry {
    fn built_in() -> Self {
        let mut registry = Self {
            adapters: HashMap::new(),
        };
        registry.register(LogAdapter);
        registry.register(WebhookAdapter);
        registry
    }

    fn register(&mut self, adapter: impl ChannelAdapter + 'static) {
        self.adapters.insert(adapter.scheme(), Arc::new(adapter));
    }

    fn resolve(&self, target: &str) -> Result<(Arc<dyn ChannelAdapter>, String), String> {
        let Some((scheme, address)) = target.split_once(':') else {
            return Err(format!(
                "invalid delivery target '{target}': expected '<surface>:<address>'"
            ));
        };
        if address.trim().is_empty() {
            return Err(format!("delivery target '{target}' has an empty address"));
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
            adapters: AdapterRegistry::built_in(),
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
        let packet = self.render(&record.job).await?;
        let (adapter, _) = self.adapters.resolve(&record.job.target)?;
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
        &core.cwd().join(".vakcoder").join("output.toml"),
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
