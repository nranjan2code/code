//! `office_apply`: propose edits to, or create, a Word, Excel or PowerPoint
//! file through the typed op set (docs/design/72-openxml-documents.md, P2).
//!
//! It runs in the broker worker (invariant 14). An edited source must be
//! the exact file the model read (`base_digest`, O4); a new file with no
//! source starts from Vakyartha's built-in blank ("Creating from scratch").
//! Every op is checked by a re-read before anything is written (O5), and
//! Word edits to an existing file are tracked changes under the runtime's
//! Agent id, never a name the model chose.
//!
//! It never writes the workspace file. The result is a draft in this
//! execution's `.vak/scratch/<agent>/<execution>/` directory, announced to
//! the Workbench like any other execution, so the one Review path (freeze a
//! candidate, verify it in the worker, promote atomically with undo) is how
//! a change reaches the workspace (invariant 35; docs/design/72, "Owner
//! direction").

use std::path::{Path, PathBuf};
use std::sync::LazyLock;

use async_trait::async_trait;
use serde_json::Value;

use crate::broker::OfficeOrigin;
use crate::{ResourceClaims, Tool, ToolContext, ToolOutput};

/// What a model is told: how to create a file from scratch, from a
/// template, and how to edit one. The blank's styles, sheet and layouts
/// come from the blank itself, since there is nothing to read first.
/// The shared contract only. What each op does, and each format's styles,
/// layouts and sheet, is in the op schemas, which the model reads beside it:
/// repeating per-format detail here made this one tool about four times the
/// size of the whole system prompt.
static DESCRIPTION: LazyLock<String> = LazyLock::new(|| {
    "Create or edit Word, Excel, PowerPoint and PDF files with typed ops; the result is a draft for human Review, not a changed workspace file. Read each op's schema for its format, required fields, examples and limits. \
     New file: give a new .docx, .xlsx, .pptx or .pdf name and ops; omit source and base_digest. A workspace template: source is the template, base_digest its sha256, path the new file. Existing file: read with doc_read first; use its exact anchors/names and sha256 (the printed 16-character prefix is enough). To continue a successful draft, use its returned draft path as source and its digest. \
     Ops run in order and the call is atomic: any op or read-back failure means nothing was written. Retry from the same source or blank with corrected ops; never invent a source/digest for a failed draft. Use source-read anchors for existing paragraphs, shapes and PDF lines/pages; do not guess anchors minted by earlier ops. Create/rename sheets before referencing their final names. Claim a draft only after a successful result returns its path. \
     Word text is plain; use styles for headings/lists. Vak does not calculate Excel formulas; use numeric values for chart sources in new workbooks, not newly written formulas. PDF writing uses Helvetica and Latin/WinAnsi text: offer Word before starting if the text needs Devanagari, Arabic, CJK or other unsupported characters. Images require base64 PNG/JPEG bytes, descriptive alt_text and at most 1 MiB. Macros are never added or run. \
     Visio is read-only. Legacy binary Office/Visio, .xlsb, OpenDocument and encrypted files are unsupported. Do not substitute a command/script or another file format without explaining the limitation to the person."
        .to_string()
});

pub struct OfficeApplyTool;

#[async_trait]
impl Tool for OfficeApplyTool {
    fn name(&self) -> &str {
        "office_apply"
    }

