//! What the runtime concluded the request is, and how it concluded it.

use std::collections::BTreeSet;

use serde::{Deserialize, Serialize};

use crate::axes::{Act, Attendance, Clarity, Evidence, Horizon, Modality, Stakes};
use crate::signals::Signal;

/// Which resolver tier produced a reading.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Tier {
    /// Stated outright by a caller. Free and absolute.
    Declared,
    /// Deterministic signal extraction. Free and reproducible.
    Signals,
    /// A local model classified it. Cheap and private, not reproducible.
    LocalModel,
    /// A metered provider call classified it. Not reproducible.
    CloudModel,
    /// Nothing reached threshold; the general engagement applies. This is
    /// vak's pre-kernel behaviour and is never a failure.
    General,
}

impl Tier {
    pub fn as_str(self) -> &'static str {
        match self {
            Tier::Declared => "declared",
            Tier::Signals => "signals",
            Tier::LocalModel => "local-model",
            Tier::CloudModel => "cloud-model",
            Tier::General => "general",
        }
    }

    /// Whether replaying the recorded inputs is guaranteed to reproduce this
    /// decision. Model tiers say no, and say so rather than pretending —
    /// the same honesty the routing ledger applies to `Settlement::Unknown`.
    pub fn is_reproducible(self) -> bool {
        matches!(self, Tier::Declared | Tier::Signals | Tier::General)
    }
}

/// How a reading was reached, in enough detail to reconstruct or dispute it.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Provenance {
    pub tier: Tier,
    /// Bumped whenever the lexicon or scoring changes, so a ledger entry can
    /// be interpreted against the rules that actually produced it.
    pub resolver_version: u32,
    #[serde(default)]
    pub signals: Vec<Signal>,
    /// Model that classified, when a model tier ran.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    /// Digest of the classification prompt, so a later change to it is
    /// visible rather than silent.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub prompt_digest: Option<String>,
    /// Mirrors [`Tier::is_reproducible`], recorded explicitly so a reader of
    /// old JSONL need not know today's tier semantics.
    pub reproducible: bool,
    /// Why a paid tier was or was not reached. Present whenever escalation
    /// was considered, so cost is explainable after the fact.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub escalation_note: Option<String>,
}

impl Provenance {
    pub fn new(tier: Tier, resolver_version: u32, signals: Vec<Signal>) -> Self {
        Provenance {
            tier,
            resolver_version,
            signals,
            model: None,
            prompt_digest: None,
            reproducible: tier.is_reproducible(),
            escalation_note: None,
        }
    }
}

/// Per-axis confidence.
///
/// A single scalar conflates independent questions. Capability slicing depends
/// only on `act`; whether to open a commitment depends only on `horizon`; the
/// approval floor depends only on `stakes`. Gating all three on the weakest
/// axis meant an unsignalled horizon — which is most requests, since few say
/// how long they will take — held back tool slicing that the act reading was
/// perfectly confident about.
#[derive(Debug, Clone, Copy, Default, PartialEq, Serialize, Deserialize)]
pub struct Confidences {
    pub act: f64,
    pub horizon: f64,
    pub stakes: f64,
    pub evidence: f64,
}

impl Confidences {
    /// The weakest axis. Used for the all-or-nothing general fallback, where
    /// the question really is "did we understand this request at all".
    pub fn overall(&self) -> f64 {
        self.act
            .min(self.horizon)
            .min(self.stakes)
            .min(self.evidence)
    }
}

/// What the request is, on every axis.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Reading {
    pub act: Act,
    pub horizon: Horizon,
    pub stakes: Stakes,
    pub evidence: Evidence,
    pub clarity: Clarity,
    #[serde(default)]
    pub input_modalities: BTreeSet<Modality>,
    #[serde(default)]
    pub output_modalities: BTreeSet<Modality>,
    pub attendance: Attendance,
    /// Acts that scored close enough to the winner to be part of the same
    /// request rather than rivals to it. The capability slice covers all of
    /// them; everything else (output shape, stop profile) follows `act`.
    #[serde(default)]
    pub alternate_acts: BTreeSet<Act>,
    /// Open-vocabulary subject tags. Used for skill affinity and telemetry
    /// only — deliberately never for control flow, so adding a domain can
    /// never change what the runtime is allowed to do.
    #[serde(default)]
    pub domains: BTreeSet<String>,
    /// [0,1]. Below the configured floor the general engagement applies.
    /// Equal to `axis_confidence.overall()`.
    pub confidence: f64,
    /// Per-axis confidence, so each projection can gate on the axis it
    /// actually depends on.
    #[serde(default)]
    pub axis_confidence: Confidences,
}

impl Default for Reading {
    fn default() -> Self {
        Reading::general()
    }
}

