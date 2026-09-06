//! The browser surface (docs/design/48-web-client.md).
//!
//! Everything here exists so the same workspace client that runs inside
//! the Tauri shell can run in a tab against a headless box: one login for
//! every browser surface, a host descriptor standing in for the shell's
//! `backend_info`, workspace selection backed by `CorePool`, and a
//! directory browser over the *server's* filesystem — because that is the
//! machine whose folders matter.
//!
//! Nothing here re-implements the agent protocol. The client speaks the
//! same `/sessions/*` contract every other surface does; these are only
//! the pieces a browser needs that a native shell provided locally.

use std::path::{Path, PathBuf};

use axum::Json;
use axum::extract::State;
use axum::http::{StatusCode, header};
use axum::response::sse::{Event, KeepAlive, Sse};
use axum::response::{IntoResponse, Response};
use serde::Deserialize;

use crate::AppState;
use crate::admin::SESSION_COOKIE;

// ---- auth: one login for every browser surface -----------------------------

#[derive(Debug, Deserialize)]
pub(crate) struct LoginBody {
    pub token: String,
}

/// Cookie attributes for this deployment.
///
/// `Secure` is conditional and must be: a browser silently DISCARDS a
/// `Secure` cookie delivered over plain http, so setting it unconditionally
/// would make every loopback login appear to succeed and then never
/// persist. It is switched on exactly when the operator has told us the
/// public origin is https (`[server] public_url`), or a terminating proxy
/// says so on the request itself.
fn cookie_attributes(state: &AppState, forwarded_proto: Option<&str>) -> String {
    let cfg = state.core.config();
    let https = cfg.server.cookie_is_secure() || forwarded_proto == Some("https");
    let max_age = cfg.server.session_ttl_hours.saturating_mul(3600);
    let secure = if https { "; Secure" } else { "" };
    format!("HttpOnly; SameSite=Strict; Path=/; Max-Age={max_age}{secure}")
}

fn forwarded_proto(headers: &header::HeaderMap) -> Option<&str> {
    headers
        .get("x-forwarded-proto")
        .and_then(|v| v.to_str().ok())
        .map(|v| v.split(',').next().unwrap_or(v).trim())
}

/// Constant-time token check → HttpOnly session cookie.
///
/// Browsers need this because `EventSource` cannot send an `Authorization`
/// header, so a cookie is the only channel that covers both `fetch` and
/// SSE. The token itself is never stored client-side: it arrives once in
/// this request body and what goes back is HttpOnly, so script can neither
/// read it nor exfiltrate it afterwards.
pub(crate) async fn login(
    State(state): State<AppState>,
    headers: header::HeaderMap,
    Json(body): Json<LoginBody>,
) -> Response {
    use subtle::ConstantTimeEq;
    let ok: bool = body
        .token
        .as_bytes()
        .ct_eq(state.auth_token.as_bytes())
        .into();
    if !ok {
        let ip = headers
            .get("x-forwarded-for")
            .and_then(|v| v.to_str().ok())
            .and_then(|v| v.split(',').next())
            .map(str::trim)
            .filter(|s| !s.is_empty());
        vak_core::security_events::record(
            &state.core.sessions_home(),
            vak_core::security_events::EventKind::AuthFailure,
            "login_failed",
            "invalid token on /auth/login",
            ip,
        );
        state.hub.emit_security("AuthFailure", "login_failed");
        return (
            StatusCode::UNAUTHORIZED,
            Json(serde_json::json!({ "error": "invalid token" })),
        )
            .into_response();
    }
    let attributes = cookie_attributes(&state, forwarded_proto(&headers));
    (
        [(
            header::SET_COOKIE,
            format!("{SESSION_COOKIE}={}; {attributes}", body.token),
        )],
        Json(serde_json::json!({ "ok": true })),
    )
        .into_response()
}

pub(crate) async fn logout() -> Response {
    (
        [(
            header::SET_COOKIE,
            format!("{SESSION_COOKIE}=; HttpOnly; SameSite=Strict; Path=/; Max-Age=0"),
        )],
        Json(serde_json::json!({ "ok": true })),
    )
        .into_response()
}

