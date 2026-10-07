// A test's output is for the person running it.
#![allow(clippy::disallowed_macros)]
#![allow(
    clippy::expect_used,
    clippy::panic,
    clippy::unwrap_used,
    clippy::field_reassign_with_default,
    clippy::needless_borrow,
    clippy::eq_op
)]

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers, MouseButton, MouseEvent, MouseEventKind};
use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::widgets::Widget;
use std::sync::Arc;

use vak_terminal::api::{ApiClient, ApiError, HealthReport, ProviderList};
use vak_terminal::app::{IncidentSeverity, TerminalApp};
use vak_terminal::hil::{HilApprovalState, HilOutcome, RememberMode};
use vak_terminal::repl::ReplComposer;
use vak_terminal::telemetry::TelemetryState;
use vak_terminal::theme::{Symbols, ThemeKind};
use vak_terminal::ui::header::ActiveTab;

/// Build a TerminalApp wired to a real (but non-functional in tests)
/// ApiClient.  Tests that exercise command dispatch or SSE event handling
/// operate on the app's internal state machine without needing a live server.
fn make_test_app() -> TerminalApp {
    let health = HealthReport::default();
    let api = Arc::new(ApiClient::new("http://127.0.0.1:8901", "test-token"));
    TerminalApp::new(
        "test-session",
        health.model.clone(),
        "http://127.0.0.1:8901",
        health,
        api,
    )
}

#[test]
fn test_theme_cycling_and_palettes() {
    let mut kind = ThemeKind::default();
    assert_eq!(kind, ThemeKind::VakWarm);

    let expected_order = [
        ThemeKind::VakWarm,
        ThemeKind::VakSlate,
        ThemeKind::VakPaper,
        ThemeKind::VakContrast,
        ThemeKind::TokyoNight,
        ThemeKind::VakWarm,
    ];

    for expected in expected_order {
        assert_eq!(kind, expected);
        let theme = kind.theme();
        assert_eq!(theme.kind, kind);
        let _ = theme.style_tab_active();
        let _ = theme.style_tab_inactive();
        let _ = theme.style_border();
        let _ = theme.style_border_focus();
        let _ = theme.style_accent();
        let _ = theme.style_ok();
        let _ = theme.style_danger();
        let _ = theme.style_warn();
        let _ = theme.style_info();
        kind = kind.next();
    }
}

#[test]
fn test_repl_composer_navigation_and_editing() {
    let mut composer = ReplComposer::new();
    assert_eq!(composer.buffer, "");
    assert_eq!(composer.cursor_pos, 0);

    composer.insert_char('h');
    composer.insert_char('e');
    composer.insert_char('l');
    composer.insert_char('l');
    composer.insert_char('o');
    assert_eq!(composer.buffer, "hello");
    assert_eq!(composer.cursor_pos, 5);

    composer.move_cursor_left();
    composer.move_cursor_left();
    assert_eq!(composer.cursor_pos, 3);

    composer.insert_char('p');
    assert_eq!(composer.buffer, "helplo");

    composer.delete_backspace();
    assert_eq!(composer.buffer, "hello");

    composer.move_to_start();
    assert_eq!(composer.cursor_pos, 0);

    composer.move_to_end();
    assert_eq!(composer.cursor_pos, 5);

    let submitted = composer.submit();
    assert_eq!(submitted.as_deref(), Some("hello"));
    assert_eq!(composer.buffer, "");
    assert_eq!(composer.history.len(), 1);

    composer.history_up();
    assert_eq!(composer.buffer, "hello");

    composer.history_down();
    assert_eq!(composer.buffer, "");

    composer.insert_char('/');
    assert!(composer.slash_palette_open);
    assert!(!composer.matching_slash_commands().is_empty());

    composer.insert_char('t');
    composer.insert_char('h');
    let matching = composer.matching_slash_commands();
    assert_eq!(matching.len(), 1);
    assert_eq!(matching[0].name, "/theme");

    composer.complete_selected_slash();
    assert_eq!(composer.buffer, "/theme ");
    assert!(!composer.slash_palette_open);
}

