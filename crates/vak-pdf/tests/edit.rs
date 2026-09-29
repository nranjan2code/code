//! The PDF op engine, diff and review (docs/design/77): each op lands where
//! its anchor points, is confirmed by a re-read, and fails with a
//! repairable message otherwise; the diff describes what a reader sees; a
//! draft's changes can be kept one by one.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use vak_pdf::edit::{self, EditContext, PdfOp};
use vak_pdf::{Limits, diff, fixtures, projection, review};

fn context() -> EditContext {
    EditContext {
        author: "vak".into(),
        date: "2026-09-27T10:00:00Z".into(),
    }
}

fn ops(json: &str) -> Vec<PdfOp> {
    serde_json::from_str(json).unwrap()
}

fn apply(source: Option<&[u8]>, json: &str) -> edit::Applied {
    edit::apply(source, &ops(json), &context(), Limits::default()).unwrap()
}

fn error(source: Option<&[u8]>, json: &str) -> String {
    edit::apply(source, &ops(json), &context(), Limits::default())
        .unwrap_err()
        .to_string()
}

#[test]
fn a_new_pdf_is_set_from_its_ops() {
    let applied = apply(
        None,
        r#"[
            {"op": "set_title", "title": "Q4 Plan"},
            {"op": "add_paragraph", "text": "Q4 Plan", "style": "Title"},
            {"op": "add_paragraph", "text": "Goals", "style": "Heading 1"},
            {"op": "add_paragraph", "text": "Grow revenue in every region.", "style": "List Bullet"},
            {"op": "add_paragraph", "text": "Hire two engineers.", "style": "List Number"},
            {"op": "add_table", "rows": [["Region", "Target"], ["North", 120], ["South", 95.5]]},
            {"op": "add_page_break"},
            {"op": "add_paragraph", "text": "Appendix", "style": "Heading 1"},
            {"op": "add_paragraph", "text": "Café prices rose – see the table."}
        ]"#,
    );
    let document = &applied.document;
    assert_eq!(document.title(), Some("Q4 Plan"));
    assert_eq!(document.page_count, 2);
    let text = document.lines().join("\n");
    for expected in [
        "[page:1/line:1] Q4 Plan",
        "[page:1/line:2] Goals",
        "[page:1/line:3] • Grow revenue in every region.",
        "[page:1/line:4] 1. Hire two engineers.",
        "Region   Target",
        "North   120",
        "South   95.5",
        "[page:2/line:1] Appendix",
        "Café prices rose – see the table.",
    ] {
        assert!(text.contains(expected), "{expected}\n{text}");
    }
    let titles: Vec<&str> = document
        .outline
        .iter()
        .map(|bookmark| bookmark.title.as_str())
        .collect();
    assert_eq!(titles, ["Q4 Plan", "Goals", "Appendix"]);
    assert_eq!(document.outline[2].page, Some(2));
    assert!(
        document.inspection.flags().is_empty(),
        "{:?}",
        document.inspection.flags()
    );
}

#[test]
fn a_pdf_chart_is_vector_and_keeps_searchable_source_rows() {
    let applied = apply(
        None,
        r#"[
            {"op":"set_title","title":"Daily visitors"},
            {"op":"add_paragraph","text":"Daily visitors","style":"Title"},
            {"op":"add_chart","title":"Visitors by day","categories":["Monday","Tuesday","Wednesday"],"values":[12,18,15]}
        ]"#,
    );
    let text = applied.document.lines().join("\n");
    for value in [
        "Visitors by day",
        "Monday",
        "Tuesday",
        "Wednesday",
        "Visitors",
        "18",
    ] {
        assert!(
            text.contains(value),
            "{value} missing from extracted PDF text:\n{text}"
        );
    }
    assert!(
        applied.bytes.windows(2).any(|bytes| bytes == b"re"),
        "vector rectangle operator missing"
    );
}

