use std::collections::{HashMap, HashSet, VecDeque};
use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, Instant};

use crossterm::event::{DisableBracketedPaste, EnableBracketedPaste, Event, EventStream, KeyCode};
use crossterm::execute;
use crossterm::style::Color;
use crossterm::terminal::{disable_raw_mode, enable_raw_mode};
use futures::StreamExt;
use tokio::sync::{Mutex, mpsc, oneshot};
use tokio_util::sync::CancellationToken;

use vak_agent::{AgentEvent, Approver, SteeringQueues, TurnOutcome};
use vak_core::Core;
use vak_llm::stream::StreamEvent;
use vak_session::SessionLog;

use crate::commands::{self, Command};
use crate::complete;
use crate::diffview;
use crate::editor::Editor;
use crate::keys::{Action, map_key};
use crate::markdown::LineStyler;
use crate::render::Screen;
use crate::status;
use crate::theme::{self, Theme};

pub struct UiConfig {
    pub cwd: PathBuf,
}

pub struct ApprovalRequest {
    tool: String,
    args_json: String,
    reason: String,
    respond: oneshot::Sender<bool>,
}

struct ChannelApprover {
    tx: mpsc::Sender<ApprovalRequest>,
    allowed: Arc<Mutex<HashSet<String>>>,
}

#[async_trait::async_trait]
impl Approver for ChannelApprover {
    async fn approve(&self, tool: &str, args_json: &str, reason: &str) -> bool {
        if self.allowed.lock().await.contains(tool) {
            return true;
        }
        let (respond, rx) = oneshot::channel();
        let req = ApprovalRequest {
            tool: tool.to_string(),
            args_json: args_json.to_string(),
            reason: reason.to_string(),
            respond,
        };
        if self.tx.send(req).await.is_err() {
            return false;
        }
        rx.await.unwrap_or(false)
    }
}

struct RawMode;

impl RawMode {
    fn enable() -> Self {
        let _ = enable_raw_mode();
        let _ = execute!(std::io::stdout(), EnableBracketedPaste);
        RawMode
    }
}

impl Drop for RawMode {
    fn drop(&mut self) {
        let _ = execute!(std::io::stdout(), DisableBracketedPaste);
        let _ = disable_raw_mode();
    }
}

struct RunCtx {
    core: Core,
    session_slot: Arc<Mutex<Option<SessionLog>>>,
    steering: Arc<SteeringQueues>,
    approver: Arc<dyn Approver>,
    cancel: CancellationToken,
}

impl RunCtx {
    /// Spawns one turn. Returns None when no provider/auth is available; the
    /// session slot is always restored so later turns still work.
    async fn spawn(
        &self,
        prompt: &str,
    ) -> Option<(mpsc::Receiver<AgentEvent>, mpsc::Receiver<TurnOutcome>)> {
        let taken = self.session_slot.lock().await.take()?;
        if self.core.provider().is_err() {
            *self.session_slot.lock().await = Some(taken);
            return None;
        }
        let (ev_tx, ev_rx) = mpsc::channel(1024);
        let (done_tx, done_rx) = mpsc::channel(1);
        let prompt = prompt.to_string();
        let core = self.core.clone();
        let approver = self.approver.clone();
        let cancel = self.cancel.clone();
        let steering = self.steering.clone();
        tokio::spawn(async move {
            let outcome = match core
                .run_turn_with(
                    taken,
                    &prompt,
                    cancel,
                    Some(approver),
                    None,
                    Some(steering),
                    ev_tx,
                )
                .await
            {
                Ok((o, _)) => o,
                Err(e) => {
                    let error = match e {
                        vak_core::CoreError::Llm(l) => l,
                        other => vak_llm::LlmError::InvalidRequest(other.to_string()),
                    };
                    TurnOutcome::Failed { error }
                }
            };
            // Always release the loop back to the editor: an Err outcome must
            // still clear `running`, or every later keystroke would be
            // swallowed as steering input and the TUI could never exit.
            let _ = done_tx.send(outcome).await;
        });
        Some((ev_rx, done_rx))
    }
}

struct UiState {
    styler: LineStyler,
    partial: String,
    total_in: u64,
    total_out: u64,
    tool_args: HashMap<String, (String, String)>,
    thinking_shown: bool,
    model: String,
    cost_usd: f64,
    sub_in: u64,
    sub_out: u64,
}

