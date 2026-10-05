//! Git worktree isolation: each isolated run gets its own worktree + branch
//! off HEAD, so mutations never touch the user's checkout.

use std::path::{Path, PathBuf};
use std::process::Command;

#[derive(Debug, thiserror::Error)]
pub enum WorktreeError {
    #[error("not a git repository")]
    NotARepo,
    #[error("git failed: {0}")]
    Git(String),
    #[error("io error: {0}")]
    Io(#[from] std::io::Error),
}

fn git(repo: &Path, args: &[&str]) -> Result<String, WorktreeError> {
    let out = Command::new("git")
        .current_dir(repo)
        .args(args)
        .output()
        .map_err(WorktreeError::Io)?;
    if !out.status.success() {
        return Err(WorktreeError::Git(
            String::from_utf8_lossy(&out.stderr).trim().to_string(),
        ));
    }
    Ok(String::from_utf8_lossy(&out.stdout).trim().to_string())
}

pub fn is_git_repo(cwd: &Path) -> bool {
    Command::new("git")
        .current_dir(cwd)
        .args(["rev-parse", "--is-inside-work-tree"])
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
}

pub struct Worktree {
    pub path: PathBuf,
    pub branch: String,
}

/// Creates the run's environment (`vak_config::paths::environment_dir`) as a
/// worktree on branch `vak/<run_id>`.
pub fn create(repo: &Path, run_id: &str) -> Result<Worktree, WorktreeError> {
    if !is_git_repo(repo) {
        return Err(WorktreeError::NotARepo);
    }
    let path = vak_config::paths::environment_dir(run_id);
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let branch = format!("vak/{run_id}");
    git(
        repo,
        &[
            "worktree",
            "add",
            "-b",
            &branch,
            path.to_string_lossy().as_ref(),
            "HEAD",
        ],
    )?;
    Ok(Worktree { path, branch })
}

/// Removes the worktree and its branch (uncommitted changes are discarded —
/// callers should surface that before calling).
pub fn remove(repo: &Path, wt: &Worktree) -> Result<(), WorktreeError> {
    git(
        repo,
        &[
            "worktree",
            "remove",
            "--force",
            wt.path.to_string_lossy().as_ref(),
        ],
    )?;
    git(repo, &["branch", "-D", &wt.branch])?;
    Ok(())
}

/// Removes every worktree whose branch is `vak/<run_prefix>…`, and those
/// branches; returns how many went. For work that keeps only its latest
/// run's worktree.
pub fn remove_runs_with_prefix(repo: &Path, run_prefix: &str) -> Result<usize, WorktreeError> {
    let branch_prefix = format!("refs/heads/vak/{run_prefix}");
    let listing = git(repo, &["worktree", "list", "--porcelain"])?;
    let mut removed = 0;
    let mut path: Option<PathBuf> = None;
    for line in listing.lines() {
        if let Some(p) = line.strip_prefix("worktree ") {
            path = Some(PathBuf::from(p));
        } else if let Some(branch) = line.strip_prefix("branch ")
            && branch.starts_with(&branch_prefix)
            && let Some(path) = path.take()
        {
            remove(
                repo,
                &Worktree {
                    path,
                    branch: branch.trim_start_matches("refs/heads/").to_string(),
                },
            )?;
            removed += 1;
        }
    }
    Ok(removed)
}
