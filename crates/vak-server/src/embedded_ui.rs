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
        _ => "application/octet-stream",
    }
}

pub(crate) fn serve_file(dir: &'static Dir<'static>, path: &str) -> axum::response::Response {
    match dir.get_file(path) {
        Some(file) => {
            // Hashed asset filenames are immutable; index.html must always
            // revalidate so deploys pick up new hashes.
            let cache = if path.starts_with("assets/") {
                "public, max-age=31536000, immutable"
            } else {
                "no-cache"
            };
            (
                [(CONTENT_TYPE, mime_for(path)), (CACHE_CONTROL, cache)],
                file.contents(),
            )
                .into_response()
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
        router = router.route(&uri, get(move || async move { serve_file(root, &path) }));
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
