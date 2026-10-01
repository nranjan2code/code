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
    if let Err(error) = vak_core::seed::seed_shared_capabilities() {
        eprintln!("error: {error}");
        return 1;
    }
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

// ---- Terminal flow (docs/design/46 S3) -------------------------------------

/// One choice the operator has to make, and where it came from.
///
/// `--non-interactive` exists so a scripted install can run setup without a
/// person, and it **fails on any missing choice** rather than picking one
/// (doc 46 S3). Inferring a default for an unanswered question is how a
/// machine ends up configured in a way nobody chose.
struct Answers {
    provider: Option<String>,
    model: Option<String>,
    posture: Option<String>,
    seed: bool,
    activate: bool,
    first_task: bool,
}

/// True when a person is on the other end.
///
/// Menus are only printed when they can be answered: offering a numbered
/// list to a pipe and then refusing buries the actual error in noise.
fn interactive() -> bool {
    use std::io::IsTerminal as _;
    std::io::stdin().is_terminal()
}

/// Read a line from a real terminal, or refuse.
///
/// Prompts are TTY-only. A piped stdin is not a person, so reading from it
/// would turn "no answer" into whatever bytes happened to be there.
fn ask(prompt: &str) -> Option<String> {
    use std::io::{IsTerminal as _, Write as _};
    if !std::io::stdin().is_terminal() {
        return None;
    }
    print!("{prompt}");
    let _ = std::io::stdout().flush();
    let mut line = String::new();
    std::io::stdin().read_line(&mut line).ok()?;
    Some(line.trim().to_string())
}

/// Read a secret without echoing it, from a terminal or from stdin.
///
/// Never from argv: a key in a command line is in the shell history, in
/// `ps`, and in any process listing on the machine (doc 46 S3).
fn ask_secret(prompt: &str) -> Option<String> {
    use std::io::{BufRead as _, IsTerminal as _, Write as _};
    if !std::io::stdin().is_terminal() {
        // Piped: the credential is the piped content, which is how a
        // scripted install supplies one.
        let mut line = String::new();
        return std::io::stdin()
            .lock()
            .read_line(&mut line)
            .ok()
            .filter(|read| *read > 0)
            .map(|_| line.trim().to_string());
    }
    print!("{prompt}");
    let _ = std::io::stdout().flush();
    // No portable no-echo without another dependency; say so rather than
    // let someone believe the key was hidden when it was not.
    println!();
    println!("  (the key will be visible as you type)");
    let mut line = String::new();
    std::io::stdin().read_line(&mut line).ok()?;
    Some(line.trim().to_string())
}

