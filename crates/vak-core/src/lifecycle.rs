//! Observing a data home for the lifecycle reconciler (plan M7a-c,
//! docs/design/74-lifecycle-and-data-administration.md §5). This module
//! reads; `vak_lifecycle` decides. Nothing here writes, moves or removes
//! anything: the reconciler is observe-only until each class's commit is
//! enabled (plan M7a-d onward), and what it shows is the plan it would run.

use crate::Core;
use crate::state::{self, Root};
use chrono::{DateTime, Utc};
use serde::Serialize;
use std::collections::{BTreeMap, HashSet};
use std::path::{Path, PathBuf};
use vak_lifecycle::{DataClass, Guard, Item, Label, Plan};

/// The classes this build observes. A class with a rule and no observer is
/// reported in `Plan::unobserved`, never assumed empty.
pub const OBSERVED: &[DataClass] = &[
    DataClass::Environment,
    DataClass::Telemetry,
    DataClass::DraftVersion,
];

/// The classes whose plan this build can carry out. Each is removed in
/// place with no grace: nothing else refers to an environment after its
/// run settled or to a rotated log (doc 74 §5).
pub const COMMITTED: &[DataClass] = &[DataClass::Environment, DataClass::Telemetry];

/// How far one planned transition got.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TransitionState {
    /// Written before anything is touched, so a crash leaves a trace.
    Started,
    Committed,
    Failed,
}

/// One row of the `lifecycle/` chain: a transition the reconciler began,
/// finished or could not make. It names an item by id and holds no content.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, serde::Deserialize)]
pub struct Transition {
    pub at: DateTime<Utc>,
    pub key: String,
    pub class: DataClass,
    pub item: String,
    pub does: vak_lifecycle::OnExpiry,
    pub reason: vak_lifecycle::Reason,
    pub bytes: u64,
    pub files: u64,
    pub state: TransitionState,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error_kind: Option<String>,
}

/// What one pass of the reconciler did.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Tick {
    /// `observe` or `commit`.
    pub mode: &'static str,
    pub plan: Plan,
    pub committed: Vec<Transition>,
    pub failed: Vec<Transition>,
    /// Due actions of classes this build cannot carry out yet.
    pub left: u64,
    pub reclaimed_bytes: u64,
}

/// What one declared place in the data home holds.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct UsageRow {
    pub root: String,
    pub class: String,
    pub owner: String,
    pub files: u64,
    pub bytes: u64,
}

/// Measured storage: every declared path, counted once.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Usage {
    pub at: DateTime<Utc>,
    pub rows: Vec<UsageRow>,
    pub files: u64,
    pub bytes: u64,
}

/// The reconciler as a person asks about it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Status {
    pub at: DateTime<Utc>,
    /// `observe`: plans are shown and nothing is removed. `commit`: the
    /// classes in `committed` are also carried out (`[lifecycle] mode`).
    pub mode: &'static str,
    pub label: Label,
    /// The classes a committing pass carries out.
    pub committed: Vec<DataClass>,
    pub observed: Vec<DataClass>,
    pub unobserved: Vec<DataClass>,
    pub due: u64,
    pub guarded: u64,
    pub reclaimable_bytes: u64,
    pub files: u64,
    pub bytes: u64,
}

/// Files and bytes under `path`, following no link. `seen` keeps a file
/// from being counted under two declarations.
fn measure(path: &Path, seen: &mut HashSet<PathBuf>) -> (u64, u64) {
    let Ok(meta) = std::fs::symlink_metadata(path) else {
        return (0, 0);
    };
    if meta.is_file() {
        return if seen.insert(path.to_path_buf()) {
            (1, meta.len())
        } else {
            (0, 0)
        };
    }
    if !meta.is_dir() {
        return (0, 0);
    }
    let mut total = (0, 0);
    for entry in std::fs::read_dir(path).into_iter().flatten().flatten() {
        let (files, bytes) = measure(&entry.path(), seen);
        total.0 += files;
        total.1 += bytes;
    }
    total
}

fn modified(path: &Path) -> Option<DateTime<Utc>> {
    std::fs::symlink_metadata(path)
        .ok()?
        .modified()
        .ok()
        .map(DateTime::<Utc>::from)
}

