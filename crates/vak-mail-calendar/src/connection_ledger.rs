//! Append-only per-Agent account connection metadata.
//!
//! Credential bytes never enter this ledger. Callers pass the canonical
//! directory returned by vak_config::paths::agent_home; the existing
//! agents registry root covers this child ledger for backup and purge.

use crate::{AccountStatus, ConnectedAccount};
use chrono::{DateTime, Utc};
use fs2::FileExt;
use serde::{Deserialize, Serialize};
use std::fs::{self, OpenOptions};
use std::io::{Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};
use uuid::Uuid;

const MAX_LEDGER_BYTES: u64 = 16 * 1024 * 1024;
const MAX_RECORD_BYTES: u64 = 64 * 1024;

#[derive(Debug, thiserror::Error)]
pub enum LedgerError {
    #[error("mail and calendar connection ledger I/O failed")]
    Io(#[from] std::io::Error),
    #[error("mail and calendar connection ledger contains an invalid record")]
    InvalidRecord,
    #[error("mail and calendar connection ledger exceeded its size limit")]
    TooLarge,
    #[error("mail and calendar connection ledger could not encode a record")]
    Encode,
    #[error("mail and calendar connection ledger contains conflicting account state")]
    Conflict,
}

#[derive(Debug)]
pub enum ConditionalUpdateError<E> {
    Ledger(LedgerError),
    Conflict,
    Operation(E),
}

#[derive(Debug)]
pub enum ConditionalAppendError<E> {
    Ledger(LedgerError),
    Check(E),
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
#[serde(deny_unknown_fields)]
enum ConnectionEvent {
    Pending {
        record_id: String,
        account: ConnectedAccount,
    },
    Connected {
        record_id: String,
        account: ConnectedAccount,
    },
    Revoked {
        record_id: String,
        account_id: String,
        revoked_at: DateTime<Utc>,
    },
    ReauthenticationRequired {
        record_id: String,
        account_id: String,
        required_at: DateTime<Utc>,
    },
    Disconnected {
        record_id: String,
        account_id: String,
        provider: crate::Provider,
        fence: String,
        revoked_at: DateTime<Utc>,
    },
}

#[derive(Debug, Clone)]
pub struct ConnectionLedger {
    path: PathBuf,
    agent_id: String,
}

impl ConnectionLedger {
    /// Resolves storage from the authenticated Agent identity. The identifier
    /// is validated before it reaches the canonical path resolver.
    pub fn for_agent(agent_id: &str) -> Result<Self, LedgerError> {
        if agent_id.is_empty()
            || !agent_id
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-' || byte == b'_')
        {
            return Err(LedgerError::InvalidRecord);
        }
        let agent_home = vak_config::paths::agent_home(agent_id);
        Ok(Self {
            path: agent_home.join("mail-calendar").join("connections.jsonl"),
            agent_id: agent_id.to_owned(),
        })
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    pub fn read_all(&self) -> Result<Vec<ConnectedAccount>, LedgerError> {
        validate_agent_path(self.agent_id.as_str())?;
        let Some(parent) = self.path.parent() else {
            return Err(LedgerError::InvalidRecord);
        };
        match fs::symlink_metadata(parent) {
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                return Ok(Vec::new());
            }
            Err(error) => return Err(LedgerError::Io(error)),
            Ok(metadata) if metadata.file_type().is_symlink() || !metadata.is_dir() => {
                return Err(LedgerError::InvalidRecord);
            }
            Ok(_) => {}
        }
        let metadata = match fs::symlink_metadata(&self.path) {
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                return Ok(Vec::new());
            }
            Err(error) => return Err(LedgerError::Io(error)),
            Ok(metadata) => metadata,
        };
        if metadata.file_type().is_symlink() || !metadata.is_file() {
            return Err(LedgerError::InvalidRecord);
        }
        if metadata.len() > MAX_LEDGER_BYTES {
            return Err(LedgerError::TooLarge);
        }
        let mut options = OpenOptions::new();
        options.read(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.custom_flags(libc::O_NOFOLLOW);
        }
        let mut file = match options.open(&self.path) {
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                return Ok(Vec::new());
            }
            Err(error) => return Err(LedgerError::Io(error)),
            Ok(file) => file,
        };
        let opened_metadata = file.metadata()?;
        if !opened_metadata.is_file() {
            return Err(LedgerError::InvalidRecord);
        }
        if opened_metadata.len() > MAX_LEDGER_BYTES {
            return Err(LedgerError::TooLarge);
        }
        file.lock_shared()?;
        let mut bytes = Vec::with_capacity(opened_metadata.len() as usize);
        {
            let mut limited = (&mut file).take(MAX_LEDGER_BYTES.saturating_add(1));
            limited.read_to_end(&mut bytes)?;
        }
        FileExt::unlock(&file)?;
        if bytes.len() as u64 > MAX_LEDGER_BYTES {
            return Err(LedgerError::TooLarge);
        }
        let accounts = self.decode_state(&bytes)?;
        Ok(accounts.into_values().collect())
    }

    pub fn provider_fence(&self, provider: crate::Provider) -> Result<String, LedgerError> {
        let _ = self.read_all()?;
        if !self.path.exists() {
            return Ok(String::new());
        }
        let mut options = OpenOptions::new();
        options.read(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.custom_flags(libc::O_NOFOLLOW);
        }
        let file = options.open(&self.path)?;
        file.lock_shared()?;
        let mut bytes = Vec::new();
        (&file)
            .take(MAX_LEDGER_BYTES.saturating_add(1))
            .read_to_end(&mut bytes)?;
        FileExt::unlock(&file)?;
        if bytes.len() as u64 > MAX_LEDGER_BYTES {
            return Err(LedgerError::TooLarge);
        }
        provider_fence_in_bytes(&bytes, provider)
    }

    pub fn append_disconnected(
        &self,
        account_id: &str,
        provider: crate::Provider,
        revoked_at: DateTime<Utc>,
    ) -> Result<(), LedgerError> {
        let event = ConnectionEvent::Disconnected {
            record_id: Uuid::now_v7().to_string(),
            account_id: account_id.to_owned(),
            provider,
            fence: Uuid::now_v7().to_string(),
            revoked_at,
        };
        self.append_with_check(&event, |accounts| {
            if !accounts.iter().any(|account| {
                account.id == account_id
                    && account.provider == provider
                    && account.revoked_at.is_none()
            }) {
                return Err(LedgerError::Conflict);
            }
            Ok::<(), LedgerError>(())
        })
        .map_err(|error| match error {
            ConditionalAppendError::Ledger(error) | ConditionalAppendError::Check(error) => error,
        })
    }

    pub fn append_pending(&self, account: ConnectedAccount) -> Result<(), LedgerError> {
        if !valid_account_record(&account, &self.agent_id)
            || account.status != AccountStatus::Pending
            || account.revision != 1
        {
            return Err(LedgerError::InvalidRecord);
        }
        self.append(&ConnectionEvent::Pending {
            record_id: Uuid::now_v7().to_string(),
            account,
        })
    }

