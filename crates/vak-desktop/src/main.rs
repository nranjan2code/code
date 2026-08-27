#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

//! vak-desktop: native shell around the same HTTP+SSE contract every other
//! surface (tui/exec/serve) speaks. It discovers a local base runtime or saved
//! remote connection and never starts a competing server or state owner.

mod pty;

use std::path::PathBuf;
use std::sync::Mutex;

use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Emitter, Manager, State};
use vak_domain::{ProjectContext, ProjectId};

struct BackendState {
    running: Mutex<Option<Running>>,
    switching: tokio::sync::Mutex<()>,
}

struct Running {
    info: BackendInfo,
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
    recent_projects: Vec<String>,
    project_id: Option<String>,
}

#[derive(Deserialize, Serialize, Default)]
#[serde(default)]
struct DesktopPrefs {
    last_project: Option<String>,
    recent_projects: Vec<String>,
}

fn vak_home() -> PathBuf {
    std::env::var_os("VAKCODER_HOME")
        .map(PathBuf::from)
        .or_else(|| {
            std::env::var_os("HOME")
                .map(|h| PathBuf::from(h).join("Library/Application Support/vakcoder"))
        })
        .unwrap_or_else(|| PathBuf::from(".vakcoder"))
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

fn recent_projects() -> Vec<String> {
    desktop_prefs()
        .recent_projects
        .into_iter()
        .filter(|path| PathBuf::from(path).is_dir())
        .collect()
}

fn save_project(cwd: &str) {
    let mut prefs = desktop_prefs();
    let previous = prefs.last_project.clone();
    prefs.last_project = Some(cwd.to_string());
    prefs
        .recent_projects
        .retain(|path| path != cwd && previous.as_deref() != Some(path));
    prefs.recent_projects.insert(0, cwd.to_string());
    if let Some(previous) = previous.filter(|path| path != cwd && PathBuf::from(path).is_dir()) {
        prefs.recent_projects.insert(1, previous);
    }
    prefs.recent_projects.truncate(8);
    let _ = std::fs::create_dir_all(vak_home());
    if let Ok(json) = serde_json::to_string(&prefs) {
        let _ = std::fs::write(prefs_path(), json);
    }
}

fn runtime_connection() -> Option<(String, String)> {
    let raw = std::fs::read_to_string(vak_home().join("runtime/gateway.json")).ok()?;
    let value: serde_json::Value = serde_json::from_str(&raw).ok()?;
    let addr = value.get("addr")?.as_str()?;
    let token = value.get("token")?.as_str()?;
    let url = if addr.starts_with("http://") || addr.starts_with("https://") {
        addr.to_string()
    } else {
        format!("http://{addr}")
    };
    Some((url, token.to_string()))
}

fn configured_connection() -> Option<(String, String)> {
    let root = std::env::var_os("VAKCODER_HOME")
        .map(PathBuf::from)
        .or_else(|| {
            std::env::var_os("HOME")
                .map(|h| PathBuf::from(h).join("Library/Application Support/vakcoder"))
        })?;
    let snapshot = vak_config2::ConfigService::new(root, std::env::current_dir().ok()?)
        .load()
        .ok()?;
    Some((snapshot.config.connect.url?, snapshot.config.connect.token?))
}

/// Connect the desktop addon to the installed or configured base. The desktop
/// owns no server, scheduler, session store, or tool worker.
async fn boot_backend(cwd: PathBuf) -> Result<Running, String> {
    let (base_url, token) = runtime_connection()
        .or_else(configured_connection)
        .ok_or_else(|| {
            "no vakcoder base found; install/start the base or configure [connect]".to_string()
        })?;
    let client =
        vak_client2::Client::new(base_url.clone(), token.clone()).map_err(|e| e.to_string())?;
    client
        .health()
        .await
        .map_err(|e| format!("base is unavailable: {e}"))?;
    let project_id = client
        .projects()
        .await
        .ok()
        .and_then(|projects| {
            projects
                .into_iter()
                .find(|p| p.root == cwd.to_string_lossy())
        })
        .map(|p| p.id.to_string());
    let project_id = match project_id {
        Some(id) => id,
        None => {
            let id = ProjectId::new();
            client
                .register_project(&ProjectContext {
                    id: id.clone(),
                    root: cwd.to_string_lossy().into_owned(),
                    display_name: None,
                })
                .await
                .map_err(|e| e.to_string())?;
            id.to_string()
        }
    };

    let info = BackendInfo {
        ready: true,
        base_url: Some(base_url),
        token: Some(token),
        cwd: Some(cwd.to_string_lossy().into_owned()),
        boot_error: None,
        recent_projects: Vec::new(),
        project_id: Some(project_id),
    };
    Ok(Running { info })
}

fn install_backend(
    app: &AppHandle,
    state: &State<'_, BackendState>,
    mut running: Running,
    persist: bool,
) -> BackendInfo {
    let cwd = running.info.cwd.clone().unwrap_or_default();
    if persist {
        save_project(&cwd);
    }
    running.info.recent_projects = recent_projects();
    let info = running.info.clone();
    state
        .running
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .replace(running);
    let _ = app.emit("backend-ready", &info);
    info
}

#[tauri::command]
async fn start_backend(
    app: AppHandle,
    state: State<'_, BackendState>,
    cwd: String,
) -> Result<BackendInfo, String> {
    let path = PathBuf::from(&cwd);
    let path = match path.canonicalize() {
        Ok(path) if path.is_dir() => path,
        _ => {
            let msg = format!("not a directory: {cwd}");
            set_boot_error(&state, Some(msg.clone()));
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
    match boot_backend(path).await {
        Ok(running) => {
            let info = install_backend(&app, &state, running, true);
            set_boot_error(&state, None);
            Ok(info)
        }
        Err(e) => {
            set_boot_error(&state, Some(e.clone()));
            Err(e)
        }
    }
}

fn set_boot_error(state: &State<'_, BackendState>, error: Option<String>) {
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
                    project_id: None,
                    ..BackendInfo::default()
                },
            });
        }
    }
}

