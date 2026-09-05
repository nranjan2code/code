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
    let mut commands = discover_with_plugins(cwd, home, &[]);
    // Standalone inspection preserves the historical local plugin view. Core
    // turn admission never uses this convenience path; it supplies only
    // package roots returned by the enabled-plugin store.
    if let Ok(entries) = std::fs::read_dir(cwd.join(".vak/plugins")) {
        for entry in entries.flatten().filter(|entry| entry.path().is_dir()) {
            let Some(name) = entry.file_name().to_str().map(str::to_string) else {
                continue;
            };
            collect_dir(
                &entry.path().join("commands"),
                &format!("plugin:{name}"),
                &mut commands,
            );
        }
    }
    commands.sort_by(|a, b| a.name.cmp(&b.name).then(a.source.cmp(&b.source)));
    commands.dedup_by(|a, b| a.name == b.name);
    commands
}

pub fn discover_with_plugins(
    cwd: &Path,
    home: &Path,
    plugins: &[(std::path::PathBuf, String)],
) -> Vec<CustomCommand> {
    let mut out = Vec::new();
    let mut roots: Vec<(std::path::PathBuf, String)> = vec![
        (cwd.join(".vak/plugins"), String::new()),
        (cwd.join(".vak/commands"), "project".to_string()),
        (home.join("commands"), "user".to_string()),
    ];
    roots.dedup();
    // Plugin roots are supplied by the enabled-plugin resolver. Never scan
    // `.vak/plugins` directly: an unpacked or disabled package is not a
    // capability source.
    for (root, provenance) in plugins {
        collect_dir(
            &root.join("commands"),
            &format!("{provenance}:commands"),
            &mut out,
        );
    }
    for (root, label) in roots.iter().skip(1).rev() {
        collect_dir(root, label, &mut out);
    }
    out.sort_by(|a, b| {
        let rank = |source: &str| {
            if source == "project" {
                0
            } else if source.starts_with("plugin:") {
                1
            } else {
                2
            }
        };
        a.name
            .cmp(&b.name)
            .then(rank(&a.source).cmp(&rank(&b.source)))
    });
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

/// Expands a leading admitted `/command` invocation from the frozen
/// capability packet. Unknown slash-prefixed text is left untouched.
pub fn expand_capability_invocation(
    capabilities: &[vak_session::CapabilityDescriptor],
    input: &str,
) -> Option<String> {
    let trimmed = input.trim_start();
    let rest = trimmed.strip_prefix('/')?;
    let mut parts = rest.splitn(2, char::is_whitespace);
    let name = parts.next().filter(|name| !name.is_empty())?;
    let command = capabilities.iter().find(|capability| {
        capability.kind == vak_session::CapabilityKind::Command && capability.name == name
    })?;
    let template = command
        .configuration
        .get("template")
        .and_then(serde_json::Value::as_str)?;
    Some(expand(template, parts.next().unwrap_or_default()))
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

    #[test]
    fn invocation_expands_only_admitted_commands() {
        let commands = vec![vak_session::CapabilityDescriptor {
            name: "review".into(),
            kind: vak_session::CapabilityKind::Command,
            invocation: vak_session::CapabilityInvocation::UserCommand,
            description: "Review changes".into(),
            source: None,
            digest: None,
            provenance: Some("user".into()),
            configuration: serde_json::json!({"template": "Review $ARGUMENTS"}),
        }];
        assert_eq!(
            expand_capability_invocation(&commands, "/review src/lib.rs"),
            Some("Review src/lib.rs".into())
        );
        assert_eq!(
            expand_capability_invocation(&commands, "/unknown hello"),
            None
        );
    }

    #[test]
    fn turn_discovery_ignores_unmanaged_plugin_directory() {
        let dir = tempfile::tempdir().unwrap();
        let plugin = dir.path().join(".vak/plugins/ghost/commands");
        std::fs::create_dir_all(&plugin).unwrap();
        std::fs::write(plugin.join("ghost.md"), "# Ghost\nBody").unwrap();
        let commands = discover_with_plugins(dir.path(), &dir.path().join("home"), &[]);
        assert!(commands.is_empty());
    }

    #[test]
    fn project_command_wins_over_enabled_plugin_command() {
        let dir = tempfile::tempdir().unwrap();
        let project = dir.path().join(".vak/commands");
        let package = dir.path().join("package");
        std::fs::create_dir_all(&project).unwrap();
        std::fs::create_dir_all(package.join("commands")).unwrap();
        std::fs::write(
            project.join("review.md"),
            "---\ndescription: Project\n---\nProject",
        )
        .unwrap();
        std::fs::write(
            package.join("commands/review.md"),
            "---\ndescription: Plugin\n---\nPlugin",
        )
        .unwrap();
        let commands = discover_with_plugins(
            dir.path(),
            &dir.path().join("home"),
            &[(package, "plugin:acme:trace".into())],
        );
        assert_eq!(commands[0].source, "project");
    }
}
