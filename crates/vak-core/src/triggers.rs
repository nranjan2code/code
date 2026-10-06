//! Triggers (plan M4.3, docs/design/73-data-architecture-and-lifecycle.md
//! §8): the one model of when Vakyartha starts work by itself. A trigger is
//! desired state, a versioned Document `triggers/<trg>` in the tenant store
//! (`vak_session::documents`); everything a run of it did is a run record
//! (`vak_session::runs`) naming the trigger, so "last run" is a query and
//! nothing about a run is written back onto the trigger.

use std::collections::BTreeSet;
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

/// Named-IANA equivalent of [`cron_next_after`].
pub fn cron_next_after_timezone(
    expr: &str,
    after: DateTime<Utc>,
    timezone: &str,
) -> Result<DateTime<Utc>, String> {
    let tz: chrono_tz::Tz = timezone
        .parse()
        .map_err(|_| format!("unknown IANA timezone '{timezone}'"))?;
    let local_after = after.with_timezone(&tz);
    next_fire(&CronExpr::parse(expr)?, local_after)
        .map(|value| value.with_timezone(&Utc))
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

/// What a trigger does when it fires.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum TriggerAction {
    /// An Agent turn with this prompt; `model_pin` dispatches only that
    /// model and never escalates.
    Prompt {
        text: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        model_pin: Option<String>,
    },
    /// A watchdog shell one-liner, run brokered; empty stdout costs nothing.
    Script { command: String },
    /// Polls an intake source (plan M6.5): fetch, parse in the worker, take
    /// what is new. No model is involved.
    SourcePoll { source: vak_session::ids::SourceId },
}

/// When a scheduled trigger's slots fall.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Schedule {
    /// 5-field cron (`m h dom mon dow`), in `timezone` when named, else in
    /// this machine's local time.
    Cron {
        expr: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        timezone: Option<String>,
    },
    /// Slots at `anchor + k * every_secs`.
    Interval {
        every_secs: u64,
        anchor: DateTime<Utc>,
    },
    /// One slot.
    Once { at: DateTime<Utc> },
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum TriggerKind {
    Schedule {
        schedule: Schedule,
    },
    /// Fires only when someone runs it.
    Manual,
}

/// What a slot whose run was interrupted (its holder stopped) gets.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OnCrash {
    #[default]
    Skip,
    RetryOnce,
}

/// The shortest interval a scheduled trigger may have.
pub const MIN_INTERVAL_SECS: u64 = 60;

#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct Trigger {
    pub id: vak_session::ids::TriggerId,
    pub name: String,
    /// The Agent whose work this is (`AgentDefinition::id`).
    pub agent: String,
    /// The Agent revision a routine was pinned to; a changed Agent refuses
    /// the run.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub agent_revision: Option<u64>,
    /// The space it runs in (`vak_config::spaces`), never a folder path.
    pub space: String,
    pub enabled: bool,
    pub kind: TriggerKind,
    pub action: TriggerAction,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub deliver_to: Option<String>,
    #[serde(default)]
    pub on_crash: OnCrash,
    /// A read-only mail/calendar routine's account and operations; its
    /// `routine_id` is this trigger's UUID, the vault's namespace for it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub scope: Option<vak_mail_calendar::RoutineScope>,
    pub created_at: DateTime<Utc>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub created_by: Option<vak_session::ids::PrincipalId>,
}

#[derive(Debug, thiserror::Error)]
pub enum TriggerError {
    #[error("automation '{name}': {reason}")]
    Invalid { name: String, reason: String },
    #[error("invalid schedule '{expr}': {reason}")]
    BadSchedule { expr: String, reason: String },
    #[error("mail/calendar routines require a valid read scope and pinned Agent revision")]
    InvalidMailCalendarScope,
    #[error("no automation {0}")]
    NotFound(String),
    #[error("automation store: {0}")]
    Store(String),
    #[error("automation {name} cannot be read: {reason}")]
    Corrupt { name: String, reason: String },
}

impl Trigger {
    /// The folder this machine binds the trigger's space to; `None` when the
    /// space has no folder here, which a run reports as its refusal.
    pub fn workspace(&self) -> Option<PathBuf> {
        vak_config::spaces::bindings(&self.space).into_iter().next()
    }

