//! A skill's program: a script with a declared argument schema that the
//! runtime runs as a tool (docs/design/93-self-evolving-capabilities.md §3).
//!
//! The declaration (`capability.json` beside `SKILL.md`) is the tool's
//! permission and the program's sandbox at once: the sandbox is built from
//! what the program declares, never from the session's mode, so a program
//! that declares no network has none even under FullAccess, and one that
//! declares hosts reaches only those, through an egress proxy of its own
//! (`crate::egress`). Pinned dependencies are installed once, by a run of
//! their own whose one way out is the package index the declaration names,
//! into a content-addressed environment in the cache home. The host side
//! ([`CapabilityTool`]) checks and dispatches; the work runs in a broker
//! worker ([`Runner`]).

use std::path::{Component, Path, PathBuf};
use std::process::Stdio;
use std::sync::Arc;

use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest, Sha256};
use tokio::io::AsyncWriteExt;

use crate::{Fault, Tool, ToolContext, ToolOutput};

/// The file beside `SKILL.md` that makes a skill runnable.
pub const DECLARATION: &str = "capability.json";

/// What a runnable skill folder holds, as the one statement of it a model
/// is shown: in `propose_skill`'s description and with every refusal of a
/// declaration, so a caller learns the whole shape at once.
pub const FORMAT: &str = "capability.json: {\"entry\": \"run.py\" (or .sh, .mjs), \"arguments\": a JSON Schema {\"type\": \"object\", \"properties\": {...}} whose every property has a \"type\" and a \"description\"} and, only when needed, \"hosts\": [exact host names it connects to], \"writes\": \"drafts\", \"requirements\": {\"file\": \"requirements.txt\" with every package pinned by hash, \"hosts\": [its package index hosts]}. The program reads its arguments as one JSON object on stdin and prints one JSON value on stdout. Each test is tests/<name>.json: {\"arguments\": {...}, \"expect\": {the keys the result must have}} or {\"arguments\": {...}, \"fails\": true}.";

/// The folder of a runnable skill's test cases, one JSON file each.
pub const TESTS_DIR: &str = "tests";

/// The argument the runtime fills with the call's time limit
/// (`Tool::time_limit_argument`); a program never receives it.
const TIME_LIMIT_ARGUMENT: &str = "timeout_ms";

/// What a program declares about itself.
#[derive(Debug, Clone, Deserialize)]
pub struct Declaration {
    /// The script, relative to the skill directory.
    pub entry: String,
    /// The JSON Schema of its arguments: an object whose every property
    /// has a type and a description.
    pub arguments: Value,
    /// Hosts it connects to, each a name (HTTPS and HTTP) or `name:port`;
    /// empty means no network at all.
    #[serde(default)]
    pub hosts: Vec<String>,
    #[serde(default)]
    pub writes: Writes,
    #[serde(default)]
    pub requirements: Option<Requirements>,
}

/// Pinned dependencies and where they come from (doc 93, R3).
#[derive(Debug, Clone, Deserialize)]
pub struct Requirements {
    /// A pip requirements file in the skill directory, every package
    /// pinned with its hash (`pip install --require-hashes`).
    pub file: String,
    /// The package index hosts the install may reach, and nothing else.
    pub hosts: Vec<String>,
}

/// One of a program's own tests (`tests/<name>.json`): the arguments a call
/// sends, and either the result it must answer with or that it must fail.
#[derive(Debug, Clone, Deserialize)]
pub struct TestCase {
    pub arguments: Value,
    /// What the result must contain: every key of an object, with its value
    /// contained in turn; every item of an array, in order; a number or
    /// any other value, equal.
    #[serde(default)]
    pub expect: Option<Value>,
    /// The call must fail rather than answer.
    #[serde(default)]
    pub fails: bool,
}

impl TestCase {
    /// Whether `output`, the call's answer, is what this case asks for.
    /// `drafts` says the answer wraps the program's result with the
    /// drafts it wrote.
    pub fn judge(&self, output: &ToolOutput, drafts: bool) -> Result<(), String> {
        if self.fails {
            return if output.is_error() {
                Ok(())
            } else {
                Err(format!(
                    "the program was to fail; it answered {}",
                    output.content
                ))
            };
        }
        if output.is_error() {
            return Err(format!("the program failed: {}", output.content));
        }
        let answer: Value = serde_json::from_str(&output.content)
            .map_err(|_| format!("the answer is not JSON: {}", output.content))?;
        let result = if drafts {
            answer.get("result").cloned().unwrap_or(Value::Null)
        } else {
            answer
        };
        let expect = self.expect.as_ref().unwrap_or(&Value::Null);
        contained(expect, &result, "$")
    }
}

/// `expect` within `actual`, naming where the first difference is.
fn contained(expect: &Value, actual: &Value, at: &str) -> Result<(), String> {
    match (expect, actual) {
        (Value::Object(expect), Value::Object(actual)) => {
            for (key, value) in expect {
                let here = format!("{at}.{key}");
                match actual.get(key) {
                    Some(found) => contained(value, found, &here)?,
                    None => return Err(format!("{here} is missing")),
                }
            }
            Ok(())
        }
        (Value::Array(expect), Value::Array(actual)) => {
            if expect.len() != actual.len() {
                return Err(format!(
                    "{at} has {} items, expected {}",
                    actual.len(),
                    expect.len()
                ));
            }
            for (index, (value, found)) in expect.iter().zip(actual).enumerate() {
                contained(value, found, &format!("{at}[{index}]"))?;
            }
            Ok(())
        }
        (Value::Number(expect), Value::Number(actual)) if expect.as_f64() == actual.as_f64() => {
            Ok(())
        }
        (expect, actual) if expect == actual => Ok(()),
        (expect, actual) => Err(format!("{at} is {actual}, expected {expect}")),
    }
}

