//! The narrowing lattice.
//!
//! [`Limits`] is the half of an engagement that carries authority
//! implications. Every field is a **restriction**, the whole thing is a meet
//! semilattice, and [`Limits::unrestricted`] is its top element — the identity
//! that reproduces vak's behaviour before the intent kernel existed.
//!
//! This module exists so invariant 1 ("intent narrows, never widens") is a
//! property of the type rather than a rule reviewers have to remember.
//! Composition is only ever [`Limits::meet`], `meet` is proven `⊑` both
//! operands by test, and nothing in the crate offers a "widen" operation to
//! call by accident.
//!
//! Selections that carry no authority implication — which renderer to use,
//! how chatty to be — deliberately live in [`crate::Posture`] instead, so that
//! this type stays small enough to reason about completely. Capacity is not
//! here either: a reading never caps the route ladder, the turn count or the
//! workers, because a wrong reading would then take away what the request
//! needed (invariant 32: a reading decides what is loaded, never what is
//! possible).

use std::collections::BTreeSet;

use serde::{Deserialize, Serialize};

use crate::authority::{ApprovalCeiling, PermissionCeiling};
use crate::axes::{Modality, Satisfaction};

/// Which domain capabilities this turn loads.
///
/// Forms a meet-semilattice:
/// - `All` is the top element (unconstrained, admits any domain).
/// - `Only(set)` restricts to the specified domains.
/// - `Empty` is the bottom element (admits no domain).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(tag = "kind", rename_all = "kebab-case")]
pub enum DomainSet {
    /// No domain restriction: any capability domain is admitted.
    #[serde(rename = "all")]
    #[default]
    All,
    /// Only capabilities declaring at least one of these domains are admitted.
    #[serde(rename = "only")]
    Only { names: BTreeSet<String> },
    /// No domain admitted.
    #[serde(rename = "empty")]
    Empty,
}

impl DomainSet {
    pub fn all() -> Self {
        DomainSet::All
    }

    pub fn only<I, S>(names: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        let names: BTreeSet<String> = names.into_iter().map(Into::into).collect();
        if names.is_empty() {
            DomainSet::Empty
        } else {
            DomainSet::Only { names }
        }
    }

    pub fn empty() -> Self {
        DomainSet::Empty
    }

    pub fn allows(&self, domain: &str) -> bool {
        match self {
            DomainSet::All => true,
            DomainSet::Only { names } => names.contains(domain),
            DomainSet::Empty => false,
        }
    }

    pub fn contains(&self, domain: &str) -> bool {
        self.allows(domain)
    }

    pub fn is_unconstrained(&self) -> bool {
        matches!(self, DomainSet::All)
    }

    pub fn is_empty(&self) -> bool {
        match self {
            DomainSet::All => false,
            DomainSet::Only { names } => names.is_empty(),
            DomainSet::Empty => true,
        }
    }

    /// Least upper bound over the *names*: the domains either side needs.
    ///
    /// This is not a lattice `join` on authority — it exists for one caller,
    /// composing the strands of a multi-intent turn, where a turn that is
    /// "search the web and then run the tests" needs both toolsets. Relative
    /// to the unrestricted baseline it is still a narrowing (a finite set),
    /// which is the invariant that matters; see `Engagement::compose`.
    pub fn union(&self, other: &DomainSet) -> DomainSet {
        match (self, other) {
            (DomainSet::All, _) | (_, DomainSet::All) => DomainSet::All,
            (DomainSet::Empty, other) => other.clone(),
            (this, DomainSet::Empty) => this.clone(),
            (DomainSet::Only { names: a }, DomainSet::Only { names: b }) => {
                DomainSet::only(a.union(b).cloned())
            }
        }
    }

