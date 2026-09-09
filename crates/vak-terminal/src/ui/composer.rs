//! Pinned bottom command composer and floating Quick Action Palette overlay.

use ratatui::buffer::Buffer;
use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::Modifier;
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Clear, Paragraph, Widget};

use crate::repl::ReplComposer;
use crate::theme::{Symbols, Theme};

pub struct ComposerView<'a> {
    pub composer: &'a ReplComposer,
    pub theme: &'a Theme,
    pub connected: bool,
    pub error: Option<String>,
}

impl<'a> Widget for ComposerView<'a> {
    fn render(self, area: Rect, buf: &mut Buffer) {
        let chunks = Layout::default()
            .direction(Direction::Vertical)
            .constraints([Constraint::Length(3), Constraint::Length(1)])
            .split(area);

        // 1. Input Box
        let prompt_label = if self.connected {
            format!(" {} ", Symbols::PROMPT_CHEVRON)
        } else {
            format!(" {} ", Symbols::STATUS_ACTIVE)
        };
        let prompt_span = Span::styled(
            prompt_label,
            self.theme.style_accent().add_modifier(Modifier::BOLD),
        );

        let input_text = &self.composer.buffer;
        let input_span = if self.connected {
            Span::styled(input_text, self.theme.style_card())
        } else {
            Span::styled(
                input_text,
                self.theme.style_card().add_modifier(Modifier::DIM),
            )
        };
        let cursor_span = Span::styled("█", self.theme.style_accent());

        let mut line_spans = vec![prompt_span, input_span, cursor_span];
        if let Some(err) = &self.error {
            line_spans.push(Span::styled(
                format!("  [ERR: {err}]"),
                self.theme.style_danger(),
            ));
        }
        let line = Line::from(line_spans);

        let block = Block::default()
            .borders(Borders::ALL)
            .border_style(self.theme.style_border_focus())
            .title(" Command composer ")
            .title_style(self.theme.style_accent());

        let input_para = Paragraph::new(line).block(block);
        input_para.render(chunks[0], buf);

        // 2. Shortcut Hint Strip
        let shortcuts = [
            ("[Enter]", "Send"),
            ("[Shift+Enter]", "Line"),
            ("[/]", "Commands"),
            ("[Tab]", "Deck Focus"),
            ("[F2]", "Theme"),
            ("[Ctrl+D]", "Detach"),
        ];

        let mut hint_spans = Vec::new();
        hint_spans.push(Span::raw(" "));
        for (key, action) in shortcuts {
            hint_spans.push(Span::styled(
                key,
                self.theme.style_card().add_modifier(Modifier::BOLD),
            ));
            hint_spans.push(Span::styled(
                format!(" {action}  "),
                self.theme.style_card(),
            ));
        }

        let hint_para = Paragraph::new(Line::from(hint_spans));
        hint_para.render(chunks[1], buf);

        // 3. Quick Action Palette Overlay (if open)
        if self.composer.slash_palette_open {
            let matching = self.composer.matching_slash_commands();
            let pal_h = (matching.len() as u16 + 2).min(8);
            let pal_w = 48.min(area.width.saturating_sub(4));
            let pal_area = Rect {
                x: area.x + 4,
                y: chunks[0].y.saturating_sub(pal_h),
                width: pal_w,
                height: pal_h,
            };

            Clear.render(pal_area, buf);

            let mut pal_lines = Vec::new();
            for (idx, cmd) in matching.iter().enumerate() {
                let is_selected = idx == self.composer.selected_slash_cmd;
                let prefix = if is_selected { " > " } else { "   " };
                let style = if is_selected {
                    self.theme.style_tab_active()
                } else {
                    self.theme.style_card()
                };

                pal_lines.push(Line::from(vec![
                    Span::styled(format!("{prefix}{:<10}", cmd.name), style),
                    Span::styled(format!(" {}", cmd.description), self.theme.style_card()),
                ]));
            }

            let pal_block = Block::default()
                .borders(Borders::ALL)
                .border_style(self.theme.style_border_focus())
                .title(" Quick Action Palette (Enter/Tab) ")
                .title_style(self.theme.style_accent());

            let pal_para = Paragraph::new(pal_lines).block(pal_block);
            pal_para.render(pal_area, buf);
        }
    }
}
