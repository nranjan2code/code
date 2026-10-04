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

fn authorize_feed_mutation(state: &AppState, scope: &str) -> Result<(), (StatusCode, String)> {
    let mode = state.core.effective_permission_mode();
    if authorize_feed_scope(scope, mode).is_ok() {
        Ok(())
    } else {
        Err((
            StatusCode::FORBIDDEN,
            format!(
                "feed mutation requires permission for {scope} scope; current mode is {mode:?}"
            ),
        ))
    }
}

fn authorize_feed_scope(scope: &str, mode: vak_config::PermissionMode) -> Result<(), ()> {
    use vak_config::PermissionMode;
    if matches!(
        (scope, mode),
        (
            "workspace",
            PermissionMode::WorkspaceWrite | PermissionMode::FullAccess
        ) | ("global", PermissionMode::FullAccess)
    ) {
        Ok(())
    } else {
        Err(())
    }
}

/// The tenant tree feeds keep their store, log and config in (doc 73 §6).
fn tenant_home() -> PathBuf {
    vak_config::paths::tenant_home_at(
        &vak_config::paths::data_home(),
        vak_config::paths::LOCAL_TENANT,
    )
}

/// What a feed script is told about where it runs. Every path it writes is
/// decided here, in the tenant tree under the canonical data home (so an
/// overridden `VAK_HOME` holds everything), never guessed by the script.
fn feed_environment(
    cwd: &std::path::Path,
    scripts: &std::path::Path,
) -> Vec<(&'static str, String)> {
    let tenant = tenant_home();
    let path = |p: PathBuf| p.to_string_lossy().into_owned();
    let mut env = vec![
        ("PYTHONPATH", path(scripts.to_path_buf())),
        ("VAK_FEED_WORKSPACE", path(cwd.to_path_buf())),
        (
            "VAK_FEEDS_DB",
            path(tenant.join("feeds").join("feeds.duckdb")),
        ),
        (
            "VAK_FEEDS_LOG",
            path(tenant.join("feeds").join("security.log")),
        ),
        ("VAK_FEEDS_CONFIG", path(global_feeds_config_path())),
    ];
    // The subprocess runs with `env_clear` (see `run_feed_script`), so pass
    // the operational minimum the interpreter needs and nothing else: PATH to
    // resolve `python3`, HOME for the per-user source registry. The parent's
    // secrets — `VAK_GATEWAY_TOKEN`, provider keys — never reach a feed
    // subprocess (invariant 12); before this they were inherited wholesale.
    for key in ["PATH", "HOME"] {
        if let Ok(value) = std::env::var(key) {
            env.push((key, value));
        }
    }
    env
}

/// Feed pipeline Python script directory, resolved **only** from the running
/// binary's own location — the install's bundled `feeds`, or, in a dev build,
/// the source checkout the binary was built in. It is never resolved from the
/// session workspace: a workspace is untrusted input, and taking the script to
/// run from `cwd/scripts/feeds` let a cloned repository supply the code the
/// server executed unattended (invariants 12, 14, 15).
fn feeds_dir() -> PathBuf {
    let Ok(exe) = std::env::current_exe() else {
        return PathBuf::from("feeds");
    };
    let Some(bin_dir) = exe.parent() else {
        return PathBuf::from("feeds");
    };
    let mut candidates = vec![
        bin_dir.join("feeds"),
        bin_dir.join("..").join("Resources").join("feeds"),
        bin_dir.join("..").join("share").join("vak").join("feeds"),
        bin_dir
            .join("..")
            .join("..")
            .join("share")
            .join("vak")
            .join("feeds"),
    ];
    // Dev build: the binary lives under the checkout's `target/`, so the
    // repo's `scripts/feeds` is an ancestor of the executable — trusted
    // because it is where this binary came from, and never the workspace.
    for ancestor in exe.ancestors() {
        candidates.push(ancestor.join("scripts").join("feeds"));
    }
    candidates
        .into_iter()
        .find(|candidate| candidate.join("feed_ingest.py").exists())
        .unwrap_or_else(|| bin_dir.join("feeds"))
}

fn validate_source_url(raw: &str) -> Result<(), String> {
    let url = reqwest::Url::parse(raw).map_err(|_| "source URL is not valid".to_string())?;
    if !matches!(url.scheme(), "http" | "https") || url.host_str().is_none() {
        return Err("source URL must use http or https and include a host".into());
    }
    let host = url.host_str().unwrap_or_default();
    let host_for_ip = host.trim_matches(|character| character == '[' || character == ']');
    let private_ip = host_for_ip
        .parse::<std::net::IpAddr>()
        .ok()
        .is_some_and(|ip| match ip {
            std::net::IpAddr::V4(ip) => ip.is_private() || ip.is_loopback() || ip.is_link_local(),
            std::net::IpAddr::V6(ip) => {
                ip.is_loopback() || ip.is_unique_local() || ip.is_unicast_link_local()
            }
        });
    if private_ip || host.eq_ignore_ascii_case("localhost") || host.ends_with(".localhost") {
        return Err("source URL targets a local or private address".into());
    }
    Ok(())
}

