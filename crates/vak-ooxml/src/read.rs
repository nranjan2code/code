//! L2 read projections with anchors and O6 labels.
//!
//! A projection is data for a reader, never instructions: hidden runs,
//! tracked deletions, comments, hidden slides and sheets, off-slide shapes
//! and speaker notes are all kept, and each is labelled for what it is so a
//! prompt injection hidden in a document is visible as hidden content.

use std::collections::{BTreeMap, HashMap};
use std::io::{Read, Seek};
use std::ops::Range;

use serde::Serialize;

use crate::package::{Inspection, Package, Vocabulary};
use crate::xml::{self, Element, XmlEvent};
use crate::{Error, Limits};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum UnitKind {
    Heading,
    Paragraph,
    TableRow,
    Comment,
    SheetRow,
    DefinedName,
    Slide,
    Shape,
    Notes,
    Page,
    Image,
}

/// One addressable piece of a document. `anchor` is what an op or a
/// citation names (docs/design/72, "Anchors").
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Unit {
    pub anchor: String,
    pub kind: UnitKind,
    /// Heading level; 0 for everything that is not a heading.
    pub level: u8,
    pub text: String,
    pub labels: Vec<String>,
    /// For a sheet row, its cells as (address, shown value); empty
    /// otherwise.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub cells: Vec<(String, String)>,
    /// For a Word table row, each cell's paragraphs as (anchor, text), so a
    /// cell can be named and changed like any paragraph; empty otherwise.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub row_cells: Vec<Vec<(String, String)>>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Section {
    pub anchor: String,
    pub title: String,
    pub level: u8,
    pub units: Range<usize>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Table {
    pub anchor: String,
    pub title: String,
    /// First row is the header row.
    pub rows: Vec<Vec<String>>,
    pub labels: Vec<String>,
    /// Used columns beyond [`MAX_TABLE_COLUMNS`] that the grid leaves out;
    /// the anchored lines still carry every cell.
    pub omitted_columns: usize,
}

/// A small, presentation-only subset of an XLSX cell style. It carries no
/// cell content, and the Canvas keeps it separate from the extraction view.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
pub struct CellStyle {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub fill_color: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub font_color: Option<String>,
    pub bold: bool,
    pub italic: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub number_format: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub horizontal_alignment: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub vertical_alignment: Option<String>,
    #[serde(skip_serializing_if = "is_false")]
    pub wrap_text: bool,
}

fn is_false(value: &bool) -> bool {
    !value
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct TableStyleRange {
    pub sheet_anchor: String,
    pub range: String,
    pub style_name: Option<String>,
    pub show_row_stripes: bool,
    pub show_column_stripes: bool,
}

/// Worksheet sizing hints, kept in the visual projection and out of RAG text.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
pub struct SheetGeometry {
    /// Default pixel width keyed by sheet name.
    pub default_column_widths: HashMap<String, u32>,
    /// Default pixel height keyed by sheet name.
    pub default_row_heights: HashMap<String, u32>,
    /// Pixel widths keyed as `Sheet!A`.
    pub column_widths: HashMap<String, u32>,
    /// Pixel heights keyed as `Sheet!1`.
    pub row_heights: HashMap<String, u32>,
    /// Native worksheet merged-cell ranges keyed by sheet name.
    pub merged_ranges: Vec<MergedCellRange>,
}

/// A worksheet's merged range, kept as visual metadata separate from cells.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct MergedCellRange {
    pub sheet_anchor: String,
    pub range: String,
}

/// A bounded image preview for the local Office Canvas. This stays in the
/// worker-backed projection; `doc_read` continues to expose alt text only.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ImagePreview {
    pub alt_text: String,
    pub object_id: String,
    pub mime_type: String,
    pub data_url: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cell: Option<String>,
    /// Offset in pixels from the top-left worksheet cell marker.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub offset_x_px: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub offset_y_px: Option<u32>,
    /// Bottom-right cell boundary for two-cell anchored worksheet images.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub end_cell: Option<String>,
    /// Offset in pixels from the bottom-right worksheet cell marker.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub end_offset_x_px: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub end_offset_y_px: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub width_px: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub height_px: Option<u32>,
}

/// Widest grid a table projection builds. A sheet can use 16,384 columns,
/// and a dense rows-by-columns grid of that width is a memory bomb.
pub const MAX_TABLE_COLUMNS: usize = 64;

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Document {
    pub inspection: Inspection,
    pub title: Option<String>,
    pub stats: Vec<(String, usize)>,
    pub units: Vec<Unit>,
    pub sections: Vec<Section>,
    pub tables: Vec<Table>,
    /// Visual-only, package-scoped media identity keyed by public image anchor.
    pub image_object_ids: HashMap<String, String>,
    #[serde(skip)]
    pub cell_styles: HashMap<String, CellStyle>,
    #[serde(skip)]
    pub table_styles: Vec<TableStyleRange>,
    #[serde(skip)]
    pub sheet_geometry: SheetGeometry,
    #[serde(skip)]
    chart_placements: HashMap<String, String>,
    #[serde(skip)]
    chart_ends: HashMap<String, String>,
    #[serde(skip)]
    chart_sizes: HashMap<String, (u32, u32)>,
    /// Parts of the vocabulary this reader does not project yet. Stated so
    /// a reader never mistakes an omission for absence.
    pub not_read: Vec<&'static str>,
}

const MAX_PREVIEW_IMAGE_BYTES_PER_IMAGE: usize = 1024 * 1024;
const MAX_PREVIEW_IMAGE_BYTES_TOTAL: usize = 8 * 1024 * 1024;

/// Read bounded PNG/JPEG previews referenced by a package's drawing parts.
/// Parsing and decompression happen in the document worker, never in the UI
/// or server process. Unsupported media remains available through its alt text.
pub fn image_previews<R: Read + Seek>(
    reader: R,
    limits: Limits,
) -> Result<Vec<ImagePreview>, Error> {
    use base64::Engine;
    let mut package = Package::open(reader, limits)?;
    let parts: Vec<String> = package.part_names().map(str::to_string).collect();
    let mut previews = Vec::new();
    let mut total_bytes = 0usize;
    for part in parts {
        if !package
            .content_type(&part)
            .is_some_and(|kind| kind.ends_with("+xml"))
        {
            continue;
        }
        let relationships: Vec<_> = package
            .relationships(&part)?
            .iter()
            .filter(|relationship| !relationship.external && relationship.short_kind() == "image")
            .map(|relationship| (relationship.id.clone(), relationship.target.clone()))
            .collect();
        if relationships.is_empty() {
            continue;
        }
        let rel_targets: HashMap<String, String> = relationships.into_iter().collect();
        let bytes = package.read_part(&part)?;
        for reference in drawing_references(&bytes, &part, &limits)? {
            let Some(target) = rel_targets.get(&reference.relationship) else {
                continue;
            };
            let Some(mime) = package.content_type(target) else {
                continue;
            };
            if !matches!(mime.as_str(), "image/png" | "image/jpeg") {
                continue;
            }
            let image = package.read_part(target)?;
            if image.is_empty()
                || image.len() > MAX_PREVIEW_IMAGE_BYTES_PER_IMAGE
                || total_bytes.saturating_add(image.len()) > MAX_PREVIEW_IMAGE_BYTES_TOTAL
            {
                continue;
            }
            let (signature, _) = if mime == "image/png" {
                (image.starts_with(b"\x89PNG\r\n\x1a\n"), ())
            } else {
                (image.starts_with(&[0xff, 0xd8, 0xff]), ())
            };
            if !signature {
                continue;
            }
            total_bytes += image.len();
            previews.push(ImagePreview {
                alt_text: reference.alt_text,
                object_id: drawing_object_id(&part, &reference.id),
                mime_type: mime.clone(),
                data_url: format!(
                    "data:{mime};base64,{}",
                    base64::engine::general_purpose::STANDARD.encode(image)
                ),
                cell: reference.cell,
                offset_x_px: reference.offset_x_px,
                offset_y_px: reference.offset_y_px,
                end_cell: reference.end_cell,
                end_offset_x_px: reference.end_offset_x_px,
                end_offset_y_px: reference.end_offset_y_px,
                width_px: reference.width_px,
                height_px: reference.height_px,
            });
        }
    }
    Ok(previews)
}

#[derive(Debug, Clone)]
struct DrawingReference {
    relationship: String,
    id: String,
    alt_text: String,
    cell: Option<String>,
    offset_x_px: Option<u32>,
    offset_y_px: Option<u32>,
    end_cell: Option<String>,
    end_offset_x_px: Option<u32>,
    end_offset_y_px: Option<u32>,
    width_px: Option<u32>,
    height_px: Option<u32>,
}

