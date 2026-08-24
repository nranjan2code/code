//! Routing evidence ledger + ladder admission (docs/design/27 Phase B).
//!
//! Evidence is append-only JSONL; reads apply a TTL and treat outcomes as
//! success / failure / UNKNOWN. Billing without a verdict is UNKNOWN --
//! it shrinks confidence without punishing the model.

use std::collections::HashMap;
use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};

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
    /// skipped rather than trusted -- never misranked.
    pub fn snapshot(&self) -> EvidenceSnapshot {
        let cutoff = chrono::Utc::now() - chrono::Duration::days(EVIDENCE_TTL_DAYS as i64);
        let mut by_key: HashMap<(String, String), ModelEvidence> = HashMap::new();
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
            let e = by_key
                .entry((row.provider.clone(), row.model.clone()))
                .or_default();
            match row.outcome.as_str() {
                "success" => {
                    e.success += 1;
                    if e.p50_latency_ms.is_none() {
                        e.p50_latency_ms = Some(row.latency_ms);
                    }
                }
                "failure" => e.failure += 1,
                _ => e.unknown += 1,
            }
        }
        EvidenceSnapshot { by_key }
    }

    /// Fold a finished run's work receipts into evidence. Non-winning leg
    /// attribution is conservative: attempts record against the receipt's
    /// final model (fallback legs are what failed over FROM).
    pub fn record_receipts(&self, receipts: &[vak_llm::WorkReceipt]) {
        let now = chrono::Utc::now();
        for r in receipts {
            for a in &r.attempts {
                let outcome = match a.settlement {
                    vak_llm::Settlement::Ok => Some("success"),
                    vak_llm::Settlement::Failed => Some("failure"),
                    vak_llm::Settlement::Unknown => Some("unknown"),
                    _ => None, // user cancellation contributes nothing
                };
                if let Some(outcome) = outcome {
                    let _ = self.append(&EvidenceRow {
                        ts: now,
                        provider: String::new(),
                        model: r.model.clone(),
                        outcome: outcome.into(),
                        latency_ms: a.latency_ms,
                    });
                }
            }
        }
    }
}

/// Admission-time ordering wrapper over the versioned pure function.
pub fn order(
    candidates: Vec<RouteLeg>,
    snap: &EvidenceSnapshot,
    cost_of: impl Fn(&str) -> Option<f64>,
) -> Vec<RouteLeg> {
    vak_llm::route::order_ladder_v1(candidates, snap, false, cost_of)
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
                latency_ms: 42,
            })
            .unwrap();
        let mut p = dir.path().join("routing-evidence.jsonl");
        let mut existing = std::fs::read_to_string(&p).unwrap();
        existing.push_str("not json\n");
        std::fs::write(&mut p, existing).unwrap();

        let snap = ledger.snapshot();
        let e = snap.get("p", "m");
        assert_eq!(e.success, 1);
        assert_eq!(e.failure, 0, "40-day-old rows must be TTL-dropped");
        assert_eq!(e.p50_latency_ms, Some(42));
    }

    #[test]
    fn record_receipts_maps_settlements() {
        let dir = tempdir().unwrap();
        let ledger = EvidenceLedger::new(dir.path());
        let mut r = vak_llm::WorkReceipt::new(vak_llm::WorkPurpose::Execute, "m1");
        r.record(
            vak_llm::AttemptReason::Initial,
            vak_llm::FailureDomain::Network,
            vak_llm::Settlement::Unknown,
            10,
            None,
            None,
        );
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
        let e = snap.get("", "m1");
        assert_eq!(e.success, 1);
        assert_eq!(e.unknown, 1, "cancelled/unknown settlements stay unknown");
    }
}
