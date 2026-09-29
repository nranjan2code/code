//! `doc_read` parses Office packages only in the broker worker
//! (docs/design/72-openxml-documents.md, F3; invariant 14).

#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::path::PathBuf;

use serde_json::json;
use vak_tools::ToolContext;

fn doc_read(
    tools: Vec<std::sync::Arc<dyn vak_tools::Tool>>,
) -> std::sync::Arc<dyn vak_tools::Tool> {
    tools
        .into_iter()
        .find(|tool| tool.name() == "doc_read")
        .expect("doc_read is a built-in tool")
}

#[tokio::test]
async fn doc_read_reads_a_workbook_through_the_real_worker() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("budget.xlsx"), vak_ooxml::fixtures::xlsx()).unwrap();
    let worker = PathBuf::from(env!("CARGO_BIN_EXE_vak-tool-worker"));
    let tool = doc_read(vak_tools::brokered_default_tools(worker));
    let output = tool
        .execute(
            &json!({"path": "budget.xlsx", "section": "Budget"}),
            &ToolContext::new(dir.path().to_path_buf()),
        )
        .await;
    assert!(!output.is_error, "{}", output.content);
    assert!(
        output
            .content
            .contains("[Budget!B4:C4] B4: =SUM(B2:B3) [cached: 120] | C4: TRUE"),
        "{}",
        output.content
    );
}

#[tokio::test]
async fn core_never_runs_doc_read_in_its_own_process() {
    vak_config::paths::isolate_home_for_tests();
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("deck.pptx"), vak_ooxml::fixtures::pptx()).unwrap();
    let core = vak_core::Core::new_with_trust(dir.path().to_path_buf(), true).unwrap();
    core.set_tool_worker_exe(PathBuf::from("/nonexistent/vak-tool-worker"));
    let tools = core.agent_tools();
    assert_eq!(
        tools
            .iter()
            .filter(|tool| tool.name() == "doc_read")
            .count(),
        1,
        "exactly one doc_read, the brokered one"
    );
    let output = doc_read(tools)
        .execute(
            &json!({"path": "deck.pptx"}),
            &ToolContext::new(dir.path().to_path_buf()),
        )
        .await;
    assert!(output.is_error);
    assert!(
        output.content.contains("tool broker unavailable"),
        "without a worker the read must fail, not fall back in-process: {}",
        output.content
    );
    let read_only = core.agent_read_only_tools();
    assert!(read_only.iter().any(|tool| tool.name() == "doc_read"));
}

#[tokio::test]
async fn office_apply_edits_through_the_worker_as_the_calling_agent() {
    use sha2::Digest as _;
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("memo.docx");
    std::fs::write(&file, vak_ooxml::fixtures::docx()).unwrap();
    let digest: String = sha2::Sha256::digest(std::fs::read(&file).unwrap())
        .iter()
        .take(8)
        .map(|byte| format!("{byte:02x}"))
        .collect();
    let worker = PathBuf::from(env!("CARGO_BIN_EXE_vak-tool-worker"));
    let tool = vak_tools::brokered_default_tools(worker)
        .into_iter()
        .find(|tool| tool.name() == "office_apply")
        .expect("office_apply is a built-in tool");
    let output = tool
        .execute(
            &json!({
                "path": "memo.docx",
                "base_digest": digest,
                "ops": [{"op": "replace_paragraph_text", "anchor": "p@11", "text": "Up."}]
            }),
            &ToolContext::new(dir.path().to_path_buf()).with_agent_id("mira"),
        )
        .await;
    assert!(!output.is_error, "{}", output.content);
    assert!(
        output.content.contains("tracked changes by mira"),
        "the worker received the calling Agent's id: {}",
        output.content
    );
    assert_eq!(
        std::fs::read(&file).unwrap(),
        vak_ooxml::fixtures::docx(),
        "the workspace file waits for review"
    );
    let draft = output
        .content
        .split("written to ")
        .nth(1)
        .and_then(|rest| rest.split(". ").next())
        .unwrap();
    assert!(draft.starts_with(".vak/scratch/mira/"), "{draft}");
    let bytes = std::fs::read(dir.path().join(draft)).unwrap();
    let document =
        vak_ooxml::read::read(std::io::Cursor::new(bytes), vak_ooxml::Limits::default()).unwrap();
    assert!(
        document
            .lines()
            .join("\n")
            .contains("[deleted by mira: Steady][inserted by mira: Up].")
    );
}