    /// Append a pending link only after checking the current ledger while
    /// holding its cross-process exclusive lock. The check and append are one
    /// transaction, so parallel server processes cannot both admit a link
    /// that conflicts with current account state.
    pub fn append_pending_if<E>(
        &self,
        account: ConnectedAccount,
        check: impl FnOnce(&[ConnectedAccount]) -> Result<(), E>,
    ) -> Result<(), ConditionalAppendError<E>> {
        if !valid_account_record(&account, &self.agent_id)
            || account.status != AccountStatus::Pending
            || account.revision != 1
        {
            return Err(ConditionalAppendError::Ledger(LedgerError::InvalidRecord));
        }
        self.append_with_check(
            &ConnectionEvent::Pending {
                record_id: Uuid::now_v7().to_string(),
                account,
            },
            check,
        )
    }

    pub fn append_connected(&self, account: ConnectedAccount) -> Result<(), LedgerError> {
        if !valid_account_record(&account, &self.agent_id)
            || account.status != AccountStatus::Connected
        {
            return Err(LedgerError::InvalidRecord);
        }
        self.append(&ConnectionEvent::Connected {
            record_id: Uuid::now_v7().to_string(),
            account,
        })
    }

    /// Serialize a credential-side update with account state transitions in
    /// every process sharing this ledger. The operation runs only while the
    /// latest account revision is still connected, and while the exclusive
    /// ledger lock prevents disconnect from racing the credential write.
    pub fn append_connected_if_current<E>(
        &self,
        account: ConnectedAccount,
        expected_revision: u64,
        operation: impl FnOnce() -> Result<(), E>,
    ) -> Result<(), ConditionalUpdateError<E>> {
        self.append_connected_if_state(
            account,
            AccountStatus::Connected,
            expected_revision,
            None,
            operation,
        )
    }

    /// Atomically activate a pending link with its credential-side operation.
    /// A disconnect in another process either wins before the credential write,
    /// or runs after activation and can remove the saved credential.
    pub fn append_connected_if_pending<E>(
        &self,
        account: ConnectedAccount,
        operation: impl FnOnce() -> Result<(), E>,
    ) -> Result<(), ConditionalUpdateError<E>> {
        self.append_connected_if_state(account, AccountStatus::Pending, 1, None, operation)
    }

    pub fn append_connected_if_pending_fenced<E>(
        &self,
        account: ConnectedAccount,
        provider: crate::Provider,
        expected_fence: &str,
        operation: impl FnOnce() -> Result<(), E>,
    ) -> Result<(), ConditionalUpdateError<E>> {
        if account.status != AccountStatus::Connected {
            return Err(ConditionalUpdateError::Ledger(LedgerError::InvalidRecord));
        }
        self.append_connected_if_state(
            account,
            AccountStatus::Pending,
            1,
            Some((provider, expected_fence)),
            operation,
        )
    }

    fn append_connected_if_state<E>(
        &self,
        account: ConnectedAccount,
        expected_status: AccountStatus,
        expected_revision: u64,
        expected_fence: Option<(crate::Provider, &str)>,
        operation: impl FnOnce() -> Result<(), E>,
    ) -> Result<(), ConditionalUpdateError<E>> {
        let valid_status = (expected_status == AccountStatus::Pending
            && matches!(
                account.status,
                AccountStatus::Connected | AccountStatus::ConnectedUnverified
            ))
            || (expected_status == AccountStatus::Connected
                && account.status == AccountStatus::Connected);
        if !valid_account_record(&account, &self.agent_id)
            || !valid_status
            || expected_revision.checked_add(1) != Some(account.revision)
        {
            return Err(ConditionalUpdateError::Ledger(LedgerError::InvalidRecord));
        }
        validate_agent_path(self.agent_id.as_str()).map_err(ConditionalUpdateError::Ledger)?;
        let parent = self
            .path
            .parent()
            .ok_or(ConditionalUpdateError::Ledger(LedgerError::InvalidRecord))?;
        create_private_dir(parent)
            .map_err(|error| ConditionalUpdateError::Ledger(LedgerError::Io(error)))?;
        reject_symlink(parent).map_err(ConditionalUpdateError::Ledger)?;
        if self.path.exists() {
            let metadata = fs::symlink_metadata(&self.path)
                .map_err(|error| ConditionalUpdateError::Ledger(LedgerError::Io(error)))?;
            if metadata.file_type().is_symlink() || !metadata.is_file() {
                return Err(ConditionalUpdateError::Ledger(LedgerError::InvalidRecord));
            }
        }

        let mut options = OpenOptions::new();
        options.create(true).append(true).read(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(0o600);
            options.custom_flags(libc::O_NOFOLLOW);
        }
        let mut file = options
            .open(&self.path)
            .map_err(|error| ConditionalUpdateError::Ledger(LedgerError::Io(error)))?;
        file.lock_exclusive()
            .map_err(|error| ConditionalUpdateError::Ledger(LedgerError::Io(error)))?;
        file.seek(SeekFrom::Start(0))
            .map_err(|error| ConditionalUpdateError::Ledger(LedgerError::Io(error)))?;
        let mut current = Vec::new();
        {
            let mut limited = (&mut file).take(MAX_LEDGER_BYTES.saturating_add(1));
            limited
                .read_to_end(&mut current)
                .map_err(|error| ConditionalUpdateError::Ledger(LedgerError::Io(error)))?;
        }
        if current.len() as u64 > MAX_LEDGER_BYTES {
            return Err(ConditionalUpdateError::Ledger(LedgerError::TooLarge));
        }
        if let Some((provider, expected)) = expected_fence {
            let current_fence = provider_fence_in_bytes(&current, provider)
                .map_err(ConditionalUpdateError::Ledger)?;
            if current_fence != expected {
                return Err(ConditionalUpdateError::Conflict);
            }
        }
        let mut accounts = self
            .decode_state(&current)
            .map_err(ConditionalUpdateError::Ledger)?;
        let Some(latest) = accounts.get(&account.id) else {
            return Err(ConditionalUpdateError::Conflict);
        };
        if latest.status != expected_status
            || latest.revoked_at.is_some()
            || latest.revision != expected_revision
            || !same_authority(latest, &account)
        {
            return Err(ConditionalUpdateError::Conflict);
        }

        let event = ConnectionEvent::Connected {
            record_id: Uuid::now_v7().to_string(),
            account,
        };
        self.apply_event(&mut accounts, &event)
            .map_err(ConditionalUpdateError::Ledger)?;
        let line = serde_json::to_vec(&event)
            .map_err(|_| ConditionalUpdateError::Ledger(LedgerError::Encode))?;
        let current_size = file
            .metadata()
            .map_err(|error| ConditionalUpdateError::Ledger(LedgerError::Io(error)))?
            .len();
        if line.len() as u64 > MAX_RECORD_BYTES
            || current_size.saturating_add(line.len() as u64 + 1) > MAX_LEDGER_BYTES
        {
            return Err(ConditionalUpdateError::Ledger(LedgerError::TooLarge));
        }

        operation().map_err(ConditionalUpdateError::Operation)?;
        file.seek(SeekFrom::End(0))
            .map_err(|error| ConditionalUpdateError::Ledger(LedgerError::Io(error)))?;
        file.write_all(&line)
            .and_then(|_| file.write_all(b"\n"))
            .and_then(|_| file.sync_data())
            .map_err(|error| ConditionalUpdateError::Ledger(LedgerError::Io(error)))?;
        FileExt::unlock(&file)
            .map_err(|error| ConditionalUpdateError::Ledger(LedgerError::Io(error)))?;
        Ok(())
    }

