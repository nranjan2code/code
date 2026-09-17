//! `vak doctor` (docs/design/29-personal-os.md P3): CLI parity with the
//! TUI's `/doctor`, rendered from the same `vak_core::health::collect`
//! report — checks first (failures last), facts, then the frozen route
//! ladder when one is on record.
//!
//! `--repair` (docs/design/32-release-engineering.md) acts on the small
//! subset of checks that have a mechanical fix — today, self version
//! parity via `self install --force` — then re-collects and re-prints the
//! report. Checks with no mechanical fix (provider auth, config warnings,
//! an unwritable sessions home) are left for the operator; doctor never
//! guesses at those.

use std::path::PathBuf;

use vak_core::{Core, health};

pub fn run_doctor(cwd: PathBuf, trusted: bool, repair: bool) -> i32 {
    let core = match Core::new_with_trust(cwd.clone(), trusted) {
        Ok(c) => c,
        Err(e) => {
            eprintln!("error: {e}");
            return 2;
        }
    };
    crate::print_config_warnings(&core);

    let mut report = health::collect(&core, None);

    if repair {
        let repaired = repair_known_failures(&core, &report);
        if !repaired.is_empty() {
            for line in &repaired {
                println!("  ↻ {line}");
            }
            println!();
            report = health::collect(&core, None);
        }
    }

    println!("doctor:");
    // Passing checks first so failures read as one block at the bottom.
    let mut ordered: Vec<&health::HealthCheck> = report.checks.iter().collect();
    ordered.sort_by_key(|c| c.detail.is_err());
    let width = ordered.iter().map(|c| c.label.len()).max().unwrap_or(0);
    for check in &ordered {
        match &check.detail {
            Ok(detail) => {
                println!("  ✓ {:<width$}  {}", check.label, detail, width = width)
            }
            Err(detail) => {
                println!("  ✗ {:<width$}  {}", check.label, detail, width = width)
            }
        }
    }

    for fact in &report.facts {
        println!("  · {fact}");
    }
    if let Some(ladder) = &report.ladder {
        println!(
            "  · route ladder (frozen at admission): {}",
            if ladder.rendered.is_empty() {
                "(none recorded)"
            } else {
                &ladder.rendered
            }
        );
        if !ladder.objective.is_empty() || ladder.fallback_legs > 0 {
            println!(
                "  · route objective: {} · fallback legs: {}",
                ladder.objective, ladder.fallback_legs
            );
        }
        for note in &ladder.annotations {
            println!("  · route: {note}");
        }
    }
    if report.failures == 0 {
        println!("all checks passed");
        0
    } else {
        println!("{} check(s) failed", report.failures);
        if !repair {
            println!("run `vak doctor --repair` to act on the ones with a known fix");
        }
        1
    }
}

