//! `office_apply`: propose edits to, or create, a Word, Excel or PowerPoint
//! file through the typed op set (docs/design/72-openxml-documents.md, P2).
//!
//! It runs in the broker worker (invariant 14). The source must be the
//! exact file the model read (`base_digest`, O4); every op is checked by a
//! re-read before anything is written (O5); and Word edits are tracked
//! changes under the runtime's Agent id, never a name the model chose.
//!
//! It never writes the workspace file. The result is a draft in this
//! execution's `.vak/scratch/<agent>/<execution>/` directory, announced to
//! the Workbench like any other execution, so the one Review path (freeze a
//! candidate, verify it in the worker, promote atomically with undo) is how
//! a change reaches the workspace (invariant 35; docs/design/72, "Owner
//! direction").

use std::path::{Path, PathBuf};

use async_trait::async_trait;
use serde_json::Value;

use crate::{ResourceClaims, Tool, ToolContext, ToolOutput};

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
        "Propose edits to, or create, a Word, Excel or PowerPoint file with typed ops. The result is a draft a person reviews and accepts; the workspace file does not change until then. Read the source with doc_read first and pass the sha256 value it printed as base_digest; ops name anchors from that read (p@12, p:1A2B3C4D, Budget!B4, slide:256/shape:3, slide:256/placeholder:title). Word edits become tracked changes. Excel formulas recalculate when the file is opened. New slides come from the deck's own layouts. To create a file from a template, set source to the template and path to the new file. To keep editing a draft, pass the draft as source. Macros are never added or run."
    }

    fn schema(&self) -> Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "path": {
                    "type": "string",
                    "description": "The workspace file the draft is for (relative to the workspace). May equal source."
                },
                "source": {
                    "type": "string",
                    "description": "File to start from: the file being edited, a template, or an earlier draft. Defaults to path."
                },
                "base_digest": {
                    "type": "string",
                    "description": "The sha256 value doc_read printed for the source file, proving the ops were written against its current content."
                },
                "ops": {
                    "type": "array",
                    "minItems": 1,
                    "description": "Ops applied in order. Each names its `op` and only that op's fields. Anchors come from doc_read of the source: paragraphs p:1A2B3C4D or p@12, cells Budget!B4, slides slide:256, shapes slide:256/shape:3, placeholders slide:256/placeholder:title.",
                    "items": { "oneOf": op_schemas() }
                }
            },
            "required": ["path", "base_digest", "ops"]
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
        let source = args
            .get("source")
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|source| !source.is_empty())
            .unwrap_or(path);
        let Some(base_digest) = args.get("base_digest").and_then(Value::as_str) else {
            return ToolOutput::error(
                "missing required parameter: base_digest (the sha256 value doc_read printed for the source)",
            );
        };
        let ops: Vec<vak_ooxml::edit::OfficeOp> = match args.get("ops") {
            Some(ops) => match serde_json::from_value(ops.clone()) {
                Ok(ops) => ops,
                Err(error) => {
                    return ToolOutput::error(format!(
                        "ops are not valid: {error}. Each op is an object with an \"op\" name and only that op's fields"
                    ));
                }
            },
            None => return ToolOutput::error("missing required parameter: ops"),
        };
        let (source_path, destination) = match (
            confined_existing(&ctx.cwd, source),
            confined_destination(&ctx.cwd, path),
        ) {
            (Ok(source), Ok(destination)) => (source, destination),
            (Err(error), _) | (_, Err(error)) => return ToolOutput::error(error),
        };
        let Some(target) = destination
            .extension()
            .and_then(|extension| extension.to_str())
            .and_then(vak_ooxml::Format::from_extension)
        else {
            return ToolOutput::error(format!(
                "{path} is not named as a Word, Excel or PowerPoint file (.docx, .xlsx, .pptx and their variants)"
            ));
        };
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
        let agent = ctx.agent_id.clone().unwrap_or_else(|| "vak".into());
        let author = tracked_change_author(&agent);
        let now = chrono::Utc::now();
        let date = now.format("%Y-%m-%dT%H:%M:%SZ").to_string();
        let execution = ctx
            .sandbox_sink
            .as_ref()
            .map(|sink| sink.execution_id().to_string())
            .unwrap_or_else(|| format!("office-{}", now.timestamp_nanos_opt().unwrap_or(0)));
        let draft_root = root
            .join(".vak")
            .join("scratch")
            .join(&agent)
            .join(&execution);
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
                "source": source,
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
        let base_digest = base_digest.to_string();
        let job = Job {
            source: source_path,
            destination,
            draft: draft.clone(),
            base_digest,
            ops,
            context: vak_ooxml::edit::EditContext { author, date },
            target,
        };
        let work = tokio::task::spawn_blocking(move || job.run()).await;
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
                ToolOutput::ok(format!(
                    "Draft for {path} written to {draft_relative}. {path} in the workspace is unchanged until a person reviews and accepts the draft: the draft is already shown to them with Review draft and its change list, so do not present it again as a card, HTML or a diff; answer with one sentence saying what you changed. To keep editing, call office_apply again with source \"{draft_relative}\" and the draft's sha256 as base_digest.\n{report}"
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

/// The refusal a text tool gives for an Office file: a package is a ZIP of
/// XML parts, so a text read shows nothing useful and a text edit or write
/// can only fail or destroy it. Names the tools that do handle it.
pub fn text_tool_refusal(path: &Path, tool: &str) -> Option<String> {
    let name = path.to_string_lossy();
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

/// The name Word shows on an Agent's tracked changes: the runtime's Agent
/// id, never a name the model chose.
pub fn tracked_change_author(agent_id: &str) -> String {
    if agent_id == "vak" {
        "Vak".to_string()
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
            serde_json::json!({ "anchor": anchor("paragraph anchor, e.g. p@12"), "text": text }),
            &["anchor", "text"],
            "Word: replace a paragraph's text (a tracked change)"
        ),
        op(
            "insert_paragraph_after",
            serde_json::json!({ "anchor": anchor("paragraph anchor"), "text": text, "style": { "type": "string", "description": "style id or name, e.g. Heading 2" } }),
            &["anchor", "text"],
            "Word: insert a new paragraph after one"
        ),
        op(
            "delete_paragraph",
            serde_json::json!({ "anchor": anchor("paragraph anchor") }),
            &["anchor"],
            "Word: delete a paragraph (a tracked change)"
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
            "add_slide_from_layout",
            serde_json::json!({ "layout": { "type": "string", "description": "layout name from doc_read, e.g. Title and Content" }, "after": anchor("slide anchor to insert after, e.g. slide:256; omit to add at the end"), "placeholders": { "type": "object", "description": "placeholder to text, e.g. {\"title\": \"Next steps\", \"body\": [\"First\", \"Second\"]}" } }),
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
    ])
}

struct Job {
    source: PathBuf,
    destination: PathBuf,
    draft: PathBuf,
    base_digest: String,
    ops: Vec<vak_ooxml::edit::OfficeOp>,
    context: vak_ooxml::edit::EditContext,
    target: vak_ooxml::Format,
}

/// Applies `ops` to the file at `source`, refusing unless `base_digest`
/// names its bytes: the one path every Office edit takes, from the
/// `office_apply` tool and from `vak office apply`. Returns the source's
/// bytes and what the engine wrote; nothing is written here.
pub(crate) fn apply_checked(
    source: &Path,
    base_digest: &str,
    ops: &[vak_ooxml::edit::OfficeOp],
    context: &vak_ooxml::edit::EditContext,
    target: vak_ooxml::Format,
) -> Result<(Vec<u8>, vak_ooxml::edit::Applied), String> {
    let limits = vak_ooxml::Limits::default();
    let bytes = read_bounded(source, &limits)?;
    let digest = sha256_hex(&bytes);
    let base_digest = base_digest
        .trim()
        .trim_end_matches('…')
        .to_ascii_lowercase();
    if base_digest.len() < 16 || !digest.starts_with(&base_digest) {
        return Err(format!(
            "base_digest {base_digest:?} does not match the source (sha256 {}…); the file changed since it was read or the digest was mistyped. Read it again (doc_read, or `vak office read`) and use the anchors and sha256 from that read",
            &digest[..16]
        ));
    }
    let applied = vak_ooxml::edit::apply(&bytes, ops, context, limits, Some(target))
        .map_err(|error| format!("nothing was written: {error}"))?;
    Ok((bytes, applied))
}

impl Job {
    fn run(self) -> Result<String, String> {
        let limits = vak_ooxml::Limits::default();
        let (_, applied) = apply_checked(
            &self.source,
            &self.base_digest,
            &self.ops,
            &self.context,
            self.target,
        )?;
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
        report.push_str("Compared with the workspace file: ");
        if changes.summary.is_empty() {
            report.push_str("no visible change.\n");
        } else {
            report.push_str(&changes.summary.join("; "));
            report.push('\n');
        }
        match self.target.vocabulary {
            vak_ooxml::Vocabulary::Word => report.push_str(&format!(
                "Word edits are tracked changes by {}; they can also be accepted or rejected in Word.\n",
                self.context.author
            )),
            vak_ooxml::Vocabulary::Excel => report.push_str(
                "Vak does not calculate formulas: Excel recalculates when the file is opened, and until then cached values are stale.\n",
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
            {"op": "insert_paragraph_after", "anchor": "p@1", "text": "x", "style": "Heading 2"},
            {"op": "delete_paragraph", "anchor": "p@1"},
            {"op": "set_cells", "sheet": "Budget", "cells": {"B4": 1, "C4": "=A1", "D4": true}},
            {"op": "append_rows", "sheet": "Budget", "rows": [["a", 1]]},
            {"op": "add_sheet", "name": "Q4"},
            {"op": "add_slide_from_layout", "layout": "Title and Content", "after": "slide:256", "placeholders": {"title": "T", "body": ["a", "b"]}},
            {"op": "set_placeholder_text", "anchor": "slide:256/placeholder:title", "text": ["a", "b"]},
            {"op": "set_notes", "anchor": "slide:256", "text": "n"},
            {"op": "delete_slide", "anchor": "slide:256"},
            {"op": "move_slide", "anchor": "slide:256"},
            {"op": "set_title", "title": "T"}
        ]);
        crate::validate_input(&schema, &call(valid.clone())).unwrap();
        let ops: Vec<vak_ooxml::edit::OfficeOp> = serde_json::from_value(valid).unwrap();
        assert_eq!(ops.len(), 12, "every op the engine has, in the schema");

        // The call a model made live: no `op`, and `after` as a boolean.
        let error = crate::validate_input(
            &schema,
            &call(serde_json::json!([{"after": true, "layout": "Title and Content", "placeholders": {"title": "Next steps"}}])),
        )
        .unwrap_err();
        assert!(
            error.contains("needs `op`, one of: replace_paragraph_text, insert_paragraph_after"),
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
                .contains("recalculates when the file is opened")
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
            first
                .content
                .contains("Compared with the workspace file: Deck: 1 added"),
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
        assert!(second.is_error, "the template's slide has no notes page");
        assert!(
            second.content.contains("no notes page"),
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
                serde_json::json!({"path": "memo.docx", "base_digest": digest, "ops": [{"op": "replace_paragraph_text", "anchor": "p@2", "text": "x"}]}),
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
}