/// Config file path for feed sources, in the canonical data home.
///
/// This used to hand-roll a second, unsupported feed configuration layout.
/// convention nothing else in the tree used — and `HOME` was read with
/// `unwrap_or_default()`, so an unset `HOME` produced a *relative* path
/// resolved against whatever directory the server happened to start in.
fn feeds_config_path(_cwd: &std::path::Path) -> PathBuf {
    vak_config::scope::WorkspaceScope::new(_cwd).feeds_config()
}

fn global_feeds_config_path() -> PathBuf {
    tenant_home().join("feeds.toml")
}

/// Run a Python feed script and return its JSON output.
async fn run_feed_script(
    cwd: &std::path::Path,
    script: &str,
    args: &[&str],
) -> Result<Value, (StatusCode, String)> {
    let dir = feeds_dir();
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
        .env_clear()
        .envs(feed_environment(cwd, &dir))
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

/// Like `run_feed_script`, but for the admin-console CRUD subcommands
/// (`--remove-source` / `--update-source`) that print `{"status": "ok" |
/// "not_found", ...}` to stdout and use the exit code only as a signal,
/// not as proof of a crash. `run_feed_script` treats any non-zero exit
/// as a hard failure and only looks at stderr, which would swallow the
/// well-formed "not found" response these emit on stdout. Parses stdout
/// as JSON regardless of exit status; only a genuine crash (unparseable
/// stdout) surfaces via stderr.
async fn run_feed_admin_script(
    cwd: &std::path::Path,
    script: &str,
    args: &[&str],
) -> Result<Value, (StatusCode, String)> {
    let dir = feeds_dir();
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
        .env_clear()
        .envs(feed_environment(cwd, &dir))
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

    let stdout = String::from_utf8_lossy(&output.stdout);
    serde_json::from_str(&stdout).map_err(|_| {
        let stderr = String::from_utf8_lossy(&output.stderr);
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("Feed script error: {stderr}"),
        )
    })
}

/// Run a request through the feed MCP server.
async fn run_feed_mcp_request(
    cwd: &std::path::Path,
    request: &Value,
) -> Result<Value, (StatusCode, String)> {
    let dir = feeds_dir();
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
        .env_clear()
        .envs(feed_environment(cwd, &dir))
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
            return unwrap_mcp_response(response);
        }
    }

    Err((
        StatusCode::INTERNAL_SERVER_ERROR,
        "No valid response from MCP server".into(),
    ))
}

