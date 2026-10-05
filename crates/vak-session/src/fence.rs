//! Fencing (plan M4.1, docs/design/73-data-architecture-and-lifecycle.md
//! §7). A tenant store has one writer epoch, bumped only when the store is
//! restored (and later when a host hands it over, doc 79 §11). A process
//! reads it once when it opens the store ([`WriterEpoch`]); from the moment
//! the store's epoch is newer, the process is fenced: it opens no ledger for
//! writing, begins no turn, appends to no record chain and moves no ref,
//! and the server stops its background work. Being fenced is sticky, since
//! an epoch never goes back.
//!
//! Live processes on one store share the epoch; ref generations keep them
//! apart. Each process also has a [`ProcessId`] and renews a liveness ref,
//! `proc/<prc>`, so a lease names a holder that can be judged alive or gone
//! without a lease per run.

use crate::ids::ProcessId;
use crate::objects::{TenantObjects, objects_error};
use crate::types::SessionError;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::path::Path;
use std::sync::OnceLock;

/// The writer epoch a process read when it opened a tenant store.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct WriterEpoch(pub u64);

static FENCED: OnceLock<(u64, u64)> = OnceLock::new();

/// Refuses with [`SessionError::Fenced`] once any tenant store this process
/// opened has a newer epoch than the one it read at open. A process that
/// opened no store holds no epoch and has nothing to be fenced from.
pub fn check() -> Result<(), SessionError> {
    if let Some(&(held, current)) = FENCED.get() {
        return Err(SessionError::Fenced { held, current });
    }
    for tenant in crate::objects::opened() {
        let held = tenant.writer_epoch().0;
        let current = tenant.store().epoch().map_err(objects_error)?;
        if current > held {
            let &(held, current) = FENCED.get_or_init(|| (held, current));
            return Err(SessionError::Fenced { held, current });
        }
    }
    Ok(())
}

/// Whether this process is fenced. An error reading an epoch is not an
/// answer, so it reads as not fenced here; every write path calls
/// [`check`] and fails on it.
pub fn is_fenced() -> bool {
    matches!(check(), Err(SessionError::Fenced { .. }))
}

/// This process's id, minted on first use.
pub fn process() -> ProcessId {
    static PROCESS: OnceLock<ProcessId> = OnceLock::new();
    *PROCESS.get_or_init(ProcessId::new)
}

const LIVENESS_PREFIX: &str = "proc/";

/// The target of a liveness ref.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Liveness {
    pub alive_until: DateTime<Utc>,
}

fn liveness_ref(process: &ProcessId) -> String {
    format!("{LIVENESS_PREFIX}{process}")
}

/// Moves this process's liveness ref in the tenant at `tenant_home` to
/// `alive_until`, under the writer epoch: a fenced process cannot renew,
/// so its leases lapse.
pub fn renew_liveness(tenant_home: &Path, alive_until: DateTime<Utc>) -> Result<(), SessionError> {
    check()?;
    let tenant = TenantObjects::for_tenant(tenant_home)?;
    let store = tenant.store();
    let name = liveness_ref(&process());
    let target = serde_json::to_vec(&Liveness { alive_until })
        .map_err(|error| SessionError::Objects(error.to_string()))?;
    let current = store.get_ref(&name).map_err(objects_error)?;
    store
        .cas_ref(
            &name,
            current.map(|value| value.generation),
            tenant.writer_epoch().0,
            &target,
        )
        .map_err(|error| match error {
            vak_storage::StorageError::StaleEpoch { presented, current } => SessionError::Fenced {
                held: presented,
                current,
            },
            other => objects_error(other),
        })?;
    Ok(())
}

/// When `process` last said it would still be alive, `None` if it never
/// renewed in this tenant.
pub fn liveness(tenant_home: &Path, process: &ProcessId) -> Result<Option<Liveness>, SessionError> {
    let tenant = TenantObjects::for_tenant(tenant_home)?;
    let Some(value) = tenant
        .store()
        .get_ref(&liveness_ref(process))
        .map_err(objects_error)?
    else {
        return Ok(None);
    };
    serde_json::from_slice(&value.target)
        .map(Some)
        .map_err(|error| SessionError::Objects(format!("liveness ref: {error}")))
}

/// Whether `process` is alive at `now` by its liveness ref.
pub fn is_alive(
    tenant_home: &Path,
    process: &ProcessId,
    now: DateTime<Utc>,
) -> Result<bool, SessionError> {
    Ok(liveness(tenant_home, process)?.is_some_and(|live| live.alive_until > now))
}
