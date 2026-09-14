//! In-Memory Universal Document Ingestion (`doc_read`).
//!
//! Provides structured, token-bounded document extraction across
//! Markdown, Plaintext, CSV, TSV, JSON, YAML, TOML, INI, ENV, and HTML/XML formats
//! without external subprocesses. Supports section outline navigation, table rendering,
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
        "Inspect and extract text, sections, tables, or summaries from documents and data files (Markdown, Plaintext, CSV, TSV, JSON, YAML, TOML, INI, ENV, HTML, XML). Supports section navigation, outlines, and paginated table views."
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
                    "description": "Optional section heading to extract (e.g. '## Methodology', 'Results', or '[server]')"
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
                    ToolOutput::ok(extract_section(&content, target_sec, &ext, offset, limit))
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
        "toml" => {
            if let Ok(val) = toml::from_str::<toml::Value>(content) {
                if let toml::Value::Table(tbl) = val {
                    let mut sections = Vec::new();
                    for (k, v) in tbl {
                        match v {
                            toml::Value::Table(_) => sections.push(format!("- [{k}] (table)")),
                            toml::Value::Array(arr) => sections.push(format!("- [[{k}]] (array of {} items)", arr.len())),
                            _ => sections.push(format!("- {k}")),
                        }
                    }
                    format!("TOML Configuration sections & keys:\n{}", sections.join("\n"))
                } else {
                    "TOML Document".into()
                }
            } else {
                "Malformed TOML".into()
            }
        }
        "yaml" | "yml" => {
            let mut keys = Vec::new();
            for line in content.lines() {
                let trimmed = line.trim();
                if !trimmed.starts_with('#') && trimmed.contains(':') && !line.starts_with(' ') && !line.starts_with('\t') {
                    if let Some((k, _)) = trimmed.split_once(':') {
                        let clean_k = k.trim().trim_start_matches('-').trim();
                        if !clean_k.is_empty() {
                            keys.push(format!("- {clean_k}"));
                        }
                    }
                }
            }
            if keys.is_empty() {
                "YAML Document".into()
            } else {
                format!("YAML Root Keys:\n{}", keys.join("\n"))
            }
        }
        "ini" | "env" | "properties" | "conf" => {
            let mut sections = Vec::new();
            let mut key_count = 0;
            for line in content.lines() {
                let trimmed = line.trim();
                if trimmed.starts_with('[') && trimmed.ends_with(']') {
                    sections.push(trimmed.to_string());
                } else if !trimmed.starts_with('#') && !trimmed.starts_with(';') && (trimmed.contains('=') || trimmed.contains(':')) {
                    key_count += 1;
                }
            }
            if sections.is_empty() {
                format!("Key-Value configuration with {key_count} entries.")
            } else {
                format!("Config with {} sections and {} keys:\n{}", sections.len(), key_count, sections.join("\n"))
            }
        }
        "html" | "htm" | "xml" => {
            let mut headings = Vec::new();
            for line in content.lines() {
                let lower = line.to_ascii_lowercase();
                for tag in &["<title>", "<h1>", "<h2>", "<h3>", "<h4>", "<h5>", "<h6>"] {
                    if let Some(start) = lower.find(tag) {
                        let end_tag = tag.replace('<', "</");
                        let text = if let Some(end) = lower.find(&end_tag) {
                            &line[start + tag.len()..end]
                        } else {
                            &line[start + tag.len()..]
                        };
                        let clean = text.trim();
                        if !clean.is_empty() {
                            headings.push(format!("{}: {}", tag.trim_matches(&['<', '>'][..]), clean));
                        }
                    }
                }
            }
            if headings.is_empty() {
                "HTML/XML document without explicit heading tags.".into()
            } else {
                format!("Document Headings:\n{}", headings.join("\n"))
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
    match ext {
        "json" => render_json_table(content, offset, limit),
        "ini" | "env" | "properties" | "conf" => render_kv_table(content, offset, limit),
        "html" | "htm" | "xml" => render_html_table(content, offset, limit),
        _ => render_delimited_table(content, ext, offset, limit),
    }
}

fn render_delimited_table(content: &str, ext: &str, offset: usize, limit: usize) -> String {
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

fn render_json_table(content: &str, offset: usize, limit: usize) -> String {
    let Ok(val) = serde_json::from_str::<Value>(content) else {
        return "Malformed JSON dataset.".into();
    };
    let Some(arr) = val.as_array() else {
        return "JSON value is not an array of records.".into();
    };
    if arr.is_empty() {
        return "Empty JSON array.".into();
    }

    let mut header_keys = Vec::new();
    for item in arr {
        if let Some(obj) = item.as_object() {
            for k in obj.keys() {
                if !header_keys.contains(k) {
                    header_keys.push(k.clone());
                }
            }
        }
    }
    if header_keys.is_empty() {
        return "JSON array contains no structured objects.".into();
    }

    let header_line = format!("| {} |", header_keys.join(" | "));
    let separator_line = format!("| {} |", vec!["---"; header_keys.len()].join(" | "));

    let total = arr.len();
    let start_idx = (offset.saturating_sub(1)).min(total);
    let end_idx = (start_idx + limit).min(total);

    let mut rows = Vec::new();
    for item in &arr[start_idx..end_idx] {
        let cells: Vec<String> = header_keys.iter().map(|k| {
            item.get(k).map(|v| match v {
                Value::String(s) => s.clone(),
                other => other.to_string(),
            }).unwrap_or_default()
        }).collect();
        rows.push(format!("| {} |", cells.join(" | ")));
    }

    format!(
        "{header_line}\n{separator_line}\n{}\n\n[Showing rows {}..{} of {} total rows]",
        rows.join("\n"),
        start_idx + 1,
        end_idx,
        total
    )
}

fn render_kv_table(content: &str, offset: usize, limit: usize) -> String {
    let mut pairs = Vec::new();
    let mut current_section = String::new();

    for line in content.lines() {
        let trimmed = line.trim();
        if trimmed.starts_with('#') || trimmed.starts_with(';') || trimmed.is_empty() {
            continue;
        }
        if trimmed.starts_with('[') && trimmed.ends_with(']') {
            current_section = trimmed[1..trimmed.len() - 1].trim().to_string();
            continue;
        }
        let sep = if trimmed.contains('=') { '=' } else { ':' };
        if let Some((k, v)) = trimmed.split_once(sep) {
            let key_display = if current_section.is_empty() {
                k.trim().to_string()
            } else {
                format!("{}.{}", current_section, k.trim())
            };
            pairs.push((key_display, v.trim().to_string()));
        }
    }

    if pairs.is_empty() {
        return "No key-value entries found in configuration.".into();
    }

    let total = pairs.len();
    let start_idx = (offset.saturating_sub(1)).min(total);
    let end_idx = (start_idx + limit).min(total);

    let header_line = "| Key | Value |";
    let sep_line = "| --- | --- |";
    let rows: Vec<String> = pairs[start_idx..end_idx]
        .iter()
        .map(|(k, v)| format!("| {k} | {v} |"))
        .collect();

    format!(
        "{header_line}\n{sep_line}\n{}\n\n[Showing rows {}..{} of {} total rows]",
        rows.join("\n"),
        start_idx + 1,
        end_idx,
        total
    )
}

fn render_html_table(content: &str, offset: usize, limit: usize) -> String {
    // Extract table rows between <tr> and </tr>
    let mut rows: Vec<Vec<String>> = Vec::new();
    let lower = content.to_ascii_lowercase();
    let mut cursor = 0;

    while let Some(tr_start) = lower[cursor..].find("<tr") {
        let abs_tr_start = cursor + tr_start;
        let Some(tr_close) = lower[abs_tr_start..].find('>') else { break; };
        let content_start = abs_tr_start + tr_close + 1;
        let Some(tr_end) = lower[content_start..].find("</tr>") else { break; };
        let row_html = &content[content_start..content_start + tr_end];
        cursor = content_start + tr_end + 5;

        // Extract <th> or <td> inside row_html
        let mut cells = Vec::new();
        let row_lower = row_html.to_ascii_lowercase();
        let mut cell_cursor = 0;
        while cell_cursor < row_html.len() {
            let next_th = row_lower[cell_cursor..].find("<th");
            let next_td = row_lower[cell_cursor..].find("<td");
            let next_cell = match (next_th, next_td) {
                (Some(a), Some(b)) => Some((a.min(b), if a <= b { "</th>" } else { "</td>" })),
                (Some(a), None) => Some((a, "</th>")),
                (None, Some(b)) => Some((b, "</td>")),
                (None, None) => None,
            };

            let Some((cell_start_rel, close_tag)) = next_cell else { break; };
            let abs_cell_start = cell_cursor + cell_start_rel;
            let Some(open_tag_close) = row_lower[abs_cell_start..].find('>') else { break; };
            let val_start = abs_cell_start + open_tag_close + 1;
            let val_end = row_lower[val_start..].find(close_tag).map(|idx| val_start + idx).unwrap_or(row_html.len());

            // Clean inner tags
            let raw_text = &row_html[val_start..val_end];
            let clean_text = strip_html_tags(raw_text).replace('|', "\\|").trim().to_string();
            cells.push(clean_text);
            cell_cursor = val_end + close_tag.len();
        }

        if !cells.is_empty() {
            rows.push(cells);
        }
    }

    if rows.is_empty() {
        return "No HTML <table> rows found in document.".into();
    }

    let headers = &rows[0];
    let header_line = format!("| {} |", headers.join(" | "));
    let sep_line = format!("| {} |", vec!["---"; headers.len()].join(" | "));

    let total = rows.len().saturating_sub(1);
    let start_idx = (offset.saturating_sub(1)).min(total);
    let end_idx = (start_idx + limit).min(total);

    let mut out_rows = Vec::new();
    for row in rows.iter().skip(1 + start_idx).take(limit) {
        out_rows.push(format!("| {} |", row.join(" | ")));
    }

    format!(
        "{header_line}\n{sep_line}\n{}\n\n[Showing rows {}..{} of {} total rows]",
        out_rows.join("\n"),
        start_idx + 1,
        end_idx,
        total
    )
}

fn strip_html_tags(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut in_tag = false;
    for c in s.chars() {
        if c == '<' {
            in_tag = true;
        } else if c == '>' {
            in_tag = false;
        } else if !in_tag {
            out.push(c);
        }
    }
    out
}

fn extract_section(content: &str, target_section: &str, ext: &str, offset: usize, limit: usize) -> String {
    match ext {
        "ini" | "toml" | "conf" => extract_bracket_section(content, target_section, offset, limit),
        _ => extract_heading_section(content, target_section, offset, limit),
    }
}

fn extract_bracket_section(content: &str, target_section: &str, offset: usize, limit: usize) -> String {
    let clean_target = target_section.trim().trim_matches(&['[', ']'][..]).to_ascii_lowercase();
    let lines: Vec<&str> = content.lines().collect();

    let mut in_section = false;
    let mut collected = Vec::new();

    for line in lines {
        let trimmed = line.trim();
        if trimmed.starts_with('[') && trimmed.ends_with(']') {
            let sec_name = trimmed[1..trimmed.len() - 1].trim().to_ascii_lowercase();
            if in_section {
                break;
            } else if sec_name == clean_target || sec_name.contains(&clean_target) {
                in_section = true;
                collected.push(line);
                continue;
            }
        }
        if in_section {
            collected.push(line);
        }
    }

    if collected.is_empty() {
        format!("Section '[{target_section}]' not found in configuration.")
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

fn extract_heading_section(content: &str, target_section: &str, offset: usize, limit: usize) -> String {
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
        let extracted = extract_section(doc, "Methods", "md", 1, 50);
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
    fn test_render_json_table() {
        let json_data = r#"[
            {"id": "usr-1", "name": "Alice", "active": true},
            {"id": "usr-2", "name": "Bob", "active": false}
        ]"#;
        let rendered = render_table_view(json_data, "json", 1, 10);
        assert!(rendered.contains("id") && rendered.contains("name") && rendered.contains("active"));
        assert!(rendered.contains("Alice") && rendered.contains("usr-1"));
        assert!(rendered.contains("Bob") && rendered.contains("usr-2"));
        assert!(rendered.contains("Showing rows 1..2 of 2 total rows"));
    }

    #[test]
    fn test_render_kv_table_ini() {
        let ini = "[server]\nport = 8080\nhost = localhost\n[db]\nurl = postgresql://db\n";
        let rendered = render_table_view(ini, "ini", 1, 10);
        assert!(rendered.contains("| server.port | 8080 |"));
        assert!(rendered.contains("| db.url | postgresql://db |"));
    }

    #[test]
    fn test_render_html_table() {
        let html = "<table><tr><th>Metric</th><th>Value</th></tr><tr><td>Latency</td><td>12ms</td></tr><tr><td>Throughput</td><td>1000req/s</td></tr></table>";
        let rendered = render_table_view(html, "html", 1, 5);
        assert!(rendered.contains("| Metric | Value |"));
        assert!(rendered.contains("| Latency | 12ms |"));
        assert!(rendered.contains("| Throughput | 1000req/s |"));
    }

    #[test]
    fn test_extract_bracket_section_toml() {
        let toml_doc = "[gateway]\nport = 3000\n[agent]\nname = \"Research\"\nrole = \"researcher\"\n";
        let extracted = extract_section(toml_doc, "agent", "toml", 1, 20);
        assert!(extracted.contains("name = \"Research\""));
        assert!(extracted.contains("role = \"researcher\""));
        assert!(!extracted.contains("port = 3000"));
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

