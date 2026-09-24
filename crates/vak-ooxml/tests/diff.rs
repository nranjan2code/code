//! L4 semantic diff (docs/design/72-openxml-documents.md, P3).

#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::collections::BTreeMap;
use std::io::Cursor;

use vak_ooxml::diff::{ChangeKind, diff};
use vak_ooxml::edit::{self, CellValue, EditContext, OfficeOp, TextValue};
use vak_ooxml::{Limits, fixtures, read};

fn project(bytes: &[u8]) -> read::Document {
    read::read(Cursor::new(bytes.to_vec()), Limits::default()).unwrap()
}

fn edited(bytes: &[u8], ops: Vec<OfficeOp>) -> Vec<u8> {
    let context = EditContext {
        author: "Mira".into(),
        date: "2026-09-24T10:00:00Z".into(),
    };
    edit::apply(bytes, &ops, &context, Limits::default(), None)
        .unwrap()
        .bytes
}

#[test]
fn an_unchanged_file_has_no_changes() {
    let bytes = fixtures::docx();
    assert!(diff(Some(&project(&bytes)), &project(&bytes)).is_empty());
}

#[test]
fn word_changes_are_grouped_under_their_heading() {
    let before = fixtures::docx();
    let after = edited(
        &before,
        vec![
            OfficeOp::ReplaceParagraphText {
                anchor: "p@11".into(),
                text: "Growing.".into(),
            },
            OfficeOp::InsertParagraphAfter {
                anchor: "p@1".into(),
                text: "Details".into(),
                style: None,
            },
        ],
    );
    let result = diff(Some(&project(&before)), &project(&after));
    let rewritten = result
        .changes
        .iter()
        .find(|change| change.anchor == "p@11")
        .unwrap();
    assert_eq!(rewritten.kind, ChangeKind::Changed);
    assert_eq!(rewritten.section, "Outlook");
    assert_eq!(rewritten.before.as_deref(), Some("Steady."));
    assert!(
        rewritten
            .after
            .as_deref()
            .unwrap()
            .contains("[inserted by Mira: Growing.]")
    );
    let added = result
        .changes
        .iter()
        .find(|change| change.kind == ChangeKind::Added)
        .unwrap();
    assert_eq!(added.section, "Summary");
    assert_eq!(
        result.summary,
        vec!["Summary: 1 added", "Outlook: 1 changed"]
    );
}

#[test]
fn excel_changes_are_cells_and_stale_caches_do_not_count() {
    let before = fixtures::xlsx();
    let after = edited(
        &before,
        vec![
            OfficeOp::SetCells {
                sheet: "Budget".into(),
                cells: BTreeMap::from([
                    ("B2".to_string(), CellValue::Number(150.0)),
                    ("F9".to_string(), CellValue::Text("note".into())),
                ]),
            },
            OfficeOp::AddSheet { name: "Q4".into() },
        ],
    );
    let result = diff(Some(&project(&before)), &project(&after));
    let anchors: Vec<(&str, ChangeKind)> = result
        .changes
        .iter()
        .map(|change| (change.anchor.as_str(), change.kind))
        .collect();
    assert_eq!(
        anchors,
        vec![
            ("Budget!B2", ChangeKind::Changed),
            ("Budget!F9", ChangeKind::Added),
            ("Q4!", ChangeKind::Added),
        ],
        "B4's cached value turning stale is not reported as a change"
    );
    let b2 = &result.changes[0];
    assert_eq!(
        (b2.before.as_deref(), b2.after.as_deref()),
        (Some("100"), Some("150"))
    );
}

#[test]
fn deck_changes_report_added_moved_and_retitled_slides() {
    let before = fixtures::pptx();
    let after = edited(
        &before,
        vec![
            OfficeOp::SetPlaceholderText {
                anchor: "slide:256/placeholder:title".into(),
                text: TextValue::One("Launch plan v2".into()),
            },
            OfficeOp::MoveSlide {
                anchor: "slide:257".into(),
                after: None,
            },
        ],
    );
    let result = diff(Some(&project(&before)), &project(&after));
    assert!(
        result
            .changes
            .iter()
            .any(|change| change.kind == ChangeKind::Moved)
    );
    let retitled = result
        .changes
        .iter()
        .find(|change| change.anchor == "slide:256" && change.kind == ChangeKind::Changed)
        .unwrap();
    assert_eq!(retitled.before.as_deref(), Some("Launch plan"));
    assert_eq!(retitled.after.as_deref(), Some("Launch plan v2"));
}

#[test]
fn a_new_file_is_one_added_change() {
    let result = diff(None, &project(&fixtures::pptx()));
    assert_eq!(result.changes.len(), 1);
    assert!(
        result.changes[0]
            .after
            .as_deref()
            .unwrap()
            .starts_with("new file: 2 slides")
    );
}
