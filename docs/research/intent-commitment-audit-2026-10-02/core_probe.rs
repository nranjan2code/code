#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
use vak_commit::{Advancement, CommitmentLedger, Event, EventKind};
use vak_intent::{Authority, Declared, Request, ResolverConfig};

#[test]
fn over_budget_and_suspended_commitments_are_admitted_again() {
    let home = tempfile::tempdir().unwrap();
    let mut config = vak_config::Config::default();
    config.commitment.enabled = true;
    config.commitment.lifetime_budget_usd = Some(1.0);
    let intent = vak_intent::resolve(
        &Request {
            text: "Write a poem every morning",
            turn_id: "audit",
            ..Request::default()
        },
        &Declared::default(),
        &Authority::default(),
        &ResolverConfig::default(),
    )
    .intent();
    let plan =
        vak_core::commitments::plan_episodes(home.path(), &config, &intent, chrono::Utc::now());
    let handles = vak_core::commitments::begin_episodes(
        home.path(),
        &config,
        &intent,
        &plan,
        "Write a poem every morning",
        "session",
        home.path(),
        Some("local"),
        None,
    );
    assert_eq!(handles.len(), 1);
    let h = &handles[0];
    let ledger = CommitmentLedger::new(home.path());
    ledger
        .append(&Event::new(
            &h.commitment_id,
            EventKind::EpisodeEnded {
                episode_id: h.episode_id.clone(),
                advancement: Advancement::Stalled {
                    reason: "no progress".into(),
                },
                spend_usd: 2.0,
            },
        ))
        .unwrap();
    ledger
        .append(&Event::new(
            &h.commitment_id,
            EventKind::Suspended {
                suspension: vak_commit::Suspension::Human {
                    question_id: "q".into(),
                    question: "May I continue?".into(),
                    addressed_to: None,
                    escalation: vak_intent::Escalation::WaitIndefinitely,
                },
            },
        ))
        .unwrap();
    let before = ledger.get(&h.commitment_id).unwrap().unwrap();
    assert!(before.is_over_budget());
    assert_eq!(before.phase, vak_commit::Phase::Suspended);
    let plan =
        vak_core::commitments::plan_episodes(home.path(), &config, &intent, chrono::Utc::now());
    assert_eq!(plan.strands.len(), 1);
    let handles = vak_core::commitments::begin_episodes(
        home.path(),
        &config,
        &intent,
        &plan,
        "Write a poem every morning",
        "session",
        home.path(),
        Some("local"),
        None,
    );
    assert_eq!(handles.len(), 1);
    let after = ledger.get(&h.commitment_id).unwrap().unwrap();
    assert_eq!(after.phase, vak_commit::Phase::Active);
    assert!(after.suspension.is_none());
    assert!(after.is_over_budget());
    println!("over-budget suspended commitment admitted; unanswered question cleared");
}
