//! Application state machine, event loop coordination, and view dispatch.

use ratatui::buffer::Buffer;
use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::widgets::Widget;

use crate::hil::{HilApprovalState, HilOutcome};
use crate::repl::ReplComposer;
use crate::telemetry::TelemetryState;
use crate::theme::{Theme, ThemeKind};
use crate::ui::admin::AdminView;
use crate::ui::composer::ComposerView;
use crate::ui::header::{ActiveTab, HeaderView};
use crate::ui::hil_modal::HilModalView;
use crate::ui::inbox::InboxView;
use crate::ui::ops::OpsView;
use crate::ui::studio::StudioView;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum DeckFocus {
    #[default]
    Left,
    Right,
}

pub struct TerminalApp {
    pub active_tab: ActiveTab,
    pub deck_focus: DeckFocus,
    pub theme: Theme,
    pub telemetry: TelemetryState,
    pub composer: ReplComposer,
    pub hil_state: Option<HilApprovalState>,
    pub session_name: String,
    pub model_name: String,
    pub host_endpoint: String,
    pub selected_permission_mode: String,
    pub approval_mode: String,
    pub workspace_name: String,
    pub scroll_offset: usize,
    pub card_expanded: bool,
    pub alert_acknowledged: bool,
    pub skill_promoted: bool,
    pub watchdog_restarted: bool,
    pub pending_chat_status: Option<bool>,
    pub should_quit: bool,
    pub should_detach: bool,
}

impl TerminalApp {
    pub fn new(
        session_name: impl Into<String>,
        model_name: impl Into<String>,
        host_endpoint: impl Into<String>,
    ) -> Self {
        Self {
            active_tab: ActiveTab::Studio,
            deck_focus: DeckFocus::Left,
            theme: ThemeKind::default().theme(),
            telemetry: TelemetryState::new(),
            composer: ReplComposer::new(),
            hil_state: None,
            session_name: session_name.into(),
            model_name: model_name.into(),
            host_endpoint: host_endpoint.into(),
            selected_permission_mode: "WorkspaceWrite".into(),
            approval_mode: "Ask".into(),
            workspace_name: "vak".into(),
            scroll_offset: 0,
            card_expanded: false,
            alert_acknowledged: false,
            skill_promoted: false,
            watchdog_restarted: false,
            pending_chat_status: None,
            should_quit: false,
            should_detach: false,
        }
    }

    pub fn with_workspace(mut self, name: impl Into<String>) -> Self {
        self.workspace_name = name.into();
        self
    }

    /// Advance tick animations (50ms).
    pub fn on_tick(&mut self) {
        self.telemetry.tick();
    }

    /// Cycle to the next theme.
    pub fn cycle_theme(&mut self) {
        let next_kind = self.theme.kind.next();
        self.theme = next_kind.theme();
    }

