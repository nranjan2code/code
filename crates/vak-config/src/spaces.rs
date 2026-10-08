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
    /// What a person calls the space; its folder's name when unset.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    name: Option<String>,
    /// Hidden from the lists a surface offers until it is opened again.
    /// Forgetting is not deleting: its sessions, memory and settings stay.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    forgotten: bool,
    /// When it was last opened as a workspace, in seconds since the epoch.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    last_opened: Option<u64>,
    #[serde(flatten)]
    rest: toml::Table,
}

/// One space as a surface lists it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SpaceRecord {
    pub id: String,
    /// The folder this machine binds it to.
    pub folder: Option<PathBuf>,
    pub name: Option<String>,
    pub forgotten: bool,
    pub last_opened: Option<u64>,
}

/// The registry file of the local tenant.
pub fn registry_path() -> PathBuf {
    registry_path_at(&crate::paths::data_home())
}

/// The local tenant's registry file under the data home `data`.
pub fn registry_path_at(data: &Path) -> PathBuf {
    crate::paths::tenant_home_at(data, crate::paths::LOCAL_TENANT).join("spaces.toml")
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

/// Change the space `path` is bound to (binding it first) under the
/// registry lock, and write the registry back atomically.
fn modify(
    registry_file: &Path,
    path: &Path,
    change: impl FnOnce(&mut Space),
) -> Result<String, String> {
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
    let id = find(&registry, path).unwrap_or_else(mint);
    let space = registry.spaces.entry(id.clone()).or_default();
    if !space.bindings.iter().any(|bound| bound == path) {
        space.bindings.push(path.to_path_buf());
    }
    change(space);
    let text = toml::to_string(&registry).map_err(|error| error.to_string())?;
    let temp = dir.join(format!("spaces.toml.{}", std::process::id()));
    let mut file = std::fs::File::create(&temp).map_err(|error| error.to_string())?;
    file.write_all(text.as_bytes())
        .and_then(|()| file.sync_all())
        .map_err(|error| error.to_string())?;
    std::fs::rename(&temp, registry_file).map_err(|error| error.to_string())?;
    Ok(id)
}

/// The key of a run's environment (`paths::environment_dir`): `env-<run>`.
/// An environment belongs to its run, never to a space of its own, so it is
/// never bound and the registry does not grow with every run.
fn environment_owner(path: &Path) -> Option<String> {
    let root = crate::paths::local_tenant_home().join("environments");
    for root in [canonical(&root), root] {
        if let Ok(rest) = path.strip_prefix(&root)
            && let Some(std::path::Component::Normal(run)) = rest.components().next()
        {
            return Some(format!("env-{}", run.to_str()?));
        }
    }
    None
}

/// What `path` belongs to without the registry: an Agent workspace's space,
/// or a run environment's run.
fn owner(path: &Path) -> Option<String> {
    agent_workspace_owner(path).or_else(|| environment_owner(path))
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
    bind_at(&crate::paths::data_home(), path)
}

/// [`bind`] against the registry of the data home `data`.
pub fn bind_at(data: &Path, path: &Path) -> Result<String, String> {
    let path = canonical(path);
    if let Some(id) = owner(&path) {
        return Ok(id);
    }
    let registry = registry_path_at(data);
    if let Some(id) = cached(&registry, &path) {
        return Ok(id);
    }
    let id = match find(&load(&registry)?, &path) {
        Some(id) => id,
        None => modify(&registry, &path, |_| {})?,
    };
    remember(&registry, &path, &id);
    Ok(id)
}

fn change(path: &Path, apply: impl FnOnce(&mut Space)) -> Result<String, String> {
    let path = canonical(path);
    if let Some(id) = owner(&path) {
        return Ok(id);
    }
    let registry = registry_path();
    let id = modify(&registry, &path, apply)?;
    remember(&registry, &path, &id);
    Ok(id)
}

/// Record that the folder `path` was opened as a workspace: bound, shown
/// again if it was forgotten, and first in the recent list.
pub fn opened(path: &Path) -> Result<String, String> {
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|elapsed| elapsed.as_secs())
        .unwrap_or_default();
    change(path, |space| {
        space.forgotten = false;
        space.last_opened = Some(now);
    })
}

