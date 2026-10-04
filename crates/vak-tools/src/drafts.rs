//! Where a name a tool call is given lives (plan M3b slice 4). A draft is
//! named `.vak/scratch/<agent>/<execution>/<path>`, the name the model is
//! told and the ledger records; the file is in this space's execution root
//! in the runtime directory, outside the project tree. That root is the one
//! scoped exception to invariant 10: a call reaches exactly its own Agent's
//! drafts there, canonicalised, never another Agent's and never through a
//! symlink or `..`. Every other name resolves inside the workspace.

use crate::ToolContext;
use std::path::{Component, Path, PathBuf};

/// The directory this call's Agent keeps its executions' drafts in.
pub fn own_root(ctx: &ToolContext) -> PathBuf {
    ctx.executions_root().join(ctx.agent())
}

/// Where `execution`'s drafts are written.
pub fn dir(ctx: &ToolContext, execution: &str) -> PathBuf {
    own_root(ctx).join(execution)
}

/// The name of the draft of the workspace file `relative` that `execution`
/// writes.
pub fn name(ctx: &ToolContext, execution: &str, relative: &Path) -> String {
    Path::new(vak_config::scope::SCRATCH_DIR)
        .join(ctx.agent())
        .join(execution)
        .join(relative)
        .to_string_lossy()
        .replace('\\', "/")
}

/// The existing file `name` names, canonical and confined: a draft of this
/// call's Agent inside its execution root, or a file inside the workspace.
pub fn existing(ctx: &ToolContext, name: &str) -> Result<PathBuf, String> {
    let given = Path::new(name);
    let own = own_root(ctx);
    if let Some(rest) = given
        .strip_prefix(vak_config::scope::SCRATCH_DIR)
        .ok()
        .filter(|_| !given.is_absolute())
    {
        let mut parts = rest.components();
        match parts.next() {
            Some(Component::Normal(agent)) if agent == ctx.agent() => {}
            _ => return Err(format!("access denied: {name} is another Agent's draft")),
        }
        return confined(&own.join(parts.as_path()), &own, name);
    }
    let workspace = ctx
        .cwd
        .canonicalize()
        .map_err(|error| format!("cannot resolve workspace root: {error}"))?;
    let candidate = if given.is_absolute() {
        given.to_path_buf()
    } else {
        ctx.cwd.join(given)
    };
    let canonical = candidate
        .canonicalize()
        .map_err(|error| format!("cannot open {name}: {error}"))?;
    if canonical.starts_with(&workspace) {
        return Ok(canonical);
    }
    let own = own.canonicalize().unwrap_or(own);
    if canonical.starts_with(&own) {
        return Ok(canonical);
    }
    Err(format!("access denied: {name} is outside the workspace"))
}

fn confined(path: &Path, root: &Path, name: &str) -> Result<PathBuf, String> {
    let canonical = path
        .canonicalize()
        .map_err(|error| format!("cannot open {name}: {error}"))?;
    let root = root.canonicalize().unwrap_or_else(|_| root.to_path_buf());
    if canonical.starts_with(&root) {
        Ok(canonical)
    } else {
        Err(format!("access denied: {name} escapes its draft directory"))
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    fn ctx(workspace: &Path, executions: &Path, agent: &str) -> ToolContext {
        ToolContext::new(workspace.to_path_buf())
            .with_executions(executions.to_path_buf())
            .with_agent_id(agent)
    }

    /// Exit test of M3b slice 4: a draft is named as before, lives outside
    /// the project tree, and reads back by its name.
    #[test]
    fn drafts_live_outside_the_project() {
        let workspace = tempfile::tempdir().unwrap();
        let executions = tempfile::tempdir().unwrap();
        let ctx = ctx(workspace.path(), executions.path(), "mira");
        let written = dir(&ctx, "exec-7").join("budget.xlsx");
        std::fs::create_dir_all(written.parent().unwrap()).unwrap();
        std::fs::write(&written, b"draft").unwrap();
        let named = name(&ctx, "exec-7", Path::new("budget.xlsx"));
        assert_eq!(named, ".vak/scratch/mira/exec-7/budget.xlsx");
        let found = existing(&ctx, &named).unwrap();
        assert_eq!(found, written.canonicalize().unwrap());
        assert!(!found.starts_with(workspace.path().canonicalize().unwrap()));
        assert!(!workspace.path().join(".vak").exists());
    }

    #[test]
    fn another_agents_drafts_are_unreachable() {
        let workspace = tempfile::tempdir().unwrap();
        let executions = tempfile::tempdir().unwrap();
        let theirs = executions.path().join("writer/exec-1/notes.docx");
        std::fs::create_dir_all(theirs.parent().unwrap()).unwrap();
        std::fs::write(&theirs, b"private").unwrap();
        let mine = ctx(workspace.path(), executions.path(), "mira");
        assert!(existing(&mine, ".vak/scratch/writer/exec-1/notes.docx").is_err());
        assert!(existing(&mine, ".vak/scratch/mira/../writer/exec-1/notes.docx").is_err());
        assert!(existing(&mine, &theirs.display().to_string()).is_err());
        std::fs::create_dir_all(executions.path().join("mira/exec-2")).unwrap();
        std::os::unix::fs::symlink(&theirs, executions.path().join("mira/exec-2/link.docx"))
            .unwrap();
        assert!(existing(&mine, ".vak/scratch/mira/exec-2/link.docx").is_err());
    }
}
