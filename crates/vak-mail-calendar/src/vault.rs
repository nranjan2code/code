//! Agent-scoped mail/calendar secrets backed by Vak's credential vault.
//!
//! This module never writes credential material to the connection ledger or
//! filesystem. The credential service chooses the native OS vault or its
//! encrypted-file fallback. Secret values are not `Debug` and are zeroized
//! when dropped.

use crate::{ActionCandidate, ActionReceipt, ActionState};
use fs2::FileExt;
use serde::{Deserialize, Serialize};
use std::fs::{self, File, OpenOptions};
use std::path::PathBuf;
use uuid::Uuid;
use zeroize::{Zeroize, Zeroizing};

const MAX_SECRET_BYTES: usize = 256 * 1024;
const MAX_PRINCIPAL_BYTES: usize = 512;
const WORK_AREA_KEY: &str = "vak_mail_calendar_work_area";
const MAX_WORK_AREA_BYTES: usize = 2 * 1024 * 1024;
const MAX_WORK_AREA_CANDIDATES: usize = 32;
const ROUTINE_CURSOR_KEY: &str = "vak_mail_calendar_routine_cursors";
const ROUTINE_HISTORY_KEY: &str = "vak_mail_calendar_routine_history";
const MAX_ROUTINE_HISTORY: usize = 256;
const MAX_ROUTINE_HISTORY_BYTES: usize = 256 * 1024;
const MAX_ROUTINE_CURSORS: usize = 128;
const MAX_ROUTINE_SEEN_IDS: usize = 512;
const MAX_ROUTINE_PENDING_IDS: usize = crate::MAX_ROUTINE_MAIL_BACKLOG;
const MAX_PROVIDER_CURSOR_BYTES: usize = 8192;
const MAX_ROUTINE_CURSOR_BYTES: usize = 512 * 1024;
const ACTION_RECEIPTS_KEY: &str = "vak_mail_calendar_action_receipts";
const MAX_ACTION_RECEIPTS: usize = 128;
const MAX_ACTION_RECEIPTS_BYTES: usize = 512 * 1024;

#[derive(Debug, thiserror::Error)]
pub enum VaultError {
    #[error("mail and calendar vault operation failed")]
    Store(#[from] std::io::Error),
    #[error("mail and calendar vault reference is invalid")]
    InvalidReference,
    #[error("mail and calendar vault data is unavailable or invalid")]
    Unavailable,
    #[error("mail and calendar vault data exceeded its size limit")]
    TooLarge,
    #[error("mail and calendar candidate revision changed")]
    Conflict,
}

/// Cross-process exclusive claim for one Agent's mail/calendar routine.
/// Keep this value alive for the full run; closing the file releases the OS
/// advisory lock, including when the process exits or crashes.
pub struct RoutineLease {
    _lock_file: File,
}

/// Cross-process exclusive claim for one Agent account's OAuth rotation.
/// Keep it alive until the rotated secret and ledger metadata are committed.
pub struct AccountRefreshLease {
    _lock_file: File,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RoutineRunTrigger {
    Manual,
    Scheduled,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RoutineRunStatus {
    Running,
    Complete,
    Failed,
    NoChanges,
    Interrupted,
}

/// Content-free execution history for one Agent-owned routine. Source data
/// and model output stay in the Agent session; this record contains only
/// lifecycle metadata needed to review past runs.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RoutineRunRecord {
    pub run_id: String,
    pub routine_id: String,
    pub account_id: String,
    pub session_id: Option<String>,
    pub trigger: RoutineRunTrigger,
    pub status: RoutineRunStatus,
    pub started_at: chrono::DateTime<chrono::Utc>,
    pub finished_at: Option<chrono::DateTime<chrono::Utc>>,
    /// Count of provider items actually returned by brokered reads, excluding
    /// content. Older encrypted records default to zero.
    #[serde(default)]
    pub items_returned: u8,
}

/// Secret material accepted from a provider setup flow. Keep this type out of
/// logs and serialized APIs; the vault's encrypted storage is the only
/// persistence boundary.
pub struct AccountSecretMaterial {
    principal: String,
    display_identity: Option<String>,
    client_id: Option<String>,
    access_token: Option<String>,
    refresh_token: Option<String>,
    app_login: Option<String>,
    app_password: Option<String>,
}

impl AccountSecretMaterial {
    pub fn uses_app_password(&self) -> bool {
        self.app_login.is_some() && self.app_password.is_some()
    }

    pub fn new(
        mut principal: String,
        mut display_identity: Option<String>,
        mut client_id: Option<String>,
        mut access_token: Option<String>,
        mut refresh_token: Option<String>,
        mut app_login: Option<String>,
        mut app_password: Option<String>,
    ) -> Result<Self, VaultError> {
        let total = principal.len()
            + display_identity.as_ref().map_or(0, String::len)
            + client_id.as_ref().map_or(0, String::len)
            + access_token.as_ref().map_or(0, String::len)
            + refresh_token.as_ref().map_or(0, String::len)
            + app_login.as_ref().map_or(0, String::len)
            + app_password.as_ref().map_or(0, String::len);
        if principal.trim().is_empty()
            || principal.len() > MAX_PRINCIPAL_BYTES
            || principal.chars().any(char::is_control)
            || display_identity.as_ref().is_some_and(|value| {
                value.trim().is_empty() || value.len() > 320 || value.chars().any(char::is_control)
            })
            || client_id.as_ref().is_some_and(|value| {
                value.trim().is_empty() || value.len() > 512 || value.chars().any(char::is_control)
            })
            || access_token.as_ref().is_some_and(|value| {
                value.is_empty() || value.len() > 32 * 1024 || value.chars().any(char::is_control)
            })
            || refresh_token.as_ref().is_some_and(|value| {
                value.is_empty() || value.len() > 32 * 1024 || value.chars().any(char::is_control)
            })
            || app_login.as_ref().is_some_and(|value| {
                value.trim().is_empty() || value.len() > 320 || value.chars().any(char::is_control)
            })
            || app_password.as_ref().is_some_and(|value| {
                value.is_empty() || value.len() > 256 || value.chars().any(char::is_control)
            })
        {
            principal.zeroize();
            display_identity.zeroize();
            client_id.zeroize();
            access_token.zeroize();
            refresh_token.zeroize();
            app_login.zeroize();
            app_password.zeroize();
            return Err(VaultError::InvalidReference);
        }
        if total > MAX_SECRET_BYTES {
            principal.zeroize();
            display_identity.zeroize();
            client_id.zeroize();
            access_token.zeroize();
            refresh_token.zeroize();
            app_login.zeroize();
            app_password.zeroize();
            return Err(VaultError::TooLarge);
        }
        Ok(Self {
            principal,
            display_identity,
            client_id,
            access_token,
            refresh_token,
            app_login,
            app_password,
        })
    }

    /// Returns a display-safe email hint; the full identity remains vaulted.
    pub fn masked_display_identity(&self) -> Option<String> {
        let identity = self.display_identity.as_deref()?;
        let (local, domain) = identity.split_once('@')?;
        let first = local.chars().next()?;
        if domain.is_empty() || domain.contains('@') {
            return None;
        }
        Some(format!("{first}***@{domain}"))
    }

    /// Match an owner-provided account address without returning or logging
    /// the complete vaulted identity. Callers must not pass provider content.
    pub fn display_identity_matches(&self, candidate: &str) -> bool {
        if candidate.chars().any(char::is_control) {
            return false;
        }
        let candidate = candidate.trim();
        !candidate.is_empty()
            && self
                .display_identity
                .as_deref()
                .is_some_and(|identity| identity.trim().eq_ignore_ascii_case(candidate))
    }

    /// Compare vaulted provider subject identifiers without exposing them to
    /// callers or serializing them into the account ledger.
    pub fn has_same_principal(&self, other: &Self) -> bool {
        self.principal == other.principal
    }

    /// iCloud's current local enrollment uses an email hint rather than a
    /// provider-issued stable subject, so compare it case-insensitively.
    pub fn has_same_principal_ignoring_ascii_case(&self, other: &Self) -> bool {
        self.principal.eq_ignore_ascii_case(&other.principal)
    }

    /// Compare account identities when one side uses a provider app password
    /// whose principal is an email address and the other uses OAuth's opaque
    /// subject identifier. The full identity remains vault-owned.
    pub fn has_same_display_identity_ignoring_ascii_case(&self, other: &Self) -> bool {
        self.display_identity
            .as_deref()
            .zip(other.display_identity.as_deref())
            .is_some_and(|(left, right)| left.eq_ignore_ascii_case(right))
    }

    pub fn icloud_imap_credentials(
        &self,
    ) -> Result<(Zeroizing<String>, Zeroizing<String>), VaultError> {
        let login = self.app_login.clone().ok_or(VaultError::Unavailable)?;
        let password = self.app_password.clone().ok_or(VaultError::Unavailable)?;
        Ok((Zeroizing::new(login), Zeroizing::new(password)))
    }
}

impl Drop for AccountSecretMaterial {
    fn drop(&mut self) {
        self.principal.zeroize();
        self.display_identity.zeroize();
        self.client_id.zeroize();
        self.access_token.zeroize();
        self.refresh_token.zeroize();
        self.app_login.zeroize();
        self.app_password.zeroize();
    }
}

#[derive(Serialize, Deserialize)]
struct VaultPayload {
    principal: String,
    #[serde(default)]
    display_identity: Option<String>,
    #[serde(default)]
    client_id: Option<String>,
    access_token: Option<String>,
    refresh_token: Option<String>,
    app_login: Option<String>,
    app_password: Option<String>,
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct RoutineCursor {
    routine_id: String,
    account_id: String,
    seen_ids: Vec<String>,
    #[serde(default)]
    pending_ids: Vec<String>,
    #[serde(default)]
    delivered_ids: Vec<String>,
    #[serde(default)]
    seen_calendar_occurrences: Vec<String>,
    #[serde(default)]
    pending_calendar_occurrences: Vec<String>,
    #[serde(default)]
    delivered_calendar_occurrences: Vec<String>,
    #[serde(default)]
    provider_cursor: Option<String>,
}

impl Drop for VaultPayload {
    fn drop(&mut self) {
        self.principal.zeroize();
        self.display_identity.zeroize();
        self.client_id.zeroize();
        self.access_token.zeroize();
        self.refresh_token.zeroize();
        self.app_login.zeroize();
        self.app_password.zeroize();
    }
}

/// A credential vault whose scope is fixed to one authenticated Agent.
#[derive(Clone)]
pub struct AccountVault {
    agent_id: String,
    scope_hint: PathBuf,
}

impl AccountVault {
    pub fn for_agent(agent_id: &str) -> Result<Self, VaultError> {
        if !valid_agent_id(agent_id) {
            return Err(VaultError::InvalidReference);
        }
        let agent_home = vak_config::paths::agent_home(agent_id);
        let agents_dir = agent_home.parent().ok_or(VaultError::InvalidReference)?;
        let data_home = agents_dir.parent().ok_or(VaultError::InvalidReference)?;
        ensure_agent_directory(data_home, false)?;
        ensure_agent_directory(agents_dir, true)?;
        ensure_agent_directory(agent_home.as_path(), true)?;
        // Credential scope ids are derived from a canonicalized directory.
        // Creating the Agent home here keeps that scope stable when the
        // connection ledger is first appended afterwards.
        let agent_home = std::fs::canonicalize(agent_home).map_err(VaultError::Store)?;
        Ok(Self {
            agent_id: agent_id.to_owned(),
            scope_hint: agent_home.join(".env"),
        })
    }

    /// The returned reference is derived from a validated UUIDv7 and carries
    /// no credential or principal data.
    pub fn credential_ref(account_id: &str) -> Result<String, VaultError> {
        validate_account_id(account_id)?;
        Ok(format!("vak_mail_calendar_{account_id}"))
    }

