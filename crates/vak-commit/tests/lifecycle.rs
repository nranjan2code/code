//! End-to-end commitment lifecycle over a real on-disk ledger.
//!
//! These exercise the properties that make a commitment worth having: that it
//! survives a restart, that it cannot be closed dishonestly, and that a
//! multi-day suspension is a pause rather than a loss.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::path::PathBuf;

use vak_commit::ledger::{Event, EventKind};
use vak_commit::{
    Advancement, CommitmentLedger, CommitmentSpec, Economics, Phase, Suspension, Verdict,
};
use vak_intent::{Escalation, Evidence, Horizon, Reading, Satisfaction, Stakes};
use vak_session::types::{CriterionKind, CriterionResult, WorkCriterion};

fn criterion(id: &str, kind: CriterionKind) -> WorkCriterion {
    WorkCriterion {
        criterion_id: id.into(),
        statement: format!("criterion {id}"),
        kind,
        required: true,
    }
}

fn spec(evidence: Evidence, criteria: Vec<WorkCriterion>) -> CommitmentSpec {
    let reading = Reading {
        horizon: Horizon::Durable,
        stakes: Stakes::Reversible,
        evidence,
        ..Reading::general()
    };
    vak_commit::spec_from_reading(
        "migrate the billing schema",
        reading,
        criteria,
        PathBuf::from("/tmp/workspace"),
        Economics::default(),
    )
}

#[test]
fn a_commitment_survives_a_process_restart() {
    let dir = tempfile::tempdir().unwrap();
    let id = {
        let ledger = CommitmentLedger::new(dir.path());
        let id = ledger
            .open_commitment(spec(Evidence::None, Vec::new()))
            .unwrap();
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
        id
    };

    // A completely fresh handle, as a restarted process would have.
    let reopened = CommitmentLedger::new(dir.path());
    let commitment = reopened.get(&id).unwrap().expect("commitment survived");
    assert_eq!(commitment.phase, Phase::Active);
    assert_eq!(commitment.episodes.len(), 1);
    assert_eq!(commitment.spec.objective, "migrate the billing schema");
}

/// The closure invariant, end to end: a commitment held to `Verified` cannot
/// be closed by the model saying it went well.
#[test]
fn asserted_evidence_cannot_close_work_that_requires_observation() {
    let dir = tempfile::tempdir().unwrap();
    let ledger = CommitmentLedger::new(dir.path());
    let id = ledger
        .open_commitment(spec(
            Evidence::Verified,
            vec![criterion(
                "tests-pass",
                CriterionKind::Shell {
                    command: "cargo test".into(),
                },
            )],
        ))
        .unwrap();

    // The model claims success. Recorded as `Semantic`-grade evidence.
    ledger
        .append(&Event::new(
            &id,
            EventKind::CriterionEvaluated {
                criterion_id: "tests-pass".into(),
                result: CriterionResult::Passed {
                    evidence: "I ran the tests and they passed".into(),
                },
                strength: Satisfaction::Asserted,
            },
        ))
        .unwrap();

    let refused = ledger.append(&Event::new(
        &id,
        EventKind::Closed {
            verdict: Verdict::Fulfilled,
            strength: Satisfaction::Asserted,
            evidence: Vec::new(),
            note: "done".into(),
        },
    ));
    let message = refused.expect_err("closure should be refused").to_string();
    assert!(
        message.contains("requires observed"),
        "unexpected refusal: {message}"
    );

    // The commitment is untouched and still open.
    let commitment = ledger.get(&id).unwrap().unwrap();
    assert!(!commitment.phase.is_terminal());
    assert!(commitment.closure.is_none());
}

#[test]
fn a_runtime_observed_criterion_does_close_the_same_work() {
    let dir = tempfile::tempdir().unwrap();
    let ledger = CommitmentLedger::new(dir.path());
    let id = ledger
        .open_commitment(spec(
            Evidence::Verified,
            vec![criterion(
                "tests-pass",
                CriterionKind::Shell {
                    command: "cargo test".into(),
                },
            )],
        ))
        .unwrap();

    ledger
        .append(&Event::new(
            &id,
            EventKind::CriterionEvaluated {
                criterion_id: "tests-pass".into(),
                result: CriterionResult::Passed {
                    evidence: "exit 0".into(),
                },
                strength: Satisfaction::Observed,
            },
        ))
        .unwrap();
    ledger
        .append(&Event::new(
            &id,
            EventKind::Closed {
                verdict: Verdict::Fulfilled,
                strength: Satisfaction::Observed,
                evidence: Vec::new(),
                note: "tests green".into(),
            },
        ))
        .unwrap();

    let commitment = ledger.get(&id).unwrap().unwrap();
    assert_eq!(commitment.phase, Phase::Closed);
    assert_eq!(
        commitment.closure.as_ref().unwrap().verdict,
        Verdict::Fulfilled
    );
}

