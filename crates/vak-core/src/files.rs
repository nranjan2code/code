//! How a workspace file should be presented.
//!
//! Shared by every surface: the desktop editor, the HTTP layer and the TUI
//! `/view` command must agree on what counts as text, or one of them will
//! happily render — or worse, save over — bytes it cannot represent.

use std::path::Path;

/// Editable text, a renderable image, or opaque bytes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FileKind {
    Text,
    Image,
    Binary,
}

impl FileKind {
    pub fn as_str(self) -> &'static str {
        match self {
            FileKind::Text => "text",
            FileKind::Image => "image",
            FileKind::Binary => "binary",
        }
    }

    /// Only text can be round-tripped through an editor without loss.
    pub fn editable(self) -> bool {
        self == FileKind::Text
    }
}

fn extension_of(path: &Path) -> String {
    path.extension()
        .and_then(|e| e.to_str())
        .unwrap_or("")
        .to_ascii_lowercase()
}

/// Content type for an image path, for building data URLs.
pub fn mime_for(path: &Path) -> &'static str {
    match extension_of(path).as_str() {
        "png" => "image/png",
        "jpg" | "jpeg" => "image/jpeg",
        "gif" => "image/gif",
        "webp" => "image/webp",
        "bmp" => "image/bmp",
        "ico" => "image/x-icon",
        "svg" => "image/svg+xml",
        _ => "application/octet-stream",
    }
}

/// Classify by extension first, then by content: a NUL byte or invalid UTF-8
/// means the bytes are not text whatever the name claims.
pub fn classify(path: &Path, bytes: &[u8]) -> FileKind {
    if matches!(
        extension_of(path).as_str(),
        "png" | "jpg" | "jpeg" | "gif" | "webp" | "bmp" | "ico"
    ) {
        return FileKind::Image;
    }
    // SVG is XML: renderable *and* meaningfully editable, so it stays text.
    if bytes.contains(&0) || std::str::from_utf8(bytes).is_err() {
        return FileKind::Binary;
    }
    FileKind::Text
}

/// Human-readable byte count for status lines.
pub fn format_bytes(n: u64) -> String {
    if n < 1024 {
        format!("{n} B")
    } else if n < 1024 * 1024 {
        format!("{:.1} KB", n as f64 / 1024.0)
    } else {
        format!("{:.1} MB", n as f64 / (1024.0 * 1024.0))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn nul_bytes_are_binary_even_with_a_text_extension() {
        assert_eq!(classify(Path::new("a.txt"), b"ok\0bad"), FileKind::Binary);
    }

    #[test]
    fn image_extensions_classify_before_content() {
        assert_eq!(classify(Path::new("a.png"), b"\x89PNG"), FileKind::Image);
    }

    #[test]
    fn svg_stays_editable_text() {
        let k = classify(Path::new("a.svg"), b"<svg></svg>");
        assert_eq!(k, FileKind::Text);
        assert!(k.editable());
    }

    #[test]
    fn invalid_utf8_is_binary() {
        assert_eq!(
            classify(Path::new("a.dat"), &[0xff, 0xfe]),
            FileKind::Binary
        );
    }

    #[test]
    fn plain_source_is_text() {
        assert_eq!(classify(Path::new("a.py"), b"print(1)\n"), FileKind::Text);
    }
}
