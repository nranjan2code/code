//! Screen 4: Attention Inbox, Memory & Scheduled Automation Tasks.
//!
//! All data is real:
//! - Inbox entries derived from SSE `AgentEvent` (approvals, errors, budget alerts)
//! - Health warnings surfaced as inbox items
//! - Memory store shows real workspace path + health warnings
//! - Skill proposals and scheduled tasks derived from real session/health data

use ratatui::buffer::Buffer;
use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::Modifier;
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Paragraph, Widget};

use crate::api::HealthReport;
use crate::app::{DeckFocus, InboxEntry, InboxKind};
use crate::theme::{Symbols, Theme};

pub struct InboxView<'a> {
    pub theme: &'a Theme,
    pub deck_focus: DeckFocus,
    pub alert_acknowledged: bool,
    pub skill_promoted: bool,
    pub watchdog_restarted: bool,
    pub inbox_entries: &'a [InboxEntry],
    pub health: &'a HealthReport,
}

impl<'a> Widget for InboxView<'a> {
    fn render(self, area: Rect, buf: &mut Buffer) {
        let left_border = if self.deck_focus == DeckFocus::Left {
            self.theme.style_border_focus()
        } else {
            self.theme.style_border()
        };
        let right_border = if self.deck_focus == DeckFocus::Right {
            self.theme.style_border_focus()
        } else {
            self.theme.style_border()
        };

        let deck_chunks = Layout::default()
            .direction(Direction::Horizontal)
            .constraints([Constraint::Percentage(60), Constraint::Percentage(40)])
            .split(area);

        // -------------------------------------------------------------
        // LEFT DECK: Prioritized Attention Inbox
        // -------------------------------------------------------------
        let inbox_lines: Vec<Line> = if self.inbox_entries.is_empty() {
            vec![Line::from(vec![Span::styled(
                "Inbox is clear — awaiting real events…",
                self.theme.style_card().add_modifier(Modifier::DIM),
            )])]
        } else {
            self.inbox_entries
                .iter()
                .rev()
                .take(5)
                .map(|entry| {
                    let (icon, style) = self.icon_for_kind(&entry.kind);
                    let action_hint = match &entry.actionable {
                        Some(_) => " [ [a] Ack ] [ [p] Promote ] [ [r] Restart ]".to_string(),
                        None => String::new(),
                    };
                    Line::from(vec![
                        Span::styled(
                            format!("[{}] ", entry.timestamp),
                            self.theme.style_card().add_modifier(Modifier::DIM),
                        ),
                        Span::styled(format!("{icon} [{:?}] ", entry.kind), style),
                        Span::styled(
                            &entry.title,
                            self.theme.style_card().add_modifier(Modifier::BOLD),
                        ),
                        Span::styled(&entry.body, self.theme.style_card()),
                        Span::styled(
                            action_hint,
                            self.theme.style_card().add_modifier(Modifier::DIM),
                        ),
                    ])
                })
                .collect()
        };

        let unread_count = self
            .inbox_entries
            .iter()
            .filter(|e| !matches!(e.kind, InboxKind::Heartbeat))
            .count();

        let inbox_block = Block::default()
            .borders(Borders::ALL)
            .border_style(left_border)
            .title(format!(
                " {} Attention Inbox ({} Unread) [Tab] ",
                Symbols::STATUS_ACTIVE,
                unread_count
            ))
            .title_style(self.theme.style_accent().add_modifier(Modifier::BOLD));

        Paragraph::new(inbox_lines)
            .block(inbox_block)
            .render(deck_chunks[0], buf);

        // -------------------------------------------------------------
        // RIGHT DECK: Durable Memory, Skill Proposals, Scheduled Tasks
        // -------------------------------------------------------------
        let right_chunks = Layout::default()
            .direction(Direction::Vertical)
            .constraints([
                Constraint::Percentage(40),
                Constraint::Percentage(30),
                Constraint::Percentage(30),
            ])
            .split(deck_chunks[1]);

        // Memory Store — real from health cwd + warnings
        let mem_lines: Vec<Line> = {
            let mut lines = vec![Line::from(vec![Span::styled(
                "Workspace Memory:",
                self.theme.style_card().add_modifier(Modifier::BOLD),
            )])];
            lines.push(Line::from(vec![
                Span::styled(
                    format!("  {} Path: ", Symbols::BULLET),
                    self.theme.style_accent(),
                ),
                Span::styled(&self.health.cwd, self.theme.style_card()),
            ]));
            if !self.health.warnings.is_empty() {
                for w in &self.health.warnings {
                    lines.push(Line::from(vec![
                        Span::styled(
                            format!("  {} Warning: ", Symbols::STATUS_ACTIVE),
                            self.theme.style_warn(),
                        ),
                        Span::styled(w, self.theme.style_card()),
                    ]));
                }
            } else {
                lines.push(Line::from(vec![Span::styled(
                    "  (no warnings)",
                    self.theme.style_card().add_modifier(Modifier::DIM),
                )]));
            }
            lines
        };

        let mem_block = Block::default()
            .borders(Borders::ALL)
            .border_style(right_border)
            .title(format!(" {} Durable Memory Store ", Symbols::BULLET))
            .title_style(self.theme.style_card());

        Paragraph::new(mem_lines)
            .block(mem_block)
            .render(right_chunks[0], buf);

        // Skill Proposals Queue — real from inbox entries
        let prop_entries: Vec<&InboxEntry> = self
            .inbox_entries
            .iter()
            .filter(|e| e.kind == InboxKind::SkillProposal)
            .collect();

        let prop_lines: Vec<Line> = if prop_entries.is_empty() {
            vec![Line::from(vec![Span::styled(
                "(no skill proposals pending)",
                self.theme.style_card().add_modifier(Modifier::DIM),
            )])]
        } else {
            let mut lines = Vec::new();
            for entry in prop_entries.iter().take(3) {
                lines.push(Line::from(vec![Span::styled(
                    entry.title.clone(),
                    self.theme.style_card().add_modifier(Modifier::BOLD),
                )]));
                lines.push(Line::from(vec![Span::styled(
                    if self.skill_promoted {
                        "  [ ✓ PROMOTED TO PIPELINE ]"
                    } else {
                        "  [ [p] Promote ]  [ [r] Reject ]"
                    },
                    if self.skill_promoted {
                        self.theme.style_ok().add_modifier(Modifier::BOLD)
                    } else {
                        self.theme.style_card().add_modifier(Modifier::DIM)
                    },
                )]));
            }
            lines
        };

        let prop_block = Block::default()
            .borders(Borders::ALL)
            .border_style(right_border)
            .title(" Skill Proposals Queue ")
            .title_style(self.theme.style_card());

        Paragraph::new(prop_lines)
            .block(prop_block)
            .render(right_chunks[1], buf);

        // Scheduled Tasks & Automation — real from inbox entries
        let task_entries: Vec<&InboxEntry> = self
            .inbox_entries
            .iter()
            .filter(|e| e.kind == InboxKind::ScheduledTask)
            .collect();

        let task_lines: Vec<Line> = if task_entries.is_empty() {
            vec![Line::from(vec![Span::styled(
                "(no scheduled tasks reported)",
                self.theme.style_card().add_modifier(Modifier::DIM),
            )])]
        } else {
            task_entries
                .iter()
                .take(4)
                .map(|entry| {
                    let status_icon = if entry.body.contains("Next:") {
                        Symbols::STATUS_ACTIVE
                    } else {
                        Symbols::CHECK
                    };
                    Line::from(vec![
                        Span::styled(
                            format!("{} {} ", status_icon, entry.title),
                            self.theme.style_ok(),
                        ),
                        Span::styled(
                            &entry.body,
                            self.theme.style_card().add_modifier(Modifier::DIM),
                        ),
                    ])
                })
                .collect()
        };

        let task_block = Block::default()
            .borders(Borders::ALL)
            .border_style(right_border)
            .title(" Scheduled Tasks & Automation ")
            .title_style(self.theme.style_card());

        Paragraph::new(task_lines)
            .block(task_block)
            .render(right_chunks[2], buf);
    }
}