/// Recording bad news must always be possible, or the ledger cannot tell the
/// truth about work that went wrong.
#[test]
fn failure_verdicts_are_never_blocked_by_the_evidence_requirement() {
    for verdict in [
        Verdict::Failed,
        Verdict::Abandoned,
        Verdict::Expired,
        Verdict::Unknown,
        Verdict::Partial,
    ] {
        let dir = tempfile::tempdir().unwrap();
        let ledger = CommitmentLedger::new(dir.path());
        let id = ledger
            .open_commitment(spec(
                Evidence::Audited,
                vec![criterion("never-run", CriterionKind::Semantic)],
            ))
            .unwrap();
        ledger
            .append(&Event::new(
                &id,
                EventKind::Closed {
                    verdict,
                    strength: Satisfaction::Asserted,
                    evidence: Vec::new(),
                    note: "recorded honestly".into(),
                },
            ))
            .unwrap_or_else(|error| panic!("{verdict:?} was refused: {error}"));
        assert_eq!(
            ledger.get(&id).unwrap().unwrap().closure.unwrap().verdict,
            verdict
        );
    }
}

#[test]
fn outstanding_criteria_block_a_success_claim() {
    let dir = tempfile::tempdir().unwrap();
    let ledger = CommitmentLedger::new(dir.path());
    let id = ledger
        .open_commitment(spec(
            Evidence::None,
            vec![
                criterion("a", CriterionKind::Semantic),
                criterion("b", CriterionKind::Semantic),
            ],
        ))
        .unwrap();
    ledger
        .append(&Event::new(
            &id,
            EventKind::CriterionEvaluated {
                criterion_id: "a".into(),
                result: CriterionResult::Passed {
                    evidence: "looks right".into(),
                },
                strength: Satisfaction::Asserted,
            },
        ))
        .unwrap();

    let refused = ledger.append(&Event::new(
        &id,
        EventKind::Closed {
            verdict: Verdict::Fulfilled,
            strength: Satisfaction::Asserted,
            evidence: Vec::new(),
            note: "half done".into(),
        },
    ));
    assert!(
        refused.unwrap_err().to_string().contains("have not passed"),
        "a commitment closed with a criterion outstanding"
    );
}

/// A multi-day wait is a pause, not a loss: the work suspends, survives a
/// restart, and resumes exactly where it was.
#[test]
fn a_suspended_commitment_pauses_and_resumes_across_a_restart() {
    let dir = tempfile::tempdir().unwrap();
    let id = {
        let ledger = CommitmentLedger::new(dir.path());
        let id = ledger
            .open_commitment(spec(Evidence::None, Vec::new()))
            .unwrap();
        ledger
            .append(&Event::new(
                &id,
                EventKind::Suspended {
                    suspension: Suspension::Human {
                        question_id: "q1".into(),
                        question: "which database should this target?".into(),
                        addressed_to: Some("nisheeth".into()),
                        escalation: Escalation::WaitIndefinitely,
                    },
                },
            ))
            .unwrap();
        id
    };

    let ledger = CommitmentLedger::new(dir.path());
    let suspended = ledger.get(&id).unwrap().unwrap();
    assert_eq!(suspended.phase, Phase::Suspended);
    assert!(
        suspended
            .suspension
            .as_ref()
            .unwrap()
            .describe()
            .contains("which database")
    );
    // A suspended commitment is not schedulable, but it is also not closed.
    assert!(!suspended.phase.is_schedulable());
    assert!(!suspended.phase.is_terminal());

    ledger
        .append(&Event::new(
            &id,
            EventKind::QuestionAnswered {
                question_id: "q1".into(),
                answer: "the staging replica".into(),
            },
        ))
        .unwrap();
    let resumed = ledger.get(&id).unwrap().unwrap();
    assert_eq!(resumed.phase, Phase::Active);
    assert!(resumed.suspension.is_none());
}

