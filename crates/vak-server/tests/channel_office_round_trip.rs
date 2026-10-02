#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

//! A file sent on a channel comes back edited (docs/design/72, P5): the
//! Agent drafts a change to the inbox copy with `office_apply`, and the
//! waiting reply carries the draft, under the name it was sent with and
//! with a caption saying what changed, to a bridge that accepts files. A
//! draft with a sensitivity label is held, and a bridge that takes no
//! files is told where the draft is.

mod support;

use std::collections::VecDeque;
use std::sync::{Arc, Mutex};

use base64::Engine as _;
use sha2::Digest as _;
use tokio_util::sync::CancellationToken;

use vak_core::Core;
use vak_llm::stream;
use vak_llm::types::{AssistantMessage, ChatRequest, ContentBlock, StopReason, Usage};
use vak_llm::{EventStream, LlmError, Provider};

struct Scripted(
    Mutex<VecDeque<AssistantMessage>>,
    Mutex<Vec<ChatRequest>>,
    crate::support::CapacityKey,
);

#[async_trait::async_trait]
impl Provider for Scripted {
    fn name(&self) -> &str {
        "scripted"
    }

    fn rate_limit_key(&self) -> String {
        self.2.0.clone()
    }

    async fn stream(
        &self,
        request: ChatRequest,
        _cancel: CancellationToken,
    ) -> Result<EventStream, LlmError> {
        self.1.lock().unwrap().push(request);
        // After the script, the last answer again: a side request (a
        // completion check) must not stall the turn on retries.
        let next = {
            let mut script = self.0.lock().unwrap();
            if script.len() > 1 {
                script.pop_front()
            } else {
                script.front().cloned()
            }
        };
        let (mut sink, rx) = stream::channel(16);
        match next {
            Some(message) => {
                sink.push(stream::StreamEvent::Start {
                    partial: message.clone(),
                });
                sink.close_message(message).await;
            }
            None => sink.close_error(LlmError::Parse("exhausted".into())).await,
        }
        Ok(rx)
    }
}

fn message(content: ContentBlock, stop_reason: StopReason) -> AssistantMessage {
    AssistantMessage {
        content: vec![content],
        stop_reason,
        usage: Usage::default(),
        model: "test-model".into(),
        response_id: None,
    }
}

/// Where the gateway saves `bytes` sent as `name`, and the file's sha256.
fn inbox_path(name: &str, bytes: &[u8]) -> (String, String) {
    let digest = sha2::Sha256::digest(bytes);
    let hex: String = digest.iter().map(|byte| format!("{byte:02x}")).collect();
    (format!("inbox/{}-{name}", &hex[..12]), hex)
}

