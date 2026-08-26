//! Config-related commands: settings/features dashboards, provider/model/key
//! pickers, theme choices, sandbox and mode surfaces.

use std::collections::BTreeMap;

use crate::data::ClientData;
use crate::editor::{ComposerMode, Editor, VimState};
use crate::palette::ChoiceItem;
use crate::state::UiState;
use crate::theme::ThemeColors;

pub fn mode_label(mode: &str) -> String {
    match mode {
        "ReadOnly" | "read-only" | "read_only" => "read-only".to_string(),
        "WorkspaceWrite" | "workspace-write" | "workspace_write" => "workspace-write".to_string(),
        "FullAccess" | "full-access" | "full_access" => "full-access".to_string(),
        other => other.to_ascii_lowercase(),
    }
}

pub fn mode_modal_rows(mode: &str) -> Vec<String> {
    vec![
        format!("current      {}", mode_label(mode)),
        String::new(),
        "read-only         every write, edit, and bash exec asks".to_string(),
        "workspace-write   workspace edits allowed; outside writes ask".to_string(),
        "full-access       unsandboxed · explicit human trust decision".to_string(),
        String::new(),
        "/mode <mode> switches immediately when idle".to_string(),
        "switching mid-run cancels the run and denies pending approvals first".to_string(),
    ]
}

/// Renders the `/sandbox` status panel from the server's `sandbox_info`
/// payload (`{backend, name, image}`); missing fields fall back to defaults.
pub fn sandbox_rows(info: &serde_json::Value) -> Vec<String> {
    let backend = info["backend"].as_str().unwrap_or("default");
    let name = info["name"].as_str().unwrap_or("restricted");
    let image = info["image"].as_str().unwrap_or("default");
    vec![
        format!("effective     {name}"),
        format!("backend       {backend}"),
        format!("docker        image {image}"),
        String::new(),
        "os            platform-native (Seatbelt on macOS, Landlock on Linux)".to_string(),
        "docker        commands run inside the configured image".to_string(),
        "default       follow config.toml [sandbox]".to_string(),
        String::new(),
        "full-access runs without any sandbox by design".to_string(),
    ]
}

pub fn composer_label(ui: &UiState, inbox_unread: usize) -> String {
    let base = match &ui.attached_label {
        Some(label) => {
            let provider_model = format!("{}/{}", ui.provider, ui.model);
            format!("subagent · {} · {provider_model}", trunc_cells(label, 24))
        }
        None => format!("task · {}/{}", ui.provider, ui.model),
    };
    format!("{base}{}", inbox_badge(inbox_unread))
}

pub fn composer_footer(editor: &Editor) -> String {
    let (line, column) = editor.line_col();
    let mode_tag = match (editor.mode(), editor.vim_state()) {
        (ComposerMode::Vim, VimState::Insert) => "[-- INSERT --] ",
        (ComposerMode::Vim, VimState::Normal) => "[NORMAL] ",
        (ComposerMode::Emacs, _) => "",
    };
    let mut footer = format!(
        "{mode_tag}Ln {}, Col {} · Enter send · Alt-Enter newline · Ctrl-P commands",
        line + 1,
        column + 1
    );
    if editor.mode() == ComposerMode::Vim {
        footer.push_str(" · /composer emacs exits modal");
    }
    if editor.has_stashes() {
        footer.push_str(&format!(
            " · {} stashed paste(s), exact on submit",
            editor.stash_count()
        ));
    }
    footer
}

/// Theme is a local presentation preference: built-in packs plus
/// `[ui.themes.<name>]` customs overlaying them. Pure — persistence lives in
/// prefs.rs, not here.
pub fn theme_choices(current: &str, custom: &BTreeMap<String, ThemeColors>) -> Vec<ChoiceItem> {
    let mut items: Vec<ChoiceItem> = [
        ("dark", "balanced charcoal · calm cyan signals"),
        ("light", "paper-bright · crisp blue contrast"),
        ("neo", "electric cyan and magenta · high energy"),
        ("rich", "deep jewel tones · amber and violet"),
        (
            "teenage",
            "Teenage Engineering-inspired · cream, ink, and orange",
        ),
        ("plain", "terminal defaults · no imposed color palette"),
        ("midnight", "truecolor deep blue · soft moonlight accents"),
        ("synthwave", "truecolor neon pink/cyan over violet dusk"),
        ("forest", "truecolor moss greens on pine ground"),
    ]
    .into_iter()
    .map(|(value, description)| ChoiceItem {
        value: value.to_string(),
        description: description.to_string(),
        active: value == current,
    })
    .collect();
    for name in custom.keys() {
        if items.iter().any(|i| &i.value == name) {
            continue;
        }
        items.push(ChoiceItem {
            value: name.clone(),
            description: format!(
                "custom [ui.themes]{}",
                if *name == current { " · current" } else { "" }
            ),
            active: *name == current,
        });
    }
    items
}

