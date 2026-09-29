//! `vak office`: the Office and PDF engines for scripts and headless
//! machines (docs/design/72-openxml-documents.md P5,
//! docs/design/77-pdf-documents.md). Each verb only parses its
//! arguments and prints the worker's JSON answer: reading, applying,
//! comparing and verifying run in the same broker worker tasks, over the
//! same op schema, as the Agent's tools and the Review screen, so a file is
//! never parsed in this process (invariants 14 and 39).

use std::io::Read as _;
use std::path::{Path, PathBuf};

use crate::cli::OfficeAction;

pub(crate) async fn run_office(action: OfficeAction) -> i32 {
    let worker = match std::env::current_exe() {
        Ok(executable) => executable,
        Err(error) => return fail(&format!("tool broker unavailable: {error}")),
    };
    let answer = match action {
        OfficeAction::Read {
            file,
            from,
            at,
            structure,
            facts,
        } => read(&worker, &file, from, at, structure, facts).await,
        OfficeAction::Apply {
            file,
            ops,
            base_digest,
            out,
            agent,
        } => {
            apply(
                &worker,
                file.as_deref(),
                &ops,
                base_digest.as_deref(),
                &out,
                &agent,
            )
            .await
        }
        OfficeAction::Diff { before, after } => diff(&worker, &before, &after).await,
        OfficeAction::Verify { file } => {
            let verifier = if vak_pdf::is_pdf_path(&file.to_string_lossy()) {
                "format.pdf-structure"
            } else {
                "format.openxml"
            };
            return verify(&worker, &file, verifier).await;
        }
    };
    match answer {
        Ok(value) => print(&value),
        Err(error) => fail(&error),
    }
}

async fn read(
    worker: &Path,
    file: &Path,
    from: usize,
    at: Option<String>,
    structure: bool,
    facts: bool,
) -> Result<serde_json::Value, String> {
    use vak_tools::broker::OfficeView;
    let view = match (structure, facts, at) {
        (true, _, _) => OfficeView::Structure,
        (_, true, _) => OfficeView::Facts,
        (_, _, Some(anchor)) => OfficeView::At { anchor },
        _ => OfficeView::Content { from },
    };
    vak_tools::broker::office_project(worker, &absolute(file)?, view).await
}

async fn apply(
    worker: &Path,
    file: Option<&Path>,
    ops: &str,
    base_digest: Option<&str>,
    out: &Path,
    agent: &str,
) -> Result<serde_json::Value, String> {
    let text = if ops == "-" {
        let mut text = String::new();
        std::io::stdin()
            .read_to_string(&mut text)
            .map_err(|error| format!("could not read ops from stdin: {error}"))?;
        text
    } else {
        std::fs::read_to_string(ops).map_err(|error| format!("could not read {ops}: {error}"))?
    };
    let pdf = vak_pdf::is_pdf_path(&out.to_string_lossy());
    let (ops, pdf_ops): (Vec<vak_ooxml::edit::OfficeOp>, Vec<vak_pdf::edit::PdfOp>) = if pdf {
        let ops = serde_json::from_str(&text).map_err(|error| {
            format!("ops are not valid for a PDF: {error}. Pass a JSON array of ops, each an object with an \"op\" name and only that op's fields")
        })?;
        (Vec::new(), ops)
    } else {
        let ops = serde_json::from_str(&text).map_err(|error| {
            format!("ops are not valid: {error}. Pass a JSON array of ops, each an object with an \"op\" name and only that op's fields")
        })?;
        (ops, Vec::new())
    };
    let origin = match (file, base_digest) {
        (Some(file), Some(base_digest)) => vak_tools::broker::OfficeOrigin::File {
            path: absolute(file)?,
            base_digest: base_digest.to_string(),
        },
        (None, None) => vak_tools::broker::OfficeOrigin::Blank,
        (Some(_), None) => {
            return Err(
                "--base-digest is required with a file: pass the sha256 `vak office read` printed for it"
                    .into(),
            );
        }
        (None, Some(_)) => {
            return Err(
                "--base-digest names the content of a file; give the file, or leave both out to create --out from scratch"
                    .into(),
            );
        }
    };
    // A new file from the blank or a template is written clean; anything
    // else is an edit of the file, tracked (docs/design/72, R7).
    let new_file = file.is_none_or(|file| {
        file.extension()
            .and_then(|extension| extension.to_str())
            .and_then(vak_ooxml::Format::from_extension)
            .is_some_and(|format| format.kind == vak_ooxml::FormatKind::Template)
    });
    let lineage = vak_tools::broker::OfficeLineage {
        origin,
        planned_ops: vak_ooxml::edit::plan_chart_locations(&ops).map_err(|error| error.message)?,
        ops,
        author: vak_tools::office_apply::tracked_change_author(agent),
        new_file,
        pdf: if pdf { vec![pdf_ops] } else { Vec::new() },
    };
    vak_tools::broker::office_apply_to(worker, &lineage, &absolute(out)?).await
}

async fn diff(worker: &Path, before: &Path, after: &Path) -> Result<serde_json::Value, String> {
    vak_tools::broker::office_review(worker, Some(&absolute(before)?), &absolute(after)?, None)
        .await
}

/// Runs one target verifier on a file in the broker worker. Exits 0 when
/// the file passes, 1 when it fails.
pub(crate) async fn verify(worker: &Path, file: &Path, verifier: &str) -> i32 {
    let (root, name) = match absolute(file).and_then(|path| {
        let name = path
            .file_name()
            .and_then(|name| name.to_str())
            .map(str::to_string)
            .ok_or_else(|| format!("{} has no file name", path.display()))?;
        let root = path
            .parent()
            .map(Path::to_path_buf)
            .ok_or_else(|| format!("{} has no directory", path.display()))?;
        Ok((root, name))
    }) {
        Ok(parts) => parts,
        Err(error) => return fail(&error),
    };
    let checks = [vak_sandbox::TargetCheckPlan {
        verifier: verifier.into(),
        path: name,
    }];
    let results = vak_tools::broker::verify_targets(worker, &root, &checks).await;
    let passed = results.iter().all(|result| result.status == "passed");
    let code = print(&serde_json::json!({ "passed": passed, "checks": results }));
    if code != 0 {
        code
    } else if passed {
        0
    } else {
        1
    }
}

pub(crate) fn absolute(path: &Path) -> Result<PathBuf, String> {
    std::path::absolute(path).map_err(|error| format!("{}: {error}", path.display()))
}

pub(crate) fn print(value: &serde_json::Value) -> i32 {
    match serde_json::to_string_pretty(value) {
        Ok(text) => {
            println!("{text}");
            0
        }
        Err(error) => fail(&error.to_string()),
    }
}

pub(crate) fn fail(message: &str) -> i32 {
    eprintln!("{}", serde_json::json!({ "error": message }));
    2
}