/// Name the space of `path`.
pub fn set_name(path: &Path, name: &str) -> Result<String, String> {
    let name = name.to_string();
    change(path, |space| space.name = Some(name))
}

/// Change the space `id` under the registry lock. A space not in the
/// registry is an error; nothing is created.
fn change_id(id: &str, apply: impl FnOnce(&mut Space)) -> Result<(), String> {
    let registry_file = registry_path();
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
    let mut registry = load(&registry_file)?;
    let space = registry
        .spaces
        .get_mut(id)
        .ok_or_else(|| format!("no project {id}"))?;
    apply(space);
    let text = toml::to_string(&registry).map_err(|error| error.to_string())?;
    let temp = dir.join(format!("spaces.toml.{}", std::process::id()));
    let mut file = std::fs::File::create(&temp).map_err(|error| error.to_string())?;
    file.write_all(text.as_bytes())
        .and_then(|()| file.sync_all())
        .map_err(|error| error.to_string())?;
    std::fs::rename(&temp, &registry_file).map_err(|error| error.to_string())
}

/// Name the space `id`.
pub fn rename(id: &str, name: &str) -> Result<(), String> {
    let name = name.to_string();
    change_id(id, |space| space.name = Some(name))
}

/// Hide the space `id` from the lists surfaces offer, or show it again.
pub fn set_forgotten(id: &str, forgotten: bool) -> Result<(), String> {
    change_id(id, |space| space.forgotten = forgotten)
}

/// Every space in the registry, the most recently opened first.
pub fn all() -> Vec<SpaceRecord> {
    let Ok(registry) = load(&registry_path()) else {
        return Vec::new();
    };
    let mut spaces: Vec<SpaceRecord> = registry
        .spaces
        .into_iter()
        .map(|(id, space)| SpaceRecord {
            id,
            folder: space.bindings.into_iter().next(),
            name: space.name,
            forgotten: space.forgotten,
            last_opened: space.last_opened,
        })
        .collect();
    spaces.sort_by_key(|space| std::cmp::Reverse(space.last_opened));
    spaces
}

/// The id of the space `path` is bound to; `None` for a folder never opened.
pub fn bound_space(path: &Path) -> Option<String> {
    let path = canonical(path);
    if let Some(id) = owner(&path) {
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
    bindings_at(&crate::paths::data_home(), id)
}

/// [`bindings`] in the registry of the data home `data`.
pub fn bindings_at(data: &Path, id: &str) -> Vec<PathBuf> {
    load(&registry_path_at(data))
        .ok()
        .and_then(|mut registry| registry.spaces.remove(id))
        .map(|space| space.bindings)
        .unwrap_or_default()
}

/// Refuse a write whose location is partitioned by an `unbound-` key: the
/// folder was never opened as a workspace, so anything written there would
/// be orphaned the moment it is. Space-keyed writers call this.
pub fn require_bound(location: &Path) -> Result<(), String> {
    match location.components().find(|part| {
        part.as_os_str()
            .to_str()
            .is_some_and(|part| part.starts_with("unbound-"))
    }) {
        Some(_) => Err(format!(
            "{} belongs to a folder never opened as a workspace; open it first",
            location.display()
        )),
        None => Ok(()),
    }
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
        let orphan = Path::new("/data/agents/vak/sessions").join(key(one.path()));
        assert!(
            require_bound(&orphan).is_err(),
            "no write under an unbound key"
        );
        let id = bind(one.path()).unwrap();
        assert!(id.starts_with(PREFIX));
        assert_eq!(bind(one.path()).unwrap(), id);
        assert_eq!(key(one.path()), id);
        assert_ne!(bind(two.path()).unwrap(), id);
        let workspace = crate::paths::ensure_agent_workspace(one.path(), "mira").unwrap();
        assert_eq!(key(&workspace), id, "an Agent workspace is its space's");
        let environment = crate::paths::environment_dir("run-7").join("work");
        std::fs::create_dir_all(&environment).unwrap();
        assert_eq!(bind(&environment).unwrap(), "env-run-7");
        assert!(
            all()
                .iter()
                .all(|space| space.folder.as_deref() != Some(environment.as_path())),
            "a run's environment is never a space"
        );
        assert_eq!(bindings(&id), vec![one.path().canonicalize().unwrap()]);
        assert!(!one.path().join(".vak").exists());
    }
}