/// The drawing reference's relationship, alt text and optional worksheet
/// cell. The XML reader is bounded by the same depth and attribute limits as
/// the rest of the package projection.
fn drawing_references(
    bytes: &[u8],
    part: &str,
    limits: &Limits,
) -> Result<Vec<DrawingReference>, Error> {
    let mut references = Vec::new();
    let mut alt_text = String::new();
    let mut object_id = String::new();
    let mut cell = None;
    let mut end_cell = None;
    let mut offset_x_px = None;
    let mut offset_y_px = None;
    let mut end_offset_x_px = None;
    let mut end_offset_y_px = None;
    let mut width_px = None;
    let mut height_px = None;
    let mut in_from = false;
    let mut in_to = false;
    let mut coordinate: Option<(&'static str, String)> = None;
    let mut column: Option<usize> = None;
    let mut row: Option<usize> = None;
    xml::walk(bytes, part, limits, |event| {
        match event {
            XmlEvent::Open(element) => match element.local() {
                "oneCellAnchor" | "twoCellAnchor" => {
                    cell = None;
                    end_cell = None;
                    offset_x_px = None;
                    offset_y_px = None;
                    end_offset_x_px = None;
                    end_offset_y_px = None;
                    width_px = None;
                    height_px = None;
                    column = None;
                    row = None;
                }
                "from" => {
                    in_from = true;
                    in_to = false;
                    column = None;
                    row = None;
                }
                "to" => {
                    in_to = true;
                    in_from = false;
                    column = None;
                    row = None;
                }
                "col" | "colOff" if in_from || in_to => {
                    coordinate = Some((
                        if element.local() == "col" {
                            "col"
                        } else {
                            "colOff"
                        },
                        String::new(),
                    ))
                }
                "row" | "rowOff" if in_from || in_to => {
                    coordinate = Some((
                        if element.local() == "row" {
                            "row"
                        } else {
                            "rowOff"
                        },
                        String::new(),
                    ))
                }
                "ext" => {
                    width_px = element.attr("cx").and_then(emu_to_pixels);
                    height_px = element.attr("cy").and_then(emu_to_pixels);
                }
                "docPr" | "cNvPr" => {
                    object_id = element.attr("id").unwrap_or_default().to_string();
                    alt_text = element
                        .attr("descr")
                        .map(str::trim)
                        .unwrap_or_default()
                        .to_string();
                }
                "blip" | "chart" => {
                    if let Some(relationship) = element
                        .attr_prefixed("embed")
                        .or_else(|| element.attr_prefixed("id"))
                        && (element.local() == "chart" || !alt_text.trim().is_empty())
                    {
                        references.push(DrawingReference {
                            relationship: relationship.to_string(),
                            id: object_id.clone(),
                            alt_text: alt_text.trim().to_string(),
                            cell: cell.clone(),
                            offset_x_px,
                            offset_y_px,
                            end_cell: end_cell.clone(),
                            end_offset_x_px,
                            end_offset_y_px,
                            width_px,
                            height_px,
                        });
                        alt_text.clear();
                    }
                }
                _ => {}
            },
            XmlEvent::Text(text) => {
                if let Some((_, value)) = &mut coordinate {
                    value.push_str(&text);
                }
            }
            XmlEvent::Close(name) => match xml::local_name(&name) {
                "col" if coordinate.as_ref().is_some_and(|(kind, _)| *kind == "col") => {
                    column = coordinate.take().and_then(|(_, text)| text.parse().ok());
                }
                "colOff"
                    if coordinate
                        .as_ref()
                        .is_some_and(|(kind, _)| *kind == "colOff") =>
                {
                    let offset = coordinate.take().map(|(_, text)| text);
                    let pixels = offset.as_deref().and_then(emu_to_pixels);
                    if in_from {
                        offset_x_px = pixels;
                    } else if in_to {
                        end_offset_x_px = pixels;
                    }
                }
                "row" if coordinate.as_ref().is_some_and(|(kind, _)| *kind == "row") => {
                    row = coordinate.take().and_then(|(_, text)| text.parse().ok());
                }
                "rowOff"
                    if coordinate
                        .as_ref()
                        .is_some_and(|(kind, _)| *kind == "rowOff") =>
                {
                    let offset = coordinate.take().map(|(_, text)| text);
                    let pixels = offset.as_deref().and_then(emu_to_pixels);
                    if in_from {
                        offset_y_px = pixels;
                    } else if in_to {
                        end_offset_y_px = pixels;
                    }
                }
                "from" | "to" => {
                    let address = || {
                        Some(format!(
                            "{}{}",
                            column_name(u32::try_from(column? + 1).ok()?),
                            row?.saturating_add(1)
                        ))
                    };
                    if xml::local_name(&name) == "from" {
                        cell = address();
                    } else {
                        // The OOXML `to` coordinate is an exclusive cell
                        // boundary; retaining that address lets the Canvas
                        // size the object from the same grid it uses for its
                        // top-left anchor.
                        end_cell = address();
                    }
                    in_from = false;
                    in_to = false;
                }
                "oneCellAnchor" | "twoCellAnchor" => {
                    in_from = false;
                    in_to = false;
                    coordinate = None;
                }
                _ => {}
            },
        }
        Ok(())
    })?;
    Ok(references)
}

fn emu_to_pixels(value: &str) -> Option<u32> {
    let emu = value.parse::<u64>().ok()?;
    u32::try_from(emu.saturating_add(4_762) / 9_525).ok()
}

/// Drawing object IDs are unique only within their drawing part. Include its
/// stable part name so two worksheets can both contain object ID `1`.
fn drawing_object_id(part: &str, id: &str) -> String {
    let name = part.rsplit('/').next().unwrap_or(part);
    format!("{name}#{id}")
}

/// Opens and projects one package.
pub fn read<R: Read + Seek>(reader: R, limits: Limits) -> Result<Document, Error> {
    let mut package = Package::open(reader, limits)?;
    project(&mut package)
}

pub fn project<R: Read + Seek>(package: &mut Package<R>) -> Result<Document, Error> {
    package.check_main_root()?;
    let inspection = package.inspect()?;
    let title = core_title(package)?;
    let mut document = Document {
        inspection,
        title,
        stats: Vec::new(),
        units: Vec::new(),
        sections: Vec::new(),
        tables: Vec::new(),
        image_object_ids: HashMap::new(),
        cell_styles: HashMap::new(),
        table_styles: Vec::new(),
        sheet_geometry: SheetGeometry::default(),
        chart_placements: HashMap::new(),
        chart_ends: HashMap::new(),
        chart_sizes: HashMap::new(),
        not_read: Vec::new(),
    };
    match package.format().vocabulary {
        Vocabulary::Word => word(package, &mut document)?,
        Vocabulary::Excel => excel(package, &mut document)?,
        Vocabulary::PowerPoint => powerpoint(package, &mut document)?,
        Vocabulary::Visio => visio(package, &mut document)?,
    }
    let mut chart_count = 0usize;
    if package.format().vocabulary == Vocabulary::Excel {
        let chart_parts: Vec<String> = package
            .part_names()
            .filter(|part| {
                package
                    .content_type(part)
                    .is_some_and(|kind| kind.ends_with("drawingml.chart+xml"))
            })
            .map(str::to_string)
            .collect();
        for part in chart_parts {
            let bytes = package.read_part(&part)?;
            if let Some(mut table) = chart_table(&bytes, &part, package.limits())? {
                if let Some(placement) = document.chart_placements.get(&part) {
                    table.labels.push(format!("chart position: {placement}"));
                }
                if let Some(end) = document.chart_ends.get(&part) {
                    table.labels.push(format!("chart end cell: {end}"));
                }
                if let Some((width, height)) = document.chart_sizes.get(&part) {
                    table.labels.push(format!("chart size: {width}x{height}px"));
                }
                // Cached chart values are part of the workbook's readable
                // content as well as its package metadata. Project them as
                // anchored rows so doc_read, citations and RAG can retrieve
                // the same categories and values that drive the chart.
                for (row_index, row) in table.rows.iter().enumerate() {
                    let row_cells = row
                        .iter()
                        .map(|text| vec![(String::new(), text.clone())])
                        .collect();
                    document.units.push(Unit {
                        anchor: format!("{}/r{}", table.anchor, row_index + 1),
                        kind: UnitKind::TableRow,
                        level: 0,
                        text: row.join(" | "),
                        labels: table.labels.clone(),
                        cells: Vec::new(),
                        row_cells,
                    });
                }
                document.tables.push(table);
                chart_count += 1;
            }
        }
    }
    if chart_count > 0 {
        document.stats.push(("charts".into(), chart_count));
        for item in &mut document.not_read {
            *item = match (package.format().vocabulary, *item) {
                (Vocabulary::Word, "images and charts") => "images",
                (Vocabulary::Excel, "charts, pivot tables and cell comments") => {
                    "pivot tables and cell comments"
                }
                (Vocabulary::PowerPoint, "chart data and SmartArt text") => "SmartArt text",
                (_, other) => other,
            };
        }
    }
    if package.format().vocabulary == Vocabulary::PowerPoint {
        chart_count = document
            .tables
            .iter()
            .filter(|table| {
                table
                    .labels
                    .iter()
                    .any(|label| label == "chart data; cached values")
            })
            .count();
        if chart_count > 0 {
            document.stats.push(("charts".into(), chart_count));
            document
                .not_read
                .retain(|item| *item != "chart data and SmartArt text");
            document.not_read.push("SmartArt text");
        }
    }
    Ok(document)
}

#[derive(Default)]
struct ChartSeries {
    name: String,
    categories: BTreeMap<usize, String>,
    values: BTreeMap<usize, String>,
    chart_type: String,
    next_category: usize,
    next_value: usize,
    cache_truncated: bool,
}

const MAX_CHART_POINTS: usize = 10_000;

/// Projects cached chart values into a table. Keeping the category and
/// series labels beside the numbers makes chart content available to search
/// and RAG without interpreting a rendered image.
fn chart_table(bytes: &[u8], part: &str, limits: &Limits) -> Result<Option<Table>, Error> {
    let mut stack: Vec<String> = Vec::new();
    let mut chart_series = Vec::new();
    let mut current: Option<ChartSeries> = None;
    let mut field: Option<&'static str> = None;
    let mut in_value = false;
    let mut value_text = String::new();
    let mut point_index: Option<usize> = None;
    let mut title_depth = None;
    let mut title = String::new();
    let mut chart_type = String::new();
    xml::walk(bytes, part, limits, |event| {
        match event {
            XmlEvent::Open(element) => {
                let name = element.local().to_string();
                if name == "barChart" {
                    chart_type = "column".into();
                } else if name == "lineChart" {
                    chart_type = "line".into();
                } else if name == "pieChart" {
                    chart_type = "pie".into();
                } else if name == "areaChart" {
                    chart_type = "area".into();
                } else if name == "doughnutChart" {
                    chart_type = "doughnut".into();
                } else if name == "scatterChart" || name == "bubbleChart" {
                    chart_type = "scatter".into();
                } else if name == "barDir" {
                    if element.attr("val") == Some("bar") {
                        chart_type = "bar".into();
                    } else if element.attr("val") == Some("col") {
                        chart_type = "column".into();
                    }
                } else if name == "ser" {
                    if let Some(previous) = current.take() {
                        chart_series.push(previous);
                    }
                    current = Some(ChartSeries {
                        chart_type: chart_type.clone(),
                        ..ChartSeries::default()
                    });
                } else if name == "title" && title_depth.is_none() {
                    title_depth = Some(stack.len());
                } else if name == "tx" {
                    field = Some("name");
                } else if name == "cat" || name == "xVal" {
                    field = Some("category");
                } else if name == "val" || name == "yVal" {
                    field = Some("value");
                } else if name == "pt" {
                    point_index = element.attr("idx").and_then(|value| value.parse().ok());
                } else if name == "v" {
                    in_value = true;
                    value_text.clear();
                }
                stack.push(name);
            }
            XmlEvent::Text(text) if in_value => {
                // Entity references and CDATA split one <v> into multiple
                // text events. Commit the whole value at its closing tag.
                value_text.push_str(&text);
            }
            XmlEvent::Text(text)
                if stack.last().is_some_and(|name| name == "t") && title_depth.is_some() =>
            {
                title.push_str(&text);
            }
            XmlEvent::Close(name) => {
                let local = xml::local_name(&name);
                if local == "v" {
                    in_value = false;
                    if let Some(series) = current.as_mut() {
                        match field {
                            Some("name") if series.name.is_empty() => {
                                series.name = std::mem::take(&mut value_text);
                            }
                            Some("category") => {
                                let index = point_index.unwrap_or(series.next_category);
                                if index < MAX_CHART_POINTS {
                                    series
                                        .categories
                                        .insert(index, std::mem::take(&mut value_text));
                                    series.next_category = index.saturating_add(1);
                                } else {
                                    series.cache_truncated = true;
                                }
                            }
                            Some("value") => {
                                let index = point_index.unwrap_or(series.next_value);
                                if index < MAX_CHART_POINTS {
                                    series.values.insert(index, std::mem::take(&mut value_text));
                                    series.next_value = index.saturating_add(1);
                                } else {
                                    series.cache_truncated = true;
                                }
                            }
                            _ => {}
                        }
                    }
                }
                if local == "pt" {
                    point_index = None;
                }
                if matches!(local, "tx" | "cat" | "val" | "xVal" | "yVal") {
                    field = None;
                }
                if local == "ser"
                    && let Some(series) = current.take()
                {
                    chart_series.push(series);
                }
                if local == "title" {
                    title_depth = None;
                }
                stack.pop();
            }
            _ => {}
        }
        Ok(())
    })?;
    if let Some(series) = current {
        chart_series.push(series);
    }
    let series: Vec<ChartSeries> = chart_series
        .into_iter()
        .filter(|series| !series.values.is_empty())
        .collect();
    if series.is_empty() {
        return Ok(None);
    }
    let width = series
        .iter()
        .map(|series| {
            series
                .values
                .keys()
                .chain(series.categories.keys())
                .max()
                .map_or(0, |index| index.saturating_add(1))
        })
        .max()
        .unwrap_or(0);
    let mut rows = Vec::with_capacity(width + 1);
    rows.push(
        std::iter::once("Category".to_string())
            .chain(series.iter().enumerate().map(|(index, series)| {
                if series.name.is_empty() {
                    format!("Series {}", index + 1)
                } else {
                    series.name.clone()
                }
            }))
            .collect(),
    );
    for row in 0..width {
        let category = series
            .iter()
            .find_map(|series| series.categories.get(&row))
            .cloned()
            .unwrap_or_else(|| (row + 1).to_string());
        rows.push(
            std::iter::once(category)
                .chain(
                    series
                        .iter()
                        .map(|series| series.values.get(&row).cloned().unwrap_or_default()),
                )
                .collect(),
        );
    }
    let file = part.rsplit('/').next().unwrap_or("chart");
    let chart_title = if title.trim().is_empty() {
        format!("Chart {file}")
    } else {
        title.trim().to_string()
    };
    let mut labels = vec![
        "chart data; cached values".into(),
        format!("chart title: {chart_title}"),
    ];
    if !chart_type.is_empty() {
        labels.push(format!("chart type: {chart_type}"));
    }
    if series.iter().any(|series| series.cache_truncated) {
        labels.push(format!(
            "chart cache truncated after {MAX_CHART_POINTS} points"
        ));
    }
    for (index, series) in series.iter().enumerate() {
        labels.push(format!(
            "chart series type {}: {}",
            index,
            if series.chart_type.is_empty() {
                "column"
            } else {
                &series.chart_type
            }
        ));
    }
    Ok(Some(Table {
        anchor: format!("chart:{file}"),
        title: chart_title.clone(),
        rows,
        labels,
        omitted_columns: 0,
    }))
}

/// Longest unit text one line carries. A longer unit continues on further
/// lines that name the same anchor and their part, so no text is dropped
/// and no single line can outgrow a page.
pub const MAX_LINE_CHARS: usize = 16_000;

impl Document {
    /// The document as anchored lines: `[anchor] text  ⟨labels⟩`.
    pub fn lines(&self) -> Vec<String> {
        self.units.iter().flat_map(render_unit).collect()
    }

    /// Anchored lines for a range of units.
    pub fn lines_of(&self, units: Range<usize>) -> Vec<String> {
        self.units[units].iter().flat_map(render_unit).collect()
    }

    /// A section by anchor, exact title, or title fragment, in that order.
    pub fn section(&self, wanted: &str) -> Option<&Section> {
        let wanted = wanted.trim().trim_start_matches('#').trim().to_lowercase();
        self.sections
            .iter()
            .find(|section| section.anchor.to_lowercase() == wanted)
            .or_else(|| {
                self.sections
                    .iter()
                    .find(|section| section.title.trim().to_lowercase() == wanted)
            })
            .or_else(|| {
                self.sections
                    .iter()
                    .find(|section| section.title.to_lowercase().contains(&wanted))
            })
    }

    /// A table by anchor, exact title or title fragment; the first table
    /// when nothing is named.
    pub fn table(&self, wanted: Option<&str>) -> Option<&Table> {
        match wanted.map(|value| value.trim().to_lowercase()) {
            None => self.tables.first(),
            Some(wanted) => self
                .tables
                .iter()
                .find(|table| {
                    table.anchor.to_lowercase() == wanted || table.title.to_lowercase() == wanted
                })
                .or_else(|| {
                    self.tables
                        .iter()
                        .find(|table| table.title.to_lowercase().contains(&wanted))
                }),
        }
    }

    pub fn outline(&self) -> Vec<String> {
        self.sections
            .iter()
            .map(|section| {
                let indent = "  ".repeat(usize::from(section.level.saturating_sub(1)));
                format!("{indent}- [{}] {}", section.anchor, section.title)
            })
            .collect()
    }
}

/// Separates the paragraphs of one shape's text in a slide unit, so a
/// bullet boundary is never confused with a slash in the text itself.
pub const PARAGRAPH_BREAK: &str = " ¶ ";

/// A unit's text as a line shows it: a Word table row names each cell's
/// paragraphs by anchor, so a cell can be cited and changed.
fn display_text(unit: &Unit) -> String {
    if unit.row_cells.is_empty() {
        return unit.text.clone();
    }
    unit.row_cells
        .iter()
        .map(|paragraphs| {
            paragraphs
                .iter()
                .map(|(anchor, text)| {
                    if text.is_empty() {
                        format!("[{anchor}]")
                    } else {
                        format!("[{anchor}] {text}")
                    }
                })
                .collect::<Vec<_>>()
                .join(" ")
        })
        .collect::<Vec<_>>()
        .join(" | ")
}

fn render_unit(unit: &Unit) -> Vec<String> {
    let text = display_text(unit);
    let characters: Vec<char> = text.chars().collect();
    let chunks: Vec<String> = if characters.len() <= MAX_LINE_CHARS {
        vec![text]
    } else {
        characters
            .chunks(MAX_LINE_CHARS)
            .map(|chunk| chunk.iter().collect())
            .collect()
    };
    let parts = chunks.len();
    chunks
        .into_iter()
        .enumerate()
        .map(|(index, text)| {
            let mut line = if parts == 1 {
                format!("[{}] ", unit.anchor)
            } else {
                format!("[{} ⟨part {} of {parts}⟩] ", unit.anchor, index + 1)
            };
            if unit.kind == UnitKind::Heading && index == 0 {
                line.push_str(&"#".repeat(usize::from(unit.level.clamp(1, 6))));
                line.push(' ');
            }
            line.push_str(&text);
            if !unit.labels.is_empty() {
                line.push_str("  ⟨");
                line.push_str(&unit.labels.join("; "));
                line.push('⟩');
            }
            line
        })
        .collect()
}

fn core_title<R: Read + Seek>(package: &mut Package<R>) -> Result<Option<String>, Error> {
    let Some(part) = package
        .related_part("", "core-properties")?
        .filter(|part| package.has_part(part))
    else {
        return Ok(None);
    };
    let bytes = package.read_part(&part)?;
    let mut title = String::new();
    let mut inside = false;
    xml::walk(&bytes, &part, package.limits(), |event| {
        match event {
            XmlEvent::Open(element) if element.local() == "title" => inside = true,
            XmlEvent::Close(name) if xml::local_name(&name) == "title" => inside = false,
            XmlEvent::Text(text) if inside => title.push_str(&text),
            _ => {}
        }
        Ok(())
    })?;
    let title = title.trim().to_string();
    Ok((!title.is_empty()).then_some(title))
}

fn is_on(element: &Element) -> bool {
    !matches!(element.attr("val"), Some("0" | "false" | "off"))
}

fn words(text: &str) -> usize {
    text.split_whitespace().count()
}

// ---- Word ------------------------------------------------------------------

#[derive(Default)]
struct Paragraph {
    anchor: String,
    style: Option<String>,
    outline: Option<u8>,
    text: String,
    /// The text a person reading the document sees: no reader markers, no
    /// deleted or hidden text. Words are counted from it.
    visible: String,
    labels: Vec<String>,
    comments: Vec<String>,
    in_table: bool,
    text_box: bool,
}

#[derive(Default)]
struct Run {
    hidden: bool,
    white: bool,
}

#[derive(Default)]
struct WordTotals {
    hidden_runs: usize,
    tracked: usize,
}

fn word<R: Read + Seek>(package: &mut Package<R>, document: &mut Document) -> Result<(), Error> {
    let main = package.main_part().to_string();
    let styles = match package.related_part(&main, "styles")? {
        Some(part) => {
            let bytes = package.read_part(&part)?;
            heading_styles(&bytes, &part, package.limits())?
        }
        None => HashMap::new(),
    };
    let comments = match package.related_part(&main, "comments")? {
        Some(part) => {
            let bytes = package.read_part(&part)?;
            word_comments(&bytes, &part, package.limits())?
        }
        None => HashMap::new(),
    };
    let bytes = package.read_part(&main)?;
    let limits = *package.limits();

    let mut stack: Vec<Paragraph> = Vec::new();
    let mut run = Run::default();
    let mut in_run = false;
    let mut in_run_properties = false;
    let mut revision: Vec<(bool, String)> = Vec::new();
    let mut in_text = false;
    let mut in_instruction = false;
    // One entry per open complex field: its instruction text and whether
    // it has been evaluated (at `separate`, or at `end` without one).
    let mut fields: Vec<(String, bool)> = Vec::new();
    // Inside `w:rPrChange` and friends: formatting that was, not that is.
    let mut change_depth = 0usize;
    let mut table_depth = 0usize;
    let mut table_rows: Vec<Vec<String>> = Vec::new();
    let mut row: Vec<String> = Vec::new();
    let mut cell = String::new();
    // The same rows, each cell as its paragraphs with their anchors.
    let mut table_row_cells: Vec<Vec<Vec<(String, String)>>> = Vec::new();
    let mut row_cells: Vec<Vec<(String, String)>> = Vec::new();
    let mut cell_paragraphs: Vec<(String, String)> = Vec::new();
    let mut visible_words = 0usize;
    let mut paragraph_ordinal = 0usize;
    let mut table_ordinal = 0usize;
    let mut image_ordinal = 0usize;
    let mut units: Vec<Unit> = Vec::new();
    let mut tables: Vec<Table> = Vec::new();
    let mut totals = WordTotals::default();
    let mut risky_fields = 0usize;
    let mut text_box_depth = 0usize;

    xml::walk(&bytes, &main, &limits, |event| {
        if let XmlEvent::Open(element) = &event
            && is_property_change(element.local())
        {
            change_depth += 1;
            return Ok(());
        }
        if let XmlEvent::Close(name) = &event
            && is_property_change(xml::local_name(name))
        {
            change_depth = change_depth.saturating_sub(1);
            return Ok(());
        }
        if change_depth > 0 {
            return Ok(());
        }
        match event {
            XmlEvent::Open(element) => match element.local() {
                "docPr" => {
                    if let Some(description) = element
                        .attr("descr")
                        .map(str::trim)
                        .filter(|description| !description.is_empty())
                    {
                        image_ordinal += 1;
                        let object_id = element
                            .attr("id")
                            .map(str::to_string)
                            .unwrap_or_else(|| image_ordinal.to_string());
                        let anchor = format!("image@{object_id}");
                        document
                            .image_object_ids
                            .insert(anchor.clone(), drawing_object_id(&main, &object_id));
                        units.push(Unit {
                            anchor,
                            kind: UnitKind::Image,
                            level: 0,
                            text: description.to_string(),
                            labels: vec![
                                "image alternative text; image content not read".into(),
                                format!(
                                    "image position: inline with {}",
                                    stack
                                        .last()
                                        .map(|paragraph| paragraph.anchor.as_str())
                                        .unwrap_or("an unanchored paragraph")
                                ),
                            ],
                            cells: Vec::new(),
                            row_cells: Vec::new(),
                        });
                    }
                }
                "p" => {
                    let nested = !stack.is_empty();
                    let anchor = match element.attr("paraId") {
                        Some(id) => format!("p:{id}"),
                        None => {
                            paragraph_ordinal += 1;
                            format!("p@{paragraph_ordinal}")
                        }
                    };
                    stack.push(Paragraph {
                        anchor,
                        in_table: table_depth > 0,
                        text_box: nested || text_box_depth > 0,
                        ..Paragraph::default()
                    });
                }
                "txbxContent" => text_box_depth += 1,
                "pStyle" => {
                    if let (Some(paragraph), Some(value)) = (stack.last_mut(), element.attr("val"))
                    {
                        paragraph.style = Some(value.to_string());
                    }
                }
                "outlineLvl" => {
                    if let (Some(paragraph), Some(value)) = (stack.last_mut(), element.attr("val"))
                    {
                        paragraph.outline = value.parse::<u8>().ok().map(|level| level + 1);
                    }
                }
                "r" => {
                    run = Run::default();
                    in_run = true;
                }
                "rPr" => in_run_properties = true,
                "vanish" | "specVanish" if in_run_properties && is_on(&element) => {
                    run.hidden = true
                }
                "color" if in_run_properties => {
                    run.white = element
                        .attr("val")
                        .is_some_and(|value| value.eq_ignore_ascii_case("FFFFFF"));
                }
                "ins" | "moveTo" => {
                    totals.tracked += 1;
                    revision.push((true, element.attr("author").unwrap_or("unknown").into()));
                }
                "del" | "moveFrom" => {
                    totals.tracked += 1;
                    revision.push((false, element.attr("author").unwrap_or("unknown").into()));
                }
                "t" | "delText" => in_text = true,
                "instrText" => in_instruction = true,
                "fldSimple" => {
                    if let Some(field) = element.attr("instr").and_then(risky_field) {
                        flag_field(&mut stack, field, &mut risky_fields);
                    }
                }
                "fldChar" => match element.attr("fldCharType") {
                    Some("begin") => fields.push((String::new(), false)),
                    Some("separate") => {
                        if let Some((instruction, evaluated)) = fields.last_mut()
                            && !*evaluated
                        {
                            *evaluated = true;
                            if let Some(field) = risky_field(instruction) {
                                flag_field(&mut stack, field, &mut risky_fields);
                            }
                        }
                    }
                    Some("end") => {
                        if let Some((instruction, evaluated)) = fields.pop()
                            && !evaluated
                            && let Some(field) = risky_field(&instruction)
                        {
                            flag_field(&mut stack, field, &mut risky_fields);
                        }
                    }
                    _ => {}
                },
                "tab" if in_run => push_word_text(&mut stack, &run, &revision, "\t", &mut totals),
                "br" | "cr" if in_run => {
                    push_word_text(&mut stack, &run, &revision, " ", &mut totals)
                }
                "noBreakHyphen" if in_run => {
                    push_word_text(&mut stack, &run, &revision, "-", &mut totals)
                }
                "commentReference" => {
                    if let (Some(paragraph), Some(id)) = (stack.last_mut(), element.attr("id")) {
                        paragraph.comments.push(id.to_string());
                    }
                }
                "tbl" => {
                    table_depth += 1;
                    if table_depth == 1 {
                        table_rows.clear();
                        table_row_cells.clear();
                    }
                }
                "tr" if table_depth == 1 => {
                    row.clear();
                    row_cells.clear();
                }
                "tc" if table_depth == 1 => {
                    cell.clear();
                    cell_paragraphs.clear();
                }
                _ => {}
            },
            XmlEvent::Text(text) => {
                if in_text {
                    push_word_text(&mut stack, &run, &revision, &text, &mut totals);
                } else if in_instruction && let Some((instruction, false)) = fields.last_mut() {
                    instruction.push_str(&text);
                }
            }
            XmlEvent::Close(name) => match xml::local_name(&name) {
                "t" | "delText" => in_text = false,
                "instrText" => {
                    in_instruction = false;
                    if let Some((instruction, false)) = fields.last_mut() {
                        instruction.push(' ');
                    }
                }
                "rPr" => in_run_properties = false,
                "r" => in_run = false,
                "ins" | "del" | "moveTo" | "moveFrom" => {
                    revision.pop();
                }
                "txbxContent" => text_box_depth = text_box_depth.saturating_sub(1),
                "p" => {
                    let Some(paragraph) = stack.pop() else {
                        return Ok(());
                    };
                    let text = paragraph.text.trim().to_string();
                    visible_words += words(&paragraph.visible);
                    let level = heading_level(&paragraph, &styles);
                    let in_cell = paragraph.in_table && !paragraph.text_box;
                    if in_cell {
                        if !cell.is_empty() && !text.is_empty() {
                            cell.push(' ');
                        }
                        cell.push_str(&text);
                        cell_paragraphs.push((paragraph.anchor.clone(), text));
                    } else if !text.is_empty() || !paragraph.labels.is_empty() {
                        let mut labels = paragraph.labels.clone();
                        if paragraph.text_box {
                            labels.push("text box".into());
                        }
                        units.push(Unit {
                            anchor: paragraph.anchor.clone(),
                            kind: if level > 0 {
                                UnitKind::Heading
                            } else {
                                UnitKind::Paragraph
                            },
                            level,
                            text,
                            labels,
                            cells: Vec::new(),
                            row_cells: Vec::new(),
                        });
                    }
                    attach_comments(&mut units, &paragraph, &comments);
                }
                "tc" if table_depth == 1 => {
                    row.push(std::mem::take(&mut cell));
                    row_cells.push(std::mem::take(&mut cell_paragraphs));
                }
                "tr" if table_depth == 1 => {
                    table_rows.push(std::mem::take(&mut row));
                    table_row_cells.push(std::mem::take(&mut row_cells));
                }
                "tbl" => {
                    table_depth = table_depth.saturating_sub(1);
                    if table_depth == 0 {
                        table_ordinal += 1;
                        let anchor = format!("tbl@{table_ordinal}");
                        for (index, cells) in table_rows.iter().enumerate() {
                            units.push(Unit {
                                anchor: format!("{anchor}/r{}", index + 1),
                                kind: UnitKind::TableRow,
                                level: 0,
                                text: cells.join(" | "),
                                labels: Vec::new(),
                                cells: Vec::new(),
                                row_cells: table_row_cells.get(index).cloned().unwrap_or_default(),
                            });
                        }
                        tables.push(Table {
                            anchor,
                            title: format!("Table {table_ordinal}"),
                            rows: std::mem::take(&mut table_rows),
                            labels: Vec::new(),
                            omitted_columns: 0,
                        });
                    }
                }
                _ => {}
            },
        }
        Ok(())
    })?;

    let mut sections = Vec::new();
    for (index, unit) in units.iter().enumerate() {
        if unit.kind != UnitKind::Heading {
            continue;
        }
        let end = units[index + 1..]
            .iter()
            .position(|next| next.kind == UnitKind::Heading && next.level <= unit.level)
            .map(|offset| index + 1 + offset)
            .unwrap_or(units.len());
        sections.push(Section {
            anchor: unit.anchor.clone(),
            title: unit.text.clone(),
            level: unit.level,
            units: index..end,
        });
    }
    let paragraphs = units
        .iter()
        .filter(|unit| matches!(unit.kind, UnitKind::Paragraph | UnitKind::Heading))
        .count();
    document.stats = vec![
        ("paragraphs".into(), paragraphs),
        ("words".into(), visible_words),
        ("headings".into(), sections.len()),
        ("tables".into(), tables.len()),
        ("comments".into(), comments.len()),
        (
            "images with alternative text".into(),
            units
                .iter()
                .filter(|unit| unit.kind == UnitKind::Image)
                .count(),
        ),
        ("tracked changes".into(), totals.tracked),
        ("hidden runs".into(), totals.hidden_runs),
        ("risky fields".into(), risky_fields),
    ];
    document.units = units;
    document.sections = sections;
    document.tables = tables;
    document.not_read = vec![
        "headers and footers",
        "footnotes and endnotes",
        "image pixels; no OCR",
        "heading levels inherited through style basedOn chains",
    ];
    Ok(())
}

fn push_word_text(
    stack: &mut [Paragraph],
    run: &Run,
    revision: &[(bool, String)],
    text: &str,
    totals: &mut WordTotals,
) {
    let Some(paragraph) = stack.last_mut() else {
        return;
    };
    let marked = match revision.last() {
        Some((true, author)) => format!("[inserted by {author}: {text}]"),
        Some((false, author)) => format!("[deleted by {author}: {text}]"),
        None => text.to_string(),
    };
    if !run.hidden && !matches!(revision.last(), Some((false, _))) {
        paragraph.visible.push_str(text);
    }
    if run.hidden {
        totals.hidden_runs += 1;
        paragraph.text.push_str(&format!("[hidden: {marked}]"));
        add_label(&mut paragraph.labels, "hidden text");
    } else if run.white && !text.trim().is_empty() {
        paragraph.text.push_str(&format!("[white text: {marked}]"));
        add_label(&mut paragraph.labels, "white text");
    } else {
        paragraph.text.push_str(&marked);
    }
}

fn add_label(labels: &mut Vec<String>, label: &str) {
    if !labels.iter().any(|existing| existing == label) {
        labels.push(label.to_string());
    }
}

fn attach_comments(
    units: &mut Vec<Unit>,
    paragraph: &Paragraph,
    comments: &HashMap<String, (String, String)>,
) {
    for id in &paragraph.comments {
        if let Some((author, text)) = comments.get(id) {
            units.push(Unit {
                anchor: format!("{}/comment:{id}", paragraph.anchor),
                kind: UnitKind::Comment,
                level: 0,
                text: text.clone(),
                labels: vec![format!("comment by {author}")],
                cells: Vec::new(),
                row_cells: Vec::new(),
            });
        }
    }
}

/// Tracked formatting changes (`w:rPrChange`, `w:tblPrExChange`, ...)
/// hold the properties before the change; they describe no visible text.
fn is_property_change(local: &str) -> bool {
    local.ends_with("PrChange") || local == "tblPrExChange" || local == "numberingChange"
}

fn flag_field(stack: &mut [Paragraph], field: &str, count: &mut usize) {
    *count += 1;
    if let Some(paragraph) = stack.last_mut() {
        add_label(
            &mut paragraph.labels,
            &format!("{field} field (never executed)"),
        );
    }
}

fn risky_field(instructions: &str) -> Option<&'static str> {
    let upper = instructions.to_ascii_uppercase();
    match upper.split_whitespace().next()? {
        "DDE" | "DDEAUTO" => Some("DDE"),
        "INCLUDETEXT" => Some("INCLUDETEXT"),
        "INCLUDEPICTURE" => Some("INCLUDEPICTURE"),
        "MACROBUTTON" => Some("MACROBUTTON"),
        _ => None,
    }
}

