//! Scheduled-task store (docs/design/29-personal-os.md P2). Same
//! `tasks.json` shape the server surface wrote — legacy files load
//! unchanged — plus additive optional fields: 5-field cron schedules,
//! watchdog shell one-liners (XOR with prompt), and per-task model pins.
//! The store is plain JSON under `<sessions-home>/tasks.json`; sessions
//! themselves stay append-only ledgers.

use std::collections::{BTreeSet, HashMap};
use std::io::Write;
use std::path::{Path, PathBuf};

use chrono::{DateTime, Datelike, TimeZone, Timelike, Utc};

/// How far ahead [`cron_next_after`] searches before giving up. Cron can
/// legitimately sleep ~4 years (Feb 29); anything beyond five is a bug in
/// the expression, not a schedule.
const CRON_HORIZON_DAYS: i64 = 366 * 5;

#[derive(Debug, Clone, PartialEq)]
struct FieldSet {
    values: BTreeSet<u32>,
    /// True only for a literal `*` field — vixie-cron's dom/dow OR rule
    /// keys on restriction, not on which values ended up selected.
    starred: bool,
}

impl FieldSet {
    fn contains(&self, v: u32) -> bool {
        self.starred || self.values.contains(&v)
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct CronExpr {
    minutes: FieldSet,
    hours: FieldSet,
    days_of_month: FieldSet,
    months: FieldSet,
    days_of_week: FieldSet,
}

impl CronExpr {
    pub fn parse(expr: &str) -> Result<Self, String> {
        let fields: Vec<&str> = expr.split_whitespace().collect();
        if fields.len() != 5 {
            return Err(format!(
                "expected 5 fields (min hour dom mon dow), got {}",
                fields.len()
            ));
        }
        Ok(CronExpr {
            minutes: parse_field(fields[0], 0, 59, false)?,
            hours: parse_field(fields[1], 0, 23, false)?,
            days_of_month: parse_field(fields[2], 1, 31, false)?,
            months: parse_field(fields[3], 1, 12, false)?,
            days_of_week: parse_field(fields[4], 0, 7, true)?,
        })
    }
}

fn parse_field(spec: &str, min: u32, max: u32, dow_wrap: bool) -> Result<FieldSet, String> {
    let mut values = BTreeSet::new();
    let mut starred = false;
    // Vixie convention: 0 and 7 are both Sunday; mapping happens AFTER
    // range expansion so `5-7` means Fri–Sun rather than a reversed range.
    let wrap = |v: u32| if dow_wrap && v == 7 { 0 } else { v };
    for term in spec.split(',') {
        let term = term.trim();
        if term.is_empty() {
            return Err(format!("empty list element in '{spec}'"));
        }
        let (base, step) = match term.split_once('/') {
            Some((b, s)) => {
                let step: u32 = s.parse().map_err(|_| format!("bad step '{s}'"))?;
                if step == 0 {
                    return Err(format!("step must be >= 1 in '{term}'"));
                }
                (b, Some(step))
            }
            None => (term, None),
        };
        let range: Vec<u32> = match base {
            "*" => {
                if term == "*" && step.is_none() {
                    starred = true;
                }
                (min..=max).collect()
            }
            b if b.contains('-') => {
                let (lo, hi) = b
                    .split_once('-')
                    .ok_or_else(|| format!("bad range '{b}'"))?;
                let lo = parse_number(lo, min, max)?;
                let hi = parse_number(hi, min, max)?;
                if hi < lo {
                    return Err(format!("reversed range '{b}'"));
                }
                (lo..=hi).map(wrap).collect()
            }
            b => {
                if step.is_some() {
                    return Err(format!("step requires '*' or a range in '{term}'"));
                }
                vec![wrap(parse_number(b, min, max)?)]
            }
        };
        match step {
            None => values.extend(range),
            Some(s) => values.extend(range.into_iter().step_by(s as usize)),
        }
    }
    Ok(FieldSet { values, starred })
}

fn parse_number(raw: &str, min: u32, max: u32) -> Result<u32, String> {
    let v: u32 = raw
        .trim()
        .parse()
        .map_err(|_| format!("bad number '{raw}'"))?;
    if !(min..=max).contains(&v) {
        return Err(format!("value {v} out of range {min}-{max}"));
    }
    Ok(v)
}

/// Next local time strictly after `after` where the expression matches.
///
/// DST semantics are deterministic by construction: candidate wall-clock
/// times that do not exist (spring-forward gap) never fire — scanning
/// continues to the next matching time that DOES exist; ambiguous times
/// (fall-back fold) resolve to the earliest instant. Pure: no clock reads.
pub fn cron_next_after(
    expr: &str,
    after: chrono::DateTime<chrono::Local>,
) -> Result<DateTime<chrono::Local>, String> {
    let parsed = CronExpr::parse(expr)?;
    next_fire(&parsed, after)
        .ok_or_else(|| format!("expression '{expr}' never fires within {CRON_HORIZON_DAYS} days"))
}

fn next_fire<Tz: TimeZone>(expr: &CronExpr, after: DateTime<Tz>) -> Option<DateTime<Tz>> {
    let tz = after.timezone();
    let naive = after.naive_local();
    // Strictly-after: start from the minute boundary following `after`.
    let start_minute =
        naive.date().and_hms_opt(naive.hour(), naive.minute(), 0)? + chrono::Duration::minutes(1);

    let dom_restricted = !expr.days_of_month.starred;
    let dow_restricted = !expr.days_of_week.starred;

    for day_offset in 0..CRON_HORIZON_DAYS {
        let date = start_minute.date() + chrono::Duration::days(day_offset);
        if !expr.months.contains(date.month()) {
            continue;
        }
        let dom_match = expr.days_of_month.contains(date.day());
        // 0=Sunday … 6=Saturday, matching the cron numbering.
        let dow_match = expr
            .days_of_week
            .contains(date.weekday().num_days_from_sunday());
        let day_matches = match (dom_restricted, dow_restricted) {
            // Vixie rule: when both day fields are restricted, EITHER may
            // fire; otherwise every listed field must match.
            (true, true) => dom_match || dow_match,
            _ => dom_match && dow_match,
        };
        if !day_matches {
            continue;
        }
        for h in &expr.hours.values {
            for m in &expr.minutes.values {
                let candidate = date.and_hms_opt(*h, *m, 0)?;
                if candidate < start_minute {
                    continue;
                }
                // Gap → None → skip forward to the next existing match;
                // fold → earliest instant wins.
                if let Some(dt) = tz.from_local_datetime(&candidate).earliest() {
                    return Some(dt);
                }
            }
        }
    }
    None
}

pub fn tasks_file(sessions_home: &Path) -> PathBuf {
    sessions_home.join("tasks.json")
}

/// Worktree metadata recorded with the last run. Shape-compatible twin of
/// the server's WtMeta so old tasks.json round-trips without data loss.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct WtMeta {
    pub path: PathBuf,
    pub branch: String,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct TaskDef {
    pub id: String,
    pub name: String,
    #[serde(default)]
    pub prompt: String,
    #[serde(default = "default_interval")]
    pub interval_secs: u64,
    pub enabled: bool,
    pub cwd: PathBuf,
    pub created_at: DateTime<Utc>,
    pub last_run_at: Option<DateTime<Utc>>,
    pub last_session_id: Option<String>,
    pub last_summary: Option<String>,
    #[serde(default)]
    pub last_result_id: Option<String>,
    #[serde(default)]
    pub last_run_status: Option<String>,
    #[serde(default)]
    pub last_delivery_state: Option<String>,
    #[serde(default)]
    pub last_wt: Option<WtMeta>,
    #[serde(default)]
    pub deliver_to: Option<String>,
    /// 5-field cron (`m h dom mon dow`, local time) replacing interval
    /// ticking when present. Validated against the cron grammar.
    #[serde(default)]
    pub schedule: Option<String>,
    /// Watchdog shell one-liner. XOR with `prompt`: script tasks run
    /// brokered bash and cost zero tokens when stdout stays empty.
    #[serde(default)]
    pub script: Option<String>,
    /// Pinned model id; a pinned task dispatches only this model and
    /// never escalates.
    #[serde(default)]
    pub model_pin: Option<String>,
    /// Saved helper identity used when this task was created.
    #[serde(default)]
    pub agent_profile_id: Option<String>,
    #[serde(default)]
    pub agent_profile_revision: Option<u64>,
}

fn default_interval() -> u64 {
    3600
}

#[derive(Debug, thiserror::Error)]
pub enum TaskError {
    #[error("task '{name}': exactly one of `prompt` or `script` is required")]
    PromptScriptXor { name: String },
    #[error("invalid schedule '{expr}': {reason}")]
    BadSchedule { expr: String, reason: String },
    #[error("io error on {path}: {source}")]
    Io {
        path: PathBuf,
        source: std::io::Error,
    },
    #[error("corrupt tasks file {path}: {source}")]
    Json {
        path: PathBuf,
        source: serde_json::Error,
    },
}

impl TaskDef {
    /// Structural validation: prompt XOR script required, cron parsed when
    /// scheduled. Pure; never touches the store.
    pub fn validate(&self) -> Result<(), TaskError> {
        let has_prompt = !self.prompt.trim().is_empty();
        let has_script = self.script.as_deref().is_some_and(|s| !s.trim().is_empty());
        if has_prompt == has_script {
            return Err(TaskError::PromptScriptXor {
                name: self.name.clone(),
            });
        }
        if let Some(expr) = &self.schedule {
            CronExpr::parse(expr).map_err(|reason| TaskError::BadSchedule {
                expr: expr.clone(),
                reason,
            })?;
        }
        Ok(())
    }
}

/// fsyncs a directory so a prior rename into it is durable across a crash.
/// On non-unix platforms directory fsync isn't a thing; the rename itself
/// is still atomic there, so this is a no-op rather than an error.
#[cfg(unix)]
fn sync_directory(path: &Path) -> std::io::Result<()> {
    std::fs::File::open(path).and_then(|dir| dir.sync_all())
}

#[cfg(not(unix))]
fn sync_directory(_path: &Path) -> std::io::Result<()> {
    Ok(())
}

/// In-process mirror of the persisted tasks.json array, keyed by id.
pub struct TaskStore {
    path: PathBuf,
    tasks: HashMap<String, TaskDef>,
}

impl TaskStore {
    /// Missing file loads as an empty store; a corrupt file is a typed
    /// error, never silently dropped work.
    pub fn load(sessions_home: &Path) -> Result<Self, TaskError> {
        let path = tasks_file(sessions_home);
        let raw = match std::fs::read_to_string(&path) {
            Ok(raw) => raw,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                return Ok(TaskStore {
                    path,
                    tasks: HashMap::new(),
                });
            }
            Err(source) => {
                return Err(TaskError::Io {
                    path: path.clone(),
                    source,
                });
            }
        };
        let list: Vec<TaskDef> = serde_json::from_str(&raw).map_err(|source| TaskError::Json {
            path: path.clone(),
            source,
        })?;
        Ok(TaskStore {
            path,
            tasks: list.into_iter().map(|t| (t.id.clone(), t)).collect(),
        })
    }

    pub fn save(&self) -> Result<(), TaskError> {
        if let Some(parent) = self.path.parent() {
            std::fs::create_dir_all(parent).map_err(|source| TaskError::Io {
                path: parent.to_path_buf(),
                source,
            })?;
        }
        let mut list: Vec<TaskDef> = self.tasks.values().cloned().collect();
        list.sort_by_key(|t| t.created_at);
        let json = serde_json::to_string_pretty(&list).map_err(|source| TaskError::Json {
            path: self.path.clone(),
            source,
        })?;
        // Atomic + durable write: a plain `fs::write` truncates the file
        // before the new bytes land, so a crash or power loss mid-write
        // leaves `tasks.json` corrupt and unrecoverable. Write to a sibling
        // temp file, fsync its contents, rename over the real path, then
        // fsync the containing directory — on all platforms this crate
        // targets, `rename` onto an existing path is atomic, so readers
        // (this store, the server, the desktop app) only ever see the
        // fully-old or fully-new content, never a partial write; the fsyncs
        // ensure that content and the rename itself survive a crash right
        // after this call returns, not just torn-write-free while running.
        let tmp_path = self.path.with_extension("json.tmp");
        {
            let mut file = std::fs::File::create(&tmp_path).map_err(|source| TaskError::Io {
                path: tmp_path.clone(),
                source,
            })?;
            file.write_all(json.as_bytes())
                .map_err(|source| TaskError::Io {
                    path: tmp_path.clone(),
                    source,
                })?;
            file.sync_all().map_err(|source| TaskError::Io {
                path: tmp_path.clone(),
                source,
            })?;
        }
        std::fs::rename(&tmp_path, &self.path).map_err(|source| TaskError::Io {
            path: self.path.clone(),
            source,
        })?;
        if let Some(parent) = self.path.parent() {
            sync_directory(parent).map_err(|source| TaskError::Io {
                path: parent.to_path_buf(),
                source,
            })?;
        }
        Ok(())
    }

    pub fn get(&self, id: &str) -> Option<&TaskDef> {
        self.tasks.get(id)
    }

    /// Inserts or replaces; returns the previous definition when replacing.
    pub fn put(&mut self, task: TaskDef) -> Option<TaskDef> {
        self.tasks.insert(task.id.clone(), task)
    }

    pub fn remove(&mut self, id: &str) -> bool {
        self.tasks.remove(id).is_some()
    }

    pub fn len(&self) -> usize {
        self.tasks.len()
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.tasks.is_empty()
    }

    /// Every task, oldest first (created_at order, like the server API).
    pub fn all(&self) -> Vec<TaskDef> {
        let mut list: Vec<TaskDef> = self.tasks.values().cloned().collect();
        list.sort_by_key(|t| t.created_at);
        list
    }

    pub fn for_cwd(&self, cwd: &Path) -> Vec<TaskDef> {
        self.all().into_iter().filter(|t| t.cwd == cwd).collect()
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]
    use super::*;
    use chrono::{FixedOffset, LocalResult, NaiveDate, NaiveDateTime, Offset, TimeZone};

    fn local(y: i32, mo: u32, d: u32, h: u32, mi: u32) -> DateTime<chrono::Local> {
        chrono::Local
            .with_ymd_and_hms(y, mo, d, h, mi, 0)
            .earliest()
            .unwrap()
    }

    fn next(expr: &str, after: DateTime<chrono::Local>) -> DateTime<chrono::Local> {
        cron_next_after(expr, after).unwrap()
    }

    // ---- cron grammar matrix -------------------------------------------

    #[test]
    fn wildcard_every_step_minutes() {
        assert_eq!(
            next("*/15 * * * *", local(2026, 8, 24, 0, 0)),
            local(2026, 8, 24, 0, 15)
        );
        assert_eq!(
            next("*/15 * * * *", local(2026, 8, 24, 0, 16)),
            local(2026, 8, 24, 0, 30)
        );
    }

    #[test]
    fn wildcard_every_minute_is_strictly_after() {
        assert_eq!(
            next("* * * * *", local(2026, 8, 24, 12, 34)),
            local(2026, 8, 24, 12, 35)
        );
    }

    #[test]
    fn ranges_and_lists_combine() {
        // 04:05 on any weekday, Mon-Fri.
        assert_eq!(
            next("5 4 * * 1-5", local(2026, 8, 22, 10, 0)),
            local(2026, 8, 24, 4, 5)
        );
        assert_eq!(
            next("0 9-17 * * *", local(2026, 8, 24, 18, 0)),
            local(2026, 8, 25, 9, 0)
        );
        assert_eq!(
            next("0 0 1,15 * *", local(2026, 1, 16, 0, 0)),
            local(2026, 2, 1, 0, 0)
        );
        // Range with step.
        assert_eq!(
            next("0 0-23/6 * * *", local(2026, 8, 24, 7, 30)),
            local(2026, 8, 24, 12, 0)
        );
    }

    #[test]
    fn month_rollover_includes_feb_29() {
        assert_eq!(
            next("0 0 29 2 *", local(2026, 3, 1, 0, 0)),
            local(2028, 2, 29, 0, 0)
        );
        assert_eq!(
            next("0 0 29 2 *", local(2028, 3, 1, 0, 0)),
            local(2032, 2, 29, 0, 0)
        );
    }

    #[test]
    fn dom_and_dow_use_vixie_or_semantics_when_both_restricted() {
        // Friday the 13th OR "any Friday" — either restricted field fires.
        // 2026-01-14 is a Wednesday; the first Friday after is Jan 16.
        assert_eq!(
            next("0 12 13 * 5", local(2026, 1, 14, 0, 0)),
            local(2026, 1, 16, 12, 0)
        );
        // Fridays keep firing even when the 13th doesn't match…
        assert_eq!(
            next("0 12 13 * 5", local(2026, 1, 17, 0, 0)),
            local(2026, 1, 23, 12, 0)
        );
        // …and Friday Feb 13 matches BOTH restricted fields at once.
        assert_eq!(
            next("0 12 13 * 5", local(2026, 2, 12, 0, 0)),
            local(2026, 2, 13, 12, 0)
        );
    }

    #[test]
    fn dom_alone_is_conjunctive_with_unrestricted_fields() {
        assert_eq!(
            next("0 12 13 * *", local(2026, 1, 14, 0, 0)),
            local(2026, 2, 13, 12, 0)
        );
    }

    #[test]
    fn dow_seven_maps_to_sunday() {
        // 2026-08-22 is a Saturday.
        assert_eq!(
            next("0 12 * * 7", local(2026, 8, 22, 0, 0)),
            local(2026, 8, 23, 12, 0)
        );
        assert_eq!(
            next("0 12 * * 0", local(2026, 8, 22, 0, 0)),
            local(2026, 8, 23, 12, 0)
        );
    }

    #[test]
    fn parse_errors_are_typed_strings() {
        for bad in [
            "* * * *",
            "* * * * * *",
            "",
            "60 * * * *",
            "-1 * * * *",
            "* 24 * * *",
            "* * 0 * *",
            "* * * 13 *",
            "* * * * 8",
            "*/0 * * * *",
            "5-1 * * * *",
            "1,,2 * * * *",
            "abc * * * *",
            "5/x * * * *",
            "5/10 * * * *",
        ] {
            assert!(
                CronExpr::parse(bad).is_err(),
                "expected '{bad}' to be rejected"
            );
        }
    }

    #[test]
    fn unreachable_date_hits_horizon_error() {
        // Feb 31 does not exist in any year.
        let err = cron_next_after("0 0 31 2 *", local(2026, 1, 1, 0, 0)).unwrap_err();
        assert!(err.contains("never fires"), "{err}");
    }

    /// Synthetic zone modeling a spring-forward gap (local 02:00–02:59 of
    /// 2026-03-08 do not exist) and a fall-back fold (local 01:00–01:59 of
    /// 2026-11-01 occur twice), so gap/fold behavior is testable without
    /// touching process TZ state.
    #[derive(Debug, Clone, Copy)]
    struct GapTz;

    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    struct GapOffset(FixedOffset);

    const EAST: FixedOffset = FixedOffset::east_opt(3600).unwrap();
    const WEST: FixedOffset = FixedOffset::east_opt(0).unwrap();

    impl TimeZone for GapTz {
        type Offset = GapOffset;
        fn from_offset(_offset: &GapOffset) -> Self {
            GapTz
        }
        fn offset_from_utc_date(&self, _utc: &chrono::NaiveDate) -> GapOffset {
            GapOffset(EAST)
        }
        fn offset_from_utc_datetime(&self, _utc: &NaiveDateTime) -> GapOffset {
            GapOffset(EAST)
        }
        fn offset_from_local_date(&self, local: &chrono::NaiveDate) -> LocalResult<GapOffset> {
            // A date inherits its gap/fold status from some instant on it.
            let probe = local.and_hms_opt(12, 0, 0).unwrap();
            self.offset_from_local_datetime(&probe)
        }
        fn offset_from_local_datetime(&self, local: &NaiveDateTime) -> LocalResult<GapOffset> {
            use chrono::NaiveDate;
            let gap_day = NaiveDate::from_ymd_opt(2026, 3, 8).unwrap();
            let fold_day = NaiveDate::from_ymd_opt(2026, 11, 1).unwrap();
            let t = local.time();
            if local.date() == gap_day && t.hour() == 2 {
                return LocalResult::None;
            }
            if local.date() == fold_day && t.hour() == 1 {
                // (earliest, latest): the larger offset yields the earlier
                // instant, so the pre-fold EAST pass comes first.
                return LocalResult::Ambiguous(GapOffset(EAST), GapOffset(WEST));
            }
            LocalResult::Single(GapOffset(EAST))
        }
    }

    impl chrono::Offset for GapOffset {
        fn fix(&self) -> FixedOffset {
            self.0
        }
    }

    fn gap_next(expr: &str, after: NaiveDateTime) -> Option<DateTime<GapTz>> {
        next_fire(
            &CronExpr::parse(expr).unwrap(),
            GapTz.from_utc_datetime(&after),
        )
    }

    #[test]
    fn dst_gap_skips_forward_to_first_existing_match() {
        let before_gap = NaiveDate::from_ymd_opt(2026, 3, 8)
            .unwrap()
            .and_hms_opt(1, 30, 0)
            .unwrap();
        // Hourly job: 02:00 does not exist → fires at 03:00.
        let fired = gap_next("0 * * * *", before_gap).unwrap();
        assert_eq!(
            fired.naive_local(),
            NaiveDate::from_ymd_opt(2026, 3, 8)
                .unwrap()
                .and_hms_opt(3, 0, 0)
                .unwrap()
        );
        // Daily-at-02:30 job: today's 02:30 doesn't exist → tomorrow's.
        let daily = gap_next("30 2 * * *", before_gap).unwrap();
        assert_eq!(
            daily.naive_local(),
            NaiveDate::from_ymd_opt(2026, 3, 9)
                .unwrap()
                .and_hms_opt(2, 30, 0)
                .unwrap()
        );
    }

    #[test]
    fn dst_fold_resolves_to_earliest_instant() {
        let before_fold = NaiveDate::from_ymd_opt(2026, 11, 1)
            .unwrap()
            .and_hms_opt(0, 30, 0)
            .unwrap();
        let fired = gap_next("45 1 * * *", before_fold).unwrap();
        assert_eq!(
            fired.naive_local(),
            NaiveDate::from_ymd_opt(2026, 11, 1)
                .unwrap()
                .and_hms_opt(1, 45, 0)
                .unwrap()
        );
        // Earliest pass carries the pre-fold offset (EAST here).
        assert_eq!(fired.offset().fix(), EAST);
    }

    // ---- task validation + store ----------------------------------------

    fn base_task() -> TaskDef {
        TaskDef {
            id: uuid::Uuid::now_v7().to_string(),
            name: "nightly".into(),
            prompt: "tidy the repo".into(),
            interval_secs: 3600,
            enabled: true,
            cwd: PathBuf::from("/tmp/ws"),
            created_at: Utc::now(),
            last_run_at: None,
            last_session_id: None,
            last_summary: None,
            last_result_id: None,
            last_run_status: None,
            last_delivery_state: None,
            last_wt: None,
            deliver_to: None,
            schedule: None,
            script: None,
            model_pin: None,
            agent_profile_id: None,
            agent_profile_revision: None,
        }
    }

    #[test]
    fn prompt_xor_script_validation_matrix() {
        let mut both = base_task();
        both.script = Some("echo tick".into());
        assert!(matches!(
            both.validate(),
            Err(TaskError::PromptScriptXor { .. })
        ));

        let neither = TaskDef {
            prompt: "   ".into(),
            ..base_task()
        };
        assert!(matches!(
            neither.validate(),
            Err(TaskError::PromptScriptXor { .. })
        ));

        assert!(base_task().validate().is_ok());
        let watchdog = TaskDef {
            prompt: String::new(),
            script: Some("curl -sf http://x/health".into()),
            ..base_task()
        };
        assert!(watchdog.validate().is_ok());
    }

    #[test]
    fn bad_cron_rejected_by_validate() {
        let mut t = base_task();
        t.schedule = Some("99 * * * *".into());
        let err = t.validate().unwrap_err();
        assert!(
            matches!(err, TaskError::BadSchedule { ref expr, .. } if expr == "99 * * * *"),
            "{err}"
        );
        t.schedule = Some("*/15 * * * *".into());
        assert!(t.validate().is_ok());
    }

    #[test]
    fn a_tasks_file_written_before_added_fields_loads_and_roundtrips() {
        let dir = tempfile::tempdir().unwrap();
        let raw = r#"[
          {
            "id": "018f0000-0000-7000-8000-000000000001",
            "name": "standup-notes",
            "prompt": "summarize yesterday",
            "interval_secs": 86400,
            "enabled": true,
            "cwd": "/Users/me/proj",
            "created_at": "2026-07-01T09:00:00Z",
            "last_run_at": "2026-08-01T09:00:00Z",
            "last_session_id": "abc",
            "last_summary": "done",
            "last_result_id": "result-1",
            "last_wt": { "path": "/tmp/wt", "branch": "vak/abc" },
            "deliver_to": "log:ops"
          }
        ]"#;
        std::fs::create_dir_all(dir.path()).unwrap();
        std::fs::write(tasks_file(dir.path()), raw).unwrap();

        let store = TaskStore::load(dir.path()).unwrap();
        assert_eq!(store.len(), 1);
        let t = store.get("018f0000-0000-7000-8000-000000000001").unwrap();
        assert_eq!(t.prompt, "summarize yesterday");
        assert_eq!(t.interval_secs, 86400);
        assert_eq!(t.last_wt.as_ref().unwrap().branch, "vak/abc");
        assert_eq!(t.deliver_to.as_deref(), Some("log:ops"));
        assert_eq!(t.schedule, None);
        assert_eq!(t.script, None);
        assert_eq!(t.model_pin, None);

        store.save().unwrap();
        let reloaded = TaskStore::load(dir.path()).unwrap();
        assert_eq!(reloaded.all().len(), 1);
        assert_eq!(
            reloaded.get(t.id.as_str()).unwrap().last_wt,
            t.last_wt,
            "legacy last_wt must survive a load/save cycle"
        );
    }

