//! Doctor logic (docs/design/29-personal-os.md P3), extracted verbatim in
//! substance from the TUI's `/doctor` so CLI, desktop, and TUI report the
//! same facts from one implementation. Pure collection: no rendering.

use std::path::{Path, PathBuf};

use vak_session::SessionLog;

use crate::{APP_VERSION, Core};

/// Where `self install` records the deployed release
/// (docs/design/32-release-engineering.md).
pub fn install_manifest_path(home: &Path) -> PathBuf {
    home.join("local/release/install.json")
}

fn user_home() -> PathBuf {
    std::env::var_os("HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("/"))
}

/// Canonical-layout conformance (doc 32). The legacy dotdir must not
/// reappear as the data home: if it exists alongside the canonical home
/// something skipped migration, and any service still writing there is
/// invisible to the rest of the stack.
fn layout_check() -> HealthCheck {
    let label = "install layout".to_string();
    let overridden = std::env::var_os("VAKCODER_HOME").is_some();
    let canonical = vak_config::paths::data_home();
    let legacy = user_home().join(".vakcoder");
    if overridden {
        return HealthCheck {
            label,
            detail: Ok(format!("override active → {}", canonical.display())),
        };
    }
    // A dotdir holding only logs/store remnants after migration is fine;
    // sessions living there means the tree never migrated.
    let legacy_sessions = legacy.join("sessions");
    if legacy_sessions.is_dir() && !canonical.join("sessions").is_dir() {
        return HealthCheck {
            label,
            detail: Err(format!(
                "sessions found in legacy {} — run vakcoder once to migrate to {}",
                legacy_sessions.display(),
                canonical.join("sessions").display()
            )),
        };
    }
    HealthCheck {
        label,
        detail: Ok(canonical.display().to_string()),
    }
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
/// passes — nothing is managed yet, so nothing can drift.
pub fn version_parity_check(home: &Path) -> HealthCheck {
    let label = "self version parity".to_string();
    let manifest = install_manifest_path(home);
    let Ok(text) = std::fs::read_to_string(&manifest) else {
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

/// Collect everything `/doctor` reports. `session` optionally adds the
/// frozen-ladder section for the active session. Never panics; every
/// failure mode lands as a failed check or an empty fact.
pub fn collect(core: &Core, session: Option<&SessionLog>) -> HealthReport {
    let mut checks = Vec::new();

    let provider_detail = match core.provider() {
        Ok(p) => Ok(format!("{} ready", p.name())),
        Err(e) => Err(e.to_string()),
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
    checks.push(layout_check());
    checks.push(version_parity_check(&user_home()));
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
            "extensions: {} skills · {} hooks · {} mcp servers · subagents {}",
            core.skills().len(),
            core.config().hooks.len(),
            core.config().mcp.servers.len(),
            if core.config().subagents { "on" } else { "off" },
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
                "install layout",
                "self version parity"
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
            Some(&home.path().display().to_string())
        );
        // Default config carries no warnings and no active session ladder.
        assert_eq!(
            report.checks[2].detail.as_ref().ok(),
            Some(&"none".to_string())
        );
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
    fn parity_passes_without_manifest() {
        let home = tempfile::tempdir().unwrap();
        let check = version_parity_check(home.path());
        assert_eq!(check.label, "self version parity");
        assert!(check.detail.is_ok());
    }

    #[test]
    fn parity_matches_installed_and_flags_drift() {
        let home = tempfile::tempdir().unwrap();
        let manifest = install_manifest_path(home.path());
        std::fs::create_dir_all(manifest.parent().unwrap()).unwrap();

        std::fs::write(
            &manifest,
            format!(r#"{{"version":"{APP_VERSION}","git_sha":"deadbeef"}}"#),
        )
        .unwrap();
        let ok = version_parity_check(home.path());
        assert!(ok.detail.is_ok());

        std::fs::write(&manifest, r#"{"version":"0.0.9-legacy"}"#).unwrap();
        let drifted = version_parity_check(home.path());
        assert_eq!(
            drifted.detail.as_ref().err(),
            Some(&format!("build {APP_VERSION} != installed 0.0.9-legacy"))
        );

        std::fs::write(&manifest, "not json").unwrap();
        assert!(version_parity_check(home.path()).detail.is_err());
    }

    #[test]
    fn ladder_section_reflects_frozen_contract() {
        use vak_session::types::{FrozenContract, SessionHeader};
        let dir = tempfile::tempdir().unwrap();
        let home = tempfile::tempdir().unwrap();
        let core = Core::new(dir.path().to_path_buf()).unwrap();
        core.set_sessions_home(home.path().to_path_buf());

        let header = SessionHeader {
            session_id: "s-health".into(),
            created_at: chrono::Utc::now(),
            cwd: dir.path().to_path_buf(),
            parent_session_id: None,
            contract: FrozenContract {
                app_version: "0.0.0-test".into(),
                provider: "anthropic".into(),
                model: "claude-sonnet-4-5".into(),
                route_ladder: vec![
                    vak_llm::RouteLeg {
                        provider: "anthropic".into(),
                        model: "claude-sonnet-4-5".into(),
                    },
                    vak_llm::RouteLeg {
                        provider: "openai".into(),
                        model: "gpt-fallback".into(),
                    },
                ],
                route_objective: "balanced".into(),
                route_annotations: vec!["thin primary evidence".into()],
                system_prompt: String::new(),
                tools: Vec::new(),
                permission_mode: "workspace-write".into(),
                skills: Vec::new(),
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
}
