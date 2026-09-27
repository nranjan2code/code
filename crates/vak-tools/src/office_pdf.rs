//! PDFs in the Office loop (docs/design/77-pdf-documents.md): the PDF half
//! of `office_apply` and of the worker's review, narrowing, projection and
//! apply tasks. Every call runs in the broker worker, like every Office
//! read and write (invariants 14 and 39); only the engine differs.

use std::path::{Path, PathBuf};

use serde_json::Value;

use crate::broker::{OfficeLineage, OfficeOrigin, OfficeView};
use crate::office_apply::{sha256_hex, write_atomically};

/// The ops a PDF takes, as the tool names them.
pub(crate) const OPS: &str = "replace_paragraph_text, delete_paragraph, add_paragraph, add_table, add_page_break, set_title, add_comment, highlight, fill_field, rotate_page, delete_page and move_page";

pub(crate) fn context(author: &str) -> vak_pdf::edit::EditContext {
    vak_pdf::edit::EditContext {
        author: author.to_string(),
        date: chrono::Utc::now().format("%Y-%m-%dT%H:%M:%SZ").to_string(),
    }
}

pub(crate) fn read_bounded(path: &Path) -> Result<Vec<u8>, String> {
    let limit = vak_pdf::Limits::default().max_file_bytes;
    let size = std::fs::metadata(path)
        .map_err(|error| format!("cannot read {}: {error}", path.display()))?
        .len();
    if size > limit {
        return Err(format!(
            "{} is over the size limit for a PDF",
            path.display()
        ));
    }
    std::fs::read(path).map_err(|error| format!("cannot read {}: {error}", path.display()))
}

fn read(path: &Path) -> Result<vak_pdf::Document, String> {
    let bytes = read_bounded(path)?;
    vak_pdf::read(&bytes, vak_pdf::Limits::default())
        .map_err(|error| format!("{} could not be read: {error}", path.display()))
}

/// The bytes a lineage starts from: its file, refused unless it still has
/// the digest the draft was made against; `None` for the blank.
fn source(origin: &OfficeOrigin) -> Result<Option<Vec<u8>>, String> {
    let (path, base_digest) = match origin {
        OfficeOrigin::File { path, base_digest } => (path, base_digest),
        OfficeOrigin::Blank => return Ok(None),
    };
    let bytes = read_bounded(path)?;
    if !vak_pdf::sniff(&bytes) {
        return Err(format!(
            "{} is not a PDF; a PDF is made from a PDF or from scratch",
            path.display()
        ));
    }
    let digest = sha256_hex(&bytes);
    let expected = base_digest
        .trim()
        .trim_end_matches('…')
        .to_ascii_lowercase();
    if expected.len() < 16 || !digest.starts_with(&expected) {
        return Err(format!(
            "base_digest {expected:?} does not match the source (sha256 {}…); the file changed since it was read or the digest was mistyped. Pass the sha256 exactly as the read shows it, `{}…`, without completing it; if the file changed, read it again (doc_read, or `vak office read`) and use the anchors and sha256 from that read",
            &digest[..16],
            &digest[..16]
        ));
    }
    Ok(Some(bytes))
}

/// Applies each step to the result of the one before, starting from
/// `origin`. Returns the starting bytes and the last step's result;
/// nothing is written here.
pub(crate) fn apply_checked(
    origin: &OfficeOrigin,
    steps: &[Vec<vak_pdf::edit::PdfOp>],
    context: &vak_pdf::edit::EditContext,
) -> Result<(Option<Vec<u8>>, vak_pdf::edit::Applied), String> {
    let start = source(origin)?;
    let mut current = start.clone();
    let mut last = None;
    for ops in steps {
        let applied =
            vak_pdf::edit::apply(current.as_deref(), ops, context, vak_pdf::Limits::default())
                .map_err(|error| format!("nothing was written: {error}"))?;
        current = Some(applied.bytes.clone());
        last = Some(applied);
    }
    let applied = last.ok_or("nothing was written: there are no ops")?;
    Ok((start, applied))
}

