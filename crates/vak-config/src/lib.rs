//! Greenfield configuration and secret services.
//!
//! Configuration has exactly two TOML layers: the runtime's `data_home/config.toml`
//! and the project's `.vakcoder/project.toml`. Writes are serialized and committed
//! with an atomic rename; callers receive a monotonically increasing revision.

use serde::{Deserialize, Serialize};
use std::{
    collections::{BTreeMap, BTreeSet},
    fs::{self, File, OpenOptions},
    io::{self, Write},
    path::{Path, PathBuf},
    sync::atomic::{AtomicU64, Ordering},
    thread,
    time::{Duration, SystemTime},
};
use thiserror::Error;

pub mod paths {
    use std::path::PathBuf;

    pub fn data_home() -> PathBuf {
        homes().0
    }

    pub fn cache_home() -> PathBuf {
        homes().1
    }

    pub fn logs_dir() -> PathBuf {
        homes().2
    }

    pub fn global_config() -> PathBuf {
        data_home().join("config.toml")
    }

    pub fn user_env() -> PathBuf {
        data_home().join(".env")
    }

    fn homes() -> (PathBuf, PathBuf, PathBuf) {
        let base = std::env::var_os("HOME")
            .or_else(|| std::env::var_os("USERPROFILE"))
            .map(PathBuf::from)
            .unwrap_or_else(|| PathBuf::from("."));
        if let Some(override_home) =
            std::env::var_os("VAKCODER_HOME").filter(|value| !value.is_empty())
        {
            let data = PathBuf::from(override_home);
            return (data.clone(), data.join("cache"), data.join("logs"));
        }
        #[cfg(target_os = "macos")]
        {
            (
                base.join("Library/Application Support/vakcoder"),
                base.join("Library/Caches/vakcoder"),
                base.join("Library/Logs/vakcoder"),
            )
        }
        #[cfg(not(target_os = "macos"))]
        {
            let xdg = |key: &str, suffix: &str| {
                std::env::var_os(key)
                    .filter(|value| !value.is_empty())
                    .map(PathBuf::from)
                    .unwrap_or_else(|| base.join(suffix))
                    .join("vakcoder")
            };
            (
                xdg("XDG_DATA_HOME", ".local/share"),
                xdg("XDG_CACHE_HOME", ".cache"),
                xdg("XDG_STATE_HOME", ".local/state").join("logs"),
            )
        }
    }
}

static TEMP_COUNTER: AtomicU64 = AtomicU64::new(0);
const LOCK_WAIT: Duration = Duration::from_millis(20);
const LOCK_TIMEOUT: Duration = Duration::from_secs(5);

#[derive(Debug, Error)]
pub enum ConfigError {
    #[error("I/O error at {path}: {source}")]
    Io { path: PathBuf, source: io::Error },
    #[error("invalid TOML at {path}: {source}")]
    Parse {
        path: PathBuf,
        source: toml::de::Error,
    },
    #[error("cannot serialize configuration: {0}")]
    Serialize(#[from] toml::ser::Error),
    #[error("timed out acquiring lock {0}")]
    LockTimeout(PathBuf),
    #[error("configuration revision conflict: expected {expected}, actual {actual}")]
    RevisionConflict { expected: u64, actual: u64 },
    #[error("project configuration scope is not registered: {0}")]
    UnregisteredProject(PathBuf),
    #[error("invalid environment entry: {0}")]
    InvalidEnv(String),
}

fn io_err(path: &Path, source: io::Error) -> ConfigError {
    ConfigError::Io {
        path: path.to_path_buf(),
        source,
    }
}

#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
pub struct Config {
    #[serde(default)]
    pub provider: ProviderConfig,
    #[serde(default)]
    pub model: ModelConfig,
    #[serde(default)]
    pub permission: PermissionConfig,
    #[serde(default)]
    pub sandbox: SandboxConfig,
    #[serde(default)]
    pub limits: LimitsConfig,
    #[serde(default)]
    pub connect: ConnectConfig,
}

#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
pub struct ProviderConfig {
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default)]
    pub endpoint: Option<String>,
}

