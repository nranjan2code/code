use crossterm::event::{KeyCode, KeyModifiers};

pub enum Action {
    Insert(char),
    Backspace,
    Delete,
    Left,
    Right,
    WordLeft,
    WordRight,
    DeleteWordBack,
    ClearLine,
    Home,
    End,
    Submit,
    HistoryPrev,
    HistoryNext,
    Complete,
    CancelOrClear,
    Exit,
    Ignore,
}

pub fn map_key(code: KeyCode, mods: KeyModifiers, running: bool) -> Action {
    match code {
        KeyCode::Char('c') if mods.contains(KeyModifiers::CONTROL) => {
            if running {
                Action::CancelOrClear
            } else if mods.contains(KeyModifiers::SHIFT) {
                Action::Exit
            } else {
                Action::CancelOrClear
            }
        }
        KeyCode::Char('d') if mods.contains(KeyModifiers::CONTROL) => Action::Exit,
        KeyCode::Char('u') if mods.contains(KeyModifiers::CONTROL) => Action::ClearLine,
        KeyCode::Char('w') if mods.contains(KeyModifiers::CONTROL) => Action::DeleteWordBack,
        KeyCode::Backspace if mods.contains(KeyModifiers::ALT) => Action::DeleteWordBack,
        KeyCode::Char('b') if mods.contains(KeyModifiers::ALT) => Action::WordLeft,
        KeyCode::Char('f') if mods.contains(KeyModifiers::ALT) => Action::WordRight,
        KeyCode::Char('j') if mods.contains(KeyModifiers::CONTROL) => Action::Insert('\n'),
        KeyCode::Esc => {
            if running {
                Action::CancelOrClear
            } else {
                Action::Ignore
            }
        }
        KeyCode::Enter if mods.contains(KeyModifiers::ALT) => Action::Insert('\n'),
        KeyCode::Enter => Action::Submit,
        KeyCode::Backspace => Action::Backspace,
        KeyCode::Delete => Action::Delete,
        KeyCode::Left => Action::Left,
        KeyCode::Right => Action::Right,
        KeyCode::Home => Action::Home,
        KeyCode::End => Action::End,
        KeyCode::Up => Action::HistoryPrev,
        KeyCode::Down => Action::HistoryNext,
        KeyCode::Tab => Action::Complete,
        KeyCode::Char(c) => {
            if mods.is_empty() || mods == KeyModifiers::SHIFT {
                Action::Insert(c)
            } else {
                Action::Ignore
            }
        }
        _ => Action::Ignore,
    }
}
