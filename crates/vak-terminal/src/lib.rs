//! vak-terminal: Rich modern terminal surface for vak (`vak term`).
//! Implements docs/design/55-rich-terminal-surface.md.
//!
//! Unlike the previous presentation prototype, this surface connects to a
//! real vak server over HTTP/SSE. Every rendered value — health, sessions,
//! model routes, MCP inventory, approval state, telemetry — is fetched
//! live from the server and kept fresh by background SSE watchers.

pub mod api;
pub mod app;
pub mod graphics;
pub mod hil;
pub mod repl;
pub mod telemetry;
pub mod theme;
pub mod ui;

use std::io::stdout;
use std::sync::Arc;
use std::time::{Duration, Instant};

use crossterm::event::{DisableMouseCapture, EnableMouseCapture, Event, EventStream};
use crossterm::execute;
use crossterm::terminal::{
    EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode, enable_raw_mode,
};
use futures::StreamExt;
use ratatui::Terminal;
use ratatui::backend::CrosstermBackend;
use tokio::sync::mpsc;

use crate::api::{ApiClient, TerminalEvent};
use crate::app::TerminalApp;

/// Error returned when the terminal cannot establish a connection to the
/// server at startup.
#[derive(Debug)]
pub struct ConnectionError {
    pub message: String,
    pub server_url: String,
}

/// RAII Terminal Mode guard to ensure raw mode and alternate screen
/// are ALWAYS restored even on panic or error (Invariant 6 & Rule 15).
pub struct TerminalGuard;

impl TerminalGuard {
    pub fn enter() -> std::io::Result<Self> {
        enable_raw_mode()?;
        execute!(stdout(), EnterAlternateScreen, EnableMouseCapture)?;
        Ok(Self)
    }
}

impl Drop for TerminalGuard {
    fn drop(&mut self) {
        let _ = execute!(stdout(), DisableMouseCapture, LeaveAlternateScreen);
        let _ = disable_raw_mode();
    }
}

#[derive(Debug, Clone)]
pub struct TerminalOptions {
    pub session_id: Option<String>,
    pub server_url: Option<String>,
    pub token: Option<String>,
    pub workspace_cwd: Option<std::path::PathBuf>,
}

