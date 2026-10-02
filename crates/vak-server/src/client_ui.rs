//! The workspace client, served at `/app` (docs/design/48-web-client.md).
//!
//! Same bundle as the Tauri desktop shell ships — one client, two hosts —
//! built a second time with `base: "/app/"` so its asset URLs resolve here
//! (`crates/vak-client-ui`, `npm run build:web` → `dist-web/`).
//!
//! The shell and its hashed assets are auth-exempt because the login form
//! is part of the bundle: a browser has to be able to load the page in
//! order to be asked for a token. Nothing here carries data — every route
//! the client then calls is authenticated.

use axum::routing::get;
use include_dir::{Dir, include_dir};

use crate::embedded_ui::{register_files, serve_file};

static CLIENT_UI: Dir<'_> = include_dir!("$CARGO_MANIFEST_DIR/../vak-client-ui/dist-web");

async fn index() -> axum::response::Response {
    serve_file(&CLIENT_UI, "index.html")
}

pub(crate) fn routes() -> axum::Router<crate::AppState> {
    let router = axum::Router::new()
        .route("/app", get(index))
        .route("/app/", get(index))
        // The client uses hash routing, so there are no deep paths to
        // rewrite — but a bookmark to `/app/anything` should still land on
        // the shell rather than a bare 404 from the router.
        .route("/app/{*rest}", get(index));
    register_files(router, &CLIENT_UI, &CLIENT_UI, "/app")
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;
    use crate::embedded_ui::referenced_assets;

    /// A preview can navigate its own frame to any address; the app page is
    /// what refuses an internet one (docs/design/66 §3.2).
    #[test]
    fn the_app_page_confines_frames_to_the_app_and_loopback() {
        let response = serve_file(&CLIENT_UI, "index.html");
        let policy = response.headers()["content-security-policy"]
            .to_str()
            .unwrap();
        assert!(policy.starts_with("frame-src 'self'"), "{policy}");
        for allowed in policy.split_whitespace().skip(1) {
            assert!(
                matches!(
                    allowed,
                    "'self'" | "blob:" | "data:" | "http://127.0.0.1:*" | "http://localhost:*"
                ),
                "{allowed}"
            );
        }
    }

    /// The `/admin` version of this test exists because the failure it
    /// catches shipped three times. `/app` is served by the same code, so
    /// it gets the same proof rather than the same trust.
    #[test]
    fn every_asset_index_html_references_is_actually_embedded() {
        let referenced = referenced_assets(&CLIENT_UI, "/app");
        assert!(
            !referenced.is_empty(),
            "index.html references no /app/assets/ paths — either the web bundle was \
             built without `base: /app/`, or this extraction no longer matches the markup"
        );
        for url_path in referenced {
            let embedded_path = url_path.trim_start_matches("/app/");
            assert!(
                CLIENT_UI.get_file(embedded_path).is_some(),
                "index.html references {url_path}, but no such file is embedded"
            );
        }
    }

    #[tokio::test]
    async fn every_referenced_asset_is_actually_routable() {
        use http_body_util::BodyExt as _;
        use tower::ServiceExt as _;

        let referenced = referenced_assets(&CLIENT_UI, "/app");
        assert!(!referenced.is_empty(), "nothing to check");

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
                "GET {url_path} did not route to 200"
            );
            let bytes = response.into_body().collect().await.unwrap().to_bytes();
            assert!(
                !bytes.is_empty(),
                "GET {url_path} routed but returned nothing"
            );
        }
    }

    /// A bookmarked deep link must reach the shell, not a 404 — the client
    /// routes on the hash, but nothing stops someone pasting a path.
    #[tokio::test]
    async fn a_deep_path_serves_the_shell() {
        use tower::ServiceExt as _;
        let router = routes().with_state(crate::test_support::state());
        let response = router
            .oneshot(
                axum::http::Request::builder()
                    .uri("/app/w/abc/s/def")
                    .body(axum::body::Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), axum::http::StatusCode::OK);
    }
}
