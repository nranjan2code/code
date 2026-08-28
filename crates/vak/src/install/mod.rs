//! Managed release lifecycle: install, reinstall, verify, status, update,
//! uninstall (docs/design/32-release-engineering.md).
//!
//! Design rules this module holds to, each of which had a counterexample
//! in the code it replaces:
//!
//! * One prefix resolution shared by every subcommand, so an install to a
//!   custom prefix stays inspectable and removable.
//! * Every mutation is a transaction that rolls back, so a failure never
//!   leaves a half-installed prefix.
//! * The manifest records a digest per component, so `status` reports
//!   drift instead of assuming its absence.
//! * Version decisions are semantic, never lexical.
//! * A downloaded artifact is verified before it is allowed near the
//!   install root.

pub mod atomic;
pub mod bundle;
pub mod digest;
pub mod feed;
pub mod layout;
pub mod manifest;
pub mod transaction;

use std::io::{IsTerminal as _, Write as _};
use std::path::{Path, PathBuf};

use feed::{Decision, Feed};
use layout::InstallRoot;
use manifest::{Component, Manifest};
use transaction::Transaction;

/// What a release is made of. Only the CLI is required; the rest ride
/// along when the build produced them, and their absence is normal
/// rather than a defect.
struct ComponentSpec {
    name: &'static str,
    required: bool,
}

const COMPONENTS: &[ComponentSpec] = &[
    ComponentSpec {
        name: "vak",
        required: true,
    },
    ComponentSpec {
        name: "vak-desktop",
        required: false,
    },
    ComponentSpec {
        name: "vak-delivery-worker",
        required: false,
    },
];

fn confirm(prompt: &str, yes: bool) -> bool {
    if yes || !std::io::stdin().is_terminal() {
        // Non-interactive without --yes is a refusal, not an assumption:
        // a script must say so explicitly before anything is replaced.
        return yes;
    }
    print!("{prompt} [y/N] ");
    let _ = std::io::stdout().flush();
    let mut line = String::new();
    let _ = std::io::stdin().read_line(&mut line);
    line.trim().eq_ignore_ascii_case("y")
}

// ---------------------------------------------------------------- install

/// Install the running binary and its siblings into `prefix`.
pub fn run_install(prefix: Option<PathBuf>, force: bool) -> i32 {
    let root = InstallRoot::resolve(prefix);
    match install_into(&root, force) {
        Ok(m) => {
            println!(
                "installed {} ({}) → {}",
                m.version,
                m.git_sha,
                root.prefix().display()
            );
            for c in &m.components {
                println!("  {:<22} {}", c.name, c.path.display());
            }
            bootstrap_default_workspace_if_fresh();
            report_next_steps(&root);
            0
        }
        Err(e) => {
            eprintln!("error: {e}");
            1
        }
    }
}