/// Act on the checks in `report` that have a mechanical fix. Returns one
/// human-readable line per repair attempted, success or failure — never
/// silent. Checks with no known fix (provider auth, config warnings) are
/// left untouched.
pub(crate) fn repair_known_failures(core: &Core, report: &health::HealthReport) -> Vec<String> {
    let mut lines = Vec::new();
    for check in &report.checks {
        let Err(detail) = &check.detail else {
            continue;
        };
        // docs/design/34: an expired pending channel request auto-denies —
        // mechanical, no judgment call. An `allowed` entry with an
        // unreachable workspace is left alone on purpose: re-pointing it is
        // an operator decision (`PATCH .../allowlist/{key}` or the Admin
        // UI), not something doctor may guess at.
        if check.label == health::GATEWAY_CHANNELS_LABEL {
            let days = core.config().gateway.pending_expiry_days;
            let denied = health::expire_pending_entries(&core.shared_data_home(), days);
            lines.push(if denied.is_empty() {
                format!(
                    "gateway channels ({detail}): nothing mechanically repairable \
                     — re-point unreachable workspaces from the Admin UI"
                )
            } else {
                format!(
                    "gateway channels: auto-denied {} pending request(s) older than {days}d \
                     (added_by=expiry, still visible): {}",
                    denied.len(),
                    denied.join(", ")
                )
            });
            continue;
        }
        if check.label == "self version parity" {
            lines.push(format!("self version parity ({detail}): reinstalling…"));
            // Reinstalling from a unit test would rewrite the developer's
            // own installed binary, so the call itself is compiled out
            // there; the gateway-channel repair above is what the tests
            // exercise.
            #[cfg(not(test))]
            {
                let code = crate::install::run_install(None, true);
                lines.push(if code == 0 {
                    "self version parity: reinstalled".to_string()
                } else {
                    format!("self version parity: reinstall failed (exit {code})")
                });
            }
        }
        if check.label == "retired plugins" {
            lines.push(format!("retired plugins ({detail}): running cleanup…"));
            match vak_core::seed::seed_shared_capabilities() {
                Ok(()) => lines.push(
                    "retired plugins: cleanup pass complete (run `vak doctor` to verify)"
                        .to_string(),
                ),
                Err(error) => lines.push(format!("retired plugins: cleanup failed ({error})")),
            }
        }
    }
    lines
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;

    fn core_with_channels(entries: serde_json::Value) -> (tempfile::TempDir, Core) {
        let dir = tempfile::tempdir().unwrap();
        let core = Core::new(dir.path().to_path_buf()).unwrap();
        core.set_sessions_home(dir.path().join("home"));
        let path = health::allowlist_path(&core.shared_data_home());
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(
            path,
            serde_json::json!({ "schema": 1, "entries": entries }).to_string(),
        )
        .unwrap();
        (dir, core)
    }

    fn ago(days: i64) -> String {
        (chrono::Utc::now() - chrono::Duration::days(days)).to_rfc3339()
    }

    #[test]
    fn repair_acts_on_an_expired_pending_channel_and_says_what_it_did() {
        let (_dir, core) = core_with_channels(serde_json::json!([
            { "key": "telegram:9", "status": "pending", "added_at": ago(30), "added_by": "gateway" },
        ]));
        let report = health::collect(&core, None);
        let lines = repair_known_failures(&core, &report);
        let line = lines
            .iter()
            .find(|l| l.starts_with("gateway channels"))
            .expect("the expired pending entry must be reported as repaired");
        assert!(line.contains("telegram:9"), "{line}");
        assert!(line.contains("added_by=expiry"), "{line}");
        // Re-collecting proves the repair actually landed in the store.
        assert!(
            health::collect(&core, None)
                .checks
                .iter()
                .find(|c| c.label == health::GATEWAY_CHANNELS_LABEL)
                .unwrap()
                .detail
                .is_ok()
        );
    }

    #[test]
    fn repair_defers_an_unreachable_workspace_to_the_operator() {
        let (_dir, core) = core_with_channels(serde_json::json!([
            { "key": "telegram:9", "status": "allowed", "workspace": "/definitely/not/here",
              "added_at": ago(1), "added_by": "admin" },
        ]));
        let report = health::collect(&core, None);
        let lines = repair_known_failures(&core, &report);
        let line = lines
            .iter()
            .find(|l| l.starts_with("gateway channels"))
            .expect("doctor must still say something, not repair silently");
        assert!(line.contains("nothing mechanically repairable"), "{line}");
        // And the failure survives: re-pointing is a judgment call.
        assert!(
            health::collect(&core, None)
                .checks
                .iter()
                .find(|c| c.label == health::GATEWAY_CHANNELS_LABEL)
                .unwrap()
                .detail
                .is_err()
        );
    }

    #[test]
    fn healthy_channels_are_not_touched_by_repair() {
        let (_dir, core) = core_with_channels(serde_json::json!([
            { "key": "telegram:9", "status": "pending", "added_at": ago(1), "added_by": "gateway" },
        ]));
        let report = health::collect(&core, None);
        assert!(
            !repair_known_failures(&core, &report)
                .iter()
                .any(|l| l.starts_with("gateway channels")),
            "repair must act only on failing checks"
        );
    }
}
