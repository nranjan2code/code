use std::collections::{HashMap, HashSet, VecDeque};
use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, Instant};

use crossterm::cursor::{Hide, Show};
use crossterm::event::{DisableBracketedPaste, EnableBracketedPaste, Event, EventStream, KeyCode};
use crossterm::execute;
use crossterm::style::Color;
use crossterm::terminal::{
    EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode, enable_raw_mode,
};
use futures::StreamExt;
use tokio::sync::{Mutex, mpsc, oneshot};
use tokio_util::sync::CancellationToken;

use vak_agent::{AgentEvent, Approver, SteeringQueues, TurnOutcome};
use vak_config::PermissionMode;
use vak_core::Core;
use vak_core::inbox::{Entry, Kind};
use vak_llm::stream::StreamEvent;
use vak_session::SessionLog;

use crate::commands::{self, Command};
use crate::complete;
use crate::diffview;
use crate::editor::{ComposerMode, Editor, VimState};
use crate::keymap::{KeySpec, Keymap};
use crate::keys::Action;
use crate::markdown::LineStyler;
use crate::mentions;
use crate::palette::{ChoiceItem, ChoicePicker, CommandPalette, PaletteItem};
use crate::render::Screen;
use crate::status;
use crate::theme::{self, Theme};

pub struct UiConfig {
    pub cwd: PathBuf,
}

/// Custom theme definitions from `[ui.themes]`, keyed by name.
type CustomThemes = std::collections::BTreeMap<String, theme::ThemeColors>;

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
        let _ = execute!(
            std::io::stdout(),
            EnterAlternateScreen,
            EnableBracketedPaste,
            Hide
        );
        RawMode
    }
}

impl Drop for RawMode {
    fn drop(&mut self) {
        let _ = execute!(
            std::io::stdout(),
            DisableBracketedPaste,
            Show,
            LeaveAlternateScreen
        );
        let _ = disable_raw_mode();
    }
}

/// Splits "/goal objective -- c1; c2" spec into (objective, criteria).
fn split_goal_spec(spec: &str) -> (String, Vec<String>) {
    let (objective, criteria_str) = match spec.find("--") {
        Some(i) => (spec[..i].trim(), &spec[i + 2..]),
        None => (spec.trim(), ""),
    };
    let criteria = criteria_str
        .split(';')
        .map(|c| c.trim().to_string())
        .filter(|c| !c.is_empty())
        .collect();
    (objective.to_string(), criteria)
}

/// Armed goal: objective + acceptance criteria (docs/design/27 Phase H).
type PendingGoal = Arc<std::sync::Mutex<Option<(String, Vec<String>)>>>;

/// Upper bound on one background reflection pass (docs/design/29 P1) so a
/// stuck auxiliary stream cannot keep the session ledger pinned.
const REFLECTION_CALL_TIMEOUT: Duration = Duration::from_secs(120);

/// Cadence for refreshing the cached unread-inbox badge count (personal-os
/// P6): disk reads stay bounded to one small scan per minute at most, plus
/// explicit refreshes whenever /inbox itself runs.
const INBOX_REFRESH_SECS: u64 = 60;

struct RunCtx {
    core: Core,
    session_slot: Arc<Mutex<Option<SessionLog>>>,
    steering: Arc<SteeringQueues>,
    approver: Arc<dyn Approver>,
    cancel: Arc<Mutex<CancellationToken>>,
    /// Goal mode (docs/design/27 Phase H): armed via /goal, consumed by
    /// the next spawned turn.
    pending_goal: PendingGoal,
    /// Transient status ticks emitted by the post-turn reflection pass.
    refl_tx: mpsc::UnboundedSender<String>,
}

