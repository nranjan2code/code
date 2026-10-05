//! Effect records (plan M4.5, docs/design/73-data-architecture-and-lifecycle.md
//! §8): one record chain of [`EffectEvent`]s for every action Vak takes
//! outside itself, folded into [`EffectRecord`]s. An effect is prepared,
//! with its payload stored as a tenant object, before anything is sent.
//!
//! Only the process that moves the ref `eff/<idempotency key>` under the
//! writer epoch calls the provider for an attempt, so two processes on one
//! store never send the same effect twice. A dispatched effect whose
//! sender stopped before the provider answered becomes unknown and is never
//! sent again by itself: either the provider drops duplicates (a resend
//! under the same key settles it) or the owner reconciles it, as sent, as
//! not sent, or by sending again as a new effect that supersedes it.

use crate::chain::RecordChain;
use crate::ids::{EffectId, PrincipalId, ProcessId, RunId};
use crate::objects::{ObjectRef, Objects, TenantObjects};
use crate::trace::TraceKey;
use crate::types::SessionError;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::HashMap;
use std::path::{Path, PathBuf};

/// How many times an effect proven not sent is tried before it is left
/// failed.
pub const MAX_ATTEMPTS: u32 = 10;

/// What an effect does.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum EffectKind {
    /// A message to a channel or endpoint (`surface:chat[:bot]`).
    Delivery {
        surface: String,
        chat: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        bot: Option<String>,
    },
}

impl EffectKind {
    /// The delivery kind for a target `surface:chat[:bot]`.
    pub fn delivery(target: &str) -> Self {
        let mut parts = target.splitn(3, ':');
        let surface = parts.next().unwrap_or_default().to_string();
        let chat = parts.next().unwrap_or_default().to_string();
        let bot = parts
            .next()
            .filter(|bot| !bot.is_empty())
            .map(str::to_string);
        Self::Delivery { surface, chat, bot }
    }
}

/// The provider's word that it took or landed an effect.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Receipt {
    pub provider: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub provider_id: Option<String>,
    pub at: DateTime<Utc>,
}

/// How the owner settled an effect nobody could prove.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Reconciled {
    Sent,
    NotSent,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "step", rename_all = "snake_case")]
pub enum EffectStep {
    Prepared {
        kind: EffectKind,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        run: Option<RunId>,
        idempotency_key: String,
        payload_digest: String,
        target: String,
        payload: ObjectRef,
        /// Why it waits instead of going out now (a digest).
        #[serde(default, skip_serializing_if = "Option::is_none")]
        hold: Option<String>,
        /// The effect the owner chose to send again by this one.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        supersedes: Option<EffectId>,
    },
    Dispatched {
        attempt: u32,
        holder: ProcessId,
    },
    /// The provider took it.
    Accepted {
        receipt: Receipt,
    },
    /// The provider proved it landed.
    Confirmed {
        receipt: Receipt,
    },
    Failed {
        reason: String,
        proven_not_sent: bool,
    },
    Unknown {
        reason: String,
    },
    Reconciled {
        outcome: Reconciled,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        receipt: Option<Receipt>,
        by: PrincipalId,
    },
    /// The owner chose to send it again, as the effect `by`.
    Superseded {
        by: EffectId,
    },
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct EffectEvent {
    pub effect: EffectId,
    pub at: DateTime<Utc>,
    /// The key of the run the effect serves, on the event that prepares it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub trace: Option<TraceKey>,
    /// Who caused this step: the run's actor when it is prepared, the
    /// owner when they reconcile it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub actor: Option<PrincipalId>,
    #[serde(flatten)]
    pub step: EffectStep,
}

crate::impl_traced!(EffectEvent, "effect_event");

/// Where an effect stands.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EffectStatus {
    /// Prepared and waiting for a digest or the end of its work.
    Held,
    /// Prepared, not yet sent.
    Queued,
    /// A process is sending it.
    Sending,
    /// The provider took or landed it, or the owner said it was sent.
    Sent,
    /// Proven not sent, and it will be tried again.
    Retrying,
    /// Proven not sent and given up, or the owner said it was not sent.
    Failed,
    /// Nobody can say whether it landed; the owner decides.
    Unknown,
    /// The owner sent it again as another effect.
    Superseded,
}

