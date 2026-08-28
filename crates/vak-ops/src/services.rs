//! Generated service units (docs/design/32-release-engineering.md §3).
//!
//! Units are rendered from [`SERVICES`] and diffed onto disk by
//! [`services_sync`]; nothing here is hand-edited. Templates embed zero
//! credentials: the binary self-sources `data_home()/.env`, so regenerating
//! units can never strand auth. Logs stay at the platform `logs_dir()` and
//! user data remains under the canonical platform data home.
//! The unit working directory is captured when `self services-sync` runs, so
//! the gateway loads the selected workspace's config and project `.env`.
//!
//! All manager interaction goes through [`CommandRunner`], so tests inject a
//! recorder instead of shelling out to launchctl/systemctl.

use std::path::{Path, PathBuf};
use std::process::Command;

/// Where units are installed for the current user.
#[derive(Debug, Clone)]
pub struct Paths {
    pub launch_agents_dir: PathBuf,
    pub systemd_unit_dir: PathBuf,
}

impl Default for Paths {
    fn default() -> Self {
        let home = super::home();
        Paths {
            launch_agents_dir: home.join("Library/LaunchAgents"),
            systemd_unit_dir: home.join(".config/systemd/user"),
        }
    }
}

/// One resolvable service definition: binary placement plus arguments,
/// mirroring the hand-made plists minus their embedded secrets.
#[derive(Debug, Clone)]
pub struct ServiceSpec {
    /// launchd label, also the plist stem (`com.vakcoder.gateway`).
    pub name: &'static str,
    pub bin_path: PathBuf,
    pub args: Vec<String>,
    /// Stable log destination under the canonical platform logs directory.
    pub log_path: PathBuf,
    /// Workspace the service must load for config and project-local `.env`.
    pub working_dir: PathBuf,
    /// User home required by platform path resolution in the sanitized
    /// service-manager environment. This is operational state, not a secret.
    pub home_dir: PathBuf,
}

/// Static template table behind [`ServiceSpec`].
#[derive(Debug, Clone, Copy)]
pub struct ServiceDef {
    pub name: &'static str,
    /// Binary file name inside the install prefix's `bin` directory.
    pub bin_file: &'static str,
    pub args: &'static [&'static str],
    /// Log file name under `<vak-home>/logs`.
    pub log_file: &'static str,
}

pub const SERVICES: &[ServiceDef] = &[
    ServiceDef {
        name: "com.vakcoder.gateway",
        bin_file: "vakcoder",
        args: &["serve", "--gateway", "--trust"],
        log_file: "gateway.log",
    },
    ServiceDef {
        name: "com.vakcoder.telegram",
        bin_file: "vakcoder",
        args: &["telegram", "--server", "http://127.0.0.1:8901"],
        log_file: "telegram.log",
    },
];

/// Obsolete GUI launch agents that must be removed during an upgrade. The
/// desktop app now owns its tray, so a separate launchd tray must never keep
/// the bundle's LaunchServices identity alive without a window to reveal.
pub const RETIRED_SERVICES: &[&str] = &["com.vakcoder.tray"];

impl ServiceDef {
    /// Resolve against an install prefix: `bin_dir` holds the release
    /// binaries; logs land in the canonical platform logs dir
    /// (`~/Library/Logs/vakcoder` / XDG state) — never inside data.
    pub fn spec(&self, bin_dir: &Path, home_dir: &Path, working_dir: &Path) -> ServiceSpec {
        ServiceSpec {
            name: self.name,
            bin_path: bin_dir.join(self.bin_file),
            args: self.args.iter().map(|a| (*a).to_string()).collect(),
            log_path: vak_config::paths::logs_dir().join(self.log_file),
            working_dir: working_dir.to_path_buf(),
            home_dir: home_dir.to_path_buf(),
        }
    }

    /// systemd unit stem derived from the launchd label
    /// (`com.vakcoder.gateway` → `vakcoder-gateway.service`).
    pub fn systemd_unit(&self) -> String {
        format!(
            "vakcoder-{}.service",
            self.name.strip_prefix("com.vakcoder.").unwrap_or(self.name)
        )
    }
}

fn short_name(name: &str) -> &str {
    name.strip_prefix("com.vakcoder.").unwrap_or(name)
}

