//! The durable commitment and its lifecycle.
//!
//! # Why this exists
//!
//! vak's unit of identity has been the session, and completion has been "the
//! model stopped talking". Both break the moment work outlives one
//! conversation: there is nowhere to put a month-long obligation, and no way
//! to say whether it was ever actually met.
//!
//! A [`Commitment`] inverts that. It is the durable identity; sessions become
//! [`Episode`]s that advance it. It carries a satisfaction condition the
//! **runtime** evaluates, so "done" is a fact about the world rather than a
//! claim in a transcript. And it closes explicitly, with a [`Verdict`] and
//! evidence — including [`Verdict::Unknown`], because a system that cannot
//! admit it lost track of something will quietly accumulate work nobody is
//! doing.
//!
//! Criterion and evidence types are reused from `vak-session` rather than
//! redefined: `CriterionKind` already knows how to express "this shell command
//! exits zero" and "this flow completed", and a second vocabulary for the same
//! idea would be a liability.

use serde::{Deserialize, Serialize};

use vak_intent::{Envelope, Reading, Satisfaction};
use vak_session::types::{CriterionResult, EvidenceRef, WorkCriterion};

/// Where a commitment is in its life.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Phase {
    /// Resolved but not yet accepted — awaiting a human, or a clarification.
    Proposed,
    /// Being worked.
    Active,
    /// Waiting on a typed external condition. Not a failure: the work is
    /// intact and will resume.
    Suspended,
    /// Cannot proceed and cannot wait its way out. Needs intervention.
    Blocked,
    /// Criteria are being evaluated.
    Satisfying,
    /// Terminal.
    Closed,
}

impl Phase {
    pub fn as_str(self) -> &'static str {
        match self {
            Phase::Proposed => "proposed",
            Phase::Active => "active",
            Phase::Suspended => "suspended",
            Phase::Blocked => "blocked",
            Phase::Satisfying => "satisfying",
            Phase::Closed => "closed",
        }
    }

    pub fn is_terminal(self) -> bool {
        self == Phase::Closed
    }

    /// Whether the portfolio scheduler may pick this up for work.
    pub fn is_schedulable(self) -> bool {
        matches!(self, Phase::Active | Phase::Satisfying)
    }
}

/// How a commitment ended.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Verdict {
    /// The satisfaction condition was met at the required strength.
    Fulfilled,
    /// Some required criteria passed and others did not.
    Partial,
    /// The work was attempted and did not succeed.
    Failed,
    /// A human called it off, or an escalation policy did.
    Abandoned,
    /// Replaced by a different commitment; see the lineage link.
    Superseded,
    /// Outlived its relevance window without closing.
    Expired,
    /// The runtime lost track of it — a crash mid-flight, a ledger gap, an
    /// external system that never answered.
    ///
    /// This variant is mandatory rather than a nicety. Without it the only way
    /// to tidy an untracked commitment is to assert an outcome nobody
    /// verified, which is exactly the dishonesty the satisfaction lattice
    /// exists to prevent. It mirrors `Settlement::Unknown` in the routing
    /// ledger.
    Unknown,
}

impl Verdict {
    pub fn as_str(self) -> &'static str {
        match self {
            Verdict::Fulfilled => "fulfilled",
            Verdict::Partial => "partial",
            Verdict::Failed => "failed",
            Verdict::Abandoned => "abandoned",
            Verdict::Superseded => "superseded",
            Verdict::Expired => "expired",
            Verdict::Unknown => "unknown",
        }
    }

    /// Whether this verdict claims the work actually got done. Only these
    /// are subject to the closure invariant.
    pub fn claims_success(self) -> bool {
        matches!(self, Verdict::Fulfilled)
    }
}