/// One `office_apply` call on a PDF: the draft and the report the model
/// reads.
pub(crate) struct Job {
    pub(crate) origin: OfficeOrigin,
    pub(crate) destination: PathBuf,
    pub(crate) draft: PathBuf,
    pub(crate) ops: Vec<vak_pdf::edit::PdfOp>,
    pub(crate) context: vak_pdf::edit::EditContext,
}

impl Job {
    pub(crate) fn run(self) -> Result<String, String> {
        let (_, applied) = apply_checked(&self.origin, &[self.ops], &self.context)?;
        let current = if self.destination.is_file() {
            Some(read(&self.destination).map_err(|error| {
                format!("the current workspace file cannot be read for comparison: {error}")
            })?)
        } else {
            None
        };
        let changes = vak_pdf::diff::diff(current.as_ref(), &applied.document);
        if let Some(parent) = self.draft.parent() {
            std::fs::create_dir_all(parent)
                .map_err(|error| format!("cannot create the draft directory: {error}"))?;
        }
        write_atomically(&self.draft, &applied.bytes)?;
        let mut report = format!("Draft sha256 {}….\n", &sha256_hex(&applied.bytes)[..16]);
        for result in &applied.results {
            report.push_str(&format!("- {result}\n"));
        }
        for notice in &applied.notices {
            report.push_str(&format!("Note: {notice}\n"));
        }
        if current.is_none() {
            report.push_str(&format!(
                "It is a new file: a PDF of {} pages.\n",
                applied.document.page_count
            ));
        } else if changes.summary.is_empty() {
            report.push_str("Compared with the workspace file: no visible change.\n");
        } else {
            report.push_str("Compared with the workspace file: ");
            report.push_str(&changes.summary.join("; "));
            report.push('\n');
        }
        Ok(report)
    }
}

fn steps(lineage: &OfficeLineage) -> &[Vec<vak_pdf::edit::PdfOp>] {
    &lineage.pdf
}

fn author(lineage: &OfficeLineage) -> vak_pdf::edit::EditContext {
    context(&lineage.author)
}

/// A draft compared with the file it would replace, with the changes a
/// person can keep, in the Office review's shape.
pub(crate) fn review_in_worker(
    before: Option<&Path>,
    after: &Path,
    lineage: Option<&OfficeLineage>,
) -> Result<String, String> {
    let current = before.map(read).transpose()?;
    let draft = read(after)?;
    let diff = vak_pdf::diff::diff(current.as_ref(), &draft);
    let mut body = serde_json::json!({
        "summary": diff.summary,
        "changes": diff.changes,
        "flags": draft.inspection.flags(),
        "impact": vak_pdf::diff::impact(current.as_ref(), &draft),
    });
    let choices = match lineage {
        None => Err("the draft was not made by office_apply in this conversation".to_string()),
        Some(lineage) => source(&lineage.origin).and_then(|source| {
            vak_pdf::review::choices(
                source.as_deref(),
                steps(lineage),
                &author(lineage),
                vak_pdf::Limits::default(),
                &draft,
            )
        }),
    };
    match choices {
        Ok(choices) => body["choices"] = serde_json::json!(choices),
        Err(reason) => body["choices_unavailable"] = Value::String(reason),
    }
    serde_json::to_string(&body).map_err(|error| error.to_string())
}

pub(crate) fn narrow_in_worker(
    lineage: &OfficeLineage,
    draft: &Path,
    keep: &[String],
    out: &Path,
) -> Result<String, String> {
    let source = source(&lineage.origin)?;
    let draft = read(draft)?;
    let applied = vak_pdf::review::narrow(
        source.as_deref(),
        steps(lineage),
        keep,
        &author(lineage),
        vak_pdf::Limits::default(),
        &draft,
    )
    .map_err(|error| format!("nothing was written: {error}"))?;
    write_atomically(out, &applied.bytes)?;
    serde_json::to_string(&serde_json::json!({
        "results": applied.results,
        "sha256": sha256_hex(&applied.bytes),
    }))
    .map_err(|error| error.to_string())
}

