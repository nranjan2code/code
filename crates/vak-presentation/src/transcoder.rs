//! Universal Outcome Transcoder.
//!
//! Compiles session outcomes, dynamic living canvases (tables, dataframes,
//! decision matrices), and cited claims into standalone, zero-dependency,
//! responsive HTML documents suitable for distribution, archiving, or viewing.

use serde_json::Value;

/// Escape HTML special characters to prevent injection.
pub fn html_escape(input: &str) -> String {
    let mut escaped = String::with_capacity(input.len());
    for c in input.chars() {
        match c {
            '&' => escaped.push_str("&amp;"),
            '<' => escaped.push_str("&lt;"),
            '>' => escaped.push_str("&gt;"),
            '"' => escaped.push_str("&quot;"),
            '\'' => escaped.push_str("&#39;"),
            other => escaped.push(other),
        }
    }
    escaped
}

/// Transcodes an outcome markdown transcript with structured vak-blocks
/// into a self-contained interactive HTML document.
pub fn transcode_to_html(title: &str, content: &str) -> String {
    let safe_title = html_escape(title);
    let body_html = transcode_body(content);

    format!(
        r#"<!DOCTYPE html>
<html lang="en">
<head>
<meta charset="UTF-8">
<meta name="viewport" content="width=device-width, initial-scale=1.0">
<title>{safe_title}</title>
<style>
:root {{
  --bg: #0d1117;
  --surface: #161b22;
  --border: #30363d;
  --text: #c9d1d9;
  --text-bright: #f0f6fc;
  --text-muted: #8b949e;
  --accent: #58a6ff;
  --accent-muted: rgba(56, 139, 253, 0.15);
  --success: #3fb950;
  --font: -apple-system, BlinkMacSystemFont, "Segoe UI", Helvetica, Arial, sans-serif;
  --code-font: ui-monospace, SFMono-Regular, SF Mono, Menlo, Consolas, monospace;
}}
@media (prefers-color-scheme: light) {{
  :root {{
    --bg: #ffffff;
    --surface: #f6f8fa;
    --border: #d0d7de;
    --text: #24292f;
    --text-bright: #1f2328;
    --text-muted: #57606a;
    --accent: #0969da;
    --accent-muted: rgba(9, 105, 218, 0.1);
    --success: #1a7f37;
  }}
}}
* {{ box-sizing: border-box; margin: 0; padding: 0; }}
body {{
  background-color: var(--bg);
  color: var(--text);
  font-family: var(--font);
  font-size: 15px;
  line-height: 1.6;
  padding: 40px 20px;
  display: flex;
  justify-content: center;
}}
.container {{
  max-width: 880px;
  width: 100%;
}}
header {{
  margin-bottom: 32px;
  padding-bottom: 20px;
  border-bottom: 1px solid var(--border);
}}
h1 {{ font-size: 28px; font-weight: 600; color: var(--text-bright); margin-bottom: 8px; }}
h2 {{ font-size: 20px; font-weight: 600; color: var(--text-bright); margin: 24px 0 12px; }}
h3 {{ font-size: 16px; font-weight: 600; color: var(--text-bright); margin: 20px 0 8px; }}
p {{ margin-bottom: 16px; }}
ul, ol {{ margin: 0 0 16px 24px; }}
li {{ margin-bottom: 4px; }}
code {{ font-family: var(--code-font); font-size: 85%; background: var(--surface); padding: 2px 6px; border-radius: 4px; border: 1px solid var(--border); }}
pre {{ background: var(--surface); border: 1px solid var(--border); border-radius: 6px; padding: 16px; overflow-x: auto; margin-bottom: 20px; }}
pre code {{ background: none; border: none; padding: 0; font-size: 13px; }}
blockquote {{ border-left: 4px solid var(--accent); padding: 8px 16px; margin: 16px 0; background: var(--accent-muted); color: var(--text); border-radius: 0 4px 4px 0; }}
.card-table-container {{
  background: var(--surface);
  border: 1px solid var(--border);
  border-radius: 8px;
  margin: 24px 0;
  overflow: hidden;
}}
.table-header-bar {{
  padding: 12px 16px;
  border-bottom: 1px solid var(--border);
  display: flex;
  justify-content: space-between;
  align-items: center;
  gap: 12px;
}}
.table-title {{ font-weight: 600; font-size: 14px; color: var(--text-bright); }}
.table-search {{
  background: var(--bg);
  border: 1px solid var(--border);
  border-radius: 4px;
  padding: 4px 10px;
  font-size: 12px;
  color: var(--text);
  outline: none;
}}
.table-scroll {{ overflow-x: auto; }}
table {{
  width: 100%;
  border-collapse: collapse;
  font-size: 13px;
  text-align: left;
}}
th {{
  background: var(--surface);
  color: var(--text-bright);
  font-weight: 600;
  padding: 10px 14px;
  border-bottom: 1px solid var(--border);
  cursor: pointer;
  user-select: none;
  white-space: nowrap;
}}
th:hover {{ background: var(--accent-muted); }}
th .sort-icon {{ margin-left: 4px; opacity: 0.5; font-size: 10px; }}
td {{
  padding: 8px 14px;
  border-bottom: 1px solid var(--border);
}}
tr:last-child td {{ border-bottom: none; }}
tbody tr:hover {{ background: var(--accent-muted); }}
.decision-grid {{
  display: grid;
  grid-template-columns: repeat(auto-fit, minmax(260px, 1fr));
  gap: 16px;
  margin: 24px 0;
}}
.decision-card {{
  background: var(--surface);
  border: 1px solid var(--border);
  border-radius: 8px;
  padding: 16px;
  display: flex;
  flex-direction: column;
}}
.decision-title {{ font-size: 16px; font-weight: 600; color: var(--text-bright); margin-bottom: 8px; }}
.decision-summary {{ font-size: 13px; color: var(--text-muted); margin-bottom: 12px; }}
.pro-list, .con-list {{ font-size: 12px; margin: 4px 0 8px 16px; }}
.pro-list li {{ color: var(--success); }}
.con-list li {{ color: #f85149; }}
.citation-sup {{
  font-size: 75%;
  vertical-align: super;
  line-height: 0;
  color: var(--accent);
  text-decoration: none;
  cursor: pointer;
  padding: 0 2px;
}}
.citation-sup:hover {{ text-decoration: underline; }}
footer {{
  margin-top: 48px;
  padding-top: 16px;
  border-top: 1px solid var(--border);
  font-size: 12px;
  color: var(--text-muted);
  display: flex;
  justify-content: space-between;
}}
</style>
</head>
<body>
<div class="container">
  <header>
    <h1>{safe_title}</h1>
  </header>
  <main>
    {body_html}
  </main>
  <footer>
    <span>Generated by vak universal engine</span>
    <span>Self-contained living document</span>
  </footer>
</div>
<script>
document.querySelectorAll('.card-table-container').forEach(container => {{
  const input = container.querySelector('.table-search');
  const table = container.querySelector('table');
  if (!table) return;
  const tbody = table.querySelector('tbody');
  const headers = table.querySelectorAll('th');
  if (input) {{
    input.addEventListener('input', () => {{
      const q = input.value.toLowerCase();
      tbody.querySelectorAll('tr').forEach(tr => {{
        tr.style.display = tr.innerText.toLowerCase().includes(q) ? '' : 'none';
      }});
    }});
  }}
  headers.forEach((th, colIdx) => {{
    let asc = true;
    th.addEventListener('click', () => {{
      const rows = Array.from(tbody.querySelectorAll('tr'));
      rows.sort((a, b) => {{
        const cellA = a.children[colIdx]?.innerText.trim() || '';
        const cellB = b.children[colIdx]?.innerText.trim() || '';
        const numA = parseFloat(cellA);
        const numB = parseFloat(cellB);
        if (!isNaN(numA) && !isNaN(numB)) {{
          return asc ? numA - numB : numB - numA;
        }}
        return asc ? cellA.localeCompare(cellB) : cellB.localeCompare(cellA);
      }});
      asc = !asc;
      rows.forEach(r => tbody.appendChild(r));
    }});
  }});
}});
</script>
</body>
</html>"#
    )
}

fn transcode_body(content: &str) -> String {
    let mut out = String::new();
    let mut lines = content.lines().peekable();

    while let Some(line) = lines.next() {
        let trimmed = line.trim();

        // 1. Check for structured vak code blocks
        if trimmed.starts_with("```vak-table") || trimmed.starts_with("```vak-dataframe") {
            let mut json_str = String::new();
            for next in lines.by_ref() {
                if next.trim() == "```" {
                    break;
                }
                json_str.push_str(next);
                json_str.push('\n');
            }
            if let Ok(v) = serde_json::from_str::<Value>(&json_str) {
                out.push_str(&render_vak_table(&v));
            }
            continue;
        }

        if trimmed.starts_with("```vak-decision") || trimmed.starts_with("```vak-comparison") {
            let mut json_str = String::new();
            for next in lines.by_ref() {
                if next.trim() == "```" {
                    break;
                }
                json_str.push_str(next);
                json_str.push('\n');
            }
            if let Ok(v) = serde_json::from_str::<Value>(&json_str) {
                out.push_str(&render_vak_decision(&v));
            }
            continue;
        }

        // Standard code blocks
        if trimmed.starts_with("```") {
            let lang = trimmed.trim_start_matches('`').trim();
            let mut code_str = String::new();
            for next in lines.by_ref() {
                if next.trim() == "```" {
                    break;
                }
                code_str.push_str(next);
                code_str.push('\n');
            }
            out.push_str(&format!(
                "<pre><code class=\"language-{}\">{}</code></pre>\n",
                html_escape(lang),
                html_escape(&code_str)
            ));
            continue;
        }

        // Headings
        if let Some(rest) = trimmed.strip_prefix("### ") {
            out.push_str(&format!("<h3>{}</h3>\n", render_inline(rest)));
            continue;
        }
        if let Some(rest) = trimmed.strip_prefix("## ") {
            out.push_str(&format!("<h2>{}</h2>\n", render_inline(rest)));
            continue;
        }
        if let Some(rest) = trimmed.strip_prefix("# ") {
            out.push_str(&format!("<h1>{}</h1>\n", render_inline(rest)));
            continue;
        }

        // Blockquotes
        if let Some(rest) = trimmed.strip_prefix("> ") {
            out.push_str(&format!(
                "<blockquote>{}</blockquote>\n",
                render_inline(rest)
            ));
            continue;
        }

        // Unordered Lists
        if let Some(rest) = trimmed
            .strip_prefix("- ")
            .or_else(|| trimmed.strip_prefix("* "))
        {
            out.push_str(&format!("<ul><li>{}</li></ul>\n", render_inline(rest)));
            continue;
        }

        // Empty lines
        if trimmed.is_empty() {
            continue;
        }

        // Regular paragraphs
        out.push_str(&format!("<p>{}</p>\n", render_inline(trimmed)));
    }

    out
}

fn render_inline(text: &str) -> String {
    let out = html_escape(text);

    // Transform footnote citations [^1] into <a class="citation-sup">[^1]</a>
    let mut res = String::new();
    let mut rest = out.as_str();
    while let Some(start) = rest.find("[^") {
        res.push_str(&rest[..start]);
        let after = &rest[start..];
        if let Some(end) = after.find(']') {
            let tag = &after[..end + 1];
            res.push_str(&format!(
                "<a class=\"citation-sup\" href=\"#{}\">{}</a>",
                html_escape(tag),
                html_escape(tag)
            ));
            rest = &after[end + 1..];
        } else {
            res.push_str(after);
            rest = "";
            break;
        }
    }
    res.push_str(rest);
    res
}

fn render_vak_table(v: &Value) -> String {
    let title = v
        .get("title")
        .and_then(Value::as_str)
        .unwrap_or("Data Table");
    let cols = v.get("columns").and_then(Value::as_array);
    let rows = v.get("rows").and_then(Value::as_array);

    let mut out = format!(
        "<div class=\"card-table-container\">\n  <div class=\"table-header-bar\">\n    <span class=\"table-title\">{}</span>\n    <input type=\"text\" class=\"table-search\" placeholder=\"Search table...\">\n  </div>\n  <div class=\"table-scroll\">\n    <table>\n      <thead><tr>\n",
        html_escape(title)
    );

    if let Some(cols) = cols {
        for c in cols {
            let col_name = c.as_str().unwrap_or("");
            out.push_str(&format!(
                "        <th>{} <span class=\"sort-icon\">▲▼</span></th>\n",
                html_escape(col_name)
            ));
        }
    }
    out.push_str("      </tr></thead>\n      <tbody>\n");

    if let Some(rows) = rows {
        for row in rows {
            out.push_str("        <tr>\n");
            if let Some(cells) = row.as_array() {
                for cell in cells {
                    let text = match cell {
                        Value::String(s) => s.clone(),
                        other => other.to_string(),
                    };
                    out.push_str(&format!("          <td>{}</td>\n", html_escape(&text)));
                }
            }
            out.push_str("        </tr>\n");
        }
    }

    out.push_str("      </tbody>\n    </table>\n  </div>\n</div>\n");
    out
}

fn render_vak_decision(v: &Value) -> String {
    let mut out = String::from("<div class=\"decision-grid\">\n");
    if let Some(options) = v.get("options").and_then(Value::as_array) {
        for opt in options {
            let label = opt
                .get("label")
                .or_else(|| opt.get("name"))
                .and_then(Value::as_str)
                .unwrap_or("Option");
            let summary = opt
                .get("summary")
                .or_else(|| opt.get("description"))
                .and_then(Value::as_str)
                .unwrap_or("");
            out.push_str(&format!(
                "  <div class=\"decision-card\">\n    <div class=\"decision-title\">{}</div>\n    <div class=\"decision-summary\">{}</div>\n",
                html_escape(label),
                html_escape(summary)
            ));

            if let Some(pros) = opt.get("pros").and_then(Value::as_array) {
                out.push_str("    <ul class=\"pro-list\">\n");
                for p in pros {
                    out.push_str(&format!(
                        "      <li>+ {}</li>\n",
                        html_escape(p.as_str().unwrap_or(""))
                    ));
                }
                out.push_str("    </ul>\n");
            }

            if let Some(cons) = opt.get("cons").and_then(Value::as_array) {
                out.push_str("    <ul class=\"con-list\">\n");
                for c in cons {
                    out.push_str(&format!(
                        "      <li>- {}</li>\n",
                        html_escape(c.as_str().unwrap_or(""))
                    ));
                }
                out.push_str("    </ul>\n");
            }

            out.push_str("  </div>\n");
        }
    }
    out.push_str("</div>\n");
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn transcoder_renders_standalone_html_with_table_and_decision() {
        let text = r#"
# Executive Analysis

A comprehensive evaluation[^1] of architecture choices.

```vak-table
{
  "title": "System Latencies",
  "columns": ["Service", "p50 (ms)", "p99 (ms)"],
  "rows": [
    ["Auth Gateway", "12", "45"],
    ["Payment Broker", "34", "120"]
  ]
}
```

## Architectural Tradeoffs

```vak-decision
{
  "options": [
    {
      "label": "Option A: Event-Driven",
      "summary": "Decoupled async message bus",
      "pros": ["High scalability", "Fault tolerant"],
      "cons": ["Eventual consistency"]
    },
    {
      "label": "Option B: Sync REST",
      "summary": "Point-to-point HTTP/2",
      "pros": ["Immediate consistency"],
      "cons": ["Cascading latency"]
    }
  ]
}
```

[^1]: Verified via 2026 Telemetry Audit
"#;

        let html = transcode_to_html("System Architecture Brief", text);

        assert!(html.contains("<!DOCTYPE html>"));
        assert!(html.contains("System Architecture Brief"));
        assert!(html.contains("System Latencies"));
        assert!(html.contains("Auth Gateway"));
        assert!(html.contains("Option A: Event-Driven"));
        assert!(html.contains("Option B: Sync REST"));
        assert!(html.contains("citation-sup"));
        assert!(html.contains("document.querySelectorAll('.card-table-container')"));
    }
}
