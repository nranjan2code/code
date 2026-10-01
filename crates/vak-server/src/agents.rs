//! Durable user-facing Agent definitions: templates, validation and saving.
//! The types and the reader live in `vak_core::agent_definitions`.

use std::{
    collections::HashSet,
    path::{Path, PathBuf},
};

use serde::{Deserialize, Serialize};

pub use vak_core::agent_definitions::{AgentDefinition, AgentLifecycle, effective, load, path};

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
        }
    }
}

pub fn builtin_templates() -> Vec<AgentTemplate> {
    vec![
        AgentTemplate {
            template_id: "researcher".into(),
            domain: "Research & Synthesis".into(),
            name: "Research Analyst".into(),
            description: "Finds and checks sources, then sums up what they say, with links you can follow.".into(),
            character: "moss".into(),
            personality: "Rigorous, impartial, inquisitive, and evidence-driven.".into(),
            behaviour: "Cite every factual finding with numbered brackets [1], [2] linked to bibliography. Actively highlight epistemic uncertainty and counter-evidence.".into(),
            responsibilities: "Literature review, competitive intelligence, factual verification, and multi-source synthesis.".into(),
            instructions: "When analyzing sources, verify source reliability before adopting claims. Never present unverified inferences as established fact. Check claims against primary sources with the tools you have.".into(),
            animation: "subtle".into(),
            voice: "calm".into(),
        },
        AgentTemplate {
            template_id: "writer".into(),
            domain: "Communications & Writing".into(),
            name: "Communications & Writer".into(),
            description: "Drafts and edits emails, posts, reports and documents in the right tone for who reads them.".into(),
            character: "pip".into(),
            personality: "Clear, articulate, engaging, and rhetorically adaptable.".into(),
            behaviour: "Structure deliverables with clear hierarchies, compelling introductions, scannable body sections, and concise executive summaries.".into(),
            responsibilities: "Drafting essays, briefing memos, documentation, announcements, and narrative communications.".into(),
            instructions: "Adapt tone and vocabulary precisely to target audience requirements. Ensure high scannability using clear headings, concise paragraphs, and bulleted takeaways.".into(),
            animation: "expressive".into(),
            voice: "bright".into(),
        },
        AgentTemplate {
            template_id: "operator".into(),
            domain: "Operations".into(),
            name: "Operations Lead".into(),
            description: "Looks before it acts, makes changes in small steps you can undo, and checks each one worked.".into(),
            character: "beni".into(),
            personality: "Pragmatic, careful, risk-aware, and action-oriented.".into(),
            behaviour: "Inspect the current state before changing it, act in small reversible steps, confirm each effect before the next, and report exactly what changed and what did not.".into(),
            responsibilities: "Running and changing systems, services and workflows; incident checks; routine operational tasks.".into(),
            instructions: "Never change something you have not inspected first. Prefer the smallest change that achieves the goal, and say how to undo it.".into(),
            animation: "subtle".into(),
            voice: "calm".into(),
        },
        AgentTemplate {
            template_id: "analyst".into(),
            domain: "Data & Analytics".into(),
            name: "Data Analyst".into(),
            description: "Works through spreadsheets and tables and explains the numbers plainly.".into(),
            character: "tavi".into(),
            personality: "Precise, analytical, detail-oriented, and statistically sound.".into(),
            behaviour: "Compute aggregations with tools rather than estimating them, inspect row distributions, verify numerical totals, and format outputs as clean tables.".into(),
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

/// Set one Agent's lifecycle in the layer that defines it: the Shared layer,
/// else the active workspace's project layer (trusted projects only). The
/// definition's other fields are carried through unchanged, so this never
/// bumps the revision. Returns the layer root it wrote.
pub fn set_lifecycle(
    core: &vak_core::Core,
    id: &str,
    lifecycle: AgentLifecycle,
) -> Result<PathBuf, String> {
    let shared = vak_config::paths::default_workspace();
    let mut layers = vec![shared.clone()];
    if core.cwd() != &shared && core.project_config_trusted() {
        layers.push(core.cwd().clone());
    }
    // The project layer shadows Shared, so it is the one to change when both
    // define the Agent.
    for root in layers.into_iter().rev() {
        let mut profiles = load(&root)?;
        let Some(profile) = profiles.iter_mut().find(|p| p.id == id) else {
            continue;
        };
        profile.lifecycle = lifecycle;
        save(&root, &profiles, core.project_config_trusted())?;
        return Ok(root);
    }
    Err(format!(
        "agent '{id}' is not defined in a layer this workspace can read"
    ))
}

/// The companion set the clients ship (`vak-client-ui/src/agentGlyph.ts`).
const CHARACTERS: [&str; 8] = ["vak", "mira", "moss", "nori", "pip", "lumi", "tavi", "beni"];
const ANIMATIONS: [&str; 3] = ["subtle", "expressive", "off"];
const VOICES: [&str; 4] = ["default", "calm", "bright", "quiet"];

/// Refuse what a save would write that this version does not accept, naming
/// the agent.
///
/// Only the values a save changes are judged; a value carried over as stored
/// is written back as it was. An agent saved by an earlier version with a
/// value since retired (the character presets were renamed in 4.0.0) must
/// never block creating or editing a different agent, and the clients always
/// send the whole layer; changing that value itself must pick one this
/// version offers.
fn check(profiles: &[AgentDefinition], stored: &[AgentDefinition]) -> Result<(), String> {
    type Field = fn(&AgentDefinition) -> &str;
    if profiles.len() > 100 {
        return Err("at most 100 agents are allowed".into());
    }
    let mut ids = HashSet::with_capacity(profiles.len());
    for profile in profiles {
        if profile.id == "vak" {
            return Err("Vakyartha is the built-in agent; choose another id".into());
        }
        if profile.id.trim().is_empty() || profile.name.trim().is_empty() {
            return Err("agent id and name are required".into());
        }
        if !ids.insert(profile.id.as_str()) {
            return Err(format!("agent id '{}' is duplicated", profile.id));
        }
        let old = stored.iter().find(|old| old.id == profile.id);
        let changed = |field: Field| old.is_none_or(|old| field(old) != field(profile));
        let name = profile.name.trim();
        let limits: [(Field, usize, &str); 6] = [
            (|p| p.name.as_str(), 120, "name"),
            (|p| p.personality.as_str(), 4000, "personality"),
            (|p| p.behaviour.as_str(), 4000, "behaviour"),
            (|p| p.responsibilities.as_str(), 2000, "responsibilities"),
            (|p| p.instructions.as_str(), 8000, "instructions"),
            (|p| p.voice.as_str(), 80, "voice setting"),
        ];
        for (field, limit, label) in limits {
            if changed(field) && field(profile).len() > limit {
                return Err(format!("agent '{name}': the {label} is too long"));
            }
        }
        if changed(|p| p.animation.as_str()) && !ANIMATIONS.contains(&profile.animation.as_str()) {
            return Err(format!(
                "agent '{name}': animation must be subtle, expressive, or off"
            ));
        }
        if changed(|p| p.character.as_str()) && !CHARACTERS.contains(&profile.character.as_str()) {
            return Err(format!(
                "agent '{name}': unknown character '{}'",
                profile.character
            ));
        }
        if changed(|p| p.voice.as_str()) && !VOICES.contains(&profile.voice.as_str()) {
            return Err(format!(
                "agent '{name}': voice must be default, calm, bright, or quiet"
            ));
        }
    }
    Ok(())
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
    check(profiles, &previous)?;
    let mut next = profiles.to_vec();
    for profile in &mut next {
        if let Some(old) = previous.iter().find(|candidate| candidate.id == profile.id) {
            if profile.name != old.name
                || profile.character != old.character
                || profile.personality != old.personality
                || profile.behaviour != old.behaviour
                || profile.responsibilities != old.responsibilities
                || profile.instructions != old.instructions
                || profile.animation != old.animation
                || profile.voice != old.voice
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

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::*;

    #[test]
    fn profiles_round_trip_atomically() {
        vak_config::paths::isolate_home_for_tests();
        let dir = tempfile::tempdir().expect("profile workspace");
        let profiles = vec![AgentDefinition {
            id: "pip".into(),
            revision: 1,
            lifecycle: AgentLifecycle::Active,
            name: "Pip".into(),
            character: "vak".into(),
            personality: "Warm".into(),
            behaviour: "Be useful".into(),
            responsibilities: String::new(),
            instructions: String::new(),
            animation: "subtle".into(),
            voice: "default".into(),
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
            character: "vak".into(),
            personality: String::new(),
            behaviour: String::new(),
            responsibilities: String::new(),
            instructions: String::new(),
            animation: "off".into(),
            voice: "unknown".into(),
        };
        assert!(save(dir.path(), &[profile], true).is_err());
    }

    #[test]
    fn lifecycle_round_trips_and_admission_is_active_only() {
        vak_config::paths::isolate_home_for_tests();
        let dir = tempfile::tempdir().expect("agent workspace");
        let mut agent = AgentDefinition {
            id: "paused".into(),
            revision: 1,
            lifecycle: AgentLifecycle::Paused,
            name: "Paused".into(),
            character: "vak".into(),
            personality: String::new(),
            behaviour: String::new(),
            responsibilities: String::new(),
            instructions: String::new(),
            animation: "off".into(),
            voice: "default".into(),
        };
        save(dir.path(), &[agent.clone()], true).expect("save paused agent");
        agent = load(dir.path()).expect("load paused agent").remove(0);
        assert_eq!(agent.lifecycle, AgentLifecycle::Paused);
        assert!(!agent.is_admissible());
    }

    #[test]
    fn edits_increment_saved_revision() {
        vak_config::paths::isolate_home_for_tests();
        let dir = tempfile::tempdir().expect("profile workspace");
        let profile = AgentDefinition {
            id: "pip".into(),
            revision: 1,
            lifecycle: AgentLifecycle::Active,
            name: "Pip".into(),
            character: "vak".into(),
            personality: "Warm".into(),
            behaviour: "Be useful".into(),
            responsibilities: String::new(),
            instructions: String::new(),
            animation: "subtle".into(),
            voice: "default".into(),
        };
        let mut untouched = profile.clone();
        untouched.id = "atlas".into();
        untouched.name = "Atlas".into();
        let saved =
            save(dir.path(), &[profile.clone(), untouched.clone()], true).expect("first save");
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
            character: "vak".into(),
            personality: String::new(),
            behaviour: String::new(),
            responsibilities: String::new(),
            instructions: String::new(),
            animation: "off".into(),
            voice: "default".into(),
        };
        let mut duplicate = profile.clone();
        duplicate.name = "Two".into();
        assert!(save(dir.path(), &[profile, duplicate], true).is_err());
    }

    #[test]
    fn builtin_templates_are_all_valid() {
        vak_config::paths::isolate_home_for_tests();
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

    fn agent(id: &str, name: &str, character: &str) -> AgentDefinition {
        AgentDefinition {
            id: id.into(),
            revision: 1,
            lifecycle: AgentLifecycle::Active,
            name: name.into(),
            character: character.into(),
            personality: String::new(),
            behaviour: String::new(),
            responsibilities: String::new(),
            instructions: String::new(),
            animation: "subtle".into(),
            voice: "default".into(),
        }
    }

    /// A layer as a 3.x install wrote it, with a character preset 4.0.0
    /// retired. `save` would refuse to write it, so it is written directly.
    fn layer_with_retired_character(dir: &Path) -> AgentDefinition {
        let retired = agent("researcher", "Research Analyst", "leaf");
        std::fs::create_dir_all(dir.join(".vak")).expect("agent config dir");
        std::fs::write(
            path(dir),
            serde_json::to_vec(&[&retired]).expect("stored layer"),
        )
        .expect("write stored layer");
        retired
    }

    #[test]
    fn an_agent_with_a_retired_value_does_not_block_creating_another() {
        vak_config::paths::isolate_home_for_tests();
        let dir = tempfile::tempdir().expect("agent workspace");
        let retired = layer_with_retired_character(dir.path());
        let created = agent("analyst", "Data Analyst", "tavi");
        let saved = save(dir.path(), &[retired, created], true)
            .expect("an untouched agent never blocks another");
        assert_eq!(saved.len(), 2);
        let loaded = load(dir.path()).expect("load agents");
        assert_eq!(loaded[0].character, "leaf", "carried through as stored");
        assert_eq!(loaded[1].id, "analyst");
    }

    #[test]
    fn editing_an_agent_judges_only_what_the_edit_changes() {
        vak_config::paths::isolate_home_for_tests();
        let dir = tempfile::tempdir().expect("agent workspace");
        let retired = layer_with_retired_character(dir.path());

        let mut paused = retired.clone();
        paused.lifecycle = AgentLifecycle::Paused;
        paused.personality = "Careful".into();
        save(dir.path(), &[paused.clone()], true)
            .expect("an edit that keeps the stored character is accepted");

        let mut renamed = paused.clone();
        renamed.character = "wave".into();
        let error = save(dir.path(), &[renamed], true)
            .expect_err("a changed character must be one this version offers");
        assert!(
            error.contains("Research Analyst") && error.contains("wave"),
            "the refusal names the agent and the value: {error}"
        );

        let mut fixed = paused;
        fixed.character = "moss".into();
        save(dir.path(), &[fixed], true).expect("a current character is accepted");
        assert_eq!(load(dir.path()).expect("load agents")[0].character, "moss");
    }

    #[test]
    fn a_new_agent_is_judged_in_full() {
        let dir = tempfile::tempdir().expect("agent workspace");
        let mut created = agent("analyst", "Data Analyst", "tavi");
        created.instructions = "x".repeat(8001);
        let error = save(dir.path(), &[created], true).expect_err("oversized instructions");
        assert!(error.contains("Data Analyst"), "{error}");
        assert!(load(dir.path()).expect("load agents").is_empty());
    }
}
