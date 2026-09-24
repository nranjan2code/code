//! Office document tools (docs/design/72-openxml-documents.md): `doc_read`
//! is a path-scoped read, `office_apply` a path-scoped write.

#![allow(clippy::unwrap_used, clippy::expect_used)]

use serde_json::json;
use vak_permission::{Decision, Mode, PermissionEngine};

fn decide(tool: &str, path: &str, mode: Mode) -> Decision {
    let workspace = std::env::temp_dir().join(format!(
        "vak-perm-documents-{}-{:?}",
        std::process::id(),
        std::thread::current().id()
    ));
    std::fs::create_dir_all(&workspace).unwrap();
    let decision =
        PermissionEngine::default().evaluate(tool, &json!({ "path": path }), mode, &workspace);
    let _ = std::fs::remove_dir_all(&workspace);
    decision
}

#[test]
fn doc_read_is_a_read_confined_to_the_workspace() {
    for mode in [Mode::ReadOnly, Mode::WorkspaceWrite] {
        assert!(
            matches!(decide("doc_read", "report.docx", mode), Decision::Allow),
            "{mode:?}"
        );
        assert!(
            matches!(
                decide("doc_read", "/etc/passwd", mode),
                Decision::Deny { .. }
            ),
            "{mode:?}"
        );
    }
}

#[test]
fn office_apply_is_a_write() {
    assert!(matches!(
        decide("office_apply", "report.docx", Mode::WorkspaceWrite),
        Decision::Allow
    ));
    assert!(matches!(
        decide(
            "office_apply",
            "/elsewhere/report.docx",
            Mode::WorkspaceWrite
        ),
        Decision::Ask { .. }
    ));
    assert!(matches!(
        decide("office_apply", "report.docx", Mode::ReadOnly),
        Decision::Deny { .. }
    ));
}
