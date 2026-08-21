use std::path::PathBuf;
use std::sync::Arc;

use crossterm::event::{Event, EventStream, KeyCode};
use crossterm::terminal::{disable_raw_mode, enable_raw_mode};
use futures::StreamExt;
use tokio::sync::{Mutex, mpsc, oneshot};
use tokio_util::sync::CancellationToken;

use vak_agent::{AgentEvent, Approver, SteeringQueues, TurnOutcome};
use vak_core::Core;
use vak_llm::stream::StreamEvent;
use vak_session::SessionLog;

use crate::commands::{self, Command};
use crate::editor::Editor;
use crate::keys::{Action, map_key};
use crate::render::Screen;

pub struct UiConfig {
    pub cwd: PathBuf,
}

pub struct ApprovalRequest {
    tool: String,
    reason: String,
    respond: oneshot::Sender<bool>,
}

struct ChannelApprover {
    tx: mpsc::Sender<ApprovalRequest>,
}

#[async_trait::async_trait]
impl Approver for ChannelApprover {
    async fn approve(&self, tool: &str, reason: &str) -> bool {
        let (respond, rx) = oneshot::channel();
        let req = ApprovalRequest {
            tool: tool.to_string(),
            reason: reason.to_string(),
            respond,
        };
        if self.tx.send(req).await.is_err() {
            return false;
        }
        rx.await.unwrap_or(false)
    }
}

enum RunSignal {
    Done(TurnOutcome),
}

struct RawMode;

impl RawMode {
    fn enable() -> Self {
        let _ = enable_raw_mode();
        RawMode
    }
}

impl Drop for RawMode {
    fn drop(&mut self) {
        let _ = disable_raw_mode();
    }
}