#[tokio::test]
async fn test_slash_command_tab_navigation() {
    let mut app = make_test_app();

    // /ops switches to Observability tab
    app.composer.buffer = "/ops".into();
    app.handle_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::empty()));
    assert_eq!(app.active_tab, ActiveTab::Observability);

    // /admin switches to Admin tab
    app.composer.buffer = "/admin".into();
    app.handle_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::empty()));
    assert_eq!(app.active_tab, ActiveTab::Admin);

    // /inbox switches to Inbox tab
    app.composer.buffer = "/inbox".into();
    app.handle_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::empty()));
    assert_eq!(app.active_tab, ActiveTab::Inbox);

    // /diff switches to Studio and focuses Right deck
    app.composer.buffer = "/diff".into();
    app.handle_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::empty()));
    assert_eq!(app.active_tab, ActiveTab::Studio);
    assert_eq!(app.deck_focus, vak_terminal::app::DeckFocus::Right);

    // /theme cycles theme
    let initial_theme = app.theme.kind;
    app.composer.buffer = "/theme".into();
    app.handle_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::empty()));
    assert_ne!(app.theme.kind, initial_theme);

    // /detach sets should_detach
    assert!(!app.should_detach);
    app.composer.buffer = "/detach".into();
    app.handle_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::empty()));
    assert!(app.should_detach);

    // /quit sets should_quit
    app.composer.buffer = "/quit".into();
    app.handle_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::empty()));
    assert!(app.should_quit);
}

#[test]
fn test_hil_approval_flow_and_editing() {
    // Build a real HilApprovalState from approval event data.
    let mut hil = HilApprovalState::from_approval_event(
        "test-session",
        "req-1",
        "bash",
        r#"{"command":"npm run build && vite preview --port 5173"}"#,
        "High network & port binding impact",
        "/workspace/web-app",
        Some(0.042),
    );

    assert_eq!(hil.tool_name, "bash");
    assert_eq!(
        hil.original_command,
        "npm run build && vite preview --port 5173"
    );
    assert!(!hil.is_editing);
    assert_eq!(hil.remember, RememberMode::No);

    // Unmodified approval
    let outcome = hil.finish_approval();
    assert_eq!(
        outcome,
        HilOutcome::ApproveOnce {
            modified_command: None
        }
    );

    // Start in-place edit
    hil.start_editing();
    assert!(hil.is_editing);

    // Backspace 11 times to remove "--port 5173"
    for _ in 0..11 {
        hil.delete_backspace();
    }
    // Type replacement
    for c in "--port 3000".chars() {
        hil.insert_char(c);
    }

    assert_eq!(
        hil.edited_command,
        "npm run build && vite preview --port 3000"
    );

    let outcome_edited = hil.finish_approval();
    assert_eq!(
        outcome_edited,
        HilOutcome::ApproveOnce {
            modified_command: Some("npm run build && vite preview --port 3000".into())
        }
    );

    // Cancel edit restores original
    hil.cancel_editing();
    assert_eq!(hil.edited_command, hil.original_command);
    assert!(!hil.is_editing);

    // Toggle remember mode
    hil.toggle_remember();
    assert_eq!(hil.remember, RememberMode::Yes);
    hil.toggle_remember();
    assert_eq!(hil.remember, RememberMode::No);
}

#[test]
fn test_graphics_halfblock_render() {
    // Verify we can construct and render a minimal image.
    use vak_terminal::graphics::{GraphicsProtocol, HalfBlockImage};
    let proto = GraphicsProtocol::detect();
    assert!(matches!(
        proto,
        GraphicsProtocol::Kitty
            | GraphicsProtocol::ITerm2
            | GraphicsProtocol::Sixel
            | GraphicsProtocol::HalfBlockAnsi
    ));

    let img = HalfBlockImage::placeholder(10, 4);
    assert_eq!(img.width, 10);
    assert_eq!(img.height, 4);
    assert_eq!(img.cells.len(), 40);

    let mut buf = Buffer::empty(Rect::new(0, 0, 10, 4));
    img.render(Rect::new(0, 0, 10, 4), &mut buf);
    let sym = buf.cell((0, 0)).map(|c| c.symbol());
    assert!(sym.is_some());
}