/// Sends `bytes` as `name` on a channel whose model edits it with `ops`,
/// and returns the waiting reply.
async fn round_trip(
    name: &str,
    bytes: &[u8],
    ops: serde_json::Value,
    accepts_files: bool,
) -> serde_json::Value {
    let (path, digest) = inbox_path(name, bytes);
    let provider = Arc::new(Scripted(
        Mutex::new(VecDeque::from([
            message(
                ContentBlock::ToolUse {
                    id: "call_edit".into(),
                    name: "office_apply".into(),
                    input: serde_json::json!({"path": path, "base_digest": digest, "ops": ops}),
                },
                StopReason::ToolUse,
            ),
            message(ContentBlock::text("Updated it."), StopReason::EndTurn),
        ])),
        Mutex::new(Vec::new()),
        crate::support::CapacityKey::default(),
    ));
    let dir = tempfile::tempdir().unwrap();
    let cwd = dir.path().to_path_buf();
    std::fs::create_dir_all(cwd.join(".vak")).unwrap();
    std::fs::write(
        cwd.join(".vak/config.toml"),
        "[memory]\nreflection = false\n[gateway]\nchat_allowlist_open = true\n",
    )
    .unwrap();
    vak_config::paths::isolate_home_for_tests();
    let core = Core::new_with_trust(cwd.clone(), true).unwrap();
    core.set_sessions_home(dir.path().join("home"));
    core.set_permission_mode(vak_config::PermissionMode::WorkspaceWrite);
    core.set_tool_worker_exe(std::path::PathBuf::from(env!(
        "CARGO_BIN_EXE_vak-tool-worker"
    )));
    core.set_provider_instance(provider);
    std::mem::forget(dir);

    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        axum::serve(listener, vak_server::gateway_router(core))
            .await
            .unwrap();
    });
    let mut request = serde_json::json!({
        "surface": "telegram",
        "chat": "42",
        "sender": "7",
        "text": "update the file",
        "wait": true,
        "attachments": [{
            "mime": "application/octet-stream",
            "data": base64::engine::general_purpose::STANDARD.encode(bytes),
            "filename": name,
            "kind": "document",
        }],
    });
    if accepts_files {
        request["capabilities"] = serde_json::json!({"accepts_files": true});
    }
    let response = reqwest::Client::new()
        .post(format!("http://{addr}/gateway/inbound"))
        .json(&request)
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), 200);
    response.json().await.unwrap()
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn an_edited_workbook_comes_back_to_the_chat_it_came_from() {
    let reply = round_trip(
        "budget.xlsx",
        &vak_ooxml::fixtures::xlsx(),
        serde_json::json!([{"op": "set_cells", "sheet": "Budget", "cells": {"B2": 150}}]),
        true,
    )
    .await;
    let files = reply["files"].as_array().unwrap();
    assert_eq!(files.len(), 1, "{reply}");
    assert_eq!(files[0]["name"], "budget.xlsx");
    assert!(
        files[0]["caption"]
            .as_str()
            .unwrap()
            .contains("Budget: 1 changed"),
        "{reply}"
    );
    let bytes = base64::engine::general_purpose::STANDARD
        .decode(files[0]["data"].as_str().unwrap())
        .unwrap();
    let document =
        vak_ooxml::read::read(std::io::Cursor::new(bytes), vak_ooxml::Limits::default()).unwrap();
    assert!(
        document
            .units
            .iter()
            .flat_map(|unit| unit.cells.iter())
            .any(|(address, value)| address == "B2" && value == "150"),
        "the returned workbook holds the edit"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_labelled_draft_is_held_and_a_bridge_without_files_is_told_where_it_is() {
    let labelled = round_trip(
        "memo.docx",
        &vak_ooxml::fixtures::signed_labelled_docx(),
        serde_json::json!([{"op": "replace_paragraph_text", "anchor": "p@1", "text": "Hello again"}]),
        true,
    )
    .await;
    assert!(
        labelled["files"].as_array().unwrap().is_empty(),
        "{labelled}"
    );
    assert!(
        labelled["text"]
            .as_str()
            .unwrap()
            .contains("memo.docx carries the sensitivity label Confidential"),
        "{labelled}"
    );

    let plain = round_trip(
        "budget.xlsx",
        &vak_ooxml::fixtures::xlsx(),
        serde_json::json!([{"op": "set_cells", "sheet": "Budget", "cells": {"B2": 150}}]),
        false,
    )
    .await;
    assert!(plain["files"].as_array().unwrap().is_empty(), "{plain}");
    assert!(
        plain["text"]
            .as_str()
            .unwrap()
            .contains("The updated budget.xlsx is ready in Vak"),
        "{plain}"
    );
}

/// The whole Telegram path against a Bot API double: a workbook arrives as
/// a document, the bridge hands it to the gateway, and the edited workbook
/// goes back to the same chat with `sendDocument`, after the text reply.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn the_telegram_bridge_sends_the_edited_workbook_back() {
    let workbook = vak_ooxml::fixtures::xlsx();
    let (path, digest) = inbox_path("budget.xlsx", &workbook);
    let sent: Arc<Mutex<Vec<(String, String)>>> = Arc::new(Mutex::new(Vec::new()));
    let served = Arc::new(Mutex::new(false));
    let app = {
        let sent_message = sent.clone();
        let sent_document = sent.clone();
        let file = workbook.clone();
        axum::Router::new()
            .route(
                "/botbottok/getUpdates",
                axum::routing::get(move || async move {
                    let mut once = served.lock().unwrap();
                    let result = if *once {
                        serde_json::json!([])
                    } else {
                        *once = true;
                        serde_json::json!([{
                            "update_id": 900,
                            "message": {
                                "chat": {"id": 4242},
                                "from": {"id": 7},
                                "caption": "raise food to 150",
                                "document": {"file_id": "doc1", "file_name": "budget.xlsx", "mime_type": "application/vnd.openxmlformats-officedocument.spreadsheetml.sheet"}
                            }
                        }])
                    };
                    axum::Json(serde_json::json!({"ok": true, "result": result}))
                }),
            )
            .route(
                "/botbottok/getFile",
                axum::routing::get(|| async {
                    axum::Json(serde_json::json!({"ok": true, "result": {"file_path": "documents/budget.xlsx"}}))
                }),
            )
            .route(
                "/file/botbottok/documents/budget.xlsx",
                axum::routing::get(move || {
                    let file = file.clone();
                    async move { file }
                }),
            )
            .route(
                "/botbottok/sendMessage",
                axum::routing::post(move |body: String| async move {
                    sent_message.lock().unwrap().push(("message".into(), body));
                    axum::Json(serde_json::json!({"ok": true}))
                }),
            )
            .route(
                "/botbottok/sendDocument",
                axum::routing::post(move |body: axum::body::Bytes| async move {
                    sent_document
                        .lock()
                        .unwrap()
                        .push(("document".into(), String::from_utf8_lossy(&body).into_owned()));
                    axum::Json(serde_json::json!({"ok": true}))
                }),
            )
            .layer(axum::extract::DefaultBodyLimit::max(64 * 1024 * 1024))
    };
    let telegram = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let telegram_addr = telegram.local_addr().unwrap();
    tokio::spawn(async move { axum::serve(telegram, app).await.unwrap() });

    let provider = Arc::new(Scripted(
        Mutex::new(VecDeque::from([
            message(
                ContentBlock::ToolUse {
                    id: "call_edit".into(),
                    name: "office_apply".into(),
                    input: serde_json::json!({
                        "path": path,
                        "base_digest": digest,
                        "ops": [{"op": "set_cells", "sheet": "Budget", "cells": {"B2": 150}}],
                    }),
                },
                StopReason::ToolUse,
            ),
            message(ContentBlock::text("Raised it to 150."), StopReason::EndTurn),
        ])),
        Mutex::new(Vec::new()),
        crate::support::CapacityKey::default(),
    ));
    let dir = tempfile::tempdir().unwrap();
    let cwd = dir.path().to_path_buf();
    std::fs::create_dir_all(cwd.join(".vak")).unwrap();
    std::fs::write(
        cwd.join(".vak/config.toml"),
        "[memory]\nreflection = false\n[gateway]\nchat_allowlist = [\"telegram:4242\"]\n",
    )
    .unwrap();
    vak_config::paths::isolate_home_for_tests();
    let core = Core::new_with_trust(cwd.clone(), true).unwrap();
    core.set_sessions_home(dir.path().join("home"));
    core.set_permission_mode(vak_config::PermissionMode::WorkspaceWrite);
    core.set_tool_worker_exe(std::path::PathBuf::from(env!(
        "CARGO_BIN_EXE_vak-tool-worker"
    )));
    core.set_provider_instance(provider.clone());
    std::mem::forget(dir);
    let gateway = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let gateway_addr = gateway.local_addr().unwrap();
    tokio::spawn(async move {
        axum::serve(gateway, vak_server::gateway_router(core))
            .await
            .unwrap();
    });

    let bridge = vak_server::surfaces::telegram::TelegramBridge {
        token_env: String::new(),
        locks_dir: None,
        api_base: format!("http://{telegram_addr}"),
        bot_token: "bottok".into(),
        gateway_url: format!("http://{gateway_addr}"),
        gateway_token: "vk_test".into(),
        bot_id: None,
    };
    assert_eq!(bridge.tick(0).await.unwrap(), 901);

    let sent = sent.lock().unwrap();
    assert_eq!(sent.len(), 2, "{sent:?}");
    assert_eq!(sent[0].0, "message");
    assert!(sent[0].1.contains("Raised it to 150."), "{}", sent[0].1);
    let (kind, body) = &sent[1];
    assert_eq!(kind, "document");
    assert!(
        body.contains("filename=\"budget.xlsx\""),
        "the file keeps its sent name"
    );
    assert!(
        body.contains("Updated budget.xlsx: Budget: 1 changed"),
        "{body}"
    );
    assert!(
        body.contains("name=\"chat_id\"\r\n\r\n4242"),
        "back to the same chat"
    );
    let asked = serde_json::to_string(&provider.1.lock().unwrap()[0].messages).unwrap();
    assert!(
        asked.contains("raise food to 150"),
        "the file's caption is the request"
    );
}
