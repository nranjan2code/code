//! Screen 3: Remote Administration & Settings Cockpit (Workspaces, Permissions, MCP, Gateways).

use ratatui::buffer::Buffer;
use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::Modifier;
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Paragraph, Widget};

use crate::app::DeckFocus;
use crate::theme::{Symbols, Theme};

pub struct AdminView<'a> {
    pub theme: &'a Theme,
    pub selected_permission_mode: &'a str,
    pub approval_mode: &'a str,
    pub workspace_name: &'a str,
    pub deck_focus: DeckFocus,
    pub pending_chat_status: Option<bool>,
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
            .constraints([
                Constraint::Percentage(50), // Workspace tree
                Constraint::Percentage(50), // Security engine
            ])
            .split(deck_chunks[0]);

        // CorePool Workspaces
        let ws_lines = vec![
            Line::from(vec![
                Span::styled(format!(" {} root ", Symbols::CARET_EXPANDED), self.theme.style_accent().add_modifier(Modifier::BOLD)),
            ]),
            Line::from(vec![
                Span::styled("   ├── configs/       ", self.theme.style_card()),
                Span::styled("[SHARED BASE LAYER]", self.theme.style_info()),
            ]),
            Line::from(vec![
                Span::styled(format!("   ├── projects/{:<6} ", self.workspace_name), self.theme.style_card()),
                Span::styled("[ACTIVE WORKSPACE ●]", self.theme.style_ok()),
            ]),
            Line::from(vec![
                Span::styled("   └── users/global/  ", self.theme.style_card()),
                Span::styled("USER.md (Profile)", self.theme.style_card().add_modifier(Modifier::DIM)),
            ]),
        ];

        let ws_block = Block::default()
            .borders(Borders::ALL)
            .border_style(left_border)
            .title(" CorePool Multi-Tenant Workspaces [Tab] ")
            .title_style(self.theme.style_card());

        Paragraph::new(ws_lines).block(ws_block).render(left_chunks[0], buf);

        // Security Engine
        let perm_lines = vec![
            Line::from(vec![
                Span::styled("PERMISSION MODES: ", self.theme.style_card().add_modifier(Modifier::BOLD)),
            ]),
            Line::from(vec![
                Span::styled(format!("  {} [ WorkspaceWrite ] ", if self.selected_permission_mode == "WorkspaceWrite" { Symbols::STATUS_ACTIVE } else { Symbols::STATUS_IDLE }), self.theme.style_ok().add_modifier(Modifier::BOLD)),
                Span::styled("(Permits workspace reads & edits; escapes denied)", self.theme.style_card().add_modifier(Modifier::DIM)),
            ]),
            Line::from(vec![
                Span::styled(format!("  {} [ ReadOnly ]       ", if self.selected_permission_mode == "ReadOnly" { Symbols::STATUS_ACTIVE } else { Symbols::STATUS_IDLE }), self.theme.style_card()),
                Span::styled("(Inspection only; all writes fail-closed)", self.theme.style_card().add_modifier(Modifier::DIM)),
            ]),
            Line::from(vec![
                Span::styled(format!("  {} [ FullAccess ]     ", if self.selected_permission_mode == "FullAccess" { Symbols::STATUS_ACTIVE } else { Symbols::STATUS_IDLE }), self.theme.style_card()),
                Span::styled("(Human trust decision; unconstrained execution)", self.theme.style_card().add_modifier(Modifier::DIM)),
            ]),
            Line::from(vec![
                Span::styled("APPROVALS POLICY: ", self.theme.style_card().add_modifier(Modifier::BOLD)),
                Span::styled(format!(" [ {} Ask ]  [ {} AutoApprove ]", if self.approval_mode == "Ask" { Symbols::STATUS_ACTIVE } else { Symbols::STATUS_IDLE }, if self.approval_mode == "AutoApprove" { Symbols::STATUS_ACTIVE } else { Symbols::STATUS_IDLE }), self.theme.style_accent()),
            ]),
        ];

        let perm_block = Block::default()
            .borders(Borders::ALL)
            .border_style(left_border)
            .title(format!(" {} Security & Permissions Engine ", Symbols::STATUS_ACTIVE))
            .title_style(self.theme.style_accent());

        Paragraph::new(perm_lines).block(perm_block).render(left_chunks[1], buf);

        // -------------------------------------------------------------
        // RIGHT DECK: MCP Servers, Gateways, Pending Chat Queue
        // -------------------------------------------------------------
        let right_chunks = Layout::default()
            .direction(Direction::Vertical)
            .constraints([
                Constraint::Length(7), // MCP Servers
                Constraint::Length(5), // Channel Gateways
                Constraint::Min(6),    // Pending Queue
            ])
            .split(deck_chunks[1]);

        // MCP Server Inventory
        let mcp_lines = vec![
            Line::from(vec![
                Span::styled("SERVER NAME    TOOLS    MEMORY      STATUS", self.theme.style_card().add_modifier(Modifier::BOLD)),
            ]),
            Line::from(vec![
                Span::styled("tavily         14       2.1 GB      ", self.theme.style_card()),
                Span::styled("OK", self.theme.style_ok()),
            ]),
            Line::from(vec![
                Span::styled("docker         21       4.8 GB      ", self.theme.style_card()),
                Span::styled("OK", self.theme.style_ok()),
            ]),
            Line::from(vec![
                Span::styled("github          9       3.5 GB      ", self.theme.style_card()),
                Span::styled("OK", self.theme.style_ok()),
            ]),
        ];

        let mcp_block = Block::default()
            .borders(Borders::ALL)
            .border_style(right_border)
            .title(" MCP Server Inventory ")
            .title_style(self.theme.style_card());

        Paragraph::new(mcp_lines).block(mcp_block).render(right_chunks[0], buf);

        // Channel Gateways
        let gw_lines = vec![
            Line::from(vec![
                Span::styled("Telegram Bot:  ", self.theme.style_card()),
                Span::styled("ACTIVE (14 users) ", self.theme.style_ok()),
            ]),
            Line::from(vec![
                Span::styled("Discord Bot:   ", self.theme.style_card()),
                Span::styled("INACTIVE (reconnect 30s)", self.theme.style_card().add_modifier(Modifier::DIM)),
            ]),
            Line::from(vec![
                Span::styled("Slack Gateway: ", self.theme.style_card()),
                Span::styled("DEPLOYING", self.theme.style_warn()),
            ]),
        ];

        let gw_block = Block::default()
            .borders(Borders::ALL)
            .border_style(right_border)
            .title(" Channel Gateways ")
            .title_style(self.theme.style_card());

        Paragraph::new(gw_lines).block(gw_block).render(right_chunks[1], buf);

        // Pending Authorization Queue
        let action_pill = match self.pending_chat_status {
            None => Line::from(vec![
                Span::styled(" [ [y] Approve ] ", self.theme.style_tab_active()),
                Span::raw("   "),
                Span::styled(" [ [d] Deny ] ", self.theme.style_card().add_modifier(Modifier::BOLD)),
            ]),
            Some(true) => Line::from(vec![
                Span::styled(" [ ✓ APPROVED & ALLOWED ] ", self.theme.style_ok().add_modifier(Modifier::BOLD)),
            ]),
            Some(false) => Line::from(vec![
                Span::styled(" [ ✗ DENIED & BLOCKED ] ", self.theme.style_danger().add_modifier(Modifier::BOLD)),
            ]),
        };

        let queue_lines = vec![
            Line::from(vec![
                Span::styled("INBOUND USER:  ", self.theme.style_card()),
                Span::styled("U_9124 (Discord: #general)", self.theme.style_accent().add_modifier(Modifier::BOLD)),
            ]),
            Line::from(vec![
                Span::styled("PROVENANCE:    ", self.theme.style_card()),
                Span::styled("Unknown sender; held at gateway gate", self.theme.style_card()),
            ]),
            action_pill,
        ];

        let queue_block = Block::default()
            .borders(Borders::ALL)
            .border_style(right_border)
            .title(" Pending Chat Authorization Queue [!] ")
            .title_style(self.theme.style_accent());

        Paragraph::new(queue_lines).block(queue_block).render(right_chunks[2], buf);
    }
}
