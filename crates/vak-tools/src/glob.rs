use std::path::Path;

use async_trait::async_trait;
use globset::{GlobBuilder, GlobSetBuilder};
use serde_json::Value;
use walkdir::WalkDir;

use crate::{Tool, ToolContext, ToolOutput};

const MAX_MATCHES: usize = 500;

pub struct GlobTool;

#[async_trait]
impl Tool for GlobTool {
    fn name(&self) -> &str {
        "glob"
    }

    fn description(&self) -> &str {
        "Find files matching a glob pattern (supports **). Returns matching paths relative to the search directory."
    }

    fn schema(&self) -> Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "pattern": {"type": "string", "description": "Glob pattern, e.g. src/**/*.rs"},
                "path": {"type": "string", "description": "Directory to search (default: cwd)"}
            },
            "required": ["pattern"]
        })
    }

    async fn execute(&self, args: &Value, ctx: &ToolContext) -> ToolOutput {
        let Some(pattern) = args.get("pattern").and_then(|p| p.as_str()) else {
            return ToolOutput::error("missing required parameter: pattern");
        };
        let base = match args.get("path").and_then(|p| p.as_str()) {
            Some(p) => ctx.resolve(Path::new(p)),
            None => ctx.cwd.clone(),
        };
        if !base.is_dir() {
            return ToolOutput::error(format!("not a directory: {}", base.display()));
        }

        let glob = match GlobBuilder::new(pattern).literal_separator(true).build() {
            Ok(g) => g,
            Err(e) => return ToolOutput::error(format!("invalid pattern: {e}")),
        };
        let gs = match GlobSetBuilder::new().add(glob).build() {
            Ok(gs) => gs,
            Err(e) => return ToolOutput::error(format!("invalid pattern: {e}")),
        };

        let mut matches = Vec::new();
        for entry in WalkDir::new(&base)
            .follow_links(false)
            .into_iter()
            .filter_entry(|e| !is_ignored(e.path()))
            .flatten()
        {
            if !entry.file_type().is_file() {
                continue;
            }
            let rel = entry
                .path()
                .strip_prefix(&base)
                .map(|p| p.to_path_buf())
                .unwrap_or_else(|_| entry.path().to_path_buf());
            if gs.is_match(&rel) || gs.is_match(entry.path()) {
                matches.push(rel.display().to_string());
                if matches.len() >= MAX_MATCHES {
                    break;
                }
            }
        }

        if matches.is_empty() {
            return ToolOutput::ok("no files matched");
        }
        matches.sort();
        ToolOutput::ok(ctx.truncate_output(matches.join("\n")))
    }
}

pub fn is_ignored(path: &Path) -> bool {
    path.file_name()
        .and_then(|f| f.to_str())
        .map(|f| f == ".git" || f == "node_modules" || f == "target")
        .unwrap_or(false)
}
