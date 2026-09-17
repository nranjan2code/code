//! vak-ops: one service-control layer shared by the tray, TUI and desktop.
//!
//! Wraps platform service managers (launchd on macOS, systemd --user on
//! Linux) behind a tiny API so every surface shows the same truth:
//!
//! ```ignore
//! let st = vak_ops::status(Service::Gateway, &OpsConfig::detect());
//! ```
//!
//! Design rules (docs/design/28-operations.md):
//! - No daemon of its own: it shells out to the platform manager, which is
//!   already keeping the services alive.
//! - Every command is idempotent from the user's point of view — start on a
//!   running service is a no-op, stop on a stopped one too.
//! - Health checks are plain HTTP against the gateway's /health.

use std::fmt;
use std::path::PathBuf;
use std::process::Command;
#[cfg(target_os = "macos")]
use std::sync::OnceLock;

pub mod services;
pub use services::{
    CommandRunner, Paths, SERVICES, ServiceDef, ServiceRow, ServiceSpec, SyncAction, SyncOutcome,
    SystemRunner, bot_service_name, bot_service_specs, configured_bot_service_names,
    configured_bot_service_names_all, default_service_names, is_autostart_configured,
    is_desktop_autostart_persisted, is_service_autostart_enabled, persist_desktop_autostart,
    render_launchd_plist, render_systemd_unit, resolve_specs, restart_bot_unit, services_status,
    services_sync, services_uninstall, set_service_autostart, status_specs, sync_bots, sync_specs,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Service {
    Gateway,
    /// Every chat bridge, across every transport.
    ///
    /// Deliberately not one variant per surface. A bridge unit belongs to
    /// a *bot* (AGENTS.md invariant 23) and a bot names its own transport,
    /// so the set of bridges is data in `bots.json`, not a shape in this
    /// enum. The variant this replaces was `Telegram` — one transport with
    /// a named service while Discord and Slack had none, which made
    /// Telegram read as the real one and the others as extras. They are
    /// peers; per-transport detail comes from the bot list.
    Bridges,
}

impl Service {
    pub fn label(self) -> &'static str {
        match self {
            Service::Gateway => "Gateway",
            Service::Bridges => "Chat bridges",
        }
    }

    /// The single unit this service is, where it is one.
    ///
    /// `Telegram` is **not** a unit: a bot owns its own bridge unit
    /// (`com.vak.telegram-<id>`), and the bot-id-less `com.vak.telegram`
    /// that used to sit here could only ever describe one bot per
    /// transport (AGENTS.md invariant 23). `Telegram` is an aggregate view
    /// over whatever per-bot units are configured, so it resolves to a
    /// list rather than a name — see [`Service::unit_labels`].
    pub fn launchd_label(self) -> Option<&'static str> {
        match self {
            Service::Gateway => Some("com.vak.gateway"),
            Service::Bridges => None,
        }
    }

    pub fn systemd_unit(self) -> Option<&'static str> {
        match self {
            Service::Gateway => Some("vak-gateway.service"),
            Service::Bridges => None,
        }
    }

    /// Every unit this service covers right now. One for the gateway; one
    /// per configured bot for `Telegram`, which is why this is resolved
    /// from `bots.json` on each call rather than being a constant.
    pub fn unit_labels(self) -> Vec<String> {
        match self {
            Service::Gateway => self
                .launchd_label()
                .map(|l| vec![l.to_string()])
                .unwrap_or_default(),
            Service::Bridges => configured_bot_service_names_all(&vak_config::paths::data_home()),
        }
    }

    /// Systemd counterpart of [`Service::unit_labels`].
    pub fn systemd_units(self) -> Vec<String> {
        match self {
            Service::Gateway => self
                .systemd_unit()
                .map(|u| vec![u.to_string()])
                .unwrap_or_default(),
            Service::Bridges => self
                .unit_labels()
                .into_iter()
                .map(|name| format!("{name}.service"))
                .collect(),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum State {
    Running,
    Stopped,
    NotInstalled,
    Unknown,
}

impl fmt::Display for State {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            State::Running => "running",
            State::Stopped => "stopped",
            State::NotInstalled => "not installed",
            State::Unknown => "unknown",
        })
    }
}

/// Where the user's service manager lives.
#[derive(Debug, Clone)]
pub struct OpsConfig {
    pub port: u16,
}

impl Default for OpsConfig {
    fn default() -> Self {
        OpsConfig { port: 8901 }
    }
}

