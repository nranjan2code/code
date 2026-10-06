//! vak-config: layered configuration. Defaults < global file < project file
//! < environment. Unknown keys are ignored with a warning, never fatal.

pub mod credentials;
pub mod file_update;
pub mod finops;
pub mod paths;
pub mod scope;
pub mod spaces;
#[cfg(any(test, feature = "test-support"))]
pub mod test_scope;

pub use finops::{estimate_cost_usd, resolve_usd_per_mtok, usd_per_mtok_heuristic};

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Mutex, MutexGuard};

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash, serde::Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum PermissionMode {
    ReadOnly,
    #[default]
    WorkspaceWrite,
    FullAccess,
}

impl PermissionMode {
    pub fn deserialize_str(s: &str) -> Option<PermissionMode> {
        match s {
            "read-only" | "readonly" => Some(PermissionMode::ReadOnly),
            "workspace-write" => Some(PermissionMode::WorkspaceWrite),
            "full-access" | "fullaccess" => Some(PermissionMode::FullAccess),
            _ => None,
        }
    }

    pub fn as_str(&self) -> &'static str {
        match self {
            PermissionMode::ReadOnly => "read-only",
            PermissionMode::WorkspaceWrite => "workspace-write",
            PermissionMode::FullAccess => "full-access",
        }
    }

    /// How much this mode grants, as a total order. Deliberately spelled
    /// out rather than derived from declaration order: the security cap in
    /// `capped_by` depends on this ranking being correct, and a future
    /// reordering of the variants must not silently invert it.
    pub fn rank(&self) -> u8 {
        match self {
            PermissionMode::ReadOnly => 0,
            PermissionMode::WorkspaceWrite => 1,
            PermissionMode::FullAccess => 2,
        }
    }

    /// The least permissive of `self` and `ceiling`.
    ///
    /// This is the one place a requested permission grant is reconciled
    /// against a trust boundary: an override may match or reduce what the
    /// ceiling already allows, never exceed it. Used by the gateway so a
    /// per-channel permission override can never grant a channel more than
    /// the workspace's own configuration would give a local `vak` run.
    pub fn capped_by(self, ceiling: PermissionMode) -> PermissionMode {
        if self.rank() > ceiling.rank() {
            ceiling
        } else {
            self
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum ApprovalMode {
    #[default]
    Ask,
    ApproveSafe,
    AutoApprove,
}

impl ApprovalMode {
    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "ask" | "ask-approval" => Some(Self::Ask),
            "approve-safe" | "approve-for-me" => Some(Self::ApproveSafe),
            "auto-approve" => Some(Self::AutoApprove),
            _ => None,
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Ask => "ask",
            Self::ApproveSafe => "approve-safe",
            Self::AutoApprove => "auto-approve",
        }
    }
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default)]
pub struct Profile {
    pub model: Option<String>,
    pub provider: Option<String>,
    pub permission_mode: Option<PermissionMode>,
    pub max_turns: Option<usize>,
}

#[derive(Debug, Clone, Deserialize, Default)]
#[serde(default)]
pub struct FileConfig {
    pub provider: Option<String>,
    pub model: Option<String>,
    pub max_tokens: Option<u32>,
    pub max_turns: Option<usize>,
    pub permission_mode: Option<PermissionMode>,
    pub approval_mode: Option<ApprovalMode>,
    pub profile: Option<String>,
    pub profiles: std::collections::BTreeMap<String, Profile>,
    pub anthropic_base_url: Option<String>,
    /// AWS region used by the Bedrock Mantle endpoint (for example
    /// `ap-south-1`). A server-level VAK_BEDROCK_BASE_URL still takes
    /// precedence when explicitly configured.
    pub bedrock_region: Option<String>,
    #[serde(default)]
    pub allow: Vec<String>,
    #[serde(default)]
    pub ask: Vec<String>,
    #[serde(default)]
    pub deny: Vec<String>,
    /// `subagents` is accepted as a legacy alias so existing `vak.toml`
    /// files written before the subagent->worker rename keep working.
    #[serde(alias = "subagents")]
    pub workers: Option<bool>,
    #[serde(default)]
    pub hooks: Vec<HookConfig>,
    pub max_retries: Option<u32>,
    pub retry_base_backoff_ms: Option<u64>,
    pub request_timeout_secs: Option<u64>,
    pub run_retry_attempts: Option<u32>,
    pub run_retry_base_backoff_ms: Option<u64>,
    pub circuit_breaker_threshold: Option<u32>,
    pub circuit_breaker_cooldown_secs: Option<u64>,
    pub context_window: Option<u64>,
    #[serde(default)]
    pub mcp: McpConfig,
    #[serde(default)]
    pub capabilities: CapabilityInheritanceSettings,
    pub ui: UiSettings,
    pub stop_policy: Option<StopPolicySettings>,
    #[serde(default)]
    pub gateway: GatewaySettings,
    #[serde(default)]
    pub memory: MemorySettings,
    #[serde(default)]
    pub sandbox: SandboxSettings,
    #[serde(default)]
    pub finops: FinopsSettings,
    #[serde(default)]
    pub goal: GoalSettings,
    #[serde(default)]
    pub work: WorkSettings,
    #[serde(default)]
    pub route: RouteSettings,
    #[serde(default)]
    pub probe: ProbeSettings,
    #[serde(default)]
    pub intent: IntentSettings,
    #[serde(default)]
    pub commitment: CommitmentSettings,
    #[serde(default)]
    pub automation: AutomationSettings,
    #[serde(default)]
    pub update: UpdateSettings,
    #[serde(default)]
    pub tools: ToolsSettings,
    #[serde(default)]
    pub heartbeat: HeartbeatSettings,
    #[serde(default)]
    pub server: ServerSettings,
    #[serde(default)]
    pub plugins: PluginSettings,
    #[serde(default)]
    pub voice: Option<VoiceSettings>,
    /// Per-provider tuning knobs (docs/design/68-context-engine.md §8).
    /// NOT privileged: these only shape a provider's own request body.
    #[serde(default)]
    pub providers: ProvidersSettings,
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
#[serde(default)]
pub struct VoiceSettings {
    pub enabled: bool,
    /// `gemini`, `openai` or `local` (`vak_voice::VoiceProvider`). Unset
    /// inherits from a wider layer; unset everywhere is a configuration gap
    /// the voice routes report, never an implicit vendor.
    pub provider: Option<String>,
    /// Hosted speech-to-text model, from the operator's discovered catalogue.
    pub transcription_model: Option<String>,
    /// Hosted text-to-speech model, from the operator's discovered catalogue.
    pub synthesis_model: Option<String>,
    pub max_session_secs: u64,
    pub max_concurrent: usize,
    pub max_audio_bytes: u64,
    /// Paid voice requests per rolling minute per server process, shared by
    /// transcription, synthesis and socket utterances.
    pub max_requests_per_minute: usize,
    /// Maximum synthesis input characters per request.
    pub max_text_chars: usize,
}

impl Default for VoiceSettings {
    fn default() -> Self {
        Self {
            enabled: false,
            provider: None,
            transcription_model: None,
            synthesis_model: None,
            max_session_secs: 900,
            max_concurrent: 2,
            max_audio_bytes: 16 * 1024 * 1024,
            max_requests_per_minute: 60,
            max_text_chars: 100_000,
        }
    }
}

impl VoiceSettings {
    pub fn validate(&self) -> Result<(), String> {
        for (name, value) in [
            ("voice.provider", self.provider.as_deref()),
            (
                "voice.transcription_model",
                self.transcription_model.as_deref(),
            ),
            ("voice.synthesis_model", self.synthesis_model.as_deref()),
        ] {
            if let Some(value) = value
                && (value.trim().is_empty() || value.chars().count() > 256)
            {
                return Err(format!(
                    "{name} must be non-empty and at most 256 characters"
                ));
            }
        }
        if self.max_session_secs == 0 || self.max_session_secs > 86_400 {
            return Err("voice.max_session_secs must be between 1 and 86400".into());
        }
        if self.max_concurrent == 0 || self.max_concurrent > 64 {
            return Err("voice.max_concurrent must be between 1 and 64".into());
        }
        if self.max_audio_bytes == 0 || self.max_audio_bytes > 256 * 1024 * 1024 {
            return Err("voice.max_audio_bytes must be between 1 and 268435456".into());
        }
        if self.max_requests_per_minute == 0 || self.max_requests_per_minute > 10_000 {
            return Err("voice.max_requests_per_minute must be between 1 and 10000".into());
        }
        if self.max_text_chars == 0 || self.max_text_chars > 10_000_000 {
            return Err("voice.max_text_chars must be between 1 and 10000000".into());
        }
        Ok(())
    }
}

/// How the HTTP surface is exposed (docs/design/48-web-client.md §4.2).
///
/// PRIVILEGED, in full. Every key here either widens what the network can
/// reach or relaxes a check that exists to stop it: `bind` decides which
/// interface answers at all, `trusted_hosts` decides which `Host` headers
/// are accepted (the DNS-rebinding defence), and `web.terminal` decides
/// whether a remote caller can reach a real shell. A cloned repository
/// setting any of these would be handing itself the machine.
#[derive(Debug, Clone, Deserialize, Default)]
#[serde(default)]
pub struct ServerSettings {
    /// Interface to bind. Default `127.0.0.1`.
    pub bind: Option<String>,
    /// `Host` headers accepted besides loopback names. Exact matches only —
    /// a wildcard here is a rebinding hole with extra steps.
    pub trusted_hosts: Option<Vec<String>>,
    /// Public origin when behind a TLS-terminating proxy, e.g.
    /// `https://vak.example.com`. Enables `Secure` on the session cookie.
    pub public_url: Option<String>,
    /// Session cookie lifetime. Default 168 (one week); a public
    /// deployment should shorten it considerably.
    pub session_ttl_hours: Option<u64>,
    /// Hand a browser on THIS machine a session without asking for the
    /// token. Default true. See `ServerResolved::loopback_auto_login`.
    pub loopback_auto_login: Option<bool>,
    /// Directories the workspace picker may browse. Default: the user's
    /// home directory.
    pub workspace_roots: Option<Vec<String>>,
    #[serde(default)]
    pub web: WebSettings,
    /// Distributed event fabric configuration (vak-bus, docs/design/53).
    /// PRIVILEGED: a non-loopback NATS URL is network exposure, and the
    /// workspace secret is a credential. Stripped for untrusted projects.
    #[serde(default)]
    pub bus: BusConfig,
}

/// Distributed event bus configuration (docs/design/53-distributed-bus.md).
/// Lives inside `[server]` because a NATS endpoint is network exposure
/// (rule 34) and the workspace secret is a credential (rule 8).
#[derive(Debug, Clone, Deserialize, Default)]
#[serde(default)]
pub struct BusConfig {
    /// NATS server URL. Empty/unset = local InMemoryBus only.
    pub nats_url: Option<String>,
    /// Name of the env var holding the workspace encryption secret.
    /// The secret itself is never stored in config — only the env var name.
    pub workspace_secret_env: Option<String>,
}

/// The NATS credentials JWT, a secret: kept in the secrets chain under this
/// name (project scope when set through `/config/bus`), never in TOML.
pub const BUS_NATS_JWT_VAR: &str = "VAK_BUS_NATS_CREDENTIALS_JWT";
/// The NATS nkey seed, a secret, kept like [`BUS_NATS_JWT_VAR`].
pub const BUS_NATS_NKEY_SEED_VAR: &str = "VAK_BUS_NATS_NKEY_SEED";

#[derive(Debug, Clone, Deserialize, Default)]
#[serde(default)]
pub struct WebSettings {
    /// Serve a real PTY over the web client. Default false: a shell over
    /// HTTP is remote code execution, and unlike every other effect in this
    /// product it is not mediated by the permission engine.
    pub terminal: Option<bool>,
    /// Refuse the terminal to non-loopback hosts even when it is enabled.
    /// Default true.
    pub terminal_requires_loopback: Option<bool>,
}

/// Resolved HTTP exposure settings.
#[derive(Debug, Clone)]
pub struct ServerResolved {
    pub bind: String,
    /// Whether a loopback browser is signed in automatically.
    ///
    /// The token exists to stop OTHER local processes driving the agent.
    /// Against a process running as *you* it was never much of a boundary —
    /// that process can read the same credential store the token is pinned
    /// in. What it
    /// does protect is a machine with other human users on it, who can reach
    /// 127.0.0.1 but cannot read your files.
    ///
    /// So: on by default, because a single-user laptop is the overwhelming
    /// case and making someone hunt for a token to reach their own machine
    /// is friction with nothing on the other side of it. Turn it off on a
    /// shared box. It NEVER applies beyond loopback — a remote deployment
    /// always asks, whatever this says.
    pub loopback_auto_login: bool,
    pub trusted_hosts: Vec<String>,
    pub public_url: Option<String>,
    pub session_ttl_hours: u64,
    pub workspace_roots: Vec<std::path::PathBuf>,
    pub web_terminal: bool,
    pub web_terminal_requires_loopback: bool,
    pub bus: BusResolved,
}

/// Resolved distributed event bus configuration.
#[derive(Debug, Clone, Default)]
pub struct BusResolved {
    /// NATS server URL. None = local InMemoryBus only.
    pub nats_url: Option<String>,
    /// Resolved workspace encryption key (read from the env var named in
    /// `BusConfig.workspace_secret_env`). Never the env var name itself.
    pub workspace_secret: Option<Vec<u8>>,
}

impl ServerResolved {
    /// Whether `bind` reaches beyond this machine's loopback interface.
    pub fn binds_publicly(&self) -> bool {
        !matches!(self.bind.as_str(), "127.0.0.1" | "::1" | "localhost")
    }

    /// Cookies may only carry `Secure` when the browser actually reached us
    /// over TLS; setting it on plain http makes the browser drop the cookie
    /// and the session silently never persists.
    pub fn cookie_is_secure(&self) -> bool {
        self.public_url
            .as_deref()
            .is_some_and(|url| url.starts_with("https://"))
    }
}

/// Cross-session recall (docs/design/23-memory.md). Read-only and
/// workspace-scoped, so unlike [gateway] this section is NOT privileged.
#[derive(Debug, Clone, Deserialize, Default)]
#[serde(default)]
pub struct MemorySettings {
    /// Expose the `session_search` tool to agent runs. Default true.
    pub search_enabled: Option<bool>,
    /// `remember` tool: append durable per-workspace notes. Default true.
    pub write_enabled: Option<bool>,
    /// `propose_skill` tool: queue drafts for human promotion. Default true.
    pub skill_proposals: Option<bool>,
    /// Post-run reflection: an auxiliary model call proposes durable notes /
    /// skill drafts after clean completions, deduped against existing
    /// memory. Costs one extra request per run — default false.
    pub reflection: Option<bool>,
}

/// Execution backend selection (docs/design/25-docker-sandbox.md).
/// Privileged: an untrusted repo must not pick the image its commands run
/// in. "auto" keeps platform defaults (Seatbelt on macOS, Landlock on
/// Linux); "docker" runs bash in a throwaway no-network container.
#[derive(Debug, Clone, Deserialize, Default)]
#[serde(default)]
pub struct SandboxSettings {
    pub backend: Option<String>,
    /// Container image for the docker backend (default alpine:3.20).
    pub image: Option<String>,
}

/// Always-on gateway surfaces (docs/design/22-gateway.md). Privileged:
/// stripped from untrusted project config because enabling it allows
/// remote execution.
#[derive(Debug, Clone, Deserialize, Default)]
#[serde(default)]
pub struct GatewaySettings {
    pub enabled: Option<bool>,
    /// "deny" (default) auto-denies approval gates on unattended turns.
    /// "forward" routes them to the `approver` target for a yes/no reply.
    pub approvals: Option<String>,
    /// Routing target ("<surface>:<chat>") that answers forwarded approval
    /// gates. Required when approvals = "forward".
    pub approver: Option<String>,
    /// How long a forwarded gate waits for a reply before failing closed
    /// (default 300, minimum 5).
    pub approval_timeout_secs: Option<u64>,
    #[serde(default)]
    pub outbound: OutboundSettings,
    /// Inbound rate limiting (0a-06). None uses sensible defaults.
    pub rate_limit: Option<RateLimitSettings>,
    /// Allowed inbound chat keys: `["telegram:12345", "log:ops"]`.
    /// Empty list fails closed (0c-02): every inbound chat is rejected
    /// until either this is populated or `chat_allowlist_open` is set.
    #[serde(default)]
    pub chat_allowlist: Vec<String>,
    /// Explicit opt-out of the allowlist: any chat may reach the gateway.
    /// Only takes effect when `chat_allowlist` is empty; ignored otherwise.
    /// Default false — an operator must opt in to open access.
    pub chat_allowlist_open: Option<bool>,
    /// Process-wide cap on concurrently pooled per-workspace `Core`
    /// instances (docs/design/34-channel-onboarding.md Phase 2). Default 8.
    pub core_pool_max: Option<usize>,
    /// Idle duration (seconds) after which a pooled non-default-workspace
    /// `Core` is evicted. Default 1800 (30 minutes).
    pub core_pool_idle_secs: Option<u64>,
    /// Days a `pending` allowlist entry may sit unreviewed before
    /// `vak doctor` flags it and `--repair` auto-denies it
    /// (docs/design/34-channel-onboarding.md). Default 7.
    pub pending_expiry_days: Option<u64>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize, Serialize)]
pub struct RateLimitSettings {
    /// Socket peers allowed to supply X-Real-IP. Configure only for a
    /// reverse proxy that overwrites that header with its observed peer.
    #[serde(default)]
    pub trusted_proxy_ips: Vec<std::net::IpAddr>,
    /// Max requests per window for `POST /gateway/inbound`.
    pub inbound_per_min: Option<u32>,
    /// Max requests per window for `POST /sessions`.
    pub sessions_per_min: Option<u32>,
    /// Max requests per window for `POST /sessions/{id}/run`.
    pub runs_per_min: Option<u32>,
    /// Max requests per window for all other POST endpoints.
    pub other_post_per_min: Option<u32>,
    /// Window duration in seconds.
    pub window_secs: Option<u64>,
}

#[derive(Debug, Clone, Deserialize, Default)]
#[serde(default)]
pub struct OutboundSettings {
    /// Named webhook delivery targets: `[gateway.outbound.webhooks.<name>]`.
    /// Target strings use `webhook:<name>`.
    pub webhooks: std::collections::BTreeMap<String, WebhookTarget>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct WebhookTarget {
    pub url: String,
    /// Name of an env var holding a bearer token attached to each
    /// delivery. The value is resolved at delivery time and never stored
    /// in config; a configured-but-missing token fails the delivery
    /// closed instead of posting unauthenticated.
    pub token_env: Option<String>,
}

#[derive(Debug, Clone, Deserialize, Default)]
#[serde(default)]
pub struct UiSettings {
    pub theme: Option<String>,
    pub bell: Option<bool>,
    /// Keymap overrides: `"Ctrl-P" = "command-palette"` or
    /// `"running|Tab" = "queue"`.
    pub keymap: std::collections::BTreeMap<String, String>,
    /// `"emacs"` (default) or `"vim"`.
    pub composer: Option<String>,
    /// Opt-in OSC52 clipboard copy. Never automatic: an explicit user
    /// action (Alt-Y / `/copy`) is required even when enabled.
    pub osc52: Option<bool>,
    #[serde(default)]
    pub accessibility: Option<AccessibilitySettings>,
    /// Custom theme definitions: `[ui.themes.<name>]` with color keys
    /// (`accent`, `dim`, ...) mapped to `#rrggbb` or named colors. Held as
    /// raw TOML values so a stray non-string entry warns instead of making
    /// the whole config unparseable.
    #[serde(default)]
    pub themes: std::collections::BTreeMap<String, std::collections::BTreeMap<String, toml::Value>>,
}

#[derive(Debug, Clone, Deserialize, Default)]
pub struct AccessibilitySettings {
    pub plain: Option<bool>,
    pub reduced_motion: Option<bool>,
    pub screen_reader: Option<bool>,
}

#[derive(Debug, Clone, Deserialize, Default)]
pub struct StopPolicySettings {
    pub enabled: Option<bool>,
    pub marker_gate: Option<bool>,
    pub verify_gate: Option<bool>,
    pub max_blocks: Option<u32>,
}

/// Spend admission (docs/design/15-reliability.md). Absent prices are UNKNOWN:
/// unpriced models bypass USD math rather than guessing at zero.
#[derive(Debug, Clone, Deserialize, Default)]
#[serde(default)]
pub struct GoalSettings {
    pub handoff_reset: Option<bool>,
    pub max_audit_blocks: Option<u32>,
}

#[derive(Debug, Clone, Deserialize, Default)]
#[serde(default)]
pub struct WorkSettings {
    pub enabled: Option<bool>,
    pub default_mode: Option<String>,
    pub max_items: Option<usize>,
    pub max_revisions: Option<u32>,
    pub max_parallel: Option<usize>,
    pub confirmation: Option<String>,
}

#[derive(Debug, Clone, Deserialize, Default)]
#[serde(default)]
pub struct FinopsSettings {
    pub max_run_usd: Option<f64>,
    pub max_day_usd: Option<f64>,
    /// Exact model id → (input USD/MTok, output USD/MTok). Overrides the
    /// built-in heuristic table; estimates stay labeled as estimates.
    pub price_overrides: std::collections::BTreeMap<String, PriceEntry>,
}

/// Capacity-probe cost control (docs/design/68-context-engine.md §1 "Cost
/// control for hosted models"). Local models are always probed in full
/// regardless of this setting; it governs hosted models only.
#[derive(Debug, Clone, Deserialize, Default)]
#[serde(default)]
pub struct ProbeSettings {
    /// "none" (default) starts a hosted profile's instruction horizon at
    /// its declared window with low confidence, tightened only by
    /// feedback; "full" opts in to running the horizon ladder against
    /// hosted models too.
    pub hosted: Option<String>,
}

/// Resolved capacity-probe policy.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProbeResolved {
    pub hosted: String,
}

/// Per-provider tuning sections. One field per provider that has knobs
/// beyond credentials/base-url; a provider with nothing to tune has none.
#[derive(Debug, Clone, Deserialize, Default)]
#[serde(default)]
pub struct ProvidersSettings {
    pub ollama: OllamaSettings,
    pub anthropic: AnthropicSettings,
    pub google: GoogleSettings,
}

/// Gemini capacity identity. Multiple API keys for one Google Cloud project
/// consume the same published project quota; this non-secret label lets the
/// local scheduler share observations across those credentials.
#[derive(Debug, Clone, Deserialize, Default, PartialEq, Eq)]
#[serde(default)]
pub struct GoogleSettings {
    pub project_id: Option<String>,
}

/// Anthropic provider tuning (docs/design/68-context-engine.md §11
/// "Anthropic" row, the API's "Fast Mode" quick reference).
#[derive(Debug, Clone, Deserialize, Default, PartialEq, Eq)]
#[serde(default)]
pub struct AnthropicSettings {
    /// Opt-in fast-mode research preview: `speed: "fast"` plus the
    /// `anthropic-beta: fast-mode-2026-02-01` header, sent only for a model
    /// whose discovered capabilities confirm support. Premium pricing and a
    /// separate rate-limit bucket, so this defaults to off (`None` resolves
    /// to `false`).
    pub fast_mode: Option<bool>,
}

/// Native Ollama provider tuning (docs/design/68-context-engine.md §8): the
/// OpenAI-compatible path silently ignores both of these, so the native
/// `/api/chat` adapter needs them threaded from config.
#[derive(Debug, Clone, Deserialize, Default, PartialEq, Eq)]
#[serde(default)]
pub struct OllamaSettings {
    /// Go-style duration string ("10m", "24h", "0"). Sent on every request
    /// so the runner does not evict the model under the default 5-minute
    /// idle unload.
    pub keep_alive: Option<String>,
    /// `options.num_ctx`. Must be >= 1024 when set; omitted entirely when
    /// `None` so the server's own modelfile default applies.
    pub num_ctx: Option<u64>,
}

impl OllamaSettings {
    pub fn validate(&self) -> Result<(), String> {
        if let Some(ka) = &self.keep_alive
            && !is_go_duration(ka)
        {
            return Err(format!(
                "providers.ollama.keep_alive '{ka}' is not a valid Go-style \
                 duration (e.g. \"10m\", \"24h\", \"0\")"
            ));
        }
        if let Some(n) = self.num_ctx
            && n < 1024
        {
            return Err("providers.ollama.num_ctx must be >= 1024 when set".into());
        }
        Ok(())
    }
}

/// Minimal Go `time.ParseDuration` shape check: `"0"`, or one or more
/// `<number><unit>` pairs with no separators, units restricted to the ones
/// Ollama's own duration parsing accepts.
fn is_go_duration(s: &str) -> bool {
    let s = s.trim();
    if s == "0" {
        return true;
    }
    let mut chars = s.chars().peekable();
    let mut matched_any = false;
    while chars.peek().is_some() {
        let mut num = String::new();
        while let Some(&c) = chars.peek() {
            if c.is_ascii_digit() || c == '.' {
                num.push(c);
                chars.next();
            } else {
                break;
            }
        }
        if num.is_empty() || num == "." {
            return false;
        }
        let mut unit = String::new();
        while let Some(&c) = chars.peek() {
            if c.is_alphabetic() || c == '\u{00b5}' {
                unit.push(c);
                chars.next();
            } else {
                break;
            }
        }
        if !matches!(
            unit.as_str(),
            "ns" | "us" | "\u{00b5}s" | "ms" | "s" | "m" | "h"
        ) {
            return false;
        }
        matched_any = true;
    }
    matched_any
}

/// Frozen-ladder routing preferences (docs/design/15-reliability.md + Phase R).
/// NOT privileged: choosing how to order discovered candidates grants no
/// execution power.
#[derive(Debug, Clone, Deserialize, Default)]
#[serde(default)]
pub struct RouteSettings {
    /// "auto" (default) derives utility/balanced/quality-critical from
    /// request demand; explicit "utility" | "balanced" |
    /// "quality-critical" overrides the derivation.
    pub objective: Option<String>,
    /// Explicit cross-model fallback allowlist. Model ids here become
    /// candidate legs WHEN warm discovery shows a configured key can
    /// reach them; empty keeps the legacy same-model-only ladder.
    pub fallback_models: Vec<String>,
    /// Confirmed groups of `provider/model` ids that name one model at
    /// different services. A member stands in for the primary only through
    /// a group: one model has a different id at each service, so an id
    /// alone is never evidence across services. Groups that share a member
    /// merge; every layer's groups apply.
    pub same_model: Vec<Vec<String>>,
    /// Total ladder length cap INCLUDING the primary leg (default 4).
    pub max_fallbacks: Option<usize>,
    /// Caller-declared frontier-tier model-id substrings promoted under
    /// balanced/quality-critical objectives. Routing knowledge stays
    /// operator-supplied, never baked into source (invariant 9).
    pub quality_hints: Vec<String>,
    /// Model-id substrings that mark a leg as able to serve non-text input
    /// (images, audio, …). A vision turn is served only by matching legs;
    /// with no hints declared every leg is assumed capable, because a
    /// restriction the operator did not state is not ours to invent
    /// (docs/design/47-commitment-kernel.md, invariant 10).
    pub modality_hints: Vec<String>,
}

