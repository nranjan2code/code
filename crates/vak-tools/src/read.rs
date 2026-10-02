use async_trait::async_trait;
use serde_json::Value;

use crate::{Tool, ToolContext, ToolOutput};

const DEFAULT_LIMIT: usize = 2000;
const IMAGE_EXTS: [&str; 5] = ["jpg", "jpeg", "png", "gif", "webp"];

pub struct ReadTool;

#[async_trait]
impl Tool for ReadTool {
    fn name(&self) -> &str {
        "read"
    }

    fn serves(&self) -> &'static [&'static str] {
        &["filesystem"]
    }

    fn always_loaded(&self) -> bool {
        true
    }

    fn description(&self) -> &str {
        "Read a text file from disk. Returns numbered lines. Use offset/limit to page through large files. For Word, Excel, PowerPoint, Visio and PDF files use doc_read."
    }

    fn schema(&self) -> Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "path": {"type": "string", "description": "File path (relative to cwd or absolute)"},
                "offset": {"type": "integer", "minimum": 1, "description": "1-based start line"},
                "limit": {"type": "integer", "minimum": 1, "description": "Max lines to return"}
            },
            "required": ["path"]
        })
    }

    fn refusal(&self, args: &Value) -> Option<String> {
        let path = args.get("path").and_then(Value::as_str)?;
        crate::office_apply::text_tool_refusal(std::path::Path::new(path), "read")
    }

    fn claims(&self, _args: &Value) -> crate::ResourceClaims {
        crate::ResourceClaims {
            exclusive: false,
            read_only: true,
            paths: Vec::new(),
        }
    }

    async fn execute(&self, args: &Value, ctx: &ToolContext) -> ToolOutput {
        let Some(path_str) = args.get("path").and_then(|p| p.as_str()) else {
            return ToolOutput::error("missing required parameter: path");
        };
        let path = ctx.resolve(std::path::Path::new(path_str));
        if crate::write::protected_control_path(&path, ctx).is_some() {
            return ToolOutput::error("sandbox denied reads of workspace control files");
        }
        if let Some(refusal) = crate::office_apply::text_tool_refusal(&path, "read") {
            return ToolOutput::error(refusal);
        }

        let ext = path
            .extension()
            .and_then(|e| e.to_str())
            .map(|e| e.to_ascii_lowercase())
            .unwrap_or_default();
        if IMAGE_EXTS.contains(&ext.as_str()) {
            return ToolOutput::ok(format!(
                "[image file: {} — inline image rendering is not enabled in this build]",
                path.display()
            ));
        }

        let bytes = match tokio::fs::read(&path).await {
            Ok(b) => b,
            Err(e) => return ToolOutput::error(format!("cannot read {}: {e}", path.display())),
        };
        if bytes.iter().take(8192).any(|&b| b == 0) {
            return ToolOutput::ok(format!("[binary file: {}]", path.display()));
        }

        let text = String::from_utf8_lossy(&bytes);
        let total_lines = text.lines().count();
        let offset = args
            .get("offset")
            .and_then(|o| o.as_u64())
            .unwrap_or(1)
            .max(1) as usize;
        let limit = args
            .get("limit")
            .and_then(|l| l.as_u64())
            .map(|l| l as usize)
            .unwrap_or(DEFAULT_LIMIT);

        let start_idx = offset - 1;
        if start_idx >= total_lines {
            return ToolOutput::error(format!(
                "offset {offset} is beyond end of file ({total_lines} lines)"
            ));
        }
        let end_idx = (start_idx + limit).min(total_lines);

        let mut out = String::new();
        for (i, line) in text
            .lines()
            .enumerate()
            .skip(start_idx)
            .take(end_idx - start_idx)
        {
            out.push_str(&format!("{:>6}\t{line}\n", i + 1));
        }
        if end_idx < total_lines {
            out.push_str(&format!(
                "\n[Showing lines {}–{} of {total_lines}. Use offset={} to continue.]\n",
                start_idx + 1,
                end_idx,
                end_idx + 1
            ));
        }
        ToolOutput::ok(out)
    }
}
