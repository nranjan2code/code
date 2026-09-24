//! Session event streams (docs/design/48-web-client.md §4.4, §4.7).
//!
//! Each subscription a client can hold — a session's agent events, its
//! presentation frames, its side-chat branch, its coworking signal, the host
//! and config changes — is built here exactly once. The single-session
//! routes (`/sessions/{id}/events`, `/sessions/{id}/presentation/events`)
//! serve one of them each for clients that follow one session, such as
//! `vak term`. `GET /stream` merges any set of them onto ONE connection.
//!
//! The merge exists because a browser allows six HTTP/1.1 connections per
//! host across all of its tabs, and HTTP/2 is not available on plain-http
//! loopback. A tab that held one EventSource per subscription exhausted the
//! pool with two or three tabs open, and every ordinary `fetch` then queued
//! behind streams that never end.

use std::collections::BTreeMap;
use std::pin::Pin;
use std::sync::Arc;

use axum::extract::State;
use axum::http::StatusCode;
use axum::response::sse::{Event, KeepAlive, Sse};
use axum::response::{IntoResponse, Response};
use futures::stream::{self, Stream, StreamExt};
use tokio_stream::wrappers::{BroadcastStream, IntervalStream};
use vak_agent::AgentEvent;

use crate::events::SeqEvent;
use crate::{AppState, SessionHandle};

type Frames<T> = Pin<Box<dyn Stream<Item = T> + Send>>;

/// Most sessions one `/stream` connection may follow. A client follows the
/// sessions on screen plus any best-of-N candidates; anything past this is a
/// client bug, refused rather than turned into unbounded server work.
pub(crate) const MAX_STREAM_SESSIONS: usize = 32;

