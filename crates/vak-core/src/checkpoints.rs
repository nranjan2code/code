//! Workspace checkpoints: full-content snapshots captured at turn starts,
//! stored under the sessions home. Rewind restores file contents and removes
//! files created after the checkpoint. Bash mutations are covered because
//! snapshots are state-based, not operation-based.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use walkdir::WalkDir;

const IGNORED_DIRS: [&str; 5] = [".git", "target", "node_modules", ".vakcoder", "dist"];
const MAX_FILE_BYTES: u64 = 8 * 1024 * 1024;
const MAX_TOTAL_BYTES: usize = 64 * 1024 * 1024;

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
    pub files: Vec<FileSnapshot>,
}

fn is_ignored(rel: &Path) -> bool {
    rel.components().any(|c| {
        matches!(
            c,
            std::path::Component::Normal(name) if IGNORED_DIRS
                .contains(&name.to_string_lossy().as_ref())
        )
    })
}

/// Captures every regular file under `cwd` (bounded), sorted by path.
pub fn capture(cwd: &Path, session_id: &str, seq: u32, label: &str) -> std::io::Result<Checkpoint> {
    let mut files = Vec::new();
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
        if !entry.file_type().is_file() {
            continue;
        }
        let Ok(meta) = entry.metadata() else { continue };
        if meta.len() > MAX_FILE_BYTES {
            continue;
        }
        let Ok(abs) = entry.path().canonicalize() else {
            continue;
        };
        let Ok(rel) = abs.strip_prefix(cwd.canonicalize().unwrap_or_else(|_| cwd.into())) else {
            continue;
        };
        if is_ignored(rel) {
            continue;
        }
        total += meta.len() as usize;
        if total > MAX_TOTAL_BYTES {
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
    files.sort_by(|a, b| a.rel_path.cmp(&b.rel_path));
    Ok(Checkpoint {
        seq,
        session_id: session_id.to_string(),
        created_at: chrono::Utc::now(),
        label: label.to_string(),
        files,
    })
}

fn checkpoint_dir(sessions_home: &Path, session_id: &str) -> PathBuf {
    sessions_home.join("checkpoints").join(session_id)
}

/// Persists a checkpoint atomically and returns its file path.
pub fn store(sessions_home: &Path, cp: &Checkpoint) -> std::io::Result<PathBuf> {
    let dir = checkpoint_dir(sessions_home, &cp.session_id);
    std::fs::create_dir_all(&dir)?;
    let path = dir.join(format!("{:04}.json", cp.seq));
    let tmp = dir.join(format!(".{:04}.tmp", cp.seq));
    std::fs::write(&tmp, serde_json::to_vec(cp).map_err(std::io::Error::other)?)?;
    std::fs::rename(&tmp, &path)?;
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

/// Restores the snapshot: rewrites snapshotted files and deletes any
/// tracked-path file that exists now but didn't at capture time.
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

    // Remove files created after the checkpoint (tracked scope only),
    // compared by relative path so symlinked roots (/tmp vs /private/tmp)
    // can't cause false mismatches.
    let snap: std::collections::HashSet<&str> =
        cp.files.iter().map(|f| f.rel_path.as_str()).collect();
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
        if is_ignored(rel) {
            continue;
        }
        if !snap.contains(rel.to_string_lossy().as_ref())
            && std::fs::remove_file(entry.path()).is_ok()
        {
            deleted += 1;
        }
    }
    Ok((restored, deleted))
}