/// An effect as its events leave it.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct EffectRecord {
    pub id: EffectId,
    pub kind: EffectKind,
    pub run: Option<RunId>,
    pub trace: Option<TraceKey>,
    pub target: String,
    pub idempotency_key: String,
    pub payload_digest: String,
    pub payload: ObjectRef,
    pub hold: Option<String>,
    pub supersedes: Option<EffectId>,
    pub superseded_by: Option<EffectId>,
    pub status: EffectStatus,
    pub attempts: u32,
    pub holder: Option<ProcessId>,
    pub receipt: Option<Receipt>,
    /// Why it failed or is unknown.
    pub reason: Option<String>,
    pub reconciled_by: Option<PrincipalId>,
    pub prepared_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

impl EffectRecord {
    fn prepared(
        effect: EffectId,
        at: DateTime<Utc>,
        trace: Option<TraceKey>,
        step: EffectStep,
    ) -> Option<Self> {
        let EffectStep::Prepared {
            kind,
            run,
            idempotency_key,
            payload_digest,
            target,
            payload,
            hold,
            supersedes,
        } = step
        else {
            return None;
        };
        Some(Self {
            id: effect,
            status: if hold.is_some() {
                EffectStatus::Held
            } else {
                EffectStatus::Queued
            },
            kind,
            run,
            trace,
            target,
            idempotency_key,
            payload_digest,
            payload,
            hold,
            supersedes,
            superseded_by: None,
            attempts: 0,
            holder: None,
            receipt: None,
            reason: None,
            reconciled_by: None,
            prepared_at: at,
            updated_at: at,
        })
    }

    fn apply(&mut self, at: DateTime<Utc>, step: EffectStep) {
        self.updated_at = at;
        match step {
            EffectStep::Prepared { .. } => {}
            EffectStep::Dispatched { attempt, holder } => {
                self.attempts = attempt;
                self.holder = Some(holder);
                self.status = EffectStatus::Sending;
                self.reason = None;
            }
            EffectStep::Accepted { receipt } | EffectStep::Confirmed { receipt } => {
                self.receipt = Some(receipt);
                self.status = EffectStatus::Sent;
                self.reason = None;
            }
            EffectStep::Failed {
                reason,
                proven_not_sent,
            } => {
                self.status = if !proven_not_sent {
                    EffectStatus::Unknown
                } else if self.attempts < MAX_ATTEMPTS {
                    EffectStatus::Retrying
                } else {
                    EffectStatus::Failed
                };
                self.reason = Some(reason);
            }
            EffectStep::Unknown { reason } => {
                self.status = EffectStatus::Unknown;
                self.reason = Some(reason);
            }
            EffectStep::Reconciled {
                outcome,
                receipt,
                by,
            } => {
                self.status = match outcome {
                    Reconciled::Sent => EffectStatus::Sent,
                    Reconciled::NotSent => EffectStatus::Failed,
                };
                if receipt.is_some() {
                    self.receipt = receipt;
                }
                self.reconciled_by = Some(by);
            }
            EffectStep::Superseded { by } => {
                self.status = EffectStatus::Superseded;
                self.superseded_by = Some(by);
            }
        }
    }

    /// Whether a sender may take it now: queued, or proven not sent with
    /// attempts left.
    pub fn is_dispatchable(&self) -> bool {
        matches!(self.status, EffectStatus::Queued | EffectStatus::Retrying)
    }

    /// The agent the effect's run works for.
    pub fn agent(&self) -> Option<String> {
        self.trace.as_ref().map(|trace| trace.agent.to_string())
    }
}

/// What a caller asks to prepare.
#[derive(Debug, Clone)]
pub struct Prepare {
    pub kind: EffectKind,
    pub target: String,
    pub trace: Option<TraceKey>,
    pub payload: Vec<u8>,
    pub hold: Option<String>,
    pub supersedes: Option<EffectId>,
}

/// How a sender asks to take an effect.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Dispatch {
    /// A queued effect, or one proven not sent.
    Fresh,
    /// An unknown effect, sent again under the same key, which is safe
    /// only where the provider drops a repeat of a key it has seen.
    Dedupe,
}