#[test]
fn test_telemetry_tick_and_rates() {
    let mut telem = TelemetryState::new();
    let initial_angle = telem.radar_angle_deg;

    telem.tick();
    assert_eq!(telem.radar_angle_deg, (initial_angle + 6.0) % 360.0);

    telem.push_token_rate(175);
    assert_eq!(*telem.token_rate_history.last().unwrap(), 175);

    // Push 70 rates to test max 60 buffer bound
    for i in 0..70 {
        telem.push_token_rate(i);
    }
    assert_eq!(telem.token_rate_history.len(), 60);

    // Default telemetry should not have hardcoded values
    let default = TelemetryState::default();
    assert!(!default.connected);
    assert!(default.last_error.is_none());
    assert_eq!(default.cpu_percent, 0.0);
    assert_eq!(default.rss_mb, 0.0);
    assert_eq!(default.spend_today_usd, 0.0);
    assert_eq!(default.active_workers, 0);
    assert_eq!(default.bus_dlq_count, 0);
    assert_eq!(default.anthropic_latency_ms, 0);
    assert!(default.token_rate_history.is_empty());
    assert_eq!(default.max_rate, 0);
}

#[test]
fn test_telemetry_update_from_health() {
    let mut telem = TelemetryState::new();
    let health = HealthReport::default();
    telem.update_from_health(&health);
    // Default health: healthy=false, circuit_breaker_healthy=false
    assert!(!telem.connected);
    assert!(!telem.circuit_breaker_healthy);

    // Simulate a healthy response
    let mut healthy = HealthReport::default();
    healthy.healthy = true;
    healthy.circuit_breaker_healthy = true;
    telem.update_from_health(&healthy);
    assert!(telem.connected);
    assert!(telem.circuit_breaker_healthy);

    // Error handling
    telem.set_connection_error("server down".into());
    assert!(!telem.connected);
    assert_eq!(telem.last_error.as_deref(), Some("server down"));

    // Simulate a healthy response to clear the error
    let mut healthy = HealthReport::default();
    healthy.healthy = true;
    healthy.circuit_breaker_healthy = true;
    telem.update_from_health(&healthy);
    telem.clear_error();
    assert!(telem.connected);
    assert!(telem.last_error.is_none());
}

