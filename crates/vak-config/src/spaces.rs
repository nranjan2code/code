//! Space identity (data-architecture plan M3b slice 5, review R12). A space
//! is named by a `spc_` id minted once, when one of its folders is first
//! opened as a workspace ([`bind`]), and recorded in the tenant's space
//! registry (`tenants/<tenant>/spaces.toml`, Desired class) with this
//! machine's path bindings. Everything keyed by a space (session ledgers,
//! executions, Agent workspaces, credential scopes, trust, schedules, the
//! gateway) keys by this id, never by a hash of the path. Only [`bind`]
//! writes: resolving a key ([`key`], [`bound_space`]) never binds a folder,
//! so computing a path has no side effect. An Agent's workspace
//! (`paths::agent_workspace`) belongs to the space it was made for. Moving
//! a folder makes a new space until it is bound again; nothing is written
//! into the project.

use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, HashMap};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock};

/// The prefix of a space id, as `vak_session::ids::SpaceId` prints it.
pub const PREFIX: &str = "spc_";

#[derive(Debug, Default, Serialize, Deserialize)]
struct Registry {
    #[serde(default)]
    spaces: BTreeMap<String, Space>,
    #[serde(flatten)]
    rest: toml::Table,
}

#[derive(Debug, Default, Serialize, Deserialize)]
struct Space {
    #[serde(default)]
    bindings: Vec<PathBuf>,
    #[serde(flatten)]
    rest: toml::Table,
}

/// The registry file of the local tenant.
pub fn registry_path() -> PathBuf {
    crate::paths::local_tenant_home().join("spaces.toml")
}

fn canonical(path: &Path) -> PathBuf {
    path.canonicalize().unwrap_or_else(|_| path.to_path_buf())
}

type Cache = Mutex<HashMap<(PathBuf, PathBuf), String>>;

fn cache() -> &'static Cache {
    static CACHE: OnceLock<Cache> = OnceLock::new();
    CACHE.get_or_init(Default::default)
}

fn cached(registry: &Path, path: &Path) -> Option<String> {
    cache()
        .lock()
        .ok()?
        .get(&(registry.to_path_buf(), path.to_path_buf()))
        .cloned()
}

fn remember(registry: &Path, path: &Path, id: &str) {
    if let Ok(mut cache) = cache().lock() {
        cache.insert((registry.to_path_buf(), path.to_path_buf()), id.to_string());
    }
}

fn load(registry: &Path) -> Result<Registry, String> {
    match std::fs::read_to_string(registry) {
        Ok(text) => toml::from_str(&text).map_err(|error| {
            format!(
                "space registry {} is unreadable: {error}",
                registry.display()
            )
        }),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(Registry::default()),
        Err(error) => Err(format!("cannot read {}: {error}", registry.display())),
    }
}

fn find(registry: &Registry, path: &Path) -> Option<String> {
    registry
        .spaces
        .iter()
        .find(|(_, space)| space.bindings.iter().any(|bound| bound == path))
        .map(|(id, _)| id.clone())
}

fn mint() -> String {
    format!("{PREFIX}{}", uuid::Uuid::now_v7().hyphenated())
}

fn insert(registry_file: &Path, path: &Path) -> Result<String, String> {
    let dir = registry_file
        .parent()
        .ok_or("space registry has no directory")?;
    std::fs::create_dir_all(dir).map_err(|error| error.to_string())?;
    let lock = std::fs::OpenOptions::new()
        .create(true)
        .truncate(false)
        .write(true)
        .open(dir.join("spaces.lock"))
        .map_err(|error| error.to_string())?;
    lock.lock().map_err(|error| error.to_string())?;
    let mut registry = load(registry_file)?;
    if let Some(id) = find(&registry, path) {
        return Ok(id);
    }
    let id = mint();
    registry
        .spaces
        .entry(id.clone())
        .or_default()
        .bindings
        .push(path.to_path_buf());
    let text = toml::to_string(&registry).map_err(|error| error.to_string())?;
    let temp = dir.join(format!("spaces.toml.{}", std::process::id()));
    let mut file = std::fs::File::create(&temp).map_err(|error| error.to_string())?;
    file.write_all(text.as_bytes())
        .and_then(|()| file.sync_all())
        .map_err(|error| error.to_string())?;
    std::fs::rename(&temp, registry_file).map_err(|error| error.to_string())?;
    Ok(id)
}