/// Thin proxy for appending to the global USER.md memory tier. Writes go
/// through the connected base (`POST /memory`, scope=profile) like every
/// other surface — the desktop never touches memory stores directly
/// (docs/design/34-base-addons.md ownership contract).
#[derive(Deserialize)]
struct ProfileNoteDraft {
    kind: String,
    #[serde(default)]
    tag: String,
    text: String,
}

#[derive(Serialize)]
struct ProfileNoteCreated {
    id: String,
    ts: String,
}

#[tauri::command]
async fn append_profile_note(
    state: State<'_, BackendState>,
    draft: ProfileNoteDraft,
) -> Result<ProfileNoteCreated, String> {
    let _ = (state, &draft.kind, &draft.tag, &draft.text);
    Err("profile memory is unavailable until the Runtime memory command is exposed".into())
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

#[tauri::command]
fn backend_info(state: State<'_, BackendState>) -> BackendInfo {
    let guard = state
        .running
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let mut info = guard
        .as_ref()
        .map_or_else(BackendInfo::default, |running| running.info.clone());
    info.recent_projects = recent_projects();
    info
}

fn main() {
    tauri::Builder::default()
        // Exactly one instance ever runs: a second launch hands its argv to
        // the live process and refocuses that window instead of starting a
        // rival shell with its own backend and its own project state.
        // Must be registered first so it can bail out before any other
        // plugin or the setup hook does work.
        .plugin(tauri_plugin_single_instance::init(|app, _argv, _cwd| {
            if let Some(window) = app.get_webview_window("main") {
                let _ = window.unminimize();
                let _ = window.show();
                let _ = window.set_focus();
            }
        }))
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_notification::init())
        .manage(BackendState {
            running: Mutex::new(None),
            switching: tokio::sync::Mutex::new(()),
        })
        .manage(pty::PtyMap::default())
        .setup(|app| {
            let handle = app.handle().clone();
            tauri::async_runtime::spawn(async move {
                if let Some(cwd) = last_project() {
                    let state = handle.state::<BackendState>();
                    let _switch = state.switching.lock().await;
                    if state
                        .running
                        .lock()
                        .unwrap_or_else(std::sync::PoisonError::into_inner)
                        .is_some()
                    {
                        return;
                    }
                    if let Err(e) = boot_backend(cwd)
                        .await
                        .map(|running| install_backend(&handle, &state, running, false))
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
            start_backend,
            append_profile_note,
            export_text_file,
            pty::spawn_pty,
            pty::pty_write,
            pty::pty_resize
        ])
        .run(tauri::generate_context!())
        .inspect_err(|e| eprintln!("fatal: {e}"))
        .ok();
}
