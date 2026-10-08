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
    /// `observe`: plans are shown and nothing is committed.
    pub mode: &'static str,
    pub label: Label,
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

    /// The reconciler's state and what the data home holds.
    pub fn data_status(&self) -> Status {
        let plan = self.lifecycle_plan();
        let usage = self.data_usage();
        Status {
            at: plan.at,
            mode: "observe",
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
