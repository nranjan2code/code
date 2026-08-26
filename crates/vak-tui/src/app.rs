//! Thin-client terminal UI: renders the base's SSE stream into native
//! scrollback. All agent state lives behind [`ClientData`]; this module owns
//! presentation state and key dispatch only.

use std::collections::VecDeque;
use std::path::PathBuf;
use std::sync::OnceLock;
use std::time::{Duration, Instant};

use crossterm::cursor::{Hide, Show};
use crossterm::event::{
    DisableBracketedPaste, EnableBracketedPaste, Event, EventStream, KeyCode, KeyEventKind,
    KeyModifiers,
};
use crossterm::execute;
use crossterm::terminal::{
    EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode, enable_raw_mode,
};
use futures::StreamExt;
use tokio::sync::mpsc;

use vak_client::types::{
    AgentEvent, ApprovalRequest as WireApproval, RunRequest, SearchHit, TurnOutcome, Usage,
};

use crate::commands::{self, Command};
use crate::complete;
use crate::data::ClientData;
use crate::editor::{ComposerMode, Editor, VimState};
use crate::events;
use crate::inbox::{self, InboxAck, InboxOutcome};
use crate::keymap::Keymap;
use crate::keys::Action;
use crate::markdown::LineStyler;
use crate::mentions;
use crate::modals;
use crate::palette::{ChoiceItem, ChoicePicker, CommandPalette, PaletteItem};
use crate::prefs::UiPrefs;
use crate::render::Screen;
use crate::state::{ModalView, PickerKind, RunState, ThinkingMode, UiState};
use crate::tasks::{self};
use crate::theme::{self, Theme};

pub struct UiConfig {
    pub cwd: PathBuf,
}

const INBOX_REFRESH_SECS: u64 = 60;
const FINOPS_REFRESH_SECS: u64 = 120;
const RUN_WATCHDOG_SECS: u64 = 600;

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

#[derive(Debug, Clone)]
struct ApprovalCard {
    id: String,
    tool: String,
    args_json: String,
    reason: String,
}

impl ApprovalCard {
    fn from_wire(w: &WireApproval) -> Self {
        Self {
            id: w.id.clone(),
            tool: w.tool.clone(),
            args_json: w.args_json.clone(),
            reason: w.reason.clone(),
        }
    }

    fn detail_modal(&self, diff: bool) -> ModalView {
        let mut rows = vec![format!("tool: {}", self.tool)];
        if !self.reason.is_empty() {
            rows.push(format!("reason: {}", self.reason));
        }
        rows.push(String::new());
        if diff {
            match events::approval_edit_diff(&self.args_json) {
                Some(d) => rows.extend(d.lines().map(String::from)),
                None => rows.push("(no file edit in this request)".into()),
            }
        } else {
            rows.extend(events::pretty_args_lines(&self.args_json));
        }
        ModalView {
            title: "approval · detail".into(),
            rows,
            footer: "Esc back".into(),
            ..ModalView::default()
        }
    }
}

enum Confirm {
    Forget { note_id: String },
    Ack(InboxAck),
}

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

fn keymap() -> &'static Keymap {
    static KM: OnceLock<Keymap> = OnceLock::new();
    KM.get_or_init(Keymap::default)
}

fn outcome_summary(out: &TurnOutcome) -> String {
    match out {
        TurnOutcome::Completed { .. } => "done".into(),
        TurnOutcome::Aborted => "stopped".into(),
        TurnOutcome::Failed { error } => error.clone(),
    }
}

struct App {
    data: ClientData,
    cwd: PathBuf,
    session_id: String,
    screen: Screen,
    theme_name: String,
    theme_origin: Option<String>,
    prefs: UiPrefs,
    editor: Editor,
    vim: VimState,
    ui: UiState,
    running: bool,
    run_started: Option<Instant>,
    follow_ups: VecDeque<String>,
    approvals: VecDeque<ApprovalCard>,
    approval_focused: bool,
    approval_diff: bool,
    confirm: Option<Confirm>,
    deferred_mode: Option<String>,
    goal: Option<(String, Vec<String>)>,
    attached_child: Option<String>,
    palette: Option<CommandPalette>,
    picker: Option<(PickerKind, ChoicePicker)>,
    modal: Option<ModalView>,
    hits: Vec<SearchHit>,
    hit_sel: usize,
    raw_keys: bool,
    inbox_unread: u32,
    day_usd: f64,
    last_inbox: Instant,
    last_finops: Instant,
    ev_tx: mpsc::UnboundedSender<AgentEvent>,
    done_tx: mpsc::Sender<TurnOutcome>,
    hist_path: PathBuf,
    draft_path: PathBuf,
}

impl App {
    fn target_session(&self) -> String {
        self.attached_child
            .clone()
            .unwrap_or_else(|| self.session_id.clone())
    }

    fn save_prefs(&mut self) {
        self.prefs.theme = Some(self.theme_name.clone());
        self.prefs.composer = match self.editor.mode() {
            ComposerMode::Vim => Some("vim".into()),
            ComposerMode::Emacs => Some("emacs".into()),
        };
        self.prefs.save_to_project(&self.cwd);
    }

    fn resolve_theme(&mut self, name: &str) {
        self.screen
            .set_theme(theme::resolve(name, &self.prefs.custom_themes));
    }

    fn redraw(&mut self) {
        let th = *self.screen.theme();
        if self.running {
            self.screen.redraw_running(
                &self.ui.run_state.label(),
                "",
                self.follow_ups.len(),
                self.approvals.len(),
            );
            return;
        }
        if let Some(card) = self.approvals.front() {
            draw_approval_card(&mut self.screen, &th, card);
            return;
        }
        if let Some(confirm) = &self.confirm {
            match confirm {
                Confirm::Forget { note_id } => {
                    draw_confirm_block(
                        &mut self.screen,
                        &th,
                        "forget this note?",
                        note_id,
                        "y forget · n/Esc keep",
                    );
                }
                Confirm::Ack(ack) => inbox::draw_ack_confirm(&mut self.screen, &th, ack),
            }
            return;
        }
        let inbox_marker = usize::from(self.inbox_unread > 0);
        let label = commands::composer_label(&self.ui, inbox_marker);
        let (buf, cursor) = self.editor.view();
        let footer = commands::composer_footer(&self.editor);
        self.screen.redraw_composer(&label, buf, cursor, &footer);
        let mut status = events::status_ansi(&self.ui, false, &th);
        if let Some(note) = self.data.version_note() {
            status.push_str(&format!(" \x1b[{}m· {note}\x1b[39m", theme::fg(th.dim)));
        }
        self.screen.redraw_status(&status);
        if let Some((kind, pk)) = self.picker.as_ref() {
            modals::draw_picker(&mut self.screen, *kind, pk);
        }
        if let Some(pal) = self.palette.as_ref() {
            let items = pal.items();
            let sel = pal.selected(items.len());
            self.screen.redraw_palette(pal.query(), &items, sel);
        }
        if let Some(m) = self.modal.as_ref() {
            self.screen
                .redraw_modal(&m.title, &m.rows, m.scroll, &m.footer, m.selected);
        }
    }

