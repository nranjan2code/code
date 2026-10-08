//! The records the catalog reads, found under a data home: every session
//! ledger, the shared `runs/` and `effects/` chains, each Agent's
//! commitments chain, the trigger, memory and intake source Documents, the
//! `intake/` chain, and the `lifecycle/` and `erasures/` chains.

use std::path::{Path, PathBuf};
use vak_session::tail::{self, Position};

/// One source of rows, with its own cursor.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Source {
    /// A session ledger directory.
    Session(PathBuf),
    /// The run records.
    Runs(PathBuf),
    /// The effect records.
    Effects(PathBuf),
    /// One Agent's commitments chain.
    Commitments { dir: PathBuf, agent: String },
    /// A trigger Document; its cursor counts versions.
    Trigger(PathBuf),
    /// A memory Document of `agent`; its cursor counts versions.
    Memory { path: PathBuf, agent: String },
    /// An entity Document of `agent`; its cursor counts versions.
    Entity { path: PathBuf, agent: String },
    /// An intake source Document; its cursor counts versions.
    IntakeSource(PathBuf),
    /// The intake chain, with the tenant whose objects hold item bodies.
    Intake { dir: PathBuf, tenant: PathBuf },
    /// The artifact records.
    Artifacts(PathBuf),
    /// The grant records.
    Grants(PathBuf),
    /// The reconciler's transitions.
    Lifecycle(PathBuf),
    /// The erasure receipts.
    Erasures(PathBuf),
}

impl Source {
    /// The cursor's key: the kind and the path.
    pub fn key(&self) -> String {
        let (kind, path) = match self {
            Self::Session(path) => ("session", path),
            Self::Runs(path) => ("runs", path),
            Self::Effects(path) => ("effects", path),
            Self::Commitments { dir, .. } => ("commitments", dir),
            Self::Trigger(path) => ("trigger", path),
            Self::Memory { path, .. } => ("memory", path),
            Self::Entity { path, .. } => ("entity", path),
            Self::IntakeSource(path) => ("source", path),
            Self::Intake { dir, .. } => ("intake", dir),
            Self::Artifacts(dir) => ("artifacts", dir),
            Self::Grants(dir) => ("grants", dir),
            Self::Lifecycle(dir) => ("lifecycle", dir),
            Self::Erasures(dir) => ("erasures", dir),
        };
        format!("{kind}:{}", path.display())
    }

    /// Whether rows lie past `from`.
    pub fn has_more(&self, from: Position) -> bool {
        match self {
            Self::Session(dir) | Self::Runs(dir) | Self::Effects(dir) => chain_has_more(dir, from),
            Self::Commitments { dir, .. }
            | Self::Intake { dir, .. }
            | Self::Artifacts(dir)
            | Self::Lifecycle(dir)
            | Self::Erasures(dir)
            | Self::Grants(dir) => chain_has_more(dir, from),
            Self::Trigger(path)
            | Self::IntakeSource(path)
            | Self::Memory { path, .. }
            | Self::Entity { path, .. } => {
                vak_session::documents::version_count(path) as u64 > from.frames
            }
        }
    }
}

fn chain_has_more(dir: &Path, from: Position) -> bool {
    let mut more = false;
    tail::tail(dir, from, |_, _| {
        more = true;
        false
    });
    more
}

/// The Agent homes under a data home: the data home itself (the built-in
/// Agent's), then each `agents/<id>/`.
fn agent_roots(data: &Path) -> Vec<(PathBuf, String)> {
    let mut roots = vec![(data.to_path_buf(), "vak".to_string())];
    let mut named: Vec<(PathBuf, String)> = std::fs::read_dir(data.join("agents"))
        .into_iter()
        .flatten()
        .flatten()
        .filter(|entry| entry.path().is_dir())
        .filter_map(|entry| {
            let id = entry.file_name().to_str()?.to_string();
            Some((entry.path(), id))
        })
        .collect();
    named.sort();
    roots.extend(named);
    roots
}

/// Every source under the data home `data`, in a stable order.
pub fn discover(data: &Path) -> Vec<Source> {
    let mut found = Vec::new();
    let shared = vak_config::scope::SharedScope::new(data);
    for (chain, source) in [
        (shared.runs(), Source::Runs as fn(PathBuf) -> Source),
        (shared.effects(), Source::Effects),
    ] {
        if chain.is_dir() {
            found.push(source(chain));
        }
    }
    let mut triggers = vak_session::documents::under(&shared.triggers());
    triggers.sort();
    found.extend(triggers.into_iter().map(Source::Trigger));
    let mut sources = vak_session::documents::under(&shared.sources());
    sources.sort();
    found.extend(sources.into_iter().map(Source::IntakeSource));
    if shared.artifacts().is_dir() {
        found.push(Source::Artifacts(shared.artifacts()));
    }
    if shared.grants().is_dir() {
        found.push(Source::Grants(shared.grants()));
    }
    if shared.lifecycle().is_dir() {
        found.push(Source::Lifecycle(shared.lifecycle()));
    }
    if shared.erasures().is_dir() {
        found.push(Source::Erasures(shared.erasures()));
    }
    if shared.intake().is_dir() {
        found.push(Source::Intake {
            dir: shared.intake(),
            tenant: vak_config::paths::tenant_home_at(data, vak_config::paths::LOCAL_TENANT),
        });
    }
    for (root, agent) in agent_roots(data) {
        let scope = vak_config::scope::AgentScope::new(&root);
        let mut ledgers: Vec<PathBuf> = std::fs::read_dir(scope.sessions_root())
            .into_iter()
            .flatten()
            .flatten()
            .filter(|space| space.path().is_dir())
            .flat_map(|space| {
                std::fs::read_dir(space.path())
                    .into_iter()
                    .flatten()
                    .flatten()
                    .map(|entry| entry.path())
                    .filter(|path| vak_config::scope::ledger_session_id(path).is_some())
            })
            .collect();
        ledgers.sort();
        found.extend(ledgers.into_iter().map(Source::Session));
        let commitments = scope.commitments();
        if commitments.is_dir() {
            found.push(Source::Commitments {
                dir: commitments,
                agent: agent.clone(),
            });
        }
        let mut memory = vak_session::documents::under(&scope.memory_root());
        memory.sort();
        found.extend(memory.into_iter().map(|path| Source::Memory {
            path,
            agent: agent.clone(),
        }));
        let mut entities = vak_session::documents::under(&root.join("entities"));
        entities.sort();
        found.extend(entities.into_iter().map(|path| Source::Entity {
            path,
            agent: agent.clone(),
        }));
    }
    found
}
