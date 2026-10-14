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
            hosts: Vec::new(),
        },
    );
    Arc::new(McpManager::new(servers, std::env::temp_dir()))
}

fn ctx() -> ToolContext {
    ToolContext::new(std::env::temp_dir())
}

#[test]
fn broker_schema_names_configured_servers_without_starting_them() {
    let manager = manager();
    let tool = McpTool::new(manager.clone());
    let schema = tool.schema();
    assert_eq!(schema["properties"]["server"]["enum"], json!(["fake"]));
    assert!(
        manager.observations().is_empty(),
        "schema construction must preserve lazy MCP startup"
    );
}

#[tokio::test]
async fn tool_scoped_allow_keeps_server_discoverable_without_exposing_other_tools() {
    let tool = McpTool::with_policy(manager(), Some(vec!["fake/echo".into()]), Vec::new());
    assert_eq!(
        tool.schema()["properties"]["server"]["enum"],
        json!(["fake"])
    );
    let out = tool
        .execute(&json!({"action": "list", "server": "fake"}), &ctx())
        .await;
    assert!(!out.is_error(), "{}", out.content);
    assert!(out.content.contains("echo —"), "{}", out.content);
    assert!(!out.content.contains("boom —"), "{}", out.content);
    let denied = tool
        .execute(
            &json!({"action": "call", "server": "fake", "tool": "boom"}),
            &ctx(),
        )
        .await;
    assert!(denied.is_error());
    assert!(denied.content.contains("denied by channel policy"));
}

#[tokio::test]
async fn list_discovers_server_and_tools() {
    let tool = McpTool::new(manager());
    let out = tool
        .execute(&json!({"action": "list", "server": "fake"}), &ctx())
        .await;
    assert!(!out.is_error(), "list failed: {}", out.content);
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
    assert!(!out.is_error(), "call failed: {}", out.content);
    assert_eq!(out.content.trim(), "echo: hello mcp");
}

#[tokio::test]
async fn a_call_returns_the_whole_result() {
    let tool = McpTool::new(manager());
    let out = tool
        .execute(
            &json!({
                "action": "call",
                "server": "fake",
                "tool": "echo",
                "arguments": {"text": "x".repeat(50_000)}
            }),
            &ctx(),
        )
        .await;
    assert!(!out.is_error(), "{}", out.content);
    assert!(out.content.contains(&"x".repeat(50_000)));
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
    assert!(out.is_error());
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
    assert!(!result.is_error());
    assert!(!result.content.contains(secret), "got: {}", result.content);
    assert!(result.content.contains("[REDACTED]"));

    let error = tool
        .execute(
            &json!({"action": "call", "server": "fake", "tool": "boom"}),
            &ctx(),
        )
        .await;
    assert!(error.is_error());
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
    assert!(out.is_error());
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
    assert!(out.is_error());
    assert!(out.content.contains("cannot connect") || out.content.contains("unknown mcp server"));
}

#[tokio::test]
async fn missing_server_is_an_error_value_before_spawn() {
    let tool = McpTool::new(manager());
    let out = tool
        .execute(
            &json!({"action": "call", "tool": "echo", "arguments": {"text": "hello"}}),
            &ctx(),
        )
        .await;
    assert!(out.is_error());
    assert!(out.content.contains("missing required parameter: server"));
}

#[tokio::test]
async fn malformed_arguments_remain_typed_error_values() {
    let tool = McpTool::new(manager());
    let out = tool
        .execute(
            &json!({"action": "call", "server": "fake", "tool": "echo", "arguments": "not-an-object"}),
            &ctx(),
        )
        .await;
    assert!(out.is_error());
    assert!(out.content.contains("arguments") || out.content.contains("invalid"));
}

#[tokio::test]
async fn missing_action_parameter_is_rejected() {
    let tool = McpTool::new(manager());
    let out = tool.execute(&json!({}), &ctx()).await;
    assert!(out.is_error());
    assert!(out.content.contains("action"));
}

/// A networked server runs inside the sandbox and reaches only its declared
/// hosts, through its own egress proxy (docs/design/09-extensibility.md
/// "Network"). Local listeners stand in for the hosts.
#[cfg(target_os = "macos")]
#[tokio::test]
async fn a_networked_server_reaches_only_its_declared_hosts_through_its_proxy() {
    use vak_tools::sandbox::{SandboxMode, Seatbelt};
    let declared = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let other = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let declared = format!("127.0.0.1:{}", declared.local_addr().unwrap().port());
    let other = format!("127.0.0.1:{}", other.local_addr().unwrap().port());
    // The script's own folder is the sandbox's workspace, so it can read it.
    let scripts = server_script().parent().unwrap().to_path_buf();
    let mut servers = HashMap::new();
    servers.insert(
        "fake".to_string(),
        vak_mcp::ServerConfig {
            command: "python3".into(),
            args: vec![server_script().display().to_string()],
            env: Vec::new(),
            hosts: vec![declared.clone()],
        },
    );
    let sandbox: Arc<dyn vak_tools::sandbox::Sandbox> =
        Arc::new(Seatbelt::new(SandboxMode::ReadOnly, &scripts));
    let tool = McpTool::new(Arc::new(McpManager::new_sandboxed(
        servers,
        scripts,
        Some(sandbox),
    )));
    let probe = |target: &str| {
        json!({"action": "call", "server": "fake", "tool": "net_probe",
               "arguments": {"target": target}})
    };
    let allowed = tool.execute(&probe(&declared), &ctx()).await;
    assert!(!allowed.is_error(), "{}", allowed.content);
    assert!(
        allowed.content.contains("direct:denied"),
        "{}",
        allowed.content
    );
    assert!(
        allowed.content.contains("proxy:HTTP/1.1 200"),
        "{}",
        allowed.content
    );
    let refused = tool.execute(&probe(&other), &ctx()).await;
    assert!(
        refused.content.contains("direct:denied"),
        "{}",
        refused.content
    );
    assert!(
        refused.content.contains("proxy:HTTP/1.1 403"),
        "{}",
        refused.content
    );
}
