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
    DeleteWordForward,
    DeleteToLineEnd,
    Undo,
    Redo,
    ToggleThinking,
    CommandPalette,
    ClearViewport,
    OpenApproval,
    ClearLine,
    Home,
    End,
    Submit,
    HistoryPrev,
    HistoryNext,
    HistorySearch,
    Complete,
    Queue,
    Interrupt,
    CancelOrClear,
    Exit,
    Ignore,
}

pub fn map_key(code: KeyCode, mods: KeyModifiers, running: bool) -> Action {
    match code {
        KeyCode::Char('c') if mods.contains(KeyModifiers::CONTROL) => {
            if running {
                Action::Interrupt
            } else if mods.contains(KeyModifiers::SHIFT) {
                Action::Exit
            } else {
                Action::CancelOrClear
            }
        }
        KeyCode::Char('d') if mods.contains(KeyModifiers::CONTROL) => Action::Exit,
        KeyCode::Char('u') if mods.contains(KeyModifiers::CONTROL) => Action::ClearLine,
        KeyCode::Char('a') if mods.contains(KeyModifiers::CONTROL) => Action::Home,
        KeyCode::Char('e') if mods.contains(KeyModifiers::CONTROL) => Action::End,
        KeyCode::Char('k') if mods.contains(KeyModifiers::CONTROL) => Action::DeleteToLineEnd,
        KeyCode::Char('h') if mods.contains(KeyModifiers::CONTROL) => Action::Backspace,
        KeyCode::Char('l') if mods.contains(KeyModifiers::CONTROL) => Action::ClearViewport,
        KeyCode::Char('a') if mods.contains(KeyModifiers::ALT) => Action::OpenApproval,
        KeyCode::Char('n') if mods.contains(KeyModifiers::CONTROL) => Action::HistoryNext,
        KeyCode::Char('r') if mods.contains(KeyModifiers::CONTROL) => Action::HistorySearch,
        KeyCode::Char('w') if mods.contains(KeyModifiers::CONTROL) => Action::DeleteWordBack,
        KeyCode::Char('d') if mods.contains(KeyModifiers::ALT) => Action::DeleteWordForward,
        KeyCode::Char('z') if mods.contains(KeyModifiers::CONTROL) => Action::Undo,
        KeyCode::Char('z') if mods.contains(KeyModifiers::ALT) => Action::Redo,
        KeyCode::Char('t') if mods.contains(KeyModifiers::CONTROL) => Action::ToggleThinking,
        KeyCode::Char('p') if mods.contains(KeyModifiers::CONTROL) => Action::CommandPalette,
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
        KeyCode::Tab => {
            if running {
                Action::Queue
            } else {
                Action::Complete
            }
        }
        KeyCode::BackTab if running => Action::Queue,
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
