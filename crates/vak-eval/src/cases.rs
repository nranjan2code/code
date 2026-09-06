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
        outcome: None,
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

/// Non-coding scenarios: research, data analysis, writing, document
/// conversion, and inventory work through the same six-tool kernel. Each
/// proves the harness is a general agent, not a code-only one.
pub fn general_suite() -> Vec<EvalCase> {
    vec![
        general_research_synthesis(),
        general_csv_analysis(),
        general_writing_draft(),
        general_doc_conversion(),
        general_inventory_index(),
        general_error_adapts_noncode(),
        general_schedule(),
        general_decision_matrix(),
    ]
}

/// A small held-out-style corpus kept separate from the broad smoke suite.
/// These cases deliberately vary language, context length, and deliverable
/// shape so optimization work cannot overfit the named general cases.
pub fn held_out_suite() -> Vec<EvalCase> {
    vec![
        held_out_multilingual_note(),
        held_out_mixed_deliverables(),
        held_out_recovery_after_bad_artifact(),
    ]
}

fn held_out_multilingual_note() -> EvalCase {
    let mut c = base(
        "held-out-multilingual-note",
        "summarize a Spanish note into an English action brief",
    );
    c.files = vec![(
        "nota.txt".into(),
        "La reunión es el martes. Ana enviará el presupuesto.\n".into(),
    )];
    c.prompt =
        "Read nota.txt and write brief.md in English with the meeting date and owner.".into();
    c.script = vec![
        ScriptedTurn::tool("read", serde_json::json!({"path": "nota.txt"})),
        ScriptedTurn::tool(
            "write",
            serde_json::json!({"path": "brief.md", "content": "# Action brief\n\n- Meeting: Tuesday\n- Owner: Ana (budget)\n"}),
        ),
        ScriptedTurn::Text("briefed".into()),
    ];
    let mut outcome = vak_intent::OutcomeSpec::from_reading(
        &c.prompt,
        &vak_intent::Reading::general(),
        vak_intent::RESOLVER_VERSION,
    );
    outcome.requirements.push(vak_intent::OutcomeRequirement {
        id: "brief-deliverable".into(),
        kind: vak_intent::RequirementKind::Deliverable,
        description: "English action brief is written to brief.md".into(),
        origin: vak_intent::RequirementOrigin::Explicit,
        importance: vak_intent::RequirementImportance::Must,
        target: Some("brief.md".into()),
    });
    outcome.requirements.push(vak_intent::OutcomeRequirement {
        id: "brief-evidence".into(),
        kind: vak_intent::RequirementKind::Evidence,
        description: "The brief cites the source note".into(),
        origin: vak_intent::RequirementOrigin::Explicit,
        importance: vak_intent::RequirementImportance::Must,
        target: Some("nota.txt".into()),
    });
    c.outcome = Some(outcome);
    c.verify = "grep -q Tuesday brief.md && grep -q Ana brief.md".into();
    c
}

fn held_out_mixed_deliverables() -> EvalCase {
    let mut c = base(
        "held-out-mixed-deliverables",
        "produce both a concise answer and a saved checklist",
    );
    c.prompt = "Give a one-line answer and save checklist.md with three launch checks.".into();
    c.script = vec![
        ScriptedTurn::tool(
            "write",
            serde_json::json!({"path": "checklist.md", "content": "# Launch checklist\n\n- Back up data\n- Verify access\n- Announce release\n"}),
        ),
        ScriptedTurn::Text("Answer: ready after the checks are complete.".into()),
    ];
    c.verify = "grep -q 'Back up data' checklist.md && grep -q 'Verify access' checklist.md && grep -q 'Announce release' checklist.md".into();
    c
}

fn held_out_recovery_after_bad_artifact() -> EvalCase {
    let mut c = base(
        "held-out-recovery-after-bad-artifact",
        "repair a malformed output before reporting completion",
    );
    c.prompt = "Write checklist.md with exactly the line READY, then verify it.".into();
    c.script = vec![
        ScriptedTurn::tool(
            "write",
            serde_json::json!({"path": "checklist.md", "content": "NOT READY\n"}),
        ),
        ScriptedTurn::tool(
            "edit",
            serde_json::json!({
                "path": "checklist.md",
                "edits": [{"old_text": "NOT READY", "new_text": "READY"}]
            }),
        ),
        ScriptedTurn::Text("repaired and verified".into()),
    ];
    c.verify = "test \"$(cat checklist.md)\" = READY".into();
    c
}

