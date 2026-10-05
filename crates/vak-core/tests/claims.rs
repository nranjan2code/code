//! Trigger claims and the one `due(now)` (plan M4.4): a slot starts only
//! through its trigger's claim, so it starts at most once whichever process
//! stops where, two processes never both start it, and `on_crash =
//! retry_once` gives an interrupted slot exactly one more attempt.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use chrono::{DateTime, Duration, Utc};
use std::path::Path;
use std::sync::{Arc, Barrier};
use vak_core::triggers::{
    self, CatchUp, Claimed, OnCrash, Schedule, Trigger, TriggerAction, TriggerKind,
};
use vak_session::ids::{ProcessId, TriggerId};
use vak_session::runs::{ActiveRun, RunOutcome, RunStatus, Runs, Slot};

fn trigger(cwd: &Path, anchor: DateTime<Utc>, on_crash: OnCrash) -> Trigger {
    Trigger {
        id: TriggerId::new(),
        name: "every minute".into(),
        agent: "vak".into(),
        agent_revision: None,
        space: vak_config::spaces::key(cwd),
        enabled: true,
        kind: TriggerKind::Schedule {
            schedule: Schedule::Interval {
                every_secs: 60,
                anchor,
            },
        },
        action: TriggerAction::Script {
            command: "true".into(),
        },
        deliver_to: None,
        on_crash,
        scope: None,
        created_at: anchor,
        created_by: None,
    }
}

const CATCH_UP: CatchUp = CatchUp {
    missed: true,
    floor: DateTime::<Utc>::MIN_UTC,
};

struct Setup {
    dir: tempfile::TempDir,
    tenant_home: std::path::PathBuf,
}

impl Setup {
    fn new() -> Self {
        vak_config::paths::isolate_home_for_tests();
        Self {
            dir: tempfile::tempdir().unwrap(),
            tenant_home: vak_config::paths::local_tenant_home(),
        }
    }

    /// The records as this process sees them.
    fn runs(&self) -> Runs {
        Runs::at(self.dir.path().join("runs"), &self.tenant_home)
    }

    /// The records as another process, which renews its liveness only when
    /// it claims or opens and so is gone a hold later.
    fn other_process(&self) -> Runs {
        self.runs().as_process(ProcessId::new())
    }

    fn claim(&self, runs: &Runs, trigger: &Trigger, now: DateTime<Utc>) -> Claimed {
        let cwd = self.dir.path().to_path_buf();
        triggers::claim_due(runs, trigger, now, CATCH_UP, None, |slot| {
            trigger.trace_for(slot, &cwd)
        })
        .unwrap()
    }
}

/// Every run of `trigger` that opened, as (slot, attempt).
fn starts(runs: &Runs, trigger: &Trigger) -> Vec<(Slot, u32)> {
    runs.of_trigger(&trigger.id)
        .unwrap()
        .into_iter()
        .filter(|run| run.trace.is_some())
        .map(|run| (run.slot.unwrap(), run.attempt))
        .collect()
}

/// Where the first process stops while serving its slot.
#[derive(Debug, Clone, Copy)]
enum Crash {
    BeforeClaim,
    AfterClaim,
    MidRun,
    AfterSettle,
    Clean,
}

#[test]
fn schedule_slot_at_most_once_under_restart() {
    let setup = Setup::new();
    let hold = Duration::seconds(vak_session::runs::LIVENESS_HOLD_SECS + 1);
    for crash in [
        Crash::BeforeClaim,
        Crash::AfterClaim,
        Crash::MidRun,
        Crash::AfterSettle,
        Crash::Clean,
    ] {
        for on_crash in [OnCrash::Skip, OnCrash::RetryOnce] {
            for restarts in 1..=3 {
                let now = Utc::now();
                // Slots at anchor + 60 s and + 120 s have passed.
                let trigger = trigger(setup.dir.path(), now - Duration::seconds(150), on_crash);
                let slot = Slot::At {
                    at: now - Duration::seconds(30),
                };
                let first = setup.other_process();
                match crash {
                    Crash::BeforeClaim => {}
                    Crash::AfterClaim => {
                        first.renew().unwrap();
                        first
                            .move_claim(&trigger.id, now, |claim, holding| {
                                let mut decision =
                                    triggers::due(&trigger, claim, holding, now, CATCH_UP, None);
                                let (slot, attempt) = decision.start.clone()?;
                                decision.claim.active = Some(ActiveRun {
                                    run: trigger.trace_for(&slot, setup.dir.path()).run,
                                    holder: first.holder(),
                                    attempt,
                                    slot,
                                });
                                Some((decision.claim, ()))
                            })
                            .unwrap()
                            .unwrap();
                    }
                    Crash::MidRun | Crash::AfterSettle | Crash::Clean => {
                        let Claimed::Started(started) = setup.claim(&first, &trigger, now) else {
                            panic!("{crash:?}: the first process starts the slot");
                        };
                        assert_eq!(started.slot, slot);
                        let mut started = *started;
                        match crash {
                            Crash::MidRun => std::mem::forget(started.run),
                            Crash::AfterSettle => {
                                first
                                    .settle(started.run.id(), RunOutcome::Completed, None)
                                    .unwrap();
                                std::mem::forget(started.run);
                            }
                            _ => started.run.settle_with(RunOutcome::Completed, None),
                        }
                    }
                }

                // This process restarts the scheduler once the first is gone,
                // and ticks a few times.
                let me = setup.runs();
                let mut held = Vec::new();
                for tick in 0..restarts {
                    if let Claimed::Started(started) =
                        setup.claim(&me, &trigger, now + hold + Duration::seconds(tick))
                    {
                        held.push(*started);
                    }
                }
                for mut started in held {
                    started.run.settle_with(RunOutcome::Completed, None);
                }

                let case = format!("{crash:?}, {on_crash:?}, {restarts} ticks");
                let starts = starts(&me, &trigger);
                for (index, start) in starts.iter().enumerate() {
                    assert!(
                        !starts[index + 1..].contains(start),
                        "{case}: {start:?} started twice"
                    );
                }
                let first_attempts = starts.iter().filter(|(s, a)| *s == slot && *a == 1).count();
                let retries = starts.iter().filter(|(s, a)| *s == slot && *a == 2).count();
                assert!(first_attempts <= 1, "{case}: {starts:?}");
                let interrupted = matches!(crash, Crash::AfterClaim | Crash::MidRun);
                assert_eq!(
                    retries,
                    usize::from(interrupted && on_crash == OnCrash::RetryOnce),
                    "{case}: {starts:?}"
                );
                assert!(
                    starts.iter().all(|(_, attempt)| *attempt <= 2),
                    "{case}: {starts:?}"
                );
                let records = me.of_trigger(&trigger.id).unwrap();
                assert!(
                    records.iter().all(|run| !run.is_open()),
                    "{case}: nothing is left running: {records:#?}"
                );
                if interrupted {
                    assert!(
                        records.iter().any(|run| run.status == RunStatus::Abandoned),
                        "{case}: the interrupted run is recorded abandoned: {records:#?}"
                    );
                }
            }
        }
    }
}

