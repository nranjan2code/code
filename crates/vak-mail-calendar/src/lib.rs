//! Provider-neutral mail and calendar contracts.
//!
//! This crate deliberately knows neither OAuth tokens nor raw provider
//! requests. Host code admits accounts and grants; adapters receive a single
//! validated typed operation through that boundary.

#![cfg_attr(test, allow(clippy::expect_used, clippy::panic, clippy::unwrap_used))]

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::BTreeSet;
use uuid::Uuid;

pub mod connection_ledger;
pub mod effect;
pub mod oauth;
pub mod provider;
pub mod vault;

pub const MAX_ADDRESSES: usize = 100;
pub const MAX_SUBJECT_BYTES: usize = 998;
pub const MAX_BODY_BYTES: usize = 1_048_576;
pub const MAX_EVENT_TITLE_BYTES: usize = 512;
/// Maximum content-free provider-ID scan and durable backlog size for one
/// unattended mail routine.
pub const MAX_ROUTINE_MAIL_BACKLOG: usize = 100;
/// Maximum number of messages returned for one explicitly selected thread page.
pub const MAX_MAIL_THREAD_MESSAGES: usize = 20;
/// Reserved account key for an encrypted Agent-local draft that has not yet
/// been assigned to a provider account. It can never authorize a provider
/// effect.
pub const LOCAL_DRAFT_ACCOUNT_ID: &str = "local-draft";

