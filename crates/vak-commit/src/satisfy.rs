//! Evaluating whether a commitment is actually done.
//!
//! # The separation of powers
//!
//! The model may **propose** criteria. It may never **mark one passed**. That
//! is the same split as permission-before-dispatch, applied to completion: the
//! thing being evaluated does not get to grade itself.
//!
//! Mechanically that means every criterion carries a [`Satisfaction`] derived
//! from *how it was established*, not from how confident anyone is:
//!
//! | Criterion | Strength | Because |
//! |---|---|---|
//! | `Shell`, `FileExists`, `FileContains`, `ToolSucceeded`, `FlowCompleted` | `Observed` | the runtime ran it against the world |
//! | `ExternalReceipt` | `Attested` | a third party confirmed it |
//! | `Semantic` | `Asserted` | only the model's judgement backs it |
//!
//! A commitment whose `evidence` axis was `Verified` therefore cannot be
//! closed by a pile of `Semantic` criteria, however emphatic the transcript
//! is. That is the whole point.
//!
//! # Why the evaluator is a trait
//!
//! Running a shell command is a permissioned effect that belongs behind
//! vak's tool broker and permission engine. This crate defines the contract
//! and the strength mapping; `vak-core` supplies the implementation that
//! actually crosses that boundary.

use vak_intent::Satisfaction;
use vak_session::types::{CriterionKind, CriterionResult, WorkCriterion};

/// How strongly a criterion of this kind is backed once it passes.
///
/// This mapping is the closure invariant's factual half, and it is
/// deliberately a total function over `CriterionKind` so that adding a new
/// kind forces an explicit decision about its evidentiary weight.
pub fn strength_of(kind: &CriterionKind) -> Satisfaction {
    match kind {
        // The runtime executed something and observed the result.
        CriterionKind::Shell { .. }
        | CriterionKind::FileExists { .. }
        | CriterionKind::FileContains { .. }
        | CriterionKind::ToolSucceeded { .. }
        | CriterionKind::FlowCompleted { .. } => Satisfaction::Observed,
        // Someone outside this runtime vouched for it.
        CriterionKind::ExternalReceipt { .. } => Satisfaction::Attested,
        // Nothing but the model's own judgement.
        CriterionKind::Semantic => Satisfaction::Asserted,
    }
}

/// Whether this criterion can be checked without a model.
pub fn is_machine_checkable(kind: &CriterionKind) -> bool {
    strength_of(kind).rank() >= Satisfaction::Observed.rank()
}

/// One criterion's evaluation.
#[derive(Debug, Clone, PartialEq)]
pub struct Evaluation {
    pub criterion_id: String,
    pub result: CriterionResult,
    pub strength: Satisfaction,
}

impl Evaluation {
    /// Record a runtime-observed outcome.
    pub fn observed(criterion: &WorkCriterion, result: CriterionResult) -> Self {
        Evaluation {
            criterion_id: criterion.criterion_id.clone(),
            // A criterion that failed or could not be determined carries no
            // evidentiary weight, so its strength is the floor regardless of
            // kind. Only a pass earns the kind's strength.
            strength: match &result {
                CriterionResult::Passed { .. } => strength_of(&criterion.kind),
                _ => Satisfaction::Asserted,
            },
            result,
        }
    }

    /// Record a human's attestation. The only path to `Attested` other than an
    /// external receipt.
    pub fn attested(criterion_id: impl Into<String>, by: &str, note: &str) -> Self {
        Evaluation {
            criterion_id: criterion_id.into(),
            result: CriterionResult::Passed {
                evidence: format!("attested by {by}: {note}"),
            },
            strength: Satisfaction::Attested,
        }
    }

    /// Record that the runtime could not determine the outcome.
    ///
    /// Distinct from a failure: a check that could not run says nothing about
    /// the work, and recording it as a failure would be as dishonest as
    /// recording it as a pass.
    pub fn unknown(criterion_id: impl Into<String>, reason: impl Into<String>) -> Self {
        Evaluation {
            criterion_id: criterion_id.into(),
            result: CriterionResult::Unknown {
                reason: reason.into(),
            },
            strength: Satisfaction::Asserted,
        }
    }

    pub fn passed(&self) -> bool {
        matches!(self.result, CriterionResult::Passed { .. })
    }
}

/// Evaluates criteria against the world.
///
/// Implemented in `vak-core`, where the tool broker, permission engine and
/// sandbox live. A criterion evaluation is an ordinary permissioned effect and
/// crosses exactly the same boundary as any other.
#[allow(async_fn_in_trait)]
pub trait CriterionEvaluator {
    /// Evaluate one criterion. Implementations must return
    /// [`CriterionResult::Unknown`] rather than a failure when the check could
    /// not be performed — a denied permission, a missing binary, a timeout.
    async fn evaluate(&self, criterion: &WorkCriterion) -> Evaluation;
}

