//! Generated service units (docs/design/32-release-engineering.md §3).
//!
//! Units are rendered from [`SERVICES`] and diffed onto disk by
//! [`services_sync`]; nothing here is hand-edited. Templates embed zero
//! credentials: the binary self-sources `data_home()/.env`, so regenerating
//! units can never strand auth. Logs stay at the platform `logs_dir()` and
//! user data remains under the canonical platform data home.
//! Headless units always run from [`vak_config::paths::default_workspace`],
//! so invoking `self services-sync` from a source checkout or another project
//! can never silently rebind the gateway and channel bridges to that directory.
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
    /// launchd label, also the plist stem (`com.vak.gateway`). Owned rather
    /// than `&'static str` because per-bot units (`com.vak.telegram-<id>`)
    /// are named dynamically from `bots.json`, not from the static
    /// [`SERVICES`] table.
    pub name: String,
    pub bin_path: PathBuf,
    pub args: Vec<String>,
    /// Stable log destination under the canonical platform logs directory.
    pub log_path: PathBuf,
    /// Workspace the service must load for config and project-local `.env`.
    pub working_dir: PathBuf,
    /// User home required by platform path resolution in the sanitized
    /// service-manager environment. This is operational state, not a secret.
    pub home_dir: PathBuf,
    /// Non-secret executable search path captured when the unit is synced.
    /// MCP commands such as `npx` are resolved by the service manager, whose
    /// default PATH is usually smaller than the interactive shell's PATH.
    pub path_env: String,
    /// Whether the manager should resurrect the process when it exits
    /// (launchd `KeepAlive`, systemd `Restart=always`). False for GUI
    /// services, where an explicit user quit must actually quit.
    pub keep_alive: bool,
    /// Whether the process needs a real Aqua login session (a WindowServer
    /// connection and a LaunchServices check-in). See [`ServiceDef::gui`].
    pub gui: bool,
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
    /// Manager-level resurrection (launchd `KeepAlive`, systemd
    /// `Restart=always`).
    pub keep_alive: bool,
    /// Whether the unit must be pinned to Vak's canonical default workspace.
    /// Headless servers load that workspace's config and project `.env`; a
    /// GUI app that picks its own project in-app instead runs from the account
    /// home and must not be silently bound to one directory.
    pub workspace_scoped: bool,
    /// The binary ships only when the build produced it (see `COMPONENTS`
    /// in the installer). A unit exec'ing a path that does not exist is
    /// worse than no unit, so these are skipped when absent.
    pub optional: bool,
    /// The unit draws on screen and must therefore be pinned to the Aqua
    /// login session (`LimitLoadToSessionType`).
    ///
    /// Without that key launchd runs the job in the plain background
    /// `gui/<uid>` domain: the process starts and stays up, but it never
    /// checks in with LaunchServices and gets no WindowServer (CGS)
    /// connection, so it can draw no menu-bar icon at all. `lsappinfo`
    /// reports the difference exactly — `bundle path=[NULL]`,
    /// `Arch=!!none`, `!cgsConnection` without the key, versus
    /// `type="Foreground"` with a real session token once it is set.
    /// Headless services must stay false: they have no UI to place, and
    /// pinning them to Aqua would stop them loading in a non-GUI session.
    pub gui: bool,
}

pub const SERVICES: &[ServiceDef] = &[
    ServiceDef {
        name: "com.vak.gateway",
        bin_file: "vak",
        args: &["serve", "--gateway", "--trust"],
        log_file: "gateway.log",
        keep_alive: true,
        workspace_scoped: true,
        optional: false,
        gui: false,
    },
    ServiceDef {
        name: "com.vak.telegram",
        bin_file: "vak",
        args: &["telegram", "--server", "http://127.0.0.1:8901"],
        log_file: "telegram.log",
        keep_alive: true,
        workspace_scoped: true,
        optional: false,
        gui: false,
    },
    // The desktop app is what puts the menu-bar icon on screen; without a
    // unit nothing brings it back after a logout or reboot, so the tray —
    // the surface that starts and stops everything else — was the one
    // thing that did not survive one.
    //
    // KeepAlive is deliberately OFF, unlike the headless services above.
    // The tray menu's `Quit Vak` calls `app.exit(0)`; under KeepAlive
    // launchd would relaunch it a second later and Quit would visibly not
    // quit. RunAtLoad still gives the "back after login/reboot" behaviour
    // that is the whole point, and a genuinely crashed GUI app is better
    // left down than silently respawned in a loop the user cannot see.
    //
    // `--tray` starts with the window hidden: a login-launched app that
    // threw a 1440x900 window on screen at every boot would be a worse
    // regression than the missing persistence it fixes.
    ServiceDef {
        name: "com.vak.desktop",
        bin_file: "vak-desktop",
        args: &["--tray"],
        log_file: "desktop.log",
        keep_alive: false,
        workspace_scoped: false,
        optional: true,
        gui: true,
    },
];