/// Unit file location for `name` under `paths` (pure; no filesystem IO).
pub fn unit_file_path(name: &str, paths: &Paths) -> PathBuf {
    #[cfg(target_os = "macos")]
    {
        paths.launch_agents_dir.join(format!("{name}.plist"))
    }
    #[cfg(not(target_os = "macos"))]
    {
        paths
            .systemd_unit_dir
            .join(format!("vakcoder-{}.service", short_name(name)))
    }
}

fn xml_escape(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
}

/// launchd property list: RunAtLoad + KeepAlive, stdout/stderr to the stable
/// log, with only non-secret HOME in the environment so canonical path
/// resolution cannot mistake the workspace for the user home. Credentials
/// still come from the user env file loaded by the binary itself.
pub fn render_launchd_plist(spec: &ServiceSpec) -> String {
    let mut prog_args = String::new();
    let bin = xml_escape(&spec.bin_path.to_string_lossy());
    prog_args.push_str(&format!("\n\t\t<string>{bin}</string>"));
    for arg in &spec.args {
        prog_args.push_str(&format!("\n\t\t<string>{}</string>", xml_escape(arg)));
    }
    let log = xml_escape(&spec.log_path.to_string_lossy());
    format!(
        r#"<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
	<key>KeepAlive</key>
	<true/>
	<key>Label</key>
	<string>{}</string>
	<key>ProgramArguments</key>
	<array>{}
	</array>
	<key>RunAtLoad</key>
	<true/>
	<key>EnvironmentVariables</key>
	<dict>
		<key>HOME</key>
		<string>{}</string>
	</dict>
	<key>WorkingDirectory</key>
	<string>{}</string>
	<key>StandardErrorPath</key>
	<string>{}</string>
	<key>StandardOutPath</key>
	<string>{}</string>
</dict>
</plist>
"#,
        xml_escape(spec.name),
        prog_args,
        xml_escape(&spec.home_dir.to_string_lossy()),
        xml_escape(&spec.working_dir.to_string_lossy()),
        log,
        log
    )
}

/// systemd user unit: Restart=always, journald bypassed in favour of the same
/// stable log files launchd uses, and no Environment= lines.
pub fn render_systemd_unit(spec: &ServiceSpec) -> String {
    let mut exec = spec.bin_path.to_string_lossy().into_owned();
    for arg in &spec.args {
        exec.push(' ');
        exec.push_str(arg);
    }
    let log = spec.log_path.to_string_lossy();
    format!(
        "# Generated by vak-ops — regenerate with `self services-sync`.\n\
         [Unit]\n\
         Description=vakcoder {}\n\
         After=network-online.target\n\
         Wants=network-online.target\n\
         \n\
         [Service]\n\
         ExecStart={exec}\n\
         Environment=HOME={}\n\
         WorkingDirectory={}\n\
         Restart=always\n\
         StandardOutput=append:{log}\n\
         StandardError=append:{log}\n\
         \n\
         [Install]\n\
         WantedBy=default.target\n",
        short_name(spec.name),
        spec.home_dir.display(),
        spec.working_dir.display(),
    )
}

fn render(spec: &ServiceSpec) -> String {
    #[cfg(target_os = "macos")]
    {
        render_launchd_plist(spec)
    }
    #[cfg(not(target_os = "macos"))]
    {
        render_systemd_unit(spec)
    }
}

/// Shell-out seam so tests record commands instead of touching the real
/// service manager.
pub trait CommandRunner {
    /// Run a command; true when it exited successfully.
    fn success(&self, program: &str, args: &[String]) -> bool;
    /// Run a command capturing trimmed stdout; None when it failed.
    fn text(&self, program: &str, args: &[String]) -> Option<String>;
}

/// The real runner: quiet subprocess execution.
#[derive(Debug, Default, Clone, Copy)]
pub struct SystemRunner;

impl CommandRunner for SystemRunner {
    fn success(&self, program: &str, args: &[String]) -> bool {
        Command::new(program)
            .args(args)
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .status()
            .map(|s| s.success())
            .unwrap_or(false)
    }

    fn text(&self, program: &str, args: &[String]) -> Option<String> {
        let out = Command::new(program)
            .args(args)
            .stdin(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .output()
            .ok()?;
        if !out.status.success() {
            return None;
        }
        Some(String::from_utf8_lossy(&out.stdout).trim().to_string())
    }
}

mod platform {
    use super::CommandRunner;
    use std::path::Path;

    #[cfg(target_os = "macos")]
    fn uid(runner: &dyn CommandRunner) -> String {
        runner
            .text("id", &["-u".to_string()])
            .filter(|u| !u.is_empty())
            .unwrap_or_else(|| "501".to_string())
    }