/// Run the rich terminal surface, connected to a live vak server.
///
/// The `--server`, `--token`, and `--session` flags are all required for a
/// real connection.  If no server URL is supplied, the terminal falls back
/// to the local loopback default (`http://127.0.0.1:8901`) so that a
/// locally-started `vak serve` works out of the box.
pub async fn run_terminal(opts: TerminalOptions) -> std::io::Result<i32> {
    let _guard = TerminalGuard::enter()?;
    let backend = CrosstermBackend::new(stdout());
    let mut terminal = Terminal::new(backend)?;
    terminal.clear()?;

    // --- Resolve server connection parameters ---
    let server_url = opts
        .server_url
        .unwrap_or_else(|| "http://127.0.0.1:8901".to_string());
    let token = opts.token.unwrap_or_default();

    // --- Create the real API client ---
    let api = Arc::new(ApiClient::new(server_url, &token));

    // --- Fetch initial health — required to proceed ---
    let health = match api.health().await {
        Ok(h) => h,
        Err(e) => {
            eprintln!(
                "vak term: cannot connect to server at {}: {}",
                api.base_url(),
                e
            );
            eprintln!("  Is the vak server running? Try `vak serve` first.");
            return Err(std::io::Error::other(format!("connection error: {e}")));
        }
    };

    // --- Resolve session (attach, create, or pick from list) ---
    let session_id = if let Some(ref sid) = opts.session_id {
        sid.clone()
    } else {
        // Try to find a running session, or create one.
        match api.list_sessions().await {
            Ok(sessions) => {
                if let Some(s) = sessions
                    .iter()
                    .find(|s| s.running.unwrap_or(false) && !s.archived.unwrap_or(false))
                {
                    s.session_id.clone()
                } else {
                    match api.create_session().await {
                        Ok(id) => id,
                        Err(e) => {
                            eprintln!("vak term: failed to create session: {e}");
                            return Err(std::io::Error::other(e.to_string()));
                        }
                    }
                }
            }
            Err(_) => match api.create_session().await {
                Ok(id) => id,
                Err(e) => {
                    eprintln!("vak term: failed to create session: {e}");
                    return Err(std::io::Error::other(e.to_string()));
                }
            },
        }
    };

    // Attach to the session (marks it as the active terminal consumer).
    if let Err(e) = api.attach_session(&session_id).await {
        eprintln!("vak term: warning: could not attach to session {session_id}: {e}");
    }

    // --- Fetch initial sessions list ---
    let sessions = api.list_sessions().await.unwrap_or_default();

    // --- Fetch initial providers/model list ---
    let providers = api.list_providers().await.unwrap_or_default();
    let models = if !providers.providers.is_empty() {
        let first = &providers.providers[0];
        api.discover_models(&first.name).await.unwrap_or_default()
    } else {
        Vec::new()
    };

    // --- Fetch initial MCP inventory ---
    let mcp_servers = api.get_mcp_servers().await.unwrap_or_default();

    // --- Fetch initial gateway/approval policy ---
    let gateway = api.get_gateway_approvals().await.unwrap_or_default();

    // --- Fetch initial config snapshot ---
    let config = api.get_config().await.unwrap_or_default();

    // --- Fetch initial control state ---
    let control = api.get_control_state(&session_id).await.unwrap_or_default();

    // --- Fetch initial launch servers (for the preview URL) ---
    let launch_servers = api
        .get_launch_servers(&session_id)
        .await
        .unwrap_or_default();

    // --- Set up SSE background watchers ---
    let (tx, mut rx) = mpsc::unbounded_channel::<TerminalEvent>();

    let agent_handle = api.spawn_agent_event_watcher(&session_id, tx.clone());
    let _present_handle = api.spawn_presentation_watcher(&session_id, tx.clone());
    let health_handle = api.clone().spawn_health_watcher(tx.clone());

    // --- Build the app with real data ---
    let mut app = TerminalApp::new_with_api(
        session_id,
        health.model.clone(),
        health.clone(),
        api.clone(),
    )
    .with_workspace(
        opts.workspace_cwd
            .and_then(|p| p.file_name().and_then(|n| n.to_str()).map(String::from))
            .unwrap_or_else(|| "default".into()),
    )
    .with_providers(providers)
    .with_models(models)
    .with_mcp_servers(mcp_servers)
    .with_gateway(gateway)
    .with_config(config)
    .with_control_state(control)
    .with_launch_servers(launch_servers)
    .with_sessions(sessions);

    let tick_rate = Duration::from_millis(50);
    let mut last_tick = Instant::now();

    let mut event_stream = EventStream::new();

    // --- Main event loop: crossterm input + SSE-driven updates ---
    loop {
        terminal.draw(|f| {
            f.render_widget(&app, f.area());
        })?;

        let timeout = tick_rate.saturating_sub(last_tick.elapsed());

        tokio::select! {
            // 1. Incoming SSE / health events from background tasks.
            maybe_terminal_ev = rx.recv() => {
                if let Some(ev) = maybe_terminal_ev {
                    app.handle_terminal_event(ev);
                }
            }

            // 2. crossterm keyboard / mouse input.
            maybe_event = event_stream.next() => {
                if let Some(Ok(event)) = maybe_event {
                    match event {
                        Event::Key(key) => {
                            app.handle_key(key);
                            if app.should_quit || app.should_detach {
                                break;
                            }
                        }
                        Event::Mouse(mouse) => {
                            app.handle_mouse(mouse);
                        }
                        Event::Resize(_, _) => {
                            // Terminal resize automatically handles buffer redraw.
                        }
                        _ => {}
                    }
                }
            }

            // 3. Periodic tick for animations.
            _ = tokio::time::sleep(timeout) => {
                app.on_tick();
                last_tick = Instant::now();
            }
        }
    }

    // Clean up background tasks.
    agent_handle.abort();
    health_handle.abort();

    Ok(0)
}
