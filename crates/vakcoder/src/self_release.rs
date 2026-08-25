//! Managed release lifecycle (docs/design/32-release-engineering.md).

use std::io::{IsTerminal as _, Write as _};
use std::path::{Path, PathBuf};

use vak_config::get_var;

const BIN_DIR: &str = "bin";

fn home() -> PathBuf {
    get_var("VAKCODER_HOME")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".vakcoder")))
        .unwrap_or_else(|| PathBuf::from(".vakcoder"))
}

fn prefix_default() -> PathBuf {
    home().join("local/release")
}

fn manifest_path(prefix: &Path) -> PathBuf {
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

fn current_git_sha() -> String {
    option_env!("VAKCODER_GIT_SHA")
        .map(str::to_string)
        .unwrap_or_else(|| "unknown".into())
}

pub(crate) fn run_install(prefix: Option<PathBuf>) -> i32 {
    let prefix = prefix.unwrap_or_else(prefix_default);
    let exe = match std::env::current_exe() {
        Ok(e) => e,
        Err(e) => {
            eprintln!("error: cannot locate running binary: {e}");
            return 1;
        }
    };
    let bin_dir = prefix.join(BIN_DIR);
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
    println!("next: vakcoder self services-sync");
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
        let flag = if r.points_at_installed {
            "✓"
        } else {
            "✗ legacy path"
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
            print!("ALSO delete ~/.vakcoder (sessions, memory, tasks)? [y/N] ");
            let _ = std::io::stdout().flush();
            let mut line = String::new();
            let _ = std::io::stdin().read_line(&mut line);
            if !line.trim().eq_ignore_ascii_case("y") {
                println!("kept ~/.vakcoder");
                return 0;
            }
        }
        let data = home();
        if let Err(e) = std::fs::remove_dir_all(&data) {
            eprintln!("warning: purge {}: {e}", data.display());
        } else {
            println!("purged {}", data.display());
        }
    }
    0
}

pub(crate) fn run_update(url: &str, yes: bool) -> i32 {
    let prefix = prefix_default();
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
