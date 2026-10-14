//! A skill's program: a script with a declared argument schema that the
//! runtime runs as a tool (docs/design/93-self-evolving-capabilities.md §3).
//!
//! The declaration (`capability.json` beside `SKILL.md`) is the tool's
//! permission and the program's sandbox at once: the sandbox is built from
//! what the program declares, never from the session's mode, so a program
//! that declares no network has none even under FullAccess. The host side
//! ([`CapabilityTool`]) checks and dispatches; the program runs in a broker
//! worker ([`Runner`]) that speaks JSON on stdin and stdout.

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
    /// Hosts it connects to; empty means no network at all.
    #[serde(default)]
    pub hosts: Vec<String>,
    #[serde(default)]
    pub writes: Writes,
    /// Pinned dependencies, relative to the skill directory.
    #[serde(default)]
    pub requirements: Option<String>,
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

/// A loaded, checked program.
#[derive(Debug, Clone)]
pub struct Program {
    pub dir: PathBuf,
    pub entry: PathBuf,
    pub interpreter: Interpreter,
    pub arguments: Value,
    pub writes: Writes,
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
        let declaration: Declaration = serde_json::from_str(&text)
            .map_err(|error| format!("{DECLARATION} is not a valid declaration: {error}"))?;
        if !declaration.hosts.is_empty() {
            return Err(format!(
                "{DECLARATION} declares hosts; a program that reaches the network runs \
                 through the egress proxy, which is not built yet"
            ));
        }
        if declaration.requirements.is_some() {
            return Err(format!(
                "{DECLARATION} declares requirements; installing dependencies is not built yet"
            ));
        }
        let entry = inside(dir, &declaration.entry)?;
        let interpreter = Interpreter::of(&entry).ok_or_else(|| {
            format!(
                "entry '{}' must end in .py, .sh, .mjs or .js",
                declaration.entry
            )
        })?;
        check_arguments(&declaration.arguments)?;
        Ok(Some(Self {
            dir: dir.to_path_buf(),
            entry,
            interpreter,
            arguments: declaration.arguments,
            writes: declaration.writes,
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

    /// What it reaches, from its declaration alone.
    pub fn permission(&self) -> vak_permission::Declared {
        vak_permission::Declared::reach(match self.writes {
            Writes::None => vak_permission::Reach::ReadPath,
            Writes::Drafts => vak_permission::Reach::WritePath,
        })
    }
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
            "entry '{relative}' must be a path inside the skill directory"
        ));
    }
    let root = dir
        .canonicalize()
        .map_err(|error| format!("skill directory is unavailable: {error}"))?;
    let resolved = root
        .join(candidate)
        .canonicalize()
        .map_err(|_| format!("entry '{relative}' does not exist"))?;
    if !resolved.starts_with(&root) || !resolved.is_file() {
        return Err(format!(
            "entry '{relative}' must be a file inside the skill directory"
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

/// The sandbox a program runs in: reads the system, the workspace, its own
/// skill directory and its interpreter's installation; writes its temporary
/// directory and, when declared, its drafts; reaches no network. `None`
/// where no OS backend can confine it, and the call is then refused.
fn sandbox_for(
    workspace: &Path,
    reads: &[PathBuf],
    writes: &[PathBuf],
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
    {
        let mut sandbox =
            crate::sandbox::Seatbelt::task_copy(crate::sandbox::SandboxMode::ReadOnly, workspace);
        sandbox.mode = crate::sandbox::SandboxMode::WorkspaceWrite;
        sandbox.read_paths.extend(reads);
        sandbox.read_paths.extend(writes.iter().cloned());
        sandbox.write_paths = writes;
        Some(Arc::new(sandbox))
    }
    #[cfg(target_os = "linux")]
    {
        let mut sandbox =
            crate::landlock::Landlock::task_copy(crate::sandbox::SandboxMode::ReadOnly, workspace);
        sandbox.mode = crate::sandbox::SandboxMode::WorkspaceWrite;
        sandbox.read_paths.extend(reads);
        sandbox.read_paths.extend(writes.iter().cloned());
        sandbox.write_paths = writes;
        Some(Arc::new(sandbox))
    }
    #[cfg(not(any(target_os = "macos", target_os = "linux")))]
    {
        let _ = (workspace, reads, writes);
        None
    }
}

/// A runnable skill as a tool on the host: what the model sees and what
/// the permission engine reads. Calls go to a worker under the program's
/// own sandbox.
pub struct CapabilityTool {
    name: String,
    description: String,
    program: Program,
    worker_exe: PathBuf,
}

impl CapabilityTool {
    pub fn new(name: String, description: String, program: Program, worker_exe: PathBuf) -> Self {
        Self {
            name,
            description,
            program,
            worker_exe,
        }
    }
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
        let execution_id = ctx
            .sandbox_sink
            .as_ref()
            .map(|sink| sink.execution_id().to_string())
            .unwrap_or_else(|| "capability".into());
        let own = crate::drafts::own_root(ctx);
        let tmp = own.join("tmp").join(&execution_id);
        let cache = own.join("cache");
        let drafts =
            (self.program.writes == Writes::Drafts).then(|| crate::drafts::dir(ctx, &execution_id));
        for dir in [Some(&tmp), Some(&cache), drafts.as_ref()]
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
        let mut writes = vec![tmp.clone(), cache.clone()];
        writes.extend(drafts.clone());
        let Some(sandbox) = sandbox_for(&ctx.cwd, &reads, &writes) else {
            return ToolOutput::error(
                Fault::Refused,
                "no sandbox on this system can confine a skill's program, so it is not run",
            );
        };
        crate::broker::run_capability(
            &self.worker_exe,
            sandbox.as_ref(),
            crate::broker::CapabilityRun {
                name: self.name.clone(),
                interpreter,
                entry: self.program.entry.clone(),
                args: program_args,
                tmp,
                cache,
                drafts,
                timeout_ms,
            },
            ctx,
        )
        .await
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

/// The program run inside the worker: arguments as JSON on stdin, one JSON
/// value on stdout.
pub struct Runner {
    pub run: crate::broker::CapabilityRun,
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
        let mut command = tokio::process::Command::new(&run.interpreter);
        command
            .arg(&run.entry)
            .current_dir(&ctx.cwd)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        crate::bash::scrub_environment(&mut command);
        command
            .env("TMPDIR", &run.tmp)
            .env("XDG_CACHE_HOME", &run.cache)
            .env("PYTHONDONTWRITEBYTECODE", "1")
            .env("npm_config_cache", run.cache.join("npm"));
        if let Some(drafts) = &run.drafts {
            command.env("VAK_DRAFTS", drafts);
        }
        crate::bash::isolate_process_group(&mut command);
        if let Some(sink) = &ctx.sandbox_sink {
            sink.emit_execution_started(
                &run.name,
                &run.entry.display().to_string(),
                run.interpreter
                    .file_name()
                    .and_then(|name| name.to_str())
                    .unwrap_or("program"),
                &ctx.cwd.display().to_string(),
            );
        }
        let started = std::time::Instant::now();
        let mut child = match command.spawn() {
            Ok(child) => child,
            Err(error) => {
                return ToolOutput::error(
                    Fault::Tool,
                    format!("the program did not start: {error}"),
                );
            }
        };
        let pid = child.id();
        if let Some(mut stdin) = child.stdin.take() {
            let input = serde_json::to_vec(&run.args).unwrap_or_default();
            if stdin.write_all(&input).await.is_err() {
                crate::bash::kill_process_group(&pid);
                let _ = child.wait().await;
                return ToolOutput::error(Fault::Tool, "the program did not read its arguments");
            }
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
        let limit = tokio::time::sleep(std::time::Duration::from_millis(run.timeout_ms));
        let ended = tokio::select! {
            status = child.wait() => Ended::Exited(status),
            _ = limit => Ended::Limit,
            _ = ctx.cancel.cancelled() => Ended::Cancelled,
        };
        crate::bash::kill_process_group(&pid);
        if !matches!(ended, Ended::Exited(_)) {
            let _ = child.wait().await;
        }
        let out = out.await.unwrap_or_default();
        let err = err.await.unwrap_or_default();
        let duration_ms = started.elapsed().as_millis() as u64;
        let drafts = run
            .drafts
            .as_deref()
            .map(|dir| written(dir, ctx))
            .unwrap_or_default();
        if let Some(sink) = &ctx.sandbox_sink {
            let code = match &ended {
                Ended::Exited(Ok(status)) => status.code().unwrap_or(-1),
                _ => -1,
            };
            sink.emit_finished(code, duration_ms, drafts.clone());
        }
        let status = match ended {
            Ended::Limit => {
                return ToolOutput::error(
                    Fault::Limit,
                    format!(
                        "the program stopped at its time limit of {}ms{}",
                        run.timeout_ms,
                        diagnostics(&err)
                    ),
                )
                .ran(None, duration_ms);
            }
            Ended::Cancelled => {
                return ToolOutput::error(Fault::Cancelled, "the program was stopped")
                    .ran(None, duration_ms);
            }
            Ended::Exited(Err(error)) => {
                return ToolOutput::error(Fault::Tool, format!("the program was lost: {error}"));
            }
            Ended::Exited(Ok(status)) => status,
        };
        let code = status.code();
        if !status.success() {
            return ToolOutput::error(
                Fault::Tool,
                format!(
                    "the program exited with {}{}{}",
                    code.map_or_else(|| "a signal".to_string(), |code| code.to_string()),
                    diagnostics(&err),
                    if out.trim().is_empty() {
                        String::new()
                    } else {
                        format!("\n\n[output]\n{}", out.trim())
                    }
                ),
            )
            .ran(code, duration_ms);
        }
        let result: Value = match serde_json::from_str(out.trim()) {
            Ok(result) => result,
            Err(_) => {
                return ToolOutput::error(
                    Fault::Tool,
                    format!(
                        "the program must print one JSON value on stdout; it printed:\n{}{}",
                        out.trim(),
                        diagnostics(&err)
                    ),
                )
                .ran(code, duration_ms);
            }
        };
        let content = match &run.drafts {
            Some(_) => serde_json::json!({"result": result, "drafts": drafts}),
            None => result,
        };
        ToolOutput::ok(content.to_string()).ran(code, duration_ms)
    }
}

enum Ended {
    Exited(std::io::Result<std::process::ExitStatus>),
    Limit,
    Cancelled,
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
    fn what_is_not_built_yet_is_refused_by_name() {
        let dir = skill(
            json!({"entry": "run.py", "arguments": arguments(), "hosts": ["api.example.com"]}),
        );
        assert!(
            Program::load(dir.path())
                .unwrap_err()
                .contains("egress proxy")
        );
        let dir = skill(
            json!({"entry": "run.py", "arguments": arguments(), "requirements": "requirements.txt"}),
        );
        assert!(
            Program::load(dir.path())
                .unwrap_err()
                .contains("dependencies")
        );
    }

    #[test]
    fn any_file_changing_changes_the_program() {
        let dir = skill(json!({"entry": "run.py", "arguments": arguments()}));
        let before = Program::load(dir.path()).unwrap().unwrap().digest;
        std::fs::create_dir(dir.path().join("tests")).unwrap();
        std::fs::write(dir.path().join("tests/case.json"), "{}").unwrap();
        let after = Program::load(dir.path()).unwrap().unwrap().digest;
        assert_ne!(before, after);
    }
}
