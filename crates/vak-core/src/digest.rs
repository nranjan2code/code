//! Weekly usage digest (docs/design/29-personal-os.md P3): one report over
//! the cost ledger, memory stores, and skill-proposal queue. Reads are
//! bounded (line-streamed, window-filtered); missing files contribute
//! zeros, never errors — absence of history is a fact about the report,
//! not a failure of the run.

use std::collections::{BTreeMap, BTreeSet};
use std::io::{BufRead, BufReader};
use std::path::{Path, PathBuf};

use serde::Serialize;

use crate::finops::CostRow;

#[derive(Debug, Clone, Default, Serialize, PartialEq)]
pub struct ModelRollup {
    pub rows: u64,
    pub usd: f64,
    pub input_tokens: u64,
    pub output_tokens: u64,
    pub cache_read_tokens: u64,
}

#[derive(Debug, Clone, Default, Serialize, PartialEq)]
pub struct ProviderRollup {
    pub rows: u64,
    pub usd: f64,
}

#[derive(Debug, Clone, Default, Serialize, PartialEq)]
pub struct DayRollup {
    /// Local calendar day, "YYYY-MM-DD".
    pub day: String,
    pub usd: f64,
    /// Rows with unknown price that day: UNKNOWN is never folded into $0.
    pub unpriced_rows: u64,
}

#[derive(Debug, Clone, Default, Serialize)]
pub struct DigestReport {
    pub days: u32,
    /// Window start (now − days), echoed so surfaces can label it.
    pub since: Option<chrono::DateTime<chrono::Utc>>,
    pub total_usd: f64,
    /// Priced dispatches excluded from total_usd because the model had no
    /// price — surfaced instead of silently reading as zero spend.
    pub unpriced_rows: u64,
    pub input_tokens: u64,
    pub output_tokens: u64,
    pub cache_read_tokens: u64,
    pub dispatches: u64,
    pub by_model: BTreeMap<String, ModelRollup>,
    pub by_provider: BTreeMap<String, ProviderRollup>,
    /// Ascending by day; only days with rows appear.
    pub per_day: Vec<DayRollup>,
    /// Distinct session ids touched in the window, sorted.
    pub distinct_sessions: Vec<String>,
    /// Memory notes (per-workspace MEMORY.md + global USER.md profile)
    /// whose provenance timestamp falls inside the window.
    pub memory_notes_appended: usize,
    /// Skill proposals created inside the window (proposal comment
    /// timestamp; file mtime as fallback).
    pub skill_proposals_opened: usize,
}

fn cost_log_path(scope: &vak_config::scope::AgentScope) -> PathBuf {
    scope.cost_log()
}

/// Stream the cost ledger once, folding priced/unpriced rows inside the
/// window into the running aggregates.
fn fold_costs(
    scope: &vak_config::scope::AgentScope,
    since: chrono::DateTime<chrono::Utc>,
    sessions: &mut BTreeSet<String>,
    report: &mut DigestReport,
) {
    let Ok(f) = std::fs::File::open(cost_log_path(scope)) else {
        return;
    };
    for line in BufReader::new(f).lines().map_while(Result::ok) {
        let Ok(row) = serde_json::from_str::<CostRow>(&line) else {
            continue;
        };
        if row.ts < since {
            continue;
        }
        let local_day = row
            .ts
            .with_timezone(&chrono::Local)
            .format("%Y-%m-%d")
            .to_string();
        let rollup = |usd: Option<f64>, map: &mut BTreeMap<String, ModelRollup>| {
            let e = map.entry(row.model.clone()).or_default();
            e.rows += 1;
            e.input_tokens += row.input_tokens;
            e.output_tokens += row.output_tokens;
            e.cache_read_tokens += row.cache_read_input_tokens.unwrap_or(0);
            if let Some(usd) = usd {
                e.usd += usd;
            }
        };
        rollup(row.usd, &mut report.by_model);

        let provider_key = if row.provider.is_empty() {
            "(unattributed)".to_string()
        } else {
            row.provider.clone()
        };
        let pe = report.by_provider.entry(provider_key).or_default();
        pe.rows += 1;
        if let Some(usd) = row.usd {
            pe.usd += usd;
        }

        let de = report.per_day.iter_mut().find(|d| d.day == local_day);
        match de {
            Some(d) => {
                d.usd += row.usd.unwrap_or(0.0);
                d.unpriced_rows += u64::from(row.usd.is_none());
            }
            None => report.per_day.push(DayRollup {
                day: local_day,
                usd: row.usd.unwrap_or(0.0),
                unpriced_rows: u64::from(row.usd.is_none()),
            }),
        }

        report.total_usd += row.usd.unwrap_or(0.0);
        report.unpriced_rows += u64::from(row.usd.is_none());
        report.input_tokens += row.input_tokens;
        report.output_tokens += row.output_tokens;
        report.cache_read_tokens += row.cache_read_input_tokens.unwrap_or(0);
        report.dispatches += 1;
        sessions.insert(row.session_id.clone());
    }
    report.per_day.sort_by(|a, b| a.day.cmp(&b.day));
}

