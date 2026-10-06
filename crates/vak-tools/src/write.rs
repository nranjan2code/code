use async_trait::async_trait;
use serde_json::Value;

use crate::{Tool, ToolContext, ToolOutput};

pub struct WriteTool;

#[async_trait]
impl Tool for WriteTool {
    fn name(&self) -> &str {
        "write"
    }

    fn file_access(&self, args: &Value) -> Option<(crate::FileAccess, String)> {
        args.get("path")
            .and_then(Value::as_str)
            .map(|path| (crate::FileAccess::Write, path.to_string()))
    }

    fn serves(&self) -> &'static [&'static str] {
        &["documents"]
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
        })
    }

    fn description(&self) -> &str {
        "Write text content to a file, creating parent directories as needed. Overwrites the file if it exists. Not for Word, Excel, PowerPoint or PDF files: those are made with office_apply."
    }

    fn schema(&self) -> Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "path": {"type": "string", "description": "File path (relative to cwd or absolute)"},
                "content": {"type": "string", "description": "Full file content to write"},
                "title": {"type": "string", "description": "Names a deliverable for the user; omit for helper files"},
                "summary": {"type": "string", "description": "One line on the deliverable"}
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
            return ToolOutput::error("missing required parameter: path");
        };
        let Some(content) = args.get("content").and_then(|c| c.as_str()) else {
            return ToolOutput::error("missing required parameter: content");
        };
        let path = ctx.resolve(std::path::Path::new(path_str));
        if let Some(refusal) = protected_control_path(&path, ctx) {
            return ToolOutput::error(refusal);
        }
        if let Some(refusal) = crate::office_apply::text_tool_refusal(&path, "write") {
            return ToolOutput::error(refusal);
        }

        if let Some(parent) = path.parent()
            && let Err(e) = tokio::fs::create_dir_all(parent).await
        {
            return ToolOutput::error(format!("cannot create directory {}: {e}", parent.display()));
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
            Err(e) => ToolOutput::error(format!("cannot write {}: {e}", path.display())),
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
    let root = ctx.cwd.canonicalize().ok()?;
    let candidate = if path.is_absolute() {
        path.to_path_buf()
    } else {
        root.join(path)
    };
    let resolved = if candidate.exists() {
        candidate.canonicalize().ok()?
    } else {
        let parent = candidate.parent()?;
        let canonical_parent = parent.canonicalize().ok()?;
        canonical_parent.join(candidate.file_name()?)
    };
    if !resolved.starts_with(&root) {
        return Some("sandbox denied access outside the workspace".into());
    }
    let relative = resolved.strip_prefix(&root).ok()?;
    let scope = vak_config::scope::WorkspaceScope::relative();
    let text = relative.to_string_lossy().replace('\\', "/");
    if text == ".env"
        || text.starts_with(".env.")
        || relative == scope.config_file()
        || relative == scope.permissions_local()
        || relative.starts_with(scope.project_dir().join("config"))
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

        assert!(output.is_error);
        assert_eq!(std::fs::read_to_string(control).unwrap(), "allow = []");
    }
}

pub(crate) async fn write_atomic(path: &std::path::Path, bytes: &[u8]) -> std::io::Result<()> {
    let parent = path.parent().unwrap_or_else(|| std::path::Path::new("."));
    let mut file = tempfile::NamedTempFile::new_in(parent)?;
    use std::io::Write as _;
    file.write_all(bytes)?;
    file.persist(path).map(|_| ()).map_err(|error| error.error)
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

    use super::*;
    use crate::{SandboxEvent, SandboxEventSink};

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

        assert!(!output.is_error, "{}", output.content);
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

        assert!(!output.is_error, "{}", output.content);
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
