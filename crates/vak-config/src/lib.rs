//! vak-config: layered configuration. Defaults < global file < project file
//! < environment. Unknown keys are ignored with a warning, never fatal.

pub mod finops;
pub mod paths;

pub use finops::{estimate_cost_usd, resolve_usd_per_mtok, usd_per_mtok_heuristic};

use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock};

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
    pub profile: Option<String>,
    pub profiles: std::collections::BTreeMap<String, Profile>,
    pub anthropic_base_url: Option<String>,
    #[serde(default)]
    pub allow: Vec<String>,
    #[serde(default)]
    pub ask: Vec<String>,
    #[serde(default)]
    pub deny: Vec<String>,
    pub subagents: Option<bool>,
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
    pub route: RouteSettings,
    #[serde(default)]
    pub automation: AutomationSettings,
    #[serde(default)]
    pub update: UpdateSettings,
    #[serde(default)]
    pub tools: ToolsSettings,
    #[serde(default)]
    pub heartbeat: HeartbeatSettings,
    #[serde(default)]
    pub feeds: FeedSettings,
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

/// Spend admission (docs/design/27 Phase D). Absent prices are UNKNOWN:
/// unpriced models bypass USD math rather than guessing at zero.
#[derive(Debug, Clone, Deserialize, Default)]
#[serde(default)]
pub struct GoalSettings {
    pub handoff_reset: Option<bool>,
    pub max_audit_blocks: Option<u32>,
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

/// Frozen-ladder routing preferences (docs/design/27 Phase B + Phase R).
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
    /// Total ladder length cap INCLUDING the primary leg (default 4).
    pub max_fallbacks: Option<usize>,
    /// Caller-declared frontier-tier model-id substrings promoted under
    /// balanced/quality-critical objectives. Routing knowledge stays
    /// operator-supplied, never baked into source (invariant 9).
    pub quality_hints: Vec<String>,
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

/// Feed pipeline settings. Read-only and workspace-scoped.
#[derive(Debug, Clone, Deserialize, Default)]
#[serde(default)]
pub struct FeedSettings {
    /// Enable the feed pipeline. Default false.
    pub enabled: Option<bool>,
    /// Path to feeds.toml config file. None = auto-detect.
    pub config_path: Option<String>,
    /// Path to the DuckDB database file. None = auto-detect.
    pub db_path: Option<String>,
    /// Default check interval for sources (e.g. "30m", "1h").
    pub default_check_interval: Option<String>,
    /// Maximum items to keep per feed.
    pub max_items_per_feed: Option<u32>,
    /// Days to keep dedup hashes.
    pub dedup_window_days: Option<u32>,
}

/// Resolved feed pipeline settings.
#[derive(Debug, Clone)]
pub struct FeedResolved {
    pub enabled: bool,
    pub config_path: Option<String>,
    pub db_path: Option<String>,
    pub default_check_interval: String,
    pub max_items_per_feed: u32,
    pub dedup_window_days: u32,
}

#[derive(Debug, Clone, Deserialize, PartialEq)]
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
}

impl ChannelPolicy {
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
        }
    }
}

// Note: the `Bot` entity itself (id/surface/label/token_env/policy/
// permission_mode/route/workspace) lives in `vak-server::gateway` next to
// `AllowlistEntry` and `AllowlistRoute`, since it needs `AllowlistRoute` and
// vak-config must not depend on vak-server. It reuses `ChannelPolicy::merge`
// and `PermissionMode::capped_by` from here for its slot in the bot → chat →
// workspace resolution chain.

#[derive(Debug, Clone, PartialEq, Deserialize)]
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
    pub anthropic_base_url: Option<String>,
    pub allow: Vec<String>,
    pub ask: Vec<String>,
    pub deny: Vec<String>,
    pub subagents: bool,
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
    pub ui: UiResolved,
    pub stop_policy: StopPolicyResolved,
    pub gateway: GatewayResolved,
    pub memory: MemoryResolved,
    pub sandbox: SandboxResolved,
    pub finops: FinopsResolved,
    pub goal: GoalResolved,
    pub route: RouteResolved,
    pub automation: AutomationResolved,
    pub update: UpdateResolved,
    pub tools: ToolsResolved,
    pub heartbeat: HeartbeatResolved,
    pub feeds: FeedResolved,
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

/// Resolved goal-mode policy (docs/design/27 Phase H).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GoalResolved {
    /// Reset-with-handoff rescue on still-over contexts.
    pub handoff_reset: bool,
    /// Audit blocks per goal before degrading to Unverified.
    pub max_audit_blocks: u32,
}