    /// Try to claim one routine run across server processes. `None` means
    /// another process currently owns the run; errors fail closed.
    pub fn try_acquire_routine_lease(
        &self,
        routine_id: &str,
    ) -> Result<Option<RoutineLease>, VaultError> {
        validate_account_id(routine_id)?;
        let agent_home = self
            .scope_hint
            .parent()
            .ok_or(VaultError::InvalidReference)?;
        let work_dir = agent_home.join("mail-calendar");
        ensure_agent_directory(&work_dir, true)?;
        let lock_path = work_dir.join(format!(".routine-{routine_id}.lease"));
        if let Ok(metadata) = fs::symlink_metadata(&lock_path)
            && (metadata.file_type().is_symlink() || !metadata.is_file())
        {
            return Err(VaultError::InvalidReference);
        }
        let mut options = OpenOptions::new();
        options.create(true).truncate(false).read(true).write(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(0o600).custom_flags(libc::O_NOFOLLOW);
        }
        let file = options.open(lock_path).map_err(VaultError::Store)?;
        match file.try_lock_exclusive() {
            Ok(()) => Ok(Some(RoutineLease { _lock_file: file })),
            Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => Ok(None),
            Err(error) => Err(VaultError::Store(error)),
        }
    }

    /// Prevent separate local Vakyartha processes from refreshing one
    /// provider account concurrently while its refresh token may rotate.
    pub fn try_acquire_account_refresh_lease(
        &self,
        account_id: &str,
    ) -> Result<Option<AccountRefreshLease>, VaultError> {
        validate_account_id(account_id)?;
        let agent_home = self
            .scope_hint
            .parent()
            .ok_or(VaultError::InvalidReference)?;
        let work_dir = agent_home.join("mail-calendar");
        ensure_agent_directory(&work_dir, true)?;
        let lock_path = work_dir.join(format!(".account-refresh-{account_id}.lease"));
        if let Ok(metadata) = fs::symlink_metadata(&lock_path)
            && (metadata.file_type().is_symlink() || !metadata.is_file())
        {
            return Err(VaultError::InvalidReference);
        }
        let mut options = OpenOptions::new();
        options.create(true).truncate(false).read(true).write(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(0o600).custom_flags(libc::O_NOFOLLOW);
        }
        let file = options.open(lock_path).map_err(VaultError::Store)?;
        match file.try_lock_exclusive() {
            Ok(()) => Ok(Some(AccountRefreshLease { _lock_file: file })),
            Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => Ok(None),
            Err(error) => Err(VaultError::Store(error)),
        }
    }

    pub fn store(
        &self,
        account_id: &str,
        material: AccountSecretMaterial,
    ) -> Result<(), VaultError> {
        let key = Self::credential_ref(account_id)?;
        let payload = VaultPayload {
            principal: material.principal.clone(),
            display_identity: material.display_identity.clone(),
            client_id: material.client_id.clone(),
            access_token: material.access_token.clone(),
            refresh_token: material.refresh_token.clone(),
            app_login: material.app_login.clone(),
            app_password: material.app_password.clone(),
        };
        let encoded =
            Zeroizing::new(serde_json::to_string(&payload).map_err(|_| VaultError::Unavailable)?);
        if encoded.len() > MAX_SECRET_BYTES {
            return Err(VaultError::TooLarge);
        }
        vak_config::upsert_env_file(&self.scope_hint, &key, &encoded)?;
        Ok(())
    }

    /// Resolves only within this vault's Agent scope. The caller must still
    /// authorize the operation and recipient before injecting a credential.
    pub fn load(&self, account_id: &str) -> Result<AccountSecretMaterial, VaultError> {
        let key = Self::credential_ref(account_id)?;
        let encoded = Zeroizing::new(
            vak_config::read_env_file_var(&self.scope_hint, &key).ok_or(VaultError::Unavailable)?,
        );
        if encoded.len() > MAX_SECRET_BYTES {
            return Err(VaultError::TooLarge);
        }
        let payload: VaultPayload =
            serde_json::from_str(&encoded).map_err(|_| VaultError::Unavailable)?;
        AccountSecretMaterial::new(
            payload.principal.clone(),
            payload.display_identity.clone(),
            payload.client_id.clone(),
            payload.access_token.clone(),
            payload.refresh_token.clone(),
            payload.app_login.clone(),
            payload.app_password.clone(),
        )
    }

    /// Check only that a credential envelope can be loaded for admission;
    /// callers never receive any field from the decoded secret material.
    pub fn credential_available(&self, account_id: &str) -> bool {
        self.load(account_id).is_ok()
    }

    /// Removes all account credential material from the Agent's credential
    /// scope and its unsent local candidates. Safe to retry after partial
    /// disconnects. Append-only session copies cannot be removed here.
    pub fn remove(&self, account_id: &str) -> Result<(), VaultError> {
        let key = Self::credential_ref(account_id)?;
        vak_config::remove_env_file_key(&self.scope_hint, &key)?;
        self.remove_candidates_for_account(account_id)?;
        self.remove_routine_cursors_for_account(account_id)?;
        self.remove_action_receipts_for_account(account_id)?;
        self.remove_routine_history_for_account(account_id)
    }

    /// Persist a dispatch claim before any external effect. A candidate can
    /// be claimed only once, including after an ambiguous outcome; this
    /// intentionally prevents blind retries for non-idempotent provider APIs.
    pub fn begin_action(&self, candidate: &ActionCandidate) -> Result<ActionReceipt, VaultError> {
        candidate
            .validate()
            .map_err(|_| VaultError::InvalidReference)?;
        if candidate.agent_id != self.agent_id {
            return Err(VaultError::InvalidReference);
        }
        self.with_action_receipts(|mut receipts| {
            if receipts
                .iter()
                .any(|receipt| receipt.candidate_id == candidate.id)
            {
                return Err(VaultError::Conflict);
            }
            if receipts.len() >= MAX_ACTION_RECEIPTS {
                return Err(VaultError::TooLarge);
            }
            let receipt = ActionReceipt::new(candidate, ActionState::Dispatching)
                .map_err(|_| VaultError::InvalidReference)?;
            receipts.push(receipt.clone());
            self.write_action_receipts(&receipts)?;
            Ok(receipt)
        })
    }

    /// Record a provider result only against the exact durable claim and its
    /// candidate digest. Terminal failures and unknown outcomes are retained
    /// as non-retryable receipts.
    pub fn settle_action(&self, receipt: ActionReceipt) -> Result<(), VaultError> {
        let candidate_id_is_v7 =
            Uuid::parse_str(&receipt.candidate_id).is_ok_and(|id| id.get_version_num() == 7);
        let attempt_id_is_v7 =
            Uuid::parse_str(&receipt.attempt_id).is_ok_and(|id| id.get_version_num() == 7);
        if receipt.state == ActionState::Dispatching
            || receipt.logical_action_id != receipt.candidate_id
            || !candidate_id_is_v7
            || !attempt_id_is_v7
            || receipt.account_id.trim().is_empty()
            || receipt.candidate_digest.len() != 64
            || !receipt
                .candidate_digest
                .bytes()
                .all(|byte| byte.is_ascii_hexdigit())
        {
            return Err(VaultError::InvalidReference);
        }
        self.with_action_receipts(|mut receipts| {
            let Some(previous) = receipts
                .iter_mut()
                .find(|previous| previous.candidate_id == receipt.candidate_id)
            else {
                return Err(VaultError::InvalidReference);
            };
            if previous.candidate_digest != receipt.candidate_digest
                || previous.account_id != receipt.account_id
                || previous.attempt_id != receipt.attempt_id
                || !previous.state.can_transition_to(receipt.state)
            {
                return Err(VaultError::Conflict);
            }
            *previous = receipt;
            self.write_action_receipts(&receipts)
        })
    }

    pub fn list_action_receipts(&self) -> Result<Vec<ActionReceipt>, VaultError> {
        self.with_action_receipts(|receipts| Ok(receipts))
    }

    /// Queue observed message IDs without consuming them. The encrypted
    /// Agent vault provides a small durable backlog for bounded routine runs.
    pub fn queue_unseen_mail_ids(
        &self,
        routine_id: &str,
        account_id: &str,
        item_ids: &[String],
    ) -> Result<bool, VaultError> {
        self.queue_mail_ids_with_cursor(routine_id, account_id, item_ids, None)
    }

    /// Atomically persist IDs discovered through a provider cursor and the
    /// cursor position that follows them, so restart cannot skip a page.
    pub fn queue_mail_ids_with_cursor(
        &self,
        routine_id: &str,
        account_id: &str,
        item_ids: &[String],
        provider_cursor: Option<&str>,
    ) -> Result<bool, VaultError> {
        validate_routine_mail_ids(routine_id, account_id, item_ids)?;
        if provider_cursor.is_some_and(|cursor| {
            cursor.is_empty()
                || cursor.len() > MAX_PROVIDER_CURSOR_BYTES
                || cursor.chars().any(char::is_control)
        }) {
            return Err(VaultError::InvalidReference);
        }
        self.with_routine_cursors(|mut cursors| {
            let index = match cursors
                .iter()
                .position(|cursor| cursor.routine_id == routine_id)
            {
                Some(index) => {
                    if cursors[index].account_id != account_id {
                        return Err(VaultError::InvalidReference);
                    }
                    index
                }
                None => {
                    if cursors.len() >= MAX_ROUTINE_CURSORS {
                        return Err(VaultError::TooLarge);
                    }
                    cursors.push(RoutineCursor {
                        routine_id: routine_id.to_owned(),
                        account_id: account_id.to_owned(),
                        seen_ids: Vec::new(),
                        pending_ids: Vec::new(),
                        delivered_ids: Vec::new(),
                        seen_calendar_occurrences: Vec::new(),
                        pending_calendar_occurrences: Vec::new(),
                        delivered_calendar_occurrences: Vec::new(),
                        provider_cursor: None,
                    });
                    cursors.len() - 1
                }
            };
            let cursor = &mut cursors[index];
            if let Some(provider_cursor) = provider_cursor {
                cursor.provider_cursor = Some(provider_cursor.to_owned());
            }
            for item_id in item_ids {
                if cursor.seen_ids.iter().any(|seen| seen == item_id)
                    || cursor.pending_ids.iter().any(|pending| pending == item_id)
                    || cursor
                        .delivered_ids
                        .iter()
                        .any(|delivered| delivered == item_id)
                {
                    continue;
                }
                if cursor.pending_ids.len() + cursor.delivered_ids.len() >= MAX_ROUTINE_PENDING_IDS
                {
                    return Err(VaultError::TooLarge);
                }
                cursor.pending_ids.push(item_id.clone());
            }
            let has_pending = !cursor.pending_ids.is_empty();
            self.write_routine_cursors(&cursors)?;
            Ok(has_pending)
        })
    }

    /// Return the encrypted provider continuation token for one routine.
    pub fn routine_provider_cursor(
        &self,
        routine_id: &str,
        account_id: &str,
    ) -> Result<Option<String>, VaultError> {
        validate_account_id(account_id)?;
        validate_account_id(routine_id)?;
        self.with_routine_cursors(|cursors| {
            let Some(cursor) = cursors
                .iter()
                .find(|cursor| cursor.routine_id == routine_id)
            else {
                return Ok(None);
            };
            if cursor.account_id != account_id {
                return Err(VaultError::InvalidReference);
            }
            Ok(cursor.provider_cursor.clone())
        })
    }

    /// Return the next bounded batch from a routine's durable mail backlog.
    pub fn pending_mail_ids(
        &self,
        routine_id: &str,
        account_id: &str,
        limit: usize,
    ) -> Result<Vec<String>, VaultError> {
        validate_account_id(account_id)?;
        validate_account_id(routine_id)?;
        if !(1..=MAX_ROUTINE_PENDING_IDS).contains(&limit) {
            return Err(VaultError::InvalidReference);
        }
        self.with_routine_cursors(|cursors| {
            let Some(cursor) = cursors
                .iter()
                .find(|cursor| cursor.routine_id == routine_id)
            else {
                return Ok(Vec::new());
            };
            if cursor.account_id != account_id {
                return Err(VaultError::InvalidReference);
            }
            Ok(cursor.pending_ids.iter().take(limit).cloned().collect())
        })
    }