    pub fn append_revoked(
        &self,
        account_id: &str,
        revoked_at: DateTime<Utc>,
    ) -> Result<(), LedgerError> {
        let exists = self
            .read_all()?
            .iter()
            .any(|account| account.id == account_id && account.revoked_at.is_none());
        if !exists {
            return Err(LedgerError::Conflict);
        }
        self.append(&ConnectionEvent::Revoked {
            record_id: Uuid::now_v7().to_string(),
            account_id: account_id.to_owned(),
            revoked_at,
        })
    }

    /// Records that a provider refused token refresh and this account needs a
    /// fresh owner sign-in. The transition immediately removes its admission.
    pub fn append_reauthentication_required(
        &self,
        account_id: &str,
        required_at: DateTime<Utc>,
    ) -> Result<(), LedgerError> {
        self.append(&ConnectionEvent::ReauthenticationRequired {
            record_id: Uuid::now_v7().to_string(),
            account_id: account_id.to_owned(),
            required_at,
        })
    }

    fn append(&self, event: &ConnectionEvent) -> Result<(), LedgerError> {
        match self.append_with_check(event, |_| Ok::<(), std::convert::Infallible>(())) {
            Ok(()) => Ok(()),
            Err(ConditionalAppendError::Ledger(error)) => Err(error),
            Err(ConditionalAppendError::Check(never)) => match never {},
        }
    }

    fn append_with_check<E>(
        &self,
        event: &ConnectionEvent,
        check: impl FnOnce(&[ConnectedAccount]) -> Result<(), E>,
    ) -> Result<(), ConditionalAppendError<E>> {
        validate_agent_path(self.agent_id.as_str()).map_err(ConditionalAppendError::Ledger)?;
        let parent = self
            .path
            .parent()
            .ok_or(ConditionalAppendError::Ledger(LedgerError::InvalidRecord))?;
        create_private_dir(parent)
            .map_err(|error| ConditionalAppendError::Ledger(LedgerError::Io(error)))?;
        reject_symlink(parent).map_err(ConditionalAppendError::Ledger)?;
        if self.path.exists() {
            let metadata = fs::symlink_metadata(&self.path)
                .map_err(|error| ConditionalAppendError::Ledger(LedgerError::Io(error)))?;
            if metadata.file_type().is_symlink() || !metadata.is_file() {
                return Err(ConditionalAppendError::Ledger(LedgerError::InvalidRecord));
            }
        }

        let mut options = OpenOptions::new();
        options.create(true).append(true).read(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(0o600);
            options.custom_flags(libc::O_NOFOLLOW);
        }
        let mut file = options
            .open(&self.path)
            .map_err(|error| ConditionalAppendError::Ledger(LedgerError::Io(error)))?;
        file.lock_exclusive()
            .map_err(|error| ConditionalAppendError::Ledger(LedgerError::Io(error)))?;
        file.seek(SeekFrom::Start(0))
            .map_err(|error| ConditionalAppendError::Ledger(LedgerError::Io(error)))?;
        let mut current = Vec::new();
        file.read_to_end(&mut current)
            .map_err(|error| ConditionalAppendError::Ledger(LedgerError::Io(error)))?;
        let mut accounts = self
            .decode_state(&current)
            .map_err(ConditionalAppendError::Ledger)?;
        let visible_accounts = accounts.values().cloned().collect::<Vec<_>>();
        if let Err(error) = check(&visible_accounts) {
            let _ = FileExt::unlock(&file);
            return Err(ConditionalAppendError::Check(error));
        }
        self.apply_event(&mut accounts, event)
            .map_err(ConditionalAppendError::Ledger)?;
        file.seek(SeekFrom::End(0))
            .map_err(|error| ConditionalAppendError::Ledger(LedgerError::Io(error)))?;
        let line = serde_json::to_vec(event)
            .map_err(|_| ConditionalAppendError::Ledger(LedgerError::Encode))?;
        if line.len() as u64 > MAX_RECORD_BYTES
            || file
                .metadata()
                .map_err(|error| ConditionalAppendError::Ledger(LedgerError::Io(error)))?
                .len()
                .saturating_add(line.len() as u64 + 1)
                > MAX_LEDGER_BYTES
        {
            let _ = FileExt::unlock(&file);
            return Err(ConditionalAppendError::Ledger(LedgerError::TooLarge));
        }
        file.write_all(&line)
            .and_then(|_| file.write_all(b"\n"))
            .and_then(|_| file.sync_data())
            .map_err(|error| ConditionalAppendError::Ledger(LedgerError::Io(error)))?;
        FileExt::unlock(&file)
            .map_err(|error| ConditionalAppendError::Ledger(LedgerError::Io(error)))?;
        Ok(())
    }

    fn decode_state(
        &self,
        bytes: &[u8],
    ) -> Result<std::collections::BTreeMap<String, ConnectedAccount>, LedgerError> {
        if bytes.len() as u64 > MAX_LEDGER_BYTES {
            return Err(LedgerError::TooLarge);
        }
        let mut accounts = std::collections::BTreeMap::<String, ConnectedAccount>::new();
        for line in bytes.split(|byte| *byte == b'\n') {
            if line.is_empty() {
                continue;
            }
            if line.len() as u64 > MAX_RECORD_BYTES {
                return Err(LedgerError::TooLarge);
            }
            let event: ConnectionEvent =
                serde_json::from_slice(line).map_err(|_| LedgerError::InvalidRecord)?;
            self.apply_event(&mut accounts, &event)?;
        }
        Ok(accounts)
    }