pub async fn run_terminal(cwd: PathBuf, non_interactive: bool) -> i32 {
    let answers = Answers {
        provider: std::env::var("VAK_SETUP_PROVIDER").ok(),
        model: std::env::var("VAK_SETUP_MODEL").ok(),
        posture: std::env::var("VAK_SETUP_POSTURE").ok(),
        seed: std::env::var("VAK_SETUP_SEED").is_ok(),
        activate: std::env::var("VAK_SETUP_ACTIVATE").is_ok(),
        first_task: std::env::var("VAK_SETUP_FIRST_TASK").is_ok(),
    };

    let trusted = vak_core::trust::is_trusted(&cwd);
    let core = match vak_core::Core::new_with_trust(cwd.clone(), trusted) {
        Ok(c) => c,
        Err(e) => {
            eprintln!("error: {e}");
            return 2;
        }
    };

    println!("vak setup — {}", cwd.display());
    println!();

    // --- workspace and trust -------------------------------------------
    let asks = vak_core::trust::requested_privileges(&cwd);
    if !asks.is_empty() && !trusted {
        println!("This folder's own settings ask for:");
        for item in &asks {
            println!("  · {item}");
        }
        println!("Opening safely ignores them. Trusting lets them take effect.");
        match ask("Trust this folder? [y/N] ").as_deref() {
            Some(a) if a.eq_ignore_ascii_case("y") => {
                if let Err(e) = vak_core::trust::record(&cwd) {
                    eprintln!("warning: could not record the decision: {e}");
                } else {
                    println!("  trusted");
                }
            }
            Some(_) => println!("  opened safely — those settings stay ignored"),
            None if non_interactive => println!("  opened safely (non-interactive)"),
            None => {
                eprintln!("error: this folder needs a trust decision and stdin is not a terminal");
                eprintln!("       re-run with --non-interactive to open it safely");
                return 2;
            }
        }
        println!();
    }

    // --- provider and route --------------------------------------------
    if core.provider().is_err() || core.effective_model().trim().is_empty() {
        let provider = match answers.provider.clone().or_else(|| {
            if !interactive() {
                return None;
            }
            println!("Available services: {}", core.provider_names().join(", "));
            println!("For hosted OpenAI GPT models with tools or reasoning, choose openai-responses; openai is the Chat Completions compatibility adapter.");
            ask("Which service should answer? ")
        }) {
            Some(p) if !p.is_empty() => p,
            _ => {
                eprintln!("error: no provider chosen");
                eprintln!("       set VAK_SETUP_PROVIDER, or run without --non-interactive");
                return 2;
            }
        };
        if !core.provider_names().iter().any(|n| n == &provider) {
            eprintln!("error: unknown provider '{provider}'");
            return 2;
        }

        if let Some(key) = ask_secret(&format!("Paste the {provider} API key (blank to skip): "))
            && !key.is_empty()
            && let Err(e) = core.set_provider_key(&provider, &key)
        {
            eprintln!("error: could not store the key: {e}");
            return 2;
        }

        // A stored key is not success. The route is verified by asking the
        // provider what this key can actually reach (invariant 9).
        println!("asking {provider} which models your key can reach…");
        let discovered = core.discover_models(&provider).await;
        let model = match (answers.model.clone(), &discovered) {
            (Some(m), _) => m,
            (None, Ok(models)) if !models.is_empty() && interactive() => {
                for (i, m) in models.iter().take(20).enumerate() {
                    println!("  {:>2}. {m}", i + 1);
                }
                match ask("Model (number or exact id): ") {
                    Some(a) if a.is_empty() => {
                        eprintln!("error: no model chosen");
                        return 2;
                    }
                    Some(a) => a
                        .parse::<usize>()
                        .ok()
                        .and_then(|n| models.get(n.saturating_sub(1)).cloned())
                        .unwrap_or(a),
                    None => {
                        eprintln!("error: no model chosen and stdin is not a terminal");
                        eprintln!("       set VAK_SETUP_MODEL to choose one explicitly");
                        return 2;
                    }
                }
            }
            (None, Ok(_)) | (None, Err(_)) => {
                if let Err(e) = &discovered {
                    println!("  could not list models: {e}");
                }
                match ask("Enter an exact model id: ") {
                    Some(a) if !a.is_empty() => a,
                    _ => {
                        eprintln!("error: no model chosen");
                        return 2;
                    }
                }
            }
        };

        // Provider and model are one atomic route (invariant 17).
        if let Err(e) = vak_config::persist_global_preferences(
            Some(&provider),
            Some(&model),
            None,
            None,
            None,
            None,
        ) {
            eprintln!("error: could not save the route: {e}");
            return 2;
        }
        core.apply_persisted_route(provider.clone(), model.clone());
        println!("  route saved: {provider}/{model}");
        println!();
    }

    // --- safety posture -------------------------------------------------
    let posture = answers.posture.clone().or_else(|| {
        if !interactive() {
            return None;
        }
        println!("How much should vak be allowed to do on its own?");
        println!("  1. Inspect only        — reads and searches, changes nothing");
        println!("  2. Work with approval  — edits here, asks before shell commands (recommended)");
        println!("  3. Unrestricted        — full access to this machine, unsandboxed");
        ask("Choose [1/2/3]: ").map(|a| match a.as_str() {
            "1" => "read-only".to_string(),
            "3" => "full-access".to_string(),
            _ => "workspace-write".to_string(),
        })
    });
    match posture {
        Some(mode) => {
            let parsed = match mode.as_str() {
                "read-only" => vak_config::PermissionMode::ReadOnly,
                "workspace-write" => vak_config::PermissionMode::WorkspaceWrite,
                "full-access" => vak_config::PermissionMode::FullAccess,
                other => {
                    eprintln!("error: unknown posture '{other}'");
                    return 2;
                }
            };
            if let Err(e) =
                vak_config::persist_global_preferences(None, None, None, Some(parsed), None, None)
            {
                eprintln!("error: could not save the posture: {e}");
                return 2;
            }
            core.set_permission_mode(parsed);
            println!("  posture: {parsed:?}");
        }
        None => {
            eprintln!("error: no safety posture chosen and stdin is not a terminal");
            eprintln!("       set VAK_SETUP_POSTURE to read-only|workspace-write|full-access");
            return 2;
        }
    }
    println!();

    // --- seeds ----------------------------------------------------------
    let seed = answers.seed
        || matches!(ask("Install the starter skills? [Y/n] ").as_deref(), Some(a) if !a.eq_ignore_ascii_case("n"));
    if seed {
        if let Err(error) = vak_core::seed::seed_shared_capabilities() {
            eprintln!("error: {error}");
            return 2;
        }
        println!("  starter skills installed");
    }

    // --- activation -----------------------------------------------------
    let activate = answers.activate
        || matches!(ask("Run vak in the background (durable services)? [y/N] ").as_deref(), Some(a) if a.eq_ignore_ascii_case("y"));
    if activate {
        println!("  registering services…");
        let code = crate::install::run_services_sync(None, Vec::new());
        if code != 0 {
            eprintln!("warning: some services did not register; `vak self status` has detail");
        }
    }

    // --- first result ---------------------------------------------------
    //
    // Read-only is the strictest mode, so passing it as the run's override
    // can only ever cap — it cannot raise the ceiling whatever the posture
    // above was (doc 46 security invariant 5). The web wizard reaches the
    // same guarantee through `CorePool`; this reaches it through the
    // scoped override `exec` already takes.
    let first = answers.first_task
        || matches!(
            ask("Run a safe, read-only starter task now? [Y/n] ").as_deref(),
            Some(a) if !a.eq_ignore_ascii_case("n")
        );
    if first {
        println!();
        let code = crate::run_exec(
            cwd.clone(),
            vak_core::onboarding::FIRST_TASK_PROMPT.to_string(),
            None,
            None,
            None,
            // Matches `exec`'s own default rather than inventing a
            // second number the two could drift apart on.
            40,
            false,
            true,
            Some("read-only".to_string()),
            Vec::new(),
            false,
            None,
            false,
            None,
            Vec::new(),
            trusted,
        )
        .await;
        if code != 0 {
            eprintln!("the starter task did not finish; `vak doctor` has detail");
        }
    }

    println!();
    run_status(cwd, None, false)
}

