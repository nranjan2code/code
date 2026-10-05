//! Run records (plan M4.2, docs/design/73-data-architecture-and-lifecycle.md
//! §8): one record chain of [`RunEvent`]s for every unit of work, whatever
//! caused it, folded into [`RunRecord`]s. Whoever mints a run's id opens
//! it, before any side effect; if that append fails, the run is not
//! admitted. An open run names the process holding it, and a run whose
//! holder stopped renewing its liveness (`crate::fence`) is recorded
//! abandoned by whichever process notices.

use crate::chain::RecordChain;
use crate::ids::{EffectId, PrincipalId, ProcessId, RunId, TriggerId};
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
    },
    /// Missed slots folded into one run.
    Coalesced { into: RunId },
    /// A ledger the run writes or spawns.
    Session { session_id: String },
    Settled {
        #[serde(flatten)]
        outcome: RunOutcome,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        result_id: Option<String>,
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        effects: Vec<EffectId>,
    },
    Abandoned {
        holder: ProcessId,
        noticed_by: ProcessId,
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
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub effects: Vec<EffectId>,
    /// Why it failed or was skipped.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub coalesced_into: Option<RunId>,
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
            effects: Vec::new(),
            reason: None,
            coalesced_into: None,
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
            } => {
                self.status = RunStatus::Skipped;
                self.trigger = trigger.or(self.trigger);
                self.slot = slot.or(self.slot.take());
                self.reason = Some(reason);
                self.settled_at = Some(at);
            }
            RunStep::Coalesced { into } => {
                self.status = RunStatus::Skipped;
                self.coalesced_into = Some(into);
                self.settled_at = Some(at);
            }
            RunStep::Session { session_id } => {
                if !self.sessions.contains(&session_id) {
                    self.sessions.push(session_id);
                }
            }
            RunStep::Settled {
                outcome,
                result_id,
                effects,
            } => {
                self.status = match &outcome {
                    RunOutcome::Completed => RunStatus::Completed,
                    RunOutcome::Failed { reason } => {
                        self.reason = Some(reason.clone());
                        RunStatus::Failed
                    }
                    RunOutcome::Cancelled => RunStatus::Cancelled,
                };
                self.result_id = result_id;
                self.effects = effects;
                self.settled_at = Some(at);
            }
            RunStep::Abandoned { holder, noticed_by } => {
                self.status = RunStatus::Abandoned;
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
}

impl Runs {
    /// The records in `dir` (`SharedScope::runs`), with holders judged by
    /// liveness in the tenant at `tenant_home`.
    pub fn at(dir: impl Into<PathBuf>, tenant_home: impl Into<PathBuf>) -> Self {
        Self {
            chain: RecordChain::at(dir),
            tenant_home: tenant_home.into(),
        }
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
        keep_alive(&self.tenant_home)?;
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
                holder: crate::fence::process(),
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
            handed_off: false,
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
        self.append(
            run,
            RunStep::Settled {
                outcome,
                result_id,
                effects: Vec::new(),
            },
        )
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
        let me = crate::fence::process();
        let mut abandoned = Vec::new();
        for record in self.list()? {
            if !record.is_open() {
                continue;
            }
            let Some(holder) = record.holder else {
                continue;
            };
            if holder == me || crate::fence::is_alive(&self.tenant_home, &holder, now)? {
                continue;
            }
            self.append(
                record.id,
                RunStep::Abandoned {
                    holder,
                    noticed_by: me,
                },
            )?;
            abandoned.push(record.id);
        }
        Ok(abandoned)
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
    handed_off: bool,
}

impl OpenRun {
    pub fn id(&self) -> RunId {
        self.run
    }

    pub fn end_with(&mut self, end: RunEnd) {
        self.end = Some(end);
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
            RunEnd::Settled(outcome) => self.runs.settle(self.run, outcome, None),
            RunEnd::Skipped(reason) => self.runs.append(
                self.run,
                RunStep::Skipped {
                    trigger: None,
                    slot: None,
                    reason,
                },
            ),
        };
        if let Err(error) = written {
            eprintln!("[runs] {} did not settle: {error}", self.run);
        }
    }
}

/// Renews this process's liveness now, and keeps renewing it from a
/// background thread for as long as the process lives and is not fenced,
/// so a run it holds is never judged abandoned while it works.
fn keep_alive(tenant_home: &Path) -> Result<(), SessionError> {
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