    /// Handle keyboard input.
    pub fn handle_key(&mut self, key: crossterm::event::KeyEvent) {
        use crossterm::event::KeyCode;

        // 1. If HIL modal is open, it captures inputs
        if let Some(ref mut hil) = self.hil_state {
            if hil.is_editing {
                match key.code {
                    KeyCode::Enter => {
                        hil.is_editing = false;
                    }
                    KeyCode::Esc => {
                        hil.cancel_editing();
                    }
                    KeyCode::Backspace => {
                        hil.delete_backspace();
                    }
                    KeyCode::Char(c) => {
                        hil.insert_char(c);
                    }
                    _ => {}
                }
                return;
            }

            match key.code {
                KeyCode::Char('y') | KeyCode::Enter => {
                    let _ = hil.finish_approval();
                    self.hil_state = None;
                }
                KeyCode::Char('a') => {
                    let _ = HilOutcome::AlwaysForSession;
                    self.hil_state = None;
                }
                KeyCode::Char('d') => {
                    let _ = HilOutcome::Deny;
                    self.hil_state = None;
                }
                KeyCode::Char('e') => {
                    hil.start_editing();
                }
                KeyCode::Esc => {
                    self.hil_state = None;
                }
                _ => {}
            }
            return;
        }

        // 2. Global Hotkeys
        if key
            .modifiers
            .contains(crossterm::event::KeyModifiers::CONTROL)
        {
            match key.code {
                KeyCode::Char('d') => {
                    self.should_detach = true;
                    return;
                }
                KeyCode::Char('c') => {
                    self.should_quit = true;
                    return;
                }
                KeyCode::Char('k') => {
                    self.composer.buffer = "/".into();
                    self.composer.cursor_pos = 1;
                    self.composer.slash_palette_open = true;
                    return;
                }
                _ => {}
            }
        }

        // F2 cycles theme
        if key.code == KeyCode::F(2) {
            self.cycle_theme();
            return;
        }

        // Tab switches deck focus or completes slash commands
        if key.code == KeyCode::Tab {
            if self.composer.slash_palette_open {
                self.composer.complete_selected_slash();
                return;
            }
            self.deck_focus = match self.deck_focus {
                DeckFocus::Left => DeckFocus::Right,
                DeckFocus::Right => DeckFocus::Left,
            };
            return;
        }

        if key.code == KeyCode::BackTab {
            self.deck_focus = match self.deck_focus {
                DeckFocus::Left => DeckFocus::Right,
                DeckFocus::Right => DeckFocus::Left,
            };
            return;
        }

        // Number keys 1-4 and deck context hotkeys when composer buffer is empty
        if self.composer.buffer.is_empty() {
            match key.code {
                KeyCode::Char('1') => {
                    self.active_tab = ActiveTab::Studio;
                    return;
                }
                KeyCode::Char('2') => {
                    self.active_tab = ActiveTab::Observability;
                    return;
                }
                KeyCode::Char('3') => {
                    self.active_tab = ActiveTab::Admin;
                    return;
                }
                KeyCode::Char('4') => {
                    self.active_tab = ActiveTab::Inbox;
                    return;
                }
                KeyCode::Char(' ') => {
                    self.card_expanded = !self.card_expanded;
                    return;
                }
                KeyCode::Char('o') if self.active_tab == ActiveTab::Studio => {
                    let _ = std::process::Command::new("open")
                        .arg("http://localhost:5173")
                        .spawn();
                    return;
                }
                KeyCode::Char('m') if self.active_tab == ActiveTab::Admin => {
                    self.selected_permission_mode = match self.selected_permission_mode.as_str() {
                        "WorkspaceWrite" => "ReadOnly".into(),
                        "ReadOnly" => "FullAccess".into(),
                        _ => "WorkspaceWrite".into(),
                    };
                    return;
                }
                KeyCode::Char('p') if self.active_tab == ActiveTab::Admin => {
                    self.approval_mode = if self.approval_mode == "Ask" {
                        "AutoApprove".into()
                    } else {
                        "Ask".into()
                    };
                    return;
                }
                KeyCode::Char('y') if self.active_tab == ActiveTab::Admin => {
                    self.pending_chat_status = Some(true);
                    return;
                }
                KeyCode::Char('d') if self.active_tab == ActiveTab::Admin => {
                    self.pending_chat_status = Some(false);
                    return;
                }
                KeyCode::Char('a') if self.active_tab == ActiveTab::Inbox => {
                    self.alert_acknowledged = true;
                    return;
                }
                KeyCode::Char('p') if self.active_tab == ActiveTab::Inbox => {
                    self.skill_promoted = true;
                    return;
                }
                KeyCode::Char('r') if self.active_tab == ActiveTab::Inbox => {
                    self.watchdog_restarted = true;
                    return;
                }
                _ => {}
            }
        }

        // 3. Quick Action Palette navigation
        if self.composer.slash_palette_open {
            match key.code {
                KeyCode::Down => {
                    self.composer.select_next_slash();
                    return;
                }
                KeyCode::Up => {
                    self.composer.select_prev_slash();
                    return;
                }
                KeyCode::Esc => {
                    self.composer.slash_palette_open = false;
                    return;
                }
                _ => {}
            }
        }

        // 4. Composer typing
        match key.code {
            KeyCode::Enter => {
                if let Some(cmd) = self.composer.submit() {
                    self.execute_user_command(&cmd);
                }
            }
            KeyCode::Char(c) => {
                self.composer.insert_char(c);
            }
            KeyCode::Backspace => {
                self.composer.delete_backspace();
            }
            KeyCode::Left => {
                self.composer.move_cursor_left();
            }
            KeyCode::Right => {
                self.composer.move_cursor_right();
            }
            KeyCode::Home => {
                self.composer.move_to_start();
            }
            KeyCode::End => {
                self.composer.move_to_end();
            }
            KeyCode::Up => {
                self.composer.history_up();
            }
            KeyCode::Down => {
                self.composer.history_down();
            }
            KeyCode::Esc => {
                self.composer.clear();
            }
            _ => {}
        }
    }