/// Run the real local authoring and extraction tools for each supported
/// everyday format, then fetch the bytes through the server's raw download
/// route and prove the package is unchanged. No model or network is used.
#[tokio::test]
async fn real_office_drafts_are_downloadable_and_rag_readable() {
    use axum::body::Body;
    use axum::http::{Request, StatusCode};
    use sha2::Digest as _;
    use tower::ServiceExt;

    let dir = tempfile::tempdir().unwrap();
    let workspace = dir.path().join("workspace");
    let home = dir.path().join("home");
    std::fs::create_dir_all(&workspace).unwrap();
    std::fs::create_dir_all(workspace.join(".vak")).unwrap();
    std::fs::create_dir_all(&home).unwrap();
    std::fs::write(
        workspace.join(".vak/config.toml"),
        "[memory]\nreflection = false\n",
    )
    .unwrap();
    vak_config::paths::isolate_home_for_tests();
    let core = vak_core::Core::new_with_trust(workspace.clone(), true).unwrap();
    core.set_sessions_home(home);
    let worker = PathBuf::from(env!("CARGO_BIN_EXE_vak-tool-worker"));
    core.set_tool_worker_exe(worker.clone());
    let tools = vak_tools::brokered_default_tools(worker);
    let apply = tools
        .iter()
        .find(|tool| tool.name() == "office_apply")
        .unwrap()
        .clone();
    let read = doc_read(tools.clone());

    let cases = [
        (
            "daily.docx",
            json!([
                {"op":"add_paragraph","text":"Daily notes"},
                {"op":"add_table","rows":[["Item","Count"],["Apples",3]]},
                {"op":"add_image","image":{"mime_type":"image/png","data":"iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVR4nGNQaHjwHwAExAKAc00zmAAAAABJRU5ErkJggg==","alt_text":"A blue square used as the daily status marker"}}
            ]),
        ),
        (
            "daily.xlsx",
            json!([
                {"op":"set_cells","sheet":"Sheet1","cells":{"A1":"Day","B1":"Visitors","A2":"Mon","B2":25,"A3":"Tue","B3":31,"D1":"Day","E1":"Orders","D2":"Mon","E2":8,"D3":"Tue","E3":12}},
                {"op":"add_chart","sheet":"Sheet1","range":"A1:B3","chart_type":"bar","title":"Daily visitors"},
                {"op":"add_chart","sheet":"Sheet1","range":"D1:E3","chart_type":"bar","title":"Daily orders","cell":"K10"},
                {"op":"add_excel_table","sheet":"Sheet1","range":"A1:B3","name":"DailyVisitors"},
                {"op":"add_excel_table","sheet":"Sheet1","range":"D1:E3","name":"DailyOrders"},
                {"op":"add_excel_image","sheet":"Sheet1","cell":"D2","image":{"mime_type":"image/png","data":"iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVR4nGNQaHjwHwAExAKAc00zmAAAAABJRU5ErkJggg==","alt_text":"A blue square used as the daily status marker"}},
                {"op":"add_excel_image","sheet":"Sheet1","cell":"G2","image":{"mime_type":"image/png","data":"iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVR4nGNQaHjwHwAExAKAc00zmAAAAABJRU5ErkJggg==","alt_text":"A blue square used as the order status marker"}}
            ]),
        ),
        (
            "daily.pptx",
            json!([
                {"op":"add_slide_from_layout","layout":"Title and Content","placeholders":{"title":"Daily summary"},"tables":{"body":[["Metric","Value"],["Visitors",56]]},"notes":"Source data stays in the table."}
                ,{"op":"add_slide_from_layout","layout":"Title and Content","placeholders":{"title":"Daily averages"},"charts":{"body":{"title":"Visitors by day","chart_type":"bar","categories":["Monday","Tuesday","Wednesday"],"values":[12,18,15]}}}
                ,{"op":"add_slide_from_layout","layout":"Title and Content","placeholders":{"title":"Status marker"},"images":{"body":{"mime_type":"image/png","data":"iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVR4nGNQaHjwHwAExAKAc00zmAAAAABJRU5ErkJggg==","alt_text":"A blue square used as the daily status marker"}}}
            ]),
        ),
        (
            "daily.pdf",
            json!([
                {"op":"add_paragraph","text":"Daily summary"},
                {"op":"add_chart","title":"Daily visitors","categories":["Mon","Tue","Wed"],"values":[25,31,28]},
                {"op":"add_image","image":{"mime_type":"image/png","data":"iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVR4nGNQaHjwHwAExAKAc00zmAAAAABJRU5ErkJggg==","alt_text":"A blue square used as the daily status marker"}}
            ]),
        ),
    ];
    let (app, token) = vak_server::secured_router(core);
    for (index, (name, ops)) in cases.iter().enumerate() {
        let output = apply
            .execute(
                &json!({"path":name,"ops":ops}),
                &ToolContext::new(workspace.clone()).with_agent_id("vak"),
            )
            .await;
        assert!(!output.is_error, "{name}: {}", output.content);
        assert!(
            !workspace.join(name).exists(),
            "new Office content in {name} must remain a draft until Review accepts it"
        );
        let draft = output
            .content
            .split("written to ")
            .nth(1)
            .unwrap()
            .split(". ")
            .next()
            .unwrap();
        let draft_path = workspace.join(draft);
        let original_bytes = std::fs::read(&draft_path).unwrap();
        assert!(
            original_bytes.len() > 500,
            "{name} is a real generated file, not a label or placeholder"
        );

        let projected = vak_tools::broker::office_project(
            &PathBuf::from(env!("CARGO_BIN_EXE_vak-tool-worker")),
            &draft_path,
            vak_tools::broker::OfficeView::Content { from: 0 },
        )
        .await
        .unwrap();
        let extracted = read
            .execute(&json!({"path":draft}), &ToolContext::new(workspace.clone()))
            .await;
        assert!(
            !extracted.is_error,
            "doc_read {name}: {}",
            extracted.content
        );
        assert!(
            extracted.content.contains("sha256"),
            "doc_read returns digest citations for {name}"
        );
        match *name {
            "daily.docx" => {
                assert!(projected.to_string().contains("Apples"));
                assert!(extracted.content.contains("Daily notes"));
                assert!(
                    projected
                        .to_string()
                        .contains("A blue square used as the daily status marker")
                );
                assert!(
                    extracted
                        .content
                        .contains("A blue square used as the daily status marker")
                );
            }
            "daily.xlsx" => {
                assert!(projected.to_string().contains("DailyVisitors/r3"));
                assert!(projected.to_string().contains("Tue | 31"));
                assert!(projected.to_string().contains("DailyOrders/r3"));
                assert!(projected.to_string().contains("Tue | 12"));
                assert!(projected.to_string().contains("Daily orders"));
                assert!(projected.to_string().contains("Sheet1!K10"));
                assert!(
                    projected
                        .to_string()
                        .contains("A blue square used as the daily status marker")
                );
                assert!(
                    projected
                        .to_string()
                        .contains("A blue square used as the order status marker")
                );
                assert!(
                    extracted
                        .content
                        .contains("A blue square used as the daily status marker")
                );
                assert!(
                    extracted
                        .content
                        .contains("A blue square used as the order status marker")
                );
                let native_table = read
                    .execute(
                        &json!({"path":draft,"view":"table","section":"DailyVisitors"}),
                        &ToolContext::new(workspace.clone()),
                    )
                    .await;
                assert!(
                    !native_table.is_error,
                    "native Excel table extraction: {}",
                    native_table.content
                );
                assert!(
                    native_table.content.contains("Tuesday")
                        || native_table.content.contains("Tue")
                );
                let second_native_table = read
                    .execute(
                        &json!({"path":draft,"view":"table","section":"DailyOrders"}),
                        &ToolContext::new(workspace.clone()),
                    )
                    .await;
                assert!(
                    !second_native_table.is_error,
                    "second native table: {}",
                    second_native_table.content
                );
                assert!(
                    second_native_table.content.contains("Orders")
                        && second_native_table.content.contains("12")
                );
                let chart_table = read
                    .execute(
                        &json!({"path":draft,"view":"table","section":"Daily visitors"}),
                        &ToolContext::new(workspace.clone()),
                    )
                    .await;
                assert!(
                    !chart_table.is_error,
                    "chart table extraction: {}",
                    chart_table.content
                );
                assert!(chart_table.content.contains("chart data; cached values"));
                assert!(chart_table.content.contains("Mon") && chart_table.content.contains("25"));
                let second_chart_table = read
                    .execute(
                        &json!({"path":draft,"view":"table","section":"Daily orders"}),
                        &ToolContext::new(workspace.clone()),
                    )
                    .await;
                assert!(
                    !second_chart_table.is_error,
                    "second chart extraction: {}",
                    second_chart_table.content
                );
                assert!(
                    second_chart_table.content.contains("Tue")
                        && second_chart_table.content.contains("12")
                );
            }
            "daily.pptx" => {
                assert!(projected.to_string().contains("Visitors"));
                assert!(extracted.content.contains("Daily summary"));
                assert!(projected.to_string().contains("chart data; cached values"));
                assert!(projected.to_string().contains("Tuesday | 18"));
                assert!(
                    projected
                        .to_string()
                        .contains("A blue square used as the daily status marker")
                );
                assert!(
                    extracted
                        .content
                        .contains("A blue square used as the daily status marker")
                );
                let chart_table = read
                    .execute(
                        &json!({"path":draft,"view":"table","section":"Visitors by day"}),
                        &ToolContext::new(workspace.clone()),
                    )
                    .await;
                assert!(
                    !chart_table.is_error,
                    "PowerPoint chart table extraction: {}",
                    chart_table.content
                );
                assert!(
                    chart_table.content.contains("Tuesday") && chart_table.content.contains("18")
                );
            }
            _ => {
                assert!(projected.to_string().contains("Daily summary"));
                assert!(extracted.content.contains("Daily summary"));
                assert!(extracted.content.contains("Daily visitors"));
                assert!(extracted.content.contains("Tue"));
                assert!(extracted.content.contains("31"));
                assert!(
                    extracted
                        .content
                        .contains("A blue square used as the daily status marker")
                );
            }
        }

        // The browser's raw download endpoint reads workspace bytes without
        // decoding/re-encoding. Accept this local draft into a distinct path
        // for the accepted-file download route, then compare the response.
        let accepted_name = format!("download-{index}-{name}");
        std::fs::copy(&draft_path, workspace.join(&accepted_name)).unwrap();
        let response = app
            .clone()
            .oneshot(
                Request::builder()
                    .method("GET")
                    .uri(format!("/fs/file/raw?path={accepted_name}"))
                    .header("authorization", format!("Bearer {token}"))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK, "download {name}");
        let downloaded = axum::body::to_bytes(response.into_body(), 16 * 1024 * 1024)
            .await
            .unwrap();
        assert_eq!(
            sha2::Sha256::digest(&downloaded).as_slice(),
            sha2::Sha256::digest(&original_bytes).as_slice(),
            "download altered the {name} package"
        );
    }
}

