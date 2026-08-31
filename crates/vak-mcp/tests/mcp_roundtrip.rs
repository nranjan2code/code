#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;

use serde_json::json;

use vak_mcp::{McpManager, McpTool};
use vak_tools::{Tool, ToolContext};

fn server_script() -> PathBuf {
    let manifest = env!("CARGO_MANIFEST_DIR");
    PathBuf::from(manifest)
        .parent()
        .unwrap()
        .parent()
        .unwrap()
        .join("scripts/fake_mcp_server.py")
}

fn manager() -> Arc<McpManager> {
    let mut servers = HashMap::new();
    servers.insert(
        "fake".to_string(),
        vak_mcp::ServerConfig {
            command: "python3".into(),
            args: vec![server_script().display().to_string()],
            env: Vec::new(),
            network: false,
        },
    );
    Arc::new(McpManager::new(servers, std::env::temp_dir()))
}

fn ctx() -> ToolContext {
    ToolContext::new(std::env::temp_dir())
}

#[tokio::test]
async fn list_discovers_server_and_tools() {
    let tool = McpTool::new(manager());
    let out = tool.execute(&json!({"action": "list"}), &ctx()).await;
    assert!(!out.is_error, "list failed: {}", out.content);
    assert!(out.content.contains("fake:"), "got: {}", out.content);
    assert!(out.content.contains("echo — Echo back"));
}

#[tokio::test]
async fn call_invokes_tool_and_returns_text() {
    let tool = McpTool::new(manager());
    let out = tool
        .execute(
            &json!({
                "action": "call",
                "server": "fake",
                "tool": "echo",
                "arguments": {"text": "hello mcp"}
            }),
            &ctx(),
        )
        .await;
    assert!(!out.is_error, "call failed: {}", out.content);
    assert_eq!(out.content.trim(), "echo: hello mcp");
}

#[tokio::test]
async fn server_side_tool_errors_become_error_values() {
    let tool = McpTool::new(manager());
    let out = tool
        .execute(
            &json!({
                "action": "call",
                "server": "fake",
                "tool": "boom"
            }),
            &ctx(),
        )
        .await;
    assert!(out.is_error);
    assert!(out.content.contains("boom failed on purpose"));
}

#[tokio::test]
async fn unknown_tool_is_rejected_from_discovered_catalog() {
    let tool = McpTool::new(manager());
    let out = tool
        .execute(
            &json!({
                "action": "call",
                "server": "fake",
                "tool": "search"
            }),
            &ctx(),
        )
        .await;
    assert!(out.is_error);
    assert!(
        out.content.contains("unknown tool 'search'"),
        "got: {}",
        out.content
    );
    assert!(out.content.contains("echo, boom"), "got: {}", out.content);
}

#[tokio::test]
async fn unknown_server_is_an_error_value() {
    let tool = McpTool::new(manager());
    let out = tool
        .execute(
            &json!({"action": "call", "server": "nope", "tool": "x"}),
            &ctx(),
        )
        .await;
    assert!(out.is_error);
    assert!(out.content.contains("cannot connect") || out.content.contains("unknown mcp server"));
}

#[tokio::test]
async fn missing_action_parameter_is_rejected() {
    let tool = McpTool::new(manager());
    let out = tool.execute(&json!({}), &ctx()).await;
    assert!(out.is_error);
    assert!(out.content.contains("action"));
}
