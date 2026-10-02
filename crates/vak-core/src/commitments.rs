//! Wiring durable commitments into the run loop.
//!
//! `vak-commit` owns the lifecycle, the satisfaction lattice and the ledger;
//! it knows nothing about sessions, providers or the agent loop. This module
//! is the seam that opens a commitment when a turn's reading calls for one,
//! records the episode that advanced it, and evaluates its criteria against
//! the world.
//!
//! # Why episodes are recorded here rather than in the agent
//!
//! An episode's outcome is a judgement about *the commitment*, not about the
//! turn: a turn that ended cleanly may still have moved nothing. Only the
//! caller that holds both the commitment and the turn's result can say which
//! it was, so the classification lives at this seam and `vak-agent` stays
//! unaware that commitments exist at all.

use std::path::Path;

use vak_commit::CriterionEvaluator;
use vak_commit::{
    Advancement, CommitmentLedger, CommitmentSpec, Economics, Evaluation, Event, EventKind, Verdict,
};
use vak_intent::{Intent, Satisfaction};
use vak_session::types::{CriterionKind, CriterionResult, WorkCriterion};

/// A commitment this turn is serving, and the episode it opened on it.
#[derive(Debug, Clone)]
pub struct EpisodeHandle {
    pub commitment_id: String,
    pub episode_id: String,
    /// The strand this episode serves.
    pub strand_id: String,
}

/// Economics from configuration, resolved once.
pub fn economics(config: &vak_config::Config) -> Economics {
    Economics {
        lifetime_budget_usd: config.commitment.lifetime_budget_usd,
        expires_at: config
            .commitment
            .default_ttl_days
            .map(|days| chrono::Utc::now() + chrono::Duration::days(i64::from(days))),
        review_every_hours: config.commitment.review_every_hours,
        stall_limit: config.commitment.stall_limit,
    }
}

/// Criteria proposed from the reading alone.
///
/// Deliberately thin. The runtime can only propose what it can also *check*,
/// and from a bare request it can check almost nothing — so this yields at
/// most a semantic placeholder and leaves the real criteria to the planner, a
/// flow, or a human. Inventing a `Shell` criterion by guessing at a test
/// command would manufacture `Observed` evidence out of a guess, which is
/// precisely the laundering the satisfaction lattice exists to prevent.
pub fn seed_criteria(intent: &Intent, objective: &str) -> Vec<WorkCriterion> {
    if intent.reading.evidence.min_satisfaction().rank() < Satisfaction::Cited.rank() {
        return Vec::new();
    }
    vec![WorkCriterion {
        criterion_id: "objective".into(),
        statement: objective.to_string(),
        // Semantic, so it carries `Asserted` strength and therefore cannot by
        // itself close work held to `Verified` or `Audited`. That is the
        // intended outcome: the commitment stays open, visibly, until someone
        // supplies a criterion the runtime can actually check.
        kind: CriterionKind::Semantic,
        required: true,
    }]
}

/// One line a human would recognise months later.
pub fn objective_from(prompt: &str) -> String {
    let first = prompt.trim().lines().next().unwrap_or("").trim();
    let mut out: String = first.chars().take(120).collect();
    if first.chars().count() > 120 {
        out.push('…');
    }
    if out.is_empty() {
        "untitled work".into()
    } else {
        out
    }
}

/// What a turn will do with the commitment ledger, decided before it writes
/// anything: which strands work on a commitment that already exists, and
/// which open a new one.
#[derive(Debug, Clone, Default)]
pub struct EpisodePlan {
    pub strands: Vec<PlannedStrand>,
}

#[derive(Debug, Clone)]
pub struct PlannedStrand {
    pub strand_id: String,
    pub target: PlannedTarget,
}

#[derive(Debug, Clone)]
pub enum PlannedTarget {
    /// Continue the open commitment on this strand's thread.
    Existing {
        commitment_id: String,
        /// Its envelope, when one is live: what the human pre-authorized
        /// for this work.
        envelope: Option<vak_intent::Envelope>,
    },
    /// Open a new commitment, superseding the one the strand replaces.
    Open { supersedes: Option<String> },
}

impl EpisodePlan {
    /// Live envelopes by strand id, for `vak_intent::apply_envelopes`. Only
    /// an existing commitment can carry one: a grant is given to work that
    /// already exists, never to work this turn is about to open.
    pub fn envelopes(&self) -> std::collections::BTreeMap<String, vak_intent::Envelope> {
        self.strands
            .iter()
            .filter_map(|planned| match &planned.target {
                PlannedTarget::Existing {
                    envelope: Some(envelope),
                    ..
                } => Some((planned.strand_id.clone(), envelope.clone())),
                _ => None,
            })
            .collect()
    }

    /// The commitments whose live envelopes a gated action may fall inside.
    pub fn enveloped_commitments(&self) -> Vec<String> {
        self.strands
            .iter()
            .filter_map(|planned| match &planned.target {
                PlannedTarget::Existing {
                    commitment_id,
                    envelope: Some(_),
                } => Some(commitment_id.clone()),
                _ => None,
            })
            .collect()
    }
}

/// Decide, without writing, what each strand of this turn does with the
/// commitment ledger.
///
/// One commitment per *thread*, not per turn:
/// - a strand on a thread that already has an open commitment works on it,
///   whatever its own horizon reads as — "now also check staging" is part of
///   the weekly job it continues;
/// - a strand that replaces a thread with an open commitment opens the
///   successor and supersedes it — `/goal replace` on durable work yields
///   durable work;
/// - any other strand opens a commitment only when it is itself durable and
///   its reading is confident enough to carry an obligation.
///
/// Empty when commitments are disabled or nothing here is durable.
pub fn plan_episodes(
    sessions_home: &Path,
    config: &vak_config::Config,
    intent: &Intent,
    now: chrono::DateTime<chrono::Utc>,
) -> EpisodePlan {
    if !config.commitment.enabled || intent.strands.is_empty() {
        return EpisodePlan::default();
    }
    let open = CommitmentLedger::new(sessions_home).open();
    let on_thread = |thread: &str| {
        open.iter()
            .find(|commitment| commitment.spec.thread_id.as_deref() == Some(thread))
    };
    let mut plan = EpisodePlan::default();
    for strand in &intent.strands {
        let durable = strand.engagement.posture.open_commitment
            && strand
                .reading
                .may_open_commitment(config.intent.accept_confidence);
        let target = if let Some(existing) = on_thread(&strand.thread_id) {
            PlannedTarget::Existing {
                commitment_id: existing.commitment_id.clone(),
                envelope: existing
                    .envelope
                    .clone()
                    .filter(|envelope| envelope.is_live(now)),
            }
        } else if let Some(replaced) = strand.lineage.replaced_thread().and_then(on_thread) {
            PlannedTarget::Open {
                supersedes: Some(replaced.commitment_id.clone()),
            }
        } else if durable {
            PlannedTarget::Open { supersedes: None }
        } else {
            continue;
        };
        plan.strands.push(PlannedStrand {
            strand_id: strand.strand_id.clone(),
            target,
        });
    }
    plan
}