/// Intent kernel (docs/design/47-commitment-kernel.md).
///
/// PARTLY PRIVILEGED. Most of this section only ever narrows what a turn may
/// do, and a repository choosing to give itself fewer tools is harmless. Two
/// keys are different and are stripped for an untrusted project by
/// `load_with_trust`:
///
/// * `autonomy` — `delegated` and `autonomous` suppress approval gates the
///   agent would otherwise raise. That is execution power, and a cloned
///   repository must not be able to grant it to itself.
/// * `escalate = "cloud"` — spends the user's credentials on a classification
///   dispatch before the run they actually asked for.
#[derive(Debug, Clone, Deserialize, Default)]
#[serde(default)]
pub struct IntentSettings {
    /// Master switch. `false` resolves every turn to the general engagement,
    /// which is vak's behaviour before the kernel existed. Default true.
    pub enabled: Option<bool>,
    /// Confidence at or above which a reading may narrow capability.
    pub accept_confidence: Option<f64>,
    /// Confidence at or above which a reading may raise risk posture but not
    /// remove tools.
    pub provisional_confidence: Option<f64>,
    /// Allow progressive disclosure of the capability packet. Default true.
    pub slice_capabilities: Option<bool>,
    /// Allow stakes to raise the approval floor. Default true.
    pub posture: Option<bool>,
    /// How far the cascade may escalate: "none" | "local" | "cloud".
    pub escalate: Option<String>,
    /// Model for the classification tier: `model`, or `provider/model`.
    /// Local escalation runs it on `ollama`; cloud on the effective
    /// provider unless a provider is named. Empty uses the effective route.
    pub classify_model: Option<String>,
    /// Hard ceiling on one classification dispatch.
    pub max_classify_usd: Option<f64>,
    /// Watchdog on one classification dispatch, in seconds (default 10).
    /// A classifier that overruns it is abandoned and the free-tier reading
    /// stands; the run never waits on it.
    pub classify_timeout_secs: Option<u64>,
    /// Standing delegation: "manual" | "assisted" | "delegated" | "autonomous".
    pub autonomy: Option<String>,
    pub evidence_max_age_secs: Option<i64>,
}

/// Durable commitments (docs/design/47-commitment-kernel.md). NOT privileged:
/// every key here bounds long-running work rather than enabling it.
#[derive(Debug, Clone, Deserialize, Default)]
#[serde(default)]
pub struct CommitmentSettings {
    /// Open durable commitments for session-or-longer work. Default true.
    pub enabled: Option<bool>,
    /// Lifetime spend cap per commitment. None inherits the FinOps caps only.
    pub lifetime_budget_usd: Option<f64>,
    /// Consecutive stalled episodes before the stall breaker trips.
    pub stall_limit: Option<u32>,
    /// Surface a commitment for human review after this long untouched.
    pub review_every_hours: Option<u32>,
    /// Default relevance window. A commitment past it closes `expired`
    /// explicitly rather than lingering.
    pub default_ttl_days: Option<u32>,
}

/// Scheduled-task behavior (docs/design/29-personal-os.md P2). NOT
/// privileged: catch-up only widens when an already-configured task may run.
#[derive(Debug, Clone, Deserialize, Default)]
#[serde(default)]
pub struct AutomationSettings {
    /// Run tasks that missed their schedule while the app was shut down.
    /// Default true.
    pub catch_up_missed: Option<bool>,
}

/// Opt-in update awareness (docs/design/29-personal-os.md P3). A `None`
/// url disables update checks entirely; checks never auto-install.
#[derive(Debug, Clone, Deserialize, Default)]
#[serde(default)]
pub struct UpdateSettings {
    /// Version-manifest URL polled for update banners. Default: none
    /// (update checks fully disabled).
    pub url: Option<String>,
    /// Hours between update checks (default 24).
    pub interval_hours: Option<u64>,
}

/// Master switches for optional built-in tool registration.
#[derive(Debug, Clone, Deserialize, Default)]
#[serde(default)]
pub struct ToolsSettings {
    /// Register the bounded webfetch tool. Default true; network access is
    /// still permission-classified per request.
    pub web_fetch: Option<bool>,
    /// Register the headless-browser DOM render tool (`browse`). Default
    /// true; requires a locally installed Chromium-family browser and is
    /// still permission-classified per request.
    pub browse: Option<bool>,
}

/// Proactive heartbeat (docs/design/29-personal-os.md P7). NOT privileged:
/// it spends this server's own configured credentials on a bounded review
/// turn, never grants new execution power.
#[derive(Debug, Clone, Deserialize, Default)]
#[serde(default)]
pub struct HeartbeatSettings {
    pub enabled: Option<bool>,
    /// Seconds between review turns (default 1800, minimum 300).
    pub interval_secs: Option<u64>,
    /// Model pin ("model" or "provider/model"); default keeps the
    /// provider's current model.
    pub model: Option<String>,
    /// Local-time quiet window "HH:MM-HH:MM" during which cycles skip.
    /// Wraps midnight ("22:00-07:00").
    pub quiet_hours: Option<String>,
    /// Maximum findings reported per beat (default 3).
    pub max_findings: Option<usize>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct PriceEntry {
    pub input: f64,
    pub output: f64,
}

#[derive(Debug, Clone, Deserialize, Default)]
pub struct McpConfig {
    #[serde(default)]
    pub servers: std::collections::BTreeMap<String, McpServerConfig>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct McpServerConfig {
    pub command: String,
    #[serde(default)]
    pub args: Vec<String>,
    #[serde(default)]
    pub env: std::collections::BTreeMap<String, String>,
    /// Allow outbound network for this MCP server. Privileged (mcp.servers
    /// is stripped from untrusted projects).
    #[serde(default)]
    pub network: bool,
    /// What this server is for, in its own words: `serves = ["live-data"]`.
    ///
    /// Optional, and deliberately so. It decides whether a call to this
    /// server counts as retrieval that an answer must be grounded in; an
    /// empty list means *undeclared*, and the server inherits the `mcp`
    /// broker's web/live-data claim. It never hides the server from a turn.
    /// The alternative, guessing a domain from the server's tool names,
    /// would put a keyword table back in the harness and reintroduce the
    /// coupling this field exists to remove.
    ///
    /// Skipped when empty so the config file and the management API keep
    /// exactly the shape they had before this field existed — a server that
    /// declares nothing should look no different from one written last year.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub serves: Vec<String>,
}

/// Project-layer switches for severing one inherited capability category.
/// Missing means inherit. User-layer values are accepted but only become
/// meaningful when a narrower layer is merged over them.
#[derive(Debug, Clone, Deserialize, Default)]
#[serde(default)]
pub struct CapabilityInheritanceSettings {
    pub inherit_mcp: Option<bool>,
    pub inherit_hooks: Option<bool>,
    pub inherit_skills: Option<bool>,
    pub inherit_commands: Option<bool>,
    pub inherit_plugins: Option<bool>,
}

#[derive(Debug, Clone)]
pub struct CapabilityInheritanceResolved {
    pub inherit_mcp: bool,
    pub inherit_hooks: bool,
    pub inherit_skills: bool,
    pub inherit_commands: bool,
    pub inherit_plugins: bool,
}

/// Global and workspace settings governing capability plugins.
#[derive(Debug, Clone, Deserialize, Serialize, Default, PartialEq, Eq)]
#[serde(default)]
pub struct PluginSettings {
    /// Explicitly enabled plugin names.
    pub enabled: Vec<String>,
    /// Explicitly disabled plugin names.
    pub disabled: Vec<String>,
    /// Optional allowlist of permitted plugin names. If specified, only matching plugins may be enabled.
    pub allow: Option<Vec<String>>,
    /// Denylist of forbidden plugin names. Deny always takes precedence over allow.
    pub deny: Vec<String>,
    /// Plugins permitted outbound network access. Privileged.
    pub network_allow: Option<Vec<String>>,
    /// Plugins forbidden outbound network access. Precedence: an entry
    /// here always beats `network_allow` (channel `plugins_network_deny`
    /// layers on top of both and can only take egress away).
    pub network_deny: Vec<String>,
}

/// Resolved plugin policy across global and workspace layers.
#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq, Eq)]
pub struct PluginResolved {
    pub enabled: Vec<String>,
    pub disabled: Vec<String>,
    pub allow: Option<Vec<String>>,
    pub deny: Vec<String>,
    pub network_allow: Option<Vec<String>>,
    pub network_deny: Vec<String>,
}

impl PluginResolved {
    pub fn is_enabled(&self, name: &str) -> bool {
        // Deny always wins
        if self.deny.iter().any(|d| d == name || d == "*") {
            return false;
        }
        if self.disabled.iter().any(|d| d == name) {
            return false;
        }
        if let Some(allow) = &self.allow {
            return allow.iter().any(|a| a == name || a == "*");
        }
        if !self.enabled.is_empty() {
            return self.enabled.iter().any(|e| e == name || e == "*");
        }
        true
    }

    pub fn is_network_allowed(&self, name: &str) -> bool {
        if !self.is_enabled(name) {
            return false;
        }
        if self.network_deny.iter().any(|d| d == name || d == "*") {
            return false;
        }
        if let Some(allow) = &self.network_allow {
            return allow.iter().any(|a| a == name || a == "*");
        }
        false
    }
}

/// Restrictive capability overlay for a gateway channel. `None` means inherit
/// the workspace policy; `Some([])` means deny everything in that category.
#[derive(Debug, Clone, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(default)]
pub struct ChannelPolicy {
    pub tools_allow: Option<Vec<String>>,
    pub tools_deny: Vec<String>,
    pub mcp_allow: Option<Vec<String>>,
    pub mcp_deny: Vec<String>,
    pub skills_allow: Option<Vec<String>>,
    pub skills_deny: Vec<String>,
    pub hooks_allow: Option<Vec<String>>,
    pub hooks_deny: Vec<String>,
    /// Server-name patterns (matched the same way as `mcp_allow`/`mcp_deny`)
    /// for which this channel forces outbound network off, even when the
    /// server's own `McpServerConfig.network` is `true`. Restrictive only —
    /// there is deliberately no matching "network_allow": a channel can
    /// only take network access away from a server it can already reach,
    /// never grant it to one the server config itself denies.
    pub mcp_network_deny: Vec<String>,
    /// Plugin-name patterns for which this channel forces outbound network off.
    pub plugins_network_deny: Vec<String>,
    /// Optional allowlist of plugin names permitted on this channel.
    pub plugins_allow: Option<Vec<String>>,
    /// Denylist of plugin names forbidden on this channel.
    pub plugins_deny: Vec<String>,
    /// Autonomy ceiling for this channel (docs/design/47-commitment-kernel.md).
    ///
    /// **Restrictive only**, like everything else on this type: a channel may
    /// cap delegation below what the workspace granted, never raise it. That
    /// asymmetry is the point — a Telegram chat should be able to say "propose
    /// only, in here", and must never be able to say "act freely" on a
    /// workspace whose operator did not.
    ///
    /// `None` inherits. Values: `manual` | `assisted` | `delegated` |
    /// `autonomous`.
    pub autonomy_ceiling: Option<String>,
}

/// Rank an autonomy name, mirroring `vak_intent::Autonomy::rank`.
///
/// Duplicated rather than imported because `vak-config` deliberately does not
/// depend on the intent kernel; the ranking is asserted equal by a test in
/// `vak-core`, which sees both.
fn autonomy_rank(name: &str) -> u8 {
    match name {
        "manual" => 0,
        "assisted" => 1,
        "delegated" => 2,
        "autonomous" => 3,
        _ => 1,
    }
}

impl ChannelPolicy {
    /// The least-delegated of two autonomy ceilings. `None` on either side
    /// means "says nothing", not "allows everything".
    pub fn cap_autonomy(lower: Option<&str>, higher: Option<&str>) -> Option<String> {
        match (lower, higher) {
            (None, None) => None,
            (Some(one), None) | (None, Some(one)) => Some(one.to_string()),
            (Some(a), Some(b)) => Some(
                if autonomy_rank(b) < autonomy_rank(a) {
                    b
                } else {
                    a
                }
                .to_string(),
            ),
        }
    }

    /// Fold a lower tier (e.g. bot) and a higher tier (e.g. chat) into the
    /// single effective policy applied at dispatch. Restrictive-only: an
    /// `_allow` list from the higher tier wins outright when present (it is
    /// itself already capped against whatever it's allowed to name), a
    /// missing `_allow` falls back to the lower tier's, and `_deny` lists
    /// concatenate across tiers since denies only ever remove, never add,
    /// access. `lower` is the more permissive default (bot), `higher` is
    /// the more specific override (chat).
    pub fn merge(lower: &ChannelPolicy, higher: &ChannelPolicy) -> ChannelPolicy {
        fn merge_allow(
            lower: &Option<Vec<String>>,
            higher: &Option<Vec<String>>,
        ) -> Option<Vec<String>> {
            higher.clone().or_else(|| lower.clone())
        }
        fn merge_deny(lower: &[String], higher: &[String]) -> Vec<String> {
            let mut out = lower.to_vec();
            for item in higher {
                if !out.contains(item) {
                    out.push(item.clone());
                }
            }
            out
        }
        ChannelPolicy {
            tools_allow: merge_allow(&lower.tools_allow, &higher.tools_allow),
            tools_deny: merge_deny(&lower.tools_deny, &higher.tools_deny),
            mcp_allow: merge_allow(&lower.mcp_allow, &higher.mcp_allow),
            mcp_deny: merge_deny(&lower.mcp_deny, &higher.mcp_deny),
            skills_allow: merge_allow(&lower.skills_allow, &higher.skills_allow),
            skills_deny: merge_deny(&lower.skills_deny, &higher.skills_deny),
            hooks_allow: merge_allow(&lower.hooks_allow, &higher.hooks_allow),
            hooks_deny: merge_deny(&lower.hooks_deny, &higher.hooks_deny),
            mcp_network_deny: merge_deny(&lower.mcp_network_deny, &higher.mcp_network_deny),
            plugins_network_deny: merge_deny(
                &lower.plugins_network_deny,
                &higher.plugins_network_deny,
            ),
            plugins_allow: merge_allow(&lower.plugins_allow, &higher.plugins_allow),
            plugins_deny: merge_deny(&lower.plugins_deny, &higher.plugins_deny),
            autonomy_ceiling: Self::cap_autonomy(
                lower.autonomy_ceiling.as_deref(),
                higher.autonomy_ceiling.as_deref(),
            ),
        }
    }
}

/// Optional spoken voice + persona for a bot/chat, resolved through the
/// same bot→chat inheritance idiom as `route`/`permission_mode` (see
/// `GatewayState::core_for_entry` in vak-server::gateway). `None` on a
/// field means "no override for that piece"; the whole `VoiceConfig` being
/// `None` on the entity means "inherit the parent tier's voice entirely".
#[derive(Debug, Clone, PartialEq, Eq, Default, serde::Serialize, serde::Deserialize)]
pub struct VoiceConfig {
    /// Provider override for voice operations at this scope.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub provider: Option<String>,
    /// Live API prebuilt voice name, e.g. "Kore", "Puck", "Zephyr".
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub voice_name: Option<String>,
    /// Provider model override for transcription at this scope.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub transcription_model: Option<String>,
    /// Provider model override for synthesis at this scope.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub synthesis_model: Option<String>,
    /// **Deprecated** (docs/design/45-prompt-layers.md): the bot/chat
    /// `identity` prompt block is the persona now, so a bot's spoken and
    /// written selves cannot drift apart. Still read as a fallback when no
    /// prompt tier sets an identity, and still honoured as an explicit
    /// per-request override, so existing configs keep working. New writes
    /// should set the `identity` block instead.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub persona: Option<String>,
}

impl VoiceConfig {
    /// Overlay a narrower scope onto its parent. Missing fields inherit.
    pub fn overlay(parent: Option<&Self>, child: &Self) -> Self {
        Self {
            provider: child
                .provider
                .clone()
                .or_else(|| parent.and_then(|v| v.provider.clone())),
            voice_name: child
                .voice_name
                .clone()
                .or_else(|| parent.and_then(|v| v.voice_name.clone())),
            transcription_model: child
                .transcription_model
                .clone()
                .or_else(|| parent.and_then(|v| v.transcription_model.clone())),
            synthesis_model: child
                .synthesis_model
                .clone()
                .or_else(|| parent.and_then(|v| v.synthesis_model.clone())),
            persona: child
                .persona
                .clone()
                .or_else(|| parent.and_then(|v| v.persona.clone())),
        }
    }
}

// Note: the `Bot` entity itself (id/surface/label/token_env/policy/
// permission_mode/route/workspace) lives in `vak-server::gateway` next to
// `AllowlistEntry` and `AllowlistRoute`, since it needs `AllowlistRoute` and
// vak-config must not depend on vak-server. It reuses `ChannelPolicy::merge`
// and `PermissionMode::capped_by` from here for its slot in the bot → chat →
// workspace resolution chain.

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct HookConfig {
    pub event: String,
    #[serde(rename = "match")]
    pub matcher: Option<String>,
    pub command: String,
    pub timeout_ms: Option<u64>,
    /// Missing in an existing `config.toml` means enabled — this field was
    /// added after `[[hooks]]` shipped without one, and every hook written
    /// before it must keep firing.
    #[serde(default = "default_hook_config_enabled")]
    pub enabled: bool,
    #[serde(default)]
    pub failure_mode: Option<String>,
}

/// Add the built-in disabled automation templates to the Shared layer once.
/// Existing operator hooks are preserved byte-for-byte in the same atomic
/// rewrite used by every other persisted configuration mutation.
pub fn seed_global_hooks_if_empty(hooks: &[HookConfig]) -> Result<bool, ConfigError> {
    let path = global_path().ok_or_else(|| ConfigError::Write {
        path: PathBuf::from("<shared>"),
        source: std::io::Error::other("shared workspace unavailable"),
    })?;
    update_config_file(&path, |document| {
        if document
            .get("hooks")
            .and_then(toml::Value::as_array)
            .is_some_and(|existing| !existing.is_empty())
        {
            return Ok(false);
        }
        document.insert("hooks".into(), encode_hooks(&path, hooks)?);
        Ok(true)
    })
}

/// Replace the `[[hooks]]` list in the configuration file at `path`,
/// leaving every other key alone. A disabled hook is written with
/// `enabled = false`, never dropped (invariant 21).
pub fn persist_hooks(path: &Path, hooks: &[HookConfig]) -> Result<(), ConfigError> {
    update_config_file(path, |document| {
        document.insert("hooks".into(), encode_hooks(path, hooks)?);
        Ok(())
    })
}

fn encode_hooks(path: &Path, hooks: &[HookConfig]) -> Result<toml::Value, ConfigError> {
    hooks
        .iter()
        .map(toml::Value::try_from)
        .collect::<Result<Vec<_>, _>>()
        .map(toml::Value::Array)
        .map_err(|error| ConfigError::Write {
            path: path.to_path_buf(),
            source: std::io::Error::other(error.to_string()),
        })
}

/// Seed the default execution plugin policy into the given config file's
/// `[plugins] network_allow` table if unconfigured.
pub fn seed_plugins_network_allow_if_empty(path: &Path) -> Result<bool, ConfigError> {
    update_config_file(path, |document| {
        let plugins = child_table(document, "plugins", path)?;
        if plugins.contains_key("network_allow") {
            return Ok(false);
        }
        plugins.insert("network_allow".into(), toml::Value::Array(Vec::new()));
        Ok(true)
    })
}

/// Seed the Shared layer's `[plugins] network_allow` table once if unconfigured.
pub fn seed_global_plugins_network_allow_if_empty() -> Result<bool, ConfigError> {
    let path = global_path().ok_or_else(|| ConfigError::Write {
        path: PathBuf::from("<shared>"),
        source: std::io::Error::other("shared workspace unavailable"),
    })?;
    seed_plugins_network_allow_if_empty(&path)
}

/// Remove a plugin name from `[plugins] network_allow` in the config at `path`.
/// Called during retired-plugin cleanup so the allowlist stays consistent
/// with the on-disk plugin store. Silently succeeds if the entry, or the
/// file, was not present.
pub fn prune_plugins_network_allow(path: &Path, plugin_name: &str) -> Result<bool, ConfigError> {
    update_config_file(path, |document| {
        let Some(allow) = document
            .get_mut("plugins")
            .and_then(toml::Value::as_table_mut)
            .and_then(|plugins| plugins.get_mut("network_allow"))
            .and_then(toml::Value::as_array_mut)
        else {
            return Ok(false);
        };
        let before = allow.len();
        allow.retain(|name| name.as_str() != Some(plugin_name));
        Ok(allow.len() != before)
    })
}

fn default_hook_config_enabled() -> bool {
    true
}

#[derive(Debug, Clone)]
pub struct Config {
    pub provider: String,
    pub model: String,
    pub max_tokens: u32,
    pub max_turns: usize,
    pub permission_mode: PermissionMode,
    pub approval_mode: ApprovalMode,
    pub anthropic_base_url: Option<String>,
    pub bedrock_region: String,
    pub allow: Vec<String>,
    pub ask: Vec<String>,
    pub deny: Vec<String>,
    pub workers: bool,
    pub hooks: Vec<HookConfig>,
    pub max_retries: u32,
    pub retry_base_backoff_ms: u64,
    pub request_timeout_secs: u64,
    pub run_retry_attempts: u32,
    pub run_retry_base_backoff_ms: u64,
    pub circuit_breaker_threshold: u32,
    pub circuit_breaker_cooldown_secs: u64,
    pub context_window: u64,
    pub mcp: McpConfig,
    pub capabilities: CapabilityInheritanceResolved,
    pub ui: UiResolved,
    pub stop_policy: StopPolicyResolved,
    pub gateway: GatewayResolved,
    pub memory: MemoryResolved,
    pub sandbox: SandboxResolved,
    pub finops: FinopsResolved,
    pub goal: GoalResolved,
    pub work: WorkResolved,
    pub route: RouteResolved,
    pub probe: ProbeResolved,
    pub intent: IntentResolved,
    pub commitment: CommitmentResolved,
    pub automation: AutomationResolved,
    pub update: UpdateResolved,
    pub tools: ToolsResolved,
    pub heartbeat: HeartbeatResolved,
    pub server: ServerResolved,
    pub plugins: PluginResolved,
    pub voice: VoiceSettings,
    pub ollama: OllamaResolved,
    pub anthropic: AnthropicResolved,
    pub google_project_id: Option<String>,
    pub warnings: Vec<String>,
}

/// Resolved heartbeat policy (docs/design/29-personal-os.md P7).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HeartbeatResolved {
    pub enabled: bool,
    pub interval_secs: u64,
    /// Model pin; None keeps the provider's current model.
    pub model: Option<String>,
    /// Parsed quiet window; None means cycles may fire any time.
    pub quiet_hours: Option<QuietWindow>,
    pub max_findings: usize,
}

/// Local-time quiet window parsed from "HH:MM-HH:MM". `start_min` is
/// inclusive, `end_min` exclusive, and the window may wrap midnight.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct QuietWindow {
    pub start_min: u32,
    pub end_min: u32,
}

impl QuietWindow {
    /// Parses "HH:MM-HH:MM". A zero-length window is rejected as
    /// meaningless rather than treated as empty or full-day.
    pub fn parse(s: &str) -> Option<Self> {
        let (a, b) = s.split_once('-')?;
        let start_min = parse_hhmm(a.trim())?;
        let end_min = parse_hhmm(b.trim())?;
        if start_min == end_min {
            return None;
        }
        Some(QuietWindow { start_min, end_min })
    }

    /// True when `minutes_from_midnight` falls inside the window. The
    /// start bound is inclusive, the end exclusive.
    pub fn contains(&self, minutes_from_midnight: u32) -> bool {
        let t = minutes_from_midnight % (24 * 60);
        if self.start_min < self.end_min {
            t >= self.start_min && t < self.end_min
        } else {
            t >= self.start_min || t < self.end_min
        }
    }
}

fn parse_hhmm(s: &str) -> Option<u32> {
    let (h, m) = s.split_once(':')?;
    let h: u32 = h.trim().parse().ok()?;
    let m: u32 = m.trim().parse().ok()?;
    if h > 23 || m > 59 {
        return None;
    }
    Some(h * 60 + m)
}

/// Resolved goal-mode policy (docs/design/42-managed-work-contracts.md).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GoalResolved {
    /// Reset-with-handoff rescue on still-over contexts.
    pub handoff_reset: bool,
    /// Audit blocks per goal before degrading to Unverified.
    pub max_audit_blocks: u32,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WorkResolved {
    pub enabled: bool,
    pub default_mode: String,
    pub max_items: usize,
    pub max_revisions: u32,
    pub max_parallel: usize,
    pub confirmation: String,
}

/// Resolved spend-admission policy (docs/design/15-reliability.md).
#[derive(Debug, Clone, Default, PartialEq)]
pub struct FinopsResolved {
    pub max_run_usd: Option<f64>,
    pub max_day_usd: Option<f64>,
    pub price_overrides: std::collections::BTreeMap<String, PriceEntry>,
}

/// Resolved routing policy (Phase R).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RouteResolved {
    /// "auto" | "utility" | "balanced" | "quality-critical".
    pub objective: String,
    /// Cross-model fallback allowlist (exact model ids).
    pub fallback_models: Vec<String>,
    /// Confirmed same-model groups, merged across layers so no member
    /// appears in two groups.
    pub same_model: Vec<Vec<vak_llm::ModelRef>>,
    /// Total ladder length cap including the primary leg.
    pub max_fallbacks: usize,
    /// Frontier-tier model-id substrings (lowercased for matching).
    pub quality_hints: Vec<String>,
    /// Model-id substrings that can serve non-text modalities (lowercased).
    pub modality_hints: Vec<String>,
}

/// Resolved intent-kernel policy.
#[derive(Debug, Clone, PartialEq)]
pub struct IntentResolved {
    pub enabled: bool,
    pub accept_confidence: f64,
    pub provisional_confidence: f64,
    pub slice_capabilities: bool,
    pub posture: bool,
    /// "none" | "local" | "cloud".
    pub escalate: String,
    pub classify_model: Option<String>,
    pub max_classify_usd: f64,
    pub classify_timeout_secs: u64,
    /// Standing delegation for this workspace.
    pub autonomy: String,
    pub evidence_max_age_secs: i64,
}

