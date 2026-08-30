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
}

pub fn discover(cwd: &Path, home: &Path) -> Vec<Skill> {
    discover_with_plugins(cwd, home, &[])
}

pub fn discover_with_plugins(cwd: &Path, home: &Path, plugins: &[(PathBuf, String)]) -> Vec<Skill> {
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
    out.dedup_by(|a, b| a.name == b.name);
    out
}

pub fn parse(path: &Path) -> Option<Skill> {
    let text = std::fs::read_to_string(path).ok()?;
    let rest = text.strip_prefix("---")?;
    let (frontmatter, _) = rest.split_once("---")?;
    let mut name = None;
    let mut description = String::new();
    for line in frontmatter.lines() {
        let line = line.trim();
        if let Some(v) = line.strip_prefix("name:") {
            name = Some(v.trim().trim_matches('"').to_string());
        } else if let Some(v) = line.strip_prefix("description:") {
            description = v.trim().trim_matches('"').to_string();
        }
    }
    let name = name.or_else(|| {
        path.parent()
            .and_then(|p| p.file_name())
            .map(|n| n.to_string_lossy().into_owned())
    })?;
    if name.is_empty() {
        return None;
    }
    Some(Skill {
        name,
        description,
        path: path.to_path_buf(),
        provenance: None,
    })
}

pub fn prompt_section(skills: &[Skill]) -> String {
    if skills.is_empty() {
        return String::new();
    }
    let mut s = String::from(
        "\nSkills available (read the SKILL.md with the read tool before using one):\n",
    );
    for sk in skills {
        s.push_str(&format!(
            "- {}: {} ({})\n",
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
