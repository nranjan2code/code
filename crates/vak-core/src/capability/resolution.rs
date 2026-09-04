//! Whether a capability is usable right now, and if not, when to look again.
//!
//! The old shape was `Option<Inventory>`, and a failed probe was stored as a
//! *successful* inventory containing a tool literally named `error`. Three
//! things went wrong at once: the prompt advertised a broken server as
//! usable, the alias table minted a callable tool called `error`, and the
//! re-entry guard (`inventory.is_none()`) saw `Some(_)` and refused to retry
//! for the life of the process. Under "never restart", that last one means
//! never.
//!
//! Failure is a state here, never data. It carries a reason, a remedy, and a
//! `retry_at`, and the reconcile loop honours the retry rather than treating
//! the first answer as final.

use std::time::{Duration, SystemTime};

use serde::{Deserialize, Serialize};

/// Backoff schedule for a failing capability. Exponential with a cap, so a
/// server that is down for a week is probed on a slow rhythm rather than
/// hammered, and one that is down for ten seconds recovers almost at once.
const BACKOFF_BASE: Duration = Duration::from_secs(5);
const BACKOFF_CAP: Duration = Duration::from_secs(10 * 60);

/// How long a `Ready` catalog is trusted for a server that does **not**
/// announce its own changes. A server that declares `tools.listChanged` is
/// re-probed only when it says so (plus this as a slow backstop); one that
/// does not must be re-probed on a rhythm, because silence carries no
/// information.
pub const CATALOG_TTL_ANNOUNCED: Duration = Duration::from_secs(30 * 60);
pub const CATALOG_TTL_SILENT: Duration = Duration::from_secs(5 * 60);

/// Why a capability cannot be used, in terms an operator can act on.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Failure {
    /// What went wrong, already redacted of any secret values.
    pub reason: String,
    /// What the operator can do about it. Empty when there is nothing
    /// actionable, which is itself worth saying rather than inventing advice.
    pub remedy: String,
    /// Consecutive failed attempts. Drives the backoff and tells a reader
    /// whether this is a blip or a standing outage.
    pub attempts: u32,
}

impl Failure {
    pub fn new(reason: impl Into<String>, remedy: impl Into<String>) -> Self {
        Failure {
            reason: reason.into(),
            remedy: remedy.into(),
            attempts: 1,
        }
    }

    /// Delay before the next probe, from the attempt count.
    pub fn backoff(&self) -> Duration {
        let shift = self.attempts.saturating_sub(1).min(16);
        BACKOFF_BASE
            .saturating_mul(1u32 << shift.min(16))
            .min(BACKOFF_CAP)
    }
}

/// The lifecycle of one capability's usability.
///
/// `Static` exists because most capabilities have nothing to probe: a
/// built-in tool, a hook, a command and a skill body are all knowable
/// offline. Only things that talk to something else — today, MCP servers —
/// travel the rest of this machine.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "state", rename_all = "kebab-case")]
pub enum Resolution {
    /// Nothing to probe; usable as declared.
    Static,
    /// A probe is in flight, or one is due and has not run yet.
    Probing,
    /// Usable. `checked_at` drives the catalog TTL.
    Ready {
        checked_at: SystemTime,
        /// Whether the source promised to announce its own changes. When
        /// false the loop must re-probe on a rhythm instead of trusting
        /// silence.
        announces_changes: bool,
    },
    /// Not usable. Never carries a catalog.
    Degraded {
        failure: Failure,
        retry_at: SystemTime,
    },
    /// Removed at its source, or revoked. Kept as a tombstone only while a
    /// live epoch still references it, so a session mid-turn can still
    /// resolve what it was told about.
    Retired { reason: String },
}

impl Resolution {
    /// Whether a turn may use this capability.
    pub fn is_usable(&self) -> bool {
        matches!(self, Resolution::Static | Resolution::Ready { .. })
    }

