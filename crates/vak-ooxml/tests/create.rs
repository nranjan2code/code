//! Creating a file from scratch (docs/design/72-openxml-documents.md,
//! "Creating from scratch"): each built-in blank is a valid, empty,
//! deterministic package, and the anchor-free ops fill a new file in one
//! call, checked by a re-read like every edit.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::collections::BTreeMap;
use std::io::Cursor;

use vak_ooxml::blank::{DECK_LAYOUTS, DOCUMENT_STYLES, WORKBOOK_SHEET, blank};
use vak_ooxml::edit::{self, CellValue, EditContext, OfficeOp, SlideChart, SlideImage, TextValue};
use vak_ooxml::read::{self, UnitKind};
use vak_ooxml::{Format, FormatKind, Limits, Package, Vocabulary, fixtures, review};

fn format(extension: &str) -> Format {
    Format::from_extension(extension).unwrap()
}

fn clean() -> EditContext {
    EditContext {
        author: "Vakyartha".into(),
        date: "2026-09-27T10:00:00Z".into(),
        tracked: false,
    }
}

fn create(extension: &str, ops: Vec<OfficeOp>) -> Result<edit::Applied, edit::EditError> {
    let target = format(extension);
    edit::apply(
        &blank(target).unwrap(),
        &ops,
        &clean(),
        Limits::default(),
        Some(target),
    )
}

fn part(bytes: &[u8], name: &str) -> String {
    let mut package = Package::open(Cursor::new(bytes.to_vec()), Limits::default()).unwrap();
    String::from_utf8(package.read_part(name).unwrap()).unwrap()
}

fn paragraph(text: &str, style: Option<&str>) -> OfficeOp {
    OfficeOp::AddParagraph {
        text: text.into(),
        style: style.map(str::to_string),
        after: None,
    }
}

fn text(value: &str) -> CellValue {
    CellValue::Text(value.into())
}

#[test]
fn each_blank_is_an_empty_deterministic_package_of_its_format() {
    for extension in ["docx", "xlsx", "pptx"] {
        let bytes = blank(format(extension)).unwrap();
        assert_eq!(bytes, blank(format(extension)).unwrap(), "{extension}");
        let document = read::read(Cursor::new(bytes), Limits::default()).unwrap();
        assert_eq!(document.inspection.format, format(extension), "{extension}");
        assert!(document.inspection.flags().is_empty(), "{extension}");
        match extension {
            "xlsx" => {
                assert!(document.units.is_empty(), "{:?}", document.units);
                assert_eq!(document.sections.len(), 1);
                assert_eq!(document.sections[0].anchor, format!("{WORKBOOK_SHEET}!"));
            }
            _ => assert!(
                document.units.is_empty(),
                "{extension}: {:?}",
                document.units
            ),
        }
    }
}

#[test]
fn a_blank_is_refused_for_macro_enabled_names_and_visio() {
    for extension in ["docm", "xlsm", "pptm", "xlam", "ppam"] {
        let error = blank(format(extension)).unwrap_err();
        assert!(
            error.contains("never makes a macro-enabled file"),
            "{error}"
        );
    }
    let error = blank(format("vsdx")).unwrap_err();
    assert!(error.contains("Visio"), "{error}");
}

