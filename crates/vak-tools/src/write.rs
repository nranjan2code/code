use crate::Fault;
use async_trait::async_trait;
use serde_json::Value;

use crate::{Tool, ToolContext, ToolOutput};

pub struct WriteTool;

#[async_trait]
impl Tool for WriteTool {
    fn permission(&self) -> vak_permission::Declared {
        vak_permission::Declared::new(
            vak_permission::Reach::WritePath,
            vak_permission::Subject::Path {
                argument: "path",
                required: false,
            },
        )
    }

    fn name(&self) -> &str {
        "write"
    }

    fn file_access(&self, args: &Value) -> Option<(crate::FileAccess, String)> {
        args.get("path")
            .and_then(Value::as_str)
            .map(|path| (crate::FileAccess::Write, path.to_string()))
    }

    fn serves(&self) -> Vec<&str> {
        vec!["documents"]
    }

    fn always_loaded(&self) -> bool {
        true
    }

    /// Every file written is claimed; one written with a `title` is
    /// declared a deliverable.
    fn artifact(&self, args: &Value) -> Option<crate::ArtifactClaim> {
        let text = |key: &str| {
            args.get(key)
                .and_then(Value::as_str)
                .map(str::trim)
                .filter(|text| !text.is_empty())
                .map(str::to_string)
        };
        let title = text("title");
        Some(crate::ArtifactClaim {
            path: text("path")?,
            declared: title.is_some(),
            title,
            summary: text("summary"),
            drafted: false,
            based_on: None,
            change: text("change"),
            read_first: Vec::new(),
        })
    }

    fn description(&self) -> &str {
        "Write a text file the user requested in the workspace, creating parent directories as needed. Do not create a file just to duplicate an answer already delivered in cards or prose. Overwrites the file if it exists. Not for Word, Excel, PowerPoint or PDF files: those are made with office_apply. An HTML page is shown in Vakyartha's look: take colours, fonts and radius from its CSS variables with a fallback for outside it (var(--vak-bg, white), var(--vak-fg, black), var(--vak-surface, white), var(--vak-accent, blue), var(--vak-rule, silver), var(--vak-muted, gray), var(--vak-series-1, blue) to --vak-series-4, var(--vak-font, sans-serif)); plain elements and the classes vak-card, vak-grid, vak-muted and vak-primary are styled already."
    }

    /// A path sent as `file_path` or `file` is the path.
    fn fill_defaults(&self, args: &mut Value) {
        crate::arguments::alias(args, "path", &["file_path", "file"]);
    }

    fn schema(&self) -> Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "path": {"type": "string", "description": "File path (relative to cwd or absolute)"},
                "content": {"type": "string", "description": "Full file content to write"},
                "title": {"type": "string", "description": "A name for the file when it is a deliverable the user asked for; it then appears in their Library. Leave out for supporting or scratch files."},
                "summary": {"type": "string", "description": "One sentence on what the deliverable is"},
                "change": {"type": "string", "description": "A few words naming this version, shown as its title in the file's history (for example \"Shorter version\" or \"Added the tram 28 paragraph\"). Leave out when there is nothing to say beyond the file's name."}
            },
            "required": ["path", "content"]
        })
    }

    fn refusal(&self, args: &Value) -> Option<String> {
        let path = args.get("path").and_then(Value::as_str)?;
        crate::office_apply::text_tool_refusal(std::path::Path::new(path), "write")
    }

    fn claims(&self, args: &Value) -> crate::ResourceClaims {
        crate::ResourceClaims {
            exclusive: false,
            read_only: false,
            paths: args
                .get("path")
                .and_then(|p| p.as_str())
                .map(|p| vec![p.to_string()])
                .unwrap_or_default(),
        }
    }

    async fn execute(&self, args: &Value, ctx: &ToolContext) -> ToolOutput {
        let Some(path_str) = args.get("path").and_then(|p| p.as_str()) else {
            return ToolOutput::error(Fault::Shape, "missing required parameter: path");
        };
        let Some(content) = args.get("content").and_then(|c| c.as_str()) else {
            return ToolOutput::error(Fault::Shape, "missing required parameter: content");
        };
        let path = ctx.resolve(std::path::Path::new(path_str));
        if let Some(refusal) = protected_control_path(&path, ctx) {
            return ToolOutput::error(Fault::Refused, refusal);
        }
        if let Some(refusal) = crate::office_apply::text_tool_refusal(&path, "write") {
            return ToolOutput::error(Fault::Refused, refusal);
        }

        if let Some(parent) = path.parent()
            && let Err(e) = tokio::fs::create_dir_all(parent).await
        {
            return ToolOutput::error(
                Fault::Tool,
                format!(
                    "cannot create directory {}: {e}{}",
                    parent.display(),
                    ctx.outside_note(&path)
                ),
            );
        }
        match write_atomic(&path, content.as_bytes()).await {
            Ok(()) => {
                crate::artifact::emit_file(ctx.sandbox_sink.as_ref(), &path, &ctx.cwd);
                ToolOutput::ok(format!(
                    "wrote {} bytes to {}",
                    content.len(),
                    path.display()
                ))
            }
            Err(e) => ToolOutput::error(
                Fault::Tool,
                format!(
                    "cannot write {}: {e}{}",
                    path.display(),
                    ctx.outside_note(&path)
                ),
            ),
        }
    }
}

