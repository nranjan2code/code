//! vak-terminal: Rich modern terminal surface for vak (`vak term`).
//! Implements docs/design/55-rich-terminal-surface.md.

pub mod app;
pub mod graphics;
pub mod hil;
pub mod repl;
pub mod telemetry;
pub mod theme;
pub mod ui;

use std::io::stdout;
use std::time::{Duration, Instant};

use crossterm::event::{
    DisableMouseCapture, EnableMouseCapture, Event, EventStream,
};
use crossterm::execute;
use crossterm::terminal::{
    EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode, enable_raw_mode,
};
use futures::StreamExt;
use ratatui::backend::CrosstermBackend;
use ratatui::Terminal;

use app::TerminalApp;

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

/// Run the rich terminal surface.
pub async fn run_terminal(opts: TerminalOptions) -> std::io::Result<i32> {
    let _guard = TerminalGuard::enter()?;
    let backend = CrosstermBackend::new(stdout());
    let mut terminal = Terminal::new(backend)?;
    terminal.clear()?;

    let session_name = opts.session_id.unwrap_or_else(|| "s-2026-react".into());
    let host_endpoint = opts.server_url.unwrap_or_else(|| "local:8901".into());
    let model_name = "claude-3-7-sonnet";

    let mut app = TerminalApp::new(session_name, model_name, host_endpoint);
    if let Some(ref cwd) = opts.workspace_cwd {
        if let Some(name) = cwd.file_name().and_then(|n| n.to_str()) {
            app = app.with_workspace(name);
        }
    }

    let mut event_stream = EventStream::new();
    let tick_rate = Duration::from_millis(50);
    let mut last_tick = Instant::now();

    loop {
        terminal.draw(|f| {
            f.render_widget(&app, f.area());
        })?;

        let timeout = tick_rate.saturating_sub(last_tick.elapsed());

        tokio::select! {
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
                            // Terminal resize automatically handles buffer redraw
                        }
                        _ => {}
                    }
                }
            }
            _ = tokio::time::sleep(timeout) => {
                app.on_tick();
                last_tick = Instant::now();
            }
        }
    }

    Ok(0)
}
