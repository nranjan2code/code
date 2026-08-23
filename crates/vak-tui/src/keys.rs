use crossterm::event::{KeyCode, KeyModifiers};

impl Action {
    /// Stable identifier used by keymap configuration and `/keymap` display.
    pub fn name(&self) -> &'static str {
        match self {
            Action::Insert(_) => "insert",
            Action::Backspace => "backspace",
            Action::Delete => "delete",
            Action::Left => "left",
            Action::Right => "right",
            Action::WordLeft => "word-left",
            Action::WordRight => "word-right",
            Action::DeleteWordBack => "delete-word-back",
            Action::DeleteWordForward => "delete-word-forward",
            Action::DeleteToLineEnd => "delete-to-line-end",
            Action::Undo => "undo",
            Action::Redo => "redo",
            Action::ToggleThinking => "toggle-thinking",
            Action::CommandPalette => "command-palette",
            Action::ClearViewport => "clear-viewport",
            Action::OpenApproval => "open-approval",
            Action::ExpandStash => "expand-stash",
            Action::ExternalEditor => "external-editor",
            Action::ClearLine => "clear-line",
            Action::Home => "home",
            Action::End => "end",
            Action::Submit => "submit",
            Action::HistoryPrev => "history-prev",
            Action::HistoryNext => "history-next",
            Action::HistorySearch => "history-search",
            Action::Complete => "complete",
            Action::Queue => "queue",
            Action::Interrupt => "interrupt",
            Action::CancelOrClear => "cancel-or-clear",
            Action::CopyResponse => "copy-response",
            Action::Subagents => "subagents",
            Action::Exit => "exit",
            Action::Ignore => "ignore",
        }
    }

    pub fn parse(name: &str) -> Option<Action> {
        Some(match name {
            "insert" | "self-insert" => return None,
            "backspace" => Action::Backspace,
            "delete" => Action::Delete,
            "left" => Action::Left,
            "right" => Action::Right,
            "word-left" => Action::WordLeft,
            "word-right" => Action::WordRight,
            "delete-word-back" => Action::DeleteWordBack,
            "delete-word-forward" => Action::DeleteWordForward,
            "delete-to-line-end" => Action::DeleteToLineEnd,
            "undo" => Action::Undo,
            "redo" => Action::Redo,
            "toggle-thinking" => Action::ToggleThinking,
            "command-palette" => Action::CommandPalette,
            "clear-viewport" => Action::ClearViewport,
            "open-approval" => Action::OpenApproval,
            "expand-stash" => Action::ExpandStash,
            "external-editor" => Action::ExternalEditor,
            "clear-line" => Action::ClearLine,
            "home" => Action::Home,
            "end" => Action::End,
            "submit" => Action::Submit,
            "history-prev" => Action::HistoryPrev,
            "history-next" => Action::HistoryNext,
            "history-search" => Action::HistorySearch,
            "complete" => Action::Complete,
            "queue" => Action::Queue,
            "interrupt" => Action::Interrupt,
            "cancel-or-clear" => Action::CancelOrClear,
            "copy-response" => Action::CopyResponse,
            "subagents" => Action::Subagents,
            "exit" => Action::Exit,
            "ignore" => Action::Ignore,
            _ => return None,
        })
    }
}

/// Renders a key event as a human-readable literal, for `/keys raw`
/// diagnostics: `Ctrl-Shift-A`, `Alt-Enter`, `F3`, `BackTab`, `Char 'x'`.
pub fn describe(code: KeyCode, mods: KeyModifiers) -> String {
    let mut out = String::new();
    if mods.contains(KeyModifiers::CONTROL) {
        out.push_str("Ctrl-");
    }
    if mods.contains(KeyModifiers::ALT) {
        out.push_str("Alt-");
    }
    if mods.contains(KeyModifiers::SHIFT) && !matches!(code, KeyCode::Char(_) | KeyCode::BackTab) {
        out.push_str("Shift-");
    }
    match code {
        KeyCode::Char(c) => {
            if mods.contains(KeyModifiers::CONTROL)
                || mods.contains(KeyModifiers::ALT)
                || c.is_uppercase()
            {
                out.push_str(&format!("Char '{c}'"));
            } else {
                out.push(c);
            }
        }
        KeyCode::F(n) => out.push_str(&format!("F{n}")),
        other => out.push_str(&format!("{other:?}")),
    }
    out
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
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
    ExpandStash,
    ExternalEditor,
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
    CopyResponse,
    Subagents,
    Exit,
    Ignore,
}

pub fn map_key(code: KeyCode, mods: KeyModifiers, running: bool) -> Action {
    static DEFAULT: std::sync::OnceLock<crate::keymap::Keymap> = std::sync::OnceLock::new();
    DEFAULT
        .get_or_init(crate::keymap::Keymap::default)
        .action_for(running, code, mods)
}
