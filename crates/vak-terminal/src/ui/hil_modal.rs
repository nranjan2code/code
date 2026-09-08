//! Floating Human-In-The-Loop (HIL) modal dialog with in-place command editor.

use ratatui::buffer::Buffer;
use ratatui::layout::{Alignment, Constraint, Direction, Layout, Rect};
use ratatui::style::Modifier;
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Clear, Paragraph, Widget};

use crate::hil::HilApprovalState;
use crate::theme::Theme;

pub struct HilModalView<'a> {
    pub state: &'a HilApprovalState,
    pub theme: &'a Theme,
}

impl<'a> Widget for HilModalView<'a> {
    fn render(self, area: Rect, buf: &mut Buffer) {
        let modal_w = 78.min(area.width.saturating_sub(4));
        let modal_h = 16.min(area.height.saturating_sub(2));

        let modal_area = Rect {
            x: area.x + (area.width.saturating_sub(modal_w)) / 2,
            y: area.y + (area.height.saturating_sub(modal_h)) / 2,
            width: modal_w,
            height: modal_h,
        };

        Clear.render(modal_area, buf);

        let block = Block::default()
            .borders(Borders::ALL)
            .border_style(self.theme.style_border_focus().add_modifier(Modifier::BOLD))
            .title(" ! PERMISSION APPROVAL REQUIRED ")
            .title_style(self.theme.style_accent().add_modifier(Modifier::BOLD))
            .title_alignment(Alignment::Center);

        let inner = block.inner(modal_area);
        block.render(modal_area, buf);

        let chunks = Layout::default()
            .direction(Direction::Vertical)
            .constraints([
                Constraint::Length(4), // Metadata fields
                Constraint::Length(5), // Command box (or in-place editor)
                Constraint::Min(3),    // Action key pills
            ])
            .split(inner);

        // 1. Metadata
        let meta_lines = vec![
            Line::from(vec![
                Span::styled(
                    "TOOL:      ",
                    self.theme.style_card().add_modifier(Modifier::BOLD),
                ),
                Span::styled(
                    &self.state.tool_name,
                    self.theme.style_accent().add_modifier(Modifier::BOLD),
                ),
                Span::styled(
                    "   RISK: ",
                    self.theme.style_card().add_modifier(Modifier::BOLD),
                ),
                Span::styled(&self.state.risk_reason, self.theme.style_warn()),
            ]),
            Line::from(vec![
                Span::styled(
                    "DIR:       ",
                    self.theme.style_card().add_modifier(Modifier::BOLD),
                ),
                Span::styled(&self.state.workspace_path, self.theme.style_card()),
                Span::styled(
                    format!("   COST: +${:.3} est.", self.state.cost_estimate_usd),
                    self.theme.style_card().add_modifier(Modifier::DIM),
                ),
            ]),
        ];
        Paragraph::new(meta_lines).render(chunks[0], buf);

        // 2. Command Box
        let cmd_text = if self.state.is_editing {
            &self.state.edited_command
        } else {
            &self.state.original_command
        };

        let cmd_title = if self.state.is_editing {
            " Edit Command In-Place [Enter to finish, Esc to cancel] "
        } else {
            " Proposed Shell Command "
        };

        let cmd_border_style = if self.state.is_editing {
            self.theme.style_border_focus()
        } else {
            self.theme.style_border()
        };

        let cmd_block = Block::default()
            .borders(Borders::ALL)
            .border_style(cmd_border_style)
            .title(cmd_title)
            .title_style(self.theme.style_card().add_modifier(Modifier::BOLD));

        let cmd_line = if self.state.is_editing {
            Line::from(vec![
                Span::styled(
                    cmd_text,
                    self.theme.style_card().add_modifier(Modifier::BOLD),
                ),
                Span::styled("█", self.theme.style_accent()),
            ])
        } else {
            Line::from(vec![Span::styled(cmd_text, self.theme.style_card())])
        };

        Paragraph::new(cmd_line)
            .block(cmd_block)
            .render(chunks[1], buf);

        // 3. Action Pills
        let action_spans = vec![
            Span::styled(
                " [y] Approve Once ",
                self.theme.style_ok().add_modifier(Modifier::BOLD),
            ),
            Span::raw("   "),
            Span::styled(" [a] Always for Session ", self.theme.style_accent()),
            Span::raw("   "),
            Span::styled(
                " [d] Deny ",
                self.theme.style_danger().add_modifier(Modifier::BOLD),
            ),
            Span::raw("   "),
            Span::styled(
                " [e] Edit In-Place ",
                self.theme.style_info().add_modifier(Modifier::BOLD),
            ),
            Span::raw("   "),
            Span::styled(
                " [Esc] Cancel ",
                self.theme.style_card().add_modifier(Modifier::DIM),
            ),
        ];

        Paragraph::new(Line::from(action_spans))
            .alignment(Alignment::Center)
            .render(chunks[2], buf);
    }
}
