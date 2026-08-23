//! Learning loop (docs/design/26-learning.md): the `remember` tool appends
//! durable notes; `propose_skill` queues skill drafts for human promotion.
//! Proposals never enter discovery by themselves — promotion is an explicit
//! human action over HTTP or CLI.

use std::path::{Path, PathBuf};

use serde_json::Value;

use crate::memory;

const KINDS: [&str; 4] = ["fact", "decision", "preference", "reference"];

// ---- remember ---------------------------------------------------------------

pub struct RememberTool {
    pub sessions_home: PathBuf,
    pub cwd: PathBuf,
    pub session_id: String,
}

#[async_trait::async_trait]
impl vak_tools::Tool for RememberTool {
    fn name(&self) -> &str {
        "remember"
    }

    fn description(&self) -> &str {
        "Persist a durable note about this workspace for FUTURE sessions \
         (decisions, facts, preferences, pointers). Use sparingly for things \
         worth remembering after this conversation ends — not transient \
         details. Notes are recalled via session_search and are visible to \
         the user, who can edit them."
    }

    fn schema(&self) -> Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "note": {"type": "string", "description": "The content to persist"},
                "kind": {"type": "string", "enum": KINDS.to_vec(),
                         "description": "One of fact/decision/preference/reference (default fact)"},
                "tag":  {"type": "string", "description": "Short slug for grouping, e.g. 'deploy-rollbacks'"}
            },
            "required": ["note"]
        })
    }

    async fn execute(&self, args: &Value, _ctx: &vak_tools::ToolContext) -> vak_tools::ToolOutput {
        let Some(note) = args.get("note").and_then(Value::as_str).map(str::trim) else {
            return vak_tools::ToolOutput::error("missing required argument 'note'");
        };
        let kind = args.get("kind").and_then(Value::as_str).unwrap_or("fact");
        if !KINDS.contains(&kind) {
            return vak_tools::ToolOutput::error(format!(
                "unknown kind '{kind}'; expected one of {KINDS:?}"
            ));
        }
        let tag = args.get("tag").and_then(Value::as_str).unwrap_or("");
        match memory::append_note(
            &self.sessions_home,
            &self.cwd,
            kind,
            tag,
            &self.session_id,
            note,
        ) {
            Ok(_) => vak_tools::ToolOutput::ok(format!(
                "remembered ({kind}{tag_suffix}). It will surface in future session_search queries.",
                tag_suffix = if tag.is_empty() {
                    String::new()
                } else {
                    format!(", tag '{tag}'")
                }
            )),
            Err(e) => vak_tools::ToolOutput::error(format!("could not persist note: {e}")),
        }
    }

    fn claims(&self, _args: &Value) -> vak_tools::ResourceClaims {
        vak_tools::ResourceClaims {
            exclusive: true,
            read_only: false,
            paths: vec![],
        }
    }
}

// ---- propose_skill ----------------------------------------------------------

pub struct SkillProposal {
    pub id: String,
    pub name: String,
    pub description: String,
    pub path: PathBuf,
}

fn proposals_dir(home: &Path, cwd: &Path) -> PathBuf {
    home.join("skill-proposals").join(memory::hash_cwd(cwd))
}

pub struct ProposeSkillTool {
    pub sessions_home: PathBuf,
    pub cwd: PathBuf,
    pub session_id: String,
}

#[async_trait::async_trait]
impl vak_tools::Tool for ProposeSkillTool {
    fn name(&self) -> &str {
        "propose_skill"
    }

    fn description(&self) -> &str {
        "Draft a reusable SKILL from something learned this session (a \
         procedure that worked, a gotcha and its fix). Goes to a review \
         queue — it becomes available to future runs ONLY after the user \
         promotes it. Do not propose one-off steps."
    }