/// Resolved durable-commitment policy.
#[derive(Debug, Clone, PartialEq)]
pub struct CommitmentResolved {
    pub enabled: bool,
    pub lifetime_budget_usd: Option<f64>,
    pub stall_limit: u32,
    pub review_every_hours: Option<u32>,
    pub default_ttl_days: Option<u32>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UiResolved {
    /// Built-in theme name, or any name defined in `ui.themes`; unknown
    /// values normalize to "dark".
    pub theme: String,
    pub bell: bool,
    /// Keymap overrides merged project-over-user.
    pub keymap: std::collections::BTreeMap<String, String>,
    pub composer: String,
    pub osc52: bool,
    pub accessibility: AccessibilityResolved,
    /// Custom theme definitions passed through to the UI layer.
    pub themes: std::collections::BTreeMap<String, std::collections::BTreeMap<String, String>>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AccessibilityResolved {
    pub plain: bool,
    pub reduced_motion: bool,
    pub screen_reader: bool,
}

/// Built-in premature-completion gate. On by default; conservative.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StopPolicyResolved {
    pub enabled: bool,
    pub marker_gate: bool,
    pub verify_gate: bool,
    pub max_blocks: u32,
}

/// Resolved gateway policy (docs/design/22-gateway.md).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GatewayResolved {
    pub enabled: bool,
    pub approvals: String,
    pub approver: Option<String>,
    pub approval_timeout_secs: u64,
    pub webhooks: std::collections::BTreeMap<String, WebhookResolved>,
    pub rate_limit: Option<RateLimitSettings>,
    /// Allowed inbound chat keys. Empty fails closed unless `chat_allowlist_open`.
    pub chat_allowlist: Vec<String>,
    /// Empty `chat_allowlist` was explicitly opted into staying open.
    pub chat_allowlist_open: bool,
    /// Process-wide cap on concurrently pooled per-workspace `Core`
    /// instances (docs/design/34 Phase 2). Default 8.
    pub core_pool_max: usize,
    /// Idle duration after which a pooled non-default-workspace `Core` is
    /// evicted. Default 1800s (30 minutes).
    pub core_pool_idle_secs: u64,
    /// Days a `pending` allowlist entry may sit unreviewed before it is
    /// flagged by doctor and auto-denied by `--repair`. Default 7.
    pub pending_expiry_days: u64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SandboxResolved {
    pub backend: String,
    pub image: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WebhookResolved {
    pub url: String,
    pub token_env: Option<String>,
}

/// Resolved memory policy (docs/design/23-memory.md).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MemoryResolved {
    pub search_enabled: bool,
    pub write_enabled: bool,
    pub skill_proposals: bool,
    pub reflection: bool,
}

/// Resolved scheduled-task behavior (docs/design/29-personal-os.md P2).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AutomationResolved {
    pub catch_up_missed: bool,
}

/// Resolved update-check policy (docs/design/29-personal-os.md P3).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UpdateResolved {
    pub url: Option<String>,
    pub interval_hours: u64,
}

/// Resolved optional-tool registration policy.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ToolsResolved {
    pub web_fetch: bool,
    pub browse: bool,
}

/// Resolved native-Ollama tuning (docs/design/68-context-engine.md §8).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OllamaResolved {
    pub keep_alive: String,
    pub num_ctx: Option<u64>,
}

/// Resolved Anthropic tuning (docs/design/68-context-engine.md §11).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AnthropicResolved {
    pub fast_mode: bool,
}

impl Default for Config {
    fn default() -> Self {
        Config {
            // A fresh install has no provider route until the operator
            // chooses a provider and a model together.
            provider: String::new(),
            model: String::new(),
            max_tokens: 8192,
            max_turns: 40,
            permission_mode: PermissionMode::WorkspaceWrite,
            approval_mode: ApprovalMode::Ask,
            anthropic_base_url: None,
            bedrock_region: "us-east-1".into(),
            allow: Vec::new(),
            ask: Vec::new(),
            deny: Vec::new(),
            workers: true,
            hooks: Vec::new(),
            max_retries: 3,
            retry_base_backoff_ms: 500,
            request_timeout_secs: 600,
            run_retry_attempts: 6,
            run_retry_base_backoff_ms: 2_000,
            circuit_breaker_threshold: 5,
            circuit_breaker_cooldown_secs: 60,
            context_window: 128_000,
            mcp: McpConfig::default(),
            capabilities: CapabilityInheritanceResolved {
                inherit_mcp: true,
                inherit_hooks: true,
                inherit_skills: true,
                inherit_commands: true,
                inherit_plugins: true,
            },
            ui: UiResolved {
                theme: "dark".into(),
                bell: true,
                keymap: std::collections::BTreeMap::new(),
                composer: "emacs".into(),
                osc52: false,
                accessibility: AccessibilityResolved {
                    plain: false,
                    reduced_motion: false,
                    screen_reader: false,
                },
                themes: std::collections::BTreeMap::new(),
            },
            stop_policy: StopPolicyResolved {
                enabled: true,
                marker_gate: true,
                verify_gate: true,
                max_blocks: 2,
            },
            finops: FinopsResolved::default(),
            goal: GoalResolved {
                handoff_reset: true,
                max_audit_blocks: 2,
            },
            work: WorkResolved {
                enabled: true,
                default_mode: "direct".into(),
                max_items: 20,
                max_revisions: 8,
                max_parallel: 4,
                confirmation: "risk-based".into(),
            },
            route: RouteResolved {
                objective: "auto".into(),
                fallback_models: Vec::new(),
                same_model: Vec::new(),
                max_fallbacks: 4,
                quality_hints: Vec::new(),
                modality_hints: Vec::new(),
            },
            probe: ProbeResolved {
                hosted: "none".into(),
            },
            intent: IntentResolved {
                enabled: true,
                accept_confidence: 0.75,
                provisional_confidence: 0.45,
                slice_capabilities: true,
                posture: true,
                // Deterministic tiers only by default. A paid classification
                // before the run the user actually asked for is a real cost
                // and a real latency, so it is opt-in.
                escalate: "none".into(),
                classify_model: None,
                max_classify_usd: 0.01,
                classify_timeout_secs: 10,
                autonomy: "assisted".into(),
                evidence_max_age_secs: 86_400,
            },
            commitment: CommitmentResolved {
                enabled: true,
                lifetime_budget_usd: None,
                stall_limit: 3,
                review_every_hours: None,
                default_ttl_days: None,
            },
            automation: AutomationResolved {
                catch_up_missed: true,
            },
            update: UpdateResolved {
                url: None,
                interval_hours: 24,
            },
            tools: ToolsResolved {
                web_fetch: true,
                browse: true,
            },
            heartbeat: HeartbeatResolved {
                enabled: false,
                interval_secs: 1800,
                model: None,
                quiet_hours: None,
                max_findings: 3,
            },
            gateway: GatewayResolved {
                enabled: false,
                approvals: "deny".into(),
                approver: None,
                approval_timeout_secs: 300,
                webhooks: std::collections::BTreeMap::new(),
                rate_limit: None,
                chat_allowlist: Vec::new(),
                chat_allowlist_open: false,
                core_pool_max: 8,
                core_pool_idle_secs: 1800,
                pending_expiry_days: 7,
            },
            memory: MemoryResolved {
                search_enabled: true,
                write_enabled: true,
                skill_proposals: true,
                reflection: false,
            },
            sandbox: SandboxResolved {
                backend: "auto".into(),
                image: None,
            },
            server: ServerResolved {
                // Loopback, no trusted hosts, no terminal over the web: a
                // default install is reachable only from the machine it runs
                // on, and every step away from that is deliberate.
                bind: "127.0.0.1".into(),
                trusted_hosts: Vec::new(),
                public_url: None,
                session_ttl_hours: 168,
                loopback_auto_login: true,
                workspace_roots: Vec::new(),
                web_terminal: false,
                web_terminal_requires_loopback: true,
                bus: BusResolved::default(),
            },
            plugins: PluginResolved::default(),
            voice: VoiceSettings::default(),
            ollama: OllamaResolved {
                keep_alive: "30m".into(),
                num_ctx: None,
            },
            anthropic: AnthropicResolved { fast_mode: false },
            google_project_id: None,
            warnings: Vec::new(),
        }
    }
}

#[derive(Debug, thiserror::Error)]
pub enum ConfigError {
    #[error("cannot read config file {path}: {source}")]
    Read {
        path: PathBuf,
        source: std::io::Error,
    },
    #[error("cannot parse config file {path}: {source}")]
    Parse {
        path: PathBuf,
        source: toml::de::Error,
    },
    #[error("cannot write config file {path}: {source}")]
    Write {
        path: PathBuf,
        source: std::io::Error,
    },
}

/// The topmost editable configuration layer. `vak-home` is deliberately a
/// normal workspace people can inspect, and every other workspace inherits
/// this file without copying it.
pub fn global_path() -> Option<PathBuf> {
    Some(crate::scope::WorkspaceScope::new(crate::paths::default_workspace()).config_file())
}

pub fn project_path(cwd: &Path) -> PathBuf {
    crate::scope::WorkspaceScope::new(cwd).config_file()
}

/// Cheap, stat-only "has anything changed" signal for the global + project
/// config layers. Callers that hold a resolved value derived from these
/// files (e.g. `Core`'s cached `RouteSelection`) can compare fingerprints on
/// every access instead of re-parsing TOML, and only pay for a full re-load
/// when this value actually moves (docs/design/44-shared-config.md,
/// "Liveness").
pub fn config_fingerprint(cwd: &Path) -> u64 {
    let mut hash = 0xcbf29ce484222325_u64;
    for path in [global_path(), Some(project_path(cwd))]
        .into_iter()
        .flatten()
    {
        let (mtime_nanos, len) = std::fs::metadata(&path)
            .and_then(|m| m.modified().map(|t| (t, m.len())))
            .map(|(t, len)| {
                let nanos = t
                    .duration_since(std::time::UNIX_EPOCH)
                    .map(|d| d.as_nanos() as u64)
                    .unwrap_or(0);
                (nanos, len)
            })
            .unwrap_or((0, 0));
        for byte in mtime_nanos
            .to_le_bytes()
            .into_iter()
            .chain(len.to_le_bytes())
        {
            hash ^= u64::from(byte);
            hash = hash.wrapping_mul(0x100000001b3);
        }
    }
    hash
}

/// Initialize the project layer used by interactive clients.
///
/// The file intentionally contains no copied global values. An empty project
/// layer inherits the user's global configuration through [`load_with_trust`],
/// so later changes to shared defaults reach projects that have not opted into
/// a local override. `create_new` also keeps two desktop launches from
/// overwriting a project config created by the other launch.
pub fn ensure_project_config(cwd: &Path) -> Result<PathBuf, ConfigError> {
    let dir = crate::scope::WorkspaceScope::new(cwd).project_dir();
    std::fs::create_dir_all(&dir).map_err(|source| ConfigError::Write {
        path: dir.clone(),
        source,
    })?;
    let path = project_path(cwd);
    let _guard = lock_config_files();
    match std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&path)
    {
        Ok(mut file) => {
            use std::io::Write;
            file.write_all(
                b"# Project-local overrides. Unset values inherit from the user config.\n",
            )
            .map_err(|source| ConfigError::Write {
                path: path.clone(),
                source,
            })?;
        }
        Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {}
        Err(source) => {
            return Err(ConfigError::Write {
                path: path.clone(),
                source,
            });
        }
    }
    Ok(path)
}

/// Atomically replace the set of MCP servers at one explicit configuration
/// scope. The caller selects either [`global_path`] or [`project_path`]; no
/// values are inferred from the process directory. A server missing from
/// `servers` is removed; a server that stays keeps every key the management
/// API does not own, and every other key in the file is preserved.
pub fn persist_mcp_servers(
    path: &Path,
    servers: &std::collections::BTreeMap<String, McpServerConfig>,
) -> Result<(), ConfigError> {
    update_config_file(path, |document| {
        let mcp = child_table(document, "mcp", path)?;
        let mut existing = match mcp.remove("servers") {
            Some(toml::Value::Table(existing)) => existing,
            _ => toml::Table::new(),
        };
        let entries = servers
            .iter()
            .map(|(name, server)| {
                let mut entry = match existing.remove(name) {
                    Some(toml::Value::Table(entry)) => entry,
                    _ => toml::Table::new(),
                };
                write_mcp_server(&mut entry, server);
                (name.clone(), toml::Value::Table(entry))
            })
            .collect();
        mcp.insert("servers".into(), toml::Value::Table(entries));
        Ok(())
    })
}

/// Add, replace (`Some`) or remove (`None`) the one MCP server `name` at one
/// explicit configuration scope, leaving every other server as the file has
/// it. The read happens under the same lock as the write, so a change made
/// to another server in the meantime is not lost to a stale copy.
pub fn persist_mcp_server(
    path: &Path,
    name: &str,
    server: Option<&McpServerConfig>,
) -> Result<(), ConfigError> {
    update_config_file(path, |document| {
        match server {
            Some(server) => {
                let servers = child_table(child_table(document, "mcp", path)?, "servers", path)?;
                write_mcp_server(child_table(servers, name, path)?, server);
            }
            None => {
                if let Some(servers) = document
                    .get_mut("mcp")
                    .and_then(toml::Value::as_table_mut)
                    .and_then(|mcp| mcp.get_mut("servers"))
                    .and_then(toml::Value::as_table_mut)
                {
                    servers.remove(name);
                }
            }
        }
        Ok(())
    })
}

/// Write the keys the management API owns onto one `[mcp.servers.<name>]`
/// entry. Any other key there — `serves`, which no API sets, or one this
/// version does not know — stays as the file had it; `serves` is written
/// only when the caller declares it.
fn write_mcp_server(entry: &mut toml::Table, server: &McpServerConfig) {
    let strings = |values: &[String]| {
        toml::Value::Array(values.iter().cloned().map(toml::Value::String).collect())
    };
    entry.insert(
        "command".into(),
        toml::Value::String(server.command.clone()),
    );
    entry.insert("args".into(), strings(&server.args));
    if server.env.is_empty() {
        entry.remove("env");
    } else {
        entry.insert(
            "env".into(),
            toml::Value::Table(
                server
                    .env
                    .iter()
                    .map(|(key, value)| (key.clone(), toml::Value::String(value.clone())))
                    .collect(),
            ),
        );
    }
    if server.network {
        entry.insert("network".into(), toml::Value::Boolean(true));
    } else {
        entry.remove("network");
    }
    if !server.serves.is_empty() {
        entry.insert("serves".into(), strings(&server.serves));
    }
}

/// Persist project capability inheritance switches without materializing any
/// inherited definitions into the project file.
pub fn persist_capability_inheritance(
    path: &Path,
    inherit_mcp: Option<bool>,
    inherit_hooks: Option<bool>,
    inherit_skills: Option<bool>,
    inherit_commands: Option<bool>,
    inherit_plugins: Option<bool>,
) -> Result<(), ConfigError> {
    update_config_file(path, |document| {
        let capabilities = child_table(document, "capabilities", path)?;
        for (name, value) in [
            ("inherit_mcp", inherit_mcp),
            ("inherit_hooks", inherit_hooks),
            ("inherit_skills", inherit_skills),
            ("inherit_commands", inherit_commands),
            ("inherit_plugins", inherit_plugins),
        ] {
            if let Some(value) = value {
                capabilities.insert(name.into(), toml::Value::Boolean(value));
            }
        }
        Ok(())
    })
}

/// Persist user-selected agent preferences without disturbing unrelated
/// project configuration. The write is atomic so every client sees either
/// the old or the new complete document, never a partial TOML file.
pub fn persist_project_preferences(
    cwd: &Path,
    provider: Option<&str>,
    model: Option<&str>,
    max_turns: Option<usize>,
    permission_mode: Option<PermissionMode>,
    approval_mode: Option<ApprovalMode>,
    theme: Option<&str>,
) -> Result<(), ConfigError> {
    persist_preferences_at(
        project_path(cwd),
        provider,
        model,
        max_turns,
        permission_mode,
        approval_mode,
        theme,
    )
}

/// Persist user-level defaults. Project configurations inherit these values
/// through [`load_with_trust`] until they set their own scoped override.
pub fn persist_global_preferences(
    provider: Option<&str>,
    model: Option<&str>,
    max_turns: Option<usize>,
    permission_mode: Option<PermissionMode>,
    approval_mode: Option<ApprovalMode>,
    theme: Option<&str>,
) -> Result<(), ConfigError> {
    let path = global_path().ok_or_else(|| ConfigError::Write {
        path: PathBuf::from("<user-config>"),
        source: std::io::Error::other("user home is unavailable"),
    })?;
    persist_preferences_at(
        path,
        provider,
        model,
        max_turns,
        permission_mode,
        approval_mode,
        theme,
    )
}

/// Persist preferences to an explicitly chosen layer file.
///
/// The project and global wrappers above cover the two named scopes; this
/// takes the path directly, for a caller that has already resolved which
/// layer it means (`vak config set-mode --scope`).
pub fn persist_preferences_to(
    path: PathBuf,
    provider: Option<&str>,
    model: Option<&str>,
    max_turns: Option<usize>,
    permission_mode: Option<PermissionMode>,
    approval_mode: Option<ApprovalMode>,
    theme: Option<&str>,
) -> Result<(), ConfigError> {
    persist_preferences_at(
        path,
        provider,
        model,
        max_turns,
        permission_mode,
        approval_mode,
        theme,
    )
}

/// Persist the Bedrock region for one configuration layer. The region is
/// configuration, not a credential; provider keys remain in the OS secret
/// store through `Core::set_provider_key`.
pub fn persist_bedrock_region(path: PathBuf, region: &str) -> Result<(), ConfigError> {
    update_config_file(&path, |document| {
        document.insert(
            "bedrock_region".into(),
            toml::Value::String(region.trim().into()),
        );
        Ok(())
    })
}

fn persist_preferences_at(
    path: PathBuf,
    provider: Option<&str>,
    model: Option<&str>,
    max_turns: Option<usize>,
    permission_mode: Option<PermissionMode>,
    approval_mode: Option<ApprovalMode>,
    theme: Option<&str>,
) -> Result<(), ConfigError> {
    update_config_file(&path, |document| {
        if let Some(value) = provider {
            document.insert("provider".into(), toml::Value::String(value.into()));
        }
        if let Some(value) = model {
            document.insert("model".into(), toml::Value::String(value.into()));
        }
        if let Some(value) = max_turns {
            document.insert("max_turns".into(), toml::Value::Integer(value as i64));
        }
        if let Some(value) = permission_mode {
            document.insert(
                "permission_mode".into(),
                toml::Value::String(value.as_str().into()),
            );
        }
        if let Some(value) = approval_mode {
            document.insert(
                "approval_mode".into(),
                toml::Value::String(value.as_str().into()),
            );
        }
        if let Some(value) = theme {
            child_table(document, "ui", &path)?
                .insert("theme".into(), toml::Value::String(value.into()));
        }
        Ok(())
    })
}

/// Persist the evidence freshness policy in exactly one configuration layer.
pub fn persist_evidence_max_age(path: PathBuf, seconds: i64) -> Result<(), ConfigError> {
    update_config_file(&path, |document| {
        child_table(document, "intent", &path)?.insert(
            "evidence_max_age_secs".into(),
            toml::Value::Integer(seconds.max(0)),
        );
        Ok(())
    })
}

/// Replace one layer's `[route]` backup lists. `Some(list)` replaces that
/// layer's own list and an empty one removes the key; `None` leaves it
/// alone. Other layers are untouched, so what a wider layer allows still
/// applies (lists are unions across layers).
pub fn persist_route_backups_at(
    path: PathBuf,
    same_model: Option<&[Vec<vak_llm::ModelRef>]>,
    fallback_models: Option<&[String]>,
) -> Result<(), ConfigError> {
    update_config_file(&path, |document| {
        let route = child_table(document, "route", &path)?;
        if let Some(groups) = same_model {
            if groups.is_empty() {
                route.remove("same_model");
            } else {
                let groups = groups
                    .iter()
                    .map(|group| {
                        toml::Value::Array(
                            group
                                .iter()
                                .map(|member| toml::Value::String(member.spelling()))
                                .collect(),
                        )
                    })
                    .collect();
                route.insert("same_model".into(), toml::Value::Array(groups));
            }
        }
        if let Some(models) = fallback_models {
            if models.is_empty() {
                route.remove("fallback_models");
            } else {
                route.insert(
                    "fallback_models".into(),
                    toml::Value::Array(models.iter().cloned().map(toml::Value::String).collect()),
                );
            }
        }
        Ok(())
    })
}

/// Changes the project's `[server.bus]` settings (the NATS URL and the name
/// of the workspace-secret variable), leaving the rest of the file as it
/// was: `None` leaves a key alone, `Some(None)` or an empty value removes
/// it, `Some(Some(value))` sets it. The bus's secrets never go here.
pub fn persist_bus_settings(
    path: PathBuf,
    nats_url: Option<Option<&str>>,
    workspace_secret_env: Option<Option<&str>>,
) -> Result<(), ConfigError> {
    update_config_file(&path, |document| {
        let bus = child_table(child_table(document, "server", &path)?, "bus", &path)?;
        for (key, change) in [
            ("nats_url", nats_url),
            ("workspace_secret_env", workspace_secret_env),
        ] {
            let Some(value) = change else {
                continue;
            };
            match value.map(str::trim).filter(|value| !value.is_empty()) {
                Some(value) => {
                    bus.insert(key.into(), toml::Value::String(value.to_string()));
                }
                None => {
                    bus.remove(key);
                }
            }
        }
        Ok(())
    })
}

/// The Shared layer owns the HTTP listener's public address. A workspace
/// cannot change the server process that also serves other workspaces.
/// Read only that layer so an admin GET can safely seed its matching PUT.
pub fn global_server_settings() -> Result<ServerSettings, ConfigError> {
    let path = global_path().ok_or_else(|| ConfigError::Read {
        path: PathBuf::from("<user-config>"),
        source: std::io::Error::other("user home is unavailable"),
    })?;
    if !path.is_file() {
        return Ok(ServerSettings::default());
    }
    parse_file(&path).map(|(config, _)| config.server)
}

/// Save the public web address in the Shared layer atomically. The caller
/// validates URLs and hostnames before writing; this function preserves all
/// unrelated server fields, including the event bus and terminal policy.
pub fn persist_global_web_address(
    public_url: Option<&str>,
    trusted_hosts: &[String],
    session_ttl_hours: u64,
) -> Result<(), ConfigError> {
    let path = global_path().ok_or_else(|| ConfigError::Write {
        path: PathBuf::from("<user-config>"),
        source: std::io::Error::other("user home is unavailable"),
    })?;
    update_config_file(&path, |document| {
        let server = child_table(document, "server", &path)?;
        match public_url {
            Some(url) => {
                server.insert("public_url".into(), toml::Value::String(url.to_owned()));
            }
            None => {
                server.remove("public_url");
            }
        }
        server.insert(
            "trusted_hosts".into(),
            toml::Value::Array(
                trusted_hosts
                    .iter()
                    .cloned()
                    .map(toml::Value::String)
                    .collect(),
            ),
        );
        server.insert(
            "session_ttl_hours".into(),
            toml::Value::Integer(session_ttl_hours as i64),
        );
        Ok(())
    })
}

/// Persist `[memory]` toggles for the current project without disturbing
/// unrelated config (docs/design/23-memory.md). Mirrors
/// [`persist_project_preferences`]'s atomic-write shape exactly.
pub fn persist_project_memory_prefs(
    cwd: &Path,
    search_enabled: Option<bool>,
    write_enabled: Option<bool>,
    reflection: Option<bool>,
    skill_proposals: Option<bool>,
) -> Result<(), ConfigError> {
    persist_memory_prefs_at(
        project_path(cwd),
        search_enabled,
        write_enabled,
        reflection,
        skill_proposals,
    )
}

/// Persist user-level `[memory]` defaults, inherited by project configs
/// through [`load_with_trust`] until they set their own scoped override.
pub fn persist_global_memory_prefs(
    search_enabled: Option<bool>,
    write_enabled: Option<bool>,
    reflection: Option<bool>,
    skill_proposals: Option<bool>,
) -> Result<(), ConfigError> {
    let path = global_path().ok_or_else(|| ConfigError::Write {
        path: PathBuf::from("<user-config>"),
        source: std::io::Error::other("user home is unavailable"),
    })?;
    persist_memory_prefs_at(
        path,
        search_enabled,
        write_enabled,
        reflection,
        skill_proposals,
    )
}

fn persist_memory_prefs_at(
    path: PathBuf,
    search_enabled: Option<bool>,
    write_enabled: Option<bool>,
    reflection: Option<bool>,
    skill_proposals: Option<bool>,
) -> Result<(), ConfigError> {
    update_config_file(&path, |document| {
        if search_enabled.is_some()
            || write_enabled.is_some()
            || reflection.is_some()
            || skill_proposals.is_some()
        {
            let memory = child_table(document, "memory", &path)?;
            if let Some(value) = search_enabled {
                memory.insert("search_enabled".into(), toml::Value::Boolean(value));
            }
            if let Some(value) = write_enabled {
                memory.insert("write_enabled".into(), toml::Value::Boolean(value));
            }
            if let Some(value) = reflection {
                memory.insert("reflection".into(), toml::Value::Boolean(value));
            }
            if let Some(value) = skill_proposals {
                memory.insert("skill_proposals".into(), toml::Value::Boolean(value));
            }
        }
        Ok(())
    })
}

/// Persist `[gateway] approvals` / `approver` without disturbing unrelated
/// config keys.
///
/// This is the one setting that decides whether an `Ask` raised on an
/// unattended chat surface reaches a human at all: with `approvals =
/// "deny"` every gate is a foregone denial, `GatewayApprover::answerable()`
/// is false, and `vak_core::reach` correctly strips every gated capability
/// from the turn before the prompt is composed. It had no writer on any
/// surface — the remedy `reach` prints named an action nothing could
/// perform — so an operator's only route was editing this file by hand.
///
/// `approver` is `Option<Option<String>>`: absent leaves it alone, `Some(None)`
/// clears it back to unset, `Some(Some(t))` pins the target. The
/// forward-requires-an-approver rule is NOT enforced here; it lives in
/// [`load_with_trust`], which is what every reader goes through, and
/// duplicating it would be a second contract that must agree forever.
/// Callers that want to reject the combination up front should check it
/// themselves and say so — writing a `forward` with no target simply
/// resolves back to `deny` with a warning, which is safe.
pub fn persist_gateway_approvals(
    path: PathBuf,
    approvals: Option<&str>,
    approver: Option<Option<&str>>,
    approval_timeout_secs: Option<u64>,
) -> Result<(), ConfigError> {
    persist_gateway_approvals_at(path, approvals, approver, approval_timeout_secs)
}

fn persist_gateway_approvals_at(
    path: PathBuf,
    approvals: Option<&str>,
    approver: Option<Option<&str>>,
    approval_timeout_secs: Option<u64>,
) -> Result<(), ConfigError> {
    update_config_file(&path, |document| {
        if approvals.is_some() || approver.is_some() || approval_timeout_secs.is_some() {
            let gateway = child_table(document, "gateway", &path)?;
            if let Some(value) = approvals {
                gateway.insert("approvals".into(), toml::Value::String(value.into()));
            }
            match approver {
                None => {}
                Some(None) => {
                    gateway.remove("approver");
                }
                Some(Some(target)) => {
                    gateway.insert("approver".into(), toml::Value::String(target.into()));
                }
            }
            if let Some(value) = approval_timeout_secs {
                gateway.insert(
                    "approval_timeout_secs".into(),
                    toml::Value::Integer(value as i64),
                );
            }
        }
        Ok(())
    })
}

