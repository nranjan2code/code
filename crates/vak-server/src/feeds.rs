//! Feed pipeline HTTP handlers and routes.
//!
//! Provides endpoints for managing feed sources, items, search, alerts,
//! and statistics. All handlers delegate to the Python feed pipeline
//! scripts via subprocess execution.

use axum::{
    Json, Router,
    extract::{Path, Query, State},
    http::StatusCode,
    response::IntoResponse,
    routing::{delete, get, post},
};
use serde_json::{Value, json};
use std::collections::HashMap;
use std::path::PathBuf;
use std::process::Stdio;
use tokio::io::AsyncWriteExt;
use tokio::process::Command;

use crate::AppState;

/// Feed pipeline Python script directory.
fn feeds_dir(cwd: &std::path::Path) -> PathBuf {
    cwd.join("scripts").join("feeds")
}

/// Config file path for feed sources.
fn feeds_config_path(_cwd: &std::path::Path) -> PathBuf {
    // Global config lives at ~/.config/vak/feeds.toml
    let home = std::env::var("HOME").unwrap_or_default();
    PathBuf::from(home)
        .join(".config")
        .join("vak")
        .join("feeds.toml")
}

/// Run a Python feed script and return its JSON output.
async fn run_feed_script(
    cwd: &std::path::Path,
    script: &str,
    args: &[&str],
) -> Result<Value, (StatusCode, String)> {
    let dir = feeds_dir(cwd);
    let script_path = dir.join(script);

    if !script_path.exists() {
        return Err((
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("Feed script not found: {}", script_path.display()),
        ));
    }

    let output = Command::new("python3")
        .arg(&script_path)
        .args(args)
        .current_dir(&dir)
        .env("PYTHONPATH", dir.to_string_lossy().to_string())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .output()
        .await
        .map_err(|e| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("Failed to run feed script: {e}"),
            )
        })?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err((
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("Feed script error: {stderr}"),
        ));
    }

    let stdout = String::from_utf8_lossy(&output.stdout);
    serde_json::from_str(&stdout).map_err(|e| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("Failed to parse feed script output: {e}"),
        )
    })
}

/// Run a request through the feed MCP server.
async fn run_feed_mcp_request(
    cwd: &std::path::Path,
    request: &Value,
) -> Result<Value, (StatusCode, String)> {
    let dir = feeds_dir(cwd);
    let script = dir.join("feed_mcp.py");

    if !script.exists() {
        return Err((StatusCode::NOT_FOUND, "Feed MCP server not found".into()));
    }

    let input = serde_json::to_string(request).map_err(|e| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("JSON error: {e}"),
        )
    })?;

    let mut child = Command::new("python3")
        .arg(&script)
        .current_dir(&dir)
        .env("PYTHONPATH", dir.to_string_lossy().to_string())
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("Failed to spawn MCP: {e}"),
            )
        })?;

    if let Some(mut stdin) = child.stdin.take() {
        stdin.write_all(input.as_bytes()).await.map_err(|e| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("stdin write error: {e}"),
            )
        })?;
        stdin.shutdown().await.map_err(|e| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("stdin shutdown error: {e}"),
            )
        })?;
    }

    let output = child.wait_with_output().await.map_err(|e| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("MCP wait error: {e}"),
        )
    })?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err((
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("MCP server error: {stderr}"),
        ));
    }

    let stdout = String::from_utf8_lossy(&output.stdout);
    for line in stdout.lines() {
        if let Ok(response) = serde_json::from_str::<Value>(line) {
            return Ok(response);
        }
    }

    Err((
        StatusCode::INTERNAL_SERVER_ERROR,
        "No valid response from MCP server".into(),
    ))
}

