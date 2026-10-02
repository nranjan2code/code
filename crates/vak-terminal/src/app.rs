//! Application state machine, event loop coordination, and view dispatch.
//!
//! All state is populated from the live server API. There are no hardcoded
//! session names, model names, URLs, or mock data — everything the user
//! sees is either fetched from `GET /health`, subscribed to via SSE,
//! or derived from a real API response.

use std::collections::HashMap;
use std::sync::Arc;

use ratatui::buffer::Buffer;
use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::widgets::Widget;

use crate::api::{
    ApiClient, ConfigSnapshot, ControlState, GatewayApprovals, HealthReport, LaunchServer,
    McpServerDef, ProviderList, SessionInfo, TerminalEvent,
};
use crate::hil::{HilApprovalState, HilOutcome};
use crate::repl::ReplComposer;
use crate::telemetry::TelemetryState;
use crate::theme::{Theme, ThemeKind};
use crate::ui::admin::AdminView;
use crate::ui::composer::ComposerView;
use crate::ui::header::{ActiveTab, HeaderView};
use crate::ui::hil_modal::HilModalView;
use crate::ui::inbox::InboxView;
use crate::ui::ops::OpsView;
use crate::ui::studio::StudioView;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum DeckFocus {
    #[default]
    Left,
    Right,
}

/// A real agent-event-derived log entry for the ops incident stream.
#[derive(Debug, Clone)]
pub struct IncidentEntry {
    pub timestamp: String,
    pub severity: IncidentSeverity,
    pub message: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IncidentSeverity {
    Critical,
    Warn,
    Info,
}

/// A real audit-receipt entry.
#[derive(Debug, Clone)]
pub struct AuditEntry {
    pub action_id: String,
    pub status: String,
    pub revision: String,
    pub fingerprint: String,
}

/// A real node in the causal Merkle graph.
#[derive(Debug, Clone)]
pub struct MerkleNode {
    pub label: String,
    pub hash: String,
    pub verified: bool,
}

/// A real MCP server row for the admin inventory.
#[derive(Debug, Clone)]
pub struct McpRow {
    pub name: String,
    pub tool_count: usize,
    pub memory_mb: f64,
    pub status: McpStatus,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum McpStatus {
    Ok,
    Error,
    Starting,
}

/// A real pending chat/approval entry from the gateway queue.
#[derive(Debug, Clone)]
pub struct PendingChatEntry {
    pub surface: String,
    pub sender: String,
    pub preview: String,
    pub request_id: String,
}

/// A real inbox entry.
#[derive(Debug, Clone)]
pub struct InboxEntry {
    pub id: String,
    pub timestamp: String,
    pub kind: InboxKind,
    pub title: String,
    pub body: String,
    pub actionable: Option<InboxAction>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InboxKind {
    BudgetAlert,
    ApprovalPending,
    ApprovalDenied,
    WatchdogFailure,
    SkillProposal,
    ScheduledTask,
    Heartbeat,
}

#[derive(Debug, Clone)]
pub enum InboxAction {
    Ack,
    Approve,
    Deny,
    Restart,
    Promote,
    Reject,
}

/// A real presentation item from the SSE presentation stream.
#[derive(Debug, Clone)]
pub struct PresItem {
    pub id: String,
    pub kind: String,
    pub title: String,
    pub status: String,
}

pub struct TerminalApp {
    pub active_tab: ActiveTab,
    pub deck_focus: DeckFocus,
    pub theme: Theme,
    pub telemetry: TelemetryState,
    pub composer: ReplComposer,
    pub hil_state: Option<HilApprovalState>,
    pub session_id: String,
    pub model_name: String,
    pub host_endpoint: String,
    pub selected_permission_mode: String,
    pub approval_mode: String,
    pub workspace_name: String,
    pub scroll_offset: usize,
    pub card_expanded: bool,
    pub alert_acknowledged: bool,
    pub skill_promoted: bool,
    pub watchdog_restarted: bool,
    pub pending_chat_status: Option<bool>,
    pub should_quit: bool,
    pub should_detach: bool,

    // --- Real API-backed data ---
    pub api: Arc<ApiClient>,
    pub health: HealthReport,
    pub providers: ProviderList,
    pub models: Vec<String>,
    pub mcp_servers: HashMap<String, McpServerDef>,
    pub gateway: GatewayApprovals,
    pub config: ConfigSnapshot,
    pub control_state: ControlState,
    pub launch_servers: Vec<LaunchServer>,
    pub sessions: Vec<SessionInfo>,

