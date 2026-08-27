//! vak-tui: inline stream-based terminal UI.
//!
//! Renders to native scrollback (no alternate screen); only the input line
//! redraws in place. Keystrokes while the agent runs become steering input.

pub mod app;
pub mod data;

pub use app::{UiConfig, run};