    /// Stop + deregister. Best-effort: a not-loaded service fails here and
    /// that is fine.
    pub fn unload(name: &str, runner: &dyn CommandRunner) {
        #[cfg(target_os = "macos")]
        {
            let target = format!("gui/{}/{}", uid(runner), name);
            runner.success("launchctl", &["bootout".into(), target]);
        }
        #[cfg(not(target_os = "macos"))]
        {
            runner.success(
                "systemctl",
                &[
                    "--user".to_string(),
                    "disable".to_string(),
                    "--now".to_string(),
                    systemd_unit(name),
                ],
            );
        }
    }

    /// Register the freshly written unit; RunAtLoad/enable --now starts it.
    pub fn load(name: &str, unit_path: &Path, runner: &dyn CommandRunner) -> bool {
        #[cfg(target_os = "macos")]
        {
            let _ = name;
            runner.success(
                "launchctl",
                &[
                    "bootstrap".to_string(),
                    format!("gui/{}", uid(runner)),
                    unit_path.display().to_string(),
                ],
            )
        }
        #[cfg(not(target_os = "macos"))]
        {
            let _ = unit_path;
            runner.success(
                "systemctl",
                &[
                    "--user".to_string(),
                    "enable".to_string(),
                    "--now".to_string(),
                    systemd_unit(name),
                ],
            )
        }
    }

    /// Start without touching registration (used when the unit is current
    /// but the process is down).
    pub fn start(name: &str, runner: &dyn CommandRunner) -> bool {
        #[cfg(target_os = "macos")]
        {
            runner.success(
                "launchctl",
                &[
                    "kickstart".to_string(),
                    format!("gui/{}/{}", uid(runner), name),
                ],
            )
        }
        #[cfg(not(target_os = "macos"))]
        {
            runner.success(
                "systemctl",
                &[
                    "--user".to_string(),
                    "start".to_string(),
                    systemd_unit(name),
                ],
            )
        }
    }

    /// Kill + restart in one step (kickstart -k keeps launchd KeepAlive
    /// semantics; systemctl restart is the systemd analogue). Used when the
    /// unit is current but a live process predates the installed binary —
    /// it would otherwise keep executing the old image indefinitely.
    pub fn restart(name: &str, runner: &dyn CommandRunner) -> bool {
        #[cfg(target_os = "macos")]
        {
            runner.success(
                "launchctl",
                &[
                    "kickstart".to_string(),
                    "-k".to_string(),
                    format!("gui/{}/{}", uid(runner), name),
                ],
            )
        }
        #[cfg(not(target_os = "macos"))]
        {
            runner.success(
                "systemctl",
                &[
                    "--user".to_string(),
                    "restart".to_string(),
                    systemd_unit(name),
                ],
            )
        }
    }

    /// Live PID according to the manager, None when not running.
    pub fn running_pid(name: &str, runner: &dyn CommandRunner) -> Option<u32> {
        #[cfg(target_os = "macos")]
        {
            let text = runner.text(
                "launchctl",
                &["print".to_string(), format!("gui/{}/{}", uid(runner), name)],
            )?;
            parse_launchd_pid(&text)
        }
        #[cfg(not(target_os = "macos"))]
        {
            let text = runner.text(
                "systemctl",
                &[
                    "--user".to_string(),
                    "show".to_string(),
                    "-P".to_string(),
                    "MainPID".to_string(),
                    systemd_unit(name),
                ],
            )?;
            text.parse::<u32>().ok().filter(|pid| *pid != 0)
        }
    }

    #[cfg(target_os = "macos")]
    fn parse_launchd_pid(text: &str) -> Option<u32> {
        for line in text.lines() {
            if let Some(rest) = line.trim().strip_prefix("pid = ") {
                let digits: String = rest.chars().take_while(char::is_ascii_digit).collect();
                if let Ok(pid) = digits.parse::<u32>() {
                    return Some(pid);
                }
            }
        }
        None
    }
}

use platform::{load, running_pid, start, unload};

/// Outcome of syncing one service.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SyncAction {
    /// Unit file did not exist; written and loaded.
    Created,
    /// Unit content drifted; unloaded, rewritten, reloaded.
    Updated,
    /// Unit identical and process live; untouched.
    Unchanged,
    /// Unit identical but process down; started without rewrite.
    Restarted,
    /// Unit identical but the running process predated the installed
    /// binary (started before the last `self install`); bounced so it
    /// executes the current image. Without this, an in-place upgrade
    /// leaves every service silently running stale code while `status`
    /// reports healthy pids (doc 32 invariant 4).
    Bounced,
    Failed(String),
}

