//! Screen 2: Remote Observability & Operations Deck (Incidents, Merkle Graph, Bus Traffic, Circuit Breakers).

use ratatui::buffer::Buffer;
use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::Modifier;
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Paragraph, Widget};

use crate::app::DeckFocus;
use crate::telemetry::TelemetryState;
use crate::theme::{Symbols, Theme};

pub struct OpsView<'a> {
    pub telemetry: &'a TelemetryState,
    pub theme: &'a Theme,
    pub deck_focus: DeckFocus,
}

impl<'a> Widget for OpsView<'a> {
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

        let main_chunks = Layout::default()
            .direction(Direction::Vertical)
            .constraints([
                Constraint::Length(3), // Top KPI row
                Constraint::Min(10),   // Split decks
            ])
            .split(area);

        // 1. Top KPI Row (4 widgets)
        let kpi_chunks = Layout::default()
            .direction(Direction::Horizontal)
            .constraints([
                Constraint::Percentage(25),
                Constraint::Percentage(25),
                Constraint::Percentage(25),
                Constraint::Percentage(25),
            ])
            .split(main_chunks[0]);

        // FinOps
        let finops_block = Block::default()
            .borders(Borders::ALL)
            .border_style(self.theme.style_border());
        let finops_para = Paragraph::new(Line::from(vec![
            Span::styled("FinOps: ", self.theme.style_card()),
            Span::styled(
                format!(
                    "${:.2} / ${:.2}",
                    self.telemetry.spend_today_usd, self.telemetry.spend_budget_cap_usd
                ),
                self.theme.style_accent().add_modifier(Modifier::BOLD),
            ),
        ]))
        .block(finops_block);
        finops_para.render(kpi_chunks[0], buf);

        // Circuit Breaker
        let cb_block = Block::default()
            .borders(Borders::ALL)
            .border_style(self.theme.style_border());
        let cb_para = Paragraph::new(Line::from(vec![
            Span::styled("Breakers: ", self.theme.style_card()),
            Span::styled(
                "100% HEALTHY",
                self.theme.style_ok().add_modifier(Modifier::BOLD),
            ),
        ]))
        .block(cb_block);
        cb_para.render(kpi_chunks[1], buf);

        // Services
        let srv_block = Block::default()
            .borders(Borders::ALL)
            .border_style(self.theme.style_border());
        let srv_para = Paragraph::new(Line::from(vec![
            Span::styled("Services: ", self.theme.style_card()),
            Span::styled(
                format!(
                    "{}/{} ONLINE",
                    self.telemetry.active_services_count.0, self.telemetry.active_services_count.1
                ),
                self.theme.style_ok().add_modifier(Modifier::BOLD),
            ),
        ]))
        .block(srv_block);
        srv_para.render(kpi_chunks[2], buf);

        // Bus Queue
        let bus_block = Block::default()
            .borders(Borders::ALL)
            .border_style(self.theme.style_border());
        let bus_para = Paragraph::new(Line::from(vec![
            Span::styled("Bus Queue: ", self.theme.style_card()),
            Span::styled(
                format!("{} DLQ", self.telemetry.bus_dlq_count),
                self.theme.style_card(),
            ),
        ]))
        .block(bus_block);
        bus_para.render(kpi_chunks[3], buf);

        // 2. Dual-Deck Split (60% Left / 40% Right)
        let deck_chunks = Layout::default()
            .direction(Direction::Horizontal)
            .constraints([Constraint::Percentage(60), Constraint::Percentage(40)])
            .split(main_chunks[1]);

        // -------------------------------------------------------------
        // LEFT DECK: Incident Stream, Audit Receipts, Merkle Graph
        // -------------------------------------------------------------
        let left_chunks = Layout::default()
            .direction(Direction::Vertical)
            .constraints([
                Constraint::Length(7), // Incident stream
                Constraint::Length(5), // Audit receipts table
                Constraint::Min(6),    // Merkle causal graph
            ])
            .split(deck_chunks[0]);

