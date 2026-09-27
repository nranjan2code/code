//! `vak pdf`: the PDF reader for scripts and headless machines
//! (docs/design/77-pdf-documents.md). Each verb only parses its arguments
//! and prints the worker's JSON answer: reading and verifying run in the
//! same broker worker as the Agent's `doc_read` and the Review screen's
//! checks, so a file is never parsed in this process (invariant 14).

use crate::cli::PdfAction;
use crate::office::{absolute, fail, print, verify};

pub(crate) async fn run_pdf(action: PdfAction) -> i32 {
    let worker = match std::env::current_exe() {
        Ok(executable) => executable,
        Err(error) => return fail(&format!("tool broker unavailable: {error}")),
    };
    match action {
        PdfAction::Read {
            file,
            from,
            at,
            facts,
        } => {
            use vak_tools::broker::PdfView;
            let view = match (facts, at) {
                (true, _) => PdfView::Facts,
                (_, Some(anchor)) => PdfView::At { anchor },
                _ => PdfView::Content { from },
            };
            let answer = match absolute(&file) {
                Ok(path) => vak_tools::broker::pdf_read(&worker, &path, view).await,
                Err(error) => Err(error),
            };
            match answer {
                Ok(value) => print(&value),
                Err(error) => fail(&error),
            }
        }
        PdfAction::Verify { file } => verify(&worker, &file, "format.pdf-structure").await,
    }
}