    /// Report whether any pending or staged mail exists for this routine.
    pub fn has_unresolved_mail_ids(
        &self,
        routine_id: &str,
        account_id: &str,
    ) -> Result<bool, VaultError> {
        validate_account_id(account_id)?;
        validate_account_id(routine_id)?;
        self.with_routine_cursors(|cursors| {
            let Some(cursor) = cursors
                .iter()
                .find(|cursor| cursor.routine_id == routine_id)
            else {
                return Ok(false);
            };
            if cursor.account_id != account_id {
                return Err(VaultError::InvalidReference);
            }
            Ok(!cursor.pending_ids.is_empty() || !cursor.delivered_ids.is_empty())
        })
    }

    /// Remove the cursor and queued provider IDs when a routine is deleted.
    pub fn remove_routine_cursor(
        &self,
        routine_id: &str,
        account_id: &str,
    ) -> Result<(), VaultError> {
        validate_account_id(account_id)?;
        validate_account_id(routine_id)?;
        self.with_routine_cursors(|mut cursors| {
            if cursors
                .iter()
                .find(|cursor| cursor.routine_id == routine_id)
                .is_some_and(|cursor| cursor.account_id != account_id)
            {
                return Err(VaultError::InvalidReference);
            }
            cursors.retain(|cursor| cursor.routine_id != routine_id);
            self.write_routine_cursors(&cursors)
        })
    }

    /// Stage successfully fetched IDs until the corresponding run settles.
    /// Keeping them in a separate list lets a restart retry an interrupted
    /// run instead of losing mail between provider fetch and session logging.
    pub fn stage_delivered_mail_ids(
        &self,
        routine_id: &str,
        account_id: &str,
        item_ids: &[String],
    ) -> Result<(), VaultError> {
        validate_routine_mail_ids(routine_id, account_id, item_ids)?;
        self.with_routine_cursors(|mut cursors| {
            let Some(index) = cursors
                .iter()
                .position(|cursor| cursor.routine_id == routine_id)
            else {
                return Err(VaultError::InvalidReference);
            };
            if cursors[index].account_id != account_id {
                return Err(VaultError::InvalidReference);
            }
            let cursor = &mut cursors[index];
            if item_ids
                .iter()
                .any(|id| !cursor.pending_ids.iter().any(|pending| pending == id))
            {
                return Err(VaultError::InvalidReference);
            }
            for item_id in item_ids {
                cursor.pending_ids.retain(|pending| pending != item_id);
                if !cursor
                    .delivered_ids
                    .iter()
                    .any(|delivered| delivered == item_id)
                {
                    cursor.delivered_ids.push(item_id.clone());
                }
            }
            self.write_routine_cursors(&cursors)
        })
    }

    /// Resolve staged IDs after the scheduler has observed a terminal run.
    /// Successful runs consume them; failed, interrupted or refused runs put
    /// them back at the front of the durable queue for at-least-once recovery.
    pub fn resolve_delivered_mail_ids(
        &self,
        routine_id: &str,
        account_id: &str,
        run_succeeded: bool,
    ) -> Result<(), VaultError> {
        validate_account_id(account_id)?;
        validate_account_id(routine_id)?;
        self.with_routine_cursors(|mut cursors| {
            let Some(index) = cursors
                .iter()
                .position(|cursor| cursor.routine_id == routine_id)
            else {
                return Ok(());
            };
            if cursors[index].account_id != account_id {
                return Err(VaultError::InvalidReference);
            }
            let cursor = &mut cursors[index];
            let delivered = std::mem::take(&mut cursor.delivered_ids);
            if run_succeeded {
                for item_id in delivered {
                    if !cursor.seen_ids.iter().any(|seen| seen == &item_id) {
                        cursor.seen_ids.push(item_id);
                    }
                }
                if cursor.seen_ids.len() > MAX_ROUTINE_SEEN_IDS {
                    let remove = cursor.seen_ids.len() - MAX_ROUTINE_SEEN_IDS;
                    cursor.seen_ids.drain(..remove);
                }
            } else {
                let mut pending = delivered;
                pending.append(&mut cursor.pending_ids);
                if pending.len() > MAX_ROUTINE_PENDING_IDS {
                    return Err(VaultError::TooLarge);
                }
                cursor.pending_ids = pending;
            }
            self.write_routine_cursors(&cursors)
        })
    }

    /// Queue opaque calendar occurrence keys in this Agent's encrypted vault.
    /// Occurrences have their own backlog so mail IDs cannot be consumed by
    /// calendar runs (or vice versa).
    pub fn queue_calendar_occurrences(
        &self,
        routine_id: &str,
        account_id: &str,
        occurrence_keys: &[String],
    ) -> Result<bool, VaultError> {
        validate_calendar_occurrence_keys(routine_id, account_id, occurrence_keys)?;
        self.with_routine_cursors(|mut cursors| {
            let index = match cursors
                .iter()
                .position(|cursor| cursor.routine_id == routine_id)
            {
                Some(index) => {
                    if cursors[index].account_id != account_id {
                        return Err(VaultError::InvalidReference);
                    }
                    index
                }
                None => {
                    if cursors.len() >= MAX_ROUTINE_CURSORS {
                        return Err(VaultError::TooLarge);
                    }
                    cursors.push(RoutineCursor {
                        routine_id: routine_id.to_owned(),
                        account_id: account_id.to_owned(),
                        seen_ids: Vec::new(),
                        pending_ids: Vec::new(),
                        delivered_ids: Vec::new(),
                        seen_calendar_occurrences: Vec::new(),
                        pending_calendar_occurrences: Vec::new(),
                        delivered_calendar_occurrences: Vec::new(),
                        provider_cursor: None,
                    });
                    cursors.len() - 1
                }
            };
            let cursor = &mut cursors[index];
            for key in occurrence_keys {
                if cursor.seen_calendar_occurrences.contains(key)
                    || cursor.pending_calendar_occurrences.contains(key)
                    || cursor.delivered_calendar_occurrences.contains(key)
                {
                    continue;
                }
                if cursor.pending_calendar_occurrences.len()
                    + cursor.delivered_calendar_occurrences.len()
                    >= MAX_ROUTINE_PENDING_IDS
                {
                    return Err(VaultError::TooLarge);
                }
                cursor.pending_calendar_occurrences.push(key.clone());
            }
            let has_pending = !cursor.pending_calendar_occurrences.is_empty();
            self.write_routine_cursors(&cursors)?;
            Ok(has_pending)
        })
    }

    /// Replace unclaimed calendar candidates with the latest successful
    /// provider observation. A moved or cancelled event must not leave an
    /// obsolete occurrence blocking future polls; failed reads never call
    /// this method, so their prior queue remains recoverable.
    pub fn reconcile_calendar_occurrences(
        &self,
        routine_id: &str,
        account_id: &str,
        observed_keys: &[String],
    ) -> Result<bool, VaultError> {
        validate_calendar_occurrence_keys(routine_id, account_id, observed_keys)?;
        self.with_routine_cursors(|mut cursors| {
            let index = match cursors
                .iter()
                .position(|cursor| cursor.routine_id == routine_id)
            {
                Some(index) => {
                    if cursors[index].account_id != account_id {
                        return Err(VaultError::InvalidReference);
                    }
                    index
                }
                None => {
                    if cursors.len() >= MAX_ROUTINE_CURSORS {
                        return Err(VaultError::TooLarge);
                    }
                    cursors.push(RoutineCursor {
                        routine_id: routine_id.to_owned(),
                        account_id: account_id.to_owned(),
                        seen_ids: Vec::new(),
                        pending_ids: Vec::new(),
                        delivered_ids: Vec::new(),
                        seen_calendar_occurrences: Vec::new(),
                        pending_calendar_occurrences: Vec::new(),
                        delivered_calendar_occurrences: Vec::new(),
                        provider_cursor: None,
                    });
                    cursors.len() - 1
                }
            };
            let cursor = &mut cursors[index];
            if !cursor.delivered_calendar_occurrences.is_empty() {
                return Err(VaultError::Conflict);
            }
            let mut pending = Vec::new();
            for key in observed_keys {
                if cursor.seen_calendar_occurrences.contains(key) || pending.contains(key) {
                    continue;
                }
                if pending.len() >= MAX_ROUTINE_PENDING_IDS {
                    return Err(VaultError::TooLarge);
                }
                pending.push(key.clone());
            }
            cursor.pending_calendar_occurrences = pending;
            let has_pending = !cursor.pending_calendar_occurrences.is_empty();
            self.write_routine_cursors(&cursors)?;
            Ok(has_pending)
        })
    }

    pub fn pending_calendar_occurrences(
        &self,
        routine_id: &str,
        account_id: &str,
        limit: usize,
    ) -> Result<Vec<String>, VaultError> {
        validate_routine_mail_ids(routine_id, account_id, &[])?;
        if !(1..=MAX_ROUTINE_PENDING_IDS).contains(&limit) {
            return Err(VaultError::InvalidReference);
        }
        self.with_routine_cursors(|cursors| {
            let Some(cursor) = cursors
                .iter()
                .find(|cursor| cursor.routine_id == routine_id)
            else {
                return Ok(Vec::new());
            };
            if cursor.account_id != account_id {
                return Err(VaultError::InvalidReference);
            }
            Ok(cursor
                .pending_calendar_occurrences
                .iter()
                .take(limit)
                .cloned()
                .collect())
        })
    }

    pub fn has_unresolved_calendar_occurrences(
        &self,
        routine_id: &str,
        account_id: &str,
    ) -> Result<bool, VaultError> {
        validate_routine_mail_ids(routine_id, account_id, &[])?;
        self.with_routine_cursors(|cursors| {
            let Some(cursor) = cursors
                .iter()
                .find(|cursor| cursor.routine_id == routine_id)
            else {
                return Ok(false);
            };
            if cursor.account_id != account_id {
                return Err(VaultError::InvalidReference);
            }
            Ok(!cursor.pending_calendar_occurrences.is_empty()
                || !cursor.delivered_calendar_occurrences.is_empty())
        })
    }

    pub fn stage_delivered_calendar_occurrences(
        &self,
        routine_id: &str,
        account_id: &str,
        occurrence_keys: &[String],
    ) -> Result<(), VaultError> {
        validate_calendar_occurrence_keys(routine_id, account_id, occurrence_keys)?;
        self.with_routine_cursors(|mut cursors| {
            let Some(index) = cursors
                .iter()
                .position(|cursor| cursor.routine_id == routine_id)
            else {
                return Err(VaultError::InvalidReference);
            };
            if cursors[index].account_id != account_id {
                return Err(VaultError::InvalidReference);
            }
            let cursor = &mut cursors[index];
            if occurrence_keys
                .iter()
                .any(|key| !cursor.pending_calendar_occurrences.contains(key))
            {
                return Err(VaultError::InvalidReference);
            }
            for key in occurrence_keys {
                cursor
                    .pending_calendar_occurrences
                    .retain(|pending| pending != key);
                if !cursor.delivered_calendar_occurrences.contains(key) {
                    cursor.delivered_calendar_occurrences.push(key.clone());
                }
            }
            self.write_routine_cursors(&cursors)
        })
    }

    pub fn resolve_delivered_calendar_occurrences(
        &self,
        routine_id: &str,
        account_id: &str,
        run_succeeded: bool,
    ) -> Result<(), VaultError> {
        validate_routine_mail_ids(routine_id, account_id, &[])?;
        self.with_routine_cursors(|mut cursors| {
            let Some(index) = cursors
                .iter()
                .position(|cursor| cursor.routine_id == routine_id)
            else {
                return Ok(());
            };
            if cursors[index].account_id != account_id {
                return Err(VaultError::InvalidReference);
            }
            let cursor = &mut cursors[index];
            let delivered = std::mem::take(&mut cursor.delivered_calendar_occurrences);
            if run_succeeded {
                for key in delivered {
                    if !cursor.seen_calendar_occurrences.contains(&key) {
                        cursor.seen_calendar_occurrences.push(key);
                    }
                }
                if cursor.seen_calendar_occurrences.len() > MAX_ROUTINE_SEEN_IDS {
                    let remove = cursor.seen_calendar_occurrences.len() - MAX_ROUTINE_SEEN_IDS;
                    cursor.seen_calendar_occurrences.drain(..remove);
                }
            } else {
                let mut pending = delivered;
                pending.append(&mut cursor.pending_calendar_occurrences);
                if pending.len() > MAX_ROUTINE_PENDING_IDS {
                    return Err(VaultError::TooLarge);
                }
                cursor.pending_calendar_occurrences = pending;
            }
            self.write_routine_cursors(&cursors)
        })
    }

