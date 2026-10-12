//! A web page `webfetch` or `browse` returns is read as text only in the
//! network-denied worker (invariant 14), and says what it left out.

#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::path::Path;
use vak_tools::webfetch::{PageFormat, page};

fn worker() -> &'static Path {
    Path::new(env!("CARGO_BIN_EXE_vak-tool-worker"))
}

const PAGE: &str = r#"<!DOCTYPE html><html><head><title>Lighthouse</title>
<script>document.write("<p>injected</p>")</script></head><body>
<nav><a href="/">Home</a></nav>
<main><h1>Lighthouse</h1><p>A tower with a <a href="/wiki/Lamp">lamp</a>.</p>
<table><tr><th>Name</th><th>Country</th></tr><tr><td>Fastnet</td><td>Ireland</td></tr></table>
</main></body></html>"#;

#[tokio::test]
async fn an_html_page_is_read_as_text_in_the_worker() {
    let text = page(
        worker(),
        PAGE,
        "https://en.example.org/wiki/Lighthouse",
        PageFormat::Text,
    )
    .await
    .unwrap();
    assert!(
        text.starts_with("[page] \"Lighthouse\", the readable text of its main content"),
        "{text}"
    );
    assert!(
        text.ends_with("# Lighthouse\n\nA tower with a lamp.\n\nName | Country\nFastnet | Ireland")
    );
    assert!(!text.contains("injected"), "{text}");

    let links = page(
        worker(),
        PAGE,
        "https://en.example.org/wiki/Lighthouse",
        PageFormat::Links,
    )
    .await
    .unwrap();
    assert!(
        links.ends_with("- lamp: https://en.example.org/wiki/Lamp"),
        "{links}"
    );

    let source = page(worker(), PAGE, "https://x.example/", PageFormat::Source)
        .await
        .unwrap();
    assert_eq!(source, PAGE);
}

#[tokio::test]
async fn without_a_worker_a_page_is_refused_not_passed_through() {
    let error = page(
        Path::new("/nonexistent/vak-tool-worker"),
        PAGE,
        "https://x.example/",
        PageFormat::Text,
    )
    .await
    .unwrap_err();
    assert!(error.contains("format \"source\""), "{error}");
}