/// Whether this browser already holds a session.
///
/// Deliberately auth-exempt and deliberately NOT a 401: an unauthenticated
/// client needs to tell "no session yet" (show the login form) apart from
/// "server unreachable" (show an error), and a 401 conflates them.
pub(crate) async fn session_status(
    State(state): State<AppState>,
    headers: header::HeaderMap,
) -> Response {
    use subtle::ConstantTimeEq;
    let held = headers
        .get(header::COOKIE)
        .and_then(|v| v.to_str().ok())
        .and_then(|cookies| {
            cookies.split(';').find_map(|pair| {
                pair.trim()
                    .strip_prefix(&format!("{SESSION_COOKIE}="))
                    .map(str::trim)
                    .map(String::from)
            })
        })
        .map(|value| {
            let ok: bool = value.as_bytes().ct_eq(state.auth_token.as_bytes()).into();
            ok
        })
        .unwrap_or(false);
    if held {
        return Json(serde_json::json!({ "authenticated": true })).into_response();
    }

    // No session yet. On THIS machine, hand one over rather than asking
    // someone to go and find a token to reach their own computer.
    //
    // The probe doubles as the sign-in deliberately: the client already
    // calls it before deciding whether to show a login form, so there is no
    // second endpoint to discover and no extra round trip. Scope is exactly
    // what `[server] loopback_auto_login` describes — loopback only, off if
    // an operator says so, and never reachable from a real hostname.
    let cfg = state.core.config();
    let host = headers.get(header::HOST).and_then(|v| v.to_str().ok());
    if cfg.server.loopback_auto_login && crate::host_is_loopback(host) {
        let attributes = cookie_attributes(&state, forwarded_proto(&headers));
        return (
            [(
                header::SET_COOKIE,
                format!("{SESSION_COOKIE}={}; {attributes}", state.auth_token),
            )],
            Json(serde_json::json!({ "authenticated": true, "granted": "loopback" })),
        )
            .into_response();
    }
    Json(serde_json::json!({ "authenticated": false })).into_response()
}

// ---- the front door --------------------------------------------------------
//
// `/` and its sub-pages moved to `site.rs` when the landing page grew from
// one hand-written file into a multi-page site built from one source
// (docs/design/48-web-client.md §4.6). Only the build stamp those pages
// read stayed here.

/// Build identity, for the public site's footer and build readout.
///
/// Version and commit only. `/health` already answers unauthenticated (it
/// is a liveness probe) but reports provider, model, sandbox and permission
/// mode with it — detail a public front door has no business handing out.
pub(crate) async fn version() -> Response {
    Json(serde_json::json!({
        "version": env!("CARGO_PKG_VERSION"),
        "git_sha": option_env!("VAK_GIT_SHA").unwrap_or("unknown"),
    }))
    .into_response()
}

// ---- host descriptor -------------------------------------------------------

/// What `backend_info` is on the desktop: everything the client needs to
/// know about the process it is talking to, before it knows anything else.
///
/// `base_url` and `token` are deliberately absent. The web client is
/// same-origin and cookie-authenticated, and an absent base URL is how it
/// knows that — see `adoptBackend` in the client's api.ts.
fn host_payload(state: &AppState) -> serde_json::Value {
    let core = state.active_core();
    let cfg = state.core.config();
    // A terminal reaches a real shell, so it is advertised only when the
    // operator enabled it — and, unless they said otherwise, only to
    // loopback. The client renders no Terminal tab at all when this is
    // false: a disabled control that cannot explain itself is worse than
    // an absent one (docs/design/48-web-client.md §6).
    let terminal = cfg.server.web_terminal;
    serde_json::json!({
        "ready": true,
        "cwd": core.cwd().to_string_lossy(),
        "recent_workspaces": recent_workspaces(state),
        "terminal": terminal,
    })
}

pub(crate) async fn host_info(State(state): State<AppState>) -> Response {
    Json(host_payload(&state)).into_response()
}

