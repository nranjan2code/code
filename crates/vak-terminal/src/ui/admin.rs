//! Screen 3: Remote Administration & Settings Cockpit.
//!
//! All data is real:
//! - Workspace from `/health` cwd
//! - Permission mode from `/health` + PATCH via server API
//! - MCP inventory from `GET /config/mcp`
//! - Gateway/bot status from `GET /gateway/bots` + `GET /gateway/approvals`
//! - Pending approval queue from SSE `ApprovalRequested` events

use ratatui::buffer::Buffer;
use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::Modifier;
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Paragraph, Widget};

use crate::api::{GatewayApprovals, HealthReport, McpServerDef, SessionInfo};
use crate::app::{DeckFocus, McpRow};
use crate::theme::{Symbols, Theme};
use std::collections::HashMap;

pub struct AdminView<'a> {
    pub theme: &'a Theme,
    pub selected_permission_mode: &'a str,
    pub approval_mode: &'a str,
    pub workspace_name: &'a str,
    pub deck_focus: DeckFocus,
    pub pending_chat_status: Option<bool>,
    pub mcp_rows: &'a [McpRow],
    pub mcp_servers: &'a HashMap<String, McpServerDef>,
    pub gateway: &'a GatewayApprovals,
    pub sessions: &'a [SessionInfo],
    pub pending_chats: &'a [crate::app::PendingChatEntry],
    pub health: &'a HealthReport,
    pub connected: bool,
}

impl<'a> Widget for AdminView<'a> {
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
        // LEFT DECK: CorePool Workspaces & Security Engine
        // -------------------------------------------------------------
        let left_chunks = Layout::default()
            .direction(Direction::Vertical)
            .constraints([Constraint::Percentage(50), Constraint::Percentage(50)])
            .split(deck_chunks[0]);

        // CorePool Workspaces — real from sessions/health
        let ws_lines: Vec<Line> = {
            let mut lines = vec![Line::from(vec![Span::styled(
                format!(" {} {} ", Symbols::CARET_EXPANDED, self.workspace_name),
                self.theme.style_accent().add_modifier(Modifier::BOLD),
            )])];
            let session_count = self.sessions.len();
            lines.push(Line::from(vec![
                Span::styled("  sessions/           ", self.theme.style_card()),
                Span::styled(
                    format!("[{} active]", session_count),
                    self.theme.style_info(),
                ),
            ]));
            for s in self.sessions.iter().take(3) {
                lines.push(Line::from(vec![
                    Span::styled(
                        format!("    ├── {} ", s.session_id),
                        self.theme.style_card(),
                    ),
                    Span::styled(
                        format!("[{}]", s.status()),
                        if s.status() == "active" {
                            self.theme.style_ok()
                        } else {
                            self.theme.style_card()
                        },
                    ),
                ]));
            }
            lines
        };

        let ws_block = Block::default()
            .borders(Borders::ALL)
            .border_style(left_border)
            .title(" CorePool Multi-Tenant Workspaces [Tab] ")
            .title_style(self.theme.style_card());

        Paragraph::new(ws_lines)
            .block(ws_block)
            .render(left_chunks[0], buf);

