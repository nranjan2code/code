//! Managed release lifecycle (docs/design/32-release-engineering.md).

use std::io::{IsTerminal as _, Write as _};
use std::path::{Path, PathBuf};

const BIN_DIR: &str = "bin";
const MACOS_BUNDLE_NAME: &str = "vakcoder.app";

fn home() -> PathBuf {
    // Canonical data home (doc 32) — used only for the Linux default
    // prefix and legacy-path warnings, never as the data home itself.
    vak_config::paths::data_home()
}

/// Default install root. macOS installs a proper application bundle in
/// `/Applications` (falling back to the per-user `~/Applications` when
/// the system folder is not writable); Linux keeps a managed prefix
/// under the XDG data home. An explicit `--prefix` always wins so tests
/// and portable installs stay deterministic.
fn prefix_default() -> PathBuf {
    #[cfg(target_os = "macos")]
    {
        let system = PathBuf::from("/Applications").join(MACOS_BUNDLE_NAME);
        if dir_writable(Path::new("/Applications")) || system.exists() {
            return system;
        }
        if let Some(h) = std::env::var_os("HOME") {
            return PathBuf::from(h)
                .join("Applications")
                .join(MACOS_BUNDLE_NAME);
        }
        system
    }
    #[cfg(not(target_os = "macos"))]
    {
        home().join("local").join("release")
    }
}

#[cfg(target_os = "macos")]
fn dir_writable(dir: &Path) -> bool {
    let probe = dir.join(format!(".vak-write-probe-{}", std::process::id()));
    match std::fs::write(&probe, b"") {
        Ok(_) => {
            let _ = std::fs::remove_file(&probe);
            true
        }
        Err(_) => false,
    }
}

/// True when the install root is a macOS application bundle; binaries
/// then live under Contents/MacOS and the manifest under Contents/
/// Resources instead of `<root>/bin`.
#[cfg(target_os = "macos")]
fn is_bundle(root: &Path) -> bool {
    root.extension().and_then(|e| e.to_str()) == Some("app")
}

#[cfg(not(target_os = "macos"))]
fn is_bundle(_root: &Path) -> bool {
    false
}

fn bin_dir_of(prefix: &Path) -> PathBuf {
    #[cfg(target_os = "macos")]
    {
        if is_bundle(prefix) {
            return prefix.join("Contents").join("MacOS");
        }
    }
    prefix.join(BIN_DIR)
}