    async fn start_turn(&mut self, prompt: String) {
        let (goal, criteria) = match self.goal.take() {
            Some((objective, list)) => (Some(objective), Some(list)),
            None => (None, None),
        };
        let req = RunRequest {
            prompt,
            attachments: None,
            goal,
            criteria,
        };
        let sid = self.target_session();
        let data = self.data.clone();
        let ev_tx = self.ev_tx.clone();
        let done_tx = self.done_tx.clone();

        self.running = true;
        self.run_started = Some(Instant::now());
        self.ui.run_state = RunState::Thinking;

        tokio::spawn(async move {
            if let Err(e) = data.run_prompt(&sid, &req).await {
                let _ = done_tx
                    .send(TurnOutcome::Failed {
                        error: e.to_string(),
                    })
                    .await;
                return;
            }
            let mut finished: Option<TurnOutcome> = None;
            let mut stream = std::pin::pin!(data.events(&sid));
            while let Some(item) = stream.next().await {
                match item {
                    Ok(ev) => {
                        if let AgentEvent::RunFinished { summary, is_error } = &ev {
                            finished.get_or_insert(if *is_error {
                                TurnOutcome::Failed {
                                    error: summary.clone(),
                                }
                            } else {
                                TurnOutcome::Completed {
                                    usage: Usage::default(),
                                }
                            });
                        }
                        if ev_tx.send(ev).is_err() {
                            break;
                        }
                    }
                    Err(e) => {
                        finished.get_or_insert_with(|| TurnOutcome::Failed {
                            error: e.to_string(),
                        });
                        break;
                    }
                }
            }
            let outcome = finished.unwrap_or(TurnOutcome::Completed {
                usage: Usage::default(),
            });
            let _ = done_tx.send(outcome).await;
        });
    }

    async fn submit_input(&mut self, raw: &str) {
        let text = raw.trim();
        if text.starts_with('/') && text.len() > 1 {
            if commands::parse(text).is_some() {
                let line = text.to_string();
                self.dispatch_line(&line).await;
                return;
            }
            match self.data.expand_custom(text) {
                Some(expanded) => {
                    self.screen.user_message(text, false);
                    self.editor.clear();
                    self.start_turn(expanded).await;
                }
                None => self.screen.dim(&format!("unknown command: {text}")),
            }
            return;
        }
        if let Some(cmd) = text.strip_prefix('!') {
            let th = *self.screen.theme();
            commands::run_local_shell(cmd.trim(), &mut self.screen, &th);
            return;
        }
        let expanded = mentions::expand(text, &self.cwd);
        if !expanded.missing.is_empty() {
            self.screen
                .error(&format!("missing @file: {}", expanded.missing.join(", ")));
            return;
        }
        self.screen.user_message(text, false);
        self.editor.clear();
        self.start_turn(expanded.prompt).await;
    }

    async fn dispatch_line(&mut self, line: &str) {
        let Some(cmd) = commands::parse(line) else {
            self.screen.error("unknown command");
            return;
        };
        self.handle_command(cmd).await;
    }

    async fn handle_command(&mut self, cmd: Command) {
        match cmd {
            Command::Help => {
                let rows = modals::help_modal_rows(&self.data);
                self.modal = Some(ModalView {
                    title: "help".into(),
                    rows,
                    footer: "Esc close".into(),
                    ..ModalView::default()
                });
            }
            Command::Exit => {}
            Command::Cost => {
                self.screen.dim(&format!(
                    "{} in · {} out · {}",
                    fmt_tokens(self.ui.total_in),
                    fmt_tokens(self.ui.total_out),
                    fmt_cost(self.ui.cost_usd),
                ));
            }
            Command::Context => self.show_context().await,
            Command::Sessions => commands::list_sessions(&self.data, &mut self.screen).await,
            Command::Resume(arg) => self.cmd_resume(arg).await,
            Command::Rewind(arg) => {
                commands::cmd_rewind(
                    &self.data,
                    arg.as_deref(),
                    &self.session_id,
                    &mut self.screen,
                )
                .await;
            }
            Command::Theme(arg) => self.cmd_theme(arg),
            Command::Transcript(arg) => self.cmd_transcript(arg).await,
            Command::View(path) => self.cmd_view(path).await,
            Command::Doctor => {
                self.modal = Some(commands::run_doctor(&self.data).await);
            }
            Command::Services(arg) => {
                self.modal = Some(commands::run_services(arg));
            }
            Command::Details => {
                self.ui.expanded_tools = !self.ui.expanded_tools;
                self.screen.dim(&format!(
                    "tool previews {}",
                    if self.ui.expanded_tools {
                        "expanded"
                    } else {
                        "collapsed"
                    }
                ));
            }
            Command::Keys(arg) => {
                if matches!(arg.as_deref(), Some("raw") | Some("capture")) {
                    self.raw_keys = true;
                    self.screen.dim("capturing next key…");
                } else {
                    self.open_text_modal("keys", commands::keys_text().lines());
                }
            }
            Command::Keymap => {
                self.open_keymap_modal(0);
            }
            Command::Model(arg) => self.cmd_model(arg).await,
            Command::Provider(arg) => self.cmd_provider(arg).await,
            Command::Key(arg) => self.cmd_key(arg).await,
            Command::Config => {
                let rows = commands::settings_rows(&self.data, &self.ui).await;
                self.modal = Some(ModalView {
                    title: "settings".into(),
                    rows,
                    footer: "Esc close".into(),
                    ..ModalView::default()
                });
            }
            Command::Features => {
                let rows = commands::feature_rows(&self.data).await;
                self.modal = Some(ModalView {
                    title: "features".into(),
                    rows,
                    footer: "Esc close".into(),
                    ..ModalView::default()
                });
            }
            Command::Clear => match self.data.create_session(None).await {
                Ok(id) => {
                    let msg = format!("new session {id}");
                    self.session_id = id;
                    self.attached_child = None;
                    self.screen.dim(&msg);
                }
                Err(e) => self.screen.error(&format!("cannot start session: {e}")),
            },
            Command::Composer(arg) => {
                let mode = match arg.as_deref() {
                    Some("vim") => Some(ComposerMode::Vim),
                    Some("emacs") => Some(ComposerMode::Emacs),
                    _ => None,
                };
                match mode {
                    Some(m) => {
                        self.editor.set_mode(m);
                        self.screen.dim(&format!(
                            "composer: {}",
                            commands::composer_mode_name(&self.editor)
                        ));
                        self.save_prefs();
                    }
                    None => self.screen.dim(&format!(
                        "composer: {} (/vim or /emacs to switch)",
                        commands::composer_mode_name(&self.editor)
                    )),
                }
            }
            Command::Subagents => {
                self.open_subagents().await;
            }
            Command::A11y(arg) => self.cmd_a11y(arg),
            Command::Copy => {
                if self.prefs.osc52 {
                    if self.ui.last_response.is_empty() {
                        self.screen.dim("nothing to copy yet");
                    } else {
                        self.screen.osc52_copy(&self.ui.last_response);
                        self.screen.dim("copied via OSC52");
                    }
                } else {
                    self.screen.dim("OSC52 disabled — /config to inspect");
                }
            }
            Command::Goal(arg) => self.cmd_goal(arg),
            Command::Mode(arg) => self.cmd_mode(arg).await,
            Command::Compact => self.cmd_compact().await,
            Command::Budget => self.cmd_budget().await,
            Command::Memory(arg) => self.cmd_memory(arg).await,
            Command::Search(arg) => self.cmd_search(arg).await,
            Command::Tasks(arg) => {
                tasks::handle_tasks(&self.data, arg.as_deref(), &mut self.screen).await;
            }
            Command::Proposals(arg) => {
                tasks::handle_proposals(&self.data, arg.as_deref(), &mut self.screen).await;
            }
            Command::Inbox(arg) => {
                let th = *self.screen.theme();
                let outcome =
                    inbox::handle_inbox(&self.data, arg.as_deref(), &th, &mut self.screen).await;
                match outcome {
                    InboxOutcome::Listed {
                        entries,
                        unread,
                        modal,
                    } => {
                        self.inbox_unread = unread;
                        self.hits.clear();
                        self.hit_sel = 0;
                        let _ = entries;
                        self.modal = Some(modal);
                    }
                    InboxOutcome::ConfirmAck { entry_id, preview } => {
                        self.confirm = Some(Confirm::Ack(InboxAck {
                            id: entry_id,
                            preview,
                        }));
                    }
                    InboxOutcome::Handled => {}
                }
            }
            Command::Mcp => self.cmd_mcp().await,
            Command::Sandbox(arg) => self.cmd_sandbox(arg).await,
        }
    }