    fn apply_event(
        &self,
        accounts: &mut std::collections::BTreeMap<String, ConnectedAccount>,
        event: &ConnectionEvent,
    ) -> Result<(), LedgerError> {
        match event {
            ConnectionEvent::Pending { record_id, account } => {
                if !is_uuid_v7(record_id)
                    || !valid_account_record(account, &self.agent_id)
                    || account.status != AccountStatus::Pending
                    || account.revision != 1
                {
                    return Err(LedgerError::InvalidRecord);
                }
                if accounts.contains_key(&account.id) {
                    return Err(LedgerError::Conflict);
                }
                accounts.insert(account.id.clone(), account.clone());
            }
            ConnectionEvent::Connected { record_id, account } => {
                if !is_uuid_v7(record_id)
                    || !valid_account_record(account, &self.agent_id)
                    || !matches!(
                        account.status,
                        AccountStatus::Connected | AccountStatus::ConnectedUnverified
                    )
                {
                    return Err(LedgerError::InvalidRecord);
                }
                let Some(current) = accounts.get(&account.id) else {
                    return Err(LedgerError::Conflict);
                };
                if current.revoked_at.is_some()
                    || current.revision.checked_add(1) != Some(account.revision)
                    || !same_authority(current, account)
                    || !matches!(
                        (current.status, account.status),
                        (AccountStatus::Pending, AccountStatus::Connected)
                            | (AccountStatus::Pending, AccountStatus::ConnectedUnverified)
                            | (AccountStatus::Connected, AccountStatus::Connected)
                    )
                {
                    return Err(LedgerError::Conflict);
                }
                accounts.insert(account.id.clone(), account.clone());
            }
            ConnectionEvent::Revoked {
                record_id,
                account_id,
                revoked_at,
            } => {
                if !is_uuid_v7(record_id) || !is_uuid_v7(account_id) {
                    return Err(LedgerError::InvalidRecord);
                }
                let Some(account) = accounts.get_mut(account_id) else {
                    return Err(LedgerError::Conflict);
                };
                if account.revoked_at.is_some() {
                    return Err(LedgerError::Conflict);
                }
                account.revoked_at = Some(*revoked_at);
                account.revision = account.revision.saturating_add(1);
            }
            ConnectionEvent::ReauthenticationRequired {
                record_id,
                account_id,
                required_at: _,
            } => {
                if !is_uuid_v7(record_id) || !is_uuid_v7(account_id) {
                    return Err(LedgerError::InvalidRecord);
                }
                let Some(account) = accounts.get_mut(account_id) else {
                    return Err(LedgerError::Conflict);
                };
                if account.status != AccountStatus::Connected || account.revoked_at.is_some() {
                    return Err(LedgerError::Conflict);
                }
                let Some(revision) = account.revision.checked_add(1) else {
                    return Err(LedgerError::Conflict);
                };
                account.status = AccountStatus::ReauthenticationRequired;
                account.revision = revision;
            }
            ConnectionEvent::Disconnected {
                record_id,
                account_id,
                fence,
                provider,
                revoked_at,
                ..
            } => {
                if !is_uuid_v7(record_id) || !is_uuid_v7(fence) || !is_uuid_v7(account_id) {
                    return Err(LedgerError::InvalidRecord);
                }
                let Some(account) = accounts.get_mut(account_id) else {
                    return Err(LedgerError::Conflict);
                };
                if account.provider != *provider || account.revoked_at.is_some() {
                    return Err(LedgerError::Conflict);
                }
                account.revoked_at = Some(*revoked_at);
                account.revision = account.revision.saturating_add(1);
            }
        }
        Ok(())
    }
}

fn provider_fence_in_bytes(bytes: &[u8], provider: crate::Provider) -> Result<String, LedgerError> {
    let mut fence = String::new();
    for line in bytes
        .split(|byte| *byte == b'\n')
        .filter(|line| !line.is_empty())
    {
        let event: ConnectionEvent =
            serde_json::from_slice(line).map_err(|_| LedgerError::InvalidRecord)?;
        if let ConnectionEvent::Disconnected {
            record_id,
            account_id,
            provider: event_provider,
            fence: event_fence,
            ..
        } = event
        {
            if !is_uuid_v7(&record_id) || !is_uuid_v7(&account_id) || !is_uuid_v7(&event_fence) {
                return Err(LedgerError::InvalidRecord);
            }
            if event_provider == provider {
                fence = event_fence;
            }
        }
    }
    Ok(fence)
}

fn valid_account_record(account: &ConnectedAccount, agent_id: &str) -> bool {
    is_uuid_v7(&account.id)
        && account.owner_agent_id == agent_id
        && (account.status != AccountStatus::ConnectedUnverified
            || account.provider == crate::Provider::AppleIcloud)
        && crate::vault::AccountVault::credential_ref(&account.id)
            .ok()
            .as_deref()
            == Some(account.credential_ref.as_str())
        && account.principal_ref == account.credential_ref
        && account.revoked_at.is_none()
        && account.revision > 0
        && !account.allowed_audiences.is_empty()
        && account.allowed_audiences.iter().all(|audience| {
            !audience.trim().is_empty()
                && audience.len() <= 256
                && !audience.chars().any(char::is_control)
        })
        && (account.provider != crate::Provider::AppleIcloud || account.provider_scopes.is_empty())
        && (account.provider == crate::Provider::AppleIcloud || !account.provider_scopes.is_empty())
        && account.provider_scopes.len() <= 16
        && account.provider_scopes.iter().all(|scope| {
            !scope.trim().is_empty() && scope.len() <= 512 && !scope.chars().any(char::is_control)
        })
        && crate::oauth::account_grants_are_valid(
            account.provider,
            &account.capabilities.iter().copied().collect::<Vec<_>>(),
            &account.provider_scopes,
        )
        && account
            .access_token_expires_at
            .is_none_or(|expires| expires > account.connected_at)
}

/// Revisions may update token-expiry metadata, but never widen or redirect
/// the original account grant. A changed consent requires a fresh account id.
fn same_authority(current: &ConnectedAccount, next: &ConnectedAccount) -> bool {
    current.id == next.id
        && current.provider == next.provider
        && current.owner_agent_id == next.owner_agent_id
        && current.allowed_audiences == next.allowed_audiences
        && current.capabilities == next.capabilities
        && current.provider_scopes == next.provider_scopes
        && current.credential_ref == next.credential_ref
        && current.principal_ref == next.principal_ref
        && current.refresh_token_available == next.refresh_token_available
        && current.connected_at == next.connected_at
}

fn validate_agent_path(agent_id: &str) -> Result<(), LedgerError> {
    let home = vak_config::paths::agent_home(agent_id);
    let Some(agents_dir) = home.parent() else {
        return Err(LedgerError::InvalidRecord);
    };
    let Some(data_home) = agents_dir.parent() else {
        return Err(LedgerError::InvalidRecord);
    };
    for path in [data_home, agents_dir, home.as_path()] {
        match fs::symlink_metadata(path) {
            Ok(metadata) if metadata.file_type().is_symlink() || !metadata.is_dir() => {
                return Err(LedgerError::InvalidRecord);
            }
            Ok(_) => {}
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => return Err(LedgerError::Io(error)),
        }
    }
    Ok(())
}

