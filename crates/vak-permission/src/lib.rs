//! vak-permission: rule-based permission decisions, decoupled from
//! enforcement. Evaluate(tool, args, mode) -> Decision; the agent loop and
//! UI approvers decide what Ask becomes.

#![cfg_attr(test, allow(clippy::expect_used, clippy::panic, clippy::unwrap_used))]

pub mod engine;
pub mod rules;

pub use engine::{AskSource, Decision, PermissionEngine, path_in_workspace};
pub use rules::{Rule, RuleDecision, RuleError};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Mode {
    ReadOnly,
    #[default]
    WorkspaceWrite,
    FullAccess,
}

impl Mode {
    pub fn parse(s: &str) -> Option<Mode> {
        match s {
            "read-only" | "readonly" => Some(Mode::ReadOnly),
            "workspace-write" => Some(Mode::WorkspaceWrite),
            "full-access" | "fullaccess" => Some(Mode::FullAccess),
            _ => None,
        }
    }

    pub fn as_str(&self) -> &'static str {
        match self {
            Mode::ReadOnly => "read-only",
            Mode::WorkspaceWrite => "workspace-write",
            Mode::FullAccess => "full-access",
        }
    }
}