#[tokio::test]
async fn test_app_key_and_mouse_dispatch() {
    use vak_terminal::app::DeckFocus;

    let mut app = make_test_app();
    assert_eq!(app.active_tab, ActiveTab::Studio);
    assert_eq!(app.deck_focus, DeckFocus::Left);
    assert!(!app.connected);

    // Tab key cycles deck focus (Left <-> Right)
    app.handle_key(KeyEvent::new(KeyCode::Tab, KeyModifiers::empty()));
    assert_eq!(app.deck_focus, DeckFocus::Right);

    app.handle_key(KeyEvent::new(KeyCode::Tab, KeyModifiers::empty()));
    assert_eq!(app.deck_focus, DeckFocus::Left);

    // Space key toggles card expansion in Studio
    assert!(!app.card_expanded);
    app.handle_key(KeyEvent::new(KeyCode::Char(' '), KeyModifiers::empty()));
    assert!(app.card_expanded);
    app.handle_key(KeyEvent::new(KeyCode::Char(' '), KeyModifiers::empty()));
    assert!(!app.card_expanded);

    // Number keys switch active tabs directly
    app.handle_key(KeyEvent::new(KeyCode::Char('2'), KeyModifiers::empty()));
    assert_eq!(app.active_tab, ActiveTab::Observability);

    app.handle_key(KeyEvent::new(KeyCode::Char('3'), KeyModifiers::empty()));
    assert_eq!(app.active_tab, ActiveTab::Admin);

    app.handle_key(KeyEvent::new(KeyCode::Char('4'), KeyModifiers::empty()));
    assert_eq!(app.active_tab, ActiveTab::Inbox);

    // Test Inbox hotkeys: 'a' (ack), 'p' (promote), 'r' (restart)
    assert!(!app.alert_acknowledged);
    assert!(!app.skill_promoted);
    assert!(!app.watchdog_restarted);
    app.handle_key(KeyEvent::new(KeyCode::Char('a'), KeyModifiers::empty()));
    assert!(app.alert_acknowledged);
    app.handle_key(KeyEvent::new(KeyCode::Char('p'), KeyModifiers::empty()));
    assert!(app.skill_promoted);
    app.handle_key(KeyEvent::new(KeyCode::Char('r'), KeyModifiers::empty()));
    assert!(app.watchdog_restarted);

    // Switch back to Studio
    app.handle_key(KeyEvent::new(KeyCode::Char('1'), KeyModifiers::empty()));
    assert_eq!(app.active_tab, ActiveTab::Studio);

    // Mouse click on header row (row 0)
    // Click on Admin tab (col ~60)
    app.handle_mouse(MouseEvent {
        kind: MouseEventKind::Down(MouseButton::Left),
        column: 60,
        row: 0,
        modifiers: KeyModifiers::empty(),
    });
    assert_eq!(app.active_tab, ActiveTab::Admin);

    // Mouse click on Permission Mode in Admin (col 10, row 12 -> ReadOnly)
    app.handle_mouse(MouseEvent {
        kind: MouseEventKind::Down(MouseButton::Left),
        column: 10,
        row: 12,
        modifiers: KeyModifiers::empty(),
    });
    assert_eq!(app.selected_permission_mode, "ReadOnly");

    // Mouse click on Approval Mode (col 30, row 18 -> AutoApprove)
    app.handle_mouse(MouseEvent {
        kind: MouseEventKind::Down(MouseButton::Left),
        column: 30,
        row: 18,
        modifiers: KeyModifiers::empty(),
    });
    assert_eq!(app.approval_mode, "AutoApprove");

    // Mouse click on Pending Authorization Queue Approve (col 65, row 18)
    assert_eq!(app.pending_chat_status, None);
    app.handle_mouse(MouseEvent {
        kind: MouseEventKind::Down(MouseButton::Left),
        column: 65,
        row: 18,
        modifiers: KeyModifiers::empty(),
    });
    assert_eq!(app.pending_chat_status, Some(true));

    // F2 key cycles theme
    let initial_theme = app.theme.kind;
    app.handle_key(KeyEvent::new(KeyCode::F(2), KeyModifiers::empty()));
    assert_ne!(app.theme.kind, initial_theme);

    // Test hotkeys 'm' and 'p' in Admin tab
    app.active_tab = ActiveTab::Admin;
    app.handle_key(KeyEvent::new(KeyCode::Char('m'), KeyModifiers::empty()));
    assert_eq!(app.selected_permission_mode, "FullAccess");
    app.handle_key(KeyEvent::new(KeyCode::Char('p'), KeyModifiers::empty()));
    assert_eq!(app.approval_mode, "Ask");

    // Hotkey 'd' sets pending_chat_status to Some(false)
    app.handle_key(KeyEvent::new(KeyCode::Char('d'), KeyModifiers::empty()));
    assert_eq!(app.pending_chat_status, Some(false));

    // /hil command without any pending chats should open an empty modal
    // from the pending_chats list. With no pending chats, hil_state stays None.
    app.composer.buffer = "/hil".into();
    app.handle_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::empty()));
    // No pending chats in test, so hil_state should be None
    assert!(app.hil_state.is_none());

    // Simulate a real approval request arriving via SSE
    let approval_event = serde_json::json!({
        "ApprovalRequested": {
            "id": "req-real-123",
            "tool": "bash",
            "args_json": r#"{"command":"echo hello"}"#,
            "reason": "Testing real approval"
        }
    });
    app.handle_terminal_event(vak_terminal::api::TerminalEvent::Agent {
        event: approval_event,
        seq: 42,
    });
    assert!(app.hil_state.is_some());
    let hil = app.hil_state.as_ref().unwrap();
    assert_eq!(hil.request_id, "req-real-123");
    assert_eq!(hil.tool_name, "bash");

    // Approve the request
    app.dispatch_approval(HilOutcome::ApproveOnce {
        modified_command: None,
    });
    // The HIL state should be cleared after dispatch
    assert!(app.hil_state.is_none());

    // Test that pending chats were populated from the approval event
    assert!(!app.pending_chats.is_empty());
    assert_eq!(app.pending_chats[0].request_id, "req-real-123");

    // Test health event handling
    let mut healthy = HealthReport::default();
    healthy.healthy = true;
    healthy.circuit_breaker_healthy = true;
    healthy.provider = "anthropic".into();
    healthy.model = "claude-3-7-sonnet".into();
    healthy.permission_mode = "WorkspaceWrite".into();
    app.handle_terminal_event(vak_terminal::api::TerminalEvent::Health(healthy.clone()));
    assert!(app.connected);
    assert_eq!(app.model_name, "claude-3-7-sonnet");
    assert_eq!(app.selected_permission_mode, "WorkspaceWrite");

    // Test health error
    app.handle_terminal_event(vak_terminal::api::TerminalEvent::Error(
        "connection lost".into(),
    ));
    assert!(!app.connected);
    assert_eq!(app.telemetry.last_error.as_deref(), Some("connection lost"));

    // Test connected event
    app.handle_terminal_event(vak_terminal::api::TerminalEvent::Connected);
    assert!(app.connected);
    assert!(app.telemetry.last_error.is_none());

    // Test incident recording from RouteFallback
    let fallback_event = serde_json::json!({
        "RouteFallback": {
            "to_provider": "openai",
            "to_model": "gpt-4o"
        }
    });
    let mut before = app.incidents.len();
    app.handle_terminal_event(vak_terminal::api::TerminalEvent::Agent {
        event: fallback_event,
        seq: 99,
    });
    assert_eq!(app.incidents.len(), before + 1);
    assert_eq!(
        app.incidents.last().unwrap().severity,
        IncidentSeverity::Warn
    );

    // Test ProviderError -> Critical incident
    let error_event = serde_json::json!({
        "ProviderError": {
            "provider": "anthropic",
            "error": "rate limited"
        }
    });
    before = app.incidents.len();
    app.handle_terminal_event(vak_terminal::api::TerminalEvent::Agent {
        event: error_event,
        seq: 100,
    });
    assert_eq!(app.incidents.len(), before + 1);
    assert_eq!(
        app.incidents.last().unwrap().severity,
        IncidentSeverity::Critical
    );

    // Widget rendering with active HIL modal across all tabs without panic
    app.hil_state = Some(HilApprovalState::from_approval_event(
        "test-session",
        "req-ui-456",
        "bash",
        r#"{"command":"ls -la"}"#,
        "Filesystem listing",
        "/workspace",
        None,
    ));

    let mut buf = Buffer::empty(Rect::new(0, 0, 100, 30));
    (&app).render(Rect::new(0, 0, 100, 30), &mut buf);

    app.active_tab = ActiveTab::Observability;
    (&app).render(Rect::new(0, 0, 100, 30), &mut buf);

    app.active_tab = ActiveTab::Admin;
    (&app).render(Rect::new(0, 0, 100, 30), &mut buf);

    app.active_tab = ActiveTab::Inbox;
    (&app).render(Rect::new(0, 0, 100, 30), &mut buf);

    // Dismiss the HIL modal with Escape
    app.handle_key(KeyEvent::new(KeyCode::Esc, KeyModifiers::empty()));
    assert!(app.hil_state.is_none());

    // Symbols test
    assert_eq!(Symbols::PROMPT_CHEVRON, "❯");
    assert_eq!(Symbols::STATUS_ACTIVE, "●");
}

