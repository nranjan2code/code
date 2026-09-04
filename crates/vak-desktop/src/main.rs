#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

//! vak-desktop: native shell around the same HTTP+SSE contract every other
//! surface (tui/exec/serve) speaks. The webview gets a loopback bearer token
//! for the embedded `vak-server` router; nothing about the agent protocol is
//! re-implemented here.

mod pty;

use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use serde::{Deserialize, Serialize};
use tauri::menu::{CheckMenuItem, Menu, MenuItem, PredefinedMenuItem};
use tauri::tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent};
use tauri::{AppHandle, Emitter, Manager, RunEvent, State, WindowEvent};

const TRAY_ID: &str = "vak";
const TRAY_OPEN_ID: &str = "desktop.open";
const TRAY_HOME_ID: &str = "desktop.home";
const TRAY_APP_ID: &str = "desktop.app";
const TRAY_ADMIN_ID: &str = "desktop.admin";
const TRAY_WATCHDOG_ID: &str = "desktop.watchdog";
const TRAY_QUIT_ID: &str = "desktop.quit";
const GATEWAY: usize = 0;
const BRIDGES: usize = 1;

struct TrayState {
    watchdog: Arc<AtomicBool>,
    last_rendered: Mutex<Option<([vak_ops::State; 2], bool)>>,
}

/// Start with the menu-bar icon only, leaving the main window hidden.
///
/// The `com.vak.desktop` / `vak-desktop.service` unit passes this so a
/// login launch restores the tray without throwing a window on screen at
/// every boot. Every other way in — double-click, Dock, `Open Vak`, a
/// second launch handed over by the single-instance plugin — reveals the
/// window, so the flag only suppresses the one startup nobody asked for.
const TRAY_FLAG: &str = "--tray";

/// The window is created hidden (`visible: false` in `tauri.conf.json`)
/// and revealed here, rather than created visible and hidden again: the
/// latter flashes a full-size window on screen before the setup hook can
/// run.
fn tray_only_start() -> bool {
    is_tray_launch(std::env::args_os())
}

/// Whether an argv asks for a tray-only launch.
///
/// Split out from [`tray_only_start`] so it can also classify the argv the
/// single-instance plugin hands over from a *second* process, which is not
/// this process's own `std::env::args`.
fn is_tray_launch<S: AsRef<std::ffi::OsStr>>(args: impl IntoIterator<Item = S>) -> bool {
    args.into_iter()
        .any(|arg| arg.as_ref() == std::ffi::OsStr::new(TRAY_FLAG))
}

fn requested_project<S: AsRef<std::ffi::OsStr>>(
    args: impl IntoIterator<Item = S>,
) -> Option<PathBuf> {
    args.into_iter()
        .map(|arg| PathBuf::from(arg.as_ref()))
        .find(|path| path.is_dir())
}

fn show_main_window(app: &AppHandle) {
    if let Some(window) = app.get_webview_window("main") {
        let _ = window.unminimize();
        let _ = window.show();
        let _ = window.set_focus();
    }
}

/// The desktop process is Vak's only GUI lifecycle owner. Keeping the
/// tray here means a Dock/Finder activation and a tray activation target the
/// same process and always have a window to reveal.
fn service(index: usize) -> vak_ops::Service {
    if index == GATEWAY {
        vak_ops::Service::Gateway
    } else {
        vak_ops::Service::Bridges
    }
}

fn states_now() -> [vak_ops::State; 2] {
    let config = vak_ops::OpsConfig::detect();
    [
        vak_ops::status(vak_ops::Service::Gateway, &config),
        vak_ops::status(vak_ops::Service::Bridges, &config),
    ]
}

fn status_tooltip(states: &[vak_ops::State; 2]) -> String {
    format!(
        "Vak — gateway {}, chat bridges {}",
        states[GATEWAY], states[BRIDGES]
    )
}