fn heading_level(paragraph: &Paragraph, styles: &HashMap<String, u8>) -> u8 {
    if let Some(level) = paragraph.outline
        && level <= 9
    {
        return level;
    }
    paragraph
        .style
        .as_ref()
        .and_then(|style| styles.get(style).copied())
        .unwrap_or(0)
}

/// Style id → heading level, from built-in style names (`heading 1`,
/// `Title`), which stay English whatever the document's language, or from
/// an explicit outline level on the style.
fn heading_styles(bytes: &[u8], part: &str, limits: &Limits) -> Result<HashMap<String, u8>, Error> {
    let mut levels = HashMap::new();
    let mut current: Option<String> = None;
    xml::walk(bytes, part, limits, |event| {
        match event {
            XmlEvent::Open(element) => match element.local() {
                "style" => current = element.attr("styleId").map(str::to_string),
                "name" => {
                    if let (Some(id), Some(name)) = (&current, element.attr("val")) {
                        let name = name.to_ascii_lowercase();
                        if name == "title" {
                            levels.insert(id.clone(), 1);
                        } else if let Some(level) = name
                            .strip_prefix("heading ")
                            .and_then(|level| level.parse::<u8>().ok())
                        {
                            levels.insert(id.clone(), level);
                        }
                    }
                }
                "outlineLvl" => {
                    if let (Some(id), Some(level)) = (&current, element.attr("val"))
                        && let Ok(level) = level.parse::<u8>()
                        && level < 9
                    {
                        levels.entry(id.clone()).or_insert(level + 1);
                    }
                }
                _ => {}
            },
            XmlEvent::Close(name) if xml::local_name(&name) == "style" => current = None,
            _ => {}
        }
        Ok(())
    })?;
    Ok(levels)
}

