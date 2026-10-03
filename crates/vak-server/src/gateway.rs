//! Gateway: route always-on surfaces (chat adapters, webhooks, cron) to
//! persistent agent sessions. See docs/design/22-gateway.md.
//!
//! A surface speaks HTTP: `POST /gateway/inbound` carries `{surface, chat,
//! sender, text}` and either returns immediately or long-polls the final
//! assistant text (`wait`). Bindings persist `(surface:chat) -> session_id`
//! under `<home>/gateway/bindings.json` so conversations survive restarts.
//!
//! Unattended turns fail closed: approval gates are auto-denied and the
//! denial is fed back to the model as a tool error. Messages arriving while
//! a turn runs are queued as logged steering input and consumed between
//! model steps — nothing typed while busy is ever dropped.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use crate::inbox::{self, save_to_inbox};

use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::response::IntoResponse;
use axum::{Json, Router};
use tokio::sync::oneshot;

use vak_agent::{AgentEvent, AutoDeny};
use vak_core::Core;
use vak_delivery::{
    AnswerDraft, ApprovalPayload, DeliveryAction, DeliveryContent, DeliveryKind, DeliveryPacket,
};

use crate::{AppState, SessionHandle};

/// One-shot guard so the forward-mode-but-FullAccess warning does not spam
/// stderr on every inbound turn.
static FORWARD_FULLACCESS_WARNED: std::sync::atomic::AtomicBool =
    std::sync::atomic::AtomicBool::new(false);

/// Contract every inbound channel bridge (Telegram today; Slack, Discord,
/// ... later) must satisfy before calling `POST /gateway/inbound` (0c-03).
/// `gateway.chat_allowlist` and the per-conversation session binding are
/// only as strong as `chat`/`sender` being the real remote identity — a
/// bridge that reuses one fixed value for every user would silently merge
/// every stranger into one session and defeat the allowlist outright.
/// Route new bridges through `InboundRequest::new` rather than hand-rolling
/// the JSON body so that mistake fails loudly instead of shipping quietly.
pub trait InboundChannel {
    /// Stable lowercase surface name ("telegram", "slack", ...) — the
    /// first half of the `surface:chat` allowlist key.
    fn surface(&self) -> &'static str;
}

/// Validated `{surface, chat, sender, text}` payload for one inbound
/// message, built via [`InboundRequest::new`].
#[derive(Debug, Clone, serde::Serialize)]
pub struct InboundRequest {
    pub surface: String,
    pub chat: String,
    pub sender: String,
    pub text: String,
    #[serde(default)]
    pub attachments: Vec<serde_json::Value>,
    #[serde(default)]
    pub wait: bool,
    /// Which configured bot this bridge process is running as
    /// (multi-bot-per-channel, docs/design/34), when it knows — set via
    /// `--bot-id` on the CLI bridge. Lets a chat's first-sight pending
    /// entry record the bot that actually delivered it, instead of
    /// forcing the operator to pick one by hand for a fact the bridge
    /// already had.
    #[serde(default)]
    pub bot_id: Option<String>,
    #[serde(default)]
    pub request_id: Option<String>,
    /// What the bridge can show and send, when it says.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub capabilities: Option<crate::delivery::RequestedCapabilities>,
}

impl InboundRequest {
    /// Rejects the two shapes a careless bridge tends to produce before it
    /// has wired up real per-user identity: an empty `chat`/`sender`, or
    /// one that is literally the surface name (a copy-pasted placeholder).
    pub fn new(
        channel: &impl InboundChannel,
        chat: impl Into<String>,
        sender: impl Into<String>,
        text: impl Into<String>,
    ) -> Result<Self, String> {
        let surface = channel.surface().to_string();
        let chat = chat.into();
        let sender = sender.into();
        if chat.trim().is_empty() {
            return Err(format!("{surface} bridge: chat key must not be empty"));
        }
        if sender.trim().is_empty() {
            return Err(format!("{surface} bridge: sender id must not be empty"));
        }
        if chat.trim() == surface || sender.trim() == surface {
            return Err(format!(
                "{surface} bridge: chat/sender must be the remote identity, not the surface name itself"
            ));
        }
        Ok(Self {
            surface,
            chat,
            sender,
            text: text.into(),
            attachments: Vec::new(),
            wait: false,
            bot_id: None,
            request_id: None,
            capabilities: None,
        })
    }

    pub fn with_attachments(mut self, attachments: Vec<serde_json::Value>) -> Self {
        const MAX_AUDIO_ATTACHMENT_BYTES: usize = 16 * 1024 * 1024;
        self.attachments = attachments
            .into_iter()
            .map(|mut attachment| {
                if attachment.get("kind").and_then(|v| v.as_str()) == Some("audio") {
                    let encoded = attachment
                        .get("data")
                        .and_then(|v| v.as_str())
                        .unwrap_or_default();
                    if encoded.len() > MAX_AUDIO_ATTACHMENT_BYTES.saturating_mul(4) / 3 {
                        attachment["data"] = serde_json::Value::String(String::new());
                        attachment["error"] =
                            serde_json::Value::String("audio attachment exceeds 16 MiB".into());
                    }
                }
                attachment
            })
            .collect();
        self
    }

    pub fn waiting(mut self) -> Self {
        self.wait = true;
        self
    }

    /// The bridge sends files to the chat, so a turn's Office drafts come
    /// back in the reply's `files` rather than as a note.
    pub fn accepting_files(mut self) -> Self {
        self.capabilities
            .get_or_insert_with(Default::default)
            .accepts_files = Some(true);
        self
    }

    /// Tag this request with the bot identity the bridge is running as, if
    /// any (see the `bot_id` field doc).
    pub fn with_bot_id(mut self, bot_id: Option<String>) -> Self {
        self.bot_id = bot_id;
        self
    }

    /// Attach the bridge's durable idempotency key when the upstream
    /// transport provides one (Telegram update id, webhook event id, etc.).
    pub fn with_request_id(mut self, request_id: Option<String>) -> Self {
        self.request_id = request_id;
        self
    }
}

/// Long-poll ceiling for `wait: true` inbound messages.
const WAIT_TIMEOUT: Duration = Duration::from_secs(240);

/// Upper bound on one background reflection pass (docs/design/29 P1) so a
/// stuck auxiliary stream cannot hold the session ledger indefinitely.
const REFLECTION_CALL_TIMEOUT: Duration = Duration::from_secs(120);

fn bindings_path(home: &std::path::Path) -> PathBuf {
    vak_config::scope::SharedScope::new(home).gateway_bindings()
}

fn allowlist_path(home: &std::path::Path) -> PathBuf {
    vak_config::scope::SharedScope::new(home).gateway_allowlist()
}

fn bots_path(home: &std::path::Path) -> PathBuf {
    vak_config::scope::SharedScope::new(home).gateway_bots()
}

/// Multi-bot-per-channel (docs/design/34 Phase 5 follow-up): an allowlist
/// key is `surface:chat` for a legacy/single-bot chat, or
/// `surface:chat:bot_id` once a specific bot is scoped into it — the bot id
/// is the third segment precisely so [`legacy_key_for`] can strip it back
/// off. Returns `None` for a key that is already legacy-shaped (nothing to
/// strip) or malformed.
fn legacy_key_for(key: &str) -> Option<String> {
    let mut parts = key.splitn(3, ':');
    let surface = parts.next()?;
    let chat = parts.next()?;
    parts.next()?; // only a genuinely 3-part (bot-scoped) key has a legacy form
    Some(format!("{surface}:{chat}"))
}

/// Read-only lookup of one bot's token env var by id, straight from
/// `bots.json`, without needing a running `GatewayState` — the `vak
/// telegram/discord/slack --bot-id` CLI bridges are separate short-lived
/// processes that never construct one, but still need to resolve which env
/// var holds their token (docs/design/34, multi-bot).
pub fn bot_token_env_for_id(sessions_home: &std::path::Path, id: &str) -> Option<String> {
    let raw = std::fs::read_to_string(bots_path(sessions_home)).ok()?;
    let file: BotsFile = serde_json::from_str(&raw).ok()?;
    file.bots
        .into_iter()
        .find(|b| b.id == id)
        .map(|b| b.token_env)
}

/// Truncation cap for `first_seen_text` on a freshly pending entry — kept
/// only for operator review, never used as agent input.
const FIRST_SEEN_TEXT_MAX_CHARS: usize = 500;

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AllowlistStatus {
    Pending,
    Allowed,
    Denied,
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct AllowlistRoute {
    pub provider: String,
    pub model: String,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct AllowlistEntry {
    pub key: String,
    pub status: AllowlistStatus,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub workspace: Option<PathBuf>,
    /// Agent selected for this endpoint. Missing values are normalized to the
    /// reserved Vakyartha identity when loading older allowlist rows.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub agent_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub route: Option<AllowlistRoute>,
    /// Voice/persona override for this chat. `None` inherits the bound
    /// bot's (or workspace default's) voice, gated the same as `route` by
    /// `inherit_bot_policy`. See `GatewayState::core_for_entry`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub voice: Option<vak_config::VoiceConfig>,
    /// Per-channel permission mode (docs/design/34 "Per-channel permission
    /// mode"). `None` inherits the target workspace's own configured mode,
    /// which is the pre-existing behavior and stays the default. `Some(m)`
    /// pins this channel to `m` — but only ever as a *reduction*: the pool
    /// caps it to the workspace's own resolved mode, so an override can
    /// never grant more than a local `vak` run in that workspace has.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub permission_mode: Option<vak_config::PermissionMode>,
    /// Per-channel capability restrictions. Each `None` field inherits the
    /// selected workspace; an explicit empty allow list denies that class.
    #[serde(default, skip_serializing_if = "is_default_channel_policy")]
    pub policy: vak_config::ChannelPolicy,
    /// Which `Bot` this chat is bound to, when the surface has more than
    /// one. `None` keeps today's behavior (surface's sole/legacy bot).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub bot_id: Option<String>,
    /// Whether this chat inherits its bot's policy/permission_mode/route as
    /// a tier below its own (default) or resolves purely against the
    /// workspace, ignoring the bot entirely — the explicit "break
    /// inheritance" switch. Meaningless when `bot_id` is `None`.
    #[serde(default = "default_true")]
    pub inherit_bot_policy: bool,
    pub added_at: String,
    pub added_by: String,
    /// Only meaningful while `status == Pending` — the first message text
    /// that triggered this entry, truncated for operator review.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub first_seen_text: Option<String>,
    /// Prompt tier for this chat, the narrowest gateway layer
    /// (docs/design/45). Gated by `inherit_bot_policy` the same way `voice`
    /// and `route` are.
    #[serde(
        default,
        skip_serializing_if = "vak_core::prompts::LayerContent::is_empty"
    )]
    pub prompt: vak_core::prompts::LayerContent,
}

pub(crate) fn default_true() -> bool {
    true
}

/// Deserializer for a PATCH field shaped `Option<Option<T>>`, where the
/// three JSON states must stay distinguishable: the key absent ("leave
/// this alone"), the key present as `null` ("clear it"), and the key
/// present with a value ("set it"). A plain `Option<Option<T>>` field
/// cannot do this on its own — serde's derived `deserialize_option` maps
/// JSON `null` to the *outer* `None`, identical to the key being absent,
/// so "explicit null clears it" silently never fires
/// (<https://github.com/serde-rs/serde/issues/984>). Pair with
/// `#[serde(default, deserialize_with = "deserialize_present")]`: the
/// `default` only ever applies when the key is missing entirely (serde
/// skips `deserialize_with` in that case), and this function itself
/// wraps whatever it sees — including a `null` that becomes `Some(None)`
/// — in the outer `Some`.
pub(crate) fn deserialize_present<'de, T, D>(deserializer: D) -> Result<Option<T>, D::Error>
where
    T: serde::Deserialize<'de>,
    D: serde::Deserializer<'de>,
{
    T::deserialize(deserializer).map(Some)
}

fn is_default_channel_policy(policy: &vak_config::ChannelPolicy) -> bool {
    policy == &vak_config::ChannelPolicy::default()
}

#[derive(serde::Serialize, serde::Deserialize)]
struct AllowlistFile {
    schema: u32,
    entries: Vec<AllowlistEntry>,
}

/// A gateway bot identity: one credential/token slot, independently
/// addressable even when it shares a `surface` with other bots. Sits
/// between the workspace and a chat's `AllowlistEntry` in the
/// policy/permission/route resolution chain (`core_for_entry`,
/// `resolve_channel_permission`) — see `vak_config::ChannelPolicy::merge`
/// and `vak_config::PermissionMode::capped_by`.
#[derive(Debug, Clone, Default, serde::Serialize, serde::Deserialize)]
pub struct Bot {
    /// Stable slug, e.g. "telegram-support". Chosen at creation, immutable.
    pub id: String,
    /// "telegram" | "discord" | "slack".
    pub surface: String,
    /// Operator-facing name shown in the admin console.
    pub label: String,
    /// Name of the env var holding this bot's token. The token value
    /// itself is never stored here or returned by the admin API.
    pub token_env: String,
    /// Default agent identity this bot binds to (e.g. "support", "researcher").
    /// If unset, resolves to "vak" (built-in default agent).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub agent_id: Option<String>,
    #[serde(default, skip_serializing_if = "is_default_channel_policy")]
    pub policy: vak_config::ChannelPolicy,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub permission_mode: Option<vak_config::PermissionMode>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub route: Option<AllowlistRoute>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub workspace: Option<PathBuf>,
    /// Voice/persona override for this bot's spoken replies. `None`
    /// inherits the workspace default (no voice); `Some` sets this bot's
    /// tier for any chat that inherits it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub voice: Option<vak_config::VoiceConfig>,
    /// Prompt tier for this bot (docs/design/45). Identity and rules fall
    /// through to the chat tier below; guardrails concatenate and cannot be
    /// removed by anything narrower.
    #[serde(
        default,
        skip_serializing_if = "vak_core::prompts::LayerContent::is_empty"
    )]
    pub prompt: vak_core::prompts::LayerContent,
}

#[derive(serde::Serialize, serde::Deserialize, Default)]
struct BotsFile {
    schema: u32,
    bots: Vec<Bot>,
}

/// Outcome of resolving an inbound key against the allowlist store, so the
/// caller can distinguish "just became pending" from "still pending" from
/// a flat denial without re-deriving it from mutable state.
pub(crate) enum AllowlistDecision {
    Allowed,
    Denied,
    NewlyPending,
    StillPending,
}

/// The resolved approval policy (docs/design/22-gateway.md G2), held as one
/// value so the three fields can never be observed mid-update.
///
/// This is behind a lock rather than being plain fields because the policy
/// is now settable at runtime: it decides whether an `Ask` on a chat
/// surface reaches a human at all, and an operator who changes it must see
/// the next inbound message honour the change without restarting the
/// process. `forward` without a target is not representable — the
/// constructor and the setter both collapse that case to `deny`, which is
/// the same rule `vak_config`'s loader applies.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ApprovalPolicy {
    pub(crate) approvals: String,
    pub(crate) approver: Option<String>,
    pub(crate) timeout: Duration,
}

impl ApprovalPolicy {
    /// Build a policy, collapsing an unbacked `forward` to `deny`.
    /// A target must carry a `<surface>:<chat>` separator to count.
    pub(crate) fn resolve(
        approvals: &str,
        approver: Option<&str>,
        timeout: Duration,
    ) -> ApprovalPolicy {
        let target = approver
            .map(str::trim)
            .filter(|t| !t.is_empty() && t.contains(':'));
        let forward = approvals == "forward" && target.is_some();
        ApprovalPolicy {
            approvals: if forward { "forward" } else { "deny" }.into(),
            approver: forward.then(|| target.unwrap_or_default().to_string()),
            timeout,
        }
    }
}

pub struct GatewayState {
    pub enabled: bool,
    bindings: Mutex<HashMap<String, ChannelBinding>>,
    /// Resolved approval policy (docs/design/22-gateway.md G2). Mutable at
    /// runtime through [`GatewayState::set_approval_policy`].
    approval_policy: Mutex<ApprovalPolicy>,
    /// Forwarded gates awaiting a yes/no from the approver surface,
    /// oldest first (uuidv7 keys sort by insertion time).
    pending_approvals: Mutex<std::collections::BTreeMap<String, PendingGate>>,
    pending_questions: Mutex<std::collections::BTreeMap<String, ForwardedQuestion>>,
    chat_allowlist_open: bool,
    /// Live, schema-versioned allowlist store (docs/design/34). Authoritative
    /// once it exists on disk; seeded once from `chat_allowlist` otherwise.
    allowlist: Mutex<HashMap<String, AllowlistEntry>>,
    /// Bot identities (docs/design/34, multi-bot). Keyed by `Bot::id`.
    /// Independent of `allowlist`/`bindings` on purpose: several chats can
    /// share a bot, and a bot can exist with no chats bound to it yet.
    bots: Mutex<HashMap<String, Bot>>,
    /// Multi-tenant Core pool (docs/design/34 Phase 2). The gateway's own
    /// default workspace is the pool's permanent entry; every other
    /// workspace an allowlist entry names is lazily started here.
    pub(crate) core_pool: crate::core_pool::CorePool,
}

#[derive(Debug, Clone, Default, serde::Serialize, serde::Deserialize)]
pub(crate) struct ChannelBinding {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub session_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub provider: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub workspace: Option<PathBuf>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub route_revision: Option<String>,
}

#[derive(serde::Serialize, serde::Deserialize)]
struct BindingsFile {
    version: u32,
    bindings: HashMap<String, ChannelBinding>,
}

#[derive(serde::Deserialize)]
#[serde(untagged)]
enum StoredBindings {
    Versioned(BindingsFile),
    Legacy(HashMap<String, String>),
}

struct PendingGate {
    session_id: String,
    tx: oneshot::Sender<bool>,
}

/// A worker's question that was carried to the approver chat, kept until it is
/// answered, expires, or its session is revoked. The board is the source of
/// truth; this only lets a chat reply find it.
struct ForwardedQuestion {
    parent_session_id: String,
    label: String,
    board: Arc<vak_agent::WorkerRegistry>,
}

/// What an `answer …` reply did.
pub(crate) enum QuestionReply {
    Answered {
        code: String,
        label: String,
        session_id: String,
    },
    /// Nothing is waiting.
    NoneOpen,
    /// More than one is waiting and the reply did not name one.
    Ambiguous(Vec<String>),
    /// The named question is no longer open (answered elsewhere, expired).
    Closed,
    Refused(String),
}

/// The short code a chat uses to name a forwarded question: the tail of its
/// id, which is the random part (the head of a v7 id is a timestamp that
/// repeats across questions asked a minute apart).
pub(crate) fn question_code(id: &str) -> String {
    let start = id.len().saturating_sub(6);
    id.get(start..).unwrap_or(id).to_lowercase()
}

/// What a resolved gate was, so a bare yes/no is never silent about which
/// session's tool run it just decided.
pub(crate) struct ResolvedGate {
    pub id: String,
    pub session_id: String,
    pub remaining: usize,
}

impl GatewayState {
    /// Load persisted bindings; `force` overrides the config gate
    /// (`serve --gateway`).
    pub fn load(core: &Core, force: bool) -> Self {
        let mut bindings = HashMap::new();
        if let Ok(raw) = std::fs::read_to_string(bindings_path(&core.shared_scope().into_root()))
            && let Ok(stored) = serde_json::from_str::<StoredBindings>(&raw)
        {
            bindings = match stored {
                StoredBindings::Versioned(file) => file.bindings,
                StoredBindings::Legacy(map) => map
                    .into_iter()
                    .map(|(key, session_id)| {
                        (
                            key,
                            ChannelBinding {
                                session_id: Some(session_id),
                                ..ChannelBinding::default()
                            },
                        )
                    })
                    .collect(),
            };
        }
        let gw = &core.config().gateway;

        // Allowlist store: authoritative once allowlist.json exists; a
        // one-time import from config.toml's `chat_allowlist` seeds it the
        // first time a process ever loads (same relationship bindings.json
        // already has to route overrides — config.toml itself is untouched).
        let path = allowlist_path(&core.shared_scope().into_root());
        let allowlist: HashMap<String, AllowlistEntry> = match std::fs::read_to_string(&path) {
            Ok(raw) => serde_json::from_str::<AllowlistFile>(&raw)
                .map(|file| {
                    file.entries
                        .into_iter()
                        .map(|e| (e.key.clone(), e))
                        .collect()
                })
                .unwrap_or_default(),
            Err(_) => {
                let now = chrono::Utc::now().to_rfc3339();
                let seeded: HashMap<String, AllowlistEntry> = gw
                    .chat_allowlist
                    .iter()
                    .map(|key| {
                        (
                            key.clone(),
                            AllowlistEntry {
                                key: key.clone(),
                                status: AllowlistStatus::Allowed,
                                workspace: None,
                                agent_id: Some("vak".into()),
                                route: None,
                                voice: None,
                                permission_mode: None,
                                policy: vak_config::ChannelPolicy::default(),
                                added_at: now.clone(),
                                added_by: "config_import".into(),
                                first_seen_text: None,
                                prompt: Default::default(),
                                bot_id: None,
                                inherit_bot_policy: true,
                            },
                        )
                    })
                    .collect();
                if !seeded.is_empty() {
                    write_allowlist_file(&path, &seeded);
                }
                seeded
            }
        };

        // Bot store. There is no migration from a per-surface token slot:
        // those are deleted (AGENTS.md invariants 23 and 29), and
        // synthesizing a bot from one would be exactly the pre-baseline
        // fold-forward the baseline forbids. A bot is created explicitly,
        // through setup or the admin console, and owns its own token env.
        let bots_file_path = bots_path(&core.shared_scope().into_root());
        let bots: HashMap<String, Bot> = match std::fs::read_to_string(&bots_file_path) {
            Ok(raw) => serde_json::from_str::<BotsFile>(&raw)
                .map(|file| file.bots.into_iter().map(|b| (b.id.clone(), b)).collect())
                .unwrap_or_default(),
            // No bots.json yet means no bots. Not an error.
            Err(_) => HashMap::new(),
        };

        let state = GatewayState {
            enabled: force || gw.enabled,
            bindings: Mutex::new(bindings),
            bots: Mutex::new(bots),
            approval_policy: Mutex::new(ApprovalPolicy::resolve(
                &gw.approvals,
                gw.approver.as_deref(),
                Duration::from_secs(gw.approval_timeout_secs),
            )),
            pending_approvals: Mutex::new(std::collections::BTreeMap::new()),
            pending_questions: Mutex::new(std::collections::BTreeMap::new()),
            chat_allowlist_open: gw.chat_allowlist_open,
            allowlist: Mutex::new(allowlist),
            core_pool: crate::core_pool::CorePool::new(
                core.clone(),
                gw.core_pool_max,
                Duration::from_secs(gw.core_pool_idle_secs),
            ),
        };
        // docs/design/34: a pending request nobody acted on inside the
        // expiry window auto-denies (visibly, `added_by = "expiry"`).
        // `vak doctor --repair` does the same thing offline against the
        // store; doing it here too means a restarted gateway self-heals
        // and the two paths converge on the same state.
        let expired = state
            .allowlist_expire_pending(core, chrono::Duration::days(gw.pending_expiry_days as i64));
        for key in expired {
            vak_core::security_events::record(
                &core.scope(),
                vak_core::security_events::EventKind::ChatDenied,
                "chat_denied",
                &format!("key={key} reason=expiry"),
                None,
            );
        }
        state
    }

