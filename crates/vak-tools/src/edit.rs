use async_trait::async_trait;
use serde_json::Value;
use similar::TextDiff;

use crate::{Tool, ToolContext, ToolOutput};

pub struct EditTool;

struct PreparedEdit {
    old: String,
    new: String,
}

#[async_trait]
impl Tool for EditTool {
    fn name(&self) -> &str {
        "edit"
    }

    fn serves(&self) -> &'static [&'static str] {
        &["documents"]
    }

    fn description(&self) -> &str {
        "Apply exact string replacements to a text file. All edits are atomic: every old_text must match exactly once or the whole operation fails without changes. Word, Excel, PowerPoint and PDF files are not text: change them with office_apply."
    }

    fn schema(&self) -> Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "path": {"type": "string", "description": "File path (relative to cwd or absolute)"},
                "edits": {
                    "type": "array",
                    "minItems": 1,
                    "items": {
                        "type": "object",
                        "properties": {
                            "old_text": {"type": "string", "description": "Exact text to replace; must be unique in the file"},
                            "new_text": {"type": "string", "description": "Replacement text"}
                        },
                        "required": ["old_text", "new_text"]
                    }
                }
            },
            "required": ["path", "edits"]
        })
    }

    fn refusal(&self, args: &Value) -> Option<String> {
        let path = args.get("path").and_then(Value::as_str)?;
        crate::office_apply::text_tool_refusal(std::path::Path::new(path), "edit")
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
        let Some(edits_val) = args.get("edits").and_then(|e| e.as_array()) else {
            return ToolOutput::error("missing required parameter: edits");
        };
        let path = ctx.resolve(std::path::Path::new(path_str));
        if let Some(refusal) = crate::write::protected_control_path(&path, ctx) {
            return ToolOutput::error(refusal);
        }
        if let Some(refusal) = crate::office_apply::text_tool_refusal(&path, "edit") {
            return ToolOutput::error(refusal);
        }

        let bytes = match tokio::fs::read(&path).await {
            Ok(b) => b,
            Err(e) => return ToolOutput::error(format!("cannot read {}: {e}", path.display())),
        };

        // Editing is a text operation on the exact bytes on disk; silently
        // replacing invalid sequences would corrupt binary or mixed-encoding
        // files on write-back.
        if std::str::from_utf8(&bytes).is_err() {
            return ToolOutput::error(format!(
                "{} is not valid UTF-8 (binary or non-UTF-8 encoding); edit refused",
                path.display()
            ));
        }

        let bom = bytes.starts_with(&[0xEF, 0xBB, 0xBF]);
        let raw = if bom {
            String::from_utf8_lossy(&bytes[3..]).into_owned()
        } else {
            String::from_utf8(bytes).unwrap_or_default()
        };
        let crlf = raw.contains("\r\n");

        let mut prepared: Vec<PreparedEdit> = Vec::with_capacity(edits_val.len());
        for (i, e) in edits_val.iter().enumerate() {
            let Some(old) = e.get("old_text").and_then(|o| o.as_str()) else {
                return ToolOutput::error(format!("edit {i}: missing old_text"));
            };
            let Some(new) = e.get("new_text").and_then(|n| n.as_str()) else {
                return ToolOutput::error(format!("edit {i}: missing new_text"));
            };
            let (old, new) = if crlf && !old.contains("\r\n") {
                (old.replace('\n', "\r\n"), new.replace('\n', "\r\n"))
            } else {
                (old.to_string(), new.to_string())
            };
            prepared.push(PreparedEdit { old, new });
        }

        let mut buf = raw.clone();
        for (i, e) in prepared.iter().enumerate() {
            let count = buf.matches(&e.old).count();
            if count == 0 {
                return ToolOutput::error(format!(
                    "edit {i}: old_text not found in {} (no changes applied)",
                    path.display()
                ));
            }
            if count > 1 {
                return ToolOutput::error(format!(
                    "edit {i}: old_text matches {count} locations in {}; make it unique (no changes applied)",
                    path.display()
                ));
            }
            buf = buf.replacen(&e.old, &e.new, 1);
        }

        let final_bytes = if bom {
            let mut b = vec![0xEF, 0xBB, 0xBF];
            b.extend_from_slice(buf.as_bytes());
            b
        } else {
            buf.as_bytes().to_vec()
        };
        if let Err(e) = crate::write::write_atomic(&path, &final_bytes).await {
            return ToolOutput::error(format!("cannot write {}: {e}", path.display()));
        }
        crate::artifact::emit_file(ctx.sandbox_sink.as_ref(), &path, &ctx.cwd);

        let diff = TextDiff::from_lines(&raw, &buf);
        let mut out = format!("edited {}", path.display());
        let mut changed = 0usize;
        let mut diff_text = String::new();
        for hunk in diff.unified_diff().iter_hunks() {
            changed += 1;
            diff_text.push_str(&format!("{hunk}\n"));
        }
        if changed > 0 {
            out.push_str(&format!(" ({changed} hunks)\n\n{diff_text}"));
        } else {
            out.push_str(" (no changes)");
        }
        ToolOutput::ok(out)
    }
}