#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
pub struct ModelConfig {
    #[serde(default)]
    pub name: Option<String>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct PermissionConfig {
    #[serde(default = "default_permission")]
    pub mode: PermissionMode,
}

impl Default for PermissionConfig {
    fn default() -> Self {
        Self {
            mode: default_permission(),
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PermissionMode {
    ReadOnly,
    WorkspaceWrite,
    FullAccess,
}

fn default_permission() -> PermissionMode {
    PermissionMode::ReadOnly
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct SandboxConfig {
    #[serde(default = "default_sandbox")]
    pub backend: SandboxBackend,
}

impl Default for SandboxConfig {
    fn default() -> Self {
        Self {
            backend: default_sandbox(),
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SandboxBackend {
    Seatbelt,
    Landlock,
    Docker,
    None,
}

fn default_sandbox() -> SandboxBackend {
    SandboxBackend::Seatbelt
}

#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
pub struct LimitsConfig {
    #[serde(default)]
    pub max_turns: Option<u32>,
    #[serde(default)]
    pub context_tokens: Option<u32>,
    #[serde(default)]
    pub budget_cents: Option<u64>,
}

#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
pub struct ConnectConfig {
    #[serde(default)]
    pub url: Option<String>,
    #[serde(default)]
    pub token: Option<String>,
    #[serde(default)]
    pub profiles: BTreeMap<String, ConnectProfile>,
}

#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
pub struct ConnectProfile {
    pub url: String,
    pub token: String,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ConfigSnapshot {
    pub revision: u64,
    pub config: Config,
    pub warnings: Vec<String>,
}

#[derive(Clone, Debug)]
pub struct ConfigService {
    data_home: PathBuf,
    project_root: PathBuf,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ConfigScope<'a> {
    Global,
    Project(&'a Path),
}

impl ConfigService {
    pub fn new(data_home: impl Into<PathBuf>, project_root: impl Into<PathBuf>) -> Self {
        Self {
            data_home: data_home.into(),
            project_root: project_root.into(),
        }
    }

    pub fn data_home(&self) -> &Path {
        &self.data_home
    }
    pub fn global_path(&self) -> PathBuf {
        self.data_home.join("config.toml")
    }
    pub fn project_path(&self) -> PathBuf {
        self.project_root.join(".vakcoder/project.toml")
    }

    pub fn load(&self) -> Result<ConfigSnapshot, ConfigError> {
        let global_path = self.global_path();
        let project_path = self.project_path();
        let (mut value, mut warnings, revision) = if global_path.exists() {
            let text = read_text(&global_path)?;
            let value: toml::Value = text.parse().map_err(|source| ConfigError::Parse {
                path: global_path.clone(),
                source,
            })?;
            let revision = value
                .get("revision")
                .and_then(toml::Value::as_integer)
                .unwrap_or(0)
                .max(0) as u64;
            let warnings = unknown_keys(&value, &known_keys());
            (value, warnings, revision)
        } else {
            (toml::Value::Table(toml::map::Map::new()), Vec::new(), 0)
        };

        if project_path.exists() {
            let text = read_text(&project_path)?;
            let project: toml::Value = text.parse().map_err(|source| ConfigError::Parse {
                path: project_path.clone(),
                source,
            })?;
            warnings.extend(unknown_keys(&project, &known_keys()));
            merge_values(&mut value, project);
        }
        let mut config: Config = value.try_into().map_err(|source| ConfigError::Parse {
            path: global_path.clone(),
            source,
        })?;
        let secrets = SecretService::new(&self.data_home);
        apply_environment(&mut config, &secrets);
        warnings.sort();
        warnings.dedup();
        Ok(ConfigSnapshot {
            revision,
            config,
            warnings,
        })
    }

    /// Atomically updates only the global layer and returns its new revision.
    pub fn update<F>(&self, update: F) -> Result<ConfigSnapshot, ConfigError>
    where
        F: FnOnce(&mut Config),
    {
        fs::create_dir_all(&self.data_home).map_err(|e| io_err(&self.data_home, e))?;
        let snapshot = self.load_unlocked()?;
        self.update_global(snapshot.revision, update)
    }

    pub fn update_global<F>(
        &self,
        expected_revision: u64,
        update: F,
    ) -> Result<ConfigSnapshot, ConfigError>
    where
        F: FnOnce(&mut Config),
    {
        self.update_scope(ConfigScope::Global, expected_revision, update)
    }

    pub fn update_project<F>(
        &self,
        project_root: impl AsRef<Path>,
        expected_revision: u64,
        update: F,
    ) -> Result<ConfigSnapshot, ConfigError>
    where
        F: FnOnce(&mut Config),
    {
        let root = project_root.as_ref();
        if root != self.project_root {
            return Err(ConfigError::UnregisteredProject(root.to_path_buf()));
        }
        self.update_scope(
            ConfigScope::Project(&self.project_root),
            expected_revision,
            update,
        )
    }

    pub fn update_scope<F>(
        &self,
        scope: ConfigScope<'_>,
        expected_revision: u64,
        update: F,
    ) -> Result<ConfigSnapshot, ConfigError>
    where
        F: FnOnce(&mut Config),
    {
        let project_scoped = matches!(scope, ConfigScope::Project(_));
        let path = match scope {
            ConfigScope::Global => self.global_path(),
            ConfigScope::Project(root) => root.join(".vakcoder/project.toml"),
        };
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).map_err(|e| io_err(parent, e))?;
        }
        let _lock = LockFile::acquire(&path.with_extension("toml.lock"))?;
        let raw = if path.exists() {
            read_text(&path)?
                .parse::<toml::Value>()
                .map_err(|source| ConfigError::Parse {
                    path: path.clone(),
                    source,
                })?
        } else {
            toml::Value::Table(toml::map::Map::new())
        };
        let actual = raw
            .get("revision")
            .and_then(toml::Value::as_integer)
            .unwrap_or(0)
            .max(0) as u64;
        if actual != expected_revision {
            return Err(ConfigError::RevisionConflict {
                expected: expected_revision,
                actual,
            });
        }
        let mut config: Config = raw
            .clone()
            .try_into()
            .map_err(|source| ConfigError::Parse {
                path: path.clone(),
                source,
            })?;
        update(&mut config);
        let mut next = raw;
        let typed = toml::Value::try_from(&config)?;
        merge_known(&mut next, typed);
        if let toml::Value::Table(table) = &mut next {
            table.insert(
                "revision".into(),
                toml::Value::Integer(actual.saturating_add(1) as i64),
            );
        }
        atomic_write(&path, toml::to_string(&next)?.as_bytes())?;
        let mut snapshot = self.load()?;
        if project_scoped {
            snapshot.revision = actual.saturating_add(1);
        }
        Ok(snapshot)
    }

    fn load_unlocked(&self) -> Result<ConfigSnapshot, ConfigError> {
        self.load()
    }
}

fn known_keys() -> BTreeMap<&'static str, BTreeSet<&'static str>> {
    BTreeMap::from([
        ("provider", BTreeSet::from(["name", "endpoint"])),
        ("model", BTreeSet::from(["name"])),
        ("permission", BTreeSet::from(["mode"])),
        ("sandbox", BTreeSet::from(["backend"])),
        (
            "limits",
            BTreeSet::from(["max_turns", "context_tokens", "budget_cents"]),
        ),
        ("connect", BTreeSet::from(["url", "token", "profiles"])),
    ])
}

fn unknown_keys(value: &toml::Value, known: &BTreeMap<&str, BTreeSet<&str>>) -> Vec<String> {
    let Some(table) = value.as_table() else {
        return vec!["configuration root must be a table".into()];
    };
    let mut warnings = Vec::new();
    for key in table.keys() {
        if key != "revision" && !known.contains_key(key.as_str()) {
            warnings.push(format!("unknown configuration key: {key}"));
        }
    }
    for (section, keys) in known {
        if let Some(t) = table.get(*section).and_then(toml::Value::as_table) {
            for key in t.keys() {
                if !keys.contains(key.as_str()) {
                    warnings.push(format!("unknown configuration key: {section}.{key}"));
                }
            }
        }
    }
    warnings
}

fn merge_values(base: &mut toml::Value, overlay: toml::Value) {
    let (Some(base), Some(overlay)) = (base.as_table_mut(), overlay.as_table()) else {
        return;
    };
    for (key, value) in overlay {
        if let (Some(existing), Some(incoming)) = (base.get_mut(key), value.as_table())
            && existing.is_table()
        {
            merge_values(existing, toml::Value::Table(incoming.clone()));
            continue;
        }
        base.insert(key.clone(), value.clone());
    }
}

fn apply_environment(config: &mut Config, secrets: &SecretService) {
    if let Ok(Some(value)) = secrets.get("VAKCODER_PROVIDER") {
        config.provider.name = Some(value);
    }
    if let Ok(Some(value)) = secrets.get("VAKCODER_MODEL") {
        config.model.name = Some(value);
    }
    if let Ok(value) = std::env::var("VAKCODER_PROVIDER") {
        config.provider.name = Some(value);
    }
    if let Ok(value) = std::env::var("VAKCODER_MODEL") {
        config.model.name = Some(value);
    }
}

fn merge_known(base: &mut toml::Value, typed: toml::Value) {
    let (Some(base), Some(typed)) = (base.as_table_mut(), typed.as_table()) else {
        return;
    };
    for section in [
        "provider",
        "model",
        "permission",
        "sandbox",
        "limits",
        "connect",
    ] {
        let Some(incoming) = typed.get(section) else {
            continue;
        };
        if let Some(existing) = base.get_mut(section)
            && existing.is_table()
            && incoming.is_table()
        {
            merge_values(existing, incoming.clone());
            continue;
        }
        base.insert(section.to_string(), incoming.clone());
    }
}

fn read_text(path: &Path) -> Result<String, ConfigError> {
    fs::read_to_string(path).map_err(|source| io_err(path, source))
}

fn atomic_write(path: &Path, contents: &[u8]) -> Result<(), ConfigError> {
    let parent = path.parent().unwrap_or_else(|| Path::new("."));
    fs::create_dir_all(parent).map_err(|e| io_err(parent, e))?;
    let id = TEMP_COUNTER.fetch_add(1, Ordering::Relaxed);
    let temp = parent.join(format!(
        ".{}.tmp.{}.{}",
        path.file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("config"),
        std::process::id(),
        id
    ));
    let result = (|| {
        let mut file = File::create(&temp).map_err(|e| io_err(&temp, e))?;
        file.write_all(contents).map_err(|e| io_err(&temp, e))?;
        file.sync_all().map_err(|e| io_err(&temp, e))?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            fs::set_permissions(&temp, fs::Permissions::from_mode(0o600))
                .map_err(|e| io_err(&temp, e))?;
        }
        fs::rename(&temp, path).map_err(|e| io_err(path, e))
    })();
    if result.is_err() {
        let _ = fs::remove_file(&temp);
    }
    result
}

struct LockFile {
    path: PathBuf,
}
impl LockFile {
    fn acquire(path: &Path) -> Result<Self, ConfigError> {
        let start = SystemTime::now();
        loop {
            match OpenOptions::new().write(true).create_new(true).open(path) {
                Ok(mut file) => {
                    let _ = writeln!(file, "{}", std::process::id());
                    return Ok(Self {
                        path: path.to_path_buf(),
                    });
                }
                Err(e) if e.kind() == io::ErrorKind::AlreadyExists => {
                    if start.elapsed().unwrap_or(LOCK_TIMEOUT) >= LOCK_TIMEOUT {
                        return Err(ConfigError::LockTimeout(path.to_path_buf()));
                    }
                    thread::sleep(LOCK_WAIT);
                }
                Err(source) => return Err(io_err(path, source)),
            }
        }
    }
}
impl Drop for LockFile {
    fn drop(&mut self) {
        let _ = fs::remove_file(&self.path);
    }
}

/// Atomic, locked key/value access to the canonical `<data_home>/.env` file.
#[derive(Clone, Debug)]
pub struct SecretService {
    path: PathBuf,
}
impl SecretService {
    pub fn new(data_home: impl AsRef<Path>) -> Self {
        Self {
            path: data_home.as_ref().join(".env"),
        }
    }
    pub fn path(&self) -> &Path {
        &self.path
    }
    pub fn get(&self, key: &str) -> Result<Option<String>, ConfigError> {
        let entries = self.read_entries()?;
        Ok(entries.get(key).cloned())
    }
    pub fn set(&self, key: &str, value: &str) -> Result<(), ConfigError> {
        validate_key(key)?;
        let parent = self.path.parent().unwrap_or_else(|| Path::new("."));
        fs::create_dir_all(parent).map_err(|e| io_err(parent, e))?;
        let _lock = LockFile::acquire(&self.path.with_extension("env.lock"))?;
        let mut entries = self.read_entries()?;
        entries.insert(key.to_string(), value.to_string());
        atomic_write(&self.path, render_env(&entries).as_bytes())
    }
    pub fn remove(&self, key: &str) -> Result<bool, ConfigError> {
        validate_key(key)?;
        let _lock = LockFile::acquire(&self.path.with_extension("env.lock"))?;
        let mut entries = self.read_entries()?;
        let removed = entries.remove(key).is_some();
        if removed {
            atomic_write(&self.path, render_env(&entries).as_bytes())?;
        }
        Ok(removed)
    }
    fn read_entries(&self) -> Result<BTreeMap<String, String>, ConfigError> {
        if !self.path.exists() {
            return Ok(BTreeMap::new());
        }
        let text = read_text(&self.path)?;
        let mut result = BTreeMap::new();
        for (line_no, line) in text.lines().enumerate() {
            let line = line.trim();
            if line.is_empty() || line.starts_with('#') {
                continue;
            }
            let Some((key, value)) = line.split_once('=') else {
                return Err(ConfigError::InvalidEnv(format!("line {}", line_no + 1)));
            };
            validate_key(key.trim())?;
            result.insert(key.trim().to_string(), parse_env_value(value.trim()));
        }
        Ok(result)
    }
}
fn validate_key(key: &str) -> Result<(), ConfigError> {
    if key.is_empty() || !key.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_') {
        return Err(ConfigError::InvalidEnv(key.to_string()));
    }
    Ok(())
}
fn parse_env_value(value: &str) -> String {
    if value.len() >= 2 && value.starts_with('"') && value.ends_with('"') {
        let mut out = String::new();
        let mut escaped = false;
        for ch in value[1..value.len() - 1].chars() {
            if escaped {
                out.push(match ch {
                    'n' => '\n',
                    'r' => '\r',
                    '"' => '"',
                    '\\' => '\\',
                    other => other,
                });
                escaped = false;
            } else if ch == '\\' {
                escaped = true;
            } else {
                out.push(ch);
            }
        }
        if escaped {
            out.push('\\');
        }
        return out;
    }
    if value.len() >= 2 && value.starts_with('\'') && value.ends_with('\'') {
        return value[1..value.len() - 1].replace("'\\''", "'");
    }
    value.to_string()
}
fn render_env(entries: &BTreeMap<String, String>) -> String {
    entries
        .iter()
        .map(|(k, v)| {
            format!(
                "{k}=\"{}\"\n",
                v.replace('\\', "\\\\")
                    .replace('"', "\\\"")
                    .replace('\n', "\\n")
                    .replace('\r', "\\r")
            )
        })
        .collect()
}
