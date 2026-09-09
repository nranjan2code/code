//! The resolution cascade.
//!
//! Tiers run cheapest first and stop as soon as confidence clears the bar:
//!
//! | Tier | Cost | Reproducible |
//! |---|---|---|
//! | [`Tier::Declared`] — a caller said so | free | yes |
//! | [`Tier::Signals`] — deterministic extraction | free | yes |
//! | [`Tier::LocalModel`] / [`Tier::CloudModel`] | metered | no |
//! | [`Tier::General`] — nothing reached the bar | free | yes |
//!
//! # Why this module does not dispatch
//!
//! Escalating to a model is a provider dispatch, and in vak a dispatch means a
//! work receipt, a spend-gate admission, a frozen ladder leg, a watchdog and a
//! cancellation token. All of that machinery lives in `vak-core`, so this
//! module decides **whether** a paid tier is warranted and hands back a
//! [`Resolution::Escalate`] carrying the partial reading; the host performs
//! the call and folds the answer back in with [`apply_classification`].
//!
//! That split also keeps the kernel synchronous and free of provider
//! dependencies, which is what lets the whole decision layer be unit-tested
//! without a network or a model.

use std::collections::BTreeSet;

use serde::{Deserialize, Serialize};

use crate::authority::Authority;
use crate::axes::{Act, Attendance, Clarity, Evidence, Horizon, Modality, Stakes};
use crate::engage::derive;
use crate::reading::{Confidences, Intent, Provenance, Reading, Tier};
use crate::signals::{Extraction, Request, Signal, SignalKind, extract};

/// Bumped whenever the lexicon or scoring changes, so a ledger entry can be
/// read against the rules that actually produced it.
pub const RESOLVER_VERSION: u32 = 1;

/// Thresholds and switches for the cascade.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ResolverConfig {
    /// Master switch. Off resolves everything to the general engagement,
    /// which is vak's pre-kernel behaviour.
    pub enabled: bool,
    /// At or above this, a reading is trusted enough to narrow capability.
    pub accept_confidence: f64,
    /// At or above this but below `accept_confidence`, the reading is
    /// *provisional*: risk-raising narrowings apply, capability narrowing does
    /// not. Getting an approval floor wrong is an annoyance; removing a tool
    /// the task needed looks like the agent is broken.
    pub provisional_confidence: f64,
    /// Whether capability slicing is permitted at all.
    pub slice_capabilities: bool,
    /// Whether a below-threshold reading may escalate to a model tier.
    pub allow_escalation: bool,
}

impl Default for ResolverConfig {
    fn default() -> Self {
        ResolverConfig {
            enabled: true,
            accept_confidence: 0.75,
            provisional_confidence: 0.45,
            slice_capabilities: true,
            allow_escalation: true,
        }
    }
}

/// A caller stating the reading outright — tier 0.
///
/// Every field is optional; whatever is set overrides the corresponding axis
/// and whatever is not falls through to the signal tier. This is what a
/// `--act` flag, a flow's declared intent, a pinned channel policy, or a
/// parent handing an engagement to a subagent all produce.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Declared {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub act: Option<Act>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub horizon: Option<Horizon>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub stakes: Option<Stakes>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub evidence: Option<Evidence>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub clarity: Option<Clarity>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub attendance: Option<Attendance>,
    #[serde(default)]
    pub domains: BTreeSet<String>,
}

impl Declared {
    pub fn is_empty(&self) -> bool {
        self.act.is_none()
            && self.horizon.is_none()
            && self.stakes.is_none()
            && self.evidence.is_none()
            && self.clarity.is_none()
            && self.attendance.is_none()
            && self.domains.is_empty()
    }

    /// How much of the reading was stated rather than inferred, in [0,1].
    fn coverage(&self) -> f64 {
        let stated = [
            self.act.is_some(),
            self.horizon.is_some(),
            self.stakes.is_some(),
            self.evidence.is_some(),
        ]
        .into_iter()
        .filter(|x| *x)
        .count();
        stated as f64 / 4.0
    }
}

/// What a model tier is asked to return. Strict JSON, every field optional so
/// a partial answer is usable rather than discarded.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Classification {
    #[serde(default)]
    pub act: Option<String>,
    #[serde(default)]
    pub horizon: Option<String>,
    #[serde(default)]
    pub stakes: Option<String>,
    #[serde(default)]
    pub evidence: Option<String>,
    #[serde(default)]
    pub clarity: Option<String>,
    #[serde(default)]
    pub domains: Vec<String>,
    #[serde(default)]
    pub confidence: Option<f64>,
}

