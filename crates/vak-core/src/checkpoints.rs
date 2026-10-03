//! Workspace checkpoints: content-addressed manifests captured at turn
//! starts, stored under the sessions home. Rewind restores file contents
//! and removes files created after the checkpoint. Bash mutations are
//! covered because manifests are state-based, not operation-based.
//!
//! A per-checkpoint **manifest** (`<session_id>/<seq>.json` under the
//! Agent's checkpoints root: path, object id, size and mtime per file, plus
//! the observed set) names file contents held as tenant objects granted to
//! the conversation (plan M3b slice 2), so equal contents are stored once.
//! A file whose (size, mtime) match its entry in the immediately preceding
//! manifest is assumed unchanged and its id is reused without being
//! re-read; this is the standard rsync/make-style fast path and trades an
//! astronomically small risk (a same-second, same-size content change with
//! untouched mtime, on a filesystem coarse enough to collide) for turning
//! a multi-thousand-file workspace's per-turn capture into a handful of
//! stats plus however many files actually changed.
//!
//! Safety contract: `restore` only deletes files that were OBSERVED at
//! capture time and are absent from the stored set's deletion candidates
//! -- i.e. files the capture walk never saw (over budget, unreadable,
//! secret, gitignored, or beyond the walk break) are left untouched. A
//! rewind can lose the changes made during a session; it must never
//! destroy files it knows nothing about. `store` prunes old manifests and
//! releases the conversation's grant on contents no remaining manifest of
//! that session names. A checkpoint file from before this manifest
//! format (which embedded base64 file content directly) fails to
//! deserialize -- missing `hash`/`size`/`mtime_ns` -- and is simply not
//! read (AGENTS.md invariant 29): it is never partially read or migrated.

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use vak_session::objects::{ObjectRef, Objects, conversation_scope};
use walkdir::WalkDir;

const IGNORED_DIRS: [&str; 5] = [
    ".git",
    "target",
    "node_modules",
    vak_config::scope::PROJECT_DIR,
    "dist",
];

/// Rebuildable runtime artifacts (the vak-store SQLite index and its WAL
/// sidecars). Never meaningful workspace content: capturing them into a
/// checkpoint would snapshot a derived cache, and restoring a stale one
/// would corrupt the live index.
const IGNORED_RUNTIME_FILES: [&str; 3] = ["store.db", "store.db-wal", "store.db-shm"];
const MAX_FILE_BYTES: u64 = 8 * 1024 * 1024;
const MAX_TOTAL_BYTES: usize = 64 * 1024 * 1024;
/// Checkpoints accumulate once per turn; keep only the newest N per session.
const MAX_STORED_CHECKPOINTS: usize = 20;

/// One captured file. Content is the tenant object whose id is `hash`;
/// `size`/`mtime_ns` are the fast-path signature the next capture compares
/// against to decide whether it can reuse `hash` without reading the file.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ManifestEntry {
    pub rel_path: String,
    pub hash: String,
    pub size: u64,
    pub mtime_ns: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Manifest {
    pub seq: u32,
    pub session_id: String,
    pub created_at: chrono::DateTime<chrono::Utc>,
    pub label: String,
    /// Files whose content is stored (in the blob store) and will be
    /// rewritten on restore.
    pub files: Vec<ManifestEntry>,
    /// Every regular-file path the capture walk observed, including files
    /// whose content was NOT stored (oversized, unreadable, secret,
    /// gitignored). Restore only deletes files absent from this list.
    pub observed: Vec<String>,
}

/// How much of a [`capture`] call was served from the previous manifest
/// versus freshly read from disk. Exists so callers (and tests) can
/// observe the incremental fast path directly rather than through timing
/// alone, which is flaky under load.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct CaptureStats {
    pub files_observed: usize,
    pub files_reused: usize,
    pub files_read: usize,
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

// ---------------------------------------------------------------------------
// Content-addressed blob store, shared by every manifest under a sessions
// home.
// ---------------------------------------------------------------------------

fn checkpoint_root(scope: &vak_config::scope::AgentScope) -> PathBuf {
    scope.checkpoints()
}

fn manifest_dir(scope: &vak_config::scope::AgentScope, session_id: &str) -> PathBuf {
    checkpoint_root(scope).join(session_id)
}

fn mtime_nanos(meta: &std::fs::Metadata) -> Option<u64> {
    let modified = meta.modified().ok()?;
    let since_epoch = modified.duration_since(std::time::UNIX_EPOCH).ok()?;
    u64::try_from(since_epoch.as_nanos()).ok()
}