    /// Return only this Agent's saved local candidates. Content is held as one
    /// bounded secret-store item, never in a plaintext JSON file or browser
    /// local storage.
    pub fn list_candidates(&self) -> Result<Vec<ActionCandidate>, VaultError> {
        self.with_work_area(|candidates| Ok(candidates))
    }

    /// Save a candidate revision using compare-and-swap semantics. Its Agent,
    /// account, audience, source lineage, and creation time cannot change
    /// across revisions. The credential store encrypts the whole work area.
    pub fn save_candidate(
        &self,
        candidate: ActionCandidate,
        expected_revision: Option<u64>,
    ) -> Result<(), VaultError> {
        candidate
            .validate()
            .map_err(|_| VaultError::InvalidReference)?;
        if candidate.agent_id != self.agent_id {
            return Err(VaultError::InvalidReference);
        }
        self.with_work_area(|mut candidates| {
            let index = candidates.iter().position(|saved| saved.id == candidate.id);
            match (index, expected_revision) {
                (None, None) if candidate.revision == 1 => {}
                (Some(index), Some(expected)) => {
                    let previous = &candidates[index];
                    if previous.revision != expected
                        || candidate.revision
                            != expected.checked_add(1).ok_or(VaultError::Conflict)?
                        || previous.account_id != candidate.account_id
                        || previous.agent_id != candidate.agent_id
                        || previous.audience_id != candidate.audience_id
                        || previous.source_refs != candidate.source_refs
                        || previous.created_at != candidate.created_at
                    {
                        return Err(VaultError::Conflict);
                    }
                    candidates[index] = candidate;
                    return self.write_work_area(&candidates);
                }
                _ => return Err(VaultError::Conflict),
            }
            if candidates.len() >= MAX_WORK_AREA_CANDIDATES {
                return Err(VaultError::TooLarge);
            }
            candidates.push(candidate);
            self.write_work_area(&candidates)
        })
    }

    pub fn delete_candidate(
        &self,
        candidate_id: &str,
        expected_revision: u64,
    ) -> Result<(), VaultError> {
        self.with_work_area(|mut candidates| {
            let Some(index) = candidates.iter().position(|item| item.id == candidate_id) else {
                return Ok(());
            };
            if candidates[index].revision != expected_revision {
                return Err(VaultError::Conflict);
            }
            candidates.remove(index);
            self.write_work_area(&candidates)
        })
    }

    fn remove_candidates_for_account(&self, account_id: &str) -> Result<(), VaultError> {
        self.with_work_area(|mut candidates| {
            let before = candidates.len();
            candidates.retain(|candidate| candidate.account_id != account_id);
            if candidates.len() != before {
                self.write_work_area(&candidates)?;
            }
            Ok(())
        })
    }

    fn remove_routine_cursors_for_account(&self, account_id: &str) -> Result<(), VaultError> {
        self.with_routine_cursors(|mut cursors| {
            let before = cursors.len();
            cursors.retain(|cursor| cursor.account_id != account_id);
            if cursors.len() != before {
                self.write_routine_cursors(&cursors)?;
            }
            Ok(())
        })
    }

    fn remove_action_receipts_for_account(&self, account_id: &str) -> Result<(), VaultError> {
        self.with_action_receipts(|mut receipts| {
            let before = receipts.len();
            receipts.retain(|receipt| receipt.account_id != account_id);
            if receipts.len() != before {
                self.write_action_receipts(&receipts)?;
            }
            Ok(())
        })
    }

    fn with_action_receipts<T>(
        &self,
        operation: impl FnOnce(Vec<ActionReceipt>) -> Result<T, VaultError>,
    ) -> Result<T, VaultError> {
        let agent_home = self
            .scope_hint
            .parent()
            .ok_or(VaultError::InvalidReference)?;
        let work_dir = agent_home.join("mail-calendar");
        ensure_agent_directory(&work_dir, true)?;
        let lock_path = work_dir.join(".action-receipts.lock");
        if let Ok(metadata) = fs::symlink_metadata(&lock_path)
            && (metadata.file_type().is_symlink() || !metadata.is_file())
        {
            return Err(VaultError::InvalidReference);
        }
        let mut options = OpenOptions::new();
        options.create(true).truncate(false).read(true).write(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(0o600).custom_flags(libc::O_NOFOLLOW);
        }
        let lock = options.open(&lock_path).map_err(VaultError::Store)?;
        lock.lock_exclusive().map_err(VaultError::Store)?;
        let result = (|| {
            let Some(encoded) =
                vak_config::read_env_file_var(&self.scope_hint, ACTION_RECEIPTS_KEY)
            else {
                return operation(Vec::new());
            };
            let encoded = Zeroizing::new(encoded);
            if encoded.len() > MAX_ACTION_RECEIPTS_BYTES {
                return Err(VaultError::TooLarge);
            }
            let receipts: Vec<ActionReceipt> =
                serde_json::from_str(&encoded).map_err(|_| VaultError::Unavailable)?;
            if receipts.len() > MAX_ACTION_RECEIPTS
                || receipts.iter().any(|receipt| {
                    validate_account_id(&receipt.account_id).is_err()
                        || !Uuid::parse_str(&receipt.candidate_id)
                            .is_ok_and(|id| id.get_version_num() == 7)
                        || !Uuid::parse_str(&receipt.attempt_id)
                            .is_ok_and(|id| id.get_version_num() == 7)
                        || receipt.logical_action_id != receipt.candidate_id
                        || receipt.candidate_digest.len() != 64
                        || !receipt
                            .candidate_digest
                            .bytes()
                            .all(|byte| byte.is_ascii_hexdigit())
                })
                || receipts.iter().enumerate().any(|(index, receipt)| {
                    receipts[index + 1..]
                        .iter()
                        .any(|next| next.candidate_id == receipt.candidate_id)
                })
            {
                return Err(VaultError::InvalidReference);
            }
            operation(receipts)
        })();
        let unlock_result = FileExt::unlock(&lock).map_err(VaultError::Store);
        if let Err(error) = unlock_result {
            return Err(error);
        }
        result
    }

    fn write_action_receipts(&self, receipts: &[ActionReceipt]) -> Result<(), VaultError> {
        let encoded =
            Zeroizing::new(serde_json::to_string(receipts).map_err(|_| VaultError::Unavailable)?);
        if encoded.len() > MAX_ACTION_RECEIPTS_BYTES {
            return Err(VaultError::TooLarge);
        }
        vak_config::upsert_env_file(&self.scope_hint, ACTION_RECEIPTS_KEY, &encoded)
            .map_err(VaultError::Store)
    }

    fn with_routine_cursors<T>(
        &self,
        operation: impl FnOnce(Vec<RoutineCursor>) -> Result<T, VaultError>,
    ) -> Result<T, VaultError> {
        let agent_home = self
            .scope_hint
            .parent()
            .ok_or(VaultError::InvalidReference)?;
        let work_dir = agent_home.join("mail-calendar");
        ensure_agent_directory(&work_dir, true)?;
        let lock_path = work_dir.join(".routine-cursors.lock");
        if let Ok(metadata) = fs::symlink_metadata(&lock_path)
            && (metadata.file_type().is_symlink() || !metadata.is_file())
        {
            return Err(VaultError::InvalidReference);
        }
        let mut options = OpenOptions::new();
        options.create(true).truncate(false).read(true).write(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(0o600).custom_flags(libc::O_NOFOLLOW);
        }
        let lock = options.open(&lock_path).map_err(VaultError::Store)?;
        lock.lock_exclusive().map_err(VaultError::Store)?;
        let result =
            (|| {
                let Some(encoded) =
                    vak_config::read_env_file_var(&self.scope_hint, ROUTINE_CURSOR_KEY)
                else {
                    return operation(Vec::new());
                };
                let encoded = Zeroizing::new(encoded);
                if encoded.len() > MAX_ROUTINE_CURSOR_BYTES {
                    return Err(VaultError::TooLarge);
                }
                let cursors: Vec<RoutineCursor> =
                    serde_json::from_str(&encoded).map_err(|_| VaultError::Unavailable)?;
                if cursors.len() > MAX_ROUTINE_CURSORS
                    || cursors.iter().any(|cursor| {
                        validate_account_id(&cursor.account_id).is_err()
                            || validate_account_id(&cursor.routine_id).is_err()
                            || cursor.seen_ids.len() > MAX_ROUTINE_SEEN_IDS
                            || cursor.pending_ids.len() > MAX_ROUTINE_PENDING_IDS
                            || cursor.delivered_ids.len() > MAX_ROUTINE_PENDING_IDS
                            || cursor.seen_calendar_occurrences.len() > MAX_ROUTINE_SEEN_IDS
                            || cursor.pending_calendar_occurrences.len() > MAX_ROUTINE_PENDING_IDS
                            || cursor.delivered_calendar_occurrences.len() > MAX_ROUTINE_PENDING_IDS
                            || cursor.provider_cursor.as_ref().is_some_and(|value| {
                                value.is_empty()
                                    || value.len() > MAX_PROVIDER_CURSOR_BYTES
                                    || value.chars().any(char::is_control)
                            })
                            || cursor.pending_ids.len() + cursor.delivered_ids.len()
                                > MAX_ROUTINE_PENDING_IDS
                            || cursor.seen_ids.iter().any(|id| {
                                id.is_empty() || id.len() > 512 || id.chars().any(char::is_control)
                            })
                            || cursor
                                .pending_calendar_occurrences
                                .iter()
                                .any(|key| !valid_calendar_occurrence_key(key))
                            || cursor
                                .delivered_calendar_occurrences
                                .iter()
                                .any(|key| !valid_calendar_occurrence_key(key))
                            || cursor
                                .seen_calendar_occurrences
                                .iter()
                                .any(|key| !valid_calendar_occurrence_key(key))
                            || cursor.pending_calendar_occurrences.len()
                                + cursor.delivered_calendar_occurrences.len()
                                > MAX_ROUTINE_PENDING_IDS
                            || cursor.pending_calendar_occurrences.iter().any(|pending| {
                                cursor
                                    .seen_calendar_occurrences
                                    .iter()
                                    .any(|seen| seen == pending)
                            })
                            || cursor
                                .delivered_calendar_occurrences
                                .iter()
                                .any(|delivered| {
                                    cursor
                                        .seen_calendar_occurrences
                                        .iter()
                                        .any(|seen| seen == delivered)
                                        || cursor
                                            .pending_calendar_occurrences
                                            .iter()
                                            .any(|pending| pending == delivered)
                                })
                            || cursor.pending_calendar_occurrences.iter().enumerate().any(
                                |(i, key)| {
                                    cursor.pending_calendar_occurrences[i + 1..]
                                        .iter()
                                        .any(|next| next == key)
                                },
                            )
                            || cursor
                                .delivered_calendar_occurrences
                                .iter()
                                .enumerate()
                                .any(|(i, key)| {
                                    cursor.delivered_calendar_occurrences[i + 1..]
                                        .iter()
                                        .any(|next| next == key)
                                })
                            || cursor.seen_calendar_occurrences.iter().enumerate().any(
                                |(i, key)| {
                                    cursor.seen_calendar_occurrences[i + 1..]
                                        .iter()
                                        .any(|next| next == key)
                                },
                            )
                            || cursor.pending_ids.iter().any(|id| {
                                id.is_empty() || id.len() > 512 || id.chars().any(char::is_control)
                            })
                            || cursor
                                .pending_ids
                                .iter()
                                .any(|pending| cursor.seen_ids.iter().any(|seen| seen == pending))
                            || cursor.delivered_ids.iter().any(|delivered| {
                                cursor.seen_ids.iter().any(|seen| seen == delivered)
                                    || cursor
                                        .pending_ids
                                        .iter()
                                        .any(|pending| pending == delivered)
                            })
                            || cursor
                                .pending_ids
                                .iter()
                                .enumerate()
                                .any(|(index, pending)| {
                                    cursor.pending_ids[index + 1..]
                                        .iter()
                                        .any(|next| next == pending)
                                })
                            || cursor
                                .delivered_ids
                                .iter()
                                .enumerate()
                                .any(|(index, delivered)| {
                                    cursor.delivered_ids[index + 1..]
                                        .iter()
                                        .any(|next| next == delivered)
                                })
                    })
                    || cursors.iter().enumerate().any(|(index, cursor)| {
                        cursors[index + 1..]
                            .iter()
                            .any(|next| next.routine_id == cursor.routine_id)
                    })
                {
                    return Err(VaultError::InvalidReference);
                }
                operation(cursors)
            })();
        let unlock_result = FileExt::unlock(&lock).map_err(VaultError::Store);
        if let Err(error) = unlock_result {
            return Err(error);
        }
        result
    }