fn service_menu(
    app: &tauri::AppHandle,
    index: usize,
    state: vak_ops::State,
) -> tauri::Result<Vec<tauri::menu::MenuItem<tauri::Wry>>> {
    let prefix = if index == GATEWAY {
        "gateway"
    } else {
        // One label for every transport: a bridge belongs to a bot, and
        // bots name their own surface (AGENTS.md invariant 23).
        "bridges"
    };
    let dot = match state {
        vak_ops::State::Running => "●",
        vak_ops::State::Stopped => "○",
        vak_ops::State::NotInstalled => "×",
        vak_ops::State::Unknown => "?",
    };
    let header = MenuItem::with_id(
        app,
        format!("desktop.{prefix}.status"),
        format!("{dot} {} — {state}", service(index).label()),
        false,
        None::<&str>,
    )?;
    let mut items = vec![header];
    if state == vak_ops::State::NotInstalled {
        items.push(MenuItem::with_id(
            app,
            format!("desktop.{prefix}.install"),
            "Install service",
            true,
            None::<&str>,
        )?);
    } else {
        let running = state == vak_ops::State::Running;
        items.push(MenuItem::with_id(
            app,
            format!(
                "desktop.{prefix}.{}",
                if running { "stop" } else { "start" }
            ),
            if running { "Stop" } else { "Start" },
            true,
            None::<&str>,
        )?);
        items.push(MenuItem::with_id(
            app,
            format!("desktop.{prefix}.restart"),
            "Restart",
            running,
            None::<&str>,
        )?);
        items.push(MenuItem::with_id(
            app,
            format!("desktop.{prefix}.uninstall"),
            "Uninstall service",
            true,
            None::<&str>,
        )?);
    }
    items.push(MenuItem::with_id(
        app,
        format!("desktop.{prefix}.log"),
        "Open log",
        true,
        None::<&str>,
    )?);
    Ok(items)
}

fn build_tray_menu(
    app: &tauri::AppHandle,
    states: &[vak_ops::State; 2],
    watchdog_on: bool,
) -> tauri::Result<Menu<tauri::Wry>> {
    let open = MenuItem::with_id(app, TRAY_OPEN_ID, "Open Vak", true, None::<&str>)?;
    // The same three destinations the landing page offers, in the same
    // order and with the same names. Two entries that both opened the admin
    // console at different hash routes was the tray describing one page as
    // if it were two products.
    let home = MenuItem::with_id(app, TRAY_HOME_ID, "Home", true, None::<&str>)?;
    let workspace = MenuItem::with_id(app, TRAY_APP_ID, "Workspace", true, None::<&str>)?;
    let admin = MenuItem::with_id(app, TRAY_ADMIN_ID, "Operations", true, None::<&str>)?;
    let separator_top = PredefinedMenuItem::separator(app)?;
    let separator = PredefinedMenuItem::separator(app)?;
    let gateway = service_menu(app, GATEWAY, states[GATEWAY])?;
    let separator_gateway = PredefinedMenuItem::separator(app)?;
    let bridges = service_menu(app, BRIDGES, states[BRIDGES])?;
    let separator_bridges = PredefinedMenuItem::separator(app)?;
    let watchdog = CheckMenuItem::with_id(
        app,
        TRAY_WATCHDOG_ID,
        "Watchdog: alert on service failures",
        true,
        watchdog_on,
        None::<&str>,
    )?;
    let separator_watchdog = PredefinedMenuItem::separator(app)?;
    let quit = MenuItem::with_id(app, TRAY_QUIT_ID, "Quit Vak", true, None::<&str>)?;
    let mut items: Vec<&dyn tauri::menu::IsMenuItem<tauri::Wry>> =
        vec![&open, &separator_top, &home, &workspace, &admin, &separator];
    items.extend(
        gateway
            .iter()
            .map(|item| item as &dyn tauri::menu::IsMenuItem<tauri::Wry>),
    );
    items.push(&separator_gateway);
    items.extend(
        bridges
            .iter()
            .map(|item| item as &dyn tauri::menu::IsMenuItem<tauri::Wry>),
    );
    items.extend([
        &separator_bridges as &dyn tauri::menu::IsMenuItem<tauri::Wry>,
        &watchdog,
        &separator_watchdog,
        &quit,
    ]);
    Menu::with_items(app, &items)
}

