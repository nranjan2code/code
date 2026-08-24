//! Routing evidence ledger + ladder admission (docs/design/27 Phase B,
//! Phase R upgrades ported from the vakrouter study).
//!
//! Evidence is append-only JSONL; reads apply a TTL and treat outcomes as
//! success / failure / UNKNOWN. Billing without a verdict is UNKNOWN --
//! it shrinks confidence without punishing the model.
//!
//! Beliefs are the session-scoped complement to persisted evidence:
//! domain-weighted doubt (a provider outage says more than one malformed
//! stream) that demotes a leg below fully-trusted peers until ONE success
//! on that leg clears it. Reality outranks priors.

use std::collections::HashMap;
use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};
use std::sync::Mutex;

use serde::{Deserialize, Serialize};
use vak_llm::{EvidenceSnapshot, ModelEvidence, RouteLeg};

const EVIDENCE_TTL_DAYS: u64 = 30;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EvidenceRow {
    pub ts: chrono::DateTime<chrono::Utc>,
    pub provider: String,
    pub model: String,
    /// success | failure | unknown
    pub outcome: String,
    pub latency_ms: u64,
}

fn p50(samples: &mut [u64]) -> Option<u64> {
    if samples.is_empty() {
        return None;
    }
    samples.sort_unstable();
    Some(samples[(samples.len() - 1) / 2])
}

pub struct EvidenceLedger {
    path: PathBuf,
}

impl EvidenceLedger {
    pub fn new(sessions_home: &Path) -> Self {
        EvidenceLedger {
            path: sessions_home.join("routing-evidence.jsonl"),
        }
    }

    pub fn append(&self, row: &EvidenceRow) -> std::io::Result<()> {
        if let Some(parent) = self.path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let line = serde_json::to_string(row)
            .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))?;
        let mut f = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&self.path)?;
        writeln!(f, "{line}")
    }

    /// TTL-filtered snapshot for the ordering function. Corrupt lines are
    /// skipped rather than trusted -- never misranked. Latency is a true
    /// p50 over each leg's samples, not first-seen.
    pub fn snapshot(&self) -> EvidenceSnapshot {
        let cutoff = chrono::Utc::now() - chrono::Duration::days(EVIDENCE_TTL_DAYS as i64);
        let mut by_key: HashMap<(String, String), ModelEvidence> = HashMap::new();
        let mut latencies: HashMap<(String, String), Vec<u64>> = HashMap::new();
        let Ok(f) = std::fs::File::open(&self.path) else {
            return EvidenceSnapshot::default();
        };
        for line in BufReader::new(f).lines().map_while(Result::ok) {
            let Ok(row) = serde_json::from_str::<EvidenceRow>(&line) else {
                continue;
            };
            if row.ts < cutoff {
                continue;
            }
            let key = (row.provider.clone(), row.model.clone());
            let e = by_key.entry(key.clone()).or_default();
            match row.outcome.as_str() {
                "success" => {
                    e.success += 1;
                    latencies.entry(key).or_default().push(row.latency_ms);
                }
                "failure" => e.failure += 1,
                _ => e.unknown += 1,
            }
        }
        for (key, e) in by_key.iter_mut() {
            e.p50_latency_ms = p50(latencies.entry(key.clone()).or_default());
        }
        EvidenceSnapshot { by_key }
    }

    /// Fold a finished run's work receipts into evidence. Attribution is
    /// per attempt: fallback legs record against the leg that actually
    /// served (or failed) -- never against the receipt's final stamp.
    pub fn record_receipts(&self, receipts: &[vak_llm::WorkReceipt]) {
        let now = chrono::Utc::now();
        for r in receipts {
            for a in &r.attempts {
                let outcome = match a.settlement {
                    vak_llm::Settlement::Ok => Some("success"),
                    vak_llm::Settlement::Failed => Some("failure"),
                    vak_llm::Settlement::Unknown => Some("unknown"),
                    vak_llm::Settlement::Cancelled => None, // contributes nothing
                };
                if let Some(outcome) = outcome {
                    let (provider, model) = r.attempt_leg(a);
                    let _ = self.append(&EvidenceRow {
                        ts: now,
                        provider: provider.to_string(),
                        model: model.to_string(),
                        outcome: outcome.into(),
                        latency_ms: a.latency_ms,
                    });
                }
            }
        }
    }
}