pub(crate) fn protected_control_path(path: &std::path::Path, ctx: &ToolContext) -> Option<String> {
    let runtime_call = ctx
        .sandbox_sink
        .as_ref()
        .is_some_and(|sink| sink.is_runtime_call());
    if !runtime_call {
        return None;
    }
    let root = match ctx.cwd.canonicalize() {
        Ok(root) => root,
        Err(_) => return Some("sandbox cannot resolve the workspace".into()),
    };
    if !vak_permission::path_in_workspace(path, &root) {
        return Some("sandbox denied access outside the workspace".into());
    }
    let resolved = crate::context::resolved(path);
    if !resolved.starts_with(&root) {
        return Some("sandbox denied access outside the workspace".into());
    }
    let relative = resolved.strip_prefix(&root).ok()?;
    let scope = vak_config::scope::WorkspaceScope::relative();
    let text = relative.to_string_lossy().replace('\\', "/");
    if text == ".env"
        || text.starts_with(".env.")
        || relative == scope.config_file()
        || relative.starts_with(scope.project_dir())
    {
        Some("sandbox denied writes to workspace control files".into())
    } else {
        None
    }
}

#[cfg(test)]
mod control_path_tests {
    use super::*;

    #[tokio::test]
    async fn runtime_write_cannot_change_permission_control_files() {
        let workspace = tempfile::tempdir().unwrap();
        let control = workspace.path().join(".vak/permissions.local.toml");
        std::fs::create_dir_all(control.parent().unwrap()).unwrap();
        std::fs::write(&control, "allow = []").unwrap();
        let (sink, _events) = crate::SandboxEventSink::new_with_id("runtime-write".into());
        let ctx = ToolContext::new(workspace.path().to_path_buf())
            .with_sandbox_sink(sink.with_owner_session("session"));

        let output = WriteTool
            .execute(
                &serde_json::json!({"path": ".vak/permissions.local.toml", "content": "allow = [\"+Bash(*)\"]"}),
                &ctx,
            )
            .await;

        assert!(output.is_error());
        assert_eq!(std::fs::read_to_string(control).unwrap(), "allow = []");
    }
}

pub(crate) async fn write_atomic(path: &std::path::Path, bytes: &[u8]) -> std::io::Result<()> {
    let parent = path.parent().unwrap_or_else(|| std::path::Path::new("."));
    // The temporary file's own name is not the caller's: an error names
    // only what went wrong, and the caller names the path it asked for.
    let bare = |error: std::io::Error| {
        error
            .raw_os_error()
            .map_or_else(|| error.kind().into(), std::io::Error::from_raw_os_error)
    };
    let mut file = tempfile::NamedTempFile::new_in(parent).map_err(bare)?;
    use std::io::Write as _;
    file.write_all(bytes).map_err(bare)?;
    file.persist(path).map(|_| ()).map_err(|error| error.error)
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

    use super::*;
    use crate::{SandboxEvent, SandboxEventSink};

    /// Found live 2026-10-13: a refused write said `at path "/.tmpQPMP2I"`,
    /// a name the caller never wrote.
    #[tokio::test]
    async fn a_failed_write_never_names_its_temporary_file() {
        let workspace = tempfile::tempdir().unwrap();
        let error = write_atomic(&workspace.path().join("missing/notes.md"), b"x")
            .await
            .unwrap_err();
        assert_eq!(error.kind(), std::io::ErrorKind::NotFound);
        assert!(!error.to_string().contains(".tmp"), "{error}");
    }

    #[tokio::test]
    async fn write_reports_any_file_as_an_artifact() {
        let workspace = tempfile::tempdir().unwrap();
        let (sink, mut events) = SandboxEventSink::new_with_id("write-artifact".into());
        let ctx = ToolContext::new(workspace.path().to_path_buf()).with_sandbox_sink(sink);

        let output = WriteTool
            .execute(
                &serde_json::json!({
                    "path": "reports/market-dashboard.html",
                    "content": "<!doctype html><title>Market week</title>"
                }),
                &ctx,
            )
            .await;

        assert!(!output.is_error(), "{}", output.content);
        let event = events.try_recv().unwrap();
        assert!(matches!(
            event,
            SandboxEvent::ArtifactGenerated { path, mime_type, .. }
                if path == "reports/market-dashboard.html" && mime_type == "text/html"
        ));
    }

    #[tokio::test]
    async fn write_replaces_a_symlink_without_following_it() {
        let workspace = tempfile::tempdir().unwrap();
        let outside = tempfile::tempdir().unwrap();
        let victim = outside.path().join("victim.txt");
        std::fs::write(&victim, "keep me").unwrap();
        #[cfg(unix)]
        std::os::unix::fs::symlink(&victim, workspace.path().join("target.txt")).unwrap();
        #[cfg(windows)]
        std::os::windows::fs::symlink_file(&victim, workspace.path().join("target.txt")).unwrap();
        let ctx = ToolContext::new(workspace.path().to_path_buf());

        let output = WriteTool
            .execute(
                &serde_json::json!({"path": "target.txt", "content": "replacement"}),
                &ctx,
            )
            .await;

        assert!(!output.is_error(), "{}", output.content);
        assert_eq!(std::fs::read_to_string(&victim).unwrap(), "keep me");
        assert!(
            !std::fs::symlink_metadata(workspace.path().join("target.txt"))
                .unwrap()
                .file_type()
                .is_symlink()
        );
        assert_eq!(
            std::fs::read_to_string(workspace.path().join("target.txt")).unwrap(),
            "replacement"
        );
    }
}