pub async fn run(core: Core, _cfg: UiConfig) -> i32 {
    let _raw = RawMode::enable();
    let ui_theme_name = core.effective_theme();
    let bell_on = core.config().ui.bell;
    let context_window = core.config().context_window;
    let mut ui_theme = Theme::from_name(&ui_theme_name);
    let mut screen = Screen::new(ui_theme);
    let hist_path = core.sessions_home().join("input_history.txt");
    let mut editor = Editor::new();
    editor.load_history(&hist_path);

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
    let session_slot = Arc::new(Mutex::new(Some(session)));
    let allowed: Arc<Mutex<HashSet<String>>> = Arc::new(Mutex::new(HashSet::new()));
    let (approval_tx, mut approval_rx) = mpsc::channel::<ApprovalRequest>(16);
    let approver: Arc<dyn Approver> = Arc::new(ChannelApprover {
        tx: approval_tx,
        allowed: allowed.clone(),
    });
    let ctx = RunCtx {
        core: core.clone(),
        session_slot: session_slot.clone(),
        steering: steering.clone(),
        approver,
        cancel: cancel.clone(),
    };

    screen.set_title(&format!("vakcoder · {}", core.effective_model()));
    screen.banner(
        vak_core::APP_VERSION,
        &core.config().model.clone(),
        &session_path,
    );

    let mut reader = EventStream::new();
    let mut running: Option<mpsc::Receiver<AgentEvent>> = None;
    let mut run_done: Option<mpsc::Receiver<TurnOutcome>> = None;
    let mut pending = String::new();
    let mut ui = UiState {
        styler: LineStyler::new(),
        partial: String::new(),
        total_in: 0,
        total_out: 0,
        tool_args: HashMap::new(),
        thinking_shown: false,
        model: core.effective_model(),
        cost_usd: 0.0,
        sub_in: 0,
        sub_out: 0,
    };
    let mut approvals: VecDeque<ApprovalRequest> = VecDeque::new();
    let mut run_started: Option<Instant> = None;
    let mut tick_count: usize = 0;

    loop {
        let running_now = running.is_some();
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
        let tick_delay = if running_now {
            Duration::from_millis(200)
        } else {
            Duration::from_secs(3600)
        };

        tokio::select! {
            maybe_event = input_ev => {
                match maybe_event {
                    Some(Ok(Event::Key(key))) => {
                        if key.kind != crossterm::event::KeyEventKind::Press {
                            continue;
                        }
                        if let Some(req) = approvals.pop_front() {
                            match key.code {
                                KeyCode::Char('y' | 'Y') => {
                                    let _ = req.respond.send(true);
                                    screen.clear_input();
                                    screen.success(&format!("✓ allowed {}", req.tool));
                                }
                                KeyCode::Char('a' | 'A') => {
                                    allowed.lock().await.insert(req.tool.clone());
                                    let _ = req.respond.send(true);
                                    screen.clear_input();
                                    screen.accent(&format!(
                                        "✓ {} allowed for this session",
                                        req.tool
                                    ));
                                }
                                KeyCode::Char('p' | 'P') => match learned_spec(
                                    &req.tool,
                                    &req.args_json,
                                ) {
                                    Some(spec) => match core.learn_allow_rule(&spec) {
                                        Ok(()) => {
                                            allowed.lock().await.insert(req.tool.clone());
                                            let _ = req.respond.send(true);
                                            screen.clear_input();
                                            screen.accent(&format!(
                                                "✓ saved {} → {}",
                                                spec,
                                                vak_core::PERMISSIONS_LOCAL_FILE
                                            ));
                                        }
                                        Err(e) => {
                                            approvals.push_front(req);
                                            screen.clear_input();
                                            screen.error(&format!("save failed: {e}"));
                                            continue;
                                        }
                                    },
                                    None => {
                                        approvals.push_front(req);
                                        screen.clear_input();
                                        screen.dim(
                                            "cannot scope this call safely — [a] for session-only, [n] deny",
                                        );
                                        continue;
                                    }
                                },
                                KeyCode::Char('n' | 'N' | 'q' | 'Q') | KeyCode::Esc => {
                                    let _ = req.respond.send(false);
                                    screen.clear_input();
                                    screen.error(&format!("✗ denied {}", req.tool));
                                }
                                // Stray keys never answer a pending approval:
                                // they are ignored outright instead of leaking
                                // into the editor or silently denying.
                                _ => {
                                    approvals.push_front(req);
                                    continue;
                                }
                            }
                            if let Some(next) = approvals.front() {
                                draw_approval(&mut screen, &ui_theme, next);
                            }
                            continue;
                        }
                        let is_running = running.is_some();
                        if editor.search_active() && !is_running {
                            match key.code {
                                KeyCode::Char('r') if key.modifiers.contains(crossterm::event::KeyModifiers::CONTROL) => {
                                    editor.search_next();
                                }
                                KeyCode::Esc => editor.cancel_search(),
                                KeyCode::Enter => editor.accept_search(),
                                KeyCode::Backspace => editor.search_backspace(),
                                KeyCode::Char(c)
                                    if key.modifiers.is_empty()
                                        || key.modifiers == crossterm::event::KeyModifiers::SHIFT =>
                                {
                                    editor.search_push(c);
                                }
                                _ => {}
                            }
                            continue;
                        }
                        match map_key(key.code, key.modifiers, is_running) {
                            Action::Insert(c) => {
                                if is_running {
                                    pending.push(c);
                                } else {
                                    editor.insert(c);
                                }
                            }
                            Action::Backspace => {
                                if is_running {
                                    let _ = pending.pop();
                                } else {
                                    editor.backspace();
                                }
                            }
                            Action::Delete => editor.delete(),
                            Action::Left => editor.left(),
                            Action::Right => editor.right(),
                            Action::WordLeft => editor.word_left(),
                            Action::WordRight => editor.word_right(),
                            Action::DeleteWordBack => editor.delete_word_back(),
                            Action::ClearLine => {
                                if is_running {
                                    pending.clear();
                                } else {
                                    editor.clear();
                                }
                            }
                            Action::Home => editor.home(),
                            Action::End => editor.end(),
                            Action::HistoryPrev => editor.history_prev(),
                            Action::HistoryNext => editor.history_next(),
                            Action::HistorySearch => {
                                if !is_running {
                                    editor.begin_search();
                                }
                            }
                            Action::Complete => {
                                if !is_running {
                                    handle_complete(&mut editor, &core, &mut screen);
                                }
                            }
                            Action::Submit => {
                                if is_running {
                                    let text = std::mem::take(&mut pending);
                                    if !text.trim().is_empty() {
                                        steering.push_steering(text);
                                        screen.clear_input();
                                        screen.dim("[steering queued]");
                                    }
                                } else {
                                    let text = editor.take();
                                    if text.trim().is_empty() {
                                        continue;
                                    }
                                    editor.save_history(&hist_path);
                                    screen.clear_input();
                                    if text.trim_start().starts_with('/') {
                                        match commands::parse(&text) {
                                            Some(Command::Exit) => break,
                                            Some(Command::Help) => {
                                                screen.dim(&commands::help_text());
                                            }
                                                Some(Command::Cost) => {
                                                    let cost = crate::pricing::session_cost(
                                                        &ui.model,
                                                        ui.total_in,
                                                        ui.total_out,
                                                    );
                                                    let sub_cost = crate::pricing::session_cost(
                                                        &ui.model,
                                                        ui.sub_in,
                                                        ui.sub_out,
                                                    );
                                                    let dollars = match (cost, sub_cost) {
                                                        (Some(c), Some(s)) => {
                                                            format!(
                                                                " · ~{}",
                                                                crate::pricing::format_cost(c + s)
                                                            )
                                                        }
                                                        _ => format!(
                                                            " (no pricing for {})",
                                                            ui.model
                                                        ),
                                                    };
                                                    screen.dim(&format!(
                                                        "tokens in {} / out {}{dollars}",
                                                        status::fmt_tokens(ui.total_in),
                                                        status::fmt_tokens(ui.total_out),
                                                    ));
                                                    if ui.sub_in > 0 || ui.sub_out > 0 {
                                                        let s = sub_cost
                                                            .unwrap_or(0.0);
                                                        let suffix = if sub_cost.is_some() {
                                                            format!(" · ~{}", crate::pricing::format_cost(s))
                                                        } else {
                                                            String::new()
                                                        };
                                                        screen.dim(&format!(
                                                            "  subagents: in {} / out {}{suffix}",
                                                            status::fmt_tokens(ui.sub_in),
                                                            status::fmt_tokens(ui.sub_out),
                                                        ));
                                                    }
                                                }
                                            Some(Command::Context) => show_context(
                                                &mut screen, &session_slot, context_window,
                                            )
                                            .await,
                                            Some(Command::Sessions) => {
                                                list_sessions(&core, &mut screen);
                                            }
                                            Some(Command::Resume(arg)) => {
                                                cmd_resume(&core, &session_slot, arg, &mut screen).await;
                                            }
                                                Some(Command::Rewind(arg)) => {
                                                    cmd_rewind(&core, &session_slot, arg, &mut screen).await;
                                                }
                                                Some(Command::Theme(arg)) => match arg {
                                                    None => screen.dim(&format!(
                                                        "themes: {} (current: {})",
                                                        crate::theme::names().join(", "),
                                                        core.effective_theme(),
                                                    )),
                                                    Some(name)
                                                        if crate::theme::names()
                                                            .contains(&name.as_str()) =>
                                                    {
                                                        core.set_theme(name.clone());
                                                        ui_theme = Theme::from_name(&name);
                                                        screen.set_theme(ui_theme);
                                                        screen.accent(&format!("theme → {name}"));
                                                    }
                                                    Some(other) => screen.dim(&format!(
                                                        "unknown theme '{other}' — {}",
                                                        crate::theme::names().join(", "),
                                                    )),
                                                },
                                                Some(Command::Transcript(arg)) => {
                                                    show_transcript(
                                                        &mut screen,
                                                        &session_slot,
                                                        arg.as_deref(),
                                                        &ui_theme,
                                                    )
                                                    .await;
                                                }
                                                Some(Command::Doctor) => {
                                                    run_doctor(&core, &mut screen);
                                                }
                                            Some(Command::Model(m)) => {
                                                core.set_model(m.clone());
                                                screen.accent(&format!(
                                                    "model → {m} (applies to next turn)"
                                                ));
                                            }
                                            Some(Command::Clear) => match core.start_session().await {
                                                Ok(s) => {
                                                    *session_slot.lock().await = Some(s);
                                                    screen.accent("started a fresh session");
                                                }
                                                Err(e) => screen.dim(&format!("error: {e}")),
                                            },
                                            None => screen.dim("unknown command — /help"),
                                        }
                                        continue;
                                    }
                                    match ctx.spawn(&text).await {
                                        Some((ev_rx, done_rx)) => {
                                            running = Some(ev_rx);
                                            run_done = Some(done_rx);
                                            run_started = Some(Instant::now());
                                            ui.styler = LineStyler::new();
                                            ui.partial.clear();
                                        }
                                        None => screen.dim("provider unavailable — check auth/config"),
                                    }
                                    screen.accent(&format!("▸ {}", text.replace('\n', " ⏎ ")));
                                }
                            }
                            Action::CancelOrClear => {
                                if is_running {
                                    cancel.cancel();
                                    screen.clear_input();
                                    screen.dim("[cancelling…]");
                                } else if !editor.is_empty() {
                                    editor = Editor::new();
                                } else {
                                    screen.clear_input();
                                    break;
                                }
                            }
                            Action::Exit => {
                                screen.clear_input();
                                break;
                            }
                            Action::Ignore => {}
                        }
                    }
                    Some(Ok(Event::Paste(text))) => {
                        if approvals.is_empty() && running_now {
                            pending.push_str(&text);
                        } else if approvals.is_empty() {
                            editor.paste_str(&text);
                        }
                    }
                    _ => {}
                }
            }
            Some(agent_event) = agent_ev => {
                render_event(&mut screen, &mut ui, &ui_theme, agent_event);
            }
            Some(req) = approval_ev => {
                approvals.push_back(req);
                if let Some(front) = approvals.front() {
                    draw_approval(&mut screen, &ui_theme, front);
                }
            }
            Some(outcome) = done_ev => {
                if let Some(mut rx) = running.take() {
                    while let Ok(ev) = rx.try_recv() {
                        render_event(&mut screen, &mut ui, &ui_theme, ev);
                    }
                }
                run_done = None;
                flush_md(&mut screen, &mut ui, &ui_theme);
                let cost = crate::pricing::session_cost(
                    &ui.model,
                    ui.total_in + ui.sub_in,
                    ui.total_out + ui.sub_out,
                );
                finish_outcome(
                    &mut screen,
                    outcome,
                    &session_path,
                    bell_on,
                    run_started.map(|t| t.elapsed().as_secs()).unwrap_or(0),
                    cost,
                    ui.total_in + ui.sub_in,
                    ui.total_out + ui.sub_out,
                );
                run_started = None;
                screen.line("");
            }
            _ = tokio::time::sleep(tick_delay) => {
                tick_count += 1;
            }
        }

        if running.is_some() {
            let elapsed = run_started.map(|t| t.elapsed().as_secs()).unwrap_or(0);
            screen.redraw_status(&status_ansi(
                &ui_theme,
                ui.total_in,
                ui.total_out,
                elapsed,
                tick_count,
                context_window,
                &pending,
            ));
        } else if editor.search_active() {
            let q = editor.search_query().to_string();
            let (buf, cur) = editor.view();
            screen.redraw_input(&format!("(r-search)`{q}` "), buf, cur);
        } else {
            let (buf, cur) = editor.view();
            screen.redraw_input("> ", buf, cur);
        }
    }

    editor.save_history(&hist_path);
    screen.dim("bye");
    0
}

