//! `vak setup` (`docs/design/46-stabilization-install-and-onboarding.md`).
//!
//! Install places bits; **setup** chooses and activates (D6). This module
//! owns the terminal half of that contract. S1 lands `status`, the read
//! side of the shared projection, so every surface can already agree on
//! what is configured before any of them can change it. The guided flow
//! itself is S2 (web) and S3 (terminal).

use std::path::PathBuf;

use vak_core::onboarding::{self, OnboardingState, ProbedFacts, StepFailure, StepState};

/// Probe the facts the library deliberately does not gather for itself:
/// the install manifest, and the service manager.
///
/// Both live behind process boundaries a status read should touch once,
/// explicitly, rather than have a library shell out on every call.
pub fn probe(prefix: Option<PathBuf>) -> ProbedFacts {
    ProbedFacts {
        install: probe_install(prefix),
        services: probe_services(),
    }
}

fn probe_install(prefix: Option<PathBuf>) -> Option<Result<String, StepFailure>> {
    let root = crate::install::layout::InstallRoot::resolve(prefix);
    if !root.is_installed() {
        // Running from a source tree. Normal, not a defect.
        return None;
    }
    let manifest = match crate::install::manifest::Manifest::read(&root) {
        Ok(m) => m,
        Err(e) => {
            // `Manifest::read` refuses pre-baseline state with the one
            // shared message, whose repair is a purge -- NOT a reinstall.
            // Handing back a generic "run reinstall" here would give the
            // reader a command that cannot work (AGENTS.md invariant 29).
            let failure = if e.contains(vak_core::baseline::BASELINE) {
                StepFailure::new(
                    format!(
                        "This machine has an install that predates the {} baseline.",
                        vak_core::baseline::BASELINE
                    ),
                    "Your project files are untouched; only vak's own state is removed.",
                    "Run `vak self uninstall --purge`, then install and run setup.",
                )
            } else {
                StepFailure::new(
                    "The install manifest could not be read.",
                    "Your configuration, sessions, and secrets are untouched.",
                    "Run `vak self reinstall` to replace the installed files.",
                )
            };
            return Some(Err(failure.with_detail(e)));
        }
    };
    let defects = manifest.verify();
    if defects.is_empty() {
        return Some(Ok(format!(
            "{} verified at {}",
            manifest.version,
            root.prefix().display()
        )));
    }
    Some(Err(StepFailure::new(
        "Installed components do not match the manifest.",
        "Your configuration, sessions, and secrets are untouched.",
        "Run `vak self reinstall` to replace the installed files.",
    )
    .with_detail(
        defects
            .iter()
            .map(ToString::to_string)
            .collect::<Vec<_>>()
            .join("; "),
    )))
}

/// What the service manager says about the units this installation
/// expects. `None` when nothing is registered — an installation that
/// never asked for durable services is complete without them.
fn probe_services() -> Option<Vec<(String, bool)>> {
    let cfg = vak_ops::OpsConfig::detect();
    let probed: Vec<(String, bool)> = [vak_ops::Service::Gateway, vak_ops::Service::Telegram]
        .into_iter()
        .filter_map(|service| {
            let state = vak_ops::status(service, &cfg);
            (state != vak_ops::State::NotInstalled).then(|| {
                (
                    service.launchd_label().to_string(),
                    state == vak_ops::State::Running,
                )
            })
        })
        .collect();
    (!probed.is_empty()).then_some(probed)
}

pub fn run_status(cwd: PathBuf, prefix: Option<PathBuf>, json: bool) -> i32 {
    let core = match vak_core::Core::new_with_trust(cwd.clone(), vak_core::trust::is_trusted(&cwd))
    {
        Ok(c) => c,
        Err(e) => {
            eprintln!("error: {e}");
            return 2;
        }
    };
    let state = onboarding::derive(&core, &probe(prefix));

    if json {
        match serde_json::to_string_pretty(&state) {
            Ok(text) => println!("{text}"),
            Err(e) => {
                eprintln!("error: {e}");
                return 2;
            }
        }
        // JSON is for machines: the exit code carries the verdict so a
        // script does not have to parse the body to branch on it.
        return i32::from(!state.core_ready);
    }

    render(&state);
    i32::from(!state.core_ready)
}

fn render(state: &OnboardingState) {
    let width = state
        .steps()
        .iter()
        .map(|(label, _)| label.len())
        .max()
        .unwrap_or(0);

    println!("setup:");
    for (label, step) in state.steps() {
        match step {
            StepState::Satisfied { detail, provenance } => {
                let from = provenance
                    .as_deref()
                    .map(|p| format!("  ({p})"))
                    .unwrap_or_default();
                println!("  ✓ {label:<width$}  {detail}{from}");
            }
            StepState::NotApplicable { reason } => {
                println!("  · {label:<width$}  {reason}");
            }
            StepState::Incomplete(f) => {
                println!("  ✗ {label:<width$}  {}", f.what);
            }
        }
    }

    println!();
    println!(
        "core ready        {}",
        if state.core_ready { "yes" } else { "no" }
    );
    println!(
        "unattended ready  {}",
        if state.unattended_ready { "yes" } else { "no" }
    );

    // Failures repeat at the bottom in full, because the four fields are
    // the whole point: one line each is a summary, not a remedy.
    let incomplete = state.incomplete();
    if incomplete.is_empty() {
        return;
    }
    println!();
    for (label, f) in incomplete {
        println!("{label}");
        println!("  {}", f.what);
        println!("  {}", f.preserved);
        println!("  → {}", f.repair);
        if let Some(detail) = &f.detail {
            println!("  details: {detail}");
        }
        println!();
    }
}

/// Apply the Shared capability seeds (`crate::setup_seed`).
///
/// This is a **setup** action, never an install side effect (D6). Placing
/// binaries used to do it, which meant a `--prefix` install silently got
/// nothing and an update got nothing at all. Idempotent: an existing
/// skill, plugin, or hook is never overwritten, so re-running after an
/// upgrade adds what is new and leaves edited files alone.
pub fn run_seed() -> i32 {
    let root = vak_config::paths::default_workspace();
    println!("seeding Shared capabilities into {}", root.display());
    crate::setup_seed::seed_shared_capabilities();
    let skills = root.join(".vak/skills");
    let count = std::fs::read_dir(&skills)
        .map(|e| e.flatten().filter(|e| e.path().is_dir()).count())
        .unwrap_or(0);
    println!("{count} shared skills available");
    0
}