/// Points the durable-service units at a real workspace on a truly fresh
/// install, instead of leaving "which directory" to whatever cwd
/// `self services-sync` is later run from (docs/design/32 invariant 3 is
/// correct as a mechanism, but named no default — an operator who ran
/// `services-sync` from inside the vak source checkout while developing
/// it got an always-on Telegram bridge silently bound to the dev repo).
///
/// Only acts when NEITHER service is configured with the platform
/// service manager yet (`vak_ops::status` reports `NotInstalled` for
/// both) — an operator who already made a workspace choice, on this
/// install or an earlier one, is never silently rebound. The gateway
/// server is left running so the admin console is reachable right away;
/// unattended remote chat surfaces stay fail-closed on their own axis
/// (empty `gateway.chat_allowlist`, `AGENTS.md` invariant 15) regardless
/// of whether the server process itself is up. Telegram stays stopped
/// until a bot token is actually configured.
fn bootstrap_default_workspace_if_fresh() {
    let cfg = vak_ops::OpsConfig::detect();
    let already_configured = vak_ops::status(vak_ops::Service::Gateway, &cfg)
        != vak_ops::State::NotInstalled
        || vak_ops::status(vak_ops::Service::Telegram, &cfg) != vak_ops::State::NotInstalled;
    if already_configured {
        return;
    }
    let workspace = vak_config::paths::default_workspace();
    if let Err(e) = std::fs::create_dir_all(&workspace) {
        eprintln!(
            "warning: could not create default workspace {}: {e}",
            workspace.display()
        );
        return;
    }
    let original_cwd = std::env::current_dir().ok();
    if std::env::set_current_dir(&workspace).is_err() {
        eprintln!(
            "warning: could not switch into default workspace {}",
            workspace.display()
        );
        return;
    }
    println!();
    println!(
        "no services configured yet — bootstrapping the default workspace at {}",
        workspace.display()
    );
    let code = run_services_sync(None, Vec::new());
    if let Some(cwd) = original_cwd {
        let _ = std::env::set_current_dir(cwd);
    }
    // Leave the gateway server running: it's what serves the admin
    // console, and an operator needs that reachable right after install
    // to actually configure anything (provider key, channels) — a
    // service that's "stopped until you dig up a separate start step"
    // just moves the same footgun one step later. This does not weaken
    // "gateway ships disabled" (AGENTS.md invariant 15): that invariant
    // is about *unattended remote surfaces* accepting inbound chat, which
    // stays fail-closed on its own axis — `gateway.chat_allowlist` is
    // empty on a fresh install, so every inbound message still gets
    // rejected into `pending` until an operator approves one via the
    // admin console this server now makes reachable. The HTTP server
    // itself is loopback-only and bearer-token gated regardless.
    //
    // The Telegram bridge is different: with no bot token configured yet
    // it has nothing to poll and would just crash-loop under KeepAlive,
    // so it's stopped (not started) until a token is set — the same
    // point where Desktop Settings / the admin console already restarts
    // it automatically on save.
    vak_ops::stop(vak_ops::Service::Telegram, &cfg);
    // The desktop service needs no special handling here: it is in
    // SERVICES, so the sync above wrote and loaded its unit, and its
    // RunAtLoad started it — with `--tray`, so a fresh install ends with a
    // live menu-bar icon and no window the operator did not ask for. It is
    // skipped entirely when the build shipped no `vak-desktop` binary
    // (`default_service_names`), which is what keeps a headless server from
    // acquiring a GUI unit that could only ever fail.
    if code == 0 {
        println!(
            "gateway is running at {} — open the admin console to set a provider key \
             and approve channels; Telegram stays stopped until a bot token is configured",
            workspace.display()
        );
    }
}

/// Remove an existing install and place a fresh one. Distinct from
/// `install` because installing over a tree leaves files from the old
/// version that the new manifest does not describe.
pub fn run_reinstall(prefix: Option<PathBuf>, yes: bool) -> i32 {
    let root = InstallRoot::resolve(prefix);
    if root.is_installed()
        && !confirm(
            &format!("remove and reinstall {}?", root.prefix().display()),
            yes,
        )
    {
        println!("aborted");
        return 0;
    }
    if root.is_installed() {
        // Services keep running against the old inode until sync; the
        // data home is untouched, so this is not destructive to state.
        if let Err(e) = std::fs::remove_dir_all(root.prefix()) {
            eprintln!("error: clear {}: {e}", root.prefix().display());
            return 1;
        }
    }
    run_install(Some(root.prefix().to_path_buf()), true)
}

