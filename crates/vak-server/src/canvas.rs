//! The Canvas of each conversation, kept with the Agent that owns the
//! conversation, so every surface showing it (the desktop app and the web app
//! at once, or one after the other) shows the same Canvas (docs/design/66
//! §0a).
//!
//! The server keeps what a client wrote: the tabs, which one is in front, and
//! what the reader did in each (view, selection, unsent note). Nothing here
//! interprets a tab. The rules for opening, closing and changing one live
//! once, in the client (`canvasStack.ts`). A write names the revision it was
//! made from and is refused when another surface wrote first; the client then
//! replays its change on the newer Canvas and writes again. Each write is
//! announced on the conversation's stream as a `canvas` hint carrying the new
//! revision; the Canvas itself is always read from here.

use std::sync::{Arc, Mutex};

use axum::{
    Json,
    extract::{Path, State},
    http::StatusCode,
    response::{IntoResponse, Response},
};
use serde::{Deserialize, Serialize};

use crate::AppState;

/// The schema this version writes. A Canvas written by a newer one is
/// refused rather than half-read (invariant 29).
const SCHEMA: u32 = 1;
/// Tabs a Canvas keeps; the client's strip holds the same (`MAX_ENTRIES`).
const MAX_TABS: usize = 8;
/// A Canvas holds a few tabs and notes; markup from the conversation is the
/// largest thing in one.
const MAX_BYTES: usize = 2 * 1024 * 1024;

/// One conversation's Canvas as it is stored and served.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct ConversationCanvas {
    schema: u32,
    revision: u64,
    entries: Vec<serde_json::Value>,
    active: Option<String>,
    /// Fields a newer client wrote, kept as they are.
    #[serde(flatten)]
    other: serde_json::Map<String, serde_json::Value>,
}

impl ConversationCanvas {
    fn empty() -> Self {
        Self {
            schema: SCHEMA,
            revision: 0,
            entries: Vec::new(),
            active: None,
            other: serde_json::Map::new(),
        }
    }
}

/// A change from a client: the whole Canvas as it should be, and the revision
/// it was made from.
#[derive(Debug, Deserialize)]
pub(crate) struct CanvasWrite {
    revision: u64,
    entries: Vec<serde_json::Value>,
    active: Option<String>,
    #[serde(flatten)]
    other: serde_json::Map<String, serde_json::Value>,
}

/// Announced when a conversation's Canvas changes.
#[derive(Debug, Clone)]
pub(crate) struct CanvasChanged {
    pub(crate) session: String,
    pub(crate) revision: u64,
}

/// Announces changes and serializes this process's writes.
#[derive(Clone)]
pub(crate) struct CanvasHub {
    changed: tokio::sync::broadcast::Sender<CanvasChanged>,
    writes: Arc<Mutex<()>>,
}

impl Default for CanvasHub {
    fn default() -> Self {
        Self {
            changed: tokio::sync::broadcast::channel(64).0,
            writes: Arc::new(Mutex::new(())),
        }
    }
}

impl CanvasHub {
    pub(crate) fn subscribe(&self) -> tokio::sync::broadcast::Receiver<CanvasChanged> {
        self.changed.subscribe()
    }
}

/// A session id as Vak makes them; anything else never names a file.
fn valid_session(id: &str) -> bool {
    !id.is_empty()
        && id.len() <= 128
        && id
            .chars()
            .all(|ch| ch.is_ascii_alphanumeric() || ch == '-' || ch == '_')
}

/// Where a conversation's Canvas is kept: in the home of the Agent that owns
/// it. A conversation that is unknown or in the trash has none.
fn canvas_path(state: &AppState, session: &str) -> Option<std::path::PathBuf> {
    if !valid_session(session) {
        return None;
    }
    let shared = state.core.shared_scope();
    if vak_core::trash::is_trashed(&vak_config::scope::SharedScope::new(shared.root()), session) {
        return None;
    }
    let header = crate::read_historical_header(state, session, None)?;
    let agent = header
        .agent
        .map(|agent| agent.id)
        .unwrap_or_else(|| vak_core::vak_agent_identity().id);
    Some(shared.agent(&agent).canvas(session))
}

fn read(path: &std::path::Path) -> Result<ConversationCanvas, Refusal> {
    let raw = match std::fs::read_to_string(path) {
        Ok(raw) => raw,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            return Ok(ConversationCanvas::empty());
        }
        Err(error) => return Err(Refusal::Unreadable(error.to_string())),
    };
    let canvas: ConversationCanvas =
        serde_json::from_str(&raw).map_err(|error| Refusal::Unreadable(error.to_string()))?;
    if canvas.schema > SCHEMA {
        return Err(Refusal::Newer);
    }
    Ok(canvas)
}

