//! Vakyartha's built-in blank packages, where a file created from scratch
//! starts (docs/design/72-openxml-documents.md, "Creating from scratch").
//!
//! They are written for Vakyartha, not copied from any application, and
//! zipped with fixed timestamps, so one version always yields the same
//! bytes. The parts are plain XML under `blank/` beside this crate. A blank
//! has no content: a new file's content comes only from ops, which the edit
//! engine checks by a re-read like any other edit.

use std::io::{Cursor, Write};

use crate::{Format, FormatKind, Vocabulary};

const THEME: &str = include_str!("../blank/shared/theme.xml");
const PACKAGE_RELS: &str = include_str!("../blank/shared/package.rels");
const CORE: &str = include_str!("../blank/shared/core.xml");
const APP: &str = include_str!("../blank/shared/app.xml");

const WORD: &[(&str, &str)] = &[
    (
        "[Content_Types].xml",
        include_str!("../blank/word/content-types.xml"),
    ),
    (
        "word/document.xml",
        include_str!("../blank/word/document.xml"),
    ),
    (
        "word/_rels/document.xml.rels",
        include_str!("../blank/word/document.xml.rels"),
    ),
    ("word/styles.xml", include_str!("../blank/word/styles.xml")),
    (
        "word/numbering.xml",
        include_str!("../blank/word/numbering.xml"),
    ),
    (
        "word/settings.xml",
        include_str!("../blank/word/settings.xml"),
    ),
    (
        "word/fontTable.xml",
        include_str!("../blank/word/fontTable.xml"),
    ),
];

const EXCEL: &[(&str, &str)] = &[
    (
        "[Content_Types].xml",
        include_str!("../blank/excel/content-types.xml"),
    ),
    (
        "xl/workbook.xml",
        include_str!("../blank/excel/workbook.xml"),
    ),
    (
        "xl/_rels/workbook.xml.rels",
        include_str!("../blank/excel/workbook.xml.rels"),
    ),
    (
        "xl/worksheets/sheet1.xml",
        include_str!("../blank/excel/sheet1.xml"),
    ),
    ("xl/styles.xml", include_str!("../blank/excel/styles.xml")),
];

const POWERPOINT: &[(&str, &str)] = &[
    (
        "[Content_Types].xml",
        include_str!("../blank/powerpoint/content-types.xml"),
    ),
    (
        "ppt/presentation.xml",
        include_str!("../blank/powerpoint/presentation.xml"),
    ),
    (
        "ppt/_rels/presentation.xml.rels",
        include_str!("../blank/powerpoint/presentation.xml.rels"),
    ),
    (
        "ppt/slideMasters/slideMaster1.xml",
        include_str!("../blank/powerpoint/slideMaster1.xml"),
    ),
    (
        "ppt/slideMasters/_rels/slideMaster1.xml.rels",
        include_str!("../blank/powerpoint/slideMaster1.xml.rels"),
    ),
    (
        "ppt/slideLayouts/slideLayout1.xml",
        include_str!("../blank/powerpoint/slideLayout1.xml"),
    ),
    (
        "ppt/slideLayouts/_rels/slideLayout1.xml.rels",
        include_str!("../blank/powerpoint/slideLayout1.xml.rels"),
    ),
    (
        "ppt/slideLayouts/slideLayout2.xml",
        include_str!("../blank/powerpoint/slideLayout2.xml"),
    ),
    (
        "ppt/slideLayouts/_rels/slideLayout2.xml.rels",
        include_str!("../blank/powerpoint/slideLayout2.xml.rels"),
    ),
    (
        "ppt/slideLayouts/slideLayout3.xml",
        include_str!("../blank/powerpoint/slideLayout3.xml"),
    ),
    (
        "ppt/slideLayouts/_rels/slideLayout3.xml.rels",
        include_str!("../blank/powerpoint/slideLayout3.xml.rels"),
    ),
    (
        "ppt/slideLayouts/slideLayout4.xml",
        include_str!("../blank/powerpoint/slideLayout4.xml"),
    ),
    (
        "ppt/slideLayouts/_rels/slideLayout4.xml.rels",
        include_str!("../blank/powerpoint/slideLayout4.xml.rels"),
    ),
    (
        "ppt/slideLayouts/slideLayout5.xml",
        include_str!("../blank/powerpoint/slideLayout5.xml"),
    ),
    (
        "ppt/slideLayouts/_rels/slideLayout5.xml.rels",
        include_str!("../blank/powerpoint/slideLayout5.xml.rels"),
    ),
    (
        "ppt/slideLayouts/slideLayout6.xml",
        include_str!("../blank/powerpoint/slideLayout6.xml"),
    ),
    (
        "ppt/slideLayouts/_rels/slideLayout6.xml.rels",
        include_str!("../blank/powerpoint/slideLayout6.xml.rels"),
    ),
    (
        "ppt/notesMasters/notesMaster1.xml",
        include_str!("../blank/powerpoint/notesMaster1.xml"),
    ),
    (
        "ppt/notesMasters/_rels/notesMaster1.xml.rels",
        include_str!("../blank/powerpoint/notesMaster1.xml.rels"),
    ),
    (
        "ppt/presProps.xml",
        include_str!("../blank/powerpoint/presProps.xml"),
    ),
    (
        "ppt/viewProps.xml",
        include_str!("../blank/powerpoint/viewProps.xml"),
    ),
    (
        "ppt/tableStyles.xml",
        include_str!("../blank/powerpoint/tableStyles.xml"),
    ),
];

