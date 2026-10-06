//! `CopyEnvironment` (data-architecture plan M4.8): the environment for work
//! on a folder that is not a git repository. The folder is copied, within
//! fixed ignore rules and size caps, into the run's own environment
//! (`vak_config::paths::environment_dir`); the work runs there; and what it
//! changed, added and deleted is exported as a candidate for the one Review
//! path, never written back by itself.

use crate::{
    CandidateFile, CandidateManifest, CandidateOperation, EnvironmentBackend, EnvironmentPlan,
    EnvironmentState, Error, digest,
};
use std::collections::{BTreeMap, HashMap};
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use walkdir::WalkDir;

/// Names never copied in or out, at any depth: version-control and Vak
/// control state, and rebuildable dependency and build trees.
pub const IGNORED: &[&str] = &[
    ".git",
    vak_config::scope::PROJECT_DIR,
    "node_modules",
    "target",
    ".venv",
    "__pycache__",
    ".DS_Store",
];

/// How large a folder a copy environment takes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CopyLimits {
    pub max_files: usize,
    pub max_bytes: u64,
}

impl Default for CopyLimits {
    fn default() -> Self {
        Self {
            max_files: 20_000,
            max_bytes: 512 * 1024 * 1024,
        }
    }
}

/// The copy backend: one plan per environment id.
#[derive(Debug, Default)]
pub struct CopyEnvironment {
    limits: CopyLimits,
    plans: Mutex<HashMap<String, (EnvironmentPlan, EnvironmentState)>>,
}

/// Every file under `root` a copy takes, by its relative path: regular files
/// only (a symlink is neither followed nor copied), skipping [`IGNORED`].
fn files(root: &Path, limits: CopyLimits) -> Result<BTreeMap<String, PathBuf>, Error> {
    let mut found = BTreeMap::new();
    let mut bytes = 0u64;
    let walk = WalkDir::new(root)
        .follow_links(false)
        .into_iter()
        .filter_entry(|entry| {
            entry.depth() == 0
                || !IGNORED
                    .iter()
                    .any(|name| entry.file_name() == std::ffi::OsStr::new(name))
        });
    for entry in walk {
        let entry = entry.map_err(|error| Error::Io(std::io::Error::other(error.to_string())))?;
        if !entry.file_type().is_file() {
            continue;
        }
        let relative = entry
            .path()
            .strip_prefix(root)
            .map_err(|_| Error::PathEscape(entry.path().display().to_string()))?
            .to_string_lossy()
            .replace('\\', "/");
        bytes += entry
            .metadata()
            .map_err(|error| Error::Io(std::io::Error::other(error.to_string())))?
            .len();
        found.insert(relative, entry.path().to_path_buf());
        if found.len() > limits.max_files || bytes > limits.max_bytes {
            return Err(Error::InvalidPlan(format!(
                "{} is larger than a copy holds ({} files or {} MiB at most)",
                root.display(),
                limits.max_files,
                limits.max_bytes / (1024 * 1024)
            )));
        }
    }
    Ok(found)
}

impl CopyEnvironment {
    pub fn new(limits: CopyLimits) -> Self {
        Self {
            limits,
            plans: Mutex::new(HashMap::new()),
        }
    }

    fn plan(&self, environment_id: &str) -> Result<EnvironmentPlan, Error> {
        self.plans
            .lock()
            .map_err(|_| Error::InvalidPlan("copy environments poisoned".into()))?
            .get(environment_id)
            .map(|(plan, _)| plan.clone())
            .ok_or_else(|| Error::InvalidPlan(format!("no copy environment {environment_id}")))
    }

    fn set_state(&self, environment_id: &str, state: EnvironmentState) {
        if let Ok(mut plans) = self.plans.lock()
            && let Some(entry) = plans.get_mut(environment_id)
        {
            entry.1 = state;
        }
    }

    /// Removes the environment's copy and its staged changes.
    pub fn remove(&self, environment_id: &str) -> Result<(), Error> {
        let plan = self.plan(environment_id)?;
        let _ = fs::remove_dir_all(staging_dir(&plan));
        match fs::remove_dir_all(&plan.task_root) {
            Ok(()) => {}
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => return Err(error.into()),
        }
        self.set_state(environment_id, EnvironmentState::Expired);
        Ok(())
    }
}

