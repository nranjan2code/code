#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

//! vak-desktop: native shell around the same HTTP+SSE contract every other
//! surface (tui/exec/serve) speaks. The webview gets a loopback bearer token
//! for the embedded `vak-server` router; nothing about the agent protocol is
//! re-implemented here.

mod pty;

use std::path::PathBuf;
use std::sync::Mutex;

use serde::Serialize;
use tauri::{AppHandle, Emitter, Manager, State};

#[derive(Clone)]
struct Backend {
    shutdown: tokio::sync::watch::Sender<bool>,
}

struct BackendState(Mutex<Option<Running>>);

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
}

fn vak_home() -> PathBuf {
    let home = std::env::var("HOME")
        .or_else(|_| std::env::var("USERPROFILE"))
        .unwrap_or_else(|_| ".".to_string());
    PathBuf::from(home).join(".vakcoder")
}

fn prefs_path() -> PathBuf {
    vak_home().join("desktop.json")
}

fn last_project() -> Option<PathBuf> {
    let text = std::fs::read_to_string(prefs_path()).ok()?;
    let value: serde_json::Value = serde_json::from_str(&text).ok()?;
    let cwd = PathBuf::from(value.get("last_project")?.as_str()?);
    cwd.is_dir().then_some(cwd)
}

fn save_last_project(cwd: &str) {
    let _ = std::fs::create_dir_all(vak_home());
    let _ = std::fs::write(
        prefs_path(),
        serde_json::json!({ "last_project": cwd }).to_string(),
    );
}

/// Boot the embedded agent server on an ephemeral loopback port.
///
/// Trust note: the user picked this folder explicitly in-app, so its project
/// config is trusted — mirroring an interactive CLI session.
async fn start_backend_inner(app: &AppHandle, cwd: PathBuf) -> Result<BackendInfo, String> {
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
    };
    {
        let state = app.state::<BackendState>();
        *state
            .0
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner) = Some(Running {
            info: info.clone(),
            backend: Backend {
                shutdown: shutdown_tx,
            },
        });
    }
    let _ = app.emit("backend-ready", &info);
    Ok(info)
}

#[tauri::command]
async fn start_backend(
    app: AppHandle,
    state: State<'_, BackendState>,
    cwd: String,
) -> Result<BackendInfo, String> {
    let path = PathBuf::from(&cwd);
    if !path.is_dir() {
        return Err(format!("not a directory: {cwd}"));
    }
    stop_current(&state).await;
    let info = start_backend_inner(&app, path).await?;
    save_last_project(&cwd);
    Ok(info)
}

async fn stop_current(state: &State<'_, BackendState>) {
    let taken = state
        .0
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .take();
    if let Some(running) = taken {
        let _ = running.backend.shutdown.send(true);
    }
}

#[tauri::command]
fn backend_info(state: State<'_, BackendState>) -> BackendInfo {
    let guard = state
        .0
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    guard
        .as_ref()
        .map_or_else(BackendInfo::default, |running| running.info.clone())
}

fn main() {
    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_notification::init())
        .manage(BackendState(Mutex::new(None)))
        .manage(pty::PtyMap::default())
        .setup(|app| {
            let handle = app.handle().clone();
            tauri::async_runtime::spawn(async move {
                if let Some(cwd) = last_project()
                    && let Err(e) = start_backend_inner(&handle, cwd).await
                {
                    eprintln!("backend boot failed: {e}");
                }
            });
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            backend_info,
            start_backend,
            pty::spawn_pty,
            pty::pty_write,
            pty::pty_resize
        ])
        .run(tauri::generate_context!())
        .inspect_err(|e| eprintln!("fatal: {e}"))
        .ok();
}