/// Held by every write to a configuration file in this process, from the
/// read to the rename. One lock for every path rather than one per path:
/// the same file is reachable under more than one spelling (the default
/// workspace's project layer *is* the Shared layer), and a per-path lock
/// would first have to agree which spellings name one file. Configuration
/// writes are rare and small, so nothing waits on it for long.
static CONFIG_FILE_LOCK: Mutex<()> = Mutex::new(());

fn lock_config_files() -> MutexGuard<'static, ()> {
    CONFIG_FILE_LOCK
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
}

/// The one way a configuration file is rewritten: under
/// [`CONFIG_FILE_LOCK`], read the document at `path` (a missing file is an
/// empty one), let `edit` change it, and atomically replace the file with
/// the result.
///
/// Holding the lock from the read to the rename is what stops two settings
/// saved at the same moment from each rewriting the file from a read taken
/// before the other one landed. The document is edited as a raw TOML table,
/// so a key this version does not know is written back as it was read
/// (invariant 29). An edit that changes nothing writes nothing. `edit` must
/// not call another configuration writer: the lock is not reentrant.
fn update_config_file<T>(
    path: &Path,
    edit: impl FnOnce(&mut toml::Table) -> Result<T, ConfigError>,
) -> Result<T, ConfigError> {
    let _guard = lock_config_files();
    let before = match std::fs::read_to_string(path) {
        Ok(text) => toml::from_str::<toml::Table>(&text).map_err(|source| ConfigError::Parse {
            path: path.to_path_buf(),
            source,
        })?,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => toml::Table::new(),
        Err(source) => {
            return Err(ConfigError::Read {
                path: path.to_path_buf(),
                source,
            });
        }
    };
    let mut document = before.clone();
    let outcome = edit(&mut document)?;
    if document != before {
        let text = toml::to_string_pretty(&document).map_err(|error| ConfigError::Write {
            path: path.to_path_buf(),
            source: std::io::Error::other(error.to_string()),
        })?;
        replace_file(path, &text)?;
    }
    Ok(outcome)
}

/// Write `contents` to a new sibling of `path` and rename it over `path`, so
/// a reader sees the whole old document or the whole new one. The temporary
/// name carries the process id and a per-process sequence number, and is
/// created exclusively, so no two writes ever share one.
fn replace_file(path: &Path, contents: &str) -> Result<(), ConfigError> {
    static SEQUENCE: AtomicU64 = AtomicU64::new(0);
    let write_error = |source| ConfigError::Write {
        path: path.to_path_buf(),
        source,
    };
    let (Some(parent), Some(name)) = (path.parent(), path.file_name()) else {
        return Err(write_error(std::io::Error::other(
            "config path has no parent directory",
        )));
    };
    std::fs::create_dir_all(parent).map_err(|source| ConfigError::Write {
        path: parent.to_path_buf(),
        source,
    })?;
    let mut attempts = 0;
    let (temp, mut file) = loop {
        let temp = parent.join(format!(
            ".{}.{}.{}.tmp",
            name.to_string_lossy(),
            std::process::id(),
            SEQUENCE.fetch_add(1, Ordering::Relaxed)
        ));
        match std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&temp)
        {
            Ok(file) => break (temp, file),
            // Only another process that had this pid can hold a fresh name;
            // step past its file rather than write into it.
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists && attempts < 8 => {
                attempts += 1;
            }
            Err(source) => return Err(write_error(source)),
        }
    };
    let written = std::io::Write::write_all(&mut file, contents.as_bytes());
    drop(file);
    written
        .and_then(|()| std::fs::rename(&temp, path))
        .map_err(|source| {
            let _ = std::fs::remove_file(&temp);
            write_error(source)
        })
}

/// The table under `key`, created empty when absent. A key that holds some
/// other kind of value is refused rather than overwritten.
fn child_table<'a>(
    parent: &'a mut toml::Table,
    key: &str,
    path: &Path,
) -> Result<&'a mut toml::Table, ConfigError> {
    parent
        .entry(key)
        .or_insert_with(|| toml::Value::Table(toml::Table::new()))
        .as_table_mut()
        .ok_or_else(|| ConfigError::Write {
            path: path.to_path_buf(),
            source: std::io::Error::other(format!("`{key}` config must be a TOML table")),
        })
}

/// Persist the three permission rule lists (`allow` / `ask` / `deny`) as
/// the engine reads them, without disturbing unrelated config keys.
///
/// Each list is `Option`: absent leaves that list alone, `Some(vec)`
/// replaces it wholesale (an empty vec clears it).
///
/// Rule SYNTAX is not validated here on purpose: the grammar lives in
/// `vak_permission::Rule::parse`, which this crate sits below and must not
/// depend on. Callers parse every spec before calling — a second grammar
/// here would be two definitions of a rule that must agree forever.
pub fn persist_permission_rules(
    path: PathBuf,
    allow: Option<&[String]>,
    ask: Option<&[String]>,
    deny: Option<&[String]>,
) -> Result<(), ConfigError> {
    update_config_file(&path, |document| {
        for (key, list) in [("allow", allow), ("ask", ask), ("deny", deny)] {
            let Some(list) = list else { continue };
            document.insert(
                key.into(),
                toml::Value::Array(
                    list.iter()
                        .map(|spec| toml::Value::String(spec.clone()))
                        .collect(),
                ),
            );
        }
        Ok(())
    })
}

/// Persist the top-level `workers` toggle for the current project.
/// Mirrors [`persist_project_preferences`]'s atomic-write shape exactly.
pub fn persist_project_workers(cwd: &Path, enabled: bool) -> Result<(), ConfigError> {
    persist_workers_at(project_path(cwd), enabled)
}

/// Persist the user-level `workers` default, inherited by project
/// configs through [`load_with_trust`] until they set their own override.
pub fn persist_global_workers(enabled: bool) -> Result<(), ConfigError> {
    let path = global_path().ok_or_else(|| ConfigError::Write {
        path: PathBuf::from("<user-config>"),
        source: std::io::Error::other("user home is unavailable"),
    })?;
    persist_workers_at(path, enabled)
}

/// Persist the optional `[work]` policy fields without disturbing unrelated
/// config keys. The whole document is rewritten through the same atomic
/// rename boundary as the other authenticated preference endpoints.
pub fn persist_work_preferences(
    path: PathBuf,
    enabled: Option<bool>,
    default_mode: Option<&str>,
    max_items: Option<usize>,
    max_revisions: Option<u32>,
    max_parallel: Option<usize>,
    confirmation: Option<&str>,
) -> Result<(), ConfigError> {
    if [default_mode, confirmation]
        .into_iter()
        .flatten()
        .any(|value| value.trim().is_empty())
    {
        return Err(ConfigError::Write {
            path,
            source: std::io::Error::other("work policy values cannot be empty"),
        });
    }
    update_config_file(&path, |document| {
        let work = child_table(document, "work", &path)?;
        if let Some(value) = enabled {
            work.insert("enabled".into(), toml::Value::Boolean(value));
        }
        if let Some(value) = default_mode {
            work.insert("default_mode".into(), toml::Value::String(value.into()));
        }
        if let Some(value) = max_items {
            work.insert("max_items".into(), toml::Value::Integer(value as i64));
        }
        if let Some(value) = max_revisions {
            work.insert("max_revisions".into(), toml::Value::Integer(value as i64));
        }
        if let Some(value) = max_parallel {
            work.insert("max_parallel".into(), toml::Value::Integer(value as i64));
        }
        if let Some(value) = confirmation {
            work.insert("confirmation".into(), toml::Value::String(value.into()));
        }
        Ok(())
    })
}

/// Persist `[plugins] network_allow` at the given layer path.
///
/// `grant` is a three-state override: `Some(names)` grants egress to those
/// plugins, `Some(vec![])` removes the key entirely (so the layer reads as
/// deny-by-default and a narrower layer may inherit from a wider one), and
/// `None` leaves the file untouched. Grants are privileged and are also
/// demoted on read for untrusted project layers; callers must still refuse
/// a non-empty grant into an untrusted project rather than writing a value
/// the loader would silently discard.
pub fn persist_plugins_network_allow(
    path: &Path,
    grant: Option<Vec<String>>,
) -> Result<(), ConfigError> {
    if let Some(names) = &grant
        && names.iter().any(|name| name.trim().is_empty())
    {
        return Err(ConfigError::Write {
            path: path.to_path_buf(),
            source: std::io::Error::other("plugin names cannot be empty"),
        });
    }
    update_config_file(path, |document| {
        let plugins = child_table(document, "plugins", path)?;
        match grant {
            Some(names) if names.is_empty() => {
                plugins.remove("network_allow");
            }
            Some(names) => {
                plugins.insert(
                    "network_allow".into(),
                    toml::Value::Array(names.into_iter().map(toml::Value::String).collect()),
                );
            }
            None => {}
        }
        Ok(())
    })
}

fn persist_workers_at(path: PathBuf, enabled: bool) -> Result<(), ConfigError> {
    update_config_file(&path, |document| {
        document.insert("workers".into(), toml::Value::Boolean(enabled));
        // Drop the legacy alias so we don't leave two competing keys behind
        // once this layer has been rewritten under the new name.
        document.remove("subagents");
        Ok(())
    })
}

/// Persist `[finops]` budget caps for the current project. `None` leaves
/// that cap alone; `Some(None)` clears it (removes the key, so it reads
/// back as "no cap" rather than as an explicit zero); `Some(Some(v))` sets
/// it. Mirrors [`persist_project_preferences`]'s atomic-write shape.
pub fn persist_project_finops_caps(
    cwd: &Path,
    max_run_usd: Option<Option<f64>>,
    max_day_usd: Option<Option<f64>>,
) -> Result<(), ConfigError> {
    persist_finops_caps_at(project_path(cwd), max_run_usd, max_day_usd)
}

/// Persist one exact model price in the current project's `[finops.price_overrides]`
/// table. `None` removes the project entry and resumes inheritance.
pub fn persist_project_finops_price(
    cwd: &Path,
    model: &str,
    price: Option<PriceEntry>,
) -> Result<(), ConfigError> {
    let path = project_path(cwd);
    update_config_file(&path, |document| {
        let finops = child_table(document, "finops", &path)?;
        let prices = child_table(finops, "price_overrides", &path)?;
        match price {
            Some(entry) => {
                prices.insert(
                    model.to_string(),
                    toml::Value::try_from(entry).map_err(|error| ConfigError::Write {
                        path: path.clone(),
                        source: std::io::Error::other(error.to_string()),
                    })?,
                );
            }
            None => {
                prices.remove(model);
            }
        }
        Ok(())
    })
}

/// A partial update to `[voice]`. `None` leaves a key untouched; for the
/// optional route keys `Some(None)` removes the key so it inherits again.
#[derive(Debug, Clone, Default)]
pub struct VoicePatch {
    pub enabled: Option<bool>,
    pub max_session_secs: Option<u64>,
    pub max_concurrent: Option<usize>,
    pub max_audio_bytes: Option<u64>,
    pub provider: Option<Option<String>>,
    pub transcription_model: Option<Option<String>>,
    pub synthesis_model: Option<Option<String>>,
}

impl VoicePatch {
    pub fn is_empty(&self) -> bool {
        self.enabled.is_none()
            && self.max_session_secs.is_none()
            && self.max_concurrent.is_none()
            && self.max_audio_bytes.is_none()
            && self.provider.is_none()
            && self.transcription_model.is_none()
            && self.synthesis_model.is_none()
    }
}

/// Atomically apply `patch` to the `[voice]` table of the config file at
/// `path`, preserving every key the patch does not name.
pub fn persist_voice_settings_at(path: PathBuf, patch: &VoicePatch) -> Result<(), ConfigError> {
    update_config_file(&path, |document| {
        let voice = child_table(document, "voice", &path)?;
        if let Some(v) = patch.enabled {
            voice.insert("enabled".into(), toml::Value::Boolean(v));
        }
        for (key, value) in [
            ("max_session_secs", patch.max_session_secs),
            ("max_concurrent", patch.max_concurrent.map(|v| v as u64)),
            ("max_audio_bytes", patch.max_audio_bytes),
        ] {
            if let Some(v) = value {
                voice.insert(key.into(), toml::Value::Integer(v as i64));
            }
        }
        for (key, value) in [
            ("provider", &patch.provider),
            ("transcription_model", &patch.transcription_model),
            ("synthesis_model", &patch.synthesis_model),
        ] {
            match value {
                Some(Some(v)) => {
                    voice.insert(key.into(), toml::Value::String(v.clone()));
                }
                Some(None) => {
                    voice.remove(key);
                }
                None => {}
            }
        }
        Ok(())
    })
}

/// Persist the user-level `[finops]` defaults, inherited by project
/// configs through [`load_with_trust`] until they set their own override.
pub fn persist_global_finops_caps(
    max_run_usd: Option<Option<f64>>,
    max_day_usd: Option<Option<f64>>,
) -> Result<(), ConfigError> {
    let path = global_path().ok_or_else(|| ConfigError::Write {
        path: PathBuf::from("<user-config>"),
        source: std::io::Error::other("user home is unavailable"),
    })?;
    persist_finops_caps_at(path, max_run_usd, max_day_usd)
}

fn persist_finops_caps_at(
    path: PathBuf,
    max_run_usd: Option<Option<f64>>,
    max_day_usd: Option<Option<f64>>,
) -> Result<(), ConfigError> {
    update_config_file(&path, |document| {
        if max_run_usd.is_some() || max_day_usd.is_some() {
            let finops = child_table(document, "finops", &path)?;
            for (key, cap) in [("max_run_usd", max_run_usd), ("max_day_usd", max_day_usd)] {
                match cap {
                    Some(Some(value)) => {
                        finops.insert(key.into(), toml::Value::Float(value));
                    }
                    Some(None) => {
                        finops.remove(key);
                    }
                    None => {}
                }
            }
        }
        Ok(())
    })
}

pub fn load(cwd: &Path) -> Result<Config, ConfigError> {
    load_with_trust(cwd, true)
}

/// Keys a PROJECT-level config may not set when its workspace has not been
/// marked trusted: they grant execution or redirect credentials.
const PRIVILEGED_KEYS_NOTICE: &str = "permission_mode, approval_mode, allow, hooks, anthropic_base_url, bedrock_region, mcp.servers, gateway, sandbox, server, update, capabilities, intent.autonomy, intent.escalate=cloud, intent.enabled=false, intent.posture=false, plugins.network_allow, server.bus";

pub fn load_with_trust(cwd: &Path, trust_project: bool) -> Result<Config, ConfigError> {
    let mut warnings = Vec::new();
    let mut layers: Vec<FileConfig> = vec![FileConfig::default()];

    if let Some(gp) = global_path().filter(|gp| gp.is_file()) {
        let (fc, w) = parse_file(&gp)?;
        warnings.extend(w);
        layers.push(fc);
    }
    let pp = project_path(cwd);
    let is_global_workspace = global_path().is_some_and(|global| global == pp);
    if pp.is_file() && !is_global_workspace {
        let (mut fc, w) = parse_file(&pp)?;
        warnings.extend(w);
        if !trust_project {
            // A repository must not be able to configure itself into
            // execution power on first run. Restrictive keys (deny/ask)
            // still apply.
            if fc.permission_mode.is_some() {
                fc.permission_mode = None;
            }
            if fc.approval_mode.is_some() {
                fc.approval_mode = None;
            }
            if fc.anthropic_base_url.is_some() {
                fc.anthropic_base_url = None;
            }
            if fc.bedrock_region.is_some() {
                fc.bedrock_region = None;
            }
            fc.allow.clear();
            fc.hooks.clear();
            fc.mcp.servers.clear();
            if fc.gateway.enabled.is_some() || !fc.gateway.outbound.webhooks.is_empty() {
                // Outbound webhook URLs are exfil targets just like base-url
                // redirection: the whole section is privileged.
                fc.gateway = GatewaySettings::default();
            }
            // Image choice is supply-chain power; keep it with the user.
            if fc.sandbox.backend.is_some() || fc.sandbox.image.is_some() {
                fc.sandbox = SandboxSettings::default();
            }
            // The release feed decides which binary replaces this one, and
            // its artifact hashes come from the feed itself — an attacker who
            // picks the URL picks the checksum too. Same class of power as
            // anthropic_base_url, and it was not stripped here.
            if fc.update.url.is_some() {
                fc.update.url = None;
            }
            // `inherit_* = false` clears the corresponding global layer in
            // `merge_into`, so an untrusted project could switch off the
            // user's own hooks and MCP servers — disabling a protection is
            // as privileged as adding a capability.
            fc.capabilities = CapabilityInheritanceSettings::default();
            // `delegated` and `autonomous` suppress approval gates, and a
            // cloud classification tier spends the user's credentials before
            // the run they asked for. Both are execution power; a cloned
            // repository must not grant them to itself. The rest of [intent]
            // only ever narrows, so it survives untrusted.
            if fc.intent.autonomy.is_some() {
                fc.intent.autonomy = None;
            }
            if fc.intent.escalate.as_deref() == Some("cloud") {
                fc.intent.escalate = None;
            }
            // Switching the kernel or its posture off removes the approval
            // floor it raises — "force push to production" asks even under
            // auto-approve only while both are on — so turning either off is
            // disabling a protection, which is as privileged as granting.
            if fc.intent.enabled == Some(false) {
                fc.intent.enabled = None;
            }
            if fc.intent.posture == Some(false) {
                fc.intent.posture = None;
            }
            // Network exposure is not a project's decision to make. `bind`
            // chooses which interface answers, `trusted_hosts` relaxes the
            // DNS-rebinding defence, and `web.terminal` opens a shell to
            // whoever can reach the port — a cloned repository that could
            // set these would be handing itself the machine.
            fc.server = ServerSettings::default();
            fc.plugins.network_allow = None;
            fc.plugins.allow = None;
            warnings.push(format!(
                "project .vak/config.toml is not trusted for this workspace; \
                 ignored privileged keys ({PRIVILEGED_KEYS_NOTICE}). \
                 Re-run and confirm the workspace prompt, or pass --trust, to apply them."
            ));
        }
        layers.push(fc);
    }

    let mut merged = FileConfig::default();
    for layer in layers {
        merge_into(&mut merged, layer);
    }

    if let Ok(m) = std::env::var("VAK_MODEL") {
        merged.model = Some(m);
    }
    if let Ok(p) = std::env::var("VAK_PROVIDER") {
        merged.provider = Some(p);
    }

    let mut cfg = Config {
        warnings,
        ..Default::default()
    };
    if let Some(provider) = merged.provider {
        cfg.provider = provider;
    }
    if let Some(model) = merged.model {
        cfg.model = model;
    }
    if let Some(mt) = merged.max_tokens {
        cfg.max_tokens = mt;
    }
    if let Some(turns) = merged.max_turns {
        cfg.max_turns = turns;
    }
    if let Some(mode) = merged.permission_mode {
        cfg.permission_mode = mode;
    }
    if let Some(mode) = merged.approval_mode {
        cfg.approval_mode = mode;
    }
    cfg.anthropic_base_url = merged.anthropic_base_url;
    if let Some(region) = merged.bedrock_region {
        cfg.bedrock_region = region;
    }
    cfg.allow = merged.allow;
    cfg.ask = merged.ask;
    cfg.deny = merged.deny;
    if let Some(sa) = merged.workers {
        cfg.workers = sa;
    }
    if let Some(r) = merged.max_retries {
        cfg.max_retries = r;
    }
    if let Some(ms) = merged.retry_base_backoff_ms {
        cfg.retry_base_backoff_ms = ms;
    }
    if let Some(secs) = merged.request_timeout_secs {
        cfg.request_timeout_secs = secs;
    }
    if let Some(n) = merged.run_retry_attempts {
        cfg.run_retry_attempts = n;
    }
    if let Some(ms) = merged.run_retry_base_backoff_ms {
        cfg.run_retry_base_backoff_ms = ms;
    }
    if let Some(t) = merged.circuit_breaker_threshold {
        cfg.circuit_breaker_threshold = t;
    }
    if let Some(c) = merged.circuit_breaker_cooldown_secs {
        cfg.circuit_breaker_cooldown_secs = c;
    }
    if let Some(w) = merged.context_window {
        if w < 16_384 {
            cfg.warnings.push(format!(
                "context_window {w} too small; using default {}",
                cfg.context_window
            ));
        } else {
            cfg.context_window = w;
        }
    }
    cfg.voice = merged.voice.unwrap_or_default();
    cfg.hooks = merged.hooks;
    for (name, srv) in merged.mcp.servers {
        cfg.mcp.servers.insert(name, srv);
    }
    cfg.capabilities = CapabilityInheritanceResolved {
        inherit_mcp: merged.capabilities.inherit_mcp.unwrap_or(true),
        inherit_hooks: merged.capabilities.inherit_hooks.unwrap_or(true),
        inherit_skills: merged.capabilities.inherit_skills.unwrap_or(true),
        inherit_commands: merged.capabilities.inherit_commands.unwrap_or(true),
        inherit_plugins: merged.capabilities.inherit_plugins.unwrap_or(true),
    };
    cfg.ui.theme = merged.ui.theme.clone().unwrap_or_else(|| "dark".into());
    let builtin = matches!(
        cfg.ui.theme.as_str(),
        "dark"
            | "light"
            | "neo"
            | "rich"
            | "teenage"
            | "plain"
            | "midnight"
            | "synthwave"
            | "forest"
    );
    if !builtin && !merged.ui.themes.contains_key(&cfg.ui.theme) {
        cfg.warnings
            .push(format!("unknown ui.theme '{}'; using 'dark'", cfg.ui.theme));
        cfg.ui.theme = "dark".into();
    }
    cfg.ui.bell = merged.ui.bell.unwrap_or(true);
    cfg.ui.keymap = merged.ui.keymap;
    for (name, colors) in merged.ui.themes {
        let entry = cfg.ui.themes.entry(name.clone()).or_default();
        for (key, value) in colors {
            match value.as_str() {
                Some(v) => {
                    entry.insert(key, v.to_string());
                }
                None => cfg.warnings.push(format!(
                    "ui.themes.{name}.{key} must be a string color (ignored)"
                )),
            }
        }
    }
    cfg.ui.composer = match merged.ui.composer.as_deref() {
        Some("vim") => "vim".into(),
        Some("emacs") | None => "emacs".into(),
        Some(other) => {
            cfg.warnings
                .push(format!("unknown ui.composer '{other}'; using 'emacs'"));
            "emacs".into()
        }
    };
    cfg.ui.osc52 = merged.ui.osc52.unwrap_or(false);
    let acc = merged.ui.accessibility.unwrap_or_default();
    cfg.ui.accessibility = AccessibilityResolved {
        plain: acc.plain.unwrap_or(false),
        reduced_motion: acc.reduced_motion.unwrap_or(false),
        screen_reader: acc.screen_reader.unwrap_or(false),
    };

    let sp = merged.stop_policy.unwrap_or_default();
    cfg.stop_policy = StopPolicyResolved {
        enabled: sp.enabled.unwrap_or(true),
        marker_gate: sp.marker_gate.unwrap_or(true),
        verify_gate: sp.verify_gate.unwrap_or(true),
        max_blocks: sp.max_blocks.unwrap_or(2),
    };

    cfg.finops = FinopsResolved {
        max_run_usd: merged.finops.max_run_usd,
        max_day_usd: merged.finops.max_day_usd,
        price_overrides: merged.finops.price_overrides.clone(),
    };

    cfg.goal = GoalResolved {
        handoff_reset: merged.goal.handoff_reset.unwrap_or(true),
        max_audit_blocks: merged.goal.max_audit_blocks.unwrap_or(2),
    };

    cfg.work = WorkResolved {
        enabled: merged.work.enabled.unwrap_or(true),
        default_mode: match merged.work.default_mode.as_deref() {
            None | Some("direct") => "direct".into(),
            Some("managed") => "managed".into(),
            Some("auto") => "auto".into(),
            Some(other) => {
                cfg.warnings.push(format!(
                    "unknown work.default_mode '{other}'; using 'direct'"
                ));
                "direct".into()
            }
        },
        max_items: merged.work.max_items.unwrap_or(20).clamp(1, 128),
        max_revisions: merged.work.max_revisions.unwrap_or(8).clamp(1, 64),
        max_parallel: merged.work.max_parallel.unwrap_or(4).clamp(1, 32),
        confirmation: match merged.work.confirmation.as_deref() {
            None | Some("risk-based") => "risk-based".into(),
            Some("always") => "always".into(),
            Some("never") => "never".into(),
            Some(other) => {
                cfg.warnings.push(format!(
                    "unknown work.confirmation '{other}'; using 'risk-based'"
                ));
                "risk-based".into()
            }
        },
    };

    cfg.route.objective = match merged.route.objective.as_deref() {
        None | Some("auto") => "auto".into(),
        Some("utility") => "utility".into(),
        Some("balanced") => "balanced".into(),
        Some("quality-critical") | Some("quality_critical") => "quality-critical".into(),
        Some(other) => {
            cfg.warnings.push(format!(
                "unknown route.objective '{other}'; using 'auto' \
                 (valid: auto | utility | balanced | quality-critical)"
            ));
            "auto".into()
        }
    };
    cfg.route.fallback_models = merged.route.fallback_models.clone();
    cfg.route.same_model = resolve_same_model(&merged.route.same_model, &mut cfg.warnings);
    cfg.route.max_fallbacks = merged.route.max_fallbacks.unwrap_or(4).clamp(1, 16);
    cfg.route.quality_hints = merged
        .route
        .quality_hints
        .iter()
        .map(|h| h.to_ascii_lowercase())
        .collect();
    cfg.route.modality_hints = merged
        .route
        .modality_hints
        .iter()
        .map(|h| h.to_ascii_lowercase())
        .collect();

    cfg.probe.hosted = match merged.probe.hosted.as_deref() {
        None | Some("none") => "none".into(),
        Some("full") => "full".into(),
        Some(other) => {
            cfg.warnings.push(format!(
                "unknown probe.hosted '{other}'; using 'none' (valid: none | full)"
            ));
            "none".into()
        }
    };

    // --- providers.ollama (docs/design/68-context-engine.md §8) ---
    match merged.providers.ollama.validate() {
        Ok(()) => {
            if let Some(keep_alive) = merged.providers.ollama.keep_alive.clone() {
                cfg.ollama.keep_alive = keep_alive;
            }
            cfg.ollama.num_ctx = merged.providers.ollama.num_ctx;
        }
        Err(msg) => cfg.warnings.push(msg),
    }

    // --- providers.anthropic (docs/design/68-context-engine.md §11) ---
    cfg.anthropic.fast_mode = merged.providers.anthropic.fast_mode.unwrap_or(false);
    cfg.google_project_id = merged.providers.google.project_id.clone();

    // --- intent kernel (docs/design/47-commitment-kernel.md) ---
    cfg.intent.enabled = merged.intent.enabled.unwrap_or(true);
    // Clamped rather than rejected: a nonsensical threshold should not stop
    // the runtime, and the clamp keeps the ordering invariant that
    // `provisional <= accept` even if an operator inverts them. TOML can
    // spell `nan` and `inf`, and `clamp` passes NaN straight through, where
    // every comparison against it is false — so a non-finite value is
    // replaced by the default before it is clamped.
    let mut finite = |value: Option<f64>, default: f64, key: &str| match value {
        Some(value) if !value.is_finite() => {
            cfg.warnings.push(format!(
                "{key} = {value} is not a finite number; using {default}"
            ));
            default
        }
        other => other.unwrap_or(default),
    };
    let accept = finite(
        merged.intent.accept_confidence,
        0.75,
        "intent.accept_confidence",
    );
    let provisional = finite(
        merged.intent.provisional_confidence,
        0.45,
        "intent.provisional_confidence",
    );
    let max_classify = finite(
        merged.intent.max_classify_usd,
        0.01,
        "intent.max_classify_usd",
    );
    let lifetime_budget = merged
        .commitment
        .lifetime_budget_usd
        .map(|budget| finite(Some(budget), 0.0, "commitment.lifetime_budget_usd"));
    cfg.intent.accept_confidence = accept.clamp(0.0, 1.0);
    cfg.intent.provisional_confidence = provisional.clamp(0.0, cfg.intent.accept_confidence);
    cfg.intent.slice_capabilities = merged.intent.slice_capabilities.unwrap_or(true);
    cfg.intent.posture = merged.intent.posture.unwrap_or(true);
    cfg.intent.escalate = match merged.intent.escalate.as_deref() {
        Some("none") | None => "none".into(),
        Some("local") => "local".into(),
        Some("cloud") => "cloud".into(),
        Some(other) => {
            cfg.warnings.push(format!(
                "unknown intent.escalate '{other}'; using 'none' \
                 (valid: none, local, cloud)"
            ));
            "none".into()
        }
    };
    cfg.intent.classify_model = merged
        .intent
        .classify_model
        .clone()
        .filter(|model| !model.trim().is_empty());
    cfg.intent.max_classify_usd = max_classify.clamp(0.0, 1.0);
    cfg.intent.classify_timeout_secs = merged
        .intent
        .classify_timeout_secs
        .unwrap_or(10)
        .clamp(1, 120);
    cfg.intent.autonomy = match merged.intent.autonomy.as_deref() {
        Some("manual") => "manual".into(),
        Some("assisted") | None => "assisted".into(),
        Some("delegated") => "delegated".into(),
        Some("autonomous") => "autonomous".into(),
        Some(other) => {
            cfg.warnings.push(format!(
                "unknown intent.autonomy '{other}'; using 'assisted' \
                 (valid: manual, assisted, delegated, autonomous)"
            ));
            "assisted".into()
        }
    };
    cfg.intent.evidence_max_age_secs = merged.intent.evidence_max_age_secs.unwrap_or(86_400).max(0);

    // --- durable commitments ---
    cfg.commitment.enabled = merged.commitment.enabled.unwrap_or(true);
    cfg.commitment.lifetime_budget_usd = lifetime_budget.filter(|budget| *budget > 0.0);
    cfg.commitment.stall_limit = merged.commitment.stall_limit.unwrap_or(3).clamp(1, 100);
    cfg.commitment.review_every_hours = merged
        .commitment
        .review_every_hours
        .filter(|hours| *hours > 0);
    cfg.commitment.default_ttl_days = merged.commitment.default_ttl_days.filter(|days| *days > 0);

    cfg.gateway.enabled = merged.gateway.enabled.unwrap_or(false);
    cfg.gateway.approvals = match merged.gateway.approvals.as_deref() {
        Some("deny") | None => "deny".into(),
        Some("forward") => "forward".into(),
        Some(other) => {
            cfg.warnings
                .push(format!("unknown gateway.approvals '{other}'; using 'deny'"));
            "deny".into()
        }
    };
    cfg.gateway.approver = merged.gateway.approver.clone();
    if cfg.gateway.approvals == "forward" {
        let ok = cfg
            .gateway
            .approver
            .as_deref()
            .is_some_and(|t| t.contains(':') && !t.trim().is_empty());
        if !ok {
            cfg.warnings.push(
                "gateway.approvals = 'forward' requires gateway.approver = '<surface>:<chat>'; \
                 falling back to 'deny'"
                    .into(),
            );
            cfg.gateway.approvals = "deny".into();
            cfg.gateway.approver = None;
        }
    }
    match merged.gateway.approval_timeout_secs {
        Some(t) if t < 5 => {
            cfg.warnings.push(format!(
                "gateway.approval_timeout_secs {t} below minimum; using 5"
            ));
            cfg.gateway.approval_timeout_secs = 5;
        }
        Some(t) => cfg.gateway.approval_timeout_secs = t,
        None => {}
    }
    cfg.gateway.rate_limit = merged.gateway.rate_limit.clone();
    cfg.gateway.chat_allowlist = merged.gateway.chat_allowlist.clone();
    cfg.gateway.chat_allowlist_open = merged.gateway.chat_allowlist_open.unwrap_or(false);
    cfg.gateway.core_pool_max = merged.gateway.core_pool_max.unwrap_or(8).max(1);
    cfg.gateway.core_pool_idle_secs = merged.gateway.core_pool_idle_secs.unwrap_or(1800).max(60);
    cfg.gateway.pending_expiry_days = merged.gateway.pending_expiry_days.unwrap_or(7).max(1);
    if cfg.gateway.enabled
        && cfg.gateway.chat_allowlist.is_empty()
        && cfg.gateway.chat_allowlist_open
    {
        cfg.warnings.push(
            "gateway.chat_allowlist is empty and gateway.chat_allowlist_open = true: \
             every inbound chat is accepted. Set gateway.chat_allowlist to restrict access."
                .into(),
        );
    }
    cfg.memory.search_enabled = merged.memory.search_enabled.unwrap_or(true);
    cfg.memory.write_enabled = merged.memory.write_enabled.unwrap_or(true);
    cfg.memory.skill_proposals = merged.memory.skill_proposals.unwrap_or(true);
    cfg.memory.reflection = merged.memory.reflection.unwrap_or(false);
    cfg.sandbox.backend = match merged.sandbox.backend.as_deref() {
        None => "auto".into(),
        Some(b @ ("auto" | "seatbelt" | "landlock" | "docker")) => b.into(),
        Some(other) => {
            cfg.warnings
                .push(format!("unknown sandbox.backend '{other}'; using 'auto'"));
            "auto".into()
        }
    };
    cfg.sandbox.image = merged.sandbox.image.clone();
    cfg.automation.catch_up_missed = merged.automation.catch_up_missed.unwrap_or(true);
    cfg.update.url = merged.update.url.clone();
    cfg.update.interval_hours = merged.update.interval_hours.unwrap_or(24);
    cfg.tools.web_fetch = merged.tools.web_fetch.unwrap_or(true);
    cfg.tools.browse = merged.tools.browse.unwrap_or(true);
    let hb = &merged.heartbeat;
    cfg.heartbeat.interval_secs = match hb.interval_secs {
        Some(v) if v < 300 => {
            cfg.warnings.push(format!(
                "heartbeat.interval_secs {v} below minimum; using 300"
            ));
            300
        }
        Some(v) => v,
        None => 1800,
    };
    cfg.heartbeat.enabled = hb.enabled.unwrap_or(false);
    cfg.heartbeat.model = hb.model.clone();
    cfg.heartbeat.quiet_hours = match hb.quiet_hours.as_deref() {
        None => None,
        Some(raw) => match QuietWindow::parse(raw) {
            Some(w) => Some(w),
            None => {
                cfg.warnings.push(format!(
                    "heartbeat.quiet_hours '{raw}' is not 'HH:MM-HH:MM'; ignoring"
                ));
                None
            }
        },
    };
    match hb.max_findings {
        Some(0) => {
            cfg.warnings
                .push("heartbeat.max_findings must be >= 1; using 1".into());
            cfg.heartbeat.max_findings = 1;
        }
        Some(n) => cfg.heartbeat.max_findings = n,
        None => cfg.heartbeat.max_findings = 3,
    }
    // ---- [server] (docs/design/48-web-client.md §4.2) --------------------
    //
    // Defaults reproduce the pre-web behaviour exactly: loopback only, no
    // trusted hosts, no terminal over the web. Every step away from that is
    // something an operator typed on purpose.
    let sv = &merged.server;
    cfg.server.bind = sv
        .bind
        .clone()
        .map(|b| b.trim().to_string())
        .filter(|b| !b.is_empty())
        .unwrap_or_else(|| "127.0.0.1".into());
    cfg.server.trusted_hosts = sv
        .trusted_hosts
        .clone()
        .unwrap_or_default()
        .into_iter()
        .map(|h| h.trim().to_ascii_lowercase())
        .filter(|h| !h.is_empty())
        .collect();
    if cfg.server.trusted_hosts.iter().any(|h| h.contains('*')) {
        cfg.server.trusted_hosts.retain(|h| !h.contains('*'));
        cfg.warnings.push(
            "server.trusted_hosts entries containing '*' were ignored: a wildcard \
             defeats the DNS-rebinding check the list exists to enforce"
                .into(),
        );
    }
    cfg.server.public_url = sv
        .public_url
        .clone()
        .map(|u| u.trim().trim_end_matches('/').to_string())
        .filter(|u| !u.is_empty());
    cfg.server.session_ttl_hours = sv.session_ttl_hours.filter(|h| *h > 0).unwrap_or(168);
    cfg.server.loopback_auto_login = sv.loopback_auto_login.unwrap_or(true);
    cfg.server.workspace_roots = sv
        .workspace_roots
        .clone()
        .unwrap_or_default()
        .into_iter()
        .map(std::path::PathBuf::from)
        .filter(|p| p.is_absolute())
        .collect();
    cfg.server.web_terminal = sv.web.terminal.unwrap_or(false);
    cfg.server.web_terminal_requires_loopback = sv.web.terminal_requires_loopback.unwrap_or(true);
    // Bus config: resolve the workspace encryption secret from the named env var.
    // The env var name itself is never stored in the resolved config — only the
    // secret bytes read from the environment at resolution time.
    cfg.server.bus = BusResolved {
        nats_url: sv.bus.nats_url.clone(),
        workspace_secret: sv
            .bus
            .workspace_secret_env
            .as_deref()
            .and_then(|env_name| std::env::var(env_name).ok())
            .map(|s| s.into_bytes()),
    };
    for (name, hook) in merged.gateway.outbound.webhooks {
        if hook.url.trim().is_empty() {
            cfg.warnings.push(format!(
                "gateway.outbound.webhooks.{name}.url is empty; ignored"
            ));
            continue;
        }
        if let Some(env_name) = &hook.token_env
            && env_name.trim().is_empty()
        {
            cfg.warnings.push(format!(
                "gateway.outbound.webhooks.{name}.token_env is empty; delivery will fail closed"
            ));
        }
        cfg.gateway.webhooks.insert(
            name,
            WebhookResolved {
                url: hook.url,
                token_env: hook.token_env,
            },
        );
    }

    cfg.plugins = PluginResolved {
        enabled: merged.plugins.enabled,
        disabled: merged.plugins.disabled,
        allow: merged.plugins.allow,
        deny: merged.plugins.deny,
        network_allow: merged.plugins.network_allow,
        network_deny: merged.plugins.network_deny,
    };

    if let Some(name) = &merged.profile {
        if let Some(profile) = merged.profiles.get(name) {
            if let Some(model) = &profile.model {
                cfg.model = model.clone();
            }
            if let Some(provider) = &profile.provider {
                cfg.provider = provider.clone();
            }
            if let Some(mode) = profile.permission_mode {
                cfg.permission_mode = mode;
            }
            if let Some(turns) = profile.max_turns {
                cfg.max_turns = turns;
            }
        } else {
            cfg.warnings.push(format!("profile '{name}' not defined"));
        }
    }

    Ok(cfg)
}

