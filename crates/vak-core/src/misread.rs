//! Did we read the request right? (docs/design/47-commitment-kernel.md, I8)
//!
//! Every other feedback loop in this codebase closes on *observed outcomes*
//! rather than on self-assessment: the routing ladder learns from settlements,
//! not from asking a model whether the answer was good. Intent resolution gets
//! the same treatment.
//!
//! The strongest signal is a gift from capability slicing. When an engagement
//! removes a tool and the model then asks for that exact tool, the reading was
//! **measurably** wrong — not suspected wrong. Slicing does not merely improve
//! the turn; it turns misclassification into an observable event, which is the
//! only reason a feedback loop is possible at all.
//!
//! Epistemics match the routing ledger deliberately: success / failure /
//! **unknown**, TTL-filtered, and absence is a neutral prior rather than a
//! zero. A reading nobody corrected is not thereby proven right.

use std::collections::{BTreeSet, HashMap};
use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use vak_intent::{Act, Tier};

/// How long an observation stays relevant. Matches the routing ledger's TTL:
/// a lexicon that changed six weeks ago should not be judged on what it did
/// before the change.
const TTL_DAYS: i64 = 30;

/// What we observed about a reading after the fact.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Outcome {
    /// The turn proceeded and nothing contradicted the reading.
    Held,
    /// The engagement withheld a capability the turn then asked for. The one
    /// signal here that is measured rather than inferred.
    Escalated,
    /// A human overrode the reading before the run.
    Overridden,
    /// The user immediately restated the same request, which usually means
    /// the first attempt did the wrong thing.
    Restated,
    /// The turn was abandoned. Says something, but not what — it could be
    /// the reading, the answer, or the user changing their mind.
    Abandoned,
}

impl Outcome {
    pub fn as_str(self) -> &'static str {
        match self {
            Outcome::Held => "held",
            Outcome::Escalated => "escalated",
            Outcome::Overridden => "overridden",
            Outcome::Restated => "restated",
            Outcome::Abandoned => "abandoned",
        }
    }

    /// Whether this counts against the reading.
    ///
    /// `Abandoned` deliberately does not: a user who walked away tells us the
    /// turn ended, not that it was misread, and counting it as a failure would
    /// train the resolver on noise.
    pub fn contradicts(self) -> bool {
        matches!(
            self,
            Outcome::Escalated | Outcome::Overridden | Outcome::Restated
        )
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MisreadRow {
    pub ts: chrono::DateTime<chrono::Utc>,
    /// The act that was read, so accuracy can be reported per cell.
    pub act: String,
    pub stakes: String,
    /// Which tier produced the reading; a lexicon miss and a classifier miss
    /// call for different fixes.
    pub tier: String,
    pub resolver_version: u32,
    pub outcome: String,
    /// The capability the model asked for after it was sliced away. Present
    /// only on `Escalated`, and the most actionable field in the row: it names
    /// the exact lexicon or slice entry to change.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub wanted: Option<String>,
}

/// Append-only observations about intent readings.
///
/// Sits beside `routing-evidence.jsonl` and `commitments.jsonl` for the same
/// reason: it is evidence about the runtime's own behaviour, not conversation
/// content, and it must outlive any session.
pub struct MisreadLedger {
    path: PathBuf,
}

/// Accuracy for one `(act, stakes)` cell.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct CellAccuracy {
    pub act: String,
    pub stakes: String,
    pub held: u64,
    pub contradicted: u64,
    /// Observations that say nothing either way.
    pub unknown: u64,
    /// Capabilities the slice removed and the model then asked for, most
    /// frequent first. This is the actionable output of the whole loop.
    pub wanted: Vec<(String, u64)>,
}

impl CellAccuracy {
    /// Laplace-shrunk accuracy in [0,1].
    ///
    /// Identical treatment to `routing::reliability`: unknowns shrink
    /// confidence without punishing direction, and an absent record is a
    /// neutral prior rather than a zero. Three observations must not read as
    /// certainty in either direction.
    pub fn accuracy(&self) -> f64 {
        let trials = self.held + self.contradicted + self.unknown;
        (self.held as f64 + 0.5 * self.unknown as f64 + 1.0) / (trials as f64 + 2.0)
    }

    pub fn observations(&self) -> u64 {
        self.held + self.contradicted + self.unknown
    }
}

impl MisreadLedger {
    pub fn new(sessions_home: &Path) -> Self {
        MisreadLedger {
            path: sessions_home.join("intent-evidence.jsonl"),
        }
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    pub fn record(
        &self,
        reading: &vak_intent::Reading,
        tier: Tier,
        resolver_version: u32,
        outcome: Outcome,
        wanted: Option<String>,
    ) {
        let row = MisreadRow {
            ts: chrono::Utc::now(),
            act: reading.act.as_str().to_string(),
            stakes: reading.stakes.as_str().to_string(),
            tier: tier.as_str().to_string(),
            resolver_version,
            outcome: outcome.as_str().to_string(),
            wanted,
        };
        let Ok(line) = serde_json::to_string(&row) else {
            return;
        };
        if let Some(parent) = self.path.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        // Best-effort: losing a telemetry row must never cost the user a turn.
        if let Ok(mut file) = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&self.path)
        {
            let _ = writeln!(file, "{line}");
        }
    }

