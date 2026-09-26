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