impl RunCtx {
    /// Spawns one turn. Returns None when no provider/auth is available; the
    /// session slot is always restored so later turns still work.
    async fn spawn(
        &self,
        prompt: &str,
    ) -> Option<(mpsc::Receiver<AgentEvent>, mpsc::Receiver<TurnOutcome>)> {
        let taken = self.session_slot.lock().await.take()?;
        let goal = self
            .pending_goal
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .take();
        if self.core.provider().is_err() {
            *self.session_slot.lock().await = Some(taken);
            return None;
        }
        let (ev_tx, ev_rx) = mpsc::channel(1024);
        let (done_tx, done_rx) = mpsc::channel(1);
        let prompt = prompt.to_string();
        let core = self.core.clone();
        let approver = self.approver.clone();
        let cancel = CancellationToken::new();
        *self.cancel.lock().await = cancel.clone();
        let steering = self.steering.clone();
        let session_slot = self.session_slot.clone();
        let refl_tx = self.refl_tx.clone();
        tokio::spawn(async move {
            let turned = if let Some((objective, criteria)) = goal {
                core.run_goal_turn_with(
                    taken,
                    &prompt,
                    &objective,
                    criteria,
                    cancel,
                    Some(approver),
                    None,
                    Some(steering),
                    ev_tx,
                )
                .await
            } else {
                core.run_turn_with(
                    taken,
                    &prompt,
                    cancel,
                    Some(approver),
                    None,
                    Some(steering),
                    ev_tx,
                )
                .await
            };
            let (outcome, session_log) = match turned {
                Ok((o, log)) => (o, Some(log)),
                Err(e) => {
                    let error = match e {
                        vak_core::CoreError::Llm(l) => l,
                        other => vak_llm::LlmError::InvalidRequest(other.to_string()),
                    };
                    (TurnOutcome::Failed { error }, None)
                }
            };
            // Always release the loop back to the editor: an Err outcome must
            // still clear `running`, or every later keystroke would be
            // swallowed as steering input and the TUI could never exit.
            let completed = matches!(outcome, TurnOutcome::Completed { .. });
            let _ = done_tx.send(outcome).await;
            // Background reflection seam (docs/design/29 P1): runs only after
            // the loop was released (input stays live while it works),
            // bounded so a stuck auxiliary stream cannot pin the ledger.
            // The slot is restored afterwards on every path so later turns
            // still work.
            if let Some(log) = session_log {
                if completed {
                    let pass = tokio::time::timeout(
                        REFLECTION_CALL_TIMEOUT,
                        core.reflect_after_turn(&log, ""),
                    )
                    .await;
                    if let Ok(vak_core::reflection::ReflectionOutcome::Reflected {
                        notes_added,
                        ..
                    }) = pass
                    {
                        let _ = refl_tx.send(format!("✎ memory +{notes_added}"));
                    }
                }
                *session_slot.lock().await = Some(log);
            }
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
    provider: String,
    cost_usd: f64,
    sub_in: u64,
    sub_out: u64,
    thinking_mode: ThinkingMode,
    thinking_partial: String,
    expanded_tools: bool,
    run_state: RunState,
    /// Final assistant text of the current/last turn, for Alt-Y /copy.
    last_response: String,
    /// When attached to a subagent, its label drives composer identity.
    attached_label: Option<String>,
}

/// Typed run-state truth (doc 21 §4): the status row always names what the
/// agent is doing. No generic spinner may stand in for a specific state.
#[derive(Clone, PartialEq, Eq)]
enum RunState {
    Thinking,
    Streaming,
    Tool {
        name: String,
    },
    Retrying {
        attempt: u32,
        delay_ms: u64,
        reason: String,
    },
    Compacting,
}

impl RunState {
    fn label(&self) -> String {
        match self {
            Self::Thinking => "thinking".to_string(),
            Self::Streaming => "streaming".to_string(),
            Self::Tool { name } => format!("tool · {name}"),
            Self::Retrying {
                attempt,
                delay_ms,
                reason,
            } => format!("retry {attempt} in {delay_ms}ms — {reason}"),
            Self::Compacting => "compacting context".to_string(),
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum ThinkingMode {
    Off,
    Indicator,
    Full,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum PickerKind {
    Provider,
    Model,
    Theme,
    Subagents,
}

#[derive(Default)]
struct ModalView {
    title: String,
    rows: Vec<String>,
    scroll: usize,
    footer: String,
    /// Row indices of user prompts, for n/p jumps in the transcript viewer.
    anchors: Vec<usize>,
    /// Highlighted row, for interactive modals (keymap rebind).
    selected: Option<usize>,
}

impl ThinkingMode {
    fn next(self) -> Self {
        match self {
            Self::Off => Self::Indicator,
            Self::Indicator => Self::Full,
            Self::Full => Self::Off,
        }
    }

    fn name(self) -> &'static str {
        match self {
            Self::Off => "off",
            Self::Indicator => "indicator",
            Self::Full => "full",
        }
    }
}

pub async fn run(core: Core, _cfg: UiConfig) -> i32 {
    let mut raw: Option<RawMode> = Some(RawMode::enable());
    let ui_theme_name = core.effective_theme();
    let bell_on = core.config().ui.bell;
    let context_window = core.config().context_window;
    let custom_themes: CustomThemes = core.config().ui.themes.clone();
    let mut ui_theme = theme::resolve(&ui_theme_name, &custom_themes);
    let mut screen = Screen::new(ui_theme);
    screen.clear_viewport();
    let acc = &core.config().ui.accessibility;
    let mut a11y_motion = acc.reduced_motion;
    let mut a11y_plain = acc.plain;
    let mut a11y_reader = acc.screen_reader;
    screen.set_accessibility(a11y_plain, a11y_reader);
    let sessions_home = core.sessions_home();
    let hist_path = sessions_home.join("input_history.txt");
    let draft_path = sessions_home.join("composer_draft.txt");
    let mut editor = Editor::new();
    if core.config().ui.composer == "vim" {
        editor.set_mode(ComposerMode::Vim);
    }
    if editor.recover_draft(&draft_path) {
        screen.dim("[recovered an unsent draft from a previous session · Enter sends it]");
    }
    editor.load_history(&hist_path);

    let steering: Arc<SteeringQueues> = Arc::new(SteeringQueues::new());
    let cancel = Arc::new(Mutex::new(CancellationToken::new()));

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
    let pending_goal: PendingGoal = Arc::new(std::sync::Mutex::new(None));
    let (refl_tx, mut refl_rx) = mpsc::unbounded_channel::<String>();
    // Background reflection surfaces here as a transient composer-footer
    // tick ("✎ memory +N"); expires on its own, never steals focus.
    let mut refl_tick: Option<(String, std::time::Instant)> = None;
    let ctx = RunCtx {
        core: core.clone(),
        session_slot: session_slot.clone(),
        steering: steering.clone(),
        approver,
        cancel: cancel.clone(),
        pending_goal: pending_goal.clone(),
        refl_tx,
    };

    screen.set_title(&format!("VakCoder · {}", core.effective_model()));
    screen.banner(
        vak_core::APP_VERSION,
        &core.effective_provider(),
        &core.config().model.clone(),
        &session_path,
    );

    let mut reader: Option<EventStream> = Some(EventStream::new());
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
        provider: core.effective_provider(),
        cost_usd: 0.0,
        sub_in: 0,
        sub_out: 0,
        thinking_mode: ThinkingMode::Indicator,
        thinking_partial: String::new(),
        expanded_tools: false,
        run_state: RunState::Thinking,
        last_response: String::new(),
        attached_label: None,
    };
    let mut approvals: VecDeque<ApprovalRequest> = VecDeque::new();
    let mut follow_ups: VecDeque<String> = VecDeque::new();
    let mut run_started: Option<Instant> = None;
    let mut tick_count: usize = 0;
    let mut last_agent_event = Instant::now();
    let mut palette: Option<CommandPalette> = None;
    let mut picker: Option<(PickerKind, ChoicePicker)> = None;
    let mut theme_preview_origin: Option<String> = None;
    let mut modal: Option<ModalView> = None;
    let mut palette_from_slash = false;
    let mut approval_focused = false;
    let mut key_capture = false;
    let mut keymap = Keymap::default().with_overrides(&core.config().ui.keymap);
    let mut transcript_search: Option<String> = None;
    let mut transcript_match_idx: usize = 0;
    // (child id, label) while attached to a running subagent.
    let mut attached: Option<(String, String)> = None;
    // Keymap modal: selectable row index and the action bound on each row.
    let mut keymap_select: Option<usize> = None;
    let mut keymap_meta: Vec<(usize, &'static str)> = Vec::new();
    // Pending interactive rebind: next captured key becomes the binding.
    let mut rebind_action: Option<&'static str> = None;
    // /search result rows aligned with the open search modal's selection.
    let mut search_hits: Vec<SearchHit> = Vec::new();
    let mut search_select: usize = 0;
    // /inbox entries aligned with the open inbox modal's selection, so Enter
    // can resolve the highlighted row back to its ledger without re-reading.
    let mut inbox_hits: Vec<Entry> = Vec::new();
    let mut inbox_select: usize = 0;
    // /memory forget armed but not yet confirmed (y/n inline prompt).
    let mut confirm_forget: Option<ForgetConfirm> = None;
    // /inbox ack armed but not yet confirmed (y/n inline prompt).
    let mut confirm_ack: Option<InboxAck> = None;
    // Cached unread-inbox count behind the idle ✉ badge; refreshed on
    // /inbox use and by the idle tick at most every INBOX_REFRESH_SECS.
    let mut inbox_unread = vak_core::inbox::unread_count(&core.sessions_home());
    let mut inbox_next_poll = Instant::now() + Duration::from_secs(INBOX_REFRESH_SECS);
    // Mode switch requested while a run was in flight; applied when the run
    // actually stops so the new mode is never reported early (invariant 11).
    let mut deferred_mode: Option<vak_config::PermissionMode> = None;

    let (buf, cur) = editor.view();
    screen.redraw_composer(
        &composer_label(&ui, inbox_unread),
        buf,
        cur,
        &composer_footer(&editor),
    );

    loop {
        let running_now = running.is_some();
        let input_ev = async {
            match reader.as_mut() {
                Some(r) => r.next().await,
                None => std::future::pending().await,
            }
        };
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
        let refl_ev = refl_rx.recv();
        let tick_delay = if running_now {
            Duration::from_millis(200)
        } else {
            // Idle ticks wake early only when the inbox badge cache is due
            // for its bounded refresh; otherwise they stay hour-scaled.
            Duration::from_secs(3600).min(inbox_next_poll.saturating_duration_since(Instant::now()))
        };

        tokio::select! {
            refl = refl_ev => {
                if let Some(msg) = refl {
                    refl_tick = Some((msg, std::time::Instant::now()));
                }
            }
            maybe_event = input_ev => {
                match maybe_event {
                    Some(Ok(Event::Key(key))) => {
                        if key.kind != crossterm::event::KeyEventKind::Press {
                            continue;
                        }
                        if key_capture {
                            if matches!(key.code, KeyCode::Esc) {
                                key_capture = false;
                                screen.dim("literal-key capture off");
                            } else {
                                screen.line(&crate::keys::describe(key.code, key.modifiers));
                            }
                            continue;
                        }
                        if approval_focused
                            && let Some(req) = approvals.pop_front()
                        {
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
                                KeyCode::Char('n' | 'N' | 'q' | 'Q') => {
                                    let _ = req.respond.send(false);
                                    screen.clear_input();
                                    screen.error(&format!("✗ denied {}", req.tool));
                                }
                                KeyCode::Esc => {
                                    approvals.push_front(req);
                                    approval_focused = false;
                                    continue;
                                }
                                // Stray keys never answer a pending approval:
                                // they are ignored outright instead of leaking
                                // into the editor or silently denying.
                                _ => {
                                    approvals.push_front(req);
                                    continue;
                                }
                            }
                            approval_focused = false;
                            continue;
                        }
                        // Inline y/n confirm for destructive /memory ops:
                        // stray keys are ignored exactly like approvals.
                        if !approval_focused && confirm_forget.is_some() {
                            if let Some(action) = confirm_forget.take() {
                                match key.code {
                                    KeyCode::Char('y' | 'Y') => {
                                        match vak_core::memory::forget_note(
                                            &action.path,
                                            &action.id,
                                        ) {
                                            Ok(bytes) => screen.success(&format!(
                                                "✓ forgot note {} · {bytes} bytes reclaimed",
                                                short_hex(&action.id),
                                            )),
                                            Err(e) => screen.error(&format!("forget failed: {e}")),
                                        }
                                    }
                                    KeyCode::Char('n' | 'N' | 'q' | 'Q') | KeyCode::Esc => {
                                        screen.dim("forget cancelled — note kept");
                                    }
                                    _ => confirm_forget = Some(action),
                                }
                            }
                            continue;
                        }
                        // Inline y/n confirm for /inbox ack: same stray-key
                        // discipline as the forget card.
                        if !approval_focused && confirm_ack.is_some() {
                            if let Some(action) = confirm_ack.take() {
                                match key.code {
                                    KeyCode::Char('y' | 'Y') => {
                                        match vak_core::inbox::ack(
                                            &core.sessions_home(),
                                            &action.id,
                                        ) {
                                            Ok(true) => {
                                                inbox_unread =
                                                    vak_core::inbox::unread_count(
                                                        &core.sessions_home(),
                                                    );
                                                inbox_next_poll = Instant::now()
                                                    + Duration::from_secs(INBOX_REFRESH_SECS);
                                                refl_tick = Some((
                                                    format!(
                                                        "✓ acked {}",
                                                        short_hex(&action.id)
                                                    ),
                                                    Instant::now(),
                                                ));
                                            }
                                            Ok(false) => screen.dim("already acked"),
                                            Err(e) => {
                                                screen.error(&format!("ack failed: {e}"))
                                            }
                                        }
                                    }
                                    KeyCode::Char('n' | 'N' | 'q' | 'Q') | KeyCode::Esc => {
                                        screen.dim("ack cancelled — entry left unread");
                                    }
                                    _ => confirm_ack = Some(action),
                                }
                            }
                            continue;
                        }
                        // Interactive rebind capture: the next key becomes
                        // the binding for the selected action.
                        if let Some(action) = rebind_action.take() {
                            match key.code {
                                KeyCode::Esc => screen.dim("rebind cancelled"),
                                _ => {
                                    let spec = KeySpec {
                                        code: key.code,
                                        mods: key.modifiers,
                                    };
                                    if keymap.rebind_named(action, spec) {
                                        let conflicts = keymap.conflicts();
                                        let note = conflicts
                                            .iter()
                                            .find(|(k, _)| k == &crate::keymap::keymap_label(spec))
                                            .map(|(_, d)| d.clone())
                                            .unwrap_or_default();
                                        screen.accent(&format!(
                                            "✓ {} → {}{}",
                                            action,
                                            crate::keymap::keymap_label(spec),
                                            if note.is_empty() {
                                                String::new()
                                            } else {
                                                format!(" · !! {note}")
                                            }
                                        ));
                                        if modal.as_ref().is_some_and(|m| m.title == "keymap · bindings") {
                                            let (m, meta) =
                                                build_keymap_modal(&keymap, keymap_select);
                                            keymap_meta = meta;
                                            modal = Some(m);
                                        }
                                    } else {
                                        screen
                                            .dim("that action cannot be rebound");
                                    }
                                }
                            }
                            continue;
                        }
                        let is_running = running.is_some();
                        if modal.is_some() && !is_running {
                            let mut close = false;
                            let mut modal_action = None;
                            if let Some(active) = modal.as_mut() {
                                let page = crossterm::terminal::size()
                                    .map(|(_, rows)| rows.saturating_sub(8) as usize)
                                    .unwrap_or(16)
                                    .max(1);
                                let max_scroll = active.rows.len().saturating_sub(page);
                                if transcript_search.is_some() {
                                    let mut cancel = false;
                                    if let Some(query) = transcript_search.as_mut() {
                                        match key.code {
                                            KeyCode::Esc => cancel = true,
                                            KeyCode::Enter => {
                                                let matches =
                                                    find_matches(&active.rows, query);
                                                if !matches.is_empty() {
                                                    transcript_match_idx =
                                                        (transcript_match_idx + 1)
                                                            % matches.len();
                                                    active.scroll =
                                                        matches[transcript_match_idx]
                                                            .min(max_scroll);
                                                }
                                            }
                                            KeyCode::Backspace => {
                                                query.pop();
                                                let matches =
                                                    find_matches(&active.rows, query);
                                                transcript_match_idx = 0;
                                                if let Some(&first) = matches.first() {
                                                    active.scroll = first.min(max_scroll);
                                                }
                                            }
                                            KeyCode::Char(c)
                                                if key.modifiers.is_empty()
                                                    || key.modifiers
                                                        == crossterm::event::KeyModifiers::SHIFT =>
                                            {
                                                query.push(c);
                                                let matches =
                                                    find_matches(&active.rows, query);
                                                transcript_match_idx = 0;
                                                if let Some(&first) = matches.first() {
                                                    active.scroll = first.min(max_scroll);
                                                }
                                            }
                                            _ => {}
                                        }
                                    }
                                    if cancel {
                                        transcript_search = None;
                                    }
                                    if let Some(query) = transcript_search.as_ref() {
                                        let count = find_matches(&active.rows, query).len();
                                        active.footer = format!(
                                            "search '{query}' · {count} matches · Enter next · Esc cancel"
                                        );
                                    } else {
                                        active.footer =
                                            "n/p prompt jumps · / search · e export · End latest · Esc close".to_string();
                                    }
                                } else {
                                match key.code {
                                    KeyCode::Esc | KeyCode::Char('q') => close = true,
                                    KeyCode::Up if active.title == "keymap · bindings" => {
                                        keymap_select = step_select(
                                            keymap_select, &keymap_meta, false,
                                        );
                                        active.selected = keymap_select;
                                    }
                                    KeyCode::Down if active.title == "keymap · bindings" => {
                                        keymap_select =
                                            step_select(keymap_select, &keymap_meta, true);
                                        active.selected = keymap_select;
                                    }
                                    KeyCode::Char('r' | 'R') | KeyCode::Enter
                                        if active.title == "keymap · bindings" =>
                                    {
                                        if let Some(&(_, action)) = keymap_select
                                            .and_then(|sel| keymap_meta.get(sel))
                                            .or_else(|| keymap_meta.first())
                                        {
                                            rebind_action = Some(action);
                                            active.footer = format!(
                                                "press the new key for '{action}' · Esc cancels"
                                            );
                                        }
                                    }
                                    KeyCode::Char('p' | 'P') if active.title == "settings" => {
                                        modal_action = Some('p');
                                    }
                                    KeyCode::Char('m' | 'M') if active.title == "settings" => {
                                        modal_action = Some('m');
                                    }
                                    KeyCode::Char('f' | 'F') if active.title == "settings" => {
                                        modal_action = Some('f');
                                    }
                                    KeyCode::Char('t' | 'T') if active.title == "settings" => {
                                        modal_action = Some('t');
                                    }
                                    KeyCode::Char('n' | 'N') if active.title == "transcript" => {
                                        if let Some(&next) = active
                                            .anchors
                                            .iter()
                                            .find(|&&a| a > active.scroll)
                                        {
                                            active.scroll = next.min(max_scroll);
                                        }
                                    }
                                    KeyCode::Char('p' | 'P') if active.title == "transcript" => {
                                        if let Some(&prev) = active
                                            .anchors
                                            .iter()
                                            .rev()
                                            .find(|&&a| a < active.scroll)
                                        {
                                            active.scroll = prev;
                                        }
                                    }
                                    KeyCode::Char('e' | 'E') if active.title == "transcript" => {
                                        modal_action = Some('e');
                                    }
                                    KeyCode::Char('/')
                                        if active.title == "transcript" =>
                                    {
                                        transcript_search = Some(String::new());
                                        transcript_match_idx = 0;
                                        active.footer =
                                            "search: type query · Enter next · Esc cancel".to_string();
                                    }
                                    KeyCode::Up | KeyCode::Down | KeyCode::Enter
                                        if !search_hits.is_empty()
                                            && active.title == "session search" =>
                                    {
                                        let last = search_hits.len().saturating_sub(1);
                                        match key.code {
                                            KeyCode::Up => {
                                                search_select = search_select.saturating_sub(1);
                                            }
                                            KeyCode::Down => {
                                                search_select = (search_select + 1).min(last);
                                            }
                                            _ => modal_action = Some('o'),
                                        }
                                        active.selected = Some(search_select);
                                        // Keep the highlighted hit inside the viewport.
                                        if search_select < active.scroll {
                                            active.scroll = search_select;
                                        } else if search_select >= active.scroll + page {
                                            active.scroll =
                                                (search_select + 1).saturating_sub(page);
                                        }
                                    }
                                    KeyCode::Up | KeyCode::Down | KeyCode::Enter
                                        if !inbox_hits.is_empty()
                                            && active.title == "inbox" =>
                                    {
                                        let last = inbox_hits.len().saturating_sub(1);
                                        match key.code {
                                            KeyCode::Up => {
                                                inbox_select = inbox_select.saturating_sub(1);
                                            }
                                            KeyCode::Down => {
                                                inbox_select = (inbox_select + 1).min(last);
                                            }
                                            _ => modal_action = Some('i'),
                                        }
                                        active.selected = Some(inbox_select);
                                        // Keep the highlighted hit inside the viewport.
                                        if inbox_select < active.scroll {
                                            active.scroll = inbox_select;
                                        } else if inbox_select >= active.scroll + page {
                                            active.scroll =
                                                (inbox_select + 1).saturating_sub(page);
                                        }
                                    }
                                    KeyCode::Up => active.scroll = active.scroll.saturating_sub(1),
                                    KeyCode::Down => {
                                        active.scroll = (active.scroll + 1).min(max_scroll);
                                    }
                                    KeyCode::PageUp => {
                                        active.scroll = active.scroll.saturating_sub(page);
                                    }
                                    KeyCode::PageDown => {
                                        active.scroll = (active.scroll + page).min(max_scroll);
                                    }
                                    KeyCode::Home => active.scroll = 0,
                                    KeyCode::End => active.scroll = max_scroll,
                                    _ => {}
                                }
                                }
                            }
                            if close {
                                modal = None;
                                transcript_search = None;
                                search_hits.clear();
                                inbox_hits.clear();
                            }
                            match modal_action {
                                Some('i') => {
                                    let entry = inbox_hits.get(inbox_select).cloned();
                                    inbox_hits.clear();
                                    if let Some(entry) = entry {
                                        let linked =
                                            entry.session_id.filter(|s| !s.is_empty());
                                        match linked {
                                            None => screen.dim(
                                                "this entry has no linked session",
                                            ),
                                            Some(sid) => {
                                                match locate_session_dir(
                                                    &core.sessions_home(),
                                                    core.cwd(),
                                                    &sid,
                                                )
                                                .ok_or_else(|| {
                                                    format!("session {sid} not found on disk")
                                                })
                                                .and_then(|dir| {
                                                    session_transcript_modal(
                                                        &dir,
                                                        &sid,
                                                        "opened read-only from inbox",
                                                    )
                                                }) {
                                                    Ok(m) => modal = Some(m),
                                                    Err(e) => screen.error(&format!(
                                                        "cannot open {} · {e}",
                                                        short_hex(&sid)
                                                    )),
                                                }
                                            }
                                        }
                                    }
                                }
                                Some('o') => {
                                    let hit = search_hits.get(search_select).cloned();
                                    search_hits.clear();
                                    if let Some(hit) = hit {
                                        match session_view_modal(
                                            &core.sessions_home(),
                                            core.cwd(),
                                            &hit,
                                        ) {
                                            Ok(m) => modal = Some(m),
                                            Err(e) => screen.error(&format!(
                                                "cannot open {} · {e}",
                                                short_hex(&hit.session_id)
                                            )),
                                        }
                                    }
                                }
                                Some('p') => {
                                    modal = None;
                                    picker = Some((
                                        PickerKind::Provider,
                                        ChoicePicker::new(
                                            provider_choices(&core, &ui.provider),
                                            false,
                                        ),
                                    ));
                                }
                                Some('m') => {
                                    modal = None;
                                    picker = Some((
                                        PickerKind::Model,
                                        ChoicePicker::new(
                                            model_choices(&core, &ui.provider, &ui.model).await,
                                            true,
                                        ),
                                    ));
                                }
                                Some('f') => {
                                    modal = Some(ModalView {
                                        title: "feature explorer".to_string(),
                                        rows: feature_rows(&core),
                                        scroll: 0,
                                        footer: "All implemented surfaces · ↑↓ scroll · Esc close"
                                            .to_string(),
                                        ..Default::default()
                                    });
                                }
                                Some('t') => {
                                    modal = None;
                                    let current = core.effective_theme();
                                    theme_preview_origin = Some(current.clone());
                                    picker = Some((
                                        PickerKind::Theme,
                                        ChoicePicker::new(theme_choices(&current, &custom_themes), false),
                                    ));
                                }
                                Some('e') => match export_transcript(&session_slot, core.cwd()).await
                                {
                                    Ok(path) => screen.accent(&format!(
                                        "transcript exported → {}",
                                        path.display()
                                    )),
                                    Err(e) => screen.error(&format!("export failed: {e}")),
                                },
                                _ => {}
                            }
                            if let Some(active) = modal.as_ref() {
                                screen.redraw_modal(
                                    &active.title,
                                    &active.rows,
                                    active.scroll,
                                    &active.footer,
                                    active.selected,
                                );
                            } else if let Some((kind, active)) = picker.as_ref() {
                                draw_picker(&mut screen, *kind, active);
                            } else {
                                let (buf, cur) = editor.view();
                                screen.redraw_composer(
                                    &composer_label(&ui, inbox_unread),
                                    buf,
                                    cur,
                                    &composer_footer(&editor),
                                );
                            }
                            continue;
                        }
                        // The subagent picker stays live during runs: attach
                        // is exactly for steering a run in flight.
                        let picker_usable = picker.is_some()
                            && (!is_running || matches!(picker, Some((PickerKind::Subagents, _))));
                        if picker_usable {
                            let mut choice = None;
                            let mut close = false;
                            let mut preview = None;
                            if let Some((kind, active)) = picker.as_mut() {
                                match key.code {
                                    KeyCode::Esc => close = true,
                                    KeyCode::Up => active.up(),
                                    KeyCode::Down => active.down(),
                                    KeyCode::Backspace => active.backspace(),
                                    KeyCode::Enter => {
                                        choice = active.value().map(|value| (*kind, value, false));
                                    }
                                    KeyCode::Char('s')
                                        if key.modifiers.contains(
                                            crossterm::event::KeyModifiers::CONTROL,
                                        ) && *kind != PickerKind::Subagents =>
                                    {
                                        choice = active.value().map(|value| (*kind, value, true));
                                    }
                                    KeyCode::Char(c)
                                        if key.modifiers.is_empty()
                                            || key.modifiers
                                                == crossterm::event::KeyModifiers::SHIFT =>
                                    {
                                        active.push(c);
                                    }
                                    _ => {}
                                }
                                if *kind == PickerKind::Theme && choice.is_none() && !close {
                                    preview = active.value();
                                }
                            }
                            if close {
                                picker = None;
                                if let Some(origin) = theme_preview_origin.take() {
                                    core.set_theme(origin.clone());
                                    ui_theme = theme::resolve(&origin, &custom_themes);
                                    screen.set_theme(ui_theme);
                                }
                            } else if let Some(name) = preview {
                                core.set_theme(name.clone());
                                ui_theme = theme::resolve(&name, &custom_themes);
                                screen.set_theme(ui_theme);
                            }
                            if let Some((kind, value, persist)) = choice {
                                match kind {
                                    PickerKind::Provider => {
                                        core.set_provider(value.clone());
                                        ui.provider = value.clone();
                                        if let Some(model) = default_model(&core, &ui.provider).await {
                                            core.set_model(model.clone());
                                            ui.model = model;
                                        }
                                    }
                                    PickerKind::Model => {
                                        core.set_model(value.clone());
                                        ui.model = value.clone();
                                    }
                                    PickerKind::Theme => {
                                        core.set_theme(value.clone());
                                        ui_theme = theme::resolve(&value, &custom_themes);
                                        screen.set_theme(ui_theme);
                                        theme_preview_origin = None;
                                    }
                                    PickerKind::Subagents => {
                                        if let Some(sub) =
                                            core.subagents().active().into_iter().find(|a| a.id == value)
                                        {
                                            attached = Some((sub.id.clone(), sub.label.clone()));
                                            ui.attached_label = Some(sub.label.clone());
                                            screen.accent(&format!(
                                                "\u{25c6} attached to {} · Enter steers it · Esc detaches · Ctrl-C stops it",
                                                sub.label
                                            ));
                                        } else {
                                            screen.dim("that subagent already finished");
                                        }
                                    }
                                }
                                if !matches!(kind, PickerKind::Theme | PickerKind::Subagents) {
                                    screen.update_agent_identity(&ui.provider, &ui.model);
                                }
                                if kind == PickerKind::Subagents {
                                    picker = None;
                                    continue;
                                }
                                picker = None;
                                screen.clear_input();
                                if persist {
                                    let saved = match kind {
                                        PickerKind::Theme => {
                                            Some(persist_theme_config(&core, &value))
                                        }
                                        PickerKind::Provider | PickerKind::Model => Some(
                                            persist_agent_config(&core, &ui.provider, &ui.model),
                                        ),
                                        // The subagent attach flow continues
                                        // before any persist path.
                                        PickerKind::Subagents => None,
                                    };
                                    match saved {
                                        Some(Ok(path)) => screen.success(&format!(
                                            "✓ saved {} → {}",
                                            if kind == PickerKind::Theme {
                                                format!("theme {value}")
                                            } else {
                                                format!("{}/{}", ui.provider, ui.model)
                                            },
                                            path.display()
                                        )),
                                        Some(Err(e)) => {
                                            screen.error(&format!("could not save config: {e}"))
                                        }
                                        None => {}
                                    }
                                } else if kind == PickerKind::Theme {
                                    screen.accent(&format!("theme → {value} for this session"));
                                } else {
                                    screen.accent(&format!(
                                        "using {}/{} for this session",
                                        ui.provider, ui.model
                                    ));
                                }
                            }
                            if let Some((kind, active)) = picker.as_ref() {
                                draw_picker(&mut screen, *kind, active);
                            } else {
                                let (buf, cur) = editor.view();
                                screen.redraw_composer(
                                    &composer_label(&ui, inbox_unread),
                                    buf,
                                    cur,
                                    &composer_footer(&editor),
                                );
                            }
                            continue;
                        }
                        if let Some(active) = palette.as_mut()
                            && !is_running
                        {
                            let mut submit_selection = false;
                            let count = active.items().len();
                            match key.code {
                                KeyCode::Esc => {
                                    if palette_from_slash {
                                        editor.set_text(&format!("/{}", active.query()));
                                    }
                                    palette = None;
                                    palette_from_slash = false;
                                }
                                KeyCode::Char('p')
                                    if key.modifiers.contains(
                                        crossterm::event::KeyModifiers::CONTROL,
                                    ) => palette = None,
                                KeyCode::Up => active.up(count),
                                KeyCode::Down => active.down(count),
                                KeyCode::Backspace => active.backspace(),
                                KeyCode::Enter => {
                                    // Slash mode: the composer already holds
                                    // the full typed command; close the hint and
                                    // let Submit process it verbatim.
                                    palette = None;
                                    palette_from_slash = false;
                                    submit_selection = true;
                                }
                                KeyCode::Tab => {
                                    let items = active.items();
                                    if let Some(item) = items.get(active.selected(items.len())) {
                                        editor.set_text(&format!("/{} ", item.name));
                                    }
                                    palette = None;
                                    palette_from_slash = false;
                                }
                                KeyCode::Char(c)
                                    if key.modifiers.is_empty()
                                        || key.modifiers
                                            == crossterm::event::KeyModifiers::SHIFT =>
                                {
                                    active.push(c);
                                }
                                _ => {}
                            }
                            if let Some(active) = palette.as_ref() {
                                let items = active.items();
                                screen.redraw_palette(
                                    active.query(),
                                    &items,
                                    active.selected(items.len()),
                                );
                            } else {
                                let (buf, cur) = editor.view();
                                screen.redraw_composer(
                                    &composer_label(&ui, inbox_unread),
                                    buf,
                                    cur,
                                    &composer_footer(&editor),
                                );
                            }
                            if !submit_selection {
                                continue;
                            }
                        }
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
                            let q = editor.search_query().to_string();
                            let (buf, cur) = editor.view();
                            if editor.search_active() {
                                screen.redraw_composer(
                                    &format!("history search · {q}"),
                                    buf,
                                    cur,
                                    "Ctrl-R older · Enter accept · Esc restore",
                                );
                            } else {
                                screen.redraw_composer(
                                    &composer_label(&ui, inbox_unread),
                                    buf,
                                    cur,
                                    &composer_footer(&editor),
                                );
                            }
                            continue;
                        }
                        // Vim modal editing intercepts keys in normal mode
                        // and Esc-in-insert; everything else falls through.
                        if !is_running && editor.mode() == ComposerMode::Vim {
                            let consumed = if editor.vim_state() == VimState::Insert {
                                if key.code == KeyCode::Esc && key.modifiers.is_empty() {
                                    editor.enter_normal();
                                    true
                                } else {
                                    false
                                }
                            } else {
                                editor.vim_normal_key(key.code, key.modifiers)
                            };
                            if consumed {
                                continue;
                            }
                        }
                        match keymap.action_for(is_running, key.code, key.modifiers) {
                            Action::Insert(c) => {
                                if is_running {
                                    pending.push(c);
                                } else {
                                    editor.insert(c);
                                    // Keep the slash-palette hint in sync with
                                    // the composer (source of truth) so Enter
                                    // preserves typed arguments.
                                    if palette_from_slash
                                        && let Some(active) = palette.as_mut()
                                    {
                                        let (buf, _) = editor.view();
                                        active.set_query(buf.trim_start_matches('/'));
                                    }
                                }
                            }
                            Action::Backspace => {
                                if is_running {
                                    let _ = pending.pop();
                                } else {
                                    editor.backspace();
                                    if palette_from_slash
                                        && let Some(active) = palette.as_mut()
                                    {
                                        let (buf, _) = editor.view();
                                        active.set_query(buf.trim_start_matches('/'));
                                    }
                                }
                            }
                            Action::Delete => editor.delete(),
                            Action::Left => editor.left(),
                            Action::Right => editor.right(),
                            Action::WordLeft => editor.word_left(),
                            Action::WordRight => editor.word_right(),
                            Action::DeleteWordBack => editor.delete_word_back(),
                            Action::DeleteWordForward => editor.delete_word_forward(),
                            Action::DeleteToLineEnd => editor.delete_to_line_end(),
                            Action::Undo => {
                                if !is_running {
                                    editor.undo();
                                }
                            }
                            Action::Redo => {
                                if !is_running {
                                    editor.redo();
                                }
                            }
                            Action::ToggleThinking => {
                                ui.thinking_mode = ui.thinking_mode.next();
                                screen.clear_input();
                                screen.dim(&format!(
                                    "thinking display → {}",
                                    ui.thinking_mode.name()
                                ));
                            }
                            Action::CommandPalette => {
                                if !is_running {
                                    palette =
                                        Some(CommandPalette::new(palette_extras(&core)));
                                    palette_from_slash = false;
                                }
                            }
                            Action::ClearViewport => {
                                screen.clear_viewport();
                            }
                            Action::OpenApproval => {
                                if !approvals.is_empty() {
                                    approval_focused = true;
                                }
                            }
                            Action::Subagents => open_subagent_picker(
                                &core, &mut screen, &mut picker,
                            ),
                            Action::CopyResponse => copy_last_response(&core, &mut screen, &ui),
                            Action::ExpandStash => {
                                if !is_running
                                    && let Some(msg) = editor.expand_stash_at_cursor()
                                {
                                    screen.dim(&msg);
                                }
                            }
                            Action::ExternalEditor => {
                                if !is_running && approvals.is_empty() {
                                    open_external_editor(
                                        &mut editor,
                                        &mut raw,
                                        &mut reader,
                                        &mut screen,
                                        &draft_path,
                                    );
                                } else if is_running {
                                    screen.dim("external editor unavailable while a run is active");
                                }
                            }
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
                                        if let Some((id, label)) = attached.clone() {
                                            if core.subagents().steer(&id, &text) {
                                                screen.clear_input();
                                                screen.dim(&format!("[steered {label}]"));
                                            } else {
                                                attached = None;
                                                ui.attached_label = None;
                                                steering.push_steering(text);
                                                screen.clear_input();
                                                screen.dim(
                                                    "[subagent ended — steered the main run]",
                                                );
                                            }
                                        } else {
                                            steering.push_steering(text);
                                            screen.clear_input();
                                            screen.dim("[steering queued]");
                                        }
                                    }
                                } else {
                                    let mut text = editor.take();
                                    if text.trim().is_empty() {
                                        continue;
                                    }
                                    editor.save_history(&hist_path);
                                    screen.clear_input();
                                    // /goal bypasses the command table: its
                                    // argument must reach the handler byte-exact.
                                    if text.trim_start().starts_with("/goal") {
                                        let arg = text.trim_start()[5..].trim().to_string();
                                        text.clear();
                                        if arg.is_empty() {
                                            let armed = pending_goal
                                                .lock()
                                                .unwrap_or_else(std::sync::PoisonError::into_inner)
                                                .is_some();
                                            if armed {
                                                screen.dim("🎯 goal armed — next prompt runs audited; /goal off to disarm");
                                            } else {
                                                screen.dim("no goal armed · usage: /goal <objective> [-- criterion1; criterion2]");
                                            }
                                        } else if arg == "off" {
                                            *pending_goal
                                                .lock()
                                                .unwrap_or_else(std::sync::PoisonError::into_inner) = None;
                                            screen.dim("goal disarmed");
                                        } else {
                                            let (objective, criteria) = split_goal_spec(&arg);
                                            if criteria.is_empty() {
                                                screen.error("goal needs criteria: /goal <objective> -- c1; c2");
                                            } else {
                                                *pending_goal
                                                    .lock()
                                                    .unwrap_or_else(std::sync::PoisonError::into_inner) =
                                                    Some((objective.clone(), criteria.clone()));
                                                screen.success(&format!(
                                                    "🎯 goal armed ({} criteria) — next prompt will be audited: {objective}",
                                                    criteria.len()
                                                ));
                                            }
                                        }
                                        continue;
                                    }
                                    if text.trim_start().starts_with('/') {
                                        match commands::parse(&text) {
                                            Some(Command::Exit) => break,
                                            Some(Command::Help) => {
                                                modal = Some(ModalView {
                                                    title: "help · commands".to_string(),
                                                    rows: help_modal_rows(&core),
                                                    scroll: 0,
                                                    footer: "↑↓ scroll · /keys shortcuts · Esc close".to_string(),
                                                    ..Default::default()
                                                });
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
                                                    None => {
                                                        let current = core.effective_theme();
                                                        theme_preview_origin = Some(current.clone());
                                                        let active = ChoicePicker::new(
                                                            theme_choices(&current, &custom_themes),
                                                            false,
                                                        );
                                                        draw_picker(
                                                            &mut screen,
                                                            PickerKind::Theme,
                                                            &active,
                                                        );
                                                        picker = Some((PickerKind::Theme, active));
                                                    }
                                                    Some(name)
                                                        if theme::builtin(name.as_str())
                                                            || custom_themes.contains_key(&name) =>
                                                    {
                                                        core.set_theme(name.clone());
                                                        ui_theme =
                                                            theme::resolve(&name, &custom_themes);
                                                        screen.set_theme(ui_theme);
                                                        screen.accent(&format!("theme → {name}"));
                                                    }
                                                    Some(other) => screen.dim(&format!(
                                                        "unknown theme '{other}' — {}",
                                                        crate::theme::all_names(&custom_themes)
                                                            .join(", "),
                                                    )),
                                                },
                                                Some(Command::Transcript(arg)) => {
                                                    match transcript_modal(&session_slot, arg.as_deref())
                                                        .await
                                                    {
                                                        Some(m) => modal = Some(m),
                                                        None => screen.dim("no active session"),
                                                    }
                                                }
                                                Some(Command::View(arg)) => {
                                                    match arg.as_deref().map(str::trim).filter(|a| !a.is_empty()) {
                                                        None => screen.error("usage: /view <path>"),
                                                        Some(rel) => match view_file_modal(&core, rel) {
                                                            Ok(m) => modal = Some(m),
                                                            Err(e) => screen.error(&e),
                                                        },
                                                    }
                                                }
                                                Some(Command::Doctor) => {
                                                    let session = session_slot.lock().await.take();
                                                    run_doctor(&core, session.as_ref(), &mut screen);
                                                    if let Some(s) = session {
                                                        *session_slot.lock().await = Some(s);
                                                    }
                                                }
                                                Some(Command::Services(arg)) => {
                                                    run_services(arg, &mut screen);
                                                }
                                                Some(Command::Mode(arg)) => {
                                                    match arg
                                                        .as_deref()
                                                        .map(str::trim)
                                                        .filter(|a| !a.is_empty())
                                                    {
                                                        None => {
                                                            modal = Some(ModalView {
                                                                title: "permission mode".to_string(),
                                                                rows: mode_modal_rows(&core),
                                                                scroll: 0,
                                                                footer: "/mode <mode> switches · switching cancels in-flight runs · Esc close"
                                                                    .to_string(),
                                                                ..Default::default()
                                                            });
                                                        }
                                                        Some(raw) => {
                                                            match PermissionMode::deserialize_str(raw) {
                                                                None => screen.error(&format!(
                                                                    "unknown mode '{raw}' — read-only, workspace-write, full-access"
                                                                )),
                                                                Some(mode)
                                                                    if mode == core.effective_permission_mode() =>
                                                                {
                                                                    screen.dim(&format!(
                                                                        "already in {}",
                                                                        mode_label(mode)
                                                                    ));
                                                                }
                                                                Some(mode) => {
                                                                    // Invariant 11: pending
                                                                    // approvals die and in-flight
                                                                    // runs stop before the new
                                                                    // mode is reported.
                                                                    while let Some(req) =
                                                                        approvals.pop_front()
                                                                    {
                                                                        let _ = req.respond.send(false);
                                                                    }
                                                                    allowed.lock().await.clear();
                                                                    if let Some((id, label)) =
                                                                        attached.take()
                                                                    {
                                                                        ui.attached_label = None;
                                                                        let _ = core.subagents().stop(&id);
                                                                        screen.dim(&format!(
                                                                            "[stopped subagent {label}]"
                                                                        ));
                                                                    }
                                                                    if running_now {
                                                                        cancel.lock().await.cancel();
                                                                        deferred_mode = Some(mode);
                                                                        screen.accent(
                                                                            "cancelling in-flight run — mode applies when it stops",
                                                                        );
                                                                    } else {
                                                                        core.set_permission_mode(mode);
                                                                        screen.accent(&format!(
                                                                            "mode → {}",
                                                                            mode_label(mode)
                                                                        ));
                                                                    }
                                                                }
                                                            }
                                                        }
                                                    }
                                                }
                                                Some(Command::Compact) => {
                                                    if running_now {
                                                        screen.error(
                                                            "cannot compact while a run is active — Esc to cancel first",
                                                        );
                                                    } else {
                                                        let taken = session_slot.lock().await.take();
                                                        match taken {
                                                            None => screen.dim("no active session"),
                                                            Some(session) => {
                                                                if core.provider().is_err() {
                                                                    *session_slot.lock().await = Some(session);
                                                                    screen.error("provider unavailable — nothing compacted");
                                                                } else {
                                                                    screen.dim("compacting…");
                                                                    let (session, outcome) = core
                                                                        .compact_session_now(
                                                                            session,
                                                                            CancellationToken::new(),
                                                                        )
                                                                        .await;
                                                                    *session_slot.lock().await = Some(session);
                                                                    match (outcome.report, outcome.error) {
                                                                        (Some(rep), _) => screen.success(&format!(
                                                                            "compacted: ~{} → ~{} tokens ({} messages summarized)",
                                                                            status::fmt_tokens(rep.before_tokens),
                                                                            status::fmt_tokens(rep.after_tokens),
                                                                            rep.summarized_messages,
                                                                        )),
                                                                        (None, Some(e)) => {
                                                                            screen.error(&format!("compaction failed: {e}"))
                                                                        }
                                                                        (None, None) => screen.dim(
                                                                            "nothing to compact — no older message prefix",
                                                                        ),
                                                                    }
                                                                }
                                                            }
                                                        }
                                                    }
                                                }
                                                Some(Command::Budget) => {
                                                    let f = &core.config().finops;
                                                    let day = core.spend_day_usd();
                                                    let week = core.spend_trailing_usd(7);
                                                    let mut rows = vec![format!(
                                                        "today        ~${:.2}",
                                                        day
                                                    )];
                                                    rows.push(format!("trailing 7d  ~${week:.2}"));
                                                    rows.push(String::new());
                                                    rows.push(format!(
                                                        "run cap   {}",
                                                        f.max_run_usd
                                                            .map(|c| format!("${c:.2} per run"))
                                                            .unwrap_or_else(|| "unset".into())
                                                    ));
                                                    rows.push(format!(
                                                        "day cap   {}",
                                                        f.max_day_usd
                                                            .map(|c| format!("${c:.2} per day"))
                                                            .unwrap_or_else(|| "unset".into())
                                                    ));
                                                    if !f.price_overrides.is_empty() {
                                                        rows.push(format!(
                                                            "{} custom price override(s)",
                                                            f.price_overrides.len()
                                                        ));
                                                    }
                                                    modal = Some(ModalView {
                                                        title: "budget · finops".to_string(),
                                                        rows,
                                                        scroll: 0,
                                                        footer: "caps live under [finops] in config.toml · estimates only · Esc close"
                                                            .to_string(),
                                                        ..Default::default()
                                                    });
                                                }
                                                Some(Command::Memory(arg)) => {
                                                    let home = core.sessions_home();
                                                    let cwd = core.cwd().clone();
                                                    let raw = arg.as_deref().map(str::trim);
                                                    let (profile_scope, rest) = match raw {
                                                        Some(rest)
                                                            if rest == "--profile"
                                                                || rest
                                                                    .starts_with("--profile ") =>
                                                        {
                                                            let tail = rest
                                                                ["--profile".len()..]
                                                                .trim()
                                                                .to_string();
                                                            (true, Some(tail))
                                                        }
                                                        other => (false, other.map(String::from)),
                                                    };
                                                    let notes = if profile_scope {
                                                        vak_core::memory::list_profile_notes(&home)
                                                    } else {
                                                        vak_core::memory::list_notes(&home, &cwd)
                                                    };
                                                    let memory_path = if profile_scope {
                                                        vak_core::memory::profile_path(&home)
                                                    } else {
                                                        home.join("memory")
                                                            .join(vak_core::memory::hash_cwd(&cwd))
                                                            .join("MEMORY.md")
                                                    };
                                                    // Forms: bare list · <text> append ·
                                                    // forget <id> · amend <id> <text>.
                                                    let action = MemoryAction::parse(
                                                        rest.as_deref(),
                                                    );
                                                    match action {
                                                        MemoryAction::List => {
                                                            let tier = if profile_scope {
                                                                "profile · USER.md"
                                                            } else {
                                                                "workspace"
                                                            };
                                                            let rows: Vec<String> = if notes.is_empty() {
                                                                vec![
                                                                    format!("no {tier} notes yet"),
                                                                    "the model saves via its remember tool; /memory <text> saves one directly".into(),
                                                                    "/memory --profile <text> saves to the USER.md tier that follows you across projects".into(),
                                                                ]
                                                            } else {
                                                                notes
                                                                    .iter()
                                                                    .rev()
                                                                    .map(|n| {
                                                                        format!(
                                                                            "{} [{}] {} · {}",
                                                                            short_hex(&n.id),
                                                                            n.kind,
                                                                            n.tag,
                                                                            n.text.replace('\n', " ")
                                                                        )
                                                                    })
                                                                    .collect()
                                                            };
                                                            modal = Some(ModalView {
                                                                title: format!(
                                                                    "memory · {tier} · {} note(s)",
                                                                    notes.len()
                                                                ),
                                                                rows,
                                                                scroll: 0,
                                                                footer: "/memory forget <id> · /memory amend <id> <text> · ids accept the 8-char prefix · Esc close".to_string(),
                                                                ..Default::default()
                                                            });
                                                        }
                                                        MemoryAction::Append(text) => {
                                                            let session_id = session_slot
                                                                .lock()
                                                                .await
                                                                .as_ref()
                                                                .and_then(|s| s.header())
                                                                .map(|h| h.session_id.clone())
                                                                .unwrap_or_else(|| "manual".into());
                                                            let saved = if profile_scope {
                                                                vak_core::memory::append_profile_note(
                                                                    &home, "note", "", &text, &session_id,
                                                                )
                                                            } else {
                                                                vak_core::memory::append_note(
                                                                    &home,
                                                                    &cwd,
                                                                    "note",
                                                                    "",
                                                                    &session_id,
                                                                    &text,
                                                                )
                                                            };
                                                            match saved {
                                                                Ok(_) => screen.success(if profile_scope {
                                                                    "note saved to USER.md profile"
                                                                } else {
                                                                    "note saved to durable memory"
                                                                }),
                                                                Err(e) => screen.error(&format!("save failed: {e}")),
                                                            }
                                                        }
                                                        MemoryAction::Forget(given) => {
                                                            if given.is_empty() {
                                                                screen.error(
                                                                    "usage: /memory forget <id> — ids show in /memory",
                                                                );
                                                            } else {
                                                                match find_note(&notes, &given) {
                                                                    Err(e) => screen.error(&e.to_string()),
                                                                    Ok(note) => {
                                                                        confirm_forget = Some(ForgetConfirm {
                                                                            path: memory_path.clone(),
                                                                            id: note.id.clone(),
                                                                            preview: trunc_cells(
                                                                                &note.text.replace('\n', " "),
                                                                                80,
                                                                            ),
                                                                        });
                                                                    }
                                                                }
                                                            }
                                                        }
                                                        MemoryAction::Amend(given, text) => {
                                                            if given.is_empty() || text.trim().is_empty() {
                                                                screen.error(
                                                                    "usage: /memory amend <id> <new text>",
                                                                );
                                                            } else {
                                                                let resolved = find_note(&notes, &given);
                                                                match resolved {
                                                                    Err(e) => screen.error(&e.to_string()),
                                                                    Ok(note) => {
                                                                        let full = note.id.clone();
                                                                        match vak_core::memory::amend_note(
                                                                            &memory_path, &full, &text,
                                                                        ) {
                                                                            Ok(()) => screen.success(&format!(
                                                                                "✓ amended note {}",
                                                                                short_hex(&full),
                                                                            )),
                                                                            Err(e) => screen.error(&format!("amend failed: {e}")),
                                                                        }
                                                                    }
                                                                }
                                                            }
                                                        }
                                                    }
                                                }
                                                Some(Command::Search(spec)) => match spec {
                                                    None => screen.error("usage: /search <query> [--all]"),
                                                    Some((query, all)) => {
                                                        let home = core.sessions_home();
                                                        let current = session_slot
                                                            .lock()
                                                            .await
                                                            .as_ref()
                                                            .and_then(|s| s.header())
                                                            .map(|h| h.session_id.clone());
                                                        let hits = if all {
                                                            vak_session::search::search_all(
                                                                &home,
                                                                &query,
                                                                vak_session::search::DEFAULT_LIMIT,
                                                                current.as_deref(),
                                                            )
                                                            .map(|hs| {
                                                                hs.into_iter()
                                                                    .map(|p| SearchHit {
                                                                        session_id: p.hit.session_id,
                                                                        project_hash: p.project_hash,
                                                                        ts: p.hit.ts,
                                                                        score: p.hit.score,
                                                                        snippet: p.hit.snippet,
                                                                    })
                                                                    .collect::<Vec<_>>()
                                                            })
                                                        } else {
                                                            vak_session::search::search(
                                                                &home,
                                                                core.cwd(),
                                                                &query,
                                                                vak_session::search::DEFAULT_LIMIT,
                                                                current.as_deref(),
                                                            )
                                                            .map(|hs| {
                                                                hs.into_iter()
                                                                    .map(|h| SearchHit {
                                                                        session_id: h.session_id,
                                                                        project_hash: String::new(),
                                                                        ts: h.ts,
                                                                        score: h.score,
                                                                        snippet: h.snippet,
                                                                    })
                                                                    .collect::<Vec<_>>()
                                                            })
                                                        };
                                                        match hits {
                                                            Err(e) => screen.error(&format!("search failed: {e}")),
                                                            Ok(hs) if hs.is_empty() => screen.dim(&format!(
                                                                "no matches for '{query}'{}",
                                                                if all { " across all projects" } else { "" },
                                                            )),
                                                            Ok(hs) => {
                                                                let mut rows = vec![format!(
                                                                    "'{query}' · {} hit(s){}",
                                                                    hs.len(),
                                                                    if all { " · all projects" } else { "" },
                                                                )];
                                                                for (i, h) in hs.iter().enumerate() {
                                                                    let project =
                                                                        if all && !h.project_hash.is_empty() {
                                                                            format!(
                                                                                " [{}]",
                                                                                short_hex(&h.project_hash),
                                                                            )
                                                                        } else {
                                                                            String::new()
                                                                        };
                                                                    rows.push(format!(
                                                                        "{:>2}. {}{} · {} · {:>4.1}  {}",
                                                                        i + 1,
                                                                        short_hex(&h.session_id),
                                                                        project,
                                                                        rel_time(h.ts),
                                                                        h.score,
                                                                        trunc_cells(&h.snippet, 160),
                                                                    ));
                                                                }
                                                                search_select = 0;
                                                                modal = Some(ModalView {
                                                                    title: "session search".to_string(),
                                                                    rows,
                                                                    scroll: 0,
                                                                    footer: "↑↓ select · Enter opens read-only · /search <q> --all spans projects · Esc close"
                                                                        .to_string(),
                                                                    selected: Some(0),
                                                                    ..Default::default()
                                                                });
                                                                search_hits = hs;
                                                            }
                                                        }
                                                    }
                                                },
                                                Some(Command::Tasks(arg)) => {
                                                    handle_tasks(&core, arg.as_deref(), &mut screen);
                                                }
                                                Some(Command::Proposals(arg)) => {
                                                    let home = core.sessions_home();
                                                    let cwd = core.cwd().clone();
                                                    let parsed = arg.as_deref().and_then(|a| {
                                                        let mut it = a.split_whitespace();
                                                        let action = it.next()?;
                                                        let id = it.next()?.trim();
                                                        Some((action.to_lowercase(), id.to_string()))
                                                    });
                                                    match parsed {
                                                        Some((action, id))
                                                            if action == "promote" || action == "accept" =>
                                                        {
                                                            match vak_core::learning::promote(&home, &cwd, &id) {
                                                                Ok(name) => screen.success(&format!(
                                                                    "skill '{name}' promoted to ~/.vakcoder/skills"
                                                                )),
                                                                Err(e) => screen.error(&e),
                                                            }
                                                        }
                                                        Some((action, id))
                                                            if action == "reject" || action == "deny" =>
                                                        {
                                                            match vak_core::learning::reject(&home, &cwd, &id) {
                                                                Ok(()) => screen.accent(&format!("proposal {id} rejected")),
                                                                Err(e) => screen.error(&e),
                                                            }
                                                        }
                                                        Some((action, _)) => screen.error(&format!(
                                                            "unknown proposals action '{action}' — promote <id>, reject <id>"
                                                        )),
                                                        None => {
                                                            let props = vak_core::learning::list_proposals(&home, &cwd);
                                                            let rows: Vec<String> = if props.is_empty() {
                                                                vec![
                                                                    "no pending skill proposals".into(),
                                                                    "the model proposes skills after repeated successful patterns; review them here".into(),
                                                                ]
                                                            } else {
                                                                props
                                                                    .iter()
                                                                    .flat_map(|p| {
                                                                        [
                                                                            format!("/{}", p.id),
                                                                            format!("  {}", p.name),
                                                                            format!("  {}", p.description),
                                                                            String::new(),
                                                                        ]
                                                                    })
                                                                    .collect()
                                                            };
                                                            modal = Some(ModalView {
                                                                title: format!(
                                                                    "skill proposals · {} pending",
                                                                    props.len()
                                                                ),
                                                                rows,
                                                                scroll: 0,
                                                                footer: "/proposals promote <id> installs · /proposals reject <id> discards · Esc close"
                                                                    .to_string(),
                                                                ..Default::default()
                                                            });
                                                        }
                                                    }
                                                }
                                                Some(Command::Inbox(arg)) => {
                                                    // Any /inbox touch refreshes the
                                                    // badge cache immediately.
                                                    let home = core.sessions_home();
                                                    inbox_unread =
                                                        vak_core::inbox::unread_count(&home);
                                                    inbox_next_poll = Instant::now()
                                                        + Duration::from_secs(
                                                            INBOX_REFRESH_SECS,
                                                        );
                                                    match InboxAction::parse(arg.as_deref()) {
                                                        Err(usage) => screen.error(&usage),
                                                        Ok(InboxAction::List { all }) => {
                                                            let entries = if all {
                                                                vak_core::inbox::list(
                                                                    &home,
                                                                    vak_core::inbox::MAX_SCAN,
                                                                )
                                                            } else {
                                                                vak_core::inbox::unread(&home)
                                                            };
                                                            let total = if all {
                                                                entries.len()
                                                            } else {
                                                                vak_core::inbox::list(
                                                                    &home,
                                                                    vak_core::inbox::MAX_SCAN,
                                                                )
                                                                .len()
                                                            };
                                                            let rows = if entries.is_empty()
                                                                && total == 0
                                                            {
                                                                vec![
                                                                    "no inbox entries yet".into(),
                                                                    "gateway pushes, task summaries, and budget alerts land here".into(),
                                                                    "/inbox ack <id8> marks an entry read".into(),
                                                                ]
                                                            } else if entries.is_empty() {
                                                                vec![
                                                                    "no unread entries".into(),
                                                                    format!(
                                                                        "{total} already read — /inbox all shows them"
                                                                    ),
                                                                ]
                                                            } else {
                                                                inbox_rows(&entries, &ui_theme)
                                                            };
                                                            inbox_select = 0;
                                                            inbox_hits = entries;
                                                            modal = Some(ModalView {
                                                                title: "inbox".to_string(),
                                                                rows,
                                                                scroll: 0,
                                                                footer: format!(
                                                                    "{} unread of {} · ↑↓ select · Enter opens linked session · /inbox ack <id8>",
                                                                    inbox_unread, total
                                                                ),
                                                                selected: (!inbox_hits.is_empty())
                                                                    .then_some(0),
                                                                ..Default::default()
                                                            });
                                                        }
                                                        Ok(InboxAction::Ack(given)) => {
                                                            // Unread entries resolve
                                                            // first; the full ledger
                                                            // is the fallback tier.
                                                            let unread_list =
                                                                vak_core::inbox::unread(&home);
                                                            let all_list = vak_core::inbox::list(
                                                                &home,
                                                                vak_core::inbox::MAX_SCAN,
                                                            );
                                                            match resolve_entry(
                                                                &unread_list,
                                                                &all_list,
                                                                &given,
                                                            ) {
                                                                Err(e) => {
                                                                    screen.error(&e.to_string())
                                                                }
                                                                Ok(entry) => {
                                                                    confirm_ack = Some(InboxAck {
                                                                        id: entry.id.clone(),
                                                                        preview: inbox_title(
                                                                            &entry.title,
                                                                            80,
                                                                        ),
                                                                    });
                                                                }
                                                            }
                                                        }
                                                    }
                                                }
                                                 Some(Command::Mcp) => {
                                                    let mut rows: Vec<String> = Vec::new();
                                                    let servers = &core.config().mcp.servers;
                                                    if servers.is_empty() {
                                                        rows.push("no MCP servers configured".into());
                                                        rows.push("add [mcp.servers.<name>] blocks to config.toml".into());
                                                    }
                                                    for (name, def) in servers {
                                                        rows.push(name.clone());
                                                        rows.push(format!("  cmd  {} {}", def.command, def.args.join(" ")));
                                                    }
                                                    let mcp_tools: Vec<String> = core
                                                        .tool_names()
                                                        .into_iter()
                                                        .filter(|n| n.starts_with("mcp("))
                                                        .collect();
                                                    if !mcp_tools.is_empty() {
                                                        rows.push(String::new());
                                                        rows.push(format!(
                                                            "{} discovered meta-tool(s):",
                                                            mcp_tools.len()
                                                        ));
                                                        for t in mcp_tools {
                                                            rows.push(format!("  {t}"));
                                                        }
                                                    }
                                                    modal = Some(ModalView {
                                                        title: "mcp servers".to_string(),
                                                        rows,
                                                        scroll: 0,
                                                        footer: "servers execute as sandboxed workers · Esc close".to_string(),
                                                        ..Default::default()
                                                    });
                                                }
                                                Some(Command::Sandbox(arg)) => {
                                                    match arg
                                                        .as_deref()
                                                        .map(str::trim)
                                                        .filter(|a| !a.is_empty())
                                                    {
                                                        None => {
                                                            modal = Some(ModalView {
                                                                title: "execution sandbox".to_string(),
                                                                rows: sandbox_rows(&core),
                                                                scroll: 0,
                                                                footer: "/sandbox os|docker|default switches for this session · full-access needs none · Esc close"
                                                                    .to_string(),
                                                                ..Default::default()
                                                            });
                                                        }
                                                        Some(raw) => match raw.to_lowercase().as_str() {
                                                            "default" | "config" | "reset" => {
                                                                core.set_sandbox_backend(None);
                                                                screen.accent(&format!(
                                                                    "sandbox backend → config default ({})",
                                                                    core.effective_sandbox_name()
                                                                ));
                                                            }
                                                            "os" | "landlock" | "seatbelt" => {
                                                                core.set_sandbox_backend(Some("os".into()));
                                                                screen.accent(&format!(
                                                                    "sandbox backend → OS-native ({})",
                                                                    core.effective_sandbox_name()
                                                                ));
                                                            }
                                                            "docker" => {
                                                                if !vak_core::sandbox_docker::DockerSandbox::available() {
                                                                    screen.error(
                                                                        "docker daemon unreachable — commands would fail closed at call time; start docker or keep 'os'",
                                                                    );
                                                                } else {
                                                                    core.set_sandbox_backend(Some("docker".into()));
                                                                    screen.accent(&format!(
                                                                        "sandbox backend → docker ({})",
                                                                        core.effective_sandbox_name()
                                                                    ));
                                                                }
                                                            }
                                                            other => screen.error(&format!(
                                                                "unknown sandbox '{other}' — os, docker, default"
                                                            )),
                                                        },
                                                    }
                                                }
                                                Some(Command::Details) => {
                                                    ui.expanded_tools = !ui.expanded_tools;
                                                    let state = if ui.expanded_tools {
                                                        "expanded"
                                                    } else {
                                                        "compact"
                                                    };
                                                    screen.accent(&format!(
                                                        "tool output → {state}"
                                                    ));
                                                }
                                                Some(Command::Keys(Some(arg)))
                                                    if arg == "raw" || arg == "capture" =>
                                                {
                                                    key_capture = true;
                                                    screen.dim(
                                                        "literal-key capture on · press any keys · Esc exits",
                                                    );
                                                }
                                                Some(Command::Keys(_)) => {
                                                    modal = Some(ModalView {
                                                        title: "keyboard shortcuts".to_string(),
                                                        rows: commands::keys_text()
                                                            .lines()
                                                            .skip(1)
                                                            .map(str::to_string)
                                                            .collect(),
                                                        scroll: 0,
                                                        footer: "↑↓ scroll · Esc close".to_string(),
                                                        ..Default::default()
                                                    });
                                                }
                                                Some(Command::Keymap) => {
                                                    keymap_select = None;
                                                    let (m, meta) =
                                                        build_keymap_modal(&keymap, keymap_select);
                                                    keymap_meta = meta;
                                                    modal = Some(m);
                                                }
                                            Some(Command::Model(Some(model))) => {
                                                core.set_model(model.clone());
                                                ui.model = model;
                                                screen.update_agent_identity(
                                                    &ui.provider,
                                                    &ui.model,
                                                );
                                                screen.accent(&format!(
                                                    "using {}/{} for this session",
                                                    ui.provider, ui.model
                                                ));
                                            }
                                            Some(Command::Model(None)) => {
                                                let active = ChoicePicker::new(
                                                    model_choices(&core, &ui.provider, &ui.model).await,
                                                    true,
                                                );
                                                draw_picker(&mut screen, PickerKind::Model, &active);
                                                picker = Some((PickerKind::Model, active));
                                            }
                                            Some(Command::Provider(Some(provider))) => {
                                                if core.provider_names().contains(&provider) {
                                                    core.set_provider(provider.clone());
                                                    ui.provider = provider;
                                                    if let Some(model) = default_model(&core, &ui.provider).await {
                                                        core.set_model(model.clone());
                                                        ui.model = model;
                                                    }
                                                    screen.update_agent_identity(
                                                        &ui.provider,
                                                        &ui.model,
                                                    );
                                                    screen.accent(&format!(
                                                        "using {}/{} for this session",
                                                        ui.provider, ui.model
                                                    ));
                                                } else {
                                                    screen.error("unknown provider — run /provider to choose");
                                                }
                                            }
                                            Some(Command::Provider(None)) => {
                                                let active = ChoicePicker::new(
                                                    provider_choices(&core, &ui.provider),
                                                    false,
                                                );
                                                draw_picker(
                                                    &mut screen,
                                                    PickerKind::Provider,
                                                    &active,
                                                );
                                                picker = Some((PickerKind::Provider, active));
                                            }
                                            Some(Command::Key(None)) => {
                                                // Credential status board: what each
                                                // provider needs and whether it is ready.
                                                let rows: Vec<String> = core
                                                    .provider_names()
                                                    .into_iter()
                                                    .map(|p| {
                                                        format!(
                                                            "/{:<16} {}",
                                                            p,
                                                            provider_status(&core, &p)
                                                        )
                                                    })
                                                    .collect();
                                                modal = Some(ModalView {
                                                    title: "provider keys".to_string(),
                                                    rows,
                                                    scroll: 0,
                                                    footer: "/key <provider> SECRET stores · /key <provider> --remove revokes · Esc close"
                                                        .to_string(),
                                                    ..Default::default()
                                                });
                                            }
                                            Some(Command::Key(Some(arg))) => {
                                                let mut parts = arg.splitn(2, char::is_whitespace);
                                                let target = parts.next().unwrap_or("").trim();
                                                match parts.next().map(str::trim).filter(|s| !s.is_empty()) {
                                                    None => {
                                                        if core.provider_configured(target) {
                                                            screen.accent(&format!(
                                                                "{target} is ready — nothing to store",
                                                            ));
                                                        } else {
                                                            screen.error(
                                                                "usage: /key <provider> SECRET",
                                                            );
                                                        }
                                                    }
                                                    Some("--remove" | "--revoke" | "--clear") => {
                                                        match core.remove_provider_key(target) {
                                                            Ok(removed) if removed.shadowed_by_env => {
                                                                screen.error(&format!(
                                                                    "removed the stored key, but {} is still set in your environment — {target} stays authenticated",
                                                                    removed.env_var,
                                                                ));
                                                            }
                                                            Ok(removed) => screen.accent(&format!(
                                                                "{target} key removed ({} cleared from ~/.vakcoder/.env)",
                                                                removed.env_var,
                                                            )),
                                                            Err(e) => screen.error(&e.to_string()),
                                                        }
                                                    }
                                                    Some(secret) => match core
                                                        .set_provider_key(target, secret)
                                                    {
                                                        Ok(env_var) => {
                                                            screen.accent(&format!(
                                                                "{target} key stored as {env_var} (~/.vakcoder/.env, owner-only) · effective immediately · /provider {target} to switch",
                                                            ));
                                                        }
                                                        Err(e) => screen.error(&e.to_string()),
                                                    },
                                                }
                                            }
                                            Some(Command::Config) => {
                                                modal = Some(ModalView {
                                                    title: "settings".to_string(),
                                                    rows: settings_rows(&core, &ui),
                                                    scroll: 0,
                                                    footer: "P provider · M model · T theme · F features · ↑↓ scroll · Esc close".to_string(),
                                                    ..Default::default()
                                                });
                                            }
                                            Some(Command::Features) => {
                                                modal = Some(ModalView {
                                                    title: "feature explorer".to_string(),
                                                    rows: feature_rows(&core),
                                                    scroll: 0,
                                                    footer: "All implemented surfaces · ↑↓ scroll · Esc close".to_string(),
                                                    ..Default::default()
                                                });
                                            }
                                            Some(Command::Composer(arg)) => match arg.as_deref() {
                                                Some("vim") | Some("Vim") => {
                                                    editor.set_mode(ComposerMode::Vim);
                                                    screen.accent(
                                                        "composer → vim · Esc normal · i/a/o insert",
                                                    );
                                                }
                                                Some("emacs") | Some("Emacs") => {
                                                    editor.set_mode(ComposerMode::Emacs);
                                                    screen.accent("composer → emacs");
                                                }
                                                Some(other) => screen.dim(&format!(
                                                    "unknown composer mode '{other}' — emacs, vim"
                                                )),
                                                None => {
                                                    editor.set_mode(match editor.mode() {
                                                        ComposerMode::Emacs => ComposerMode::Vim,
                                                        ComposerMode::Vim => ComposerMode::Emacs,
                                                    });
                                                    screen.accent(&format!(
                                                        "composer → {}",
                                                        match editor.mode() {
                                                            ComposerMode::Vim => "vim",
                                                            ComposerMode::Emacs => "emacs",
                                                        }
                                                    ));
                                                }
                                            },
                                            Some(Command::Goal(arg)) => {
                                                match arg {
                                                    None => {
                                                        let armed = pending_goal
                                                            .lock()
                                                            .unwrap_or_else(
                                                                std::sync::PoisonError::into_inner,
                                                            )
                                                            .is_some();
                                                        if armed {
                                                            screen.dim(
                                                                "🎯 goal armed — next prompt runs audited; /goal off to disarm",
                                                            );
                                                        } else {
                                                            screen.dim(
                                                                "no goal armed · usage: /goal <objective> [-- criterion1; criterion2]",
                                                            );
                                                        }
                                                    }
                                                    Some(a) if a.trim() == "off" => {
                                                        *pending_goal
                                                            .lock()
                                                            .unwrap_or_else(
                                                                std::sync::PoisonError::into_inner,
                                                            ) = None;
                                                        screen.dim("goal disarmed");
                                                    }
                                                    Some(spec) => {
                                                        let (objective, criteria) =
                                                            split_goal_spec(&spec);
                                                        if criteria.is_empty() {
                                                            screen.error(
                                                                "goal needs criteria: /goal <objective> -- c1; c2",
                                                            );
                                                        } else {
                                                            *pending_goal
                                                                .lock()
                                                                .unwrap_or_else(
                                                                    std::sync::PoisonError::into_inner,
                                                                ) = Some((objective, criteria));
                                                            screen.success(&format!(
                                                                "🎯 goal armed ({} criteria) — next prompt will be audited",
                                                                pending_goal
                                                                    .lock()
                                                                    .unwrap_or_else(
                                                                        std::sync::PoisonError::into_inner,
                                                                    )
                                                                    .as_ref()
                                                                    .map(|(_, cs)| cs.len())
                                                                    .unwrap_or(0)
                                                            ));
                                                        }
                                                    }
                                                }
                                            }
                                            Some(Command::Subagents) => open_subagent_picker(
                                                &core, &mut screen, &mut picker,
                                            ),
                                            Some(Command::A11y(arg)) => {
                                                match commands::parse_a11y(arg.as_deref()) {
                                                    Ok((feature, state)) => {
                                                        let current = match feature {
                                                            commands::A11yFeature::Plain => a11y_plain,
                                                            commands::A11yFeature::Motion => a11y_motion,
                                                            commands::A11yFeature::Reader => a11y_reader,
                                                        };
                                                        let target = state.unwrap_or(!current);
                                                        match feature {
                                                            commands::A11yFeature::Plain => a11y_plain = target,
                                                            commands::A11yFeature::Motion => a11y_motion = target,
                                                            commands::A11yFeature::Reader => a11y_reader = target,
                                                        }
                                                        screen.set_accessibility(a11y_plain, a11y_reader);
                                                        screen.accent(&format!(
                                                            "{} {} — /a11y plain|motion|reader on|off",
                                                            feature.name(),
                                                            if target { "on" } else { "off" },
                                                        ));
                                                    }
                                                    Err(msg) => screen.dim(&msg),
                                                }
                                            }
                                            Some(Command::Copy) => {
                                                copy_last_response(&core, &mut screen, &ui)
                                            }
                                            Some(Command::Clear) => match core.start_session().await {
                                                Ok(s) => {
                                                    *session_slot.lock().await = Some(s);
                                                    screen.accent("started a fresh session");
                                                }
                                                Err(e) => screen.dim(&format!("error: {e}")),
                                            },
                                            None => {
                                                let customs = core.custom_commands();
                                                let mut parts = text
                                                    .trim_start_matches('/')
                                                    .splitn(2, char::is_whitespace);
                                                let name = parts.next().unwrap_or("").trim();
                                                let args =
                                                    parts.next().unwrap_or("").trim();
                                                match customs.iter().find(|c| c.name == name) {
                                                    Some(custom) => {
                                                        let prompt =
                                                            vak_core::custom_commands::expand(
                                                                &custom.template,
                                                                args,
                                                            );
                                                        let expanded =
                                                            mentions::expand(&prompt, core.cwd());
                                                        for miss in &expanded.missing {
                                                            screen.error(&format!(
                                                                "no such file: {miss}"
                                                            ));
                                                        }
                                                        screen.user_message(&text, false);
                                                        match ctx.spawn(&expanded.prompt).await {
                                                            Some((ev_rx, done_rx)) => {
                                                                running = Some(ev_rx);
                                                                run_done = Some(done_rx);
                                                                run_started = Some(Instant::now());
                                                                ui.styler = LineStyler::new();
                                                                ui.partial.clear();
                                                                last_agent_event = Instant::now();
                                                            }
                                                            None => screen.dim(
                                                                "provider unavailable — check auth/config",
                                                            ),
                                                        }
                                                        continue;
                                                    }
                                                    None => {
                                                        screen.dim("unknown command — /help")
                                                    }
                                                }
                                            }
                                        }
                                        if let Some(active) = modal.as_ref() {
                                            screen.redraw_modal(
                                                &active.title,
                                                &active.rows,
                                                active.scroll,
                                                &active.footer,
                                                active.selected,
                                            );
                                        } else if picker.is_none() {
                                            let (buf, cur) = editor.view();
                                            screen.redraw_composer(
                                                &composer_label(&ui, inbox_unread),
                                                buf,
                                                cur,
                                                &composer_footer(&editor),
                                            );
                                        }
                                        continue;
                                    }
                                    if let Some(cmd) = text.strip_prefix('!') {
                                        run_shell_passthrough(
                                            &core,
                                            &session_slot,
                                            cmd.trim(),
                                            &mut screen,
                                            &ui_theme,
                                        )
                                        .await;
                                        let (buf, cur) = editor.view();
                                        screen.redraw_composer(
                                            &composer_label(&ui, inbox_unread),
                                            buf,
                                            cur,
                                            &composer_footer(&editor),
                                        );
                                        continue;
                                    }
                                    let expanded = mentions::expand(&text, core.cwd());
                                    if !expanded.attached.is_empty() {
                                        screen.dim(&format!(
                                            "attached {} file(s): {}",
                                            expanded.attached.len(),
                                            expanded.attached.join(", ")
                                        ));
                                    }
                                    for miss in &expanded.missing {
                                        screen.error(&format!("no such file: {miss}"));
                                    }
                                    match ctx.spawn(&expanded.prompt).await {
                                        Some((ev_rx, done_rx)) => {
                                            running = Some(ev_rx);
                                            run_done = Some(done_rx);
                                            run_started = Some(Instant::now());
                                            ui.styler = LineStyler::new();
                                            ui.partial.clear();
                                            last_agent_event = Instant::now();
                                        }
                                        None => screen.dim("provider unavailable — check auth/config"),
                                    }
                                    screen.user_message(&text, false);
                                }
                            }
                            Action::Queue => {
                                if is_running {
                                    let text = std::mem::take(&mut pending);
                                    if !text.trim().is_empty() {
                                        if let Some((id, label)) = attached.clone() {
                                            if core.subagents().queue_follow_up(&id, &text) {
                                                screen.clear_input();
                                                screen.dim(&format!(
                                                    "[queued into {label} · runs after its current turn]"
                                                ));
                                            } else {
                                                follow_ups.push_back(text);
                                                screen.clear_input();
                                                screen.dim(&format!(
                                                    "[subagent ended · queued for main run · {} waiting]",
                                                    follow_ups.len()
                                                ));
                                            }
                                        } else {
                                            follow_ups.push_back(text);
                                            screen.clear_input();
                                            screen.dim(&format!(
                                                "[queued for next turn · {} waiting]",
                                                follow_ups.len()
                                            ));
                                        }
                                    }
                                }
                            }
                            Action::Interrupt => {
                                if is_running {
                                    if let Some((id, label)) = attached.clone()
                                        && core.subagents().stop(&id)
                                    {
                                        screen.clear_input();
                                        screen.dim(&format!("[stopping subagent {label}]"));
                                    } else {
                                        cancel.lock().await.cancel();
                                        screen.clear_input();
                                        screen.dim("[cancelling…]");
                                    }
                                }
                            }
                            Action::CancelOrClear => {
                                if let Some((_, label)) = attached.take() {
                                    ui.attached_label = None;
                                    screen.clear_input();
                                    screen.dim(&format!(
                                        "[detached from {label}]"
                                    ));
                                } else if is_running {
                                    if let Some(_popped) = follow_ups.pop_back() {
                                        screen.clear_input();
                                        screen.dim(&format!(
                                            "[queue item removed · {} left]",
                                            follow_ups.len()
                                        ));
                                    } else {
                                        cancel.lock().await.cancel();
                                        screen.clear_input();
                                        screen.dim("[cancelling…]");
                                    }
                                } else if !editor.is_empty() {
                                    editor.clear();
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
                    Some(Ok(Event::Resize(_, _))) => screen.resize(),
                    _ => {}
                }
            }
            Some(agent_event) = agent_ev => {
                last_agent_event = Instant::now();
                render_event(&mut screen, &mut ui, &ui_theme, agent_event);
            }
            Some(req) = approval_ev => {
                if bell_on {
                    screen.bell();
                    screen.notify(&format!("VakCoder · approval requested: {}", req.tool));
                }
                approvals.push_back(req);
            }
            Some(outcome) = done_ev => {
                let aborted = matches!(outcome, TurnOutcome::Aborted { .. });
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
                // A deferred /mode applies now that the run has actually
                // stopped — before any queued follow-up fires under it.
                if let Some(mode) = deferred_mode.take() {
                    core.set_permission_mode(mode);
                    screen.success(&format!("mode → {} (in-flight run stopped)", mode_label(mode)));
                }
                // Follow-up queue drains only after a non-aborted run: a
                // cancelled run must not silently fire what the user queued.
                if !aborted
                    && let Some(next) = follow_ups.pop_front()
                {
                    screen.user_message(&next, true);
                    match ctx.spawn(&next).await {
                        Some((ev_rx, done_rx)) => {
                            running = Some(ev_rx);
                            run_done = Some(done_rx);
                            run_started = Some(Instant::now());
                            ui.styler = LineStyler::new();
                            ui.partial.clear();
                            last_agent_event = Instant::now();
                        }
                        None => screen.dim("provider unavailable — queued item dropped"),
                    }
                }
            }
            _ = tokio::time::sleep(tick_delay) => {
                tick_count += 1;
                if Instant::now() >= inbox_next_poll {
                    inbox_unread = vak_core::inbox::unread_count(&core.sessions_home());
                    inbox_next_poll = Instant::now() + Duration::from_secs(INBOX_REFRESH_SECS);
                }
            }
        }

        if approval_focused && let Some(front) = approvals.front() {
            draw_approval(&mut screen, &ui_theme, front);
        } else if let Some(req) = confirm_forget.as_ref() {
            draw_confirm(&mut screen, &ui_theme, req);
        } else if let Some(req) = confirm_ack.as_ref() {
            draw_ack_confirm(&mut screen, &ui_theme, req);
        } else if running.is_some() {
            let elapsed = run_started.map(|t| t.elapsed().as_secs()).unwrap_or(0);
            let budget = core
                .config()
                .finops
                .max_day_usd
                .and_then(|cap| budget_marker(core.spend_day_usd(), cap));
            let live_status = status_ansi(
                &ui_theme,
                &ui.run_state,
                ui.total_in,
                ui.total_out,
                elapsed,
                tick_count,
                context_window,
                last_agent_event.elapsed().as_secs(),
                !a11y_motion,
                budget,
            );
            screen.redraw_running(&live_status, &pending, follow_ups.len(), approvals.len());
        } else if let Some(active) = modal.as_ref() {
            screen.redraw_modal(
                &active.title,
                &active.rows,
                active.scroll,
                &active.footer,
                active.selected,
            );
        } else if let Some((kind, active)) = picker.as_ref() {
            draw_picker(&mut screen, *kind, active);
        } else if let Some(active) = palette.as_ref() {
            let items = active.items();
            screen.redraw_palette(active.query(), &items, active.selected(items.len()));
        } else if editor.search_active() {
            let q = editor.search_query().to_string();
            let (buf, cur) = editor.view();
            screen.redraw_composer(
                &format!("history search · {q}"),
                buf,
                cur,
                "Ctrl-R older · Enter accept · Esc restore",
            );
        } else {
            let (buf, cur) = editor.view();
            if let Some((msg, at)) = refl_tick.as_ref() {
                if at.elapsed() < std::time::Duration::from_secs(6) {
                    let footer = format!("{msg} · {}", composer_footer(&editor));
                    screen.redraw_composer(&composer_label(&ui, inbox_unread), buf, cur, &footer);
                    continue;
                }
                refl_tick = None;
            }
            screen.redraw_composer(
                &composer_label(&ui, inbox_unread),
                buf,
                cur,
                &composer_footer(&editor),
            );
        }
    }

    editor.save_history(&hist_path);
    screen.dim("bye");
    0
}

/// Hands the current draft to `$VISUAL`/`$EDITOR` (fallback `vi`).
///
/// The draft is staged on disk before launch, so a crash mid-edit still
/// recovers the text on the next start. The tty event reader is dropped and
/// raw mode suspended for the duration so the child owns the terminal.
fn open_external_editor(
    editor: &mut Editor,
    raw: &mut Option<RawMode>,
    reader: &mut Option<EventStream>,
    screen: &mut Screen,
    draft_path: &std::path::Path,
) {
    if !editor.save_draft(draft_path) {
        screen.error("could not stage draft — external editor skipped");
        return;
    }
    let cmd = ["VISUAL", "EDITOR"]
        .iter()
        .find_map(|k| std::env::var(k).ok().filter(|v| !v.is_empty()))
        .unwrap_or_else(|| "vi".to_string());
    reader.take();
    drop(raw.take());
    let result = std::process::Command::new("sh")
        .arg("-c")
        .arg(format!("{cmd} \"$0\""))
        .arg(draft_path)
        .status();
    *raw = Some(RawMode::enable());
    *reader = Some(EventStream::new());
    screen.resize();
    match result {
        Ok(status) if status.success() => match std::fs::read_to_string(draft_path) {
            Ok(text) => {
                let _ = std::fs::remove_file(draft_path);
                editor.set_text(&text);
                screen.dim("draft loaded from external editor");
            }
            Err(e) => screen.error(&format!("could not read edited draft: {e}")),
        },
        Ok(_) => screen.dim("editor exited with an error — draft kept for recovery"),
        Err(e) => screen.error(&format!("could not launch {cmd}: {e}")),
    }
}

fn render_event(screen: &mut Screen, ui: &mut UiState, theme: &Theme, ev: AgentEvent) {
    match ev {
        AgentEvent::Stream(StreamEvent::TextDelta { delta, .. }) => {
            flush_thinking(screen, ui, theme);
            ui.thinking_shown = false;
            ui.run_state = RunState::Streaming;
            ui.last_response.push_str(&delta);
            feed_text(ui, screen, theme, &delta)
        }
        AgentEvent::Stream(StreamEvent::ThinkingDelta { delta, .. }) => match ui.thinking_mode {
            ThinkingMode::Off => {}
            ThinkingMode::Indicator if !ui.thinking_shown => {
                screen.clear_input();
                screen.dim("  · thinking…");
                ui.thinking_shown = true;
            }
            ThinkingMode::Indicator => {}
            ThinkingMode::Full => feed_thinking(ui, screen, theme, &delta),
        },
        AgentEvent::Stream(_) => {}
        AgentEvent::ToolCallStart {
            id,
            name,
            args_json,
        } => {
            flush_md(screen, ui, theme);
            ui.run_state = RunState::Tool { name: name.clone() };
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
            ui.run_state = RunState::Streaming;
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
                && let Some(prev) = result_preview.as_deref()
                && !prev.trim().is_empty()
            {
                render_tool_preview(screen, prev, theme.error, ui.expanded_tools);
            } else if ui.expanded_tools
                && let Some(prev) = result_preview.as_deref()
                && !prev.trim().is_empty()
            {
                render_tool_preview(screen, prev, theme.dim, true);
            }
        }
        AgentEvent::TurnEnd { usage } => {
            flush_thinking(screen, ui, theme);
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
            ui.last_response.clear();
            ui.thinking_shown = false;
            ui.thinking_partial.clear();
            ui.run_state = RunState::Thinking;
        }
        AgentEvent::RetryScheduled {
            attempt,
            delay_ms,
            reason,
        } => {
            ui.run_state = RunState::Retrying {
                attempt,
                delay_ms,
                reason: reason.clone(),
            };
            screen.clear_input();
            screen.dim(&format!("⟳ retry {attempt} in {delay_ms}ms — {reason}"));
        }
        AgentEvent::RouteFallback {
            to_provider,
            to_model,
        } => {
            ui.run_state = RunState::Streaming;
            screen.clear_input();
            screen.dim(&format!(
                "⤵ route fallback → {to_provider}/{to_model} (frozen ladder leg)"
            ));
        }
        AgentEvent::ContextCompacting { estimated_tokens } => {
            ui.run_state = RunState::Compacting;
            screen.clear_input();
            screen.dim(&format!(
                "📦 compacting context (~{estimated_tokens} tokens)…"
            ));
        }
        AgentEvent::ContextCompacted {
            before_tokens,
            after_tokens,
            summarized_messages,
            selected_messages,
            dropped_messages,
        } => {
            ui.run_state = RunState::Streaming;
            screen.clear_input();
            screen.dim(&format!(
                "📦 context compacted: ~{before_tokens} → ~{after_tokens} tokens ({summarized_messages} summarized · {selected_messages} kept verbatim · {dropped_messages} dropped)"
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
        AgentEvent::HandoffReset { before_tokens } => {
            screen.dim(&format!(
                "✋ context reset — structured handoff written (~{before_tokens} → fresh segment)"
            ));
        }
        AgentEvent::StreamOpened
        | AgentEvent::ApprovalRequested { .. }
        | AgentEvent::RunFinished { .. } => {}
    }
}

fn feed_thinking(ui: &mut UiState, screen: &mut Screen, theme: &Theme, delta: &str) {
    ui.thinking_partial.push_str(delta);
    while let Some(pos) = ui.thinking_partial.find('\n') {
        let raw: String = ui.thinking_partial.drain(..=pos).collect();
        screen.clear_input();
        screen.styled(&format!("  │ {}", raw.trim_end_matches('\n')), theme.dim);
    }
    ui.thinking_shown = true;
}

fn flush_thinking(screen: &mut Screen, ui: &mut UiState, theme: &Theme) {
    if ui.thinking_partial.is_empty() {
        return;
    }
    let rest = std::mem::take(&mut ui.thinking_partial);
    screen.clear_input();
    screen.styled(&format!("  │ {rest}"), theme.dim);
}

fn render_tool_preview(screen: &mut Screen, preview: &str, color: Color, expanded: bool) {
    let cap = if expanded { 30 } else { 6 };
    let lines: Vec<&str> = preview
        .lines()
        .filter(|line| !line.trim().is_empty())
        .collect();
    let skipped = lines.len().saturating_sub(cap);
    if skipped > 0 {
        screen.styled(&format!("  │ … {skipped} earlier lines"), color);
    }
    for line in lines.iter().skip(skipped) {
        screen.styled(&format!("  │ {line}"), color);
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
    ui.last_response.push_str(&rest);
    let styled = ui.styler.line(&rest, theme);
    screen.clear_input();
    screen.md_line(&styled);
}

fn draw_approval(screen: &mut Screen, theme: &Theme, req: &ApprovalRequest) {
    let styled = |text: String, color: Color| {
        format!("{}{text}{}", theme::fg(color), theme::fg(Color::Reset))
    };
    let tool_label = if req.tool == "finops-budget" {
        "budget raise request".to_string()
    } else {
        req.tool.clone()
    };
    let mut lines = vec![styled(format!("╭─ approval · {tool_label}"), theme.warning)];
    if req.tool == "webfetch"
        && let Ok(v) = serde_json::from_str::<serde_json::Value>(&req.args_json)
        && let Some(url) = v.get("url").and_then(|u| u.as_str())
    {
        // The destination is the decision: show it prominently, before the
        // generic args dump.
        lines.push(styled(
            format!("│ url · {}", trunc_cells(url, 120)),
            theme.accent,
        ));
    }
    if req.tool == "edit"
        && let Some(diff) = approval_edit_diff(&req.args_json, theme)
    {
        for line in pretty_args_lines(&req.args_json, 1) {
            lines.push(styled(format!("│ {}", line.trim_start()), theme.dim));
        }
        for line in diff.lines() {
            lines.push(format!("│ {line}"));
        }
    } else {
        for line in pretty_args_lines(&req.args_json, 6) {
            lines.push(styled(format!("│ {}", line.trim_start()), theme.dim));
        }
    }
    if !req.reason.trim().is_empty() {
        lines.push(styled(
            format!("│ rule · {}", trunc_cells(&req.reason, 90)),
            theme.dim,
        ));
    }
    lines.push(styled(
        "╰─ Y once · A session · P save rule · N/Esc deny".to_string(),
        theme.dim,
    ));
    screen.redraw_block(&lines);
}

/// Inline confirm card for a destructive memory op — same shape as an
/// approval so the interaction reads identically.
fn draw_confirm(screen: &mut Screen, theme: &Theme, req: &ForgetConfirm) {
    let styled = |text: String, color: Color| {
        format!("{}{text}{}", theme::fg(color), theme::fg(Color::Reset))
    };
    let lines = vec![
        styled(
            format!("╭─ confirm forget · {}", short_hex(&req.id)),
            theme.warning,
        ),
        styled(format!("│ {}", req.preview), theme.dim),
        styled("╰─ y forget · n/Esc keep".to_string(), theme.dim),
    ];
    screen.redraw_block(&lines);
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
        "webfetch" => pick("url"),
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

/// Compact day-cap marker for the live status row (personal-os budget
/// awareness): ≥80% of cap shows `⚠ $spend/$cap`, ≥100% swaps ⚠ for ✗.
/// Thresholds come from `vak_core::finops::alert_level`; no cap → None.
pub fn budget_marker(day_usd: f64, cap: f64) -> Option<String> {
    let level = vak_core::finops::alert_level(day_usd, cap)?;
    let mark = match level {
        vak_core::finops::AlertLevel::Full => '✗',
        vak_core::finops::AlertLevel::Eighty => '⚠',
    };
    Some(format!("{mark} ${day_usd:.2}/${cap:.2}"))
}

#[allow(clippy::too_many_arguments)]
fn status_ansi(
    theme: &Theme,
    state: &RunState,
    total_in: u64,
    total_out: u64,
    elapsed_secs: u64,
    tick: usize,
    window: u64,
    idle_secs: u64,
    animated: bool,
    budget: Option<String>,
) -> String {
    let total = total_in + total_out;
    let pct = if window > 0 {
        total.saturating_mul(100) / window.max(1)
    } else {
        0
    };
    let (spinner, spinner_color) = match state {
        RunState::Retrying { .. } | RunState::Compacting => (
            if animated {
                format!("⏳ {}", state.label())
            } else {
                state.label()
            },
            theme.warning,
        ),
        RunState::Tool { .. } => (format!("⚙ {}", state.label()), theme.accent),
        RunState::Thinking => (
            format!("{} thinking", status::frame_motion(tick, animated)),
            theme.spinner,
        ),
        RunState::Streaming => (
            status::frame_motion(tick, animated).to_string(),
            theme.spinner,
        ),
    };
    let mut line = format!(
        "{}{} · ↑{} ↓{} · ctx {}%",
        theme::fg(theme.dim),
        status::fmt_elapsed(elapsed_secs),
        status::fmt_tokens(total_in),
        status::fmt_tokens(total_out),
        pct.min(999),
    );
    if idle_secs >= 10 {
        line.push_str(&format!(
            "{} · no events {}{}",
            theme::fg(theme.warning),
            status::fmt_elapsed(idle_secs),
            theme::fg(Color::Reset),
        ));
    }
    if let Some(marker) = budget {
        let color = if marker.starts_with('✗') {
            theme.error
        } else {
            theme.warning
        };
        line.push_str(&format!(
            "{} · {marker}{}",
            theme::fg(color),
            theme::fg(Color::Reset),
        ));
    }
    format!(
        "{}{}{} {}",
        theme::fg(spinner_color),
        spinner,
        theme::fg(Color::Reset),
        line
    )
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

/// Compact read-only viewer for the active session's message tree.
async fn transcript_modal(
    slot: &Arc<Mutex<Option<SessionLog>>>,
    arg: Option<&str>,
) -> Option<ModalView> {
    let limit = arg.and_then(|a| a.parse::<usize>().ok()).unwrap_or(40);
    let guard = slot.lock().await;
    let s = guard.as_ref()?;
    let msgs = s.derive_messages();
    let start = msgs.len().saturating_sub(limit);
    let (mut rows, anchors) = build_transcript_rows(&msgs, start);
    let scroll = rows.len().saturating_sub(1);
    rows.insert(
        0,
        format!(
            "transcript · showing {} of {} messages · n/p prompt jumps · e export",
            msgs.len() - start,
            msgs.len()
        ),
    );
    Some(ModalView {
        title: "transcript".to_string(),
        rows,
        scroll,
        footer: "n/p prompt jumps · / search · e export · End latest · Esc close".to_string(),
        anchors,
        ..Default::default()
    })
}

fn find_matches(rows: &[String], query: &str) -> Vec<usize> {
    let trimmed = query.trim();
    if trimmed.is_empty() {
        return Vec::new();
    }
    let needle = trimmed.to_lowercase();
    rows.iter()
        .enumerate()
        .filter(|(_, row)| row.to_lowercase().contains(&needle))
        .map(|(i, _)| i)
        .collect()
}

fn build_transcript_rows(
    msgs: &[vak_llm::types::Message],
    start: usize,
) -> (Vec<String>, Vec<usize>) {
    let mut rows = Vec::new();
    let mut anchors = Vec::new();
    for (idx, m) in msgs[start..].iter().enumerate() {
        let absolute = start + idx;
        match m.role {
            vak_llm::Role::User => {
                anchors.push(rows.len());
                rows.push(format!("{absolute:>4} ▸ user"));
            }
            _ => rows.push(format!("{absolute:>4} ◆ assistant")),
        }
        for block in &m.content {
            match block {
                vak_llm::ContentBlock::Text { text } => {
                    for line in text.trim().lines().take(6) {
                        rows.push(format!("     {}", trunc_cells(line.trim_end(), 160)));
                    }
                }
                vak_llm::ContentBlock::Thinking { .. } => {}
                vak_llm::ContentBlock::ToolUse { name, input, .. } => {
                    rows.push(format!(
                        "     · tool {name} {}",
                        trunc_cells(&input.to_string(), 90)
                    ));
                }
                vak_llm::ContentBlock::ToolResult {
                    content, is_error, ..
                } => {
                    let mark = if *is_error { "✗" } else { "→" };
                    rows.push(format!(
                        "     {mark} {}",
                        trunc_cells(content.replace('\n', " ").trim(), 130)
                    ));
                }
                vak_llm::ContentBlock::Image { source } => {
                    rows.push(format!("     ▣ image ({})", source.media_type));
                }
            }
        }
        rows.push(String::new());
    }
    (rows, anchors)
}

/// Writes the full session transcript as Markdown next to the workspace.
/// Byte-parity with the server-side export is guaranteed by
/// `vak_core::transcript_md`'s shared-renderer test.
/// Returns the path written.
async fn export_transcript(
    slot: &Arc<Mutex<Option<SessionLog>>>,
    cwd: &std::path::Path,
) -> Result<std::path::PathBuf, String> {
    let guard = slot.lock().await;
    let Some(s) = guard.as_ref() else {
        return Err("no active session".to_string());
    };
    let out = vak_core::transcript_md::render_markdown(&s.derive_messages());
    let stem = s
        .path()
        .file_stem()
        .map(|s| s.to_string_lossy().to_string())
        .unwrap_or_else(|| "session".to_string());
    let path = cwd.join(format!("vakcoder-transcript-{stem}.md"));
    std::fs::write(&path, out).map_err(|e| e.to_string())?;
    Ok(path)
}

/// `!cmd` runs locally through the production BashTool; output is appended
/// to the active session as a user context entry so the model sees it on
/// the next turn (model-visible ⇒ logged).
async fn run_shell_passthrough(
    core: &Core,
    slot: &Arc<Mutex<Option<SessionLog>>>,
    cmd: &str,
    screen: &mut Screen,
    theme: &Theme,
) {
    if cmd.is_empty() {
        screen.dim("usage: !<shell command>");
        return;
    }
    // Brokered + sandboxed like every other bash execution (invariant 14):
    // the command crosses the __tool_worker protocol and lands under the
    // active permission mode's OS/docker sandbox instead of running raw
    // and unsandboxed in-process.
    let Some(tool) = core.agent_tools().into_iter().find(|t| t.name() == "bash") else {
        screen.error("bash tool unavailable");
        return;
    };
    let ctx = vak_tools::ToolContext {
        cwd: core.cwd().clone(),
        cancel: CancellationToken::new(),
        limits: Default::default(),
        sandbox: core.agent_sandbox(),
    };
    let out = tool
        .execute(&serde_json::json!({"command": cmd}), &ctx)
        .await;
    screen.clear_input();
    let tail: Vec<&str> = out
        .content
        .lines()
        .filter(|l| !l.trim().is_empty())
        .collect();
    let shown = tail.len().saturating_sub(12);
    let mark = if out.is_error { "✗" } else { "✓" };
    let color = if out.is_error {
        theme.error
    } else {
        theme.success
    };
    screen.styled(&format!("{mark} !{cmd}"), color);
    for line in &tail[shown..] {
        screen.styled(&format!("  │ {line}"), theme.dim);
    }
    let mut guard = slot.lock().await;
    if let Some(s) = guard.as_mut() {
        let _ = s.append_message(vak_session::MessageRecord {
            message: vak_llm::Message::user_text(format!(
                "[! shell]\n$ {cmd}\n\noutput:\n{}",
                out.content
            )),
            meta: None,
        });
    }
}

/// Health check rendered as a checklist: auth, sandbox, storage, config
/// warnings, extension surface.
/// `/services` — background service status and control via vak-ops
/// (docs/design/28-operations.md).
fn run_services(arg: Option<(String, String)>, screen: &mut Screen) {
    let cfg = vak_ops::OpsConfig::detect();

    if let Some((action, svc_name)) = arg {
        let svc = match svc_name.as_str() {
            "gateway" => vak_ops::Service::Gateway,
            _ => vak_ops::Service::Telegram,
        };
        match action.as_str() {
            "start" | "stop" | "restart" => {
                let ok = match action.as_str() {
                    "start" => vak_ops::start(svc, &cfg),
                    "stop" => vak_ops::stop(svc, &cfg),
                    _ => {
                        vak_ops::restart(svc, &cfg);
                        true
                    }
                };
                if ok {
                    screen.accent(&format!("services: {action} {svc_name} — done"));
                } else {
                    screen.error(&format!(
                        "services: {action} {svc_name} failed (is it installed? see /services)"
                    ));
                }
            }
            _ => {
                screen.error("usage: /services [start|stop|restart] [gateway|telegram]");
                return;
            }
        }
    }

    screen.accent("services:");
    for (name, svc) in [
        ("gateway", vak_ops::Service::Gateway),
        ("telegram", vak_ops::Service::Telegram),
    ] {
        let st = vak_ops::status(svc, &cfg);
        let healthy = name != "gateway" || vak_ops::health_ok(&cfg);
        let extra = if name == "gateway" && st == vak_ops::State::Running && !healthy {
            " · not answering"
        } else {
            ""
        };
        screen.line(&format!("  {name:<8} {st}{extra}"));
    }
    screen.dim("  /services start|stop|restart gateway|telegram");
}

/// Health check rendered as a checklist: auth, storage, config warnings,
/// extensions, breaker, finops, frozen route ladder. Facts come from the
/// shared `vak_core::health` collector so CLI/desktop/TUI agree.
fn run_doctor(core: &Core, session: Option<&SessionLog>, screen: &mut Screen) {
    screen.clear_input();
    screen.accent("doctor:");
    let health = vak_core::health::collect(core, session);
    let mut failures = 0usize;
    for check in &health.checks {
        report(screen, &check.label, &check.detail, &mut failures);
    }
    for fact in &health.facts {
        screen.dim(&format!("  · {fact}"));
    }
    if let Some(ladder) = &health.ladder {
        screen.dim(&format!(
            "  · route ladder (frozen at admission): {}",
            ladder.rendered
        ));
        if !ladder.objective.is_empty() {
            screen.dim(&format!(
                "  · route objective: {} · fallback legs: {}",
                ladder.objective, ladder.fallback_legs,
            ));
        }
        for note in &ladder.annotations {
            screen.warn(&format!("  · route: {note}"));
        }
    }
    if health.failures == 0 {
        screen.success("all checks passed");
    } else {
        screen.error(&format!("{} check(s) failed", health.failures));
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

// ---- personal-os surfaces (docs/design/29-personal-os.md) ------------------

/// Idle status-row unread marker (P6): `· ✉ N` appended to the composer
/// label — the quiet twin of the live row's budget marker. Empty while
/// nothing awaits attention so the row stays clean at zero.
pub fn inbox_badge(unread: usize) -> String {
    if unread == 0 {
        String::new()
    } else {
        format!(" · ✉ {unread}")
    }
}

const INBOX_TITLE_CELLS: usize = 48;

/// Titles collapse whitespace and clip to a cell budget (CLI parity).
fn inbox_title(title: &str, max: usize) -> String {
    let flat = title.split_whitespace().collect::<Vec<_>>().join(" ");
    trunc_cells(&flat, max)
}

/// Short display chip per notification kind; labels match `vakcoder inbox`.
fn kind_chip(kind: Kind) -> &'static str {
    match kind {
        Kind::TaskSummary => "task",
        Kind::ApprovalPending => "approval",
        Kind::ApprovalDenied => "denied",
        Kind::BudgetAlert => "budget",
        Kind::Digest => "digest",
        Kind::Heartbeat => "beat",
        Kind::ProposalOpened => "proposal",
    }
}

/// Theme slot per kind — approvals and budget alerts warn, denials error,
/// digests succeed, the rest take their house accent/dim/code slots.
fn kind_color(kind: Kind, theme: &Theme) -> Color {
    match kind {
        Kind::TaskSummary => theme.accent,
        Kind::ApprovalPending => theme.warning,
        Kind::ApprovalDenied => theme.error,
        Kind::BudgetAlert => theme.warning,
        Kind::Digest => theme.success,
        Kind::Heartbeat => theme.dim,
        Kind::ProposalOpened => theme.code,
    }
}

/// One `/inbox` table row: rel-ts · colored kind chip · clipped title · id8.
/// The chip embeds its SGR like the budget marker does; the base heading
/// color is restored after so the rest of the row renders uniformly.
fn inbox_row(entry: &Entry, theme: &Theme) -> String {
    format!(
        "{} · {}{}{} · {} · {}",
        rel_time(entry.ts),
        theme::fg(kind_color(entry.kind, theme)),
        kind_chip(entry.kind),
        theme::fg(theme.heading),
        inbox_title(&entry.title, INBOX_TITLE_CELLS),
        short_hex(&entry.id),
    )
}

fn inbox_rows(entries: &[Entry], theme: &Theme) -> Vec<String> {
    entries.iter().map(|e| inbox_row(e, theme)).collect()
}

/// Parsed `/inbox [all|ack <id>]` payload. Err carries the usage line.
enum InboxAction {
    List { all: bool },
    Ack(String),
}

impl InboxAction {
    /// Bare/blank lists unread; `all` includes acked entries;
    /// `ack <id8-prefix>` arms the inline confirm.
    fn parse(arg: Option<&str>) -> Result<Self, String> {
        const USAGE: &str = "usage: /inbox [all|ack <id8-prefix>]";
        let Some(arg) = arg.map(str::trim).filter(|a| !a.is_empty()) else {
            return Ok(Self::List { all: false });
        };
        let mut tokens = arg.splitn(2, char::is_whitespace);
        match tokens.next().unwrap_or("") {
            "all" => Ok(Self::List { all: true }),
            "ack" | "done" | "read" => {
                let id = tokens.next().map(str::trim).unwrap_or("");
                if id.is_empty() {
                    Err(USAGE.to_string())
                } else {
                    Ok(Self::Ack(id.to_string()))
                }
            }
            _ => Err(USAGE.to_string()),
        }
    }
}

/// Why a `/inbox ack <prefix>` id could not be resolved.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum InboxMatchError {
    NotFound(String),
    Ambiguous(String),
}

impl std::fmt::Display for InboxMatchError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::NotFound(given) => write!(f, "no inbox entry matches '{given}'"),
            Self::Ambiguous(given) => {
                write!(
                    f,
                    "'{given}' matches several inbox entries — use more of the id"
                )
            }
        }
    }
}

/// Resolve a user-supplied entry id (full 16-hex or unique prefix) against
/// UNREAD entries first, then the whole ledger; an exact full-id hit wins
/// over any prefix set. Uniqueness is judged per tier, so one unread entry
/// shadows acked entries sharing its prefix.
pub fn resolve_entry<'a>(
    unread: &'a [Entry],
    all: &'a [Entry],
    given: &str,
) -> Result<&'a Entry, InboxMatchError> {
    enum Tier {
        NotFound,
        Ambiguous,
    }
    fn tier<'a>(entries: &'a [Entry], given: &str) -> Result<&'a Entry, Tier> {
        if let Some(e) = entries.iter().find(|e| e.id == given) {
            return Ok(e);
        }
        let hits: Vec<&Entry> = entries.iter().filter(|e| e.id.starts_with(given)).collect();
        match hits.as_slice() {
            [one] => Ok(one),
            [] => Err(Tier::NotFound),
            _ => Err(Tier::Ambiguous),
        }
    }
    match tier(unread, given) {
        Ok(e) => Ok(e),
        Err(Tier::Ambiguous) => Err(InboxMatchError::Ambiguous(given.to_string())),
        Err(Tier::NotFound) => tier(all, given).map_err(|t| match t {
            Tier::NotFound => InboxMatchError::NotFound(given.to_string()),
            Tier::Ambiguous => InboxMatchError::Ambiguous(given.to_string()),
        }),
    }
}

/// An `/inbox ack <prefix>` armed by the command, awaiting y/n.
struct InboxAck {
    id: String,
    preview: String,
}

/// Inline confirm card for marking an inbox entry read — same shape as the
/// forget card so destructive-op interactions read identically.
fn draw_ack_confirm(screen: &mut Screen, theme: &Theme, req: &InboxAck) {
    let styled = |text: String, color: Color| {
        format!("{}{text}{}", theme::fg(color), theme::fg(Color::Reset))
    };
    let lines = vec![
        styled(
            format!("╭─ confirm ack · {}", short_hex(&req.id)),
            theme.warning,
        ),
        styled(format!("│ {}", req.preview), theme.dim),
        styled("╰─ y mark read · n/Esc keep unread".to_string(), theme.dim),
    ];
    screen.redraw_block(&lines);
}

/// Finds the project directory holding `<session_id>.jsonl`: the active
/// workspace's dir first, then any other project under `<home>/sessions`.
fn locate_session_dir(
    home: &std::path::Path,
    cwd: &std::path::Path,
    session_id: &str,
) -> Option<PathBuf> {
    let name = format!("{session_id}.jsonl");
    let primary = vak_session::SessionPath::sessions_dir(home, cwd);
    if primary.join(&name).exists() {
        return Some(primary);
    }
    for dir in std::fs::read_dir(home.join("sessions")).ok()?.flatten() {
        if dir.file_type().is_ok_and(|t| t.is_dir()) && dir.path().join(&name).exists() {
            return Some(dir.path());
        }
    }
    None
}

/// One `/search` result; kept beside the open modal so Enter can resolve the
/// highlighted hit back to its ledger without re-searching.
#[derive(Clone)]
struct SearchHit {
    session_id: String,
    project_hash: String,
    ts: chrono::DateTime<chrono::Utc>,
    score: f32,
    snippet: String,
}

/// A destructive memory op armed by `/memory forget`, awaiting y/n.
struct ForgetConfirm {
    path: PathBuf,
    id: String,
    preview: String,
}

/// Parsed `/memory [--profile] <form>` payload.
enum MemoryAction {
    List,
    Append(String),
    Forget(String),
    Amend(String, String),
}

impl MemoryAction {
    /// Empty arg lists; otherwise the first token selects forget/amend and
    /// anything else is new-note text.
    fn parse(arg: Option<&str>) -> Self {
        let Some(arg) = arg.map(str::trim).filter(|a| !a.is_empty()) else {
            return Self::List;
        };
        let mut tokens = arg.splitn(2, char::is_whitespace);
        match tokens.next().unwrap_or("") {
            "forget" | "rm" => Self::Forget(tokens.next().unwrap_or("").trim().to_string()),
            "amend" => {
                let rest = tokens.next().unwrap_or("").trim();
                match rest.split_once(char::is_whitespace) {
                    Some((id, text)) => Self::Amend(id.trim().to_string(), text.trim().to_string()),
                    None => Self::Amend(rest.to_string(), String::new()),
                }
            }
            _ => Self::Append(arg.to_string()),
        }
    }
}

/// First 8 chars of a hex id / project hash — display form only; all stored
/// operations run on full ids.
fn short_hex(hex: &str) -> String {
    hex.chars().take(8).collect()
}

/// Relative timestamp for recall surfaces: now / 5m ago / 3h ago / 2d ago.
pub fn rel_time(ts: chrono::DateTime<chrono::Utc>) -> String {
    let secs = (chrono::Utc::now() - ts).num_seconds().max(0) as u64;
    if secs < 60 {
        "now".to_string()
    } else if secs < 3600 {
        format!("{}m ago", secs / 60)
    } else if secs < 86_400 {
        format!("{}h ago", secs / 3600)
    } else {
        format!("{}d ago", secs / 86_400)
    }
}

/// Why a `/memory forget|amend <prefix>` id could not be resolved.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum NoteMatchError {
    NotFound(String),
    Ambiguous(String),
}

impl std::fmt::Display for NoteMatchError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::NotFound(given) => write!(f, "no note matches id '{given}'"),
            Self::Ambiguous(given) => {
                write!(f, "'{given}' matches several notes — use more of the id")
            }
        }
    }
}

