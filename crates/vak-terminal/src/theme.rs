//! 24-bit TrueColor theme definitions and minimalist typography contract
//! for `vak term` (docs/design/55-rich-terminal-surface.md).

use ratatui::style::{Color, Modifier, Style};

/// Minimalist symbols (no emojis, clean engineering glyphs).
pub struct Symbols;

impl Symbols {
    pub const CARET_EXPANDED: &'static str = "▾";
    pub const CARET_COLLAPSED: &'static str = "▸";
    pub const PROMPT_CHEVRON: &'static str = "❯";
    pub const EXTERNAL_LINK: &'static str = "↗";
    pub const STATUS_ACTIVE: &'static str = "●";
    pub const STATUS_IDLE: &'static str = "○";
    pub const CHECK: &'static str = "✓";
    pub const CROSS: &'static str = "✗";
    pub const DELTA: &'static str = "Δ";
    pub const BULLET: &'static str = "•";
    pub const WAVE: &'static str = "∿";
    pub const SPINNER_FRAMES: &'static [&'static str] =
        &["⠋", "⠙", "⠹", "⠸", "⠼", "⠴", "⠦", "⠧", "⠇", "⠏"];
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ThemeKind {
    #[default]
    VakWarm,
    VakSlate,
    VakPaper,
    VakContrast,
    TokyoNight,
}

impl ThemeKind {
    pub fn label(&self) -> &'static str {
        match self {
            ThemeKind::VakWarm => "Vak Warm (Terracotta)",
            ThemeKind::VakSlate => "Vak Slate (Dark)",
            ThemeKind::VakPaper => "Vak Paper (Light)",
            ThemeKind::VakContrast => "Vak Contrast (OLED)",
            ThemeKind::TokyoNight => "Tokyo Night (Cyber)",
        }
    }

    pub fn next(&self) -> Self {
        match self {
            ThemeKind::VakWarm => ThemeKind::VakSlate,
            ThemeKind::VakSlate => ThemeKind::VakPaper,
            ThemeKind::VakPaper => ThemeKind::VakContrast,
            ThemeKind::VakContrast => ThemeKind::TokyoNight,
            ThemeKind::TokyoNight => ThemeKind::VakWarm,
        }
    }

    pub fn theme(&self) -> Theme {
        match self {
            ThemeKind::VakWarm => Theme::vak_warm(),
            ThemeKind::VakSlate => Theme::vak_slate(),
            ThemeKind::VakPaper => Theme::vak_paper(),
            ThemeKind::VakContrast => Theme::vak_contrast(),
            ThemeKind::TokyoNight => Theme::tokyo_night(),
        }
    }
}

#[derive(Debug, Clone)]
pub struct Theme {
    pub kind: ThemeKind,
    pub bg: Color,
    pub surface: Color,
    pub surface_hover: Color,
    pub border: Color,
    pub border_focus: Color,
    pub accent: Color,
    pub accent_bright: Color,
    pub status_ok: Color,
    pub status_warn: Color,
    pub status_danger: Color,
    pub status_info: Color,
    pub text_main: Color,
    pub text_muted: Color,
    pub text_faint: Color,
}

impl Theme {
    /// Canonical Vak Web Terracotta Theme (`#171714` / `#df795f`).
    pub fn vak_warm() -> Self {
        Self {
            kind: ThemeKind::VakWarm,
            bg: Color::Rgb(0x17, 0x17, 0x14),
            surface: Color::Rgb(0x1c, 0x1c, 0x19),
            surface_hover: Color::Rgb(0x29, 0x29, 0x25),
            border: Color::Rgb(0x34, 0x34, 0x2f),
            border_focus: Color::Rgb(0xdf, 0x79, 0x5f),
            accent: Color::Rgb(0xdf, 0x79, 0x5f),
            accent_bright: Color::Rgb(0xee, 0x92, 0x78),
            status_ok: Color::Rgb(0x73, 0xa9, 0x82),
            status_warn: Color::Rgb(0xd4, 0xa8, 0x5d),
            status_danger: Color::Rgb(0xd8, 0x6f, 0x72),
            status_info: Color::Rgb(0x7c, 0x9f, 0xc9),
            text_main: Color::Rgb(0xee, 0xea, 0xe2),
            text_muted: Color::Rgb(0x91, 0x8e, 0x86),
            text_faint: Color::Rgb(0x8b, 0x88, 0x80),
        }
    }

    /// Vak Slate Theme (`#121419` / `#7f9fca`).
    pub fn vak_slate() -> Self {
        Self {
            kind: ThemeKind::VakSlate,
            bg: Color::Rgb(0x12, 0x14, 0x19),
            surface: Color::Rgb(0x17, 0x1a, 0x20),
            surface_hover: Color::Rgb(0x27, 0x2c, 0x34),
            border: Color::Rgb(0x33, 0x39, 0x43),
            border_focus: Color::Rgb(0x7f, 0x9f, 0xca),
            accent: Color::Rgb(0x7f, 0x9f, 0xca),
            accent_bright: Color::Rgb(0x9a, 0xb7, 0xdc),
            status_ok: Color::Rgb(0x73, 0xa9, 0x82),
            status_warn: Color::Rgb(0xd4, 0xa8, 0x5d),
            status_danger: Color::Rgb(0xd8, 0x6f, 0x72),
            status_info: Color::Rgb(0x9a, 0xb7, 0xdc),
            text_main: Color::Rgb(0xf1, 0xf5, 0xf9),
            text_muted: Color::Rgb(0x94, 0xa3, 0xb8),
            text_faint: Color::Rgb(0x64, 0x74, 0x8b),
        }
    }