pub async fn provider_choices(data: &ClientData, current: &str) -> Vec<ChoiceItem> {
    match data.providers().await {
        Ok(providers) => providers
            .into_iter()
            .map(|p| ChoiceItem {
                description: provider_status(&p.name, p.configured),
                active: p.name == current,
                value: p.name,
            })
            .collect(),
        Err(_) => Vec::new(),
    }
}

fn provider_env_var(provider: &str) -> Option<&'static str> {
    match provider {
        "anthropic" => Some("ANTHROPIC_API_KEY"),
        "google" => Some("GEMINI_API_KEY"),
        "openai" | "openai-responses" => Some("OPENAI_API_KEY"),
        "openrouter" => Some("OPENROUTER_API_KEY"),
        "opencode-zen" => Some("OPENCODE_API_KEY"),
        _ => None,
    }
}

fn provider_known(provider: &str) -> bool {
    matches!(
        provider,
        "anthropic"
            | "google"
            | "openai"
            | "openai-responses"
            | "openrouter"
            | "opencode-zen"
            | "ollama"
    )
}

/// One source of truth for provider→env-var wiring; readiness comes from the
/// server's configured flag, which resolves real env, runtime overrides, then
/// loaded `.env` files — a key stored from any surface reads ready everywhere.
pub fn provider_status(provider: &str, configured: bool) -> String {
    match provider_env_var(provider) {
        None if !provider_known(provider) => "custom provider".to_string(),
        None => "local · no API key".to_string(),
        Some(env) => {
            if configured {
                format!("ready · {env}")
            } else {
                format!("needs {env} · /key {provider} SECRET")
            }
        }
    }
}

fn model_hint(value: &str, discovered_name: Option<&str>) -> String {
    let hint = match value {
        "claude-sonnet-4-5" => "balanced coding",
        "claude-haiku-4-5" => "fast coding",
        "claude-opus-4-1" | "gemini-2.5-pro" => "deep reasoning",
        "gpt-5-codex" => "frontier coding",
        "gpt-5" => "frontier general",
        "o3" | "gpt-4.1" => "strong general",
        "gpt-4o" => "fast general model",
        "x-preview-f-free" => "free preview",
        _ if value.contains('/') => "routed",
        _ => "",
    };
    if !hint.is_empty() {
        return hint.to_string();
    }
    match discovered_name.map(str::trim).filter(|n| !n.is_empty()) {
        Some(name) => name.to_string(),
        None => "suggested by core".to_string(),
    }
}

/// Asks the provider what this key reaches. On discovery failure the picker
/// keeps offering the current model plus a notice row carrying the same id —
/// selecting it re-commits the current model rather than a fabricated one.
pub async fn model_choices(data: &ClientData, provider: &str, current: &str) -> Vec<ChoiceItem> {
    let mut choices = vec![ChoiceItem {
        value: current.to_string(),
        description: "current model".to_string(),
        active: true,
    }];
    match data.discover_models(provider).await {
        Ok(models) => {
            for m in models {
                if m.id != current {
                    choices.push(ChoiceItem {
                        description: model_hint(&m.id, m.name.as_deref()),
                        value: m.id,
                        active: false,
                    });
                }
            }
        }
        Err(e) => choices.push(ChoiceItem {
            value: current.to_string(),
            description: format!("model discovery failed · {e}"),
            active: false,
        }),
    }
    choices
}

pub async fn default_model(data: &ClientData, provider: &str) -> Option<String> {
    data.discover_models(provider)
        .await
        .ok()?
        .first()
        .map(|m| m.id.clone())
}