    pub fn iter(&self) -> std::collections::btree_set::Iter<'_, String> {
        static EMPTY_SET: std::sync::LazyLock<BTreeSet<String>> =
            std::sync::LazyLock::new(BTreeSet::new);
        match self {
            DomainSet::All | DomainSet::Empty => EMPTY_SET.iter(),
            DomainSet::Only { names } => names.iter(),
        }
    }

    /// Greatest lower bound (meet).
    pub fn meet(&self, other: &DomainSet) -> DomainSet {
        match (self, other) {
            (DomainSet::All, other) => other.clone(),
            (this, DomainSet::All) => this.clone(),
            (DomainSet::Empty, _) | (_, DomainSet::Empty) => DomainSet::Empty,
            (DomainSet::Only { names: a }, DomainSet::Only { names: b }) => {
                let inter: BTreeSet<String> = a.intersection(b).cloned().collect();
                if inter.is_empty() {
                    DomainSet::Empty
                } else {
                    DomainSet::Only { names: inter }
                }
            }
        }
    }

    /// Whether self admits nothing other forbids (self ⊑ other).
    pub fn is_at_most(&self, other: &DomainSet) -> bool {
        match (self, other) {
            (_, DomainSet::All) => true,
            (DomainSet::Empty, _) => true,
            (DomainSet::All, _) => false,
            // `Only {}` (reachable through deserialisation) is ⊥ too.
            (DomainSet::Only { names }, DomainSet::Empty) => names.is_empty(),
            (DomainSet::Only { names: a }, DomainSet::Only { names: b }) => a.is_subset(b),
        }
    }
}

impl<S: Into<String>> FromIterator<S> for DomainSet {
    /// Same rule as [`DomainSet::only`]: an empty collection is the bottom
    /// element, never the top. Collecting nothing must not silently admit
    /// everything.
    fn from_iter<T: IntoIterator<Item = S>>(iter: T) -> Self {
        DomainSet::only(iter)
    }
}

impl From<BTreeSet<String>> for DomainSet {
    fn from(names: BTreeSet<String>) -> Self {
        DomainSet::only(names)
    }
}

/// Take the smaller of two optional caps, treating `None` as "no cap".
fn meet_cap(a: Option<f64>, b: Option<f64>) -> Option<f64> {
    match (a, b) {
        (None, other) => other,
        (this, None) => this,
        (Some(x), Some(y)) => Some(x.min(y)),
    }
}

/// Whether `a` is no larger a cap than `b`. `None` is unbounded, so it is only
/// `⊑` another `None`.
fn cap_is_at_most(a: Option<f64>, b: Option<f64>) -> bool {
    match (a, b) {
        (_, None) => true,
        (None, Some(_)) => false,
        (Some(x), Some(y)) => x <= y,
    }
}

/// The authority-bearing half of an engagement.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Limits {
    /// Modalities a serving leg must support. Requiring more narrows the set
    /// of legs that may serve, so union is the narrowing direction.
    #[serde(default)]
    pub required_modalities: BTreeSet<Modality>,
    /// Kinds of work this turn plausibly needs, as domain names a capability
    /// can declare itself against (`crate::capability::domain` in vak-core).
    /// Decides which admitted tools are *loaded*; the rest stay one
    /// `find_tools` call away.
    #[serde(default)]
    pub required_domains: DomainSet,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub spend_ceiling_usd: Option<f64>,
    #[serde(default)]
    pub approval_ceiling: ApprovalCeiling,
    #[serde(default)]
    pub permission_ceiling: PermissionCeiling,
    /// The weakest evidence that may close this work as fulfilled. Raising it
    /// is a narrowing: it forbids closures that were previously allowed.
    #[serde(default)]
    pub min_satisfaction: Satisfaction,
}

impl Default for Limits {
    fn default() -> Self {
        Limits::unrestricted()
    }
}

impl Limits {
    /// The top element: restricts nothing.
    pub fn unrestricted() -> Self {
        Limits {
            required_modalities: BTreeSet::new(),
            required_domains: DomainSet::All,
            spend_ceiling_usd: None,
            approval_ceiling: ApprovalCeiling::AutoApprove,
            permission_ceiling: PermissionCeiling::FullAccess,
            min_satisfaction: Satisfaction::Asserted,
        }
    }

