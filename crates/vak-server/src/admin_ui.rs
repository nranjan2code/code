//! Embedded admin SPA (crates/vak-admin-ui/dist). The frontend is a static
//! SolidJS bundle; every data call goes through the authenticated
//! /admin/api/* routes in `crate::admin`. Serving is read-only and
//! stateless — no session state lives here, auth rides the cookie.

use axum::http::header::{CACHE_CONTROL, CONTENT_TYPE};
use axum::response::IntoResponse;
use axum::routing::get;
use include_dir::{Dir, include_dir};

static ADMIN_UI: Dir<'_> = include_dir!("$CARGO_MANIFEST_DIR/../vak-admin-ui/dist");

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

    for file in ADMIN_UI.files() {
        let path = file.path().to_string_lossy().to_string();
        let uri = format!("/admin/{path}");
        router = router.route(&uri, get(move || async move { serve_file(&path) }));
    }
    router
}
