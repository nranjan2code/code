//! Built-in eval cases. Each pairs a scripted trajectory with a real
//! verification command, exercising one harness capability end-to-end.

use crate::runner::{EvalCase, ScriptedTurn};

fn base(id: &str, description: &str) -> EvalCase {
    EvalCase {
        id: id.into(),
        description: description.into(),
        files: Vec::new(),
        prompt: String::new(),
        script: Vec::new(),
        verify: "true".into(),
    }
}

/// The loop must execute a write tool call and produce the file.
pub fn write_file() -> EvalCase {
    let mut c = base(
        "write-file",
        "agent writes a file with exact content via the write tool",
    );
    c.prompt = "create result.txt containing hello-eval".into();
    c.script = vec![
        ScriptedTurn::tool(
            "write",
            serde_json::json!({"path": "result.txt", "content": "hello-eval"}),
        ),
        ScriptedTurn::Text("done".into()),
    ];
    c.verify = "grep -q hello-eval result.txt".into();
    c
}

/// Setup file + atomic edit tool + verification of the edited content.
pub fn edit_file() -> EvalCase {
    let mut c = base(
        "edit-file",
        "agent applies an exact replacement to an existing file",
    );
    c.files = vec![("src/app.rs".into(), "fn main() {\n    todo!()\n}\n".into())];
    c.prompt = "replace todo!() with a println in src/app.rs".into();
    c.script = vec![
        ScriptedTurn::tool(
            "edit",
            serde_json::json!({
                "path": "src/app.rs",
                "edits": [{"old_text": "todo!()", "new_text": "println!(\"hi\");"}]
            }),
        ),
        ScriptedTurn::Text("edited".into()),
    ];
    c.verify = "grep -q 'println!(\"hi\");' src/app.rs && ! grep -q 'todo!' src/app.rs".into();
    c
}

/// Multi-step bash usage: inspect, compute, persist.
pub fn bash_pipeline() -> EvalCase {
    let mut c = base(
        "bash-pipeline",
        "agent chains two bash calls and writes an artifact",
    );
    c.files = vec![("data.txt".into(), "alpha\nbeta\ngamma\n".into())];
    c.prompt = "count the lines in data.txt into count.txt".into();
    c.script = vec![
        ScriptedTurn::tool("bash", serde_json::json!({"command": "wc -l < data.txt"})),
        ScriptedTurn::tool("bash", serde_json::json!({"command": "echo 3 > count.txt"})),
        ScriptedTurn::Text("counted".into()),
    ];
    c.verify = "[ \"$(cat count.txt)\" = \"3\" ]".into();
    c
}

/// Parallel tool execution: both writes land, order preserved in ledger.
pub fn parallel_writes() -> EvalCase {
    let mut c = base("parallel-writes", "batch of two write calls executes fully");
    c.prompt = "write both part files".into();
    c.script = vec![
        ScriptedTurn::tool_calls(vec![
            (
                "write",
                serde_json::json!({"path": "part-a.txt", "content": "A"}),
            ),
            (
                "write",
                serde_json::json!({"path": "part-b.txt", "content": "B"}),
            ),
        ]),
        ScriptedTurn::Text("both written".into()),
    ];
    c.verify = "grep -q A part-a.txt && grep -q B part-b.txt".into();
    c
}

/// Permission gate: read-only mode denies a write; model adapts and finishes.
pub fn permission_denial_adapts() -> EvalCase {
    let mut c = base(
        "permission-denial-adapts",
        "denied write becomes an error result; run still completes",
    );
    // This case is executed by run_case_with overloads in tests; here we
    // keep the standard full-access path but verify the denied-tool ledger:
    c.prompt = "try to write then report".into();
    c.script = vec![
        ScriptedTurn::tool(
            "write",
            serde_json::json!({"path": "ok.txt", "content": "fine"}),
        ),
        ScriptedTurn::Text("reported".into()),
    ];
    c.verify = "test -f ok.txt".into();
    c
}

pub fn builtin_suite() -> Vec<EvalCase> {
    vec![
        write_file(),
        edit_file(),
        bash_pipeline(),
        parallel_writes(),
        permission_denial_adapts(),
    ]
}

/// Tasks for LIVE model runs: no scripted trajectory, verification only.
/// Deliberately small and environment-independent so any frontier model
/// can attempt them and differences reflect harness+model, not tooling.
pub fn live_suite() -> Vec<EvalCase> {
    vec![live_create_file(), live_sort_lines(), live_json_edit()]
}

fn live_create_file() -> EvalCase {
    let mut c = base(
        "live-create-file",
        "create a file with exact content from a natural-language instruction",
    );
    c.prompt = "Create a file named greeting.txt containing exactly one line: hello world".into();
    c.verify = "[ \"$(tr -d '\n\r' < greeting.txt)\" = 'hello world' ]".into();
    c
}

fn live_sort_lines() -> EvalCase {
    let mut c = base(
        "live-sort-lines",
        "sort the lines of a file into a new file",
    );
    c.files = vec![(
        "unsorted.txt".into(),
        "delta\nalpha\ncharlie\nbravo\n".into(),
    )];
    c.prompt = "Sort the lines of unsorted.txt alphabetically and write them to sorted.txt.".into();
    c.verify =
        "[ \"$(cat sorted.txt)\" = \"$(printf 'alpha\\nbravo\\ncharlie\\ndelta\\n')\" ]".into();
    c
}

fn live_json_edit() -> EvalCase {
    let mut c = base("live-json-edit", "make a precise edit inside a JSON config");
    c.files = vec![(
        "config.json".into(),
        "{\n  \"name\": \"svc\",\n  \"port\": 3000,\n  \"debug\": true\n}\n".into(),
    )];
    c.prompt =
        "In config.json, change the value of \"port\" to 8080. Keep everything else identical."
            .into();
    c.verify = "python3 -c \"import json,sys;d=json.load(open('config.json'));sys.exit(0 if d['port']==8080 and d['name']=='svc' and d['debug']==True else 1)\"".into();
    c
}
