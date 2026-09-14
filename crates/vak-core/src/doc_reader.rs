//! In-Memory Universal Document Ingestion (`doc_read`).
//!
//! Provides structured, token-bounded document extraction across
//! Markdown, Plaintext, CSV, TSV, and JSON formats without external
//! subprocesses. Supports section outline navigation, table rendering,
//! summary statistics, and strict workspace boundary confinement (Invariant 10).

use std::path::Path;

use async_trait::async_trait;
use serde_json::Value;
use vak_tools::{ResourceClaims, Tool, ToolContext, ToolOutput};

pub struct DocReaderTool;

#[async_trait]
impl Tool for DocReaderTool {
    fn name(&self) -> &str {
        "doc_read"
    }

    fn description(&self) -> &str {
        "Inspect and extract text, sections, tables, or summaries from documents and data files (Markdown, Plaintext, CSV, TSV, JSON). Supports section navigation, outlines, and paginated table views."
    }

    fn schema(&self) -> Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "path": {
                    "type": "string",
                    "description": "File path (relative to workspace root)"
                },
                "section": {
                    "type": "string",
                    "description": "Optional section heading to extract (e.g. '## Methodology' or 'Results')"
                },
                "offset": {
                    "type": "integer",
                    "minimum": 1,
                    "description": "1-based starting line or row number (default 1)"
                },
                "limit": {
                    "type": "integer",
                    "minimum": 1,
                    "maximum": 500,
                    "description": "Maximum number of lines or table rows to extract (default 100)"
                },
                "view": {
                    "type": "string",
                    "enum": ["text", "table", "summary", "outline"],
                    "description": "Extraction view: 'text' (default), 'table' (formatted markdown table), 'summary' (metadata & statistics), or 'outline' (headings and keys)"
                }
            },
            "required": ["path"]
        })
    }

    fn claims(&self, _args: &Value) -> ResourceClaims {
        ResourceClaims {
            exclusive: false,
            read_only: true,
            paths: Vec::new(),
        }
    }

    async fn execute(&self, args: &Value, ctx: &ToolContext) -> ToolOutput {
        let Some(path_str) = args.get("path").and_then(Value::as_str).map(str::trim) else {
            return ToolOutput::error("missing required parameter: path");
        };
        if path_str.is_empty() {
            return ToolOutput::error("'path' must not be empty");
        }

        let p = Path::new(path_str);
        let resolved = if p.is_absolute() {
            p.to_path_buf()
        } else {
            ctx.cwd.join(p)
        };

        // Strict workspace boundary confinement (Invariant 10)
        let canonical_target = match resolved.canonicalize() {
            Ok(c) => c,
            Err(e) => return ToolOutput::error(format!("cannot open file '{}': {e}", p.display())),
        };
        let canonical_cwd = match ctx.cwd.canonicalize() {
            Ok(c) => c,
            Err(e) => return ToolOutput::error(format!("cannot resolve workspace root: {e}")),
        };
        if !canonical_target.starts_with(&canonical_cwd) {
            return ToolOutput::error("access denied: path escapes canonical workspace root");
        }

        let content = match tokio::fs::read_to_string(&canonical_target).await {
            Ok(s) => s,
            Err(e) => return ToolOutput::error(format!("failed to read file: {e}")),
        };

        let offset = args.get("offset").and_then(Value::as_u64).unwrap_or(1).max(1) as usize;
        let limit = args.get("limit").and_then(Value::as_u64).unwrap_or(100).min(500) as usize;
        let section = args.get("section").and_then(Value::as_str).map(str::trim);
        let view = args.get("view").and_then(Value::as_str).unwrap_or("text");

        let ext = canonical_target
            .extension()
            .and_then(|e| e.to_str())
            .unwrap_or("")
            .to_ascii_lowercase();

        match view {
            "summary" => ToolOutput::ok(summarize_document(&canonical_target, &content, &ext)),
            "outline" => ToolOutput::ok(outline_document(&content, &ext)),
            "table" => ToolOutput::ok(render_table_view(&content, &ext, offset, limit)),
            _ => {
                if let Some(target_sec) = section {
                    ToolOutput::ok(extract_section(&content, target_sec, offset, limit))
                } else {
                    ToolOutput::ok(extract_text_lines(&content, offset, limit))
                }
            }
        }
    }
}

