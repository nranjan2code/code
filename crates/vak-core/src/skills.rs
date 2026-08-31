//! Skills: markdown packages with frontmatter, discovered from
//! `.vak/skills/<name>/SKILL.md` (project) and
//! `<home>/skills/<name>/SKILL.md` (user). Discovery metadata enters the
//! capability packet; the brokered `skill` loader returns full content.

use async_trait::async_trait;
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, PartialEq)]
pub struct Skill {
    pub name: String,
    pub description: String,
    pub path: PathBuf,
    pub provenance: Option<String>,
    pub shadowed: bool,
}

impl Skill {
    pub fn digest(&self) -> Result<String, std::io::Error> {
        let bytes = std::fs::read(&self.path)?;
        Ok(format!("{:x}", Sha256::digest(bytes)))
    }
}

#[derive(Debug, Clone)]
pub struct FrozenSkill {
    pub name: String,
    pub description: String,
    pub path: PathBuf,
    pub digest: String,
    pub provenance: Option<String>,
}

impl FrozenSkill {
    fn load(&self) -> Result<String, String> {
        let bytes = std::fs::read(&self.path).map_err(|error| {
            format!(
                r#"{{"type":"capability_unavailable","kind":"skill","name":{},"message":{}}}"#,
                json_string(&self.name),
                json_string(&error.to_string())
            )
        })?;
        let actual = format!("{:x}", Sha256::digest(&bytes));
        if actual != self.digest {
            return Err(format!(
                r#"{{"type":"capability_stale","kind":"skill","name":{},"message":"skill changed after session admission; start a new session"}}"#,
                json_string(&self.name)
            ));
        }
        let content = String::from_utf8(bytes).map_err(|error| {
            format!(
                r#"{{"type":"capability_invalid","kind":"skill","name":{},"message":{}}}"#,
                json_string(&self.name),
                json_string(&error.to_string())
            )
        })?;
        let body = strip_frontmatter(&content).trim();
        let base = self.path.parent().unwrap_or(Path::new("."));
        let provenance = self.provenance.as_deref().unwrap_or("workspace-or-user");
        Ok(format!(
            "<skill name=\"{}\" location=\"{}\" provenance=\"{}\">\nReferences are relative to {}.\n\n{}\n</skill>",
            self.name,
            self.path.display(),
            provenance,
            base.display(),
            body
        ))
    }
}

pub fn frozen_from_capabilities(
    capabilities: &[vak_session::types::CapabilityDescriptor],
) -> Vec<FrozenSkill> {
    capabilities
        .iter()
        .filter(|capability| capability.kind == vak_session::types::CapabilityKind::Skill)
        .filter_map(|capability| {
            Some(FrozenSkill {
                name: capability.name.clone(),
                description: capability.description.clone(),
                path: capability.source.clone()?,
                digest: capability.digest.clone()?,
                provenance: capability.provenance.clone(),
            })
        })
        .collect()
}

pub fn expand_invocation(input: &str, skills: &[FrozenSkill]) -> Result<Option<String>, String> {
    let trimmed = input.trim_start();
    let Some(rest) = trimmed.strip_prefix("/skill:") else {
        return Ok(None);
    };
    let mut parts = rest.splitn(2, char::is_whitespace);
    let name = parts.next().unwrap_or_default();
    let Some(skill) = skills.iter().find(|skill| skill.name == name) else {
        return Err(format!(
            r#"{{"type":"capability_not_admitted","kind":"skill","name":{}}}"#,
            json_string(name)
        ));
    };
    let block = skill.load()?;
    let args = parts.next().unwrap_or_default().trim();
    Ok(Some(if args.is_empty() {
        block
    } else {
        format!("{block}\n\n{args}")
    }))
}

#[derive(Debug, Clone)]
pub struct SkillTool {
    skills: BTreeMap<String, FrozenSkill>,
    description: String,
}

impl SkillTool {
    pub fn new(skills: impl IntoIterator<Item = FrozenSkill>) -> Self {
        let skills = skills
            .into_iter()
            .map(|skill| (skill.name.clone(), skill))
            .collect::<BTreeMap<_, _>>();
        let mut description = String::from(
            "Load one admitted skill document by name. Skills are instructions, not executable functions. Available skills:\n",
        );
        for skill in skills.values() {
            description.push_str(&format!("- {}: {}\n", skill.name, skill.description));
        }
        Self {
            skills,
            description,
        }
    }
}

