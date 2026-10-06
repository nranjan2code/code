//! Write leases for background workers
//! (docs/design/84-worker-questions-and-control.md §5.1).
//!
//! A foreground `task` call holds its resource claims only while it runs, so
//! the parent cannot touch the same files in the meantime. A background
//! worker outlives its `task` call, so its claims become a lease the registry
//! holds until the worker ends. Two halves keep it honest: the worker's file
//! tools refuse any path outside its lease (`ScopedWriteTool`), and the
//! parent's own calls that would touch leased paths are refused until the
//! worker finishes or is stopped (`WorkerRegistry::lease_conflict`).

use std::path::{Component, Path, PathBuf};
use std::sync::Arc;

use serde_json::Value;
use vak_tools::{ResourceClaims, Tool, ToolContext, ToolOutput};

/// The file tools a background writer is given, each fenced to its lease.
/// `bash` and every other effectful tool stay out: they cannot be confined to
/// a path.
pub const WRITER_TOOLS: &[&str] = &["write", "edit"];

/// A lease must name workspace-relative files or folders, never the whole
/// workspace, and must not climb out of it.
pub fn normalize_scopes(paths: &[String]) -> Result<Vec<String>, String> {
    let mut scopes = Vec::new();
    for raw in paths {
        let trimmed = raw.trim().trim_end_matches(['/', '*']);
        let trimmed = trimmed.strip_prefix("./").unwrap_or(trimmed);
        if trimmed.is_empty() || trimmed == "." {
            return Err(
                "a background writer must name specific files or folders, not the whole workspace"
                    .into(),
            );
        }
        let path = Path::new(trimmed);
        if path.is_absolute()
            || path
                .components()
                .any(|part| matches!(part, Component::ParentDir | Component::Prefix(_)))
        {
            return Err(format!(
                "'{raw}' is not a path inside the workspace; use workspace-relative paths without '..'"
            ));
        }
        scopes.push(trimmed.to_string());
    }
    if scopes.is_empty() {
        return Err("a background writer needs paths: the files or folders it may change".into());
    }
    Ok(scopes)
}

/// The claims a lease makes, in the same terms the loop's scheduler uses.
pub fn lease_claims(scopes: &[String]) -> ResourceClaims {
    ResourceClaims {
        exclusive: false,
        read_only: false,
        paths: scopes.to_vec(),
    }
}

fn lexical(path: &Path) -> PathBuf {
    let mut out = PathBuf::new();
    for part in path.components() {
        match part {
            Component::CurDir => {}
            Component::ParentDir => {
                out.pop();
            }
            other => out.push(other.as_os_str()),
        }
    }
    out
}

/// Canonicalize the longest existing prefix and append the missing suffix.
/// `Path::canonicalize` on the full target alone misses a symlink escape when
/// only the final file (or a deeper child) has not been created yet.
fn resolve_missing_suffix(path: &Path) -> Option<PathBuf> {
    let mut ancestor = path;
    let mut suffix = Vec::new();
    while ancestor.canonicalize().is_err() {
        suffix.push(ancestor.file_name()?.to_os_string());
        ancestor = ancestor.parent()?;
    }
    let mut resolved = ancestor.canonicalize().ok()?;
    for part in suffix.into_iter().rev() {
        resolved.push(part);
    }
    Some(resolved)
}

/// Whether `target` (relative to `cwd`, or absolute) lies inside one of the
/// scopes. Compared lexically, and again through the filesystem when the path
/// already exists, so a symlink inside a scope cannot lead outside it.
pub fn in_scope(cwd: &Path, target: &str, scopes: &[String]) -> bool {
    let full = lexical(&cwd.join(target));
    let Some(root) = cwd.canonicalize().ok() else {
        return false;
    };
    let Some(real_target) = resolve_missing_suffix(&full) else {
        return false;
    };
    scopes.iter().any(|scope| {
        let lexical_scope = lexical(&cwd.join(scope));
        if !full.starts_with(&lexical_scope) {
            return false;
        }
        resolve_missing_suffix(&lexical_scope).is_some_and(|real_scope| {
            real_scope.starts_with(&root)
                && real_target.starts_with(&real_scope)
                && real_target.starts_with(&root)
        })
    })
}