/// GET /feeds/sources — List available source types from registry.
pub async fn list_source_types() -> Result<impl IntoResponse, (StatusCode, String)> {
    Ok(Json(json!({
        "source_types": [
            {"id": "rss", "name": "RSS / Atom Feed", "icon": "rss", "description": "Any RSS or Atom feed URL", "fetcher": "rss", "default_interval": "1h"},
            {"id": "youtube", "name": "YouTube Channel", "icon": "youtube", "description": "Follow a channel's uploads", "fetcher": "youtube", "default_interval": "6h"},
            {"id": "hacker_news", "name": "Hacker News", "icon": "fire", "description": "Top stories, best new, or Ask HN", "fetcher": "aggregator", "default_interval": "15m"},
            {"id": "reddit", "name": "Reddit", "icon": "reddit", "description": "Follow subreddits", "fetcher": "aggregator", "default_interval": "30m"},
            {"id": "lobsters", "name": "Lobste.rs", "icon": "lobsters", "description": "Lobste.rs technology stories", "fetcher": "aggregator", "default_interval": "30m"},
            {"id": "custom_http", "name": "Custom HTTP Source", "icon": "globe", "description": "Any HTTP endpoint", "fetcher": "custom", "default_interval": "1h"},
        ]
    })))
}

/// GET /feeds/config — Get merged feed config as structured JSON.
pub async fn get_feed_config(
    State(state): State<AppState>,
) -> Result<impl IntoResponse, (StatusCode, String)> {
    let cwd = state.core.cwd();
    let config_path = feeds_config_path(cwd);

    if !config_path.exists() {
        return Ok(Json(json!({
            "general": {
                "default_check_interval": "30m",
                "max_items_per_feed": 500,
                "dedup_window_days": 90
            },
            "sources": [],
            "alerts": []
        })));
    }

    // Use Python to parse TOML and return structured JSON
    let result = run_feed_script(cwd, "feed_ingest.py", &["--stats"]).await?;

    // Also read the config file for the full source list
    let content = tokio::fs::read_to_string(&config_path).await.map_err(|e| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("Failed to read config: {e}"),
        )
    })?;

    Ok(Json(json!({
        "config_path": config_path.to_string_lossy(),
        "stats": result,
        "raw_content": content
    })))
}

/// GET /feeds/items — List ingested items via MCP.
pub async fn list_feed_items(
    State(state): State<AppState>,
    Query(params): Query<HashMap<String, String>>,
) -> Result<impl IntoResponse, (StatusCode, String)> {
    let cwd = state.core.cwd();
    let limit = params
        .get("limit")
        .and_then(|s| s.parse::<usize>().ok())
        .unwrap_or(50);
    let source = params.get("source").map(|s| s.as_str()).unwrap_or("");

    // Use the MCP feed_latest tool to get items
    let mut arguments = json!({"limit": limit});
    if !source.is_empty() {
        arguments["source"] = json!(source);
    }

    let request = json!({
        "jsonrpc": "2.0",
        "id": 1,
        "method": "tools/call",
        "params": {
            "name": "feed_latest",
            "arguments": arguments
        }
    });

    let result = run_feed_mcp_request(cwd, &request).await?;
    Ok(Json(result))
}

/// GET /feeds/items/:id — Get single item.
pub async fn get_feed_item(
    State(state): State<AppState>,
    Path(id): Path<i64>,
) -> Result<impl IntoResponse, (StatusCode, String)> {
    let cwd = state.core.cwd();
    let request = json!({
        "jsonrpc": "2.0",
        "id": 1,
        "method": "tools/call",
        "params": {
            "name": "feed_item",
            "arguments": {"id": id}
        }
    });
    let result = run_feed_mcp_request(cwd, &request).await?;
    Ok(Json(result))
}

/// GET /feeds/search — Search feed items.
pub async fn search_feed_items(
    State(state): State<AppState>,
    Query(params): Query<HashMap<String, String>>,
) -> Result<impl IntoResponse, (StatusCode, String)> {
    let query = params.get("q").map(|s| s.as_str()).unwrap_or("");
    if query.is_empty() {
        return Err((
            StatusCode::BAD_REQUEST,
            "Missing query parameter 'q'".into(),
        ));
    }

    let cwd = state.core.cwd();
    let mut arguments = json!({"query": query});

    if let Some(tags) = params.get("tags") {
        let tag_list: Vec<&str> = tags.split(',').map(|s| s.trim()).collect();
        arguments["tags"] = json!(tag_list);
    }
    if let Some(since) = params.get("since") {
        arguments["since"] = json!(since);
    }
    if let Some(limit) = params.get("limit") {
        arguments["limit"] = json!(limit.parse::<usize>().unwrap_or(10));
    }
    if let Some(source) = params.get("source") {
        arguments["source"] = json!(source);
    }

    let request = json!({
        "jsonrpc": "2.0",
        "id": 1,
        "method": "tools/call",
        "params": {
            "name": "feed_search",
            "arguments": arguments
        }
    });

    let result = run_feed_mcp_request(cwd, &request).await?;
    Ok(Json(result))
}

