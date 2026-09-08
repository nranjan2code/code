//! Screen 1: Agent Studio view (Dual-Deck: Stream & Web Preview + Telemetry HUD).

use ratatui::buffer::Buffer;
use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::Modifier;
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Paragraph, Widget};

use crate::app::DeckFocus;
use crate::graphics::HalfBlockImage;
use crate::telemetry::TelemetryState;
use crate::theme::{Symbols, Theme};

pub struct StudioView<'a> {
    pub telemetry: &'a TelemetryState,
    pub theme: &'a Theme,
    pub is_running: bool,
    pub deck_focus: DeckFocus,
    pub card_expanded: bool,
}

impl<'a> Widget for StudioView<'a> {
    fn render(self, area: Rect, buf: &mut Buffer) {
        // 60% Left Deck (Stream & Previews) / 40% Right Deck (Telemetry HUD)
        let main_decks = Layout::default()
            .direction(Direction::Horizontal)
            .constraints([Constraint::Percentage(60), Constraint::Percentage(40)])
            .split(area);

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

        // -------------------------------------------------------------
        // LEFT DECK: Stream, Previews & Tool Runs
        // -------------------------------------------------------------
        let doc_height = if self.card_expanded { 11 } else { 7 };
        let left_chunks = Layout::default()
            .direction(Direction::Vertical)
            .constraints([
                Constraint::Length(9),           // Web preview card
                Constraint::Length(doc_height),  // Markdown doc card (collapsible)
                Constraint::Min(6),              // Tool execution card
            ])
            .split(main_decks[0]);

        // 1. Live Webpage Preview Card
        let prev_block = Block::default()
            .borders(Borders::ALL)
            .border_style(left_border)
            .title(format!(" {} [PREVIEW] http://localhost:5173 {} ", Symbols::BULLET, Symbols::EXTERNAL_LINK))
            .title_style(self.theme.style_accent().add_modifier(Modifier::BOLD));

        let prev_inner = prev_block.inner(left_chunks[0]);
        prev_block.render(left_chunks[0], buf);

        if prev_inner.height >= 4 && prev_inner.width >= 10 {
            let img_area = Rect {
                x: prev_inner.x + 1,
                y: prev_inner.y,
                width: prev_inner.width.saturating_sub(2),
                height: prev_inner.height.saturating_sub(2),
            };
            let wireframe = HalfBlockImage::sample_dashboard_wireframe(img_area.width, img_area.height);
            wireframe.render(img_area, buf);

            // Action button pill
            let btn_area = Rect {
                x: prev_inner.x + 2,
                y: prev_inner.y + prev_inner.height.saturating_sub(1),
                width: 28.min(prev_inner.width.saturating_sub(4)),
                height: 1,
            };
            let btn_span = Span::styled(
                " [ [PREVIEW] in Browser (o) ] ",
                self.theme.style_tab_active(),
            );
            Paragraph::new(Line::from(vec![btn_span])).render(btn_area, buf);
        }

        // 2. Markdown Document Card
        let mut doc_lines = vec![
            Line::from(vec![
                Span::styled("```typescript ", self.theme.style_card()),
                Span::styled("react-app.ts", self.theme.style_accent()),
            ]),
            Line::from(vec![
                Span::styled("export const ", self.theme.style_info()),
                Span::styled("App = () => <TelemetryCanvas />;", self.theme.style_card()),
            ]),
            Line::from(vec![Span::styled("```", self.theme.style_card())]),
            Line::from(vec![
                Span::styled(format!(" {} ", Symbols::BULLET), self.theme.style_accent()),
                Span::styled("Compiled with 100% test coverage; 0 audit vulnerabilities.", self.theme.style_card()),
            ]),
        ];

        if self.card_expanded {
            doc_lines.push(Line::from(vec![
                Span::styled(format!(" {} ", Symbols::CHECK), self.theme.style_ok()),
                Span::styled("Hermetic sandbox execution verified (isolation: strict).", self.theme.style_ok()),
            ]));
            doc_lines.push(Line::from(vec![
                Span::styled(format!(" {} ", Symbols::BULLET), self.theme.style_accent()),
                Span::styled("Bundle footprint: 142 kB gzip (chunks dynamically split).", self.theme.style_card().add_modifier(Modifier::DIM)),
            ]));
        }

        let doc_caret = if self.card_expanded {
            Symbols::CARET_EXPANDED
        } else {
            Symbols::CARET_COLLAPSED
        };

        let doc_block = Block::default()
            .borders(Borders::ALL)
            .border_style(left_border)
            .title(format!(" {} Markdown Document Card [Space] ", doc_caret))
            .title_style(self.theme.style_card());

        Paragraph::new(doc_lines)
            .block(doc_block)
            .render(left_chunks[1], buf);

        // 3. Tool Execution Card
        let tool_lines = vec![
            Line::from(vec![
                Span::styled(format!(" {} ", Symbols::CHECK), self.theme.style_ok()),
                Span::styled("Installing dependencies (vite, react, @types/react)...", self.theme.style_card()),
            ]),
            Line::from(vec![
                Span::styled(format!(" {} ", Symbols::CHECK), self.theme.style_ok()),
                Span::styled("Running production build and bundle verification...", self.theme.style_card()),
            ]),
            Line::from(vec![
                Span::styled(format!(" {} ", Symbols::STATUS_ACTIVE), self.theme.style_ok()),
                Span::styled("Serving dist/ on http://localhost:5173", self.theme.style_card()),
            ]),
        ];

        let spinner = Symbols::SPINNER_FRAMES[self.telemetry.tick_count % Symbols::SPINNER_FRAMES.len()];
        let (title, title_style) = if self.is_running {
            (
                format!(" {} bash: npm run test ── [{} RUNNING 1.4s] ", Symbols::CARET_EXPANDED, spinner),
                self.theme.style_accent(),
            )
        } else {
            (
                format!(" {} Tool Execution: deploy_script.sh [OK 2.1s • RSS 42MB] ", Symbols::CARET_EXPANDED),
                self.theme.style_ok(),
            )
        };

        let tool_block = Block::default()
            .borders(Borders::ALL)
            .border_style(self.theme.style_border_focus())
            .title(title)
            .title_style(title_style);

        Paragraph::new(tool_lines)
            .block(tool_block)
            .render(left_chunks[2], buf);

        // -------------------------------------------------------------
        // RIGHT DECK: Telemetry HUD (Gauges, Radar, Waveform, Diffs)
        // -------------------------------------------------------------
        let hud_chunks = Layout::default()
            .direction(Direction::Vertical)
            .constraints([
                Constraint::Length(6), // Circular CPU & Memory Gauges + Radar
                Constraint::Length(4), // Token velocity wave
                Constraint::Min(8),    // Git diff inspector
            ])
            .split(main_decks[1]);

        // 1. System Metrics & Swarm Radar
        let gauge_chunks = Layout::default()
            .direction(Direction::Horizontal)
            .constraints([Constraint::Percentage(50), Constraint::Percentage(50)])
            .split(hud_chunks[0]);

        // CPU & RSS Gauges
        let gauge_lines = vec![
            Line::from(vec![
                Span::styled("CPU UTIL:  ", self.theme.style_card()),
                Span::styled(format!("{:.1}% ", self.telemetry.cpu_percent), self.theme.style_accent().add_modifier(Modifier::BOLD)),
                Span::styled("[■■■■□□□□]", self.theme.style_accent()),
            ]),
            Line::from(vec![
                Span::styled("MEMORY RSS:", self.theme.style_card()),
                Span::styled(format!("{:.1}MB ", self.telemetry.rss_mb), self.theme.style_ok().add_modifier(Modifier::BOLD)),
                Span::styled("[■■■■■■□□]", self.theme.style_ok()),
            ]),
            Line::from(vec![
                Span::styled("SANDBOX:   ", self.theme.style_card()),
                Span::styled("QUARANTINED (PID 4821)", self.theme.style_ok()),
            ]),
        ];

        let gauge_block = Block::default()
            .borders(Borders::ALL)
            .border_style(right_border)
            .title(" System Metrics [Tab] ")
            .title_style(self.theme.style_card());

        Paragraph::new(gauge_lines)
            .block(gauge_block)
            .render(gauge_chunks[0], buf);

        // Swarm Radar
        let radar_lines = vec![
            Line::from(vec![
                Span::styled(format!(" {} ACTIVE:  ", Symbols::STATUS_ACTIVE), self.theme.style_accent()),
                Span::styled(format!("{} agents", self.telemetry.active_subagents), self.theme.style_card().add_modifier(Modifier::BOLD)),
            ]),
            Line::from(vec![
                Span::styled(format!(" {} PENDING: ", Symbols::STATUS_IDLE), self.theme.style_card()),
                Span::styled(format!("{} tasks", self.telemetry.pending_tasks), self.theme.style_card()),
            ]),
            Line::from(vec![
                Span::styled(" RADAR:    ", self.theme.style_card()),
                Span::styled(format!("SWEEP {:.0}°", self.telemetry.radar_angle_deg), self.theme.style_ok()),
            ]),
        ];

        let radar_block = Block::default()
            .borders(Borders::ALL)
            .border_style(right_border)
            .title(" Agent Swarm Radar ")
            .title_style(self.theme.style_card());

        Paragraph::new(radar_lines)
            .block(radar_block)
            .render(gauge_chunks[1], buf);

        // 2. Token Velocity Waveform
        let mut wave_str = String::new();
        for val in &self.telemetry.token_rate_history {
            let symbol = if *val > 150 {
                "▲"
            } else if *val > 100 {
                "∿"
            } else {
                "~"
            };
            wave_str.push_str(symbol);
        }

        let wave_lines = vec![
            Line::from(vec![
                Span::styled("RATE: ", self.theme.style_card()),
                Span::styled("142 tok/s  ", self.theme.style_accent().add_modifier(Modifier::BOLD)),
                Span::styled(wave_str, self.theme.style_accent()),
            ]),
        ];

        let wave_block = Block::default()
            .borders(Borders::ALL)
            .border_style(right_border)
            .title(" Token Burn-Rate [1.2M/min] ")
            .title_style(self.theme.style_card());

        Paragraph::new(wave_lines)
            .block(wave_block)
            .render(hud_chunks[1], buf);

        // 3. Multi-File Git Diff Inspector
        let diff_lines = vec![
            Line::from(vec![
                Span::styled("src/index.js ", self.theme.style_card().add_modifier(Modifier::BOLD)),
                Span::styled("(+4 / -1)", self.theme.style_ok()),
            ]),
            Line::from(vec![
                Span::styled("  import React from 'react';", self.theme.style_card()),
            ]),
            Line::from(vec![
                Span::styled("+ export const status = 'production';", self.theme.style_diff_add()),
            ]),
            Line::from(vec![
                Span::styled("- const status = 'draft';", self.theme.style_diff_del()),
            ]),
            Line::from(vec![
                Span::styled("src/components/Button.js ", self.theme.style_card().add_modifier(Modifier::BOLD)),
                Span::styled("(+12 / -3)", self.theme.style_ok()),
            ]),
            Line::from(vec![
                Span::styled("+ export function Button({ label }) {", self.theme.style_diff_add()),
            ]),
            Line::from(vec![
                Span::styled("+   return <button className='btn'>{label}</button>;", self.theme.style_diff_add()),
            ]),
            Line::from(vec![
                Span::styled("+ }", self.theme.style_diff_add()),
            ]),
        ];

        let diff_block = Block::default()
            .borders(Borders::ALL)
            .border_style(right_border)
            .title(" Multi-File Git Diff Inspector [Ctrl+K] ")
            .title_style(self.theme.style_card());

        Paragraph::new(diff_lines)
            .block(diff_block)
            .render(hud_chunks[2], buf);
    }
}