fn refresh_tray(app: &AppHandle) {
    let states = states_now();
    let tray_state = app.state::<TrayState>();
    let watchdog_on = tray_state.watchdog.load(Ordering::SeqCst);
    let snapshot = (states, watchdog_on);
    let mut last = tray_state
        .last_rendered
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    if *last == Some(snapshot) {
        return;
    }
    if let (Some(tray), Ok(menu)) = (
        app.tray_by_id(TRAY_ID),
        build_tray_menu(app, &states, watchdog_on),
    ) {
        let _ = tray.set_menu(Some(menu));
        let _ = tray.set_tooltip(Some(status_tooltip(&states)));
        *last = Some(snapshot);
    }
}

fn persist_watchdog(on: bool) {
    let path = vak_config::paths::data_home().join("tray.json");
    if let Some(parent) = path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    let _ = std::fs::write(path, serde_json::json!({ "auto_restart": on }).to_string());
}

fn load_watchdog() -> bool {
    std::fs::read_to_string(vak_config::paths::data_home().join("tray.json"))
        .ok()
        .and_then(|text| serde_json::from_str::<serde_json::Value>(&text).ok())
        .and_then(|value| value["auto_restart"].as_bool())
        .unwrap_or(true)
}

fn notify(title: &str, body: &str) {
    let _ = notify_rust::Notification::new()
        .summary(title)
        .body(body)
        .show();
}

fn pinned_gateway_token() -> Option<String> {
    std::fs::read_to_string(vak_config::user_env_path()?)
        .ok()?
        .lines()
        .find_map(|line| {
            let (key, value) = line.split_once('=')?;
            (key.trim() == "VAK_GATEWAY_TOKEN")
                .then(|| value.trim().to_string())
                .filter(|value| !value.is_empty())
        })
}

/// Open one of the gateway's web surfaces in a browser.
///
/// `path` is a whole path (`/`, `/app`, `/admin`), not an admin-relative
/// fragment: the tray used to know only about the admin console and
/// addressed it by hash route, which is why it grew two entries pointing
/// into the same page instead of one entry per surface.
///
/// The token still rides along for a server that predates loopback
/// auto-login, or one where an operator turned it off; the client consumes
/// it once and strips it from the address bar.
fn open_web_path(path: &str) {
    let config = vak_ops::OpsConfig::detect();
    if vak_ops::status(vak_ops::Service::Gateway, &config) != vak_ops::State::Running {
        notify("Vak", "Start the gateway service first.");
        return;
    }
    let base = format!("http://127.0.0.1:{}{path}", config.port);
    let url = match pinned_gateway_token() {
        Some(token) => format!("{base}?token={token}"),
        None => base,
    };
    if let Err(error) = std::process::Command::new("open").arg(url).spawn() {
        notify("Vak", &format!("Could not open that page: {error}"));
    }
}

fn run_service_action(app: &AppHandle, service: vak_ops::Service, action: &str) {
    let config = vak_ops::OpsConfig::detect();
    let problem = match action {
        "start" if !vak_ops::start(service, &config) => Some("could not start service".to_string()),
        "stop" if !vak_ops::stop(service, &config) => Some("could not stop service".to_string()),
        "restart" if !vak_ops::restart(service, &config) => {
            Some("could not restart service".to_string())
        }
        "install" => vak_ops::install(service, &config).err(),
        "uninstall" => vak_ops::uninstall(service, &config).err(),
        "log" => {
            vak_ops::open_log(service);
            None
        }
        _ => None,
    };
    if let Some(problem) = problem {
        notify("Vak", &format!("{}: {problem}", service.label()));
    }
    refresh_tray(app);
}

fn handle_tray_menu(app: &AppHandle, id: &str) {
    match id {
        TRAY_OPEN_ID => show_main_window(app),
        TRAY_HOME_ID => open_web_path("/"),
        TRAY_APP_ID => open_web_path("/app"),
        TRAY_ADMIN_ID => open_web_path("/admin"),
        TRAY_WATCHDOG_ID => {
            let tray = app.state::<TrayState>();
            let new_value = !tray.watchdog.load(Ordering::SeqCst);
            tray.watchdog.store(new_value, Ordering::SeqCst);
            persist_watchdog(new_value);
            refresh_tray(app);
        }
        TRAY_QUIT_ID => app.exit(0),
        _ => {
            for (prefix, service) in [
                ("desktop.gateway.", vak_ops::Service::Gateway),
                ("desktop.bridges.", vak_ops::Service::Bridges),
            ] {
                if let Some(action) = id.strip_prefix(prefix) {
                    run_service_action(app, service, action);
                    break;
                }
            }
        }
    }
}