fn parse_file(path: &Path) -> Result<(FileConfig, Vec<String>), ConfigError> {
    let text = std::fs::read_to_string(path).map_err(|source| ConfigError::Read {
        path: path.to_path_buf(),
        source,
    })?;
    let fc: FileConfig = toml::from_str(&text).map_err(|source| ConfigError::Parse {
        path: path.to_path_buf(),
        source,
    })?;
    let warnings = unknown_key_warnings(path, &text);
    Ok((fc, warnings))
}

const KNOWN_TOP_KEYS: &[&str] = &[
    "provider",
    "model",
    "max_tokens",
    "max_turns",
    "permission_mode",
    "approval_mode",
    "profile",
    "profiles",
    "anthropic_base_url",
    "bedrock_region",
    "allow",
    "ask",
    "deny",
    "workers",
    // Legacy alias for `workers`, kept so old configs don't trigger an
    // "unknown key" warning.
    "subagents",
    "hooks",
    "max_retries",
    "retry_base_backoff_ms",
    "request_timeout_secs",
    "run_retry_attempts",
    "run_retry_base_backoff_ms",
    "circuit_breaker_threshold",
    "circuit_breaker_cooldown_secs",
    "context_window",
    "mcp",
    "capabilities",
    "ui",
    "stop_policy",
    "work",
    "gateway",
    "memory",
    "sandbox",
    "finops",
    "automation",
    "update",
    "tools",
    "heartbeat",
    "server",
    "plugins",
    "intent",
    "voice",
    "probe",
    "providers",
];
const KNOWN_PLUGINS_KEYS: &[&str] = &[
    "enabled",
    "disabled",
    "allow",
    "deny",
    "network_allow",
    "network_deny",
];
const KNOWN_FINOPS_KEYS: &[&str] = &["max_run_usd", "max_day_usd", "price_overrides"];
const KNOWN_GOAL_KEYS: &[&str] = &["handoff_reset", "max_audit_blocks"];
const KNOWN_WORK_KEYS: &[&str] = &[
    "enabled",
    "default_mode",
    "max_items",
    "max_revisions",
    "max_parallel",
    "confirmation",
];
const KNOWN_PROFILE_KEYS: &[&str] = &["model", "provider", "permission_mode", "max_turns"];
const KNOWN_HOOK_KEYS: &[&str] = &[
    "event",
    "match",
    "command",
    "timeout_ms",
    "enabled",
    "failure_mode",
];
const KNOWN_MCP_SERVER_KEYS: &[&str] = &["command", "args", "env", "network"];
const KNOWN_CAPABILITY_KEYS: &[&str] = &[
    "inherit_mcp",
    "inherit_hooks",
    "inherit_skills",
    "inherit_commands",
    "inherit_plugins",
];
const KNOWN_UI_KEYS: &[&str] = &[
    "theme",
    "bell",
    "keymap",
    "composer",
    "osc52",
    "accessibility",
    "themes",
];
const KNOWN_THEME_COLORS: &[&str] = &[
    "accent",
    "dim",
    "success",
    "error",
    "warning",
    "heading",
    "code",
    "user",
    "spinner",
    "panel_bg",
    "selected_bg",
];
const KNOWN_ACCESSIBILITY_KEYS: &[&str] = &["plain", "reduced_motion", "screen_reader"];
const KNOWN_STOP_POLICY_KEYS: &[&str] = &["enabled", "marker_gate", "verify_gate", "max_blocks"];
const KNOWN_GATEWAY_KEYS: &[&str] = &[
    "enabled",
    "approvals",
    "approver",
    "approval_timeout_secs",
    "outbound",
    "rate_limit",
    "chat_allowlist",
    "chat_allowlist_open",
    "core_pool_max",
    "core_pool_idle_secs",
    "pending_expiry_days",
];
const KNOWN_MEMORY_KEYS: &[&str] = &[
    "search_enabled",
    "write_enabled",
    "skill_proposals",
    "reflection",
];
const KNOWN_SANDBOX_KEYS: &[&str] = &["backend", "image"];
const KNOWN_OUTBOUND_KEYS: &[&str] = &["webhooks"];
const KNOWN_WEBHOOK_KEYS: &[&str] = &["url", "token_env"];
const KNOWN_AUTOMATION_KEYS: &[&str] = &["catch_up_missed"];
const KNOWN_UPDATE_KEYS: &[&str] = &["url", "interval_hours"];
const KNOWN_TOOLS_KEYS: &[&str] = &["web_fetch", "browse"];
const KNOWN_HEARTBEAT_KEYS: &[&str] = &[
    "enabled",
    "interval_secs",
    "model",
    "quiet_hours",
    "max_findings",
];
const KNOWN_PROVIDERS_KEYS: &[&str] = &["ollama", "anthropic", "google"];
const KNOWN_OLLAMA_KEYS: &[&str] = &["keep_alive", "num_ctx"];
const KNOWN_ANTHROPIC_PROVIDER_KEYS: &[&str] = &["fast_mode"];
const KNOWN_GOOGLE_PROVIDER_KEYS: &[&str] = &["project_id"];

/// A typo'd key must be visible, not silently dead: diff the raw TOML
/// against the known schema and surface every unrecognized key.
fn unknown_key_warnings(path: &Path, text: &str) -> Vec<String> {
    let Ok(v) = toml::from_str::<toml::Value>(text) else {
        return Vec::new();
    };
    let mut out = Vec::new();
    let Some(top) = v.as_table() else {
        return out;
    };
    for key in top.keys() {
        if !KNOWN_TOP_KEYS.contains(&key.as_str()) {
            out.push(format!(
                "{}: unknown config key '{key}' (ignored)",
                path.display()
            ));
        }
    }
    if let Some(profiles) = top.get("profiles").and_then(toml::Value::as_table) {
        for (name, t) in profiles {
            if let Some(t) = t.as_table() {
                for key in t.keys() {
                    if !KNOWN_PROFILE_KEYS.contains(&key.as_str()) {
                        out.push(format!(
                            "{}: unknown profile key 'profiles.{name}.{key}' (ignored)",
                            path.display()
                        ));
                    }
                }
            }
        }
    }
    if let Some(hooks) = top.get("hooks").and_then(toml::Value::as_array) {
        for (i, h) in hooks.iter().enumerate() {
            if let Some(h) = h.as_table() {
                for key in h.keys() {
                    if !KNOWN_HOOK_KEYS.contains(&key.as_str()) {
                        out.push(format!(
                            "{}: unknown hook key 'hooks[{i}].{key}' (ignored)",
                            path.display()
                        ));
                    }
                }
            }
        }
    }
    if let Some(servers) = top
        .get("mcp")
        .and_then(|m| m.get("servers"))
        .and_then(toml::Value::as_table)
    {
        for (name, t) in servers {
            if let Some(t) = t.as_table() {
                for key in t.keys() {
                    if !KNOWN_MCP_SERVER_KEYS.contains(&key.as_str()) {
                        out.push(format!(
                            "{}: unknown mcp server key 'mcp.servers.{name}.{key}' (ignored)",
                            path.display()
                        ));
                    }
                }
            }
        }
    }
    if let Some(t) = top.get("capabilities").and_then(toml::Value::as_table) {
        for key in t.keys() {
            if !KNOWN_CAPABILITY_KEYS.contains(&key.as_str()) {
                out.push(format!(
                    "{}: unknown capabilities key 'capabilities.{key}' (ignored)",
                    path.display()
                ));
            }
        }
    }
    if let Some(ui) = top.get("ui").and_then(toml::Value::as_table) {
        for key in ui.keys() {
            if !KNOWN_UI_KEYS.contains(&key.as_str()) {
                out.push(format!(
                    "{}: unknown ui key 'ui.{key}' (ignored)",
                    path.display()
                ));
            }
        }
        if let Some(acc) = ui.get("accessibility").and_then(toml::Value::as_table) {
            for key in acc.keys() {
                if !KNOWN_ACCESSIBILITY_KEYS.contains(&key.as_str()) {
                    out.push(format!(
                        "{}: unknown ui.accessibility key 'ui.accessibility.{key}' (ignored)",
                        path.display()
                    ));
                }
            }
        }
        if let Some(themes) = ui.get("themes").and_then(toml::Value::as_table) {
            for (name, t) in themes {
                if let Some(t) = t.as_table() {
                    for key in t.keys() {
                        if !KNOWN_THEME_COLORS.contains(&key.as_str()) {
                            out.push(format!(
                                "{}: unknown theme color 'ui.themes.{name}.{key}' (ignored)",
                                path.display()
                            ));
                        }
                    }
                }
            }
        }
    }
    if let Some(sp) = top.get("stop_policy").and_then(toml::Value::as_table) {
        for key in sp.keys() {
            if !KNOWN_STOP_POLICY_KEYS.contains(&key.as_str()) {
                out.push(format!(
                    "{}: unknown stop_policy key 'stop_policy.{key}' (ignored)",
                    path.display()
                ));
            }
        }
    }
    if let Some(gl) = top.get("goal").and_then(toml::Value::as_table) {
        for key in gl.keys() {
            if !KNOWN_GOAL_KEYS.contains(&key.as_str()) {
                out.push(format!(
                    "{}: unknown goal key 'goal.{key}' (ignored)",
                    path.display()
                ));
            }
        }
    }
    if let Some(fo) = top.get("finops").and_then(toml::Value::as_table) {
        for key in fo.keys() {
            if !KNOWN_FINOPS_KEYS.contains(&key.as_str()) {
                out.push(format!(
                    "{}: unknown finops key 'finops.{key}' (ignored)",
                    path.display()
                ));
            }
        }
    }
    if let Some(work) = top.get("work").and_then(toml::Value::as_table) {
        for key in work.keys() {
            if !KNOWN_WORK_KEYS.contains(&key.as_str()) {
                out.push(format!(
                    "{}: unknown work key 'work.{key}' (ignored)",
                    path.display()
                ));
            }
        }
    }
    if let Some(gw) = top.get("gateway").and_then(toml::Value::as_table) {
        for key in gw.keys() {
            if !KNOWN_GATEWAY_KEYS.contains(&key.as_str()) {
                out.push(format!(
                    "{}: unknown gateway key 'gateway.{key}' (ignored)",
                    path.display()
                ));
            }
        }
        if let Some(ob) = gw.get("outbound").and_then(toml::Value::as_table) {
            for key in ob.keys() {
                if !KNOWN_OUTBOUND_KEYS.contains(&key.as_str()) {
                    out.push(format!(
                        "{}: unknown gateway.outbound key 'gateway.outbound.{key}' (ignored)",
                        path.display()
                    ));
                }
            }
            if let Some(hooks) = ob.get("webhooks").and_then(toml::Value::as_table) {
                for (name, t) in hooks {
                    if let Some(t) = t.as_table() {
                        for key in t.keys() {
                            if !KNOWN_WEBHOOK_KEYS.contains(&key.as_str()) {
                                out.push(format!(
                                    "{}: unknown webhook key 'gateway.outbound.webhooks.{name}.{key}' (ignored)",
                                    path.display()
                                ));
                            }
                        }
                    }
                }
            }
        }
    }
    if let Some(mem) = top.get("memory").and_then(toml::Value::as_table) {
        for key in mem.keys() {
            if !KNOWN_MEMORY_KEYS.contains(&key.as_str()) {
                out.push(format!(
                    "{}: unknown memory key 'memory.{key}' (ignored)",
                    path.display()
                ));
            }
        }
    }
    if let Some(sb) = top.get("sandbox").and_then(toml::Value::as_table) {
        for key in sb.keys() {
            if !KNOWN_SANDBOX_KEYS.contains(&key.as_str()) {
                out.push(format!(
                    "{}: unknown sandbox key 'sandbox.{key}' (ignored)",
                    path.display()
                ));
            }
        }
    }
    if let Some(t) = top.get("automation").and_then(toml::Value::as_table) {
        for key in t.keys() {
            if !KNOWN_AUTOMATION_KEYS.contains(&key.as_str()) {
                out.push(format!(
                    "{}: unknown automation key 'automation.{key}' (ignored)",
                    path.display()
                ));
            }
        }
    }
    if let Some(t) = top.get("update").and_then(toml::Value::as_table) {
        for key in t.keys() {
            if !KNOWN_UPDATE_KEYS.contains(&key.as_str()) {
                out.push(format!(
                    "{}: unknown update key 'update.{key}' (ignored)",
                    path.display()
                ));
            }
        }
    }
    if let Some(t) = top.get("tools").and_then(toml::Value::as_table) {
        for key in t.keys() {
            if !KNOWN_TOOLS_KEYS.contains(&key.as_str()) {
                out.push(format!(
                    "{}: unknown tools key 'tools.{key}' (ignored)",
                    path.display()
                ));
            }
        }
    }
    if let Some(t) = top.get("heartbeat").and_then(toml::Value::as_table) {
        for key in t.keys() {
            if !KNOWN_HEARTBEAT_KEYS.contains(&key.as_str()) {
                out.push(format!(
                    "{}: unknown heartbeat key 'heartbeat.{key}' (ignored)",
                    path.display()
                ));
            }
        }
    }
    if let Some(t) = top.get("plugins").and_then(toml::Value::as_table) {
        for key in t.keys() {
            if !KNOWN_PLUGINS_KEYS.contains(&key.as_str()) {
                out.push(format!(
                    "{}: unknown plugins key 'plugins.{key}' (ignored)",
                    path.display()
                ));
            }
        }
    }
    if let Some(t) = top.get("providers").and_then(toml::Value::as_table) {
        for key in t.keys() {
            if !KNOWN_PROVIDERS_KEYS.contains(&key.as_str()) {
                out.push(format!(
                    "{}: unknown providers key 'providers.{key}' (ignored)",
                    path.display()
                ));
            }
        }
        if let Some(t) = t.get("ollama").and_then(toml::Value::as_table) {
            for key in t.keys() {
                if !KNOWN_OLLAMA_KEYS.contains(&key.as_str()) {
                    out.push(format!(
                        "{}: unknown providers key 'providers.ollama.{key}' (ignored)",
                        path.display()
                    ));
                }
            }
        }
        if let Some(t) = t.get("anthropic").and_then(toml::Value::as_table) {
            for key in t.keys() {
                if !KNOWN_ANTHROPIC_PROVIDER_KEYS.contains(&key.as_str()) {
                    out.push(format!(
                        "{}: unknown providers key 'providers.anthropic.{key}' (ignored)",
                        path.display()
                    ));
                }
            }
        }
        if let Some(t) = t.get("google").and_then(toml::Value::as_table) {
            for key in t.keys() {
                if !KNOWN_GOOGLE_PROVIDER_KEYS.contains(&key.as_str()) {
                    out.push(format!(
                        "{}: unknown providers key 'providers.google.{key}' (ignored)",
                        path.display()
                    ));
                }
            }
        }
    }
    out
}