fn create_private_dir(path: &Path) -> Result<(), std::io::Error> {
    // Check the final component before create_dir_all: following an existing
    // symlink here could create directories outside the Agent home before the
    // later post-creation check rejects it.
    match fs::symlink_metadata(path) {
        Ok(metadata) if metadata.file_type().is_symlink() || !metadata.is_dir() => {
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidInput,
                "mail/calendar storage directory is not a real directory",
            ));
        }
        Ok(_) => {}
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(error) => return Err(error),
    }
    fs::create_dir_all(path)?;
    let metadata = fs::symlink_metadata(path)?;
    if metadata.file_type().is_symlink() || !metadata.is_dir() {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            "mail/calendar storage directory is not a real directory",
        ));
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(path, fs::Permissions::from_mode(0o700))?;
    }
    Ok(())
}

fn reject_symlink(path: &Path) -> Result<(), LedgerError> {
    let metadata = fs::symlink_metadata(path)?;
    if metadata.file_type().is_symlink() || !metadata.is_dir() {
        return Err(LedgerError::InvalidRecord);
    }
    Ok(())
}

fn is_uuid_v7(value: &str) -> bool {
    Uuid::parse_str(value).is_ok_and(|id| id.get_version_num() == 7)
}

#[cfg(test)]
mod tests {
    use super::{
        ConditionalAppendError, ConditionalUpdateError, ConnectionEvent, ConnectionLedger,
        LedgerError, valid_account_record,
    };
    use crate::{AccountStatus, Capability, ConnectedAccount, Provider};
    use chrono::Utc;
    use std::collections::{BTreeMap, BTreeSet};
    use uuid::Uuid;

    fn connected_account(agent_id: &str, account_id: &str, revision: u64) -> ConnectedAccount {
        ConnectedAccount {
            id: account_id.to_owned(),
            provider: Provider::Google,
            status: AccountStatus::Connected,
            owner_agent_id: agent_id.to_owned(),
            allowed_audiences: BTreeSet::from(["owner".to_owned()]),
            capabilities: BTreeSet::from([Capability::MailRead]),
            provider_scopes: BTreeSet::from([
                "openid".to_owned(),
                "email".to_owned(),
                "https://www.googleapis.com/auth/gmail.readonly".to_owned(),
            ]),
            credential_ref: crate::vault::AccountVault::credential_ref(account_id).unwrap(),
            principal_ref: crate::vault::AccountVault::credential_ref(account_id).unwrap(),
            revision,
            connected_at: Utc::now(),
            access_token_expires_at: None,
            refresh_token_available: true,
            revoked_at: None,
        }
    }

    fn add_connected_account(
        ledger: &ConnectionLedger,
        accounts: &mut BTreeMap<String, ConnectedAccount>,
        mut account: ConnectedAccount,
    ) {
        account.status = AccountStatus::Pending;
        account.revision = 1;
        ledger
            .apply_event(
                accounts,
                &ConnectionEvent::Pending {
                    record_id: Uuid::now_v7().to_string(),
                    account: account.clone(),
                },
            )
            .unwrap();
        account.status = AccountStatus::Connected;
        account.revision = 2;
        ledger
            .apply_event(
                accounts,
                &ConnectionEvent::Connected {
                    record_id: Uuid::now_v7().to_string(),
                    account,
                },
            )
            .unwrap();
    }

    #[test]
    fn pending_link_check_and_append_are_atomic_across_threads() {
        vak_config::paths::isolate_home_for_tests();
        let agent_id = format!("mailcal-pending-race-{}", Uuid::now_v7());
        let ledger = ConnectionLedger::for_agent(&agent_id).unwrap();
        let barrier = std::sync::Arc::new(std::sync::Barrier::new(3));
        let workers = (0..2)
            .map(|_| {
                let ledger = ledger.clone();
                let barrier = barrier.clone();
                let agent_id = agent_id.clone();
                std::thread::spawn(move || {
                    let account_id = Uuid::now_v7().to_string();
                    let mut account = connected_account(&agent_id, &account_id, 1);
                    account.status = AccountStatus::Pending;
                    barrier.wait();
                    ledger.append_pending_if(account, |existing| {
                        if existing.iter().any(|account| {
                            account.provider == Provider::Google && account.revoked_at.is_none()
                        }) {
                            Err(())
                        } else {
                            Ok(())
                        }
                    })
                })
            })
            .collect::<Vec<_>>();
        barrier.wait();

        let results = workers
            .into_iter()
            .map(|worker| worker.join().unwrap())
            .collect::<Vec<_>>();
        assert_eq!(results.iter().filter(|result| result.is_ok()).count(), 1);
        assert_eq!(
            results
                .iter()
                .filter(|result| matches!(result, Err(ConditionalAppendError::Check(()))))
                .count(),
            1
        );
        assert_eq!(ledger.read_all().unwrap().len(), 1);
    }

    #[test]
    fn ledger_event_schema_rejects_unexpected_secret_fields() {
        let encoded = serde_json::json!({
            "kind": "revoked",
            "record_id": Uuid::now_v7().to_string(),
            "account_id": Uuid::now_v7().to_string(),
            "revoked_at": chrono::Utc::now(),
            "access_token": "must-not-be-metadata"
        });

        assert!(serde_json::from_value::<ConnectionEvent>(encoded).is_err());
    }

    #[test]
    fn pending_account_is_visible_but_cannot_be_activated_without_its_vault_phase() {
        let agent_id = "agent-1";
        let account_id = Uuid::now_v7().to_string();
        let ledger = ConnectionLedger {
            path: std::path::PathBuf::new(),
            agent_id: agent_id.to_owned(),
        };
        let mut accounts = BTreeMap::new();
        let mut pending = connected_account(agent_id, &account_id, 1);
        pending.status = AccountStatus::Pending;
        pending.revision = 1;
        ledger
            .apply_event(
                &mut accounts,
                &ConnectionEvent::Pending {
                    record_id: Uuid::now_v7().to_string(),
                    account: pending.clone(),
                },
            )
            .unwrap();

        assert_eq!(accounts.get(&account_id), Some(&pending));
        assert!(!pending.admits("agent-1", "owner", Capability::MailRead));

        let mut connected = pending.clone();
        connected.status = AccountStatus::Connected;
        connected.revision = 2;
        ledger
            .apply_event(
                &mut accounts,
                &ConnectionEvent::Connected {
                    record_id: Uuid::now_v7().to_string(),
                    account: connected,
                },
            )
            .unwrap();
        assert_eq!(accounts[&account_id].status, AccountStatus::Connected);
    }

