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
    pub panel_bg: Color,
    pub selected_bg: Color,
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
                panel_bg: Color::AnsiValue(255),
                selected_bg: Color::AnsiValue(153),
            },
            "neo" => Theme {
                accent: Color::AnsiValue(51),
                dim: Color::AnsiValue(245),
                success: Color::AnsiValue(82),
                error: Color::AnsiValue(197),
                warning: Color::AnsiValue(226),
                heading: Color::AnsiValue(231),
                code: Color::AnsiValue(213),
                user: Color::AnsiValue(51),
                spinner: Color::AnsiValue(201),
                panel_bg: Color::AnsiValue(233),
                selected_bg: Color::AnsiValue(54),
            },
            "rich" => Theme {
                accent: Color::AnsiValue(214),
                dim: Color::AnsiValue(244),
                success: Color::AnsiValue(42),
                error: Color::AnsiValue(203),
                warning: Color::AnsiValue(220),
                heading: Color::AnsiValue(230),
                code: Color::AnsiValue(141),
                user: Color::AnsiValue(81),
                spinner: Color::AnsiValue(214),
                panel_bg: Color::AnsiValue(234),
                selected_bg: Color::AnsiValue(94),
            },
            "teenage" => Theme {
                accent: Color::AnsiValue(208),
                dim: Color::AnsiValue(240),
                success: Color::AnsiValue(64),
                error: Color::AnsiValue(160),
                warning: Color::AnsiValue(166),
                heading: Color::Black,
                code: Color::AnsiValue(125),
                user: Color::AnsiValue(17),
                spinner: Color::AnsiValue(208),
                panel_bg: Color::AnsiValue(230),
                selected_bg: Color::AnsiValue(208),
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
                panel_bg: Color::Reset,
                selected_bg: Color::Reset,
            },
            "midnight" => Theme {
                accent: Color::Rgb {
                    r: 130,
                    g: 170,
                    b: 255,
                },
                dim: Color::Rgb {
                    r: 96,
                    g: 104,
                    b: 124,
                },
                success: Color::Rgb {
                    r: 94,
                    g: 220,
                    b: 148,
                },
                error: Color::Rgb {
                    r: 255,
                    g: 105,
                    b: 110,
                },
                warning: Color::Rgb {
                    r: 240,
                    g: 190,
                    b: 90,
                },
                heading: Color::Rgb {
                    r: 224,
                    g: 230,
                    b: 245,
                },
                code: Color::Rgb {
                    r: 178,
                    g: 140,
                    b: 255,
                },
                user: Color::Rgb {
                    r: 224,
                    g: 230,
                    b: 245,
                },
                spinner: Color::Rgb {
                    r: 130,
                    g: 170,
                    b: 255,
                },
                panel_bg: Color::Rgb {
                    r: 16,
                    g: 20,
                    b: 34,
                },
                selected_bg: Color::Rgb {
                    r: 42,
                    g: 52,
                    b: 84,
                },
            },
            "synthwave" => Theme {
                accent: Color::Rgb {
                    r: 255,
                    g: 94,
                    b: 190,
                },
                dim: Color::Rgb {
                    r: 122,
                    g: 112,
                    b: 168,
                },
                success: Color::Rgb {
                    r: 62,
                    g: 225,
                    b: 190,
                },
                error: Color::Rgb {
                    r: 255,
                    g: 92,
                    b: 92,
                },
                warning: Color::Rgb {
                    r: 251,
                    g: 191,
                    b: 82,
                },
                heading: Color::Rgb {
                    r: 236,
                    g: 226,
                    b: 255,
                },
                code: Color::Rgb {
                    r: 97,
                    g: 214,
                    b: 255,
                },
                user: Color::Rgb {
                    r: 236,
                    g: 226,
                    b: 255,
                },
                spinner: Color::Rgb {
                    r: 255,
                    g: 94,
                    b: 190,
                },
                panel_bg: Color::Rgb {
                    r: 30,
                    g: 22,
                    b: 48,
                },
                selected_bg: Color::Rgb {
                    r: 74,
                    g: 40,
                    b: 96,
                },
            },
            "forest" => Theme {
                accent: Color::Rgb {
                    r: 126,
                    g: 200,
                    b: 116,
                },
                dim: Color::Rgb {
                    r: 108,
                    g: 118,
                    b: 98,
                },
                success: Color::Rgb {
                    r: 88,
                    g: 204,
                    b: 96,
                },
                error: Color::Rgb {
                    r: 222,
                    g: 100,
                    b: 74,
                },
                warning: Color::Rgb {
                    r: 216,
                    g: 172,
                    b: 76,
                },
                heading: Color::Rgb {
                    r: 232,
                    g: 238,
                    b: 222,
                },
                code: Color::Rgb {
                    r: 156,
                    g: 220,
                    b: 190,
                },
                user: Color::Rgb {
                    r: 232,
                    g: 238,
                    b: 222,
                },
                spinner: Color::Rgb {
                    r: 126,
                    g: 200,
                    b: 116,
                },
                panel_bg: Color::Rgb {
                    r: 24,
                    g: 32,
                    b: 26,
                },
                selected_bg: Color::Rgb {
                    r: 46,
                    g: 66,
                    b: 50,
                },
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
                panel_bg: Color::AnsiValue(235),
                selected_bg: Color::AnsiValue(24),
            },
        }
    }
}

