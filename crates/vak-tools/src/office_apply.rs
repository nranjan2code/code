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
static DESCRIPTION: LazyLock<String> = LazyLock::new(|| {
    format!(
        "Create, or propose edits to, a Word, Excel or PowerPoint file with typed ops. The result is a draft a person reviews and accepts; the workspace does not change until then. \
         To create a new file from scratch, give path (a new .docx, .xlsx or .pptx name) and ops, and leave out source and base_digest. A new Word document offers the styles {styles}; add content with add_paragraph and add_table. A new workbook has one empty sheet, {sheet}; use rename_sheet, set_cells, format_cells and set_column_widths. A new deck has the layouts {layouts}; add slides with add_slide_from_layout, with notes if wanted. \
         To create a file from a template in the workspace, set source to the template, base_digest to its sha256, and path to the new file. \
         To edit a file, read it with doc_read first and pass the sha256 it printed as base_digest; ops name anchors from that read (p@12, p:1A2B3C4D, a table cell's paragraph as its row shows it, Budget!B4, slide:256/shape:3, slide:256/placeholder:title). Word edits to an existing file become tracked changes; a new file is written clean. \
         Text is plain: Markdown is not interpreted, so headings and lists come from styles. Excel calculates formulas when the file is opened. To keep editing a draft, pass the draft as source with its sha256. Macros are never added or run. A Visio drawing cannot be created or edited: say so, and never build one with a command or script. \
         A PDF works the same way, created from scratch in the same styles or edited after a doc_read, whose anchors are page:3 and page:3/line:12: replace_paragraph_text and delete_paragraph act on one line (a replaced line is drawn in Helvetica), add_comment and highlight mark a line, fill_field sets a form field, rotate_page, delete_page and move_page rearrange pages, and add_paragraph, add_table and add_page_break set new content on new pages after a page (after: page:3) or at the end. Only Latin text can be written into a PDF.",
        styles = vak_ooxml::blank::DOCUMENT_STYLES.join(", "),
        sheet = vak_ooxml::blank::WORKBOOK_SHEET,
        layouts = vak_ooxml::blank::DECK_LAYOUTS
            .iter()
            .map(|(layout, placeholders)| format!("{layout} ({placeholders})"))
            .collect::<Vec<_>>()
            .join(", "),
    )
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

    fn description(&self) -> &str {
        DESCRIPTION.as_str()
    }

    fn schema(&self) -> Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "path": {
                    "type": "string",
                    "description": "The workspace file the draft is for (relative to the workspace): an existing file to edit, or a new name to create. May equal source."
                },
                "source": {
                    "type": "string",
                    "description": "File to start from: the file being edited, a template, or an earlier draft. Defaults to path. Leave out, with base_digest, to create path from scratch."
                },
                "base_digest": {
                    "type": "string",
                    "description": "The sha256 exactly as doc_read printed it for the source file (its first 16 characters, as shown, are enough; never fill in the rest), proving the ops were written against its current content. Leave out only when creating a new file from scratch."
                },
                "ops": {
                    "type": "array",
                    "minItems": 1,
                    "description": "Ops applied in order. Each names its `op` and only that op's fields. Anchors come from doc_read of the source: paragraphs p:1A2B3C4D or p@12, cells Budget!B4, slides slide:256, shapes slide:256/shape:3, placeholders slide:256/placeholder:title.",
                    "items": { "oneOf": op_schemas() }
                }
            },
            "required": ["path", "ops"]
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
                match confined_existing(&ctx.cwd, source) {
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
        let target = destination
            .extension()
            .and_then(|extension| extension.to_str())
            .and_then(vak_ooxml::Format::from_extension);
        if target.is_none() && !pdf {
            return ToolOutput::error(format!(
                "{path} is not named as a Word, Excel, PowerPoint or PDF file (.docx, .xlsx, .pptx and their variants, or .pdf)"
            ));
        }
        let root = match canonical_root(&ctx.cwd) {
            Ok(root) => root,
            Err(error) => return ToolOutput::error(error),
        };
        let Ok(relative) = destination.strip_prefix(&root).map(Path::to_path_buf) else {
            return ToolOutput::error(format!("access denied: {path} is outside the workspace"));
        };
        if relative.starts_with(".vak") {
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
        let draft_root = root.join(draft_dir(agent, &execution));
        let draft = draft_root.join(&relative);
        let draft_relative = draft
            .strip_prefix(&root)
            .unwrap_or(&draft)
            .display()
            .to_string();
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

/// The refusal a text tool gives for an Office file or a PDF: a package is
/// a ZIP of XML parts and a PDF a binary object graph, so a text read shows
/// nothing useful and a text edit or write can only fail or destroy one.
/// Names the tools that do handle it.
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
                "{display} is a PDF, which {tool} would corrupt; nothing was changed. Read it with doc_read, then change it with office_apply, which writes a draft for review."
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
            "{display} is {what}, a ZIP package that {tool} would corrupt; nothing was changed. Read it with doc_read, then change it with office_apply, which writes a draft for review."
        ),
    })
}