fn word_comments(
    bytes: &[u8],
    part: &str,
    limits: &Limits,
) -> Result<HashMap<String, (String, String)>, Error> {
    let mut comments = HashMap::new();
    let mut current: Option<(String, String)> = None;
    let mut text = String::new();
    let mut in_text = false;
    xml::walk(bytes, part, limits, |event| {
        match event {
            XmlEvent::Open(element) => match element.local() {
                "comment" => {
                    current = element.attr("id").map(|id| {
                        (
                            id.to_string(),
                            element.attr("author").unwrap_or("unknown").to_string(),
                        )
                    });
                    text.clear();
                }
                "t" => in_text = true,
                "p" if !text.is_empty() => text.push(' '),
                _ => {}
            },
            XmlEvent::Text(value) if in_text => text.push_str(&value),
            XmlEvent::Close(name) => match xml::local_name(&name) {
                "t" => in_text = false,
                "comment" => {
                    if let Some((id, author)) = current.take() {
                        comments.insert(id, (author, text.trim().to_string()));
                    }
                }
                _ => {}
            },
            _ => {}
        }
        Ok(())
    })?;
    Ok(comments)
}

// ---- Excel -----------------------------------------------------------------

fn excel<R: Read + Seek>(package: &mut Package<R>, document: &mut Document) -> Result<(), Error> {
    let main = package.main_part().to_string();
    let limits = *package.limits();
    let bytes = package.read_part(&main)?;
    let mut sheets: Vec<(String, Option<String>, String)> = Vec::new();
    let mut defined_names: Vec<(String, String)> = Vec::new();
    let mut current_name: Option<String> = None;
    let mut name_text = String::new();
    let mut recalculate_on_open = false;
    xml::walk(&bytes, &main, &limits, |event| {
        match event {
            XmlEvent::Open(element) => match element.local() {
                "calcPr" => {
                    recalculate_on_open = element
                        .attr("fullCalcOnLoad")
                        .is_some_and(|value| value == "1" || value == "true");
                }
                "sheet" => {
                    if let (Some(name), Some(id)) =
                        (element.attr("name"), element.attr_prefixed("id"))
                    {
                        sheets.push((
                            name.to_string(),
                            element.attr("state").map(str::to_string),
                            id.to_string(),
                        ));
                    }
                }
                "definedName" => {
                    current_name = element.attr("name").map(str::to_string);
                    name_text.clear();
                }
                _ => {}
            },
            XmlEvent::Text(text) if current_name.is_some() => name_text.push_str(&text),
            XmlEvent::Close(name) if xml::local_name(&name) == "definedName" => {
                if let Some(name) = current_name.take() {
                    defined_names.push((name, name_text.trim().to_string()));
                }
            }
            _ => {}
        }
        Ok(())
    })?;
    let shared = match package.related_part(&main, "sharedStrings")? {
        Some(part) => {
            let bytes = package.read_part(&part)?;
            shared_strings(&bytes, &part, &limits)?
        }
        None => Vec::new(),
    };
    let style_defs = match package.related_part(&main, "styles")? {
        Some(part) => {
            let bytes = package.read_part(&part)?;
            spreadsheet_styles(&bytes, &part, &limits)?
        }
        None => Vec::new(),
    };

    let mut units = Vec::new();
    let mut sections = Vec::new();
    let mut tables = Vec::new();
    let mut total_rows = 0usize;
    let mut formulas = 0usize;
    let mut structured_table_count = 0usize;
    let mut image_count = 0usize;
    let sheet_count = sheets.len();
    for (name, state, relationship) in sheets {
        let quoted = quote_sheet(&name);
        let anchor = format!("{quoted}!");
        let mut labels = Vec::new();
        match state.as_deref() {
            Some("hidden") => labels.push("hidden sheet".to_string()),
            Some("veryHidden") => labels.push("very hidden sheet".to_string()),
            _ => {}
        }
        let start = units.len();
        let Some(part) = package.part_by_relationship_id(&main, &relationship)? else {
            continue;
        };
        let is_worksheet = package
            .content_type(&part)
            .is_some_and(|content_type| content_type.contains("worksheet"));
        if !is_worksheet {
            sections.push(Section {
                anchor,
                title: format!("{name} (chart or dialog sheet, not read)"),
                level: 1,
                units: start..start,
            });
            continue;
        }
        let bytes = package.read_part(&part)?;
        let rows = sheet_rows(
            &bytes,
            &part,
            &limits,
            &shared,
            &style_defs,
            recalculate_on_open,
            &mut formulas,
        )?;
        let geometry = sheet_geometry(&bytes, &part, &limits, &quoted)?;
        document
            .sheet_geometry
            .column_widths
            .extend(geometry.column_widths);
        document
            .sheet_geometry
            .row_heights
            .extend(geometry.row_heights);
        document
            .sheet_geometry
            .default_column_widths
            .extend(geometry.default_column_widths);
        document
            .sheet_geometry
            .default_row_heights
            .extend(geometry.default_row_heights);
        document
            .sheet_geometry
            .merged_ranges
            .extend(geometry.merged_ranges);
        for row in &rows {
            for (column, style) in &row.cell_styles {
                if style.fill_color.is_some()
                    || style.font_color.is_some()
                    || style.bold
                    || style.italic
                    || style.number_format.is_some()
                {
                    document.cell_styles.insert(
                        format!("{quoted}!{}{}", column_name(*column), row.number),
                        style.clone(),
                    );
                }
            }
        }
        if let Some(drawing) = package.related_part(&part, "drawing")? {
            let drawing_bytes = package.read_part(&drawing)?;
            let references = drawing_references(&drawing_bytes, &drawing, &limits)?;
            let relationships = package.relationships(&drawing)?.to_vec();
            let chart_cells: HashMap<String, String> = references
                .iter()
                .filter_map(|reference| {
                    let relation = relationships.iter().find(|relation| {
                        relation.id == reference.relationship
                            && !relation.external
                            && relation.short_kind() == "chart"
                    })?;
                    Some((relation.target.clone(), reference.cell.as_ref()?.clone()))
                })
                .collect();
            for reference in &references {
                if let Some(end_cell) = &reference.end_cell
                    && let Some(relation) = relationships.iter().find(|relation| {
                        relation.id == reference.relationship
                            && !relation.external
                            && relation.short_kind() == "chart"
                    })
                {
                    document
                        .chart_ends
                        .insert(relation.target.clone(), format!("{quoted}!{end_cell}"));
                }
                if let (Some(width), Some(height)) = (reference.width_px, reference.height_px)
                    && let Some(relation) = relationships.iter().find(|relation| {
                        relation.id == reference.relationship
                            && !relation.external
                            && relation.short_kind() == "chart"
                    })
                {
                    document
                        .chart_sizes
                        .insert(relation.target.clone(), (width, height));
                }
            }
            for (chart_part, cell) in chart_cells {
                document
                    .chart_placements
                    .insert(chart_part, format!("{quoted}!{cell}"));
            }
            for reference in references {
                let Some(relation) = relationships
                    .iter()
                    .find(|relation| relation.id == reference.relationship && !relation.external)
                else {
                    continue;
                };
                if relation.short_kind() == "chart"
                    && let Some(cell) = &reference.cell
                {
                    document
                        .chart_placements
                        .insert(relation.target.clone(), format!("{quoted}!{cell}"));
                }
                match relation.short_kind() {
                    "image" if !reference.alt_text.trim().is_empty() => {
                        image_count += 1;
                        let image_anchor = format!("{quoted}!image@{}", reference.id);
                        document.image_object_ids.insert(
                            image_anchor.clone(),
                            drawing_object_id(&drawing, &reference.id),
                        );
                        let mut image_labels =
                            vec!["image alternative text; image content not read".into()];
                        image_labels.push(format!(
                            "image position: {quoted}!{}",
                            reference.cell.as_deref().unwrap_or("A1")
                        ));
                        if let Some(end_cell) = &reference.end_cell {
                            image_labels.push(format!("image end cell: {quoted}!{end_cell}"));
                        }
                        if let (Some(width), Some(height)) =
                            (reference.width_px, reference.height_px)
                        {
                            image_labels.push(format!("image size: {width}x{height}px"));
                        }
                        units.push(Unit {
                            anchor: image_anchor,
                            kind: UnitKind::Image,
                            level: 0,
                            text: reference.alt_text,
                            labels: image_labels,
                            cells: Vec::new(),
                            row_cells: Vec::new(),
                        });
                    }
                    "chart" => {}
                    _ => {}
                }
            }
        }
        let table_relations: Vec<(String, String)> = package
            .relationships(&part)?
            .iter()
            .filter(|relation| !relation.external && relation.short_kind() == "table")
            .map(|relation| (relation.id.clone(), relation.target.clone()))
            .collect();
        for (_, table_part) in table_relations {
            let table_bytes = package.read_part(&table_part)?;
            let Some((table_name, table_range, table_style)) =
                spreadsheet_table_info(&table_bytes, &table_part, &limits)?
            else {
                continue;
            };
            let Some((first_col, first_row, last_col, last_row)) = cell_range(&table_range) else {
                continue;
            };
            document.table_styles.push(TableStyleRange {
                sheet_anchor: anchor.clone(),
                range: table_range.clone(),
                style_name: table_style.0,
                show_row_stripes: table_style.1,
                show_column_stripes: table_style.2,
            });
            let row_map: HashMap<u32, &SheetRow> =
                rows.iter().map(|row| (row.number, row)).collect();
            let mut structured_rows = Vec::new();
            for row_number in first_row..=last_row {
                let cells = row_map.get(&row_number).map(|row| &row.cells);
                let values = (first_col..=last_col)
                    .map(|column| {
                        cells
                            .and_then(|cells| {
                                cells.iter().find(|(cell_column, _)| *cell_column == column)
                            })
                            .map(|(_, value)| value.clone())
                            .unwrap_or_default()
                    })
                    .collect::<Vec<_>>();
                units.push(Unit {
                    anchor: format!("{table_name}/r{}", row_number - first_row + 1),
                    kind: UnitKind::TableRow,
                    level: 0,
                    text: values.join(" | "),
                    labels: labels.clone(),
                    cells: Vec::new(),
                    row_cells: values
                        .iter()
                        .map(|text| vec![(String::new(), text.clone())])
                        .collect(),
                });
                structured_rows.push(values);
            }
            tables.push(Table {
                anchor: table_name.clone(),
                title: table_name,
                rows: structured_rows,
                labels: labels.clone(),
                omitted_columns: 0,
            });
            structured_table_count += 1;
        }
        let used: std::collections::BTreeSet<u32> = rows
            .iter()
            .flat_map(|row| row.cells.iter().map(|(column, _)| *column))
            .collect();
        let omitted_columns = used.len().saturating_sub(MAX_TABLE_COLUMNS);
        let shown: Vec<u32> = used.into_iter().take(MAX_TABLE_COLUMNS).collect();
        let slot_of: HashMap<u32, usize> = shown
            .iter()
            .enumerate()
            .map(|(slot, column)| (*column, slot))
            .collect();
        let mut table_rows = vec![
            std::iter::once(String::new())
                .chain(shown.iter().copied().map(column_name))
                .collect::<Vec<_>>(),
        ];
        for row in &rows {
            total_rows += 1;
            let mut dense = vec![String::new(); shown.len()];
            for (column, value) in &row.cells {
                if let Some(slot) = slot_of.get(column) {
                    dense[*slot] = value.clone();
                }
            }
            let first = row.cells.first().map(|(column, _)| *column).unwrap_or(1);
            let last = row.cells.last().map(|(column, _)| *column).unwrap_or(1);
            let mut row_labels = labels.clone();
            if row.hidden {
                row_labels.push("hidden row".into());
            }
            units.push(Unit {
                anchor: format!(
                    "{quoted}!{}{}:{}{}",
                    column_name(first),
                    row.number,
                    column_name(last),
                    row.number
                ),
                kind: UnitKind::SheetRow,
                level: 0,
                text: row
                    .cells
                    .iter()
                    .map(|(column, value)| {
                        format!("{}{}: {value}", column_name(*column), row.number)
                    })
                    .collect::<Vec<_>>()
                    .join(" | "),
                labels: row_labels,
                cells: row
                    .cells
                    .iter()
                    .map(|(column, value)| {
                        (
                            format!("{}{}", column_name(*column), row.number),
                            value.clone(),
                        )
                    })
                    .collect(),
                row_cells: Vec::new(),
            });
            table_rows.push(
                std::iter::once(row.number.to_string())
                    .chain(dense)
                    .collect(),
            );
        }
        sections.push(Section {
            anchor: anchor.clone(),
            title: name.clone(),
            level: 1,
            units: start..units.len(),
        });
        tables.push(Table {
            anchor,
            title: name,
            rows: table_rows,
            labels,
            omitted_columns,
        });
    }
    document.stats = vec![
        ("sheets".into(), sheet_count),
        ("rows".into(), total_rows),
        ("formulas".into(), formulas),
        ("defined names".into(), defined_names.len()),
        ("structured tables".into(), structured_table_count),
        ("images with alternative text".into(), image_count),
    ];
    for (name, reference) in defined_names {
        units.push(Unit {
            anchor: name.clone(),
            kind: UnitKind::DefinedName,
            level: 0,
            text: format!("defined name {name} = {reference}"),
            labels: Vec::new(),
            cells: Vec::new(),
            row_cells: Vec::new(),
        });
    }
    document.units = units;
    document.sections = sections;
    document.tables = tables;
    document.not_read = vec![
        "unsupported number formats (Canvas shows the stored value for these patterns)",
        "conditional formatting and exact table theme colors",
        "charts, pivot tables and cell comments",
        "recalculation (a formula shows the value cached in the file, which may be stale)",
        "image pixels; no OCR",
    ];
    Ok(())
}