#[derive(Debug, Clone)]
pub struct SyncOutcome {
    /// The requested service name; unknown requests carry the raw string.
    pub name: String,
    pub action: SyncAction,
}

/// A manager-level view of one service.
#[derive(Debug, Clone)]
pub struct ServiceRow {
    pub name: String,
    pub unit_path: PathBuf,
    /// Whether a unit file exists at all. Distinguishes "never synced"
    /// from "synced against the wrong binary": both leave
    /// `unit_points_at_installed` false, but only the second is a
    /// misconfiguration. A fresh install is the first, and reporting it
    /// as the second sends the operator after a problem they do not have.
    pub unit_present: bool,
    /// False for missing units and for legacy units exec'ing outside the
    /// install prefix (e.g. anything under `target/`) — the drift flag.
    /// Read together with `unit_present` to tell the two cases apart.
    pub unit_points_at_installed: bool,
    /// True when a live process predates the installed binary (started
    /// before the last install touched it) — stale-image drift.
    pub binary_stale: bool,
    pub running_pid: Option<u32>,
}

fn write_atomic(path: &Path, contents: &str) -> Result<(), String> {
    let tmp = path.with_extension("tmp");
    std::fs::write(&tmp, contents).map_err(|e| format!("write {}: {e}", tmp.display()))?;
    std::fs::rename(&tmp, path).map_err(|e| format!("rename into {}: {e}", path.display()))
}

/// True when the installed binary changed after the unit was last written:
/// a live service synced against the older image is executing stale code.
/// Missing files never count as stale (handled by other branches).
fn binary_newer_than_unit(bin_path: &Path, unit_path: &Path) -> bool {
    let (Ok(bin_meta), Ok(unit_meta)) = (std::fs::metadata(bin_path), std::fs::metadata(unit_path))
    else {
        return false;
    };
    let (Ok(bin_mtime), Ok(unit_mtime)) = (bin_meta.modified(), unit_meta.modified()) else {
        return false;
    };
    bin_mtime > unit_mtime
}

fn sync_one(spec: &ServiceSpec, paths: &Paths, runner: &dyn CommandRunner) -> SyncOutcome {
    let action = sync_one_inner(spec, paths, runner).unwrap_or_else(SyncAction::Failed);
    SyncOutcome {
        name: spec.name.to_string(),
        action,
    }
}

fn sync_one_inner(
    spec: &ServiceSpec,
    paths: &Paths,
    runner: &dyn CommandRunner,
) -> Result<SyncAction, String> {
    let rendered = render(spec);
    let unit_path = unit_file_path(spec.name, paths);
    if let Some(parent) = unit_path.parent() {
        std::fs::create_dir_all(parent).map_err(|e| format!("mkdir {}: {e}", parent.display()))?;
    }
    let previous = std::fs::read_to_string(&unit_path).ok();
    let was_running = running_pid(spec.name, runner).is_some();

    if previous.as_deref() == Some(rendered.as_str()) {
        return if !was_running {
            if start(spec.name, runner) {
                Ok(SyncAction::Restarted)
            } else {
                Err(format!("start {} failed", spec.name))
            }
        } else if binary_newer_than_unit(&spec.bin_path, &unit_path) {
            // The live process predates the installed binary. Bounce it and
            // re-stamp the unit so staleness converges — without the stamp,
            // every later sync would bounce again forever.
            if platform::restart(spec.name, runner) && write_atomic(&unit_path, &rendered).is_ok() {
                Ok(SyncAction::Bounced)
            } else {
                Err(format!(
                    "restart {} failed: process predates the installed binary",
                    spec.name
                ))
            }
        } else {
            Ok(SyncAction::Unchanged)
        };
    }

    // Content drift: take the old instance down before replacing its unit.
    // Bootout unconditionally — a crashed-but-loaded service (pid absent,
    // registration alive) would otherwise fail the later bootstrap with
    // "already bootstrapped" and roll back a perfectly good unit.
    unload(spec.name, runner);
    write_atomic(&unit_path, &rendered)?;
    if load(spec.name, &unit_path, runner) {
        Ok(if previous.is_some() {
            SyncAction::Updated
        } else {
            SyncAction::Created
        })
    } else {
        // A failed sync leaves the previous healthy unit in place.
        match previous {
            Some(old) => {
                let _ = write_atomic(&unit_path, &old);
            }
            None => {
                let _ = std::fs::remove_file(&unit_path);
            }
        }
        Err(format!("loading {} failed", unit_path.display()))
    }
}