    /// Resolve the `Core` a channel's entry should actually run through:
    /// the pool's default entry when the entry has no workspace override or
    /// names the gateway's own workspace, otherwise the (lazily started)
    /// pooled `Core` for that workspace. This is the Phase 2 seam that
    /// makes an allowlist entry's `workspace` field actually run that
    /// workspace's own sandbox/permission/session state, not just pick its
    /// provider/model.
    pub(crate) fn core_for_entry(&self, default_core: &Core, key: &str) -> Result<Core, String> {
        let entry = self.allowlist_get(key);
        let allowed_entry = entry
            .as_ref()
            .filter(|e| e.status == AllowlistStatus::Allowed);
        // Bot tier: only consulted when the chat both names a bot and has
        // not opted out of inheriting from it (`inherit_bot_policy`). A
        // dangling `bot_id` (removed bot) resolves as "no bot tier", same
        // as an unset one — never a hard failure at dispatch.
        let bot = allowed_entry
            .filter(|e| e.inherit_bot_policy)
            .and_then(|e| e.bot_id.as_deref())
            .and_then(|id| self.bot_get(id));
        let configured_workspace = allowed_entry
            .and_then(|e| e.workspace.clone())
            .or_else(|| bot.as_ref().and_then(|b| b.workspace.clone()))
            .unwrap_or_else(|| default_core.cwd().clone());

        // Policy: bot policy (lower tier) folded under the chat's own
        // (higher tier) via the same restrictive-only merge used to
        // reconcile any two policy layers.
        let chat_policy = allowed_entry.map(|e| e.policy.clone()).unwrap_or_default();
        let policy = match &bot {
            Some(b) => vak_config::ChannelPolicy::merge(&b.policy, &chat_policy),
            None => chat_policy,
        };

        // Permission mode: chat pin capped by bot mode (itself already
        // capped by the workspace inside `resolve_at_with_policy`) so a bot
        // can narrow but never widen what the workspace allows, and a chat
        // can narrow but never widen what its bot allows.
        let permission_override = match (allowed_entry.and_then(|e| e.permission_mode), &bot) {
            (Some(chat_mode), Some(b)) => Some(match b.permission_mode {
                Some(bot_mode) => chat_mode.capped_by(bot_mode),
                None => chat_mode,
            }),
            (Some(chat_mode), None) => Some(chat_mode),
            (None, Some(b)) => b.permission_mode,
            (None, None) => None,
        };

        // A channel's configured workspace is the Agent's base space. The
        // Agent itself owns the isolated execution workspace beneath it,
        // exactly as it does for desktop/web admission. Keep channel policy
        // and permission overlays on the pooled Core, but resolve the
        // endpoint's effective Agent before asking for that Core.
        let selected_agent = allowed_entry
            .and_then(|entry| {
                let entry_agent = entry.agent_id.as_deref();
                let bot_agent = bot.as_ref().and_then(|b| b.agent_id.as_deref());
                if entry.inherit_bot_policy
                    && (entry_agent.is_none() || entry_agent == Some("vak"))
                    && bot_agent.is_some()
                {
                    return bot_agent;
                }
                entry_agent
            })
            .or_else(|| bot.as_ref().and_then(|b| b.agent_id.as_deref()))
            .unwrap_or("vak");
        let default_root = vak_config::paths::default_workspace();
        let configured_catalog = if selected_agent != "vak" {
            Some(self.core_pool.resolve_at_with_policy(
                &configured_workspace,
                None,
                vak_config::ChannelPolicy::default(),
                std::time::Instant::now(),
            )?)
        } else {
            None
        };
        let workspace_scoped = selected_agent != "vak"
            && configured_workspace != default_root
            && configured_catalog
                .as_ref()
                .is_some_and(Core::project_config_trusted)
            && crate::agents::load(&configured_workspace)
                .unwrap_or_default()
                .iter()
                .any(|profile| profile.id == selected_agent);
        // Match local Agent admission: a workspace profile is effective only
        // in a trusted workspace; otherwise the shared/default Agent owns the
        // conversation and its execution workspace.
        let agent_base = if selected_agent != "vak" && !workspace_scoped {
            default_root
        } else {
            configured_workspace.clone()
        };
        let effective_catalog = if selected_agent == "vak" {
            None
        } else if workspace_scoped {
            configured_catalog.clone()
        } else {
            Some(self.core_pool.resolve_at_with_policy(
                &agent_base,
                None,
                vak_config::ChannelPolicy::default(),
                std::time::Instant::now(),
            )?)
        };
        let identity = if selected_agent == "vak" {
            vak_core::vak_agent_identity()
        } else {
            let profiles = crate::agents::effective(
                effective_catalog
                    .as_ref()
                    .ok_or_else(|| "effective Agent catalog was not resolved".to_string())?,
            )
            .map_err(|error| format!("agent catalog unavailable: {error}"))?;
            profiles
                .into_iter()
                .find(|profile| profile.id == selected_agent)
                .filter(|profile| profile.is_admissible())
                .map(|profile| profile.identity())
                .ok_or_else(|| format!("configured Agent '{selected_agent}' is unavailable"))?
        };
        let workspace = if selected_agent == "vak" {
            agent_base.clone()
        } else {
            vak_config::paths::agent_workspace(&agent_base, selected_agent)
        };
        if !workspace.is_dir() {
            std::fs::create_dir_all(&workspace).map_err(|error| {
                format!(
                    "could not create Agent workspace {}: {error}",
                    workspace.display()
                )
            })?;
        }
        if identity.id != "vak"
            && effective_catalog
                .as_ref()
                .is_some_and(Core::project_config_trusted)
            && !vak_core::trust::is_trusted(&workspace)
        {
            let _ = vak_core::trust::mark_trusted(&workspace);
        }

        let resolved = self.core_pool.resolve_at_with_policy(
            &workspace,
            permission_override,
            policy,
            std::time::Instant::now(),
        )?;
        resolved.set_sessions_home(default_core.shared_scope().into_root());
        if let Some(provider) = default_core.provider_instance_override() {
            resolved.set_provider_instance(provider);
        }
        if let Some(mode) = default_core.permission_mode_override_value() {
            resolved.set_permission_mode(mode.capped_by(resolved.effective_permission_mode()));
        }
        if let Some(backend) = default_core.sandbox_backend_override_value() {
            resolved.set_sandbox_backend(Some(backend));
        }
        Ok(resolved.with_agent_identity(Some(identity)))
    }

    pub(crate) fn workspace_for_entry(&self, default_core: &Core, key: &str) -> PathBuf {
        let entry = self.allowlist_get(key);
        let allowed = entry
            .as_ref()
            .filter(|e| e.status == AllowlistStatus::Allowed);
        let bot = allowed
            .filter(|e| e.inherit_bot_policy)
            .and_then(|e| e.bot_id.as_deref())
            .and_then(|id| self.bot_get(id));
        allowed
            .and_then(|e| e.workspace.clone())
            .or_else(|| bot.and_then(|b| b.workspace))
            .unwrap_or_else(|| default_core.cwd().to_path_buf())
    }

    pub(crate) fn workspace_override_for_entry(&self, key: &str) -> Option<PathBuf> {
        let entry = self.allowlist_get(key)?;
        if entry.status != AllowlistStatus::Allowed {
            return None;
        }
        let bot = entry
            .inherit_bot_policy
            .then_some(entry.bot_id.as_deref())
            .flatten()
            .and_then(|id| self.bot_get(id));
        entry.workspace.or_else(|| bot.and_then(|b| b.workspace))
    }

    pub(crate) fn set_enabled(&mut self, enabled: bool) {
        self.enabled = enabled;
    }

    /// Snapshot of current surface bindings (key→value).
    pub(crate) fn bindings_snapshot(&self) -> Vec<(String, ChannelBinding)> {
        self.bindings
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .iter()
            .map(|(k, v)| (k.clone(), v.clone()))
            .collect()
    }

    /// True when forwarded gates are active.
    fn approval_policy(&self) -> ApprovalPolicy {
        self.approval_policy
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .clone()
    }

    pub(crate) fn forward_mode(&self) -> bool {
        let policy = self.approval_policy();
        self.enabled && policy.approvals == "forward" && policy.approver.is_some()
    }

    /// The chat that answers forwarded gates. Returns an owned `String`
    /// rather than a borrow because the policy now lives behind a lock —
    /// handing out a reference into it would either hold the lock across
    /// an await or dangle.
    pub(crate) fn approver_target(&self) -> Option<String> {
        self.approval_policy().approver
    }

    pub(crate) fn approval_timeout(&self) -> Duration {
        self.approval_policy().timeout
    }

    pub(crate) fn approvals_mode(&self) -> String {
        self.approval_policy().approvals
    }

    /// Chats that could serve as the forwarded-approval target, as
    /// `<surface>:<chat>` delivery addresses.
    ///
    /// An allowlist key may be bot-scoped (`telegram:12345:vakyartha`);
    /// that third segment identifies the bot the message arrived through,
    /// not a place a reply can be delivered. `deliver_to` addresses are
    /// two-part, so the key is truncated here rather than at every reader.
    /// Only `Allowed` entries are offered: forwarding a gate to a pending
    /// or denied chat would announce it somewhere the operator has
    /// explicitly not admitted.
    pub(crate) fn approver_candidates(&self) -> Vec<String> {
        let mut out: Vec<String> = self
            .allowlist
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .values()
            .filter(|entry| entry.status == AllowlistStatus::Allowed)
            .filter_map(|entry| {
                let mut parts = entry.key.splitn(3, ':');
                match (parts.next(), parts.next()) {
                    (Some(surface), Some(chat)) if !surface.is_empty() && !chat.is_empty() => {
                        Some(format!("{surface}:{chat}"))
                    }
                    _ => None,
                }
            })
            .collect();
        out.sort();
        out.dedup();
        out
    }

