//! Run records (plan M4.2, docs/design/73-data-architecture-and-lifecycle.md
//! §8): one record chain of [`RunEvent`]s for every unit of work, whatever
//! caused it, folded into [`RunRecord`]s. Whoever mints a run's id opens
//! it, before any side effect; if that append fails, the run is not
//! admitted. An open run names the process holding it, and a run whose
//! holder stopped renewing its liveness (`crate::fence`) is recorded
//! abandoned by whichever process notices.
//!
//! A trigger's slots are started through its claim (plan M4.4), a ref
//! `trg/<id>/claim` moved by CAS under the writer epoch before the run
//! opens: whoever moves it starts the slot, and nobody else can.

use crate::chain::RecordChain;
use crate::ids::{PrincipalId, ProcessId, RunId, TriggerId};
use crate::trace::TraceKey;
use crate::types::SessionError;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::{Path, PathBuf};

/// Which slot of a trigger a run serves.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Slot {
    At { at: DateTime<Utc> },
    Event { event: String },
}

/// A range of a trigger's slots, `from` through `through`, decided
/// together without a run of their own.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct Missed {
    pub from: DateTime<Utc>,
    pub through: DateTime<Utc>,
}

/// What a run does when it is not a conversation turn: the name a list of
/// such runs is kept by.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum RunWork {
    /// One execution of the named flow, or one attempt of a plan.
    Flow { name: String },
    /// A plan: planned and run as flow attempts, each its own run.
    Plan,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "outcome", rename_all = "snake_case")]
pub enum RunOutcome {
    Completed,
    Failed { reason: String },
    Cancelled,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "step", rename_all = "snake_case")]
pub enum RunStep {
    /// The run's key, and so its cause, is the event's `trace`.
    Opened {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        work: Option<RunWork>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        trigger: Option<TriggerId>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        slot: Option<Slot>,
        attempt: u32,
        holder: ProcessId,
    },
    /// A slot or event decided without starting.
    Skipped {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        trigger: Option<TriggerId>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        slot: Option<Slot>,
        reason: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        missed: Option<Missed>,
    },
    /// Missed slots folded into the run `into`, which serves the newest.
    Coalesced {
        into: RunId,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        trigger: Option<TriggerId>,
        missed: Missed,
    },
    /// A ledger the run writes or spawns.
    Session { session_id: String },
    Settled {
        #[serde(flatten)]
        outcome: RunOutcome,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        result_id: Option<String>,
    },
    Abandoned {
        holder: ProcessId,
        noticed_by: ProcessId,
        /// The trigger and slot whose claim named it, so a run abandoned
        /// before it opened is still found as that trigger's.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        trigger: Option<TriggerId>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        slot: Option<Slot>,
    },
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RunEvent {
    pub run: RunId,
    pub at: DateTime<Utc>,
    /// The run's key, on the event that opens it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub trace: Option<TraceKey>,
    /// Who caused this step: the run's actor when it opens.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub actor: Option<PrincipalId>,
    #[serde(flatten)]
    pub step: RunStep,
}

crate::impl_traced!(RunEvent, "run_event");

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RunStatus {
    Running,
    Completed,
    Failed,
    Cancelled,
    Abandoned,
    Skipped,
}

/// The fold of one run's events.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RunRecord {
    pub id: RunId,
    pub status: RunStatus,
    pub opened_at: DateTime<Utc>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub settled_at: Option<DateTime<Utc>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub trace: Option<TraceKey>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub work: Option<RunWork>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub trigger: Option<TriggerId>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub slot: Option<Slot>,
    pub attempt: u32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub holder: Option<ProcessId>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub sessions: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub result_id: Option<String>,
    /// Why it failed or was skipped.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub coalesced_into: Option<RunId>,
    /// The slots a skipped or coalesced record stands for.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub missed: Option<Missed>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub noticed_by: Option<ProcessId>,
}

