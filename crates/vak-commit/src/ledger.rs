//! The append-only commitment ledger and its projection.
//!
//! Events are never rewritten; the current [`Commitment`] is always a fold
//! over them, exactly as `vak-session`'s work projector folds `WorkEvent`s.
//! That is what makes "what did this agent commit to, under whose authority,
//! and what closed it" answerable months later rather than a matter of trust.
//!
//! One rule is enforced *at append time* rather than at read time: a closure
//! claiming success is refused unless the evidence actually supports it. A
//! ledger that can record a lie is not an audit trail.

use std::io::Write;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use vak_intent::{Envelope, Reading, Satisfaction};
use vak_session::types::{CriterionResult, EvidenceRef, WorkCriterion};

use crate::types::{
    Advancement, Closure, ClosureRefusal, Commitment, CommitmentSpec, CriterionState, Economics,
    Episode, Phase, Suspension, Verdict,
};

/// One thing that happened to a commitment.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "kebab-case")]
pub enum EventKind {
    Opened {
        spec: Box<CommitmentSpec>,
    },
    /// Resumed after a gap; the engagement was re-resolved against current
    /// reality and this is what had changed underneath it.
    Readmitted {
        #[serde(default)]
        drift: Vec<String>,
        reading: Box<Reading>,
    },
    EpisodeStarted {
        episode_id: String,
        session_id: String,
    },
    EpisodeEnded {
        episode_id: String,
        advancement: Advancement,
        #[serde(default)]
        spend_usd: f64,
    },
    Suspended {
        suspension: Suspension,
    },
    Resumed {
        reason: String,
    },
    Blocked {
        blocker: String,
    },
    Unblocked {
        reason: String,
    },
    /// A criterion was evaluated. `strength` records *how* it was established,
    /// which is what the closure invariant later checks.
    CriterionEvaluated {
        criterion_id: String,
        result: CriterionResult,
        strength: Satisfaction,
    },
    EnvelopeGranted {
        envelope: Box<Envelope>,
    },
    EnvelopeRevoked {
        envelope_id: String,
        by: String,
    },
    QuestionAnswered {
        question_id: String,
        answer: String,
    },
    Superseded {
        by: String,
        reason: String,
    },
    Closed {
        verdict: Verdict,
        strength: Satisfaction,
        #[serde(default)]
        evidence: Vec<EvidenceRef>,
        note: String,
    },
}