/// The skill's test cases, by name, in order; a program has at least one.
fn load_tests(dir: &Path) -> Result<Vec<(String, TestCase)>, String> {
    let missing = || {
        format!(
            "a runnable skill needs tests: add {TESTS_DIR}/<name>.json with the \"arguments\" a call sends and the result it must \"expect\" (or \"fails\": true)"
        )
    };
    let entries = match std::fs::read_dir(dir.join(TESTS_DIR)) {
        Ok(entries) => entries,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Err(missing()),
        Err(error) => return Err(format!("{TESTS_DIR} cannot be read: {error}")),
    };
    let mut cases = Vec::new();
    for entry in entries.flatten() {
        let path = entry.path();
        if path.extension().and_then(|ext| ext.to_str()) != Some("json") {
            continue;
        }
        let name = path
            .file_stem()
            .and_then(|stem| stem.to_str())
            .unwrap_or_default()
            .to_string();
        let text = std::fs::read_to_string(&path)
            .map_err(|error| format!("test '{name}' cannot be read: {error}"))?;
        let case: TestCase = serde_json::from_str(&text)
            .map_err(|error| format!("test '{name}' is not a test case: {error}"))?;
        if !case.arguments.is_object() {
            return Err(format!(
                "test '{name}' must give its arguments as an object"
            ));
        }
        if case.fails == case.expect.is_some() {
            return Err(format!(
                "test '{name}' must have either \"expect\" or \"fails\": true"
            ));
        }
        cases.push((name, case));
    }
    if cases.is_empty() {
        return Err(missing());
    }
    cases.sort_by(|a, b| a.0.cmp(&b.0));
    Ok(cases)
}

/// Where a program may write besides its own temporary directory.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Writes {
    #[default]
    None,
    /// Its execution's drafts, which reach the workspace only through
    /// Review.
    Drafts,
}

/// What runs the script, chosen by its extension.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Interpreter {
    Python,
    Shell,
    Node,
}

impl Interpreter {
    fn of(entry: &Path) -> Option<Self> {
        match entry.extension()?.to_str()? {
            "py" => Some(Self::Python),
            "sh" => Some(Self::Shell),
            "mjs" | "js" => Some(Self::Node),
            _ => None,
        }
    }

    fn program(self) -> &'static str {
        match self {
            Self::Python => "python3",
            Self::Shell => "sh",
            Self::Node => "node",
        }
    }
}

/// A program's installed dependencies.
#[derive(Debug, Clone)]
pub struct Dependencies {
    pub file: PathBuf,
    pub hosts: Vec<String>,
    /// The environment they are installed in, named by the digest of the
    /// requirements and the interpreter, so one install serves every
    /// program that pins the same packages.
    pub env: PathBuf,
}

impl Dependencies {
    pub fn installed(&self) -> bool {
        self.env.is_dir()
    }
}

/// A loaded, checked program.
#[derive(Debug, Clone)]
pub struct Program {
    pub dir: PathBuf,
    pub entry: PathBuf,
    pub interpreter: Interpreter,
    pub arguments: Value,
    pub hosts: Vec<String>,
    pub writes: Writes,
    pub requirements: Option<Dependencies>,
    pub tests: Vec<(String, TestCase)>,
    /// Over every file in the skill directory, so a change to the script,
    /// its tests or its declaration is a different program.
    pub digest: String,
}

impl Program {
    /// The skill directory's program, `None` when it declares none.
    pub fn load(dir: &Path) -> Result<Option<Self>, String> {
        let path = dir.join(DECLARATION);
        let text = match std::fs::read_to_string(&path) {
            Ok(text) => text,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            Err(error) => return Err(format!("{DECLARATION} cannot be read: {error}")),
        };
        let declaration: Declaration = serde_json::from_str(&text).map_err(|error| {
            format!("{DECLARATION} is not a valid declaration ({error}). Expected {FORMAT}")
        })?;
        let entry = inside(dir, &declaration.entry)?;
        let interpreter = Interpreter::of(&entry).ok_or_else(|| {
            format!(
                "entry '{}' must end in .py, .sh, .mjs or .js",
                declaration.entry
            )
        })?;
        check_arguments(&declaration.arguments)?;
        check_hosts("hosts", &declaration.hosts)?;
        let tests = load_tests(dir)?;
        let requirements = match &declaration.requirements {
            None => None,
            Some(requirements) => {
                if interpreter != Interpreter::Python {
                    return Err(format!(
                        "requirements are installed for Python programs; a {} program cannot declare them",
                        interpreter.program()
                    ));
                }
                if requirements.hosts.is_empty() {
                    return Err(
                        "requirements must name the package index hosts they are fetched from"
                            .into(),
                    );
                }
                check_hosts("requirements.hosts", &requirements.hosts)?;
                let file = inside(dir, &requirements.file)?;
                let bytes = std::fs::read(&file)
                    .map_err(|error| format!("requirements cannot be read: {error}"))?;
                Some(Dependencies {
                    env: environment_for(&bytes, interpreter),
                    file,
                    hosts: requirements.hosts.clone(),
                })
            }
        };
        Ok(Some(Self {
            dir: dir.to_path_buf(),
            entry,
            interpreter,
            arguments: declaration.arguments,
            hosts: declaration.hosts,
            writes: declaration.writes,
            requirements,
            tests,
            digest: digest(dir)?,
        }))
    }

    /// Its arguments as the model is shown them: the declared schema plus
    /// the time limit the runtime fills.
    pub fn schema(&self) -> Value {
        let mut schema = self.arguments.clone();
        if let Some(properties) = schema.get_mut("properties").and_then(Value::as_object_mut) {
            properties.insert(
                TIME_LIMIT_ARGUMENT.into(),
                serde_json::json!({
                    "type": "integer",
                    "minimum": 1000,
                    "description": "Time limit in milliseconds (default: the configured command limit)"
                }),
            );
        }
        schema
    }

    /// Whether a call can leave the machine: it declares hosts, or its
    /// dependencies are not installed yet and this call installs them.
    pub fn leaves_machine(&self) -> bool {
        !self.hosts.is_empty()
            || self
                .requirements
                .as_ref()
                .is_some_and(|requirements| !requirements.installed())
    }

    /// What a call reaches, in the owner's words, from its declaration
    /// alone: the one wording for an approval, a proposal waiting for
    /// review and its inbox entry (docs/design/93-self-evolving-capabilities.md §9.2).
    pub fn reaches(&self) -> Vec<String> {
        let mut lines = vec!["Reads the files in this project".to_string()];
        if self.writes == Writes::Drafts {
            lines.push("Writes only drafts, which reach the project through Review".into());
        }
        if !self.hosts.is_empty() {
            lines.push(format!("Connects to {}", self.hosts.join(", ")));
        }
        if let Some(requirements) = &self.requirements
            && !requirements.installed()
        {
            lines.push(format!(
                "Installs its packages from {}, once",
                requirements.hosts.join(", ")
            ));
        }
        if !self.leaves_machine() {
            lines.push("Reaches nothing outside this computer".into());
        }
        lines
    }