/// Render → diff → reload each spec. Idempotent: identical content plus a
/// live process is a no-op.
pub fn sync_specs(
    specs: &[ServiceSpec],
    paths: &Paths,
    runner: &dyn CommandRunner,
) -> Vec<SyncOutcome> {
    specs
        .iter()
        .map(|spec| sync_one(spec, paths, runner))
        .collect()
}

/// Manager status per spec: does the on-disk unit reference the installed
/// binary, and what PID is the manager reporting?
pub fn status_specs(
    specs: &[ServiceSpec],
    paths: &Paths,
    runner: &dyn CommandRunner,
) -> Vec<ServiceRow> {
    specs
        .iter()
        .map(|spec| {
            let unit_path = unit_file_path(spec.name, paths);
            let on_disk = std::fs::read_to_string(&unit_path).ok();
            let wanted = spec.bin_path.to_string_lossy().into_owned();
            let pid = running_pid(spec.name, runner);
            let unit_present = on_disk.is_some();
            let points_at_installed = on_disk.is_some_and(|t| t.contains(wanted.as_str()));
            ServiceRow {
                name: spec.name.to_string(),
                unit_present,
                unit_points_at_installed: points_at_installed,
                // Stale-image drift: a live process synced against an
                // older binary. Only meaningful when the unit itself is
                // current, otherwise the legacy-path flag covers it.
                binary_stale: pid.is_some()
                    && points_at_installed
                    && binary_newer_than_unit(&spec.bin_path, &unit_path),
                unit_path,
                running_pid: pid,
            }
        })
        .collect()
}

/// Stop + unload + delete the named units. Missing units are already
/// uninstalled; nothing under the canonical platform data home is touched.
pub fn services_uninstall(
    names: &[&str],
    paths: &Paths,
    runner: &dyn CommandRunner,
) -> Result<(), String> {
    let mut failures = Vec::new();
    for name in names {
        unload(name, runner);
        let unit_path = unit_file_path(name, paths);
        if let Err(e) = std::fs::remove_file(&unit_path)
            && e.kind() != std::io::ErrorKind::NotFound
        {
            failures.push(format!("remove {}: {e}", unit_path.display()));
        }
    }
    if failures.is_empty() {
        Ok(())
    } else {
        Err(failures.join("; "))
    }
}

/// Resolve `names` against [`SERVICES`], preserving order. Unknown names land
/// as Err entries so callers can report them individually.
pub fn resolve_specs(bin_path: &Path, names: &[&str]) -> Vec<Result<ServiceSpec, String>> {
    let bin_dir = bin_path.parent().unwrap_or(Path::new("/"));
    let working_dir = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("/"));
    let home_dir = super::home();
    names
        .iter()
        .map(|name| {
            SERVICES
                .iter()
                .find(|def| def.name == *name)
                .map(|def| def.spec(bin_dir, &home_dir, &working_dir))
                .ok_or_else(|| format!("unknown service: {name}"))
        })
        .collect()
}

/// [`sync_specs`] over [`SERVICES`], rooted at the installed `bin_path`.
/// Unknown names yield `Failed` outcomes without touching anything.
pub fn services_sync(
    bin_path: &Path,
    names: &[&str],
    paths: &Paths,
    runner: &dyn CommandRunner,
) -> Vec<SyncOutcome> {
    resolve_specs(bin_path, names)
        .into_iter()
        .zip(names.iter())
        .map(|(resolved, name)| match resolved {
            Ok(spec) => sync_one(&spec, paths, runner),
            Err(msg) => SyncOutcome {
                name: (*name).to_string(),
                action: SyncAction::Failed(msg),
            },
        })
        .collect()
}