    pub fn schedule(&self) -> Option<&Schedule> {
        match &self.kind {
            TriggerKind::Schedule { schedule } => Some(schedule),
            TriggerKind::Manual => None,
        }
    }

    pub fn prompt(&self) -> Option<&str> {
        match &self.action {
            TriggerAction::Prompt { text, .. } => Some(text),
            TriggerAction::Script { .. } | TriggerAction::SourcePoll { .. } => None,
        }
    }

    pub fn script(&self) -> Option<&str> {
        match &self.action {
            TriggerAction::Script { command } => Some(command),
            TriggerAction::Prompt { .. } | TriggerAction::SourcePoll { .. } => None,
        }
    }

    /// The intake source a `source_poll` trigger polls.
    pub fn source(&self) -> Option<vak_session::ids::SourceId> {
        match &self.action {
            TriggerAction::SourcePoll { source } => Some(*source),
            TriggerAction::Prompt { .. } | TriggerAction::Script { .. } => None,
        }
    }

    pub fn model_pin(&self) -> Option<&str> {
        match &self.action {
            TriggerAction::Prompt { model_pin, .. } => model_pin.as_deref(),
            TriggerAction::Script { .. } | TriggerAction::SourcePoll { .. } => None,
        }
    }

    /// Structural validation. Pure; never touches the store.
    pub fn validate(&self) -> Result<(), TriggerError> {
        let invalid = |reason: &str| TriggerError::Invalid {
            name: self.name.clone(),
            reason: reason.into(),
        };
        if self.name.trim().is_empty() {
            return Err(invalid("a name is required"));
        }
        if self.agent.trim().is_empty() {
            return Err(invalid("an Agent is required"));
        }
        match &self.action {
            TriggerAction::Prompt { text, .. } if text.trim().is_empty() => {
                return Err(invalid("the prompt is empty"));
            }
            TriggerAction::Script { command } if command.trim().is_empty() => {
                return Err(invalid("the script is empty"));
            }
            // A poll delivers nothing itself: what it takes reaches people
            // through intake's alerts.
            TriggerAction::SourcePoll { .. }
                if self.deliver_to.is_some() || self.scope.is_some() =>
            {
                return Err(invalid(
                    "a source poll has no delivery target or mail scope",
                ));
            }
            _ => {}
        }
        if let Some(scope) = &self.scope
            && (self.agent_revision.is_none_or(|revision| revision == 0)
                || self.script().is_some()
                || self.deliver_to.is_some()
                || scope.routine_id != self.id.uuid().to_string()
                || scope.validate().is_err())
        {
            return Err(TriggerError::InvalidMailCalendarScope);
        }
        match self.schedule() {
            Some(Schedule::Cron { expr, timezone }) => {
                CronExpr::parse(expr).map_err(|reason| TriggerError::BadSchedule {
                    expr: expr.clone(),
                    reason,
                })?;
                if let Some(zone) = timezone.as_deref()
                    && zone.parse::<chrono_tz::Tz>().is_err()
                {
                    return Err(TriggerError::BadSchedule {
                        expr: zone.into(),
                        reason: "unknown IANA timezone".into(),
                    });
                }
            }
            Some(Schedule::Interval { every_secs, .. }) if *every_secs < MIN_INTERVAL_SECS => {
                return Err(TriggerError::BadSchedule {
                    expr: format!("every {every_secs}s"),
                    reason: format!("the shortest interval is {MIN_INTERVAL_SECS} seconds"),
                });
            }
            _ => {}
        }
        Ok(())
    }

    /// The first slot strictly after `after`, `None` when there is none
    /// (a manual trigger, a past one-shot, an unsatisfiable cron).
    pub fn next_slot_after(&self, after: DateTime<Utc>) -> Option<DateTime<Utc>> {
        match self.schedule()? {
            Schedule::Cron { expr, timezone } => match timezone.as_deref() {
                Some(zone) => cron_next_after_timezone(expr, after, zone).ok(),
                None => cron_next_after(expr, after.with_timezone(&chrono::Local))
                    .ok()
                    .map(|next| next.with_timezone(&Utc)),
            },
            Schedule::Interval { every_secs, anchor } => {
                let every = i64::try_from(*every_secs).ok()?.max(1);
                if after < *anchor {
                    return Some(*anchor);
                }
                let elapsed = after.signed_duration_since(*anchor).num_seconds();
                let k = elapsed / every + 1;
                Some(*anchor + chrono::Duration::seconds(k * every))
            }
            Schedule::Once { at } => (*at > after).then_some(*at),
        }
    }