    /// What it reaches, from its declaration alone.
    pub fn permission(&self) -> vak_permission::Declared {
        vak_permission::Declared::reach(if self.leaves_machine() {
            vak_permission::Reach::Network
        } else {
            match self.writes {
                Writes::None => vak_permission::Reach::ReadPath,
                Writes::Drafts => vak_permission::Reach::WritePath,
            }
        })
    }

    /// It reads the workspace; what a declared host answers is someone
    /// else's words.
    pub fn provenance(&self) -> vak_control::Provenance {
        if self.hosts.is_empty() {
            vak_control::Provenance::PRIVATE
        } else {
            vak_control::Provenance::BOTH
        }
    }
}

/// A skill folder as one value: every file by its relative path, text only,
/// in path order, so the same folder is always the same bytes. A Library
/// version of a skill holds this (docs/design/93 §6).
pub fn bundle(dir: &Path) -> Result<Vec<u8>, String> {
    let mut files = std::collections::BTreeMap::new();
    let mut pending = vec![dir.to_path_buf()];
    while let Some(next) = pending.pop() {
        let entries =
            std::fs::read_dir(&next).map_err(|error| format!("cannot list the skill: {error}"))?;
        for entry in entries {
            let entry = entry.map_err(|error| format!("cannot list the skill: {error}"))?;
            let kind = entry
                .file_type()
                .map_err(|error| format!("cannot list the skill: {error}"))?;
            if kind.is_symlink() {
                return Err("a skill may not contain symlinks".into());
            }
            if kind.is_dir() {
                pending.push(entry.path());
                continue;
            }
            let path = entry.path();
            let relative = path
                .strip_prefix(dir)
                .map_err(|_| "a skill file is outside its folder".to_string())?
                .to_string_lossy()
                .replace('\\', "/");
            let bytes =
                std::fs::read(&path).map_err(|error| format!("cannot read {relative}: {error}"))?;
            let text = String::from_utf8(bytes)
                .map_err(|_| format!("{relative} is not text; a skill holds text files only"))?;
            files.insert(relative, text);
        }
    }
    serde_json::to_vec(&serde_json::json!({ "files": files }))
        .map_err(|error| format!("the skill did not encode: {error}"))
}

/// Writes a [`bundle`] out as a folder at `into`, which must not exist yet.
/// A path that is absolute or climbs out is refused.
pub fn unbundle(bytes: &[u8], into: &Path) -> Result<(), String> {
    let value: Value =
        serde_json::from_slice(bytes).map_err(|_| "not a skill bundle".to_string())?;
    let files = value
        .get("files")
        .and_then(Value::as_object)
        .ok_or_else(|| "not a skill bundle".to_string())?;
    if into.exists() {
        return Err(format!("{} already exists", into.display()));
    }
    for (relative, text) in files {
        let candidate = Path::new(relative);
        if relative.is_empty()
            || candidate
                .components()
                .any(|part| !matches!(part, Component::Normal(_)))
        {
            return Err(format!("'{relative}' is not a path inside a skill"));
        }
        let text = text
            .as_str()
            .ok_or_else(|| format!("'{relative}' is not text"))?;
        let path = into.join(candidate);
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)
                .map_err(|error| format!("cannot write the skill: {error}"))?;
        }
        std::fs::write(&path, text).map_err(|error| format!("cannot write the skill: {error}"))?;
    }
    Ok(())
}

/// The environment for these requirements under this interpreter.
fn environment_for(requirements: &[u8], interpreter: Interpreter) -> PathBuf {
    let mut hasher = Sha256::new();
    if let Some(path) = resolve(interpreter).and_then(|path| path.canonicalize().ok()) {
        hasher.update(path.to_string_lossy().as_bytes());
    }
    hasher.update([0]);
    hasher.update(requirements);
    vak_config::paths::cache_home()
        .join("capability-envs")
        .join(format!("{:x}", hasher.finalize()))
}

/// Exact host names, each optionally with a port; no wildcard, scheme or
/// path, because the proxy matches names exactly.
fn check_hosts(field: &str, hosts: &[String]) -> Result<(), String> {
    for host in hosts {
        let (name, port) = match host.rsplit_once(':') {
            Some((name, port)) => (name, Some(port)),
            None => (host.as_str(), None),
        };
        let name_ok = !name.is_empty()
            && name
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || c == '.' || c == '-');
        let port_ok = port.is_none_or(|port| port.parse::<u16>().is_ok_and(|port| port > 0));
        if !name_ok || !port_ok {
            return Err(format!(
                "{field} entry '{host}' must be an exact host name, optionally with :port"
            ));
        }
    }
    Ok(())
}

/// `relative` resolved inside `dir`, refusing an absolute path, `..`, or a
/// symlink out.
fn inside(dir: &Path, relative: &str) -> Result<PathBuf, String> {
    let candidate = Path::new(relative);
    if relative.is_empty()
        || candidate
            .components()
            .any(|part| !matches!(part, Component::Normal(_)))
    {
        return Err(format!(
            "'{relative}' must be a path inside the skill directory"
        ));
    }
    let root = dir
        .canonicalize()
        .map_err(|error| format!("skill directory is unavailable: {error}"))?;
    let resolved = root
        .join(candidate)
        .canonicalize()
        .map_err(|_| format!("'{relative}' does not exist"))?;
    if !resolved.starts_with(&root) || !resolved.is_file() {
        return Err(format!(
            "'{relative}' must be a file inside the skill directory"
        ));
    }
    Ok(resolved)
}

fn check_arguments(schema: &Value) -> Result<(), String> {
    if schema.get("type").and_then(Value::as_str) != Some("object") {
        return Err("arguments must be a JSON Schema of type object".into());
    }
    let Some(properties) = schema.get("properties").and_then(Value::as_object) else {
        return Err("arguments must list its properties".into());
    };
    for (name, property) in properties {
        if name == TIME_LIMIT_ARGUMENT {
            return Err(format!(
                "argument '{TIME_LIMIT_ARGUMENT}' is the runtime's: the call's time limit"
            ));
        }
        if property.get("type").is_none() {
            return Err(format!("argument '{name}' must have a type"));
        }
        if property
            .get("description")
            .and_then(Value::as_str)
            .is_none_or(|text| text.trim().is_empty())
        {
            return Err(format!("argument '{name}' must have a description"));
        }
    }
    Ok(())
}