    /// Replace the live approval policy. Returns the policy actually
    /// installed, which is [`ApprovalPolicy::resolve`]'s answer — asking
    /// for `forward` with no usable target installs `deny`, so a caller
    /// can compare and tell the operator their request was reduced instead
    /// of reporting a success that did not happen.
    ///
    /// In-flight forwarded gates are NOT resolved here. They were raised
    /// under the old policy and already have an announcement sitting in the
    /// approver's chat; cancelling them would strand a run that a human is
    /// actively about to answer. Narrowing to `deny` stops the NEXT gate,
    /// which is the guarantee that matters (nothing new reaches a chat that
    /// should no longer be asked).
    pub(crate) fn set_approval_policy(&self, next: ApprovalPolicy) -> ApprovalPolicy {
        let resolved =
            ApprovalPolicy::resolve(&next.approvals, next.approver.as_deref(), next.timeout);
        *self
            .approval_policy
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner) = resolved.clone();
        resolved
    }

    /// True when an empty `chat_allowlist` was explicitly opted into
    /// staying open. Defaults to false: fail closed (0c-02).
    pub(crate) fn chat_allowlist_open(&self) -> bool {
        self.chat_allowlist_open
    }

    /// The codes of the questions waiting on the approver chat.
    fn forwarded_question_codes(&self) -> Vec<String> {
        let mut map = self
            .pending_questions
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        Self::prune_questions(&mut map);
        map.keys().map(|id| question_code(id)).collect()
    }

    pub(crate) fn pending_approval_count(&self) -> usize {
        self.pending_approvals
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .len()
    }

    /// Register a gate for `session_id` and hand back the reply receiver.
    /// The sender must be stored before the request is announced so an
    /// instant reply cannot race a missing entry.
    pub(crate) fn register_gate(&self, id: &str, session_id: &str) -> oneshot::Receiver<bool> {
        let (tx, rx) = oneshot::channel();
        self.pending_approvals
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .insert(
                id.to_string(),
                PendingGate {
                    session_id: session_id.to_string(),
                    tx,
                },
            );
        rx
    }

    /// Resolve a gate. With an id prefix, only that exact gate resolves —
    /// a reply meant for one session can never approve another's tool run.
    /// Without one, the globally oldest gate resolves and is reported so
    /// the approver surface can see what their bare yes/no did.
    pub(crate) fn resolve_gate(
        &self,
        approve: bool,
        id_prefix: Option<&str>,
    ) -> Result<ResolvedGate, ()> {
        let mut map = self
            .pending_approvals
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let key = match id_prefix {
            // A code must name exactly one gate: an ambiguous or empty one
            // resolves nothing, because silence and doubt mean no.
            Some(prefix) if !prefix.is_empty() => {
                let mut matching = map.keys().filter(|k| k.ends_with(prefix));
                match (matching.next().cloned(), matching.next()) {
                    (Some(k), None) => k,
                    _ => return Err(()),
                }
            }
            Some(_) => return Err(()),
            None => map.keys().next().cloned().ok_or(())?,
        };
        let (_, gate) = map.remove_entry(&key).ok_or(())?;
        let _ = gate.tx.send(approve);
        Ok(ResolvedGate {
            id: key,
            session_id: gate.session_id,
            remaining: map.len(),
        })
    }

    /// Remember a question carried to the approver chat, so a reply can find
    /// it. Stored before the question is announced, so an instant reply
    /// cannot race a missing entry.
    fn register_question(
        &self,
        id: &str,
        parent_session_id: &str,
        label: &str,
        board: Arc<vak_agent::WorkerRegistry>,
    ) {
        self.pending_questions
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .insert(
                id.to_string(),
                ForwardedQuestion {
                    parent_session_id: parent_session_id.to_string(),
                    label: label.to_string(),
                    board,
                },
            );
    }

    /// Drop forwarded questions the board no longer has open (expired,
    /// cancelled, answered elsewhere), so a stale one is never counted or
    /// offered as "the only question waiting".
    fn prune_questions(map: &mut std::collections::BTreeMap<String, ForwardedQuestion>) {
        map.retain(|id, forwarded| forwarded.board.questions().is_open(id));
    }

    fn forget_question(&self, id: &str) {
        self.pending_questions
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .remove(id);
    }

    /// Answer a forwarded question from the approver chat. With a code, only
    /// that question; without one, the only one waiting, never "the oldest"
    /// of several, because a worker acts on what it is told and a reply meant
    /// for one must not land on another.
    pub(crate) fn answer_forwarded(&self, code: Option<&str>, text: &str) -> QuestionReply {
        let mut map = self
            .pending_questions
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        Self::prune_questions(&mut map);
        let id = match code {
            Some(code) => {
                let code = code.to_lowercase();
                match map.keys().find(|id| question_code(id) == code).cloned() {
                    Some(id) => id,
                    None => return QuestionReply::Closed,
                }
            }
            None => match map.len() {
                0 => return QuestionReply::NoneOpen,
                1 => map.keys().next().cloned().unwrap_or_default(),
                _ => {
                    return QuestionReply::Ambiguous(
                        map.keys().map(|id| question_code(id)).collect(),
                    );
                }
            },
        };
        let Some(forwarded) = map.get(&id) else {
            return QuestionReply::Closed;
        };
        match forwarded.board.questions().answer(
            &forwarded.parent_session_id,
            &id,
            text,
            "the approver chat",
        ) {
            Ok(_) => {
                let forwarded = map.remove(&id);
                match forwarded {
                    Some(f) => QuestionReply::Answered {
                        code: question_code(&id),
                        label: f.label,
                        session_id: f.parent_session_id,
                    },
                    None => QuestionReply::Closed,
                }
            }
            Err(vak_agent::questions::AnswerError::Unknown) => {
                map.remove(&id);
                QuestionReply::Closed
            }
            Err(error) => QuestionReply::Refused(error.to_string()),
        }
    }

    /// Reject forwarded approval gates belonging to a revoked session. A
    /// late reply then finds no gate and cannot authorize stale work.
    pub(crate) fn deny_pending_for_session(&self, session_id: &str) -> usize {
        {
            let mut questions = self
                .pending_questions
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            let ids: Vec<String> = questions
                .iter()
                .filter(|(_, q)| q.parent_session_id == session_id)
                .map(|(id, _)| id.clone())
                .collect();
            for id in ids {
                if let Some(q) = questions.remove(&id) {
                    q.board.questions().deny_all(session_id);
                }
            }
        }
        let mut pending = self
            .pending_approvals
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let keys: Vec<String> = pending
            .iter()
            .filter(|(_, gate)| gate.session_id == session_id)
            .map(|(id, _)| id.clone())
            .collect();
        let mut denied = 0;
        for key in keys {
            if let Some(gate) = pending.remove(&key) {
                let _ = gate.tx.send(false);
                denied += 1;
            }
        }
        denied
    }

    pub(crate) fn snapshot(&self) -> Vec<(String, ChannelBinding)> {
        let mut pairs: Vec<(String, ChannelBinding)> = self
            .bindings
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .iter()
            .map(|(k, v)| (k.clone(), v.clone()))
            .collect();
        pairs.sort_by(|a, b| a.0.cmp(&b.0));
        pairs
    }

    fn bind(&self, core: &Core, key: String, session_id: String, revision: String) {
        let mut bindings = self
            .bindings
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let binding = bindings.entry(key).or_default();
        binding.session_id = Some(session_id);
        binding.workspace = Some(core.cwd().clone());
        binding.route_revision = Some(revision);
        drop(bindings);
        persist_bindings(core, self);
    }

    pub(crate) fn set_route_override(
        &self,
        core: &Core,
        key: String,
        route: Option<(String, String)>,
    ) {
        let mut bindings = self
            .bindings
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let binding = bindings.entry(key).or_default();
        match route {
            Some((provider, model)) => {
                binding.provider = Some(provider);
                binding.model = Some(model);
            }
            None => {
                binding.provider = None;
                binding.model = None;
            }
        }
        binding.route_revision = None;
        drop(bindings);
        persist_bindings(core, self);
    }

    pub(crate) fn rotate(&self, core: &Core, key: &str) -> bool {
        let mut bindings = self
            .bindings
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let Some(binding) = bindings.get_mut(key) else {
            return false;
        };
        binding.session_id = None;
        binding.route_revision = None;
        drop(bindings);
        persist_bindings(core, self);
        true
    }

    pub(crate) fn unbind(&self, core: &Core, key: &str) -> bool {
        let removed = self
            .bindings
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .remove(key)
            .is_some();
        if removed {
            persist_bindings(core, self);
        }
        removed
    }

    // ---- Bot store (multi-bot-per-channel) ---------------------------------

    /// Snapshot of all bots, sorted by id. Secrets never included — a `Bot`
    /// row only ever holds the env var *name*, not the token value.
    pub(crate) fn bots_snapshot(&self) -> Vec<Bot> {
        let mut bots: Vec<Bot> = self
            .bots
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .values()
            .cloned()
            .collect();
        bots.sort_by(|a, b| a.id.cmp(&b.id));
        bots
    }

    pub(crate) fn bot_get(&self, id: &str) -> Option<Bot> {
        self.bots
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .get(id)
            .cloned()
    }

    /// Insert or replace a bot row wholesale (create, rename label, or edit
    /// policy/permission_mode/route/workspace). `token_env` is set
    /// separately from the actual secret by the caller before this is
    /// invoked, keeping the write here free of the token value itself.
    pub(crate) fn bot_upsert(&self, core: &Core, bot: Bot) {
        let id = bot.id.clone();
        self.bots
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .insert(id.clone(), bot);
        persist_bots(core, self);
        self.invalidate_bindings_for_bot(core, &id);
    }

    /// Remove a bot row. Chats whose `bot_id` names it keep the id on
    /// record (a dangling reference resolves as "no bot" at dispatch,
    /// same as an unset `bot_id`) rather than being silently rewritten.
    pub(crate) fn bot_remove(&self, core: &Core, id: &str) -> bool {
        let removed = self
            .bots
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .remove(id)
            .is_some();
        if removed {
            persist_bots(core, self);
            self.invalidate_bindings_for_bot(core, id);
        }
        removed
    }

    // ---- Allowlist store (docs/design/34-channel-onboarding.md) -----------

    /// Snapshot of all allowlist entries, any status, sorted by key.
    pub(crate) fn allowlist_snapshot(&self) -> Vec<AllowlistEntry> {
        let mut entries: Vec<AllowlistEntry> = self
            .allowlist
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .values()
            .cloned()
            .collect();
        entries.sort_by(|a, b| a.key.cmp(&b.key));
        entries
    }

    pub(crate) fn allowlist_get(&self, key: &str) -> Option<AllowlistEntry> {
        self.allowlist
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .get(key)
            .cloned()
    }

    /// Resolve an inbound key against the store: creates a pending entry on
    /// first sight, never duplicates or bumps `added_at` on a repeat
    /// message from an already-pending key.
    pub(crate) fn allowlist_resolve_inbound(
        &self,
        core: &Core,
        key: &str,
        first_seen_text: &str,
        bot_id: Option<&str>,
    ) -> AllowlistDecision {
        let mut map = self
            .allowlist
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        // Distinct from `decision` below: repeat traffic from an already-
        // `Allowed` key takes the same `AllowlistDecision::Allowed` value
        // as the freshly-inherited case, but must not re-persist the file
        // on every single inbound message — only a real map mutation
        // (a fresh Pending row, or a fresh inherited-Allowed row) should.
        let mut mutated = false;
        let decision = match map.get(key).map(|e| e.status) {
            Some(AllowlistStatus::Allowed) => AllowlistDecision::Allowed,
            Some(AllowlistStatus::Denied) => AllowlistDecision::Denied,
            Some(AllowlistStatus::Pending) => AllowlistDecision::StillPending,
            None => {
                // Multi-bot-per-channel (docs/design/34 Phase 5 follow-up):
                // a bot-scoped key (`surface:chat:bot_id`) seen for the
                // first time inherits an already-allowed legacy
                // (`surface:chat`) entry's approval/workspace/policy when
                // one exists — the same physical chat an operator already
                // trusted, just now seen through a second bot. Without
                // this, every existing approved chat would need a needless
                // re-approval the moment its bridge started sending a
                // bot id, and a `chat_allowlist` entry in config.toml
                // (also just a row in this same map) would silently stop
                // matching too.
                if let Some(legacy_key) = legacy_key_for(key)
                    && let Some(legacy) = map.get(&legacy_key).cloned()
                    && legacy.status == AllowlistStatus::Allowed
                {
                    map.insert(
                        key.to_string(),
                        AllowlistEntry {
                            key: key.to_string(),
                            bot_id: bot_id.map(str::to_string),
                            added_by: format!("gateway (inherited from {legacy_key})"),
                            added_at: chrono::Utc::now().to_rfc3339(),
                            first_seen_text: None,
                            prompt: Default::default(),
                            ..legacy
                        },
                    );
                    // The legacy row's *allowlist entry* stays — a third
                    // bot arriving later needs it as the ancestor to
                    // inherit from too, same as this one just did. But its
                    // *binding/session* is now genuinely dead: every future
                    // message for this physical chat will always carry a
                    // bot id and therefore always route through a
                    // bot-scoped key, never this one again. Left bound, it
                    // would sit forever in the Chats list looking like a
                    // confusing duplicate of the bot-scoped row — the exact
                    // bug a live operator hit. `unbind` locks
                    // `self.bindings`, a different mutex than the `map`
                    // guard held here, so no deadlock.
                    self.unbind(core, &legacy_key);
                    mutated = true;
                    AllowlistDecision::Allowed
                } else {
                    let truncated: String = first_seen_text
                        .chars()
                        .take(FIRST_SEEN_TEXT_MAX_CHARS)
                        .collect();
                    let bot_agent = bot_id
                        .and_then(|id| self.bot_get(id))
                        .and_then(|b| b.agent_id);
                    map.insert(
                        key.to_string(),
                        AllowlistEntry {
                            key: key.to_string(),
                            status: AllowlistStatus::Pending,
                            workspace: None,
                            agent_id: bot_agent.or_else(|| Some("vak".into())),
                            route: None,
                            voice: None,
                            permission_mode: None,
                            policy: vak_config::ChannelPolicy::default(),
                            added_at: chrono::Utc::now().to_rfc3339(),
                            added_by: "gateway".into(),
                            first_seen_text: Some(truncated),
                            prompt: Default::default(),
                            bot_id: bot_id.map(str::to_string),
                            inherit_bot_policy: true,
                        },
                    );
                    mutated = true;
                    AllowlistDecision::NewlyPending
                }
            }
        };
        if mutated {
            drop(map);
            persist_allowlist(core, self);
        }
        decision
    }

    /// Approve a key: pending or unknown → allowed, with an explicit
    /// workspace (never silently inherited) and optional route override.
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn allowlist_approve(
        &self,
        core: &Core,
        key: &str,
        workspace: PathBuf,
        agent_id: Option<String>,
        route: Option<AllowlistRoute>,
        permission_mode: Option<vak_config::PermissionMode>,
        policy: vak_config::ChannelPolicy,
        bot_id: Option<String>,
        inherit_bot_policy: bool,
        added_by: &str,
    ) -> AllowlistEntry {
        let entry = {
            let mut map = self
                .allowlist
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            let bot_agent = bot_id
                .as_deref()
                .and_then(|id| self.bot_get(id))
                .and_then(|b| b.agent_id);
            let entry = AllowlistEntry {
                key: key.to_string(),
                status: AllowlistStatus::Allowed,
                workspace: Some(workspace),
                agent_id: agent_id.or(bot_agent).or_else(|| Some("vak".into())),
                route,
                voice: None,
                permission_mode,
                policy,
                added_at: chrono::Utc::now().to_rfc3339(),
                added_by: added_by.to_string(),
                first_seen_text: None,
                prompt: Default::default(),
                bot_id,
                inherit_bot_policy,
            };
            map.insert(key.to_string(), entry.clone());
            entry
        };
        persist_allowlist(core, self);
        entry
    }

    /// Deny a key: pending or unknown → denied (sticky).
    pub(crate) fn allowlist_deny(&self, core: &Core, key: &str, added_by: &str) -> AllowlistEntry {
        let entry = {
            let mut map = self
                .allowlist
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            let entry = AllowlistEntry {
                key: key.to_string(),
                status: AllowlistStatus::Denied,
                workspace: None,
                agent_id: Some("vak".into()),
                route: None,
                voice: None,
                permission_mode: None,
                policy: vak_config::ChannelPolicy::default(),
                added_at: chrono::Utc::now().to_rfc3339(),
                added_by: added_by.to_string(),
                first_seen_text: None,
                prompt: Default::default(),
                bot_id: None,
                inherit_bot_policy: true,
            };
            map.insert(key.to_string(), entry.clone());
            entry
        };
        persist_allowlist(core, self);
        entry
    }

    /// Edit an already-`allowed` entry's `workspace`/`route` in place
    /// (docs/design/34 "Editing an already-allowed entry"). Provenance
    /// (`added_at`/`added_by`) is deliberately preserved — this is a
    /// re-point, not a re-approval. `None` for either field clears it
    /// (inherit the gateway workspace / the workspace's default route).
    ///
    /// The caller is expected to follow this with
    /// [`GatewayState::invalidate_binding_revision`] so the change goes
    /// through the same stale-session-rotation path
    /// `PATCH .../bindings/{key}` already uses, rather than mutating
    /// allowlist state the binding/session layer never learns about.
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn allowlist_patch(
        &self,
        core: &Core,
        key: &str,
        workspace: Option<PathBuf>,
        agent_id: Option<Option<String>>,
        route: Option<AllowlistRoute>,
        permission_mode: Option<vak_config::PermissionMode>,
        policy: vak_config::ChannelPolicy,
        bot_id: Option<Option<String>>,
        inherit_bot_policy: Option<bool>,
        voice: Option<Option<vak_config::VoiceConfig>>,
        prompt: Option<vak_core::prompts::LayerContent>,
    ) -> Option<AllowlistEntry> {
        let entry = {
            let mut map = self
                .allowlist
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            let entry = map.get_mut(key)?;
            if entry.status != AllowlistStatus::Allowed {
                return None;
            }
            entry.workspace = workspace;
            if let Some(agent_id) = agent_id {
                entry.agent_id = agent_id.and_then(|s| {
                    let trimmed = s.trim();
                    if trimmed.is_empty() {
                        None
                    } else {
                        Some(trimmed.to_string())
                    }
                });
            }
            entry.route = route;
            entry.permission_mode = permission_mode;
            entry.policy = policy;
            if let Some(bot_id) = bot_id {
                entry.bot_id = bot_id;
            }
            if let Some(inherit) = inherit_bot_policy {
                entry.inherit_bot_policy = inherit;
            }
            if let Some(voice) = voice {
                entry.voice = voice;
            }
            if let Some(prompt) = prompt {
                entry.prompt = prompt;
            }
            entry.clone()
        };
        persist_allowlist(core, self);
        Some(entry)
    }

    /// Write-through for the pre-existing `PATCH .../bindings/{key}`
    /// surface: when this key has an `allowed` entry, its route is the
    /// source of truth, so a route change made from the binding editor
    /// lands there too instead of drifting. Returns true when an entry was
    /// actually updated.
    pub(crate) fn allowlist_patch_route_if_allowed(
        &self,
        core: &Core,
        key: &str,
        route: Option<AllowlistRoute>,
    ) -> bool {
        let Some(entry) = self.allowlist_get(key) else {
            return false;
        };
        if entry.status != AllowlistStatus::Allowed {
            return false;
        }
        // Preserve the permission override: the binding editor only ever
        // speaks about routes, so it must not silently clear a channel's
        // pinned permission mode as a side effect.
        self.allowlist_patch(
            core,
            key,
            entry.workspace,
            Some(entry.agent_id),
            route,
            entry.permission_mode,
            entry.policy,
            Some(entry.bot_id),
            Some(entry.inherit_bot_policy),
            Some(entry.voice),
            // A route write must not disturb this chat's prompt tier, for
            // the same reason the comment above gives about permission mode.
            None,
        )
        .is_some()
    }

    /// Auto-deny every `pending` entry older than `max_age`, stamping
    /// `added_by = "expiry"` so an expired request stays visibly distinct
    /// from an operator's own deny (docs/design/34 open question 1).
    /// Returns the keys denied.
    pub(crate) fn allowlist_expire_pending(
        &self,
        core: &Core,
        max_age: chrono::Duration,
    ) -> Vec<String> {
        let cutoff = chrono::Utc::now() - max_age;
        let expired: Vec<String> = self
            .allowlist
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .values()
            .filter(|e| {
                e.status == AllowlistStatus::Pending && parse_added_at_before(&e.added_at, cutoff)
            })
            .map(|e| e.key.clone())
            .collect();
        for key in &expired {
            self.allowlist_deny(core, key, "expiry");
        }
        expired
    }

    /// The single answer to "what does this channel route to": the
    /// allowlist entry's pinned route when it has one (the Phase 1 admin
    /// surface), otherwise the binding's own provider/model override (the
    /// pre-existing `PATCH .../bindings/{key}` surface). One source of
    /// truth read at dispatch, so the two admin surfaces cannot drift.
    fn effective_route_override(&self, key: &str) -> Option<(String, String)> {
        let entry = self
            .allowlist_get(key)
            .filter(|e| e.status == AllowlistStatus::Allowed);
        if let Some(route) = entry.as_ref().and_then(|e| e.route.clone()) {
            return Some((route.provider, route.model));
        }
        // Bot tier: the chat named no route of its own, so fall through to
        // its bot's route (if any, and if inheritance wasn't broken) before
        // the legacy binding override / workspace default.
        if let Some(route) = entry
            .as_ref()
            .filter(|e| e.inherit_bot_policy)
            .and_then(|e| e.bot_id.as_deref())
            .and_then(|id| self.bot_get(id))
            .and_then(|b| b.route)
        {
            return Some((route.provider, route.model));
        }
        self.bindings
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .get(key)
            .and_then(|binding| Some((binding.provider.clone()?, binding.model.clone()?)))
    }

    /// The single answer to "what voice/persona does this channel speak
    /// with": the chat's own override when it has one, otherwise its
    /// bot's (unless inheritance was broken), otherwise `None` (no voice
    /// configured — caller falls back to a built-in default). Mirrors
    /// `effective_route_override` exactly.
    /// Bot then chat prompt tiers, broadest first, for `Core::with_prompt_overlays`.
    ///
    /// Deliberately *not* shaped like `resolve_voice`, which picks one
    /// winner: guardrails from both tiers must survive, so both are handed
    /// to the resolver and it applies the narrowest-wins rule to identity
    /// and rules while concatenating guardrails. `inherit_bot_policy` gates
    /// the bot tier exactly as it gates policy, route, and voice.
    pub(crate) fn resolve_prompt_overlays(&self, key: &str) -> Vec<vak_core::prompts::LayerInput> {
        let Some(entry) = self
            .allowlist_get(key)
            .filter(|e| e.status == AllowlistStatus::Allowed)
        else {
            return Vec::new();
        };
        let mut out = Vec::new();
        if entry.inherit_bot_policy
            && let Some(bot) = entry.bot_id.as_deref().and_then(|id| self.bot_get(id))
            && !bot.prompt.is_empty()
        {
            out.push(vak_core::prompts::LayerInput::new(
                vak_core::prompts::PromptLayer::Bot,
                Some(format!("bot:{}", bot.id)),
                bot.prompt,
            ));
        }
        if !entry.prompt.is_empty() {
            out.push(vak_core::prompts::LayerInput::new(
                vak_core::prompts::PromptLayer::Chat,
                Some(format!("chat:{key}")),
                entry.prompt,
            ));
        }
        out
    }

    /// The style directive for spoken replies on this chat.
    ///
    /// One source of truth (docs/design/45-prompt-layers.md): the bot/chat
    /// `identity` block *is* the persona. `VoiceConfig.persona` predates
    /// prompt layers and said the same thing in a second place; keeping both
    /// authoritative would let a bot's spoken and written selves drift apart
    /// within a release. The legacy field is still honoured when no prompt
    /// tier sets an identity, so existing configs keep working untouched.
    ///
    /// Only the *gateway tiers'* own identity text is used, never the
    /// assembled prompt — the seed identity and capability contract are
    /// meaningless as a text-to-speech style directive.
    pub(crate) fn resolve_persona(&self, key: &str) -> Option<String> {
        self.resolve_prompt_overlays(key)
            .into_iter()
            .rev()
            .find_map(|layer| layer.content.identity)
            .map(|text| text.trim().to_string())
            .filter(|text| !text.is_empty())
            .or_else(|| {
                self.resolve_voice(key)
                    .and_then(|voice| voice.persona)
                    .map(|p| p.trim().to_string())
                    .filter(|p| !p.is_empty())
            })
    }

    pub(crate) fn resolve_voice(&self, key: &str) -> Option<vak_config::VoiceConfig> {
        let entry = self
            .allowlist_get(key)
            .filter(|e| e.status == AllowlistStatus::Allowed);
        let parent = entry
            .as_ref()
            .filter(|e| e.inherit_bot_policy)
            .and_then(|e| e.bot_id.as_deref())
            .and_then(|id| self.bot_get(id))
            .and_then(|b| b.voice);
        entry
            .as_ref()
            .and_then(|e| e.voice.as_ref())
            .map(|v| vak_config::VoiceConfig::overlay(parent.as_ref(), v))
            .or(parent)
    }

    /// Drop the cached route revision for `key` without dropping the
    /// session id — exactly what `set_route_override` does — so the next
    /// inbound message re-derives the effective route and rotates the
    /// frozen session if (and only if) it actually changed.
    pub(crate) fn invalidate_binding_revision(&self, core: &Core, key: &str) {
        let touched = {
            let mut bindings = self
                .bindings
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            match bindings.get_mut(key) {
                Some(binding) => {
                    binding.route_revision = None;
                    true
                }
                None => false,
            }
        };
        if touched {
            persist_bindings(core, self);
        }
    }

    pub(crate) fn invalidate_bindings_for_bot(&self, core: &Core, bot_id: &str) {
        let keys: Vec<String> = {
            let allowlist = self
                .allowlist
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            allowlist
                .values()
                .filter(|entry| entry.bot_id.as_deref() == Some(bot_id))
                .map(|entry| entry.key.clone())
                .collect()
        };
        let touched = {
            let mut bindings = self
                .bindings
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            let mut changed = false;
            for key in keys {
                if let Some(binding) = bindings.get_mut(&key) {
                    binding.route_revision = None;
                    changed = true;
                }
            }
            changed
        };
        if touched {
            persist_bindings(core, self);
        }
    }

    /// Test seam: plant a bound session with a frozen route revision, the
    /// state dispatch leaves behind, without running a whole turn.
    #[cfg(test)]
    pub(crate) fn bind_for_test(&self, key: &str, session_id: &str, revision: &str) {
        let mut bindings = self
            .bindings
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let binding = bindings.entry(key.to_string()).or_default();
        binding.session_id = Some(session_id.to_string());
        binding.route_revision = Some(revision.to_string());
    }

    #[cfg(test)]
    pub(crate) fn route_revision_for_test(&self, key: &str) -> Option<String> {
        self.bindings
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .get(key)
            .and_then(|b| b.route_revision.clone())
    }

    #[cfg(test)]
    pub(crate) fn session_id_for_test(&self, key: &str) -> Option<String> {
        self.bindings
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .get(key)
            .and_then(|b| b.session_id.clone())
    }

    /// Revoke an allowed entry: removes it from the store entirely (a
    /// future message from that key starts a fresh pending review, not a
    /// stale "denied" record masquerading as an audit trail).
    pub(crate) fn allowlist_revoke(&self, core: &Core, key: &str) -> bool {
        if self.allowlist_get(key).map(|e| e.status) != Some(AllowlistStatus::Allowed) {
            return false;
        }
        let removed = self
            .allowlist
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .remove(key)
            .is_some();
        if removed {
            persist_allowlist(core, self);
        }
        removed
    }
}

/// What a channel's permission mode actually resolves to, for display and
/// for the approve/patch audit trail.
pub(crate) struct ResolvedPermission {
    /// The target workspace's own configured mode — the outermost ceiling.
    pub workspace_mode: vak_config::PermissionMode,
    /// The bot tier's pin, when the chat inherits from a bot that has one.
    /// `None` means no bot tier applies, not "the bot allows everything".
    pub bot_mode: Option<vak_config::PermissionMode>,
    /// What the entry asked for, if anything.
    pub requested: Option<vak_config::PermissionMode>,
    /// What the channel actually gets, folded the same way `core_for_entry`
    /// folds it: chat pin capped by bot pin, then capped by the workspace.
    pub effective: vak_config::PermissionMode,
}

impl ResolvedPermission {
    /// True when a pin asked for more than the layers above it allow and
    /// was reduced. This is the condition worth an audit-log entry.
    ///
    /// It covers the bot tier too: a bot pinned narrower than its chat
    /// reduces that chat just as surely as the workspace does, and reporting
    /// only the workspace cap is what let the console show a chat as wider
    /// than it actually ran.
    pub fn was_capped(&self) -> bool {
        matches!(self.requested, Some(r) if r != self.effective)
            || matches!(
                (self.bot_mode, self.requested),
                (Some(bot), None) if bot != self.effective
            )
    }
}

/// Read-only mirror of the cap dispatch enforces, for the admin surface.
/// Shares `PermissionMode::capped_by` with the pool so the number the
/// console shows is derived the same way as the one dispatch pins.
///
/// It must fold the SAME three tiers `GatewayState::core_for_entry` folds:
/// chat pin capped by bot pin, then capped by the workspace. Leaving the
/// bot tier out — as this did — meant a bot pinned to `read-only` under a
/// chat pinned to `workspace-write` ran read-only and was reported as
/// workspace-write, and a chat with no pin under a bot that had one was
/// reported as the workspace's mode instead of the bot's. Showing a channel
/// as wider than it runs is the one direction of error that matters here.
///
/// A workspace whose config fails to load falls back to the compiled
/// default (`WorkspaceWrite`), matching `vak_config`'s own layering; the
/// pool remains the authority at dispatch either way.
pub(crate) fn resolve_channel_permission(
    workspace: &std::path::Path,
    requested: Option<vak_config::PermissionMode>,
    bot_mode: Option<vak_config::PermissionMode>,
) -> ResolvedPermission {
    // Same trust the pool will use at dispatch. Reading with `true` here
    // while the pool read the marker store would put the console back to
    // reporting a mode no run would get.
    let workspace_mode =
        vak_config::load_with_trust(workspace, vak_core::trust::is_trusted(workspace))
            .map(|c| c.permission_mode)
            .unwrap_or_default();
    // Exactly `core_for_entry`'s fold, then the pool's workspace cap.
    let pinned = match (requested, bot_mode) {
        (Some(chat), Some(bot)) => Some(chat.capped_by(bot)),
        (Some(chat), None) => Some(chat),
        (None, bot) => bot,
    };
    let effective = match pinned {
        Some(mode) => mode.capped_by(workspace_mode),
        None => workspace_mode,
    };
    ResolvedPermission {
        workspace_mode,
        bot_mode,
        requested,
        effective,
    }
}

/// True when `added_at` parses as an RFC3339 stamp strictly older than
/// `cutoff`. An unparseable stamp is never treated as expired: a corrupt
/// timestamp must not silently auto-deny a live channel.
fn parse_added_at_before(added_at: &str, cutoff: chrono::DateTime<chrono::Utc>) -> bool {
    chrono::DateTime::parse_from_rfc3339(added_at)
        .map(|ts| ts.with_timezone(&chrono::Utc) < cutoff)
        .unwrap_or(false)
}

fn persist_bindings(core: &Core, gw: &GatewayState) {
    let bindings = gw
        .bindings
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .clone();
    let path = bindings_path(&core.shared_scope().into_root());
    if let Some(parent) = path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    let file = BindingsFile {
        version: 2,
        bindings,
    };
    if let Ok(json) = serde_json::to_string_pretty(&file) {
        let temp = path.with_extension(format!("json.{}.tmp", std::process::id()));
        if std::fs::write(&temp, json).is_ok() {
            let _ = std::fs::rename(temp, path);
        }
    }
}

fn persist_allowlist(core: &Core, gw: &GatewayState) {
    let entries = gw
        .allowlist
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .clone();
    let path = allowlist_path(&core.shared_scope().into_root());
    write_allowlist_file(&path, &entries);
}

fn persist_bots(core: &Core, gw: &GatewayState) {
    let bots = gw
        .bots
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .clone();
    persist_bots_map(&bots_path(&core.shared_scope().into_root()), &bots);
}