/// Evaluate every machine-checkable criterion in a spec.
///
/// `Semantic` criteria are skipped: there is nothing for the runtime to
/// observe, and asking a model to confirm its own work would launder an
/// assertion into something that looks stronger than it is. They stay at
/// `Asserted` unless a human attests them.
pub async fn evaluate_all<E: CriterionEvaluator>(
    evaluator: &E,
    criteria: &[WorkCriterion],
) -> Vec<Evaluation> {
    let mut out = Vec::new();
    for criterion in criteria {
        if !is_machine_checkable(&criterion.kind) {
            continue;
        }
        out.push(evaluator.evaluate(criterion).await);
    }
    out
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn criterion(id: &str, kind: CriterionKind) -> WorkCriterion {
        WorkCriterion {
            criterion_id: id.into(),
            statement: format!("criterion {id}"),
            kind,
            required: true,
        }
    }

    #[test]
    fn semantic_criteria_are_only_ever_asserted() {
        assert_eq!(
            strength_of(&CriterionKind::Semantic),
            Satisfaction::Asserted
        );
        assert!(!is_machine_checkable(&CriterionKind::Semantic));
        // And therefore cannot discharge a `Verified` requirement.
        assert!(!Satisfaction::Asserted.satisfies(Satisfaction::Observed));
    }

    #[test]
    fn runtime_checked_criteria_are_observed() {
        for kind in [
            CriterionKind::Shell {
                command: "cargo test".into(),
            },
            CriterionKind::FileExists {
                path: PathBuf::from("out.txt"),
            },
            CriterionKind::FileContains {
                path: PathBuf::from("out.txt"),
                pattern: "ok".into(),
            },
            CriterionKind::ToolSucceeded {
                tool: "bash".into(),
            },
            CriterionKind::FlowCompleted {
                flow: "release".into(),
            },
        ] {
            assert_eq!(strength_of(&kind), Satisfaction::Observed);
            assert!(is_machine_checkable(&kind));
        }
    }

    #[test]
    fn external_receipts_are_attested() {
        let kind = CriterionKind::ExternalReceipt {
            integration: "stripe".into(),
        };
        assert_eq!(strength_of(&kind), Satisfaction::Attested);
    }

    /// A failed or undetermined check must not carry its kind's strength —
    /// otherwise a failing shell criterion would count as `Observed` evidence.
    #[test]
    fn only_a_pass_earns_the_kinds_strength() {
        let shell = criterion(
            "c1",
            CriterionKind::Shell {
                command: "false".into(),
            },
        );
        let failed = Evaluation::observed(
            &shell,
            CriterionResult::Failed {
                reason: "exit 1".into(),
            },
        );
        assert_eq!(failed.strength, Satisfaction::Asserted);
        assert!(!failed.passed());

        let passed = Evaluation::observed(
            &shell,
            CriterionResult::Passed {
                evidence: "exit 0".into(),
            },
        );
        assert_eq!(passed.strength, Satisfaction::Observed);
    }

    #[test]
    fn an_undeterminable_check_is_unknown_not_failed() {
        let evaluation = Evaluation::unknown("c1", "permission denied");
        assert!(matches!(evaluation.result, CriterionResult::Unknown { .. }));
        assert!(!evaluation.passed());
    }

    #[test]
    fn human_attestation_reaches_the_top_of_the_lattice() {
        let evaluation = Evaluation::attested("c1", "nisheeth", "checked the dashboard");
        assert_eq!(evaluation.strength, Satisfaction::Attested);
        assert!(evaluation.strength.satisfies(Satisfaction::Attested));
    }

    struct StubEvaluator;

    impl CriterionEvaluator for StubEvaluator {
        async fn evaluate(&self, criterion: &WorkCriterion) -> Evaluation {
            Evaluation::observed(
                criterion,
                CriterionResult::Passed {
                    evidence: "stub".into(),
                },
            )
        }
    }

    #[tokio::test]
    async fn evaluate_all_skips_semantic_criteria() {
        let criteria = vec![
            criterion("c1", CriterionKind::Semantic),
            criterion(
                "c2",
                CriterionKind::Shell {
                    command: "true".into(),
                },
            ),
        ];
        let evaluations = evaluate_all(&StubEvaluator, &criteria).await;
        assert_eq!(evaluations.len(), 1);
        assert_eq!(evaluations[0].criterion_id, "c2");
    }
}
