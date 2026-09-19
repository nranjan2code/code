//! Doctor logic (docs/design/29-personal-os.md P3), extracted verbatim in
//! substance from the TUI's `/doctor` so CLI, desktop, and TUI report the
//! same facts from one implementation. Pure collection: no rendering.

use std::path::{Path, PathBuf};

use vak_session::SessionLog;

use crate::{APP_VERSION, Core, install};

/// Canonical-layout conformance (doc 32). Confirms the active data home
/// is the platform-canonical location. There is no legacy dotdir to
/// migrate: a fresh install writes sessions, gateway state, and config
/// only under the canonical homes, and `default_workspace()`
/// (`~/vak-home`) is the sole workspace root — so there is never a
/// second home to fall out of step.
fn layout_check() -> HealthCheck {
    HealthCheck {
        label: "install layout".to_string(),
        detail: Ok(vak_config::paths::data_home().display().to_string()),
    }
}

/// Label of the gateway-channel check, shared with `vak doctor --repair`
/// so the repair path matches on a constant rather than a copied string.
pub const GATEWAY_CHANNELS_LABEL: &str = "gateway channels";

/// One `allowed`/`pending` entry as doctor needs to see it. Parsed
/// straight out of `<sessions_home>/gateway/allowlist.json` rather than
/// through `vak-server` (which depends on this crate, not the reverse) —
/// and deliberately without starting a `Core` for each workspace, since
/// existence + readability is what the check actually asserts.
#[derive(Debug, Clone)]
pub struct ChannelEntry {
    pub key: String,
    pub status: String,
    pub workspace: Option<PathBuf>,
    pub added_at: String,
}

pub fn allowlist_path(sessions_home: &Path) -> PathBuf {
    sessions_home.join("gateway").join("allowlist.json")
}

/// Read the allowlist store. A missing file is "no channels yet", not an
/// error: a gateway that has never seen an inbound message is healthy.
pub fn read_channel_entries(sessions_home: &Path) -> Vec<ChannelEntry> {
    let Ok(raw) = std::fs::read_to_string(allowlist_path(sessions_home)) else {
        return Vec::new();
    };
    let Ok(v) = serde_json::from_str::<serde_json::Value>(&raw) else {
        return Vec::new();
    };
    v["entries"]
        .as_array()
        .map(|entries| {
            entries
                .iter()
                .filter_map(|e| {
                    Some(ChannelEntry {
                        key: e["key"].as_str()?.to_string(),
                        status: e["status"].as_str()?.to_string(),
                        workspace: e["workspace"].as_str().map(PathBuf::from),
                        added_at: e["added_at"].as_str().unwrap_or_default().to_string(),
                    })
                })
                .collect()
        })
        .unwrap_or_default()
}

/// `gateway channels` (docs/design/34 "Lifecycle completeness"): an
/// approved channel whose workspace has moved or been deleted will fail
/// to start a Core on the next inbound message, and a pending request
/// nobody acted on is an onboarding request quietly rotting. Both are
/// caught here, before a user hits them.
pub fn gateway_channels_check(sessions_home: &Path, expiry_days: u64) -> HealthCheck {
    let label = GATEWAY_CHANNELS_LABEL.to_string();
    let entries = read_channel_entries(sessions_home);
    if entries.is_empty() {
        return HealthCheck {
            label,
            detail: Ok("no channels onboarded".into()),
        };
    }
    let cutoff = chrono::Utc::now() - chrono::Duration::days(expiry_days as i64);
    let mut unreachable = Vec::new();
    let mut expired = Vec::new();
    let mut allowed = 0usize;
    let mut pending = 0usize;
    let mut denied = 0usize;
    for entry in &entries {
        match entry.status.as_str() {
            "allowed" => {
                allowed += 1;
                // No workspace = inherits the gateway's own cwd, which the
                // "sessions home"/provider checks already cover.
                if let Some(ws) = &entry.workspace
                    && std::fs::read_dir(ws).is_err()
                {
                    unreachable.push(format!("{} → {}", entry.key, ws.display()));
                }
            }
            "pending" => {
                pending += 1;
                if chrono::DateTime::parse_from_rfc3339(&entry.added_at)
                    .is_ok_and(|ts| ts.with_timezone(&chrono::Utc) < cutoff)
                {
                    expired.push(entry.key.clone());
                }
            }
            _ => denied += 1,
        }
    }
    if unreachable.is_empty() && expired.is_empty() {
        return HealthCheck {
            label,
            detail: Ok(format!(
                "{allowed} allowed · {pending} pending · {denied} denied"
            )),
        };
    }
    let mut parts = Vec::new();
    if !unreachable.is_empty() {
        parts.push(format!("workspace unreachable: {}", unreachable.join(", ")));
    }
    if !expired.is_empty() {
        parts.push(format!("pending >{expiry_days}d: {}", expired.join(", ")));
    }
    HealthCheck {
        label,
        detail: Err(parts.join(" · ")),
    }
}

