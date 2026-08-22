use std::fs;
use std::path::Path;

const MAX_RESULTS: usize = 50;
const MAX_DEPTH: usize = 2;
const SKIP_DIRS: [&str; 5] = [".git", "target", "node_modules", "dist", ".vakcoder"];

#[derive(Debug, Clone)]
pub struct Suggestion {
    pub replace: String,
    pub hint: String,
}

pub fn complete(input: &str, commands: &[(&str, &str)], cwd: &Path) -> Vec<Suggestion> {
    let trimmed = input.trim_start();
    if trimmed.starts_with('/') {
        return complete_command(trimmed, commands);
    }
    if let Some(prefix) = trimmed
        .split_whitespace()
        .next_back()
        .and_then(|token| token.strip_prefix('@'))
    {
        return complete_path(prefix, cwd);
    }
    Vec::new()
}

fn complete_command(input: &str, commands: &[(&str, &str)]) -> Vec<Suggestion> {
    let token = input.split_whitespace().next().unwrap_or(input);
    let Some(prefix) = token.strip_prefix('/') else {
        return Vec::new();
    };
    let mut out: Vec<Suggestion> = commands
        .iter()
        .filter(|(name, _)| name.starts_with(prefix))
        .map(|(name, desc)| Suggestion {
            replace: format!("/{name} "),
            hint: (*desc).to_string(),
        })
        .collect();
    out.sort_by(|a, b| a.replace.cmp(&b.replace));
    out
}

fn complete_path(prefix: &str, cwd: &Path) -> Vec<Suggestion> {
    let allow_dot = prefix.starts_with('.');
    let mut candidates: Vec<(String, bool)> = Vec::new();
    walk(cwd, cwd, 1, allow_dot, &mut candidates);
    candidates.retain(|(rel, _)| rel.starts_with(prefix));
    candidates.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.cmp(&b.0)));
    candidates.truncate(MAX_RESULTS);
    candidates
        .into_iter()
        .map(|(rel, is_dir)| Suggestion {
            replace: if is_dir {
                format!("@{rel}/")
            } else {
                format!("@{rel}")
            },
            hint: if is_dir { "dir" } else { "file" }.to_string(),
        })
        .collect()
}

fn walk(dir: &Path, root: &Path, depth: usize, allow_dot: bool, out: &mut Vec<(String, bool)>) {
    let Ok(entries) = fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let name = entry.file_name().to_string_lossy().into_owned();
        if name.starts_with('.') && !allow_dot {
            continue;
        }
        if SKIP_DIRS.contains(&name.as_str()) {
            continue;
        }
        let is_dir = entry.file_type().is_ok_and(|t| t.is_dir());
        let path = entry.path();
        let Ok(rel) = path.strip_prefix(root) else {
            continue;
        };
        let rel = rel.to_string_lossy().into_owned();
        if is_dir && depth < MAX_DEPTH {
            walk(&path, root, depth + 1, allow_dot, out);
        }
        out.push((rel, is_dir));
    }
}