    #[test]
    fn connection_ledger_rejects_activation_without_a_pending_record() {
        let agent_id = "agent-1";
        let account_id = Uuid::now_v7().to_string();
        let ledger = ConnectionLedger {
            path: std::path::PathBuf::new(),
            agent_id: agent_id.to_owned(),
        };
        let mut accounts = BTreeMap::new();
        assert!(matches!(
            ledger.apply_event(
                &mut accounts,
                &ConnectionEvent::Connected {
                    record_id: Uuid::now_v7().to_string(),
                    account: connected_account(agent_id, &account_id, 1),
                },
            ),
            Err(LedgerError::Conflict)
        ));
    }

    #[test]
    fn revoked_account_identity_cannot_be_reconnected() {
        let agent_id = "agent-1";
        let account_id = Uuid::now_v7().to_string();
        let ledger = ConnectionLedger {
            path: std::path::PathBuf::new(),
            agent_id: agent_id.to_owned(),
        };
        let mut accounts = BTreeMap::new();
        add_connected_account(
            &ledger,
            &mut accounts,
            connected_account(agent_id, &account_id, 2),
        );
        ledger
            .apply_event(
                &mut accounts,
                &ConnectionEvent::Revoked {
                    record_id: Uuid::now_v7().to_string(),
                    account_id: account_id.clone(),
                    revoked_at: Utc::now(),
                },
            )
            .unwrap();

        assert!(matches!(
            ledger.apply_event(
                &mut accounts,
                &ConnectionEvent::Connected {
                    record_id: Uuid::now_v7().to_string(),
                    account: connected_account(agent_id, &account_id, 4),
                },
            ),
            Err(LedgerError::Conflict)
        ));
        assert!(accounts[&account_id].revoked_at.is_some());
    }

    #[test]
    fn reauthentication_required_fences_account_admission_and_is_append_only() {
        vak_config::paths::isolate_home_for_tests();
        let agent_id = "agent-1";
        let account_id = Uuid::now_v7().to_string();
        let vault = crate::vault::AccountVault::for_agent(agent_id).unwrap();
        let ledger = ConnectionLedger::for_agent(agent_id).unwrap();
        let mut account = connected_account(agent_id, &account_id, 1);
        account.status = AccountStatus::Pending;
        account.revision = 1;
        vault
            .store(
                &account_id,
                crate::vault::AccountSecretMaterial::new(
                    "google:subject".into(),
                    Some("person@example.com".into()),
                    Some("test-client".into()),
                    Some("access-token".into()),
                    Some("refresh-token".into()),
                    None,
                    None,
                )
                .unwrap(),
            )
            .unwrap();
        ledger.append_pending(account.clone()).unwrap();
        account.status = AccountStatus::Connected;
        account.revision = 2;
        ledger.append_connected(account).unwrap();
        ledger
            .append_reauthentication_required(&account_id, Utc::now())
            .unwrap();
        let reopened = ConnectionLedger::for_agent(agent_id).unwrap();
        let saved = reopened.read_all().unwrap();
        let saved_account = saved
            .iter()
            .find(|account| account.id == account_id)
            .unwrap();
        assert_eq!(
            saved_account.status,
            AccountStatus::ReauthenticationRequired
        );
        assert!(!saved_account.admits("agent-1", "owner", Capability::MailRead));

        let mut attempted_reactivation = saved_account.clone();
        attempted_reactivation.status = AccountStatus::Connected;
        attempted_reactivation.revision += 1;
        let mut accounts = reopened
            .decode_state(&std::fs::read(reopened.path()).unwrap())
            .unwrap();
        assert!(matches!(
            ledger.apply_event(
                &mut accounts,
                &ConnectionEvent::Connected {
                    record_id: Uuid::now_v7().to_string(),
                    account: attempted_reactivation,
                },
            ),
            Err(LedgerError::Conflict)
        ));
        vault.remove(&account_id).unwrap();
    }

    #[test]
    fn conditional_refresh_commit_skips_vault_write_after_disconnect() {
        vak_config::paths::isolate_home_for_tests();
        let agent_id = format!("mailcal-conditional-{}", Uuid::now_v7());
        let account_id = Uuid::now_v7().to_string();
        let ledger = ConnectionLedger::for_agent(&agent_id).unwrap();
        let mut account = connected_account(&agent_id, &account_id, 1);
        account.status = AccountStatus::Pending;
        account.revision = 1;
        ledger.append_pending(account.clone()).unwrap();
        account.status = AccountStatus::Connected;
        account.revision = 2;
        ledger.append_connected(account.clone()).unwrap();

        let mut refreshed = account.clone();
        refreshed.revision = 3;
        refreshed.access_token_expires_at = Some(Utc::now() + chrono::Duration::hours(1));
        let mut credential_written = false;
        ledger
            .append_connected_if_current(refreshed, 2, || {
                credential_written = true;
                Ok::<_, ()>(())
            })
            .unwrap();
        assert!(credential_written);

        ledger.append_revoked(&account_id, Utc::now()).unwrap();
        let mut stale_refresh = account;
        stale_refresh.revision = 4;
        stale_refresh.access_token_expires_at = Some(Utc::now() + chrono::Duration::hours(2));
        credential_written = false;
        assert!(matches!(
            ledger.append_connected_if_current(stale_refresh, 3, || {
                credential_written = true;
                Ok::<_, ()>(())
            }),
            Err(ConditionalUpdateError::Conflict)
        ));
        assert!(!credential_written);
        let latest = ledger.read_all().unwrap().remove(0);
        assert!(latest.revoked_at.is_some());
        assert_eq!(latest.revision, 4);
    }

    #[test]
    fn conditional_link_commit_skips_vault_write_after_disconnect() {
        vak_config::paths::isolate_home_for_tests();
        let agent_id = format!("mailcal-link-race-{}", Uuid::now_v7());
        let account_id = Uuid::now_v7().to_string();
        let ledger = ConnectionLedger::for_agent(&agent_id).unwrap();
        let mut pending = connected_account(&agent_id, &account_id, 1);
        pending.status = AccountStatus::Pending;
        ledger.append_pending(pending.clone()).unwrap();
        ledger.append_revoked(&account_id, Utc::now()).unwrap();

        let mut activated = pending;
        activated.status = AccountStatus::Connected;
        activated.revision = 2;
        let mut credential_written = false;
        assert!(matches!(
            ledger.append_connected_if_pending(activated, || {
                credential_written = true;
                Ok::<_, ()>(())
            }),
            Err(ConditionalUpdateError::Conflict)
        ));
        assert!(!credential_written);
        let latest = ledger.read_all().unwrap().remove(0);
        assert!(latest.revoked_at.is_some());
        assert_eq!(latest.revision, 2);
    }

