//! Integrated terminal: one real PTY per pane, streamed to the webview over
//! a Tauri IPC channel. Raw ANSI bytes pass through untouched; xterm.js does
//! the rendering.

use std::collections::HashMap;
use std::io::{Read, Write};
use std::sync::Mutex;

use portable_pty::{CommandBuilder, MasterPty, PtySize, native_pty_system};
use tauri::ipc::Channel;
use tauri::{AppHandle, Emitter, State};

pub struct PtyEntry {
    writer: Box<dyn Write + Send>,
    master: Box<dyn MasterPty + Send>,
}

#[derive(Default)]
pub struct PtyMap(pub Mutex<HashMap<String, PtyEntry>>);

#[tauri::command]
pub fn spawn_pty(
    app: AppHandle,
    map: State<'_, PtyMap>,
    cwd: String,
    cols: u16,
    rows: u16,
    on_data: Channel<Vec<u8>>,
) -> Result<String, String> {
    let id = uuid::Uuid::now_v7().to_string();
    let pty_system = native_pty_system();
    let pair = pty_system
        .openpty(PtySize {
            rows,
            cols,
            pixel_width: 0,
            pixel_height: 0,
        })
        .map_err(|e| format!("openpty failed: {e}"))?;

    // Default prog honors $SHELL / ComSpec like an interactive terminal.
    let mut cmd = CommandBuilder::new_default_prog();
    cmd.cwd(&cwd);
    cmd.env("TERM", "xterm-256color");

    let child = pair
        .slave
        .spawn_command(cmd)
        .map_err(|e| format!("shell spawn failed: {e}"))?;
    drop(pair.slave); // our copy is not needed once the child holds its side

    let mut reader = pair
        .master
        .try_clone_reader()
        .map_err(|e| format!("pty reader failed: {e}"))?;
    let writer = pair
        .master
        .take_writer()
        .map_err(|e| format!("pty writer failed: {e}"))?;

    map.0
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .insert(
            id.clone(),
            PtyEntry {
                writer,
                master: pair.master,
            },
        );

    // Reader thread: forward raw bytes to the webview until EOF.
    let exit_app = app.clone();
    let exit_id = id.clone();
    std::thread::spawn(move || {
        let mut buf = [0u8; 8192];
        loop {
            match reader.read(&mut buf) {
                Ok(0) => break,
                Ok(n) => {
                    if on_data.send(buf[..n].to_vec()).is_err() {
                        break;
                    }
                }
                Err(_) => break,
            }
        }
        let _ = exit_app.emit("pty-exit", &exit_id);
    });

    // Reap the child so no zombie lingers after the pane closes.
    std::thread::spawn(move || {
        let mut child = child;
        let _ = child.wait();
    });

    Ok(id)
}

#[tauri::command]
pub fn pty_write(map: State<'_, PtyMap>, id: String, data: Vec<u8>) -> Result<(), String> {
    let mut guard = map
        .0
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let Some(entry) = guard.get_mut(&id) else {
        return Err("unknown pty".into());
    };
    entry.writer.write_all(&data).map_err(|e| e.to_string())
}

#[tauri::command]
pub fn pty_resize(map: State<'_, PtyMap>, id: String, cols: u16, rows: u16) -> Result<(), String> {
    let guard = map
        .0
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let Some(entry) = guard.get(&id) else {
        return Err("unknown pty".into());
    };
    entry
        .master
        .resize(PtySize {
            rows,
            cols,
            pixel_width: 0,
            pixel_height: 0,
        })
        .map_err(|e| e.to_string())
}
