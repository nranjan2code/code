//! A conversation's ledger key (plan M7a-a,
//! docs/design/73-data-architecture-and-lifecycle.md §7.3). Every frame of a
//! session ledger is sealed under its conversation's scope key, the scope its
//! objects are granted to, so destroying that one scope leaves the ledger's
//! bytes in place, its chain verifiable and its content unreadable. The
//! ledger directory names its scope in `KEY`, because the header that
//! carries the session id is itself sealed.

use crate::objects::TenantObjects;
use crate::types::SessionError;
use std::path::Path;
use vak_storage::records::ScopeKey;

const SCOPE_FILE: &str = "KEY";

fn tenant() -> Result<std::sync::Arc<TenantObjects>, SessionError> {
    TenantObjects::for_tenant(&vak_config::paths::local_tenant_home())
}

/// The key scope the ledger or chain at `dir` is sealed under; `None` for a
/// directory that declares none.
pub fn scope_of(dir: &Path) -> Option<String> {
    let scope = std::fs::read_to_string(dir.join(SCOPE_FILE)).ok()?;
    let scope = scope.trim();
    (!scope.is_empty()).then(|| scope.to_string())
}

/// Declares that the new ledger at `dir` is sealed under `scope`, creating
/// the scope's key when it has none.
pub(crate) fn declare(dir: &Path, scope: &str) -> Result<ScopeKey, SessionError> {
    let key = tenant()?.create_scope_key(scope)?;
    let staged = dir.join(format!("{SCOPE_FILE}.tmp"));
    std::fs::write(&staged, scope)?;
    std::fs::File::open(&staged)?.sync_all()?;
    std::fs::rename(&staged, dir.join(SCOPE_FILE))?;
    Ok(key)
}

/// The key that opens the frames in `dir`: `None` when the directory
/// declares no scope (a record chain's frames are not sealed), an error
/// when its scope's key is gone.
pub(crate) fn of(dir: &Path) -> Result<Option<ScopeKey>, SessionError> {
    match scope_of(dir) {
        Some(scope) => tenant()?.scope_key(&scope).map(Some),
        None => Ok(None),
    }
}

/// As `of`, for a session ledger, which is never read unsealed.
pub(crate) fn of_ledger(dir: &Path) -> Result<ScopeKey, SessionError> {
    of(dir)?.ok_or_else(|| SessionError::Unencrypted(dir.to_path_buf()))
}

/// As `of`, for the directory holding the segment file `segment`.
pub(crate) fn of_segment(segment: &Path) -> Result<Option<ScopeKey>, SessionError> {
    match segment.parent() {
        Some(dir) => of(dir),
        None => Ok(None),
    }
}
