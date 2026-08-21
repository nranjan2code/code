#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::path::Path;
use std::sync::Arc;

use tempfile::tempdir;

use vak_tools::context::OutputLimits;
use vak_tools::{
    Tool, ToolContext, ToolOutput, bash::BashTool, edit::EditTool, glob::GlobTool, grep::GrepTool,
    read::ReadTool, write::WriteTool,
};

async fn run(tool: &dyn Tool, dir: &Path, args: serde_json::Value) -> ToolOutput {
    let ctx = ToolContext {
        cwd: dir.to_path_buf(),
        cancel: tokio_util::sync::CancellationToken::new(),
        limits: OutputLimits {
            max_bytes: 500,
            max_line_chars: 100,
            spill_to_disk: false,
        },
        sandbox: None,
    };
    tool.execute(&args, &ctx).await
}

#[tokio::test]
async fn write_then_read_roundtrip() {
    let dir = tempdir().unwrap();
    run(
        &WriteTool,
        dir.path(),
        serde_json::json!({
            "path": "nested/dir/file.txt",
            "content": "alpha\nbeta\n"
        }),
    )
    .await;

    let out = run(
        &ReadTool,
        dir.path(),
        serde_json::json!({"path": "nested/dir/file.txt"}),
    )
    .await;
    assert!(!out.is_error);
    assert!(out.content.contains("alpha"));
    assert!(out.content.contains('1'));
}

#[tokio::test]
async fn read_reports_missing_file_as_error_value() {
    let dir = tempdir().unwrap();
    let out = run(
        &ReadTool,
        dir.path(),
        serde_json::json!({"path": "nope.txt"}),
    )
    .await;
    assert!(out.is_error);
    assert!(out.content.contains("cannot read"));
}

#[tokio::test]
async fn read_paging_hint() {
    let dir = tempdir().unwrap();
    let content: String = (0..50).map(|i| format!("line{i}\n")).collect();
    run(
        &WriteTool,
        dir.path(),
        serde_json::json!({"path": "big.txt", "content": content}),
    )
    .await;
    let out = run(
        &ReadTool,
        dir.path(),
        serde_json::json!({"path": "big.txt", "limit": 10}),
    )
    .await;
    assert!(!out.is_error);
    assert!(out.content.contains("Use offset=11 to continue."));
}

#[tokio::test]
async fn edit_is_atomic_on_duplicate_match() {
    let dir = tempdir().unwrap();
    run(
        &WriteTool,
        dir.path(),
        serde_json::json!({
            "path": "f.txt",
            "content": "x = 1\ny = 1\nz = 1\n"
        }),
    )
    .await;

    let out = run(&EditTool, dir.path(), serde_json::json!({
        "path": "f.txt",
        "edits": [{"old_text": "y = 1", "new_text": "y = 2"}, {"old_text": "= 1", "new_text": "= 9"}]
    })).await;

    assert!(
        out.is_error,
        "second edit matches twice; whole op must fail"
    );
    assert!(out.content.contains("no changes applied"));
    let after = tokio::fs::read_to_string(dir.path().join("f.txt"))
        .await
        .unwrap();
    assert_eq!(after, "x = 1\ny = 1\nz = 1\n", "file must be untouched");
}

#[tokio::test]
async fn edit_applies_sequential_edits_with_diff() {
    let dir = tempdir().unwrap();
    run(
        &WriteTool,
        dir.path(),
        serde_json::json!({
            "path": "f.txt",
            "content": "fn main() {\n    println!(\"hello\");\n}\n"
        }),
    )
    .await;

    let out = run(
        &EditTool,
        dir.path(),
        serde_json::json!({
            "path": "f.txt",
            "edits": [
                {"old_text": "\"hello\"", "new_text": "\"goodbye\""},
                {"old_text": "fn main()", "new_text": "fn main() -> ()"}
            ]
        }),
    )
    .await;

    assert!(!out.is_error);
    let after = tokio::fs::read_to_string(dir.path().join("f.txt"))
        .await
        .unwrap();
    assert_eq!(after, "fn main() -> () {\n    println!(\"goodbye\");\n}\n");
    assert!(
        out.content.contains("-fn main() {"),
        "diff should show removed line"
    );
}

