//! Authority: what a human has **delegated**, as distinct from what the
//! request happens to need.
//!
//! # Why this is separate from the reading
//!
//! [`crate::Reading`] is inferred from the request. Authority is granted by a
//! person. Keeping them apart is the whole point: a resolution bug can change
//! what vak *thinks the work is*, and must never change *what it is allowed to
//! do about it*.
//!
//! # How this composes with the permission engine
//!
//! It does not replace it, and it cannot outvote it. There are two separate
//! questions and vak had been treating them as one:
//!
//! 1. **Is a gate raised?** Decided by the permission engine, exactly as
//!    today, and then *possibly tightened* by [`Authority::approval_ceiling`].
//!    Autonomy can suppress a gate the engine already made optional; it can
//!    never suppress a `Deny`, and it can never turn a `Deny` into an `Ask`.
//! 2. **Can a raised gate be answered?** Decided by [`Attendance`] and the
//!    hosting surface's approver. This is where an unattended surface used to
//!    fail closed unconditionally — correct for a one-shot turn, wrong for
//!    month-long work, which should *wait* rather than *fail*. See
//!    [`GateFallback`].
//!
//! Both directions only ever narrow. `ApprovalCeiling` is a cap on
//! permissiveness, and an [`Envelope`] carries a `permission_ceiling` that can
//! lower the effective mode but never raise it — pre-authorization *within*
//! existing authority, never a grant of new authority.

use serde::{Deserialize, Serialize};

use crate::axes::{Attendance, Horizon, Stakes};

/// How much authority a human has delegated for this work.
///
/// Ordered least to most delegated. Note that this is *not* capped by
/// [`Attendance`]: how much you have delegated and whether you are watching
/// are independent facts. Their interaction is handled where it belongs — in
/// [`Authority::gate_fallback`], which decides what happens to a gate nobody
/// is there to answer.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Hash, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Autonomy {
    /// Propose only. Every effect is approved before it happens.
    Manual,
    /// Act on reversible things; ask before anything costly or irreversible.
    #[default]
    Assisted,
    /// Act freely inside a declared [`Envelope`]; escalate outside it.
    Delegated,
    /// Act freely within the permission mode and report afterwards.
    Autonomous,
}

impl Autonomy {
    pub const ALL: [Autonomy; 4] = [
        Autonomy::Manual,
        Autonomy::Assisted,
        Autonomy::Delegated,
        Autonomy::Autonomous,
    ];

    /// Explicit total order; spelled out rather than derived so reordering
    /// the variants cannot silently widen a grant.
    pub fn rank(self) -> u8 {
        match self {
            Autonomy::Manual => 0,
            Autonomy::Assisted => 1,
            Autonomy::Delegated => 2,
            Autonomy::Autonomous => 3,
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Autonomy::Manual => "manual",
            Autonomy::Assisted => "assisted",
            Autonomy::Delegated => "delegated",
            Autonomy::Autonomous => "autonomous",
        }
    }

    pub fn parse(value: &str) -> Option<Autonomy> {
        Autonomy::ALL
            .into_iter()
            .find(|a| a.as_str() == value.trim().to_ascii_lowercase())
    }

    /// The least delegated of the two. Used wherever two grants meet — a
    /// per-chat grant under a per-bot grant, or a worker under its parent —
    /// so composition can only ever reduce.
    pub fn capped_by(self, ceiling: Autonomy) -> Autonomy {
        if self.rank() > ceiling.rank() {
            ceiling
        } else {
            self
        }
    }
}

/// A cap on how permissive approval handling may be.
///
/// Mirrors the ranking of `vak_config::ApprovalMode` (`ask` < `approve-safe` <
/// `auto-approve`). It is duplicated rather than imported so the intent kernel
/// stays a pure decision layer with no configuration dependency and can be
/// tested standalone; `vak-core` maps between the two in exactly one place.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Hash, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum ApprovalCeiling {
    /// Every gate reaches a human.
    Ask,
    /// Gates the engine considers safe may resolve themselves.
    ApproveSafe,
    /// Gates may resolve themselves.
    #[default]
    AutoApprove,
}

impl ApprovalCeiling {
    pub const ALL: [ApprovalCeiling; 3] = [
        ApprovalCeiling::Ask,
        ApprovalCeiling::ApproveSafe,
        ApprovalCeiling::AutoApprove,
    ];

