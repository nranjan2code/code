use crossterm::event::{KeyCode, KeyEvent, KeyModifiers, MouseButton, MouseEvent, MouseEventKind};
use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::widgets::Widget;

use vak_terminal::app::TerminalApp;
use vak_terminal::graphics::{GraphicsProtocol, HalfBlockImage};
use vak_terminal::hil::{HilApprovalState, HilOutcome};
use vak_terminal::repl::ReplComposer;
use vak_terminal::telemetry::TelemetryState;
use vak_terminal::theme::{Symbols, ThemeKind};
use vak_terminal::ui::header::ActiveTab;

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
        // Verify TrueColor styles can be constructed
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

    // Submit into history
    let submitted = composer.submit();
    assert_eq!(submitted.as_deref(), Some("hello"));
    assert_eq!(composer.buffer, "");
    assert_eq!(composer.history.len(), 1);

    // History up
    composer.history_up();
    assert_eq!(composer.buffer, "hello");

    // History down
    composer.history_down();
    assert_eq!(composer.buffer, "");

    // Slash command trigger
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

#[test]
fn test_all_slash_commands_execution() {
    let mut app = TerminalApp::new("s-test", "claude-3-7-sonnet", "local:8901");

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

    // /model switches model route
    assert_eq!(app.model_name, "claude-3-7-sonnet");
    app.composer.buffer = "/model".into();
    app.handle_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::empty()));
    assert_eq!(app.model_name, "claude-3-5-sonnet");

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
    assert!(!app.should_quit);
    app.composer.buffer = "/quit".into();
    app.handle_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::empty()));
    assert!(app.should_quit);
}

#[test]
fn test_hil_approval_flow_and_editing() {
    let mut hil = HilApprovalState::new(
        "req-1",
        "bash",
        "npm run build && vite preview --port 5173",
        "High network & port binding impact",
        "/workspace/web-app",
        0.042,
    );

    assert_eq!(hil.tool_name, "bash");
    assert_eq!(
        hil.original_command,
        "npm run build && vite preview --port 5173"
    );
    assert!(!hil.is_editing);

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
}

#[test]
fn test_graphics_detection_and_wireframe() {
    let proto = GraphicsProtocol::detect();
    // In test environment, usually HalfBlockAnsi or ITerm2
    assert!(matches!(
        proto,
        GraphicsProtocol::Kitty
            | GraphicsProtocol::ITerm2
            | GraphicsProtocol::Sixel
            | GraphicsProtocol::HalfBlockAnsi
    ));

    let wireframe = HalfBlockImage::sample_dashboard_wireframe(30, 8);
    assert_eq!(wireframe.width, 30);
    assert_eq!(wireframe.height, 8);
    assert_eq!(wireframe.cells.len(), 240);

    // Verify cell rendering on a ratatui buffer
    let mut buf = Buffer::empty(Rect::new(0, 0, 30, 8));
    wireframe.render(Rect::new(0, 0, 30, 8), &mut buf);
    assert_eq!(buf.cell((0, 0)).map(|c| c.symbol()), Some("▀"));
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
}

#[test]
fn test_app_key_and_mouse_dispatch() {
    use vak_terminal::app::DeckFocus;

    let mut app = TerminalApp::new("s-test", "claude-3-7-sonnet", "local:8901");
    assert_eq!(app.active_tab, ActiveTab::Studio);
    assert_eq!(app.deck_focus, DeckFocus::Left);

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
    // Click on Admin tab (col 60)
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

    // Test /hil command triggers HIL modal
    app.composer.buffer = "/hil".into();
    app.handle_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::empty()));
    assert!(app.hil_state.is_some());

    // Widget rendering with active HIL modal across all tabs without panic
    let mut buf = Buffer::empty(Rect::new(0, 0, 100, 30));
    (&app).render(Rect::new(0, 0, 100, 30), &mut buf);

    app.active_tab = ActiveTab::Observability;
    (&app).render(Rect::new(0, 0, 100, 30), &mut buf);

    app.active_tab = ActiveTab::Admin;
    (&app).render(Rect::new(0, 0, 100, 30), &mut buf);

    app.active_tab = ActiveTab::Inbox;
    (&app).render(Rect::new(0, 0, 100, 30), &mut buf);

    // Symbols test
    assert_eq!(Symbols::PROMPT_CHEVRON, "❯");
    assert_eq!(Symbols::STATUS_ACTIVE, "●");
}
