//! Agent-scoped mail/calendar secrets backed by Vak's credential vault.
//!
//! This module never writes credential material to the connection ledger or
//! filesystem. The credential service chooses the native OS vault or its
//! encrypted-file fallback. Secret values are not `Debug` and are zeroized
//! when dropped.

use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use uuid::Uuid;
use zeroize::{Zeroize, Zeroizing};

const MAX_SECRET_BYTES: usize = 256 * 1024;
const MAX_PRINCIPAL_BYTES: usize = 512;

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

    /// Removes all account credential material from the Agent's credential
    /// scope. Safe to retry after partial disconnects.
    pub fn remove(&self, account_id: &str) -> Result<(), VaultError> {
        let key = Self::credential_ref(account_id)?;
        vak_config::remove_env_file_key(&self.scope_hint, &key)?;
        Ok(())
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

#[cfg(test)]
mod tests {
    use super::{AccountSecretMaterial, AccountVault, Uuid, VaultError};

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