fn install_into(root: &InstallRoot, force: bool) -> Result<Manifest, String> {
    let exe = std::env::current_exe().map_err(|e| format!("cannot locate running binary: {e}"))?;
    let build_dir = exe
        .parent()
        .ok_or_else(|| "running binary has no parent directory".to_string())?;

    if root.is_installed() && !force {
        let existing = Manifest::read(root)?;
        if existing.version == manifest::build_version()
            && existing.git_sha == manifest::build_git_sha()
        {
            return Err(format!(
                "{} is already at {} ({}) — use `--force` to reinstall the same build",
                root.prefix().display(),
                existing.version,
                existing.git_sha
            ));
        }
    }

    let bin_dir = root.bin_dir();
    let mut tx = Transaction::begin(root)?;
    let mut components = Vec::new();

    for spec in COMPONENTS {
        // The CLI is the binary we are running; siblings come from the
        // same build directory.
        let source = if spec.name == "vak" {
            exe.clone()
        } else {
            build_dir.join(spec.name)
        };
        if !source.exists() {
            if spec.required {
                return Err(format!(
                    "required component {} not found at {}",
                    spec.name,
                    source.display()
                ));
            }
            continue;
        }
        let destination = bin_dir.join(spec.name);
        tx.stage_file(spec.name, &source, destination.clone(), true)?;
        components.push(Component {
            name: spec.name.to_string(),
            path: destination,
            sha256: digest::of_file(&source)?,
            required: spec.required,
        });
    }

    tx.commit()?;

    // Before the desktop app is opened again, retire the old independently
    // launchd-owned tray. It lived inside the same .app bundle and made macOS
    // activate a background process instead of launching the desktop window.
    retire_legacy_tray(root);

    // Pin the gateway's bearer token into the canonical .env, once,
    // rather than letting it re-mint on every boot (the un-pinned
    // fallback in vak-server::AppState::new). A token that changes every
    // restart invalidates every admin-console session cookie and any
    // saved one-click login link on each restart; it also means nothing
    // outside the running process -- the tray's "Open Admin Console",
    // in particular -- can know the current token to build a URL with.
    // Loaded automatically on every future boot: main.rs always sources
    // this file via `vak_config::user_env_path()` before dispatching to
    // `serve`. Idempotent: an existing token is left alone so a
    // reinstall or update never invalidates a link already in use.
    ensure_gateway_token()?;

    let version = manifest::build_version().to_string();
    bundle::write_metadata(root, &version, bundle::locate_frontend_assets().as_deref())?;

    let m = Manifest::new(version, root.prefix().to_path_buf(), components);
    m.write(root)?;

    // Verify what we just wrote rather than trusting that we wrote it.
    let defects = m.verify();
    if !defects.is_empty() {
        return Err(format!(
            "install completed but does not verify: {}",
            defects
                .iter()
                .map(ToString::to_string)
                .collect::<Vec<_>>()
                .join("; ")
        ));
    }
    Ok(m)
}

fn retire_legacy_tray(root: &InstallRoot) {
    let retired = vak_ops::services::RETIRED_SERVICES;
    if !retired.is_empty() {
        let _ = vak_ops::services::services_uninstall(
            retired,
            &vak_ops::services::Paths::default(),
            &vak_ops::services::SystemRunner,
        );
    }
    let obsolete_binary = root.bin_dir().join("vak-tray");
    if let Err(e) = std::fs::remove_file(&obsolete_binary)
        && e.kind() != std::io::ErrorKind::NotFound
    {
        eprintln!(
            "warning: could not remove retired tray binary {}: {e}",
            obsolete_binary.display()
        );
    }
}

/// Ensure `VAK_GATEWAY_TOKEN` is set in the canonical `.env`,
/// generating one only if the key is entirely absent (an existing empty
/// value is left as-is too -- that is an explicit "unpinned" choice, not
/// something install should override).
fn ensure_gateway_token() -> Result<(), String> {
    let Some(env_path) = vak_config::user_env_path() else {
        return Ok(());
    };
    let existing = std::fs::read_to_string(&env_path).unwrap_or_default();
    let already_set = existing.lines().any(|line| {
        line.split_once('=')
            .is_some_and(|(k, _)| k.trim() == "VAK_GATEWAY_TOKEN")
    });
    if already_set {
        return Ok(());
    }
    let token = format!("vk_{}", uuid::Uuid::now_v7());
    vak_config::upsert_env_file(&env_path, "VAK_GATEWAY_TOKEN", &token)
        .map_err(|e| format!("writing {}: {e}", env_path.display()))
}

fn report_next_steps(root: &InstallRoot) {
    let cli = root.bin_dir().join("vak");
    let on_path = std::env::var_os("PATH")
        .map(|p| std::env::split_paths(&p).any(|d| d == root.bin_dir()))
        .unwrap_or(false);
    if !on_path {
        println!();
        println!("the CLI is not on PATH; either add it:");
        println!("  export PATH=\"{}:$PATH\"", root.bin_dir().display());
        println!("or link it:");
        println!("  ln -sf {} /usr/local/bin/vak", cli.display());
    }
    println!();
    println!("next: vak self services-sync");
}

// ----------------------------------------------------------------- verify