/// The outcome of the free tiers.
#[derive(Debug, Clone, PartialEq)]
pub enum Resolution {
    /// Good enough to act on. No paid tier needed.
    Settled(Intent),
    /// Below threshold and escalation is permitted. `partial` is safe to use
    /// as-is if the host declines to spend — it is never worse than the
    /// general engagement.
    Escalate {
        partial: Intent,
        /// Why escalation was recommended, recorded so cost is explainable.
        reason: String,
    },
}

impl Resolution {
    /// The intent to use if no further tier runs.
    pub fn intent(self) -> Intent {
        match self {
            Resolution::Settled(intent) => intent,
            Resolution::Escalate { partial, .. } => partial,
        }
    }

    pub fn peek(&self) -> &Intent {
        match self {
            Resolution::Settled(intent) => intent,
            Resolution::Escalate { partial, .. } => partial,
        }
    }
}

/// Run tiers 0 and 1.
///
/// Pure: identical inputs always produce an identical result, which is what
/// makes a recorded decision reconstructable.
pub fn resolve(
    request: &Request<'_>,
    declared: &Declared,
    authority: &Authority,
    config: &ResolverConfig,
) -> Resolution {
    if !config.enabled {
        return Resolution::Settled(Intent::general(RESOLVER_VERSION));
    }

    let extraction = extract(request);
    let mut signals = extraction.signals.clone();

    // --- tier 0: declared ----------------------------------------------
    if !declared.is_empty() {
        signals.push(Signal {
            kind: SignalKind::Declared,
            name: "declared".into(),
            weight: 1.0,
            detail: format!(
                "caller stated {:.0}% of the axes",
                declared.coverage() * 100.0
            ),
        });
    }

    // --- tier 1: signals -----------------------------------------------
    let (reading, weakest) = assemble(&extraction, declared, request);

    let tier = if declared.coverage() >= 1.0 {
        Tier::Declared
    } else {
        Tier::Signals
    };
    let mut provenance = Provenance::new(tier, RESOLVER_VERSION, signals);

    // Capability slicing needs the higher bar, and gates on the `act` axis
    // alone — that is the only axis the slice is derived from, and most
    // requests never say anything about their horizon.
    let may_slice =
        config.slice_capabilities && reading.may_slice_capabilities(config.accept_confidence);

    if reading.confidence < config.provisional_confidence {
        // Nothing reached the bar. Fall back to the general engagement, which
        // is exactly today's behaviour, and say why.
        provenance.tier = Tier::General;
        provenance.reproducible = true;
        provenance.escalation_note = Some(format!(
            "confidence {:.2} below floor {:.2}; general engagement applied",
            reading.confidence, config.provisional_confidence
        ));
        let general = Intent {
            reading: Reading {
                confidence: reading.confidence,
                axis_confidence: reading.axis_confidence,
                ..Reading::general()
            },
            engagement: crate::Engagement::general(),
            provenance,
        };
        return if config.allow_escalation {
            Resolution::Escalate {
                partial: general,
                reason: format!("weak reading on {weakest}"),
            }
        } else {
            Resolution::Settled(general)
        };
    }

    let engagement = derive(&reading, authority, may_slice);
    let provisional = !reading.may_slice_capabilities(config.accept_confidence);
    if provisional {
        provenance.escalation_note = Some(format!(
            "provisional at {:.2}: risk narrowing applied, capability slicing withheld",
            reading.confidence
        ));
    }
    let intent = Intent {
        reading,
        engagement,
        provenance,
    };

    if provisional && config.allow_escalation {
        Resolution::Escalate {
            partial: intent,
            reason: format!("provisional reading, weakest on {weakest}"),
        }
    } else {
        Resolution::Settled(intent)
    }
}

