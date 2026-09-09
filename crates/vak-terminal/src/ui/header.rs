//! Top persistent navigation tab bar and session status pills.

use ratatui::buffer::Buffer;
use ratatui::layout::{Alignment, Rect};
use ratatui::style::Modifier;
use ratatui::text::{Line, Span};
use ratatui::widgets::{Paragraph, Widget};

use crate::theme::Theme;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ActiveTab {
    #[default]
    Studio = 1,
    Observability = 2,
    Admin = 3,
    Inbox = 4,
}

impl ActiveTab {
    pub fn next(&self) -> Self {
        match self {
            ActiveTab::Studio => ActiveTab::Observability,
            ActiveTab::Observability => ActiveTab::Admin,
            ActiveTab::Admin => ActiveTab::Inbox,
            ActiveTab::Inbox => ActiveTab::Studio,
        }
    }

    pub fn prev(&self) -> Self {
        match self {
            ActiveTab::Studio => ActiveTab::Inbox,
            ActiveTab::Observability => ActiveTab::Studio,
            ActiveTab::Admin => ActiveTab::Observability,
            ActiveTab::Inbox => ActiveTab::Admin,
        }
    }
}

pub struct HeaderView<'a> {
    pub active_tab: ActiveTab,
    pub session_name: &'a str,
    pub model_name: &'a str,
    pub host_endpoint: &'a str,
    pub spend_usd: f64,
    pub connected: bool,
    pub error: Option<String>,
    pub theme: &'a Theme,
}

impl<'a> Widget for HeaderView<'a> {
    fn render(self, area: Rect, buf: &mut Buffer) {
        let tabs = [
            (ActiveTab::Studio, "1: Studio"),
            (ActiveTab::Observability, "2: Observability & Ops"),
            (ActiveTab::Admin, "3: Settings & Remote Admin"),
            (ActiveTab::Inbox, "4: Inbox, Memory & Tasks"),
        ];

        let mut spans = Vec::new();
        spans.push(Span::raw(" "));

        // Connection status dot
        let conn_dot = if self.connected { "🟢 " } else { "🔴 " };
        spans.push(Span::raw(conn_dot));

        for (tab, label) in tabs {
            let is_active = self.active_tab == tab;
            let dot = if is_active { "● " } else { "○ " };
            let style = if is_active {
                self.theme.style_tab_active()
            } else {
                self.theme.style_tab_inactive()
            };

            spans.push(Span::styled(format!(" [ {dot}{label} ] "), style));
            spans.push(Span::raw(" "));
        }

        // Right-aligned telemetry badge — all real values from health.
        let right_text = if let Some(err) = &self.error {
            format!(
                "ERROR: {} • {} • {} • {} ",
                err, self.session_name, self.model_name, self.host_endpoint
            )
        } else {
            format!(
                "{} • {} • {} • ${:.3} ",
                self.session_name, self.model_name, self.host_endpoint, self.spend_usd
            )
        };
        let right_style = if self.connected {
            self.theme.style_card()
        } else {
            self.theme.style_card().add_modifier(Modifier::DIM)
        };
        let right_span = Span::styled(right_text, right_style.add_modifier(Modifier::DIM));

        let left_para = Paragraph::new(Line::from(spans)).alignment(Alignment::Left);
        let right_para = Paragraph::new(Line::from(vec![right_span])).alignment(Alignment::Right);

        left_para.render(area, buf);
        right_para.render(area, buf);
    }
}
