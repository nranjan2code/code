//! Durable operational incident records.
//!
//! The control-plane snapshot is sampled, but incidents must survive a page
//! refresh and a server restart. This module stores an append-only event log
//! under the server's sessions home and folds it into the current incident
//! state. A repeated observation updates an incident at most once per minute;
//! a signal disappearing records a resolution event instead of deleting the
//! object.

use std::collections::HashMap;
use std::io::Write;
use std::path::{Path, PathBuf};

use chrono::{DateTime, Duration, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

fn log_path(home: &Path) -> PathBuf {
    home.join("operations").join("incidents.jsonl")
}

fn actions_path(home: &Path) -> PathBuf {
    home.join("operations").join("actions.jsonl")
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct IncidentCandidate {
    pub fingerprint: String,
    pub severity: String,
    pub source: String,
    pub title: String,
    pub detail: String,
    pub workspace: Option<String>,
    pub evidence: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct IncidentRecord {
    pub id: String,
    pub fingerprint: String,
    pub severity: String,
    pub status: String,
    pub source: String,
    pub title: String,
    pub detail: String,
    pub first_seen: DateTime<Utc>,
    pub last_seen: DateTime<Utc>,
    pub occurrences: u64,
    pub workspace: Option<String>,
    pub evidence: Vec<String>,
    pub resolution: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub trace: Option<vak_session::trace::TraceKey>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub actor: Option<vak_session::ids::PrincipalId>,
}

vak_session::impl_traced!(IncidentRecord, "incident_record");

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ActionVerification {
    pub status: String,
    pub before: String,
    pub after: String,
    pub detail: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ActionReceipt {
    pub receipt_id: String,
    pub service: String,
    pub action: String,
    pub requested_at: DateTime<Utc>,
    pub completed_at: DateTime<Utc>,
    pub succeeded: bool,
    pub verification: ActionVerification,
    pub persisted: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub trace: Option<vak_session::trace::TraceKey>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub actor: Option<vak_session::ids::PrincipalId>,
}

vak_session::impl_traced!(ActionReceipt, "action_receipt");

#[derive(Debug, Clone, Serialize, Deserialize)]
struct IncidentEvent {
    ts: DateTime<Utc>,
    event: String,
    incident: IncidentRecord,
}

fn read_events(home: &Path) -> Vec<IncidentEvent> {
    let Ok(raw) = std::fs::read_to_string(log_path(home)) else {
        return Vec::new();
    };
    raw.lines()
        .filter(|line| !line.trim().is_empty())
        .filter_map(|line| serde_json::from_str(line).ok())
        .collect()
}

fn folded(home: &Path) -> Vec<IncidentRecord> {
    let mut records = HashMap::<String, IncidentRecord>::new();
    for event in read_events(home) {
        records.insert(event.incident.id.clone(), event.incident);
    }
    let mut values: Vec<_> = records.into_values().collect();
    values.sort_by_key(|record| std::cmp::Reverse(record.last_seen));
    values
}

fn append(home: &Path, event: &IncidentEvent) {
    let path = log_path(home);
    let Some(parent) = path.parent() else { return };
    if std::fs::create_dir_all(parent).is_err() {
        return;
    }
    let Ok(mut line) = serde_json::to_vec(event) else {
        return;
    };
    line.push(b'\n');
    if let Ok(mut file) = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)
    {
        let _ = file.write_all(&line);
    }
}

/// Persist an operation receipt. Receipts are append-only so an operator can
/// prove what was requested, what the service manager returned, and what a
/// follow-up state probe observed.
pub fn record_action(home: &Path, receipt: &ActionReceipt) -> Result<(), String> {
    let path = actions_path(home);
    let Some(parent) = path.parent() else {
        return Err("operations path has no parent".to_string());
    };
    std::fs::create_dir_all(parent)
        .map_err(|error| format!("create operations directory: {error}"))?;
    let mut line =
        serde_json::to_vec(receipt).map_err(|error| format!("encode receipt: {error}"))?;
    line.push(b'\n');
    let mut file = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)
        .map_err(|error| format!("open action ledger: {error}"))?;
    file.write_all(&line)
        .map_err(|error| format!("append action receipt: {error}"))
}

/// Return the newest durable action receipts, newest first.
pub fn recent_actions(home: &Path, limit: usize) -> Vec<ActionReceipt> {
    let Ok(raw) = std::fs::read_to_string(actions_path(home)) else {
        return Vec::new();
    };
    raw.lines()
        .rev()
        .filter(|line| !line.trim().is_empty())
        .filter_map(|line| serde_json::from_str::<ActionReceipt>(line).ok())
        .take(limit)
        .collect()
}

fn new_record(candidate: IncidentCandidate, now: DateTime<Utc>) -> IncidentRecord {
    IncidentRecord {
        id: format!("INC-{}", Uuid::now_v7().simple()),
        fingerprint: candidate.fingerprint,
        severity: candidate.severity,
        status: "investigating".to_string(),
        source: candidate.source,
        title: candidate.title,
        detail: candidate.detail,
        first_seen: now,
        last_seen: now,
        occurrences: 1,
        workspace: candidate.workspace,
        evidence: candidate.evidence,
        resolution: None,
        trace: None,
        actor: None,
    }
}

/// Reconcile the current probe candidates into durable incident records.
/// Returns open records first, followed by the most recently resolved records.
pub fn reconcile(home: &Path, candidates: Vec<IncidentCandidate>) -> Vec<IncidentRecord> {
    let now = Utc::now();
    let mut all = folded(home);
    let mut by_fingerprint: HashMap<String, usize> = all
        .iter()
        .enumerate()
        .map(|(index, record)| (record.fingerprint.clone(), index))
        .collect();
    let mut seen = std::collections::HashSet::new();

    for candidate in candidates {
        seen.insert(candidate.fingerprint.clone());
        if let Some(index) = by_fingerprint.get(&candidate.fingerprint).copied() {
            let record = &mut all[index];
            let was_resolved = record.status == "resolved";
            let due = now.signed_duration_since(record.last_seen) >= Duration::minutes(1);
            record.severity = candidate.severity;
            record.source = candidate.source;
            record.title = candidate.title;
            record.detail = candidate.detail;
            record.workspace = candidate.workspace;
            record.evidence = candidate.evidence;
            record.status = "investigating".to_string();
            record.resolution = None;
            if was_resolved || due {
                record.last_seen = now;
                record.occurrences = record.occurrences.saturating_add(1);
                append(
                    home,
                    &IncidentEvent {
                        ts: now,
                        event: if was_resolved { "reopened" } else { "observed" }.to_string(),
                        incident: record.clone(),
                    },
                );
            }
        } else {
            let record = new_record(candidate, now);
            append(
                home,
                &IncidentEvent {
                    ts: now,
                    event: "opened".to_string(),
                    incident: record.clone(),
                },
            );
            by_fingerprint.insert(record.fingerprint.clone(), all.len());
            all.push(record);
        }
    }

    for record in &mut all {
        if record.status != "resolved" && !seen.contains(&record.fingerprint) {
            record.status = "resolved".to_string();
            record.resolution =
                Some("No longer present in current control-plane probes".to_string());
            record.last_seen = now;
            append(
                home,
                &IncidentEvent {
                    ts: now,
                    event: "resolved".to_string(),
                    incident: record.clone(),
                },
            );
        }
    }

    all.sort_by(|a, b| {
        (a.status != "investigating")
            .cmp(&(b.status != "investigating"))
            .then_with(|| b.last_seen.cmp(&a.last_seen))
    });
    all
}

/// Read the folded incident history without creating new records.
pub fn list(home: &Path) -> Vec<IncidentRecord> {
    folded(home)
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;

    fn candidate(fingerprint: &str) -> IncidentCandidate {
        IncidentCandidate {
            fingerprint: fingerprint.to_string(),
            severity: "warning".to_string(),
            source: "test".to_string(),
            title: "Test incident".to_string(),
            detail: "evidence".to_string(),
            workspace: Some("/tmp/project".to_string()),
            evidence: vec!["check:test".to_string()],
        }
    }

    #[test]
    fn opens_and_folds_a_durable_incident() {
        let dir = tempfile::tempdir().unwrap();
        let rows = reconcile(dir.path(), vec![candidate("test")]);
        assert_eq!(rows.len(), 1);
        assert!(rows[0].id.starts_with("INC-"));
        assert_eq!(rows[0].status, "investigating");
        assert_eq!(list(dir.path()), rows);
    }

    #[test]
    fn repeated_observation_groups_instead_of_creating_duplicates() {
        let dir = tempfile::tempdir().unwrap();
        let first = reconcile(dir.path(), vec![candidate("test")]);
        let second = reconcile(dir.path(), vec![candidate("test")]);
        assert_eq!(second.len(), 1);
        assert_eq!(second[0].id, first[0].id);
        assert_eq!(second[0].occurrences, 1);
    }

    #[test]
    fn disappearance_records_resolution_and_reappearance_reopens() {
        let dir = tempfile::tempdir().unwrap();
        let first = reconcile(dir.path(), vec![candidate("test")]);
        let resolved = reconcile(dir.path(), Vec::new());
        assert_eq!(resolved[0].id, first[0].id);
        assert_eq!(resolved[0].status, "resolved");
        let reopened = reconcile(dir.path(), vec![candidate("test")]);
        assert_eq!(reopened[0].id, first[0].id);
        assert_eq!(reopened[0].status, "investigating");
        assert_eq!(reopened[0].occurrences, 2);
    }

    #[test]
    fn action_receipts_round_trip_newest_first() {
        let dir = tempfile::tempdir().unwrap();
        let receipt = ActionReceipt {
            receipt_id: "OP-test".to_string(),
            service: "gateway".to_string(),
            action: "restart".to_string(),
            requested_at: Utc::now(),
            completed_at: Utc::now(),
            succeeded: true,
            verification: ActionVerification {
                status: "verified".to_string(),
                before: "stopped".to_string(),
                after: "running".to_string(),
                detail: "probe reached running".to_string(),
            },
            persisted: true,
            trace: None,
            actor: None,
        };
        record_action(dir.path(), &receipt).unwrap();
        assert_eq!(recent_actions(dir.path(), 10), vec![receipt]);
    }
}
