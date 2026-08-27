//! Managed release lifecycle (docs/design/32-release-engineering.md).

use std::io::{IsTerminal as _, Write as _};
use std::path::{Path, PathBuf};

const BIN_DIR: &str = "bin";
const INSTALL_POINTER: &str = "install-root.json";
const GATEWAY_TOKEN_KEY: &str = "VAKCODER_GATEWAY_TOKEN";

fn home() -> PathBuf {
    // Canonical data home (doc 32) is the anchor for the managed release prefix.
    vak_config::paths::data_home()
}

fn prefix_default() -> PathBuf {
    home().join("runtime-bin")
}

#[derive(serde::Serialize, serde::Deserialize)]
struct InstallPointer {
    root: PathBuf,
}

fn pointer_path() -> PathBuf {
    home().join(INSTALL_POINTER)
}

fn active_prefix() -> PathBuf {
    std::fs::read(pointer_path())
        .ok()
        .and_then(|raw| serde_json::from_slice::<InstallPointer>(&raw).ok())
        .map_or_else(prefix_default, |pointer| pointer.root)
}

fn bin_dir_of(prefix: &Path) -> PathBuf {
    prefix.join("current").join(BIN_DIR)
}

fn manifest_path(prefix: &Path) -> PathBuf {
    prefix.join("install.json")
}

fn version_dir(prefix: &Path, version: &str) -> PathBuf {
    prefix.join("versions").join(version)
}

#[derive(serde::Serialize, serde::Deserialize, Debug)]
struct Manifest {
    version: String,
    git_sha: String,
    installed_at: String,
    binaries: Vec<(String, PathBuf)>,
}

fn write_atomic(path: &Path, bytes: &[u8]) -> Result<(), String> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|e| format!("mkdir {}: {e}", parent.display()))?;
    }
    let tmp = path.with_extension("tmp");
    std::fs::write(&tmp, bytes).map_err(|e| format!("write {}: {e}", tmp.display()))?;
    std::fs::rename(&tmp, path).map_err(|e| format!("rename into {}: {e}", path.display()))
}

fn copy_executable(src: &Path, dst: &Path) -> Result<(), String> {
    let bytes = std::fs::read(src).map_err(|e| format!("read {}: {e}", src.display()))?;
    write_atomic(dst, &bytes)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(dst, std::fs::Permissions::from_mode(0o755))
            .map_err(|e| format!("chmod {}: {e}", dst.display()))?;
    }
    Ok(())
}

/// Mint a 256-bit bearer token from the OS CSPRNG, hex-encoded.
fn mint_gateway_token() -> Result<String, String> {
    let mut bytes = [0u8; 32];
    getrandom::fill(&mut bytes).map_err(|e| format!("system CSPRNG unavailable: {e}"))?;
    Ok(bytes.iter().fold(String::with_capacity(64), |mut acc, b| {
        use std::fmt::Write as _;
        let _ = write!(acc, "{b:02x}");
        acc
    }))
}

/// The managed gateway refuses to start without a bearer token, and a service
/// manager cannot prompt for one. Provisioning is therefore part of install,
/// and is idempotent: an existing user secret is never rewritten.
///
/// Returns whether a new token was minted.
fn ensure_gateway_token() -> Result<bool, String> {
    ensure_gateway_token_in(&vak_config::SecretService::new(home()))
}

fn ensure_gateway_token_in(secrets: &vak_config::SecretService) -> Result<bool, String> {
    match secrets.get(GATEWAY_TOKEN_KEY) {
        Ok(Some(existing)) if !existing.trim().is_empty() => return Ok(false),
        Ok(_) => {}
        Err(e) => return Err(format!("read {}: {e}", secrets.path().display())),
    }
    let token = mint_gateway_token()?;
    secrets
        .set(GATEWAY_TOKEN_KEY, &token)
        .map_err(|e| format!("write {}: {e}", secrets.path().display()))?;
    Ok(true)
}

fn current_git_sha() -> String {
    option_env!("VAKCODER_GIT_SHA")
        .map(str::to_string)
        .unwrap_or_else(|| "unknown".into())
}

#[cfg(unix)]
fn replace_symlink(link: &Path, target: &Path) -> Result<(), String> {
    use std::os::unix::fs::symlink;

    if let Some(parent) = link.parent() {
        std::fs::create_dir_all(parent).map_err(|e| format!("mkdir {}: {e}", parent.display()))?;
    }
    let temporary = link.with_extension(format!("tmp.{}", std::process::id()));
    let _ = std::fs::remove_file(&temporary);
    symlink(target, &temporary).map_err(|e| format!("symlink {}: {e}", temporary.display()))?;
    std::fs::rename(&temporary, link).map_err(|e| format!("activate {}: {e}", link.display()))
}

#[cfg(windows)]
fn replace_symlink(link: &Path, target: &Path) -> Result<(), String> {
    let source = target.join(BIN_DIR);
    let staged = link.with_extension(format!("tmp.{}", std::process::id()));
    if staged.exists() {
        std::fs::remove_dir_all(&staged).map_err(|e| e.to_string())?;
    }
    std::fs::create_dir_all(&staged).map_err(|e| e.to_string())?;
    for entry in std::fs::read_dir(source).map_err(|e| e.to_string())? {
        let entry = entry.map_err(|e| e.to_string())?;
        std::fs::copy(entry.path(), staged.join(entry.file_name())).map_err(|e| e.to_string())?;
    }
    if link.exists() {
        std::fs::remove_dir_all(link).map_err(|e| e.to_string())?;
    }
    std::fs::rename(staged, link).map_err(|e| e.to_string())
}

