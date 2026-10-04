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
//!   `Core::scope` resolves.
//! - [`SharedScope`]: the data home shared by every Agent, the value
//!   `Core::shared_scope` resolves.
//! - [`WorkspaceScope`]: a workspace directory and its project layer
//!   (`<workspace>/.vak/`).

use std::path::{Path, PathBuf};

/// The project-layer directory inside a workspace.
pub const PROJECT_DIR: &str = ".vak";
/// Workspace-relative paths of the project layer, for code that declares a
/// path as data (the state registry) or matches text rather than resolving
/// one; everything else goes through [`WorkspaceScope`].
pub const PROJECT_CONFIG: &str = ".vak/config.toml";
pub const PROJECT_CONFIG_STEM: &str = ".vak/config";
pub const PROJECT_SKILLS: &str = ".vak/skills";
pub const PROJECT_PLUGINS: &str = ".vak/plugins";
pub const SEED_MANIFEST: &str = ".vak/.seed-manifest.json";
/// The prefix of a draft's name; the draft itself lives in the runtime
/// root (`draft_location`), never under this path in the project.
pub const SCRATCH_DIR: &str = ".vak/scratch";

/// Fixed-hash identity of a workspace directory (FNV-1a over its path, so a
/// session or memory directory stays reachable across toolchain upgrades).
/// Every per-workspace store keys off this one function.
/// The ledger of session `session_id` inside a sessions directory: the one
/// place a session ledger's on-disk name is decided.
pub fn session_ledger(dir: &Path, session_id: &str) -> PathBuf {
    dir.join(session_id)
}

/// The session id of a ledger found in a sessions directory, or `None`
/// when `path` is not a session ledger: a ledger is a directory of record
/// segments (docs/design/73 §6), named by its session id.
pub fn ledger_session_id(path: &Path) -> Option<String> {
    let first = ["seg-00000001.log", "seg-00000001.sealed"];
    if !first.iter().any(|segment| path.join(segment).is_file()) {
        return None;
    }
    path.file_name()?.to_str().map(str::to_string)
}

/// Where a space's executions keep their temp files and tool caches: the
/// runtime root, never the project tree (plan M3b slice 4, L4). Sandboxes
/// grant exactly this beside the workspace.
pub fn executions_root(cwd: &Path) -> PathBuf {
    crate::paths::runtime_dir()
        .join("executions")
        .join(crate::spaces::key(cwd))
}

/// One Agent's executions in the space at `cwd`.
pub fn execution_dir(cwd: &Path, agent_id: &str) -> PathBuf {
    executions_root(cwd).join(agent_id)
}

/// Where the draft named `name` lives. A draft is named
/// `.vak/scratch/<agent>/<execution>/<path>`: that name is what the model is
/// told and the ledger records, and the file is under `executions` (a
/// space's [`executions_root`]) at `<agent>/<execution>/<path>`, outside the
/// project tree (plan M3b slice 4). `None` when `name` is not a draft name.
pub fn draft_location(executions: &Path, name: &Path) -> Option<PathBuf> {
    name.strip_prefix(SCRATCH_DIR)
        .ok()
        .map(|rest| executions.join(rest))
}

/// An Agent's private home.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AgentScope {
    root: PathBuf,
}