    pub fn rank(self) -> u8 {
        match self {
            ApprovalCeiling::Ask => 0,
            ApprovalCeiling::ApproveSafe => 1,
            ApprovalCeiling::AutoApprove => 2,
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            ApprovalCeiling::Ask => "ask",
            ApprovalCeiling::ApproveSafe => "approve-safe",
            ApprovalCeiling::AutoApprove => "auto-approve",
        }
    }

    pub fn parse(value: &str) -> Option<ApprovalCeiling> {
        match value.trim().to_ascii_lowercase().as_str() {
            "ask" | "ask-approval" => Some(ApprovalCeiling::Ask),
            "approve-safe" | "approve-for-me" => Some(ApprovalCeiling::ApproveSafe),
            "auto-approve" => Some(ApprovalCeiling::AutoApprove),
            _ => None,
        }
    }

    /// The stricter of the two. Every composition point uses this, which is
    /// what makes "intent may tighten approval, never loosen it" hold by
    /// construction rather than by review.
    pub fn meet(self, other: ApprovalCeiling) -> ApprovalCeiling {
        if other.rank() < self.rank() {
            other
        } else {
            self
        }
    }
}

/// A cap on the permission mode.
///
/// Mirrors `vak_config::PermissionMode`'s ranking for the same reason
/// [`ApprovalCeiling`] mirrors `ApprovalMode`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Hash, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum PermissionCeiling {
    ReadOnly,
    WorkspaceWrite,
    #[default]
    FullAccess,
}

impl PermissionCeiling {
    pub const ALL: [PermissionCeiling; 3] = [
        PermissionCeiling::ReadOnly,
        PermissionCeiling::WorkspaceWrite,
        PermissionCeiling::FullAccess,
    ];

    pub fn rank(self) -> u8 {
        match self {
            PermissionCeiling::ReadOnly => 0,
            PermissionCeiling::WorkspaceWrite => 1,
            PermissionCeiling::FullAccess => 2,
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            PermissionCeiling::ReadOnly => "read-only",
            PermissionCeiling::WorkspaceWrite => "workspace-write",
            PermissionCeiling::FullAccess => "full-access",
        }
    }

    pub fn parse(value: &str) -> Option<PermissionCeiling> {
        match value.trim().to_ascii_lowercase().as_str() {
            "read-only" | "readonly" => Some(PermissionCeiling::ReadOnly),
            "workspace-write" => Some(PermissionCeiling::WorkspaceWrite),
            "full-access" | "fullaccess" => Some(PermissionCeiling::FullAccess),
            _ => None,
        }
    }

    pub fn meet(self, other: PermissionCeiling) -> PermissionCeiling {
        if other.rank() < self.rank() {
            other
        } else {
            self
        }
    }
}

/// What happens to a gate that was raised and cannot be answered here.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum GateFallback {
    /// Fail closed. The historical behaviour, and still correct for a
    /// one-shot turn with nowhere to park the question.
    Deny,
    /// Suspend the commitment on a `Human` wake condition and put the
    /// question in the inbox. Nothing happens without the answer, but the
    /// work survives to be resumed — which is what long-horizon work needs
    /// and what a hard denial destroys.
    Defer,
}

/// What to do when a deferred question goes unanswered past its deadline.
///
/// A deferred question with no timeout policy is how an agent quietly
/// accumulates a graveyard of half-finished work, so this is not optional.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "kebab-case")]
pub enum Escalation {
    /// Keep waiting. Appropriate when there is no safe default, and the
    /// default for exactly that reason.
    #[default]
    WaitIndefinitely,
    /// Take the conservative branch and record that it was taken without an
    /// answer. Never available above `Stakes::Costly`.
    AssumeConservative { after_hours: u32 },
    /// Give up and close the commitment as `Abandoned`.
    AbandonAfter { after_hours: u32 },
    /// Ask somebody else.
    Reassign { to: String, after_hours: u32 },
}

impl Escalation {
    /// Whether this policy may be applied to work at `stakes`.
    ///
    /// Assuming a default for an irreversible action because nobody replied
    /// is exactly the class of autonomy this system exists to prevent, so the
    /// restriction is enforced rather than documented.
    pub fn permitted_for(&self, stakes: Stakes) -> bool {
        match self {
            Escalation::AssumeConservative { .. } => stakes.rank() <= Stakes::Costly.rank(),
            _ => true,
        }
    }
}