/// `Learned` is progress. An episode that reduced uncertainty without moving a
/// criterion must clear the stall streak, or exploration gets punished.
#[test]
fn learning_clears_the_stall_streak_but_stalling_accumulates() {
    let dir = tempfile::tempdir().unwrap();
    let ledger = CommitmentLedger::new(dir.path());
    let id = ledger
        .open_commitment(spec(Evidence::None, Vec::new()))
        .unwrap();

    let end = |episode: &str, advancement: Advancement| {
        ledger
            .append(&Event::new(
                &id,
                EventKind::EpisodeStarted {
                    episode_id: episode.into(),
                    session_id: format!("s-{episode}"),
                    strand_id: None,
                },
            ))
            .unwrap();
        ledger
            .append(&Event::new(
                &id,
                EventKind::EpisodeEnded {
                    episode_id: episode.into(),
                    advancement,
                    spend_usd: 0.25,
                },
            ))
            .unwrap();
    };

    end(
        "e1",
        Advancement::Stalled {
            reason: "went in circles".into(),
        },
    );
    end(
        "e2",
        Advancement::Stalled {
            reason: "again".into(),
        },
    );
    assert_eq!(ledger.get(&id).unwrap().unwrap().consecutive_stalls, 2);

    end(
        "e3",
        Advancement::Learned {
            fact: "the schema is owned by another service".into(),
        },
    );
    let commitment = ledger.get(&id).unwrap().unwrap();
    assert_eq!(commitment.consecutive_stalls, 0, "learning is progress");
    assert!(!commitment.is_stalled());
    assert!((commitment.spend_usd - 0.75).abs() < 1e-9);

    for episode in ["e4", "e5", "e6"] {
        end(
            episode,
            Advancement::Stalled {
                reason: "stuck".into(),
            },
        );
    }
    assert!(
        ledger.get(&id).unwrap().unwrap().is_stalled(),
        "three consecutive stalls should trip the breaker"
    );
}

#[test]
fn a_closed_commitment_cannot_be_closed_again() {
    let dir = tempfile::tempdir().unwrap();
    let ledger = CommitmentLedger::new(dir.path());
    let id = ledger
        .open_commitment(spec(Evidence::None, Vec::new()))
        .unwrap();
    let close = |note: &str| {
        Event::new(
            &id,
            EventKind::Closed {
                verdict: Verdict::Failed,
                strength: Satisfaction::Asserted,
                evidence: Vec::new(),
                note: note.into(),
            },
        )
    };
    ledger.append(&close("first")).unwrap();
    assert!(
        ledger
            .append(&close("second"))
            .unwrap_err()
            .to_string()
            .contains("already closed")
    );
}

#[test]
fn supersession_records_lineage_rather_than_orphaning_work() {
    let dir = tempfile::tempdir().unwrap();
    let ledger = CommitmentLedger::new(dir.path());
    let old = ledger
        .open_commitment(spec(Evidence::None, Vec::new()))
        .unwrap();
    let new = ledger
        .open_commitment(spec(Evidence::None, Vec::new()))
        .unwrap();

    ledger
        .append(&Event::new(
            &old,
            EventKind::Superseded {
                by: new.clone(),
                reason: "just add the index instead".into(),
            },
        ))
        .unwrap();

    let superseded = ledger.get(&old).unwrap().unwrap();
    assert_eq!(superseded.phase, Phase::Closed);
    assert_eq!(
        superseded.closure.as_ref().unwrap().verdict,
        Verdict::Superseded
    );
    assert_eq!(superseded.superseded_by.as_deref(), Some(new.as_str()));
    // The replacement is still open and independently trackable.
    assert!(!ledger.get(&new).unwrap().unwrap().phase.is_terminal());
    assert_eq!(ledger.open().len(), 1);
}

/// A truncated or corrupt ledger must decline to invent state rather than
/// projecting something plausible.
#[test]
fn a_ledger_without_an_opening_event_projects_nothing() {
    let dir = tempfile::tempdir().unwrap();
    let ledger = CommitmentLedger::new(dir.path());
    let path = ledger.path().to_path_buf();
    std::fs::write(
        &path,
        format!(
            "{}\nnot json at all\n",
            serde_json::to_string(&Event::new(
                "orphan",
                EventKind::Resumed {
                    reason: "no opening event".into()
                },
            ))
            .unwrap()
        ),
    )
    .unwrap();
    assert!(ledger.get("orphan").unwrap().is_none());
    assert!(ledger.all().is_empty());
}