/// Resolve a user-supplied note id (full 16-hex or unique prefix) to exactly
/// one note. An exact full-id hit wins even if it is also another's prefix.
pub fn find_note<'a>(
    notes: &'a [vak_core::memory::NoteBlock],
    given: &str,
) -> Result<&'a vak_core::memory::NoteBlock, NoteMatchError> {
    if let Some(note) = notes.iter().find(|n| n.id == given) {
        return Ok(note);
    }
    let matches: Vec<&vak_core::memory::NoteBlock> =
        notes.iter().filter(|n| n.id.starts_with(given)).collect();
    match matches.as_slice() {
        [one] => Ok(one),
        [] => Err(NoteMatchError::NotFound(given.to_string())),
        _ => Err(NoteMatchError::Ambiguous(given.to_string())),
    }
}

/// Opens another session's ledger purely for viewing: no session-slot swap,
/// so nothing can append to it and the active session stays untouched.
fn session_view_modal(
    home: &std::path::Path,
    cwd: &std::path::Path,
    hit: &SearchHit,
) -> Result<ModalView, String> {
    let dir = if hit.project_hash.is_empty() {
        vak_session::SessionPath::sessions_dir(home, cwd)
    } else {
        home.join("sessions").join(&hit.project_hash)
    };
    session_transcript_modal(&dir, &hit.session_id, "opened read-only from search")
}

