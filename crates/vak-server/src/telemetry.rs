//! The telemetry readers (data-architecture plan M5b): the services that
//! write a log, their content-free lines, and one run's spans for its
//! waterfall. Every line is already content-free when it is written
//! (`vak_telemetry`), so these only read and filter.

use axum::Json;
use axum::extract::{Path, Query};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use serde::Deserialize;

/// The most lines one request returns.
const MAX_LINES: usize = 1000;
/// The most spans one run's waterfall reads.
const MAX_SPANS: usize = 2000;

fn services() -> Vec<String> {
    vak_telemetry::services(&vak_config::paths::logs_dir())
}

/// `GET /telemetry/services`: the services that write a log here.
pub(crate) async fn list_services() -> Json<serde_json::Value> {
    Json(serde_json::json!({ "services": services() }))
}

#[derive(Debug, Default, Deserialize)]
pub(crate) struct LogsQuery {
    /// One service's log; every service's when absent.
    service: Option<String>,
    /// The least severe level shown (`error`, `warn`, `info`, `debug`,
    /// `trace`); `info` when absent.
    level: Option<String>,
    /// Only lines of this run.
    trace: Option<String>,
    /// Whether span closes are shown beside events.
    #[serde(default)]
    spans: bool,
    limit: Option<usize>,
}

/// `GET /telemetry/logs`: the newest lines, newest first.
pub(crate) async fn logs(Query(query): Query<LogsQuery>) -> Response {
    let known = services();
    let selected = match query.service {
        Some(service) if known.contains(&service) => vec![service],
        Some(_) => {
            return (
                StatusCode::NOT_FOUND,
                Json(serde_json::json!({ "error": "no log for that service" })),
            )
                .into_response();
        }
        None => known,
    };
    let floor = vak_telemetry::severity(query.level.as_deref().unwrap_or("info"));
    let limit = query.limit.unwrap_or(200).clamp(1, MAX_LINES);
    let trace = query.trace.filter(|trace| !trace.is_empty());
    let lines = tokio::task::spawn_blocking(move || {
        vak_telemetry::read_services(&selected, limit, |line| {
            let span = line["event"] == "span.close";
            (query.spans || !span)
                && (span
                    || vak_telemetry::severity(line["level"].as_str().unwrap_or_default()) <= floor)
                && trace
                    .as_deref()
                    .is_none_or(|trace| line["trace_id"].as_str() == Some(trace))
        })
    })
    .await
    .unwrap_or_default();
    Json(serde_json::json!({ "lines": lines })).into_response()
}

/// `GET /runs/{id}/spans`: the spans of one run, in the order they started,
/// each with its start (`started_at`, its close time less its duration).
pub(crate) async fn run_spans(Path(id): Path<String>) -> Response {
    if vak_session::ids::RunId::parse(&id).is_err() {
        return (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({ "error": "not a run id" })),
        )
            .into_response();
    }
    let known = services();
    let mut spans = tokio::task::spawn_blocking(move || {
        vak_telemetry::read_services(&known, MAX_SPANS, |line| {
            line["event"] == "span.close" && line["trace_id"].as_str() == Some(id.as_str())
        })
    })
    .await
    .unwrap_or_default();
    for span in &mut spans {
        let closed = span["ts"]
            .as_str()
            .and_then(|ts| chrono::DateTime::parse_from_rfc3339(ts).ok());
        let took = span["duration_ms"].as_i64().unwrap_or_default();
        if let (Some(closed), Some(line)) = (closed, span.as_object_mut()) {
            let started = closed - chrono::Duration::milliseconds(took);
            line.insert(
                "started_at".into(),
                started
                    .to_rfc3339_opts(chrono::SecondsFormat::Millis, true)
                    .into(),
            );
        }
    }
    spans.sort_by(|a, b| {
        let at =
            |line: &serde_json::Value| line["started_at"].as_str().unwrap_or_default().to_string();
        at(a).cmp(&at(b))
    });
    Json(serde_json::json!({ "spans": spans })).into_response()
}