fn start_tray_monitor(app: AppHandle) {
    std::thread::spawn(move || {
        let mut last_running = [false; 2];
        loop {
            let states = states_now();
            let tray = app.state::<TrayState>();
            if tray.watchdog.load(Ordering::SeqCst) {
                for index in 0..2 {
                    let running = states[index] == vak_ops::State::Running;
                    if last_running[index] && !running {
                        notify(
                            "Vak watchdog",
                            &format!(
                                "{} is down — launchd will recover it",
                                service(index).label()
                            ),
                        );
                    }
                    last_running[index] = running;
                }
            } else {
                for index in 0..2 {
                    last_running[index] = states[index] == vak_ops::State::Running;
                }
            }
            refresh_tray(&app);
            std::thread::sleep(Duration::from_secs(3));
        }
    });
}

fn install_tray(app: &tauri::App) -> tauri::Result<()> {
    let states = states_now();
    let watchdog_on = app.state::<TrayState>().watchdog.load(Ordering::SeqCst);
    let menu = build_tray_menu(app.handle(), &states, watchdog_on)?;
    let mut tray = TrayIconBuilder::with_id("vak")
        .menu(&menu)
        .tooltip(status_tooltip(&states))
        .show_menu_on_left_click(false)
        .on_menu_event(|app, event| handle_tray_menu(app, event.id().as_ref()))
        .on_tray_icon_event(|tray, event| {
            if matches!(
                event,
                TrayIconEvent::Click {
                    button: MouseButton::Left,
                    button_state: MouseButtonState::Up,
                    ..
                }
            ) {
                show_main_window(tray.app_handle());
            }
        });
    let icon = tauri::image::Image::from_bytes(include_bytes!("../icons/icon.png"))?;
    tray = tray.icon(icon);
    tray.build(app)?;
    start_tray_monitor(app.handle().clone());
    Ok(())
}

#[derive(Clone)]
struct Backend {
    shutdown: tokio::sync::watch::Sender<bool>,
}

struct BackendState {
    running: Mutex<Option<Running>>,
    switching: tokio::sync::Mutex<()>,
}

struct Running {
    info: BackendInfo,
    backend: Backend,
}

#[derive(Serialize, Clone, Default)]
struct BackendInfo {
    ready: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    base_url: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    token: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    cwd: Option<String>,
    /// Why the last boot attempt failed, if it did. The webview reads this
    /// so a silent launch failure never traps the user on the project gate.
    #[serde(skip_serializing_if = "Option::is_none")]
    boot_error: Option<String>,
    recent_workspaces: Vec<String>,
}

#[derive(Deserialize, Serialize, Default)]
#[serde(default)]
struct DesktopPrefs {
    last_project: Option<String>,
    recent_workspaces: Vec<String>,
}

/// The same canonical data home the CLI, TUI, and wizard use
/// (`vak_config::paths::data_home`, doc 32) — never a hand-rolled path.
///
/// This used to hardcode `~/.vak`, the pre-canonical-layout location.
/// `Core::set_provider_key` — the wizard, and the TUI's `/key` command —
/// write credentials through `Core::user_env_file()`, which resolves to
/// the canonical home. A desktop launch reading `.env` from the old
/// dotdir could therefore never see a key saved anywhere else: every run
/// failed `Core::provider()`, and with the silent-503 bug this fix's
/// sibling change addresses, that failure was invisible. It also
/// explains the "home migration skipped: both ... exist" warning — this
/// function kept writing `desktop.json` and profile notes into the
/// legacy dir, so it could never go away.
fn vak_home() -> PathBuf {
    vak_config::paths::data_home()
}

fn prefs_path() -> PathBuf {
    vak_home().join("desktop.json")
}

fn desktop_prefs() -> DesktopPrefs {
    std::fs::read_to_string(prefs_path())
        .ok()
        .and_then(|text| serde_json::from_str(&text).ok())
        .unwrap_or_default()
}

fn last_project() -> Option<PathBuf> {
    let cwd = PathBuf::from(desktop_prefs().last_project?);
    cwd.is_dir().then_some(cwd)
}