/// Host-level changes, pushed. Currently one fact — which workspace is
/// active — so this is a low-rate poll folded into a stream rather than a
/// broadcast channel of its own: a workspace switch is a human action, and
/// a second of latency on it is not a defect worth a new event family.
pub(crate) async fn host_events(
    State(state): State<AppState>,
) -> Sse<impl tokio_stream::Stream<Item = Result<Event, std::convert::Infallible>>> {
    use tokio_stream::StreamExt;
    let stream = tokio_stream::wrappers::IntervalStream::new(tokio::time::interval(
        std::time::Duration::from_secs(2),
    ))
    .map(move |_| {
        Ok(Event::default()
            .data(serde_json::to_string(&host_payload(&state)).unwrap_or_else(|_| "{}".into())))
    });
    Sse::new(stream).keep_alive(KeepAlive::default())
}

// ---- workspaces ------------------------------------------------------------

/// Workspaces this server has seen, active one first.
///
/// Read back out of the session ledger rather than kept as its own list.
/// Sessions are already stored per workspace (`<home>/sessions/<hash of
/// cwd>/`) and every ledger header names the cwd it was created in, so the
/// answer is derivable — and a second store of the same fact is a second
/// thing that has to stay true forever (invariant 30). The hash is one-way,
/// hence reading a header rather than reversing a directory name.
fn recent_workspaces(state: &AppState) -> Vec<String> {
    let mut discovered: Vec<PathBuf> = vec![state.active_core().cwd().clone()];
    let root = state.core.sessions_home().join("sessions");
    let Ok(read) = std::fs::read_dir(&root) else {
        return vak_core::workspaces::visible(discovered)
            .into_iter()
            .map(|p| p.to_string_lossy().into_owned())
            .collect();
    };
    // Most recently touched project directory first, which is what makes
    // this a "recents" list rather than an arbitrary one.
    let mut dirs: Vec<(std::time::SystemTime, PathBuf)> = read
        .flatten()
        .map(|entry| entry.path())
        .filter(|path| path.is_dir())
        .map(|path| {
            let modified = std::fs::metadata(&path)
                .and_then(|m| m.modified())
                .unwrap_or(std::time::UNIX_EPOCH);
            (modified, path)
        })
        .collect();
    // Newest first: `Reverse` rather than a flipped comparator, which
    // clippy rightly reads as a sort key spelled the long way.
    dirs.sort_by_key(|(modified, _)| std::cmp::Reverse(*modified));

    for (_, dir) in dirs.into_iter().take(24) {
        if let Some(cwd) = workspace_of_ledger_dir(&dir) {
            discovered.push(PathBuf::from(cwd));
        }
    }
    // `visible` applies the operator's own removals and drops folders that
    // no longer exist. Filtering here rather than in each surface is what
    // stops a ledger rescan from resurrecting something someone removed.
    vak_core::workspaces::visible(discovered)
        .into_iter()
        .take(12)
        .map(|p| p.to_string_lossy().into_owned())
        .collect()
}

/// The cwd recorded in the first readable ledger header under `dir`.
fn workspace_of_ledger_dir(dir: &Path) -> Option<String> {
    let read = std::fs::read_dir(dir).ok()?;
    for entry in read.flatten() {
        let path = entry.path();
        if path.extension().and_then(|e| e.to_str()) != Some("jsonl") {
            continue;
        }
        let Ok(text) = std::fs::read_to_string(&path) else {
            continue;
        };
        // The header is the first line by construction (append-only), so
        // this reads one line rather than parsing a whole transcript.
        let Some(first) = text.lines().next() else {
            continue;
        };
        if let Ok(value) = serde_json::from_str::<serde_json::Value>(first)
            && let Some(cwd) = value
                .get("cwd")
                .or_else(|| value.pointer("/header/cwd"))
                .and_then(|v| v.as_str())
        {
            return Some(cwd.to_string());
        }
    }
    None
}

pub(crate) async fn list_workspaces(State(state): State<AppState>) -> Response {
    let active = state.active_core().cwd().to_string_lossy().into_owned();
    let entries: Vec<serde_json::Value> = recent_workspaces(&state)
        .into_iter()
        .map(|path| {
            let trusted = vak_core::trust::is_trusted(Path::new(&path));
            serde_json::json!({
                "path": path,
                "active": path == active,
                "trusted": trusted,
            })
        })
        .collect();
    Json(serde_json::json!({ "workspaces": entries })).into_response()
}