#[async_trait]
impl vak_tools::Tool for SkillTool {
    fn name(&self) -> &str {
        "skill"
    }

    fn description(&self) -> &str {
        &self.description
    }

    fn schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "name": {
                    "type": "string",
                    "enum": self.skills.keys().collect::<Vec<_>>(),
                    "description": "Exact admitted skill name"
                }
            },
            "required": ["name"],
            "additionalProperties": false
        })
    }

    async fn execute(&self, args: &Value, ctx: &vak_tools::ToolContext) -> vak_tools::ToolOutput {
        let Some(name) = args.get("name").and_then(Value::as_str) else {
            return vak_tools::ToolOutput::error(
                r#"{"type":"invalid_arguments","capability":"skill","message":"missing required string 'name'"}"#,
            );
        };
        let Some(skill) = self.skills.get(name) else {
            return vak_tools::ToolOutput::error(format!(
                r#"{{"type":"capability_not_admitted","kind":"skill","name":{}}}"#,
                json_string(name)
            ));
        };
        match skill.load() {
            Ok(content) => vak_tools::ToolOutput::ok(ctx.truncate_output(content)),
            Err(error) => vak_tools::ToolOutput::error(error),
        }
    }

    fn claims(&self, _args: &Value) -> vak_tools::ResourceClaims {
        vak_tools::ResourceClaims {
            read_only: true,
            ..Default::default()
        }
    }
}

fn json_string(value: &str) -> String {
    serde_json::to_string(value).unwrap_or_else(|_| "\"invalid\"".into())
}

fn strip_frontmatter(content: &str) -> &str {
    let Some(rest) = content.strip_prefix("---") else {
        return content;
    };
    rest.split_once("\n---")
        .map_or(content, |(_, body)| body.trim_start_matches(['\r', '\n']))
}

pub fn discover(cwd: &Path, home: &Path) -> Vec<Skill> {
    discover_with_plugins(cwd, home, &[])
}

pub fn discover_with_plugins(cwd: &Path, home: &Path, plugins: &[(PathBuf, String)]) -> Vec<Skill> {
    discover_all_with_plugins(cwd, home, plugins)
        .into_iter()
        .filter(|skill| !skill.shadowed)
        .collect()
}

/// Discovers every valid skill, retaining lower-precedence entries so
/// inspection surfaces can explain why a skill is not active.
pub fn discover_all_with_plugins(
    cwd: &Path,
    home: &Path,
    plugins: &[(PathBuf, String)],
) -> Vec<Skill> {
    let mut roots = vec![(cwd.join(".vak/skills"), None), (home.join("skills"), None)];
    roots.extend(
        plugins
            .iter()
            .map(|(root, provenance)| (root.join("skills"), Some(provenance.clone()))),
    );
    roots.dedup_by(|a, b| a.0 == b.0);
    let mut out = Vec::new();
    for (root, provenance) in roots {
        let Ok(entries) = std::fs::read_dir(&root) else {
            continue;
        };
        for entry in entries.flatten() {
            let skill_path = entry.path().join("SKILL.md");
            if !skill_path.is_file() {
                continue;
            }
            if let Some(mut skill) = parse(&skill_path) {
                skill.provenance = provenance.clone();
                out.push(skill);
            }
        }
    }
    out.sort_by(|a, b| a.name.cmp(&b.name));
    let mut seen = std::collections::HashSet::new();
    for skill in &mut out {
        skill.shadowed = !seen.insert(skill.name.clone());
    }
    out
}

pub fn parse(path: &Path) -> Option<Skill> {
    validate(path).ok().map(|(skill, _)| skill)
}