        // Incident Stream
        let inc_lines = vec![
            Line::from(vec![
                Span::styled(
                    "[17:41:02] ",
                    self.theme.style_card().add_modifier(Modifier::DIM),
                ),
                Span::styled(
                    format!("{} [CRITICAL] ", Symbols::STATUS_ACTIVE),
                    self.theme.style_danger(),
                ),
                Span::styled(
                    "Memory leak detected in model_bridge. Auto rollback in 3s.",
                    self.theme.style_card(),
                ),
            ]),
            Line::from(vec![
                Span::styled(
                    "[17:41:00] ",
                    self.theme.style_card().add_modifier(Modifier::DIM),
                ),
                Span::styled(
                    format!("{} [WARN] ", Symbols::STATUS_ACTIVE),
                    self.theme.style_warn(),
                ),
                Span::styled(
                    "Provider latency spike: anthropic p95 reached 840ms.",
                    self.theme.style_card(),
                ),
            ]),
            Line::from(vec![
                Span::styled(
                    "[17:40:55] ",
                    self.theme.style_card().add_modifier(Modifier::DIM),
                ),
                Span::styled(
                    format!("{} [INFO] ", Symbols::STATUS_ACTIVE),
                    self.theme.style_info(),
                ),
                Span::styled(
                    "Distributed bus topic 'session.events' checkpoint committed.",
                    self.theme.style_card(),
                ),
            ]),
        ];

        let inc_block = Block::default()
            .borders(Borders::ALL)
            .border_style(left_border)
            .title(" Live Real-Time Incident Stream [Tab] ")
            .title_style(self.theme.style_card());

        Paragraph::new(inc_lines)
            .block(inc_block)
            .render(left_chunks[0], buf);

        // Audit Receipts
        let audit_lines = vec![
            Line::from(vec![Span::styled(
                "ACTION ID                      STATUS     REVISION     FINGERPRINT",
                self.theme.style_card().add_modifier(Modifier::BOLD),
            )]),
            Line::from(vec![
                Span::styled("act_38b2889a_bridge_sagent     ", self.theme.style_card()),
                Span::styled("VERIFIED   ", self.theme.style_ok()),
                Span::styled("r09a47f      ", self.theme.style_accent()),
                Span::styled(
                    "sha256:7c9f…",
                    self.theme.style_card().add_modifier(Modifier::DIM),
                ),
            ]),
            Line::from(vec![
                Span::styled("act_e128b930_sandbox_lease     ", self.theme.style_card()),
                Span::styled("ACTIVE     ", self.theme.style_info()),
                Span::styled("r14b82a      ", self.theme.style_accent()),
                Span::styled(
                    "sha256:4821…",
                    self.theme.style_card().add_modifier(Modifier::DIM),
                ),
            ]),
        ];

        let audit_block = Block::default()
            .borders(Borders::ALL)
            .border_style(left_border)
            .title(" Audit Receipts [Ctrl+K] ")
            .title_style(self.theme.style_card());

        Paragraph::new(audit_lines)
            .block(audit_block)
            .render(left_chunks[1], buf);

        // Merkle Causal Graph
        let merkle_lines = vec![
            Line::from(vec![
                Span::styled(
                    "Root Genesis (Turn 0)",
                    self.theme.style_accent().add_modifier(Modifier::BOLD),
                ),
                Span::styled(" ──► ", self.theme.style_card()),
                Span::styled("Intent Read", self.theme.style_info()),
                Span::styled(" ──► ", self.theme.style_card()),
                Span::styled("Sandbox Lease", self.theme.style_ok()),
            ]),
            Line::from(vec![
                Span::styled(
                    "    └── Turn 1 (Dispatched child flow) ",
                    self.theme.style_card(),
                ),
                Span::styled("──► Hash: 0xcbf29ce4", self.theme.style_accent()),
            ]),
            Line::from(vec![
                Span::styled(
                    "    └── Turn 2 (Bash verify receipt)   ",
                    self.theme.style_card(),
                ),
                Span::styled("──► Hash: 0xa5d53825 [VERIFIED]", self.theme.style_ok()),
            ]),
        ];

        let merkle_block = Block::default()
            .borders(Borders::ALL)
            .border_style(left_border)
            .title(format!(" {} Causal Lineage Merkle Graph ", Symbols::DELTA))
            .title_style(self.theme.style_accent());

        Paragraph::new(merkle_lines)
            .block(merkle_block)
            .render(left_chunks[2], buf);

