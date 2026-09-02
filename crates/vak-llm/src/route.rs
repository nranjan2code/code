//! Frozen-ladder routing primitives (docs/design/15-reliability.md).
//!
//! The candidate SET is chosen by constraint satisfaction upstream (key
//! present, model actually discovered on that provider); THIS module owns
//! the versioned ORDERING function over that set. Ordering is pure and
//! deterministic: identical inputs yield an identical ladder, so freezing
//! the ladder into the frozen contract reproduces the decision exactly.
//!
//! Epistemics: outcomes are success/failure/**unknown**. Unknowns shrink
//! confidence (denominator) without punishing direction — billing proves
//! nothing about answer quality. Absent evidence is a neutral prior,
//! never zero.
//!
//! Phase R (vakrouter adoption): v2 adds request-demand scoring (the
//! difficulty of the work selects utility/balanced/quality-critical
//! ordering instead of hardcoded model-name bands), belief demotion
//! (domain-weighted doubt ranks a doubted leg below fully-trusted peers),
//! and price honesty (known pricing outranks unknown before cost math).
//! Model ids are NEVER hardcoded here; quality hints are caller-declared
//! configuration, not baked-in knowledge.

use serde::{Deserialize, Serialize};
use std::collections::HashMap;

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub struct RouteLeg {
    pub provider: String,
    pub model: String,
    /// Non-secret credential fingerprint selected at admission. `None` is
    /// the legacy/default credential for callers that do not use a pool.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub credential_id: Option<String>,
}

/// One provider/model observation aggregate.
#[derive(Debug, Clone, Default)]
pub struct ModelEvidence {
    pub success: u64,
    pub failure: u64,
    /// Settled-billing-without-verdict (or transport ambiguity). Shrinks
    /// confidence but does not count against the model.
    pub unknown: u64,
    pub p50_latency_ms: Option<u64>,
}

#[derive(Debug, Clone, Default)]
pub struct EvidenceSnapshot {
    pub by_key: HashMap<(String, String), ModelEvidence>,
}

impl EvidenceSnapshot {
    pub fn get(&self, provider: &str, model: &str) -> ModelEvidence {
        self.by_key
            .get(&(provider.to_string(), model.to_string()))
            .cloned()
            .unwrap_or_default()
    }
}

/// Immutable belief-multiplier view for the pure ordering function.
/// Multipliers live in [BELIEF_FLOOR, 1]; 1.0 means fully trusted.
#[derive(Debug, Clone, Default)]
pub struct BeliefMap {
    pub multipliers: HashMap<(String, String), f64>,
}

/// Accumulated doubt never fully zeroes a candidate.
pub const BELIEF_FLOOR: f64 = 0.1;

impl BeliefMap {
    pub fn get(&self, provider: &str, model: &str) -> f64 {
        self.multipliers
            .get(&(provider.to_string(), model.to_string()))
            .copied()
            .unwrap_or(1.0)
    }
}

/// Laplace-shrunk reliability in [0,1]. Unknown outcomes count as
/// HALF-weight successes: they shrink confidence without punishing
/// direction — billing proves nothing about quality, and an absent verdict
/// is not a failure.
fn reliability(e: &ModelEvidence) -> f64 {
    let trials = e.success + e.failure + e.unknown;
    (e.success as f64 + 0.5 * e.unknown as f64 + 1.0) / (trials as f64 + 2.0)
}

/// LEGACY band table, retained ONLY so replaying a v1-frozen contract
/// reproduces its original ordering byte-for-byte. New admissions use
/// order_ladder_v2, which takes caller-declared quality hints instead —
/// model ids must not be hardcoded into routing knowledge.
const FRONTIER_BANDS: [&str; 6] = ["opus", "gpt-5", "o3", "pro", "claude-4", "qwen3-max"];

fn frontier_band(model: &str) -> bool {
    let m = model.to_ascii_lowercase();
    FRONTIER_BANDS.iter().any(|b| m.contains(b))
}

fn latency_penalty(ms: Option<u64>) -> f64 {
    ms.unwrap_or(0).min(30_000) as f64 / 10_000.0 // ≤3.0
}