fn install_cli_link(prefix: &Path) -> Result<Option<PathBuf>, String> {
    #[cfg(unix)]
    {
        let Some(user_home) = std::env::var_os("HOME") else {
            return Ok(None);
        };
        let link = PathBuf::from(user_home).join(".local/bin/vakcoder");
        if link.exists()
            && std::fs::symlink_metadata(&link).is_ok_and(|meta| !meta.file_type().is_symlink())
        {
            return Err(format!(
                "{} already exists and is not a managed symlink",
                link.display()
            ));
        }
        replace_symlink(&link, &bin_dir_of(prefix).join("vakcoder"))?;
        Ok(Some(link))
    }
    #[cfg(not(unix))]
    {
        let _ = prefix;
        Ok(None)
    }
}

pub(crate) fn run_install(prefix: Option<PathBuf>, no_service: bool) -> i32 {
    let prefix = prefix.unwrap_or_else(prefix_default);
    let prefix = if prefix.is_absolute() {
        prefix
    } else {
        match std::env::current_dir() {
            Ok(cwd) => cwd.join(prefix),
            Err(e) => {
                eprintln!("error: cannot resolve install prefix: {e}");
                return 1;
            }
        }
    };
    let exe = match std::env::current_exe() {
        Ok(e) => e,
        Err(e) => {
            eprintln!("error: cannot locate running binary: {e}");
            return 1;
        }
    };
    let staged_root = version_dir(&prefix, env!("CARGO_PKG_VERSION"));
    let bin_dir = staged_root.join(BIN_DIR);
    let mut binaries = Vec::new();
    if let Err(e) = copy_executable(&exe, &bin_dir.join("vakcoder")) {
        eprintln!("error: {e}");
        return 1;
    }
    binaries.push(("vakcoder".into(), bin_dir.join("vakcoder")));
    // The delivery worker is part of the base. Presentation clients are
    // separate packages and are never copied opportunistically.
    if let Some(sibling) = exe.parent().map(|p| p.join("vak-delivery-worker"))
        && sibling.exists()
        && let Err(e) = copy_executable(&sibling, &bin_dir.join("vak-delivery-worker"))
    {
        eprintln!("warning: delivery worker not installed: {e}");
    }
    // Preserve an explicitly installed tray addon when a base reinstall advances `current`.
    let previous_bin = bin_dir_of(&prefix);
    let previous_tray = previous_bin.join("vakcoder-tray");
    if previous_tray.exists()
        && let Err(e) = copy_executable(&previous_tray, &bin_dir.join("vakcoder-tray"))
    {
        eprintln!("warning: tray addon not preserved: {e}");
    }
    if let Err(e) = replace_symlink(&prefix.join("current"), &staged_root) {
        eprintln!("error: {e}");
        return 1;
    }

    let stable_bin = bin_dir_of(&prefix);
    for (_, path) in &mut binaries {
        if let Some(name) = path.file_name() {
            *path = stable_bin.join(name);
        }
    }

    let manifest = Manifest {
        version: env!("CARGO_PKG_VERSION").to_string(),
        git_sha: current_git_sha(),
        installed_at: chrono::Utc::now().to_rfc3339(),
        binaries,
    };
    let json = match serde_json::to_vec_pretty(&manifest) {
        Ok(v) => v,
        Err(e) => {
            eprintln!("error: manifest serialize: {e}");
            return 1;
        }
    };
    if let Err(e) = write_atomic(&manifest_path(&prefix), &json) {
        eprintln!("error: {e}");
        return 1;
    }
    let pointer = InstallPointer {
        root: prefix.clone(),
    };
    let pointer_json = match serde_json::to_vec_pretty(&pointer) {
        Ok(json) => json,
        Err(e) => {
            eprintln!("error: install pointer serialize: {e}");
            return 1;
        }
    };
    if let Err(e) = write_atomic(&pointer_path(), &pointer_json) {
        eprintln!("error: {e}");
        return 1;
    }
    println!(
        "installed {} ({}) → {}",
        manifest.version,
        manifest.git_sha,
        prefix.display()
    );
    for (name, path) in &manifest.binaries {
        println!("  {name}: {}", path.display());
    }
    match install_cli_link(&prefix) {
        Ok(Some(link)) => println!("  launcher: {}", link.display()),
        Ok(None) => {}
        Err(e) => eprintln!("warning: CLI launcher not installed: {e}"),
    }

    // The gateway service starts unattended and cannot prompt, so its bearer
    // token must exist before the unit is loaded.
    match ensure_gateway_token() {
        Ok(true) => println!(
            "  gateway token: minted in {}",
            home().join(".env").display()
        ),
        Ok(false) => println!("  gateway token: already provisioned"),
        Err(e) => {
            eprintln!("error: gateway token not provisioned: {e}");
            return 1;
        }
    }

    // A base install owns only the base service. Optional channel bridges and
    // presentation addons must be enabled explicitly after their credentials
    // or binaries exist.
    if no_service {
        println!("start the Runtime with `vakcoder serve --gateway`.");
        return 0;
    }

    let svc_names = ["com.vakcoder.gateway"];
    let outcomes = vak_ops::services::services_sync(
        &stable_bin.join("vakcoder"),
        &svc_names,
        &vak_ops::services::Paths::default(),
        &vak_ops::services::SystemRunner,
    );
    let mut service_failed = false;
    for o in &outcomes {
        match &o.action {
            vak_ops::services::SyncAction::Failed(e) => {
                service_failed = true;
                eprintln!("  warning: {}: {e}", o.name);
            }
            action => println!("  service: {} — {action:?}", o.name),
        }
    }
    if service_failed {
        eprintln!("install incomplete: the gateway service did not load");
        return 1;
    }

    // An install that reports success without a reachable Runtime is the
    // worst possible outcome: every surface fails later with no explanation.
    match await_gateway_ready() {
        Ok(connection) => println!("  gateway: ready at {}", connection.base_url()),
        Err(e) => {
            eprintln!("  warning: gateway did not become ready: {e}");
            eprintln!(
                "  diagnose with `vakcoder doctor`; logs: {}",
                vak_config::paths::logs_dir().join("gateway.log").display()
            );
            return 1;
        }
    }

    println!("open the admin console with `vakcoder admin`.");
    0
}

