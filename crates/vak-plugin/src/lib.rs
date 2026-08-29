//! Secure plugin package inspection and immutable local installation.

use std::collections::{BTreeMap, BTreeSet};
use std::fs::{self, File, OpenOptions};
use std::io::{Read, Write};
use std::path::{Component, Path, PathBuf};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use semver::Version;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use walkdir::WalkDir;

pub const REGISTRY_SCHEMA: u32 = 1;

#[derive(Debug, thiserror::Error)]
pub enum PluginError {
    #[error("plugin package not found: {0}")]
    NotFound(PathBuf),
    #[error("plugin package has no supported manifest or SKILL.md at {0}")]
    ManifestMissing(PathBuf),
    #[error("plugin manifest is invalid: {0}")]
    InvalidManifest(String),
    #[error("unsafe plugin package: {0}")]
    UnsafePackage(String),
    #[error("plugin package exceeds limit: {0}")]
    LimitExceeded(String),
    #[error(
        "plugin '{0}' is already installed with different content; remove it before installing another generation"
    )]
    AlreadyInstalled(String),
    #[error("plugin '{0}' is not installed")]
    NotInstalled(String),
    #[error(
        "plugin '{0}' has no declared license; pass allow_unlicensed only after reviewing its terms"
    )]
    Unlicensed(String),
    #[error("plugin registry is busy; another mutation may be in progress")]
    RegistryBusy,
    #[error("plugin I/O failed at {path}: {source}")]
    Io {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error("plugin JSON failed at {path}: {source}")]
    Json {
        path: PathBuf,
        #[source]
        source: serde_json::Error,
    },
}

