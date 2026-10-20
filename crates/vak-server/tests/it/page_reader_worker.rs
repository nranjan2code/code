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
        text.ends_with(
            "# Lighthouse\n\nA tower with a [lamp](https://en.example.org/wiki/Lamp).\n\nName | Country\nFastnet | Ireland"
        ),
        "{text}"
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

/// Protection is tested through the production worker protocol, not only a
/// direct Tool fixture carrying telemetry ownership.
#[cfg(target_os = "macos")]
#[tokio::test]
async fn real_workers_cannot_read_or_mutate_workspace_control_state() {
    use std::sync::Arc;
    use vak_tools::sandbox::{SandboxMode, Seatbelt};
    use vak_tools::{Tool, ToolContext};
    vak_config::paths::isolate_home_for_tests();
    let root = tempfile::tempdir().unwrap();
    let control = vak_config::scope::WorkspaceScope::new(root.path());
    std::fs::create_dir_all(control.project_dir()).unwrap();
    std::fs::write(control.permissions_local(), "allow = []").unwrap();
    std::fs::write(root.path().join(".env"), "SYNTHETIC_PRIVATE_MARKER").unwrap();
    let mut ctx = ToolContext::new(root.path().to_path_buf());
    ctx.sandbox = Some(Arc::new(Seatbelt::new(
        SandboxMode::WorkspaceWrite,
        root.path(),
    )));
    let cases: Vec<(Arc<dyn Tool>, serde_json::Value)> = vec![
        (
            Arc::new(vak_tools::read::ReadTool),
            serde_json::json!({"path":".env"}),
        ),
        (
            Arc::new(vak_tools::grep::GrepTool),
            serde_json::json!({"path":".env","pattern":"."}),
        ),
        (
            Arc::new(vak_tools::write::WriteTool),
            serde_json::json!({"path":control.permissions_local(),"content":"allow = ['+Bash(*)']"}),
        ),
        (
            Arc::new(vak_tools::bash::BashTool),
            serde_json::json!({"command":"p=$(printf '.e%s' nv); cat \"$p\""}),
        ),
        (
            Arc::new(vak_tools::bash::BashTool),
            serde_json::json!({"command":"p=$(printf '.va%s' k); printf corrupted > \"$p/permissions.local.toml\""}),
        ),
        (
            Arc::new(vak_tools::bash::BashTool),
            serde_json::json!({"command":"mv .vak moved-controls"}),
        ),
        (
            Arc::new(vak_tools::bash::BashTool),
            serde_json::json!({"command":"ln .env public-copy && cat public-copy"}),
        ),
    ];
    for (tool, args) in cases {
        let name = tool.name().to_string();
        let broker = vak_tools::broker::BrokeredTool::new(tool, worker().to_path_buf(), vec![]);
        let result = broker.execute(&args, &ctx).await;
        assert!(result.is_error(), "{name}: {:?}: {}", args, result.content);
        assert!(!result.content.contains("SYNTHETIC_PRIVATE_MARKER"));
        assert_eq!(
            std::fs::read_to_string(control.permissions_local()).unwrap(),
            "allow = []"
        );
    }
    let broker = vak_tools::broker::BrokeredTool::new(
        Arc::new(vak_tools::write::WriteTool),
        worker().to_path_buf(),
        vec![],
    );
    assert!(
        !broker
            .execute(
                &serde_json::json!({"path":"ordinary.txt","content":"ok"}),
                &ctx
            )
            .await
            .is_error()
    );
}

#[tokio::test]
#[ignore = "requires installed Chromium and a public HTTPS page"]
async fn rendered_browser_works_through_enforced_screened_proxy() {
    let root = tempfile::tempdir().unwrap();
    let ctx = vak_tools::ToolContext::new(root.path().to_path_buf());
    let output = vak_tools::webbrowse::Renderer::new(worker().to_path_buf())
        .render(
            &serde_json::json!({"url":"https://example.com/", "format":"source"}),
            &ctx,
        )
        .await;
    assert!(!output.is_error(), "{}", output.content);
    assert!(
        output.content.contains("Example Domain"),
        "{}",
        output.content
    );
}
