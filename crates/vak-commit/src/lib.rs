//! vak-commit: durable obligations, and a falsifiable answer to "is it done?"
//!
//! # What this replaces
//!
//! vak's unit of identity was the session, and completion was "the model
//! stopped talking". Both assumptions break as soon as work outlives one
//! conversation. A [`Commitment`] inverts them:
//!
//! | Before | Now |
//! |---|---|
//! | the session is the identity | the commitment is; sessions are [`Episode`]s that advance it |
//! | the model decides it is done | the **runtime** evaluates a predicate against the world |
//! | progress is tokens spent | progress is [`Advancement`]: criteria moved, or an explicit reason none did |
//! | blocking or losing the thread | [`Suspension`] against a typed wake condition |
//! | work quietly evaporates | every commitment [`Closure`]s with a [`Verdict`] and evidence |
//!
//! The agent loop does not disappear; it becomes the implementation of an
//! episode rather than the top-level abstraction.
//!
//! # The closure invariant
//!
//! A commitment declares the weakest [`vak_intent::Satisfaction`] that may
//! close it as [`Verdict::Fulfilled`], taken from its reading's `evidence`
//! axis. [`Commitment::may_close`] enforces it, and
//! [`ledger::CommitmentLedger::append`] refuses the event outright — a ledger
//! that can record a lie is not an audit trail.
//!
//! Crucially, the strength of a criterion comes from **how it was
//! established**, not from anyone's confidence: a shell command the runtime
//! ran is `Observed`, an external receipt is `Attested`, and the model's own
//! judgement is `Asserted` no matter how emphatically it is phrased. See
//! [`satisfy`].
//!
//! Failure verdicts are deliberately *unconstrained*. Recording a
//! [`Verdict::Failed`], a [`Verdict::Abandoned`], or an honest
//! [`Verdict::Unknown`] must always be possible, or the ledger could not tell
//! the truth about work that went wrong.
//!
//! # Reuse
//!
//! Criterion and evidence vocabulary comes from `vak-session`
//! (`CriterionKind`, `CriterionResult`, `EvidenceRef`, `WorkCriterion`,
//! `WorkOwner`) rather than being redefined here. That vocabulary already
//! expresses "this shell command exits zero" and "this flow completed"; a
//! second one for the same idea would be a liability.

pub mod ledger;
pub mod portfolio;
pub mod satisfy;
pub mod types;

pub use ledger::{CommitmentLedger, Event, EventKind, LedgerError, project, spec_from_reading};
pub use portfolio::{Priority, SchedulerContext, next, prioritize, rank};
pub use satisfy::{
    CriterionEvaluator, Evaluation, evaluate_all, is_machine_checkable, strength_of,
};
pub use types::{
    Advancement, Closure, ClosureRefusal, Commitment, CommitmentSpec, CriterionState, Economics,
    Episode, Phase, Suspension, Verdict,
};
