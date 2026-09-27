#![allow(clippy::unwrap_used, clippy::expect_used)]

//! `vak pdf` (docs/design/77): read prints the worker's JSON and verify's
//! exit code is the verdict.

use std::path::Path;
use std::process::{Command, Output};

fn vak(dir: &Path, args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_vak"))
        .current_dir(dir)
        .args(args)
        .output()
        .unwrap()
}

fn json(output: &Output) -> serde_json::Value {
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    serde_json::from_slice(&output.stdout).unwrap()
}

#[test]
fn read_and_verify_a_pdf_from_the_command_line() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("report.pdf"), vak_pdf::fixtures::report()).unwrap();

    let facts = json(&vak(dir.path(), &["pdf", "read", "report.pdf", "--facts"]));
    assert_eq!(facts["page_count"], 2);
    assert!(facts["pages"].as_array().unwrap().is_empty());
    assert_eq!(facts["sha256"].as_str().unwrap().len(), 64);
    let flags = facts["flags"].to_string();
    assert!(flags.contains("JavaScript action (never run)"), "{flags}");

    let at = json(&vak(
        dir.path(),
        &["pdf", "read", "report.pdf", "--at", "page:2/line:1"],
    ));
    assert_eq!(at["focus"], "page:2/line:1");
    assert_eq!(at["pages"][0]["number"], 2);
    assert_eq!(at["pages"][0]["lines"][0]["text"], "Appendix");

    let all = json(&vak(dir.path(), &["pdf", "read", "report.pdf"]));
    assert_eq!(all["pages"].as_array().unwrap().len(), 2);
    assert!(all["next_page"].is_null());
    assert_eq!(all["pages"][0]["lines"][3]["labels"][0], "white");

    let verified = vak(dir.path(), &["pdf", "verify", "report.pdf"]);
    assert!(
        verified.status.success(),
        "{}",
        String::from_utf8_lossy(&verified.stdout)
    );
    std::fs::write(dir.path().join("fake.pdf"), b"not a pdf").unwrap();
    let refused = vak(dir.path(), &["pdf", "verify", "fake.pdf"]);
    assert_eq!(refused.status.code(), Some(1));
    assert!(String::from_utf8_lossy(&refused.stdout).contains("not a PDF"));
}
