#![allow(clippy::unwrap_used, clippy::expect_used)]

//! `vak office` (docs/design/72, P5): each verb prints the worker's JSON,
//! apply never changes its source or overwrites, and verify's exit code
//! is the verdict.

use std::path::Path;
use std::process::{Command, Output};

fn vak(dir: &Path, args: &[&str], stdin: Option<&str>) -> Output {
    use std::io::Write as _;
    let mut child = Command::new(env!("CARGO_BIN_EXE_vak"))
        .current_dir(dir)
        .args(args)
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .unwrap();
    if let Some(text) = stdin {
        child
            .stdin
            .take()
            .unwrap()
            .write_all(text.as_bytes())
            .unwrap();
    }
    drop(child.stdin.take());
    child.wait_with_output().unwrap()
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
fn read_apply_diff_and_verify_a_workbook_from_the_command_line() {
    let dir = tempfile::tempdir().unwrap();
    let source = vak_ooxml::fixtures::xlsx();
    std::fs::write(dir.path().join("budget.xlsx"), &source).unwrap();

    let facts = json(&vak(
        dir.path(),
        &["office", "read", "budget.xlsx", "--facts"],
        None,
    ));
    assert_eq!(facts["kind"], "Excel workbook");
    assert!(facts["units"].as_array().unwrap().is_empty());
    let digest = facts["sha256"].as_str().unwrap()[..16].to_string();

    let ops = r#"[{"op":"set_cells","sheet":"Budget","cells":{"B2":150}}]"#;
    let applied = json(&vak(
        dir.path(),
        &[
            "office",
            "apply",
            "budget.xlsx",
            "--ops",
            "-",
            "--base-digest",
            &digest,
            "--out",
            "budget-2.xlsx",
        ],
        Some(ops),
    ));
    assert_eq!(applied["results"][0]["op"], "set_cells");
    assert_eq!(
        applied["changes"]["summary"][0], "Budget: 1 changed",
        "{applied}"
    );
    assert_eq!(
        std::fs::read(dir.path().join("budget.xlsx")).unwrap(),
        source,
        "the source is unchanged"
    );

    let again = vak(
        dir.path(),
        &[
            "office",
            "apply",
            "budget.xlsx",
            "--ops",
            "-",
            "--base-digest",
            &digest,
            "--out",
            "budget-2.xlsx",
        ],
        Some(ops),
    );
    assert_eq!(again.status.code(), Some(2));
    assert!(String::from_utf8_lossy(&again.stderr).contains("already exists"));

    let stale = vak(
        dir.path(),
        &[
            "office",
            "apply",
            "budget.xlsx",
            "--ops",
            "-",
            "--base-digest",
            "0000000000000000",
            "--out",
            "budget-3.xlsx",
        ],
        Some(ops),
    );
    assert_eq!(stale.status.code(), Some(2));
    assert!(!dir.path().join("budget-3.xlsx").exists());

    let diff = json(&vak(
        dir.path(),
        &["office", "diff", "budget.xlsx", "budget-2.xlsx"],
        None,
    ));
    assert_eq!(diff["changes"][0]["anchor"], "Budget!B2", "{diff}");

    let verified = json(&vak(
        dir.path(),
        &["office", "verify", "budget-2.xlsx"],
        None,
    ));
    assert_eq!(verified["passed"], true);
    std::fs::copy(
        dir.path().join("budget-2.xlsx"),
        dir.path().join("budget.docx"),
    )
    .unwrap();
    let renamed = vak(dir.path(), &["office", "verify", "budget.docx"], None);
    assert_eq!(
        renamed.status.code(),
        Some(1),
        "a workbook named .docx fails"
    );
}