#[derive(Debug, Deserialize)]
pub(crate) struct OpenWorkspaceBody {
    pub path: String,
    /// The operator's answer when they have just been asked; `None` when
    /// nobody is being asked, in which case the decision already on record
    /// governs. Opening safely is the *absence* of a decision, so there is
    /// nothing to record for it.
    #[serde(default)]
    pub trust: Option<bool>,
}

/// Make `path` the workspace new tasks run in.
///
/// Resolution goes through `CorePool`, which calls `Core::new_with_trust`
/// exactly as a local `vak` run in that folder would (docs/design/34 Phase
/// 2): pooling grants nothing a local session would not already have.
pub(crate) async fn open_workspace(
    State(state): State<AppState>,
    Json(body): Json<OpenWorkspaceBody>,
) -> Response {
    let path = PathBuf::from(body.path.trim());
    let Ok(path) = path.canonicalize() else {
        return (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({ "error": format!("not a directory: {}", body.path) })),
        )
            .into_response();
    };
    if !path.is_dir() {
        return (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({ "error": format!("not a directory: {}", path.display()) })),
        )
            .into_response();
    }
    if !within_roots(&state, &path) {
        return (
            StatusCode::FORBIDDEN,
            Json(serde_json::json!({
                "error": "that folder is outside [server] workspace_roots",
            })),
        )
            .into_response();
    }
    if body.trust == Some(true)
        && let Err(e) = vak_core::trust::record(&path)
    {
        eprintln!("warning: could not record the trust decision: {e}");
    }
    if let Err(e) = vak_config::ensure_project_config(&path) {
        return (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({ "error": e.to_string() })),
        )
            .into_response();
    }
    let core = match state
        .gateway
        .core_pool
        .resolve_at(&path, None, std::time::Instant::now())
    {
        Ok(core) => core.with_surface(vak_core::Surface::Web),
        Err(e) => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(serde_json::json!({ "error": e })),
            )
                .into_response();
        }
    };
    // Opening is also how a removed workspace comes back — there is no
    // separate "restore" verb, because the action a person takes is to
    // open it again.
    if let Err(e) = vak_core::workspaces::remember(&path) {
        eprintln!("warning: could not record the workspace: {e}");
    }
    *state
        .active_core
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner) = Some(core);
    Json(host_payload(&state)).into_response()
}

/// Stop listing a workspace. Its sessions, memory, and settings survive.
///
/// Deliberately NOT a delete: removing a project from a list is a thing
/// people do casually, and it must therefore be a thing that costs nothing
/// to undo. Erasing an append-only ledger is a different operation with
/// different consequences, and it does not live behind this button.
pub(crate) async fn forget_workspace(
    State(state): State<AppState>,
    Json(body): Json<OpenWorkspaceBody>,
) -> Response {
    let path = PathBuf::from(body.path.trim());
    // If the active workspace is being forgotten, fall back to the default core.
    if canonical_eq(&path, state.active_core().cwd()) && !canonical_eq(&path, state.core.cwd()) {
        *state
            .active_core
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner) = None;
    }
    match vak_core::workspaces::forget(&path) {
        Ok(()) => Json(serde_json::json!({ "forgotten": body.path })).into_response(),
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({ "error": e.to_string() })),
        )
            .into_response(),
    }
}

/// Same folder, allowing for symlinks.
fn canonical_eq(a: &Path, b: &Path) -> bool {
    let resolve = |p: &Path| p.canonicalize().unwrap_or_else(|_| p.to_path_buf());
    resolve(a) == resolve(b)
}

// ---- directory browser -----------------------------------------------------

/// Roots the picker may browse. Empty config means the operator's home
/// directory, which is where projects live on every deployment this
/// targets; naming roots explicitly narrows it further.
fn workspace_roots(state: &AppState) -> Vec<PathBuf> {
    let configured = state.core.config().server.workspace_roots.clone();
    if !configured.is_empty() {
        return configured;
    }
    std::env::var_os("HOME")
        .map(PathBuf::from)
        .into_iter()
        .collect()
}

fn within_roots(state: &AppState, path: &Path) -> bool {
    path_within(&workspace_roots(state), path)
}