pub fn names() -> &'static [&'static str] {
    &[
        "dark",
        "light",
        "neo",
        "rich",
        "teenage",
        "plain",
        "midnight",
        "synthwave",
        "forest",
    ]
}

/// True when `name` is one of the built-in packs.
pub fn builtin(name: &str) -> bool {
    names().contains(&name)
}

/// Every selectable theme: built-ins plus `[ui.themes.<name>]` customs.
pub fn all_names(custom: &std::collections::BTreeMap<String, ThemeColors>) -> Vec<String> {
    let mut out: Vec<String> = names().iter().map(|s| (*s).to_string()).collect();
    out.extend(custom.keys().cloned());
    out
}

pub type ThemeColors = std::collections::BTreeMap<String, String>;

/// The color slots a custom theme may define.
pub const THEME_COLOR_KEYS: &[&str] = &[
    "accent",
    "dim",
    "success",
    "error",
    "warning",
    "heading",
    "code",
    "user",
    "spinner",
    "panel_bg",
    "selected_bg",
];

/// Parses `#rgb`, `#rrggbb`, crossterm color names, or `default`/`none`.
pub fn parse_color(s: &str) -> Option<Color> {
    let s = s.trim();
    if let Some(hex) = s.strip_prefix('#') {
        let hex = hex.to_ascii_lowercase();
        return match hex.len() {
            3 => {
                let r = u8::from_str_radix(&hex[0..1].repeat(2), 16).ok()?;
                let g = u8::from_str_radix(&hex[1..2].repeat(2), 16).ok()?;
                let b = u8::from_str_radix(&hex[2..3].repeat(2), 16).ok()?;
                Some(Color::Rgb { r, g, b })
            }
            6 => {
                let r = u8::from_str_radix(&hex[0..2], 16).ok()?;
                let g = u8::from_str_radix(&hex[2..4], 16).ok()?;
                let b = u8::from_str_radix(&hex[4..6], 16).ok()?;
                Some(Color::Rgb { r, g, b })
            }
            _ => None,
        };
    }
    match s.to_ascii_lowercase().as_str() {
        "default" | "none" | "reset" => Some(Color::Reset),
        "black" => Some(Color::Black),
        "darkgrey" | "dark-grey" => Some(Color::DarkGrey),
        "red" => Some(Color::Red),
        "darkred" | "dark-red" => Some(Color::DarkRed),
        "green" => Some(Color::Green),
        "darkgreen" | "dark-green" => Some(Color::DarkGreen),
        "yellow" => Some(Color::Yellow),
        "darkyellow" | "dark-yellow" => Some(Color::DarkYellow),
        "blue" => Some(Color::Blue),
        "darkblue" | "dark-blue" => Some(Color::DarkBlue),
        "magenta" => Some(Color::Magenta),
        "darkmagenta" | "dark-magenta" => Some(Color::DarkMagenta),
        "cyan" => Some(Color::Cyan),
        "darkcyan" | "dark-cyan" => Some(Color::DarkCyan),
        "white" => Some(Color::White),
        "grey" | "gray" => Some(Color::Grey),
        _ => None,
    }
}