    #[test]
    fn provider_disconnect_fence_blocks_callback_across_ledger_handles() {
        vak_config::paths::isolate_home_for_tests();
        let agent_id = format!("mailcal-fence-{}", Uuid::now_v7());
        let initiating_process = ConnectionLedger::for_agent(&agent_id).unwrap();
        let disconnecting_process = ConnectionLedger::for_agent(&agent_id).unwrap();

        let owner_id = Uuid::now_v7().to_string();
        let mut owner_pending = connected_account(&agent_id, &owner_id, 1);
        owner_pending.status = AccountStatus::Pending;
        initiating_process
            .append_pending(owner_pending.clone())
            .unwrap();
        let mut owner_connected = owner_pending;
        owner_connected.status = AccountStatus::Connected;
        owner_connected.revision = 2;
        initiating_process
            .append_connected_if_pending(owner_connected, || Ok::<(), ()>(()))
            .unwrap();

        let callback_fence = initiating_process.provider_fence(Provider::Google).unwrap();
        let callback_id = Uuid::now_v7().to_string();
        let mut callback_pending = connected_account(&agent_id, &callback_id, 1);
        callback_pending.status = AccountStatus::Pending;
        initiating_process
            .append_pending(callback_pending.clone())
            .unwrap();

        disconnecting_process
            .append_disconnected(&owner_id, Provider::Google, Utc::now())
            .unwrap();

        let mut callback_connected = callback_pending;
        callback_connected.status = AccountStatus::Connected;
        callback_connected.revision = 2;
        let mut credential_write_ran = false;
        let result = initiating_process.append_connected_if_pending_fenced(
            callback_connected,
            Provider::Google,
            &callback_fence,
            || {
                credential_write_ran = true;
                Ok::<(), ()>(())
            },
        );
        assert!(matches!(result, Err(ConditionalUpdateError::Conflict)));
        assert!(!credential_write_ran);
        assert!(
            !initiating_process
                .provider_fence(Provider::Google)
                .unwrap()
                .is_empty()
        );
    }

    #[test]
    fn failed_credential_write_leaves_link_pending() {
        vak_config::paths::isolate_home_for_tests();
        let agent_id = format!("mailcal-link-failure-{}", Uuid::now_v7());
        let account_id = Uuid::now_v7().to_string();
        let ledger = ConnectionLedger::for_agent(&agent_id).unwrap();
        let mut pending = connected_account(&agent_id, &account_id, 1);
        pending.status = AccountStatus::Pending;
        ledger.append_pending(pending.clone()).unwrap();

        let mut activated = pending;
        activated.status = AccountStatus::Connected;
        activated.revision = 2;
        assert!(matches!(
            ledger.append_connected_if_pending(activated, || Err("vault unavailable")),
            Err(ConditionalUpdateError::Operation("vault unavailable"))
        ));
        let latest = ledger.read_all().unwrap().remove(0);
        assert_eq!(latest.status, AccountStatus::Pending);
        assert_eq!(latest.revision, 1);
        assert!(latest.revoked_at.is_none());
    }

    #[test]
    fn shared_ledger_lock_orders_concurrent_refresh_before_disconnect() {
        vak_config::paths::isolate_home_for_tests();
        let agent_id = format!("mailcal-race-{}", Uuid::now_v7());
        let account_id = Uuid::now_v7().to_string();
        let ledger = ConnectionLedger::for_agent(&agent_id).unwrap();
        let mut account = connected_account(&agent_id, &account_id, 1);
        account.status = AccountStatus::Pending;
        account.revision = 1;
        ledger.append_pending(account.clone()).unwrap();
        account.status = AccountStatus::Connected;
        account.revision = 2;
        ledger.append_connected(account.clone()).unwrap();

        let mut refreshed = account;
        refreshed.revision = 3;
        refreshed.access_token_expires_at = Some(Utc::now() + chrono::Duration::hours(1));
        let refresh_ledger = ledger.clone();
        let (refresh_locked_tx, refresh_locked_rx) = std::sync::mpsc::channel();
        let (release_refresh_tx, release_refresh_rx) = std::sync::mpsc::channel();
        let refresh = std::thread::spawn(move || {
            refresh_ledger.append_connected_if_current(refreshed, 2, || {
                refresh_locked_tx.send(()).unwrap();
                release_refresh_rx.recv().unwrap();
                Ok::<_, ()>(())
            })
        });
        refresh_locked_rx
            .recv_timeout(std::time::Duration::from_secs(1))
            .unwrap();

        let disconnect_ledger = ledger.clone();
        let (disconnect_started_tx, disconnect_started_rx) = std::sync::mpsc::channel();
        let (disconnect_done_tx, disconnect_done_rx) = std::sync::mpsc::channel();
        let disconnect = std::thread::spawn(move || {
            disconnect_started_tx.send(()).unwrap();
            let result = disconnect_ledger.append_revoked(&account_id, Utc::now());
            disconnect_done_tx.send(result).unwrap();
        });
        disconnect_started_rx
            .recv_timeout(std::time::Duration::from_secs(1))
            .unwrap();
        assert!(
            disconnect_done_rx
                .recv_timeout(std::time::Duration::from_millis(50))
                .is_err()
        );

        release_refresh_tx.send(()).unwrap();
        refresh.join().unwrap().unwrap();
        disconnect_done_rx
            .recv_timeout(std::time::Duration::from_secs(1))
            .unwrap()
            .unwrap();
        disconnect.join().unwrap();

        let latest = ledger.read_all().unwrap().remove(0);
        assert!(latest.revoked_at.is_some());
        assert_eq!(latest.revision, 4);
    }

    #[test]
    fn shared_ledger_lock_orders_concurrent_link_before_disconnect() {
        vak_config::paths::isolate_home_for_tests();
        let agent_id = format!("mailcal-link-order-{}", Uuid::now_v7());
        let account_id = Uuid::now_v7().to_string();
        let ledger = ConnectionLedger::for_agent(&agent_id).unwrap();
        let mut pending = connected_account(&agent_id, &account_id, 1);
        pending.status = AccountStatus::Pending;
        ledger.append_pending(pending.clone()).unwrap();
        let mut activated = pending;
        activated.status = AccountStatus::Connected;
        activated.revision = 2;

        let activation_ledger = ledger.clone();
        let (activation_locked_tx, activation_locked_rx) = std::sync::mpsc::channel();
        let (release_activation_tx, release_activation_rx) = std::sync::mpsc::channel();
        let activation = std::thread::spawn(move || {
            activation_ledger.append_connected_if_pending(activated, || {
                activation_locked_tx.send(()).unwrap();
                release_activation_rx.recv().unwrap();
                Ok::<_, ()>(())
            })
        });
        activation_locked_rx
            .recv_timeout(std::time::Duration::from_secs(1))
            .unwrap();

        let disconnect_ledger = ledger.clone();
        let (disconnect_started_tx, disconnect_started_rx) = std::sync::mpsc::channel();
        let (disconnect_done_tx, disconnect_done_rx) = std::sync::mpsc::channel();
        let disconnect = std::thread::spawn(move || {
            disconnect_started_tx.send(()).unwrap();
            let result = disconnect_ledger.append_revoked(&account_id, Utc::now());
            disconnect_done_tx.send(result).unwrap();
        });
        disconnect_started_rx
            .recv_timeout(std::time::Duration::from_secs(1))
            .unwrap();
        assert!(
            disconnect_done_rx
                .recv_timeout(std::time::Duration::from_millis(50))
                .is_err()
        );

        release_activation_tx.send(()).unwrap();
        activation.join().unwrap().unwrap();
        disconnect_done_rx
            .recv_timeout(std::time::Duration::from_secs(1))
            .unwrap()
            .unwrap();
        disconnect.join().unwrap();
        let latest = ledger.read_all().unwrap().remove(0);
        assert_eq!(latest.status, AccountStatus::Connected);
        assert!(latest.revoked_at.is_some());
        assert_eq!(latest.revision, 3);
    }