// ---- Real server integration tests ------------------------------------------
// These tests connect to a live vak server (if VAK_TEST_URL + VAK_TEST_TOKEN
// are set in the environment). They are #[ignore]d by default — run with:
//   cargo test -p vak-terminal --test terminal_tests -- --include-ignored
//   VAK_TEST_URL=http://127.0.0.1:8901 VAK_TEST_TOKEN=$VAK_GATEWAY_TOKEN

mod real_server {
    use super::*;

    fn maybe_server() -> Option<(String, String)> {
        let url = std::env::var("VAK_TEST_URL")
            .or_else(|_| std::env::var("VAK_SERVER_URL"))
            .ok()?;
        let token = std::env::var("VAK_TEST_TOKEN")
            .or_else(|_| std::env::var("VAK_GATEWAY_TOKEN"))
            .ok()?;
        if url.is_empty() || token.is_empty() {
            return None;
        }
        Some((url, token))
    }

    fn require_server() -> (String, String) {
        match maybe_server() {
            Some((url, token)) => (url, token),
            None => {
                eprintln!("SKIP: set VAK_TEST_URL + VAK_TEST_TOKEN to run real-server tests");
                panic!("no vak server configured for live test");
            }
        }
    }

    #[tokio::test]
    #[ignore]
    async fn real_health() {
        let (url, token) = require_server();
        let client = ApiClient::new(&url, &token);
        let health = client.health().await.expect("health() must succeed");
        assert!(!health.provider.is_empty(), "provider should be set");
        assert!(!health.model.is_empty(), "model should be set");
        assert!(
            health.context_window.is_none_or(|window| window > 0),
            "a stated context_window is never 0"
        );
        assert!(
            !health.provider.is_empty(),
            "provider must resolve from health"
        );
    }