/// Turn votes into a reading, honouring anything the caller declared.
///
/// Returns the reading and the name of the axis that scored worst, which is
/// what an escalation prompt should focus a model's attention on.
fn assemble(
    extraction: &Extraction,
    declared: &Declared,
    request: &Request<'_>,
) -> (Reading, &'static str) {
    let mut confidences: Vec<(&'static str, f64)> = Vec::new();

    // A declared axis is certain by construction; an inferred one carries the
    // vote margin. Defaults are chosen to be the *safe* value on risk axes and
    // the *ordinary* value elsewhere.
    let (act, act_confidence) = match declared.act {
        Some(act) => (act, 1.0),
        None => match extraction.act.winner() {
            Some((act, confidence)) => (act, confidence),
            None => (Act::Answer, 0.0),
        },
    };
    // Confidence in a capability *slice* is confidence that the slice covers
    // the request — not confidence about which single act won. A request that
    // is genuinely both a modification and a verification is not ambiguous
    // once both toolsets are included.
    //
    // So: acts within half the winner's weight join the slice, and confidence
    // is how much of the total act evidence that set accounts for, damped by
    // how much evidence there was at all.
    const ACT_BAND: f64 = 0.5;
    let mut alternate_acts: BTreeSet<Act> = BTreeSet::new();
    let mut act_confidence = act_confidence;
    if declared.act.is_none() {
        let ranked = extraction.act.ranked();
        let contenders = extraction.act.contenders(ACT_BAND);
        if !contenders.is_empty() {
            alternate_acts.extend(contenders.iter().copied().filter(|a| *a != act));
            let total: f64 = ranked.iter().map(|(_, weight)| weight).sum();
            let covered: f64 = ranked
                .iter()
                .filter(|(value, _)| contenders.contains(value))
                .map(|(_, weight)| weight)
                .sum();
            let coverage = if total > 0.0 { covered / total } else { 0.0 };
            let best = ranked.first().map(|(_, w)| *w).unwrap_or(0.0);
            let mass = (best / 1.5).min(1.0);
            act_confidence = (coverage * 0.6 + mass * 0.4).clamp(0.0, 1.0);
        }
    }
    confidences.push(("act", act_confidence));

    let (horizon, horizon_confidence) = match declared.horizon {
        Some(horizon) => (horizon, 1.0),
        None => match extraction.horizon.winner() {
            Some((horizon, confidence)) => (horizon, confidence),
            // Genuinely uncertain: a request with no recurrence or enumeration
            // markers could be a one-liner or a week of work, and nothing in
            // the text distinguishes them.
            None => (Horizon::Turn, 0.5),
        },
    };
    confidences.push(("horizon", horizon_confidence));

    // Stakes default upward from the act rather than to a fixed value: an
    // unrecognised request to `deploy` should not read as inert just because
    // no stakes word appeared next to it.
    //
    // Environment-derived stakes (a dirty working tree) apply only to acts
    // that touch something. A repository mid-edit does not make answering a
    // question risky, and folding that vote in unconditionally made every
    // greeting in a working repo read as `reversible`.
    let mut stakes_votes = extraction.stakes.clone();
    if act.is_effectful() {
        for (value, weight) in extraction.stakes_from_environment.ranked() {
            stakes_votes.add(value, weight);
        }
    }
    let (stakes, stakes_confidence) = match declared.stakes {
        Some(stakes) => (stakes, 1.0),
        None => match stakes_votes.winner() {
            Some((stakes, confidence)) => {
                let floor = implied_stakes(act);
                if floor.rank() > stakes.rank() {
                    (floor, confidence.min(0.6))
                } else {
                    (stakes, confidence)
                }
            }
            // No stakes language at all. The act's own floor is then the
            // answer, and it is only as trustworthy as the act reading that
            // produced it — so it inherits that confidence rather than
            // inventing one.
            None => (implied_stakes(act), act_confidence.max(0.5)),
        },
    };
    confidences.push(("stakes", stakes_confidence));

    // Absence is informative here, unlike on the other axes: a request with no
    // citation, verification or sign-off language genuinely does have no
    // special evidentiary standard, so silence reads as a confident
    // `Evidence::None` rather than a coin flip. Treating it as uncertain held
    // the whole reading's confidence down and suppressed capability slicing on
    // exactly the ordinary requests that benefit from it most.
    let (evidence, evidence_confidence) = match declared.evidence {
        Some(evidence) => (evidence, 1.0),
        None => match extraction.evidence.winner() {
            Some((evidence, confidence)) => (evidence, confidence),
            None => (Evidence::None, 0.85),
        },
    };
    confidences.push(("evidence", evidence_confidence));

    let (clarity, _) = match declared.clarity {
        Some(clarity) => (clarity, 1.0),
        None => match extraction.clarity.winner() {
            Some((clarity, confidence)) => (clarity, confidence),
            None => (Clarity::Clear, 0.5),
        },
    };

    let attendance = declared.attendance.unwrap_or(extraction.attendance);

    let mut domains: BTreeSet<String> = extraction.domains.iter().cloned().collect();
    domains.extend(declared.domains.iter().cloned());

    let axis_confidence = Confidences {
        act: act_confidence,
        horizon: horizon_confidence,
        stakes: stakes_confidence,
        evidence: evidence_confidence,
    };
    // The overall figure is the weakest axis, not an average: averaging would
    // let a confident `act` hide a coin flip on `stakes`. It gates only the
    // all-or-nothing fallback to the general engagement; each projection gates
    // on the specific axis it depends on.
    let confidence = axis_confidence.overall();
    let weakest = confidences
        .iter()
        .min_by(|a, b| a.1.partial_cmp(&b.1).unwrap_or(std::cmp::Ordering::Equal))
        .map(|(name, _)| *name)
        .unwrap_or("act");

    let input_modalities: BTreeSet<Modality> =
        extraction.input_modalities.iter().copied().collect();
    let output_modalities: BTreeSet<Modality> =
        extraction.output_modalities.iter().copied().collect();

    let _ = request;
    (
        Reading {
            act,
            horizon,
            stakes,
            evidence,
            clarity,
            input_modalities,
            output_modalities,
            attendance,
            alternate_acts,
            domains,
            confidence: if confidence.is_finite() {
                confidence
            } else {
                0.0
            },
            axis_confidence,
        },
        weakest,
    )
}

/// The lowest stakes an act can honestly carry when nothing else is known.
///
/// This is a floor, never a cap: a stakes word in the request can raise it,
/// and nothing here lowers what the request itself implies.
fn implied_stakes(act: Act) -> Stakes {
    match act {
        Act::Converse | Act::Answer | Act::Locate | Act::Analyze => Stakes::Inert,
        Act::Author | Act::Modify | Act::Verify | Act::Orchestrate => Stakes::Reversible,
        // Reaching outside the workspace or rewriting the agent's own rules is
        // never assumed to be cheap.
        Act::Operate | Act::Govern => Stakes::Irreversible,
    }
}

/// Fold a model tier's answer into a partial reading.
///
/// Only axes the model actually returned are overwritten, and the result is
/// always marked non-reproducible with the model id and prompt digest that
/// produced it. A malformed or empty classification leaves the partial
/// untouched, so a bad model response degrades to the free tiers rather than
/// to nonsense.
pub fn apply_classification(
    partial: Intent,
    classification: &Classification,
    model: &str,
    prompt_digest: &str,
    authority: &Authority,
    config: &ResolverConfig,
    cloud: bool,
) -> Intent {
    let mut reading = partial.reading.clone();
    let mut applied = 0usize;

    if let Some(act) = classification.act.as_deref().and_then(Act::parse) {
        reading.act = act;
        applied += 1;
    }
    if let Some(horizon) = classification.horizon.as_deref().and_then(Horizon::parse) {
        reading.horizon = horizon;
        applied += 1;
    }
    if let Some(stakes) = classification.stakes.as_deref().and_then(Stakes::parse) {
        // A model may raise stakes freely; lowering them below what the act
        // implies is not accepted, so a classifier cannot talk the runtime out
        // of caution it had arrived at deterministically.
        let floor = implied_stakes(reading.act);
        reading.stakes = if stakes.rank() >= floor.rank() {
            stakes
        } else {
            floor
        };
        applied += 1;
    }
    if let Some(evidence) = classification.evidence.as_deref().and_then(Evidence::parse) {
        reading.evidence = evidence;
        applied += 1;
    }
    if let Some(clarity) = classification.clarity.as_deref().and_then(Clarity::parse) {
        reading.clarity = clarity;
        applied += 1;
    }
    // A classifier may omit stakes, but it may never lower the deterministic
    // floor implied by the final act it selected.
    let act_floor = implied_stakes(reading.act);
    if reading.stakes.rank() < act_floor.rank() {
        reading.stakes = act_floor;
    }
    reading
        .domains
        .extend(classification.domains.iter().cloned());

    if applied == 0 {
        // Nothing usable came back. Keep the free-tier result and say so.
        let mut provenance = partial.provenance;
        provenance.escalation_note = Some(format!(
            "{} returned no usable axes; free-tier reading retained",
            model
        ));
        return Intent {
            reading: partial.reading,
            engagement: partial.engagement,
            provenance,
        };
    }

    let stated = classification.confidence.unwrap_or(0.8).clamp(0.0, 1.0);
    // A classifier's confidence covers every axis it actually answered;
    // axes it left alone keep whatever the free tiers concluded.
    if classification.act.is_some() {
        reading.axis_confidence.act = stated;
    }
    if classification.horizon.is_some() {
        reading.axis_confidence.horizon = stated;
    }
    if classification.stakes.is_some() {
        reading.axis_confidence.stakes = stated;
    }
    if classification.evidence.is_some() {
        reading.axis_confidence.evidence = stated;
    }
    reading.confidence = reading.axis_confidence.overall();

    let may_slice =
        config.slice_capabilities && reading.may_slice_capabilities(config.accept_confidence);
    let engagement = derive(&reading, authority, may_slice);

    let mut provenance = partial.provenance;
    provenance.tier = if cloud {
        Tier::CloudModel
    } else {
        Tier::LocalModel
    };
    provenance.reproducible = false;
    provenance.model = Some(model.to_string());
    provenance.prompt_digest = Some(prompt_digest.to_string());
    provenance.escalation_note = Some(format!("{applied} axis/axes set by {model}"));

    Intent {
        reading,
        engagement,
        provenance,
    }
}

/// Digest of a classification prompt, so a later change to it is visible in
/// old ledger entries rather than silent.
pub fn prompt_digest(prompt: &str) -> String {
    use sha2::{Digest, Sha256};
    format!("{:x}", Sha256::digest(prompt.as_bytes()))
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::*;
    use crate::signals::{Surface, WorkspaceFacts};

    fn request<'a>(text: &'a str) -> Request<'a> {
        Request {
            text,
            surface: Surface::Cli,
            attachments: &[],
            workspace: WorkspaceFacts::default(),
            history: Default::default(),
            attendance_override: None,
        }
    }

    fn resolve_text(text: &str) -> Resolution {
        resolve(
            &request(text),
            &Declared::default(),
            &Authority::default(),
            &ResolverConfig::default(),
        )
    }

    #[test]
    fn resolution_is_deterministic() {
        let a = resolve_text("refactor the parser");
        let b = resolve_text("refactor the parser");
        assert_eq!(a, b);
    }

    #[test]
    fn disabling_the_kernel_reproduces_the_general_engagement() {
        let config = ResolverConfig {
            enabled: false,
            ..ResolverConfig::default()
        };
        let resolution = resolve(
            &request("deploy everything to production right now"),
            &Declared::default(),
            &Authority::default(),
            &config,
        );
        let intent = resolution.intent();
        assert_eq!(intent.engagement, crate::Engagement::general());
        assert_eq!(intent.provenance.tier, Tier::General);
    }

    #[test]
    fn a_fully_declared_reading_is_tier_zero_and_reproducible() {
        let declared = Declared {
            act: Some(Act::Modify),
            horizon: Some(Horizon::Session),
            stakes: Some(Stakes::Reversible),
            evidence: Some(Evidence::Verified),
            ..Declared::default()
        };
        let resolution = resolve(
            &request("whatever"),
            &declared,
            &Authority::default(),
            &ResolverConfig::default(),
        );
        let intent = resolution.intent();
        assert_eq!(intent.provenance.tier, Tier::Declared);
        assert!(intent.provenance.reproducible);
        assert_eq!(intent.reading.act, Act::Modify);
        assert_eq!(intent.reading.evidence, Evidence::Verified);
    }

    /// Confidence is the weakest axis, not the average — otherwise a
    /// confident verb hides a coin-flip on stakes.
    #[test]
    fn confidence_tracks_the_weakest_axis() {
        let resolution = resolve_text("refactor the parser");
        let intent = resolution.peek();
        assert!(intent.reading.confidence <= 1.0);
        let declared = Declared {
            act: Some(Act::Modify),
            horizon: Some(Horizon::Turn),
            stakes: Some(Stakes::Reversible),
            evidence: Some(Evidence::None),
            ..Declared::default()
        };
        let full = resolve(
            &request("refactor the parser"),
            &declared,
            &Authority::default(),
            &ResolverConfig::default(),
        );
        assert!(full.peek().reading.confidence > intent.reading.confidence);
    }

    #[test]
    fn unknown_input_falls_back_to_general_and_recommends_escalation() {
        let resolution = resolve_text("zorble the frobnicator immediately");
        match resolution {
            Resolution::Escalate { partial, reason } => {
                assert_eq!(partial.provenance.tier, Tier::General);
                assert_eq!(partial.engagement, crate::Engagement::general());
                assert!(!reason.is_empty());
            }
            Resolution::Settled(intent) => {
                // Acceptable only if it genuinely reached the bar.
                assert!(intent.reading.confidence >= 0.45);
            }
        }
    }

    /// The single most important safety property of the paid tier: a model
    /// cannot talk the runtime out of caution it already arrived at.
    #[test]
    fn a_classifier_may_raise_stakes_but_never_lower_them_below_the_act_floor() {
        let partial = resolve_text("deploy the service").intent();
        let downplayed = Classification {
            act: Some("operate".into()),
            stakes: Some("inert".into()),
            confidence: Some(0.99),
            ..Classification::default()
        };
        let intent = apply_classification(
            partial,
            &downplayed,
            "cheap-model",
            "digest",
            &Authority::default(),
            &ResolverConfig::default(),
            true,
        );
        assert_eq!(intent.reading.stakes, Stakes::Irreversible);
    }

    #[test]
    fn a_model_tier_is_recorded_as_non_reproducible_with_its_digest() {
        let partial = resolve_text("something unclear").intent();
        let intent = apply_classification(
            partial,
            &Classification {
                act: Some("analyze".into()),
                confidence: Some(0.9),
                ..Classification::default()
            },
            "local-model",
            "abc123",
            &Authority::default(),
            &ResolverConfig::default(),
            false,
        );
        assert_eq!(intent.provenance.tier, Tier::LocalModel);
        assert!(!intent.provenance.reproducible);
        assert_eq!(intent.provenance.model.as_deref(), Some("local-model"));
        assert_eq!(intent.provenance.prompt_digest.as_deref(), Some("abc123"));
    }

    #[test]
    fn an_empty_classification_leaves_the_free_tier_result_intact() {
        let partial = resolve_text("refactor the parser").intent();
        let before = partial.clone();
        let after = apply_classification(
            partial,
            &Classification::default(),
            "flaky-model",
            "digest",
            &Authority::default(),
            &ResolverConfig::default(),
            true,
        );
        assert_eq!(after.reading, before.reading);
        assert_eq!(after.engagement, before.engagement);
    }

    #[test]
    fn operate_and_govern_are_never_assumed_cheap() {
        assert_eq!(implied_stakes(Act::Operate), Stakes::Irreversible);
        assert_eq!(implied_stakes(Act::Govern), Stakes::Irreversible);
        assert_eq!(implied_stakes(Act::Answer), Stakes::Inert);
    }

    /// Removing a tool the task needed looks to a user like the agent is
    /// broken, so slicing only happens above the acceptance bar. Below it the
    /// full admitted set survives.
    #[test]
    fn provisional_readings_withhold_capability_slicing() {
        let config = ResolverConfig {
            accept_confidence: 0.99,
            provisional_confidence: 0.0,
            ..ResolverConfig::default()
        };
        // Two acts far enough apart that the slice cannot cover both, so the
        // reading is genuinely uncertain about what this turn needs.
        let resolution = resolve(
            &request("explain the deploy script and then deploy it"),
            &Declared::default(),
            &Authority::default(),
            &config,
        );
        assert_eq!(
            resolution.peek().engagement.limits.required_domains,
            std::collections::BTreeSet::new(),
            "a provisional reading must not narrow what the turn can reach"
        );
    }

    /// And above the bar it does slice, or the whole mechanism is decorative.
    #[test]
    fn a_confident_reading_does_slice() {
        let resolution = resolve(
            &request("hello"),
            &Declared::default(),
            &Authority::default(),
            &ResolverConfig::default(),
        );
        assert!(
            !resolution
                .peek()
                .engagement
                .limits
                .required_domains
                .is_empty(),
            "a confident reading must express what the turn needs"
        );
    }
}
