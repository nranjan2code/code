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
                "component_path": {
                    "type": "string",
                    "description": "Optional relative path to an existing .tsx/.jsx component file in the workspace."
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
            }
        })
    }

    fn claims(&self, args: &Value) -> crate::ResourceClaims {
        let mut paths = Vec::new();
        if let Some(path) = args
            .get("component_path")
            .or_else(|| args.get("path"))
            .or_else(|| args.get("file"))
            .and_then(Value::as_str)
        {
            paths.push(path.to_string());
        }
        crate::ResourceClaims {
            exclusive: false,
            read_only: false,
            paths,
        }
    }

    async fn execute(&self, args: &Value, ctx: &ToolContext) -> ToolOutput {
        let inline_code = args
            .get("component_code")
            .or_else(|| args.get("code"))
            .or_else(|| args.get("component"))
            .or_else(|| args.get("jsx"))
            .or_else(|| args.get("tsx"))
            .and_then(Value::as_str);

        let file_path = args
            .get("component_path")
            .or_else(|| args.get("path"))
            .or_else(|| args.get("file"))
            .and_then(Value::as_str);

        let code = match (inline_code, file_path) {
            (Some(c), _) if !c.trim().is_empty() => c.to_string(),
            (_, Some(rel)) => {
                let candidate = ctx.cwd.join(rel);
                match candidate.canonicalize() {
                    Ok(canon) => {
                        let ws_canon = ctx.cwd.canonicalize().unwrap_or_else(|_| ctx.cwd.clone());
                        if !canon.starts_with(&ws_canon) {
                            return ToolOutput::error(format!(
                                "access denied: component path '{}' resolves outside workspace",
                                rel
                            ));
                        }
                        match std::fs::read_to_string(&canon) {
                            Ok(content) => content,
                            Err(e) => {
                                return ToolOutput::error(format!(
                                    "failed to read component file '{rel}': {e}"
                                ));
                            }
                        }
                    }
                    Err(e) => {
                        return ToolOutput::error(format!("component file not found '{rel}': {e}"));
                    }
                }
            }
            _ => {
                return ToolOutput::error(
                    "missing required parameter: either 'component_code' or 'component_path' must be provided",
                );
            }
        };

        if code.trim().is_empty() {
            return ToolOutput::error("component code must not be empty");
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

        // Persist the component source code to scratch so it can be inspected/edited
        let source_filename = format!("{preview_id}.tsx");
        let source_path = previews_dir.join(&source_filename);
        if let Err(e) = std::fs::write(&source_path, &code) {
            return ToolOutput::error(format!("failed to write component source artifact: {e}"));
        }

        let preview_filename = format!("{preview_id}.html");
        let preview_path = previews_dir.join(&preview_filename);

        // Generate the self-contained sandboxed preview HTML
        let html_content = generate_preview_html(title, &code, &props_val, &deps, self.network);
        if let Err(e) = std::fs::write(&preview_path, html_content) {
            return ToolOutput::error(format!("failed to write preview artifact: {e}"));
        }

        let rel_source = format!(".vak/scratch/previews/{source_filename}");
        let rel_path = format!(".vak/scratch/previews/{preview_filename}");
        let response_payload = serde_json::json!({
            "semantic_type": "react.preview",
            "payload": {
                "status": "ready",
                "preview_id": preview_id,
                "title": title,
                "artifact_path": rel_path,
                "source_path": rel_source,
                "sandbox": "allow-scripts",
                "connect_src": if self.network { "allowed" } else { "blocked" }
            }
        });

        let mut summary = format!(
            "[React Preview Generated]\n\
Preview ID: {preview_id}\n\
Title: {title}\n\
Component Source: {rel_source}\n\
Preview HTML: {rel_path}\n\
Policy: Sandboxed frame (allow-scripts), quarantined in .vak/scratch/previews/\n\
Status: Live in Desktop Preview dock and chat cards."
        );
        if let Some(src_file) = file_path {
            summary.push_str(&format!("\nLoaded from: {src_file}"));
        }

        let mut output = ToolOutput::ok(ctx.truncate_output(summary));
        output
            .content
            .push_str(&format!("\n\n```vak\n{}\n```", response_payload));
        output
    }
}

