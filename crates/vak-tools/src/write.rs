use async_trait::async_trait;
use serde_json::Value;

use crate::{Tool, ToolContext, ToolOutput};

pub struct WriteTool;

#[async_trait]
impl Tool for WriteTool {
    fn name(&self) -> &str {
        "write"
    }

    fn serves(&self) -> &'static [&'static str] {
        &["documents"]
    }

    fn description(&self) -> &str {
        "Write text content to a file, creating parent directories as needed. Overwrites the file if it exists. Not for Word, Excel, PowerPoint or PDF files: those are made with office_apply."
    }

    fn schema(&self) -> Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "path": {"type": "string", "description": "File path (relative to cwd or absolute)"},
                "content": {"type": "string", "description": "Full file content to write"}
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
        if let Some(refusal) = crate::office_apply::text_tool_refusal(&path, "write") {
            return ToolOutput::error(refusal);
        }

        if let Some(parent) = path.parent()
            && let Err(e) = tokio::fs::create_dir_all(parent).await
        {
            return ToolOutput::error(format!("cannot create directory {}: {e}", parent.display()));
        }
        // Temp file + rename: a crash mid-write cannot truncate the target.
        let tmp = path.with_extension(format!(
            "{}vak-tmp",
            path.extension()
                .map(|e| format!("{}.", e.to_string_lossy()))
                .unwrap_or_default()
        ));
        let write_res = tokio::fs::write(&tmp, content).await;
        let rename_res = match write_res {
            Ok(()) => {
                let r = std::fs::rename(&tmp, &path);
                if r.is_err() {
                    let _ = std::fs::remove_file(&tmp);
                }
                r
            }
            Err(e) => Err(e),
        };
        match rename_res {
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
}