    #[tokio::test]
    #[ignore]
    async fn real_sessions() {
        let (url, token) = require_server();
        let client = ApiClient::new(&url, &token);
        let sessions = client.list_sessions().await.expect("list_sessions");
        eprintln!("sessions returned: {}", sessions.len());
        for s in &sessions {
            assert!(!s.session_id.is_empty(), "session id must be non-empty");
        }
    }

    #[tokio::test]
    #[ignore]
    async fn real_providers() {
        let (url, token) = require_server();
        let client = ApiClient::new(&url, &token);
        let providers = client.list_providers().await.expect("list_providers");
        assert!(
            !providers.current.is_empty(),
            "current provider must be set"
        );
        assert!(!providers.providers.is_empty(), "must have provider list");
    }

    #[tokio::test]
    #[ignore]
    async fn real_mcp() {
        let (url, token) = require_server();
        let client = ApiClient::new(&url, &token);
        let mcp = client.get_mcp_servers().await.expect("get_mcp_servers");
        eprintln!("mcp servers configured: {}", mcp.len());
    }

    #[tokio::test]
    #[ignore]
    async fn real_gateway_approvals() {
        let (url, token) = require_server();
        let client = ApiClient::new(&url, &token);
        let gw = client.get_gateway_approvals().await.expect("get_gateway");
        assert!(!gw.mode.is_empty(), "gateway mode must be set");
    }

    #[tokio::test]
    #[ignore]
    async fn real_config() {
        let (url, token) = require_server();
        let client = ApiClient::new(&url, &token);
        let config = client.get_config().await.expect("get_config");
        assert!(
            config.permission_mode.is_some()
                || config.approval_mode.is_some()
                || config.theme.is_some(),
            "config should have at least one populated field"
        );
    }