/// Versioned ordering function. Cheap-first by default; `quality_first`
/// promotes the frontier band for planning/verification purposes.
/// `cost_usd_per_mtok` supplies the caller's pricing view (output rate);
/// unpriced models sit at a middling rank — absent is UNKNOWN, never free.
///
/// v1 score = reliability × band / (1 + cost/weight + latency); ties break
/// deterministically on (cost, provider, model).
pub fn order_ladder_v1(
    mut candidates: Vec<RouteLeg>,
    evidence: &EvidenceSnapshot,
    quality_first: bool,
    cost_usd_per_mtok: impl Fn(&str) -> Option<f64>,
) -> Vec<RouteLeg> {
    const UNPRICED_RANK: f64 = 60.0;
    candidates.sort_by(|a, b| {
        let ea = evidence.get(&a.provider, &a.model);
        let eb = evidence.get(&b.provider, &b.model);
        let ra = reliability(&ea);
        let rb = reliability(&eb);
        let ca = cost_usd_per_mtok(&a.model).unwrap_or(UNPRICED_RANK);
        let cb = cost_usd_per_mtok(&b.model).unwrap_or(UNPRICED_RANK);
        let la = latency_penalty(ea.p50_latency_ms);
        let lb = latency_penalty(eb.p50_latency_ms);
        let score = |r: f64, c: f64, l: f64, m: &str| -> f64 {
            let boost = if quality_first && frontier_band(m) {
                1.5
            } else {
                1.0
            };
            let cost_weight = if quality_first { 1000.0 } else { 100.0 };
            (r * boost) / (1.0 + c / cost_weight + l)
        };
        let sa = score(ra, ca, la, &a.model);
        let sb = score(rb, cb, lb, &b.model);
        sb.partial_cmp(&sa)
            .unwrap_or(std::cmp::Ordering::Equal)
            .then(ca.partial_cmp(&cb).unwrap_or(std::cmp::Ordering::Equal))
            .then(a.provider.cmp(&b.provider))
            .then(a.model.cmp(&b.model))
    });
    candidates
}

/// How hard the work is (Phase R demand scoring). Weights mirror the
/// vakrouter study: context and output dominate because those are what a
/// small model physically cannot do. Unavailable factors read as 0 —
/// never fabricated.
#[derive(Debug, Clone, Copy, Default)]
pub struct DemandInput {
    pub estimated_input_tokens: u64,
    pub output_budget_tokens: u64,
    pub tool_count: usize,
    pub structured_output: bool,
    pub reasoning_required: bool,
    pub evidence_required: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DemandBand {
    Low,
    Moderate,
    High,
}

impl DemandBand {
    pub fn as_str(&self) -> &'static str {
        match self {
            DemandBand::Low => "low",
            DemandBand::Moderate => "moderate",
            DemandBand::High => "high",
        }
    }
}

#[derive(Debug, Clone, Copy)]
pub struct Demand {
    /// [0, 1] weighted saturation score.
    pub score: f64,
    pub band: DemandBand,
}

const CONTEXT_SATURATION_TOKENS: f64 = 32_768.0;
const TOOL_BREADTH_SATURATION: f64 = 12.0;
const OUTPUT_SATURATION_TOKENS: f64 = 8_192.0;

fn saturate(value: f64, anchor: f64) -> f64 {
    if value <= 0.0 || !value.is_finite() {
        return 0.0;
    }
    (value / anchor).min(1.0)
}

/// Score request difficulty from saturating anchors. Unknown context reads
/// as moderate (0.5), never zero — absent is not free.
pub fn score_demand(input: DemandInput) -> Demand {
    const CONTEXT_W: f64 = 0.30;
    const OUTPUT_W: f64 = 0.25;
    const REASONING_W: f64 = 0.15;
    const TOOLS_W: f64 = 0.12;
    const STRUCTURED_W: f64 = 0.10;
    const EVIDENCE_W: f64 = 0.08;

    let ctx = if input.estimated_input_tokens == 0 {
        0.5
    } else {
        saturate(
            input.estimated_input_tokens as f64,
            CONTEXT_SATURATION_TOKENS,
        )
    };
    let out = saturate(input.output_budget_tokens as f64, OUTPUT_SATURATION_TOKENS);
    let reasoning = if input.reasoning_required { 1.0 } else { 0.0 };
    let tools = saturate(input.tool_count as f64, TOOL_BREADTH_SATURATION);
    let structured = if input.structured_output { 1.0 } else { 0.0 };
    let evidence = if input.evidence_required { 1.0 } else { 0.0 };

    let score = CONTEXT_W * ctx
        + OUTPUT_W * out
        + REASONING_W * reasoning
        + TOOLS_W * tools
        + STRUCTURED_W * structured
        + EVIDENCE_W * evidence;

    let band = if score < 0.25 {
        DemandBand::Low
    } else if score < 0.6 {
        DemandBand::Moderate
    } else {
        DemandBand::High
    };
    Demand { score, band }
}