/// Writes `content` under `hash` if not already present. Idempotent: the
/// hash is the content, so a lost race between two writers rewrites the
/// same bytes, and the atomic rename means a reader never observes a
/// partial blob.
/// Every stored sequence number for `session_id`, ascending. Reads only
/// the manifest directory's file names -- never opens or parses a
/// manifest -- so this is cheap even with the full `MAX_STORED_CHECKPOINTS`
/// present.
fn list_seqs(scope: &vak_config::scope::AgentScope, session_id: &str) -> std::io::Result<Vec<u32>> {
    let dir = manifest_dir(scope, session_id);
    let entries = match std::fs::read_dir(&dir) {
        Ok(rd) => rd,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(e) => return Err(e),
    };
    let mut seqs: Vec<u32> = entries
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
    Ok(seqs)
}

/// The next checkpoint sequence number for `session_id`. Unlike `list`,
/// this never opens a manifest file -- it is safe to call on every turn.
pub fn next_seq(scope: &vak_config::scope::AgentScope, session_id: &str) -> u32 {
    list_seqs(scope, session_id)
        .ok()
        .and_then(|seqs| seqs.last().map(|s| s + 1))
        .unwrap_or(0)
}

/// The most recently stored manifest for `session_id`, if any. This is the
/// baseline [`capture`] diffs against for its incremental fast path.
fn latest_manifest(scope: &vak_config::scope::AgentScope, session_id: &str) -> Option<Manifest> {
    let seq = *list_seqs(scope, session_id).ok()?.last()?;
    load(scope, session_id, seq).ok()
}

/// Captures every regular file under `cwd` (bounded), sorted by path.
/// `observed` records every walked path even when its content was
/// skipped. A file whose (size, mtime) match its entry in the previous
/// manifest for `session_id` reuses that entry's hash instead of being
/// re-read; everything else is read, hashed, and written to the blob
/// store.
pub fn capture(
    cwd: &Path,
    scope: &vak_config::scope::AgentScope,
    objects: &dyn Objects,
    session_id: &str,
    seq: u32,
    label: &str,
) -> std::io::Result<(Manifest, CaptureStats)> {
    let canonical_root = cwd.canonicalize().unwrap_or_else(|_| cwd.into());
    let mut ignores = IgnoreRules::default();
    ignores.load_dir(cwd, Path::new("."));

    let previous = latest_manifest(scope, session_id);
    let prev_index: HashMap<&str, &ManifestEntry> = previous
        .as_ref()
        .map(|m| m.files.iter().map(|e| (e.rel_path.as_str(), e)).collect())
        .unwrap_or_default();

    let mut files = Vec::new();
    let mut observed: Vec<String> = Vec::new();
    let mut total = 0usize;
    let mut stats = CaptureStats::default();
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

        let rel_str = rel.display().to_string();
        let size = meta.len();
        let mtime_ns = mtime_nanos(&meta);
        let reusable = mtime_ns.is_some_and(|ns| {
            prev_index
                .get(rel_str.as_str())
                .is_some_and(|prev| prev.size == size && prev.mtime_ns == ns)
        });
        if reusable && let Some(prev) = prev_index.get(rel_str.as_str()) {
            files.push((*prev).clone());
            stats.files_reused += 1;
            continue;
        }

        match std::fs::read(entry.path()) {
            Ok(content) => {
                let Ok(object) = objects.put(&content, &conversation_scope(session_id)) else {
                    continue;
                };
                files.push(ManifestEntry {
                    rel_path: rel_str,
                    hash: object.id,
                    size,
                    mtime_ns: mtime_ns.unwrap_or_default(),
                });
                stats.files_read += 1;
            }
            Err(_) => continue,
        }
    }
    observed.sort();
    observed.dedup();
    files.sort_by(|a, b| a.rel_path.cmp(&b.rel_path));
    stats.files_observed = observed.len();
    Ok((
        Manifest {
            seq,
            session_id: session_id.to_string(),
            created_at: chrono::Utc::now(),
            label: label.to_string(),
            files,
            observed,
        },
        stats,
    ))
}

