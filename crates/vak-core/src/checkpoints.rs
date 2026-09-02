//! Workspace checkpoints: full-content snapshots captured at turn starts,
//! stored under the sessions home. Rewind restores file contents and removes
//! files created after the checkpoint. Bash mutations are covered because
//! snapshots are state-based, not operation-based.
//!
//! Safety contract: `restore` only deletes files that were OBSERVED at
//! capture time and are absent from the stored set's deletion candidates —
//! i.e. files the capture walk never saw (over budget, unreadable, secret,
//! gitignored, or beyond the walk break) are left untouched. A rewind can
//! lose the changes made during a session; it must never destroy files it
//! knows nothing about.

use std::collections::HashSet;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use walkdir::WalkDir;

const IGNORED_DIRS: [&str; 5] = [".git", "target", "node_modules", ".vak", "dist"];

/// Rebuildable runtime artifacts (the vak-store SQLite index and its WAL
/// sidecars). Never meaningful workspace content: capturing them into a
/// checkpoint would snapshot a derived cache, and restoring a stale one
/// would corrupt the live index.
const IGNORED_RUNTIME_FILES: [&str; 3] = ["store.db", "store.db-wal", "store.db-shm"];
const MAX_FILE_BYTES: u64 = 8 * 1024 * 1024;
const MAX_TOTAL_BYTES: usize = 64 * 1024 * 1024;
/// Checkpoints accumulate once per turn; keep only the newest N per session.
const MAX_STORED_CHECKPOINTS: usize = 20;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FileSnapshot {
    pub rel_path: String,
    #[serde(with = "base64_bytes")]
    pub content: Vec<u8>,
}

mod base64_bytes {
    use base64::Engine as _;
    use serde::{Deserialize, Deserializer, Serializer};

    pub fn serialize<S: Serializer>(v: &[u8], s: S) -> Result<S::Ok, S::Error> {
        s.serialize_str(&base64::engine::general_purpose::STANDARD.encode(v))
    }