/// The workspaces this shell offers, from the ONE store every surface
/// shares (`vak_core::workspaces`).
///
/// The desktop used to keep its own list in `desktop.json`, so a workspace
/// removed in the browser stayed in the desktop's sidebar and vice versa —
/// two lists of the same thing, guaranteed to disagree. `desktop.json`
/// still holds `last_project` (which workspace to reopen at launch), which
/// is genuinely this shell's own business.
fn recent_workspaces() -> Vec<String> {
    vak_core::workspaces::visible(
        desktop_prefs()
            .recent_workspaces
            .into_iter()
            .map(PathBuf::from),
    )
    .into_iter()
    .map(|p| p.to_string_lossy().into_owned())
    .collect()
}

fn save_project(cwd: &str) {
    // The shared store is what every surface reads; this also un-forgets a
    // workspace, so reopening one is how it comes back anywhere.
    if let Err(e) = vak_core::workspaces::remember(std::path::Path::new(cwd)) {
        eprintln!("warning: could not record the workspace: {e}");
    }
    let mut prefs = desktop_prefs();
    let previous = prefs.last_project.clone();
    prefs.last_project = Some(cwd.to_string());
    prefs
        .recent_workspaces
        .retain(|path| path != cwd && previous.as_deref() != Some(path));
    prefs.recent_workspaces.insert(0, cwd.to_string());
    if let Some(previous) = previous.filter(|path| path != cwd && PathBuf::from(path).is_dir()) {
        prefs.recent_workspaces.insert(1, previous);
    }
    prefs.recent_workspaces.truncate(8);
    let _ = std::fs::create_dir_all(vak_home());
    if let Ok(json) = serde_json::to_string(&prefs) {
        let _ = std::fs::write(prefs_path(), json);
    }
}

/// Load the Shared `.env`, and the project's own only when the workspace
/// is trusted.
///
/// A project `.env` can inject `VAK_*_BASE_URL` and other privileged
/// values, so loading it is part of the trust decision — not a consequence
/// of having opened a folder (doc 46 security invariant 2).
fn load_workspace_env(cwd: &std::path::Path, trusted: bool) {
    let user = vak_home().join(".env");
    if trusted {
        let project = cwd.join(".env");
        vak_config::replace_env_files(&[user.as_path(), project.as_path()]);
    } else {
        vak_config::replace_env_files(&[user.as_path()]);
    }
}

/// Boot the embedded agent server on an ephemeral loopback port.
///
/// `trusted` is the operator's recorded decision about *this* folder, not
/// an assumption drawn from having opened it. Selecting a directory used
/// to imply full consent to whatever its `.vak/config.toml` asked for —
/// hooks, MCP servers, a redirected provider endpoint — which is the
/// hole doc 46 Step 2 exists to close.
async fn boot_backend(cwd: PathBuf, trusted: bool) -> Result<Running, String> {
    vak_config::ensure_project_config(&cwd).map_err(|e| e.to_string())?;
    let core = vak_core::Core::new_with_trust(cwd.clone(), trusted)
        .map(|c| c.with_surface(vak_core::Surface::Desktop))
        .map_err(|e| e.to_string())?;

    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .map_err(|e| format!("bind failed: {e}"))?;
    let addr = listener.local_addr().map_err(|e| e.to_string())?;
    let (router, token) = vak_server::secured_router(core);
    let (shutdown_tx, mut shutdown_rx) = tokio::sync::watch::channel(false);

    let join = tauri::async_runtime::spawn(async move {
        let _ = axum::serve(listener, router)
            .with_graceful_shutdown(async move {
                let _ = shutdown_rx.changed().await;
            })
            .await;
    });
    // Keep the handle alive without blocking state access.
    tauri::async_runtime::spawn(async move {
        let _ = join.await;
    });

    let info = BackendInfo {
        ready: true,
        base_url: Some(format!("http://{addr}")),
        token: Some(token),
        cwd: Some(cwd.to_string_lossy().into_owned()),
        boot_error: None,
        recent_workspaces: Vec::new(),
    };
    Ok(Running {
        info,
        backend: Backend {
            shutdown: shutdown_tx,
        },
    })
}

