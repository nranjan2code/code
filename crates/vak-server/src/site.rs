//! The public website, served at `/` (docs/design/48-web-client.md §4.6).
//!
//! Static pages built from one source by `site/build.py` and embedded
//! here, the same way `/admin` and `/app` embed their bundles. It replaced a
//! single hand-written `assets/landing.html` when the front door grew past
//! one page: a shared rail, footer and design system copied four times drift
//! the moment anyone edits three of them.
//!
//! Two properties this module has to preserve.
//!
//! **It renders before anything else is up.** Each page carries its own CSS
//! and its own script inline; the only sub-resource is the shared,
//! content-hashed `motion.js`, which is deferred and which the site works
//! entirely without. A front door that needs a second round trip to show the
//! product is not a front door.
//!
//! **It is auth-exempt, so it must not disclose anything.** Every route here
//! answers an unauthenticated stranger. The pages show the product and, via
//! `/version`, this build's version and commit — which that endpoint already
//! publishes. They must never carry bind address, permission mode, workspace
//! names, or session counts; `/health` reports several of those and is
//! deliberately not what these pages read.

use axum::routing::get;
use include_dir::{Dir, include_dir};

use crate::embedded_ui::{register_files, serve_file};

static SITE: Dir<'_> = include_dir!("$CARGO_MANIFEST_DIR/site/dist");

/// The routes the site owns, and the file each is served from.
///
/// A table rather than a filesystem walk on purpose: these are the paths
/// this server promises, and a page appearing under `site/dist` should not
/// silently become a public URL without someone writing it down here. The
/// `auth_exempt_path` list in `lib.rs` is checked against this, so the two
/// cannot drift apart.
pub(crate) const ROUTES: &[(&str, &str)] = &[
    ("/", "index.html"),
    ("/outcomes", "outcomes/index.html"),
    ("/vak", "vak/index.html"),
    ("/surfaces", "surfaces/index.html"),
    ("/tour", "tour/index.html"),
    ("/architecture", "architecture/index.html"),
    ("/doctor", "doctor/index.html"),
    ("/security", "security/index.html"),
    ("/install", "install/index.html"),
    ("/wallpapers", "wallpapers/index.html"),
    ("/terms", "terms/index.html"),
    ("/privacy", "privacy/index.html"),
];

fn page(file: &'static str) -> axum::response::Response {
    serve_file(&SITE, file)
}

pub(crate) fn routes() -> axum::Router<crate::AppState> {
    let mut router = axum::Router::new();
    for (uri, file) in ROUTES {
        router = router.route(uri, get(move || async move { page(file) }));
        // `/surfaces/` and `/surfaces` are the same page. A trailing slash
        // is the single most common way a hand-typed URL misses.
        if *uri != "/" {
            router = router.route(&format!("{uri}/"), get(move || async move { page(file) }));
        }
    }
    // The shared vendored script, under its content-hashed name.
    if let Some(assets) = SITE.get_dir("site") {
        router = register_files(router, assets, &SITE, "");
    }
    router
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;
    use http_body_util::BodyExt as _;
    use tower::ServiceExt as _;

    async fn get_path(path: &str) -> (axum::http::StatusCode, String) {
        let router = routes().with_state(crate::test_support::state());
        let response = router
            .oneshot(
                axum::http::Request::builder()
                    .uri(path)
                    .body(axum::body::Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        let status = response.status();
        let body = response.into_body().collect().await.unwrap().to_bytes();
        (status, String::from_utf8_lossy(&body).into_owned())
    }

    #[tokio::test]
    async fn every_declared_route_serves_its_page() {
        for (uri, _file) in ROUTES {
            let (status, body) = get_path(uri).await;
            assert_eq!(status, axum::http::StatusCode::OK, "{uri}");
            assert!(
                body.contains("<title>"),
                "{uri} served something that is not a page"
            );
            assert!(
                body.contains("vak"),
                "{uri} served a page that does not mention the product"
            );
        }
    }

    /// The failure `/admin` and `/app` each shipped: the page loads, and
    /// every asset it references 404s from the router.
    #[tokio::test]
    async fn every_script_the_pages_reference_is_routable() {
        let (_, home) = get_path("/").await;
        let mut referenced: Vec<String> = home
            .split(['"', '\''])
            .filter(|s| s.starts_with("/site/"))
            .map(String::from)
            .collect();
        referenced.sort();
        referenced.dedup();
        assert!(
            !referenced.is_empty(),
            "the home page references no /site/ asset — the layout no longer loads motion, \
             or this extraction no longer matches the markup"
        );
        for path in referenced {
            let (status, _) = get_path(&path).await;
            assert_eq!(
                status,
                axum::http::StatusCode::OK,
                "{path} is referenced but not routable"
            );
        }
    }

    #[tokio::test]
    async fn all_wallpaper_downloads_serve_jpegs() {
        let (_, page) = get_path("/wallpapers").await;
        let downloads: Vec<_> = page
            .split('"')
            .filter(|path| path.starts_with("/site/wallpapers/") && path.ends_with(".jpg"))
            .collect();
        assert_eq!(downloads.len(), 16);
        for path in downloads {
            let response = routes()
                .with_state(crate::test_support::state())
                .oneshot(
                    axum::http::Request::builder()
                        .uri(path)
                        .body(axum::body::Body::empty())
                        .unwrap(),
                )
                .await
                .unwrap();
            assert_eq!(response.status(), axum::http::StatusCode::OK, "{path}");
            assert_eq!(response.headers()["content-type"], "image/jpeg", "{path}");
            let bytes = response.into_body().collect().await.unwrap().to_bytes();
            assert!(bytes.starts_with(&[0xff, 0xd8, 0xff]), "{path}");
        }
    }

    /// These pages answer an unauthenticated stranger, so the only server
    /// data they may pull is what `/version` already publishes.
    ///
    /// Checking the *text* for machine-specific words does not work and was
    /// tried first: `/security` legitimately names `0.0.0.0` while
    /// explaining that the server refuses to bind it. The property that
    /// actually holds is about fetches — these are static files, so nothing
    /// machine-specific can appear unless a script goes and gets it.
    #[tokio::test]
    async fn the_pages_fetch_nothing_but_the_build_stamp() {
        for (uri, _file) in ROUTES {
            let (_, body) = get_path(uri).await;
            let mut fetched: Vec<&str> = Vec::new();
            for tail in body.split("fetch(\"").skip(1) {
                fetched.push(tail.split('"').next().unwrap_or(""));
            }
            assert!(
                !fetched.is_empty(),
                "{uri} fetches nothing at all — the build stamp script is gone, or this \
                 extraction no longer matches how it is written"
            );
            for url in fetched {
                assert_eq!(
                    url, "/version",
                    "{uri} fetches {url}; an auth-exempt page may only read /version, which \
                     publishes the version and commit and nothing else. /health reports \
                     provider, model, sandbox and permission mode, and is not for strangers."
                );
            }
        }
    }

    /// A trailing slash is the commonest way a hand-typed URL misses.
    #[tokio::test]
    async fn sub_pages_answer_with_and_without_a_trailing_slash() {
        for (uri, _file) in ROUTES.iter().filter(|(u, _)| *u != "/") {
            let (status, _) = get_path(&format!("{uri}/")).await;
            assert_eq!(status, axum::http::StatusCode::OK, "{uri}/");
        }
    }
}