/// What an episode achieved.
///
/// The distinction between `Learned` and `Stalled` is the point of this type.
/// A naive "did a criterion move?" progress metric punishes the exploration
/// that hard problems require; a naive "did we spend tokens?" metric cannot
/// see a loop spinning. Separating them makes motion-without-progress
/// detectable without penalising genuine investigation.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "kebab-case")]
pub enum Advancement {
    /// A criterion moved, with the evidence that moved it.
    Advanced {
        criteria_moved: Vec<String>,
        #[serde(default)]
        evidence: Vec<EvidenceRef>,
    },
    /// Nothing moved, but uncertainty was reduced. Legitimate progress.
    Learned { fact: String },
    /// Needs something before it can continue.
    Blocked { blocker: String },
    /// Spent budget, moved nothing, learned nothing.
    Stalled { reason: String },
}

impl Advancement {
    pub fn as_str(&self) -> &'static str {
        match self {
            Advancement::Advanced { .. } => "advanced",
            Advancement::Learned { .. } => "learned",
            Advancement::Blocked { .. } => "blocked",
            Advancement::Stalled { .. } => "stalled",
        }
    }

    /// Whether this counts against the stall breaker.
    pub fn is_stall(&self) -> bool {
        matches!(self, Advancement::Stalled { .. })
    }
}

/// What a suspended commitment is waiting for.
///
/// Every variant maps onto a wake mechanism vak already has, which is why
/// suspension is a rewiring rather than new infrastructure: the inbox and
/// gateway approvals serve `Human`, the cron engine serves `Schedule`, the
/// script watchdog serves `Predicate` at zero token cost while the predicate
/// stays false, and the delivery outbox serves `External`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "kebab-case")]
pub enum Suspension {
    /// Waiting on a person.
    Human {
        question_id: String,
        question: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        addressed_to: Option<String>,
        /// What happens if nobody answers. Never optional: a deferred question
        /// with no timeout is how an agent accumulates abandoned work.
        escalation: vak_intent::Escalation,
    },
    /// Waiting for a time.
    Schedule {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        at: Option<chrono::DateTime<chrono::Utc>>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        cron: Option<String>,
    },
    /// Waiting for a machine-checkable condition to become true.
    Predicate { criterion: WorkCriterion },
    /// Waiting on another commitment.
    Commitment { commitment_id: String },
    /// Waiting on an external system.
    External {
        integration: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        operation_id: Option<String>,
    },
}

impl Suspension {
    pub fn as_str(&self) -> &'static str {
        match self {
            Suspension::Human { .. } => "human",
            Suspension::Schedule { .. } => "schedule",
            Suspension::Predicate { .. } => "predicate",
            Suspension::Commitment { .. } => "commitment",
            Suspension::External { .. } => "external",
        }
    }

    /// A one-line description for an inbox entry or a portfolio row.
    pub fn describe(&self) -> String {
        match self {
            Suspension::Human { question, .. } => format!("awaiting an answer: {question}"),
            Suspension::Schedule { at: Some(at), .. } => format!("scheduled for {at}"),
            Suspension::Schedule { cron: Some(c), .. } => format!("on schedule `{c}`"),
            Suspension::Schedule { .. } => "scheduled".into(),
            Suspension::Predicate { criterion } => {
                format!("waiting for: {}", criterion.statement)
            }
            Suspension::Commitment { commitment_id } => {
                format!("waiting on commitment {commitment_id}")
            }
            Suspension::External { integration, .. } => format!("waiting on {integration}"),
        }
    }
}

/// One session's worth of work on a commitment.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Episode {
    pub episode_id: String,
    pub session_id: String,
    /// The strand this episode works on.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub strand_id: Option<String>,
    pub started_at: chrono::DateTime<chrono::Utc>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ended_at: Option<chrono::DateTime<chrono::Utc>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub advancement: Option<Advancement>,
    #[serde(default)]
    pub spend_usd: f64,
}

