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
    DataClass::Trash,
    DataClass::Execution,
    DataClass::Checkpoint,
    DataClass::Environment,
    DataClass::Telemetry,
    DataClass::DraftVersion,
    DataClass::InboxEntry,
    DataClass::ActivitySegment,
    DataClass::Incident,
    DataClass::DocumentHistory,
];

/// The classes whose plan this build can carry out. Each is removed in
/// place with no grace: nothing else refers to an environment after its
/// run settled or to a rotated log (doc 74 §5).
pub const COMMITTED: &[DataClass] = &[
    DataClass::Trash,
    DataClass::Execution,
    DataClass::Checkpoint,
    DataClass::Environment,
    DataClass::Telemetry,
    DataClass::DraftVersion,
    DataClass::InboxEntry,
    DataClass::ActivitySegment,
    DataClass::Incident,
    DataClass::DocumentHistory,
];

const DAY_SECS: i64 = 86_400;
/// The longest a keep time may be set to, in days.
pub const MAX_KEEP_DAYS: i64 = 3650;

/// What the owner changed from the default rules: a keep time in days for
/// some kinds of data. Everything not named keeps its default.
#[derive(Debug, Default, Serialize, serde::Deserialize)]
struct StoredRules {
    keep_days: std::collections::BTreeMap<DataClass, i64>,
}

fn rules_path() -> PathBuf {
    vak_config::scope::SharedScope::new(vak_config::paths::data_home()).retention_rules()
}

fn stored_rules() -> StoredRules {
    vak_session::documents::read(&rules_path())
        .ok()
        .flatten()
        .and_then(|text| serde_json::from_str(&text).ok())
        .unwrap_or_default()
}

fn label_with(keep_days: &std::collections::BTreeMap<DataClass, i64>) -> Label {
    let mut label = Label::default_tenant();
    if keep_days.is_empty() {
        return label;
    }
    label.id = "install".into();
    label.name = "This install".into();
    for rule in &mut label.rules {
        if let Some(days) = keep_days.get(&rule.class) {
            rule.delete_after_secs = Some(days * DAY_SECS);
        }
    }
    label
}

/// The retention rules this install runs under: the defaults, with the
/// keep times the owner changed (plan M7b-a). There is one set for the
/// whole install.
pub fn retention_label() -> Label {
    label_with(&stored_rules().keep_days)
}

fn window(class: DataClass) -> chrono::Duration {
    chrono::Duration::seconds(
        retention_label()
            .rule(class)
            .and_then(|rule| rule.delete_after_secs)
            .unwrap_or(0),
    )
}

/// How long something stays in the trash before it is erased.
pub fn trash_window() -> chrono::Duration {
    window(DataClass::Trash)
}

/// How long a draft nobody accepted, saved, starred or shared is kept
/// before it goes to the trash.
pub fn draft_window() -> chrono::Duration {
    window(DataClass::DraftVersion)
}

#[derive(Debug, thiserror::Error)]
pub enum RulesError {
    #[error("{0:?} has no keep time to change")]
    NotEditable(DataClass),
    #[error("a keep time is between 1 and {MAX_KEEP_DAYS} days")]
    OutOfRange,
    #[error("a shorter keep time removes things sooner; look at what it would remove and confirm")]
    Confirm,
    #[error("the rules could not be saved: {0}")]
    Store(String),
}

/// What one kind of data would lose under proposed rules that it keeps
/// under the current ones.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ClassImpact {
    pub class: DataClass,
    pub items: u64,
    pub bytes: u64,
}

/// What changing the rules would do, before they are changed.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct RulesPreview {
    /// A confirmation of shortened rules carries this.
    pub digest: String,
    pub label: Label,
    /// The kinds whose keep time gets shorter.
    pub shortened: Vec<DataClass>,
    /// What the next pass would remove or move to the trash that it would
    /// not under the current rules.
    pub newly_due: Vec<ClassImpact>,
}

