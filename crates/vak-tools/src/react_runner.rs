use async_trait::async_trait;
use serde_json::Value;

use crate::{Tool, ToolContext, ToolOutput};

pub struct ReactPreviewTool {
    pub network: bool,
}

impl ReactPreviewTool {
    pub fn new(network: bool) -> Self {
        Self { network }
    }
}

impl Default for ReactPreviewTool {
    fn default() -> Self {
        Self::new(false)
    }
}

#[async_trait]
impl Tool for ReactPreviewTool {
    fn name(&self) -> &str {
        "react_preview"
    }

    fn description(&self) -> &str {
        "Compile and bundle an interactive React/TSX component inside an isolated, sandboxed preview document. \
The preview HTML and intermediate bundles are quarantined under .vak/scratch/previews/ and will never pollute \
the workspace directory until explicitly requested. The component renders inside a sandboxed cross-origin frame \
with strict CSP (network connect-src blocked unless allowed) and an ErrorBoundary to catch runtime exceptions."
    }

    fn schema(&self) -> Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "component_code": {
                    "type": "string",
                    "description": "Self-contained React component code in TSX/JSX (with a default export or top-level component)."
                },
                "title": {
                    "type": "string",
                    "description": "Title for this preview (default: React Component Preview)."
                },
                "props": {
                    "type": "object",
                    "description": "Optional mock props to pass into the root component."
                },
                "dependencies": {
                    "type": "array",
                    "items": { "type": "string" },
                    "description": "Optional list of additional packages to import (e.g. ['lucide-react', 'canvas-confetti'])."
                }
            },
            "required": ["component_code"]
        })
    }

    fn claims(&self, _args: &Value) -> crate::ResourceClaims {
        crate::ResourceClaims {
            exclusive: false,
            read_only: false,
            paths: Vec::new(),
        }
    }

    async fn execute(&self, args: &Value, ctx: &ToolContext) -> ToolOutput {
        let Some(code) = args.get("component_code").and_then(Value::as_str) else {
            return ToolOutput::error("missing required parameter: component_code");
        };

        if code.trim().is_empty() {
            return ToolOutput::error("component_code must not be empty");
        }

        let title = args
            .get("title")
            .and_then(Value::as_str)
            .unwrap_or("React Component Preview");

        let props_val = args
            .get("props")
            .cloned()
            .unwrap_or_else(|| serde_json::json!({}));

        let deps: Vec<String> = args
            .get("dependencies")
            .and_then(Value::as_array)
            .map(|arr| {
                arr.iter()
                    .filter_map(Value::as_str)
                    .map(str::to_string)
                    .collect()
            })
            .unwrap_or_default();

        // Quarantined scratch directory: .vak/scratch/previews/
        let previews_dir = ctx.cwd.join(".vak/scratch/previews");
        if let Err(e) = std::fs::create_dir_all(&previews_dir) {
            return ToolOutput::error(format!(
                "failed to initialize scratch previews directory: {e}"
            ));
        }

        let now_nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0);
        let preview_id = format!("preview_{now_nanos}");
        let preview_filename = format!("{preview_id}.html");
        let preview_path = previews_dir.join(&preview_filename);

        // Generate the self-contained sandboxed preview HTML
        let html_content = generate_preview_html(title, code, &props_val, &deps, self.network);
        if let Err(e) = std::fs::write(&preview_path, html_content) {
            return ToolOutput::error(format!("failed to write preview artifact: {e}"));
        }

        let rel_path = format!(".vak/scratch/previews/{preview_filename}");
        let response_payload = serde_json::json!({
            "status": "ready",
            "preview_id": preview_id,
            "title": title,
            "artifact_path": rel_path,
            "sandbox": "allow-scripts",
            "connect_src": if self.network { "allowed" } else { "blocked" }
        });

        let summary = format!(
            "[React Preview Generated]\n\
Preview ID: {preview_id}\n\
Title: {title}\n\
File: {rel_path}\n\
Policy: Sandboxed frame (allow-scripts), quarantined in .vak/scratch/previews/\n\
Status: Ready for presentation and preview dock rendering."
        );

        let mut output = ToolOutput::ok(ctx.truncate_output(summary));
        // Append structured metadata if needed
        output.content.push_str(&format!(
            "\n\n```vak:presentation\n{}\n```",
            response_payload
        ));
        output
    }
}