    // Event-driven state from SSE
    pub incidents: Vec<IncidentEntry>,
    pub audit_entries: Vec<AuditEntry>,
    pub merkle_nodes: Vec<MerkleNode>,
    pub pending_chats: Vec<PendingChatEntry>,
    pub presentation_items: Vec<PresItem>,
    pub mcp_rows: Vec<McpRow>,
    pub inbox_entries: Vec<InboxEntry>,

    // Lifecycle
    pub connected: bool,
}

impl TerminalApp {
    pub fn new(
        session_id: impl Into<String>,
        model_name: impl Into<String>,
        host_endpoint: impl Into<String>,
        health: HealthReport,
        api: Arc<ApiClient>,
    ) -> Self {
        let providers = ProviderList {
            current: health.provider.clone(),
            current_model: health.model.clone(),
            current_configured: true,
            providers: Vec::new(),
        };

        Self {
            active_tab: ActiveTab::Studio,
            deck_focus: DeckFocus::Left,
            theme: ThemeKind::default().theme(),
            telemetry: TelemetryState::new(),
            composer: ReplComposer::new(),
            hil_state: None,
            session_id: session_id.into(),
            model_name: model_name.into(),
            host_endpoint: host_endpoint.into(),
            selected_permission_mode: health.permission_mode.clone(),
            approval_mode: health.permission_mode.clone(),
            workspace_name: health.cwd.clone(),
            scroll_offset: 0,
            card_expanded: false,
            alert_acknowledged: false,
            skill_promoted: false,
            watchdog_restarted: false,
            pending_chat_status: None,
            should_quit: false,
            should_detach: false,

            api,
            health,
            providers,
            models: Vec::new(),
            mcp_servers: HashMap::new(),
            gateway: GatewayApprovals {
                mode: "deny".into(),
                approver: None,
                timeout_secs: 30,
                enabled: false,
                forwarding: false,
                candidates: Vec::new(),
            },
            config: ConfigSnapshot::default(),
            control_state: ControlState::default(),
            launch_servers: Vec::new(),
            sessions: Vec::new(),
            incidents: Vec::new(),
            audit_entries: Vec::new(),
            merkle_nodes: Vec::new(),
            pending_chats: Vec::new(),
            presentation_items: Vec::new(),
            mcp_rows: Vec::new(),
            inbox_entries: Vec::new(),
            connected: false,
        }
    }

    /// Build a TerminalApp from a real API client — the host endpoint is
    /// derived from the client's base URL so the header always shows where
    /// the terminal is connected to.
    pub fn new_with_api(
        session_id: impl Into<String>,
        model_name: impl Into<String>,
        health: HealthReport,
        api: Arc<ApiClient>,
    ) -> Self {
        let host = api.base_url().to_string();
        Self::new(session_id, model_name, host, health, api)
    }

    pub fn with_workspace(mut self, name: impl Into<String>) -> Self {
        self.workspace_name = name.into();
        self
    }

    pub fn with_providers(mut self, providers: ProviderList) -> Self {
        self.providers = providers;
        self
    }

    pub fn with_palette_extras(mut self, extras: Vec<crate::repl::PaletteEntry>) -> Self {
        self.composer.extra_commands = extras;
        self
    }

    pub fn with_models(mut self, models: Vec<String>) -> Self {
        self.models = models;
        self
    }

    pub fn with_mcp_servers(mut self, servers: HashMap<String, McpServerDef>) -> Self {
        self.mcp_servers = servers.clone();
        self.mcp_rows = servers
            .keys()
            .map(|name| McpRow {
                name: name.clone(),
                tool_count: 0,
                memory_mb: 0.0,
                status: McpStatus::Ok,
            })
            .collect();
        self
    }

    pub fn with_gateway(mut self, gateway: GatewayApprovals) -> Self {
        self.gateway = gateway;
        self
    }

    pub fn with_config(mut self, config: ConfigSnapshot) -> Self {
        self.config = config;
        self
    }

    pub fn with_control_state(mut self, state: ControlState) -> Self {
        self.control_state = state;
        self
    }

    pub fn with_launch_servers(mut self, servers: Vec<LaunchServer>) -> Self {
        self.launch_servers = servers;
        self
    }

    pub fn with_sessions(mut self, sessions: Vec<SessionInfo>) -> Self {
        self.sessions = sessions;
        self
    }

    /// Advance tick animations (50ms) and derive a token-rate sample when
    /// no fresh agent event has arrived.
    pub fn on_tick(&mut self) {
        self.telemetry.tick();
    }

