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

use std::path::Path as FsPath;

use axum::{
    Json,
    extract::{Path, State},
    http::StatusCode,
    response::{IntoResponse, Response},
};
use serde::{Deserialize, Serialize};
use vak_session::objects::conversation_scope;

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
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
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
#[derive(Debug, Clone, Deserialize)]
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

/// Announces changes. Durable writes use the Document store's CAS.
#[derive(Clone)]
pub(crate) struct CanvasHub {
    changed: tokio::sync::broadcast::Sender<CanvasChanged>,
}

impl Default for CanvasHub {
    fn default() -> Self {
        Self {
            changed: tokio::sync::broadcast::channel(64).0,
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
    let header = crate::read_historical_header(state, session)?;
    let agent = header
        .agent
        .map(|agent| agent.id)
        .unwrap_or_else(|| vak_core::vak_agent_identity().id);
    Some(shared.agent(&agent).canvas(session))
}

fn decode_canvas(raw: &str) -> Result<ConversationCanvas, Refusal> {
    if raw.len() > MAX_BYTES {
        return Err(Refusal::TooLarge);
    }
    let canvas: ConversationCanvas =
        serde_json::from_str(raw).map_err(|error| Refusal::Unreadable(error.to_string()))?;
    if canvas.schema > SCHEMA {
        return Err(Refusal::Newer);
    }
    validate_canvas(&canvas)?;
    Ok(canvas)
}

fn validate_canvas(canvas: &ConversationCanvas) -> Result<(), Refusal> {
    check(&CanvasWrite {
        revision: canvas.revision,
        entries: canvas.entries.clone(),
        active: canvas.active.clone(),
        other: canvas.other.clone(),
    })
}

fn decode_document(raw: Option<&str>, _session: &str) -> Result<ConversationCanvas, Refusal> {
    match raw {
        Some(raw) => decode_canvas(raw),
        None => Ok(ConversationCanvas::empty()),
    }
}

/// Convert an existing plaintext Canvas once, under its historical lock.
/// Documents are the only normal read/write path; any failed conversion
/// leaves the source file available for recovery.
fn ensure_document(path: &FsPath, session: &str) -> Result<(), Refusal> {
    if !path.exists() {
        return Ok(());
    }
    let metadata = match std::fs::symlink_metadata(path) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(error) => return Err(Refusal::Unreadable(error.to_string())),
    };
    if !metadata.file_type().is_file() {
        return Err(Refusal::Legacy);
    }
    if metadata.len() > MAX_BYTES as u64 {
        return Err(Refusal::TooLarge);
    }
    vak_session::fence::check().map_err(|error| Refusal::Unreadable(error.to_string()))?;
    let lock_path = path.with_extension("lock");
    let lock = std::fs::OpenOptions::new()
        .create(true)
        .truncate(false)
        .read(true)
        .write(true)
        .open(lock_path)
        .map_err(|error| Refusal::Unreadable(error.to_string()))?;
    lock.lock()
        .map_err(|error| Refusal::Unreadable(error.to_string()))?;
    let metadata = match std::fs::symlink_metadata(path) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(error) => return Err(Refusal::Unreadable(error.to_string())),
    };
    if !metadata.file_type().is_file() {
        return Err(Refusal::Legacy);
    }
    if metadata.len() > MAX_BYTES as u64 {
        return Err(Refusal::TooLarge);
    }
    let legacy = match std::fs::read_to_string(path) {
        Ok(legacy) => legacy,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(error) => return Err(Refusal::Unreadable(error.to_string())),
    };
    let legacy_canvas = decode_canvas(&legacy)?;
    let scope = conversation_scope(session);
    let current = vak_session::documents::read_sealed(path, &scope).map_err(Refusal::Unreadable)?;
    if let Some(current) = current {
        let stored = decode_document(Some(&current), session)?;
        if stored != legacy_canvas {
            return Err(Refusal::Legacy);
        }
    } else {
        let document = serde_json::to_string(&legacy_canvas)
            .map_err(|error| Refusal::Unreadable(error.to_string()))?;
        match vak_session::documents::create_sealed(path, &scope, &document) {
            Ok(()) => {}
            Err(error) => {
                let winner = vak_session::documents::read_sealed(path, &scope)
                    .map_err(Refusal::Unreadable)?;
                let stored = decode_document(winner.as_deref(), session)?;
                if stored != legacy_canvas {
                    return Err(Refusal::Unreadable(error));
                }
            }
        }
    }
    let verified =
        vak_session::documents::read_sealed(path, &scope).map_err(Refusal::Unreadable)?;
    if decode_document(verified.as_deref(), session)? != legacy_canvas {
        return Err(Refusal::Legacy);
    }
    vak_session::fence::check().map_err(|error| Refusal::Unreadable(error.to_string()))?;
    std::fs::remove_file(path).map_err(|error| Refusal::Unreadable(error.to_string()))?;
    Ok(())
}