/// A torn or non-UTF-8 line in the middle of the ledger is skipped, and every
/// event written after it still counts. Stopping at the first bad line would
/// silently roll every later commitment back to an older state.
#[test]
fn a_row_that_is_not_an_event_hides_nothing_written_after_it() {
    let dir = tempfile::tempdir().unwrap();
    let ledger = CommitmentLedger::new(dir.path());
    let id = ledger
        .open_commitment(spec(Evidence::None, Vec::new()))
        .unwrap();
    vak_session::chain::RecordChain::at(ledger.path())
        .append(&serde_json::json!({ "torn": true }))
        .unwrap();
    ledger
        .append(&Event::new(
            &id,
            EventKind::Blocked {
                blocker: "waiting on credentials".into(),
            },
        ))
        .unwrap();
    let commitment = ledger.get(&id).unwrap().unwrap();
    assert_eq!(commitment.phase, Phase::Blocked);
    assert_eq!(
        commitment.blocker.as_deref(),
        Some("waiting on credentials")
    );
}

/// The strength a closure records is the runtime's, recomputed from the
/// criteria at append time — a caller cannot write a stronger claim than the
/// commitment holds, even for a verdict that claims no success.
#[test]
fn a_closure_records_the_achieved_strength_not_the_claimed_one() {
    let dir = tempfile::tempdir().unwrap();
    let ledger = CommitmentLedger::new(dir.path());
    let id = ledger
        .open_commitment(spec(
            Evidence::None,
            vec![criterion("looked", CriterionKind::Semantic)],
        ))
        .unwrap();
    ledger
        .append(&Event::new(
            &id,
            EventKind::Closed {
                verdict: Verdict::Partial,
                strength: Satisfaction::Attested,
                evidence: Vec::new(),
                note: "claims an audit that never happened".into(),
            },
        ))
        .unwrap();
    let closure = ledger.get(&id).unwrap().unwrap().closure.unwrap();
    assert_eq!(closure.strength, Satisfaction::Asserted);
}

/// A new episode is somebody working the commitment again, so the blocker
/// that stopped the previous one no longer describes it.
#[test]
fn starting_an_episode_clears_the_previous_blocker() {
    let dir = tempfile::tempdir().unwrap();
    let ledger = CommitmentLedger::new(dir.path());
    let id = ledger
        .open_commitment(spec(Evidence::None, Vec::new()))
        .unwrap();
    for event in [
        EventKind::EpisodeStarted {
            episode_id: "e1".into(),
            session_id: "s1".into(),
            strand_id: None,
        },
        EventKind::EpisodeEnded {
            episode_id: "e1".into(),
            advancement: Advancement::Blocked {
                blocker: "cancelled".into(),
            },
            spend_usd: 0.0,
        },
        EventKind::EpisodeStarted {
            episode_id: "e2".into(),
            session_id: "s1".into(),
            strand_id: None,
        },
    ] {
        ledger.append(&Event::new(&id, event)).unwrap();
    }
    let commitment = ledger.get(&id).unwrap().unwrap();
    assert_eq!(commitment.phase, Phase::Active);
    assert!(commitment.blocker.is_none());
}

/// An episode names the strand it works on (docs/design/85-turn-graph.md,
/// G0); its turn comes from the event's trace key.
#[test]
fn an_episode_names_its_strand() {
    let dir = tempfile::tempdir().unwrap();
    let ledger = CommitmentLedger::new(dir.path());
    let id = ledger
        .open_commitment(spec(Evidence::None, Vec::new()))
        .unwrap();
    ledger
        .append(&Event::new(
            &id,
            EventKind::EpisodeStarted {
                episode_id: "e1".into(),
                session_id: "s1".into(),
                strand_id: Some("turn-1.0".into()),
            },
        ))
        .unwrap();
    let commitment = ledger.get(&id).unwrap().unwrap();
    assert_eq!(
        commitment.episodes[0].strand_id.as_deref(),
        Some("turn-1.0")
    );
}