/// Persists a manifest atomically and prunes manifests older than the
/// newest [`MAX_STORED_CHECKPOINTS`] for this session. A pruned manifest's
/// contents lose this session's grant unless a surviving manifest still
/// names them, and objects no scope holds any more are collected. Returns
/// the new manifest file's path.
pub fn store(
    scope: &vak_config::scope::AgentScope,
    objects: &dyn Objects,
    m: &Manifest,
) -> std::io::Result<PathBuf> {
    let dir = manifest_dir(scope, &m.session_id);
    std::fs::create_dir_all(&dir)?;
    let path = dir.join(format!("{:04}.json", m.seq));
    let tmp = dir.join(format!(".{:04}.tmp", m.seq));
    std::fs::write(&tmp, serde_json::to_vec(m).map_err(std::io::Error::other)?)?;
    std::fs::rename(&tmp, &path)?;

    let seqs = list_seqs(scope, &m.session_id)?;
    if seqs.len() > MAX_STORED_CHECKPOINTS {
        let (pruned, kept) = seqs.split_at(seqs.len() - MAX_STORED_CHECKPOINTS);
        let contents = |seq: &u32| -> Vec<ManifestEntry> {
            load(scope, &m.session_id, *seq)
                .map(|manifest| manifest.files)
                .unwrap_or_default()
        };
        let live: HashSet<String> = kept
            .iter()
            .flat_map(contents)
            .map(|entry| entry.hash)
            .collect();
        let grant = conversation_scope(&m.session_id);
        let mut released = false;
        for oldest in pruned {
            for entry in contents(oldest) {
                if !live.contains(&entry.hash) {
                    let object = ObjectRef {
                        id: entry.hash,
                        len: entry.size,
                    };
                    released |= objects.release(&object, &grant).is_ok();
                }
            }
            let _ = std::fs::remove_file(dir.join(format!("{oldest:04}.json")));
        }
        if released {
            let _ = objects.collect();
        }
    }
    Ok(path)
}

/// Every manifest stored for `session_id`, oldest first. A file that
/// fails to deserialize (an old-format checkpoint, or anything else that
/// does not match [`Manifest`]) is simply skipped, never partially read.
pub fn list(
    scope: &vak_config::scope::AgentScope,
    session_id: &str,
) -> std::io::Result<Vec<Manifest>> {
    let dir = manifest_dir(scope, session_id);
    let mut out = Vec::new();
    for entry in std::fs::read_dir(&dir)?.flatten() {
        let p = entry.path();
        if p.extension().and_then(|e| e.to_str()) == Some("json") {
            match std::fs::read(&p)
                .map_err(std::io::Error::other)
                .and_then(|b| serde_json::from_slice::<Manifest>(&b).map_err(std::io::Error::other))
            {
                Ok(m) => out.push(m),
                Err(_) => continue,
            }
        }
    }
    out.sort_by_key(|c| c.seq);
    Ok(out)
}

pub fn load(
    scope: &vak_config::scope::AgentScope,
    session_id: &str,
    seq: u32,
) -> std::io::Result<Manifest> {
    let path = manifest_dir(scope, session_id).join(format!("{seq:04}.json"));
    let bytes = std::fs::read(path)?;
    serde_json::from_slice(&bytes).map_err(std::io::Error::other)
}

