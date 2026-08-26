#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

//! vak-desktop: native shell around the same HTTP+SSE contract every other
//! surface (tui/exec/serve) speaks. The webview gets a loopback bearer token
//! for the embedded `vak-server` router; nothing about the agent protocol is
//! re-implemented here.

mod pty;

use std::path::PathBuf;
use std::sync::Mutex;

use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Emitter, Manager, State};

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
    recent_projects: Vec<String>,
}

#[derive(Deserialize, Serialize, Default)]
#[serde(default)]
struct DesktopPrefs {
    last_project: Option<String>,
    recent_projects: Vec<String>,
}

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

fn load_workspace_env(cwd: &std::path::Path) {
    let user = vak_home().join(".env");
    let project = cwd.join(".env");
    vak_config::replace_env_files(&[user.as_path(), project.as_path()]);
}

/// Boot the embedded agent server on an ephemeral loopback port.
///
/// Trust note: the user picked this folder explicitly in-app, so its project
/// config is trusted — mirroring an interactive CLI session.
async fn boot_backend(cwd: PathBuf) -> Result<Running, String> {
    let core = vak_core::Core::new_with_trust(cwd.clone(), true).map_err(|e| e.to_string())?;

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
        recent_projects: Vec::new(),
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
    let previous_cwd = state
        .running
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .as_ref()
        .and_then(|running| running.info.cwd.clone());
    // The freshly picked workspace is trusted; its .env joins the process
    // env table (real environment variables keep precedence).
    load_workspace_env(&path);
    match boot_backend(path).await {
        Ok(running) => {
            let info = install_backend(&app, &state, running, true);
            set_boot_error(&state, None);
            Ok(info)
        }
        Err(e) => {
            if let Some(previous_cwd) = previous_cwd {
                load_workspace_env(std::path::Path::new(&previous_cwd));
            } else {
                vak_config::replace_env_files(&[vak_home().join(".env").as_path()]);
            }
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
                    ..BackendInfo::default()
                },
                backend: Backend {
                    shutdown: tokio::sync::watch::channel(true).0,
                },
            });
        }
    }
}

/// Thin proxy for appending to the global USER.md memory tier: the embedded
/// router exposes list/forget/amend but no append, and memory stores are
/// plain hand-editable markdown by design (docs/design/29-personal-os.md
/// P1), so this mirrors what `vakcoder memory add --profile` does locally.
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
async fn append_profile_note(draft: ProfileNoteDraft) -> Result<ProfileNoteCreated, String> {
    tauri::async_runtime::spawn_blocking(move || {
        vak_core::memory::append_profile_note(
            &vak_home(),
            draft.kind.trim(),
            draft.tag.trim(),
            &draft.text,
            "desktop",
        )
        .map(|note| ProfileNoteCreated {
            id: note.id,
            ts: note.ts.to_rfc3339(),
        })
    })
    .await
    .map_err(|e| format!("join: {e}"))?
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
    // Canonical layout migration (doc 32): pre-0.8 dotdir → Library/XDG
    // homes. One-time rename; no-op when absent or overridden.
    if let Err(e) = vak_config::paths::migrate_legacy_home() {
        eprintln!("[warn] home migration skipped: {e}");
    }
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
                // Same secret-loading contract as the CLI: user-level
                // .env always; the picked workspace's own .env too (the
                // folder was explicitly chosen, so it is trusted).
                vak_config::replace_env_files(&[vak_home().join(".env").as_path()]);
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
                    load_workspace_env(&cwd);
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
