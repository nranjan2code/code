//! Exit test of M3b slice 3: a real turn leaves nothing undeclared in any
//! root Vak owns. Its own test binary, so no other test writes into the
//! isolated home it inspects.

#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::path::{Path, PathBuf};
use vak_core::state::{self, Root};

fn files_under(root: &Path) -> Vec<PathBuf> {
    let mut found = Vec::new();
    let mut stack = vec![root.to_path_buf()];
    while let Some(dir) = stack.pop() {
        for entry in std::fs::read_dir(&dir).into_iter().flatten().flatten() {
            let path = entry.path();
            if path.is_dir() {
                stack.push(path);
            } else if let Ok(rel) = path.strip_prefix(root) {
                found.push(rel.to_path_buf());
            }
        }
    }
    found
}

fn is_incidental(relative: &Path) -> bool {
    relative
        .components()
        .any(|c| matches!(c.as_os_str().to_str(), Some(".DS_Store") | Some(".git")))
}

/// A provider that reads the file a turn names, then answers.
struct ReadThenAnswer;

#[async_trait::async_trait]
impl vak_llm::Provider for ReadThenAnswer {
    fn name(&self) -> &str {
        "scripted"
    }

    async fn stream(
        &self,
        request: vak_llm::types::ChatRequest,
        _cancel: tokio_util::sync::CancellationToken,
    ) -> Result<vak_llm::EventStream, vak_llm::LlmError> {
        use vak_llm::types::{AssistantMessage, ContentBlock, StopReason, Usage};
        let answered = request.messages.last().is_some_and(|message| {
            message
                .content
                .iter()
                .any(|block| matches!(block, ContentBlock::ToolResult { .. }))
        });
        let (content, stop_reason) = if answered {
            (
                vec![ContentBlock::text("It is a long list.")],
                StopReason::EndTurn,
            )
        } else {
            (
                vec![ContentBlock::ToolUse {
                    id: "read-1".into(),
                    name: "read".into(),
                    input: serde_json::json!({ "path": "big.txt" }),
                }],
                StopReason::ToolUse,
            )
        };
        let message = AssistantMessage {
            content,
            stop_reason,
            usage: Usage::default(),
            model: "test-model".into(),
            response_id: None,
        };
        let (mut sink, rx) = vak_llm::stream::channel(64);
        sink.push(vak_llm::stream::StreamEvent::Start {
            partial: message.clone(),
        });
        sink.close_message(message).await;
        Ok(rx)
    }
}

/// Exit test of M3b slice 3: a real turn (a ledger, a windowed tool result
/// stored as an object, a checkpoint, the side ledgers, the tenant keys)
/// leaves nothing undeclared in any root Vak owns.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn no_undeclared_paths_any_root() {
    vak_config::paths::isolate_home_for_tests();
    let workspace = tempfile::tempdir().expect("workspace");
    std::fs::create_dir_all(workspace.path().join(".vak")).unwrap();
    std::fs::write(
        workspace.path().join(".vak/config.toml"),
        "[memory]\nreflection = false\n\n[stop_policy]\nenabled = false\n",
    )
    .unwrap();
    let big: String = (0..4000).map(|n| format!("item {n}\n")).collect();
    std::fs::write(workspace.path().join("big.txt"), big).unwrap();
    let core = vak_core::Core::new_with_trust(workspace.path().to_path_buf(), true).expect("core");
    core.set_provider_instance(std::sync::Arc::new(ReadThenAnswer));
    let session = core.start_session().await.expect("session");
    let (tx, mut rx) = tokio::sync::mpsc::channel(256);
    tokio::spawn(async move { while rx.recv().await.is_some() {} });
    core.run_turn_with(
        session,
        "what is in big.txt?",
        tokio_util::sync::CancellationToken::new(),
        None,
        None,
        None,
        tx,
    )
    .await
    .expect("turn");

    let roots = [
        (Root::Data, vak_config::paths::data_home()),
        (Root::Cache, vak_config::paths::cache_home()),
        (Root::Logs, vak_config::paths::logs_dir()),
        (Root::Runtime, vak_config::paths::runtime_dir()),
    ];
    let mut undeclared = Vec::new();
    for (root, path) in &roots {
        // An overridden home nests the other roots inside the data home;
        // each file is judged by the root it belongs to.
        let nested = |file: &Path| {
            roots
                .iter()
                .any(|(other, dir)| other != root && dir.starts_with(path) && file.starts_with(dir))
        };
        for rel in files_under(path) {
            if !is_incidental(&rel) && !nested(&path.join(&rel)) && !state::is_declared(*root, &rel)
            {
                undeclared.push(format!("{root:?}: {}", rel.display()));
            }
        }
    }
    assert!(
        vak_config::paths::tenant_home_at(
            &vak_config::paths::data_home(),
            &vak_session::trace::local::tenant().to_string()
        )
        .join("store")
        .exists(),
        "the turn stored its windowed result as an object"
    );
    assert!(
        undeclared.is_empty(),
        "undeclared durable files: {undeclared:#?}"
    );
}
