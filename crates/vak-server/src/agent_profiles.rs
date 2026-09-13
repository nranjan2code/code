//! Durable user-facing agent character profiles.
//!
//! Profiles describe presentation and prompt preferences. They never grant
//! tools, permissions, credentials, budget, or a wider execution scope.

use std::{
    collections::HashSet,
    path::{Path, PathBuf},
};

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentProfile {
    pub id: String,
    #[serde(default = "default_revision")]
    pub revision: u64,
    pub name: String,
    pub character: String,
    pub personality: String,
    pub behaviour: String,
    #[serde(default)]
    pub responsibilities: String,
    pub animation: String,
    pub voice: String,
}

fn default_revision() -> u64 {
    1
}

impl AgentProfile {
    pub(crate) fn identity(&self) -> vak_session::types::AgentIdentity {
        vak_session::types::AgentIdentity {
            id: self.id.clone(),
            revision: self.revision,
            name: self.name.clone(),
            personality: self.personality.clone(),
            behaviour: self.behaviour.clone(),
            responsibilities: self.responsibilities.clone(),
        }
    }
}

pub(crate) fn effective(core: &vak_core::Core) -> Result<Vec<AgentProfile>, String> {
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
    cwd.join(".vak").join("agent-profiles.json")
}

pub fn load(cwd: &Path) -> Result<Vec<AgentProfile>, String> {
    let file = path(cwd);
    let raw = match std::fs::read_to_string(&file) {
        Ok(raw) => raw,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(e) => return Err(format!("cannot read {}: {e}", file.display())),
    };
    serde_json::from_str(&raw).map_err(|e| format!("invalid agent profiles: {e}"))
}

pub fn save(cwd: &Path, profiles: &[AgentProfile]) -> Result<Vec<AgentProfile>, String> {
    if profiles.len() > 100 {
        return Err("at most 100 agent profiles are allowed".into());
    }
    let mut ids = HashSet::with_capacity(profiles.len());
    for profile in profiles {
        if profile.id == "vak" {
            return Err("Vak is the built-in agent; choose another id".into());
        }
        if profile.id.trim().is_empty() || profile.name.trim().is_empty() {
            return Err("agent profile id and name are required".into());
        }
        if !ids.insert(profile.id.clone()) {
            return Err(format!("agent profile id '{}' is duplicated", profile.id));
        }
        if profile.name.len() > 120
            || profile.personality.len() > 4000
            || profile.behaviour.len() > 4000
            || profile.responsibilities.len() > 2000
        {
            return Err(format!("agent profile '{}' is too large", profile.id));
        }
        if !matches!(profile.animation.as_str(), "subtle" | "expressive" | "off") {
            return Err("animation must be subtle, expressive, or off".into());
        }
        if profile.voice.len() > 80 {
            return Err(format!(
                "agent profile '{}' voice setting is too large",
                profile.id
            ));
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
        .open(dir.join("agent-profiles.lock"))
        .map_err(|e| e.to_string())?;
    lock.try_lock()
        .map_err(|e| format!("agent profiles are being edited; retry: {e}"))?;
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
        Ok(raw) => {
            serde_json::from_str(&raw).map_err(|e| format!("invalid agent profiles: {e}"))?
        }
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
    Ok(next)
}

#[cfg(test)]
#[allow(clippy::expect_used)]
mod tests {
    use super::*;

    #[test]
    fn profiles_round_trip_atomically() {
        let dir = tempfile::tempdir().expect("profile workspace");
        let profiles = vec![AgentProfile {
            id: "pip".into(),
            revision: 1,
            name: "Pip".into(),
            character: "spark".into(),
            personality: "Warm".into(),
            behaviour: "Be useful".into(),
            responsibilities: String::new(),
            animation: "subtle".into(),
            voice: "default".into(),
        }];
        save(dir.path(), &profiles).expect("save profiles");
        assert_eq!(load(dir.path()).expect("load profiles")[0].name, "Pip");
    }

    #[test]
    fn invalid_character_is_rejected() {
        let dir = tempfile::tempdir().expect("profile workspace");
        let profile = AgentProfile {
            id: "x".into(),
            revision: 1,
            name: "X".into(),
            character: "unknown".into(),
            personality: String::new(),
            behaviour: String::new(),
            responsibilities: String::new(),
            animation: "off".into(),
            voice: "default".into(),
        };
        assert!(save(dir.path(), &[profile]).is_err());
    }

    #[test]
    fn invalid_voice_is_rejected() {
        let dir = tempfile::tempdir().expect("profile workspace");
        let profile = AgentProfile {
            id: "x".into(),
            revision: 1,
            name: "X".into(),
            character: "orb".into(),
            personality: String::new(),
            behaviour: String::new(),
            responsibilities: String::new(),
            animation: "off".into(),
            voice: "unknown".into(),
        };
        assert!(save(dir.path(), &[profile]).is_err());
    }

    #[test]
    fn edits_increment_saved_revision() {
        let dir = tempfile::tempdir().expect("profile workspace");
        let profile = AgentProfile {
            id: "pip".into(),
            revision: 1,
            name: "Pip".into(),
            character: "orb".into(),
            personality: "Warm".into(),
            behaviour: "Be useful".into(),
            responsibilities: String::new(),
            animation: "subtle".into(),
            voice: "default".into(),
        };
        let mut untouched = profile.clone();
        untouched.id = "atlas".into();
        untouched.name = "Atlas".into();
        let saved = save(dir.path(), &[profile.clone(), untouched.clone()]).expect("first save");
        assert_eq!(saved[0].revision, 1);
        let mut edited = profile;
        edited.personality = "Warm and direct".into();
        let saved = save(dir.path(), &[edited, untouched]).expect("edited save");
        assert_eq!(saved[0].revision, 2);
        assert_eq!(saved[1].revision, 1);
    }

    #[test]
    fn duplicate_ids_are_rejected() {
        let dir = tempfile::tempdir().expect("profile workspace");
        let profile = AgentProfile {
            id: "same".into(),
            revision: 1,
            name: "One".into(),
            character: "orb".into(),
            personality: String::new(),
            behaviour: String::new(),
            responsibilities: String::new(),
            animation: "off".into(),
            voice: "default".into(),
        };
        let mut duplicate = profile.clone();
        duplicate.name = "Two".into();
        assert!(save(dir.path(), &[profile, duplicate]).is_err());
    }
}
