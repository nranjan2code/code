#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::path::Path;

use tempfile::tempdir;

use vak_tools::{
    Tool, ToolContext, ToolOutput, bash::BashTool, edit::EditTool, glob::GlobTool, grep::GrepTool,
    read::ReadTool, write::WriteTool,
};

async fn run(tool: &dyn Tool, dir: &Path, args: serde_json::Value) -> ToolOutput {
    let ctx = ToolContext {
        cwd: dir.to_path_buf(),
        cancel: tokio_util::sync::CancellationToken::new(),
        sandbox: None,
        sandbox_sink: None,
        agent_id: None,
        trace: None,
        new_documents: Vec::new(),
        executions: None,
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

/// `.vak/agents/<id>/workspace` is another Agent's isolated project
/// workspace (see `vak_config::paths::agent_workspace`), nested inside the
/// shared base workspace on disk. Glob/grep must never wander into it — the
/// whole point of an isolated workspace is defeated if a different Agent's
/// tool calls can read its files.
#[tokio::test]
async fn glob_does_not_walk_into_another_agents_isolated_workspace() {
    let dir = tempdir().unwrap();
    for p in ["notes.md", ".vak/agents/other-agent/workspace/secret.md"] {
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
        serde_json::json!({"pattern": "**/*.md"}),
    )
    .await;
    assert!(!out.is_error);
    assert!(out.content.contains("notes.md"));
    assert!(!out.content.contains("secret.md"));
}

#[tokio::test]
async fn grep_does_not_walk_into_another_agents_isolated_workspace() {
    let dir = tempdir().unwrap();
    run(
        &WriteTool,
        dir.path(),
        serde_json::json!({"path": "notes.md", "content": "the answer is 42\n"}),
    )
    .await;
    run(
        &WriteTool,
        dir.path(),
        serde_json::json!({
            "path": ".vak/agents/other-agent/workspace/secret.md",
            "content": "the answer is 42\n"
        }),
    )
    .await;
    let out = run(&GrepTool, dir.path(), serde_json::json!({"pattern": "42"})).await;
    assert!(!out.is_error);
    assert!(out.content.contains("notes.md"));
    assert!(!out.content.contains("secret.md"));
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
async fn a_tool_returns_its_whole_output() {
    let dir = tempdir().unwrap();
    let long: String = (0..40_000).map(|i| format!("L{i:05}\n")).collect();
    std::fs::write(dir.path().join("long.txt"), &long).unwrap();
    let out = run(
        &BashTool,
        dir.path(),
        serde_json::json!({"command": "cat long.txt"}),
    )
    .await;
    assert!(!out.is_error, "{}", out.content);
    assert!(out.content.contains("L00000") && out.content.contains("L39999"));
    assert!(out.content.chars().count() > long.chars().count());
}

#[tokio::test]
async fn edit_rejects_non_utf8_instead_of_corrupting() {
    let dir = tempdir().unwrap();
    let bytes: &[u8] = &[0x68, 0x69, 0xFF, 0xFE, 0x00, 0x01];
    std::fs::write(dir.path().join("blob.bin"), bytes).unwrap();
    let out = run(
        &EditTool,
        dir.path(),
        serde_json::json!({"path": "blob.bin", "edits": [{"old_text": "hi", "new_text": "ho"}]}),
    )
    .await;
    assert!(out.is_error, "non-UTF-8 edit must be refused");
    assert_eq!(
        std::fs::read(dir.path().join("blob.bin")).unwrap(),
        bytes,
        "file must be untouched"
    );
}