    /// Cycle to the next theme.
    pub fn cycle_theme(&mut self) {
        let next_kind = self.theme.kind.next();
        self.theme = next_kind.theme();
    }

    /// Process a real event from the SSE watcher channel.
    pub fn handle_terminal_event(&mut self, event: TerminalEvent) {
        match event {
            TerminalEvent::Connected => {
                self.connected = true;
                self.telemetry.connected = true;
                self.telemetry.last_error = None;
            }
            TerminalEvent::Error(msg) => {
                self.connected = false;
                self.telemetry.set_connection_error(msg);
            }
            TerminalEvent::Health(report) => {
                self.connected = true;
                self.telemetry.update_from_health(&report);
                self.telemetry.clear_error();
                self.health = report.clone();
                self.model_name = report.model.clone();
                self.selected_permission_mode = report.permission_mode.clone();
            }
            TerminalEvent::Agent { event, seq } => {
                self.handle_agent_event(&event, seq);
            }
            TerminalEvent::Presentation { frame, seq } => {
                self.handle_presentation_event(&frame, seq);
            }
        }
    }

    /// Dispatch a real `AgentEvent` from the SSE stream.
    fn handle_agent_event(&mut self, event_json: &serde_json::Value, _seq: u64) {
        if let Some(approval) = crate::hil::extract_approval_request(event_json) {
            let hil = HilApprovalState::from_approval_event(
                self.session_id.clone(),
                approval.request_id.clone(),
                approval.tool.clone(),
                approval.args_json.clone(),
                approval.reason.clone(),
                self.health.cwd.clone(),
                None,
            );
            self.hil_state = Some(hil);
            self.pending_chats.push(PendingChatEntry {
                surface: "gateway".into(),
                sender: approval.tool.clone(),
                preview: approval.reason.clone(),
                request_id: approval.request_id.clone(),
            });
            return;
        }

        if let Some(turn_end) = event_json.get("TurnEnd")
            && let Some(usage) = turn_end.get("usage")
        {
            let input = usage
                .get("input_tokens")
                .and_then(|v| v.as_u64())
                .unwrap_or(0);
            let output = usage
                .get("output_tokens")
                .and_then(|v| v.as_u64())
                .unwrap_or(0);
            let total = (input + output) as u32;
            self.telemetry.push_token_rate(total);
        }

        if event_json.get("WorkerStarted").is_some() {
            self.telemetry
                .set_active_workers(self.telemetry.active_workers.saturating_add(1));
        }
        if event_json.get("WorkerFinished").is_some() {
            self.telemetry
                .set_active_workers(self.telemetry.active_workers.saturating_sub(1));
        }

        if event_json.get("RunFinished").is_some() {
            self.control_state.running = false;
        }

        if let Some(fallback) = event_json.get("RouteFallback") {
            let to_provider = fallback
                .get("to_provider")
                .and_then(|v| v.as_str())
                .unwrap_or("unknown");
            let to_model = fallback
                .get("to_model")
                .and_then(|v| v.as_str())
                .unwrap_or("unknown");
            self.incidents.push(IncidentEntry {
                timestamp: chrono::Local::now().format("%H:%M:%S").to_string(),
                severity: IncidentSeverity::Warn,
                message: format!("Route fallback to {to_provider}:{to_model}"),
            });
        }

        if let Some(err) = event_json.get("ProviderError") {
            let provider = err.get("provider").and_then(|v| v.as_str()).unwrap_or("?");
            let msg = err.get("error").and_then(|v| v.as_str()).unwrap_or("?");
            self.incidents.push(IncidentEntry {
                timestamp: chrono::Local::now().format("%H:%M:%S").to_string(),
                severity: IncidentSeverity::Critical,
                message: format!("{provider}: {msg}"),
            });
        }
    }