pub fn validate(path: &Path) -> Result<(Skill, Vec<String>), String> {
    let text = std::fs::read_to_string(path).map_err(|error| error.to_string())?;
    let rest = text
        .strip_prefix("---")
        .ok_or_else(|| "file must begin with YAML frontmatter delimiter ---".to_string())?;
    let (frontmatter, _) = rest
        .split_once("\n---")
        .ok_or_else(|| "frontmatter is missing its closing --- delimiter".to_string())?;
    let mut name = None;
    let mut description = None;
    let mut compatibility = None;
    let mut warnings = Vec::new();
    for line in frontmatter.lines() {
        let line = line.trim();
        if let Some(v) = line.strip_prefix("name:") {
            name = Some(v.trim().trim_matches('"').to_string());
        } else if let Some(v) = line.strip_prefix("description:") {
            description = Some(v.trim().trim_matches('"').to_string());
        } else if let Some(v) = line.strip_prefix("compatibility:") {
            compatibility = Some(v.trim().trim_matches('"').to_string());
        } else if line.starts_with("allowed-tools:") {
            warnings.push("allowed-tools is advisory and never grants authorization".into());
        }
    }
    let name = name.ok_or_else(|| "frontmatter requires name".to_string())?;
    if !valid_name(&name) {
        return Err(format!(
            "name '{name}' is not valid lowercase kebab-case (1-64 chars)"
        ));
    }
    let description = description.ok_or_else(|| "frontmatter requires description".to_string())?;
    if description.trim().is_empty() {
        return Err("description must not be empty".into());
    }
    if description.chars().count() > 1024 {
        return Err("description must be at most 1024 characters".into());
    }
    if compatibility.as_deref().is_some_and(str::is_empty) {
        return Err("compatibility must not be empty when provided".into());
    }
    Ok((
        Skill {
            name,
            description,
            path: path.to_path_buf(),
            provenance: None,
            shadowed: false,
        },
        warnings,
    ))
}

fn valid_name(name: &str) -> bool {
    !name.is_empty()
        && name.len() <= 64
        && !name.starts_with('-')
        && !name.ends_with('-')
        && !name.contains("--")
        && name
            .chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-')
}

pub fn prompt_section(skills: &[Skill]) -> String {
    if skills.is_empty() {
        return String::new();
    }
    let mut s = String::from(
        "\nSkills available. Load instructions with the `skill` tool using the exact name; then use ordinary tools to perform the work:\n",
    );
    for sk in skills {
        s.push_str(&format!(
            "- `{}`: {}\n",
            sk.name,
            if sk.description.is_empty() {
                "(no description)"
            } else {
                &sk.description
            }
        ));
    }
    s
}