/// Poll the canonical receipt until the freshly loaded gateway answers its
/// authenticated handshake. Service managers return as soon as the unit is
/// loaded, which is strictly before the listener exists.
fn await_gateway_ready() -> Result<vak_client::GatewayConnection, String> {
    const ATTEMPTS: u32 = 40;
    const INTERVAL: std::time::Duration = std::time::Duration::from_millis(250);
    let mut last = "the gateway never published a receipt".to_string();
    for _ in 0..ATTEMPTS {
        match vak_client::GatewayConnection::discover_local() {
            Ok(connection) => {
                if vak_ops::health_ok(&vak_ops::OpsConfig::default()) {
                    return Ok(connection);
                }
                last = "the gateway published a receipt but failed its handshake".to_string();
            }
            Err(e) => last = e.to_string(),
        }
        std::thread::sleep(INTERVAL);
    }
    Err(last)
}

fn read_manifest(prefix: &Path) -> Result<Manifest, String> {
    let raw = std::fs::read(manifest_path(prefix))
        .map_err(|_| "no managed install — run `vakcoder self install` first".to_string())?;
    serde_json::from_slice(&raw).map_err(|e| format!("manifest unreadable: {e}"))
}

fn installed_bin(prefix: &Path) -> Result<PathBuf, String> {
    read_manifest(prefix)?
        .binaries
        .iter()
        .find(|(n, _)| n == "vakcoder")
        .map(|(_, p)| p.clone())
        .ok_or_else(|| "manifest lacks vakcoder entry".into())
}