/// What the ref `eff/<key>` holds: the attempt last taken, by whom, and
/// whether another may follow it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
struct DispatchClaim {
    effect: EffectId,
    attempt: u32,
    holder: ProcessId,
    retryable: bool,
}

fn dispatch_ref(key: &str) -> String {
    format!("eff/{key}")
}

fn payload_scope(effect: &EffectId) -> String {
    format!("effect:{effect}")
}

fn digest(bytes: &[u8]) -> String {
    let hash = Sha256::digest(bytes);
    hash.iter().map(|byte| format!("{byte:02x}")).collect()
}

/// The effect records of one data home.
#[derive(Debug, Clone)]
pub struct Effects {
    chain: RecordChain,
    tenant_home: PathBuf,
    holder: ProcessId,
}

impl Effects {
    /// The records in `dir` (`SharedScope::effects`), with payloads and
    /// dispatch refs in the tenant at `tenant_home`.
    pub fn at(dir: impl Into<PathBuf>, tenant_home: impl Into<PathBuf>) -> Self {
        Self {
            chain: RecordChain::at(dir),
            tenant_home: tenant_home.into(),
            holder: crate::fence::process(),
        }
    }

    /// These records, acting as the process `holder` rather than this one,
    /// for tests and tools that stand in for another process. Its liveness
    /// is renewed once, when it dispatches, and then lapses.
    pub fn as_process(mut self, holder: ProcessId) -> Self {
        self.holder = holder;
        self
    }

    pub fn path(&self) -> &Path {
        self.chain.path()
    }

    fn append(
        &self,
        effect: EffectId,
        trace: Option<TraceKey>,
        actor: Option<PrincipalId>,
        step: EffectStep,
    ) -> Result<(), SessionError> {
        self.chain.append(&EffectEvent {
            effect,
            at: Utc::now(),
            trace,
            actor,
            step,
        })
    }

    /// Records an effect before anything is sent, its payload stored as a
    /// tenant object. Its idempotency key is the hash of its run, kind,
    /// target, payload digest and ordinal (how many effects of that run
    /// already carry the same four), so it is the same if the same job is
    /// prepared from the same run again.
    pub fn prepare(&self, prepare: Prepare) -> Result<EffectRecord, SessionError> {
        let effect = EffectId::new();
        let run = prepare.trace.as_ref().map(|trace| trace.run);
        let payload_digest = digest(&prepare.payload);
        let ordinal = self
            .list()?
            .iter()
            .filter(|record| {
                record.run == run
                    && record.kind == prepare.kind
                    && record.target == prepare.target
                    && record.payload_digest == payload_digest
            })
            .count();
        let kind_json = serde_json::to_string(&prepare.kind)
            .map_err(|error| SessionError::Objects(error.to_string()))?;
        let run_text = run.map(|run| run.to_string()).unwrap_or_default();
        let key_source = format!(
            "{run_text}\n{kind_json}\n{}\n{payload_digest}\n{ordinal}",
            prepare.target
        );
        // Twenty hex digits: short enough that a key with a chunk index
        // fits a provider's 25-character nonce.
        let idempotency_key = digest(key_source.as_bytes())[..20].to_string();
        let tenant = TenantObjects::for_tenant(&self.tenant_home)?;
        let payload = tenant.put(&prepare.payload, &payload_scope(&effect))?;
        let actor = prepare.trace.as_ref().and_then(|trace| trace.actor);
        let step = EffectStep::Prepared {
            kind: prepare.kind,
            run,
            idempotency_key,
            payload_digest,
            target: prepare.target,
            payload,
            hold: prepare.hold,
            supersedes: prepare.supersedes,
        };
        let at = Utc::now();
        self.chain.append(&EffectEvent {
            effect,
            at,
            trace: prepare.trace.clone(),
            actor,
            step: step.clone(),
        })?;
        EffectRecord::prepared(effect, at, prepare.trace, step)
            .ok_or_else(|| SessionError::Objects("effect was not prepared".into()))
    }