fn install_backend(
    app: &AppHandle,
    state: &BackendState,
    mut running: Running,
    persist: bool,
) -> BackendInfo {
    let cwd = running.info.cwd.clone().unwrap_or_default();
    if persist {
        save_project(&cwd);
    }
    running.info.recent_workspaces = recent_workspaces();
    let info = running.info.clone();
    let previous = state
        .running
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .replace(running);
    if let Some(previous) = previous {
        let _ = previous.backend.shutdown.send(true);
    }
    let _ = app.emit("backend-ready", &info);
    info
}

/// Start the embedded backend for `cwd`.
///
/// `trust` is the operator's answer when they have just been asked, and
/// `None` when nobody is being asked — reopening a remembered project, for
/// instance — in which case the decision already on record governs.
/// Opening safely is the *absence* of a decision and is the default, so
/// there is nothing to record for it.
async fn start_project_backend(
    app: AppHandle,
    state: &BackendState,
    cwd: String,
    persist: bool,
    trust: Option<bool>,
) -> Result<BackendInfo, String> {
    let path = PathBuf::from(&cwd);
    let path = match path.canonicalize() {
        Ok(path) if path.is_dir() => path,
        _ => {
            let msg = format!("not a directory: {cwd}");
            set_boot_error(state, Some(msg.clone()));
            return Err(msg);
        }
    };
    let _switch = state.switching.lock().await;
    let canonical = path.to_string_lossy().into_owned();
    if let Some(info) = state
        .running
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .as_ref()
        .filter(|running| running.info.cwd.as_deref() == Some(canonical.as_str()))
        .map(|running| running.info.clone())
    {
        return Ok(info);
    }
    let previous_cwd = state
        .running
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .as_ref()
        .and_then(|running| running.info.cwd.clone());
    if trust == Some(true)
        && let Err(e) = vak_core::trust::record(&path)
    {
        eprintln!("warning: could not record the trust decision: {e}");
    }
    let trusted = trust.unwrap_or_else(|| vak_core::trust::is_trusted(&path));
    load_workspace_env(&path, trusted);
    match boot_backend(path, trusted).await {
        Ok(running) => {
            let info = install_backend(&app, state, running, persist);
            set_boot_error(state, None);
            Ok(info)
        }
        Err(e) => {
            if let Some(previous_cwd) = previous_cwd {
                let previous = std::path::Path::new(&previous_cwd);
                load_workspace_env(previous, vak_core::trust::is_trusted(previous));
            } else {
                vak_config::replace_env_files(&[vak_home().join(".env").as_path()]);
            }
            set_boot_error(state, Some(e.clone()));
            Err(e)
        }
    }
}

#[tauri::command]
async fn start_backend(
    app: AppHandle,
    state: State<'_, BackendState>,
    cwd: String,
    trust: Option<bool>,
) -> Result<BackendInfo, String> {
    start_project_backend(app, &state, cwd, true, trust).await
}

fn set_boot_error(state: &BackendState, error: Option<String>) {
    if let Ok(mut guard) = state.running.lock() {
        if error.is_some() && guard.as_ref().is_some_and(|r| r.info.ready) {
            return; // a live backend outranks a stale failure note
        }
        if let Some(running) = guard.as_mut() {
            running.info.boot_error = error;
        } else if let Some(err) = error {
            // No live backend yet: remember the failure so the gate can
            // render it instead of spinning forever.
            *guard = Some(Running {
                info: BackendInfo {
                    ready: false,
                    boot_error: Some(err),
                    ..BackendInfo::default()
                },
                backend: Backend {
                    shutdown: tokio::sync::watch::channel(true).0,
                },
            });
        }
    }
}

/// Persist an exported document (e.g. a session transcript) to a path the
/// user explicitly chose in a native save dialog. The webview has no fs
/// plugin, so this is the one sanctioned write-out path; content arrives
/// from the embedded router, not from ambient state.
#[tauri::command]
async fn export_text_file(path: String, contents: String) -> Result<usize, String> {
    tokio::fs::write(&path, contents.as_bytes())
        .await
        .map(|_| contents.len())
        .map_err(|e| format!("could not write {path}: {e}"))
}

/// What a folder would ask for, before anything opens it.
///
/// The same facts `POST /onboarding/workspace-review` reports, but reached
/// without a server: this runs *before* a backend exists, which is exactly
/// when the operator needs to decide. Reads section headers as text and
/// never through the config loader — describing a project's privileged
/// settings by parsing them normally would activate the very thing being
/// asked about.
#[derive(serde::Serialize)]
struct WorkspaceReview {
    path: String,
    git: bool,
    requests_privilege: bool,
    privileges: Vec<&'static str>,
    trusted: bool,
}