pub(crate) fn run_services_sync(names: Vec<String>) -> i32 {
    let prefix = active_prefix();
    let bin = match installed_bin(&prefix) {
        Ok(b) => b,
        Err(e) => {
            eprintln!("error: {e}");
            return 1;
        }
    };
    let requested: Vec<&str> = if names.is_empty() {
        vak_ops::services::SERVICES.iter().map(|d| d.name).collect()
    } else {
        names.iter().map(String::as_str).collect()
    };
    let outcomes = vak_ops::services::services_sync(
        &bin,
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

/// One row of the drift matrix.
struct StatusRow {
    name: String,
    unit_path: PathBuf,
    unit_exists: bool,
    points_at_installed: bool,
    binary_stale: bool,
    pid: Option<u32>,
}

pub(crate) fn run_status() -> i32 {
    let prefix = active_prefix();
    let build_version = env!("CARGO_PKG_VERSION");
    let manifest = read_manifest(&prefix);
    let bin = installed_bin(&prefix);

    let rows: Vec<StatusRow> = match &bin {
        Ok(b) => vak_ops::services::services_status(
            b,
            &vak_ops::services::SERVICES
                .iter()
                .map(|d| d.name)
                .collect::<Vec<_>>(),
            &vak_ops::services::Paths::default(),
            &vak_ops::services::SystemRunner,
        )
        .into_iter()
        .map(|r| StatusRow {
            name: r.name,
            unit_exists: r.unit_path.exists(),
            unit_path: r.unit_path,
            points_at_installed: r.unit_points_at_installed,
            binary_stale: r.binary_stale,
            pid: r.running_pid,
        })
        .collect(),
        Err(_) => Vec::new(),
    };

    println!("build     {} ({})", build_version, current_git_sha());
    // Machine-readable: scripts that place addons beside the base binary read
    // this instead of guessing at the launcher symlink.
    println!("prefix    {}", prefix.display());
    println!("bin       {}", bin_dir_of(&prefix).display());
    match &manifest {
        Ok(m) => println!("manifest  {} installed {}", m.version, m.installed_at),
        Err(e) => println!("manifest  — ({e})"),
    }
    for r in &rows {
        let state = match r.pid {
            Some(pid) => format!("running (pid {pid})"),
            None => "down".to_string(),
        };
        let flag = if !r.unit_exists {
            "— not installed"
        } else if !r.points_at_installed {
            "✗ unmanaged path"
        } else if r.binary_stale {
            "⚠ process drift — run `self services-sync` to reconcile"
        } else {
            "✓"
        };
        println!(
            "service   {} {state} · {} · {flag}",
            r.name,
            r.unit_path.display()
        );
    }

    let mut drifted = false;
    if let Ok(m) = &manifest
        && m.version != build_version
    {
        eprintln!("drift: manifest {} != build {build_version}", m.version);
        drifted = true;
    }
    if rows.iter().any(|r| r.unit_exists && !r.points_at_installed) {
        eprintln!("drift: a unit still execs outside the managed prefix");
        drifted = true;
    }
    let stale: Vec<&str> = rows
        .iter()
        .filter(|r| r.binary_stale)
        .map(|r| r.name.as_str())
        .collect();
    if !stale.is_empty() {
        eprintln!(
            "drift: {} are not using the installed binary",
            stale.join(", ")
        );
        drifted = true;
    }
    if drifted { 1 } else { 0 }
}

/// One diagnostic line: what was checked, what was found, and — when it is
/// wrong — the single command that fixes it.
struct Check {
    label: &'static str,
    state: CheckState,
    detail: String,
    remedy: Option<&'static str>,
}

enum CheckState {
    Ok,
    Warn,
    Fail,
}

impl Check {
    fn ok(label: &'static str, detail: impl Into<String>) -> Self {
        Check {
            label,
            state: CheckState::Ok,
            detail: detail.into(),
            remedy: None,
        }
    }
    fn warn(label: &'static str, detail: impl Into<String>, remedy: &'static str) -> Self {
        Check {
            label,
            state: CheckState::Warn,
            detail: detail.into(),
            remedy: Some(remedy),
        }
    }
    fn fail(label: &'static str, detail: impl Into<String>, remedy: &'static str) -> Self {
        Check {
            label,
            state: CheckState::Fail,
            detail: detail.into(),
            remedy: Some(remedy),
        }
    }
    fn glyph(&self) -> &'static str {
        match self.state {
            CheckState::Ok => "\u{2713}",
            CheckState::Warn => "\u{26a0}",
            CheckState::Fail => "\u{2717}",
        }
    }
}

/// The configured provider and whether its credential is resolvable, using the
/// same environment-key mapping Runtime admission uses.
fn configured_provider_credential(
    data: &Path,
    secrets: &vak_config::SecretService,
) -> Result<(String, Option<&'static str>), String> {
    let cwd = std::env::current_dir().map_err(|e| e.to_string())?;
    let snapshot = vak_config::ConfigService::new(data, cwd)
        .load()
        .map_err(|e| format!("configuration unreadable: {e}"))?;
    let provider = snapshot
        .config
        .provider
        .name
        .unwrap_or_else(|| "anthropic".to_owned());
    let key_name = match provider.as_str() {
        "anthropic" => "ANTHROPIC_API_KEY",
        "google" => "GOOGLE_API_KEY",
        // Local models authenticate by reachability, not by credential.
        "ollama" => return Ok((provider, Some("no key required"))),
        "opencode-zen" => "OPENCODE_API_KEY",
        _ => "OPENAI_API_KEY",
    };
    let present = secrets
        .get(key_name)
        .ok()
        .flatten()
        .filter(|value| !value.trim().is_empty())
        .is_some()
        || std::env::var(key_name).is_ok_and(|value| !value.trim().is_empty());
    Ok((provider, present.then_some(key_name)))
}

/// Diagnose the installation without assuming any of it works.
///
/// A doctor that needs a healthy gateway to say anything cannot diagnose the
/// only situations worth diagnosing, so every check here degrades to a
/// finding instead of an early exit.
pub(crate) fn run_doctor() -> i32 {
    let mut checks = Vec::new();
    let data = home();
    let prefix = active_prefix();

    checks.push(Check::ok(
        "build",
        format!("{} ({})", env!("CARGO_PKG_VERSION"), current_git_sha()),
    ));
    checks.push(Check::ok("data home", data.display().to_string()));

    let manifest = read_manifest(&prefix);
    match &manifest {
        Ok(m) if m.version == env!("CARGO_PKG_VERSION") => checks.push(Check::ok(
            "install",
            format!(
                "{} at {} (installed {})",
                m.version,
                prefix.display(),
                m.installed_at
            ),
        )),
        Ok(m) => checks.push(Check::warn(
            "install",
            format!(
                "manifest {} but this binary is {}",
                m.version,
                env!("CARGO_PKG_VERSION")
            ),
            "vakcoder self install",
        )),
        Err(e) => checks.push(Check::fail("install", e.clone(), "vakcoder self install")),
    }

    #[cfg(unix)]
    if let Some(user_home) = std::env::var_os("HOME") {
        let launcher = PathBuf::from(user_home).join(".local/bin/vakcoder");
        match std::fs::read_link(&launcher) {
            Ok(target) if target.starts_with(&prefix) => {
                checks.push(Check::ok("launcher", launcher.display().to_string()));
            }
            Ok(target) => checks.push(Check::warn(
                "launcher",
                format!(
                    "{} points outside the managed prefix ({})",
                    launcher.display(),
                    target.display()
                ),
                "vakcoder self install",
            )),
            Err(_) => checks.push(Check::warn(
                "launcher",
                format!("{} is missing or not a managed symlink", launcher.display()),
                "vakcoder self install",
            )),
        }
    }

    let secrets = vak_config::SecretService::new(&data);
    match secrets.get(GATEWAY_TOKEN_KEY) {
        Ok(Some(token)) if !token.trim().is_empty() => {
            checks.push(Check::ok(
                "gateway token",
                format!("present in {}", secrets.path().display()),
            ));
        }
        Ok(_) => checks.push(Check::fail(
            "gateway token",
            format!("absent from {}", secrets.path().display()),
            "vakcoder self install",
        )),
        Err(e) => checks.push(Check::fail(
            "gateway token",
            format!("{} is unreadable: {e}", secrets.path().display()),
            "vakcoder self install",
        )),
    }

    // The gateway now opens without one, so an absent provider key is a
    // degraded state rather than a startup failure — but it is still the
    // reason runs will not start, so it must be visible here.
    match configured_provider_credential(&data, &secrets) {
        Ok((provider, Some(key))) => {
            checks.push(Check::ok("provider", format!("{provider} ({key} present)")));
        }
        Ok((provider, None)) => checks.push(Check::warn(
            "provider",
            format!("{provider} has no API key; runs will not start"),
            "vakcoder admin",
        )),
        Err(e) => checks.push(Check::warn("provider", e, "vakcoder admin")),
    }

    match installed_bin(&prefix) {
        Ok(bin) => {
            let rows = vak_ops::services::services_status(
                &bin,
                &vak_ops::services::SERVICES
                    .iter()
                    .map(|d| d.name)
                    .collect::<Vec<_>>(),
                &vak_ops::services::Paths::default(),
                &vak_ops::services::SystemRunner,
            );
            for row in rows {
                let label: &'static str = if row.name.ends_with("gateway") {
                    "service gateway"
                } else {
                    "service tray"
                };
                if !row.unit_path.exists() {
                    // The tray is an opt-in addon; only the gateway is required.
                    if label == "service gateway" {
                        checks.push(Check::fail(
                            label,
                            "no unit installed",
                            "vakcoder self install",
                        ));
                    } else {
                        checks.push(Check::ok(label, "not installed (optional addon)"));
                    }
                } else if !row.unit_points_at_installed {
                    checks.push(Check::fail(
                        label,
                        format!(
                            "{} execs outside the managed prefix",
                            row.unit_path.display()
                        ),
                        "vakcoder self services-sync",
                    ));
                } else if row.binary_stale {
                    checks.push(Check::warn(
                        label,
                        "running process predates the installed binary",
                        "vakcoder self services-sync",
                    ));
                } else {
                    match row.running_pid {
                        Some(pid) => checks.push(Check::ok(label, format!("running (pid {pid})"))),
                        None => {
                            checks.push(Check::fail(label, "down", "vakcoder self services-sync"))
                        }
                    }
                }
            }
        }
        Err(e) => checks.push(Check::fail("services", e, "vakcoder self install")),
    }

    match vak_client::GatewayConnection::discover_local() {
        Ok(connection) => {
            checks.push(Check::ok(
                "runtime receipt",
                format!(
                    "{} (pid {})",
                    connection.base_url(),
                    connection.process_id()
                ),
            ));
            if vak_ops::health_ok(&vak_ops::OpsConfig::default()) {
                checks.push(Check::ok(
                    "handshake",
                    "authenticated /version and /health agree",
                ));
            } else {
                checks.push(Check::fail(
                    "handshake",
                    "the gateway is listening but failed the authenticated handshake",
                    "vakcoder self services-sync",
                ));
            }
        }
        Err(e) => {
            checks.push(Check::fail(
                "runtime receipt",
                e.to_string(),
                "vakcoder self services-sync",
            ));
            checks.push(Check::warn(
                "handshake",
                "skipped: no reachable gateway",
                "vakcoder self services-sync",
            ));
        }
    }

    let log = vak_config::paths::logs_dir().join("gateway.log");
    checks.push(Check::ok("gateway log", log.display().to_string()));

    let width = checks.iter().map(|c| c.label.len()).max().unwrap_or(0);
    let mut failed = 0usize;
    let mut warned = 0usize;
    for check in &checks {
        println!("{} {:width$}  {}", check.glyph(), check.label, check.detail);
        match check.state {
            CheckState::Fail => failed += 1,
            CheckState::Warn => warned += 1,
            CheckState::Ok => {}
        }
    }
    let mut remedies: Vec<&'static str> = checks
        .iter()
        .filter(|c| !matches!(c.state, CheckState::Ok))
        .filter_map(|c| c.remedy)
        .collect();
    remedies.dedup();
    if !remedies.is_empty() {
        println!();
        println!("next:");
        for remedy in remedies {
            println!("  {remedy}");
        }
    }
    if failed > 0 {
        eprintln!("\n{failed} failing, {warned} degraded");
        1
    } else if warned > 0 {
        println!("\nhealthy with {warned} degraded");
        0
    } else {
        println!("\nhealthy");
        0
    }
}