/// Whether `path` sits under any of `roots`.
///
/// BOTH sides are canonicalized, and that is the whole substance of this
/// function. Comparing a raw path against a canonical root silently fails
/// wherever a path component is a symlink — `/var` is a link to
/// `/private/var` on macOS, `/home` often is on Linux — so the check would
/// reject folders that are genuinely inside a root. Canonicalizing only the
/// root (the first version of this) had exactly that bug.
///
/// A path that does not exist yet cannot be canonicalized, so it falls back
/// to its literal form; callers that matter (`open_workspace`, `list_dirs`)
/// canonicalize before calling anyway, because a folder you are about to
/// open has to exist.
fn path_within(roots: &[PathBuf], path: &Path) -> bool {
    if roots.is_empty() {
        return true; // no HOME and nothing configured: nothing to enforce
    }
    let path = path.canonicalize().unwrap_or_else(|_| path.to_path_buf());
    roots.iter().any(|root| {
        let root = root.canonicalize().unwrap_or_else(|_| root.clone());
        path.starts_with(root)
    })
}

#[derive(Debug, Deserialize)]
pub(crate) struct DirQuery {
    #[serde(default)]
    pub path: Option<String>,
}

/// Directory names under `path`. Never file contents, never files at all —
/// this answers "which folders are there", and nothing more, because that
/// is the whole question the workspace picker asks.
pub(crate) async fn list_dirs(
    State(state): State<AppState>,
    axum::extract::Query(query): axum::extract::Query<DirQuery>,
) -> Response {
    let roots = workspace_roots(&state);
    let here = match query
        .path
        .as_deref()
        .map(str::trim)
        .filter(|p| !p.is_empty())
    {
        Some(path) => match PathBuf::from(path).canonicalize() {
            Ok(path) => path,
            Err(e) => {
                return (
                    StatusCode::BAD_REQUEST,
                    Json(serde_json::json!({ "error": format!("{path}: {e}") })),
                )
                    .into_response();
            }
        },
        None => roots.first().cloned().unwrap_or_else(|| PathBuf::from("/")),
    };
    if !within_roots(&state, &here) {
        return (
            StatusCode::FORBIDDEN,
            Json(serde_json::json!({ "error": "outside [server] workspace_roots" })),
        )
            .into_response();
    }
    let mut entries: Vec<serde_json::Value> = Vec::new();
    if let Ok(read) = std::fs::read_dir(&here) {
        for item in read.flatten() {
            let path = item.path();
            if !path.is_dir() {
                continue;
            }
            let name = item.file_name().to_string_lossy().into_owned();
            // Dotfolders are noise in a project picker, and `.git` is the
            // one thing a repository is *inside*, never a project itself.
            if name.starts_with('.') {
                continue;
            }
            entries.push(serde_json::json!({
                "name": name,
                "path": path.to_string_lossy(),
                "git": path.join(".git").exists(),
            }));
        }
    }
    entries.sort_by(|a, b| a["name"].as_str().cmp(&b["name"].as_str()));

    // Only offer "up" while it stays inside a root: the picker must not be
    // walkable to `/` one click at a time.
    let parent = here
        .parent()
        .filter(|p| within_roots(&state, p))
        .map(|p| p.to_string_lossy().into_owned());

    Json(serde_json::json!({
        "path": here.to_string_lossy(),
        "parent": parent,
        "entries": entries,
    }))
    .into_response()
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;

    /// The picker must not be walkable out of its roots one "up" at a time.
    #[test]
    fn paths_outside_the_roots_are_refused() {
        let dir = tempfile::tempdir().unwrap();
        let roots = vec![dir.path().to_path_buf()];
        assert!(!path_within(&roots, Path::new("/etc")));
        assert!(!path_within(&roots, Path::new("/")));
        assert!(path_within(&roots, dir.path()));
    }

    /// A root reached through a symlink is still that root.
    ///
    /// This is not hypothetical: `tempfile` hands out paths under `/var` on
    /// macOS, which is a symlink to `/private/var`. Canonicalizing only the
    /// root — the first version of this check — rejected every real folder
    /// inside it, and the picker would have refused the operator's own
    /// project directory with "outside workspace_roots".
    #[test]
    fn a_symlinked_root_still_contains_its_children() {
        let dir = tempfile::tempdir().unwrap();
        let child = dir.path().join("project");
        std::fs::create_dir_all(&child).unwrap();
        let roots = vec![dir.path().to_path_buf()];
        assert!(path_within(&roots, &child));
        // And the canonical spelling of the same folder agrees.
        assert!(path_within(&roots, &child.canonicalize().unwrap()));
    }

    /// No roots at all (no HOME, nothing configured) enforces nothing —
    /// but must not accidentally enforce *everything* and lock the picker.
    #[test]
    fn an_empty_root_list_enforces_nothing() {
        assert!(path_within(&[], Path::new("/anywhere")));
    }

    /// The recents list reads a workspace back out of a real ledger header.
    ///
    /// Written against the actual `Entry`/`SessionHeader` types rather than
    /// a hand-rolled JSON string, because the thing that could break this
    /// is precisely the ledger's serialization shape changing — and a test
    /// that hardcodes today's shape would keep passing through exactly the
    /// change it exists to catch.
    #[test]
    fn a_workspace_is_recovered_from_its_ledger_header() {
        use vak_session::types::{Entry, EntryPayload, FrozenContract, SessionHeader};

        let dir = tempfile::tempdir().unwrap();
        let workspace = dir.path().join("some-project");
        std::fs::create_dir_all(&workspace).unwrap();
        let ledger_dir = dir.path().join("ledger");
        std::fs::create_dir_all(&ledger_dir).unwrap();

        let header = SessionHeader {
            session_id: "s1".into(),
            created_at: chrono::Utc::now(),
            cwd: workspace.clone(),
            parent_session_id: None,
            contract_id: None,
            work_item_id: None,
            contract: FrozenContract {
                app_version: "test".into(),
                provider: "p".into(),
                model: "m".into(),
                route_ladder: Vec::new(),
                route_objective: String::new(),
                route_annotations: Vec::new(),
                system_prompt: String::new(),
                permission_mode: "read-only".into(),
                capabilities: Vec::new(),
                prompt_layers: Vec::new(),
            },
        };
        let entry = Entry::new(None, EntryPayload::Header(header));
        std::fs::write(
            ledger_dir.join("s1.jsonl"),
            format!("{}\n", serde_json::to_string(&entry).unwrap()),
        )
        .unwrap();

        assert_eq!(
            workspace_of_ledger_dir(&ledger_dir).as_deref(),
            Some(workspace.to_string_lossy().as_ref()),
            "the recents list could not read a workspace out of a real header"
        );
    }
}