    async fn show_context(&mut self) {
        let resp = self.data.transcript(&self.session_id).await;
        match resp {
            Ok(t) => {
                let window = self.data.config().await.context_window.unwrap_or(128_000) as u64;
                let used = t.usage.total_tokens();
                let pct = (used * 100).checked_div(window).unwrap_or(0);
                self.screen.dim(&format!(
                    "context ≈ {} / {} ({pct}%) · {} messages",
                    fmt_tokens(used),
                    fmt_tokens(window),
                    t.count,
                ));
            }
            Err(e) => self.screen.error(&format!("context unavailable: {e}")),
        }
    }

    async fn cmd_resume(&mut self, arg: Option<String>) {
        match commands::cmd_resume(&self.data, arg.as_deref()).await {
            Ok(Some(canonical)) => {
                self.session_id = canonical;
                self.attached_child = None;
                self.screen
                    .dim(&format!("resumed {}", short_hex(&self.session_id)));
            }
            Ok(None) => match commands::session_choices(&self.data).await {
                Ok(rows) => {
                    let items = rows
                        .into_iter()
                        .map(|r| ChoiceItem {
                            value: r,
                            description: String::new(),
                            active: false,
                        })
                        .collect();
                    self.picker = Some((PickerKind::Sessions, ChoicePicker::new(items, true)));
                }
                Err(e) => self.screen.error(&e),
            },
            Err(e) => self.screen.error(&e),
        }
    }

    fn cmd_theme(&mut self, arg: Option<String>) {
        match arg {
            None => {
                let current = self.theme_name.clone();
                let items = commands::theme_choices(&current, &self.prefs.custom_themes);
                self.theme_origin = Some(current);
                self.picker = Some((PickerKind::Theme, ChoicePicker::new(items, true)));
            }
            Some(name) => {
                self.apply_theme_choice(Some(name));
            }
        }
    }

    fn apply_theme_choice(&mut self, chosen: Option<String>) {
        self.theme_origin = None;
        let Some(name) = chosen else {
            return;
        };
        if name == self.theme_name {
            return;
        }
        self.resolve_theme(&name);
        self.theme_name = name;
        let title = format!("VakCoder · {}", self.ui.model);
        self.screen.set_title(&title);
        self.save_prefs();
    }

    async fn cmd_transcript(&mut self, arg: Option<String>) {
        match commands::transcript_modal(&self.data, &self.session_id, arg.as_deref()).await {
            Ok(m) => self.modal = Some(m),
            Err(e) => self.screen.error(&e),
        }
    }

    async fn cmd_view(&mut self, path: Option<String>) {
        let Some(rel) = path else {
            self.screen.error("usage: /view <path>");
            return;
        };
        match modals::view_file_modal(&self.data, &rel).await {
            Ok(m) => self.modal = Some(m),
            Err(e) => self.screen.error(&e),
        }
    }