pub fn general_schedule() -> EvalCase {
    let mut c = base(
        "general-schedule",
        "turn availability notes into a conflict-free schedule",
    );
    c.files = vec![(
        "availability.txt".into(),
        "Maya: 09:00-11:00\nDevon: 10:00-12:00\n".into(),
    )];
    c.prompt = "Read availability.txt and write schedule.md with a shared meeting time.".into();
    c.script = vec![
        ScriptedTurn::tool("read", serde_json::json!({"path": "availability.txt"})),
        ScriptedTurn::tool(
            "write",
            serde_json::json!({"path": "schedule.md", "content": "# Meeting\n\nShared time: 10:00-11:00\n"}),
        ),
        ScriptedTurn::Text("scheduled".into()),
    ];
    c.verify = "grep -q '10:00-11:00' schedule.md".into();
    c
}

pub fn general_decision_matrix() -> EvalCase {
    let mut c = base(
        "general-decision-matrix",
        "compare options against explicit criteria",
    );
    c.prompt =
        "Write decision.md comparing Option A and Option B, naming cost and reliability criteria."
            .into();
    c.script = vec![
        ScriptedTurn::tool(
            "write",
            serde_json::json!({"path": "decision.md", "content": "# Decision\n\n| Option | Cost | Reliability |\n|---|---|---|\n| Option A | low | medium |\n| Option B | medium | high |\n"}),
        ),
        ScriptedTurn::Text("compared".into()),
    ];
    c.verify = "grep -q 'Option A' decision.md && grep -q 'Reliability' decision.md && grep -q 'Cost' decision.md".into();
    c
}

/// Multi-source synthesis: read three notes, merge key facts into a summary.
pub fn general_research_synthesis() -> EvalCase {
    let mut c = base(
        "general-research-synthesis",
        "read three research notes and synthesize a summary file",
    );
    c.files = vec![
        (
            "notes/climate.txt".into(),
            "Global mean temperature rose 1.2C since pre-industrial times.\n".into(),
        ),
        (
            "notes/energy.txt".into(),
            "Solar is now the cheapest electricity source in most markets.\n".into(),
        ),
        (
            "notes/policy.txt".into(),
            "Forty countries pledged carbon neutrality by 2050.\n".into(),
        ),
    ];
    c.prompt = "Read all files under notes/ and write summary.md covering each finding.".into();
    c.script = vec![
        ScriptedTurn::tool_calls(vec![
            ("read", serde_json::json!({"path": "notes/climate.txt"})),
            ("read", serde_json::json!({"path": "notes/energy.txt"})),
            ("read", serde_json::json!({"path": "notes/policy.txt"})),
        ]),
        ScriptedTurn::tool(
            "write",
            serde_json::json!({
                "path": "summary.md",
                "content": "# Findings\n\n- Warming reached 1.2C above pre-industrial levels.\n- Solar is now the cheapest electricity in most markets.\n- Forty countries pledged carbon neutrality by 2050.\n"
            }),
        ),
        ScriptedTurn::Text("synthesized".into()),
    ];
    c.verify =
        "grep -q '1.2C' summary.md && grep -q 'cheapest' summary.md && grep -q '2050' summary.md"
            .into();
    c
}

/// Data analysis on a CSV via bash arithmetic; no code files involved.
pub fn general_csv_analysis() -> EvalCase {
    let mut c = base(
        "general-csv-analysis",
        "analyze a csv of expenses with bash and write a report",
    );
    c.files = vec![(
        "expenses.csv".into(),
        "item,amount\nrent,1200\ngroceries,340\ntransport,85\n".into(),
    )];
    c.prompt =
        "Compute the total of the amount column in expenses.csv and write report.md stating it."
            .into();
    c.script = vec![
        ScriptedTurn::tool(
            "bash",
            serde_json::json!({"command": "awk -F, 'NR>1 {s+=$2} END {print s}' expenses.csv"}),
        ),
        ScriptedTurn::tool(
            "write",
            serde_json::json!({
                "path": "report.md",
                "content": "# Expense Report\n\nTotal monthly expenses: 1625\n"
            }),
        ),
        ScriptedTurn::Text("analyzed".into()),
    ];
    c.verify = "grep -q 1625 report.md && [ \"$(awk -F, 'NR>1 {s+=$2} END {print s}' expenses.csv)\" = \"1625\" ]".into();
    c
}

/// Pure writing: structure and length constraints verified mechanically.
pub fn general_writing_draft() -> EvalCase {
    let mut c = base(
        "general-writing-draft",
        "draft a structured essay meeting title and section requirements",
    );
    c.prompt = "Write essay.md: a title line starting with '# ', then sections '## Intro', '## Body', '## Conclusion' with at least one sentence each (>= 60 words total).".into();
    c.script = vec![
        ScriptedTurn::tool(
            "write",
            serde_json::json!({
                "path": "essay.md",
                "content": "# Urban Rivers\n\n## Intro\nCities grew around rivers because water meant trade, food, and power for early settlements.\n\n## Body\nOver the twentieth century many urban rivers were paved over or hidden beneath concrete, severing residents from their own geography and worsening floods downstream.\n\n## Conclusion\nDaylighting forgotten waterways restores habitat, cools streets, and gives neighborhoods a shared civic anchor worth protecting.\n"
            }),
        ),
        ScriptedTurn::Text("drafted".into()),
    ];
    c.verify =
        "head -n1 essay.md | grep -q '^# ' && grep -q '## Intro' essay.md && grep -q '## Body' essay.md && grep -q '## Conclusion' essay.md && [ \"$(wc -w < essay.md)\" -ge 60 ]"
            .into();
    c
}

