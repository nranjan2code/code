//! Multi-line REPL input composer, history navigation, and slash command autocomplete.

#[derive(Debug, Clone, Default)]
pub struct ReplComposer {
    pub buffer: String,
    pub cursor_pos: usize,
    pub history: Vec<String>,
    pub history_idx: Option<usize>,
    pub slash_palette_open: bool,
    pub selected_slash_cmd: usize,
    pub extra_commands: Vec<PaletteEntry>,
}

/// One row of the quick-action palette: a built-in, a custom command or a
/// skill (`/skill:name`, the server's deterministic invocation).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PaletteEntry {
    pub name: String,
    pub description: String,
}

pub struct SlashCommand {
    pub name: &'static str,
    pub description: &'static str,
}

pub const AVAILABLE_SLASH_COMMANDS: &[SlashCommand] = &[
    SlashCommand {
        name: "/theme",
        description: "Switch visual theme (Vakyartha Warm, Slate, Paper, Tokyo Night)",
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
        name: "/compact",
        description: "Summarise older turns to free up context window",
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

    pub fn matching_slash_commands(&self) -> Vec<PaletteEntry> {
        if !self.slash_palette_open {
            return Vec::new();
        }
        AVAILABLE_SLASH_COMMANDS
            .iter()
            .map(|cmd| PaletteEntry {
                name: cmd.name.to_string(),
                description: cmd.description.to_string(),
            })
            .chain(self.extra_commands.iter().cloned())
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
        if let Some(cmd) = matches.get(self.selected_slash_cmd).cloned() {
            self.buffer = format!("{} ", cmd.name);
            self.cursor_pos = self.buffer.len();
            self.slash_palette_open = false;
        }
    }
}

#[cfg(test)]
mod palette_tests {
    use super::*;

    fn composer_with(buffer: &str) -> ReplComposer {
        let mut composer = ReplComposer::new();
        composer.extra_commands = vec![
            PaletteEntry {
                name: "/oneline".into(),
                description: "Summarise".into(),
            },
            PaletteEntry {
                name: "/skill:debugging".into(),
                description: "Debug".into(),
            },
        ];
        for c in buffer.chars() {
            composer.insert_char(c);
        }
        composer
    }

    #[test]
    fn custom_commands_and_skills_join_the_builtins() {
        let names = |buffer: &str| -> Vec<String> {
            composer_with(buffer)
                .matching_slash_commands()
                .into_iter()
                .map(|e| e.name)
                .collect()
        };
        assert!(names("/").contains(&"/theme".to_string()));
        assert!(names("/").contains(&"/oneline".to_string()));
        assert_eq!(names("/skill:d"), vec!["/skill:debugging".to_string()]);
        assert_eq!(names("/one"), vec!["/oneline".to_string()]);
    }

    #[test]
    fn plain_text_and_paths_open_no_palette() {
        assert!(
            composer_with("read @notes.txt")
                .matching_slash_commands()
                .is_empty()
        );
        assert!(
            composer_with("/usr/bin")
                .matching_slash_commands()
                .is_empty()
        );
    }

    #[test]
    fn completing_inserts_the_full_name_and_a_space() {
        let mut composer = composer_with("/skill:d");
        composer.complete_selected_slash();
        assert_eq!(composer.buffer, "/skill:debugging ");
        assert!(!composer.slash_palette_open);
    }
}
