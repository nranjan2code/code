use crossterm::event::{KeyCode, KeyModifiers};

pub enum Action {
    Insert(char),
    Backspace,
    Delete,
    Left,
    Right,
    Home,
    End,
    Submit,
    HistoryPrev,
    HistoryNext,
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
        KeyCode::Esc => {
            if running {
                Action::CancelOrClear
            } else {
                Action::Ignore
            }
        }
        KeyCode::Enter => Action::Submit,
        KeyCode::Backspace => Action::Backspace,
        KeyCode::Delete => Action::Delete,
        KeyCode::Left => Action::Left,
        KeyCode::Right => Action::Right,
        KeyCode::Home => Action::Home,
        KeyCode::End => Action::End,
        KeyCode::Up => Action::HistoryPrev,
        KeyCode::Down => Action::HistoryNext,
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