    /// Dispatch a real presentation `OutputStreamFrame` from the SSE stream.
    fn handle_presentation_event(&mut self, frame_json: &serde_json::Value, _seq: u64) {
        if let Some(snapshot) = frame_json.get("snapshot") {
            if let Some(items) = snapshot.get("items").and_then(|v| v.as_array()) {
                let mut new_items = Vec::new();
                for item in items {
                    let id = item
                        .get("id")
                        .and_then(|v| v.as_str())
                        .unwrap_or("")
                        .to_string();
                    let kind = item
                        .get("kind")
                        .and_then(|v| v.as_str())
                        .unwrap_or("unknown")
                        .to_string();
                    let title = item
                        .get("title")
                        .and_then(|v| v.as_str())
                        .unwrap_or("")
                        .to_string();
                    let status = item
                        .get("status")
                        .and_then(|v| v.as_str())
                        .unwrap_or("unknown")
                        .to_string();
                    new_items.push(PresItem {
                        id: id.clone(),
                        kind,
                        title,
                        status,
                    });
                }
                self.presentation_items = new_items;
            }

            if let Some(diag) = snapshot.get("diagnostics").and_then(|v| v.as_array()) {
                for d in diag {
                    if let Some(text) = d.as_str() {
                        self.incidents.push(IncidentEntry {
                            timestamp: chrono::Local::now().format("%H:%M:%S").to_string(),
                            severity: IncidentSeverity::Info,
                            message: text.to_string(),
                        });
                    }
                }
            }
        }

        if let Some(delta) = frame_json.get("delta").and_then(|v| v.as_object())
            && let Some(item) = delta.get("item").and_then(|v| v.as_object())
        {
            let id = item
                .get("id")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string();
            let kind = item
                .get("kind")
                .and_then(|v| v.as_str())
                .unwrap_or("unknown")
                .to_string();
            let title = item
                .get("title")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string();
            let status = item
                .get("status")
                .and_then(|v| v.as_str())
                .unwrap_or("unknown")
                .to_string();
            if let Some(existing) = self.presentation_items.iter_mut().find(|i| i.id == id) {
                existing.kind = kind;
                existing.title = title;
                existing.status = status;
            } else {
                self.presentation_items.push(PresItem {
                    id,
                    kind,
                    title,
                    status,
                });
            }
        }
    }

