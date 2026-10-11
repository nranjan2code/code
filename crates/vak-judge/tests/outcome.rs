//! The outcome evaluators over whole specifications.
#![allow(clippy::unwrap_used, clippy::expect_used)]

use vak_intent::outcome::{
    CompletionVerdict, EvidenceState, OutcomeSpec, OutcomeStatus, RequirementEvaluation,
    RequirementStatus,
};
use vak_intent::reading::Reading;
use vak_intent::{Act, Evidence};
use vak_judge::outcome::{
    TurnFacts, evaluate_completion, evaluate_requirements, evidence_state_from_age,
    human_review_state,
};

#[test]
fn outcome_completion_verdict_matrix() {
    let reading = Reading {
        act: Act::Modify,
        evidence: Evidence::Cited,
        ..Reading::general()
    };
    let spec = OutcomeSpec::from_reading("research", &reading, 1);

    let all_met = vec![
        RequirementEvaluation {
            requirement_id: "deliverable-1".into(),
            status: RequirementStatus::Met,
            reason: "content exists".into(),
        },
        RequirementEvaluation {
            requirement_id: "evidence-1".into(),
            status: RequirementStatus::Met,
            reason: "sources cited".into(),
        },
    ];
    assert_eq!(
        evaluate_completion(OutcomeStatus::Produced, &all_met, &spec),
        CompletionVerdict::Complete
    );

    let partial = vec![
        RequirementEvaluation {
            requirement_id: "deliverable-1".into(),
            status: RequirementStatus::Met,
            reason: "content exists".into(),
        },
        RequirementEvaluation {
            requirement_id: "evidence-1".into(),
            status: RequirementStatus::Unmet,
            reason: "no evidence".into(),
        },
    ];
    assert_eq!(
        evaluate_completion(OutcomeStatus::Produced, &partial, &spec),
        CompletionVerdict::Partial
    );

    let unknown_evals = vec![
        RequirementEvaluation {
            requirement_id: "deliverable-1".into(),
            status: RequirementStatus::Met,
            reason: "content exists".into(),
        },
        RequirementEvaluation {
            requirement_id: "evidence-1".into(),
            status: RequirementStatus::Unknown,
            reason: "sources cited but unverified".into(),
        },
    ];
    assert_eq!(
        evaluate_completion(OutcomeStatus::Produced, &unknown_evals, &spec),
        CompletionVerdict::Unknown
    );

    assert_eq!(
        evaluate_completion(OutcomeStatus::Failed, &[], &spec),
        CompletionVerdict::Failed
    );
    assert_eq!(
        evaluate_completion(OutcomeStatus::Cancelled, &[], &spec),
        CompletionVerdict::Cancelled
    );
}

#[test]
fn outcome_evaluation_unobserved_deliverable_is_not_met() {
    let reading = Reading {
        act: Act::Author,
        evidence: Evidence::None,
        ..Reading::general()
    };
    let mut spec = OutcomeSpec::from_reading("create a report", &reading, 1);
    spec.merge_declared_requirement("report", "deliverable", "create report.md", "must", None)
        .unwrap();

    let unobserved = TurnFacts {
        deliverable_observed: false,
        ..TurnFacts::reply(Some("I cannot do that."))
    };
    let evaluations = evaluate_requirements(&spec, &unobserved);
    assert_eq!(evaluations[1].status, RequirementStatus::Unknown);
    assert_eq!(
        evaluate_completion(OutcomeStatus::Produced, &evaluations, &spec),
        CompletionVerdict::Unknown
    );
}

#[test]
fn outcome_evidence_state_age_based() {
    let now = chrono::Utc::now();
    assert_eq!(
        evidence_state_from_age(
            now,
            now - chrono::Duration::hours(1),
            chrono::Duration::hours(2)
        ),
        EvidenceState::Fresh
    );
    assert_eq!(
        evidence_state_from_age(
            now,
            now - chrono::Duration::hours(3),
            chrono::Duration::hours(2)
        ),
        EvidenceState::Stale
    );
    assert_eq!(
        evidence_state_from_age(
            now,
            now + chrono::Duration::minutes(1),
            chrono::Duration::hours(2)
        ),
        EvidenceState::Fresh
    );
}

#[test]
fn outcome_human_review_state() {
    assert_eq!(
        human_review_state(CompletionVerdict::Complete),
        "not_required"
    );
    assert_eq!(
        human_review_state(CompletionVerdict::Partial),
        "recommended"
    );
    assert_eq!(
        human_review_state(CompletionVerdict::Unknown),
        "recommended"
    );
    assert_eq!(
        human_review_state(CompletionVerdict::Failed),
        "required_for_recovery"
    );
    assert_eq!(
        human_review_state(CompletionVerdict::Cancelled),
        "required_for_recovery"
    );
}