impl RunRecord {
    fn new(id: RunId, at: DateTime<Utc>) -> Self {
        Self {
            id,
            status: RunStatus::Running,
            opened_at: at,
            settled_at: None,
            trace: None,
            work: None,
            trigger: None,
            slot: None,
            attempt: 1,
            holder: None,
            sessions: Vec::new(),
            result_id: None,
            reason: None,
            coalesced_into: None,
            missed: None,
            noticed_by: None,
        }
    }

    fn apply(&mut self, at: DateTime<Utc>, trace: Option<TraceKey>, step: RunStep) {
        match step {
            RunStep::Opened {
                work,
                trigger,
                slot,
                attempt,
                holder,
            } => {
                self.opened_at = at;
                self.trace = trace;
                self.work = work;
                self.trigger = trigger;
                self.slot = slot;
                self.attempt = attempt;
                self.holder = Some(holder);
            }
            RunStep::Skipped {
                trigger,
                slot,
                reason,
                missed,
            } => {
                self.status = RunStatus::Skipped;
                self.trigger = trigger.or(self.trigger);
                self.slot = slot.or(self.slot.take());
                self.reason = Some(reason);
                self.missed = missed.or(self.missed);
                self.settled_at = Some(at);
            }
            RunStep::Coalesced {
                into,
                trigger,
                missed,
            } => {
                self.status = RunStatus::Skipped;
                self.trigger = trigger.or(self.trigger);
                self.coalesced_into = Some(into);
                self.missed = Some(missed);
                self.settled_at = Some(at);
            }
            RunStep::Session { session_id } => {
                if !self.sessions.contains(&session_id) {
                    self.sessions.push(session_id);
                }
            }
            RunStep::Settled { outcome, result_id } => {
                self.status = match &outcome {
                    RunOutcome::Completed => RunStatus::Completed,
                    RunOutcome::Failed { reason } => {
                        self.reason = Some(reason.clone());
                        RunStatus::Failed
                    }
                    RunOutcome::Cancelled => RunStatus::Cancelled,
                };
                self.result_id = result_id;
                self.settled_at = Some(at);
            }
            RunStep::Abandoned {
                holder,
                noticed_by,
                trigger,
                slot,
            } => {
                self.status = RunStatus::Abandoned;
                self.trigger = self.trigger.or(trigger);
                self.slot = self.slot.take().or(slot);
                self.holder = Some(holder);
                self.noticed_by = Some(noticed_by);
                self.settled_at = Some(at);
            }
        }
    }

    /// Whether nothing has settled it yet.
    pub fn is_open(&self) -> bool {
        self.status == RunStatus::Running
    }

    /// The agent the run worked as, from its key.
    pub fn agent(&self) -> Option<String> {
        self.trace.as_ref().map(|trace| trace.agent.to_string())
    }
}

/// How long one liveness renewal holds, and how often a process holding
/// runs renews it.
pub const LIVENESS_HOLD_SECS: i64 = 60;
const LIVENESS_RENEW_SECS: u64 = 20;

/// The run records of one data home.
#[derive(Debug, Clone)]
pub struct Runs {
    chain: RecordChain,
    tenant_home: PathBuf,
    holder: ProcessId,
}

impl Runs {
    /// The records in `dir` (`SharedScope::runs`), with holders judged by
    /// liveness in the tenant at `tenant_home`.
    pub fn at(dir: impl Into<PathBuf>, tenant_home: impl Into<PathBuf>) -> Self {
        Self {
            chain: RecordChain::at(dir),
            tenant_home: tenant_home.into(),
            holder: crate::fence::process(),
        }
    }

    /// These records, acting as the process `holder` rather than this one:
    /// what it opens names it, and its liveness is renewed once per open
    /// and never kept fresh, so its runs lapse as a stopped process's do.
    /// For tests and tools that stand in for another process.
    pub fn as_process(mut self, holder: ProcessId) -> Self {
        self.holder = holder;
        self
    }

    /// Renews the holder's liveness: kept fresh from now on for this
    /// process, once for a stand-in. A claimant renews before it moves a
    /// claim, so nobody judges its fresh claim's holder gone.
    pub fn renew(&self) -> Result<(), SessionError> {
        if self.holder == crate::fence::process() {
            keep_alive(&self.tenant_home)
        } else {
            crate::fence::renew_liveness_of(
                &self.tenant_home,
                &self.holder,
                Utc::now() + chrono::Duration::seconds(LIVENESS_HOLD_SECS),
            )
        }
    }