    /// Whether the reconcile loop should probe this capability now.
    pub fn is_due(&self, now: SystemTime) -> bool {
        match self {
            Resolution::Static | Resolution::Retired { .. } => false,
            Resolution::Probing => true,
            Resolution::Degraded { retry_at, .. } => now >= *retry_at,
            Resolution::Ready {
                checked_at,
                announces_changes,
            } => {
                let ttl = if *announces_changes {
                    CATALOG_TTL_ANNOUNCED
                } else {
                    CATALOG_TTL_SILENT
                };
                now.duration_since(*checked_at)
                    .map(|elapsed| elapsed >= ttl)
                    .unwrap_or(true)
            }
        }
    }

    /// Fold a failed probe in, carrying the attempt count forward so backoff
    /// grows across consecutive failures rather than resetting each pass.
    pub fn failed(
        &self,
        now: SystemTime,
        reason: impl Into<String>,
        remedy: impl Into<String>,
    ) -> Resolution {
        let attempts = match self {
            Resolution::Degraded { failure, .. } => failure.attempts.saturating_add(1),
            _ => 1,
        };
        let failure = Failure {
            reason: reason.into(),
            remedy: remedy.into(),
            attempts,
        };
        let retry_at = now + failure.backoff();
        Resolution::Degraded { failure, retry_at }
    }

    pub fn ready(now: SystemTime, announces_changes: bool) -> Resolution {
        Resolution::Ready {
            checked_at: now,
            announces_changes,
        }
    }

    /// One-line status for the operator report and the model's standing
    /// section. Both read this, so they cannot disagree.
    pub fn summary(&self) -> String {
        match self {
            Resolution::Static => "ready".into(),
            Resolution::Probing => "checking".into(),
            Resolution::Ready { .. } => "ready".into(),
            Resolution::Degraded { failure, retry_at } => {
                let secs = retry_at
                    .duration_since(SystemTime::now())
                    .map(|d| d.as_secs())
                    .unwrap_or(0);
                format!(
                    "unavailable: {} (attempt {}, retrying in {}s)",
                    failure.reason, failure.attempts, secs
                )
            }
            Resolution::Retired { reason } => format!("removed: {reason}"),
        }
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::*;

    #[test]
    fn a_failure_is_never_usable() {
        let r = Resolution::Static.failed(SystemTime::now(), "connection refused", "start it");
        assert!(!r.is_usable());
        assert!(matches!(r, Resolution::Degraded { .. }));
    }

    #[test]
    fn consecutive_failures_back_off_and_cap() {
        let now = SystemTime::now();
        let mut r = Resolution::Static.failed(now, "x", "");
        let mut last = Duration::ZERO;
        for _ in 0..24 {
            let Resolution::Degraded { failure, .. } = &r else {
                panic!("expected degraded");
            };
            let backoff = failure.backoff();
            assert!(backoff >= last || backoff == BACKOFF_CAP);
            assert!(backoff <= BACKOFF_CAP);
            last = backoff;
            r = r.failed(now, "x", "");
        }
        assert_eq!(last, BACKOFF_CAP);
    }

    #[test]
    fn a_degraded_capability_becomes_due_again() {
        let now = SystemTime::now();
        let r = Resolution::Static.failed(now, "x", "");
        assert!(!r.is_due(now), "should wait for its backoff");
        assert!(
            r.is_due(now + BACKOFF_CAP + Duration::from_secs(1)),
            "must become due again — this is what makes recovery without a restart possible"
        );
    }

    #[test]
    fn a_silent_server_is_reprobed_sooner_than_an_announcing_one() {
        let now = SystemTime::now();
        let silent = Resolution::ready(now, false);
        let announcing = Resolution::ready(now, true);
        let later = now + CATALOG_TTL_SILENT + Duration::from_secs(1);
        assert!(silent.is_due(later));
        assert!(!announcing.is_due(later));
    }

    #[test]
    fn static_capabilities_are_never_probed() {
        assert!(!Resolution::Static.is_due(SystemTime::now()));
        assert!(Resolution::Static.is_usable());
    }
}