fn read(path: &FsPath, session: &str) -> Result<ConversationCanvas, Refusal> {
    ensure_document(path, session)?;
    let raw = vak_session::documents::read_sealed(path, &conversation_scope(session))
        .map_err(Refusal::Unreadable)?;
    decode_document(raw.as_deref(), session)
}

#[derive(Debug)]
enum Refusal {
    Unreadable(String),
    Newer,
    Invalid(&'static str),
    TooLarge,
    Stale(ConversationCanvas),
    Legacy,
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
            Refusal::Legacy => (StatusCode::UNPROCESSABLE_ENTITY, Json(serde_json::json!({
                "reason": "legacy_canvas",
                "error": "This saved Canvas could not be safely converted. Its original data is still available for recovery."
            }))).into_response(),
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
/// revision. The Document update path supplies cross-process CAS.
fn commit(
    _hub: &CanvasHub,
    path: &FsPath,
    session: &str,
    write: CanvasWrite,
) -> Result<ConversationCanvas, Refusal> {
    check(&write)?;
    ensure_document(path, session)?;
    let mut refusal = None;
    let session_scope = conversation_scope(session);
    let result = vak_session::documents::update_sealed(path, &session_scope, |raw| {
        let current = decode_document(raw, session).map_err(|why| {
            refusal = Some(why);
            "Canvas read refused".to_string()
        })?;
        if current.revision != write.revision {
            refusal = Some(Refusal::Stale(current));
            return Ok(None);
        }
        let mut other = current.other;
        other.extend(write.other.clone());
        let revision = current
            .revision
            .checked_add(1)
            .ok_or_else(|| "Canvas revision overflow".to_string())?;
        let next = ConversationCanvas {
            schema: SCHEMA,
            revision,
            entries: write.entries.clone(),
            active: write.active.clone(),
            other,
        };
        let raw = serde_json::to_string(&next).map_err(|error| error.to_string())?;
        if raw.len() > MAX_BYTES {
            refusal = Some(Refusal::TooLarge);
            return Err("Canvas too large".to_string());
        }
        Ok(Some((raw, next)))
    });
    match result {
        Ok(Some(canvas)) => Ok(canvas),
        Ok(None) => {
            Err(refusal.unwrap_or(Refusal::Unreadable("Canvas update was not saved".into())))
        }
        Err(error) => Err(refusal.unwrap_or(Refusal::Unreadable(error))),
    }
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
    let session = id.clone();
    match tokio::task::spawn_blocking(move || read(&path, &session)).await {
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
    let session = id.clone();
    match tokio::task::spawn_blocking(move || commit(&hub, &path, &session, write)).await {
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

    fn setup() -> (std::path::PathBuf, String) {
        let home = vak_config::paths::isolate_home_for_tests();
        let session = format!("canvas-test-{}", uuid::Uuid::now_v7());
        let path = home
            .join("agents/canvas-test/canvas")
            .join(format!("{session}.json"));
        (path, session)
    }

    fn write(revision: u64, keys: &[&str], active: Option<&str>) -> CanvasWrite {
        serde_json::from_value(serde_json::json!({
            "revision": revision,
            "entries": keys.iter().map(|key| serde_json::json!({
                "key": key, "subject": { "kind": "file", "path": key }, "draft": ""
            })).collect::<Vec<_>>(),
            "active": active,
        }))
        .unwrap()
    }

    fn seed(path: &FsPath, session: &str, canvas: &ConversationCanvas) {
        let value = serde_json::to_value(canvas).unwrap();
        vak_session::documents::create_sealed(
            path,
            &conversation_scope(session),
            &serde_json::to_string(&value).unwrap(),
        )
        .unwrap();
    }

    #[test]
    fn document_cas_preserves_stale_refusal_and_unknown_fields() {
        let (path, session) = setup();
        let hub = CanvasHub::default();
        assert_eq!(read(&path, &session).unwrap().revision, 0);
        let mut initial = ConversationCanvas::empty();
        initial
            .other
            .insert("layout".into(), serde_json::json!("wide"));
        seed(&path, &session, &initial);
        let first = commit(&hub, &path, &session, write(0, &["a"], Some("a"))).unwrap();
        assert_eq!(first.revision, 1);
        assert_eq!(first.other["layout"], "wide");
        match commit(&hub, &path, &session, write(0, &["b"], Some("b"))) {
            Err(Refusal::Stale(current)) => {
                assert_eq!(current.revision, 1);
                assert_eq!(current.entries[0]["key"], "a");
            }
            other => panic!("expected a stale refusal, got {other:?}"),
        }
        // Unknown fields in a stored Document survive subsequent CAS updates.
        let replaced = commit(&hub, &path, &session, write(1, &["a", "b"], Some("b"))).unwrap();
        assert_eq!(replaced.revision, 2);
        assert_eq!(read(&path, &session).unwrap().entries.len(), 2);
        let closed = commit(&hub, &path, &session, write(2, &[], None)).unwrap();
        assert_eq!(closed.revision, 3);
        assert!(matches!(
            commit(&hub, &path, &session, write(1, &["c"], Some("c"))),
            Err(Refusal::Stale(_))
        ));
    }

    #[test]
    fn concurrent_writes_from_the_same_revision_have_one_winner() {
        let (path, session) = setup();
        let path_a = path.clone();
        let path_b = path.clone();
        let session_a = session.clone();
        let session_b = session.clone();
        let hub_a = CanvasHub::default();
        let hub_b = CanvasHub::default();
        let (a, b) = std::thread::scope(|scope| {
            let a =
                scope.spawn(|| commit(&hub_a, &path_a, &session_a, write(0, &["a"], Some("a"))));
            let b =
                scope.spawn(|| commit(&hub_b, &path_b, &session_b, write(0, &["b"], Some("b"))));
            (a.join().unwrap(), b.join().unwrap())
        });
        assert_eq!(usize::from(a.is_ok()) + usize::from(b.is_ok()), 1);
        assert_eq!(
            usize::from(matches!(a, Err(Refusal::Stale(_))))
                + usize::from(matches!(b, Err(Refusal::Stale(_)))),
            1
        );
        assert_eq!(read(&path, &session).unwrap().revision, 1);
    }

    #[test]
    fn document_contains_only_conversation_key_ciphertext() {
        let (path, session) = setup();
        let hub = CanvasHub::default();
        commit(
            &hub,
            &path,
            &session,
            write(0, &["secret-tab"], Some("secret-tab")),
        )
        .unwrap();
        let raw = vak_session::documents::read(&path).unwrap().unwrap();
        assert!(!raw.contains("secret-tab"));
        let envelope: serde_json::Value = serde_json::from_str(&raw).unwrap();
        assert_eq!(envelope["scope"], conversation_scope(&session));
        assert!(envelope["ciphertext"].is_array());
        assert!(vak_session::documents::read_sealed(&path, "other-scope").is_err());
        assert_eq!(
            read(&path, &session).unwrap().entries[0]["key"],
            "secret-tab"
        );
        assert!(matches!(
            read(&path, "other-session"),
            Err(Refusal::Unreadable(_))
        ));
    }

    #[test]
    fn legacy_canvas_converts_once_and_unsafe_legacy_stays_recoverable() {
        let (path, session) = setup();
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        let legacy = r#"{"schema":1,"revision":4,"entries":[],"active":null,"layout":"wide"}"#;
        std::fs::write(&path, legacy).unwrap();
        let converted = read(&path, &session).unwrap();
        assert_eq!(converted.revision, 4);
        assert_eq!(converted.other["layout"], "wide");
        assert!(!path.exists());
        let saved = vak_session::documents::read(&path).unwrap().unwrap();
        assert!(!saved.contains("wide"));
        assert_eq!(
            vak_session::documents::read_sealed(&path, &conversation_scope(&session))
                .unwrap()
                .unwrap(),
            serde_json::to_string(&converted).unwrap()
        );

        let (bad_path, bad_session) = setup();
        std::fs::create_dir_all(bad_path.parent().unwrap()).unwrap();
        let bad = r#"{"schema":2,"revision":4,"entries":[],"active":null}"#;
        std::fs::write(&bad_path, bad).unwrap();
        assert!(matches!(read(&bad_path, &bad_session), Err(Refusal::Newer)));
        assert_eq!(std::fs::read_to_string(&bad_path).unwrap(), bad);
        assert!(
            vak_session::documents::read_sealed(&bad_path, &conversation_scope(&bad_session))
                .unwrap()
                .is_none()
        );

        let (divergent_path, divergent_session) = setup();
        let mut stored_canvas = ConversationCanvas::empty();
        stored_canvas.revision = 1;
        seed(&divergent_path, &divergent_session, &stored_canvas);
        std::fs::create_dir_all(divergent_path.parent().unwrap()).unwrap();
        let divergent = r#"{"schema":1,"revision":4,"entries":[],"active":null}"#;
        std::fs::write(&divergent_path, divergent).unwrap();
        assert!(matches!(
            read(&divergent_path, &divergent_session),
            Err(Refusal::Legacy)
        ));
        assert_eq!(std::fs::read_to_string(&divergent_path).unwrap(), divergent);
        assert!(
            vak_session::documents::read_sealed(
                &divergent_path,
                &conversation_scope(&divergent_session)
            )
            .unwrap()
            .is_some()
        );
    }

    #[test]
    fn invalid_writes_do_not_create_a_document() {
        let (path, session) = setup();
        let hub = CanvasHub::default();
        let too_many: Vec<String> = (0..=MAX_TABS).map(|n| format!("t{n}")).collect();
        let keys: Vec<&str> = too_many.iter().map(String::as_str).collect();
        assert!(matches!(
            commit(&hub, &path, &session, write(0, &keys, Some("t0"))),
            Err(Refusal::Invalid(_))
        ));
        assert!(matches!(
            commit(&hub, &path, &session, write(0, &["a", "a"], Some("a"))),
            Err(Refusal::Invalid(_))
        ));
        assert!(matches!(
            commit(&hub, &path, &session, write(0, &["a"], Some("b"))),
            Err(Refusal::Invalid(_))
        ));
        assert!(matches!(
            commit(&hub, &path, &session, write(0, &["a"], None)),
            Err(Refusal::Invalid(_))
        ));
        let mut huge = write(0, &["a"], Some("a"));
        huge.entries[0]["subject"]["html"] = serde_json::Value::String("x".repeat(MAX_BYTES));
        assert!(matches!(
            commit(&hub, &path, &session, huge),
            Err(Refusal::TooLarge)
        ));
        assert_eq!(read(&path, &session).unwrap().revision, 0);
        assert!(
            vak_session::documents::read_sealed(&path, &conversation_scope(&session))
                .unwrap()
                .is_none()
        );
        assert!(!valid_session("../escape") && !valid_session("") && valid_session("01a0-fbce"));
    }
}
