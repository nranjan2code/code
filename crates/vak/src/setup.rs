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
        awaiting_activation: probe_awaiting_activation(),
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

/// Bots configured in `bots.json` that the service manager has never been
/// told about. Configuring a bot does not activate it (doc 46 D6), so this
/// is a normal, deliberate state — and one the operator has to be able to
/// see, or a created bot looks identical to a running one.
fn probe_awaiting_activation() -> Vec<String> {
    let data_home = vak_config::paths::data_home();
    let paths = vak_ops::services::Paths::default();
    vak_ops::services::configured_bot_service_names_all(&data_home)
        .into_iter()
        .filter(|name| !vak_ops::services::unit_is_registered(name, &paths))
        .collect()
}

/// What the service manager says about the units this installation
/// expects. `None` when nothing is registered — an installation that
/// never asked for durable services is complete without them.
fn probe_services() -> Option<Vec<(String, bool)>> {
    let cfg = vak_ops::OpsConfig::detect();
    let probed: Vec<(String, bool)> = [vak_ops::Service::Gateway, vak_ops::Service::Bridges]
        .into_iter()
        .filter_map(|service| {
            let state = vak_ops::status(service, &cfg);
            (state != vak_ops::State::NotInstalled).then(|| {
                (
                    service.label().to_string(),
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

/// `vak setup` — start the local setup server and hand over its URL.
///
/// The wizard is the web admin console (doc 46 D7): it is present in every
/// install — headless box, Linux server, macOS desktop — so it is the only
/// surface that can carry one first-run experience everywhere. There is no
/// second server and no second frontend; this binds the same secured
/// router the desktop shell uses.
///
/// It is **not** a durable service (D9). Nothing is registered with
/// launchd or systemd, so `vak self install` still starts nothing and
/// nothing unattended exists until the wizard's activation step says so.
/// The process ends when the operator ends it.
pub async fn run_wizard(cwd: PathBuf, open_browser: bool, print_url_only: bool) -> i32 {
    // The server runs against the invoking directory; the wizard's
    // workspace step is what actually chooses where work happens, and
    // durable services always resolve the canonical default independently
    // (AGENTS.md invariant 18) regardless of where this was run.
    let workspace = cwd;
    if let Err(e) = std::fs::create_dir_all(&workspace) {
        eprintln!("error: cannot create {}: {e}", workspace.display());
        return 2;
    }

    let trusted = vak_core::trust::is_trusted(&workspace);
    let core = match vak_core::Core::new_with_trust(workspace.clone(), trusted) {
        Ok(c) => c,
        Err(e) => {
            eprintln!("error: {e}");
            return 2;
        }
    };

    // Loopback only. A setup server binds no external interface, so the
    // window in which an unconfigured install is reachable is this
    // machine, and only with the token below.
    let listener = match tokio::net::TcpListener::bind(("127.0.0.1", 0)).await {
        Ok(l) => l,
        Err(e) => {
            eprintln!("error: cannot bind a local port: {e}");
            return 2;
        }
    };
    let addr = match listener.local_addr() {
        Ok(a) => a,
        Err(e) => {
            eprintln!("error: {e}");
            return 2;
        }
    };
    let (app, token) = vak_server::secured_router(core);
    // The token reaches the operator on stdout and nowhere else: not a
    // file, not a log, not a service unit.
    let url = format!("http://{addr}/admin?token={token}#/setup");

    println!("vak setup — {}", workspace.display());
    println!();
    println!("  {url}");
    println!();
    if print_url_only {
        println!(
            "open that in a browser (or tunnel to it: ssh -L {0}:127.0.0.1:{0} <host>)",
            addr.port()
        );
    } else if open_browser && open_in_browser(&url) {
        println!("opened in your browser");
    } else {
        println!("open that URL to continue");
    }
    println!("press Ctrl-C when you are finished");

    if let Err(e) = vak_server::serve_router(listener, app).await {
        eprintln!("error: setup server stopped: {e}");
        return 2;
    }
    0
}

/// Best-effort browser launch. A headless box has none, which is normal —
/// the URL was already printed, so failure here costs nothing.
fn open_in_browser(url: &str) -> bool {
    #[cfg(target_os = "macos")]
    let mut command = std::process::Command::new("open");
    #[cfg(not(target_os = "macos"))]
    let mut command = std::process::Command::new("xdg-open");
    command
        .arg(url)
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .map(|s| s.success())
        .unwrap_or(false)
}
