//! Routing evidence ledger + ladder admission (docs/design/15-reliability.md,
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
use vak_llm::{EvidenceSnapshot, ModelEvidence, ModelRef, RouteLeg};

const EVIDENCE_TTL_DAYS: u64 = 30;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EvidenceRow {
    pub ts: chrono::DateTime<chrono::Utc>,
    pub provider: String,
    pub model: String,
    /// success | failure | unknown
    pub outcome: String,
    pub latency_ms: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub trace: Option<vak_session::trace::TraceKey>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub actor: Option<vak_session::ids::PrincipalId>,
}

vak_session::impl_traced!(EvidenceRow, "evidence_row");

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
            path: vak_config::scope::AgentScope::new(sessions_home).routing_evidence(),
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
    pub fn record_receipts(
        &self,
        receipts: &[vak_llm::WorkReceipt],
        trace: Option<&vak_session::trace::TraceKey>,
    ) {
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
                        trace: trace.cloned(),
                        actor: trace.and_then(|t| t.actor),
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

/// One service's model list as the ladder planner sees it: fetched inside
/// the discovery TTL, with a credential that still resolves.
#[derive(Debug, Clone)]
pub struct WarmCatalogue {
    pub provider: String,
    pub credential_id: String,
    pub models: Vec<String>,
}

fn leg_on(catalogue: &WarmCatalogue, model: &str, needs_tools: bool) -> RouteLeg {
    RouteLeg {
        provider: catalogue.provider.clone(),
        model: model.to_string(),
        dialect: vak_llm::EndpointDialect::for_provider(&catalogue.provider, needs_tools),
        credential_id: Some(catalogue.credential_id.clone()),
    }
}

/// Legs that serve the primary's own model somewhere else, in catalogue
/// order; the caller ranks them.
///
/// An id is evidence of identity only within one account: the same id on
/// another key for the primary's provider, or on a provider name that
/// shares the primary's credential (one account, two wire protocols).
/// Across services one model has a different id at each and a matching id
/// proves nothing, so a leg there joins only as a member of a confirmed
/// `[route] same_model` group.
pub fn stand_in_legs(
    primary: &RouteLeg,
    catalogues: &[WarmCatalogue],
    groups: &[Vec<ModelRef>],
    needs_tools: bool,
) -> Vec<RouteLeg> {
    let mates = vak_llm::model_identity::group_mates(groups, &primary.provider, &primary.model);
    let mut legs: Vec<RouteLeg> = Vec::new();
    for catalogue in catalogues {
        let same_account = catalogue.provider == primary.provider
            || primary.credential_id.as_deref() == Some(catalogue.credential_id.as_str());
        let own_id = same_account.then_some(primary.model.as_str());
        let mate_ids = mates
            .iter()
            .filter(|mate| mate.provider == catalogue.provider)
            .map(|mate| mate.model.as_str());
        for model in own_id.into_iter().chain(mate_ids) {
            if !catalogue.models.iter().any(|listed| listed == model) {
                continue;
            }
            let leg = leg_on(catalogue, model, needs_tools);
            let is_primary = leg.provider == primary.provider
                && leg.model == primary.model
                && leg.credential_id == primary.credential_id;
            if !is_primary && !legs.contains(&leg) {
                legs.push(leg);
            }
        }
    }
    legs
}

/// Legs for the other models the operator allowed as backups
/// (`[route] fallback_models`): exact ids, on every service whose warm
/// catalogue lists them.
pub fn alternate_legs(
    primary: &RouteLeg,
    catalogues: &[WarmCatalogue],
    allowed: &[String],
    needs_tools: bool,
) -> Vec<RouteLeg> {
    let mut legs = Vec::new();
    for catalogue in catalogues {
        for model in &catalogue.models {
            if model == &primary.model || !allowed.contains(model) {
                continue;
            }
            let leg = leg_on(catalogue, model, needs_tools);
            if !legs.contains(&leg) {
                legs.push(leg);
            }
        }
    }
    legs
}

/// Assemble the ladder from ranked candidates (Phase R diversity
/// constraints, ported from the vakrouter chain study).
///
/// The operator-selected primary NEVER loses its head position — v2
/// ordering decides the FALLBACK order, never whether the user's explicit
/// choice serves first. The same model somewhere else is a like-for-like
/// swap while another model changes behaviour, so stand-ins are seated
/// before alternates, each tier in its ranked order. When an alternate is
/// reachable and there are at least two fallback seats, the last seat is
/// kept for it: every copy of one model can share an upstream outage, and a
/// different model is the leg that survives one. A seat an alternate cannot
/// take goes back to the stand-ins. Remaining seats cap per provider at
/// ceil(max_total/3) so one failure domain cannot own every slot.
/// Annotations are freeze-time warnings for traces/TUI, never model input.
pub fn assemble_ladder(
    primary: &RouteLeg,
    stand_ins: Vec<RouteLeg>,
    alternates: Vec<RouteLeg>,
    max_total: usize,
    cross_model_requested: bool,
) -> (Vec<RouteLeg>, Vec<String>) {
    let max_total = max_total.max(1);
    let seats_per_provider = max_total.div_ceil(3).max(1);
    let alternates_unreachable = cross_model_requested && alternates.is_empty();
    let stand_in_limit = if !alternates.is_empty() && max_total >= 3 {
        max_total - 1
    } else {
        max_total
    };
    let mut legs = vec![primary.clone()];
    let mut counts: HashMap<String, usize> = HashMap::new();
    counts.insert(primary.provider.clone(), 1);
    let mut seat = |leg: RouteLeg, legs: &mut Vec<RouteLeg>| {
        if leg == *primary {
            return;
        }
        let seats = counts.entry(leg.provider.clone()).or_insert(0);
        let independent_credential = leg.provider == primary.provider
            && leg.credential_id.is_some()
            && leg.credential_id != primary.credential_id;
        if *seats >= seats_per_provider && !independent_credential {
            return;
        }
        *seats += 1;
        legs.push(leg);
    };

    let mut stand_ins = stand_ins.into_iter();
    while legs.len() < stand_in_limit {
        let Some(leg) = stand_ins.next() else {
            break;
        };
        seat(leg, &mut legs);
    }
    for leg in alternates.into_iter().chain(stand_ins) {
        if legs.len() >= max_total {
            break;
        }
        seat(leg, &mut legs);
    }

    let mut annotations = Vec::new();
    if alternates_unreachable {
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
                trace: None,
                actor: None,
            })
            .unwrap();
        ledger
            .append(&EvidenceRow {
                ts: chrono::Utc::now(),
                provider: "p".into(),
                model: "m".into(),
                outcome: "success".into(),
                latency_ms: 900,
                trace: None,
                actor: None,
            })
            .unwrap();
        ledger
            .append(&EvidenceRow {
                ts: chrono::Utc::now(),
                provider: "p".into(),
                model: "m".into(),
                outcome: "success".into(),
                latency_ms: 42,
                trace: None,
                actor: None,
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
        ledger.record_receipts(std::slice::from_ref(&r), None);
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
            dialect: vak_llm::EndpointDialect::AnthropicMessages,
            credential_id: None,
        };
        let mk = |m: &str| RouteLeg {
            provider: "openai".into(),
            model: m.into(),
            dialect: vak_llm::EndpointDialect::Responses,
            credential_id: None,
        };
        let ranked = vec![
            mk("gpt-a"),
            mk("gpt-b"),
            RouteLeg {
                provider: "google".into(),
                model: "gemini-a".into(),
                dialect: vak_llm::EndpointDialect::GoogleGenerateContent,
                credential_id: None,
            },
        ];
        // max_total 4 → seats/provider = 2; openai can hold at most two
        // fallback seats alongside the primary.
        let (legs, annotations) = assemble_ladder(&primary, Vec::new(), ranked, 4, true);
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
            Vec::new(),
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
            dialect: vak_llm::EndpointDialect::AnthropicMessages,
            credential_id: None,
        };
        let (legs, annotations) = assemble_ladder(&primary, vec![], vec![], 4, false);
        assert_eq!(legs.len(), 1);
        assert!(annotations.iter().any(|a| a.contains("single point")));

        let (_, annotations) = assemble_ladder(
            &primary,
            vec![],
            vec![RouteLeg {
                provider: "anthropic".into(),
                model: "claude-y".into(),
                dialect: vak_llm::EndpointDialect::AnthropicMessages,
                credential_id: None,
            }],
            4,
            true,
        );
        assert!(
            !annotations.iter().any(|a| a.contains("cross-model")),
            "a reachable alternate model means cross-model worked"
        );
        let (_, annotations) = assemble_ladder(&primary, vec![], vec![], 4, true);
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
            dialect: vak_llm::EndpointDialect::AnthropicMessages,
            credential_id: None,
        };
        let ranked = vec![
            primary.clone(),
            RouteLeg {
                provider: "openai".into(),
                model: "gpt".into(),
                dialect: vak_llm::EndpointDialect::Responses,
                credential_id: None,
            },
        ];
        let (legs, _) = assemble_ladder(&primary, Vec::new(), ranked, 4, false);
        assert_eq!(legs.len(), 2);
    }

    #[test]
    fn assemble_keeps_same_model_on_a_distinct_credential() {
        let primary = RouteLeg {
            provider: "openrouter".into(),
            model: "shared-model".into(),
            dialect: vak_llm::EndpointDialect::Responses,
            credential_id: Some("key-a".into()),
        };
        let alternate = RouteLeg {
            credential_id: Some("key-b".into()),
            ..primary.clone()
        };
        let (legs, _) = assemble_ladder(&primary, vec![alternate], Vec::new(), 3, false);
        assert_eq!(legs.len(), 2);
        assert_eq!(legs[1].credential_id.as_deref(), Some("key-b"));
    }

    fn leg(provider: &str, model: &str, credential: &str) -> RouteLeg {
        RouteLeg {
            provider: provider.into(),
            model: model.into(),
            dialect: vak_llm::EndpointDialect::for_provider(provider, true),
            credential_id: Some(credential.into()),
        }
    }

    fn catalogue(provider: &str, credential: &str, models: &[&str]) -> WarmCatalogue {
        WarmCatalogue {
            provider: provider.into(),
            credential_id: credential.into(),
            models: models.iter().map(|m| m.to_string()).collect(),
        }
    }

    #[test]
    fn a_shared_id_stands_in_only_within_one_account() {
        let primary = leg("openai", "gpt-x", "acct-o");
        let catalogues = [
            catalogue("openai", "acct-o", &["gpt-x"]),
            catalogue("openai", "acct-o2", &["gpt-x"]),
            catalogue("openai-responses", "acct-o", &["gpt-x"]),
            catalogue("zen", "acct-z", &["gpt-x"]),
            catalogue("openrouter", "acct-r", &["vendor/gpt-x"]),
        ];
        assert_eq!(
            stand_in_legs(&primary, &catalogues, &[], true),
            vec![
                leg("openai", "gpt-x", "acct-o2"),
                leg("openai-responses", "gpt-x", "acct-o"),
            ],
            "another key, or another wire over the same key, is the same account; \
             an equal id at another service proves nothing"
        );

        let groups = vak_llm::model_identity::merge_groups([vec![
            ModelRef::parse("openai/gpt-x").unwrap(),
            ModelRef::parse("openrouter/vendor/gpt-x").unwrap(),
            ModelRef::parse("zen/gpt-x").unwrap(),
        ]]);
        let legs = stand_in_legs(&primary, &catalogues, &groups, true);
        assert!(legs.contains(&leg("openrouter", "vendor/gpt-x", "acct-r")));
        assert!(legs.contains(&leg("zen", "gpt-x", "acct-z")));
        assert!(
            !legs
                .iter()
                .any(|l| l.credential_id.as_deref() == Some("acct-o") && l.provider == "openai"),
            "the primary itself is never its own stand-in"
        );
    }

    #[test]
    fn alternates_are_allowed_ids_other_than_the_primary() {
        let primary = leg("openai", "gpt-x", "acct-o");
        let catalogues = [
            catalogue("openai", "acct-o", &["gpt-x", "gpt-y", "gpt-z"]),
            catalogue("zen", "acct-z", &["gpt-y", "gpt-x"]),
        ];
        let allowed = vec!["gpt-y".to_string(), "gpt-x".to_string()];
        assert_eq!(
            alternate_legs(&primary, &catalogues, &allowed, true),
            vec![
                leg("openai", "gpt-y", "acct-o"),
                leg("zen", "gpt-y", "acct-z")
            ]
        );
    }

    #[test]
    fn stand_ins_are_seated_before_any_alternate() {
        let primary = leg("anthropic", "model-a", "acct-a");
        let stand_in = leg("openrouter", "vendor/model-a", "acct-r");
        let alternate = leg("google", "model-b", "acct-g");
        let (legs, annotations) = assemble_ladder(
            &primary,
            vec![stand_in.clone()],
            vec![alternate.clone()],
            4,
            true,
        );
        assert_eq!(legs, vec![primary.clone(), stand_in, alternate.clone()]);
        assert!(
            !annotations.iter().any(|a| a.contains("cross-model")),
            "{annotations:?}"
        );

        let full = vec![
            leg("openrouter", "vendor/model-a", "acct-r"),
            leg("bedrock", "us.vendor.model-a-v1:0", "acct-b"),
            leg("zen", "model-a", "acct-z"),
        ];
        let (legs, _) = assemble_ladder(&primary, full.clone(), vec![alternate.clone()], 4, true);
        assert_eq!(
            legs,
            vec![
                primary.clone(),
                full[0].clone(),
                full[1].clone(),
                alternate.clone()
            ],
            "the last seat is kept for a different model"
        );

        let (legs, annotations) =
            assemble_ladder(&primary, full.clone(), vec![alternate.clone()], 2, true);
        assert_eq!(
            legs,
            vec![primary.clone(), full[0].clone()],
            "one fallback seat goes to the same model"
        );
        assert!(
            !annotations.iter().any(|a| a.contains("none of")),
            "a reachable alternate that lost its seat is not unreachable: {annotations:?}"
        );
    }

    #[test]
    fn a_seat_an_alternate_cannot_take_goes_back_to_the_stand_ins() {
        let primary = leg("anthropic", "model-a", "acct-a");
        let stand_ins = vec![
            leg("openrouter", "vendor/model-a", "acct-r"),
            leg("openrouter", "vendor/model-a-copy", "acct-r"),
            leg("zen", "model-a", "acct-z"),
        ];
        let blocked = leg("openrouter", "model-b", "acct-r");
        let (legs, _) = assemble_ladder(&primary, stand_ins.clone(), vec![blocked], 4, true);
        assert_eq!(
            legs,
            vec![
                primary,
                stand_ins[0].clone(),
                stand_ins[1].clone(),
                stand_ins[2].clone()
            ],
            "openrouter already holds its two seats, so the kept seat is refilled"
        );
    }
}