pub type AccountId = String;
pub type CandidateId = String;
pub type AgentId = String;
pub type AudienceId = String;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Provider {
    Google,
    Microsoft,
    AppleIcloud,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AccountStatus {
    Pending,
    Connected,
    ConnectedUnverified,
    ReauthenticationRequired,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Capability {
    MailRead,
    MailPrepare,
    MailSend,
    CalendarFreeBusy,
    CalendarRead,
    CalendarWrite,
}

/// Immutable source and operation boundary for an unattended mail/calendar
/// routine. This is stored as part of the existing `TaskDef`; it does not
/// introduce another scheduler or credential store.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RoutineScope {
    /// Set by the server from the owning trigger's UUID; callers cannot
    /// choose a different durable cursor namespace.
    #[serde(default)]
    pub routine_id: String,
    pub account_id: AccountId,
    /// One owner-selected mail folder/label for recent mail reads. `None`
    /// preserves the provider Inbox default for older saved routines.
    #[serde(default)]
    pub mail_folder_id: Option<String>,
    /// One owner-selected calendar for unattended event reads. `None`
    /// preserves the provider's legacy default behavior for saved routines.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub calendar_source_id: Option<String>,
    pub operations: BTreeSet<RoutineOperation>,
    pub max_items: u8,
    #[serde(default)]
    pub watch_new_mail: bool,
    /// Explicitly allow this routine to read its owning Agent's open
    /// commitments for cross-activity summaries. Never grants write access.
    #[serde(default)]
    pub read_commitments: bool,
    /// Optional trigger relative to one timed calendar event. Execution still
    /// goes through the owning trigger's scheduler and its claim, one run at
    /// a time.
    #[serde(default)]
    pub calendar_event_trigger: Option<CalendarEventTrigger>,
}

/// Trigger a routine around an event's start or end. Positive offsets fire
/// before the boundary; negative offsets fire after it. The scheduler may
/// catch up a missed trigger only within `max_lateness_minutes`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CalendarEventTrigger {
    pub boundary: CalendarEventBoundary,
    pub offset_minutes: i16,
    pub max_lateness_minutes: u16,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CalendarEventBoundary {
    Start,
    End,
}

impl CalendarEventTrigger {
    pub fn validate(&self) -> Result<(), ContractError> {
        if self.offset_minutes.unsigned_abs() > 10_080
            || !(1..=1_440).contains(&self.max_lateness_minutes)
        {
            return Err(ContractError::InvalidRoutineScope);
        }
        Ok(())
    }

    pub fn trigger_at(
        &self,
        event_starts_at: DateTime<Utc>,
        event_ends_at: DateTime<Utc>,
    ) -> DateTime<Utc> {
        let boundary = match self.boundary {
            CalendarEventBoundary::Start => event_starts_at,
            CalendarEventBoundary::End => event_ends_at,
        };
        boundary - chrono::Duration::minutes(i64::from(self.offset_minutes))
    }

    /// True for a newly crossed trigger that is still within its explicit
    /// catch-up window. `checked_after` is the prior successful calendar
    /// poll, not the previous model run.
    pub fn is_due_between(
        &self,
        event_starts_at: DateTime<Utc>,
        event_ends_at: DateTime<Utc>,
        checked_after: DateTime<Utc>,
        now: DateTime<Utc>,
    ) -> bool {
        let trigger_at = self.trigger_at(event_starts_at, event_ends_at);
        trigger_at > checked_after
            && trigger_at <= now
            && now - trigger_at <= chrono::Duration::minutes(i64::from(self.max_lateness_minutes))
    }

    /// Opaque content-free key for deduplicating one timed event occurrence.
    /// Moving the occurrence creates a fresh key; changing its title does not.
    pub fn occurrence_key(
        &self,
        provider_event_id: &str,
        event_starts_at: DateTime<Utc>,
        event_ends_at: DateTime<Utc>,
    ) -> String {
        let mut hasher = Sha256::new();
        hasher.update(b"mail-calendar-event-trigger-v1");
        hasher.update((provider_event_id.len() as u64).to_be_bytes());
        hasher.update(provider_event_id.as_bytes());
        hasher.update([match self.boundary {
            CalendarEventBoundary::Start => 0,
            CalendarEventBoundary::End => 1,
        }]);
        hasher.update(self.offset_minutes.to_be_bytes());
        hasher.update(event_starts_at.timestamp_millis().to_be_bytes());
        hasher.update(event_ends_at.timestamp_millis().to_be_bytes());
        let digest = hasher.finalize();
        let mut encoded = String::with_capacity(32 * 2);
        for byte in digest {
            use std::fmt::Write as _;
            let _ = write!(encoded, "{byte:02x}");
        }
        format!("calendar:{encoded}")
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RoutineOperation {
    RecentMail,
    MailThread,
    CalendarEvents,
    FreeBusy,
}

impl RoutineScope {
    pub fn validate(&self) -> Result<(), ContractError> {
        if Uuid::parse_str(&self.account_id).is_err()
            || !Uuid::parse_str(&self.routine_id).is_ok_and(|id| id.get_version_num() == 7)
            || self.operations.is_empty()
            || !(1..=20).contains(&self.max_items)
            || (self.watch_new_mail && !self.operations.contains(&RoutineOperation::RecentMail))
            || (self.calendar_event_trigger.is_some()
                && (!self.operations.contains(&RoutineOperation::CalendarEvents)
                    || self.watch_new_mail))
            || self.mail_folder_id.as_ref().is_some_and(|folder| {
                folder.trim().is_empty()
                    || folder.len() > 512
                    || folder.chars().any(char::is_control)
            })
            || (self.mail_folder_id.is_some()
                && !self.operations.contains(&RoutineOperation::RecentMail))
            || self.calendar_source_id.as_ref().is_some_and(|source| {
                source.trim().is_empty()
                    || source.len() > 2048
                    || source.chars().any(char::is_control)
            })
            || (self.calendar_source_id.is_some()
                && !self.operations.contains(&RoutineOperation::CalendarEvents))
            || (self.watch_new_mail
                && self
                    .mail_folder_id
                    .as_deref()
                    .is_some_and(|folder| !matches!(folder, "INBOX" | "inbox")))
        {
            return Err(ContractError::InvalidRoutineScope);
        }
        if self
            .calendar_event_trigger
            .is_some_and(|trigger| trigger.validate().is_err())
        {
            return Err(ContractError::InvalidRoutineScope);
        }
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ConnectedAccount {
    pub id: AccountId,
    pub provider: Provider,
    pub status: AccountStatus,
    pub owner_agent_id: AgentId,
    pub allowed_audiences: BTreeSet<AudienceId>,
    pub capabilities: BTreeSet<Capability>,
    pub provider_scopes: BTreeSet<String>,
    /// Opaque secret-store handle. It is never a token or password.
    pub credential_ref: String,
    /// Account identity/display data stays in the secret vault so deleting
    /// the connection can remove it without rewriting this event ledger.
    pub principal_ref: String,
    pub revision: u64,
    pub connected_at: DateTime<Utc>,
    /// OAuth tokens expire; app-specific passwords do not expose a token
    /// expiry, so this is absent for those credentials.
    pub access_token_expires_at: Option<DateTime<Utc>>,
    pub refresh_token_available: bool,
    pub revoked_at: Option<DateTime<Utc>>,
}

impl ConnectedAccount {
    pub fn admits(&self, agent: &str, audience: &str, capability: Capability) -> bool {
        self.status == AccountStatus::Connected
            && self.revoked_at.is_none()
            && self.owner_agent_id == agent
            && self.allowed_audiences.contains(audience)
            && self.capabilities.contains(&capability)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MailAddress {
    pub address: String,
    pub display_name: Option<String>,
}

impl MailAddress {
    pub fn validate(&self) -> Result<(), ContractError> {
        let address = self.address.trim();
        if address.is_empty()
            || address.len() > 320
            || address.contains(['\r', '\n', '<', '>'])
            || address.matches('@').count() != 1
        {
            return Err(ContractError::InvalidAddress);
        }
        let (local, domain) = address
            .split_once('@')
            .ok_or(ContractError::InvalidAddress)?;
        if local.is_empty()
            || local.len() > 64
            || domain.is_empty()
            || !domain.contains('.')
            || domain.starts_with('.')
            || domain.ends_with('.')
        {
            return Err(ContractError::InvalidAddress);
        }
        if self
            .display_name
            .as_ref()
            .is_some_and(|name| name.len() > 256 || name.contains(['\r', '\n']))
        {
            return Err(ContractError::InvalidAddress);
        }
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MailDraft {
    pub from_alias: Option<String>,
    pub to: Vec<MailAddress>,
    pub cc: Vec<MailAddress>,
    pub bcc: Vec<MailAddress>,
    pub subject: String,
    /// Plain text is canonical. HTML authoring requires a separately
    /// sanitized, deterministic representation in the client.
    pub body_text: String,
    pub attachment_refs: Vec<String>,
    #[serde(default)]
    pub reply_to_message_id: Option<String>,
    #[serde(default)]
    pub reply_to_thread_id: Option<String>,
}

impl MailDraft {
    pub fn validate(&self) -> Result<(), ContractError> {
        if self.to.is_empty() || self.to.len() + self.cc.len() + self.bcc.len() > MAX_ADDRESSES {
            return Err(ContractError::InvalidRecipients);
        }
        self.validate_fields()
    }

    fn validate_fields(&self) -> Result<(), ContractError> {
        if self.to.len() + self.cc.len() + self.bcc.len() > MAX_ADDRESSES {
            return Err(ContractError::InvalidRecipients);
        }
        for address in self.to.iter().chain(&self.cc).chain(&self.bcc) {
            address.validate()?;
        }
        if self.subject.len() > MAX_SUBJECT_BYTES
            || self.subject.contains(['\r', '\n'])
            || self.body_text.len() > MAX_BODY_BYTES
            || self.attachment_refs.len() > 20
            || self
                .attachment_refs
                .iter()
                .any(|value| value.trim().is_empty())
        {
            return Err(ContractError::LimitExceeded);
        }
        match (
            self.reply_to_message_id.as_deref(),
            self.reply_to_thread_id.as_deref(),
        ) {
            (None, None) => {}
            (Some(message_id), Some(thread_id))
                if !message_id.is_empty()
                    && message_id.len() <= 512
                    && !message_id.chars().any(char::is_control)
                    && !thread_id.is_empty()
                    && thread_id.len() <= 512
                    && !thread_id.chars().any(char::is_control) => {}
            _ => return Err(ContractError::LimitExceeded),
        }
        if self
            .from_alias
            .as_ref()
            .is_some_and(|value| value.trim().is_empty() || value.contains(['\r', '\n']))
        {
            return Err(ContractError::InvalidAddress);
        }
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CalendarDraft {
    pub title: String,
    pub description: String,
    pub location: Option<String>,
    pub starts_at: DateTime<Utc>,
    pub ends_at: DateTime<Utc>,
    pub time_zone: String,
    pub all_day: bool,
    pub attendee_addresses: Vec<MailAddress>,
    pub recurrence: Option<String>,
    pub occurrence_id: Option<String>,
}

impl CalendarDraft {
    pub fn validate(&self) -> Result<(), ContractError> {
        if self.title.trim().is_empty()
            || self.title.len() > MAX_EVENT_TITLE_BYTES
            || self.description.len() > MAX_BODY_BYTES
            || self.time_zone.parse::<chrono_tz::Tz>().is_err()
            || self.ends_at <= self.starts_at
            || self.attendee_addresses.len() > MAX_ADDRESSES
        {
            return Err(ContractError::InvalidEvent);
        }
        for address in &self.attendee_addresses {
            address.validate()?;
        }
        if self
            .recurrence
            .as_ref()
            .is_some_and(|value| value.len() > 4096)
        {
            return Err(ContractError::LimitExceeded);
        }
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum ProposedAction {
    SendMail {
        draft: MailDraft,
    },
    CreateEvent {
        draft: CalendarDraft,
    },
    UpdateEvent {
        event_id: String,
        source_version: String,
        draft: CalendarDraft,
    },
    CancelEvent {
        event_id: String,
        source_version: String,
        occurrence_id: Option<String>,
        whole_series: bool,
    },
    RespondToEvent {
        event_id: String,
        source_version: String,
        response: AttendeeResponse,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AttendeeResponse {
    Accept,
    Tentative,
    Decline,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ActionCandidate {
    pub id: CandidateId,
    pub account_id: AccountId,
    pub agent_id: AgentId,
    pub audience_id: AudienceId,
    pub source_refs: Vec<SourceRef>,
    pub action: ProposedAction,
    pub revision: u64,
    pub created_at: DateTime<Utc>,
}

impl ActionCandidate {
    pub fn new(
        account_id: AccountId,
        agent_id: AgentId,
        audience_id: AudienceId,
        source_refs: Vec<SourceRef>,
        action: ProposedAction,
    ) -> Result<Self, ContractError> {
        validate_candidate_scope(&account_id, &source_refs, &action)?;
        validate_scope(&agent_id)?;
        validate_scope(&audience_id)?;
        validate_source_refs(&source_refs)?;
        if account_id == LOCAL_DRAFT_ACCOUNT_ID {
            validate_local_draft(&source_refs, &action)?;
        } else {
            validate_action(&action)?;
        }
        Ok(Self {
            id: Uuid::now_v7().to_string(),
            account_id,
            agent_id,
            audience_id,
            source_refs,
            action,
            revision: 1,
            created_at: Utc::now(),
        })
    }

    /// Digest of the exact candidate payload, bound to account, Agent,
    /// audience, source versions, and revision.
    pub fn digest(&self) -> Result<String, ContractError> {
        let bytes = serde_json::to_vec(self).map_err(|_| ContractError::Serialization)?;
        Ok(hex_digest(&bytes))
    }

    /// Revalidate persisted or externally supplied candidate data before it
    /// enters a vault, preview, approval, or provider effect path.
    pub fn validate(&self) -> Result<(), ContractError> {
        let parsed = Uuid::parse_str(&self.id).map_err(|_| ContractError::InvalidScope)?;
        if parsed.get_version_num() != 7 || self.revision == 0 {
            return Err(ContractError::InvalidScope);
        }
        validate_candidate_scope(&self.account_id, &self.source_refs, &self.action)?;
        validate_scope(&self.agent_id)?;
        validate_scope(&self.audience_id)?;
        validate_source_refs(&self.source_refs)?;
        if self.account_id == LOCAL_DRAFT_ACCOUNT_ID {
            validate_local_draft(&self.source_refs, &self.action)
        } else {
            validate_action(&self.action)
        }
    }

    pub fn revise(&mut self, action: ProposedAction) -> Result<(), ContractError> {
        if self.account_id == LOCAL_DRAFT_ACCOUNT_ID {
            validate_local_draft(&self.source_refs, &action)?;
        } else {
            validate_action(&action)?;
        }
        let revision = self
            .revision
            .checked_add(1)
            .ok_or(ContractError::LimitExceeded)?;
        self.action = action;
        self.revision = revision;
        Ok(())
    }

    /// Capability required to stage the candidate's external provider effect.
    pub fn required_capability(&self) -> Capability {
        match &self.action {
            ProposedAction::SendMail { .. } => Capability::MailSend,
            ProposedAction::CreateEvent { .. }
            | ProposedAction::UpdateEvent { .. }
            | ProposedAction::CancelEvent { .. }
            | ProposedAction::RespondToEvent { .. } => Capability::CalendarWrite,
        }
    }

    /// Pure security gate for the broker to call immediately before dispatch.
    /// It verifies the exact connected account, Agent, audience, capability,
    /// and fresh digest-bound human approval. It does not grant runtime tool
    /// permission; the host still evaluates its permission engine separately.
    pub fn authorize_effect(
        &self,
        account: &ConnectedAccount,
        approval: &CandidateApproval,
        now: DateTime<Utc>,
    ) -> Result<(), ContractError> {
        if self.account_id == LOCAL_DRAFT_ACCOUNT_ID
            || self.account_id != account.id
            || !account.admits(
                &self.agent_id,
                &self.audience_id,
                self.required_capability(),
            )
        {
            return Err(ContractError::AccountDenied);
        }
        if !approval.matches(self, now)? {
            return Err(ContractError::ApprovalRequired);
        }
        Ok(())
    }
}

fn validate_candidate_scope(
    account_id: &str,
    source_refs: &[SourceRef],
    action: &ProposedAction,
) -> Result<(), ContractError> {
    if account_id == LOCAL_DRAFT_ACCOUNT_ID {
        validate_local_draft(source_refs, action)
    } else {
        validate_scope(account_id)
    }
}

fn validate_local_draft(
    source_refs: &[SourceRef],
    action: &ProposedAction,
) -> Result<(), ContractError> {
    let eligible = source_refs.is_empty()
        && match action {
            ProposedAction::SendMail { draft } => {
                draft.reply_to_message_id.is_none()
                    && draft.reply_to_thread_id.is_none()
                    && draft.validate_fields().is_ok()
            }
            ProposedAction::CreateEvent { draft } => {
                let mut draft = draft.clone();
                if draft.title.trim().is_empty() {
                    draft.title = "Draft event".into();
                }
                draft.attendee_addresses.is_empty()
                    && draft.recurrence.is_none()
                    && draft.occurrence_id.is_none()
                    && draft.validate().is_ok()
            }
            _ => false,
        };
    if eligible {
        Ok(())
    } else {
        Err(ContractError::AccountDenied)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SourceRef {
    pub item_id: String,
    pub version: Option<String>,
    pub label: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CandidateApproval {
    pub candidate_id: CandidateId,
    pub candidate_digest: String,
    pub account_id: AccountId,
    pub agent_id: AgentId,
    pub audience_id: AudienceId,
    pub approved_at: DateTime<Utc>,
    pub expires_at: DateTime<Utc>,
    pub single_use: bool,
}

impl CandidateApproval {
    pub fn matches(
        &self,
        candidate: &ActionCandidate,
        now: DateTime<Utc>,
    ) -> Result<bool, ContractError> {
        Ok(self.single_use
            && self.approved_at <= now
            && self.approved_at < self.expires_at
            && self.expires_at - self.approved_at <= chrono::Duration::minutes(5)
            && now < self.expires_at
            && self.candidate_id == candidate.id
            && self.account_id == candidate.account_id
            && self.agent_id == candidate.agent_id
            && self.audience_id == candidate.audience_id
            && self.candidate_digest == candidate.digest()?)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ActionState {
    Prepared,
    AwaitingApproval,
    Dispatching,
    ProviderAccepted,
    Confirmed,
    Failed,
    Unknown,
    Cancelled,
    Expired,
}

impl ActionState {
    pub fn can_transition_to(self, next: Self) -> bool {
        use ActionState::*;
        matches!(
            (self, next),
            (
                Prepared,
                AwaitingApproval | Dispatching | Cancelled | Expired
            ) | (AwaitingApproval, Dispatching | Cancelled | Expired)
                | (Dispatching, ProviderAccepted | Confirmed | Failed | Unknown)
                | (ProviderAccepted, Confirmed | Failed | Unknown)
                | (Unknown, Confirmed | Failed)
        )
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ActionReceipt {
    pub logical_action_id: String,
    pub attempt_id: String,
    pub candidate_id: CandidateId,
    pub candidate_digest: String,
    pub account_id: AccountId,
    pub state: ActionState,
    pub provider_item_id: Option<String>,
    pub observed_at: DateTime<Utc>,
    pub detail_code: Option<String>,
}

impl ActionReceipt {
    pub fn new(candidate: &ActionCandidate, state: ActionState) -> Result<Self, ContractError> {
        Ok(Self {
            logical_action_id: candidate.id.clone(),
            attempt_id: Uuid::now_v7().to_string(),
            candidate_id: candidate.id.clone(),
            candidate_digest: candidate.digest()?,
            account_id: candidate.account_id.clone(),
            state,
            provider_item_id: None,
            observed_at: Utc::now(),
            detail_code: None,
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MailMessage {
    pub id: String,
    pub thread_id: Option<String>,
    pub version: Option<String>,
    pub from: Option<MailAddress>,
    pub reply_to: Vec<MailAddress>,
    pub to: Vec<MailAddress>,
    pub cc: Vec<MailAddress>,
    pub subject: String,
    pub received_at: DateTime<Utc>,
    /// Sanitized text projection only. Raw MIME and HTML are never exposed to
    /// model tools or trusted client DOM.
    pub body_text: String,
    pub attachments: Vec<AttachmentSummary>,
    pub external_content: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AttachmentSummary {
    pub id: String,
    pub name: String,
    pub media_type: Option<String>,
    pub size_bytes: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CalendarEvent {
    pub id: String,
    pub version: Option<String>,
    pub title: String,
    pub description: Option<String>,
    pub starts_at: DateTime<Utc>,
    pub ends_at: DateTime<Utc>,
    pub time_zone: String,
    pub all_day: bool,
    pub private: bool,
    pub attendee_count: usize,
    pub recurrence: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FreeBusySlot {
    pub starts_at: DateTime<Utc>,
    pub ends_at: DateTime<Utc>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ReadRequest {
    pub account_id: AccountId,
    pub agent_id: AgentId,
    pub audience_id: AudienceId,
    pub time_min: DateTime<Utc>,
    pub time_max: DateTime<Utc>,
    pub max_results: u16,
}

impl ReadRequest {
    pub fn validate(&self) -> Result<(), ContractError> {
        if self.time_max <= self.time_min
            || self.time_max - self.time_min > chrono::Duration::days(366)
            || self.max_results == 0
            || self.max_results > 500
        {
            return Err(ContractError::InvalidReadWindow);
        }
        Ok(())
    }
}

#[derive(Debug, thiserror::Error, Clone, PartialEq, Eq)]
pub enum ContractError {
    #[error("email address is invalid or contains header control characters")]
    InvalidAddress,
    #[error("mail must have at least one recipient and stay within recipient limits")]
    InvalidRecipients,
    #[error("content exceeds the supported size limit")]
    LimitExceeded,
    #[error("calendar event is invalid")]
    InvalidEvent,
    #[error("read window or result limit is invalid")]
    InvalidReadWindow,
    #[error("candidate serialization failed")]
    Serialization,
    #[error("account, Agent, or audience scope is invalid")]
    InvalidScope,
    #[error("candidate source reference is invalid or exceeds its limit")]
    InvalidSource,
    #[error("the connected account does not authorize this candidate")]
    AccountDenied,
    #[error("a fresh approval for this exact candidate is required")]
    ApprovalRequired,
    #[error("mail/calendar routine scope is invalid")]
    InvalidRoutineScope,
}

fn validate_scope(value: &str) -> Result<(), ContractError> {
    if value.trim().is_empty() || value.len() > 256 || value.chars().any(char::is_control) {
        return Err(ContractError::InvalidScope);
    }
    Ok(())
}

fn validate_source_refs(source_refs: &[SourceRef]) -> Result<(), ContractError> {
    if source_refs.len() > 100
        || source_refs.iter().any(|source| {
            source.item_id.trim().is_empty()
                || source.item_id.len() > 512
                || source.item_id.chars().any(char::is_control)
                || source.version.as_ref().is_some_and(|value| {
                    value.is_empty() || value.len() > 1024 || value.chars().any(char::is_control)
                })
                || source
                    .label
                    .as_ref()
                    .is_some_and(|value| value.len() > 256 || value.chars().any(char::is_control))
        })
    {
        return Err(ContractError::InvalidSource);
    }
    Ok(())
}

fn validate_action(action: &ProposedAction) -> Result<(), ContractError> {
    match action {
        ProposedAction::SendMail { draft } => draft.validate(),
        ProposedAction::CreateEvent { draft } => draft.validate(),
        ProposedAction::UpdateEvent {
            event_id,
            source_version,
            draft,
        } => {
            if event_id.trim().is_empty()
                || event_id.len() > 512
                || event_id.chars().any(char::is_control)
                || source_version.trim().is_empty()
                || source_version.len() > 512
                || source_version.chars().any(char::is_control)
            {
                return Err(ContractError::InvalidEvent);
            }
            draft.validate()
        }
        ProposedAction::CancelEvent {
            event_id,
            source_version,
            occurrence_id,
            whole_series,
        } => {
            if event_id.len() < 5
                || event_id.len() > 1024
                || !event_id
                    .bytes()
                    .all(|byte| byte.is_ascii_digit() || (b'a'..=b'v').contains(&byte))
                || source_version.trim().is_empty()
                || source_version.len() > 512
                || source_version.chars().any(char::is_control)
                || occurrence_id.is_some()
                || *whole_series
            {
                Err(ContractError::InvalidEvent)
            } else {
                Ok(())
            }
        }
        ProposedAction::RespondToEvent { event_id, .. }
            if event_id.trim().is_empty() || event_id.len() > 512 =>
        {
            Err(ContractError::InvalidEvent)
        }
        ProposedAction::RespondToEvent { .. } => Ok(()),
    }
}

fn hex_digest(bytes: &[u8]) -> String {
    let digest = Sha256::digest(bytes);
    let mut value = String::with_capacity(digest.len() * 2);
    for byte in digest {
        use std::fmt::Write as _;
        let _ = write!(value, "{byte:02x}");
    }
    value
}

/// An implementation operates on typed values only. The host broker resolves
/// and injects credentials, pins the provider endpoint, and authorizes every
/// method independently; this trait alone conveys no authority.
#[async_trait]
pub trait ProviderAdapter: Send + Sync {
    fn provider(&self) -> Provider;

    async fn list_messages(&self, request: ReadRequest) -> Result<Vec<MailMessage>, ProviderError>;

    async fn list_events(&self, request: ReadRequest) -> Result<Vec<CalendarEvent>, ProviderError>;

    async fn free_busy(&self, request: ReadRequest) -> Result<Vec<FreeBusySlot>, ProviderError>;

    async fn commit(
        &self,
        candidate: ActionCandidate,
        approval: CandidateApproval,
    ) -> Result<ActionReceipt, ProviderError>;
}

#[derive(Debug, thiserror::Error)]
pub enum ProviderError {
    #[error("account authorization is required")]
    ReauthorizationRequired,
    #[error("the connected account does not permit this operation")]
    CapabilityDenied,
    #[error("the provider item changed; review the updated item")]
    Conflict,
    #[error("the provider outcome is unknown and must be reconciled")]
    OutcomeUnknown,
    #[error("the provider request failed: {code}")]
    Failed { code: String },
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod contract_tests {
    use super::*;

    #[test]
    fn routine_folder_scope_defaults_for_saved_legacy_tasks_and_rejects_unsafe_watches() {
        let account_id = Uuid::now_v7().to_string();
        let routine_id = Uuid::now_v7().to_string();
        let legacy: RoutineScope = serde_json::from_value(serde_json::json!({
            "routine_id": routine_id,
            "account_id": account_id,
            "operations": ["recent_mail"],
            "max_items": 5,
            "watch_new_mail": true
        }))
        .unwrap();
        assert_eq!(legacy.mail_folder_id, None);
        assert_eq!(legacy.calendar_event_trigger, None);
        assert!(!legacy.read_commitments);
        assert!(legacy.validate().is_ok());

        let mut invalid_watch = legacy;
        invalid_watch.mail_folder_id = Some("SENT".into());
        assert_eq!(
            invalid_watch.validate(),
            Err(ContractError::InvalidRoutineScope)
        );
        let mut invalid_folder = invalid_watch.clone();
        invalid_folder.mail_folder_id = Some("\nInjected".into());
        assert_eq!(
            invalid_folder.validate(),
            Err(ContractError::InvalidRoutineScope)
        );
    }

    #[test]
    fn routine_calendar_source_defaults_for_legacy_and_requires_calendar_read_scope() {
        let account_id = Uuid::now_v7().to_string();
        let routine_id = Uuid::now_v7().to_string();
        let mut scope: RoutineScope = serde_json::from_value(serde_json::json!({
            "routine_id": routine_id,
            "account_id": account_id,
            "operations": ["calendar_events"],
            "max_items": 5,
            "watch_new_mail": false
        }))
        .unwrap();
        assert_eq!(scope.calendar_source_id, None);
        assert!(scope.validate().is_ok());
        scope.calendar_source_id = Some("calendar-id".into());
        assert!(scope.validate().is_ok());
        scope.operations = [RoutineOperation::RecentMail].into_iter().collect();
        assert_eq!(scope.validate(), Err(ContractError::InvalidRoutineScope));
        scope.operations = [RoutineOperation::CalendarEvents].into_iter().collect();
        scope.calendar_source_id = Some("bad\nsource".into());
        assert_eq!(scope.validate(), Err(ContractError::InvalidRoutineScope));
        scope.calendar_source_id = Some("x".repeat(2049));
        assert_eq!(scope.validate(), Err(ContractError::InvalidRoutineScope));
    }

    #[test]
    fn calendar_event_trigger_contract_validates_scope_and_offset_windows() {
        let trigger = CalendarEventTrigger {
            boundary: CalendarEventBoundary::Start,
            offset_minutes: 15,
            max_lateness_minutes: 10,
        };
        assert_eq!(serde_json::to_value(trigger).unwrap()["boundary"], "start");
        let mut scope: RoutineScope = serde_json::from_value(serde_json::json!({
            "routine_id": Uuid::now_v7().to_string(),
            "account_id": Uuid::now_v7().to_string(),
            "operations": ["calendar_events"],
            "max_items": 5,
            "watch_new_mail": false,
            "calendar_event_trigger": trigger
        }))
        .unwrap();
        assert!(scope.validate().is_ok());

        scope.operations.clear();
        scope.operations.insert(RoutineOperation::RecentMail);
        assert_eq!(scope.validate(), Err(ContractError::InvalidRoutineScope));

        scope.operations = [RoutineOperation::CalendarEvents].into_iter().collect();
        scope.watch_new_mail = true;
        assert_eq!(scope.validate(), Err(ContractError::InvalidRoutineScope));

        scope.watch_new_mail = false;
        scope.calendar_event_trigger = Some(CalendarEventTrigger {
            offset_minutes: 10_081,
            ..trigger
        });
        assert_eq!(scope.validate(), Err(ContractError::InvalidRoutineScope));
        scope.calendar_event_trigger = Some(CalendarEventTrigger {
            max_lateness_minutes: 0,
            ..trigger
        });
        assert_eq!(scope.validate(), Err(ContractError::InvalidRoutineScope));
    }

    #[test]
    fn calendar_event_trigger_applies_before_after_and_bounded_catch_up() {
        let starts_at = DateTime::parse_from_rfc3339("2026-10-02T10:00:00Z")
            .unwrap()
            .with_timezone(&Utc);
        let ends_at = DateTime::parse_from_rfc3339("2026-10-02T11:00:00Z")
            .unwrap()
            .with_timezone(&Utc);
        let before = CalendarEventTrigger {
            boundary: CalendarEventBoundary::Start,
            offset_minutes: 15,
            max_lateness_minutes: 10,
        };
        let due_at = DateTime::parse_from_rfc3339("2026-10-02T09:45:00Z")
            .unwrap()
            .with_timezone(&Utc);
        assert_eq!(before.trigger_at(starts_at, ends_at), due_at);
        assert!(before.is_due_between(
            starts_at,
            ends_at,
            due_at - chrono::Duration::seconds(1),
            due_at + chrono::Duration::minutes(3),
        ));
        assert!(!before.is_due_between(
            starts_at,
            ends_at,
            due_at,
            due_at + chrono::Duration::minutes(3),
        ));
        assert!(!before.is_due_between(
            starts_at,
            ends_at,
            due_at - chrono::Duration::minutes(30),
            due_at + chrono::Duration::minutes(11),
        ));

        let after = CalendarEventTrigger {
            boundary: CalendarEventBoundary::End,
            offset_minutes: -5,
            max_lateness_minutes: 10,
        };
        assert_eq!(
            after.trigger_at(starts_at, ends_at),
            ends_at + chrono::Duration::minutes(5)
        );
    }

    #[test]
    fn calendar_event_occurrence_key_is_opaque_stable_and_move_sensitive() {
        let starts_at = Utc::now();
        let ends_at = starts_at + chrono::Duration::hours(1);
        let trigger = CalendarEventTrigger {
            boundary: CalendarEventBoundary::Start,
            offset_minutes: 15,
            max_lateness_minutes: 10,
        };
        let first = trigger.occurrence_key("opaque-provider-id", starts_at, ends_at);
        assert_eq!(
            first,
            trigger.occurrence_key("opaque-provider-id", starts_at, ends_at)
        );
        assert!(first.starts_with("calendar:"));
        assert!(!first.contains("opaque-provider-id"));
        assert_ne!(
            first,
            trigger.occurrence_key(
                "opaque-provider-id",
                starts_at + chrono::Duration::minutes(1),
                ends_at + chrono::Duration::minutes(1),
            )
        );
        assert_ne!(
            first,
            CalendarEventTrigger {
                boundary: CalendarEventBoundary::End,
                ..trigger
            }
            .occurrence_key("opaque-provider-id", starts_at, ends_at)
        );
    }

    fn candidate() -> ActionCandidate {
        ActionCandidate::new(
            "account-a".into(),
            "agent-a".into(),
            "conversation-a".into(),
            vec![SourceRef {
                item_id: "message-1".into(),
                version: Some("etag-1".into()),
                label: Some("Selected thread".into()),
            }],
            ProposedAction::SendMail {
                draft: MailDraft {
                    from_alias: None,
                    to: vec![MailAddress {
                        address: "reader@example.com".into(),
                        display_name: None,
                    }],
                    cc: Vec::new(),
                    bcc: Vec::new(),
                    subject: "Reviewed reply".into(),
                    body_text: "Hello".into(),
                    attachment_refs: Vec::new(),
                    reply_to_message_id: None,
                    reply_to_thread_id: None,
                },
            },
        )
        .unwrap()
    }

    #[test]
    fn unassigned_local_drafts_are_revisionable_but_not_effect_candidates() {
        let draft = ProposedAction::SendMail {
            draft: MailDraft {
                from_alias: None,
                to: vec![MailAddress {
                    address: "reader@example.com".into(),
                    display_name: None,
                }],
                cc: Vec::new(),
                bcc: Vec::new(),
                subject: "Local draft".into(),
                body_text: "Saved locally".into(),
                attachment_refs: Vec::new(),
                reply_to_message_id: None,
                reply_to_thread_id: None,
            },
        };
        let mut candidate = ActionCandidate::new(
            LOCAL_DRAFT_ACCOUNT_ID.into(),
            "agent-a".into(),
            "agent:agent-a".into(),
            Vec::new(),
            draft.clone(),
        )
        .unwrap();
        assert!(candidate.validate().is_ok());
        assert_eq!(candidate.account_id, LOCAL_DRAFT_ACCOUNT_ID);
        let connected_fake = ConnectedAccount {
            id: LOCAL_DRAFT_ACCOUNT_ID.into(),
            provider: Provider::Google,
            status: AccountStatus::Connected,
            owner_agent_id: "agent-a".into(),
            allowed_audiences: ["agent:agent-a".into()].into_iter().collect(),
            capabilities: [Capability::MailSend].into_iter().collect(),
            provider_scopes: BTreeSet::new(),
            credential_ref: "opaque".into(),
            principal_ref: "opaque".into(),
            revision: 1,
            connected_at: Utc::now(),
            access_token_expires_at: None,
            refresh_token_available: false,
            revoked_at: None,
        };
        let now = Utc::now();
        let approval = CandidateApproval {
            candidate_id: candidate.id.clone(),
            candidate_digest: candidate.digest().unwrap(),
            account_id: LOCAL_DRAFT_ACCOUNT_ID.into(),
            agent_id: "agent-a".into(),
            audience_id: "agent:agent-a".into(),
            approved_at: now,
            expires_at: now + chrono::Duration::minutes(1),
            single_use: true,
        };
        assert_eq!(
            candidate.authorize_effect(&connected_fake, &approval, now),
            Err(ContractError::AccountDenied)
        );
        assert_eq!(
            ActionCandidate::new(
                LOCAL_DRAFT_ACCOUNT_ID.into(),
                "agent-a".into(),
                "agent:agent-a".into(),
                vec![SourceRef {
                    item_id: "provider-message".into(),
                    version: None,
                    label: None,
                }],
                draft.clone(),
            ),
            Err(ContractError::AccountDenied)
        );
        let reply = ProposedAction::SendMail {
            draft: MailDraft {
                reply_to_message_id: Some("provider-message".into()),
                reply_to_thread_id: Some("provider-thread".into()),
                ..match draft {
                    ProposedAction::SendMail { draft } => draft,
                    _ => unreachable!(),
                }
            },
        };
        assert_eq!(
            ActionCandidate::new(
                LOCAL_DRAFT_ACCOUNT_ID.into(),
                "agent-a".into(),
                "agent:agent-a".into(),
                Vec::new(),
                reply,
            ),
            Err(ContractError::AccountDenied)
        );
        candidate
            .revise(ProposedAction::CreateEvent {
                draft: CalendarDraft {
                    title: "Planning".into(),
                    description: String::new(),
                    location: None,
                    starts_at: Utc::now() + chrono::Duration::hours(1),
                    ends_at: Utc::now() + chrono::Duration::hours(2),
                    time_zone: "UTC".into(),
                    all_day: false,
                    attendee_addresses: Vec::new(),
                    recurrence: None,
                    occurrence_id: None,
                },
            })
            .unwrap();
        assert_eq!(candidate.revision, 2);
        assert!(candidate.validate().is_ok());
    }

    #[test]
    fn local_mail_drafts_may_be_incomplete_but_provider_candidates_may_not() {
        let draft = ProposedAction::SendMail {
            draft: MailDraft {
                from_alias: None,
                to: Vec::new(),
                cc: Vec::new(),
                bcc: Vec::new(),
                subject: String::new(),
                body_text: String::new(),
                attachment_refs: Vec::new(),
                reply_to_message_id: None,
                reply_to_thread_id: None,
            },
        };
        let local = ActionCandidate::new(
            LOCAL_DRAFT_ACCOUNT_ID.into(),
            "agent-a".into(),
            "agent:agent-a".into(),
            Vec::new(),
            draft.clone(),
        )
        .unwrap();
        assert!(local.validate().is_ok());
        assert_eq!(
            ActionCandidate::new(
                "provider-account".into(),
                "agent-a".into(),
                "agent:agent-a".into(),
                Vec::new(),
                draft,
            ),
            Err(ContractError::InvalidRecipients)
        );
    }

    fn approval(candidate: &ActionCandidate) -> CandidateApproval {
        let now = Utc::now();
        CandidateApproval {
            candidate_id: candidate.id.clone(),
            candidate_digest: candidate.digest().unwrap(),
            account_id: candidate.account_id.clone(),
            agent_id: candidate.agent_id.clone(),
            audience_id: candidate.audience_id.clone(),
            approved_at: now,
            expires_at: now + chrono::Duration::minutes(5),
            single_use: true,
        }
    }

    #[test]
    fn approval_is_bound_to_the_exact_candidate_revision_and_scope() {
        let mut candidate = candidate();
        let approval = approval(&candidate);
        assert!(approval.matches(&candidate, Utc::now()).unwrap());

        candidate
            .revise(ProposedAction::SendMail {
                draft: MailDraft {
                    from_alias: None,
                    to: vec![MailAddress {
                        address: "reader@example.com".into(),
                        display_name: None,
                    }],
                    cc: Vec::new(),
                    bcc: Vec::new(),
                    subject: "Changed after review".into(),
                    body_text: "Hello".into(),
                    attachment_refs: Vec::new(),
                    reply_to_message_id: None,
                    reply_to_thread_id: None,
                },
            })
            .unwrap();
        assert!(!approval.matches(&candidate, Utc::now()).unwrap());
    }

    #[test]
    fn approval_rejects_wrong_scope_expiry_and_non_single_use_grants() {
        let candidate = candidate();
        let mut approval = approval(&candidate);
        approval.audience_id = "another-audience".into();
        assert!(!approval.matches(&candidate, Utc::now()).unwrap());
        approval.audience_id = candidate.audience_id.clone();
        approval.approved_at = Utc::now() + chrono::Duration::seconds(1);
        assert!(!approval.matches(&candidate, Utc::now()).unwrap());
        approval.approved_at = Utc::now() - chrono::Duration::minutes(1);
        approval.expires_at = Utc::now() - chrono::Duration::seconds(1);
        assert!(!approval.matches(&candidate, Utc::now()).unwrap());
        approval.expires_at = Utc::now() + chrono::Duration::minutes(5);
        approval.single_use = false;
        assert!(!approval.matches(&candidate, Utc::now()).unwrap());
        approval.single_use = true;
        approval.expires_at = approval.approved_at + chrono::Duration::minutes(6);
        assert!(!approval.matches(&candidate, Utc::now()).unwrap());
    }

    #[test]
    fn candidate_rejects_controlled_or_unbounded_source_metadata() {
        let mut candidate = candidate();
        candidate.source_refs[0].item_id = "message-1\nInjected".into();
        assert_eq!(candidate.source_refs.len(), 1);
        let result = ActionCandidate::new(
            candidate.account_id,
            candidate.agent_id,
            candidate.audience_id,
            candidate.source_refs,
            candidate.action,
        );
        assert_eq!(result.unwrap_err(), ContractError::InvalidSource);
    }

    #[test]
    fn account_admission_is_agent_audience_and_capability_scoped() {
        let now = Utc::now();
        let account = ConnectedAccount {
            id: "account-a".into(),
            provider: Provider::Google,
            status: AccountStatus::Connected,
            owner_agent_id: "agent-a".into(),
            allowed_audiences: ["conversation-a".into()].into_iter().collect(),
            capabilities: [Capability::MailRead].into_iter().collect(),
            provider_scopes: ["mail.read".into()].into_iter().collect(),
            credential_ref: "opaque-vault-ref".into(),
            principal_ref: "opaque-principal-ref".into(),
            revision: 1,
            connected_at: now,
            access_token_expires_at: Some(now),
            refresh_token_available: false,
            revoked_at: None,
        };
        assert!(account.admits("agent-a", "conversation-a", Capability::MailRead));
        assert!(!account.admits("agent-b", "conversation-a", Capability::MailRead));
        assert!(!account.admits("agent-a", "conversation-b", Capability::MailRead));
        assert!(!account.admits("agent-a", "conversation-a", Capability::MailSend));

        let mut unverified_icloud = account.clone();
        unverified_icloud.provider = Provider::AppleIcloud;
        unverified_icloud.status = AccountStatus::ConnectedUnverified;
        assert!(!unverified_icloud.admits("agent-a", "conversation-a", Capability::MailRead));

        let mut verified_icloud_mail = unverified_icloud.clone();
        verified_icloud_mail.status = AccountStatus::Connected;
        assert!(verified_icloud_mail.admits("agent-a", "conversation-a", Capability::MailRead));
        assert!(!verified_icloud_mail.admits(
            "agent-a",
            "conversation-a",
            Capability::CalendarRead
        ));

        let mut pending = account;
        pending.status = AccountStatus::Pending;
        assert!(!pending.admits("agent-a", "conversation-a", Capability::MailRead));
    }

    #[test]
    fn connected_account_schema_rejects_unexpected_secret_fields() {
        let now = Utc::now();
        let account = ConnectedAccount {
            id: Uuid::now_v7().to_string(),
            provider: Provider::Google,
            status: AccountStatus::Connected,
            owner_agent_id: "agent-a".into(),
            allowed_audiences: ["agent:agent-a".into()].into_iter().collect(),
            capabilities: [Capability::MailRead].into_iter().collect(),
            provider_scopes: ["mail.read".into()].into_iter().collect(),
            credential_ref: "opaque-vault-ref".into(),
            principal_ref: "opaque-principal-ref".into(),
            revision: 1,
            connected_at: now,
            access_token_expires_at: None,
            refresh_token_available: false,
            revoked_at: None,
        };
        let mut encoded = serde_json::to_value(account).unwrap();
        encoded.as_object_mut().unwrap().insert(
            "access_token".into(),
            serde_json::json!("must-not-be-metadata"),
        );

        assert!(serde_json::from_value::<ConnectedAccount>(encoded).is_err());
    }

    #[test]
    fn effect_authorization_requires_account_capability_and_exact_review() {
        let candidate = candidate();
        let approval = approval(&candidate);
        let now = Utc::now();
        let account = ConnectedAccount {
            id: candidate.account_id.clone(),
            provider: Provider::Google,
            status: AccountStatus::Connected,
            owner_agent_id: candidate.agent_id.clone(),
            allowed_audiences: [candidate.audience_id.clone()].into_iter().collect(),
            capabilities: [Capability::MailSend].into_iter().collect(),
            provider_scopes: ["mail.send".into()].into_iter().collect(),
            credential_ref: "opaque-vault-ref".into(),
            principal_ref: "opaque-principal-ref".into(),
            revision: 1,
            connected_at: now,
            access_token_expires_at: Some(now),
            refresh_token_available: false,
            revoked_at: None,
        };
        assert_eq!(candidate.authorize_effect(&account, &approval, now), Ok(()));

        let mut unverified_icloud = account.clone();
        unverified_icloud.provider = Provider::AppleIcloud;
        unverified_icloud.status = AccountStatus::ConnectedUnverified;
        assert_eq!(
            candidate.authorize_effect(&unverified_icloud, &approval, now),
            Err(ContractError::AccountDenied)
        );

        let mut wrong_account = account.clone();
        wrong_account.id = "other-account".into();
        assert_eq!(
            candidate.authorize_effect(&wrong_account, &approval, now),
            Err(ContractError::AccountDenied)
        );
        let mut revoked = account.clone();
        revoked.revoked_at = Some(now);
        assert_eq!(
            candidate.authorize_effect(&revoked, &approval, now),
            Err(ContractError::AccountDenied)
        );
        let mut replayed = approval;
        replayed.single_use = false;
        assert_eq!(
            candidate.authorize_effect(&account, &replayed, now),
            Err(ContractError::ApprovalRequired)
        );
    }

    #[test]
    fn candidate_revision_overflow_fails_without_mutating_the_action() {
        let mut candidate = candidate();
        candidate.revision = u64::MAX;
        let original = candidate.action.clone();
        let replacement = ProposedAction::SendMail {
            draft: MailDraft {
                from_alias: None,
                to: vec![MailAddress {
                    address: "reader@example.com".into(),
                    display_name: None,
                }],
                cc: Vec::new(),
                bcc: Vec::new(),
                subject: "Changed".into(),
                body_text: "Hello".into(),
                attachment_refs: Vec::new(),
                reply_to_message_id: None,
                reply_to_thread_id: None,
            },
        };
        assert_eq!(
            candidate.revise(replacement),
            Err(ContractError::LimitExceeded)
        );
        assert_eq!(candidate.action, original);
    }
}
