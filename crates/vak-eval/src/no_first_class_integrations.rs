//! Banned-token gate (docs/design/68-context-engine.md §9): production
//! source under `crates/*/src` must never let a vendor name or a topic word
//! act as a routing key. A `#[cfg(test)]` fixture is allowed to call itself
//! whatever it likes — the scan stops at the first `#[cfg(test)]` marker in
//! each file — and a comment does not count as a hard-coding, so `//` and
//! `///` text is stripped before the remaining lines are checked.
//!
//! The banned set is deliberately just the words this codebase's own history
//! produced: a search vendor's name that once drove tool-name sniffing, and
//! a topic word from the bug report that motivated deleting it.

use std::path::{Path, PathBuf};

const BANNED_TOKENS: &[&str] = &["tavily", "weather", "noida"];

/// One line the gate refused, already resolved to a workspace-relative path
/// so the report reads the same regardless of where the suite runs from.
#[derive(Debug, Clone)]
pub struct Offense {
    pub path: String,
    pub line: usize,
    pub text: String,
}

impl std::fmt::Display for Offense {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}:{}: {}", self.path, self.line, self.text.trim())
    }
}

fn workspace_root() -> Result<PathBuf, String> {
    let manifest_dir = Path::new(env!("CARGO_MANIFEST_DIR"));
    manifest_dir
        .parent()
        .and_then(Path::parent)
        .map(Path::to_path_buf)
        .ok_or_else(|| {
            format!(
                "{} has no workspace root two levels up",
                manifest_dir.display()
            )
        })
}

fn collect_rs_files(dir: &Path, out: &mut Vec<PathBuf>) -> Result<(), String> {
    let entries = std::fs::read_dir(dir).map_err(|error| format!("{}: {error}", dir.display()))?;
    for entry in entries {
        let entry = entry.map_err(|error| format!("{}: {error}", dir.display()))?;
        let path = entry.path();
        let file_type = entry
            .file_type()
            .map_err(|error| format!("{}: {error}", path.display()))?;
        if file_type.is_dir() {
            collect_rs_files(&path, out)?;
        } else if path.extension().is_some_and(|ext| ext == "rs") {
            out.push(path);
        }
    }
    Ok(())
}

/// Every `.rs` file under `crates/*/src`, in the workspace named by `root`.
fn all_src_files(root: &Path) -> Result<Vec<PathBuf>, String> {
    let mut out = Vec::new();
    let crates_dir = root.join("crates");
    let entries = std::fs::read_dir(&crates_dir)
        .map_err(|error| format!("{}: {error}", crates_dir.display()))?;
    for entry in entries {
        let entry = entry.map_err(|error| format!("{}: {error}", crates_dir.display()))?;
        let src = entry.path().join("src");
        if src.is_dir() {
            collect_rs_files(&src, &mut out)?;
        }
    }
    out.sort();
    Ok(out)
}

/// Truncates a line at its first `//` that is not inside a string literal,
/// which covers `///` and `//!` doc comments too (both start with `//`).
/// Block comments (`/* */`) do not appear in this codebase's `.rs` sources
/// today, so they are deliberately not handled here.
fn strip_line_comment(line: &str) -> &str {
    let bytes = line.as_bytes();
    let mut quote: Option<u8> = None;
    let mut i = 0;
    while i < bytes.len() {
        let byte = bytes[i];
        match quote {
            Some(q) => {
                if byte == b'\\' {
                    i += 1;
                } else if byte == q {
                    quote = None;
                }
            }
            None => {
                if byte == b'"' || byte == b'\'' {
                    quote = Some(byte);
                } else if byte == b'/' && bytes.get(i + 1) == Some(&b'/') {
                    return &line[..i];
                }
            }
        }
        i += 1;
    }
    line
}

/// `(1-based line number, comment-stripped text)` for every line before the
/// file's first `#[cfg(test)]` marker.
fn production_lines(text: &str) -> Vec<(usize, String)> {
    let mut out = Vec::new();
    for (index, raw_line) in text.lines().enumerate() {
        if raw_line.contains("#[cfg(test)]") {
            break;
        }
        out.push((index + 1, strip_line_comment(raw_line).to_string()));
    }
    out
}

/// Small, explicit exceptions — vocabulary that is presentation vocabulary
/// or provider-registry data, never a vendor/topic routing key:
///
/// * `vak-llm/src/registry.rs` — verified to contain none of the banned
///   words; allow-listed anyway per the invariant this gate implements.
/// * `docs/` — out of the walked tree today (only `crates/*/src` is
///   scanned), kept here so the rule still reads correctly if the walk ever
///   widens.
/// * `vak-core/src/presentation_tools.rs`'s `semantic_types` arrays — the
///   `weather` semantic type is a card shape the runtime still emits
///   (`emit_metric_card`), not a vendor name.
/// * `vak-delivery/src/skills.rs`'s registry `provides` list and built-in
///   recipe id mirror that same `weather` semantic type on the receiving
///   side; the recipe id is consumed by `vak-agent`'s tests (outside this
///   agent's scope to rename in the same change).
fn is_allowed(rel_path: &str, line: &str) -> bool {
    if rel_path.starts_with("docs/") {
        return true;
    }
    if rel_path == "crates/vak-llm/src/registry.rs" {
        return true;
    }
    if rel_path == "crates/vak-core/src/presentation_tools.rs" && line.contains("semantic_types") {
        return true;
    }
    if rel_path == "crates/vak-delivery/src/skills.rs" {
        let trimmed = line.trim().trim_end_matches(',');
        if trimmed == "\"weather\"" || trimmed == "\"weather.forecast\"" {
            return true;
        }
    }
    false
}

pub fn scan_banned_tokens() -> Result<Vec<Offense>, String> {
    let root = workspace_root()?;
    let mut offenses = Vec::new();
    for path in all_src_files(&root)? {
        let rel = path
            .strip_prefix(&root)
            .unwrap_or(&path)
            .to_string_lossy()
            .replace('\\', "/");
        let text = std::fs::read_to_string(&path)
            .map_err(|error| format!("{}: {error}", path.display()))?;
        for (line, stripped) in production_lines(&text) {
            let lower = stripped.to_ascii_lowercase();
            if BANNED_TOKENS.iter().any(|token| lower.contains(token))
                && !is_allowed(&rel, &stripped)
            {
                offenses.push(Offense {
                    path: rel.clone(),
                    line,
                    text: stripped,
                });
            }
        }
    }
    Ok(offenses)
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]
    use super::*;

    #[test]
    fn no_vendor_or_topic_routing_keys_in_production_code() {
        let offenses = scan_banned_tokens().expect("scan crates/*/src");
        assert!(
            offenses.is_empty(),
            "banned tokens found outside tests/comments/allow-list:\n{}",
            offenses
                .iter()
                .map(Offense::to_string)
                .collect::<Vec<_>>()
                .join("\n")
        );
    }
}
