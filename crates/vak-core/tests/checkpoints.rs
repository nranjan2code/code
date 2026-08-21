#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::fs;
use std::path::Path;

use tempfile::tempdir;

use vak_core::checkpoints;

fn write(p: &Path, rel: &str, content: &str) {
    let target = p.join(rel);
    fs::create_dir_all(target.parent().unwrap()).unwrap();
    fs::write(target, content).unwrap();
}

#[test]
fn capture_includes_files_and_skips_ignored_dirs() {
    let dir = tempdir().unwrap();
    write(dir.path(), "src/main.rs", "fn main() {}");
    write(dir.path(), "README.md", "readme");
    write(dir.path(), "target/debug/blob.o", "binary junk");
    write(dir.path(), ".git/config", "gitconfig");

    let cp = checkpoints::capture(dir.path(), "s1", 0, "initial").unwrap();
    let paths: Vec<&str> = cp.files.iter().map(|f| f.rel_path.as_str()).collect();
    assert!(paths.contains(&"src/main.rs"));
    assert!(paths.contains(&"README.md"));
    assert!(!paths.iter().any(|p| p.starts_with("target/")));
    assert!(!paths.iter().any(|p| p.starts_with(".git/")));
}

#[test]
fn store_list_load_roundtrip_preserves_content() {
    let dir = tempdir().unwrap();
    let home = dir.path().join("home");
    let binary: &[u8] = &[0u8, 1, 2, b'b', b'i', b'n', 0xff];
    fs::write(dir.path().join("data.bin"), binary).unwrap();

    let cp = checkpoints::capture(dir.path(), "sess", 3, "third").unwrap();
    checkpoints::store(&home, &cp).unwrap();

    let list = checkpoints::list(&home, "sess").unwrap();
    assert_eq!(list.len(), 1);
    assert_eq!(list[0].seq, 3);

    let loaded = checkpoints::load(&home, "sess", 3).unwrap();
    assert_eq!(
        loaded.files[0].content,
        binary.to_vec(),
        "binary content must round-trip via base64"
    );
}

#[test]
fn restore_reverts_edits_and_removes_new_files() {
    let dir = tempdir().unwrap();
    let home = dir.path().join("home");

    // State at checkpoint time.
    write(dir.path(), "keep.txt", "original");
    write(dir.path(), "src/lib.rs", "old code");
    let cp = checkpoints::capture(dir.path(), "s", 0, "before").unwrap();
    checkpoints::store(&home, &cp).unwrap();

    // Mutate after the checkpoint: edit one file, delete another, add a third.
    write(dir.path(), "src/lib.rs", "rewritten!");
    fs::remove_file(dir.path().join("keep.txt")).unwrap();
    write(dir.path(), "created-later.txt", "new junk");

    let restored_cp = checkpoints::load(&home, "s", 0).unwrap();
    let (restored, deleted) = checkpoints::restore(dir.path(), &restored_cp).unwrap();

    assert!(restored >= 2);
    assert_eq!(
        fs::read_to_string(dir.path().join("src/lib.rs")).unwrap(),
        "old code",
        "edited file must revert"
    );
    assert_eq!(
        fs::read_to_string(dir.path().join("keep.txt")).unwrap(),
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
    let dir = tempdir().unwrap();
    let home = dir.path().join("home");
    write(dir.path(), "a.txt", "a");

    let cp0 = checkpoints::capture(dir.path(), "sess-a", 0, "a0").unwrap();
    checkpoints::store(&home, &cp0).unwrap();
    let cp1 = checkpoints::capture(dir.path(), "sess-a", 1, "a1").unwrap();
    checkpoints::store(&home, &cp1).unwrap();
    let cp_other = checkpoints::capture(dir.path(), "sess-b", 0, "b0").unwrap();
    checkpoints::store(&home, &cp_other).unwrap();

    assert_eq!(checkpoints::list(&home, "sess-a").unwrap().len(), 2);
    assert_eq!(checkpoints::list(&home, "sess-b").unwrap().len(), 1);
}
