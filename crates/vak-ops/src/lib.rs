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
use std::sync::OnceLock;

pub mod services;
pub use services::{
    CommandRunner, Paths, SERVICES, ServiceDef, ServiceRow, ServiceSpec, SyncAction, SyncOutcome,
    SystemRunner, bot_service_name, bot_service_specs, configured_bot_service_names,
    default_service_names, render_launchd_plist, render_systemd_unit, resolve_specs,
    restart_bot_unit, services_status, services_sync, services_uninstall, status_specs, sync_bots,
    sync_specs,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Service {
    Gateway,
    Telegram,
}

impl Service {
    pub fn label(self) -> &'static str {
        match self {
            Service::Gateway => "Gateway",
            Service::Telegram => "Telegram bridge",
        }
    }

    pub fn launchd_label(self) -> &'static str {
        match self {
            Service::Gateway => "com.vak.gateway",
            Service::Telegram => "com.vak.telegram",
        }
    }

    pub fn systemd_unit(self) -> &'static str {
        match self {
            Service::Gateway => "vak-gateway.service",
            Service::Telegram => "vak-telegram.service",
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
pub fn install(service: Service, cfg: &OpsConfig) -> Result<(), String> {
    #[cfg(target_os = "macos")]
    {
        let plist = home()
            .join("Library/LaunchAgents")
            .join(format!("{}.plist", service.launchd_label()));
        if !plist.is_file() {
            return Err(format!(
                "no plist at {} — run scripts/install_gateway_service.sh once",
                plist.display()
            ));
        }
        if !run(Command::new("launchctl").args([
            "bootstrap",
            &format!("gui/{}", uid()),
            &plist.display().to_string(),
        ])) {
            // Already bootstrapped is fine.
        }
        start(service, cfg);
        Ok(())
    }
    #[cfg(not(target_os = "macos"))]
    {
        if !run(Command::new("systemctl").args([
            "--user",
            "enable",
            "--now",
            service.systemd_unit(),
        ])) {
            return Err(format!(
                "unit {} not found — install via scripts/install_gateway_service.sh",
                service.systemd_unit()
            ));
        }
        Ok(())
    }
}

/// Stop and deregister. Files under ~/.vak are never touched.
pub fn uninstall(service: Service, cfg: &OpsConfig) -> Result<(), String> {
    stop(service, cfg);
    #[cfg(target_os = "macos")]
    {
        let plist = home()
            .join("Library/LaunchAgents")
            .join(format!("{}.plist", service.launchd_label()));
        std::fs::remove_file(&plist).map_err(|e| format!("remove plist: {e}"))?;
    }
    #[cfg(not(target_os = "macos"))]
    {
        run(Command::new("systemctl").args(["--user", "disable", service.systemd_unit()]));
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
        let labels = if service == Service::Telegram {
            let names = configured_bot_service_names(&vak_config::paths::data_home(), "telegram");
            if names.is_empty() {
                vec![service.launchd_label().to_string()]
            } else {
                names
            }
        } else {
            vec![service.launchd_label().to_string()]
        };
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
        let active = Command::new("systemctl")
            .args(["--user", "is-active", "--quiet", service.systemd_unit()])
            .status()
            .map(|s| s.success())
            .unwrap_or(false);
        if active {
            State::Running
        } else {
            let enabled = Command::new("systemctl")
                .args(["--user", "is-enabled", service.systemd_unit()])
                .stdout(std::process::Stdio::null())
                .stderr(std::process::Stdio::null())
                .status()
                .map(|s| s.success())
                .unwrap_or(false);
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
        run(Command::new("launchctl").args([
            "kickstart",
            "-k",
            &format!("gui/{}/{}", uid(), service.launchd_label()),
        ]))
    }
    #[cfg(not(target_os = "macos"))]
    {
        run(Command::new("systemctl").args(["--user", "start", service.systemd_unit()]))
    }
}

pub fn stop(service: Service, _cfg: &OpsConfig) -> bool {
    #[cfg(target_os = "macos")]
    {
        run(Command::new("launchctl").args([
            "bootout",
            &format!("gui/{}/{}", uid(), service.launchd_label()),
        ]))
    }
    #[cfg(not(target_os = "macos"))]
    {
        run(Command::new("systemctl").args(["--user", "stop", service.systemd_unit()]))
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
            Service::Telegram => "telegram.log",
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
            Service::Telegram => "vak-telegram",
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