impl Core {
    /// What every declared path of this data home holds, by root, class and
    /// owner (`state::REGISTRY`). Measured now; nothing is estimated.
    pub fn data_usage(&self) -> Usage {
        let mut seen = HashSet::new();
        let mut rows: BTreeMap<(String, String, String), (u64, u64)> = BTreeMap::new();
        for root in Root::ALL {
            let base = state::root_path(root);
            for entry in state::entries_for(root) {
                let mut held = (0, 0);
                for relative in entry.expand(&base) {
                    let (files, bytes) = measure(&state::resolve(&base, &relative), &mut seen);
                    held.0 += files;
                    held.1 += bytes;
                }
                let key = (
                    format!("{root:?}").to_lowercase(),
                    format!("{:?}", entry.class).to_lowercase(),
                    entry.owner.to_string(),
                );
                let row = rows.entry(key).or_default();
                row.0 += held.0;
                row.1 += held.1;
            }
        }
        let rows: Vec<UsageRow> = rows
            .into_iter()
            .filter(|(_, held)| held.0 > 0)
            .map(|((root, class, owner), (files, bytes))| UsageRow {
                root,
                class,
                owner,
                files,
                bytes,
            })
            .collect();
        Usage {
            at: Utc::now(),
            files: rows.iter().map(|row| row.files).sum(),
            bytes: rows.iter().map(|row| row.bytes).sum(),
            rows,
        }
    }

    /// Everything the reconciler observes, as items a label is applied to.
    pub fn lifecycle_items(&self) -> Vec<Item> {
        let mut items = self.environment_items();
        items.extend(telemetry_items());
        items.extend(self.draft_items());
        items
    }

    /// A run's environment: its clock starts when the run settles, and one
    /// whose run is still going is live. One with no run record is an
    /// orphan, aged from when it was last written.
    fn environment_items(&self) -> Vec<Item> {
        let root = vak_config::paths::tenant_home_at(
            &self.inner.sessions_home,
            vak_config::paths::LOCAL_TENANT,
        )
        .join("environments");
        let runs: BTreeMap<String, vak_session::runs::RunRecord> = self
            .runs()
            .list()
            .unwrap_or_default()
            .into_iter()
            .map(|run| (run.id.to_string(), run))
            .collect();
        let mut items = Vec::new();
        for entry in std::fs::read_dir(&root).into_iter().flatten().flatten() {
            let path = entry.path();
            let Some(name) = path.file_name().and_then(|name| name.to_str()) else {
                continue;
            };
            let run = runs.get(name);
            let since = run
                .and_then(|run| run.settled_at)
                .or_else(|| modified(&path));
            let Some(since) = since else {
                continue;
            };
            let (files, bytes) = measure(&path, &mut HashSet::new());
            let live = run.is_some_and(|run| run.settled_at.is_none());
            items.push(Item {
                id: name.to_string(),
                class: DataClass::Environment,
                since,
                bytes,
                files,
                guard: live.then_some(Guard::Live),
            });
        }
        items
    }

    /// A version nobody accepted or saved is a draft. One on a starred or
    /// shared artifact is kept.
    fn draft_items(&self) -> Vec<Item> {
        let grants = crate::grants::Grants::at(&self.shared_scope());
        let now = Utc::now();
        let mut items = Vec::new();
        for artifact in self.artifacts().list() {
            let shared = grants
                .on(&crate::grants::GrantObject::Artifact(artifact.id))
                .ok()
                .is_some_and(|held| {
                    held.iter()
                        .any(|held| held.status(now) == crate::grants::GrantStatus::Active)
                });
            let kept = artifact.starred || shared;
            for version in &artifact.versions {
                if version.promoted || version.saved || version.removed || version.object.is_none()
                {
                    continue;
                }
                items.push(Item {
                    id: version.id.to_string(),
                    class: DataClass::DraftVersion,
                    since: version.at,
                    bytes: version.size,
                    files: 1,
                    guard: kept.then_some(Guard::Kept),
                });
            }
        }
        items
    }

    /// The plan the reconciler would run now under the default label. It
    /// is computed and shown; nothing is committed.
    pub fn lifecycle_plan(&self) -> Plan {
        vak_lifecycle::plan(
            &self.lifecycle_items(),
            OBSERVED,
            &Label::default_tenant(),
            Utc::now(),
        )
    }

    fn lifecycle_chain(&self) -> vak_session::chain::RecordChain {
        vak_session::chain::RecordChain::at(self.shared_scope().lifecycle())
    }

    /// The newest `limit` transitions, newest first.
    pub fn lifecycle_transitions(&self, limit: usize) -> Vec<Transition> {
        let mut rows: Vec<Transition> = self.lifecycle_chain().read();
        rows.reverse();
        rows.truncate(limit);
        rows
    }