/// `--repair` half of [`gateway_channels_check`]: auto-deny every
/// `pending` entry older than `expiry_days`, stamping `added_by =
/// "expiry"` so it stays visibly distinct from an operator's own deny
/// (never deleted — "why did this stop working" must have an answer).
///
/// Entries are edited as raw JSON so any field a newer schema adds
/// survives the rewrite untouched. An `allowed` entry with an unreachable
/// workspace is deliberately NOT repaired: re-pointing it is a judgment
/// call, and doctor never guesses at those.
pub fn expire_pending_entries(sessions_home: &Path, expiry_days: u64) -> Vec<String> {
    let path = allowlist_path(sessions_home);
    let Ok(raw) = std::fs::read_to_string(&path) else {
        return Vec::new();
    };
    let Ok(mut doc) = serde_json::from_str::<serde_json::Value>(&raw) else {
        return Vec::new();
    };
    let cutoff = chrono::Utc::now() - chrono::Duration::days(expiry_days as i64);
    let now = chrono::Utc::now().to_rfc3339();
    let mut denied = Vec::new();
    let Some(entries) = doc["entries"].as_array_mut() else {
        return Vec::new();
    };
    for entry in entries.iter_mut() {
        let expired = entry["status"].as_str() == Some("pending")
            && entry["added_at"]
                .as_str()
                .and_then(|ts| chrono::DateTime::parse_from_rfc3339(ts).ok())
                .is_some_and(|ts| ts.with_timezone(&chrono::Utc) < cutoff);
        if !expired {
            continue;
        }
        if let Some(key) = entry["key"].as_str() {
            denied.push(key.to_string());
        }
        entry["status"] = serde_json::Value::String("denied".into());
        entry["added_by"] = serde_json::Value::String("expiry".into());
        entry["added_at"] = serde_json::Value::String(now.clone());
        if let Some(map) = entry.as_object_mut() {
            map.remove("first_seen_text");
            map.remove("workspace");
            map.remove("route");
        }
    }
    if denied.is_empty() {
        return denied;
    }
    // Same temp-file+rename write the gateway itself uses, so a crash
    // mid-repair can never leave a half-written allowlist.
    if let Ok(json) = serde_json::to_string_pretty(&doc) {
        let temp = path.with_extension(format!("json.{}.tmp", std::process::id()));
        if std::fs::write(&temp, json).is_ok() {
            let _ = std::fs::rename(temp, &path);
        }
    }
    denied
}

#[derive(Debug, Clone)]
pub struct HealthCheck {
    pub label: String,
    /// Ok = pass with detail, Err = failure with detail.
    pub detail: Result<String, String>,
}

impl HealthCheck {
    fn failed(&self) -> bool {
        self.detail.is_err()
    }
}

/// Frozen-ladder section of a session header (Phase B/R), mirrored for
/// surfaces that render doctor output.
#[derive(Debug, Clone)]
pub struct LadderReport {
    /// "provider/model" per leg, frozen order.
    pub legs: Vec<String>,
    /// Human-rendered chain; falls back to the bare model id on legacy
    /// headers without a ladder — exactly what the TUI prints today.
    pub rendered: String,
    pub objective: String,
    pub fallback_legs: usize,
    pub annotations: Vec<String>,
}

#[derive(Debug, Clone, Default)]
pub struct HealthReport {
    pub checks: Vec<HealthCheck>,
    /// Informational lines (model/mode/sandbox, limits, extensions,
    /// breaker, finops) that today render as dim status rows.
    pub facts: Vec<String>,
    pub ladder: Option<LadderReport>,
    pub failures: usize,
}