    /// Up to `count` slots strictly after `after`, in order.
    pub fn slots_after(&self, after: DateTime<Utc>, count: usize) -> Vec<DateTime<Utc>> {
        let mut slots = Vec::new();
        let mut cursor = after;
        while slots.len() < count {
            let Some(next) = self.next_slot_after(cursor) else {
                break;
            };
            slots.push(next);
            cursor = next;
        }
        slots
    }
}

/// Where the triggers of the data home at `shared` are kept: the Document
/// names `triggers/<trg>`.
fn trigger_path(shared: &vak_config::scope::SharedScope, id: &str) -> PathBuf {
    shared.triggers().join(id)
}

fn decode(name: &str, text: &str) -> Result<Trigger, TriggerError> {
    serde_json::from_str(text).map_err(|error| TriggerError::Corrupt {
        name: name.into(),
        reason: error.to_string(),
    })
}

/// Every trigger, oldest first. One that cannot be read is an error, never
/// skipped: a scheduler that silently dropped one would lose its work.
pub fn list(shared: &vak_config::scope::SharedScope) -> Result<Vec<Trigger>, TriggerError> {
    let mut triggers = Vec::new();
    for path in vak_session::documents::under(&shared.triggers()) {
        let name = path.display().to_string();
        let Some(text) = vak_session::documents::read(&path).map_err(TriggerError::Store)? else {
            continue;
        };
        triggers.push(decode(&name, &text)?);
    }
    triggers.sort_by_key(|trigger| trigger.created_at);
    Ok(triggers)
}

pub fn get(
    shared: &vak_config::scope::SharedScope,
    id: &str,
) -> Result<Option<Trigger>, TriggerError> {
    let path = trigger_path(shared, id);
    match vak_session::documents::read(&path).map_err(TriggerError::Store)? {
        Some(text) => decode(id, &text).map(Some),
        None => Ok(None),
    }
}

/// Saves a new trigger; fails if one with its id exists.
pub fn create(
    shared: &vak_config::scope::SharedScope,
    trigger: &Trigger,
) -> Result<(), TriggerError> {
    trigger.validate()?;
    let text =
        serde_json::to_string(trigger).map_err(|error| TriggerError::Store(error.to_string()))?;
    vak_session::documents::create(&trigger_path(shared, &trigger.id.to_string()), &text)
        .map_err(TriggerError::Store)
}

/// Applies `change` to the current trigger and saves the result as a new
/// version; re-reads and retries when another writer saved in between.
pub fn update(
    shared: &vak_config::scope::SharedScope,
    id: &str,
    mut change: impl FnMut(&mut Trigger) -> Result<(), TriggerError>,
) -> Result<Trigger, TriggerError> {
    let path = trigger_path(shared, id);
    let mut failure: Option<TriggerError> = None;
    let saved = vak_session::documents::update(&path, |current| {
        let Some(text) = current else {
            failure = Some(TriggerError::NotFound(id.into()));
            return Ok(None);
        };
        let mut trigger = match decode(id, text) {
            Ok(trigger) => trigger,
            Err(error) => {
                failure = Some(error);
                return Ok(None);
            }
        };
        let fixed = (trigger.id.to_string(), trigger.created_at);
        if let Err(error) = change(&mut trigger).and_then(|()| trigger.validate()) {
            failure = Some(error);
            return Ok(None);
        }
        if (trigger.id.to_string(), trigger.created_at) != fixed {
            failure = Some(TriggerError::Invalid {
                name: trigger.name.clone(),
                reason: "an automation's id and creation time never change".into(),
            });
            return Ok(None);
        }
        let text = serde_json::to_string(&trigger).map_err(|error| error.to_string())?;
        Ok(Some((text, trigger)))
    })
    .map_err(TriggerError::Store)?;
    match (saved, failure) {
        (Some(trigger), _) => Ok(trigger),
        (None, Some(error)) => Err(error),
        (None, None) => Err(TriggerError::NotFound(id.into())),
    }
}

