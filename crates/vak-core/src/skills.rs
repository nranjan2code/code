//! Skills: markdown packages with frontmatter, discovered from
//! `.vak/skills/<name>/SKILL.md` (project) and
//! `<home>/skills/<name>/SKILL.md` (user). Only names + descriptions enter
//! the system prompt; the model reads the file when it needs the content.

use std::path::{Path, PathBuf};

#[derive(Debug, Clone, PartialEq)]
pub struct Skill {
    pub name: String,
    pub description: String,
    pub path: PathBuf,
    pub provenance: Option<String>,
    pub shadowed: bool,
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
        "\nSkills available (these are documents only, never callable tools; if the task says `use <skill>`, first call `read` on that skill's exact absolute SKILL.md path, then apply it with ordinary tools):\n",
    );
    for sk in skills {
        s.push_str(&format!(
            "- SKILL `{}` (not a tool): {} — read {}\n",
            sk.name,
            if sk.description.is_empty() {
                "(no description)"
            } else {
                &sk.description
            },
            sk.path.display()
        ));
    }
    s
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
    fn prompt_identifies_skill_as_guidance_and_includes_exact_path() {
        let skill = Skill {
            name: "code-task".into(),
            description: "focused implementation".into(),
            path: std::path::PathBuf::from("/workspace/.vak/skills/code-task/SKILL.md"),
            provenance: None,
            shadowed: false,
        };
        let prompt = prompt_section(&[skill]);
        assert!(prompt.contains("if the task says `use <skill>`, first call `read`"));
        assert!(prompt.contains("SKILL `code-task` (not a tool)"));
        assert!(prompt.contains("/workspace/.vak/skills/code-task/SKILL.md"));
    }
}