fn render_event(screen: &mut Screen, ui: &mut UiState, theme: &Theme, ev: AgentEvent) {
    match ev {
        AgentEvent::Stream(StreamEvent::TextDelta { delta, .. }) => {
            ui.thinking_shown = false;
            feed_text(ui, screen, theme, &delta)
        }
        AgentEvent::Stream(StreamEvent::ThinkingDelta { .. }) => {
            if !ui.thinking_shown {
                screen.clear_input();
                screen.dim("  · thinking…");
                ui.thinking_shown = true;
            }
        }
        AgentEvent::Stream(_) => {}
        AgentEvent::ToolCallStart {
            id,
            name,
            args_json,
        } => {
            flush_md(screen, ui, theme);
            ui.tool_args.insert(id, (name.clone(), args_json.clone()));
            screen.clear_input();
            screen.tool_start(&name, &summarize_args(&name, &args_json));
        }
        AgentEvent::ToolCallEnd {
            id,
            name,
            is_error,
            result_preview,
        } => {
            flush_md(screen, ui, theme);
            let stored = ui.tool_args.remove(&id);
            screen.clear_input();
            screen.tool_line(if is_error { "✗" } else { "✓" }, &name, is_error);
            if name == "edit"
                && !is_error
                && let Some((_, args_json)) = &stored
                && let Some(diff) = edit_diff_text(args_json, theme, 10)
            {
                for line in diff.lines() {
                    screen.md_line(&format!("  {line}"));
                }
            } else if is_error
                && let Some(prev) = result_preview
                && !prev.trim().is_empty()
            {
                let tail: Vec<&str> = prev
                    .lines()
                    .filter(|l| !l.trim().is_empty())
                    .rev()
                    .take(6)
                    .collect();
                for line in tail.iter().rev() {
                    screen.styled(&format!("  │ {line}"), theme.error);
                }
            }
        }
        AgentEvent::TurnEnd { usage } => {
            ui.total_in += usage.input_tokens;
            ui.total_out += usage.output_tokens;
            if let Some(c) =
                crate::pricing::session_cost(&ui.model, usage.input_tokens, usage.output_tokens)
            {
                ui.cost_usd += c;
            }
            flush_md(screen, ui, theme);
        }
        AgentEvent::TurnStart { .. } => {
            ui.styler = LineStyler::new();
            ui.partial.clear();
            ui.thinking_shown = false;
        }
        AgentEvent::RetryScheduled {
            attempt,
            delay_ms,
            reason,
        } => {
            screen.clear_input();
            screen.dim(&format!("⟳ retry {attempt} in {delay_ms}ms — {reason}"));
        }
        AgentEvent::ContextCompacting { estimated_tokens } => {
            screen.clear_input();
            screen.dim(&format!(
                "📦 compacting context (~{estimated_tokens} tokens)…"
            ));
        }
        AgentEvent::ContextCompacted {
            before_tokens,
            after_tokens,
            summarized_messages,
        } => {
            screen.clear_input();
            screen.dim(&format!(
                "📦 context compacted: ~{before_tokens} → ~{after_tokens} tokens ({summarized_messages} messages summarized)"
            ));
        }
        AgentEvent::StopHookContinuation { reason } => {
            screen.clear_input();
            screen.dim(&format!("[stop-hook] {reason} — continuing"));
        }
        AgentEvent::SubagentStarted { label } => {
            flush_md(screen, ui, theme);
            screen.clear_input();
            screen.accent(&format!("  ◆ subagent: {}", trunc_cells(&label, 70)));
        }
        AgentEvent::SubagentToolCall {
            label,
            name,
            is_error,
        } => {
            flush_md(screen, ui, theme);
            screen.clear_input();
            let mark = if is_error { "✗" } else { "·" };
            let color = if is_error { theme.error } else { theme.dim };
            screen.styled(
                &format!("    {} {} ({})", mark, name, trunc_cells(&label, 24)),
                color,
            );
        }
        AgentEvent::SubagentUsage {
            input_tokens,
            output_tokens,
            ..
        } => {
            ui.sub_in += input_tokens;
            ui.sub_out += output_tokens;
        }
        AgentEvent::SubagentFinished {
            label,
            is_error,
            elapsed_ms,
        } => {
            flush_md(screen, ui, theme);
            screen.clear_input();
            if is_error {
                screen.error(&format!(
                    "  ◇ subagent failed: {} · {}",
                    trunc_cells(&label, 60),
                    status::fmt_elapsed(elapsed_ms / 1000)
                ));
            } else {
                screen.success(&format!(
                    "  ◇ subagent done: {} · {}",
                    trunc_cells(&label, 60),
                    status::fmt_elapsed(elapsed_ms / 1000)
                ));
            }
        }
        AgentEvent::StreamOpened
        | AgentEvent::ApprovalRequested { .. }
        | AgentEvent::RunFinished { .. } => {}
    }
}