/// The space an Agent workspace under the tenant's `workspaces/` belongs to.
pub(crate) fn agent_workspace_owner(path: &Path) -> Option<String> {
    let root = crate::paths::local_tenant_home().join("workspaces");
    for root in [canonical(&root), root] {
        if let Ok(rest) = path.strip_prefix(&root) {
            let parts: Vec<_> = rest.components().collect();
            if let [space, _agent] = parts.as_slice() {
                let space = space.as_os_str().to_str()?;
                return space.starts_with(PREFIX).then(|| space.to_string());
            }
        }
    }
    None
}

/// Bind the folder `path` to a space, minting the space the first time the
/// folder is opened as a workspace, and return its id. The one writer.
pub fn bind(path: &Path) -> Result<String, String> {
    let path = canonical(path);
    if let Some(id) = agent_workspace_owner(&path) {
        return Ok(id);
    }
    let registry = registry_path();
    if let Some(id) = cached(&registry, &path) {
        return Ok(id);
    }
    let id = match find(&load(&registry)?, &path) {
        Some(id) => id,
        None => insert(&registry, &path)?,
    };
    remember(&registry, &path, &id);
    Ok(id)
}

/// The id of the space `path` is bound to; `None` for a folder never opened.
pub fn bound_space(path: &Path) -> Option<String> {
    let path = canonical(path);
    if let Some(id) = agent_workspace_owner(&path) {
        return Some(id);
    }
    let registry = registry_path();
    if let Some(id) = cached(&registry, &path) {
        return Some(id);
    }
    let id = find(&load(&registry).ok()?, &path)?;
    remember(&registry, &path, &id);
    Some(id)
}

/// The folders on this machine bound to the space `id`.
pub fn bindings(id: &str) -> Vec<PathBuf> {
    load(&registry_path())
        .ok()
        .and_then(|mut registry| registry.spaces.remove(id))
        .map(|space| space.bindings)
        .unwrap_or_default()
}

/// The key a space-keyed store files `path` under: its space id, or for a
/// folder never opened as a workspace an `unbound-` name derived from the
/// path, which no space id can equal. Never writes.
pub fn key(path: &Path) -> String {
    bound_space(path).unwrap_or_else(|| {
        let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
        for byte in canonical(path).to_string_lossy().bytes() {
            hash ^= u64::from(byte);
            hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
        }
        format!("unbound-{hash:016x}")
    })
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    #[test]
    fn a_folder_keeps_its_space_and_another_gets_its_own() {
        crate::paths::isolate_home_for_tests();
        let one = tempfile::tempdir().unwrap();
        let two = tempfile::tempdir().unwrap();
        assert_eq!(bound_space(one.path()), None);
        assert!(key(one.path()).starts_with("unbound-"));
        assert_eq!(bound_space(one.path()), None, "resolving a key never binds");
        let id = bind(one.path()).unwrap();
        assert!(id.starts_with(PREFIX));
        assert_eq!(bind(one.path()).unwrap(), id);
        assert_eq!(key(one.path()), id);
        assert_ne!(bind(two.path()).unwrap(), id);
        let workspace = crate::paths::ensure_agent_workspace(one.path(), "mira").unwrap();
        assert_eq!(key(&workspace), id, "an Agent workspace is its space's");
        assert_eq!(bindings(&id), vec![one.path().canonicalize().unwrap()]);
        assert!(!one.path().join(".vak").exists());
    }
}