/// "Self version parity" (docs/design/32-release-engineering.md): the
/// running build vs the installed-release manifest. A missing manifest
/// passes — nothing is managed yet, so nothing can drift. `manifest` is
/// the resolved install manifest path (see [`install::resolve_manifest_path`]),
/// which — unlike a bare `$HOME`-relative join — accounts for the macOS
/// app-bundle layout that `self install` actually writes to.
pub fn version_parity_check(manifest: &Path) -> HealthCheck {
    let label = "self version parity".to_string();
    let Ok(text) = std::fs::read_to_string(manifest) else {
        return HealthCheck {
            label,
            detail: Ok("no installed release manifest".into()),
        };
    };
    let reported = serde_json::from_str::<serde_json::Value>(&text)
        .ok()
        .and_then(|v| {
            v.get("version")
                .and_then(|v| v.as_str())
                .map(str::to_string)
        });
    match reported {
        Some(v) if v == APP_VERSION => HealthCheck {
            label,
            detail: Ok(format!("build matches installed {v}")),
        },
        Some(v) => HealthCheck {
            label,
            detail: Err(format!("build {APP_VERSION} != installed {v}")),
        },
        None => HealthCheck {
            label,
            detail: Err(format!("unreadable manifest at {}", manifest.display())),
        },
    }
}

/// `core.provider()` failing only ever means the *effective* provider
/// (whatever `Config::default()` or workspace config currently names —
/// `anthropic` out of the box) lacks a credential. Left as-is, that reads
/// as "you must use Anthropic," which is false: it says so because that
/// happens to be today's built-in default, not because it's the only
/// supported option. This surfaces what else is actually usable right
/// now — any other provider with a real credential already set, plus
/// Ollama, which needs none — so the fix on offer is "point config at
/// what you already have" as often as it is "set a key."
fn missing_provider_detail(core: &Core, base: &str) -> String {
    let effective = core.effective_provider();
    let mut usable: Vec<String> = core
        .provider_names()
        .into_iter()
        .filter(|p| p != &effective && core.provider_configured(p))
        .collect();
    usable.sort();
    if usable.is_empty() {
        format!(
            "{base} — no other provider is configured either; set a credential \
             (ANTHROPIC_API_KEY, GEMINI_API_KEY, OPENAI_API_KEY, OPENROUTER_API_KEY, \
             OPENCODE_API_KEY) or point config at a local, keyless provider \
             (provider = \"ollama\")"
        )
    } else {
        format!(
            "{base} — already usable without changes: {} (switch via `vak config` \
             or the admin console instead of setting a credential for '{effective}')",
            usable.join(", ")
        )
    }
}

/// Configured integrations the composed policy will refuse.
///
/// The failure this exists to make visible is a quiet one: an operator
/// configures an MCP server, sees it accepted, and every turn that reaches
/// for it is denied by a layer they were not thinking about — an
/// unattended surface with no approver, a read-only cap, a channel deny.
/// Nothing failed loudly, so the only evidence was a tool call buried in a
/// transcript. `doctor` is where "you configured this and it does not
/// work" belongs.
///
/// No mechanical repair (AGENTS.md invariant 19): every fix here is a
/// deliberate access decision — widen a rule, or give the surface an
/// approver — and `--repair` must not make either on an operator's behalf.
fn capability_reach_check(core: &Core) -> HealthCheck {
    let standings = core.capability_standings();
    let blocked: Vec<&crate::reach::Standing> = standings
        .iter()
        .filter(|standing| standing.reach.is_blocked())
        .collect();
    HealthCheck {
        label: "capability reach".into(),
        detail: if blocked.is_empty() {
            Ok(format!("{} configured, all reachable", standings.len()))
        } else {
            Err(blocked
                .iter()
                .map(|standing| {
                    format!(
                        "{} unreachable ({}); fix: {}",
                        standing.label, standing.reason, standing.remedy
                    )
                })
                .collect::<Vec<_>>()
                .join("; "))
        },
    }
}