#[allow(clippy::type_complexity)]
fn spreadsheet_table_info(
    bytes: &[u8],
    part: &str,
    limits: &Limits,
) -> Result<Option<(String, String, (Option<String>, bool, bool))>, Error> {
    let mut info = None;
    let mut table_style = (None, false, false);
    xml::walk(bytes, part, limits, |event| {
        if let XmlEvent::Open(element) = event {
            if element.local() == "table" {
                info = element
                    .attr("displayName")
                    .or_else(|| element.attr("name"))
                    .zip(element.attr("ref"))
                    .map(|(name, range)| (name.to_string(), range.to_string()));
            } else if element.local() == "tableStyleInfo" {
                table_style = (
                    element.attr("name").map(str::to_string),
                    element
                        .attr("showRowStripes")
                        .is_some_and(|value| value == "1" || value == "true"),
                    element
                        .attr("showColumnStripes")
                        .is_some_and(|value| value == "1" || value == "true"),
                );
            }
        }
        Ok(())
    })?;
    Ok(info.map(|(name, range)| (name, range, table_style)))
}

fn cell_range(range: &str) -> Option<(u32, u32, u32, u32)> {
    fn cell(address: &str) -> Option<(u32, u32)> {
        let address = address.trim().trim_start_matches('$');
        let column = column_of(address)?;
        let digits: String = address
            .chars()
            .skip_while(|ch| ch.is_ascii_alphabetic())
            .filter(|ch| ch.is_ascii_digit())
            .collect();
        Some((column, digits.parse().ok()?))
    }
    let (start, end) = range
        .split_once(':')
        .map_or((range, range), |(start, end)| (start, end));
    let (first_col, first_row) = cell(start)?;
    let (last_col, last_row) = cell(end)?;
    (first_col <= last_col && first_row <= last_row)
        .then_some((first_col, first_row, last_col, last_row))
}

struct SheetRow {
    number: u32,
    hidden: bool,
    cells: Vec<(u32, String)>,
    cell_styles: Vec<(u32, CellStyle)>,
}

fn sheet_geometry(
    bytes: &[u8],
    part: &str,
    limits: &Limits,
    sheet: &str,
) -> Result<SheetGeometry, Error> {
    let mut geometry = SheetGeometry::default();
    xml::walk(bytes, part, limits, |event| {
        if let XmlEvent::Open(element) = event {
            match element.local() {
                "col" => {
                    let first = element
                        .attr("min")
                        .and_then(|v| v.parse::<u32>().ok())
                        .unwrap_or(1);
                    let last = element
                        .attr("max")
                        .and_then(|v| v.parse::<u32>().ok())
                        .unwrap_or(first);
                    let width = element.attr("width").and_then(|v| v.parse::<f32>().ok());
                    let hidden = element
                        .attr("hidden")
                        .is_some_and(|v| v == "1" || v == "true");
                    if let Some(width) = width.filter(|v| v.is_finite() && *v >= 0.0) {
                        let pixels = if hidden {
                            0
                        } else {
                            (width * 7.0 + 5.0).round() as u32
                        };
                        for column in first..=last.min(16_384) {
                            geometry
                                .column_widths
                                .insert(format!("{sheet}!{}", column_name(column)), pixels);
                        }
                    }
                }
                "sheetFormatPr" => {
                    if let Some(width) = element
                        .attr("defaultColWidth")
                        .and_then(|v| v.parse::<f32>().ok())
                        .filter(|v| v.is_finite() && *v >= 0.0)
                    {
                        geometry
                            .default_column_widths
                            .insert(sheet.to_string(), (width * 7.0 + 5.0).round() as u32);
                    }
                    if let Some(height) = element
                        .attr("defaultRowHeight")
                        .and_then(|v| v.parse::<f32>().ok())
                        .filter(|v| v.is_finite() && *v >= 0.0)
                    {
                        geometry
                            .default_row_heights
                            .insert(sheet.to_string(), (height * 4.0 / 3.0).round() as u32);
                    }
                }
                "row" => {
                    let row = element.attr("r").and_then(|v| v.parse::<u32>().ok());
                    let height = element.attr("ht").and_then(|v| v.parse::<f32>().ok());
                    let hidden = element
                        .attr("hidden")
                        .is_some_and(|v| v == "1" || v == "true");
                    if let (Some(row), Some(height)) =
                        (row, height.filter(|v| v.is_finite() && *v >= 0.0))
                    {
                        geometry.row_heights.insert(
                            format!("{sheet}!{row}"),
                            if hidden {
                                0
                            } else {
                                (height * 4.0 / 3.0).round() as u32
                            },
                        );
                    }
                }
                "mergeCell" => {
                    if geometry.merged_ranges.len() < 10_000
                        && let Some(range) = element.attr("ref")
                        && cell_range(range).is_some()
                    {
                        geometry.merged_ranges.push(MergedCellRange {
                            sheet_anchor: format!("{sheet}!"),
                            range: range.to_string(),
                        });
                    }
                }
                _ => {}
            }
        }
        Ok(())
    })?;
    Ok(geometry)
}

#[derive(Default)]
struct CellState {
    column: u32,
    kind: Option<String>,
    style_index: Option<usize>,
    value: String,
    formula: Option<String>,
}

fn sheet_rows(
    bytes: &[u8],
    part: &str,
    limits: &Limits,
    shared: &[String],
    style_defs: &[CellStyle],
    recalculate_on_open: bool,
    formulas: &mut usize,
) -> Result<Vec<SheetRow>, Error> {
    let mut rows: Vec<SheetRow> = Vec::new();
    let mut cell: Option<CellState> = None;
    let mut in_value = false;
    let mut in_formula = false;
    let mut in_inline = false;
    let mut next_row = 1u32;
    let mut next_column = 1u32;
    xml::walk(bytes, part, limits, |event| {
        match event {
            XmlEvent::Open(element) => match element.local() {
                "row" => {
                    let number = element
                        .attr("r")
                        .and_then(|value| value.parse::<u32>().ok())
                        .unwrap_or(next_row);
                    next_row = number.saturating_add(1);
                    next_column = 1;
                    rows.push(SheetRow {
                        number,
                        hidden: element
                            .attr("hidden")
                            .is_some_and(|value| value == "1" || value == "true"),
                        cells: Vec::new(),
                        cell_styles: Vec::new(),
                    });
                }
                "c" => {
                    let column = element.attr("r").and_then(column_of).unwrap_or(next_column);
                    next_column = column.saturating_add(1);
                    cell = Some(CellState {
                        column,
                        kind: element.attr("t").map(str::to_string),
                        style_index: element
                            .attr("s")
                            .and_then(|value| value.parse::<usize>().ok()),
                        ..CellState::default()
                    });
                }
                "v" => in_value = true,
                "f" => {
                    in_formula = true;
                    if let Some(cell) = cell.as_mut() {
                        cell.formula.get_or_insert_with(String::new);
                    }
                }
                "is" => in_inline = true,
                _ => {}
            },
            XmlEvent::Text(text) => {
                if let Some(cell) = cell.as_mut() {
                    if in_formula {
                        if let Some(formula) = cell.formula.as_mut() {
                            formula.push_str(&text);
                        }
                    } else if in_value || in_inline {
                        cell.value.push_str(&text);
                    }
                }
            }
            XmlEvent::Close(name) => match xml::local_name(&name) {
                "v" => in_value = false,
                "f" => in_formula = false,
                "is" => in_inline = false,
                "c" => {
                    if let Some(state) = cell.take() {
                        if let Some(style) =
                            state.style_index.and_then(|index| style_defs.get(index))
                            && let Some(row) = rows.last_mut()
                        {
                            row.cell_styles.push((state.column, style.clone()));
                        }
                        let shown = match state.kind.as_deref() {
                            Some("s") => state
                                .value
                                .trim()
                                .parse::<usize>()
                                .ok()
                                .and_then(|index| shared.get(index).cloned())
                                .unwrap_or_default(),
                            Some("b") if state.value.trim() == "1" => "TRUE".into(),
                            Some("b") => "FALSE".into(),
                            Some("e") => format!("#ERROR {}", state.value.trim()),
                            _ => state.value.clone(),
                        };
                        let rendered = match state.formula {
                            Some(formula) => {
                                *formulas += 1;
                                let cached = match (shown.is_empty(), recalculate_on_open) {
                                    (true, _) => "[not calculated yet]".to_string(),
                                    (false, true) => {
                                        format!("[cached: {shown}, stale until recalculated]")
                                    }
                                    (false, false) => format!("[cached: {shown}]"),
                                };
                                if formula.trim().is_empty() {
                                    format!("(shared formula) {cached}")
                                } else {
                                    format!("={} {cached}", formula.trim())
                                }
                            }
                            None => shown,
                        };
                        if !rendered.is_empty()
                            && let Some(row) = rows.last_mut()
                        {
                            row.cells.push((state.column, rendered));
                        }
                    }
                }
                _ => {}
            },
        }
        Ok(())
    })?;
    rows.retain(|row| !row.cells.is_empty() || !row.cell_styles.is_empty());
    Ok(rows)
}

