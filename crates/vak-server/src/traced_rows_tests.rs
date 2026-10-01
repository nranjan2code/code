#![allow(clippy::unwrap_used, clippy::expect_used)]

use vak_session::trace::Traced;

fn name<T: Traced>() -> &'static str {
    T::ROW_TYPE
}

/// Every side-ledger row type carries the optional trace key and actor
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
    ];
    names.sort_unstable();
    let total = names.len();
    names.dedup();
    assert_eq!(total, names.len(), "row type names are unique");
    assert_eq!(total, 12);
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