pub(crate) fn run_uninstall(yes: bool, purge: bool, no_service: bool) -> i32 {
    if !yes && std::io::stdin().is_terminal() {
        if no_service {
            print!("remove managed program files? [y/N] ");
        } else {
            print!("stop services and remove managed install? [y/N] ");
        }
        let _ = std::io::stdout().flush();
        let mut line = String::new();
        let _ = std::io::stdin().read_line(&mut line);
        if !line.trim().eq_ignore_ascii_case("y") {
            println!("aborted");
            return 0;
        }
    }
    if !no_service {
        let names: Vec<&str> = vak_ops::services::SERVICES.iter().map(|d| d.name).collect();
        if let Err(e) = vak_ops::services::services_uninstall(
            &names,
            &vak_ops::services::Paths::default(),
            &vak_ops::services::SystemRunner,
        ) {
            eprintln!("warning: service teardown incomplete: {e}");
        }
    }
    let prefix = active_prefix();
    if let Err(e) = std::fs::remove_dir_all(&prefix) {
        eprintln!("warning: remove {}: {e}", prefix.display());
    } else {
        println!("removed {}", prefix.display());
    }
    #[cfg(unix)]
    if let Some(user_home) = std::env::var_os("HOME") {
        let launcher = PathBuf::from(user_home).join(".local/bin/vakcoder");
        if std::fs::read_link(&launcher).is_ok_and(|target| target.starts_with(&prefix)) {
            let _ = std::fs::remove_file(&launcher);
            println!("removed launcher {}", launcher.display());
        }
    }
    let _ = std::fs::remove_file(pointer_path());
    report_unmanaged_leftovers();
    if purge {
        if !yes && std::io::stdin().is_terminal() {
            print!(
                "ALSO delete data home, cache, and logs? (sessions, memory, tasks, store) [y/N] "
            );
            let _ = std::io::stdout().flush();
            let mut line = String::new();
            let _ = std::io::stdin().read_line(&mut line);
            if !line.trim().eq_ignore_ascii_case("y") {
                println!("kept data at {}", home().display());
                return 0;
            }
        }
        // Data home: sessions, memory, tasks, .env
        let data = home();
        if let Err(e) = std::fs::remove_dir_all(&data) {
            eprintln!("warning: purge data {}: {e}", data.display());
        } else {
            println!("purged data {}", data.display());
        }
        // Cache home: store.db, store.db-wal, store.db-shm
        let cache = vak_config::paths::cache_home();
        if cache != data {
            if let Err(e) = std::fs::remove_dir_all(&cache) {
                eprintln!("warning: purge cache {}: {e}", cache.display());
            } else {
                println!("purged cache {}", cache.display());
            }
        }
        // Logs home for the Runtime and optional tray.
        let logs = vak_config::paths::logs_dir();
        if logs != data {
            if let Err(e) = std::fs::remove_dir_all(&logs) {
                eprintln!("warning: purge logs {}: {e}", logs.display());
            } else {
                println!("purged logs {}", logs.display());
            }
        }
    }
    0
}