#[derive(Default)]
struct SpreadsheetFont {
    color: Option<String>,
    bold: bool,
    italic: bool,
}

#[derive(Default)]
struct SpreadsheetCellFormat {
    font_id: usize,
    fill_id: usize,
    number_format_id: u32,
    horizontal_alignment: Option<String>,
    vertical_alignment: Option<String>,
    wrap_text: bool,
}

fn spreadsheet_styles(bytes: &[u8], part: &str, limits: &Limits) -> Result<Vec<CellStyle>, Error> {
    let mut fonts = Vec::new();
    let mut fills = Vec::new();
    let mut xfs: Vec<SpreadsheetCellFormat> = Vec::new();
    let mut custom_formats: HashMap<u32, String> = HashMap::new();
    let mut section = "";
    let mut font: Option<SpreadsheetFont> = None;
    let mut fill: Option<String> = None;
    let mut cell_format: Option<SpreadsheetCellFormat> = None;
    xml::walk(bytes, part, limits, |event| {
        match event {
            XmlEvent::Open(element) => match element.local() {
                "fonts" => section = "fonts",
                "fills" => section = "fills",
                "cellXfs" => section = "cellXfs",
                "font" if section == "fonts" => font = Some(SpreadsheetFont::default()),
                "fill" if section == "fills" => fill = None,
                "b" if font.is_some() => {
                    if let Some(font) = font.as_mut() {
                        font.bold = element
                            .attr("val")
                            .is_none_or(|value| value != "0" && value != "false");
                    }
                }
                "i" if font.is_some() => {
                    if let Some(font) = font.as_mut() {
                        font.italic = element
                            .attr("val")
                            .is_none_or(|value| value != "0" && value != "false");
                    }
                }
                "color" if font.is_some() => {
                    if let Some(color) = element.attr("rgb").and_then(normalize_argb)
                        && let Some(font) = font.as_mut()
                    {
                        font.color = Some(color);
                    }
                }
                "fgColor" | "bgColor" if section == "fills" => {
                    if let Some(color) = element.attr("rgb").and_then(normalize_argb)
                        && (fill.is_none() || element.local() == "fgColor")
                    {
                        fill = Some(color);
                    }
                }
                "xf" if section == "cellXfs" => {
                    cell_format = Some(SpreadsheetCellFormat {
                        font_id: element
                            .attr("fontId")
                            .and_then(|value| value.parse::<usize>().ok())
                            .unwrap_or(0),
                        fill_id: element
                            .attr("fillId")
                            .and_then(|value| value.parse::<usize>().ok())
                            .unwrap_or(0),
                        number_format_id: element
                            .attr("numFmtId")
                            .and_then(|value| value.parse::<u32>().ok())
                            .unwrap_or(0),
                        ..SpreadsheetCellFormat::default()
                    });
                }
                "alignment" if cell_format.is_some() => {
                    if let Some(format) = cell_format.as_mut() {
                        format.horizontal_alignment = element
                            .attr("horizontal")
                            .and_then(spreadsheet_horizontal_alignment);
                        format.vertical_alignment = element
                            .attr("vertical")
                            .and_then(spreadsheet_vertical_alignment);
                        format.wrap_text = element.attr("wrapText").is_some_and(|value| {
                            value == "1" || value.eq_ignore_ascii_case("true")
                        });
                    }
                }
                "numFmt" => {
                    if let (Some(id), Some(code)) = (
                        element
                            .attr("numFmtId")
                            .and_then(|value| value.parse().ok()),
                        element.attr("formatCode"),
                    ) {
                        custom_formats.insert(id, code.to_string());
                    }
                }
                _ => {}
            },
            XmlEvent::Close(name) => match xml::local_name(&name) {
                "font" if section == "fonts" => {
                    fonts.push(font.take().unwrap_or_default());
                }
                "fill" if section == "fills" => fills.push(fill.take()),
                "xf" if section == "cellXfs" => {
                    xfs.push(cell_format.take().unwrap_or_default());
                }
                "fonts" | "fills" | "cellXfs" => section = "",
                _ => {}
            },
            _ => {}
        }
        Ok(())
    })?;
    Ok(xfs
        .into_iter()
        .map(|format| {
            let font = fonts.get(format.font_id);
            CellStyle {
                fill_color: fills.get(format.fill_id).and_then(Clone::clone),
                font_color: font.and_then(|font| font.color.clone()),
                bold: font.is_some_and(|font| font.bold),
                italic: font.is_some_and(|font| font.italic),
                number_format: custom_formats
                    .get(&format.number_format_id)
                    .cloned()
                    .or_else(|| builtin_number_format(format.number_format_id).map(str::to_string)),
                horizontal_alignment: format.horizontal_alignment,
                vertical_alignment: format.vertical_alignment,
                wrap_text: format.wrap_text,
            }
        })
        .collect())
}

fn spreadsheet_horizontal_alignment(value: &str) -> Option<String> {
    match value {
        "general" | "left" | "center" | "centerContinuous" | "right" | "fill" | "justify"
        | "distributed" => Some(value.to_string()),
        _ => None,
    }
}

fn spreadsheet_vertical_alignment(value: &str) -> Option<String> {
    match value {
        "top" | "center" | "bottom" | "justify" | "distributed" => Some(value.to_string()),
        _ => None,
    }
}

fn builtin_number_format(id: u32) -> Option<&'static str> {
    Some(match id {
        0 => "General",
        1 => "0",
        2 => "0.00",
        3 => "#,##0",
        4 => "#,##0.00",
        5 => "$#,##0;($#,##0)",
        6 => "$#,##0;($#,##0)",
        7 => "$#,##0.00;($#,##0.00)",
        8 => "$#,##0.00;($#,##0.00)",
        9 => "0%",
        10 => "0.00%",
        11 => "0.00E+00",
        12 => "# ?/?",
        13 => "# ??/??",
        14 => "m/d/yy",
        15 => "d-mmm-yy",
        16 => "d-mmm",
        17 => "mmm-yy",
        18 => "h:mm AM/PM",
        19 => "h:mm:ss AM/PM",
        20 => "h:mm",
        21 => "h:mm:ss",
        22 => "m/d/yy h:mm",
        37 => "#,##0;(#,##0)",
        38 => "#,##0;(#,##0)",
        39 => "#,##0.00;(#,##0.00)",
        40 => "#,##0.00;(#,##0.00)",
        49 => "@",
        _ => return None,
    })
}

fn normalize_argb(value: &str) -> Option<String> {
    let hex = if value.len() == 8 {
        value.get(2..)?
    } else if value.len() == 6 {
        value
    } else {
        return None;
    };
    hex.bytes()
        .all(|byte| byte.is_ascii_hexdigit())
        .then(|| format!("#{}", hex.to_ascii_uppercase()))
}

fn shared_strings(bytes: &[u8], part: &str, limits: &Limits) -> Result<Vec<String>, Error> {
    let mut strings = Vec::new();
    let mut current = String::new();
    let mut in_text = false;
    let mut phonetic = 0usize;
    xml::walk(bytes, part, limits, |event| {
        match event {
            XmlEvent::Open(element) => match element.local() {
                "si" => current.clear(),
                "rPh" => phonetic += 1,
                "t" if phonetic == 0 => in_text = true,
                _ => {}
            },
            XmlEvent::Text(text) if in_text => current.push_str(&text),
            XmlEvent::Close(name) => match xml::local_name(&name) {
                "t" => in_text = false,
                "rPh" => phonetic = phonetic.saturating_sub(1),
                "si" => strings.push(std::mem::take(&mut current)),
                _ => {}
            },
            _ => {}
        }
        Ok(())
    })?;
    Ok(strings)
}

fn column_of(reference: &str) -> Option<u32> {
    let letters: String = reference
        .chars()
        .take_while(|character| character.is_ascii_alphabetic())
        .collect();
    if letters.is_empty() || letters.len() > 3 {
        return None;
    }
    letters.chars().try_fold(0u32, |total, character| {
        Some(total * 26 + (character.to_ascii_uppercase() as u32 - 'A' as u32 + 1))
    })
}

/// `1` → `A`, `27` → `AA`.
pub fn column_name(mut column: u32) -> String {
    let mut name = Vec::new();
    while column > 0 {
        let remainder = (column - 1) % 26;
        name.push(char::from(b'A' + remainder as u8));
        column = (column - 1) / 26;
    }
    name.iter().rev().collect()
}

fn quote_sheet(name: &str) -> String {
    if name
        .chars()
        .all(|character| character.is_alphanumeric() || character == '_' || character == '.')
    {
        name.to_string()
    } else {
        format!("'{}'", name.replace('\'', "''"))
    }
}

// ---- PowerPoint ------------------------------------------------------------

#[derive(Default, Clone)]
struct Shape {
    id: String,
    name: String,
    hidden: bool,
    alternative_text: String,
    placeholder: Option<String>,
    offset: Option<(i64, i64)>,
    extent: Option<(i64, i64)>,
    paragraphs: Vec<String>,
    table: Vec<Vec<String>>,
    chart_relationship: Option<String>,
}

fn slide_object_position(slide: usize, shape: &Shape, slide_size: Option<(i64, i64)>) -> String {
    let kind = shape
        .placeholder
        .as_deref()
        .map(|kind| format!("{kind} placeholder"))
        .unwrap_or_else(|| "slide object".into());
    let name = shape.name.trim();
    let object = if name.is_empty() {
        kind
    } else {
        format!("{name} ({kind})")
    };
    let coordinates = shape.offset.map(|(x, y)| {
        let size = shape
            .extent
            .map(|(width, height)| {
                format!(
                    ", width {:.2} in, height {:.2} in",
                    width as f64 / 914_400.0,
                    height as f64 / 914_400.0
                )
            })
            .unwrap_or_default();
        format!(
            ", x {:.2} in from left, y {:.2} in from top{size}",
            x as f64 / 914_400.0,
            y as f64 / 914_400.0
        )
    });
    let bounds = match (shape.offset, shape.extent, slide_size) {
        (Some((x, y)), Some((width, height)), Some((slide_width, slide_height))) => {
            let right = x.saturating_add(width);
            let bottom = y.saturating_add(height);
            if x < 0 || y < 0 || right > slide_width || bottom > slide_height {
                ", extends beyond slide bounds".to_string()
            } else {
                String::new()
            }
        }
        _ => String::new(),
    };
    format!(
        "Slide {slide}: {object}{}{}",
        coordinates.unwrap_or_default(),
        bounds
    )
}