fn feed_text(ui: &mut UiState, screen: &mut Screen, theme: &Theme, delta: &str) {
    ui.partial.push_str(delta);
    while let Some(pos) = ui.partial.find('\n') {
        let raw: String = ui.partial.drain(..=pos).collect();
        let styled = ui.styler.line(raw.trim_end_matches('\n'), theme);
        screen.clear_input();
        screen.md_line(&styled);
    }
}

fn flush_md(screen: &mut Screen, ui: &mut UiState, theme: &Theme) {
    if ui.partial.is_empty() {
        return;
    }
    let rest = std::mem::take(&mut ui.partial);
    let styled = ui.styler.line(&rest, theme);
    screen.clear_input();
    screen.md_line(&styled);
}

fn draw_approval(screen: &mut Screen, theme: &Theme, req: &ApprovalRequest) {
    screen.clear_input();
    screen.styled(&format!("? {} needs approval", req.tool), theme.warning);
    if req.tool == "edit"
        && let Some(diff) = approval_edit_diff(&req.args_json, theme)
    {
        for line in pretty_args_lines(&req.args_json, 1) {
            screen.dim(&line);
        }
        for line in diff.lines() {
            screen.md_line(&format!("  {line}"));
        }
    } else {
        for line in pretty_args_lines(&req.args_json, 6) {
            screen.dim(&line);
        }
    }
    if !req.reason.trim().is_empty() {
        screen.dim(&format!("    rule: {}", trunc_cells(&req.reason, 90)));
    }
    screen
        .dim("    [y] allow once · [a] always this session · [p] save scoped rule · [n]/Esc deny");
}