impl OpsConfig {
    pub fn detect() -> Self {
        OpsConfig {
            port: std::env::var("VAK_PORT")
                .ok()
                .or_else(|| vak_config::get_var("VAK_PORT"))
                .and_then(|p| p.parse().ok())
                .unwrap_or(8901),
        }
    }

    pub fn base_url(&self) -> String {
        format!("http://127.0.0.1:{}", self.port)
    }
}

fn run(cmd: &mut Command) -> bool {
    cmd.stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .map(|s| s.success())
        .unwrap_or(false)
}

pub(crate) fn home() -> PathBuf {
    std::env::var_os("HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("/"))
}

/// Register the service with the platform manager and start it. On macOS
/// this requires the plist produced by
/// `scripts/install_gateway_service.sh`; on Linux it enables the unit.
pub fn install(service: Service, #[allow(unused_variables)] cfg: &OpsConfig) -> Result<(), String> {
    #[cfg(target_os = "macos")]
    {
        let labels = service.unit_labels();
        if labels.is_empty() {
            return Err(format!(
                "{} has no units configured — create a bot, then run `vak self services-sync`",
                service.label()
            ));
        }
        for label in &labels {
            let plist = home()
                .join("Library/LaunchAgents")
                .join(format!("{label}.plist"));
            if !plist.is_file() {
                return Err(format!(
                    "no plist at {} — run `vak self services-sync`",
                    plist.display()
                ));
            }
            // Already bootstrapped is fine.
            run(Command::new("launchctl").args([
                "bootstrap",
                &format!("gui/{}", uid()),
                &plist.display().to_string(),
            ]));
        }
        start(service, cfg);
        Ok(())
    }
    #[cfg(not(target_os = "macos"))]
    {
        let units = service.systemd_units();
        if units.is_empty() {
            return Err(format!(
                "{} has no units configured — create a bot, then run `vak self services-sync`",
                service.label()
            ));
        }
        for unit in &units {
            if !run(Command::new("systemctl").args(["--user", "enable", "--now", unit])) {
                return Err(format!(
                    "unit {unit} not found — run `vak self services-sync`"
                ));
            }
        }
        Ok(())
    }
}

/// Stop and deregister. Files under ~/.vak are never touched.
pub fn uninstall(service: Service, cfg: &OpsConfig) -> Result<(), String> {
    stop(service, cfg);
    #[cfg(target_os = "macos")]
    {
        for label in service.unit_labels() {
            let plist = home()
                .join("Library/LaunchAgents")
                .join(format!("{label}.plist"));
            // A unit that is already gone is the desired end state.
            match std::fs::remove_file(&plist) {
                Ok(()) => {}
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
                Err(e) => return Err(format!("remove plist: {e}")),
            }
        }
    }
    #[cfg(not(target_os = "macos"))]
    {
        for unit in service.systemd_units() {
            run(Command::new("systemctl").args(["--user", "disable", &unit]));
        }
    }
    Ok(())
}

/// Platform state probe. Falls back to HTTP liveness when the manager has
/// no opinion (service not installed as such).
pub fn status(service: Service, cfg: &OpsConfig) -> State {
    let managed = manager_state(service);
    if managed == State::Running {
        return managed;
    }
    if service == Service::Gateway && managed == State::NotInstalled && health_ok(cfg) {
        return State::Running;
    }
    managed
}

fn manager_state(service: Service) -> State {
    #[cfg(target_os = "macos")]
    {
        let labels = service.unit_labels();
        if labels.is_empty() {
            // No units configured is "nothing installed", not "stopped":
            // a transport with no bots has nothing that could be running.
            return State::NotInstalled;
        }
        let states: Vec<State> = labels.iter().map(|label| launchd_state(label)).collect();
        if states.iter().all(|state| *state == State::Running) {
            State::Running
        } else if states.iter().any(|state| *state != State::NotInstalled) {
            State::Stopped
        } else {
            State::NotInstalled
        }
    }
    #[cfg(not(target_os = "macos"))]
    {
        let units = service.systemd_units();
        if units.is_empty() {
            return State::NotInstalled;
        }
        let active = units.iter().all(|unit| {
            Command::new("systemctl")
                .args(["--user", "is-active", "--quiet", unit])
                .status()
                .map(|s| s.success())
                .unwrap_or(false)
        });
        if active {
            State::Running
        } else {
            let enabled = units.iter().all(|unit| {
                Command::new("systemctl")
                    .args(["--user", "is-enabled", unit])
                    .stdout(std::process::Stdio::null())
                    .stderr(std::process::Stdio::null())
                    .status()
                    .map(|s| s.success())
                    .unwrap_or(false)
            });
            if enabled {
                State::Stopped
            } else {
                State::NotInstalled
            }
        }
    }
}

#[cfg(target_os = "macos")]
fn launchd_state(label: &str) -> State {
    let output = Command::new("launchctl")
        .args(["print", &format!("gui/{}/{}", uid(), label)])
        .output();
    let Ok(output) = output else {
        return State::Unknown;
    };
    if !output.status.success() {
        return State::NotInstalled;
    }
    parse_launchd_state(&String::from_utf8_lossy(&output.stdout))
}

#[cfg(target_os = "macos")]
fn parse_launchd_state(text: &str) -> State {
    if text.lines().any(|line| {
        line.trim()
            .strip_prefix("pid = ")
            .and_then(|pid| pid.parse::<u32>().ok())
            .is_some_and(|pid| pid > 0)
    }) {
        State::Running
    } else {
        State::Stopped
    }
}

#[cfg(all(test, target_os = "macos"))]
mod tests {
    use super::{State, parse_launchd_state};

    #[test]
    fn launchd_registration_without_pid_is_stopped() {
        assert_eq!(
            parse_launchd_state("state = waiting\nruns = 3\n"),
            State::Stopped
        );
    }

    #[test]
    fn launchd_live_pid_is_running() {
        assert_eq!(
            parse_launchd_state("state = running\npid = 123\n"),
            State::Running
        );
    }
}

/// True when the gateway answers /health.
pub fn health_ok(cfg: &OpsConfig) -> bool {
    // Blocking call by design: callers are UI threads that want a quick,
    // bounded answer.
    let url = format!("{}/health", cfg.base_url());
    reqwest::blocking::Client::builder()
        .timeout(std::time::Duration::from_secs(2))
        .build()
        .and_then(|c| c.get(&url).send().and_then(|r| r.error_for_status()))
        .is_ok()
}

pub fn start(service: Service, _cfg: &OpsConfig) -> bool {
    #[cfg(target_os = "macos")]
    {
        let labels = service.unit_labels();
        !labels.is_empty()
            && labels.into_iter().all(|label| {
                run(Command::new("launchctl").args([
                    "kickstart",
                    "-k",
                    &format!("gui/{}/{label}", uid()),
                ]))
            })
    }
    #[cfg(not(target_os = "macos"))]
    {
        let units = service.systemd_units();
        !units.is_empty()
            && units
                .into_iter()
                .all(|unit| run(Command::new("systemctl").args(["--user", "start", &unit])))
    }
}

pub fn stop(service: Service, _cfg: &OpsConfig) -> bool {
    #[cfg(target_os = "macos")]
    {
        let labels = service.unit_labels();
        !labels.is_empty()
            && labels.into_iter().all(|label| {
                run(Command::new("launchctl").args(["bootout", &format!("gui/{}/{label}", uid())]))
            })
    }
    #[cfg(not(target_os = "macos"))]
    {
        let units = service.systemd_units();
        !units.is_empty()
            && units
                .into_iter()
                .all(|unit| run(Command::new("systemctl").args(["--user", "stop", &unit])))
    }
}

pub fn restart(service: Service, cfg: &OpsConfig) -> bool {
    // bootout+bootstrap would lose KeepAlive semantics; kickstart -k IS the
    // restart primitive on launchd.
    start(service, cfg)
}

pub fn open_log(service: Service) {
    #[cfg(target_os = "macos")]
    {
        // Canonical logs home (doc 32): ~/Library/Logs/vak —
        // Console.app-visible. Overridden homes keep self-contained logs.
        let log = vak_config::paths::logs_dir().join(match service {
            Service::Gateway => "gateway.log",
            Service::Bridges => "bridges.log",
        });
        if let Some(parent) = log.parent() {
            std::fs::create_dir_all(parent).ok();
        }
        if !log.exists() {
            std::fs::write(&log, "").ok();
        }
        run(Command::new("open").arg("-t").arg(&log));
    }
    #[cfg(not(target_os = "macos"))]
    {
        let unit = match service {
            Service::Gateway => "vak-gateway",
            Service::Bridges => "vak-bridges",
        };
        run(Command::new("sh").arg("-c").arg(format!(
            "journalctl --user -u {unit} -n 200 --no-pager 2>/dev/null || true"
        )));
    }
}

#[cfg(target_os = "macos")]
fn uid() -> String {
    static UID: OnceLock<String> = OnceLock::new();
    UID.get_or_init(|| {
        Command::new("id")
            .arg("-u")
            .output()
            .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
            .unwrap_or_else(|_| "501".into())
    })
    .clone()
}
