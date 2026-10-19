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
use std::path::{Path, PathBuf};

mod projection;

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
    /// An email sent from a connected account (plan M4.6). The target
    /// names the reviewed candidate; the effect carries no content.
    MailSend {
        account: String,
    },
    /// A calendar event created, changed, cancelled or answered from a
    /// connected account.
    CalendarCreate {
        account: String,
    },
    CalendarUpdate {
        account: String,
    },
    CalendarCancel {
        account: String,
    },
    CalendarRsvp {
        account: String,
    },
}

impl EffectKind {
    /// Whether an attempt proven not sent is tried again by itself. A
    /// delivery is; a mail or calendar change was approved once, for one
    /// attempt, and is never sent again without a new review.
    pub fn retries(&self) -> bool {
        matches!(self, Self::Delivery { .. })
    }

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
    pub supersedes: Option<EffectId>,
    pub superseded_by: Option<EffectId>,
    pub status: EffectStatus,
    pub attempts: u32,
    pub holder: Option<ProcessId>,
    pub receipt: Option<Receipt>,
    /// Whether the provider proved it landed, rather than only took it.
    pub confirmed: bool,
    /// Why it failed or is unknown.
    pub reason: Option<String>,
    pub reconciled_by: Option<PrincipalId>,
    pub prepared_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

/// The `delivery` span of one attempt to send `record`, under its run's
/// `trace_id` (plan M5b).
pub fn delivery_span(record: &EffectRecord) -> tracing::Span {
    let span = tracing::info_span!(
        "delivery",
        trace_id = tracing::field::Empty,
        effect = %record.id,
        kind = record.kind.name(),
        attempt = record.attempts,
    );
    if let Some(run) = record.run {
        span.record("trace_id", tracing::field::display(run));
    }
    span
}

impl EffectKind {
    /// The kind's name, for telemetry: never its target or account.
    pub fn name(&self) -> &'static str {
        match self {
            Self::Delivery { .. } => "delivery",
            Self::MailSend { .. } => "mail_send",
            Self::CalendarCreate { .. } => "calendar_create",
            Self::CalendarUpdate { .. } => "calendar_update",
            Self::CalendarCancel { .. } => "calendar_cancel",
            Self::CalendarRsvp { .. } => "calendar_rsvp",
        }
    }
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
            supersedes,
        } = step
        else {
            return None;
        };
        Some(Self {
            id: effect,
            status: EffectStatus::Queued,
            kind,
            run,
            trace,
            target,
            idempotency_key,
            payload_digest,
            payload,
            supersedes,
            superseded_by: None,
            attempts: 0,
            holder: None,
            receipt: None,
            confirmed: false,
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
            EffectStep::Accepted { receipt } => {
                self.receipt = Some(receipt);
                self.status = EffectStatus::Sent;
                self.reason = None;
            }
            EffectStep::Confirmed { receipt } => {
                self.receipt = Some(receipt);
                self.confirmed = true;
                self.status = EffectStatus::Sent;
                self.reason = None;
            }
            EffectStep::Failed {
                reason,
                proven_not_sent,
            } => {
                self.status = if !proven_not_sent {
                    EffectStatus::Unknown
                } else if self.kind.retries() && self.attempts < MAX_ATTEMPTS {
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
    pub supersedes: Option<EffectId>,
}

impl Prepare {
    /// An effect that supersedes nothing.
    pub fn new(
        kind: EffectKind,
        target: impl Into<String>,
        trace: Option<TraceKey>,
        payload: Vec<u8>,
    ) -> Self {
        Self {
            kind,
            target: target.into(),
            trace,
            payload,
            supersedes: None,
        }
    }
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

/// An idempotency key: twenty hex digits of the hash of the kind, the
/// target, the payload digest and `basis`, short enough that a key with a
/// chunk index fits a provider's 25-character nonce.
fn key_of(prepare: &Prepare, payload_digest: &str, basis: &str) -> Result<String, SessionError> {
    let kind_json = serde_json::to_string(&prepare.kind)
        .map_err(|error| SessionError::Objects(error.to_string()))?;
    let source = format!("{basis}\n{kind_json}\n{}\n{payload_digest}", prepare.target);
    Ok(digest(source.as_bytes())[..20].to_string())
}

/// The scope an effect's payload is granted to: its conversation's when
/// the effect came from one, so the text dies with the conversation
/// (plan M7a-b); otherwise the effect's own.
fn payload_scope(effect: &EffectId, trace: Option<&TraceKey>) -> String {
    match trace.and_then(TraceKey::session_id) {
        Some(session) => crate::objects::conversation_scope(&session),
        None => format!("effect:{effect}"),
    }
}

fn digest(bytes: &[u8]) -> String {
    let hash = Sha256::digest(bytes);
    hash.iter().map(|byte| format!("{byte:02x}")).collect()
}

/// The effect records of one data home.
#[derive(Debug, Clone)]
pub struct Effects {
    chain: RecordChain,
    projection: PathBuf,
    tenant_home: PathBuf,
    holder: ProcessId,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct EffectCursor {
    prepared_at: DateTime<Utc>,
    id: EffectId,
}

/// One bounded page of effects, newest first. `next_cursor` resumes strictly
/// after the last returned record in that order.
#[derive(Debug, Clone, Serialize)]
pub struct EffectPage {
    pub effects: Vec<EffectRecord>,
    pub next_cursor: Option<String>,
}

fn encode_cursor(cursor: &EffectCursor) -> Result<String, SessionError> {
    let bytes =
        serde_json::to_vec(cursor).map_err(|error| SessionError::Objects(error.to_string()))?;
    let mut encoded = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        use std::fmt::Write as _;
        let _ = write!(encoded, "{byte:02x}");
    }
    Ok(encoded)
}

fn decode_cursor(encoded: &str) -> Result<EffectCursor, SessionError> {
    if encoded.is_empty()
        || !encoded.len().is_multiple_of(2)
        || !encoded.bytes().all(|b| b.is_ascii_hexdigit())
    {
        return Err(SessionError::Objects("invalid effect page cursor".into()));
    }
    let mut bytes = Vec::with_capacity(encoded.len() / 2);
    for pair in encoded.as_bytes().as_chunks::<2>().0 {
        let pair = std::str::from_utf8(pair)
            .map_err(|_| SessionError::Objects("invalid effect page cursor".into()))?;
        bytes.push(
            u8::from_str_radix(pair, 16)
                .map_err(|_| SessionError::Objects("invalid effect page cursor".into()))?,
        );
    }
    let cursor: EffectCursor = serde_json::from_slice(&bytes)
        .map_err(|_| SessionError::Objects("invalid effect page cursor".into()))?;
    if encode_cursor(&cursor)? != encoded {
        return Err(SessionError::Objects(
            "non-canonical effect page cursor".into(),
        ));
    }
    Ok(cursor)
}

impl Effects {
    /// Validates the canonical opaque cursor accepted by [`Self::list_page`].
    pub fn validate_page_cursor(cursor: &str) -> Result<(), SessionError> {
        decode_cursor(cursor).map(|_| ())
    }

    /// The records in `dir` (`SharedScope::effects`), with payloads and
    /// dispatch refs in the tenant at `tenant_home`.
    pub fn at(dir: impl Into<PathBuf>, tenant_home: impl Into<PathBuf>) -> Self {
        let chain = RecordChain::at(dir);
        let projection = vak_config::scope::SharedScope::new(
            chain.path().parent().unwrap_or_else(|| Path::new(".")),
        )
        .effects_projection();
        Self {
            chain,
            projection,
            tenant_home: tenant_home.into(),
            holder: crate::fence::process(),
        }
    }

    /// Uses the canonical encrypted effects projection from `SharedScope`.
    pub fn with_projection(mut self, path: impl Into<PathBuf>) -> Self {
        self.projection = path.into();
        self
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
        let run = prepare.trace.as_ref().map(|trace| trace.run);
        let payload_digest = digest(&prepare.payload);
        let ordinal = self.projection()?.matching_count(
            run,
            &prepare.kind,
            &prepare.target,
            &payload_digest,
        )?;
        let run_text = run.map(|run| run.to_string()).unwrap_or_default();
        let key = key_of(&prepare, &payload_digest, &format!("{run_text}\n{ordinal}"))?;
        self.write_prepared(prepare, key, payload_digest)
    }

    /// Records the one effect `prepare` describes, whatever run asks: its
    /// key is the hash of its kind, target and payload digest alone, so
    /// the same action prepared twice (a second click, a second process)
    /// is one effect. Returns the effect prepared now, or the one that
    /// already exists as `Err`. Only one effect under a key is ever taken
    /// (`begin_dispatch`), so a race that prepares two sends at most once.
    pub fn prepare_once(
        &self,
        prepare: Prepare,
    ) -> Result<Result<EffectRecord, EffectRecord>, SessionError> {
        let payload_digest = digest(&prepare.payload);
        let key = key_of(&prepare, &payload_digest, "once")?;
        if let Some(existing) = self.projection()?.find_key(&key)? {
            return Ok(Err(existing));
        }
        self.write_prepared(prepare, key, payload_digest).map(Ok)
    }

    fn write_prepared(
        &self,
        prepare: Prepare,
        idempotency_key: String,
        payload_digest: String,
    ) -> Result<EffectRecord, SessionError> {
        let effect = EffectId::new();
        let run = prepare.trace.as_ref().map(|trace| trace.run);
        let tenant = TenantObjects::for_tenant(&self.tenant_home)?;
        let payload = tenant.put(
            &prepare.payload,
            &payload_scope(&effect, prepare.trace.as_ref()),
        )?;
        let actor = prepare.trace.as_ref().and_then(|trace| trace.actor);
        let step = EffectStep::Prepared {
            kind: prepare.kind,
            run,
            idempotency_key,
            payload_digest,
            target: prepare.target,
            payload,
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
        TenantObjects::for_tenant(&self.tenant_home)?.get(
            &record.payload,
            &payload_scope(&record.id, record.trace.as_ref()),
        )
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
        let mut taken_by = None;
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
                // Another effect under this key took it: this one is a
                // twin a race prepared, and is never sent.
                if let Some(claim) = current.as_ref().filter(|claim| claim.effect != effect) {
                    taken_by = Some(claim.effect);
                    return Ok(None);
                }
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
            if let Some(by) = taken_by {
                self.append(effect, None, None, EffectStep::Superseded { by })?;
            }
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

    /// Records as failed, never sent, every effect of a kind that is not
    /// sent again by itself that is still queued `max_age` after it was
    /// prepared: the request that approved it stopped before it sent it,
    /// and its approval has lapsed. Returns them.
    pub fn fail_unsent(
        &self,
        now: DateTime<Utc>,
        max_age: chrono::Duration,
    ) -> Result<Vec<EffectId>, SessionError> {
        let mut failed = Vec::new();
        for record in self.list()? {
            if record.status == EffectStatus::Queued
                && !record.kind.retries()
                && now - record.prepared_at > max_age
            {
                self.append(
                    record.id,
                    None,
                    None,
                    EffectStep::Failed {
                        reason: "its approval lapsed before it was sent".into(),
                        proven_not_sent: true,
                    },
                )?;
                failed.push(record.id);
            }
        }
        Ok(failed)
    }

    /// Every event, in order. A row that is not an effect event is an
    /// error, never skipped.
    pub fn events(&self) -> Result<Vec<EffectEvent>, SessionError> {
        self.chain
            .try_read::<serde_json::Value>()?
            .into_iter()
            .map(|row| {
                serde_json::from_value(row).map_err(|error| SessionError::Corrupt {
                    line: 0,
                    message: format!("effect record: {error}"),
                })
            })
            .collect()
    }

    fn projection(&self) -> Result<projection::Projection, SessionError> {
        let mut projection = projection::Projection::open(&self.projection, &self.tenant_home)?;
        projection.sync(self.chain.path())?;
        Ok(projection)
    }

    /// Every effect, newest first. Full-list callers deliberately read every
    /// record; detail and page callers decode only their selected rows.
    pub fn list(&self) -> Result<Vec<EffectRecord>, SessionError> {
        self.projection()?.list()
    }

    /// Reads a bounded page from the verified encrypted projection. The
    /// cursor resumes strictly before its prior record in newest-first order.
    pub fn list_page(
        &self,
        run: Option<RunId>,
        state: Option<EffectStatus>,
        limit: usize,
        cursor: Option<&str>,
    ) -> Result<EffectPage, SessionError> {
        let after = cursor.map(decode_cursor).transpose()?;
        let (effects, more) = self.projection()?.list_page(
            run,
            state,
            limit,
            after.map(|cursor| (cursor.prepared_at, cursor.id)),
        )?;
        let next_cursor = if more {
            effects
                .last()
                .map(|record| {
                    encode_cursor(&EffectCursor {
                        prepared_at: record.prepared_at,
                        id: record.id,
                    })
                })
                .transpose()?
        } else {
            None
        };
        Ok(EffectPage {
            effects,
            next_cursor,
        })
    }

    pub fn get(&self, effect: EffectId) -> Result<Option<EffectRecord>, SessionError> {
        self.projection()?.get(effect)
    }

    /// Every effect the run `run` caused, newest first.
    pub fn of_run(&self, run: RunId) -> Result<Vec<EffectRecord>, SessionError> {
        self.projection()?.of_run(run)
    }

    /// Effects the background sender may take now, oldest first: only
    /// kinds that are sent again by themselves (`EffectKind::retries`). A
    /// mail or calendar change is sent only by the request that approved it.
    pub fn dispatchable(&self) -> Result<Vec<EffectRecord>, SessionError> {
        self.projection()?.dispatchable()
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

    #[test]
    fn malformed_and_unreadable_records_cannot_enable_dispatch() {
        let (_dir, effects) = home();
        let record = delivery(&effects, &trace(), "queued");
        assert!(effects.get(record.id).unwrap().is_some());
        let segments = vak_storage::segments::SegmentSet::open(effects.chain.path()).unwrap();
        let lock = segments.lock().unwrap();
        let mut writer = segments.writer(1, &lock).unwrap();
        writer.append(b"{", None).unwrap();
        drop(writer);
        drop(lock);
        assert!(effects.events().is_err());
        assert!(effects.get(record.id).is_err());
        assert!(effects.dispatchable().is_err());

        std::fs::write(segments.log_path(1), b"broken").unwrap();
        assert!(effects.events().is_err());
        assert!(effects.dispatchable().is_err());
    }

    #[test]
    fn empty_projection_and_deleted_source_are_distinguished() {
        let (_dir, effects) = home();
        assert!(effects.list().unwrap().is_empty());
        assert!(effects.get(EffectId::new()).unwrap().is_none());
        assert!(
            !effects.chain.path().exists(),
            "reading never creates a source chain"
        );
        let record = delivery(&effects, &trace(), "source survives");
        assert!(effects.get(record.id).unwrap().is_some());
        // Fault injection only: normal data removal uses lifecycle/erasure.
        std::fs::remove_dir_all(effects.chain.path()).unwrap();
        assert!(effects.get(record.id).is_err());
    }

    #[test]
    fn projection_catch_up_rolls_back_all_rows_on_an_invalid_suffix() {
        let (_dir, effects) = home();
        let first = delivery(&effects, &trace(), "first");
        let mut index = effects.projection().unwrap();
        let next_id = EffectId::new();
        let mut rows = effects.events().unwrap();
        rows[0].effect = next_id;
        rows.push(EffectEvent {
            effect: EffectId::new(),
            at: Utc::now(),
            trace: None,
            actor: None,
            step: EffectStep::Unknown {
                reason: "orphan".into(),
            },
        });
        effects.chain.append_all(&rows).unwrap();
        assert!(index.sync(effects.path()).is_err());
        assert!(index.get(first.id).unwrap().is_some());
        assert!(
            index.get(next_id).unwrap().is_none(),
            "no partial transaction is visible"
        );
        drop(index);
        assert!(
            effects.get(next_id).is_err(),
            "the failed suffix is retried after reopen"
        );
    }

    #[test]
    fn projection_is_encrypted_and_live_handles_refuse_a_destroyed_key() {
        let (_dir, effects) = home();
        let record = delivery(&effects, &trace(), "payload");
        let mut index = effects.projection().unwrap();
        // A unique content-bearing reason exercises both the database and WAL.
        let marker = "private-effect-reason-7d8fa9e0";
        effects.failed(record.id, marker, false).unwrap();
        index.sync(effects.path()).unwrap();
        for entry in std::fs::read_dir(effects.projection.parent().unwrap()).unwrap() {
            let bytes = std::fs::read(entry.unwrap().path()).unwrap();
            assert!(
                !bytes
                    .windows(marker.len())
                    .any(|part| part == marker.as_bytes())
            );
            assert!(!bytes.starts_with(b"SQLite format 3"));
        }
        let tenant = TenantObjects::for_tenant(&effects.tenant_home).unwrap();
        // Key destruction is a fixture for the production erasure boundary.
        tenant.destroy_scope_key("effects-index").unwrap();
        assert!(index.get(record.id).is_err());
        assert!(effects.get(record.id).is_err());
    }

    #[test]
    fn filtered_projection_pages_keep_state_and_run_boundaries() {
        let (_dir, effects) = home();
        let first_trace = trace();
        let other_trace = trace();
        let records: Vec<_> = (0..5)
            .map(|n| delivery(&effects, &first_trace, &format!("first-{n}")))
            .collect();
        let other = delivery(&effects, &other_trace, "other");
        effects.failed(records[1].id, "unknown", false).unwrap();
        effects.failed(records[3].id, "unknown", false).unwrap();
        let first = effects
            .list_page(Some(first_trace.run), Some(EffectStatus::Unknown), 1, None)
            .unwrap();
        assert_eq!(first.effects.len(), 1);
        let second = effects
            .list_page(
                Some(first_trace.run),
                Some(EffectStatus::Unknown),
                1,
                first.next_cursor.as_deref(),
            )
            .unwrap();
        assert_eq!(second.effects.len(), 1);
        assert_ne!(first.effects[0].id, second.effects[0].id);
        assert!(second.next_cursor.is_none());
        assert_eq!(
            effects
                .of_run(other_trace.run)
                .unwrap()
                .iter()
                .map(|r| r.id)
                .collect::<Vec<_>>(),
            vec![other.id]
        );
        assert!(
            effects
                .dispatchable()
                .unwrap()
                .iter()
                .all(|r| r.status == EffectStatus::Queued)
        );
    }

    #[test]
    fn projection_reads_cold_and_warm_and_pages_without_losing_records() {
        let (_dir, effects) = home();
        let trace = trace();
        let records: Vec<_> = (0..5)
            .map(|n| delivery(&effects, &trace, &format!("page-{n}")))
            .collect();

        let cold = effects.list_page(None, None, 2, None).expect("cold page");
        assert_eq!(cold.effects.len(), 2);
        assert!(cold.next_cursor.is_some());
        assert!(
            effects.projection.is_file(),
            "cold read writes the encrypted projection"
        );
        let warm = effects.list_page(None, None, 2, None).expect("warm page");
        assert_eq!(cold.effects, warm.effects);

        let mut seen = cold
            .effects
            .iter()
            .map(|record| record.id)
            .collect::<Vec<_>>();
        let next = effects
            .list_page(None, None, 2, cold.next_cursor.as_deref())
            .expect("second page");
        assert_eq!(next.effects.len(), 2);
        let final_page = effects
            .list_page(None, None, 2, next.next_cursor.as_deref())
            .expect("final page");
        assert_eq!(final_page.effects.len(), 1);
        assert!(final_page.next_cursor.is_none());
        seen.extend(next.effects.iter().map(|record| record.id));
        seen.extend(final_page.effects.iter().map(|record| record.id));
        seen.sort();
        seen.dedup();
        let mut expected = records.iter().map(|record| record.id).collect::<Vec<_>>();
        expected.sort();
        assert_eq!(seen, expected);
        assert!(effects.list_page(None, None, 2, Some("zz")).is_err());
    }

    #[allow(clippy::disallowed_macros)] // prints a measurement for the person running it
    /// Local growth measurement for A09. Run with
    /// `cargo test -p vak-session effects_index_growth_measurement -- --ignored --nocapture`.
    /// The chain is seeded through RecordChain in one synced batch so setup
    /// does not measure per-row fsync. Reads below use the production Effects
    /// encrypted projection and lookup/page APIs. Cold reads include replay;
    /// warm reads verify the source suffix and decode only selected rows.
    #[test]
    #[ignore = "manual local growth measurement; writes up to 10,000 effect rows"]
    fn effects_index_growth_measurement() {
        use std::hint::black_box;
        use std::time::Instant;

        for count in [100usize, 1_000, 10_000] {
            let (_dir, effects) = home();
            let events: Vec<_> = (0..count)
                .map(|n| EffectEvent {
                    effect: EffectId::new(),
                    at: Utc::now() + chrono::Duration::microseconds(n as i64),
                    trace: None,
                    actor: None,
                    step: EffectStep::Prepared {
                        kind: EffectKind::delivery("benchmark:target"),
                        run: None,
                        idempotency_key: format!("{n:020}"),
                        payload_digest: "0".repeat(64),
                        target: "benchmark:target".into(),
                        payload: ObjectRef {
                            id: "benchmark-object".into(),
                            len: 0,
                        },
                        supersedes: None,
                    },
                })
                .collect();
            effects.chain.append_all(&events).expect("seed effect rows");
            let chosen = events[count / 2].effect;

            let cold_start = Instant::now();
            let cold = effects.list_page(None, None, 25, None).expect("cold page");
            let cold_page = cold_start.elapsed();
            assert_eq!(cold.effects.len(), 25);

            let get_start = Instant::now();
            let found = effects.get(black_box(chosen)).expect("indexed get");
            let get = get_start.elapsed();
            assert!(found.is_some());

            let page_start = Instant::now();
            let page = effects.list_page(None, None, 25, None).expect("warm page");
            let warm_page = page_start.elapsed();
            assert_eq!(page.effects.len(), 25);

            eprintln!(
                "effects_growth rows={count} cold_page_ms={:.3} warm_get_ms={:.3} warm_page_ms={:.3}",
                cold_page.as_secs_f64() * 1_000.0,
                get.as_secs_f64() * 1_000.0,
                warm_page.as_secs_f64() * 1_000.0,
            );
        }
    }

    #[test]
    fn a_delivery_from_a_conversation_goes_with_it() {
        let (dir, effects) = home();
        let session = uuid::Uuid::now_v7().to_string();
        let from_conversation = trace().in_turn(&session, &uuid::Uuid::now_v7().to_string());
        let said = delivery(&effects, &from_conversation, "the offer is accepted");
        let system = delivery(&effects, &trace(), "the budget is near its cap");
        assert_eq!(
            effects.payload(&said).expect("payload"),
            b"the offer is accepted"
        );

        TenantObjects::for_tenant(&dir.path().join("tenant"))
            .expect("tenant")
            .destroy_scope_key(&crate::objects::conversation_scope(&session))
            .expect("destroy");

        assert!(matches!(
            effects.payload(&said),
            Err(SessionError::Erased(_))
        ));
        assert_eq!(
            effects.payload(&system).expect("payload"),
            b"the budget is near its cap"
        );
        assert_eq!(
            effects.get(said.id).expect("read").map(|r| r.id),
            Some(said.id)
        );
    }

    fn delivery(effects: &Effects, trace: &TraceKey, text: &str) -> EffectRecord {
        effects
            .prepare(Prepare {
                kind: EffectKind::delivery("discord:chan:bot"),
                target: "discord:chan:bot".into(),
                trace: Some(trace.clone()),
                payload: text.as_bytes().to_vec(),
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

    fn mail(candidate: &str) -> Prepare {
        Prepare::new(
            EffectKind::MailSend {
                account: "acct".into(),
            },
            format!("candidate:{candidate}"),
            Some(trace()),
            candidate.as_bytes().to_vec(),
        )
    }

    #[test]
    fn a_once_effect_is_one_effect_and_is_never_retried() {
        let (_dir, effects) = home();
        let first = effects
            .prepare_once(mail("c1"))
            .expect("prepare")
            .expect("new");
        let again = effects.prepare_once(mail("c1")).expect("prepare");
        assert_eq!(again.expect_err("existing").id, first.id);
        effects
            .begin_dispatch(first.id, Dispatch::Fresh)
            .expect("dispatch")
            .expect("taken");
        effects.failed(first.id, "refused", true).expect("failed");
        let failed = effects.get(first.id).expect("get").expect("record");
        assert_eq!(failed.status, EffectStatus::Failed);
        assert!(effects.dispatchable().expect("dispatchable").is_empty());
        assert!(
            effects
                .begin_dispatch(first.id, Dispatch::Fresh)
                .expect("fresh")
                .is_none()
        );
    }

    #[test]
    fn a_raced_twin_is_superseded_not_sent() {
        let (_dir, effects) = home();
        let first = effects
            .prepare_once(mail("c2"))
            .expect("prepare")
            .expect("new");
        // A second process prepared the same action before it saw the first.
        let twin = effects
            .write_prepared(
                mail("c2"),
                first.idempotency_key.clone(),
                first.payload_digest.clone(),
            )
            .expect("twin");
        effects
            .begin_dispatch(first.id, Dispatch::Fresh)
            .expect("dispatch")
            .expect("taken");
        assert!(
            effects
                .begin_dispatch(twin.id, Dispatch::Fresh)
                .expect("twin dispatch")
                .is_none()
        );
        let twin = effects.get(twin.id).expect("get").expect("record");
        assert_eq!(twin.status, EffectStatus::Superseded);
        assert_eq!(twin.superseded_by, Some(first.id));
    }

    #[test]
    fn an_approved_change_left_unsent_fails_once_its_approval_lapses() {
        let (_dir, effects) = home();
        let left = effects
            .prepare_once(mail("c3"))
            .expect("prepare")
            .expect("new");
        let soon = Utc::now() + chrono::Duration::minutes(1);
        assert!(
            effects
                .fail_unsent(soon, chrono::Duration::minutes(5))
                .expect("sweep")
                .is_empty()
        );
        let later = Utc::now() + chrono::Duration::minutes(6);
        assert_eq!(
            effects
                .fail_unsent(later, chrono::Duration::minutes(5))
                .expect("sweep"),
            vec![left.id]
        );
        assert_eq!(
            effects.get(left.id).expect("get").expect("record").status,
            EffectStatus::Failed
        );
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
