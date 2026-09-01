//! vak-tray: the app bundle's actual entry point (`Info.plist`
//! `CFBundleExecutable`), so double-clicking Vak.app or clicking it
//! in Spotlight runs this binary, not `vak-desktop`.
//!
//! The icon is the same brand mark as the desktop app (`brand_icon`,
//! downsampled from `crates/vak-desktop/icons/icon.png`) — a menu-bar
//! controller with no window of its own was otherwise the one surface a
//! user could not visually tie back to "Vak" at a glance. "One
//! glance answers is my agent alive?" now lives in the tooltip
//! (`status_tooltip`) instead of an icon-color swap: gateway/telegram
//! status in words on hover, not a color the user has to remember the
//! meaning of. The menu starts/stops/restarts, installs/uninstalls,
//! opens logs, and toggles a watchdog that reports a crashed service
//! automatically and posts a system notification when it does. See
//! docs/design/28-operations.md.
//!
//! It also opens the chat window and the admin console. `vak-desktop` —
//! the actual product surface — is a sibling binary this process spawns;
//! nothing about it runs inside the tray. Without that spawn, opening
//! the installed app showed only a menu-bar dot and nothing a
//! first-time user would recognize as "the app is now open."
//! `vak-desktop` guards itself with `tauri_plugin_single_instance`, so
//! spawning it when a window is already open just refocuses that window
//! rather than duplicating it — both the automatic launch below and the
//! always-present "Open Vak" menu item rely on that guarantee.
//! "Open Admin Console" opens a pre-authenticated link built from the
//! gateway token `self install` pins into the canonical `.env`
//! (`ensure_gateway_token`, crates/vak/src/install/mod.rs) — see
//! `open_admin_console` for why that has to be a query param and not a
//! URL fragment.

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
use winit::platform::macos::{ActivationPolicy, EventLoopBuilderExtMacOS as _};

#[derive(Debug, Clone)]
enum TrayEvent {
    Refresh([vak_ops::State; 2]),
    Command(u32),
}

const SVC_COUNT: usize = 2;
const GATEWAY: usize = 0;
const TELEGRAM: usize = 1;

fn service(idx: usize) -> vak_ops::Service {
    if idx == GATEWAY {
        vak_ops::Service::Gateway
    } else {
        vak_ops::Service::Telegram
    }
}