/// Obsolete GUI launch agents that must be removed during an upgrade. The
/// desktop app now owns its tray, so a separate launchd tray must never keep
/// the bundle's LaunchServices identity alive without a window to reveal.
pub const RETIRED_SERVICES: &[&str] = &["com.vak.tray"];

// ------------------------------------------------------- multi-bot units
//
// docs/design/34 (multi-bot-per-channel): a user adds/removes Telegram,
// Discord, and Slack bots at any time from the admin console, each getting
// its own id and its own token env var recorded in `bots.json`. Unlike
// [`SERVICES`] above, these units cannot be a static compile-time table —
// there is no fixed number of bots, and the set changes at runtime as bots
// are created, deleted, or renamed. Everything below reads `bots.json`
// fresh each time and derives one unit per configured bot, named
// `com.vak.<surface>-<id>` (e.g. `com.vak.telegram-VakBot`), each launched
// with `--bot-id <id>` so it resolves that bot's own token env var
// (`vak_server::gateway::bot_token_env_for_id`) instead of the legacy
// single-bot slot. This crate cannot depend on vak-server (vak-server
// already depends on vak-ops), so the tiny bit of `bots.json` schema it
// needs is duplicated here rather than shared.

/// Surfaces with a CLI bridge that takes `--server <url> --bot-id <id>`
/// (see `vak telegram|discord|slack` in `crates/vak/src/cli.rs`).
const BRIDGE_SURFACES: &[&str] = &["telegram", "discord", "slack"];

#[derive(Debug, Clone, serde::Deserialize)]
struct BotRecord {
    id: String,
    surface: String,
}

#[derive(Debug, Default, serde::Deserialize)]
struct BotsFile {
    #[serde(default)]
    bots: Vec<BotRecord>,
}

fn bots_json_path(data_home: &Path) -> PathBuf {
    data_home.join("gateway").join("bots.json")
}

/// Read the bots a user has configured. Missing file or parse failure reads
/// as "no bots" rather than an error — a fresh install has no `bots.json`
/// yet, and a corrupt one must not stop the gateway/desktop units from
/// syncing.
fn read_bots(data_home: &Path) -> Vec<BotRecord> {
    std::fs::read_to_string(bots_json_path(data_home))
        .ok()
        .and_then(|raw| serde_json::from_str::<BotsFile>(&raw).ok())
        .map(|f| f.bots)
        .unwrap_or_default()
}

/// Whether the user has configured at least one bot for `surface` in
/// `bots.json`. Used to keep the legacy bot-id-less static unit for that
/// surface (e.g. `com.vak.telegram`) out of the sync set once real
/// per-bot units have taken over — running both against the same token
/// env produces two long-pollers on the same bot and 409 Conflicts on the
/// Telegram API.
pub fn has_configured_bots(data_home: &Path, surface: &str) -> bool {
    read_bots(data_home)
        .into_iter()
        .any(|b| b.surface == surface)
}

/// launchd/systemd labels only tolerate a narrow character set; a bot id is
/// operator-chosen (the admin console enforces alphanumeric/hyphen today,
/// but this is a second, independent line of defense against a stray id
/// producing a unit name the service manager rejects or a path that escapes
/// the units directory).
fn sanitize_for_unit_name(id: &str) -> String {
    id.chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '-' {
                c
            } else {
                '_'
            }
        })
        .collect()
}

/// The launchd label / systemd stem for one bot's bridge unit.
pub fn bot_service_name(surface: &str, id: &str) -> String {
    format!("com.vak.{surface}-{}", sanitize_for_unit_name(id))
}