/// Over every file under `dir`, by relative path and bytes. A symlink is
/// refused: the program would be something other than what was read.
fn digest(dir: &Path) -> Result<String, String> {
    let mut files = Vec::new();
    let mut pending = vec![dir.to_path_buf()];
    while let Some(next) = pending.pop() {
        let entries =
            std::fs::read_dir(&next).map_err(|error| format!("cannot list the skill: {error}"))?;
        for entry in entries {
            let entry = entry.map_err(|error| format!("cannot list the skill: {error}"))?;
            let kind = entry
                .file_type()
                .map_err(|error| format!("cannot list the skill: {error}"))?;
            if kind.is_symlink() {
                return Err("a runnable skill may not contain symlinks".into());
            }
            if kind.is_dir() {
                pending.push(entry.path());
            } else {
                files.push(entry.path());
            }
        }
    }
    files.sort();
    let mut hasher = Sha256::new();
    for file in files {
        let relative = file.strip_prefix(dir).unwrap_or(&file);
        hasher.update(relative.to_string_lossy().as_bytes());
        hasher.update([0]);
        let bytes =
            std::fs::read(&file).map_err(|error| format!("cannot read the skill: {error}"))?;
        hasher.update((bytes.len() as u64).to_le_bytes());
        hasher.update(&bytes);
    }
    Ok(format!("{:x}", hasher.finalize()))
}

/// The interpreter's absolute path on the PATH a scrubbed command gets.
fn resolve(interpreter: Interpreter) -> Option<PathBuf> {
    let program = interpreter.program();
    std::env::split_paths(&crate::bash::execution_path())
        .map(|directory| directory.join(program))
        .find(|path| path.is_file())
}

/// The sandbox a program or an install runs in: reads the system, the
/// workspace and `reads`; writes only `writes`; reaches the network only
/// through `proxy`'s loopback port when one is given. `None` where no OS
/// backend can confine it, and the call is then refused.
fn sandbox_for(
    workspace: &Path,
    reads: &[PathBuf],
    writes: &[PathBuf],
    proxy: Option<u16>,
) -> Option<Arc<dyn crate::sandbox::Sandbox>> {
    let real = |paths: &[PathBuf]| -> Vec<PathBuf> {
        paths
            .iter()
            .map(|path| path.canonicalize().unwrap_or_else(|_| path.clone()))
            .collect()
    };
    let reads = real(reads);
    let writes = real(writes);
    #[cfg(target_os = "macos")]
    let sandbox: Option<Arc<dyn crate::sandbox::Sandbox>> = {
        let mut sandbox =
            crate::sandbox::Seatbelt::task_copy(crate::sandbox::SandboxMode::ReadOnly, workspace);
        sandbox.mode = crate::sandbox::SandboxMode::WorkspaceWrite;
        sandbox.read_paths.extend(reads);
        sandbox.read_paths.extend(writes.iter().cloned());
        sandbox.write_paths = writes;
        Some(Arc::new(sandbox))
    };
    #[cfg(target_os = "linux")]
    let sandbox: Option<Arc<dyn crate::sandbox::Sandbox>> = {
        let mut sandbox =
            crate::landlock::Landlock::task_copy(crate::sandbox::SandboxMode::ReadOnly, workspace);
        sandbox.mode = crate::sandbox::SandboxMode::WorkspaceWrite;
        sandbox.read_paths.extend(reads);
        sandbox.read_paths.extend(writes.iter().cloned());
        sandbox.write_paths = writes;
        Some(Arc::new(sandbox))
    };
    #[cfg(not(any(target_os = "macos", target_os = "linux")))]
    let sandbox: Option<Arc<dyn crate::sandbox::Sandbox>> = {
        let _ = (workspace, reads, writes);
        None
    };
    match proxy {
        Some(port) => sandbox?.proxied_variant(port),
        None => sandbox,
    }
}

/// What the worker does for a call.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Work {
    /// Run the program: arguments as JSON on stdin, one JSON value out.
    Program {
        interpreter: PathBuf,
        entry: PathBuf,
        args: Value,
    },
    /// Run each command in turn: an install's steps.
    Install { steps: Vec<Vec<String>> },
}

/// A runnable skill as a tool on the host: what the model sees and what
/// the permission engine reads. Calls go to a worker under the program's
/// own sandbox.
pub struct CapabilityTool {
    name: String,
    description: String,
    program: Program,
    worker_exe: PathBuf,
    serves: Vec<String>,
}

impl CapabilityTool {
    pub fn program(&self) -> &Program {
        &self.program
    }

    pub fn new(name: String, description: String, program: Program, worker_exe: PathBuf) -> Self {
        Self {
            name,
            description,
            program,
            worker_exe,
            serves: Vec::new(),
        }
    }

    /// The kinds of work its skill says it serves (`serves:` in SKILL.md),
    /// so the tool surface can defer it and `find_tools` rank it; none
    /// declared, it is loaded every turn.
    pub fn with_serves(mut self, serves: Vec<String>) -> Self {
        self.serves = serves;
        self
    }

