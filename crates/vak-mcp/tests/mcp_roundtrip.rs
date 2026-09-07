#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;

use serde_json::json;

use vak_mcp::{McpManager, McpTool};
use vak_tools::{OutputLimits, Tool, ToolContext};

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
    manager_with_env(Vec::new())
}

fn manager_with_env(env: Vec<(String, String)>) -> Arc<McpManager> {
    let mut servers = HashMap::new();
    servers.insert(
        "fake".to_string(),
        vak_mcp::ServerConfig {
            command: "python3".into(),
            args: vec![server_script().display().to_string()],
            env,
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
async fn call_output_is_bounded_before_entering_the_session() {
    let tool = McpTool::new(manager());
    let ctx = ToolContext {
        cwd: std::env::temp_dir(),
        cancel: tokio_util::sync::CancellationToken::new(),
        limits: OutputLimits {
            max_bytes: 500,
            max_line_chars: 2_000,
            spill_to_disk: false,
        },
        sandbox: None,
        sandbox_sink: None,
    };
    let out = tool
        .execute(
            &json!({
                "action": "call",
                "server": "fake",
                "tool": "echo",
                "arguments": {"text": "x".repeat(5_000)}
            }),
            &ctx,
        )
        .await;
    assert!(!out.is_error);
    assert!(out.content.chars().count() <= 620);
    assert!(out.content.contains("truncated"));
    let artifact_path = out
        .content
        .lines()
        .next()
        .and_then(|line| line.split("stored at ").nth(1))
        .and_then(|path| path.split(". Use the read").next())
        .expect("artifact path");
    let artifact = std::fs::read_to_string(artifact_path).expect("full MCP artifact");
    assert!(artifact.contains(&"x".repeat(5_000)));
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
async fn configured_secrets_are_redacted_from_mcp_results_and_errors() {
    let secret = "tavily-test-secret-value";
    let tool = McpTool::new(manager_with_env(vec![(
        "MCP_TEST_SECRET".into(),
        secret.into(),
    )]));

    let result = tool
        .execute(
            &json!({"action": "call", "server": "fake", "tool": "secret_result"}),
            &ctx(),
        )
        .await;
    assert!(!result.is_error);
    assert!(!result.content.contains(secret), "got: {}", result.content);
    assert!(result.content.contains("[REDACTED]"));

    let error = tool
        .execute(
            &json!({"action": "call", "server": "fake", "tool": "boom"}),
            &ctx(),
        )
        .await;
    assert!(error.is_error);
    assert!(!error.content.contains(secret), "got: {}", error.content);
    assert!(error.content.contains("[REDACTED]"));
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