/// Addons installed outside the managed prefix cannot be removed on the
/// user's behalf — they are plain copies with no provenance record, and
/// deleting an arbitrary file at a conventional path is not ours to do. Name
/// them instead, so an uninstall never leaves silent orphans behind.
fn report_unmanaged_leftovers() {
    let mut leftovers: Vec<PathBuf> = Vec::new();
    #[cfg(unix)]
    if let Some(user_home) = std::env::var_os("HOME") {
        let user_home = PathBuf::from(user_home);
        let tui = std::env::var_os("VAKCODER_TUI_INSTALL_DIR")
            .map_or_else(|| user_home.join(".local/bin"), PathBuf::from)
            .join("vakcoder-tui");
        if tui.exists() {
            leftovers.push(tui);
        }
        for app in [
            user_home.join("Applications/VakCoder.app"),
            PathBuf::from("/Applications/VakCoder.app"),
        ] {
            if app.exists() {
                leftovers.push(app);
            }
        }
    }
    if leftovers.is_empty() {
        return;
    }
    println!("\nseparately installed addons were left in place:");
    for path in leftovers {
        println!("  {}", path.display());
    }
    println!("remove them yourself if you no longer want them.");
}

pub(crate) fn run_update(url: &str, yes: bool) -> i32 {
    let prefix = active_prefix();
    let current_bin = match installed_bin(&prefix) {
        Ok(p) => p,
        Err(e) => {
            eprintln!("error: {e}");
            return 1;
        }
    };
    let client = match reqwest::blocking::Client::builder()
        .timeout(std::time::Duration::from_secs(30))
        .build()
    {
        Ok(c) => c,
        Err(e) => {
            eprintln!("error: http client: {e}");
            return 1;
        }
    };
    let manifest: ReleaseManifest = match client.get(url).send().and_then(|r| r.error_for_status())
    {
        Ok(r) => match r.json() {
            Ok(m) => m,
            Err(e) => {
                eprintln!("error: release manifest unreadable: {e}");
                return 1;
            }
        },
        Err(e) => {
            eprintln!("error: release feed unreachable: {e}");
            return 1;
        }
    };
    let newer = match version_is_newer(&manifest.version, env!("CARGO_PKG_VERSION")) {
        Ok(newer) => newer,
        Err(e) => {
            eprintln!("error: release version is invalid: {e}");
            return 1;
        }
    };
    if !newer {
        println!(
            "up to date ({} >= {})",
            env!("CARGO_PKG_VERSION"),
            manifest.version
        );
        return 0;
    }
    let key = format!("{}/{}", std::env::consts::OS, std::env::consts::ARCH);
    let Some(artifact) = manifest.artifacts.get(&key) else {
        eprintln!("error: no artifact for {key} in release manifest");
        return 1;
    };
    if !yes && std::io::stdin().is_terminal() {
        print!(
            "update {} → {}? [y/N] ",
            env!("CARGO_PKG_VERSION"),
            manifest.version
        );
        let _ = std::io::stdout().flush();
        let mut line = String::new();
        let _ = std::io::stdin().read_line(&mut line);
        if !line.trim().eq_ignore_ascii_case("y") {
            println!("aborted");
            return 0;
        }
    }
    let body = match client
        .get(&artifact.url)
        .send()
        .and_then(|r| r.error_for_status())
    {
        Ok(r) => r,
        Err(e) => {
            eprintln!("error: download failed: {e}");
            return 1;
        }
    };
    let bytes = match body.bytes() {
        Ok(b) => b,
        Err(e) => {
            eprintln!("error: download read failed: {e}");
            return 1;
        }
    };
    use sha2::Digest as _;
    let actual_sha256 = format!("{:x}", sha2::Sha256::digest(&bytes));
    if !actual_sha256.eq_ignore_ascii_case(&artifact.sha256) {
        eprintln!(
            "error: artifact checksum mismatch (expected {}, got {actual_sha256})",
            artifact.sha256
        );
        return 1;
    }
    let staged_root = version_dir(&prefix, &manifest.version);
    let staged_bin_dir = staged_root.join(BIN_DIR);
    let staged_binary = staged_bin_dir.join("vakcoder");
    if let Err(e) = write_atomic(&staged_binary, &bytes) {
        eprintln!("error: {e}");
        return 1;
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = std::fs::set_permissions(&staged_binary, std::fs::Permissions::from_mode(0o755));
    }
    if let Some(current_dir) = current_bin.parent() {
        for name in ["vak-delivery-worker", "vakcoder-tray"] {
            let source = current_dir.join(name);
            if source.exists()
                && let Err(e) = copy_executable(&source, &staged_bin_dir.join(name))
            {
                eprintln!("warning: preserve {name}: {e}");
            }
        }
    }
    if let Err(e) = replace_symlink(&prefix.join("current"), &staged_root) {
        eprintln!("error: activation failed: {e}");
        return 1;
    }
    // The manifest version moves with the artifact so status stays truthful.
    if let Ok(mut m) = read_manifest(&prefix) {
        m.version = manifest.version.clone();
        if let Ok(json) = serde_json::to_vec_pretty(&m)
            && let Err(e) = write_atomic(&manifest_path(&prefix), &json)
        {
            eprintln!("warning: manifest refresh failed: {e}");
        }
    }
    println!("updated → {}", manifest.version);
    println!("reloading services…");
    let code = run_services_sync(Vec::new());
    if code == 0 {
        println!("done");
    }
    code
}