/// Check an install against its manifest.
pub fn run_verify(prefix: Option<PathBuf>) -> i32 {
    let root = InstallRoot::resolve(prefix);
    let m = match Manifest::read(&root) {
        Ok(m) => m,
        Err(e) => {
            eprintln!("error: {e}");
            return 2;
        }
    };
    let defects = m.verify();
    println!("prefix    {}", root.prefix().display());
    println!("version   {} ({})", m.version, m.git_sha);
    for c in &m.components {
        let mark = if defects.iter().any(|d| defect_names(d) == c.name) {
            "✗"
        } else if c.path.exists() {
            "✓"
        } else {
            "·"
        };
        println!("  {mark} {:<22} {}", c.name, c.path.display());
    }
    if defects.is_empty() {
        println!("install verifies clean");
        0
    } else {
        for d in &defects {
            eprintln!("defect: {d}");
        }
        eprintln!("repair with: vak self reinstall");
        1
    }
}

fn defect_names(d: &manifest::Defect) -> &str {
    match d {
        manifest::Defect::Missing { name, .. }
        | manifest::Defect::Corrupt { name, .. }
        | manifest::Defect::Unreadable { name, .. } => name,
    }
}

// ----------------------------------------------------------------- status

pub fn run_status(prefix: Option<PathBuf>) -> i32 {
    let root = InstallRoot::resolve(prefix);
    let build = manifest::build_version();
    println!("build     {} ({})", build, manifest::build_git_sha());

    let m = match Manifest::read(&root) {
        Ok(m) => m,
        Err(e) => {
            println!("manifest  — ({e})");
            return 1;
        }
    };
    println!(
        "prefix    {}{}",
        root.prefix().display(),
        if root.is_bundle() {
            " (app bundle)"
        } else {
            ""
        }
    );
    println!("manifest  {} installed {}", m.version, m.installed_at);

    let mut drifted = false;
    let defects = m.verify();
    for d in &defects {
        eprintln!("drift: {d}");
        drifted = true;
    }

    let cli = match m.cli_path() {
        Ok(p) => p,
        Err(e) => {
            eprintln!("drift: {e}");
            return 1;
        }
    };
    let rows = vak_ops::services::services_status(
        &cli,
        // Same set `services-sync` would write, so an optional component
        // this build never shipped is not reported as an unregistered
        // service the operator is told to go and sync.
        &vak_ops::services::default_service_names(&cli),
        &vak_ops::services::Paths::default(),
        &vak_ops::services::SystemRunner,
    );
    for r in &rows {
        let state = match r.running_pid {
            Some(pid) => format!("running (pid {pid})"),
            None => "down".to_string(),
        };
        // "never synced" and "synced against the wrong binary" both leave
        // unit_points_at_installed false, but they need different
        // instructions — and a fresh install is always the former.
        let flag = if !r.unit_present {
            "not registered — run `self services-sync`"
        } else if !r.unit_points_at_installed {
            "✗ execs outside the managed prefix — run `self services-sync`"
        } else if r.binary_stale {
            "⚠ stale process — run `self services-sync`"
        } else {
            "✓"
        };
        println!(
            "service   {} {state} · {} · {flag}",
            r.name,
            r.unit_path.display()
        );
        if !r.unit_points_at_installed || r.binary_stale {
            drifted = true;
        }
    }

    // Compare the manifest against the binary it describes, not against
    // whichever build happens to be running this command — otherwise a
    // dev build always reports drift against a good install.
    if let Ok(installed_version) = installed_cli_version(&cli)
        && installed_version != m.version
    {
        eprintln!(
            "drift: manifest says {} but the installed binary reports {installed_version}",
            m.version
        );
        drifted = true;
    }
    if m.version != build {
        println!(
            "note      running build {build} differs from the install ({}) — expected when running from a source tree",
            m.version
        );
    }

    if drifted { 1 } else { 0 }
}

/// Ask the installed binary what version it is. Used only to detect
/// manifest drift; a failure here is not itself a defect.
fn installed_cli_version(cli: &Path) -> Result<String, String> {
    let out = std::process::Command::new(cli)
        .arg("--version")
        .output()
        .map_err(|e| format!("exec {}: {e}", cli.display()))?;
    let text = String::from_utf8_lossy(&out.stdout);
    text.split_whitespace()
        .find_map(|t| feed::parse_version(t).ok())
        .map(|v| v.to_string())
        .ok_or_else(|| format!("no version in `{} --version` output", cli.display()))
}

