//! Embedded admin SPA (crates/vak-admin-ui/dist). The frontend is a static
//! SolidJS bundle; every data call goes through the authenticated
//! /admin/api/* routes in `crate::admin`. Serving is read-only and
//! stateless — no session state lives here, auth rides the cookie.

use axum::http::header::{CACHE_CONTROL, CONTENT_TYPE};
use axum::response::IntoResponse;
use axum::routing::get;
use include_dir::{Dir, include_dir};

static ADMIN_UI: Dir<'_> = include_dir!("$CARGO_MANIFEST_DIR/../vak-admin-ui/dist");

/// Recursively collect all file paths in the embedded dist tree.
/// `Dir::files()` in include_dir 0.7 only returns immediate children;
/// assets live in `assets/` subdirectories and must be walked.
/// `File::path()` already returns root-relative paths, so we use them
/// directly.
fn collect_all_files(dir: &Dir<'_>, out: &mut Vec<String>) {
    for file in dir.files() {
        out.push(file.path().to_string_lossy().into_owned());
    }
    for sub in dir.dirs() {
        collect_all_files(sub, out);
    }
}

fn mime_for(path: &str) -> &'static str {
    match path.rsplit('.').next() {
        Some("html") => "text/html; charset=utf-8",
        Some("js") => "text/javascript; charset=utf-8",
        Some("css") => "text/css; charset=utf-8",
        Some("svg") => "image/svg+xml",
        Some("json") => "application/json",
        Some("png") => "image/png",
        Some("ico") => "image/x-icon",
        _ => "application/octet-stream",
    }
}

fn serve_file(path: &str) -> axum::response::Response {
    match ADMIN_UI.get_file(path) {
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

async fn index() -> axum::response::Response {
    serve_file("index.html")
}

pub(crate) fn routes() -> axum::Router<crate::AppState> {
    let mut router = axum::Router::new()
        .route("/admin", get(index))
        .route("/admin/", get(index))
        .route("/favicon.svg", get(|| async { serve_file("favicon.svg") }));

    let mut all_files = Vec::new();
    collect_all_files(&ADMIN_UI, &mut all_files);
    for path in all_files {
        let uri = format!("/admin/{path}");
        router = router.route(&uri, get(move || async move { serve_file(&path) }));
    }
    router
}

#[cfg(test)]
mod embed_tests {
    use super::{ADMIN_UI, collect_all_files};

    #[test]
    fn dist_assets_are_embedded_and_routable() {
        let mut names = Vec::new();
        collect_all_files(&ADMIN_UI, &mut names);
        assert!(
            names.iter().any(|n| n.starts_with("assets/index-")),
            "no hashed assets embedded; got: {names:?}"
        );
        assert!(names.iter().any(|n| n == "index.html"));
    }
}
