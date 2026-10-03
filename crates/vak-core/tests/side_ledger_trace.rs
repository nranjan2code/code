//! Side ledgers written inside a run carry that run's key and its actor
//! (docs/design/73 §4); a write with no run in scope carries neither.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use vak_commit::{CommitmentLedger, Economics, Event, EventKind, spec_from_reading};
use vak_core::{inbox, misread, routing};
use vak_session::ids::{AgentId, PrincipalId, SpaceId, TenantId};
use vak_session::trace::{Cause, TraceKey};

fn key() -> (TraceKey, PrincipalId) {
    let actor = PrincipalId::new();
    let key = TraceKey::root(
        TenantId::new(),
        SpaceId::new(),
        AgentId::new(),
        Cause::Heartbeat,
    )
    .acting(actor, None);
    (key, actor)
}

fn reading() -> vak_intent::Reading {
    let request = vak_intent::Request {
        text: "check the build",
        ..vak_intent::Request::default()
    };
    vak_intent::resolve(
        &request,
        &vak_intent::Declared::default(),
        &vak_intent::Authority::default(),
        &vak_intent::ResolverConfig::default(),
    )
    .intent()
    .reading
}

#[test]
fn inbox_entries_carry_the_run_key_when_one_is_in_scope() {
    let dir = tempfile::tempdir().unwrap();
    let (key, actor) = key();
    let traced = inbox::record(
        &vak_config::scope::AgentScope::new(dir.path()),
        inbox::Kind::Heartbeat,
        "t",
        "b",
        None,
        None,
        Some(&key),
    )
    .unwrap();
    assert_eq!(traced.trace.as_ref().map(|t| t.run), Some(key.run));
    assert_eq!(traced.actor, Some(actor));
    let bare = inbox::record(
        &vak_config::scope::AgentScope::new(dir.path()),
        inbox::Kind::Heartbeat,
        "t2",
        "b",
        None,
        None,
        None,
    )
    .unwrap();
    assert!(bare.trace.is_none() && bare.actor.is_none());
}

#[test]
fn misread_and_routing_rows_carry_the_run_key() {
    let dir = tempfile::tempdir().unwrap();
    let (key, actor) = key();
    let ledger = misread::MisreadLedger::new(dir.path());
    ledger.record(
        &reading(),
        vak_intent::Tier::Signals,
        vak_intent::RESOLVER_VERSION,
        misread::Outcome::Held,
        None,
        false,
        Some(&key),
    );
    let rows = ledger.rows();
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].trace.as_ref().map(|t| t.run), Some(key.run));
    assert_eq!(rows[0].actor, Some(actor));

    let evidence = routing::EvidenceLedger::new(dir.path());
    let mut receipt = vak_llm::WorkReceipt::new(vak_llm::WorkPurpose::Execute, "p", "m");
    receipt.record(
        vak_llm::AttemptReason::Initial,
        vak_llm::FailureDomain::Unknown,
        vak_llm::Settlement::Ok,
        10,
        None,
        None,
    );
    evidence.record_receipts(std::slice::from_ref(&receipt), Some(&key));
    let rows: Vec<routing::EvidenceRow> =
        vak_session::chain::RecordChain::at(dir.path().join("routing-evidence")).read();
    let row = rows.into_iter().next().unwrap();
    assert_eq!(row.trace.as_ref().map(|t| t.run), Some(key.run));
    assert_eq!(row.actor, Some(actor));
}

#[test]
fn commitment_events_are_stamped_with_the_ledgers_run_key() {
    let dir = tempfile::tempdir().unwrap();
    let (key, actor) = key();
    let spec = spec_from_reading(
        "check the build",
        reading(),
        Vec::new(),
        dir.path().to_path_buf(),
        Economics::default(),
    );
    let ledger = CommitmentLedger::new(dir.path()).with_trace(Some(&key));
    let id = ledger.open_commitment(spec).unwrap();
    ledger
        .append(&Event::new(
            &id,
            EventKind::EpisodeStarted {
                episode_id: "e1".into(),
                session_id: "s1".into(),
                strand_id: None,
            },
        ))
        .unwrap();
    let events = ledger.events();
    assert!(events.len() >= 2);
    for event in &events {
        assert_eq!(event.trace.as_ref().map(|t| t.run), Some(key.run));
        assert_eq!(event.actor, Some(actor));
    }
    let plain = CommitmentLedger::new(dir.path());
    plain
        .append(&Event::new(
            &id,
            EventKind::EpisodeStarted {
                episode_id: "e2".into(),
                session_id: "s1".into(),
                strand_id: None,
            },
        ))
        .unwrap();
    assert!(plain.events().last().unwrap().trace.is_none());
}

/// A gate forwarded to a chat names the call it gates in the inbox
/// (docs/design/85-turn-graph.md, G0).
#[test]
fn a_forwarded_approval_names_its_call() {
    let dir = tempfile::tempdir().unwrap();
    let (key, _) = key();
    let entry = inbox::record_for_call(
        &vak_config::scope::AgentScope::new(dir.path()),
        inbox::Kind::ApprovalPending,
        "Approval requested",
        "Tool: write",
        Some("s1"),
        Some("call-9"),
        Some(&key),
    )
    .unwrap();
    assert_eq!(entry.tool_use_id.as_deref(), Some("call-9"));
    let listed = inbox::list(&vak_config::scope::AgentScope::new(dir.path()), 10);
    assert_eq!(listed[0].tool_use_id.as_deref(), Some("call-9"));
}