// ---- terminal over WebSocket (docs/design/48-web-client.md §6) -------------
//
// A PTY over HTTP is remote shell access. Everything else the client can
// reach is mediated by the permission engine and the broker (invariants 14
// and 16); a terminal is not — it is the operator's own hands, which is
// exactly what makes it useful and exactly why it does not ship on by
// default.
//
// Three gates, all of which must pass:
//   1. `[server.web] terminal` — off unless an operator turned it on.
//   2. `terminal_requires_loopback` — on by default, so enabling the
//      terminal for local convenience does not silently also expose it to
//      whatever hostname the server answers to.
//   3. The session cookie, checked at upgrade like any other route.

use axum::extract::ws::{Message, WebSocket, WebSocketUpgrade};

#[derive(Debug, Deserialize)]
pub(crate) struct PtyQuery {
    #[serde(default)]
    pub cwd: Option<String>,
}

/// Whether this request may open a shell, and why not when it may not.
fn terminal_refusal(state: &AppState, headers: &header::HeaderMap) -> Option<&'static str> {
    let cfg = state.core.config();
    if !cfg.server.web_terminal {
        return Some("the terminal is disabled; set [server.web] terminal = true to enable it");
    }
    if cfg.server.web_terminal_requires_loopback {
        let host = headers.get(header::HOST).and_then(|v| v.to_str().ok());
        if !crate::host_is_loopback(host) {
            return Some(
                "the terminal is enabled but restricted to loopback; \
                 set [server.web] terminal_requires_loopback = false to allow remote shells",
            );
        }
    }
    None
}

