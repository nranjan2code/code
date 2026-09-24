//! `office_apply`: edit or create a Word, Excel or PowerPoint file through
//! the typed op set (docs/design/72-openxml-documents.md, P2).
//!
//! It runs in the broker worker (invariant 14). The source must be the
//! exact file the model read (`base_digest`, O4); every op is checked by a
//! re-read before anything is written (O5); the write is atomic; and Word
//! edits are tracked changes under the runtime's Agent id, never a name the
//! model chose.

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

    fn description(&self) -> &str {
        "Edit or create a Word, Excel or PowerPoint file with typed ops. Read the source with doc_read first and pass the sha256 value it printed as base_digest; ops name anchors from that read (p@12, p:1A2B3C4D, Budget!B4, slide:256/shape:3, slide:256/placeholder:title). Word edits become tracked changes. Excel formulas recalculate when the file is opened. New slides come from the deck's own layouts. To create a file from a template, set source to the template and path to the new file. Macros are never added or run."
    }

    fn schema(&self) -> Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "path": {
                    "type": "string",
                    "description": "File to write (relative to the workspace). May equal source."
                },
                "source": {
                    "type": "string",
                    "description": "File to start from: the file being edited, or a template. Defaults to path."
                },
                "base_digest": {
                    "type": "string",
                    "description": "The sha256 value doc_read printed for the source file, proving the ops were written against its current content."
                },
                "ops": {
                    "type": "array",
                    "description": "Ops applied in order. Word: {op:'replace_paragraph_text', anchor, text} {op:'insert_paragraph_after', anchor, text, style?} {op:'delete_paragraph', anchor}. Excel: {op:'set_cells', sheet, cells:{'B4':120,'C4':'=SUM(B1:B3)','D4':'text'}} {op:'append_rows', sheet, rows:[[...]]} {op:'add_sheet', name}. PowerPoint: {op:'add_slide_from_layout', layout, after?, placeholders:{title:'...', body:['...']}} {op:'set_placeholder_text', anchor, text} {op:'set_notes', anchor, text} {op:'delete_slide', anchor} {op:'move_slide', anchor, after?}. Any: {op:'set_title', title}.",
                    "items": {
                        "type": "object",
                        "properties": { "op": { "type": "string" } },
                        "required": ["op"]
                    }
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
        let author = match ctx.agent_id.as_deref() {
            None | Some("vak") => "Vak".to_string(),
            Some(id) => id.to_string(),
        };
        let date = chrono::Utc::now().format("%Y-%m-%dT%H:%M:%SZ").to_string();
        let base_digest = base_digest
            .trim()
            .trim_end_matches('…')
            .to_ascii_lowercase();
        let display = path.to_string();
        let work = tokio::task::spawn_blocking(move || {
            apply(
                &source_path,
                &destination,
                &base_digest,
                &ops,
                author,
                date,
                target,
            )
        })
        .await;
        match work {
            Ok(Ok(report)) => ToolOutput::ok(format!("Wrote {display}. {report}")),
            Ok(Err(error)) => ToolOutput::error(error),
            Err(error) => ToolOutput::error(format!("office_apply failed: {error}")),
        }
    }
}

fn apply(
    source: &Path,
    destination: &Path,
    base_digest: &str,
    ops: &[vak_ooxml::edit::OfficeOp],
    author: String,
    date: String,
    target: vak_ooxml::Format,
) -> Result<String, String> {
    let limits = vak_ooxml::Limits::default();
    let size = std::fs::metadata(source)
        .map_err(|error| format!("cannot read the source: {error}"))?
        .len();
    if size > limits.max_total_bytes {
        return Err("the source is over the size limit for an Office package".into());
    }
    let bytes =
        std::fs::read(source).map_err(|error| format!("cannot read the source: {error}"))?;
    let digest = sha256_hex(&bytes);
    if base_digest.len() < 16 || !digest.starts_with(base_digest) {
        return Err(format!(
            "base_digest {base_digest:?} does not match the source (sha256 {}…); the file changed since it was read or the digest was mistyped. Read it again with doc_read and use the anchors and sha256 from that read",
            &digest[..16]
        ));
    }
    let context = vak_ooxml::edit::EditContext {
        author: author.clone(),
        date,
    };
    let applied = vak_ooxml::edit::apply(&bytes, ops, &context, limits, Some(target))
        .map_err(|error| format!("nothing was written: {error}"))?;
    write_atomically(destination, &applied.bytes)?;
    let new_digest = sha256_hex(&applied.bytes);
    let mut report = format!(
        "sha256 {}… (use it as base_digest for further edits).\n",
        &new_digest[..16]
    );
    for result in &applied.results {
        report.push_str(&format!(
            "- {}: {} ({})\n",
            result.op, result.summary, result.check
        ));
    }
    match target.vocabulary {
        vak_ooxml::Vocabulary::Word => report.push_str(&format!(
            "Word edits are tracked changes by {author}; a reviewer can accept or reject each in Word.\n"
        )),
        vak_ooxml::Vocabulary::Excel => report.push_str(
            "Vak does not calculate formulas: Excel recalculates when the file is opened, and until then cached values are stale.\n",
        ),
        _ => {}
    }
    Ok(report)
}