        // Security Engine — real permission mode from health
        let perm_lines = vec![
            Line::from(vec![Span::styled(
                "PERMISSION MODES: ",
                self.theme.style_card().add_modifier(Modifier::BOLD),
            )]),
            Line::from(vec![
                Span::styled(
                    format!(
                        "  {} [ WorkspaceWrite ] ",
                        if self.selected_permission_mode == "WorkspaceWrite" {
                            Symbols::STATUS_ACTIVE
                        } else {
                            Symbols::STATUS_IDLE
                        }
                    ),
                    if self.selected_permission_mode == "WorkspaceWrite" {
                        self.theme.style_ok().add_modifier(Modifier::BOLD)
                    } else {
                        self.theme.style_card()
                    },
                ),
                Span::styled(
                    "(Permits workspace reads & edits; escapes denied)",
                    self.theme.style_card().add_modifier(Modifier::DIM),
                ),
            ]),
            Line::from(vec![
                Span::styled(
                    format!(
                        "  {} [ ReadOnly ]       ",
                        if self.selected_permission_mode == "ReadOnly" {
                            Symbols::STATUS_ACTIVE
                        } else {
                            Symbols::STATUS_IDLE
                        }
                    ),
                    self.theme.style_card(),
                ),
                Span::styled(
                    "(Inspection only; all writes fail-closed)",
                    self.theme.style_card().add_modifier(Modifier::DIM),
                ),
            ]),
            Line::from(vec![
                Span::styled(
                    format!(
                        "  {} [ FullAccess ]     ",
                        if self.selected_permission_mode == "FullAccess" {
                            Symbols::STATUS_ACTIVE
                        } else {
                            Symbols::STATUS_IDLE
                        }
                    ),
                    self.theme.style_card(),
                ),
                Span::styled(
                    "(Human trust decision; unconstrained execution)",
                    self.theme.style_card().add_modifier(Modifier::DIM),
                ),
            ]),
            Line::from(vec![
                Span::styled(
                    "APPROVALS POLICY: ",
                    self.theme.style_card().add_modifier(Modifier::BOLD),
                ),
                Span::styled(
                    format!(
                        " [ {} Ask ]  [ {} AutoApprove ]",
                        if self.approval_mode == "Ask" {
                            Symbols::STATUS_ACTIVE
                        } else {
                            Symbols::STATUS_IDLE
                        },
                        if self.approval_mode == "AutoApprove" {
                            Symbols::STATUS_ACTIVE
                        } else {
                            Symbols::STATUS_IDLE
                        }
                    ),
                    self.theme.style_accent(),
                ),
            ]),
        ];

        let perm_block = Block::default()
            .borders(Borders::ALL)
            .border_style(left_border)
            .title(format!(
                " {} Security & Permissions Engine ",
                Symbols::STATUS_ACTIVE
            ))
            .title_style(self.theme.style_accent());

        Paragraph::new(perm_lines)
            .block(perm_block)
            .render(left_chunks[1], buf);

        // -------------------------------------------------------------
        // RIGHT DECK: MCP Servers, Channel Gateways, Pending Chat Queue
        // -------------------------------------------------------------
        let right_chunks = Layout::default()
            .direction(Direction::Vertical)
            .constraints([
                Constraint::Length(7),
                Constraint::Length(5),
                Constraint::Min(6),
            ])
            .split(deck_chunks[1]);

        // MCP Server Inventory — real from /config/mcp
        let mcp_lines: Vec<Line> = if self.mcp_rows.is_empty() {
            vec![
                Line::from(vec![Span::styled(
                    "SERVER NAME    TOOLS    MEMORY      STATUS",
                    self.theme.style_card().add_modifier(Modifier::BOLD),
                )]),
                Line::from(vec![Span::styled(
                    "(no MCP servers configured)",
                    self.theme.style_card().add_modifier(Modifier::DIM),
                )]),
            ]
        } else {
            let mut lines = vec![Line::from(vec![Span::styled(
                "SERVER NAME    TOOLS    MEMORY      STATUS",
                self.theme.style_card().add_modifier(Modifier::BOLD),
            )])];
            for row in self.mcp_rows {
                lines.push(Line::from(vec![
                    Span::styled(format!("{:<14} ", row.name), self.theme.style_card()),
                    Span::styled(format!("{:<8} ", row.tool_count), self.theme.style_card()),
                    Span::styled(
                        format!("{:<5.1}GB ", row.memory_mb),
                        self.theme.style_card(),
                    ),
                    Span::styled(
                        match row.status {
                            crate::app::McpStatus::Ok => "OK",
                            crate::app::McpStatus::Error => "ERR",
                            crate::app::McpStatus::Starting => "…",
                        },
                        match row.status {
                            crate::app::McpStatus::Ok => self.theme.style_ok(),
                            crate::app::McpStatus::Error => self.theme.style_danger(),
                            crate::app::McpStatus::Starting => self.theme.style_warn(),
                        },
                    ),
                ]));
            }
            lines
        };