    fn schema(&self) -> Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "name": {"type": "string",
                         "description": "kebab-case identifier, e.g. 'rotate-release-tags'"},
                "description": {"type": "string",
                                "description": "One line: what it is for and when to use it"},
                "instructions": {"type": "string",
                                 "description": "Markdown procedure the future agent should follow"}
            },
            "required": ["name", "description", "instructions"]
        })
    }

    async fn execute(&self, args: &Value, _ctx: &vak_tools::ToolContext) -> vak_tools::ToolOutput {
        let Some(name) = sanitize_name(args.get("name").and_then(Value::as_str).unwrap_or(""))
        else {
            return vak_tools::ToolOutput::error(
                "'name' must be kebab-case (lowercase letters, digits, dashes)",
            );
        };
        let Some(description) = args
            .get("description")
            .and_then(Value::as_str)
            .map(str::trim)
        else {
            return vak_tools::ToolOutput::error("missing required argument 'description'");
        };
        let Some(instructions) = args
            .get("instructions")
            .and_then(Value::as_str)
            .map(str::trim)
        else {
            return vak_tools::ToolOutput::error("missing required argument 'instructions'");
        };
        if description.is_empty() || instructions.is_empty() {
            return vak_tools::ToolOutput::error(
                "'description' and 'instructions' must not be empty",
            );
        }

        let id = uuid::Uuid::now_v7().simple().to_string();
        let dir = proposals_dir(&self.sessions_home, &self.cwd);
        if let Err(e) = std::fs::create_dir_all(&dir) {
            return vak_tools::ToolOutput::error(format!("create proposals dir: {e}"));
        }
        let body = format!(
            "---\nname: \"{name}\"\ndescription: \"{desc}\"\n---\n\n{instr}\n\n<!-- proposed-by: {sid} at {ts}; proposal id {id} -->\n",
            desc = description.replace('"', "'"),
            instr = instructions,
            sid = self.session_id,
            ts = chrono::Utc::now().to_rfc3339(),
        );
        let path = dir.join(format!("{id}.md"));
        if let Err(e) = std::fs::write(&path, body) {
            return vak_tools::ToolOutput::error(format!("write proposal: {e}"));
        }
        vak_tools::ToolOutput::ok(format!(
            "skill '{name}' queued for review as proposal {id}. It will NOT be \
             available until the user promotes it."
        ))
    }

    fn claims(&self, _args: &Value) -> vak_tools::ResourceClaims {
        vak_tools::ResourceClaims {
            exclusive: true,
            read_only: false,
            paths: vec![],
        }
    }
}

/// Kebab-case enforcement: lowercase letters/digits/dashes only, at least
/// one letter, no leading/trailing dash.
pub fn sanitize_name(raw: &str) -> Option<String> {
    let name = raw.trim().to_lowercase();
    if name.is_empty()
        || name.len() > 64
        || !name
            .chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-')
        || name.starts_with('-')
        || name.ends_with('-')
        || !name.chars().any(|c| c.is_ascii_alphabetic())
    {
        return None;
    }
    Some(name)
}

// ---- Review queue API -------------------------------------------------------

pub fn list_proposals(home: &Path, cwd: &Path) -> Vec<SkillProposal> {
    let mut out = Vec::new();
    let Ok(entries) = std::fs::read_dir(proposals_dir(home, cwd)) else {
        return out;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        let Some(id) = path.file_stem().and_then(|s| s.to_str()).map(String::from) else {
            continue;
        };
        let Some(skill) = crate::skills::parse(&path) else {
            continue;
        };
        out.push(SkillProposal {
            id,
            name: skill.name,
            description: skill.description,
            path,
        });
    }
    out.sort_by(|a, b| b.id.cmp(&a.id));
    out
}

/// Install a proposal into user-level discovery. Refuses to silently
/// overwrite an existing skill of the same name.
pub fn promote(home: &Path, cwd: &Path, id: &str) -> Result<String, String> {
    let proposals = list_proposals(home, cwd);
    let p = proposals
        .iter()
        .find(|p| p.id == id)
        .ok_or_else(|| format!("no proposal '{id}'"))?;
    let target_dir = home.join("skills").join(&p.name);
    let target = target_dir.join("SKILL.md");
    if target.exists() {
        return Err(format!(
            "a skill named '{}' already exists at {}; remove or rename it first",
            p.name,
            target.display()
        ));
    }
    std::fs::create_dir_all(&target_dir).map_err(|e| format!("create skill dir: {e}"))?;
    std::fs::copy(&p.path, &target).map_err(|e| format!("install skill: {e}"))?;
    std::fs::remove_file(&p.path).map_err(|e| format!("remove pending file: {e}"))?;
    Ok(p.name.clone())
}

pub fn reject(home: &Path, cwd: &Path, id: &str) -> Result<(), String> {
    let proposals = list_proposals(home, cwd);
    let p = proposals
        .iter()
        .find(|p| p.id == id)
        .ok_or_else(|| format!("no proposal '{id}'"))?;
    std::fs::remove_file(&p.path).map_err(|e| format!("remove proposal: {e}"))
}
