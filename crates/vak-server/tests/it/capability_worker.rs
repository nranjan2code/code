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

/// An echo server on loopback, and the port it listens on.
async fn echo() -> u16 {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    tokio::spawn(async move {
        while let Ok((mut stream, _)) = listener.accept().await {
            tokio::spawn(async move {
                let (mut read, mut write) = stream.split();
                let _ = tokio::io::copy(&mut read, &mut write).await;
            });
        }
    });
    port
}

/// A program that asks its proxy for `target` and reports what came back,
/// and whether it could also connect to `direct` without the proxy.
fn through_proxy(target: &str, direct: u16) -> String {
    format!(
        r#"import json, os, socket
proxy = os.environ.get('HTTPS_PROXY', '')
answer = {{'proxy': bool(proxy)}}
if proxy:
    host, port = proxy.rsplit('/', 1)[-1].split(':')
    s = socket.create_connection((host, int(port)), timeout=5)
    s.sendall(b'CONNECT {target} HTTP/1.1\r\nHost: {target}\r\n\r\n')
    head = s.recv(1024).decode()
    answer['status'] = head.split(' ')[1] if ' ' in head else head
    if answer['status'] == '200':
        s.sendall(b'ping')
        answer['echo'] = s.recv(16).decode()
try:
    socket.create_connection(('127.0.0.1', {direct}), timeout=3)
    answer['direct'] = True
except OSError:
    answer['direct'] = False
print(json.dumps(answer))
"#
    )
}

#[cfg(any(target_os = "macos", target_os = "linux"))]
#[tokio::test]
async fn a_declared_host_is_reached_through_the_proxy_and_nothing_else() {
    let allowed = echo().await;
    let other = echo().await;
    let workspace = tempfile::tempdir().unwrap();
    for (target, status, echoed) in [
        (format!("127.0.0.1:{allowed}"), "200", Some("ping")),
        (format!("127.0.0.1:{other}"), "403", None),
    ] {
        let skill = tempfile::tempdir().unwrap();
        let tool = tool(
            skill.path(),
            &through_proxy(&target, allowed),
            json!({"hosts": [format!("127.0.0.1:{allowed}")]}),
        );
        assert!(tool.leaves_machine());
        let output = tool
            .execute(&json!({"timeout_ms": 20000}), &context(workspace.path()))
            .await;
        assert!(!output.is_error(), "{}", output.content);
        let answer: Value = serde_json::from_str(&output.content).unwrap();
        assert_eq!(answer["proxy"], json!(true), "{answer}");
        assert_eq!(answer["status"], json!(status), "{target}: {answer}");
        assert_eq!(answer["echo"].as_str(), echoed, "{answer}");
        assert_eq!(
            answer["direct"],
            json!(false),
            "only through the proxy: {answer}"
        );
    }
}

#[cfg(any(target_os = "macos", target_os = "linux"))]
#[tokio::test]
async fn requirements_are_installed_once_into_their_own_environment() {
    let workspace = tempfile::tempdir().unwrap();
    let skill = tempfile::tempdir().unwrap();
    std::fs::write(skill.path().join("requirements.txt"), "").unwrap();
    let first = tool(
        skill.path(),
        "import json, sys\nprint(json.dumps({'prefix': sys.prefix}))\n",
        json!({"requirements": {"file": "requirements.txt", "hosts": ["127.0.0.1:9"]}}),
    );
    let env = Program::load(skill.path())
        .unwrap()
        .unwrap()
        .requirements
        .unwrap()
        .env;
    let _ = std::fs::remove_dir_all(&env);
    assert!(first.leaves_machine(), "the first call installs");
    let output = first
        .execute(&json!({"timeout_ms": 120000}), &context(workspace.path()))
        .await;
    assert!(!output.is_error(), "{}", output.content);
    let answer: Value = serde_json::from_str(&output.content).unwrap();
    let prefix = PathBuf::from(answer["prefix"].as_str().unwrap());
    assert_eq!(
        prefix.canonicalize().unwrap(),
        env.canonicalize().unwrap(),
        "{answer}"
    );
    let staging: Vec<_> = std::fs::read_dir(env.parent().unwrap())
        .unwrap()
        .flatten()
        .filter(|entry| entry.file_name().to_string_lossy().ends_with(".staging"))
        .collect();
    assert!(staging.is_empty(), "no staging left behind");
    let again = tool(
        skill.path(),
        "import json, sys\nprint(json.dumps({'prefix': sys.prefix}))\n",
        json!({"requirements": {"file": "requirements.txt", "hosts": ["127.0.0.1:9"]}}),
    );
    assert!(
        !again.leaves_machine(),
        "installed: later calls stay on the machine"
    );
}
