//! vakcoder-tray: a native menu-bar controller for the vakcoder gateway.
//!
//! One glance answers "is my agent alive?" — the icon is green when both
//! service runs, red when it is down, grey when it is not installed. The menu
//! starts/stops/restarts,
//! installs/uninstalls, opens logs, and toggles a watchdog that restarts a
//! crashed service automatically and posts a system notification when it
//! does. See docs/design/28-operations.md.

// GUI bootstrap: every setup call here is infallible in practice, and a
// controller that cannot start should be loud about it.
#![allow(clippy::expect_used)]

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use tray_icon::menu::{CheckMenuItem, Menu, MenuEvent, MenuItem, PredefinedMenuItem};
use tray_icon::{Icon, TrayIcon, TrayIconBuilder};
use winit::application::ApplicationHandler;
use winit::event::WindowEvent;
use winit::event_loop::{ActiveEventLoop, EventLoop};

#[derive(Debug, Clone)]
enum TrayEvent {
    Refresh([vak_ops::State; 1]),
    Command(u32),
}

const SVC_COUNT: usize = 1;
const GATEWAY: usize = 0;

fn service(idx: usize) -> vak_ops::Service {
    let _ = idx;
    vak_ops::Service::Gateway
}

/// 16x16 RGBA dot used as the tray glyph; colour encodes aggregate state.
#[allow(clippy::expect_used)] // infallible: fixed non-zero dimensions
fn icon_dot(rgb: [u8; 3]) -> Icon {
    const S: usize = 16;
    let mut rgba = Vec::with_capacity(S * S * 4);
    let c = (S as f32 - 1.0) / 2.0;
    for y in 0..S {
        for x in 0..S {
            let d = ((x as f32 - c).powi(2) + (y as f32 - c).powi(2)).sqrt();
            let alpha = if d <= 6.5 {
                255u8
            } else if d <= 7.5 {
                140
            } else {
                0
            };
            rgba.extend_from_slice(&[rgb[0], rgb[1], rgb[2], alpha]);
        }
    }
    // A fixed-size buffer can only fail on zero width/height — neither
    // applies here.
    Icon::from_rgba(rgba, S as u32, S as u32).expect("static icon")
}

struct Ui {
    tray: TrayIcon,
    watchdog: Arc<AtomicBool>,
    states: [vak_ops::State; 1],
}

impl ApplicationHandler<TrayEvent> for Ui {
    fn resumed(&mut self, _loop: &ActiveEventLoop) {}

    fn about_to_wait(&mut self, _loop: &ActiveEventLoop) {}

    fn user_event(&mut self, loop_handle: &ActiveEventLoop, event: TrayEvent) {
        match event {
            TrayEvent::Refresh(states) => {
                let _ = loop_handle;
                self.states = states;
                self.tray.set_menu(Some(Box::new(build_menu(
                    &states,
                    self.watchdog.load(Ordering::SeqCst),
                ))));
                let rgb = match states[GATEWAY] {
                    vak_ops::State::Running => [76u8, 175, 80],
                    vak_ops::State::NotInstalled => [158u8, 158, 158],
                    _ => [244u8, 67, 54],
                };
                let _ = self.tray.set_icon(Some(icon_dot(rgb)));
            }
            TrayEvent::Command(id) => self.handle_command(id),
        }
    }

    fn window_event(&mut self, _: &ActiveEventLoop, _: winit::window::WindowId, _: WindowEvent) {}
}

impl Ui {
    fn handle_command(&mut self, id: u32) {
        // Commands are encoded as (slot << 8) | action; see build_menu.
        let slot = (id >> 8) as usize;
        let action = id & 0xff;
        let cfg = vak_ops::OpsConfig::detect();
        match action {
            ACT_START => {
                let _ = vak_ops::start(service(slot), &cfg);
            }
            ACT_STOP => {
                let _ = vak_ops::stop(service(slot), &cfg);
            }
            ACT_RESTART => {
                vak_ops::restart(service(slot), &cfg);
            }
            ACT_INSTALL => {
                if let Err(e) = vak_ops::install(service(slot), &cfg) {
                    notify("vakcoder", &e);
                }
            }
            ACT_UNINSTALL => {
                let _ = vak_ops::uninstall(service(slot), &cfg);
            }
            ACT_LOG => vak_ops::open_log(service(slot)),
            ACT_WATCHDOG_TOGGLE => {
                let newval = !self.watchdog.load(Ordering::SeqCst);
                self.watchdog.store(newval, Ordering::SeqCst);
                persist_watchdog(newval);
            }
            ACT_OPEN_DESKTOP => open_desktop(),
            ACT_OPEN_ADMIN => open_admin_console(),
            ACT_QUIT => std::process::exit(0),
            _ => {}
        }
    }
}

const ACT_START: u32 = 1;
const ACT_STOP: u32 = 2;
const ACT_RESTART: u32 = 3;
const ACT_INSTALL: u32 = 4;
const ACT_UNINSTALL: u32 = 5;
const ACT_LOG: u32 = 6;
const ACT_WATCHDOG_TOGGLE: u32 = 7;
const ACT_QUIT: u32 = 8;
const ACT_OPEN_DESKTOP: u32 = 9;
const ACT_OPEN_ADMIN: u32 = 10;

