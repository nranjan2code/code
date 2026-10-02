#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use serde::Serialize;
use serde::de::DeserializeOwned;
use vak_session::ids::PrincipalId;
use vak_session::trace::Traced;

fn name<T: Traced>() -> &'static str {
    T::ROW_TYPE
}

/// Every durable ledger row type carries the optional trace key and actor
/// (docs/design/73 §4). Adding a ledger row type means adding it here.
#[test]
fn every_ledger_row_type_is_traced() {
    let mut names = vec![
        name::<vak_core::finops::CostRow>(),
        name::<vak_core::finops::ActivityRow>(),
        name::<vak_core::finops::BudgetAlertRow>(),
        name::<vak_core::routing::EvidenceRow>(),
        name::<vak_core::misread::MisreadRow>(),
        name::<vak_core::security_events::SecurityEvent>(),
        name::<vak_core::inbox::Entry>(),
        name::<vak_commit::Event>(),
        name::<crate::operations::IncidentRecord>(),
        name::<crate::operations::ActionReceipt>(),
        name::<crate::coworking::AudienceGrant>(),
        name::<vak_delivery::outbox::OutboxRecord>(),
        name::<vak_sandbox::EnvironmentRecord>(),
        name::<vak_sandbox::PreviewPreparationRecord>(),
        name::<vak_sandbox::PromotionRecord>(),
        name::<vak_sandbox::CandidateRecord>(),
    ];
    names.sort_unstable();
    let total = names.len();
    names.dedup();
    assert_eq!(total, names.len(), "row type names are unique");
    assert_eq!(total, 16);
}

/// A row written under an actor reads back naming that actor, and writes it
/// out again unchanged: the field is part of the row's schema, not ambient.
fn names_actor<T: Traced + Serialize + DeserializeOwned>(mut sample: serde_json::Value) {
    assert!(T::ROW_TYPE.len() > 2);
    let actor = PrincipalId::new();
    sample["actor"] = serde_json::json!(actor.to_string());
    let row: T =
        serde_json::from_value(sample).unwrap_or_else(|error| panic!("{}: {error}", T::ROW_TYPE));
    assert_eq!(row.actor(), Some(&actor), "{}", T::ROW_TYPE);
    let again = serde_json::to_value(&row).unwrap();
    assert_eq!(again["actor"], serde_json::json!(actor.to_string()));
}

#[test]
fn every_ledger_row_type_names_its_actor() {
    let ts = "2026-10-02T00:00:00Z";
    names_actor::<vak_core::finops::CostRow>(serde_json::json!({
        "ts": ts, "model": "m", "input_tokens": 1, "output_tokens": 1,
        "source": "estimated", "session_id": "s",
    }));
    names_actor::<vak_core::finops::ActivityRow>(serde_json::json!({
        "ts": ts, "kind": "tool", "name": "n", "success": true,
    }));
    names_actor::<vak_core::finops::BudgetAlertRow>(serde_json::json!({
        "kind": "budget_alert", "ts": ts, "level": "eighty", "day_total_usd": 1.0,
        "session_id": "s",
    }));
    names_actor::<vak_core::routing::EvidenceRow>(serde_json::json!({
        "ts": ts, "provider": "p", "model": "m", "outcome": "success", "latency_ms": 1,
    }));
    names_actor::<vak_core::misread::MisreadRow>(serde_json::json!({
        "ts": ts, "act": "a", "stakes": "s", "tier": "t", "resolver_version": 1,
        "outcome": "o",
    }));
    names_actor::<vak_core::security_events::SecurityEvent>(serde_json::json!({
        "ts": ts, "kind": "rate_limit", "label": "l", "detail": "d",
    }));
    names_actor::<vak_core::inbox::Entry>(serde_json::json!({
        "id": "i", "ts": ts, "kind": "heartbeat", "title": "t", "body": "b",
    }));
    names_actor::<vak_commit::Event>(serde_json::json!({
        "event_id": "e", "commitment_id": "c", "ts": ts, "kind": "episode-started",
        "episode_id": "x", "session_id": "s",
    }));
    names_actor::<crate::operations::IncidentRecord>(serde_json::json!({
        "id": "i", "fingerprint": "f", "severity": "s", "status": "open", "source": "x",
        "title": "t", "detail": "d", "first_seen": ts, "last_seen": ts, "occurrences": 1,
        "workspace": null, "evidence": [], "resolution": null,
    }));
    names_actor::<crate::operations::ActionReceipt>(serde_json::json!({
        "receipt_id": "r", "service": "s", "action": "a", "requested_at": ts,
        "completed_at": ts, "succeeded": true, "persisted": true,
        "verification": {"status": "s", "before": "b", "after": "a", "detail": "d"},
    }));
    names_actor::<crate::coworking::AudienceGrant>(serde_json::json!({
        "grant_id": "g", "principal_id": "p", "display_name": "d", "conversation_id": "c",
        "audience_id": "a", "capabilities": [], "token_hash": "h", "created_at": ts,
        "expires_at": ts,
    }));
}

#[test]
fn a_row_without_a_trace_omits_the_fields_and_ignores_unknown_ones() {
    let dir = tempfile::tempdir().unwrap();
    let event = vak_core::security_events::record(
        dir.path(),
        vak_core::security_events::EventKind::RateLimit,
        "l",
        "d",
        None,
    );
    assert!(event.trace().is_none() && event.actor().is_none());
    let json = serde_json::to_string(&event).unwrap();
    assert!(!json.contains("trace") && !json.contains("actor"), "{json}");
    let mut value: serde_json::Value = serde_json::from_str(&json).unwrap();
    value["unknown_future_field"] = serde_json::json!(1);
    let back: vak_core::security_events::SecurityEvent = serde_json::from_value(value).unwrap();
    assert_eq!(back, event);
}