    /// Greatest lower bound: the strictest limits that satisfy both.
    ///
    /// This is the **only** composition operator in the crate. There is no
    /// `join`, because widening is not an operation this system is allowed to
    /// perform.
    pub fn meet(&self, other: &Limits) -> Limits {
        Limits {
            required_domains: self.required_domains.meet(&other.required_domains),
            required_modalities: self
                .required_modalities
                .union(&other.required_modalities)
                .copied()
                .collect(),
            spend_ceiling_usd: meet_cap(self.spend_ceiling_usd, other.spend_ceiling_usd),
            approval_ceiling: self.approval_ceiling.meet(other.approval_ceiling),
            permission_ceiling: self.permission_ceiling.meet(other.permission_ceiling),
            min_satisfaction: if other.min_satisfaction.rank() > self.min_satisfaction.rank() {
                other.min_satisfaction
            } else {
                self.min_satisfaction
            },
        }
    }

    /// Whether `self` grants nothing that `baseline` does not already grant.
    ///
    /// This is invariant 1 as a predicate. `vak-core` asserts it on every
    /// turn in debug builds, and the property tests prove it holds for every
    /// reading the resolver can produce.
    pub fn is_at_most(&self, baseline: &Limits) -> bool {
        baseline
            .required_modalities
            .is_subset(&self.required_modalities)
            && self.required_domains.is_at_most(&baseline.required_domains)
            && cap_is_at_most(self.spend_ceiling_usd, baseline.spend_ceiling_usd)
            && self.approval_ceiling.rank() <= baseline.approval_ceiling.rank()
            && self.permission_ceiling.rank() <= baseline.permission_ceiling.rank()
            && self.min_satisfaction.rank() >= baseline.min_satisfaction.rank()
    }

