//! PowerPoint ops. New slides come only from the deck's own layouts and
//! their placeholders, never free-positioned shapes, so output follows the
//! template and stays reliable with small models.

use std::collections::BTreeMap;
use std::io::{Read, Seek};

use super::{CellValue, EditError, Expect, Outcome, TextValue, Work, fail, free_part_name};
use crate::package::rels_part_name;
use crate::splice::{Splice, Tree, escape_attr, escape_text};

const P: &str = "http://schemas.openxmlformats.org/presentationml/2006/main";
const P_STRICT: &str = "http://purl.oclc.org/ooxml/presentationml/main";
const A: &str = "http://schemas.openxmlformats.org/drawingml/2006/main";
const A_STRICT: &str = "http://purl.oclc.org/ooxml/drawingml/main";
const R: &str = "http://schemas.openxmlformats.org/officeDocument/2006/relationships";
const R_STRICT: &str = "http://purl.oclc.org/ooxml/officeDocument/relationships";
const SLIDE_TYPE: &str = "application/vnd.openxmlformats-officedocument.presentationml.slide+xml";
const LAYOUT_TYPE: &str =
    "application/vnd.openxmlformats-officedocument.presentationml.slideLayout+xml";
const NOTES_TYPE: &str =
    "application/vnd.openxmlformats-officedocument.presentationml.notesSlide+xml";
const CHART_TYPE: &str = "application/vnd.openxmlformats-officedocument.drawingml.chart+xml";
const PNG_TYPE: &str = "image/png";
const JPEG_TYPE: &str = "image/jpeg";
const PNG_REL_TYPE: &str =
    "http://schemas.openxmlformats.org/officeDocument/2006/relationships/image";
const MAX_IMAGE_BYTES: usize = 1024 * 1024;

struct Deck {
    part: String,
    bytes: Vec<u8>,
    tree: Tree,
    p: String,
    r: String,
    list: Option<usize>,
    /// (slide id, relationship id, sldId node) in presentation order.
    slides: Vec<(String, String, usize)>,
}

fn deck<R2: Read + Seek>(work: &mut Work<'_, R2>) -> Result<Deck, EditError> {
    let part = work.main_part();
    let bytes = work.get(&part)?;
    let tree = Tree::parse(&bytes, &part, work.limits())?;
    let Some(p) = tree.prefix_for(P).or_else(|| tree.prefix_for(P_STRICT)) else {
        return fail("the presentation does not declare the PresentationML namespace");
    };
    if tree.children(0, "modifyVerifier").next().is_some() {
        return fail(
            "the presentation has a password to modify (O8); the owner must remove it before it can be edited",
        );
    }
    let r = tree
        .prefix_for(R)
        .or_else(|| tree.prefix_for(R_STRICT))
        .unwrap_or_else(|| "r:".into());
    let list = tree.children(0, "sldIdLst").next();
    let slides = list
        .map(|list| {
            tree.children(list, "sldId")
                .filter_map(|node| {
                    let element = &tree.nodes[node].element;
                    Some((
                        element.attr_unprefixed("id")?.to_string(),
                        element.attr_prefixed("id")?.to_string(),
                        node,
                    ))
                })
                .collect()
        })
        .unwrap_or_default();
    Ok(Deck {
        part,
        bytes,
        tree,
        p,
        r,
        list,
        slides,
    })
}

/// `slide:256` or `slide:256/...` → `256`.
fn slide_id(anchor: &str) -> Result<&str, EditError> {
    anchor
        .strip_prefix("slide:")
        .map(|rest| rest.split('/').next().unwrap_or(rest))
        .filter(|id| !id.is_empty())
        .ok_or_else(|| EditError {
            op: None,
            message: format!(
                "{anchor:?} is not a slide anchor; use one like slide:256 from doc_read"
            ),
        })
}

fn slide_part<R2: Read + Seek>(
    work: &mut Work<'_, R2>,
    deck: &Deck,
    id: &str,
) -> Result<String, EditError> {
    let Some((_, relationship, _)) = deck.slides.iter().find(|(slide, _, _)| slide == id) else {
        let known: Vec<String> = deck
            .slides
            .iter()
            .map(|(slide, _, _)| format!("slide:{slide}"))
            .collect();
        return fail(format!("no slide:{id}; slides: {}", known.join(", ")));
    };
    match work.by_id(&deck.part, relationship)? {
        Some(part) => Ok(part),
        None => fail(format!("slide:{id} points at no part")),
    }
}

fn prefixes(tree: &Tree) -> Result<(String, String), EditError> {
    let p = tree.prefix_for(P).or_else(|| tree.prefix_for(P_STRICT));
    let a = tree.prefix_for(A).or_else(|| tree.prefix_for(A_STRICT));
    match (p, a) {
        (Some(p), Some(a)) => Ok((p, a)),
        _ => fail(
            "the slide does not declare the PresentationML and DrawingML namespaces on its root",
        ),
    }
}

/// A shape's `(id, name, placeholder type, placeholder idx)`.
fn shape_facts(tree: &Tree, shape: usize) -> (String, String, Option<String>, Option<String>) {
    let (id, name) = tree
        .descendants(shape, "cNvPr")
        .next()
        .map(|node| {
            let element = &tree.nodes[node].element;
            (
                element.attr("id").unwrap_or_default().to_string(),
                element.attr("name").unwrap_or_default().to_string(),
            )
        })
        .unwrap_or_default();
    let placeholder = tree
        .descendants(shape, "ph")
        .next()
        .map(|node| &tree.nodes[node].element);
    let kind = placeholder.map(|element| element.attr("type").unwrap_or("body").to_string());
    let index = placeholder.and_then(|element| element.attr("idx").map(str::to_string));
    (id, name, kind, index)
}

/// Whether a placeholder of `kind`/`index` answers to `key` (`title`,
/// `subtitle`, `body`, a raw type, or `idx:N`).
fn placeholder_matches(key: &str, kind: Option<&str>, index: Option<&str>) -> bool {
    let key = key.trim().to_ascii_lowercase();
    if let Some(wanted) = key.strip_prefix("idx:") {
        return index == Some(wanted);
    }
    match (key.as_str(), kind) {
        ("title", Some("title" | "ctrTitle")) => true,
        ("subtitle", Some("subTitle")) => true,
        (wanted, Some(kind)) => kind.eq_ignore_ascii_case(wanted),
        _ => false,
    }
}