#[derive(serde::Deserialize, Debug)]
struct ReleaseManifest {
    version: String,
    #[serde(default)]
    artifacts: std::collections::HashMap<String, ReleaseArtifact>,
}

#[derive(serde::Deserialize, Debug)]
struct ReleaseArtifact {
    url: String,
    sha256: String,
}

/// Semver precedence, enough of it for a release feed.
///
/// Build metadata is ignored (semver §10) rather than rejected: the release
/// gate accepts `X.Y.Z+meta`, so an updater that cannot parse it would refuse
/// every artifact the project is allowed to ship. Prereleases rank below their
/// release and against each other by identifier, so `1.0.0-rc.2` supersedes
/// `1.0.0-rc.1` instead of looking identical to it.
fn version_is_newer(candidate: &str, current: &str) -> Result<bool, String> {
    fn parse(value: &str) -> Result<(Vec<u64>, Option<&str>), String> {
        let value = value.split_once('+').map_or(value, |(head, _)| head);
        let (numbers, prerelease) = value
            .split_once('-')
            .map_or((value, None), |(numbers, suffix)| (numbers, Some(suffix)));
        let numbers = numbers
            .split('.')
            .map(|part| {
                part.parse::<u64>()
                    .map_err(|_| format!("{value:?} is not numeric semver"))
            })
            .collect::<Result<Vec<_>, _>>()?;
        if numbers.len() != 3 {
            return Err(format!("{value:?} must contain major.minor.patch"));
        }
        Ok((numbers, prerelease.filter(|suffix| !suffix.is_empty())))
    }

    /// Numeric identifiers compare numerically and rank below alphanumeric
    /// ones; a longer identifier list wins when all shared fields tie.
    fn prerelease_is_greater(candidate: &str, current: &str) -> bool {
        let mut left = candidate.split('.');
        let mut right = current.split('.');
        loop {
            match (left.next(), right.next()) {
                (None, None) => return false,
                (None, Some(_)) => return false,
                (Some(_), None) => return true,
                (Some(a), Some(b)) => {
                    if a == b {
                        continue;
                    }
                    return match (a.parse::<u64>(), b.parse::<u64>()) {
                        (Ok(a), Ok(b)) => a > b,
                        (Ok(_), Err(_)) => false,
                        (Err(_), Ok(_)) => true,
                        (Err(_), Err(_)) => a > b,
                    };
                }
            }
        }
    }

    let (candidate_numbers, candidate_pre) = parse(candidate)?;
    let (current_numbers, current_pre) = parse(current)?;
    if candidate_numbers != current_numbers {
        return Ok(candidate_numbers > current_numbers);
    }
    Ok(match (candidate_pre, current_pre) {
        (None, None) => false,
        (None, Some(_)) => true,
        (Some(_), None) => false,
        (Some(candidate), Some(current)) => prerelease_is_greater(candidate, current),
    })
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::*;

    fn temp_prefix(tag: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "vak-self-{tag}-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.subsec_nanos())
                .unwrap_or(0)
        ));
        std::fs::create_dir_all(dir.join(BIN_DIR)).unwrap();
        dir
    }

    fn fake_binary(dir: &Path, name: &str) -> PathBuf {
        let p = dir.join(name);
        std::fs::write(&p, "#!/bin/sh\necho ok\n").unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&p, std::fs::Permissions::from_mode(0o755)).unwrap();
        }
        p
    }

    #[test]
    fn atomic_write_replaces_content_and_cleans_tmp() {
        let dir = temp_prefix("atomic");
        let f = dir.join("f.json");
        write_atomic(&f, b"one").unwrap();
        write_atomic(&f, b"two").unwrap();
        assert_eq!(std::fs::read_to_string(&f).unwrap(), "two");
        assert!(!dir.join("f.tmp").exists());
    }

    #[test]
    fn copy_executable_preserves_mode_bits() {
        let dir = temp_prefix("copy");
        let src = fake_binary(&dir, "src-bin");
        let dst = dir.join(BIN_DIR).join("dst-bin");
        copy_executable(&src, &dst).unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = std::fs::metadata(&dst).unwrap().permissions().mode();
            assert_eq!(mode & 0o111, 0o111, "exec bits must survive the copy");
        }
    }

    #[test]
    fn minted_gateway_tokens_are_full_entropy_and_distinct() {
        let first = mint_gateway_token().unwrap();
        let second = mint_gateway_token().unwrap();
        assert_eq!(first.len(), 64, "256 bits, hex-encoded");
        assert!(first.bytes().all(|b| b.is_ascii_hexdigit()));
        assert_ne!(first, second);
    }

    #[test]
    fn gateway_token_provisioning_never_overwrites_an_existing_secret() {
        let dir = tempfile::tempdir().unwrap();
        let secrets = vak_config::SecretService::new(dir.path());

        assert!(
            ensure_gateway_token_in(&secrets).unwrap(),
            "first install mints"
        );
        let minted = secrets.get(GATEWAY_TOKEN_KEY).unwrap().unwrap();
        assert_eq!(minted.len(), 64);

        // Reinstall must not rotate a credential that live surfaces are
        // already holding: it would revoke every one of them mid-session.
        assert!(
            !ensure_gateway_token_in(&secrets).unwrap(),
            "reinstall reuses"
        );
        assert_eq!(secrets.get(GATEWAY_TOKEN_KEY).unwrap().unwrap(), minted);
    }

    #[test]
    fn update_manifest_deserializes_artifact_map() {
        let raw = br#"{"version":"9.9.9","artifacts":{"macos/aarch64":{"url":"https://x/vak","sha256":"abc"}}} "#;
        let m: ReleaseManifest = serde_json::from_slice(raw).unwrap();
        assert_eq!(m.version, "9.9.9");
        assert_eq!(m.artifacts["macos/aarch64"].url, "https://x/vak");
    }

    #[test]
    fn update_versions_use_numeric_semver_order() {
        assert!(version_is_newer("0.10.0", "0.9.0").unwrap());
        assert!(!version_is_newer("0.8.0", "0.8.0").unwrap());
        assert!(version_is_newer("1.0.0", "1.0.0-rc.1").unwrap());
        assert!(!version_is_newer("1.0.0-rc.1", "1.0.0").unwrap());
        assert!(version_is_newer("not-a-version", "0.8.0").is_err());
    }

    #[test]
    fn update_orders_prereleases_against_each_other() {
        assert!(version_is_newer("1.0.0-rc.2", "1.0.0-rc.1").unwrap());
        assert!(!version_is_newer("1.0.0-rc.1", "1.0.0-rc.2").unwrap());
        // Numeric identifiers rank below alphanumeric ones (semver §11).
        assert!(version_is_newer("1.0.0-rc", "1.0.0-1").unwrap());
        // A longer identifier list wins when every shared field ties.
        assert!(version_is_newer("1.0.0-rc.1.1", "1.0.0-rc.1").unwrap());
    }

    #[test]
    fn update_ignores_build_metadata_the_release_gate_permits() {
        // check-release-version.sh accepts X.Y.Z+meta; refusing to parse it
        // would strand every such build with an unusable updater.
        assert!(version_is_newer("0.9.4+ci.7", "0.9.3").unwrap());
        assert!(!version_is_newer("0.9.3+ci.8", "0.9.3+ci.7").unwrap());
    }
}