pub fn prompt_section_from_capabilities(
    capabilities: &[vak_session::types::CapabilityDescriptor],
) -> String {
    let skills = capabilities
        .iter()
        .filter(|capability| capability.kind == vak_session::types::CapabilityKind::Skill)
        .collect::<Vec<_>>();
    if skills.is_empty() {
        return String::new();
    }
    let mut out = String::from(
        "\nSkills available. Load instructions with the `skill` tool using the exact name; then use ordinary tools to perform the work:\n",
    );
    for skill in skills {
        out.push_str(&format!("- `{}`: {}\n", skill.name, skill.description));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parser_requires_standard_name_and_description() -> Result<(), Box<dyn std::error::Error>> {
        let dir = tempfile::tempdir()?;
        let path = dir.path().join("SKILL.md");
        std::fs::write(&path, "---\nname: Good_Name\ndescription: bad\n---\nbody")?;
        assert!(parse(&path).is_none());
        std::fs::write(
            &path,
            "---\nname: good-name\ndescription: useful\n---\nbody",
        )?;
        let skill =
            parse(&path).ok_or_else(|| std::io::Error::other("valid skill should parse"))?;
        assert_eq!(skill.name, "good-name");
        Ok(())
    }

    #[test]
    fn discovery_marks_lower_precedence_duplicates_shadowed()
    -> Result<(), Box<dyn std::error::Error>> {
        let dir = tempfile::tempdir()?;
        let project = dir.path().join(".vak/skills/demo");
        let home = dir.path().join("home/skills/demo");
        std::fs::create_dir_all(&project)?;
        std::fs::create_dir_all(&home)?;
        let body = "---\nname: demo\ndescription: demo skill\n---\nbody";
        std::fs::write(project.join("SKILL.md"), body)?;
        std::fs::write(home.join("SKILL.md"), body)?;
        let all = discover_all_with_plugins(dir.path(), &dir.path().join("home"), &[]);
        assert_eq!(all.len(), 2);
        assert!(!all[0].shadowed);
        assert!(all[1].shadowed);
        assert_eq!(discover(dir.path(), &dir.path().join("home")).len(), 1);
        Ok(())
    }

    #[test]
    fn validation_reports_advisory_fields_and_rejects_long_descriptions()
    -> Result<(), Box<dyn std::error::Error>> {
        let dir = tempfile::tempdir()?;
        let path = dir.path().join("SKILL.md");
        std::fs::write(
            &path,
            "---\nname: safe-skill\ndescription: useful\nallowed-tools: Bash\n---\nbody",
        )?;
        let (_, warnings) = validate(&path).map_err(std::io::Error::other)?;
        assert_eq!(
            warnings,
            vec!["allowed-tools is advisory and never grants authorization"]
        );
        std::fs::write(
            &path,
            format!(
                "---\nname: safe-skill\ndescription: {}\n---\nbody",
                "x".repeat(1025)
            ),
        )?;
        assert!(validate(&path).is_err());
        Ok(())
    }

    #[test]
    fn prompt_advertises_the_typed_skill_loader_without_leaking_paths() {
        let skill = Skill {
            name: "code-task".into(),
            description: "focused implementation".into(),
            path: std::path::PathBuf::from("/workspace/.vak/skills/code-task/SKILL.md"),
            provenance: None,
            shadowed: false,
        };
        let prompt = prompt_section(&[skill]);
        assert!(prompt.contains("`skill` tool using the exact name"));
        assert!(prompt.contains("`code-task`: focused implementation"));
        assert!(!prompt.contains("/workspace/.vak/skills"));
    }

    #[tokio::test]
    async fn skill_tool_loads_only_the_frozen_digest() -> Result<(), Box<dyn std::error::Error>> {
        use vak_tools::Tool;

        let dir = tempfile::tempdir()?;
        let path = dir.path().join("SKILL.md");
        std::fs::write(
            &path,
            "---\nname: code-task\ndescription: focused implementation\n---\nUse table-driven tests.",
        )?;
        let skill = parse(&path).ok_or("skill should parse")?;
        let digest = skill.digest()?;
        let tool = SkillTool::new([FrozenSkill {
            name: skill.name,
            description: skill.description,
            path: path.clone(),
            digest,
            provenance: None,
        }]);
        let ctx = vak_tools::ToolContext {
            cwd: dir.path().to_path_buf(),
            cancel: tokio_util::sync::CancellationToken::new(),
            limits: Default::default(),
            sandbox: None,
        };
        let loaded = tool
            .execute(&serde_json::json!({"name": "code-task"}), &ctx)
            .await;
        assert!(!loaded.is_error);
        assert!(loaded.content.contains("Use table-driven tests."));
        assert!(loaded.content.contains("References are relative to"));

        std::fs::write(&path, "changed after admission")?;
        let stale = tool
            .execute(&serde_json::json!({"name": "code-task"}), &ctx)
            .await;
        assert!(stale.is_error);
        assert!(stale.content.contains("capability_stale"));
        Ok(())
    }

    #[test]
    fn explicit_skill_command_expands_before_model_dispatch()
    -> Result<(), Box<dyn std::error::Error>> {
        let dir = tempfile::tempdir()?;
        let path = dir.path().join("SKILL.md");
        std::fs::write(
            &path,
            "---\nname: code-task\ndescription: focused implementation\n---\nFollow the workflow.",
        )?;
        let skill = parse(&path).ok_or("skill should parse")?;
        let digest = skill.digest()?;
        let frozen = FrozenSkill {
            name: skill.name,
            description: skill.description,
            path,
            digest,
            provenance: None,
        };
        let expanded = expand_invocation("/skill:code-task fix parser", &[frozen])?
            .ok_or("command should expand")?;
        assert!(expanded.contains("<skill name=\"code-task\""));
        assert!(expanded.contains("Follow the workflow."));
        assert!(expanded.ends_with("fix parser"));
        Ok(())
    }
}
