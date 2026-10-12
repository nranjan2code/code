//! Target verification runs in the broker worker, never in the server
//! (docs/design/72-openxml-documents.md, F4), and fails closed.

#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::path::Path;

use vak_ooxml::fixtures;
use vak_sandbox::TargetCheckPlan;

fn worker() -> &'static Path {
    Path::new(env!("CARGO_BIN_EXE_vak-tool-worker"))
}

fn plan(path: &str) -> TargetCheckPlan {
    TargetCheckPlan {
        verifier: "format.openxml".into(),
        path: path.into(),
    }
}

#[tokio::test]
async fn office_candidates_are_verified_in_the_worker() {
    let root = tempfile::tempdir().unwrap();
    std::fs::write(root.path().join("deck.pptx"), fixtures::pptx()).unwrap();
    std::fs::write(
        root.path().join("invoice.docx"),
        fixtures::word_with(
            "application/vnd.ms-word.document.macroEnabled.main+xml",
            fixtures::MINIMAL_WORD_BODY,
            &[],
            &[],
            &[],
            &[],
        ),
    )
    .unwrap();
    let results = vak_tools::broker::verify_targets(
        worker(),
        root.path(),
        &[plan("deck.pptx"), plan("invoice.docx")],
    )
    .await;
    assert_eq!(results.len(), 2);
    assert_eq!(results[0].status, "passed", "{}", results[0].evidence);
    assert!(
        results[0].evidence.contains("2 slides"),
        "{}",
        results[0].evidence
    );
    assert_eq!(results[1].status, "failed");
    assert!(
        results[1].evidence.contains("but is named .docx"),
        "{}",
        results[1].evidence
    );
}

#[tokio::test]
async fn verification_fails_closed_when_the_worker_cannot_run() {
    let root = tempfile::tempdir().unwrap();
    std::fs::write(root.path().join("book.xlsx"), fixtures::xlsx()).unwrap();
    let results = vak_tools::broker::verify_targets(
        Path::new("/nonexistent/vak-tool-worker"),
        root.path(),
        &[plan("book.xlsx")],
    )
    .await;
    assert_eq!(results.len(), 1);
    assert_eq!(results[0].status, "failed");
    assert!(results[0].evidence.starts_with("verification did not run"));
}

#[tokio::test]
async fn checks_outside_the_root_are_refused_by_the_worker() {
    let root = tempfile::tempdir().unwrap();
    let results =
        vak_tools::broker::verify_targets(worker(), root.path(), &[plan("../escape.docx")]).await;
    assert_eq!(results[0].status, "failed", "{}", results[0].evidence);
}