impl AgentScope {
    /// The scope rooted at an Agent home already resolved by the caller
    /// (`Core::scope`, `paths::agent_home`).
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
        self.sessions_root().join(crate::spaces::key(cwd))
    }

    pub fn session_file(&self, cwd: &Path, session_id: &str) -> PathBuf {
        session_ledger(&self.sessions_dir(cwd), session_id)
    }

    pub fn checkpoints(&self) -> PathBuf {
        self.root.join("checkpoints")
    }

    pub fn memory_root(&self) -> PathBuf {
        self.root.join("memory")
    }

    pub fn memory_dir(&self, cwd: &Path) -> PathBuf {
        self.memory_root().join(crate::spaces::key(cwd))
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
            Some(dir) => crate::spaces::key(dir),
            None => "global".to_string(),
        };
        self.root.join("entities").join(sub).join("ENTITIES.jsonl")
    }

    pub fn skill_proposals_root(&self) -> PathBuf {
        self.root.join("skill-proposals")
    }

    pub fn skill_proposals(&self, cwd: &Path) -> PathBuf {
        self.skill_proposals_root().join(crate::spaces::key(cwd))
    }

    pub fn skills(&self) -> PathBuf {
        self.root.join("skills")
    }

    /// One conversation's Canvas, the same on every surface that shows the
    /// conversation (`vak-server/src/canvas.rs`).
    pub fn canvas(&self, session_id: &str) -> PathBuf {
        self.root.join("canvas").join(format!("{session_id}.json"))
    }

    pub fn skill(&self, name: &str) -> PathBuf {
        self.skills().join(name)
    }

    /// `agents/`, which holds the homes of the Agents beneath this one.
    pub fn agents_dir(&self) -> PathBuf {
        self.root.join("agents")
    }

    pub fn security_events(&self) -> PathBuf {
        self.root.join("security-events")
    }

    pub fn cost_log(&self) -> PathBuf {
        self.root.join("cost-log")
    }

    pub fn activity_log(&self) -> PathBuf {
        self.root.join("activity-log")
    }

    pub fn routing_evidence(&self) -> PathBuf {
        self.root.join("routing-evidence")
    }

    pub fn intent_evidence(&self) -> PathBuf {
        self.root.join("intent-evidence")
    }

    pub fn commitments(&self) -> PathBuf {
        self.root.join("commitments")
    }

    pub fn inbox(&self) -> PathBuf {
        self.root.join("inbox")
    }

    pub fn inbox_dedupe_lock(&self) -> PathBuf {
        self.root.join("inbox.dedupe.lock")
    }

    pub fn env_file(&self) -> PathBuf {
        self.root.join(".env")
    }

    /// The presentation-pack library (`presentations.json`).
    pub fn presentations(&self) -> PathBuf {
        self.root.join("presentations.json")
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

    /// Display names the admin console keeps for workspaces.
    pub fn workspace_names(&self) -> PathBuf {
        self.root.join("workspace-names.json")
    }

    /// Output preferences of the user layer.
    pub fn output_prefs(&self) -> PathBuf {
        self.root.join("output.toml")
    }

    /// Server-side sandbox records of this Agent's sessions. The caller
    /// picks the session's Agent (doc 73 D25).
    pub fn sandbox_records(&self) -> PathBuf {
        self.sandbox_dir().join("records.jsonl")
    }

    /// Candidate roots of this Agent's sessions.
    pub fn sandbox_candidates(&self) -> PathBuf {
        self.sandbox_dir().join("candidates")
    }

    /// One of this Agent's sessions' execution stream.
    pub fn sandbox_executions(&self, session_id: &str) -> PathBuf {
        self.sandbox_dir()
            .join("executions")
            .join(format!("{session_id}.jsonl"))
    }

    /// Coworking grants of this Agent's conversations.
    pub fn coworking_grants(&self) -> PathBuf {
        self.root.join("coworking").join("grants.jsonl")
    }

    /// Shared Office rooms of one of this Agent's sessions.
    pub fn office_workspaces(&self, session_id: &str) -> PathBuf {
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

    /// The shared home read as an Agent-shaped layout: shared skills and
    /// their proposals sit at the same relative paths as an Agent's own.
    pub fn as_agent(&self) -> AgentScope {
        AgentScope::new(self.root.clone())
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
        self.gateway().join("deliveries")
    }

    /// The durable delivery outbox.
    pub fn delivery_jobs(&self) -> PathBuf {
        self.root.join("delivery").join("jobs")
    }

    pub fn operations_incidents(&self) -> PathBuf {
        self.operations().join("incidents")
    }

    pub fn operations_actions(&self) -> PathBuf {
        self.operations().join("actions")
    }

    pub fn tasks(&self) -> PathBuf {
        self.root.join("tasks.json")
    }

    /// The trash sidecar.
    pub fn deleted(&self) -> PathBuf {
        self.root.join("deleted.json")
    }

    /// The X preview's monthly request ceiling and usage count. Shared, because
    /// one bearer token is one bill however many Agents use it.
    pub fn social_x_usage(&self) -> PathBuf {
        self.root.join("social-x-usage.json")
    }

    /// The archive sidecar.
    pub fn archive(&self) -> PathBuf {
        self.root.join("archive.json")
    }

    pub fn operations(&self) -> PathBuf {
        self.root.join("operations")
    }

    pub fn cost_log(&self) -> PathBuf {
        self.root.join("cost-log")
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

    pub fn launch_file(&self) -> PathBuf {
        self.project_dir().join("launch.toml")
    }

    pub fn seed_manifest(&self) -> PathBuf {
        self.root.join(SEED_MANIFEST)
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
        crate::paths::isolate_home_for_tests();
        let key = crate::spaces::key(cwd);
        let s = AgentScope::new(&home);
        assert_eq!(s.sessions_dir(cwd), home.join("sessions").join(&key));
        assert_eq!(
            s.session_file(cwd, "abc"),
            home.join("sessions").join(&key).join("abc")
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
        assert_eq!(s.security_events(), home.join("security-events"));
        assert_eq!(s.cost_log(), home.join("cost-log"));
        assert_eq!(s.activity_log(), home.join("activity-log"));
        assert_eq!(s.routing_evidence(), home.join("routing-evidence"));
        assert_eq!(s.intent_evidence(), home.join("intent-evidence"));
        assert_eq!(s.commitments(), home.join("commitments"));
        assert_eq!(s.inbox(), home.join("inbox"));
        assert_eq!(s.inbox_dedupe_lock(), home.join("inbox.dedupe.lock"));
        assert_eq!(s.env_file(), home.join(".env"));
        assert_eq!(s.managed_flow_runs(), home.join("flow-runs/managed"));
        assert_eq!(s.sandbox_records(), home.join("sandbox/records.jsonl"));
        assert_eq!(s.sandbox_candidates(), home.join("sandbox/candidates"));
        assert_eq!(
            s.sandbox_executions("s1"),
            home.join("sandbox/executions/s1.jsonl")
        );
        assert_eq!(s.coworking_grants(), home.join("coworking/grants.jsonl"));
        assert_eq!(s.workspace_names(), home.join("workspace-names.json"));
        assert_eq!(s.output_prefs(), home.join("output.toml"));
        assert_eq!(s.office_workspaces("s1"), home.join("office-workspaces/s1"));
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
        assert_eq!(s.cost_log(), data.join("cost-log"));
        assert_eq!(s.trusted(), data.join("trusted"));
        assert_eq!(s.gateway_bindings(), data.join("gateway/bindings.json"));
        assert_eq!(s.gateway_bots(), data.join("gateway/bots.json"));
        assert_eq!(s.gateway_deliveries(), data.join("gateway/deliveries"));
        assert_eq!(s.delivery_jobs(), data.join("delivery/jobs"));
        assert_eq!(s.operations_incidents(), data.join("operations/incidents"));
        assert_eq!(s.operations_actions(), data.join("operations/actions"));
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
        assert_eq!(s.output_prefs(), ws.join(".vak/output.toml"));
        assert_eq!(s.feeds_config(), ws.join(".vak/feeds.toml"));
        assert_eq!(
            draft_location(
                Path::new("/run/x"),
                Path::new(".vak/scratch/mira/e1/a.docx")
            ),
            Some(PathBuf::from("/run/x/mira/e1/a.docx"))
        );
        assert_eq!(
            draft_location(Path::new("/run/x"), Path::new("a.docx")),
            None
        );
        assert_eq!(
            s.permissions_local(),
            ws.join(".vak/permissions.local.toml")
        );
        assert_eq!(
            WorkspaceScope::relative().config_file(),
            PathBuf::from(".vak/config.toml")
        );
    }
}