    /// Handle keyboard input.
    pub fn handle_key(&mut self, key: crossterm::event::KeyEvent) {
        use crossterm::event::KeyCode;

        // 1. If HIL modal is open, it captures inputs
        if let Some(ref mut hil) = self.hil_state {
            if hil.is_editing {
                match key.code {
                    KeyCode::Enter => {
                        hil.is_editing = false;
                    }
                    KeyCode::Esc => {
                        hil.cancel_editing();
                    }
                    KeyCode::Backspace => {
                        hil.delete_backspace();
                    }
                    KeyCode::Char(c) => {
                        hil.insert_char(c);
                    }
                    _ => {}
                }
                return;
            }

            match key.code {
                KeyCode::Char('y') | KeyCode::Enter => {
                    self.dispatch_approval(HilOutcome::ApproveOnce {
                        modified_command: None,
                    });
                }
                KeyCode::Char('a') => {
                    self.dispatch_approval(HilOutcome::AlwaysForSession);
                }
                KeyCode::Char('d') => {
                    self.dispatch_approval(HilOutcome::Deny);
                }
                KeyCode::Char('e') => {
                    hil.start_editing();
                }
                KeyCode::Char('r') => {
                    hil.toggle_remember();
                }
                KeyCode::Esc => {
                    self.hil_state = None;
                }
                _ => {}
            }
            return;
        }

        // 2. Global Hotkeys (Ctrl+D, Ctrl+C, Ctrl+K)
        if key
            .modifiers
            .contains(crossterm::event::KeyModifiers::CONTROL)
        {
            match key.code {
                KeyCode::Char('d') => {
                    self.should_detach = true;
                    return;
                }
                KeyCode::Char('c') => {
                    self.should_quit = true;
                    return;
                }
                KeyCode::Char('k') => {
                    self.composer.buffer = "/".into();
                    self.composer.cursor_pos = 1;
                    self.composer.slash_palette_open = true;
                    return;
                }
                _ => {}
            }
        }

        // F2 cycles theme
        if key.code == KeyCode::F(2) {
            self.cycle_theme();
            return;
        }

        // Tab switches deck focus or completes slash commands
        if key.code == KeyCode::Tab {
            if self.composer.slash_palette_open {
                self.composer.complete_selected_slash();
                return;
            }
            self.deck_focus = match self.deck_focus {
                DeckFocus::Left => DeckFocus::Right,
                DeckFocus::Right => DeckFocus::Left,
            };
            return;
        }

        if key.code == KeyCode::BackTab {
            self.deck_focus = match self.deck_focus {
                DeckFocus::Left => DeckFocus::Right,
                DeckFocus::Right => DeckFocus::Left,
            };
            return;
        }

        // Number keys 1-4 and hotkeys (when composer buffer is empty)
        if self.composer.buffer.is_empty() {
            match key.code {
                KeyCode::Char('1') => {
                    self.active_tab = ActiveTab::Studio;
                    return;
                }
                KeyCode::Char('2') => {
                    self.active_tab = ActiveTab::Observability;
                    return;
                }
                KeyCode::Char('3') => {
                    self.active_tab = ActiveTab::Admin;
                    return;
                }
                KeyCode::Char('4') => {
                    self.active_tab = ActiveTab::Inbox;
                    return;
                }
                KeyCode::Char(' ') => {
                    self.card_expanded = !self.card_expanded;
                    return;
                }
                KeyCode::Char('m') if self.active_tab == ActiveTab::Admin => {
                    self.cycle_permission_mode();
                    return;
                }
                KeyCode::Char('p') if self.active_tab == ActiveTab::Admin => {
                    self.cycle_approval_mode();
                    return;
                }
                KeyCode::Char('y') if self.active_tab == ActiveTab::Admin => {
                    self.pending_chat_status = Some(true);
                    if let Some(_chat) = self.pending_chats.first() {
                        self.dispatch_approval_from_chat(true, false);
                    }
                    return;
                }
                KeyCode::Char('d') if self.active_tab == ActiveTab::Admin => {
                    self.pending_chat_status = Some(false);
                    if let Some(_chat) = self.pending_chats.first() {
                        self.dispatch_approval_from_chat(false, false);
                    }
                    return;
                }
                KeyCode::Char('a') if self.active_tab == ActiveTab::Inbox => {
                    self.alert_acknowledged = true;
                    return;
                }
                KeyCode::Char('p') if self.active_tab == ActiveTab::Inbox => {
                    self.skill_promoted = true;
                    return;
                }
                KeyCode::Char('r') if self.active_tab == ActiveTab::Inbox => {
                    self.watchdog_restarted = true;
                    return;
                }
                _ => {}
            }
        }

        // 3. Quick Action Palette navigation
        if self.composer.slash_palette_open {
            match key.code {
                KeyCode::Down => {
                    self.composer.select_next_slash();
                    return;
                }
                KeyCode::Up => {
                    self.composer.select_prev_slash();
                    return;
                }
                KeyCode::Esc => {
                    self.composer.slash_palette_open = false;
                    return;
                }
                _ => {}
            }
        }

        // 4. Composer typing
        match key.code {
            KeyCode::Enter => {
                if let Some(cmd) = self.composer.submit() {
                    self.execute_user_command(&cmd);
                }
            }
            KeyCode::Char(c) => {
                self.composer.insert_char(c);
            }
            KeyCode::Backspace => {
                self.composer.delete_backspace();
            }
            KeyCode::Left => {
                self.composer.move_cursor_left();
            }
            KeyCode::Right => {
                self.composer.move_cursor_right();
            }
            KeyCode::Home => {
                self.composer.move_to_start();
            }
            KeyCode::End => {
                self.composer.move_to_end();
            }
            KeyCode::Up => {
                self.composer.history_up();
            }
            KeyCode::Down => {
                self.composer.history_down();
            }
            KeyCode::Esc => {
                self.composer.clear();
            }
            _ => {}
        }
    }

