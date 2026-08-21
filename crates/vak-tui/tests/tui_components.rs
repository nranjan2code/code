#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use crossterm::event::{KeyCode, KeyModifiers};
use vak_tui::commands::{Command, help_text, parse};
use vak_tui::editor::Editor;
use vak_tui::keys::{Action, map_key};

fn type_str(e: &mut Editor, s: &str) {
    for c in s.chars() {
        e.insert(c);
    }
}

#[test]
fn editor_insert_cursor_and_backspace() {
    let mut e = Editor::new();
    type_str(&mut e, "hello");
    let (buf, cur) = e.view();
    assert_eq!(buf, "hello");
    assert_eq!(cur, 5);

    e.left();
    e.left();
    e.insert('X');
    assert_eq!(e.view().0, "helXlo");
    assert_eq!(e.view().1, 4);

    e.backspace();
    assert_eq!(e.view().0, "hello");
}

#[test]
fn editor_home_end_delete() {
    let mut e = Editor::new();
    type_str(&mut e, "abc");
    e.home();
    assert_eq!(e.view().1, 0);
    e.delete();
    assert_eq!(e.view().0, "bc");
    e.end();
    assert_eq!(e.view().1, 2);
}

#[test]
fn editor_take_records_history_and_clears() {
    let mut e = Editor::new();
    type_str(&mut e, "first prompt");
    assert_eq!(e.take(), "first prompt");
    assert!(e.is_empty());
    type_str(&mut e, "second");
    e.history_prev();
    assert_eq!(e.view().0, "first prompt");
    e.history_next();
    assert_eq!(e.view().0, "second");
}

#[test]
fn editor_history_walks_backwards_then_returns_to_draft() {
    let mut e = Editor::new();
    type_str(&mut e, "one");
    let _ = e.take();
    type_str(&mut e, "two");
    let _ = e.take();

    type_str(&mut e, "draft text");
    e.history_prev();
    assert_eq!(e.view().0, "two");
    e.history_prev();
    assert_eq!(e.view().0, "one");
    e.history_prev();
    assert_eq!(e.view().0, "one", "stays at oldest");
    e.history_next();
    assert_eq!(e.view().0, "two");
    e.history_next();
    assert_eq!(e.view().0, "draft text", "returns to draft");
}

#[test]
fn editor_multibyte_safety() {
    let mut e = Editor::new();
    for c in "héllo→".chars() {
        e.insert(c);
    }
    e.home();
    e.right();
    e.right();
    e.backspace();
    assert_eq!(
        e.view().0,
        "hllo→",
        "backspace removes the char before the cursor"
    );
}

#[test]
fn ctrl_c_maps_to_cancel_or_clear_in_both_states() {
    assert!(matches!(
        map_key(KeyCode::Char('c'), KeyModifiers::CONTROL, true),
        Action::CancelOrClear
    ));
    assert!(matches!(
        map_key(KeyCode::Char('c'), KeyModifiers::CONTROL, false),
        Action::CancelOrClear
    ));
}

#[test]
fn plain_keys_map() {
    assert!(matches!(
        map_key(KeyCode::Enter, KeyModifiers::NONE, false),
        Action::Submit
    ));
    assert!(matches!(
        map_key(KeyCode::Up, KeyModifiers::NONE, false),
        Action::HistoryPrev
    ));
    assert!(matches!(
        map_key(KeyCode::Down, KeyModifiers::NONE, false),
        Action::HistoryNext
    ));
    assert!(matches!(
        map_key(KeyCode::Backspace, KeyModifiers::NONE, false),
        Action::Backspace
    ));
    assert!(matches!(
        map_key(KeyCode::Char('x'), KeyModifiers::NONE, false),
        Action::Insert('x')
    ));
    assert!(matches!(
        map_key(KeyCode::Char('x'), KeyModifiers::SHIFT, false),
        Action::Insert('x')
    ));
    assert!(matches!(
        map_key(KeyCode::Char('d'), KeyModifiers::CONTROL, false),
        Action::Exit
    ));
    assert!(matches!(
        map_key(KeyCode::Esc, KeyModifiers::NONE, false),
        Action::Ignore
    ));
    assert!(matches!(
        map_key(KeyCode::Esc, KeyModifiers::NONE, true),
        Action::CancelOrClear
    ));
}

#[test]
fn command_parsing() {
    assert!(matches!(parse("/help"), Some(Command::Help)));
    assert!(matches!(parse("  /help  "), Some(Command::Help)));
    assert!(matches!(parse("/?"), Some(Command::Help)));
    assert!(matches!(parse("/exit"), Some(Command::Exit)));
    assert!(matches!(parse("/quit"), Some(Command::Exit)));
    assert!(matches!(parse("/cost"), Some(Command::Cost)));
    assert!(matches!(parse("/context"), Some(Command::Context)));
    assert!(matches!(parse("/sessions"), Some(Command::Sessions)));
    assert!(matches!(parse("/clear"), Some(Command::Clear)));
    match parse("/model gpt-5.6-terra") {
        Some(Command::Model(m)) => assert_eq!(m, "gpt-5.6-terra"),
        other => panic!("expected model command, got {other:?}"),
    }
    assert!(
        parse("/model").is_none(),
        "/model without arg is not a command"
    );
    assert!(parse("/nope").is_none());
    assert!(parse("plain text").is_none());
    assert!(parse("").is_none());
    assert!(help_text().lines().count() > 3);
}
