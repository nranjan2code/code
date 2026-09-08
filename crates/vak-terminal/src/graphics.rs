//! Hardware terminal graphics protocol detection and image/framebuffer
//! rendering (Kitty, iTerm2, Sixel, Half-block ANSI fallback).

use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::Color;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GraphicsProtocol {
    Kitty,
    ITerm2,
    Sixel,
    HalfBlockAnsi,
}

impl GraphicsProtocol {
    /// Autodetect terminal graphics support from environment variables.
    pub fn detect() -> Self {
        // Ghostty, Kitty, WezTerm support Kitty graphics protocol natively
        if let Ok(term) = std::env::var("TERM") {
            if term.contains("kitty") || term.contains("ghostty") || term.contains("wezterm") {
                return GraphicsProtocol::Kitty;
            }
        }
        if std::env::var("KITTY_WINDOW_ID").is_ok() || std::env::var("GHOSTTY_RESOURCES_DIR").is_ok() {
            return GraphicsProtocol::Kitty;
        }
        // iTerm2 or VSCode integrated terminal
        if let Ok(program) = std::env::var("TERM_PROGRAM") {
            if program == "iTerm.app" || program == "vscode" {
                return GraphicsProtocol::ITerm2;
            }
            if program == "WezTerm" {
                return GraphicsProtocol::Kitty;
            }
        }
        // Fallback: 24-bit TrueColor Half-block ANSI
        GraphicsProtocol::HalfBlockAnsi
    }
}

/// A rendered visual artifact cell for terminal previewing.
#[derive(Debug, Clone)]
pub struct HalfBlockCell {
    pub char_glyph: char,
    pub fg: Color,
    pub bg: Color,
}

/// Scaled half-block grid for rendering images on any terminal without protocols.
#[derive(Debug, Clone)]
pub struct HalfBlockImage {
    pub width: u16,
    pub height: u16,
    pub cells: Vec<HalfBlockCell>,
}

impl HalfBlockImage {
    /// Generate a sample live wireframe preview for React dashboards.
    pub fn sample_dashboard_wireframe(width: u16, height: u16) -> Self {
        let w = width.max(10) as usize;
        let h = height.max(4) as usize;
        let mut cells = Vec::with_capacity(w * h);

        for y in 0..h {
            for x in 0..w {
                // Header bar
                if y == 0 {
                    cells.push(HalfBlockCell {
                        char_glyph: '▀',
                        fg: Color::Rgb(0xdf, 0x79, 0x5f), // Terracotta
                        bg: Color::Rgb(0x1c, 0x1c, 0x19),
                    });
                } else if x == 0 || x == w - 1 || y == h - 1 {
                    cells.push(HalfBlockCell {
                        char_glyph: '█',
                        fg: Color::Rgb(0x34, 0x34, 0x2f),
                        bg: Color::Rgb(0x17, 0x17, 0x14),
                    });
                } else if y >= 2 && y <= 4 && x >= 3 && x <= w.saturating_sub(4) {
                    // Sparkline wave
                    let wave_val = ((x as f32 * 0.4).sin() * 2.0) as i32;
                    let target_y = 3 + wave_val;
                    if (y as i32) == target_y {
                        cells.push(HalfBlockCell {
                            char_glyph: '▄',
                            fg: Color::Rgb(0x73, 0xa9, 0x82), // Sage green
                            bg: Color::Rgb(0x1c, 0x1c, 0x19),
                        });
                    } else {
                        cells.push(HalfBlockCell {
                            char_glyph: ' ',
                            fg: Color::Rgb(0xee, 0xea, 0xe2),
                            bg: Color::Rgb(0x1c, 0x1c, 0x19),
                        });
                    }
                } else {
                    cells.push(HalfBlockCell {
                        char_glyph: ' ',
                        fg: Color::Rgb(0xee, 0xea, 0xe2),
                        bg: Color::Rgb(0x1c, 0x1c, 0x19),
                    });
                }
            }
        }

        Self {
            width: w as u16,
            height: h as u16,
            cells,
        }
    }

    /// Render onto a ratatui buffer
    pub fn render(&self, area: Rect, buf: &mut Buffer) {
        let draw_w = (area.width as usize).min(self.width as usize);
        let draw_h = (area.height as usize).min(self.height as usize);

        for y in 0..draw_h {
            for x in 0..draw_w {
                let idx = y * (self.width as usize) + x;
                if let Some(cell) = self.cells.get(idx) {
                    if let Some(cell_ref) = buf.cell_mut((area.x + x as u16, area.y + y as u16)) {
                        cell_ref.set_char(cell.char_glyph);
                        cell_ref.set_fg(cell.fg);
                        cell_ref.set_bg(cell.bg);
                    }
                }
            }
        }
    }
}