    /// Where the item of a committable action is, or `None` when its id is
    /// not a single name inside the class's own directory.
    fn committable_path(&self, action: &vak_lifecycle::Action) -> Option<PathBuf> {
        let name = Path::new(&action.item);
        if name.components().count() != 1 || name.file_name()?.to_str()? != action.item {
            return None;
        }
        match action.class {
            DataClass::Environment => Some(
                vak_config::paths::tenant_home_at(
                    &self.inner.sessions_home,
                    vak_config::paths::LOCAL_TENANT,
                )
                .join("environments")
                .join(name),
            ),
            DataClass::Telemetry => Some(vak_config::paths::logs_dir().join(name)),
            _ => None,
        }
    }

    /// One pass: observe, plan and, when `commit`, carry out the due
    /// actions of the classes in `COMMITTED`. Each is recorded before it
    /// is touched and again when it is done; a fenced process and an
    /// observing one commit nothing.
    pub fn lifecycle_tick(&self, commit: bool) -> Tick {
        let plan = self.lifecycle_plan();
        let commit = commit && vak_session::fence::check().is_ok();
        let mut tick = Tick {
            mode: if commit { "commit" } else { "observe" },
            committed: Vec::new(),
            failed: Vec::new(),
            left: 0,
            reclaimed_bytes: 0,
            plan,
        };
        let chain = self.lifecycle_chain();
        for action in &tick.plan.actions {
            let path = COMMITTED
                .contains(&action.class)
                .then(|| self.committable_path(action))
                .flatten();
            let (true, Some(path)) = (commit, path) else {
                tick.left += 1;
                continue;
            };
            let mut row = Transition {
                at: Utc::now(),
                key: action.key.clone(),
                class: action.class,
                item: action.item.clone(),
                does: action.does,
                reason: action.reason,
                bytes: action.bytes,
                files: action.files,
                state: TransitionState::Started,
                error_kind: None,
            };
            if chain.append(&row).is_err() {
                // A transition that cannot be recorded is not made.
                tick.left += 1;
                continue;
            }
            let removed = match std::fs::symlink_metadata(&path) {
                Ok(meta) if meta.is_dir() => std::fs::remove_dir_all(&path),
                Ok(_) => std::fs::remove_file(&path),
                // Already gone: a pass that stopped after removing it.
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
                Err(error) => Err(error),
            };
            row.at = Utc::now();
            match removed {
                Ok(()) => {
                    row.state = TransitionState::Committed;
                    tick.reclaimed_bytes += row.bytes;
                    let _ = chain.append(&row);
                    tick.committed.push(row);
                }
                Err(error) => {
                    row.state = TransitionState::Failed;
                    row.error_kind = Some(vak_telemetry::error_kind(&error).to_string());
                    let _ = chain.append(&row);
                    tick.failed.push(row);
                }
            }
        }
        tracing::info!(
            kind = "lifecycle",
            outcome = tick.mode,
            count = tick.committed.len() as u64,
            bytes = tick.reclaimed_bytes,
            "a lifecycle pass finished"
        );
        tick
    }

    /// The reconciler's state and what the data home holds.
    pub fn data_status(&self) -> Status {
        let plan = self.lifecycle_plan();
        let usage = self.data_usage();
        Status {
            at: plan.at,
            mode: if self.config().lifecycle.commit {
                "commit"
            } else {
                "observe"
            },
            committed: COMMITTED.to_vec(),
            label: Label::default_tenant(),
            observed: OBSERVED.to_vec(),
            unobserved: plan.unobserved.clone(),
            due: plan.actions.len() as u64,
            guarded: plan.guarded.len() as u64,
            reclaimable_bytes: plan.reclaimable_bytes,
            files: usage.files,
            bytes: usage.bytes,
        }
    }
}

/// A service's log files. The file a service is writing has no number
/// after it and is live; its rotated copies age from their last write.
fn telemetry_items() -> Vec<Item> {
    let dir = vak_config::paths::logs_dir();
    let mut items = Vec::new();
    for entry in std::fs::read_dir(&dir).into_iter().flatten().flatten() {
        let path = entry.path();
        let (Some(name), Ok(meta)) = (
            path.file_name().and_then(|name| name.to_str()),
            std::fs::symlink_metadata(&path),
        ) else {
            continue;
        };
        let Some(since) = modified(&path).filter(|_| meta.is_file()) else {
            continue;
        };
        items.push(Item {
            id: name.to_string(),
            class: DataClass::Telemetry,
            since,
            bytes: meta.len(),
            files: 1,
            guard: name.ends_with(".jsonl").then_some(Guard::Live),
        });
    }
    items
}
