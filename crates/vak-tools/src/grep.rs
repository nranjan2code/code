use std::path::Path;

use async_trait::async_trait;
use globset::{Glob, GlobSetBuilder};
use regex::Regex;
use serde_json::Value;
use walkdir::WalkDir;

use crate::{Tool, ToolContext, ToolOutput};

const MAX_MATCH_LINES: usize = 300;

pub struct GrepTool;

#[async_trait]
impl Tool for GrepTool {
    fn name(&self) -> &str {
        "grep"
    }

    fn description(&self) -> &str {
        "Search file contents with a regular expression. Returns file:line:text matches. Use include to filter by glob (e.g. \"*.rs\")."
    }

    fn schema(&self) -> Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "pattern": {"type": "string", "description": "Regular expression to search for"},
                "path": {"type": "string", "description": "File or directory to search (default: cwd)"},
                "include": {"type": "string", "description": "Glob filter for filenames, e.g. *.ts"}
            },
            "required": ["pattern"]
        })
    }

    async fn execute(&self, args: &Value, ctx: &ToolContext) -> ToolOutput {
        let Some(pattern) = args.get("pattern").and_then(|p| p.as_str()) else {
            return ToolOutput::error("missing required parameter: pattern");
        };
        let re = match Regex::new(pattern) {
            Ok(r) => r,
            Err(e) => return ToolOutput::error(format!("invalid regex: {e}")),
        };
        let base = match args.get("path").and_then(|p| p.as_str()) {
            Some(p) => ctx.resolve(Path::new(p)),
            None => ctx.cwd.clone(),
        };
        let include = args.get("include").and_then(|i| i.as_str());
        let include_set = match include {
            Some(glob_str) => {
                let g = match Glob::new(glob_str) {
                    Ok(g) => g,
                    Err(e) => return ToolOutput::error(format!("invalid include: {e}")),
                };
                match GlobSetBuilder::new().add(g).build() {
                    Ok(gs) => Some(gs),
                    Err(e) => return ToolOutput::error(format!("invalid include: {e}")),
                }
            }
            None => None,
        };

        if base.is_file() {
            return search_file(&base, &base, &re, None, ctx);
        }
        if !base.is_dir() {
            return ToolOutput::error(format!("path not found: {}", base.display()));
        }

        let mut out = String::new();
        let mut count = 0usize;
        'outer: for entry in WalkDir::new(&base)
            .follow_links(false)
            .into_iter()
            .filter_entry(|e| !crate::glob::is_ignored(e.path()))
            .flatten()
        {
            if !entry.file_type().is_file() {
                continue;
            }
            if let Some(gs) = &include_set {
                let name_ok = entry
                    .file_name()
                    .to_str()
                    .map(|n| gs.is_match(n))
                    .unwrap_or(false);
                if !name_ok {
                    continue;
                }
            }
            let path = entry.path();
            let Ok(bytes) = std::fs::read(path) else {
                continue;
            };
            if bytes.iter().take(8192).any(|&b| b == 0) {
                continue;
            }
            let text = String::from_utf8_lossy(&bytes);
            for (i, line) in text.lines().enumerate() {
                if re.is_match(line) {
                    out.push_str(&format!(
                        "{}:{}:{line}\n",
                        path.strip_prefix(&ctx.cwd).unwrap_or(path).display(),
                        i + 1
                    ));
                    count += 1;
                    if count >= MAX_MATCH_LINES {
                        out.push_str("\n[match limit reached]");
                        break 'outer;
                    }
                }
            }
        }

        if count == 0 {
            return ToolOutput::ok("no matches");
        }
        ToolOutput::ok(ctx.truncate_output(out))
    }
}

fn search_file(
    path: &Path,
    root: &Path,
    re: &Regex,
    _gs: Option<()>,
    ctx: &ToolContext,
) -> ToolOutput {
    let Ok(bytes) = std::fs::read(path) else {
        return ToolOutput::error(format!("cannot read {}", path.display()));
    };
    let text = String::from_utf8_lossy(&bytes);
    let mut out = String::new();
    for (i, line) in text.lines().enumerate() {
        if re.is_match(line) {
            out.push_str(&format!(
                "{}:{}:{line}\n",
                path.strip_prefix(root).unwrap_or(path).display(),
                i + 1
            ));
        }
    }
    if out.is_empty() {
        ToolOutput::ok("no matches")
    } else {
        ToolOutput::ok(ctx.truncate_output(out))
    }
}