/// The economics and relevance window of a long-lived commitment.
///
/// A lifetime budget rather than a per-run cap, because per-run caps are
/// exactly how an agent bleeds money across three months while every
/// individual run looks reasonable. Expiry produces an explicit
/// [`Verdict::Expired`] and never a silent deletion.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Economics {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub lifetime_budget_usd: Option<f64>,
    /// Stop considering this relevant after this instant.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub expires_at: Option<chrono::DateTime<chrono::Utc>>,
    /// How often the portfolio should surface this for human review.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub review_every_hours: Option<u32>,
    /// Consecutive stalls before the stall breaker trips.
    #[serde(default = "default_stall_limit")]
    pub stall_limit: u32,
}

fn default_stall_limit() -> u32 {
    3
}

/// Hand-written rather than derived. `#[serde(default = "…")]` only applies
/// when deserializing, so a derived `Default` would give `stall_limit: 0` and
/// every commitment constructed in Rust would be born already stalled.
impl Default for Economics {
    fn default() -> Self {
        Economics {
            lifetime_budget_usd: None,
            expires_at: None,
            review_every_hours: None,
            stall_limit: default_stall_limit(),
        }
    }
}

/// What a commitment is, as declared when it opens.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CommitmentSpec {
    /// One sentence a human would recognise months later.
    pub objective: String,
    /// The reading that opened it.
    pub reading: Reading,
    /// Criteria the runtime evaluates. `WorkCriterion::kind` says how.
    #[serde(default)]
    pub criteria: Vec<WorkCriterion>,
    /// The weakest evidence that may close this `Fulfilled`, taken from the
    /// reading's `evidence` axis.
    pub min_satisfaction: Satisfaction,
    #[serde(default)]
    pub economics: Economics,
    /// The workspace this belongs to.
    pub cwd: std::path::PathBuf,
    /// Which commitment this replaced, if any.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub supersedes: Option<String>,
    /// The conversation thread (strand lineage) this commitment serves, so
    /// a later turn that continues the thread attaches to it instead of
    /// opening a twin (docs/design/47-commitment-kernel.md, strands).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub thread_id: Option<String>,
    /// The conversation audience that asked for this work
    /// (docs/design/64-agent-owned-platform.md). An Agent serving several
    /// chats owes each of them its own obligations, and a portfolio read
    /// from one chat must not reveal another's.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub audience_id: Option<String>,
}

/// One criterion's standing.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CriterionState {
    pub criterion_id: String,
    pub statement: String,
    pub required: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub result: Option<CriterionResult>,
    /// How strongly the result is backed. A `Semantic` criterion the model
    /// asserted is `Asserted`; a `Shell` criterion the runtime ran is
    /// `Observed`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub strength: Option<Satisfaction>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub evaluated_at: Option<chrono::DateTime<chrono::Utc>>,
}

impl CriterionState {
    pub fn passed(&self) -> bool {
        matches!(self.result, Some(CriterionResult::Passed { .. }))
    }
}

/// How a commitment closed.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Closure {
    pub verdict: Verdict,
    /// The weakest strength among the criteria that justified this closure.
    pub strength: Satisfaction,
    pub closed_at: chrono::DateTime<chrono::Utc>,
    #[serde(default)]
    pub evidence: Vec<EvidenceRef>,
    pub note: String,
}

/// The projected current state of a commitment.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Commitment {
    pub commitment_id: String,
    pub opened_at: chrono::DateTime<chrono::Utc>,
    pub spec: CommitmentSpec,
    pub phase: Phase,
    #[serde(default)]
    pub criteria: Vec<CriterionState>,
    #[serde(default)]
    pub episodes: Vec<Episode>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub suspension: Option<Suspension>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub blocker: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub envelope: Option<Envelope>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub closure: Option<Closure>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub superseded_by: Option<String>,
    /// Total spend across every episode.
    #[serde(default)]
    pub spend_usd: f64,
    /// Consecutive stalled episodes. Reset by any other advancement.
    #[serde(default)]
    pub consecutive_stalls: u32,
    /// Drift recorded at the last re-admission, if any.
    #[serde(default)]
    pub drift: Vec<String>,
    pub updated_at: chrono::DateTime<chrono::Utc>,
}

