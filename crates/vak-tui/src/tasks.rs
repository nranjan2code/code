//! `/tasks` and `/proposals`: read-mostly scheduled-task management plus
//! learned-skill proposal review, over the base's API. Cron next-fire is a
//! local 5-field vixie-style port (Local time, dom/dow OR rule).

use std::collections::BTreeSet;

use chrono::{DateTime, Datelike, Local, TimeZone, Timelike};
use vak_client::types::{ProposalInfo, Task};

use crate::data::ClientData;
use crate::render::Screen;
use crate::state::ModalView;

const TASKS_FOOTER: &str =
    "/tasks enable|disable <name> toggles · creation/editing lives in `vakcoder tasks`";

/// How far ahead the next-fire scan searches before giving up. Cron can
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
struct CronExpr {
    minutes: FieldSet,
    hours: FieldSet,
    days_of_month: FieldSet,
    months: FieldSet,
    days_of_week: FieldSet,
}

impl CronExpr {
    fn parse(expr: &str) -> Result<Self, String> {
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
pub fn cron_next_after(expr: &str, after: DateTime<Local>) -> Result<DateTime<Local>, String> {
    let parsed = CronExpr::parse(expr)?;
    next_fire(&parsed, after)
        .ok_or_else(|| format!("expression '{expr}' never fires within {CRON_HORIZON_DAYS} days"))
}

fn next_fire(expr: &CronExpr, after: DateTime<Local>) -> Option<DateTime<Local>> {
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

/// `MM-DD HH:MM` preview of a cron expression's next fire from now, or "?"
/// when the expression is invalid or never fires.
pub fn next_fire_display(expr: &str) -> String {
    next_fire_display_at(expr, Local::now())
}

fn next_fire_display_at(expr: &str, now: DateTime<Local>) -> String {
    cron_next_after(expr, now)
        .map(|dt| dt.format("%m-%d %H:%M").to_string())
        .unwrap_or_else(|_| "?".to_string())
}

/// Humanized interval formatting shared with surfaces that still carry an
/// interval-seconds schedule (CLI parity).
pub fn fmt_interval(secs: u64) -> String {
    if secs == 0 {
        return "0s".to_string();
    }
    if secs.is_multiple_of(86_400) {
        format!("{}d", secs / 86_400)
    } else if secs.is_multiple_of(3600) {
        format!("{}h", secs / 3600)
    } else if secs.is_multiple_of(60) {
        format!("{}m", secs / 60)
    } else {
        format!("{secs}s")
    }
}

/// One row per task: name · kind · schedule · enabled · next cron fire from
/// `now`. Tasks without a cron expression show "-" in both columns — their
/// cadence lives server-side where this surface cannot see it.
pub fn task_rows(tasks: &[Task], now: DateTime<Local>) -> Vec<String> {
    const HEADER: [&str; 5] = ["name", "kind", "schedule", "on", "next fire"];
    let mut cells: Vec<[String; 5]> = Vec::with_capacity(tasks.len());
    for t in tasks {
        let sched = t.schedule.clone().unwrap_or_else(|| "-".to_string());
        let next = match (t.schedule.as_deref(), t.enabled) {
            (Some(expr), true) => next_fire_display_at(expr, now),
            _ => "-".to_string(),
        };
        cells.push([
            t.name.clone(),
            if t.is_watchdog() { "script" } else { "prompt" }.to_string(),
            sched,
            if t.enabled { "✓" } else { "✗" }.to_string(),
            next,
        ]);
    }
    let widths: [usize; 5] = std::array::from_fn(|i| {
        HEADER[i].chars().count().max(
            cells
                .iter()
                .map(|row| row[i].chars().count())
                .max()
                .unwrap_or(0),
        )
    });
    let line = |row: &[String; 5]| {
        row.iter()
            .zip(widths)
            .map(|(c, w)| format!("{c:<w$}"))
            .collect::<Vec<_>>()
            .join("  ")
    };
    let header = HEADER.map(String::from);
    std::iter::once(line(&header))
        .chain(cells.iter().map(line))
        .collect()
}

/// Match a task by exact name (case-insensitive), falling back to a unique
/// case-insensitive name prefix.
fn resolve_task<'a>(tasks: &'a [Task], given: &str) -> Result<&'a Task, String> {
    if let Some(t) = tasks.iter().find(|t| t.name.eq_ignore_ascii_case(given)) {
        return Ok(t);
    }
    let hits: Vec<&Task> = tasks
        .iter()
        .filter(|t| t.name.to_lowercase().starts_with(&given.to_lowercase()))
        .collect();
    match hits.as_slice() {
        [one] => Ok(one),
        [] => Err(format!("no task named '{given}'")),
        many => Err(format!(
            "'{given}' matches several tasks ({}) — use more of the name",
            many.iter()
                .map(|t| t.name.as_str())
                .collect::<Vec<_>>()
                .join(", ")
        )),
    }
}

async fn set_task_enabled(
    data: &ClientData,
    id: &str,
    name: &str,
    enable: bool,
    screen: &mut Screen,
) {
    let call = if enable {
        data.client().task_enable(id).await
    } else {
        data.client().task_disable(id).await
    };
    match call {
        Ok(status) if status.is_success() => screen.success(&format!(
            "{} task '{name}'",
            if enable {
                "✓ enabled"
            } else {
                "✗ disabled"
            }
        )),
        Ok(status) => screen.error(&format!(
            "{} task '{name}' failed: HTTP {status}",
            if enable { "enable" } else { "disable" }
        )),
        Err(e) => screen.error(&format!(
            "{} task '{name}' failed: {e}",
            if enable { "enable" } else { "disable" }
        )),
    }
}

/// `/tasks [enable|disable <name>]`. Bare lists the table; toggles resolve
/// names to ids case-insensitively (exact first, then unique prefix).
pub async fn handle_tasks(data: &ClientData, arg: Option<&str>, screen: &mut Screen) {
    let tasks = match data.tasks().await {
        Ok(tasks) => tasks,
        Err(e) => {
            screen.error(&format!("tasks unavailable: {e}"));
            return;
        }
    };
    match arg.map(str::trim).filter(|a| !a.is_empty()) {
        None => {
            let rows = if tasks.is_empty() {
                vec![
                    "no scheduled tasks".to_string(),
                    "create them with `vakcoder tasks` on the CLI or the desktop Tasks page"
                        .to_string(),
                ]
            } else {
                task_rows(&tasks, chrono::Local::now())
            };
            screen.panel("scheduled tasks", &rows, TASKS_FOOTER);
        }
        Some(argstr) => {
            let mut tokens = argstr.splitn(2, char::is_whitespace);
            let action = tokens.next().unwrap_or("");
            let name = tokens.next().map(str::trim).unwrap_or("");
            if !matches!(action, "enable" | "disable") || name.is_empty() {
                screen.error("usage: /tasks · /tasks enable <name> · /tasks disable <name>");
                return;
            }
            let enable = action == "enable";
            match resolve_task(&tasks, name) {
                Err(e) => screen.error(&e),
                Ok(task) => {
                    set_task_enabled(data, &task.id, &task.name, enable, screen).await;
                }
            }
        }
    }
}

/// One `/proposals` modal block per pending skill proposal.
pub fn proposal_rows(proposals: &[ProposalInfo]) -> Vec<String> {
    proposals
        .iter()
        .flat_map(|p| {
            [
                format!("/{}", p.id),
                format!("  {}", p.name),
                format!("  {}", p.description),
                String::new(),
            ]
        })
        .collect()
}

/// What `/proposals` leaves for the caller to do after messages are printed.
pub enum ProposalsOutcome {
    Handled,
    /// A listing to open in the modal layer.
    Modal(ModalView),
}

/// `/proposals [promote|reject <id>]`. Bare lists pending learned-skill
/// proposals; promote installs, reject discards.
pub async fn handle_proposals(
    data: &ClientData,
    arg: Option<&str>,
    screen: &mut Screen,
) -> ProposalsOutcome {
    let parsed = arg.and_then(|a| {
        let mut it = a.split_whitespace();
        let action = it.next()?;
        let id = it.next()?.trim();
        Some((action.to_lowercase(), id.to_string()))
    });
    match parsed {
        Some((action, id)) if action == "promote" || action == "accept" => {
            match data.client().proposal_promote(&id).await {
                Ok(resp) => {
                    let name = resp["promoted"].as_str().unwrap_or(&id);
                    screen.success(&format!("skill '{name}' promoted to skills"));
                }
                Err(e) => screen.error(&e.to_string()),
            }
            ProposalsOutcome::Handled
        }
        Some((action, id)) if action == "reject" || action == "deny" => {
            match data.client().proposal_reject(&id).await {
                Ok(_) => screen.accent(&format!("proposal {id} rejected")),
                Err(e) => screen.error(&e.to_string()),
            }
            ProposalsOutcome::Handled
        }
        Some((action, _)) => {
            screen.error(&format!(
                "unknown proposals action '{action}' — promote <id>, reject <id>"
            ));
            ProposalsOutcome::Handled
        }
        None => {
            let proposals = match data.client().proposals_list().await {
                Ok(resp) => resp.proposals,
                Err(e) => {
                    screen.error(&format!("proposals unavailable: {e}"));
                    return ProposalsOutcome::Handled;
                }
            };
            let rows = if proposals.is_empty() {
                vec![
                    "no pending skill proposals".to_string(),
                    "the model proposes skills after repeated successful patterns; review them here"
                        .to_string(),
                ]
            } else {
                proposal_rows(&proposals)
            };
            ProposalsOutcome::Modal(ModalView {
                title: format!("skill proposals · {} pending", proposals.len()),
                rows,
                scroll: 0,
                footer:
                    "/proposals promote <id> installs · /proposals reject <id> discards · Esc close"
                        .to_string(),
                ..Default::default()
            })
        }
    }
}

#[cfg(test)]
mod task_tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]
    use super::*;
    use vak_client::types::ProposalInfo;

    fn task(name: &str) -> Task {
        Task {
            id: format!("test-{name}"),
            name: name.into(),
            interval_secs: 3600,
            enabled: true,
            schedule: None,
            prompt: "tidy".into(),
            script: None,
            model_pin: None,
        }
    }

    #[test]
    fn fmt_interval_humanizes_common_steps() {
        assert_eq!(fmt_interval(45), "45s");
        assert_eq!(fmt_interval(300), "5m");
        assert_eq!(fmt_interval(3600), "1h");
        assert_eq!(fmt_interval(7200), "2h");
        assert_eq!(fmt_interval(86_400), "1d");
        assert_eq!(fmt_interval(0), "0s");
    }

    #[test]
    fn cron_next_after_is_strictly_after_and_local() {
        let now = Local
            .with_ymd_and_hms(2026, 8, 24, 12, 0, 0)
            .earliest()
            .unwrap();
        assert_eq!(
            cron_next_after("0 9 * * *", now).unwrap(),
            Local
                .with_ymd_and_hms(2026, 8, 25, 9, 0, 0)
                .earliest()
                .unwrap()
        );
        // Same-minute fire is excluded: strictly-after semantics.
        assert_eq!(
            cron_next_after("0 12 * * *", now).unwrap(),
            Local
                .with_ymd_and_hms(2026, 8, 25, 12, 0, 0)
                .earliest()
                .unwrap()
        );
        // Dom/dow OR rule: Friday the 13th OR any Monday.
        let got = cron_next_after("0 0 13 * 1", now).unwrap();
        assert!(
            got.day() == 13 || got.weekday() == chrono::Weekday::Mon,
            "{got}"
        );
        // Invalid expressions are errors, never guesses.
        assert!(cron_next_after("99 * * * *", now).is_err());
        assert!(cron_next_after("* * * *", now).is_err());
    }

    #[test]
    fn task_table_lists_kind_schedule_enabled_and_next_fire() {
        let now = Local
            .with_ymd_and_hms(2026, 8, 24, 12, 0, 0)
            .earliest()
            .unwrap();

        let mut cron_task = task("standup");
        cron_task.schedule = Some("0 9 * * *".into());

        let mut watchdog = task("watch");
        watchdog.script = Some("true".into());
        watchdog.schedule = Some("*/5 * * * *".into());

        let mut off = task("paused");
        off.enabled = false;

        let rows = task_rows(&[cron_task, watchdog, off], now);

        // Header + one row per task, columns aligned.
        assert!(rows[0].contains("next fire"));
        assert_eq!(rows.len(), 4);

        let standup = &rows[1];
        assert!(standup.contains("standup"));
        assert!(standup.contains("prompt"));
        assert!(standup.contains("0 9 * * *"));
        assert!(standup.contains("✓"));
        assert!(
            standup.contains("08-25 09:00"),
            "cron next fire preview from the given now: {standup}"
        );

        assert!(rows[2].contains("script"), "script task kind: {}", rows[2]);
        assert!(rows[2].contains("*/5 * * * *"));

        let paused = &rows[3];
        assert!(paused.contains("✗"), "disabled mark: {paused}");
        assert!(
            paused.trim_end().ends_with('-'),
            "no next fire for disabled/cron-less tasks: {paused}"
        );
    }

    #[test]
    fn task_names_resolve_case_insensitively_by_exact_or_unique_prefix() {
        let mut nightly = task("nightly-digest");
        nightly.id = "id-nightly".into();
        let mut notes = task("notes-cleanup");
        notes.id = "id-notes".into();
        let tasks = vec![nightly, notes];

        assert_eq!(
            resolve_task(&tasks, "NIGHTLY-DIGEST").unwrap().id,
            "id-nightly"
        );
        assert_eq!(resolve_task(&tasks, "nig").unwrap().id, "id-nightly");
        assert_eq!(resolve_task(&tasks, "NOTES").unwrap().id, "id-notes");
        // Ambiguous prefix is refused, not silently resolved.
        assert!(resolve_task(&tasks, "n").is_err());
        assert_eq!(
            resolve_task(&tasks, "ghost").err(),
            Some("no task named 'ghost'".to_string())
        );
    }

    #[test]
    fn proposal_rows_block_per_proposal() {
        let props = vec![ProposalInfo {
            id: "rust-fixes".into(),
            name: "Rust fixes".into(),
            description: "Prefers rustfix batches".into(),
        }];
        assert_eq!(
            proposal_rows(&props),
            vec![
                "/rust-fixes",
                "  Rust fixes",
                "  Prefers rustfix batches",
                "",
            ]
        );
    }
}