    fn with_routine_history<T>(
        &self,
        operation: impl FnOnce(Vec<RoutineRunRecord>) -> Result<T, VaultError>,
    ) -> Result<T, VaultError> {
        let agent_home = self
            .scope_hint
            .parent()
            .ok_or(VaultError::InvalidReference)?;
        let work_dir = agent_home.join("mail-calendar");
        ensure_agent_directory(&work_dir, true)?;
        let lock_path = work_dir.join(".routine-history.lock");
        if let Ok(metadata) = fs::symlink_metadata(&lock_path)
            && (metadata.file_type().is_symlink() || !metadata.is_file())
        {
            return Err(VaultError::InvalidReference);
        }
        let mut options = OpenOptions::new();
        options.create(true).truncate(false).read(true).write(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(0o600).custom_flags(libc::O_NOFOLLOW);
        }
        let lock = options.open(&lock_path).map_err(VaultError::Store)?;
        lock.lock_exclusive().map_err(VaultError::Store)?;
        let result = (|| {
            let Some(encoded) =
                vak_config::read_env_file_var(&self.scope_hint, ROUTINE_HISTORY_KEY)
            else {
                return operation(Vec::new());
            };
            let encoded = Zeroizing::new(encoded);
            if encoded.len() > MAX_ROUTINE_HISTORY_BYTES {
                return Err(VaultError::TooLarge);
            }
            let runs: Vec<RoutineRunRecord> =
                serde_json::from_str(&encoded).map_err(|_| VaultError::Unavailable)?;
            if runs.len() > MAX_ROUTINE_HISTORY
                || runs
                    .iter()
                    .any(|run| validate_routine_run_record(run).is_err())
                || runs.iter().enumerate().any(|(index, run)| {
                    runs[index + 1..]
                        .iter()
                        .any(|next| next.run_id == run.run_id)
                })
            {
                return Err(VaultError::InvalidReference);
            }
            operation(runs)
        })();
        let unlock_result = FileExt::unlock(&lock).map_err(VaultError::Store);
        if let Err(error) = unlock_result {
            return Err(error);
        }
        result
    }

    fn write_routine_history(&self, runs: &[RoutineRunRecord]) -> Result<(), VaultError> {
        if runs.len() > MAX_ROUTINE_HISTORY
            || runs
                .iter()
                .any(|run| validate_routine_run_record(run).is_err())
        {
            return Err(VaultError::InvalidReference);
        }
        let encoded =
            Zeroizing::new(serde_json::to_string(runs).map_err(|_| VaultError::Unavailable)?);
        if encoded.len() > MAX_ROUTINE_HISTORY_BYTES {
            return Err(VaultError::TooLarge);
        }
        vak_config::upsert_env_file(&self.scope_hint, ROUTINE_HISTORY_KEY, &encoded)
            .map_err(VaultError::Store)
    }

    fn write_routine_cursors(&self, cursors: &[RoutineCursor]) -> Result<(), VaultError> {
        let encoded =
            Zeroizing::new(serde_json::to_string(cursors).map_err(|_| VaultError::Unavailable)?);
        if encoded.len() > MAX_ROUTINE_CURSOR_BYTES {
            return Err(VaultError::TooLarge);
        }
        vak_config::upsert_env_file(&self.scope_hint, ROUTINE_CURSOR_KEY, &encoded)
            .map_err(VaultError::Store)
    }

    fn with_work_area<T>(
        &self,
        operation: impl FnOnce(Vec<ActionCandidate>) -> Result<T, VaultError>,
    ) -> Result<T, VaultError> {
        let agent_home = self
            .scope_hint
            .parent()
            .ok_or(VaultError::InvalidReference)?;
        let work_dir = agent_home.join("mail-calendar");
        ensure_agent_directory(&work_dir, true)?;
        let lock_path = work_dir.join(".working-area.lock");
        if let Ok(metadata) = fs::symlink_metadata(&lock_path)
            && (metadata.file_type().is_symlink() || !metadata.is_file())
        {
            return Err(VaultError::InvalidReference);
        }
        let mut options = OpenOptions::new();
        options.create(true).truncate(false).read(true).write(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(0o600).custom_flags(libc::O_NOFOLLOW);
        }
        let lock = options.open(&lock_path).map_err(VaultError::Store)?;
        lock.lock_exclusive().map_err(VaultError::Store)?;
        let result = (|| {
            let encoded = vak_config::read_env_file_var(&self.scope_hint, WORK_AREA_KEY);
            let Some(encoded) = encoded else {
                return operation(Vec::new());
            };
            let encoded = Zeroizing::new(encoded);
            if encoded.len() > MAX_WORK_AREA_BYTES {
                return Err(VaultError::TooLarge);
            }
            let candidates: Vec<ActionCandidate> =
                serde_json::from_str(&encoded).map_err(|_| VaultError::Unavailable)?;
            if candidates.len() > MAX_WORK_AREA_CANDIDATES
                || candidates.iter().any(|candidate| {
                    candidate.agent_id != self.agent_id || candidate.validate().is_err()
                })
                || candidates.iter().enumerate().any(|(index, candidate)| {
                    candidates[index + 1..]
                        .iter()
                        .any(|next| next.id == candidate.id)
                })
            {
                return Err(VaultError::InvalidReference);
            }
            operation(candidates)
        })();
        let unlock_result = FileExt::unlock(&lock).map_err(VaultError::Store);
        if let Err(error) = unlock_result {
            return Err(error);
        }
        result
    }

    fn write_work_area(&self, candidates: &[ActionCandidate]) -> Result<(), VaultError> {
        let encoded =
            Zeroizing::new(serde_json::to_string(candidates).map_err(|_| VaultError::Unavailable)?);
        if encoded.len() > MAX_WORK_AREA_BYTES {
            return Err(VaultError::TooLarge);
        }
        vak_config::upsert_env_file(&self.scope_hint, WORK_AREA_KEY, &encoded)
            .map_err(VaultError::Store)
    }

    pub fn agent_id(&self) -> &str {
        &self.agent_id
    }

    /// Replace OAuth token material while preserving the vaulted provider
    /// identity and client configuration. Used only after a fixed-host token
    /// refresh has passed response and granted-scope validation.
    pub(crate) fn rotate_oauth_tokens(
        &self,
        account_id: &str,
        mut access_token: Zeroizing<String>,
        refresh_token: Option<Zeroizing<String>>,
    ) -> Result<(), VaultError> {
        let current = self.load(account_id)?;
        let mut next_refresh =
            refresh_token.or_else(|| current.refresh_token.clone().map(Zeroizing::new));
        let replacement = AccountSecretMaterial::new(
            current.principal.clone(),
            current.display_identity.clone(),
            current.client_id.clone(),
            Some(std::mem::take(&mut *access_token)),
            next_refresh
                .take()
                .map(|mut token| std::mem::take(&mut *token)),
            current.app_login.clone(),
            current.app_password.clone(),
        )?;
        self.store(account_id, replacement)
    }

    pub(crate) fn oauth_refresh_credentials(
        &self,
        account_id: &str,
    ) -> Result<(Zeroizing<String>, Zeroizing<String>), VaultError> {
        let material = self.load(account_id)?;
        let client_id = material.client_id.clone().ok_or(VaultError::Unavailable)?;
        let refresh_token = material
            .refresh_token
            .clone()
            .ok_or(VaultError::Unavailable)?;
        Ok((Zeroizing::new(client_id), Zeroizing::new(refresh_token)))
    }

    pub(crate) fn oauth_revoke_token(
        &self,
        account_id: &str,
    ) -> Result<Zeroizing<String>, VaultError> {
        let material = self.load(account_id)?;
        material
            .refresh_token
            .clone()
            .or_else(|| material.access_token.clone())
            .map(Zeroizing::new)
            .ok_or(VaultError::Unavailable)
    }

    /// Load only the short-lived OAuth access token for a connected provider
    /// request. The caller must first admit the account and capability; the
    /// returned value is zeroized when dropped and is never serialized.
    pub fn access_token(&self, account_id: &str) -> Result<Zeroizing<String>, VaultError> {
        let material = self.load(account_id)?;
        material
            .access_token
            .clone()
            .map(Zeroizing::new)
            .ok_or(VaultError::Unavailable)
    }

    /// Return the identity hint needed by Graph's delegated getSchedule API.
    /// The value remains vault-owned and callers should drop it immediately
    /// after the request completes.
    pub fn display_identity(
        &self,
        account_id: &str,
    ) -> Result<Option<Zeroizing<String>>, VaultError> {
        let material = self.load(account_id)?;
        Ok(material.display_identity.clone().map(Zeroizing::new))
    }

    /// Load only the vaulted iCloud IMAP principal and app-specific password.
    /// Callers must first admit the Agent/account/capability and must never
    /// place either value in logs, URLs, or child-process environments.
    pub(crate) fn icloud_imap_credentials(
        &self,
        account_id: &str,
    ) -> Result<(Zeroizing<String>, Zeroizing<String>), VaultError> {
        let material = self.load(account_id)?;
        let login = material.app_login.clone().ok_or(VaultError::Unavailable)?;
        let password = material
            .app_password
            .clone()
            .ok_or(VaultError::Unavailable)?;
        Ok((Zeroizing::new(login), Zeroizing::new(password)))
    }

    /// Load the local app-password credential used by fixed-host IMAP
    /// adapters. Callers must already have admitted the Agent/account/read.
    pub(crate) fn app_password_credentials(
        &self,
        account_id: &str,
    ) -> Result<(Zeroizing<String>, Zeroizing<String>), VaultError> {
        self.icloud_imap_credentials(account_id)
    }

    pub fn has_app_password(&self, account_id: &str) -> bool {
        self.load(account_id)
            .is_ok_and(|material| material.app_login.is_some() && material.app_password.is_some())
    }

    /// Record a content-free run start in this Agent's encrypted vault.
    pub fn start_routine_run(
        &self,
        routine_id: &str,
        account_id: &str,
        trigger: RoutineRunTrigger,
        started_at: chrono::DateTime<chrono::Utc>,
    ) -> Result<RoutineRunRecord, VaultError> {
        validate_routine_mail_ids(routine_id, account_id, &[])?;
        let record = RoutineRunRecord {
            run_id: Uuid::now_v7().to_string(),
            routine_id: routine_id.to_owned(),
            account_id: account_id.to_owned(),
            session_id: None,
            trigger,
            status: RoutineRunStatus::Running,
            started_at,
            finished_at: None,
            items_returned: 0,
        };
        validate_routine_run_record(&record)?;
        self.with_routine_history(|mut runs| {
            // The caller holds this routine's OS lease. Any still-running
            // record therefore belongs to a process that exited before it
            // could settle the run.
            for previous in runs.iter_mut().filter(|previous| {
                previous.routine_id == routine_id && previous.status == RoutineRunStatus::Running
            }) {
                previous.status = RoutineRunStatus::Interrupted;
                previous.finished_at = Some(started_at.max(previous.started_at));
            }
            if runs.iter().any(|run| run.run_id == record.run_id) {
                return Err(VaultError::Conflict);
            }
            if runs.len() >= MAX_ROUTINE_HISTORY {
                let Some(oldest_terminal) = runs
                    .iter()
                    .position(|run| run.status != RoutineRunStatus::Running)
                else {
                    return Err(VaultError::TooLarge);
                };
                runs.remove(oldest_terminal);
            }
            runs.push(record.clone());
            self.write_routine_history(&runs)?;
            Ok(record)
        })
    }