pub async fn set_provider_key(
    data: &ClientData,
    provider: &str,
    key: &str,
) -> Result<String, String> {
    let response = data
        .client()
        .set_provider_key(provider, key)
        .await
        .map_err(|e| e.to_string())?;
    Ok(format!(
        "{provider} key stored as {} (user .env, owner-only) · effective immediately · /provider {provider} to switch",
        response.env_var
    ))
}

pub async fn remove_provider_key_cmd(data: &ClientData, provider: &str) -> Result<String, String> {
    let removed = data
        .client()
        .remove_provider_key(provider)
        .await
        .map_err(|e| e.to_string())?;
    if removed.shadowed_by_env {
        return Ok(format!(
            "stored key removed, but {} is still set in your environment — {provider} stays authenticated",
            removed.env_var
        ));
    }
    Ok(format!(
        "{provider} key removed ({} cleared from the user .env)",
        removed.env_var
    ))
}

pub async fn settings_rows(data: &ClientData, ui: &UiState) -> Vec<String> {
    let cfg = data.config().await;
    let configured = data.provider_configured(&ui.provider).await;
    let breaker = data.breaker().await;
    let context_window = match cfg.context_window {
        Some(window) => format!("{window} tokens"),
        None => "provider default".to_string(),
    };
    let mut rows = vec![
        "AGENT".to_string(),
        format!(
            "Provider              {} · {}",
            ui.provider,
            provider_status(&ui.provider, configured)
        ),
        format!("Model                 {}", ui.model),
        format!("Maximum turns         {}", cfg.max_turns),
        format!("Maximum output        {} tokens", cfg.max_tokens),
        format!("Context window        {context_window}"),
        format!(
            "Subagents             {}",
            if cfg.subagents.as_bool().unwrap_or(false) {
                "enabled"
            } else {
                "disabled"
            }
        ),
        String::new(),
        "SAFETY".to_string(),
        format!("Permission mode       {}", mode_label(&cfg.permission_mode)),
        String::new(),
        "RELIABILITY".to_string(),
        format!(
            "Request retries       {} · base {}ms",
            cfg.max_retries, cfg.retry_base_backoff_ms
        ),
        format!("Request watchdog      {}s", cfg.request_timeout_secs),
        format!(
            "Run endurance         {} · base {}ms",
            cfg.run_retry_attempts, cfg.run_retry_base_backoff_ms
        ),
        format!(
            "Circuit breaker       {} failures · {}s cooldown",
            cfg.circuit_breaker_threshold, cfg.circuit_breaker_cooldown_secs
        ),
        format!(
            "Breaker state         {}",
            match &breaker {
                Ok(state) => breaker_state(state),
                Err(_) => "unavailable".to_string(),
            }
        ),
        format!(
            "Completion guard      {} · max {} continuations",
            if cfg.stop_policy.enabled {
                "enabled"
            } else {
                "disabled"
            },
            value_display(&cfg.stop_policy.max_blocks)
        ),
        String::new(),
        "INTERFACE".to_string(),
        format!(
            "Theme / bell          {} / {}",
            cfg.theme,
            if cfg.bell { "on" } else { "off" }
        ),
        String::new(),
        "EXTENSIONS".to_string(),
        format!(
            "Skills                {} discovered",
            cfg.integrations.skills.len()
        ),
        format!(
            "Hooks                 {} configured",
            cfg.integrations.hooks
        ),
        format!(
            "MCP servers           {} configured",
            cfg.integrations.mcp_servers.len()
        ),
        String::new(),
        "PATHS".to_string(),
        format!("Project config        {}", cfg.paths.project_config),
        format!("Session store         {}", cfg.paths.sessions_home),
    ];
    if !cfg.paths.global_config.is_empty() {
        rows.push(format!("Global config         {}", cfg.paths.global_config));
    }
    if !cfg.warnings.is_empty() {
        rows.push(String::new());
        rows.push("WARNINGS".to_string());
        rows.extend(cfg.warnings.iter().map(|warning| format!("! {warning}")));
    }
    rows.extend([
        String::new(),
        "CHANGE SETTINGS".to_string(),
        "P provider · M model · T theme · F feature explorer".to_string(),
        "/provider, /model, and /theme open searchable pickers".to_string(),
        "Enter applies a picker choice to this session".to_string(),
        "Ctrl-S saves the chosen value to the project config".to_string(),
        "/doctor diagnoses setup".to_string(),
    ]);
    rows
}