/// The Agent a draft is filed under when the session names none.
const DEFAULT_AGENT: &str = "vak";

/// Where one execution's drafts live, relative to the workspace root:
/// `.vak/scratch/<agent>/<execution>/`, the draft of a file keeping the
/// file's workspace path beneath it. The execution is the `office_apply`
/// call's id and the agent is the session's Agent id, so the ledger alone
/// locates every draft a session delivered.
pub fn draft_dir(agent_id: Option<&str>, execution: &str) -> PathBuf {
    Path::new(".vak")
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
        serde_json::json!({
            "type": "object",
            "description": description,
            "properties": properties,
            "required": all_required,
            "additionalProperties": false
        })
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
            "Word: give the paragraph's whole new text, as it should read. Only the words that differ become tracked changes; its formatting, links, footnote marks and fields stay as they are. PDF: the line's whole new text, drawn in Helvetica where the old text was"
        ),
        op(
            "add_paragraph",
            serde_json::json!({ "text": text, "style": { "type": "string", "description": "style id or name, e.g. Heading 1, List Bullet, List Number" }, "after": anchor("paragraph anchor to add after (in a PDF, a page, e.g. page:3); omit to add at the end of the document") }),
            &["text"],
            "Word: add a paragraph, at the end or after one. A List Number paragraph continues the list just above it, else starts at 1. PDF: set on new pages, after a page or at the end"
        ),
        op(
            "add_table",
            serde_json::json!({ "rows": { "type": "array", "items": { "type": "array" }, "description": "rows of cell text, the first row being the header, e.g. [[\"Region\", \"Sales\"], [\"North\", \"120\"]]" }, "after": anchor("paragraph anchor to add after; omit to add at the end"), "header": { "type": "boolean", "description": "false when the first row is not a header row; default true" } }),
            &["rows"],
            "Word: add a table spanning the page width"
        ),
        op(
            "delete_paragraph",
            serde_json::json!({ "anchor": anchor("paragraph anchor; in a PDF, a line, e.g. page:3/line:12") }),
            &["anchor"],
            "Word: delete a paragraph (a tracked change in an existing file; removed outright in a new one). PDF: remove a line from its page"
        ),
        op(
            "set_cells",
            serde_json::json!({ "sheet": { "type": "string" }, "cells": { "type": "object", "description": "address to value, e.g. {\"B4\": 120, \"C4\": \"=SUM(B1:B3)\"}" } }),
            &["sheet", "cells"],
            "Excel: set cell values or formulas"
        ),
        op(
            "append_rows",
            serde_json::json!({ "sheet": { "type": "string" }, "rows": { "type": "array", "items": { "type": "array" } } }),
            &["sheet", "rows"],
            "Excel: append rows after the last used row"
        ),
        op(
            "add_sheet",
            serde_json::json!({ "name": { "type": "string" } }),
            &["name"],
            "Excel: add an empty sheet"
        ),
        op(
            "rename_sheet",
            serde_json::json!({ "sheet": { "type": "string" }, "name": { "type": "string", "description": "the new name" } }),
            &["sheet", "name"],
            "Excel: rename a sheet, before anything refers to it by name"
        ),
        op(
            "format_cells",
            serde_json::json!({ "sheet": { "type": "string" }, "range": { "type": "string", "description": "cells, e.g. A1:D1 or B4" }, "bold": { "type": "boolean" }, "italic": { "type": "boolean" }, "number_format": { "type": "string", "description": "an Excel format code, e.g. #,##0.00 or 0% or yyyy-mm-dd" }, "fill": { "type": "string", "description": "background colour as six hex digits, e.g. D9E2F3" }, "wrap": { "type": "boolean", "description": "wrap long text" } }),
            &["sheet", "range"],
            "Excel: format cells; every other part of each cell's format is kept"
        ),
        op(
            "set_column_widths",
            serde_json::json!({ "sheet": { "type": "string" }, "widths": { "type": "object", "description": "column letter to width in characters, e.g. {\"A\": 30, \"B\": 12}" } }),
            &["sheet", "widths"],
            "Excel: set column widths"
        ),
        op(
            "add_slide_from_layout",
            serde_json::json!({ "layout": { "type": "string", "description": "layout name, e.g. Title and Content" }, "after": anchor("slide anchor to insert after, e.g. slide:256; omit to add at the end"), "placeholders": { "type": "object", "description": "placeholder to text, e.g. {\"title\": \"Next steps\", \"body\": [\"First\", \"Second\"]}; a Title Slide has title and subtitle, Two Content has idx:1 and idx:2" }, "notes": { "type": "string", "description": "speaker notes for the slide" } }),
            &["layout"],
            "PowerPoint: add a slide from one of the deck's layouts"
        ),
        op(
            "set_placeholder_text",
            serde_json::json!({ "anchor": anchor("placeholder or shape anchor, e.g. slide:256/placeholder:title"), "text": lines }),
            &["anchor", "text"],
            "PowerPoint: set a placeholder's or shape's text"
        ),
        op(
            "set_notes",
            serde_json::json!({ "anchor": anchor("slide anchor"), "text": text }),
            &["anchor", "text"],
            "PowerPoint: set a slide's speaker notes"
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
            "PDF: fill a form field"
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
            let applied = vak_ooxml::edit::apply(&bytes, ops, context, limits, Some(target))
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
    let applied = vak_ooxml::edit::apply(&bytes, ops, context, limits, Some(target))
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

/// An existing file inside the workspace (invariant 10).
fn confined_existing(cwd: &Path, path: &str) -> Result<PathBuf, String> {
    let root = canonical_root(cwd)?;
    let candidate = if Path::new(path).is_absolute() {
        PathBuf::from(path)
    } else {
        cwd.join(path)
    };
    let canonical = candidate
        .canonicalize()
        .map_err(|error| format!("cannot open {path}: {error}"))?;
    if !canonical.starts_with(&root) {
        return Err(format!("access denied: {path} is outside the workspace"));
    }
    Ok(canonical)
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

    #[tokio::test]
    async fn text_tools_refuse_an_office_file_and_name_the_office_tools() {
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
        let draft = dir.path().join(".vak/scratch/mira/exec-7/budget.xlsx");
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
                if tool == "office_apply" && scratch_dir.ends_with(".vak/scratch/mira/exec-7"))));
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
                "base_digest": digest_of(&dir.path().join(&draft)),
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
                "base_digest": digest_of(&dir.path().join(&draft)),
                "ops": [{"op": "set_title", "title": "Q3 review"}]
            }),
        )
        .await;
        assert!(!third.is_error, "{}", third.content);
        let latest = dir.path().join(draft_path(&third));
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
        let dir = tempfile::tempdir().unwrap();
        let brand = dir.path().join("brand.docx");
        std::fs::write(&brand, vak_ooxml::fixtures::docx()).unwrap();
        let body = |output: &ToolOutput| {
            let bytes = std::fs::read(dir.path().join(draft_path(output))).unwrap();
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
                "not named as a Word",
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
        let draft = dir.path().join(draft_path(&created));
        assert!(draft.starts_with(dir.path().join(".vak/scratch/mira")));
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