pub async fn run(core: Core, _cfg: UiConfig) -> i32 {
    let _raw = RawMode::enable();
    let mut screen = Screen::new();
    let mut editor = Editor::new();
    let steering: Arc<SteeringQueues> = Arc::new(SteeringQueues::new());
    let cancel = CancellationToken::new();

    let session = match core.start_session().await {
        Ok(s) => s,
        Err(e) => {
            eprintln!("error: {e}");
            return 2;
        }
    };
    let session_path = session.path().display().to_string();
    let session = Arc::new(Mutex::new(Some(session)));
    let (approval_tx, mut approval_rx) = mpsc::channel::<ApprovalRequest>(1);
    let approver: Arc<dyn Approver> = Arc::new(ChannelApprover { tx: approval_tx });

    screen.banner(
        vak_core::APP_VERSION,
        &core.config().model.clone(),
        &session_path,
    );

    let mut reader = EventStream::new();
    let mut running: Option<mpsc::Receiver<AgentEvent>> = None;
    let mut run_done: Option<mpsc::Receiver<RunSignal>> = None;
    let mut pending = String::new();
    let mut total_in: u64 = 0;
    let mut total_out: u64 = 0;
    let mut awaiting: Option<ApprovalRequest> = None;

    loop {
        let input_ev = reader.next();
        let agent_ev = async {
            match running.as_mut() {
                Some(rx) => rx.recv().await,
                None => std::future::pending().await,
            }
        };
        let done_ev = async {
            match run_done.as_mut() {
                Some(rx) => rx.recv().await,
                None => std::future::pending().await,
            }
        };
        let approval_ev = approval_rx.recv();

        tokio::select! {
            maybe_event = input_ev => {
                let Some(Ok(Event::Key(key))) = maybe_event else { continue };
                if key.kind != crossterm::event::KeyEventKind::Press {
                    continue;
                }

                if let Some(req) = awaiting.take() {
                    let approve = matches!(key.code, KeyCode::Char('y') | KeyCode::Char('Y'));
                    let _ = req.respond.send(approve);
                    screen.clear_input_row();
                    screen.dim(&format!(
                        "{} {}",
                        if approve { "✓ allowed" } else { "✗ denied" },
                        req.tool
                    ));
                    continue;
                }
                let is_running = running.is_some();
                match map_key(key.code, key.modifiers, is_running) {
                    Action::Insert(c) => {
                        if is_running {
                            pending.push(c);
                        } else {
                            editor.insert(c);
                        }
                    }
                    Action::Backspace => {
                        if is_running { pending.pop(); } else { editor.backspace(); }
                    }
                    Action::Delete => editor.delete(),
                    Action::Left => editor.left(),
                    Action::Right => editor.right(),
                    Action::Home => editor.home(),
                    Action::End => editor.end(),
                    Action::HistoryPrev => editor.history_prev(),
                    Action::HistoryNext => editor.history_next(),
                    Action::Submit => {
                        if is_running {
                            let text = std::mem::take(&mut pending);
                            if !text.trim().is_empty() {
                                steering.push_steering(text);
                                screen.clear_input_row();
                                screen.dim("[steering queued]");
                            }
                        } else {
                            let text = editor.take();
                            if text.trim().is_empty() { continue; }
                            screen.clear_input_row();
                            if text.trim_start().starts_with('/') {
                                match commands::parse(&text) {
                                    Some(Command::Exit) => break,
                                    Some(Command::Help) => screen.dim(&commands::help_text()),
                                    Some(Command::Cost) => {
                                        screen.dim(&format!("tokens in {total_in} / out {total_out}"));
                                    }
                                    Some(Command::Context) => show_context(&mut screen, &session).await,
                                    Some(Command::Sessions) => list_sessions(&core, &mut screen),
                                    Some(Command::Model(m)) => {
                                        core.set_model(m.clone());
                                        screen.accent(&format!("model → {m} (applies to next turn)"));
                                    }
                                    Some(Command::Clear) => {
                                        match core.start_session().await {
                                            Ok(s) => {
                                                *session.lock().await = Some(s);
                                                screen.accent("started a fresh session");
                                            }
                                            Err(e) => screen.dim(&format!("error: {e}")),
                                        }
                                    }
                                    None => screen.dim("unknown command — /help"),
                                }
                                continue;
                            }
                            submit(&core, &session, &steering, approver.clone(), cancel.clone(), &mut running, &mut run_done, &text).await;
                            screen.dim(&format!("▸ {text}"));
                        }
                    }
                    Action::CancelOrClear => {
                        if is_running {
                            cancel.cancel();
                            screen.clear_input_row();
                            screen.dim("[cancelling…]");
                        } else if !editor.is_empty() {
                            editor = Editor::new();
                        } else {
                            screen.clear_input_row();
                            break;
                        }
                    }
                    Action::Exit => {
                        screen.clear_input_row();
                        break;
                    }
                    Action::Ignore => {}
                }
            }
            Some(agent_event) = agent_ev => {
                render_agent_event(&mut screen, agent_event, &mut total_in, &mut total_out);
            }
            Some(req) = approval_ev => {
                awaiting = Some(req);
            }
            signal = done_ev => {
                if let Some(mut rx) = running.take() {
                    while let Ok(ev) = rx.try_recv() {
                        render_agent_event(&mut screen, ev, &mut total_in, &mut total_out);
                    }
                }
                run_done = None;
                if let Some(RunSignal::Done(outcome)) = signal {
                    finish_outcome(&mut screen, outcome, &session_path);
                }
                screen.line("");
            }
        }

        redraw(&mut screen, &editor, running.is_some(), &pending);
        if let Some(req) = &awaiting {
            screen.redraw_input(
                "[approve] ",
                &format!("{} — y/n: {}", req.tool, req.reason),
                0,
            );
        }
    }

    screen.dim("bye");
    0
}

