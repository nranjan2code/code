//! Retired tool registry.
//!
//! When a tool is removed from the agent's callable interface, its name
//! enters `RETIRED_TOOLS` so that plugin skill descriptions, prompt text,
//! or cached session data referencing it can be detected and cleaned up.
//!
//! A retired tool name may appear in:
//! - A skill `SKILL.md` body/description that still instructs the model to
//!   call it (the `python_eval` / `react_preview` hallucination class).
//! - A stale `[plugins] network_allow` entry in `.vak/config.toml`.
//! - A cached `TurnCapabilitiesBound` entry in a session ledger.
//!
//! The cleanup pass in `vak_core::seed` and `PluginStore::retired_plugins`
//! consults this list. Each entry carries the last version in which the
//! tool was available and the replacement instruction for migration.

/// A single retired tool reference with migration context.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RetiredTool {
    /// The exact tool name the model might try to call.
    pub name: &'static str,
    /// The AGENTS.md version when the tool was removed.
    pub retired_in: &'static str,
    /// Replacement: what the model should use instead.
    pub replacement: &'static str,
}

/// The compiled-in list of retired tool names, ordered by retirement date.
pub const RETIRED_TOOLS: &[RetiredTool] = &[
    RetiredTool {
        name: "python_eval",
        retired_in: "3.0.21",
        replacement: "bash — run Python via the `bash` tool (e.g. `python3 script.py`)",
    },
    RetiredTool {
        name: "react_preview",
        retired_in: "3.0.21",
        replacement: "bash — run React/preview tooling via the `bash` tool",
    },
];

/// Returns `true` if `name` matches a retired tool (case-insensitive).
pub fn is_retired(name: &str) -> bool {
    RETIRED_TOOLS
        .iter()
        .any(|tool| tool.name.eq_ignore_ascii_case(name))
}

/// Returns the replacement instruction for `name`, if it is retired.
pub fn replacement_for(name: &str) -> Option<&'static str> {
    RETIRED_TOOLS
        .iter()
        .find(|tool| tool.name.eq_ignore_ascii_case(name))
        .map(|tool| tool.replacement)
}

/// Returns all retired tool names as a `Vec<&str>`.
pub fn retired_names() -> Vec<&'static str> {
    RETIRED_TOOLS.iter().map(|t| t.name).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn python_eval_is_retired() {
        assert!(is_retired("python_eval"));
        assert!(is_retired("python_eval"));
        assert!(!is_retired("bash"));
    }

    #[test]
    fn react_preview_is_retired() {
        assert!(is_retired("react_preview"));
        assert!(!is_retired("read"));
    }

    #[test]
    fn replacement_is_not_empty() {
        for tool in RETIRED_TOOLS {
            assert!(
                !tool.replacement.is_empty(),
                "{} must have a non-empty replacement",
                tool.name
            );
        }
    }

    #[test]
    fn no_duplicate_retired_names() {
        let names: Vec<_> = RETIRED_TOOLS.iter().map(|t| t.name).collect();
        let unique: std::collections::HashSet<_> = names.iter().collect();
        assert_eq!(names.len(), unique.len(), "duplicate retired tool names");
    }
}