fn validate_chart(chart: &super::SlideChart) -> Result<(), EditError> {
    if chart.title.trim().is_empty() || chart.title.len() > 512 {
        return fail("a PowerPoint chart needs a title of 1–512 bytes");
    }
    if !matches!(chart.chart_type.as_str(), "bar" | "line" | "pie") {
        return fail("chart_type must be bar, line or pie");
    }
    if chart.categories.is_empty()
        || chart.categories.len() != chart.values.len()
        || chart.categories.len() > 100
    {
        return fail("a PowerPoint chart needs 1–100 categories and one value per category");
    }
    if chart
        .categories
        .iter()
        .any(|category| category.trim().is_empty() || category.len() > 512)
    {
        return fail("chart categories must be non-empty and at most 512 bytes each");
    }
    if chart
        .values
        .iter()
        .any(|value| !value.is_finite() || value.abs() > 1.0e12)
    {
        return fail("chart values must be finite numbers between -1e12 and 1e12");
    }
    if chart.chart_type == "pie" && chart.values.iter().any(|value| *value < 0.0) {
        return fail("pie chart values cannot be negative");
    }
    Ok(())
}

fn image_dimensions(bytes: &[u8], mime: &str) -> Result<(u32, u32), EditError> {
    let dimensions = if mime == PNG_TYPE {
        bytes.get(16..24).and_then(|dimensions| {
            Some((
                u32::from_be_bytes(dimensions.get(0..4)?.try_into().ok()?),
                u32::from_be_bytes(dimensions.get(4..8)?.try_into().ok()?),
            ))
        })
    } else {
        let mut cursor = 2usize;
        let mut found = None;
        while cursor + 4 <= bytes.len() {
            if bytes[cursor] != 0xff {
                cursor += 1;
                continue;
            }
            while bytes.get(cursor) == Some(&0xff) {
                cursor += 1;
            }
            let Some(&marker) = bytes.get(cursor) else {
                break;
            };
            cursor += 1;
            if matches!(marker, 0xd8 | 0xd9 | 0x01 | 0xd0..=0xd7) {
                continue;
            }
            let Some(length) = bytes
                .get(cursor..cursor + 2)
                .and_then(|value| value.try_into().ok())
                .map(u16::from_be_bytes)
            else {
                break;
            };
            let length = usize::from(length);
            if length < 2 || cursor + length > bytes.len() {
                break;
            }
            if matches!(marker, 0xc0..=0xc3 | 0xc5..=0xc7 | 0xc9..=0xcb | 0xcd..=0xcf) {
                found = bytes.get(cursor + 3..cursor + 7).and_then(|value| {
                    Some((
                        u32::from(u16::from_be_bytes(value.get(2..4)?.try_into().ok()?)),
                        u32::from(u16::from_be_bytes(value.get(0..2)?.try_into().ok()?)),
                    ))
                });
                break;
            }
            cursor += length;
        }
        found
    };
    let Some((width, height)) = dimensions else {
        return fail("the image has no readable pixel dimensions");
    };
    if width == 0
        || height == 0
        || width > 10_000
        || height > 10_000
        || u64::from(width) * u64::from(height) > 40_000_000
    {
        return fail(
            "image dimensions must be at most 10,000 pixels per side and 40 megapixels total",
        );
    }
    Ok((width, height))
}

pub(super) fn validate_image(image: &super::SlideImage) -> Result<(Vec<u8>, u32, u32), EditError> {
    use base64::Engine as _;
    if image.alt_text.trim().is_empty() || image.alt_text.len() > 2048 {
        return fail("a PowerPoint image needs alternative text of 1–2048 bytes");
    }
    let (mime, signature) = match image.mime_type.as_str() {
        PNG_TYPE => (PNG_TYPE, b"\x89PNG\r\n\x1a\n".as_slice()),
        JPEG_TYPE => (JPEG_TYPE, b"\xff\xd8\xff".as_slice()),
        _ => return fail("image mime_type must be image/png or image/jpeg"),
    };
    if image.data.len() > (MAX_IMAGE_BYTES * 4 / 3) + 8 {
        return fail("an embedded image must be between 1 byte and 1 MiB");
    }
    let decoded = base64::engine::general_purpose::STANDARD
        .decode(image.data.as_bytes())
        .map_err(|_| EditError {
            op: None,
            message: "image data must be standard base64".into(),
        })?;
    if decoded.is_empty() || decoded.len() > MAX_IMAGE_BYTES {
        return fail("an embedded image must be between 1 byte and 1 MiB");
    }
    if !decoded.starts_with(signature) {
        return fail(format!("image data does not match {mime}"));
    }
    let (width, height) = image_dimensions(&decoded, mime)?;
    Ok((decoded, width, height))
}