/// One item of a session's agent-event subscription.
pub(crate) enum AgentFrame {
    /// The resume point is older than the replay ring, or this consumer
    /// lagged: what the client holds may be missing events it cannot see.
    Resync(&'static str),
    Event(Box<SeqEvent>),
}

/// A session's agent events, resuming after `resume` when given.
///
/// Subscribes BEFORE reading the replay ring, so an event published between
/// the two is received live rather than falling into the gap between them.
/// Duplicates are filtered by sequence number; a gap could not be recovered.
pub(crate) fn agent_frames(handle: &SessionHandle, resume: Option<u64>) -> Frames<AgentFrame> {
    let rx = handle.events_tx.subscribe();

    // `None` from the ring means it no longer reaches back that far, and the
    // client is told to rebuild from the durable transcript rather than being
    // handed a stream with a hole in it that it cannot see.
    let (replay, resync) = match resume {
        Some(seq) => match handle.events_tx.replay_after(seq) {
            Some(missed) => (missed, false),
            None => (Vec::new(), true),
        },
        None => (Vec::new(), false),
    };
    let highest_replayed = replay.last().map(|e| e.seq).unwrap_or(0);

    handle.subscribed.notify_one();
    handle.events_tx.send(AgentEvent::StreamOpened);

    let resync = resync.then_some(AgentFrame::Resync("events older than the replay window"));
    let live = BroadcastStream::new(rx).filter_map(move |event| {
        std::future::ready(match event {
            // At or below what the replay already delivered is a duplicate.
            Ok(framed) if framed.seq <= highest_replayed => None,
            Ok(framed) => Some(AgentFrame::Event(Box::new(framed))),
            Err(_) => Some(AgentFrame::Resync("live event consumer lagged")),
        })
    });
    Box::pin(
        stream::iter(resync)
            .chain(stream::iter(
                replay
                    .into_iter()
                    .map(|framed| AgentFrame::Event(Box::new(framed))),
            ))
            .chain(live),
    )
}

/// One serialized `OutputStreamFrame`, with the sequence it brings the
/// client up to when it has one.
pub(crate) struct PresentationFrame {
    pub(crate) sequence: Option<u64>,
    pub(crate) json: String,
}

fn presentation_frame(frame: &vak_delivery::OutputStreamFrame) -> PresentationFrame {
    PresentationFrame {
        sequence: frame.sequence,
        json: serde_json::to_string(frame).unwrap_or_else(|error| {
            serde_json::json!({
                "error": "presentation serialization failed",
                "detail": error.to_string(),
            })
            .to_string()
        }),
    }
}

/// A session's presentation frames: an authoritative snapshot first, then
/// one frame per projected live event. A session with no live handle gets
/// its historical snapshot and nothing after it; `None` is an unknown id.
///
/// Snapshot-based on purpose: a reconnect receives a fresh snapshot, whose
/// sequence becomes the new cursor. Delta replay is never manufactured from
/// an unknown historical baseline.
pub(crate) fn presentation_frames(
    state: &AppState,
    id: &str,
    handle: Option<Arc<SessionHandle>>,
) -> Option<Frames<PresentationFrame>> {
    let Some(handle) = handle else {
        let session = crate::open_historical_session(state, id)?;
        let frame = vak_delivery::OutputStreamFrame {
            sequence: None,
            delta: None,
            snapshot: crate::projection::snapshot(id, &session),
        };
        return Some(Box::pin(stream::once(std::future::ready(
            presentation_frame(&frame),
        ))));
    };
    let id = id.to_owned();
    let rx = handle.events_tx.subscribe();
    let mut timeline = {
        let guard = handle
            .session
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        guard
            .as_ref()
            .map(|session| crate::live_presentation_snapshot(&handle.core, &id, session))
            .unwrap_or_else(|| {
                handle
                    .presentation
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner)
                    .clone()
            })
    };
    let mut last_sequence = live_cursor(&timeline).unwrap_or(0);
    let initial = presentation_frame(&vak_delivery::OutputStreamFrame {
        sequence: Some(last_sequence),
        delta: None,
        snapshot: timeline.clone(),
    });
    handle.subscribed.notify_one();
    let live = BroadcastStream::new(rx).filter_map(move |event| {
        let frame = match event {
            Ok(framed) => {
                if framed.seq <= last_sequence {
                    None
                } else {
                    last_sequence = framed.seq;
                    if matches!(&framed.event, AgentEvent::RunFinished { .. }) {
                        // The run owner publishes the rebuilt durable
                        // projection before broadcasting RunFinished. Rebase
                        // this long-lived subscriber now: carrying its private
                        // live timeline into the next run would otherwise
                        // replace settled cards with the prior run's
                        // prose/progress snapshot.
                        timeline = handle
                            .presentation
                            .lock()
                            .unwrap_or_else(std::sync::PoisonError::into_inner)
                            .clone();
                        Some(crate::projection::settled_frame(last_sequence, &timeline))
                    } else {
                        crate::projection::project_frame(&mut timeline, framed)
                    }
                }
            }
            Err(_) => {
                if let Some(events) = handle.events_tx.replay_after(last_sequence) {
                    for framed in events {
                        last_sequence = framed.seq;
                        crate::projection::project_frame(&mut timeline, framed);
                    }
                } else {
                    let guard = handle
                        .session
                        .lock()
                        .unwrap_or_else(std::sync::PoisonError::into_inner);
                    timeline = guard
                        .as_ref()
                        .map(|session| crate::projection::snapshot(&id, session))
                        .unwrap_or_else(|| {
                            handle
                                .presentation
                                .lock()
                                .unwrap_or_else(std::sync::PoisonError::into_inner)
                                .clone()
                        });
                    last_sequence = live_cursor(&timeline).unwrap_or(last_sequence);
                    timeline
                        .diagnostics
                        .push("Presentation stream resynchronized after a gap.".into());
                }
                Some(vak_delivery::OutputStreamFrame {
                    sequence: Some(last_sequence),
                    delta: None,
                    snapshot: timeline.clone(),
                })
            }
        };
        std::future::ready(frame.map(|frame| presentation_frame(&frame)))
    });
    Some(Box::pin(
        stream::once(std::future::ready(initial)).chain(live),
    ))
}

fn live_cursor(timeline: &vak_delivery::OutputTimeline) -> Option<u64> {
    timeline
        .cursor
        .as_deref()
        .and_then(|cursor| cursor.strip_prefix("live:"))
        .and_then(|seq| seq.parse().ok())
}

/// A session's side-chat (`/btw`) events. `None` marks a lagged consumer.
fn side_frames(handle: &SessionHandle) -> Frames<Option<SeqEvent>> {
    let mut rx = handle.side_events_tx.subscribe();
    let _ = rx.try_recv();
    handle.side_events_tx.send(AgentEvent::StreamOpened);
    Box::pin(BroadcastStream::new(rx).map(Result::ok))
}

/// Content-free wakeups for a session's shared candidate comments.
fn coworking_frames(handle: &SessionHandle) -> Frames<()> {
    Box::pin(BroadcastStream::new(handle.coworking_comments_tx.subscribe()).map(|_| ()))
}

/// Host-level facts (active workspace, recents, terminal), sent once on
/// connect and then only when they change. Sampled rather than broadcast:
/// a workspace switch is a human action, and a two-second latency on it is
/// not worth an event family of its own.
fn host_frames(state: AppState) -> Frames<String> {
    let mut last: Option<String> = None;
    let ticks = IntervalStream::new(tokio::time::interval(std::time::Duration::from_secs(2)));
    Box::pin(ticks.filter_map(move |_| {
        let payload = serde_json::to_string(&crate::web::host_payload(&state))
            .unwrap_or_else(|_| "{}".into());
        let changed = last.as_deref() != Some(payload.as_str());
        if changed {
            last = Some(payload.clone());
        }
        std::future::ready(changed.then_some(payload))
    }))
}