    /// The process this handle opens runs as.
    pub fn holder(&self) -> ProcessId {
        self.holder
    }

    /// Whether `holder` is alive at `now`: this handle's own process is
    /// while it runs.
    pub fn is_alive(&self, holder: &ProcessId, now: DateTime<Utc>) -> Result<bool, SessionError> {
        if *holder == crate::fence::process() {
            return Ok(true);
        }
        crate::fence::is_alive(&self.tenant_home, holder, now)
    }

    pub fn path(&self) -> &Path {
        self.chain.path()
    }

    fn append(&self, run: RunId, step: RunStep) -> Result<(), SessionError> {
        self.chain.append(&RunEvent {
            run,
            at: Utc::now(),
            trace: None,
            actor: None,
            step,
        })
    }

    /// Opens the run `trace.run`, held by this process, before any work it
    /// does. A failure here means the run must not start.
    pub fn open(
        &self,
        trace: &TraceKey,
        trigger: Option<TriggerId>,
        slot: Option<Slot>,
        attempt: u32,
    ) -> Result<RunId, SessionError> {
        self.open_in(trace, trigger, slot, attempt, None, None)
    }

    /// Opens the run as [`Runs::open`] does, naming the ledger it writes in
    /// the same synced append.
    pub fn open_in(
        &self,
        trace: &TraceKey,
        trigger: Option<TriggerId>,
        slot: Option<Slot>,
        attempt: u32,
        session_id: Option<&str>,
        work: Option<RunWork>,
    ) -> Result<RunId, SessionError> {
        self.renew()?;
        let now = Utc::now();
        let mut events = vec![RunEvent {
            run: trace.run,
            at: now,
            trace: Some(trace.clone()),
            actor: trace.actor,
            step: RunStep::Opened {
                work,
                trigger,
                slot,
                attempt,
                holder: self.holder,
            },
        }];
        if let Some(session_id) = session_id {
            events.push(RunEvent {
                run: trace.run,
                at: now,
                trace: None,
                actor: None,
                step: RunStep::Session {
                    session_id: session_id.to_string(),
                },
            });
        }
        self.chain.append_all(&events)?;
        Ok(trace.run)
    }

    /// Opens `trace.run` as [`Runs::open`] does, as a guard that ends it on
    /// every path.
    pub fn begin(
        &self,
        trace: &TraceKey,
        trigger: Option<TriggerId>,
        slot: Option<Slot>,
    ) -> Result<OpenRun, SessionError> {
        let run = self.open(trace, trigger, slot, 1)?;
        Ok(OpenRun {
            runs: self.clone(),
            run,
            end: None,
            result_id: None,
            handed_off: false,
            claim: None,
        })
    }

    /// Records that `run` writes or spawned the ledger `session_id`.
    pub fn session(&self, run: RunId, session_id: &str) -> Result<(), SessionError> {
        self.append(
            run,
            RunStep::Session {
                session_id: session_id.to_string(),
            },
        )
    }

    pub fn settle(
        &self,
        run: RunId,
        outcome: RunOutcome,
        result_id: Option<String>,
    ) -> Result<(), SessionError> {
        self.append(run, RunStep::Settled { outcome, result_id })
    }

    /// Records a slot or event decided without starting a run.
    pub fn skip(
        &self,
        trigger: Option<TriggerId>,
        slot: Option<Slot>,
        reason: &str,
    ) -> Result<RunId, SessionError> {
        let run = RunId::new();
        self.append(
            run,
            RunStep::Skipped {
                trigger,
                slot,
                reason: reason.to_string(),
                missed: None,
            },
        )?;
        Ok(run)
    }

    /// Ends the open run `run` as decided without doing its work.
    pub fn skip_open(&self, run: RunId, reason: &str) -> Result<(), SessionError> {
        self.append(
            run,
            RunStep::Skipped {
                trigger: None,
                slot: None,
                reason: reason.to_string(),
                missed: None,
            },
        )
    }

