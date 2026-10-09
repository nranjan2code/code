//! The tenant's keys as the owner sees them (data-architecture plan
//! M7b-g): where they are kept, which version is in use, and rotation.
//! No key material is ever returned.

use crate::Core;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

/// One row of the `key-rotations/` chain: a rotation that finished.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct KeyRotation {
    pub id: String,
    pub at: DateTime<Utc>,
    /// The key version in use from this rotation on.
    pub version: u32,
    /// How many stored keys were wrapped again under it.
    pub rewrapped: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub actor: Option<vak_session::ids::PrincipalId>,
}

/// What the Keys screen shows.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct KeyStatus {
    /// Where the keys are kept: `keychain` or `encrypted_file`.
    pub kept_in: &'static str,
    /// The version new keys are wrapped under.
    pub version: u32,
    /// The oldest version any stored key is still under.
    pub oldest_in_use: u32,
    pub keys: u64,
    pub destroyed: u64,
    pub held: u64,
    /// Every rotation, oldest first.
    pub rotations: Vec<KeyRotation>,
}

impl Core {
    /// Where the keys are kept and which are in use. Reads only.
    pub fn key_status(&self) -> Result<KeyStatus, String> {
        let tenant = self.tenant_objects().map_err(|error| error.to_string())?;
        let (version, oldest_in_use) = tenant.key_versions().map_err(|error| error.to_string())?;
        let (keys, destroyed) = tenant.scope_counts();
        Ok(KeyStatus {
            kept_in: vak_config::credentials::backend(),
            version,
            oldest_in_use,
            keys: keys as u64,
            destroyed: destroyed as u64,
            held: tenant.held_count() as u64,
            rotations: vak_session::chain::RecordChain::at(self.shared_scope().key_rotations())
                .read(),
        })
    }

    /// The text of a key file: this install's keys sealed under
    /// `passphrase`. A second machine imports it to read the same data
    /// from a remote (data-architecture plan M9-b). It is as sensitive as
    /// the passphrase is weak, and Vakyartha never puts it on a remote.
    pub fn export_key_file(&self, passphrase: &str) -> Result<String, String> {
        check_passphrase(passphrase)?;
        vak_session::objects::TenantObjects::key_material(&self.tenant_home())
            .and_then(|material| {
                material
                    .seal(passphrase)
                    .map_err(|error| vak_session::SessionError::Objects(error.to_string()))
            })
            .map_err(|error| error.to_string())
    }

    /// Takes another machine's keys from a key file. Refused once this
    /// install holds anything under its own keys, which it could no
    /// longer read. The process must stop afterwards.
    pub fn import_key_file(&self, file: &str, passphrase: &str) -> Result<(), String> {
        let material = vak_storage::keys::KeyMaterial::open(file, passphrase).map_err(|_| {
            "the key file could not be opened: the passphrase is wrong or the file is damaged"
                .to_string()
        })?;
        let status = self.key_status()?;
        if status.keys + status.destroyed > 0 {
            return Err(
                "this install already holds data under its own keys; a key file is for a new install"
                    .into(),
            );
        }
        vak_session::objects::TenantObjects::install_key_material(&self.tenant_home(), &material)
            .map_err(|error| error.to_string())?;
        vak_session::fence::retire();
        Ok(())
    }

    /// Starts a new tenant key and wraps every stored key under it, then
    /// records the rotation. Everything stays readable; what was erased
    /// stays erased. Earlier keys are kept in the credential store, so a
    /// backup made before still opens.
    pub fn rotate_keys(
        &self,
        actor: Option<vak_session::ids::PrincipalId>,
    ) -> Result<KeyRotation, String> {
        let _turn = crate::erasure::one_at_a_time();
        let tenant = self.tenant_objects().map_err(|error| error.to_string())?;
        let (version, rewrapped) = tenant.rotate_keys().map_err(|error| error.to_string())?;
        let rotation = KeyRotation {
            id: format!("rot_{}", uuid::Uuid::now_v7()),
            at: Utc::now(),
            version,
            rewrapped: rewrapped as u64,
            actor,
        };
        vak_session::chain::RecordChain::at(self.shared_scope().key_rotations())
            .append(&rotation)
            .map_err(|error| error.to_string())?;
        tracing::info!(
            kind = "key_rotation",
            outcome = "completed",
            count = rotation.rewrapped,
            "the tenant key was rotated"
        );
        Ok(rotation)
    }
}

/// The shortest passphrase a key file is sealed under.
pub const MIN_PASSPHRASE: usize = 12;

fn check_passphrase(passphrase: &str) -> Result<(), String> {
    if passphrase.chars().count() < MIN_PASSPHRASE {
        return Err(format!(
            "the passphrase needs at least {MIN_PASSPHRASE} characters"
        ));
    }
    Ok(())
}
