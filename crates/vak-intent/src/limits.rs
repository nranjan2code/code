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
//! this type stays small enough to reason about completely.

use std::collections::BTreeSet;

use serde::{Deserialize, Serialize};

use crate::authority::{ApprovalCeiling, PermissionCeiling};
use crate::axes::{Modality, Satisfaction};

/// Which capabilities from the session's admitted packet this turn may see.
///
/// Always a *subset* of the frozen packet. The packet itself is unchanged —
/// this narrows advertisement and tool-schema construction, never admission,
/// so a sliced-away capability was still admitted and can still be restored
/// by a wider reading on the next turn without a new session.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "kebab-case")]
pub enum CapabilitySlice {
    /// No narrowing: everything admitted is advertised. The top element, and
    /// the default: absent information must never remove a capability.
    #[default]
    All,
    /// Only these names, intersected with whatever was admitted.
    Only { names: BTreeSet<String> },
}

impl CapabilitySlice {
    pub fn only<I, S>(names: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        CapabilitySlice::Only {
            names: names.into_iter().map(Into::into).collect(),
        }
    }

    pub fn allows(&self, name: &str) -> bool {
        match self {
            CapabilitySlice::All => true,
            CapabilitySlice::Only { names } => names.contains(name),
        }
    }

    /// Intersection. Two slices compose to the capabilities both allow.
    pub fn meet(&self, other: &CapabilitySlice) -> CapabilitySlice {
        match (self, other) {
            (CapabilitySlice::All, other) => other.clone(),
            (this, CapabilitySlice::All) => this.clone(),
            (CapabilitySlice::Only { names: a }, CapabilitySlice::Only { names: b }) => {
                CapabilitySlice::Only {
                    names: a.intersection(b).cloned().collect(),
                }
            }
        }
    }

    /// Whether `self` allows nothing that `other` forbids.
    pub fn is_at_most(&self, other: &CapabilitySlice) -> bool {
        match (self, other) {
            (_, CapabilitySlice::All) => true,
            (CapabilitySlice::All, CapabilitySlice::Only { .. }) => false,
            (CapabilitySlice::Only { names: a }, CapabilitySlice::Only { names: b }) => {
                a.is_subset(b)
            }
        }
    }
}

/// Which domain capabilities this turn may see.
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
            (DomainSet::Only { .. }, DomainSet::Empty) => false,
            (DomainSet::Only { names: a }, DomainSet::Only { names: b }) => a.is_subset(b),
        }
    }
}

impl<S: Into<String>> FromIterator<S> for DomainSet {
    fn from_iter<T: IntoIterator<Item = S>>(iter: T) -> Self {
        let names: BTreeSet<String> = iter.into_iter().map(Into::into).collect();
        if names.is_empty() {
            DomainSet::All
        } else {
            DomainSet::Only { names }
        }
    }
}

impl From<BTreeSet<String>> for DomainSet {
    fn from(names: BTreeSet<String>) -> Self {
        DomainSet::only(names)
    }
}

/// Take the smaller of two optional caps, treating `None` as "no cap".
fn meet_cap<T: PartialOrd + Copy>(a: Option<T>, b: Option<T>) -> Option<T> {
    match (a, b) {
        (None, other) => other,
        (this, None) => this,
        (Some(x), Some(y)) => Some(if y < x { y } else { x }),
    }
}

/// Whether `a` is no larger a cap than `b`. `None` is unbounded, so it is only
/// `⊑` another `None`.
fn cap_is_at_most<T: PartialOrd>(a: Option<T>, b: Option<T>) -> bool {
    match (a, b) {
        (_, None) => true,
        (None, Some(_)) => false,
        (Some(x), Some(y)) => x <= y,
    }
}

