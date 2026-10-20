//! Serving an embedded SPA bundle, shared by the operations console
//! (`/admin`) and the workspace client (`/app`).
//!
//! This exists as one module because the naive version of it shipped
//! broken three times. `include_dir`'s `Dir::files()` is NOT recursive —
//! unlike `Dir::get_file`'s lookup — so a route-registration loop that
//! calls it once on the top-level directory never descends into
//! `assets/`. Every file is correctly embedded, `index.html` correctly
//! references the right hashed names, and every one of those requests
//! 404s from axum's router before the (correct) recursive lookup ever
//! runs. The failure reads as "the page loads but nothing on it works",
//! because the shell always renders and the JS never arrives.
//!
//! That is a mistake worth making once, in one place, with a test.

use axum::http::header::{CACHE_CONTROL, CONTENT_TYPE};
use axum::response::IntoResponse;
use axum::routing::get;
use include_dir::Dir;

pub(crate) fn mime_for(path: &str) -> &'static str {
    match path.rsplit('.').next() {
        Some("html") => "text/html; charset=utf-8",
        Some("js") => "text/javascript; charset=utf-8",
        Some("css") => "text/css; charset=utf-8",
        Some("svg") => "image/svg+xml",
        Some("json") => "application/json",
        Some("webmanifest") => "application/manifest+json",
        Some("png") => "image/png",
        Some("jpg" | "jpeg") => "image/jpeg",
        Some("webp") => "image/webp",
        Some("ico") => "image/x-icon",
        Some("woff2") => "font/woff2",
        Some("mp3") => "audio/mpeg",
        _ => "application/octet-stream",
    }
}

/// Where a frame in a page of the app may go. A preview runs in a sandboxed
/// frame whose own policy cannot stop it navigating itself away (a script, a
/// link or a refresh), so the embedding page is what holds it to the app's own
/// files and the loopback origins previews are served from
/// (`preview.rs`); an internet address is refused, whatever the frame does.
const FRAME_POLICY: &str = "frame-src 'self' blob: data: http://127.0.0.1:* http://localhost:*; img-src 'self' data: blob:; media-src 'self' data: blob:; object-src 'none'; base-uri 'self'";

/// Whether the bundler put a content hash in this file's name, which is
/// what makes it safe to cache for good: the files it writes directly into
/// `assets/`. A file copied from `public/` keeps its name when its content
/// changes (`assets/sounds/glass/error.mp3`, a brand image), so it is
/// revalidated against its `ETag` instead, like `index.html`.
fn has_hashed_name(path: &str) -> bool {
    path.strip_prefix("assets/")
        .is_some_and(|name| !name.contains('/'))
}

fn etag_of(contents: &[u8]) -> String {
    use sha2::Digest;
    use std::fmt::Write;
    let digest = sha2::Sha256::digest(contents);
    let mut tag = String::from("\"");
    for byte in &digest[..16] {
        let _ = write!(tag, "{byte:02x}");
    }
    tag.push('"');
    tag
}

/// A routed file with its validator: a request that names the current
/// `ETag` gets `304` and no body.
fn serve_validated(
    dir: &'static Dir<'static>,
    path: &str,
    etag: &str,
    headers: &axum::http::HeaderMap,
) -> axum::response::Response {
    let Ok(value) = axum::http::HeaderValue::from_str(etag) else {
        return serve_file(dir, path);
    };
    let current = headers
        .get(axum::http::header::IF_NONE_MATCH)
        .is_some_and(|sent| sent.as_bytes() == etag.as_bytes());
    let mut response = if current && !has_hashed_name(path) {
        (
            axum::http::StatusCode::NOT_MODIFIED,
            [(CACHE_CONTROL, "no-cache")],
        )
            .into_response()
    } else {
        serve_file(dir, path)
    };
    response
        .headers_mut()
        .insert(axum::http::header::ETAG, value);
    response
}

pub(crate) fn serve_file(dir: &'static Dir<'static>, path: &str) -> axum::response::Response {
    match dir.get_file(path) {
        Some(file) => {
            let cache = if has_hashed_name(path) {
                "public, max-age=31536000, immutable"
            } else {
                "no-cache"
            };
            let mut response = (
                [(CONTENT_TYPE, mime_for(path)), (CACHE_CONTROL, cache)],
                file.contents(),
            )
                .into_response();
            if path.ends_with(".html") {
                // Link hostnames can contain private data too: speculative
                // DNS must not turn an inert answer link into an outbound leak.
                response.headers_mut().insert(
                    "x-dns-prefetch-control",
                    axum::http::HeaderValue::from_static("off"),
                );
                response.headers_mut().insert(
                    axum::http::header::CONTENT_SECURITY_POLICY,
                    axum::http::HeaderValue::from_static(FRAME_POLICY),
                );
            }
            response
        }
        None => (axum::http::StatusCode::NOT_FOUND, "not found").into_response(),
    }
}

/// Register a route for every file under `dir`, recursing into
/// subdirectories. `prefix` is the URL root, e.g. `/admin` or `/app`.
pub(crate) fn register_files(
    mut router: axum::Router<crate::AppState>,
    dir: &'static Dir<'static>,
    root: &'static Dir<'static>,
    prefix: &str,
) -> axum::Router<crate::AppState> {
    for file in dir.files() {
        let path = file.path().to_string_lossy().to_string();
        let uri = format!("{prefix}/{path}");
        let etag = etag_of(file.contents());
        router = router.route(
            &uri,
            get(move |headers: axum::http::HeaderMap| async move {
                serve_validated(root, &path, &etag, &headers)
            }),
        );
    }
    for sub in dir.dirs() {
        router = register_files(router, sub, root, prefix);
    }
    router
}

/// Every asset `index.html` references must be embedded AND routable.
///
/// Two distinct layers, both of which have been broken in shipped
/// releases: v0.8.1 embedded a stale bundle whose hashed names no longer
/// matched, and v0.8.1–v0.8.3 registered no routes under `assets/` at all.
#[cfg(test)]
pub(crate) fn referenced_assets(dir: &'static Dir<'static>, prefix: &str) -> Vec<String> {
    let index = dir
        .get_file("index.html")
        .and_then(|f| f.contents_utf8())
        .unwrap_or_default();
    let needle = format!("{prefix}/assets/");
    index
        .split(['"', '\''])
        .filter(|s| s.starts_with(&needle))
        .map(String::from)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_bundler_hashed_names_are_cached_for_good() {
        assert!(has_hashed_name("assets/index-D4GxP0Q-.js"));
        assert!(!has_hashed_name("assets/sounds/glass/error.mp3"));
        assert!(!has_hashed_name(
            "assets/brand/dimensional/loops/vak-64.webp"
        ));
        assert!(!has_hashed_name("index.html"));
        assert!(!has_hashed_name("characters/vak-64.webp"));
    }

    #[test]
    fn a_sound_is_served_as_audio() {
        assert_eq!(mime_for("assets/sounds/glass/error.mp3"), "audio/mpeg");
    }

    #[test]
    fn an_etag_names_the_content() {
        assert_eq!(etag_of(b"a"), etag_of(b"a"));
        assert_ne!(etag_of(b"a"), etag_of(b"b"));
        assert!(etag_of(b"a").starts_with('"') && etag_of(b"a").ends_with('"'));
    }
}