fn powerpoint<R: Read + Seek>(
    package: &mut Package<R>,
    document: &mut Document,
) -> Result<(), Error> {
    let main = package.main_part().to_string();
    let limits = *package.limits();
    let bytes = package.read_part(&main)?;
    let mut slide_ids: Vec<(String, String)> = Vec::new();
    let mut size: Option<(i64, i64)> = None;
    xml::walk(&bytes, &main, &limits, |event| {
        if let XmlEvent::Open(element) = event {
            match element.local() {
                "sldId" => {
                    if let (Some(id), Some(relationship)) =
                        (element.attr_unprefixed("id"), element.attr_prefixed("id"))
                    {
                        slide_ids.push((id.to_string(), relationship.to_string()));
                    }
                }
                "sldSz" => {
                    size = element
                        .attr("cx")
                        .and_then(|cx| cx.parse().ok())
                        .zip(element.attr("cy").and_then(|cy| cy.parse().ok()));
                }
                _ => {}
            }
        }
        Ok(())
    })?;

    let mut units = Vec::new();
    let mut sections = Vec::new();
    let mut tables = Vec::new();
    let mut hidden_slides = 0usize;
    let mut with_notes = 0usize;
    for (index, (slide_id, relationship)) in slide_ids.iter().enumerate() {
        let number = index + 1;
        let Some(part) = package.part_by_relationship_id(&main, relationship)? else {
            continue;
        };
        let bytes = package.read_part(&part)?;
        let (hidden, shapes) = slide_shapes(&bytes, &part, &limits)?;
        let anchor = format!("slide:{slide_id}");
        let chart_relations: Vec<(String, String)> = package
            .relationships(&part)?
            .iter()
            .filter(|relation| relation.kind.ends_with("/chart") && !relation.external)
            .map(|relation| (relation.id.clone(), relation.target.clone()))
            .collect();
        let mut slide_charts = HashMap::new();
        for (relationship_id, chart_part) in chart_relations {
            if package
                .content_type(&chart_part)
                .is_some_and(|kind| kind.ends_with("drawingml.chart+xml"))
            {
                let chart_bytes = package.read_part(&chart_part)?;
                if let Some(mut table) = chart_table(&chart_bytes, &chart_part, &limits)? {
                    table.anchor = format!("{anchor}/{}", table.anchor);
                    slide_charts.insert(relationship_id, table);
                }
            }
        }
        let title = shapes
            .iter()
            .find(|shape| matches!(shape.placeholder.as_deref(), Some("title" | "ctrTitle")))
            .map(|shape| shape.paragraphs.join(" "))
            .filter(|title| !title.trim().is_empty())
            .unwrap_or_else(|| format!("Slide {number}"));
        let start = units.len();
        let mut slide_labels = Vec::new();
        if hidden {
            hidden_slides += 1;
            slide_labels.push("hidden slide".to_string());
        }
        units.push(Unit {
            anchor: anchor.clone(),
            kind: UnitKind::Slide,
            level: 1,
            text: format!("Slide {number}: {title}"),
            labels: slide_labels.clone(),
            cells: Vec::new(),
            row_cells: Vec::new(),
        });
        for shape in &shapes {
            let mut labels = slide_labels.clone();
            if shape.hidden {
                labels.push("hidden shape".into());
            }
            let is_title = matches!(shape.placeholder.as_deref(), Some("title" | "ctrTitle"));
            if let (Some((x, y)), Some((width, height)), Some((slide_width, slide_height))) =
                (shape.offset, shape.extent, size)
                && (x >= slide_width || y >= slide_height || x + width <= 0 || y + height <= 0)
            {
                labels.push("off-slide, not visible when presented".into());
            }
            let position = slide_object_position(number, shape, size);
            if !is_title {
                labels.push(format!("shape position: {position}"));
            }
            if let Some(relationship_id) = &shape.chart_relationship
                && let Some(mut table) = slide_charts.remove(relationship_id)
            {
                table.labels.push(format!("chart position: {position}"));
                for (row_index, row) in table.rows.iter().enumerate() {
                    units.push(Unit {
                        anchor: format!("{}/r{}", table.anchor, row_index + 1),
                        kind: UnitKind::TableRow,
                        level: 0,
                        text: row.join(" | "),
                        labels: table.labels.clone(),
                        cells: Vec::new(),
                        row_cells: row
                            .iter()
                            .map(|text| vec![(String::new(), text.clone())])
                            .collect(),
                    });
                }
                document.tables.push(table);
            }
            let text = shape.paragraphs.join(PARAGRAPH_BREAK);
            if !text.trim().is_empty() {
                units.push(Unit {
                    anchor: format!("{anchor}/shape:{}", shape.id),
                    kind: UnitKind::Shape,
                    level: 0,
                    text,
                    labels: labels.clone(),
                    cells: Vec::new(),
                    row_cells: Vec::new(),
                });
            }
            if !shape.alternative_text.trim().is_empty() {
                let image_anchor = format!("{anchor}/shape:{}", shape.id);
                document
                    .image_object_ids
                    .insert(image_anchor.clone(), drawing_object_id(&part, &shape.id));
                units.push(Unit {
                    anchor: image_anchor,
                    kind: UnitKind::Image,
                    level: 0,
                    text: shape.alternative_text.clone(),
                    labels: vec![
                        "image alternative text; image content not read".into(),
                        format!("image position: {position}"),
                    ],
                    cells: Vec::new(),
                    row_cells: Vec::new(),
                });
            }
            if !shape.table.is_empty() {
                let mut table_labels = labels.clone();
                table_labels.push(format!("table position: {position}"));
                for (row_index, row) in shape.table.iter().enumerate() {
                    units.push(Unit {
                        anchor: format!("{anchor}/shape:{}/tbl@1/r{}", shape.id, row_index + 1),
                        kind: UnitKind::TableRow,
                        level: 0,
                        text: row.join(" | "),
                        labels: table_labels.clone(),
                        cells: Vec::new(),
                        row_cells: row
                            .iter()
                            .map(|text| vec![(String::new(), text.clone())])
                            .collect(),
                    });
                }
                tables.push(Table {
                    anchor: format!("{anchor}/shape:{}", shape.id),
                    title: format!("Slide {number} · {}", shape.name),
                    rows: shape.table.clone(),
                    labels: table_labels,
                    omitted_columns: 0,
                });
            }
        }
        if let Some(notes_part) = package.related_part(&part, "notesSlide")? {
            let bytes = package.read_part(&notes_part)?;
            let (_, note_shapes) = slide_shapes(&bytes, &notes_part, &limits)?;
            let notes: Vec<String> = note_shapes
                .iter()
                .filter(|shape| shape.placeholder.as_deref() == Some("body"))
                .map(|shape| shape.paragraphs.join(PARAGRAPH_BREAK))
                .filter(|text| !text.trim().is_empty())
                .collect();
            if !notes.is_empty() {
                with_notes += 1;
                units.push(Unit {
                    anchor: format!("{anchor}/notes"),
                    kind: UnitKind::Notes,
                    level: 0,
                    text: notes.join(PARAGRAPH_BREAK),
                    labels: vec!["speaker notes".into()],
                    cells: Vec::new(),
                    row_cells: Vec::new(),
                });
            }
        }
        sections.push(Section {
            anchor,
            title: format!("Slide {number}: {title}"),
            level: 1,
            units: start..units.len(),
        });
    }
    document.stats = vec![
        ("slides".into(), slide_ids.len()),
        ("hidden slides".into(), hidden_slides),
        ("slides with notes".into(), with_notes),
        ("tables".into(), tables.len()),
    ];
    document.units = units;
    document.sections = sections;
    document.tables.extend(tables);
    document.not_read = vec![
        "comments",
        "chart data and SmartArt text",
        "text inherited from layouts and masters",
        "off-slide checks for shapes inside groups",
    ];
    Ok(())
}

fn slide_shapes(bytes: &[u8], part: &str, limits: &Limits) -> Result<(bool, Vec<Shape>), Error> {
    let mut hidden_slide = false;
    let mut shapes: Vec<Shape> = Vec::new();
    let mut stack: Vec<Shape> = Vec::new();
    let mut group_depth = 0usize;
    let mut paragraph: Option<String> = None;
    let mut in_text = false;
    let mut row: Option<Vec<String>> = None;
    let mut cell: Option<String> = None;
    xml::walk(bytes, part, limits, |event| {
        match event {
            XmlEvent::Open(element) => match element.local() {
                "sld" => {
                    hidden_slide = element
                        .attr("show")
                        .is_some_and(|value| value == "0" || value == "false")
                }
                "grpSp" => group_depth += 1,
                "sp" | "pic" | "graphicFrame" | "cxnSp" => stack.push(Shape::default()),
                "cNvPr" => {
                    if let Some(shape) = stack.last_mut()
                        && shape.id.is_empty()
                    {
                        shape.id = element.attr("id").unwrap_or_default().to_string();
                        shape.name = element.attr("name").unwrap_or_default().to_string();
                        shape.alternative_text =
                            element.attr("descr").unwrap_or_default().to_string();
                        shape.hidden = element
                            .attr("hidden")
                            .is_some_and(|value| value == "1" || value == "true");
                    }
                }
                "chart" => {
                    if let Some(shape) = stack.last_mut() {
                        shape.chart_relationship = element.attr_prefixed("id").map(str::to_string);
                    }
                }
                "ph" => {
                    if let Some(shape) = stack.last_mut() {
                        shape.placeholder =
                            Some(element.attr("type").unwrap_or("body").to_string());
                    }
                }
                "off" if group_depth == 0 => {
                    if let Some(shape) = stack.last_mut()
                        && shape.offset.is_none()
                    {
                        shape.offset = element
                            .attr("x")
                            .and_then(|x| x.parse().ok())
                            .zip(element.attr("y").and_then(|y| y.parse().ok()));
                    }
                }
                "ext" if group_depth == 0 => {
                    if let Some(shape) = stack.last_mut()
                        && shape.extent.is_none()
                        && element.attr("cx").is_some()
                    {
                        shape.extent = element
                            .attr("cx")
                            .and_then(|x| x.parse().ok())
                            .zip(element.attr("cy").and_then(|y| y.parse().ok()));
                    }
                }
                "p" => paragraph = Some(String::new()),
                "t" => in_text = true,
                "br" => {
                    if let Some(paragraph) = paragraph.as_mut() {
                        paragraph.push(' ');
                    }
                }
                "tr" => row = Some(Vec::new()),
                "tc" => cell = Some(String::new()),
                _ => {}
            },
            XmlEvent::Text(text) if in_text => {
                if let Some(paragraph) = paragraph.as_mut() {
                    paragraph.push_str(&text);
                }
            }
            XmlEvent::Close(name) => match xml::local_name(&name) {
                "t" => in_text = false,
                "p" => {
                    if let Some(text) = paragraph.take() {
                        let text = text.trim().to_string();
                        if let Some(cell) = cell.as_mut() {
                            if !cell.is_empty() && !text.is_empty() {
                                cell.push(' ');
                            }
                            cell.push_str(&text);
                        } else if !text.is_empty()
                            && let Some(shape) = stack.last_mut()
                        {
                            shape.paragraphs.push(text);
                        }
                    }
                }
                "tc" => {
                    if let (Some(text), Some(row)) = (cell.take(), row.as_mut()) {
                        row.push(text);
                    }
                }
                "tr" => {
                    if let (Some(cells), Some(shape)) = (row.take(), stack.last_mut()) {
                        shape.table.push(cells);
                    }
                }
                "sp" | "pic" | "graphicFrame" | "cxnSp" => {
                    if let Some(shape) = stack.pop() {
                        shapes.push(shape);
                    }
                }
                "grpSp" => group_depth = group_depth.saturating_sub(1),
                _ => {}
            },
            _ => {}
        }
        Ok(())
    })?;
    Ok((hidden_slide, shapes))
}

// ---- Visio -----------------------------------------------------------------

struct VisioPage {
    id: String,
    name: String,
    background: bool,
    relationship: Option<String>,
}

fn visio<R: Read + Seek>(package: &mut Package<R>, document: &mut Document) -> Result<(), Error> {
    let main = package.main_part().to_string();
    let limits = *package.limits();
    let Some(pages_part) = package.related_part(&main, "pages")? else {
        document.not_read = vec!["pages (the drawing has no pages part)"];
        return Ok(());
    };
    let bytes = package.read_part(&pages_part)?;
    let mut pages: Vec<VisioPage> = Vec::new();
    xml::walk(&bytes, &pages_part, &limits, |event| {
        if let XmlEvent::Open(element) = event {
            match element.local() {
                "Page" => pages.push(VisioPage {
                    id: element.attr("ID").unwrap_or_default().to_string(),
                    name: element
                        .attr("Name")
                        .or_else(|| element.attr("NameU"))
                        .unwrap_or_default()
                        .to_string(),
                    background: element.attr("Background").is_some_and(|value| value == "1"),
                    relationship: None,
                }),
                "Rel" => {
                    if let (Some(page), Some(id)) = (pages.last_mut(), element.attr_prefixed("id"))
                    {
                        page.relationship = Some(id.to_string());
                    }
                }
                _ => {}
            }
        }
        Ok(())
    })?;
    let mut units = Vec::new();
    let mut sections = Vec::new();
    let mut shape_count = 0usize;
    for page in pages {
        let anchor = format!("page:{}", page.id);
        let start = units.len();
        let labels = if page.background {
            vec!["background page".to_string()]
        } else {
            Vec::new()
        };
        units.push(Unit {
            anchor: anchor.clone(),
            kind: UnitKind::Page,
            level: 1,
            text: format!("Page: {}", page.name),
            labels: labels.clone(),
            cells: Vec::new(),
            row_cells: Vec::new(),
        });
        if let Some(relationship) = page.relationship
            && let Some(part) = package.part_by_relationship_id(&pages_part, &relationship)?
        {
            let bytes = package.read_part(&part)?;
            for (shape_id, shape_name, text) in visio_shapes(&bytes, &part, &limits)? {
                shape_count += 1;
                if text.is_empty() {
                    continue;
                }
                units.push(Unit {
                    anchor: format!("{anchor}/shape:{shape_id}"),
                    kind: UnitKind::Shape,
                    level: 0,
                    text: if shape_name.is_empty() {
                        text
                    } else {
                        format!("{shape_name}: {text}")
                    },
                    labels: labels.clone(),
                    cells: Vec::new(),
                    row_cells: Vec::new(),
                });
            }
        }
        sections.push(Section {
            anchor,
            title: page.name,
            level: 1,
            units: start..units.len(),
        });
    }
    document.stats = vec![
        ("pages".into(), sections.len()),
        ("shapes".into(), shape_count),
    ];
    document.units = units;
    document.sections = sections;
    document.not_read = vec![
        "text and geometry inherited from masters",
        "shape data (properties) and connectors",
    ];
    Ok(())
}