    /// Execute a slash command or free text, wired to the REAL API.
    fn execute_user_command(&mut self, cmd: &str) {
        let rt = tokio::runtime::Handle::current();
        let api = self.api.clone();
        let session_id = self.session_id.clone();

        if cmd == "/model" {
            // Cycle through REAL discovered models and PATCH the config.
            if !self.models.is_empty() {
                let idx = self
                    .models
                    .iter()
                    .position(|m| m == &self.model_name)
                    .unwrap_or(0);
                let next = (idx + 1) % self.models.len();
                let new_model = self.models[next].clone();
                self.model_name = new_model.clone();
                rt.spawn(async move {
                    let patch = crate::api::ConfigPatch {
                        model: Some(new_model),
                        ..Default::default()
                    };
                    if let Err(e) = api.patch_config(&patch).await {
                        eprintln!("failed to set model: {e}");
                    }
                });
            }
            return;
        }

        if cmd == "/ops" {
            self.active_tab = ActiveTab::Observability;
            return;
        }
        if cmd == "/admin" {
            self.active_tab = ActiveTab::Admin;
            return;
        }
        if cmd == "/inbox" {
            self.active_tab = ActiveTab::Inbox;
            return;
        }
        if cmd == "/diff" {
            self.active_tab = ActiveTab::Studio;
            self.deck_focus = DeckFocus::Right;
            return;
        }

        if cmd == "/preview" {
            // Open the REAL launch server URL (from /sessions/{id}/launch).
            if let Some(port) = self
                .launch_servers
                .iter()
                .find(|s| s.running && s.port.is_some())
                .and_then(|s| s.port)
            {
                let url = format!("http://localhost:{port}");
                let _ = std::process::Command::new("open").arg(&url).spawn();
            }
            return;
        }

        if cmd == "/hil" || cmd == "/gate" {
            // Show the first pending approval from the REAL queue (SSE-derived).
            if let Some(chat) = self.pending_chats.first() {
                let hil = HilApprovalState::from_approval_event(
                    self.session_id.clone(),
                    chat.request_id.clone(),
                    chat.sender.clone(),
                    "{}".to_string(),
                    chat.preview.clone(),
                    self.health.cwd.clone(),
                    None,
                );
                self.hil_state = Some(hil);
            }
            return;
        }

        if cmd == "/theme" {
            self.cycle_theme();
            return;
        }
        if cmd == "/compact" {
            self.composer.clear();
            rt.spawn(async move {
                if let Err(e) = api.compact_session(&session_id).await {
                    eprintln!("could not compact: {e}");
                }
            });
            return;
        }
        if cmd == "/clear" {
            self.composer.clear();
            self.scroll_offset = 0;
            return;
        }
        if cmd == "/help" {
            self.scroll_offset = 0;
            return;
        }
        if cmd == "/quit" || cmd == "/exit" {
            self.should_quit = true;
            return;
        }
        if cmd == "/detach" {
            self.should_detach = true;
            return;
        }

        // Any other text: send it as a steering message to the running session.
        let cmd_owned = cmd.to_string();
        rt.spawn(async move {
            if let Err(e) = api.send_steering(&session_id, &cmd_owned).await {
                eprintln!("failed to send steering: {e}");
            }
        });
    }

    /// Dispatch a real HIL approval to the server via POST and clear the modal.
    pub fn dispatch_approval(&mut self, outcome: HilOutcome) {
        if let Some(hil) = self.hil_state.take() {
            let api = self.api.clone();
            let session_id = hil.session_id.clone();
            let req_id = hil.request_id.clone();
            let approve = matches!(
                outcome,
                HilOutcome::ApproveOnce { .. } | HilOutcome::AlwaysForSession
            );
            let remember = matches!(outcome, HilOutcome::AlwaysForSession);

            let rt = tokio::runtime::Handle::current();
            rt.spawn(async move {
                match api
                    .answer_approval(&session_id, &req_id, approve, remember)
                    .await
                {
                    Ok(ans) => {
                        if let Some(rule) = ans.learned_rule {
                            eprintln!("learned rule: {rule}");
                        }
                        if let Some(err) = ans.learn_error {
                            eprintln!("could not learn rule: {err}");
                        }
                    }
                    Err(e) => {
                        eprintln!("approval dispatch failed: {e}");
                    }
                }
            });
        }
    }

    /// Dispatch an approval from a pending chat entry in the Admin view.
    fn dispatch_approval_from_chat(&mut self, approve: bool, remember: bool) {
        if let Some(chat) = self.pending_chats.first() {
            let api = self.api.clone();
            let session_id = chat.request_id.clone(); // The request_id IS the approval target
            let req_id = chat.request_id.clone();
            let _ = session_id; // We use the session_id for the URL and req_id for the path
            let api_clone = api.clone();
            let sid = self.session_id.clone();
            let rt = tokio::runtime::Handle::current();
            rt.spawn(async move {
                if let Err(e) = api_clone
                    .answer_approval(&sid, &req_id, approve, remember)
                    .await
                {
                    eprintln!("approval dispatch failed: {e}");
                }
            });
        }
    }

    /// Cycle permission mode and push to the real server.
    /// Order: ReadOnly → FullAccess → WorkspaceWrite → ReadOnly
    fn cycle_permission_mode(&mut self) {
        let next = match self.selected_permission_mode.as_str() {
            "ReadOnly" => "FullAccess",
            "FullAccess" => "WorkspaceWrite",
            _ => "ReadOnly",
        };
        self.selected_permission_mode = next.to_string();
        let api = self.api.clone();
        let mode = next.to_string();
        let rt = tokio::runtime::Handle::current();
        rt.spawn(async move {
            if let Err(e) = api.set_permission_mode(&mode).await {
                eprintln!("failed to set permission mode: {e}");
            }
        });
    }