/// Read `[route] same_model`. A member that does not spell `provider/model`
/// is dropped with a warning, and groups that share a member merge, so the
/// resolved list states each fact once.
fn resolve_same_model(
    groups: &[Vec<String>],
    warnings: &mut Vec<String>,
) -> Vec<Vec<vak_llm::ModelRef>> {
    let mut parsed = Vec::with_capacity(groups.len());
    for group in groups {
        let mut members = Vec::with_capacity(group.len());
        for spelling in group {
            match vak_llm::ModelRef::parse(spelling) {
                Some(member) => members.push(member),
                None => warnings.push(format!(
                    "route.same_model member '{spelling}' is not provider/model; ignored"
                )),
            }
        }
        if members.len() < 2 {
            warnings.push(format!(
                "route.same_model group {group:?} names fewer than two models; ignored"
            ));
            continue;
        }
        parsed.push(members);
    }
    vak_llm::model_identity::merge_groups(parsed)
}

fn merge_into(base: &mut FileConfig, over: FileConfig) {
    if over.provider.is_some() {
        base.provider = over.provider;
    }
    if over.model.is_some() {
        base.model = over.model;
    }
    if over.max_tokens.is_some() {
        base.max_tokens = over.max_tokens;
    }
    if over.max_turns.is_some() {
        base.max_turns = over.max_turns;
    }
    if over.permission_mode.is_some() {
        base.permission_mode = over.permission_mode;
    }
    if over.approval_mode.is_some() {
        base.approval_mode = over.approval_mode;
    }
    if over.profile.is_some() {
        base.profile = over.profile;
    }
    if over.anthropic_base_url.is_some() {
        base.anthropic_base_url = over.anthropic_base_url;
    }
    if over.bedrock_region.is_some() {
        base.bedrock_region = over.bedrock_region;
    }
    for r in over.allow {
        if !base.allow.contains(&r) {
            base.allow.push(r);
        }
    }
    for r in over.ask {
        if !base.ask.contains(&r) {
            base.ask.push(r);
        }
    }
    for r in over.deny {
        if !base.deny.contains(&r) {
            base.deny.push(r);
        }
    }
    if over.workers.is_some() {
        base.workers = over.workers;
    }
    if over.max_retries.is_some() {
        base.max_retries = over.max_retries;
    }
    if over.retry_base_backoff_ms.is_some() {
        base.retry_base_backoff_ms = over.retry_base_backoff_ms;
    }
    if over.request_timeout_secs.is_some() {
        base.request_timeout_secs = over.request_timeout_secs;
    }
    if over.run_retry_attempts.is_some() {
        base.run_retry_attempts = over.run_retry_attempts;
    }
    if over.run_retry_base_backoff_ms.is_some() {
        base.run_retry_base_backoff_ms = over.run_retry_base_backoff_ms;
    }
    if over.circuit_breaker_threshold.is_some() {
        base.circuit_breaker_threshold = over.circuit_breaker_threshold;
    }
    if over.circuit_breaker_cooldown_secs.is_some() {
        base.circuit_breaker_cooldown_secs = over.circuit_breaker_cooldown_secs;
    }
    if over.context_window.is_some() {
        base.context_window = over.context_window;
    }
    // Voice settings are scalar overrides; keep the narrower layer's intent.
    if over.voice.is_some() {
        base.voice = over.voice;
    }
    if over.capabilities.inherit_hooks == Some(false) {
        base.hooks.clear();
    }
    if over.capabilities.inherit_mcp == Some(false) {
        base.mcp.servers.clear();
    }
    if over.capabilities.inherit_mcp.is_some() {
        base.capabilities.inherit_mcp = over.capabilities.inherit_mcp;
    }
    if over.capabilities.inherit_hooks.is_some() {
        base.capabilities.inherit_hooks = over.capabilities.inherit_hooks;
    }
    if over.capabilities.inherit_skills.is_some() {
        base.capabilities.inherit_skills = over.capabilities.inherit_skills;
    }
    if over.capabilities.inherit_commands.is_some() {
        base.capabilities.inherit_commands = over.capabilities.inherit_commands;
    }
    if over.capabilities.inherit_plugins.is_some() {
        base.capabilities.inherit_plugins = over.capabilities.inherit_plugins;
    }
    // Unlike allow/ask/deny just above, this used to be a plain `.extend`
    // with no dedup. `PUT /config/hooks` (vak-server) always resubmitted the
    // full *effective* list it had just read — global layer included — so
    // every hook edit re-wrote the project's own inherited copy of every
    // global hook back into the project file, and the next load merged
    // both: the same hook doubled, then tripled on the next edit, without
    // bound. `get_hooks` no longer echoes inherited hooks for this reason,
    // but a hand-edited config with a genuine duplicate should not compound
    // either.
    for h in over.hooks {
        let key = |hook: &HookConfig| {
            (
                hook.event.clone(),
                hook.matcher.clone(),
                hook.command.clone(),
            )
        };
        let incoming = key(&h);
        if let Some(existing) = base
            .hooks
            .iter_mut()
            .find(|candidate| key(candidate) == incoming)
        {
            *existing = h;
        } else {
            base.hooks.push(h);
        }
    }
    for (name, srv) in over.mcp.servers {
        base.mcp.servers.insert(name, srv);
    }
    if over.ui.theme.is_some() {
        base.ui.theme = over.ui.theme;
    }
    if over.ui.bell.is_some() {
        base.ui.bell = over.ui.bell;
    }
    for (k, v) in &over.ui.keymap {
        base.ui.keymap.insert(k.clone(), v.clone());
    }
    if over.ui.composer.is_some() {
        base.ui.composer = over.ui.composer;
    }
    if over.ui.osc52.is_some() {
        base.ui.osc52 = over.ui.osc52;
    }
    if let Some(acc) = over.ui.accessibility {
        let entry = base
            .ui
            .accessibility
            .get_or_insert_with(AccessibilitySettings::default);
        if acc.plain.is_some() {
            entry.plain = acc.plain;
        }
        if acc.reduced_motion.is_some() {
            entry.reduced_motion = acc.reduced_motion;
        }
        if acc.screen_reader.is_some() {
            entry.screen_reader = acc.screen_reader;
        }
    }
    for (name, colors) in &over.ui.themes {
        let entry = base.ui.themes.entry(name.clone()).or_default();
        for (k, v) in colors {
            entry.insert(k.clone(), v.clone());
        }
    }
    if let Some(sp) = over.stop_policy {
        base.stop_policy = Some(sp);
    }
    if over.gateway.enabled.is_some() {
        base.gateway.enabled = over.gateway.enabled;
    }
    if over.gateway.approvals.is_some() {
        base.gateway.approvals = over.gateway.approvals;
    }
    if over.gateway.approver.is_some() {
        base.gateway.approver = over.gateway.approver;
    }
    if over.gateway.approval_timeout_secs.is_some() {
        base.gateway.approval_timeout_secs = over.gateway.approval_timeout_secs;
    }
    if over.gateway.rate_limit.is_some() {
        base.gateway.rate_limit = over.gateway.rate_limit;
    }
    if !over.gateway.chat_allowlist.is_empty() {
        base.gateway.chat_allowlist = over.gateway.chat_allowlist;
    }
    if over.gateway.chat_allowlist_open.is_some() {
        base.gateway.chat_allowlist_open = over.gateway.chat_allowlist_open;
    }
    if over.gateway.core_pool_max.is_some() {
        base.gateway.core_pool_max = over.gateway.core_pool_max;
    }
    if over.gateway.core_pool_idle_secs.is_some() {
        base.gateway.core_pool_idle_secs = over.gateway.core_pool_idle_secs;
    }
    if over.gateway.pending_expiry_days.is_some() {
        base.gateway.pending_expiry_days = over.gateway.pending_expiry_days;
    }
    if over.memory.search_enabled.is_some() {
        base.memory.search_enabled = over.memory.search_enabled;
    }
    if over.memory.write_enabled.is_some() {
        base.memory.write_enabled = over.memory.write_enabled;
    }
    if over.memory.skill_proposals.is_some() {
        base.memory.skill_proposals = over.memory.skill_proposals;
    }
    if over.memory.reflection.is_some() {
        base.memory.reflection = over.memory.reflection;
    }
    if over.sandbox.backend.is_some() {
        base.sandbox.backend = over.sandbox.backend;
    }
    if over.sandbox.image.is_some() {
        base.sandbox.image = over.sandbox.image;
    }
    if over.finops.max_run_usd.is_some() {
        base.finops.max_run_usd = over.finops.max_run_usd;
    }
    if over.finops.max_day_usd.is_some() {
        base.finops.max_day_usd = over.finops.max_day_usd;
    }
    for (k, v) in over.finops.price_overrides {
        base.finops.price_overrides.insert(k, v);
    }
    if over.goal.handoff_reset.is_some() {
        base.goal.handoff_reset = over.goal.handoff_reset;
    }
    if over.goal.max_audit_blocks.is_some() {
        base.goal.max_audit_blocks = over.goal.max_audit_blocks;
    }
    if over.work.enabled.is_some() {
        base.work.enabled = over.work.enabled;
    }
    if over.work.default_mode.is_some() {
        base.work.default_mode = over.work.default_mode;
    }
    if over.work.max_items.is_some() {
        base.work.max_items = over.work.max_items;
    }
    if over.work.max_revisions.is_some() {
        base.work.max_revisions = over.work.max_revisions;
    }
    if over.work.max_parallel.is_some() {
        base.work.max_parallel = over.work.max_parallel;
    }
    if over.work.confirmation.is_some() {
        base.work.confirmation = over.work.confirmation;
    }
    if over.route.objective.is_some() {
        base.route.objective = over.route.objective;
    }
    for m in over.route.fallback_models {
        if !base.route.fallback_models.contains(&m) {
            base.route.fallback_models.push(m);
        }
    }
    for group in over.route.same_model {
        if !base.route.same_model.contains(&group) {
            base.route.same_model.push(group);
        }
    }
    if over.route.max_fallbacks.is_some() {
        base.route.max_fallbacks = over.route.max_fallbacks;
    }
    if over.probe.hosted.is_some() {
        base.probe.hosted = over.probe.hosted;
    }
    if over.providers.ollama.keep_alive.is_some() {
        base.providers.ollama.keep_alive = over.providers.ollama.keep_alive;
    }
    if over.providers.ollama.num_ctx.is_some() {
        base.providers.ollama.num_ctx = over.providers.ollama.num_ctx;
    }
    if over.providers.anthropic.fast_mode.is_some() {
        base.providers.anthropic.fast_mode = over.providers.anthropic.fast_mode;
    }
    if over.providers.google.project_id.is_some() {
        base.providers.google.project_id = over.providers.google.project_id;
    }
    if over.intent.enabled.is_some() {
        base.intent.enabled = over.intent.enabled;
    }
    if over.intent.accept_confidence.is_some() {
        base.intent.accept_confidence = over.intent.accept_confidence;
    }
    if over.intent.provisional_confidence.is_some() {
        base.intent.provisional_confidence = over.intent.provisional_confidence;
    }
    if over.intent.slice_capabilities.is_some() {
        base.intent.slice_capabilities = over.intent.slice_capabilities;
    }
    if over.intent.posture.is_some() {
        base.intent.posture = over.intent.posture;
    }
    if over.intent.escalate.is_some() {
        base.intent.escalate = over.intent.escalate;
    }
    if over.intent.classify_model.is_some() {
        base.intent.classify_model = over.intent.classify_model;
    }
    if over.intent.max_classify_usd.is_some() {
        base.intent.max_classify_usd = over.intent.max_classify_usd;
    }
    if over.intent.classify_timeout_secs.is_some() {
        base.intent.classify_timeout_secs = over.intent.classify_timeout_secs;
    }
    if over.intent.autonomy.is_some() {
        base.intent.autonomy = over.intent.autonomy;
    }
    if over.intent.evidence_max_age_secs.is_some() {
        base.intent.evidence_max_age_secs = over.intent.evidence_max_age_secs;
    }
    if over.commitment.enabled.is_some() {
        base.commitment.enabled = over.commitment.enabled;
    }
    if over.commitment.lifetime_budget_usd.is_some() {
        base.commitment.lifetime_budget_usd = over.commitment.lifetime_budget_usd;
    }
    if over.commitment.stall_limit.is_some() {
        base.commitment.stall_limit = over.commitment.stall_limit;
    }
    if over.commitment.review_every_hours.is_some() {
        base.commitment.review_every_hours = over.commitment.review_every_hours;
    }
    if over.commitment.default_ttl_days.is_some() {
        base.commitment.default_ttl_days = over.commitment.default_ttl_days;
    }
    for h in over.route.quality_hints {
        if !base.route.quality_hints.contains(&h) {
            base.route.quality_hints.push(h);
        }
    }
    for h in over.route.modality_hints {
        if !base.route.modality_hints.contains(&h) {
            base.route.modality_hints.push(h);
        }
    }
    if over.automation.catch_up_missed.is_some() {
        base.automation.catch_up_missed = over.automation.catch_up_missed;
    }
    if over.update.url.is_some() {
        base.update.url = over.update.url;
    }
    if over.update.interval_hours.is_some() {
        base.update.interval_hours = over.update.interval_hours;
    }
    if over.tools.web_fetch.is_some() {
        base.tools.web_fetch = over.tools.web_fetch;
    }
    if over.tools.browse.is_some() {
        base.tools.browse = over.tools.browse;
    }
    if over.heartbeat.enabled.is_some() {
        base.heartbeat.enabled = over.heartbeat.enabled;
    }
    if over.heartbeat.interval_secs.is_some() {
        base.heartbeat.interval_secs = over.heartbeat.interval_secs;
    }
    if over.heartbeat.model.is_some() {
        base.heartbeat.model = over.heartbeat.model;
    }
    if over.heartbeat.quiet_hours.is_some() {
        base.heartbeat.quiet_hours = over.heartbeat.quiet_hours;
    }
    if over.heartbeat.max_findings.is_some() {
        base.heartbeat.max_findings = over.heartbeat.max_findings;
    }
    // Server exposure is privileged at the layer boundary above. Once a
    // layer is trusted, merge every field so Shared defaults and explicit
    // workspace overrides follow the same contract on every host.
    if over.server.bind.is_some() {
        base.server.bind = over.server.bind;
    }
    if over.server.trusted_hosts.is_some() {
        base.server.trusted_hosts = over.server.trusted_hosts;
    }
    if over.server.public_url.is_some() {
        base.server.public_url = over.server.public_url;
    }
    if over.server.session_ttl_hours.is_some() {
        base.server.session_ttl_hours = over.server.session_ttl_hours;
    }
    if over.server.loopback_auto_login.is_some() {
        base.server.loopback_auto_login = over.server.loopback_auto_login;
    }
    if over.server.workspace_roots.is_some() {
        base.server.workspace_roots = over.server.workspace_roots;
    }
    if over.server.web.terminal.is_some() {
        base.server.web.terminal = over.server.web.terminal;
    }
    if over.server.web.terminal_requires_loopback.is_some() {
        base.server.web.terminal_requires_loopback = over.server.web.terminal_requires_loopback;
    }
    if over.server.bus.nats_url.is_some() {
        base.server.bus.nats_url = over.server.bus.nats_url;
    }
    if over.server.bus.workspace_secret_env.is_some() {
        base.server.bus.workspace_secret_env = over.server.bus.workspace_secret_env;
    }
    for (name, hook) in over.gateway.outbound.webhooks {
        base.gateway.outbound.webhooks.insert(name, hook);
    }
    for p in over.plugins.enabled {
        if !base.plugins.enabled.contains(&p) {
            base.plugins.enabled.push(p);
        }
    }
    for p in over.plugins.disabled {
        if !base.plugins.disabled.contains(&p) {
            base.plugins.disabled.push(p);
        }
    }
    if over.plugins.allow.is_some() {
        base.plugins.allow = over.plugins.allow;
    }
    for p in over.plugins.deny {
        if !base.plugins.deny.contains(&p) {
            base.plugins.deny.push(p);
        }
    }
    if over.plugins.network_allow.is_some() {
        base.plugins.network_allow = over.plugins.network_allow;
    }
    for p in over.plugins.network_deny {
        if !base.plugins.network_deny.contains(&p) {
            base.plugins.network_deny.push(p);
        }
    }
    for (k, v) in over.profiles {
        base.profiles.insert(k, v);
    }
}

/// Process-wide extra environment sourced from .env files. Real
/// environment variables always take precedence.
type ExtraMap = std::collections::BTreeMap<String, String>;

fn dotenv_extra() -> std::sync::MutexGuard<'static, ExtraMap> {
    static EXTRA: std::sync::OnceLock<std::sync::Mutex<ExtraMap>> = std::sync::OnceLock::new();
    EXTRA
        .get_or_init(|| std::sync::Mutex::new(ExtraMap::new()))
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
}

/// Loads a scope's stored secrets (docs/design/44-shared-config.md,
/// "Secrets Chain") into the extra-env table. `path` is a scope hint — the
/// directory that used to hold a literal `.env` file — not a file read
/// directly; see [`credentials`]. Existing real environment variables are
/// never overridden. Only the encrypted-file credential backend can
/// enumerate a scope's contents; an OS-native secret service is reached by
/// point lookup only, so bulk-seeding this cache has no effect there (see
/// `CredentialStore::list`) — callers needing a specific key from that
/// backend should resolve it explicitly instead of relying on this cache.
pub fn load_env_file(path: &std::path::Path) {
    // `credentials::list` can itself take the `dotenv_extra` lock further
    // down (e.g. `EncryptedFileStore::new` reads `VAK_HOME` via `get_var`
    // on first use) — it must run to completion BEFORE this function takes
    // that lock itself, or a thread deadlocks against its own held guard.
    let entries = credentials::list(path);
    let mut extra = dotenv_extra();
    for (key, value) in entries {
        extra.entry(key).or_insert(value);
    }
}

/// Replaces credential-sourced environment values as one atomic scope
/// change. Runtime overrides and real environment variables remain
/// untouched.
pub fn replace_env_files(paths: &[&std::path::Path]) {
    // Same ordering requirement as `load_env_file` above: resolve every
    // scope's entries before touching the `dotenv_extra` lock.
    let entries: Vec<_> = paths
        .iter()
        .flat_map(|path| credentials::list(path))
        .collect();
    let mut extra = dotenv_extra();
    extra.clear();
    for (key, value) in entries {
        extra.entry(key).or_insert(value);
    }
}

/// Environment lookup: runtime overrides first (values set this session,
/// e.g. a key the user just saved), then real env, then loaded .env files.
pub fn get_var(key: &str) -> Option<String> {
    var_overrides()
        .get(key)
        .cloned()
        .or_else(|| std::env::var(key).ok())
        .or_else(|| dotenv_extra().get(key).cloned())
}

fn var_overrides() -> std::sync::MutexGuard<'static, ExtraMap> {
    static OVERRIDES: std::sync::OnceLock<std::sync::Mutex<ExtraMap>> = std::sync::OnceLock::new();
    OVERRIDES
        .get_or_init(|| std::sync::Mutex::new(ExtraMap::new()))
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
}

/// Registers a value with the highest lookup precedence for THIS process.
/// Persistence is the caller's job (see `upsert_env_file`); overrides die
/// with the process, and a real environment variable set after this call
/// still loses to it until the process restarts.
pub fn set_override(key: impl Into<String>, value: impl Into<String>) {
    var_overrides().insert(key.into(), value.into());
}

/// Drops a runtime override so lookups fall back to the real environment
/// and .env files again. Used when a key is revoked mid-session.
pub fn clear_override(key: &str) {
    var_overrides().remove(key);
}

/// Forgets a key loaded from a .env file earlier this session. Without
/// this a revoked key keeps resolving from the in-memory dotenv map.
pub fn forget_dotenv_var(key: &str) {
    dotenv_extra().remove(key);
}

/// Scope hint for the shared secret layer inherited by every workspace
/// (docs/design/44-shared-config.md, "Secrets Chain"). No file is written
/// at this literal path anymore — it only identifies the scope passed to
/// [`credentials`]; kept as a `~/vak-home/.env`-shaped path so existing
/// callers' scoping (same directory = same layer) is unchanged.
pub fn user_env_path() -> Option<std::path::PathBuf> {
    Some(crate::paths::default_workspace().join(".env"))
}

/// Reads one credential from a specific scope without merging it into the
/// process-wide environment cache. Scoped MCP credentials use this so two
/// pooled workspaces can resolve different values for the same variable.
/// `path` is a scope hint (previously a literal `.env` path), not a file
/// read directly — see [`credentials`].
pub fn read_env_file_var(path: &std::path::Path, key: &str) -> Option<String> {
    credentials::get(path, key)
}

/// Removes `key` from the scope named by `path`. A missing scope or key is
/// a no-op success.
pub fn remove_env_file_key(path: &std::path::Path, key: &str) -> std::io::Result<()> {
    credentials::remove(path, key)
}

/// Upserts `key = value` into the scope named by `path`, persisted through
/// whichever [`credentials::CredentialStore`] backend this host resolved
/// (OS-native secret service, or the encrypted-file fallback) — never as
/// plaintext.
pub fn upsert_env_file(path: &std::path::Path, key: &str, value: &str) -> std::io::Result<()> {
    credentials::set(path, key, value)
}

#[cfg(test)]
#[allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::field_reassign_with_default
)]
mod tests {
    #[test]
    fn voice_defaults_are_safe_and_serializable() {
        let settings = VoiceSettings::default();
        assert!(!settings.enabled);
        assert_eq!(settings.max_session_secs, 900);
        assert_eq!(settings.max_concurrent, 2);
        assert_eq!(settings.max_audio_bytes, 16 * 1024 * 1024);
        assert_eq!(settings.max_requests_per_minute, 60);
        assert_eq!(settings.max_text_chars, 100_000);
        assert!(settings.validate().is_ok());
        let encoded = toml::to_string(&settings).expect("voice settings serialize");
        let decoded: VoiceSettings = toml::from_str(&encoded).expect("voice settings deserialize");
        assert_eq!(decoded, settings);
        let routed = VoiceSettings {
            provider: Some("openai".into()),
            transcription_model: Some("stt-v1".into()),
            ..settings
        };
        let encoded = toml::to_string(&routed).expect("routed voice settings serialize");
        let decoded: VoiceSettings =
            toml::from_str(&encoded).expect("routed voice settings deserialize");
        assert_eq!(decoded.provider.as_deref(), Some("openai"));
        assert_eq!(decoded.transcription_model.as_deref(), Some("stt-v1"));
        assert!(decoded.validate().is_ok());
    }

    #[test]
    fn ollama_settings_validate_accepts_go_style_durations() {
        for keep_alive in ["10m", "24h", "0", "1h30m", "500ms", "90s"] {
            let settings = OllamaSettings {
                keep_alive: Some(keep_alive.to_string()),
                num_ctx: Some(4096),
            };
            assert!(settings.validate().is_ok(), "{keep_alive} should be valid");
        }
    }

    #[test]
    fn ollama_settings_validate_rejects_malformed_duration_and_small_num_ctx() {
        let settings = OllamaSettings {
            keep_alive: Some("forever".into()),
            num_ctx: None,
        };
        assert!(settings.validate().is_err());

        let settings = OllamaSettings {
            keep_alive: None,
            num_ctx: Some(1023),
        };
        assert!(settings.validate().is_err());

        let settings = OllamaSettings {
            keep_alive: None,
            num_ctx: Some(1024),
        };
        assert!(settings.validate().is_ok());
    }

    #[test]
    fn voice_validation_rejects_zero_and_excessive_limits() {
        let mut settings = VoiceSettings::default();
        settings.max_concurrent = 0;
        assert!(settings.validate().is_err());
        settings = VoiceSettings::default();
        settings.max_audio_bytes = 512 * 1024 * 1024;
        assert!(settings.validate().is_err());
        settings = VoiceSettings::default();
        settings.max_requests_per_minute = 0;
        assert!(settings.validate().is_err());
        settings = VoiceSettings::default();
        settings.max_text_chars = 0;
        assert!(settings.validate().is_err());
        settings = VoiceSettings {
            provider: Some(" ".into()),
            ..VoiceSettings::default()
        };
        assert!(settings.validate().is_err());
        settings = VoiceSettings {
            synthesis_model: Some("x".repeat(257)),
            ..VoiceSettings::default()
        };
        assert!(settings.validate().is_err());
    }

