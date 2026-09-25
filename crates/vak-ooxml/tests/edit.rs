//! The op engine (docs/design/72-openxml-documents.md, P2): each op lands
//! where its anchor points, leaves everything else byte-identical, is
//! confirmed by a re-read, and fails with a repairable message otherwise.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::collections::BTreeMap;
use std::io::{Cursor, Read};

use vak_ooxml::edit::{self, CellValue, EditContext, OfficeOp, TextValue};
use vak_ooxml::fixtures;
use vak_ooxml::read::UnitKind;
use vak_ooxml::{Limits, Package};

fn context() -> EditContext {
    EditContext {
        author: "Mira".into(),
        date: "2026-09-24T10:00:00Z".into(),
    }
}

fn apply(bytes: &[u8], ops: Vec<OfficeOp>) -> Result<edit::Applied, edit::EditError> {
    edit::apply(bytes, &ops, &context(), Limits::default(), None)
}

fn part(bytes: &[u8], name: &str) -> String {
    let mut package = Package::open(Cursor::new(bytes.to_vec()), Limits::default()).unwrap();
    String::from_utf8(package.read_part(name).unwrap()).unwrap()
}

/// Raw compressed bytes of every entry except `except`.
fn untouched(bytes: &[u8], except: &[&str]) -> Vec<(String, Vec<u8>)> {
    let mut archive = zip::ZipArchive::new(Cursor::new(bytes.to_vec())).unwrap();
    (0..archive.len())
        .filter_map(|index| {
            let mut file = archive.by_index_raw(index).unwrap();
            if except.contains(&file.name()) {
                return None;
            }
            let mut raw = Vec::new();
            file.read_to_end(&mut raw).unwrap();
            Some((file.name().to_string(), raw))
        })
        .collect()
}

fn lines(applied: &edit::Applied) -> String {
    applied.document.lines().join("\n")
}

// ---- Word ----------------------------------------------------------------------

