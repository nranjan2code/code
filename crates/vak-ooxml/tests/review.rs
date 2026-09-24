//! Keeping some of a draft's changes (docs/design/72-openxml-documents.md,
//! P3): the choices a draft offers, what each requires, and that a narrower
//! draft is a replay of exactly the kept ops against the same source.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::collections::BTreeMap;

use vak_ooxml::Limits;
use vak_ooxml::diff::ChangeKind;
use vak_ooxml::edit::{self, CellValue, EditContext, OfficeOp, TextValue};
use vak_ooxml::fixtures;
use vak_ooxml::review::{self, Choice};

fn context() -> EditContext {
    EditContext {
        author: "Mira".into(),
        date: "2026-09-24T10:00:00Z".into(),
    }
}

fn draft(source: &[u8], ops: &[OfficeOp]) -> edit::Applied {
    edit::apply(source, ops, &context(), Limits::default(), None).unwrap()
}

fn choices(source: &[u8], ops: &[OfficeOp]) -> Vec<Choice> {
    let draft = draft(source, ops);
    review::choices(
        source,
        ops,
        &context(),
        Limits::default(),
        None,
        &draft.document,
    )
    .unwrap()
}

fn narrow(source: &[u8], ops: &[OfficeOp], keep: &[&str]) -> Result<String, String> {
    let keep: Vec<String> = keep.iter().map(|id| id.to_string()).collect();
    review::narrow(source, ops, &keep, &context(), Limits::default(), None)
        .map(|applied| applied.document.lines().join("\n"))
        .map_err(|error| error.to_string())
}

fn insert(anchor: &str, text: &str) -> OfficeOp {
    OfficeOp::InsertParagraphAfter {
        anchor: anchor.into(),
        text: text.into(),
        style: None,
    }
}

#[test]
fn a_kept_paragraph_still_follows_the_paragraph_it_was_written_after() {
    let source = fixtures::docx();
    // Op 4 names the paragraph op 2 minted (the second new id). Leaving op 1
    // out shifts every later new id down by one, so a replay that did not
    // remap would put op 4's text after op 3's paragraph instead.
    let ops = vec![
        insert("p@1", "Alpha."),
        insert("p@11", "Bravo."),
        insert("p@3", "Charlie."),
        insert("p:1A000002", "Delta."),
    ];
    let offered = choices(&source, &ops);
    let ids: Vec<&str> = offered.iter().map(|choice| choice.id.as_str()).collect();
    assert_eq!(ids, ["0", "1", "2", "3"]);
    assert_eq!(offered[3].requires, ["1"]);
    assert!(offered[..3].iter().all(|choice| choice.requires.is_empty()));
    assert_eq!(offered[0].label, "New paragraph after p@1");
    assert_eq!(offered[0].changes.len(), 1);
    assert_eq!(offered[0].changes[0].kind, ChangeKind::Added);

    let text = narrow(&source, &ops, &["1", "2", "3"]).unwrap();
    assert!(!text.contains("Alpha."), "{text}");
    let bravo = text.find("Bravo.").expect("Bravo kept");
    let delta = text.find("Delta.").expect("Delta kept");
    let charlie = text.find("Charlie.").expect("Charlie kept");
    let after_bravo = &text[bravo..];
    let next_line = after_bravo.lines().nth(1).unwrap_or_default();
    assert!(next_line.contains("Delta."), "Delta follows Bravo:\n{text}");
    assert!(charlie < bravo && bravo < delta, "{text}");

    let error = narrow(&source, &ops, &["0", "3"]).unwrap_err();
    assert!(
        error.contains("builds on edit 2 (New paragraph after p@11)"),
        "{error}"
    );
    let error = narrow(&source, &ops, &[]).unwrap_err();
    assert!(error.contains("no change was kept"), "{error}");
    let error = narrow(&source, &ops, &["9"]).unwrap_err();
    assert!(error.contains("\"9\" is not a change"), "{error}");
}

#[test]
fn cells_are_chosen_one_at_a_time_and_a_new_sheet_carries_its_cells() {
    let source = fixtures::xlsx();
    let ops = vec![
        OfficeOp::SetCells {
            sheet: "Budget".into(),
            cells: BTreeMap::from([
                ("B2".to_string(), CellValue::Number(150.0)),
                ("b3".to_string(), CellValue::Number(70.0)),
            ]),
        },
        OfficeOp::AddSheet { name: "Q4".into() },
        OfficeOp::SetCells {
            sheet: "q4".into(),
            cells: BTreeMap::from([("A1".to_string(), CellValue::Text("Plan".into()))]),
        },
    ];
    let offered = choices(&source, &ops);
    let ids: Vec<&str> = offered.iter().map(|choice| choice.id.as_str()).collect();
    assert_eq!(ids, ["0:B2", "0:B3", "1", "2"]);
    assert_eq!(offered[0].label, "Set Budget!B2");
    assert_eq!(offered[0].changes.len(), 1, "{:?}", offered[0].changes);
    assert_eq!(offered[0].changes[0].anchor, "Budget!B2");
    assert_eq!(offered[3].requires, ["1"]);

    let text = narrow(&source, &ops, &["0:B3"]).unwrap();
    assert!(text.contains("B3: 70"), "{text}");
    assert!(!text.contains("B2: 150"), "{text}");
    assert!(!text.contains("Q4"), "{text}");

    let error = narrow(&source, &ops, &["2"]).unwrap_err();
    assert!(error.contains("builds on edit 2"), "{error}");
    let error = narrow(&source, &ops, &["0:Z9"]).unwrap_err();
    assert!(error.contains("not a change in this draft"), "{error}");
}

#[test]
fn a_kept_edit_to_a_new_slide_still_lands_on_that_slide() {
    let source = fixtures::pptx_template();
    let slide = |title: &str| OfficeOp::AddSlideFromLayout {
        layout: "Title Slide".into(),
        after: None,
        placeholders: BTreeMap::from([("title".to_string(), TextValue::One(title.into()))]),
    };
    let ops = vec![
        slide("First"),
        slide("Second"),
        OfficeOp::SetPlaceholderText {
            anchor: "slide:258/placeholder:title".into(),
            text: TextValue::One("Second, renamed".into()),
        },
    ];
    let offered = choices(&source, &ops);
    assert_eq!(offered[2].requires, ["1"]);

    let text = narrow(&source, &ops, &["1", "2"]).unwrap();
    assert!(!text.contains("First"), "{text}");
    assert!(!text.contains("slide:258"), "{text}");
    assert!(
        text.contains("[slide:257] Slide 2: Second, renamed"),
        "{text}"
    );
}

#[test]
fn a_draft_the_ops_do_not_reproduce_is_taken_whole() {
    let source = fixtures::docx();
    let ops = vec![insert("p@1", "Alpha."), insert("p@3", "Bravo.")];
    let edited_later = draft(&source, &[insert("p@1", "Alpha, edited by hand.")]);
    let error = review::choices(
        &source,
        &ops,
        &context(),
        Limits::default(),
        None,
        &edited_later.document,
    )
    .unwrap_err();
    assert!(
        error.contains("not what its recorded edits produce"),
        "{error}"
    );

    let too_many: Vec<OfficeOp> = (0..=review::MAX_OPS)
        .map(|_| OfficeOp::SetTitle { title: "x".into() })
        .collect();
    let error = review::choices(
        &source,
        &too_many,
        &context(),
        Limits::default(),
        None,
        &edited_later.document,
    )
    .unwrap_err();
    assert!(error.contains("choosing among more than"), "{error}");
}
