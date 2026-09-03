//! Embedded admin SPA (crates/vak-admin-ui/dist). The frontend is a static
//! SolidJS bundle; every data call goes through the authenticated
//! /admin/api/* routes in `crate::admin`. Serving is read-only and
//! stateless — no session state lives here, auth rides the cookie.
//!
//! The serving mechanics are shared with the workspace client at `/app`;
//! see `crate::embedded_ui` for why they live in one place.

use axum::routing::get;
use include_dir::{Dir, include_dir};

use crate::embedded_ui::{register_files, serve_file};

static ADMIN_UI: Dir<'_> = include_dir!("$CARGO_MANIFEST_DIR/../vak-admin-ui/dist");

async fn index() -> axum::response::Response {
    serve_file(&ADMIN_UI, "index.html")
}

pub(crate) fn routes() -> axum::Router<crate::AppState> {
    let router = axum::Router::new()
        .route("/admin", get(index))
        .route("/admin/", get(index))
        // dist/favicon.svg is served by register_files below at
        // /admin/favicon.svg; the bare /favicon.svg alias is registered
        // here since it lives outside that prefix.
        .route("/favicon.svg", get(|| async { serve_file(&ADMIN_UI, "favicon.svg") }));
    register_files(router, &ADMIN_UI, &ADMIN_UI, "/admin")
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;
    use crate::embedded_ui::referenced_assets;

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
    #[test]
    fn every_asset_index_html_references_is_actually_embedded() {
        let referenced = referenced_assets(&ADMIN_UI, "/admin");
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
    /// registration loop never reached anything under `assets/`.
    #[tokio::test]
    async fn every_referenced_asset_is_actually_routable() {
        use http_body_util::BodyExt as _;
        use tower::ServiceExt as _;

        let referenced = referenced_assets(&ADMIN_UI, "/admin");
        assert!(
            !referenced.is_empty(),
            "nothing to check — extraction is not matching this markup"
        );

        for url_path in referenced {
            let router = routes().with_state(crate::test_support::state());
            let response = router
                .oneshot(
                    axum::http::Request::builder()
                        .uri(&url_path)
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
}