#[tauri::command]
fn review_workspace(cwd: String) -> WorkspaceReview {
    let path = std::path::PathBuf::from(&cwd);
    WorkspaceReview {
        git: path.join(".git").exists(),
        requests_privilege: vak_core::trust::requests_privilege(&path),
        privileges: vak_core::trust::requested_privileges(&path),
        trusted: vak_core::trust::is_trusted(&path),
        path: cwd,
    }
}

/// Open the admin console at a route, from the UI.
///
/// Chat bots are created and credentialed there, not in desktop Settings:
/// a credential belongs to a bot, and several bots can share a transport
/// (AGENTS.md invariant 23), so a per-surface field here could only ever
/// describe one of them. One place owns that, and this is how the desktop
/// hands the operator over to it.
#[tauri::command]
fn open_admin(route: String) {
    // Route is chosen by our own UI, never by remote content; the admin
    // fragment is appended to a loopback URL built here.
    open_web_path(&format!("/admin{route}"));
}

#[tauri::command]
fn backend_info(state: State<'_, BackendState>) -> BackendInfo {
    let guard = state
        .running
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let mut info = guard
        .as_ref()
        .map_or_else(BackendInfo::default, |running| running.info.clone());
    info.recent_workspaces = recent_workspaces();
    info
}

fn main() {
    // Augment GUI process PATH with canonical toolchain paths so brokers and MCP servers resolve node/python/etc.
    #[allow(unsafe_code)]
    unsafe {
        std::env::set_var("PATH", vak_config::paths::augmented_process_path());
    }

    // The tray is installed before the async project/backend bootstrap. Load
    // the user environment synchronously so its service status and admin
    // links use a configured VAK_PORT from the first paint onward.
    vak_config::load_env_file(&vak_home().join(".env"));
    let internal = std::env::args_os().nth(1);
    #[cfg(target_os = "linux")]
    {
        if internal.as_deref()
            == Some(std::ffi::OsStr::new(
                vak_tools::landlock::SANDBOX_SUBCOMMAND,
            ))
        {
            std::process::exit(vak_tools::landlock::runner_main(
                std::env::args_os().skip(2),
            ));
        }
    }
    if internal.as_deref() == Some(std::ffi::OsStr::new(vak_tools::broker::WORKER_SUBCOMMAND)) {
        let runtime = match tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
        {
            Ok(runtime) => runtime,
            Err(_) => std::process::exit(125),
        };
        std::process::exit(runtime.block_on(vak_tools::broker::worker_main()));
    }
    if internal.as_deref()
        == Some(std::ffi::OsStr::new(
            vak_delivery::worker::WORKER_SUBCOMMAND,
        ))
    {
        std::process::exit(vak_delivery::worker::run_stdio());
    }
    // One line, always, before any plugin can bail out. The
    // single-instance plugin calls `std::process::exit(0)` from inside its
    // own setup when another instance already owns the socket, so without
    // this a launchd start that hands over leaves `desktop.log` completely
    // empty and indistinguishable from a job that never ran at all.
    eprintln!(
        "[vak-desktop] starting pid={} tray={} args={:?}",
        std::process::id(),
        tray_only_start(),
        std::env::args().skip(1).collect::<Vec<_>>()
    );
    let launch_project = requested_project(std::env::args_os());
    tauri::Builder::default()
        // Exactly one instance ever runs: a second launch hands its argv to
        // the live process and refocuses that window instead of starting a
        // rival shell with its own backend and its own project state.
        // Must be registered first so it can bail out before any other
        // plugin or the setup hook does work.
        // A handover from a `--tray` launch must stay silent. The
        // `com.vak.desktop` LaunchAgent fires at every login, and if the
        // app is already up (macOS reopened it, or the user never quit)
        // that agent's process hands its argv here and exits 0. Showing
        // the window then would throw one on screen at login — exactly
        // what `--tray` exists to prevent. Every human second launch
        // (Dock, Finder, `open`) still carries no flag and still reveals.
        .plugin(tauri_plugin_single_instance::init(|app, argv, _cwd| {
            if let Some(project) = requested_project(&argv) {
                let handle = app.clone();
                tauri::async_runtime::spawn(async move {
                    let state = handle.state::<BackendState>();
                    if let Err(e) = start_project_backend(
                        handle.clone(),
                        &state,
                        project.to_string_lossy().into_owned(),
                        true,
                        None,
                    )
                    .await
                    {
                        eprintln!("project launch failed: {e}");
                    }
                });
            }
            if !is_tray_launch(&argv) {
                show_main_window(app);
            }
        }))
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_notification::init())
        .manage(BackendState {
            running: Mutex::new(None),
            switching: tokio::sync::Mutex::new(()),
        })
        .manage(TrayState {
            watchdog: Arc::new(AtomicBool::new(load_watchdog())),
            last_rendered: Mutex::new(None),
        })
        .manage(pty::PtyMap::default())
        .setup(move |app| {
            install_tray(app)?;
            // The window ships hidden so a `--tray` login launch never
            // flashes one; an ordinary launch reveals it right here.
            if !tray_only_start() {
                show_main_window(app.handle());
            }
            let handle = app.handle().clone();
            tauri::async_runtime::spawn(async move {
                // Same secret-loading contract as the CLI: user-level
                // .env always; the picked workspace's own .env too (the
                // folder was explicitly chosen, so it is trusted).
                vak_config::replace_env_files(&[vak_home().join(".env").as_path()]);
                let explicit_project = launch_project;
                if let Some(cwd) = explicit_project.clone().or_else(last_project) {
                    let state = handle.state::<BackendState>();
                    if let Err(e) = start_project_backend(
                        handle.clone(),
                        &state,
                        cwd.to_string_lossy().into_owned(),
                        explicit_project.is_some(),
                        None,
                    )
                    .await
                    {
                        eprintln!("backend boot failed: {e}");
                        // Surface it to the project gate; a bundled app has
                        // no stderr to show.
                        set_boot_error(&state, Some(e));
                    }
                }
            });
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            backend_info,
            open_admin,
            review_workspace,
            start_backend,
            export_text_file,
            pty::spawn_pty,
            pty::pty_write,
            pty::pty_resize,
            pty::pty_close
        ])
        .build(tauri::generate_context!())
        .map(|app| {
            app.run(|app, event| match event {
                #[cfg(target_os = "macos")]
                RunEvent::Reopen { .. } => show_main_window(app),
                RunEvent::WindowEvent {
                    label,
                    event: WindowEvent::CloseRequested { api, .. },
                    ..
                } if label == "main" => {
                    api.prevent_close();
                    if let Some(window) = app.get_webview_window("main") {
                        let _ = window.hide();
                    }
                }
                _ => {}
            })
        })
        .inspect_err(|e| eprintln!("fatal: {e}"))
        .ok();
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::{TRAY_FLAG, is_tray_launch, requested_project};

    /// The single-instance plugin hands a second process's whole argv
    /// (program name included) to the live app. A login launch from the
    /// `com.vak.desktop` LaunchAgent carries `--tray` and must not be
    /// mistaken for a user asking for the window.
    #[test]
    fn tray_flag_is_recognised_in_a_handed_over_argv() {
        assert!(is_tray_launch([
            "/Applications/Vak.app/Contents/MacOS/vak-desktop",
            TRAY_FLAG
        ]));
        assert!(is_tray_launch(["vak-desktop", "--tray"]));
    }

    /// Dock, Finder and `open` launches carry no flag; those are the ones
    /// that must reveal the window.
    #[test]
    fn an_ordinary_launch_is_not_a_tray_launch() {
        assert!(!is_tray_launch(["vak-desktop"]));
        assert!(!is_tray_launch(Vec::<String>::new()));
        assert!(
            !is_tray_launch(["vak-desktop", "--traypad"]),
            "the flag must match whole arguments, not prefixes"
        );
    }

    #[test]
    fn project_argument_is_selected_without_treating_flags_as_paths() {
        let project = std::env::current_dir().expect("test has a current directory");
        let project_text = project.to_string_lossy().into_owned();
        assert_eq!(
            requested_project(["vak-desktop", "--tray", project_text.as_str()]),
            Some(project)
        );
    }
}
