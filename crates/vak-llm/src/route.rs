//! Frozen-ladder routing primitives (docs/design/27 Phase B).
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

use serde::{Deserialize, Serialize};
use std::collections::HashMap;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RouteLeg {
    pub provider: String,
    pub model: String,
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

/// Laplace-shrunk reliability in [0,1]. Unknown outcomes count as
/// HALF-weight successes: they shrink confidence without punishing
/// direction — billing proves nothing about quality, and an absent verdict
/// is not a failure.
fn reliability(e: &ModelEvidence) -> f64 {
    let trials = e.success + e.failure + e.unknown;
    (e.success as f64 + 0.5 * e.unknown as f64 + 1.0) / (trials as f64 + 2.0)
}

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

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;

    fn leg(p: &str, m: &str) -> RouteLeg {
        RouteLeg {
            provider: p.into(),
            model: m.into(),
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
}