    fn open_text_modal<'a>(&mut self, title: &str, lines: impl Iterator<Item = &'a str>) {
        self.modal = Some(ModalView {
            title: title.into(),
            rows: lines.map(String::from).collect(),
            footer: "Esc close".into(),
            ..ModalView::default()
        });
    }

    fn open_keymap_modal(&mut self, selected: usize) {
        let (modal, _meta) = modals::build_keymap_modal(keymap(), Some(selected));
        self.modal = Some(modal);
    }

    async fn cmd_model(&mut self, arg: Option<String>) {
        match arg {
            None => {
                let items =
                    commands::model_choices(&self.data, &self.ui.provider, &self.ui.model).await;
                self.picker = Some((PickerKind::Model, ChoicePicker::new(items, true)));
            }
            Some(model) => {
                self.set_model(model).await;
            }
        }
    }

    async fn set_model(&mut self, model: String) {
        match self
            .data
            .patch_config(&vak_client::types::PatchConfigRequest {
                model: Some(model.clone()),
                ..Default::default()
            })
            .await
        {
            Ok(()) => {
                self.ui.model = model.clone();
                self.screen.set_title(&format!("VakCoder · {model}"));
                self.screen.success(&format!("model → {model}"));
            }
            Err(e) => self.screen.error(&format!("model change failed: {e}")),
        }
    }

    async fn cmd_provider(&mut self, arg: Option<String>) {
        match arg {
            None => {
                let items = commands::provider_choices(&self.data, &self.ui.provider).await;
                self.picker = Some((PickerKind::Provider, ChoicePicker::new(items, true)));
            }
            Some(provider) => {
                if let Err(e) = self
                    .data
                    .patch_config(&vak_client::types::PatchConfigRequest {
                        provider: Some(provider.clone()),
                        ..Default::default()
                    })
                    .await
                {
                    self.screen.error(&format!("provider change failed: {e}"));
                    return;
                }
                self.ui.provider = provider.clone();
                match commands::default_model(&self.data, &provider).await {
                    Some(model) => self.set_model(model).await,
                    None => self.screen.success(&format!("provider → {provider}")),
                }
            }
        }
    }

    async fn cmd_key(&mut self, arg: Option<String>) {
        let Some(arg) = arg else {
            let providers = self.data.providers().await.unwrap_or_default();
            let mut rows = vec!["credential status:".to_string()];
            for p in providers {
                rows.push(format!(
                    "  {:<16} {}",
                    p.name,
                    commands::provider_status(&p.name, p.configured),
                ));
            }
            rows.push(String::new());
            rows.push("/key <provider> <SECRET>".into());
            rows.push("/key <provider> --remove".into());
            self.modal = Some(ModalView {
                title: "provider keys".into(),
                rows,
                footer: "Esc close".into(),
                ..ModalView::default()
            });
            return;
        };
        let mut parts = arg.split_whitespace();
        let Some(target) = parts.next() else {
            self.screen
                .error("usage: /key [provider [SECRET|--remove]]");
            return;
        };
        match parts.next() {
            Some(secret) if secret != "--remove" => {
                match commands::set_provider_key(&self.data, target, secret).await {
                    Ok(msg) => self.screen.success(&msg),
                    Err(e) => self.screen.error(&e),
                }
            }
            Some(_) | None => {
                let configured = self
                    .data
                    .providers()
                    .await
                    .unwrap_or_default()
                    .iter()
                    .any(|p| p.name == target && p.configured);
                if !configured {
                    self.screen.dim(&format!("{target}: no stored key"));
                    return;
                }
                match commands::remove_provider_key_cmd(&self.data, target).await {
                    Ok(msg) => self.screen.success(&msg),
                    Err(e) => self.screen.error(&e),
                }
            }
        }
    }

    async fn open_subagents(&mut self) {
        match self.data.subagents_list(&self.session_id).await {
            Ok(resp) if resp.subagents.is_empty() => {
                self.screen.dim("no running subagents");
            }
            Ok(resp) => {
                let items = resp
                    .subagents
                    .into_iter()
                    .map(|s| ChoiceItem {
                        value: format!("{} · {}", s.session_id, s.label),
                        description: String::from("attach — steers route to this subagent"),
                        active: false,
                    })
                    .collect();
                self.picker = Some((PickerKind::Subagents, ChoicePicker::new(items, false)));
            }
            Err(e) => self.screen.error(&format!("subagents unavailable: {e}")),
        }
    }

    fn cmd_a11y(&mut self, arg: Option<String>) {
        match commands::parse_a11y(arg.as_deref()) {
            Err(usage) => self.screen.error(&usage),
            Ok((feature, state)) => {
                let turn = |on: &mut bool| state.unwrap_or(!*on);
                match feature {
                    commands::A11yFeature::Plain => {
                        self.prefs.a11y_plain = turn(&mut self.prefs.a11y_plain)
                    }
                    commands::A11yFeature::Motion => {
                        self.prefs.a11y_motion = turn(&mut self.prefs.a11y_motion)
                    }
                    commands::A11yFeature::Reader => {
                        self.prefs.a11y_reader = turn(&mut self.prefs.a11y_reader)
                    }
                }
                self.screen
                    .set_accessibility(self.prefs.a11y_plain, self.prefs.a11y_reader);
                self.screen.dim(&format!(
                    "{feature:?} {}",
                    on_off(match feature {
                        commands::A11yFeature::Plain => self.prefs.a11y_plain,
                        commands::A11yFeature::Motion => self.prefs.a11y_motion,
                        commands::A11yFeature::Reader => self.prefs.a11y_reader,
                    }),
                ));
                self.save_prefs();
            }
        }
    }

    fn cmd_goal(&mut self, arg: Option<String>) {
        match arg.as_deref() {
            Some("off") => {
                if self.goal.take().is_some() {
                    self.screen.dim("goal disarmed");
                } else {
                    self.screen.dim("no goal was armed");
                }
            }
            None => match &self.goal {
                Some((objective, criteria)) => self.screen.dim(&format!(
                    "goal armed: {objective} ({} criteria)",
                    criteria.len()
                )),
                None => self.screen.dim("no goal armed"),
            },
            Some(spec) => {
                let (objective, criteria) = split_goal_spec(spec);
                if objective.is_empty() {
                    self.screen.error("usage: /goal <objective> [-- c1; c2]");
                    return;
                }
                self.goal = Some((objective.clone(), criteria));
                self.screen.accent(&format!("goal armed: {objective}"));
            }
        }
    }

    async fn cmd_mode(&mut self, arg: Option<String>) {
        let mode = match arg {
            None => {
                let cfg = self.data.config().await;
                let rows = commands::mode_modal_rows(&cfg.permission_mode);
                self.modal = Some(ModalView {
                    title: "permission mode".into(),
                    rows,
                    footer: "/mode read-only|workspace-write|full-access".into(),
                    ..ModalView::default()
                });
                return;
            }
            Some(raw) => raw.to_lowercase(),
        };
        if !matches!(
            mode.as_str(),
            "read-only" | "workspace-write" | "full-access"
        ) {
            self.screen
                .error("/mode takes read-only | workspace-write | full-access");
            return;
        }
        if self.running {
            self.deferred_mode = Some(mode);
            let _ = self.data.cancel_run(&self.target_session()).await;
            self.screen
                .warn("run interrupted; applying mode after it ends…");
            return;
        }
        match self.data.set_permission_mode(&mode).await {
            Ok(()) => self.screen.success(&format!("permission mode → {mode}")),
            Err(e) => self.screen.error(&format!("mode switch rejected: {e}")),
        }
    }

    async fn cmd_compact(&mut self) {
        if self.running {
            self.screen
                .error("a run is active — wait or interrupt first");
            return;
        }
        self.screen.dim("compacting context…");
        match self.data.compact(&self.session_id).await {
            Ok(v) if v.get("noop").is_some_and(|n| n.as_bool() == Some(true)) => {
                self.screen.dim("nothing to compact");
            }
            Ok(v) => {
                let before = v["before_tokens"].as_u64().unwrap_or(0);
                let after = v["after_tokens"].as_u64().unwrap_or(0);
                let summarized = v["summarized_messages"].as_u64().unwrap_or(0);
                self.screen.success(&format!(
                    "compacted: {} → {} tokens ({summarized} messages summarized)",
                    fmt_tokens(before),
                    fmt_tokens(after),
                ));
            }
            Err(e) => self.screen.error(&format!("compact failed: {e}")),
        }
    }

    async fn cmd_budget(&mut self) {
        let fin = match self.data.finops().await {
            Ok(f) => f,
            Err(e) => {
                self.screen.error(&format!("finops unavailable: {e}"));
                return;
            }
        };
        self.day_usd = fin.day_usd;
        let marker = events::budget_marker(fin.day_usd, fin.day_cap_usd);
        let mut rows = vec![
            format!("today      ${:.4} {}", fin.day_usd, marker),
            format!(
                "caps       run {} · day {}",
                money_cap(fin.run_cap_usd),
                money_cap(fin.day_cap_usd),
            ),
        ];
        if !fin.by_provider.is_empty() {
            rows.push(String::new());
            rows.push("by provider:".into());
            for e in fin.by_provider.iter().take(6) {
                rows.push(format!(
                    "  {:<14} ${:.4} ({} calls)",
                    e.name, e.usd, e.calls
                ));
            }
        }
        self.modal = Some(ModalView {
            title: "budget".into(),
            rows,
            footer: "Esc close".into(),
            ..ModalView::default()
        });
    }

    async fn cmd_memory(&mut self, arg: Option<String>) {
        match commands::handle_memory(&self.data, arg.as_deref(), &mut self.screen).await {
            commands::MemoryOutcome::Handled => {}
            commands::MemoryOutcome::Modal(m) => self.modal = Some(m),
            commands::MemoryOutcome::ConfirmForget {
                note_id,
                preview: _,
            } => {
                self.confirm = Some(Confirm::Forget { note_id });
            }
        }
    }

    async fn cmd_search(&mut self, arg: Option<(String, bool)>) {
        let Some((query, all)) = arg else {
            self.screen.error("usage: /search <query> [--all]");
            return;
        };
        match self
            .data
            .search(&query, 20, all, Some(&self.session_id))
            .await
        {
            Ok(resp) if resp.hits.is_empty() => self.screen.dim("no hits"),
            Ok(resp) => {
                let mut rows = Vec::with_capacity(resp.hits.len());
                for h in &resp.hits {
                    rows.push(format!(
                        "{} · {} · {}",
                        short_hex(&h.session_id),
                        h.role,
                        first_line(&h.snippet, 90),
                    ));
                }
                rows.insert(0, format!("search: {query}"));
                self.hits = resp.hits;
                self.hit_sel = 1;
                self.modal = Some(ModalView {
                    title: "search results".into(),
                    rows,
                    footer: "Enter/o view session · Esc close".into(),
                    selected: Some(1),
                    ..ModalView::default()
                });
            }
            Err(e) => self.screen.error(&format!("search failed: {e}")),
        }
    }

    async fn cmd_mcp(&mut self) {
        let cfg = self.data.config().await;
        let tools = self.data.tools().await.unwrap_or_default();
        let mut rows = vec![format!(
            "{} servers configured:",
            cfg.integrations.mcp_servers.len()
        )];
        if cfg.integrations.mcp_servers.is_empty() {
            rows.push("  (none — desktop manages the table)".to_string());
        } else {
            for s in &cfg.integrations.mcp_servers {
                rows.push(format!("  • {s}"));
            }
        }
        rows.push(String::new());
        let mcp_tools: Vec<&String> = tools.iter().filter(|t| t.starts_with("mcp(")).collect();
        rows.push(format!("{} discovered meta-tools:", mcp_tools.len()));
        for t in mcp_tools.iter().take(12) {
            rows.push(format!("  • {t}"));
        }
        self.modal = Some(ModalView {
            title: "mcp".into(),
            rows,
            footer: "manage servers via /config or the desktop".into(),
            ..ModalView::default()
        });
    }

    async fn cmd_sandbox(&mut self, arg: Option<String>) {
        let choice = arg.map(|a| a.to_lowercase());
        match choice.as_deref() {
            None => {}
            Some(raw @ ("os" | "docker" | "default")) => {
                let backend = if raw == "default" { None } else { Some(raw) };
                match self.data.set_sandbox_backend(backend).await {
                    Ok(()) => self.screen.success(&format!("sandbox → {raw}")),
                    Err(e) => self.screen.error(&format!("sandbox switch rejected: {e}")),
                }
            }
            Some(other) => {
                self.screen.error(&format!(
                    "unknown backend '{other}' — os | docker | default"
                ));
                return;
            }
        }
        match self.data.sandbox_info().await {
            Ok(v) => {
                let rows = commands::sandbox_rows(&v);
                self.modal = Some(ModalView {
                    title: "execution sandbox".into(),
                    rows,
                    footer: "/sandbox os|docker|default".into(),
                    ..ModalView::default()
                });
            }
            Err(e) => self.screen.error(&format!("sandbox info unavailable: {e}")),
        }
    }

    async fn on_key(&mut self, code: KeyCode, mods: KeyModifiers) -> bool {
        // Returns true when the loop should exit.
        if self.raw_keys {
            self.raw_keys = false;
            self.screen.dim(&format!("captured {code:?} {mods:?}"));
            return false;
        }

        if code == KeyCode::Char('d') && mods.contains(KeyModifiers::CONTROL) {
            return true;
        }

        if self.approval_focused {
            let front = self.approvals.front().cloned();
            match (code, front) {
                (_, None) => {
                    self.approval_focused = false;
                }
                (KeyCode::Char('y') | KeyCode::Char('Y'), Some(card)) => {
                    self.approvals.pop_front();
                    self.approval_focused = false;
                    self.approval_diff = false;
                    let sid = self.session_id.clone();
                    if let Err(e) = self.data.answer_approval(&sid, &card.id, true).await {
                        self.screen.error(&format!("approval failed: {e}"));
                    } else {
                        self.screen.dim(&format!("approved {}", card.tool));
                    }
                }
                (KeyCode::Char('n') | KeyCode::Char('N'), Some(card)) => {
                    self.approvals.pop_front();
                    self.approval_focused = false;
                    self.approval_diff = false;
                    let sid = self.session_id.clone();
                    if let Err(e) = self.data.answer_approval(&sid, &card.id, false).await {
                        self.screen.error(&format!("denial failed: {e}"));
                    } else {
                        self.screen.warn(&format!("denied {}", card.tool));
                    }
                }
                (KeyCode::Char('e') | KeyCode::Char('E'), Some(card)) => {
                    self.approval_diff = !self.approval_diff;
                    self.modal = Some(card.detail_modal(self.approval_diff));
                }
                _ => {}
            }
            return false;
        }

        if let Some(c) = self.confirm.as_ref() {
            match code {
                KeyCode::Char('y') | KeyCode::Char('Y') => match c {
                    Confirm::Forget { note_id } => {
                        let note_id = note_id.clone();
                        self.confirm = None;
                        commands::forget_confirmed(&self.data, &note_id, &mut self.screen).await;
                    }
                    Confirm::Ack(ack) => {
                        let ack = ack.clone();
                        self.confirm = None;
                        self.inbox_unread =
                            inbox::ack_confirmed(&self.data, &ack.id, &mut self.screen).await;
                    }
                },
                KeyCode::Char('n') | KeyCode::Char('N') | KeyCode::Esc => {
                    self.confirm = None;
                    self.screen.dim("cancelled");
                }
                _ => {}
            }
            return false;
        }

        if self.modal.is_some() {
            self.on_modal_key(code).await;
            return false;
        }

        if self.picker.is_some() {
            self.on_picker_key(code).await;
            return false;
        }

        if self.palette.is_some() {
            self.on_palette_key(code).await;
            return false;
        }

        if self.editor.search_active() {
            match code {
                KeyCode::Esc => self.editor.cancel_search(),
                KeyCode::Enter => self.editor.accept_search(),
                KeyCode::Backspace => self.editor.search_backspace(),
                KeyCode::Char(c) => {
                    self.editor.search_push(c);
                    self.editor.search_next();
                }
                _ => {}
            }
            return false;
        }

        if self.editor.mode() == ComposerMode::Vim
            && self.vim == VimState::Normal
            && handle_vim_normal(code, &mut self.editor, &mut self.vim)
        {
            return false;
        }

        self.on_action(code, mods).await
    }

    async fn on_modal_key(&mut self, code: KeyCode) {
        let viewing_search = self
            .modal
            .as_ref()
            .is_some_and(|m| m.title.contains("search"));
        if viewing_search && matches!(code, KeyCode::Enter | KeyCode::Char('o')) {
            let hit = self.hits.get(self.hit_sel.saturating_sub(1)).cloned();
            if let Some(hit) = hit {
                self.hits.clear();
                match crate::transcript::session_transcript_modal(
                    &self.data,
                    &hit.session_id,
                    &hit.entry_id,
                )
                .await
                {
                    Ok(m) => self.modal = Some(m),
                    Err(e) => {
                        self.modal = None;
                        self.screen.error(&e);
                    }
                }
                return;
            }
        }
        let mut close = false;
        if let Some(m) = self.modal.as_mut() {
            match code {
                KeyCode::Esc | KeyCode::Char('q') => close = true,
                KeyCode::Up | KeyCode::Char('k') => m.scroll = m.scroll.saturating_sub(1),
                KeyCode::Down | KeyCode::Char('j') => m.scroll += 1,
                KeyCode::PageUp => m.scroll = m.scroll.saturating_sub(10),
                KeyCode::PageDown => m.scroll += 10,
                KeyCode::Home | KeyCode::Char('g') => m.scroll = 0,
                KeyCode::Char('n') if viewing_search => self.step_hit(1),
                KeyCode::Char('p') if viewing_search => self.step_hit(-1),
                _ => {}
            }
        }
        if close {
            self.modal = None;
        }
    }

    fn step_hit(&mut self, delta: i32) {
        if self.hits.is_empty() {
            return;
        }
        let len = self.hits.len();
        let cur = self.hit_sel as i64 - 1 + delta as i64;
        let next = ((cur.rem_euclid(len as i64)) as usize) % len;
        self.hit_sel = next + 1;
    }

    async fn on_picker_key(&mut self, code: KeyCode) {
        let kind = self.picker.as_ref().map(|(k, _)| *k);
        let Some(kind) = kind else { return };
        if code == KeyCode::Esc {
            if kind == PickerKind::Theme {
                if let Some(origin) = self.theme_origin.take() {
                    self.resolve_theme(&origin);
                    self.theme_name = origin;
                }
            } else {
                self.theme_origin = None;
            }
            self.picker = None;
            return;
        }
        let Some((_, pk)) = self.picker.as_mut() else {
            return;
        };
        match code {
            KeyCode::Up => pk.up(),
            KeyCode::Down => pk.down(),
            KeyCode::Backspace => pk.backspace(),
            KeyCode::Enter => {
                let chosen = pk.value();
                let kind = self.picker.take().map(|(k, _)| k).unwrap_or(kind);
                self.theme_origin = None;
                self.apply_choice(kind, chosen).await;
            }
            KeyCode::Char(c) => {
                pk.push(c);
                if kind == PickerKind::Theme
                    && let Some(value) = pk.filtered().first().map(|c| c.value.clone())
                {
                    self.resolve_theme(&value);
                }
            }
            _ => {}
        }
    }

    async fn apply_choice(&mut self, kind: PickerKind, chosen: Option<String>) {
        match kind {
            PickerKind::Theme => self.apply_theme_choice(chosen),
            PickerKind::Provider => {
                if let Some(p) = chosen.filter(|v| !v.is_empty()) {
                    self.cmd_provider(Some(p)).await;
                }
            }
            PickerKind::Model => {
                if let Some(m) = chosen.filter(|v| !v.is_empty()) {
                    self.set_model(m).await;
                }
            }
            PickerKind::Subagents => {
                if let Some(child) = chosen.and_then(|v| v.split(" · ").next().map(String::from)) {
                    self.screen
                        .dim(&format!("attached — steers now route to {child}"));
                    self.attached_child = Some(child);
                }
            }
            PickerKind::Sessions => {
                let Some(row) = chosen else { return };
                let token = row
                    .split_whitespace()
                    .next()
                    .unwrap_or_default()
                    .to_string();
                self.cmd_resume(Some(token)).await;
            }
        }
    }

    async fn on_palette_key(&mut self, code: KeyCode) {
        let Some(pal) = self.palette.as_mut() else {
            return;
        };
        match code {
            KeyCode::Esc => self.palette = None,
            KeyCode::Up => pal.up(1),
            KeyCode::Down => pal.down(1),
            KeyCode::Backspace => pal.backspace(),
            KeyCode::Char(c) => pal.push(c),
            KeyCode::Enter => {
                let items = pal.items();
                let sel = pal.selected(items.len());
                self.palette = None;
                if let Some(item) = items.get(sel) {
                    let line = format!("/{}", item.name);
                    match self.data.expand_custom(&line) {
                        Some(expanded) => {
                            self.screen.user_message(&line, false);
                            self.editor.clear();
                            self.start_turn(expanded).await;
                        }
                        None => self.dispatch_line(&line).await,
                    }
                }
            }
            _ => {}
        }
    }

    async fn on_action(&mut self, code: KeyCode, mods: KeyModifiers) -> bool {
        let action = keymap().action_for(self.running, code, mods);
        match action {
            Action::Insert(c) => self.editor.insert(c),
            Action::Backspace => self.editor.backspace(),
            Action::Delete => self.editor.delete(),
            Action::Left => self.editor.left(),
            Action::Right => self.editor.right(),
            Action::WordLeft => self.editor.word_left(),
            Action::WordRight => self.editor.word_right(),
            Action::DeleteWordBack => self.editor.delete_word_back(),
            Action::DeleteWordForward => self.editor.delete_word_forward(),
            Action::DeleteToLineEnd => self.editor.delete_to_line_end(),
            Action::Undo => self.editor.undo(),
            Action::Redo => self.editor.redo(),
            Action::Home => self.editor.home(),
            Action::End => self.editor.end(),
            Action::ClearLine => self.editor.clear(),
            Action::ToggleThinking => self.ui.thinking_mode = self.ui.thinking_mode.next(),
            Action::ClearViewport => self.screen.clear_viewport(),
            Action::ExpandStash => {
                if let Some(n) = self.editor.expand_stash_at_cursor() {
                    self.screen.dim(&format!("expanded paste · {n} chars"));
                }
            }
            Action::HistoryPrev => self.editor.history_prev(),
            Action::HistoryNext => self.editor.history_next(),
            Action::HistorySearch => self.editor.begin_search(),
            Action::Complete => {
                let (text, _) = self.editor.view();
                let sugg = complete::complete(text, commands::COMMANDS, &self.cwd);
                if let Some(first) = sugg.first() {
                    self.editor.paste_str(&first.replace);
                }
            }
            Action::CommandPalette => {
                self.palette = Some(CommandPalette::new(self.palette_items()));
            }
            Action::OpenApproval => {
                if self.approvals.is_empty() {
                    self.screen.dim("no pending approvals");
                } else {
                    self.approval_focused = true;
                    self.approval_diff = false;
                }
            }
            Action::CopyResponse => {
                if self.prefs.osc52 {
                    if self.ui.last_response.is_empty() {
                        self.screen.dim("nothing to copy yet");
                    } else {
                        self.screen.osc52_copy(&self.ui.last_response);
                        self.screen.dim("copied via OSC52");
                    }
                } else {
                    self.screen
                        .dim("OSC52 copy is off (/copy toggles nothing here)");
                }
            }
            Action::ExternalEditor => self.external_editor(),
            Action::Subagents => self.open_subagents().await,
            Action::Interrupt => {
                if self.running {
                    let target = self.target_session();
                    if let Err(e) = self.data.cancel_run(&target).await {
                        self.screen.error(&format!("interrupt failed: {e}"));
                    } else {
                        self.screen.warn("interrupting…");
                    }
                } else {
                    self.editor.clear();
                }
            }
            Action::Queue => {
                let text = self.editor.take();
                let trimmed = text.trim().to_string();
                if !trimmed.is_empty() {
                    self.follow_ups.push_back(trimmed);
                }
            }
            Action::Submit => {
                let text = self.editor.take();
                if text.trim().is_empty() {
                    return false;
                }
                if self.running {
                    let trimmed = text.trim().to_string();
                    let attached = self.attached_child.clone();
                    let result = match attached.as_ref() {
                        Some(child) => {
                            self.data
                                .subagent_steer(&self.session_id, child, &trimmed)
                                .await
                        }
                        None => self.data.steer(&self.session_id, &trimmed).await,
                    };
                    match result {
                        Ok(()) => {
                            self.screen.user_message(&trimmed, true);
                            self.follow_ups.push_back(trimmed);
                        }
                        Err(e) => self.screen.error(&format!("steer failed: {e}")),
                    }
                } else {
                    self.submit_input(&text).await;
                }
            }
            Action::CancelOrClear => {
                if self.running {
                    let target = self.target_session();
                    let _ = self.data.cancel_run(&target).await;
                    self.screen.warn("stopping…");
                } else {
                    self.editor.clear();
                }
            }
            Action::Exit => return true,
            Action::Ignore => {}
        }
        false
    }

    fn palette_items(&self) -> Vec<PaletteItem> {
        let mut items: Vec<PaletteItem> = commands::COMMANDS
            .iter()
            .map(|(name, desc)| PaletteItem {
                name: (*name).to_string(),
                description: (*desc).to_string(),
            })
            .collect();
        for (cmd, template) in self.data.custom_commands() {
            let description = template.lines().next().unwrap_or_default().to_string();
            items.push(PaletteItem {
                name: cmd.trim_start_matches('/').to_string(),
                description,
            });
        }
        items
    }

    fn external_editor(&mut self) {
        let Some(editor_cmd) = std::env::var_os("EDITOR")
            .or_else(|| std::env::var_os("VISUAL"))
            .filter(|v| !v.is_empty())
        else {
            self.screen.error("$EDITOR not set");
            return;
        };
        let dir = std::env::temp_dir().join("vakcoder-edit.md");
        let (buf, _) = self.editor.view();
        if std::fs::write(&dir, buf).is_err() {
            self.screen.error("could not stage draft for $EDITOR");
            return;
        }
        let _ = execute!(std::io::stdout(), LeaveAlternateScreen, Show);
        let _ = disable_raw_mode();
        let status = std::process::Command::new(editor_cmd).arg(&dir).status();
        let _ = enable_raw_mode();
        let _ = execute!(
            std::io::stdout(),
            EnterAlternateScreen,
            EnableBracketedPaste,
            Hide
        );
        match status {
            Ok(s) if s.success() => {
                if let Ok(edited) = std::fs::read_to_string(&dir) {
                    self.editor.set_text(edited.trim_end());
                }
            }
            Ok(s) => self.screen.warn(&format!("editor exited with {s}")),
            Err(e) => self.screen.error(&format!("editor failed: {e}")),
        }
    }

    async fn drain_done(&mut self, outcome: Option<TurnOutcome>) {
        self.running = false;
        self.run_started = None;
        self.attached_child = None;
        let failed = matches!(&outcome, Some(TurnOutcome::Failed { .. }));
        let summary = outcome.as_ref().map(outcome_summary).unwrap_or_default();
        let th = *self.screen.theme();
        events::finish_outcome(failed, &summary, &mut self.ui, &mut self.screen, &th);
        if let Some(mode) = self.deferred_mode.take() {
            match self.data.set_permission_mode(&mode).await {
                Ok(()) => self.screen.success(&format!("permission mode → {mode}")),
                Err(e) => self.screen.error(&format!("deferred mode rejected: {e}")),
            }
        }
        if let Some(next) = self.follow_ups.pop_front() {
            self.screen.user_message(&next, false);
            self.start_turn(next).await;
            return;
        }
        self.ui.run_state = RunState::Thinking;
    }
}