    /// TTL-filtered rows. Corrupt lines are skipped rather than trusted.
    pub fn rows(&self) -> Vec<MisreadRow> {
        let cutoff = chrono::Utc::now() - chrono::Duration::days(TTL_DAYS);
        let Ok(file) = std::fs::File::open(&self.path) else {
            return Vec::new();
        };
        BufReader::new(file)
            .lines()
            .map_while(Result::ok)
            .filter_map(|line| serde_json::from_str::<MisreadRow>(&line).ok())
            .filter(|row| row.ts >= cutoff)
            .collect()
    }

    /// Accuracy per `(act, stakes)` cell, weakest first.
    ///
    /// Ordering by weakest is the point: this report exists to say where the
    /// lexicon needs work, not to produce a flattering aggregate.
    pub fn accuracy(&self) -> Vec<CellAccuracy> {
        let mut cells: HashMap<(String, String), CellAccuracy> = HashMap::new();
        let mut wanted: HashMap<(String, String), HashMap<String, u64>> = HashMap::new();
        for row in self.rows() {
            let key = (row.act.clone(), row.stakes.clone());
            let cell = cells.entry(key.clone()).or_insert_with(|| CellAccuracy {
                act: row.act.clone(),
                stakes: row.stakes.clone(),
                ..CellAccuracy::default()
            });
            match row.outcome.as_str() {
                "held" => cell.held += 1,
                "escalated" | "overridden" | "restated" => cell.contradicted += 1,
                _ => cell.unknown += 1,
            }
            if let Some(name) = row.wanted {
                *wanted.entry(key).or_default().entry(name).or_insert(0) += 1;
            }
        }
        let mut out: Vec<CellAccuracy> = cells
            .into_iter()
            .map(|(key, mut cell)| {
                if let Some(counts) = wanted.remove(&key) {
                    let mut ranked: Vec<(String, u64)> = counts.into_iter().collect();
                    ranked.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.cmp(&b.0)));
                    cell.wanted = ranked;
                }
                cell
            })
            .collect();
        out.sort_by(|a, b| {
            a.accuracy()
                .partial_cmp(&b.accuracy())
                .unwrap_or(std::cmp::Ordering::Equal)
                .then_with(|| a.act.cmp(&b.act))
                .then_with(|| a.stakes.cmp(&b.stakes))
        });
        out
    }
}

/// Detect the measured misread: a capability the domain slice removed, that
/// the model then tried to call anyway.
///
/// `excluded_by_domain_slice` is `TurnCapabilities::excluded_by_domain_slice`
/// (crates/vak-core/src/capability/turn.rs) — the names that passed channel +
/// reach + contract (they really were admitted to this session) but were
/// then hidden from this turn by domain slicing. `attempted` is every tool
/// name the model tried to invoke this turn, admitted or not.
///
/// This used to check `engagement.limits.capabilities` (a `CapabilitySlice`),
/// but nothing in production ever narrows that field — domain slicing (the
/// mechanism that actually runs) narrows `required_domains` instead, which
/// `CapabilitySlice` never reflected. That made this function permanently
/// dead: `CapabilitySlice::All` always matched and the signal never fired.
/// Now it reads directly from what actually excluded the capability this
/// turn.
pub fn escalated_capability(excluded_by_domain_slice: &BTreeSet<String>, attempted: &[String]) -> Option<String> {
    attempted
        .iter()
        .find(|name| excluded_by_domain_slice.contains(name.as_str()))
        .cloned()
}

/// Acts whose readings are contradicted often enough to be worth a look.
///
/// Requires a real sample: a cell with two observations says nothing, and
/// reporting it as a problem would send someone to rewrite a lexicon entry on
/// the strength of a coin flip.
pub fn weak_cells(ledger: &MisreadLedger, min_observations: u64) -> Vec<CellAccuracy> {
    ledger
        .accuracy()
        .into_iter()
        .filter(|cell| cell.observations() >= min_observations && cell.accuracy() < 0.75)
        .collect()
}