pub(crate) fn apply_in_worker(lineage: &OfficeLineage, out: &Path) -> Result<String, String> {
    if std::fs::symlink_metadata(out).is_ok() {
        return Err(format!(
            "{} already exists; name a new file for the result",
            out.display()
        ));
    }
    let (source, applied) = apply_checked(&lineage.origin, steps(lineage), &author(lineage))?;
    let before = source
        .map(|bytes| vak_pdf::read(&bytes, vak_pdf::Limits::default()))
        .transpose()
        .map_err(|error| format!("the source could not be read: {error}"))?;
    write_atomically(out, &applied.bytes)?;
    serde_json::to_string(&serde_json::json!({
        "path": out,
        "sha256": sha256_hex(&applied.bytes),
        "results": applied.results,
        "notices": applied.notices,
        "changes": vak_pdf::diff::diff(before.as_ref(), &applied.document),
        "impact": vak_pdf::diff::impact(before.as_ref(), &applied.document),
    }))
    .map_err(|error| error.to_string())
}

pub(crate) fn project_in_worker(path: &Path, view: OfficeView) -> Result<String, String> {
    let bytes = read_bounded(path)?;
    let sha256 = sha256_hex(&bytes);
    let document = vak_pdf::read(&bytes, vak_pdf::Limits::default())
        .map_err(|error| format!("{} could not be read: {error}", path.display()))?;
    let budget = vak_pdf::projection::PAGE_BYTES;
    let (from, focus) = match &view {
        OfficeView::Content { from } => (*from, None),
        OfficeView::At { anchor } => match vak_pdf::projection::locate(&document, anchor) {
            Some(index) => (
                vak_pdf::projection::page_start(&document, index, budget),
                Some(anchor.clone()),
            ),
            None => (0, None),
        },
        OfficeView::Facts => (usize::MAX, None),
        OfficeView::Structure => {
            return Err("a PDF has no package parts; ask for its facts or content instead".into());
        }
    };
    let mut page = serde_json::to_value(vak_pdf::projection::project(&document, from, budget))
        .map_err(|error| error.to_string())?;
    if let Some(focus) = focus {
        page["focus"] = Value::String(focus);
    }
    page["sha256"] = Value::String(sha256);
    serde_json::to_string(&page).map_err(|error| error.to_string())
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::*;
    use crate::{Tool, ToolContext};

    async fn call(dir: &Path, args: Value) -> crate::ToolOutput {
        crate::office_apply::OfficeApplyTool
            .execute(&args, &ToolContext::new(dir.to_path_buf()))
            .await
    }

    fn draft_of(output: &str) -> String {
        output
            .split("written to ")
            .nth(1)
            .and_then(|rest| rest.split(". ").next())
            .unwrap()
            .to_string()
    }

    #[tokio::test]
    async fn office_apply_creates_and_edits_a_pdf_as_a_draft() {
        let dir = tempfile::tempdir().unwrap();
        let created = call(
            dir.path(),
            serde_json::json!({"path": "plan.pdf", "ops": [
                {"op": "add_paragraph", "text": "Plan", "style": "Title"},
                {"op": "add_paragraph", "text": "Ship the reader first."}
            ]}),
        )
        .await;
        assert!(!created.is_error, "{}", created.content);
        assert!(
            created
                .content
                .contains("It is a new file: a PDF of 1 pages"),
            "{}",
            created.content
        );
        assert!(
            !dir.path().join("plan.pdf").exists(),
            "the workspace is unchanged until review"
        );
        let draft = dir.path().join(draft_of(&created.content));
        let document =
            vak_pdf::read(&std::fs::read(&draft).unwrap(), vak_pdf::Limits::default()).unwrap();
        assert!(
            document
                .lines()
                .join("\n")
                .contains("Ship the reader first.")
        );

        std::fs::write(dir.path().join("report.pdf"), vak_pdf::fixtures::report()).unwrap();
        let digest = sha256_hex(&vak_pdf::fixtures::report())[..16].to_string();
        let edited = call(
            dir.path(),
            serde_json::json!({"path": "report.pdf", "base_digest": digest, "ops": [
                {"op": "replace_paragraph_text", "anchor": "page:1/line:2", "text": "Revenue grew 15%"},
                {"op": "add_comment", "anchor": "page:1/line:1", "text": "Sourced from Q3 close"}
            ]}),
        )
        .await;
        assert!(!edited.is_error, "{}", edited.content);
        assert!(
            edited
                .content
                .contains("“Revenue grew 12%” → “Revenue grew 15%”"),
            "{}",
            edited.content
        );
        assert!(
            edited
                .content
                .contains("Compared with the workspace file: Page 1:"),
            "{}",
            edited.content
        );
        assert_eq!(
            std::fs::read(dir.path().join("report.pdf")).unwrap(),
            vak_pdf::fixtures::report(),
            "the source is untouched"
        );
    }

    #[tokio::test]
    async fn office_apply_refuses_a_pdf_call_with_a_reason() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("report.pdf"), vak_pdf::fixtures::report()).unwrap();
        let stale = call(
            dir.path(),
            serde_json::json!({"path": "report.pdf", "base_digest": "0000000000000000", "ops": [{"op": "set_title", "title": "x"}]}),
        )
        .await;
        assert!(
            stale.is_error && stale.content.contains("does not match the source"),
            "{}",
            stale.content
        );
        let office_op = call(
            dir.path(),
            serde_json::json!({"path": "new.pdf", "ops": [{"op": "set_cells", "sheet": "A", "cells": {"A1": 1}}]}),
        )
        .await;
        assert!(
            office_op.is_error && office_op.content.contains("A PDF takes"),
            "{}",
            office_op.content
        );
    }

    #[test]
    fn the_worker_reviews_narrows_and_projects_a_pdf_draft() {
        let dir = tempfile::tempdir().unwrap();
        let source = dir.path().join("report.pdf");
        std::fs::write(&source, vak_pdf::fixtures::report()).unwrap();
        let ops: Vec<vak_pdf::edit::PdfOp> = serde_json::from_value(serde_json::json!([
            {"op": "replace_paragraph_text", "anchor": "page:1/line:2", "text": "Revenue grew 15%"},
            {"op": "set_title", "title": "Revised"}
        ]))
        .unwrap();
        let lineage = OfficeLineage {
            origin: OfficeOrigin::File {
                path: source.clone(),
                base_digest: sha256_hex(&vak_pdf::fixtures::report())[..16].to_string(),
            },
            ops: Vec::new(),
            pdf: vec![ops],
            author: "vak".into(),
            new_file: false,
        };
        let draft = dir.path().join("draft.pdf");
        let applied: Value =
            serde_json::from_str(&apply_in_worker(&lineage, &draft).unwrap()).unwrap();
        assert_eq!(applied["results"].as_array().unwrap().len(), 2);
        let review: Value =
            serde_json::from_str(&review_in_worker(Some(&source), &draft, Some(&lineage)).unwrap())
                .unwrap();
        assert_eq!(review["choices"].as_array().unwrap().len(), 2, "{review}");
        assert!(
            review["changes"].to_string().contains("Revenue grew 15%"),
            "{review}"
        );
        let narrowed = dir.path().join("narrowed.pdf");
        narrow_in_worker(&lineage, &draft, &["1".into()], &narrowed).unwrap();
        let document = vak_pdf::read(
            &std::fs::read(&narrowed).unwrap(),
            vak_pdf::Limits::default(),
        )
        .unwrap();
        assert_eq!(document.title(), Some("Revised"));
        assert!(document.lines().join("\n").contains("Revenue grew 12%"));
        let projection: Value = serde_json::from_str(
            &project_in_worker(
                &draft,
                OfficeView::At {
                    anchor: "page:2/line:1".into(),
                },
            )
            .unwrap(),
        )
        .unwrap();
        assert_eq!(projection["vocabulary"], "pdf");
        assert_eq!(projection["focus"], "page:2/line:1");
        assert_eq!(projection["sha256"].as_str().unwrap().len(), 64);
    }
}