    fn execute_user_command(&mut self, cmd: &str) {
        if cmd == "/theme" {
            self.cycle_theme();
        } else if cmd == "/ops" {
            self.active_tab = ActiveTab::Observability;
        } else if cmd == "/admin" {
            self.active_tab = ActiveTab::Admin;
        } else if cmd == "/inbox" {
            self.active_tab = ActiveTab::Inbox;
        } else if cmd == "/diff" {
            self.active_tab = ActiveTab::Studio;
            self.deck_focus = DeckFocus::Right;
        } else if cmd == "/preview" {
            let _ = std::process::Command::new("open")
                .arg("http://localhost:5173")
                .spawn();
        } else if cmd == "/model" {
            self.model_name = if self.model_name == "claude-3-7-sonnet" {
                "claude-3-5-sonnet".into()
            } else if self.model_name == "claude-3-5-sonnet" {
                "ollama/llama3.3".into()
            } else {
                "claude-3-7-sonnet".into()
            };
        } else if cmd == "/hil" || cmd == "/gate" {
            self.hil_state = Some(HilApprovalState::new(
                "req-demo-1",
                "bash",
                "rm -rf dist/ && npm run build:prod",
                "High - Destructive file removal outside sandbox",
                "/Users/nisheethranjan/Projects/vakcoder",
                0.042,
            ));
        } else if cmd == "/clear" {
            self.composer.clear();
            self.scroll_offset = 0;
        } else if cmd == "/help" {
            self.scroll_offset = 0;
        } else if cmd == "/quit" || cmd == "/exit" {
            self.should_quit = true;
        } else if cmd == "/detach" {
            self.should_detach = true;
        }
    }

    /// Handle mouse clicks and scrolling.
    pub fn handle_mouse(&mut self, mouse: crossterm::event::MouseEvent) {
        use crossterm::event::{MouseButton, MouseEventKind};

        match mouse.kind {
            MouseEventKind::Down(MouseButton::Left) => {
                let x = mouse.column;
                let y = mouse.row;

                // 1. Top navigation tab bar (row 0)
                if y == 0 {
                    if (1..19).contains(&x) {
                        self.active_tab = ActiveTab::Studio;
                    } else if (19..49).contains(&x) {
                        self.active_tab = ActiveTab::Observability;
                    } else if (49..81).contains(&x) {
                        self.active_tab = ActiveTab::Admin;
                    } else if (81..115).contains(&x) {
                        self.active_tab = ActiveTab::Inbox;
                    }
                    return;
                }

                // 2. If HIL modal is active, check click on modal buttons
                if let Some(ref mut hil) = self.hil_state {
                    if (10..=16).contains(&y) {
                        if (15..=32).contains(&x) {
                            let _ = hil.finish_approval();
                            self.hil_state = None;
                            return;
                        } else if (33..=56).contains(&x) {
                            self.hil_state = None;
                            return;
                        } else if (57..=70).contains(&x) {
                            hil.start_editing();
                            return;
                        } else if (71..=85).contains(&x) {
                            self.hil_state = None;
                            return;
                        }
                    }
                    return;
                }

                // Clicking in main viewport adjusts deck focus
                if y > 0 && y < 26 {
                    self.deck_focus = if x < 60 {
                        DeckFocus::Left
                    } else {
                        DeckFocus::Right
                    };
                }

                // 3. Screen-specific interactive clicks
                match self.active_tab {
                    ActiveTab::Admin => {
                        // Clicking on Permission Modes
                        if x < 45 {
                            if (8..=10).contains(&y) {
                                self.selected_permission_mode = "WorkspaceWrite".into();
                            } else if (11..=13).contains(&y) {
                                self.selected_permission_mode = "ReadOnly".into();
                            } else if (14..=16).contains(&y) {
                                self.selected_permission_mode = "FullAccess".into();
                            }
                        }
                        // Clicking on Approval Policy
                        if (17..=19).contains(&y) {
                            if (15..=25).contains(&x) {
                                self.approval_mode = "Ask".into();
                            } else if (26..=45).contains(&x) {
                                self.approval_mode = "AutoApprove".into();
                            }
                        }
                        // Clicking on Pending Authorization Queue buttons
                        if x >= 55 && (16..=20).contains(&y) {
                            if (55..=75).contains(&x) {
                                self.pending_chat_status = Some(true);
                            } else if (76..=95).contains(&x) {
                                self.pending_chat_status = Some(false);
                            }
                        }
                    }
                    ActiveTab::Inbox => {
                        // Clicking on Ack button
                        if (1..=4).contains(&y) && (60..=75).contains(&x) {
                            self.alert_acknowledged = true;
                        }
                        // Clicking on Watchdog restart
                        if (5..=9).contains(&y) && (40..=65).contains(&x) {
                            self.watchdog_restarted = true;
                        }
                        // Clicking on Skill proposal promote
                        if (15..=20).contains(&y) && x >= 60 {
                            self.skill_promoted = true;
                        }
                    }
                    ActiveTab::Studio => {
                        // Clicking [PREVIEW in browser] button
                        if (8..=11).contains(&y) && (2..=32).contains(&x) {
                            let _ = std::process::Command::new("open")
                                .arg("http://localhost:5173")
                                .spawn();
                        }
                        // Clicking on Markdown Card
                        if (11..=15).contains(&y) && x < 60 {
                            self.card_expanded = !self.card_expanded;
                        }
                    }
                    ActiveTab::Observability => {}
                }
            }
            MouseEventKind::ScrollDown => {
                self.scroll_offset = self.scroll_offset.saturating_add(1);
            }
            MouseEventKind::ScrollUp => {
                self.scroll_offset = self.scroll_offset.saturating_sub(1);
            }
            _ => {}
        }
    }
}