/// Read-only transcript over any project dir's `<session_id>.jsonl`.
fn session_transcript_modal(
    dir: &std::path::Path,
    session_id: &str,
    origin: &str,
) -> Result<ModalView, String> {
    let path = dir.join(format!("{session_id}.jsonl"));
    let log = SessionLog::open(path).map_err(|e| e.to_string())?;
    let msgs = log.derive_messages();
    let start = msgs.len().saturating_sub(200);
    let (mut rows, anchors) = build_transcript_rows(&msgs, start);
    rows.insert(0, format!("{} messages · {origin}", msgs.len() - start));
    Ok(ModalView {
        title: format!("session transcript · {}", short_hex(session_id)),
        scroll: rows.len().saturating_sub(1),
        rows,
        footer: "read-only view · /resume resumes your own sessions · Esc close".to_string(),
        anchors,
        ..Default::default()
    })
}

const TASKS_FOOTER: &str =
    "/tasks enable|disable <name> toggles · creation/editing lives in `vakcoder tasks`";

/// `/tasks` — read-mostly manager over TaskStore. Bare lists the table;
/// `enable|disable <name>` toggles and persists. Creation/editing stays with
/// the CLI/desktop surfaces on purpose.
fn handle_tasks(core: &Core, arg: Option<&str>, screen: &mut Screen) {
    let mut store = match vak_core::tasks::TaskStore::load(&core.sessions_home()) {
        Ok(store) => store,
        Err(e) => {
            screen.error(&e.to_string());
            return;
        }
    };
    match arg.map(str::trim).filter(|a| !a.is_empty()) {
        None => {
            let tasks = store.all();
            let rows = if tasks.is_empty() {
                vec![
                    "no scheduled tasks".to_string(),
                    "create them with `vakcoder tasks` on the CLI or the desktop Tasks page"
                        .to_string(),
                ]
            } else {
                task_rows(&tasks, chrono::Local::now())
            };
            screen.panel("scheduled tasks", &rows, TASKS_FOOTER);
        }
        Some(argstr) => {
            let mut tokens = argstr.splitn(2, char::is_whitespace);
            let action = tokens.next().unwrap_or("");
            let name = tokens.next().map(str::trim).unwrap_or("");
            if !matches!(action, "enable" | "disable") || name.is_empty() {
                screen.error("usage: /tasks · /tasks enable <name> · /tasks disable <name>");
                return;
            }
            let enable = action == "enable";
            match store.all().into_iter().find(|t| t.name == name) {
                None => screen.error(&format!("no task named '{name}'")),
                Some(mut task) => {
                    task.enabled = enable;
                    store.put(task);
                    match store.save() {
                        Ok(()) => screen.success(&format!(
                            "{} task '{name}'",
                            if enable {
                                "✓ enabled"
                            } else {
                                "✗ disabled"
                            }
                        )),
                        Err(e) => screen.error(&e.to_string()),
                    }
                }
            }
        }
    }
}