/// Forgets a trigger; returns whether it existed. Its runs stay.
pub fn delete(shared: &vak_config::scope::SharedScope, id: &str) -> Result<bool, TriggerError> {
    vak_session::documents::forget(&trigger_path(shared, id)).map_err(TriggerError::Store)
}

/// The triggers of the space the folder `cwd` is bound to.
pub fn for_workspace(triggers: Vec<Trigger>, cwd: &Path) -> Vec<Trigger> {
    let space = vak_config::spaces::key(cwd);
    triggers
        .into_iter()
        .filter(|trigger| trigger.space == space)
        .collect()
}

/// The newest run of `trigger`: "last run" is this query, never a field.
/// Slots folded into a run are part of that run, not a run of their own.
pub fn last_run(
    runs: &vak_session::runs::Runs,
    trigger: &vak_session::ids::TriggerId,
) -> Result<Option<vak_session::runs::RunRecord>, vak_session::SessionError> {
    Ok(runs
        .of_trigger(trigger)?
        .into_iter()
        .find(|run| run.coalesced_into.is_none()))
}

// ---- Claims and the one `due(now)` (plan M4.4) -----------------------------

/// How the scheduler treats slots that passed while nothing ran them.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CatchUp {
    /// `[automation] catch_up_missed`: missed slots fold into the newest,
    /// which runs; off, they are skipped, and so is a newest slot from
    /// before `floor`.
    pub missed: bool,
    /// When this scheduler started.
    pub floor: DateTime<Utc>,
}

/// What `due` decided for one trigger.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Decision {
    /// The claim to write.
    pub claim: vak_session::runs::Claim,
    /// The slot that starts, and its attempt.
    pub start: Option<(vak_session::runs::Slot, u32)>,
    /// The run the claim named, whose holder stopped before it settled.
    pub abandon: Option<vak_session::runs::ActiveRun>,
    /// Earlier slots spent with the one that starts (coalesced into it
    /// when catch-up is on) or without a run (skipped).
    pub missed: Option<vak_session::runs::Missed>,
    /// The slot spent because the previous run still holds the trigger.
    pub busy: Option<vak_session::runs::Slot>,
}

impl Decision {
    fn idle(claim: &vak_session::runs::Claim) -> Self {
        Self {
            claim: claim.clone(),
            start: None,
            abandon: None,
            missed: None,
            busy: None,
        }
    }

    /// Whether it changes nothing, so the claim is left as it is.
    pub fn is_idle(&self, claim: &vak_session::runs::Claim) -> bool {
        self.start.is_none()
            && self.abandon.is_none()
            && self.missed.is_none()
            && self.busy.is_none()
            && self.claim == *claim
    }
}

/// Past this many slots a scan jumps to the last day: only a dense
/// schedule left for weeks gets there, and its range is still exact.
const SCAN_CAP: usize = 20_000;

/// The slots of a trigger that passed: the first, the one before the
/// newest, and the newest.
struct Passed {
    first: DateTime<Utc>,
    previous: Option<DateTime<Utc>>,
    newest: DateTime<Utc>,
}

/// The slots of `trigger` after `after` up to `now`.
fn passed_slots(trigger: &Trigger, after: DateTime<Utc>, now: DateTime<Utc>) -> Option<Passed> {
    let first = trigger.next_slot_after(after).filter(|slot| *slot <= now)?;
    let (mut previous, mut newest) = (None, first);
    let mut cursor = first;
    let mut scanned = 0;
    let mut jumped = false;
    loop {
        scanned += 1;
        if scanned == SCAN_CAP && !jumped {
            jumped = true;
            cursor = cursor.max(now - chrono::Duration::days(1));
        }
        let Some(next) = trigger.next_slot_after(cursor).filter(|slot| *slot <= now) else {
            break;
        };
        if next > newest {
            previous = Some(newest);
            newest = next;
        }
        cursor = next;
    }
    Some(Passed {
        first,
        previous,
        newest,
    })
}