    #[test]
    fn voice_patch_sets_clears_and_preserves_keys() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.toml");
        std::fs::write(
            &path,
            "[voice]\nprovider = \"gemini\"\nsynthesis_model = \"tts-a\"\nfuture_key = 7\n",
        )
        .unwrap();
        persist_voice_settings_at(
            path.clone(),
            &VoicePatch {
                enabled: Some(true),
                transcription_model: Some(Some("stt-a".into())),
                synthesis_model: Some(None),
                ..VoicePatch::default()
            },
        )
        .unwrap();
        let value: toml::Value = toml::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
        let voice = value["voice"].as_table().unwrap();
        assert_eq!(voice["enabled"].as_bool(), Some(true));
        assert_eq!(voice["provider"].as_str(), Some("gemini"));
        assert_eq!(voice["transcription_model"].as_str(), Some("stt-a"));
        assert!(!voice.contains_key("synthesis_model"));
        assert_eq!(voice["future_key"].as_integer(), Some(7));
        assert!(VoicePatch::default().is_empty());
    }
    use super::*;

    #[test]
    fn ensure_project_config_creates_inheriting_layer_without_overwriting_it() {
        let project = tempfile::tempdir().unwrap();
        let path = ensure_project_config(project.path()).unwrap();
        assert_eq!(path, project.path().join(".vak/config.toml"));
        assert_eq!(
            std::fs::read_to_string(&path).unwrap(),
            "# Project-local overrides. Unset values inherit from the user config.\n"
        );

        std::fs::write(&path, "provider = \"ollama\"\n").unwrap();
        ensure_project_config(project.path()).unwrap();
        assert_eq!(
            std::fs::read_to_string(path).unwrap(),
            "provider = \"ollama\"\n"
        );
    }

    #[test]
    fn evidence_policy_writer_preserves_other_layers_and_clamps_negative() {
        let project = tempfile::tempdir().unwrap();
        let path = project.path().join(".vak/config.toml");
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(&path, "provider = \"ollama\"\n\n[ui]\ntheme = \"dark\"\n").unwrap();
        persist_evidence_max_age(path.clone(), -5).unwrap();
        let text = std::fs::read_to_string(path).unwrap();
        assert!(text.contains("provider = \"ollama\""));
        assert!(text.contains("theme = \"dark\""));
        assert!(text.contains("evidence_max_age_secs = 0"));
    }

    #[test]
    fn evidence_policy_resolves_from_project_layer() {
        let project = tempfile::tempdir().unwrap();
        let path = project.path().join(".vak/config.toml");
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(&path, "[intent]\nevidence_max_age_secs = 120\n").unwrap();
        let config = load_with_trust(project.path(), true).unwrap();
        assert_eq!(config.intent.evidence_max_age_secs, 120);
        assert!(config.intent.enabled);
    }

    /// `capped_by` is the single arithmetic the gateway's per-channel
    /// permission override rests on: it must be a true `min` over the
    /// permissiveness ranking, in both argument orders, for every pair.
    /// The whole point of a scoped writer: it must not disturb keys it was
    /// not asked about. This one had no writer at all, so the only way to
    /// set it was hand-editing the file — and hand-editing is exactly what
    /// loses the rest of the document to a typo.
    #[test]
    fn writing_gateway_approvals_leaves_every_other_key_alone() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.toml");
        std::fs::write(
            &path,
            "provider = \"anthropic\"\npermission_mode = \"read-only\"\n\n             [gateway]\nenabled = true\n\n[mcp.servers.thing]\ncommand = \"sh\"\n",
        )
        .unwrap();

        persist_gateway_approvals(
            path.clone(),
            Some("forward"),
            Some(Some("telegram:42")),
            Some(120),
        )
        .unwrap();

        let text = std::fs::read_to_string(&path).unwrap();
        let parsed: toml::Value = toml::from_str(&text).unwrap();
        let gw = parsed.get("gateway").unwrap();
        assert_eq!(gw.get("approvals").unwrap().as_str(), Some("forward"));
        assert_eq!(gw.get("approver").unwrap().as_str(), Some("telegram:42"));
        assert_eq!(
            gw.get("approval_timeout_secs").unwrap().as_integer(),
            Some(120)
        );
        assert_eq!(gw.get("enabled").unwrap().as_bool(), Some(true));
        assert_eq!(parsed.get("provider").unwrap().as_str(), Some("anthropic"));
        assert!(parsed.get("mcp").is_some(), "unrelated tables survive");
    }

    /// `Some(None)` clears the target, so returning to `deny` cannot leave a
    /// stale chat behind for a later `forward` to reuse silently.
    #[test]
    fn clearing_the_approver_removes_the_key() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.toml");
        persist_gateway_approvals(
            path.clone(),
            Some("forward"),
            Some(Some("telegram:1")),
            None,
        )
        .unwrap();
        persist_gateway_approvals(path.clone(), Some("deny"), Some(None), None).unwrap();

        let parsed: toml::Value = toml::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
        let gw = parsed.get("gateway").unwrap();
        assert_eq!(gw.get("approvals").unwrap().as_str(), Some("deny"));
        assert!(gw.get("approver").is_none());
    }

    /// Absent means "leave alone"; present replaces the list wholesale, so
    /// an empty vec is how a list is cleared.
    #[test]
    fn writing_rule_lists_replaces_only_the_lists_named() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.toml");
        std::fs::write(&path, "allow = [\"Bash(git *)\"]\nask = [\"Edit(~/**)\"]\n").unwrap();

        persist_permission_rules(path.clone(), None, None, Some(&["Bash(rm *)".to_string()]))
            .unwrap();
        let cfg: FileConfig = toml::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
        assert_eq!(cfg.allow, vec!["Bash(git *)"], "untouched");
        assert_eq!(cfg.ask, vec!["Edit(~/**)"], "untouched");
        assert_eq!(cfg.deny, vec!["Bash(rm *)"]);

        persist_permission_rules(path.clone(), Some(&[]), None, None).unwrap();
        let cfg: FileConfig = toml::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
        assert!(cfg.allow.is_empty(), "an empty list clears");
        assert_eq!(cfg.deny, vec!["Bash(rm *)"], "and still nothing else moved");
    }

    #[test]
    fn capped_by_is_min_over_permissiveness_and_never_escalates() {
        use PermissionMode::*;
        let all = [ReadOnly, WorkspaceWrite, FullAccess];
        assert!(ReadOnly.rank() < WorkspaceWrite.rank());
        assert!(WorkspaceWrite.rank() < FullAccess.rank());
        for requested in all {
            for ceiling in all {
                let got = requested.capped_by(ceiling);
                // Never more permissive than the ceiling — the whole point.
                assert!(
                    got.rank() <= ceiling.rank(),
                    "{requested:?} capped by {ceiling:?} escalated to {got:?}"
                );
                // Never more permissive than what was asked for either.
                assert!(got.rank() <= requested.rank());
                // And it is exactly the min, not an over-eager clamp.
                assert_eq!(got.rank(), requested.rank().min(ceiling.rank()));
            }
        }
        // Concrete spot checks of the security-relevant direction.
        assert_eq!(FullAccess.capped_by(ReadOnly), ReadOnly);
        assert_eq!(FullAccess.capped_by(WorkspaceWrite), WorkspaceWrite);
        assert_eq!(ReadOnly.capped_by(FullAccess), ReadOnly);
        assert_eq!(WorkspaceWrite.capped_by(WorkspaceWrite), WorkspaceWrite);
    }

    /// `ChannelPolicy::merge` is the bot→chat fold used at dispatch
    /// (`core_for_entry`): the chat's own `_allow` wins when set, denies
    /// concatenate, and an unset chat field falls back to the bot's.
    #[test]
    fn channel_policy_merge_lets_chat_allow_win_and_denies_accumulate() {
        let bot = ChannelPolicy {
            tools_allow: Some(vec!["read".into(), "write".into()]),
            tools_deny: vec!["shell".into()],
            ..ChannelPolicy::default()
        };
        let chat = ChannelPolicy {
            tools_allow: Some(vec!["read".into()]),
            tools_deny: vec!["fetch".into()],
            ..ChannelPolicy::default()
        };
        let merged = ChannelPolicy::merge(&bot, &chat);
        // Chat's narrower allow list wins outright.
        assert_eq!(merged.tools_allow, Some(vec!["read".to_string()]));
        // Denies from both tiers accumulate — restrictive-only.
        assert_eq!(
            merged.tools_deny,
            vec!["shell".to_string(), "fetch".to_string()]
        );
    }

    #[test]
    fn channel_policy_merge_falls_back_to_bot_when_chat_is_unset() {
        let bot = ChannelPolicy {
            mcp_allow: Some(vec!["search/*".into()]),
            ..ChannelPolicy::default()
        };
        let chat = ChannelPolicy::default();
        let merged = ChannelPolicy::merge(&bot, &chat);
        assert_eq!(merged.mcp_allow, Some(vec!["search/*".to_string()]));
    }

    #[test]
    fn channel_policy_merge_of_two_defaults_is_default() {
        assert_eq!(
            ChannelPolicy::merge(&ChannelPolicy::default(), &ChannelPolicy::default()),
            ChannelPolicy::default()
        );
    }

    fn write_project_config(dir: &Path, text: &str) {
        let project = dir.join(".vak");
        std::fs::create_dir_all(&project).expect("project dir");
        std::fs::write(project.join("config.toml"), text).expect("write config");
    }

    #[test]
    fn automation_update_tools_default_when_absent() {
        let dir = tempfile::tempdir().unwrap();
        let cfg = load_with_trust(dir.path(), true).unwrap();
        assert!(cfg.automation.catch_up_missed);
        assert_eq!(cfg.update.url, None);
        assert_eq!(cfg.update.interval_hours, 24);
        assert!(cfg.tools.web_fetch);
        assert!(cfg.tools.browse);
        assert!(
            !cfg.warnings.iter().any(|w| w.contains("automation")),
            "absent sections must not warn: {:?}",
            cfg.warnings
        );
    }

    #[test]
    fn work_settings_parse_and_unknown_keys_warn() {
        let dir = tempfile::tempdir().unwrap();
        write_project_config(
            dir.path(),
            "[work]\nenabled = true\ndefault_mode = \"managed\"\nmax_items = 12\nmax_revisions = 5\nmax_parallel = 3\nconfirmation = \"always\"\nunknown = true\n",
        );
        let cfg = load_with_trust(dir.path(), true).unwrap();
        assert!(cfg.work.enabled);
        assert_eq!(cfg.work.default_mode, "managed");
        assert_eq!(cfg.work.max_items, 12);
        assert_eq!(cfg.work.max_revisions, 5);
        assert_eq!(cfg.work.max_parallel, 3);
        assert_eq!(cfg.work.confirmation, "always");
        assert!(
            cfg.warnings
                .iter()
                .any(|warning| warning.contains("work.unknown"))
        );
    }

    #[test]
    fn intent_section_parses_and_clamps() {
        let dir = tempfile::tempdir().unwrap();
        write_project_config(
            dir.path(),
            "[intent]\nenabled = true\naccept_confidence = 0.9\n\
             provisional_confidence = 0.99\nslice_capabilities = false\n\
             escalate = \"local\"\nautonomy = \"delegated\"\n",
        );
        let cfg = load_with_trust(dir.path(), true).unwrap();
        assert!(cfg.intent.enabled);
        assert_eq!(cfg.intent.accept_confidence, 0.9);
        // Inverted thresholds clamp rather than break the ordering invariant.
        assert!(cfg.intent.provisional_confidence <= cfg.intent.accept_confidence);
        assert!(!cfg.intent.slice_capabilities);
        assert_eq!(cfg.intent.escalate, "local");
        assert_eq!(cfg.intent.autonomy, "delegated");
    }

    #[test]
    fn unknown_intent_values_warn_and_fall_back_to_the_safe_default() {
        let dir = tempfile::tempdir().unwrap();
        write_project_config(
            dir.path(),
            "[intent]\nescalate = \"telepathy\"\nautonomy = \"unlimited\"\n",
        );
        let cfg = load_with_trust(dir.path(), true).unwrap();
        assert_eq!(cfg.intent.escalate, "none");
        assert_eq!(cfg.intent.autonomy, "assisted");
        assert!(cfg.warnings.iter().any(|w| w.contains("intent.escalate")));
        assert!(cfg.warnings.iter().any(|w| w.contains("intent.autonomy")));
    }

    /// A cloned repository must not be able to grant itself the right to act
    /// without asking, nor to spend the user's credentials classifying.
    #[test]
    fn an_untrusted_project_cannot_grant_itself_autonomy_or_a_paid_classifier() {
        let dir = tempfile::tempdir().unwrap();
        write_project_config(
            dir.path(),
            "[intent]\nautonomy = \"autonomous\"\nescalate = \"cloud\"\n\
             slice_capabilities = false\n",
        );
        let cfg = load_with_trust(dir.path(), false).unwrap();
        assert_eq!(cfg.intent.autonomy, "assisted");
        assert_eq!(cfg.intent.escalate, "none");
        // The non-privileged half of the section still applies: choosing to
        // see more of your own tools grants nothing.
        assert!(!cfg.intent.slice_capabilities);

        // And with trust, the same file does take effect.
        let trusted = load_with_trust(dir.path(), true).unwrap();
        assert_eq!(trusted.intent.autonomy, "autonomous");
        assert_eq!(trusted.intent.escalate, "cloud");
    }

    /// Switching the kernel or its posture off removes the approval floor it
    /// raises for irreversible work; a cloned repository may not do that to
    /// itself, and a trusted one still may.
    #[test]
    fn an_untrusted_project_cannot_switch_the_intent_floor_off() {
        let dir = tempfile::tempdir().unwrap();
        write_project_config(dir.path(), "[intent]\nenabled = false\nposture = false\n");
        let cfg = load_with_trust(dir.path(), false).unwrap();
        assert!(cfg.intent.enabled);
        assert!(cfg.intent.posture);
        let trusted = load_with_trust(dir.path(), true).unwrap();
        assert!(!trusted.intent.enabled);
        assert!(!trusted.intent.posture);
    }

    /// TOML can spell `nan` and `inf`. A threshold or a cap every comparison
    /// is false against would silently switch its check off, so a
    /// non-finite value falls back to the default with a warning.
    #[test]
    fn non_finite_intent_numbers_fall_back_to_their_defaults() {
        let dir = tempfile::tempdir().unwrap();
        write_project_config(
            dir.path(),
            "[intent]\naccept_confidence = nan\nprovisional_confidence = inf\n\
             max_classify_usd = nan\n[commitment]\nlifetime_budget_usd = nan\n",
        );
        let cfg = load_with_trust(dir.path(), true).unwrap();
        assert_eq!(cfg.intent.accept_confidence, 0.75);
        assert_eq!(cfg.intent.provisional_confidence, 0.45);
        assert_eq!(cfg.intent.max_classify_usd, 0.01);
        assert_eq!(cfg.commitment.lifetime_budget_usd, None);
        assert!(
            cfg.warnings
                .iter()
                .any(|w| w.contains("intent.accept_confidence")),
            "{:?}",
            cfg.warnings
        );
    }

    #[test]
    fn an_untrusted_project_may_still_restrict_itself_to_a_local_classifier() {
        let dir = tempfile::tempdir().unwrap();
        write_project_config(dir.path(), "[intent]\nescalate = \"local\"\n");
        let cfg = load_with_trust(dir.path(), false).unwrap();
        assert_eq!(cfg.intent.escalate, "local");
    }

    #[test]
    fn commitment_section_parses() {
        let dir = tempfile::tempdir().unwrap();
        write_project_config(
            dir.path(),
            "[commitment]\nlifetime_budget_usd = 25.0\nstall_limit = 5\n\
             review_every_hours = 24\ndefault_ttl_days = 30\n",
        );
        let cfg = load_with_trust(dir.path(), true).unwrap();
        assert_eq!(cfg.commitment.lifetime_budget_usd, Some(25.0));
        assert_eq!(cfg.commitment.stall_limit, 5);
        assert_eq!(cfg.commitment.review_every_hours, Some(24));
        assert_eq!(cfg.commitment.default_ttl_days, Some(30));
    }

    /// The kernel must be switchable off entirely, because "reproduce the old
    /// behaviour exactly" has to remain one line of config.
    #[test]
    fn the_intent_kernel_can_be_switched_off() {
        let dir = tempfile::tempdir().unwrap();
        write_project_config(dir.path(), "[intent]\nenabled = false\n");
        assert!(!load_with_trust(dir.path(), true).unwrap().intent.enabled);
    }

    #[test]
    fn automation_catch_up_missed_parses() {
        let dir = tempfile::tempdir().unwrap();
        write_project_config(dir.path(), "[automation]\ncatch_up_missed = false\n");
        let cfg = load_with_trust(dir.path(), true).unwrap();
        assert!(!cfg.automation.catch_up_missed);
    }

    #[test]
    fn update_url_and_interval_hours_parse() {
        let dir = tempfile::tempdir().unwrap();
        write_project_config(
            dir.path(),
            "[update]\nurl = \"https://example.com/manifest.json\"\ninterval_hours = 6\n",
        );
        let cfg = load_with_trust(dir.path(), true).unwrap();
        assert_eq!(
            cfg.update.url.as_deref(),
            Some("https://example.com/manifest.json")
        );
        assert_eq!(cfg.update.interval_hours, 6);
    }

    #[test]
    fn tools_web_fetch_switch_parses() {
        let dir = tempfile::tempdir().unwrap();
        write_project_config(dir.path(), "[tools]\nweb_fetch = false\n");
        let cfg = load_with_trust(dir.path(), true).unwrap();
        assert!(!cfg.tools.web_fetch);
    }

    #[test]
    fn tools_browse_switch_parses() {
        let dir = tempfile::tempdir().unwrap();
        write_project_config(dir.path(), "[tools]\nbrowse = false\n");
        let cfg = load_with_trust(dir.path(), true).unwrap();
        assert!(!cfg.tools.browse);
        let dir2 = tempfile::tempdir().unwrap();
        write_project_config(dir2.path(), "[tools]\nbrowse = true\nweb_fetch = false\n");
        let cfg2 = load_with_trust(dir2.path(), true).unwrap();
        assert!(cfg2.tools.browse);
        assert!(!cfg2.tools.web_fetch);
    }

    #[test]
    fn unknown_keys_in_new_sections_warn() {
        let dir = tempfile::tempdir().unwrap();
        write_project_config(
            dir.path(),
            "[automation]\nbogus = 1\n\n[update]\nbogus = 1\n\n[tools]\nbogus = 1\n",
        );
        let cfg = load_with_trust(dir.path(), true).unwrap();
        for key in ["automation.bogus", "update.bogus", "tools.bogus"] {
            assert!(
                cfg.warnings.iter().any(|w| w.contains(key)),
                "unknown {key} must warn: {:?}",
                cfg.warnings
            );
        }
        assert!(cfg.automation.catch_up_missed);
        assert_eq!(cfg.update.interval_hours, 24);
        assert!(cfg.tools.web_fetch);
    }

    #[test]
    fn heartbeat_defaults_when_absent() {
        let dir = tempfile::tempdir().unwrap();
        let cfg = load_with_trust(dir.path(), true).unwrap();
        assert!(!cfg.heartbeat.enabled);
        assert_eq!(cfg.heartbeat.interval_secs, 1800);
        assert_eq!(cfg.heartbeat.model, None);
        assert_eq!(cfg.heartbeat.quiet_hours, None);
        assert_eq!(cfg.heartbeat.max_findings, 3);
        assert!(
            !cfg.warnings.iter().any(|w| w.contains("heartbeat")),
            "absent heartbeat section must not warn: {:?}",
            cfg.warnings
        );
    }

    #[test]
    fn heartbeat_full_section_parses() {
        let dir = tempfile::tempdir().unwrap();
        write_project_config(
            dir.path(),
            "[heartbeat]\nenabled = true\ninterval_secs = 900\nmodel = \"openai/gpt-5-mini\"\nquiet_hours = \"22:00-07:00\"\nmax_findings = 5\n",
        );
        let cfg = load_with_trust(dir.path(), true).unwrap();
        assert!(cfg.heartbeat.enabled);
        assert_eq!(cfg.heartbeat.interval_secs, 900);
        assert_eq!(cfg.heartbeat.model.as_deref(), Some("openai/gpt-5-mini"));
        assert_eq!(
            cfg.heartbeat.quiet_hours,
            Some(QuietWindow {
                start_min: 22 * 60,
                end_min: 7 * 60
            })
        );
        assert_eq!(cfg.heartbeat.max_findings, 5);
    }

    #[test]
    fn heartbeat_interval_below_minimum_warns_and_clamps() {
        let dir = tempfile::tempdir().unwrap();
        write_project_config(dir.path(), "[heartbeat]\ninterval_secs = 10\n");
        let cfg = load_with_trust(dir.path(), true).unwrap();
        assert_eq!(cfg.heartbeat.interval_secs, 300);
        assert!(cfg.warnings.iter().any(|w| w.contains("interval_secs")));
    }

    #[test]
    fn heartbeat_zero_max_findings_clamps_with_warning() {
        let dir = tempfile::tempdir().unwrap();
        write_project_config(dir.path(), "[heartbeat]\nmax_findings = 0\n");
        let cfg = load_with_trust(dir.path(), true).unwrap();
        assert_eq!(cfg.heartbeat.max_findings, 1);
        assert!(cfg.warnings.iter().any(|w| w.contains("max_findings")));
    }

    #[test]
    fn heartbeat_bad_quiet_hours_warns_and_ignores() {
        for bad in [
            "9am-5pm",
            "25:00-07:00",
            "07:00-07:00",
            "22:00",
            "22:61-07:00",
        ] {
            let dir = tempfile::tempdir().unwrap();
            write_project_config(
                dir.path(),
                &format!("[heartbeat]\nquiet_hours = \"{bad}\"\n"),
            );
            let cfg = load_with_trust(dir.path(), true).unwrap();
            assert_eq!(cfg.heartbeat.quiet_hours, None, "{bad} must be ignored");
            assert!(
                cfg.warnings.iter().any(|w| w.contains("quiet_hours")),
                "{bad} must warn: {:?}",
                cfg.warnings
            );
        }
    }

    #[test]
    fn quiet_window_contains_boundary_matrix() {
        // Wrapping window 22:00-07:00.
        let night = QuietWindow {
            start_min: 22 * 60,
            end_min: 7 * 60,
        };
        assert!(night.contains(22 * 60), "start bound inclusive");
        assert!(night.contains(23 * 60 + 59));
        assert!(night.contains(0));
        assert!(night.contains(6 * 60 + 59));
        assert!(!night.contains(7 * 60), "end bound exclusive");
        assert!(!night.contains(21 * 60 + 59));
        assert!(!night.contains(12 * 60));

        // Plain window 13:00-14:00.
        let lunch = QuietWindow {
            start_min: 13 * 60,
            end_min: 14 * 60,
        };
        assert!(lunch.contains(13 * 60));
        assert!(lunch.contains(13 * 60 + 59));
        assert!(!lunch.contains(14 * 60), "end bound exclusive");
        assert!(!lunch.contains(12 * 60 + 59));
    }

    #[test]
    fn unknown_heartbeat_key_warns() {
        let dir = tempfile::tempdir().unwrap();
        write_project_config(dir.path(), "[heartbeat]\nbogus = 1\n");
        let cfg = load_with_trust(dir.path(), true).unwrap();
        assert!(
            cfg.warnings.iter().any(|w| w.contains("heartbeat.bogus")),
            "{:?}",
            cfg.warnings
        );
        assert!(!cfg.heartbeat.enabled);
    }

    #[test]
    fn persisted_preferences_preserve_unrelated_project_config() {
        let dir = tempfile::tempdir().unwrap();
        write_project_config(
            dir.path(),
            "deny = [\"bash\"]\n[route]\nobjective = \"quality-critical\"\n",
        );
        persist_project_preferences(
            dir.path(),
            Some("google"),
            Some("gemini-test"),
            Some(17),
            Some(PermissionMode::WorkspaceWrite),
            None,
            Some("dark"),
        )
        .unwrap();
        let cfg = load_with_trust(dir.path(), true).unwrap();
        assert_eq!(cfg.provider, "google");
        assert_eq!(cfg.model, "gemini-test");
        assert_eq!(cfg.max_turns, 17);
        assert_eq!(cfg.permission_mode, PermissionMode::WorkspaceWrite);
        assert_eq!(cfg.ui.theme, "dark");
        assert_eq!(cfg.deny, vec!["bash"]);
        assert_eq!(cfg.route.objective, "quality-critical");
    }

    #[test]
    fn route_same_model_parses_merges_and_warns() {
        crate::paths::isolate_home_for_tests();
        let dir = tempfile::tempdir().unwrap();
        write_project_config(
            dir.path(),
            "[route]\n\
             same_model = [\n\
               [\"anthropic/model-a\", \"openrouter/vendor/model-a\"],\n\
               [\"bedrock/us.vendor.model-a-v1:0\", \"openrouter/vendor/model-a\"],\n\
               [\"no-provider\", \"google/model-b\"],\n\
             ]\n\
             quality_hints = [\"model-a\"]\n\
             modality_hints = [\"vision\"]\n",
        );
        let cfg = load_with_trust(dir.path(), true).unwrap();
        let spelled: Vec<Vec<String>> = cfg
            .route
            .same_model
            .iter()
            .map(|group| group.iter().map(vak_llm::ModelRef::spelling).collect())
            .collect();
        assert_eq!(
            spelled,
            vec![vec![
                "anthropic/model-a",
                "bedrock/us.vendor.model-a-v1:0",
                "openrouter/vendor/model-a",
            ]],
            "groups sharing a member merge into one"
        );
        assert!(
            cfg.warnings
                .iter()
                .any(|w| w.contains("'no-provider' is not provider/model")),
            "{:?}",
            cfg.warnings
        );
        assert!(
            cfg.warnings
                .iter()
                .any(|w| w.contains("fewer than two models")),
            "{:?}",
            cfg.warnings
        );
        assert_eq!(cfg.route.quality_hints, vec!["model-a"]);
        assert_eq!(cfg.route.modality_hints, vec!["vision"]);
    }

    #[test]
    fn route_lists_are_unions_across_layers() {
        let group = |a: &str, b: &str| vec![a.to_string(), b.to_string()];
        let mut base = FileConfig {
            route: RouteSettings {
                fallback_models: vec!["shared-backup".into()],
                same_model: vec![group("p/x", "q/x")],
                quality_hints: vec!["shared".into()],
                ..RouteSettings::default()
            },
            ..FileConfig::default()
        };
        let over = FileConfig {
            route: RouteSettings {
                fallback_models: vec!["agent-backup".into(), "shared-backup".into()],
                same_model: vec![group("p/x", "q/x"), group("r/y", "s/y")],
                quality_hints: vec!["agent".into()],
                modality_hints: vec!["vision".into()],
                ..RouteSettings::default()
            },
            ..FileConfig::default()
        };
        merge_into(&mut base, over);
        assert_eq!(
            base.route.fallback_models,
            vec!["shared-backup", "agent-backup"]
        );
        assert_eq!(
            base.route.same_model,
            vec![group("p/x", "q/x"), group("r/y", "s/y")]
        );
        assert_eq!(base.route.quality_hints, vec!["shared", "agent"]);
        assert_eq!(base.route.modality_hints, vec!["vision"]);
    }

    #[test]
    fn route_backups_persist_to_one_layer_and_clear_in_place() {
        crate::paths::isolate_home_for_tests();
        let dir = tempfile::tempdir().unwrap();
        write_project_config(
            dir.path(),
            "deny = [\"bash\"]\n[route]\nobjective = \"utility\"\n",
        );
        let path = project_path(dir.path());
        let group = vec![
            vak_llm::ModelRef::parse("anthropic/model-a").unwrap(),
            vak_llm::ModelRef::parse("openrouter/vendor/model-a").unwrap(),
        ];
        persist_route_backups_at(
            path.clone(),
            Some(std::slice::from_ref(&group)),
            Some(&["backup-model".to_string()]),
        )
        .unwrap();
        let cfg = load_with_trust(dir.path(), true).unwrap();
        assert_eq!(cfg.route.same_model, vec![group.clone()]);
        assert_eq!(cfg.route.fallback_models, vec!["backup-model"]);
        assert_eq!(cfg.route.objective, "utility");
        assert_eq!(cfg.deny, vec!["bash"]);

        persist_route_backups_at(path.clone(), None, Some(&[])).unwrap();
        let cfg = load_with_trust(dir.path(), true).unwrap();
        assert_eq!(
            cfg.route.same_model,
            vec![group],
            "absent leaves a list alone"
        );
        assert!(cfg.route.fallback_models.is_empty());
        let text = std::fs::read_to_string(&path).unwrap();
        assert!(
            !text.contains("fallback_models"),
            "an empty list removes the key:\n{text}"
        );
    }

    #[test]
    fn probe_hosted_defaults_to_none_and_accepts_full() {
        let dir = tempfile::tempdir().unwrap();
        let cfg = load_with_trust(dir.path(), true).unwrap();
        assert_eq!(cfg.probe.hosted, "none");

        let dir = tempfile::tempdir().unwrap();
        write_project_config(dir.path(), "[probe]\nhosted = \"full\"\n");
        let cfg = load_with_trust(dir.path(), true).unwrap();
        assert_eq!(cfg.probe.hosted, "full");
        assert!(cfg.warnings.is_empty(), "{:?}", cfg.warnings);

        let dir = tempfile::tempdir().unwrap();
        write_project_config(dir.path(), "[probe]\nhosted = \"bogus\"\n");
        let cfg = load_with_trust(dir.path(), true).unwrap();
        assert_eq!(cfg.probe.hosted, "none");
        assert!(cfg.warnings.iter().any(|w| w.contains("probe.hosted")));
    }

    #[test]
    fn ollama_settings_default_to_thirty_minute_keep_alive_and_no_num_ctx() {
        let dir = tempfile::tempdir().unwrap();
        let cfg = load_with_trust(dir.path(), true).unwrap();
        assert_eq!(cfg.ollama.keep_alive, "30m");
        assert_eq!(cfg.ollama.num_ctx, None);
    }

    #[test]
    fn ollama_settings_parse_keep_alive_and_num_ctx() {
        let dir = tempfile::tempdir().unwrap();
        write_project_config(
            dir.path(),
            "[providers.ollama]\nkeep_alive = \"10m\"\nnum_ctx = 8192\n",
        );
        let cfg = load_with_trust(dir.path(), true).unwrap();
        assert_eq!(cfg.ollama.keep_alive, "10m");
        assert_eq!(cfg.ollama.num_ctx, Some(8192));
        assert!(cfg.warnings.is_empty(), "{:?}", cfg.warnings);
    }

    #[test]
    fn ollama_settings_accept_hour_and_zero_durations() {
        for (input, expected) in [("24h", "24h"), ("0", "0"), ("1h30m", "1h30m")] {
            let dir = tempfile::tempdir().unwrap();
            write_project_config(
                dir.path(),
                &format!("[providers.ollama]\nkeep_alive = \"{input}\"\n"),
            );
            let cfg = load_with_trust(dir.path(), true).unwrap();
            assert_eq!(cfg.ollama.keep_alive, expected);
            assert!(cfg.warnings.is_empty(), "{:?}", cfg.warnings);
        }
    }

    #[test]
    fn ollama_settings_reject_malformed_keep_alive() {
        let dir = tempfile::tempdir().unwrap();
        write_project_config(dir.path(), "[providers.ollama]\nkeep_alive = \"soon\"\n");
        let cfg = load_with_trust(dir.path(), true).unwrap();
        // Falls back to the default rather than failing config load.
        assert_eq!(cfg.ollama.keep_alive, "30m");
        assert!(
            cfg.warnings
                .iter()
                .any(|w| w.contains("providers.ollama.keep_alive")),
            "{:?}",
            cfg.warnings
        );
    }

    #[test]
    fn ollama_settings_reject_num_ctx_below_1024() {
        let dir = tempfile::tempdir().unwrap();
        write_project_config(dir.path(), "[providers.ollama]\nnum_ctx = 512\n");
        let cfg = load_with_trust(dir.path(), true).unwrap();
        assert_eq!(cfg.ollama.num_ctx, None);
        assert!(
            cfg.warnings
                .iter()
                .any(|w| w.contains("providers.ollama.num_ctx")),
            "{:?}",
            cfg.warnings
        );
    }

    #[test]
    fn anthropic_settings_default_fast_mode_off() {
        let dir = tempfile::tempdir().unwrap();
        let cfg = load_with_trust(dir.path(), true).unwrap();
        assert!(!cfg.anthropic.fast_mode);
    }

    #[test]
    fn anthropic_settings_parse_fast_mode() {
        let dir = tempfile::tempdir().unwrap();
        write_project_config(dir.path(), "[providers.anthropic]\nfast_mode = true\n");
        let cfg = load_with_trust(dir.path(), true).unwrap();
        assert!(cfg.anthropic.fast_mode);
        assert!(cfg.warnings.is_empty(), "{:?}", cfg.warnings);
    }

    #[test]
    fn anthropic_settings_unknown_key_warns_but_does_not_fail_load() {
        let dir = tempfile::tempdir().unwrap();
        write_project_config(dir.path(), "[providers.anthropic]\nturbo = true\n");
        let cfg = load_with_trust(dir.path(), true).unwrap();
        assert!(
            cfg.warnings
                .iter()
                .any(|w| w.contains("providers.anthropic.turbo")),
            "{:?}",
            cfg.warnings
        );
    }

    #[test]
    fn plugins_network_allow_persists_and_clears_in_place() {
        crate::paths::isolate_home_for_tests();
        let dir = tempfile::tempdir().unwrap();
        write_project_config(
            dir.path(),
            "deny = [\"bash\"]\n[plugins]\nnetwork_deny = [\"older-plugin\"]\n",
        );
        let grant = Some(vec!["plugin-alpha".into(), "plugin-beta".into()]);
        persist_plugins_network_allow(&project_path(dir.path()), grant).unwrap();
        let cfg = load_with_trust(dir.path(), true).unwrap();
        assert!(cfg.plugins.is_network_allowed("plugin-alpha"));
        assert!(cfg.plugins.is_network_allowed("plugin-beta"));
        assert!(!cfg.plugins.is_network_allowed("web-search-plugin"));
        assert!(cfg.plugins.network_deny.contains(&"older-plugin".into()));
        assert_eq!(cfg.deny, vec!["bash"]);

        persist_plugins_network_allow(&project_path(dir.path()), Some(vec![])).unwrap();
        let cfg = load_with_trust(dir.path(), true).unwrap();
        assert!(!cfg.plugins.is_network_allowed("plugin-alpha"));
        assert_eq!(cfg.plugins.network_deny, vec!["older-plugin"]);
        assert_eq!(cfg.deny, vec!["bash"]);
    }

    #[test]
    fn seed_plugins_network_allow_if_empty_seeds_and_is_idempotent() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join(".vak/config.toml");

        // First run: seeds [plugins] network_allow
        assert!(seed_plugins_network_allow_if_empty(&path).unwrap());
        let text = std::fs::read_to_string(&path).unwrap();
        assert!(text.contains("network_allow"));

        // Second run: no-op, returns false
        assert!(!seed_plugins_network_allow_if_empty(&path).unwrap());

        // Custom config with existing network_allow is preserved
        let dir2 = tempfile::tempdir().unwrap();
        let path2 = dir2.path().join(".vak/config.toml");
        std::fs::create_dir_all(path2.parent().unwrap()).unwrap();
        std::fs::write(&path2, "[plugins]\nnetwork_allow = [\"custom\"]\n").unwrap();
        assert!(!seed_plugins_network_allow_if_empty(&path2).unwrap());
        let text2 = std::fs::read_to_string(&path2).unwrap();
        assert_eq!(text2, "[plugins]\nnetwork_allow = [\"custom\"]\n");
    }

    fn hook(command: &str, enabled: bool) -> HookConfig {
        HookConfig {
            event: "pre_tool_use".into(),
            matcher: None,
            command: command.into(),
            timeout_ms: None,
            enabled,
            failure_mode: None,
        }
    }

    /// `PUT /config/hooks` (vak-server) always resubmits whatever `GET
    /// /config/hooks` last reported, and used to report the merged
    /// *effective* list — global layer included. Every save re-extended an
    /// already-inherited global hook into the project layer, so the next
    /// merge doubled it, then the next save tripled it. `base.hooks.extend`
    /// without a dedup check (unlike allow/ask/deny two blocks above it)
    /// let that compound with no bound. This is the one guard against it
    /// staying fixed at the merge layer even if the endpoint-level fix
    /// (`get_hooks` reading its own file instead of the merged config)
    /// ever regresses.
    #[test]
    fn merge_into_does_not_duplicate_an_already_inherited_hook() {
        let mut base = FileConfig {
            hooks: vec![hook("global.sh", true)],
            ..FileConfig::default()
        };
        let over = FileConfig {
            // Exactly what a naive resubmit of the merged list looks like:
            // the inherited hook, unchanged, plus one genuinely new to this
            // layer.
            hooks: vec![hook("global.sh", true), hook("project.sh", true)],
            ..FileConfig::default()
        };
        merge_into(&mut base, over);
        assert_eq!(
            base.hooks,
            vec![hook("global.sh", true), hook("project.sh", true)],
            "the shared hook must appear once, not twice"
        );
    }

    /// A project hook with the same identity replaces the inherited hook,
    /// including when the edit only changes `enabled`.
    #[test]
    fn merge_into_keeps_a_hook_whose_enabled_state_changed() {
        let mut base = FileConfig {
            hooks: vec![hook("audit.sh", true)],
            ..FileConfig::default()
        };
        let over = FileConfig {
            hooks: vec![hook("audit.sh", false)],
            ..FileConfig::default()
        };
        merge_into(&mut base, over);
        assert_eq!(base.hooks, vec![hook("audit.sh", false)]);
    }

    /// A `[[hooks]]` entry written before `enabled` existed has no such key
    /// in its TOML; it must still deserialize as enabled, not silently vanish.
    /// Parsed directly from the project file (not `load_with_trust`) so the
    /// assertion is hermetic and does not inherit an ambient user-global hook.
    #[test]
    fn hook_without_enabled_key_deserializes_as_enabled() {
        let dir = tempfile::tempdir().unwrap();
        write_project_config(
            dir.path(),
            "[[hooks]]\nevent = \"pre_tool_use\"\ncommand = \"echo hi\"\n",
        );
        let (fc, _warnings) = parse_file(&dir.path().join(".vak/config.toml")).unwrap();
        assert_eq!(fc.hooks.len(), 1);
        assert!(fc.hooks[0].enabled);
    }

    /// `PUT /config/mcp` and `PATCH /config` can reach the server at the
    /// same moment, and every setting has its own writer that rewrites the
    /// whole file from what it read. Run the writers against one file at
    /// once, round after round: no write may fail, every round's changes
    /// must all land, and a reader running alongside must only ever see a
    /// whole document that still carries the keys no writer owns.
    #[test]
    fn concurrent_config_writers_all_land_and_the_file_always_parses() {
        use std::collections::BTreeMap;
        use std::sync::atomic::{AtomicBool, Ordering};

        type Write = fn(&Path, u32) -> Result<(), ConfigError>;
        type Landed = fn(&toml::Table, u32) -> bool;

        fn at<'a>(table: &'a toml::Table, keys: &[&str]) -> Option<&'a toml::Value> {
            let (last, parents) = keys.split_last()?;
            let mut table = table;
            for key in parents {
                table = table.get(*key)?.as_table()?;
            }
            table.get(*last)
        }
        fn text_at(table: &toml::Table, keys: &[&str]) -> Option<String> {
            at(table, keys)
                .and_then(toml::Value::as_str)
                .map(str::to_string)
        }
        fn strings_at(table: &toml::Table, keys: &[&str]) -> Option<Vec<String>> {
            at(table, keys)?
                .as_array()?
                .iter()
                .map(|value| value.as_str().map(str::to_string))
                .collect()
        }

        struct StopOnDrop<'a>(&'a AtomicBool);
        impl Drop for StopOnDrop<'_> {
            fn drop(&mut self) {
                self.0.store(true, Ordering::Release);
            }
        }

        let writers: [(&str, Write, Landed); 14] = [
            (
                "mcp servers",
                |cwd, round| {
                    let server = McpServerConfig {
                        command: format!("mcp-{round}"),
                        args: Vec::new(),
                        env: BTreeMap::new(),
                        network: false,
                        serves: Vec::new(),
                    };
                    persist_mcp_servers(
                        &project_path(cwd),
                        &BTreeMap::from([(format!("server-{round}"), server)]),
                    )
                },
                |table, round| {
                    text_at(
                        table,
                        &["mcp", "servers", &format!("server-{round}"), "command"],
                    ) == Some(format!("mcp-{round}"))
                },
            ),
            (
                "preferences",
                |cwd, round| {
                    let model = format!("model-{round}");
                    persist_project_preferences(cwd, None, Some(&model), None, None, None, None)
                },
                |table, round| text_at(table, &["model"]) == Some(format!("model-{round}")),
            ),
            (
                "voice",
                |cwd, round| {
                    let patch = VoicePatch {
                        transcription_model: Some(Some(format!("stt-{round}"))),
                        ..VoicePatch::default()
                    };
                    persist_voice_settings_at(project_path(cwd), &patch)
                },
                |table, round| {
                    text_at(table, &["voice", "transcription_model"])
                        == Some(format!("stt-{round}"))
                },
            ),
            (
                "bus",
                |cwd, round| {
                    let url = format!("nats://bus-{round}");
                    persist_bus_settings(project_path(cwd), Some(Some(&url)), None)
                },
                |table, round| {
                    text_at(table, &["server", "bus", "nats_url"])
                        == Some(format!("nats://bus-{round}"))
                },
            ),
            (
                "evidence policy",
                |cwd, round| persist_evidence_max_age(project_path(cwd), i64::from(round)),
                |table, round| {
                    at(table, &["intent", "evidence_max_age_secs"])
                        .and_then(toml::Value::as_integer)
                        == Some(i64::from(round))
                },
            ),
            (
                "memory",
                |cwd, round| {
                    persist_project_memory_prefs(cwd, Some(round % 2 == 0), None, None, None)
                },
                |table, round| {
                    at(table, &["memory", "search_enabled"]).and_then(toml::Value::as_bool)
                        == Some(round % 2 == 0)
                },
            ),
            (
                "workers",
                |cwd, round| persist_project_workers(cwd, round % 2 == 0),
                |table, round| {
                    at(table, &["workers"]).and_then(toml::Value::as_bool) == Some(round % 2 == 0)
                },
            ),
            (
                "work policy",
                |cwd, round| {
                    persist_work_preferences(
                        project_path(cwd),
                        None,
                        None,
                        Some(round as usize + 1),
                        None,
                        None,
                        None,
                    )
                },
                |table, round| {
                    at(table, &["work", "max_items"]).and_then(toml::Value::as_integer)
                        == Some(i64::from(round) + 1)
                },
            ),
            (
                "gateway approvals",
                |cwd, round| {
                    persist_gateway_approvals(
                        project_path(cwd),
                        None,
                        None,
                        Some(u64::from(round) + 1),
                    )
                },
                |table, round| {
                    at(table, &["gateway", "approval_timeout_secs"])
                        .and_then(toml::Value::as_integer)
                        == Some(i64::from(round) + 1)
                },
            ),
            (
                "permission rules",
                |cwd, round| {
                    let deny = [format!("Bash(rm-{round} *)")];
                    persist_permission_rules(project_path(cwd), None, None, Some(&deny))
                },
                |table, round| {
                    strings_at(table, &["deny"]) == Some(vec![format!("Bash(rm-{round} *)")])
                },
            ),
            (
                "plugin network grants",
                |cwd, round| {
                    persist_plugins_network_allow(
                        &project_path(cwd),
                        Some(vec![format!("plugin-{round}")]),
                    )
                },
                |table, round| {
                    strings_at(table, &["plugins", "network_allow"])
                        == Some(vec![format!("plugin-{round}")])
                },
            ),
            (
                "finops caps",
                |cwd, round| {
                    persist_project_finops_caps(cwd, Some(Some(f64::from(round) + 0.5)), None)
                },
                |table, round| {
                    at(table, &["finops", "max_run_usd"]).and_then(toml::Value::as_float)
                        == Some(f64::from(round) + 0.5)
                },
            ),
            (
                "capability inheritance",
                |cwd, round| {
                    persist_capability_inheritance(
                        &project_path(cwd),
                        Some(round % 2 == 0),
                        None,
                        None,
                        None,
                        None,
                    )
                },
                |table, round| {
                    at(table, &["capabilities", "inherit_mcp"]).and_then(toml::Value::as_bool)
                        == Some(round % 2 == 0)
                },
            ),
            (
                "hooks",
                |cwd, round| {
                    let hook = HookConfig {
                        event: "stop".into(),
                        matcher: None,
                        command: format!("hook-{round}.sh"),
                        timeout_ms: Some(1_000),
                        enabled: true,
                        failure_mode: Some("open".into()),
                    };
                    persist_hooks(&project_path(cwd), &[hook])
                },
                |table, round| {
                    at(table, &["hooks"])
                        .and_then(toml::Value::as_array)
                        .and_then(|hooks| hooks.first())
                        .and_then(|hook| hook.get("command"))
                        .and_then(toml::Value::as_str)
                        == Some(format!("hook-{round}.sh").as_str())
                },
            ),
        ];

        let dir = tempfile::tempdir().unwrap();
        let cwd = dir.path();
        let path = project_path(cwd);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(
            &path,
            "future_key = \"kept\"\n\n[future_table]\nnested = 1\n",
        )
        .unwrap();

        const ROUNDS: u32 = 100;
        let stop = AtomicBool::new(false);
        let mut failures = Vec::new();
        let (reads, torn) = std::thread::scope(|scope| {
            let stop_reader = StopOnDrop(&stop);
            let reader = scope.spawn(|| {
                let mut reads = 0_u32;
                let mut torn = Vec::new();
                while !stop.load(Ordering::Acquire) {
                    let seen = match std::fs::read_to_string(&path) {
                        Ok(text) => match toml::from_str::<toml::Table>(&text) {
                            Ok(table) if text_at(&table, &["future_key"]).is_some() => None,
                            Ok(_) => Some(format!("a key no writer owns was dropped: {text:?}")),
                            Err(error) => Some(format!("unparseable: {error} in {text:?}")),
                        },
                        Err(error) => Some(format!("unreadable: {error}")),
                    };
                    reads += 1;
                    torn.extend(seen);
                }
                (reads, torn)
            });
            for round in 0..ROUNDS {
                let barrier = std::sync::Barrier::new(writers.len());
                let results: Vec<_> = std::thread::scope(|round_scope| {
                    let handles: Vec<_> = writers
                        .iter()
                        .map(|&(name, write, _)| {
                            let barrier = &barrier;
                            round_scope.spawn(move || {
                                barrier.wait();
                                (name, write(cwd, round))
                            })
                        })
                        .collect();
                    handles
                        .into_iter()
                        .map(|handle| handle.join().expect("writer thread"))
                        .collect()
                });
                for (name, result) in results {
                    if let Err(error) = result {
                        failures.push(format!("round {round}: {name} failed: {error}"));
                    }
                }
                let text = std::fs::read_to_string(&path).expect("config file");
                match toml::from_str::<toml::Table>(&text) {
                    Ok(table) => {
                        for (name, _, landed) in &writers {
                            if !landed(&table, round) {
                                failures.push(format!("round {round}: {name}'s change was lost"));
                            }
                        }
                        if at(&table, &["future_table", "nested"]).is_none() {
                            failures.push(format!("round {round}: [future_table] was dropped"));
                        }
                    }
                    Err(error) => failures.push(format!("round {round}: unparseable: {error}")),
                }
            }
            drop(stop_reader);
            reader.join().expect("reader thread")
        });

        assert!(reads > 0, "the reader never ran");
        assert!(
            torn.is_empty() && failures.is_empty(),
            "{} of {reads} concurrent reads saw a broken document (first: {:?}); \
             {} writes failed or were lost across {ROUNDS} rounds (first few: {:#?})",
            torn.len(),
            torn.first(),
            failures.len(),
            &failures[..failures.len().min(6)]
        );
        let left: Vec<_> = std::fs::read_dir(path.parent().unwrap())
            .unwrap()
            .map(|entry| entry.unwrap().file_name())
            .collect();
        assert_eq!(left, ["config.toml"], "no temporary file is left behind");
    }

    /// The management API owns `command`, `args`, `env` and `network`.
    /// Rewriting the server list must keep what it does not own: `serves`,
    /// a key a later version adds, and any other key in `[mcp]` (invariant
    /// 29). A single-server change must leave every other server alone.
    #[test]
    fn mcp_writers_keep_the_keys_they_do_not_own() {
        use std::collections::BTreeMap;
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.toml");
        std::fs::write(
            &path,
            "[mcp]\nfuture_mcp_key = true\n\n\
             [mcp.servers.search]\ncommand = \"old\"\nnetwork = true\n\
             serves = [\"web\"]\nfuture_server_key = 3\n\
             [mcp.servers.search.env]\nOLD = \"1\"\n\n\
             [mcp.servers.gone]\ncommand = \"gone\"\n",
        )
        .unwrap();
        let server = |command: &str| McpServerConfig {
            command: command.into(),
            args: vec!["--stdio".into()],
            env: BTreeMap::new(),
            network: false,
            serves: Vec::new(),
        };
        let read =
            || -> toml::Table { toml::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap() };

        persist_mcp_servers(
            &path,
            &BTreeMap::from([("search".to_string(), server("new"))]),
        )
        .unwrap();
        let document = read();
        let mcp = document["mcp"].as_table().unwrap();
        assert_eq!(mcp["future_mcp_key"].as_bool(), Some(true));
        let servers = mcp["servers"].as_table().unwrap();
        assert!(
            !servers.contains_key("gone"),
            "a server left out is removed"
        );
        let search = servers["search"].as_table().unwrap();
        assert_eq!(search["command"].as_str(), Some("new"));
        assert_eq!(search["args"].as_array().unwrap().len(), 1);
        assert!(!search.contains_key("env"), "an emptied env is cleared");
        assert!(!search.contains_key("network"), "network off is cleared");
        assert_eq!(search["serves"].as_array().unwrap().len(), 1);
        assert_eq!(search["future_server_key"].as_integer(), Some(3));

        persist_mcp_server(&path, "other", Some(&server("other"))).unwrap();
        persist_mcp_server(&path, "search", Some(&server("newer"))).unwrap();
        let document = read();
        let servers = document["mcp"]["servers"].as_table().unwrap();
        assert_eq!(servers["other"]["command"].as_str(), Some("other"));
        assert_eq!(servers["search"]["command"].as_str(), Some("newer"));
        assert_eq!(servers["search"]["future_server_key"].as_integer(), Some(3));

        persist_mcp_server(&path, "other", None).unwrap();
        let document = read();
        let servers = document["mcp"]["servers"].as_table().unwrap();
        assert!(!servers.contains_key("other"));
        assert!(servers.contains_key("search"));
        assert_eq!(document["mcp"]["future_mcp_key"].as_bool(), Some(true));
    }

    /// Saving a value the file already holds is not a change, so the file,
    /// comments included, is left exactly as the operator wrote it.
    #[test]
    fn saving_an_unchanged_setting_leaves_the_file_untouched() {
        let dir = tempfile::tempdir().unwrap();
        let text = "# chosen by hand\nmodel = \"kept\" # keep this\n";
        write_project_config(dir.path(), text);
        persist_project_preferences(dir.path(), None, Some("kept"), None, None, None, None)
            .unwrap();
        assert_eq!(
            std::fs::read_to_string(project_path(dir.path())).unwrap(),
            text
        );
        persist_project_preferences(dir.path(), None, Some("changed"), None, None, None, None)
            .unwrap();
        let cfg = load_with_trust(dir.path(), true).unwrap();
        assert_eq!(cfg.model, "changed");
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod channel_autonomy_tests {
    use super::*;

    fn policy(ceiling: Option<&str>) -> ChannelPolicy {
        ChannelPolicy {
            autonomy_ceiling: ceiling.map(str::to_string),
            ..ChannelPolicy::default()
        }
    }

    /// Restrictive only, like every other key on this type. A chat may say
    /// "propose only, in here"; it may never say "act freely" on a workspace
    /// whose operator did not.
    #[test]
    fn a_channel_can_only_lower_autonomy_never_raise_it() {
        let composed = ChannelPolicy::merge(&policy(Some("delegated")), &policy(Some("manual")));
        assert_eq!(composed.autonomy_ceiling.as_deref(), Some("manual"));

        // The narrower tier asking for MORE does not get it.
        let composed = ChannelPolicy::merge(&policy(Some("manual")), &policy(Some("autonomous")));
        assert_eq!(composed.autonomy_ceiling.as_deref(), Some("manual"));
    }

    #[test]
    fn silence_on_one_tier_inherits_rather_than_permitting_everything() {
        assert_eq!(
            ChannelPolicy::merge(&policy(Some("assisted")), &policy(None))
                .autonomy_ceiling
                .as_deref(),
            Some("assisted")
        );
        assert_eq!(
            ChannelPolicy::merge(&policy(None), &policy(Some("manual")))
                .autonomy_ceiling
                .as_deref(),
            Some("manual")
        );
        assert!(
            ChannelPolicy::merge(&policy(None), &policy(None))
                .autonomy_ceiling
                .is_none()
        );
    }

    /// An unparseable ceiling must not read as maximum delegation.
    #[test]
    fn an_unknown_ceiling_is_treated_as_assisted_not_autonomous() {
        let composed = ChannelPolicy::merge(&policy(Some("banana")), &policy(Some("autonomous")));
        assert_eq!(composed.autonomy_ceiling.as_deref(), Some("banana"));
        assert!(autonomy_rank("banana") < autonomy_rank("autonomous"));
    }

    #[test]
    fn plugin_resolved_allow_deny_network_matrix() {
        let mut resolved = PluginResolved::default();
        assert!(resolved.is_enabled("test-plugin"));
        assert!(!resolved.is_network_allowed("test-plugin"));

        // Enable network
        resolved.network_allow = Some(vec!["test-plugin".into()]);
        assert!(resolved.is_network_allowed("test-plugin"));

        // Global or channel deny takes priority
        resolved.network_deny.push("test-plugin".into());
        assert!(!resolved.is_network_allowed("test-plugin"));
        assert!(resolved.is_enabled("test-plugin"));

        // Disabling or denying plugin shuts it off completely
        resolved.disabled.push("test-plugin".into());
        assert!(!resolved.is_enabled("test-plugin"));
        assert!(!resolved.is_network_allowed("test-plugin"));
    }

    #[test]
    fn channel_policy_merges_plugin_network_and_allow_deny() {
        let bot = ChannelPolicy {
            plugins_allow: Some(vec!["test-plugin-a".into(), "test-plugin-b".into()]),
            plugins_deny: vec!["untrusted-plugin".into()],
            plugins_network_deny: vec!["test-plugin-b".into()],
            ..ChannelPolicy::default()
        };
        let chat = ChannelPolicy {
            plugins_allow: Some(vec!["test-plugin-a".into()]),
            plugins_deny: vec!["banned-plugin".into()],
            plugins_network_deny: vec!["test-plugin-a".into()],
            ..ChannelPolicy::default()
        };
        let merged = ChannelPolicy::merge(&bot, &chat);
        assert_eq!(merged.plugins_allow, Some(vec!["test-plugin-a".into()]));
        assert!(
            merged
                .plugins_deny
                .contains(&"untrusted-plugin".to_string())
        );
        assert!(merged.plugins_deny.contains(&"banned-plugin".to_string()));
        assert!(
            merged
                .plugins_network_deny
                .contains(&"test-plugin-b".to_string())
        );
        assert!(
            merged
                .plugins_network_deny
                .contains(&"test-plugin-a".to_string())
        );
    }

    /// The persisted TOML field was renamed from `subagents` to `workers`.
    /// An existing `vak.toml` with the old key (and no `workers` key) must
    /// still be honored rather than silently falling back to the default.
    #[test]
    fn legacy_subagents_key_is_honored_as_workers() {
        let dir = tempfile::tempdir().unwrap();
        let path = project_path(dir.path());
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(&path, "subagents = false\n").unwrap();

        let (fc, warnings) = parse_file(&path).unwrap();
        assert_eq!(fc.workers, Some(false));
        assert!(
            warnings.is_empty(),
            "legacy `subagents` key should not warn as unknown: {warnings:?}"
        );

        let cfg = load_with_trust(dir.path(), true).unwrap();
        assert!(!cfg.workers);
    }
}