    /// Vak Paper Light Theme (`#f4f1ea` / `#a8462a`).
    pub fn vak_paper() -> Self {
        Self {
            kind: ThemeKind::VakPaper,
            bg: Color::Rgb(0xf4, 0xf1, 0xea),
            surface: Color::Rgb(0xfa, 0xf8, 0xf3),
            surface_hover: Color::Rgb(0xef, 0xeb, 0xe1),
            border: Color::Rgb(0xd9, 0xd3, 0xc6),
            border_focus: Color::Rgb(0xa8, 0x46, 0x2a),
            accent: Color::Rgb(0xa8, 0x46, 0x2a),
            accent_bright: Color::Rgb(0x8d, 0x39, 0x21),
            status_ok: Color::Rgb(0x2f, 0x6b, 0x40),
            status_warn: Color::Rgb(0x7a, 0x54, 0x11),
            status_danger: Color::Rgb(0xa5, 0x2a, 0x2f),
            status_info: Color::Rgb(0x2f, 0x55, 0x80),
            text_main: Color::Rgb(0x23, 0x21, 0x1c),
            text_muted: Color::Rgb(0x63, 0x5f, 0x54),
            text_faint: Color::Rgb(0x6c, 0x67, 0x5b),
        }
    }

    /// Vak Contrast OLED Theme (`#080808` / `#ff8e70`).
    pub fn vak_contrast() -> Self {
        Self {
            kind: ThemeKind::VakContrast,
            bg: Color::Rgb(0x08, 0x08, 0x08),
            surface: Color::Rgb(0x10, 0x10, 0x10),
            surface_hover: Color::Rgb(0x24, 0x24, 0x24),
            border: Color::Rgb(0x52, 0x52, 0x52),
            border_focus: Color::Rgb(0xff, 0x8e, 0x70),
            accent: Color::Rgb(0xff, 0x8e, 0x70),
            accent_bright: Color::Rgb(0xff, 0xa4, 0x8d),
            status_ok: Color::Rgb(0x00, 0xe6, 0x76),
            status_warn: Color::Rgb(0xff, 0xd6, 0x00),
            status_danger: Color::Rgb(0xff, 0x52, 0x52),
            status_info: Color::Rgb(0x40, 0xc4, 0xff),
            text_main: Color::Rgb(0xff, 0xff, 0xff),
            text_muted: Color::Rgb(0xb9, 0xb9, 0xb9),
            text_faint: Color::Rgb(0x85, 0x85, 0x85),
        }
    }

    /// Tokyo Night Cyber Theme (`#1a1b26` / `#70d6ff`).
    pub fn tokyo_night() -> Self {
        Self {
            kind: ThemeKind::TokyoNight,
            bg: Color::Rgb(0x1a, 0x1b, 0x26),
            surface: Color::Rgb(0x24, 0x28, 0x3b),
            surface_hover: Color::Rgb(0x2f, 0x35, 0x4e),
            border: Color::Rgb(0x41, 0x48, 0x68),
            border_focus: Color::Rgb(0x70, 0xd6, 0xff),
            accent: Color::Rgb(0x70, 0xd6, 0xff),
            accent_bright: Color::Rgb(0xb4, 0xf9, 0xf8),
            status_ok: Color::Rgb(0x9e, 0xce, 0x6a),
            status_warn: Color::Rgb(0xe0, 0xaf, 0x68),
            status_danger: Color::Rgb(0xf7, 0x76, 0x8e),
            status_info: Color::Rgb(0x7a, 0xa2, 0xf7),
            text_main: Color::Rgb(0xc0, 0xca, 0xf5),
            text_muted: Color::Rgb(0x9a, 0xa5, 0xce),
            text_faint: Color::Rgb(0x56, 0x5f, 0x89),
        }
    }

    // Helper styles
    pub fn style_base(&self) -> Style {
        Style::default().bg(self.bg).fg(self.text_main)
    }

    pub fn style_card(&self) -> Style {
        Style::default().bg(self.surface).fg(self.text_main)
    }

    pub fn style_border(&self) -> Style {
        Style::default().fg(self.border)
    }

    pub fn style_border_focus(&self) -> Style {
        Style::default().fg(self.border_focus)
    }

    pub fn style_accent(&self) -> Style {
        Style::default().fg(self.accent)
    }

    pub fn style_tab_active(&self) -> Style {
        Style::default()
            .bg(self.accent)
            .fg(self.bg)
            .add_modifier(Modifier::BOLD)
    }

    pub fn style_tab_inactive(&self) -> Style {
        Style::default().bg(self.surface).fg(self.text_muted)
    }

    pub fn style_ok(&self) -> Style {
        Style::default().fg(self.status_ok)
    }

    pub fn style_warn(&self) -> Style {
        Style::default().fg(self.status_warn)
    }

    pub fn style_danger(&self) -> Style {
        Style::default().fg(self.status_danger)
    }

    pub fn style_info(&self) -> Style {
        Style::default().fg(self.status_info)
    }

    pub fn style_diff_add(&self) -> Style {
        Style::default()
            .bg(Color::Rgb(0x1a, 0x2e, 0x22))
            .fg(self.status_ok)
    }

    pub fn style_diff_del(&self) -> Style {
        Style::default()
            .bg(Color::Rgb(0x35, 0x19, 0x1b))
            .fg(self.status_danger)
    }
}