/// Human-readable workspace delta between a stored checkpoint and the
/// current tree (docs/design/42-managed-work-contracts.md): modified/added/deleted paths
/// with byte deltas plus bounded excerpts for changed text files. Feeds
/// goal-mode auditors so verdicts rest on environment facts. Like
/// `capture`, a file whose (size, mtime) match the manifest entry is
/// assumed unchanged without being read; only a real difference in either
/// pays for a read.
pub fn delta_summary(
    cwd: &Path,
    scope: &vak_config::scope::AgentScope,
    objects: &dyn Objects,
    session_id: &str,
    seq: u32,
    max_bytes: usize,
) -> std::io::Result<String> {
    let m = load(scope, session_id, seq)?;

    let baseline: HashMap<&str, &ManifestEntry> =
        m.files.iter().map(|f| (f.rel_path.as_str(), f)).collect();
    let observed: HashSet<String> = m.observed.iter().cloned().collect();

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
        match baseline.get(rel_full.as_str()) {
            Some(old) => {
                let unchanged = entry
                    .metadata()
                    .ok()
                    .filter(|meta| meta.len() == old.size)
                    .and_then(|meta| mtime_nanos(&meta))
                    .is_some_and(|ns| ns == old.mtime_ns);
                if unchanged {
                    continue;
                }
                let bytes = std::fs::read(entry.path()).unwrap_or_default();
                if bytes.len() as u64 == old.size && objects.id_of(&bytes) == old.hash {
                    // mtime moved (e.g. a touch or a checkout) but the
                    // content did not.
                    continue;
                }
                modified.push(format!(
                    "M {} ({} -> {} bytes)",
                    rel_full,
                    old.size,
                    bytes.len()
                ));
                if excerpts.len() < 8
                    && let Ok(text) = String::from_utf8(bytes[..bytes.len().min(400)].to_vec())
                {
                    excerpts.push((rel_full.clone(), text));
                }
            }
            None => {
                let bytes = std::fs::read(entry.path()).unwrap_or_default();
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
        if !baseline.contains_key(rel.as_str()) {
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

/// Restores the snapshot: rewrites snapshotted files from their objects
/// and deletes ONLY files that exist now but were never observed at
/// capture time (i.e. created after the checkpoint, tracked scope).
/// Anything the capture could not vouch for -- oversized, unreadable,
/// secret, gitignored, or beyond-budget -- is left untouched.
pub fn restore(cwd: &Path, objects: &dyn Objects, m: &Manifest) -> std::io::Result<(usize, usize)> {
    let mut restored = 0usize;
    let mut deleted = 0usize;

    for f in &m.files {
        let object = ObjectRef {
            id: f.hash.clone(),
            len: f.size,
        };
        let content = objects
            .get(&object, &conversation_scope(&m.session_id))
            .map_err(std::io::Error::other)?;
        let target = cwd.join(&f.rel_path);
        if let Some(parent) = target.parent() {
            std::fs::create_dir_all(parent)?;
        }
        std::fs::write(&target, &content)?;
        restored += 1;
    }

    // Nothing was ever observed (an empty workspace at capture time, or an
    // old-format manifest defaulted to empty): deleting anything would be
    // a guess, so delete nothing.
    if m.observed.is_empty() {
        return Ok((restored, deleted));
    }

    // Compared by relative path so symlinked roots (/tmp vs /private/tmp)
    // can't cause false mismatches.
    let observed: HashSet<&str> = m.observed.iter().map(String::as_str).collect();
    let stored: HashSet<&str> = m.files.iter().map(|f| f.rel_path.as_str()).collect();
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
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::*;

    fn objects() -> &'static vak_session::objects::MemoryObjects {
        static OBJECTS: std::sync::LazyLock<vak_session::objects::MemoryObjects> =
            std::sync::LazyLock::new(Default::default);
        &OBJECTS
    }

    fn write(p: &Path, rel: &str, content: &str) {
        let target = p.join(rel);
        std::fs::create_dir_all(target.parent().unwrap()).unwrap();
        std::fs::write(target, content).unwrap();
    }

    #[test]
    fn capture_includes_files_and_skips_ignored_dirs() {
        let dir = tempfile::tempdir().unwrap();
        // A sibling temp dir, never nested inside `dir` — sessions_home is
        // never inside a real workspace either, and capturing the blob
        // store's own files while walking the workspace would be wrong.
        let home_dir = tempfile::tempdir().unwrap();
        let home = home_dir.path().to_path_buf();
        write(dir.path(), "src/main.rs", "fn main() {}");
        write(dir.path(), "README.md", "readme");
        write(dir.path(), "target/debug/blob.o", "binary junk");
        write(dir.path(), ".git/config", "gitconfig");

        let (cp, stats) = capture(
            dir.path(),
            &vak_config::scope::AgentScope::new(&home),
            objects(),
            "s1",
            0,
            "initial",
        )
        .unwrap();
        let paths: Vec<&str> = cp.files.iter().map(|f| f.rel_path.as_str()).collect();
        assert!(paths.contains(&"src/main.rs"));
        assert!(paths.contains(&"README.md"));
        assert!(!paths.iter().any(|p| p.starts_with("target/")));
        assert!(!paths.iter().any(|p| p.starts_with(".git/")));
        assert_eq!(stats.files_read, 2, "first capture reads everything");
        assert_eq!(stats.files_reused, 0);
    }

    #[test]
    fn store_list_load_roundtrip_preserves_content() {
        let dir = tempfile::tempdir().unwrap();
        // A sibling temp dir, never nested inside `dir` — sessions_home is
        // never inside a real workspace either, and capturing the blob
        // store's own files while walking the workspace would be wrong.
        let home_dir = tempfile::tempdir().unwrap();
        let home = home_dir.path().to_path_buf();
        let binary: &[u8] = &[0u8, 1, 2, b'b', b'i', b'n', 0xff];
        std::fs::write(dir.path().join("data.bin"), binary).unwrap();

        let (cp, _) = capture(
            dir.path(),
            &vak_config::scope::AgentScope::new(&home),
            objects(),
            "sess",
            3,
            "third",
        )
        .unwrap();
        store(&vak_config::scope::AgentScope::new(&home), objects(), &cp).unwrap();

        let list = list(&vak_config::scope::AgentScope::new(&home), "sess").unwrap();
        assert_eq!(list.len(), 1);
        assert_eq!(list[0].seq, 3);

        let loaded = load(&vak_config::scope::AgentScope::new(&home), "sess", 3).unwrap();
        let hash = loaded
            .files
            .iter()
            .find(|f| f.rel_path == "data.bin")
            .unwrap()
            .hash
            .clone();
        assert_eq!(
            objects()
                .get(
                    &ObjectRef {
                        id: hash,
                        len: binary.len() as u64,
                    },
                    &conversation_scope("sess"),
                )
                .unwrap(),
            binary.to_vec(),
            "binary content must round-trip through the object store"
        );
    }

    #[test]
    fn restore_reverts_edits_and_removes_new_files() {
        let dir = tempfile::tempdir().unwrap();
        // A sibling temp dir, never nested inside `dir` — sessions_home is
        // never inside a real workspace either, and capturing the blob
        // store's own files while walking the workspace would be wrong.
        let home_dir = tempfile::tempdir().unwrap();
        let home = home_dir.path().to_path_buf();

        // State at checkpoint time.
        write(dir.path(), "keep.txt", "original");
        write(dir.path(), "src/lib.rs", "old code");
        let (cp, _) = capture(
            dir.path(),
            &vak_config::scope::AgentScope::new(&home),
            objects(),
            "s",
            0,
            "before",
        )
        .unwrap();
        store(&vak_config::scope::AgentScope::new(&home), objects(), &cp).unwrap();

        // Mutate after the checkpoint: edit one file, delete another, add a third.
        write(dir.path(), "src/lib.rs", "rewritten!");
        std::fs::remove_file(dir.path().join("keep.txt")).unwrap();
        write(dir.path(), "created-later.txt", "new junk");

        let restored_cp = load(&vak_config::scope::AgentScope::new(&home), "s", 0).unwrap();
        let (restored, deleted) = restore(dir.path(), objects(), &restored_cp).unwrap();

        assert!(restored >= 2);
        assert_eq!(
            std::fs::read_to_string(dir.path().join("src/lib.rs")).unwrap(),
            "old code",
            "edited file must revert"
        );
        assert_eq!(
            std::fs::read_to_string(dir.path().join("keep.txt")).unwrap(),
            "original",
            "deleted file must come back"
        );
        assert!(
            !dir.path().join("created-later.txt").exists(),
            "post-checkpoint file must be removed"
        );
        assert!(deleted >= 1);
    }

    #[test]
    fn sequences_are_per_session() {
        let dir = tempfile::tempdir().unwrap();
        // A sibling temp dir, never nested inside `dir` — sessions_home is
        // never inside a real workspace either, and capturing the blob
        // store's own files while walking the workspace would be wrong.
        let home_dir = tempfile::tempdir().unwrap();
        let home = home_dir.path().to_path_buf();
        write(dir.path(), "a.txt", "a");

        let (cp0, _) = capture(
            dir.path(),
            &vak_config::scope::AgentScope::new(&home),
            objects(),
            "sess-a",
            0,
            "a0",
        )
        .unwrap();
        store(&vak_config::scope::AgentScope::new(&home), objects(), &cp0).unwrap();
        let (cp1, _) = capture(
            dir.path(),
            &vak_config::scope::AgentScope::new(&home),
            objects(),
            "sess-a",
            1,
            "a1",
        )
        .unwrap();
        store(&vak_config::scope::AgentScope::new(&home), objects(), &cp1).unwrap();
        let (cp_other, _) = capture(
            dir.path(),
            &vak_config::scope::AgentScope::new(&home),
            objects(),
            "sess-b",
            0,
            "b0",
        )
        .unwrap();
        store(
            &vak_config::scope::AgentScope::new(&home),
            objects(),
            &cp_other,
        )
        .unwrap();

        assert_eq!(
            list(&vak_config::scope::AgentScope::new(&home), "sess-a")
                .unwrap()
                .len(),
            2
        );
        assert_eq!(
            list(&vak_config::scope::AgentScope::new(&home), "sess-b")
                .unwrap()
                .len(),
            1
        );
        assert_eq!(
            next_seq(&vak_config::scope::AgentScope::new(&home), "sess-a"),
            2
        );
        assert_eq!(
            next_seq(&vak_config::scope::AgentScope::new(&home), "sess-b"),
            1
        );
        assert_eq!(
            next_seq(
                &vak_config::scope::AgentScope::new(&home),
                "sess-never-seen"
            ),
            0
        );
    }

    #[test]
    fn restore_never_deletes_files_capture_could_not_store() {
        let dir = tempfile::tempdir().unwrap();
        // A sibling temp dir, never nested inside `dir` — sessions_home is
        // never inside a real workspace either, and capturing the blob
        // store's own files while walking the workspace would be wrong.
        let home_dir = tempfile::tempdir().unwrap();
        let home = home_dir.path().to_path_buf();
        write(dir.path(), "small.txt", "ok");

        // Oversized: capture skips the CONTENT but must record the path.
        let big = vec![b'x'; 9 * 1024 * 1024];
        std::fs::write(dir.path().join("asset.bin"), &big).unwrap();
        // Secret files are never captured either.
        write(dir.path(), ".env", "SECRET=1");

        let (cp, _) = capture(
            dir.path(),
            &vak_config::scope::AgentScope::new(&home),
            objects(),
            "s",
            0,
            "before",
        )
        .unwrap();
        assert!(
            !cp.files.iter().any(|f| f.rel_path == "asset.bin"),
            "oversized content must not be stored"
        );
        assert!(cp.observed.contains(&"asset.bin".to_string()));
        assert!(cp.observed.contains(&".env".to_string()));

        let (restored, deleted) = restore(dir.path(), objects(), &cp).unwrap();
        assert!(restored >= 1);
        assert_eq!(
            deleted, 0,
            "rewind deleted a file it never stored — data loss"
        );
        assert!(
            dir.path().join("asset.bin").exists(),
            "oversized file destroyed"
        );
        assert_eq!(std::fs::read(dir.path().join("asset.bin")).unwrap(), big);
        assert!(dir.path().join(".env").exists(), "secret file destroyed");
    }

    #[test]
    fn restore_removes_only_files_created_after_checkpoint() {
        let dir = tempfile::tempdir().unwrap();
        // A sibling temp dir, never nested inside `dir` — sessions_home is
        // never inside a real workspace either, and capturing the blob
        // store's own files while walking the workspace would be wrong.
        let home_dir = tempfile::tempdir().unwrap();
        let home = home_dir.path().to_path_buf();
        write(dir.path(), "base.txt", "base");
        let (cp, _) = capture(
            dir.path(),
            &vak_config::scope::AgentScope::new(&home),
            objects(),
            "s",
            0,
            "c",
        )
        .unwrap();

        write(dir.path(), "created-later.txt", "junk");
        restore(dir.path(), objects(), &cp).unwrap();

        assert!(!dir.path().join("created-later.txt").exists());
        assert!(dir.path().join("base.txt").exists());
    }

    #[test]
    fn gitignored_and_secret_files_are_not_captured() {
        let dir = tempfile::tempdir().unwrap();
        // A sibling temp dir, never nested inside `dir` — sessions_home is
        // never inside a real workspace either, and capturing the blob
        // store's own files while walking the workspace would be wrong.
        let home_dir = tempfile::tempdir().unwrap();
        let home = home_dir.path().to_path_buf();
        write(dir.path(), ".gitignore", "secrets/\n*.local\n!keep.local\n");
        write(dir.path(), "src/main.rs", "code");
        write(dir.path(), "secrets/token.txt", "t");
        write(dir.path(), "cfg.local", "x");
        write(dir.path(), "keep.local", "y");
        write(dir.path(), ".env", "K=V");
        write(dir.path(), "server.pem", "pem");

        let (cp, _) = capture(
            dir.path(),
            &vak_config::scope::AgentScope::new(&home),
            objects(),
            "s",
            0,
            "c",
        )
        .unwrap();
        let paths: Vec<&str> = cp.files.iter().map(|f| f.rel_path.as_str()).collect();
        assert!(paths.contains(&"src/main.rs"));
        assert!(paths.contains(&"keep.local"), "negation must un-ignore");
        assert!(!paths.iter().any(|p| p.starts_with("secrets/")));
        assert!(!paths.contains(&"cfg.local"));
        assert!(!paths.contains(&".env"));
        assert!(!paths.contains(&"server.pem"));
    }

    #[test]
    fn store_prunes_old_checkpoints_and_releases_their_contents() {
        let dir = tempfile::tempdir().unwrap();
        // A sibling temp dir, never nested inside `dir` — sessions_home is
        // never inside a real workspace either, and capturing the blob
        // store's own files while walking the workspace would be wrong.
        let home_dir = tempfile::tempdir().unwrap();
        let home = home_dir.path().to_path_buf();

        for seq in 0..25u32 {
            // A distinct, growing file per seq so every checkpoint owns
            // blobs nothing else references, making pruning's GC observable.
            write(dir.path(), "f.txt", &format!("v{seq}"));
            let (cp, _) = capture(
                dir.path(),
                &vak_config::scope::AgentScope::new(&home),
                objects(),
                "s",
                seq,
                "turn",
            )
            .unwrap();
            store(&vak_config::scope::AgentScope::new(&home), objects(), &cp).unwrap();
        }
        let list = list(&vak_config::scope::AgentScope::new(&home), "s").unwrap();
        assert_eq!(list.len(), 20, "old checkpoints must be pruned");
        assert_eq!(list[0].seq, 5, "oldest pruned first");

        let grant = conversation_scope("s");
        let readable = |seq: u32| {
            let body = format!("v{seq}");
            objects()
                .get(
                    &ObjectRef {
                        id: objects().id_of(body.as_bytes()),
                        len: body.len() as u64,
                    },
                    &grant,
                )
                .is_ok()
        };
        assert!(
            (0..5).all(|seq| !readable(seq)),
            "pruned contents are released"
        );
        assert!(
            (5..25).all(readable),
            "each surviving manifest's contents remain"
        );
    }

    #[test]
    fn incremental_capture_reads_only_changed_files() {
        let dir = tempfile::tempdir().unwrap();
        // A sibling temp dir, never nested inside `dir` — sessions_home is
        // never inside a real workspace either, and capturing the blob
        // store's own files while walking the workspace would be wrong.
        let home_dir = tempfile::tempdir().unwrap();
        let home = home_dir.path().to_path_buf();
        let cwd = dir.path().join("work");
        std::fs::create_dir_all(&cwd).unwrap();

        const TOTAL: usize = 2_000;
        const CHANGED: usize = 5;
        for i in 0..TOTAL {
            write(&cwd, &format!("file-{i:04}.txt"), "unchanged content");
        }

        let t0 = std::time::Instant::now();
        let (cp0, stats0) = capture(
            &cwd,
            &vak_config::scope::AgentScope::new(&home),
            objects(),
            "perf",
            0,
            "first",
        )
        .unwrap();
        let first_elapsed = t0.elapsed();
        store(&vak_config::scope::AgentScope::new(&home), objects(), &cp0).unwrap();
        assert_eq!(cp0.files.len(), TOTAL);
        assert_eq!(stats0.files_read, TOTAL, "first capture reads everything");
        assert_eq!(stats0.files_reused, 0);

        // mtime resolution can be as coarse as one second; sleep past it
        // so the changed files are unambiguously newer.
        std::thread::sleep(std::time::Duration::from_millis(1100));
        for i in 0..CHANGED {
            write(&cwd, &format!("file-{i:04}.txt"), "this file was edited");
        }

        let t1 = std::time::Instant::now();
        let (cp1, stats1) = capture(
            &cwd,
            &vak_config::scope::AgentScope::new(&home),
            objects(),
            "perf",
            1,
            "second",
        )
        .unwrap();
        let second_elapsed = t1.elapsed();

        assert_eq!(cp1.files.len(), TOTAL);
        assert_eq!(stats1.files_observed, TOTAL);
        assert_eq!(
            stats1.files_read, CHANGED,
            "only the edited files should be re-read"
        );
        assert_eq!(stats1.files_reused, TOTAL - CHANGED);

        eprintln!(
            "checkpoint capture timings: first={first_elapsed:?} ({TOTAL} files, all read), \
             second={second_elapsed:?} ({CHANGED} changed of {TOTAL}, {} reused)",
            stats1.files_reused
        );
    }
}
