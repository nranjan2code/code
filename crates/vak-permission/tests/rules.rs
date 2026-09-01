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
fn restricted_modes_deny_reads_outside_workspace() {
    let cwd = std::env::temp_dir().join("vak-perm-read-scope");
    std::fs::create_dir_all(&cwd).unwrap();
    let outside = cwd.parent().unwrap().join("vak-perm-secret.txt");
    std::fs::write(&outside, "secret").unwrap();
    let eng = PermissionEngine::default();

    for mode in [Mode::ReadOnly, Mode::WorkspaceWrite] {
        for tool in ["read", "glob", "grep"] {
            let d = eng.evaluate(tool, &json!({"path": outside}), mode, &cwd);
            assert!(
                matches!(d, Decision::Deny { .. }),
                "{tool} escaped in {mode:?}"
            );
        }
    }

    let d = eng.evaluate("read", &json!({"path": outside}), Mode::FullAccess, &cwd);
    assert_eq!(d, Decision::Allow);

    let _ = std::fs::remove_dir_all(&cwd);
    let _ = std::fs::remove_file(&outside);
}

#[test]
fn restricted_read_scope_resolves_symlinks() {
    #[cfg(unix)]
    {
        let cwd = std::env::temp_dir().join("vak-perm-read-symlink");
        let outside = std::env::temp_dir().join("vak-perm-read-symlink-secret.txt");
        std::fs::create_dir_all(&cwd).unwrap();
        std::fs::write(&outside, "secret").unwrap();
        std::os::unix::fs::symlink(&outside, cwd.join("looks-safe")).unwrap();

        let d = PermissionEngine::default().evaluate(
            "read",
            &json!({"path": "looks-safe"}),
            Mode::WorkspaceWrite,
            &cwd,
        );
        assert!(matches!(d, Decision::Deny { .. }));

        let _ = std::fs::remove_dir_all(&cwd);
        let _ = std::fs::remove_file(&outside);
    }
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
        Decision::Ask { reason, .. } => assert!(reason.contains("cargo test")),
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

#[test]
fn deny_beats_allow_regardless_of_registration_order() {
    // Deny registered AFTER the allow: severity aggregation must still
    // pick Deny. Under first-match-wins this silently allowed.
    let eng = PermissionEngine::from_rule_strings(&[
        "Bash(git *)".to_string(),
        "-Bash(git push *)".to_string(),
    ])
    .unwrap();
    let d = eng.evaluate(
        "bash",
        &bash("git push origin main"),
        Mode::WorkspaceWrite,
        std::path::Path::new("/tmp"),
    );
    assert!(matches!(d, Decision::Deny { .. }));

    let d = eng.evaluate(
        "bash",
        &bash("git status"),
        Mode::WorkspaceWrite,
        std::path::Path::new("/tmp"),
    );
    assert_eq!(d, Decision::Allow);
}

#[test]
fn substitution_and_newline_commands_are_opaque_to_allow_patterns() {
    let eng = PermissionEngine::from_rule_strings(&[
        "Bash(git *)".to_string(),
        "-Bash(rm *)".to_string(),
    ])
    .unwrap();

    for cmd in [
        "git status\nrm -rf ~",
        "git log --format=$(rm -rf ~)",
        "git log `rm -rf ~`",
    ] {
        let d = eng.evaluate(
            "bash",
            &bash(cmd),
            Mode::WorkspaceWrite,
            std::path::Path::new("/tmp"),
        );
        assert!(
            matches!(d, Decision::Ask { .. }),
            "'{cmd}' must not match a patterned allow rule"
        );
    }
}

#[test]
fn blanket_rules_still_cover_opaque_commands() {
    let eng = PermissionEngine::from_rule_strings(&["Bash(*)".to_string()]).unwrap();
    let d = eng.evaluate(
        "bash",
        &bash("git status\necho hi"),
        Mode::WorkspaceWrite,
        std::path::Path::new("/tmp"),
    );
    assert_eq!(d, Decision::Allow);
}

#[test]
fn mcp_calls_match_server_tool_candidates() {
    let eng = PermissionEngine::from_rule_strings(&[
        "-Mcp(evil-server/*)".to_string(),
        "Mcp(docs/*)".to_string(),
    ])
    .unwrap();
    let args = json!({"action": "call", "server": "docs", "tool": "search", "arguments": {}});
    let d = eng.evaluate(
        "mcp",
        &args,
        Mode::WorkspaceWrite,
        std::path::Path::new("/tmp"),
    );
    assert_eq!(d, Decision::Allow);

    let args = json!({"action": "call", "server": "evil-server", "tool": "wipe"});
    let d = eng.evaluate(
        "mcp",
        &args,
        Mode::WorkspaceWrite,
        std::path::Path::new("/tmp"),
    );
    assert!(matches!(d, Decision::Deny { .. }));
}

#[test]
fn declared_write_scope_denies_other_direct_file_mutations_before_rules() {
    let cwd = std::env::temp_dir().join("vak-permission-write-scope-test");
    let engine = PermissionEngine::default()
        .restrict_write_paths(&cwd, &[std::path::PathBuf::from("app.py")]);

    let allowed = engine.evaluate(
        "write",
        &json!({"path": "app.py", "content": "ok"}),
        Mode::FullAccess,
        &cwd,
    );
    assert_eq!(allowed, Decision::Allow);

    let denied = engine.evaluate(
        "edit",
        &json!({"path": "test_app.py", "old_text": "x", "new_text": "y"}),
        Mode::FullAccess,
        &cwd,
    );
    assert!(matches!(denied, Decision::Deny { .. }));
}