    #[test]
    fn new_fields_roundtrip_through_store() {
        let dir = tempfile::tempdir().unwrap();
        let mut store = TaskStore::load(dir.path()).unwrap();
        assert!(store.is_empty());

        let mut t = base_task();
        t.prompt = String::new();
        t.script = Some("systemctl is-active nginx".into());
        t.schedule = Some("0 7 * * 1-5".into());
        t.model_pin = Some("haiku-fast".into());
        store.put(t.clone());
        store.save().unwrap();

        let back = TaskStore::load(dir.path()).unwrap();
        let got = back.get(&t.id).unwrap();
        assert_eq!(got.script.as_deref(), Some("systemctl is-active nginx"));
        assert_eq!(got.schedule.as_deref(), Some("0 7 * * 1-5"));
        assert_eq!(got.model_pin.as_deref(), Some("haiku-fast"));
        assert!(got.validate().is_ok());
    }

    #[test]
    fn missing_file_is_empty_store_and_save_creates_it() {
        let dir = tempfile::tempdir().unwrap();
        let mut store = TaskStore::load(dir.path()).unwrap();
        assert!(store.is_empty());
        store.put(base_task());
        store.save().unwrap();
        assert!(tasks_file(dir.path()).is_file());
    }

    #[test]
    fn save_is_atomic_and_leaves_no_tmp_file_behind() {
        let dir = tempfile::tempdir().unwrap();
        let mut store = TaskStore::load(dir.path()).unwrap();
        store.put(base_task());
        store.save().unwrap();
        assert!(tasks_file(dir.path()).is_file());
        assert!(
            !dir.path().join("tasks.json.tmp").exists(),
            "the temp file used for the atomic rename must not survive a successful save"
        );

        // A second save (overwrite path) must round-trip cleanly too, and
        // still leave no tmp file — this is the path a real crash-mid-write
        // would otherwise corrupt with a plain `fs::write`.
        store.put(base_task());
        store.save().unwrap();
        let reloaded = TaskStore::load(dir.path()).unwrap();
        assert_eq!(reloaded.tasks.len(), 2);
        assert!(!dir.path().join("tasks.json.tmp").exists());
    }

    #[test]
    fn corrupt_file_is_a_typed_error_not_silence() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(tasks_file(dir.path()), "{not json").unwrap();
        assert!(matches!(
            TaskStore::load(dir.path()),
            Err(TaskError::Json { .. })
        ));
    }

    #[test]
    fn for_cwd_filters_and_sorts_by_created_at() {
        let ws_a = PathBuf::from("/ws/a");
        let ws_b = PathBuf::from("/ws/b");
        let mut early = base_task();
        early.id = "early".into();
        early.cwd = ws_a.clone();
        early.created_at = Utc::now() - chrono::Duration::hours(2);
        let mut late = early.clone();
        late.id = "late".into();
        late.created_at = Utc::now();
        let mut other = base_task();
        other.id = "other".into();
        other.cwd = ws_b;

        let mut map = HashMap::new();
        for t in [early, late, other] {
            map.insert(t.id.clone(), t);
        }
        let store = TaskStore {
            path: PathBuf::from("unused"),
            tasks: map,
        };
        let mine = store.for_cwd(&ws_a);
        assert_eq!(mine.len(), 2);
        assert_eq!(mine[0].id, "early", "oldest first");
        assert_eq!(mine[1].id, "late");
    }
}