fn prepare_component_code(code: &str) -> (String, Vec<String>) {
    let mut detected_names = Vec::new();
    let mut processed_lines = Vec::new();

    for line in code.lines() {
        let trimmed = line.trim();
        // Comment out duplicate imports of react and react-dom to avoid Babel duplicate identifier errors
        if (trimmed.starts_with("import ") || trimmed.starts_with("import\t"))
            && (trimmed.contains("'react'")
                || trimmed.contains("\"react\"")
                || trimmed.contains("'react-dom")
                || trimmed.contains("\"react-dom"))
        {
            processed_lines.push(format!("// [vak runtime] {line}"));
            continue;
        }

        // Handle export default variants
        if trimmed.starts_with("export default ") {
            let after_export = &trimmed["export default ".len()..];
            if after_export.starts_with("function ") || after_export.starts_with("function\t") {
                let func_part = after_export["function".len()..].trim_start();
                if let Some(name_end) = func_part.find(|c: char| !c.is_alphanumeric() && c != '_') {
                    let fn_name = &func_part[..name_end];
                    if !fn_name.is_empty() {
                        detected_names.push(fn_name.to_string());
                    }
                }
                processed_lines.push(format!("window.__vak_default_export = {after_export};"));
                continue;
            } else if after_export.starts_with("class ") || after_export.starts_with("class\t") {
                let class_part = after_export["class".len()..].trim_start();
                if let Some(name_end) = class_part.find(|c: char| !c.is_alphanumeric() && c != '_')
                {
                    let class_name = &class_part[..name_end];
                    if !class_name.is_empty() {
                        detected_names.push(class_name.to_string());
                    }
                }
                processed_lines.push(format!("window.__vak_default_export = {after_export};"));
                continue;
            } else {
                let expr = after_export.trim_end_matches(';');
                if expr.chars().all(|c| c.is_alphanumeric() || c == '_') && !expr.is_empty() {
                    detected_names.push(expr.to_string());
                }
                processed_lines.push(format!("window.__vak_default_export = {expr};"));
                continue;
            }
        }

        // Detect named component functions or consts e.g. function Dashboard() or const Dashboard =
        if trimmed.starts_with("function ") || trimmed.starts_with("function\t") {
            let func_part = trimmed["function".len()..].trim_start();
            if let Some(name_end) = func_part.find(|c: char| !c.is_alphanumeric() && c != '_') {
                let fn_name = &func_part[..name_end];
                if fn_name
                    .chars()
                    .next()
                    .is_some_and(|c| c.is_ascii_uppercase())
                {
                    detected_names.push(fn_name.to_string());
                }
            }
        } else if trimmed.starts_with("const ")
            || trimmed.starts_with("let ")
            || trimmed.starts_with("var ")
        {
            let decl_part = trimmed
                .trim_start_matches("const ")
                .trim_start_matches("let ")
                .trim_start_matches("var ")
                .trim_start();
            if let Some(name_end) = decl_part.find(|c: char| !c.is_alphanumeric() && c != '_') {
                let var_name = &decl_part[..name_end];
                if var_name
                    .chars()
                    .next()
                    .is_some_and(|c| c.is_ascii_uppercase())
                {
                    detected_names.push(var_name.to_string());
                }
            }
        }

        processed_lines.push(line.to_string());
    }

    detected_names.dedup();
    (processed_lines.join("\n"), detected_names)
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

    let (processed_code, detected_names) = prepare_component_code(code);
    let mut detection_checks = String::new();
    for name in &detected_names {
        detection_checks.push_str(&format!(
            "      if (!ComponentToRender && typeof {name} !== 'undefined') ComponentToRender = {name};\n"
        ));
    }

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

  <script>
    window.addEventListener('error', function(event) {{
      console.error('React Preview Error:', event.error || event.message);
      var root = document.getElementById('root');
      if (root && !root.querySelector('.error-boundary')) {{
        var msg = event.error ? (event.error.stack || event.error.message) : event.message;
        root.innerHTML = '<div class="error-boundary"><h3 style="margin:0 0 8px 0;font-size:16px;">Component Error</h3><pre style="font-size:12px;margin:0;">' + (msg || 'Unknown error occurred') + '</pre></div>';
      }}
    }});
    window.addEventListener('unhandledrejection', function(event) {{
      console.error('React Preview Unhandled Rejection:', event.reason);
      var root = document.getElementById('root');
      if (root && !root.querySelector('.error-boundary')) {{
        var reason = event.reason ? (event.reason.stack || event.reason.message || String(event.reason)) : 'Promise rejected';
        root.innerHTML = '<div class="error-boundary"><h3 style="margin:0 0 8px 0;font-size:16px;">Promise Rejection</h3><pre style="font-size:12px;margin:0;">' + reason + '</pre></div>';
      }}
    }});
  </script>

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
    window.__vak_default_export = null;
    {processed_code}

    const defaultProps = {props_json};

    const container = document.getElementById('root');
    const root = ReactDOM.createRoot(container);

    // Identify target component (default export or detected component)
    let ComponentToRender = null;
    try {{
      if (typeof window.__vak_default_export === 'function' || (typeof window.__vak_default_export === 'object' && window.__vak_default_export !== null)) {{
        ComponentToRender = window.__vak_default_export;
      }}
{detection_checks}      if (!ComponentToRender && typeof App !== 'undefined') ComponentToRender = App;
      if (!ComponentToRender && typeof Component !== 'undefined') ComponentToRender = Component;
      if (!ComponentToRender && typeof PreviewComponent !== 'undefined') ComponentToRender = PreviewComponent;
      if (!ComponentToRender && typeof Main !== 'undefined') ComponentToRender = Main;
      if (!ComponentToRender && typeof Root !== 'undefined') ComponentToRender = Root;
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
          <p>Please define a React component function or use <code>export default function Component()</code>.</p>
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
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
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

        // Verify html and tsx files generated in scratch
        let entries: Vec<_> = std::fs::read_dir(&previews_dir)
            .unwrap()
            .flatten()
            .collect();
        assert_eq!(
            entries.len(),
            2,
            "expected .html and .tsx artifacts, found: {:?}",
            entries
        );
        let has_html = entries
            .iter()
            .any(|e| e.path().extension().is_some_and(|ext| ext == "html"));
        let has_tsx = entries
            .iter()
            .any(|e| e.path().extension().is_some_and(|ext| ext == "tsx"));
        assert!(has_html && has_tsx);

        let html_entry = entries
            .iter()
            .find(|e| e.path().extension().is_some_and(|ext| ext == "html"))
            .unwrap();
        let html = std::fs::read_to_string(html_entry.path()).unwrap();
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
    async fn react_preview_requires_code_or_path() {
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
        let html_entry = std::fs::read_dir(&previews_dir)
            .unwrap()
            .flatten()
            .find(|e| e.path().extension().is_some_and(|ext| ext == "html"))
            .unwrap();
        let html = std::fs::read_to_string(html_entry.path()).unwrap();
        assert!(html.contains("connect-src *"));
    }

    #[tokio::test]
    async fn react_preview_accepts_parameter_aliases_and_mounts_custom_components() {
        let dir = tempdir().unwrap();
        let tool = ReactPreviewTool::new(false);
        let ctx = ToolContext::new(dir.path().to_path_buf());
        // Use alias "code" instead of "component_code", with explicit import React and custom component name
        let args = serde_json::json!({
            "title": "Custom Dashboard",
            "code": "import React, { useState } from 'react';\nexport default function CustomDashboard() { return <div>Dashboard Ready</div>; }"
        });
        let res = tool.execute(&args, &ctx).await;
        assert!(!res.is_error, "error: {}", res.content);
        assert!(res.content.contains("Custom Dashboard"));

        let previews_dir = dir.path().join(".vak/scratch/previews");
        let html_entry = std::fs::read_dir(&previews_dir)
            .unwrap()
            .flatten()
            .find(|e| e.path().extension().is_some_and(|ext| ext == "html"))
            .unwrap();
        let html = std::fs::read_to_string(html_entry.path()).unwrap();
        // Verify that duplicate import was neutralized
        assert!(html.contains("// [vak runtime] import React, { useState } from 'react';"));
        // Verify that custom component is registered and mounted
        assert!(html.contains("CustomDashboard"));
        assert!(html.contains("window.__vak_default_export"));
    }

    #[tokio::test]
    async fn react_preview_loads_component_from_file_path() {
        let dir = tempdir().unwrap();
        let tool = ReactPreviewTool::new(false);
        let ctx = ToolContext::new(dir.path().to_path_buf());

        // Write a component file in the workspace
        let comp_file = dir.path().join("Widget.tsx");
        std::fs::write(
            &comp_file,
            "export default function Widget() { return <span>Loaded from file</span>; }",
        )
        .unwrap();

        let args = serde_json::json!({
            "title": "File Widget",
            "component_path": "Widget.tsx"
        });
        let res = tool.execute(&args, &ctx).await;
        assert!(!res.is_error, "error: {}", res.content);
        assert!(res.content.contains("File Widget"));
        assert!(res.content.contains("Widget.tsx"));
    }
}