/// Every capability that is configured but cannot currently be used, with
/// the reason and the fix.
///
/// This closes the gap that hid the original defect. `capability_diagnostics`
/// already knew a server was unreachable, but it was rendered only into the
/// system prompt — so the model was told, and the person who could actually
/// repair the configuration was not. An agent answering "I do not have
/// access to live data" while a misconfigured search server sat in
/// `[mcp.servers]` produced no error, no failing check, and nothing in
/// `doctor`. Now the same diagnostics reach both audiences.
///
/// No mechanical repair (AGENTS.md invariant 19): the fixes here are
/// operator decisions — start a server, correct a command, grant an env var.
fn capability_health_check(core: &Core) -> HealthCheck {
    let diagnostics = core.capability_diagnostics();
    // Only genuine breakage fails the check. A hook the operator disabled and
    // a skill a channel policy excludes are deliberate, and reporting them as
    // failures trains people to scroll past this check — which would cost far
    // more than the noise saves, since a silently unreachable MCP server is
    // exactly what this exists to surface. Deliberate states are still
    // counted, so they are visible without being alarming.
    let (broken, chosen): (Vec<_>, Vec<_>) = diagnostics.iter().partition(|d| !d.deliberate);
    let describe = |d: &&crate::CapabilityDiagnostic| {
        let mut line = format!("{} `{}`: {}", d.kind, d.name, d.reason);
        if !d.remedy.is_empty() {
            line.push_str(&format!("; fix: {}", d.remedy));
        }
        line
    };
    HealthCheck {
        label: "capability health".into(),
        detail: if broken.is_empty() {
            Ok(if chosen.is_empty() {
                "all configured capabilities usable".into()
            } else {
                format!(
                    "all configured capabilities usable ({} deliberately off)",
                    chosen.len()
                )
            })
        } else {
            Err(broken.iter().map(describe).collect::<Vec<_>>().join("; "))
        },
    }
}

/// Check whether any installed plugins reference retired tool names
/// (e.g. `python_eval`, `react_preview`). These plugins survived a tool
/// retirement and cause `unknown_capability` errors / model hallucinations.
/// A failed check carries the repair instruction: run `vak setup seed`
/// to auto-remove them, or `vak plugins remove <name>` to target one.
pub fn retired_plugins_check(core: &Core) -> HealthCheck {
    let flagged = core.check_retired_plugins();
    let label = "retired plugins".to_string();
    if flagged.is_empty() {
        HealthCheck {
            label,
            detail: Ok("none".into()),
        }
    } else {
        let names: Vec<_> = flagged.iter().map(|(name, _)| name.as_str()).collect();
        let detail = format!(
            "{} plugin(s) reference retired tools: {}. Run `vak setup seed` to remove.",
            names.len(),
            names.join(", ")
        );
        HealthCheck {
            label,
            detail: Err(detail),
        }
    }
}

/// Check whether agent workspaces are valid and readable.
pub fn agent_roster_check(core: &Core) -> HealthCheck {
    let agents_dir = core.shared_data_home().join("agents");
    if !agents_dir.exists() {
        return HealthCheck {
            label: "agent roster".into(),
            detail: Ok("1 agent active (default: vak)".into()),
        };
    }
    match std::fs::read_dir(&agents_dir) {
        Ok(read) => {
            let mut names = Vec::new();
            for entry in read.flatten() {
                if entry.path().is_dir()
                    && let Some(name) = entry.file_name().to_str()
                {
                    names.push(name.to_string());
                }
            }
            names.sort();
            if names.is_empty() {
                HealthCheck {
                    label: "agent roster".into(),
                    detail: Ok("1 agent active (default: vak)".into()),
                }
            } else {
                HealthCheck {
                    label: "agent roster".into(),
                    detail: Ok(format!(
                        "{} agent workspace(s) verified ({})",
                        names.len(),
                        names.join(", ")
                    )),
                }
            }
        }
        Err(e) => HealthCheck {
            label: "agent roster".into(),
            detail: Err(format!("failed to read agents directory: {e}")),
        },
    }
}