/// One [`ServiceSpec`] per bot currently in `bots.json`, ready for
/// [`sync_specs`]. Surfaces without a CLI bridge (unrecognized `surface`
/// values) are skipped rather than producing a unit that can never run.
pub fn bot_service_specs(
    bin_dir: &Path,
    home_dir: &Path,
    default_workspace: &Path,
    data_home: &Path,
    gateway_url: &str,
) -> Vec<ServiceSpec> {
    read_bots(data_home)
        .into_iter()
        .filter(|b| BRIDGE_SURFACES.contains(&b.surface.as_str()))
        .map(|b| {
            let name = bot_service_name(&b.surface, &b.id);
            let log_file = format!("{}-{}.log", b.surface, sanitize_for_unit_name(&b.id));
            ServiceSpec {
                name,
                bin_path: bin_dir.join("vak"),
                args: vec![
                    b.surface,
                    "--server".to_string(),
                    gateway_url.to_string(),
                    "--bot-id".to_string(),
                    b.id,
                ],
                log_path: vak_config::paths::logs_dir().join(log_file),
                working_dir: default_workspace.to_path_buf(),
                home_dir: home_dir.to_path_buf(),
                path_env: std::env::var("PATH").unwrap_or_default(),
                keep_alive: true,
                gui: false,
            }
        })
        .collect()
}

/// Units matching `com.vak.<surface>-*` on disk that are no longer in
/// `wanted` get stopped, deregistered, and their unit file removed — the
/// counterpart to a bot being deleted or renamed in the admin console.
/// Without this, a deleted bot's bridge process (and its stale token env
/// reference) would keep running forever, invisible to `bots.json`.
fn prune_stale_bot_units(wanted: &[String], paths: &Paths, runner: &dyn CommandRunner) {
    let dir = if cfg!(target_os = "macos") {
        &paths.launch_agents_dir
    } else {
        &paths.systemd_unit_dir
    };
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let file_name = entry.file_name();
        let file_name = file_name.to_string_lossy();
        let is_bot_unit = BRIDGE_SURFACES
            .iter()
            .any(|s| file_name.starts_with(&format!("com.vak.{s}-")));
        if !is_bot_unit {
            continue;
        }
        let stem = file_name
            .strip_suffix(".plist")
            .or_else(|| {
                file_name
                    .strip_suffix(".service")
                    .map(|s| s.trim_start_matches("vak-"))
            })
            .unwrap_or(&file_name);
        // systemd stems are stripped of the "com.vak." prefix by
        // `short_name`; reconstruct the launchd-style label to compare.
        let label = if file_name.ends_with(".service") {
            format!("com.vak.{stem}")
        } else {
            stem.to_string()
        };
        if !wanted.contains(&label) {
            let _ = services_uninstall(&[label.as_str()], paths, runner);
        }
    }
}

/// Reconcile every currently-configured bot's unit against the service
/// manager: create units for new bots, update ones whose args changed
/// (token env, surface), leave healthy ones alone, and remove units for
/// bots that were deleted or renamed. Called after every bot create/update
/// (surface change)/delete/token change so a user editing bots in the admin
/// console never has to know a launchd/systemd unit is involved.
pub fn sync_bots(
    bin_path: &Path,
    data_home: &Path,
    gateway_url: &str,
    paths: &Paths,
    runner: &dyn CommandRunner,
) -> Vec<SyncOutcome> {
    let bin_dir = bin_path.parent().unwrap_or(Path::new("/"));
    let home_dir = super::home();
    let default_workspace = vak_config::paths::default_workspace();
    let specs = bot_service_specs(
        bin_dir,
        &home_dir,
        &default_workspace,
        data_home,
        gateway_url,
    );
    let wanted: Vec<String> = specs.iter().map(|s| s.name.clone()).collect();
    prune_stale_bot_units(&wanted, paths, runner);
    sync_specs(&specs, paths, runner)
}

/// Remove every per-bot bridge unit found on disk (`com.vak.<surface>-*`),
/// regardless of what `bots.json` currently says. For use at uninstall
/// time, where nothing should be left running — [`sync_bots`]'s normal
/// diff-against-`bots.json` behaviour is the wrong shape there, since an
/// uninstall wants "wanted = nothing", not "wanted = whatever's still
/// configured".
pub fn uninstall_bot_units(paths: &Paths, runner: &dyn CommandRunner) {
    prune_stale_bot_units(&[], paths, runner);
}

