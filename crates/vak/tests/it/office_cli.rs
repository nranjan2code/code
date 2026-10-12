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

#[test]
fn a_file_is_created_from_scratch_verified_and_read_from_the_command_line() {
    let dir = tempfile::tempdir().unwrap();
    let cases = [
        (
            "memo.docx",
            r#"[{"op":"add_paragraph","text":"Q3 review","style":"Title"},
                {"op":"add_paragraph","text":"Steps","style":"Heading 1"},
                {"op":"add_paragraph","text":"Draft the budget","style":"List Number"},
                {"op":"add_table","rows":[["Region","Revenue"],["North",120]]}]"#,
            "Word document",
        ),
        (
            "budget.xlsx",
            r##"[{"op":"rename_sheet","sheet":"Sheet1","name":"Budget"},
                 {"op":"set_cells","sheet":"Budget","cells":{"A1":"Item","B1":"Cost","A2":"Rent","B2":1200,"B3":"=SUM(B2:B2)"}},
                 {"op":"format_cells","sheet":"Budget","range":"A1:B1","bold":true,"fill":"#D9E2F3"},
                 {"op":"set_column_widths","sheet":"Budget","widths":{"A":24}}]"##,
            "Excel workbook",
        ),
        (
            "launch.pptx",
            r#"[{"op":"add_slide_from_layout","layout":"Title Slide","placeholders":{"title":"Launch plan","subtitle":"October"}},
                {"op":"add_slide_from_layout","layout":"Title and Content","placeholders":{"title":"Goals","body":["Ship v1","Sign ten partners"]},"notes":"Keep it short."}]"#,
            "PowerPoint presentation",
        ),
    ];
    for (name, ops, kind) in cases {
        let created = json(&vak(
            dir.path(),
            &["office", "apply", "--ops", "-", "--out", name],
            Some(ops),
        ));
        assert!(
            created["changes"]["changes"][0]["after"]
                .as_str()
                .unwrap()
                .starts_with("new file: "),
            "{created}"
        );
        let verified = json(&vak(dir.path(), &["office", "verify", name], None));
        assert_eq!(verified["passed"], true, "{name}: {verified}");
        let facts = json(&vak(dir.path(), &["office", "read", name, "--facts"], None));
        assert_eq!(facts["kind"], kind, "{facts}");
        assert!(
            facts["flags"].as_array().is_none_or(Vec::is_empty),
            "{name}: {facts}"
        );
    }

    let digest_without_file = vak(
        dir.path(),
        &[
            "office",
            "apply",
            "--ops",
            "-",
            "--base-digest",
            "0123456789abcdef",
            "--out",
            "other.docx",
        ],
        Some(r#"[{"op":"add_paragraph","text":"x"}]"#),
    );
    assert_eq!(digest_without_file.status.code(), Some(2));
    assert!(String::from_utf8_lossy(&digest_without_file.stderr).contains("give the file"));
    assert!(!dir.path().join("other.docx").exists());
}

#[test]
fn read_create_edit_diff_and_verify_a_pdf_from_the_command_line() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("report.pdf"), vak_pdf::fixtures::report()).unwrap();

    let facts = json(&vak(
        dir.path(),
        &["office", "read", "report.pdf", "--facts"],
        None,
    ));
    assert_eq!(facts["vocabulary"], "pdf", "{facts}");
    assert!(facts["units"].as_array().unwrap().is_empty());
    assert!(
        facts["flags"]
            .to_string()
            .contains("JavaScript action (never run)"),
        "{facts}"
    );
    let digest = facts["sha256"].as_str().unwrap()[..16].to_string();

    let at = json(&vak(
        dir.path(),
        &["office", "read", "report.pdf", "--at", "page:2/line:1"],
        None,
    ));
    assert_eq!(at["focus"], "page:2/line:1");
    assert!(at["units"].to_string().contains("Appendix"), "{at}");

    let edited = json(&vak(
        dir.path(),
        &[
            "office",
            "apply",
            "report.pdf",
            "--ops",
            "-",
            "--base-digest",
            &digest,
            "--out",
            "revised.pdf",
        ],
        Some(
            r#"[{"op":"replace_paragraph_text","anchor":"page:1/line:2","text":"Revenue grew 15%"}]"#,
        ),
    ));
    assert!(
        edited["results"][0]
            .as_str()
            .unwrap()
            .contains("Revenue grew 15%"),
        "{edited}"
    );
    assert_eq!(
        edited["changes"]["changes"][0]["kind"], "changed",
        "{edited}"
    );

    let diff = json(&vak(
        dir.path(),
        &["office", "diff", "report.pdf", "revised.pdf"],
        None,
    ));
    assert!(
        diff["changes"].to_string().contains("Revenue grew 12%"),
        "{diff}"
    );

    let created = json(&vak(
        dir.path(),
        &["office", "apply", "--ops", "-", "--out", "new.pdf"],
        Some(r#"[{"op":"add_paragraph","text":"Hello","style":"Title"}]"#),
    ));
    assert_eq!(
        created["changes"]["changes"][0]["kind"], "added",
        "{created}"
    );

    for name in ["report.pdf", "revised.pdf", "new.pdf"] {
        let verified = json(&vak(dir.path(), &["office", "verify", name], None));
        assert_eq!(verified["passed"], true, "{name}: {verified}");
    }
    std::fs::write(dir.path().join("fake.pdf"), b"not a pdf").unwrap();
    let refused = vak(dir.path(), &["office", "verify", "fake.pdf"], None);
    assert_eq!(refused.status.code(), Some(1));
    assert!(String::from_utf8_lossy(&refused.stdout).contains("not a PDF"));
}