impl std::fmt::Display for ThinkingMode {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.name())
    }
}

fn draw_approval_card(screen: &mut Screen, theme: &Theme, card: &ApprovalCard) {
    use crossterm::style::Color;
    let fg = |c: Color| format!("{}{{}}{}", theme::fg(c), theme::fg(Color::Reset));
    let warning = fg(theme.warning);
    let dim = fg(theme.dim);
    let heading = fg(theme.heading);
    let lines = vec![
        warning.replace("{}", &format!("╭─ approval · {}", card.tool)),
        dim.replace("{}", &format!("│ {}", card.reason)),
        dim.replace(
            "{}",
            &format!("│ args {}", events::summarize_args(&card.args_json)),
        ),
        heading.replace("{}", "│ Alt-A again after Esc · y approve"),
        dim.replace("{}", "╰─ n deny · e diff preview"),
    ];
    screen.redraw_block(&lines);
}

fn draw_confirm_block(screen: &mut Screen, theme: &Theme, prompt: &str, subject: &str, hint: &str) {
    use crossterm::style::Color;
    let fg = |c: Color| format!("{}{{}}{}", theme::fg(c), theme::fg(Color::Reset));
    let warning = fg(theme.warning);
    let dim = fg(theme.dim);
    let lines = vec![
        warning.replace("{}", &format!("╭─ confirm · {prompt}")),
        dim.replace("{}", &format!("│ {subject}")),
        dim.replace("{}", &format!("╰─ {hint}")),
    ];
    screen.redraw_block(&lines);
}

