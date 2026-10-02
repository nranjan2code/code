//! Typed scope handles over the current on-disk layout
//! (`docs/plans/data-architecture-plan.md`, M3a).
//!
//! Every durable path that a Core, a ledger or a store reaches is named here
//! once, as an accessor on the scope that owns it, instead of being spelled
//! `home.join("...")` at the call site. The accessors resolve to *today's*
//! paths byte for byte; M3b changes where a scope's files live by changing
//! this module, not its callers.
//!
//! A handle is a thin owner of a root path. Construction does no I/O, and an
//! accessor only joins path components.
//!
//! - [`AgentScope`]: one Agent's private home, the value
//!   `Core::sessions_home()` returns.
//! - [`SharedScope`]: the data home shared by every Agent, the value
//!   `Core::shared_data_home()` returns.
//! - [`WorkspaceScope`]: a workspace directory and its project layer
//!   (`<workspace>/.vak/`).

use std::path::{Path, PathBuf};

/// The project-layer directory inside a workspace.
pub const PROJECT_DIR: &str = ".vak";

/// Fixed-hash identity of a workspace directory (FNV-1a over its path, so a
/// session or memory directory stays reachable across toolchain upgrades).
/// Every per-workspace store keys off this one function.
pub fn workspace_key(cwd: &Path) -> String {
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for b in cwd.to_string_lossy().as_bytes() {
        h ^= u64::from(*b);
        h = h.wrapping_mul(0x0000_0100_0000_01b3);
    }
    format!("{h:016x}")
}

/// An Agent's private home.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AgentScope {
    root: PathBuf,
}