fn manifest_path(prefix: &Path) -> PathBuf {
    #[cfg(target_os = "macos")]
    {
        if is_bundle(prefix) {
            return prefix
                .join("Contents")
                .join("Resources")
                .join("install.json");
        }
    }
    prefix.join("install.json")
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

fn copy_dir_recursive(src: &Path, dst: &Path) -> Result<(), String> {
    std::fs::create_dir_all(dst).map_err(|e| format!("mkdir {}: {e}", dst.display()))?;
    for entry in std::fs::read_dir(src).map_err(|e| format!("read dir {}: {e}", src.display()))? {
        let entry = entry.map_err(|e| format!("dir entry: {e}"))?;
        let ty = entry
            .file_type()
            .map_err(|e| format!("file type {}: {e}", entry.path().display()))?;
        let target = dst.join(entry.file_name());
        if ty.is_dir() {
            copy_dir_recursive(&entry.path(), &target)?;
        } else {
            let bytes = std::fs::read(entry.path())
                .map_err(|e| format!("read {}: {e}", entry.path().display()))?;
            write_atomic(&target, &bytes)?;
        }
    }
    Ok(())
}

fn current_git_sha() -> String {
    option_env!("VAKCODER_GIT_SHA")
        .map(str::to_string)
        .unwrap_or_else(|| "unknown".into())
}

pub(crate) fn run_install(prefix: Option<PathBuf>) -> i32 {
    let prefix = prefix.unwrap_or_else(prefix_default);
    let bundle = is_bundle(&prefix);
    let exe = match std::env::current_exe() {
        Ok(e) => e,
        Err(e) => {
            eprintln!("error: cannot locate running binary: {e}");
            return 1;
        }
    };
    let bin_dir = bin_dir_of(&prefix);
    let mut binaries = Vec::new();
    if let Err(e) = copy_executable(&exe, &bin_dir.join("vakcoder")) {
        eprintln!("error: {e}");
        return 1;
    }
    binaries.push(("vakcoder".into(), bin_dir.join("vakcoder")));
    // Tray ships beside the main binary in release trees; optional.
    if let Some(sibling) = exe.parent().map(|p| p.join("vakcoder-tray"))
        && sibling.exists()
    {
        if let Err(e) = copy_executable(&sibling, &bin_dir.join("vakcoder-tray")) {
            eprintln!("warning: tray not installed: {e}");
        } else {
            binaries.push(("vakcoder-tray".into(), bin_dir.join("vakcoder-tray")));
        }
    }
    // Delivery outbox worker (doc 30) rides along when built.
    if let Some(sibling) = exe.parent().map(|p| p.join("vak-delivery-worker"))
        && sibling.exists()
        && let Err(e) = copy_executable(&sibling, &bin_dir.join("vak-delivery-worker"))
    {
        eprintln!("warning: delivery worker not installed: {e}");
    }
    // Desktop app (Tauri) rides along when built.
    if let Some(sibling) = exe.parent().map(|p| p.join("vak-desktop"))
        && sibling.exists()
    {
        if let Err(e) = copy_executable(&sibling, &bin_dir.join("vak-desktop")) {
            eprintln!("warning: desktop not installed: {e}");
        } else {
            binaries.push(("vak-desktop".into(), bin_dir.join("vak-desktop")));
        }
    }

    if bundle && let Err(e) = write_bundle_metadata(&prefix) {
        eprintln!("error: {e}");
        return 1;
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
    println!(
        "installed {} ({}) → {}",
        manifest.version,
        manifest.git_sha,
        prefix.display()
    );
    for (name, path) in &manifest.binaries {
        println!("  {name}: {}", path.display());
    }

    // Auto-sync background services (gateway + telegram).
    // Tray is opt-in: `vakcoder self services-sync com.vakcoder.tray`.
    let svc_names = ["com.vakcoder.gateway", "com.vakcoder.telegram"];
    let outcomes = vak_ops::services::services_sync(
        &bin_dir.join("vakcoder"),
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

    #[cfg(target_os = "macos")]
    {
        println!();
        println!("open VakCoder from Applications to launch the desktop app.");
    }
    0
}

/// Info.plist for the menu-bar app bundle. LSUIElement keeps the tray
/// out of the Dock (menu-bar-only identity, per macOS HIG for agents).
#[cfg(target_os = "macos")]
fn info_plist() -> String {
    let version = env!("CARGO_PKG_VERSION");
    format!(
        r#"<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
	<key>CFBundleName</key>
	<string>VakCoder</string>
	<key>CFBundleDisplayName</key>
	<string>VakCoder</string>
	<key>CFBundleIdentifier</key>
	<string>dev.vakcoder.desktop</string>
	<key>CFBundleExecutable</key>
	<string>vak-desktop</string>
	<key>CFBundlePackageType</key>
	<string>APPL</string>
	<key>CFBundleShortVersionString</key>
	<string>{version}</string>
	<key>CFBundleVersion</key>
	<string>{version}</string>
	<key>LSMinimumSystemVersion</key>
	<string>11.0</string>
	<key>NSHumanReadableCopyright</key>
	<string>MIT licensed — see https://github.com/vakcoder/vakcoder</string>
</dict>
</plist>
"#
    )
}

#[cfg(target_os = "macos")]
fn write_bundle_metadata(prefix: &Path) -> Result<(), String> {
    let contents = prefix.join("Contents");
    std::fs::create_dir_all(contents.join("Resources")).map_err(|e| e.to_string())?;
    // Plist first: a bundle without it is not launchable as an app.
    write_atomic(&contents.join("Info.plist"), info_plist().as_bytes())?;
    // Copy desktop frontend assets into Resources so vak-desktop can find them.
    // Walk up from the binary to the workspace root, then into the source tree.
    let resources = contents.join("Resources");
    if let Ok(exe) = std::env::current_exe() {
        // Binary is at <workspace>/target/release/vakcoder or
        // <bundle>/Contents/MacOS/vakcoder — walk up to find ui/dist.
        let candidates = [
            exe.parent()
                .and_then(|p| p.parent())
                .and_then(|p| p.parent())
                .map(|p| p.join("crates/vak-desktop/ui/dist")),
            exe.parent()
                .and_then(|p| p.parent())
                .and_then(|p| p.parent())
                .and_then(|p| p.parent())
                .map(|p| p.join("crates/vak-desktop/ui/dist")),
        ];
        for candidate in candidates.into_iter().flatten() {
            if candidate.exists() {
                copy_dir_recursive(&candidate, &resources)?;
                break;
            }
        }
    }
    Ok(())
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
    let prefix = prefix_default();
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
    points_at_installed: bool,
    binary_stale: bool,
    pid: Option<u32>,
}

pub(crate) fn run_status() -> i32 {
    let prefix = prefix_default();
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
        Err(e) => {
            println!("manifest  — ({e})");
            // Adoption hint: a pre-bundle install at the old dotdir
            // prefix is the one migration path into the canonical layout.
            let legacy = home().join("local").join("release");
            #[cfg(target_os = "macos")]
            if legacy.join("install.json").exists() {
                eprintln!(
                    "note: legacy install found at {} — run `vakcoder self install` to adopt {}",
                    legacy.display(),
                    prefix.display()
                );
            }
            #[cfg(not(target_os = "macos"))]
            {
                let _ = legacy;
            }
        }
    }
    for r in &rows {
        let state = match r.pid {
            Some(pid) => format!("running (pid {pid})"),
            None => "down".to_string(),
        };
        let flag = if !r.points_at_installed {
            "✗ legacy path"
        } else if r.binary_stale {
            "⚠ stale process — run `self services-sync` to bounce"
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
    if !rows.is_empty() && rows.iter().any(|r| !r.points_at_installed) {
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
            "drift: {} predate the installed binary (still executing the old image)",
            stale.join(", ")
        );
        drifted = true;
    }
    if drifted { 1 } else { 0 }
}

pub(crate) fn run_uninstall(yes: bool, purge: bool) -> i32 {
    if !yes && std::io::stdin().is_terminal() {
        print!("stop services and remove managed install? [y/N] ");
        let _ = std::io::stdout().flush();
        let mut line = String::new();
        let _ = std::io::stdin().read_line(&mut line);
        if !line.trim().eq_ignore_ascii_case("y") {
            println!("aborted");
            return 0;
        }
    }
    let names: Vec<&str> = vak_ops::services::SERVICES.iter().map(|d| d.name).collect();
    if let Err(e) = vak_ops::services::services_uninstall(
        &names,
        &vak_ops::services::Paths::default(),
        &vak_ops::services::SystemRunner,
    ) {
        eprintln!("warning: service teardown incomplete: {e}");
    }
    let prefix = prefix_default();
    if let Err(e) = std::fs::remove_dir_all(&prefix) {
        eprintln!("warning: remove {}: {e}", prefix.display());
    } else {
        println!("removed {}", prefix.display());
    }
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
        // Logs home: gateway.log, telegram.log, tray.log
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
    let prefix = prefix_default();
    let bin_dir = bin_dir_of(&prefix);
    let target = match installed_bin(&prefix) {
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
    let manifest: ReleaseManifest = match client.get(url).send() {
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
    if manifest.version.as_str() <= env!("CARGO_PKG_VERSION") {
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
        .get(artifact)
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
    // Atomic replace of the INSTALLED binary; the running process keeps its
    // inode until exit, so this is safe even mid-service.
    if let Err(e) = write_atomic(&target, &bytes) {
        eprintln!("error: {e}");
        return 1;
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = std::fs::set_permissions(&target, std::fs::Permissions::from_mode(0o755));
    }
    // Companion binaries live beside the main binary in release trees.
    // Replace each one that shipped with this build.
    let companions = ["vakcoder-tray", "vak-desktop", "vak-delivery-worker"];
    for name in companions {
        let in_bundle = bin_dir.join(name);
        if !in_bundle.exists() {
            continue;
        }
        if let Some(src) = target
            .parent()
            .and_then(|p| p.parent())
            .map(|p| p.join(name))
        {
            // When running from the build tree, the sibling is beside the
            // build output; when running from the bundle, it's in the same
            // directory.  Try the build-tree path first.
            let src = if src.exists() {
                src
            } else {
                bin_dir.join(name)
            };
            if let Err(e) = copy_executable(&src, &in_bundle) {
                eprintln!("warning: update {name}: {e}");
            } else {
                println!("  updated {name}");
            }
        }
    }
    // Re-write bundle plist so version stays current.
    #[cfg(target_os = "macos")]
    if is_bundle(&prefix)
        && let Err(e) = write_bundle_metadata(&prefix)
    {
        eprintln!("warning: bundle refresh: {e}");
    }
    // Manifest version moves with the artifact so status stays truthful.
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
    artifacts: std::collections::HashMap<String, String>,
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
        let raw = br#"{"version":"9.9.9","artifacts":{"macos/aarch64":"https://x/vak"}} "#;
        let m: ReleaseManifest = serde_json::from_slice(raw).unwrap();
        assert_eq!(m.version, "9.9.9");
        assert_eq!(m.artifacts["macos/aarch64"], "https://x/vak");
    }
}