/// What the ladder optimizes for. `auto` resolves from the demand band:
/// high → quality-critical, low → utility, moderate → balanced.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum QualityObjective {
    Utility,
    Balanced,
    QualityCritical,
}

impl QualityObjective {
    pub fn as_str(&self) -> &'static str {
        match self {
            QualityObjective::Utility => "utility",
            QualityObjective::Balanced => "balanced",
            QualityObjective::QualityCritical => "quality-critical",
        }
    }

    /// Resolve the explicit config override ("auto" or unknown → None)
    /// against the demand-derived default.
    pub fn resolve(explicit: Option<&str>, band: DemandBand) -> Self {
        match explicit {
            Some("utility") => Self::Utility,
            Some("balanced") => Self::Balanced,
            Some("quality-critical") | Some("quality_critical") => Self::QualityCritical,
            _ => match band {
                DemandBand::High => Self::QualityCritical,
                DemandBand::Low => Self::Utility,
                DemandBand::Moderate => Self::Balanced,
            },
        }
    }
}

/// Versioned ordering function v2 (Phase R). Differences from v1:
/// - objective-driven instead of hardcoded frontier bands; `hints` are
///   CALLER-declared model-id substrings treated as frontier tier.
/// - belief demotion: a doubted leg ranks below every fully-trusted peer,
///   whatever its cost advantage — retries are a cost too.
/// - price honesty: known pricing outranks unknown before cost math.
///
/// Ordering is a total order with deterministic tiebreaks
/// (cost, provider, model), so identical inputs freeze identically.
pub fn order_ladder_v2(
    mut candidates: Vec<RouteLeg>,
    evidence: &EvidenceSnapshot,
    beliefs: &BeliefMap,
    objective: QualityObjective,
    hints: &[String],
    cost_usd_per_mtok: impl Fn(&str) -> Option<f64>,
) -> Vec<RouteLeg> {
    const UNPRICED_RANK: f64 = 60.0;
    let hint_boost = |model: &str| -> f64 {
        let m = model.to_ascii_lowercase();
        if hints.iter().any(|h| m.contains(&h.to_ascii_lowercase())) {
            1.5
        } else {
            1.0
        }
    };
    let quality = |leg: &RouteLeg| -> f64 {
        let e = evidence.get(&leg.provider, &leg.model);
        let r = reliability(&e);
        let c = cost_usd_per_mtok(&leg.model).unwrap_or(UNPRICED_RANK);
        let l = latency_penalty(e.p50_latency_ms);
        // Cost weight by objective: quality-critical is deliberately
        // cost-insensitive; balanced still respects spend.
        let cost_weight = match objective {
            QualityObjective::Utility => 100.0,
            QualityObjective::Balanced => 100.0,
            QualityObjective::QualityCritical => 1000.0,
        };
        (r * hint_boost(&leg.model)) / (1.0 + c / cost_weight + l)
    };
    candidates.sort_by(|a, b| {
        let ma = beliefs.get(&a.provider, &a.model);
        let mb = beliefs.get(&b.provider, &b.model);
        // Belief demotion precedes everything: doubted legs lose to trusted
        // peers regardless of price advantage.
        let a_doubted = ma < 1.0;
        let b_doubted = mb < 1.0;
        if a_doubted != b_doubted {
            // Trusted candidate sorts first.
            return if a_doubted {
                std::cmp::Ordering::Greater
            } else {
                std::cmp::Ordering::Less
            };
        }
        let ka = cost_usd_per_mtok(&a.model);
        let kb = cost_usd_per_mtok(&b.model);
        // Price honesty before cost comparison: known beats unknown.
        if ka.is_some() != kb.is_some() {
            return ka.is_some().cmp(&kb.is_some()).reverse();
        }
        let ca = ka.unwrap_or(UNPRICED_RANK);
        let cb = kb.unwrap_or(UNPRICED_RANK);
        let qa = quality(a) * ma;
        let qb = quality(b) * mb;
        match objective {
            // Utility: cost first, quality breaks ties.
            QualityObjective::Utility => ca
                .partial_cmp(&cb)
                .unwrap_or(std::cmp::Ordering::Equal)
                .then(qb.partial_cmp(&qa).unwrap_or(std::cmp::Ordering::Equal)),
            // Balanced / quality-critical: quality first, cost breaks ties.
            _ => qb
                .partial_cmp(&qa)
                .unwrap_or(std::cmp::Ordering::Equal)
                .then(ca.partial_cmp(&cb).unwrap_or(std::cmp::Ordering::Equal)),
        }
        .then(ca.partial_cmp(&cb).unwrap_or(std::cmp::Ordering::Equal))
        .then(a.provider.cmp(&b.provider))
        .then(a.model.cmp(&b.model))
    });
    candidates
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;

    fn leg(p: &str, m: &str) -> RouteLeg {
        RouteLeg {
            provider: p.into(),
            model: m.into(),
            credential_id: None,
        }
    }

    #[test]
    fn empty_evidence_orders_cheap_first_deterministically() {
        let cands = vec![
            leg("p", "claude-opus-x"),
            leg("p", "some-haiku"),
            leg("q", "gpt-4o-mini"),
        ];
        let costs = |m: &str| -> Option<f64> {
            if m.contains("opus") {
                Some(75.0)
            } else if m.contains("haiku") {
                Some(4.0)
            } else {
                Some(0.6)
            }
        };
        let a = order_ladder_v1(cands.clone(), &Default::default(), false, costs);
        assert_eq!(a[0].model, "gpt-4o-mini");
        assert!(a.last().unwrap().model.contains("opus"));
        let b = order_ladder_v1(
            vec![
                leg("p", "claude-opus-x"),
                leg("p", "some-haiku"),
                leg("q", "gpt-4o-mini"),
            ],
            &Default::default(),
            false,
            costs,
        );
        assert_eq!(a, b, "pure function must be deterministic");
    }

    #[test]
    fn failures_push_a_model_down_unknowns_only_shrink() {
        let mut ev = EvidenceSnapshot::default();
        ev.by_key.insert(
            ("p".into(), "m-fail".into()),
            ModelEvidence {
                failure: 9,
                ..Default::default()
            },
        );
        ev.by_key.insert(
            ("p".into(), "m-unknown".into()),
            ModelEvidence {
                unknown: 9,
                ..Default::default()
            },
        );
        ev.by_key.insert(
            ("p".into(), "m-good".into()),
            ModelEvidence {
                success: 9,
                ..Default::default()
            },
        );
        let cands = vec![
            leg("p", "m-fail"),
            leg("p", "m-unknown"),
            leg("p", "m-good"),
        ];
        let ordered = order_ladder_v1(cands, &ev, false, |_m: &str| Some(3.0));
        assert_eq!(ordered[0].model, "m-good");
        assert_eq!(
            ordered[1].model, "m-unknown",
            "unknown must outrank failure"
        );
        assert_eq!(ordered[2].model, "m-fail");
    }

    #[test]
    fn quality_first_promotes_frontier_band() {
        let cands = vec![leg("p", "cheap-haiku"), leg("q", "big-claude-opus")];
        let cheap_first = order_ladder_v1(cands.clone(), &Default::default(), false, |_m: &str| {
            Some(3.0)
        });
        assert_eq!(cheap_first[0].model, "cheap-haiku");
        let quality_first = order_ladder_v1(cands, &Default::default(), true, |_m: &str| Some(3.0));
        assert_eq!(quality_first[0].model, "big-claude-opus");
    }

    #[test]
    fn high_latency_degrades_rank() {
        let mut ev = EvidenceSnapshot::default();
        ev.by_key.insert(
            ("p".into(), "slow".into()),
            ModelEvidence {
                success: 5,
                p50_latency_ms: Some(29_000),
                ..Default::default()
            },
        );
        ev.by_key.insert(
            ("p".into(), "quick".into()),
            ModelEvidence {
                success: 5,
                p50_latency_ms: Some(300),
                ..Default::default()
            },
        );
        let ordered = order_ladder_v1(
            vec![leg("p", "slow"), leg("p", "quick")],
            &ev,
            false,
            |_m: &str| Some(3.0),
        );
        assert_eq!(ordered[0].model, "quick");
    }

    fn leg2(p: &str, m: &str) -> RouteLeg {
        leg(p, m)
    }

    #[test]
    fn credential_identity_round_trips_and_legacy_route_defaults_empty() {
        let scoped = RouteLeg {
            provider: "openrouter".into(),
            model: "model-a".into(),
            credential_id: Some("deadbeef".into()),
        };
        let encoded = serde_json::to_string(&scoped).unwrap();
        let decoded: RouteLeg = serde_json::from_str(&encoded).unwrap();
        assert_eq!(decoded, scoped);

        let legacy: RouteLeg =
            serde_json::from_str(r#"{"provider":"ollama","model":"local-model"}"#).unwrap();
        assert_eq!(legacy.credential_id, None);
    }

    #[test]
    fn v2_demand_scores_bands() {
        let light = score_demand(DemandInput {
            estimated_input_tokens: 1_000,
            output_budget_tokens: 1_000,
            tool_count: 0,
            ..Default::default()
        });
        assert_eq!(light.band, DemandBand::Low);
        let heavy = score_demand(DemandInput {
            estimated_input_tokens: 60_000,
            output_budget_tokens: 16_000,
            tool_count: 20,
            structured_output: true,
            reasoning_required: true,
            evidence_required: true,
        });
        assert_eq!(heavy.band, DemandBand::High);
        // Unknown context reads as moderate, never zero.
        let unknown_ctx = score_demand(DemandInput {
            estimated_input_tokens: 0,
            output_budget_tokens: 4_000,
            tool_count: 6,
            ..Default::default()
        });
        assert_eq!(unknown_ctx.band, DemandBand::Moderate);
    }

    #[test]
    fn v2_resolves_objective_from_band_and_override() {
        assert_eq!(
            QualityObjective::resolve(None, DemandBand::High),
            QualityObjective::QualityCritical
        );
        assert_eq!(
            QualityObjective::resolve(None, DemandBand::Low),
            QualityObjective::Utility
        );
        assert_eq!(
            QualityObjective::resolve(Some("utility"), DemandBand::High),
            QualityObjective::Utility,
            "explicit override beats the band"
        );
    }

    #[test]
    fn v2_utility_orders_strictly_cheap_first() {
        let cands = vec![leg2("p", "big-expensive"), leg2("q", "cheap-mini")];
        let ordered = order_ladder_v2(
            cands,
            &Default::default(),
            &BeliefMap::default(),
            QualityObjective::Utility,
            &[],
            |m: &str| {
                if m.contains("expensive") {
                    Some(75.0)
                } else {
                    Some(1.0)
                }
            },
        );
        assert_eq!(ordered[0].model, "cheap-mini");
    }

    #[test]
    fn v2_quality_critical_promotes_hinted_models_over_cost() {
        let cands = vec![leg2("p", "cheap-haiku"), leg2("q", "frontier-opus-xl")];
        let ordered = order_ladder_v2(
            cands,
            &Default::default(),
            &BeliefMap::default(),
            QualityObjective::QualityCritical,
            &["opus".to_string()],
            |_m: &str| Some(3.0),
        );
        assert_eq!(ordered[0].model, "frontier-opus-xl");
    }

    #[test]
    fn v2_doubted_leg_ranks_below_trusted_peers_even_when_cheaper() {
        let mut beliefs = BeliefMap::default();
        beliefs.multipliers.insert(("p".into(), "m".into()), 0.5);
        let cands = vec![leg2("p", "m"), leg2("p", "trusted")];
        for objective in [
            QualityObjective::Utility,
            QualityObjective::Balanced,
            QualityObjective::QualityCritical,
        ] {
            let ordered = order_ladder_v2(
                cands.clone(),
                &Default::default(),
                &beliefs,
                objective,
                &[],
                |_m: &str| Some(3.0),
            );
            assert_eq!(ordered[0].model, "trusted", "{objective:?}");
        }
    }

    #[test]
    fn v2_known_pricing_outranks_unknown_before_cost_math() {
        let cands = vec![leg2("p", "priced-cheap"), leg2("q", "unpriced")];
        let ordered = order_ladder_v2(
            cands,
            &Default::default(),
            &BeliefMap::default(),
            QualityObjective::Utility,
            &[],
            |m: &str| if m == "unpriced" { None } else { Some(0.5) },
        );
        assert_eq!(ordered[0].model, "priced-cheap");
    }

    #[test]
    fn v2_is_deterministic() {
        let mk = || vec![leg2("a", "z"), leg2("b", "y"), leg2("c", "x")];
        let one = order_ladder_v2(
            mk(),
            &Default::default(),
            &BeliefMap::default(),
            QualityObjective::Balanced,
            &[],
            |_| None,
        );
        let two = order_ladder_v2(
            mk(),
            &Default::default(),
            &BeliefMap::default(),
            QualityObjective::Balanced,
            &[],
            |_| None,
        );
        assert_eq!(one, two);
    }
}