fn summarize_document(path: &Path, content: &str, ext: &str) -> String {
    let lines: Vec<&str> = content.lines().collect();
    let word_count: usize = content.split_whitespace().count();
    let byte_size = content.len();
    let line_count = lines.len();

    let mut out = format!(
        "Document: {}\nFormat: {}\nLines: {}\nWords: {}\nBytes: {}\n\n",
        path.file_name().and_then(|n| n.to_str()).unwrap_or("document"),
        if ext.is_empty() { "plaintext" } else { ext },
        line_count,
        word_count,
        byte_size
    );

    let outline = outline_document(content, ext);
    if !outline.trim().is_empty() {
        out.push_str("Outline Structure:\n");
        out.push_str(&outline);
    }
    out
}

fn outline_document(content: &str, ext: &str) -> String {
    match ext {
        "csv" | "tsv" => {
            let lines: Vec<&str> = content.lines().filter(|l| !l.trim().is_empty()).collect();
            if lines.is_empty() {
                return "Empty tabular dataset".into();
            }
            let sep = if ext == "tsv" { '\t' } else { ',' };
            let headers = lines[0].split(sep).map(str::trim).collect::<Vec<_>>().join(" | ");
            format!("Columns ({}): {}\nTotal Rows: {}", lines[0].split(sep).count(), headers, lines.len().saturating_sub(1))
        }
        "json" => {
            if let Ok(val) = serde_json::from_str::<Value>(content) {
                match val {
                    Value::Array(arr) => {
                        let sample_keys = arr.first().and_then(|item| item.as_object()).map(|obj| {
                            obj.keys().cloned().collect::<Vec<_>>().join(", ")
                        }).unwrap_or_else(|| "primitive values".into());
                        format!("JSON Array with {} elements.\nObject Keys: [{}]", arr.len(), sample_keys)
                    }
                    Value::Object(obj) => {
                        let keys = obj.keys().map(|k| format!("- {k}")).collect::<Vec<_>>().join("\n");
                        format!("JSON Object with keys:\n{keys}")
                    }
                    _ => "Primitive JSON scalar".into(),
                }
            } else {
                "Malformed JSON".into()
            }
        }
        _ => {
            // Markdown / text heading extraction
            let mut headings = Vec::new();
            for (idx, line) in content.lines().enumerate() {
                let trimmed = line.trim();
                if trimmed.starts_with('#') {
                    headings.push(format!("L{:03}: {}", idx + 1, trimmed));
                }
            }
            if headings.is_empty() {
                "No explicit Markdown headings found (use 'text' view with offset/limit).".into()
            } else {
                headings.join("\n")
            }
        }
    }
}

fn render_table_view(content: &str, ext: &str, offset: usize, limit: usize) -> String {
    let sep = if ext == "tsv" { '\t' } else { ',' };
    let lines: Vec<&str> = content.lines().filter(|l| !l.trim().is_empty()).collect();
    if lines.is_empty() {
        return "Dataset contains no rows.".into();
    }

    let headers: Vec<&str> = lines[0].split(sep).map(str::trim).collect();
    let header_line = format!("| {} |", headers.join(" | "));
    let separator_line = format!("| {} |", vec!["---"; headers.len()].join(" | "));

    let total_data_rows = lines.len().saturating_sub(1);
    let start_idx = (offset.saturating_sub(1)).min(total_data_rows);
    let end_idx = (start_idx + limit).min(total_data_rows);

    let mut table_rows = Vec::new();
    for line in lines.iter().skip(1 + start_idx).take(limit) {
        let cells: Vec<&str> = line.split(sep).map(str::trim).collect();
        table_rows.push(format!("| {} |", cells.join(" | ")));
    }

    format!(
        "{header_line}\n{separator_line}\n{}\n\n[Showing rows {}..{} of {} total rows]",
        table_rows.join("\n"),
        start_idx + 1,
        end_idx,
        total_data_rows
    )
}

