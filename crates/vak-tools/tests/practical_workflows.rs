//! Product-shaped sandbox exercises. These deliberately use the real BashTool
//! contract so a passing unit test cannot hide a broken workbench workflow:
//! a command works in the workspace, and what it creates is reported.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use vak_tools::{Tool, ToolContext, bash::BashTool, sandbox_events::SandboxEventSink};

async fn run(
    workspace: &std::path::Path,
    id: &str,
    command: &str,
) -> (vak_tools::ToolOutput, Vec<vak_tools::SandboxEvent>) {
    let (sink, mut rx) = SandboxEventSink::new_with_id(id.into());
    let ctx = ToolContext::new(workspace.to_path_buf()).with_sandbox_sink(sink);
    let output = BashTool
        .execute(&serde_json::json!({"command": command}), &ctx)
        .await;
    let mut events = Vec::new();
    while let Ok(event) = rx.try_recv() {
        events.push(event);
    }
    (output, events)
}

#[tokio::test]
async fn full_stack_build_and_smoke_test_runs_in_the_workspace() {
    let workspace = tempfile::tempdir().unwrap();
    let command = r#"set -eu
mkdir -p app
cat > app/index.html <<'EOF'
<!doctype html><html><body><h1>vak app</h1></body></html>
EOF
cat > app/server.py <<'EOF'
from http.server import HTTPServer, SimpleHTTPRequestHandler
HTTPServer(('127.0.0.1', 8765), SimpleHTTPRequestHandler).serve_forever()
EOF
python3 -m py_compile app/server.py
python3 -m http.server 8765 --directory app >/tmp/vak-stack.log 2>&1 &
server_pid=$!
trap 'kill "$server_pid" 2>/dev/null || true' EXIT
sleep 0.3
python3 - <<'PY'
from urllib.request import urlopen
from pathlib import Path
assert '<h1>vak app</h1>' in Path('app/index.html').read_text()
assert Path('app/server.py').exists()
assert b'<h1>vak app</h1>' in urlopen('http://127.0.0.1:8765', timeout=2).read()
print('full-stack build and smoke test passed')
PY
"#;
    let (output, events) = run(workspace.path(), "workflow-stack", command).await;
    assert!(!output.is_error, "{}", output.content);
    assert!(
        output
            .content
            .contains("full-stack build and smoke test passed")
    );
    assert!(events.iter().any(|e| matches!(e, vak_tools::SandboxEvent::ArtifactGenerated { path, .. } if path.ends_with("app/index.html"))));
    assert!(workspace.path().join("app/index.html").is_file());
}

#[tokio::test]
async fn research_bundle_validates_sources_and_citations() {
    let workspace = tempfile::tempdir().unwrap();
    let command = r#"set -eu
mkdir -p research
cat > research/sources.json <<'EOF'
[{"title":"Rust Book","url":"https://doc.rust-lang.org/book/"},{"title":"HTTP Semantics","url":"https://httpwg.org/specs/"}]
EOF
cat > research/synthesis.md <<'EOF'
# Findings

Rust provides memory-safe systems programming [1]. HTTP semantics define interoperable requests [2].

## Sources
1. https://doc.rust-lang.org/book/
2. https://httpwg.org/specs/
EOF
python3 - <<'PY'
import json
from pathlib import Path
sources=json.loads(Path('research/sources.json').read_text())
text=Path('research/synthesis.md').read_text()
assert len(sources)==2
assert all(item['url'].startswith('https://') for item in sources)
assert '[1]' in text and '[2]' in text
print('research evidence and citations verified')
PY
"#;
    let (output, events) = run(workspace.path(), "workflow-research", command).await;
    assert!(!output.is_error, "{}", output.content);
    assert!(
        output
            .content
            .contains("research evidence and citations verified")
    );
    assert!(events.iter().any(|e| matches!(e, vak_tools::SandboxEvent::ArtifactGenerated { path, .. } if path.ends_with("research/synthesis.md"))));
    assert!(workspace.path().join("research/synthesis.md").is_file());
}

#[tokio::test]
async fn document_and_data_outputs_are_reported_as_artifacts() {
    let workspace = tempfile::tempdir().unwrap();
    let command = r#"set -eu
mkdir -p deliverable
cat > deliverable/report.md <<'EOF'
# Quarterly report

Generated from the verified task environment.
EOF
cat > deliverable/metrics.csv <<'EOF'
metric,value
latency_ms,42
success_rate,0.99
EOF
python3 - <<'PY'
from pathlib import Path
assert Path('deliverable/report.md').read_text().startswith('# Quarterly report')
rows=Path('deliverable/metrics.csv').read_text().splitlines()
assert rows[0]=='metric,value' and len(rows)==3
print('document and data outputs validated')
PY
"#;
    let (output, events) = run(workspace.path(), "workflow-docs", command).await;
    assert!(!output.is_error, "{}", output.content);
    assert!(
        output
            .content
            .contains("document and data outputs validated")
    );
    let artifacts: Vec<_> = events
        .iter()
        .filter_map(|event| match event {
            vak_tools::SandboxEvent::ArtifactGenerated { path, .. } => Some(path.as_str()),
            _ => None,
        })
        .collect();
    assert!(
        artifacts
            .iter()
            .any(|path| path.ends_with("deliverable/report.md"))
    );
    assert!(
        artifacts
            .iter()
            .any(|path| path.ends_with("deliverable/metrics.csv"))
    );
    assert!(workspace.path().join("deliverable/report.md").is_file());
}