fn count_fresh_notes(path: &Path, since: chrono::DateTime<chrono::Utc>) -> usize {
    let Ok(raw) = std::fs::read_to_string(path) else {
        return 0;
    };
    crate::memory::parse_blocks(&raw)
        .iter()
        .filter(|n| n.ts >= since)
        .count()
}

/// Every markdown store under `<home>/memory/` — per-workspace MEMORY.md
/// files plus the global USER.md profile tier.
fn memory_files(scope: &vak_config::scope::AgentScope) -> Vec<PathBuf> {
    let mut out = Vec::new();
    let root = scope.memory_root();
    let Ok(read) = std::fs::read_dir(&root) else {
        return out;
    };
    let mut projects: Vec<PathBuf> = read.flatten().map(|e| e.path()).collect();
    projects.sort();
    for project in projects {
        if project.is_file() {
            if project.extension().and_then(|e| e.to_str()) == Some("md") {
                out.push(project);
            }
            continue;
        }
        let Ok(files) = std::fs::read_dir(&project) else {
            continue;
        };
        out.extend(
            files
                .flatten()
                .map(|f| f.path())
                .filter(|p| p.is_file() && p.extension().and_then(|e| e.to_str()) == Some("md")),
        );
    }
    out.sort();
    out
}

fn proposal_opened_ts(path: &Path) -> Option<chrono::DateTime<chrono::Utc>> {
    let raw = std::fs::read_to_string(path).ok()?;
    // Proposals carry "<!-- proposed-by: <sid> at <rfc3339>; ... -->".
    if let Some(rest) = raw.split(" at ").nth(1)
        && let Some(ts_raw) = rest.split(';').next()
        && let Ok(ts) = chrono::DateTime::parse_from_rfc3339(ts_raw.trim())
    {
        return Some(ts.with_timezone(&chrono::Utc));
    }
    let meta = std::fs::metadata(path).ok()?;
    let modified = meta.modified().ok()?;
    Some(chrono::DateTime::<chrono::Utc>::from(modified))
}