#[tokio::test]
async fn edit_preserves_crlf_line_endings() {
    let dir = tempdir().unwrap();
    tokio::fs::write(dir.path().join("crlf.txt"), "a = 1\r\nb = 2\r\n")
        .await
        .unwrap();

    let out = run(
        &EditTool,
        dir.path(),
        serde_json::json!({
            "path": "crlf.txt",
            "edits": [{"old_text": "a = 1", "new_text": "a = 10"}]
        }),
    )
    .await;

    assert!(!out.is_error);
    let after = tokio::fs::read_to_string(dir.path().join("crlf.txt"))
        .await
        .unwrap();
    assert_eq!(after, "a = 10\r\nb = 2\r\n");
}

#[tokio::test]
async fn bash_runs_and_captures() {
    let dir = tempdir().unwrap();
    let out = run(
        &BashTool,
        dir.path(),
        serde_json::json!({"command": "echo hi && echo err >&2"}),
    )
    .await;
    assert!(!out.is_error);
    assert!(out.content.contains("[stdout]\nhi"));
    assert!(out.content.contains("[stderr]\nerr"));
}

#[tokio::test]
async fn bash_nonzero_exit_is_error_value() {
    let dir = tempdir().unwrap();
    let out = run(
        &BashTool,
        dir.path(),
        serde_json::json!({"command": "exit 3"}),
    )
    .await;
    assert!(out.is_error);
    assert!(out.content.contains("exit code: 3"));
}

#[tokio::test]
async fn bash_timeout_kills_process() {
    let dir = tempdir().unwrap();
    let start = std::time::Instant::now();
    let out = run(
        &BashTool,
        dir.path(),
        serde_json::json!({
            "command": "sleep 30",
            "timeout_ms": 1500
        }),
    )
    .await;
    assert!(out.is_error);
    assert!(start.elapsed() < std::time::Duration::from_secs(5));
    assert!(out.content.contains("timed out"));
}

#[tokio::test]
async fn glob_finds_nested_files() {
    let dir = tempdir().unwrap();
    for p in ["src/a.rs", "src/deep/b.rs", "docs/c.md"] {
        run(
            &WriteTool,
            dir.path(),
            serde_json::json!({"path": p, "content": "x"}),
        )
        .await;
    }
    let out = run(
        &GlobTool,
        dir.path(),
        serde_json::json!({"pattern": "src/**/*.rs"}),
    )
    .await;
    assert!(!out.is_error);
    assert!(out.content.contains("src/a.rs"));
    assert!(out.content.contains("src/deep/b.rs"));
    assert!(!out.content.contains("docs/c.md"));
}

#[tokio::test]
async fn grep_matches_with_include_filter() {
    let dir = tempdir().unwrap();
    run(
        &WriteTool,
        dir.path(),
        serde_json::json!({"path": "code.rs", "content": "let answer = 42;\n"}),
    )
    .await;
    run(
        &WriteTool,
        dir.path(),
        serde_json::json!({"path": "note.md", "content": "the answer is 42\n"}),
    )
    .await;

    let out = run(
        &GrepTool,
        dir.path(),
        serde_json::json!({
            "pattern": "42",
            "include": "*.rs"
        }),
    )
    .await;
    assert!(!out.is_error);
    assert!(out.content.contains("code.rs:1"));
    assert!(!out.content.contains("note.md"));
}

#[tokio::test]
async fn output_truncation_keeps_head_and_tail() {
    let dir = tempdir().unwrap();
    let long: String = (0..200).map(|i| format!("L{i:04}\n")).collect();
    run(
        &WriteTool,
        dir.path(),
        serde_json::json!({"path": "long.txt", "content": long}),
    )
    .await;
    let out = run(
        &ReadTool,
        dir.path(),
        serde_json::json!({"path": "long.txt"}),
    )
    .await;
    let truncated = Arc::new(ToolContext {
        cwd: dir.path().to_path_buf(),
        cancel: tokio_util::sync::CancellationToken::new(),
        limits: OutputLimits {
            max_bytes: 300,
            max_line_chars: 2000,
            spill_to_disk: false,
        },
        sandbox: None,
    });
    let t = truncated.truncate_output(out.content);
    assert!(t.len() < 600);
    assert!(t.contains("truncated"));
    assert!(t.contains("L0000"), "head preserved");
    assert!(t.contains("L0199"), "tail preserved");
}