/// Humanized interval for the schedule column when no cron expression is set.
pub fn fmt_interval(secs: u64) -> String {
    if secs == 0 {
        return "0s".to_string();
    }
    if secs.is_multiple_of(86_400) {
        format!("{}d", secs / 86_400)
    } else if secs.is_multiple_of(3600) {
        format!("{}h", secs / 3600)
    } else if secs.is_multiple_of(60) {
        format!("{}m", secs / 60)
    } else {
        format!("{secs}s")
    }
}

/// One row per task: name · kind · schedule · enabled · model pin · last run
/// · next cron fire (`cron_next_after` from `now`). Interval tasks show "-":
/// their next fire depends on ticker state this surface does not own.
pub fn task_rows(
    tasks: &[vak_core::tasks::TaskDef],
    now: chrono::DateTime<chrono::Local>,
) -> Vec<String> {
    const HEADER: [&str; 7] = [
        "name",
        "kind",
        "schedule",
        "on",
        "model",
        "last run",
        "next fire",
    ];
    let mut cells: Vec<[String; 7]> = Vec::with_capacity(tasks.len());
    for t in tasks {
        let sched = t
            .schedule
            .clone()
            .unwrap_or_else(|| fmt_interval(t.interval_secs));
        let last = t
            .last_run_at
            .map(rel_time)
            .unwrap_or_else(|| "never".to_string());
        let next = match (&t.schedule, t.enabled) {
            (Some(expr), true) => vak_core::tasks::cron_next_after(expr, now)
                .map(|dt| dt.format("%m-%d %H:%M").to_string())
                .unwrap_or_else(|_| "?".to_string()),
            _ => "-".to_string(),
        };
        cells.push([
            t.name.clone(),
            if t.script.is_some() {
                "script"
            } else {
                "prompt"
            }
            .to_string(),
            sched,
            if t.enabled { "✓" } else { "✗" }.to_string(),
            t.model_pin.clone().unwrap_or_else(|| "-".to_string()),
            last,
            next,
        ]);
    }
    let widths: [usize; 7] = std::array::from_fn(|i| {
        HEADER[i].chars().count().max(
            cells
                .iter()
                .map(|row| row[i].chars().count())
                .max()
                .unwrap_or(0),
        )
    });
    let line = |row: &[String; 7]| {
        row.iter()
            .zip(widths)
            .map(|(c, w)| format!("{c:<w$}"))
            .collect::<Vec<_>>()
            .join("  ")
    };
    let header = HEADER.map(String::from);
    std::iter::once(line(&header))
        .chain(cells.iter().map(line))
        .collect()
}