/// The layouts a blank deck offers, by the names `add_slide_from_layout`
/// takes, each with the placeholder keys its slides fill.
pub const DECK_LAYOUTS: &[(&str, &str)] = &[
    ("Title Slide", "title, subtitle"),
    ("Title and Content", "title, body"),
    ("Section Header", "title, body"),
    ("Two Content", "title, idx:1, idx:2"),
    ("Title Only", "title"),
    ("Blank", "no placeholders"),
];

/// The paragraph styles a blank Word document offers, by name.
pub const DOCUMENT_STYLES: &[&str] = &[
    "Normal",
    "Title",
    "Subtitle",
    "Heading 1",
    "Heading 2",
    "Heading 3",
    "List Bullet",
    "List Number",
    "Quote",
];

/// The one sheet a blank workbook has.
pub const WORKBOOK_SHEET: &str = "Sheet1";

/// The blank package a new file named as `format` starts from. It is the
/// document form of the vocabulary; a template name (`.dotx`, `.xltx`,
/// `.potx`) is made by the edit engine changing the main part's content
/// type, exactly as it does when a template is the source. Refuses a
/// macro-enabled format (O10) and Visio, which has no editing engine yet.
pub fn blank(format: Format) -> Result<Vec<u8>, String> {
    if format.macro_enabled || format.kind == FormatKind::AddIn {
        return Err(format!(
            "Vakyartha never makes a macro-enabled file, so a .{} cannot be created; name it .{} instead",
            format.extension(),
            Format {
                macro_enabled: false,
                kind: if format.kind == FormatKind::AddIn {
                    FormatKind::Document
                } else {
                    format.kind
                },
                ..format
            }
            .extension()
        ));
    }
    let (main, parts, major, minor, theme) = match format.vocabulary {
        Vocabulary::Word => (
            "word/document.xml",
            WORD,
            "Aptos Display",
            "Aptos",
            &["word/theme/theme1.xml"][..],
        ),
        Vocabulary::Excel => (
            "xl/workbook.xml",
            EXCEL,
            "Aptos Display",
            "Aptos Narrow",
            &["xl/theme/theme1.xml"][..],
        ),
        Vocabulary::PowerPoint => (
            "ppt/presentation.xml",
            POWERPOINT,
            "Aptos Display",
            "Aptos",
            &["ppt/theme/theme1.xml", "ppt/theme/theme2.xml"][..],
        ),
        Vocabulary::Visio => {
            return Err(
                "a Visio drawing cannot be created from scratch yet; Vakyartha reads Visio files but has no Visio editing yet. Tell the person so, and do not build one with a command or script".into(),
            );
        }
    };
    let theme_xml = THEME
        .replace("{{MAJOR}}", major)
        .replace("{{MINOR}}", minor);
    let package_rels = PACKAGE_RELS.replace("{{MAIN}}", main);
    let mut entries: Vec<(&str, &str)> = Vec::with_capacity(parts.len() + 6);
    entries.push(parts[0]);
    entries.push(("_rels/.rels", &package_rels));
    entries.push(("docProps/core.xml", CORE));
    entries.push(("docProps/app.xml", APP));
    entries.extend_from_slice(&parts[1..]);
    for name in theme {
        entries.push((name, &theme_xml));
    }
    zip(&entries)
}

/// Zips the parts in order with Deflate and a fixed timestamp.
fn zip(entries: &[(&str, &str)]) -> Result<Vec<u8>, String> {
    let mut writer = zip::ZipWriter::new(Cursor::new(Vec::new()));
    let options = zip::write::SimpleFileOptions::default()
        .compression_method(zip::CompressionMethod::Deflated)
        .last_modified_time(zip::DateTime::default());
    for (name, text) in entries {
        writer
            .start_file(*name, options)
            .and_then(|()| writer.write_all(text.as_bytes()).map_err(Into::into))
            .map_err(|error| format!("could not build the blank package: {error}"))?;
    }
    writer
        .finish()
        .map(Cursor::into_inner)
        .map_err(|error| format!("could not build the blank package: {error}"))
}
