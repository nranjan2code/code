//! Agent-authored state as Documents (plan M3b slice 3, doc 73 §5): each is
//! named by the path its file had, relative to the data home, and stored as
//! immutable versions in the tenant store with a CAS ref to the current
//! one. A Document under `agents/<id>/` is granted to that Agent's scope,
//! so erasing an Agent can drop exactly its own. Writers never lock: an
//! `update` re-reads and retries when another writer moved the head.

use std::path::{Path, PathBuf};
use std::sync::Arc;
use vak_storage::documents::Documents;

/// Attempts at a contended `update` before it gives up.
const RETRIES: usize = 16;

fn tenant() -> Result<Arc<crate::objects::TenantObjects>, String> {
    let tenant = vak_config::paths::local_tenant_home();
    crate::objects::TenantObjects::for_tenant(&tenant).map_err(|error| error.to_string())
}

/// The Document name for `path`, and the scope its versions are granted to.
fn name_and_scope(path: &Path) -> (String, String) {
    let data = vak_config::paths::data_home();
    let name = path
        .strip_prefix(&data)
        .unwrap_or(path)
        .to_string_lossy()
        .replace('\\', "/");
    let mut parts = name.split('/');
    let scope = match (parts.next(), parts.next()) {
        (Some("agents"), Some(agent)) if !agent.is_empty() => format!("agent:{agent}"),
        _ => "tenant".to_string(),
    };
    (name, scope)
}

/// Documents in `scope`, written under the epoch this process read at
/// open, so a restore since then refuses every save.
fn documents(scope: &str) -> Result<Documents, String> {
    let tenant = tenant()?;
    Ok(Documents::new(
        tenant.store(),
        scope,
        tenant.writer_epoch().0,
    ))
}

/// The current content of the Document at `path`, `None` if it was never
/// written or was forgotten.
pub fn read(path: &Path) -> Result<Option<String>, String> {
    let (name, scope) = name_and_scope(path);
    match documents(&scope)?.current(&name) {
        Ok(Some((_, bytes))) => String::from_utf8(bytes)
            .map(Some)
            .map_err(|_| format!("{name} is not text")),
        Ok(None) | Err(vak_storage::StorageError::Forgotten) => Ok(None),
        Err(error) => Err(format!("read {name}: {error}")),
    }
}

/// Applies `change` to the current content (`None` when there is none) and
/// saves what it returns as a new version; `Ok(None)` from `change` writes
/// nothing. Re-reads and retries when another writer saved in between.
pub fn update<T>(
    path: &Path,
    mut change: impl FnMut(Option<&str>) -> Result<Option<(String, T)>, String>,
) -> Result<Option<T>, String> {
    vak_config::spaces::require_bound(path)?;
    crate::fence::check().map_err(|error| error.to_string())?;
    let (name, scope) = name_and_scope(path);
    let docs = documents(&scope)?;
    for _ in 0..RETRIES {
        let generation = docs
            .generation(&name)
            .map_err(|error| format!("read {name}: {error}"))?;
        let current = match docs.current(&name) {
            Ok(Some((_, bytes))) => {
                Some(String::from_utf8(bytes).map_err(|_| format!("{name} is not text"))?)
            }
            Ok(None) | Err(vak_storage::StorageError::Forgotten) => None,
            Err(error) => return Err(format!("read {name}: {error}")),
        };
        let Some((next, value)) = change(current.as_deref())? else {
            return Ok(None);
        };
        match docs.save(&name, next.as_bytes(), generation) {
            Ok(_) => return Ok(Some(value)),
            Err(vak_storage::StorageError::Conflict { .. }) => continue,
            Err(error) => return Err(format!("save {name}: {error}")),
        }
    }
    Err(format!("save {name}: too many concurrent writers"))
}

/// The paths of the live Documents under the directory `dir`.
pub fn under(dir: &Path) -> Vec<PathBuf> {
    let (prefix, scope) = name_and_scope(dir);
    let prefix = format!("{}/", prefix.trim_end_matches('/'));
    let data = vak_config::paths::data_home();
    let Ok(docs) = documents(&scope) else {
        return Vec::new();
    };
    docs.names(&prefix)
        .unwrap_or_default()
        .into_iter()
        .map(|name| {
            let path = PathBuf::from(&name);
            if path.is_absolute() {
                path
            } else {
                data.join(path)
            }
        })
        .collect()
}

/// Saves `content` as a new Document at `path`; fails if one is already
/// there, so two writers can never both believe they created it.
pub fn create(path: &Path, content: &str) -> Result<(), String> {
    vak_config::spaces::require_bound(path)?;
    crate::fence::check().map_err(|error| error.to_string())?;
    let (name, scope) = name_and_scope(path);
    documents(&scope)?
        .save(&name, content.as_bytes(), None)
        .map(|_| ())
        .map_err(|error| format!("save {name}: {error}"))
}