/// A Document's id in a lifecycle item: a digest, because its name is a
/// path and paths are content.
fn document_id(name: &str) -> String {
    use sha2::{Digest, Sha256};
    let digest = Sha256::digest(name.as_bytes());
    digest[..8]
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

/// One item per Document with earlier versions, aged from the oldest whose
/// time is known: it is due when at least one version is past the rule,
/// and carrying it out removes exactly those. The current version is not
/// history and is never counted.
fn document_items() -> Vec<Item> {
    let mut items = Vec::new();
    for name in vak_session::documents::names() {
        let times = vak_session::documents::history_times(&name);
        let Some(oldest) = times.iter().flatten().min().copied() else {
            continue;
        };
        let Some(since) = DateTime::<Utc>::from_timestamp(oldest, 0) else {
            continue;
        };
        items.push(Item {
            id: document_id(&name),
            class: DataClass::DocumentHistory,
            since,
            bytes: 0,
            files: times.len() as u64,
            guard: None,
            group: None,
            rank: 0,
        });
    }
    items
}

/// How long a grant must have been held before collection may release
/// it: longer than any save takes between writing its objects and moving
/// its Document's ref.
const COLLECT_GRACE: std::time::Duration = std::time::Duration::from_secs(60 * 60);

/// How long a chain's open segment takes rows before it is sealed, so
/// that the rows of a quiet chain age out no more than this late.
const SEAL_AFTER_DAYS: i64 = 30;

/// A record chain whose rows all belong to one retention class.
struct ExpiringChain {
    /// Names the chain in an item id: a kind, and the Agent it is of.
    id: String,
    class: DataClass,
    chain: vak_session::chain::RecordChain,
}

/// When a chain row was written: the first time field it carries.
fn row_time(row: &serde_json::Value) -> Option<DateTime<Utc>> {
    ["ts", "at", "updated_at", "opened_at", "created_at"]
        .iter()
        .find_map(|field| row.get(field)?.as_str()?.parse().ok())
}

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
    /// Grants released on objects no Document names any more, and objects
    /// deleted because no scope holds them.
    pub released_grants: u64,
    pub deleted_objects: u64,
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

/// Where an install stands against its quota (docs/design/74 §3.3).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum QuotaState {
    /// No limit is set.
    None,
    Ok,
    /// Past four fifths of the limit: a warning, nothing refused.
    Soft,
    /// At the limit: new work is refused. Nothing is removed to make room
    /// but what can be rebuilt.
    Hard,
}

/// The install's storage against its limit, as last measured.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub struct Quota {
    pub state: QuotaState,
    pub limit_bytes: Option<u64>,
    /// What cannot be rebuilt: records, objects, Documents, settings.
    pub kept_bytes: u64,
    /// What can: derived indexes, caches, telemetry, scratch.
    pub rebuildable_bytes: u64,
    pub measured_at: DateTime<Utc>,
}

/// What must be kept, what can be rebuilt, and when they were measured.
type Measured = (u64, u64, DateTime<Utc>);
type Measurements = std::sync::Mutex<std::collections::HashMap<PathBuf, Measured>>;

/// The last measurement of each data home this process has measured: what
/// the check before a turn reads, so that it never walks the files itself.
fn measurements() -> &'static Measurements {
    static MEASURED: std::sync::OnceLock<Measurements> = std::sync::OnceLock::new();
    MEASURED.get_or_init(Default::default)
}

const REBUILDABLE: &[&str] = &["derived", "ephemeral", "telemetry"];

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
    pub quota: Quota,
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

/// When anything under `path` was last written.
fn newest(path: &Path) -> Option<std::time::SystemTime> {
    let meta = std::fs::symlink_metadata(path).ok()?;
    let mut latest = meta.modified().ok();
    if meta.is_dir() {
        for entry in std::fs::read_dir(path).into_iter().flatten().flatten() {
            latest = latest.max(newest(&entry.path()));
        }
    }
    latest
}

/// The directories directly under `path`, by name.
fn child_dirs(path: &Path) -> Vec<(String, PathBuf)> {
    std::fs::read_dir(path)
        .into_iter()
        .flatten()
        .flatten()
        .filter(|entry| entry.path().is_dir())
        .filter_map(|entry| Some((entry.file_name().into_string().ok()?, entry.path())))
        .collect()
}