    /// Install the dependencies by a run of their own: a fresh environment
    /// in a staging folder, the package index its one way out, then moved
    /// into place by the host. Another call that installed the same
    /// environment first wins, and this one's staging is dropped.
    async fn install(
        &self,
        requirements: &Dependencies,
        interpreter: &Path,
        reads: &[PathBuf],
        scratch: &Scratch,
        ctx: &ToolContext,
    ) -> Result<(), Box<ToolOutput>> {
        let failed = |content: String| Box::new(ToolOutput::error(Fault::Tool, content));
        let Some(parent) = requirements.env.parent() else {
            return Err(failed("the dependency environment has no folder".into()));
        };
        let staging = parent.join(format!(
            "{}.{}.staging",
            requirements
                .env
                .file_name()
                .and_then(|name| name.to_str())
                .unwrap_or("env"),
            scratch.execution
        ));
        let _ = std::fs::remove_dir_all(&staging);
        if let Err(error) = std::fs::create_dir_all(&staging) {
            return Err(failed(format!(
                "cannot prepare the dependency environment: {error}"
            )));
        }
        let egress = crate::egress::Egress::start(requirements.hosts.clone())
            .await
            .map_err(|error| failed(format!("the package index proxy did not start: {error}")))?;
        let writes = [staging.clone(), scratch.tmp.clone(), scratch.cache.clone()];
        let Some(sandbox) = sandbox_for(&ctx.cwd, reads, &writes, Some(egress.port())) else {
            let _ = std::fs::remove_dir_all(&staging);
            return Err(Box::new(ToolOutput::error(
                Fault::Refused,
                "no sandbox on this system can confine an install to its package index",
            )));
        };
        let python = staging.join("bin").join("python3");
        let steps = vec![
            vec![
                interpreter.display().to_string(),
                "-m".into(),
                "venv".into(),
                staging.display().to_string(),
            ],
            vec![
                python.display().to_string(),
                "-m".into(),
                "pip".into(),
                "install".into(),
                "--require-hashes".into(),
                "--no-input".into(),
                "--disable-pip-version-check".into(),
                "-r".into(),
                requirements.file.display().to_string(),
            ],
        ];
        let output = crate::broker::run_capability(
            &self.worker_exe,
            sandbox.as_ref(),
            crate::broker::CapabilityRun {
                name: self.name.clone(),
                tmp: scratch.tmp.clone(),
                cache: scratch.cache.clone(),
                drafts: None,
                timeout_ms: scratch.timeout_ms,
                env: egress.environment(),
                work: Work::Install { steps },
            },
            ctx,
        )
        .await;
        drop(egress);
        if output.is_error() {
            let _ = std::fs::remove_dir_all(&staging);
            return Err(Box::new(ToolOutput::error(
                output.fault.unwrap_or(Fault::Tool),
                format!(
                    "installing the dependencies of '{}' failed: {}",
                    self.name, output.content
                ),
            )));
        }
        if std::fs::rename(&staging, &requirements.env).is_err() {
            let _ = std::fs::remove_dir_all(&staging);
            if !requirements.installed() {
                return Err(failed(
                    "the installed dependencies could not be moved into place".into(),
                ));
            }
        }
        Ok(())
    }
}

/// A call's own folders, and its time limit.
struct Scratch {
    execution: String,
    tmp: PathBuf,
    cache: PathBuf,
    drafts: Option<PathBuf>,
    timeout_ms: u64,
}

#[async_trait]
impl Tool for CapabilityTool {
    fn name(&self) -> &str {
        &self.name
    }

    fn description(&self) -> &str {
        &self.description
    }

    fn schema(&self) -> Value {
        self.program.schema()
    }

    fn permission(&self) -> vak_permission::Declared {
        self.program.permission()
    }

    fn provenance(&self) -> vak_control::Provenance {
        self.program.provenance()
    }

    fn leaves_machine(&self) -> bool {
        self.program.leaves_machine()
    }

    fn reaches(&self) -> Vec<String> {
        self.program.reaches()
    }

    fn serves(&self) -> Vec<&str> {
        self.serves.iter().map(String::as_str).collect()
    }

    fn time_limit_argument(&self) -> Option<&'static str> {
        Some(TIME_LIMIT_ARGUMENT)
    }

    async fn execute(&self, args: &Value, ctx: &ToolContext) -> ToolOutput {
        match Program::load(&self.program.dir) {
            Ok(Some(now)) if now.digest == self.program.digest => {}
            _ => {
                return ToolOutput::error(
                    Fault::Tool,
                    format!(
                        "'{}' changed during this turn; its new version is offered from the next turn",
                        self.name
                    ),
                );
            }
        }
        let Some(interpreter) = resolve(self.program.interpreter) else {
            return ToolOutput::error(
                Fault::Tool,
                format!(
                    "'{}' needs {}, which is not installed on this computer",
                    self.name,
                    self.program.interpreter.program()
                ),
            );
        };
        let mut program_args = args.clone();
        let timeout_ms = program_args
            .as_object_mut()
            .and_then(|object| object.remove(TIME_LIMIT_ARGUMENT))
            .and_then(|value| value.as_u64())
            .unwrap_or(vak_control::TimeLimits::shipped().command_secs * 1000)
            .max(1000);
        let execution = ctx
            .sandbox_sink
            .as_ref()
            .map(|sink| sink.execution_id().to_string())
            .unwrap_or_else(|| "capability".into());
        let own = crate::drafts::own_root(ctx);
        let scratch = Scratch {
            tmp: own.join("tmp").join(&execution),
            cache: own.join("cache"),
            drafts: (self.program.writes == Writes::Drafts)
                .then(|| crate::drafts::dir(ctx, &execution)),
            execution,
            timeout_ms,
        };
        for dir in [
            Some(&scratch.tmp),
            Some(&scratch.cache),
            scratch.drafts.as_ref(),
        ]
        .into_iter()
        .flatten()
        {
            if let Err(error) = std::fs::create_dir_all(dir) {
                return ToolOutput::error(
                    Fault::Tool,
                    format!("cannot prepare the program's directories: {error}"),
                );
            }
        }
        let mut reads = vec![self.program.dir.clone()];
        reads.extend(self.worker_exe.parent().map(Path::to_path_buf));
        reads.extend(installation_of(&interpreter));
        let mut run_with = interpreter.clone();
        if let Some(requirements) = &self.program.requirements {
            if !requirements.installed()
                && let Err(failed) = self
                    .install(requirements, &interpreter, &reads, &scratch, ctx)
                    .await
            {
                return *failed;
            }
            reads.push(requirements.env.clone());
            run_with = requirements.env.join("bin").join("python3");
        }
        let egress = if self.program.hosts.is_empty() {
            None
        } else {
            match crate::egress::Egress::start(self.program.hosts.clone()).await {
                Ok(egress) => Some(egress),
                Err(error) => {
                    return ToolOutput::error(
                        Fault::Tool,
                        format!("the program's network proxy did not start: {error}"),
                    );
                }
            }
        };
        let mut writes = vec![scratch.tmp.clone(), scratch.cache.clone()];
        writes.extend(scratch.drafts.clone());
        let Some(sandbox) = sandbox_for(
            &ctx.cwd,
            &reads,
            &writes,
            egress.as_ref().map(crate::egress::Egress::port),
        ) else {
            return ToolOutput::error(
                Fault::Refused,
                "no sandbox on this system can confine a skill's program, so it is not run",
            );
        };
        let output = crate::broker::run_capability(
            &self.worker_exe,
            sandbox.as_ref(),
            crate::broker::CapabilityRun {
                name: self.name.clone(),
                tmp: scratch.tmp,
                cache: scratch.cache,
                drafts: scratch.drafts,
                timeout_ms,
                env: egress
                    .as_ref()
                    .map(crate::egress::Egress::environment)
                    .unwrap_or_default(),
                work: Work::Program {
                    interpreter: run_with,
                    entry: self.program.entry.clone(),
                    args: program_args,
                },
            },
            ctx,
        )
        .await;
        drop(egress);
        output
    }
}