/// Where the changes of an environment are staged: beside its copy, never
/// inside it.
fn staging_dir(plan: &EnvironmentPlan) -> PathBuf {
    let mut name = plan
        .task_root
        .file_name()
        .map(|name| name.to_os_string())
        .unwrap_or_default();
    name.push(".changes");
    plan.task_root.with_file_name(name)
}

impl EnvironmentBackend for CopyEnvironment {
    fn name(&self) -> &str {
        "copy"
    }

    /// Copies `input_root` into a fresh `task_root`.
    fn prepare(&self, plan: &EnvironmentPlan) -> Result<(), Error> {
        if plan.backend != "copy" {
            return Err(Error::InvalidPlan(format!(
                "a copy environment cannot prepare a {} plan",
                plan.backend
            )));
        }
        if plan.task_root.starts_with(&plan.input_root) {
            return Err(Error::InvalidPlan(
                "a copy environment never lives inside the folder it copies".into(),
            ));
        }
        self.plans
            .lock()
            .map_err(|_| Error::InvalidPlan("copy environments poisoned".into()))?
            .insert(plan.id.clone(), (plan.clone(), EnvironmentState::Preparing));
        let copied = (|| -> Result<(), Error> {
            if plan.task_root.exists() {
                return Err(Error::InvalidPlan(format!(
                    "{} already exists",
                    plan.task_root.display()
                )));
            }
            fs::create_dir_all(&plan.task_root)?;
            for (relative, source) in files(&plan.input_root, self.limits)? {
                let target = plan.task_root.join(&relative);
                if let Some(parent) = target.parent() {
                    fs::create_dir_all(parent)?;
                }
                fs::copy(&source, &target)?;
            }
            Ok(())
        })();
        match copied {
            Ok(()) => {
                self.set_state(&plan.id, EnvironmentState::Ready);
                Ok(())
            }
            Err(error) => {
                let _ = fs::remove_dir_all(&plan.task_root);
                self.set_state(&plan.id, EnvironmentState::Failed);
                Err(error)
            }
        }
    }

    fn state(&self, environment_id: &str) -> EnvironmentState {
        self.plans
            .lock()
            .ok()
            .and_then(|plans| plans.get(environment_id).map(|(_, state)| state.clone()))
            .unwrap_or(EnvironmentState::Expired)
    }

    fn cancel(&self, execution_id: &str) -> Result<(), Error> {
        self.set_state(execution_id, EnvironmentState::Stopped);
        Ok(())
    }

    /// What the work in the copy changed, against the folder as it is now:
    /// added and changed files staged beside the copy, deleted files as
    /// delete operations. Unchanged files are not part of it.
    fn export_candidate(&self, environment_id: &str) -> Result<CandidateManifest, Error> {
        let plan = self.plan(environment_id)?;
        let after = files(&plan.task_root, self.limits)?;
        let before = files(&plan.input_root, self.limits)?;
        let staging = staging_dir(&plan);
        let _ = fs::remove_dir_all(&staging);
        fs::create_dir_all(&staging)?;
        let mut deletes = Vec::new();
        for (relative, path) in &after {
            let bytes = fs::read(path)?;
            let unchanged = before
                .get(relative)
                .and_then(|original| fs::read(original).ok())
                .is_some_and(|original| original == bytes);
            if unchanged {
                continue;
            }
            let target = staging.join(relative);
            if let Some(parent) = target.parent() {
                fs::create_dir_all(parent)?;
            }
            fs::write(&target, &bytes)?;
        }
        for (relative, original) in &before {
            if !after.contains_key(relative) {
                let bytes = fs::read(original)?;
                deletes.push(CandidateFile {
                    path: relative.clone(),
                    candidate_hash: digest(&bytes),
                    base_hash: Some(digest(&bytes)),
                    bytes: 0,
                    operation: CandidateOperation::Delete,
                });
            }
        }
        let mut manifest = crate::candidate_manifest(environment_id, &staging, &plan.input_root)?;
        manifest.files.extend(deletes);
        manifest.files.sort_by(|a, b| a.path.cmp(&b.path));
        Ok(manifest)
    }
}

