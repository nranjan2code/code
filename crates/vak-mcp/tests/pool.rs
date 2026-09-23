#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

//! The pool starts a server only on demand (AGENTS.md invariant 25). Each
//! test wraps the server command so every spawn appends a line to a marker
//! file, which makes "was a process started?" a count, not a guess.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use serde_json::json;

use vak_mcp::{McpManager, McpTool, ServerConfig};
use vak_tools::{Tool, ToolContext};

fn server_script() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../scripts/fake_mcp_server.py")
        .canonicalize()
        .unwrap()
}

/// A server whose every start is recorded in `marker`; `then` is what the
/// shell runs after recording.
fn counted(marker: &Path, then: &str) -> ServerConfig {
    ServerConfig {
        command: "sh".into(),
        args: vec![
            "-c".into(),
            format!("echo started >> '{}'; {then}", marker.display()),
        ],
        env: Vec::new(),
        network: false,
    }
}

fn spawns(marker: &Path) -> usize {
    std::fs::read_to_string(marker)
        .map(|text| text.lines().count())
        .unwrap_or(0)
}

fn pool(name: &str, config: ServerConfig, cwd: &Path) -> McpManager {
    McpManager::new(
        HashMap::from([(name.to_string(), config)]),
        cwd.to_path_buf(),
    )
}

fn ctx(cwd: &Path) -> ToolContext {
    ToolContext::new(cwd.to_path_buf())
}

#[tokio::test]
async fn nothing_starts_until_a_call_needs_the_server() {
    let dir = tempfile::tempdir().unwrap();
    let marker = dir.path().join("spawns");
    let script = format!("exec python3 '{}'", server_script().display());
    let manager = Arc::new(pool("fake", counted(&marker, &script), dir.path()));
    let tool = McpTool::new(manager.clone());

    assert!(manager.observations().is_empty());
    let names = tool
        .execute(&json!({"action": "list"}), &ctx(dir.path()))
        .await;
    assert!(!names.is_error, "{}", names.content);
    assert!(names.content.contains("- fake"), "{}", names.content);
    assert_eq!(spawns(&marker), 0, "listing server names starts nothing");

    let listed = tool
        .execute(
            &json!({"action": "list", "server": "fake"}),
            &ctx(dir.path()),
        )
        .await;
    assert!(
        listed.content.contains("echo — Echo back"),
        "{}",
        listed.content
    );
    assert_eq!(
        spawns(&marker),
        1,
        "demand starts exactly the server asked for"
    );

    let called = tool
        .execute(
            &json!({"action": "call", "server": "fake", "tool": "echo", "arguments": {"text": "hi"}}),
            &ctx(dir.path()),
        )
        .await;
    assert!(!called.is_error, "{}", called.content);
    assert_eq!(spawns(&marker), 1, "the pooled connection is reused");

    let observed = manager.observations();
    let tools = observed["fake"].tools.as_ref().expect("catalog observed");
    assert!(tools.iter().any(|t| t.name == "echo"));
}

#[tokio::test]
async fn an_evicted_server_is_not_restarted_without_demand() {
    let dir = tempfile::tempdir().unwrap();
    let marker = dir.path().join("spawns");
    let script = format!("exec python3 '{}'", server_script().display());
    let manager = pool("fake", counted(&marker, &script), dir.path());
    manager.list_tools("fake").await.unwrap();
    assert_eq!(spawns(&marker), 1);

    assert_eq!(manager.evict_idle(Duration::ZERO).await, vec!["fake"]);
    tokio::time::sleep(Duration::from_millis(100)).await;
    assert_eq!(spawns(&marker), 1, "eviction never triggers a respawn");
    assert!(
        manager.observations()["fake"].tools.is_some(),
        "the last catalog survives eviction, so the prompt still names its tools"
    );

    manager.list_tools("fake").await.unwrap();
    assert_eq!(spawns(&marker), 2, "the next demand respawns it");
}

#[tokio::test]
async fn a_failing_server_backs_off_instead_of_respawning_on_every_call() {
    let dir = tempfile::tempdir().unwrap();
    let marker = dir.path().join("spawns");
    let manager = pool("broken", counted(&marker, "exit 1"), dir.path());

    let first = manager.list_tools("broken").await.unwrap_err();
    assert_eq!(spawns(&marker), 1);
    let observed = manager.observations();
    assert!(
        observed["broken"].failure.is_some(),
        "the failure is recorded"
    );
    assert!(observed["broken"].tools.is_none());

    let second = manager.list_tools("broken").await.unwrap_err();
    assert_eq!(
        spawns(&marker),
        1,
        "inside the backoff window nothing is spawned"
    );
    assert!(
        second.contains("next attempt in"),
        "{second} (first: {first})"
    );
}

#[tokio::test]
async fn the_observer_hears_what_demand_learned() {
    let dir = tempfile::tempdir().unwrap();
    let marker = dir.path().join("spawns");
    let script = format!("exec python3 '{}'", server_script().display());
    let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();
    let manager = pool("fake", counted(&marker, &script), dir.path()).with_observer(tx);

    manager.list_tools("fake").await.unwrap();
    assert_eq!(
        rx.try_recv().unwrap(),
        "fake",
        "a learned catalog is announced"
    );
    manager.list_tools("fake").await.unwrap();
    assert!(rx.try_recv().is_err(), "an unchanged catalog is not news");

    manager.forget_catalog("fake");
    assert_eq!(rx.try_recv().unwrap(), "fake");
    assert!(manager.observations()["fake"].tools.is_none());
}
