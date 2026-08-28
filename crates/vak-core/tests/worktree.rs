#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::fs;
use std::path::Path;
use std::process::Command;

use tempfile::tempdir;

use vak_core::worktree;

fn git(repo: &Path, args: &[&str]) {
    let out = Command::new("git")
        .current_dir(repo)
        .args(args)
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "git {args:?} failed: {}",
        String::from_utf8_lossy(&out.stderr)
    );
}

fn init_repo() -> (tempfile::TempDir, std::path::PathBuf) {
    let dir = tempdir().unwrap();
    let repo = dir.path().join("repo");
    fs::create_dir_all(&repo).unwrap();
    git(&repo, &["init", "-q"]);
    git(&repo, &["config", "user.email", "t@t"]);
    git(&repo, &["config", "user.name", "t"]);
    fs::write(repo.join("base.txt"), "base\n").unwrap();
    git(&repo, &["add", "."]);
    git(&repo, &["commit", "-q", "-m", "init"]);
    (dir, repo)
}

#[test]
fn worktree_create_and_remove_lifecycle() {
    let (_dir, repo) = init_repo();

    assert!(worktree::is_git_repo(&repo));
    let wt = worktree::create(&repo, "run-1").unwrap();
    assert!(wt.path.is_dir());
    assert_eq!(wt.branch, "vak/run-1");
    assert!(
        wt.path.join("base.txt").exists(),
        "worktree must contain committed files"
    );

    // Mutations in the worktree never touch the main checkout.
    fs::write(wt.path.join("mutated.txt"), "dirty").unwrap();
    assert!(!repo.join("mutated.txt").exists());

    worktree::remove(&repo, &wt).unwrap();
    assert!(!wt.path.exists());
    let branches = Command::new("git")
        .current_dir(&repo)
        .args(["branch", "--list", "vak/run-1"])
        .output()
        .unwrap();
    assert!(
        String::from_utf8_lossy(&branches.stdout).trim().is_empty(),
        "branch must be deleted"
    );
}

#[test]
fn non_repo_is_rejected() {
    let dir = tempdir().unwrap();
    assert!(matches!(
        worktree::create(dir.path(), "x"),
        Err(worktree::WorktreeError::NotARepo)
    ));
}