/// Builds the unified-diff text for an `edit` call, accepting both the
/// canonical `edits[]` array and models' occasional flat
/// `{old_string,new_string}` shape. None when nothing renderable.
pub fn edit_diff_text(args_json: &str, theme: &Theme, max_lines: usize) -> Option<String> {
    let v: serde_json::Value = serde_json::from_str(args_json).ok()?;
    let pairs: Vec<(String, String)> =
        if let Some(edits) = v.get("edits").and_then(|e| e.as_array()) {
            edits
                .iter()
                .filter_map(|e| {
                    let old = e.get("old_string").and_then(|x| x.as_str())?;
                    let new = e.get("new_string").and_then(|x| x.as_str())?;
                    Some((old.to_string(), new.to_string()))
                })
                .collect()
        } else {
            let old = v.get("old_string").and_then(|x| x.as_str())?;
            let new = v.get("new_string").and_then(|x| x.as_str())?;
            vec![(old.to_string(), new.to_string())]
        };
    let mut out = String::new();
    let mut any = false;
    for (old, new) in pairs {
        if old.is_empty() && new.is_empty() {
            continue;
        }
        out.push_str(&diffview::unified(&old, &new, theme, max_lines));
        out.push('\n');
        any = true;
    }
    any.then_some(out)
}

/// Derives a scoped, round-trip-validated rule spec from the call being
/// approved. Returns None when no safe scope can be derived (opaque bash,
/// missing args) — those stay session-only via [a].
pub fn learned_spec(tool: &str, args_json: &str) -> Option<String> {
    let v: serde_json::Value = serde_json::from_str(args_json).ok()?;
    let spec = match tool {
        "bash" => {
            let cmd = v.get("command").and_then(|c| c.as_str())?;
            let word = cmd
                .split_whitespace()
                .next()?
                .trim_start_matches(|c: char| {
                    !c.is_ascii_alphanumeric() && c != '_' && c != '-' && c != '.'
                })
                .to_string();
            if word.is_empty()
                || !word
                    .chars()
                    .all(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | '-' | '.'))
            {
                return None;
            }
            format!("bash({word} *)")
        }
        "write" | "edit" => format!("{}({})", tool, v.get("path").and_then(|p| p.as_str())?),
        "mcp" => {
            let server = v.get("server").and_then(|s| s.as_str())?;
            if v.get("action").and_then(|a| a.as_str()) != Some("call") {
                return None;
            }
            format!("mcp({server}/*)")
        }
        "task" => format!("task({})", v.get("label").and_then(|l| l.as_str())?),
        _ => return None,
    };
    // Round-trip gate: the persisted rule must parse AND match THIS call.
    let rule = vak_permission::Rule::parse(&spec).ok()?;
    rule.matches(tool, &v).then_some(spec)
}

/// Renders the proposed edit as a unified diff so the decision is informed
/// by what will actually change, not by a JSON blob.
fn approval_edit_diff(args_json: &str, theme: &Theme) -> Option<String> {
    let v = serde_json::from_str::<serde_json::Value>(args_json).ok()?;
    let edits = v.get("edits")?.as_array()?;
    let mut out = String::new();
    let mut shown = 0usize;
    for e in edits.iter().take(3) {
        let old = e.get("old_string").and_then(|x| x.as_str()).unwrap_or("");
        let new = e.get("new_string").and_then(|x| x.as_str()).unwrap_or("");
        if old.is_empty() && new.is_empty() {
            continue;
        }
        out.push_str(&diffview::unified(old, new, theme, 6));
        out.push('\n');
        shown += 1;
    }
    (shown > 0).then_some(out)
}

