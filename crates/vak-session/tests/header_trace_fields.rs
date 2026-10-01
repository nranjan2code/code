#![allow(clippy::unwrap_used, clippy::expect_used)]

//! Invariant 29: the header's trace fields are additive. A header written
//! without them reads back with `None`; unknown fields are ignored; set
//! fields round-trip.

use vak_session::ids::{RunId, SpaceId};
use vak_session::trace::Cause;
use vak_session::types::SessionHeader;

fn base() -> serde_json::Value {
    serde_json::json!({
        "session_id": "s1",
        "created_at": "2026-10-01T00:00:00Z",
        "cwd": "/tmp/x",
        "contract": {
            "app_version": "1", "provider": "p", "model": "m",
            "system_prompt": "s", "permission_mode": "workspace-write"
        }
    })
}

#[test]
fn header_without_trace_fields_reads_as_none() {
    let h: SessionHeader = serde_json::from_value(base()).unwrap();
    assert!(h.space.is_none() && h.run.is_none() && h.cause.is_none());
    let back = serde_json::to_value(&h).unwrap();
    assert!(back.get("space").is_none() && back.get("run").is_none());
}

#[test]
fn trace_fields_round_trip_and_unknown_fields_are_ignored() {
    let mut v = base();
    let (space, run) = (SpaceId::new(), RunId::new());
    v["space"] = serde_json::json!(space.to_string());
    v["run"] = serde_json::json!(run.to_string());
    v["cause"] = serde_json::json!({"kind": "heartbeat"});
    v["future_field"] = serde_json::json!(true);
    let h: SessionHeader = serde_json::from_value(v).unwrap();
    assert_eq!(h.space, Some(space));
    assert_eq!(h.run, Some(run));
    assert_eq!(h.cause, Some(Cause::Heartbeat));
    let again: SessionHeader = serde_json::from_value(serde_json::to_value(&h).unwrap()).unwrap();
    assert_eq!(again.run, Some(run));
}