impl EventKind {
    pub fn as_str(&self) -> &'static str {
        match self {
            EventKind::Opened { .. } => "opened",
            EventKind::Readmitted { .. } => "readmitted",
            EventKind::EpisodeStarted { .. } => "episode-started",
            EventKind::EpisodeEnded { .. } => "episode-ended",
            EventKind::Suspended { .. } => "suspended",
            EventKind::Resumed { .. } => "resumed",
            EventKind::Blocked { .. } => "blocked",
            EventKind::Unblocked { .. } => "unblocked",
            EventKind::CriterionEvaluated { .. } => "criterion-evaluated",
            EventKind::EnvelopeGranted { .. } => "envelope-granted",
            EventKind::EnvelopeRevoked { .. } => "envelope-revoked",
            EventKind::QuestionAnswered { .. } => "question-answered",
            EventKind::Superseded { .. } => "superseded",
            EventKind::Closed { .. } => "closed",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Event {
    pub event_id: String,
    pub commitment_id: String,
    pub ts: chrono::DateTime<chrono::Utc>,
    #[serde(flatten)]
    pub kind: EventKind,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub trace: Option<vak_session::trace::TraceKey>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub actor: Option<vak_session::ids::PrincipalId>,
}

vak_session::impl_traced!(Event, "commitment_event");

impl Event {
    pub fn new(commitment_id: impl Into<String>, kind: EventKind) -> Self {
        Event {
            event_id: uuid::Uuid::now_v7().to_string(),
            commitment_id: commitment_id.into(),
            ts: chrono::Utc::now(),
            kind,
            trace: None,
            actor: None,
        }
    }
}

#[derive(Debug, thiserror::Error)]
pub enum LedgerError {
    #[error("io error: {0}")]
    Io(#[from] std::io::Error),
    #[error("serialization error: {0}")]
    Serde(#[from] serde_json::Error),
    #[error("unknown commitment '{0}'")]
    UnknownCommitment(String),
    #[error("commitment '{0}' is already closed")]
    AlreadyClosed(String),
    #[error(transparent)]
    Closure(#[from] ClosureRefusal),
}

/// Held while one check-and-append runs. The OS releases the lock when the
/// file closes — on drop, and when a process dies holding it — so there is
/// no stale lock to detect and none to break.
struct LedgerLock {
    _file: std::fs::File,
}

/// Append-only JSONL of commitment events, one file per home.
///
/// Sits beside `routing-evidence.jsonl` in the sessions home for the same
/// reason: it is runtime evidence about the agent's own behaviour, not
/// conversation content, and it must survive any individual session.
pub struct CommitmentLedger {
    path: PathBuf,
    trace: Option<vak_session::trace::TraceKey>,
}

impl CommitmentLedger {
    pub fn new(sessions_home: &Path) -> Self {
        CommitmentLedger {
            path: sessions_home.join("commitments.jsonl"),
            trace: None,
        }
    }

    /// Stamp the run's key (and its actor) onto every event this handle
    /// appends that does not already carry one.
    pub fn with_trace(mut self, trace: Option<&vak_session::trace::TraceKey>) -> Self {
        self.trace = trace.cloned();
        self
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    /// Append one event, after checking it is legal against current state.
    ///
    /// The check happens here rather than in a caller because there are many
    /// callers and exactly one ledger: a rule enforced at the write boundary
    /// cannot be bypassed by a surface that forgot about it. The check and
    /// the append happen under an exclusive lock, so two writers — a running
    /// turn and `vak commit close`, or the upkeep tick — cannot both read
    /// "not closed" and both close it.
    ///
    /// A closure's strength is the runtime's to state, not the caller's: it
    /// is recomputed here from the criteria as they stand, so a `Closed`
    /// event can never record stronger evidence than the commitment holds.
    pub fn append(&self, event: &Event) -> Result<(), LedgerError> {
        let _guard = self.lock()?;
        let current = self
            .get(&event.commitment_id)?
            .ok_or_else(|| LedgerError::UnknownCommitment(event.commitment_id.clone()))?;
        if current.phase.is_terminal() {
            return Err(LedgerError::AlreadyClosed(event.commitment_id.clone()));
        }
        if let EventKind::Closed { verdict, .. } = &event.kind {
            // The closure invariant, as a hard error rather than a lint.
            let achieved = current.may_close(*verdict)?;
            let mut event = event.clone();
            if let EventKind::Closed { strength, .. } = &mut event.kind {
                *strength = achieved;
            }
            return self.append_unchecked(&event);
        }
        self.append_unchecked(event)
    }

    /// A cross-process lock on the ledger, held for one check-and-append.
    /// The lock file itself is never removed: deleting it while another
    /// writer waits on it would hand the two of them different files.
    fn lock(&self) -> Result<LedgerLock, LedgerError> {
        let path = self.path.with_extension("lock");
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let file = std::fs::OpenOptions::new()
            .create(true)
            .truncate(false)
            .write(true)
            .open(&path)?;
        file.lock()?;
        Ok(LedgerLock { _file: file })
    }

    /// Append without the closure check. Used by the projector's own tests and
    /// by recovery tooling that is deliberately reconstructing history.
    fn append_unchecked(&self, event: &Event) -> Result<(), LedgerError> {
        if let Some(parent) = self.path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let line = match (&self.trace, &event.trace) {
            (Some(trace), None) => {
                let mut stamped = event.clone();
                stamped.actor = stamped.actor.or(trace.actor);
                stamped.trace = Some(trace.clone());
                serde_json::to_string(&stamped)?
            }
            _ => serde_json::to_string(event)?,
        };
        let mut file = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&self.path)?;
        writeln!(file, "{line}")?;
        Ok(())
    }

    /// Every event, oldest first. A corrupt line — torn, not UTF-8, or not
    /// an event — is skipped rather than trusted, and never ends the read:
    /// a malformed row must not become a state change, and must not hide
    /// every row written after it either.
    pub fn events(&self) -> Vec<Event> {
        let Ok(bytes) = std::fs::read(&self.path) else {
            return Vec::new();
        };
        bytes
            .split(|byte| *byte == b'\n')
            .filter_map(|line| serde_json::from_slice::<Event>(line).ok())
            .collect()
    }

    /// Events for one commitment.
    pub fn events_for(&self, commitment_id: &str) -> Vec<Event> {
        self.events()
            .into_iter()
            .filter(|event| event.commitment_id == commitment_id)
            .collect()
    }

    /// Project one commitment's current state.
    pub fn get(&self, commitment_id: &str) -> Result<Option<Commitment>, LedgerError> {
        let events = self.events_for(commitment_id);
        Ok(project(&events))
    }

    /// Every commitment, most recently updated first.
    pub fn all(&self) -> Vec<Commitment> {
        let mut by_id: std::collections::BTreeMap<String, Vec<Event>> =
            std::collections::BTreeMap::new();
        for event in self.events() {
            by_id
                .entry(event.commitment_id.clone())
                .or_default()
                .push(event);
        }
        let mut out: Vec<Commitment> = by_id
            .values()
            .filter_map(|events| project(events))
            .collect();
        out.sort_by_key(|commitment| std::cmp::Reverse(commitment.updated_at));
        out
    }

    /// Commitments the portfolio scheduler could act on now.
    pub fn open(&self) -> Vec<Commitment> {
        self.all()
            .into_iter()
            .filter(|commitment| !commitment.phase.is_terminal())
            .collect()
    }

    /// Open a new commitment and return its id.
    pub fn open_commitment(&self, spec: CommitmentSpec) -> Result<String, LedgerError> {
        let commitment_id = uuid::Uuid::now_v7().to_string();
        self.append_unchecked(&Event::new(
            commitment_id.clone(),
            EventKind::Opened {
                spec: Box::new(spec),
            },
        ))?;
        Ok(commitment_id)
    }
}

/// Fold events into current state.
///
/// Returns `None` when the first event is not an `Opened`, which is how a
/// truncated or partially-corrupt ledger declines to invent a commitment
/// rather than projecting a plausible-looking fiction.
pub fn project(events: &[Event]) -> Option<Commitment> {
    let first = events.first()?;
    let EventKind::Opened { spec } = &first.kind else {
        return None;
    };
    let spec = (**spec).clone();
    let criteria = spec
        .criteria
        .iter()
        .map(|criterion| CriterionState {
            criterion_id: criterion.criterion_id.clone(),
            statement: criterion.statement.clone(),
            required: criterion.required,
            result: None,
            strength: None,
            evaluated_at: None,
        })
        .collect();

    let mut commitment = Commitment {
        commitment_id: first.commitment_id.clone(),
        opened_at: first.ts,
        spec,
        phase: Phase::Active,
        criteria,
        episodes: Vec::new(),
        suspension: None,
        blocker: None,
        envelope: None,
        closure: None,
        superseded_by: None,
        spend_usd: 0.0,
        consecutive_stalls: 0,
        drift: Vec::new(),
        updated_at: first.ts,
    };

    for event in events.iter().skip(1) {
        commitment.updated_at = event.ts;
        match &event.kind {
            EventKind::Opened { .. } => {
                // A second open for the same id is a ledger fault, not a
                // reset. Ignore it rather than losing the accumulated history.
            }
            EventKind::Readmitted { drift, reading } => {
                commitment.drift = drift.clone();
                commitment.spec.reading = (**reading).clone();
            }
            EventKind::EpisodeStarted {
                episode_id,
                session_id,
            } => {
                // A new episode is someone working it again: whatever
                // blocked or suspended the last one is no longer the state.
                commitment.phase = Phase::Active;
                commitment.suspension = None;
                commitment.blocker = None;
                commitment.episodes.push(Episode {
                    episode_id: episode_id.clone(),
                    session_id: session_id.clone(),
                    started_at: event.ts,
                    ended_at: None,
                    advancement: None,
                    spend_usd: 0.0,
                });
            }
            EventKind::EpisodeEnded {
                episode_id,
                advancement,
                spend_usd,
            } => {
                if let Some(episode) = commitment
                    .episodes
                    .iter_mut()
                    .find(|episode| episode.episode_id == *episode_id)
                {
                    episode.ended_at = Some(event.ts);
                    episode.advancement = Some(advancement.clone());
                    episode.spend_usd = *spend_usd;
                }
                commitment.spend_usd += spend_usd;
                if advancement.is_stall() {
                    commitment.consecutive_stalls += 1;
                } else {
                    // Any real advancement — including `Learned` — clears the
                    // streak. Exploration is not stalling.
                    commitment.consecutive_stalls = 0;
                }
                if let Advancement::Blocked { blocker } = advancement {
                    commitment.phase = Phase::Blocked;
                    commitment.blocker = Some(blocker.clone());
                }
            }
            EventKind::Suspended { suspension } => {
                commitment.phase = Phase::Suspended;
                commitment.suspension = Some(suspension.clone());
            }
            EventKind::Resumed { .. } => {
                commitment.phase = Phase::Active;
                commitment.suspension = None;
            }
            EventKind::Blocked { blocker } => {
                commitment.phase = Phase::Blocked;
                commitment.blocker = Some(blocker.clone());
            }
            EventKind::Unblocked { .. } => {
                commitment.phase = Phase::Active;
                commitment.blocker = None;
            }
            EventKind::CriterionEvaluated {
                criterion_id,
                result,
                strength,
            } => {
                if let Some(criterion) = commitment
                    .criteria
                    .iter_mut()
                    .find(|criterion| criterion.criterion_id == *criterion_id)
                {
                    criterion.result = Some(result.clone());
                    criterion.strength = Some(*strength);
                    criterion.evaluated_at = Some(event.ts);
                }
                if commitment.phase == Phase::Active {
                    commitment.phase = Phase::Satisfying;
                }
            }
            EventKind::EnvelopeGranted { envelope } => {
                commitment.envelope = Some((**envelope).clone());
            }
            EventKind::EnvelopeRevoked { envelope_id, .. } => {
                if let Some(envelope) = commitment.envelope.as_mut()
                    && envelope.envelope_id == *envelope_id
                {
                    envelope.revoked_at = Some(event.ts);
                }
            }
            EventKind::QuestionAnswered { question_id, .. } => {
                let answered = matches!(
                    &commitment.suspension,
                    Some(Suspension::Human { question_id: id, .. }) if id == question_id
                );
                if answered {
                    commitment.phase = Phase::Active;
                    commitment.suspension = None;
                }
            }
            EventKind::Superseded { by, reason } => {
                commitment.superseded_by = Some(by.clone());
                commitment.phase = Phase::Closed;
                commitment.closure = Some(Closure {
                    verdict: Verdict::Superseded,
                    strength: commitment.achieved_strength(),
                    closed_at: event.ts,
                    evidence: Vec::new(),
                    note: reason.clone(),
                });
            }
            EventKind::Closed {
                verdict,
                strength,
                evidence,
                note,
            } => {
                commitment.phase = Phase::Closed;
                commitment.closure = Some(Closure {
                    verdict: *verdict,
                    strength: *strength,
                    closed_at: event.ts,
                    evidence: evidence.clone(),
                    note: note.clone(),
                });
            }
        }
    }
    Some(commitment)
}

/// Build a spec from a resolved reading.
///
/// `criteria` come from whatever proposed them — the planner, a flow, or a
/// human. The model may propose criteria; it never marks them passed.
pub fn spec_from_reading(
    objective: impl Into<String>,
    reading: Reading,
    criteria: Vec<WorkCriterion>,
    cwd: std::path::PathBuf,
    economics: Economics,
) -> CommitmentSpec {
    let min_satisfaction = reading.evidence.min_satisfaction();
    CommitmentSpec {
        objective: objective.into(),
        reading,
        criteria,
        min_satisfaction,
        economics,
        cwd,
        supersedes: None,
        thread_id: None,
        audience_id: None,
    }
}