    fn serves(&self) -> &'static [&'static str] {
        &["documents"]
    }

    fn delivered_file(&self, args: &Value) -> Option<String> {
        args.get("path").and_then(Value::as_str).map(str::to_string)
    }

    /// The document a call edits from: its `source`, or `path` when the call
    /// names a `base_digest` of the existing file. The draft it writes is in
    /// scratch, not the workspace; the workspace file changes only through
    /// Review, which records that write itself.
    fn file_access(&self, args: &Value) -> Option<(crate::FileAccess, String)> {
        let source = args.get("source").and_then(Value::as_str);
        let edited = args
            .get("base_digest")
            .and_then(Value::as_str)
            .and(args.get("path").and_then(Value::as_str));
        source
            .or(edited)
            .map(|path| (crate::FileAccess::Read, path.to_string()))
    }

    fn description(&self) -> &str {
        DESCRIPTION.as_str()
    }

    fn schema(&self) -> Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "path": {
                    "type": "string",
                    "description": "The workspace file the draft is for (relative to the workspace): an existing file to edit, or a new name to create. May equal source; path cannot be inside .vak (an earlier draft belongs in source)."
                },
                "source": {
                    "type": "string",
                    "description": "File to start from, with the same document format as path: the file being edited, a template, or an earlier draft. Defaults to path. Leave out, with base_digest, to create path from scratch."
                },
                "base_digest": {
                    "type": "string",
                    "description": "The sha256 exactly as doc_read printed it for the source file (its first 16 characters, as shown, are enough; never fill in the rest), proving the ops were written against its current content. Leave out only when creating a new file from scratch."
                },
                "ops": {
                    "type": "array",
                    "minItems": 1,
                    "description": "Ops applied in order; any failure writes nothing. Open XML Review offers individual choices for up to 100 ops per draft; PDF allows at most 200 ops per call. Each names its `op` and only that op's fields. Anchors come from doc_read of the source: paragraphs p:1A2B3C4D or p@12, cells Budget!B4, slides slide:256, shapes slide:256/shape:3, placeholders slide:256/placeholder:title.",
                    "items": { "oneOf": op_schemas() }
                }
            },
            "required": ["path", "ops"],
            "examples": [
                {"path":"report.docx","ops":[
                    {"op":"add_paragraph","text":"Demonstration report","style":"Title"},
                    {"op":"add_table","rows":[["Region","Value"],["North",120],["South",95.5]]}
                ]},
                {"path":"dashboard.xlsx","ops":[
                    {"op":"rename_sheet","sheet":"Sheet1","name":"Market Data"},
                    {"op":"append_rows","sheet":"Market Data","rows":[["Index","Change"],["Demo A",1.25],["Demo B",-0.42]]},
                    {"op":"add_excel_table","sheet":"Market Data","range":"A1:B3"},
                    {"op":"add_chart","sheet":"Market Data","range":"A1:B3","chart_type":"bar","title":"Demonstration change","cell":"D2"}
                ]},
                {"path":"slides.pptx","ops":[
                    {"op":"add_slide_from_layout","layout":"Title and Content","placeholders":{"title":"Demonstration chart"},"charts":{"body":{"title":"Value","chart_type":"bar","categories":["North","South"],"values":[120,95.5]}}}
                ]},
                {"path":"report.pdf","ops":[
                    {"op":"add_paragraph","text":"Demonstration report","style":"Title"},
                    {"op":"add_chart","title":"Value","categories":["North","South"],"values":[120,95.5]}
                ]}
            ]
        })
    }

    fn claims(&self, args: &Value) -> ResourceClaims {
        ResourceClaims {
            exclusive: false,
            read_only: false,
            paths: ["path", "source"]
                .iter()
                .filter_map(|key| args.get(*key).and_then(Value::as_str))
                .map(str::to_string)
                .collect(),
        }
    }

    async fn execute(&self, args: &Value, ctx: &ToolContext) -> ToolOutput {
        let Some(path) = args.get("path").and_then(Value::as_str).map(str::trim) else {
            return ToolOutput::error("missing required parameter: path");
        };
        let named_source = args
            .get("source")
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|source| !source.is_empty());
        let base_digest = args
            .get("base_digest")
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|digest| !digest.is_empty());
        let Some(raw_ops) = args.get("ops") else {
            return ToolOutput::error("missing required parameter: ops");
        };
        let pdf = vak_pdf::is_pdf_path(path);
        // Checked first: a text file sent here once got "pass the sha256
        // doc_read prints as base_digest", a digest doc_read never prints
        // for text, and the call looped on it (measured live).
        let target = std::path::Path::new(path)
            .extension()
            .and_then(|extension| extension.to_str())
            .and_then(vak_ooxml::Format::from_extension);
        if target.is_none() && !pdf {
            return ToolOutput::error(format!(
                "{path} is not a Word, Excel, PowerPoint or PDF file (.docx, .xlsx, .pptx and their variants, or .pdf), so office_apply cannot change it. Change a text file such as Markdown, code or data with edit, or rewrite it whole with write."
            ));
        }
        if let Some(error) = validate_chart_shapes(raw_ops, pdf) {
            return ToolOutput::error(error);
        }
        let (ops, pdf_ops): (Vec<vak_ooxml::edit::OfficeOp>, Vec<vak_pdf::edit::PdfOp>) = if pdf {
            match serde_json::from_value(raw_ops.clone()) {
                Ok(ops) => (Vec::new(), ops),
                Err(error) => {
                    return ToolOutput::error(format!(
                        "ops are not valid for a PDF: {error}. A PDF takes {}; each op is an object with an \"op\" name and only that op's fields",
                        crate::office_pdf::OPS
                    ));
                }
            }
        } else {
            match serde_json::from_value(raw_ops.clone()) {
                Ok(ops) => (ops, Vec::new()),
                Err(error) => {
                    return ToolOutput::error(format!(
                        "ops are not valid: {error}. Each op is an object with an \"op\" name and only that op's fields"
                    ));
                }
            }
        };
        let destination = match confined_destination(&ctx.cwd, path) {
            Ok(destination) => destination,
            Err(error) => return ToolOutput::error(error),
        };
        // One way to create: a new path with no source and no digest starts
        // from the built-in blank. A digest for a file that does not exist
        // means the model thought it was editing one (a mistyped name), so
        // it is never taken as a request to create.
        let origin = match (named_source, base_digest) {
            (None, None) if destination.exists() => {
                return ToolOutput::error(format!(
                    "{path} already exists. To change it, read it with doc_read and pass the sha256 it prints as base_digest; to create a new file from scratch, give a name that does not exist yet"
                ));
            }
            (None, None) => OfficeOrigin::Blank,
            (Some(source), None) => {
                return ToolOutput::error(format!(
                    "missing base_digest for source {source}: read it with doc_read and pass the sha256 it prints. To create a new file from scratch, leave out source as well"
                ));
            }
            (source, Some(digest)) => {
                let source = source.unwrap_or(path);
                match crate::drafts::existing(ctx, source) {
                    Ok(source) => OfficeOrigin::File {
                        path: source,
                        base_digest: digest.to_string(),
                    },
                    Err(_) if source == path && !destination.exists() => {
                        return ToolOutput::error(format!(
                            "{path} does not exist, so there is no file for base_digest to name. To create it from scratch, leave out base_digest; to edit a file, check its name"
                        ));
                    }
                    Err(error) => return ToolOutput::error(error),
                }
            }
        };
        let root = match canonical_root(&ctx.cwd) {
            Ok(root) => root,
            Err(error) => return ToolOutput::error(error),
        };
        let Ok(relative) = destination.strip_prefix(&root).map(Path::to_path_buf) else {
            return ToolOutput::error(format!("access denied: {path} is outside the workspace"));
        };
        if relative.starts_with(vak_config::scope::PROJECT_DIR) {
            return ToolOutput::error(format!(
                "{path} is inside .vak; name the workspace file the draft is for, and pass an earlier draft as source"
            ));
        }
        let agent = ctx.agent_id.as_deref();
        let author = tracked_change_author(agent.unwrap_or(DEFAULT_AGENT));
        let now = chrono::Utc::now();
        let date = now.format("%Y-%m-%dT%H:%M:%SZ").to_string();
        let execution = ctx
            .sandbox_sink
            .as_ref()
            .map(|sink| sink.execution_id().to_string())
            .unwrap_or_else(|| format!("office-{}", now.timestamp_nanos_opt().unwrap_or(0)));
        let draft_root = crate::drafts::dir(ctx, &execution);
        let draft = draft_root.join(&relative);
        let draft_relative = crate::drafts::name(ctx, &execution, &relative);
        let started = std::time::Instant::now();
        if let Some(sink) = &ctx.sandbox_sink {
            let preview = serde_json::json!({
                "path": path,
                "source": named_source.unwrap_or(if origin == OfficeOrigin::Blank { "blank" } else { path }),
                "ops": args.get("ops"),
            })
            .to_string();
            sink.emit_execution_started(
                "office_apply",
                &preview,
                "json",
                &draft_root.display().to_string(),
            );
        }
        // A file not in the workspace yet is a new document, written clean;
        // an existing one is edited with tracked changes (docs/design/72, R7).
        let relative_path = relative.to_string_lossy().replace('\\', "/");
        let exists = destination.is_file();
        let tracked = exists && !ctx.new_documents.contains(&relative_path);
        let work = match target {
            Some(target) => {
                let job = Job {
                    origin,
                    destination,
                    draft: draft.clone(),
                    ops,
                    context: vak_ooxml::edit::EditContext {
                        author,
                        date,
                        tracked,
                    },
                    target,
                };
                tokio::task::spawn_blocking(move || job.run()).await
            }
            None => {
                let job = crate::office_pdf::Job {
                    origin,
                    destination,
                    draft: draft.clone(),
                    ops: pdf_ops,
                    context: vak_pdf::edit::EditContext { author, date },
                };
                tokio::task::spawn_blocking(move || job.run()).await
            }
        };
        let duration = started.elapsed().as_millis() as u64;
        let finish = |code: i32, artifacts: Vec<String>| {
            if let Some(sink) = &ctx.sandbox_sink {
                sink.emit_finished(code, duration, artifacts);
            }
        };
        match work {
            Ok(Ok(report)) => {
                crate::artifact::emit_file(ctx.sandbox_sink.as_ref(), &draft, &root);
                finish(0, vec![draft_relative.clone()]);
                let state = if exists {
                    format!("{path} in the workspace is unchanged")
                } else {
                    format!("{path} is a new file, not in the workspace")
                };
                ToolOutput::ok(format!(
                    "Draft for {path} written to {draft_relative}. {state} until a person reviews and accepts the draft: the draft is already shown to them with Review draft and its change list, so do not present it again as a card, HTML or a diff, and never copy, move or rename the draft into the workspace yourself: that would skip the person's review. Answer with one sentence saying what you changed, and stop. To keep editing, call office_apply again with source \"{draft_relative}\" and the draft's sha256 as base_digest.\n{report}"
                ))
            }
            Ok(Err(error)) => {
                finish(1, Vec::new());
                ToolOutput::error(error)
            }
            Err(error) => {
                finish(1, Vec::new());
                ToolOutput::error(format!("office_apply failed: {error}"))
            }
        }
    }
}

fn validate_chart_shapes(ops: &Value, pdf: bool) -> Option<String> {
    let ops = ops.as_array()?;
    for (index, op) in ops.iter().enumerate() {
        if op.get("op").and_then(Value::as_str) != Some("add_chart") {
            continue;
        }
        let has_pdf = op.get("categories").is_some() || op.get("values").is_some();
        let has_excel = op.get("sheet").is_some()
            || op.get("range").is_some()
            || op.get("chart_type").is_some();
        if pdf && (!has_pdf || has_excel) {
            return Some(format!(
                "arguments.ops[{index}] for a PDF chart needs categories and values and cannot use sheet, range or chart_type"
            ));
        }
        if !pdf && (!has_excel || has_pdf || op.get("after").is_some()) {
            return Some(format!(
                "arguments.ops[{index}] for an Excel chart needs sheet, range and chart_type and cannot use categories, values or after"
            ));
        }
    }
    None
}

/// The refusal a text tool gives for an Office file or a PDF: a package is
/// a ZIP of XML parts and a PDF a binary object graph, so a text read shows
/// nothing useful and a text edit or write can only fail or destroy one.
/// Names the tools that do handle it, for a file that exists and for one
/// still to be made: told only to read it first, a model asked to create a
/// document read a file that was not there and stopped (live, the M8
/// acceptance run on Ollama).
pub fn text_tool_refusal(path: &Path, tool: &str) -> Option<String> {
    let name = path.to_string_lossy();
    if vak_pdf::is_pdf_path(&name) {
        let display = path
            .file_name()
            .map(|file| file.to_string_lossy().into_owned())
            .unwrap_or_else(|| name.into_owned());
        return Some(match tool {
            "read" => format!(
                "{display} is a PDF, which {tool} cannot show. Read it with doc_read, which returns its text by page and line with anchors and a sha256."
            ),
            _ => format!(
                "{display} is a PDF, which {tool} would corrupt; nothing was changed. Use office_apply, which writes a draft for review: to create {display}, give its path and the ops that add its content; to change one that exists, read it with doc_read first."
            ),
        });
    }
    if !vak_ooxml::is_openxml_path(&name) {
        return None;
    }
    let what = path
        .extension()
        .and_then(|extension| extension.to_str())
        .and_then(vak_ooxml::Format::from_extension)
        .map(|format| format.vocabulary.with_article())
        .unwrap_or("an Office file");
    let display = path
        .file_name()
        .map(|file| file.to_string_lossy().into_owned())
        .unwrap_or_else(|| name.into_owned());
    Some(match tool {
        "read" => format!(
            "{display} is {what}, a ZIP package that {tool} cannot show. Read it with doc_read, which returns its text with anchors and a sha256."
        ),
        _ => format!(
            "{display} is {what}, a ZIP package that {tool} would corrupt; nothing was changed. Use office_apply, which writes a draft for review: to create {display}, give its path and the ops that add its content; to change one that exists, read it with doc_read first."
        ),
    })
}