#[allow(clippy::too_many_arguments)]
fn picture_frame(
    id: usize,
    name: &str,
    alt_text: &str,
    placeholder_type: Option<&str>,
    placeholder_index: Option<&str>,
    x: i64,
    y: i64,
    width: i64,
    height: i64,
    relationship: &str,
) -> String {
    let type_attr = placeholder_type
        .map(|value| format!(r#" type="{}""#, escape_attr(value)))
        .unwrap_or_default();
    let index_attr = placeholder_index
        .map(|value| format!(r#" idx="{}""#, escape_attr(value)))
        .unwrap_or_default();
    format!(
        r#"<p:pic><p:nvPicPr><p:cNvPr id="{id}" name="{}" descr="{}"/><p:cNvPicPr><a:picLocks noChangeAspect="1"/></p:cNvPicPr><p:nvPr><p:ph{type_attr}{index_attr}/></p:nvPr></p:nvPicPr><p:blipFill><a:blip r:embed="{relationship}"/><a:stretch><a:fillRect/></a:stretch></p:blipFill><p:spPr><a:xfrm><a:off x="{x}" y="{y}"/><a:ext cx="{width}" cy="{height}"/></a:xfrm><a:prstGeom prst="rect"><a:avLst/></a:prstGeom></p:spPr></p:pic>"#,
        escape_attr(name),
        escape_attr(alt_text),
    )
}

fn create_image<R2: Read + Seek>(
    work: &mut Work<'_, R2>,
    image: &super::SlideImage,
) -> Result<(String, u32, u32), EditError> {
    let (bytes, width, height) = validate_image(image)?;
    let extension = if image.mime_type == PNG_TYPE {
        ".png"
    } else {
        ".jpg"
    };
    let part = free_part_name(work, "ppt/media/image", extension);
    work.put(&part, bytes);
    work.set_override(
        &part,
        if image.mime_type == PNG_TYPE {
            PNG_TYPE
        } else {
            JPEG_TYPE
        },
    )?;
    Ok((part, width, height))
}

fn create_chart<R2: Read + Seek>(
    work: &mut Work<'_, R2>,
    slide: &str,
    chart: &super::SlideChart,
) -> Result<(String, Vec<Vec<String>>), EditError> {
    validate_chart(chart)?;
    let part = free_part_name(work, "ppt/charts/chart", ".xml");
    let points = |values: &[String]| {
        format!(
            "<c:ptCount val=\"{}\"/>{}",
            values.len(),
            values
                .iter()
                .enumerate()
                .map(|(index, value)| format!(
                    "<c:pt idx=\"{index}\"><c:v>{}</c:v></c:pt>",
                    escape_text(value)
                ))
                .collect::<String>()
        )
    };
    let categories: Vec<String> = chart.categories.clone();
    let values: Vec<String> = chart.values.iter().map(ToString::to_string).collect();
    let series = format!(
        r#"<c:ser><c:idx val="0"/><c:order val="0"/><c:tx><c:strRef><c:f>Sheet1!$B$1</c:f><c:strCache><c:ptCount val="1"/><c:pt idx="0"><c:v>Series 1</c:v></c:pt></c:strCache></c:strRef></c:tx><c:cat><c:strLit>{}</c:strLit></c:cat><c:val><c:numLit><c:formatCode>General</c:formatCode>{}</c:numLit></c:val>{}</c:ser>"#,
        points(&categories),
        points(&values),
        if chart.chart_type == "line" {
            "<c:marker><c:symbol val=\"circle\"/><c:size val=\"5\"/></c:marker>"
        } else {
            ""
        }
    );
    let chart_kind = match chart.chart_type.as_str() {
        "bar" => format!(
            "<c:barChart><c:barDir val=\"col\"/><c:grouping val=\"clustered\"/><c:varyColors val=\"0\"/>{series}<c:gapWidth val=\"150\"/><c:axId val=\"48650112\"/><c:axId val=\"48672768\"/></c:barChart>"
        ),
        "line" => format!(
            "<c:lineChart><c:grouping val=\"standard\"/>{series}<c:axId val=\"48650112\"/><c:axId val=\"48672768\"/></c:lineChart>"
        ),
        "pie" => format!(
            "<c:pieChart><c:varyColors val=\"1\"/>{series}<c:firstSliceAng val=\"0\"/></c:pieChart>"
        ),
        _ => return fail("chart_type must be bar, line or pie"),
    };
    let axes = if chart.chart_type == "pie" {
        String::new()
    } else {
        r#"<c:catAx><c:axId val="48650112"/><c:scaling><c:orientation val="minMax"/></c:scaling><c:delete val="0"/><c:axPos val="b"/><c:tickLblPos val="nextTo"/><c:crossAx val="48672768"/><c:crosses val="autoZero"/><c:auto val="1"/><c:lblAlgn val="ctr"/><c:lblOffset val="100"/></c:catAx><c:valAx><c:axId val="48672768"/><c:scaling><c:orientation val="minMax"/></c:scaling><c:delete val="0"/><c:axPos val="l"/><c:majorGridlines/><c:numFmt formatCode="General" sourceLinked="1"/><c:tickLblPos val="nextTo"/><c:crossAx val="48650112"/><c:crosses val="autoZero"/><c:crossBetween val="between"/></c:valAx>"#.to_string()
    };
    let chart_xml = format!(
        r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?><c:chartSpace xmlns:c="http://schemas.openxmlformats.org/drawingml/2006/chart" xmlns:a="http://schemas.openxmlformats.org/drawingml/2006/main"><c:lang val="en-US"/><c:chart><c:title><c:tx><c:rich><a:bodyPr/><a:lstStyle/><a:p><a:pPr/><a:r><a:rPr lang="en-US"/><a:t>{}</a:t></a:r><a:endParaRPr lang="en-US"/></a:p></c:rich></c:tx><c:overlay val="0"/></c:title><c:autoTitleDeleted val="0"/><c:plotArea><c:layout/>{chart_kind}{axes}</c:plotArea><c:legend><c:legendPos val="b"/><c:overlay val="0"/></c:legend><c:plotVisOnly val="1"/><c:dispBlanksAs val="gap"/></c:chart><c:printSettings><c:headerFooter/><c:pageMargins b="0.75" l="0.7" r="0.7" t="0.75" header="0.3" footer="0.3"/><c:pageSetup/></c:printSettings></c:chartSpace>"#,
        escape_text(&chart.title)
    );
    work.put(&part, chart_xml.into_bytes());
    work.set_override(&part, CHART_TYPE)?;
    let rows = std::iter::once(vec!["Category".to_string(), "Series 1".to_string()])
        .chain(
            categories
                .iter()
                .cloned()
                .zip(values.iter().cloned())
                .map(|(category, value)| vec![category, value]),
        )
        .collect();
    let _ = slide;
    Ok((part, rows))
}

#[allow(clippy::too_many_arguments)]
fn chart_frame(
    id: usize,
    name: &str,
    placeholder_type: Option<&str>,
    placeholder_index: Option<&str>,
    x: i64,
    y: i64,
    width: i64,
    height: i64,
    relationship: &str,
) -> String {
    let type_attr = placeholder_type
        .map(|value| format!(r#" type="{}""#, escape_attr(value)))
        .unwrap_or_default();
    let index_attr = placeholder_index
        .map(|value| format!(r#" idx="{}""#, escape_attr(value)))
        .unwrap_or_default();
    format!(
        r#"<p:graphicFrame><p:nvGraphicFramePr><p:cNvPr id="{id}" name="{}"/><p:cNvGraphicFramePr/><p:nvPr><p:ph{type_attr}{index_attr}/></p:nvPr></p:nvGraphicFramePr><p:xfrm><a:off x="{x}" y="{y}"/><a:ext cx="{width}" cy="{height}"/></p:xfrm><a:graphic><a:graphicData uri="http://schemas.openxmlformats.org/drawingml/2006/chart"><c:chart xmlns:c="http://schemas.openxmlformats.org/drawingml/2006/chart" xmlns:r="{R}" r:id="{relationship}"/></a:graphicData></a:graphic></p:graphicFrame>"#,
        escape_attr(name)
    )
}

#[allow(clippy::type_complexity)]
fn placeholder_bounds(tree: &Tree, shape: usize) -> (Option<(i64, i64)>, Option<(i64, i64)>) {
    let offset = tree.descendants(shape, "off").next().and_then(|node| {
        let element = &tree.nodes[node].element;
        element
            .attr("x")?
            .parse::<i64>()
            .ok()
            .zip(element.attr("y")?.parse().ok())
    });
    let extent = tree.descendants(shape, "ext").next().and_then(|node| {
        let element = &tree.nodes[node].element;
        element
            .attr("cx")?
            .parse::<i64>()
            .ok()
            .zip(element.attr("cy")?.parse().ok())
    });
    (offset, extent)
}

fn paragraphs(
    a: &str,
    lines: &[String],
    paragraph_properties: &str,
    run_properties: &str,
) -> String {
    if lines.iter().all(|line| line.is_empty()) {
        return format!("<{a}p/>");
    }
    lines
        .iter()
        .map(|line| {
            if line.is_empty() {
                format!("<{a}p>{paragraph_properties}</{a}p>")
            } else {
                format!(
                    "<{a}p>{paragraph_properties}<{a}r>{run_properties}<{a}t>{}</{a}t></{a}r></{a}p>",
                    escape_text(line)
                )
            }
        })
        .collect()
}

#[allow(clippy::too_many_arguments)]
fn table_frame(
    id: usize,
    name: &str,
    placeholder_type: Option<&str>,
    placeholder_index: Option<&str>,
    x: i64,
    y: i64,
    width: i64,
    height: i64,
    rows: &[Vec<CellValue>],
) -> String {
    let cols = rows.first().map_or(1, Vec::len);
    let col_width = width / i64::try_from(cols).unwrap_or(1);
    let row_height = height / i64::try_from(rows.len()).unwrap_or(1);
    let type_attr = placeholder_type
        .map(|value| format!(r#" type="{}""#, escape_attr(value)))
        .unwrap_or_default();
    let index_attr = placeholder_index
        .map(|value| format!(r#" idx="{}""#, escape_attr(value)))
        .unwrap_or_default();
    let mut xml = format!(
        r#"<p:graphicFrame><p:nvGraphicFramePr><p:cNvPr id="{id}" name="{}"/><p:cNvGraphicFramePr/><p:nvPr><p:ph{type_attr}{index_attr}/></p:nvPr></p:nvGraphicFramePr><p:xfrm><a:off x="{x}" y="{y}"/><a:ext cx="{width}" cy="{height}"/></p:xfrm><a:graphic><a:graphicData uri="http://schemas.openxmlformats.org/drawingml/2006/table"><a:tbl><a:tblPr firstRow="1"/><a:tblGrid>"#,
        escape_attr(name)
    );
    for _ in 0..cols {
        xml.push_str(&format!(r#"<a:gridCol w="{col_width}"/>"#));
    }
    xml.push_str("</a:tblGrid>");
    for (row_index, row) in rows.iter().enumerate() {
        xml.push_str(&format!(r#"<a:tr h="{row_height}">"#));
        for value in row {
            let text = escape_text(&value.as_text());
            let bold = if row_index == 0 { r#" b="1""# } else { "" };
            xml.push_str(&format!(
                r#"<a:tc><a:txBody><a:bodyPr/><a:lstStyle/><a:p><a:r><a:rPr lang="en-US"{bold}/><a:t>{text}</a:t></a:r><a:endParaRPr lang="en-US"/></a:p></a:txBody><a:tcPr/></a:tc>"#
            ));
        }
        xml.push_str("</a:tr>");
    }
    xml.push_str("</a:tbl></a:graphicData></a:graphic></p:graphicFrame>");
    xml
}

/// Replaces the text of shape `shape` in `tree`, keeping its body and list
/// properties and the first paragraph's and run's formatting.
fn replace_shape_text(
    splice: &mut Splice,
    bytes: &[u8],
    tree: &Tree,
    shape: usize,
    lines: &[String],
) -> Result<(), EditError> {
    let (p, a) = prefixes(tree)?;
    match tree.children(shape, "txBody").next() {
        Some(body) => {
            let existing: Vec<usize> = tree.children(body, "p").collect();
            let slice = |index: usize| {
                String::from_utf8_lossy(&bytes[tree.nodes[index].span.clone()]).into_owned()
            };
            let paragraph_properties = existing
                .first()
                .and_then(|first| tree.children(*first, "pPr").next())
                .map(slice)
                .unwrap_or_default();
            let run_properties = existing
                .first()
                .and_then(|first| tree.descendants(*first, "rPr").next())
                .map(slice)
                .unwrap_or_default();
            let new = paragraphs(&a, lines, &paragraph_properties, &run_properties);
            match (existing.first(), existing.last()) {
                (Some(first), Some(last)) => splice.replace(
                    tree.nodes[*first].span.start..tree.nodes[*last].span.end,
                    new,
                ),
                _ => {
                    let node = &tree.nodes[body];
                    if node.is_empty_element() {
                        return fail("the shape's text body is empty markup this op cannot extend");
                    }
                    splice.insert(node.inner.end, new);
                }
            }
        }
        None => {
            let after = tree
                .children(shape, "style")
                .chain(tree.children(shape, "spPr"))
                .map(|index| tree.nodes[index].span.end)
                .max();
            let Some(after) = after else {
                return fail("the shape has no shape properties to place text after");
            };
            splice.insert(
                after,
                format!(
                    "<{p}txBody><{a}bodyPr/><{a}lstStyle/>{}</{p}txBody>",
                    paragraphs(&a, lines, "", "")
                ),
            );
        }
    }
    Ok(())
}

fn expect_text(anchor: String, lines: &[String]) -> Expect {
    let needles: Vec<String> = lines
        .iter()
        .map(|line| line.trim().to_string())
        .filter(|line| !line.is_empty())
        .collect();
    if needles.is_empty() {
        Expect::Absent { anchor }
    } else {
        Expect::UnitContains { anchor, needles }
    }
}

pub(super) fn set_placeholder_text<R2: Read + Seek>(
    work: &mut Work<'_, R2>,
    anchor: &str,
    text: &TextValue,
) -> Result<Outcome, EditError> {
    let deck = deck(work)?;
    let id = slide_id(anchor)?.to_string();
    let target = anchor
        .split_once('/')
        .map(|(_, rest)| rest.to_string())
        .ok_or_else(|| EditError {
            op: None,
            message: format!(
                "{anchor:?} names no shape; use slide:ID/shape:N or slide:ID/placeholder:title"
            ),
        })?;
    let part = slide_part(work, &deck, &id)?;
    let bytes = work.get(&part)?;
    let tree = Tree::parse(&bytes, &part, work.limits())?;
    let mut shapes = Vec::new();
    let mut found = None;
    for shape in tree.descendants(0, "sp") {
        let (shape_id, name, kind, index) = shape_facts(&tree, shape);
        let hit = match target.split_once(':') {
            Some(("shape", wanted)) => shape_id == wanted,
            Some(("placeholder", wanted)) => {
                placeholder_matches(wanted, kind.as_deref(), index.as_deref())
            }
            _ => false,
        };
        if hit && found.is_none() {
            found = Some((shape, shape_id.clone()));
        }
        shapes.push(match kind {
            Some(kind) => format!("shape:{shape_id} ({name}, placeholder {kind})"),
            None => format!("shape:{shape_id} ({name})"),
        });
    }
    let Some((shape, shape_id)) = found else {
        return fail(format!(
            "slide:{id} has no text shape {target}; shapes: {}",
            shapes.join(", ")
        ));
    };
    let lines = text.lines();
    let mut splice = Splice::default();
    replace_shape_text(&mut splice, &bytes, &tree, shape, &lines)?;
    work.put(&part, splice.apply(&bytes, &part)?);
    Ok(Outcome {
        summary: format!(
            "slide:{id}/shape:{shape_id} text set ({} line(s))",
            lines.len()
        ),
        expect: vec![expect_text(format!("slide:{id}/shape:{shape_id}"), &lines)],
        created: Vec::new(),
    })
}

/// Creates `slide`'s notes page, holding `lines`, from the deck's notes
/// master, and returns its part.
fn create_notes<R2: Read + Seek>(
    work: &mut Work<'_, R2>,
    deck_part: &str,
    slide: &str,
    lines: &[String],
) -> Result<String, EditError> {
    let Some(master) = work.related(deck_part, "notesMaster")? else {
        return fail(
            "this deck has no notes master, so speaker notes cannot be added to a slide that has none; open it in PowerPoint and add notes there once",
        );
    };
    let part = free_part_name(work, "ppt/notesSlides/notesSlide", ".xml");
    work.put(
        &part,
        format!(
            r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<p:notes xmlns:a="{A}" xmlns:r="{R}" xmlns:p="{P}"><p:cSld><p:spTree><p:nvGrpSpPr><p:cNvPr id="1" name=""/><p:cNvGrpSpPr/><p:nvPr/></p:nvGrpSpPr><p:grpSpPr><a:xfrm><a:off x="0" y="0"/><a:ext cx="0" cy="0"/><a:chOff x="0" y="0"/><a:chExt cx="0" cy="0"/></a:xfrm></p:grpSpPr><p:sp><p:nvSpPr><p:cNvPr id="2" name="Slide Image Placeholder 1"/><p:cNvSpPr><a:spLocks noGrp="1" noRot="1" noChangeAspect="1"/></p:cNvSpPr><p:nvPr><p:ph type="sldImg"/></p:nvPr></p:nvSpPr><p:spPr/></p:sp><p:sp><p:nvSpPr><p:cNvPr id="3" name="Notes Placeholder 2"/><p:cNvSpPr><a:spLocks noGrp="1"/></p:cNvSpPr><p:nvPr><p:ph type="body" idx="1"/></p:nvPr></p:nvSpPr><p:spPr/><p:txBody><a:bodyPr/><a:lstStyle/>{}</p:txBody></p:sp></p:spTree></p:cSld><p:clrMapOvr><a:masterClrMapping/></p:clrMapOvr></p:notes>"#,
            paragraphs("a:", lines, "", "")
        )
        .into_bytes(),
    );
    work.set_override(&part, NOTES_TYPE)?;
    work.add_relationship(&part, &format!("{R}/notesMaster"), &master)?;
    work.add_relationship(&part, &format!("{R}/slide"), slide)?;
    work.add_relationship(slide, &format!("{R}/notesSlide"), &part)?;
    Ok(part)
}

pub(super) fn set_notes<R2: Read + Seek>(
    work: &mut Work<'_, R2>,
    anchor: &str,
    text: &str,
) -> Result<Outcome, EditError> {
    let deck = deck(work)?;
    let id = slide_id(anchor)?.to_string();
    let part = slide_part(work, &deck, &id)?;
    let Some(notes) = work.related(&part, "notesSlide")? else {
        if work.strict() {
            return fail("adding a notes page to a Strict presentation is not supported yet");
        }
        let lines: Vec<String> = text.split('\n').map(str::to_string).collect();
        create_notes(work, &deck.part, &part, &lines)?;
        return Ok(Outcome {
            summary: format!("slide:{id} speaker notes added"),
            expect: vec![expect_text(format!("slide:{id}/notes"), &lines)],
            created: Vec::new(),
        });
    };
    let bytes = work.get(&notes)?;
    let tree = Tree::parse(&bytes, &notes, work.limits())?;
    let body = tree.descendants(0, "sp").find(|shape| {
        let (_, _, kind, _) = shape_facts(&tree, *shape);
        kind.as_deref() == Some("body")
    });
    let Some(body) = body else {
        return fail(format!("slide:{id}'s notes page has no notes placeholder"));
    };
    let lines: Vec<String> = text.split('\n').map(str::to_string).collect();
    let mut splice = Splice::default();
    replace_shape_text(&mut splice, &bytes, &tree, body, &lines)?;
    work.put(&notes, splice.apply(&bytes, &notes)?);
    Ok(Outcome {
        summary: format!("slide:{id} speaker notes set"),
        expect: vec![expect_text(format!("slide:{id}/notes"), &lines)],
        created: Vec::new(),
    })
}

#[allow(clippy::too_many_arguments)]
pub(super) fn add_slide_from_layout<R2: Read + Seek>(
    work: &mut Work<'_, R2>,
    layout: &str,
    after: Option<&str>,
    placeholders: &BTreeMap<String, TextValue>,
    tables: &BTreeMap<String, Vec<Vec<CellValue>>>,
    charts: &BTreeMap<String, super::SlideChart>,
    images: &BTreeMap<String, super::SlideImage>,
    notes: Option<&str>,
) -> Result<Outcome, EditError> {
    if work.strict() {
        return fail("adding a slide to a Strict presentation is not supported yet");
    }
    let mut layouts = Vec::new();
    let mut chosen = None;
    for part in work.part_names() {
        if work.content_type(&part)?.as_deref() != Some(LAYOUT_TYPE) {
            continue;
        }
        let bytes = work.get(&part)?;
        let tree = Tree::parse(&bytes, &part, work.limits())?;
        let name = tree
            .descendants(0, "cSld")
            .next()
            .and_then(|node| tree.nodes[node].element.attr("name"))
            .unwrap_or_default()
            .to_string();
        if chosen.is_none() && name.trim().eq_ignore_ascii_case(layout.trim()) {
            chosen = Some((part.clone(), tree));
        }
        layouts.push(name);
    }
    let Some((layout_part, layout_tree)) = chosen else {
        layouts.sort();
        layouts.dedup();
        return fail(format!(
            "no layout {layout:?}; this deck's layouts: {}",
            layouts.join(", ")
        ));
    };

    let master_tree = match work.related(&layout_part, "slideMaster")? {
        Some(part) => {
            let bytes = work.get(&part)?;
            Some(Tree::parse(&bytes, &part, work.limits())?)
        }
        None => None,
    };

    let mut shapes = Vec::new();
    let mut keys = Vec::new();
    for shape in layout_tree.descendants(0, "sp") {
        let (_, name, kind, index) = shape_facts(&layout_tree, shape);
        let Some(kind) = kind else {
            continue;
        };
        if matches!(kind.as_str(), "dt" | "ftr" | "sldNum" | "hdr") {
            continue;
        }
        keys.push(match kind.as_str() {
            "title" | "ctrTitle" => "title".to_string(),
            "subTitle" => "subtitle".to_string(),
            other => match &index {
                Some(index) if other == "body" => format!("body (or idx:{index})"),
                _ => other.to_string(),
            },
        });
        // The slide's placeholder names the layout's exactly: a content
        // placeholder has no type (it means `obj`), a text one says `body`.
        let written_type = layout_tree
            .descendants(shape, "ph")
            .next()
            .and_then(|ph| layout_tree.nodes[ph].element.attr("type"))
            .map(str::to_string);
        let (mut offset, mut extent) = placeholder_bounds(&layout_tree, shape);
        if (offset.is_none() || extent.is_none()) && master_tree.is_some() {
            let master = master_tree.as_ref().ok_or_else(|| EditError {
                op: None,
                message: "the layout's master could not be read".into(),
            })?;
            if let Some(master_shape) = master.descendants(0, "sp").find(|candidate| {
                let (_, _, master_kind, master_index) = shape_facts(master, *candidate);
                master_kind
                    .as_deref()
                    .is_some_and(|master_kind| master_kind.eq_ignore_ascii_case(&kind))
                    && master_index.as_deref() == index.as_deref()
            }) {
                let (master_offset, master_extent) = placeholder_bounds(master, master_shape);
                offset = offset.or(master_offset);
                extent = extent.or(master_extent);
            }
        }
        shapes.push((name, kind, index, written_type, offset, extent));
    }
    for key in placeholders.keys() {
        if !shapes
            .iter()
            .any(|(_, kind, index, _, _, _)| placeholder_matches(key, Some(kind), index.as_deref()))
        {
            return fail(format!(
                "layout {layout:?} has no placeholder {key:?}; it has: {}",
                keys.join(", ")
            ));
        }
    }
    for key in tables.keys() {
        let Some((_, kind, index, _, offset, extent)) =
            shapes.iter().find(|(_, kind, index, _, _, _)| {
                placeholder_matches(key, Some(kind), index.as_deref())
            })
        else {
            return fail(format!(
                "layout {layout:?} has no table placeholder {key:?}; choose a body placeholder"
            ));
        };
        if !matches!(kind.to_ascii_lowercase().as_str(), "body" | "obj") {
            return fail(format!(
                "placeholder {key:?} is not a content placeholder; native tables go in a body placeholder"
            ));
        }
        if offset.is_none_or(|(x, y)| x < 0 || y < 0)
            || extent.is_none_or(|(width, height)| width <= 0 || height <= 0)
        {
            return fail(format!(
                "layout {layout:?} does not give table placeholder {key:?} usable bounds"
            ));
        }
        if placeholders
            .keys()
            .any(|placeholder| placeholder_matches(placeholder, Some(kind), index.as_deref()))
        {
            return fail(format!(
                "placeholder {key:?} cannot have both text and a table"
            ));
        }
        for (row_index, row) in tables[key].iter().enumerate() {
            if row.is_empty()
                || row.len() > 32
                || (row_index > 0 && row.len() != tables[key][0].len())
            {
                return fail(format!(
                    "table {key:?} must have 1–32 equal-width columns and no empty rows"
                ));
            }
        }
        let text_bytes: usize = tables[key]
            .iter()
            .flatten()
            .map(|cell| cell.as_text().len())
            .sum();
        if text_bytes > 128 * 1024 {
            return fail(format!(
                "table {key:?} is over the 128 KiB text limit; split it across slides"
            ));
        }
        if tables[key].is_empty() || tables[key].len() > 100 {
            return fail(format!("table {key:?} must have 1–100 rows"));
        }
    }
    for (key, chart) in charts {
        let Some((_, kind, index, _, offset, extent)) =
            shapes.iter().find(|(_, kind, index, _, _, _)| {
                placeholder_matches(key, Some(kind), index.as_deref())
            })
        else {
            return fail(format!(
                "layout {layout:?} has no chart placeholder {key:?}"
            ));
        };
        if !matches!(kind.to_ascii_lowercase().as_str(), "body" | "obj") {
            return fail(format!("placeholder {key:?} is not a content placeholder"));
        }
        if offset.is_none_or(|(x, y)| x < 0 || y < 0)
            || extent.is_none_or(|(width, height)| width <= 0 || height <= 0)
        {
            return fail(format!(
                "layout {layout:?} does not give chart placeholder {key:?} usable bounds"
            ));
        }
        if placeholders
            .keys()
            .chain(tables.keys())
            .any(|placeholder| placeholder_matches(placeholder, Some(kind), index.as_deref()))
        {
            return fail(format!(
                "placeholder {key:?} cannot hold text, a table and a chart together"
            ));
        }
        validate_chart(chart)?;
    }
    for (key, image) in images {
        let Some((_, kind, index, _, offset, extent)) =
            shapes.iter().find(|(_, kind, index, _, _, _)| {
                placeholder_matches(key, Some(kind), index.as_deref())
            })
        else {
            return fail(format!(
                "layout {layout:?} has no image placeholder {key:?}"
            ));
        };
        if !matches!(kind.to_ascii_lowercase().as_str(), "body" | "obj") {
            return fail(format!("placeholder {key:?} is not a content placeholder"));
        }
        if offset.is_none_or(|(x, y)| x < 0 || y < 0)
            || extent.is_none_or(|(width, height)| width <= 0 || height <= 0)
        {
            return fail(format!(
                "layout {layout:?} does not give image placeholder {key:?} usable bounds"
            ));
        }
        if placeholders
            .keys()
            .chain(tables.keys())
            .chain(charts.keys())
            .any(|placeholder| placeholder_matches(placeholder, Some(kind), index.as_deref()))
        {
            return fail(format!(
                "placeholder {key:?} cannot hold text, a table, a chart and an image together"
            ));
        }
        validate_image(image)?;
    }

    let deck = deck(work)?;
    let a = "a:";
    let mut used = std::collections::HashSet::new();
    let mut body = String::new();
    let mut used_tables = std::collections::HashSet::new();
    let mut used_charts = std::collections::HashSet::new();
    let mut used_images = std::collections::HashSet::new();
    let mut chart_expectations = Vec::new();
    let slide = free_part_name(work, "ppt/slides/slide", ".xml");
    for (position, (name, kind, index, written_type, offset, extent)) in shapes.iter().enumerate() {
        let table = tables.iter().find(|(key, _)| {
            placeholder_matches(key, Some(kind), index.as_deref())
                && used_tables.insert((*key).clone())
        });
        if let Some((_, rows)) = table {
            let Some((x, y)) = offset else {
                return fail("table placeholder has no position");
            };
            let Some((width, height)) = extent else {
                return fail("table placeholder has no size");
            };
            body.push_str(&table_frame(
                position + 2,
                name,
                written_type.as_deref(),
                index.as_deref(),
                *x,
                *y,
                *width,
                *height,
                rows,
            ));
            continue;
        }
        let chart = charts.iter().find(|(key, _)| {
            placeholder_matches(key, Some(kind), index.as_deref())
                && used_charts.insert((*key).clone())
        });
        if let Some((_, chart)) = chart {
            let Some((x, y)) = offset else {
                return fail("chart placeholder has no position");
            };
            let Some((width, height)) = extent else {
                return fail("chart placeholder has no size");
            };
            let (part, rows) = create_chart(work, &slide, chart)?;
            let relationship = work.add_relationship(&slide, &format!("{R}/chart"), &part)?;
            body.push_str(&chart_frame(
                position + 2,
                name,
                written_type.as_deref(),
                index.as_deref(),
                *x,
                *y,
                *width,
                *height,
                &relationship,
            ));
            chart_expectations.push((part, rows));
            continue;
        }
        let image = images.iter().find(|(key, _)| {
            placeholder_matches(key, Some(kind), index.as_deref())
                && used_images.insert((*key).clone())
        });
        if let Some((_, image)) = image {
            let Some((x, y)) = offset else {
                return fail("image placeholder has no position");
            };
            let Some((width, height)) = extent else {
                return fail("image placeholder has no size");
            };
            let (part, image_width, image_height) = create_image(work, image)?;
            let relationship = work.add_relationship(&slide, PNG_REL_TYPE, &part)?;
            let scale = ((*width as f64) / f64::from(image_width))
                .min((*height as f64) / f64::from(image_height));
            let draw_width = (f64::from(image_width) * scale).round() as i64;
            let draw_height = (f64::from(image_height) * scale).round() as i64;
            let draw_x = *x + (*width - draw_width) / 2;
            let draw_y = *y + (*height - draw_height) / 2;
            body.push_str(&picture_frame(
                position + 2,
                name,
                &image.alt_text,
                written_type.as_deref(),
                index.as_deref(),
                draw_x,
                draw_y,
                draw_width,
                draw_height,
                &relationship,
            ));
            continue;
        }
        let text = placeholders
            .iter()
            .find(|(key, _)| {
                placeholder_matches(key, Some(kind), index.as_deref())
                    && used.insert((*key).clone())
            })
            .map(|(_, text)| text.lines())
            .unwrap_or_default();
        let type_attr = written_type
            .as_ref()
            .map(|kind| format!(r#" type="{}""#, escape_attr(kind)))
            .unwrap_or_default();
        let index_attr = index
            .as_ref()
            .map(|index| format!(r#" idx="{}""#, escape_attr(index)))
            .unwrap_or_default();
        body.push_str(&format!(
            r#"<p:sp><p:nvSpPr><p:cNvPr id="{}" name="{}"/><p:cNvSpPr><a:spLocks noGrp="1"/></p:cNvSpPr><p:nvPr><p:ph{type_attr}{index_attr}/></p:nvPr></p:nvSpPr><p:spPr/><p:txBody><a:bodyPr/><a:lstStyle/>{}</p:txBody></p:sp>"#,
            position + 2,
            escape_attr(name),
            paragraphs(a, &text, "", "")
        ));
    }
    work.put(
        &slide,
        format!(
            r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<p:sld xmlns:a="{A}" xmlns:r="{R}" xmlns:p="{P}"><p:cSld><p:spTree><p:nvGrpSpPr><p:cNvPr id="1" name=""/><p:cNvGrpSpPr/><p:nvPr/></p:nvGrpSpPr><p:grpSpPr><a:xfrm><a:off x="0" y="0"/><a:ext cx="0" cy="0"/><a:chOff x="0" y="0"/><a:chExt cx="0" cy="0"/></a:xfrm></p:grpSpPr>{body}</p:spTree></p:cSld><p:clrMapOvr><a:masterClrMapping/></p:clrMapOvr></p:sld>"#
        )
        .into_bytes(),
    );
    work.set_override(&slide, SLIDE_TYPE)?;
    work.add_relationship(&slide, &format!("{R}/slideLayout"), &layout_part)?;
    let relationship = work.add_relationship(&deck.part, &format!("{R}/slide"), &slide)?;

    let used_ids = deck
        .tree
        .nodes
        .iter()
        .filter(|node| node.local() == "sldId")
        .filter_map(|node| {
            node.element
                .attr_unprefixed("id")
                .and_then(|id| id.parse::<u32>().ok())
        });
    let new_id = used_ids.max().map(|max| max + 1).unwrap_or(256).max(256);
    let (p, r) = (&deck.p, &deck.r);
    let entry = format!(r#"<{p}sldId id="{new_id}" {r}id="{relationship}"/>"#);
    let mut splice = Splice::default();
    let mut order: Vec<String> = deck
        .slides
        .iter()
        .map(|(id, _, _)| format!("slide:{id}"))
        .collect();
    match (deck.list, after) {
        (Some(_), Some(after)) => {
            let after_id = slide_id(after)?;
            let Some(position) = deck.slides.iter().position(|(id, _, _)| id == after_id) else {
                return fail(format!("no slide {after} to add after"));
            };
            splice.insert(deck.tree.nodes[deck.slides[position].2].span.end, entry);
            order.insert(position + 1, format!("slide:{new_id}"));
        }
        (Some(list), None) => {
            let node = &deck.tree.nodes[list];
            if node.is_empty_element() {
                splice.replace(
                    node.span.clone(),
                    format!("<{p}sldIdLst>{entry}</{p}sldIdLst>"),
                );
            } else {
                splice.insert(node.inner.end, entry);
            }
            order.push(format!("slide:{new_id}"));
        }
        (None, _) => {
            let before = deck
                .tree
                .children(0, "sldSz")
                .chain(deck.tree.children(0, "notesSz"))
                .map(|index| deck.tree.nodes[index].span.start)
                .min();
            let Some(before) = before else {
                return fail(
                    "the presentation has no slide list and no slide size to place one before",
                );
            };
            splice.insert(before, format!("<{p}sldIdLst>{entry}</{p}sldIdLst>"));
            order.push(format!("slide:{new_id}"));
        }
    }
    work.put(&deck.part, splice.apply(&deck.bytes, &deck.part)?);

    let mut expect = vec![Expect::SlideOrder(order)];
    for (part, rows) in chart_expectations {
        let file = part.rsplit('/').next().unwrap_or("chart");
        expect.push(Expect::Table {
            anchor: format!("slide:{new_id}/chart:{file}"),
            rows,
        });
    }
    if let Some(notes) = notes.filter(|notes| !notes.trim().is_empty()) {
        let lines: Vec<String> = notes.split('\n').map(str::to_string).collect();
        create_notes(work, &deck.part, &slide, &lines)?;
        expect.push(expect_text(format!("slide:{new_id}/notes"), &lines));
    }
    for (key, text) in placeholders {
        if let Some(first) = text
            .lines()
            .into_iter()
            .map(|line| line.trim().to_string())
            .find(|line| !line.is_empty())
        {
            if key.eq_ignore_ascii_case("title") {
                expect.push(Expect::UnitContains {
                    anchor: format!("slide:{new_id}"),
                    needles: vec![first],
                });
            } else {
                expect.push(Expect::AnyUnitContains {
                    prefix: format!("slide:{new_id}/shape:"),
                    needle: first,
                });
            }
        }
    }
    for (key, rows) in tables {
        let shape_index = shapes
            .iter()
            .position(|(_, kind, index, _, _, _)| {
                placeholder_matches(key, Some(kind), index.as_deref())
            })
            .unwrap_or(0);
        let table_anchor = format!("slide:{new_id}/shape:{}", shape_index + 2);
        let text_rows: Vec<Vec<String>> = rows
            .iter()
            .map(|row| row.iter().map(CellValue::as_text).collect())
            .collect();
        expect.push(Expect::Table {
            anchor: table_anchor,
            rows: text_rows,
        });
    }
    for (key, image) in images {
        let shape_index = shapes
            .iter()
            .position(|(_, kind, index, _, _, _)| {
                placeholder_matches(key, Some(kind), index.as_deref())
            })
            .unwrap_or(0);
        expect.push(Expect::UnitContains {
            anchor: format!("slide:{new_id}/shape:{}", shape_index + 2),
            needles: vec![image.alt_text.trim().to_string()],
        });
    }
    Ok(Outcome {
        summary: format!("slide:{new_id} added from layout {layout:?}"),
        expect,
        created: vec![format!("slide:{new_id}")],
    })
}

pub(super) fn delete_slide<R2: Read + Seek>(
    work: &mut Work<'_, R2>,
    anchor: &str,
) -> Result<Outcome, EditError> {
    let deck = deck(work)?;
    let id = slide_id(anchor)?.to_string();
    let part = slide_part(work, &deck, &id)?;
    let Some((_, relationship, _)) = deck
        .slides
        .iter()
        .find(|(slide, _, _)| *slide == id)
        .cloned()
    else {
        return fail(format!("no slide:{id}"));
    };
    if deck.slides.len() == 1 {
        return fail("this is the deck's only slide; add another before deleting it");
    }
    let notes = work.related(&part, "notesSlide")?;
    let own_rels = [
        rels_part_name(&deck.part),
        rels_part_name(&part),
        notes.as_deref().map(rels_part_name).unwrap_or_default(),
    ];
    for rels in work.part_names() {
        if !rels.to_ascii_lowercase().ends_with(".rels") || own_rels.contains(&rels) {
            continue;
        }
        let source = rels
            .rsplit_once("_rels/")
            .map(|(directory, file)| format!("{directory}{}", file.trim_end_matches(".rels")))
            .unwrap_or_default();
        if work
            .relationships(&source)?
            .iter()
            .any(|relationship| !relationship.external && relationship.target == part)
        {
            return fail(format!(
                "slide:{id} is referenced by {source} (a link or custom show); remove that reference first"
            ));
        }
    }
    let mut splice = Splice::default();
    for node in &deck.tree.nodes {
        if node.local() == "sldId" && node.element.attr_unprefixed("id") == Some(id.as_str()) {
            splice.replace(node.span.clone(), "");
        }
    }
    work.put(&deck.part, splice.apply(&deck.bytes, &deck.part)?);
    work.remove_relationship(&deck.part, &relationship)?;
    if let Some(notes) = notes {
        work.remove(&notes);
        work.remove(&rels_part_name(&notes));
        work.remove_override(&notes)?;
    }
    work.remove(&part);
    work.remove(&rels_part_name(&part));
    work.remove_override(&part)?;
    Ok(Outcome {
        summary: format!("slide:{id} deleted"),
        expect: vec![
            Expect::Absent {
                anchor: format!("slide:{id}"),
            },
            Expect::SlideOrder(
                deck.slides
                    .iter()
                    .filter(|(slide, _, _)| *slide != id)
                    .map(|(slide, _, _)| format!("slide:{slide}"))
                    .collect(),
            ),
        ],
        created: Vec::new(),
    })
}

pub(super) fn move_slide<R2: Read + Seek>(
    work: &mut Work<'_, R2>,
    anchor: &str,
    after: Option<&str>,
) -> Result<Outcome, EditError> {
    let deck = deck(work)?;
    let id = slide_id(anchor)?.to_string();
    let Some(from) = deck.slides.iter().position(|(slide, _, _)| *slide == id) else {
        return fail(format!("no slide:{id}"));
    };
    let Some(list) = deck.list else {
        return fail("the presentation has no slide list");
    };
    let moving = &deck.tree.nodes[deck.slides[from].2];
    let entry = String::from_utf8_lossy(&deck.bytes[moving.span.clone()]).into_owned();
    let mut order: Vec<String> = deck
        .slides
        .iter()
        .map(|(id, _, _)| format!("slide:{id}"))
        .collect();
    let moved = order.remove(from);
    let mut splice = Splice::default();
    splice.replace(moving.span.clone(), "");
    match after {
        Some(after) => {
            let after_id = slide_id(after)?;
            if after_id == id {
                return fail("a slide cannot move after itself");
            }
            let Some(target) = deck
                .slides
                .iter()
                .position(|(slide, _, _)| slide == after_id)
            else {
                return fail(format!("no slide {after} to move after"));
            };
            splice.insert(deck.tree.nodes[deck.slides[target].2].span.end, entry);
            let position = order
                .iter()
                .position(|slide| *slide == format!("slide:{after_id}"))
                .unwrap_or(0);
            order.insert(position + 1, moved);
        }
        None => {
            splice.insert(deck.tree.nodes[list].inner.start, entry);
            order.insert(0, moved);
        }
    }
    work.put(&deck.part, splice.apply(&deck.bytes, &deck.part)?);
    Ok(Outcome {
        summary: format!(
            "slide:{id} moved {}",
            after
                .map(|after| format!("after {after}"))
                .unwrap_or_else(|| "to the start".into())
        ),
        expect: vec![Expect::SlideOrder(order)],
        created: Vec::new(),
    })
}