/// Resolves a theme by name against the built-in packs and the user's
/// `[ui.themes]` definitions (custom colors overlay the dark base).
/// Unknown names fall back to dark.
pub fn resolve(name: &str, custom: &std::collections::BTreeMap<String, ThemeColors>) -> Theme {
    if let Some(colors) = custom.get(name) {
        let mut theme = Theme::from_name("dark");
        for (key, value) in colors {
            let Some(color) = parse_color(value) else {
                continue;
            };
            match key.as_str() {
                "accent" => theme.accent = color,
                "dim" => theme.dim = color,
                "success" => theme.success = color,
                "error" => theme.error = color,
                "warning" => theme.warning = color,
                "heading" => theme.heading = color,
                "code" => theme.code = color,
                "user" => theme.user = color,
                "spinner" => theme.spinner = color,
                "panel_bg" => theme.panel_bg = color,
                "selected_bg" => theme.selected_bg = color,
                _ => {}
            }
        }
        return theme;
    }
    Theme::from_name(name)
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

pub fn bg(color: Color) -> String {
    match color {
        Color::Reset => "\x1b[49m".to_string(),
        Color::Black => "\x1b[40m".to_string(),
        Color::DarkGrey => "\x1b[100m".to_string(),
        Color::Red => "\x1b[101m".to_string(),
        Color::DarkRed => "\x1b[41m".to_string(),
        Color::Green => "\x1b[102m".to_string(),
        Color::DarkGreen => "\x1b[42m".to_string(),
        Color::Yellow => "\x1b[103m".to_string(),
        Color::DarkYellow => "\x1b[43m".to_string(),
        Color::Blue => "\x1b[104m".to_string(),
        Color::DarkBlue => "\x1b[44m".to_string(),
        Color::Magenta => "\x1b[105m".to_string(),
        Color::DarkMagenta => "\x1b[45m".to_string(),
        Color::Cyan => "\x1b[106m".to_string(),
        Color::DarkCyan => "\x1b[46m".to_string(),
        Color::White => "\x1b[107m".to_string(),
        Color::Grey => "\x1b[47m".to_string(),
        Color::Rgb { r, g, b } => format!("\x1b[48;2;{r};{g};{b}m"),
        Color::AnsiValue(n) => format!("\x1b[48;5;{n}m"),
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;
    use std::collections::BTreeMap;

    #[test]
    fn parse_color_handles_hex_names_and_reset() {
        assert_eq!(
            parse_color("#ff5500"),
            Some(Color::Rgb {
                r: 255,
                g: 85,
                b: 0
            })
        );
        assert_eq!(
            parse_color("#F50"),
            Some(Color::Rgb {
                r: 255,
                g: 85,
                b: 0
            })
        );
        assert_eq!(parse_color("cyan"), Some(Color::Cyan));
        assert_eq!(parse_color("Dark-Grey"), Some(Color::DarkGrey));
        assert_eq!(parse_color("none"), Some(Color::Reset));
        assert_eq!(parse_color("#12345"), None);
        assert_eq!(parse_color("chartreuse"), None);
    }

    fn custom(name: &str, pairs: &[(&str, &str)]) -> BTreeMap<String, ThemeColors> {
        let mut out = BTreeMap::new();
        let mut colors = BTreeMap::new();
        for (k, v) in pairs {
            colors.insert((*k).to_string(), (*v).to_string());
        }
        out.insert(name.to_string(), colors);
        out
    }

    #[test]
    fn custom_theme_overlays_dark_base_and_lists_in_all_names() {
        let themes = custom("sunset", &[("accent", "#ff5500"), ("error", "red")]);
        let theme = resolve("sunset", &themes);
        assert_eq!(
            theme.accent,
            Color::Rgb {
                r: 255,
                g: 85,
                b: 0
            }
        );
        assert_eq!(theme.error, Color::Red);
        // Unspecified slots inherit the dark base.
        assert_eq!(theme.success, Theme::from_name("dark").success);

        let names = all_names(&themes);
        assert!(names.contains(&"sunset".to_string()));
        assert!(names.contains(&"dark".to_string()));
        // Unknown names fall back to dark rather than panicking.
        assert_eq!(resolve("nope", &themes), Theme::from_name("dark"));
    }

    #[test]
    fn truecolor_packs_use_rgb_and_resolve_by_name() {
        for name in ["midnight", "synthwave", "forest"] {
            let t = resolve(name, &BTreeMap::new());
            assert!(matches!(t.accent, Color::Rgb { .. }), "{name}");
        }
    }
}