/// The Agent a draft is filed under when the session names none.
const DEFAULT_AGENT: &str = "vak";

/// The name of one execution's drafts: `.vak/scratch/<agent>/<execution>/`,
/// the draft of a file keeping the file's workspace path beneath it. The
/// execution is the `office_apply` call's id and the agent is the session's
/// Agent id, so the ledger alone names every draft a session delivered; the
/// files live in the runtime root (`vak_config::scope::draft_location`).
pub fn draft_dir(agent_id: Option<&str>, execution: &str) -> PathBuf {
    Path::new(vak_config::scope::PROJECT_DIR)
        .join("scratch")
        .join(agent_id.unwrap_or(DEFAULT_AGENT))
        .join(execution)
}

/// The name Word shows on an Agent's tracked changes: the runtime's Agent
/// id, never a name the model chose.
pub fn tracked_change_author(agent_id: &str) -> String {
    if agent_id == "vak" {
        "Vakyartha".to_string()
    } else {
        agent_id.to_string()
    }
}

/// One schema branch per op, so a model sees each op's exact fields and a
/// malformed op is reported against the op it names (`contract.rs`).
fn op_schemas() -> Value {
    fn op(name: &str, fields: Value, required: &[&str], description: &str) -> Value {
        let mut properties = serde_json::json!({ "op": { "type": "string", "enum": [name] } });
        if let (Some(target), Some(extra)) = (properties.as_object_mut(), fields.as_object()) {
            target.extend(extra.clone());
        }
        let mut all_required = vec!["op"];
        all_required.extend_from_slice(required);
        let mut schema = serde_json::json!({
            "type": "object",
            "description": description,
            "properties": properties,
            "required": all_required,
            "additionalProperties": false
        });
        if name == "add_chart" {
            // The chart variants have different required fields. Keep them
            // explicit so incomplete or mixed PDF/Excel calls fail at admission.
            let variants = [
                (
                    vec!["op", "title", "sheet", "range", "chart_type", "cell"],
                    vec!["op", "title", "sheet", "range", "chart_type"],
                ),
                (
                    vec!["op", "title", "categories", "values", "after"],
                    vec!["op", "title", "categories", "values"],
                ),
            ];
            schema["oneOf"] = Value::Array(variants.into_iter().map(|(allowed, required)| {
                let properties = allowed.into_iter().map(|key| {
                    let field = if key == "op" { serde_json::json!({"type":"string"}) } else { schema["properties"][key].clone() };
                    (key.to_string(), field)
                }).collect::<serde_json::Map<String, Value>>();
                serde_json::json!({"type":"object","properties":properties,"required":required,"additionalProperties":false})
            }).collect());
        }
        schema
    }
    let anchor = |what: &str| serde_json::json!({ "type": "string", "description": what });
    let text = serde_json::json!({ "type": "string" });
    let lines = serde_json::json!({
        "type": ["string", "array"],
        "items": { "type": "string" },
        "description": "One line, or an array of lines (one bullet or paragraph each)"
    });
    serde_json::json!([
        op(
            "replace_paragraph_text",
            serde_json::json!({ "anchor": anchor("paragraph anchor, e.g. p@12; in a PDF, a line, e.g. page:3/line:12"), "text": text }),
            &["anchor", "text"],
            "Word: give the paragraph's whole new text, as it should read. Only the words that differ become tracked changes; its formatting, links, footnote marks and fields stay as they are. PDF: replace one visible page line using Helvetica at the old position/size; it may not match the source font, and lines inside form XObjects cannot be replaced. Only WinAnsi/Latin text can be written."
        ),
        op(
            "add_paragraph",
            serde_json::json!({ "text": text, "style": { "type": "string", "description": format!("style id or name; a new document offers {}", vak_ooxml::blank::DOCUMENT_STYLES.join(", ")) }, "after": anchor("paragraph anchor to add after (in a PDF, a page, e.g. page:3); omit to add at the end of the document") }),
            &["text"],
            "Word: add plain text using a source style; Markdown is not interpreted. A List Number paragraph continues the list just above it, else starts at 1. PDF: add Latin/WinAnsi text in Helvetica on new A4 pages after a source page or at the end; it does not reflow existing page content."
        ),
        op(
            "add_table",
            serde_json::json!({ "rows": { "type": "array", "items": { "type": "array" }, "description": "rows of cell text, the first row being the header, e.g. [[\"Region\", \"Sales\"], [\"North\", \"120\"]]" }, "after": anchor("paragraph anchor to add after; omit to add at the end"), "header": { "type": "boolean", "description": "false when the first row is not a header row; default true" } }),
            &["rows"],
            "Word: add a rectangular table spanning the page width (up to 1,000 rows by 63 columns). Cell values are text, numbers or booleans, never null or objects. PDF: add a searchable table on new A4 page content; it may flow across pages. The first row is a header unless header is false."
        ),
        op(
            "add_image",
            serde_json::json!({ "image": { "type": "object", "properties": { "mime_type": { "type": "string", "enum": ["image/png", "image/jpeg"] }, "data": { "type": "string", "description": "base64-encoded PNG/JPEG file bytes (no URL, path or data-URL prefix); decoded image at most 1 MiB, 10,000 pixels per side and 40 megapixels total" }, "alt_text": { "type": "string", "minLength": 1, "maxLength": 2048, "description": "required meaningful alternative text, at most 2,048 UTF-8 bytes" } }, "required": ["mime_type", "data", "alt_text"], "additionalProperties": false }, "after": anchor("paragraph anchor to add the inline image after; omit to add at the end") }),
            &["image"],
            "Word: add an inline PNG/JPEG with required alternative text. PDF: add a PNG/JPEG to new page content with a searchable alternative-text caption. Only PNG/JPEG images up to 1 MiB are accepted. Image pixels are not OCR-read by doc_read."
        ),
        op(
            "add_chart",
            serde_json::json!({ "title": text, "categories": { "type": "array", "items": { "type": "string" }, "description": "PDF only: required with values; 1–20 non-empty short labels" }, "values": { "type": "array", "items": { "type": "number" }, "description": "PDF only: same number of values as categories; finite numbers from -1e12 to 1e12" }, "sheet": { "type": "string", "description": "Excel only: required with range and chart_type; exact existing worksheet name containing the source data, not a table name" }, "range": { "type": "string", "description": "Excel only: an A1 range without the sheet name, e.g. A1:B4; exactly two columns (category labels, then one numeric series), a header row and 2–1,000 data rows. Values must read as numbers; formulas added or changed in this draft have stale/missing results and cannot be charted until recalculated and saved in Excel." }, "chart_type": { "type": "string", "enum": ["bar", "line", "pie"], "description": "Excel only: bar, line or pie; one numeric series per chart" }, "cell": { "type": "string", "description": "Excel only: exact top-left anchor cell, e.g. D2, outside the entire source range and clear of other drawings. Omit to place below the range." }, "after": anchor("PDF only: page anchor, e.g. page:3; omit to add at the end") }),
            &["title"],
            "PDF: categories and values make a vector bar chart plus searchable data table (1–20 categories). Excel: sheet, range and chart_type make a native chart with one numeric series. Title must be non-empty (Excel: at most 200 characters). Excel chart creation on Strict workbooks is unsupported. PowerPoint charts use the charts map in add_slide_from_layout."
        ),
        op(
            "delete_paragraph",
            serde_json::json!({ "anchor": anchor("paragraph anchor; in a PDF, a line, e.g. page:3/line:12") }),
            &["anchor"],
            "Word: delete a paragraph (a tracked change in an existing file; removed outright in a new one). PDF: remove a line from its page"
        ),
        op(
            "set_cells",
            serde_json::json!({ "sheet": { "type": "string", "description": "exact existing worksheet name" }, "cells": { "type": "object", "description": "address to text, number, boolean or formula; formulas begin with =, and prefix an apostrophe to store text beginning with =. Vak does not calculate formulas. Use an empty string to clear a cell; omit untouched cells. Null, arrays and objects are invalid cell values.", "additionalProperties": { "oneOf": [{"type":"string"},{"type":"number"},{"type":"boolean"}] } } }),
            &["sheet", "cells"],
            "Excel: set cell values or formulas. Newly added or changed formula results remain stale or unavailable until Excel recalculates and saves the workbook; chart only formula results that doc_read confirms are current numeric cached values."
        ),
        op(
            "append_rows",
            serde_json::json!({ "sheet": { "type": "string", "description": "exact existing worksheet name" }, "rows": { "type": "array", "description": "rectangular rows of text, numbers, booleans or formulas; formulas are not calculated by Vak", "items": { "type": "array", "items": {"oneOf":[{"type":"string"},{"type":"number"},{"type":"boolean"}]} } } }),
            &["sheet", "rows"],
            "Excel: append rows after the last used row; keep row widths consistent with the table header and data. Formula results are not calculated by Vak."
        ),
        op(
            "add_sheet",
            serde_json::json!({ "name": { "type": "string" } }),
            &["name"],
            "Excel: add an empty sheet with a unique name, 1–31 characters; no [ ] : * ? / or backslash, and no leading/trailing apostrophe"
        ),
        op(
            "rename_sheet",
            serde_json::json!({ "sheet": { "type": "string" }, "name": { "type": "string", "description": "the new name" } }),
            &["sheet", "name"],
            &format!(
                "Excel: rename a sheet, before anything refers to it by name. A new workbook has one empty sheet, {}",
                vak_ooxml::blank::WORKBOOK_SHEET
            )
        ),
        op(
            "format_cells",
            serde_json::json!({ "sheet": { "type": "string" }, "range": { "type": "string", "description": "cells, e.g. A1:D1 or B4" }, "bold": { "type": "boolean" }, "italic": { "type": "boolean" }, "number_format": { "type": "string", "description": "an Excel format code, e.g. #,##0.00 or 0% or yyyy-mm-dd" }, "fill": { "type": "string", "description": "background colour as six hex digits, e.g. D9E2F3" }, "wrap": { "type": "boolean", "description": "wrap long text" } }),
            &["sheet", "range"],
            "Excel: format cells using only the offered fields (bold, italic, number_format, fill, wrap); every other part of each cell's format is kept. No borders, alignment, font selection, merged cells or conditional formatting can be authored by this op"
        ),
        op(
            "set_column_widths",
            serde_json::json!({ "sheet": { "type": "string" }, "widths": { "type": "object", "description": "column letter to finite width greater than 0 and at most 255 characters, e.g. {\"A\": 30, \"B\": 12}" } }),
            &["sheet", "widths"],
            "Excel: set column widths"
        ),
        op(
            "add_excel_table",
            serde_json::json!({ "sheet": { "type": "string" }, "range": { "type": "string", "description": "a populated header row and 1–10,000 data rows, 1–64 columns, e.g. A1:C20; headers must be non-empty, unique ignoring case, at most 255 characters; cannot overlap another table" }, "name": { "type": "string", "description": "optional unique table name: starts with a letter or underscore; only letters, digits, underscores or periods. Omit to generate a name; this names the table, not the sheet." } }),
            &["sheet", "range"],
            "Excel: Strict workbook table creation is unsupported. Turn a header and data range into a native, filterable table (up to 10,000 data rows and 64 columns); rows remain available to doc_read and RAG"
        ),
        op(
            "add_excel_image",
            serde_json::json!({ "sheet": { "type": "string" }, "cell": { "type": "string", "description": "top-left cell for the image, e.g. D2" }, "image": { "type": "object", "properties": { "mime_type": { "type": "string", "enum": ["image/png", "image/jpeg"] }, "data": { "type": "string", "description": "base64-encoded PNG/JPEG file bytes (no URL, path or data-URL prefix); decoded image at most 1 MiB, 10,000 pixels per side and 40 megapixels total" }, "alt_text": { "type": "string", "minLength": 1, "maxLength": 2048, "description": "required meaningful alternative text, at most 2,048 UTF-8 bytes" } }, "required": ["mime_type", "data", "alt_text"], "additionalProperties": false } }),
            &["sheet", "cell", "image"],
            "Excel: embed a bounded PNG/JPEG picture at a cell with required searchable alternative text; adding drawings to Strict workbooks is unsupported"
        ),
        op(
            "add_slide_from_layout",
            serde_json::json!({ "layout": { "type": "string", "description": format!("layout name; a new deck has {}", vak_ooxml::blank::DECK_LAYOUTS.iter().map(|(layout, placeholders)| format!("{layout} ({placeholders})")).collect::<Vec<_>>().join(", ")) }, "after": anchor("slide anchor to insert after, e.g. slide:256; omit to add at the end"), "placeholders": { "type": "object", "description": "placeholder to text or lines, e.g. {\"title\": \"Next steps\", \"body\": [\"First\", \"Second\"]}" }, "tables": { "type": "object", "description": "placeholder to native table rows, e.g. {\"body\": [[\"Region\", \"Sales\"], [\"North\", 120]]}", "additionalProperties": { "type": "array", "items": { "type": "array", "items": { "oneOf": [{"type":"string"},{"type":"number"},{"type":"boolean"}] } } } }, "charts": { "type": "object", "description": "placeholder to a native bar/line/pie chart with one series: 1–100 non-empty category labels (at most 512 UTF-8 bytes each), a non-empty title at most 512 UTF-8 bytes, same count of numeric values in [-1e12,1e12]; pie values cannot be negative", "additionalProperties": { "type": "object", "properties": { "title": { "type": "string" }, "chart_type": { "type": "string", "enum": ["bar", "line", "pie"] }, "categories": { "type": "array", "items": { "type": "string" } }, "values": { "type": "array", "items": { "type": "number" } } }, "required": ["title", "chart_type", "categories", "values"], "additionalProperties": false } }, "images": { "type": "object", "description": "placeholder to a PNG/JPEG image with descriptive alt_text", "additionalProperties": { "type": "object", "properties": { "mime_type": { "type": "string", "enum": ["image/png", "image/jpeg"] }, "data": { "type": "string", "description": "base64-encoded PNG/JPEG file bytes (no URL, path or data-URL prefix); decoded image at most 1 MiB, 10,000 pixels per side and 40 megapixels total" }, "alt_text": { "type": "string", "minLength": 1, "maxLength": 2048, "description": "required meaningful alternative text, at most 2,048 UTF-8 bytes" } }, "required": ["mime_type", "data", "alt_text"], "additionalProperties": false } }, "notes": { "type": "string", "description": "speaker notes for the slide" } }),
            &["layout"],
            "PowerPoint: add a slide from one of the deck's layouts; adding slides to Strict decks is unsupported. Use only placeholders offered by that exact layout. Fill each placeholder with at most one of text, table, chart or image; do not reuse one placeholder across maps. Tables allow at most 100 rows by 32 columns, charts 1–100 categories, and chart category/value arrays must have equal lengths."
        ),
        op(
            "set_placeholder_text",
            serde_json::json!({ "anchor": anchor("placeholder or shape anchor, e.g. slide:256/placeholder:title"), "text": lines }),
            &["anchor", "text"],
            "PowerPoint: replace a source placeholder/shape's text; arrays make one paragraph per item. Use anchors from doc_read, not guessed slide numbers."
        ),
        op(
            "set_notes",
            serde_json::json!({ "anchor": anchor("slide anchor"), "text": text }),
            &["anchor", "text"],
            "PowerPoint: set a slide's speaker notes; Strict decks are unsupported"
        ),
        op(
            "delete_slide",
            serde_json::json!({ "anchor": anchor("slide anchor") }),
            &["anchor"],
            "PowerPoint: delete a slide"
        ),
        op(
            "move_slide",
            serde_json::json!({ "anchor": anchor("slide anchor"), "after": anchor("slide anchor to move after; omit to move to the start") }),
            &["anchor"],
            "PowerPoint: move a slide"
        ),
        op(
            "set_title",
            serde_json::json!({ "title": text }),
            &["title"],
            "Any: set the document title property"
        ),
        op(
            "add_page_break",
            serde_json::json!({ "after": anchor("page anchor to add after, e.g. page:3; omit to add at the end") }),
            &[],
            "PDF: start a new page for the content that follows"
        ),
        op(
            "add_comment",
            serde_json::json!({ "anchor": anchor("line anchor, e.g. page:3/line:12"), "text": text }),
            &["anchor", "text"],
            "PDF: a sticky-note comment beside a line"
        ),
        op(
            "highlight",
            serde_json::json!({ "anchor": anchor("line anchor, e.g. page:3/line:12"), "note": { "type": "string", "description": "an optional note on the highlight" } }),
            &["anchor"],
            "PDF: highlight a line"
        ),
        op(
            "fill_field",
            serde_json::json!({ "field": { "type": "string", "description": "the field's name, as the read lists it" }, "value": { "type": "string", "description": "text, a choice, or on/off for a check box" } }),
            &["field", "value"],
            "PDF: fill an existing text, choice, checkbox or radio form field by its exact source-read name. Follow its offered values; signatures and XFA are unsupported."
        ),
        op(
            "rotate_page",
            serde_json::json!({ "anchor": anchor("page anchor, e.g. page:3"), "degrees": { "type": "integer", "description": "a multiple of 90, clockwise" } }),
            &["anchor", "degrees"],
            "PDF: turn a page"
        ),
        op(
            "delete_page",
            serde_json::json!({ "anchor": anchor("page anchor, e.g. page:3") }),
            &["anchor"],
            "PDF: remove a page"
        ),
        op(
            "move_page",
            serde_json::json!({ "anchor": anchor("page anchor, e.g. page:3"), "after": anchor("page anchor to move after; omit to move to the front") }),
            &["anchor"],
            "PDF: move a page"
        ),
    ])
}

