//! Generated service units (docs/design/32-release-engineering.md §3).
//!
//! Units are rendered from [`SERVICES`] and diffed onto disk by
//! [`services_sync`]; nothing here is hand-edited. Templates embed zero
//! credentials: the binary self-sources `~/.vakcoder/.env`, so regenerating
//! units can never strand auth. Logs stay at stable
//! `~/.vakcoder/logs/<service>.log`; user data under `~/.vakcoder` is only
//! ever appended to by the running services themselves.
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
    /// Stable log destination under `~/.vakcoder/logs`.
    pub log_path: PathBuf,
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
        args: &["serve", "--gateway"],
        log_file: "gateway.log",
    },
    ServiceDef {
        name: "com.vakcoder.telegram",
        bin_file: "vakcoder",
        args: &["telegram", "--server", "http://127.0.0.1:8901"],
        log_file: "telegram.log",
    },
    ServiceDef {
        name: "com.vakcoder.tray",
        bin_file: "vakcoder-tray",
        args: &[],
        log_file: "tray.log",
    },
];

impl ServiceDef {
    /// Resolve against an install prefix: `bin_dir` holds the release
    /// binaries, `vak_home` anchors the stable log directory.
    pub fn spec(&self, bin_dir: &Path, vak_home: &Path) -> ServiceSpec {
        ServiceSpec {
            name: self.name,
            bin_path: bin_dir.join(self.bin_file),
            args: self.args.iter().map(|a| (*a).to_string()).collect(),
            log_path: vak_home.join("logs").join(self.log_file),
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
/// log, and deliberately no EnvironmentVariables block (secrets come from
/// `~/.vakcoder/.env`, loaded by the binary itself).
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
	<key>StandardErrorPath</key>
	<string>{}</string>
	<key>StandardOutPath</key>
	<string>{}</string>
</dict>
</plist>
"#,
        xml_escape(spec.name),
        prog_args,
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
        "# Generated by vak-ops — regenerate with `self services sync`.\n\
         [Unit]\n\
         Description=vakcoder {}\n\
         After=network-online.target\n\
         Wants=network-online.target\n\
         \n\
         [Service]\n\
         ExecStart={exec}\n\
         Restart=always\n\
         StandardOutput=append:{log}\n\
         StandardError=append:{log}\n\
         \n\
         [Install]\n\
         WantedBy=default.target\n",
        short_name(spec.name),
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

    #[cfg(target_os = "macos")]
    fn uid(runner: &dyn CommandRunner) -> String {
        runner
            .text("id", &["-u".to_string()])
            .filter(|u| !u.is_empty())
            .unwrap_or_else(|| "501".to_string())
    }

    fn systemd_unit(name: &str) -> String {
        format!("vakcoder-{}.service", super::short_name(name))
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
                &["kickstart".to_string(), format!("gui/{}/{}", uid(runner), name)],
            )
        }
        #[cfg(not(target_os = "macos"))]
        {
            runner.success("systemctl", &["--user".to_string(), "start".to_string(), systemd_unit(name)])
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
                let digits: String =
                    rest.chars().take_while(char::is_ascii_digit).collect();
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
    /// False for missing units and for legacy units exec'ing outside the
    /// install prefix (e.g. anything under `target/`) — the drift flag.
    pub unit_points_at_installed: bool,
    pub running_pid: Option<u32>,
}

fn write_atomic(path: &Path, contents: &str) -> Result<(), String> {
    let tmp = path.with_extension("tmp");
    std::fs::write(&tmp, contents).map_err(|e| format!("write {}: {e}", tmp.display()))?;
    std::fs::rename(&tmp, path).map_err(|e| format!("rename into {}: {e}", path.display()))
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
        return if was_running {
            Ok(SyncAction::Unchanged)
        } else if start(spec.name, runner) {
            Ok(SyncAction::Restarted)
        } else {
            Err(format!("start {} failed", spec.name))
        };
    }

    // Content drift: take the old instance down before replacing its unit.
    if was_running {
        unload(spec.name, runner);
    }
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
            ServiceRow {
                name: spec.name.to_string(),
                unit_points_at_installed: on_disk.is_some_and(|t| t.contains(wanted.as_str())),
                unit_path,
                running_pid: running_pid(spec.name, runner),
            }
        })
        .collect()
}

/// Stop + unload + delete the named units. Missing units are already
/// uninstalled; nothing under `~/.vakcoder` is touched.
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
    let vak_home = super::home().join(".vakcoder");
    names
        .iter()
        .map(|name| {
            SERVICES
                .iter()
                .find(|def| def.name == *name)
                .map(|def| def.spec(bin_dir, &vak_home))
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