/// Forgets the Document at `path`: it reads as absent and its versions'
/// objects are released for collection. Returns whether it existed.
pub fn forget(path: &Path) -> Result<bool, String> {
    crate::fence::check().map_err(|error| error.to_string())?;
    let (name, scope) = name_and_scope(path);
    let docs = documents(&scope)?;
    match docs.current(&name) {
        Ok(Some(_)) => docs
            .forget(&name)
            .map(|_| true)
            .map_err(|error| format!("forget {name}: {error}")),
        Ok(None) | Err(vak_storage::StorageError::Forgotten) => Ok(false),
        Err(error) => Err(format!("read {name}: {error}")),
    }
}

/// The scope a Document named `name` is in (`name_and_scope`, by name).
fn scope_of(name: &str) -> String {
    name_and_scope(Path::new(name)).1
}

/// Every live Document's name, across the tenant's and each Agent's scope.
pub fn names() -> Vec<String> {
    let Ok(docs) = documents("tenant") else {
        return Vec::new();
    };
    docs.names("").unwrap_or_default()
}

/// When each earlier version of the Document named `name` was saved,
/// newest first, without the current one. `None` for a version written
/// before versions carried a time.
pub fn history_times(name: &str) -> Vec<Option<i64>> {
    documents(&scope_of(name))
        .and_then(|docs| docs.history(name).map_err(|error| error.to_string()))
        .map(|history| history.iter().skip(1).map(|version| version.at).collect())
        .unwrap_or_default()
}

/// Removes from the history of the Document named `name` every version
/// saved at or before `cutoff` (seconds since the epoch); the current one
/// always stays (plan M7a-d). Returns how many left.
pub fn prune_history(name: &str, cutoff: i64) -> Result<usize, String> {
    crate::fence::check().map_err(|error| error.to_string())?;
    documents(&scope_of(name))?
        .prune(name, cutoff)
        .map_err(|error| format!("prune {name}: {error}"))
}

/// Releases, in each scope Documents are kept in, every object no live
/// Document names any more (what pruning and forgetting leave), then
/// deletes the objects no scope holds (plan M7a-d). A grant younger than
/// `min_age` stays: a save in flight has written objects its Document
/// does not name yet. The `tenant` and `agent:<id>` scopes hold Documents
/// and nothing else, which is what makes this safe; a failure to read a
/// Document ends the pass with nothing released for its scope. Returns
/// how many grants were released and how many objects were deleted.
pub fn collect(min_age: std::time::Duration) -> Result<(usize, usize), String> {
    crate::fence::check().map_err(|error| error.to_string())?;
    let mut scopes: std::collections::BTreeSet<String> =
        names().iter().map(|name| scope_of(name)).collect();
    scopes.insert("tenant".into());
    let mut released = 0;
    for scope in scopes {
        released += documents(&scope)?
            .collect(min_age)
            .map_err(|error| format!("collect {scope}: {error}"))?;
    }
    let deleted = {
        use crate::objects::Objects;
        tenant()?.collect().map_err(|error| error.to_string())?
    };
    Ok((released, deleted))
}

/// How many versions the Document at `path` has had.
pub fn version_count(path: &Path) -> usize {
    let (name, scope) = name_and_scope(path);
    documents(&scope)
        .and_then(|docs| docs.history(&name).map_err(|error| error.to_string()))
        .map(|history| history.len())
        .unwrap_or(0)
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    #[test]
    fn a_document_reads_back_lists_and_keeps_its_versions() {
        let home = vak_config::paths::isolate_home_for_tests();
        let path = home.join("agents/docs-test/memory/w/MEMORY.md");
        assert_eq!(read(&path).unwrap(), None);
        update(&path, |old| {
            assert!(old.is_none());
            Ok(Some(("first\n".to_string(), ())))
        })
        .unwrap();
        update(&path, |old| {
            Ok(Some((format!("{}second\n", old.unwrap()), ())))
        })
        .unwrap();
        assert_eq!(read(&path).unwrap().as_deref(), Some("first\nsecond\n"));
        assert_eq!(version_count(&path), 2);
        assert_eq!(
            under(&home.join("agents/docs-test/memory")),
            vec![path.clone()]
        );
        assert!(!path.exists(), "a Document is not a file");
        assert_eq!(
            name_and_scope(&path).1,
            "agent:docs-test",
            "granted to its Agent"
        );
    }

    #[test]
    fn concurrent_updates_lose_nothing() {
        let home = vak_config::paths::isolate_home_for_tests();
        let path = home.join("agents/docs-race/memory/race/MEMORY.md");
        let writers: Vec<_> = (0..4)
            .map(|n| {
                let path = path.clone();
                std::thread::spawn(move || {
                    update(&path, |old| {
                        Ok(Some((format!("{}line {n}\n", old.unwrap_or_default()), ())))
                    })
                    .unwrap();
                })
            })
            .collect();
        for writer in writers {
            writer.join().unwrap();
        }
        assert_eq!(read(&path).unwrap().unwrap().lines().count(), 4);
    }
}