fn visio_shapes(
    bytes: &[u8],
    part: &str,
    limits: &Limits,
) -> Result<Vec<(String, String, String)>, Error> {
    let mut shapes = Vec::new();
    let mut stack: Vec<(String, String, String)> = Vec::new();
    let mut in_text = 0usize;
    xml::walk(bytes, part, limits, |event| {
        match event {
            XmlEvent::Open(element) => match element.local() {
                "Shape" => stack.push((
                    element.attr("ID").unwrap_or_default().to_string(),
                    element
                        .attr("Name")
                        .or_else(|| element.attr("NameU"))
                        .unwrap_or_default()
                        .to_string(),
                    String::new(),
                )),
                "Text" => in_text += 1,
                _ => {}
            },
            XmlEvent::Text(text) if in_text > 0 => {
                if let Some(shape) = stack.last_mut() {
                    shape.2.push_str(&text);
                }
            }
            XmlEvent::Close(name) => match xml::local_name(&name) {
                "Text" => in_text = in_text.saturating_sub(1),
                "Shape" => {
                    if let Some((id, name, text)) = stack.pop() {
                        let text = text.split_whitespace().collect::<Vec<_>>().join(" ");
                        shapes.push((id, name, text));
                    }
                }
                _ => {}
            },
            _ => {}
        }
        Ok(())
    })?;
    Ok(shapes)
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::*;

    #[test]
    fn drawing_projection_keeps_two_cell_object_bounds() {
        let drawing = br#"<xdr:wsDr xmlns:xdr="http://schemas.openxmlformats.org/drawingml/2006/spreadsheetDrawing" xmlns:a="http://schemas.openxmlformats.org/drawingml/2006/main" xmlns:r="http://schemas.openxmlformats.org/officeDocument/2006/relationships"><xdr:twoCellAnchor><xdr:from><xdr:col>6</xdr:col><xdr:colOff>9525</xdr:colOff><xdr:row>3</xdr:row><xdr:rowOff>19050</xdr:rowOff></xdr:from><xdr:to><xdr:col>10</xdr:col><xdr:colOff>28575</xdr:colOff><xdr:row>14</xdr:row><xdr:rowOff>38100</xdr:rowOff></xdr:to><xdr:pic><xdr:nvPicPr><xdr:cNvPr id="7" name="Photo" descr="Daily status marker"/></xdr:nvPicPr><xdr:blipFill><a:blip r:embed="rId1"/></xdr:blipFill></xdr:pic></xdr:twoCellAnchor></xdr:wsDr>"#;
        let references =
            drawing_references(drawing, "xl/drawings/drawing1.xml", &Limits::default()).unwrap();
        assert_eq!(references.len(), 1);
        assert_eq!(references[0].cell.as_deref(), Some("G4"));
        assert_eq!(references[0].end_cell.as_deref(), Some("K15"));
        assert_eq!(references[0].offset_x_px, Some(1));
        assert_eq!(references[0].offset_y_px, Some(2));
        assert_eq!(references[0].end_offset_x_px, Some(3));
        assert_eq!(references[0].end_offset_y_px, Some(4));
        assert_eq!(references[0].alt_text, "Daily status marker");
    }

    #[test]
    fn drawing_projection_reads_one_cell_object_size() {
        let drawing = br#"<xdr:wsDr xmlns:xdr="http://schemas.openxmlformats.org/drawingml/2006/spreadsheetDrawing" xmlns:a="http://schemas.openxmlformats.org/drawingml/2006/main" xmlns:r="http://schemas.openxmlformats.org/officeDocument/2006/relationships"><xdr:oneCellAnchor><xdr:from><xdr:col>3</xdr:col><xdr:row>1</xdr:row></xdr:from><xdr:ext cx="1371600" cy="1371600"/><xdr:pic><xdr:nvPicPr><xdr:cNvPr id="3" name="Image" descr="Status marker"/></xdr:nvPicPr><xdr:blipFill><a:blip r:embed="rId2"/></xdr:blipFill></xdr:pic></xdr:oneCellAnchor></xdr:wsDr>"#;
        let references =
            drawing_references(drawing, "xl/drawings/drawing1.xml", &Limits::default()).unwrap();
        assert_eq!(references.len(), 1);
        assert_eq!(references[0].cell.as_deref(), Some("D2"));
        assert_eq!(references[0].width_px, Some(144));
        assert_eq!(references[0].height_px, Some(144));
    }

    #[test]
    fn spreadsheet_styles_keep_safe_fill_and_font_emphasis() {
        let styles = br#"<styleSheet xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main"><fonts count="2"><font><sz val="11"/><color rgb="FF112233"/></font><font><b/><i/><color rgb="FF445566"/></font></fonts><fills count="2"><fill><patternFill patternType="none"/></fill><fill><patternFill patternType="solid"><fgColor rgb="FFAABBCC"/></patternFill></fill></fills><cellXfs count="2"><xf fontId="0" fillId="0"/><xf fontId="1" fillId="1"><alignment horizontal="center" vertical="center" wrapText="1"/></xf></cellXfs></styleSheet>"#;
        let styles = spreadsheet_styles(styles, "xl/styles.xml", &Limits::default()).unwrap();
        assert_eq!(styles.len(), 2);
        assert_eq!(styles[0].font_color.as_deref(), Some("#112233"));
        assert_eq!(styles[1].fill_color.as_deref(), Some("#AABBCC"));
        assert_eq!(styles[1].font_color.as_deref(), Some("#445566"));
        assert!(styles[1].bold);
        assert!(styles[1].italic);
        assert_eq!(styles[1].horizontal_alignment.as_deref(), Some("center"));
        assert_eq!(styles[1].vertical_alignment.as_deref(), Some("center"));
        assert!(styles[1].wrap_text);
    }

    #[test]
    fn worksheet_geometry_reads_custom_widths_heights_and_hidden_sizes() {
        let sheet = br#"<worksheet xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main"><sheetFormatPr defaultColWidth="8.43" defaultRowHeight="15"/><cols><col min="1" max="2" width="18.5" customWidth="1"/><col min="3" max="3" width="9" hidden="1"/></cols><sheetData><row r="1" ht="30" customHeight="1"><c r="A1"><v>1</v></c></row><row r="2" ht="15" hidden="1" /></sheetData><mergeCells count="1"><mergeCell ref="A1:C1"/></mergeCells></worksheet>"#;
        let geometry = sheet_geometry(
            sheet,
            "xl/worksheets/sheet1.xml",
            &Limits::default(),
            "Sheet1",
        )
        .unwrap();
        assert_eq!(geometry.column_widths.get("Sheet1!A"), Some(&135));
        assert_eq!(geometry.column_widths.get("Sheet1!B"), Some(&135));
        assert_eq!(geometry.column_widths.get("Sheet1!C"), Some(&0));
        assert_eq!(geometry.row_heights.get("Sheet1!1"), Some(&40));
        assert_eq!(geometry.row_heights.get("Sheet1!2"), Some(&0));
        assert_eq!(geometry.default_column_widths.get("Sheet1"), Some(&64));
        assert_eq!(geometry.default_row_heights.get("Sheet1"), Some(&20));
        assert_eq!(geometry.merged_ranges.len(), 1);
        assert_eq!(geometry.merged_ranges[0].sheet_anchor, "Sheet1!");
        assert_eq!(geometry.merged_ranges[0].range, "A1:C1");
    }

    #[test]
    fn styled_sheet_cells_keep_readable_value_and_style_separately() {
        let sheet = br#"<worksheet xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main"><sheetData><row r="1"><c r="A1" s="1" t="inlineStr"><is><t>Daily total</t></is></c></row></sheetData></worksheet>"#;
        let style_defs = vec![
            CellStyle::default(),
            CellStyle {
                fill_color: Some("#AABBCC".into()),
                font_color: Some("#112233".into()),
                bold: true,
                italic: false,
                number_format: None,
                horizontal_alignment: None,
                vertical_alignment: None,
                wrap_text: false,
            },
        ];
        let rows = sheet_rows(
            sheet,
            "xl/worksheets/sheet1.xml",
            &Limits::default(),
            &[],
            &style_defs,
            false,
            &mut 0,
        )
        .unwrap();
        assert_eq!(rows[0].cells, vec![(1, "Daily total".into())]);
        assert_eq!(rows[0].cell_styles, vec![(1, style_defs[1].clone())]);
    }

    #[test]
    fn spreadsheet_table_style_projects_range_and_banding() {
        let table = br#"<table xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main" name="Daily" displayName="Daily" ref="A1:B3"><tableStyleInfo name="TableStyleMedium2" showRowStripes="1" showColumnStripes="0"/></table>"#;
        assert_eq!(
            spreadsheet_table_info(table, "xl/tables/table1.xml", &Limits::default()).unwrap(),
            Some((
                "Daily".into(),
                "A1:B3".into(),
                (Some("TableStyleMedium2".into()), true, false)
            ))
        );
    }

    #[test]
    fn column_names_round_trip() {
        for (column, name) in [(1, "A"), (26, "Z"), (27, "AA"), (52, "AZ"), (703, "AAA")] {
            assert_eq!(column_name(column), name);
            assert_eq!(column_of(&format!("{name}12")), Some(column));
        }
    }

    #[test]
    fn sheet_names_are_quoted_when_needed() {
        assert_eq!(quote_sheet("Budget"), "Budget");
        assert_eq!(quote_sheet("Q1 plan"), "'Q1 plan'");
        assert_eq!(quote_sheet("Bob's"), "'Bob''s'");
    }

    #[test]
    fn chart_cached_values_are_projected_as_a_labeled_table() {
        let chart = br#"<c:chartSpace xmlns:c="http://schemas.openxmlformats.org/drawingml/2006/chart" xmlns:a="http://schemas.openxmlformats.org/drawingml/2006/main"><c:chart><c:title><c:tx><c:rich><a:p><a:r><a:t>Monthly sales</a:t></a:r></a:p></c:rich></c:tx></c:title><c:plotArea><c:barChart><c:ser><c:tx><c:strRef><c:strCache><c:pt idx="0"><c:v>Revenue</c:v></c:pt></c:strCache></c:strRef></c:tx><c:cat><c:strRef><c:strCache><c:pt idx="0"><c:v>Jan</c:v></c:pt><c:pt idx="1"><c:v>Feb</c:v></c:pt></c:strCache></c:strRef></c:cat><c:val><c:numRef><c:numCache><c:pt idx="0"><c:v>120</c:v></c:pt><c:pt idx="1"><c:v>145</c:v></c:pt></c:numCache></c:numRef></c:val></c:ser></c:barChart></c:plotArea></c:chart></c:chartSpace>"#;
        let table = chart_table(chart, "xl/charts/chart1.xml", &Limits::default())
            .unwrap()
            .unwrap();
        assert_eq!(table.anchor, "chart:chart1.xml");
        assert_eq!(table.title, "Monthly sales");
        assert_eq!(
            table.rows,
            vec![
                vec!["Category", "Revenue"],
                vec!["Jan", "120"],
                vec!["Feb", "145"],
            ]
        );
        assert_eq!(
            table.labels,
            vec![
                "chart data; cached values",
                "chart title: Monthly sales",
                "chart type: column",
                "chart series type 0: column",
            ]
        );
    }

    #[test]
    fn combo_chart_keeps_each_series_chart_type() {
        let chart = br#"<c:chartSpace xmlns:c="http://schemas.openxmlformats.org/drawingml/2006/chart" xmlns:a="http://schemas.openxmlformats.org/drawingml/2006/main"><c:chart><c:plotArea><c:barChart><c:barDir val="col"/><c:ser><c:tx><c:strRef><c:strCache><c:pt idx="0"><c:v>Sales</c:v></c:pt></c:strCache></c:strRef></c:tx><c:cat><c:strRef><c:strCache><c:pt idx="0"><c:v>Jan</c:v></c:pt></c:strCache></c:strRef></c:cat><c:val><c:numRef><c:numCache><c:pt idx="0"><c:v>10</c:v></c:pt></c:numCache></c:numRef></c:val></c:ser></c:barChart><c:lineChart><c:ser><c:tx><c:strRef><c:strCache><c:pt idx="0"><c:v>Margin</c:v></c:pt></c:strCache></c:strRef></c:tx><c:cat><c:strRef><c:strCache><c:pt idx="0"><c:v>Jan</c:v></c:pt></c:strCache></c:strRef></c:cat><c:val><c:numRef><c:numCache><c:pt idx="0"><c:v>2</c:v></c:pt></c:numCache></c:numRef></c:val></c:ser></c:lineChart></c:plotArea></c:chart></c:chartSpace>"#;
        let table = chart_table(chart, "xl/charts/chart2.xml", &Limits::default())
            .unwrap()
            .unwrap();
        assert_eq!(
            table.rows,
            vec![vec!["Category", "Sales", "Margin"], vec!["Jan", "10", "2"]]
        );
        assert!(
            table
                .labels
                .iter()
                .any(|label| label == "chart series type 0: column")
        );
        assert!(
            table
                .labels
                .iter()
                .any(|label| label == "chart series type 1: line")
        );
    }

    #[test]
    fn chart_cache_point_indices_keep_gaps_aligned_to_categories() {
        let chart = br#"<c:chartSpace xmlns:c="http://schemas.openxmlformats.org/drawingml/2006/chart"><c:chart><c:plotArea><c:barChart><c:ser><c:tx><c:v>Visits</c:v></c:tx><c:cat><c:strRef><c:strCache><c:pt idx="0"><c:v>Mon</c:v></c:pt><c:pt idx="1"><c:v>Tue</c:v></c:pt><c:pt idx="2"><c:v>Wed</c:v></c:pt></c:strCache></c:strRef></c:cat><c:val><c:numRef><c:numCache><c:pt idx="0"><c:v>12</c:v></c:pt><c:pt idx="2"><c:v>18</c:v></c:pt></c:numCache></c:numRef></c:val></c:ser></c:barChart></c:plotArea></c:chart></c:chartSpace>"#;
        let table = chart_table(chart, "xl/charts/chart-gaps.xml", &Limits::default())
            .unwrap()
            .unwrap();
        assert_eq!(
            table.rows,
            vec![
                vec!["Category", "Visits"],
                vec!["Mon", "12"],
                vec!["Tue", ""],
                vec!["Wed", "18"],
            ]
        );
    }

    #[test]
    fn chart_cache_point_indices_are_bounded_and_report_truncation() {
        let chart = br#"<c:chartSpace xmlns:c="http://schemas.openxmlformats.org/drawingml/2006/chart"><c:chart><c:plotArea><c:barChart><c:ser><c:tx><c:v>Visits</c:v></c:tx><c:cat><c:strRef><c:strCache><c:pt idx="0"><c:v>Mon</c:v></c:pt><c:pt idx="10000"><c:v>Far future</c:v></c:pt></c:strCache></c:strRef></c:cat><c:val><c:numRef><c:numCache><c:pt idx="0"><c:v>12</c:v></c:pt><c:pt idx="10000"><c:v>18</c:v></c:pt></c:numCache></c:numRef></c:val></c:ser></c:barChart></c:plotArea></c:chart></c:chartSpace>"#;
        let table = chart_table(chart, "xl/charts/chart-bounded.xml", &Limits::default())
            .unwrap()
            .unwrap();
        assert_eq!(
            table.rows,
            vec![vec!["Category", "Visits"], vec!["Mon", "12"]]
        );
        assert!(table.labels.iter().any(|label| {
            label == &format!("chart cache truncated after {MAX_CHART_POINTS} points")
        }));
    }
}