/// Carry out an [`EpisodePlan`]: open what it opens, supersede what it
/// replaces, and start one episode per planned strand.
///
/// A ledger failure costs the audit row, never the turn: the strand is
/// skipped and logged. Episodes come back in strand order; the first is the
/// turn's primary.
#[allow(clippy::too_many_arguments)]
pub fn begin_episodes(
    sessions_home: &Path,
    config: &vak_config::Config,
    intent: &Intent,
    plan: &EpisodePlan,
    prompt: &str,
    session_id: &str,
    cwd: &Path,
    audience_id: Option<&str>,
    trace: Option<&vak_session::trace::TraceKey>,
) -> Vec<EpisodeHandle> {
    let ledger = CommitmentLedger::new(sessions_home).with_trace(trace);
    let mut handles = Vec::new();
    for planned in &plan.strands {
        let Some(strand) = intent
            .strands
            .iter()
            .find(|strand| strand.strand_id == planned.strand_id)
        else {
            continue;
        };
        let commitment_id = match &planned.target {
            PlannedTarget::Existing { commitment_id, .. } => commitment_id.clone(),
            PlannedTarget::Open { supersedes } => {
                let Some(new_id) = open_for_strand(
                    &ledger,
                    config,
                    strand,
                    prompt,
                    cwd,
                    supersedes.clone(),
                    audience_id,
                ) else {
                    continue;
                };
                if let Some(old) = supersedes {
                    let _ = ledger.append(&Event::new(
                        old,
                        EventKind::Superseded {
                            by: new_id.clone(),
                            reason: "replaced by an explicit /goal replace".into(),
                        },
                    ));
                }
                new_id
            }
        };
        close_orphan_episode(&ledger, &commitment_id, session_id);
        let episode_id = uuid::Uuid::now_v7().to_string();
        if let Err(error) = ledger.append(&Event::new(
            &commitment_id,
            EventKind::EpisodeStarted {
                episode_id: episode_id.clone(),
                session_id: session_id.to_string(),
            },
        )) {
            eprintln!("[commit] could not start an episode: {error}");
            continue;
        }
        handles.push(EpisodeHandle {
            commitment_id,
            episode_id,
            strand_id: strand.strand_id.clone(),
        });
    }
    handles
}

/// Open a commitment for one strand.
fn open_for_strand(
    ledger: &CommitmentLedger,
    config: &vak_config::Config,
    strand: &vak_intent::Strand,
    prompt: &str,
    cwd: &Path,
    supersedes: Option<String>,
    audience_id: Option<&str>,
) -> Option<String> {
    let objective = objective_from(if strand.text.is_empty() {
        prompt
    } else {
        &strand.text
    });
    let seed = Intent {
        reading: strand.reading.clone(),
        strands: Vec::new(),
        engagement: strand.engagement.clone(),
        provenance: vak_intent::Provenance::new(
            vak_intent::Tier::Signals,
            vak_intent::RESOLVER_VERSION,
            Vec::new(),
        ),
    };
    let spec = CommitmentSpec {
        criteria: seed_criteria(&seed, &objective),
        min_satisfaction: strand.reading.evidence.min_satisfaction(),
        economics: economics(config),
        cwd: cwd.to_path_buf(),
        supersedes,
        thread_id: Some(strand.thread_id.clone()),
        audience_id: audience_id.map(str::to_string),
        objective,
        reading: strand.reading.clone(),
    };
    match ledger.open_commitment(spec) {
        Ok(id) => Some(id),
        Err(error) => {
            eprintln!("[commit] could not open a commitment: {error}");
            None
        }
    }
}

/// A resumed session may have crashed after EpisodeStarted but before
/// EpisodeEnded. Close that exact orphan as blocked before opening the new
/// episode; never replay its effects implicitly.
fn close_orphan_episode(ledger: &CommitmentLedger, commitment_id: &str, session_id: &str) {
    if let Ok(Some(commitment)) = ledger.get(commitment_id)
        && let Some(episode) = commitment
            .episodes
            .iter()
            .rev()
            .find(|episode| episode.ended_at.is_none() && episode.session_id == session_id)
    {
        let _ = ledger.append(&Event::new(
            commitment_id,
            EventKind::EpisodeEnded {
                episode_id: episode.episode_id.clone(),
                advancement: Advancement::Blocked {
                    blocker: "recovered after an interrupted process; review before retry".into(),
                },
                spend_usd: 0.0,
            },
        ));
    }
}

/// The commitment ledger rendered for the model: `ContextProfile::Full`.
///
/// One block per commitment this turn serves — objective, criteria and
/// their standing, an open question if the work is suspended on one. Terse
/// and factual, like the intent note it is appended to; the model reads the
/// state of its obligations instead of reconstructing them from history.
pub fn prompt_projection(sessions_home: &Path, episodes: &[EpisodeHandle]) -> Option<String> {
    if episodes.is_empty() {
        return None;
    }
    let ledger = CommitmentLedger::new(sessions_home);
    let mut lines = Vec::new();
    for episode in episodes {
        let Ok(Some(commitment)) = ledger.get(&episode.commitment_id) else {
            continue;
        };
        let met = commitment
            .criteria
            .iter()
            .filter(|c| {
                matches!(
                    c.result,
                    Some(vak_session::types::CriterionResult::Passed { .. })
                )
            })
            .count();
        lines.push(format!(
            "Commitment {} ({}): {} — {} of {} criteria met, {} episode(s) so far, closes at {} evidence.",
            commitment.commitment_id,
            commitment.phase.as_str(),
            commitment.spec.objective,
            met,
            commitment.criteria.len(),
            commitment.episodes.len(),
            commitment.spec.min_satisfaction.as_str()
        ));
        for criterion in &commitment.criteria {
            let result = match &criterion.result {
                Some(vak_session::types::CriterionResult::Passed { .. }) => "passed",
                Some(vak_session::types::CriterionResult::Failed { .. }) => "failed",
                Some(vak_session::types::CriterionResult::Unknown { .. }) => "unknown",
                None => "not yet evaluated",
            };
            let standing = match criterion.strength {
                Some(strength) if criterion.result.is_some() => {
                    format!("{result} ({})", strength.as_str())
                }
                _ => result.to_string(),
            };
            lines.push(format!("  - {}: {standing}", criterion.statement));
        }
        if let Some(vak_commit::Suspension::Human { question, .. }) = &commitment.suspension {
            lines.push(format!("  open question: {question}"));
        }
        if let Some(blocker) = &commitment.blocker {
            lines.push(format!("  blocked: {blocker}"));
        }
    }
    (!lines.is_empty()).then(|| lines.join("\n"))
}