/// Bounce one bot's already-installed unit so its process re-reads `.env`.
/// A token rotate or removal changes no unit *content* — the token itself
/// is never embedded in the plist/unit, only its env var name is, and that
/// name doesn't change — so [`sync_bots`]'s identity diff would see
/// `Unchanged` and never restart the process. Call this alongside
/// [`sync_bots`] whenever a bot's token is set or cleared. Best-effort: a
/// bot with no unit yet (token set before the first sync) simply reports
/// `false`, which is fine — [`sync_bots`] will create and start it fresh.
pub fn restart_bot_unit(surface: &str, id: &str, runner: &dyn CommandRunner) -> bool {
    platform::restart(&bot_service_name(surface, id), runner)
}

impl ServiceDef {
    /// Resolve against an install prefix: `bin_dir` holds the release
    /// binaries; logs land in the canonical platform logs dir
    /// (`~/Library/Logs/vak` / XDG state) — never inside data.
    pub fn spec(&self, bin_dir: &Path, home_dir: &Path, default_workspace: &Path) -> ServiceSpec {
        ServiceSpec {
            name: self.name.to_string(),
            bin_path: bin_dir.join(self.bin_file),
            args: self.args.iter().map(|a| (*a).to_string()).collect(),
            log_path: vak_config::paths::logs_dir().join(self.log_file),
            // Non-workspace-scoped services get the account home: they
            // choose their own project at runtime. Headless services use the
            // canonical default workspace, never the caller's current dir.
            working_dir: if self.workspace_scoped {
                default_workspace.to_path_buf()
            } else {
                home_dir.to_path_buf()
            },
            home_dir: home_dir.to_path_buf(),
            path_env: std::env::var("PATH").unwrap_or_default(),
            keep_alive: self.keep_alive,
            gui: self.gui,
        }
    }

    /// systemd unit stem derived from the launchd label
    /// (`com.vak.gateway` → `vak-gateway.service`).
    pub fn systemd_unit(&self) -> String {
        format!(
            "vak-{}.service",
            self.name.strip_prefix("com.vak.").unwrap_or(self.name)
        )
    }
}

fn short_name(name: &str) -> &str {
    name.strip_prefix("com.vak.").unwrap_or(name)
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
            .join(format!("vak-{}.service", short_name(name)))
    }
}

fn xml_escape(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
}

/// launchd property list: RunAtLoad always, KeepAlive per definition
/// (off for GUI services so an explicit quit sticks), stdout/stderr to the stable
/// log, with only non-secret HOME and PATH in the environment so canonical path
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
    // GUI units must be pinned to the Aqua login session or launchd hands
    // them a background job with no WindowServer connection — the process
    // runs, logs nothing, and silently draws no menu-bar icon.
    let session_type = if spec.gui {
        "\n\t<key>LimitLoadToSessionType</key>\n\t<string>Aqua</string>"
    } else {
        ""
    };
    format!(
        r#"<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
	<key>KeepAlive</key>
	<{}/>{}
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
		<key>PATH</key>
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
        if spec.keep_alive { "true" } else { "false" },
        session_type,
        xml_escape(&spec.name),
        prog_args,
        xml_escape(&spec.home_dir.to_string_lossy()),
        xml_escape(&spec.path_env),
        xml_escape(&spec.working_dir.to_string_lossy()),
        log,
        log
    )
}

/// systemd user unit: Restart per definition, journald bypassed in favour of the same
/// stable log files launchd uses, with only non-secret HOME and PATH.
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
         Description=vak {}\n\
         After=network-online.target\n\
         Wants=network-online.target\n\
         \n\
         [Service]\n\
         ExecStart={exec}\n\
         Environment=HOME={}\n\
         Environment=PATH={}\n\
         WorkingDirectory={}\n\
         Restart={}\n\
         StandardOutput=append:{log}\n\
         StandardError=append:{log}\n\
         \n\
         [Install]\n\
         WantedBy=default.target\n",
        short_name(&spec.name),
        spec.home_dir.display(),
        spec.path_env,
        spec.working_dir.display(),
        if spec.keep_alive { "always" } else { "no" },
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

/// Bootstrap can lose a transient race — e.g. the manager hasn't finished
/// tearing down the just-booted-out registration yet — and fail once even
/// though the unit is fine. A single failure here used to be terminal: the
/// caller fell back to restoring the previous unit's *content* but left the
/// service unloaded, silently, with nothing to notice or retry it (the
/// telegram bridge going dark across a `self update` traced back to exactly
/// this). Retry a few times with a short backoff before giving up.
const LOAD_RETRIES: u32 = 3;
const LOAD_RETRY_DELAY: std::time::Duration = std::time::Duration::from_millis(300);