    /// Every event, in order. A row that is not a run event is an error,
    /// never skipped: a status folded with one missing would be a
    /// confident wrong answer.
    pub fn events(&self) -> Result<Vec<RunEvent>, SessionError> {
        self.chain
            .read::<serde_json::Value>()
            .into_iter()
            .map(|row| {
                serde_json::from_value(row).map_err(|error| SessionError::Corrupt {
                    line: 0,
                    message: format!("run record: {error}"),
                })
            })
            .collect()
    }

    /// Every run, newest first.
    pub fn list(&self) -> Result<Vec<RunRecord>, SessionError> {
        let mut order = Vec::new();
        let mut runs: HashMap<RunId, RunRecord> = HashMap::new();
        for event in self.events()? {
            let record = runs.entry(event.run).or_insert_with(|| {
                order.push(event.run);
                RunRecord::new(event.run, event.at)
            });
            record.apply(event.at, event.trace, event.step);
        }
        let mut list: Vec<RunRecord> = order
            .into_iter()
            .filter_map(|id| runs.remove(&id))
            .collect();
        list.sort_by_key(|record| std::cmp::Reverse(record.opened_at));
        Ok(list)
    }

    /// Every run of the trigger `trigger`, newest first.
    pub fn of_trigger(&self, trigger: &TriggerId) -> Result<Vec<RunRecord>, SessionError> {
        Ok(self
            .list()?
            .into_iter()
            .filter(|run| run.trigger.as_ref() == Some(trigger))
            .collect())
    }

    /// Every run of the flow `name`, newest first.
    pub fn of_flow(&self, name: &str) -> Result<Vec<RunRecord>, SessionError> {
        Ok(self
            .list()?
            .into_iter()
            .filter(|run| matches!(&run.work, Some(RunWork::Flow { name: n }) if n == name))
            .collect())
    }

    pub fn get(&self, run: RunId) -> Result<Option<RunRecord>, SessionError> {
        Ok(self.list()?.into_iter().find(|record| record.id == run))
    }

    /// Records every open run whose holder is no longer alive at `now` as
    /// abandoned; returns them. A run this process holds is never swept by
    /// it.
    pub fn sweep_abandoned(&self, now: DateTime<Utc>) -> Result<Vec<RunId>, SessionError> {
        let me = self.holder;
        let mut abandoned = Vec::new();
        for record in self.list()? {
            if !record.is_open() {
                continue;
            }
            let Some(holder) = record.holder else {
                continue;
            };
            if holder == me || self.is_alive(&holder, now)? {
                continue;
            }
            self.append(
                record.id,
                RunStep::Abandoned {
                    holder,
                    noticed_by: me,
                    trigger: None,
                    slot: None,
                },
            )?;
            abandoned.push(record.id);
        }
        Ok(abandoned)
    }
}

/// What a trigger's claim ref holds: the newest schedule slot spent, and
/// the run that holds the trigger now.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Claim {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub high_water: Option<DateTime<Utc>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub active: Option<ActiveRun>,
}

/// The run a claim names as holding its trigger.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ActiveRun {
    pub run: RunId,
    pub holder: ProcessId,
    pub attempt: u32,
    pub slot: Slot,
}

/// How the run a claim names stands.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Holding {
    /// Its holder is alive and it has not settled (or is still opening).
    Live,
    /// Its holder stopped before it settled.
    Dead,
    /// It settled; the claim was not released.
    Settled,
}

fn claim_ref(trigger: &TriggerId) -> String {
    format!("trg/{trigger}/claim")
}

fn decode_claim(target: Option<&[u8]>) -> Result<Claim, SessionError> {
    target.map_or(Ok(Claim::default()), |bytes| {
        serde_json::from_slice(bytes)
            .map_err(|error| SessionError::Objects(format!("trigger claim: {error}")))
    })
}

impl Runs {
    /// The claim of `trigger` as it stands.
    pub fn claim(&self, trigger: &TriggerId) -> Result<Claim, SessionError> {
        let tenant = crate::objects::TenantObjects::for_tenant(&self.tenant_home)?;
        let value = tenant
            .store()
            .get_ref(&claim_ref(trigger))
            .map_err(crate::objects::objects_error)?;
        decode_claim(value.as_ref().map(|value| value.target.as_slice()))
    }