/// The installation an interpreter belongs to, read through its symlinks
/// (`…/node/v22/bin/node` → `…/node/v22`), so a version manager's
/// interpreter can load its own libraries.
fn installation_of(interpreter: &Path) -> Option<PathBuf> {
    let real = interpreter.canonicalize().ok()?;
    let bin = real.parent()?;
    Some(bin.parent().unwrap_or(bin).to_path_buf())
}

/// The work run inside the worker.
pub struct Runner {
    pub run: crate::broker::CapabilityRun,
}

/// How a process ended, and what it printed.
struct Finished {
    ended: Ended,
    out: String,
    err: String,
    duration_ms: u64,
}

enum Ended {
    Exited(std::io::Result<std::process::ExitStatus>),
    Limit,
    Cancelled,
}

impl Runner {
    /// Start `program` with `args`, feed it `stdin`, stream its output to
    /// the Workbench, and stop it at `deadline` or when the call is
    /// cancelled.
    async fn spawn(
        &self,
        program: &Path,
        args: &[String],
        stdin: &[u8],
        deadline: tokio::time::Instant,
        ctx: &ToolContext,
    ) -> Result<Finished, String> {
        let run = &self.run;
        let mut command = tokio::process::Command::new(program);
        command
            .args(args)
            .current_dir(&ctx.cwd)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        crate::bash::scrub_environment(&mut command);
        command
            .env("TMPDIR", &run.tmp)
            .env("XDG_CACHE_HOME", &run.cache)
            .env("PIP_CACHE_DIR", run.cache.join("pip"))
            .env("PYTHONDONTWRITEBYTECODE", "1")
            .env("npm_config_cache", run.cache.join("npm"));
        if let Some(drafts) = &run.drafts {
            command.env("VAK_DRAFTS", drafts);
        }
        for (key, value) in &run.env {
            command.env(key, value);
        }
        crate::bash::isolate_process_group(&mut command);
        let started = std::time::Instant::now();
        let mut child = command
            .spawn()
            .map_err(|error| format!("{} did not start: {error}", program.display()))?;
        let pid = child.id();
        if let Some(mut input) = child.stdin.take()
            && input.write_all(stdin).await.is_err()
        {
            crate::bash::kill_process_group(&pid);
            let _ = child.wait().await;
            return Err("the program did not read its arguments".into());
        }
        let mut stdout = child.stdout.take();
        let mut stderr = child.stderr.take();
        let sink_out = ctx.sandbox_sink.clone();
        let sink_err = ctx.sandbox_sink.clone();
        let out = tokio::spawn(async move {
            match stdout.as_mut() {
                Some(reader) => crate::bash::read_capped_streaming(reader, sink_out, false).await,
                None => String::new(),
            }
        });
        let err = tokio::spawn(async move {
            match stderr.as_mut() {
                Some(reader) => crate::bash::read_capped_streaming(reader, sink_err, true).await,
                None => String::new(),
            }
        });
        let ended = tokio::select! {
            status = child.wait() => Ended::Exited(status),
            _ = tokio::time::sleep_until(deadline) => Ended::Limit,
            _ = ctx.cancel.cancelled() => Ended::Cancelled,
        };
        crate::bash::kill_process_group(&pid);
        if !matches!(ended, Ended::Exited(_)) {
            let _ = child.wait().await;
        }
        Ok(Finished {
            ended,
            out: out.await.unwrap_or_default(),
            err: err.await.unwrap_or_default(),
            duration_ms: started.elapsed().as_millis() as u64,
        })
    }

    /// A process that did not exit cleanly, as the call's error; `None`
    /// when it exited 0.
    fn failure(&self, finished: &Finished, what: &str) -> Option<ToolOutput> {
        let failed = match &finished.ended {
            Ended::Limit => ToolOutput::error(
                Fault::Limit,
                format!(
                    "{what} stopped at its time limit of {}ms{}",
                    self.run.timeout_ms,
                    diagnostics(&finished.err)
                ),
            )
            .ran(None, finished.duration_ms),
            Ended::Cancelled => ToolOutput::error(Fault::Cancelled, format!("{what} was stopped"))
                .ran(None, finished.duration_ms),
            Ended::Exited(Err(error)) => {
                ToolOutput::error(Fault::Tool, format!("{what} was lost: {error}"))
            }
            Ended::Exited(Ok(status)) if status.success() => return None,
            Ended::Exited(Ok(status)) => ToolOutput::error(
                Fault::Tool,
                format!(
                    "{what} exited with {}{}{}",
                    status
                        .code()
                        .map_or_else(|| "a signal".to_string(), |code| code.to_string()),
                    diagnostics(&finished.err),
                    if finished.out.trim().is_empty() {
                        String::new()
                    } else {
                        format!("\n\n[output]\n{}", finished.out.trim())
                    }
                ),
            )
            .ran(status.code(), finished.duration_ms),
        };
        Some(failed)
    }
}

#[async_trait]
impl Tool for Runner {
    fn name(&self) -> &str {
        &self.run.name
    }

    fn description(&self) -> &str {
        "a skill's program"
    }

    fn schema(&self) -> Value {
        serde_json::json!({"type": "object"})
    }