fn io_error(path: impl Into<PathBuf>, source: std::io::Error) -> PluginError {
    PluginError::Io {
        path: path.into(),
        source,
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum ManifestFormat {
    Vak,
    Codex,
    AgentPlugin,
    Claude,
    Copilot,
    Cursor,
    Gemini,
    AgentSkill,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Publisher {
    pub id: String,
    pub name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub url: Option<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct Components {
    pub skills: Vec<String>,
    pub commands: Vec<String>,
    pub mcp: Vec<String>,
    pub hooks: Vec<String>,
    pub agents: Vec<String>,
    pub rules: Vec<String>,
    pub lsp: Vec<String>,
    pub policies: Vec<String>,
    pub themes: Vec<String>,
    pub presentation: Vec<String>,
    pub assets: Vec<String>,
}

impl Components {
    fn declared_paths(&self) -> impl Iterator<Item = (&'static str, &String)> {
        self.skills
            .iter()
            .map(|path| ("skill", path))
            .chain(self.commands.iter().map(|path| ("command", path)))
            .chain(self.mcp.iter().map(|path| ("MCP", path)))
            .chain(self.hooks.iter().map(|path| ("hook", path)))
            .chain(self.agents.iter().map(|path| ("agent", path)))
            .chain(self.rules.iter().map(|path| ("rule", path)))
            .chain(self.lsp.iter().map(|path| ("LSP", path)))
            .chain(self.policies.iter().map(|path| ("policy", path)))
            .chain(self.themes.iter().map(|path| ("theme", path)))
            .chain(self.presentation.iter().map(|path| ("presentation", path)))
            .chain(self.assets.iter().map(|path| ("asset", path)))
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PluginManifest {
    pub schema: u32,
    pub name: String,
    pub version: String,
    pub description: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub license: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub publisher: Option<Publisher>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub homepage: Option<String>,
    #[serde(default)]
    pub components: Components,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PackageFile {
    pub path: String,
    pub bytes: u64,
    pub executable: bool,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct CapabilityInventory {
    pub skills: Vec<String>,
    pub commands: Vec<String>,
    pub mcp_manifests: Vec<String>,
    pub hooks: Vec<String>,
    pub agents: Vec<String>,
    pub rules: Vec<String>,
    pub lsp_manifests: Vec<String>,
    pub policies: Vec<String>,
    pub themes: Vec<String>,
    pub presentation: Vec<String>,
    pub scripts: Vec<String>,
    pub executables: Vec<String>,
    pub assets: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PackageInspection {
    pub manifest: PluginManifest,
    pub format: ManifestFormat,
    pub root: PathBuf,
    pub digest: String,
    pub file_count: usize,
    pub total_bytes: u64,
    pub files: Vec<PackageFile>,
    pub capabilities: CapabilityInventory,
    pub warnings: Vec<String>,
}

#[derive(Debug, Clone, Copy)]
pub struct InspectLimits {
    pub max_files: usize,
    pub max_total_bytes: u64,
    pub max_file_bytes: u64,
    pub max_depth: usize,
}

impl Default for InspectLimits {
    fn default() -> Self {
        Self {
            max_files: 4_096,
            max_total_bytes: 64 * 1024 * 1024,
            max_file_bytes: 16 * 1024 * 1024,
            max_depth: 16,
        }
    }
}

pub fn inspect_package(root: &Path) -> Result<PackageInspection, PluginError> {
    inspect_package_with_limits(root, InspectLimits::default())
}

pub fn inspect_package_with_limits(
    root: &Path,
    limits: InspectLimits,
) -> Result<PackageInspection, PluginError> {
    if !root.exists() {
        return Err(PluginError::NotFound(root.to_path_buf()));
    }
    if !root.is_dir() {
        return Err(PluginError::UnsafePackage(format!(
            "package root must be a directory: {}",
            root.display()
        )));
    }
    let root = fs::canonicalize(root).map_err(|error| io_error(root, error))?;
    let (mut manifest, format, manifest_warnings) = load_manifest(&root)?;
    normalize_and_validate_manifest(&mut manifest, &root)?;
    let files = collect_files(&root, limits)?;
    let total_bytes = files.iter().map(|file| file.bytes).sum();
    let digest = digest_files(&root, &files)?;
    let capabilities = inventory_capabilities(&root, &manifest.components, &files)?;
    let mut warnings = manifest_warnings;
    if manifest.license.is_none() {
        warnings.push(
            "no license declared; marketplace availability does not grant redistribution rights"
                .into(),
        );
    }
    if manifest.publisher.is_none() {
        warnings.push("no publisher identity declared".into());
    }
    Ok(PackageInspection {
        manifest,
        format,
        root,
        digest,
        file_count: files.len(),
        total_bytes,
        files,
        capabilities,
        warnings,
    })
}

fn load_manifest(
    root: &Path,
) -> Result<(PluginManifest, ManifestFormat, Vec<String>), PluginError> {
    let native_path = root.join("vak-plugin.json");
    if native_path.is_file() {
        let bytes = fs::read(&native_path).map_err(|error| io_error(&native_path, error))?;
        let manifest = serde_json::from_slice(&bytes).map_err(|source| PluginError::Json {
            path: native_path,
            source,
        })?;
        return Ok((manifest, ManifestFormat::Vak, Vec::new()));
    }

    let codex_path = root.join(".codex-plugin/plugin.json");
    if codex_path.is_file() {
        let bytes = fs::read(&codex_path).map_err(|error| io_error(&codex_path, error))?;
        let value: serde_json::Value =
            serde_json::from_slice(&bytes).map_err(|source| PluginError::Json {
                path: codex_path,
                source,
            })?;
        return normalize_client_manifest(root, &value, ManifestFormat::Codex);
    }

    for (relative, format) in [
        (".claude-plugin/plugin.json", ManifestFormat::Claude),
        (".cursor-plugin/plugin.json", ManifestFormat::Cursor),
        (".github/plugin/plugin.json", ManifestFormat::Copilot),
        (".plugin/plugin.json", ManifestFormat::Copilot),
    ] {
        let path = root.join(relative);
        if path.is_file() {
            let bytes = fs::read(&path).map_err(|error| io_error(&path, error))?;
            let value = serde_json::from_slice(&bytes)
                .map_err(|source| PluginError::Json { path, source })?;
            return normalize_client_manifest(root, &value, format);
        }
    }

    let agent_path = root.join("plugin.json");
    if agent_path.is_file() {
        let bytes = fs::read(&agent_path).map_err(|error| io_error(&agent_path, error))?;
        let value: serde_json::Value =
            serde_json::from_slice(&bytes).map_err(|source| PluginError::Json {
                path: agent_path,
                source,
            })?;
        let format = if value.get("$schema").and_then(serde_json::Value::as_str)
            == Some("https://agent-plugins.org/schemas/1.0.0/plugin.schema.json")
        {
            ManifestFormat::AgentPlugin
        } else {
            ManifestFormat::Copilot
        };
        return normalize_client_manifest(root, &value, format);
    }

    let gemini_path = root.join("gemini-extension.json");
    if gemini_path.is_file() {
        let bytes = fs::read(&gemini_path).map_err(|error| io_error(&gemini_path, error))?;
        let value = serde_json::from_slice(&bytes).map_err(|source| PluginError::Json {
            path: gemini_path,
            source,
        })?;
        return normalize_client_manifest(root, &value, ManifestFormat::Gemini);
    }

    let skill_path = root.join("SKILL.md");
    if skill_path.is_file() {
        let (name, description) = parse_skill_header(&skill_path)?;
        return Ok((
            PluginManifest {
                schema: REGISTRY_SCHEMA,
                name,
                version: "0.0.0+local".into(),
                description,
                license: None,
                publisher: None,
                homepage: None,
                components: Components {
                    skills: vec![".".into()],
                    ..Components::default()
                },
            },
            ManifestFormat::AgentSkill,
            vec!["standalone Agent Skill normalized as a local skills-only plugin".into()],
        ));
    }

    Err(PluginError::ManifestMissing(root.to_path_buf()))
}

fn normalize_client_manifest(
    root: &Path,
    value: &serde_json::Value,
    format: ManifestFormat,
) -> Result<(PluginManifest, ManifestFormat, Vec<String>), PluginError> {
    let object = value
        .as_object()
        .ok_or_else(|| PluginError::InvalidManifest("plugin manifest must be an object".into()))?;
    if format == ManifestFormat::AgentPlugin {
        let schema = object.get("$schema").and_then(serde_json::Value::as_str);
        if schema != Some("https://agent-plugins.org/schemas/1.0.0/plugin.schema.json") {
            return Err(PluginError::InvalidManifest(
                "Agent Plugins 1.0 requires its canonical $schema".into(),
            ));
        }
    }
    let name = required_string(object, "name")?;
    let source_version = object
        .get("version")
        .and_then(serde_json::Value::as_str)
        .unwrap_or("0.0.0+local")
        .to_string();
    let (version, version_warning) = normalize_external_version(&source_version);
    let description = object
        .get("description")
        .and_then(serde_json::Value::as_str)
        .unwrap_or("")
        .to_string();
    let license = object
        .get("license")
        .and_then(serde_json::Value::as_str)
        .map(str::to_string);
    let homepage = object
        .get("homepage")
        .and_then(serde_json::Value::as_str)
        .map(str::to_string);
    let publisher = normalize_publisher(object.get("publisher").or_else(|| object.get("author")))?;
    let mut components = Components {
        skills: string_or_strings(object.get("skills"), "skills")?,
        commands: string_or_strings(object.get("commands"), "commands")?,
        mcp: string_or_strings(object.get("mcp"), "mcp")?,
        hooks: paths_or_inline(object.get("hooks"), "hooks", manifest_relative_path(format))?,
        agents: string_or_strings(object.get("agents"), "agents")?,
        rules: string_or_strings(object.get("rules"), "rules")?,
        lsp: paths_or_inline(
            object.get("lspServers"),
            "lspServers",
            manifest_relative_path(format),
        )?,
        policies: string_or_strings(object.get("policies"), "policies")?,
        themes: paths_or_inline(
            object.get("themes"),
            "themes",
            manifest_relative_path(format),
        )?,
        presentation: string_or_strings(object.get("presentation"), "presentation")?,
        assets: string_or_strings(object.get("assets"), "assets")?,
    };
    if object.get("mcpServers").is_some() {
        components
            .mcp
            .push(manifest_relative_path(format).to_string());
    }
    if format == ManifestFormat::AgentPlugin {
        components = Components::default();
        add_conventional_component(root, "skills", &mut components.skills);
        add_conventional_component(root, "mcp.json", &mut components.mcp);
    }
    add_conventional_component(root, "skills", &mut components.skills);
    add_conventional_component(root, "commands", &mut components.commands);
    add_conventional_component(root, ".mcp.json", &mut components.mcp);
    add_conventional_component(root, ".github/mcp.json", &mut components.mcp);
    add_conventional_component(root, "mcp.json", &mut components.mcp);
    add_conventional_component(root, "hooks.json", &mut components.hooks);
    add_conventional_component(root, "hooks/hooks.json", &mut components.hooks);
    add_conventional_component(root, "agents", &mut components.agents);
    add_conventional_component(root, "rules", &mut components.rules);
    add_conventional_component(root, "lsp.json", &mut components.lsp);
    add_conventional_component(root, ".github/lsp.json", &mut components.lsp);
    add_conventional_component(root, "policies", &mut components.policies);
    add_conventional_component(root, "themes", &mut components.themes);
    add_conventional_component(root, "presentation", &mut components.presentation);
    add_conventional_component(root, "assets", &mut components.assets);
    let known: BTreeSet<&str> = [
        "name",
        "version",
        "description",
        "license",
        "publisher",
        "author",
        "homepage",
        "skills",
        "commands",
        "mcp",
        "mcpServers",
        "hooks",
        "agents",
        "rules",
        "lspServers",
        "policies",
        "themes",
        "presentation",
        "assets",
        "apps",
        "$schema",
        "extensions",
        "category",
        "tags",
        "variables",
        "settings",
        "contextFileName",
        "excludeTools",
        "migratedTo",
        "plan",
    ]
    .into_iter()
    .collect();
    let unknown = object
        .keys()
        .filter(|key| !known.contains(key.as_str()))
        .cloned()
        .collect::<Vec<_>>();
    let mut warnings = if unknown.is_empty() {
        Vec::new()
    } else {
        vec![format!(
            "ignored unsupported declarative plugin manifest keys: {}",
            unknown.join(", ")
        )]
    };
    if let Some(warning) = version_warning {
        warnings.push(warning);
    }
    Ok((
        PluginManifest {
            schema: REGISTRY_SCHEMA,
            name,
            version,
            description,
            license,
            publisher,
            homepage,
            components,
        },
        format,
        warnings,
    ))
}

fn normalize_external_version(source: &str) -> (String, Option<String>) {
    if Version::parse(source).is_ok() {
        return (source.to_string(), None);
    }
    let digest = Sha256::digest(source.as_bytes());
    let normalized = format!("0.0.0+source.{}", &hex::encode(digest)[..12]);
    (
        normalized.clone(),
        Some(format!(
            "source version {source:?} is not semantic; normalized internally as {normalized}"
        )),
    )
}

fn manifest_relative_path(format: ManifestFormat) -> &'static str {
    match format {
        ManifestFormat::Codex => ".codex-plugin/plugin.json",
        ManifestFormat::Claude => ".claude-plugin/plugin.json",
        ManifestFormat::Cursor => ".cursor-plugin/plugin.json",
        ManifestFormat::Copilot => "plugin.json",
        ManifestFormat::Gemini => "gemini-extension.json",
        ManifestFormat::AgentPlugin => "plugin.json",
        ManifestFormat::Vak | ManifestFormat::AgentSkill => "vak-plugin.json",
    }
}

fn required_string(
    object: &serde_json::Map<String, serde_json::Value>,
    field: &str,
) -> Result<String, PluginError> {
    object
        .get(field)
        .and_then(serde_json::Value::as_str)
        .filter(|value| !value.trim().is_empty())
        .map(str::to_string)
        .ok_or_else(|| PluginError::InvalidManifest(format!("missing non-empty '{field}'")))
}

fn normalize_publisher(
    value: Option<&serde_json::Value>,
) -> Result<Option<Publisher>, PluginError> {
    let Some(value) = value else {
        return Ok(None);
    };
    if let Some(name) = value.as_str() {
        let id = normalize_id(name).ok_or_else(|| {
            PluginError::InvalidManifest("publisher string cannot form a stable id".into())
        })?;
        return Ok(Some(Publisher {
            id,
            name: name.to_string(),
            url: None,
        }));
    }
    let object = value.as_object().ok_or_else(|| {
        PluginError::InvalidManifest("publisher must be a string or object".into())
    })?;
    let name = required_string(object, "name")?;
    let id = object
        .get("id")
        .and_then(serde_json::Value::as_str)
        .and_then(normalize_id)
        .or_else(|| normalize_id(&name))
        .ok_or_else(|| PluginError::InvalidManifest("publisher needs a valid id".into()))?;
    let url = object
        .get("url")
        .and_then(serde_json::Value::as_str)
        .map(str::to_string);
    Ok(Some(Publisher { id, name, url }))
}

fn string_or_strings(
    value: Option<&serde_json::Value>,
    field: &str,
) -> Result<Vec<String>, PluginError> {
    let Some(value) = value else {
        return Ok(Vec::new());
    };
    if let Some(value) = value.as_str() {
        return Ok(vec![value.to_string()]);
    }
    value
        .as_array()
        .ok_or_else(|| {
            PluginError::InvalidManifest(format!("'{field}' must be a string or string array"))
        })?
        .iter()
        .map(|item| {
            item.as_str().map(str::to_string).ok_or_else(|| {
                PluginError::InvalidManifest(format!("'{field}' contains a non-string path"))
            })
        })
        .collect()
}

fn paths_or_inline(
    value: Option<&serde_json::Value>,
    field: &str,
    manifest_path: &str,
) -> Result<Vec<String>, PluginError> {
    let Some(value) = value else {
        return Ok(Vec::new());
    };
    if value.is_object()
        || value
            .as_array()
            .is_some_and(|items| items.iter().any(|item| !item.is_string()))
    {
        return Ok(vec![manifest_path.to_string()]);
    }
    string_or_strings(Some(value), field)
}

fn add_conventional_component(root: &Path, relative: &str, values: &mut Vec<String>) {
    if root.join(relative).exists() && !values.iter().any(|value| value == relative) {
        values.push(relative.to_string());
    }
}

fn normalize_and_validate_manifest(
    manifest: &mut PluginManifest,
    root: &Path,
) -> Result<(), PluginError> {
    manifest.name = normalize_plugin_id(&manifest.name).ok_or_else(|| {
        PluginError::InvalidManifest(
            "name must use lowercase letters, digits, dashes, or non-repeated dots".into(),
        )
    })?;
    Version::parse(&manifest.version).map_err(|error| {
        PluginError::InvalidManifest(format!(
            "version '{}' is not semver: {error}",
            manifest.version
        ))
    })?;
    if manifest.schema > REGISTRY_SCHEMA {
        return Err(PluginError::InvalidManifest(format!(
            "schema {} is newer than supported schema {REGISTRY_SCHEMA}",
            manifest.schema
        )));
    }
    for (kind, declared) in manifest.components.declared_paths() {
        let relative = validate_relative_path(declared)?;
        let path = root.join(relative);
        if !path.exists() {
            return Err(PluginError::InvalidManifest(format!(
                "declared {kind} path does not exist: {declared}"
            )));
        }
    }
    Ok(())
}

fn normalize_id(value: &str) -> Option<String> {
    let value = value.trim();
    if value.is_empty()
        || value.starts_with('-')
        || value.ends_with('-')
        || !value.chars().all(|character| {
            character.is_ascii_lowercase() || character.is_ascii_digit() || character == '-'
        })
    {
        return None;
    }
    Some(value.to_string())
}

fn normalize_plugin_id(value: &str) -> Option<String> {
    let value = value.trim();
    if value.is_empty()
        || value.len() > 64
        || !value
            .chars()
            .next()
            .is_some_and(|character| character.is_ascii_alphanumeric())
        || !value
            .chars()
            .last()
            .is_some_and(|character| character.is_ascii_alphanumeric())
        || value.contains("--")
        || value.contains("..")
        || !value.chars().all(|character| {
            character.is_ascii_lowercase()
                || character.is_ascii_digit()
                || character == '-'
                || character == '.'
        })
    {
        return None;
    }
    Some(value.to_string())
}

fn validate_relative_path(value: &str) -> Result<&Path, PluginError> {
    let path = Path::new(value);
    if value.is_empty() || path.is_absolute() {
        return Err(PluginError::UnsafePackage(format!(
            "component path must be relative: {value:?}"
        )));
    }
    for component in path.components() {
        if !matches!(component, Component::Normal(_) | Component::CurDir) {
            return Err(PluginError::UnsafePackage(format!(
                "component path escapes package root: {value:?}"
            )));
        }
    }
    Ok(path)
}

fn collect_files(root: &Path, limits: InspectLimits) -> Result<Vec<PackageFile>, PluginError> {
    let mut files = Vec::new();
    let mut total_bytes = 0_u64;
    for entry in WalkDir::new(root).follow_links(false) {
        let entry = entry.map_err(|error| {
            PluginError::UnsafePackage(format!("could not walk package: {error}"))
        })?;
        let depth = entry.depth();
        if depth > limits.max_depth {
            return Err(PluginError::LimitExceeded(format!(
                "path depth {depth} exceeds {} at {}",
                limits.max_depth,
                entry.path().display()
            )));
        }
        if depth == 0 {
            continue;
        }
        let metadata =
            fs::symlink_metadata(entry.path()).map_err(|error| io_error(entry.path(), error))?;
        if metadata.file_type().is_symlink() {
            return Err(PluginError::UnsafePackage(format!(
                "symbolic links are not allowed: {}",
                entry.path().display()
            )));
        }
        if metadata.is_dir() {
            continue;
        }
        if !metadata.is_file() {
            return Err(PluginError::UnsafePackage(format!(
                "special files are not allowed: {}",
                entry.path().display()
            )));
        }
        reject_hard_link(entry.path(), &metadata)?;
        if metadata.len() > limits.max_file_bytes {
            return Err(PluginError::LimitExceeded(format!(
                "file {} is {} bytes; maximum is {}",
                entry.path().display(),
                metadata.len(),
                limits.max_file_bytes
            )));
        }
        total_bytes = total_bytes
            .checked_add(metadata.len())
            .ok_or_else(|| PluginError::LimitExceeded("expanded size overflow".into()))?;
        if total_bytes > limits.max_total_bytes {
            return Err(PluginError::LimitExceeded(format!(
                "expanded size {total_bytes} exceeds {} bytes",
                limits.max_total_bytes
            )));
        }
        if files.len() >= limits.max_files {
            return Err(PluginError::LimitExceeded(format!(
                "file count exceeds {}",
                limits.max_files
            )));
        }
        let relative = entry.path().strip_prefix(root).map_err(|_| {
            PluginError::UnsafePackage(format!(
                "walked path escaped package root: {}",
                entry.path().display()
            ))
        })?;
        files.push(PackageFile {
            path: portable_path(relative)?,
            bytes: metadata.len(),
            executable: is_executable(&metadata),
        });
    }
    files.sort_by(|left, right| left.path.cmp(&right.path));
    if files.is_empty() {
        return Err(PluginError::InvalidManifest(
            "package contains no files".into(),
        ));
    }
    Ok(files)
}

#[cfg(unix)]
fn reject_hard_link(path: &Path, metadata: &fs::Metadata) -> Result<(), PluginError> {
    use std::os::unix::fs::MetadataExt as _;
    if metadata.nlink() > 1 {
        return Err(PluginError::UnsafePackage(format!(
            "hard-linked files are not allowed: {}",
            path.display()
        )));
    }
    Ok(())
}

#[cfg(not(unix))]
fn reject_hard_link(_path: &Path, _metadata: &fs::Metadata) -> Result<(), PluginError> {
    Ok(())
}

#[cfg(unix)]
fn is_executable(metadata: &fs::Metadata) -> bool {
    use std::os::unix::fs::PermissionsExt as _;
    metadata.permissions().mode() & 0o111 != 0
}

#[cfg(not(unix))]
fn is_executable(_metadata: &fs::Metadata) -> bool {
    false
}

fn portable_path(path: &Path) -> Result<String, PluginError> {
    let mut parts = Vec::new();
    for component in path.components() {
        match component {
            Component::Normal(value) => parts.push(value.to_string_lossy().into_owned()),
            Component::CurDir => {}
            _ => {
                return Err(PluginError::UnsafePackage(format!(
                    "invalid relative path: {}",
                    path.display()
                )));
            }
        }
    }
    Ok(parts.join("/"))
}

fn digest_files(root: &Path, files: &[PackageFile]) -> Result<String, PluginError> {
    let mut digest = Sha256::new();
    for file in files {
        let path_bytes = file.path.as_bytes();
        digest.update((path_bytes.len() as u64).to_le_bytes());
        digest.update(path_bytes);
        digest.update(file.bytes.to_le_bytes());
        let path = root.join(path_from_portable(&file.path));
        let mut input = File::open(&path).map_err(|error| io_error(&path, error))?;
        let mut buffer = [0_u8; 64 * 1024];
        loop {
            let read = input
                .read(&mut buffer)
                .map_err(|error| io_error(&path, error))?;
            if read == 0 {
                break;
            }
            digest.update(&buffer[..read]);
        }
    }
    Ok(hex::encode(digest.finalize()))
}

fn path_from_portable(path: &str) -> PathBuf {
    path.split('/').collect()
}

fn inventory_capabilities(
    root: &Path,
    components: &Components,
    files: &[PackageFile],
) -> Result<CapabilityInventory, PluginError> {
    let mut inventory = CapabilityInventory::default();
    for skill_root in &components.skills {
        let relative = validate_relative_path(skill_root)?;
        let absolute = root.join(relative);
        if absolute.join("SKILL.md").is_file() {
            let (name, _) = parse_skill_header(&absolute.join("SKILL.md"))?;
            inventory.skills.push(name);
        } else if absolute.is_dir() {
            let mut names = fs::read_dir(&absolute)
                .map_err(|error| io_error(&absolute, error))?
                .filter_map(Result::ok)
                .filter(|entry| entry.path().join("SKILL.md").is_file())
                .map(|entry| {
                    parse_skill_header(&entry.path().join("SKILL.md")).map(|header| header.0)
                })
                .collect::<Result<Vec<_>, _>>()?;
            inventory.skills.append(&mut names);
        }
    }
    inventory.skills.sort();
    inventory.skills.dedup();
    inventory.commands = component_files(root, &components.commands, Some("md"))?;
    inventory.mcp_manifests = component_files(root, &components.mcp, None)?;
    inventory.hooks = component_files(root, &components.hooks, None)?;
    inventory.agents = component_files(root, &components.agents, Some("md"))?;
    inventory.rules = component_files(root, &components.rules, None)?;
    inventory.lsp_manifests = component_files(root, &components.lsp, None)?;
    inventory.policies = component_files(root, &components.policies, Some("toml"))?;
    inventory.themes = component_files(root, &components.themes, None)?;
    inventory.presentation = component_files(root, &components.presentation, None)?;
    inventory.assets = component_files(root, &components.assets, None)?;
    for file in files {
        let path = Path::new(&file.path);
        if path
            .components()
            .any(|component| matches!(component, Component::Normal(value) if value == "scripts"))
        {
            inventory.scripts.push(file.path.clone());
        }
        if file.executable {
            inventory.executables.push(file.path.clone());
        }
    }
    Ok(inventory)
}

fn component_files(
    root: &Path,
    declared: &[String],
    extension: Option<&str>,
) -> Result<Vec<String>, PluginError> {
    let mut output = Vec::new();
    for value in declared {
        let path = root.join(validate_relative_path(value)?);
        if path.is_file() {
            output.push(portable_path(path.strip_prefix(root).map_err(|_| {
                PluginError::UnsafePackage(format!("component escaped root: {}", path.display()))
            })?)?);
            continue;
        }
        for entry in WalkDir::new(&path).follow_links(false) {
            let entry = entry.map_err(|error| {
                PluginError::UnsafePackage(format!("could not inventory component: {error}"))
            })?;
            if !entry.file_type().is_file()
                || extension.is_some_and(|expected| {
                    entry
                        .path()
                        .extension()
                        .is_none_or(|actual| actual != expected)
                })
            {
                continue;
            }
            output.push(portable_path(entry.path().strip_prefix(root).map_err(
                |_| {
                    PluginError::UnsafePackage(format!(
                        "component escaped root: {}",
                        entry.path().display()
                    ))
                },
            )?)?);
        }
    }
    output.sort();
    output.dedup();
    Ok(output)
}

fn parse_skill_header(path: &Path) -> Result<(String, String), PluginError> {
    let text = fs::read_to_string(path).map_err(|error| io_error(path, error))?;
    let rest = text.strip_prefix("---").ok_or_else(|| {
        PluginError::InvalidManifest(format!("skill has no YAML frontmatter: {}", path.display()))
    })?;
    let (frontmatter, _) = rest.split_once("---").ok_or_else(|| {
        PluginError::InvalidManifest(format!(
            "skill frontmatter is not closed: {}",
            path.display()
        ))
    })?;
    let mut name = None;
    let mut description = String::new();
    for line in frontmatter.lines() {
        let line = line.trim();
        if let Some(value) = line.strip_prefix("name:") {
            name = Some(value.trim().trim_matches(['\'', '"']).to_string());
        } else if let Some(value) = line.strip_prefix("description:") {
            description = value.trim().trim_matches(['\'', '"']).to_string();
        }
    }
    let name = name
        .or_else(|| {
            path.parent()
                .and_then(Path::file_name)
                .map(|value| value.to_string_lossy().into_owned())
        })
        .and_then(|value| normalize_id(&value))
        .ok_or_else(|| {
            PluginError::InvalidManifest(format!(
                "skill needs a valid kebab-case name: {}",
                path.display()
            ))
        })?;
    Ok((name, description))
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum InstallScope {
    User,
    Workspace,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct InstalledPlugin {
    pub name: String,
    pub version: String,
    pub digest: String,
    pub description: String,
    pub license: Option<String>,
    pub publisher: Option<Publisher>,
    pub format: ManifestFormat,
    pub source: String,
    #[serde(default)]
    pub trace_id: String,
    pub package_path: PathBuf,
    pub scope: InstallScope,
    pub enabled: bool,
    pub installed_at_unix: u64,
    pub capabilities: CapabilityInventory,
    pub warnings: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PluginRegistry {
    pub schema: u32,
    pub generation: u64,
    pub plugins: BTreeMap<String, InstalledPlugin>,
    #[serde(default)]
    pub audit: Vec<PluginAuditEvent>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum PluginAuditAction {
    Installed,
    Removed,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PluginAuditEvent {
    pub generation: u64,
    pub at_unix: u64,
    pub action: PluginAuditAction,
    pub plugin: String,
    pub version: String,
    pub digest: String,
    pub trace_id: String,
    pub source: String,
}

impl Default for PluginRegistry {
    fn default() -> Self {
        Self {
            schema: REGISTRY_SCHEMA,
            generation: 0,
            plugins: BTreeMap::new(),
            audit: Vec::new(),
        }
    }
}

#[derive(Debug, Clone, Copy)]
pub struct InstallOptions {
    pub scope: InstallScope,
    pub allow_unlicensed: bool,
}

impl Default for InstallOptions {
    fn default() -> Self {
        Self {
            scope: InstallScope::User,
            allow_unlicensed: false,
        }
    }
}

pub struct PluginStore {
    home: PathBuf,
}

impl PluginStore {
    pub fn new(home: impl Into<PathBuf>) -> Self {
        Self { home: home.into() }
    }

    pub fn registry_path(&self) -> PathBuf {
        self.home.join("plugins/registry.json")
    }

    pub fn packages_root(&self) -> PathBuf {
        self.home.join("plugins/packages")
    }

    pub fn load(&self) -> Result<PluginRegistry, PluginError> {
        let path = self.registry_path();
        if !path.exists() {
            return Ok(PluginRegistry::default());
        }
        let bytes = fs::read(&path).map_err(|error| io_error(&path, error))?;
        let registry: PluginRegistry =
            serde_json::from_slice(&bytes).map_err(|source| PluginError::Json {
                path: path.clone(),
                source,
            })?;
        if registry.schema > REGISTRY_SCHEMA {
            return Err(PluginError::InvalidManifest(format!(
                "registry schema {} is newer than supported schema {REGISTRY_SCHEMA}",
                registry.schema
            )));
        }
        Ok(registry)
    }

    pub fn list(&self) -> Result<Vec<InstalledPlugin>, PluginError> {
        Ok(self.load()?.plugins.into_values().collect())
    }

    pub fn install_local(
        &self,
        source: &Path,
        options: InstallOptions,
    ) -> Result<InstalledPlugin, PluginError> {
        let inspection = inspect_package(source)?;
        if inspection.manifest.license.is_none() && !options.allow_unlicensed {
            return Err(PluginError::Unlicensed(inspection.manifest.name));
        }
        let _lock = RegistryLock::acquire(&self.home.join("plugins"))?;
        let mut registry = self.load()?;
        if let Some(existing) = registry.plugins.get(&inspection.manifest.name) {
            if existing.digest == inspection.digest {
                return Ok(existing.clone());
            }
            return Err(PluginError::AlreadyInstalled(
                inspection.manifest.name.clone(),
            ));
        }
        let package_path = self.package_path(&inspection);
        if package_path.exists() {
            let existing = inspect_package(&package_path)?;
            if existing.digest != inspection.digest {
                return Err(PluginError::UnsafePackage(format!(
                    "immutable package destination has unexpected content: {}",
                    package_path.display()
                )));
            }
        } else {
            self.copy_verified(&inspection, &package_path)?;
        }
        let installed_at_unix = now_unix();
        let source = inspection.root.display().to_string();
        let trace_id = format!("install:{}:{}", inspection.manifest.name, inspection.digest);
        let installed = InstalledPlugin {
            name: inspection.manifest.name.clone(),
            version: inspection.manifest.version.clone(),
            digest: inspection.digest,
            description: inspection.manifest.description,
            license: inspection.manifest.license,
            publisher: inspection.manifest.publisher,
            format: inspection.format,
            source: source.clone(),
            trace_id: trace_id.clone(),
            package_path,
            scope: options.scope,
            enabled: false,
            installed_at_unix,
            capabilities: inspection.capabilities,
            warnings: inspection.warnings,
        };
        registry
            .plugins
            .insert(installed.name.clone(), installed.clone());
        registry.generation = registry.generation.saturating_add(1);
        registry.audit.push(PluginAuditEvent {
            generation: registry.generation,
            at_unix: installed_at_unix,
            action: PluginAuditAction::Installed,
            plugin: installed.name.clone(),
            version: installed.version.clone(),
            digest: installed.digest.clone(),
            trace_id,
            source,
        });
        self.save(&registry)?;
        Ok(installed)
    }

    pub fn remove(&self, name: &str) -> Result<InstalledPlugin, PluginError> {
        let name = normalize_plugin_id(name)
            .ok_or_else(|| PluginError::InvalidManifest("invalid plugin name".into()))?;
        let _lock = RegistryLock::acquire(&self.home.join("plugins"))?;
        let mut registry = self.load()?;
        let installed = registry
            .plugins
            .remove(&name)
            .ok_or_else(|| PluginError::NotInstalled(name.clone()))?;
        let packages_root = fs::canonicalize(self.packages_root())
            .map_err(|error| io_error(self.packages_root(), error))?;
        let package_path = fs::canonicalize(&installed.package_path)
            .map_err(|error| io_error(&installed.package_path, error))?;
        if !package_path.starts_with(&packages_root) || package_path == packages_root {
            return Err(PluginError::UnsafePackage(format!(
                "registry package path is outside the managed store: {}",
                package_path.display()
            )));
        }
        registry.generation = registry.generation.saturating_add(1);
        registry.audit.push(PluginAuditEvent {
            generation: registry.generation,
            at_unix: now_unix(),
            action: PluginAuditAction::Removed,
            plugin: installed.name.clone(),
            version: installed.version.clone(),
            digest: installed.digest.clone(),
            trace_id: installed.trace_id.clone(),
            source: installed.source.clone(),
        });
        self.save(&registry)?;
        fs::remove_dir_all(&package_path).map_err(|error| io_error(&package_path, error))?;
        prune_empty_parents(&package_path, &packages_root)?;
        Ok(installed)
    }

    fn package_path(&self, inspection: &PackageInspection) -> PathBuf {
        self.packages_root()
            .join(&inspection.manifest.name)
            .join(&inspection.manifest.version)
            .join(&inspection.digest)
    }

    fn copy_verified(
        &self,
        inspection: &PackageInspection,
        destination: &Path,
    ) -> Result<(), PluginError> {
        let parent = destination.parent().ok_or_else(|| {
            PluginError::UnsafePackage("package destination has no parent".into())
        })?;
        fs::create_dir_all(parent).map_err(|error| io_error(parent, error))?;
        let staging = tempfile::Builder::new()
            .prefix(".plugin-stage-")
            .tempdir_in(parent)
            .map_err(|error| io_error(parent, error))?;
        for file in &inspection.files {
            let relative = path_from_portable(&file.path);
            let source = inspection.root.join(&relative);
            let source_meta =
                fs::symlink_metadata(&source).map_err(|error| io_error(&source, error))?;
            if !source_meta.is_file() || source_meta.file_type().is_symlink() {
                return Err(PluginError::UnsafePackage(format!(
                    "source changed during install: {}",
                    source.display()
                )));
            }
            let canonical_source =
                fs::canonicalize(&source).map_err(|error| io_error(&source, error))?;
            if !canonical_source.starts_with(&inspection.root) {
                return Err(PluginError::UnsafePackage(format!(
                    "source escaped package during install: {}",
                    source.display()
                )));
            }
            let target = staging.path().join(&relative);
            if let Some(directory) = target.parent() {
                fs::create_dir_all(directory).map_err(|error| io_error(directory, error))?;
            }
            fs::copy(&source, &target).map_err(|error| io_error(&target, error))?;
        }
        let staged_inspection = inspect_package(staging.path())?;
        if staged_inspection.digest != inspection.digest {
            return Err(PluginError::UnsafePackage(
                "package changed while it was being installed".into(),
            ));
        }
        let staged_path = staging.keep();
        fs::rename(&staged_path, destination).map_err(|error| io_error(destination, error))?;
        Ok(())
    }

    fn save(&self, registry: &PluginRegistry) -> Result<(), PluginError> {
        let path = self.registry_path();
        let parent = path
            .parent()
            .ok_or_else(|| PluginError::UnsafePackage("registry path has no parent".into()))?;
        fs::create_dir_all(parent).map_err(|error| io_error(parent, error))?;
        let bytes = serde_json::to_vec_pretty(registry).map_err(|source| PluginError::Json {
            path: path.clone(),
            source,
        })?;
        let mut staged =
            tempfile::NamedTempFile::new_in(parent).map_err(|error| io_error(parent, error))?;
        staged
            .write_all(&bytes)
            .map_err(|error| io_error(staged.path(), error))?;
        staged
            .as_file()
            .sync_all()
            .map_err(|error| io_error(staged.path(), error))?;
        staged
            .persist(&path)
            .map_err(|error| io_error(&path, error.error))?;
        Ok(())
    }
}

fn prune_empty_parents(path: &Path, stop: &Path) -> Result<(), PluginError> {
    let mut current = path.parent();
    while let Some(directory) = current {
        if directory == stop {
            break;
        }
        match fs::remove_dir(directory) {
            Ok(()) => current = directory.parent(),
            Err(error) if error.kind() == std::io::ErrorKind::DirectoryNotEmpty => break,
            Err(error) => return Err(io_error(directory, error)),
        }
    }
    Ok(())
}

fn now_unix() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

struct RegistryLock {
    path: PathBuf,
}

impl RegistryLock {
    fn acquire(root: &Path) -> Result<Self, PluginError> {
        fs::create_dir_all(root).map_err(|error| io_error(root, error))?;
        let path = root.join(".registry.lock");
        match create_lock(&path) {
            Ok(()) => Ok(Self { path }),
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {
                let stale = fs::metadata(&path)
                    .and_then(|metadata| metadata.modified())
                    .ok()
                    .and_then(|modified| SystemTime::now().duration_since(modified).ok())
                    .is_some_and(|age| age > Duration::from_secs(120));
                if !stale {
                    return Err(PluginError::RegistryBusy);
                }
                fs::remove_file(&path).map_err(|remove_error| io_error(&path, remove_error))?;
                create_lock(&path).map_err(|retry_error| io_error(&path, retry_error))?;
                Ok(Self { path })
            }
            Err(error) => Err(io_error(&path, error)),
        }
    }
}

impl Drop for RegistryLock {
    fn drop(&mut self) {
        let _ = fs::remove_file(&self.path);
    }
}

fn create_lock(path: &Path) -> std::io::Result<()> {
    let mut lock = OpenOptions::new().write(true).create_new(true).open(path)?;
    writeln!(lock, "{} {}", std::process::id(), now_unix())?;
    lock.sync_all()
}

#[cfg(test)]
mod tests {
    #![allow(clippy::expect_used, clippy::panic, clippy::unwrap_used)]

    use super::*;

    fn write(path: &Path, text: &str) {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).unwrap();
        }
        fs::write(path, text).unwrap();
    }

    fn package(root: &Path) {
        write(
            &root.join("vak-plugin.json"),
            r#"{
  "schema": 1,
  "name": "daily-brief",
  "version": "1.2.3",
  "description": "Prepare a daily brief.",
  "license": "MIT",
  "publisher": {"id":"vak-labs","name":"Vak Labs"},
  "components": {"skills":["skills"],"commands":["commands"]}
}"#,
        );
        write(
            &root.join("skills/daily-brief/SKILL.md"),
            "---\nname: daily-brief\ndescription: Prepare the brief.\n---\n\nDo it.\n",
        );
        write(
            &root.join("skills/daily-brief/scripts/collect.sh"),
            "#!/bin/sh\nprintf brief\n",
        );
        write(&root.join("commands/brief.md"), "Prepare a brief.\n");
    }

    #[test]
    fn inspects_native_package_and_digest_is_deterministic() {
        let temp = tempfile::tempdir().unwrap();
        package(temp.path());
        let first = inspect_package(temp.path()).unwrap();
        let second = inspect_package(temp.path()).unwrap();
        assert_eq!(first.digest, second.digest);
        assert_eq!(first.manifest.name, "daily-brief");
        assert_eq!(first.capabilities.skills, ["daily-brief"]);
        assert_eq!(first.capabilities.commands, ["commands/brief.md"]);
        assert_eq!(
            first.capabilities.scripts,
            ["skills/daily-brief/scripts/collect.sh"]
        );
    }

    #[test]
    fn changing_content_changes_digest() {
        let temp = tempfile::tempdir().unwrap();
        package(temp.path());
        let first = inspect_package(temp.path()).unwrap();
        write(&temp.path().join("commands/brief.md"), "Changed.\n");
        let second = inspect_package(temp.path()).unwrap();
        assert_ne!(first.digest, second.digest);
    }

    #[test]
    fn normalizes_codex_manifest_and_conventional_components() {
        let temp = tempfile::tempdir().unwrap();
        write(
            &temp.path().join(".codex-plugin/plugin.json"),
            r#"{"name":"codex-compatible","version":"2.0.0","description":"Compatible","license":"Apache-2.0","skills":"./skills"}"#,
        );
        write(
            &temp.path().join("skills/hello/SKILL.md"),
            "---\nname: hello\ndescription: Hello.\n---\nHello.\n",
        );
        write(&temp.path().join(".mcp.json"), "{}\n");
        let inspected = inspect_package(temp.path()).unwrap();
        assert_eq!(inspected.format, ManifestFormat::Codex);
        assert_eq!(inspected.capabilities.skills, ["hello"]);
        assert_eq!(inspected.capabilities.mcp_manifests, [".mcp.json"]);
    }

    #[cfg(unix)]
    #[test]
    fn rejects_symlinks_and_hard_links() {
        use std::os::unix::fs::symlink;

        let symlinked = tempfile::tempdir().unwrap();
        package(symlinked.path());
        symlink("commands/brief.md", symlinked.path().join("alias.md")).unwrap();
        assert!(matches!(
            inspect_package(symlinked.path()),
            Err(PluginError::UnsafePackage(_))
        ));

        let linked = tempfile::tempdir().unwrap();
        package(linked.path());
        fs::hard_link(
            linked.path().join("commands/brief.md"),
            linked.path().join("commands/brief-copy.md"),
        )
        .unwrap();
        assert!(matches!(
            inspect_package(linked.path()),
            Err(PluginError::UnsafePackage(_))
        ));
    }

    #[test]
    fn enforces_file_and_size_limits() {
        let temp = tempfile::tempdir().unwrap();
        package(temp.path());
        let error = inspect_package_with_limits(
            temp.path(),
            InspectLimits {
                max_files: 1,
                ..InspectLimits::default()
            },
        )
        .unwrap_err();
        assert!(matches!(error, PluginError::LimitExceeded(_)));
    }

    #[test]
    fn installs_disabled_verifies_copy_and_removes() {
        let source = tempfile::tempdir().unwrap();
        let home = tempfile::tempdir().unwrap();
        package(source.path());
        let store = PluginStore::new(home.path());
        let installed = store
            .install_local(source.path(), InstallOptions::default())
            .unwrap();
        assert!(!installed.enabled);
        assert!(installed.package_path.is_dir());
        assert_eq!(
            inspect_package(&installed.package_path).unwrap().digest,
            installed.digest
        );
        assert_eq!(store.list().unwrap().len(), 1);
        let idempotent = store
            .install_local(source.path(), InstallOptions::default())
            .unwrap();
        assert_eq!(installed.digest, idempotent.digest);
        let removed = store.remove("daily-brief").unwrap();
        assert_eq!(removed.digest, installed.digest);
        assert!(!installed.package_path.exists());
        assert!(store.list().unwrap().is_empty());
    }

    #[test]
    fn refuses_unlicensed_install_without_explicit_override() {
        let source = tempfile::tempdir().unwrap();
        let home = tempfile::tempdir().unwrap();
        write(
            &source.path().join("SKILL.md"),
            "---\nname: local-skill\ndescription: Local.\n---\nLocal.\n",
        );
        let store = PluginStore::new(home.path());
        assert!(matches!(
            store.install_local(source.path(), InstallOptions::default()),
            Err(PluginError::Unlicensed(_))
        ));
        let installed = store
            .install_local(
                source.path(),
                InstallOptions {
                    allow_unlicensed: true,
                    ..InstallOptions::default()
                },
            )
            .unwrap();
        assert_eq!(installed.format, ManifestFormat::AgentSkill);
    }

    #[test]
    fn refuses_replacement_without_remove() {
        let source = tempfile::tempdir().unwrap();
        let home = tempfile::tempdir().unwrap();
        package(source.path());
        let store = PluginStore::new(home.path());
        store
            .install_local(source.path(), InstallOptions::default())
            .unwrap();
        write(&source.path().join("commands/brief.md"), "New content.\n");
        assert!(matches!(
            store.install_local(source.path(), InstallOptions::default()),
            Err(PluginError::AlreadyInstalled(_))
        ));
    }
}