/// Doubt weight per failure domain. A dead provider (.6) is strong
/// evidence against its legs; one malformed stream (.3) is weaker; an
/// account-scoped rejection (.05) barely moves the needle. Governance and
/// transport failures (request/network/deadline/unknown) say nothing about
/// answer quality and are ignored.
const DOMAIN_DOUBT: &[(vak_llm::FailureDomain, f64)] = &[
    (vak_llm::FailureDomain::Provider, 0.6),
    (vak_llm::FailureDomain::Model, 0.3),
    (vak_llm::FailureDomain::Account, 0.05),
];

/// Accumulated doubt never fully zeroes a candidate.
pub const BELIEF_FLOOR: f64 = 0.1;

/// Bounded doubt history per leg; oldest observations drop first.
const MAX_BELIEF_OBSERVATIONS: usize = 128;

#[derive(Debug, Clone)]
struct Observation {
    weight: f64,
}

/// Session-scoped domain-weighted doubt per (provider, model) leg.
#[derive(Default)]
pub struct BeliefState {
    inner: Mutex<HashMap<(String, String), Vec<Observation>>>,
}

impl BeliefState {
    pub fn new() -> Self {
        Self::default()
    }

    /// Record one dispatch outcome. Success clears ALL doubt for the leg:
    /// reality outranks priors. Non-evidence domains contribute nothing.
    pub fn record_outcome(
        &self,
        provider: &str,
        model: &str,
        domain: vak_llm::FailureDomain,
        succeeded: bool,
    ) {
        let mut inner = self
            .inner
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let key = (provider.to_string(), model.to_string());
        if succeeded {
            inner.remove(&key);
            return;
        }
        let Some(weight) = DOMAIN_DOUBT
            .iter()
            .find(|(d, _)| *d == domain)
            .map(|(_, w)| *w)
        else {
            return;
        };
        let obs = inner.entry(key).or_default();
        obs.push(Observation { weight });
        if obs.len() > MAX_BELIEF_OBSERVATIONS {
            obs.drain(0..obs.len() - MAX_BELIEF_OBSERVATIONS);
        }
    }

    /// Multiplier in [BELIEF_FLOOR, 1] plus the reasons behind it.
    pub fn multiplier(&self, provider: &str, model: &str) -> (f64, Vec<String>) {
        let inner = self
            .inner
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let mut mult = 1.0_f64;
        let mut reasons = Vec::new();
        if let Some(obs) = inner.get(&(provider.to_string(), model.to_string())) {
            for o in obs {
                mult *= 1.0 - 0.25 * o.weight;
                reasons.push(format!("doubt w={:.2}", o.weight));
            }
        }
        if mult < BELIEF_FLOOR {
            mult = BELIEF_FLOOR;
        }
        (mult, reasons)
    }

    /// Deterministic snapshot for the pure ordering function.
    pub fn snapshot(&self) -> BeliefSnapshot {
        let inner = self
            .inner
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        BeliefSnapshot {
            multipliers: inner
                .iter()
                .map(|(k, obs)| {
                    let mut m = 1.0_f64;
                    for o in obs {
                        m *= 1.0 - 0.25 * o.weight;
                    }
                    (k.clone(), m.max(BELIEF_FLOOR))
                })
                .collect(),
        }
    }
}

/// Immutable view of belief multipliers keyed by (provider, model).
#[derive(Debug, Clone, Default)]
pub struct BeliefSnapshot {
    pub multipliers: HashMap<(String, String), f64>,
}

impl BeliefSnapshot {
    pub fn get(&self, provider: &str, model: &str) -> f64 {
        self.multipliers
            .get(&(provider.to_string(), model.to_string()))
            .copied()
            .unwrap_or(1.0)
    }
}

/// The admission-time routing decision frozen into the contract header.
#[derive(Debug, Clone)]
pub struct RoutePlan {
    pub ladder: Vec<RouteLeg>,
    /// "utility" | "balanced" | "quality-critical".
    pub objective: String,
    /// Freeze-time warnings (thin chain, dominant failure domain,
    /// unreachable cross-model fallbacks).
    pub annotations: Vec<String>,
}

/// Admission-time ordering wrapper over the versioned pure function.
pub fn order(
    candidates: Vec<RouteLeg>,
    snap: &EvidenceSnapshot,
    cost_of: impl Fn(&str) -> Option<f64>,
) -> Vec<RouteLeg> {
    vak_llm::route::order_ladder_v1(candidates, snap, false, cost_of)
}