pub(crate) async fn pty_socket(
    State(state): State<AppState>,
    headers: header::HeaderMap,
    axum::extract::Query(query): axum::extract::Query<PtyQuery>,
    upgrade: WebSocketUpgrade,
) -> Response {
    if let Some(reason) = terminal_refusal(&state, &headers) {
        return (
            StatusCode::FORBIDDEN,
            Json(serde_json::json!({ "error": reason })),
        )
            .into_response();
    }
    // A shell inherits the workspace, so it is bounded by the same roots
    // the picker is — a `cwd` query parameter must not be a way to start a
    // shell somewhere the operator never authorized.
    let cwd = match query
        .cwd
        .as_deref()
        .map(str::trim)
        .filter(|c| !c.is_empty())
    {
        Some(path) => {
            let path = PathBuf::from(path);
            match path.canonicalize() {
                Ok(path) if path.is_dir() && within_roots(&state, &path) => path,
                _ => state.active_core().cwd().clone(),
            }
        }
        None => state.active_core().cwd().clone(),
    };
    upgrade.on_upgrade(move |socket| drive_pty(socket, cwd))
}

/// Control frames the client sends as text; keystrokes are binary. Nothing
/// a user can type is mistakable for a control message.
#[derive(Debug, Deserialize)]
struct PtyControl {
    resize: Option<PtyResize>,
}

#[derive(Debug, Deserialize)]
struct PtyResize {
    cols: u16,
    rows: u16,
}

async fn drive_pty(socket: WebSocket, cwd: PathBuf) {
    use futures::{SinkExt, StreamExt};
    use portable_pty::{CommandBuilder, PtySize, native_pty_system};

    let pair = match native_pty_system().openpty(PtySize {
        rows: 24,
        cols: 80,
        pixel_width: 0,
        pixel_height: 0,
    }) {
        Ok(pair) => pair,
        Err(error) => {
            let mut socket = socket;
            let _ = socket
                .send(Message::Text(format!("openpty failed: {error}").into()))
                .await;
            return;
        }
    };

    let mut cmd = CommandBuilder::new_default_prog();
    cmd.cwd(&cwd);
    cmd.env("TERM", "xterm-256color");
    let mut child = match pair.slave.spawn_command(cmd) {
        Ok(child) => child,
        Err(error) => {
            let mut socket = socket;
            let _ = socket
                .send(Message::Text(format!("shell spawn failed: {error}").into()))
                .await;
            return;
        }
    };
    drop(pair.slave);

    let killer = child.clone_killer();
    let Ok(mut reader) = pair.master.try_clone_reader() else {
        let _ = child.kill();
        return;
    };
    let Ok(mut writer) = pair.master.take_writer() else {
        let _ = child.kill();
        return;
    };
    let master = pair.master;

    let (mut sink, mut stream) = socket.split();

    // Shell -> socket. A blocking read on its own thread, handed across by
    // a channel, because `portable_pty`'s reader is not async.
    let (out_tx, mut out_rx) = tokio::sync::mpsc::channel::<Vec<u8>>(64);
    std::thread::spawn(move || {
        let mut buf = [0u8; 8192];
        loop {
            match std::io::Read::read(&mut reader, &mut buf) {
                Ok(0) | Err(_) => break,
                Ok(n) => {
                    if out_tx.blocking_send(buf[..n].to_vec()).is_err() {
                        break;
                    }
                }
            }
        }
    });
    let pump = tokio::spawn(async move {
        while let Some(bytes) = out_rx.recv().await {
            if sink.send(Message::Binary(bytes.into())).await.is_err() {
                break;
            }
        }
        let _ = sink.close().await;
    });

    // Socket -> shell, until the client goes away.
    while let Some(Ok(message)) = stream.next().await {
        match message {
            Message::Binary(bytes) => {
                if std::io::Write::write_all(&mut writer, &bytes).is_err() {
                    break;
                }
            }
            Message::Text(text) => {
                if let Ok(PtyControl { resize: Some(size) }) =
                    serde_json::from_str::<PtyControl>(&text)
                {
                    let _ = master.resize(PtySize {
                        rows: size.rows,
                        cols: size.cols,
                        pixel_width: 0,
                        pixel_height: 0,
                    });
                }
            }
            Message::Close(_) => break,
            _ => {}
        }
    }

    // The socket IS the shell's lifetime. Closing the tab kills the
    // process rather than leaving it running for the life of the server —
    // the exact leak the desktop's own PTY had until `pty_close` landed.
    let mut killer = killer;
    let _ = killer.kill();
    let _ = child.wait();
    pump.abort();
}