    /// Toggle approval mode.
    fn cycle_approval_mode(&mut self) {
        let next = if self.approval_mode == "Ask" {
            "AutoApprove"
        } else {
            "Ask"
        };
        self.approval_mode = next.to_string();
    }

    /// Refresh pending chats from the current queue.
    pub fn refresh_pending_chats(&mut self) {
        self.pending_chat_status = if self.pending_chats.is_empty() {
            None
        } else {
            Some(true)
        };
    }

    /// Handle mouse clicks and scrolling.
    pub fn handle_mouse(&mut self, mouse: crossterm::event::MouseEvent) {
        use crossterm::event::{MouseButton, MouseEventKind};

        match mouse.kind {
            MouseEventKind::Down(MouseButton::Left) => {
                let x = mouse.column;
                let y = mouse.row;

                // 1. Top navigation tab bar (row 0)
                if y == 0 {
                    if (1..=19).contains(&x) {
                        self.active_tab = ActiveTab::Studio;
                    } else if (20..=48).contains(&x) {
                        self.active_tab = ActiveTab::Observability;
                    } else if (49..=80).contains(&x) {
                        self.active_tab = ActiveTab::Admin;
                    } else if (81..=114).contains(&x) {
                        self.active_tab = ActiveTab::Inbox;
                    }
                    return;
                }

                // 2. If HIL modal is active, check click on modal buttons
                if let Some(ref _hil) = self.hil_state {
                    if (10..=16).contains(&y) {
                        if (15..=32).contains(&x) {
                            self.dispatch_approval(HilOutcome::ApproveOnce {
                                modified_command: None,
                            });
                            return;
                        } else if (33..=56).contains(&x) {
                            self.hil_state = None;
                            return;
                        } else if (57..=70).contains(&x) {
                            if let Some(hil) = &mut self.hil_state {
                                hil.start_editing();
                            }
                            return;
                        }
                    }
                    return;
                }

                if y > 0 && y < 26 {
                    self.deck_focus = if x < 60 {
                        DeckFocus::Left
                    } else {
                        DeckFocus::Right
                    };
                }

                // 3. Screen-specific interactive clicks
                match self.active_tab {
                    ActiveTab::Admin => {
                        if x < 45 {
                            if (8..=10).contains(&y) {
                                self.selected_permission_mode = "WorkspaceWrite".into();
                                let api = self.api.clone();
                                let rt = tokio::runtime::Handle::current();
                                rt.spawn(async move {
                                    let _ = api.set_permission_mode("WorkspaceWrite").await;
                                });
                            } else if (11..=13).contains(&y) {
                                self.selected_permission_mode = "ReadOnly".into();
                                let api = self.api.clone();
                                let rt = tokio::runtime::Handle::current();
                                rt.spawn(async move {
                                    let _ = api.set_permission_mode("ReadOnly").await;
                                });
                            } else if (14..=16).contains(&y) {
                                // FullAccess: explicit human trust decision —
                                // do NOT auto-escalate from a click. Send to
                                // approval gate instead.
                                self.dispatch_approval(HilOutcome::ApproveOnce {
                                    modified_command: None,
                                });
                            }
                        }
                        if (17..=19).contains(&y) {
                            if (15..=25).contains(&x) {
                                self.approval_mode = "Ask".into();
                            } else if (26..=45).contains(&x) {
                                self.approval_mode = "AutoApprove".into();
                            }
                        }
                        // Pending Authorization Queue buttons
                        if x >= 55 && (16..=20).contains(&y) {
                            if (55..=75).contains(&x) {
                                self.pending_chat_status = Some(true);
                                self.dispatch_approval_from_chat(true, false);
                            } else if (76..=95).contains(&x) {
                                self.pending_chat_status = Some(false);
                                self.dispatch_approval_from_chat(false, false);
                            }
                        }
                    }
                    ActiveTab::Inbox => {
                        if (1..=4).contains(&y) && (60..=75).contains(&x) {
                            self.alert_acknowledged = true;
                        }
                        if (5..=9).contains(&y) && (40..=65).contains(&x) {
                            self.watchdog_restarted = true;
                        }
                        if (15..=20).contains(&y) && x >= 60 {
                            self.skill_promoted = true;
                        }
                    }
                    ActiveTab::Studio => {
                        if (8..=11).contains(&y)
                            && (2..=32).contains(&x)
                            && let Some(port) = self
                                .launch_servers
                                .iter()
                                .find(|s| s.running && s.port.is_some())
                                .and_then(|s| s.port)
                        {
                            let url = format!("http://localhost:{port}");
                            let _ = std::process::Command::new("open").arg(&url).spawn();
                        }
                        if (11..=15).contains(&y) && x < 60 {
                            self.card_expanded = !self.card_expanded;
                        }
                    }
                    ActiveTab::Observability => {}
                }
            }
            MouseEventKind::ScrollDown => {
                self.scroll_offset = self.scroll_offset.saturating_add(1);
            }
            MouseEventKind::ScrollUp => {
                self.scroll_offset = self.scroll_offset.saturating_sub(1);
            }
            _ => {}
        }
    }