fn breaker_state(state: &serde_json::Value) -> String {
    if state["open"].as_bool().unwrap_or(false) {
        format!(
            "open · {}s remaining",
            state["remaining_secs"].as_u64().unwrap_or(0)
        )
    } else {
        "closed".to_string()
    }
}

fn value_display(value: &serde_json::Value) -> String {
    match value {
        serde_json::Value::String(s) => s.clone(),
        other => other.to_string(),
    }
}

pub async fn feature_rows(data: &ClientData) -> Vec<String> {
    let cfg = data.config().await;
    let tools = data.tools().await.unwrap_or_default().join(", ");
    let skills = data.skills().await.unwrap_or_default();
    let skill_names = skills
        .iter()
        .map(|skill| skill.name.as_str())
        .take(8)
        .collect::<Vec<_>>()
        .join(", ");
    let mcp = cfg.integrations.mcp_servers.join(", ");
    let sandbox_name = data
        .sandbox_info()
        .await
        .ok()
        .and_then(|info| info["name"].as_str().map(str::to_string))
        .unwrap_or_else(|| "restricted".to_string());
    vec![
        "AGENT RUNTIME".to_string(),
        "✓ streamed text and thinking with delta + snapshot events".to_string(),
        "✓ mid-run steering and queued follow-up turns".to_string(),
        "✓ parallel tool waves with resource-conflict scheduling".to_string(),
        format!(
            "{} bounded subagents with child-session lineage",
            if cfg.subagents.as_bool().unwrap_or(false) {
                "✓"
            } else {
                "○"
            }
        ),
        "✓ context compaction and token-budget enforcement".to_string(),
        String::new(),
        "SAFETY AND STATE".to_string(),
        "✓ read-only, workspace-write, and full-access permission modes".to_string(),
        "✓ informed approvals with once/session/persistent scopes".to_string(),
        format!("✓ {sandbox_name} OS sandbox"),
        "✓ append-only JSONL session trees and reconstructable model input".to_string(),
        "✓ workspace checkpoints, rewind, branching, and resume".to_string(),
        "✓ cancellation preserves partial model and tool output".to_string(),
        String::new(),
        "RELIABILITY".to_string(),
        "✓ exponential retry, Retry-After, watchdog deadlines".to_string(),
        "✓ run-level endurance and shared circuit breaker".to_string(),
        "✓ premature-completion stop gate and doom-loop protection".to_string(),
        "✓ typed retry, compaction, stale-stream, and failure states".to_string(),
        String::new(),
        "TOOLS AND EXTENSIONS".to_string(),
        format!("Built-ins              {tools}"),
        format!(
            "Skills                 {}{}",
            skills.len(),
            if skill_names.is_empty() {
                String::new()
            } else {
                format!(" · {skill_names}")
            }
        ),
        format!(
            "Hooks                  {} lifecycle handlers",
            cfg.integrations.hooks
        ),
        format!(
            "MCP                    {}{}",
            cfg.integrations.mcp_servers.len(),
            if mcp.is_empty() {
                String::new()
            } else {
                format!(" · {mcp}")
            }
        ),
        String::new(),
        "WORKFLOWS AND CLIENTS".to_string(),
        "✓ interactive TUI and headless exec".to_string(),
        "✓ dynamic planner and validated static flow DAGs".to_string(),
        "✓ deterministic and live evaluation suites".to_string(),
        "✓ HTTP + SSE server with approvals, steering, transcripts, and diffs".to_string(),
        "✓ Tauri desktop workspace, editor, terminal, side chats, and tasks".to_string(),
        String::new(),
        "TUI ENTRY POINTS".to_string(),
        "/sessions · /resume · /rewind · /transcript".to_string(),
        "/provider · /model · /settings · /doctor".to_string(),
        "/cost · /context · /details · /theme · /keys".to_string(),
        "/subagents attach-steer · /composer vim · /a11y · /copy OSC52".to_string(),
        "@file attachments · !shell · Ctrl-P palette · Ctrl-R history".to_string(),
    ]
}