struct Job {
    origin: OfficeOrigin,
    destination: PathBuf,
    draft: PathBuf,
    ops: Vec<vak_ooxml::edit::OfficeOp>,
    context: vak_ooxml::edit::EditContext,
    target: vak_ooxml::Format,
}

/// Applies `ops` to where `origin` starts: a file, refused unless its
/// `base_digest` names its bytes, or the built-in blank for `target`. The
/// one path every Office edit and creation takes, from the `office_apply`
/// tool and from `vak office apply`. Returns the starting bytes and what
/// the engine wrote; nothing is written here.
pub(crate) fn apply_checked(
    origin: &OfficeOrigin,
    ops: &[vak_ooxml::edit::OfficeOp],
    context: &vak_ooxml::edit::EditContext,
    target: vak_ooxml::Format,
) -> Result<(Vec<u8>, vak_ooxml::edit::Applied), String> {
    let limits = vak_ooxml::Limits::default();
    let (source, base_digest) = match origin {
        OfficeOrigin::File { path, base_digest } => (path, base_digest),
        OfficeOrigin::Blank => {
            let bytes = vak_ooxml::blank::blank(target)
                .map_err(|error| format!("nothing was written: {error}"))?;
            vak_ooxml::edit::check_renumbering(ops, context)
                .map_err(|error| format!("nothing was written: {error}"))?;
            let planned = vak_ooxml::edit::plan_chart_locations(ops)
                .map_err(|error| format!("nothing was written: {}", error.message))?;
            let applied = vak_ooxml::edit::apply(&bytes, &planned, context, limits, Some(target))
                .map_err(|error| format!("nothing was written: {error}"))?;
            return Ok((bytes, applied));
        }
    };
    let bytes = read_bounded(source, &limits)?;
    let digest = sha256_hex(&bytes);
    let base_digest = base_digest
        .trim()
        .trim_end_matches('…')
        .to_ascii_lowercase();
    if base_digest.len() < 16 || !digest.starts_with(&base_digest) {
        return Err(format!(
            "base_digest {base_digest:?} does not match the source (sha256 {}…); the file changed since it was read or the digest was mistyped. Pass the sha256 exactly as the read shows it, `{}…`, without completing it; if the file changed, read it again (doc_read, or `vak office read`) and use the anchors and sha256 from that read",
            &digest[..16],
            &digest[..16]
        ));
    }
    vak_ooxml::edit::check_renumbering(ops, context)
        .map_err(|error| format!("nothing was written: {error}"))?;
    let planned = vak_ooxml::edit::plan_chart_locations(ops)
        .map_err(|error| format!("nothing was written: {}", error.message))?;
    let applied = vak_ooxml::edit::apply(&bytes, &planned, context, limits, Some(target))
        .map_err(|error| format!("nothing was written: {error}"))?;
    Ok((bytes, applied))
}

