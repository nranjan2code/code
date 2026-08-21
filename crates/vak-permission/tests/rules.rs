#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use serde_json::json;
use vak_permission::engine::Decision;
use vak_permission::{Mode, PermissionEngine, Rule};

fn bash(cmd: &str) -> serde_json::Value {
    json!({"command": cmd})
}

#[test]
fn rule_parsing_forms() {
    let r = Rule::parse("Bash(git *)").unwrap();
    assert_eq!(r.tool, "Bash");
    assert_eq!(r.decision, vak_permission::RuleDecision::Allow);
    assert!(r.arg_glob.is_some());

    let r = Rule::parse("-Bash(rm *)").unwrap();
    assert_eq!(r.decision, vak_permission::RuleDecision::Deny);

    let r = Rule::parse("?Edit(src/**)").unwrap();
    assert_eq!(r.decision, vak_permission::RuleDecision::Ask);

    let r = Rule::parse("read").unwrap();
    assert_eq!(r.tool, "read");
    assert!(r.arg_glob.is_none());

    assert!(Rule::parse("Bash(git").is_err());
    assert!(Rule::parse("(x)").is_err());
    assert!(Rule::parse("Bad Tool(x)").is_err());
}

#[test]
fn allow_rule_beats_mode_default() {
    let eng = PermissionEngine::from_rule_strings(&["Bash(git *)".to_string()]).unwrap();
    let d = eng.evaluate(
        "bash",
        &bash("git push"),
        Mode::WorkspaceWrite,
        std::path::Path::new("/tmp"),
    );
    assert_eq!(d, Decision::Allow);
}

#[test]
fn deny_rule_wins_over_allow_mode() {
    let eng =
        PermissionEngine::from_rule_strings(&["-Bash(rm *)".to_string(), "Bash(*)".to_string()])
            .unwrap();
    let d = eng.evaluate(
        "bash",
        &bash("rm -rf /tmp/x"),
        Mode::FullAccess,
        std::path::Path::new("/tmp"),
    );
    assert!(matches!(d, Decision::Deny { .. }));

    let d = eng.evaluate(
        "bash",
        &bash("ls -la"),
        Mode::FullAccess,
        std::path::Path::new("/tmp"),
    );
    assert_eq!(d, Decision::Allow);
}

#[test]
fn bash_subcommand_matching() {
    let eng = PermissionEngine::from_rule_strings(&["Bash(git push *)".to_string()]).unwrap();
    let d = eng.evaluate(
        "bash",
        &bash("npm test && git push origin main"),
        Mode::WorkspaceWrite,
        std::path::Path::new("/tmp"),
    );
    assert_eq!(d, Decision::Allow);

    let d = eng.evaluate(
        "bash",
        &bash("FOO=bar git push origin main"),
        Mode::WorkspaceWrite,
        std::path::Path::new("/tmp"),
    );
    assert_eq!(d, Decision::Allow, "leading env assignments are stripped");
}

#[test]
fn read_only_mode_allows_reads_denies_writes() {
    let eng = PermissionEngine::default();
    let d = eng.evaluate(
        "read",
        &json!({"path": "a.txt"}),
        Mode::ReadOnly,
        std::path::Path::new("/tmp"),
    );
    assert_eq!(d, Decision::Allow);
    let d = eng.evaluate(
        "glob",
        &json!({"pattern": "**"}),
        Mode::ReadOnly,
        std::path::Path::new("/tmp"),
    );
    assert_eq!(d, Decision::Allow);
    let d = eng.evaluate(
        "write",
        &json!({"path": "a.txt"}),
        Mode::ReadOnly,
        std::path::Path::new("/tmp"),
    );
    assert!(matches!(d, Decision::Deny { .. }));
    let d = eng.evaluate(
        "bash",
        &bash("echo hi"),
        Mode::ReadOnly,
        std::path::Path::new("/tmp"),
    );
    assert!(matches!(d, Decision::Deny { .. }));
}

#[test]
fn workspace_write_scopes_file_tools() {
    let cwd = std::env::temp_dir().join("vak-perm-test");
    std::fs::create_dir_all(&cwd).unwrap();
    let eng = PermissionEngine::default();

    let inside = cwd.join("sub/inner.txt");
    std::fs::create_dir_all(inside.parent().unwrap()).unwrap();
    std::fs::write(&inside, "x").unwrap();
    let d = eng.evaluate(
        "write",
        &json!({"path": inside}),
        Mode::WorkspaceWrite,
        &cwd,
    );
    assert_eq!(d, Decision::Allow, "absolute path inside workspace allowed");

    let outside = std::env::temp_dir().join("vak-perm-outside.txt");
    std::fs::write(&outside, "x").unwrap();
    let d = eng.evaluate(
        "edit",
        &json!({"path": outside}),
        Mode::WorkspaceWrite,
        &cwd,
    );
    assert!(matches!(d, Decision::Ask { .. }), "outside workspace asks");

    let d = eng.evaluate(
        "write",
        &json!({"path": "relative/new.txt"}),
        Mode::WorkspaceWrite,
        &cwd,
    );
    assert_eq!(
        d,
        Decision::Allow,
        "relative paths resolve into the workspace"
    );

    let _ = std::fs::remove_dir_all(&cwd);
    let _ = std::fs::remove_file(&outside);
}

#[test]
fn bash_asks_in_workspace_write_by_default() {
    let eng = PermissionEngine::default();
    let d = eng.evaluate(
        "bash",
        &bash("cargo test"),
        Mode::WorkspaceWrite,
        std::path::Path::new("/tmp"),
    );
    match d {
        Decision::Ask { reason } => assert!(reason.contains("cargo test")),
        other => panic!("expected ask, got {other:?}"),
    }
}

#[test]
fn first_matching_rule_wins_in_order() {
    let eng = PermissionEngine::from_rule_strings(&[
        "?Bash(npm *)".to_string(),
        "Bash(npm *)".to_string(),
    ])
    .unwrap();
    let d = eng.evaluate(
        "bash",
        &bash("npm run build"),
        Mode::FullAccess,
        std::path::Path::new("/tmp"),
    );
    assert!(
        matches!(d, Decision::Ask { .. }),
        "earlier ? rule wins over later allow"
    );
}