/// Assemble the frozen ladder from ranked candidates (Phase R diversity
/// constraints, ported from the vakrouter chain study).
///
/// The operator-selected primary NEVER loses its head position — v2
/// ordering decides the FALLBACK order, never whether the user's explicit
/// choice serves first. Remaining seats cap per provider at
/// ceil(max_total/3) so one failure domain cannot own every slot.
/// Annotations are freeze-time warnings for traces/TUI, never model input.
pub fn assemble_ladder(
    primary: &RouteLeg,
    ranked: Vec<RouteLeg>,
    max_total: usize,
    cross_model_requested: bool,
) -> (Vec<RouteLeg>, Vec<String>) {
    let max_total = max_total.max(1);
    let seats_per_provider = max_total.div_ceil(3).max(1);
    let mut legs = vec![primary.clone()];
    let mut counts: HashMap<String, usize> = HashMap::new();
    counts.insert(primary.provider.clone(), 1);

    for leg in ranked {
        if legs.len() >= max_total {
            break;
        }
        if leg == *primary || leg.provider == primary.provider && leg.model == primary.model {
            continue;
        }
        let seats = counts.entry(leg.provider.clone()).or_insert(0);
        if *seats >= seats_per_provider {
            continue;
        }
        *seats += 1;
        legs.push(leg);
    }

    let mut annotations = Vec::new();
    if cross_model_requested && legs.iter().all(|l| l.model == primary.model) {
        annotations.push(
            "cross-model fallback configured but none of the allowed models are \
             reachable by warm discovery"
                .to_string(),
        );
    }
    if legs.len() <= 1 {
        annotations.push("single point of failure: no executable fallback candidates".to_string());
    } else {
        let len = legs.len();
        if let Some((dominant, max_seats)) = counts
            .iter()
            .max_by_key(|(_, v)| **v)
            .filter(|(_, v)| **v > len.div_ceil(2) && len > 2)
        {
            annotations.push(format!(
                "{dominant} holds {max_seats} of {len} ladder seats — one dominant failure domain"
            ));
        }
    }
    (legs, annotations)
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn snapshot_respects_ttl_and_skips_corrupt_lines() {
        let dir = tempdir().unwrap();
        let ledger = EvidenceLedger::new(dir.path());
        let old = chrono::Utc::now() - chrono::Duration::days(40);
        ledger
            .append(&EvidenceRow {
                ts: old,
                provider: "p".into(),
                model: "m".into(),
                outcome: "failure".into(),
                latency_ms: 5,
            })
            .unwrap();
        ledger
            .append(&EvidenceRow {
                ts: chrono::Utc::now(),
                provider: "p".into(),
                model: "m".into(),
                outcome: "success".into(),
                latency_ms: 900,
            })
            .unwrap();
        ledger
            .append(&EvidenceRow {
                ts: chrono::Utc::now(),
                provider: "p".into(),
                model: "m".into(),
                outcome: "success".into(),
                latency_ms: 42,
            })
            .unwrap();
        let mut file_path = dir.path().join("routing-evidence.jsonl");
        let mut existing = std::fs::read_to_string(&file_path).unwrap();
        existing.push_str("not json\n");
        std::fs::write(&mut file_path, existing).unwrap();

        let snap = ledger.snapshot();
        let e = snap.get("p", "m");
        assert_eq!(e.success, 2);
        assert_eq!(e.failure, 0, "40-day-old rows must be TTL-dropped");
        assert_eq!(
            e.p50_latency_ms,
            Some(42),
            "p50 must be the median of samples, not first-seen"
        );
    }

    #[test]
    fn record_receipts_attributes_per_attempt_and_maps_settlements() {
        let dir = tempdir().unwrap();
        let ledger = EvidenceLedger::new(dir.path());
        let mut r = vak_llm::WorkReceipt::new(vak_llm::WorkPurpose::Execute, "openai", "gpt-x");
        r.record(
            vak_llm::AttemptReason::Initial,
            vak_llm::FailureDomain::Network,
            vak_llm::Settlement::Unknown,
            10,
            None,
            None,
        );
        // Fall over to another leg; the winning attempt must attribute to
        // THAT leg, not the receipt's final stamp.
        r.stamp_leg("anthropic", "claude-x");
        r.record(
            vak_llm::AttemptReason::RouteFallback,
            vak_llm::FailureDomain::Unknown,
            vak_llm::Settlement::Ok,
            20,
            None,
            None,
        );
        ledger.record_receipts(std::slice::from_ref(&r));
        let snap = ledger.snapshot();
        assert_eq!(
            snap.get("openai", "gpt-x").unknown,
            1,
            "failed leg keeps its own attribution"
        );
        assert_eq!(
            snap.get("anthropic", "claude-x").success,
            1,
            "winning leg attributes to itself"
        );
        assert_eq!(snap.get("", "gpt-x").unknown, 0, "no empty-provider rows");
    }

    #[test]
    fn beliefs_demote_on_doubt_and_clear_on_success() {
        let b = BeliefState::new();
        b.record_outcome("p", "flaky", vak_llm::FailureDomain::Provider, false);
        let (mult, _reasons) = b.multiplier("p", "flaky");
        assert!(mult < 1.0, "provider failure must create doubt");
        b.record_outcome("p", "flaky", vak_llm::FailureDomain::Network, false);
        let (unchanged, _) = b.multiplier("p", "flaky");
        assert_eq!(
            unchanged, mult,
            "governance-domain failures must NOT add doubt"
        );
        let (clean, _) = b.multiplier("q", "other");
        assert_eq!(clean, 1.0);
        b.record_outcome("p", "flaky", vak_llm::FailureDomain::Unknown, true);
        let (cleared, _) = b.multiplier("p", "flaky");
        assert_eq!(cleared, 1.0, "one success clears all doubt");
    }

    #[test]
    fn belief_multiplier_floors_but_never_zeroes() {
        let b = BeliefState::new();
        for _ in 0..40 {
            b.record_outcome("p", "dead", vak_llm::FailureDomain::Provider, false);
        }
        let (mult, _) = b.multiplier("p", "dead");
        assert_eq!(mult, BELIEF_FLOOR);
        let snap = b.snapshot();
        assert_eq!(snap.get("p", "dead"), BELIEF_FLOOR);
        assert_eq!(snap.get("p", "absent"), 1.0);
    }

    #[test]
    fn assemble_pins_primary_head_and_caps_provider_seats() {
        let primary = RouteLeg {
            provider: "anthropic".into(),
            model: "claude-x".into(),
        };
        let mk = |m: &str| RouteLeg {
            provider: "openai".into(),
            model: m.into(),
        };
        let ranked = vec![
            mk("gpt-a"),
            mk("gpt-b"),
            RouteLeg {
                provider: "google".into(),
                model: "gemini-a".into(),
            },
        ];
        // max_total 4 → seats/provider = 2; openai can hold at most two
        // fallback seats alongside the primary.
        let (legs, annotations) = assemble_ladder(&primary, ranked, 4, true);
        assert_eq!(legs[0], primary, "primary must stay at the head");
        assert_eq!(legs.len(), 4);
        assert!(
            !annotations
                .iter()
                .any(|a| a.contains("dominant failure domain")),
            "two of four seats is exactly half — not dominant"
        );

        // A wide ladder lets one provider hold three fallback seats; that
        // IS a dominant failure domain worth surfacing.
        let (legs, annotations) = assemble_ladder(
            &primary,
            vec![mk("gpt-a"), mk("gpt-b"), mk("gpt-c")],
            7,
            false,
        );
        assert_eq!(legs.len(), 4);
        assert!(
            annotations
                .iter()
                .any(|a| a.contains("openai holds 3 of 4") && a.contains("dominant")),
            "three of four seats on one provider must warn, got {annotations:?}"
        );
    }

    #[test]
    fn assemble_annotates_thin_chain_and_unreachable_cross_model() {
        let primary = RouteLeg {
            provider: "anthropic".into(),
            model: "claude-x".into(),
        };
        let (legs, annotations) = assemble_ladder(&primary, vec![], 4, false);
        assert_eq!(legs.len(), 1);
        assert!(annotations.iter().any(|a| a.contains("single point")));

        let (_, annotations) = assemble_ladder(
            &primary,
            vec![RouteLeg {
                provider: "anthropic".into(),
                model: "claude-y".into(),
            }],
            4,
            true,
        );
        assert!(
            !annotations.iter().any(|a| a.contains("cross-model")),
            "a reachable alternate model means cross-model worked"
        );
        let (_, annotations) = assemble_ladder(&primary, vec![], 4, true);
        assert!(
            annotations.iter().any(|a| a.contains("cross-model")),
            "configured-but-unreachable fallbacks must be surfaced"
        );
    }

    #[test]
    fn assemble_drops_duplicate_primary() {
        let primary = RouteLeg {
            provider: "anthropic".into(),
            model: "claude-x".into(),
        };
        let ranked = vec![
            primary.clone(),
            RouteLeg {
                provider: "openai".into(),
                model: "gpt".into(),
            },
        ];
        let (legs, _) = assemble_ladder(&primary, ranked, 4, false);
        assert_eq!(legs.len(), 2);
    }
}