    /// Attach the Agent session created for a model-backed routine run.
    pub fn attach_routine_run_session(
        &self,
        routine_id: &str,
        account_id: &str,
        run_id: &str,
        session_id: &str,
    ) -> Result<(), VaultError> {
        validate_routine_mail_ids(routine_id, account_id, &[])?;
        validate_account_id(run_id)?;
        validate_account_id(session_id)?;
        self.with_routine_history(|mut runs| {
            let Some(run) = runs.iter_mut().find(|run| {
                run.run_id == run_id && run.routine_id == routine_id && run.account_id == account_id
            }) else {
                return Err(VaultError::InvalidReference);
            };
            if run.status != RoutineRunStatus::Running
                || run
                    .session_id
                    .as_deref()
                    .is_some_and(|existing| existing != session_id)
            {
                return Err(VaultError::Conflict);
            }
            run.session_id = Some(session_id.to_owned());
            self.write_routine_history(&runs)
        })
    }

    /// Settle one run once its terminal outcome is known.
    pub fn finish_routine_run(
        &self,
        routine_id: &str,
        account_id: &str,
        run_id: &str,
        status: RoutineRunStatus,
        finished_at: chrono::DateTime<chrono::Utc>,
    ) -> Result<(), VaultError> {
        validate_routine_mail_ids(routine_id, account_id, &[])?;
        validate_account_id(run_id)?;
        if matches!(status, RoutineRunStatus::Running) {
            return Err(VaultError::InvalidReference);
        }
        self.with_routine_history(|mut runs| {
            let Some(run) = runs.iter_mut().find(|run| {
                run.run_id == run_id && run.routine_id == routine_id && run.account_id == account_id
            }) else {
                return Ok(());
            };
            if run.status == status {
                return Ok(());
            }
            if run.status != RoutineRunStatus::Running {
                return Err(VaultError::Conflict);
            }
            run.status = status;
            run.finished_at = Some(finished_at);
            self.write_routine_history(&runs)
        })
    }

    /// Record content-free result usage for a live run before it is settled.
    pub fn record_routine_run_items(
        &self,
        routine_id: &str,
        account_id: &str,
        run_id: &str,
        items_returned: u8,
    ) -> Result<(), VaultError> {
        validate_routine_mail_ids(routine_id, account_id, &[])?;
        validate_account_id(run_id)?;
        if items_returned > 20 {
            return Err(VaultError::InvalidReference);
        }
        self.with_routine_history(|mut runs| {
            let Some(run) = runs.iter_mut().find(|run| {
                run.run_id == run_id && run.routine_id == routine_id && run.account_id == account_id
            }) else {
                return Err(VaultError::InvalidReference);
            };
            if run.status != RoutineRunStatus::Running {
                return Err(VaultError::Conflict);
            }
            run.items_returned = items_returned;
            self.write_routine_history(&runs)
        })
    }

    /// Mark a run interrupted when startup recovery sees its session still
    /// recorded as working after the previous service process exited.
    pub fn interrupt_routine_run_session(
        &self,
        routine_id: &str,
        account_id: &str,
        session_id: &str,
        finished_at: chrono::DateTime<chrono::Utc>,
    ) -> Result<(), VaultError> {
        validate_routine_mail_ids(routine_id, account_id, &[])?;
        validate_account_id(session_id)?;
        self.with_routine_history(|mut runs| {
            let Some(run) = runs.iter_mut().find(|run| {
                run.routine_id == routine_id
                    && run.account_id == account_id
                    && run.session_id.as_deref() == Some(session_id)
            }) else {
                return Ok(());
            };
            if run.status == RoutineRunStatus::Interrupted {
                return Ok(());
            }
            if run.status != RoutineRunStatus::Running {
                return Err(VaultError::Conflict);
            }
            run.status = RoutineRunStatus::Interrupted;
            run.finished_at = Some(finished_at);
            self.write_routine_history(&runs)
        })
    }

    /// Return newest first, scoped to one routine and its selected account.
    pub fn list_routine_runs(
        &self,
        routine_id: &str,
        account_id: &str,
    ) -> Result<Vec<RoutineRunRecord>, VaultError> {
        validate_routine_mail_ids(routine_id, account_id, &[])?;
        self.with_routine_history(|runs| {
            let mut selected = runs
                .into_iter()
                .filter(|run| run.routine_id == routine_id && run.account_id == account_id)
                .collect::<Vec<_>>();
            selected.sort_by(|left, right| right.started_at.cmp(&left.started_at));
            Ok(selected)
        })
    }

    /// Remove run metadata together with an account's credential and cursor.
    pub fn remove_routine_history_for_account(&self, account_id: &str) -> Result<(), VaultError> {
        validate_account_id(account_id)?;
        self.with_routine_history(|mut runs| {
            runs.retain(|run| run.account_id != account_id);
            self.write_routine_history(&runs)
        })
    }

    /// Remove run metadata when its routine is deleted.
    pub fn remove_routine_history(
        &self,
        routine_id: &str,
        account_id: &str,
    ) -> Result<(), VaultError> {
        validate_routine_mail_ids(routine_id, account_id, &[])?;
        self.with_routine_history(|mut runs| {
            runs.retain(|run| run.routine_id != routine_id);
            self.write_routine_history(&runs)
        })
    }
}

fn ensure_agent_directory(path: &std::path::Path, private: bool) -> Result<(), VaultError> {
    match std::fs::symlink_metadata(path) {
        Ok(metadata) if metadata.file_type().is_symlink() || !metadata.is_dir() => {
            return Err(VaultError::InvalidReference);
        }
        Ok(_) => {}
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            std::fs::create_dir_all(path).map_err(VaultError::Store)?;
        }
        Err(error) => return Err(VaultError::Store(error)),
    }
    let metadata = std::fs::symlink_metadata(path).map_err(VaultError::Store)?;
    if metadata.file_type().is_symlink() || !metadata.is_dir() {
        return Err(VaultError::InvalidReference);
    }
    #[cfg(unix)]
    if private {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o700))
            .map_err(VaultError::Store)?;
    }
    Ok(())
}

fn valid_agent_id(value: &str) -> bool {
    !value.is_empty()
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-' || byte == b'_')
}

fn validate_account_id(value: &str) -> Result<(), VaultError> {
    if Uuid::parse_str(value).is_ok_and(|id| id.get_version_num() == 7) {
        Ok(())
    } else {
        Err(VaultError::InvalidReference)
    }
}

fn validate_routine_run_record(record: &RoutineRunRecord) -> Result<(), VaultError> {
    validate_account_id(&record.run_id)?;
    validate_account_id(&record.routine_id)?;
    validate_account_id(&record.account_id)?;
    if record
        .session_id
        .as_ref()
        .is_some_and(|session_id| validate_account_id(session_id).is_err())
        || (record.status == RoutineRunStatus::Running) != record.finished_at.is_none()
        || record.items_returned > 20
        || record
            .finished_at
            .is_some_and(|finished_at| finished_at < record.started_at)
    {
        return Err(VaultError::InvalidReference);
    }
    Ok(())
}

fn validate_routine_mail_ids(
    routine_id: &str,
    account_id: &str,
    item_ids: &[String],
) -> Result<(), VaultError> {
    validate_account_id(routine_id)?;
    validate_account_id(account_id)?;
    if item_ids.len() > MAX_ROUTINE_PENDING_IDS
        || item_ids
            .iter()
            .any(|id| id.is_empty() || id.len() > 512 || id.chars().any(char::is_control))
    {
        return Err(VaultError::InvalidReference);
    }
    Ok(())
}

fn valid_calendar_occurrence_key(value: &str) -> bool {
    value.len() == 73
        && value.starts_with("calendar:")
        && value[9..].bytes().all(|byte| byte.is_ascii_hexdigit())
}