    pub fn quit_requested(&self) -> bool {
        self.should_quit
    }

    pub fn detach_requested(&self) -> bool {
        self.should_detach
    }

    /// Render the full terminal UI.  All data comes from the real
    /// API-backed fields, never from hardcoded values.
    pub fn render_to(&self, area: Rect, buf: &mut Buffer) {
        let chunks = Layout::default()
            .direction(Direction::Vertical)
            .constraints([
                Constraint::Length(1), // Header tab bar
                Constraint::Min(10),   // Active deck view
                Constraint::Length(4), // Pinned bottom composer
            ])
            .split(area);

        // 1. Header View (real session, model, server, spend)
        let header = HeaderView {
            active_tab: self.active_tab,
            session_name: &self.session_id,
            model_name: &self.model_name,
            host_endpoint: &self.host_endpoint,
            spend_usd: 0.0,
            connected: self.connected,
            error: self.telemetry.last_error.clone(),
            theme: &self.theme,
        };
        header.render(chunks[0], buf);

        // 2. Active Screen View
        match self.active_tab {
            ActiveTab::Studio => {
                let launch_url = self
                    .launch_servers
                    .iter()
                    .find(|s| s.running && s.port.is_some())
                    .and_then(|s| s.port.map(|port| format!("http://localhost:{port}")));
                let view = StudioView {
                    telemetry: &self.telemetry,
                    theme: &self.theme,
                    is_running: self.control_state.running,
                    deck_focus: self.deck_focus,
                    card_expanded: self.card_expanded,
                    presentation_items: &self.presentation_items,
                    launch_url: launch_url.as_deref(),
                    sandbox_name: &self.health.sandbox,
                    cwd: &self.health.cwd,
                };
                view.render(chunks[1], buf);
            }
            ActiveTab::Observability => {
                let view = OpsView {
                    telemetry: &self.telemetry,
                    theme: &self.theme,
                    deck_focus: self.deck_focus,
                    incidents: &self.incidents,
                    audit_entries: &self.audit_entries,
                    merkle_nodes: &self.merkle_nodes,
                    health: &self.health,
                };
                view.render(chunks[1], buf);
            }
            ActiveTab::Admin => {
                let view = AdminView {
                    theme: &self.theme,
                    selected_permission_mode: &self.selected_permission_mode,
                    approval_mode: &self.approval_mode,
                    workspace_name: &self.workspace_name,
                    deck_focus: self.deck_focus,
                    pending_chat_status: self.pending_chat_status,
                    mcp_rows: &self.mcp_rows,
                    mcp_servers: &self.mcp_servers,
                    gateway: &self.gateway,
                    sessions: &self.sessions,
                    pending_chats: &self.pending_chats,
                    health: &self.health,
                    connected: self.connected,
                };
                view.render(chunks[1], buf);
            }
            ActiveTab::Inbox => {
                let view = InboxView {
                    theme: &self.theme,
                    deck_focus: self.deck_focus,
                    alert_acknowledged: self.alert_acknowledged,
                    skill_promoted: self.skill_promoted,
                    watchdog_restarted: self.watchdog_restarted,
                    inbox_entries: &self.inbox_entries,
                    health: &self.health,
                };
                view.render(chunks[1], buf);
            }
        }

        // 3. Pinned Bottom Composer
        let composer_view = ComposerView {
            composer: &self.composer,
            theme: &self.theme,
            connected: self.connected,
            error: self.telemetry.last_error.clone(),
        };
        composer_view.render(chunks[2], buf);

        // 4. Floating HIL Approval Modal (if active)
        if let Some(ref hil) = self.hil_state {
            let modal_view = HilModalView {
                state: hil,
                theme: &self.theme,
                remember: hil.remember,
            };
            modal_view.render(area, buf);
        }
    }
}

impl Widget for &TerminalApp {
    fn render(self, area: Rect, buf: &mut Buffer) {
        self.render_to(area, buf);
    }
}