/// What a finished turn did for its commitment.
///
/// The distinction that matters is `Learned` versus `Stalled`. A turn that
/// produced a substantive answer but moved no criterion has still reduced
/// uncertainty and must not count against the stall breaker; a turn that
/// produced nothing has.
pub fn classify(
    outcome: &vak_agent::TurnOutcome,
    tool_calls: usize,
    criteria_moved: Vec<String>,
) -> Advancement {
    if !criteria_moved.is_empty() {
        return Advancement::Advanced {
            criteria_moved,
            evidence: Vec::new(),
        };
    }
    match outcome {
        vak_agent::TurnOutcome::Completed { response } => {
            let said_something = response
                .content
                .iter()
                .any(|block| matches!(block, vak_llm::ContentBlock::Text { text } if text.trim().len() > 40));
            // Tool activity alone is not progress: retries, repeated reads,
            // and failed probes are common in a long run. Only a substantive
            // response or an explicitly moved criterion clears the stall
            // streak.
            if said_something {
                Advancement::Learned {
                    fact: format!(
                        "episode completed with {tool_calls} tool call(s) and no criterion movement"
                    ),
                }
            } else {
                Advancement::Stalled {
                    reason: "episode produced neither output nor effect".into(),
                }
            }
        }
        vak_agent::TurnOutcome::Aborted { .. } => Advancement::Blocked {
            blocker: "cancelled".into(),
        },
        vak_agent::TurnOutcome::Failed { error } => Advancement::Blocked {
            blocker: error.to_string(),
        },
        // Hitting the turn ceiling is the textbook motion-without-progress
        // case, and the one the stall breaker exists to catch.
        vak_agent::TurnOutcome::MaxTurnsReached => Advancement::Stalled {
            reason: "exhausted the turn budget without moving a criterion".into(),
        },
    }
}

/// Close out this turn's episode.
pub fn end_episode(
    sessions_home: &Path,
    handle: &EpisodeHandle,
    advancement: Advancement,
    spend_usd: f64,
    trace: Option<&vak_session::trace::TraceKey>,
) {
    let ledger = CommitmentLedger::new(sessions_home).with_trace(trace);
    if let Err(error) = ledger.append(&Event::new(
        &handle.commitment_id,
        EventKind::EpisodeEnded {
            episode_id: handle.episode_id.clone(),
            advancement,
            spend_usd,
        },
    )) {
        eprintln!("[commit] could not close the episode: {error}");
    }
}

/// Suspend a commitment on a question nobody here can answer.
///
/// This is the `Defer` half of the human-in-the-loop contract: an unattended
/// surface used to fail closed unconditionally, which is right for a one-shot
/// turn and destroys month-long work that merely needed to wait.
pub fn defer_for_human(
    sessions_home: &Path,
    commitment_id: &str,
    question: &str,
    addressed_to: Option<String>,
    escalation: vak_intent::Escalation,
) -> Result<String, vak_commit::LedgerError> {
    let question_id = uuid::Uuid::now_v7().to_string();
    CommitmentLedger::new(sessions_home).append(&Event::new(
        commitment_id,
        EventKind::Suspended {
            suspension: vak_commit::Suspension::Human {
                question_id: question_id.clone(),
                question: question.to_string(),
                addressed_to,
                escalation,
            },
        },
    ))?;
    Ok(question_id)
}

/// Record a criterion evaluation the runtime performed.
pub fn record_evaluation(
    sessions_home: &Path,
    commitment_id: &str,
    evaluation: &Evaluation,
) -> Result<(), vak_commit::LedgerError> {
    CommitmentLedger::new(sessions_home).append(&Event::new(
        commitment_id,
        EventKind::CriterionEvaluated {
            criterion_id: evaluation.criterion_id.clone(),
            result: evaluation.result.clone(),
            strength: evaluation.strength,
        },
    ))
}

/// Sweep commitments whose economics have run out.
///
/// Expiry is an explicit `Expired` verdict rather than a deletion, so the
/// record says the work lapsed instead of quietly forgetting it existed.
/// Budget exhaustion deliberately does **not** close anything: a human can
/// raise the ceiling, and everything the commitment established is still
/// good, so the scheduler simply stops offering it.
pub fn sweep_expired(sessions_home: &Path) -> Vec<String> {
    let ledger = CommitmentLedger::new(sessions_home);
    let now = chrono::Utc::now();
    let mut closed = Vec::new();
    for commitment in ledger.open() {
        if !commitment.is_expired(now) {
            continue;
        }
        let strength = commitment.achieved_strength();
        if ledger
            .append(&Event::new(
                &commitment.commitment_id,
                EventKind::Closed {
                    verdict: Verdict::Expired,
                    strength,
                    evidence: Vec::new(),
                    note: "past its relevance window without closing".into(),
                },
            ))
            .is_ok()
        {
            closed.push(commitment.commitment_id);
        }
    }
    closed
}

/// One maintenance pass over the portfolio.
///
/// Runs on the server's ordinary tick, independently of whether the heartbeat
/// is enabled: heartbeat is an opt-in *model* pass and costs tokens, while
/// everything here is filesystem and clock work that costs none. Tying durable
/// work's upkeep to an opt-in prober would mean a commitment stopped being
/// durable the moment someone turned the prober off.
///
/// Everything it does is either a state transition the ledger already
/// authorises or an evaluation the runtime performs itself. It never
/// dispatches a model and never closes work as fulfilled.
#[derive(Debug, Default, PartialEq)]
pub struct Maintenance {
    /// Closed `Expired` because their relevance window passed.
    pub expired: Vec<String>,
    /// Woken because a scheduled time arrived.
    pub resumed: Vec<String>,
    /// Woken because a predicate the runtime can check became true.
    pub satisfied: Vec<String>,
    /// Deferred questions whose escalation policy came due.
    pub escalated: Vec<String>,
}