/// [`status_specs`] over [`SERVICES`], rooted at the installed `bin_path`.
pub fn services_status(
    bin_path: &Path,
    names: &[&str],
    paths: &Paths,
    runner: &dyn CommandRunner,
) -> Vec<ServiceRow> {
    let specs: Vec<ServiceSpec> = resolve_specs(bin_path, names)
        .into_iter()
        .flatten()
        .collect();
    status_specs(&specs, paths, runner)
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::*;
    use std::sync::Mutex;

    struct Fake {
        cmds: Mutex<Vec<(String, Vec<String>)>>,
        pid_text: Option<String>,
        fail_load: bool,
    }
    impl Fake {
        fn new() -> Self {
            Fake {
                cmds: Mutex::new(Vec::new()),
                pid_text: None,
                fail_load: false,
            }
        }
        fn with_pid(pid: u32) -> Self {
            Fake {
                pid_text: Some(format!("com.vakcoder.x = {{\n\tpid = {pid}\n}}")),
                ..Self::new()
            }
        }
    }
    impl CommandRunner for Fake {
        fn success(&self, program: &str, args: &[String]) -> bool {
            self.cmds
                .lock()
                .unwrap()
                .push((program.to_string(), args.to_vec()));
            !(self.fail_load
                && program == "launchctl"
                && args.first().map(String::as_str) == Some("bootstrap"))
        }
        fn text(&self, program: &str, args: &[String]) -> Option<String> {
            if program == "id" {
                return Some("501".to_string());
            }
            self.success(program, args);
            self.pid_text.clone()
        }
    }

    fn tmp_paths(tag: &str) -> (tempfile::TempDir, Paths) {
        let dir = tempfile::tempdir().unwrap();
        let paths = Paths {
            launch_agents_dir: dir.path().join(format!("{tag}-agents")),
            systemd_unit_dir: dir.path().join(format!("{tag}-units")),
        };
        (dir, paths)
    }

    fn spec_for(def: &ServiceDef, bin_dir: &Path, log_dir: &Path) -> ServiceSpec {
        ServiceSpec {
            name: def.name,
            bin_path: bin_dir.join(def.bin_file),
            args: def.args.iter().map(|a| (*a).to_string()).collect(),
            log_path: log_dir.join(def.log_file),
            working_dir: PathBuf::from("/workspace"),
            home_dir: PathBuf::from("/Users/x"),
        }
    }

    #[test]
    fn launchd_plist_renders_paths_args_and_zero_secrets() {
        let def = &SERVICES[0];
        let spec = spec_for(
            def,
            Path::new("/opt/vak/bin"),
            Path::new("/Users/x/.vakcoder/logs"),
        );
        let plist = render_launchd_plist(&spec);
        assert!(plist.contains("/opt/vak/bin/vakcoder"));
        for a in def.args {
            assert!(plist.contains(a), "missing arg {a}");
        }
        assert!(plist.contains("/Users/x/.vakcoder/logs/gateway.log"));
        assert!(plist.contains("KeepAlive"));
        assert!(plist.contains("RunAtLoad"));
        assert!(plist.contains("<key>HOME</key>"));
        assert!(plist.contains("<string>/Users/x</string>"));
        // Update safety (doc 32): units never embed credentials.
        for secret in ["TOKEN", "SECRET", "BOT_TOKEN"] {
            assert!(!plist.contains(secret), "unit must not contain {secret}");
        }
    }

    /// Canonical-layout invariant (doc 32): specs derived from the real
    /// resolver never point logs into the legacy dotdir nor binaries at
    /// a build tree. This is the regression guard for the 0.7 incident
    /// where services silently executed stale images from ad-hoc paths.
    #[test]
    fn resolved_specs_never_reference_legacy_dotdir_or_build_trees() {
        let specs: Vec<ServiceSpec> = resolve_specs(
            Path::new("/Applications/vakcoder.app/Contents/MacOS/vakcoder"),
            &[],
        )
        .into_iter()
        .flatten()
        .collect();
        for spec in specs {
            let log = spec.log_path.to_string_lossy();
            let bin = spec.bin_path.to_string_lossy();
            assert!(
                !log.contains("/.vakcoder/"),
                "log path must use the canonical logs dir, got {log}"
            );
            assert!(
                !bin.contains("/target/"),
                "binaries must come from the managed install, got {bin}"
            );
            assert!(
                bin.starts_with("/Applications/vakcoder.app/"),
                "binaries must live inside the installed bundle, got {bin}"
            );
            assert!(
                log.contains("Library/Logs/vakcoder"),
                "logs must land in Library/Logs on macOS, got {log}"
            );
        }
    }

    #[test]
    fn systemd_unit_renders_restart_and_exec() {
        let def = &SERVICES[0];
        let spec = spec_for(def, Path::new("/opt/vak/bin"), Path::new("/h/.vakcoder"));
        let unit = render_systemd_unit(&spec);
        assert!(
            unit.contains("ExecStart="),
            "unit missing ExecStart: {unit}"
        );
        assert!(unit.contains("/opt/vak/bin/vakcoder"), "{unit}");
        assert!(unit.contains("--gateway"), "{unit}");
        assert!(unit.contains("Restart=always"));
    }

    #[test]
    fn sync_is_created_then_unchanged_when_running() {
        let (_d, paths) = tmp_paths("sync1");
        let fake = Fake::with_pid(4242);
        let specs: Vec<ServiceSpec> = SERVICES
            .iter()
            .map(|d| spec_for(d, Path::new("/opt/vak/bin"), Path::new("/tmp/logs")))
            .collect();

        let first = sync_specs(&specs, &paths, &fake);
        assert!(
            matches!(first[0].action, SyncAction::Created),
            "{:?}",
            first[0]
        );
        let unit = std::fs::read_to_string(unit_file_path(SERVICES[0].name, &paths)).unwrap();
        assert!(unit.contains("/opt/vak/bin/vakcoder"));

        let second = sync_specs(&specs, &paths, &fake);
        assert!(
            matches!(second[0].action, SyncAction::Unchanged),
            "{:?}",
            second[0]
        );
    }

    #[test]
    fn sync_restarts_when_unit_current_but_process_down() {
        let (_d, paths) = tmp_paths("sync2");
        let down = Fake::new(); // no pid text => not running
        let up = Fake::with_pid(7);
        let specs: Vec<ServiceSpec> = SERVICES
            .iter()
            .map(|d| spec_for(d, Path::new("/opt/vak/bin"), Path::new("/tmp/logs")))
            .collect();
        let _ = sync_specs(&specs, &paths, &up); // create while "running"
        let out = sync_specs(&specs, &paths, &down);
        assert!(
            matches!(out[0].action, SyncAction::Restarted),
            "{:?}",
            out[0]
        );
    }

    #[test]
    fn sync_bounces_live_process_predating_installed_binary() {
        let dir = tempfile::tempdir().unwrap();
        let bin = dir.path().join("vakcoder");
        std::fs::write(&bin, b"binary").unwrap();
        let (_d, paths) = tmp_paths("sync3");
        let fake = Fake::with_pid(99);
        let specs: Vec<ServiceSpec> = SERVICES
            .iter()
            .map(|d| spec_for(d, dir.path(), Path::new("/tmp/logs")))
            .collect();

        assert!(matches!(
            sync_specs(&specs, &paths, &fake)[0].action,
            SyncAction::Created
        ));

        // Simulate `self install` replacing the binary AFTER the unit was
        // written: push the unit's mtime into the past relative to the
        // binary so the running pid predates the current image.
        let now = std::time::SystemTime::now();
        let unit_path = unit_file_path(SERVICES[0].name, &paths);
        // Unit 10s in the past, binary at now → running pid predates image.
        for (path, age) in [(&unit_path, 10u64), (&bin, 0u64)] {
            let f = std::fs::OpenOptions::new().write(true).open(path).unwrap();
            f.set_modified(now - std::time::Duration::from_secs(age))
                .unwrap();
            drop(f);
        }

        let second = sync_specs(&specs, &paths, &fake);
        assert!(
            matches!(second[0].action, SyncAction::Bounced),
            "expected Bounced, got {:?}",
            second[0]
        );
        {
            // Scope the guard: sync_specs locks the same log internally.
            let bounced = fake.cmds.lock().unwrap();
            assert!(
                bounced.iter().any(|(p, a)| p == "launchctl"
                    && a.first().map(String::as_str) == Some("kickstart")
                    && a.contains(&"-k".to_string())),
                "expected kickstart -k among {:?}",
                *bounced
            );
        }

        // The bounce re-stamps the unit, so later syncs are true no-ops.
        let third = sync_specs(&specs, &paths, &fake);
        assert!(
            matches!(third[0].action, SyncAction::Unchanged),
            "staleness must converge after a bounce, got {:?}",
            third[0]
        );
    }

    #[test]
    fn status_flags_stale_running_process() {
        let dir = tempfile::tempdir().unwrap();
        let bin = dir.path().join("vakcoder");
        std::fs::write(&bin, b"binary").unwrap();
        let (_d, paths) = tmp_paths("sync4");
        let fake = Fake::with_pid(11);
        let specs: Vec<ServiceSpec> = SERVICES
            .iter()
            .map(|d| spec_for(d, dir.path(), Path::new("/tmp/logs")))
            .collect();
        let _ = sync_specs(&specs, &paths, &fake);

        let fresh = status_specs(&specs, &paths, &fake)[0].clone();
        assert!(!fresh.binary_stale);

        // Binary replaced after the unit landed → running pid is stale.
        let now = std::time::SystemTime::now();
        let unit_path = unit_file_path(SERVICES[0].name, &paths);
        let f = std::fs::OpenOptions::new()
            .write(true)
            .open(&unit_path)
            .unwrap();
        f.set_modified(now - std::time::Duration::from_secs(10))
            .unwrap();
        drop(f);
        let stale = status_specs(&specs, &paths, &fake)[0].clone();
        assert!(stale.binary_stale, "{stale:?}");
        assert!(stale.running_pid.is_some());
    }

    #[test]
    fn sync_updates_on_drift_and_restores_previous_unit_on_load_failure() {
        let (_d, paths) = tmp_paths("sync3");
        let good = Fake::with_pid(9);
        let specs: Vec<ServiceSpec> = SERVICES
            .iter()
            .map(|d| spec_for(d, Path::new("/opt/vak/bin"), Path::new("/tmp/logs")))
            .collect();
        let _ = sync_specs(&specs, &paths, &good);
        let unit_path = unit_file_path(SERVICES[0].name, &paths);
        let original = std::fs::read_to_string(&unit_path).unwrap();

        // Drifted content + a manager that refuses bootstrap.
        let bad = Fake {
            fail_load: true,
            ..Fake::with_pid(9)
        };
        let drifted = spec_for(
            &SERVICES[0],
            Path::new("/elsewhere/bin"),
            Path::new("/tmp/logs"),
        );
        let out = sync_one(&drifted, &paths, &bad);
        assert!(
            matches!(out.action, SyncAction::Failed(_)),
            "{:?}",
            out.action
        );
        // Rollback: the previous healthy unit is still on disk.
        assert_eq!(std::fs::read_to_string(&unit_path).unwrap(), original);

        // A healthy runner sees content drift and reports Updated.
        let moved = sync_one(&drifted, &paths, &good);
        assert!(
            matches!(moved.action, SyncAction::Updated),
            "{:?}",
            moved.action
        );
    }

    #[test]
    fn status_flags_units_not_pointing_at_installed_binary() {
        let (_d, paths) = tmp_paths("status");
        let fake = Fake::with_pid(11);
        let installed: Vec<ServiceSpec> = SERVICES
            .iter()
            .map(|d| spec_for(d, Path::new("/opt/vak/bin"), Path::new("/l")))
            .collect();
        let _ = sync_specs(&installed, &paths, &fake);

        let rows = status_specs(&installed, &paths, &fake);
        assert!(rows.iter().all(|r| r.unit_points_at_installed), "{rows:?}");

        // Legacy layout: same name, binary inside a build tree ⇒ drift flag.
        let legacy: Vec<ServiceSpec> = SERVICES
            .iter()
            .map(|d| spec_for(d, Path::new("/repo/target/release"), Path::new("/l")))
            .collect();
        let rows = status_specs(&legacy, &paths, &fake);
        assert!(rows.iter().all(|r| !r.unit_points_at_installed), "{rows:?}");
    }

    #[test]
    fn uninstall_removes_units_and_tolerates_missing() {
        let (_d, paths) = tmp_paths("rm");
        let fake = Fake::new();
        let names: Vec<&str> = SERVICES.iter().map(|d| d.name).collect();
        services_uninstall(&names, &paths, &fake).unwrap(); // nothing to remove yet
        let installed: Vec<ServiceSpec> = SERVICES
            .iter()
            .map(|d| spec_for(d, Path::new("/b"), Path::new("/l")))
            .collect();
        let _ = sync_specs(&installed, &paths, &fake);
        services_uninstall(&names, &paths, &fake).unwrap();
        for def in SERVICES {
            assert!(!unit_file_path(def.name, &paths).exists());
        }
    }

    #[test]
    fn resolve_specs_reports_unknown_names_without_touching_known_ones() {
        let out = resolve_specs(Path::new("/b"), &["com.vakcoder.gateway", "nope"]);
        assert!(out[0].is_ok());
        assert!(out[1].as_ref().err().unwrap().contains("unknown service"));

        let synced = services_sync(Path::new("/b"), &["nope"], &Paths::default(), &Fake::new());
        assert!(matches!(synced[0].action, SyncAction::Failed(_)));
    }
}