/// What a trigger does at `now`, from its claim and how the claim's run
/// stands: the one decision behind the tick, startup catch-up and run-now
/// (`run_now`, an event id). Pure: the caller writes the claim by CAS and
/// then the records.
///
/// - A run whose holder stopped is abandoned; with `on_crash = retry_once`
///   its slot starts again once, as attempt 2.
/// - The newest slot after the high-water mark starts; earlier ones are
///   coalesced into it, or skipped with catch-up off.
/// - A slot that comes due while the previous run holds the trigger is
///   spent and skipped.
pub fn due(
    trigger: &Trigger,
    claim: &vak_session::runs::Claim,
    holding: Option<vak_session::runs::Holding>,
    now: DateTime<Utc>,
    catch_up: CatchUp,
    run_now: Option<String>,
) -> Decision {
    use vak_session::runs::{Holding, Missed, Slot};
    let mut decision = Decision::idle(claim);
    let live = matches!(holding, Some(Holding::Live));
    let mut retry = None;
    if let (Some(active), Some(stood)) = (&claim.active, holding)
        && stood != Holding::Live
    {
        decision.claim.active = None;
        if stood == Holding::Dead {
            decision.abandon = Some(active.clone());
            if trigger.enabled && trigger.on_crash == OnCrash::RetryOnce && active.attempt < 2 {
                retry = Some((active.slot.clone(), active.attempt + 1));
            }
        }
    }
    if let Some(event) = run_now {
        let slot = Slot::Event { event };
        if live {
            decision.busy = Some(slot);
        } else {
            decision.start = Some((slot, 1));
        }
        return decision;
    }
    if retry.is_some() {
        decision.start = retry;
        return decision;
    }
    if !trigger.enabled {
        return decision;
    }
    let after = claim.high_water.unwrap_or(trigger.created_at);
    let Some(Passed {
        first,
        previous,
        newest,
    }) = passed_slots(trigger, after, now)
    else {
        return decision;
    };
    decision.claim.high_water = Some(newest);
    let earlier = previous.map(|through| Missed {
        from: first,
        through,
    });
    if live {
        decision.busy = Some(Slot::At { at: newest });
        decision.missed = earlier;
    } else if !catch_up.missed && newest < catch_up.floor {
        decision.missed = Some(Missed {
            from: first,
            through: newest,
        });
    } else {
        decision.start = Some((Slot::At { at: newest }, 1));
        decision.missed = earlier;
    }
    decision
}

impl Trigger {
    /// The key of the run serving `slot`, in the space at `cwd`: its
    /// Agent's work, on its owner's behalf, caused by the slot (a schedule)
    /// or by someone running it (an event).
    pub fn trace_for(
        &self,
        slot: &vak_session::runs::Slot,
        cwd: &Path,
    ) -> vak_session::trace::TraceKey {
        use vak_session::trace::{Cause, TraceKey, local};
        let cause = match slot {
            vak_session::runs::Slot::At { at } => Cause::Schedule {
                schedule: self.id.to_string(),
                slot: at.to_rfc3339(),
            },
            vak_session::runs::Slot::Event { event } => Cause::Trigger {
                trigger: self.id,
                request_id: format!("manual:{event}"),
            },
        };
        TraceKey::root(
            local::tenant(),
            local::space(cwd),
            local::agent(&self.agent),
            cause,
        )
        .acting(
            local::agent_principal(&self.agent),
            Some(local::local_owner()),
        )
    }
}

/// A trigger's slot, claimed and opened: the guard settles the run and
/// then releases the claim.
pub struct Started {
    pub run: vak_session::runs::OpenRun,
    pub trace: vak_session::trace::TraceKey,
    pub slot: vak_session::runs::Slot,
    pub attempt: u32,
}

/// What claiming a trigger at `now` came to.
pub enum Claimed {
    /// Nothing was due.
    Idle,
    /// What was due was spent: the previous run still holds the trigger,
    /// or the slots were missed. Each is a run record.
    Spent,
    Started(Box<Started>),
}