        // -------------------------------------------------------------
        // RIGHT DECK: Gauges, Network Waveform, Bus Health, Latency
        // -------------------------------------------------------------
        let right_chunks = Layout::default()
            .direction(Direction::Vertical)
            .constraints([
                Constraint::Length(5), // Circular Gauges & Swarm Radar
                Constraint::Length(5), // Network traffic wave
                Constraint::Length(5), // Distributed bus queue
                Constraint::Min(4),    // Model latency
            ])
            .split(deck_chunks[1]);

        // Gauges & Radar
        let gauge_lines = vec![
            Line::from(vec![
                Span::styled("CPU: ", self.theme.style_card()),
                Span::styled(
                    format!("{:.1}%  ", self.telemetry.cpu_percent),
                    self.theme.style_accent(),
                ),
                Span::styled("RSS: ", self.theme.style_card()),
                Span::styled(
                    format!("{:.1}MB  ", self.telemetry.rss_mb),
                    self.theme.style_ok(),
                ),
                Span::styled(
                    format!("RADAR: {:.0}°", self.telemetry.radar_angle_deg),
                    self.theme.style_info(),
                ),
            ]),
            Line::from(vec![Span::styled(
                format!(
                    "{} ACTIVE AGENTS [3]    {} PENDING TASKS [12]",
                    Symbols::STATUS_ACTIVE,
                    Symbols::STATUS_IDLE
                ),
                self.theme.style_card(),
            )]),
        ];

        let gauge_block = Block::default()
            .borders(Borders::ALL)
            .border_style(right_border)
            .title(" System Gauges & Swarm Radar ")
            .title_style(self.theme.style_card());

        Paragraph::new(gauge_lines)
            .block(gauge_block)
            .render(right_chunks[0], buf);

        // Network Traffic Waveform
        let net_lines = vec![Line::from(vec![
            Span::styled("THROUGHPUT: 312 MiB/s  ", self.theme.style_card()),
            Span::styled("∿∿∿▲∿∿▲▲∿∿▲∿∿∿▲∿▲∿∿", self.theme.style_accent()),
        ])];

        let net_block = Block::default()
            .borders(Borders::ALL)
            .border_style(right_border)
            .title(" Live Network Traffic Waveform ")
            .title_style(self.theme.style_card());

        Paragraph::new(net_lines)
            .block(net_block)
            .render(right_chunks[1], buf);

        // Distributed Bus Health
        let bus_lines = vec![
            Line::from(vec![Span::styled(
                "TOPIC                 DEPTH    THROUGHPUT   STATUS",
                self.theme.style_card().add_modifier(Modifier::BOLD),
            )]),
            Line::from(vec![
                Span::styled(
                    "session.events        0        4.2k msg/s   ",
                    self.theme.style_card(),
                ),
                Span::styled("OK", self.theme.style_ok()),
            ]),
            Line::from(vec![
                Span::styled(
                    "outbox.delivery       0        18 msg/s     ",
                    self.theme.style_card(),
                ),
                Span::styled("OK", self.theme.style_ok()),
            ]),
        ];

        let bus_block = Block::default()
            .borders(Borders::ALL)
            .border_style(right_border)
            .title(" Distributed Bus Queue Health ")
            .title_style(self.theme.style_card());

        Paragraph::new(bus_lines)
            .block(bus_block)
            .render(right_chunks[2], buf);

        // Provider Latency
        let lat_lines = vec![
            Line::from(vec![
                Span::styled("Anthropic (Claude 3.7): ", self.theme.style_card()),
                Span::styled(
                    format!("{}ms ", self.telemetry.anthropic_latency_ms),
                    self.theme.style_ok(),
                ),
                Span::styled("[■■■■■■□□□□]", self.theme.style_ok()),
            ]),
            Line::from(vec![
                Span::styled("Ollama (Local):         ", self.theme.style_card()),
                Span::styled(
                    format!("{}ms ", self.telemetry.ollama_latency_ms),
                    self.theme.style_ok(),
                ),
                Span::styled("[■■■□□□□□□□]", self.theme.style_ok()),
            ]),
        ];

        let lat_block = Block::default()
            .borders(Borders::ALL)
            .border_style(right_border)
            .title(" Model Provider Latency [p50] ")
            .title_style(self.theme.style_card());

        Paragraph::new(lat_lines)
            .block(lat_block)
            .render(right_chunks[3], buf);
    }
}