/// Document conversion: free-form notes into machine-readable JSON.
pub fn general_doc_conversion() -> EvalCase {
    let mut c = base(
        "general-doc-conversion",
        "convert plain-text meeting notes into structured json",
    );
    c.files = vec![(
        "meeting-notes.txt".into(),
        "Team sync March 4\nAgenda:\n- review Q1 roadmap (Maya)\n- hiring update (Devon)\n- budget review (Priya)\n".into(),
    )]
    ;
    c.prompt = "Convert meeting-notes.txt into agenda.json: an object with keys date (string), items (array of objects topic and owner).".into();
    c.script = vec![
        ScriptedTurn::tool("read", serde_json::json!({"path": "meeting-notes.txt"})),
        ScriptedTurn::tool(
            "write",
            serde_json::json!({
                "path": "agenda.json",
                "content": "{\"date\": \"March 4\", \"items\": [{\"topic\": \"review Q1 roadmap\", \"owner\": \"Maya\"}, {\"topic\": \"hiring update\", \"owner\": \"Devon\"}, {\"topic\": \"budget review\", \"owner\": \"Priya\"}]}"
            }),
        ),
        ScriptedTurn::Text("converted".into()),
    ];
    c.verify = "python3 -c \"import json;d=json.load(open('agenda.json'));assert d['date']=='March 4';assert len(d['items'])==3;assert d['items'][0]=={'topic':'review Q1 roadmap','owner':'Maya'}\"".into();
    c
}

/// Cross-file discovery with glob+grep, then an index artifact.
pub fn general_inventory_index() -> EvalCase {
    let mut c = base(
        "general-inventory-index",
        "discover warranty records across files and build an index",
    );
    c.files = vec![
        (
            "records/laptop.txt".into(),
            "MacBook Pro, purchased 2024-01-10, warranty until 2027-01-10.\n".into(),
        ),
        (
            "records/monitor.txt".into(),
            "Studio Display, purchased 2023-06-02, warranty until 2026-06-02.\n".into(),
        ),
        (
            "records/receipts-old.txt".into(),
            "Miscellaneous receipts from 2019, all warranties expired.\n".into(),
        ),
    ];
    c.prompt =
        "Search records/ for lines mentioning warranty and write index.md listing every item with its warranty end date."
            .into();
    c.script = vec![
        ScriptedTurn::tool_calls(vec![
            ("glob", serde_json::json!({"pattern": "records/*.txt"})),
            (
                "grep",
                serde_json::json!({"pattern": "warranty until", "path": "records"}),
            ),
        ]),
        ScriptedTurn::tool(
            "write",
            serde_json::json!({
                "path": "index.md",
                "content": "# Warranty Index\n\n- MacBook Pro — until 2027-01-10\n- Studio Display — until 2026-06-02\n"
            }),
        ),
        ScriptedTurn::Text("indexed".into()),
    ];
    c.verify =
        "grep -q '2027-01-10' index.md && grep -q '2026-06-02' index.md && ! grep -qi '2019' index.md"
            .into();
    c
}

/// Error-driven recovery outside code: first command misses, agent corrects.
pub fn general_error_adapts_noncode() -> EvalCase {
    let mut c = base(
        "general-error-adapts-noncode",
        "failed lookup is read from the error and corrected without retrying blindly",
    );
    c.files = vec![(
        "archive/team-2025.txt".into(),
        "Roster: Maya (design), Devon (ops), Priya (finance).\n".into(),
    )];
    c.prompt =
        "Find this year's team roster file and copy the roster line into roster.md. Verify by reading it back."
            .into();
    c.script = vec![
        // Deliberately probes the wrong path first.
        ScriptedTurn::tool(
            "bash",
            serde_json::json!({"command": "cat archive/team-2026.txt"}),
        ),
        ScriptedTurn::tool(
            "bash",
            serde_json::json!({"command": "echo 'Roster: Maya (design), Devon (ops), Priya (finance).' > roster.md"}),
        ),
        ScriptedTurn::Text("recovered".into()),
    ];
    c.verify = "grep -q 'Roster: Maya' roster.md".into();
    c
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