fn mode_label(mode: PermissionMode) -> &'static str {
    match mode {
        PermissionMode::ReadOnly => "read-only",
        PermissionMode::WorkspaceWrite => "workspace-write",
        PermissionMode::FullAccess => "full-access",
    }
}

fn mode_modal_rows(core: &Core) -> Vec<String> {
    vec![
        format!(
            "current      {}",
            mode_label(core.effective_permission_mode())
        ),
        String::new(),
        "read-only         every write, edit, and bash exec asks".to_string(),
        "workspace-write   workspace edits allowed; outside writes ask".to_string(),
        "full-access       unsandboxed · explicit human trust decision".to_string(),
        String::new(),
        "/mode <mode> switches immediately when idle".to_string(),
        "switching mid-run cancels the run and denies pending approvals first".to_string(),
    ]
}

fn sandbox_rows(core: &Core) -> Vec<String> {
    let docker = if vak_core::sandbox_docker::DockerSandbox::available() {
        "available"
    } else {
        "unreachable"
    };
    let image = core
        .config()
        .sandbox
        .image
        .clone()
        .unwrap_or_else(|| "default".into());
    vec![
        format!("effective     {}", core.effective_sandbox_name()),
        format!(
            "backend       {} (config default: {})",
            core.effective_sandbox_backend(),
            core.config().sandbox.backend
        ),
        format!("docker        {docker} · image {image}"),
        String::new(),
        "os            platform-native (Seatbelt on macOS, Landlock on Linux)".to_string(),
        "docker        commands run inside the configured image".to_string(),
        "default       follow config.toml [sandbox]".to_string(),
        String::new(),
        "full-access runs without any sandbox by design".to_string(),
    ]
}

fn composer_label(ui: &UiState, inbox_unread: usize) -> String {
    let base = match &ui.attached_label {
        Some(label) => {
            let provider_model = format!("{}/{}", ui.provider, ui.model);
            format!("subagent · {} · {provider_model}", trunc_cells(label, 24))
        }
        None => format!("task · {}/{}", ui.provider, ui.model),
    };
    format!("{base}{}", inbox_badge(inbox_unread))
}

fn composer_footer(editor: &Editor) -> String {
    let (line, column) = editor.line_col();
    let mode_tag = match (editor.mode(), editor.vim_state()) {
        (ComposerMode::Vim, VimState::Insert) => "[-- INSERT --] ",
        (ComposerMode::Vim, VimState::Normal) => "[NORMAL] ",
        (ComposerMode::Emacs, _) => "",
    };
    let mut footer = format!(
        "{mode_tag}Ln {}, Col {} · Enter send · Alt-Enter newline · Ctrl-P commands",
        line + 1,
        column + 1
    );
    if editor.mode() == ComposerMode::Vim {
        footer.push_str(" · /composer emacs exits modal");
    }
    if editor.has_stashes() {
        footer.push_str(&format!(
            " · {} stashed paste(s), exact on submit",
            editor.stash_count()
        ));
    }
    footer
}

