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

/// Registers a route for every file under `dir`, recursing into
/// subdirectories.
///
/// `Dir::files()` only lists the entries directly inside `dir` — it is
/// not recursive, unlike `Dir::get_entry`'s lookup (which `serve_file`
/// uses). The previous version of this function called `.files()` once
/// on the top-level embedded directory and never descended into
/// `assets/`, so no route was ever registered for anything under it: a
/// request for `/admin/assets/<hash>.js` 404'd from axum's router
/// itself, before `serve_file`'s otherwise-correct recursive lookup
/// ever ran. `index.html`, at the top level, happened to work — which
/// is exactly why the failure read as "the page loads, but nothing on
/// it does": the shell always rendered, and every asset it needed
/// always 404'd.
fn register_files(
    mut router: axum::Router<crate::AppState>,
    dir: &Dir<'_>,
) -> axum::Router<crate::AppState> {
    for file in dir.files() {
        let path = file.path().to_string_lossy().to_string();
        let uri = format!("/admin/{path}");
        router = router.route(&uri, get(move || async move { serve_file(&path) }));
    }
    for sub in dir.dirs() {
        router = register_files(router, sub);
    }
    router
}

pub(crate) fn routes() -> axum::Router<crate::AppState> {
    let router = axum::Router::new()
        .route("/admin", get(index))
        .route("/admin/", get(index))
        // dist/favicon.svg is served by register_files below at
        // /admin/favicon.svg; the bare /favicon.svg alias is registered
        // here since it lives outside that prefix.
        .route("/favicon.svg", get(|| async { serve_file("favicon.svg") }));
    register_files(router, &ADMIN_UI)
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

    /// The test above checks embedding; this checks routing — a
    /// different layer, and the one that was actually broken.
    /// `Dir::files()` is not recursive, so the previous route
    /// registration loop never reached anything under `assets/`: every
    /// file was correctly embedded, `index.html` correctly referenced
    /// the right hashed filenames, and every one of those requests
    /// still 404'd because axum had no route for them at all. Confirmed
    /// live in a browser before this fix, reproduced here by driving
    /// the actual `Router` `routes()` builds — the same object the real
    /// server serves from — through every asset URL `index.html`
    /// references, exactly as a browser would request them.
    #[tokio::test]
    async fn every_referenced_asset_is_actually_routable() {
        use http_body_util::BodyExt as _;
        use tower::ServiceExt as _;

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
            "nothing to check — extraction is not matching this markup"
        );

        for url_path in referenced {
            let router = routes().with_state(test_state());
            let response = router
                .oneshot(
                    axum::http::Request::builder()
                        .uri(url_path)
                        .body(axum::body::Body::empty())
                        .unwrap(),
                )
                .await
                .unwrap();
            assert_eq!(
                response.status(),
                axum::http::StatusCode::OK,
                "GET {url_path} did not route to 200 — this is the exact defect \
                 that shipped in v0.8.1, v0.8.2, and v0.8.3's first attempt at fixing it"
            );
            let bytes = response.into_body().collect().await.unwrap().to_bytes();
            assert!(
                !bytes.is_empty(),
                "GET {url_path} routed but returned an empty body"
            );
        }
    }

    fn test_state() -> crate::AppState {
        let dir = tempfile::tempdir().unwrap();
        let core = vak_core::Core::new(dir.path().to_path_buf()).unwrap();
        crate::AppState::new(core)
    }
}