impl Reading {
    /// The reading that assumes nothing.
    ///
    /// Deliberately the *widest* capability posture paired with the *ordinary*
    /// approval posture, because uncertainty must never silently narrow what
    /// the agent can do — a wrongly-sliced turn fails in a way the user
    /// experiences as the agent being broken.
    pub fn general() -> Self {
        Reading {
            act: Act::Answer,
            horizon: Horizon::Turn,
            stakes: Stakes::Reversible,
            evidence: Evidence::None,
            clarity: Clarity::Clear,
            input_modalities: BTreeSet::from([Modality::Text]),
            output_modalities: BTreeSet::from([Modality::Text]),
            attendance: Attendance::Interactive,
            alternate_acts: BTreeSet::new(),
            domains: BTreeSet::new(),
            confidence: 0.0,
            axis_confidence: Confidences::default(),
        }
    }

    /// Whether this reading is confident enough to narrow capability.
    ///
    /// Gated on the `act` axis alone, because that is the only axis the
    /// capability slice is derived from. Two thresholds rather than one: a
    /// provisional reading may still raise an approval floor (getting that
    /// wrong is merely annoying) but may not remove a tool (getting that wrong
    /// breaks the task).
    pub fn may_slice_capabilities(&self, floor: f64) -> bool {
        self.axis_confidence.act >= floor
    }

    /// Whether this reading is confident enough to promote work to a durable
    /// commitment. Gated on `horizon` alone.
    pub fn may_open_commitment(&self, floor: f64) -> bool {
        self.axis_confidence.horizon >= floor
    }

    /// Whether the reading is usable at all.
    pub fn is_actionable(&self, floor: f64) -> bool {
        self.confidence >= floor
    }

    /// One-line summary for logs, chips, and explain views.
    pub fn summary(&self) -> String {
        format!(
            "{}/{}/{}/{} ({}, {:.0}% confident)",
            self.act.as_str(),
            self.horizon.as_str(),
            self.stakes.as_str(),
            self.evidence.as_str(),
            self.attendance.as_str(),
            self.confidence * 100.0
        )
    }

    /// Every act this reading covers: the primary plus any close contenders.
    pub fn acts(&self) -> BTreeSet<Act> {
        let mut acts = self.alternate_acts.clone();
        acts.insert(self.act);
        acts
    }

    /// Modalities a serving model must support to handle this turn at all.
    ///
    /// Plain text and structured data ride the ordinary text interface; the
    /// rest constrain which ladder legs may serve, and an unsatisfiable
    /// requirement is a typed failure rather than a silent degradation.
    pub fn required_modalities(&self) -> BTreeSet<Modality> {
        self.input_modalities
            .union(&self.output_modalities)
            .copied()
            .filter(|m| m.needs_declared_support())
            .collect()
    }
}

/// A reading paired with everything the runtime derived from it.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Intent {
    pub reading: Reading,
    pub engagement: crate::Engagement,
    pub provenance: Provenance,
}

impl Intent {
    /// The intent that changes nothing — vak's behaviour before this kernel.
    pub fn general(resolver_version: u32) -> Self {
        Intent {
            reading: Reading::general(),
            engagement: crate::Engagement::general(),
            provenance: Provenance::new(Tier::General, resolver_version, Vec::new()),
        }
    }

    /// The block of text this intent contributes to the model's context, if
    /// any.
    ///
    /// Model-visible means logged (`AGENTS.md` invariant 1), so whatever this
    /// returns is written into the session ledger verbatim as part of the
    /// intent entry rather than being regenerated at replay time.
    pub fn model_visible(&self) -> Option<String> {
        self.engagement.posture.note.clone()
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    #[test]
    fn model_tiers_are_never_marked_reproducible() {
        for tier in [Tier::LocalModel, Tier::CloudModel] {
            assert!(!tier.is_reproducible());
            assert!(!Provenance::new(tier, 1, Vec::new()).reproducible);
        }
        for tier in [Tier::Declared, Tier::Signals, Tier::General] {
            assert!(tier.is_reproducible());
            assert!(Provenance::new(tier, 1, Vec::new()).reproducible);
        }
    }

    #[test]
    fn the_general_reading_restricts_nothing() {
        let intent = Intent::general(1);
        assert!(
            intent
                .engagement
                .limits
                .is_at_most(&crate::Limits::unrestricted())
        );
        assert_eq!(intent.engagement.limits, crate::Limits::unrestricted());
    }

    #[test]
    fn only_modalities_needing_support_constrain_the_ladder() {
        let mut reading = Reading::general();
        reading.input_modalities.insert(Modality::Data);
        // Text and data both ride the ordinary interface.
        assert!(reading.required_modalities().is_empty());

        reading.input_modalities.insert(Modality::Image);
        assert_eq!(
            reading.required_modalities(),
            BTreeSet::from([Modality::Image])
        );
    }
}