fn load_with_retries(name: &str, unit_path: &Path, runner: &dyn CommandRunner) -> bool {
    for attempt in 0..LOAD_RETRIES {
        if load(name, unit_path, runner) {
            return true;
        }
        if attempt + 1 < LOAD_RETRIES {
            std::thread::sleep(LOAD_RETRY_DELAY);
        }
    }
    false
}

fn sync_one_inner(
    spec: &ServiceSpec,
    paths: &Paths,
    runner: &dyn CommandRunner,
) -> Result<SyncAction, String> {
    let rendered = render(spec);
    let unit_path = unit_file_path(&spec.name, paths);
    if let Some(parent) = unit_path.parent() {
        std::fs::create_dir_all(parent).map_err(|e| format!("mkdir {}: {e}", parent.display()))?;
    }
    let previous = std::fs::read_to_string(&unit_path).ok();
    let was_running = running_pid(&spec.name, runner).is_some();

    if previous.as_deref() == Some(rendered.as_str()) {
        return if !was_running {
            if start(&spec.name, runner) {
                Ok(SyncAction::Restarted)
            } else {
                Err(format!("start {} failed", spec.name))
            }
        } else if binary_newer_than_unit(&spec.bin_path, &unit_path) {
            // The live process predates the installed binary. Bounce it and
            // re-stamp the unit so staleness converges — without the stamp,
            // every later sync would bounce again forever.
            if platform::restart(&spec.name, runner) && write_atomic(&unit_path, &rendered).is_ok()
            {
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
    unload(&spec.name, runner);
    write_atomic(&unit_path, &rendered)?;
    if load_with_retries(&spec.name, &unit_path, runner) {
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
            let unit_path = unit_file_path(&spec.name, paths);
            let on_disk = std::fs::read_to_string(&unit_path).ok();
            let wanted = spec.bin_path.to_string_lossy().into_owned();
            let pid = running_pid(&spec.name, runner);
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

/// Every service that should be synced for an install rooted at
/// `bin_path`. Optional services whose binary the build did not produce
/// are left out: writing a unit that exec's a missing path only buys a
/// permanently-failing service the operator has to go and delete.
pub fn default_service_names(bin_path: &Path) -> Vec<&'static str> {
    let bin_dir = bin_path.parent().unwrap_or(Path::new("/"));
    SERVICES
        .iter()
        .filter(|def| !def.optional || bin_dir.join(def.bin_file).exists())
        .map(|def| def.name)
        .collect()
}

/// Resolve `names` against [`SERVICES`], preserving order. Unknown names land
/// as Err entries so callers can report them individually.
pub fn resolve_specs(bin_path: &Path, names: &[&str]) -> Vec<Result<ServiceSpec, String>> {
    let bin_dir = bin_path.parent().unwrap_or(Path::new("/"));
    let home_dir = super::home();
    let default_workspace = vak_config::paths::default_workspace();
    names
        .iter()
        .map(|name| {
            SERVICES
                .iter()
                .find(|def| def.name == *name)
                .map(|def| def.spec(bin_dir, &home_dir, &default_workspace))
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
                pid_text: Some(format!("com.vak.x = {{\n\tpid = {pid}\n}}")),
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
            name: def.name.to_string(),
            bin_path: bin_dir.join(def.bin_file),
            args: def.args.iter().map(|a| (*a).to_string()).collect(),
            log_path: log_dir.join(def.log_file),
            working_dir: PathBuf::from("/workspace"),
            home_dir: PathBuf::from("/Users/x"),
            path_env: "/usr/bin:/bin".into(),
            keep_alive: def.keep_alive,
            gui: def.gui,
        }
    }

    fn def_named(name: &str) -> &'static ServiceDef {
        SERVICES
            .iter()
            .find(|d| d.name == name)
            .expect("service must exist")
    }

    /// The desktop app is a GUI service, and the two ways it differs from
    /// the headless ones are both load-bearing:
    ///
    /// * `RunAtLoad` is the whole point — nothing else brings the menu-bar
    ///   icon back after a logout or reboot.
    /// * `KeepAlive` must be off. The tray's `Quit Vak` calls
    ///   `app.exit(0)`; with KeepAlive on, launchd relaunches a second
    ///   later and Quit visibly does not quit.
    #[test]
    fn desktop_unit_launches_at_login_hidden_and_is_not_kept_alive() {
        let def = def_named("com.vak.desktop");
        let spec = spec_for(def, Path::new("/opt/vak/bin"), Path::new("/l"));

        let plist = render_launchd_plist(&spec);
        assert!(plist.contains("<string>/opt/vak/bin/vak-desktop</string>"));
        assert!(
            plist.contains("<key>RunAtLoad</key>\n\t<true/>"),
            "the desktop service exists to come back at login: {plist}"
        );
        assert!(
            plist.contains("<key>KeepAlive</key>\n\t<false/>"),
            "KeepAlive would defeat the tray's Quit Vak: {plist}"
        );
        assert!(
            plist.contains("<string>--tray</string>"),
            "a login launch must not throw a window on screen: {plist}"
        );
        assert!(plist.contains("/l/desktop.log"));
        // Without this key launchd runs the app in the background gui/<uid>
        // domain: it starts, logs nothing, never checks in with
        // LaunchServices, gets no WindowServer connection, and therefore
        // draws no menu-bar icon — the exact failure this unit exists to
        // prevent.
        assert!(
            plist.contains("<key>LimitLoadToSessionType</key>\n\t<string>Aqua</string>"),
            "a GUI unit needs an Aqua session or it can draw no tray icon: {plist}"
        );

        let unit = render_systemd_unit(&spec);
        assert_eq!(def.systemd_unit(), "vak-desktop.service");
        assert!(
            unit.contains("ExecStart=/opt/vak/bin/vak-desktop --tray"),
            "{unit}"
        );
        assert!(
            unit.contains("Restart=no"),
            "the systemd analogue of KeepAlive=false: {unit}"
        );
        assert!(unit.contains("WantedBy=default.target"), "{unit}");
    }

    /// Headless services keep the resurrecting behaviour they have always
    /// had — the GUI exception must not leak into them.
    #[test]
    fn headless_services_are_still_kept_alive() {
        for name in ["com.vak.gateway", "com.vak.telegram"] {
            let spec = spec_for(def_named(name), Path::new("/b"), Path::new("/l"));
            assert!(render_launchd_plist(&spec).contains("<key>KeepAlive</key>\n\t<true/>"));
            assert!(render_systemd_unit(&spec).contains("Restart=always"));
            // Aqua-pinning a headless daemon would stop it loading in any
            // session without a logged-in GUI user.
            assert!(
                !render_launchd_plist(&spec).contains("LimitLoadToSessionType"),
                "{name} has no UI and must not be pinned to a GUI session"
            );
        }
    }

    /// Headless services use the canonical default workspace regardless of
    /// the caller's cwd. The desktop app picks its project in its own UI, so
    /// its service starts from the account home instead.
    #[test]
    fn workspace_scoped_services_use_the_canonical_default() {
        let home = Path::new("/Users/x");
        let default_workspace = Path::new("/Users/x/vak-home");
        for def in SERVICES {
            let spec = def.spec(Path::new("/b"), home, default_workspace);
            let expected = if def.workspace_scoped {
                default_workspace
            } else {
                home
            };
            assert_eq!(spec.working_dir, expected, "{}", def.name);
        }
    }

    /// An optional component the build never produced must not get a unit
    /// pointing at a path that does not exist.
    #[test]
    fn optional_services_are_skipped_when_their_binary_is_absent() {
        let dir = tempfile::tempdir().unwrap();
        let cli = dir.path().join("vak");
        std::fs::write(&cli, b"cli").unwrap();

        let without = default_service_names(&cli);
        assert!(without.contains(&"com.vak.gateway"));
        assert!(
            !without.contains(&"com.vak.desktop"),
            "no vak-desktop binary shipped, so no unit: {without:?}"
        );

        std::fs::write(dir.path().join("vak-desktop"), b"gui").unwrap();
        assert!(default_service_names(&cli).contains(&"com.vak.desktop"));
    }

    #[test]
    fn launchd_plist_renders_paths_args_and_zero_secrets() {
        let def = &SERVICES[0];
        let spec = spec_for(
            def,
            Path::new("/opt/vak/bin"),
            Path::new("/Users/x/.vak/logs"),
        );
        let plist = render_launchd_plist(&spec);
        assert!(plist.contains("/opt/vak/bin/vak"));
        for a in def.args {
            assert!(plist.contains(a), "missing arg {a}");
        }
        assert!(plist.contains("/Users/x/.vak/logs/gateway.log"));
        assert!(plist.contains("KeepAlive"));
        assert!(plist.contains("RunAtLoad"));
        assert!(plist.contains("<key>HOME</key>"));
        assert!(plist.contains("<string>/Users/x</string>"));
        assert!(plist.contains("<key>PATH</key>"));
        assert!(plist.contains("/usr/bin:/bin"));
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
        let specs: Vec<ServiceSpec> =
            resolve_specs(Path::new("/Applications/vak.app/Contents/MacOS/vak"), &[])
                .into_iter()
                .flatten()
                .collect();
        for spec in specs {
            let log = spec.log_path.to_string_lossy();
            let bin = spec.bin_path.to_string_lossy();
            assert!(
                !log.contains("/.vak/"),
                "log path must use the canonical logs dir, got {log}"
            );
            assert!(
                !bin.contains("/target/"),
                "binaries must come from the managed install, got {bin}"
            );
            assert!(
                bin.starts_with("/Applications/vak.app/"),
                "binaries must live inside the installed bundle, got {bin}"
            );
            assert!(
                log.contains("Library/Logs/vak"),
                "logs must land in Library/Logs on macOS, got {log}"
            );
        }
    }

    #[test]
    fn systemd_unit_renders_restart_and_exec() {
        let def = &SERVICES[0];
        let spec = spec_for(def, Path::new("/opt/vak/bin"), Path::new("/h/.vak"));
        let unit = render_systemd_unit(&spec);
        assert!(
            unit.contains("ExecStart="),
            "unit missing ExecStart: {unit}"
        );
        assert!(unit.contains("/opt/vak/bin/vak"), "{unit}");
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
        assert!(unit.contains("/opt/vak/bin/vak"));

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
        let bin = dir.path().join("vak");
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
        let bin = dir.path().join("vak");
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
        let out = resolve_specs(Path::new("/b"), &["com.vak.gateway", "nope"]);
        assert!(out[0].is_ok());
        assert!(out[1].as_ref().err().unwrap().contains("unknown service"));

        let synced = services_sync(Path::new("/b"), &["nope"], &Paths::default(), &Fake::new());
        assert!(matches!(synced[0].action, SyncAction::Failed(_)));
    }

    fn write_bots_json(data_home: &Path, bots: &[(&str, &str)]) {
        let dir = data_home.join("gateway");
        std::fs::create_dir_all(&dir).unwrap();
        let entries: Vec<String> = bots
            .iter()
            .map(|(id, surface)| format!(r#"{{"id":"{id}","surface":"{surface}"}}"#))
            .collect();
        std::fs::write(
            dir.join("bots.json"),
            format!(r#"{{"schema":1,"bots":[{}]}}"#, entries.join(",")),
        )
        .unwrap();
    }

    /// One unit per configured bot, named and argued so it resolves that
    /// bot's own token — the fix for the multi-bot regression where a
    /// single static `com.vak.telegram` unit with no `--bot-id` fell back
    /// to the dead legacy `TELEGRAM_BOT_TOKEN` slot.
    #[test]
    fn bot_service_specs_one_per_configured_bot_with_bot_id_arg() {
        let dir = tempfile::tempdir().unwrap();
        write_bots_json(
            dir.path(),
            &[("VakBot", "telegram"), ("VakyarthaBot", "telegram")],
        );
        let specs = bot_service_specs(
            Path::new("/opt/vak/bin"),
            Path::new("/Users/x"),
            Path::new("/workspace"),
            dir.path(),
            "http://127.0.0.1:8901",
        );
        assert_eq!(specs.len(), 2);
        let names: Vec<&str> = specs.iter().map(|s| s.name.as_str()).collect();
        assert!(names.contains(&"com.vak.telegram-VakBot"));
        assert!(names.contains(&"com.vak.telegram-VakyarthaBot"));
        let vakbot = specs
            .iter()
            .find(|s| s.name == "com.vak.telegram-VakBot")
            .unwrap();
        assert_eq!(
            vakbot.args,
            vec![
                "telegram",
                "--server",
                "http://127.0.0.1:8901",
                "--bot-id",
                "VakBot"
            ],
        );
        assert!(vakbot.keep_alive);
        assert_eq!(vakbot.working_dir, Path::new("/workspace"));
        // Every rendered unit must still carry zero secrets — the token
        // lives only in the env var the bridge process reads for itself.
        let plist = render_launchd_plist(vakbot);
        for secret in ["TOKEN", "SECRET", "BOT_TOKEN"] {
            assert!(!plist.contains(secret), "unit must not contain {secret}");
        }
    }

    /// A surface the CLI has no bridge for (or a bots.json typo) must not
    /// produce a unit that can never run.
    #[test]
    fn bot_service_specs_skips_unknown_surfaces_and_missing_file() {
        let dir = tempfile::tempdir().unwrap();
        write_bots_json(dir.path(), &[("Weird", "carrier-pigeon")]);
        let specs = bot_service_specs(
            Path::new("/b"),
            Path::new("/h"),
            Path::new("/w"),
            dir.path(),
            "http://127.0.0.1:8901",
        );
        assert!(specs.is_empty());

        let missing = tempfile::tempdir().unwrap();
        let specs = bot_service_specs(
            Path::new("/b"),
            Path::new("/h"),
            Path::new("/w"),
            missing.path(),
            "http://127.0.0.1:8901",
        );
        assert!(
            specs.is_empty(),
            "no bots.json yet must mean no units, not an error"
        );
    }

    /// Deleting (or renaming) a bot must take its bridge process down too —
    /// otherwise it keeps running forever against a token env var nothing
    /// references any more.
    #[test]
    fn sync_bots_prunes_units_for_deleted_bots() {
        let (_d, paths) = tmp_paths("bots-prune");
        let data = tempfile::tempdir().unwrap();
        let fake = Fake::with_pid(123);

        write_bots_json(
            data.path(),
            &[("VakBot", "telegram"), ("Second", "telegram")],
        );
        let first = sync_bots(
            Path::new("/opt/vak/bin/vak"),
            data.path(),
            "http://127.0.0.1:8901",
            &paths,
            &fake,
        );
        assert_eq!(first.len(), 2);
        assert!(unit_file_path("com.vak.telegram-VakBot", &paths).exists());
        assert!(unit_file_path("com.vak.telegram-Second", &paths).exists());

        // User deletes "Second" from the admin console.
        write_bots_json(data.path(), &[("VakBot", "telegram")]);
        let second = sync_bots(
            Path::new("/opt/vak/bin/vak"),
            data.path(),
            "http://127.0.0.1:8901",
            &paths,
            &fake,
        );
        assert_eq!(
            second.len(),
            1,
            "only the surviving bot should be (re)synced"
        );
        assert!(
            unit_file_path("com.vak.telegram-VakBot", &paths).exists(),
            "surviving bot's unit must be untouched"
        );
        assert!(
            !unit_file_path("com.vak.telegram-Second", &paths).exists(),
            "deleted bot's unit must be removed, not left running forever"
        );
    }

    #[test]
    fn bot_service_name_sanitizes_and_namespaces_by_surface() {
        assert_eq!(
            bot_service_name("telegram", "VakBot"),
            "com.vak.telegram-VakBot"
        );
        assert_eq!(
            bot_service_name("discord", "weird id!"),
            "com.vak.discord-weird_id_"
        );
    }

    #[test]
    fn has_configured_bots_reflects_bots_json_by_surface() {
        let data = tempfile::tempdir().unwrap();
        assert!(!has_configured_bots(data.path(), "telegram"));

        write_bots_json(data.path(), &[("VakBot", "telegram")]);
        assert!(has_configured_bots(data.path(), "telegram"));
        assert!(!has_configured_bots(data.path(), "discord"));
    }

    #[test]
    fn uninstall_bot_units_removes_every_bot_unit_regardless_of_bots_json() {
        let (_d, paths) = tmp_paths("bots-uninstall");
        let data = tempfile::tempdir().unwrap();
        let fake = Fake::with_pid(123);

        write_bots_json(data.path(), &[("VakBot", "telegram"), ("Ops", "discord")]);
        sync_bots(
            Path::new("/opt/vak/bin/vak"),
            data.path(),
            "http://127.0.0.1:8901",
            &paths,
            &fake,
        );
        assert!(unit_file_path("com.vak.telegram-VakBot", &paths).exists());
        assert!(unit_file_path("com.vak.discord-Ops", &paths).exists());

        // Uninstall must remove every bot unit even though bots.json still
        // lists them — an uninstall wants "wanted = nothing", not a diff
        // against the still-present config file.
        uninstall_bot_units(&paths, &fake);
        assert!(!unit_file_path("com.vak.telegram-VakBot", &paths).exists());
        assert!(!unit_file_path("com.vak.discord-Ops", &paths).exists());
    }
}