    pub fn deserialize<'de, D: Deserializer<'de>>(d: D) -> Result<Vec<u8>, D::Error> {
        let s = String::deserialize(d)?;
        base64::engine::general_purpose::STANDARD
            .decode(s)
            .map_err(serde::de::Error::custom)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Checkpoint {
    pub seq: u32,
    pub session_id: String,
    pub created_at: chrono::DateTime<chrono::Utc>,
    pub label: String,
    /// Files whose CONTENT is stored and will be rewritten on restore.
    pub files: Vec<FileSnapshot>,
    /// Every regular-file path the capture walk observed, including files
    /// whose content was NOT stored (oversized, unreadable, secret,
    /// gitignored). Restore only deletes files absent from this list.
    #[serde(default)]
    pub observed: Vec<String>,
}

fn is_ignored(rel: &Path) -> bool {
    rel.components().any(|c| {
        matches!(
            c,
            std::path::Component::Normal(name) if IGNORED_DIRS
                .contains(&name.to_string_lossy().as_ref())
        )
    }) || rel
        .file_name()
        .and_then(|n| n.to_str())
        .map(|n| IGNORED_RUNTIME_FILES.contains(&n))
        .unwrap_or(false)
}

/// Paths that must never be captured into checkpoints nor deleted by a
/// rewind, regardless of what any .gitignore says.
fn is_secret_path(rel: &Path) -> bool {
    let Some(name) = rel.file_name().and_then(|n| n.to_str()) else {
        return true;
    };
    let lower = name.to_ascii_lowercase();
    lower == ".env"
        || lower.starts_with(".env.")
        || lower.ends_with(".pem")
        || lower.ends_with(".key")
        || lower.starts_with("id_rsa")
        || lower.starts_with("id_ed25519")
        || lower == "credentials.json"
}

// ---------------------------------------------------------------------------
// Minimal .gitignore support (subset): root + nested .gitignore files,
// last matching rule wins, negation (`!pat`) supported. Patterns without
// '/' match against the file name anywhere below their directory; patterns
// with '/' match against the path relative to their directory.
// ---------------------------------------------------------------------------

#[derive(Debug, Clone)]
struct IgnoreRule {
    /// Directory containing the .gitignore this rule came from, relative
    /// to the capture root ("." for the root).
    base: PathBuf,
    negate: bool,
    dir_only: bool,
    matcher: globset::GlobSet,
}

#[derive(Debug, Default)]
struct IgnoreRules {
    rules: Vec<IgnoreRule>,
}

impl IgnoreRules {
    fn load_dir(&mut self, root: &Path, dir_rel: &Path) {
        let file = root.join(dir_rel).join(".gitignore");
        let Ok(text) = std::fs::read_to_string(&file) else {
            return;
        };
        for raw in text.lines() {
            let line = raw.trim_end_matches(['\r', ' ']).trim();
            if line.is_empty() || line.starts_with('#') {
                continue;
            }
            let (negate, line) = match line.strip_prefix('!') {
                Some(rest) => (true, rest),
                None => (false, line),
            };
            if line.is_empty() {
                continue;
            }
            let (dir_only, line) = match line.strip_suffix('/') {
                Some(rest) => (true, rest),
                None => (false, line),
            };
            // Anchored when it contains a '/' anywhere (leading slash just
            // anchors to the rule's directory).
            let anchored = line.trim_start_matches('/').contains('/');
            let pattern_text = line.trim_start_matches('/');
            let mut gb = globset::GlobBuilder::new(pattern_text);
            gb.literal_separator(anchored);
            let Ok(glob) = gb.build() else { continue };
            let Ok(matcher) = globset::GlobSetBuilder::new().add(glob).build() else {
                continue;
            };
            self.rules.push(IgnoreRule {
                // "" (not "."): strip_prefix(".") never matches plain
                // relative paths, which would silently disable every
                // root-level rule.
                base: if dir_rel.as_os_str() == "." {
                    PathBuf::new()
                } else {
                    dir_rel.to_path_buf()
                },
                negate,
                dir_only,
                matcher,
            });
        }
    }

    fn is_ignored(&self, rel: &Path, is_dir: bool) -> bool {
        let mut ignored = false;
        // Shallowest ancestor first: a dir-only rule like `secrets/`
        // prunes everything beneath it. Deeper matches (and later rules)
        // override, mirroring gitignore's last-match-wins.
        let ancestors: Vec<&Path> = rel.ancestors().collect();
        for candidate in ancestors.iter().rev() {
            if candidate.as_os_str().is_empty() {
                continue;
            }
            let candidate_is_dir = *candidate != rel || is_dir;
            for rule in &self.rules {
                let Ok(sub) = candidate.strip_prefix(&rule.base) else {
                    continue;
                };
                if sub.as_os_str().is_empty() {
                    continue;
                }
                let name_hit = sub
                    .file_name()
                    .and_then(|n| n.to_str())
                    .map(|n| rule.matcher.is_match(n))
                    .unwrap_or(false);
                let path_hit = sub
                    .to_str()
                    .map(|p| rule.matcher.is_match(p))
                    .unwrap_or(false);
                if (name_hit || path_hit) && (candidate_is_dir || !rule.dir_only) {
                    ignored = !rule.negate;
                }
            }
        }
        ignored
    }
}

/// Captures every regular file under `cwd` (bounded), sorted by path.
/// `observed` records every walked path even when its content was skipped.
pub fn capture(cwd: &Path, session_id: &str, seq: u32, label: &str) -> std::io::Result<Checkpoint> {
    let canonical_root = cwd.canonicalize().unwrap_or_else(|_| cwd.into());
    let mut ignores = IgnoreRules::default();
    ignores.load_dir(cwd, Path::new("."));

    let mut files = Vec::new();
    let mut observed: Vec<String> = Vec::new();
    let mut total = 0usize;
    for entry in WalkDir::new(cwd)
        .follow_links(false)
        .into_iter()
        .filter_entry(|e| {
            e.path()
                .file_name()
                .map(|f| !IGNORED_DIRS.contains(&f.to_string_lossy().as_ref()))
                .unwrap_or(true)
        })
        .flatten()
    {
        let rel_rooted = match entry.path().strip_prefix(cwd) {
            Ok(r) => r,
            Err(_) => continue,
        };
        if entry.file_type().is_dir() {
            if !ignores.is_ignored(rel_rooted, true) && !rel_rooted.as_os_str().is_empty() {
                ignores.load_dir(cwd, rel_rooted);
            }
            continue;
        }
        if !entry.file_type().is_file() {
            continue;
        }
        let rel = match entry.path().canonicalize() {
            Ok(abs) => abs
                .strip_prefix(&canonical_root)
                .unwrap_or(rel_rooted)
                .to_path_buf(),
            Err(_) => rel_rooted.to_path_buf(),
        };
        if is_ignored(&rel) || is_secret_path(&rel) || ignores.is_ignored(&rel, false) {
            // Observed but deliberately not captured: restore must leave
            // these alone either way.
            observed.push(rel.display().to_string());
            continue;
        }
        observed.push(rel.display().to_string());

        let meta = match entry.metadata() {
            Ok(m) => m,
            Err(_) => continue,
        };
        if meta.len() > MAX_FILE_BYTES {
            continue;
        }
        total += meta.len() as usize;
        if total > MAX_TOTAL_BYTES {
            // Walk budget exhausted; remaining paths are simply not
            // observed, so restore will refuse to delete them.
            break;
        }
        match std::fs::read(entry.path()) {
            Ok(content) => files.push(FileSnapshot {
                rel_path: rel.display().to_string(),
                content,
            }),
            Err(_) => continue,
        }
    }
    observed.sort();
    observed.dedup();
    files.sort_by(|a, b| a.rel_path.cmp(&b.rel_path));
    Ok(Checkpoint {
        seq,
        session_id: session_id.to_string(),
        created_at: chrono::Utc::now(),
        label: label.to_string(),
        files,
        observed,
    })
}

fn checkpoint_dir(sessions_home: &Path, session_id: &str) -> PathBuf {
    sessions_home.join("checkpoints").join(session_id)
}

/// Persists a checkpoint atomically, prunes older ones, and returns the
/// new file's path.
pub fn store(sessions_home: &Path, cp: &Checkpoint) -> std::io::Result<PathBuf> {
    let dir = checkpoint_dir(sessions_home, &cp.session_id);
    std::fs::create_dir_all(&dir)?;
    let path = dir.join(format!("{:04}.json", cp.seq));
    let tmp = dir.join(format!(".{:04}.tmp", cp.seq));
    std::fs::write(&tmp, serde_json::to_vec(cp).map_err(std::io::Error::other)?)?;
    std::fs::rename(&tmp, &path)?;

    let mut seqs: Vec<u32> = std::fs::read_dir(&dir)?
        .flatten()
        .filter_map(|e| {
            let name = e.file_name().to_string_lossy().into_owned();
            if name.starts_with('.') || !name.ends_with(".json") {
                return None;
            }
            name.trim_end_matches(".json").parse::<u32>().ok()
        })
        .collect();
    seqs.sort_unstable();
    while seqs.len() > MAX_STORED_CHECKPOINTS {
        let oldest = seqs.remove(0);
        let _ = std::fs::remove_file(dir.join(format!("{oldest:04}.json")));
    }
    Ok(path)
}

pub fn list(sessions_home: &Path, session_id: &str) -> std::io::Result<Vec<Checkpoint>> {
    let dir = checkpoint_dir(sessions_home, session_id);
    let mut out = Vec::new();
    for entry in std::fs::read_dir(&dir)?.flatten() {
        let p = entry.path();
        if p.extension().and_then(|e| e.to_str()) == Some("json") {
            match std::fs::read(&p)
                .map_err(std::io::Error::other)
                .and_then(|b| {
                    serde_json::from_slice::<Checkpoint>(&b).map_err(std::io::Error::other)
                }) {
                Ok(cp) => out.push(cp),
                Err(_) => continue,
            }
        }
    }
    out.sort_by_key(|c| c.seq);
    Ok(out)
}

pub fn load(sessions_home: &Path, session_id: &str, seq: u32) -> std::io::Result<Checkpoint> {
    let path = checkpoint_dir(sessions_home, session_id).join(format!("{seq:04}.json"));
    let bytes = std::fs::read(path)?;
    serde_json::from_slice(&bytes).map_err(std::io::Error::other)
}

/// Restores the snapshot: rewrites snapshotted files and deletes ONLY
/// files that exist now but were never observed at capture time (i.e.
/// created after the checkpoint, tracked scope). Anything the capture
/// could not vouch for — oversized, unreadable, secret, gitignored,
/// beyond-budget, or written by an old-format checkpoint without an
/// observed manifest — is left untouched.
/// Human-readable workspace delta between a stored checkpoint and the
/// current tree (docs/design/42-managed-work-contracts.md): modified/added/deleted paths
/// with byte deltas plus bounded excerpts for changed text files.
/// Feeds goal-mode auditors so verdicts rest on environment facts.
pub fn delta_summary(
    cwd: &Path,
    sessions_home: &Path,
    session_id: &str,
    seq: u32,
    max_bytes: usize,
) -> std::io::Result<String> {
    let cp = load(sessions_home, session_id, seq)?;

    let mut baseline: std::collections::HashMap<String, Vec<u8>> = std::collections::HashMap::new();
    for f in &cp.files {
        baseline.insert(f.rel_path.clone(), f.content.clone());
    }
    let observed: std::collections::HashSet<String> = cp.observed.iter().cloned().collect();

    // Walk current tree with the same ignore rules as capture.
    let mut ignores = IgnoreRules::default();
    ignores.load_dir(cwd, Path::new("."));

    let mut modified: Vec<String> = Vec::new();
    let mut added: Vec<String> = Vec::new();
    let mut deleted: Vec<String> = Vec::new();
    let mut excerpts: Vec<(String, String)> = Vec::new();

    let mut seen_now: std::collections::HashSet<String> = Default::default();
    for entry in WalkDir::new(cwd)
        .follow_links(false)
        .into_iter()
        .filter_entry(|e| {
            let rel = e.path().strip_prefix(cwd).unwrap_or(e.path()).to_path_buf();
            !ignores.is_ignored(&rel, e.file_type().is_dir())
        })
    {
        let Ok(entry) = entry else { continue };
        if !entry.file_type().is_file() {
            continue;
        }
        let Ok(rel_path) = entry.path().strip_prefix(cwd) else {
            continue;
        };
        let rel_full = rel_path.to_string_lossy().replace('\\', "/");
        seen_now.insert(rel_full.clone());
        let bytes = std::fs::read(entry.path()).unwrap_or_default();
        match baseline.get(&rel_full.clone()) {
            Some(old) => {
                if old.as_slice() != bytes.as_slice() {
                    modified.push(format!(
                        "M {} ({} -> {} bytes)",
                        rel_full,
                        old.len(),
                        bytes.len()
                    ));
                    if excerpts.len() < 8
                        && let Ok(text) = String::from_utf8(bytes[..bytes.len().min(400)].to_vec())
                    {
                        excerpts.push((rel_full.clone(), text));
                    }
                }
            }
            None => {
                added.push(format!("A {} ({} bytes)", rel_full, bytes.len()));
                if excerpts.len() < 8
                    && let Ok(text) = String::from_utf8(bytes[..bytes.len().min(200)].to_vec())
                {
                    excerpts.push((rel_full.clone(), text));
                }
            }
        }
    }
    for rel in &observed {
        if !baseline.contains_key(rel) {
            continue; // observed-but-unstored: cannot diff contents
        }
        if !seen_now.contains(rel) {
            deleted.push(format!("D {rel}"));
        }
    }

    modified.sort();
    added.sort();
    deleted.sort();

    let mut out = String::new();
    if modified.is_empty() && added.is_empty() && deleted.is_empty() {
        out.push_str("(workspace unchanged since checkpoint)");
        return Ok(out);
    }
    for line in modified.iter().chain(added.iter()).chain(deleted.iter()) {
        if out.len() > max_bytes {
            break;
        }
        out.push_str(line);
        out.push('\n');
    }
    for (path, text) in &excerpts {
        if out.len() > max_bytes {
            break;
        }
        out.push_str(&format!("--- {} (excerpt) ---\n{text}\n", path));
    }
    if out.len() > max_bytes {
        out.truncate(max_bytes);
        out.push_str("\n(truncated)");
    }
    Ok(out)
}

pub fn restore(cwd: &Path, cp: &Checkpoint) -> std::io::Result<(usize, usize)> {
    let mut restored = 0usize;
    let mut deleted = 0usize;

    for f in &cp.files {
        let target = cwd.join(&f.rel_path);
        if let Some(parent) = target.parent() {
            std::fs::create_dir_all(parent)?;
        }
        std::fs::write(&target, &f.content)?;
        restored += 1;
    }

    // Legacy checkpoints have no manifest: deleting anything would be a
    // guess, so delete nothing.
    if cp.observed.is_empty() {
        return Ok((restored, deleted));
    }

    // Compared by relative path so symlinked roots (/tmp vs /private/tmp)
    // can't cause false mismatches.
    let observed: HashSet<&str> = cp.observed.iter().map(String::as_str).collect();
    let stored: HashSet<&str> = cp.files.iter().map(|f| f.rel_path.as_str()).collect();
    for entry in WalkDir::new(cwd)
        .follow_links(false)
        .into_iter()
        .filter_entry(|e| {
            e.path()
                .file_name()
                .map(|f| !IGNORED_DIRS.contains(&f.to_string_lossy().as_ref()))
                .unwrap_or(true)
        })
        .flatten()
    {
        if !entry.file_type().is_file() {
            continue;
        }
        let Ok(rel) = entry.path().strip_prefix(cwd) else {
            continue;
        };
        if is_ignored(rel) || is_secret_path(rel) {
            continue;
        }
        let rel_str = rel.to_string_lossy();
        // Present now but unknown at capture ⇒ created afterwards ⇒ safe
        // to remove. Known-but-unstored files are preserved.
        if !observed.contains(rel_str.as_ref())
            && !stored.contains(rel_str.as_ref())
            && std::fs::remove_file(entry.path()).is_ok()
        {
            deleted += 1;
        }
    }
    Ok((restored, deleted))
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod delta_tests {
    use super::*;

    #[test]
    fn delta_reports_modify_add_delete_with_excerpt() {
        let dir = tempfile::tempdir().unwrap();
        let cwd = dir.path();
        std::fs::write(cwd.join("a.txt"), b"v1").unwrap();
        std::fs::create_dir_all(cwd.join("sub")).unwrap();
        std::fs::write(cwd.join("sub/b.txt"), b"doomed").unwrap();

        let cp = capture(cwd, "s1", 0, "start").unwrap();
        store(dir.path(), &cp).unwrap();

        // Mutate a, delete b, add c.
        std::fs::write(cwd.join("a.txt"), "v2-longer content").unwrap();
        let _ = std::fs::remove_file(cwd.join("sub/b.txt"));
        std::fs::write(cwd.join("c-new.txt"), "brand new file").unwrap();

        let summary = delta_summary(cwd, dir.path(), "s1", 0, 4096).unwrap();
        assert!(summary.contains("M a.txt"), "{summary}");
        assert!(summary.contains("D sub/b.txt"), "{summary}");
        assert!(summary.contains("A c-new.txt"), "{summary}");
        assert!(summary.contains("v2-longer content"), "excerpt included");
    }

    #[test]
    fn unchanged_tree_reports_no_delta() {
        let dir = tempfile::tempdir().unwrap();
        let home = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("only.txt"), b"same").unwrap();
        let cp = capture(dir.path(), "s2", 0, "l").unwrap();
        store(home.path(), &cp).unwrap();
        let summary = delta_summary(dir.path(), home.path(), "s2", 0, 1024).unwrap();
        assert!(summary.contains("unchanged"), "{summary}");
    }
}