    /// The payload an effect was prepared with.
    pub fn payload(&self, record: &EffectRecord) -> Result<Vec<u8>, SessionError> {
        TenantObjects::for_tenant(&self.tenant_home)?
            .get(&record.payload, &payload_scope(&record.id))
    }

    /// Takes the next attempt of `effect` for this process: moves its
    /// dispatch ref under the writer epoch, then records the attempt.
    /// Returns the effect as taken, or `None` when it is not this
    /// process's to send (another took it, it is settled, or it is not
    /// in a state `how` may take). Only a taken effect may be sent.
    pub fn begin_dispatch(
        &self,
        effect: EffectId,
        how: Dispatch,
    ) -> Result<Option<EffectRecord>, SessionError> {
        let Some(record) = self.get(effect)? else {
            return Ok(None);
        };
        let allowed = match how {
            Dispatch::Fresh => record.is_dispatchable(),
            Dispatch::Dedupe => record.status == EffectStatus::Unknown,
        };
        if !allowed {
            return Ok(None);
        }
        // The holder's liveness is what judges a dispatch it left behind.
        if self.holder == crate::fence::process() {
            crate::runs::keep_alive(&self.tenant_home)?;
        } else {
            crate::fence::renew_liveness_of(
                &self.tenant_home,
                &self.holder,
                Utc::now() + chrono::Duration::seconds(crate::runs::LIVENESS_HOLD_SECS),
            )?;
        }
        let attempt = record.attempts + 1;
        let holder = self.holder;
        let moved = crate::fence::swap_ref(
            &self.tenant_home,
            &dispatch_ref(&record.idempotency_key),
            |target| {
                let current: Option<DispatchClaim> = target
                    .map(|bytes| {
                        serde_json::from_slice(bytes).map_err(|error| {
                            SessionError::Objects(format!("effect claim: {error}"))
                        })
                    })
                    .transpose()?;
                let (last, retryable) = current
                    .as_ref()
                    .map_or((0, true), |claim| (claim.attempt, claim.retryable));
                let free = last == record.attempts
                    && match how {
                        Dispatch::Fresh => retryable,
                        Dispatch::Dedupe => true,
                    };
                if !free {
                    return Ok(None);
                }
                serde_json::to_vec(&DispatchClaim {
                    effect,
                    attempt,
                    holder,
                    retryable: false,
                })
                .map(Some)
                .map_err(|error| SessionError::Objects(error.to_string()))
            },
        )?;
        if !moved {
            return Ok(None);
        }
        let step = EffectStep::Dispatched { attempt, holder };
        if let Err(error) = self.append(effect, None, None, step.clone()) {
            // Nothing was sent: give the attempt back.
            let _ = self.set_retryable(&record.idempotency_key, effect, record.attempts, holder);
            return Err(error);
        }
        let mut taken = record;
        taken.apply(Utc::now(), step);
        Ok(Some(taken))
    }

    fn set_retryable(
        &self,
        key: &str,
        effect: EffectId,
        attempt: u32,
        holder: ProcessId,
    ) -> Result<(), SessionError> {
        let bytes = serde_json::to_vec(&DispatchClaim {
            effect,
            attempt,
            holder,
            retryable: true,
        })
        .map_err(|error| SessionError::Objects(error.to_string()))?;
        crate::fence::swap_ref(&self.tenant_home, &dispatch_ref(key), |_| {
            Ok(Some(bytes.clone()))
        })?;
        Ok(())
    }

    /// The provider took it.
    pub fn accepted(&self, effect: EffectId, receipt: Receipt) -> Result<(), SessionError> {
        self.append(effect, None, None, EffectStep::Accepted { receipt })
    }

    /// The provider proved it landed.
    pub fn confirmed(&self, effect: EffectId, receipt: Receipt) -> Result<(), SessionError> {
        self.append(effect, None, None, EffectStep::Confirmed { receipt })
    }

    /// The attempt failed. Only one `proven_not_sent` frees the effect for
    /// another attempt; any other failure leaves it for the owner.
    pub fn failed(
        &self,
        effect: EffectId,
        reason: impl Into<String>,
        proven_not_sent: bool,
    ) -> Result<(), SessionError> {
        self.append(
            effect,
            None,
            None,
            EffectStep::Failed {
                reason: reason.into(),
                proven_not_sent,
            },
        )?;
        if proven_not_sent && let Some(record) = self.get(effect)? {
            self.set_retryable(
                &record.idempotency_key,
                effect,
                record.attempts,
                self.holder,
            )?;
        }
        Ok(())
    }

