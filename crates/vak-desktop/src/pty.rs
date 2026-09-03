//! Integrated terminal: one real PTY per pane, streamed to the webview over
//! a Tauri IPC channel. Raw ANSI bytes pass through untouched; xterm.js does
//! the rendering.

use std::collections::HashMap;
use std::io::{Read, Write};
use std::sync::Mutex;

use portable_pty::{CommandBuilder, MasterPty, PtySize, native_pty_system};
use tauri::ipc::Channel;
use tauri::{AppHandle, Emitter, Manager, State};

pub struct PtyEntry {
    writer: Box<dyn Write + Send>,
    master: Box<dyn MasterPty + Send>,
    /// A killer cloned off the child *before* the child itself was moved
    /// into the reaper thread below — `portable_pty::Child::clone_killer`
    /// exists precisely so a shell can be killed from a thread that does
    /// not own the `Child` (whose `wait()` the reaper thread is blocked
    /// in). Used by `pty_close` so closing a pane actually ends its shell
    /// rather than relying on dropping the master to deliver a hangup the
    /// shell may or may not honor.
    killer: Box<dyn portable_pty::ChildKiller + Send + Sync>,
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
    let killer = child.clone_killer();

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
                killer,
            },
        );

    // Reader thread: forward raw bytes to the webview until EOF.
    //
    // EOF here means the shell side of the pty is gone — either the user
    // typed `exit`, or `pty_close` tore it down. Either way the map entry
    // must go too: previously nothing ever removed it, so every pty this
    // process ever opened (one per session per terminal-pane mount) stayed
    // in `PtyMap` — and its writer/master fds open — for the life of the
    // app.
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
        if let Some(map) = exit_app.try_state::<PtyMap>() {
            map.0
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .remove(&exit_id);
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

/// End one pty: kill its shell and drop its writer/master (closing the fds
/// and, once the reader thread observes EOF, removing this same entry a
/// second time — a no-op, since `HashMap::remove` on an absent key is
/// harmless). Called when a terminal pane unmounts (session switch, dock
/// close) so a pty's lifetime is scoped to the pane that opened it instead
/// of to the whole app process.
#[tauri::command]
pub fn pty_close(map: State<'_, PtyMap>, id: String) -> Result<(), String> {
    let entry = map
        .0
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .remove(&id);
    if let Some(mut entry) = entry {
        let _ = entry.killer.kill();
    }
    Ok(())
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