fn persist_watchdog(on: bool) {
    let path = home().join("tray.json");
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).ok();
    }
    std::fs::write(path, serde_json::json!({ "auto_restart": on }).to_string()).ok();
}

fn load_watchdog() -> bool {
    std::fs::read_to_string(home().join("tray.json"))
        .ok()
        .and_then(|s| serde_json::from_str::<serde_json::Value>(&s).ok())
        .and_then(|v| v["auto_restart"].as_bool())
        .unwrap_or(true)
}

fn home() -> std::path::PathBuf {
    // Canonical data home (doc 32) — never hand-roll HOME/.vakcoder.
    vak_config::paths::data_home()
}

fn notify(title: &str, body: &str) {
    let _ = notify_rust::Notification::new()
        .summary(title)
        .body(body)
        .show();
}

fn main() {
    let watchdog = Arc::new(AtomicBool::new(load_watchdog()));

    let event_loop: EventLoop<TrayEvent> =
        EventLoop::with_user_event().build().expect("event loop");

    let cfg = vak_ops::OpsConfig::detect();
    let proxy = event_loop.create_proxy();

    // Watchdog + poller thread: computes truth every 3 s, pushes a refresh
    // to the UI, and restarts crashed services when enabled.
    {
        let watchdog = watchdog.clone();
        let proxy = proxy.clone();
        std::thread::spawn(move || {
            let mut last_running = [false; SVC_COUNT];
            let mut last_down_notify = std::time::Instant::now();
            loop {
                let mut states = [vak_ops::State::Unknown; SVC_COUNT];
                for (i, st) in states.iter_mut().enumerate() {
                    *st = vak_ops::status(service(i), &cfg);
                }
                // Watchdog: a service that was running and silently died
                // gets restarted at most once per minute.
                for i in 0..SVC_COUNT {
                    let running = states[i] == vak_ops::State::Running;
                    if watchdog.load(Ordering::SeqCst)
                        && last_running[i]
                        && !running
                        && last_down_notify.elapsed() > Duration::from_secs(60)
                    {
                        let _ = vak_ops::start(service(i), &cfg);
                        notify(
                            "vakcoder watchdog",
                            &format!("{} went down — restarting", service(i).label()),
                        );
                        last_down_notify = std::time::Instant::now();
                        states[i] = vak_ops::status(service(i), &cfg);
                    }
                    last_running[i] = running;
                }
                let _ = proxy.send_event(TrayEvent::Refresh(states));
                std::thread::sleep(Duration::from_secs(3));
            }
        });
    }

    // Build the initial tray before entering the loop.
    let menu = build_menu(&states_now(), watchdog.load(Ordering::SeqCst));
    let tray = TrayIconBuilder::new()
        .with_menu(Box::new(menu))
        .with_tooltip("vakcoder services")
        .with_icon(icon_dot([158, 158, 158]))
        .build()
        .expect("tray built");

    let mut ui = Ui {
        tray,
        watchdog,
        states: [vak_ops::State::Unknown; 1],
    };

    // Menu clicks arrive on a global channel; forward them as user events so
    // everything runs on the UI thread.
    let proxy2 = event_loop.create_proxy();
    std::thread::spawn(move || {
        let rx = MenuEvent::receiver();
        for ev in rx {
            if let Ok(raw) = ev.id.0.parse::<u32>() {
                let _ = proxy2.send_event(TrayEvent::Command(raw));
            }
        }
    });

    event_loop.run_app(&mut ui).expect("event loop ran");
}

// ---- menu construction ------------------------------------------------------

fn states_now() -> [vak_ops::State; 1] {
    let cfg = vak_ops::OpsConfig::detect();
    [vak_ops::status(vak_ops::Service::Gateway, &cfg)]
}

/// Launch the desktop app: prefer the installed bundle so Dock behaviour is
/// normal, fall back to a sibling binary for dev checkouts.
fn open_desktop() {
    #[cfg(target_os = "macos")]
    {
        let ok = std::process::Command::new("open")
            .arg("-a")
            .arg("VakCoder")
            .status()
            .map(|s| s.success())
            .unwrap_or(false);
        if ok {
            return;
        }
    }
    if let Some(exe) = std::env::current_exe()
        .ok()
        .and_then(|p| p.parent().map(|d| d.join("vak-desktop")))
    {
        let _ = std::process::Command::new(exe).spawn();
    }
}

/// Open the web admin console. When the gateway runtime file is fresh, the
/// URL carries `#token=…`: the fragment never reaches the network and the
/// console trades it for an HttpOnly cookie, then scrubs itself — one click
/// for a non-technical user, no plaintext token in any server log.
fn open_admin_console() {
    let home = vak_config::paths::data_home();
    let raw =
        std::fs::read_to_string(home.join("runtime").join("gateway.json")).unwrap_or_default();
    open_url(&admin_url(&raw, &|pid| {
        std::process::Command::new("kill")
            .args(["-0", &pid.to_string()])
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .status()
            .map(|s| s.success())
            .unwrap_or(false)
    }));
}