/// GET /feeds/stats — Get ingestion statistics.
pub async fn get_feed_stats(
    State(state): State<AppState>,
) -> Result<impl IntoResponse, (StatusCode, String)> {
    let cwd = state.core.cwd();
    let result = run_feed_script(cwd, "feed_ingest.py", &["--stats"]).await?;
    Ok(Json(result))
}

/// GET /feeds/alerts — List alert rules via MCP.
pub async fn get_feed_alerts(
    State(state): State<AppState>,
) -> Result<impl IntoResponse, (StatusCode, String)> {
    let cwd = state.core.cwd();
    let request = json!({
        "jsonrpc": "2.0",
        "id": 1,
        "method": "tools/call",
        "params": {
            "name": "feed_alerts",
            "arguments": {}
        }
    });
    let result = run_feed_mcp_request(cwd, &request).await?;
    Ok(Json(result))
}

/// POST /feeds/ingest — Trigger manual ingestion.
pub async fn trigger_ingestion(
    State(state): State<AppState>,
) -> Result<impl IntoResponse, (StatusCode, String)> {
    let cwd = state.core.cwd();
    let workspace = cwd.to_string_lossy();
    let result = run_feed_script(cwd, "feed_ingest.py", &["--workspace", &workspace]).await?;
    Ok(Json(result))
}

/// POST /feeds/sources — Add a new source to feeds.toml.
pub async fn add_feed_source(
    State(state): State<AppState>,
    Json(payload): Json<Value>,
) -> Result<impl IntoResponse, (StatusCode, String)> {
    let cwd = state.core.cwd();
    let config_path = feeds_config_path(cwd);

    // Read existing config or create default
    let mut content = if config_path.exists() {
        tokio::fs::read_to_string(&config_path).await.map_err(|e| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("Failed to read config: {e}"),
            )
        })?
    } else {
        "[general]\ndefault_check_interval = \"30m\"\nmax_items_per_feed = 500\ndedup_window_days = 90\n\n".to_string()
    };

    // Extract fields from payload
    let name = payload
        .get("name")
        .and_then(|v| v.as_str())
        .unwrap_or("Untitled");
    let source_type = payload
        .get("type")
        .and_then(|v| v.as_str())
        .unwrap_or("rss");
    let url = payload.get("url").and_then(|v| v.as_str()).unwrap_or("");
    let interval = payload
        .get("interval")
        .and_then(|v| v.as_str())
        .unwrap_or("1h");
    let tags = payload.get("tags").and_then(|v| v.as_array()).map(|a| {
        a.iter()
            .filter_map(|v| v.as_str())
            .collect::<Vec<_>>()
            .join(", ")
    });
    let trust = payload
        .get("trust")
        .and_then(|v| v.as_str())
        .unwrap_or("medium");

    // Build the new source block
    let mut source_block = format!(
        "\n[[sources]]\nname = \"{}\"\ntype = \"{}\"\n",
        escape_toml(name),
        escape_toml(source_type)
    );
    if !url.is_empty() {
        source_block.push_str(&format!("url = \"{}\"\n", escape_toml(url)));
    }
    source_block.push_str(&format!("interval = \"{}\"\n", escape_toml(interval)));
    if let Some(tags_str) = &tags
        && !tags_str.is_empty()
    {
        source_block.push_str(&format!(
            "tags = [{}]\n",
            tags_str
                .split(", ")
                .map(|t| format!("\"{}\"", escape_toml(t)))
                .collect::<Vec<_>>()
                .join(", ")
        ));
    }
    source_block.push_str(&format!("trust = \"{}\"\n", escape_toml(trust)));
    source_block.push_str("enabled = true\n");

    // Ensure config ends with newline before appending
    if !content.ends_with('\n') {
        content.push('\n');
    }
    content.push_str(&source_block);

    // Ensure parent directory exists
    if let Some(parent) = config_path.parent() {
        tokio::fs::create_dir_all(parent).await.map_err(|e| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("Failed to create config dir: {e}"),
            )
        })?;
    }

    // Write config atomically
    let tmp_path = config_path.with_extension("toml.tmp");
    tokio::fs::write(&tmp_path, &content).await.map_err(|e| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("Failed to write config: {e}"),
        )
    })?;
    tokio::fs::rename(&tmp_path, &config_path)
        .await
        .map_err(|e| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("Failed to rename config: {e}"),
            )
        })?;

    Ok(Json(json!({
        "status": "ok",
        "message": format!("Source '{}' added to {}", name, config_path.display()),
        "source": payload
    })))
}