/// The parts of an item id made of plain names, or `None` when any part
/// is not one: an id never steps out of its class's directory.
fn id_parts(id: &str, count: usize) -> Option<Vec<&str>> {
    let parts: Vec<&str> = id.split('/').collect();
    let plain = |part: &&str| {
        !part.is_empty() && *part != "." && *part != ".." && !part.contains(['\\', '\0'])
    };
    (parts.len() == count && parts.iter().all(plain)).then_some(parts)
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

    /// Measures the data home and keeps the figure for `quota`. Called by
    /// each lifecycle pass and by the first check of a process.
    pub fn measure_storage(&self) -> Usage {
        let usage = self.data_usage();
        let rebuildable: u64 = usage
            .rows
            .iter()
            .filter(|row| REBUILDABLE.contains(&row.class.as_str()))
            .map(|row| row.bytes)
            .sum();
        if let Ok(mut measured) = measurements().lock() {
            measured.insert(
                self.inner.sessions_home.clone(),
                (usage.bytes - rebuildable, rebuildable, usage.at),
            );
        }
        usage
    }

    /// The install against its limit, from the last measurement (taken now
    /// when this process has none).
    pub fn quota(&self) -> Quota {
        let read = || {
            measurements()
                .lock()
                .ok()
                .and_then(|measured| measured.get(&self.inner.sessions_home).copied())
        };
        let (kept_bytes, rebuildable_bytes, measured_at) = read()
            .or_else(|| {
                self.measure_storage();
                read()
            })
            .unwrap_or((0, 0, Utc::now()));
        let limit_bytes = self.config().lifecycle.quota_bytes;
        let state = match limit_bytes {
            None => QuotaState::None,
            Some(limit) if kept_bytes >= limit => QuotaState::Hard,
            Some(limit) if kept_bytes + rebuildable_bytes >= limit / 5 * 4 => QuotaState::Soft,
            Some(_) => QuotaState::Ok,
        };
        Quota {
            state,
            limit_bytes,
            kept_bytes,
            rebuildable_bytes,
            measured_at,
        }
    }

    /// Refuses new work when what the install must keep has reached its
    /// limit. Nothing is removed to make room: a record is never evicted
    /// (docs/design/74 §3.3).
    pub(crate) fn refuse_over_quota(&self) -> Result<(), crate::CoreError> {
        let quota = self.quota();
        match (quota.state, quota.limit_bytes) {
            (QuotaState::Hard, Some(limit)) => Err(crate::CoreError::OverQuota {
                used_mb: quota.kept_bytes / (1024 * 1024),
                limit_mb: limit / (1024 * 1024),
            }),
            _ => Ok(()),
        }
    }

    /// Removes each Agent's tool cache: what a hard limit evicts, because
    /// a cache is rebuilt by the next command that wants it. Returns the
    /// transitions made.
    fn evict_caches(&self, chain: &vak_session::chain::RecordChain) -> Vec<Transition> {
        let mut made = Vec::new();
        for (space, space_dir) in child_dirs(&executions_root()) {
            for (agent, agent_dir) in child_dirs(&space_dir) {
                let path = agent_dir.join("cache");
                let (files, bytes) = measure(&path, &mut HashSet::new());
                if files == 0 {
                    continue;
                }
                let item = format!("{space}/{agent}/cache");
                let mut row = Transition {
                    at: Utc::now(),
                    key: format!("evict:{item}"),
                    class: DataClass::Execution,
                    item,
                    does: vak_lifecycle::OnExpiry::Remove,
                    reason: vak_lifecycle::Reason::Size,
                    bytes,
                    files,
                    state: TransitionState::Started,
                    error_kind: None,
                };
                if chain.append(&row).is_err() {
                    continue;
                }
                row.at = Utc::now();
                match std::fs::remove_dir_all(&path) {
                    Ok(()) => row.state = TransitionState::Committed,
                    Err(error) => {
                        row.state = TransitionState::Failed;
                        row.error_kind = Some(vak_telemetry::error_kind(&error).to_string());
                    }
                }
                let _ = chain.append(&row);
                made.push(row);
            }
        }
        made
    }

    /// Everything the reconciler observes, as items a label is applied to.
    pub fn lifecycle_items(&self) -> Vec<Item> {
        let mut items = execution_items();
        items.extend(self.checkpoint_items());
        items.extend(self.environment_items());
        items.extend(telemetry_items());
        items.extend(self.draft_items());
        items.extend(self.chain_items());
        items.extend(document_items());
        items.extend(self.trash_items());
        items
    }

    /// A conversation in the trash: its window started when it went in,
    /// and when it ends the conversation is erased (doc 74 §2.4).
    fn trash_items(&self) -> Vec<Item> {
        let mut items = self.trashed_conversations();
        let objects = self.tenant_objects().ok();
        for artifact in self.artifacts().list() {
            let held = objects.as_ref().is_some_and(|objects| {
                objects.scope_held(&crate::artifacts::object_scope(&artifact.id))
            });
            for version in &artifact.versions {
                let Some(since) = version.trashed_at.filter(|_| !version.erased) else {
                    continue;
                };
                items.push(Item {
                    id: format!("draft/{}/{}", artifact.id, version.id),
                    class: DataClass::Trash,
                    since,
                    bytes: version.size,
                    files: 1,
                    guard: held.then_some(Guard::Held),
                    group: None,
                    rank: 0,
                });
            }
        }
        items
    }

    fn trashed_conversations(&self) -> Vec<Item> {
        crate::trash::states(&self.shared_scope())
            .into_iter()
            .filter(|(_, state)| state.erased_at.is_none())
            .filter_map(|(id, state)| {
                let held = self.conversation_held(&id);
                Some(Item {
                    id,
                    class: DataClass::Trash,
                    since: state.trashed_at?,
                    bytes: 0,
                    files: 0,
                    guard: held.then_some(Guard::Held),
                    group: None,
                    rank: 0,
                })
            })
            .collect()
    }

    /// The chains whose rows age out a sealed segment at a time: each
    /// Agent's inbox and its cost, routing and intent evidence, and the
    /// Operations Center's incidents and action receipts.
    fn expiring_chains(&self) -> Vec<ExpiringChain> {
        let chain = |id: String, class, dir: PathBuf| ExpiringChain {
            id,
            class,
            chain: vak_session::chain::RecordChain::at(dir),
        };
        let mut chains = Vec::new();
        for (agent, scope) in self.agent_scopes() {
            chains.push(chain(
                format!("inbox:{agent}"),
                DataClass::InboxEntry,
                scope.inbox(),
            ));
            for (kind, dir) in [
                ("cost", scope.cost_log()),
                ("routing", scope.routing_evidence()),
                ("intent", scope.intent_evidence()),
            ] {
                chains.push(chain(
                    format!("{kind}:{agent}"),
                    DataClass::ActivitySegment,
                    dir,
                ));
            }
        }
        let shared = self.shared_scope();
        for (kind, dir) in [
            ("incidents", shared.operations_incidents()),
            ("actions", shared.operations_actions()),
        ] {
            chains.push(chain(format!("{kind}:shared"), DataClass::Incident, dir));
        }
        chains
    }

    /// One item per sealed segment but a chain's newest: its clock starts
    /// at its newest row, so it is due only when every row in it is. A
    /// segment with a row that carries no time is never planned away.
    fn chain_items(&self) -> Vec<Item> {
        let mut items = Vec::new();
        for expiring in self.expiring_chains() {
            let segments = expiring.chain.segments();
            let newest_sealed = segments
                .iter()
                .filter(|segment| !segment.open)
                .map(|segment| segment.number)
                .max();
            for segment in segments {
                if segment.open || Some(segment.number) == newest_sealed {
                    continue;
                }
                let times: Option<Vec<DateTime<Utc>>> = segment.rows.iter().map(row_time).collect();
                let Some(since) = times.and_then(|times| times.into_iter().max()) else {
                    continue;
                };
                items.push(Item {
                    id: format!("{}/{}", expiring.id, segment.number),
                    class: expiring.class,
                    since,
                    bytes: segment.bytes,
                    files: 1,
                    guard: None,
                    group: None,
                    rank: 0,
                });
            }
        }
        items
    }

    /// Seals each expiring chain's open segment once its first row is
    /// `SEAL_AFTER_DAYS` old. A seal removes nothing; it lets the rows age
    /// out together. Returns how many were sealed.
    fn seal_aged_segments(&self) -> u64 {
        let cutoff = Utc::now() - chrono::Duration::days(SEAL_AFTER_DAYS);
        let mut sealed = 0;
        for expiring in self.expiring_chains() {
            let aged = expiring
                .chain
                .segments()
                .last()
                .filter(|segment| segment.open)
                .and_then(|segment| segment.rows.first().and_then(row_time))
                .is_some_and(|first| first <= cutoff);
            if aged && expiring.chain.seal_open().unwrap_or(false) {
                sealed += 1;
            }
        }
        sealed
    }

    /// Every Agent home in this data home, by Agent id.
    fn agent_scopes(&self) -> Vec<(String, vak_config::scope::AgentScope)> {
        child_dirs(&self.shared_scope().agents_dir())
            .into_iter()
            .map(|(agent, home)| (agent, vak_config::scope::AgentScope::new(home)))
            .collect()
    }

    /// A session's checkpoints, grouped by session so the label's count
    /// applies to each. Their clock starts at the session's newest one: a
    /// session still making checkpoints keeps them.
    fn checkpoint_items(&self) -> Vec<Item> {
        let mut items = Vec::new();
        for (agent, scope) in self.agent_scopes() {
            for session in crate::checkpoints::sessions(&scope) {
                let Ok(manifests) = crate::checkpoints::list(&scope, &session) else {
                    continue;
                };
                let Some(since) = manifests.iter().map(|manifest| manifest.created_at).max() else {
                    continue;
                };
                for manifest in &manifests {
                    items.push(Item {
                        id: format!("{agent}/{session}/{}", manifest.seq),
                        class: DataClass::Checkpoint,
                        since,
                        bytes: manifest.files.iter().map(|file| file.size).sum(),
                        files: 1,
                        guard: None,
                        group: Some(format!("{agent}/{session}")),
                        rank: u64::from(manifest.seq),
                    });
                }
            }
        }
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
                group: None,
                rank: 0,
            });
        }
        items
    }

    /// A version nobody accepted or saved is a draft, aged from when it was
    /// made or last restored from the trash. One on a starred or shared
    /// artifact is kept; one already in the trash is the trash's.
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
                if !version.is_draft() || !version.is_present() {
                    continue;
                }
                items.push(Item {
                    id: format!("{}/{}", artifact.id, version.id),
                    class: DataClass::DraftVersion,
                    since: version.restored_at.unwrap_or(version.at),
                    bytes: version.size,
                    files: 1,
                    guard: kept.then_some(Guard::Kept),
                    group: None,
                    rank: 0,
                });
            }
        }
        items
    }

    /// What setting the keep times to `keep_days` would do. Kinds not
    /// named go back to their default.
    pub fn retention_preview(
        &self,
        keep_days: &std::collections::BTreeMap<DataClass, i64>,
    ) -> Result<RulesPreview, RulesError> {
        let defaults = Label::default_tenant();
        for (class, days) in keep_days {
            let editable = defaults
                .rule(*class)
                .is_some_and(|rule| rule.delete_after_secs.is_some());
            if !editable {
                return Err(RulesError::NotEditable(*class));
            }
            if !(1..=MAX_KEEP_DAYS).contains(days) {
                return Err(RulesError::OutOfRange);
            }
        }
        let current = retention_label();
        let label = label_with(keep_days);
        let shortened: Vec<DataClass> = label
            .rules
            .iter()
            .filter(|rule| {
                current
                    .rule(rule.class)
                    .is_some_and(|now| rule.delete_after_secs < now.delete_after_secs)
            })
            .map(|rule| rule.class)
            .collect();
        let now = Utc::now();
        let items = self.lifecycle_items();
        let due_now: std::collections::BTreeSet<String> =
            vak_lifecycle::plan(&items, OBSERVED, &current, now)
                .actions
                .into_iter()
                .map(|action| action.key)
                .collect();
        let mut newly: std::collections::BTreeMap<DataClass, ClassImpact> =
            std::collections::BTreeMap::new();
        let mut digest = {
            use sha2::Digest;
            sha2::Sha256::new()
        };
        {
            use sha2::Digest;
            digest.update(serde_json::to_vec(keep_days).unwrap_or_default());
        }
        for action in vak_lifecycle::plan(&items, OBSERVED, &label, now).actions {
            if due_now.contains(&action.key) {
                continue;
            }
            {
                use sha2::Digest;
                digest.update(action.key.as_bytes());
                digest.update([0]);
            }
            let impact = newly.entry(action.class).or_insert(ClassImpact {
                class: action.class,
                items: 0,
                bytes: 0,
            });
            impact.items += 1;
            impact.bytes += action.bytes;
        }
        let digest = {
            use sha2::Digest;
            digest
                .finalize()
                .iter()
                .map(|byte| format!("{byte:02x}"))
                .collect()
        };
        Ok(RulesPreview {
            digest,
            label,
            shortened,
            newly_due: newly.into_values().collect(),
        })
    }

    /// Sets the install's keep times. Shortening any of them needs the
    /// digest of the preview the owner looked at, and is refused when what
    /// it would remove has changed since. Lengthening needs nothing.
    pub fn set_retention(
        &self,
        keep_days: &std::collections::BTreeMap<DataClass, i64>,
        digest: Option<&str>,
    ) -> Result<Label, RulesError> {
        let preview = self.retention_preview(keep_days)?;
        if !preview.shortened.is_empty() && digest != Some(preview.digest.as_str()) {
            return Err(RulesError::Confirm);
        }
        let stored = serde_json::to_string(&StoredRules {
            keep_days: keep_days.clone(),
        })
        .map_err(|error| RulesError::Store(error.to_string()))?;
        vak_session::documents::update(&rules_path(), |_| Ok(Some((stored.clone(), ()))))
            .map_err(RulesError::Store)?;
        tracing::info!(
            kind = "retention_rules",
            outcome = "changed",
            count = preview.shortened.len(),
            "the install's retention rules were changed"
        );
        Ok(preview.label)
    }

    /// The plan the reconciler would run now under the default label. It
    /// is computed and shown; nothing is committed.
    pub fn lifecycle_plan(&self) -> Plan {
        self.lifecycle_plan_at(Utc::now())
    }

    /// The plan as it would be at `now`.
    pub fn lifecycle_plan_at(&self, now: DateTime<Utc>) -> Plan {
        vak_lifecycle::plan(&self.lifecycle_items(), OBSERVED, &retention_label(), now)
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

    /// Carries out one due action, or `None` when its class is not one
    /// this build commits or its id is not one an observer would make.
    /// An item already gone is done.
    fn carry_out(&self, action: &vak_lifecycle::Action) -> Option<std::io::Result<()>> {
        let remove = |path: PathBuf| match std::fs::symlink_metadata(&path) {
            Ok(meta) if meta.is_dir() => std::fs::remove_dir_all(&path),
            Ok(_) => std::fs::remove_file(&path),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(error) => Err(error),
        };
        match action.class {
            DataClass::Environment => {
                let name = id_parts(&action.item, 1)?[0];
                Some(remove(
                    vak_config::paths::tenant_home_at(
                        &self.inner.sessions_home,
                        vak_config::paths::LOCAL_TENANT,
                    )
                    .join("environments")
                    .join(name),
                ))
            }
            DataClass::Telemetry => {
                let name = id_parts(&action.item, 1)?[0];
                Some(remove(vak_config::paths::logs_dir().join(name)))
            }
            DataClass::Execution => {
                let parts = id_parts(&action.item, 3)?;
                let mut path = executions_root();
                path.extend(parts);
                Some(remove(path))
            }
            DataClass::Checkpoint => {
                let parts = id_parts(&action.item, 3)?;
                let seq: u32 = parts[2].parse().ok()?;
                let scope = vak_config::scope::AgentScope::new(
                    self.shared_scope().agents_dir().join(parts[0]),
                );
                let objects = match self.objects() {
                    Ok(objects) => objects,
                    Err(error) => return Some(Err(std::io::Error::other(error.to_string()))),
                };
                Some(
                    crate::checkpoints::remove(&scope, objects.as_ref(), parts[1], seq).map(|_| ()),
                )
            }
            DataClass::InboxEntry | DataClass::ActivitySegment | DataClass::Incident => {
                let (chain_id, number) = action.item.rsplit_once('/')?;
                let number: u64 = number.parse().ok()?;
                let expiring = self
                    .expiring_chains()
                    .into_iter()
                    .find(|chain| chain.id == chain_id && chain.class == action.class)?;
                Some(
                    expiring
                        .chain
                        .drop_segment(number)
                        .map(|_| ())
                        .map_err(|error| std::io::Error::other(error.to_string())),
                )
            }
            // An expired draft goes to the trash, whole and restorable.
            DataClass::DraftVersion => {
                let parts = id_parts(&action.item, 2)?;
                let artifact = vak_session::ids::ArtifactId::parse(parts[0]).ok()?;
                let version = vak_session::ids::VersionId::parse(parts[1]).ok()?;
                Some(
                    self.artifacts()
                        .trash_version(artifact, version, true, None)
                        .map_err(|error| std::io::Error::other(error.to_string())),
                )
            }
            // The end of the trash window is an erasure, with its receipt.
            DataClass::Trash if action.item.starts_with("draft/") => {
                let parts = id_parts(&action.item, 3)?;
                Some(
                    self.erase_draft(parts[1], parts[2], crate::erasure::Cause::Policy, None)
                        .map(|_| ())
                        .map_err(|error| std::io::Error::other(error.to_string())),
                )
            }
            DataClass::Trash => Some(
                self.erase_conversation(&action.item, None, crate::erasure::Cause::Policy, None)
                    .map(|_| ())
                    .map_err(|error| std::io::Error::other(error.to_string())),
            ),
            DataClass::DocumentHistory => {
                let name = vak_session::documents::names()
                    .into_iter()
                    .find(|name| document_id(name) == action.item)?;
                let keep = retention_label()
                    .rule(DataClass::DocumentHistory)?
                    .delete_after_secs?;
                Some(
                    vak_session::documents::prune_history(&name, Utc::now().timestamp() - keep)
                        .map(|_| ())
                        .map_err(std::io::Error::other),
                )
            }
        }
    }

    /// One pass: observe, plan and, when `commit`, carry out the due
    /// actions of the classes in `COMMITTED`. Each is recorded before it
    /// is touched and again when it is done; a fenced process and an
    /// observing one commit nothing.
    pub fn lifecycle_tick(&self, commit: bool) -> Tick {
        self.lifecycle_tick_at(commit, Utc::now())
    }

    /// One pass over the plan as it would be at `now`.
    pub fn lifecycle_tick_at(&self, commit: bool, now: DateTime<Utc>) -> Tick {
        let commit = commit && vak_session::fence::check().is_ok();
        if commit {
            self.seal_aged_segments();
        }
        let plan = self.lifecycle_plan_at(now);
        let mut tick = Tick {
            mode: if commit { "commit" } else { "observe" },
            committed: Vec::new(),
            failed: Vec::new(),
            left: 0,
            reclaimed_bytes: 0,
            released_grants: 0,
            deleted_objects: 0,
            plan,
        };
        let chain = self.lifecycle_chain();
        for action in &tick.plan.actions {
            if !commit || !COMMITTED.contains(&action.class) {
                tick.left += 1;
                continue;
            }
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
            let removed = self.carry_out(action).unwrap_or_else(|| {
                Err(std::io::Error::new(
                    std::io::ErrorKind::InvalidInput,
                    "the item's id is not one this class makes",
                ))
            });
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
        // At the limit, what can be rebuilt goes first; records never do.
        self.measure_storage();
        if commit && self.quota().state == QuotaState::Hard {
            for row in self.evict_caches(&chain) {
                if row.state == TransitionState::Committed {
                    tick.reclaimed_bytes += row.bytes;
                    tick.committed.push(row);
                } else {
                    tick.failed.push(row);
                }
            }
        }
        if commit {
            match vak_session::documents::collect(COLLECT_GRACE) {
                Ok((released, deleted)) => {
                    tick.released_grants = released as u64;
                    tick.deleted_objects = deleted as u64;
                }
                Err(error) => tracing::warn!(
                    kind = "lifecycle",
                    error_kind = %vak_telemetry::error_kind(&std::io::Error::other(error)),
                    "objects were not collected this pass"
                ),
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
        let usage = self.measure_storage();
        Status {
            quota: self.quota(),
            at: plan.at,
            mode: if self.config().lifecycle.commit {
                "commit"
            } else {
                "observe"
            },
            committed: COMMITTED.to_vec(),
            label: retention_label(),
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

fn executions_root() -> PathBuf {
    vak_config::paths::runtime_dir().join("executions")
}

/// An execution's directory in the runtime root: its temp files and the
/// drafts it wrote. Its clock starts at the last write anywhere in it; a
/// draft put up for Review is also a frozen candidate, which is not here.
/// An Agent's tool cache beside its executions is not an execution.
fn execution_items() -> Vec<Item> {
    let mut items = Vec::new();
    for (space, space_dir) in child_dirs(&executions_root()) {
        for (agent, agent_dir) in child_dirs(&space_dir) {
            for (execution, path) in child_dirs(&agent_dir) {
                if execution == "cache" {
                    continue;
                }
                let Some(since) = newest(&path).map(DateTime::<Utc>::from) else {
                    continue;
                };
                let (files, bytes) = measure(&path, &mut HashSet::new());
                items.push(Item {
                    id: format!("{space}/{agent}/{execution}"),
                    class: DataClass::Execution,
                    since,
                    bytes,
                    files,
                    guard: None,
                    group: None,
                    rank: 0,
                });
            }
        }
    }
    items
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
            group: None,
            rank: 0,
        });
    }
    items
}