fn sha256_hex(bytes: &[u8]) -> String {
    use sha2::Digest as _;
    sha2::Sha256::digest(bytes)
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

/// Writes through a sibling temporary file and a rename, so a reader never
/// sees half a package and a failure leaves the old file in place.
fn write_atomically(destination: &Path, bytes: &[u8]) -> Result<(), String> {
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

/// A file to write inside the workspace: its directory must exist inside
/// the workspace, and it must not be a symlink.
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

    #[tokio::test]
    async fn edits_a_workbook_in_place_and_reports_the_new_digest() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("budget.xlsx");
        std::fs::write(&file, vak_ooxml::fixtures::xlsx()).unwrap();
        let output = run(
            dir.path(),
            serde_json::json!({
                "path": "budget.xlsx",
                "base_digest": digest_of(&file),
                "ops": [{"op": "set_cells", "sheet": "Budget", "cells": {"B2": 150}}]
            }),
        )
        .await;
        assert!(!output.is_error, "{}", output.content);
        assert!(
            output
                .content
                .contains("set_cells: 1 cell(s) set on Budget"),
            "{}",
            output.content
        );
        assert!(
            output
                .content
                .contains(&format!("sha256 {}…", digest_of(&file))),
            "{}",
            output.content
        );
        assert!(
            output
                .content
                .contains("recalculates when the file is opened")
        );

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
    async fn creates_a_deck_from_a_template_with_the_agent_as_author() {
        let dir = tempfile::tempdir().unwrap();
        let template = dir.path().join("brand.pptx");
        std::fs::write(&template, vak_ooxml::fixtures::pptx_template()).unwrap();
        let output = run(
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
        assert!(!output.is_error, "{}", output.content);
        let bytes = std::fs::read(dir.path().join("q3.pptx")).unwrap();
        let document =
            vak_ooxml::read::read(std::io::Cursor::new(bytes), vak_ooxml::Limits::default())
                .unwrap();
        assert!(document.lines().join("\n").contains("Slide 2: Q3"));
        assert_eq!(
            std::fs::read(&template).unwrap(),
            vak_ooxml::fixtures::pptx_template(),
            "the template is untouched"
        );

        let doc = dir.path().join("memo.docx");
        std::fs::write(&doc, vak_ooxml::fixtures::docx()).unwrap();
        let output = run(
            dir.path(),
            serde_json::json!({
                "path": "memo.docx",
                "base_digest": digest_of(&doc),
                "ops": [{"op": "replace_paragraph_text", "anchor": "p@11", "text": "Up."}]
            }),
        )
        .await;
        assert!(
            output.content.contains("tracked changes by mira"),
            "{}",
            output.content
        );
    }

    #[tokio::test]
    async fn refuses_escapes_bad_ops_and_failed_edits_without_writing() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("memo.docx");
        std::fs::write(&file, vak_ooxml::fixtures::docx()).unwrap();
        let before = std::fs::read(&file).unwrap();
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
        assert_eq!(std::fs::read(&file).unwrap(), before);
        assert!(!dir.path().join("memo.docm").exists());
        let leftovers: Vec<_> = std::fs::read_dir(dir.path()).unwrap().collect();
        assert_eq!(leftovers.len(), 1, "no temporary file is left behind");
    }
}
