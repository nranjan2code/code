//! vak-intent: the decision layer that turns a request into a typed,
//! inspectable, narrowing-only engagement.
//!
//! # What this crate is for
//!
//! vak decides a great deal before a turn runs — which model, which tools,
//! which budget, which approval posture, what "done" means — and until now it
//! made every one of those decisions without any model of what the user was
//! trying to do. The only classifier in the codebase was a keyword scan that
//! fired on thirteen English verbs, and the route ladder's demand scorer was
//! being fed zeros.
//!
//! This crate supplies that missing model, as three deliberately separate
//! things:
//!
//! * [`Reading`] — what the request *is*, **inferred** on seven orthogonal
//!   axes ([`Act`], [`Horizon`], [`Stakes`], [`Evidence`], [`Clarity`],
//!   [`Modality`], [`Attendance`]).
//! * [`Authority`] — what a human has **delegated** ([`Autonomy`] and an
//!   [`Envelope`]). Never inferred, because a resolution bug must not be able
//!   to change what the agent is allowed to do.
//! * [`Engagement`] — what the runtime will therefore **do**, derived from the
//!   first two.
//!
//! # The one rule
//!
//! **Intent narrows, never widens.** An [`Engagement`]'s [`Limits`] may
//! subtract a capability, shorten the route ladder to a prefix, lower a
//! budget, or *raise* an approval floor. It can never grant a tool, extend a
//! ladder, raise a cap, or lower a floor.
//!
//! That is a property of the types rather than a rule to remember: [`Limits`]
//! is a meet semilattice whose top element ([`Limits::unrestricted`])
//! reproduces vak's pre-kernel behaviour, [`Limits::meet`] is the only
//! composition operator offered, and there is deliberately no `join` to reach
//! for by accident. [`Limits::is_at_most`] states the invariant as a
//! predicate, and the inline tests in `limits.rs` prove it holds
//! across every reading the resolver can produce.
//!
//! Two consequences worth stating explicitly:
//!
//! * **Intent never gates the permission engine.** Permission is evaluated
//!   exactly as it always was; an engagement can only add an approval
//!   requirement on top. Nothing here can authorize anything.
//! * **Uncertainty resolves to `general`.** A reading below the confidence
//!   floor produces [`Engagement::general`], which is byte-for-byte the
//!   behaviour that shipped before this crate existed. Being unsure must never
//!   silently take tools away.
//!
//! # Reproducibility
//!
//! Tiers 0 and 1 are pure functions of recorded inputs, so a decision that
//! narrowed a turn can be reconstructed from the ledger months later. Tiers 2
//! and 3 may call a model and are marked `reproducible: false` with the model
//! id and prompt digest that produced them — the same honesty the routing
//! ledger applies to `Settlement::Unknown`. Never claim replay fidelity you do
//! not have.

pub mod authority;
pub mod axes;
pub mod control;
pub mod engage;
pub mod goal;
pub mod limits;
pub mod outcome;
pub mod reading;
pub mod resolve;
pub mod signals;
pub mod strand;

pub use authority::{
    ApprovalCeiling, Authority, Autonomy, CapabilityDecision, CapabilityKind, CapabilityRequest,
    Envelope, Escalation, GateFallback, PermissionCeiling,
};
pub use axes::{
    Act, Attendance, Clarity, EpistemicStance, Evidence, Horizon, Modality, Satisfaction, Stakes,
};
pub use engage::FLOOR_DOMAINS;
pub use engage::{
    Cadence, ClarifyPolicy, ContextProfile, DeliveryPosture, DemandHint, Engagement, HilMode,
    OutputShape, Posture, StopProfile, Urgency, derive,
};
pub use goal::{GoalControlState, GoalRelation, GoalState, GoalUpdate, goal_relation};
pub use limits::{CapabilitySlice, DomainSet, Limits};
pub use outcome::{
    Command, CompletionVerdict, ControlSource, EvidenceReceipt, EvidenceState,
    InterventionDecision, InterventionEvaluation, InterventionKind, InterventionRequest,
    OutcomeRequirement, OutcomeSpec, OutcomeStatus, RequirementEvaluation, RequirementImportance,
    RequirementKind, RequirementOrigin, RequirementStatus, evaluate_completion,
    evaluate_intervention, evaluate_requirements, evaluate_requirements_with_evidence,
    evaluate_requirements_with_receipt, evaluate_requirements_with_state, evaluate_response,
    evaluate_response_with_failures, evidence_state_from_age, evidence_state_from_receipt,
    human_review_state, parse_command,
};
pub use reading::{Confidences, Intent, Provenance, Reading, Tier};
pub use resolve::{
    Classification, Declared, RESOLVER_VERSION, Resolution, ResolverConfig, apply_classification,
    classification_prompt, implied_stakes, parse_classifications, prompt_digest, resolve,
};
pub use signals::{
    Attachment, HistoryFacts, Request, Signal, SignalKind, Surface, WorkspaceFacts, extract,
};
pub use strand::{Lineage, LineageHint, Strand, StrandRelation, ThreadFact};