/// Atomic temp-file+rename write, same pattern as `write_allowlist_file`.
/// Free function (not a `GatewayState` method) so the one-time migration in
/// `GatewayState::load` can call it before a `GatewayState` exists.
fn persist_bots_map(path: &std::path::Path, bots: &HashMap<String, Bot>) {
    if let Some(parent) = path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    let mut sorted: Vec<Bot> = bots.values().cloned().collect();
    sorted.sort_by(|a, b| a.id.cmp(&b.id));
    let file = BotsFile {
        schema: 1,
        bots: sorted,
    };
    if let Ok(json) = serde_json::to_string_pretty(&file) {
        let temp = path.with_extension(format!("json.{}.tmp", std::process::id()));
        if std::fs::write(&temp, json).is_ok() {
            let _ = std::fs::rename(temp, path);
        }
    }
}

/// Atomic temp-file+rename write, same pattern as `persist_bindings`.
fn write_allowlist_file(path: &std::path::Path, entries: &HashMap<String, AllowlistEntry>) {
    if let Some(parent) = path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    let mut sorted: Vec<AllowlistEntry> = entries.values().cloned().collect();
    sorted.sort_by(|a, b| a.key.cmp(&b.key));
    let file = AllowlistFile {
        schema: 1,
        entries: sorted,
    };
    if let Ok(json) = serde_json::to_string_pretty(&file) {
        let temp = path.with_extension(format!("json.{}.tmp", std::process::id()));
        if std::fs::write(&temp, json).is_ok() {
            let _ = std::fs::rename(temp, path);
        }
    }
}

pub fn routes() -> Router<AppState> {
    Router::new()
        .route(
            "/gateway/inbound",
            axum::routing::post(gateway_inbound)
                .layer(axum::extract::DefaultBodyLimit::max(INBOUND_BODY_MAX_BYTES)),
        )
        .route("/gateway/status", axum::routing::get(gateway_status))
        .route(
            "/gateway/bindings/{key}",
            axum::routing::delete(gateway_unbind),
        )
}

// ---- Inbound ----------------------------------------------------------------

#[derive(serde::Deserialize)]
struct InboundBody {
    surface: String,
    chat: String,
    /// Who sent this, when the adapter knows (a Telegram @user, a webhook
    /// identity). Typed since G1 groundwork: it is recorded on deliveries
    /// and attributed on queued turns instead of being silently dropped.
    #[serde(default)]
    sender: Option<String>,
    text: String,
    #[serde(default)]
    wait: bool,
    /// Base64 images appended to the prompt as vision content.
    #[serde(default)]
    attachments: Vec<InboundAttachment>,
    #[serde(default)]
    capabilities: Option<crate::delivery::RequestedCapabilities>,
    /// See [`InboundRequest::bot_id`].
    #[serde(default)]
    bot_id: Option<String>,
    /// Caller-provided idempotency key. Repeating it returns the original
    /// admission without dispatching a second model turn.
    #[serde(default)]
    request_id: Option<String>,
}

#[derive(serde::Deserialize)]
struct InboundAttachment {
    /// MIME type; defaults to image/png for Telegram-style senders.
    #[serde(default = "default_image_mime")]
    mime: String,
    data: String,
    /// Original filename, when the channel knows it (documents only).
    #[serde(default)]
    filename: Option<String>,
    /// "image" (default, vision content) or "document": small text is
    /// inlined, anything else is saved to the workspace inbox and named.
    #[serde(default = "default_attachment_kind")]
    kind: String,
    #[serde(default)]
    error: Option<String>,
}

fn default_image_mime() -> String {
    "image/png".into()
}

fn default_attachment_kind() -> String {
    "image".into()
}

/// The largest document a channel may hand the gateway, shared by every
/// bridge so a file is never downloaded by one side and dropped by the
/// other. The gateway's JSON body limit (2 MiB, base64 inflates by a third)
/// bounds it.
pub(crate) const INBOUND_DOCUMENT_MAX_BYTES: usize = 20 * 1024 * 1024;

/// `/gateway/inbound`'s body limit: a document at the limit, base64-encoded,
/// with room for the rest of the request.
const INBOUND_BODY_MAX_BYTES: usize = INBOUND_DOCUMENT_MAX_BYTES / 3 * 4 + 1024 * 1024;

/// Text documents at or under this size are inlined in the prompt; larger
/// text, and every non-text file, is saved to the workspace inbox instead.
const DOCUMENT_INLINE_MAX_BYTES: usize = 64 * 1024;

/// What a channel turn answers: its text, and each workspace file the turn
/// drafted with `office_apply`, for the channel to get back.
pub(crate) struct ChatReply {
    pub(crate) text: String,
    pub(crate) drafts: Vec<TurnDraft>,
    pub(crate) cards: Vec<vak_delivery::StructuredOutput>,
}

/// The latest draft this turn made of one workspace file.
pub(crate) struct TurnDraft {
    /// Workspace-relative, as the call named it.
    path: String,
    draft: std::path::PathBuf,
}

/// Each workspace file this turn's successful `office_apply` calls drafted,
/// with the last draft of it: an `office_apply` call's draft is at
/// `.vak/scratch/<agent>/<call id>/<path>`, the convention Review's lineage
/// relies on too.
fn turn_drafts(log: &vak_session::SessionLog, workspace: &std::path::Path) -> Vec<TurnDraft> {
    let Some(directive) = log.latest_directive_entry_id() else {
        return Vec::new();
    };
    let agent = log
        .header()
        .and_then(|header| header.agent.as_ref().map(|agent| agent.id.clone()))
        .unwrap_or_else(|| "vak".into());
    let mut in_turn = false;
    let mut calls: Vec<(String, String)> = Vec::new();
    let mut succeeded = std::collections::HashSet::new();
    for (entry_id, message) in log.message_chain() {
        in_turn |= entry_id == directive;
        if !in_turn {
            continue;
        }
        for block in &message.content {
            match block {
                vak_llm::ContentBlock::ToolUse { id, name, input } if name == "office_apply" => {
                    if let Some(path) = input.get("path").and_then(serde_json::Value::as_str) {
                        calls.push((id.clone(), path.trim().to_string()));
                    }
                }
                vak_llm::ContentBlock::ToolResult {
                    tool_use_id,
                    is_error: false,
                    ..
                } => {
                    succeeded.insert(tool_use_id.clone());
                }
                _ => {}
            }
        }
    }
    let mut drafts: Vec<TurnDraft> = Vec::new();
    for (id, path) in calls.into_iter().filter(|(id, _)| succeeded.contains(id)) {
        let draft = vak_config::scope::WorkspaceScope::new(workspace)
            .scratch(&agent)
            .join(&id)
            .join(&path);
        if !draft.is_file() {
            continue;
        }
        drafts.retain(|earlier| earlier.path != path);
        drafts.push(TurnDraft { path, draft });
    }
    drafts
}

/// The largest draft sent back on a channel; Telegram's bots may send up to
/// 50 MB, and a document this large is better opened in Vakyartha.
const RETURN_FILE_MAX_BYTES: u64 = 20 * 1024 * 1024;

/// Each draft the turn made, for a channel that takes files: its bytes and a
/// caption saying what changed (from the worker's semantic diff), under the
/// name the person knows it by. A draft that carries a sensitivity label is
/// not sent (labels only narrow where a file goes); a channel that takes no
/// files, or a draft too large, gets a line saying where the file is.
async fn return_drafts(
    core: &Core,
    mut text: String,
    drafts: &[TurnDraft],
    accepts_files: bool,
) -> (String, Vec<serde_json::Value>) {
    use base64::Engine as _;
    let worker = core.tool_worker_exe();
    let mut files = Vec::new();
    let mut notes = Vec::new();
    for draft in drafts {
        let name = inbox::display_name(
            std::path::Path::new(&draft.path)
                .file_name()
                .and_then(|name| name.to_str())
                .unwrap_or(&draft.path),
        );
        let facts = vak_tools::broker::office_project(
            &worker,
            &draft.draft,
            vak_tools::broker::OfficeView::Facts,
        )
        .await;
        let labels: Vec<String> = facts
            .as_ref()
            .ok()
            .and_then(|facts| facts.get("sensitivity_labels"))
            .and_then(|labels| serde_json::from_value(labels.clone()).ok())
            .unwrap_or_default();
        if !labels.is_empty() {
            notes.push(format!(
                "{name} carries the sensitivity label {}, so it is not sent on this channel; review the draft in Vakyartha.",
                labels.join(", ")
            ));
            continue;
        }
        let size = std::fs::metadata(&draft.draft)
            .map(|metadata| metadata.len())
            .unwrap_or(u64::MAX);
        if !accepts_files || size > RETURN_FILE_MAX_BYTES {
            notes.push(format!(
                "The updated {name} is ready in Vakyartha for review; this channel does not receive it."
            ));
            continue;
        }
        let current = core.cwd().join(&draft.path);
        let review = vak_tools::broker::office_review(
            &worker,
            current.is_file().then_some(current.as_path()),
            &draft.draft,
            None,
        )
        .await;
        let summary = review
            .as_ref()
            .ok()
            .and_then(|review| review.get("summary"))
            .and_then(|summary| serde_json::from_value::<Vec<String>>(summary.clone()).ok())
            .filter(|summary| !summary.is_empty())
            .map(|summary| summary.join("; "))
            .unwrap_or_else(|| "no visible change".into());
        let Ok(bytes) = std::fs::read(&draft.draft) else {
            notes.push(format!("The updated {name} could not be read to send it."));
            continue;
        };
        files.push(serde_json::json!({
            "name": name,
            "mime": office_mime(&name),
            "data": base64::engine::general_purpose::STANDARD.encode(bytes),
            "caption": format!("Updated {name}: {summary}"),
        }));
    }
    if !notes.is_empty() {
        text = format!("{}\n\n{}", text.trim_end(), notes.join("\n"));
    }
    (text, files)
}

fn office_mime(name: &str) -> &'static str {
    match name
        .rsplit('.')
        .next()
        .map(str::to_ascii_lowercase)
        .as_deref()
    {
        Some("docx") => "application/vnd.openxmlformats-officedocument.wordprocessingml.document",
        Some("xlsx") => "application/vnd.openxmlformats-officedocument.spreadsheetml.sheet",
        Some("pptx") => "application/vnd.openxmlformats-officedocument.presentationml.presentation",
        Some("pdf") => "application/pdf",
        _ => "application/octet-stream",
    }
}

/// Compose the prompt message: text, vision blocks, inlined text documents,
/// and a note for each document saved to the inbox. A document's bytes
/// never enter the prompt unless they are text (docs/design/72, F1). The
/// ledger stores exactly what the model will see (invariant 1).
fn compose_prompt(
    text: &str,
    attachments: &[InboundAttachment],
    workspace: &std::path::Path,
) -> vak_llm::Message {
    let mut blocks = Vec::new();
    if !text.is_empty() {
        blocks.push(vak_llm::ContentBlock::text(text));
    }
    for a in attachments {
        if a.data.trim().is_empty() {
            continue;
        }
        if a.kind == "document" {
            blocks.push(vak_llm::ContentBlock::text(document_block(a, workspace)));
            continue;
        }
        // A voice note reaches the model as its transcript (or the reason
        // there is none) in the message text, never as an attachment block.
        if a.kind == "audio" {
            continue;
        }
        blocks.push(vak_llm::ContentBlock::image_base64(
            a.mime.clone(),
            a.data.trim().to_string(),
        ));
    }
    vak_llm::Message {
        role: vak_llm::Role::User,
        content: blocks,
    }
}

fn document_block(attachment: &InboundAttachment, workspace: &std::path::Path) -> String {
    use base64::Engine as _;
    let filename = attachment.filename.as_deref().unwrap_or("file");
    let Ok(bytes) = base64::engine::general_purpose::STANDARD.decode(attachment.data.trim()) else {
        return format!("[attached file '{filename}' could not be decoded; not included]");
    };
    if bytes.len() > INBOUND_DOCUMENT_MAX_BYTES {
        return format!(
            "[attached file '{filename}' ({} KiB) exceeds the {} KiB channel limit; not received]",
            bytes.len() / 1024,
            INBOUND_DOCUMENT_MAX_BYTES / 1024
        );
    }
    let text = std::str::from_utf8(&bytes)
        .ok()
        .filter(|text| !text.contains('\0'));
    if let Some(content) = text
        && bytes.len() <= DOCUMENT_INLINE_MAX_BYTES
    {
        return format!("Attached file `{filename}`:\n```\n{content}\n```");
    }
    match save_to_inbox(workspace, filename, &bytes) {
        Ok(saved) => inbox::note(filename, &saved, &bytes),
        Err(error) => {
            format!("[attached file '{filename}' could not be saved ({error}); not included]")
        }
    }
}

pub(crate) fn compose_voice_prompt(text: &str) -> vak_llm::Message {
    compose_prompt(text, &[], std::path::Path::new("."))
}

// ---- Approval forwarding (G2) ----------------------------------------------

/// Approver for gateway-driven turns. In `deny` mode this behaves like
/// `AutoDeny`. In `forward` mode the gate is announced on the approver
/// surface and resolved by a yes/no reply; timeout or silence fails closed.
struct GatewayApprover {
    events_tx: crate::events::EventBus,
    state: Arc<GatewayState>,
    core: Core,
    session_id: String,
}

#[async_trait::async_trait]
impl vak_agent::Approver for GatewayApprover {
    /// A forwarded gate reaches the configured approver chat; without
    /// forward mode nothing is listening, and the reachability preflight
    /// must see that before the prompt advertises a gated capability.
    fn answerable(&self) -> bool {
        self.state.forward_mode()
    }

    /// A worker's question is carried to the same approver chat as a gate
    /// and answered by a typed `answer` reply, so the same condition holds.
    fn answers_questions(&self) -> bool {
        self.state.forward_mode()
    }

    fn question_window(&self) -> Option<Duration> {
        Some(self.state.approval_timeout())
    }

    async fn announce_question(
        &self,
        question: &vak_agent::PendingQuestion,
        board: &Arc<vak_agent::WorkerRegistry>,
    ) {
        if !self.state.forward_mode() {
            board.questions().close(&question.id);
            return;
        }
        self.state.register_question(
            &question.id,
            &question.parent_session_id,
            &question.label,
            board.clone(),
        );
        let code = question_code(&question.id);
        let mut detail = format!(
            "Question from {} [{code}]\n{}",
            question.label, question.question
        );
        if !question.options.is_empty() {
            detail.push_str(&format!("\nOptions: {}", question.options.join(" | ")));
        }
        detail.push_str(&format!(
            "\nReply 'answer {code} <your answer>'. Your answer helps it continue; it does not approve any action."
        ));
        let delivered = deliver_approval_and_record(
            &self.core,
            self.state.approver_target().unwrap_or_default().as_str(),
            ApprovalPayload {
                request_id: question.id.clone(),
                title: format!("Question from {} [{code}]", question.label),
                detail,
                expires_at: Some(
                    (chrono::Utc::now()
                        + chrono::Duration::from_std(self.state.approval_timeout())
                            .unwrap_or_default())
                    .to_rfc3339(),
                ),
                actions: Vec::new(),
            },
            vak_core::inbox::Kind::ApprovalPending,
            format!("Question from {} [{code}]", question.label),
            Some(&self.session_id),
            None,
            None,
        )
        .await;
        if let Err(error) = delivered {
            // Nobody was told, so nobody will answer: end the worker's wait
            // now rather than leaving it blocked on a question no one saw.
            eprintln!("[gateway] question announcement failed: {error}");
            self.state.forget_question(&question.id);
            board.questions().close(&question.id);
        }
    }

    async fn approve(
        &self,
        tool: &str,
        args_json: &str,
        reason: &str,
        call_id: Option<&str>,
    ) -> bool {
        if !self.state.forward_mode() {
            return false;
        }
        let id = uuid::Uuid::now_v7().to_string();
        let rx = self.state.register_gate(&id, &self.session_id);
        let _ = self.events_tx.send(AgentEvent::ApprovalRequested {
            id: id.clone(),
            tool: tool.to_string(),
            args_json: args_json.to_string(),
            reason: reason.to_string(),
        });
        let short = question_code(&id);
        let announce = format!(
            "Approval requested [{short}]\nTool: {tool}\nArgs: {args_json}\nReason: {reason}\nReply 'yes' or 'no' to decide."
        );
        if let Err(e) = deliver_approval_and_record(
            &self.core,
            self.state.approver_target().unwrap_or_default().as_str(),
            ApprovalPayload {
                request_id: id.clone(),
                title: format!("Approval requested [{short}]"),
                detail: announce.clone(),
                expires_at: Some(
                    (chrono::Utc::now()
                        + chrono::Duration::from_std(self.state.approval_timeout())
                            .unwrap_or_default())
                    .to_rfc3339(),
                ),
                actions: vec![
                    delivery_action("approve", "Approve", "approve", &id),
                    delivery_action("deny", "Deny", "deny", &id),
                ],
            },
            vak_core::inbox::Kind::ApprovalPending,
            format!("Approval requested [{short}]"),
            Some(&self.session_id),
            None,
            call_id,
        )
        .await
        {
            eprintln!("[gateway] approval announcement failed: {e}");
            let _ = self.state.resolve_gate(false, Some(&id));
            return false;
        }
        match tokio::time::timeout(self.state.approval_timeout(), rx).await {
            Ok(Ok(v)) => v,
            Ok(Err(_)) => false, // gate dropped (run cancelled)
            Err(_) => {
                // Timed out: remove our own entry so a late reply resolves
                // nothing.
                self.state
                    .pending_approvals
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner)
                    .remove(&id);
                eprintln!("[gateway] approval {short} timed out; denied");
                false
            }
        }
    }
}

/// "yes"/"no" vocabulary for chat replies, optionally addressed to one
/// gate: "yes ab12cd34". Deliberately small and strict — casual chatter
/// from the approver chat must not resolve gates. Returns the verdict and
/// the gate-id prefix when one was supplied.
fn parse_verdict(text: &str) -> Option<(bool, Option<String>)> {
    let mut tokens = text.split_whitespace();
    let head = tokens.next()?.to_lowercase();
    let verdict = match head.as_str() {
        "y" | "yes" | "approve" | "approved" | "ok" | "allow" => true,
        "n" | "no" | "deny" | "denied" | "block" => false,
        _ => return None,
    };
    // Extra prose after a bare verdict is ignored; exactly one short token
    // is treated as a gate id.
    let id = match tokens.next() {
        Some(t)
            if tokens.next().is_none()
                && t.len() >= 4
                && t.chars().all(|c| c.is_ascii_alphanumeric()) =>
        {
            Some(t.to_lowercase())
        }
        _ => None,
    };
    Some((verdict, id))
}

/// An `answer` reply from the approver chat: `answer <code> <text>`,
/// `answer <code>: <text>`, or `answer <text>` when only one question is
/// waiting. Returns the text and the code when one was given. Strict on the
/// leading word, so ordinary chatter from that chat is not read as an answer.
/// `known` is the codes of the questions now waiting, because a first word
/// is only a code when it names one.
fn parse_answer(text: &str, known: &[String]) -> Option<(Option<String>, String)> {
    let text = text.trim();
    let (head, rest) = text.split_once(char::is_whitespace)?;
    if !head.trim_end_matches(':').eq_ignore_ascii_case("answer") {
        return None;
    }
    let rest = rest.trim();
    if rest.is_empty() {
        return None;
    }
    if let Some((first, after)) = rest.split_once(char::is_whitespace) {
        let candidate = first.trim_end_matches(':').to_lowercase();
        if known.contains(&candidate) && !after.trim().is_empty() {
            return Some((Some(candidate), after.trim().to_string()));
        }
    }
    Some((None, rest.to_string()))
}