fn pretty_args_lines(args_json: &str, max_lines: usize) -> Vec<String> {
    let v: serde_json::Value = match serde_json::from_str(args_json) {
        Ok(v) => v,
        Err(_) => return vec![format!("    {}", trunc_cells(args_json, 100))],
    };
    let pretty = serde_json::to_string_pretty(&v).unwrap_or_else(|_| args_json.to_string());
    let mut lines: Vec<String> = pretty.lines().map(|l| format!("    {l}")).collect();
    if lines.len() > max_lines {
        let rest = lines.len() - max_lines;
        lines.truncate(max_lines);
        lines.push(format!("    … (+{rest} lines)"));
    }
    lines
}

pub fn summarize_args(name: &str, args_json: &str) -> String {
    let Ok(v) = serde_json::from_str::<serde_json::Value>(args_json) else {
        return trunc_cells(args_json, 60);
    };
    let pick = |k: &str| v.get(k).and_then(|x| x.as_str()).unwrap_or("").to_string();
    let hint = match name {
        "bash" => pick("command"),
        "read" | "write" | "edit" => pick("path"),
        "glob" => pick("pattern"),
        "grep" => {
            let base = v.get("path").and_then(|x| x.as_str()).unwrap_or(".");
            format!("{} in {base}", pick("pattern"))
        }
        _ => v.to_string(),
    };
    trunc_cells(&hint, 90)
}

pub fn trunc_cells(s: &str, max: usize) -> String {
    let mut w = 0usize;
    for (i, c) in s.char_indices() {
        w += crate::width::char_width(c);
        if w > max {
            return format!("{}…", &s[..i]);
        }
    }
    s.to_string()
}

fn status_ansi(
    theme: &Theme,
    total_in: u64,
    total_out: u64,
    elapsed_secs: u64,
    tick: usize,
    window: u64,
    queued: &str,
) -> String {
    let total = total_in + total_out;
    let pct = if window > 0 {
        total.saturating_mul(100) / window.max(1)
    } else {
        0
    };
    let spinner = format!("{}{}", theme::fg(theme.spinner), status::frame(tick));
    let body = format!(
        "{}{} · ↑{} ↓{} · ctx {}%{}",
        theme::fg(theme.dim),
        status::fmt_elapsed(elapsed_secs),
        status::fmt_tokens(total_in),
        status::fmt_tokens(total_out),
        pct.min(999),
        theme::fg(Color::Reset),
    );
    if queued.is_empty() {
        format!("{spinner} {body}")
    } else {
        format!(
            "{spinner} {body}{} [queued] {}{}",
            theme::fg(theme.warning),
            trunc_cells(queued, 40),
            theme::fg(Color::Reset),
        )
    }
}

#[allow(clippy::too_many_arguments)]
fn finish_outcome(
    screen: &mut Screen,
    outcome: TurnOutcome,
    session_path: &str,
    bell_on: bool,
    elapsed_secs: u64,
    cost_usd: Option<f64>,
    total_in: u64,
    total_out: u64,
) {
    let dollars = cost_usd
        .map(|c| format!(" · ~{}", crate::pricing::format_cost(c)))
        .unwrap_or_default();
    match outcome {
        TurnOutcome::Completed { response: _ } => screen.success(&format!(
            "── completed · Σ ↑{} ↓{}{dollars} · {} · {}",
            status::fmt_tokens(total_in),
            status::fmt_tokens(total_out),
            status::fmt_elapsed(elapsed_secs),
            session_path
        )),
        TurnOutcome::Aborted { .. } => screen.dim("── aborted (partial output preserved)"),
        TurnOutcome::Failed { error } => screen.error(&format!("── failed: {error}")),
        TurnOutcome::MaxTurnsReached => screen.dim("── stopped: max turns reached"),
    }
    if bell_on {
        screen.bell();
    }
}

async fn show_context(screen: &mut Screen, slot: &Arc<Mutex<Option<SessionLog>>>, window: u64) {
    let guard = slot.lock().await;
    let Some(s) = guard.as_ref() else {
        return;
    };
    let msgs = s.derive_messages();
    let total = s.total_usage().total_tokens();
    let pct = if window > 0 {
        total.saturating_mul(100) / window.max(1)
    } else {
        0
    };
    screen.dim(&format!(
        "next request: ~{} messages · ~{total} / {window} tokens ({}% of window)",
        msgs.len(),
        pct.min(999),
    ));
}

fn list_sessions(core: &Core, screen: &mut Screen) {
    let dir = vak_session::SessionPath::sessions_dir(&core.sessions_home(), core.cwd());
    match std::fs::read_dir(&dir) {
        Ok(entries) => {
            let mut rows: Vec<(std::time::SystemTime, std::path::PathBuf, String)> = entries
                .flatten()
                .filter_map(|e| {
                    let name = e.file_name().to_string_lossy().into_owned();
                    if !name.ends_with(".jsonl") {
                        return None;
                    }
                    let m = e.metadata().ok()?.modified().ok()?;
                    Some((m, e.path(), name))
                })
                .collect();
            rows.sort_by_key(|r| std::cmp::Reverse(r.0));
            if rows.is_empty() {
                screen.dim("no sessions yet");
                return;
            }
            screen.dim("sessions:");
            for (m, path, name) in rows.iter().rev().take(15) {
                let snippet = first_user_prompt(path)
                    .map(|s| format!(" · {}", trunc_cells(&s, 52)))
                    .unwrap_or_default();
                screen.dim(&format!(
                    "  {}{} ({})",
                    name.trim_end_matches(".jsonl"),
                    snippet,
                    age_of(*m),
                ));
            }
            screen.dim("use /resume <id-prefix>");
        }
        Err(_) => screen.dim("no sessions yet"),
    }
}

