//! A skill's program runs in the broker worker under the sandbox its
//! declaration gives it (docs/design/93-self-evolving-capabilities.md, E1).

#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::path::{Path, PathBuf};

use serde_json::{Value, json};
use vak_tools::capability::{CapabilityTool, DECLARATION, Program};
use vak_tools::{Tool, ToolContext};

fn tool(skill: &Path, program: &str, extra: Value) -> CapabilityTool {
    std::fs::write(skill.join("run.py"), program).unwrap();
    let mut declaration = json!({
        "entry": "run.py",
        "arguments": {"type": "object", "properties": {
            "amount": {"type": "number", "description": "The bill"},
            "people": {"type": "integer", "description": "How many share it"}
        }}
    });
    for (key, value) in extra.as_object().unwrap() {
        declaration[key] = value.clone();
    }
    std::fs::write(skill.join(DECLARATION), declaration.to_string()).unwrap();
    let program = Program::load(skill).unwrap().unwrap();
    CapabilityTool::new(
        "split-bill".into(),
        "Split a bill".into(),
        program,
        PathBuf::from(env!("CARGO_BIN_EXE_vak-tool-worker")),
    )
}

fn context(workspace: &Path) -> ToolContext {
    ToolContext::new(workspace.to_path_buf()).with_agent_id("capability-test")
}

#[tokio::test]
async fn a_program_answers_in_json_through_the_worker() {
    let workspace = tempfile::tempdir().unwrap();
    let skill = tempfile::tempdir().unwrap();
    let tool = tool(
        skill.path(),
        "import json, sys\nargs = json.load(sys.stdin)\nprint(json.dumps({'each': args['amount'] / args['people']}))\n",
        json!({}),
    );
    let output = tool
        .execute(
            &json!({"amount": 90, "people": 3, "timeout_ms": 20000}),
            &context(workspace.path()),
        )
        .await;
    assert!(!output.is_error(), "{}", output.content);
    assert_eq!(
        serde_json::from_str::<Value>(&output.content).unwrap(),
        json!({"each": 30.0})
    );
}

#[tokio::test]
async fn a_program_that_prints_no_json_fails_with_what_it_printed() {
    let workspace = tempfile::tempdir().unwrap();
    let skill = tempfile::tempdir().unwrap();
    let tool = tool(skill.path(), "print('thirty each')\n", json!({}));
    let output = tool
        .execute(&json!({"timeout_ms": 20000}), &context(workspace.path()))
        .await;
    assert!(output.is_error());
    assert!(
        output.content.contains("one JSON value"),
        "{}",
        output.content
    );
    assert!(output.content.contains("thirty each"), "{}", output.content);
}

#[cfg(any(target_os = "macos", target_os = "linux"))]
#[tokio::test]
async fn a_program_that_declares_no_hosts_reaches_no_network() {
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    let workspace = tempfile::tempdir().unwrap();
    let skill = tempfile::tempdir().unwrap();
    let tool = tool(
        skill.path(),
        &format!(
            "import json, socket\ntry:\n    socket.create_connection(('127.0.0.1', {port}), timeout=3)\n    print(json.dumps({{'connected': True}}))\nexcept OSError as error:\n    print(json.dumps({{'connected': False}}))\n"
        ),
        json!({}),
    );
    let output = tool
        .execute(&json!({"timeout_ms": 20000}), &context(workspace.path()))
        .await;
    assert!(!output.is_error(), "{}", output.content);
    assert_eq!(
        serde_json::from_str::<Value>(&output.content).unwrap(),
        json!({"connected": false})
    );
    drop(listener);
}

#[cfg(any(target_os = "macos", target_os = "linux"))]
#[tokio::test]
async fn a_program_writes_its_drafts_and_never_the_workspace() {
    let workspace = tempfile::tempdir().unwrap();
    let skill = tempfile::tempdir().unwrap();
    let tool = tool(
        skill.path(),
        "import json, os\nopen(os.path.join(os.environ['VAK_DRAFTS'], 'split.csv'), 'w').write('a,30\\n')\ntry:\n    open('escaped.txt', 'w').write('no')\n    wrote = True\nexcept OSError:\n    wrote = False\nprint(json.dumps({'workspace_written': wrote}))\n",
        json!({"writes": "drafts"}),
    );
    let output = tool
        .execute(&json!({"timeout_ms": 20000}), &context(workspace.path()))
        .await;
    assert!(!output.is_error(), "{}", output.content);
    let result: Value = serde_json::from_str(&output.content).unwrap();
    assert_eq!(result["result"], json!({"workspace_written": false}));
    let drafts = result["drafts"].as_array().unwrap();
    assert_eq!(drafts.len(), 1, "{result}");
    assert!(
        drafts[0].as_str().unwrap().ends_with("/split.csv"),
        "{result}"
    );
    assert!(!workspace.path().join("escaped.txt").exists());
}

#[tokio::test]
async fn a_program_changed_after_the_turn_froze_it_is_not_run() {
    let workspace = tempfile::tempdir().unwrap();
    let skill = tempfile::tempdir().unwrap();
    let tool = tool(skill.path(), "print('{}')\n", json!({}));
    std::fs::write(skill.path().join("run.py"), "print('[]')\n").unwrap();
    let output = tool
        .execute(&json!({"timeout_ms": 20000}), &context(workspace.path()))
        .await;
    assert!(output.is_error());
    assert!(output.content.contains("next turn"), "{}", output.content);
}