impl AgentScope {
    /// The scope rooted at an Agent home already resolved by the caller
    /// (`Core::sessions_home()`, `paths::agent_home`).
    pub fn new(root: impl Into<PathBuf>) -> Self {
        Self { root: root.into() }
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    pub fn into_root(self) -> PathBuf {
        self.root
    }

    /// `sessions/`, holding one directory per workspace.
    pub fn sessions_root(&self) -> PathBuf {
        self.root.join("sessions")
    }

    pub fn sessions_dir(&self, cwd: &Path) -> PathBuf {
        self.sessions_root().join(workspace_key(cwd))
    }

    pub fn session_file(&self, cwd: &Path, session_id: &str) -> PathBuf {
        self.sessions_dir(cwd).join(format!("{session_id}.jsonl"))
    }

    pub fn checkpoints(&self) -> PathBuf {
        self.root.join("checkpoints")
    }

    pub fn memory_root(&self) -> PathBuf {
        self.root.join("memory")
    }

    pub fn memory_dir(&self, cwd: &Path) -> PathBuf {
        self.memory_root().join(workspace_key(cwd))
    }

    pub fn memory_notes(&self, cwd: &Path) -> PathBuf {
        self.memory_dir(cwd).join("MEMORY.md")
    }

    pub fn user_memory(&self) -> PathBuf {
        self.memory_root().join("user").join("USER.md")
    }

    /// `None` is the Agent-wide (global) entity store.
    pub fn entities_file(&self, cwd: Option<&Path>) -> PathBuf {
        let sub = match cwd {
            Some(dir) => workspace_key(dir),
            None => "global".to_string(),
        };
        self.root.join("entities").join(sub).join("ENTITIES.jsonl")
    }

    pub fn skill_proposals_root(&self) -> PathBuf {
        self.root.join("skill-proposals")
    }

    pub fn skill_proposals(&self, cwd: &Path) -> PathBuf {
        self.skill_proposals_root().join(workspace_key(cwd))
    }

    pub fn skills(&self) -> PathBuf {
        self.root.join("skills")
    }

    pub fn skill(&self, name: &str) -> PathBuf {
        self.skills().join(name)
    }

    /// `agents/`, which holds the homes of the Agents beneath this one.
    pub fn agents_dir(&self) -> PathBuf {
        self.root.join("agents")
    }

    pub fn security_events(&self) -> PathBuf {
        self.root.join("security-events.jsonl")
    }

    pub fn cost_log(&self) -> PathBuf {
        self.root.join("cost-log.jsonl")
    }

    pub fn activity_log(&self) -> PathBuf {
        self.root.join("activity-log.jsonl")
    }

    pub fn routing_evidence(&self) -> PathBuf {
        self.root.join("routing-evidence.jsonl")
    }

    pub fn intent_evidence(&self) -> PathBuf {
        self.root.join("intent-evidence.jsonl")
    }

    pub fn commitments(&self) -> PathBuf {
        self.root.join("commitments.jsonl")
    }

    pub fn inbox(&self) -> PathBuf {
        self.root.join("inbox.jsonl")
    }

    pub fn inbox_dedupe_lock(&self) -> PathBuf {
        self.root.join("inbox.dedupe.lock")
    }

    pub fn env_file(&self) -> PathBuf {
        self.root.join(".env")
    }

    pub fn flow_runs(&self) -> PathBuf {
        self.root.join("flow-runs")
    }

    pub fn managed_flow_runs(&self) -> PathBuf {
        self.flow_runs().join("managed")
    }

    pub fn sandbox_dir(&self) -> PathBuf {
        self.root.join("sandbox")
    }

    /// Where a revision copy is staged before it is frozen as a candidate.
    /// D25, as [`AgentScope::sandbox_records`].
    pub fn sandbox_staging(&self, _session_agent: &str, id: &str) -> PathBuf {
        self.sandbox_dir().join("staging").join(id)
    }

    /// Display names the admin console keeps for workspaces.
    pub fn workspace_names(&self) -> PathBuf {
        self.root.join("workspace-names.json")
    }

    /// Output preferences of the user layer.
    pub fn output_prefs(&self) -> PathBuf {
        self.root.join("output.toml")
    }

    /// Server-side sandbox record log of the Agent that owns `_session_agent`.
    ///
    /// D25: the server writes this under its own Core's home whichever Agent
    /// owns the session, so the Agent argument is accepted and ignored today.
    /// M3b resolves it to the session's Agent here, in this one place.
    pub fn sandbox_records(&self, _session_agent: &str) -> PathBuf {
        self.sandbox_dir().join("records.jsonl")
    }

    /// Candidate roots. D25, as [`AgentScope::sandbox_records`].
    pub fn sandbox_candidates(&self, _session_agent: &str) -> PathBuf {
        self.sandbox_dir().join("candidates")
    }

    /// One session's execution stream. D25, as [`AgentScope::sandbox_records`].
    pub fn sandbox_executions(&self, _session_agent: &str, session_id: &str) -> PathBuf {
        self.sandbox_dir()
            .join("executions")
            .join(format!("{session_id}.jsonl"))
    }

    /// Coworking grants. D25, as [`AgentScope::sandbox_records`].
    pub fn coworking_grants(&self, _session_agent: &str) -> PathBuf {
        self.root.join("coworking").join("grants.jsonl")
    }

    /// Shared Office rooms of one session. D25, as
    /// [`AgentScope::sandbox_records`].
    pub fn office_workspaces(&self, _session_agent: &str, session_id: &str) -> PathBuf {
        crate::paths::office_workspaces_at(&self.root, session_id)
    }
}

/// The data home shared by every Agent.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SharedScope {
    root: PathBuf,
}

impl SharedScope {
    pub fn new(root: impl Into<PathBuf>) -> Self {
        Self { root: root.into() }
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    pub fn into_root(self) -> PathBuf {
        self.root
    }

    /// `agents/`, holding every Agent's private home.
    pub fn agents_dir(&self) -> PathBuf {
        self.root.join("agents")
    }

    pub fn agent(&self, agent_id: &str) -> AgentScope {
        AgentScope::new(crate::paths::agent_home_at(&self.root, agent_id))
    }

    pub fn gateway(&self) -> PathBuf {
        self.root.join("gateway")
    }

    pub fn gateway_allowlist(&self) -> PathBuf {
        self.gateway().join("allowlist.json")
    }

    pub fn gateway_bindings(&self) -> PathBuf {
        self.gateway().join("bindings.json")
    }

    pub fn gateway_bots(&self) -> PathBuf {
        self.gateway().join("bots.json")
    }

    pub fn gateway_deliveries(&self) -> PathBuf {
        self.gateway().join("deliveries.jsonl")
    }

    /// The durable delivery outbox.
    pub fn delivery_jobs(&self) -> PathBuf {
        self.root.join("delivery").join("jobs")
    }

    pub fn operations_incidents(&self) -> PathBuf {
        self.operations().join("incidents.jsonl")
    }

    pub fn operations_actions(&self) -> PathBuf {
        self.operations().join("actions.jsonl")
    }

    pub fn tasks(&self) -> PathBuf {
        self.root.join("tasks.json")
    }

    /// The trash sidecar.
    pub fn deleted(&self) -> PathBuf {
        self.root.join("deleted.json")
    }

    /// The archive sidecar.
    pub fn archive(&self) -> PathBuf {
        self.root.join("archive.json")
    }

    pub fn operations(&self) -> PathBuf {
        self.root.join("operations")
    }

    pub fn cost_log(&self) -> PathBuf {
        self.root.join("cost-log.jsonl")
    }

    pub fn env_file(&self) -> PathBuf {
        self.root.join(".env")
    }

    /// Workspace-trust markers, one per trusted workspace.
    pub fn trusted(&self) -> PathBuf {
        self.root.join("trusted")
    }

    pub fn sandbox_promotions(&self) -> PathBuf {
        self.root.join("sandbox").join("promotions")
    }
}

/// A workspace directory and its project layer.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WorkspaceScope {
    root: PathBuf,
}