        let mcp_block = Block::default()
            .borders(Borders::ALL)
            .border_style(right_border)
            .title(" MCP Server Inventory ")
            .title_style(self.theme.style_card());

        Paragraph::new(mcp_lines)
            .block(mcp_block)
            .render(right_chunks[0], buf);

        // Channel Gateways — real from gateway approvals/bots data
        let gw_lines: Vec<Line> = {
            let mode = if self.gateway.enabled {
                "ENABLED"
            } else {
                "DISABLED"
            };
            let mode_style = if self.gateway.enabled {
                self.theme.style_ok()
            } else {
                self.theme.style_warn()
            };
            vec![
                Line::from(vec![
                    Span::styled("Mode: ", self.theme.style_card()),
                    Span::styled(mode, mode_style.add_modifier(Modifier::BOLD)),
                ]),
                Line::from(vec![
                    Span::styled("Approver: ", self.theme.style_card()),
                    Span::styled(
                        self.gateway.approver.clone().unwrap_or("(none)".into()),
                        self.theme.style_card(),
                    ),
                ]),
                Line::from(vec![
                    Span::styled("Forwarding: ", self.theme.style_card()),
                    Span::styled(
                        format!(
                            "{} ({}s)",
                            if self.gateway.forwarding { "ON" } else { "OFF" },
                            self.gateway.timeout_secs
                        ),
                        self.theme.style_info(),
                    ),
                ]),
            ]
        };

        let gw_block = Block::default()
            .borders(Borders::ALL)
            .border_style(right_border)
            .title(" Channel Gateways ")
            .title_style(self.theme.style_card());

        Paragraph::new(gw_lines)
            .block(gw_block)
            .render(right_chunks[1], buf);

        // Pending Authorization Queue — real from pending_chats (SSE-derived)
        let action_pill = match self.pending_chat_status {
            None => Line::from(vec![
                Span::styled(" [ [y] Approve ] ", self.theme.style_tab_active()),
                Span::raw("   "),
                Span::styled(
                    " [ [d] Deny ] ",
                    self.theme.style_card().add_modifier(Modifier::BOLD),
                ),
            ]),
            Some(true) => Line::from(vec![Span::styled(
                " [ ✓ APPROVED & ALLOWED ] ",
                self.theme.style_ok().add_modifier(Modifier::BOLD),
            )]),
            Some(false) => Line::from(vec![Span::styled(
                " [ ✗ DENIED & BLOCKED ] ",
                self.theme.style_danger().add_modifier(Modifier::BOLD),
            )]),
        };

        let queue_lines: Vec<Line> = if self.pending_chats.is_empty() {
            vec![
                Line::from(vec![Span::styled(
                    "INBOUND USER:  ",
                    self.theme.style_card(),
                )]),
                Line::from(vec![Span::styled(
                    "(no pending approvals)",
                    self.theme.style_card().add_modifier(Modifier::DIM),
                )]),
                action_pill,
            ]
        } else {
            let chat = &self.pending_chats[0];
            vec![
                Line::from(vec![
                    Span::styled("INBOUND USER:  ", self.theme.style_card()),
                    Span::styled(
                        format!("{} ({})", chat.sender, chat.surface),
                        self.theme.style_accent().add_modifier(Modifier::BOLD),
                    ),
                ]),
                Line::from(vec![
                    Span::styled("PROVENANCE:    ", self.theme.style_card()),
                    Span::styled(&chat.preview, self.theme.style_card()),
                ]),
                action_pill,
            ]
        };

        let queue_block = Block::default()
            .borders(Borders::ALL)
            .border_style(right_border)
            .title(" Pending Chat Authorization Queue [!] ")
            .title_style(self.theme.style_accent());

        Paragraph::new(queue_lines)
            .block(queue_block)
            .render(right_chunks[2], buf);
    }
}
