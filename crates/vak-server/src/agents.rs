//! Durable user-facing Agent definitions.
//!
//! Definitions describe presentation and prompt preferences. They never grant
//! tools, permissions, credentials, budget, or a wider execution scope.

use std::{
    collections::HashSet,
    path::{Path, PathBuf},
};

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum AgentLifecycle {
    Active,
    Paused,
    Archived,
}

fn default_lifecycle() -> AgentLifecycle {
    AgentLifecycle::Active
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentDefinition {
    pub id: String,
    #[serde(default = "default_revision")]
    pub revision: u64,
    #[serde(default = "default_lifecycle")]
    pub lifecycle: AgentLifecycle,
    pub name: String,
    pub character: String,
    pub personality: String,
    pub behaviour: String,
    #[serde(default)]
    pub responsibilities: String,
    #[serde(default)]
    pub instructions: String,
    pub animation: String,
    pub voice: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub schedule: Option<AgentSchedule>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct AgentSchedule {
    pub cron_or_interval: String,
    pub prompt: String,
    #[serde(default = "default_true")]
    pub enabled: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_run_at: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_status: Option<String>,
}

fn default_true() -> bool {
    true
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentRunRecord {
    pub run_id: String,
    pub agent_id: String,
    pub started_at: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub completed_at: Option<String>,
    pub status: String,
    pub prompt: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub summary: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

fn default_revision() -> u64 {
    1
}

impl AgentDefinition {
    pub fn is_admissible(&self) -> bool {
        self.lifecycle == AgentLifecycle::Active
    }

    pub fn identity(&self) -> vak_session::types::AgentIdentity {
        vak_session::types::AgentIdentity {
            id: self.id.clone(),
            revision: self.revision,
            name: self.name.clone(),
            personality: self.personality.clone(),
            behaviour: self.behaviour.clone(),
            responsibilities: self.responsibilities.clone(),
            instructions: self.instructions.clone(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentTemplate {
    pub template_id: String,
    pub domain: String,
    pub name: String,
    pub description: String,
    pub character: String,
    pub personality: String,
    pub behaviour: String,
    pub responsibilities: String,
    pub instructions: String,
    pub animation: String,
    pub voice: String,
}

impl AgentTemplate {
    pub fn to_agent_definition(
        &self,
        agent_id: &str,
        custom_name: Option<&str>,
    ) -> AgentDefinition {
        AgentDefinition {
            id: agent_id.to_string(),
            revision: 1,
            lifecycle: AgentLifecycle::Active,
            name: custom_name.unwrap_or(&self.name).to_string(),
            character: self.character.clone(),
            personality: self.personality.clone(),
            behaviour: self.behaviour.clone(),
            responsibilities: self.responsibilities.clone(),
            instructions: self.instructions.clone(),
            animation: self.animation.clone(),
            voice: self.voice.clone(),
            schedule: None,
        }
    }
}

pub fn builtin_templates() -> Vec<AgentTemplate> {
    vec![
        AgentTemplate {
            template_id: "researcher".into(),
            domain: "Research & Synthesis".into(),
            name: "Research Analyst".into(),
            description: "Empirical research specialist focusing on source auditing, citation integrity, and evidence synthesis.".into(),
            character: "leaf".into(),
            personality: "Rigorous, impartial, inquisitive, and evidence-driven.".into(),
            behaviour: "Cite every factual finding with numbered brackets [1], [2] linked to bibliography. Actively highlight epistemic uncertainty and counter-evidence.".into(),
            responsibilities: "Literature review, competitive intelligence, factual verification, and multi-source synthesis.".into(),
            instructions: "When analyzing sources, verify source reliability before adopting claims. Never present unverified inferences as established fact. Use doc_read and webfetch where available to check claims against primary sources.".into(),
            animation: "subtle".into(),
            voice: "calm".into(),
        },
        AgentTemplate {
            template_id: "writer".into(),
            domain: "Communications & Writing".into(),
            name: "Communications & Writer".into(),
            description: "Narrative, technical, and executive communications specialist focusing on audience tone and structural flow.".into(),
            character: "wave".into(),
            personality: "Clear, articulate, engaging, and rhetorically adaptable.".into(),
            behaviour: "Structure deliverables with clear hierarchies, compelling introductions, scannable body sections, and concise executive summaries.".into(),
            responsibilities: "Drafting essays, briefing memos, documentation, announcements, and narrative communications.".into(),
            instructions: "Adapt tone and vocabulary precisely to target audience requirements. Ensure high scannability using clear headings, concise paragraphs, and bulleted takeaways.".into(),
            animation: "expressive".into(),
            voice: "bright".into(),
        },
        AgentTemplate {
            template_id: "operator".into(),
            domain: "Operations & Strategy".into(),
            name: "Operations & Strategy Lead".into(),
            description: "Structured operational planner specializing in trade-off analysis, decision matrices, and execution timelines.".into(),
            character: "sun".into(),
            personality: "Pragmatic, structured, risk-aware, and action-oriented.".into(),
            behaviour: "Evaluate multiple strategic options with weighted criteria in structured decision cards or comparison tables. Formulate clear milestone roadmaps.".into(),
            responsibilities: "Strategic planning, operational workflows, vendor/option comparisons, and project scheduling.".into(),
            instructions: "Never present a recommendation without assessing trade-offs, potential failure modes, and required milestones. Use vak-decision matrices to compare competing paths.".into(),
            animation: "subtle".into(),
            voice: "calm".into(),
        },
        AgentTemplate {
            template_id: "analyst".into(),
            domain: "Data & Analytics".into(),
            name: "Data Analyst".into(),
            description: "Quantitative specialist for tabular data querying, statistical summary, and living dataframe presentations.".into(),
            character: "spark".into(),
            personality: "Precise, analytical, detail-oriented, and statistically sound.".into(),
            behaviour: "Execute deterministic data aggregations with data_query, inspect row distributions, verify numerical totals, and format outputs as clean tables.".into(),
            responsibilities: "CSV/TSV analysis, tabular transformations, summary statistics, and quantitative reporting.".into(),
            instructions: "Always verify mathematical accuracy against source data before stating conclusions. Provide row counts, distribution summaries, and clear column labels.".into(),
            animation: "subtle".into(),
            voice: "quiet".into(),
        },
    ]
}

pub fn find_template(id: &str) -> Option<AgentTemplate> {
    builtin_templates()
        .into_iter()
        .find(|t| t.template_id == id)
}

pub fn effective(core: &vak_core::Core) -> Result<Vec<AgentDefinition>, String> {
    let shared = vak_config::paths::default_workspace();
    let mut profiles = load(&shared)?;
    if core.cwd() != &shared && core.project_config_trusted() {
        for profile in load(core.cwd())? {
            profiles.retain(|p| p.id != profile.id);
            profiles.push(profile);
        }
    }
    Ok(profiles)
}

fn path(cwd: &Path) -> PathBuf {
    cwd.join(".vak").join("agents.json")
}

pub fn load(cwd: &Path) -> Result<Vec<AgentDefinition>, String> {
    let file = path(cwd);
    let raw = match std::fs::read_to_string(&file) {
        Ok(raw) => raw,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(e) => return Err(format!("cannot read {}: {e}", file.display())),
    };
    serde_json::from_str(&raw).map_err(|e| format!("invalid agents: {e}"))
}

/// `trusted` is the *creating* context's own trust decision (the workspace
/// this admin/client session is already running against), carried forward
/// onto each profile's isolated workspace so its own privileged config
/// (`permission_mode`, `hooks`, `mcp.servers`, ...) actually applies —
/// otherwise every user-created Agent's own settings are silently stripped
/// forever, since nothing else ever visits or prompts about that nested
/// directory (see `vak_core::trust::mark_trusted`).
pub fn save(
    cwd: &Path,
    profiles: &[AgentDefinition],
    trusted: bool,
) -> Result<Vec<AgentDefinition>, String> {
    if profiles.len() > 100 {
        return Err("at most 100 agents are allowed".into());
    }
    let mut ids = HashSet::with_capacity(profiles.len());
    for profile in profiles {
        if profile.id == "vak" {
            return Err("Vak is the built-in agent; choose another id".into());
        }
        if profile.id.trim().is_empty() || profile.name.trim().is_empty() {
            return Err("agent id and name are required".into());
        }
        if !ids.insert(profile.id.clone()) {
            return Err(format!("agent id '{}' is duplicated", profile.id));
        }
        if profile.name.len() > 120
            || profile.personality.len() > 4000
            || profile.behaviour.len() > 4000
            || profile.responsibilities.len() > 2000
            || profile.instructions.len() > 8000
        {
            return Err(format!("agent '{}' is too large", profile.id));
        }
        if !matches!(profile.animation.as_str(), "subtle" | "expressive" | "off") {
            return Err("animation must be subtle, expressive, or off".into());
        }
        if profile.voice.len() > 80 {
            return Err(format!("agent '{}' voice setting is too large", profile.id));
        }
        if !matches!(
            profile.character.as_str(),
            "orb" | "leaf" | "sun" | "wave" | "spark"
        ) {
            return Err("unknown character preset".into());
        }
        if !matches!(
            profile.voice.as_str(),
            "default" | "calm" | "bright" | "quiet"
        ) {
            return Err("voice must be default, calm, bright, or quiet".into());
        }
    }
    let dir = cwd.join(".vak");
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    let lock = std::fs::OpenOptions::new()
        .create(true)
        .truncate(false)
        .read(true)
        .write(true)
        .open(dir.join("agents.lock"))
        .map_err(|e| e.to_string())?;
    lock.try_lock()
        .map_err(|e| format!("agents are being edited; retry: {e}"))?;
    let target = path(cwd);
    let temp = target.with_extension(format!("{}.tmp", uuid::Uuid::now_v7()));
    let previous = load(cwd)?;
    let mut next = profiles.to_vec();
    for profile in &mut next {
        if let Some(old) = previous.iter().find(|candidate| candidate.id == profile.id) {
            if profile.name != old.name
                || profile.character != old.character
                || profile.personality != old.personality
                || profile.behaviour != old.behaviour
                || profile.responsibilities != old.responsibilities
                || profile.animation != old.animation
                || profile.voice != old.voice
                || profile.schedule != old.schedule
            {
                profile.revision = old.revision.saturating_add(1);
            } else {
                profile.revision = old.revision;
            }
        } else {
            profile.revision = 1;
        }
    }
    // A full-layer edit must preserve future fields on retained profiles.
    let old_values: Vec<serde_json::Value> = match std::fs::read_to_string(&target) {
        Ok(raw) => serde_json::from_str(&raw).map_err(|e| format!("invalid agents: {e}"))?,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Vec::new(),
        Err(e) => return Err(e.to_string()),
    };
    let values = next
        .iter()
        .map(|profile| {
            let mut value = old_values
                .iter()
                .find(|v| v["id"].as_str() == Some(&profile.id))
                .cloned()
                .unwrap_or_else(|| serde_json::json!({}));
            let fields = serde_json::to_value(profile).map_err(|e| e.to_string())?;
            if let (Some(old), Some(new)) = (value.as_object_mut(), fields.as_object()) {
                old.extend(new.clone());
            }
            Ok(value)
        })
        .collect::<Result<Vec<_>, String>>()?;
    let bytes = serde_json::to_vec_pretty(&values).map_err(|e| e.to_string())?;
    std::fs::write(&temp, bytes).map_err(|e| e.to_string())?;
    std::fs::rename(&temp, &target).map_err(|e| e.to_string())?;
    for profile in &next {
        let agent_dir = vak_config::paths::agent_home(&profile.id);
        let _ = std::fs::create_dir_all(&agent_dir);
        let workspace_dir = vak_config::paths::agent_workspace(cwd, &profile.id);
        let _ = std::fs::create_dir_all(&workspace_dir);
        if trusted {
            let _ = vak_core::trust::mark_trusted(&workspace_dir);
        }
    }
    Ok(next)
}

pub fn runs_path(cwd: &Path) -> PathBuf {
    cwd.join(".vak").join("agents_runs.jsonl")
}

pub fn record_run(cwd: &Path, record: &AgentRunRecord) -> Result<(), String> {
    let dir = cwd.join(".vak");
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    let path = runs_path(cwd);
    let mut file = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&path)
        .map_err(|e| e.to_string())?;
    use std::io::Write;
    let line = serde_json::to_string(record).map_err(|e| e.to_string())?;
    writeln!(file, "{line}").map_err(|e| e.to_string())?;
    Ok(())
}

pub fn list_runs(
    cwd: &Path,
    agent_id: Option<&str>,
    limit: usize,
) -> Result<Vec<AgentRunRecord>, String> {
    let path = runs_path(cwd);
    if !path.exists() {
        return Ok(Vec::new());
    }
    let content = std::fs::read_to_string(&path).map_err(|e| e.to_string())?;
    let mut records: Vec<AgentRunRecord> = Vec::new();
    for line in content.lines().rev() {
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }
        if let Ok(rec) = serde_json::from_str::<AgentRunRecord>(trimmed) {
            if agent_id.is_some_and(|target_id| rec.agent_id != target_id) {
                continue;
            }
            records.push(rec);
            if records.len() >= limit {
                break;
            }
        }
    }
    Ok(records)
}

pub fn update_schedule(
    cwd: &Path,
    agent_id: &str,
    schedule: Option<AgentSchedule>,
    trusted: bool,
) -> Result<AgentDefinition, String> {
    let mut profiles = load(cwd)?;
    let Some(profile) = profiles.iter_mut().find(|a| a.id == agent_id) else {
        return Err(format!("agent '{agent_id}' not found"));
    };
    profile.schedule = schedule;
    let saved = save(cwd, &profiles, trusted)?;
    saved
        .into_iter()
        .find(|a| a.id == agent_id)
        .ok_or_else(|| "agent disappeared after save".to_string())
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::*;

    #[test]
    fn profiles_round_trip_atomically() {
        let dir = tempfile::tempdir().expect("profile workspace");
        let profiles = vec![AgentDefinition {
            id: "pip".into(),
            revision: 1,
            lifecycle: AgentLifecycle::Active,
            name: "Pip".into(),
            character: "spark".into(),
            personality: "Warm".into(),
            behaviour: "Be useful".into(),
            responsibilities: String::new(),
            instructions: String::new(),
            animation: "subtle".into(),
            voice: "default".into(),
            schedule: None,
        }];
        save(dir.path(), &profiles, true).expect("save profiles");
        assert_eq!(load(dir.path()).expect("load profiles")[0].name, "Pip");
    }

    #[test]
    fn retired_profile_store_is_not_read_as_agent_state() {
        let dir = tempfile::tempdir().expect("agent workspace");
        std::fs::create_dir_all(dir.path().join(".vak")).expect("agent config dir");
        std::fs::write(
            dir.path().join(".vak/agent-profiles.json"),
            "[{\"id\":\"old\",\"name\":\"Old\"}]",
        )
        .expect("retired store");
        assert!(load(dir.path()).expect("load agents").is_empty());
    }

    #[test]
    fn invalid_character_is_rejected() {
        let dir = tempfile::tempdir().expect("profile workspace");
        let profile = AgentDefinition {
            id: "x".into(),
            revision: 1,
            lifecycle: AgentLifecycle::Active,
            name: "X".into(),
            character: "unknown".into(),
            personality: String::new(),
            behaviour: String::new(),
            responsibilities: String::new(),
            instructions: String::new(),
            animation: "off".into(),
            voice: "default".into(),
            schedule: None,
        };
        assert!(save(dir.path(), &[profile], true).is_err());
    }

    #[test]
    fn invalid_voice_is_rejected() {
        let dir = tempfile::tempdir().expect("profile workspace");
        let profile = AgentDefinition {
            id: "x".into(),
            revision: 1,
            lifecycle: AgentLifecycle::Active,
            name: "X".into(),
            character: "orb".into(),
            personality: String::new(),
            behaviour: String::new(),
            responsibilities: String::new(),
            instructions: String::new(),
            animation: "off".into(),
            voice: "unknown".into(),
            schedule: None,
        };
        assert!(save(dir.path(), &[profile], true).is_err());
    }

    #[test]
    fn lifecycle_round_trips_and_admission_is_active_only() {
        let dir = tempfile::tempdir().expect("agent workspace");
        let mut agent = AgentDefinition {
            id: "paused".into(),
            revision: 1,
            lifecycle: AgentLifecycle::Paused,
            name: "Paused".into(),
            character: "orb".into(),
            personality: String::new(),
            behaviour: String::new(),
            responsibilities: String::new(),
            instructions: String::new(),
            animation: "off".into(),
            voice: "default".into(),
            schedule: None,
        };
        save(dir.path(), &[agent.clone()], true).expect("save paused agent");
        agent = load(dir.path()).expect("load paused agent").remove(0);
        assert_eq!(agent.lifecycle, AgentLifecycle::Paused);
        assert!(!agent.is_admissible());
    }

    #[test]
    fn edits_increment_saved_revision() {
        let dir = tempfile::tempdir().expect("profile workspace");
        let profile = AgentDefinition {
            id: "pip".into(),
            revision: 1,
            lifecycle: AgentLifecycle::Active,
            name: "Pip".into(),
            character: "orb".into(),
            personality: "Warm".into(),
            behaviour: "Be useful".into(),
            responsibilities: String::new(),
            instructions: String::new(),
            animation: "subtle".into(),
            voice: "default".into(),
            schedule: None,
        };
        let mut untouched = profile.clone();
        untouched.id = "atlas".into();
        untouched.name = "Atlas".into();
        let saved = save(dir.path(), &[profile.clone(), untouched.clone()], true).expect("first save");
        assert_eq!(saved[0].revision, 1);
        let mut edited = profile;
        edited.personality = "Warm and direct".into();
        let saved = save(dir.path(), &[edited, untouched], true).expect("edited save");
        assert_eq!(saved[0].revision, 2);
        assert_eq!(saved[1].revision, 1);
    }

    #[test]
    fn duplicate_ids_are_rejected() {
        let dir = tempfile::tempdir().expect("profile workspace");
        let profile = AgentDefinition {
            id: "same".into(),
            revision: 1,
            lifecycle: AgentLifecycle::Active,
            name: "One".into(),
            character: "orb".into(),
            personality: String::new(),
            behaviour: String::new(),
            responsibilities: String::new(),
            instructions: String::new(),
            animation: "off".into(),
            voice: "default".into(),
            schedule: None,
        };
        let mut duplicate = profile.clone();
        duplicate.name = "Two".into();
        assert!(save(dir.path(), &[profile, duplicate], true).is_err());
    }

    #[test]
    fn builtin_templates_are_all_valid() {
        let dir = tempfile::tempdir().expect("template workspace");
        let templates = builtin_templates();
        assert_eq!(templates.len(), 4);
        let agents: Vec<_> = templates
            .iter()
            .map(|t| t.to_agent_definition(&t.template_id, None))
            .collect();
        let saved = save(dir.path(), &agents, true).expect("save all builtin templates");
        assert_eq!(saved.len(), 4);
        let loaded = load(dir.path()).expect("load all builtin templates");
        assert_eq!(loaded.len(), 4);
        assert!(loaded.iter().any(|a| a.id == "researcher"));
        assert!(loaded.iter().any(|a| a.id == "writer"));
        assert!(loaded.iter().any(|a| a.id == "operator"));
        assert!(loaded.iter().any(|a| a.id == "analyst"));
    }

    #[test]
    fn schedule_and_run_records_persist_and_load() {
        let dir = tempfile::tempdir().expect("agent workspace");
        let template = builtin_templates().remove(0);
        let agent = template.to_agent_definition("researcher", None);
        save(dir.path(), &[agent], true).expect("save agent");

        let updated = update_schedule(
            dir.path(),
            "researcher",
            Some(AgentSchedule {
                cron_or_interval: "daily".into(),
                prompt: "run daily literature check".into(),
                enabled: true,
                last_run_at: None,
                last_status: None,
            }),
            true,
        )
        .expect("update schedule");
        assert_eq!(
            updated
                .schedule
                .as_ref()
                .map(|s| s.cron_or_interval.as_str()),
            Some("daily")
        );

        let run = AgentRunRecord {
            run_id: "run-1".into(),
            agent_id: "researcher".into(),
            started_at: "2026-09-14T10:00:00Z".into(),
            completed_at: Some("2026-09-14T10:00:05Z".into()),
            status: "succeeded".into(),
            prompt: "run daily literature check".into(),
            summary: Some("Found 3 new papers".into()),
            error: None,
        };
        record_run(dir.path(), &run).expect("record run");

        let runs = list_runs(dir.path(), Some("researcher"), 10).expect("list runs");
        assert_eq!(runs.len(), 1);
        assert_eq!(runs[0].run_id, "run-1");
        assert_eq!(runs[0].status, "succeeded");
    }
}
