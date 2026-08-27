//! Managed release lifecycle (docs/design/32-release-engineering.md).

use std::io::{IsTerminal as _, Write as _};
use std::path::{Path, PathBuf};

const BIN_DIR: &str = "bin";
const INSTALL_POINTER: &str = "install-root.json";

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

    // A base install owns only the base service. Optional channel bridges and
    // presentation addons must be enabled explicitly after their credentials
    // or binaries exist.
    if !no_service {
        let svc_names = ["com.vakcoder.gateway"];
        let outcomes = vak_ops::services::services_sync(
            &stable_bin.join("vakcoder"),
            &svc_names,
            &vak_ops::services::Paths::default(),
            &vak_ops::services::SystemRunner,
        );
        for o in &outcomes {
            match &o.action {
                vak_ops::services::SyncAction::Failed(e) => {
                    eprintln!("  warning: {}: {e}", o.name);
                }
                action => println!("  service: {} — {action:?}", o.name),
            }
        }
    }

    println!("open the admin console with `vakcoder admin`.");
    0
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

fn version_is_newer(candidate: &str, current: &str) -> Result<bool, String> {
    fn parse(value: &str) -> Result<(Vec<u64>, Option<&str>), String> {
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
        Ok((numbers, prerelease))
    }

    let candidate = parse(candidate)?;
    let current = parse(current)?;
    Ok(candidate.0 > current.0
        || (candidate.0 == current.0 && candidate.1.is_none() && current.1.is_some()))
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
        assert!(version_is_newer("not-a-version", "0.8.0").is_err());
    }
}
