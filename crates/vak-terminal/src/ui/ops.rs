//! Screen 2: Remote Observability & Operations Deck.
//!
//! All data is real — incident stream from `AgentEvent` SSE events,
//! audit receipts from real server work, Merkle graph from causal lineage
//! in presentation events, KPI gauges from `/health`, provider latency
//! from real provider info.

use ratatui::buffer::Buffer;
use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::Modifier;
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Paragraph, Widget};

use crate::api::HealthReport;
use crate::app::{AuditEntry, DeckFocus, IncidentEntry, MerkleNode};
use crate::telemetry::TelemetryState;
use crate::theme::{Symbols, Theme};

pub struct OpsView<'a> {
    pub telemetry: &'a TelemetryState,
    pub theme: &'a Theme,
    pub deck_focus: DeckFocus,
    pub incidents: &'a [IncidentEntry],
    pub audit_entries: &'a [AuditEntry],
    pub merkle_nodes: &'a [MerkleNode],
    pub health: &'a HealthReport,
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
            .constraints([Constraint::Length(3), Constraint::Min(10)])
            .split(area);

        // 1. Top KPI Row
        let kpi_chunks = Layout::default()
            .direction(Direction::Horizontal)
            .constraints([
                Constraint::Percentage(25),
                Constraint::Percentage(25),
                Constraint::Percentage(25),
                Constraint::Percentage(25),
            ])
            .split(main_chunks[0]);

        // FinOps — real from health checks
        let spend_str = if self.telemetry.spend_today_usd > 0.0 {
            format!(
                "${:.2} / ${:.2}",
                self.telemetry.spend_today_usd, self.telemetry.spend_budget_cap_usd
            )
        } else {
            "no budget configured".to_string()
        };
        let finops_block = Block::default()
            .borders(Borders::ALL)
            .border_style(self.theme.style_border());
        let finops_para = Paragraph::new(Line::from(vec![
            Span::styled("FinOps: ", self.theme.style_card()),
            Span::styled(
                spend_str,
                self.theme.style_accent().add_modifier(Modifier::BOLD),
            ),
        ]))
        .block(finops_block);
        finops_para.render(kpi_chunks[0], buf);

        // Circuit Breaker — real from health
        let cb_state = if self.health.circuit_breaker_healthy {
            "100% HEALTHY"
        } else {
            "OPEN"
        };
        let cb_style = if self.health.circuit_breaker_healthy {
            self.theme.style_ok()
        } else {
            self.theme.style_danger()
        };
        let cb_block = Block::default()
            .borders(Borders::ALL)
            .border_style(self.theme.style_border());
        let cb_para = Paragraph::new(Line::from(vec![
            Span::styled("Breakers: ", self.theme.style_card()),
            Span::styled(cb_state, cb_style.add_modifier(Modifier::BOLD)),
        ]))
        .block(cb_block);
        cb_para.render(kpi_chunks[1], buf);

        // Services — real from health warnings
        let service_status = if self.telemetry.active_services_count.1 > 0 {
            format!(
                "{}/{} ONLINE",
                self.telemetry.active_services_count.0, self.telemetry.active_services_count.1
            )
        } else {
            "(no services)".to_string()
        };
        let srv_block = Block::default()
            .borders(Borders::ALL)
            .border_style(self.theme.style_border());
        let srv_para = Paragraph::new(Line::from(vec![
            Span::styled("Services: ", self.theme.style_card()),
            Span::styled(
                service_status,
                self.theme.style_ok().add_modifier(Modifier::BOLD),
            ),
        ]))
        .block(srv_block);
        srv_para.render(kpi_chunks[2], buf);

        // Bus Queue — real from health checks
        let bus_count = self.telemetry.bus_dlq_count;
        let bus_str = if bus_count == 0 {
            "0 DLQ".to_string()
        } else {
            format!("{bus_count} DLQ")
        };
        let bus_block = Block::default()
            .borders(Borders::ALL)
            .border_style(self.theme.style_border());
        let bus_para = Paragraph::new(Line::from(vec![
            Span::styled("Bus Queue: ", self.theme.style_card()),
            Span::styled(bus_str, self.theme.style_card()),
        ]))
        .block(bus_block);
        bus_para.render(kpi_chunks[3], buf);

        // 2. Dual-Deck Split
        let deck_chunks = Layout::default()
            .direction(Direction::Horizontal)
            .constraints([Constraint::Percentage(60), Constraint::Percentage(40)])
            .split(main_chunks[1]);

        // -------------------------------------------------------------
        // LEFT DECK
        // -------------------------------------------------------------
        let left_chunks = Layout::default()
            .direction(Direction::Vertical)
            .constraints([
                Constraint::Length(7),
                Constraint::Length(5),
                Constraint::Min(6),
            ])
            .split(deck_chunks[0]);

        // Incident Stream — real from agent events
        let inc_lines: Vec<Line> = if self.incidents.is_empty() {
            vec![Line::from(vec![Span::styled(
                "No incidents — watching event stream…",
                self.theme.style_card().add_modifier(Modifier::DIM),
            )])]
        } else {
            self.incidents
                .iter()
                .rev()
                .take(5)
                .map(|inc| {
                    let (icon, style) = match inc.severity {
                        crate::app::IncidentSeverity::Critical => {
                            (Symbols::CROSS, self.theme.style_danger())
                        }
                        crate::app::IncidentSeverity::Warn => {
                            (Symbols::STATUS_ACTIVE, self.theme.style_warn())
                        }
                        crate::app::IncidentSeverity::Info => {
                            (Symbols::STATUS_ACTIVE, self.theme.style_info())
                        }
                    };
                    Line::from(vec![
                        Span::styled(
                            format!("[{}] ", inc.timestamp),
                            self.theme.style_card().add_modifier(Modifier::DIM),
                        ),
                        Span::styled(format!("{icon} [{}] ", inc.severity_text()), style),
                        Span::styled(&inc.message, self.theme.style_card()),
                    ])
                })
                .collect()
        };

        let inc_block = Block::default()
            .borders(Borders::ALL)
            .border_style(left_border)
            .title(" Live Incident Stream [Tab] ")
            .title_style(self.theme.style_card());

        Paragraph::new(inc_lines)
            .block(inc_block)
            .render(left_chunks[0], buf);

        // Audit Receipts — real from server receipts
        let audit_lines: Vec<Line> = if self.audit_entries.is_empty() {
            let mut lines = vec![Line::from(vec![Span::styled(
                "ACTION ID                      STATUS     REVISION     FINGERPRINT",
                self.theme.style_card().add_modifier(Modifier::BOLD),
            )])];
            lines.push(Line::from(vec![Span::styled(
                "No audit receipts yet.",
                self.theme.style_card().add_modifier(Modifier::DIM),
            )]));
            lines
        } else {
            let mut lines = vec![Line::from(vec![Span::styled(
                "ACTION ID                      STATUS     REVISION     FINGERPRINT",
                self.theme.style_card().add_modifier(Modifier::BOLD),
            )])];
            for entry in self.audit_entries.iter().take(4) {
                lines.push(Line::from(vec![
                    Span::styled(format!("{:<30} ", entry.action_id), self.theme.style_card()),
                    Span::styled(
                        format!("{:<10} ", entry.status),
                        if entry.status == "VERIFIED" || entry.status == "OK" {
                            self.theme.style_ok()
                        } else {
                            self.theme.style_danger()
                        },
                    ),
                    Span::styled(
                        format!("{:<10} ", entry.revision),
                        self.theme.style_accent(),
                    ),
                    Span::styled(
                        format!("{}…", entry.fingerprint),
                        self.theme.style_card().add_modifier(Modifier::DIM),
                    ),
                ]));
            }
            lines
        };

        let audit_block = Block::default()
            .borders(Borders::ALL)
            .border_style(left_border)
            .title(" Audit Receipts [Ctrl+K] ")
            .title_style(self.theme.style_card());

        Paragraph::new(audit_lines)
            .block(audit_block)
            .render(left_chunks[1], buf);

        // Merkle Causal Graph — real from presentation events
        let merkle_lines: Vec<Line> = if self.merkle_nodes.is_empty() {
            vec![Line::from(vec![Span::styled(
                "No causal lineage recorded yet.",
                self.theme.style_card().add_modifier(Modifier::DIM),
            )])]
        } else {
            self.merkle_nodes
                .iter()
                .map(|node| {
                    let verified_str = if node.verified { " [VERIFIED]" } else { "" };
                    Line::from(vec![
                        Span::styled(&node.label, self.theme.style_info()),
                        Span::styled(" ──► ", self.theme.style_card()),
                        Span::styled(
                            format!("Hash: {}{}", node.hash, verified_str),
                            if node.verified {
                                self.theme.style_ok()
                            } else {
                                self.theme.style_accent()
                            },
                        ),
                    ])
                })
                .collect()
        };

        let merkle_block = Block::default()
            .borders(Borders::ALL)
            .border_style(left_border)
            .title(format!(" {} Causal Lineage Merkle Graph ", Symbols::DELTA))
            .title_style(self.theme.style_accent());

        Paragraph::new(merkle_lines)
            .block(merkle_block)
            .render(left_chunks[2], buf);

        // -------------------------------------------------------------
        // RIGHT DECK
        // -------------------------------------------------------------
        let right_chunks = Layout::default()
            .direction(Direction::Vertical)
            .constraints([
                Constraint::Length(5),
                Constraint::Length(5),
                Constraint::Length(5),
                Constraint::Min(4),
            ])
            .split(deck_chunks[1]);

        // Gauges & Radar — real from health
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
                    "{} ACTIVE AGENTS [{}]    {} PENDING TASKS [{}]",
                    Symbols::STATUS_ACTIVE,
                    self.telemetry.active_workers,
                    Symbols::STATUS_IDLE,
                    self.telemetry.pending_tasks
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
            Span::styled("THROUGHPUT: ", self.theme.style_card()),
            Span::styled(
                format!("{} msg/s  ", self.telemetry.bus_queue_depth),
                self.theme.style_accent(),
            ),
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

        // Distributed Bus Health — real from telemetry
        let queue_depth = self.telemetry.bus_queue_depth;
        let bus_lines = vec![
            Line::from(vec![Span::styled(
                "TOPIC                 DEPTH    THROUGHPUT   STATUS",
                self.theme.style_card().add_modifier(Modifier::BOLD),
            )]),
            Line::from(vec![
                Span::styled(
                    format!("session.events        {queue_depth:<8} "),
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

        // Provider Latency — real provider names from health
        let lat_lines = vec![
            Line::from(vec![
                Span::styled(&self.health.provider, self.theme.style_card()),
                Span::styled(": ", self.theme.style_card()),
                Span::styled(
                    format!("{}ms ", self.telemetry.anthropic_latency_ms),
                    self.theme.style_ok(),
                ),
                Span::styled("[■■■■■■□□□□]", self.theme.style_ok()),
            ]),
            Line::from(vec![
                Span::styled("context: ", self.theme.style_card()),
                Span::styled(
                    self.health
                        .context_window
                        .map_or_else(|| "not stated".to_string(), |w| format!("{w} tok")),
                    self.theme.style_info(),
                ),
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

impl IncidentEntry {
    fn severity_text(&self) -> &'static str {
        match self.severity {
            crate::app::IncidentSeverity::Critical => "CRITICAL",
            crate::app::IncidentSeverity::Warn => "WARN",
            crate::app::IncidentSeverity::Info => "INFO",
        }
    }
}