    /// Human-readable list of what this narrows relative to `baseline`.
    ///
    /// Powers `vak intent explain` and the admin console's engagement diff. A
    /// narrowing nobody can see is a narrowing nobody can debug.
    pub fn diff_from(&self, baseline: &Limits) -> Vec<String> {
        let mut out = Vec::new();
        if self.required_domains != baseline.required_domains {
            match &self.required_domains {
                DomainSet::All => {}
                DomainSet::Only { names } => out.push(format!(
                    "domains loaded: {} ({})",
                    names.len(),
                    names.iter().cloned().collect::<Vec<_>>().join(", ")
                )),
                DomainSet::Empty => out.push("no tool domains loaded".into()),
            }
        }
        if self.required_modalities != baseline.required_modalities
            && !self.required_modalities.is_empty()
        {
            let names: Vec<&str> = self
                .required_modalities
                .iter()
                .map(|m| m.as_str())
                .collect();
            out.push(format!("serving leg must support {}", names.join(", ")));
        }
        if self.spend_ceiling_usd != baseline.spend_ceiling_usd
            && let Some(cap) = self.spend_ceiling_usd
        {
            out.push(format!("spend ceiling ${cap:.4}"));
        }
        if self.approval_ceiling != baseline.approval_ceiling {
            out.push(format!(
                "approval capped at {}",
                self.approval_ceiling.as_str()
            ));
        }
        if self.permission_ceiling != baseline.permission_ceiling {
            out.push(format!(
                "permission capped at {}",
                self.permission_ceiling.as_str()
            ));
        }
        if self.min_satisfaction != baseline.min_satisfaction {
            out.push(format!(
                "closure requires {} evidence",
                self.min_satisfaction.as_str()
            ));
        }
        out
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::*;

    fn sample() -> Vec<Limits> {
        let mut modal = Limits::unrestricted();
        modal.required_modalities.insert(Modality::Image);

        vec![
            Limits::unrestricted(),
            Limits {
                required_domains: DomainSet::only(["filesystem", "memory"]),
                spend_ceiling_usd: Some(0.01),
                approval_ceiling: ApprovalCeiling::Ask,
                permission_ceiling: PermissionCeiling::ReadOnly,
                min_satisfaction: Satisfaction::Attested,
                ..Limits::unrestricted()
            },
            Limits {
                required_domains: DomainSet::only(["filesystem", "code-exec"]),
                spend_ceiling_usd: Some(2.0),
                approval_ceiling: ApprovalCeiling::ApproveSafe,
                min_satisfaction: Satisfaction::Observed,
                ..Limits::unrestricted()
            },
            modal,
        ]
    }

    /// Invariant 1, stated as a lattice law: a meet is below both operands.
    #[test]
    fn meet_is_below_both_operands() {
        for a in sample() {
            for b in sample() {
                let met = a.meet(&b);
                assert!(met.is_at_most(&a), "meet not below lhs: {met:?} vs {a:?}");
                assert!(met.is_at_most(&b), "meet not below rhs: {met:?} vs {b:?}");
            }
        }
    }

    #[test]
    fn unrestricted_is_the_identity() {
        for limits in sample() {
            assert_eq!(limits.meet(&Limits::unrestricted()), limits);
            assert_eq!(Limits::unrestricted().meet(&limits), limits);
            assert!(limits.is_at_most(&Limits::unrestricted()));
        }
    }

    #[test]
    fn meet_is_commutative_and_idempotent() {
        for a in sample() {
            assert_eq!(a.meet(&a), a);
            for b in sample() {
                assert_eq!(a.meet(&b), b.meet(&a));
            }
        }
    }

    #[test]
    fn meet_is_associative() {
        for a in sample() {
            for b in sample() {
                for c in sample() {
                    assert_eq!(a.meet(&b).meet(&c), a.meet(&b.meet(&c)));
                }
            }
        }
    }

    /// Widening must be detectable, or `is_at_most` is decorative.
    #[test]
    fn widening_any_field_is_rejected() {
        let narrow = Limits {
            required_domains: DomainSet::only(["filesystem"]),
            spend_ceiling_usd: Some(0.5),
            approval_ceiling: ApprovalCeiling::Ask,
            permission_ceiling: PermissionCeiling::ReadOnly,
            min_satisfaction: Satisfaction::Attested,
            required_modalities: BTreeSet::from([Modality::Image]),
        };
        let widened = [
            Limits {
                required_domains: DomainSet::All,
                ..narrow.clone()
            },
            Limits {
                spend_ceiling_usd: Some(5.0),
                ..narrow.clone()
            },
            Limits {
                spend_ceiling_usd: None,
                ..narrow.clone()
            },
            Limits {
                approval_ceiling: ApprovalCeiling::AutoApprove,
                ..narrow.clone()
            },
            Limits {
                permission_ceiling: PermissionCeiling::FullAccess,
                ..narrow.clone()
            },
            Limits {
                min_satisfaction: Satisfaction::Asserted,
                ..narrow.clone()
            },
            Limits {
                required_modalities: BTreeSet::new(),
                ..narrow.clone()
            },
        ];
        for candidate in widened {
            assert!(
                !candidate.is_at_most(&narrow),
                "widening went undetected: {candidate:?}"
            );
        }
    }

    #[test]
    fn diff_names_every_narrowing_it_applies() {
        let narrow = Limits {
            required_domains: DomainSet::only(["filesystem"]),
            approval_ceiling: ApprovalCeiling::Ask,
            min_satisfaction: Satisfaction::Observed,
            ..Limits::unrestricted()
        };
        let diff = narrow.diff_from(&Limits::unrestricted());
        assert_eq!(diff.len(), 3, "{diff:?}");
        assert!(
            Limits::unrestricted()
                .diff_from(&Limits::unrestricted())
                .is_empty()
        );
    }
}