    /// Nobody can say whether the attempt landed.
    pub fn unknown(&self, effect: EffectId, reason: impl Into<String>) -> Result<(), SessionError> {
        self.append(
            effect,
            None,
            None,
            EffectStep::Unknown {
                reason: reason.into(),
            },
        )
    }

    /// The owner settles an effect that is unknown or failed as sent or
    /// not sent.
    pub fn reconcile(
        &self,
        effect: EffectId,
        outcome: Reconciled,
        receipt: Option<Receipt>,
        by: PrincipalId,
    ) -> Result<EffectRecord, SessionError> {
        let record = self
            .get(effect)?
            .ok_or_else(|| SessionError::Objects(format!("no effect {effect}")))?;
        if !matches!(
            record.status,
            EffectStatus::Unknown | EffectStatus::Failed | EffectStatus::Retrying
        ) {
            return Err(SessionError::Objects(format!(
                "effect {effect} is {:?}; only an unsettled one is reconciled",
                record.status
            )));
        }
        self.append(
            effect,
            None,
            Some(by),
            EffectStep::Reconciled {
                outcome,
                receipt,
                by,
            },
        )?;
        self.get(effect)?
            .ok_or_else(|| SessionError::Objects(format!("no effect {effect}")))
    }

    /// The owner sends `effect` again: a new effect with the same payload,
    /// kind and target supersedes it. Only an effect that did not land, or
    /// may not have, is sent again.
    pub fn send_again(&self, effect: EffectId) -> Result<EffectRecord, SessionError> {
        let record = self
            .get(effect)?
            .ok_or_else(|| SessionError::Objects(format!("no effect {effect}")))?;
        if !matches!(
            record.status,
            EffectStatus::Unknown | EffectStatus::Failed | EffectStatus::Retrying
        ) {
            return Err(SessionError::Objects(format!(
                "effect {effect} is {:?}; only an unsettled one is sent again",
                record.status
            )));
        }
        let payload = self.payload(&record)?;
        let next = self.prepare(Prepare {
            kind: record.kind.clone(),
            target: record.target.clone(),
            trace: record.trace.clone(),
            payload,
            hold: None,
            supersedes: Some(effect),
        })?;
        self.append(effect, None, None, EffectStep::Superseded { by: next.id })?;
        Ok(next)
    }

    /// Records every effect a stopped process was sending as unknown;
    /// returns them. One this process is sending is never touched by it.
    pub fn recover(&self, now: DateTime<Utc>) -> Result<Vec<EffectId>, SessionError> {
        let mut unknown = Vec::new();
        for record in self.list()? {
            let Some(holder) = record
                .holder
                .filter(|_| record.status == EffectStatus::Sending)
            else {
                continue;
            };
            if holder == self.holder || holder == crate::fence::process() {
                continue;
            }
            if crate::fence::is_alive(&self.tenant_home, &holder, now)? {
                continue;
            }
            self.unknown(
                record.id,
                "the process sending it stopped before the provider answered",
            )?;
            unknown.push(record.id);
        }
        Ok(unknown)
    }

    /// Every event, in order. A row that is not an effect event is an
    /// error, never skipped.
    pub fn events(&self) -> Result<Vec<EffectEvent>, SessionError> {
        self.chain
            .read::<serde_json::Value>()
            .into_iter()
            .map(|row| {
                serde_json::from_value(row).map_err(|error| SessionError::Corrupt {
                    line: 0,
                    message: format!("effect record: {error}"),
                })
            })
            .collect()
    }

