#![allow(clippy::unwrap_used)]
#[test]
fn integrity_verifier_misses_broken_manifest() {
    let root = tempfile::tempdir().unwrap();
    let set = vak_storage::segments::SegmentSet::open(root.path()).unwrap();
    let lock = set.lock().unwrap();
    let mut writer = set.writer(1, &lock).unwrap();
    writer.append(b"synthetic first row", None).unwrap();
    writer.append(b"synthetic second row", None).unwrap();
    drop(writer);
    set.seal(1, &lock).unwrap();
    drop(lock);
    let manifest = root.path().join("seals.log");
    let mut bytes = std::fs::read(&manifest).unwrap();
    let last = bytes.len()-1;
    bytes[last] ^= 1;
    std::fs::write(manifest, bytes).unwrap();
    assert!(set.read(1, None).is_err(), "authoritative reader must reject corruption");
    let checked = vak_session::verify::verify(root.path());
    assert_eq!(checked.broken_segment, None, "verifier unexpectedly caught corruption");
    assert_eq!(checked.frames, 2);
    println!("CONFIRMED: verifier reports two intact frames/no broken segment despite unreadable seal manifest");
}