/// First meaningful user prompt from a session JSONL (bounded head scan).
fn first_user_prompt(path: &std::path::Path) -> Option<String> {
    use std::io::BufRead;
    let file = std::fs::File::open(path).ok()?;
    for line in std::io::BufReader::new(file).lines().take(120).flatten() {
        let Ok(v) = serde_json::from_str::<serde_json::Value>(&line) else {
            continue;
        };
        if v.get("kind").and_then(|k| k.as_str()) != Some("message") {
            continue;
        }
        let Some(msg) = v.get("message") else {
            continue;
        };
        if msg.get("role").and_then(|r| r.as_str()) != Some("user") {
            continue;
        }
        let text = msg
            .get("content")
            .and_then(|c| c.as_array())
            .map(|a| {
                a.iter()
                    .filter_map(|b| b.get("text").and_then(|t| t.as_str()))
                    .collect::<Vec<_>>()
                    .join(" ")
            })
            .unwrap_or_default();
        let t = text.trim();
        if t.is_empty() || t.starts_with("[stop-hook]") {
            continue;
        }
        return Some(t.to_string());
    }
    None
}

fn age_of(modified: std::time::SystemTime) -> String {
    let secs = modified.elapsed().map(|d| d.as_secs()).unwrap_or(0);
    if secs < 60 {
        "now".to_string()
    } else {
        status::fmt_elapsed(secs)
    }
}

async fn cmd_resume(
    core: &Core,
    slot: &Arc<Mutex<Option<SessionLog>>>,
    arg: Option<String>,
    screen: &mut Screen,
) {
    let ids = sessions_by_mtime(core);
    match arg {
        None => {
            if ids.is_empty() {
                screen.dim("no sessions yet");
                return;
            }
            screen.dim("recent sessions:");
            for (i, id) in ids.iter().take(15).enumerate() {
                screen.dim(&format!("  {:>2}. {}", i + 1, id));
            }
            screen.dim("use /resume <n> or <id-prefix>");
        }
        Some(sel) => {
            let chosen = sel
                .parse::<usize>()
                .ok()
                .and_then(|n| n.checked_sub(1).and_then(|i| ids.get(i).cloned()))
                .or_else(|| ids.iter().find(|id| id.starts_with(sel.as_str())).cloned());
            match chosen {
                Some(id) => match core.open_session(&id).await {
                    Ok(s) => {
                        let msgs = s.derive_messages().len();
                        *slot.lock().await = Some(s);
                        screen.accent(&format!("resumed {id} · {msgs} messages"));
                    }
                    Err(e) => screen.dim(&format!("error: {e}")),
                },
                None => screen.dim("no such session"),
            }
        }
    }
}

async fn cmd_rewind(
    core: &Core,
    slot: &Arc<Mutex<Option<SessionLog>>>,
    arg: Option<String>,
    screen: &mut Screen,
) {
    let sid = slot
        .lock()
        .await
        .as_ref()
        .and_then(|s| s.header())
        .map(|h| h.session_id.clone());
    let Some(sid) = sid else {
        screen.dim("no active session");
        return;
    };
    let cps = vak_core::checkpoints::list(&core.sessions_home(), &sid).unwrap_or_default();
    match arg {
        None => {
            if cps.is_empty() {
                screen.dim("no checkpoints yet");
                return;
            }
            screen.dim("checkpoints (newest first):");
            for cp in cps.iter().rev().take(10) {
                screen.dim(&format!(
                    "  {:>3}. {} · {} · {} files",
                    cp.seq,
                    cp.created_at.format("%m-%d %H:%M"),
                    trunc_cells(&cp.label, 44),
                    cp.files.len()
                ));
            }
            screen.dim("use /rewind <seq>");
        }
        Some(sel) => match sel.parse::<u32>() {
            Ok(seq) => match vak_core::checkpoints::load(&core.sessions_home(), &sid, seq) {
                Ok(cp) => match vak_core::checkpoints::restore(core.cwd(), &cp) {
                    Ok((restored, deleted)) => screen.success(&format!(
                        "rewound to {}: restored {restored} files, removed {deleted}",
                        cp.seq
                    )),
                    Err(e) => screen.dim(&format!("error: restore failed: {e}")),
                },
                Err(_) => screen.dim("checkpoint not found"),
            },
            Err(_) => screen.dim("usage: /rewind <seq>"),
        },
    }
}

fn sessions_by_mtime(core: &Core) -> Vec<String> {
    let dir = vak_session::SessionPath::sessions_dir(&core.sessions_home(), core.cwd());
    let mut rows: Vec<(std::time::SystemTime, String)> = Vec::new();
    if let Ok(rd) = std::fs::read_dir(&dir) {
        for e in rd.flatten() {
            let name = e.file_name().to_string_lossy().into_owned();
            if !name.ends_with(".jsonl") {
                continue;
            }
            if let Ok(m) = e.metadata().and_then(|m| m.modified()) {
                rows.push((m, name));
            }
        }
    }
    rows.sort_by_key(|r| std::cmp::Reverse(r.0));
    rows.into_iter()
        .map(|(_, n)| n.trim_end_matches(".jsonl").to_string())
        .collect()
}