impl Widget for &TerminalApp {
    fn render(self, area: Rect, buf: &mut Buffer) {
        let chunks = Layout::default()
            .direction(Direction::Vertical)
            .constraints([
                Constraint::Length(1), // Header tab bar
                Constraint::Min(10),   // Active deck view
                Constraint::Length(4), // Pinned bottom composer
            ])
            .split(area);

        // 1. Header View
        let header = HeaderView {
            active_tab: self.active_tab,
            session_name: &self.session_name,
            model_name: &self.model_name,
            host_endpoint: &self.host_endpoint,
            spend_usd: self.telemetry.spend_today_usd,
            theme: &self.theme,
        };
        header.render(chunks[0], buf);

        // 2. Active Screen View
        match self.active_tab {
            ActiveTab::Studio => {
                let view = StudioView {
                    telemetry: &self.telemetry,
                    theme: &self.theme,
                    is_running: false,
                    deck_focus: self.deck_focus,
                    card_expanded: self.card_expanded,
                };
                view.render(chunks[1], buf);
            }
            ActiveTab::Observability => {
                let view = OpsView {
                    telemetry: &self.telemetry,
                    theme: &self.theme,
                    deck_focus: self.deck_focus,
                };
                view.render(chunks[1], buf);
            }
            ActiveTab::Admin => {
                let view = AdminView {
                    theme: &self.theme,
                    selected_permission_mode: &self.selected_permission_mode,
                    approval_mode: &self.approval_mode,
                    workspace_name: &self.workspace_name,
                    deck_focus: self.deck_focus,
                    pending_chat_status: self.pending_chat_status,
                };
                view.render(chunks[1], buf);
            }
            ActiveTab::Inbox => {
                let view = InboxView {
                    theme: &self.theme,
                    deck_focus: self.deck_focus,
                    alert_acknowledged: self.alert_acknowledged,
                    skill_promoted: self.skill_promoted,
                    watchdog_restarted: self.watchdog_restarted,
                };
                view.render(chunks[1], buf);
            }
        }

        // 3. Pinned Bottom Composer
        let composer_view = ComposerView {
            composer: &self.composer,
            theme: &self.theme,
        };
        composer_view.render(chunks[2], buf);

        // 4. Floating HIL Approval Modal (if active)
        if let Some(ref hil) = self.hil_state {
            let modal_view = HilModalView {
                state: hil,
                theme: &self.theme,
            };
            modal_view.render(area, buf);
        }
    }
}