/// A pre-authorization attached to a commitment.
///
/// An envelope answers "what may you do without asking me again, and until
/// when". It is the mechanism that makes unattended long-horizon work possible
/// without either nagging or recklessness, and it is the shape EU AI Act
/// Article 14 human oversight actually wants: a boundary agreed up front with
/// a defined escalation, rather than per-action interrupts that fatigue an
/// overseer into rubber-stamping.
///
/// It can only narrow. `permission_ceiling` lowers the effective mode and
/// never raises it; `spend_limit_usd` lowers the effective budget; the scopes
/// restrict rather than grant.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Envelope {
    pub envelope_id: String,
    /// Who granted it. Recorded because "under whose authority" is the
    /// question an audit asks first.
    pub granted_by: String,
    pub granted_at: chrono::DateTime<chrono::Utc>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub expires_at: Option<chrono::DateTime<chrono::Utc>>,
    /// Lifetime spend for the whole commitment, not per run.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub spend_limit_usd: Option<f64>,
    /// Workspace-relative path globs the grant covers. Empty means "no path
    /// restriction beyond the permission mode's own" — it does not mean
    /// "every path", because the mode is still in force.
    #[serde(default)]
    pub path_scope: Vec<String>,
    /// Tool names the grant covers. Empty means no additional restriction.
    #[serde(default)]
    pub tool_scope: Vec<String>,
    /// Never above the mode already in force.
    #[serde(default)]
    pub permission_ceiling: PermissionCeiling,
    #[serde(default)]
    pub escalation: Escalation,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub revoked_at: Option<chrono::DateTime<chrono::Utc>>,
}

impl Envelope {
    /// Whether the envelope is in force at `now`.
    ///
    /// Revocation and expiry are checked here rather than at the call sites so
    /// there is exactly one place that can get it wrong.
    pub fn is_live(&self, now: chrono::DateTime<chrono::Utc>) -> bool {
        if self.revoked_at.is_some() {
            return false;
        }
        match self.expires_at {
            Some(expiry) => now < expiry,
            None => true,
        }
    }

    /// Whether an action on `tool` touching `paths` falls inside the grant.
    ///
    /// An empty scope is "no additional restriction", not "everything": the
    /// permission mode and rule engine are still the authority. A path that
    /// cannot be matched against the scope counts as outside it, because an
    /// unprovable claim of coverage is not coverage — the same reasoning the
    /// permission engine uses when redirection makes a compound command's
    /// coverage unprovable.
    pub fn covers(&self, tool: &str, paths: &[String]) -> bool {
        if !self.tool_scope.is_empty() && !self.tool_scope.iter().any(|t| t == tool) {
            return false;
        }
        if self.path_scope.is_empty() {
            return true;
        }
        !paths.is_empty()
            && paths.iter().all(|path| {
                workspace_relative(path)
                    .is_some_and(|path| self.path_scope.iter().any(|glob| glob_covers(glob, &path)))
            })
    }
}

/// A path the scope can be matched against: workspace-relative, with no
/// parent-directory step. `src/../.env` would otherwise match `src/**`
/// byte by byte, so anything that climbs, is absolute, or uses a Windows
/// separator is not coverable at all.
fn workspace_relative(path: &str) -> Option<String> {
    let path = path.trim();
    if path.is_empty() || path.starts_with('/') || path.contains('\\') || path.contains(':') {
        return None;
    }
    let mut parts = Vec::new();
    for part in path.split('/') {
        match part {
            "" | "." => {}
            ".." => return None,
            part => parts.push(part),
        }
    }
    (!parts.is_empty()).then(|| parts.join("/"))
}

/// Minimal `*`/`**` glob matching for envelope path scopes.
///
/// Deliberately conservative: anything it cannot prove is covered reads as not
/// covered, so a scope mistake denies rather than grants.
fn glob_covers(pattern: &str, path: &str) -> bool {
    fn matches(pattern: &[u8], path: &[u8]) -> bool {
        match pattern.first() {
            None => path.is_empty(),
            Some(b'*') => {
                // `**` spans separators; a single `*` stops at one.
                let doubled = pattern.get(1) == Some(&b'*');
                let rest = if doubled {
                    &pattern[2..]
                } else {
                    &pattern[1..]
                };
                let rest = if doubled && rest.first() == Some(&b'/') {
                    &rest[1..]
                } else {
                    rest
                };
                if matches(rest, path) {
                    return true;
                }
                for (index, byte) in path.iter().enumerate() {
                    if !doubled && *byte == b'/' {
                        break;
                    }
                    if matches(rest, &path[index + 1..]) {
                        return true;
                    }
                }
                false
            }
            Some(expected) => match path.first() {
                Some(actual) if actual == expected => matches(&pattern[1..], &path[1..]),
                _ => false,
            },
        }
    }
    matches(pattern.as_bytes(), path.as_bytes())
}

