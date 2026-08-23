//! Learning-loop tool classification (docs/design/26-learning.md):
//! journaling tools are sanctioned under workspace-write, denied by
//! read-only; session_search is a read everywhere reads are allowed.

#![allow(clippy::unwrap_used, clippy::expect_used)]

use serde_json::json;
use vak_permission::{Mode, PermissionEngine};

#[test]
fn learning_tools_allowed_under_workspace_write_without_rules() {
    let engine = PermissionEngine::default();
    for tool in ["remember", "propose_skill"] {
        let d = engine.evaluate(
            tool,
            &json!({}),
            Mode::WorkspaceWrite,
            std::path::Path::new("/ws"),
        );
        assert!(
            matches!(d, vak_permission::Decision::Allow),
            "{tool} must not require approval: {d:?}"
        );
    }
}

#[test]
fn remember_denied_in_read_only() {
    let engine = PermissionEngine::default();
    let d = engine.evaluate(
        "remember",
        &json!({}),
        Mode::ReadOnly,
        std::path::Path::new("/ws"),
    );
    assert!(matches!(d, vak_permission::Decision::Deny { .. }), "{d:?}");
}

#[test]
fn session_search_is_a_read() {
    let engine = PermissionEngine::default();
    for mode in [Mode::ReadOnly, Mode::WorkspaceWrite] {
        let d = engine.evaluate(
            "session_search",
            &json!({"query": "x"}),
            mode,
            std::path::Path::new("/ws"),
        );
        assert!(
            matches!(d, vak_permission::Decision::Allow),
            "{mode:?}: {d:?}"
        );
    }
}

#[test]
fn explicit_deny_rule_still_beats_learning_allowance() {
    // Severity aggregation: a user's deny rule outranks the built-in
    // WorkspaceWrite allowance.
    let engine = PermissionEngine::from_rule_strings(&["-remember".to_string()]).unwrap();
    let d = engine.evaluate(
        "remember",
        &json!({}),
        Mode::WorkspaceWrite,
        std::path::Path::new("/ws"),
    );
    assert!(matches!(d, vak_permission::Decision::Deny { .. }), "{d:?}");
}
