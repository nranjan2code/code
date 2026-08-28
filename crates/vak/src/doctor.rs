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
        let repaired = repair_known_failures(&report);
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
fn repair_known_failures(report: &health::HealthReport) -> Vec<String> {
    let mut lines = Vec::new();
    for check in &report.checks {
        let Err(detail) = &check.detail else {
            continue;
        };
        if check.label == "self version parity" {
            lines.push(format!("self version parity ({detail}): reinstalling…"));
            let code = crate::install::run_install(None, true);
            lines.push(if code == 0 {
                "self version parity: reinstalled".to_string()
            } else {
                format!("self version parity: reinstall failed (exit {code})")
            });
        }
    }
    lines
}
