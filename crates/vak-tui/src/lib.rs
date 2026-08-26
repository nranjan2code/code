//! vak-tui: inline stream-based terminal UI.
//!
//! Renders to native scrollback (no alternate screen); only the input line
//! redraws in place. Keystrokes while the agent runs become steering input.

pub mod app;
pub mod approval;
pub mod commands;
pub mod complete;
pub mod data;
pub mod diffview;
pub mod editor;
pub mod events;
pub mod inbox;
pub mod keymap;
pub mod keys;
pub mod markdown;
pub mod mentions;
pub mod modals;
pub mod palette;
pub mod pickers;
pub mod prefs;
pub mod pricing;
pub mod render;
pub mod state;
pub mod status;
pub mod statusline;
pub mod tasks;
pub mod theme;
pub mod transcript;
pub mod width;

pub use app::{UiConfig, run};