/// `vak self state [--verify <snapshot>]`.
///
/// The plumbing the upgrade gate drives (doc 46 VII.5). Without `--verify`
/// it prints a snapshot of every durable file the registry declares; with
/// it, it compares the current state against a snapshot taken before an
/// update and reports every entry the contract forbids changing.
///
/// The comparison rules live in `vak_core::state`, beside the registry
/// they belong to, rather than in the script that calls this — one
/// definition, so a shell and a library cannot drift apart about what an
/// update is allowed to do.
pub fn run_state(verify: Option<PathBuf>) -> i32 {
    let current = vak_core::state::snapshot(env!("CARGO_PKG_VERSION"));
    let Some(path) = verify else {
        match serde_json::to_string_pretty(&current) {
            Ok(text) => {
                println!("{text}");
                return 0;
            }
            Err(e) => {
                eprintln!("error: {e}");
                return 2;
            }
        }
    };

    let raw = match std::fs::read_to_string(&path) {
        Ok(raw) => raw,
        Err(e) => {
            eprintln!("error: cannot read {}: {e}", path.display());
            return 2;
        }
    };
    let before: vak_core::state::StateSnapshot = match serde_json::from_str(&raw) {
        Ok(s) => s,
        Err(e) => {
            eprintln!("error: {} is not a state snapshot: {e}", path.display());
            return 2;
        }
    };

    let violations = vak_core::state::verify_upgrade(&before, &current);
    println!("upgrade check: {} → {}", before.version, current.version);
    if violations.is_empty() {
        println!("  ✓ every declared entry survived the update as its rule requires");
        return 0;
    }
    for violation in &violations {
        println!("  ✗ {violation}");
    }
    eprintln!();
    eprintln!(
        "{} entr(ies) changed in a way the update contract forbids \
         (docs/design/46-stabilization-install-and-onboarding.md VII.3).",
        violations.len()
    );
    1
}