/// `ConfigChanged` events from the global hub, so a setting written by
/// another surface is reflected without a restart (docs/design/44
/// "Liveness").
fn config_frames(state: &AppState) -> Frames<String> {
    Box::pin(
        BroadcastStream::new(state.hub.subscribe()).filter_map(|event| {
            std::future::ready(match event {
                Ok(event @ crate::events::SystemEvent::ConfigChanged { .. }) => {
                    serde_json::to_string(&event).ok()
                }
                _ => None,
            })
        }),
    )
}

/// What `/stream` should carry, parsed from its query string.
#[derive(Debug, Default, PartialEq, Eq)]
pub(crate) struct Interest {
    pub(crate) sessions: Vec<String>,
    pub(crate) host: bool,
    pub(crate) config: bool,
    pub(crate) cursor: BTreeMap<String, u64>,
}

impl Interest {
    fn parse(query: Option<&str>) -> Self {
        let mut interest = Self::default();
        for part in query.unwrap_or_default().split('&') {
            let (key, value) = part.split_once('=').unwrap_or((part, ""));
            let value = percent_encoding::percent_decode_str(&value.replace('+', " "))
                .decode_utf8_lossy()
                .into_owned();
            match key {
                "session" if !value.is_empty() && !interest.sessions.contains(&value) => {
                    interest.sessions.push(value);
                }
                "host" => interest.host = value == "1",
                "config" => interest.config = value == "1",
                "cursor" => interest.cursor = parse_cursor(&value),
                _ => {}
            }
        }
        interest
    }
}

/// A `/stream` resume cursor: `<session>:<seq>` pairs joined by `,`.
///
/// It is the SSE event id of every agent frame, so a browser's own reconnect
/// sends the whole vector back as `Last-Event-ID` and every session resumes
/// from its own sequence. A client that reopens the stream itself (because
/// its set of sessions changed) passes the same string as `?cursor=`.
pub(crate) fn parse_cursor(raw: &str) -> BTreeMap<String, u64> {
    raw.split(',')
        .filter_map(|pair| {
            let (session, seq) = pair.trim().rsplit_once(':')?;
            Some((session.to_owned(), seq.parse().ok()?))
        })
        .filter(|(session, _)| !session.is_empty())
        .collect()
}

pub(crate) fn format_cursor(cursor: &BTreeMap<String, u64>) -> String {
    cursor
        .iter()
        .map(|(session, seq)| format!("{session}:{seq}"))
        .collect::<Vec<_>>()
        .join(",")
}

enum Muxed {
    Host(String),
    Config(String),
    Agent(String, AgentFrame),
    Presentation(String, PresentationFrame),
    Side(String, Option<SeqEvent>),
    Coworking(String),
    Unknown(String),
}

fn session_data(session: &str, key: &str, raw_json: &str) -> String {
    format!(
        "{{\"session\":{},\"{key}\":{raw_json}}}",
        serde_json::Value::from(session)
    )
}

fn event_json(event: &AgentEvent) -> String {
    serde_json::to_string(event).unwrap_or_else(|error| {
        serde_json::json!({
            "error": "event serialization failed",
            "detail": error.to_string(),
        })
        .to_string()
    })
}