/// Unwrap a JSON-RPC `tools/call` response into the plain structured payload
/// the frontend expects (`{items: [...]}`, `{alerts: [...]}`, etc.) instead of
/// the raw MCP envelope (`{jsonrpc, id, result: {content, structuredContent}}`).
fn unwrap_mcp_response(response: Value) -> Result<Value, (StatusCode, String)> {
    if let Some(err) = response.get("error") {
        let msg = err
            .get("message")
            .and_then(|m| m.as_str())
            .unwrap_or("MCP error");
        return Err((StatusCode::INTERNAL_SERVER_ERROR, msg.to_string()));
    }

    let result = response.get("result").cloned().unwrap_or(json!({}));

    if result
        .get("isError")
        .and_then(|v| v.as_bool())
        .unwrap_or(false)
    {
        let msg = result
            .get("content")
            .and_then(|c| c.get(0))
            .and_then(|c| c.get("text"))
            .and_then(|t| t.as_str())
            .unwrap_or("MCP tool error");
        return Err((StatusCode::INTERNAL_SERVER_ERROR, msg.to_string()));
    }

    if let Some(structured) = result.get("structuredContent") {
        return Ok(structured.clone());
    }

    Ok(result)
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

/// GET /feeds/config — Get the effective scoped feed projection.
pub async fn get_feed_config(
    State(state): State<AppState>,
) -> Result<impl IntoResponse, (StatusCode, String)> {
    let cwd = state.core.cwd();
    let sources = run_feed_mcp_request(
        cwd,
        &json!({"jsonrpc":"2.0","id":1,"method":"tools/call","params":{"name":"feed_sources","arguments":{}}}),
    ).await?;
    let alerts = run_feed_mcp_request(
        cwd,
        &json!({"jsonrpc":"2.0","id":2,"method":"tools/call","params":{"name":"feed_alerts","arguments":{}}}),
    ).await?;
    let stats = run_feed_script(cwd, "feed_ingest.py", &["--stats"]).await?;
    Ok(Json(
        json!({"sources": sources, "alerts": alerts, "stats": stats}),
    ))
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
    let result = match result {
        Value::Object(mut payload) => {
            if let Some(items) = payload.remove("results") {
                payload.insert("items".into(), items);
            }
            Value::Object(payload)
        }
        value => value,
    };
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
    run_feed_script(cwd, "feed_ingest.py", &["--sync-alerts"]).await?;
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

/// GET /feeds/runs — List recent ingestion receipts for the active workspace.
pub async fn get_feed_runs(
    State(state): State<AppState>,
) -> Result<impl IntoResponse, (StatusCode, String)> {
    let cwd = state.core.cwd();
    let request = json!({
        "jsonrpc": "2.0",
        "id": 1,
        "method": "tools/call",
        "params": {"name": "feed_runs", "arguments": {"limit": 20}}
    });
    Ok(Json(run_feed_mcp_request(cwd, &request).await?))
}

pub async fn get_feed_quarantine(
    State(state): State<AppState>,
) -> Result<impl IntoResponse, (StatusCode, String)> {
    let cwd = state.core.cwd();
    let request = json!({
        "jsonrpc": "2.0", "id": 1, "method": "tools/call",
        "params": {"name": "feed_quarantine", "arguments": {"limit": 50}}
    });
    Ok(Json(run_feed_mcp_request(cwd, &request).await?))
}

pub async fn release_feed_item(
    Path(id): Path<i64>,
    State(state): State<AppState>,
) -> Result<impl IntoResponse, (StatusCode, String)> {
    authorize_feed_mutation(&state, "workspace")?;
    let cwd = state.core.cwd();
    let id_text = id.to_string();
    let result =
        run_feed_admin_script(cwd, "feed_ingest.py", &["--release-item", &id_text]).await?;
    if result.get("status").and_then(Value::as_str) != Some("ok") {
        return Err((StatusCode::NOT_FOUND, format!("Item {id} not found")));
    }
    Ok(Json(result))
}

/// POST /feeds/ingest — Trigger manual ingestion.
pub async fn trigger_ingestion(
    State(state): State<AppState>,
) -> Result<impl IntoResponse, (StatusCode, String)> {
    authorize_feed_mutation(&state, "workspace")?;
    let cwd = state.core.cwd();
    let workspace = cwd.to_string_lossy();
    let mut result = run_feed_script(cwd, "feed_ingest.py", &["--workspace", &workspace]).await?;
    let intents = result
        .get("delivery_intents")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    if !intents.is_empty() {
        let delivered = deliver_and_ack_alerts(&state, &intents)
            .await
            .map_err(|error| (StatusCode::BAD_GATEWAY, error))?;
        result["alerts_delivered"] = json!(delivered);
    }
    Ok(Json(result))
}

pub async fn scheduled_ingestion(state: &AppState) {
    if !state.core.config().feeds.enabled {
        return;
    }
    if matches!(
        state.core.effective_permission_mode(),
        vak_config::PermissionMode::ReadOnly
    ) {
        return;
    }
    let cwd = state.core.cwd();
    let workspace = cwd.to_string_lossy();
    let Ok(mut result) = run_feed_script(cwd, "feed_ingest.py", &["--workspace", &workspace]).await
    else {
        return;
    };
    let intents = result
        .get("delivery_intents")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    if intents.is_empty() {
        return;
    }
    if let Ok(delivered) = deliver_and_ack_alerts(state, &intents).await {
        result["alerts_delivered"] = json!(delivered);
    }
}

async fn deliver_and_ack_alerts(state: &AppState, intents: &[Value]) -> Result<usize, String> {
    let delivered = crate::delivery::deliver_feed_intents(&state.core, intents).await?;
    for intent in intents {
        let Some(alert_id) = intent.get("alert_id").and_then(Value::as_i64) else {
            continue;
        };
        let Some(item_id) = intent
            .get("item")
            .and_then(|item| item.get("id"))
            .and_then(Value::as_i64)
        else {
            continue;
        };
        let alert_text = alert_id.to_string();
        let item_text = item_id.to_string();
        let result = run_feed_admin_script(
            state.core.cwd(),
            "feed_ingest.py",
            &["--mark-alert-delivered", &alert_text, &item_text],
        )
        .await
        .map_err(|(_, error)| error)?;
        if result.get("status").and_then(Value::as_str) != Some("ok") {
            return Err(format!(
                "could not acknowledge alert {alert_id} for item {item_id}"
            ));
        }
    }
    Ok(delivered)
}

/// POST /feeds/sources — Add a new source to feeds.toml.
pub async fn add_feed_source(
    State(state): State<AppState>,
    Json(payload): Json<Value>,
) -> Result<impl IntoResponse, (StatusCode, String)> {
    let cwd = state.core.cwd();
    let scope = payload
        .get("scope")
        .and_then(Value::as_str)
        .unwrap_or("workspace");
    authorize_feed_mutation(&state, scope)?;
    let config_path = if scope == "global" {
        global_feeds_config_path()
    } else {
        feeds_config_path(cwd)
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
    if !url.is_empty() {
        validate_source_url(url).map_err(|error| (StatusCode::BAD_REQUEST, error))?;
    }
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

    let source_id = payload
        .get("id")
        .and_then(Value::as_str)
        .filter(|id| !id.trim().is_empty())
        .map(ToOwned::to_owned)
        .unwrap_or_else(|| format!("src-{}", uuid::Uuid::now_v7().simple()));

    // Build the new source block
    let mut source_block = format!(
        "\n[[sources]]\nid = \"{}\"\nname = \"{}\"\ntype = \"{}\"\n",
        escape_toml(&source_id),
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

    let source_id_line = format!("id = \"{}\"", escape_toml(&source_id));
    let conflict = format!("source id '{source_id}' already exists in {scope} scope");
    edit_feeds_config(config_path.clone(), move |current| {
        let mut content = current.map_or_else(default_feeds_config, ToOwned::to_owned);
        if content.lines().any(|line| line.trim() == source_id_line) {
            return Err((StatusCode::CONFLICT, conflict));
        }
        append_block(&mut content, &source_block);
        Ok((Some(content), ()))
    })
    .await?;

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

/// Find the line range `[start, end)` of a `[[table]]` array-of-tables block
/// whose `name = "..."` field matches `name`. `end` is the index of the first
/// line after the block: the next `[[...]]` header, a `[section]` header that
/// isn't a dotted sub-table of this array item (e.g. `[alerts.match]` stays
/// part of the `[[alerts]]` block it follows), or EOF.
fn find_array_table_block(lines: &[&str], table: &str, name: &str) -> Option<(usize, usize)> {
    let header = format!("[[{}]]", table);
    let subtable_prefix = format!("[{}.", table);
    let mut i = 0;
    while i < lines.len() {
        if lines[i].trim() == header {
            let start = i;
            let mut j = i + 1;
            let mut block_key: Option<String> = None;
            while j < lines.len() {
                let trimmed = lines[j].trim();
                let is_boundary = trimmed.starts_with("[[")
                    || (trimmed.starts_with('[') && !trimmed.starts_with(&subtable_prefix));
                if is_boundary {
                    break;
                }
                if let Some(rest) = trimmed.strip_prefix("id") {
                    let rest = rest.trim_start();
                    if let Some(rest) = rest.strip_prefix('=') {
                        block_key = Some(unescape_toml(rest.trim().trim_matches('"')));
                    }
                } else if block_key.is_none()
                    && let Some(rest) = trimmed.strip_prefix("name")
                {
                    let rest = rest.trim_start();
                    if let Some(rest) = rest.strip_prefix('=') {
                        let value = rest.trim().trim_matches('"');
                        block_key = Some(unescape_toml(value));
                    }
                }
                j += 1;
            }
            if block_key.as_deref() == Some(name) {
                return Some((start, j));
            }
            i = j;
        } else {
            i += 1;
        }
    }
    None
}

/// Reverse of `escape_toml` for simple double-quoted TOML string values.
fn unescape_toml(s: &str) -> String {
    s.replace("\\\"", "\"").replace("\\\\", "\\")
}

/// Remove a `[[table]]` block by name from TOML `content`. Returns the new
/// content, or `None` if no matching block was found.
fn remove_array_table_block(content: &str, table: &str, name: &str) -> Option<String> {
    let lines: Vec<&str> = content.lines().collect();
    let (start, end) = find_array_table_block(&lines, table, name)?;
    let mut new_lines: Vec<&str> = Vec::with_capacity(lines.len());
    new_lines.extend_from_slice(&lines[..start]);
    new_lines.extend_from_slice(&lines[end..]);
    Some(new_lines.join("\n"))
}

/// Set (or add) a scalar `field = value` line inside a `[[table]]` block
/// matched by name, replacing the existing line for that field if present.
fn set_field_in_block(
    content: &str,
    table: &str,
    name: &str,
    field: &str,
    value_line: &str,
) -> Option<String> {
    let lines: Vec<&str> = content.lines().collect();
    let (start, end) = find_array_table_block(&lines, table, name)?;
    let mut new_lines: Vec<String> = lines[..=start].iter().map(|l| l.to_string()).collect();
    let mut replaced = false;
    for line in &lines[start + 1..end] {
        let trimmed = line.trim();
        if trimmed.starts_with(field) && trimmed[field.len()..].trim_start().starts_with('=') {
            new_lines.push(value_line.to_string());
            replaced = true;
        } else {
            new_lines.push(line.to_string());
        }
    }
    if !replaced {
        new_lines.push(value_line.to_string());
    }
    new_lines.extend(lines[end..].iter().map(|l| l.to_string()));
    Some(new_lines.join("\n"))
}

fn default_feeds_config() -> String {
    "[general]\ndefault_check_interval = \"30m\"\nmax_items_per_feed = 500\ndedup_window_days = 90\n\n".to_string()
}

fn append_block(content: &mut String, block: &str) {
    if !content.is_empty() && !content.ends_with('\n') {
        content.push('\n');
    }
    content.push_str(block);
}

/// Every edit to a `feeds.toml` goes through here: `edit` sees the current
/// text (`None` when the file does not exist) and returns the new text, or
/// `None` to leave it alone. The read, the edit and the atomic replace hold
/// one lock (`vak_config::file_update`), so concurrent edits never publish a
/// half-written file or rewrite the file from a stale read.
async fn edit_feeds_config<T: Send + 'static>(
    config_path: PathBuf,
    edit: impl FnOnce(Option<&str>) -> Result<(Option<String>, T), (StatusCode, String)>
    + Send
    + 'static,
) -> Result<T, (StatusCode, String)> {
    tokio::task::spawn_blocking(move || {
        vak_config::file_update::update_file(&config_path, edit).map_err(|error| match error {
            vak_config::file_update::UpdateError::Edit(error) => error,
            vak_config::file_update::UpdateError::Io { path, source } => (
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("Failed to update {}: {source}", path.display()),
            ),
        })
    })
    .await
    .map_err(|error| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("Feed config update did not finish: {error}"),
        )
    })?
}

/// DELETE /feeds/sources/{name} — Remove a feed source from config.
/// DELETE /feeds/sources/{name} — Remove a feed source.
///
/// Soft-deletes the materialized source row and removes its declaration from
/// the canonical scope-specific configuration.
pub async fn delete_feed_source(
    Path(source_id): Path<String>,
    Query(params): Query<HashMap<String, String>>,
    State(state): State<AppState>,
) -> Result<impl IntoResponse, (StatusCode, String)> {
    let cwd = state.core.cwd();
    let scope = params
        .get("scope")
        .map(String::as_str)
        .unwrap_or("workspace");
    authorize_feed_mutation(&state, scope)?;

    let removed = run_feed_admin_script(
        cwd,
        "feed_ingest.py",
        &["--remove-source", &source_id, "--scope", scope],
    )
    .await?
    .get("status")
    .and_then(|s| s.as_str())
        == Some("ok");

    if !removed {
        return Err((
            StatusCode::NOT_FOUND,
            format!("Source '{}' not found", source_id),
        ));
    }

    let config_path = if scope == "global" {
        global_feeds_config_path()
    } else {
        feeds_config_path(cwd)
    };
    let declared = source_id.clone();
    let _ = edit_feeds_config(config_path, move |current| {
        let next =
            current.and_then(|content| remove_array_table_block(content, "sources", &declared));
        Ok((next, ()))
    })
    .await;

    Ok(Json(json!({
        "status": "ok",
        "message": format!("Source '{}' removed", source_id),
    })))
}

/// PATCH /feeds/sources/{name} — Update a source's enabled/interval/trust fields.
///
/// `tags` is **refused** here. The `feeds` table has no tags column, so a
/// tag written on update reached feeds.toml and nothing else: the DB never
/// saw it and the UI never showed it. A PATCH that silently half-applies
/// is worse than one that says no, so the caller is told where tags are
/// actually set instead (AGENTS.md invariant 30).
pub async fn update_feed_source(
    Path(source_id): Path<String>,
    Query(params): Query<HashMap<String, String>>,
    State(state): State<AppState>,
    Json(payload): Json<Value>,
) -> Result<impl IntoResponse, (StatusCode, String)> {
    let cwd = state.core.cwd();
    let scope = params
        .get("scope")
        .map(String::as_str)
        .unwrap_or("workspace");
    authorize_feed_mutation(&state, scope)?;

    let enabled = payload.get("enabled").and_then(|v| v.as_bool());
    let interval = payload.get("interval").and_then(|v| v.as_str());
    let trust = payload.get("trust").and_then(|v| v.as_str());
    if payload.get("tags").is_some() {
        return Err((
            StatusCode::BAD_REQUEST,
            "tags cannot be updated here: the feeds table has no tags column, so the \
             change would reach feeds.toml and nothing else. Set tags when creating \
             the source."
                .into(),
        ));
    }

    if enabled.is_none() && interval.is_none() && trust.is_none() {
        return Err((
            StatusCode::BAD_REQUEST,
            "No recognized fields to update (enabled, interval, trust)".into(),
        ));
    }

    if enabled.is_some() || interval.is_some() || trust.is_some() {
        let mut args: Vec<String> = vec!["--update-source".into(), source_id.clone()];
        args.extend(["--scope".into(), scope.into()]);
        if let Some(e) = enabled {
            args.push("--set-enabled".into());
            args.push(if e { "true" } else { "false" }.into());
        }
        if let Some(i) = interval {
            args.push("--set-interval".into());
            args.push(i.into());
        }
        if let Some(t) = trust {
            args.push("--set-trust".into());
            args.push(t.into());
        }
        let arg_refs: Vec<&str> = args.iter().map(|s| s.as_str()).collect();
        let updated = run_feed_admin_script(cwd, "feed_ingest.py", &arg_refs)
            .await?
            .get("status")
            .and_then(|s| s.as_str())
            == Some("ok");
        if !updated {
            return Err((
                StatusCode::NOT_FOUND,
                format!("Source '{}' not found", source_id),
            ));
        }
    }

    // Reconcile the durable declaration when this source has one. The
    // materialized row above remains authoritative for the current response.
    let config_path = if scope == "global" {
        global_feeds_config_path()
    } else {
        feeds_config_path(cwd)
    };
    let mut fields = Vec::new();
    if let Some(e) = enabled {
        fields.push(("enabled", format!("enabled = {e}")));
    }
    if let Some(i) = interval {
        fields.push(("interval", format!("interval = \"{}\"", escape_toml(i))));
    }
    if let Some(t) = trust {
        fields.push(("trust", format!("trust = \"{}\"", escape_toml(t))));
    }
    let declared = source_id.clone();
    let _ = edit_feeds_config(config_path, move |current| {
        let Some(mut content) = current.map(ToOwned::to_owned) else {
            return Ok((None, ()));
        };
        let mut touched = false;
        for (field, line) in &fields {
            if let Some(next) = set_field_in_block(&content, "sources", &declared, field, line) {
                content = next;
                touched = true;
            }
        }
        Ok((touched.then_some(content), ()))
    })
    .await;

    Ok(Json(json!({
        "status": "ok",
        "message": format!("Source '{}' updated", source_id),
    })))
}

/// GET /feeds/sources/configured — List sources actually known to the DB
/// (populated by ingestion), with live enabled/trust/interval status.
pub async fn list_configured_sources(
    State(state): State<AppState>,
) -> Result<impl IntoResponse, (StatusCode, String)> {
    let cwd = state.core.cwd();
    let request = json!({
        "jsonrpc": "2.0",
        "id": 1,
        "method": "tools/call",
        "params": {
            "name": "feed_sources",
            "arguments": {}
        }
    });
    let result = run_feed_mcp_request(cwd, &request).await?;
    Ok(Json(result))
}

/// POST /feeds/alerts — Add a new alert rule to feeds.toml.
pub async fn add_feed_alert(
    State(state): State<AppState>,
    Json(payload): Json<Value>,
) -> Result<impl IntoResponse, (StatusCode, String)> {
    let cwd = state.core.cwd();
    let scope = payload
        .get("scope")
        .and_then(Value::as_str)
        .unwrap_or("workspace");
    authorize_feed_mutation(&state, scope)?;
    let config_path = if scope == "global" {
        global_feeds_config_path()
    } else {
        feeds_config_path(cwd)
    };

    let name = payload
        .get("name")
        .and_then(|v| v.as_str())
        .filter(|s| !s.is_empty())
        .ok_or((StatusCode::BAD_REQUEST, "Alert 'name' is required".into()))?;

    let str_array = |key: &str| -> Vec<String> {
        payload
            .get(key)
            .and_then(|v| v.as_array())
            .map(|a| {
                a.iter()
                    .filter_map(|v| v.as_str())
                    .map(String::from)
                    .collect()
            })
            .unwrap_or_default()
    };
    let keywords = str_array("keywords");
    let tags = str_array("tags");
    let sources = str_array("sources");

    if keywords.is_empty() && tags.is_empty() && sources.is_empty() {
        return Err((
            StatusCode::BAD_REQUEST,
            "At least one of keywords, tags, or sources must be set".into(),
        ));
    }

    let action = payload
        .get("action")
        .and_then(|v| v.as_str())
        .unwrap_or("deliver");
    if !matches!(action, "deliver" | "hook" | "both") {
        return Err((
            StatusCode::BAD_REQUEST,
            "action must be one of: deliver, hook, both".into(),
        ));
    }
    let deliver_to = payload
        .get("deliver_to")
        .and_then(|v| v.as_str())
        .unwrap_or("");
    if matches!(action, "deliver" | "both") && deliver_to.trim().is_empty() {
        return Err((
            StatusCode::BAD_REQUEST,
            "deliver_to is required for deliver and both alerts".into(),
        ));
    }
    let cooldown_minutes = payload
        .get("cooldown_minutes")
        .and_then(|v| v.as_i64())
        .unwrap_or(30);

    let to_toml_array = |items: &[String]| -> String {
        items
            .iter()
            .map(|t| format!("\"{}\"", escape_toml(t)))
            .collect::<Vec<_>>()
            .join(", ")
    };

    let mut block = format!(
        "\n[[alerts]]\nname = \"{}\"\naction = \"{}\"\ncooldown_minutes = {}\nenabled = true\n",
        escape_toml(name),
        escape_toml(action),
        cooldown_minutes
    );
    if !deliver_to.is_empty() {
        block.push_str(&format!("deliver_to = \"{}\"\n", escape_toml(deliver_to)));
    }
    block.push_str("\n[alerts.match]\n");
    if !keywords.is_empty() {
        block.push_str(&format!("keywords = [{}]\n", to_toml_array(&keywords)));
    }
    if !tags.is_empty() {
        block.push_str(&format!("tags = [{}]\n", to_toml_array(&tags)));
    }
    if !sources.is_empty() {
        block.push_str(&format!("sources = [{}]\n", to_toml_array(&sources)));
    }

    edit_feeds_config(config_path.clone(), move |current| {
        let mut content = current.map_or_else(default_feeds_config, ToOwned::to_owned);
        append_block(&mut content, &block);
        Ok((Some(content), ()))
    })
    .await?;
    run_feed_script(cwd, "feed_ingest.py", &["--sync-alerts"]).await?;

    Ok(Json(json!({
        "status": "ok",
        "message": format!("Alert '{}' added to {}", name, config_path.display()),
    })))
}

/// DELETE /feeds/alerts/{name} — Remove an alert rule from config.
pub async fn delete_feed_alert(
    Path(name): Path<String>,
    Query(params): Query<HashMap<String, String>>,
    State(state): State<AppState>,
) -> Result<impl IntoResponse, (StatusCode, String)> {
    let cwd = state.core.cwd();
    let scope = params
        .get("scope")
        .map(String::as_str)
        .unwrap_or("workspace");
    authorize_feed_mutation(&state, scope)?;
    let config_path = if scope == "global" {
        global_feeds_config_path()
    } else {
        feeds_config_path(cwd)
    };

    let alert = name.clone();
    edit_feeds_config(config_path.clone(), move |current| {
        let content =
            current.ok_or_else(|| (StatusCode::NOT_FOUND, "Config file not found".to_string()))?;
        let next = remove_array_table_block(content, "alerts", &alert)
            .ok_or_else(|| (StatusCode::NOT_FOUND, format!("Alert '{alert}' not found")))?;
        Ok((Some(next), ()))
    })
    .await?;
    run_feed_script(cwd, "feed_ingest.py", &["--sync-alerts"]).await?;

    Ok(Json(json!({
        "status": "ok",
        "message": format!("Alert '{}' removed from {}", name, config_path.display()),
    })))
}

/// Build feed routes.
pub fn routes() -> Router<AppState> {
    Router::new()
        .route(
            "/feeds/sources",
            get(list_source_types).post(add_feed_source),
        )
        .route("/feeds/sources/configured", get(list_configured_sources))
        .route(
            "/feeds/sources/{name}",
            delete(delete_feed_source).patch(update_feed_source),
        )
        .route("/feeds/config", get(get_feed_config))
        .route("/feeds/items", get(list_feed_items))
        .route("/feeds/items/{id}", get(get_feed_item))
        .route("/feeds/search", get(search_feed_items))
        .route("/feeds/stats", get(get_feed_stats))
        .route("/feeds/alerts", get(get_feed_alerts).post(add_feed_alert))
        .route("/feeds/runs", get(get_feed_runs))
        .route("/feeds/quarantine", get(get_feed_quarantine))
        .route("/feeds/quarantine/{id}/release", post(release_feed_item))
        .route("/feeds/alerts/{name}", delete(delete_feed_alert))
        .route("/feeds/ingest", post(trigger_ingestion))
}

#[cfg(test)]
#[allow(clippy::panic, clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::{authorize_feed_scope, feed_environment, feeds_dir, validate_source_url};
    use vak_config::PermissionMode;

    /// The feed subprocess runs with `env_clear`, so `feed_environment` is the
    /// complete list of what it sees. It must carry the operational minimum
    /// (PATH, so `python3` resolves) and never a parent secret — before this,
    /// the subprocess inherited the whole server environment, `VAK_GATEWAY_TOKEN`
    /// and provider keys included (invariant 12).
    #[test]
    fn feed_subprocess_environment_is_a_secret_free_allowlist() {
        let _home = vak_config::paths::isolate_home_for_tests();
        let environment = feed_environment(
            std::path::Path::new("/tmp/ws"),
            std::path::Path::new("/tmp/scripts"),
        );
        let allowed = [
            "PYTHONPATH",
            "VAK_FEED_WORKSPACE",
            "VAK_FEEDS_DB",
            "VAK_FEEDS_LOG",
            "VAK_FEEDS_CONFIG",
            "PATH",
            "HOME",
        ];
        for (key, _) in &environment {
            assert!(allowed.contains(key), "{key} is not in the feed allowlist");
        }
        assert!(
            environment.iter().any(|(key, _)| *key == "PATH"),
            "PATH must be passed so python3 resolves under env_clear"
        );
        for secret in ["VAK_GATEWAY_TOKEN", "ANTHROPIC_API_KEY", "OPENAI_API_KEY"] {
            assert!(
                !environment.iter().any(|(key, _)| *key == secret),
                "{secret} must never be passed to a feed subprocess"
            );
        }
    }

    /// The script the server runs is located only from the binary, never from
    /// the session workspace: a workspace with its own `scripts/feeds` cannot
    /// supply the code the server executes (invariants 12, 14, 15).
    #[test]
    fn feeds_dir_never_resolves_from_a_workspace() {
        let workspace = tempfile::tempdir().expect("tempdir");
        let planted = workspace.path().join("scripts").join("feeds");
        std::fs::create_dir_all(&planted).expect("mkdir");
        std::fs::write(planted.join("feed_ingest.py"), b"raise SystemExit\n").expect("write");
        assert!(
            !feeds_dir().starts_with(workspace.path()),
            "feeds_dir resolved a script from the session workspace"
        );
    }

    /// Every path a feed script writes comes from the canonical data home,
    /// so an overridden `VAK_HOME` holds the feed store, its log and its
    /// config; the scripts used to work out the platform default for
    /// themselves and wrote outside it.
    #[test]
    fn feeds_write_under_overridden_home() {
        let home = vak_config::paths::isolate_home_for_tests();
        let environment = feed_environment(
            std::path::Path::new("/tmp/ws"),
            std::path::Path::new("/tmp/scripts"),
        );
        for name in ["VAK_FEEDS_DB", "VAK_FEEDS_LOG", "VAK_FEEDS_CONFIG"] {
            let (_, value) = environment
                .iter()
                .find(|(key, _)| *key == name)
                .unwrap_or_else(|| panic!("{name} is passed"));
            assert!(
                std::path::Path::new(value).starts_with(home.join("tenants")),
                "{name} = {value} is outside the tenant tree of {}",
                home.display()
            );
        }
    }

    #[test]
    fn feed_scope_permissions_only_tighten() {
        assert!(authorize_feed_scope("workspace", PermissionMode::WorkspaceWrite).is_ok());
        assert!(authorize_feed_scope("workspace", PermissionMode::FullAccess).is_ok());
        assert!(authorize_feed_scope("global", PermissionMode::FullAccess).is_ok());
        assert!(authorize_feed_scope("global", PermissionMode::WorkspaceWrite).is_err());
        assert!(authorize_feed_scope("workspace", PermissionMode::ReadOnly).is_err());
        assert!(authorize_feed_scope("global", PermissionMode::ReadOnly).is_err());
    }

    #[test]
    fn source_admission_rejects_private_and_malformed_targets() {
        assert!(validate_source_url("https://example.com/feed").is_ok());
        assert!(validate_source_url("not a url").is_err());
        assert!(validate_source_url("file:///tmp/feed").is_err());
        assert!(validate_source_url("http://127.0.0.1/feed").is_err());
        assert!(validate_source_url("http://192.168.1.10/feed").is_err());
        assert!(validate_source_url("http://[::1]/feed").is_err());
    }
}