    /// Every effect, newest first.
    pub fn list(&self) -> Result<Vec<EffectRecord>, SessionError> {
        let mut effects: HashMap<EffectId, EffectRecord> = HashMap::new();
        for event in self.events()? {
            match effects.get_mut(&event.effect) {
                Some(record) => record.apply(event.at, event.step),
                None => {
                    let record =
                        EffectRecord::prepared(event.effect, event.at, event.trace, event.step)
                            .ok_or_else(|| SessionError::Corrupt {
                                line: 0,
                                message: format!(
                                    "effect {} has a step before it was prepared",
                                    event.effect
                                ),
                            })?;
                    effects.insert(event.effect, record);
                }
            }
        }
        let mut list: Vec<EffectRecord> = effects.into_values().collect();
        list.sort_by_key(|record| std::cmp::Reverse((record.prepared_at, record.id.to_string())));
        Ok(list)
    }

    pub fn get(&self, effect: EffectId) -> Result<Option<EffectRecord>, SessionError> {
        Ok(self.list()?.into_iter().find(|record| record.id == effect))
    }

    /// Every effect the run `run` caused, newest first.
    pub fn of_run(&self, run: RunId) -> Result<Vec<EffectRecord>, SessionError> {
        Ok(self
            .list()?
            .into_iter()
            .filter(|record| record.run == Some(run))
            .collect())
    }

    /// Effects a sender may take now, oldest first.
    pub fn dispatchable(&self) -> Result<Vec<EffectRecord>, SessionError> {
        let mut list: Vec<EffectRecord> = self
            .list()?
            .into_iter()
            .filter(EffectRecord::is_dispatchable)
            .collect();
        list.reverse();
        Ok(list)
    }
}

#[cfg(test)]
#[allow(clippy::expect_used, clippy::unwrap_used)]
mod tests {
    use super::*;
    use crate::ids::{AgentId, SpaceId, TenantId};
    use crate::trace::Cause;

    fn home() -> (tempfile::TempDir, Effects) {
        vak_config::paths::isolate_home_for_tests();
        let dir = tempfile::tempdir().expect("tempdir");
        let effects = Effects::at(dir.path().join("effects"), dir.path().join("tenant"));
        (dir, effects)
    }

    fn trace() -> TraceKey {
        TraceKey::root(
            TenantId::new(),
            SpaceId::new(),
            AgentId::new(),
            Cause::Heartbeat,
        )
    }

    fn delivery(effects: &Effects, trace: &TraceKey, text: &str) -> EffectRecord {
        effects
            .prepare(Prepare {
                kind: EffectKind::delivery("discord:chan:bot"),
                target: "discord:chan:bot".into(),
                trace: Some(trace.clone()),
                payload: text.as_bytes().to_vec(),
                hold: None,
                supersedes: None,
            })
            .expect("prepare")
    }

    #[test]
    fn a_prepared_effect_keeps_its_payload_and_names_its_run() {
        let (_dir, effects) = home();
        let trace = trace();
        let record = delivery(&effects, &trace, "hello");
        assert_eq!(record.status, EffectStatus::Queued);
        assert_eq!(record.run, Some(trace.run));
        assert_eq!(effects.payload(&record).expect("payload"), b"hello");
        assert_eq!(record.idempotency_key.len(), 20);
        let again = delivery(&effects, &trace, "hello");
        assert_ne!(
            again.idempotency_key, record.idempotency_key,
            "a second identical delivery of one run is its own effect"
        );
        assert_eq!(effects.of_run(trace.run).expect("of run").len(), 2);
    }

    #[test]
    fn only_one_process_takes_an_attempt() {
        let (_dir, effects) = home();
        let record = delivery(&effects, &trace(), "once");
        let other = effects.clone().as_process(ProcessId::new());
        let taken = effects
            .begin_dispatch(record.id, Dispatch::Fresh)
            .expect("dispatch")
            .expect("taken");
        assert_eq!(taken.attempts, 1);
        assert!(
            other
                .begin_dispatch(record.id, Dispatch::Fresh)
                .expect("second")
                .is_none()
        );
    }

    #[test]
    fn proven_not_sent_is_retried_and_gives_up() {
        let (_dir, effects) = home();
        let record = delivery(&effects, &trace(), "retry");
        for attempt in 1..=MAX_ATTEMPTS {
            let taken = effects
                .begin_dispatch(record.id, Dispatch::Fresh)
                .expect("dispatch")
                .expect("taken");
            assert_eq!(taken.attempts, attempt);
            effects.failed(record.id, "refused", true).expect("failed");
        }
        let settled = effects.get(record.id).expect("get").expect("record");
        assert_eq!(settled.status, EffectStatus::Failed);
        assert!(effects.dispatchable().expect("dispatchable").is_empty());
    }