/// The composed authority for a unit of work.
///
/// A grant on a commitment is not part of it: which commitment a strand
/// works on is only known after the request is read, so an envelope narrows
/// the strands that serve its commitment (`apply_envelopes`) and covers
/// individual actions at the approval gate (`Envelope::covers`).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Authority {
    pub autonomy: Autonomy,
    /// Observed, not granted.
    pub attendance: Attendance,
}

impl Default for Authority {
    fn default() -> Self {
        Authority {
            autonomy: Autonomy::Assisted,
            attendance: Attendance::Interactive,
        }
    }
}

impl Authority {
    /// The cap this authority places on approval permissiveness for an action
    /// at `stakes`.
    ///
    /// Delegation buys nothing here: a turn's ceiling is fixed before anyone
    /// knows which actions it will take, so `delegated` asks, and the gate
    /// lets through only the actions a live envelope covers — outside the
    /// boundary, delegation buys nothing, which is the point of declaring it.
    ///
    /// The result is a *ceiling*: `vak-core` takes the stricter of this and
    /// the configured `ApprovalMode`. Nothing here can loosen configuration.
    pub fn approval_ceiling(&self, stakes: Stakes) -> ApprovalCeiling {
        let by_stakes = match stakes {
            Stakes::Inert | Stakes::Reversible => ApprovalCeiling::AutoApprove,
            Stakes::Costly => ApprovalCeiling::ApproveSafe,
            // Irreversible always reaches a human, whatever was delegated.
            // A grant to act without asking is not a grant to act without
            // anyone ever knowing.
            Stakes::Irreversible => ApprovalCeiling::Ask,
        };
        let by_autonomy = match self.autonomy {
            Autonomy::Manual => ApprovalCeiling::Ask,
            // "Act on reversible things; ask before anything costly or
            // irreversible" — as the variant's own docstring says. The
            // earlier mapping capped *every* stakes level at `approve-safe`,
            // which made `assisted` indistinguishable from `autonomous` on
            // costly work and silently downgraded an operator's
            // `auto-approve` on a greeting.
            Autonomy::Assisted => {
                if stakes.rank() >= Stakes::Costly.rank() {
                    ApprovalCeiling::Ask
                } else {
                    ApprovalCeiling::AutoApprove
                }
            }
            Autonomy::Delegated => ApprovalCeiling::Ask,
            Autonomy::Autonomous => ApprovalCeiling::AutoApprove,
        };
        by_stakes.meet(by_autonomy)
    }