fn generate_preview_html(
    title: &str,
    code: &str,
    props: &Value,
    deps: &[String],
    allow_network: bool,
) -> String {
    let escaped_title = html_escape(title);
    let props_json = serde_json::to_string(props).unwrap_or_else(|_| "{}".into());
    let connect_csp = if allow_network {
        "connect-src * data: blob: https:;"
    } else {
        "connect-src 'none';"
    };

    let mut import_map_entries = vec![
        r#""react": "https://esm.sh/react@18.3.1""#.to_string(),
        r#""react/jsx-runtime": "https://esm.sh/react@18.3.1/jsx-runtime""#.to_string(),
        r#""react-dom": "https://esm.sh/react-dom@18.3.1""#.to_string(),
        r#""react-dom/client": "https://esm.sh/react-dom@18.3.1/client""#.to_string(),
    ];
    for dep in deps {
        let clean = dep.trim();
        if !clean.is_empty() {
            import_map_entries.push(format!(r#""{clean}": "https://esm.sh/{clean}""#));
        }
    }
    let import_map = format!(
        "{{\n      \"imports\": {{\n        {}\n      }}\n    }}",
        import_map_entries.join(",\n        ")
    );

    format!(
        r#"<!DOCTYPE html>
<html lang="en">
<head>
  <meta charset="UTF-8">
  <meta name="viewport" content="width=device-width, initial-scale=1.0">
  <title>{escaped_title}</title>
  <!-- Content Security Policy: sandbox isolated -->
  <meta http-equiv="Content-Security-Policy" content="default-src 'none'; script-src 'unsafe-inline' 'unsafe-eval' blob: https://esm.sh https://cdn.jsdelivr.net https://unpkg.com; style-src 'unsafe-inline' https://cdn.jsdelivr.net https://fonts.googleapis.com; font-src data: https://fonts.gstatic.com; img-src data: blob: https:; {connect_csp}">
  <!-- Tailwind CSS browser runtime -->
  <script src="https://cdn.jsdelivr.net/npm/@tailwindcss/browser@4"></script>
  <!-- Babel Standalone for in-browser JSX/TSX compilation -->
  <script src="https://unpkg.com/@babel/standalone@7.24.4/babel.min.js"></script>
  <script type="importmap">
    {import_map}
  </script>
  <style>
    body {{
      margin: 0;
      padding: 16px;
      font-family: -apple-system, BlinkMacSystemFont, "Segoe UI", Roboto, Helvetica, Arial, sans-serif;
      background-color: #0f1117;
      color: #e6edf3;
      min-height: 100vh;
      box-sizing: border-box;
    }}
    #root {{
      width: 100%;
      height: 100%;
    }}
    .error-boundary {{
      border: 1px solid #da3633;
      background: rgba(248, 81, 73, 0.1);
      color: #f85149;
      padding: 16px;
      border-radius: 8px;
      font-family: monospace;
      white-space: pre-wrap;
    }}
  </style>
</head>
<body>
  <div id="root">
    <div style="padding: 12px; color: #8b949e;">Mounting component...</div>
  </div>

  <script type="text/babel" data-type="module">
    import React, {{ useState, useEffect, useMemo, useCallback, useRef }} from 'react';
    import ReactDOM from 'react-dom/client';

    class ErrorBoundary extends React.Component {{
      constructor(props) {{
        super(props);
        this.state = {{ hasError: false, error: null }};
      }}
      static getDerivedStateFromError(error) {{
        return {{ hasError: true, error }};
      }}
      componentDidCatch(error, errorInfo) {{
        console.error("React Preview Error:", error, errorInfo);
      }}
      render() {{
        if (this.state.hasError) {{
          return (
            <div className="error-boundary">
              <h3 style={{{{ margin: '0 0 8px 0', fontSize: '16px' }}}}>Component Render Error</h3>
              <div>{{String(this.state.error?.message || this.state.error)}}</div>
              {{this.state.error?.stack && (
                <details style={{{{ marginTop: '8px', opacity: 0.8 }}}}>
                  <summary>Stack Trace</summary>
                  <pre style={{{{ fontSize: '12px' }}}}>{{this.state.error.stack}}</pre>
                </details>
              )}}
            </div>
          );
        }}
        return this.props.children;
      }}
    }}

    // User component injection
    {code}

    const defaultProps = {props_json};

    const container = document.getElementById('root');
    const root = ReactDOM.createRoot(container);

    // Identify target component (default export or first named component)
    let ComponentToRender = null;
    try {{
      if (typeof App !== 'undefined') {{
        ComponentToRender = App;
      }} else if (typeof Component !== 'undefined') {{
        ComponentToRender = Component;
      }} else if (typeof PreviewComponent !== 'undefined') {{
        ComponentToRender = PreviewComponent;
      }}
    }} catch (e) {{}}

    if (ComponentToRender) {{
      root.render(
        <ErrorBoundary>
          <ComponentToRender {{{{...defaultProps}}}} />
        </ErrorBoundary>
      );
    }} else {{
      root.render(
        <div className="error-boundary">
          <h3>Component Mounting Note</h3>
          <p>Please define an <code>App</code>, <code>Component</code>, or <code>PreviewComponent</code> function in your component code.</p>
        </div>
      );
    }}
  </script>
</body>
</html>
"#
    )
}