impl Job {
    fn run(self) -> Result<String, String> {
        let limits = vak_ooxml::Limits::default();
        let (_, applied) = apply_checked(&self.origin, &self.ops, &self.context, self.target)?;
        let current = if self.destination.is_file() {
            let bytes = read_bounded(&self.destination, &limits)?;
            Some(
                vak_ooxml::read::read(std::io::Cursor::new(bytes), limits).map_err(|error| {
                    format!("the current workspace file cannot be read for comparison: {error}")
                })?,
            )
        } else {
            None
        };
        let changes = vak_ooxml::diff::diff(current.as_ref(), &applied.document);
        if let Some(parent) = self.draft.parent() {
            std::fs::create_dir_all(parent)
                .map_err(|error| format!("cannot create the draft directory: {error}"))?;
        }
        write_atomically(&self.draft, &applied.bytes)?;
        let mut report = format!("Draft sha256 {}….\n", &sha256_hex(&applied.bytes)[..16]);
        for result in &applied.results {
            report.push_str(&format!(
                "- {}: {} ({})\n",
                result.op, result.summary, result.check
            ));
        }
        for notice in &applied.notices {
            report.push_str(&format!("Note: {notice}.\n"));
        }
        match (&current, changes.changes.first()) {
            (None, Some(change)) => {
                let facts = change.after.as_deref().unwrap_or("new file");
                let facts = facts.strip_prefix("new file: ").unwrap_or(facts);
                report.push_str(&format!("It is a new file: {facts}.\n"));
            }
            _ if changes.summary.is_empty() => {
                report.push_str("Compared with the workspace file: no visible change.\n")
            }
            _ => {
                report.push_str("Compared with the workspace file: ");
                report.push_str(&changes.summary.join("; "));
                report.push('\n');
            }
        }
        match self.target.vocabulary {
            vak_ooxml::Vocabulary::Word if self.context.tracked => report.push_str(&format!(
                "Word edits are tracked changes by {}; they can also be accepted or rejected in Word.\n",
                self.context.author
            )),
            vak_ooxml::Vocabulary::Excel => report.push_str(
                "Vakyartha does not calculate formulas: Excel calculates them when the file is opened, and until then a formula shows a stale value or none.\n",
            ),
            _ => {}
        }
        Ok(report)
    }
}

pub(crate) fn read_bounded(path: &Path, limits: &vak_ooxml::Limits) -> Result<Vec<u8>, String> {
    let size = std::fs::metadata(path)
        .map_err(|error| format!("cannot read {}: {error}", path.display()))?
        .len();
    if size > limits.max_total_bytes {
        return Err(format!(
            "{} is over the size limit for an Office package",
            path.display()
        ));
    }
    std::fs::read(path).map_err(|error| format!("cannot read {}: {error}", path.display()))
}

pub(crate) fn sha256_hex(bytes: &[u8]) -> String {
    use sha2::Digest as _;
    sha2::Sha256::digest(bytes)
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

/// Writes through a sibling temporary file and a rename, so a reader never
/// sees half a package and a failure leaves no partial file.
pub(crate) fn write_atomically(destination: &Path, bytes: &[u8]) -> Result<(), String> {
    use std::io::Write as _;
    let Some(parent) = destination.parent() else {
        return Err("the destination has no parent directory".into());
    };
    let name = destination
        .file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_default();
    let temporary = parent.join(format!(".{name}.vak-{}.tmp", std::process::id()));
    let result = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&temporary)
        .and_then(|mut file| {
            file.write_all(bytes)?;
            file.sync_all()
        })
        .and_then(|()| std::fs::rename(&temporary, destination));
    if let Err(error) = result {
        let _ = std::fs::remove_file(&temporary);
        return Err(format!(
            "could not write {}: {error}",
            destination.display()
        ));
    }
    Ok(())
}

fn canonical_root(cwd: &Path) -> Result<PathBuf, String> {
    cwd.canonicalize()
        .map_err(|error| format!("cannot resolve workspace root: {error}"))
}

