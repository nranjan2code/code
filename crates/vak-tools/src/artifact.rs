use std::path::Path;

use crate::SandboxEventSink;

/// Report a file produced by any built-in or delegated tool. The event uses a
/// workspace-relative path so the client can resolve it through the same
/// authenticated filesystem surface as every other result.
pub fn emit_file(sink: Option<&SandboxEventSink>, path: &Path, cwd: &Path) {
    let Some(sink) = sink else { return };
    let Ok(metadata) = path.metadata() else {
        return;
    };
    if !metadata.is_file() {
        return;
    }
    let relative = path.strip_prefix(cwd).unwrap_or(path).display().to_string();
    sink.emit_artifact(&relative, &guess_mime_type(path), metadata.len());
}

pub fn guess_mime_type(path: &Path) -> String {
    let ext = path
        .extension()
        .and_then(|s| s.to_str())
        .unwrap_or("")
        .to_ascii_lowercase();
    match ext.as_str() {
        "png" => "image/png",
        "jpg" | "jpeg" => "image/jpeg",
        "gif" => "image/gif",
        "svg" => "image/svg+xml",
        "webp" => "image/webp",
        "html" | "htm" => "text/html",
        "css" => "text/css",
        "js" | "mjs" => "application/javascript",
        "json" => "application/json",
        "csv" => "text/csv",
        "tsv" => "text/tab-separated-values",
        "md" => "text/markdown",
        "txt" | "log" => "text/plain",
        "pdf" => "application/pdf",
        "zip" => "application/zip",
        "tar" => "application/x-tar",
        "gz" | "gzip" => "application/gzip",
        "doc" => "application/msword",
        "docx" => "application/vnd.openxmlformats-officedocument.wordprocessingml.document",
        "xls" => "application/vnd.ms-excel",
        "xlsx" => "application/vnd.openxmlformats-officedocument.spreadsheetml.sheet",
        "ppt" => "application/vnd.ms-powerpoint",
        "pptx" => "application/vnd.openxmlformats-officedocument.presentationml.presentation",
        "mp3" | "wav" | "ogg" | "m4a" | "aac" => "audio/*",
        "mp4" | "webm" | "mov" | "m4v" => "video/*",
        _ => "application/octet-stream",
    }
    .to_string()
}