    async fn execute(&self, _args: &Value, ctx: &ToolContext) -> ToolOutput {
        let run = &self.run;
        let deadline =
            tokio::time::Instant::now() + std::time::Duration::from_millis(run.timeout_ms);
        let started = std::time::Instant::now();
        match &run.work {
            Work::Install { steps } => {
                if let Some(sink) = &ctx.sandbox_sink {
                    sink.emit_execution_started(
                        &run.name,
                        "installing dependencies",
                        "install",
                        &ctx.cwd.display().to_string(),
                    );
                }
                for step in steps {
                    let Some((program, args)) = step.split_first() else {
                        continue;
                    };
                    let finished = match self
                        .spawn(Path::new(program), args, &[], deadline, ctx)
                        .await
                    {
                        Ok(finished) => finished,
                        Err(error) => return ToolOutput::error(Fault::Tool, error),
                    };
                    if let Some(failed) = self.failure(&finished, "the install") {
                        if let Some(sink) = &ctx.sandbox_sink {
                            sink.emit_finished(
                                -1,
                                started.elapsed().as_millis() as u64,
                                Vec::new(),
                            );
                        }
                        return failed;
                    }
                }
                let duration_ms = started.elapsed().as_millis() as u64;
                if let Some(sink) = &ctx.sandbox_sink {
                    sink.emit_finished(0, duration_ms, Vec::new());
                }
                ToolOutput::ok("dependencies installed").ran(Some(0), duration_ms)
            }
            Work::Program {
                interpreter,
                entry,
                args,
            } => {
                if let Some(sink) = &ctx.sandbox_sink {
                    sink.emit_execution_started(
                        &run.name,
                        &entry.display().to_string(),
                        interpreter
                            .file_name()
                            .and_then(|name| name.to_str())
                            .unwrap_or("program"),
                        &ctx.cwd.display().to_string(),
                    );
                }
                let input = serde_json::to_vec(args).unwrap_or_default();
                let finished = match self
                    .spawn(
                        interpreter,
                        &[entry.display().to_string()],
                        &input,
                        deadline,
                        ctx,
                    )
                    .await
                {
                    Ok(finished) => finished,
                    Err(error) => return ToolOutput::error(Fault::Tool, error),
                };
                let drafts = run
                    .drafts
                    .as_deref()
                    .map(|dir| written(dir, ctx))
                    .unwrap_or_default();
                if let Some(sink) = &ctx.sandbox_sink {
                    let code = match &finished.ended {
                        Ended::Exited(Ok(status)) => status.code().unwrap_or(-1),
                        _ => -1,
                    };
                    sink.emit_finished(code, finished.duration_ms, drafts.clone());
                }
                if let Some(failed) = self.failure(&finished, "the program") {
                    return failed;
                }
                let code = match &finished.ended {
                    Ended::Exited(Ok(status)) => status.code(),
                    _ => None,
                };
                let result: Value = match serde_json::from_str(finished.out.trim()) {
                    Ok(result) => result,
                    Err(_) => {
                        return ToolOutput::error(
                            Fault::Tool,
                            format!(
                                "the program must print one JSON value on stdout; it printed:\n{}{}",
                                finished.out.trim(),
                                diagnostics(&finished.err)
                            ),
                        )
                        .ran(code, finished.duration_ms);
                    }
                };
                let content = match &run.drafts {
                    Some(_) => serde_json::json!({"result": result, "drafts": drafts}),
                    None => result,
                };
                ToolOutput::ok(content.to_string()).ran(code, finished.duration_ms)
            }
        }
    }
}

fn diagnostics(stderr: &str) -> String {
    let stderr = stderr.trim();
    if stderr.is_empty() {
        String::new()
    } else {
        format!("\n\n[stderr]\n{stderr}")
    }
}

