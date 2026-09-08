//! Screen 4: Attention Inbox, Memory & Scheduled Automation Tasks.

use ratatui::buffer::Buffer;
use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::Modifier;
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Paragraph, Widget};

use crate::app::DeckFocus;
use crate::theme::{Symbols, Theme};

pub struct InboxView<'a> {
    pub theme: &'a Theme,
    pub deck_focus: DeckFocus,
    pub alert_acknowledged: bool,
    pub skill_promoted: bool,
    pub watchdog_restarted: bool,
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
        let alert_line = if self.alert_acknowledged {
            Line::from(vec![
                Span::styled(
                    "[17:42:01] ",
                    self.theme.style_card().add_modifier(Modifier::DIM),
                ),
                Span::styled(
                    "✓ [ACKNOWLEDGED] Budget Alert: ",
                    self.theme.style_ok().add_modifier(Modifier::BOLD),
                ),
                Span::styled("Threshold reviewed by operator. ", self.theme.style_card()),
                Span::styled(
                    "[ ACKNOWLEDGED ]",
                    self.theme.style_card().add_modifier(Modifier::DIM),
                ),
            ])
        } else {
            Line::from(vec![
                Span::styled(
                    "[17:42:01] ",
                    self.theme.style_card().add_modifier(Modifier::DIM),
                ),
                Span::styled(
                    "! [URGENT] Budget Alert: ",
                    self.theme.style_danger().add_modifier(Modifier::BOLD),
                ),
                Span::styled(
                    "Cost-limit threshold exceeded ($5.00 limit). ",
                    self.theme.style_card(),
                ),
                Span::styled("[ [a] Ack ]", self.theme.style_tab_active()),
            ])
        };

        let watchdog_line = if self.watchdog_restarted {
            Line::from(vec![
                Span::styled(
                    "[17:39:12] ",
                    self.theme.style_card().add_modifier(Modifier::DIM),
                ),
                Span::styled(
                    "✓ [RECOVERED] Watchdog Service: ",
                    self.theme.style_ok().add_modifier(Modifier::BOLD),
                ),
                Span::styled(
                    "Process stream_monitor online (PID 5104). ",
                    self.theme.style_card(),
                ),
                Span::styled(
                    "[ ONLINE ]",
                    self.theme.style_ok().add_modifier(Modifier::BOLD),
                ),
            ])
        } else {
            Line::from(vec![
                Span::styled(
                    "[17:39:12] ",
                    self.theme.style_card().add_modifier(Modifier::DIM),
                ),
                Span::styled(
                    "! [ERROR] Watchdog Failure: ",
                    self.theme.style_danger().add_modifier(Modifier::BOLD),
                ),
                Span::styled(
                    "Process monitor stream_monitor failed. ",
                    self.theme.style_card(),
                ),
                Span::styled("[ [r] Restart ] [ [x] Analyze ]", self.theme.style_info()),
            ])
        };

        let unread_count = if self.alert_acknowledged && self.watchdog_restarted {
            1
        } else if self.alert_acknowledged || self.watchdog_restarted {
            2
        } else {
            3
        };

        let inbox_lines = vec![
            alert_line,
            Line::from(vec![Span::raw("")]),
            Line::from(vec![
                Span::styled(
                    "[17:41:30] ",
                    self.theme.style_card().add_modifier(Modifier::DIM),
                ),
                Span::styled(
                    "! [PENDING] Approval Request: ",
                    self.theme.style_warn().add_modifier(Modifier::BOLD),
                ),
                Span::styled(
                    "deploy_script.sh waiting execution grant. ",
                    self.theme.style_card(),
                ),
                Span::styled(
                    "[ [y] Approve ] [ [d] Deny ]",
                    self.theme.style_card().add_modifier(Modifier::BOLD),
                ),
            ]),
            Line::from(vec![Span::raw("")]),
            watchdog_line,
        ];

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
                Constraint::Percentage(40), // Memory store
                Constraint::Percentage(30), // Skill proposals
                Constraint::Percentage(30), // Scheduled tasks
            ])
            .split(deck_chunks[1]);

        // Memory Store
        let mem_lines = vec![
            Line::from(vec![Span::styled(
                "USER.md Profile Notes:",
                self.theme.style_card().add_modifier(Modifier::BOLD),
            )]),
            Line::from(vec![
                Span::styled(
                    format!("  {} Coding style: ", Symbols::BULLET),
                    self.theme.style_accent(),
                ),
                Span::styled(
                    "TypeScript ES6+, 2-space indentation.",
                    self.theme.style_card(),
                ),
            ]),
            Line::from(vec![
                Span::styled(
                    format!("  {} Verification: ", Symbols::BULLET),
                    self.theme.style_accent(),
                ),
                Span::styled(
                    "Prioritize local hermetic vitest runs.",
                    self.theme.style_card(),
                ),
            ]),
        ];

        let mem_block = Block::default()
            .borders(Borders::ALL)
            .border_style(right_border)
            .title(format!(" {} Durable Memory Store ", Symbols::BULLET))
            .title_style(self.theme.style_card());

        Paragraph::new(mem_lines)
            .block(mem_block)
            .render(right_chunks[0], buf);

        // Skill Proposals Queue
        let prop_action = if self.skill_promoted {
            Line::from(vec![Span::styled(
                "  [ ✓ PROMOTED TO PIPELINE ]",
                self.theme.style_ok().add_modifier(Modifier::BOLD),
            )])
        } else {
            Line::from(vec![
                Span::styled("  [ [p] Promote ] ", self.theme.style_ok()),
                Span::styled("  [ [r] Reject ]", self.theme.style_danger()),
            ])
        };

        let prop_lines = vec![
            Line::from(vec![Span::styled(
                "Prop 1: git_diff_viewer plugin v0.9",
                self.theme.style_card().add_modifier(Modifier::BOLD),
            )]),
            prop_action,
        ];

        let prop_block = Block::default()
            .borders(Borders::ALL)
            .border_style(right_border)
            .title(" Skill Proposals Queue ")
            .title_style(self.theme.style_card());

        Paragraph::new(prop_lines)
            .block(prop_block)
            .render(right_chunks[1], buf);

        // Scheduled Tasks & Automation
        let task_lines = vec![
            Line::from(vec![
                Span::styled(
                    format!("{} Task: agent_metrics_backup ", Symbols::STATUS_ACTIVE),
                    self.theme.style_ok(),
                ),
                Span::styled(
                    "(Next: 2h 14m)",
                    self.theme.style_card().add_modifier(Modifier::DIM),
                ),
            ]),
            Line::from(vec![
                Span::styled(
                    format!("{} Watchdog: check_api_endpoints ", Symbols::STATUS_ACTIVE),
                    self.theme.style_ok(),
                ),
                Span::styled(
                    "(Next: 4m 12s)",
                    self.theme.style_card().add_modifier(Modifier::DIM),
                ),
            ]),
        ];

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