/// The authority-bearing half of an engagement.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Limits {
    /// Which admitted capabilities this turn may see.
    #[serde(default)]
    pub capabilities: CapabilitySlice,
    /// Use at most this many legs from the head of the frozen route ladder.
    /// Never reorders and never extends: a prefix of a frozen ladder is still
    /// the frozen ladder, so contract replay is unaffected.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ladder_limit: Option<usize>,
    /// Modalities a serving leg must support. Requiring more narrows the set
    /// of legs that may serve, so union is the narrowing direction.
    #[serde(default)]
    pub required_modalities: BTreeSet<Modality>,
    /// Kinds of work this turn plausibly needs, as domain names a capability
    /// can declare itself against (`crate::capability::domain` in vak-core).
    ///
    /// Managed as a meet-semilattice ([`DomainSet`]): `All` admits everything,
    /// `Only(names)` narrows to declared domains, and disjoint meets collapse
    /// to `Empty`.
    #[serde(default)]
    pub required_domains: DomainSet,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub spend_ceiling_usd: Option<f64>,
    #[serde(default)]
    pub approval_ceiling: ApprovalCeiling,
    #[serde(default)]
    pub permission_ceiling: PermissionCeiling,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub subagent_budget: Option<usize>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_turns: Option<usize>,
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
    ///
    /// An engagement built from this is byte-for-byte vak's pre-kernel
    /// behaviour, which is what a low-confidence reading must fall back to.
    pub fn unrestricted() -> Self {
        Limits {
            capabilities: CapabilitySlice::All,
            ladder_limit: None,
            required_modalities: BTreeSet::new(),
            required_domains: DomainSet::All,
            spend_ceiling_usd: None,
            approval_ceiling: ApprovalCeiling::AutoApprove,
            permission_ceiling: PermissionCeiling::FullAccess,
            subagent_budget: None,
            max_turns: None,
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
            capabilities: self.capabilities.meet(&other.capabilities),
            ladder_limit: meet_cap(self.ladder_limit, other.ladder_limit),
            required_domains: self.required_domains.meet(&other.required_domains),
            required_modalities: self
                .required_modalities
                .union(&other.required_modalities)
                .copied()
                .collect(),
            spend_ceiling_usd: meet_cap(self.spend_ceiling_usd, other.spend_ceiling_usd),
            approval_ceiling: self.approval_ceiling.meet(other.approval_ceiling),
            permission_ceiling: self.permission_ceiling.meet(other.permission_ceiling),
            subagent_budget: meet_cap(self.subagent_budget, other.subagent_budget),
            max_turns: meet_cap(self.max_turns, other.max_turns),
            min_satisfaction: if other.min_satisfaction.rank() > self.min_satisfaction.rank() {
                other.min_satisfaction
            } else {
                self.min_satisfaction
            },
        }
    }

    /// Whether `self` grants nothing that `baseline` does not already grant.
    ///
    /// This is invariant 1 as a predicate. Every projection in `vak-core`
    /// asserts it in debug builds, and the property test in `tests/` proves it
    /// holds for every reading the resolver can produce.
    pub fn is_at_most(&self, baseline: &Limits) -> bool {
        self.capabilities.is_at_most(&baseline.capabilities)
            && cap_is_at_most(self.ladder_limit, baseline.ladder_limit)
            && baseline
                .required_modalities
                .is_subset(&self.required_modalities)
            && self.required_domains.is_at_most(&baseline.required_domains)
            && cap_is_at_most(self.spend_ceiling_usd, baseline.spend_ceiling_usd)
            && self.approval_ceiling.rank() <= baseline.approval_ceiling.rank()
            && self.permission_ceiling.rank() <= baseline.permission_ceiling.rank()
            && cap_is_at_most(self.subagent_budget, baseline.subagent_budget)
            && cap_is_at_most(self.max_turns, baseline.max_turns)
            && self.min_satisfaction.rank() >= baseline.min_satisfaction.rank()
    }

    /// Human-readable list of what this narrows relative to `baseline`.
    ///
    /// Powers `vak intent explain` and the admin console's engagement diff. A
    /// narrowing nobody can see is a narrowing nobody can debug.
    pub fn diff_from(&self, baseline: &Limits) -> Vec<String> {
        let mut out = Vec::new();
        if self.capabilities != baseline.capabilities {
            match &self.capabilities {
                CapabilitySlice::All => {}
                CapabilitySlice::Only { names } => out.push(format!(
                    "capabilities limited to {} ({})",
                    names.len(),
                    names.iter().cloned().collect::<Vec<_>>().join(", ")
                )),
            }
        }
        if self.required_domains != baseline.required_domains {
            match &self.required_domains {
                DomainSet::All => {}
                DomainSet::Only { names } => out.push(format!(
                    "domains limited to {} ({})",
                    names.len(),
                    names.iter().cloned().collect::<Vec<_>>().join(", ")
                )),
                DomainSet::Empty => out.push("no tool domains admitted".into()),
            }
        }
        if self.ladder_limit != baseline.ladder_limit
            && let Some(limit) = self.ladder_limit
        {
            out.push(format!("route ladder limited to {limit} leg(s)"));
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
        if self.subagent_budget != baseline.subagent_budget
            && let Some(budget) = self.subagent_budget
        {
            out.push(format!("at most {budget} subagent(s)"));
        }
        if self.max_turns != baseline.max_turns
            && let Some(turns) = self.max_turns
        {
            out.push(format!("at most {turns} turn(s)"));
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
                capabilities: CapabilitySlice::only(["read", "grep"]),
                ladder_limit: Some(1),
                spend_ceiling_usd: Some(0.01),
                approval_ceiling: ApprovalCeiling::Ask,
                permission_ceiling: PermissionCeiling::ReadOnly,
                subagent_budget: Some(0),
                max_turns: Some(1),
                min_satisfaction: Satisfaction::Attested,
                ..Limits::unrestricted()
            },
            Limits {
                capabilities: CapabilitySlice::only(["read", "edit", "bash"]),
                spend_ceiling_usd: Some(2.0),
                approval_ceiling: ApprovalCeiling::ApproveSafe,
                min_satisfaction: Satisfaction::Observed,
                ..Limits::unrestricted()
            },
            modal,
        ]
    }

    /// Invariant 1, stated as a lattice law: a meet is below both operands.
    /// Everything else in the kernel relies on this being true.
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
            capabilities: CapabilitySlice::only(["read"]),
            ladder_limit: Some(1),
            spend_ceiling_usd: Some(0.5),
            approval_ceiling: ApprovalCeiling::Ask,
            permission_ceiling: PermissionCeiling::ReadOnly,
            subagent_budget: Some(0),
            max_turns: Some(2),
            min_satisfaction: Satisfaction::Attested,
            required_modalities: BTreeSet::from([Modality::Image]),
        };
        let widened = [
            Limits {
                required_domains: DomainSet::All,
                ..narrow.clone()
            },
            Limits {
                capabilities: CapabilitySlice::All,
                ..narrow.clone()
            },
            Limits {
                ladder_limit: None,
                ..narrow.clone()
            },
            Limits {
                spend_ceiling_usd: Some(5.0),
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
                subagent_budget: Some(4),
                ..narrow.clone()
            },
            Limits {
                max_turns: None,
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
            capabilities: CapabilitySlice::only(["read"]),
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