/// Freezes a candidate an environment exported (`source_root` its staged
/// changes) into `frozen_root`, keeping its delete operations, which a
/// frozen tree cannot hold as files.
pub fn freeze_exported(
    candidate: &CandidateManifest,
    frozen_root: &Path,
) -> Result<CandidateManifest, Error> {
    let mut frozen = crate::freeze_candidate(
        &candidate.candidate_id,
        &candidate.source_root,
        &candidate.destination_root,
        frozen_root,
    )?;
    frozen.files.extend(
        candidate
            .files
            .iter()
            .filter(|file| file.operation == CandidateOperation::Delete)
            .cloned(),
    );
    frozen.files.sort_by(|a, b| a.path.cmp(&b.path));
    Ok(frozen)
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    fn plan(input: &Path, task: &Path) -> EnvironmentPlan {
        EnvironmentPlan {
            id: "run-1".into(),
            outcome_revision: 1,
            input_root: input.to_path_buf(),
            task_root: task.to_path_buf(),
            backend: "copy".into(),
            image: None,
            network_policy: "none".into(),
            setup_recipe: Vec::new(),
        }
    }

    #[test]
    fn a_copy_skips_ignored_trees_and_exports_only_what_changed() {
        let space = tempfile::tempdir().unwrap();
        let envs = tempfile::tempdir().unwrap();
        fs::write(space.path().join("keep.txt"), "same").unwrap();
        fs::write(space.path().join("edit.txt"), "before").unwrap();
        fs::write(space.path().join("gone.txt"), "bye").unwrap();
        fs::create_dir_all(space.path().join("node_modules/x")).unwrap();
        fs::write(space.path().join("node_modules/x/big.js"), "dep").unwrap();
        fs::create_dir_all(space.path().join(".vak")).unwrap();
        fs::write(space.path().join(".vak/config.toml"), "x").unwrap();
        let copy = CopyEnvironment::default();
        let task = envs.path().join("run-1");
        copy.prepare(&plan(space.path(), &task)).unwrap();
        assert_eq!(copy.state("run-1"), EnvironmentState::Ready);
        assert!(task.join("keep.txt").is_file());
        assert!(!task.join("node_modules").exists());
        assert!(!task.join(".vak").exists());

        fs::write(task.join("edit.txt"), "after").unwrap();
        fs::write(task.join("new.txt"), "hello").unwrap();
        fs::remove_file(task.join("gone.txt")).unwrap();
        let candidate = copy.export_candidate("run-1").unwrap();
        let summary: Vec<(String, CandidateOperation)> = candidate
            .files
            .iter()
            .map(|file| (file.path.clone(), file.operation.clone()))
            .collect();
        assert_eq!(
            summary,
            vec![
                ("edit.txt".into(), CandidateOperation::Upsert),
                ("gone.txt".into(), CandidateOperation::Delete),
                ("new.txt".into(), CandidateOperation::Upsert),
            ]
        );
        // Nothing reached the folder by itself.
        assert_eq!(
            fs::read_to_string(space.path().join("edit.txt")).unwrap(),
            "before"
        );

        let frozen_root = envs.path().join("frozen");
        let frozen = freeze_exported(&candidate, &frozen_root).unwrap();
        assert_eq!(frozen.files.len(), 3);
        assert_eq!(frozen.destination_root, space.path());
        copy.remove("run-1").unwrap();
        assert!(!task.exists());
    }

    #[test]
    fn a_folder_over_the_caps_is_refused_and_leaves_no_copy() {
        let space = tempfile::tempdir().unwrap();
        let envs = tempfile::tempdir().unwrap();
        for index in 0..5 {
            fs::write(space.path().join(format!("f{index}")), "x").unwrap();
        }
        let copy = CopyEnvironment::new(CopyLimits {
            max_files: 3,
            max_bytes: 1024,
        });
        let task = envs.path().join("run-1");
        assert!(copy.prepare(&plan(space.path(), &task)).is_err());
        assert!(!task.exists());
        assert_eq!(copy.state("run-1"), EnvironmentState::Failed);
    }
}