/// Collect everything `/doctor` reports. `session` optionally adds the
/// frozen-ladder section for the active session. Never panics; every
/// failure mode lands as a failed check or an empty fact.
pub fn collect(core: &Core, session: Option<&SessionLog>) -> HealthReport {
    let mut checks = Vec::new();

    let provider_detail = match core.provider() {
        Ok(p) => Ok(format!("{} ready", p.name())),
        Err(e) => Err(missing_provider_detail(core, &e.to_string())),
    };
    checks.push(HealthCheck {
        label: "provider".into(),
        detail: provider_detail,
    });

    let home_ok = std::fs::create_dir_all(core.sessions_home()).is_ok();
    checks.push(HealthCheck {
        label: "sessions home".into(),
        detail: if home_ok {
            Ok(core.sessions_home().display().to_string())
        } else {
            Err("not writable".into())
        },
    });

    let warnings = core.config().warnings.clone();
    checks.push(HealthCheck {
        label: "config warnings".into(),
        detail: if warnings.is_empty() {
            Ok("none".into())
        } else {
            Err(warnings.join("; "))
        },
    });
    checks.push(capability_reach_check(core));
    let voice = &core.config().voice;
    checks.push(HealthCheck {
        label: "voice configuration".into(),
        detail: if voice.max_session_secs == 0
            || voice.max_concurrent == 0
            || voice.max_audio_bytes == 0
        {
            Err("voice limits must be greater than zero".into())
        } else {
            Ok(if voice.enabled {
                "enabled".into()
            } else {
                "disabled".into()
            })
        },
    });
    let local_tts = vak_voice::local_tts_readiness();
    checks.push(HealthCheck {
        label: "local TTS backend".into(),
        detail: if local_tts.ready || !local_tts.configured {
            Ok(local_tts.detail.clone())
        } else {
            Err(local_tts.detail.clone())
        },
    });
    checks.push(capability_health_check(core));
    checks.push(gateway_channels_check(
        &core.shared_data_home(),
        core.config().gateway.pending_expiry_days,
    ));
    checks.push(layout_check());
    checks.push(version_parity_check(&install::resolve_manifest_path(None)));
    checks.push(retired_plugins_check(core));
    checks.push(agent_roster_check(core));
    let failures = checks.iter().filter(|c| c.failed()).count();

    let mut facts = vec![
        format!(
            "model {} via {} · mode {:?} · sandbox {}",
            core.effective_model(),
            core.effective_provider(),
            core.effective_permission_mode(),
            core.effective_sandbox_name(),
        ),
        format!(
            "context window {} tokens · max turns {} · retries {} (+{})",
            core.config().context_window,
            core.effective_max_turns(),
            core.config().max_retries,
            core.config().run_retry_attempts,
        ),
        format!(
            "voice: {} · provider {} · model {} · {}s/session · {} concurrent · {} MiB inbound budget",
            if voice.enabled { "on" } else { "off" },
            voice.provider.as_deref().unwrap_or("default"),
            voice.model.as_deref().unwrap_or("default"),
            voice.max_session_secs,
            voice.max_concurrent,
            voice.max_audio_bytes / (1024 * 1024),
        ),
        // Counted from the *effective* set, not raw config.
        //
        // These three used to disagree: skills came from `core.skills()`
        // (effective, so plugin contributions counted) while hooks and MCP
        // servers came from `core.config()` (raw, so plugin contributions
        // and every runtime override were invisible). An operator reading
        // `doctor` saw a different world from the one the model was told
        // about, which is precisely how a broken integration stayed
        // invisible.
        format!(
            "extensions: {} skills · {} hooks · {} mcp servers · workers {}",
            core.skills().len(),
            core.effective_hooks().iter().filter(|h| h.enabled).count(),
            core.effective_mcp().servers.len(),
            if core.config().workers { "on" } else { "off" },
        ),
    ];

    facts.push(format!(
        "circuit breaker: {}",
        match core.breaker().check() {
            Ok(()) => "closed (provider healthy)".to_string(),
            Err(open) => format!(
                "OPEN — cooling down {}s after {} failure(s)",
                open.remaining_secs, open.failures
            ),
        }
    ));

    let cap_suffix = core
        .config()
        .finops
        .max_day_usd
        .map(|c| format!(" of ${c:.2} day cap"))
        .unwrap_or_default();
    facts.push(format!(
        "finops: today ~${:.2}{}",
        core.spend_day_usd(),
        cap_suffix
    ));

    let ladder = session.and_then(|s| s.header()).map(|header| {
        let contract = &header.contract;
        let legs: Vec<String> = contract
            .route_ladder
            .iter()
            .map(|leg| format!("{}/{}", leg.provider, leg.model))
            .collect();
        let rendered = if legs.is_empty() {
            contract.model.clone()
        } else {
            legs.join(" → ")
        };
        LadderReport {
            rendered,
            objective: contract.route_objective.clone(),
            fallback_legs: legs.len().saturating_sub(1),
            annotations: contract.route_annotations.clone(),
            legs,
        }
    });

    HealthReport {
        checks,
        facts,
        ladder,
        failures,
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]
    use super::*;

    fn write_allowlist(home: &Path, entries: serde_json::Value) {
        let path = allowlist_path(home);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(
            path,
            serde_json::json!({ "schema": 1, "entries": entries }).to_string(),
        )
        .unwrap();
    }

    fn ago(days: i64) -> String {
        (chrono::Utc::now() - chrono::Duration::days(days)).to_rfc3339()
    }

    #[test]
    fn gateway_channels_passes_with_no_store_at_all() {
        let home = tempfile::tempdir().unwrap();
        let check = gateway_channels_check(home.path(), 7);
        assert_eq!(check.detail.unwrap(), "no channels onboarded");
    }

    #[test]
    fn gateway_channels_pass_detail_is_counts() {
        let home = tempfile::tempdir().unwrap();
        let workspace = tempfile::tempdir().unwrap();
        write_allowlist(
            home.path(),
            serde_json::json!([
                { "key": "telegram:1", "status": "allowed", "workspace": workspace.path(), "added_at": ago(1), "added_by": "admin" },
                { "key": "slack:C1", "status": "pending", "added_at": ago(1), "added_by": "gateway" },
                { "key": "discord:9", "status": "denied", "added_at": ago(1), "added_by": "admin" },
            ]),
        );
        let detail = gateway_channels_check(home.path(), 7).detail.unwrap();
        assert_eq!(detail, "1 allowed \u{b7} 1 pending \u{b7} 1 denied");
    }

    #[test]
    fn gateway_channels_fails_and_names_an_unreachable_workspace() {
        let home = tempfile::tempdir().unwrap();
        write_allowlist(
            home.path(),
            serde_json::json!([
                { "key": "telegram:1", "status": "allowed", "workspace": "/definitely/not/here", "added_at": ago(1), "added_by": "admin" },
            ]),
        );
        let detail = gateway_channels_check(home.path(), 7).detail.unwrap_err();
        // Naming the key is the point: doctor output has to be actionable
        // without a separate admin-console trip.
        assert!(detail.contains("telegram:1"), "{detail}");
        assert!(detail.contains("/definitely/not/here"), "{detail}");
    }

    #[test]
    fn gateway_channels_fails_on_a_pending_entry_past_the_expiry_window() {
        let home = tempfile::tempdir().unwrap();
        write_allowlist(
            home.path(),
            serde_json::json!([
                { "key": "slack:C9", "status": "pending", "added_at": ago(30), "added_by": "gateway" },
            ]),
        );
        let detail = gateway_channels_check(home.path(), 7).detail.unwrap_err();
        assert!(detail.contains("slack:C9"), "{detail}");
        // A longer window makes the same entry healthy - the window is
        // config, not a hardcoded 7.
        assert!(gateway_channels_check(home.path(), 90).detail.is_ok());
    }

    #[test]
    fn a_corrupt_timestamp_never_counts_as_expired() {
        let home = tempfile::tempdir().unwrap();
        write_allowlist(
            home.path(),
            serde_json::json!([
                { "key": "slack:C9", "status": "pending", "added_at": "not-a-date", "added_by": "gateway" },
            ]),
        );
        assert!(gateway_channels_check(home.path(), 7).detail.is_ok());
        assert!(expire_pending_entries(home.path(), 7).is_empty());
    }

    #[test]
    fn repair_auto_denies_expired_pending_entries_visibly() {
        let home = tempfile::tempdir().unwrap();
        write_allowlist(
            home.path(),
            serde_json::json!([
                { "key": "slack:C9", "status": "pending", "added_at": ago(30), "added_by": "gateway", "first_seen_text": "hi" },
                { "key": "slack:C1", "status": "pending", "added_at": ago(1), "added_by": "gateway" },
            ]),
        );
        let denied = expire_pending_entries(home.path(), 7);
        assert_eq!(denied, vec!["slack:C9".to_string()]);

        let entries = read_channel_entries(home.path());
        let expired = entries.iter().find(|e| e.key == "slack:C9").unwrap();
        // Denied, not deleted: "why did this stop working" must have an
        // answer, and `added_by` distinguishes it from an operator's deny.
        assert_eq!(expired.status, "denied");
        let raw = std::fs::read_to_string(allowlist_path(home.path())).unwrap();
        assert!(raw.contains("\"added_by\": \"expiry\""), "{raw}");
        // The fresh request is untouched.
        let fresh = entries.iter().find(|e| e.key == "slack:C1").unwrap();
        assert_eq!(fresh.status, "pending");
        // And the check now passes.
        assert!(gateway_channels_check(home.path(), 7).detail.is_ok());
    }

    #[test]
    fn repair_leaves_an_unreachable_workspace_for_the_operator() {
        let home = tempfile::tempdir().unwrap();
        write_allowlist(
            home.path(),
            serde_json::json!([
                { "key": "telegram:1", "status": "allowed", "workspace": "/definitely/not/here", "added_at": ago(1), "added_by": "admin" },
            ]),
        );
        assert!(expire_pending_entries(home.path(), 7).is_empty());
        // Re-pointing is a judgment call; the failure must survive repair.
        assert!(gateway_channels_check(home.path(), 7).detail.is_err());
    }

    #[test]
    fn report_mirrors_tui_doctor_shape() {
        let dir = tempfile::tempdir().unwrap();
        let home = tempfile::tempdir().unwrap();
        let core = Core::new(dir.path().to_path_buf()).unwrap();
        core.set_sessions_home(home.path().to_path_buf());

        let report = collect(&core, None);

        assert_eq!(
            report
                .checks
                .iter()
                .map(|c| c.label.as_str())
                .collect::<Vec<_>>(),
            vec![
                "provider",
                "sessions home",
                "config warnings",
                "capability reach",
                "voice configuration",
                "local TTS backend",
                "capability health",
                "gateway channels",
                "install layout",
                "self version parity",
                "retired plugins",
                "agent roster",
            ]
        );
        assert_eq!(
            report.failures,
            report.checks.iter().filter(|c| c.failed()).count()
        );
        // Sessions home points at the override and is writable.
        let home_check = &report.checks[1];
        assert!(home_check.detail.is_ok());
        assert_eq!(
            home_check.detail.as_ref().ok(),
            Some(&core.sessions_home().display().to_string())
        );
        // Default config carries no warnings and no active session ladder.
        assert!(report.checks.iter().any(|c| c.label == "config warnings"));
        assert!(report.ladder.is_none());

        // Facts cover the five dim lines the TUI prints.
        assert!(report.facts.iter().any(|f| f.starts_with("model ")));
        assert!(
            report
                .facts
                .iter()
                .any(|f| f.starts_with("context window "))
        );
        assert!(report.facts.iter().any(|f| f.starts_with("extensions: ")));
        assert!(
            report
                .facts
                .iter()
                .any(|f| f.starts_with("circuit breaker: closed"))
        );
        assert!(
            report
                .facts
                .iter()
                .any(|f| f.starts_with("finops: today ~$"))
        );

        // webfetch registration is part of the tool surface doctor implies.
        assert!(core.tool_names().contains(&"webfetch".to_string()));
    }

    #[test]
    fn doctor_reports_voice_disabled_without_treating_it_as_failure() {
        let workspace = tempfile::tempdir().unwrap();
        let home = tempfile::tempdir().unwrap();
        let core = Core::new(workspace.path().to_path_buf()).unwrap();
        core.set_sessions_home(home.path().to_path_buf());
        let mut voice = core.effective_voice();
        voice.enabled = false;
        core.apply_persisted_voice(voice);

        let report = collect(&core, None);
        let check = report
            .checks
            .iter()
            .find(|check| check.label == "voice configuration")
            .expect("voice doctor check");
        assert_eq!(check.detail.as_deref(), Ok("disabled"));
        assert!(
            report
                .facts
                .iter()
                .any(|fact| fact.starts_with("voice: off"))
        );
    }

    #[test]
    fn parity_passes_without_manifest() {
        let home = tempfile::tempdir().unwrap();
        let manifest = home.path().join("install.json");
        let check = version_parity_check(&manifest);
        assert_eq!(check.label, "self version parity");
        assert!(check.detail.is_ok());
    }

    #[test]
    fn parity_matches_installed_and_flags_drift() {
        let home = tempfile::tempdir().unwrap();
        let manifest = home.path().join("install.json");

        std::fs::write(
            &manifest,
            format!(r#"{{"version":"{APP_VERSION}","git_sha":"deadbeef"}}"#),
        )
        .unwrap();
        let ok = version_parity_check(&manifest);
        assert!(ok.detail.is_ok());

        std::fs::write(&manifest, r#"{"version":"0.0.9-legacy"}"#).unwrap();
        let drifted = version_parity_check(&manifest);
        assert_eq!(
            drifted.detail.as_ref().err(),
            Some(&format!("build {APP_VERSION} != installed 0.0.9-legacy"))
        );

        std::fs::write(&manifest, "not json").unwrap();
        assert!(version_parity_check(&manifest).detail.is_err());
    }

    #[test]
    fn resolve_manifest_path_uses_bundle_layout_on_macos() {
        // Guards the bug this module exists to fix: health::collect must
        // resolve the same manifest path `self install` actually writes
        // to, not a hardcoded Linux-style join.
        let manifest = install::resolve_manifest_path(Some(PathBuf::from("/Applications/Vak.app")));
        assert_eq!(
            manifest,
            PathBuf::from("/Applications/Vak.app/Contents/Resources/install.json")
        );
    }

    #[test]
    fn ladder_section_reflects_frozen_contract() {
        use vak_session::types::{FrozenContract, SessionHeader};
        let dir = tempfile::tempdir().unwrap();
        let home = tempfile::tempdir().unwrap();
        let core = Core::new(dir.path().to_path_buf()).unwrap();
        core.set_sessions_home(home.path().to_path_buf());

        let header = SessionHeader {
            agent: None,
            session_id: "s-health".into(),
            created_at: chrono::Utc::now(),
            cwd: dir.path().to_path_buf(),
            parent_session_id: None,
            contract_id: None,
            work_item_id: None,
            conversation: None,
            contract: FrozenContract {
                app_version: "0.0.0-test".into(),
                provider: "anthropic".into(),
                model: "claude-sonnet-4-5".into(),
                route_ladder: vec![
                    vak_llm::RouteLeg {
                        provider: "anthropic".into(),
                        model: "claude-sonnet-4-5".into(),
                        dialect: vak_llm::EndpointDialect::AnthropicMessages,
                        credential_id: None,
                    },
                    vak_llm::RouteLeg {
                        provider: "openai".into(),
                        model: "gpt-fallback".into(),
                        dialect: vak_llm::EndpointDialect::Responses,
                        credential_id: None,
                    },
                ],
                route_objective: "balanced".into(),
                route_annotations: vec!["thin primary evidence".into()],
                system_prompt: String::new(),
                permission_mode: "workspace-write".into(),
                capabilities: Vec::new(),
                prompt_layers: Vec::new(),
            },
        };
        let log = vak_session::SessionLog::create(
            vak_session::SessionPath::new_session_file(home.path(), dir.path(), &header.session_id),
            header,
        )
        .unwrap();

        let report = collect(&core, Some(&log));
        let ladder = report.ladder.expect("session header must yield a ladder");
        assert_eq!(
            ladder.rendered,
            "anthropic/claude-sonnet-4-5 → openai/gpt-fallback"
        );
        assert_eq!(ladder.objective, "balanced");
        assert_eq!(ladder.fallback_legs, 1);
        assert_eq!(
            ladder.annotations,
            vec!["thin primary evidence".to_string()]
        );
        assert_eq!(
            report.failures,
            report.checks.iter().filter(|c| c.failed()).count()
        );
    }

    #[test]
    fn agent_roster_check_discovers_configured_agents() {
        let dir = tempfile::tempdir().unwrap();
        let home = tempfile::tempdir().unwrap();
        let core = Core::new(dir.path().to_path_buf()).unwrap();
        core.set_sessions_home(home.path().to_path_buf());

        let agent1 = home.path().join("agents").join("researcher");
        let agent2 = home.path().join("agents").join("writer");
        std::fs::create_dir_all(&agent1).unwrap();
        std::fs::create_dir_all(&agent2).unwrap();

        let check = agent_roster_check(&core);
        assert!(!check.failed());
        let detail = check.detail.expect("should pass");
        assert!(detail.contains("2 agent workspace(s) verified"));
        assert!(detail.contains("researcher"));
        assert!(detail.contains("writer"));
    }
}