    #[test]
    fn account_revision_cannot_widen_capabilities_or_provider_scopes() {
        let agent_id = "agent-1";
        let account_id = Uuid::now_v7().to_string();
        let ledger = ConnectionLedger {
            path: std::path::PathBuf::new(),
            agent_id: agent_id.to_owned(),
        };
        let mut accounts = BTreeMap::new();
        add_connected_account(
            &ledger,
            &mut accounts,
            connected_account(agent_id, &account_id, 2),
        );

        let mut widened = connected_account(agent_id, &account_id, 3);
        widened.capabilities.insert(Capability::CalendarRead);
        widened
            .provider_scopes
            .insert("https://www.googleapis.com/auth/calendar.readonly".to_owned());
        assert!(matches!(
            ledger.apply_event(
                &mut accounts,
                &ConnectionEvent::Connected {
                    record_id: Uuid::now_v7().to_string(),
                    account: widened,
                },
            ),
            Err(LedgerError::Conflict)
        ));

        let mut over_scoped = connected_account(agent_id, &account_id, 3);
        over_scoped
            .provider_scopes
            .insert("https://www.googleapis.com/auth/gmail.modify".to_owned());
        assert!(
            ledger
                .apply_event(
                    &mut accounts,
                    &ConnectionEvent::Connected {
                        record_id: Uuid::now_v7().to_string(),
                        account: over_scoped,
                    },
                )
                .is_err()
        );
        assert_eq!(
            accounts[&account_id].capabilities,
            BTreeSet::from([Capability::MailRead])
        );
    }

    #[test]
    fn account_ledger_rejects_provider_write_capabilities() {
        let agent_id = "agent-1";
        let account_id = Uuid::now_v7().to_string();
        let ledger = ConnectionLedger {
            path: std::path::PathBuf::new(),
            agent_id: agent_id.to_owned(),
        };
        let mut account = connected_account(agent_id, &account_id, 1);
        account.status = AccountStatus::Pending;
        account.capabilities = BTreeSet::from([Capability::MailSend]);
        account.provider_scopes = BTreeSet::from([
            "openid".to_owned(),
            "email".to_owned(),
            "https://www.googleapis.com/auth/gmail.send".to_owned(),
        ]);
        let result = ledger.apply_event(
            &mut BTreeMap::new(),
            &ConnectionEvent::Pending {
                record_id: Uuid::now_v7().to_string(),
                account,
            },
        );
        assert!(matches!(result, Err(LedgerError::InvalidRecord)));
    }

    #[test]
    fn connected_unverified_is_reserved_for_icloud_accounts() {
        let agent_id = "agent-1";
        let mut apple = connected_account(agent_id, &Uuid::now_v7().to_string(), 2);
        apple.provider = Provider::AppleIcloud;
        apple.provider_scopes.clear();
        apple.status = AccountStatus::ConnectedUnverified;
        assert!(valid_account_record(&apple, agent_id));

        let mut google = connected_account(agent_id, &Uuid::now_v7().to_string(), 2);
        google.status = AccountStatus::ConnectedUnverified;
        assert!(!valid_account_record(&google, agent_id));
    }

    #[cfg(unix)]
    #[test]
    fn read_rejects_broken_mail_calendar_directory_symlink() {
        vak_config::paths::isolate_home_for_tests();
        let agent_id = format!("mailcal-ledger-{}", Uuid::now_v7());
        let vault = crate::vault::AccountVault::for_agent(&agent_id).unwrap();
        let home = vak_config::paths::agent_home(&agent_id);
        let parent = home.join("mail-calendar");
        let missing_target = home.join("outside-target");
        std::os::unix::fs::symlink(&missing_target, &parent).unwrap();
        let ledger = ConnectionLedger::for_agent(&agent_id).unwrap();

        assert!(matches!(ledger.read_all(), Err(LedgerError::InvalidRecord)));
        assert!(!missing_target.exists());
        drop(vault);
    }

    #[cfg(unix)]
    #[test]
    fn read_rejects_symlink_ledger_file() {
        vak_config::paths::isolate_home_for_tests();
        let agent_id = format!("mailcal-ledger-{}", Uuid::now_v7());
        let vault = crate::vault::AccountVault::for_agent(&agent_id).unwrap();
        let home = vak_config::paths::agent_home(&agent_id);
        let parent = home.join("mail-calendar");
        std::fs::create_dir(&parent).unwrap();
        let outside = tempfile::tempdir().unwrap();
        let target = outside.path().join("connections.jsonl");
        std::fs::write(&target, b"").unwrap();
        std::os::unix::fs::symlink(&target, parent.join("connections.jsonl")).unwrap();
        let ledger = ConnectionLedger::for_agent(&agent_id).unwrap();

        assert!(matches!(ledger.read_all(), Err(LedgerError::InvalidRecord)));
        drop(vault);
    }

    #[test]
    fn read_rejects_oversized_ledger_before_parsing() {
        vak_config::paths::isolate_home_for_tests();
        let agent_id = format!("mailcal-ledger-{}", Uuid::now_v7());
        let vault = crate::vault::AccountVault::for_agent(&agent_id).unwrap();
        let home = vak_config::paths::agent_home(&agent_id);
        let parent = home.join("mail-calendar");
        std::fs::create_dir(&parent).unwrap();
        let ledger = ConnectionLedger::for_agent(&agent_id).unwrap();
        let file = std::fs::File::create(ledger.path()).unwrap();
        file.set_len(super::MAX_LEDGER_BYTES + 1).unwrap();

        assert!(matches!(ledger.read_all(), Err(LedgerError::TooLarge)));
        drop(vault);
    }

    #[cfg(unix)]
    #[test]
    fn refuses_symlink_directory_before_creating_its_target() {
        let root = tempfile::tempdir().unwrap();
        let target = root.path().join("outside-target");
        let link = root.path().join("mail-calendar");
        std::os::unix::fs::symlink(&target, &link).unwrap();

        assert!(super::create_private_dir(&link).is_err());
        assert!(!target.exists());
    }
}