/// The draft names of the files the program wrote, sorted.
fn written(dir: &Path, ctx: &ToolContext) -> Vec<String> {
    let execution = dir
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or_default()
        .to_string();
    let mut names = Vec::new();
    let mut pending = vec![dir.to_path_buf()];
    while let Some(next) = pending.pop() {
        let Ok(entries) = std::fs::read_dir(&next) else {
            continue;
        };
        for entry in entries.flatten() {
            let Ok(kind) = entry.file_type() else {
                continue;
            };
            if kind.is_dir() {
                pending.push(entry.path());
            } else if kind.is_file()
                && let Ok(relative) = entry.path().strip_prefix(dir)
            {
                names.push(crate::drafts::name(ctx, &execution, relative));
            }
        }
    }
    names.sort();
    names
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]

    use super::*;
    use serde_json::json;

    fn skill(declaration: Value) -> tempfile::TempDir {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("run.py"), "print('{}')\n").unwrap();
        std::fs::create_dir(dir.path().join(TESTS_DIR)).unwrap();
        std::fs::write(
            dir.path().join(TESTS_DIR).join("case.json"),
            r#"{"arguments": {}, "expect": {}}"#,
        )
        .unwrap();
        std::fs::write(
            dir.path().join(DECLARATION),
            serde_json::to_vec(&declaration).unwrap(),
        )
        .unwrap();
        dir
    }

    fn arguments() -> Value {
        json!({"type": "object", "properties": {
            "amount": {"type": "number", "description": "The bill"}
        }, "required": ["amount"]})
    }

    #[test]
    fn a_skill_without_a_declaration_has_no_program() {
        let dir = tempfile::tempdir().unwrap();
        assert!(Program::load(dir.path()).unwrap().is_none());
    }

    #[test]
    fn a_declaration_becomes_its_permission_and_schema() {
        let dir = skill(json!({"entry": "run.py", "arguments": arguments()}));
        let program = Program::load(dir.path()).unwrap().unwrap();
        assert_eq!(program.interpreter, Interpreter::Python);
        assert_eq!(program.permission().reach, vak_permission::Reach::ReadPath);
        assert!(program.schema()["properties"][TIME_LIMIT_ARGUMENT].is_object());

        let dir = skill(json!({"entry": "run.py", "arguments": arguments(), "writes": "drafts"}));
        let program = Program::load(dir.path()).unwrap().unwrap();
        assert_eq!(program.permission().reach, vak_permission::Reach::WritePath);
    }

    #[test]
    fn an_entry_outside_the_skill_is_refused() {
        for entry in ["../run.py", "/usr/bin/python3", "", "missing.py"] {
            let dir = skill(json!({"entry": entry, "arguments": arguments()}));
            assert!(Program::load(dir.path()).is_err(), "{entry}");
        }
        let dir = skill(json!({"entry": "run.rb", "arguments": arguments()}));
        std::fs::write(dir.path().join("run.rb"), "").unwrap();
        assert!(Program::load(dir.path()).unwrap_err().contains(".py"));
    }

    #[test]
    fn every_argument_is_typed_and_described() {
        let undescribed = json!({"type": "object", "properties": {"amount": {"type": "number"}}});
        let dir = skill(json!({"entry": "run.py", "arguments": undescribed}));
        assert!(
            Program::load(dir.path())
                .unwrap_err()
                .contains("description")
        );
        let reserved = json!({"type": "object", "properties": {
            TIME_LIMIT_ARGUMENT: {"type": "integer", "description": "mine"}
        }});
        let dir = skill(json!({"entry": "run.py", "arguments": reserved}));
        assert!(Program::load(dir.path()).unwrap_err().contains("runtime's"));
    }

    #[test]
    fn hosts_are_exact_names_and_make_the_call_leave_the_machine() {
        for bad in [
            "*.example.com",
            "https://api.example.com",
            "api.example.com/v1",
            "api.example.com:0",
            "",
        ] {
            let dir = skill(json!({"entry": "run.py", "arguments": arguments(), "hosts": [bad]}));
            assert!(
                Program::load(dir.path())
                    .unwrap_err()
                    .contains("exact host name"),
                "{bad}"
            );
        }
        let dir = skill(
            json!({"entry": "run.py", "arguments": arguments(), "hosts": ["api.example.com", "localhost:8443"]}),
        );
        let program = Program::load(dir.path()).unwrap().unwrap();
        assert!(program.leaves_machine());
        assert_eq!(program.permission().reach, vak_permission::Reach::Network);
        assert_eq!(program.provenance(), vak_control::Provenance::BOTH);
    }

    #[test]
    fn requirements_name_their_index_and_are_python_only() {
        let dir = skill(json!({"entry": "run.py", "arguments": arguments(),
            "requirements": {"file": "requirements.txt", "hosts": []}}));
        std::fs::write(dir.path().join("requirements.txt"), "").unwrap();
        assert!(
            Program::load(dir.path())
                .unwrap_err()
                .contains("package index")
        );
        let dir = skill(json!({"entry": "run.sh", "arguments": arguments(),
            "requirements": {"file": "requirements.txt", "hosts": ["pypi.org"]}}));
        std::fs::write(dir.path().join("run.sh"), "").unwrap();
        std::fs::write(dir.path().join("requirements.txt"), "").unwrap();
        assert!(Program::load(dir.path()).unwrap_err().contains("Python"));
        let dir = skill(json!({"entry": "run.py", "arguments": arguments(),
            "requirements": {"file": "requirements.txt", "hosts": ["pypi.org", "files.pythonhosted.org"]}}));
        std::fs::write(
            dir.path().join("requirements.txt"),
            "six==1.16.0 --hash=sha256:00\n",
        )
        .unwrap();
        let program = Program::load(dir.path()).unwrap().unwrap();
        let requirements = program.requirements.clone().unwrap();
        assert!(
            requirements
                .env
                .starts_with(vak_config::paths::cache_home())
        );
        assert_eq!(program.leaves_machine(), !requirements.installed());
    }

    #[test]
    fn a_program_without_tests_is_refused_with_what_to_add() {
        let dir = skill(json!({"entry": "run.py", "arguments": arguments()}));
        std::fs::remove_dir_all(dir.path().join(TESTS_DIR)).unwrap();
        let refused = Program::load(dir.path()).unwrap_err();
        assert!(refused.contains("tests/<name>.json"), "{refused}");
        std::fs::create_dir(dir.path().join(TESTS_DIR)).unwrap();
        std::fs::write(dir.path().join("tests/bad.json"), r#"{"arguments": {}}"#).unwrap();
        assert!(Program::load(dir.path()).unwrap_err().contains("either"));
    }

    #[test]
    fn a_case_names_where_the_answer_differs() {
        let case = TestCase {
            arguments: json!({}),
            expect: Some(json!({"total": "292.05", "shares": ["73.02", "73.01"]})),
            fails: false,
        };
        let answer = |value: Value| ToolOutput::ok(value.to_string());
        assert!(
            case.judge(
                &answer(json!({"total": "292.05", "shares": ["73.02", "73.01"], "extra": 1})),
                false
            )
            .is_ok()
        );
        let differs = case
            .judge(
                &answer(json!({"total": "292.05", "shares": ["73.01", "73.01"]})),
                false,
            )
            .unwrap_err();
        assert!(differs.contains("$.shares[0]"), "{differs}");
        assert!(
            case.judge(&answer(json!({"shares": ["73.02", "73.01"]})), false)
                .unwrap_err()
                .contains("$.total is missing")
        );
        assert!(
            case.judge(&ToolOutput::error(Fault::Tool, "boom"), false)
                .unwrap_err()
                .contains("failed")
        );
        let numbers = TestCase {
            arguments: json!({}),
            expect: Some(json!({"each": 30})),
            fails: false,
        };
        assert!(numbers.judge(&answer(json!({"each": 30.0})), false).is_ok());
        let drafts = TestCase {
            arguments: json!({}),
            expect: Some(json!({"ok": true})),
            fails: false,
        };
        assert!(
            drafts
                .judge(&answer(json!({"result": {"ok": true}, "drafts": []})), true)
                .is_ok()
        );
        let fails = TestCase {
            arguments: json!({}),
            expect: None,
            fails: true,
        };
        assert!(
            fails
                .judge(&ToolOutput::error(Fault::Tool, "no"), false)
                .is_ok()
        );
        assert!(fails.judge(&answer(json!({})), false).is_err());
    }

    #[test]
    fn a_bundle_is_the_folder_and_writes_back_as_it() {
        let dir = skill(json!({"entry": "run.py", "arguments": arguments()}));
        let bytes = bundle(dir.path()).unwrap();
        assert_eq!(
            bytes,
            bundle(dir.path()).unwrap(),
            "the same folder is the same bytes"
        );
        let out = tempfile::tempdir().unwrap();
        let copy = out.path().join("copy");
        unbundle(&bytes, &copy).unwrap();
        assert_eq!(bundle(&copy).unwrap(), bytes);
        assert_eq!(
            Program::load(&copy).unwrap().unwrap().digest,
            Program::load(dir.path()).unwrap().unwrap().digest
        );
        assert!(
            unbundle(&bytes, &copy).is_err(),
            "never over an existing folder"
        );
        let escape = serde_json::to_vec(&json!({"files": {"../x": "no"}})).unwrap();
        assert!(unbundle(&escape, &out.path().join("other")).is_err());
    }

    #[test]
    fn any_file_changing_changes_the_program() {
        let dir = skill(json!({"entry": "run.py", "arguments": arguments()}));
        let before = Program::load(dir.path()).unwrap().unwrap().digest;
        std::fs::write(
            dir.path().join("tests/other.json"),
            r#"{"arguments": {}, "fails": true}"#,
        )
        .unwrap();
        let after = Program::load(dir.path()).unwrap().unwrap().digest;
        assert_ne!(before, after);
    }
}
