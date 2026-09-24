//! The paged projection the views draw (docs/design/72, P4) and the
//! Structure listing (U4).

#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::io::Cursor;

use vak_ooxml::projection::{self, PAGE_BYTES};
use vak_ooxml::{Limits, fixtures, read};

fn read_bytes(bytes: &[u8]) -> read::Document {
    read::read(Cursor::new(bytes.to_vec()), Limits::default()).unwrap()
}

#[test]
fn pages_cover_every_unit_once_in_order() {
    let document = read_bytes(&fixtures::docx());
    let mut from = 0;
    let mut anchors = Vec::new();
    let mut pages = 0;
    loop {
        let page = projection::project(&document, from, 200);
        assert_eq!(page.from, from);
        assert!(!page.units.is_empty(), "a page always holds a unit");
        anchors.extend(page.units.iter().map(|unit| unit.anchor.clone()));
        pages += 1;
        match page.next {
            Some(next) => from = next,
            None => break,
        }
    }
    assert!(pages > 1, "a small budget pages the document");
    let all: Vec<String> = document
        .units
        .iter()
        .map(|unit| unit.anchor.clone())
        .collect();
    assert_eq!(anchors, all);

    let whole = projection::project(&document, 0, PAGE_BYTES);
    assert_eq!(whole.next, None);
    assert_eq!(whole.total_units, document.units.len());
    assert_eq!(whole.kind, "Word document");
    assert_eq!(whole.extension, "docx");
    for entry in &whole.outline {
        let first = &document.units[entry.first_unit];
        assert!(
            entry.units == 0 || first.anchor.starts_with('p') || first.anchor == entry.anchor,
            "{entry:?} points at {first:?}"
        );
    }

    let labelled = projection::project(
        &read_bytes(&fixtures::signed_labelled_docx()),
        0,
        PAGE_BYTES,
    );
    assert!(
        labelled
            .flags
            .iter()
            .any(|flag| flag.contains("digitally signed")),
        "{:?}",
        labelled.flags
    );
    assert_eq!(labelled.sensitivity_labels, ["Confidential"]);
}

#[test]
fn a_sheet_page_carries_its_cells_and_a_slide_deck_its_outline() {
    let workbook = projection::project(&read_bytes(&fixtures::xlsx()), 0, PAGE_BYTES);
    assert_eq!(workbook.kind, "Excel workbook");
    assert!(workbook.outline.iter().any(|entry| entry.title == "Budget"));
    let row = workbook
        .units
        .iter()
        .find(|unit| unit.cells.iter().any(|(address, _)| address == "B2"))
        .expect("row 2");
    assert!(row.anchor.starts_with("Budget!"));

    let deck = projection::project(&read_bytes(&fixtures::pptx()), 0, PAGE_BYTES);
    assert_eq!(deck.kind, "PowerPoint presentation");
    assert!(
        deck.outline
            .iter()
            .all(|entry| entry.anchor.starts_with("slide:"))
    );
    assert!(!deck.outline.is_empty());
}

#[test]
fn a_unit_larger_than_the_page_is_cut_and_says_so() {
    let long = "Long paragraph text. ".repeat(400);
    let body = fixtures::MINIMAL_WORD_BODY.replace("Hello", &long);
    let bytes = fixtures::word_with(fixtures::WORD_MAIN, &body, &[], &[], &[], &[]);
    let page = projection::project(&read_bytes(&bytes), 0, 2048);
    assert_eq!(page.units.len(), 1);
    assert!(page.units[0].text.len() < long.len());
    assert!(
        page.units[0]
            .labels
            .iter()
            .any(|label| label.starts_with("shown up to")),
        "{:?}",
        page.units[0].labels
    );
}

#[test]
fn structure_lists_parts_types_and_relationships() {
    let rels = r#"<?xml version="1.0"?><Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="rId9" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/hyperlink" Target="https://example.com/" TargetMode="External"/></Relationships>"#;
    let bytes = fixtures::with_parts(
        &fixtures::docx(),
        &[("word/_rels/document.xml.rels", rels.as_bytes())],
    );
    let structure = projection::structure(Cursor::new(bytes), Limits::default()).unwrap();
    assert_eq!(structure.main_part, "word/document.xml");
    let main = structure
        .parts
        .iter()
        .find(|part| part.name == "word/document.xml")
        .unwrap();
    assert!(
        main.content_type
            .as_deref()
            .is_some_and(|kind| kind.contains("wordprocessingml.document.main"))
    );
    assert!(main.size > 0);
    assert!(
        structure
            .relationships
            .iter()
            .any(|relationship| relationship.source.is_empty()
                && relationship.kind == "officeDocument"
                && relationship.target == "word/document.xml")
    );
    assert!(
        structure
            .relationships
            .iter()
            .any(|relationship| relationship.external
                && relationship.kind == "hyperlink"
                && relationship.source == "word/document.xml"),
        "{:?}",
        structure.relationships
    );
    assert!(structure.untyped_parts.is_empty());
}
