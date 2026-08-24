//! vak-config: layered configuration. Defaults < global file < project file
//! < environment. Unknown keys are ignored with a warning, never fatal.

pub mod finops;

pub use finops::{estimate_cost_usd, resolve_usd_per_mtok, usd_per_mtok_heuristic};

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, serde::Serialize, Deserialize)]
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

#[derive(Debug, Clone, Deserialize)]
pub struct HookConfig {
    pub event: String,
    #[serde(rename = "match")]
    pub matcher: Option<String>,
    pub command: String,
    pub timeout_ms: Option<u64>,
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
    pub warnings: Vec<String>,
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
            gateway: GatewayResolved {
                enabled: false,
                approvals: "deny".into(),
                approver: None,
                approval_timeout_secs: 300,
                webhooks: std::collections::BTreeMap::new(),
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
}

pub fn global_path() -> Option<PathBuf> {
    dirs_home().map(|h| h.join(".config/vakcoder/config.toml"))
}

pub fn project_path(cwd: &Path) -> PathBuf {
    cwd.join(".vakcoder/config.toml")
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
                "project .vakcoder/config.toml is not trusted for this workspace; \
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

    if let Ok(m) = std::env::var("VAKCODER_MODEL") {
        merged.model = Some(m);
    }
    if let Ok(p) = std::env::var("VAKCODER_PROVIDER") {
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
    base.hooks.extend(over.hooks);
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
    for (name, hook) in over.gateway.outbound.webhooks {
        base.gateway.outbound.webhooks.insert(name, hook);
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

/// `$HOME/.vakcoder/.env` — the user-level secret store shared by every
/// vakcoder surface.
pub fn user_env_path() -> Option<std::path::PathBuf> {
    std::env::var_os("HOME").map(|h| std::path::PathBuf::from(h).join(".vakcoder").join(".env"))
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
