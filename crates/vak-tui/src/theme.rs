use crossterm::style::Color;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Theme {
    pub accent: Color,
    pub dim: Color,
    pub success: Color,
    pub error: Color,
    pub warning: Color,
    pub heading: Color,
    pub code: Color,
    pub user: Color,
    pub spinner: Color,
}

impl Theme {
    pub fn from_name(name: &str) -> Theme {
        match name {
            "light" => Theme {
                accent: Color::Blue,
                dim: Color::AnsiValue(245),
                success: Color::DarkGreen,
                error: Color::Red,
                warning: Color::DarkYellow,
                heading: Color::Black,
                code: Color::DarkMagenta,
                user: Color::Black,
                spinner: Color::Blue,
            },
            "plain" => Theme {
                accent: Color::Reset,
                dim: Color::Reset,
                success: Color::Reset,
                error: Color::Reset,
                warning: Color::Reset,
                heading: Color::Reset,
                code: Color::Reset,
                user: Color::Reset,
                spinner: Color::Reset,
            },
            _ => Theme {
                accent: Color::Cyan,
                dim: Color::DarkGrey,
                success: Color::DarkGreen,
                error: Color::Red,
                warning: Color::Yellow,
                heading: Color::White,
                code: Color::Magenta,
                user: Color::White,
                spinner: Color::Cyan,
            },
        }
    }
}

pub fn names() -> &'static [&'static str] {
    &["dark", "light", "plain"]
}

/// Deterministic SGR foreground sequence for a color. Hand-rolled instead of
/// crossterm's `Display` so output never depends on `NO_COLOR` or terminal
/// capability detection: styling decisions belong to the caller.
pub fn fg(color: Color) -> String {
    match color {
        Color::Reset => "\x1b[39m".to_string(),
        Color::Black => "\x1b[30m".to_string(),
        Color::DarkGrey => "\x1b[90m".to_string(),
        Color::Red => "\x1b[91m".to_string(),
        Color::DarkRed => "\x1b[31m".to_string(),
        Color::Green => "\x1b[92m".to_string(),
        Color::DarkGreen => "\x1b[32m".to_string(),
        Color::Yellow => "\x1b[93m".to_string(),
        Color::DarkYellow => "\x1b[33m".to_string(),
        Color::Blue => "\x1b[94m".to_string(),
        Color::DarkBlue => "\x1b[34m".to_string(),
        Color::Magenta => "\x1b[95m".to_string(),
        Color::DarkMagenta => "\x1b[35m".to_string(),
        Color::Cyan => "\x1b[96m".to_string(),
        Color::DarkCyan => "\x1b[36m".to_string(),
        Color::White => "\x1b[97m".to_string(),
        Color::Grey => "\x1b[37m".to_string(),
        Color::Rgb { r, g, b } => format!("\x1b[38;2;{r};{g};{b}m"),
        Color::AnsiValue(n) => format!("\x1b[38;5;{n}m"),
    }
}
