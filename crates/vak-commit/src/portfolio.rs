//! Choosing which commitment to advance next.
//!
//! Once vak holds obligations rather than a single conversation, something has
//! to decide what gets worked on. That decision is **deterministic and
//! inspectable** rather than a learned policy: every commitment's priority
//! decomposes into named components a person can read and argue with, and the
//! user can always pin one to the front.
//!
//! An opaque scheduler in a system whose entire thesis is auditability would
//! be the one place you could not ask "why did it do that".

use serde::{Deserialize, Serialize};

use vak_intent::Stakes;

use crate::types::{Commitment, Phase};

/// Why a commitment scored where it did.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Priority {
    pub commitment_id: String,
    pub score: f64,
    /// Named contributions, largest first. This is the explanation.
    pub components: Vec<(String, f64)>,
    /// Set when the commitment cannot be worked right now, with the reason.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub withheld: Option<String>,
}

impl Priority {
    pub fn is_runnable(&self) -> bool {
        self.withheld.is_none()
    }

    /// One line for `vak commit list` or the admin portfolio view.
    pub fn explain(&self) -> String {
        match &self.withheld {
            Some(reason) => format!("held: {reason}"),
            None => {
                let parts: Vec<String> = self
                    .components
                    .iter()
                    .map(|(name, value)| format!("{name} {value:+.2}"))
                    .collect();
                format!("{:.2} = {}", self.score, parts.join(", "))
            }
        }
    }
}

/// Inputs the scheduler cannot read off a commitment itself.
#[derive(Debug, Clone, Default)]
pub struct SchedulerContext {
    pub now: Option<chrono::DateTime<chrono::Utc>>,
    /// Commitment ids the user pinned to the front.
    pub pinned: Vec<String>,
}

impl SchedulerContext {
    fn now(&self) -> chrono::DateTime<chrono::Utc> {
        self.now.unwrap_or_else(chrono::Utc::now)
    }
}

/// Score one commitment.
pub fn prioritize(commitment: &Commitment, context: &SchedulerContext) -> Priority {
    let now = context.now();
    let mut components: Vec<(String, f64)> = Vec::new();
    let mut withheld = None;

    // --- reasons not to run --------------------------------------------
    if commitment.phase.is_terminal() {
        withheld = Some("closed".to_string());
    } else if commitment.phase == Phase::Suspended {
        withheld = Some(
            commitment
                .suspension
                .as_ref()
                .map(|suspension| suspension.describe())
                .unwrap_or_else(|| "suspended".into()),
        );
    } else if commitment.phase == Phase::Blocked {
        withheld = Some(
            commitment
                .blocker
                .clone()
                .unwrap_or_else(|| "blocked".into()),
        );
    } else if commitment.is_over_budget() {
        // Budget exhaustion holds the work rather than failing it: a human can
        // raise the ceiling, and destroying the commitment would throw away
        // everything it had established.
        withheld = Some(format!(
            "lifetime budget exhausted (${:.2})",
            commitment.spend_usd
        ));
    } else if commitment.is_expired(now) {
        withheld = Some("past its relevance window".to_string());
    } else if commitment.is_stalled() {
        withheld = Some(format!(
            "stalled for {} consecutive episodes",
            commitment.consecutive_stalls
        ));
    }

    // --- pin -----------------------------------------------------------
    if context.pinned.contains(&commitment.commitment_id) {
        // A pin dominates every computed factor. The user's explicit choice is
        // not something a heuristic gets to outvote.
        components.push(("pinned".into(), 1000.0));
    }

    // --- stakes --------------------------------------------------------
    let stakes_weight = match commitment.spec.reading.stakes {
        Stakes::Irreversible => 4.0,
        Stakes::Costly => 3.0,
        Stakes::Reversible => 2.0,
        Stakes::Inert => 1.0,
    };
    components.push(("stakes".into(), stakes_weight));

    // --- deadline proximity --------------------------------------------
    if let Some(expiry) = commitment.spec.economics.expires_at {
        let hours_left = (expiry - now).num_minutes() as f64 / 60.0;
        if hours_left > 0.0 {
            // Rises sharply as the window closes; 24h out is worth ~1, an hour
            // out is worth ~24.
            components.push(("deadline".into(), (24.0 / hours_left).min(50.0)));
        }
    }

    // --- staleness -----------------------------------------------------
    let idle_hours = (now - commitment.updated_at).num_minutes() as f64 / 60.0;
    if idle_hours > 0.0 {
        // Logarithmic: something untouched for a week should surface, but not
        // so hard that it starves urgent work.
        components.push(("staleness".into(), (1.0 + idle_hours).ln().max(0.0)));
    }

    // --- progress ------------------------------------------------------
    // Work that is nearly done is worth finishing before work that has barely
    // started; a half-finished commitment is a liability.
    let total = commitment.criteria.len();
    if total > 0 {
        let passed = commitment.criteria.iter().filter(|c| c.passed()).count();
        components.push(("progress".into(), 2.0 * (passed as f64 / total as f64)));
    }

    // --- review due ----------------------------------------------------
    if let Some(hours) = commitment.spec.economics.review_every_hours
        && idle_hours >= f64::from(hours)
    {
        components.push(("review-due".into(), 3.0));
    }

    components.sort_by(|a, b| {
        b.1.partial_cmp(&a.1)
            .unwrap_or(std::cmp::Ordering::Equal)
            .then_with(|| a.0.cmp(&b.0))
    });
    let score = components.iter().map(|(_, value)| value).sum();

    Priority {
        commitment_id: commitment.commitment_id.clone(),
        score,
        components,
        withheld,
    }
}