/// Claims what is due of `trigger` at `now` and records it: the claim moves
/// by CAS first, then the run opens (minted by `mint` for its slot), then
/// the abandoned, coalesced and skipped records are written. At most one
/// process starts a slot, because only one moves the claim past it.
pub fn claim_due(
    runs: &vak_session::runs::Runs,
    trigger: &Trigger,
    now: DateTime<Utc>,
    catch_up: CatchUp,
    run_now: Option<String>,
    mint: impl Fn(&vak_session::runs::Slot) -> vak_session::trace::TraceKey,
) -> Result<Claimed, vak_session::SessionError> {
    use vak_session::runs::ActiveRun;
    runs.renew()?;
    let moved = runs.move_claim(&trigger.id, now, |claim, holding| {
        let mut decision = due(trigger, claim, holding, now, catch_up, run_now.clone());
        if decision.is_idle(claim) {
            return None;
        }
        let trace = decision.start.as_ref().map(|(slot, attempt)| {
            let trace = mint(slot);
            decision.claim.active = Some(ActiveRun {
                run: trace.run,
                holder: runs.holder(),
                attempt: *attempt,
                slot: slot.clone(),
            });
            trace
        });
        Some((decision.claim.clone(), (decision, trace)))
    })?;
    let Some((_, (decision, trace))) = moved else {
        return Ok(Claimed::Idle);
    };
    if let Some(active) = &decision.abandon {
        runs.abandon(trigger.id, active)?;
    }
    let started = match (decision.start, trace) {
        (Some((slot, attempt)), Some(trace)) => {
            let run = runs.open_claimed(&trace, trigger.id, slot.clone(), attempt)?;
            Some(Started {
                run,
                trace,
                slot,
                attempt,
            })
        }
        _ => None,
    };
    if let Some(missed) = decision.missed {
        match &started {
            Some(started) if catch_up.missed => {
                runs.coalesce(trigger.id, started.run.id(), missed)?;
            }
            _ => {
                let reason = if decision.busy.is_some() {
                    "the previous run was still going"
                } else {
                    "missed while Vakyartha was not running"
                };
                runs.skip_slots(trigger.id, None, Some(missed), reason)?;
            }
        }
    }
    if let Some(slot) = decision.busy {
        runs.skip_slots(
            trigger.id,
            Some(slot),
            None,
            "the previous run was still going",
        )?;
    }
    Ok(match started {
        Some(started) => Claimed::Started(Box::new(started)),
        None => Claimed::Spent,
    })
}