/// The workspace file a draft is for: its directory must exist inside the
/// workspace, and it must not be a symlink.
fn confined_destination(cwd: &Path, path: &str) -> Result<PathBuf, String> {
    let root = canonical_root(cwd)?;
    let candidate = if Path::new(path).is_absolute() {
        PathBuf::from(path)
    } else {
        cwd.join(path)
    };
    let Some(name) = candidate.file_name() else {
        return Err(format!("{path} names no file"));
    };
    let parent = candidate
        .parent()
        .ok_or_else(|| format!("{path} has no directory"))?
        .canonicalize()
        .map_err(|error| format!("the directory for {path} does not exist: {error}"))?;
    if !parent.starts_with(&root) {
        return Err(format!("access denied: {path} is outside the workspace"));
    }
    let destination = parent.join(name);
    if std::fs::symlink_metadata(&destination)
        .is_ok_and(|metadata| metadata.file_type().is_symlink())
    {
        return Err(format!(
            "{path} is a symlink; write to the file it points at instead"
        ));
    }
    Ok(destination)
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]
    use super::*;

    /// A text tool's refusal of a document says how to make a new one as
    /// well as how to change one, and a read is pointed at doc_read.
    #[test]
    fn a_text_tools_refusal_says_how_to_create_a_document() {
        let write = text_tool_refusal(std::path::Path::new("Launch brief.docx"), "write").unwrap();
        assert!(write.contains("to create Launch brief.docx, give its path and the ops"));
        assert!(write.contains("to change one that exists, read it with doc_read first"));
        let pdf = text_tool_refusal(std::path::Path::new("report.pdf"), "edit").unwrap();
        assert!(pdf.contains("to create report.pdf"));
        let read = text_tool_refusal(std::path::Path::new("report.pdf"), "read").unwrap();
        assert!(read.contains("doc_read") && !read.contains("to create"));
        assert!(text_tool_refusal(std::path::Path::new("notes.md"), "write").is_none());
    }
    use crate::sandbox_events::{SandboxEvent, SandboxEventSink};

    async fn run(dir: &Path, args: Value) -> ToolOutput {
        OfficeApplyTool
            .execute(
                &args,
                &ToolContext::new(dir.to_path_buf()).with_agent_id("mira"),
            )
            .await
    }

    fn digest_of(path: &Path) -> String {
        sha256_hex(&std::fs::read(path).unwrap())[..16].to_string()
    }

    /// Where the draft named `name` is on disk.
    fn on_disk(workspace: &Path, name: &str) -> PathBuf {
        vak_config::scope::draft_location(
            &vak_config::scope::executions_root(workspace),
            Path::new(name),
        )
        .unwrap_or_else(|| workspace.join(name))
    }

    fn draft_path(output: &ToolOutput) -> String {
        output
            .content
            .split("written to ")
            .nth(1)
            .and_then(|rest| rest.split(". ").next())
            .unwrap()
            .to_string()
    }

    #[test]
    fn the_op_schema_accepts_exactly_what_the_engine_reads_and_names_the_fault() {
        vak_config::paths::isolate_home_for_tests();
        let schema = OfficeApplyTool.schema();
        let call = |ops: Value| serde_json::json!({"path": "a.docx", "base_digest": "0123456789abcdef", "ops": ops});
        let valid = serde_json::json!([
            {"op": "replace_paragraph_text", "anchor": "p@1", "text": "x"},
            {"op": "add_paragraph", "text": "x", "style": "Heading 2", "after": "p@1"},
            {"op": "add_paragraph", "text": "at the end"},
            {"op": "add_table", "rows": [["Region", "Sales"], ["North", 120]], "header": true},
            {"op": "delete_paragraph", "anchor": "p@1"},
            {"op": "set_cells", "sheet": "Budget", "cells": {"B4": 1, "C4": "=A1", "D4": true}},
            {"op": "append_rows", "sheet": "Budget", "rows": [["a", 1]]},
            {"op": "add_sheet", "name": "Q4"},
            {"op": "rename_sheet", "sheet": "Q4", "name": "Q4 plan"},
            {"op": "format_cells", "sheet": "Budget", "range": "A1:D1", "bold": true, "number_format": "#,##0.00", "fill": "D9E2F3", "wrap": false, "italic": false},
            {"op": "set_column_widths", "sheet": "Budget", "widths": {"A": 30, "B": 12.5}},
            {"op": "add_slide_from_layout", "layout": "Title and Content", "after": "slide:256", "placeholders": {"title": "T", "body": ["a", "b"]}, "notes": "n"},
            {"op": "set_placeholder_text", "anchor": "slide:256/placeholder:title", "text": ["a", "b"]},
            {"op": "set_notes", "anchor": "slide:256", "text": "n"},
            {"op": "delete_slide", "anchor": "slide:256"},
            {"op": "move_slide", "anchor": "slide:256"},
            {"op": "set_title", "title": "T"}
        ]);
        crate::validate_input(&schema, &call(valid.clone())).unwrap();
        let ops: Vec<vak_ooxml::edit::OfficeOp> = serde_json::from_value(valid).unwrap();
        assert_eq!(ops.len(), 17, "every op the engine has, in the schema");

        for chart in [
            serde_json::json!({"op":"add_chart","sheet":"Daily","range":"A1:B4","chart_type":"bar","title":"Daily average"}),
            serde_json::json!({"op":"add_chart","categories":["Mon","Tue"],"values":[4.0,5.0],"title":"Daily average"}),
        ] {
            crate::validate_input(&schema, &call(serde_json::json!([chart])))
                .expect("Excel and PDF chart variants share a valid public schema");
        }
        assert!(validate_chart_shapes(
            &serde_json::json!([{"op":"add_chart","sheet":"Daily","range":"A1:B4","chart_type":"bar","title":"Daily average"}]),
            false
        )
        .is_none());
        assert!(validate_chart_shapes(
            &serde_json::json!([{"op":"add_chart","categories":["Mon"],"values":[4.0],"title":"Daily average"}]),
            true
        )
        .is_none());

        // The call a model made live: no `op`, and `after` as a boolean.
        let error = crate::validate_input(
            &schema,
            &call(serde_json::json!([{"after": true, "layout": "Title and Content", "placeholders": {"title": "Next steps"}}])),
        )
        .unwrap_err();
        assert!(
            error.contains("needs `op`, one of: replace_paragraph_text, add_paragraph, add_table"),
            "{error}"
        );
        let error = crate::validate_input(
            &schema,
            &call(serde_json::json!([{"op": "add_slide_from_layout", "layout": "Title and Content", "after": true}])),
        )
        .unwrap_err();
        assert!(error.contains("arguments.ops[0].after must be"), "{error}");
        let error = crate::validate_input(
            &schema,
            &call(serde_json::json!([{"op": "add_slide", "layout": "x"}])),
        )
        .unwrap_err();
        assert!(error.contains("\"add_slide\" is not one of"), "{error}");
    }

    #[test]
    fn model_guidance_includes_searchable_pdf_image_authoring() {
        vak_config::paths::isolate_home_for_tests();
        // What the model reads: the description and the op schemas together.
        let surface = format!(
            "{} {}",
            OfficeApplyTool.description(),
            OfficeApplyTool.schema()
        );
        assert!(surface.contains("or .pdf name"));
        assert!(surface.contains("descriptive alt_text"));
        assert!(surface.contains("searchable alternative-text caption"));
        assert!(surface.contains("Image pixels are not OCR-read"));
        assert!(surface.contains("page:3/line:12"));
        assert!(surface.contains(vak_ooxml::blank::WORKBOOK_SHEET));
        assert!(
            OfficeApplyTool.description().len() < 2_000,
            "the shared contract stays short; per-op detail lives in the schema"
        );
    }

    #[tokio::test]
    async fn every_advertised_creation_example_produces_a_readable_draft() {
        vak_config::paths::isolate_home_for_tests();
        let schema = OfficeApplyTool.schema();
        for example in schema["examples"].as_array().unwrap() {
            crate::validate_input(&schema, example).unwrap();
            let dir = tempfile::tempdir().unwrap();
            let result = run(dir.path(), example.clone()).await;
            assert!(!result.is_error, "{}: {}", example["path"], result.content);
            let read = crate::doc_read::DocReadTool
                .execute(
                    &serde_json::json!({"path":draft_path(&result)}),
                    &ToolContext::new(dir.path().to_path_buf()).with_agent_id("mira"),
                )
                .await;
            assert!(!read.is_error, "{}", read.content);
            assert!(read.content.contains("sha256"));
            assert!(!dir.path().join(example["path"].as_str().unwrap()).exists());
        }
    }

    #[test]
    fn chart_schema_requires_one_complete_format_specific_variant() {
        vak_config::paths::isolate_home_for_tests();
        let schema = OfficeApplyTool.schema();
        for chart in [
            serde_json::json!({"op":"add_chart","title":"Missing fields"}),
            serde_json::json!({"op":"add_chart","title":"Incomplete","sheet":"Sheet1","range":"A1:B3"}),
            serde_json::json!({"op":"add_chart","title":"Mixed","sheet":"Sheet1","range":"A1:B3","chart_type":"bar","categories":["A"],"values":[1]}),
        ] {
            assert!(
                crate::validate_input(&schema, &serde_json::json!({"path":"a.xlsx","ops":[chart]}))
                    .is_err()
            );
        }
    }

    #[tokio::test]
    async fn text_tools_refuse_an_office_file_and_name_the_office_tools() {
        vak_config::paths::isolate_home_for_tests();
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("q3.docx");
        let original = vak_ooxml::fixtures::docx();
        std::fs::write(&file, &original).unwrap();
        let ctx = ToolContext::new(dir.path().to_path_buf());
        let read = crate::read::ReadTool
            .execute(&serde_json::json!({"path": "q3.docx"}), &ctx)
            .await;
        assert!(read.is_error);
        assert!(
            read.content.contains("a Word document") && read.content.contains("doc_read"),
            "{}",
            read.content
        );
        let edit = crate::edit::EditTool
            .execute(
                &serde_json::json!({"path": "q3.docx", "edits": [{"old_text": "Steady.", "new_text": "Growing."}]}),
                &ctx,
            )
            .await;
        assert!(edit.is_error);
        assert!(
            edit.content.contains("office_apply") && edit.content.contains("nothing was changed"),
            "{}",
            edit.content
        );
        let write = crate::write::WriteTool
            .execute(
                &serde_json::json!({"path": "q3.docx", "content": "Growing."}),
                &ctx,
            )
            .await;
        assert!(write.is_error);
        assert!(write.content.contains("office_apply"), "{}", write.content);
        assert_eq!(
            std::fs::read(&file).unwrap(),
            original,
            "the package is untouched"
        );

        std::fs::write(dir.path().join("notes.txt"), "plain").unwrap();
        let plain = crate::read::ReadTool
            .execute(&serde_json::json!({"path": "notes.txt"}), &ctx)
            .await;
        assert!(!plain.is_error, "{}", plain.content);

        let pdf = dir.path().join("scan.pdf");
        let original = vak_pdf::fixtures::report();
        std::fs::write(&pdf, &original).unwrap();
        let read = crate::read::ReadTool
            .execute(&serde_json::json!({"path": "scan.pdf"}), &ctx)
            .await;
        assert!(
            read.is_error && read.content.contains("is a PDF") && read.content.contains("doc_read"),
            "{}",
            read.content
        );
        let write = crate::write::WriteTool
            .execute(
                &serde_json::json!({"path": "scan.pdf", "content": "x"}),
                &ctx,
            )
            .await;
        assert!(
            write.is_error && write.content.contains("nothing was changed"),
            "{}",
            write.content
        );
        assert_eq!(
            std::fs::read(&pdf).unwrap(),
            original,
            "the PDF is untouched"
        );
    }

    #[tokio::test]
    async fn editing_a_signed_file_records_that_its_signature_was_removed() {
        vak_config::paths::isolate_home_for_tests();
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("memo.docx");
        std::fs::write(&file, vak_ooxml::fixtures::signed_labelled_docx()).unwrap();
        let output = run(
            dir.path(),
            serde_json::json!({
                "path": "memo.docx",
                "base_digest": digest_of(&file),
                "ops": [{"op": "replace_paragraph_text", "anchor": "p@1", "text": "Hello again"}]
            }),
        )
        .await;
        assert!(!output.is_error, "{}", output.content);
        assert!(
            output
                .content
                .contains("Note: the source was digitally signed; its 1 signature(s) were removed"),
            "{}",
            output.content
        );
    }

    #[tokio::test]
    async fn an_edit_is_a_draft_and_the_workspace_file_is_untouched() {
        vak_config::paths::isolate_home_for_tests();
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("budget.xlsx");
        std::fs::write(&file, vak_ooxml::fixtures::xlsx()).unwrap();
        let (sink, mut events) = SandboxEventSink::new_with_id("exec-7".into());
        let output = OfficeApplyTool
            .execute(
                &serde_json::json!({
                    "path": "budget.xlsx",
                    "base_digest": digest_of(&file),
                    "ops": [{"op": "set_cells", "sheet": "Budget", "cells": {"B2": 150}}]
                }),
                &ToolContext::new(dir.path().to_path_buf())
                    .with_agent_id("mira")
                    .with_sandbox_sink(sink),
            )
            .await;
        assert!(!output.is_error, "{}", output.content);
        assert_eq!(
            std::fs::read(&file).unwrap(),
            vak_ooxml::fixtures::xlsx(),
            "the workspace file is unchanged"
        );
        assert_eq!(draft_path(&output), ".vak/scratch/mira/exec-7/budget.xlsx");
        assert!(
            output
                .content
                .contains("Compared with the workspace file: Budget: 1 changed"),
            "{}",
            output.content
        );
        assert!(
            output
                .content
                .contains("Excel calculates them when the file is opened")
        );
        let draft = on_disk(dir.path(), ".vak/scratch/mira/exec-7/budget.xlsx");
        let document = vak_ooxml::read::read(
            std::io::Cursor::new(std::fs::read(&draft).unwrap()),
            vak_ooxml::Limits::default(),
        )
        .unwrap();
        assert!(document.lines().join("\n").contains("B2: 150"));

        let mut seen = Vec::new();
        while let Ok(event) = events.try_recv() {
            seen.push(event);
        }
        assert!(seen.iter().any(|event| matches!(event,
            SandboxEvent::ExecutionStarted { tool, scratch_dir, .. }
                if tool == "office_apply" && scratch_dir.ends_with("mira/exec-7"))));
        assert!(seen.iter().any(|event| matches!(event,
            SandboxEvent::ExecutionFinished { exit_code: 0, artifacts, .. }
                if artifacts == &vec![".vak/scratch/mira/exec-7/budget.xlsx".to_string()])));

        let stale = run(
            dir.path(),
            serde_json::json!({
                "path": "budget.xlsx",
                "base_digest": "0000000000000000",
                "ops": [{"op": "set_cells", "sheet": "Budget", "cells": {"B2": 1}}]
            }),
        )
        .await;
        assert!(stale.is_error);
        assert!(
            stale.content.contains("does not match the source"),
            "{}",
            stale.content
        );
    }

    #[tokio::test]
    async fn drafts_chain_and_a_template_creates_a_new_file() {
        vak_config::paths::isolate_home_for_tests();
        let dir = tempfile::tempdir().unwrap();
        let template = dir.path().join("brand.pptx");
        std::fs::write(&template, vak_ooxml::fixtures::pptx_template()).unwrap();
        let first = run(
            dir.path(),
            serde_json::json!({
                "path": "q3.pptx",
                "source": "brand.pptx",
                "base_digest": digest_of(&template),
                "ops": [{"op": "add_slide_from_layout", "layout": "Title and Content",
                         "placeholders": {"title": "Q3", "body": ["Revenue", "Hiring"]}}]
            }),
        )
        .await;
        assert!(!first.is_error, "{}", first.content);
        assert!(
            first.content.contains("It is a new file: 2 slides."),
            "{}",
            first.content
        );
        assert!(
            !dir.path().join("q3.pptx").exists(),
            "nothing lands in the workspace before review"
        );
        let draft = draft_path(&first);
        let second = run(
            dir.path(),
            serde_json::json!({
                "path": "q3.pptx",
                "source": draft,
                "base_digest": digest_of(&on_disk(dir.path(), &draft)),
                "ops": [{"op": "set_notes", "anchor": "slide:256", "text": "x"}]
            }),
        )
        .await;
        assert!(
            second.is_error,
            "the template has no notes master to make a notes page from"
        );
        assert!(
            second.content.contains("no notes master"),
            "{}",
            second.content
        );
        let third = run(
            dir.path(),
            serde_json::json!({
                "path": "q3.pptx",
                "source": draft,
                "base_digest": digest_of(&on_disk(dir.path(), &draft)),
                "ops": [{"op": "set_title", "title": "Q3 review"}]
            }),
        )
        .await;
        assert!(!third.is_error, "{}", third.content);
        let latest = on_disk(dir.path(), &draft_path(&third));
        let document = vak_ooxml::read::read(
            std::io::Cursor::new(std::fs::read(latest).unwrap()),
            vak_ooxml::Limits::default(),
        )
        .unwrap();
        assert_eq!(document.title.as_deref(), Some("Q3 review"));
        assert!(
            document.lines().join("\n").contains("Slide 2: Q3"),
            "the chained draft keeps the first edit"
        );
    }

    #[tokio::test]
    async fn a_new_document_is_written_clean_and_an_existing_one_tracked() {
        vak_config::paths::isolate_home_for_tests();
        let dir = tempfile::tempdir().unwrap();
        let brand = dir.path().join("brand.docx");
        std::fs::write(&brand, vak_ooxml::fixtures::docx()).unwrap();
        let body = |output: &ToolOutput| {
            let bytes = std::fs::read(on_disk(dir.path(), &draft_path(output))).unwrap();
            let mut package =
                vak_ooxml::Package::open(std::io::Cursor::new(bytes), vak_ooxml::Limits::default())
                    .unwrap();
            String::from_utf8(package.read_part("word/document.xml").unwrap()).unwrap()
        };
        let ops = serde_json::json!([
            {"op": "replace_paragraph_text", "anchor": "p@11", "text": "Growing."},
            {"op": "add_paragraph", "text": "Details", "after": "p@1"}
        ]);
        let created = run(
            dir.path(),
            serde_json::json!({"path": "memo.docx", "source": "brand.docx", "base_digest": digest_of(&brand), "ops": ops}),
        )
        .await;
        assert!(!created.is_error, "{}", created.content);
        assert!(
            !body(&created).contains(r#"w:author="mira""#),
            "a file not yet in the workspace is a new document, written clean"
        );
        let edited = run(
            dir.path(),
            serde_json::json!({"path": "brand.docx", "base_digest": digest_of(&brand), "ops": ops}),
        )
        .await;
        assert!(!edited.is_error, "{}", edited.content);
        assert!(
            body(&edited).contains(r#"w:author="mira""#),
            "an existing document is edited with tracked changes"
        );
        let revised = OfficeApplyTool
            .execute(
                &serde_json::json!({"path": "brand.docx", "base_digest": digest_of(&brand), "ops": ops}),
                &ToolContext::new(dir.path().to_path_buf())
                    .with_agent_id("mira")
                    .with_new_documents(vec!["brand.docx".into()]),
            )
            .await;
        assert!(!revised.is_error, "{}", revised.content);
        assert!(
            !body(&revised).contains(r#"w:author="mira""#),
            "a revision's copy of a new document stays clean, though the copy holds the file"
        );
    }

    #[tokio::test]
    async fn refuses_escapes_bad_ops_and_failed_edits_without_writing() {
        vak_config::paths::isolate_home_for_tests();
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("memo.docx");
        std::fs::write(&file, vak_ooxml::fixtures::docx()).unwrap();
        let digest = digest_of(&file);
        for (args, expected) in [
            (
                serde_json::json!({"path": "../x.docx", "source": "memo.docx", "base_digest": digest, "ops": []}),
                "outside the workspace",
            ),
            (
                serde_json::json!({"path": "memo.docx", "base_digest": digest, "ops": [{"op": "replace_paragraph_text", "anchor": "p@11", "txt": "x"}]}),
                "ops are not valid",
            ),
            (
                serde_json::json!({"path": "memo.docx", "base_digest": digest, "ops": [{"op": "delete_paragraph", "anchor": "p@11"}]}),
                "nothing was written: op 1",
            ),
            (
                serde_json::json!({"path": "memo.docm", "source": "memo.docx", "base_digest": digest, "ops": []}),
                "never turns a file macro-enabled",
            ),
            (
                serde_json::json!({"path": "memo.txt", "source": "memo.docx", "base_digest": digest, "ops": []}),
                "with edit, or rewrite it whole with write",
            ),
        ] {
            let output = run(dir.path(), args).await;
            assert!(output.is_error, "{}", output.content);
            assert!(output.content.contains(expected), "{}", output.content);
        }
        assert_eq!(std::fs::read(&file).unwrap(), vak_ooxml::fixtures::docx());
        let leftovers: Vec<_> = std::fs::read_dir(dir.path()).unwrap().collect();
        assert_eq!(
            leftovers.len(),
            1,
            "no draft and no temporary file are left behind"
        );
    }

    #[tokio::test]
    async fn a_new_file_is_created_from_scratch_as_a_draft() {
        vak_config::paths::isolate_home_for_tests();
        let dir = tempfile::tempdir().unwrap();
        let created = run(
            dir.path(),
            serde_json::json!({
                "path": "memo.docx",
                "ops": [
                    {"op": "add_paragraph", "text": "Q3 review", "style": "Title"},
                    {"op": "add_paragraph", "text": "Revenue grew 12%."},
                    {"op": "add_table", "rows": [["Region", "Revenue"], ["North", 120]]}
                ]
            }),
        )
        .await;
        assert!(!created.is_error, "{}", created.content);
        assert!(
            created.content.contains("memo.docx is a new file, not in the workspace until a person reviews and accepts the draft"),
            "{}",
            created.content
        );
        assert!(
            created
                .content
                .contains("It is a new file: 2 paragraphs, 9 words, 1 heading, 1 table."),
            "{}",
            created.content
        );
        assert!(
            !created.content.contains("tracked changes by"),
            "{}",
            created.content
        );
        assert!(
            !dir.path().join("memo.docx").exists(),
            "only a draft is written"
        );
        let draft = on_disk(dir.path(), &draft_path(&created));
        assert!(draft.starts_with(vak_config::scope::executions_root(dir.path()).join("mira")));
        let document = vak_ooxml::read::read(
            std::io::Cursor::new(std::fs::read(&draft).unwrap()),
            vak_ooxml::Limits::default(),
        )
        .unwrap();
        let lines = document.lines().join("\n");
        assert!(lines.contains("# Q3 review"), "{lines}");
        assert!(
            lines.contains("[tbl@1/r2] [p:1A000005] North | [p:1A000006] 120"),
            "{lines}"
        );

        for extension in ["xlsx", "pptx", "dotx"] {
            let op = match extension {
                "xlsx" => {
                    serde_json::json!({"op": "set_cells", "sheet": "Sheet1", "cells": {"A1": "Item"}})
                }
                "pptx" => {
                    serde_json::json!({"op": "add_slide_from_layout", "layout": "Title Slide", "placeholders": {"title": "Launch"}})
                }
                _ => serde_json::json!({"op": "add_paragraph", "text": "Letterhead"}),
            };
            let output = run(
                dir.path(),
                serde_json::json!({"path": format!("new.{extension}"), "ops": [op]}),
            )
            .await;
            assert!(!output.is_error, "{extension}: {}", output.content);
        }
    }

    #[tokio::test]
    async fn creating_is_refused_where_it_would_hide_a_mistake() {
        vak_config::paths::isolate_home_for_tests();
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("brand.docx"), vak_ooxml::fixtures::docx()).unwrap();
        let add = serde_json::json!([{"op": "add_paragraph", "text": "x"}]);
        let refusal = |output: &ToolOutput, needle: &str| {
            assert!(output.is_error, "{}", output.content);
            assert!(output.content.contains(needle), "{}", output.content);
        };
        refusal(
            &run(
                dir.path(),
                serde_json::json!({"path": "brand.docx", "ops": add}),
            )
            .await,
            "brand.docx already exists",
        );
        refusal(
            &run(
                dir.path(),
                serde_json::json!({"path": "brnad.docx", "base_digest": "0123456789abcdef", "ops": add}),
            )
            .await,
            "brnad.docx does not exist, so there is no file for base_digest to name",
        );
        refusal(
            &run(
                dir.path(),
                serde_json::json!({"path": "memo.docx", "source": "brand.docx", "ops": add}),
            )
            .await,
            "missing base_digest for source brand.docx",
        );
        refusal(
            &run(
                dir.path(),
                serde_json::json!({"path": "memo.docm", "ops": add}),
            )
            .await,
            "never makes a macro-enabled file",
        );
        refusal(
            &run(
                dir.path(),
                serde_json::json!({"path": "flow.vsdx", "ops": add}),
            )
            .await,
            "Visio drawing cannot be created from scratch yet",
        );
        refusal(
            &run(
                dir.path(),
                serde_json::json!({"path": "memo.docx", "ops": [{"op": "set_cells", "sheet": "Sheet1", "cells": {"A1": 1}}]}),
            )
            .await,
            "this op edits an Excel workbook",
        );
        let scratch = dir.path().join(".vak");
        assert!(
            !scratch.exists()
                || std::fs::read_dir(scratch.join("scratch/mira"))
                    .map(|entries| entries.count())
                    .unwrap_or(0)
                    == 0,
            "a refused creation writes no draft"
        );
    }
}