#[test]
fn replacing_a_paragraph_is_a_tracked_change_and_touches_nothing_else() {
    let source = fixtures::docx();
    let applied = apply(
        &source,
        vec![OfficeOp::ReplaceParagraphText {
            anchor: "p@11".into(),
            text: "Growing fast.".into(),
        }],
    )
    .unwrap();
    let text = lines(&applied);
    assert!(
        text.contains("[p@11] [deleted by Mira: Steady.][inserted by Mira: Growing fast.]"),
        "{text}"
    );
    assert_eq!(applied.results[0].op, "replace_paragraph_text");
    assert!(applied.results[0].check.starts_with("passed"));
    assert_eq!(
        untouched(&source, &["word/document.xml"]),
        untouched(&applied.bytes, &["word/document.xml"]),
        "every other part is copied raw"
    );
    let before = part(&source, "word/document.xml");
    let after = part(&applied.bytes, "word/document.xml");
    let edited = before
        .find("<w:p><w:r><w:t>Steady.</w:t></w:r></w:p>")
        .unwrap();
    assert_eq!(
        &after[..edited],
        &before[..edited],
        "bytes before the paragraph are identical"
    );
    assert!(after.contains(r#"w:author="Mira" w:date="2026-09-24T10:00:00Z""#));
    assert!(after.contains("<w:delText>Steady.</w:delText>"));
}

#[test]
fn inserting_a_paragraph_uses_a_style_by_name_and_a_stable_new_anchor() {
    let applied = apply(
        &fixtures::docx(),
        vec![
            OfficeOp::InsertParagraphAfter {
                anchor: "p@1".into(),
                text: "Details".into(),
                style: Some("heading 2".into()),
            },
            OfficeOp::ReplaceParagraphText {
                anchor: "p@11".into(),
                text: "Still steady.".into(),
            },
        ],
    )
    .unwrap();
    let inserted = applied
        .document
        .units
        .iter()
        .find(|unit| unit.text.contains("[inserted by Mira: Details]"))
        .unwrap();
    assert!(
        inserted.anchor.starts_with("p:"),
        "new paragraphs get a paraId: {}",
        inserted.anchor
    );
    assert_eq!(inserted.kind, UnitKind::Heading);
    assert_eq!(inserted.level, 2);
    assert!(
        lines(&applied)
            .contains("[p@11] [deleted by Mira: Steady.][inserted by Mira: Still steady.]"),
        "p@N anchors from the base read still resolve after an insert in the same call"
    );
}

#[test]
fn inserting_into_a_document_without_w14_declares_it_on_the_root() {
    let source = fixtures::word_with(
        fixtures::WORD_MAIN,
        fixtures::MINIMAL_WORD_BODY,
        &[],
        &[],
        &[],
        &[],
    );
    let applied = apply(
        &source,
        vec![OfficeOp::InsertParagraphAfter {
            anchor: "p@1".into(),
            text: "Second".into(),
            style: None,
        }],
    )
    .unwrap();
    let document = part(&applied.bytes, "word/document.xml");
    assert!(
        document.contains(r#"xmlns:w14="http://schemas.microsoft.com/office/word/2010/wordml""#)
    );
    assert!(document.contains(r#"mc:Ignorable="w14""#));
    assert!(lines(&applied).contains("[inserted by Mira: Second]"));
}

#[test]
fn deleting_a_paragraph_marks_its_text_and_mark_deleted() {
    let applied = apply(
        &fixtures::docx(),
        vec![OfficeOp::DeleteParagraph {
            anchor: "p@10".into(),
        }],
    )
    .unwrap();
    assert!(
        lines(&applied).contains("[p@10] # [deleted by Mira: Outlook]"),
        "{}",
        lines(&applied)
    );
    let document = part(&applied.bytes, "word/document.xml");
    assert!(
        document.contains(r#"<w:rPr><w:del w:id="#),
        "the paragraph mark is deleted too"
    );
}

#[test]
fn word_refusals_name_the_op_and_the_repair() {
    let cases = [
        (
            OfficeOp::ReplaceParagraphText {
                anchor: "p@2".into(),
                text: "x".into(),
            },
            "tracked-change markup",
        ),
        (
            OfficeOp::ReplaceParagraphText {
                anchor: "p@4".into(),
                text: "x".into(),
            },
            "field markup",
        ),
        (
            OfficeOp::DeleteParagraph {
                anchor: "p@11".into(),
            },
            "last paragraph",
        ),
        (
            OfficeOp::DeleteParagraph {
                anchor: "p@7".into(),
            },
            "only paragraph in its table cell",
        ),
        (
            OfficeOp::ReplaceParagraphText {
                anchor: "p@99".into(),
                text: "x".into(),
            },
            "no paragraph p@99",
        ),
        (
            OfficeOp::InsertParagraphAfter {
                anchor: "p@1".into(),
                text: "x".into(),
                style: Some("Nonexistent".into()),
            },
            "no paragraph style",
        ),
        (
            OfficeOp::SetCells {
                sheet: "Budget".into(),
                cells: BTreeMap::new(),
            },
            "edits an Excel workbook, and this file is a Word document",
        ),
    ];
    for (op, expected) in cases {
        let name = op.name();
        let error = apply(&fixtures::docx(), vec![op]).unwrap_err();
        let message = error.to_string();
        assert!(message.starts_with(&format!("op 1 ({name})")), "{message}");
        assert!(message.contains(expected), "{message}");
    }
}

// ---- Excel ---------------------------------------------------------------------

#[test]
fn setting_cells_keeps_styles_and_marks_formulas_stale() {
    let source = fixtures::xlsx();
    let cells = BTreeMap::from([
        ("B2".to_string(), CellValue::Number(150.0)),
        ("C3".to_string(), CellValue::Text("=B3*2".into())),
        ("D6".to_string(), CellValue::Text("new row".into())),
        ("E1".to_string(), CellValue::Bool(false)),
        ("A5".to_string(), CellValue::Text("'=not a formula".into())),
    ]);
    let applied = apply(
        &source,
        vec![OfficeOp::SetCells {
            sheet: "budget".into(),
            cells,
        }],
    )
    .unwrap();
    let text = lines(&applied);
    assert!(text.contains("A2: Rent | B2: 150"), "{text}");
    assert!(text.contains("C3: =B3*2 [not calculated yet]"), "{text}");
    assert!(
        text.contains("B4: =SUM(B2:B3) [cached: 120, stale until recalculated]"),
        "a changed input makes cached formula values stale: {text}"
    );
    assert!(text.contains("[Budget!D6:D6] D6: new row"), "{text}");
    assert!(text.contains("E1: FALSE"), "{text}");
    assert!(text.contains("A5: =not a formula"), "{text}");
    assert!(part(&applied.bytes, "xl/workbook.xml").contains(r#"<calcPr fullCalcOnLoad="1"/>"#));
    assert_eq!(
        untouched(&source, &["xl/workbook.xml", "xl/worksheets/sheet1.xml"]),
        untouched(
            &applied.bytes,
            &["xl/workbook.xml", "xl/worksheets/sheet1.xml"]
        ),
    );
}

#[test]
fn a_formula_change_removes_the_calculation_chain() {
    let source = fixtures::xlsx();
    let types = part(&source, "[Content_Types].xml").replace(
        "</Types>",
        r#"<Override PartName="/xl/calcChain.xml" ContentType="application/vnd.openxmlformats-officedocument.spreadsheetml.calcChain+xml"/></Types>"#,
    );
    let rels = part(&source, "xl/_rels/workbook.xml.rels").replace(
        "</Relationships>",
        r#"<Relationship Id="rId9" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/calcChain" Target="calcChain.xml"/></Relationships>"#,
    );
    let source = fixtures::with_parts(
        &source,
        &[
            ("[Content_Types].xml", types.as_bytes()),
            ("xl/_rels/workbook.xml.rels", rels.as_bytes()),
            (
                "xl/calcChain.xml",
                br#"<calcChain xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main"><c r="B4" i="1"/></calcChain>"#,
            ),
        ],
    );
    let applied = apply(
        &source,
        vec![OfficeOp::SetCells {
            sheet: "Budget".into(),
            cells: BTreeMap::from([("B4".to_string(), CellValue::Number(7.0))]),
        }],
    )
    .unwrap();
    let names: Vec<String> = untouched(&applied.bytes, &[])
        .into_iter()
        .map(|entry| entry.0)
        .collect();
    assert!(!names.contains(&"xl/calcChain.xml".to_string()));
    assert!(!part(&applied.bytes, "xl/_rels/workbook.xml.rels").contains("calcChain"));
    assert!(!part(&applied.bytes, "[Content_Types].xml").contains("calcChain"));
}

#[test]
fn appending_rows_and_adding_a_sheet() {
    let applied = apply(
        &fixtures::xlsx(),
        vec![
            OfficeOp::AppendRows {
                sheet: "Budget".into(),
                rows: vec![
                    vec![CellValue::Text("Travel".into()), CellValue::Number(30.0)],
                    vec![CellValue::Text("Food".into()), CellValue::Number(12.5)],
                ],
            },
            OfficeOp::AddSheet {
                name: "Q4 plan".into(),
            },
            OfficeOp::SetCells {
                sheet: "Q4 plan".into(),
                cells: BTreeMap::from([("A1".to_string(), CellValue::Text("Target".into()))]),
            },
        ],
    )
    .unwrap();
    let text = lines(&applied);
    assert!(
        text.contains("[Budget!A5:B5] A5: Travel | B5: 30"),
        "{text}"
    );
    assert!(
        text.contains("[Budget!A6:B6] A6: Food | B6: 12.5"),
        "{text}"
    );
    assert!(text.contains("['Q4 plan'!A1:A1] A1: Target"), "{text}");
    let error = apply(
        &fixtures::xlsx(),
        vec![OfficeOp::AddSheet {
            name: "budget".into(),
        }],
    )
    .unwrap_err();
    assert!(error.to_string().contains("already exists"), "{error}");
}

#[test]
fn excel_refusals() {
    let protected = {
        let source = fixtures::xlsx();
        let sheet = part(&source, "xl/worksheets/sheet1.xml").replace(
            "</sheetData>",
            r#"</sheetData><sheetProtection sheet="1"/>"#,
        );
        fixtures::with_parts(&source, &[("xl/worksheets/sheet1.xml", sheet.as_bytes())])
    };
    let one = |sheet: &str, cell: &str| OfficeOp::SetCells {
        sheet: sheet.into(),
        cells: BTreeMap::from([(cell.to_string(), CellValue::Number(1.0))]),
    };
    let error = apply(&protected, vec![one("Budget", "A1")]).unwrap_err();
    assert!(error.to_string().contains("is protected"), "{error}");
    let error = apply(&fixtures::xlsx(), vec![one("Nope", "A1")]).unwrap_err();
    assert!(
        error.to_string().contains("sheets: Budget, Hidden data"),
        "{error}"
    );
    let error = apply(&fixtures::xlsx(), vec![one("Budget", "ZZZZ1")]).unwrap_err();
    assert!(error.to_string().contains("not a cell address"), "{error}");

    let shared = {
        let source = fixtures::xlsx();
        let sheet = part(&source, "xl/worksheets/sheet1.xml").replace(
            "<f>SUM(B2:B3)</f>",
            r#"<f t="shared" ref="B4:C4" si="0">SUM(B2:B3)</f>"#,
        );
        fixtures::with_parts(&source, &[("xl/worksheets/sheet1.xml", sheet.as_bytes())])
    };
    let error = apply(&shared, vec![one("Budget", "B4")]).unwrap_err();
    assert!(error.to_string().contains("shared formula"), "{error}");
}

// ---- PowerPoint ----------------------------------------------------------------

#[test]
fn a_slide_from_a_template_layout_fills_its_placeholders() {
    let source = fixtures::pptx_template();
    let mut placeholders = BTreeMap::new();
    placeholders.insert("title".to_string(), TextValue::One("Agenda".into()));
    placeholders.insert(
        "body".to_string(),
        TextValue::Lines(vec!["Budget".into(), "Hiring".into()]),
    );
    let applied = apply(
        &source,
        vec![OfficeOp::AddSlideFromLayout {
            layout: "title and content".into(),
            after: Some("slide:256".into()),
            placeholders,
        }],
    )
    .unwrap();
    let text = lines(&applied);
    assert!(text.contains("[slide:257] Slide 2: Agenda"), "{text}");
    assert!(text.contains("Budget ¶ Hiring"), "{text}");
    assert!(part(&applied.bytes, "[Content_Types].xml").contains("/ppt/slides/slide2.xml"));
    assert!(
        part(&applied.bytes, "ppt/slides/_rels/slide2.xml.rels")
            .contains(r#"Target="../slideLayouts/slideLayout2.xml""#)
    );
    assert_eq!(
        untouched(
            &source,
            &[
                "ppt/presentation.xml",
                "ppt/_rels/presentation.xml.rels",
                "[Content_Types].xml"
            ]
        ),
        untouched(
            &applied.bytes,
            &[
                "ppt/presentation.xml",
                "ppt/_rels/presentation.xml.rels",
                "[Content_Types].xml",
                "ppt/slides/slide2.xml",
                "ppt/slides/_rels/slide2.xml.rels"
            ]
        ),
    );

    let error = apply(
        &source,
        vec![OfficeOp::AddSlideFromLayout {
            layout: "Blank".into(),
            after: None,
            placeholders: BTreeMap::new(),
        }],
    )
    .unwrap_err();
    assert!(
        error.to_string().contains("Title Slide, Title and Content"),
        "{error}"
    );
    let error = apply(
        &source,
        vec![OfficeOp::AddSlideFromLayout {
            layout: "Title Slide".into(),
            after: None,
            placeholders: BTreeMap::from([("body".to_string(), TextValue::One("x".into()))]),
        }],
    )
    .unwrap_err();
    assert!(
        error.to_string().contains("it has: title, subtitle"),
        "{error}"
    );
}

#[test]
fn placeholder_text_notes_move_and_delete() {
    let applied = apply(
        &fixtures::pptx(),
        vec![
            OfficeOp::SetPlaceholderText {
                anchor: "slide:256/placeholder:title".into(),
                text: TextValue::One("Launch plan v2".into()),
            },
            OfficeOp::SetPlaceholderText {
                anchor: "slide:256/shape:3".into(),
                text: TextValue::Lines(vec!["Ship in June".into(), "Beta in May".into()]),
            },
            OfficeOp::SetNotes {
                anchor: "slide:256".into(),
                text: "Mention the new date.".into(),
            },
            OfficeOp::MoveSlide {
                anchor: "slide:257".into(),
                after: None,
            },
        ],
    )
    .unwrap();
    let text = lines(&applied);
    assert!(text.contains("Slide 2: Launch plan v2"), "{text}");
    assert!(
        text.contains("[slide:256/shape:3] Ship in June ¶ Beta in May"),
        "{text}"
    );
    assert!(
        text.contains("[slide:256/notes] Mention the new date."),
        "{text}"
    );
    assert!(text.contains("[slide:257] Slide 1: Backup"), "{text}");

    let deleted = apply(
        &fixtures::pptx(),
        vec![OfficeOp::DeleteSlide {
            anchor: "slide:256".into(),
        }],
    )
    .unwrap();
    let text = lines(&deleted);
    assert!(!text.contains("slide:256"), "{text}");
    let names: Vec<String> = untouched(&deleted.bytes, &[])
        .into_iter()
        .map(|entry| entry.0)
        .collect();
    assert!(
        !names
            .iter()
            .any(|name| name.contains("slide1.xml") || name.contains("notesSlide1")),
        "{names:?}"
    );
    let error = apply(
        &deleted.bytes,
        vec![OfficeOp::DeleteSlide {
            anchor: "slide:257".into(),
        }],
    )
    .unwrap_err();
    assert!(error.to_string().contains("only slide"), "{error}");
}

#[test]
fn set_title_and_the_op_schema() {
    let applied = apply(
        &fixtures::docx(),
        vec![OfficeOp::SetTitle {
            title: "Q3 Report (final)".into(),
        }],
    )
    .unwrap();
    assert_eq!(applied.document.title.as_deref(), Some("Q3 Report (final)"));

    let parsed: Vec<OfficeOp> = serde_json::from_str(
        r#"[{"op":"set_cells","sheet":"Budget","cells":{"B2":150,"C3":"=B3*2","D1":true}},
            {"op":"set_placeholder_text","anchor":"slide:256/placeholder:title","text":["a","b"]}]"#,
    )
    .unwrap();
    assert_eq!(parsed[0].name(), "set_cells");
    assert!(
        serde_json::from_str::<OfficeOp>(r#"{"op":"set_title","title":"x","extra":1}"#).is_err(),
        "unknown fields are refused, so a misspelled field is never silently ignored"
    );
}

#[test]
fn a_template_becomes_a_document_but_never_changes_macro_state() {
    let template = fixtures::with_parts(
        &fixtures::pptx_template(),
        &[(
            "[Content_Types].xml",
            part(&fixtures::pptx_template(), "[Content_Types].xml")
                .replace(
                    "presentationml.presentation.main+xml",
                    "presentationml.template.main+xml",
                )
                .as_bytes(),
        )],
    );
    assert_eq!(
        Package::open(Cursor::new(template.clone()), Limits::default())
            .unwrap()
            .format()
            .extension(),
        "potx"
    );
    let pptx = vak_ooxml::Format::from_extension("pptx");
    let applied = edit::apply(&template, &[], &context(), Limits::default(), pptx).unwrap();
    assert_eq!(
        Package::open(Cursor::new(applied.bytes), Limits::default())
            .unwrap()
            .format()
            .extension(),
        "pptx"
    );
    let error = edit::apply(
        &fixtures::pptx(),
        &[],
        &context(),
        Limits::default(),
        vak_ooxml::Format::from_extension("pptm"),
    )
    .unwrap_err();
    assert!(
        error
            .to_string()
            .contains("never turns a file macro-enabled"),
        "{error}"
    );
    let error = edit::apply(
        &fixtures::pptx(),
        &[],
        &context(),
        Limits::default(),
        vak_ooxml::Format::from_extension("docx"),
    )
    .unwrap_err();
    assert!(
        error.to_string().contains("cannot be saved as .docx"),
        "{error}"
    );
}

// ---- protection and preservation ------------------------------------------------

fn with_word_settings(protection: &str) -> Vec<u8> {
    fixtures::word_with(
        fixtures::WORD_MAIN,
        fixtures::MINIMAL_WORD_BODY,
        &[(
            "word/settings.xml",
            format!(
                r#"<w:settings xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main">{protection}</w:settings>"#
            )
            .as_bytes(),
        )],
        &[(
            "word/settings.xml",
            "application/vnd.openxmlformats-officedocument.wordprocessingml.settings+xml",
        )],
        &[],
        &[(
            "rId1",
            "http://schemas.openxmlformats.org/officeDocument/2006/relationships/settings",
            "settings.xml",
        )],
    )
}

#[test]
fn protection_is_honoured_before_any_byte_is_written() {
    let replace = || OfficeOp::ReplaceParagraphText {
        anchor: "p@1".into(),
        text: "x".into(),
    };
    for (protection, allowed) in [
        (
            r#"<w:documentProtection w:edit="readOnly" w:enforcement="1"/>"#,
            false,
        ),
        (
            r#"<w:documentProtection w:edit="comments" w:enforcement="true"/>"#,
            false,
        ),
        (
            r#"<w:documentProtection w:edit="trackedChanges" w:enforcement="1"/>"#,
            true,
        ),
        (
            r#"<w:documentProtection w:edit="readOnly" w:enforcement="0"/>"#,
            true,
        ),
    ] {
        let result = apply(&with_word_settings(protection), vec![replace()]);
        assert_eq!(result.is_ok(), allowed, "{protection}: {:?}", result.err());
        if let Err(error) = result {
            assert!(error.to_string().contains("is protected"), "{error}");
        }
    }
    let source = fixtures::pptx();
    let presentation = part(&source, "ppt/presentation.xml").replace(
        "<p:sldSz",
        r#"<p:modifyVerifier cryptProviderType="rsaAES" cryptAlgorithmClass="hash" cryptAlgorithmType="typeAny" cryptAlgorithmSid="14" spinCount="100000" saltData="AA==" hashData="AA=="/><p:sldSz"#,
    );
    let locked = fixtures::with_parts(
        &source,
        &[("ppt/presentation.xml", presentation.as_bytes())],
    );
    let error = apply(
        &locked,
        vec![OfficeOp::DeleteSlide {
            anchor: "slide:257".into(),
        }],
    )
    .unwrap_err();
    assert!(error.to_string().contains("password to modify"), "{error}");
}

#[test]
fn markup_vak_does_not_model_survives_inside_edited_elements() {
    let unknown = r#"<x:keep xmlns:x="urn:vak-test" x:note="1"/>"#;
    let document = format!(
        r#"<w:document xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main"><w:body><w:p><w:pPr>{unknown}</w:pPr><w:r><w:rPr>{unknown}</w:rPr><w:t>Old</w:t></w:r></w:p><w:p><w:r><w:t>Last</w:t></w:r></w:p></w:body></w:document>"#
    );
    let word = fixtures::word_with(fixtures::WORD_MAIN, &document, &[], &[], &[], &[]);
    for op in [
        OfficeOp::ReplaceParagraphText {
            anchor: "p@1".into(),
            text: "New".into(),
        },
        OfficeOp::DeleteParagraph {
            anchor: "p@1".into(),
        },
        OfficeOp::InsertParagraphAfter {
            anchor: "p@1".into(),
            text: "After".into(),
            style: None,
        },
    ] {
        let name = op.name();
        let applied = apply(&word, vec![op]).unwrap();
        let written = part(&applied.bytes, "word/document.xml");
        assert!(
            written.matches(unknown).count() >= 2,
            "{name} keeps both originals (a replacement also copies the run's formatting): {written}"
        );
    }

    let source = fixtures::xlsx();
    let sheet = part(&source, "xl/worksheets/sheet1.xml").replace(
        r#"<row r="2">"#,
        r#"<row r="2" spans="1:2" x14ac:dyDescent="0.25" xmlns:x14ac="urn:vak-test"><x:ext xmlns:x="urn:vak-test"/>"#,
    );
    let workbook = fixtures::with_parts(&source, &[("xl/worksheets/sheet1.xml", sheet.as_bytes())]);
    let applied = apply(
        &workbook,
        vec![OfficeOp::SetCells {
            sheet: "Budget".into(),
            cells: BTreeMap::from([("F2".to_string(), CellValue::Number(9.0))]),
        }],
    )
    .unwrap();
    let written = part(&applied.bytes, "xl/worksheets/sheet1.xml");
    assert!(written.contains(r#"x14ac:dyDescent="0.25""#), "{written}");
    assert!(
        written.contains(r#"<x:ext xmlns:x="urn:vak-test"/>"#),
        "{written}"
    );
    assert!(
        !written.contains(r#"spans="1:2""#),
        "a stale spans hint is dropped: {written}"
    );

    let deck_source = fixtures::pptx();
    let slide = part(&deck_source, "ppt/slides/slide1.xml")
        .replace("<a:bodyPr/><a:lstStyle/>", "")
        .replacen(
            "<a:bodyPr/>",
            r#"<a:bodyPr wrap="square"><x:keep xmlns:x="urn:vak-test"/></a:bodyPr>"#,
            1,
        );
    let deck = fixtures::with_parts(&deck_source, &[("ppt/slides/slide1.xml", slide.as_bytes())]);
    let applied = apply(
        &deck,
        vec![OfficeOp::SetPlaceholderText {
            anchor: "slide:256/placeholder:title".into(),
            text: TextValue::One("Kept".into()),
        }],
    )
    .unwrap();
    let written = part(&applied.bytes, "ppt/slides/slide1.xml");
    assert!(
        written.contains(r#"<a:bodyPr wrap="square"><x:keep xmlns:x="urn:vak-test"/></a:bodyPr>"#),
        "{written}"
    );
}

#[test]
fn several_slide_list_ops_in_one_call_are_checked_against_the_order_they_leave() {
    let source = fixtures::pptx_template();
    let slide = |title: &str| OfficeOp::AddSlideFromLayout {
        layout: "Title Slide".into(),
        after: None,
        placeholders: BTreeMap::from([("title".to_string(), TextValue::One(title.into()))]),
    };
    let applied = apply(
        &source,
        vec![
            slide("First"),
            slide("Second"),
            OfficeOp::MoveSlide {
                anchor: "slide:258".into(),
                after: None,
            },
            OfficeOp::DeleteSlide {
                anchor: "slide:256".into(),
            },
        ],
    )
    .unwrap();
    let order: Vec<&str> = applied
        .document
        .sections
        .iter()
        .map(|section| section.anchor.as_str())
        .collect();
    assert_eq!(order, ["slide:258", "slide:257"]);
    assert!(
        applied
            .results
            .iter()
            .all(|r| r.check.starts_with("passed"))
    );
    assert_eq!(applied.results[0].created.as_deref(), Some("slide:257"));
}

#[test]
fn an_edit_removes_the_signatures_it_invalidates_and_keeps_the_label() {
    let source = fixtures::signed_labelled_docx();
    let before = Package::open(Cursor::new(source.clone()), Limits::default())
        .unwrap()
        .inspect()
        .unwrap();
    assert!(before.signed);
    let applied = apply(
        &source,
        vec![OfficeOp::ReplaceParagraphText {
            anchor: "p@1".into(),
            text: "Hello again".into(),
        }],
    )
    .unwrap();
    assert_eq!(applied.notices.len(), 1, "{:?}", applied.notices);
    assert!(applied.notices[0].contains("1 signature(s) were removed"));
    let after = &applied.document.inspection;
    assert!(!after.signed, "the output claims no signature");
    assert_eq!(
        after.sensitivity_labels,
        ["Confidential"],
        "the label stays"
    );
    let archive = zip::ZipArchive::new(Cursor::new(applied.bytes.clone())).unwrap();
    assert!(
        !archive
            .file_names()
            .any(|name| name.starts_with("_xmlsignatures/")),
        "{:?}",
        archive.file_names().collect::<Vec<_>>()
    );
    let types = part(&applied.bytes, "[Content_Types].xml");
    assert!(!types.contains("xmlsignatures"), "{types}");
    assert!(!part(&applied.bytes, "_rels/.rels").contains("digital-signature"));
    assert_eq!(
        part(&applied.bytes, "docProps/custom.xml"),
        fixtures::CONFIDENTIAL_CUSTOM_PROPERTIES,
        "the label's part is copied as it was"
    );

    let unsigned = apply(
        &fixtures::docx(),
        vec![OfficeOp::ReplaceParagraphText {
            anchor: "p@11".into(),
            text: "Growing.".into(),
        }],
    )
    .unwrap();
    assert!(unsigned.notices.is_empty());
}

#[test]
fn a_paragraph_named_by_its_text_gets_its_anchor_in_the_error() {
    let error = apply(
        &fixtures::docx(),
        vec![OfficeOp::ReplaceParagraphText {
            anchor: "p:Steady.".into(),
            text: "Growing fast.".into(),
        }],
    )
    .unwrap_err()
    .to_string();
    assert!(
        error.contains("The paragraph whose text is \"Steady.\" is p@11"),
        "{error}"
    );
    let error = apply(
        &fixtures::docx(),
        vec![OfficeOp::DeleteParagraph {
            anchor: "p:nothing like this".into(),
        }],
    )
    .unwrap_err()
    .to_string();
    assert!(
        error.ends_with("(anchors look like p:1A2B3C4D or p@12)"),
        "{error}"
    );
}