fn draw_picker(screen: &mut Screen, kind: PickerKind, picker: &ChoicePicker) {
    let items = picker.filtered().into_iter().cloned().collect::<Vec<_>>();
    let (title, allow_custom) = match kind {
        PickerKind::Provider => ("choose provider", false),
        PickerKind::Model => ("choose model · type any exact model ID", true),
        PickerKind::Theme => ("choose theme · up/down previews live", false),
        PickerKind::Subagents => ("attach to a running subagent", false),
    };
    screen.redraw_picker(
        title,
        picker.query(),
        &items,
        picker.selected(),
        allow_custom,
    );
}

/// Builds the interactive keymap modal plus the selectable-row metadata:
/// each entry maps a row index to the action name bound there.
fn build_keymap_modal(
    keymap: &Keymap,
    selected: Option<usize>,
) -> (ModalView, Vec<(usize, &'static str)>) {
    let mut rows: Vec<String> = Vec::new();
    let mut meta: Vec<(usize, &'static str)> = Vec::new();
    let mut last_ctx = "";
    for (ctx, key, action) in keymap.rows() {
        if ctx != last_ctx {
            rows.push(format!("-- {ctx}"));
            last_ctx = ctx;
        }
        meta.push((rows.len(), Box::leak(action.to_string().into_boxed_str())));
        rows.push(format!("{key:<16} {action}"));
    }
    let conflicts = keymap.conflicts();
    if !conflicts.is_empty() {
        rows.push(String::new());
        rows.push("!! conflicts".to_string());
        for (key, detail) in &conflicts {
            rows.push(format!("{key:<16} {detail}"));
        }
    }
    (
        ModalView {
            title: "keymap · bindings".to_string(),
            rows,
            scroll: 0,
            footer: "up/down select · r rebinds the highlighted row · Esc close".to_string(),
            selected,
            ..Default::default()
        },
        meta,
    )
}

/// Moves the modal selection through selectable rows.
fn step_select(
    current: Option<usize>,
    meta: &[(usize, &'static str)],
    forward: bool,
) -> Option<usize> {
    if meta.is_empty() {
        return None;
    }
    let pos = current
        .and_then(|sel| meta.iter().position(|(row, _)| *row == sel))
        .unwrap_or(0);
    let next = if forward {
        (pos + 1) % meta.len()
    } else {
        (pos + meta.len() - 1) % meta.len()
    };
    Some(meta[next].0)
}

/// Custom/plugin commands surfaced as palette actions alongside built-ins.
fn palette_extras(core: &Core) -> Vec<PaletteItem> {
    core.custom_commands()
        .into_iter()
        .map(|c| PaletteItem {
            name: c.name,
            description: format!("{} ({})", trunc_cells(&c.description, 40), c.source),
        })
        .collect()
}

/// Opens the live-subagent attach picker; empty registry gets a hint line.
fn open_subagent_picker(
    core: &Core,
    screen: &mut Screen,
    picker: &mut Option<(PickerKind, ChoicePicker)>,
) {
    let active = core.subagents().active();
    if active.is_empty() {
        screen.dim("no running subagents — spawn one with the task tool");
        return;
    }
    let choices = active
        .iter()
        .map(|a| ChoiceItem {
            value: a.id.clone(),
            description: format!(
                "{} · running {}s",
                trunc_cells(&a.label, 44),
                a.elapsed_secs
            ),
            active: false,
        })
        .collect();
    let active_picker = ChoicePicker::new(choices, false);
    draw_picker(screen, PickerKind::Subagents, &active_picker);
    *picker = Some((PickerKind::Subagents, active_picker));
}

/// Explicit OSC52 clipboard copy of the last response. Gated by
/// `[ui] osc52 = true`: the clipboard is never touched implicitly.
fn copy_last_response(core: &Core, screen: &mut Screen, ui: &UiState) {
    if !core.config().ui.osc52 {
        screen.dim("OSC52 is off — set [ui] osc52 = true in config, then Alt-Y or /copy");
        return;
    }
    let text = ui.last_response.trim_end();
    if text.is_empty() {
        screen.dim("nothing to copy yet");
        return;
    }
    screen.osc52_copy(text);
    screen.success(&format!("copied {} bytes via OSC52", text.len()));
}

fn theme_choices(current: &str, custom: &CustomThemes) -> Vec<ChoiceItem> {
    let mut items: Vec<ChoiceItem> = [
        ("dark", "balanced charcoal · calm cyan signals"),
        ("light", "paper-bright · crisp blue contrast"),
        ("neo", "electric cyan and magenta · high energy"),
        ("rich", "deep jewel tones · amber and violet"),
        (
            "teenage",
            "Teenage Engineering-inspired · cream, ink, and orange",
        ),
        ("plain", "terminal defaults · no imposed color palette"),
        ("midnight", "truecolor deep blue · soft moonlight accents"),
        ("synthwave", "truecolor neon pink/cyan over violet dusk"),
        ("forest", "truecolor moss greens on pine ground"),
    ]
    .into_iter()
    .map(|(value, description)| ChoiceItem {
        value: value.to_string(),
        description: description.to_string(),
        active: value == current,
    })
    .collect();
    for name in custom.keys() {
        if items.iter().any(|i| &i.value == name) {
            continue;
        }
        items.push(ChoiceItem {
            value: name.clone(),
            description: format!(
                "custom [ui.themes]{}",
                if *name == current { " · current" } else { "" }
            ),
            active: *name == current,
        });
    }
    items
}

fn provider_choices(core: &Core, current: &str) -> Vec<ChoiceItem> {
    core.provider_names()
        .into_iter()
        .map(|value| ChoiceItem {
            description: provider_status(core, &value),
            active: value == current,
            value,
        })
        .collect()
}

/// One source of truth for provider→env-var wiring: the Core's own map.
/// Readiness goes through the same lookup runs use (real env, runtime
/// overrides, then loaded .env files), so a key stored from any surface —
/// or sitting in ~/.vakcoder/.env — shows as ready everywhere.
fn provider_status(core: &Core, provider: &str) -> String {
    match Core::provider_env_var(provider) {
        None if !Core::provider_known(provider) => "custom provider".to_string(),
        None => "local · no API key".to_string(),
        Some(env) => {
            if core.provider_configured(provider) {
                format!("ready · {env}")
            } else {
                format!("needs {env} · /key {provider} SECRET")
            }
        }
    }
}

async fn model_choices(core: &Core, provider: &str, current: &str) -> Vec<ChoiceItem> {
    let hint = |value: &str| match value {
        "claude-sonnet-4-5" => "balanced coding",
        "claude-haiku-4-5" => "fast coding",
        "claude-opus-4-1" | "gemini-2.5-pro" => "deep reasoning",
        "gpt-5-codex" => "frontier coding",
        "gpt-5" => "frontier general",
        "o3" | "gpt-4.1" => "strong general",
        "gpt-4o" => "fast general model",
        "x-preview-f-free" => "free preview",
        _ if value.contains('/') => "routed",
        _ => "suggested by core",
    };
    let mut choices = vec![ChoiceItem {
        value: current.to_string(),
        description: "current model".to_string(),
        active: true,
    }];
    // Ask the provider what this key reaches; on failure the picker still
    // offers the current model rather than a stale baked-in list.
    for value in core.discover_models(provider).await.unwrap_or_default() {
        if value != current {
            choices.push(ChoiceItem {
                description: hint(&value).to_string(),
                value,
                active: false,
            });
        }
    }
    choices
}

async fn default_model(core: &Core, provider: &str) -> Option<String> {
    core.discover_models(provider).await.ok()?.first().cloned()
}

/// `/view <path>` — read a workspace file into a modal.
///
/// Mirrors what the desktop editor does: text is shown with line numbers,
/// while images and binaries report their kind and size instead of spraying
/// bytes at the terminal. Paths are confined to the workspace.
fn view_file_modal(core: &Core, rel: &str) -> Result<ModalView, String> {
    let cwd = core.cwd();
    let joined = if std::path::Path::new(rel).is_absolute() {
        std::path::PathBuf::from(rel)
    } else {
        cwd.join(rel)
    };
    let path = joined
        .canonicalize()
        .map_err(|_| format!("no such file: {rel}"))?;
    let root = cwd.canonicalize().unwrap_or_else(|_| cwd.clone());
    if !path.starts_with(&root) {
        return Err(format!("path outside the workspace: {rel}"));
    }
    let bytes = std::fs::read(&path).map_err(|e| format!("{rel}: {e}"))?;
    let kind = vak_core::files::classify(&path, &bytes);
    let size = vak_core::files::format_bytes(bytes.len() as u64);
    let name = path
        .strip_prefix(&root)
        .unwrap_or(&path)
        .to_string_lossy()
        .into_owned();

    let rows: Vec<String> = match kind {
        vak_core::files::FileKind::Text => {
            let text = String::from_utf8_lossy(&bytes);
            let total = text.lines().count();
            let width = total.to_string().len().max(2);
            text.lines()
                .take(MAX_VIEW_LINES)
                .enumerate()
                .map(|(i, line)| format!("{:>width$} │ {}", i + 1, line, width = width))
                .chain(
                    (total > MAX_VIEW_LINES)
                        .then(|| format!("… {} more lines", total - MAX_VIEW_LINES)),
                )
                .collect()
        }
        vak_core::files::FileKind::Image => vec![
            format!("image · {size}"),
            String::new(),
            "Terminals cannot render this; open it in the desktop app".to_string(),
        ],
        vak_core::files::FileKind::Binary => vec![
            format!("binary · {size}"),
            String::new(),
            "Not shown: these bytes are not text.".to_string(),
        ],
    };

    Ok(ModalView {
        title: format!("{name} · {} · {size}", kind.as_str()),
        rows,
        scroll: 0,
        footer: "↑↓ scroll · Esc close".to_string(),
        ..Default::default()
    })
}

/// Cap on lines rendered by `/view`; the modal scrolls, but building a
/// million rows for a huge file would stall the redraw.
const MAX_VIEW_LINES: usize = 5000;

fn help_modal_rows(core: &Core) -> Vec<String> {
    let mut rows = vec!["COMMANDS".to_string(), "".to_string()];
    rows.extend(commands::help_rows());
    let customs = core.custom_commands();
    if !customs.is_empty() {
        rows.push("".to_string());
        rows.push("CUSTOM COMMANDS".to_string());
        for c in &customs {
            rows.push(format!(
                "/{:<14} {} ({})",
                c.name,
                if c.description.is_empty() {
                    "(no description)"
                } else {
                    c.description.as_str()
                },
                c.source
            ));
        }
    }
    rows.extend([
        "".to_string(),
        "PROMPT INPUT".to_string(),
        "@path             attach exact file contents".to_string(),
        "!command          run a local shell command".to_string(),
        "Alt-Enter         insert a newline".to_string(),
        "Ctrl-R            search prompt history".to_string(),
        "Ctrl-P            search every command".to_string(),
        "".to_string(),
        "DURING A RUN".to_string(),
        "Enter             steer the active agent (or attached subagent)".to_string(),
        "Tab               queue the next turn".to_string(),
        "Alt-S             attach to a running subagent".to_string(),
        "Alt-Y             copy last response via OSC52 when enabled".to_string(),
        "Alt-A             review pending approval".to_string(),
        "Ctrl-O            expand a stashed large paste at the cursor".to_string(),
        "Ctrl-G            edit the composer draft in $EDITOR".to_string(),
        "Ctrl-C            interrupt and preserve partial output".to_string(),
    ]);
    rows
}

fn settings_rows(core: &Core, ui: &UiState) -> Vec<String> {
    let project = core.cwd().join(".vakcoder/config.toml");
    let global = std::env::var_os("HOME")
        .map(std::path::PathBuf::from)
        .map(|home| home.join(".config/vakcoder/config.toml"));
    let mut rows = vec![
        "AGENT".to_string(),
        format!(
            "Provider              {} · {}",
            ui.provider,
            provider_status(core, &ui.provider)
        ),
        format!("Model                 {}", ui.model),
        format!("Maximum turns         {}", core.effective_max_turns()),
        format!("Maximum output        {} tokens", core.config().max_tokens),
        format!(
            "Context window        {} tokens",
            core.config().context_window
        ),
        format!(
            "Subagents             {}",
            if core.config().subagents {
                "enabled"
            } else {
                "disabled"
            }
        ),
        "".to_string(),
        "SAFETY".to_string(),
        format!(
            "Permission mode       {:?}",
            core.effective_permission_mode()
        ),
        format!("Sandbox               {}", core.effective_sandbox_name()),
        format!(
            "Sandbox backend       {} (default {})",
            core.effective_sandbox_backend(),
            core.config().sandbox.backend
        ),
        format!(
            "Allow / ask / deny    {} / {} / {} rules",
            core.config().allow.len(),
            core.config().ask.len(),
            core.config().deny.len()
        ),
        format!(
            "Learned allow rules   {}",
            core.extra_allow_snapshot().len()
        ),
        "".to_string(),
        "RELIABILITY".to_string(),
        format!(
            "Request retries       {} · base {}ms",
            core.config().max_retries,
            core.config().retry_base_backoff_ms
        ),
        format!(
            "Request watchdog      {}s",
            core.config().request_timeout_secs
        ),
        format!(
            "Run endurance         {} · base {}ms",
            core.config().run_retry_attempts,
            core.config().run_retry_base_backoff_ms
        ),
        format!(
            "Circuit breaker       {} failures · {}s cooldown",
            core.config().circuit_breaker_threshold,
            core.config().circuit_breaker_cooldown_secs
        ),
        format!(
            "Breaker state         {}",
            match core.breaker().check() {
                Ok(()) => "closed".to_string(),
                Err(open) => format!("open · {}s remaining", open.remaining_secs),
            }
        ),
        format!(
            "Completion guard      {} · max {} continuations",
            if core.config().stop_policy.enabled {
                "enabled"
            } else {
                "disabled"
            },
            core.config().stop_policy.max_blocks
        ),
        "".to_string(),
        "INTERFACE".to_string(),
        format!(
            "Theme / bell          {} / {}",
            core.effective_theme(),
            if core.config().ui.bell { "on" } else { "off" }
        ),
        format!("Composer mode         {}", core.config().ui.composer),
        format!(
            "OSC52 clipboard       {}",
            if core.config().ui.osc52 {
                "opt-in (Alt-Y)"
            } else {
                "off"
            }
        ),
        format!(
            "Accessibility          {}{}{}",
            if core.config().ui.accessibility.plain {
                "plain "
            } else {
                ""
            },
            if core.config().ui.accessibility.reduced_motion {
                "reduced-motion "
            } else {
                ""
            },
            if core.config().ui.accessibility.screen_reader {
                "screen-reader"
            } else {
                ""
            },
        ),
        "".to_string(),
        "EXTENSIONS".to_string(),
        format!("Skills                {} discovered", core.skills().len()),
        format!(
            "Hooks                 {} configured",
            core.config().hooks.len()
        ),
        format!(
            "MCP servers           {} configured",
            core.config().mcp.servers.len()
        ),
        "".to_string(),
        "PATHS".to_string(),
        format!("Project config        {}", project.display()),
        format!("Session store         {}", core.sessions_home().display()),
    ];
    if let Some(global) = global {
        rows.push(format!("Global config         {}", global.display()));
    }
    if !core.config().warnings.is_empty() {
        rows.push("".to_string());
        rows.push("WARNINGS".to_string());
        rows.extend(
            core.config()
                .warnings
                .iter()
                .map(|warning| format!("! {warning}")),
        );
    }
    rows.extend([
        "".to_string(),
        "CHANGE SETTINGS".to_string(),
        "P provider · M model · T theme · F feature explorer".to_string(),
        "/provider, /model, and /theme open searchable pickers".to_string(),
        "Enter applies a picker choice to this session".to_string(),
        "Ctrl-S saves the chosen value to the project config".to_string(),
        "/doctor diagnoses setup".to_string(),
    ]);
    rows
}

fn feature_rows(core: &Core) -> Vec<String> {
    let tools = core.tool_names().join(", ");
    let skills = core
        .skills()
        .iter()
        .map(|skill| skill.name.as_str())
        .take(8)
        .collect::<Vec<_>>()
        .join(", ");
    let mcp = core
        .config()
        .mcp
        .servers
        .keys()
        .map(String::as_str)
        .collect::<Vec<_>>()
        .join(", ");
    vec![
        "AGENT RUNTIME".to_string(),
        "✓ streamed text and thinking with delta + snapshot events".to_string(),
        "✓ mid-run steering and queued follow-up turns".to_string(),
        "✓ parallel tool waves with resource-conflict scheduling".to_string(),
        format!(
            "{} bounded subagents with child-session lineage",
            if core.config().subagents {
                "✓"
            } else {
                "○"
            }
        ),
        "✓ context compaction and token-budget enforcement".to_string(),
        "".to_string(),
        "SAFETY AND STATE".to_string(),
        "✓ read-only, workspace-write, and full-access permission modes".to_string(),
        "✓ informed approvals with once/session/persistent scopes".to_string(),
        format!("✓ {} OS sandbox", core.effective_sandbox_name()),
        "✓ append-only JSONL session trees and reconstructable model input".to_string(),
        "✓ workspace checkpoints, rewind, branching, and resume".to_string(),
        "✓ cancellation preserves partial model and tool output".to_string(),
        "".to_string(),
        "RELIABILITY".to_string(),
        "✓ exponential retry, Retry-After, watchdog deadlines".to_string(),
        "✓ run-level endurance and shared circuit breaker".to_string(),
        "✓ premature-completion stop gate and doom-loop protection".to_string(),
        "✓ typed retry, compaction, stale-stream, and failure states".to_string(),
        "".to_string(),
        "TOOLS AND EXTENSIONS".to_string(),
        format!("Built-ins              {tools}"),
        format!(
            "Skills                 {}{}",
            core.skills().len(),
            if skills.is_empty() {
                String::new()
            } else {
                format!(" · {skills}")
            }
        ),
        format!(
            "Hooks                  {} lifecycle handlers",
            core.config().hooks.len()
        ),
        format!(
            "MCP                    {}{}",
            core.config().mcp.servers.len(),
            if mcp.is_empty() {
                String::new()
            } else {
                format!(" · {mcp}")
            }
        ),
        "".to_string(),
        "WORKFLOWS AND CLIENTS".to_string(),
        "✓ interactive TUI and headless exec".to_string(),
        "✓ dynamic planner and validated static flow DAGs".to_string(),
        "✓ deterministic and live evaluation suites".to_string(),
        "✓ HTTP + SSE server with approvals, steering, transcripts, and diffs".to_string(),
        "✓ Tauri desktop workspace, editor, terminal, side chats, and tasks".to_string(),
        "".to_string(),
        "TUI ENTRY POINTS".to_string(),
        "/sessions · /resume · /rewind · /transcript".to_string(),
        "/provider · /model · /settings · /doctor".to_string(),
        "/cost · /context · /details · /theme · /keys".to_string(),
        "/subagents attach-steer · /composer vim · /a11y · /copy OSC52".to_string(),
        "@file attachments · !shell · Ctrl-P palette · Ctrl-R history".to_string(),
    ]
}

fn persist_agent_config(
    core: &Core,
    provider: &str,
    model: &str,
) -> Result<std::path::PathBuf, std::io::Error> {
    let path = core.cwd().join(".vakcoder/config.toml");
    let current = read_optional_config(&path)?;
    let next = upsert_top_level(
        &upsert_top_level(&current, "provider", provider),
        "model",
        model,
    );
    write_project_config(path, next)
}

fn persist_theme_config(core: &Core, theme: &str) -> Result<std::path::PathBuf, std::io::Error> {
    let path = core.cwd().join(".vakcoder/config.toml");
    let current = read_optional_config(&path)?;
    let next = upsert_table_value(&current, "ui", "theme", theme);
    write_project_config(path, next)
}

fn read_optional_config(path: &std::path::Path) -> Result<String, std::io::Error> {
    match std::fs::read_to_string(path) {
        Ok(value) => Ok(value),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(String::new()),
        Err(error) => Err(error),
    }
}

fn write_project_config(
    path: std::path::PathBuf,
    contents: String,
) -> Result<std::path::PathBuf, std::io::Error> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let temporary = path.with_extension("toml.tmp");
    std::fs::write(&temporary, contents)?;
    std::fs::rename(&temporary, &path)?;
    Ok(path)
}

fn upsert_top_level(source: &str, key: &str, value: &str) -> String {
    let assignment = format!("{key} = \"{}\"", toml_escape(value));
    let mut replaced = false;
    let mut in_table = false;
    let mut lines = Vec::new();
    for line in source.lines() {
        let trimmed = line.trim_start();
        if trimmed.starts_with('[') {
            in_table = true;
        }
        let is_key = !in_table
            && trimmed
                .strip_prefix(key)
                .is_some_and(|tail| tail.trim_start().starts_with('='));
        if is_key {
            if !replaced {
                lines.push(assignment.clone());
                replaced = true;
            }
        } else {
            lines.push(line.to_string());
        }
    }
    if !replaced {
        lines.insert(0, assignment);
    }
    let mut result = lines.join("\n");
    result.push('\n');
    result
}

fn upsert_table_value(source: &str, table: &str, key: &str, value: &str) -> String {
    let assignment = format!("{key} = \"{}\"", toml_escape(value));
    let table_header = format!("[{table}]");
    let mut found_table = false;
    let mut in_table = false;
    let mut replaced = false;
    let mut lines = Vec::new();
    for line in source.lines() {
        let trimmed = line.trim();
        if trimmed.starts_with('[') {
            if in_table && !replaced {
                lines.push(assignment.clone());
                replaced = true;
            }
            in_table = trimmed == table_header;
            found_table |= in_table;
        }
        let is_key = in_table
            && trimmed
                .strip_prefix(key)
                .is_some_and(|tail| tail.trim_start().starts_with('='));
        if is_key {
            if !replaced {
                lines.push(assignment.clone());
                replaced = true;
            }
        } else {
            lines.push(line.to_string());
        }
    }
    if in_table && !replaced {
        lines.push(assignment.clone());
    }
    if !found_table {
        if lines.last().is_some_and(|line| !line.is_empty()) {
            lines.push(String::new());
        }
        lines.push(table_header);
        lines.push(assignment);
    }
    let mut result = lines.join("\n");
    result.push('\n');
    result
}

fn toml_escape(value: &str) -> String {
    value.replace('\\', "\\\\").replace('"', "\\\"")
}

fn handle_complete(editor: &mut Editor, core: &Core, screen: &mut Screen) {
    let buf = editor.view().0.to_string();
    let customs = core.custom_commands();
    let mut all: Vec<(&str, &str)> = commands::COMMANDS.to_vec();
    all.extend(
        customs
            .iter()
            .map(|c| (c.name.as_str(), c.description.as_str())),
    );
    let suggestions = complete::complete(&buf, &all, core.cwd());
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

#[cfg(test)]
mod config_edit_tests {
    use super::upsert_table_value;

    #[test]
    fn theme_update_preserves_neighboring_ui_and_other_tables() {
        let source = "provider = \"anthropic\"\n\n[ui]\nbell = false\ntheme = \"dark\"\n\n[mcp]\n";
        let updated = upsert_table_value(source, "ui", "theme", "teenage");
        assert!(updated.contains("provider = \"anthropic\""));
        assert!(updated.contains("[ui]\nbell = false\ntheme = \"teenage\""));
        assert!(updated.contains("[mcp]"));
        assert_eq!(updated.matches("theme =").count(), 1);
    }

    #[test]
    fn theme_update_creates_ui_table_when_absent() {
        let updated = upsert_table_value("model = \"m\"\n", "ui", "theme", "neo");
        assert_eq!(updated, "model = \"m\"\n\n[ui]\ntheme = \"neo\"\n");
    }
}

#[cfg(test)]
mod transcript_state_tests {
    use super::*;
    use vak_llm::types::{ContentBlock, Message};

    #[test]
    fn run_states_label_the_row() {
        assert_eq!(RunState::Thinking.label(), "thinking");
        assert_eq!(RunState::Streaming.label(), "streaming");
        assert_eq!(
            RunState::Tool {
                name: "bash".into()
            }
            .label(),
            "tool · bash"
        );
        assert_eq!(
            RunState::Retrying {
                attempt: 2,
                delay_ms: 1500,
                reason: "rate-limited".into()
            }
            .label(),
            "retry 2 in 1500ms — rate-limited"
        );
        assert_eq!(RunState::Compacting.label(), "compacting context");
    }

    #[test]
    fn status_row_shows_typed_state_not_bare_spinner() {
        let theme = Theme::from_name("dark");
        let retrying = RunState::Retrying {
            attempt: 1,
            delay_ms: 500,
            reason: "overloaded".into(),
        };
        for (state, needle) in [
            (&retrying, "retry 1"),
            (&RunState::Compacting, "compacting context"),
            (
                &RunState::Tool {
                    name: "edit".into(),
                },
                "tool · edit",
            ),
            (&RunState::Thinking, "thinking"),
        ] {
            let row = status_ansi(&theme, state, 10, 20, 5, 0, 128_000, 0, true, None);
            let still_plain = crate::markdown::strip_ansi(&status_ansi(
                &theme, state, 10, 20, 5, 0, 128_000, 0, false, None,
            ));
            assert!(
                !still_plain.contains('\u{280b}') && !still_plain.contains('\u{23f3}'),
                "reduced motion drops animated glyphs: {still_plain}"
            );
            assert!(
                crate::markdown::strip_ansi(&row).contains(needle),
                "'{needle}' missing from: {}",
                crate::markdown::strip_ansi(&row)
            );
        }
    }

    #[test]
    fn transcript_rows_mark_user_prompts_as_jump_anchors() {
        let msgs = vec![
            Message::user_text("first prompt"),
            Message::assistant(vec![ContentBlock::text("reply one")]),
            Message::user_text("second prompt"),
            Message::assistant(vec![ContentBlock::tool_result("t1", "ok")]),
        ];
        let (rows, anchors) = build_transcript_rows(&msgs, 0);
        assert_eq!(anchors.len(), 2, "one anchor per user message");
        let plain = rows.join("\n");
        assert!(plain.contains("0 ▸ user"));
        assert!(plain.contains("2 ▸ user"));
        assert!(plain.contains("1 ◆ assistant"));
        assert!(plain.contains("→ ok"));
        // Anchor indices point at the user rows within `rows`.
        assert!(rows[anchors[0]].contains("▸ user"));
        assert!(rows[anchors[1]].contains("▸ user"));
    }

    #[test]
    fn transcript_window_offset_keeps_absolute_indices() {
        let msgs: Vec<Message> = (0..6)
            .map(|i| Message::user_text(format!("prompt {i}")))
            .collect();
        let (rows, _) = build_transcript_rows(&msgs, 4);
        let plain = rows.join("\n");
        assert!(plain.contains("4 ▸ user"), "{plain}");
        assert!(!plain.contains("3 ▸ user"), "{plain}");
    }
}

#[cfg(test)]
mod transcript_search_tests {
    use super::find_matches;

    #[test]
    fn search_is_case_insensitive_and_ordered() {
        let rows: Vec<String> = vec![
            "0 ▸ user".into(),
            "     fix the parser".into(),
            "1 ◆ assistant".into(),
            "     Fix The Parser again".into(),
        ];
        assert_eq!(find_matches(&rows, "parser"), vec![1, 3]);
        assert_eq!(find_matches(&rows, "PARSER"), vec![1, 3]);
    }

    #[test]
    fn empty_or_blank_queries_match_nothing() {
        let rows = vec!["anything".to_string()];
        assert!(find_matches(&rows, "").is_empty());
        assert!(find_matches(&rows, "   ").is_empty());
    }

    #[test]
    fn no_hits_return_empty_vec() {
        let rows = vec!["alpha".to_string(), "beta".to_string()];
        assert!(find_matches(&rows, "gamma").is_empty());
    }
}

#[cfg(test)]
mod personal_os_tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]
    use super::*;
    use chrono::TimeZone as _;
    use vak_core::memory::NoteBlock;
    use vak_core::tasks::TaskDef;

    fn task(name: &str) -> TaskDef {
        TaskDef {
            id: format!("test-{name}"),
            name: name.into(),
            prompt: "tidy".into(),
            interval_secs: 3600,
            enabled: true,
            cwd: std::path::PathBuf::from("/tmp/ws"),
            created_at: chrono::Utc::now(),
            last_run_at: None,
            last_session_id: None,
            last_summary: None,
            last_wt: None,
            deliver_to: None,
            schedule: None,
            script: None,
            model_pin: None,
        }
    }

    fn note(id: &str, text: &str) -> NoteBlock {
        NoteBlock {
            id: id.into(),
            ts: chrono::Utc::now(),
            kind: "fact".into(),
            tag: "x".into(),
            session_id: "s".into(),
            text: text.into(),
        }
    }

    use vak_core::inbox::{Entry, Kind};

    fn inbox_entry(id: &str, kind: Kind, title: &str) -> Entry {
        Entry {
            id: id.into(),
            ts: chrono::Utc::now(),
            kind,
            title: title.into(),
            body: String::new(),
            session_id: None,
            task_id: None,
        }
    }

    fn bare_ui() -> UiState {
        UiState {
            styler: LineStyler::new(),
            partial: String::new(),
            total_in: 0,
            total_out: 0,
            tool_args: HashMap::new(),
            thinking_shown: false,
            model: "test-model".into(),
            provider: "test-provider".into(),
            cost_usd: 0.0,
            sub_in: 0,
            sub_out: 0,
            thinking_mode: ThinkingMode::Off,
            thinking_partial: String::new(),
            expanded_tools: false,
            run_state: RunState::Thinking,
            last_response: String::new(),
            attached_label: None,
        }
    }

    #[test]
    fn inbox_prefix_resolution_prefers_unread_then_all() {
        let unread_entry = inbox_entry("aaaa111111111111", Kind::Digest, "unread");
        let acked_a = inbox_entry("aaaa222222222222", Kind::Digest, "read one");
        let acked_b = inbox_entry("bbbb333333333333", Kind::Digest, "read two");
        let unread = vec![unread_entry.clone()];
        let all = vec![unread_entry.clone(), acked_a, acked_b.clone()];
        let id_of = |r: Result<&Entry, InboxMatchError>| r.map(|e| e.id.clone());

        // Exact full-id hit wins even when it lives in the acked tier only.
        assert_eq!(
            id_of(resolve_entry(&unread, &all, "aaaa222222222222")),
            Ok("aaaa222222222222".to_string())
        );
        // Unique prefix in the unread tier shadows acked entries sharing it.
        assert_eq!(
            id_of(resolve_entry(&unread, &all, "aaaa")),
            Ok("aaaa111111111111".to_string())
        );
        // Prefix unique only among acked entries falls through to the ledger.
        assert_eq!(id_of(resolve_entry(&[], &all, "bbbb")), Ok(acked_b.id));
        // Ambiguity inside the ledger tier is refused.
        assert_eq!(
            resolve_entry(&[], &all, "aaaa"),
            Err(InboxMatchError::Ambiguous("aaaa".into()))
        );
        assert_eq!(
            resolve_entry(&unread, &all, "ffff"),
            Err(InboxMatchError::NotFound("ffff".into()))
        );
    }

    #[test]
    fn inbox_arg_grammar_splits_all_and_ack() {
        assert!(matches!(
            InboxAction::parse(None),
            Ok(InboxAction::List { all: false })
        ));
        assert!(matches!(
            InboxAction::parse(Some("   ")),
            Ok(InboxAction::List { all: false })
        ));
        assert!(matches!(
            InboxAction::parse(Some("all")),
            Ok(InboxAction::List { all: true })
        ));
        assert!(matches!(
            InboxAction::parse(Some("ack abc12345")),
            Ok(InboxAction::Ack(ref id)) if id == "abc12345"
        ));
        // Missing id and unknown actions are usage errors, never guesses.
        assert!(InboxAction::parse(Some("ack")).is_err());
        assert!(InboxAction::parse(Some("wat")).is_err());
    }

    #[test]
    fn inbox_chips_cover_every_kind_with_theme_colors() {
        let theme = Theme::from_name("dark");
        for kind in [
            Kind::TaskSummary,
            Kind::ApprovalPending,
            Kind::ApprovalDenied,
            Kind::BudgetAlert,
            Kind::Digest,
            Kind::Heartbeat,
            Kind::ProposalOpened,
        ] {
            let chip = kind_chip(kind);
            assert!(chip.chars().count() <= 8, "{chip} breaks the column");
            // Every kind carries its own theme slot, never the default fg.
            assert_ne!(kind_color(kind, &theme), Color::Reset);
        }
        // Distinct kinds land in distinct slots where semantics differ.
        assert_ne!(
            kind_color(Kind::ApprovalDenied, &theme),
            kind_color(Kind::Digest, &theme)
        );

        let e = inbox_entry("0123456789abcdef", Kind::BudgetAlert, "day cap 92%");
        let row = inbox_row(&e, &theme);
        // The chip is embedded with its theme color; the base heading color
        // is restored after so the row tail stays uniform.
        assert!(row.contains(&theme::fg(theme.warning)), "{row}");
        assert!(row.contains(&theme::fg(theme.heading)));
        assert!(crate::markdown::strip_ansi(&row).contains("budget"));
    }

    #[test]
    fn inbox_rows_render_rel_ts_chip_title_and_id8() {
        let theme = Theme::from_name("dark");
        let e = inbox_entry(
            "0123456789abcdef",
            Kind::Digest,
            "weekly   digest\nline two",
        );
        let plain = crate::markdown::strip_ansi(&inbox_row(&e, &theme));
        assert_eq!(
            plain, "now · digest · weekly digest line two · 01234567",
            "rel-ts · chip · collapsed title · id8"
        );

        let long = inbox_entry(
            "ffffffffffffffff",
            Kind::Heartbeat,
            &format!("{} tail", "x".repeat(60)),
        );
        let plain = crate::markdown::strip_ansi(&inbox_row(&long, &theme));
        assert!(plain.contains('…'), "{plain}");
        assert!(!plain.contains("tail"), "{plain}");
    }

    #[test]
    fn inbox_badge_gates_on_positive_unread() {
        assert_eq!(inbox_badge(0), "");
        assert_eq!(inbox_badge(12), " · ✉ 12");

        let mut ui = bare_ui();
        let idle = composer_label(&ui, 0);
        assert!(!idle.contains('✉'), "{idle}");
        assert!(composer_label(&ui, 2).contains("· ✉ 2"));
        ui.attached_label = Some("nightly digest".into());
        let attached = composer_label(&ui, 1);
        assert!(attached.starts_with("subagent · "), "{attached}");
        assert!(attached.contains("✉ 1"), "{attached}");
    }

    #[test]
    fn task_table_lists_kind_schedule_pin_and_next_fire() {
        let now = chrono::Local
            .with_ymd_and_hms(2026, 8, 24, 12, 0, 0)
            .earliest()
            .unwrap();

        let mut cron_task = task("standup");
        cron_task.schedule = Some("0 9 * * *".into());
        cron_task.model_pin = Some("haiku-fast".into());
        cron_task.last_run_at = Some(chrono::Utc::now());

        let mut watchdog = task("watch");
        watchdog.prompt = String::new();
        watchdog.script = Some("curl -sf http://x/health".into());
        watchdog.schedule = Some("*/5 * * * *".into());

        let mut off = task("paused");
        off.enabled = false;
        off.interval_secs = 86_400;

        let rows = task_rows(&[cron_task, watchdog, off], now);

        // Header + one row per task, columns aligned.
        assert!(rows[0].contains("next fire"));
        assert_eq!(rows.len(), 4);

        let standup = &rows[1];
        assert!(standup.contains("standup"));
        assert!(standup.contains("prompt"));
        assert!(standup.contains("0 9 * * *"));
        assert!(standup.contains("✓"));
        assert!(standup.contains("haiku-fast"));
        assert!(!standup.contains("never"), "last run is stamped: {standup}");
        assert!(
            standup.contains("08-25 09:00"),
            "cron next fire preview from the given now: {standup}"
        );

        assert!(rows[2].contains("script"), "script task kind: {}", rows[2]);
        assert!(rows[2].contains("*/5 * * * *"));

        let paused = &rows[3];
        assert!(paused.contains("✗"), "disabled mark: {paused}");
        assert!(paused.contains("1d"), "interval fallback: {paused}");
        assert!(
            paused.trim_end().ends_with('-'),
            "no next fire for disabled/interval tasks: {paused}"
        );
    }

    #[test]
    fn fmt_interval_humanizes_common_steps() {
        assert_eq!(fmt_interval(45), "45s");
        assert_eq!(fmt_interval(300), "5m");
        assert_eq!(fmt_interval(3600), "1h");
        assert_eq!(fmt_interval(7200), "2h");
        assert_eq!(fmt_interval(86_400), "1d");
        assert_eq!(fmt_interval(0), "0s");
    }

    #[test]
    fn memory_ids_resolve_by_full_or_unique_prefix_only() {
        let a = note("aaaaaaaa11111111", "alpha");
        let b = note("bbbbbbbb22222222", "beta");
        let c = note(
            "aaaaaaaac3333333",
            "gamma shares an 8-char prefix with alpha",
        );
        let notes = vec![a, b.clone(), c.clone()];

        // Full exact id always wins.
        assert_eq!(find_note(&notes, "bbbbbbbb22222222").unwrap().id, b.id);

        // Unique prefix resolves to the full stored id.
        assert_eq!(find_note(&notes, "bbbbbbbb").unwrap().id, b.id);
        assert_eq!(find_note(&notes, "aaaaaaaac3").unwrap().id, c.id);

        // Ambiguous prefix is refused, not silently resolved.
        assert_eq!(
            find_note(&notes, "aaaaaaaa"),
            Err(NoteMatchError::Ambiguous("aaaaaaaa".into()))
        );
        // Unknown prefix.
        assert_eq!(
            find_note(&notes, "deadbeef"),
            Err(NoteMatchError::NotFound("deadbeef".into()))
        );
    }

    #[test]
    fn memory_arg_grammar_splits_forget_amend_append() {
        use MemoryAction::{Amend, Append, Forget, List};
        assert!(matches!(MemoryAction::parse(None), List));
        assert!(matches!(MemoryAction::parse(Some("   ")), List));
        assert!(matches!(MemoryAction::parse(Some("ship it")), Append(t) if t == "ship it"));
        assert!(
            matches!(MemoryAction::parse(Some("forget abc12345")), Forget(id) if id == "abc12345")
        );
        assert!(matches!(
            MemoryAction::parse(Some("amend abc12345 new body text")),
            Amend(id, text) if id == "abc12345" && text == "new body text"
        ));
        // Missing amend body parses but core's amend_note rejects empty text.
        assert!(matches!(
            MemoryAction::parse(Some("amend abc12345")),
            Amend(id, text) if id == "abc12345" && text.is_empty()
        ));
    }

    #[test]
    fn budget_marker_matches_finops_thresholds() {
        // Below 80% of cap: silent.
        assert_eq!(budget_marker(7.9, 10.0), None);
        assert_eq!(budget_marker(0.0, 10.0), None);
        // ≥80% warns; ≥100% errors — mirroring AlertLevel.
        assert_eq!(budget_marker(8.0, 10.0).as_deref(), Some("⚠ $8.00/$10.00"));
        assert_eq!(budget_marker(9.99, 10.0).as_deref(), Some("⚠ $9.99/$10.00"));
        assert_eq!(
            budget_marker(10.0, 10.0).as_deref(),
            Some("✗ $10.00/$10.00")
        );
        assert_eq!(
            budget_marker(150.0, 10.0).as_deref(),
            Some("✗ $150.00/$10.00")
        );
        // No/negative cap never alerts (alert_level contract).
        assert_eq!(budget_marker(9.0, 0.0), None);
        assert_eq!(budget_marker(9.0, -1.0), None);
    }

    #[test]
    fn status_row_carries_budget_marker_when_armed() {
        let theme = Theme::from_name("dark");
        let state = RunState::Streaming;
        let plain = crate::markdown::strip_ansi(&status_ansi(
            &theme, &state, 10, 20, 5, 0, 128_000, 0, false, None,
        ));
        assert!(!plain.contains('$'), "silent under 80%: {plain}");

        let warned = crate::markdown::strip_ansi(&status_ansi(
            &theme,
            &state,
            10,
            20,
            5,
            0,
            128_000,
            0,
            false,
            budget_marker(9.0, 10.0),
        ));
        assert!(warned.contains("⚠ $9.00/$10.00"), "{warned}");

        let over = crate::markdown::strip_ansi(&status_ansi(
            &theme,
            &state,
            10,
            20,
            5,
            0,
            128_000,
            0,
            false,
            budget_marker(12.0, 10.0),
        ));
        assert!(over.contains("✗ $12.00/$10.00"), "{over}");
    }

    #[test]
    fn rel_time_buckets_are_compact() {
        let now = chrono::Utc::now();
        assert_eq!(rel_time(now), "now");
        assert_eq!(rel_time(now - chrono::Duration::minutes(5)), "5m ago");
        assert_eq!(rel_time(now - chrono::Duration::hours(3)), "3h ago");
        assert_eq!(rel_time(now - chrono::Duration::days(2)), "2d ago");
        // Future timestamps clamp instead of going negative.
        assert_eq!(rel_time(now + chrono::Duration::hours(1)), "now");
    }

    #[test]
    fn search_arg_extracts_all_flag_position_free() {
        assert_eq!(commands::search_arg(""), None);
        assert_eq!(commands::search_arg("   "), None);
        assert_eq!(
            commands::search_arg("--all"),
            None,
            "flag alone is not a query"
        );
        assert_eq!(
            commands::search_arg("deploy rollback"),
            Some(("deploy rollback".into(), false))
        );
        assert_eq!(
            commands::search_arg("deploy rollback --all"),
            Some(("deploy rollback".into(), true))
        );
        assert_eq!(
            commands::search_arg("--all deploy"),
            Some(("deploy".into(), true))
        );
    }
}
