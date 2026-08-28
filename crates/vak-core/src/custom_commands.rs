//! Custom slash commands: markdown prompt templates discovered from
//! `.vak/commands/*.md` (project), `<home>/commands/*.md` (user), and
//! `.vak/plugins/<plugin>/commands/*.md` (plugin-contributed palette
//! actions). Project wins over plugin wins over user on name collision.

use std::path::Path;

#[derive(Debug, Clone, PartialEq)]
pub struct CustomCommand {
    pub name: String,
    pub description: String,
    /// Full markdown body; `$ARGUMENTS` is substituted at invocation.
    pub template: String,
    pub source: String,
}

pub fn discover(cwd: &Path, home: &Path) -> Vec<CustomCommand> {
    let mut out = Vec::new();
    let mut roots: Vec<(std::path::PathBuf, String)> = vec![
        (cwd.join(".vak/plugins"), String::new()),
        (cwd.join(".vak/commands"), "project".to_string()),
        (home.join("commands"), "user".to_string()),
    ];
    roots.dedup();
    // Plugin namespaces first so project/user layers can shadow them.
    if let Ok(entries) = std::fs::read_dir(&roots[0].0) {
        let mut plugins: Vec<std::path::PathBuf> = entries
            .flatten()
            .map(|e| e.path())
            .filter(|p| p.is_dir())
            .collect();
        plugins.sort();
        for plugin in plugins {
            let Some(name) = plugin.file_name().map(|n| n.to_string_lossy().into_owned()) else {
                continue;
            };
            collect_dir(
                &plugin.join("commands"),
                &format!("plugin:{name}"),
                &mut out,
            );
        }
    }
    for (root, label) in roots.iter().skip(1) {
        collect_dir(root, label, &mut out);
    }
    out.sort_by(|a, b| a.name.cmp(&b.name));
    out.dedup_by(|a, b| a.name == b.name);
    out
}

fn collect_dir(dir: &Path, source: &str, out: &mut Vec<CustomCommand>) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if !path.is_file() {
            continue;
        }
        let Some(name) = path.file_stem().map(|n| n.to_string_lossy().into_owned()) else {
            continue;
        };
        if !valid_name(&name) || path.extension().is_none_or(|e| e != "md") {
            continue;
        }
        if let Some(cmd) = parse(&path, &name, source) {
            out.push(cmd);
        }
    }
}

fn valid_name(name: &str) -> bool {
    !name.is_empty()
        && name
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_'))
}

fn parse(path: &Path, name: &str, source: &str) -> Option<CustomCommand> {
    let text = std::fs::read_to_string(path).ok()?;
    let mut body = text.as_str();
    let mut description = String::new();
    if let Some(rest) = text.strip_prefix("---")
        && let Some((frontmatter, remainder)) = rest.split_once("---")
    {
        for line in frontmatter.lines() {
            if let Some(v) = line.trim().strip_prefix("description:") {
                description = v.trim().trim_matches('"').to_string();
            }
        }
        body = remainder;
    }
    if description.is_empty() {
        for line in body.lines() {
            let trimmed = line.trim().trim_start_matches('#').trim_start();
            if trimmed.is_empty() {
                continue;
            }
            description = trimmed.trim_start_matches('>').trim().to_string();
            break;
        }
    }
    Some(CustomCommand {
        name: name.to_string(),
        description,
        template: body.trim().to_string(),
        source: source.to_string(),
    })
}

/// Substitutes `$ARGUMENTS` with the invocation arguments; when the
/// template has no placeholder and args are present they are appended so
/// the payload is never silently dropped.
pub fn expand(template: &str, args: &str) -> String {
    let args = args.trim();
    if template.contains("$ARGUMENTS") {
        return template.replace("$ARGUMENTS", args);
    }
    if args.is_empty() {
        return template.to_string();
    }
    format!("{template}\n\nArguments: {args}")
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;

    #[test]
    fn expand_substitutes_and_appends_arguments() {
        assert_eq!(
            expand("fix $ARGUMENTS please", "the parser"),
            "fix the parser please"
        );
        assert_eq!(expand("no placeholder here", ""), "no placeholder here");
        assert_eq!(
            expand("no placeholder", "extra context"),
            "no placeholder\n\nArguments: extra context"
        );
    }

    #[test]
    fn discovery_precedence_project_over_user_and_valid_names_only() {
        let dir = tempfile::tempdir().unwrap();
        let project = dir.path().join(".vak/commands");
        let user = dir.path().join("home/commands");
        std::fs::create_dir_all(&project).unwrap();
        std::fs::create_dir_all(&user).unwrap();
        std::fs::write(
            project.join("review.md"),
            "---\ndescription: project review\n---\nReview $ARGUMENTS",
        )
        .unwrap();
        std::fs::write(user.join("review.md"), "# user review\nBody").unwrap();
        std::fs::write(user.join("bad name.md"), "skipped").unwrap();

        let cmds = discover(dir.path(), &dir.path().join("home"));
        assert_eq!(cmds.len(), 1, "{cmds:?}");
        assert_eq!(cmds[0].name, "review");
        assert_eq!(cmds[0].source, "project");
        assert_eq!(cmds[0].description, "project review");
        assert_eq!(cmds[0].template, "Review $ARGUMENTS");
    }

    #[test]
    fn plugin_commands_are_discovered_with_namespace_label() {
        let dir = tempfile::tempdir().unwrap();
        let plugin = dir.path().join(".vak/plugins/acme/commands");
        std::fs::create_dir_all(&plugin).unwrap();
        std::fs::write(plugin.join("deploy.md"), "# Ship it\nAll steps").unwrap();

        let cmds = discover(dir.path(), &dir.path().join("home"));
        assert_eq!(cmds.len(), 1);
        assert_eq!(cmds[0].source, "plugin:acme");
        assert_eq!(cmds[0].name, "deploy");
        assert_eq!(cmds[0].description, "Ship it");
    }
}