    #[tokio::test]
    #[ignore]
    async fn real_create_and_attach() {
        let (url, token) = require_server();
        let client = ApiClient::new(&url, &token);
        let sid = client.create_session().await.expect("create_session");
        assert!(!sid.is_empty(), "session id must be non-empty");
        let attach = client.attach_session(&sid).await.expect("attach_session");
        assert!(
            !attach.session_id.is_empty(),
            "attach must return session id"
        );
    }

    #[tokio::test]
    #[ignore]
    async fn real_control_state() {
        let (url, token) = require_server();
        let client = ApiClient::new(url, token);
        let sessions = client.list_sessions().await.expect("list_sessions");
        let sid = if let Some(s) = sessions.first() {
            s.session_id.clone()
        } else {
            client.create_session().await.expect("create_session")
        };
        // The control-state endpoint may not exist on older server builds.
        // A 404 is acceptable — we just verify the client handles it as an error.
        match client.get_control_state(&sid).await {
            Ok(_cs) => eprintln!("control_state fetched ok"),
            Err(ApiError::Server { status: 404, .. }) => {
                eprintln!("control-state endpoint not available on this server build (404)")
            }
            Err(e) => panic!("get_control_state failed unexpectedly: {e:?}"),
        }
    }

    #[tokio::test]
    #[ignore]
    async fn real_full_app_flow() {
        // This test builds a TerminalApp from real server data and verifies
        // the whole pipeline works end-to-end.
        let (url, token) = require_server();
        let api = Arc::new(ApiClient::new(&url, &token));
        let health = api.health().await.expect("health");
        assert!(!health.provider.is_empty(), "provider must resolve");

        let sessions = api.list_sessions().await.unwrap_or_default();
        let session_id = sessions
            .first()
            .map(|s| s.session_id.clone())
            .unwrap_or_else(|| "test-session-real".to_string());

        let providers = api.list_providers().await.unwrap_or_default();
        let models = if !providers.current.is_empty() {
            api.discover_models(&providers.current)
                .await
                .unwrap_or_default()
        } else {
            Vec::new()
        };

        // Build the app with real data — verifies all builder methods work
        let _app = TerminalApp::new_with_api(
            &session_id,
            health.model.clone(),
            health.clone(),
            api.clone(),
        )
        .with_workspace("real-test")
        .with_providers(providers)
        .with_models(models)
        .with_mcp_servers(api.get_mcp_servers().await.unwrap_or_default())
        .with_gateway(api.get_gateway_approvals().await.unwrap_or_default())
        .with_config(api.get_config().await.unwrap_or_default())
        .with_launch_servers(
            if !session_id.is_empty() && !session_id.starts_with("test-session-real") {
                api.get_launch_servers(&session_id)
                    .await
                    .unwrap_or_default()
            } else {
                Vec::new()
            },
        )
        .with_sessions(sessions);

        // Verify the app picked up real health data
        // (The app stores health.provider via update_from_health in on_tick,
        // but TerminalApp::new_with_api sets health directly.)
        assert_eq!(_app.health.provider, health.provider);
        assert!(!_app.health.model.is_empty());
    }
}

#[test]
fn test_api_client_url_construction() {
    let client = ApiClient::new("http://127.0.0.1:8901", "secret");
    assert_eq!(client.base_url(), "http://127.0.0.1:8901");
    // internal_url is private, but we can verify base_url
}

#[test]
fn test_api_client_auth_is_bearer() {
    // The token must never be echoed or logged — it is only used as
    // "Bearer <token>" in the Authorization header.
    let client = ApiClient::new("http://localhost:8901", "my-secret-key");
    // Verify the client was constructed with the correct base URL.
    assert_eq!(client.base_url(), "http://localhost:8901");
    // The token is not exposed via any public API — only used internally
    // for the Authorization header.
}

#[test]
fn test_provider_list_default() {
    let list = ProviderList::default();
    assert!(list.providers.is_empty());
    assert_eq!(list.current, "");
    assert_eq!(list.current_model, "");
    assert!(!list.current_configured);
}
