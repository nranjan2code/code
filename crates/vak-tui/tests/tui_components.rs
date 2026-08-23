#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use crossterm::event::{KeyCode, KeyModifiers};
use std::fs;
use std::path::Path;
use vak_tui::commands::{Command, help_text, parse};
use vak_tui::complete::complete;
use vak_tui::editor::Editor;
use vak_tui::keys::{Action, describe, map_key};
use vak_tui::palette::{ChoiceItem, ChoicePicker, CommandPalette};
use vak_tui::width::{char_width, str_width};

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
fn editor_home_end_and_kill_are_line_aware() {
    let mut e = Editor::new();
    type_str(&mut e, "first\nsecond line\nthird");
    for _ in 0..8 {
        e.left();
    }
    e.home();
    assert_eq!(e.line_col(), (1, 0));
    e.end();
    assert_eq!(e.line_col(), (1, 11));
    for _ in 0..5 {
        e.left();
    }
    e.delete_to_line_end();
    assert_eq!(e.view().0, "first\nsecond\nthird");
    e.undo();
    assert_eq!(e.view().0, "first\nsecond line\nthird");
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
fn ctrl_c_interrupts_running_and_clears_idle() {
    assert!(matches!(
        map_key(KeyCode::Char('c'), KeyModifiers::CONTROL, true),
        Action::Interrupt
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
    assert!(matches!(parse("/details"), Some(Command::Details)));
    assert!(matches!(parse("/keys"), Some(Command::Keys(None))));
    assert!(matches!(
        parse("/keys raw"),
        Some(Command::Keys(Some(arg))) if arg == "raw"
    ));
    assert!(matches!(parse("/keymap"), Some(Command::Keymap)));
    match parse("/model gpt-5.6-terra") {
        Some(Command::Model(Some(m))) => assert_eq!(m, "gpt-5.6-terra"),
        other => panic!("expected model command, got {other:?}"),
    }
    assert!(matches!(parse("/model"), Some(Command::Model(None))));
    assert!(matches!(parse("/provider"), Some(Command::Provider(None))));
    assert!(matches!(parse("/key"), Some(Command::Key(None))));
    match parse("/key opencode-zen sk-secret") {
        Some(Command::Key(Some(arg))) => {
            let mut parts = arg.splitn(2, char::is_whitespace);
            assert_eq!(parts.next(), Some("opencode-zen"));
            assert_eq!(parts.next().map(str::trim), Some("sk-secret"));
        }
        other => panic!("expected key command, got {other:?}"),
    }
    assert!(matches!(
        parse("/key anthropic"),
        Some(Command::Key(Some(_)))
    ));
    assert!(matches!(parse("/config"), Some(Command::Config)));
    assert!(matches!(parse("/features"), Some(Command::Features)));
    assert!(parse("/nope").is_none());
    assert!(parse("plain text").is_none());
    assert!(parse("").is_none());
    assert!(help_text().lines().count() > 3);
}

#[test]
fn choice_picker_filters_navigates_and_accepts_custom_model_ids() {
    let mut picker = ChoicePicker::new(
        vec![
            ChoiceItem {
                value: "alpha".into(),
                description: "first model".into(),
                active: true,
            },
            ChoiceItem {
                value: "beta".into(),
                description: "second model".into(),
                active: false,
            },
        ],
        true,
    );
    picker.down();
    assert_eq!(picker.value().as_deref(), Some("beta"));
    for c in "missing/model".chars() {
        picker.push(c);
    }
    assert!(picker.filtered().is_empty());
    assert_eq!(picker.value().as_deref(), Some("missing/model"));
}

#[test]
fn editor_multiline_newline_cursor_math() {
    let mut e = Editor::new();
    type_str(&mut e, "ab\ncd");
    assert_eq!(e.view(), ("ab\ncd", 5));
    assert_eq!(e.line_col(), (1, 2));
    e.left();
    e.left();
    assert_eq!(e.line_col(), (1, 0));
    assert_eq!(e.text_before(), "ab\n");
    assert_eq!(e.text_after(), "cd");
    e.left();
    assert_eq!(e.line_col(), (0, 2));
    assert!(e.is_multiline());
    assert_eq!(e.take(), "ab\ncd");
    assert!(!e.is_multiline());
    assert_eq!(e.line_col(), (0, 0));
}

#[test]
fn editor_cjk_line_col_and_width() {
    let mut e = Editor::new();
    type_str(&mut e, "中\n文");
    assert_eq!(e.view().1, 3);
    assert_eq!(e.line_col(), (1, 1));
    assert_eq!(str_width(e.view().0), 4);
    assert_eq!(char_width('中'), 2);
    assert_eq!(char_width('文'), 2);
    assert_eq!(char_width('\n'), 0);
    assert_eq!(char_width('\u{301}'), 0);
    assert_eq!(str_width("中文"), 4);
    e.left();
    assert_eq!(e.line_col(), (1, 0));
    e.left();
    assert_eq!(e.line_col(), (0, 1));
}

#[test]
fn editor_paste_str_with_newlines() {
    let mut e = Editor::new();
    type_str(&mut e, "a");
    e.paste_str("中\n文\n");
    assert_eq!(e.view().0, "a中\n文\n");
    assert_eq!(e.view().1, 5);
    assert_eq!(e.line_col(), (2, 0));
    assert!(e.is_multiline());
}

#[test]
fn editor_large_paste_stashes_placeholder_and_resolves_on_take() {
    let mut e = Editor::new();
    let payload = "x".repeat(vak_tui::editor::STASH_CHAR_LIMIT + 1);
    e.paste_str(&payload);
    let (buf, _) = e.view();
    assert!(
        buf.contains("[stashed paste #0 · ") && buf.contains("· Ctrl-O expands]"),
        "placeholder expected, got: {buf}"
    );
    assert_eq!(buf.lines().count(), 1);
    assert!(e.has_stashes());
    let submitted = e.take();
    assert_eq!(submitted, payload);
    assert!(!e.has_stashes());
}

#[test]
fn editor_many_lines_paste_stashes_too() {
    let mut e = Editor::new();
    let payload = "line\n".repeat(vak_tui::editor::STASH_LINE_LIMIT + 1);
    e.paste_str(&payload);
    assert!(e.has_stashes());
    assert_eq!(e.view().0.lines().count(), 1);
    assert_eq!(e.take(), payload);
}

#[test]
fn editor_expand_stash_at_cursor_restores_exact_payload() {
    let mut e = Editor::new();
    type_str(&mut e, "before ");
    let payload = format!("{}\n", "中".repeat(40)).repeat(vak_tui::editor::STASH_LINE_LIMIT + 2);
    e.paste_str(&payload);
    type_str(&mut e, " after");
    // Cursor sits after the placeholder; move onto the placeholder line.
    for _ in 0.." after".chars().count() {
        e.left();
    }
    let msg = e.expand_stash_at_cursor();
    assert!(msg.is_some());
    // The stash renders its placeholder on its own line, so the separating
    // newline remains once expanded.
    assert_eq!(e.view().0, format!("before \n{payload} after"));
    assert!(!e.has_stashes());
}

#[test]
fn editor_expand_without_stash_is_a_no_op() {
    let mut e = Editor::new();
    type_str(&mut e, "plain text");
    e.home();
    assert!(e.expand_stash_at_cursor().is_none());
    assert_eq!(e.view().0, "plain text");
}

#[test]
fn editor_two_stashes_resolve_in_order() {
    let mut e = Editor::new();
    let first = "a".repeat(vak_tui::editor::STASH_CHAR_LIMIT + 7);
    let second = "b".repeat(vak_tui::editor::STASH_CHAR_LIMIT + 11);
    e.paste_str(&first);
    e.paste_str("\nmid\n");
    // Second stash lands on a fresh line; expand nothing, resolve both.
    e.paste_str(&second);
    assert_eq!(e.stash_count(), 2);
    let submitted = e.take();
    assert_eq!(submitted, format!("{first}\nmid\n{second}"));
}

#[test]
fn editor_draft_roundtrip_consumes_the_file() {
    let dir = std::env::temp_dir().join(format!("vak-tui-draft-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("composer_draft.txt");
    let mut e = Editor::new();
    type_str(&mut e, "unsent draft 中文");
    assert!(e.save_draft(&path));

    let mut fresh = Editor::new();
    assert!(fresh.recover_draft(&path));
    assert_eq!(fresh.view().0, "unsent draft 中文");
    assert!(!path.exists(), "draft file must be consumed");

    assert!(!fresh.recover_draft(&path));
    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn editor_empty_draft_file_does_not_restore() {
    let dir = std::env::temp_dir().join(format!("vak-tui-draft-empty-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("composer_draft.txt");
    std::fs::write(&path, "   \n").unwrap();
    let mut e = Editor::new();
    assert!(!e.recover_draft(&path));
    assert_eq!(e.view().0, "");
    assert!(!path.exists());
    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn key_describe_renders_literals() {
    assert_eq!(describe(KeyCode::Char('x'), KeyModifiers::empty()), "x");
    assert_eq!(
        describe(KeyCode::Char('a'), KeyModifiers::CONTROL),
        "Ctrl-Char 'a'"
    );
    assert_eq!(describe(KeyCode::Enter, KeyModifiers::ALT), "Alt-Enter");
    assert_eq!(
        describe(KeyCode::Char('A'), KeyModifiers::SHIFT),
        "Char 'A'"
    );
    assert_eq!(describe(KeyCode::F(3), KeyModifiers::NONE), "F3");
}

#[test]
fn editor_undo_redo_coalesces_typing_and_preserves_cursor() {
    let mut e = Editor::new();
    type_str(&mut e, "hello");
    e.undo();
    assert_eq!(e.view(), ("", 0));
    e.redo();
    assert_eq!(e.view(), ("hello", 5));

    e.left();
    e.left();
    e.insert('X');
    assert_eq!(e.view(), ("helXlo", 4));
    e.undo();
    assert_eq!(e.view(), ("hello", 3));
    e.redo();
    assert_eq!(e.view(), ("helXlo", 4));
}

#[test]
fn editor_new_edit_invalidates_redo() {
    let mut e = Editor::new();
    type_str(&mut e, "abc");
    e.undo();
    e.insert('x');
    e.redo();
    assert_eq!(e.view(), ("x", 1));
}

#[test]
fn editor_delete_word_forward_is_unicode_safe_and_undoable() {
    let mut e = Editor::new();
    type_str(&mut e, "one  中文 three");
    e.home();
    e.word_right();
    e.delete_word_forward();
    assert_eq!(e.view(), ("one three", 3));
    e.undo();
    assert_eq!(e.view(), ("one  中文 three", 3));
}

#[test]
fn modern_editing_keys_map_to_actions() {
    assert!(matches!(
        map_key(KeyCode::Char('z'), KeyModifiers::CONTROL, false),
        Action::Undo
    ));
    assert!(matches!(
        map_key(KeyCode::Char('z'), KeyModifiers::ALT, false),
        Action::Redo
    ));
    assert!(matches!(
        map_key(KeyCode::Char('d'), KeyModifiers::ALT, false),
        Action::DeleteWordForward
    ));
    assert!(matches!(
        map_key(KeyCode::Char('t'), KeyModifiers::CONTROL, true),
        Action::ToggleThinking
    ));
    assert!(matches!(
        map_key(KeyCode::Char('p'), KeyModifiers::CONTROL, false),
        Action::CommandPalette
    ));
    assert!(matches!(
        map_key(KeyCode::Char('a'), KeyModifiers::CONTROL, false),
        Action::Home
    ));
    assert!(matches!(
        map_key(KeyCode::Char('e'), KeyModifiers::CONTROL, false),
        Action::End
    ));
    assert!(matches!(
        map_key(KeyCode::Char('k'), KeyModifiers::CONTROL, false),
        Action::DeleteToLineEnd
    ));
    assert!(matches!(
        map_key(KeyCode::Char('l'), KeyModifiers::CONTROL, false),
        Action::ClearViewport
    ));
    assert!(matches!(
        map_key(KeyCode::Char('a'), KeyModifiers::ALT, true),
        Action::OpenApproval
    ));
}

#[test]
fn command_palette_fuzzy_filters_and_wraps_selection() {
    let mut palette = CommandPalette::new(Vec::new());
    palette.push('t');
    palette.push('r');
    let items = palette.items();
    assert_eq!(items[0].name, "transcript");
    assert!(items.iter().all(|item| item.name.contains('t')));

    palette.up(items.len());
    assert_eq!(palette.selected(items.len()), items.len() - 1);
    palette.down(items.len());
    assert_eq!(palette.selected(items.len()), 0);
    palette.backspace();
    assert_eq!(palette.query(), "t");
}

#[test]
fn editor_history_save_load_roundtrip() {
    let dir = tempfile::tempdir().expect("tempdir");
    let path = dir.path().join("hist").join("history.txt");
    let mut e = Editor::new();
    type_str(&mut e, "alpha");
    let _ = e.take();
    type_str(&mut e, "beta");
    let _ = e.take();
    type_str(&mut e, "beta");
    let _ = e.take();
    e.save_history(&path);
    assert!(path.exists());

    let mut loaded = Editor::new();
    loaded.load_history(&path);
    loaded.history_prev();
    assert_eq!(loaded.view().0, "beta");
    loaded.history_prev();
    assert_eq!(loaded.view().0, "alpha");
    loaded.history_next();
    loaded.history_next();
    assert_eq!(loaded.view().0, "", "returns to empty draft");

    let manual = dir.path().join("manual.txt");
    fs::write(&manual, "\nfirst\n\nsecond\nsecond\n").expect("write fixture");
    let mut e2 = Editor::new();
    e2.load_history(&manual);
    e2.history_prev();
    e2.history_prev();
    e2.history_prev();
    assert_eq!(e2.view().0, "first", "empty lines and dup-of-last skipped");

    let big = dir.path().join("big.txt");
    let content: String = (0..600).map(|i| format!("entry-{i}\n")).collect();
    fs::write(&big, content).expect("write big fixture");
    let mut e3 = Editor::new();
    e3.load_history(&big);
    let capped = dir.path().join("capped.txt");
    e3.save_history(&capped);
    let saved = fs::read_to_string(&capped).expect("read capped");
    assert_eq!(saved.lines().count(), 500);
    assert_eq!(saved.lines().next(), Some("entry-100"), "keeps newest 500");
}

#[test]
fn tab_alt_enter_and_ctrl_j_mappings() {
    assert!(matches!(
        map_key(KeyCode::Tab, KeyModifiers::NONE, false),
        Action::Complete
    ));
    assert!(matches!(
        map_key(KeyCode::Enter, KeyModifiers::ALT, false),
        Action::Insert('\n')
    ));
    assert!(matches!(
        map_key(KeyCode::Char('j'), KeyModifiers::CONTROL, false),
        Action::Insert('\n')
    ));
    assert!(matches!(
        map_key(KeyCode::Enter, KeyModifiers::NONE, false),
        Action::Submit,
    ));
}

#[test]
fn complete_command_names_by_prefix() {
    let cmds = [
        ("help", "show help"),
        ("history", "past prompts"),
        ("exit", "quit"),
    ];
    let out = complete("/h", &cmds, Path::new("."));
    assert_eq!(out.len(), 2);
    assert_eq!(out[0].replace, "/help ");
    assert_eq!(out[0].hint, "show help");
    assert_eq!(out[1].replace, "/history ");
    assert_eq!(out[1].hint, "past prompts");

    let all = complete("/", &cmds, Path::new("."));
    assert_eq!(all.len(), 3);

    assert!(complete("plain words", &cmds, Path::new(".")).is_empty());
    assert!(complete("", &cmds, Path::new(".")).is_empty());
}

#[test]
fn complete_at_paths_in_temp_fixture() {
    let dir = tempfile::tempdir().expect("tempdir");
    let root = dir.path();
    fs::create_dir_all(root.join("src/deep")).expect("mkdirs");
    fs::write(root.join("src/main.rs"), "fn main() {}\n").expect("write");
    fs::write(root.join("README.md"), "# t\n").expect("write");
    fs::create_dir(root.join(".hidden")).expect("mkdir");
    fs::write(root.join(".secret"), "").expect("write");
    fs::create_dir(root.join("target")).expect("mkdir");
    fs::write(root.join("target/out.bin"), "").expect("write");

    let out = complete("run @sr", &[], root);
    assert_eq!(out.len(), 3);
    assert_eq!(
        out.iter().map(|s| s.replace.as_str()).collect::<Vec<_>>(),
        ["@src/", "@src/deep/", "@src/main.rs"],
        "dirs-first; descendants share the rel-path prefix"
    );
    assert_eq!(out[0].hint, "dir");

    let under = complete("run @src/", &[], root);
    assert_eq!(under.len(), 2);
    assert_eq!(under[0].replace, "@src/deep/");
    assert_eq!(under[0].hint, "dir");
    assert_eq!(under[1].replace, "@src/main.rs");
    assert_eq!(under[1].hint, "file");

    let bare = complete("@", &[], root);
    assert_eq!(
        bare.iter().map(|s| s.replace.as_str()).collect::<Vec<_>>(),
        ["@src/", "@src/deep/", "@README.md", "@src/main.rs"],
        "dirs-first then alpha; dotfiles and skip-list dirs excluded"
    );
}

#[test]
fn complete_dotfiles_only_when_prefix_starts_with_dot() {
    let dir = tempfile::tempdir().expect("tempdir");
    let root = dir.path();
    fs::create_dir(root.join(".hidden")).expect("mkdir");
    fs::write(root.join(".secret"), "").expect("write");
    fs::write(root.join("visible.txt"), "").expect("write");

    let no_dot = complete("@v", &[], root);
    assert_eq!(no_dot.len(), 1);
    assert_eq!(no_dot[0].replace, "@visible.txt");

    let with_dot = complete("@.", &[], root);
    assert_eq!(with_dot.len(), 2);
    assert_eq!(with_dot[0].replace, "@.hidden/");
    assert_eq!(with_dot[0].hint, "dir");
    assert_eq!(with_dot[1].replace, "@.secret");
}

#[test]
fn complete_caps_results_at_50() {
    let dir = tempfile::tempdir().expect("tempdir");
    for i in 0..80 {
        fs::write(dir.path().join(format!("f{i:03}.txt")), "").expect("write");
    }
    let out = complete("@f", &[], dir.path());
    assert_eq!(out.len(), 50);
    assert_eq!(out.first().expect("nonempty").replace, "@f000.txt");
    assert_eq!(out.last().expect("nonempty").replace, "@f049.txt");
    assert!(out.windows(2).all(|w| w[0].replace < w[1].replace));
}

#[test]
fn reverse_search_finds_refines_accepts_and_cancels() {
    let mut e = Editor::new();
    e.take();
    e.insert('d');
    e.insert('e');
    e.insert('p');
    e.insert(' ');
    e.insert('o');
    e.insert('n');
    e.insert('e');
    assert_eq!(e.take(), "dep one");
    e.insert('t');
    e.insert('w');
    e.insert('o');
    assert_eq!(e.take(), "two");
    e.insert('z');
    e.insert('z');
    e.insert('z');
    assert_eq!(e.take(), "zzz");

    e.begin_search();
    assert!(e.search_active());
    e.search_push('o');
    assert!(e.search_query().contains("o"));
    // newest match containing "o" is "two"
    assert_eq!(e.view().0, "two");
    e.search_next();
    // next older is "dep one"
    assert_eq!(e.view().0, "dep one");

    // refine: "on" still matches "dep one"
    e.search_push('n');
    assert_eq!(e.view().0, "dep one");
    // refine to something unmatched: buffer stays on last match
    e.search_push('q');
    assert_eq!(e.view().0, "dep one");

    e.accept_search();
    assert!(!e.search_active());
    assert_eq!(e.view().0, "dep one");

    e.begin_search();
    e.cancel_search();
    assert!(!e.search_active());
}

#[test]
fn search_cancel_restores_buffer_being_edited() {
    let mut e = Editor::new();
    e.insert('h');
    e.insert('i');
    e.begin_search();
    e.search_push('h');
    e.search_push('i');
    e.cancel_search();
    assert_eq!(e.view().0, "hi");
    assert_eq!(e.view().1, 2);
}

#[test]
fn learned_spec_scopes_calls_and_rejects_unscopeable() {
    // bash: scoped to first word
    let s = vak_tui::app::learned_spec("bash", r#"{"command":"cargo test --lib"}"#);
    assert_eq!(s.as_deref(), Some("bash(cargo *)"));

    // opaque bash (command substitution) cannot be scoped safely
    assert_eq!(
        vak_tui::app::learned_spec("bash", r#"{"command":"echo $(rm -rf /)"}"#),
        None
    );

    // file tools: exact-path scope that must round-trip match the call
    let args = r#"{"path":"src/lib.rs","old_string":"a","new_string":"b"}"#;
    let s = vak_tui::app::learned_spec("edit", args);
    assert_eq!(s.as_deref(), Some("edit(src/lib.rs)"));

    // mcp: server-scoped, only for calls
    assert_eq!(
        vak_tui::app::learned_spec("mcp", r#"{"action":"call","server":"gh","tool":"pr"}"#),
        Some("mcp(gh/*)".to_string())
    );
    assert_eq!(
        vak_tui::app::learned_spec("mcp", r#"{"action":"list"}"#),
        None
    );

    // unknown tools get no persisted rule
    assert_eq!(vak_tui::app::learned_spec("mystery", "{}"), None);

    // round-trip: the derived bash rule actually matches a sibling command
    let rule = vak_permission::Rule::parse("bash(cargo *)").expect("parses");
    assert!(rule.matches(
        "bash",
        &serde_json::json!({"command": "cargo build --release"})
    ));
}

#[test]
fn edit_diff_text_accepts_both_arg_shapes() {
    let theme = vak_tui::theme::Theme::from_name("plain");
    let edits_shape =
        r#"{"path":"a.rs","edits":[{"old_string":"fn a() {}","new_string":"fn a() { b(); }"}]}"#;
    let flat_shape = r#"{"path":"a.rs","old_string":"fn a() {}","new_string":"fn a() { b(); }"}"#;
    for shape in [edits_shape, flat_shape] {
        let d = vak_tui::app::edit_diff_text(shape, &theme, 10).expect("diff");
        let plain = vak_tui::markdown::strip_ansi(&d);
        assert!(plain.contains("+ fn a() { b(); }"), "shape: {shape}");
        assert!(plain.contains("- fn a() {}"));
        assert!(plain.contains("@@"));
    }
    // nothing renderable -> None
    assert_eq!(
        vak_tui::app::edit_diff_text(r#"{"path":"a.rs"}"#, &theme, 10),
        None
    );
}