/// Escape a string for TOML output.
fn escape_toml(s: &str) -> String {
    s.replace('\\', "\\\\")
        .replace('"', "\\\"")
        .replace('\n', "\\n")
}

/// DELETE /feeds/sources/{name} — Remove a feed source from config.
pub async fn delete_feed_source(
    Path(name): Path<String>,
    State(state): State<AppState>,
) -> Result<impl IntoResponse, (StatusCode, String)> {
    let cwd = state.core.cwd();
    let config_path = feeds_config_path(cwd);

    if !config_path.exists() {
        return Err((StatusCode::NOT_FOUND, "Config file not found".into()));
    }

    let content = tokio::fs::read_to_string(&config_path).await.map_err(|e| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("Failed to read config: {e}"),
        )
    })?;

    // Find and remove the source block
    let marker_start = format!("[sources.{}]", name);
    let marker_start_alt = format!("sources.{}", name);
    let lines: Vec<&str> = content.lines().collect();
    let mut new_lines: Vec<String> = Vec::new();
    let mut in_source_block = false;
    let mut found = false;

    for line in &lines {
        let trimmed = line.trim();

        if trimmed == marker_start || trimmed.starts_with(&marker_start_alt) {
            in_source_block = true;
            found = true;
            continue;
        }

        if in_source_block {
            // End of source block: next section header or EOF
            if trimmed.starts_with('[') && !trimmed.starts_with("[sources.") {
                in_source_block = false;
                new_lines.push(line.to_string());
            }
            // Skip lines inside the source block
            continue;
        }

        new_lines.push(line.to_string());
    }

    if !found {
        return Err((
            StatusCode::NOT_FOUND,
            format!("Source '{}' not found", name),
        ));
    }

    let new_content = new_lines.join("\n");

    // Write config atomically
    let tmp_path = config_path.with_extension("toml.tmp");
    tokio::fs::write(&tmp_path, &new_content)
        .await
        .map_err(|e| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("Failed to write config: {e}"),
            )
        })?;
    tokio::fs::rename(&tmp_path, &config_path)
        .await
        .map_err(|e| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("Failed to rename config: {e}"),
            )
        })?;

    Ok(Json(json!({
        "status": "ok",
        "message": format!("Source '{}' removed from {}", name, config_path.display()),
    })))
}

/// Build feed routes.
pub fn routes() -> Router<AppState> {
    Router::new()
        .route(
            "/feeds/sources",
            get(list_source_types).post(add_feed_source),
        )
        .route("/feeds/sources/{name}", delete(delete_feed_source))
        .route("/feeds/config", get(get_feed_config))
        .route("/feeds/items", get(list_feed_items))
        .route("/feeds/items/{id}", get(get_feed_item))
        .route("/feeds/search", get(search_feed_items))
        .route("/feeds/stats", get(get_feed_stats))
        .route("/feeds/alerts", get(get_feed_alerts))
        .route("/feeds/ingest", post(trigger_ingestion))
}
