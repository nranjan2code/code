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
        // dist/favicon.svg is served by the dynamic loop below at
        // /admin/favicon.svg; the bare /favicon.svg alias is registered
        // here since it lives outside that prefix.
        .route("/favicon.svg", get(|| async { serve_file("favicon.svg") }));

    for file in ADMIN_UI.files() {
        let path = file.path().to_string_lossy().to_string();
        let uri = format!("/admin/{path}");
        router = router.route(&uri, get(move || async move { serve_file(&path) }));
    }
    router
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;

    /// Every asset `index.html` references by URL must actually be
    /// embedded, or the admin console loads a blank shell with nothing
    /// in the console to say why: no JS runs, so nothing errors.
    ///
    /// This is the same failure `crates/vak-server/build.rs` exists to
    /// prevent — `include_dir!` has no way to know dist/ is a build
    /// input unless a build script says so, and without one a `cargo
    /// build` after `npm run build` could reuse an incremental build of
    /// this crate from before the rebuild, silently embedding an
    /// `index.html` that references hashed filenames the embedded
    /// directory no longer contains. That shipped as part of v0.8.1.
    /// This test cannot substitute for the build script — a stale
    /// cached artifact that already happened to pass it once would keep
    /// passing — but it does prove the two are consistent in whatever
    /// this test run actually compiled, and a clean build (which CI and
    /// `scripts/release.sh` both do) makes that proof real.
    #[test]
    fn every_asset_index_html_references_is_actually_embedded() {
        let index = ADMIN_UI
            .get_file("index.html")
            .expect("index.html must be embedded")
            .contents_utf8()
            .expect("index.html must be UTF-8");

        let referenced: Vec<&str> = index
            .split(['"', '\''])
            .filter(|s| s.starts_with("/admin/assets/"))
            .collect();
        assert!(
            !referenced.is_empty(),
            "index.html references no /admin/assets/ paths — the extraction above is not \
             matching this build's markup, not evidence there is nothing to check"
        );

        for url_path in referenced {
            let embedded_path = url_path.trim_start_matches("/admin/");
            assert!(
                ADMIN_UI.get_file(embedded_path).is_some(),
                "index.html references {url_path}, but no such file is embedded — \
                 this is the exact defect that shipped in v0.8.1"
            );
        }
    }
}
