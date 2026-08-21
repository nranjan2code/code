#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::sync::Arc;

use serde_json::json;
use tempfile::tempdir;
use tokio_util::sync::CancellationToken;

use vak_hooks::{HookDef, HookEvent, HookOutcome, run_hooks};

fn hook(command: &str) -> Arc<Vec<HookDef>> {
    Arc::new(vec![HookDef {
        event: HookEvent::PreToolUse,
        matcher: None,
        command: command.to_string(),
        timeout_ms: 5000,
    }])
}

#[tokio::test]
async fn silent_exit_zero_is_no_opinion() {
    let dir = tempdir().unwrap();
    let out = run_hooks(
        hook("exit 0"),
        HookEvent::PreToolUse,
        "s",
        dir.path(),
        Some(("bash", &json!({"command": "ls"}))),
        None,
        &CancellationToken::new(),
    )
    .await;
    assert_eq!(out, HookOutcome::default());
}

#[tokio::test]
async fn json_block_decision_blocks_with_reason() {
    let dir = tempdir().unwrap();
    let out = run_hooks(
        hook(r#"echo '{"decision":"block","reason":"no rm allowed"}'"#),
        HookEvent::PreToolUse,
        "s",
        dir.path(),
        Some(("bash", &json!({"command": "rm -rf /"}))),
        None,
        &CancellationToken::new(),
    )
    .await;
    assert!(out.blocked);
    assert_eq!(out.reason.as_deref(), Some("no rm allowed"));
}

#[tokio::test]
async fn exit_two_blocks_with_stderr_reason() {
    let dir = tempdir().unwrap();
    let out = run_hooks(
        hook("echo policy-violation >&2; exit 2"),
        HookEvent::PreToolUse,
        "s",
        dir.path(),
        Some(("bash", &json!({"command": "x"}))),
        None,
        &CancellationToken::new(),
    )
    .await;
    assert!(out.blocked);
    assert_eq!(out.reason.as_deref(), Some("policy-violation"));
}

#[tokio::test]
async fn handler_receives_event_json_on_stdin() {
    let dir = tempdir().unwrap();
    let capture = dir.path().join("captured.json");
    let cmd = format!("cat > {}", capture.display());
    let out = run_hooks(
        hook(&cmd),
        HookEvent::PreToolUse,
        "sess-1",
        dir.path(),
        Some(("edit", &json!({"path": "a.txt"}))),
        None,
        &CancellationToken::new(),
    )
    .await;
    assert!(!out.blocked);
    let captured: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(&capture).unwrap()).unwrap();
    assert_eq!(captured["event"], "pre_tool_use");
    assert_eq!(captured["session_id"], "sess-1");
    assert_eq!(captured["tool"]["name"], "edit");
    assert_eq!(captured["tool"]["input"]["path"], "a.txt");
}

#[tokio::test]
async fn matcher_filters_by_tool_and_args() {
    let dir = tempdir().unwrap();
    let hooks = Arc::new(vec![HookDef {
        event: HookEvent::PreToolUse,
        matcher: Some(vak_permission::Rule::parse("Bash(git push *)").unwrap()),
        command: r#"echo '{"decision":"block","reason":"push blocked"}'"#.to_string(),
        timeout_ms: 5000,
    }]);

    let hit = run_hooks(
        hooks.clone(),
        HookEvent::PreToolUse,
        "s",
        dir.path(),
        Some(("bash", &json!({"command": "git push origin main"}))),
        None,
        &CancellationToken::new(),
    )
    .await;
    assert!(hit.blocked);

    let miss = run_hooks(
        hooks,
        HookEvent::PreToolUse,
        "s",
        dir.path(),
        Some(("bash", &json!({"command": "git status"}))),
        None,
        &CancellationToken::new(),
    )
    .await;
    assert!(!miss.blocked);
}

#[tokio::test]
async fn timeout_kills_hook_and_reports() {
    let dir = tempdir().unwrap();
    let hooks = Arc::new(vec![HookDef {
        event: HookEvent::PreToolUse,
        matcher: None,
        command: "sleep 30".to_string(),
        timeout_ms: 800,
    }]);
    let start = std::time::Instant::now();
    let out = run_hooks(
        hooks,
        HookEvent::PreToolUse,
        "s",
        dir.path(),
        Some(("bash", &json!({}))),
        None,
        &CancellationToken::new(),
    )
    .await;
    assert!(!out.blocked);
    assert!(start.elapsed() < std::time::Duration::from_secs(10));
}

#[tokio::test]
async fn non_matching_event_skips_handler() {
    let dir = tempdir().unwrap();
    let out = run_hooks(
        hook(r#"echo '{"decision":"block"}'"#),
        HookEvent::Stop,
        "s",
        dir.path(),
        None,
        None,
        &CancellationToken::new(),
    )
    .await;
    assert!(
        !out.blocked,
        "hook defined for PreToolUse must not fire on Stop"
    );
}