/// `GET /stream` — every subscription a client holds, on one connection.
///
/// Query: `session=<id>` (repeatable), `host=1`, `config=1`, and optionally
/// `cursor=` (see [`parse_cursor`]); a `Last-Event-ID` header wins over the
/// query cursor. Every frame is a named SSE event whose JSON names the
/// session it belongs to: `agent` (with the cursor as its id), `presentation`,
/// `side`, `coworking`, `resync`, `unknown`, `host`, `config`.
///
/// Operator-only by construction: the participant middleware admits only
/// `/sessions/<its conversation>/…` paths.
pub(crate) async fn stream(
    State(state): State<AppState>,
    headers: axum::http::HeaderMap,
    uri: axum::http::Uri,
) -> Response {
    let mut interest = Interest::parse(uri.query());
    if let Some(header) = headers
        .get("last-event-id")
        .and_then(|value| value.to_str().ok())
        .filter(|value| !value.trim().is_empty())
    {
        interest.cursor = parse_cursor(header);
    }
    if interest.sessions.len() > MAX_STREAM_SESSIONS {
        return (
            StatusCode::BAD_REQUEST,
            format!("a stream follows at most {MAX_STREAM_SESSIONS} sessions"),
        )
            .into_response();
    }
    if interest.sessions.is_empty() && !interest.host && !interest.config {
        return (
            StatusCode::BAD_REQUEST,
            "a stream needs at least one subscription",
        )
            .into_response();
    }

    let mut parts: Vec<Frames<Muxed>> = Vec::new();
    if interest.host {
        parts.push(Box::pin(host_frames(state.clone()).map(Muxed::Host)));
    }
    if interest.config {
        parts.push(Box::pin(config_frames(&state).map(Muxed::Config)));
    }
    // Only sessions this connection follows keep a cursor entry; a session
    // dropped from the set must not be resumed from a stale position later.
    let mut cursor = BTreeMap::new();
    for id in &interest.sessions {
        let handle = crate::ensure_session_handle(&state, id)
            .await
            .ok()
            .map(|(_, handle)| handle);
        let presentation = presentation_frames(&state, id, handle.clone());
        let Some(handle) = handle else {
            match presentation {
                Some(frames) => {
                    let session = id.clone();
                    parts.push(Box::pin(
                        frames.map(move |frame| Muxed::Presentation(session.clone(), frame)),
                    ));
                }
                None => parts.push(Box::pin(stream::once(std::future::ready(Muxed::Unknown(
                    id.clone(),
                ))))),
            }
            continue;
        };
        let resume = interest.cursor.get(id).copied();
        if let Some(seq) = resume {
            cursor.insert(id.clone(), seq);
        }
        let session = id.clone();
        parts.push(Box::pin(
            agent_frames(&handle, resume).map(move |frame| Muxed::Agent(session.clone(), frame)),
        ));
        if let Some(frames) = presentation {
            let session = id.clone();
            parts.push(Box::pin(
                frames.map(move |frame| Muxed::Presentation(session.clone(), frame)),
            ));
        }
        let session = id.clone();
        parts.push(Box::pin(
            side_frames(&handle).map(move |event| Muxed::Side(session.clone(), event)),
        ));
        let session = id.clone();
        parts.push(Box::pin(
            coworking_frames(&handle).map(move |()| Muxed::Coworking(session.clone())),
        ));
    }

    let frames = stream::select_all(parts).map(move |item| {
        let event = match item {
            Muxed::Host(payload) => Event::default().event("host").data(payload),
            Muxed::Config(payload) => Event::default().event("config").data(payload),
            Muxed::Agent(session, AgentFrame::Event(framed)) => {
                cursor.insert(session.clone(), framed.seq);
                Event::default()
                    .event("agent")
                    .id(format_cursor(&cursor))
                    .data(session_data(&session, "event", &event_json(&framed.event)))
            }
            Muxed::Agent(session, AgentFrame::Resync(reason)) => {
                Event::default().event("resync").data(session_data(
                    &session,
                    "reason",
                    &serde_json::Value::from(reason).to_string(),
                ))
            }
            Muxed::Presentation(session, frame) => Event::default()
                .event("presentation")
                .data(session_data(&session, "frame", &frame.json)),
            Muxed::Side(session, Some(framed)) => Event::default()
                .event("side")
                .data(session_data(&session, "event", &event_json(&framed.event))),
            Muxed::Side(session, None) => Event::default()
                .event("side")
                .data(session_data(&session, "lagged", "true")),
            Muxed::Coworking(session) => Event::default()
                .event("coworking")
                .data(session_data(&session, "refresh", "true")),
            Muxed::Unknown(session) => Event::default().event("unknown").data(session_data(
                &session,
                "error",
                "\"unknown session\"",
            )),
        };
        Ok::<_, std::convert::Infallible>(event)
    });
    Sse::new(frames)
        .keep_alive(KeepAlive::default())
        .into_response()
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    #[test]
    fn a_cursor_round_trips_through_its_event_id_form() {
        let mut cursor = BTreeMap::new();
        cursor.insert("019a-b".to_owned(), 17);
        cursor.insert("019a-c".to_owned(), 4);
        let raw = format_cursor(&cursor);
        assert_eq!(raw, "019a-b:17,019a-c:4");
        assert_eq!(parse_cursor(&raw), cursor);
    }

    #[test]
    fn a_malformed_cursor_pair_is_skipped_not_fatal() {
        let parsed = parse_cursor("a:1,nonsense,:3,b:x,c:9");
        assert_eq!(parsed.len(), 2);
        assert_eq!(parsed.get("a"), Some(&1));
        assert_eq!(parsed.get("c"), Some(&9));
    }

    #[test]
    fn interest_reads_repeated_sessions_flags_and_an_encoded_cursor() {
        let interest = Interest::parse(Some(
            "session=a&session=b&session=a&host=1&cursor=a%3A5%2Cb%3A6",
        ));
        assert_eq!(interest.sessions, vec!["a", "b"]);
        assert!(interest.host);
        assert!(!interest.config);
        assert_eq!(interest.cursor.get("a"), Some(&5));
        assert_eq!(interest.cursor.get("b"), Some(&6));
    }
}