/// Resolved spend-admission policy (docs/design/27 Phase D).
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
    /// Total ladder length cap including the primary leg.
    pub max_fallbacks: usize,
    /// Frontier-tier model-id substrings (lowercased for matching).
    pub quality_hints: Vec<String>,
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

impl Default for Config {
    fn default() -> Self {
        Config {
            provider: "anthropic".into(),
            model: "claude-sonnet-4-5".into(),
            max_tokens: 8192,
            max_turns: 40,
            permission_mode: PermissionMode::WorkspaceWrite,
            anthropic_base_url: None,
            allow: Vec::new(),
            ask: Vec::new(),
            deny: Vec::new(),
            subagents: true,
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
            route: RouteResolved {
                objective: "auto".into(),
                fallback_models: Vec::new(),
                max_fallbacks: 4,
                quality_hints: Vec::new(),
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
            feeds: FeedResolved {
                enabled: false,
                config_path: None,
                db_path: None,
                default_check_interval: "30m".into(),
                max_items_per_feed: 500,
                dedup_window_days: 90,
            },
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

pub fn global_path() -> Option<PathBuf> {
    dirs_home().map(|h| h.join(".config/vak/config.toml"))
}

pub fn project_path(cwd: &Path) -> PathBuf {
    cwd.join(".vak/config.toml")
}

/// Atomically replace the MCP table at one explicit configuration scope.
/// The caller selects either [`global_path`] or [`project_path`]; no values
/// are inferred from the process directory. Other TOML keys are preserved.
pub fn persist_mcp_servers(
    path: &Path,
    servers: &std::collections::BTreeMap<String, McpServerConfig>,
) -> Result<(), ConfigError> {
    static WRITE_LOCK: OnceLock<Mutex<()>> = OnceLock::new();
    let _guard = WRITE_LOCK
        .get_or_init(|| Mutex::new(()))
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let mut root = if path.is_file() {
        let text = std::fs::read_to_string(path).map_err(|source| ConfigError::Read {
            path: path.to_path_buf(),
            source,
        })?;
        toml::from_str::<toml::Value>(&text).map_err(|source| ConfigError::Parse {
            path: path.to_path_buf(),
            source,
        })?
    } else {
        toml::Value::Table(toml::map::Map::new())
    };
    let Some(table) = root.as_table_mut() else {
        return Err(ConfigError::Write {
            path: path.to_path_buf(),
            source: std::io::Error::other("top-level config must be a TOML table"),
        });
    };
    let entries = servers
        .iter()
        .map(|(name, server)| {
            let mut value = toml::map::Map::new();
            value.insert(
                "command".into(),
                toml::Value::String(server.command.clone()),
            );
            value.insert(
                "args".into(),
                toml::Value::Array(
                    server
                        .args
                        .iter()
                        .cloned()
                        .map(toml::Value::String)
                        .collect(),
                ),
            );
            if !server.env.is_empty() {
                value.insert(
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
                value.insert("network".into(), toml::Value::Boolean(true));
            }
            (name.clone(), toml::Value::Table(value))
        })
        .collect();
    table.insert(
        "mcp".into(),
        toml::Value::Table(
            std::iter::once(("servers".into(), toml::Value::Table(entries))).collect(),
        ),
    );
    let text = toml::to_string_pretty(&root).map_err(|error| ConfigError::Write {
        path: path.to_path_buf(),
        source: std::io::Error::other(error.to_string()),
    })?;
    let parent = path.parent().ok_or_else(|| ConfigError::Write {
        path: path.to_path_buf(),
        source: std::io::Error::other("config has no parent directory"),
    })?;
    std::fs::create_dir_all(parent).map_err(|source| ConfigError::Write {
        path: parent.to_path_buf(),
        source,
    })?;
    let temp = parent.join(format!(".config.toml.{}.tmp", std::process::id()));
    std::fs::write(&temp, text).map_err(|source| ConfigError::Write {
        path: temp.clone(),
        source,
    })?;
    std::fs::rename(&temp, path).map_err(|source| ConfigError::Write {
        path: path.to_path_buf(),
        source,
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
    theme: Option<&str>,
) -> Result<(), ConfigError> {
    persist_preferences_at(
        project_path(cwd),
        provider,
        model,
        max_turns,
        permission_mode,
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
    theme: Option<&str>,
) -> Result<(), ConfigError> {
    let path = global_path().ok_or_else(|| ConfigError::Write {
        path: PathBuf::from("<user-config>"),
        source: std::io::Error::other("user home is unavailable"),
    })?;
    persist_preferences_at(path, provider, model, max_turns, permission_mode, theme)
}

fn persist_preferences_at(
    path: PathBuf,
    provider: Option<&str>,
    model: Option<&str>,
    max_turns: Option<usize>,
    permission_mode: Option<PermissionMode>,
    theme: Option<&str>,
) -> Result<(), ConfigError> {
    static WRITE_LOCK: OnceLock<Mutex<()>> = OnceLock::new();
    let _guard = WRITE_LOCK
        .get_or_init(|| Mutex::new(()))
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let mut root = if path.is_file() {
        let text = std::fs::read_to_string(&path).map_err(|source| ConfigError::Read {
            path: path.clone(),
            source,
        })?;
        toml::from_str::<toml::Value>(&text).map_err(|source| ConfigError::Parse {
            path: path.clone(),
            source,
        })?
    } else {
        toml::Value::Table(toml::map::Map::new())
    };
    let Some(table) = root.as_table_mut() else {
        return Err(ConfigError::Write {
            path,
            source: std::io::Error::other("top-level config must be a TOML table"),
        });
    };
    if let Some(value) = provider {
        table.insert("provider".into(), toml::Value::String(value.into()));
    }
    if let Some(value) = model {
        table.insert("model".into(), toml::Value::String(value.into()));
    }
    if let Some(value) = max_turns {
        table.insert("max_turns".into(), toml::Value::Integer(value as i64));
    }
    if let Some(value) = permission_mode {
        table.insert(
            "permission_mode".into(),
            toml::Value::String(
                match value {
                    PermissionMode::ReadOnly => "read-only",
                    PermissionMode::WorkspaceWrite => "workspace-write",
                    PermissionMode::FullAccess => "full-access",
                }
                .into(),
            ),
        );
    }
    if let Some(value) = theme {
        let ui = table
            .entry("ui")
            .or_insert_with(|| toml::Value::Table(toml::map::Map::new()));
        let Some(ui) = ui.as_table_mut() else {
            return Err(ConfigError::Write {
                path,
                source: std::io::Error::other("ui config must be a TOML table"),
            });
        };
        ui.insert("theme".into(), toml::Value::String(value.into()));
    }
    let text = toml::to_string_pretty(&root).map_err(|error| ConfigError::Write {
        path: path.clone(),
        source: std::io::Error::other(error.to_string()),
    })?;
    let parent = path.parent().ok_or_else(|| ConfigError::Write {
        path: path.clone(),
        source: std::io::Error::other("config has no parent directory"),
    })?;
    std::fs::create_dir_all(parent).map_err(|source| ConfigError::Write {
        path: parent.to_path_buf(),
        source,
    })?;
    let temp = parent.join(format!(".config.toml.{}.tmp", std::process::id()));
    std::fs::write(&temp, text).map_err(|source| ConfigError::Write {
        path: temp.clone(),
        source,
    })?;
    std::fs::rename(&temp, &path).map_err(|source| ConfigError::Write { path, source })
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
    static WRITE_LOCK: OnceLock<Mutex<()>> = OnceLock::new();
    let _guard = WRITE_LOCK
        .get_or_init(|| Mutex::new(()))
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let mut root = if path.is_file() {
        let text = std::fs::read_to_string(&path).map_err(|source| ConfigError::Read {
            path: path.clone(),
            source,
        })?;
        toml::from_str::<toml::Value>(&text).map_err(|source| ConfigError::Parse {
            path: path.clone(),
            source,
        })?
    } else {
        toml::Value::Table(toml::map::Map::new())
    };
    let Some(table) = root.as_table_mut() else {
        return Err(ConfigError::Write {
            path,
            source: std::io::Error::other("top-level config must be a TOML table"),
        });
    };
    if search_enabled.is_some()
        || write_enabled.is_some()
        || reflection.is_some()
        || skill_proposals.is_some()
    {
        let memory = table
            .entry("memory")
            .or_insert_with(|| toml::Value::Table(toml::map::Map::new()));
        let Some(memory) = memory.as_table_mut() else {
            return Err(ConfigError::Write {
                path,
                source: std::io::Error::other("memory config must be a TOML table"),
            });
        };
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
    let text = toml::to_string_pretty(&root).map_err(|error| ConfigError::Write {
        path: path.clone(),
        source: std::io::Error::other(error.to_string()),
    })?;
    let parent = path.parent().ok_or_else(|| ConfigError::Write {
        path: path.clone(),
        source: std::io::Error::other("config has no parent directory"),
    })?;
    std::fs::create_dir_all(parent).map_err(|source| ConfigError::Write {
        path: parent.to_path_buf(),
        source,
    })?;
    let temp = parent.join(format!(".config.toml.{}.tmp", std::process::id()));
    std::fs::write(&temp, text).map_err(|source| ConfigError::Write {
        path: temp.clone(),
        source,
    })?;
    std::fs::rename(&temp, &path).map_err(|source| ConfigError::Write { path, source })
}

/// Persist the top-level `subagents` toggle for the current project.
/// Mirrors [`persist_project_preferences`]'s atomic-write shape exactly.
pub fn persist_project_subagents(cwd: &Path, enabled: bool) -> Result<(), ConfigError> {
    persist_subagents_at(project_path(cwd), enabled)
}

/// Persist the user-level `subagents` default, inherited by project
/// configs through [`load_with_trust`] until they set their own override.
pub fn persist_global_subagents(enabled: bool) -> Result<(), ConfigError> {
    let path = global_path().ok_or_else(|| ConfigError::Write {
        path: PathBuf::from("<user-config>"),
        source: std::io::Error::other("user home is unavailable"),
    })?;
    persist_subagents_at(path, enabled)
}

fn persist_subagents_at(path: PathBuf, enabled: bool) -> Result<(), ConfigError> {
    static WRITE_LOCK: OnceLock<Mutex<()>> = OnceLock::new();
    let _guard = WRITE_LOCK
        .get_or_init(|| Mutex::new(()))
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let mut root = if path.is_file() {
        let text = std::fs::read_to_string(&path).map_err(|source| ConfigError::Read {
            path: path.clone(),
            source,
        })?;
        toml::from_str::<toml::Value>(&text).map_err(|source| ConfigError::Parse {
            path: path.clone(),
            source,
        })?
    } else {
        toml::Value::Table(toml::map::Map::new())
    };
    let Some(table) = root.as_table_mut() else {
        return Err(ConfigError::Write {
            path,
            source: std::io::Error::other("top-level config must be a TOML table"),
        });
    };
    table.insert("subagents".into(), toml::Value::Boolean(enabled));
    let text = toml::to_string_pretty(&root).map_err(|error| ConfigError::Write {
        path: path.clone(),
        source: std::io::Error::other(error.to_string()),
    })?;
    let parent = path.parent().ok_or_else(|| ConfigError::Write {
        path: path.clone(),
        source: std::io::Error::other("config has no parent directory"),
    })?;
    std::fs::create_dir_all(parent).map_err(|source| ConfigError::Write {
        path: parent.to_path_buf(),
        source,
    })?;
    let temp = parent.join(format!(".config.toml.{}.tmp", std::process::id()));
    std::fs::write(&temp, text).map_err(|source| ConfigError::Write {
        path: temp.clone(),
        source,
    })?;
    std::fs::rename(&temp, &path).map_err(|source| ConfigError::Write { path, source })
}

fn dirs_home() -> Option<PathBuf> {
    std::env::var_os("HOME").map(PathBuf::from)
}

pub fn load(cwd: &Path) -> Result<Config, ConfigError> {
    load_with_trust(cwd, true)
}

/// Keys a PROJECT-level config may not set when its workspace has not been
/// marked trusted: they grant execution or redirect credentials.
const PRIVILEGED_KEYS_NOTICE: &str =
    "permission_mode, allow, hooks, anthropic_base_url, mcp.servers, gateway, sandbox";

pub fn load_with_trust(cwd: &Path, trust_project: bool) -> Result<Config, ConfigError> {
    let mut warnings = Vec::new();
    let mut layers: Vec<FileConfig> = vec![FileConfig::default()];

    if let Some(gp) = global_path().filter(|gp| gp.is_file()) {
        let (fc, w) = parse_file(&gp)?;
        warnings.extend(w);
        layers.push(fc);
    }
    let pp = project_path(cwd);
    if pp.is_file() {
        let (mut fc, w) = parse_file(&pp)?;
        warnings.extend(w);
        if !trust_project {
            // A repository must not be able to configure itself into
            // execution power on first run. Restrictive keys (deny/ask)
            // still apply.
            if fc.permission_mode.is_some() {
                fc.permission_mode = None;
            }
            if fc.anthropic_base_url.is_some() {
                fc.anthropic_base_url = None;
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
    cfg.anthropic_base_url = merged.anthropic_base_url;
    cfg.allow = merged.allow;
    cfg.ask = merged.ask;
    cfg.deny = merged.deny;
    if let Some(sa) = merged.subagents {
        cfg.subagents = sa;
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
    cfg.hooks = merged.hooks;
    for (name, srv) in merged.mcp.servers {
        cfg.mcp.servers.insert(name, srv);
    }
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
    cfg.route.max_fallbacks = merged.route.max_fallbacks.unwrap_or(4).clamp(1, 16);
    cfg.route.quality_hints = merged
        .route
        .quality_hints
        .iter()
        .map(|h| h.to_ascii_lowercase())
        .collect();

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
    let fs = &merged.feeds;
    cfg.feeds.enabled = fs.enabled.unwrap_or(false);
    cfg.feeds.config_path = fs.config_path.clone();
    cfg.feeds.db_path = fs.db_path.clone();
    cfg.feeds.default_check_interval = fs
        .default_check_interval
        .clone()
        .unwrap_or_else(|| "30m".into());
    cfg.feeds.max_items_per_feed = fs.max_items_per_feed.unwrap_or(500);
    cfg.feeds.dedup_window_days = fs.dedup_window_days.unwrap_or(90);
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
    "profile",
    "profiles",
    "anthropic_base_url",
    "allow",
    "ask",
    "deny",
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
    "ui",
    "stop_policy",
    "gateway",
    "memory",
    "sandbox",
    "finops",
    "automation",
    "update",
    "tools",
    "heartbeat",
];
const KNOWN_FINOPS_KEYS: &[&str] = &["max_run_usd", "max_day_usd", "price_overrides"];
const KNOWN_GOAL_KEYS: &[&str] = &["handoff_reset", "max_audit_blocks"];
const KNOWN_PROFILE_KEYS: &[&str] = &["model", "provider", "permission_mode", "max_turns"];
const KNOWN_HOOK_KEYS: &[&str] = &["event", "match", "command", "timeout_ms"];
const KNOWN_MCP_SERVER_KEYS: &[&str] = &["command", "args", "env", "network"];
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
    out
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
    if over.profile.is_some() {
        base.profile = over.profile;
    }
    if over.anthropic_base_url.is_some() {
        base.anthropic_base_url = over.anthropic_base_url;
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
    if over.subagents.is_some() {
        base.subagents = over.subagents;
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
        if !base.hooks.contains(&h) {
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
    if over.route.objective.is_some() {
        base.route.objective = over.route.objective;
    }
    for m in over.route.fallback_models {
        if !base.route.fallback_models.contains(&m) {
            base.route.fallback_models.push(m);
        }
    }
    if over.route.max_fallbacks.is_some() {
        base.route.max_fallbacks = over.route.max_fallbacks;
    }
    for h in over.route.quality_hints {
        if !base.route.quality_hints.contains(&h) {
            base.route.quality_hints.push(h);
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
    for (name, hook) in over.gateway.outbound.webhooks {
        base.gateway.outbound.webhooks.insert(name, hook);
    }
    if over.feeds.enabled.is_some() {
        base.feeds.enabled = over.feeds.enabled;
    }
    if over.feeds.config_path.is_some() {
        base.feeds.config_path = over.feeds.config_path;
    }
    if over.feeds.db_path.is_some() {
        base.feeds.db_path = over.feeds.db_path;
    }
    if over.feeds.default_check_interval.is_some() {
        base.feeds.default_check_interval = over.feeds.default_check_interval;
    }
    if over.feeds.max_items_per_feed.is_some() {
        base.feeds.max_items_per_feed = over.feeds.max_items_per_feed;
    }
    if over.feeds.dedup_window_days.is_some() {
        base.feeds.dedup_window_days = over.feeds.dedup_window_days;
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

/// Loads KEY=VALUE pairs from a .env file into the extra-env table.
/// Existing real environment variables are never overridden.
pub fn load_env_file(path: &std::path::Path) {
    let Ok(text) = std::fs::read_to_string(path) else {
        return;
    };
    let mut extra = dotenv_extra();
    merge_env_text(&mut extra, &text);
}

/// Replaces file-sourced environment values as one atomic scope change.
/// Runtime overrides and real environment variables remain untouched.
pub fn replace_env_files(paths: &[&std::path::Path]) {
    let mut extra = dotenv_extra();
    extra.clear();
    for path in paths {
        if let Ok(text) = std::fs::read_to_string(path) {
            merge_env_text(&mut extra, &text);
        }
    }
}

fn merge_env_text(extra: &mut ExtraMap, text: &str) {
    for line in text.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let Some((key, value)) = line.split_once('=') else {
            continue;
        };
        let key = key.trim();
        let value = value.trim().trim_matches('"').trim_matches('\'');
        if key.is_empty() {
            continue;
        }
        extra
            .entry(key.to_string())
            .or_insert_with(|| value.to_string());
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

/// `data_home()/.env` — the user-level secret store shared by every surface.
pub fn user_env_path() -> Option<std::path::PathBuf> {
    Some(crate::paths::data_home().join(".env"))
}

/// Removes every definition of `key` from `path`, preserving the rest of
/// the file. Missing file or missing key are both a no-op success.
pub fn remove_env_file_key(path: &std::path::Path, key: &str) -> std::io::Result<()> {
    let Ok(existing) = std::fs::read_to_string(path) else {
        return Ok(());
    };
    let mut lines: Vec<String> = Vec::new();
    for line in existing.lines() {
        let trimmed = line.trim_start();
        if trimmed.starts_with('#') || trimmed.is_empty() {
            lines.push(line.to_string());
            continue;
        }
        match line.split_once('=') {
            Some((k, _)) if k.trim() == key => continue,
            _ => lines.push(line.to_string()),
        }
    }
    let mut out = lines.join("\n");
    out.push('\n');
    let tmp = path.with_extension("env.tmp");
    std::fs::write(&tmp, out)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = std::fs::set_permissions(&tmp, std::fs::Permissions::from_mode(0o600));
    }
    std::fs::rename(&tmp, path)?;
    Ok(())
}

/// Upserts `KEY=VALUE` into `path` (created if missing, 0600 on unix).
/// Existing lines for KEY are replaced; everything else is preserved.
pub fn upsert_env_file(path: &std::path::Path, key: &str, value: &str) -> std::io::Result<()> {
    let existing = std::fs::read_to_string(path).unwrap_or_default();
    let mut replaced = false;
    let mut lines: Vec<String> = Vec::new();
    for line in existing.lines() {
        let trimmed = line.trim_start();
        if trimmed.starts_with('#') || trimmed.is_empty() {
            lines.push(line.to_string());
            continue;
        }
        match line.split_once('=') {
            Some((k, _)) if k.trim() == key => {
                if !replaced {
                    lines.push(format!("{key}={value}"));
                    replaced = true;
                }
                // drop duplicate definitions of the same key
            }
            _ => lines.push(line.to_string()),
        }
    }
    if !replaced {
        lines.push(format!("{key}={value}"));
    }
    let mut out = lines.join("\n");
    out.push('\n');

    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let tmp = path.with_extension("env.tmp");
    std::fs::write(&tmp, out)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = std::fs::set_permissions(&tmp, std::fs::Permissions::from_mode(0o600));
    }
    std::fs::rename(&tmp, path)?;
    Ok(())
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;

    /// `capped_by` is the single arithmetic the gateway's per-channel
    /// permission override rests on: it must be a true `min` over the
    /// permissiveness ranking, in both argument orders, for every pair.
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

    fn hook(command: &str, enabled: bool) -> HookConfig {
        HookConfig {
            event: "pre_tool_use".into(),
            matcher: None,
            command: command.into(),
            timeout_ms: None,
            enabled,
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

    /// A hook that differs only in `enabled` is a real edit, not a
    /// duplicate — the dedup must key on the whole value, not just command.
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
        assert_eq!(
            base.hooks,
            vec![hook("audit.sh", true), hook("audit.sh", false)]
        );
    }

    /// A `[[hooks]]` entry written before `enabled` existed has no such key
    /// in its TOML; it must still load as enabled, not silently vanish.
    #[test]
    fn hook_without_enabled_key_deserializes_as_enabled() {
        let dir = tempfile::tempdir().unwrap();
        write_project_config(
            dir.path(),
            "[[hooks]]\nevent = \"pre_tool_use\"\ncommand = \"echo hi\"\n",
        );
        let cfg = load_with_trust(dir.path(), true).unwrap();
        assert_eq!(cfg.hooks.len(), 1);
        assert!(cfg.hooks[0].enabled);
    }
}