async fn gateway_inbound(
    State(state): State<AppState>,
    Json(body): Json<InboundBody>,
) -> axum::response::Response {
    crate::refresh_control_plane(&state);
    if !state.gateway.enabled {
        return (
            StatusCode::CONFLICT,
            Json(serde_json::json!({
                "error": "gateway disabled: set [gateway] enabled = true (trusted config) or pass serve --gateway"
            })),
        )
            .into_response();
    }
    let text = body.text.trim().to_string();
    let has_attachment = body.attachments.iter().any(|a| !a.data.trim().is_empty());
    if body.surface.trim().is_empty()
        || body.chat.trim().is_empty()
        || (text.is_empty() && !has_attachment)
    {
        return (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({"error": "surface, chat and text are required"})),
        )
            .into_response();
    }
    // Multi-bot-per-channel: a chat's key is scoped to the bot that
    // delivered the message whenever the bridge knows its own bot id
    // (`--bot-id`), so the same physical chat served by several bots gets
    // one independent allowlist entry, session, and policy per bot instead
    // of all of them colliding onto one shared conversation. Legacy/
    // single-bot bridges (no bot id) keep the original two-part key
    // unchanged. See `legacy_key_for` for the one-time migration this
    // implies for an already-approved chat or a `chat_allowlist` row.
    let key = match body
        .bot_id
        .as_deref()
        .map(str::trim)
        .filter(|b| !b.is_empty())
    {
        Some(bot_id) => format!("{}:{}:{bot_id}", body.surface.trim(), body.chat.trim()),
        None => format!("{}:{}", body.surface.trim(), body.chat.trim()),
    };
    // 0c-01/0c-02/docs/design/34: chat allowlist — reject messages from
    // unknown chats, but record a reviewable *pending* entry instead of a
    // flat rejection so the operator has a forward path to "let it
    // through" that isn't a hand-edited config file + process restart.
    // `chat_allowlist_open = true` still bypasses the store entirely.
    if !state.gateway.chat_allowlist_open() {
        let decision = state.gateway.allowlist_resolve_inbound(
            &state.core,
            &key,
            &text,
            body.bot_id.as_deref(),
        );
        match decision {
            AllowlistDecision::Allowed => {}
            AllowlistDecision::Denied => {
                vak_core::security_events::record(
                    &state.core.scope(),
                    vak_core::security_events::EventKind::ChatDenied,
                    "chat_denied",
                    &format!("key={key}"),
                    None,
                );
                return (
                    StatusCode::FORBIDDEN,
                    Json(serde_json::json!({
                        "error": format!("chat '{key}' rejected: denied by operator"),
                        "state": "denied",
                    })),
                )
                    .into_response();
            }
            AllowlistDecision::NewlyPending => {
                vak_core::security_events::record(
                    &state.core.scope(),
                    vak_core::security_events::EventKind::ChatPending,
                    "chat_pending",
                    &format!("key={key}"),
                    None,
                );
                return (
                    StatusCode::FORBIDDEN,
                    Json(serde_json::json!({
                        "error": format!(
                            "chat '{key}' rejected: awaiting operator approval in the admin console"
                        ),
                        "state": "pending",
                    })),
                )
                    .into_response();
            }
            AllowlistDecision::StillPending => {
                // A lighter, non-security-event log line: this is expected
                // repeat traffic from an already-reviewable key, not a
                // fresh incident to append to the audit trail each time.
                eprintln!("[gateway] chat '{key}' still pending operator review");
                return (
                    StatusCode::FORBIDDEN,
                    Json(serde_json::json!({
                        "error": format!(
                            "chat '{key}' rejected: still awaiting operator approval"
                        ),
                        "state": "pending",
                    })),
                )
                    .into_response();
            }
        }
    }

    // Approval replies from the designated approver surface resolve the
    // addressed gate (or the oldest one) instead of becoming conversation
    // input. Any non-verdict text from that chat falls through to normal
    // routing.
    if state.gateway.forward_mode()
        && state.gateway.approver_target().as_deref() == Some(key.as_str())
        && let Some((verdict, gate_id)) = parse_verdict(&text)
    {
        return match state.gateway.resolve_gate(verdict, gate_id.as_deref()) {
            Ok(resolved) => {
                // A chat "no" is observable here and nowhere else, so the
                // durable record of the denial is written at the same beat.
                if !verdict {
                    let short = question_code(&resolved.id);
                    let _ = vak_core::inbox::record(
                        &vak_config::scope::AgentScope::new(state.core.shared_scope().into_root()),
                        vak_core::inbox::Kind::ApprovalDenied,
                        &format!("approval denied [{short}]"),
                        &format!(
                            "session {} denied forwarded gate {} ({} pending)",
                            resolved.session_id, resolved.id, resolved.remaining
                        ),
                        Some(&resolved.session_id),
                        None,
                        None,
                    );
                }
                (
                    StatusCode::OK,
                    Json(serde_json::json!({
                        "state": "approval_resolved",
                        "approved": verdict,
                        "gate": resolved.id,
                        "session_id": resolved.session_id,
                        "remaining": resolved.remaining,
                    })),
                )
                    .into_response()
            }
            Err(()) => (
                StatusCode::OK,
                Json(serde_json::json!({ "state": "no_pending_approvals" })),
            )
                .into_response(),
        };
    }

    // A worker's question was carried to the approver chat; its `answer`
    // reply resolves it instead of becoming conversation input, and only the
    // designated approver chat can (invariant 15). Anything else from that
    // chat falls through to normal routing.
    if state.gateway.forward_mode()
        && state.gateway.approver_target().as_deref() == Some(key.as_str())
        && let Some((code, answer)) = parse_answer(&text, &state.gateway.forwarded_question_codes())
    {
        let (status, reply) = match state.gateway.answer_forwarded(code.as_deref(), &answer) {
            QuestionReply::Answered {
                code,
                label,
                session_id,
            } => (
                StatusCode::OK,
                serde_json::json!({
                    "state": "question_answered",
                    "question": code,
                    "worker": label,
                    "session_id": session_id,
                }),
            ),
            QuestionReply::NoneOpen => (
                StatusCode::OK,
                serde_json::json!({ "state": "no_pending_questions" }),
            ),
            QuestionReply::Ambiguous(codes) => (
                StatusCode::OK,
                serde_json::json!({
                    "state": "question_ambiguous",
                    "error": "more than one question is waiting; say which: answer <code> <text>",
                    "codes": codes,
                }),
            ),
            QuestionReply::Closed => (
                StatusCode::OK,
                serde_json::json!({ "state": "question_closed" }),
            ),
            QuestionReply::Refused(why) => (
                StatusCode::BAD_REQUEST,
                serde_json::json!({ "state": "question_refused", "error": why }),
            ),
        };
        return (status, Json(reply)).into_response();
    }

    // docs/design/34 Phase 2: run this key's entry through its own
    // workspace's Core (sandbox, permission mode, session ledger) — not
    // just its provider/model — when the entry names a workspace other
    // than the gateway's own. Falls back to the gateway's default Core
    // when the entry has no workspace override, exactly as before.
    let core = match state.gateway.core_for_entry(&state.core, &key) {
        Ok(core) => core,
        Err(e) => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(serde_json::json!({"error": format!("workspace core unavailable: {e}")})),
            )
                .into_response();
        }
    };

    // Admission owns the conversation context. Stamp it before creating or
    // reopening the bound session so the ledger, prompt contract, and every
    // later delivery can identify the authorized audience and originating
    // transport without reverse-engineering mutable gateway state.
    let conversation_context = vak_session::ConversationContext {
        conversation_id: key.clone(),
        audience_id: key.clone(),
        origin: Some(vak_session::ConversationOrigin {
            surface: body.surface.trim().to_string(),
            address: body.chat.trim().to_string(),
            bot_id: body.bot_id.clone(),
        }),
    };
    let request_id = body
        .request_id
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToOwned::to_owned)
        .unwrap_or_else(|| format!("gateway-{}", uuid::Uuid::now_v7()));
    // The run's cause and actor are what the gateway resolved: the endpoint
    // the message arrived on, and the sender behind it (docs/design/73 §4).
    let endpoint = match body.bot_id.as_deref().filter(|id| !id.trim().is_empty()) {
        Some(bot) => format!("{}:{}:{bot}", body.surface.trim(), body.chat.trim()),
        None => format!("{}:{}", body.surface.trim(), body.chat.trim()),
    };
    let sender = body.sender.as_deref().map(str::trim).unwrap_or_default();
    let admission = vak_core::admission::RunAdmission::default()
        .cause(vak_session::trace::Cause::Channel {
            endpoint,
            request_id: request_id.clone(),
        })
        .actor(vak_session::trace::local::channel_sender(
            body.surface.trim(),
            body.chat.trim(),
            sender,
        ));
    let core = core
        .with_conversation_context(Some(conversation_context))
        .with_run_admission(admission.clone());
    // One key for the request: the bus announcement below and the run it
    // starts share it, so they are one trace.
    let admitted = core.mint_trace(None);
    let core = core.with_run_admission(admission.trace(admitted.clone()));

    let handle = match resolve_session(&state, &core, &key).await {
        Ok(h) => h,
        Err(e) => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(serde_json::json!({"error": e})),
            )
                .into_response();
        }
    };

    let already_admitted = handle
        .admissions
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .contains(&request_id)
        || handle
            .session
            .lock()
            .ok()
            .and_then(|guard| {
                guard
                    .as_ref()
                    .map(|log| log.has_request_admission(&request_id))
            })
            .unwrap_or(false);
    if already_admitted {
        return (
            StatusCode::ACCEPTED,
            Json(serde_json::json!({
                "request_id": request_id,
                "state": "already_admitted",
                "decision": "duplicate",
                "session_id": binding_session(&state, &key),
            })),
        )
            .into_response();
    }
    handle
        .admissions
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .insert(request_id.clone());
    // Voice notes are transcribed only now — after allowlist admission and
    // request de-duplication, so an unknown chat or a retried request never
    // spends a provider call — through this chat's bot → chat voice tiers.
    // Spoken words are steering text, never control: commands are parsed from
    // what was typed.
    let mut voice_notes = Vec::new();
    for attachment in body.attachments.iter().filter(|a| a.kind == "audio") {
        voice_notes.push(
            crate::voice::transcribe_voice_note(
                &state,
                &core,
                &key,
                &attachment.mime,
                &attachment.data,
                attachment.error.as_deref(),
            )
            .await,
        );
    }
    let expanded_text = std::iter::once(text.clone())
        .filter(|typed| !typed.is_empty())
        .chain(voice_notes.iter().map(crate::voice::VoiceNote::prompt_line))
        .collect::<Vec<_>>()
        .join("\n");
    let mut admission_data = std::collections::BTreeMap::from([
        ("request_id".into(), request_id.clone()),
        ("sender".into(), body.sender.clone().unwrap_or_default()),
        ("origin_surface".into(), body.surface.trim().to_string()),
        ("origin_address".into(), body.chat.trim().to_string()),
        (
            "agent_id".into(),
            core.agent_identity()
                .map(|agent| agent.id.clone())
                .unwrap_or_else(|| "vak".into()),
        ),
    ]);
    if let Some(context) = core.conversation_context() {
        admission_data.insert("audience_id".into(), context.audience_id.clone());
        admission_data.insert("conversation_id".into(), context.conversation_id.clone());
        if let Some(origin) = &context.origin {
            admission_data.insert("bot_id".into(), origin.bot_id.clone().unwrap_or_default());
        }
    }
    crate::record_activity_or_buffer(
        &handle,
        vak_session::ActivityRecord {
            activity_id: format!("admission-{request_id}"),
            kind: vak_session::ActivityKind::Run,
            status: vak_session::ActivityStatus::Running,
            label: "Gateway request accepted".into(),
            detail: Some(expanded_text.clone()),
            data: admission_data,
        },
    );

    // Busy? Queue as logged steering input; the running loop consumes it
    // between model steps, and any leftovers run as a continuation turn.
    // The full composed message (text + images) is queued so nothing the
    // sender supplied is degraded to bare text.
    let who = body
        .sender
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .unwrap_or("unknown");
    let preview: String = expanded_text.chars().take(80).collect();
    state
        .hub
        .emit_gateway_inbound(&body.surface, who, &preview, Some(admitted));
    // Only an explicit command is control; everything else a person types
    // while the run is busy is steering text (docs/design/47, control
    // plane). "Stop using semicolons" steers; "/stop" or a bare "stop"
    // cancels.
    let command = vak_intent::parse_command(&text);
    let intervention = command
        .as_ref()
        .map(vak_intent::Command::intervention_kind)
        .unwrap_or(vak_intent::InterventionKind::Steer);
    let session_id = binding_session(&state, &key);
    if matches!(
        intervention,
        vak_intent::InterventionKind::Replan
            | vak_intent::InterventionKind::Reprioritize
            | vak_intent::InterventionKind::AddRequirement
            | vak_intent::InterventionKind::RemoveRequirement
    ) && let Some(id) = session_id.clone()
    {
        return crate::plan_change(
            axum::extract::State(state.clone()),
            axum::extract::Path(id),
            axum::Json(crate::PlanChangeBody {
                text: text.clone(),
                source: "human".into(),
                target_revision: None,
            }),
        )
        .await;
    }
    match intervention {
        vak_intent::InterventionKind::Status => {
            crate::record_control_activity(&handle, "Run status requested", "status");
            let paused = handle.steering.is_paused();
            let running = handle
                .session
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .is_none();
            return (
                StatusCode::OK,
                Json(serde_json::json!({
                    "state": "status",
                    "session_id": session_id,
                    "running": running,
                    "paused": paused,
                })),
            )
                .into_response();
        }
        vak_intent::InterventionKind::Pause => {
            handle.steering.pause();
            crate::record_control_activity(&handle, "Run paused", "pause");
            return (
                StatusCode::ACCEPTED,
                Json(serde_json::json!({
                    "state": "paused",
                    "session_id": session_id,
                })),
            )
                .into_response();
        }
        vak_intent::InterventionKind::Resume => {
            handle.steering.resume();
            crate::record_control_activity(&handle, "Run resumed", "resume");
            return (
                StatusCode::ACCEPTED,
                Json(serde_json::json!({
                    "state": "resumed",
                    "session_id": session_id,
                })),
            )
                .into_response();
        }
        vak_intent::InterventionKind::Cancel => {
            handle
                .cancel
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .cancel();
            crate::record_control_activity(&handle, "Run cancelled", "cancel");
            return (
                StatusCode::ACCEPTED,
                Json(serde_json::json!({
                    "state": "cancelled",
                    "session_id": session_id,
                })),
            )
                .into_response();
        }
        _ => {}
    }
    let busy = handle
        .session
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .is_none();
    if busy {
        let attributed = match body.sender.as_deref().map(str::trim) {
            Some(who) if !who.is_empty() => format!("[from {who}] {expanded_text}"),
            _ => expanded_text.clone(),
        };
        handle.steering.push_steering_message(compose_prompt(
            &attributed,
            &body.attachments,
            core.cwd(),
        ));
        return (
            StatusCode::ACCEPTED,
            Json(serde_json::json!({
                "state": "steering_queued",
                "session_id": session_id.unwrap_or_default(),
            })),
        )
            .into_response();
    }

    if core.provider().is_err() {
        return (
            StatusCode::SERVICE_UNAVAILABLE,
            Json(serde_json::json!({"error": "no provider credential configured"})),
        )
            .into_response();
    }

    let (reply_tx, reply_rx) = oneshot::channel::<ChatReply>();
    let want_reply = body.wait;
    // 0c-02: attribute the sender identity to the prompt text.
    let attributed = match body.sender.as_deref().map(str::trim) {
        Some(who) if !who.is_empty() => format!("[from {who}] {expanded_text}"),
        _ => expanded_text,
    };
    let prompt = compose_prompt(&attributed, &body.attachments, core.cwd());
    // Each heard voice note is a durable transcript activity on the
    // append-only ledger, the same evidence the web voice socket records.
    for note in &voice_notes {
        if let crate::voice::VoiceNote::Heard(transcript) = note
            && let Ok(mut session) = handle.session.lock()
            && let Some(session) = session.as_mut()
        {
            let _ = session.append_voice_transcript(
                format!("voice:{request_id}"),
                transcript.clone(),
                true,
            );
        }
    }
    // Bind this turn's `tasks` tool default (`Core::with_default_deliver_to`)
    // to the exact chat/bot destination it is running in. Chat surfaces use
    // the three-part target whenever a bot identity is known; this prevents a
    // scheduled result from falling back to another bot's credentials.
    // So "remind me every morning at 8" typed (or spoken, via Gemini Live
    // transcription feeding the same turn) into this chat reports back into
    // this same chat unless the model is told to route it elsewhere.
    // Same clone-and-stamp carries the surface, so the system prompt tells
    // the model its reply is read as a chat message rather than printed in a
    // terminal (docs/design/07-prompt.md). The pooled core underneath is
    // stamped `Server`; this narrows it to the actual transport.
    let core_for_turn = core
        .clone()
        .with_default_deliver_to(Some(format!(
            "{}:{}{}",
            body.surface.trim(),
            body.chat.trim(),
            body.bot_id
                .as_deref()
                .filter(|id| !id.trim().is_empty())
                .map(|id| format!(":{id}"))
                .unwrap_or_default()
        )))
        .with_surface(vak_core::Surface::Chat {
            channel: body.surface.trim().to_string(),
        })
        // Must agree with the approver `execute_turn_chain` installs below:
        // `GatewayApprover` in forward mode, `AutoDeny` otherwise. Stamped
        // here, before the session's prompt is composed, so the prompt can
        // decline to advertise a capability this chat could never use.
        .with_approver_answerable(state.gateway.forward_mode())
        .with_prompt_overlays(state.gateway.resolve_prompt_overlays(&key));
    start_turn_chain(
        &state,
        &core_for_turn,
        handle,
        prompt,
        want_reply.then_some(reply_tx),
    );

    if !want_reply {
        return (
            StatusCode::ACCEPTED,
            Json(serde_json::json!({ "state": "started", "request_id": request_id })),
        )
            .into_response();
    }
    match tokio::time::timeout(WAIT_TIMEOUT, reply_rx).await {
        Ok(Ok(reply)) => {
            let accepts_files = body
                .capabilities
                .as_ref()
                .and_then(|capabilities| capabilities.accepts_files)
                .unwrap_or(false);
            let cards = reply.cards;
            let (text, files) =
                return_drafts(&core, reply.text, &reply.drafts, accepts_files).await;
            // The turn's delivery posture, from its intent entry: the
            // session ledger when the run has handed it back, else the
            // handle's last known record.
            let intent_posture = binding_session(&state, &key)
                .as_deref()
                .and_then(|session_id| state.get(session_id))
                .and_then(|handle| {
                    let from_ledger = handle.session.lock().ok().and_then(|guard| {
                        guard.as_ref().and_then(|session| {
                            session.chain_to_root().iter().rev().find_map(|entry| {
                                match &entry.payload {
                                    vak_session::EntryPayload::Intent(record) => {
                                        Some(record.engagement.posture.delivery)
                                    }
                                    _ => None,
                                }
                            })
                        })
                    });
                    from_ledger.or_else(|| {
                        handle.intent.lock().ok().and_then(|guard| {
                            guard.as_ref().map(|record| record.engagement.posture.delivery)
                        })
                    })
                });
            let outcome_metadata = binding_session(&state, &key)
                .as_deref()
                .and_then(|session_id| state.get(session_id))
                .and_then(|handle| {
                    handle.presentation.lock().ok().map(|timeline| {
                        timeline
                            .items
                            .iter()
                            .rev()
                            .find_map(|item| match &item.content {
                                vak_delivery::OutputContent::Document { document } => {
                                    Some(document.metadata.clone())
                                }
                                _ => None,
                            })
                    })
                })
                .flatten();
            match crate::delivery::render_response(
                &core,
                body.surface.trim(),
                body.chat.trim(),
                text.clone(),
                body.capabilities.as_ref(),
                outcome_metadata,
                Some(std::collections::BTreeMap::from([
                    ("request_id".into(), request_id.clone()),
                    ("agent_id".into(),
                        core.agent_identity()
                            .map(|agent| agent.id.clone())
                            .unwrap_or_else(|| "vak".into())),
                    ("audience_id".into(),
                        core.conversation_context()
                            .map(|context| context.audience_id.clone())
                            .unwrap_or_else(|| key.clone())),
                    ("conversation_id".into(),
                        core.conversation_context()
                            .map(|context| context.conversation_id.clone())
                            .unwrap_or_else(|| key.clone())),
                    ("origin".into(), format!("{}:{}", body.surface.trim(), body.chat.trim())),
                    ("bot_id".into(), body.bot_id.clone().unwrap_or_default()),
                ])),
                body.bot_id.as_deref(),
                session_id.as_deref(),
                intent_posture,
                cards,
            )
            .await
            {
                Ok(delivery) => (
                    StatusCode::OK,
                    Json(serde_json::json!({
                        "state": "completed",
                        "request_id": request_id,
                        "text": text,
                        "session_id": session_id,
                        "delivery": delivery,
                        "files": files,
                    })),
                )
                    .into_response(),
                Err(error) => (
                    StatusCode::OK,
                    Json(serde_json::json!({
                        "state": "completed",
                        "request_id": request_id,
                        "text": text,
                        "session_id": session_id,
                        "delivery_error": error,
                        "files": files,
                    })),
                )
                    .into_response(),
            }
        }
        Ok(Err(_)) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({"error": "turn chain ended without a reply", "request_id": request_id})),
        )
            .into_response(),
        Err(_) => (
            StatusCode::GATEWAY_TIMEOUT,
            Json(serde_json::json!({
                "error": "turn did not finish in time; poll /sessions/{id}/transcript"
                ,"request_id": request_id
            })),
        )
            .into_response(),
    }
}

async fn gateway_status(State(state): State<AppState>) -> Json<serde_json::Value> {
    let bindings: Vec<serde_json::Value> = state
        .gateway
        .snapshot()
        .into_iter()
        .map(|(target, binding)| {
            let agent_id = state
                .gateway
                .allowlist_get(&target)
                .and_then(|entry| entry.agent_id)
                .unwrap_or_else(|| "vak".into());
            let paused = binding
                .session_id
                .as_deref()
                .and_then(|id| state.get(id))
                .is_some_and(|handle| handle.steering.is_paused());
            serde_json::json!({
                "target": target,
                "agent_id": agent_id,
                "session_id": binding.session_id,
                "provider": binding.provider,
                "model": binding.model,
                "workspace": binding.workspace,
                "route_revision": binding.route_revision,
                "paused": paused,
            })
        })
        .collect();
    Json(serde_json::json!({
        "enabled": state.gateway.enabled,
        "cwd": state.core.cwd(),
        "bindings": bindings,
        "approvals": {
            "mode": state.gateway.approvals_mode(),
            "approver": state.gateway.approver_target(),
            "pending": state.gateway.pending_approval_count(),
        },
        "questions": { "pending": state.gateway.forwarded_question_codes().len() },
    }))
}

async fn gateway_unbind(State(state): State<AppState>, Path(key): Path<String>) -> StatusCode {
    if state.gateway.unbind(&state.core, &key) {
        StatusCode::OK
    } else {
        StatusCode::NOT_FOUND
    }
}

// ---- Session resolution -----------------------------------------------------

fn binding_session(state: &AppState, key: &str) -> Option<String> {
    state
        .gateway
        .bindings
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .get(key)
        .and_then(|binding| binding.session_id.clone())
}

fn binding_route(state: &AppState, core: &Core, key: &str) -> (String, String, String) {
    let override_route = state.gateway.effective_route_override(key);
    if let Some((provider, model)) = override_route {
        let revision = format!("channel:{}:{}", provider, model);
        return (provider, model, revision);
    }
    let _ = core.refresh_persisted_route();
    let route = core.effective_route();
    (route.provider, route.model, route.revision)
}

fn busy_binding_matches_revision(state: &AppState, core: &Core, key: &str, revision: &str) -> bool {
    state
        .gateway
        .bindings
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .get(key)
        .is_some_and(|binding| {
            binding.route_revision.as_deref() == Some(revision)
                && binding.workspace.as_deref() == Some(core.cwd().as_path())
        })
}