/// Rank a portfolio, highest priority first.
///
/// Held commitments sort after runnable ones but are still returned, because
/// "why is nothing happening" is a question the portfolio view must be able to
/// answer without a second query.
pub fn rank(commitments: &[Commitment], context: &SchedulerContext) -> Vec<Priority> {
    let mut out: Vec<Priority> = commitments
        .iter()
        .map(|commitment| prioritize(commitment, context))
        .collect();
    out.sort_by(|a, b| {
        a.is_runnable()
            .cmp(&b.is_runnable())
            .reverse()
            .then_with(|| {
                b.score
                    .partial_cmp(&a.score)
                    .unwrap_or(std::cmp::Ordering::Equal)
            })
            // Total order: ties break on id so the ranking is stable across
            // runs and reproducible in a test.
            .then_with(|| a.commitment_id.cmp(&b.commitment_id))
    });
    out
}

/// The next commitment to advance, if any is runnable.
pub fn next(commitments: &[Commitment], context: &SchedulerContext) -> Option<String> {
    rank(commitments, context)
        .into_iter()
        .find(|priority| priority.is_runnable())
        .map(|priority| priority.commitment_id)
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::*;
    use crate::types::{CommitmentSpec, Economics, Phase};
    use vak_intent::{Reading, Satisfaction};

    fn commitment(id: &str, stakes: Stakes) -> Commitment {
        let now = chrono::Utc::now();
        Commitment {
            commitment_id: id.into(),
            opened_at: now,
            spec: CommitmentSpec {
                objective: format!("objective {id}"),
                reading: Reading {
                    stakes,
                    ..Reading::general()
                },
                criteria: Vec::new(),
                min_satisfaction: Satisfaction::Asserted,
                economics: Economics::default(),
                cwd: std::path::PathBuf::from("/tmp"),
                supersedes: None,
            },
            phase: Phase::Active,
            criteria: Vec::new(),
            episodes: Vec::new(),
            suspension: None,
            blocker: None,
            envelope: None,
            closure: None,
            superseded_by: None,
            spend_usd: 0.0,
            consecutive_stalls: 0,
            drift: Vec::new(),
            updated_at: now,
        }
    }

    fn context() -> SchedulerContext {
        SchedulerContext {
            now: Some(chrono::Utc::now()),
            pinned: Vec::new(),
        }
    }

    #[test]
    fn ranking_is_deterministic() {
        let commitments = vec![
            commitment("b", Stakes::Reversible),
            commitment("a", Stakes::Reversible),
        ];
        let context = context();
        assert_eq!(rank(&commitments, &context), rank(&commitments, &context),);
    }

    #[test]
    fn a_pin_outranks_every_computed_factor() {
        let mut urgent = commitment("urgent", Stakes::Irreversible);
        urgent.spec.economics.expires_at = Some(chrono::Utc::now() + chrono::Duration::minutes(30));
        let boring = commitment("boring", Stakes::Inert);
        let context = SchedulerContext {
            now: Some(chrono::Utc::now()),
            pinned: vec!["boring".into()],
        };
        assert_eq!(next(&[urgent, boring], &context).as_deref(), Some("boring"));
    }

    #[test]
    fn higher_stakes_outrank_lower_all_else_equal() {
        let low = commitment("low", Stakes::Inert);
        let high = commitment("high", Stakes::Irreversible);
        assert_eq!(next(&[low, high], &context()).as_deref(), Some("high"));
    }

    #[test]
    fn suspended_blocked_and_closed_work_is_held_with_a_reason() {
        let mut suspended = commitment("s", Stakes::Costly);
        suspended.phase = Phase::Suspended;
        suspended.suspension = Some(crate::types::Suspension::Schedule {
            at: Some(chrono::Utc::now() + chrono::Duration::hours(4)),
            cron: None,
        });

        let mut blocked = commitment("b", Stakes::Costly);
        blocked.phase = Phase::Blocked;
        blocked.blocker = Some("needs a database password".into());

        let active = commitment("a", Stakes::Inert);

        let ranked = rank(&[suspended, blocked, active], &context());
        assert_eq!(ranked[0].commitment_id, "a");
        assert!(ranked[0].is_runnable());
        for held in &ranked[1..] {
            assert!(!held.is_runnable());
            assert!(held.withheld.as_ref().is_some_and(|r| !r.is_empty()));
        }
    }

    /// Budget exhaustion must hold the work, not destroy it: a human can raise
    /// the ceiling, and everything the commitment established is still good.
    #[test]
    fn an_over_budget_commitment_is_held_rather_than_closed() {
        let mut broke = commitment("broke", Stakes::Costly);
        broke.spec.economics.lifetime_budget_usd = Some(5.0);
        broke.spend_usd = 6.0;
        let priority = prioritize(&broke, &context());
        assert!(!priority.is_runnable());
        assert!(priority.withheld.as_ref().unwrap().contains("budget"));
        assert!(!broke.phase.is_terminal());
    }

    #[test]
    fn a_stalled_commitment_stops_being_scheduled() {
        let mut stalled = commitment("stalled", Stakes::Costly);
        stalled.consecutive_stalls = 3;
        assert!(stalled.is_stalled());
        assert!(!prioritize(&stalled, &context()).is_runnable());
    }

    #[test]
    fn an_imminent_deadline_raises_priority() {
        let mut soon = commitment("soon", Stakes::Inert);
        soon.spec.economics.expires_at = Some(chrono::Utc::now() + chrono::Duration::minutes(30));
        let later = commitment("later", Stakes::Inert);
        assert_eq!(next(&[later, soon], &context()).as_deref(), Some("soon"));
    }

    #[test]
    fn every_priority_explains_itself() {
        let priority = prioritize(&commitment("x", Stakes::Costly), &context());
        assert!(!priority.components.is_empty());
        assert!(priority.explain().contains("stakes"));
    }
}
