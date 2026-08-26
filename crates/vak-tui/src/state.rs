//! Shared mutable/immutable TUI state types used across modules.

use std::collections::HashMap;

use crate::markdown::LineStyler;

#[derive(Default)]
pub struct UiState {
    pub styler: LineStyler,
    pub partial: String,
    pub total_in: u64,
    pub total_out: u64,
    pub tool_args: HashMap<String, (String, String)>,
    pub thinking_shown: bool,
    pub model: String,
    pub provider: String,
    pub cost_usd: f64,
    pub sub_in: u64,
    pub sub_out: u64,
    pub thinking_mode: ThinkingMode,
    pub thinking_partial: String,
    pub expanded_tools: bool,
    pub run_state: RunState,
    /// Final assistant text of the current/last turn, for Alt-Y /copy.
    pub last_response: String,
    /// When attached to a subagent, its label drives composer identity.
    pub attached_label: Option<String>,
}

/// Typed run-state truth (doc 21 §4): the status row always names what the
/// agent is doing. No generic spinner may stand in for a specific state.
#[derive(Clone, Default, PartialEq, Eq)]
pub enum RunState {
    #[default]
    Thinking,
    Streaming,
    Tool {
        name: String,
    },
    Retrying {
        attempt: u32,
        delay_ms: u64,
        reason: String,
    },
    Compacting,
}

impl RunState {
    pub fn label(&self) -> String {
        match self {
            Self::Thinking => "thinking".to_string(),
            Self::Streaming => "streaming".to_string(),
            Self::Tool { name } => format!("tool · {name}"),
            Self::Retrying {
                attempt,
                delay_ms,
                reason,
            } => format!("retry {attempt} in {delay_ms}ms — {reason}"),
            Self::Compacting => "compacting context".to_string(),
        }
    }
}

#[derive(Clone, Copy, Default, PartialEq, Eq)]
pub enum ThinkingMode {
    Off,
    #[default]
    Indicator,
    Full,
}

impl ThinkingMode {
    pub fn next(self) -> Self {
        match self {
            Self::Off => Self::Indicator,
            Self::Indicator => Self::Full,
            Self::Full => Self::Off,
        }
    }

    pub fn name(self) -> &'static str {
        match self {
            Self::Off => "off",
            Self::Indicator => "indicator",
            Self::Full => "full",
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum PickerKind {
    Provider,
    Model,
    Theme,
    Subagents,
    Sessions,
}

#[derive(Default)]
pub struct ModalView {
    pub title: String,
    pub rows: Vec<String>,
    pub scroll: usize,
    pub footer: String,
    /// Row indices of user prompts, for n/p jumps in the transcript viewer.
    pub anchors: Vec<usize>,
    /// Highlighted row, for interactive modals (keymap rebind).
    pub selected: Option<usize>,
}