#[allow(clippy::too_many_arguments)]
async fn submit(
    core: &Core,
    session: &Arc<Mutex<Option<SessionLog>>>,
    _steering: &Arc<SteeringQueues>,
    approver: Arc<dyn Approver>,
    cancel: CancellationToken,
    running: &mut Option<mpsc::Receiver<AgentEvent>>,
    run_done: &mut Option<mpsc::Receiver<RunSignal>>,
    prompt: &str,
) {
    let Some(taken) = session.lock().await.take() else {
        return;
    };
    if core.provider().is_err() {
        *session.lock().await = Some(taken);
        return;
    }
    let (ev_tx, ev_rx) = mpsc::channel(1024);
    let (done_tx, done_rx) = mpsc::channel(1);
    let prompt = prompt.to_string();
    let core = core.clone();
    tokio::spawn(async move {
        let outcome = core
            .run_turn_with(taken, &prompt, cancel, Some(approver), None, ev_tx)
            .await;
        if let Ok((o, _session)) = outcome {
            let _ = done_tx.send(RunSignal::Done(o)).await;
        }
    });
    *running = Some(ev_rx);
    *run_done = Some(done_rx);
}

fn render_agent_event(
    screen: &mut Screen,
    ev: AgentEvent,
    total_in: &mut u64,
    total_out: &mut u64,
) {
    match ev {
        AgentEvent::Stream(StreamEvent::TextDelta { delta, .. }) => screen.inline(&delta),
        AgentEvent::Stream(_) => {}
        AgentEvent::ToolCallStart { name, .. } => {
            screen.clear_input_row();
            screen.inline(&format!("  ▸ {name} "));
        }
        AgentEvent::ToolCallEnd { name, is_error, .. } => {
            screen.tool_line(if is_error { "✗" } else { "✓" }, &name);
        }
        AgentEvent::TurnEnd { usage } => {
            *total_in += usage.input_tokens;
            *total_out += usage.output_tokens;
        }
        AgentEvent::TurnStart { .. } => {}
        AgentEvent::StopHookContinuation { reason } => {
            screen.clear_input_row();
            screen.dim(&format!("[stop-hook] {reason} — continuing"));
        }
        AgentEvent::StreamOpened
        | AgentEvent::ApprovalRequested { .. }
        | AgentEvent::RunFinished { .. } => {}
    }
}

fn finish_outcome(screen: &mut Screen, outcome: TurnOutcome, session_path: &str) {
    match outcome {
        TurnOutcome::Completed { response } => {
            screen.dim(&format!(
                "\n── completed · in {} out {} · {}",
                response.usage.input_tokens, response.usage.output_tokens, session_path
            ));
        }
        TurnOutcome::Aborted { .. } => {
            screen.dim("\n── aborted (partial output preserved)");
        }
        TurnOutcome::Failed { error } => {
            screen.dim(&format!("\n── failed: {error}"));
        }
        TurnOutcome::MaxTurnsReached => {
            screen.dim("\n── stopped: max turns reached");
        }
    }
}

async fn show_context(screen: &mut Screen, session: &Arc<Mutex<Option<SessionLog>>>) {
    let guard = session.lock().await;
    let Some(s) = guard.as_ref() else {
        return;
    };
    let msgs = s.derive_messages();
    screen.dim(&format!(
        "next request: {} messages · {} total tokens so far",
        msgs.len(),
        s.total_usage().total_tokens()
    ));
}

fn list_sessions(core: &Core, screen: &mut Screen) {
    let dir = vak_session::SessionPath::sessions_dir(&core.sessions_home(), core.cwd());
    match std::fs::read_dir(&dir) {
        Ok(entries) => {
            for e in entries.flatten() {
                screen.dim(&format!("{}", e.file_name().to_string_lossy()));
            }
        }
        Err(_) => screen.dim("no sessions yet"),
    }
}

fn redraw(screen: &mut Screen, editor: &Editor, running: bool, pending: &str) {
    if running {
        let hint = if pending.is_empty() {
            "…".to_string()
        } else {
            format!("[queued] {pending}")
        };
        screen.redraw_input("· ", &hint, hint.chars().count());
    } else {
        let (buf, cur) = editor.view();
        screen.redraw_input("> ", buf, cur);
    }
}