/// Concurrent writers serialize: every append lands, none is lost or torn.
#[test]
fn concurrent_appends_all_land() {
    let dir = tempfile::tempdir().unwrap();
    let id = CommitmentLedger::new(dir.path())
        .open_commitment(spec(Evidence::None, Vec::new()))
        .unwrap();
    let writers: Vec<_> = (0..8)
        .map(|writer| {
            let home = dir.path().to_path_buf();
            let id = id.clone();
            std::thread::spawn(move || {
                let ledger = CommitmentLedger::new(&home);
                for n in 0..10 {
                    ledger
                        .append(&Event::new(
                            &id,
                            EventKind::Resumed {
                                reason: format!("writer {writer} pass {n}"),
                            },
                        ))
                        .unwrap();
                }
            })
        })
        .collect();
    for writer in writers {
        writer.join().unwrap();
    }
    assert_eq!(CommitmentLedger::new(dir.path()).events_for(&id).len(), 81);
}

#[test]
fn revoking_an_envelope_stops_it_granting_anything() {
    let dir = tempfile::tempdir().unwrap();
    let ledger = CommitmentLedger::new(dir.path());
    let id = ledger
        .open_commitment(spec(Evidence::None, Vec::new()))
        .unwrap();
    let envelope = vak_intent::Envelope {
        envelope_id: "env-1".into(),
        granted_by: "nisheeth".into(),
        granted_at: chrono::Utc::now(),
        expires_at: None,
        spend_limit_usd: Some(20.0),
        path_scope: vec!["src/**".into()],
        tool_scope: vec!["edit".into()],
        permission_ceiling: vak_intent::PermissionCeiling::WorkspaceWrite,
        escalation: Escalation::WaitIndefinitely,
        revoked_at: None,
    };
    ledger
        .append(&Event::new(
            &id,
            EventKind::EnvelopeGranted {
                envelope: Box::new(envelope),
            },
        ))
        .unwrap();
    assert!(
        ledger
            .get(&id)
            .unwrap()
            .unwrap()
            .envelope
            .unwrap()
            .is_live(chrono::Utc::now())
    );

    ledger
        .append(&Event::new(
            &id,
            EventKind::EnvelopeRevoked {
                envelope_id: "env-1".into(),
                by: "nisheeth".into(),
            },
        ))
        .unwrap();
    assert!(
        !ledger
            .get(&id)
            .unwrap()
            .unwrap()
            .envelope
            .unwrap()
            .is_live(chrono::Utc::now())
    );
}

fn in_session(session: uuid::Uuid) -> vak_session::trace::TraceKey {
    use vak_session::ids::{AgentId, SpaceId, TenantId};
    vak_session::trace::TraceKey::root(
        TenantId::new(),
        SpaceId::new(),
        AgentId::new(),
        vak_session::trace::Cause::Heartbeat,
    )
    .in_turn(&session.to_string(), &uuid::Uuid::now_v7().to_string())
}

/// What a commitment says belongs to the conversation it was made in
/// (plan M7a-b): the chain keeps ids and kinds, and erasing the
/// conversation takes the commitment with it and leaves others alone.
#[test]
fn a_commitment_goes_with_the_conversation_that_made_it() {
    let dir = tempfile::tempdir().unwrap();
    let (kept_session, gone_session) = (uuid::Uuid::now_v7(), uuid::Uuid::now_v7());
    let open = |session| {
        CommitmentLedger::new(dir.path())
            .with_trace(Some(&in_session(session)))
            .open_commitment(spec(Evidence::None, Vec::new()))
            .unwrap()
    };
    let (kept, gone) = (open(kept_session), open(gone_session));
    let ledger = CommitmentLedger::new(dir.path());

    let stored = vak_session::chain::RecordChain::at(ledger.path()).text();
    assert!(!stored.contains("billing schema"), "{stored}");
    assert!(stored.contains(&gone) && stored.contains("\"kind\":\"opened\""));
    assert_eq!(ledger.all().len(), 2);
    assert_eq!(
        ledger.get(&gone).unwrap().unwrap().spec.objective,
        "migrate the billing schema"
    );

    vak_session::objects::TenantObjects::for_tenant(&vak_config::paths::local_tenant_home())
        .unwrap()
        .destroy_scope_key(&vak_session::objects::conversation_scope(
            &gone_session.to_string(),
        ))
        .unwrap();

    assert_eq!(ledger.events().len(), 1);
    assert_eq!(ledger.events_for(&gone).len(), 0);
    assert_eq!(ledger.events_for(&kept).len(), 1);
    // A rollup written before the erasure still holds what it folded; a
    // fresh fold of the chain does not.
    std::fs::remove_dir_all(dir.path()).ok();
}