#[test]
fn two_processes_do_not_double_start() {
    let setup = Arc::new(Setup::new());
    for _ in 0..20 {
        let now = Utc::now();
        let trigger = Arc::new(trigger(
            setup.dir.path(),
            now - Duration::seconds(150),
            OnCrash::Skip,
        ));
        let barrier = Arc::new(Barrier::new(2));
        let racers: Vec<_> = [setup.other_process(), setup.runs()]
            .into_iter()
            .map(|runs| {
                let (setup, trigger, barrier) = (setup.clone(), trigger.clone(), barrier.clone());
                std::thread::spawn(move || {
                    barrier.wait();
                    match setup.claim(&runs, &trigger, now) {
                        Claimed::Started(started) => Some(*started),
                        _ => None,
                    }
                })
            })
            .collect();
        let started: Vec<_> = racers
            .into_iter()
            .filter_map(|racer| racer.join().unwrap())
            .collect();
        assert_eq!(started.len(), 1, "exactly one process starts the slot");
        assert_eq!(starts(&setup.runs(), &trigger).len(), 1);
    }
}

#[test]
fn retry_once_retries_once() {
    let setup = Setup::new();
    let hold = Duration::seconds(vak_session::runs::LIVENESS_HOLD_SECS + 1);
    let now = Utc::now();
    let trigger = trigger(
        setup.dir.path(),
        now - Duration::seconds(150),
        OnCrash::RetryOnce,
    );
    let slot = Slot::At {
        at: now - Duration::seconds(30),
    };

    // The first attempt's process stops mid-run.
    let Claimed::Started(first) = setup.claim(&setup.other_process(), &trigger, now) else {
        panic!("the slot starts");
    };
    assert_eq!((first.slot.clone(), first.attempt), (slot.clone(), 1));
    std::mem::forget(first.run);

    // The next process retries it, and stops mid-run too.
    let Claimed::Started(second) = setup.claim(&setup.other_process(), &trigger, now + hold) else {
        panic!("the interrupted slot is retried");
    };
    assert_eq!((second.slot.clone(), second.attempt), (slot.clone(), 2));
    std::mem::forget(second.run);

    // The third does not try that slot again; it goes on to the next.
    let me = setup.runs();
    let later = now + hold + hold;
    if let Claimed::Started(mut third) = setup.claim(&me, &trigger, later) {
        assert_ne!(third.slot, slot, "a slot is retried once, not twice");
        third.run.settle_with(RunOutcome::Completed, None);
    }
    let attempts: Vec<u32> = starts(&me, &trigger)
        .into_iter()
        .filter(|(s, _)| *s == slot)
        .map(|(_, attempt)| attempt)
        .collect();
    assert_eq!(attempts.len(), 2);
    assert!(attempts.contains(&1) && attempts.contains(&2));
    let abandoned = me
        .of_trigger(&trigger.id)
        .unwrap()
        .into_iter()
        .filter(|run| run.slot == Some(slot.clone()) && run.status == RunStatus::Abandoned)
        .count();
    assert_eq!(abandoned, 2, "both interrupted attempts are recorded");
}

#[test]
fn catch_up_off_skips_missed_slots() {
    let setup = Setup::new();
    let now = Utc::now();
    let trigger = trigger(
        setup.dir.path(),
        now - Duration::seconds(150),
        OnCrash::Skip,
    );
    let runs = setup.runs();
    let cwd = setup.dir.path().to_path_buf();
    let off = CatchUp {
        missed: false,
        floor: now,
    };
    let claimed = triggers::claim_due(&runs, &trigger, now, off, None, |slot| {
        trigger.trace_for(slot, &cwd)
    })
    .unwrap();
    assert!(matches!(claimed, Claimed::Spent));
    let records = runs.of_trigger(&trigger.id).unwrap();
    assert_eq!(records.len(), 1, "{records:#?}");
    let missed = records[0].missed.expect("the skipped range");
    assert_eq!(missed.from, now - Duration::seconds(90));
    assert_eq!(missed.through, now - Duration::seconds(30));
    assert!(starts(&runs, &trigger).is_empty());
}