impl Maintenance {
    pub fn is_empty(&self) -> bool {
        self.expired.is_empty()
            && self.resumed.is_empty()
            && self.satisfied.is_empty()
            && self.escalated.is_empty()
    }

    fn extend(&mut self, other: Maintenance) {
        self.expired.extend(other.expired);
        self.resumed.extend(other.resumed);
        self.satisfied.extend(other.satisfied);
        self.escalated.extend(other.escalated);
    }
}

/// One upkeep pass over every Agent's portfolio.
///
/// Commitments live in the Agent that holds them
/// (`<data home>/agents/<agent_id>/`, docs/design/64-agent-owned-platform.md),
/// so a pass over one home would leave every other Agent's scheduled wakes
/// and deadlines unkept. The data home's own ledger is included for work
/// admitted without an Agent.
pub async fn maintain_all(shared_data_home: &Path) -> Maintenance {
    let mut homes = vec![shared_data_home.to_path_buf()];
    if let Ok(entries) =
        std::fs::read_dir(vak_config::scope::SharedScope::new(shared_data_home).agents_dir())
    {
        let mut agents: Vec<_> = entries
            .flatten()
            .map(|entry| entry.path())
            .filter(|path| {
                vak_config::scope::AgentScope::new(path)
                    .commitments()
                    .is_file()
            })
            .collect();
        agents.sort();
        homes.extend(agents);
    }
    let mut report = Maintenance::default();
    for home in homes {
        report.extend(maintain(&home).await);
    }
    report
}

/// Advance whatever the clock and the workspace now permit, for the
/// portfolio in one home. A predicate is checked against the workspace its
/// commitment belongs to, never the caller's.
pub async fn maintain(sessions_home: &Path) -> Maintenance {
    let ledger = CommitmentLedger::new(sessions_home);
    let now = chrono::Utc::now();
    let mut report = Maintenance {
        expired: sweep_expired(sessions_home),
        ..Maintenance::default()
    };

    for commitment in ledger.open() {
        let Some(suspension) = commitment.suspension.clone() else {
            continue;
        };
        match suspension {
            vak_commit::Suspension::Schedule { at: Some(at), .. } if now >= at => {
                if ledger
                    .append(&Event::new(
                        &commitment.commitment_id,
                        EventKind::Resumed {
                            reason: "scheduled time reached".into(),
                        },
                    ))
                    .is_ok()
                {
                    report.resumed.push(commitment.commitment_id.clone());
                }
            }
            // A predicate is checked by the runtime for free. This is the
            // path that lets "watch X and tell me when Y" cost nothing at all
            // while Y stays false.
            vak_commit::Suspension::Predicate { criterion } => {
                let evaluation = WorkspaceEvaluator {
                    cwd: &commitment.spec.cwd,
                }
                .evaluate(&criterion)
                .await;
                if evaluation.passed()
                    && record_evaluation(sessions_home, &commitment.commitment_id, &evaluation)
                        .is_ok()
                    && ledger
                        .append(&Event::new(
                            &commitment.commitment_id,
                            EventKind::Resumed {
                                reason: format!("condition met: {}", criterion.statement),
                            },
                        ))
                        .is_ok()
                {
                    report.satisfied.push(commitment.commitment_id.clone());
                }
            }
            vak_commit::Suspension::Commitment { commitment_id } => {
                if ledger
                    .get(&commitment_id)
                    .ok()
                    .flatten()
                    .is_some_and(|dependency| {
                        dependency
                            .closure
                            .as_ref()
                            .is_some_and(|closure| closure.verdict == Verdict::Fulfilled)
                    })
                    && ledger
                        .append(&Event::new(
                            &commitment.commitment_id,
                            EventKind::Resumed {
                                reason: format!("dependency {commitment_id} was fulfilled"),
                            },
                        ))
                        .is_ok()
                {
                    report.resumed.push(commitment.commitment_id.clone());
                }
            }
            vak_commit::Suspension::Human {
                question_id,
                escalation,
                ..
            } => {
                if let Some(id) =
                    escalate_if_due(&ledger, &commitment, &question_id, &escalation, now)
                {
                    report.escalated.push(id);
                }
            }
            _ => {}
        }
    }
    report
}

/// Apply a deferred question's escalation policy once its deadline passes.
///
/// A question with no policy waits forever by design; that is a decision, not
/// a leak. What must never happen is a silent default standing in for consent
/// on work that cannot be undone, so `AssumeConservative` is refused there
/// even if a grant somehow carried it.
fn escalate_if_due(
    ledger: &CommitmentLedger,
    commitment: &vak_commit::Commitment,
    question_id: &str,
    escalation: &vak_intent::Escalation,
    now: chrono::DateTime<chrono::Utc>,
) -> Option<String> {
    let waited_hours = (now - commitment.updated_at).num_minutes() as f64 / 60.0;
    let (after, verdict, note): (u32, Option<Verdict>, &str) = match escalation {
        vak_intent::Escalation::WaitIndefinitely => return None,
        vak_intent::Escalation::AssumeConservative { after_hours } => {
            if !escalation.permitted_for(commitment.spec.reading.stakes) {
                return None;
            }
            (
                *after_hours,
                None,
                "no answer; taking the conservative branch",
            )
        }
        vak_intent::Escalation::AbandonAfter { after_hours } => (
            *after_hours,
            Some(Verdict::Abandoned),
            "no answer within the agreed window",
        ),
        vak_intent::Escalation::Reassign { after_hours, .. } => {
            (*after_hours, None, "reassigned after no answer")
        }
    };
    if waited_hours < f64::from(after) {
        return None;
    }
    let event = match verdict {
        Some(verdict) => Event::new(
            &commitment.commitment_id,
            EventKind::Closed {
                verdict,
                strength: commitment.achieved_strength(),
                evidence: Vec::new(),
                note: format!("{note} (question {question_id})"),
            },
        ),
        None => Event::new(
            &commitment.commitment_id,
            EventKind::Resumed {
                reason: format!("{note} (question {question_id})"),
            },
        ),
    };
    ledger
        .append(&event)
        .ok()
        .map(|()| commitment.commitment_id.clone())
}

/// A criterion evaluator backed by the workspace filesystem.
///
/// Only the checks that need no permission gate live here: file existence and
/// content. `Shell`, `ToolSucceeded` and `FlowCompleted` are permissioned
/// effects and must cross the tool broker, so they return `Unknown` until a
/// caller supplies a brokered evaluator — an honest "not determined" rather
/// than a failure the work did not earn.
///
/// It runs in the server process, outside any sandbox, so a criterion may
/// only name a path inside its commitment's workspace (invariant 10): an
/// absolute path, a `..` step, or a symlink out of the tree is not checked
/// at all, rather than turning a pass/fail into a probe of the machine.
pub struct WorkspaceEvaluator<'a> {
    pub cwd: &'a Path,
}