// ---------------------------------------------------------- services-sync

pub fn run_services_sync(prefix: Option<PathBuf>, names: Vec<String>) -> i32 {
    let root = InstallRoot::resolve(prefix);
    let cli = match Manifest::read(&root).and_then(|m| m.cli_path()) {
        Ok(p) => p,
        Err(e) => {
            eprintln!("error: {e}");
            return 1;
        }
    };
    let requested: Vec<&str> = if names.is_empty() {
        let retired = vak_ops::services::RETIRED_SERVICES;
        if !retired.is_empty()
            && let Err(e) = vak_ops::services::services_uninstall(
                retired,
                &vak_ops::services::Paths::default(),
                &vak_ops::services::SystemRunner,
            )
        {
            eprintln!("warning: retired tray teardown incomplete: {e}");
        }
        vak_ops::services::default_service_names(&cli)
    } else {
        names.iter().map(String::as_str).collect()
    };
    let outcomes = vak_ops::services::services_sync(
        &cli,
        &requested,
        &vak_ops::services::Paths::default(),
        &vak_ops::services::SystemRunner,
    );
    let mut failed = false;
    for o in outcomes {
        match o.action {
            vak_ops::services::SyncAction::Failed(e) => {
                failed = true;
                println!("✗ {}: {e}", o.name);
            }
            action => println!("✓ {}: {action:?}", o.name),
        }
    }
    if failed { 1 } else { 0 }
}

// ----------------------------------------------------------------- update

/// Pull a newer release from a feed and replace every component at once.
///
/// The fetch uses a blocking HTTP client, and these subcommands dispatch
/// from inside the CLI's tokio runtime — constructing or dropping a
/// blocking client there panics ("Cannot drop a runtime in a context
/// where blocking is not allowed"). The whole transfer therefore runs on
/// a dedicated thread, the same confinement `update_check` uses.
pub fn run_update(prefix: Option<PathBuf>, url: &str, yes: bool, dry_run: bool) -> i32 {
    let root = InstallRoot::resolve(prefix);
    let outcome = {
        let root = root.clone();
        let url = url.to_string();
        match std::thread::spawn(move || update(&root, &url, yes, dry_run)).join() {
            Ok(r) => r,
            Err(_) => Err("update thread panicked".to_string()),
        }
    };
    match outcome {
        Ok(Some(version)) => {
            println!("updated → {version}");
            println!("reloading services…");
            run_services_sync(Some(root.prefix().to_path_buf()), Vec::new())
        }
        Ok(None) => 0,
        Err(e) => {
            eprintln!("error: {e}");
            1
        }
    }
}