    /// How `active` stands at `now`.
    pub fn holding(&self, active: &ActiveRun, now: DateTime<Utc>) -> Result<Holding, SessionError> {
        if self
            .get(active.run)?
            .is_some_and(|record| !record.is_open())
        {
            return Ok(Holding::Settled);
        }
        Ok(if self.is_alive(&active.holder, now)? {
            Holding::Live
        } else {
            Holding::Dead
        })
    }

    /// Moves the claim of `trigger` to what `decide` makes of it, under the
    /// writer epoch. `decide` sees the claim and how its active run stands
    /// at `now`, and returns the claim to write with what that decided, or
    /// `None` to leave it. A lost race re-reads and decides again, so only
    /// a decision made on the claim as written is returned, with the claim
    /// it replaced.
    pub fn move_claim<T>(
        &self,
        trigger: &TriggerId,
        now: DateTime<Utc>,
        mut decide: impl FnMut(&Claim, Option<Holding>) -> Option<(Claim, T)>,
    ) -> Result<Option<(Claim, T)>, SessionError> {
        let mut decided = None;
        crate::fence::swap_ref(&self.tenant_home, &claim_ref(trigger), |target| {
            let current = decode_claim(target)?;
            let holding = match &current.active {
                Some(active) => Some(self.holding(active, now)?),
                None => None,
            };
            let Some((next, value)) = decide(&current, holding) else {
                decided = None;
                return Ok(None);
            };
            let bytes = serde_json::to_vec(&next)
                .map_err(|error| SessionError::Objects(error.to_string()))?;
            decided = Some((current, value));
            Ok(Some(bytes))
        })?;
        Ok(decided)
    }

    /// Opens the run `trace.run` for the slot of `trigger` whose claim now
    /// names it, as a guard that settles it and then releases the claim.
    /// If the open fails, the claim is released at once.
    pub fn open_claimed(
        &self,
        trace: &TraceKey,
        trigger: TriggerId,
        slot: Slot,
        attempt: u32,
    ) -> Result<OpenRun, SessionError> {
        match self.open(trace, Some(trigger), Some(slot), attempt) {
            Ok(run) => Ok(OpenRun {
                runs: self.clone(),
                run,
                end: None,
                result_id: None,
                handed_off: false,
                claim: Some(trigger),
            }),
            Err(error) => {
                let _ = self.release_claim(&trigger, trace.run);
                Err(error)
            }
        }
    }

    /// Clears the claim of `trigger` if it still names `run`.
    pub fn release_claim(&self, trigger: &TriggerId, run: RunId) -> Result<(), SessionError> {
        crate::fence::swap_ref(&self.tenant_home, &claim_ref(trigger), |target| {
            let mut claim = decode_claim(target)?;
            if claim.active.as_ref().is_none_or(|active| active.run != run) {
                return Ok(None);
            }
            claim.active = None;
            serde_json::to_vec(&claim)
                .map(Some)
                .map_err(|error| SessionError::Objects(error.to_string()))
        })?;
        Ok(())
    }

    /// Records the run `active`, which held the claim of `trigger`, as
    /// abandoned by its holder, unless it already settled.
    pub fn abandon(&self, trigger: TriggerId, active: &ActiveRun) -> Result<(), SessionError> {
        if self
            .get(active.run)?
            .is_some_and(|record| !record.is_open())
        {
            return Ok(());
        }
        self.append(
            active.run,
            RunStep::Abandoned {
                holder: active.holder,
                noticed_by: self.holder,
                trigger: Some(trigger),
                slot: Some(active.slot.clone()),
            },
        )
    }

    /// Records the slots `missed` of `trigger` as folded into the run `into`.
    pub fn coalesce(
        &self,
        trigger: TriggerId,
        into: RunId,
        missed: Missed,
    ) -> Result<RunId, SessionError> {
        let run = RunId::new();
        self.append(
            run,
            RunStep::Coalesced {
                into,
                trigger: Some(trigger),
                missed,
            },
        )?;
        Ok(run)
    }