    /// What to do with a gate that was raised and cannot be answered here.
    ///
    /// Deferring needs somewhere to park the question, so it is only offered
    /// when the work is durable enough to own a commitment. A one-shot
    /// unattended turn still fails closed, exactly as before.
    pub fn gate_fallback(&self, horizon: Horizon) -> GateFallback {
        if self.attendance.can_answer_now() {
            // Somebody is here; the gate resolves the ordinary way and this
            // fallback never applies.
            return GateFallback::Deny;
        }
        if horizon.opens_commitment() {
            GateFallback::Defer
        } else {
            GateFallback::Deny
        }
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::*;

    fn envelope() -> Envelope {
        Envelope {
            envelope_id: "env-1".into(),
            granted_by: "nisheeth".into(),
            granted_at: chrono::Utc::now(),
            expires_at: None,
            spend_limit_usd: Some(5.0),
            path_scope: vec!["src/**".into()],
            tool_scope: vec!["edit".into(), "write".into()],
            permission_ceiling: PermissionCeiling::WorkspaceWrite,
            escalation: Escalation::WaitIndefinitely,
            revoked_at: None,
        }
    }

    /// The single most important property in this module: no combination of
    /// delegation can let an irreversible action past without a human.
    #[test]
    fn irreversible_always_reaches_a_human_whatever_was_delegated() {
        for autonomy in Autonomy::ALL {
            for attendance in Attendance::ALL {
                let authority = Authority {
                    autonomy,
                    attendance,
                };
                assert_eq!(
                    authority.approval_ceiling(Stakes::Irreversible),
                    ApprovalCeiling::Ask,
                    "autonomy={autonomy:?}"
                );
            }
        }
    }

    /// The autonomy table as documented: assisted acts on reversible work
    /// without a gate and asks before anything costly.
    #[test]
    fn assisted_auto_approves_reversible_and_asks_for_costly() {
        let assisted = Authority::default();
        assert_eq!(
            assisted.approval_ceiling(Stakes::Inert),
            ApprovalCeiling::AutoApprove
        );
        assert_eq!(
            assisted.approval_ceiling(Stakes::Reversible),
            ApprovalCeiling::AutoApprove
        );
        assert_eq!(
            assisted.approval_ceiling(Stakes::Costly),
            ApprovalCeiling::Ask
        );
        let autonomous = Authority {
            autonomy: Autonomy::Autonomous,
            ..Authority::default()
        };
        assert_eq!(
            autonomous.approval_ceiling(Stakes::Costly),
            ApprovalCeiling::ApproveSafe
        );
    }

    /// A turn under delegation asks: only the actions a live envelope covers
    /// get through, and that is decided per action at the gate.
    #[test]
    fn delegation_alone_buys_nothing_at_the_turn_level() {
        let authority = Authority {
            autonomy: Autonomy::Delegated,
            attendance: Attendance::Supervised,
        };
        for stakes in [Stakes::Inert, Stakes::Reversible, Stakes::Costly] {
            assert_eq!(authority.approval_ceiling(stakes), ApprovalCeiling::Ask);
        }
    }

    #[test]
    fn approval_ceiling_only_ever_tightens_when_composed() {
        for a in ApprovalCeiling::ALL {
            for b in ApprovalCeiling::ALL {
                let met = a.meet(b);
                assert!(met.rank() <= a.rank() && met.rank() <= b.rank());
            }
        }
    }

    #[test]
    fn revoked_and_expired_envelopes_are_not_live() {
        let now = chrono::Utc::now();
        let mut revoked = envelope();
        revoked.revoked_at = Some(now);
        assert!(!revoked.is_live(now));

        let mut expired = envelope();
        expired.expires_at = Some(now - chrono::Duration::hours(1));
        assert!(!expired.is_live(now));
        assert!(envelope().is_live(now));
    }

    #[test]
    fn envelope_coverage_is_conservative() {
        let envelope = envelope();
        assert!(envelope.covers("edit", &["src/main.rs".into()]));
        assert!(envelope.covers("edit", &["src/deep/nested/file.rs".into()]));
        // Wrong tool.
        assert!(!envelope.covers("bash", &["src/main.rs".into()]));
        // Outside the path scope.
        assert!(!envelope.covers("edit", &["docs/readme.md".into()]));
        // One covered and one not is not coverage.
        assert!(!envelope.covers("edit", &["src/main.rs".into(), "docs/x.md".into()]));
        // A scoped envelope with nothing to check cannot prove coverage.
        assert!(!envelope.covers("edit", &[]));
        // A path that climbs out of the scope, or is absolute, is not
        // covered by a pattern it happens to start with.
        assert!(!envelope.covers("edit", &["src/../.env".into()]));
        assert!(!envelope.covers("edit", &["/etc/src/main.rs".into()]));
        assert!(envelope.covers("edit", &["./src/main.rs".into()]));
    }

    #[test]
    fn unattended_durable_work_defers_instead_of_failing_closed() {
        let authority = Authority {
            autonomy: Autonomy::Delegated,
            attendance: Attendance::Unattended,
        };
        assert_eq!(
            authority.gate_fallback(Horizon::Durable),
            GateFallback::Defer
        );
        // A one-shot turn has nowhere to park the question, so it still fails
        // closed exactly as it did before.
        assert_eq!(
            authority.gate_fallback(Horizon::Immediate),
            GateFallback::Deny
        );
    }

    #[test]
    fn conservative_assumption_is_refused_for_irreversible_work() {
        let policy = Escalation::AssumeConservative { after_hours: 24 };
        assert!(policy.permitted_for(Stakes::Reversible));
        assert!(policy.permitted_for(Stakes::Costly));
        assert!(!policy.permitted_for(Stakes::Irreversible));
        assert!(Escalation::WaitIndefinitely.permitted_for(Stakes::Irreversible));
    }

    #[test]
    fn autonomy_composition_only_reduces() {
        for a in Autonomy::ALL {
            for b in Autonomy::ALL {
                let capped = a.capped_by(b);
                assert!(capped.rank() <= a.rank() && capped.rank() <= b.rank());
            }
        }
    }
}