fn session_matches_route(
    session: &vak_session::SessionLog,
    core: &Core,
    provider: &str,
    model: &str,
) -> bool {
    // A channel-scoped route override (revision prefix "channel:") is an
    // explicit operator-level binding: the session must have been created
    // with the overridden provider/model to be reusable — otherwise the
    // channel would silently get a different identity than intended (rule 23).
    //
    // Without an explicit override the current provider/model is always the
    // effective_route() which every turn already re-evaluates, so there is
    // nothing to rotate on — the existing session is always compatible.
    let has_channel_override = provider.contains('/') || {
        // Detect the "channel:p:m" revision stamp that binding_route emits
        // for overrides. We check by whether the caller got the route from an
        // override or from effective_route(): overrides set revision to
        // "channel:provider:model", effective_route returns a hash/timestamp.
        // The simplest proxy: provider/model differ from the workspace default.
        let route = core.effective_route();
        provider != route.provider || model != route.model
    };
    session.header().is_some_and(|header| {
        let workspace_ok = header.cwd.as_path() == core.cwd().as_path();
        let header_agent_id = header
            .agent
            .as_ref()
            .map(|a| a.id.as_str())
            .unwrap_or("vak");
        let core_agent_id = core
            .agent_identity()
            .map(|a| a.id.as_str())
            .unwrap_or("vak");
        let agent_ok = header_agent_id == core_agent_id;
        let conv_ok = core
            .conversation_context()
            .is_none_or(|expected| header.conversation.as_ref() == Some(expected));
        // Capabilities are rebound from the live admitted registry at each
        // turn boundary. The frozen packet is an audit snapshot, not a
        // reason to sever a conversation. In particular,
        // `capability_descriptors()` is the unresolved projection and can
        // differ from the admitted packet even with no configuration change.
        if has_channel_override {
            // Channel route overrides: session must match the pinned route.
            // Per-turn routing does not apply across explicit bot/channel splits.
            workspace_ok
                && agent_ok
                && conv_ok
                && header.contract.provider == provider
                && header.contract.model == model
        } else {
            // No override: per-turn routing handles provider/model, so a
            // session in this workspace and conversation remains valid.
            workspace_ok && agent_ok && conv_ok
        }
    })
}

/// Note a prompt-layer change in the security log before rotating.
///
/// Rotation is otherwise indistinguishable from a route change or a deleted
/// ledger, and "my bot started answering differently" is exactly the question
/// an operator brings to the audit trail.
///
/// Takes the already-computed drift rather than a session id: the caller
/// holds the ledger lock through its live handle, so reopening the session
/// here would fail every time and silently record nothing.
fn record_prompt_drift(
    core: &Core,
    key: &str,
    session_id: &str,
    drift: Option<vak_core::prompts::PromptDrift>,
) {
    let Some(drift) = drift else {
        return;
    };
    vak_core::security_events::record(
        &core.scope(),
        vak_core::security_events::EventKind::ConfigChange,
        "prompt layers changed",
        &format!(
            "chat {key} rotated off session {session_id}: {}",
            drift.lines().join("; ")
        ),
        None,
    );
}

/// Attach-or-create the session bound to `key`. Stale bindings (ledger
/// deleted through the normal endpoint) rebind to a fresh session.
async fn resolve_session(
    state: &AppState,
    core: &Core,
    key: &str,
) -> Result<Arc<SessionHandle>, String> {
    let (provider, model, revision) = binding_route(state, core, key);
    if let Some(sid) = binding_session(state, key) {
        if let Some(handle) = state.get(&sid) {
            let (matches, drift) = {
                let session = handle
                    .session
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner);
                match session.as_ref() {
                    Some(session) => (
                        session_matches_route(session, core, &provider, &model),
                        session
                            .header()
                            .and_then(|header| core.prompt_drift(&header.contract)),
                    ),
                    None => (
                        busy_binding_matches_revision(state, core, key, &revision),
                        None,
                    ),
                }
            };
            if matches {
                return Ok(handle);
            }
            record_prompt_drift(core, key, &sid, drift);
            state.gateway.rotate(core, key);
        } else {
            match core.open_session(&sid).await {
                Ok(session) => {
                    if session_matches_route(&session, core, &provider, &model) {
                        let id = session
                            .header()
                            .map(|h| h.session_id.clone())
                            .unwrap_or_else(|| sid.clone());
                        return Ok(crate::register_handle(
                            state,
                            id,
                            session,
                            core.cwd().clone(),
                            core.clone(),
                        ));
                    }
                    record_prompt_drift(
                        core,
                        key,
                        &sid,
                        session
                            .header()
                            .and_then(|header| core.prompt_drift(&header.contract)),
                    );
                    state.gateway.rotate(core, key);
                }
                Err(_) => {
                    state.gateway.rotate(core, key);
                }
            }
        }
    }
    let session = core
        .start_session_with_route(provider, model)
        .await
        .map_err(|e| format!("start session: {e}"))?;
    let id = session
        .header()
        .map(|h| h.session_id.clone())
        .unwrap_or_default();
    let handle =
        crate::register_handle(state, id.clone(), session, core.cwd().clone(), core.clone());
    // Two racing first-messages could each mint a session; last bind wins
    // and the loser stays a hidden header-only draft.
    state.gateway.bind(core, key.to_string(), id, revision);
    Ok(handle)
}

// ---- Turn execution ---------------------------------------------------------

/// Run a turn chain: prompt, then any steering left queued by concurrent
/// inbound messages, until the queue is dry. Sends one `RunFinished` per
/// leg so SSE consumers see normal terminal markers. The loop/lock
/// mechanics (run a leg, decide whether to continue) are
/// `crate::run_turn_chain` — the ONE executor also used by the HTTP `/run`
/// and `/steering` endpoints (invariant 30); only the approver construction
/// and the per-leg settle bookkeeping below are gateway-specific.
fn start_turn_chain(
    state: &AppState,
    core: &Core,
    handle: Arc<SessionHandle>,
    prompt: vak_llm::Message,
    reply: Option<oneshot::Sender<ChatReply>>,
) {
    let core = core.clone();
    let gw = state.gateway.clone();
    tokio::spawn(execute_turn_chain(core, gw, handle, prompt, reply));
}

/// Voice and other non-HTTP surfaces use the same governed executor while
/// already holding the frozen session core and gateway state.
pub(crate) fn start_turn_chain_with_gateway(
    gateway: Arc<GatewayState>,
    core: &Core,
    handle: Arc<SessionHandle>,
    prompt: vak_llm::Message,
    reply: Option<oneshot::Sender<ChatReply>>,
) {
    tokio::spawn(execute_turn_chain(
        core.clone(),
        gateway,
        handle,
        prompt,
        reply,
    ));
}

async fn execute_turn_chain(
    core: Core,
    gw: Arc<GatewayState>,
    handle: Arc<SessionHandle>,
    prompt: vak_llm::Message,
    mut reply: Option<oneshot::Sender<ChatReply>>,
) {
    let taken = handle
        .session
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .take();
    let Some(taken) = taken else {
        // Lost the race with another writer; hand our full prompt (text +
        // images) to the winner as steering instead of dropping it.
        handle.steering.push_steering_message(prompt);
        return;
    };

    let approver_gw = gw.clone();
    let approver_core = core.clone();
    let approver_handle = handle.clone();
    let approver_factory = move |session_id: &str| -> Arc<dyn vak_agent::Approver> {
        // Forward mode is the human-in-the-loop gate (AGENTS rule 16). Under
        // FullAccess the engine returns `Allow` for bash/read/write, so no
        // Ask is ever raised and the approver is never invoked — the gate is
        // silently hollow. Warn once; keep going so legitimate setups still
        // run, but make the bypass unmistakable.
        if approver_gw.forward_mode()
            && matches!(
                approver_core.effective_permission_mode(),
                vak_config::PermissionMode::FullAccess
            )
            && FORWARD_FULLACCESS_WARNED
                .compare_exchange(
                    false,
                    true,
                    std::sync::atomic::Ordering::AcqRel,
                    std::sync::atomic::Ordering::Acquire,
                )
                .is_ok()
        {
            eprintln!(
                "vak gateway: WARNING approvals=\"forward\" is configured while the \
                 effective permission_mode is FullAccess — bash/read/write resolve to \
                 Allow, so no Ask gate is raised and the forward approver is never \
                 invoked (the human-in-the-loop is silently bypassed for Allow-class \
                 tools). Set [permissions] permission_mode = \"workspace-write\" on this \
                 workspace to make forward mode effective, or drop the forward \
                 approval configuration."
            );
        }
        // Unattended policy: deny by default, forward to the approver
        // surface when configured (G2).
        if approver_gw.forward_mode() {
            Arc::new(GatewayApprover {
                events_tx: approver_handle.events_tx.clone(),
                state: approver_gw.clone(),
                core: approver_core.clone(),
                session_id: session_id.to_string(),
            })
        } else {
            Arc::new(AutoDeny)
        }
    };

    let settle_core = core.clone();
    let settle = move |session_id: &str,
                       outcome: Result<
        (vak_agent::TurnOutcome, vak_session::SessionLog),
        vak_core::CoreError,
    >| {
        let core = settle_core.clone();
        let reply_tx = reply.take();
        let session_id = session_id.to_string();
        async move {
            match outcome {
                Ok((o, log)) => {
                    let err = outcome_is_error(&o);
                    let text = crate::projection::channel_narration(&log, outcome_text(&o));
                    if let Some(tx) = reply_tx {
                        let _ = tx.send(ChatReply {
                            text: text.clone(),
                            drafts: turn_drafts(&log, core.cwd()),
                            cards: crate::projection::run_cards(&log),
                        });
                    }
                    // Background reflection seam (docs/design/29 P1): the
                    // shared best-effort pass over the just-settled leg. It
                    // runs strictly after the reply above was handed over so
                    // delivery never waits on it, and while this chain still
                    // owns the ledger — a second in-process handle cannot
                    // take the file lock. Bounded; failures collapse into
                    // the outcome envelope.
                    if !err && core.config().memory.reflection {
                        let pass = tokio::time::timeout(
                            REFLECTION_CALL_TIMEOUT,
                            core.reflect_after_turn(&log, ""),
                        )
                        .await;
                        match pass {
                            Ok(outcome) => log_gateway_reflection(outcome),
                            Err(_) => eprintln!("[gateway] reflection skipped: timeout"),
                        }
                    }
                    (Some(log), short_summary(&text), err)
                }
                Err(e) => {
                    // The full error is recorded internally for operators;
                    // the channel reply stays a human sentence (see
                    // `outcome_text`) — never the raw `CoreError` display.
                    vak_core::security_events::record(
                        &core.scope(),
                        vak_core::security_events::EventKind::ExecutionError,
                        "inbound turn failed",
                        &format!("session_id={session_id} error={e}"),
                        None,
                    );
                    let recovered = recover_ledger(&core, &session_id).await;
                    let text = crate::client_events::run_outcome_message(
                        crate::client_events::RunOutcome::Failed,
                    )
                    .to_string();
                    if let Some(tx) = reply_tx {
                        let _ = tx.send(ChatReply {
                            text: text.clone(),
                            drafts: Vec::new(),
                            cards: Vec::new(),
                        });
                    }
                    (recovered, short_summary(&text), true)
                }
            }
        }
    };

    crate::run_turn_chain(
        core,
        handle,
        taken,
        crate::TurnStart::message(prompt),
        approver_factory,
        settle,
    )
    .await;
}

/// A `CoreError` loses the ledger handle (the agent consumed it); reopen the
/// JSONL so the session stays usable in this long-lived process. `None`
/// when `session_id` is empty or the reopen itself fails — the caller (the
/// settle closure above) treats that as "nothing to continue with" and lets
/// `run_turn_chain` end the chain, exactly like `recover_ledger` returning
/// `false` used to leave `handle.session` untouched.
async fn recover_ledger(core: &Core, session_id: &str) -> Option<vak_session::SessionLog> {
    if session_id.is_empty() {
        return None;
    }
    core.open_session(session_id).await.ok()
}

/// The text sent back to a Telegram/Slack/Discord user as the bot's reply
/// for this turn. A completed turn's real answer passes through untouched;
/// anything that did not produce a normal answer falls back to the same
/// small set of human sentences `ClientEvent::RunFinished` uses for every
/// other client (`client_events::run_outcome_message`), never the raw
/// `TurnOutcome::Failed` error — a channel reply is not a debug log.
fn outcome_text(o: &vak_agent::TurnOutcome) -> String {
    use crate::client_events::{RunOutcome, run_outcome_message};
    use vak_agent::TurnOutcome;
    match o {
        TurnOutcome::Completed { response } => {
            let t = response.text_content();
            if t.trim().is_empty() {
                "(no text)".into()
            } else {
                t
            }
        }
        TurnOutcome::Aborted { partial } => partial
            .as_ref()
            .map(|m| m.text_content())
            .filter(|t| !t.trim().is_empty())
            .unwrap_or_else(|| run_outcome_message(RunOutcome::Stopped).into()),
        TurnOutcome::Failed { .. } => run_outcome_message(RunOutcome::Failed).into(),
        TurnOutcome::MaxTurnsReached => run_outcome_message(RunOutcome::MaxTurns).into(),
    }
}

fn outcome_is_error(o: &vak_agent::TurnOutcome) -> bool {
    matches!(
        o,
        vak_agent::TurnOutcome::Failed { .. } | vak_agent::TurnOutcome::MaxTurnsReached
    )
}

fn short_summary(text: &str) -> String {
    let first = text.lines().next().unwrap_or("").trim();
    let mut s: String = first.chars().take(80).collect();
    if first.chars().count() > 80 {
        s.push('…');
    }
    if s.is_empty() { "completed".into() } else { s }
}

// ---- Outbound delivery ------------------------------------------------------

fn http_client() -> reqwest::Client {
    static CLIENT: std::sync::OnceLock<reqwest::Client> = std::sync::OnceLock::new();
    CLIENT
        .get_or_init(|| {
            reqwest::Client::builder()
                .timeout(Duration::from_secs(15))
                .build()
                .unwrap_or_default()
        })
        .clone()
}

/// Delivery plus its durable pull-side twin (docs/design/29-personal-os.md
/// P6): append the same signal before transport so a failed push cannot erase it.
/// `<home>/inbox.jsonl` under `inbox_kind` so unattended output survives
/// even when no chat channel is reachable. Inbox recording is best-effort
/// by contract — it can never fail a delivery that already happened.
pub(crate) async fn deliver_and_record(
    core: &Core,
    target: &str,
    text: &str,
    inbox_kind: vak_core::inbox::Kind,
    title: String,
    session_id: Option<&str>,
    task_id: Option<&str>,
) -> Result<(), String> {
    deliver_and_record_with_result(
        core, target, text, inbox_kind, title, session_id, task_id, None,
    )
    .await
    .map(|_| ())
}

#[allow(clippy::too_many_arguments)]
pub(crate) async fn deliver_and_record_with_result(
    core: &Core,
    target: &str,
    text: &str,
    inbox_kind: vak_core::inbox::Kind,
    title: String,
    session_id: Option<&str>,
    task_id: Option<&str>,
    result_id: Option<&str>,
) -> Result<&'static str, String> {
    let dedupe_key = result_id.map(|result| format!("{target}|{result}|{}", inbox_kind as u8));
    let _ = vak_core::inbox::record_with_result_and_key(
        &vak_config::scope::AgentScope::new(core.shared_scope().into_root()),
        inbox_kind,
        &title,
        text,
        session_id,
        task_id,
        result_id,
        dedupe_key.as_deref(),
        core.admitted_trace(),
    );
    let cleaned_text = crate::projection::clean_scaffolding(text);
    let mut answer = AnswerDraft::from_markdown(cleaned_text);
    if let Some(value) = task_id {
        answer.metadata.insert("vak_task_id".into(), value.into());
    }
    if let Some(value) = session_id {
        answer
            .metadata
            .insert("vak_session_id".into(), value.into());
    }
    if let Some(value) = result_id {
        answer.metadata.insert("vak_result_id".into(), value.into());
    }
    crate::delivery::deliver(
        core,
        target,
        DeliveryKind::TaskSummary,
        DeliveryContent::Answer(answer),
    )
    .await
    .map(|packet| {
        if packet
            .diagnostics
            .iter()
            .any(|diagnostic| diagnostic.starts_with("delivery held:"))
        {
            "queued"
        } else {
            "delivered"
        }
    })
}

#[allow(clippy::too_many_arguments)]
async fn deliver_approval_and_record(
    core: &Core,
    target: &str,
    approval: ApprovalPayload,
    inbox_kind: vak_core::inbox::Kind,
    title: String,
    session_id: Option<&str>,
    task_id: Option<&str>,
    tool_use_id: Option<&str>,
) -> Result<(), String> {
    let _ = match tool_use_id {
        Some(call) => vak_core::inbox::record_for_call(
            &vak_config::scope::AgentScope::new(core.shared_scope().into_root()),
            inbox_kind,
            &title,
            &approval.detail,
            session_id,
            Some(call),
            core.admitted_trace(),
        ),
        None => vak_core::inbox::record(
            &vak_config::scope::AgentScope::new(core.shared_scope().into_root()),
            inbox_kind,
            &title,
            &approval.detail,
            session_id,
            task_id,
            core.admitted_trace(),
        ),
    };
    crate::delivery::deliver(
        core,
        target,
        DeliveryKind::Approval,
        DeliveryContent::Approval(approval),
    )
    .await
    .map(|_| ())
}

fn delivery_action(id: &str, label: &str, verb: &str, request_id: &str) -> DeliveryAction {
    DeliveryAction {
        id: id.into(),
        label: label.into(),
        verb: verb.into(),
        data: [("request_id".into(), request_id.into())]
            .into_iter()
            .collect(),
    }
}

/// Transient webhook failures retry with bounded exponential backoff.
/// 4xx (except 429) are the receiver's permanent answer and return at once;
/// network errors, timeouts, 429 and 5xx are retried.
const WEBHOOK_ATTEMPTS: u32 = 3;

fn webhook_retryable(status: Option<u16>) -> bool {
    match status {
        None => true,
        Some(429) => true,
        Some(c) => c >= 500,
    }
}

pub(crate) async fn deliver_webhook_packet(
    core: &Core,
    name: &str,
    packet: &DeliveryPacket,
) -> Result<(), String> {
    let hook = core.config().gateway.webhooks.get(name).ok_or_else(|| {
        let known: Vec<&String> = core.config().gateway.webhooks.keys().collect();
        format!("unknown webhook '{name}'; configured: {known:?}")
    })?;
    // Fail closed: a configured credential that is missing must not turn
    // into an unauthenticated post of agent output.
    let token = match &hook.token_env {
        Some(env_name) => Some(
            vak_config::get_var(env_name)
                .ok_or_else(|| format!("webhook '{name}' token_env '{env_name}' is not set"))?,
        ),
        None => None,
    };
    let payload = serde_json::json!({
        "target": format!("webhook:{name}"),
        "text": packet.fallback_markdown,
        "ts": chrono::Utc::now().to_rfc3339(),
        "job_id": packet.job_id,
        "delivery": packet,
    });

    let mut last_error = String::new();
    for attempt in 0..WEBHOOK_ATTEMPTS {
        if attempt > 0 {
            tokio::time::sleep(Duration::from_millis(400u64 << (attempt - 1))).await;
        }
        let mut req = http_client()
            .post(&hook.url)
            .header("Idempotency-Key", &packet.job_id)
            .json(&payload);
        if let Some(token) = &token {
            req = req.bearer_auth(token);
        }
        match req.send().await {
            Ok(resp) => {
                let status = resp.status();
                if status.is_success() {
                    return Ok(());
                }
                last_error = format!("webhook '{name}' returned {status}");
                if !webhook_retryable(Some(status.as_u16())) {
                    return Err(last_error);
                }
            }
            Err(e) => {
                last_error = format!("webhook '{name}' post failed: {e}");
            }
        }
    }
    Err(last_error)
}