/// Builds the console URL from the gateway runtime file contents. Falls
/// back to the tokenless login page unless pid liveness confirms the file.
fn admin_url(runtime_json: &str, pid_alive: &dyn Fn(i32) -> bool) -> String {
    let fallback = "http://127.0.0.1:8901/admin".to_string();
    let Ok(v) = serde_json::from_str::<serde_json::Value>(runtime_json) else {
        return fallback;
    };
    let pid = v["pid"].as_i64().unwrap_or(0);
    if pid <= 0 || !pid_alive(pid as i32) {
        return fallback;
    }
    match (v["addr"].as_str(), v["token"].as_str()) {
        (Some(addr), Some(token)) => {
            format!("http://{addr}/admin#token={}", urlencode(token))
        }
        _ => fallback,
    }
}

fn urlencode(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for b in s.bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                out.push(b as char)
            }
            _ => out.push_str(&format!("%{b:02X}")),
        }
    }
    out
}

fn open_url(url: &str) {
    #[cfg(target_os = "macos")]
    let _ = std::process::Command::new("open").arg(url).spawn();
    #[cfg(target_os = "linux")]
    let _ = std::process::Command::new("xdg-open").arg(url).spawn();
}

fn build_menu(states: &[vak_ops::State; 1], watchdog_on: bool) -> Menu {
    let menu = Menu::new();
    let open_app = MenuItem::with_id(ACT_OPEN_DESKTOP.to_string(), "Open VakCoder", true, None);
    let _ = menu.append(&open_app);
    let admin = MenuItem::with_id(
        ACT_OPEN_ADMIN.to_string(),
        "Open Admin Console…",
        true,
        None,
    );
    let _ = menu.append(&admin);
    let _ = menu.append(&PredefinedMenuItem::separator());
    for (i, st) in states.iter().enumerate() {
        let dot = match st {
            vak_ops::State::Running => "●",
            vak_ops::State::Stopped => "○",
            vak_ops::State::NotInstalled => "×",
            vak_ops::State::Unknown => "?",
        };
        let base = (i as u32) << 8;
        let header = MenuItem::new(
            format!("{dot} {} — {}", service(i).label(), st),
            false,
            None,
        );
        let _ = menu.append(&header);

        match st {
            vak_ops::State::NotInstalled => {
                let install = MenuItem::with_id(
                    (base | ACT_INSTALL).to_string(),
                    "Install service",
                    true,
                    None,
                );
                let _ = menu.append(&install);
            }
            other => {
                let running = *other == vak_ops::State::Running;
                let toggle_label = if running { "Stop" } else { "Start" };
                let toggle_action = if running { ACT_STOP } else { ACT_START };
                let tgl =
                    MenuItem::with_id((base | toggle_action).to_string(), toggle_label, true, None);
                let rst =
                    MenuItem::with_id((base | ACT_RESTART).to_string(), "Restart", running, None);
                let _ = menu.append(&tgl);
                let _ = menu.append(&rst);
                let un = MenuItem::with_id(
                    (base | ACT_UNINSTALL).to_string(),
                    "Uninstall service",
                    true,
                    None,
                );
                let _ = menu.append(&un);
            }
        }
        let log = MenuItem::with_id((base | ACT_LOG).to_string(), "Open log", true, None);
        let _ = menu.append(&log);
        let _ = menu.append(&PredefinedMenuItem::separator());
    }

    let wd = CheckMenuItem::with_id(
        ACT_WATCHDOG_TOGGLE.to_string(),
        "Watchdog: auto-restart crashed services",
        true,
        watchdog_on,
        None,
    );
    let _ = menu.append(&wd);
    let _ = menu.append(&PredefinedMenuItem::separator());
    let quit = MenuItem::with_id(ACT_QUIT.to_string(), "Quit tray", true, None);
    let _ = menu.append(&quit);
    menu
}

#[cfg(test)]
mod admin_url_tests {
    use super::admin_url;

    const ALIVE: &dyn Fn(i32) -> bool = &|_| true;
    const DEAD: &dyn Fn(i32) -> bool = &|_| false;

    #[test]
    fn fresh_runtime_yields_one_click_fragment() {
        let raw = r#"{"pid":42,"addr":"127.0.0.1:8901","token":"vk_a/b.c~d"}"#;
        assert_eq!(
            admin_url(raw, ALIVE),
            "http://127.0.0.1:8901/admin#token=vk_a%2Fb.c~d"
        );
    }

    #[test]
    fn dead_pid_or_garbage_falls_back_to_login_page() {
        let raw = r#"{"pid":42,"addr":"127.0.0.1:8901","token":"t"}"#;
        assert_eq!(admin_url(raw, DEAD), "http://127.0.0.1:8901/admin");
        assert_eq!(admin_url("not json", ALIVE), "http://127.0.0.1:8901/admin");
    }
}