/// 32x32 RGBA dot: the fallback glyph on the rare path where the
/// embedded brand icon fails to decode. No longer the everyday icon —
/// see `brand_icon` for why the tray now shows the same mark as the
/// desktop app. Sized to match `brand_icon`'s output so a decode failure
/// swaps the glyph, not also the icon's apparent size in the menu bar.
#[allow(clippy::expect_used)] // infallible: fixed non-zero dimensions
fn icon_dot(rgb: [u8; 3]) -> Icon {
    const S: usize = 32;
    let mut rgba = Vec::with_capacity(S * S * 4);
    let c = (S as f32 - 1.0) / 2.0;
    for y in 0..S {
        for x in 0..S {
            let d = ((x as f32 - c).powi(2) + (y as f32 - c).powi(2)).sqrt();
            let alpha = if d <= 13.0 {
                255u8
            } else if d <= 15.0 {
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

/// The desktop app's own icon (`crates/vak-desktop/icons/icon.png`),
/// embedded so the tray never depends on a runtime asset path -- it is
/// not shipped into the bundle's Resources by anything else, only the
/// desktop frontend's `dist/` is. Compiled in once via
/// `CARGO_MANIFEST_DIR`, so the same 512x512 PNG both crates already
/// ship from stays the single source of truth for this artwork.
const BRAND_ICON_PNG: &[u8] = include_bytes!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../vak-desktop/icons/icon.png"
));

/// The tray's glyph, downsampled from the real brand icon rather than a
/// plain colour dot: a menu-bar controller with no other window is
/// otherwise the one surface a user cannot visually tie back to
/// "Vak" at a glance. 512 -> 32 is an exact 16:1 block average
/// (not a naive nearest-neighbour point sample), which is close to an
/// ideal filter at this ratio and keeps the downsized glyph legible
/// rather than aliased. 16x16 was tried first and rejected by actually
/// looking at the result: the glyph's letterform collapsed into
/// illegible grey mush at that size, confirmed by rendering the real
/// downsample output rather than trusting the resampling math alone.
///
/// Status used to be color-coded here (green/amber/red/grey). That
/// signal moves to the tooltip instead of a badge composited onto an
/// already-detailed glyph: the icon has three-tone content (navy glyph,
/// orange bar, blue accent) inside a rounded white square, and a second
/// small colored dot layered on top of that was judged more likely to
/// read as noise than as a clear status signal -- a tooltip that says so
/// in words has no such risk.
fn brand_icon() -> Option<Icon> {
    let (rgba, w, h) = decode_and_downsample(BRAND_ICON_PNG, 32)?;
    Icon::from_rgba(rgba, w, h).ok()
}

/// Decode an RGBA8 PNG and box-average it down to `out x out`, returning
/// `(pixels, out, out)`. Pure and allocation-only — no `Icon`, no FFI —
/// so the actual resampling math is unit-testable without a live tray or
/// a platform icon backend.
///
/// Requires `out` to evenly divide both source dimensions, matching
/// `brand_icon`'s use against a known 512x512 asset with `out = 32`
/// (an exact 16:1 block average); returns `None` rather than guess at a
/// fractional-box filter for a source this function was not written to
/// handle.
fn decode_and_downsample(png_bytes: &[u8], out: usize) -> Option<(Vec<u8>, u32, u32)> {
    let decoder = png::Decoder::new(png_bytes);
    let mut reader = decoder.read_info().ok()?;
    let mut buf = vec![0u8; reader.output_buffer_size()];
    let info = reader.next_frame(&mut buf).ok()?;
    if info.color_type != png::ColorType::Rgba || info.bit_depth != png::BitDepth::Eight {
        // Fail soft to the fallback rather than guess at a pixel layout
        // this function was not written to handle.
        return None;
    }
    let (src_w, src_h) = (info.width as usize, info.height as usize);
    if out == 0 || src_w % out != 0 || src_h % out != 0 {
        return None;
    }
    let (bx, by) = (src_w / out, src_h / out);
    let mut result = vec![0u8; out * out * 4];
    for oy in 0..out {
        for ox in 0..out {
            let mut sum = [0u32; 4];
            for sy in 0..by {
                for sx in 0..bx {
                    let px = (oy * by + sy) * src_w + (ox * bx + sx);
                    for c in 0..4 {
                        sum[c] += u32::from(buf[px * 4 + c]);
                    }
                }
            }
            let n = (bx * by) as u32;
            let out_px = (oy * out + ox) * 4;
            for c in 0..4 {
                result[out_px + c] = (sum[c] / n) as u8;
            }
        }
    }
    Some((result, out as u32, out as u32))
}

/// `brand_icon`, falling back to the plain grey dot if the embedded PNG
/// ever fails to decode -- the tray must still have some icon rather
/// than none.
fn startup_icon() -> Icon {
    brand_icon().unwrap_or_else(|| icon_dot([158, 158, 158]))
}

struct Ui {
    tray: TrayIcon,
    watchdog: Arc<AtomicBool>,
    states: [vak_ops::State; 2],
    /// (states, watchdog_on) as of the last menu/icon rebuild. Replacing
    /// a status item's NSMenu is only safe when it is not currently being
    /// tracked (open) by the user -- there is no "menu will open" hook in
    /// this version of tray-icon to defer the rebuild until then, so the
    /// next-best guard is to never replace it on a bare timer tick when
    /// nothing changed. Refreshing every 3s unconditionally meant any
    /// refresh landing while the user had the menu open could tear it
    /// down mid-track, which read as "the menu opens then disappears."
    /// A steady healthy system rebuilds roughly never instead of 1200
    /// times an hour.
    last_rendered: Option<([vak_ops::State; 2], bool)>,
}

impl ApplicationHandler<TrayEvent> for Ui {
    fn resumed(&mut self, _loop: &ActiveEventLoop) {}

    fn about_to_wait(&mut self, _loop: &ActiveEventLoop) {}

    fn user_event(&mut self, loop_handle: &ActiveEventLoop, event: TrayEvent) {
        match event {
            TrayEvent::Refresh(states) => {
                let _ = loop_handle;
                self.states = states;
                let watchdog_on = self.watchdog.load(Ordering::SeqCst);
                let snapshot = (states, watchdog_on);
                if self.last_rendered == Some(snapshot) {
                    return;
                }
                self.last_rendered = Some(snapshot);
                self.tray
                    .set_menu(Some(Box::new(build_menu(&states, watchdog_on))));
                // The icon itself is the fixed brand mark now (see
                // brand_icon); status moves to the tooltip instead of an
                // icon color swap.
                let _ = self.tray.set_tooltip(Some(status_tooltip(&states)));
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
        let cfg = ops_config();
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
                    notify("vak", &e);
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
            ACT_QUIT => std::process::exit(0),
            ACT_OPEN_DESKTOP => open_desktop(),
            ACT_OPEN_ADMIN => open_admin_console(),
            ACT_OPEN_OPERATIONS => open_operations_center(),
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
const ACT_OPEN_OPERATIONS: u32 = 11;

fn persist_watchdog(on: bool) {
    // home() is already the canonical data home; a further ".vak"
    // segment here would nest a bogus nested legacy-named directory
    // inside it rather than writing there directly.
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
    // Canonical data home (doc 32) — never hand-roll HOME/.vak.
    vak_config::paths::data_home()
}

fn notify(title: &str, body: &str) {
    let _ = notify_rust::Notification::new()
        .summary(title)
        .body(body)
        .show();
}

/// The chat window, as a sibling binary next to this one — same
/// directory in an installed bundle (`Contents/MacOS/`), same
/// `target/{debug,release}/` in a dev build. `None` in a dev tree where
/// only the tray was built; the caller decides whether that is worth
/// telling the user about.
fn desktop_binary() -> Option<std::path::PathBuf> {
    let sibling = std::env::current_exe().ok()?.parent()?.join("vak-desktop");
    sibling.exists().then_some(sibling)
}

/// Open the chat window. Safe to call whether or not one is already
/// open: `vak-desktop` enforces single-instance itself and refocuses the
/// existing window instead of duplicating it, so this never needs to
/// track that state here. Spawned detached — the tray outlives the
/// window and must not wait on it or inherit its lifetime.
/// Resolve the managed gateway port from the same user-level environment as
/// the server. The legacy tray is launched directly by the session manager,
/// so it does not pass through the CLI's dotenv-loading path first.
fn ops_config() -> vak_ops::OpsConfig {
    let env_path = vak_config::paths::data_home().join(".env");
    vak_config::replace_env_files(&[env_path.as_path()]);
    vak_ops::OpsConfig::detect()
}

/// The token pinned into the canonical `.env` at `self install`
/// (`ensure_gateway_token`, crates/vak/src/install/mod.rs). `None`
/// on an install that predates that pinning step -- the token still
/// exists (freshly minted on every boot), it is just not discoverable
/// from outside the running process, so there is nothing to build a link
/// with. Read fresh on every click rather than cached at tray startup,
/// so a token added by reinstalling after the tray was already running
/// is picked up immediately.
fn pinned_gateway_token() -> Option<String> {
    let path = vak_config::user_env_path()?;
    let text = std::fs::read_to_string(path).ok()?;
    text.lines().find_map(|line| {
        let (key, value) = line.split_once('=')?;
        (key.trim() == "VAK_GATEWAY_TOKEN")
            .then(|| value.trim().to_string())
            .filter(|v| !v.is_empty())
    })
}

/// Open the admin console, pre-authenticated.
///
/// `?token=` (not `#token=`) deliberately: the SPA's own router treats
/// the entire `location.hash` as the route (`#/overview` and so on,
/// vak-admin-ui/src/store.ts), so a `#token=` fragment would collide
/// with it instead of composing. The query string is unrelated to
/// routing and the SPA scrubs it via `history.replaceState` immediately
/// after logging in, so it does not linger in the address bar. This is
/// the same local-loopback pre-authenticated-link pattern Jupyter's own
/// `?token=` uses; the value is the same bearer token already used for
/// every other authenticated request, not a weaker credential minted
/// for this purpose.
fn open_admin_path(route: &str) {
    let cfg = ops_config();
    if vak_ops::status(vak_ops::Service::Gateway, &cfg) != vak_ops::State::Running {
        notify("vak", "start the gateway service first (Gateway → Start)");
        return;
    }
    let url = match pinned_gateway_token() {
        Some(token) => format!("http://127.0.0.1:{}/admin?token={token}{route}", cfg.port),
        None => {
            // Older install: no pinned token to build a one-click link
            // with. The console still works -- open it to the manual
            // login form rather than not opening it at all.
            notify(
                "vak",
                "no pinned token found — reinstall to enable one-click login; opening manual login",
            );
            format!("http://127.0.0.1:{}/admin{route}", cfg.port)
        }
    };
    if let Err(e) = std::process::Command::new("open").arg(url).spawn() {
        notify("vak", &format!("could not open admin console: {e}"));
    }
}

fn open_admin_console() {
    open_admin_path("#/overview");
}

fn open_operations_center() {
    open_admin_path("#/operations");
}

fn open_desktop() {
    let Some(bin) = desktop_binary() else {
        notify("vak", "Vak desktop app not found next to the tray binary");
        return;
    };
    if let Err(e) = std::process::Command::new(bin).spawn() {
        notify("vak", &format!("could not open Vak: {e}"));
    }
}

/// Exclusive ownership of the menu-bar icon, held for the process's
/// lifetime by whichever tray started first.
///
/// The bundle's `CFBundleExecutable` is this binary, and
/// `com.vak.tray` also runs it as a launchd service with
/// `RunAtLoad`. So the ordinary path -- install, `services-sync`, then
/// open Vak from Finder, Spotlight, or the Dock -- started a
/// *second* tray and put two identical icons in the menu bar, with no
/// guard anywhere against it.
///
/// `None` means another live tray already owns the menu bar. The caller
/// then does what launching the app actually asked for -- open the chat
/// window -- and exits, instead of duplicating an icon or (worse, once
/// macOS starts merely re-activating the running app rather than
/// spawning a new one) doing nothing visible at all.
///
/// Same mechanism as `vak_server::telegram::InstanceLock`: an O_EXCL
/// marker plus a liveness probe on the recorded pid, so a crashed holder
/// leaves a marker the next launch reclaims rather than a lock that
/// wedges the menu bar until a reboot. Deliberately not flock, which
/// would need `unsafe` -- the workspace denies it.
struct MenuBarLock {
    path: std::path::PathBuf,
}

impl Drop for MenuBarLock {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.path);
    }
}

fn claim_menu_bar() -> Option<MenuBarLock> {
    let dir = home().join("locks");
    std::fs::create_dir_all(&dir).ok()?;
    let path = dir.join("tray.lock");

    if try_claim(&path) {
        return Some(MenuBarLock { path });
    }
    // Marker present: only yield to a holder that is actually alive.
    let holder = std::fs::read_to_string(&path).unwrap_or_default();
    let pid = holder
        .split_whitespace()
        .find_map(|t| t.parse::<u32>().ok());
    if pid.is_some_and(pid_alive) {
        return None;
    }
    let _ = std::fs::remove_file(&path);
    try_claim(&path).then(|| MenuBarLock { path })
}

fn try_claim(path: &std::path::Path) -> bool {
    if std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(path)
        .is_err()
    {
        return false;
    }
    std::fs::write(path, format!("pid {}\n", std::process::id())).is_ok()
}

/// Liveness probe without libc: `kill -0` via a subprocess, matching
/// how `vak_server::telegram::InstanceLock` does it.
fn pid_alive(pid: u32) -> bool {
    std::process::Command::new("kill")
        .arg("-0")
        .arg(pid.to_string())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .map(|st| st.success())
        .unwrap_or(false)
}

fn main() {
    // Canonical layout migration (doc 32); tray has no config override
    // path of its own — a failed migration only degrades to old paths.
    if let Err(e) = vak_config::paths::migrate_legacy_home() {
        eprintln!("[warn] home migration skipped: {e}");
    }

    // Held for the whole process lifetime: dropping it early would
    // release the lock and let a later launch add a second icon.
    let Some(_menu_bar) = claim_menu_bar() else {
        // Another tray owns the menu bar. Launching the app is a request
        // to see the app, so honour that and get out of the way.
        open_desktop();
        return;
    };
    let watchdog = Arc::new(AtomicBool::new(load_watchdog()));

    // Menu-bar-only, set here rather than via the bundle's LSUIElement.
    // The bundle now launches vak-desktop (see install::bundle), and a
    // bundle-wide LSUIElement would have hidden that app too. Setting the
    // policy on our own event loop keeps this process out of the Dock
    // without constraining the app the bundle actually launches.
    let event_loop: EventLoop<TrayEvent> = EventLoop::with_user_event()
        .with_activation_policy(ActivationPolicy::Accessory)
        .build()
        .expect("event loop");

    let cfg = ops_config();
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
                // launchd/systemd owns recovery. The tray only reports a
                // transition so a transient probe can never kill a healthy
                // process by issuing a competing restart.
                for i in 0..SVC_COUNT {
                    let running = states[i] == vak_ops::State::Running;
                    if watchdog.load(Ordering::SeqCst)
                        && last_running[i]
                        && !running
                        && last_down_notify.elapsed() > Duration::from_secs(60)
                    {
                        notify(
                            "vak watchdog",
                            &format!(
                                "{} is down — the service manager will recover it",
                                service(i).label()
                            ),
                        );
                        last_down_notify = std::time::Instant::now();
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
        .with_tooltip(status_tooltip(&states_now()))
        .with_icon(startup_icon())
        .build()
        .expect("tray built");

    // Deliberately does NOT open the chat window here. This process is a
    // background service (com.vak.tray, RunAtLoad), so doing so
    // would throw a window in the user's face at every login. Opening
    // the app is now the bundle's job -- its CFBundleExecutable is
    // vak-desktop -- and "Open Vak" in the menu covers the rest.

    let mut ui = Ui {
        tray,
        watchdog,
        states: [vak_ops::State::Unknown; 2],
        last_rendered: None,
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

fn states_now() -> [vak_ops::State; 2] {
    let cfg = ops_config();
    [
        vak_ops::status(vak_ops::Service::Gateway, &cfg),
        vak_ops::status(vak_ops::Service::Telegram, &cfg),
    ]
}

/// "one glance answers is my agent alive" now lives here rather than in
/// the icon's color: a hover away, exactly as reliable, and it says the
/// actual state in words instead of asking the user to remember what
/// amber means.
fn status_tooltip(states: &[vak_ops::State; 2]) -> String {
    // State's Display already renders lowercase ("running", "stopped", …).
    format!(
        "vak — gateway {}, telegram {}",
        states[GATEWAY], states[TELEGRAM]
    )
}

fn build_menu(states: &[vak_ops::State; 2], watchdog_on: bool) -> Menu {
    let menu = Menu::new();
    // Top of the menu, always present: the three actions a user is
    // actually looking for. Everything below is service plumbing.
    let open = MenuItem::with_id(ACT_OPEN_DESKTOP.to_string(), "Open Vak", true, None);
    let _ = menu.append(&open);
    let admin = MenuItem::with_id(ACT_OPEN_ADMIN.to_string(), "Open Admin Console", true, None);
    let _ = menu.append(&admin);
    let operations = MenuItem::with_id(
        ACT_OPEN_OPERATIONS.to_string(),
        "Open Operations Center",
        true,
        None,
    );
    let _ = menu.append(&operations);
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
        "Watchdog: alert on service failures",
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
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod brand_icon_tests {
    use super::*;

    #[test]
    fn embedded_asset_is_a_decodable_512_square_and_downsamples_to_32() {
        // Guards the two assumptions decode_and_downsample's early
        // returns are silently tolerant of: that the checked-in asset is
        // still RGBA8, and that 512 is still evenly divisible by 32. A
        // change to icon.png that broke either would otherwise degrade
        // silently to the grey-dot fallback with no build-time signal.
        let (rgba, w, h) = decode_and_downsample(BRAND_ICON_PNG, 32)
            .expect("the shipped icon.png must decode and downsample");
        assert_eq!((w, h), (32, 32));
        assert_eq!(rgba.len(), 32 * 32 * 4, "one RGBA quad per output pixel");
    }

    #[test]
    fn a_uniform_source_downsamples_to_the_same_uniform_color() {
        // The averaging math itself, isolated from the real asset: every
        // source pixel identical must produce that same pixel at every
        // output position, exactly (no drift from integer truncation
        // when the block is uniform).
        let src = flat_rgba_png(32, 32, [200, 100, 50, 255]);
        let (rgba, w, h) = decode_and_downsample(&src, 4).unwrap();
        assert_eq!((w, h), (4, 4));
        for px in rgba.chunks_exact(4) {
            assert_eq!(px, [200, 100, 50, 255]);
        }
    }

    #[test]
    fn a_block_averages_its_whole_area_not_just_one_corner() {
        // A downsample that sampled only the block's top-left corner
        // (a point sample dressed up as an average) would pass the
        // uniform-source test above but land on 0 here, not the
        // midpoint, and fail silently.
        let mut px = vec![0u8; 4 * 4 * 4];
        // Top half black, bottom half white, opaque -- an even 8/8 split
        // of the whole 4x4 block, not just one quadrant of it.
        for y in 0..4usize {
            for x in 0..4usize {
                let v = if y < 2 { 0 } else { 255 };
                let i = (y * 4 + x) * 4;
                px[i..i + 4].copy_from_slice(&[v, v, v, 255]);
            }
        }
        let src = rgba_png(4, 4, &px);
        let (rgba, _, _) = decode_and_downsample(&src, 1).unwrap();
        // An even split across the whole block must land at the
        // midpoint, not at either extreme.
        assert_eq!(rgba[0..3], [127, 127, 127]);
    }

    #[test]
    fn a_size_the_source_cannot_evenly_divide_is_refused_not_approximated() {
        let src = flat_rgba_png(10, 10, [1, 2, 3, 4]);
        assert!(
            decode_and_downsample(&src, 3).is_none(),
            "10 is not a multiple of 3 -- must fail rather than silently pick a wrong filter"
        );
    }

    fn flat_rgba_png(w: u32, h: u32, rgba: [u8; 4]) -> Vec<u8> {
        let px: Vec<u8> = rgba.repeat((w * h) as usize);
        rgba_png(w, h, &px)
    }

    fn rgba_png(w: u32, h: u32, px: &[u8]) -> Vec<u8> {
        let mut out = Vec::new();
        {
            let mut encoder = png::Encoder::new(&mut out, w, h);
            encoder.set_color(png::ColorType::Rgba);
            encoder.set_depth(png::BitDepth::Eight);
            let mut writer = encoder.write_header().unwrap();
            writer.write_image_data(px).unwrap();
        }
        out
    }
}