#[derive(Debug)]
enum Refusal {
    Unreadable(String),
    Newer,
    Invalid(&'static str),
    TooLarge,
    Stale(ConversationCanvas),
}

impl IntoResponse for Refusal {
    fn into_response(self) -> Response {
        match self {
            Refusal::Unreadable(error) => (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(serde_json::json!({ "error": format!("The Canvas could not be read: {error}") })),
            )
                .into_response(),
            Refusal::Newer => (
                StatusCode::CONFLICT,
                Json(serde_json::json!({
                    "error": "This Canvas was saved by a newer version of Vakyartha.",
                    "reason": "newer_schema",
                })),
            )
                .into_response(),
            Refusal::Invalid(error) => (
                StatusCode::BAD_REQUEST,
                Json(serde_json::json!({ "error": error })),
            )
                .into_response(),
            Refusal::TooLarge => (
                StatusCode::PAYLOAD_TOO_LARGE,
                Json(serde_json::json!({ "error": "This Canvas is too large to keep." })),
            )
                .into_response(),
            // The Canvas another surface wrote first, for the client to replay
            // its change on.
            Refusal::Stale(current) => (
                StatusCode::CONFLICT,
                Json(serde_json::json!({ "reason": "stale", "canvas": current })),
            )
                .into_response(),
        }
    }
}

fn check(write: &CanvasWrite) -> Result<(), Refusal> {
    if write.entries.len() > MAX_TABS {
        return Err(Refusal::Invalid("A Canvas keeps at most eight tabs."));
    }
    let mut keys = Vec::with_capacity(write.entries.len());
    for entry in &write.entries {
        let key = entry
            .get("key")
            .and_then(serde_json::Value::as_str)
            .filter(|key| !key.is_empty());
        let Some(key) = key else {
            return Err(Refusal::Invalid("Every tab needs a key."));
        };
        if !entry
            .get("subject")
            .is_some_and(serde_json::Value::is_object)
        {
            return Err(Refusal::Invalid("Every tab needs what it shows."));
        }
        if keys.contains(&key) {
            return Err(Refusal::Invalid("Two tabs show the same thing."));
        }
        keys.push(key);
    }
    match write.active.as_deref() {
        None if keys.is_empty() => Ok(()),
        Some(active) if keys.contains(&active) => Ok(()),
        _ => Err(Refusal::Invalid(
            "The tab in front must be one of the tabs.",
        )),
    }
}

/// Writes `write` over the Canvas at `path` if it was made from the current
/// revision. Serialized in this process and locked against another one.
fn commit(
    hub: &CanvasHub,
    path: &std::path::Path,
    write: CanvasWrite,
) -> Result<ConversationCanvas, Refusal> {
    check(&write)?;
    let _guard = hub
        .writes
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let dir = path
        .parent()
        .ok_or(Refusal::Invalid("The Canvas has no place to be kept."))?;
    std::fs::create_dir_all(dir).map_err(|error| Refusal::Unreadable(error.to_string()))?;
    let lock = std::fs::OpenOptions::new()
        .create(true)
        .truncate(false)
        .read(true)
        .write(true)
        .open(path.with_extension("lock"))
        .map_err(|error| Refusal::Unreadable(error.to_string()))?;
    lock.lock()
        .map_err(|error| Refusal::Unreadable(error.to_string()))?;
    let current = read(path)?;
    if current.revision != write.revision {
        return Err(Refusal::Stale(current));
    }
    let mut other = current.other;
    other.extend(write.other);
    let next = ConversationCanvas {
        schema: SCHEMA,
        revision: current.revision + 1,
        entries: write.entries,
        active: write.active,
        other,
    };
    let bytes =
        serde_json::to_vec(&next).map_err(|error| Refusal::Unreadable(error.to_string()))?;
    if bytes.len() > MAX_BYTES {
        return Err(Refusal::TooLarge);
    }
    let temp = path.with_extension(format!("{}.tmp", uuid::Uuid::now_v7()));
    std::fs::write(&temp, &bytes).map_err(|error| Refusal::Unreadable(error.to_string()))?;
    std::fs::rename(&temp, path).map_err(|error| {
        let _ = std::fs::remove_file(&temp);
        Refusal::Unreadable(error.to_string())
    })?;
    Ok(next)
}

fn unknown() -> Response {
    (
        StatusCode::NOT_FOUND,
        Json(serde_json::json!({ "error": "That conversation is not available." })),
    )
        .into_response()
}

/// `GET /sessions/{id}/canvas`: the conversation's Canvas, empty at revision 0
/// when nothing has been opened in it.
pub(crate) async fn get_canvas(State(state): State<AppState>, Path(id): Path<String>) -> Response {
    let Some(path) = canvas_path(&state, &id) else {
        return unknown();
    };
    match tokio::task::spawn_blocking(move || read(&path)).await {
        Ok(Ok(canvas)) => Json(canvas).into_response(),
        Ok(Err(refusal)) => refusal.into_response(),
        Err(error) => Refusal::Unreadable(error.to_string()).into_response(),
    }
}

/// `PUT /sessions/{id}/canvas`: replaces the Canvas when the write was made
/// from its current revision; otherwise `409` with the Canvas as it is now.
pub(crate) async fn put_canvas(
    State(state): State<AppState>,
    Path(id): Path<String>,
    Json(write): Json<CanvasWrite>,
) -> Response {
    let Some(path) = canvas_path(&state, &id) else {
        return unknown();
    };
    let hub = state.canvases.clone();
    match tokio::task::spawn_blocking(move || commit(&hub, &path, write)).await {
        Ok(Ok(canvas)) => {
            let _ = state.canvases.changed.send(CanvasChanged {
                session: id,
                revision: canvas.revision,
            });
            Json(canvas).into_response()
        }
        Ok(Err(refusal)) => refusal.into_response(),
        Err(error) => Refusal::Unreadable(error.to_string()).into_response(),
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    fn write(revision: u64, keys: &[&str], active: Option<&str>) -> CanvasWrite {
        serde_json::from_value(serde_json::json!({
            "revision": revision,
            "entries": keys
                .iter()
                .map(|key| serde_json::json!({ "key": key, "subject": { "kind": "file", "path": key }, "draft": "" }))
                .collect::<Vec<_>>(),
            "active": active,
        }))
        .unwrap()
    }

    /// Two surfaces writing the same Canvas: the one that wrote second from an
    /// old revision is refused and handed the current Canvas to replay on.
    #[test]
    fn a_write_from_an_old_revision_is_refused_with_the_current_canvas() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("canvas/s.json");
        let hub = CanvasHub::default();
        assert_eq!(read(&path).unwrap().revision, 0);
        let first = commit(&hub, &path, write(0, &["a"], Some("a"))).unwrap();
        assert_eq!(first.revision, 1);
        match commit(&hub, &path, write(0, &["b"], Some("b"))) {
            Err(Refusal::Stale(current)) => {
                assert_eq!(current.revision, 1);
                assert_eq!(current.entries[0]["key"], "a");
            }
            other => panic!("expected a stale refusal, got {other:?}"),
        }
        let second = commit(&hub, &path, write(1, &["a", "b"], Some("b"))).unwrap();
        assert_eq!(second.revision, 2);
        assert_eq!(read(&path).unwrap().entries.len(), 2);
        // Closing every tab keeps the revision going, so an old write stays stale.
        let closed = commit(&hub, &path, write(2, &[], None)).unwrap();
        assert_eq!(closed.revision, 3);
        assert!(matches!(
            commit(&hub, &path, write(1, &["c"], Some("c"))),
            Err(Refusal::Stale(_))
        ));
    }

    #[test]
    fn a_canvas_is_checked_before_it_is_kept() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("canvas/s.json");
        let hub = CanvasHub::default();
        let too_many: Vec<String> = (0..=MAX_TABS).map(|n| format!("t{n}")).collect();
        let keys: Vec<&str> = too_many.iter().map(String::as_str).collect();
        assert!(matches!(
            commit(&hub, &path, write(0, &keys, Some("t0"))),
            Err(Refusal::Invalid(_))
        ));
        assert!(matches!(
            commit(&hub, &path, write(0, &["a", "a"], Some("a"))),
            Err(Refusal::Invalid(_))
        ));
        assert!(matches!(
            commit(&hub, &path, write(0, &["a"], Some("b"))),
            Err(Refusal::Invalid(_))
        ));
        assert!(matches!(
            commit(&hub, &path, write(0, &["a"], None)),
            Err(Refusal::Invalid(_))
        ));
        let mut huge = write(0, &["a"], Some("a"));
        huge.entries[0]["subject"]["html"] = serde_json::Value::String("x".repeat(MAX_BYTES));
        assert!(matches!(commit(&hub, &path, huge), Err(Refusal::TooLarge)));
        assert_eq!(read(&path).unwrap().revision, 0, "nothing refused was kept");
        assert!(!valid_session("../escape") && !valid_session("") && valid_session("01a0-fbce"));
    }

    /// A newer version's Canvas is refused, never half-read, and a field this
    /// version does not know survives its writes (invariant 29).
    #[test]
    fn a_newer_canvas_is_refused_and_unknown_fields_survive() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("canvas/s.json");
        let hub = CanvasHub::default();
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(
            &path,
            r#"{"schema":1,"revision":4,"entries":[],"active":null,"layout":"wide"}"#,
        )
        .unwrap();
        let next = commit(&hub, &path, write(4, &["a"], Some("a"))).unwrap();
        assert_eq!(next.other["layout"], "wide");
        std::fs::write(
            &path,
            r#"{"schema":2,"revision":9,"entries":[],"active":null}"#,
        )
        .unwrap();
        assert!(matches!(read(&path), Err(Refusal::Newer)));
        assert!(matches!(
            commit(&hub, &path, write(9, &["a"], Some("a"))),
            Err(Refusal::Newer)
        ));
    }
}