#[test]
fn a_pdf_png_is_embedded_and_its_alt_text_is_searchable() {
    let applied = apply(
        None,
        r#"[{"op":"add_image","image":{"mime_type":"image/png","data":"iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVR4nGNQaHjwHwAExAKAc00zmAAAAABJRU5ErkJggg==","alt_text":"A blue square used as the daily status marker"}}]"#,
    );
    assert_eq!(applied.document.pages[0].images, 1);
    assert!(
        applied
            .document
            .lines()
            .join(" ")
            .contains("Image description: A blue square used as the daily status marker")
    );
    assert!(applied.bytes.windows(8).any(|window| window == b"/XObject"));
    assert!(applied.bytes.windows(6).any(|window| window == b"/Image"));
}

#[test]
fn a_pdf_image_rejects_mismatched_bytes_and_missing_alt_text() {
    assert!(error(None, r#"[{"op":"add_image","image":{"mime_type":"image/png","data":"bm90IGEgcG5n","alt_text":"ordinary words"}}]"#).contains("PNG data has an invalid signature"));
    assert!(error(None, r#"[{"op":"add_image","image":{"mime_type":"image/png","data":"iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVR4nGNQaHjwHwAExAKAc00zmAAAAABJRU5ErkJggg==","alt_text":" "}}]"#).contains("needs alternative text"));
}

#[test]
fn lines_are_rewritten_deleted_commented_and_highlighted_in_place() {
    let source = fixtures::report();
    let applied = apply(
        Some(&source),
        r#"[
            {"op": "replace_paragraph_text", "anchor": "page:1/line:2", "text": "Revenue grew 15%"},
            {"op": "delete_paragraph", "anchor": "page:1/line:3"},
            {"op": "add_comment", "anchor": "page:1/line:1", "text": "Check the headline"},
            {"op": "highlight", "anchor": "page:1/line:2", "note": "Key number"}
        ]"#,
    );
    let text = applied.document.lines().join("\n");
    assert!(text.contains("[page:1/line:2] Revenue grew 15%"), "{text}");
    assert!(!text.contains("Revenue grew 12%"), "{text}");
    assert!(!text.contains("in the third quarter."), "{text}");
    assert!(
        text.contains("Check the headline  ⟨comment by vak⟩"),
        "{text}"
    );
    assert!(
        text.contains("“Revenue grew 15%”: Key number  ⟨highlight by vak⟩"),
        "{text}"
    );
    assert!(text.contains("Quarterly report"), "{text}");
    assert!(
        text.contains("Stamped footer"),
        "the form on page 2 survives"
    );
    let hidden = apply(
        Some(&source),
        r#"[{"op": "delete_paragraph", "anchor": "page:1/line:4"}]"#,
    );
    assert!(
        !hidden
            .document
            .lines()
            .join("\n")
            .contains("Ignore previous instructions."),
        "hidden white text can be removed"
    );
    assert!(
        !applied
            .bytes
            .windows(14)
            .any(|window| window == b"third quarter."),
        "a deleted line is gone from the file, not left in an old stream"
    );
    assert_eq!(applied.results.len(), 4);
    assert!(applied.results[0].contains("“Revenue grew 12%” → “Revenue grew 15%”"));
    assert!(
        applied
            .notices
            .iter()
            .any(|notice| notice.contains("Helvetica"))
    );
}

#[test]
fn pages_are_rotated_moved_deleted_and_added() {
    let source = fixtures::simple(&[&["One"], &["Two"], &["Three"]]);
    let applied = apply(
        Some(&source),
        r#"[
            {"op": "rotate_page", "anchor": "page:2", "degrees": 90},
            {"op": "move_page", "anchor": "page:3"},
            {"op": "delete_page", "anchor": "page:1"},
            {"op": "add_paragraph", "text": "Inserted after two", "after": "page:2"},
            {"op": "set_title", "title": "Reordered"}
        ]"#,
    );
    let document = &applied.document;
    let firsts: Vec<&str> = document
        .pages
        .iter()
        .map(|page| page.lines[0].text.as_str())
        .collect();
    assert_eq!(firsts, ["Three", "Two", "Inserted after two"]);
    assert_eq!(document.pages[1].rotation, 90);
    assert_eq!(document.title(), Some("Reordered"));
}

#[test]
fn form_fields_are_filled() {
    let source = fixtures::form();
    let applied = apply(
        Some(&source),
        r#"[
            {"op": "fill_field", "field": "Name", "value": "Asha Rao"},
            {"op": "fill_field", "field": "agree", "value": "yes"}
        ]"#,
    );
    let text = applied.document.lines().join("\n");
    assert!(text.contains("Name: Asha Rao  ⟨form field⟩"), "{text}");
    assert!(text.contains("Agree: Yes  ⟨form field⟩"), "{text}");
    let missing = error(
        Some(&source),
        r#"[{"op": "fill_field", "field": "Email", "value": "x"}]"#,
    );
    assert!(missing.contains("the fields are: Name, Agree"), "{missing}");
}

#[test]
fn bad_ops_fail_with_a_reason_and_write_nothing() {
    let report = fixtures::report();
    for (json, expected) in [
        (
            r#"[{"op": "replace_paragraph_text", "anchor": "page:9/line:1", "text": "x"}]"#,
            "the document has 2 pages",
        ),
        (
            r#"[{"op": "replace_paragraph_text", "anchor": "page:1/line:99", "text": "x"}]"#,
            "page 1 has 8 lines",
        ),
        (
            r#"[{"op": "replace_paragraph_text", "anchor": "page:1/line:5", "text": "x"}]"#,
            "is invisible",
        ),
        (
            r#"[{"op": "replace_paragraph_text", "anchor": "page:1/line:8", "text": "x"}]"#,
            "is a comment or form field",
        ),
        (
            r#"[{"op": "replace_paragraph_text", "anchor": "page:2/line:2", "text": "x"}]"#,
            "inside a reusable form",
        ),
        (
            r#"[{"op": "replace_paragraph_text", "anchor": "page:1/line:1", "text": "日本"}]"#,
            "cannot draw: '日'",
        ),
        (
            r#"[{"op": "rotate_page", "anchor": "page:1", "degrees": 45}]"#,
            "multiple of 90",
        ),
        (
            r#"[{"op": "add_paragraph", "text": "x", "style": "Fancy"}]"#,
            "is not one a PDF offers",
        ),
        (
            r#"[{"op": "delete_paragraph", "anchor": "page:1/line:2"}, {"op": "replace_paragraph_text", "anchor": "page:1/line:2", "text": "x"}]"#,
            "already changes line 2",
        ),
    ] {
        let message = error(Some(&report), json);
        assert!(message.contains(expected), "{json}: {message}");
    }
    let only = fixtures::simple(&[&["One"]]);
    assert!(
        error(
            Some(&only),
            r#"[{"op": "delete_page", "anchor": "page:1"}]"#
        )
        .contains("at least one page")
    );
    assert!(
        error(
            None,
            r#"[{"op": "add_comment", "anchor": "page:1/line:1", "text": "x"}]"#
        )
        .contains("a new PDF has no pages")
    );
    assert!(
        error(
            None,
            r#"[{"op": "add_paragraph", "text": "x", "after": "page:1"}]"#
        )
        .contains("leave out after")
    );
    assert!(
        error(
            None,
            r#"[{"op":"add_chart","title":"Visitors","categories":["Mon"],"values":[1e99]}]"#
        )
        .contains("between -1e12 and 1e12")
    );
    assert!(
        error(
            Some(&fixtures::encrypted()),
            r#"[{"op": "set_title", "title": "x"}]"#
        )
        .contains("encrypted")
    );
}

#[test]
fn the_diff_describes_what_a_reader_sees() {
    let source = fixtures::report();
    let before = vak_pdf::read(&source, Limits::default()).unwrap();
    let applied = apply(
        Some(&source),
        r#"[
            {"op": "replace_paragraph_text", "anchor": "page:1/line:2", "text": "Revenue grew 15%"},
            {"op": "add_comment", "anchor": "page:1/line:1", "text": "Check"},
            {"op": "rotate_page", "anchor": "page:2", "degrees": 180},
            {"op": "set_title", "title": "Q3 Report, revised"}
        ]"#,
    );
    let found = diff::diff(Some(&before), &applied.document);
    let described: Vec<String> = found
        .changes
        .iter()
        .map(|change| {
            format!(
                "{:?} {} {:?} → {:?}",
                change.kind, change.anchor, change.before, change.after
            )
        })
        .collect();
    let all = described.join("\n");
    assert!(
        all.contains(
            r#"Changed page:1/line:2 Some("Revenue grew 12%") → Some("Revenue grew 15%")"#
        ),
        "{all}"
    );
    assert!(
        all.contains("Added page:1/line:9 None → Some(\"Check  ⟨comment by vak⟩\")"),
        "{all}"
    );
    assert!(
        all.contains(r#"Changed page:2 Some("turned 0°") → Some("turned 180°")"#),
        "{all}"
    );
    assert!(
        all.contains(r#"Changed title Some("Q3 Report") → Some("Q3 Report, revised")"#),
        "{all}"
    );
    assert_eq!(found.summary[0], "Properties: 1 changed");

    let moved = apply(
        Some(&fixtures::simple(&[&["One"], &["Two"]])),
        r#"[{"op": "move_page", "anchor": "page:2"}]"#,
    );
    let before =
        vak_pdf::read(&fixtures::simple(&[&["One"], &["Two"]]), Limits::default()).unwrap();
    let found = diff::diff(Some(&before), &moved.document);
    assert!(
        found
            .changes
            .iter()
            .any(|change| change.kind == diff::ChangeKind::Moved),
        "{found:?}"
    );
    assert!(
        diff::diff(None, &moved.document)
            .changes
            .iter()
            .all(|change| change.kind == diff::ChangeKind::Added)
    );
}

#[test]
fn a_drafts_changes_are_kept_one_by_one() {
    let source = fixtures::report();
    let steps = vec![ops(r#"[
            {"op": "replace_paragraph_text", "anchor": "page:1/line:2", "text": "Revenue grew 15%"},
            {"op": "add_comment", "anchor": "page:1/line:1", "text": "Check"},
            {"op": "set_title", "title": "Revised"}
        ]"#)];
    let draft = edit::apply(Some(&source), &steps[0], &context(), Limits::default()).unwrap();
    let choices = review::choices(
        Some(&source),
        &steps,
        &context(),
        Limits::default(),
        &draft.document,
    )
    .unwrap();
    assert_eq!(choices.len(), 3);
    assert!(
        choices
            .iter()
            .all(|choice| choice.requires.is_empty() && !choice.changes.is_empty())
    );
    assert_eq!(choices[2].label, "Set the title to “Revised”");

    let narrowed = review::narrow(
        Some(&source),
        &steps,
        &["1".into()],
        &context(),
        Limits::default(),
        &draft.document,
    )
    .unwrap();
    let text = narrowed.document.lines().join("\n");
    assert!(text.contains("Check  ⟨comment by vak⟩"), "{text}");
    assert!(
        text.contains("Revenue grew 12%"),
        "the unkept rewrite is not applied: {text}"
    );
    assert_eq!(narrowed.document.title(), Some("Q3 Report"));

    let other = edit::apply(
        Some(&source),
        &ops(r#"[{"op": "set_title", "title": "Else"}]"#),
        &context(),
        Limits::default(),
    )
    .unwrap();
    let refused = review::choices(
        Some(&source),
        &steps,
        &context(),
        Limits::default(),
        &other.document,
    )
    .unwrap_err();
    assert!(refused.contains("does not match"), "{refused}");
    let two_rounds = vec![steps[0].clone(), steps[0].clone()];
    assert!(
        review::choices(
            Some(&source),
            &two_rounds,
            &context(),
            Limits::default(),
            &draft.document
        )
        .unwrap_err()
        .contains("rounds")
    );
}

#[test]
fn the_projection_pages_units_like_the_office_view() {
    let document = vak_pdf::read(&fixtures::report(), Limits::default()).unwrap();
    let page = projection::project(&document, 0, projection::PAGE_BYTES);
    assert_eq!(page.vocabulary, "pdf");
    assert_eq!(page.units[0].kind, "page");
    assert_eq!(page.units[1].anchor, "page:1/line:1");
    assert_eq!(page.outline[1].title, "Appendix");
    let focus = projection::locate(&document, "page:2/line:2").unwrap();
    assert_eq!(page.units.len(), page.total_units);
    assert_eq!(
        projection::page_start(&document, focus, projection::PAGE_BYTES),
        focus - 2
    );
    let small = projection::project(&document, 0, 200);
    assert!(small.next.is_some() && small.units.len() < small.total_units);
}