#[test]
fn a_word_document_is_written_from_scratch_in_one_call() {
    let applied = create(
        "docx",
        vec![
            OfficeOp::SetTitle {
                title: "Q3 review".into(),
            },
            paragraph("Q3 review", Some("Title")),
            paragraph("Prepared for the leadership team", Some("Subtitle")),
            paragraph("Summary", Some("Heading 1")),
            paragraph("Revenue grew 12% on the quarter.", None),
            paragraph("Close the Pune office", Some("List Number")),
            paragraph("Hire two engineers", Some("List Number")),
            paragraph("Next steps", Some("Heading 2")),
            paragraph("Draft the budget", Some("List Number")),
            paragraph("Review it in October", Some("List Number")),
            paragraph("Travel is paused", Some("List Bullet")),
            OfficeOp::AddTable {
                rows: vec![
                    vec![text("Region"), text("Revenue")],
                    vec![text("North"), CellValue::Number(120.0)],
                    vec![text("South"), CellValue::Number(95.5)],
                ],
                after: None,
                header: None,
            },
            paragraph("Figures are unaudited.", Some("Quote")),
        ],
    )
    .unwrap();
    let document = &applied.document;
    assert_eq!(document.title.as_deref(), Some("Q3 review"));
    let texts: Vec<&str> = document
        .units
        .iter()
        .map(|unit| unit.text.as_str())
        .collect();
    assert_eq!(
        texts,
        [
            "Q3 review",
            "Prepared for the leadership team",
            "Summary",
            "Revenue grew 12% on the quarter.",
            "Close the Pune office",
            "Hire two engineers",
            "Next steps",
            "Draft the budget",
            "Review it in October",
            "Travel is paused",
            "Region | Revenue",
            "North | 120",
            "South | 95.5",
            "Figures are unaudited.",
        ]
    );
    let headings: Vec<(&str, u8)> = document
        .sections
        .iter()
        .map(|section| (section.title.as_str(), section.level))
        .collect();
    assert_eq!(
        headings,
        [("Q3 review", 1), ("Summary", 1), ("Next steps", 2)]
    );
    assert_eq!(document.units[10].kind, UnitKind::TableRow);
    assert_eq!(document.units[10].anchor, "tbl@1/r1");
    let stat = |name: &str| {
        document
            .stats
            .iter()
            .find(|(stat, _)| stat == name)
            .map(|(_, count)| *count)
    };
    assert_eq!(stat("tracked changes"), Some(0), "a new document is clean");
    assert_eq!(stat("tables"), Some(1));

    // Each numbered list starts at 1: each is its own copy of the list
    // definition, without the original's style link, and the second item
    // of each list continues its own list.
    let body = part(&applied.bytes, "word/document.xml");
    let numbering = part(&applied.bytes, "word/numbering.xml");
    assert!(
        numbering.contains(r#"<w:num w:numId="3"><w:abstractNumId w:val="2"/></w:num>"#),
        "{numbering}"
    );
    assert!(
        numbering.contains(r#"<w:num w:numId="4"><w:abstractNumId w:val="3"/></w:num>"#),
        "{numbering}"
    );
    assert_eq!(
        numbering
            .matches(r#"<w:pStyle w:val="ListNumber"/>"#)
            .count(),
        1,
        "{numbering}"
    );
    assert!(
        numbering
            .find(r#"<w:abstractNum w:abstractNumId="3">"#)
            .unwrap()
            < numbering.find(r#"<w:num w:numId="1">"#).unwrap(),
        "definitions come before instances: {numbering}"
    );
    assert_eq!(body.matches(r#"<w:numId w:val="3"/>"#).count(), 2, "{body}");
    assert_eq!(body.matches(r#"<w:numId w:val="4"/>"#).count(), 2, "{body}");
    assert!(
        !body.contains(r#"<w:pStyle w:val="ListBullet"/><w:numPr>"#),
        "bullets need no list instance"
    );
    // The header row repeats on each page and is bold; cells span the text
    // width of an A4 page with 2.54 cm margins.
    assert!(body.contains("<w:tblHeader/>"), "{body}");
    assert!(
        body.contains(r#"<w:gridCol w:w="4513"/><w:gridCol w:w="4513"/>"#),
        "{body}"
    );
    assert!(
        !body.contains("<w:ins "),
        "nothing is tracked in a new document"
    );
    // Every op minted anchors a later op could name.
    assert!(
        applied
            .results
            .iter()
            .all(|result| result.check.starts_with("passed"))
    );
    assert_eq!(
        applied.results[11].created.len(),
        6,
        "{:?}",
        applied.results[11]
    );
}

#[test]
fn every_style_a_blank_document_offers_can_be_used_by_name() {
    let ops = DOCUMENT_STYLES
        .iter()
        .map(|style| paragraph(&format!("In {style}"), Some(style)))
        .collect();
    let applied = create("docx", ops).unwrap();
    assert_eq!(applied.document.units.len(), DOCUMENT_STYLES.len());
}

#[test]
fn a_table_added_to_an_existing_document_is_a_tracked_insertion() {
    let context = EditContext {
        tracked: true,
        ..clean()
    };
    let applied = edit::apply(
        &fixtures::docx(),
        &[OfficeOp::AddTable {
            rows: vec![
                vec![text("Owner"), text("Due")],
                vec![text("Mira"), text("Friday")],
            ],
            after: Some("p@1".into()),
            header: None,
        }],
        &context,
        Limits::default(),
        None,
    )
    .unwrap();
    let body = part(&applied.bytes, "word/document.xml");
    assert!(
        body.contains(r#"<w:trPr><w:tblHeader/><w:ins w:id="#),
        "{body}"
    );
    let row = applied
        .document
        .units
        .iter()
        .find(|unit| unit.text.contains("Friday"))
        .unwrap();
    assert!(
        row.text.contains("[inserted by Vakyartha: Friday]"),
        "{}",
        row.text
    );
    assert_eq!(
        row.anchor, "tbl@1/r2",
        "the new table comes before the fixture's own"
    );

    // A table is never nested in another table's cell.
    let fixture = read::read(Cursor::new(fixtures::docx()), Limits::default()).unwrap();
    let cell = fixture
        .units
        .iter()
        .find(|unit| !unit.row_cells.is_empty())
        .map(|unit| unit.row_cells[0][0].0.clone())
        .unwrap();
    let inside = edit::apply(
        &fixtures::docx(),
        &[OfficeOp::AddTable {
            rows: vec![vec![text("x")]],
            after: Some(cell),
            header: None,
        }],
        &context,
        Limits::default(),
        None,
    )
    .unwrap_err();
    assert!(inside.message.contains("is inside a table"), "{inside}");
}

#[test]
fn a_workbook_is_built_from_scratch_with_names_formats_and_widths() {
    let cells: BTreeMap<String, CellValue> = [
        ("A1", text("Item")),
        ("B1", text("Q3")),
        ("C1", text("Q4")),
        ("A2", text("Rent")),
        ("B2", CellValue::Number(1200.0)),
        ("C2", CellValue::Number(1250.5)),
        ("A3", text("Total")),
        ("B3", text("=SUM(B2:B2)")),
        ("C3", text("=SUM(C2:C2)")),
    ]
    .into_iter()
    .map(|(address, value)| (address.to_string(), value))
    .collect();
    let applied = create(
        "xlsx",
        vec![
            OfficeOp::RenameSheet {
                sheet: WORKBOOK_SHEET.into(),
                name: "Budget".into(),
            },
            OfficeOp::SetCells {
                sheet: "Budget".into(),
                cells,
            },
            OfficeOp::FormatCells {
                sheet: "Budget".into(),
                range: "A1:C1".into(),
                bold: Some(true),
                italic: None,
                number_format: None,
                fill: Some("#D9E2F3".into()),
                wrap: None,
            },
            OfficeOp::FormatCells {
                sheet: "Budget".into(),
                range: "B2:C3".into(),
                bold: None,
                italic: None,
                number_format: Some("#,##0.00".into()),
                fill: None,
                wrap: None,
            },
            OfficeOp::FormatCells {
                sheet: "Budget".into(),
                range: "A3:C3".into(),
                bold: Some(true),
                italic: None,
                number_format: None,
                fill: None,
                wrap: None,
            },
            OfficeOp::FormatCells {
                sheet: "Budget".into(),
                range: "D2".into(),
                bold: None,
                italic: None,
                number_format: Some("0.0%".into()),
                fill: None,
                wrap: None,
            },
            OfficeOp::SetColumnWidths {
                sheet: "Budget".into(),
                widths: [
                    ("A".to_string(), 28.0),
                    ("B".to_string(), 14.0),
                    ("C".to_string(), 14.0),
                ]
                .into_iter()
                .collect(),
            },
            OfficeOp::AddSheet {
                name: "Notes".into(),
            },
        ],
    )
    .unwrap();
    let sections: Vec<&str> = applied
        .document
        .sections
        .iter()
        .map(|section| section.anchor.as_str())
        .collect();
    assert_eq!(sections, ["Budget!", "Notes!"]);
    let lines = applied.document.lines().join("\n");
    assert!(lines.contains("B2: 1200"), "{lines}");
    assert!(
        lines.contains("B3: =SUM(B2:B2) [not calculated yet]"),
        "{lines}"
    );
    let styles = part(&applied.bytes, "xl/styles.xml");
    // #,##0.00 is built in (id 4); 0.0% is declared once, as the first
    // custom format.
    assert!(styles.contains(r#"numFmtId="4""#), "{styles}");
    assert!(
        styles.contains(
            r#"<numFmts count="1"><numFmt numFmtId="164" formatCode="0.0%"/></numFmts><fonts"#
        ),
        "{styles}"
    );
    assert!(styles.contains(r#"<fgColor rgb="FFD9E2F3"/>"#), "{styles}");
    // B3 is both a total (bold) and a number (#,##0.00): the second format
    // kept the first.
    let sheet = part(&applied.bytes, "xl/worksheets/sheet1.xml");
    assert!(
        sheet.contains(r#"<cols><col min="1" max="1" width="28" customWidth="1"/>"#),
        "{sheet}"
    );
    let style_of = |cell: &str| {
        let start = sheet.find(&format!(r#"r="{cell}""#)).unwrap();
        let tag = &sheet[start..start + sheet[start..].find('>').unwrap()];
        tag.split("s=\"")
            .nth(1)
            .unwrap()
            .split('"')
            .next()
            .unwrap()
            .to_string()
    };
    let b3 = style_of("B3");
    assert_ne!(b3, style_of("B2"), "B3 is bold as well");
    assert_ne!(b3, style_of("A3"), "B3 has the number format as well");
    assert!(
        applied
            .results
            .iter()
            .all(|result| result.check.starts_with("passed"))
    );
}

#[test]
fn a_workbook_chart_keeps_its_source_data_and_cached_labels() {
    let cells = BTreeMap::from([
        ("A1".into(), text("Month")),
        ("B1".into(), text("Sales")),
        ("A2".into(), text("Jan")),
        ("B2".into(), CellValue::Number(120.0)),
        ("A3".into(), text("Feb")),
        ("B3".into(), CellValue::Number(145.0)),
        ("A4".into(), text("Mar")),
        ("B4".into(), CellValue::Number(132.0)),
    ]);
    let applied = create(
        "xlsx",
        vec![
            OfficeOp::SetCells {
                sheet: "Sheet1".into(),
                cells,
            },
            OfficeOp::AddChart {
                sheet: "Sheet1".into(),
                range: "A1:B4".into(),
                chart_type: "bar".into(),
                title: "Monthly sales".into(),
                cell: Some("F2".into()),
            },
        ],
    )
    .unwrap();
    let chart = applied
        .document
        .tables
        .iter()
        .find(|table| table.anchor == "chart:chart1.xml")
        .unwrap();
    assert_eq!(chart.title, "Monthly sales");
    assert!(
        chart
            .labels
            .iter()
            .any(|label| label == "chart position: Sheet1!F2"),
        "labels: {:?}; drawing: {}",
        chart.labels,
        part(&applied.bytes, "xl/drawings/drawing1.xml")
    );
    assert_eq!(
        chart.rows,
        vec![
            vec!["Category", "Sales"],
            vec!["Jan", "120"],
            vec!["Feb", "145"],
            vec!["Mar", "132"],
        ]
    );
    let extracted = vak_ooxml::read::read(
        std::io::Cursor::new(applied.bytes.clone()),
        vak_ooxml::Limits::default(),
    )
    .unwrap();
    let chart_rows: Vec<_> = extracted
        .units
        .iter()
        .filter(|unit| unit.anchor.starts_with("chart:chart1.xml/r"))
        .map(|unit| unit.text.as_str())
        .collect();
    assert_eq!(
        chart_rows,
        ["Category | Sales", "Jan | 120", "Feb | 145", "Mar | 132"]
    );
    let chart_xml = part(&applied.bytes, "xl/charts/chart1.xml");
    assert!(
        chart_xml.contains("A1:B4") == false,
        "chart stores separate category/value references"
    );
    assert!(chart_xml.contains("Sheet1!$A$2:$A$4"), "{chart_xml}");
    assert!(chart_xml.contains("Sheet1!$B$2:$B$4"), "{chart_xml}");
    let worksheet = part(&applied.bytes, "xl/worksheets/sheet1.xml");
    assert!(
        worksheet.contains("<drawing ") || worksheet.contains(":drawing "),
        "{worksheet}"
    );
    assert!(
        part(&applied.bytes, "xl/worksheets/_rels/sheet1.xml.rels")
            .contains("drawings/drawing1.xml")
    );
    assert!(
        part(&applied.bytes, "xl/drawings/_rels/drawing1.xml.rels").contains("charts/chart1.xml")
    );
    let drawing = part(&applied.bytes, "xl/drawings/drawing1.xml");
    assert!(drawing.contains("<xdr:col>5</xdr:col>"), "{drawing}");
    assert!(drawing.contains("<xdr:row>1</xdr:row>"), "{drawing}");
}

#[test]
fn a_workbook_can_create_a_native_filterable_table_and_read_its_rows() {
    let cells = BTreeMap::from([
        ("A1".into(), text("Day")),
        ("B1".into(), text("Visitors")),
        ("A2".into(), text("Monday")),
        ("B2".into(), CellValue::Number(25.0)),
        ("A3".into(), text("Tuesday")),
        ("B3".into(), CellValue::Number(31.0)),
    ]);
    let applied = create(
        "xlsx",
        vec![
            OfficeOp::SetCells {
                sheet: "Sheet1".into(),
                cells,
            },
            OfficeOp::AddExcelTable {
                sheet: "Sheet1".into(),
                range: "A1:B3".into(),
                name: Some("DailyVisitors".into()),
            },
        ],
    )
    .unwrap();
    let table = applied
        .document
        .tables
        .iter()
        .find(|table| table.anchor == "DailyVisitors")
        .unwrap();
    assert_eq!(
        table.rows,
        vec![
            vec!["Day", "Visitors"],
            vec!["Monday", "25"],
            vec!["Tuesday", "31"]
        ]
    );
    assert_eq!(
        applied.document.table_styles,
        vec![read::TableStyleRange {
            sheet_anchor: "Sheet1!".into(),
            range: "A1:B3".into(),
            style_name: Some("TableStyleMedium2".into()),
            show_row_stripes: true,
            show_column_stripes: false,
        }]
    );
    assert_eq!(
        vak_ooxml::projection::project(&applied.document, 0, vak_ooxml::projection::PAGE_BYTES)
            .table_styles,
        applied.document.table_styles
    );
    assert!(
        applied
            .document
            .units
            .iter()
            .any(|unit| unit.anchor == "DailyVisitors/r3" && unit.text == "Tuesday | 31")
    );
    let table_xml = part(&applied.bytes, "xl/tables/table1.xml");
    assert!(table_xml.contains("displayName=\"DailyVisitors\""));
    assert!(table_xml.contains("TableStyleMedium2"));
    let sheet_xml = part(&applied.bytes, "xl/worksheets/sheet1.xml");
    assert!(sheet_xml.contains("tableParts count=\"1\""));
    assert!(
        part(&applied.bytes, "xl/worksheets/_rels/sheet1.xml.rels").contains("relationships/table")
    );
}

#[test]
fn a_workbook_image_preview_keeps_its_excel_cell_anchor() {
    let applied = create(
        "xlsx",
        vec![
            OfficeOp::AddExcelImage {
                sheet: "Sheet1".into(),
                cell: "D2".into(),
                image: SlideImage {
                    mime_type: "image/png".into(),
                    data: "iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVR4nGNQaHjwHwAExAKAc00zmAAAAABJRU5ErkJggg==".into(),
                    alt_text: "Daily trend marker".into(),
                },
            },
            OfficeOp::AddExcelImage {
                sheet: "Sheet1".into(),
                cell: "G2".into(),
                image: SlideImage {
                mime_type: "image/png".into(),
                data: "iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVR4nGNQaHjwHwAExAKAc00zmAAAAABJRU5ErkJggg==".into(),
                    alt_text: "Second daily marker".into(),
                },
            },
        ],
    )
    .unwrap();
    let unit = applied
        .document
        .units
        .iter()
        .find(|unit| unit.kind == UnitKind::Image)
        .unwrap();
    assert_eq!(unit.text, "Daily trend marker");
    assert!(
        unit.labels
            .iter()
            .any(|label| label == "image position: Sheet1!D2")
    );
    let previews = read::image_previews(Cursor::new(applied.bytes), Limits::default()).unwrap();
    assert_eq!(previews.len(), 2);
    assert_eq!(previews[0].cell.as_deref(), Some("D2"));
    assert_eq!(previews[0].alt_text, "Daily trend marker");
    assert!(previews[0].data_url.starts_with("data:image/png;base64,"));
    assert_eq!(previews[1].cell.as_deref(), Some("G2"));
    assert_eq!(previews[1].alt_text, "Second daily marker");
}

#[test]
fn a_workbook_chart_defaults_to_the_first_row_below_its_source_range() {
    let cells = BTreeMap::from([
        ("A1".into(), text("Day")),
        ("B1".into(), text("Visitors")),
        ("A2".into(), text("Monday")),
        ("B2".into(), CellValue::Number(25.0)),
        ("A3".into(), text("Tuesday")),
        ("B3".into(), CellValue::Number(31.0)),
    ]);
    let applied = create(
        "xlsx",
        vec![
            OfficeOp::SetCells {
                sheet: "Sheet1".into(),
                cells,
            },
            OfficeOp::AddChart {
                sheet: "Sheet1".into(),
                range: "A1:B3".into(),
                chart_type: "bar".into(),
                title: "Daily visitors".into(),
                cell: None,
            },
        ],
    )
    .unwrap();
    let chart = applied
        .document
        .tables
        .iter()
        .find(|table| table.anchor == "chart:chart1.xml")
        .unwrap();
    assert!(
        chart
            .labels
            .iter()
            .any(|label| label == "chart position: Sheet1!A4")
    );
}

#[test]
fn default_chart_avoids_a_later_anchored_image() {
    let cells = BTreeMap::from([
        ("A1".into(), text("Day")),
        ("B1".into(), text("Visitors")),
        ("A2".into(), text("Monday")),
        ("B2".into(), CellValue::Number(25.0)),
        ("A3".into(), text("Tuesday")),
        ("B3".into(), CellValue::Number(31.0)),
    ]);
    let ops = vec![
            OfficeOp::SetCells {
                sheet: "Sheet1".into(),
                cells,
            },
            OfficeOp::AddChart {
                sheet: "Sheet1".into(),
                range: "A1:B3".into(),
                chart_type: "bar".into(),
                title: "Daily visitors".into(),
                cell: None,
            },
            OfficeOp::AddExcelImage {
                sheet: "Sheet1".into(),
                cell: "D2".into(),
                image: SlideImage {
                    mime_type: "image/png".into(),
                    data: "iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVR4nGNQaHjwHwAExAKAc00zmAAAAABJRU5ErkJggg==".into(),
                    alt_text: "Daily marker".into(),
                },
            },
        ];
    let blank = blank(format("xlsx")).unwrap();
    let applied = edit::apply(&blank, &ops, &clean(), Limits::default(), None).unwrap();
    let chart = applied
        .document
        .tables
        .iter()
        .find(|table| table.anchor == "chart:chart1.xml")
        .unwrap();
    assert!(
        chart
            .labels
            .iter()
            .any(|label| label == "chart position: Sheet1!A10"),
        "the default chart stays below its data and clear of the D2 image: {:?}",
        chart.labels
    );
    assert!(
        applied
            .document
            .lines()
            .join("\n")
            .contains("image position: Sheet1!D2")
    );
    let choices = review::choices(
        &blank,
        &ops,
        &clean(),
        Limits::default(),
        None,
        &applied.document,
    )
    .unwrap();
    assert!(
        choices
            .iter()
            .any(|choice| choice.label.contains("at Sheet1!A10")),
        "Review names the actual collision-free chart cell: {choices:?}"
    );
}

#[test]
fn a_sheet_is_not_renamed_while_anything_names_it() {
    let error = create(
        "xlsx",
        vec![
            OfficeOp::SetCells {
                sheet: WORKBOOK_SHEET.into(),
                cells: [
                    ("A1".to_string(), CellValue::Number(2.0)),
                    ("A2".to_string(), text("=Sheet1!A1*2")),
                ]
                .into_iter()
                .collect(),
            },
            OfficeOp::RenameSheet {
                sheet: WORKBOOK_SHEET.into(),
                name: "Budget".into(),
            },
        ],
    )
    .unwrap_err();
    assert_eq!(error.op, Some((1, "rename_sheet")));
    assert!(
        error.message.contains("xl/worksheets/sheet1.xml"),
        "{error}"
    );

    let error = create(
        "xlsx",
        vec![OfficeOp::RenameSheet {
            sheet: "Missing".into(),
            name: "Budget".into(),
        }],
    )
    .unwrap_err();
    assert!(error.message.contains("sheets: Sheet1"), "{error}");
}

#[test]
fn a_deck_is_built_from_the_blanks_layouts_with_speaker_notes() {
    let slide = |layout: &str, placeholders: &[(&str, TextValue)], notes: Option<&str>| {
        OfficeOp::AddSlideFromLayout {
            layout: layout.into(),
            after: None,
            placeholders: placeholders
                .iter()
                .map(|(key, value)| (key.to_string(), value.clone()))
                .collect(),
            tables: BTreeMap::new(),
            charts: BTreeMap::new(),
            images: BTreeMap::new(),
            notes: notes.map(str::to_string),
        }
    };
    let one = |text: &str| TextValue::One(text.into());
    let lines =
        |items: &[&str]| TextValue::Lines(items.iter().map(|item| item.to_string()).collect());
    let applied = create(
        "pptx",
        vec![
            slide(
                "Title Slide",
                &[
                    ("title", one("Launch plan")),
                    ("subtitle", one("October 2026")),
                ],
                None,
            ),
            slide(
                "Title and Content",
                &[
                    ("title", one("Goals")),
                    ("body", lines(&["Ship v1", "Sign ten customers"])),
                ],
                Some("Keep this short.\nPause for questions."),
            ),
            slide(
                "Section Header",
                &[("title", one("Timeline")), ("body", one("Q4 milestones"))],
                None,
            ),
            slide(
                "Two Content",
                &[
                    ("title", one("Trade-offs")),
                    ("idx:1", lines(&["Faster"])),
                    ("idx:2", lines(&["Riskier"])),
                ],
                None,
            ),
            slide("Title Only", &[("title", one("Questions?"))], None),
            slide("Blank", &[], None),
            OfficeOp::SetTitle {
                title: "Launch plan".into(),
            },
        ],
    )
    .unwrap();
    let document = &applied.document;
    let titles: Vec<&str> = document
        .sections
        .iter()
        .map(|section| section.title.as_str())
        .collect();
    assert_eq!(
        titles,
        [
            "Slide 1: Launch plan",
            "Slide 2: Goals",
            "Slide 3: Timeline",
            "Slide 4: Trade-offs",
            "Slide 5: Questions?",
            "Slide 6: Slide 6",
        ]
    );
    let text = document.lines().join("\n");
    assert!(
        text.contains("[slide:257/notes] Keep this short. ¶ Pause for questions."),
        "{text}"
    );
    assert!(text.contains("Ship v1 ¶ Sign ten customers"), "{text}");
    assert!(
        document.units.iter().any(|unit| {
            unit.text.contains("Ship v1")
                && unit
                    .labels
                    .iter()
                    .any(|label| label.starts_with("shape position: Slide 2:"))
        }),
        "text shapes retain their placeholder position for the deck Canvas"
    );
    // A text placeholder keeps its type; a content placeholder has none.
    assert!(
        part(&applied.bytes, "ppt/slides/slide3.xml").contains(r#"<p:ph type="body" idx="1"/>"#)
    );
    assert!(part(&applied.bytes, "ppt/slides/slide2.xml").contains(r#"<p:ph idx="1"/>"#));
    let notes_rels = part(&applied.bytes, "ppt/notesSlides/_rels/notesSlide1.xml.rels");
    assert!(
        notes_rels.contains("../notesMasters/notesMaster1.xml"),
        "{notes_rels}"
    );
    assert!(notes_rels.contains("../slides/slide2.xml"), "{notes_rels}");
    assert!(
        part(&applied.bytes, "[Content_Types].xml")
            .contains(r#"<Override PartName="/ppt/notesSlides/notesSlide1.xml""#)
    );
    assert_eq!(DECK_LAYOUTS.len(), 6);

    // set_notes gives a slide without notes its notes page.
    let with_notes = edit::apply(
        &applied.bytes,
        &[OfficeOp::SetNotes {
            anchor: "slide:256".into(),
            text: "Welcome everyone.".into(),
        }],
        &clean(),
        Limits::default(),
        None,
    )
    .unwrap();
    assert!(
        with_notes
            .document
            .lines()
            .join("\n")
            .contains("[slide:256/notes] Welcome everyone.")
    );
}

#[test]
fn a_deck_can_create_a_native_table_in_a_layout_placeholder() {
    let applied = create(
        "pptx",
        vec![OfficeOp::AddSlideFromLayout {
            layout: "Title and Content".into(),
            after: None,
            placeholders: BTreeMap::from([("title".into(), TextValue::One("Weekly sales".into()))]),
            tables: BTreeMap::from([(
                "body".into(),
                vec![
                    vec![text("Region"), text("Sales")],
                    vec![text("North"), CellValue::Number(120.0)],
                    vec![text("South"), CellValue::Number(95.0)],
                ],
            )]),
            charts: BTreeMap::new(),
            images: BTreeMap::new(),
            notes: None,
        }],
    )
    .unwrap();
    let table = applied.document.tables.first().unwrap();
    assert!(table.title.starts_with("Slide 1 · "), "{}", table.title);
    assert_eq!(
        table.rows,
        vec![
            vec!["Region", "Sales"],
            vec!["North", "120"],
            vec!["South", "95"],
        ]
    );
    assert!(part(&applied.bytes, "ppt/slides/slide1.xml").contains("<a:tbl>"));
}

#[test]
fn a_deck_can_create_a_native_chart_with_rag_readable_cached_values() {
    let applied = create(
        "pptx",
        vec![OfficeOp::AddSlideFromLayout {
            layout: "Title and Content".into(),
            after: None,
            placeholders: BTreeMap::from([(
                "title".into(),
                TextValue::One("Daily average".into()),
            )]),
            tables: BTreeMap::new(),
            charts: BTreeMap::from([(
                "body".into(),
                SlideChart {
                    title: "Visitors by day".into(),
                    chart_type: "bar".into(),
                    categories: vec!["Mon".into(), "Tue".into(), "Wed".into()],
                    values: vec![25.0, 31.0, 28.0],
                },
            )]),
            images: BTreeMap::new(),
            notes: None,
        }],
    )
    .unwrap();
    let table = applied
        .document
        .tables
        .iter()
        .find(|table| {
            table
                .rows
                .iter()
                .any(|row| row.first().is_some_and(|cell| cell == "Tue"))
        })
        .expect("chart cache is projected as a table for extraction and RAG");
    assert_eq!(table.anchor, "slide:256/chart:chart1.xml");
    assert_eq!(table.rows[0], vec!["Category", "Series 1"]);
    assert_eq!(table.rows[2], vec!["Tue", "31"]);
    assert!(
        table
            .labels
            .iter()
            .any(|label| label.starts_with("chart position: Slide 1:")),
        "chart location is preserved for the slide Canvas: {:?}",
        table.labels
    );
    assert!(
        applied.document.units.iter().any(|unit| {
            unit.anchor == "slide:256/chart:chart1.xml/r3" && unit.text == "Tue | 31"
        })
    );
    let slide = part(&applied.bytes, "ppt/slides/slide1.xml");
    assert!(slide.contains("<p:graphicFrame>"), "{slide}");
    let rels = part(&applied.bytes, "ppt/slides/_rels/slide1.xml.rels");
    assert!(rels.contains("../charts/chart1.xml"), "{rels}");
    let chart = part(&applied.bytes, "ppt/charts/chart1.xml");
    assert!(chart.contains("Visitors by day"), "{chart}");
    assert!(chart.contains("<c:v>Tue</c:v>"), "{chart}");
    assert!(chart.contains("<c:v>31</c:v>"), "{chart}");
    assert!(
        part(&applied.bytes, "[Content_Types].xml")
            .contains("application/vnd.openxmlformats-officedocument.drawingml.chart+xml")
    );
}

#[test]
fn a_deck_can_embed_a_png_with_searchable_alternative_text() {
    let image = SlideImage {
        mime_type: "image/png".into(),
        data: "iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVR4nGNQaHjwHwAExAKAc00zmAAAAABJRU5ErkJggg==".into(),
        alt_text: "A blue square used as the daily status marker".into(),
    };
    let applied = create(
        "pptx",
        vec![OfficeOp::AddSlideFromLayout {
            layout: "Title and Content".into(),
            after: None,
            placeholders: BTreeMap::from([("title".into(), TextValue::One("Daily status".into()))]),
            tables: BTreeMap::new(),
            charts: BTreeMap::new(),
            images: BTreeMap::from([("body".into(), image)]),
            notes: None,
        }],
    )
    .unwrap();
    assert!(
        applied
            .document
            .lines()
            .join("\n")
            .contains("A blue square used as the daily status marker")
    );
    assert!(applied.document.units.iter().any(|unit| {
        unit.kind == UnitKind::Image
            && unit
                .labels
                .iter()
                .any(|label| label.starts_with("image position: Slide 1:"))
    }));
    assert!(
        part(&applied.bytes, "ppt/slides/slide1.xml")
            .contains("descr=\"A blue square used as the daily status marker\"")
    );
    let mut package = Package::open(Cursor::new(applied.bytes), Limits::default()).unwrap();
    let media = package.read_part("ppt/media/image1.png").unwrap();
    assert!(media.starts_with(b"\x89PNG\r\n\x1a\n"));
}

#[test]
fn a_word_document_can_embed_a_png_with_searchable_alternative_text() {
    let applied = create(
        "docx",
        vec![OfficeOp::AddImage {
            image: SlideImage {
                mime_type: "image/png".into(),
                data: "iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVR4nGNQaHjwHwAExAKAc00zmAAAAABJRU5ErkJggg==".into(),
                alt_text: "A blue square used as the daily status marker".into(),
            },
            after: None,
        }],
    )
    .unwrap();
    assert!(
        applied
            .document
            .lines()
            .join("\n")
            .contains("A blue square used as the daily status marker")
    );
    assert!(applied.document.units.iter().any(|unit| {
        unit.kind == UnitKind::Image
            && unit
                .labels
                .iter()
                .any(|label| label.starts_with("image position: inline with p:"))
    }));
    let mut package = Package::open(Cursor::new(applied.bytes), Limits::default()).unwrap();
    assert!(
        package
            .read_part("word/media/image1.png")
            .unwrap()
            .starts_with(b"\x89PNG\r\n\x1a\n")
    );
}

#[test]
fn a_word_image_is_one_review_choice() {
    let source = blank(format("docx")).unwrap();
    let ops = vec![OfficeOp::AddImage {
        image: SlideImage {
            mime_type: "image/png".into(),
            data: "iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVR4nGNQaHjwHwAExAKAc00zmAAAAABJRU5ErkJggg==".into(),
            alt_text: "A blue square used as the daily status marker".into(),
        },
        after: None,
    }];
    let full = edit::apply(&source, &ops, &clean(), Limits::default(), None).unwrap();
    let choices = review::choices(
        &source,
        &ops,
        &clean(),
        Limits::default(),
        None,
        &full.document,
    )
    .unwrap();
    assert_eq!(choices.len(), 1);
    assert_eq!(
        choices[0].label,
        "New image: A blue square used as the daily status marker"
    );
}

#[test]
fn a_template_name_makes_a_template_from_the_blank() {
    for (extension, kind) in [
        ("dotx", FormatKind::Template),
        ("potx", FormatKind::Template),
        ("ppsx", FormatKind::Slideshow),
    ] {
        let op = match format(extension).vocabulary {
            Vocabulary::Word => paragraph("Letterhead", Some("Title")),
            _ => OfficeOp::AddSlideFromLayout {
                layout: "Title Slide".into(),
                after: None,
                placeholders: BTreeMap::new(),
                tables: BTreeMap::new(),
                charts: BTreeMap::new(),
                images: BTreeMap::new(),
                notes: None,
            },
        };
        let applied = create(extension, vec![op]).unwrap();
        assert_eq!(applied.document.inspection.format.kind, kind, "{extension}");
    }
}

#[test]
fn a_from_scratch_draft_is_offered_change_by_change() {
    let source = blank(format("docx")).unwrap();
    let ops = vec![
        paragraph("Summary", Some("Heading 1")),
        OfficeOp::AddTable {
            rows: vec![vec![text("A"), text("B")]],
            after: None,
            header: None,
        },
        // Written against a read of the first draft: after the table's
        // second cell paragraph, which op 2 minted.
        OfficeOp::AddParagraph {
            text: "In the cell".into(),
            style: None,
            after: Some("p:1A000003".into()),
        },
        paragraph("Closing line", None),
    ];
    let full = edit::apply(&source, &ops, &clean(), Limits::default(), None).unwrap();
    let offered = review::choices(
        &source,
        &ops,
        &clean(),
        Limits::default(),
        None,
        &full.document,
    )
    .unwrap();
    let requires: Vec<Vec<String>> = offered
        .iter()
        .map(|choice| choice.requires.clone())
        .collect();
    assert_eq!(requires, [vec![], vec![], vec!["1".to_string()], vec![]]);
    assert_eq!(offered[0].label, "New paragraph: Summary");
    assert_eq!(offered[1].label, "New table, 1 row by 2 columns");

    // Leaving the heading out still puts the cell paragraph in the table's
    // second cell: the replay remaps the anchors it mints.
    let narrowed = review::narrow(
        &source,
        &ops,
        &["1".into(), "2".into(), "3".into()],
        &clean(),
        Limits::default(),
        None,
        &full.document,
    )
    .unwrap();
    let kept = narrowed.document.lines().join("\n");
    assert!(!kept.contains("Summary"), "{kept}");
    assert!(
        kept.contains("[p:1A000002] B [p:1A000003] In the cell"),
        "{kept}"
    );

    let excel_source = blank(format("xlsx")).unwrap();
    let excel_ops = vec![
        OfficeOp::RenameSheet {
            sheet: "Sheet1".into(),
            name: "Budget".into(),
        },
        OfficeOp::SetCells {
            sheet: "Budget".into(),
            cells: [("A1".to_string(), text("Rent"))].into_iter().collect(),
        },
    ];
    let excel = edit::apply(&excel_source, &excel_ops, &clean(), Limits::default(), None).unwrap();
    let offered = review::choices(
        &excel_source,
        &excel_ops,
        &clean(),
        Limits::default(),
        None,
        &excel.document,
    )
    .unwrap();
    assert_eq!(
        offered[1].requires,
        ["0"],
        "a cell on the renamed sheet needs the rename"
    );
}