/// `write` or `edit`, refusing every path outside the worker's lease.
pub struct ScopedWriteTool {
    inner: Arc<dyn Tool>,
    scopes: Vec<String>,
}

impl ScopedWriteTool {
    pub fn new(inner: Arc<dyn Tool>, scopes: Vec<String>) -> Self {
        ScopedWriteTool { inner, scopes }
    }

    fn outside(&self, args: &Value, cwd: &Path) -> Option<String> {
        let path = args.get("path").and_then(Value::as_str)?;
        (!in_scope(cwd, path, &self.scopes)).then(|| {
            format!(
                "'{path}' is outside this worker's write lease ({}); ask the agent that started you if more is needed",
        self.scopes.join(", ")
            )
        })
    }
}

#[async_trait::async_trait]
impl Tool for ScopedWriteTool {
    fn name(&self) -> &str {
        self.inner.name()
    }

    fn description(&self) -> &str {
        self.inner.description()
    }

    fn schema(&self) -> Value {
        self.inner.schema()
    }

    fn claims(&self, args: &Value) -> ResourceClaims {
        self.inner.claims(args)
    }

    fn refusal(&self, args: &Value) -> Option<String> {
        self.inner.refusal(args)
    }

    fn serves(&self) -> &'static [&'static str] {
        self.inner.serves()
    }

    fn always_loaded(&self) -> bool {
        self.inner.always_loaded()
    }

    fn delivered_file(&self, args: &Value) -> Option<String> {
        self.inner.delivered_file(args)
    }

    fn file_access(&self, args: &Value) -> Option<(vak_tools::FileAccess, String)> {
        self.inner.file_access(args)
    }

    fn artifact(&self, args: &Value) -> Option<vak_tools::ArtifactClaim> {
        self.inner.artifact(args)
    }

    fn canonical_input(&self, input: &Value) -> Option<Value> {
        self.inner.canonical_input(input)
    }

    async fn execute(&self, args: &Value, ctx: &ToolContext) -> ToolOutput {
        if let Some(reason) = self.outside(args, &ctx.cwd) {
            return ToolOutput::error(reason);
        }
        self.inner.execute(args, ctx).await
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::*;

    fn scopes(items: &[&str]) -> Vec<String> {
        normalize_scopes(&items.iter().map(|s| s.to_string()).collect::<Vec<_>>()).unwrap()
    }

    #[test]
    fn a_lease_names_specific_workspace_relative_paths() {
        assert_eq!(
            scopes(&["src/**", "./docs/a.md", "notes/"]),
            vec!["src", "docs/a.md", "notes"]
        );
        for bad in ["", ".", "*", "**", "/etc", "../x", "a/../../x"] {
            assert!(
                normalize_scopes(&[bad.to_string()]).is_err(),
                "{bad:?} must be refused"
            );
        }
        assert!(normalize_scopes(&[]).is_err());
    }

    #[test]
    fn scope_matching_is_by_path_component_and_cannot_be_climbed_out_of() {
        let dir = tempfile::tempdir().unwrap();
        let cwd = dir.path();
        let lease = scopes(&["src", "docs/a.md"]);
        assert!(in_scope(cwd, "src/lib.rs", &lease));
        assert!(in_scope(cwd, "docs/a.md", &lease));
        assert!(in_scope(cwd, "./src/deep/x.rs", &lease));
        assert!(
            !in_scope(cwd, "src2/lib.rs", &lease),
            "a sibling that shares a prefix"
        );
        assert!(!in_scope(cwd, "docs/b.md", &lease));
        assert!(!in_scope(cwd, "src/../secret", &lease));
        assert!(!in_scope(cwd, "/etc/passwd", &lease));
    }

    #[cfg(unix)]
    #[test]
    fn a_symlink_inside_the_lease_cannot_lead_outside_it() {
        let dir = tempfile::tempdir().unwrap();
        let cwd = dir.path();
        std::fs::create_dir_all(cwd.join("src")).unwrap();
        std::fs::create_dir_all(cwd.join("private")).unwrap();
        std::os::unix::fs::symlink(cwd.join("private"), cwd.join("src/link")).unwrap();
        let lease = scopes(&["src"]);
        assert!(!in_scope(cwd, "src/link", &lease));
        assert!(!in_scope(cwd, "src/link/new-file.txt", &lease));
    }
}