/// A revision's task copy holds a new document under its own name; the
/// runtime tells the worker so, and its Word edits stay clean
/// (docs/design/72, R7). The list travels with the call, never from the model.
#[tokio::test]
async fn the_worker_writes_a_new_document_clean_when_the_runtime_says_so() {
    use sha2::Digest as _;
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("memo.docx");
    std::fs::write(&file, vak_ooxml::fixtures::docx()).unwrap();
    let digest: String = sha2::Sha256::digest(std::fs::read(&file).unwrap())
        .iter()
        .take(8)
        .map(|byte| format!("{byte:02x}"))
        .collect();
    let worker = PathBuf::from(env!("CARGO_BIN_EXE_vak-tool-worker"));
    let body = |tools: Vec<std::sync::Arc<dyn vak_tools::Tool>>| {
        let digest = digest.clone();
        let dir = dir.path().to_path_buf();
        async move {
            let tool = tools
                .into_iter()
                .find(|tool| tool.name() == "office_apply")
                .expect("office_apply is a built-in tool");
            let output = tool
                .execute(
                    &json!({
                        "path": "memo.docx",
                        "base_digest": digest,
                        "ops": [{"op": "replace_paragraph_text", "anchor": "p@11", "text": "Up."}]
                    }),
                    &ToolContext::new(dir.clone()).with_agent_id("mira"),
                )
                .await;
            assert!(!output.is_error, "{}", output.content);
            let draft = output
                .content
                .split("written to ")
                .nth(1)
                .and_then(|rest| rest.split(". ").next())
                .unwrap()
                .to_string();
            let mut package = vak_ooxml::Package::open(
                std::io::Cursor::new(std::fs::read(dir.join(draft)).unwrap()),
                vak_ooxml::Limits::default(),
            )
            .unwrap();
            String::from_utf8(package.read_part("word/document.xml").unwrap()).unwrap()
        }
    };
    let tracked = body(vak_tools::brokered_default_tools(worker.clone())).await;
    assert!(
        tracked.contains(r#"w:author="mira""#),
        "an existing file is tracked"
    );
    let clean = body(vak_tools::brokered_tools(
        worker,
        &["memo.docx".to_string()],
    ))
    .await;
    assert!(
        !clean.contains(r#"w:author="mira""#),
        "a file the runtime names as new is written clean"
    );
    assert!(clean.contains(">Up</w:t>"), "{clean}");
}