/// Compact read-only dump of the active session's message tree.
async fn show_transcript(
    screen: &mut Screen,
    slot: &Arc<Mutex<Option<SessionLog>>>,
    arg: Option<&str>,
    theme: &Theme,
) {
    let limit = arg.and_then(|a| a.parse::<usize>().ok()).unwrap_or(40);
    let guard = slot.lock().await;
    let Some(s) = guard.as_ref() else {
        screen.dim("no active session");
        return;
    };
    let msgs = s.derive_messages();
    let start = msgs.len().saturating_sub(limit);
    screen.clear_input();
    screen.dim(&format!(
        "transcript · showing {} of {} messages",
        msgs.len() - start,
        msgs.len()
    ));
    for m in &msgs[start..] {
        let label_role = match m.role {
            vak_llm::Role::User => ("▸ user", theme.accent),
            _ => ("◆ assistant", theme.success),
        };
        for block in &m.content {
            match block {
                vak_llm::ContentBlock::Text { text } => {
                    screen.styled(
                        &format!("{} {}", label_role.0, trunc_cells(text.trim(), 160)),
                        label_role.1,
                    );
                }
                vak_llm::ContentBlock::Thinking { .. } => {}
                vak_llm::ContentBlock::ToolUse { name, input, .. } => {
                    screen.dim(&format!(
                        "  · tool {name} {}",
                        trunc_cells(&input.to_string(), 80)
                    ));
                }
                vak_llm::ContentBlock::ToolResult {
                    content, is_error, ..
                } => {
                    let mark = if *is_error { "✗" } else { "→" };
                    screen.styled(
                        &format!(
                            "  {mark} {}",
                            trunc_cells(content.replace('\n', " ").trim(), 120)
                        ),
                        if *is_error { theme.error } else { theme.dim },
                    );
                }
            }
        }
    }
}

/// Health check rendered as a checklist: auth, sandbox, storage, config
/// warnings, extension surface.
fn run_doctor(core: &Core, screen: &mut Screen) {
    screen.clear_input();
    screen.accent("doctor:");
    let mut failures = 0usize;

    let provider_check = match core.provider() {
        Ok(p) => Ok(format!("{} ready", p.name())),
        Err(e) => Err(e.to_string()),
    };
    report(screen, "provider", &provider_check, &mut failures);

    let home_ok = std::fs::create_dir_all(core.sessions_home()).is_ok();
    report(
        screen,
        "sessions home",
        &(if home_ok {
            Ok(core.sessions_home().display().to_string())
        } else {
            Err("not writable".to_string())
        }),
        &mut failures,
    );

    let warnings = core.config().warnings.clone();
    report(
        screen,
        "config warnings",
        &(if warnings.is_empty() {
            Ok("none".to_string())
        } else {
            Err(warnings.join("; "))
        }),
        &mut failures,
    );

    screen.dim(&format!(
        "  · model {} via {} · mode {:?} · sandbox {}",
        core.effective_model(),
        core.effective_provider(),
        core.effective_permission_mode(),
        core.effective_sandbox_name(),
    ));
    screen.dim(&format!(
        "  · context window {} tokens · max turns {} · retries {} (+{})",
        core.config().context_window,
        core.effective_max_turns(),
        core.config().max_retries,
        core.config().run_retry_attempts,
    ));
    screen.dim(&format!(
        "  · extensions: {} skills · {} hooks · {} mcp servers · subagents {}",
        core.skills().len(),
        core.config().hooks.len(),
        core.config().mcp.servers.len(),
        if core.config().subagents { "on" } else { "off" },
    ));
    if failures == 0 {
        screen.success("all checks passed");
    } else {
        screen.error(&format!("{failures} check(s) failed"));
    }
}

fn report(screen: &mut Screen, label: &str, result: &Result<String, String>, failures: &mut usize) {
    match result {
        Ok(detail) => screen.success(&format!("  ✓ {label}: {detail}")),
        Err(detail) => {
            screen.error(&format!("  ✗ {label}: {detail}"));
            *failures += 1;
        }
    }
}

fn handle_complete(editor: &mut Editor, core: &Core, screen: &mut Screen) {
    let buf = editor.view().0.to_string();
    let suggestions = complete::complete(&buf, commands::COMMANDS, core.cwd());
    if suggestions.is_empty() {
        return;
    }
    let start = last_token_start(&buf);
    let token = &buf[start..];
    let replaces: Vec<&str> = suggestions.iter().map(|s| s.replace.as_str()).collect();
    let prefix = common_prefix(&replaces);
    if prefix.chars().count() > token.chars().count() {
        let mut new_text = String::with_capacity(start + prefix.len());
        new_text.push_str(&buf[..start]);
        new_text.push_str(&prefix);
        editor.set_text(&new_text);
    }
    if suggestions.len() > 1 {
        screen.clear_input();
        for s in suggestions.iter().take(8) {
            screen.dim(&format!("  {:<32} {}", trunc_cells(&s.replace, 30), s.hint));
        }
        if suggestions.len() > 8 {
            screen.dim(&format!("  … (+{})", suggestions.len() - 8));
        }
    }
}

/// Byte offset just past the last whitespace character (start of the final token).
fn last_token_start(buf: &str) -> usize {
    buf.char_indices()
        .rev()
        .find(|(_, c)| c.is_whitespace())
        .map(|(i, c)| i + c.len_utf8())
        .unwrap_or(0)
}

fn common_prefix(items: &[&str]) -> String {
    let mut out = String::new();
    let Some(first) = items.first() else {
        return out;
    };
    for (i, c) in first.char_indices() {
        if items
            .iter()
            .all(|s| s.as_bytes().get(i) == first.as_bytes().get(i))
        {
            out.push(c);
        } else {
            break;
        }
    }
    out
}