/// Parse an act name back, for callers reporting per-cell coverage.
pub fn act_of(cell: &CellAccuracy) -> Option<Act> {
    Act::parse(&cell.act)
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::*;
    use vak_intent::{Reading, Stakes};

    fn reading(act: Act, stakes: Stakes) -> Reading {
        Reading {
            act,
            stakes,
            ..Reading::general()
        }
    }

    #[test]
    fn a_domain_excluded_capability_the_model_then_wanted_is_the_measured_misread() {
        // `excluded_by_domain_slice` is what `TurnCapabilities::build`
        // (crates/vak-core/src/capability/turn.rs) actually populates from
        // live domain slicing — this replaces the old, permanently-dead
        // `CapabilitySlice`-based check (nothing in production ever set
        // `engagement.limits.capabilities` to `Only`, only `required_domains`
        // was ever narrowed).
        let excluded = BTreeSet::from(["bash".to_string()]);
        assert_eq!(
            escalated_capability(&excluded, &["read".into(), "bash".into()]),
            Some("bash".into())
        );
        // Nothing wanted that was not excluded.
        assert_eq!(escalated_capability(&excluded, &["read".into()]), None);
    }

    /// A turn where nothing was domain-excluded cannot have escalated past
    /// an exclusion that never happened.
    #[test]
    fn no_exclusions_reports_no_escalation() {
        let excluded = BTreeSet::new();
        assert_eq!(
            escalated_capability(&excluded, &["bash".into(), "anything".into()]),
            None
        );
    }

    /// Abandonment says the turn ended, not that it was misread. Counting it
    /// against the reading would train the resolver on noise.
    #[test]
    fn abandonment_does_not_count_against_a_reading() {
        assert!(!Outcome::Abandoned.contradicts());
        assert!(!Outcome::Held.contradicts());
        for outcome in [Outcome::Escalated, Outcome::Overridden, Outcome::Restated] {
            assert!(outcome.contradicts());
        }
    }

    #[test]
    fn accuracy_is_shrunk_so_a_tiny_sample_is_not_certainty() {
        let dir = tempfile::tempdir().unwrap();
        let ledger = MisreadLedger::new(dir.path());
        ledger.record(
            &reading(Act::Modify, Stakes::Reversible),
            Tier::Signals,
            1,
            Outcome::Escalated,
            Some("bash".into()),
        );
        let cells = ledger.accuracy();
        assert_eq!(cells.len(), 1);
        // One contradiction out of one observation is 0.0 raw; shrinkage keeps
        // it well off the floor because one sample is not proof.
        assert!(cells[0].accuracy() > 0.2, "{}", cells[0].accuracy());
        assert!(cells[0].accuracy() < 0.5);
    }

    #[test]
    fn the_report_names_the_capability_to_put_back() {
        let dir = tempfile::tempdir().unwrap();
        let ledger = MisreadLedger::new(dir.path());
        for _ in 0..3 {
            ledger.record(
                &reading(Act::Answer, Stakes::Inert),
                Tier::Signals,
                1,
                Outcome::Escalated,
                Some("webfetch".into()),
            );
        }
        ledger.record(
            &reading(Act::Answer, Stakes::Inert),
            Tier::Signals,
            1,
            Outcome::Escalated,
            Some("bash".into()),
        );
        let cells = ledger.accuracy();
        assert_eq!(cells[0].wanted.first().unwrap().0, "webfetch");
        assert_eq!(cells[0].wanted.first().unwrap().1, 3);
    }

    #[test]
    fn weak_cells_need_a_real_sample_before_they_are_reported() {
        let dir = tempfile::tempdir().unwrap();
        let ledger = MisreadLedger::new(dir.path());
        ledger.record(
            &reading(Act::Operate, Stakes::Irreversible),
            Tier::Signals,
            1,
            Outcome::Escalated,
            None,
        );
        // One observation is not evidence of a weak cell.
        assert!(weak_cells(&ledger, 5).is_empty());
        for _ in 0..6 {
            ledger.record(
                &reading(Act::Operate, Stakes::Irreversible),
                Tier::Signals,
                1,
                Outcome::Escalated,
                None,
            );
        }
        assert_eq!(weak_cells(&ledger, 5).len(), 1);
    }

    #[test]
    fn a_healthy_cell_is_not_reported_as_weak() {
        let dir = tempfile::tempdir().unwrap();
        let ledger = MisreadLedger::new(dir.path());
        for _ in 0..20 {
            ledger.record(
                &reading(Act::Answer, Stakes::Inert),
                Tier::Signals,
                1,
                Outcome::Held,
                None,
            );
        }
        assert!(weak_cells(&ledger, 5).is_empty());
        assert!(ledger.accuracy()[0].accuracy() > 0.9);
    }

    #[test]
    fn corrupt_rows_are_skipped_rather_than_trusted() {
        let dir = tempfile::tempdir().unwrap();
        let ledger = MisreadLedger::new(dir.path());
        ledger.record(
            &reading(Act::Modify, Stakes::Reversible),
            Tier::Signals,
            1,
            Outcome::Held,
            None,
        );
        let mut file = std::fs::OpenOptions::new()
            .append(true)
            .open(ledger.path())
            .unwrap();
        writeln!(file, "not json at all").unwrap();
        drop(file);
        assert_eq!(ledger.rows().len(), 1);
    }
}