fn trunc_cells(s: &str, max: usize) -> String {
    let mut width = 0usize;
    for (index, c) in s.char_indices() {
        width += crate::width::char_width(c);
        if width > max {
            return format!("{}…", &s[..index]);
        }
    }
    s.to_string()
}

fn inbox_badge(unread: usize) -> String {
    if unread == 0 {
        String::new()
    } else {
        format!(" · ✉ {unread}")
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;

    fn custom_themes(names: &[&str]) -> BTreeMap<String, ThemeColors> {
        names
            .iter()
            .map(|n| ((*n).to_string(), BTreeMap::new()))
            .collect()
    }

    #[test]
    fn theme_choices_lists_builtins_marks_current_and_appends_customs() {
        let themes = custom_themes(&["sunset"]);
        let choices = theme_choices("neo", &themes);
        assert_eq!(choices.first().map(|c| c.value.as_str()), Some("dark"));
        assert_eq!(choices.len(), 10);
        assert!(
            choices
                .iter()
                .find(|c| c.value == "neo")
                .is_some_and(|c| c.active)
        );
        assert!(!choices.iter().any(|c| c.value == "dark" && c.active));
        let sunset = choices.last().unwrap();
        assert_eq!(sunset.value, "sunset");
        assert!(sunset.description.contains("custom"));
        assert!(!sunset.active);

        let marked = theme_choices("sunset", &themes);
        let sunset = marked.last().unwrap();
        assert!(sunset.active && sunset.description.contains("current"));
    }

    #[test]
    fn theme_choices_dedupes_custom_names_against_builtins() {
        let themes = custom_themes(&["dark"]);
        let choices = theme_choices("light", &themes);
        assert_eq!(choices.len(), 9);
        assert!(!choices.iter().any(|c| c.description.contains("custom")));
    }

    #[test]
    fn provider_status_matches_env_wiring_and_local_providers() {
        assert_eq!(
            provider_status("anthropic", true),
            "ready · ANTHROPIC_API_KEY"
        );
        assert_eq!(
            provider_status("anthropic", false),
            "needs ANTHROPIC_API_KEY · /key anthropic SECRET"
        );
        assert_eq!(provider_status("ollama", false), "local · no API key");
        assert_eq!(provider_status("acme-cloud", true), "custom provider");
    }

    #[test]
    fn mode_label_normalizes_server_spellings() {
        assert_eq!(mode_label("ReadOnly"), "read-only");
        assert_eq!(mode_label("WorkspaceWrite"), "workspace-write");
        assert_eq!(mode_label("FullAccess"), "full-access");
        assert_eq!(mode_label("read-only"), "read-only");
    }

    #[test]
    fn trunc_cells_cuts_at_cell_boundary_with_ellipsis() {
        assert_eq!(trunc_cells("short", 10), "short");
        assert_eq!(trunc_cells("abcdefghij", 4), "abcd…");
        assert_eq!(trunc_cells("日本語テスト", 5), "日本…");
    }

    #[test]
    fn sandbox_rows_read_the_info_payload() {
        let info =
            serde_json::json!({ "backend": "docker", "name": "docker", "image": "vak:slim" });
        let rows = sandbox_rows(&info);
        assert!(rows.iter().any(|r| r.contains("effective     docker")));
        assert!(rows.iter().any(|r| r.contains("vak:slim")));

        let fallback = sandbox_rows(&serde_json::json!({}));
        assert!(fallback.iter().any(|r| r.contains("default")));
    }

    #[test]
    fn breaker_state_reports_open_with_remaining_secs() {
        assert_eq!(
            breaker_state(&serde_json::json!({ "open": false })),
            "closed"
        );
        let open = breaker_state(&serde_json::json!({ "open": true, "remaining_secs": 42 }));
        assert_eq!(open, "open · 42s remaining");
    }

    #[test]
    fn model_hint_prefers_known_labels_then_discovered_names() {
        assert_eq!(model_hint("claude-haiku-4-5", None), "fast coding");
        assert_eq!(model_hint("team/model-x", None), "routed");
        assert_eq!(
            model_hint("brand-new-model", Some("Brand New")),
            "Brand New"
        );
        assert_eq!(
            model_hint("brand-new-model", Some("  ")),
            "suggested by core"
        );
    }
}