/// Build the digest over the trailing `days` (24h windows). `days == 0`
/// yields an empty report by construction (the window is empty).
/// `home` is the Agent's home (its memory and skill proposals);
/// `shared_home` holds what every Agent shares: the cost ledger and the
/// trash. Spend stays whole, because it was spent; a trashed session is left
/// out of `distinct_sessions`, the one place a digest names a session.
pub fn digest(
    scope: &vak_config::scope::AgentScope,
    shared: &vak_config::scope::SharedScope,
    days: u32,
) -> DigestReport {
    let mut report = DigestReport {
        days,
        ..Default::default()
    };
    if days == 0 {
        return report;
    }
    let since = chrono::Utc::now() - chrono::Duration::hours(days.saturating_mul(24) as i64);
    report.since = Some(since);

    let mut sessions = BTreeSet::new();
    fold_costs(&shared.as_agent(), since, &mut sessions, &mut report);
    let trashed = crate::trash::trashed(shared);
    report.distinct_sessions = sessions
        .into_iter()
        .filter(|id| !trashed.contains(id))
        .collect();

    for path in memory_files(scope) {
        report.memory_notes_appended += count_fresh_notes(&path, since);
    }

    let mut proposals: Vec<PathBuf> = Vec::new();
    let root = scope.skill_proposals_root();
    if let Ok(read) = std::fs::read_dir(&root) {
        for project in read.flatten() {
            let Ok(files) = std::fs::read_dir(project.path()) else {
                continue;
            };
            proposals.extend(
                files
                    .flatten()
                    .map(|f| f.path())
                    .filter(|p| p.extension().and_then(|e| e.to_str()) == Some("md")),
            );
        }
    }
    proposals.sort();
    report.skill_proposals_opened = proposals
        .iter()
        .filter(|p| proposal_opened_ts(p).is_some_and(|ts| ts >= since))
        .count();

    report
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]
    use super::*;
    use crate::memory;

    fn row(
        ts: chrono::DateTime<chrono::Utc>,
        model: &str,
        provider: &str,
        usd: Option<f64>,
        sid: &str,
    ) -> CostRow {
        CostRow {
            ts,
            model: model.into(),
            provider: provider.into(),
            input_tokens: 100,
            output_tokens: 50,
            cache_read_input_tokens: None,
            usd,
            source: "estimated".into(),
            session_id: sid.into(),
            trace: None,
            actor: None,
        }
    }

    #[test]
    fn missing_files_yield_zeros_not_errors() {
        let dir = tempfile::tempdir().unwrap();
        let r = digest(
            &vak_config::scope::AgentScope::new(dir.path()),
            &vak_config::scope::SharedScope::new(dir.path()),
            7,
        );
        assert_eq!(r.total_usd, 0.0);
        assert_eq!(r.dispatches, 0);
        assert!(r.by_model.is_empty());
        assert!(r.per_day.is_empty());
        assert!(r.distinct_sessions.is_empty());
        assert_eq!(r.memory_notes_appended, 0);
        assert_eq!(r.skill_proposals_opened, 0);
        // Empty home with no memory/ dir at all must behave identically.
        let empty = tempfile::tempdir().unwrap();
        assert_eq!(
            digest(
                &vak_config::scope::AgentScope::new(empty.path()),
                &vak_config::scope::SharedScope::new(empty.path()),
                30
            )
            .dispatches,
            0
        );
    }

    #[test]
    fn zero_days_is_an_empty_window() {
        let dir = tempfile::tempdir().unwrap();
        let ledger = crate::finops::FinOpsLedger::new(dir.path());
        ledger
            .append(&row(chrono::Utc::now(), "m", "p", Some(1.0), "s"))
            .unwrap();
        let r = digest(
            &vak_config::scope::AgentScope::new(dir.path()),
            &vak_config::scope::SharedScope::new(dir.path()),
            0,
        );
        assert_eq!(r.dispatches, 0);
        assert!(r.since.is_none());
    }

    #[test]
    fn ledger_math_matches_seeded_rows() {
        let dir = tempfile::tempdir().unwrap();
        let ledger = crate::finops::FinOpsLedger::new(dir.path());
        let now = chrono::Utc::now();
        ledger
            .append(&row(
                now - chrono::Duration::hours(1),
                "claude-sonnet",
                "anthropic",
                Some(2.0),
                "s1",
            ))
            .unwrap();
        ledger
            .append(&row(
                now - chrono::Duration::hours(2),
                "claude-sonnet",
                "anthropic",
                None,
                "s1",
            ))
            .unwrap();
        ledger
            .append(&row(
                now - chrono::Duration::hours(3),
                "gpt-x",
                "openai",
                Some(0.5),
                "s2",
            ))
            .unwrap();
        // Legacy row without provider attribution.
        ledger
            .append(&row(
                now - chrono::Duration::hours(4),
                "old-m",
                "",
                Some(0.25),
                "s3",
            ))
            .unwrap();
        // Outside the window: ignored entirely.
        ledger
            .append(&row(
                now - chrono::Duration::hours(24 * 9),
                "ancient",
                "anthropic",
                Some(99.0),
                "s0",
            ))
            .unwrap();

        let r = digest(
            &vak_config::scope::AgentScope::new(dir.path()),
            &vak_config::scope::SharedScope::new(dir.path()),
            7,
        );
        assert!((r.total_usd - 2.75).abs() < 1e-9, "{}", r.total_usd);
        assert_eq!(r.unpriced_rows, 1);
        assert_eq!(r.dispatches, 4);
        assert_eq!(r.input_tokens, 400);
        assert_eq!(r.output_tokens, 200);
        assert_eq!(
            r.distinct_sessions,
            vec!["s1".to_string(), "s2".to_string(), "s3".to_string()]
        );

        let sonnet = &r.by_model["claude-sonnet"];
        assert_eq!(sonnet.rows, 2);
        assert!((sonnet.usd - 2.0).abs() < 1e-9);
        assert_eq!(sonnet.input_tokens, 200);
        let openai = &r.by_provider["openai"];
        assert!((openai.usd - 0.5).abs() < 1e-9);
        assert_eq!(r.by_provider["(unattributed)"].rows, 1);

        // Per-day buckets cover every in-window row exactly once.
        let day_sum: f64 = r.per_day.iter().map(|d| d.usd).sum();
        let day_unpriced: u64 = r.per_day.iter().map(|d| d.unpriced_rows).sum();
        assert!((day_sum - r.total_usd).abs() < 1e-9);
        assert_eq!(day_unpriced, r.unpriced_rows);
        assert!(r.per_day.windows(2).all(|w| w[0].day < w[1].day));
    }

    #[test]
    fn memory_notes_and_profile_counted_from_provenance_ts() {
        let dir = tempfile::tempdir().unwrap();
        let home = dir.path();
        let cwd = home.join("ws");
        std::fs::create_dir_all(&cwd).unwrap();

        memory::append_note(home, &cwd, "fact", "fresh", "s", "brand new note").unwrap();

        let old_header =
            "## 2026-01-01T00:00:00+00:00 [fact] tag=stale session=old\nancient note\n";
        let mem_dir = home.join("memory").join(memory::hash_cwd(&cwd));
        std::fs::create_dir_all(&mem_dir).unwrap();
        let fresh = std::fs::read_to_string(mem_dir.join("MEMORY.md")).unwrap();
        std::fs::write(mem_dir.join("MEMORY.md"), format!("{fresh}{old_header}")).unwrap();

        memory::append_profile_note(home, "preference", "editor", "vim bindings", "su").unwrap();

        let r = digest(
            &vak_config::scope::AgentScope::new(home),
            &vak_config::scope::SharedScope::new(home),
            7,
        );
        assert_eq!(
            r.memory_notes_appended, 2,
            "fresh workspace + profile notes"
        );
    }

    #[test]
    fn skill_proposals_counted_within_window() {
        let dir = tempfile::tempdir().unwrap();
        let home = dir.path();
        let proj = home.join("skill-proposals").join("abc");
        std::fs::create_dir_all(&proj).unwrap();

        let now = chrono::Utc::now().to_rfc3339();
        std::fs::write(
            proj.join("fresh.md"),
            format!("---\nname: \"a\"\ndescription: \"d\"\n---\n\nbody\n\n<!-- proposed-by: s1 at {now}; proposal id x -->\n"),
        )
        .unwrap();
        std::fs::write(
            proj.join("old.md"),
            "---\nname: \"b\"\ndescription: \"d\"\n---\n\nbody\n\n<!-- proposed-by: s2 at 2026-01-01T00:00:00+00:00; proposal id y -->\n",
        )
        .unwrap();
        // No parseable comment → falls back to mtime (now) → counts.
        std::fs::write(proj.join("mtime-only.md"), "no comment here").unwrap();

        let r = digest(
            &vak_config::scope::AgentScope::new(home),
            &vak_config::scope::SharedScope::new(home),
            7,
        );
        assert_eq!(r.skill_proposals_opened, 2);
    }
}