/// When `trigger`'s next slot falls, judged from its claim; an overdue one
/// starts at the next tick.
pub fn next_slot(
    trigger: &Trigger,
    claim: &vak_session::runs::Claim,
    catch_up: CatchUp,
) -> Option<DateTime<Utc>> {
    if !trigger.enabled {
        return None;
    }
    let mut after = claim.high_water.unwrap_or(trigger.created_at);
    if !catch_up.missed {
        after = after.max(catch_up.floor);
    }
    trigger.next_slot_after(after)
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

    // ---- triggers --------------------------------------------------------

    fn prompt_trigger() -> Trigger {
        Trigger {
            id: vak_session::ids::TriggerId::new(),
            name: "morning".into(),
            agent: "vak".into(),
            agent_revision: None,
            space: "spc_test".into(),
            enabled: true,
            kind: TriggerKind::Schedule {
                schedule: Schedule::Cron {
                    expr: "0 9 * * *".into(),
                    timezone: Some("Europe/London".into()),
                },
            },
            action: TriggerAction::Prompt {
                text: "summarise the inbox".into(),
                model_pin: None,
            },
            deliver_to: None,
            on_crash: OnCrash::Skip,
            scope: None,
            created_at: Utc::now(),
            created_by: None,
        }
    }

    #[test]
    fn validation_names_what_is_wrong() {
        let mut t = prompt_trigger();
        assert!(t.validate().is_ok());
        t.action = TriggerAction::Script {
            command: "  ".into(),
        };
        assert!(matches!(t.validate(), Err(TriggerError::Invalid { .. })));
        t = prompt_trigger();
        t.kind = TriggerKind::Schedule {
            schedule: Schedule::Cron {
                expr: "99 * * * *".into(),
                timezone: None,
            },
        };
        assert!(matches!(
            t.validate(),
            Err(TriggerError::BadSchedule { .. })
        ));
        t.kind = TriggerKind::Schedule {
            schedule: Schedule::Cron {
                expr: "0 9 * * *".into(),
                timezone: Some("Mars/Olympus".into()),
            },
        };
        assert!(matches!(
            t.validate(),
            Err(TriggerError::BadSchedule { .. })
        ));
        t.kind = TriggerKind::Schedule {
            schedule: Schedule::Interval {
                every_secs: 5,
                anchor: Utc::now(),
            },
        };
        assert!(matches!(
            t.validate(),
            Err(TriggerError::BadSchedule { .. })
        ));
    }

    #[test]
    fn a_mail_calendar_routine_requires_a_pinned_revision_and_its_own_id() {
        let mut t = prompt_trigger();
        t.scope = Some(vak_mail_calendar::RoutineScope {
            routine_id: t.id.uuid().to_string(),
            account_id: uuid::Uuid::now_v7().to_string(),
            mail_folder_id: None,
            calendar_source_id: None,
            operations: [vak_mail_calendar::RoutineOperation::RecentMail]
                .into_iter()
                .collect(),
            max_items: 5,
            watch_new_mail: true,
            read_commitments: false,
            calendar_event_trigger: None,
        });
        assert!(matches!(
            t.validate(),
            Err(TriggerError::InvalidMailCalendarScope)
        ));
        t.agent_revision = Some(1);
        assert!(t.validate().is_ok());
        t.deliver_to = Some("telegram:1".into());
        assert!(matches!(
            t.validate(),
            Err(TriggerError::InvalidMailCalendarScope)
        ));
        t.deliver_to = None;
        if let Some(scope) = t.scope.as_mut() {
            scope.routine_id = "someone-else".into();
        }
        assert!(matches!(
            t.validate(),
            Err(TriggerError::InvalidMailCalendarScope)
        ));
    }

    #[test]
    fn interval_and_once_slots_are_deterministic() {
        let anchor = Utc.with_ymd_and_hms(2026, 1, 1, 0, 0, 0).unwrap();
        let mut t = prompt_trigger();
        t.kind = TriggerKind::Schedule {
            schedule: Schedule::Interval {
                every_secs: 3600,
                anchor,
            },
        };
        let after = anchor + chrono::Duration::minutes(90);
        assert_eq!(
            t.slots_after(after, 2),
            vec![
                anchor + chrono::Duration::hours(2),
                anchor + chrono::Duration::hours(3)
            ]
        );
        assert_eq!(
            t.next_slot_after(anchor),
            Some(anchor + chrono::Duration::hours(1))
        );
        t.kind = TriggerKind::Schedule {
            schedule: Schedule::Once { at: anchor },
        };
        assert_eq!(
            t.next_slot_after(anchor - chrono::Duration::seconds(1)),
            Some(anchor)
        );
        assert_eq!(t.next_slot_after(anchor), None);
        t.kind = TriggerKind::Manual;
        assert_eq!(t.next_slot_after(anchor), None);
    }

    /// Exit test (plan M4.3): a trigger is a versioned Document, stored and
    /// read back exactly, its versions kept, and nothing about its runs on it.
    #[test]
    fn trigger_round_trips_as_document() {
        vak_config::paths::isolate_home_for_tests();
        let home = tempfile::tempdir().unwrap();
        let shared = vak_config::scope::SharedScope::new(home.path());
        let trigger = prompt_trigger();
        let id = trigger.id.to_string();
        create(&shared, &trigger).unwrap();
        assert!(create(&shared, &trigger).is_err(), "an id is created once");
        assert_eq!(get(&shared, &id).unwrap(), Some(trigger.clone()));

        let renamed = update(&shared, &id, |t| {
            t.name = "evening".into();
            Ok(())
        })
        .unwrap();
        assert_eq!(renamed.name, "evening");
        assert_eq!(list(&shared).unwrap(), vec![renamed.clone()]);
        assert_eq!(
            vak_session::documents::version_count(&shared.triggers().join(&id)),
            2
        );
        let stored = serde_json::to_value(&renamed).unwrap();
        for field in stored.as_object().unwrap().keys() {
            assert!(!field.starts_with("last_"), "{field} is run state");
        }
        assert!(
            update(&shared, &id, |t| {
                t.created_at = Utc::now() + chrono::Duration::days(1);
                Ok(())
            })
            .is_err()
        );
        assert!(delete(&shared, &id).unwrap());
        assert_eq!(get(&shared, &id).unwrap(), None);
        assert!(list(&shared).unwrap().is_empty());
    }
}
