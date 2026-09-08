//! Multi-line REPL input composer, history navigation, and slash command autocomplete.

#[derive(Debug, Clone)]
pub struct ReplComposer {
    pub buffer: String,
    pub cursor_pos: usize,
    pub history: Vec<String>,
    pub history_idx: Option<usize>,
    pub slash_palette_open: bool,
    pub selected_slash_cmd: usize,
}

pub struct SlashCommand {
    pub name: &'static str,
    pub description: &'static str,
}

pub const AVAILABLE_SLASH_COMMANDS: &[SlashCommand] = &[
    SlashCommand {
        name: "/theme",
        description: "Switch visual theme (Vak Warm, Slate, Paper, Tokyo Night)",
    },
    SlashCommand {
        name: "/diff",
        description: "Toggle multi-file git diff inspector",
    },
    SlashCommand {
        name: "/preview",
        description: "Open live webpage/dev server preview",
    },
    SlashCommand {
        name: "/clear",
        description: "Clear stream scrollback buffer",
    },
    SlashCommand {
        name: "/model",
        description: "Inspect or switch active model route",
    },
    SlashCommand {
        name: "/ops",
        description: "Jump to Observability & Operations Deck",
    },
    SlashCommand {
        name: "/admin",
        description: "Jump to Remote Administration Cockpit",
    },
    SlashCommand {
        name: "/inbox",
        description: "Jump to Attention Inbox & Memory",
    },
    SlashCommand {
        name: "/hil",
        description: "Trigger Human-In-The-Loop approval modal",
    },
    SlashCommand {
        name: "/detach",
        description: "Detach from session (leave running on server)",
    },
    SlashCommand {
        name: "/help",
        description: "Show keyboard shortcuts & hotkeys",
    },
    SlashCommand {
        name: "/quit",
        description: "Exit terminal",
    },
];

impl Default for ReplComposer {
    fn default() -> Self {
        Self {
            buffer: String::new(),
            cursor_pos: 0,
            history: Vec::new(),
            history_idx: None,
            slash_palette_open: false,
            selected_slash_cmd: 0,
        }
    }
}

impl ReplComposer {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn insert_char(&mut self, c: char) {
        if self.cursor_pos >= self.buffer.len() {
            self.buffer.push(c);
        } else {
            self.buffer.insert(self.cursor_pos, c);
        }
        self.cursor_pos += c.len_utf8();
        self.update_slash_state();
    }

    pub fn delete_backspace(&mut self) {
        if self.cursor_pos > 0 && !self.buffer.is_empty() {
            // Find char boundary
            let mut prev = self.cursor_pos - 1;
            while prev > 0 && !self.buffer.is_char_boundary(prev) {
                prev -= 1;
            }
            self.buffer.remove(prev);
            self.cursor_pos = prev;
            self.update_slash_state();
        }
    }

    pub fn move_cursor_left(&mut self) {
        if self.cursor_pos > 0 {
            let mut prev = self.cursor_pos - 1;
            while prev > 0 && !self.buffer.is_char_boundary(prev) {
                prev -= 1;
            }
            self.cursor_pos = prev;
        }
    }

    pub fn move_cursor_right(&mut self) {
        if self.cursor_pos < self.buffer.len() {
            let mut next = self.cursor_pos + 1;
            while next < self.buffer.len() && !self.buffer.is_char_boundary(next) {
                next += 1;
            }
            self.cursor_pos = next;
        }
    }

    pub fn move_to_start(&mut self) {
        self.cursor_pos = 0;
    }

    pub fn move_to_end(&mut self) {
        self.cursor_pos = self.buffer.len();
    }

    pub fn insert_newline(&mut self) {
        self.insert_char('\n');
    }

    pub fn clear(&mut self) {
        self.buffer.clear();
        self.cursor_pos = 0;
        self.slash_palette_open = false;
        self.history_idx = None;
    }

    pub fn submit(&mut self) -> Option<String> {
        let trimmed = self.buffer.trim().to_string();
        if trimmed.is_empty() {
            return None;
        }
        self.history.push(trimmed.clone());
        self.clear();
        Some(trimmed)
    }

    pub fn history_up(&mut self) {
        if self.history.is_empty() {
            return;
        }
        let next_idx = match self.history_idx {
            None => self.history.len().saturating_sub(1),
            Some(i) if i > 0 => i - 1,
            Some(i) => i,
        };
        self.history_idx = Some(next_idx);
        if let Some(entry) = self.history.get(next_idx) {
            self.buffer = entry.clone();
            self.cursor_pos = self.buffer.len();
        }
    }

    pub fn history_down(&mut self) {
        if let Some(i) = self.history_idx {
            if i + 1 < self.history.len() {
                self.history_idx = Some(i + 1);
                if let Some(entry) = self.history.get(i + 1) {
                    self.buffer = entry.clone();
                    self.cursor_pos = self.buffer.len();
                }
            } else {
                self.history_idx = None;
                self.clear();
            }
        }
    }

    fn update_slash_state(&mut self) {
        self.slash_palette_open = self.buffer.starts_with('/');
        if !self.slash_palette_open {
            self.selected_slash_cmd = 0;
        }
    }

    pub fn matching_slash_commands(&self) -> Vec<&'static SlashCommand> {
        if !self.slash_palette_open {
            return Vec::new();
        }
        AVAILABLE_SLASH_COMMANDS
            .iter()
            .filter(|cmd| cmd.name.starts_with(&self.buffer))
            .collect()
    }

    pub fn select_next_slash(&mut self) {
        let count = self.matching_slash_commands().len();
        if count > 0 {
            self.selected_slash_cmd = (self.selected_slash_cmd + 1) % count;
        }
    }

    pub fn select_prev_slash(&mut self) {
        let count = self.matching_slash_commands().len();
        if count > 0 {
            self.selected_slash_cmd = (self.selected_slash_cmd + count - 1) % count;
        }
    }

    pub fn complete_selected_slash(&mut self) {
        let matches = self.matching_slash_commands();
        if let Some(cmd) = matches.get(self.selected_slash_cmd) {
            self.buffer = format!("{} ", cmd.name);
            self.cursor_pos = self.buffer.len();
            self.slash_palette_open = false;
        }
    }
}