fn validate_calendar_occurrence_keys(
    routine_id: &str,
    account_id: &str,
    occurrence_keys: &[String],
) -> Result<(), VaultError> {
    validate_routine_mail_ids(routine_id, account_id, &[])?;
    if occurrence_keys.len() > MAX_ROUTINE_PENDING_IDS {
        return Err(VaultError::TooLarge);
    }
    if occurrence_keys
        .iter()
        .any(|key| !valid_calendar_occurrence_key(key))
    {
        return Err(VaultError::InvalidReference);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::{
        AccountSecretMaterial, AccountVault, MAX_ROUTINE_PENDING_IDS, RoutineRunRecord,
        RoutineRunStatus, RoutineRunTrigger, Uuid, VaultError,
    };
    use crate::{ActionCandidate, ActionState, MailAddress, MailDraft, ProposedAction, SourceRef};

    fn candidate(agent_id: &str, account_id: &str) -> ActionCandidate {
        ActionCandidate::new(
            account_id.to_owned(),
            agent_id.to_owned(),
            format!("agent:{agent_id}"),
            vec![SourceRef {
                item_id: "message-1".into(),
                version: Some("etag-1".into()),
                label: Some("Selected email".into()),
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
                    subject: "Draft reply".into(),
                    body_text: "Not sent until reviewed".into(),
                    attachment_refs: Vec::new(),
                    reply_to_message_id: Some("message-1".into()),
                    reply_to_thread_id: Some("thread-1".into()),
                },
            },
        )
        .unwrap()
    }

    #[test]
    fn local_candidates_are_agent_vaulted_revisioned_and_deleted_with_account() {
        vak_config::paths::isolate_home_for_tests();
        let agent_id = format!("mailcal-drafts-{}", Uuid::now_v7());
        let account_id = Uuid::now_v7().to_string();
        let other_account = Uuid::now_v7().to_string();
        let vault = AccountVault::for_agent(&agent_id).unwrap();
        let other = AccountVault::for_agent(&format!("mailcal-other-{}", Uuid::now_v7())).unwrap();
        let first = candidate(&agent_id, &account_id);
        let second = candidate(&agent_id, &other_account);
        assert!(matches!(
            other.save_candidate(first.clone(), None),
            Err(VaultError::InvalidReference)
        ));
        vault.save_candidate(first.clone(), None).unwrap();
        vault.save_candidate(second.clone(), None).unwrap();
        assert_eq!(vault.list_candidates().unwrap().len(), 2);

        let mut revision = first.clone();
        if let ProposedAction::SendMail { draft } = &mut revision.action {
            draft.body_text = "Revised locally".into();
        }
        revision.revise(revision.action.clone()).unwrap();
        assert!(matches!(
            vault.save_candidate(revision.clone(), Some(0)),
            Err(VaultError::Conflict)
        ));
        vault.save_candidate(revision, Some(1)).unwrap();
        drop(vault);
        let vault = AccountVault::for_agent(&agent_id).unwrap();
        let reopened = vault.list_candidates().unwrap();
        let reopened_first = reopened
            .iter()
            .find(|candidate| candidate.id == first.id)
            .expect("the saved draft should survive reopening its Agent vault");
        assert_eq!(reopened_first.revision, 2);
        assert!(matches!(
            &reopened_first.action,
            ProposedAction::SendMail { draft } if draft.body_text == "Revised locally"
        ));
        assert!(other.list_candidates().unwrap().is_empty());
        vault.remove(&account_id).unwrap();
        let remaining = vault.list_candidates().unwrap();
        assert_eq!(remaining.len(), 1);
        assert_eq!(remaining[0].account_id, other_account);
    }

    #[test]
    fn routine_history_is_agent_vaulted_bounded_in_shape_and_removed_with_owner_data() {
        vak_config::paths::isolate_home_for_tests();
        let agent_id = format!("mailcal-history-{}", Uuid::now_v7());
        let other_agent_id = format!("mailcal-history-other-{}", Uuid::now_v7());
        let account_id = Uuid::now_v7().to_string();
        let other_account_id = Uuid::now_v7().to_string();
        let routine_id = Uuid::now_v7().to_string();
        let other_routine_id = Uuid::now_v7().to_string();
        let session_id = Uuid::now_v7().to_string();
        let vault = AccountVault::for_agent(&agent_id).unwrap();
        let other_agent = AccountVault::for_agent(&other_agent_id).unwrap();

        let started = chrono::Utc::now();
        let run = vault
            .start_routine_run(&routine_id, &account_id, RoutineRunTrigger::Manual, started)
            .unwrap();
        let mut legacy_record = serde_json::to_value(&run).unwrap();
        legacy_record
            .as_object_mut()
            .unwrap()
            .remove("items_returned");
        assert_eq!(
            serde_json::from_value::<RoutineRunRecord>(legacy_record)
                .unwrap()
                .items_returned,
            0
        );
        vault
            .attach_routine_run_session(&routine_id, &account_id, &run.run_id, &session_id)
            .unwrap();
        assert!(
            other_agent
                .list_routine_runs(&routine_id, &account_id)
                .unwrap()
                .is_empty()
        );
        let reopened = AccountVault::for_agent(&agent_id).unwrap();
        assert_eq!(
            reopened
                .list_routine_runs(&routine_id, &account_id)
                .unwrap()[0]
                .session_id,
            Some(session_id.clone())
        );
        reopened
            .record_routine_run_items(&routine_id, &account_id, &run.run_id, 4)
            .unwrap();
        reopened
            .finish_routine_run(
                &routine_id,
                &account_id,
                &run.run_id,
                RoutineRunStatus::Complete,
                started + chrono::Duration::seconds(2),
            )
            .unwrap();
        assert_eq!(
            reopened
                .list_routine_runs(&routine_id, &account_id)
                .unwrap()[0]
                .items_returned,
            4
        );
        assert!(matches!(
            reopened.finish_routine_run(
                &routine_id,
                &account_id,
                &run.run_id,
                RoutineRunStatus::Failed,
                started + chrono::Duration::seconds(3),
            ),
            Err(VaultError::Conflict)
        ));

        let interrupted = reopened
            .start_routine_run(
                &routine_id,
                &account_id,
                RoutineRunTrigger::Scheduled,
                started + chrono::Duration::seconds(4),
            )
            .unwrap();
        let interrupted_session = Uuid::now_v7().to_string();
        reopened
            .attach_routine_run_session(
                &routine_id,
                &account_id,
                &interrupted.run_id,
                &interrupted_session,
            )
            .unwrap();
        reopened
            .interrupt_routine_run_session(
                &routine_id,
                &account_id,
                &interrupted_session,
                started + chrono::Duration::seconds(5),
            )
            .unwrap();

        let other = reopened
            .start_routine_run(
                &other_routine_id,
                &other_account_id,
                RoutineRunTrigger::Scheduled,
                started,
            )
            .unwrap();
        reopened
            .finish_routine_run(
                &other_routine_id,
                &other_account_id,
                &other.run_id,
                RoutineRunStatus::NoChanges,
                started + chrono::Duration::seconds(1),
            )
            .unwrap();

        let runs = reopened
            .list_routine_runs(&routine_id, &account_id)
            .unwrap();
        assert_eq!(runs.len(), 2);
        assert_eq!(runs[0].status, RoutineRunStatus::Interrupted);
        assert_eq!(runs[1].status, RoutineRunStatus::Complete);
        reopened.remove(&account_id).unwrap();
        assert!(
            reopened
                .list_routine_runs(&routine_id, &account_id)
                .unwrap()
                .is_empty()
        );
        assert_eq!(
            reopened
                .list_routine_runs(&other_routine_id, &other_account_id)
                .unwrap()
                .len(),
            1
        );
    }

    #[test]
    fn routine_mail_backlog_is_bounded_scoped_and_recovers_interrupted_runs() {
        vak_config::paths::isolate_home_for_tests();
        let agent_id = format!("mailcal-backlog-{}", Uuid::now_v7());
        let account_id = Uuid::now_v7().to_string();
        let other_account_id = Uuid::now_v7().to_string();
        let routine_id = Uuid::now_v7().to_string();
        let ids = |values: &[&str]| {
            values
                .iter()
                .map(|value| (*value).to_string())
                .collect::<Vec<_>>()
        };
        let first = AccountVault::for_agent(&agent_id).unwrap();
        assert!(
            first
                .queue_mail_ids_with_cursor(
                    &routine_id,
                    &account_id,
                    &ids(&["m1", "m2", "m3"]),
                    Some("apple-imap:7:42"),
                )
                .unwrap()
        );
        assert!(
            first
                .has_unresolved_mail_ids(&routine_id, &account_id)
                .unwrap()
        );

        let reopened = AccountVault::for_agent(&agent_id).unwrap();
        assert_eq!(
            reopened
                .routine_provider_cursor(&routine_id, &account_id)
                .unwrap()
                .as_deref(),
            Some("apple-imap:7:42")
        );
        assert_eq!(
            reopened
                .pending_mail_ids(&routine_id, &account_id, 2)
                .unwrap(),
            ids(&["m1", "m2"])
        );
        reopened
            .stage_delivered_mail_ids(&routine_id, &account_id, &ids(&["m1"]))
            .unwrap();
        assert!(
            reopened
                .has_unresolved_mail_ids(&routine_id, &account_id)
                .unwrap()
        );
        assert_eq!(
            reopened
                .pending_mail_ids(&routine_id, &account_id, 10)
                .unwrap(),
            ids(&["m2", "m3"])
        );
        reopened
            .resolve_delivered_mail_ids(&routine_id, &account_id, false)
            .unwrap();
        assert_eq!(
            reopened
                .pending_mail_ids(&routine_id, &account_id, 10)
                .unwrap(),
            ids(&["m1", "m2", "m3"])
        );
        assert!(matches!(
            reopened.stage_delivered_mail_ids(&routine_id, &account_id, &ids(&["unfetched"])),
            Err(VaultError::InvalidReference)
        ));
        reopened
            .stage_delivered_mail_ids(&routine_id, &account_id, &ids(&["m1"]))
            .unwrap();
        reopened
            .resolve_delivered_mail_ids(&routine_id, &account_id, true)
            .unwrap();
        assert_eq!(
            reopened
                .pending_mail_ids(&routine_id, &account_id, 10)
                .unwrap(),
            ids(&["m2", "m3"])
        );
        reopened
            .stage_delivered_mail_ids(&routine_id, &account_id, &ids(&["m2", "m3"]))
            .unwrap();
        reopened
            .resolve_delivered_mail_ids(&routine_id, &account_id, true)
            .unwrap();
        assert!(
            reopened
                .pending_mail_ids(&routine_id, &account_id, 10)
                .unwrap()
                .is_empty()
        );
        assert!(
            !reopened
                .has_unresolved_mail_ids(&routine_id, &account_id)
                .unwrap()
        );
        assert!(
            !reopened
                .queue_unseen_mail_ids(&routine_id, &account_id, &ids(&["m1", "m2", "m3"]))
                .unwrap()
        );
        assert!(matches!(
            reopened.queue_unseen_mail_ids(&routine_id, &other_account_id, &ids(&["m4"])),
            Err(VaultError::InvalidReference)
        ));
        reopened.remove(&account_id).unwrap();
        assert!(
            reopened
                .pending_mail_ids(&routine_id, &account_id, 10)
                .unwrap()
                .is_empty()
        );
        assert!(
            reopened
                .queue_unseen_mail_ids(&routine_id, &other_account_id, &ids(&["m4"]))
                .unwrap()
        );
    }

    #[test]
    fn routine_mail_watch_drains_large_synthetic_history_across_restarts() {
        vak_config::paths::isolate_home_for_tests();
        let agent_id = format!("mailcal-large-watch-{}", Uuid::now_v7());
        let account_id = Uuid::now_v7().to_string();
        let routine_id = Uuid::now_v7().to_string();
        let all_ids = (0..1_200)
            .map(|index| format!("synthetic-message-{index:05}"))
            .collect::<Vec<_>>();

        let mut recovered_one_failed_batch = false;
        for (page_number, page) in all_ids.chunks(MAX_ROUTINE_PENDING_IDS).enumerate() {
            let page_ids = page.to_vec();
            let cursor = format!("synthetic-cursor-{}", page_number + 1);
            let vault = AccountVault::for_agent(&agent_id).unwrap();
            assert!(
                vault
                    .queue_mail_ids_with_cursor(&routine_id, &account_id, &page_ids, Some(&cursor))
                    .unwrap()
            );

            // Reopen the encrypted vault between pages and bounded child runs,
            // as if the service had restarted while the watch was draining.
            let reopened = AccountVault::for_agent(&agent_id).unwrap();
            assert_eq!(
                reopened
                    .routine_provider_cursor(&routine_id, &account_id)
                    .unwrap()
                    .as_deref(),
                Some(cursor.as_str())
            );
            loop {
                let vault = AccountVault::for_agent(&agent_id).unwrap();
                let batch = vault
                    .pending_mail_ids(&routine_id, &account_id, 20)
                    .unwrap();
                if batch.is_empty() {
                    break;
                }
                vault
                    .stage_delivered_mail_ids(&routine_id, &account_id, &batch)
                    .unwrap();
                let simulate_interruption = page_number == 4 && !recovered_one_failed_batch;
                vault
                    .resolve_delivered_mail_ids(&routine_id, &account_id, !simulate_interruption)
                    .unwrap();
                if simulate_interruption {
                    recovered_one_failed_batch = true;
                    let restarted = AccountVault::for_agent(&agent_id).unwrap();
                    assert_eq!(
                        restarted
                            .pending_mail_ids(&routine_id, &account_id, 20)
                            .unwrap(),
                        batch,
                        "an interrupted run must requeue its exact batch after restart"
                    );
                }
            }
        }

        assert!(recovered_one_failed_batch);
        let completed = AccountVault::for_agent(&agent_id).unwrap();
        assert!(
            !completed
                .has_unresolved_mail_ids(&routine_id, &account_id)
                .unwrap()
        );
        assert!(
            completed
                .pending_mail_ids(&routine_id, &account_id, MAX_ROUTINE_PENDING_IDS)
                .unwrap()
                .is_empty()
        );

        // Replaying recent provider IDs is harmless within the bounded
        // deduplication window; older IDs rely on the persisted provider cursor.
        let recent_ids = all_ids
            .iter()
            .skip(all_ids.len().saturating_sub(super::MAX_ROUTINE_SEEN_IDS))
            .cloned()
            .collect::<Vec<_>>();
        for page in recent_ids.chunks(MAX_ROUTINE_PENDING_IDS) {
            assert!(
                !completed
                    .queue_unseen_mail_ids(&routine_id, &account_id, &page.to_vec())
                    .unwrap()
            );
        }
        assert!(
            completed
                .pending_mail_ids(&routine_id, &account_id, MAX_ROUTINE_PENDING_IDS)
                .unwrap()
                .is_empty()
        );
    }

    #[test]
    fn calendar_occurrence_backlog_is_encrypted_scoped_and_restart_recoverable() {
        vak_config::paths::isolate_home_for_tests();
        let agent_id = format!("mailcal-calendar-backlog-{}", Uuid::now_v7());
        let other_agent_id = format!("mailcal-calendar-other-{}", Uuid::now_v7());
        let account_id = Uuid::now_v7().to_string();
        let other_account_id = Uuid::now_v7().to_string();
        let routine_id = Uuid::now_v7().to_string();
        let keys = |n: u8| vec![format!("calendar:{}", format!("{n:02x}").repeat(32))];
        let owner = AccountVault::for_agent(&agent_id).unwrap();
        let other_agent = AccountVault::for_agent(&other_agent_id).unwrap();
        assert!(
            owner
                .queue_calendar_occurrences(&routine_id, &account_id, &keys(1))
                .unwrap()
        );
        assert!(matches!(
            owner.queue_calendar_occurrences(&routine_id, &other_account_id, &keys(2)),
            Err(VaultError::InvalidReference)
        ));
        assert!(
            other_agent
                .pending_calendar_occurrences(&routine_id, &account_id, 10)
                .unwrap()
                .is_empty()
        );

        let reopened = AccountVault::for_agent(&agent_id).unwrap();
        assert_eq!(
            reopened
                .pending_calendar_occurrences(&routine_id, &account_id, 10)
                .unwrap(),
            keys(1)
        );
        reopened
            .stage_delivered_calendar_occurrences(&routine_id, &account_id, &keys(1))
            .unwrap();
        assert!(
            reopened
                .has_unresolved_calendar_occurrences(&routine_id, &account_id)
                .unwrap()
        );
        reopened
            .resolve_delivered_calendar_occurrences(&routine_id, &account_id, false)
            .unwrap();
        assert_eq!(
            reopened
                .pending_calendar_occurrences(&routine_id, &account_id, 10)
                .unwrap(),
            keys(1)
        );
        assert!(matches!(
            reopened.stage_delivered_calendar_occurrences(&routine_id, &account_id, &keys(2)),
            Err(VaultError::InvalidReference)
        ));
        reopened
            .stage_delivered_calendar_occurrences(&routine_id, &account_id, &keys(1))
            .unwrap();
        reopened
            .resolve_delivered_calendar_occurrences(&routine_id, &account_id, true)
            .unwrap();
        assert!(
            reopened
                .pending_calendar_occurrences(&routine_id, &account_id, 10)
                .unwrap()
                .is_empty()
        );
        assert!(
            !reopened
                .queue_calendar_occurrences(&routine_id, &account_id, &keys(1))
                .unwrap()
        );
        assert!(
            reopened
                .queue_calendar_occurrences(&routine_id, &account_id, &keys(2))
                .unwrap()
        );
        assert!(
            reopened
                .reconcile_calendar_occurrences(&routine_id, &account_id, &keys(3))
                .unwrap()
        );
        assert_eq!(
            reopened
                .pending_calendar_occurrences(&routine_id, &account_id, 10)
                .unwrap(),
            keys(3),
            "a successful poll replaces occurrences for events that moved"
        );
        assert!(
            !reopened
                .reconcile_calendar_occurrences(&routine_id, &account_id, &[])
                .unwrap()
        );
        assert!(
            reopened
                .pending_calendar_occurrences(&routine_id, &account_id, 10)
                .unwrap()
                .is_empty()
        );
        let overflow = (0..=MAX_ROUTINE_PENDING_IDS)
            .map(|value| format!("calendar:{value:064x}"))
            .collect::<Vec<_>>();
        assert!(matches!(
            reopened.reconcile_calendar_occurrences(&routine_id, &account_id, &overflow),
            Err(VaultError::TooLarge)
        ));
        assert!(
            reopened
                .pending_calendar_occurrences(&routine_id, &account_id, 10)
                .unwrap()
                .is_empty()
        );
        assert!(matches!(
            reopened.queue_calendar_occurrences(&routine_id, &account_id, &["not-a-key".into()]),
            Err(VaultError::InvalidReference)
        ));
    }

    #[test]
    fn routine_lease_serializes_independent_server_vault_handles() {
        let agent_id = format!("mailcal-lease-{}", Uuid::now_v7());
        let first_vault = AccountVault::for_agent(&agent_id).unwrap();
        let second_vault = AccountVault::for_agent(&agent_id).unwrap();
        let routine_id = Uuid::now_v7().to_string();

        let lease = first_vault
            .try_acquire_routine_lease(&routine_id)
            .unwrap()
            .expect("first process claim");
        assert!(
            second_vault
                .try_acquire_routine_lease(&routine_id)
                .unwrap()
                .is_none()
        );

        drop(lease);
        assert!(
            second_vault
                .try_acquire_routine_lease(&routine_id)
                .unwrap()
                .is_some()
        );
    }

    #[test]
    fn account_refresh_lease_serializes_independent_server_vault_handles() {
        let agent_id = format!("mailcal-refresh-lease-{}", Uuid::now_v7());
        let first_vault = AccountVault::for_agent(&agent_id).unwrap();
        let second_vault = AccountVault::for_agent(&agent_id).unwrap();
        let account_id = Uuid::now_v7().to_string();

        let lease = first_vault
            .try_acquire_account_refresh_lease(&account_id)
            .unwrap()
            .expect("first process owns the refresh lease");
        assert!(
            second_vault
                .try_acquire_account_refresh_lease(&account_id)
                .unwrap()
                .is_none(),
            "a second process cannot rotate the same provider credential"
        );

        drop(lease);
        assert!(
            second_vault
                .try_acquire_account_refresh_lease(&account_id)
                .unwrap()
                .is_some()
        );
    }

    #[test]
    fn routine_lease_admits_only_one_concurrent_trigger() {
        use std::sync::{Arc, Barrier, mpsc};

        const CONTENDERS: usize = 16;
        let agent_id = format!("mailcal-lease-race-{}", Uuid::now_v7());
        let routine_id = Uuid::now_v7().to_string();
        let vaults = (0..CONTENDERS)
            .map(|_| AccountVault::for_agent(&agent_id).unwrap())
            .collect::<Vec<_>>();
        let start = Arc::new(Barrier::new(CONTENDERS + 1));
        let release = Arc::new(Barrier::new(2));
        let (tx, rx) = mpsc::channel();
        let workers = vaults
            .into_iter()
            .map(|vault| {
                let start = start.clone();
                let release = release.clone();
                let tx = tx.clone();
                let routine_id = routine_id.clone();
                std::thread::spawn(move || {
                    start.wait();
                    let lease = vault.try_acquire_routine_lease(&routine_id).unwrap();
                    tx.send(lease.is_some()).unwrap();
                    if lease.is_some() {
                        release.wait();
                    }
                    drop(lease);
                })
            })
            .collect::<Vec<_>>();
        drop(tx);

        start.wait();
        let winners = (0..CONTENDERS)
            .filter(|_| rx.recv().expect("every contender reports its result"))
            .count();
        release.wait();
        for worker in workers {
            worker.join().unwrap();
        }
        assert_eq!(winners, 1, "one trigger must own the run lease");

        assert!(
            AccountVault::for_agent(&agent_id)
                .unwrap()
                .try_acquire_routine_lease(&routine_id)
                .unwrap()
                .is_some(),
            "the lease must be released after its owner exits"
        );
    }

    #[test]
    fn action_claim_is_durable_single_use_and_disconnect_scoped() {
        vak_config::paths::isolate_home_for_tests();
        let agent_id = format!("mailcal-action-{}", Uuid::now_v7());
        let account_id = Uuid::now_v7().to_string();
        let vault = AccountVault::for_agent(&agent_id).unwrap();
        let candidate = candidate(&agent_id, &account_id);
        let mut receipt = vault.begin_action(&candidate).unwrap();
        assert_eq!(receipt.state, ActionState::Dispatching);
        assert!(matches!(
            vault.begin_action(&candidate),
            Err(VaultError::Conflict)
        ));

        receipt.state = ActionState::Unknown;
        receipt.detail_code = Some("outcome_unknown".into());
        vault.settle_action(receipt.clone()).unwrap();
        assert_eq!(
            vault.list_action_receipts().unwrap()[0].state,
            ActionState::Unknown
        );
        receipt.state = ActionState::Confirmed;
        receipt.provider_item_id = Some("provider-event".into());
        receipt.detail_code = Some("provider_event_confirmed_by_reconciliation".into());
        vault.settle_action(receipt).unwrap();
        let settled = vault.list_action_receipts().unwrap();
        assert_eq!(settled[0].state, ActionState::Confirmed);
        assert_eq!(
            settled[0].provider_item_id.as_deref(),
            Some("provider-event")
        );
        assert!(matches!(
            vault.begin_action(&candidate),
            Err(VaultError::Conflict)
        ));

        vault.remove(&account_id).unwrap();
        assert!(vault.list_action_receipts().unwrap().is_empty());
    }

    #[test]
    fn only_masked_display_identity_is_available_for_account_surfaces() {
        let material = AccountSecretMaterial::new(
            "provider:opaque-subject".into(),
            Some("owner@example.com".into()),
            None,
            None,
            None,
            None,
            None,
        )
        .unwrap();
        let masked = material.masked_display_identity().unwrap();
        assert_eq!(masked, "o***@example.com");
        assert!(!masked.contains("owner"));
        assert!(material.display_identity_matches("OWNER@example.com"));
        assert!(!material.display_identity_matches("owner2@example.com"));
        assert!(!material.display_identity_matches("owner@example.com\n"));
    }

    #[test]
    fn oauth_and_app_password_links_for_the_same_mailbox_compare_by_private_identity() {
        let oauth = AccountSecretMaterial::new(
            "google:opaque-subject".into(),
            Some("owner@gmail.com".into()),
            None,
            Some("access-token".into()),
            Some("refresh-token".into()),
            None,
            None,
        )
        .unwrap();
        let app_password = AccountSecretMaterial::new(
            "OWNER@gmail.com".into(),
            Some("OWNER@gmail.com".into()),
            None,
            None,
            None,
            Some("OWNER@gmail.com".into()),
            Some("abcdabcdefghijklmnop".into()),
        )
        .unwrap();
        assert!(!oauth.has_same_principal(&app_password));
        assert!(oauth.has_same_display_identity_ignoring_ascii_case(&app_password));
    }

    #[test]
    fn non_email_provider_identity_has_no_display_hint() {
        let material = AccountSecretMaterial::new(
            "provider:opaque-subject".into(),
            Some("opaque-subject".into()),
            None,
            None,
            None,
            None,
            None,
        )
        .unwrap();
        assert_eq!(material.masked_display_identity(), None);
    }

    #[test]
    fn account_secrets_cannot_be_resolved_from_another_agent_vault() {
        vak_config::paths::isolate_home_for_tests();
        let owner_id = format!("mailcal-owner-{}", Uuid::now_v7());
        let other_id = format!("mailcal-other-{}", Uuid::now_v7());
        let account_id = Uuid::now_v7().to_string();
        let owner = AccountVault::for_agent(&owner_id).unwrap();
        let other = AccountVault::for_agent(&other_id).unwrap();

        owner
            .store(
                &account_id,
                AccountSecretMaterial::new(
                    "provider:opaque-subject".into(),
                    Some("owner@example.com".into()),
                    None,
                    Some("access-token".into()),
                    Some("refresh-token".into()),
                    None,
                    None,
                )
                .unwrap(),
            )
            .unwrap();

        assert!(owner.load(&account_id).is_ok());
        assert!(matches!(
            other.load(&account_id),
            Err(VaultError::Unavailable)
        ));

        owner.remove(&account_id).unwrap();
    }

    #[test]
    fn account_credentials_never_create_a_plaintext_agent_env_file() {
        vak_config::paths::isolate_home_for_tests();
        let agent_id = format!("mailcal-no-plaintext-{}", Uuid::now_v7());
        let account_id = Uuid::now_v7().to_string();
        let vault = AccountVault::for_agent(&agent_id).unwrap();
        let agent_home = vak_config::paths::agent_home(&agent_id);
        vault
            .store(
                &account_id,
                AccountSecretMaterial::new(
                    "provider:opaque-subject".into(),
                    Some("owner@example.com".into()),
                    Some("public-client-id".into()),
                    Some("test-access-token".into()),
                    Some("test-refresh-token".into()),
                    None,
                    None,
                )
                .unwrap(),
            )
            .unwrap();

        assert!(vault.load(&account_id).is_ok());
        assert!(
            !agent_home.join(".env").exists(),
            "the .env path is only a credential-store scope hint"
        );
        vault.remove(&account_id).unwrap();
    }

    #[cfg(unix)]
    #[test]
    fn agent_vault_directories_are_private_and_symlinks_are_rejected() {
        use std::os::unix::fs::PermissionsExt;

        vak_config::paths::isolate_home_for_tests();
        let private_id = format!("mailcal-private-{}", Uuid::now_v7());
        let private_home = vak_config::paths::agent_home(&private_id);
        std::fs::create_dir_all(&private_home).unwrap();
        std::fs::set_permissions(&private_home, std::fs::Permissions::from_mode(0o755)).unwrap();
        AccountVault::for_agent(&private_id).unwrap();
        assert_eq!(
            std::fs::metadata(&private_home)
                .unwrap()
                .permissions()
                .mode()
                & 0o777,
            0o700
        );

        let linked_id = format!("mailcal-symlink-{}", Uuid::now_v7());
        let linked_home = vak_config::paths::agent_home(&linked_id);
        let target = vak_config::paths::agent_home(&format!("mailcal-target-{}", Uuid::now_v7()));
        std::fs::create_dir_all(&target).unwrap();
        std::os::unix::fs::symlink(&target, &linked_home).unwrap();
        assert!(matches!(
            AccountVault::for_agent(&linked_id),
            Err(VaultError::InvalidReference)
        ));
    }
}