fn html_escape(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&#39;")
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[tokio::test]
    async fn react_preview_generates_isolated_html_and_quarantines() {
        let dir = tempdir().unwrap();
        let tool = ReactPreviewTool::new(false);
        let ctx = ToolContext::new(dir.path().to_path_buf());
        let args = serde_json::json!({
            "title": "Test Button",
            "component_code": "export default function App() { return <button className='bg-blue-500 text-white'>Click</button>; }"
        });
        let res = tool.execute(&args, &ctx).await;
        assert!(!res.is_error, "error: {}", res.content);
        assert!(res.content.contains("React Preview Generated"));
        assert!(res.content.contains("Test Button"));

        // Verify quarantined scratch directory
        let previews_dir = dir.path().join(".vak/scratch/previews");
        assert!(previews_dir.is_dir());

        // Verify html file generated with CSP
        let entries: Vec<_> = std::fs::read_dir(&previews_dir)
            .unwrap()
            .flatten()
            .collect();
        assert_eq!(entries.len(), 1);
        let html = std::fs::read_to_string(entries[0].path()).unwrap();
        assert!(html.contains("Content-Security-Policy"));
        assert!(html.contains("connect-src 'none'"));
        assert!(html.contains("ErrorBoundary"));

        // Verify cwd has no loose files
        let top_files: Vec<_> = std::fs::read_dir(dir.path())
            .unwrap()
            .flatten()
            .filter(|e| e.file_name() != ".vak")
            .collect();
        assert!(top_files.is_empty(), "loose files in cwd: {:?}", top_files);
    }

    #[tokio::test]
    async fn react_preview_requires_component_code() {
        let dir = tempdir().unwrap();
        let tool = ReactPreviewTool::new(false);
        let ctx = ToolContext::new(dir.path().to_path_buf());
        let args = serde_json::json!({ "title": "Empty" });
        let res = tool.execute(&args, &ctx).await;
        assert!(res.is_error);
    }

    #[tokio::test]
    async fn react_preview_supports_network_allowed_csp() {
        let dir = tempdir().unwrap();
        let tool = ReactPreviewTool::new(true);
        let ctx = ToolContext::new(dir.path().to_path_buf());
        let args = serde_json::json!({
            "title": "Network Component",
            "component_code": "export default function App() { return <div>Network</div>; }"
        });
        let res = tool.execute(&args, &ctx).await;
        assert!(!res.is_error);
        assert!(res.content.contains("allowed"));

        let previews_dir = dir.path().join(".vak/scratch/previews");
        let entries: Vec<_> = std::fs::read_dir(&previews_dir)
            .unwrap()
            .flatten()
            .collect();
        let html = std::fs::read_to_string(entries[0].path()).unwrap();
        assert!(html.contains("connect-src *"));
    }
}