    /// Records slots of `trigger` spent without a run: `slot`, or the range
    /// `missed`, and why.
    pub fn skip_slots(
        &self,
        trigger: TriggerId,
        slot: Option<Slot>,
        missed: Option<Missed>,
        reason: &str,
    ) -> Result<RunId, SessionError> {
        let run = RunId::new();
        self.append(
            run,
            RunStep::Skipped {
                trigger: Some(trigger),
                slot,
                reason: reason.to_string(),
                missed,
            },
        )?;
        Ok(run)
    }
}

/// How an [`OpenRun`] ends when it is dropped.
#[derive(Debug, Clone, PartialEq)]
pub enum RunEnd {
    Settled(RunOutcome),
    /// Decided without doing the work.
    Skipped(String),
}

/// A run opened by a surface that may end it along many paths. Dropped, it
/// ends the run as told by [`OpenRun::end_with`], or as skipped because
/// the request ended before any turn started; [`OpenRun::hand_off`] passes
/// settling to whatever runs the work.
pub struct OpenRun {
    runs: Runs,
    run: RunId,
    end: Option<RunEnd>,
    result_id: Option<String>,
    handed_off: bool,
    /// The trigger whose claim this run holds, released once it ends.
    claim: Option<TriggerId>,
}

impl OpenRun {
    pub fn id(&self) -> RunId {
        self.run
    }

    pub fn end_with(&mut self, end: RunEnd) {
        self.end = Some(end);
    }

    /// Ends it settled with `outcome`, naming the answer it produced.
    pub fn settle_with(&mut self, outcome: RunOutcome, result_id: Option<String>) {
        self.end = Some(RunEnd::Settled(outcome));
        self.result_id = result_id;
    }

    /// The work this run admitted is starting elsewhere, which settles it.
    pub fn hand_off(mut self) -> RunId {
        self.handed_off = true;
        self.run
    }
}

impl Drop for OpenRun {
    fn drop(&mut self) {
        if self.handed_off {
            return;
        }
        let end = self
            .end
            .take()
            .unwrap_or_else(|| RunEnd::Skipped("the request ended before a turn started".into()));
        let written = match end {
            RunEnd::Settled(outcome) => self.runs.settle(self.run, outcome, self.result_id.take()),
            RunEnd::Skipped(reason) => self.runs.append(
                self.run,
                RunStep::Skipped {
                    trigger: None,
                    slot: None,
                    reason,
                    missed: None,
                },
            ),
        };
        if let Err(error) = written {
            eprintln!("[runs] {} did not settle: {error}", self.run);
        }
        // Settled first, then released: a claim never frees a slot whose
        // run still reads as running.
        if let Some(trigger) = self.claim.take()
            && let Err(error) = self.runs.release_claim(&trigger, self.run)
        {
            eprintln!(
                "[runs] {trigger} kept its claim after {}: {error}",
                self.run
            );
        }
    }
}

/// Renews this process's liveness now, and keeps renewing it from a
/// background thread for as long as the process lives and is not fenced,
/// so a run it holds is never judged abandoned while it works.
pub(crate) fn keep_alive(tenant_home: &Path) -> Result<(), SessionError> {
    static STARTED: std::sync::OnceLock<()> = std::sync::OnceLock::new();
    crate::fence::renew_liveness(
        tenant_home,
        Utc::now() + chrono::Duration::seconds(LIVENESS_HOLD_SECS),
    )?;
    let tenant_home = tenant_home.to_path_buf();
    STARTED.get_or_init(|| {
        let _ = std::thread::Builder::new()
            .name("vak-liveness".into())
            .spawn(move || {
                loop {
                    std::thread::sleep(std::time::Duration::from_secs(LIVENESS_RENEW_SECS));
                    let renewed = crate::fence::renew_liveness(
                        &tenant_home,
                        Utc::now() + chrono::Duration::seconds(LIVENESS_HOLD_SECS),
                    );
                    if matches!(renewed, Err(SessionError::Fenced { .. })) {
                        return;
                    }
                }
            });
    });
    Ok(())
}