fn extract_section(content: &str, target_section: &str, offset: usize, limit: usize) -> String {
    let clean_target = target_section.trim().trim_start_matches('#').trim().to_ascii_lowercase();
    let lines: Vec<&str> = content.lines().collect();

    let mut in_section = false;
    let mut target_level = 0;
    let mut collected = Vec::new();

    for line in lines {
        let trimmed = line.trim();
        if trimmed.starts_with('#') {
            let hashes = trimmed.chars().take_while(|&c| c == '#').count();
            let heading_text = trimmed.trim_start_matches('#').trim().to_ascii_lowercase();

            if in_section {
                // Stop when encountering a heading of equal or higher level
                if hashes <= target_level {
                    break;
                }
            } else if heading_text.contains(&clean_target) {
                in_section = true;
                target_level = hashes;
                collected.push(line);
                continue;
            }
        }

        if in_section {
            collected.push(line);
        }
    }

    if collected.is_empty() {
        format!("Section '{target_section}' not found in document.")
    } else {
        let start = (offset.saturating_sub(1)).min(collected.len());
        let end = (start + limit).min(collected.len());
        let slice = &collected[start..end];
        format!(
            "{}\n\n[Section lines {}..{} of {}]",
            slice.join("\n"),
            start + 1,
            end,
            collected.len()
        )
    }
}

fn extract_text_lines(content: &str, offset: usize, limit: usize) -> String {
    let lines: Vec<&str> = content.lines().collect();
    let total = lines.len();
    let start = (offset.saturating_sub(1)).min(total);
    let end = (start + limit).min(total);

    let numbered: Vec<String> = lines[start..end]
        .iter()
        .enumerate()
        .map(|(idx, line)| format!("{:4}: {}", start + idx + 1, line))
        .collect();

    format!(
        "{}\n\n[Lines {}..{} of {} total lines]",
        numbered.join("\n"),
        start + 1,
        end,
        total
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_extract_section_markdown() {
        let doc = "# Research Paper\n\n## Abstract\nThis is the abstract.\n\n## Methods\nWe used in-memory tooling.\nStep 1: parse.\nStep 2: verify.\n\n## Results\nAll tests passed.\n";
        let extracted = extract_section(doc, "Methods", 1, 50);
        assert!(extracted.contains("We used in-memory tooling."));
        assert!(extracted.contains("Step 1: parse."));
        assert!(!extracted.contains("All tests passed."));
    }

    #[test]
    fn test_outline_markdown() {
        let doc = "# Main\n## Sec 1\n### Sub 1\n## Sec 2\n";
        let outline = outline_document(doc, "md");
        assert!(outline.contains("L001: # Main"));
        assert!(outline.contains("L002: ## Sec 1"));
        assert!(outline.contains("L004: ## Sec 2"));
    }

    #[test]
    fn test_render_table_view_csv() {
        let csv = "name,role,score\nAlice,Lead,95\nBob,Eng,88\nCharlie,Analyst,91\n";
        let rendered = render_table_view(csv, "csv", 1, 2);
        assert!(rendered.contains("| name | role | score |"));
        assert!(rendered.contains("| Alice | Lead | 95 |"));
        assert!(rendered.contains("| Bob | Eng | 88 |"));
        assert!(!rendered.contains("Charlie"));
        assert!(rendered.contains("Showing rows 1..2 of 3"));
    }

    #[test]
    fn test_extract_text_lines() {
        let text = "alpha\nbeta\ngamma\ndelta\nepsilon\n";
        let extracted = extract_text_lines(text, 2, 2);
        assert!(extracted.contains("2: beta"));
        assert!(extracted.contains("3: gamma"));
        assert!(!extracted.contains("alpha"));
        assert!(!extracted.contains("delta"));
    }
}
