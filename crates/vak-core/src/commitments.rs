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

use vak_commit::{
    Advancement, CommitmentLedger, CommitmentSpec, Economics, Event, EventKind, Evaluation,
    Verdict,
};
use vak_intent::{Intent, Satisfaction};
use vak_session::types::{CriterionKind, CriterionResult, WorkCriterion};

/// A commitment this turn is serving, and the episode it opened on it.
#[derive(Debug, Clone)]
pub struct EpisodeHandle {
    pub commitment_id: String,
    pub episode_id: String,
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
    if intent.reading.evidence.min_satisfaction().rank() <= Satisfaction::Cited.rank() {
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

/// Open a commitment for this turn, or attach to the session's existing one.
///
/// Returns `None` when the reading does not call for one, when commitments
/// are disabled, or when the ledger is unwritable — a failure to record a
/// commitment must never cost the user their turn.
pub fn begin_episode(
    sessions_home: &Path,
    config: &vak_config::Config,
    intent: &Intent,
    prompt: &str,
    session_id: &str,
    cwd: &Path,
    existing: Option<&str>,
) -> Option<EpisodeHandle> {
    if !config.commitment.enabled || !intent.engagement.posture.open_commitment {
        return None;
    }
    // A weak horizon reading must not manufacture a durable obligation: a
    // stray "then" should not leave a month-long commitment behind.
    if !intent
        .reading
        .may_open_commitment(config.intent.accept_confidence)
    {
        return None;
    }

    let ledger = CommitmentLedger::new(sessions_home);
    let commitment_id = match existing {
        Some(id) => id.to_string(),
        None => {
            let objective = objective_from(prompt);
            let spec = CommitmentSpec {
                criteria: seed_criteria(intent, &objective),
                min_satisfaction: intent.reading.evidence.min_satisfaction(),
                economics: economics(config),
                cwd: cwd.to_path_buf(),
                supersedes: None,
                objective,
                reading: intent.reading.clone(),
            };
            match ledger.open_commitment(spec) {
                Ok(id) => id,
                Err(error) => {
                    eprintln!("[commit] could not open a commitment: {error}");
                    return None;
                }
            }
        }
    };

    let episode_id = format!("ep-{session_id}-{}", chrono::Utc::now().timestamp_millis());
    if let Err(error) = ledger.append(&Event::new(
        &commitment_id,
        EventKind::EpisodeStarted {
            episode_id: episode_id.clone(),
            session_id: session_id.to_string(),
        },
    )) {
        eprintln!("[commit] could not start an episode: {error}");
        return None;
    }
    Some(EpisodeHandle {
        commitment_id,
        episode_id,
    })
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
            if said_something || tool_calls > 0 {
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
) {
    let ledger = CommitmentLedger::new(sessions_home);
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
    let question_id = format!("q-{}", chrono::Utc::now().timestamp_millis());
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

/// A criterion evaluator backed by the workspace filesystem.
///
/// Only the checks that need no permission gate live here: file existence and
/// content. `Shell`, `ToolSucceeded` and `FlowCompleted` are permissioned
/// effects and must cross the tool broker, so they return `Unknown` until a
/// caller supplies a brokered evaluator — an honest "not determined" rather
/// than a failure the work did not earn.
pub struct WorkspaceEvaluator<'a> {
    pub cwd: &'a Path,
}

impl vak_commit::CriterionEvaluator for WorkspaceEvaluator<'_> {
    async fn evaluate(&self, criterion: &WorkCriterion) -> Evaluation {
        match &criterion.kind {
            CriterionKind::FileExists { path } => {
                let resolved = self.cwd.join(path);
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
                let resolved = self.cwd.join(path);
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
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;
    use vak_commit::CriterionEvaluator;
    use vak_intent::{Evidence, Horizon, Reading};

    fn intent_with(evidence: Evidence, horizon: Horizon, confidence: f64) -> Intent {
        let mut intent = Intent::general(vak_intent::RESOLVER_VERSION);
        intent.reading = Reading {
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
        intent.engagement =
            vak_intent::derive(&intent.reading, &vak_intent::Authority::default(), false);
        intent
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
            None,
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
            None,
        )
        .expect("a durable turn opens a commitment");
        let commitment = CommitmentLedger::new(dir.path())
            .get(&handle.commitment_id)
            .unwrap()
            .unwrap();
        assert_eq!(commitment.episodes.len(), 1);
        assert_eq!(commitment.spec.objective, "watch the cloud bill every day");
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
            None,
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
                None,
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
            None,
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
                        text: "Here is a substantive answer that reduced uncertainty a lot."
                            .into(),
                    }],
                    ..vak_llm::AssistantMessage::empty("test-model")
                },
            },
            0,
            Vec::new(),
        );
        assert!(!answered.is_stall());
        assert!(matches!(answered, Advancement::Learned { .. }));
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
            None,
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
        assert!(commitment.is_over_budget() || commitment.spec.economics.lifetime_budget_usd.is_none());
        assert!(!commitment.phase.is_terminal());
    }
}