impl Commitment {
    /// Required criteria that have not yet passed.
    pub fn outstanding(&self) -> Vec<&CriterionState> {
        self.criteria
            .iter()
            .filter(|criterion| criterion.required && !criterion.passed())
            .collect()
    }

    /// The weakest strength among criteria that have passed, or `Asserted`
    /// when there are no criteria at all.
    ///
    /// Weakest rather than strongest: a closure is only as well-evidenced as
    /// its flimsiest supporting criterion, and reporting the best one would
    /// let a single shell check launder four semantic assertions.
    pub fn achieved_strength(&self) -> Satisfaction {
        self.criteria
            .iter()
            .filter(|criterion| criterion.required)
            .map(|criterion| criterion.strength.unwrap_or(Satisfaction::Asserted))
            .min_by_key(|strength| strength.rank())
            .unwrap_or(Satisfaction::Asserted)
    }

    /// Whether the closure invariant permits closing with `verdict`.
    ///
    /// Only success claims are constrained: recording a failure, an
    /// abandonment, or an honest `Unknown` must always be possible, or the
    /// ledger would be unable to tell the truth about work that went wrong.
    pub fn may_close(&self, verdict: Verdict) -> Result<Satisfaction, ClosureRefusal> {
        if !verdict.claims_success() {
            return Ok(self.achieved_strength());
        }
        let outstanding = self.outstanding();
        if !outstanding.is_empty() {
            return Err(ClosureRefusal::CriteriaOutstanding {
                ids: outstanding
                    .iter()
                    .map(|criterion| criterion.criterion_id.clone())
                    .collect(),
            });
        }
        let achieved = self.achieved_strength();
        if !achieved.satisfies(self.spec.min_satisfaction) {
            return Err(ClosureRefusal::InsufficientEvidence {
                required: self.spec.min_satisfaction,
                achieved,
            });
        }
        Ok(achieved)
    }

    /// Whether the stall breaker has tripped.
    pub fn is_stalled(&self) -> bool {
        self.consecutive_stalls >= self.spec.economics.stall_limit
    }

    /// Whether the lifetime budget is exhausted.
    pub fn is_over_budget(&self) -> bool {
        self.spec
            .economics
            .lifetime_budget_usd
            .is_some_and(|cap| self.spend_usd >= cap)
    }

    /// Whether the relevance window has passed.
    pub fn is_expired(&self, now: chrono::DateTime<chrono::Utc>) -> bool {
        self.spec
            .economics
            .expires_at
            .is_some_and(|expiry| now >= expiry)
    }

    /// One-line portfolio summary.
    pub fn summary(&self) -> String {
        let progress = format!(
            "{}/{} criteria",
            self.criteria.iter().filter(|c| c.passed()).count(),
            self.criteria.len()
        );
        match (&self.closure, &self.suspension) {
            (Some(closure), _) => format!(
                "{} — {} ({}, {progress})",
                self.spec.objective,
                closure.verdict.as_str(),
                closure.strength.as_str()
            ),
            (None, Some(suspension)) => {
                format!("{} — {}", self.spec.objective, suspension.describe())
            }
            (None, None) => format!(
                "{} — {} ({progress})",
                self.spec.objective,
                self.phase.as_str()
            ),
        }
    }
}

/// Why a closure was refused.
#[derive(Debug, Clone, PartialEq, thiserror::Error)]
pub enum ClosureRefusal {
    #[error(
        "cannot close fulfilled: {} required criteria have not passed ({})",
        ids.len(),
        ids.join(", ")
    )]
    CriteriaOutstanding { ids: Vec<String> },
    #[error(
        "cannot close fulfilled: this work requires {} evidence but only {} was achieved",
        required.as_str(),
        achieved.as_str()
    )]
    InsufficientEvidence {
        required: Satisfaction,
        achieved: Satisfaction,
    },
}