/// `path` inside `cwd`, or `None` when it names anything outside it.
fn inside_workspace(cwd: &Path, relative: &Path) -> Option<std::path::PathBuf> {
    if relative.is_absolute()
        || relative.components().any(|component| {
            !matches!(
                component,
                std::path::Component::Normal(_) | std::path::Component::CurDir
            )
        })
    {
        return None;
    }
    let root = cwd.canonicalize().ok()?;
    let joined = root.join(relative);
    let mut probe = joined.as_path();
    loop {
        if let Ok(real) = probe.canonicalize() {
            return real.starts_with(&root).then_some(joined);
        }
        probe = probe.parent()?;
    }
}

impl vak_commit::CriterionEvaluator for WorkspaceEvaluator<'_> {
    async fn evaluate(&self, criterion: &WorkCriterion) -> Evaluation {
        let outside = || {
            Evaluation::unknown(
                &criterion.criterion_id,
                "names a path outside the workspace; not checked",
            )
        };
        match &criterion.kind {
            CriterionKind::FileExists { path } => {
                let Some(resolved) = inside_workspace(self.cwd, path) else {
                    return outside();
                };
                if resolved.exists() {
                    Evaluation::observed(
                        criterion,
                        CriterionResult::Passed {
                            evidence: format!("{} exists", resolved.display()),
                        },
                    )
                } else {
                    Evaluation::observed(
                        criterion,
                        CriterionResult::Failed {
                            reason: format!("{} does not exist", resolved.display()),
                        },
                    )
                }
            }
            CriterionKind::FileContains { path, pattern } => {
                let Some(resolved) = inside_workspace(self.cwd, path) else {
                    return outside();
                };
                match std::fs::read_to_string(&resolved) {
                    Ok(text) if text.contains(pattern) => Evaluation::observed(
                        criterion,
                        CriterionResult::Passed {
                            evidence: format!("{} contains the pattern", resolved.display()),
                        },
                    ),
                    Ok(_) => Evaluation::observed(
                        criterion,
                        CriterionResult::Failed {
                            reason: format!("{} does not contain the pattern", resolved.display()),
                        },
                    ),
                    // Unreadable is not the same as failing: the check could
                    // not run, and saying otherwise would be as dishonest as
                    // claiming it passed.
                    Err(error) => Evaluation::unknown(
                        &criterion.criterion_id,
                        format!("could not read {}: {error}", resolved.display()),
                    ),
                }
            }
            _ => Evaluation::unknown(
                &criterion.criterion_id,
                "needs a brokered evaluator; not checked here",
            ),
        }
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::*;
    use vak_commit::CriterionEvaluator;
    use vak_intent::{Evidence, Horizon, Reading};

    fn intent_with(evidence: Evidence, horizon: Horizon, confidence: f64) -> Intent {
        intent_on(
            evidence,
            horizon,
            confidence,
            "t1.0",
            vak_intent::Lineage::New,
        )
    }

    /// A one-strand intent. `strand_id` is also the thread for a new strand
    /// and a replacement; a continuation inherits its thread.
    fn intent_on(
        evidence: Evidence,
        horizon: Horizon,
        confidence: f64,
        strand_id: &str,
        lineage: vak_intent::Lineage,
    ) -> Intent {
        let reading = Reading {
            horizon,
            evidence,
            confidence,
            axis_confidence: vak_intent::Confidences {
                act: confidence,
                horizon: confidence,
                stakes: confidence,
                evidence: confidence,
            },
            ..Reading::general()
        };
        let engagement = vak_intent::derive(&reading, &vak_intent::Authority::default(), false);
        let thread_id = lineage.continued_thread().unwrap_or(strand_id).to_string();
        let mut intent = Intent::general(vak_intent::RESOLVER_VERSION);
        intent.strands = vec![vak_intent::Strand {
            strand_id: strand_id.into(),
            thread_id,
            text: String::new(),
            reading: reading.clone(),
            relation: vak_intent::StrandRelation::Independent,
            lineage,
            engagement: engagement.clone(),
        }];
        intent.reading = reading;
        intent.engagement = engagement;
        intent
    }

    fn begin_episode(
        sessions_home: &Path,
        config: &vak_config::Config,
        intent: &Intent,
        prompt: &str,
        session_id: &str,
        cwd: &Path,
    ) -> Option<EpisodeHandle> {
        let plan = plan_episodes(sessions_home, config, intent, chrono::Utc::now());
        begin_episodes(
            sessions_home,
            config,
            intent,
            &plan,
            prompt,
            session_id,
            cwd,
            Some("local"),
            None,
        )
        .into_iter()
        .next()
    }

    fn config() -> vak_config::Config {
        vak_config::Config::default()
    }

    #[test]
    fn a_turn_horizon_opens_no_commitment() {
        let dir = tempfile::tempdir().unwrap();
        let handle = begin_episode(
            dir.path(),
            &config(),
            &intent_with(Evidence::None, Horizon::Turn, 0.9),
            "fix the test",
            "s1",
            dir.path(),
        );
        assert!(handle.is_none());
        assert!(CommitmentLedger::new(dir.path()).all().is_empty());
    }

    #[test]
    fn a_durable_horizon_opens_one_and_records_an_episode() {
        let dir = tempfile::tempdir().unwrap();
        let handle = begin_episode(
            dir.path(),
            &config(),
            &intent_with(Evidence::None, Horizon::Durable, 0.9),
            "watch the cloud bill every day",
            "s1",
            dir.path(),
        )
        .expect("a durable turn opens a commitment");
        let commitment = CommitmentLedger::new(dir.path())
            .get(&handle.commitment_id)
            .unwrap()
            .unwrap();
        assert_eq!(commitment.episodes.len(), 1);
        assert_eq!(commitment.spec.objective, "watch the cloud bill every day");
    }

    /// A commitment on the durable thread, opened directly.
    fn open_on_thread(ledger: &CommitmentLedger, cwd: &Path, thread: &str) -> String {
        let mut spec = vak_commit::spec_from_reading(
            "resume the migration",
            vak_intent::Reading {
                horizon: Horizon::Durable,
                ..Reading::general()
            },
            Vec::new(),
            cwd.to_path_buf(),
            Economics::default(),
        );
        spec.thread_id = Some(thread.into());
        ledger.open_commitment(spec).unwrap()
    }

    #[test]
    fn resuming_a_session_closes_its_orphaned_episode_before_starting_again() {
        let dir = tempfile::tempdir().unwrap();
        let ledger = CommitmentLedger::new(dir.path());
        let id = open_on_thread(&ledger, dir.path(), "t0.0");
        ledger
            .append(&Event::new(
                &id,
                EventKind::EpisodeStarted {
                    episode_id: "orphan".into(),
                    session_id: "s1".into(),
                },
            ))
            .unwrap();
        let intent = intent_on(
            Evidence::None,
            Horizon::Durable,
            0.9,
            "t1.0",
            vak_intent::Lineage::Continues {
                thread_id: "t0.0".into(),
            },
        );
        let next = begin_episode(
            dir.path(),
            &config(),
            &intent,
            "resume the migration",
            "s1",
            dir.path(),
        )
        .unwrap();
        assert_eq!(next.commitment_id, id);
        let commitment = ledger.get(&id).unwrap().unwrap();
        assert!(commitment.episodes[0].ended_at.is_some());
        assert!(matches!(
            commitment.episodes[0].advancement,
            Some(Advancement::Blocked { .. })
        ));
        assert_ne!(next.episode_id, "orphan");
    }

    /// A strand that continues a durable thread works on that thread's
    /// commitment even when its own reading is a one-turn request: "now also
    /// check staging" is part of the job it continues.
    #[test]
    fn a_continuation_works_on_its_threads_commitment_whatever_its_own_horizon() {
        let dir = tempfile::tempdir().unwrap();
        let ledger = CommitmentLedger::new(dir.path());
        let id = open_on_thread(&ledger, dir.path(), "t0.0");
        let intent = intent_on(
            Evidence::None,
            Horizon::Turn,
            0.9,
            "t1.0",
            vak_intent::Lineage::Continues {
                thread_id: "t0.0".into(),
            },
        );
        let handle = begin_episode(
            dir.path(),
            &config(),
            &intent,
            "also staging",
            "s2",
            dir.path(),
        )
        .expect("the continuation joins the open commitment");
        assert_eq!(handle.commitment_id, id);
        assert_eq!(ledger.all().len(), 1, "no twin was opened");
    }

    /// `/goal replace` on durable work opens its successor and supersedes the
    /// original, keyed by the replacement's own thread.
    #[test]
    fn a_replacement_supersedes_the_replaced_threads_commitment() {
        let dir = tempfile::tempdir().unwrap();
        let ledger = CommitmentLedger::new(dir.path());
        let old = open_on_thread(&ledger, dir.path(), "t0.0");
        let intent = intent_on(
            Evidence::None,
            Horizon::Turn,
            0.9,
            "t1.0",
            vak_intent::Lineage::Replaces {
                thread_id: "t0.0".into(),
            },
        );
        let handle = begin_episode(
            dir.path(),
            &config(),
            &intent,
            "do this instead",
            "s2",
            dir.path(),
        )
        .expect("the replacement opens a successor");
        assert_ne!(handle.commitment_id, old);
        let old = ledger.get(&old).unwrap().unwrap();
        assert_eq!(
            old.superseded_by.as_deref(),
            Some(handle.commitment_id.as_str())
        );
        let new = ledger.get(&handle.commitment_id).unwrap().unwrap();
        assert_eq!(new.spec.thread_id.as_deref(), Some("t1.0"));
        assert_eq!(new.spec.audience_id.as_deref(), Some("local"));
    }

    /// Two turns that each ask for durable work open two commitments: strand
    /// ids carry the turn id, so the second can never mistake the first's
    /// thread for its own.
    #[test]
    fn separate_durable_requests_open_separate_commitments() {
        let dir = tempfile::tempdir().unwrap();
        for (turn, prompt) in [
            ("t1.0", "watch the bill daily"),
            ("t2.0", "watch the logs daily"),
        ] {
            let intent = intent_on(
                Evidence::None,
                Horizon::Durable,
                0.9,
                turn,
                vak_intent::Lineage::New,
            );
            begin_episode(dir.path(), &config(), &intent, prompt, "s1", dir.path()).unwrap();
        }
        assert_eq!(CommitmentLedger::new(dir.path()).open().len(), 2);
    }

    /// Only an existing commitment can carry a grant, and only a live one is
    /// offered to the turn.
    #[test]
    fn the_plan_offers_only_live_envelopes_of_existing_commitments() {
        let dir = tempfile::tempdir().unwrap();
        let ledger = CommitmentLedger::new(dir.path());
        let id = open_on_thread(&ledger, dir.path(), "t0.0");
        let now = chrono::Utc::now();
        let envelope = vak_intent::Envelope {
            envelope_id: "env-1".into(),
            granted_by: "owner".into(),
            granted_at: now,
            expires_at: None,
            spend_limit_usd: Some(5.0),
            path_scope: vec!["docs/**".into()],
            tool_scope: Vec::new(),
            permission_ceiling: vak_intent::PermissionCeiling::WorkspaceWrite,
            escalation: vak_intent::Escalation::WaitIndefinitely,
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
        let intent = intent_on(
            Evidence::None,
            Horizon::Turn,
            0.9,
            "t1.0",
            vak_intent::Lineage::Continues {
                thread_id: "t0.0".into(),
            },
        );
        let plan = plan_episodes(dir.path(), &config(), &intent, now);
        assert!(plan.envelopes().contains_key("t1.0"));
        assert_eq!(plan.enveloped_commitments(), vec![id.clone()]);

        ledger
            .append(&Event::new(
                &id,
                EventKind::EnvelopeRevoked {
                    envelope_id: "env-1".into(),
                    by: "owner".into(),
                },
            ))
            .unwrap();
        let plan = plan_episodes(dir.path(), &config(), &intent, now);
        assert!(
            plan.envelopes().is_empty(),
            "a revoked grant is not offered"
        );
    }

    /// The upkeep evaluator runs outside any sandbox, so a criterion naming a
    /// path outside the workspace is not checked at all.
    #[tokio::test]
    async fn the_workspace_evaluator_refuses_paths_outside_the_workspace() {
        let outer = tempfile::tempdir().unwrap();
        let workspace = outer.path().join("ws");
        std::fs::create_dir_all(&workspace).unwrap();
        std::fs::write(outer.path().join("secret.txt"), "hunter2").unwrap();
        #[cfg(unix)]
        std::os::unix::fs::symlink(outer.path(), workspace.join("up")).unwrap();
        let evaluator = WorkspaceEvaluator { cwd: &workspace };
        let mut paths = vec![
            "../secret.txt".to_string(),
            outer.path().join("secret.txt").display().to_string(),
        ];
        if cfg!(unix) {
            paths.push("up/secret.txt".into());
        }
        for path in paths {
            let evaluation = evaluator
                .evaluate(&WorkCriterion {
                    criterion_id: "leak".into(),
                    statement: "probe".into(),
                    kind: CriterionKind::FileContains {
                        path: path.clone().into(),
                        pattern: "hunter2".into(),
                    },
                    required: true,
                })
                .await;
            assert!(
                matches!(evaluation.result, CriterionResult::Unknown { .. }),
                "{path} was checked: {:?}",
                evaluation.result
            );
        }
    }

    /// Upkeep reaches every Agent's portfolio, not only the server's own.
    #[tokio::test]
    async fn upkeep_covers_every_agents_ledger() {
        let data = tempfile::tempdir().unwrap();
        let agent_home = data.path().join("agents").join("helper");
        std::fs::create_dir_all(&agent_home).unwrap();
        let ledger = CommitmentLedger::new(&agent_home);
        let id = open_on_thread(&ledger, data.path(), "t0.0");
        ledger
            .append(&Event::new(
                &id,
                EventKind::Suspended {
                    suspension: vak_commit::Suspension::Schedule {
                        at: Some(chrono::Utc::now() - chrono::Duration::minutes(1)),
                        cron: None,
                    },
                },
            ))
            .unwrap();
        assert_eq!(maintain_all(data.path()).await.resumed, vec![id]);
    }

    /// A stray recurrence-ish word must not leave a month-long obligation
    /// behind. Weak horizon evidence declines to open one.
    #[test]
    fn a_weak_horizon_reading_opens_nothing() {
        let dir = tempfile::tempdir().unwrap();
        let handle = begin_episode(
            dir.path(),
            &config(),
            &intent_with(Evidence::None, Horizon::Durable, 0.3),
            "maybe keep an eye on things",
            "s1",
            dir.path(),
        );
        assert!(handle.is_none());
    }

    #[test]
    fn disabling_commitments_opens_nothing() {
        let dir = tempfile::tempdir().unwrap();
        let mut config = config();
        config.commitment.enabled = false;
        assert!(
            begin_episode(
                dir.path(),
                &config,
                &intent_with(Evidence::None, Horizon::Durable, 0.9),
                "watch the bill every day",
                "s1",
                dir.path(),
            )
            .is_none()
        );
    }

    /// The runtime may only propose criteria it could also check. Guessing a
    /// shell command would manufacture `Observed` evidence from a guess.
    #[test]
    fn seeded_criteria_are_never_stronger_than_asserted() {
        for evidence in [Evidence::Verified, Evidence::Audited] {
            let intent = intent_with(evidence, Horizon::Durable, 0.9);
            for criterion in seed_criteria(&intent, "do the thing") {
                assert_eq!(
                    vak_commit::strength_of(&criterion.kind),
                    Satisfaction::Asserted
                );
            }
        }
        // And nothing at all is seeded when no proof is owed.
        assert!(seed_criteria(&intent_with(Evidence::None, Horizon::Durable, 0.9), "x").is_empty());
    }

    /// Which means a seeded commitment cannot close itself: it stays visibly
    /// open until a checkable criterion or a human attestation arrives.
    #[test]
    fn a_seeded_verified_commitment_cannot_be_closed_by_the_seed_alone() {
        let dir = tempfile::tempdir().unwrap();
        let handle = begin_episode(
            dir.path(),
            &config(),
            &intent_with(Evidence::Verified, Horizon::Durable, 0.9),
            "migrate the schema and prove it works",
            "s1",
            dir.path(),
        )
        .unwrap();
        let ledger = CommitmentLedger::new(dir.path());
        ledger
            .append(&Event::new(
                &handle.commitment_id,
                EventKind::CriterionEvaluated {
                    criterion_id: "objective".into(),
                    result: CriterionResult::Passed {
                        evidence: "I believe this is done".into(),
                    },
                    strength: Satisfaction::Asserted,
                },
            ))
            .unwrap();
        let refused = ledger.append(&Event::new(
            &handle.commitment_id,
            EventKind::Closed {
                verdict: Verdict::Fulfilled,
                strength: Satisfaction::Asserted,
                evidence: Vec::new(),
                note: "done".into(),
            },
        ));
        assert!(refused.is_err());
    }

    #[test]
    fn exhausting_the_turn_budget_counts_as_a_stall_but_answering_does_not() {
        let stalled = classify(&vak_agent::TurnOutcome::MaxTurnsReached, 12, Vec::new());
        assert!(stalled.is_stall());

        let answered = classify(
            &vak_agent::TurnOutcome::Completed {
                response: vak_llm::AssistantMessage {
                    content: vec![vak_llm::ContentBlock::Text {
                        text: "Here is a substantive answer that reduced uncertainty a lot.".into(),
                    }],
                    ..vak_llm::AssistantMessage::empty("test-model")
                },
            },
            0,
            Vec::new(),
        );
        assert!(!answered.is_stall());
        assert!(matches!(answered, Advancement::Learned { .. }));

        let tool_only = classify(
            &vak_agent::TurnOutcome::Completed {
                response: vak_llm::AssistantMessage::empty("test-model"),
            },
            12,
            Vec::new(),
        );
        assert!(tool_only.is_stall());
    }

    #[test]
    fn moving_a_criterion_is_advancement_whatever_else_happened() {
        let advancement = classify(
            &vak_agent::TurnOutcome::MaxTurnsReached,
            0,
            vec!["tests-pass".into()],
        );
        assert!(matches!(advancement, Advancement::Advanced { .. }));
        assert!(!advancement.is_stall());
    }

    #[tokio::test]
    async fn the_workspace_evaluator_observes_files_and_abstains_on_the_rest() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("out.txt"), "all good").unwrap();
        let evaluator = WorkspaceEvaluator { cwd: dir.path() };

        let exists = evaluator
            .evaluate(&WorkCriterion {
                criterion_id: "c1".into(),
                statement: "output exists".into(),
                kind: CriterionKind::FileExists {
                    path: "out.txt".into(),
                },
                required: true,
            })
            .await;
        assert!(exists.passed());
        assert_eq!(exists.strength, Satisfaction::Observed);

        // A permissioned check abstains rather than guessing.
        let shell = evaluator
            .evaluate(&WorkCriterion {
                criterion_id: "c2".into(),
                statement: "tests pass".into(),
                kind: CriterionKind::Shell {
                    command: "cargo test".into(),
                },
                required: true,
            })
            .await;
        assert!(!shell.passed());
        assert!(matches!(shell.result, CriterionResult::Unknown { .. }));
    }

    #[tokio::test]
    async fn a_predicate_suspension_wakes_when_the_condition_becomes_true() {
        let dir = tempfile::tempdir().unwrap();
        let ledger = CommitmentLedger::new(dir.path());
        let handle = begin_episode(
            dir.path(),
            &config(),
            &intent_with(Evidence::None, Horizon::Durable, 0.9),
            "tell me when the report lands, every day",
            "s1",
            dir.path(),
        )
        .unwrap();
        let criterion = WorkCriterion {
            criterion_id: "landed".into(),
            statement: "report.csv exists".into(),
            kind: CriterionKind::FileExists {
                path: "report.csv".into(),
            },
            required: true,
        };
        ledger
            .append(&Event::new(
                &handle.commitment_id,
                EventKind::Suspended {
                    suspension: vak_commit::Suspension::Predicate {
                        criterion: criterion.clone(),
                    },
                },
            ))
            .unwrap();

        // While the condition is false the pass costs nothing and changes
        // nothing — this is the zero-token watch path.
        let report = maintain(dir.path()).await;
        assert!(report.satisfied.is_empty());
        assert_eq!(
            ledger.get(&handle.commitment_id).unwrap().unwrap().phase,
            vak_commit::Phase::Suspended
        );

        std::fs::write(dir.path().join("report.csv"), "done").unwrap();
        let report = maintain(dir.path()).await;
        assert_eq!(report.satisfied, vec![handle.commitment_id.clone()]);
        assert_ne!(
            ledger.get(&handle.commitment_id).unwrap().unwrap().phase,
            vak_commit::Phase::Suspended
        );
    }

    #[tokio::test]
    async fn a_commitment_dependency_wakes_after_fulfillment() {
        let dir = tempfile::tempdir().unwrap();
        let ledger = CommitmentLedger::new(dir.path());
        let dependency = ledger
            .open_commitment(vak_commit::spec_from_reading(
                "dependency",
                Reading::general(),
                Vec::new(),
                dir.path().to_path_buf(),
                Economics::default(),
            ))
            .unwrap();
        ledger
            .append(&Event::new(
                &dependency,
                EventKind::Closed {
                    verdict: Verdict::Fulfilled,
                    strength: Satisfaction::Asserted,
                    evidence: Vec::new(),
                    note: "done".into(),
                },
            ))
            .unwrap();
        let waiting = ledger
            .open_commitment(vak_commit::spec_from_reading(
                "waiting",
                Reading::general(),
                Vec::new(),
                dir.path().to_path_buf(),
                Economics::default(),
            ))
            .unwrap();
        ledger
            .append(&Event::new(
                &waiting,
                EventKind::Suspended {
                    suspension: vak_commit::Suspension::Commitment {
                        commitment_id: dependency,
                    },
                },
            ))
            .unwrap();
        let report = maintain(dir.path()).await;
        assert_eq!(report.resumed, vec![waiting]);
    }

    #[tokio::test]
    async fn a_scheduled_suspension_wakes_once_its_time_arrives() {
        let dir = tempfile::tempdir().unwrap();
        let ledger = CommitmentLedger::new(dir.path());
        let handle = begin_episode(
            dir.path(),
            &config(),
            &intent_with(Evidence::None, Horizon::Durable, 0.9),
            "check the bill every day",
            "s1",
            dir.path(),
        )
        .unwrap();
        ledger
            .append(&Event::new(
                &handle.commitment_id,
                EventKind::Suspended {
                    suspension: vak_commit::Suspension::Schedule {
                        at: Some(chrono::Utc::now() + chrono::Duration::hours(2)),
                        cron: None,
                    },
                },
            ))
            .unwrap();
        assert!(maintain(dir.path()).await.resumed.is_empty());

        ledger
            .append(&Event::new(
                &handle.commitment_id,
                EventKind::Suspended {
                    suspension: vak_commit::Suspension::Schedule {
                        at: Some(chrono::Utc::now() - chrono::Duration::minutes(1)),
                        cron: None,
                    },
                },
            ))
            .unwrap();
        assert_eq!(
            maintain(dir.path()).await.resumed,
            vec![handle.commitment_id]
        );
    }

    /// A question with no policy waits forever by design. That is a decision,
    /// not a leak — and it must not quietly become an assumption.
    #[tokio::test]
    async fn an_unanswered_question_waits_unless_a_policy_says_otherwise() {
        let dir = tempfile::tempdir().unwrap();
        let ledger = CommitmentLedger::new(dir.path());
        let handle = begin_episode(
            dir.path(),
            &config(),
            &intent_with(Evidence::None, Horizon::Durable, 0.9),
            "migrate the schema every night",
            "s1",
            dir.path(),
        )
        .unwrap();
        defer_for_human(
            dir.path(),
            &handle.commitment_id,
            "which database?",
            None,
            vak_intent::Escalation::WaitIndefinitely,
        )
        .unwrap();
        assert!(maintain(dir.path()).await.escalated.is_empty());
        assert_eq!(
            ledger.get(&handle.commitment_id).unwrap().unwrap().phase,
            vak_commit::Phase::Suspended
        );
    }

    #[test]
    fn expiry_closes_explicitly_and_budget_exhaustion_does_not() {
        let dir = tempfile::tempdir().unwrap();
        let ledger = CommitmentLedger::new(dir.path());
        let mut config = config();
        config.commitment.default_ttl_days = Some(1);

        let handle = begin_episode(
            dir.path(),
            &config,
            &intent_with(Evidence::None, Horizon::Durable, 0.9),
            "watch it every day",
            "s1",
            dir.path(),
        )
        .unwrap();

        // Not yet expired.
        assert!(sweep_expired(dir.path()).is_empty());

        // An over-budget commitment is held by the scheduler, never closed.
        ledger
            .append(&Event::new(
                &handle.commitment_id,
                EventKind::EpisodeEnded {
                    episode_id: handle.episode_id.clone(),
                    advancement: Advancement::Learned { fact: "x".into() },
                    spend_usd: 9_999.0,
                },
            ))
            .unwrap();
        assert!(sweep_expired(dir.path()).is_empty());
        let commitment = ledger.get(&handle.commitment_id).unwrap().unwrap();
        assert!(
            commitment.is_over_budget() || commitment.spec.economics.lifetime_budget_usd.is_none()
        );
        assert!(!commitment.phase.is_terminal());
    }
}