    #[test]
    fn effect_unknown_until_reconciled() {
        let (_dir, effects) = home();
        let record = delivery(&effects, &trace(), "maybe");
        effects
            .begin_dispatch(record.id, Dispatch::Fresh)
            .expect("dispatch")
            .expect("taken");
        effects.unknown(record.id, "timed out").expect("unknown");
        for _ in 0..3 {
            assert!(effects.recover(Utc::now()).expect("recover").is_empty());
            assert!(effects.dispatchable().expect("dispatchable").is_empty());
            assert!(
                effects
                    .begin_dispatch(record.id, Dispatch::Fresh)
                    .expect("fresh")
                    .is_none(),
                "an unknown effect is never sent again by itself"
            );
            assert_eq!(
                effects.get(record.id).expect("get").expect("record").status,
                EffectStatus::Unknown
            );
        }
        let owner = PrincipalId::new();
        let settled = effects
            .reconcile(record.id, Reconciled::Sent, None, owner)
            .expect("reconcile");
        assert_eq!(settled.status, EffectStatus::Sent);
        assert_eq!(settled.reconciled_by, Some(owner));
        assert!(
            effects
                .reconcile(record.id, Reconciled::NotSent, None, owner)
                .is_err(),
            "a settled effect is not reconciled again"
        );
    }

    #[test]
    fn effect_not_replayed_after_restart() {
        let (_dir, effects) = home();
        let stopped = ProcessId::new();
        let before = effects.clone().as_process(stopped);
        let record = delivery(&before, &trace(), "in flight");
        before
            .begin_dispatch(record.id, Dispatch::Fresh)
            .expect("dispatch")
            .expect("taken");
        // The sender stopped before the provider answered; a new process
        // starts once its liveness has lapsed.
        let later = Utc::now() + chrono::Duration::seconds(crate::runs::LIVENESS_HOLD_SECS + 1);
        let after = effects.clone().as_process(ProcessId::new());
        assert_eq!(after.recover(later).expect("recover"), vec![record.id]);
        assert!(after.dispatchable().expect("dispatchable").is_empty());
        assert!(
            after
                .begin_dispatch(record.id, Dispatch::Fresh)
                .expect("fresh")
                .is_none()
        );
        let unknown = after.get(record.id).expect("get").expect("record");
        assert_eq!(unknown.status, EffectStatus::Unknown);
        assert_eq!(unknown.attempts, 1);
        // A provider that drops repeats of a key may take it again, once.
        let again = after
            .begin_dispatch(record.id, Dispatch::Dedupe)
            .expect("dedupe")
            .expect("taken");
        assert_eq!(again.attempts, 2);
        assert_eq!(again.idempotency_key, record.idempotency_key);
    }

    #[test]
    fn send_again_supersedes_with_a_new_effect() {
        let (_dir, effects) = home();
        let record = delivery(&effects, &trace(), "again");
        effects
            .begin_dispatch(record.id, Dispatch::Fresh)
            .expect("dispatch")
            .expect("taken");
        effects.unknown(record.id, "timed out").expect("unknown");
        let next = effects.send_again(record.id).expect("send again");
        assert_eq!(next.supersedes, Some(record.id));
        assert_eq!(next.status, EffectStatus::Queued);
        assert_ne!(next.idempotency_key, record.idempotency_key);
        assert_eq!(effects.payload(&next).expect("payload"), b"again");
        let old = effects.get(record.id).expect("get").expect("record");
        assert_eq!(old.status, EffectStatus::Superseded);
        assert_eq!(old.superseded_by, Some(next.id));
    }

    #[test]
    fn a_row_that_is_not_an_effect_event_is_an_error() {
        let (_dir, effects) = home();
        delivery(&effects, &trace(), "x");
        RecordChain::at(effects.path())
            .append(&serde_json::json!({ "partial": true }))
            .expect("append");
        assert!(effects.list().is_err());
    }
}