impl WorkspaceScope {
    pub fn new(root: impl Into<PathBuf>) -> Self {
        Self { root: root.into() }
    }

    /// The scope of a workspace-relative path: every accessor then returns
    /// the path relative to the workspace root.
    pub fn relative() -> Self {
        Self::new(PathBuf::new())
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    /// `<workspace>/.vak`.
    pub fn project_dir(&self) -> PathBuf {
        self.root.join(PROJECT_DIR)
    }

    pub fn config_file(&self) -> PathBuf {
        self.project_dir().join("config.toml")
    }

    pub fn skills(&self) -> PathBuf {
        self.project_dir().join("skills")
    }

    pub fn plugins(&self) -> PathBuf {
        self.project_dir().join("plugins")
    }

    pub fn commands(&self) -> PathBuf {
        self.project_dir().join("commands")
    }

    pub fn flows(&self) -> PathBuf {
        self.project_dir().join("flows")
    }

    pub fn prompts(&self) -> PathBuf {
        self.project_dir().join("prompts")
    }

    pub fn permissions_local(&self) -> PathBuf {
        self.project_dir().join("permissions.local.toml")
    }

    pub fn agents_file(&self) -> PathBuf {
        self.project_dir().join("agents.json")
    }

    pub fn output_prefs(&self) -> PathBuf {
        self.project_dir().join("output.toml")
    }

    pub fn feeds_config(&self) -> PathBuf {
        self.project_dir().join("feeds.toml")
    }

    /// `<workspace>/.vak/scratch/<agent_id>`, an Agent's quarantined
    /// execution scratch.
    pub fn scratch(&self, agent_id: &str) -> PathBuf {
        self.project_dir().join("scratch").join(agent_id)
    }

    pub fn worktrees(&self) -> PathBuf {
        self.project_dir().join("worktrees")
    }

    pub fn env_file(&self) -> PathBuf {
        self.root.join(".env")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn agent_scope_matches_the_raw_paths() {
        let home = PathBuf::from("/data/agents/mira");
        let cwd = Path::new("/work/proj");
        let key = workspace_key(cwd);
        let s = AgentScope::new(&home);
        assert_eq!(s.sessions_dir(cwd), home.join("sessions").join(&key));
        assert_eq!(
            s.session_file(cwd, "abc"),
            home.join("sessions").join(&key).join("abc.jsonl")
        );
        assert_eq!(s.checkpoints(), home.join("checkpoints"));
        assert_eq!(
            s.memory_notes(cwd),
            home.join("memory").join(&key).join("MEMORY.md")
        );
        assert_eq!(s.user_memory(), home.join("memory/user/USER.md"));
        assert_eq!(
            s.entities_file(Some(cwd)),
            home.join("entities").join(&key).join("ENTITIES.jsonl")
        );
        assert_eq!(
            s.entities_file(None),
            home.join("entities/global/ENTITIES.jsonl")
        );
        assert_eq!(
            s.skill_proposals(cwd),
            home.join("skill-proposals").join(&key)
        );
        assert_eq!(s.skill("x"), home.join("skills/x"));
        assert_eq!(s.agents_dir(), home.join("agents"));
        assert_eq!(s.security_events(), home.join("security-events.jsonl"));
        assert_eq!(s.cost_log(), home.join("cost-log.jsonl"));
        assert_eq!(s.activity_log(), home.join("activity-log.jsonl"));
        assert_eq!(s.routing_evidence(), home.join("routing-evidence.jsonl"));
        assert_eq!(s.intent_evidence(), home.join("intent-evidence.jsonl"));
        assert_eq!(s.commitments(), home.join("commitments.jsonl"));
        assert_eq!(s.inbox(), home.join("inbox.jsonl"));
        assert_eq!(s.inbox_dedupe_lock(), home.join("inbox.dedupe.lock"));
        assert_eq!(s.env_file(), home.join(".env"));
        assert_eq!(s.managed_flow_runs(), home.join("flow-runs/managed"));
        assert_eq!(s.sandbox_records("a"), home.join("sandbox/records.jsonl"));
        assert_eq!(s.sandbox_candidates("a"), home.join("sandbox/candidates"));
        assert_eq!(
            s.sandbox_executions("a", "s1"),
            home.join("sandbox/executions/s1.jsonl")
        );
        assert_eq!(s.coworking_grants("a"), home.join("coworking/grants.jsonl"));
        assert_eq!(s.sandbox_staging("a", "x"), home.join("sandbox/staging/x"));
        assert_eq!(s.workspace_names(), home.join("workspace-names.json"));
        assert_eq!(s.output_prefs(), home.join("output.toml"));
        assert_eq!(
            s.office_workspaces("a", "s1"),
            home.join("office-workspaces/s1")
        );
    }

    #[test]
    fn shared_scope_matches_the_raw_paths() {
        let data = PathBuf::from("/data");
        let s = SharedScope::new(&data);
        assert_eq!(s.agents_dir(), data.join("agents"));
        assert_eq!(s.agent("mira").root(), data.join("agents/mira"));
        assert_eq!(s.gateway_allowlist(), data.join("gateway/allowlist.json"));
        assert_eq!(s.tasks(), data.join("tasks.json"));
        assert_eq!(s.deleted(), data.join("deleted.json"));
        assert_eq!(s.archive(), data.join("archive.json"));
        assert_eq!(s.operations(), data.join("operations"));
        assert_eq!(s.cost_log(), data.join("cost-log.jsonl"));
        assert_eq!(s.trusted(), data.join("trusted"));
        assert_eq!(s.gateway_bindings(), data.join("gateway/bindings.json"));
        assert_eq!(s.gateway_bots(), data.join("gateway/bots.json"));
        assert_eq!(
            s.gateway_deliveries(),
            data.join("gateway/deliveries.jsonl")
        );
        assert_eq!(s.delivery_jobs(), data.join("delivery/jobs"));
        assert_eq!(
            s.operations_incidents(),
            data.join("operations/incidents.jsonl")
        );
        assert_eq!(
            s.operations_actions(),
            data.join("operations/actions.jsonl")
        );
        assert_eq!(s.sandbox_promotions(), data.join("sandbox/promotions"));
    }

    #[test]
    fn workspace_scope_matches_the_raw_paths() {
        let ws = PathBuf::from("/work/proj");
        let s = WorkspaceScope::new(&ws);
        assert_eq!(s.project_dir(), ws.join(".vak"));
        assert_eq!(s.config_file(), ws.join(".vak/config.toml"));
        assert_eq!(s.skills(), ws.join(".vak/skills"));
        assert_eq!(s.plugins(), ws.join(".vak/plugins"));
        assert_eq!(s.commands(), ws.join(".vak/commands"));
        assert_eq!(s.flows(), ws.join(".vak/flows"));
        assert_eq!(s.prompts(), ws.join(".vak/prompts"));
        assert_eq!(s.agents_file(), ws.join(".vak/agents.json"));
        assert_eq!(s.worktrees(), ws.join(".vak/worktrees"));
        assert_eq!(s.output_prefs(), ws.join(".vak/output.toml"));
        assert_eq!(s.feeds_config(), ws.join(".vak/feeds.toml"));
        assert_eq!(s.scratch("mira"), ws.join(".vak/scratch/mira"));
        assert_eq!(
            s.permissions_local(),
            ws.join(".vak/permissions.local.toml")
        );
        assert_eq!(
            WorkspaceScope::relative().config_file(),
            PathBuf::from(".vak/config.toml")
        );
    }

    #[test]
    fn workspace_key_is_the_frozen_fnv1a() {
        assert_eq!(workspace_key(Path::new("")), "cbf29ce484222325");
        assert_eq!(workspace_key(Path::new("/work/proj")).len(), 16);
    }
}