fn on_off(on: bool) -> &'static str {
    if on { "on" } else { "off" }
}

/// Vim normal-mode editing over the shared Editor. Returns true when the
/// key was consumed.
fn handle_vim_normal(code: KeyCode, editor: &mut Editor, vim: &mut VimState) -> bool {
    let (text, cursor) = editor.view();
    let (text, cursor) = (text.to_string(), cursor);
    let line_start = text[..cursor].rfind('\n').map(|i| i + 1).unwrap_or(0);
    let line_end = text[cursor..]
        .find('\n')
        .map(|i| cursor + i)
        .unwrap_or(text.len());
    match code {
        KeyCode::Char('h') => editor.left(),
        KeyCode::Char('j') => {
            let next_nl = text[cursor..].find('\n').map(|i| cursor + i);
            if let Some(nl) = next_nl {
                let target = (nl + 1 + (cursor - line_start)).min(
                    text[nl + 1..]
                        .find('\n')
                        .map(|i| nl + 1 + i)
                        .unwrap_or(text.len()),
                );
                for _ in 0..(target - cursor) {
                    editor.right();
                }
            }
        }
        KeyCode::Char('k') => {
            if line_start > 0 {
                let prev_start = text[..line_start - 1]
                    .rfind('\n')
                    .map(|i| i + 1)
                    .unwrap_or(0);
                let target = (prev_start + (cursor - line_start)).min(line_start - 1);
                for _ in 0..(cursor - target) {
                    editor.left();
                }
            }
        }
        KeyCode::Char('l') => editor.right(),
        KeyCode::Char('0') => {
            while editor.view().1 > line_start {
                editor.left();
            }
        }
        KeyCode::Char('$') => {
            while editor.view().1 < line_end {
                editor.right();
            }
        }
        KeyCode::Char('x') => editor.delete(),
        KeyCode::Char('D') => editor.delete_to_line_end(),
        KeyCode::Char('u') => editor.undo(),
        KeyCode::Char('r') if true => {}
        KeyCode::Char('i') => *vim = VimState::Insert,
        KeyCode::Char('a') => {
            editor.right();
            *vim = VimState::Insert;
        }
        KeyCode::Char('A') => {
            while editor.view().1 < line_end {
                editor.right();
            }
            *vim = VimState::Insert;
        }
        KeyCode::Char('I') => {
            while editor.view().1 > line_start {
                editor.left();
            }
            *vim = VimState::Insert;
        }
        KeyCode::Char('o') | KeyCode::Char('O') => {
            editor.set_text(&format!("{text}\n"));
            while editor.view().1 < text.chars().count() + 1 {
                editor.right();
                if editor.view().1 == 0 {
                    break;
                }
            }
            *vim = VimState::Insert;
        }
        KeyCode::Char('d') | KeyCode::Char('y') => {
            // dd / yy: act on the current line.
            let mut out = String::new();
            let start = line_start;
            let end = if line_end < text.len() {
                line_end + 1
            } else {
                line_end
            };
            if code == KeyCode::Char('d') {
                out.push_str(&text[..start]);
                out.push_str(&text[end..]);
                editor.set_text(&out);
                for _ in 0..(cursor.min(out.len())) {
                    editor.right();
                }
                let cur = editor.view().1;
                for _ in 0..cur {
                    editor.left();
                }
            }
        }
        _ => return false,
    }
    true
}

