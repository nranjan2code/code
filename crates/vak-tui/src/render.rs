use std::io::Write;

use crossterm::style::{Attribute, Color, Print, SetForegroundColor};
use crossterm::{execute, queue};

pub struct Screen {
    out: std::io::Stdout,
}

impl Default for Screen {
    fn default() -> Self {
        Self::new()
    }
}

impl Screen {
    pub fn new() -> Self {
        Screen {
            out: std::io::stdout(),
        }
    }

    pub fn line(&mut self, s: &str) {
        queue!(self.out, Print(s), Print("\r\n")).ok();
        self.out.flush().ok();
    }

    pub fn dim(&mut self, s: &str) {
        queue!(
            self.out,
            SetForegroundColor(Color::DarkGrey),
            Print(s),
            Print("\r\n"),
            SetForegroundColor(Color::Reset)
        )
        .ok();
        self.out.flush().ok();
    }

    pub fn accent(&mut self, s: &str) {
        queue!(
            self.out,
            SetForegroundColor(Color::Cyan),
            Print(s),
            Print("\r\n"),
            SetForegroundColor(Color::Reset)
        )
        .ok();
        self.out.flush().ok();
    }

    pub fn tool_line(&mut self, mark: &str, name: &str) {
        let color = if mark == "✓" {
            Color::DarkGreen
        } else {
            Color::DarkRed
        };
        queue!(
            self.out,
            SetForegroundColor(color),
            Print(format!("{mark} {name}\r\n")),
            SetForegroundColor(Color::Reset)
        )
        .ok();
        self.out.flush().ok();
    }

    pub fn inline(&mut self, s: &str) {
        queue!(self.out, Print(s)).ok();
        self.out.flush().ok();
    }

    pub fn redraw_input(&mut self, prompt: &str, buf: &str, cursor: usize) {
        let col = buf.chars().take(cursor).count() + prompt.chars().count();
        queue!(
            self.out,
            Print("\r"),
            Print("\x1b[2K"),
            Print(format!("{prompt}{buf}")),
            Print(format!("\x1b[{col}G"))
        )
        .ok();
        self.out.flush().ok();
    }

    pub fn clear_input_row(&mut self) {
        queue!(self.out, Print("\r\x1b[2K")).ok();
        self.out.flush().ok();
    }

    pub fn banner(&mut self, version: &str, model: &str, session: &str) {
        execute!(
            self.out,
            Print(format!(
                "{}{version}{} · {model} · session {session}\r\n",
                Attribute::Bold,
                Attribute::Reset
            )),
            Print("type a task · Enter sends · Ctrl-C cancels/exits · /help for commands\r\n\r\n")
        )
        .ok();
    }
}