/// Reflection outcome reporting for chat surfaces: the success line keeps
/// its historical format; config-driven and raced-out skips are routine and
/// stay silent so a busy gateway does not spam its own log.
fn log_gateway_reflection(outcome: vak_core::reflection::ReflectionOutcome) {
    use vak_core::reflection::ReflectionOutcome as R;
    match outcome {
        R::Reflected {
            notes_added,
            skills_proposed,
        } => {
            if notes_added > 0 || skills_proposed {
                eprintln!(
                    "[gateway] reflection: {notes_added} note(s) persisted, skill queued: {skills_proposed}"
                );
            }
        }
        R::Skipped { reason } => match reason {
            "already-in-flight" | "reflection-disabled" | "memory-writes-disabled" => {}
            other => eprintln!("[gateway] reflection skipped: {other}"),
        },
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::*;
    use crate::inbox::{INBOX_DIR, sanitize_filename};

    #[test]
    fn an_answer_reply_is_strict_about_its_leading_word() {
        let codes = vec!["a1b2c3".to_string()];
        assert_eq!(
            parse_answer("answer a1b2c3 fiscal 2026", &codes),
            Some((Some("a1b2c3".into()), "fiscal 2026".into()))
        );
        assert_eq!(
            parse_answer("Answer A1B2C3: fiscal 2026", &codes),
            Some((Some("a1b2c3".into()), "fiscal 2026".into()))
        );
        assert_eq!(
            parse_answer("answer: fiscal 2026", &codes),
            Some((None, "fiscal 2026".into())),
            "no code: the only waiting question"
        );
        assert_eq!(
            parse_answer("answer zzzzzz hello", &codes),
            Some((None, "zzzzzz hello".into())),
            "a first word that names no waiting question is part of the answer"
        );
        for chatter in [
            "yes",
            "2026",
            "answering machine",
            "please answer a1b2c3 x",
            "answer",
        ] {
            assert_eq!(parse_answer(chatter, &codes), None, "{chatter:?}");
        }
    }

    #[test]
    fn a_question_code_is_the_random_tail_of_its_id() {
        assert_eq!(
            question_code("01a0f786-16dc-78f0-872c-8c4225034817"),
            "034817"
        );
        // Two questions asked a minute apart share an id head, never a tail.
        assert_ne!(
            question_code("01a0f786-16dc-78f0-872c-8c4225034817"),
            question_code("01a0f786-9a11-7123-9b2d-5e0000aa11bb")
        );
    }

    /// The path a note names, quoted after "at path".
    fn saved_path(note: &str) -> Option<&str> {
        note.split_once("at path \"")?
            .1
            .split_once('"')
            .map(|(path, _)| path)
    }

    #[test]
    fn a_channel_reply_for_a_failed_run_is_human_text_never_the_raw_error() {
        let outcome = vak_agent::TurnOutcome::Failed {
            error: vak_llm::LlmError::Network("connection reset by peer at 10.0.0.1:443".into()),
        };
        let text = outcome_text(&outcome);
        assert!(
            !text.contains("10.0.0.1") && !text.contains("connection reset"),
            "raw provider error leaked into the channel reply: {text:?}"
        );
        assert_eq!(
            text,
            crate::client_events::run_outcome_message(crate::client_events::RunOutcome::Failed)
        );
    }

    #[test]
    fn a_channel_reply_for_max_turns_names_the_step_limit_not_a_raw_code() {
        let text = outcome_text(&vak_agent::TurnOutcome::MaxTurnsReached);
        assert_eq!(
            text,
            crate::client_events::run_outcome_message(crate::client_events::RunOutcome::MaxTurns)
        );
    }

    #[test]
    fn voice_prompt_is_a_normal_user_message() {
        let prompt = compose_voice_prompt("  turn the lights on  ");
        assert_eq!(prompt.role, vak_llm::Role::User);
        assert_eq!(prompt.content.len(), 1);
        match &prompt.content[0] {
            vak_llm::ContentBlock::Text { text } => {
                assert_eq!(text, "  turn the lights on  ");
            }
            other => panic!("voice prompt used unexpected content block: {other:?}"),
        }
    }

    #[test]
    fn a_voice_note_reaches_the_model_only_as_its_text() {
        let prompt = compose_prompt(
            "[voice note not transcribed: no speech was recognized]",
            &[InboundAttachment {
                kind: "audio".into(),
                mime: "audio/ogg".into(),
                data: "not-model-input".into(),
                filename: Some("voice.ogg".into()),
                error: None,
            }],
            std::path::Path::new("."),
        );
        assert_eq!(prompt.content.len(), 1);
        assert!(matches!(
            &prompt.content[0],
            vak_llm::ContentBlock::Text { text } if text.contains("no speech was recognized")
        ));
    }

    #[test]
    fn inbound_audio_budget_marks_oversized_payloads() {
        struct Channel;
        impl InboundChannel for Channel {
            fn surface(&self) -> &'static str {
                "test"
            }
        }
        let encoded = "A".repeat(16 * 1024 * 1024 * 4 / 3 + 1);
        let request = InboundRequest::new(&Channel, "chat", "sender", "voice")
            .unwrap()
            .with_attachments(vec![serde_json::json!({"kind":"audio", "data": encoded})]);
        assert_eq!(request.attachments[0]["data"], "");
        assert_eq!(
            request.attachments[0]["error"],
            "audio attachment exceeds 16 MiB"
        );
    }

    /// The console resolved a chat's mode WITHOUT the bot tier, so a bot
    /// pinned narrower than its chat ran narrow and displayed wide — and a
    /// chat with no pin under a bot that had one displayed the workspace's
    /// mode instead of the bot's. Showing a channel as wider than it runs is
    /// the one direction of error that matters here.
    #[test]
    fn a_bot_pin_narrows_the_chat_and_the_console_says_so() {
        use vak_config::PermissionMode::*;
        let ws = tempfile::tempdir().unwrap();
        vak_config::paths::isolate_home_for_tests();
        std::fs::create_dir_all(ws.path().join(".vak")).unwrap();
        std::fs::write(
            ws.path().join(".vak/config.toml"),
            "permission_mode = \"full-access\"\n",
        )
        .unwrap();
        vak_core::trust::record(ws.path()).unwrap();

        // Chat asks for more than its bot allows: the bot wins.
        let r = resolve_channel_permission(ws.path(), Some(FullAccess), Some(ReadOnly));
        assert_eq!(r.effective, ReadOnly);
        assert_eq!(r.bot_mode, Some(ReadOnly));
        assert!(r.was_capped(), "a reduced grant must be visible");

        // Chat has no pin: the bot's applies exactly. Nothing was reduced,
        // so this is not a capping event — but the console must still show
        // `workspace-write`, where it used to show the workspace's
        // `full-access` because the bot tier was never consulted.
        let r = resolve_channel_permission(ws.path(), None, Some(WorkspaceWrite));
        assert_eq!(r.effective, WorkspaceWrite);
        assert_eq!(r.workspace_mode, FullAccess);
        assert!(!r.was_capped());

        // A chat narrower than its bot is not "capped" — it got what it asked.
        let r = resolve_channel_permission(ws.path(), Some(ReadOnly), Some(FullAccess));
        assert_eq!(r.effective, ReadOnly);
        assert!(!r.was_capped());
    }

    /// The workspace ceiling still wins over both, in either order.
    #[test]
    fn the_workspace_ceiling_is_never_escaped_by_a_bot_or_a_chat() {
        use vak_config::PermissionMode::*;
        let ws = tempfile::tempdir().unwrap();
        vak_config::paths::isolate_home_for_tests();
        std::fs::create_dir_all(ws.path().join(".vak")).unwrap();
        std::fs::write(
            ws.path().join(".vak/config.toml"),
            "permission_mode = \"read-only\"\n",
        )
        .unwrap();
        vak_core::trust::record(ws.path()).unwrap();

        for (chat, bot) in [
            (Some(FullAccess), Some(FullAccess)),
            (Some(FullAccess), None),
            (None, Some(FullAccess)),
            (None, None),
        ] {
            let r = resolve_channel_permission(ws.path(), chat, bot);
            assert_eq!(r.effective, ReadOnly, "chat={chat:?} bot={bot:?}");
        }
    }

    /// `forward` with no usable target is not representable: both the
    /// constructor and the setter collapse it to `deny`, the same rule the
    /// config loader applies.
    #[test]
    fn an_unbacked_forward_policy_resolves_to_deny() {
        let timeout = Duration::from_secs(300);
        for approver in [None, Some(""), Some("   "), Some("no-colon")] {
            let policy = ApprovalPolicy::resolve("forward", approver, timeout);
            assert_eq!(policy.approvals, "deny", "approver={approver:?}");
            assert!(policy.approver.is_none());
        }
        let policy = ApprovalPolicy::resolve("forward", Some("telegram:42"), timeout);
        assert_eq!(policy.approvals, "forward");
        assert_eq!(policy.approver.as_deref(), Some("telegram:42"));
    }

    /// Regression for the classic serde `Option<Option<T>>` trap: a plain
    /// double-`Option` field can't tell "the key was never sent" apart
    /// from "the key was sent as `null`" — both collapse to the outer
    /// `None`. `deserialize_present` is the fix; this locks in all three
    /// states a PATCH body actually needs.
    #[test]
    fn deserialize_present_distinguishes_absent_null_and_value() {
        #[derive(serde::Deserialize)]
        struct Body {
            #[serde(default, deserialize_with = "deserialize_present")]
            field: Option<Option<String>>,
        }

        let absent: Body = serde_json::from_str("{}").unwrap();
        assert_eq!(absent.field, None, "key never sent must mean 'leave alone'");

        let explicit_null: Body = serde_json::from_str(r#"{"field": null}"#).unwrap();
        assert_eq!(
            explicit_null.field,
            Some(None),
            "explicit null must mean 'clear it', not be indistinguishable from absent"
        );

        let set: Body = serde_json::from_str(r#"{"field": "x"}"#).unwrap();
        assert_eq!(set.field, Some(Some("x".to_string())));
    }

    #[test]
    fn bare_verdicts_have_no_gate_id() {
        assert_eq!(parse_verdict("yes"), Some((true, None)));
        assert_eq!(parse_verdict("  NO "), Some((false, None)));
        assert_eq!(parse_verdict("approve"), Some((true, None)));
    }

    #[test]
    fn addressed_verdict_extracts_single_short_token() {
        assert_eq!(
            parse_verdict("yes ab12cd34"),
            Some((true, Some("ab12cd34".into())))
        );
        assert_eq!(
            parse_verdict("no deadbeef"),
            Some((false, Some("deadbeef".into())))
        );
    }

    #[test]
    fn prose_after_verdict_is_never_an_id() {
        assert_eq!(parse_verdict("yes please do it now"), Some((true, None)));
        assert_eq!(parse_verdict("no way"), Some((false, None)));
    }

    #[test]
    fn chatter_is_not_a_verdict() {
        assert_eq!(parse_verdict("sure thing"), None);
        assert_eq!(parse_verdict(""), None);
        assert_eq!(parse_verdict("approved!"), None);
    }

    fn document(name: &str, bytes: &[u8]) -> InboundAttachment {
        use base64::Engine as _;
        InboundAttachment {
            mime: "application/octet-stream".into(),
            data: base64::engine::general_purpose::STANDARD.encode(bytes),
            filename: Some(name.into()),
            kind: "document".into(),
            error: None,
        }
    }

    fn note(msg: &vak_llm::Message) -> &str {
        let vak_llm::ContentBlock::Text { text } = &msg.content[1] else {
            unreachable!("expected a text block for a document attachment");
        };
        text
    }

    #[test]
    fn small_text_document_is_inlined_as_a_fenced_text_block() {
        let workspace = tempfile::tempdir().unwrap();
        let msg = compose_prompt(
            "check this",
            &[document("notes.py", b"print('hi')")],
            workspace.path(),
        );
        let text = note(&msg);
        assert!(text.contains("Attached file `notes.py`"));
        assert!(text.contains("print('hi')"));
        assert!(
            !workspace.path().join(INBOX_DIR).exists(),
            "inlined text is not saved"
        );
    }

    #[test]
    fn large_text_is_saved_to_the_inbox_not_inlined() {
        let workspace = tempfile::tempdir().unwrap();
        let huge = "x".repeat(DOCUMENT_INLINE_MAX_BYTES + 1);
        let msg = compose_prompt(
            "check this",
            &[document("notes.py", huge.as_bytes())],
            workspace.path(),
        );
        let text = note(&msg);
        assert!(
            !text.contains("xxxx"),
            "the raw content must not be inlined"
        );
        let saved = saved_path(text).unwrap();
        assert!(saved.ends_with("-notes.py"), "{text}");
        assert_eq!(
            std::fs::read_to_string(workspace.path().join(saved)).unwrap(),
            huge
        );
    }

    #[test]
    fn an_office_file_is_saved_and_named_never_inlined() {
        let workspace = tempfile::tempdir().unwrap();
        let bytes = vak_ooxml::fixtures::docx();
        let msg = compose_prompt(
            "summarise",
            &[document("Q3 report.docx", &bytes)],
            workspace.path(),
        );
        let text = note(&msg);
        assert!(text.contains("Read it with doc_read"), "{text}");
        assert!(text.contains("-Q3 report.docx"), "{text}");
        assert!(
            !text.contains("PK"),
            "no package bytes in the prompt: {text}"
        );
        let inbox = workspace.path().join(INBOX_DIR);
        let entries: Vec<_> = std::fs::read_dir(&inbox).unwrap().collect();
        assert_eq!(entries.len(), 1);
        let path = entries[0].as_ref().unwrap().path();
        assert_eq!(std::fs::read(path).unwrap(), bytes);

        // The same bytes again are the same file, not a second copy.
        let again = compose_prompt(
            "again",
            &[document("Q3 report.docx", &bytes)],
            workspace.path(),
        );
        assert_eq!(
            note(&again).replace("again", ""),
            text.replace("summarise", "")
        );
        assert_eq!(std::fs::read_dir(&inbox).unwrap().count(), 1);
    }

    #[test]
    fn other_binary_files_are_saved_and_described_honestly() {
        let workspace = tempfile::tempdir().unwrap();
        let msg = compose_prompt(
            "look",
            &[document("photo.heic", b"\x00\x00\x00\x18ftypheic\xff\xfe")],
            workspace.path(),
        );
        let text = note(&msg);
        assert!(text.contains("not a text, PDF or Open XML file"), "{text}");
        assert!(!text.contains("ftyp"), "{text}");
    }

    #[test]
    fn a_pdf_is_saved_and_named_for_doc_read() {
        let workspace = tempfile::tempdir().unwrap();
        let msg = compose_prompt(
            "look",
            &[document("scan.pdf", b"%PDF-1.7\x00\xff\xfe binary")],
            workspace.path(),
        );
        let text = note(&msg);
        assert!(text.contains("Read it with doc_read"), "{text}");
        assert!(!text.contains("%PDF"), "{text}");
    }

    #[test]
    fn hostile_filenames_stay_in_the_inbox() {
        let workspace = tempfile::tempdir().unwrap();
        for name in [
            "../../etc/passwd",
            "..\\..\\boot.ini",
            "...",
            "a/b/.hidden",
            "sub\x00dir",
        ] {
            let msg = compose_prompt("x", &[document(name, b"\x00binary")], workspace.path());
            let text = note(&msg);
            let saved = saved_path(text).unwrap_or_else(|| panic!("{name}: {text}"));
            assert!(
                !saved[6..].contains('/') && !saved.contains(".."),
                "{name} -> {saved}"
            );
            assert!(workspace.path().join(saved).is_file(), "{name} -> {saved}");
        }
        assert_eq!(sanitize_filename("../../etc/passwd"), "passwd");
        assert_eq!(sanitize_filename("..."), "file");
        assert_eq!(sanitize_filename(".hidden"), "hidden");
    }

    #[cfg(unix)]
    #[test]
    fn a_planted_inbox_symlink_cannot_redirect_the_write() {
        let workspace = tempfile::tempdir().unwrap();
        let outside = tempfile::tempdir().unwrap();
        std::os::unix::fs::symlink(outside.path(), workspace.path().join(INBOX_DIR)).unwrap();
        let msg = compose_prompt("x", &[document("a.bin", b"\x00")], workspace.path());
        assert!(note(&msg).contains("could not be saved"), "{}", note(&msg));
        assert_eq!(std::fs::read_dir(outside.path()).unwrap().count(), 0);
    }

    #[test]
    fn a_document_over_the_channel_cap_is_not_received() {
        let workspace = tempfile::tempdir().unwrap();
        let big = vec![0u8; INBOUND_DOCUMENT_MAX_BYTES + 1];
        let msg = compose_prompt("x", &[document("big.bin", &big)], workspace.path());
        assert!(note(&msg).contains("channel limit; not received"));
        assert!(!workspace.path().join(INBOX_DIR).exists());
    }

    #[test]
    fn image_attachment_still_becomes_vision_content() {
        let msg = compose_prompt(
            "look",
            &[InboundAttachment {
                mime: "image/png".into(),
                data: "aGVsbG8=".into(),
                filename: None,
                kind: "image".into(),
                error: None,
            }],
            std::path::Path::new("."),
        );
        assert!(matches!(
            msg.content[1],
            vak_llm::ContentBlock::Image { .. }
        ));
    }

    #[test]
    fn webhook_retry_matrix() {
        assert!(webhook_retryable(None), "network error retries");
        assert!(webhook_retryable(Some(429)));
        assert!(webhook_retryable(Some(503)));
        assert!(!webhook_retryable(Some(401)), "auth is permanent");
        assert!(!webhook_retryable(Some(404)));
        assert!(!webhook_retryable(Some(200)));
    }

    #[test]
    fn legacy_session_map_loads_as_versioned_binding_records() {
        let dir = tempfile::tempdir().unwrap();
        let core = Core::new(dir.path().to_path_buf()).unwrap();
        core.set_sessions_home(dir.path().join("home"));
        let path = bindings_path(&core.shared_scope().into_root());
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(&path, r#"{"telegram:42":"old-session"}"#).unwrap();
        let gateway = GatewayState::load(&core, true);
        let snapshot = gateway.snapshot();
        assert_eq!(snapshot.len(), 1);
        assert_eq!(snapshot[0].0, "telegram:42");
        assert_eq!(snapshot[0].1.session_id.as_deref(), Some("old-session"));
        assert!(snapshot[0].1.provider.is_none());
    }

    #[tokio::test]
    async fn route_change_rotates_binding_without_rewriting_old_session() {
        let dir = tempfile::tempdir().unwrap();
        let core = Core::new(dir.path().to_path_buf()).unwrap();
        core.set_sessions_home(dir.path().join("home"));
        let state = AppState::new(core.clone());
        let old = core
            .start_session_with_route("provider-a".into(), "model-a".into())
            .await
            .unwrap();
        let old_id = old.header().unwrap().session_id.clone();
        crate::register_handle(
            &state,
            old_id.clone(),
            old,
            core.cwd().clone(),
            core.clone(),
        );
        state.gateway.bind(
            &core,
            "telegram:42".into(),
            old_id.clone(),
            "old-revision".into(),
        );
        state.gateway.set_route_override(
            &core,
            "telegram:42".into(),
            Some(("provider-b".into(), "model-b".into())),
        );

        let fresh = resolve_session(&state, &core, "telegram:42").await.unwrap();
        assert_ne!(fresh.id, old_id);
        {
            let lock = fresh
                .session
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            let contract = &lock.as_ref().unwrap().header().unwrap().contract;
            assert_eq!(contract.provider, "provider-b");
            assert_eq!(contract.model, "model-b");
        }
        let old_path = core.scope().session_file(core.cwd(), &old_id);
        assert!(old_path.is_file(), "old append-only ledger remains intact");
    }

    #[tokio::test]
    async fn agent_change_rotates_binding_without_rewriting_old_session() {
        let dir = tempfile::tempdir().unwrap();
        let core = Core::new(dir.path().to_path_buf()).unwrap();
        core.set_sessions_home(dir.path().join("home"));
        let state = AppState::new(core.clone());
        let old = core
            .start_session_with_route("provider-a".into(), "model-a".into())
            .await
            .unwrap();
        let old_id = old.header().unwrap().session_id.clone();
        crate::register_handle(
            &state,
            old_id.clone(),
            old,
            core.cwd().clone(),
            core.clone(),
        );
        state
            .gateway
            .bind(&core, "telegram:42".into(), old_id.clone(), "rev".into());

        // Core is re-resolved with the new agent identity (e.g. Researcher)
        let researcher_identity = vak_session::types::AgentIdentity {
            id: "researcher".into(),
            revision: 1,
            name: "Researcher".into(),
            character: "vak".into(),
            personality: "curious".into(),
            animation: "subtle".into(),
            voice: "default".into(),
            behaviour: "thorough".into(),
            responsibilities: "deep research".into(),
            instructions: String::new(),
        };
        let core_researcher = core.clone().with_agent_identity(Some(researcher_identity));

        let fresh = resolve_session(&state, &core_researcher, "telegram:42")
            .await
            .unwrap();
        assert_ne!(fresh.id, old_id);
        {
            let lock = fresh
                .session
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            let agent = lock
                .as_ref()
                .unwrap()
                .header()
                .unwrap()
                .agent
                .as_ref()
                .unwrap();
            assert_eq!(agent.id, "researcher");
        }
        let old_path = core.scope().session_file(core.cwd(), &old_id);
        assert!(old_path.is_file(), "old agent ledger remains intact");
    }

    /// One persona, not two. The `identity` prompt block wins over the
    /// deprecated `VoiceConfig.persona`, and the legacy field still works
    /// when no prompt tier sets one (docs/design/45-prompt-layers.md).
    #[tokio::test]
    async fn identity_block_is_the_persona_with_voice_config_as_fallback() {
        let dir = tempfile::tempdir().unwrap();
        let core = Core::new(dir.path().to_path_buf()).unwrap();
        core.set_sessions_home(dir.path().join("home"));
        let state = AppState::new(core.clone());

        state.gateway.bot_upsert(
            &core,
            Bot {
                id: "support".into(),
                surface: "telegram".into(),
                label: "Support".into(),
                token_env: "BOT_TOKEN__SUPPORT".into(),
                agent_id: None,
                policy: Default::default(),
                permission_mode: None,
                route: None,
                workspace: None,
                voice: Some(vak_config::VoiceConfig {
                    voice_name: Some("Kore".into()),
                    persona: Some("legacy persona".into()),
                    ..Default::default()
                }),
                prompt: Default::default(),
            },
        );
        state.gateway.allowlist_approve(
            &core,
            "telegram:42",
            core.cwd().clone(),
            None,
            None,
            None,
            Default::default(),
            Some("support".into()),
            true,
            "test",
        );

        // No prompt tier yet: the legacy field still drives the voice.
        assert_eq!(
            state.gateway.resolve_persona("telegram:42").as_deref(),
            Some("legacy persona")
        );

        // Give the bot an identity block; it takes over.
        let mut bot = state.gateway.bot_get("support").unwrap();
        bot.prompt.identity = Some("You are the ACME support bot: warm and brief.".into());
        state.gateway.bot_upsert(&core, bot);
        assert_eq!(
            state.gateway.resolve_persona("telegram:42").as_deref(),
            Some("You are the ACME support bot: warm and brief.")
        );

        // The chat tier is narrower still.
        let entry = state.gateway.allowlist_get("telegram:42").unwrap();
        state.gateway.allowlist_patch(
            &core,
            "telegram:42",
            entry.workspace,
            Some(entry.agent_id),
            entry.route,
            entry.permission_mode,
            entry.policy,
            Some(entry.bot_id),
            Some(entry.inherit_bot_policy),
            Some(entry.voice),
            Some(vak_core::prompts::LayerContent {
                identity: Some("You are ACME support for this VIP chat.".into()),
                ..Default::default()
            }),
        );
        assert_eq!(
            state.gateway.resolve_persona("telegram:42").as_deref(),
            Some("You are ACME support for this VIP chat.")
        );
        // Voice *name* selection is untouched — that was never duplicated.
        assert_eq!(
            state
                .gateway
                .resolve_voice("telegram:42")
                .and_then(|v| v.voice_name)
                .as_deref(),
            Some("Kore")
        );
    }

    /// A chat binding is implicit — the operator never named the session —
    /// so an edited prompt layer rotates it rather than failing, and the old
    /// A chat binding preserves its session across prompt layer edits,
    /// refreshing capabilities dynamically without forced session rotation.
    #[tokio::test]
    async fn prompt_layer_change_preserves_binding_without_forced_rotation() {
        let dir = tempfile::tempdir().unwrap();
        let core = Core::new(dir.path().to_path_buf()).unwrap();
        core.set_sessions_home(dir.path().join("home"));
        let session = core
            .start_session_with_route(core.effective_provider(), core.effective_model())
            .await
            .unwrap();
        let old_id = session.header().unwrap().session_id.clone();
        assert!(
            !session.header().unwrap().contract.prompt_layers.is_empty(),
            "a new session must record which prompt layers it froze"
        );
        let state = AppState::new(core.clone());
        crate::register_handle(
            &state,
            old_id.clone(),
            session,
            core.cwd().clone(),
            core.clone(),
        );
        state.gateway.bind(
            &core,
            "telegram:42".into(),
            old_id.clone(),
            "route-revision".into(),
        );

        // Unchanged workspace: the same session is reused.
        let same = resolve_session(&state, &core, "telegram:42").await.unwrap();
        assert_eq!(same.id, old_id);

        // Now edit a layer under the running binding.
        let prompts_dir = core.cwd().join(".vak/prompts");
        std::fs::create_dir_all(&prompts_dir).unwrap();
        std::fs::write(prompts_dir.join("guardrails.md"), "- never touch infra/\n").unwrap();

        let fresh = resolve_session(&state, &core, "telegram:42").await.unwrap();
        assert_eq!(
            fresh.id, old_id,
            "prompt layer change preserves session without forced rotation"
        );
        let old_path = core.scope().session_file(core.cwd(), &old_id);
        assert!(old_path.is_file(), "append-only ledger remains intact");
    }

    #[tokio::test]
    async fn capability_snapshot_change_preserves_binding() {
        let dir = tempfile::tempdir().unwrap();
        let core = Core::new(dir.path().to_path_buf()).unwrap();
        core.set_sessions_home(dir.path().join("home"));
        let current = core
            .start_session_with_route("provider-a".into(), "model-a".into())
            .await
            .unwrap();
        let mut legacy_header = current.header().unwrap().clone();
        legacy_header.session_id = "legacy-session".into();
        legacy_header.contract.app_version = "0.11.35".into();
        legacy_header.contract.capabilities.clear();
        let legacy_path = vak_session::SessionPath::new_session_file(
            &core.scope().into_root(),
            core.cwd(),
            &legacy_header.session_id,
        );
        let legacy = vak_session::SessionLog::create(legacy_path.clone(), legacy_header).unwrap();
        let state = AppState::new(core.clone());
        crate::register_handle(
            &state,
            "legacy-session".into(),
            legacy,
            core.cwd().clone(),
            core.clone(),
        );
        state.gateway.bind(
            &core,
            "telegram:42".into(),
            "legacy-session".into(),
            "route-revision".into(),
        );

        let fresh = resolve_session(&state, &core, "telegram:42").await.unwrap();
        assert_eq!(fresh.id, "legacy-session");
        assert!(legacy_path.is_file(), "legacy ledger remains append-only");
    }

    // ---- Allowlist store (docs/design/34-channel-onboarding.md) -----------

    fn core_with_config(toml: &str) -> (tempfile::TempDir, Core) {
        let dir = tempfile::tempdir().unwrap();
        let cwd = dir.path().to_path_buf();
        std::fs::create_dir_all(cwd.join(".vak")).unwrap();
        std::fs::write(cwd.join(".vak/config.toml"), toml).unwrap();
        let core = Core::new_with_trust(cwd.clone(), true).unwrap();
        core.set_sessions_home(dir.path().join("home"));
        (dir, core)
    }

    #[test]
    fn allowlist_seeds_from_config_only_when_file_absent() {
        let (_dir, core) =
            core_with_config("[gateway]\nchat_allowlist = [\"telegram:1\", \"telegram:2\"]\n");
        let gw = GatewayState::load(&core, true);
        let mut entries = gw.allowlist_snapshot();
        entries.sort_by(|a, b| a.key.cmp(&b.key));
        assert_eq!(entries.len(), 2);
        assert_eq!(entries[0].key, "telegram:1");
        assert_eq!(entries[0].status, AllowlistStatus::Allowed);
        assert_eq!(entries[0].added_by, "config_import");
        assert!(allowlist_path(&core.shared_scope().into_root()).is_file());

        // Once the file exists, it is authoritative: a config change is not
        // re-imported on the next load.
        gw.allowlist_revoke(&core, "telegram:1");
        drop(gw);
        let gw2 = GatewayState::load(&core, true);
        let keys: Vec<String> = gw2
            .allowlist_snapshot()
            .into_iter()
            .map(|e| e.key)
            .collect();
        assert_eq!(keys, vec!["telegram:2"]);
    }

    #[test]
    fn allowlist_route_resolves_the_configured_agent_identity() {
        let (_dir, core) = core_with_config("[memory]\nreflection = false\n");
        std::fs::write(
            core.cwd().join(".vak/agents.json"),
            serde_json::json!([{
                "id": "support",
                "revision": 3,
                "name": "Support",
                "character": "orb",
                "personality": "calm",
                "behaviour": "helpful",
                "responsibilities": "support",
                "animation": "off",
                "voice": "default"
            }])
            .to_string(),
        )
        .unwrap();
        let gw = GatewayState::load(&core, true);
        gw.allowlist_approve(
            &core,
            "telegram:agent",
            core.cwd().clone(),
            Some("support".into()),
            None,
            None,
            Default::default(),
            None,
            true,
            "test",
        );
        let resolved = gw.core_for_entry(&core, "telegram:agent").unwrap();
        assert_eq!(
            resolved.agent_identity().map(|agent| agent.id.as_str()),
            Some("support")
        );
    }

    #[test]
    fn bot_agent_identity_resolves_when_chat_inherits_bot() {
        let (_dir, core) = core_with_config("[memory]\nreflection = false\n");
        std::fs::write(
            core.cwd().join(".vak/agents.json"),
            serde_json::json!([{
                "id": "researcher",
                "revision": 2,
                "name": "Researcher",
                "character": "orb",
                "personality": "curious",
                "behaviour": "thorough",
                "responsibilities": "research",
                "animation": "off",
                "voice": "default"
            }])
            .to_string(),
        )
        .unwrap();
        let gw = GatewayState::load(&core, true);
        gw.bot_upsert(
            &core,
            Bot {
                id: "res-bot".into(),
                surface: "telegram".into(),
                label: "Researcher Bot".into(),
                token_env: "BOT_TOKEN__RES".into(),
                agent_id: Some("researcher".into()),
                ..Default::default()
            },
        );
        gw.allowlist_approve(
            &core,
            "telegram:res-chat",
            core.cwd().clone(),
            None,
            None,
            None,
            Default::default(),
            Some("res-bot".into()),
            true,
            "test",
        );
        let resolved = gw.core_for_entry(&core, "telegram:res-chat").unwrap();
        assert_eq!(
            resolved.agent_identity().map(|agent| agent.id.as_str()),
            Some("researcher")
        );

        // Even if legacy entry was stamped with "vak", inherit_bot_policy allows the bot's agent to shine through
        gw.allowlist_patch(
            &core,
            "telegram:res-chat",
            Some(core.cwd().clone()),
            Some(Some("vak".into())),
            None,
            None,
            Default::default(),
            Some(Some("res-bot".into())),
            Some(true),
            None,
            None,
        );
        let resolved_legacy = gw.core_for_entry(&core, "telegram:res-chat").unwrap();
        assert_eq!(
            resolved_legacy
                .agent_identity()
                .map(|agent| agent.id.as_str()),
            Some("researcher")
        );

        // Explicit chat agent override takes precedence
        std::fs::write(
            core.cwd().join(".vak/agents.json"),
            serde_json::json!([
                {
                    "id": "researcher",
                    "revision": 2,
                    "name": "Researcher",
                    "character": "orb",
                    "personality": "curious",
                    "behaviour": "thorough",
                    "responsibilities": "research",
                    "animation": "off",
                    "voice": "default"
                },
                {
                    "id": "support",
                    "revision": 2,
                    "name": "Support",
                    "character": "orb",
                    "personality": "helpful",
                    "behaviour": "friendly",
                    "responsibilities": "support",
                    "animation": "off",
                    "voice": "default"
                }
            ])
            .to_string(),
        )
        .unwrap();
        gw.allowlist_patch(
            &core,
            "telegram:res-chat",
            Some(core.cwd().clone()),
            Some(Some("support".into())),
            None,
            None,
            Default::default(),
            Some(Some("res-bot".into())),
            Some(true),
            None,
            None,
        );
        let resolved_override = gw.core_for_entry(&core, "telegram:res-chat").unwrap();
        assert_eq!(
            resolved_override
                .agent_identity()
                .map(|agent| agent.id.as_str()),
            Some("support")
        );

        // Clearing back to None inherits bot's agent again
        gw.allowlist_patch(
            &core,
            "telegram:res-chat",
            Some(core.cwd().clone()),
            Some(None),
            None,
            None,
            Default::default(),
            Some(Some("res-bot".into())),
            Some(true),
            None,
            None,
        );
        let resolved_cleared = gw.core_for_entry(&core, "telegram:res-chat").unwrap();
        assert_eq!(
            resolved_cleared
                .agent_identity()
                .map(|agent| agent.id.as_str()),
            Some("researcher")
        );
    }

    #[test]
    fn paused_agent_route_fails_closed_before_core_creation() {
        let (_dir, core) = core_with_config("[memory]\nreflection = false\n");
        std::fs::write(
            core.cwd().join(".vak/agents.json"),
            serde_json::json!([{
                "id": "paused",
                "revision": 1,
                "lifecycle": "paused",
                "name": "Paused",
                "character": "orb",
                "personality": "calm",
                "behaviour": "helpful",
                "responsibilities": "support",
                "animation": "off",
                "voice": "default"
            }])
            .to_string(),
        )
        .unwrap();
        let gw = GatewayState::load(&core, true);
        gw.allowlist_approve(
            &core,
            "telegram:paused",
            core.cwd().clone(),
            Some("paused".into()),
            None,
            None,
            Default::default(),
            None,
            true,
            "test",
        );
        assert!(gw.core_for_entry(&core, "telegram:paused").is_err());
    }

    #[test]
    fn allowlist_open_flag_does_not_seed_pending_entries() {
        // An explicit (non-empty) project `chat_allowlist` always overrides
        // whatever a developer's own global config.toml might set, so this
        // stays deterministic regardless of the machine it runs on.
        let (_dir, core) = core_with_config(
            "[gateway]\nchat_allowlist = [\"testonly:1\"]\nchat_allowlist_open = true\n",
        );
        let gw = GatewayState::load(&core, true);
        assert!(gw.chat_allowlist_open());
        let entries = gw.allowlist_snapshot();
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].key, "testonly:1");
        assert_eq!(entries[0].status, AllowlistStatus::Allowed);
        assert_eq!(entries[0].added_by, "config_import");
    }

    #[test]
    fn allowlist_approve_deny_revoke_roundtrip() {
        let (_dir, core) = core_with_config("[memory]\nreflection = false\n");
        let gw = GatewayState::load(&core, true);

        // Unknown key: approve creates an allowed entry with the explicit
        // workspace visible in the record (not silently inherited).
        let approved = gw.allowlist_approve(
            &core,
            "telegram:7",
            core.cwd().clone(),
            None,
            Some(AllowlistRoute {
                provider: "anthropic".into(),
                model: "sonnet".into(),
            }),
            None,
            vak_config::ChannelPolicy::default(),
            None,
            true,
            "admin",
        );
        assert_eq!(approved.status, AllowlistStatus::Allowed);
        assert_eq!(approved.workspace.as_deref(), Some(core.cwd().as_path()));
        assert_eq!(approved.agent_id.as_deref(), Some("vak"));
        assert_eq!(approved.route.as_ref().unwrap().provider, "anthropic");

        // Persisted to disk atomically.
        let raw =
            std::fs::read_to_string(allowlist_path(&core.shared_scope().into_root())).unwrap();
        assert!(raw.contains("telegram:7"));

        // Revoke removes an allowed entry.
        assert!(gw.allowlist_revoke(&core, "telegram:7"));
        assert!(gw.allowlist_get("telegram:7").is_none());
        // Revoking a nonexistent / non-allowed entry is a no-op failure.
        assert!(!gw.allowlist_revoke(&core, "telegram:7"));

        // Deny sticks.
        let denied = gw.allowlist_deny(&core, "telegram:8", "admin");
        assert_eq!(denied.status, AllowlistStatus::Denied);
        assert!(!gw.allowlist_revoke(&core, "telegram:8"));
    }

    #[test]
    fn inbound_resolve_creates_one_pending_entry_not_duplicated() {
        // An explicit project `chat_allowlist` overrides a developer's own
        // global config.toml so the seeded starting state is deterministic.
        let (_dir, core) = core_with_config(
            "[memory]\nreflection = false\n[gateway]\nchat_allowlist = [\"testonly:0\"]\n",
        );
        let gw = GatewayState::load(&core, true);

        let first = gw.allowlist_resolve_inbound(&core, "telegram:99", "hello there", None);
        assert!(matches!(first, AllowlistDecision::NewlyPending));
        let entry = gw.allowlist_get("telegram:99").unwrap();
        assert_eq!(entry.status, AllowlistStatus::Pending);
        assert_eq!(entry.first_seen_text.as_deref(), Some("hello there"));
        let added_at = entry.added_at.clone();

        // A repeat message on the same pending key does not duplicate or
        // bump added_at.
        let second = gw.allowlist_resolve_inbound(&core, "telegram:99", "hello again", None);
        assert!(matches!(second, AllowlistDecision::StillPending));
        let entry2 = gw.allowlist_get("telegram:99").unwrap();
        assert_eq!(entry2.added_at, added_at);
        assert_eq!(entry2.first_seen_text.as_deref(), Some("hello there"));
        let pending_count = gw
            .allowlist_snapshot()
            .iter()
            .filter(|e| e.status == AllowlistStatus::Pending)
            .count();
        assert_eq!(pending_count, 1, "no duplicate pending entry created");
    }

    /// Multi-bot-per-channel (docs/design/34 Phase 5 follow-up): a chat
    /// already trusted under its legacy two-part key must not force a
    /// fresh approval the moment its bridge starts sending a bot id — the
    /// same physical chat, now seen through a bot, inherits the existing
    /// grant. And a *second* bot joining that same physical chat gets its
    /// own independent entry too, inherited from the same legacy row —
    /// this is the actual "a channel can have multiple independent bots"
    /// capability, not just the UI to configure it.
    #[test]
    fn bot_scoped_key_inherits_approval_from_already_allowed_legacy_key() {
        let (_dir, core) = core_with_config(
            "[memory]\nreflection = false\n[gateway]\nchat_allowlist = [\"telegram:8846301562\"]\n",
        );
        let gw = GatewayState::load(&core, true);
        let legacy = gw.allowlist_get("telegram:8846301562").unwrap();
        assert_eq!(legacy.status, AllowlistStatus::Allowed);

        // Seed a stale binding at the legacy key, as if a session had
        // actually been dispatched there before bots existed — this is
        // what must go away once a bot-scoped sibling takes over, so the
        // Chats list doesn't show a dead duplicate.
        gw.bind(
            &core,
            "telegram:8846301562".into(),
            "fake-session".into(),
            "rev".into(),
        );

        let decision =
            gw.allowlist_resolve_inbound(&core, "telegram:8846301562:VakBot", "hi", Some("VakBot"));
        assert!(matches!(decision, AllowlistDecision::Allowed));
        let inherited = gw.allowlist_get("telegram:8846301562:VakBot").unwrap();
        assert_eq!(inherited.status, AllowlistStatus::Allowed);
        assert_eq!(inherited.bot_id.as_deref(), Some("VakBot"));
        assert_eq!(inherited.workspace, legacy.workspace);
        // The legacy row's *allowlist entry* is untouched — it stays
        // around as the ancestor a third bot could still inherit from
        // later.
        assert_eq!(
            gw.allowlist_get("telegram:8846301562").unwrap().status,
            AllowlistStatus::Allowed
        );
        // Its *binding* is gone, though — that's the actual duplicate-row
        // fix: nothing will ever dispatch to the legacy key again.
        assert!(
            gw.bindings_snapshot()
                .iter()
                .all(|(k, _)| k != "telegram:8846301562"),
            "legacy binding must be unbound once a bot-scoped sibling exists"
        );

        // A second, independent bot on the same physical chat inherits
        // too, and gets a genuinely separate entry from the first bot's.
        let decision2 = gw.allowlist_resolve_inbound(
            &core,
            "telegram:8846301562:Vakyartha",
            "hi",
            Some("Vakyartha"),
        );
        assert!(matches!(decision2, AllowlistDecision::Allowed));
        let inherited2 = gw.allowlist_get("telegram:8846301562:Vakyartha").unwrap();
        assert_eq!(inherited2.bot_id.as_deref(), Some("Vakyartha"));
        assert_ne!(
            gw.allowlist_get("telegram:8846301562:VakBot")
                .unwrap()
                .bot_id,
            inherited2.bot_id,
            "each bot must get its own entry, not share one"
        );
    }

    /// No legacy approval to inherit means the normal pending-review path,
    /// same as any other never-seen chat.
    #[test]
    fn bot_scoped_key_starts_pending_when_no_legacy_approval_exists() {
        let (_dir, core) = core_with_config("[memory]\nreflection = false\n");
        let gw = GatewayState::load(&core, true);
        let decision =
            gw.allowlist_resolve_inbound(&core, "telegram:555:NewBot", "hi", Some("NewBot"));
        assert!(matches!(decision, AllowlistDecision::NewlyPending));
        let entry = gw.allowlist_get("telegram:555:NewBot").unwrap();
        assert_eq!(entry.status, AllowlistStatus::Pending);
    }

    #[test]
    fn legacy_key_for_strips_bot_id_and_is_none_for_shorter_keys() {
        assert_eq!(
            legacy_key_for("telegram:123:VakBot"),
            Some("telegram:123".to_string())
        );
        assert_eq!(legacy_key_for("telegram:123"), None);
        assert_eq!(legacy_key_for("telegram"), None);
    }

    #[test]
    fn inbound_resolve_dispatches_allowed_and_denied_keys() {
        let (_dir, core) = core_with_config(
            "[memory]\nreflection = false\n[gateway]\nchat_allowlist = [\"testonly:0\"]\n",
        );
        let gw = GatewayState::load(&core, true);

        // Allowed key dispatches normally.
        gw.allowlist_approve(
            &core,
            "telegram:100",
            core.cwd().clone(),
            None,
            None,
            None,
            vak_config::ChannelPolicy::default(),
            None,
            true,
            "admin",
        );
        assert!(matches!(
            gw.allowlist_resolve_inbound(&core, "telegram:100", "hi", None),
            AllowlistDecision::Allowed
        ));

        // Denied key stays rejected.
        gw.allowlist_deny(&core, "telegram:101", "admin");
        assert!(matches!(
            gw.allowlist_resolve_inbound(&core, "telegram:101", "hi", None),
            AllowlistDecision::Denied
        ));
    }

    #[test]
    fn first_seen_text_is_truncated() {
        let (_dir, core) = core_with_config("[memory]\nreflection = false\n");
        let gw = GatewayState::load(&core, true);
        let long = "x".repeat(FIRST_SEEN_TEXT_MAX_CHARS + 50);
        gw.allowlist_resolve_inbound(&core, "telegram:200", &long, None);
        let entry = gw.allowlist_get("telegram:200").unwrap();
        assert_eq!(
            entry.first_seen_text.unwrap().chars().count(),
            FIRST_SEEN_TEXT_MAX_CHARS
        );
    }

    #[tokio::test]
    async fn permission_revoke_denies_forwarded_gates_for_session() {
        let (_dir, core) = core_with_config("[memory]\nreflection = false\n");
        let gw = GatewayState::load(&core, true);
        let mut receivers = Vec::new();
        for id in ["gate-a", "gate-b", "other"] {
            let rx = gw.register_gate(id, if id == "other" { "s2" } else { "s1" });
            receivers.push((id, rx));
        }
        assert_eq!(gw.deny_pending_for_session("s1"), 2);
        let (_, first) = receivers.remove(0);
        let (_, second) = receivers.remove(0);
        assert!(!first.await.unwrap());
        assert!(!second.await.unwrap());
        assert!(gw.resolve_gate(true, Some("gate-a")).is_err());
        assert!(gw.resolve_gate(true, Some("gate-b")).is_err());
        assert_eq!(
            gw.resolve_gate(false, Some("other")).unwrap().session_id,
            "s2"
        );
    }

    #[tokio::test]
    async fn an_ambiguous_or_empty_gate_code_resolves_nothing() {
        let (_dir, core) = core_with_config("[memory]\nreflection = false\n");
        let gw = GatewayState::load(&core, true);
        let _a = gw.register_gate("018f-aaaa-a1b2c3", "s1");
        let _b = gw.register_gate("018f-bbbb-d4b2c3", "s1");
        // Both ids end in "b2c3": naming either by that tail is ambiguous.
        assert!(gw.resolve_gate(true, Some("b2c3")).is_err());
        assert!(gw.resolve_gate(true, Some("")).is_err());
        // A tail that names exactly one gate still resolves it.
        assert_eq!(
            gw.resolve_gate(true, Some("a1b2c3")).unwrap().id,
            "018f-aaaa-a1b2c3"
        );
        assert_eq!(gw.resolve_gate(true, None).unwrap().id, "018f-bbbb-d4b2c3");
    }
}