impl<'a> InboxView<'a> {
    fn icon_for_kind(&self, kind: &InboxKind) -> (&'static str, ratatui::style::Style) {
        match kind {
            InboxKind::BudgetAlert => (
                Symbols::CROSS,
                self.theme.style_danger().add_modifier(Modifier::BOLD),
            ),
            InboxKind::ApprovalPending => (
                Symbols::STATUS_ACTIVE,
                self.theme.style_warn().add_modifier(Modifier::BOLD),
            ),
            InboxKind::ApprovalDenied => (
                Symbols::CROSS,
                self.theme.style_danger().add_modifier(Modifier::BOLD),
            ),
            InboxKind::WatchdogFailure => (
                Symbols::CROSS,
                self.theme.style_danger().add_modifier(Modifier::BOLD),
            ),
            InboxKind::SkillProposal => (
                Symbols::STATUS_ACTIVE,
                self.theme.style_info().add_modifier(Modifier::BOLD),
            ),
            InboxKind::ScheduledTask => (
                Symbols::STATUS_ACTIVE,
                self.theme.style_ok().add_modifier(Modifier::BOLD),
            ),
            InboxKind::Heartbeat => (
                Symbols::STATUS_IDLE,
                self.theme.style_card().add_modifier(Modifier::DIM),
            ),
        }
    }
}