fn short_hex(id: &str) -> &str {
    let len = id.chars().count().min(8);
    &id[..len]
}

fn first_line(s: &str, width: usize) -> String {
    let line = s.lines().next().unwrap_or_default();
    line.chars().take(width).collect()
}

fn fmt_tokens(n: u64) -> String {
    if n >= 1_000_000 {
        format!("{:.1}M", n as f64 / 1_000_000.0)
    } else if n >= 1_000 {
        format!("{:.1}k", n as f64 / 1_000.0)
    } else {
        n.to_string()
    }
}

fn fmt_cost(usd: f64) -> String {
    if usd >= 1.0 {
        format!("${usd:.2}")
    } else {
        format!("${usd:.4}")
    }
}

fn money_cap(cap: Option<f64>) -> String {
    match cap {
        Some(c) => format!("${c:.2}"),
        None => "off".into(),
    }
}

pub async fn run(data: ClientData, cfg: UiConfig) -> i32 {
    let prefs = UiPrefs::load_from_project(&cfg.cwd);

    let _raw = RawMode::enable();
    let initial_theme = prefs.theme.clone().unwrap_or_else(|| "dark".into());
    let mut screen = Screen::new(theme::resolve(&initial_theme, &prefs.custom_themes));
    screen.clear_viewport();
    screen.set_accessibility(prefs.a11y_plain, prefs.a11y_reader);

    let cache_dir = vak_config::paths::cache_home();
    let hist_path = cache_dir.join("input_history.txt");
    let draft_path = cache_dir.join("composer_draft.txt");
    let mut editor = Editor::new();
    if prefs.composer.as_deref() == Some("vim") {
        editor.set_mode(ComposerMode::Vim);
    }
    if editor.recover_draft(&draft_path) {
        screen.dim("[recovered an unsent draft from a previous session · Enter sends it]");
    }
    editor.load_history(&hist_path);

    let config = data.config().await;
    let session_id = match data.create_session(None).await {
        Ok(id) => id,
        Err(e) => {
            eprintln!("error: cannot start a session on the base — {e}");
            return 2;
        }
    };

    screen.set_title(&format!("VakCoder · {}", config.model));
    screen.banner(
        env!("CARGO_PKG_VERSION"),
        &config.provider,
        &config.model,
        &session_id,
    );

    let (ev_tx, mut ev_rx) = mpsc::unbounded_channel::<AgentEvent>();
    let (done_tx, mut done_rx) = mpsc::channel::<TurnOutcome>(1);

    data.refresh_custom_commands().await;
    let inbox_unread = data.inbox_unread_count().await.unwrap_or(0);
    let day_usd = data.finops().await.map(|f| f.day_usd).unwrap_or(0.0);

    let mut app = App {
        ui: UiState {
            styler: LineStyler::new(),
            model: config.model.clone(),
            provider: config.provider.clone(),
            thinking_mode: ThinkingMode::Indicator,
            ..UiState::default()
        },
        theme_name: initial_theme,
        theme_origin: None,
        prefs,
        editor,
        vim: VimState::Normal,
        running: false,
        run_started: None,
        follow_ups: VecDeque::new(),
        approvals: VecDeque::new(),
        approval_focused: false,
        approval_diff: false,
        confirm: None,
        deferred_mode: None,
        goal: None,
        attached_child: None,
        palette: None,
        picker: None,
        modal: None,
        hits: Vec::new(),
        hit_sel: 0,
        raw_keys: false,
        inbox_unread,
        day_usd,
        last_inbox: Instant::now(),
        last_finops: Instant::now() - Duration::from_secs(FINOPS_REFRESH_SECS),
        ev_tx,
        done_tx,
        hist_path,
        draft_path,
        data,
        cwd: cfg.cwd.clone(),
        session_id,
        screen,
    };

    let mut reader: Option<EventStream> = Some(EventStream::new());
    let mut tick = tokio::time::interval(Duration::from_millis(400));
    tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    tick.reset();

    app.redraw();

    loop {
        tokio::select! {
            biased;

            maybe_ev = ev_rx.recv() => {
                match maybe_ev {
                    Some(AgentEvent::ApprovalRequested { id, tool, args_json, reason }) => {
                        let wire = WireApproval { id, tool, args_json, reason };
                        app.approvals.push_back(ApprovalCard::from_wire(&wire));
                        app.screen.bell();
                        app.screen.notify("approval requested — Alt-A to review");
                    }
                    Some(ev) => {
                        let th = *app.screen.theme();
                        events::render_event(&ev, &mut app.ui, &mut app.screen, &th);
                    }
                    None => {}
                }
            }

            outcome = done_rx.recv() => {
                app.drain_done(outcome).await;
            }

            key_ev = async {
                match reader.as_mut() {
                    Some(r) => r.next().await,
                    None => std::future::pending().await,
                }
            } => {
                match key_ev {
                    Some(Ok(Event::Resize(_, _))) => app.screen.resize(),
                    Some(Ok(Event::Key(key))) if key.kind == KeyEventKind::Press => {
                        if app.on_key(key.code, key.modifiers).await {
                            break;
                        }
                    }
                    Some(Ok(_)) => {}
                    Some(Err(_)) | None => {
                        reader = None;
                    }
                }
            }

            _ = tick.tick() => {
                if app.last_inbox.elapsed() >= Duration::from_secs(INBOX_REFRESH_SECS) {
                    app.last_inbox = Instant::now();
                    if let Ok(n) = app.data.inbox_unread_count().await {
                        app.inbox_unread = n;
                    }
                    app.data.refresh_custom_commands().await;
                }
                if app.last_finops.elapsed() >= Duration::from_secs(FINOPS_REFRESH_SECS) {
                    app.last_finops = Instant::now();
                    if let Ok(f) = app.data.finops().await {
                        app.day_usd = f.day_usd;
                    }
                }
                if app.running
                    && let Some(started) = app.run_started
                    && started.elapsed() > Duration::from_secs(RUN_WATCHDOG_SECS)
                {
                    app.run_started = Some(Instant::now());
                    app.screen.warn("long-running turn — Ctrl-C interrupts");
                }
                app.redraw();
            }
        }

        if reader.is_none() {
            break;
        }
    }

    app.editor.save_draft(&app.draft_path);
    app.editor.save_history(&app.hist_path);
    app.save_prefs();
    println!("bye");
    0
}
