use std::io::Write;

use crossterm::style::{Attribute, Color, Print};
use crossterm::{execute, queue};

use crate::theme::Theme;
use crate::width::str_width;

pub struct Screen {
    out: std::io::Stdout,
    theme: Theme,
    input_rows: usize,
}

impl Screen {
    pub fn new(theme: Theme) -> Self {
        Screen {
            out: std::io::stdout(),
            theme,
            input_rows: 0,
        }
    }

    pub fn set_theme(&mut self, theme: Theme) {
        self.theme = theme;
    }

    fn put(&mut self, s: &str) {
        queue!(self.out, Print(s)).ok();
        self.out.flush().ok();
    }

    pub fn inline(&mut self, s: &str) {
        self.put(s);
    }

    pub fn line(&mut self, s: &str) {
        self.put(&format!("{s}\r\n"));
    }

    pub fn styled(&mut self, s: &str, color: Color) {
        self.put(&format!(
            "{}{s}{}\r\n",
            crate::theme::fg(color),
            crate::theme::fg(Color::Reset)
        ));
    }

    pub fn dim(&mut self, s: &str) {
        self.styled(s, self.theme.dim);
    }

    pub fn accent(&mut self, s: &str) {
        self.styled(s, self.theme.accent);
    }

    pub fn success(&mut self, s: &str) {
        self.styled(s, self.theme.success);
    }

    pub fn error(&mut self, s: &str) {
        self.styled(s, self.theme.error);
    }

    /// Prints a pre-styled string containing raw ANSI escapes as one line.
    pub fn md_line(&mut self, styled_ansi: &str) {
        self.put(&format!("{styled_ansi}\r\n"));
    }

    pub fn tool_start(&mut self, name: &str, args_hint: &str) {
        let hint = if args_hint.is_empty() {
            String::new()
        } else {
            format!(" {args_hint}")
        };
        self.put(&format!(
            "{}  ▸ {name}{hint}{}\r\n",
            crate::theme::fg(self.theme.dim),
            crate::theme::fg(Color::Reset)
        ));
    }

    pub fn tool_line(&mut self, mark: &str, name: &str, is_error: bool) {
        let color = if is_error {
            self.theme.error
        } else {
            self.theme.success
        };
        self.put(&format!(
            "{}{mark} {name}{}\r\n",
            crate::theme::fg(color),
            crate::theme::fg(Color::Reset)
        ));
    }

    fn cols(&self) -> usize {
        crossterm::terminal::size()
            .map(|(c, _)| c)
            .unwrap_or(80)
            .max(20) as usize
    }

    /// Erases every row occupied by the current input/status block.
    pub fn clear_input(&mut self) {
        let rows = self.input_rows.max(1);
        if rows > 1 {
            self.put(&format!("\x1b[{}A", rows - 1));
        }
        for i in 0..rows {
            self.put("\r\x1b[2K");
            if i + 1 < rows {
                self.put("\x1b[1B");
            }
        }
        self.input_rows = 0;
    }

    /// Renders the input block (multiline + wrap aware) and places the caret.
    /// `cursor` is a CHAR index into `buf`.
    pub fn redraw_input(&mut self, prompt: &str, buf: &str, cursor: usize) {
        let cols = self.cols();
        let pw = str_width(prompt);
        let indent: String = " ".repeat(pw);
        let lines: Vec<&str> = buf.split('\n').collect();

        self.clear_input();
        let mut rendered = String::new();
        for (i, l) in lines.iter().enumerate() {
            if i > 0 {
                rendered.push_str("\r\n");
            }
            rendered.push_str(if i == 0 { prompt } else { &indent });
            rendered.push_str(l);
        }
        self.put(&rendered);

        let mut consumed_chars = 0usize;
        let mut cursor_line = lines.len().saturating_sub(1);
        let mut cursor_col_chars = 0usize;
        for (i, l) in lines.iter().enumerate() {
            let len = l.chars().count();
            if cursor <= consumed_chars + len {
                cursor_line = i;
                cursor_col_chars = cursor - consumed_chars;
                break;
            }
            consumed_chars += len + 1;
        }
        let mut col_in_line_cells = 0usize;
        for c in lines[cursor_line].chars().take(cursor_col_chars) {
            col_in_line_cells += crate::width::char_width(c);
        }

        let rows_before: usize = lines
            .iter()
            .take(cursor_line)
            .map(|l| (pw + str_width(l)).div_ceil(cols).max(1))
            .sum();
        let total_rows: usize = lines
            .iter()
            .map(|l| (pw + str_width(l)).div_ceil(cols).max(1))
            .sum::<usize>()
            .max(1);
        let row = (rows_before * cols + pw + col_in_line_cells) / cols;
        let col = (pw + col_in_line_cells) % cols;

        if total_rows.saturating_sub(1) > row {
            self.put(&format!("\x1b[{}A", total_rows - 1 - row));
        }
        self.put(&format!("\r\x1b[{}G", col + 1));
        self.input_rows = total_rows;
    }

    /// Replaces the input block with a single pre-styled status row.
    pub fn redraw_status(&mut self, styled_ansi: &str) {
        self.clear_input();
        self.put(styled_ansi);
        self.input_rows = 1;
    }
    pub fn banner(&mut self, version: &str, model: &str, session: &str) {
        execute!(
            self.out,
            Print(format!(
                "{}{version}{} · {model} · session {session}\r\n",
                Attribute::Bold,
                Attribute::Reset
            )),
            Print("type a task · Enter sends · Alt/Ctrl-J newline · Ctrl-C cancel/exit · Tab complete · /help\r\n\r\n")
        )
        .ok();
    }

    pub fn bell(&mut self) {
        self.put("\x07");
    }

    pub fn set_title(&mut self, title: &str) {
        self.put(&format!("\x1b]2;{title}\x07"));
    }
}