fn update(
    root: &InstallRoot,
    url: &str,
    yes: bool,
    dry_run: bool,
) -> Result<Option<String>, String> {
    let mut installed = Manifest::read(root)?;

    let client = reqwest::blocking::Client::builder()
        .timeout(std::time::Duration::from_secs(60))
        .build()
        .map_err(|e| format!("http client: {e}"))?;

    let body = client
        .get(url)
        .send()
        .and_then(reqwest::blocking::Response::error_for_status)
        .map_err(|e| format!("release feed unreachable at {url}: {e}"))?
        .bytes()
        .map_err(|e| format!("release feed body: {e}"))?;
    let feed = Feed::parse(&body)?;

    // Compare against what is installed, not against the running build.
    match feed.decide(&installed.version)? {
        Decision::UpToDate { installed, offered } => {
            println!("up to date ({installed} installed, {offered} offered)");
            return Ok(None);
        }
        Decision::Upgrade { from, to } => {
            println!("update available: {from} → {to}");
            if let Some(notes) = &feed.notes_url {
                println!("notes: {notes}");
            }
            if dry_run {
                return Ok(None);
            }
            if !confirm(&format!("update {from} → {to}?"), yes) {
                println!("aborted");
                return Ok(None);
            }
        }
    }

    let key = feed::platform_key();
    let artifacts = feed.artifacts_for(&key)?;

    // Download and verify every artifact before touching the install.
    let mut tx = Transaction::begin(root)?;
    let mut next_components = Vec::new();
    for artifact in &artifacts {
        println!("  fetching {}…", artifact.name);
        let bytes = client
            .get(&artifact.url)
            .send()
            .and_then(reqwest::blocking::Response::error_for_status)
            .map_err(|e| format!("download {}: {e}", artifact.name))?
            .bytes()
            .map_err(|e| format!("read {}: {e}", artifact.name))?;
        let actual = digest::of_bytes(&bytes);
        if !digest::matches(&artifact.sha256, &actual) {
            return Err(format!(
                "{} failed integrity check — expected {}, got {actual}. Nothing was installed.",
                artifact.name,
                artifact.sha256.trim()
            ));
        }
        let destination = installed
            .component(&artifact.name)
            .map(|c| c.path.clone())
            .unwrap_or_else(|| root.bin_dir().join(&artifact.name));
        tx.stage_bytes(&artifact.name, &bytes, destination.clone(), true)?;
        next_components.push(Component {
            name: artifact.name.clone(),
            path: destination,
            sha256: actual,
            required: artifact.required,
        });
    }

    if tx.is_empty() {
        return Err(format!("release {} offers nothing for {key}", feed.version));
    }
    tx.commit()?;

    // Carry forward components the feed did not ship, so the manifest
    // keeps describing the whole install rather than only what moved.
    for existing in installed.components.clone() {
        if !next_components.iter().any(|c| c.name == existing.name) {
            next_components.push(existing);
        }
    }
    installed.version = feed.version()?.to_string();
    installed.installed_at = manifest::now_rfc3339();
    installed.components = next_components;
    installed.write(root)?;

    bundle::write_metadata(root, &installed.version, None)?;
    Ok(Some(installed.version.clone()))
}

// -------------------------------------------------------------- uninstall

pub fn run_uninstall(prefix: Option<PathBuf>, yes: bool, purge: bool) -> i32 {
    let root = InstallRoot::resolve(prefix);
    if !confirm(
        &format!(
            "stop services and remove the managed install at {}?",
            root.prefix().display()
        ),
        yes,
    ) {
        println!("aborted");
        return 0;
    }

    let names: Vec<&str> = vak_ops::services::SERVICES.iter().map(|d| d.name).collect();
    if let Err(e) = vak_ops::services::services_uninstall(
        &names,
        &vak_ops::services::Paths::default(),
        &vak_ops::services::SystemRunner,
    ) {
        eprintln!("warning: service teardown incomplete: {e}");
    }

    // Report components installed elsewhere, which removing this prefix
    // will not reach.
    if let Ok(m) = Manifest::read(&root) {
        for c in &m.components {
            if !c.path.starts_with(root.prefix()) {
                eprintln!(
                    "note: {} lives outside the prefix and was left in place: {}",
                    c.name,
                    c.path.display()
                );
            }
        }
    }

    match std::fs::remove_dir_all(root.prefix()) {
        Ok(()) => println!("removed {}", root.prefix().display()),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            println!("nothing installed at {}", root.prefix().display());
        }
        Err(e) => eprintln!("warning: remove {}: {e}", root.prefix().display()),
    }

    if purge {
        let data = vak_config::paths::data_home();
        if !confirm(
            &format!(
                "ALSO delete the data home at {} (sessions, memory, tasks)?",
                data.display()
            ),
            yes,
        ) {
            println!("kept data home at {}", data.display());
            return 0;
        }
        match std::fs::remove_dir_all(&data) {
            Ok(()) => println!("purged {}", data.display()),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(e) => eprintln!("warning: purge {}: {e}", data.display()),
        }
    }
    0
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::*;

    #[test]
    fn confirm_refuses_rather_than_assumes_when_not_interactive() {
        // A piped stdin with no --yes must not be read as consent.
        assert!(!confirm("do the thing?", false));
        assert!(confirm("do the thing?", true));
    }

    #[test]
    fn every_optional_component_is_declared_optional() {
        let required: Vec<&str> = COMPONENTS
            .iter()
            .filter(|c| c.required)
            .map(|c| c.name)
            .collect();
        assert_eq!(
            required,
            vec!["vak"],
            "only the CLI is required; the rest ride along when built"
        );
    }
}
