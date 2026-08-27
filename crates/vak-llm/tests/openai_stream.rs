#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use tokio_util::sync::CancellationToken;

use vak_llm::Provider;
use vak_llm::error::LlmError;
use vak_llm::openai::{OpenAiCompletionsProvider, OpenAiConfig, build_body};
use vak_llm::stream::StreamEvent;
use vak_llm::types::{ChatRequest, ContentBlock, Message, Role, StopReason, ToolDefinition};

fn sample_request() -> ChatRequest {
    let mut req = ChatRequest::new("gpt-5.6");
    req.system = Some("You are a coding agent.".into());
    req.messages = vec![
        Message::user_text("list the files"),
        Message::assistant(vec![
            ContentBlock::Thinking {
                text: "hmm".into(),
                signature: Some("sig".into()),
            },
            ContentBlock::text("Let me check."),
            ContentBlock::ToolUse {
                id: "toolu_abc".into(),
                name: "bash".into(),
                input: serde_json::json!({"command": "ls"}),
            },
        ]),
        Message {
            role: Role::User,
            content: vec![ContentBlock::tool_result("toolu_abc", "src\nREADME.md")],
        },
        Message::user_text("and now?"),
    ];
    req.tools = vec![ToolDefinition::new(
        "bash",
        "Run a shell command",
        serde_json::json!({"type": "object", "properties": {"command": {"type": "string"}}, "required": ["command"]}),
    )];
    req
}

#[test]
fn body_maps_neutral_history_to_openai_shape() {
    let body = build_body(&sample_request()).unwrap();
    assert_eq!(body["model"], "gpt-5.6");
    assert_eq!(body["stream"], true);
    assert_eq!(body["stream_options"]["include_usage"], true);

    let msgs = body["messages"].as_array().unwrap();
    assert_eq!(msgs[0]["role"], "system");
    assert_eq!(msgs[1]["role"], "user");
    assert_eq!(msgs[1]["content"], "list the files");

    let assistant = &msgs[2];
    assert_eq!(assistant["role"], "assistant");
    assert!(
        assistant.get("thinking").is_none(),
        "thinking blocks must not leak into openai payloads"
    );
    let calls = assistant["tool_calls"].as_array().unwrap();
    assert_eq!(
        calls[0]["id"], "toolu_abc",
        "tool ids survive provider switches"
    );
    assert_eq!(calls[0]["function"]["name"], "bash");
    let args = calls[0]["function"]["arguments"].as_str().unwrap();
    assert!(
        serde_json::from_str::<serde_json::Value>(args).is_ok(),
        "arguments must be a JSON string"
    );

    let tool_msg = &msgs[3];
    assert_eq!(tool_msg["role"], "tool");
    assert_eq!(tool_msg["tool_call_id"], "toolu_abc");
    assert_eq!(tool_msg["content"], "src\nREADME.md");

    assert_eq!(msgs[4]["role"], "user");
    assert_eq!(msgs[4]["content"], "and now?");

    let tools = body["tools"].as_array().unwrap();
    assert_eq!(tools[0]["type"], "function");
    assert_eq!(tools[0]["function"]["name"], "bash");
}

#[test]
fn user_tool_results_become_separate_tool_messages() {
    let mut req = ChatRequest::new("m");
    req.messages = vec![Message {
        role: Role::User,
        content: vec![
            ContentBlock::text("results below"),
            ContentBlock::tool_result("a", "one"),
            ContentBlock::tool_result("b", "two"),
        ],
    }];
    let msgs = build_body(&req).unwrap()["messages"]
        .as_array()
        .unwrap()
        .clone();
    assert_eq!(msgs.len(), 3);
    assert_eq!(msgs[0]["role"], "tool");
    assert_eq!(msgs[1]["role"], "tool");
    assert_eq!(msgs[1]["tool_call_id"], "b");
    assert_eq!(msgs[2]["role"], "user");
}

const FIXTURE_TOOL_STREAM: &str = "\
data: {\"choices\":[{\"index\":0,\"delta\":{\"role\":\"assistant\",\"content\":\"Checking\"},\"finish_reason\":null}]}\n\
\n\
data: {\"choices\":[{\"index\":0,\"delta\":{\"content\":\" now.\"},\"finish_reason\":null}]}\n\
\n\
data: {\"choices\":[{\"index\":0,\"delta\":{\"tool_calls\":[{\"index\":0,\"id\":\"call_9\",\"type\":\"function\",\"function\":{\"name\":\"read\",\"arguments\":\"\"}}]},\"finish_reason\":null}]}\n\
\n\
data: {\"choices\":[{\"index\":0,\"delta\":{\"tool_calls\":[{\"index\":0,\"function\":{\"arguments\":\"{\\\"path\\\":\"}}]},\"finish_reason\":null}]}\n\
\n\
data: {\"choices\":[{\"index\":0,\"delta\":{\"tool_calls\":[{\"index\":0,\"function\":{\"arguments\":\"\\\"a.txt\\\"}\"}}]},\"finish_reason\":null}]}\n\
\n\
data: {\"choices\":[{\"index\":0,\"delta\":{},\"finish_reason\":\"tool_calls\"}]}\n\
\n\
data: {\"choices\":[],\"usage\":{\"prompt_tokens\":11,\"completion_tokens\":7}}\n\
\n\
data: [DONE]\n\
\n";

#[tokio::test]
async fn full_stream_accumulates_text_and_tool_calls() {
    let provider = OpenAiCompletionsProvider::new(OpenAiConfig {
        api_key: "k".into(),
        base_url: mock_url(FIXTURE_TOOL_STREAM).await,
    })
    .unwrap();

    let mut es = provider
        .stream(sample_request(), CancellationToken::new())
        .await
        .unwrap();
    let mut text = String::new();
    while let Some(ev) = futures::StreamExt::next(&mut es).await {
        if let StreamEvent::TextDelta { delta, .. } = ev {
            text.push_str(&delta);
        }
    }
    let msg = es.result().await.unwrap();

    assert_eq!(text, "Checking now.");
    assert_eq!(msg.stop_reason, StopReason::ToolUse);
    assert_eq!(msg.usage.input_tokens, 11);
    assert_eq!(msg.usage.output_tokens, 7);

    let calls: Vec<&ContentBlock> = msg
        .content
        .iter()
        .filter(|b| matches!(b, ContentBlock::ToolUse { .. }))
        .collect();
    assert_eq!(calls.len(), 1);
    if let ContentBlock::ToolUse { id, name, input } = calls[0] {
        assert_eq!(id, "call_9");
        assert_eq!(name, "read");
        assert_eq!(input, &serde_json::json!({"path": "a.txt"}));
    } else {
        unreachable!()
    }
}

const FIXTURE_STOP_STREAM: &str = "\
data: {\"choices\":[{\"index\":0,\"delta\":{\"content\":\"done\"},\"finish_reason\":null}]}\n\
\n\
data: {\"choices\":[{\"index\":0,\"delta\":{},\"finish_reason\":\"stop\"}]}\n\
\n\
data: [DONE]\n\
\n";

#[tokio::test]
async fn finish_stop_maps_to_end_turn() {
    let provider = OpenAiCompletionsProvider::new(OpenAiConfig {
        api_key: "k".into(),
        base_url: mock_url(FIXTURE_STOP_STREAM).await,
    })
    .unwrap();
    let mut req = ChatRequest::new("m");
    req.messages = vec![Message::user_text("hi")];
    let mut es = provider
        .stream(req, CancellationToken::new())
        .await
        .unwrap();
    while futures::StreamExt::next(&mut es).await.is_some() {}
    let msg = es.result().await.unwrap();
    assert_eq!(msg.stop_reason, StopReason::EndTurn);
    assert_eq!(msg.text_content(), "done");
}

/// The terminal chunk carries both the last content delta and finish_reason,
/// which is what opencode-zen and several other compat endpoints do.
const FIXTURE_CONTENT_WITH_FINISH: &str = "\
data: {\"choices\":[{\"index\":0,\"delta\":{\"content\":\"p\"},\"finish_reason\":null}]}\n\
\n\
data: {\"choices\":[{\"index\":0,\"delta\":{\"content\":\"ong\"},\"finish_reason\":\"stop\"}]}\n\
\n\
data: [DONE]\n\
\n";

#[tokio::test]
async fn content_in_the_finish_chunk_is_not_discarded() {
    // Ending the turn before folding in that chunk's text truncated every
    // reply: long answers lost their final token and "pong" arrived as "p".
    let provider = OpenAiCompletionsProvider::new(OpenAiConfig {
        api_key: "k".into(),
        base_url: mock_url(FIXTURE_CONTENT_WITH_FINISH).await,
    })
    .unwrap();
    let mut req = ChatRequest::new("m");
    req.messages = vec![Message::user_text("hi")];
    let mut es = provider
        .stream(req, CancellationToken::new())
        .await
        .unwrap();
    while futures::StreamExt::next(&mut es).await.is_some() {}
    let msg = es.result().await.unwrap();
    assert_eq!(msg.text_content(), "pong");
    assert_eq!(msg.stop_reason, StopReason::EndTurn);
}

/// A body that ends immediately after the final `data:` line, with no
/// terminating blank line and no `[DONE]`.
const FIXTURE_UNTERMINATED_TAIL: &str = "\
data: {\"choices\":[{\"index\":0,\"delta\":{\"content\":\"po\"},\"finish_reason\":null}]}\n\
\n\
data: {\"choices\":[{\"index\":0,\"delta\":{\"content\":\"ng\"},\"finish_reason\":\"stop\"}]}\n";

#[tokio::test]
async fn an_unterminated_final_frame_is_still_decoded() {
    let provider = OpenAiCompletionsProvider::new(OpenAiConfig {
        api_key: "k".into(),
        base_url: mock_url(FIXTURE_UNTERMINATED_TAIL).await,
    })
    .unwrap();
    let mut req = ChatRequest::new("m");
    req.messages = vec![Message::user_text("hi")];
    let mut es = provider
        .stream(req, CancellationToken::new())
        .await
        .unwrap();
    while futures::StreamExt::next(&mut es).await.is_some() {}
    let msg = es.result().await.unwrap();
    assert_eq!(msg.text_content(), "pong");
}

#[tokio::test]
async fn http_error_maps_to_typed_value() {
    let url = error_url(429, r#"{"error":{"message":"quota exceeded"}}"#).await;
    let provider = OpenAiCompletionsProvider::new(OpenAiConfig {
        api_key: "k".into(),
        base_url: url,
    })
    .unwrap();
    let err = provider
        .stream(ChatRequest::new("m"), CancellationToken::new())
        .await
        .err()
        .expect("expected error");
    assert!(matches!(err, LlmError::RateLimit { .. }), "got {err:?}");
}

// Regression (live, OpenCode Zen): proxies may end the body after the last
// content chunk with neither finish_reason nor [DONE]. Content-bearing clean
// closes complete; empty ones still fail closed.
const FIXTURE_TRUNCATED_WITH_CONTENT: &str = "\
data: {\"choices\":[{\"index\":0,\"delta\":{\"role\":\"assistant\",\"content\":\"partial answer\"},\"finish_reason\":null}]}\n\
\n";

const FIXTURE_TRUNCATED_EMPTY: &str = "\
data: {\"choices\":[{\"index\":0,\"delta\":{\"role\":\"assistant\"},\"finish_reason\":null}]}\n\
\n";

#[tokio::test]
async fn clean_close_with_content_completes_as_end_turn() {
    let provider = OpenAiCompletionsProvider::new(OpenAiConfig {
        api_key: "k".into(),
        base_url: mock_url(FIXTURE_TRUNCATED_WITH_CONTENT).await,
    })
    .unwrap();
    let mut req = ChatRequest::new("m");
    req.messages = vec![Message::user_text("hi")];
    let mut es = provider
        .stream(req, CancellationToken::new())
        .await
        .unwrap();
    while futures::StreamExt::next(&mut es).await.is_some() {}
    let msg = es.result().await.unwrap();
    assert_eq!(msg.stop_reason, StopReason::EndTurn);
    assert_eq!(msg.text_content(), "partial answer");
}

#[tokio::test]
async fn clean_close_without_content_still_fails_closed() {
    let provider = OpenAiCompletionsProvider::new(OpenAiConfig {
        api_key: "k".into(),
        base_url: mock_url(FIXTURE_TRUNCATED_EMPTY).await,
    })
    .unwrap();
    let mut req = ChatRequest::new("m");
    req.messages = vec![Message::user_text("hi")];
    let mut es = provider
        .stream(req, CancellationToken::new())
        .await
        .unwrap();
    while futures::StreamExt::next(&mut es).await.is_some() {}
    let err = es.result().await.expect_err("expected parse error");
    assert!(
        matches!(err, LlmError::Parse(ref m) if m.contains("finish_reason")),
        "got {err:?}"
    );
}

async fn spawn_server(body: Vec<u8>) -> String {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        let (mut sock, _) = listener.accept().await.unwrap();
        drain_headers(&mut sock).await;
        use tokio::io::AsyncWriteExt;
        let _ = sock.write_all(&body).await;
        let _ = sock.flush().await;
        let _ = sock.shutdown().await;
    });
    format!("http://{addr}")
}

async fn drain_headers<S>(sock: &mut S)
where
    S: tokio::io::AsyncRead + Unpin,
{
    use tokio::io::AsyncReadExt;
    let mut buf = Vec::new();
    let mut chunk = [0u8; 1024];
    loop {
        let n = sock.read(&mut chunk).await.unwrap_or(0);
        if n == 0 {
            return;
        }
        buf.extend_from_slice(&chunk[..n]);
        if buf.windows(4).any(|w| w == b"\r\n\r\n") {
            return;
        }
    }
}

async fn mock_url(sse_body: &str) -> String {
    let owned = format!(
        "HTTP/1.1 200 OK\r\ncontent-type: text/event-stream\r\nconnection: close\r\n\r\n{sse_body}"
    );
    spawn_server(owned.into_bytes()).await
}

async fn error_url(status: u16, json: &str) -> String {
    let owned = format!(
        "HTTP/1.1 {status} Err\r\ncontent-type: application/json\r\nconnection: close\r\n\r\n{json}"
    );
    spawn_server(owned.into_bytes()).await
}

// Some OpenAI-compatible endpoints (observed on opencode-zen's free tier)
// close tool-call turns with a finish_reason outside the canonical set —
// e.g. plain "stop". The accumulated tool_use blocks are ground truth: the
// loop must see StopReason::ToolUse or a dangling call kills the run.
const FIXTURE_TOOL_STREAM_BAD_FINISH: &str = "\
data: {\"choices\":[{\"index\":0,\"delta\":{\"role\":\"assistant\"},\"finish_reason\":null}]}\n\
\n\
data: {\"choices\":[{\"index\":0,\"delta\":{\"tool_calls\":[{\"index\":0,\"id\":\"call_1\",\"type\":\"function\",\"function\":{\"name\":\"glob\",\"arguments\":\"{\\\"pattern\\\":\\\"**/*.md\\\"}\"}}]},\"finish_reason\":null}]}\n\
\n\
data: {\"choices\":[{\"index\":0,\"delta\":{},\"finish_reason\":\"stop\"}]}\n\
\n\
data: [DONE]\n\
\n";

#[tokio::test]
async fn nonstandard_finish_reason_with_tool_use_still_yields_tooluse() {
    let provider = OpenAiCompletionsProvider::new(OpenAiConfig {
        api_key: "k".into(),
        base_url: mock_url(FIXTURE_TOOL_STREAM_BAD_FINISH).await,
    })
    .unwrap();

    let mut req = ChatRequest::new("m");
    req.messages = vec![Message::user_text("list markdown files")];

    let mut es = provider
        .stream(req, CancellationToken::new())
        .await
        .unwrap();
    let mut end = None;
    while let Some(ev) = futures::StreamExt::next(&mut es).await {
        if let StreamEvent::End { message } = ev {
            end = Some(message);
        }
    }
    let msg = end.expect("stream must terminate with End");
    assert_eq!(msg.stop_reason, StopReason::ToolUse);
    assert!(
        msg.content
            .iter()
            .any(|b| matches!(b, ContentBlock::ToolUse { name, .. } if name == "glob")),
        "tool_use block preserved"
    );
}

// Same endpoint family, worse: body ends right after the tool-call deltas
// with NO finish_reason and NO [DONE]. The clean-close path must also
// promote accumulated tool_use to StopReason::ToolUse.
const FIXTURE_TOOL_STREAM_EOF_NO_FINISH: &str = "\
data: {\"choices\":[{\"index\":0,\"delta\":{\"role\":\"assistant\"},\"finish_reason\":null}]}\n\
\n\
data: {\"choices\":[{\"index\":0,\"delta\":{\"tool_calls\":[{\"index\":0,\"id\":\"call_2\",\"type\":\"function\",\"function\":{\"name\":\"glob\",\"arguments\":\"{\\\"pattern\\\":\\\"*\\\"}\"}}]},\"finish_reason\":null}]}\n\
\n";

#[tokio::test]
async fn clean_close_after_tool_calls_yields_tooluse() {
    let provider = OpenAiCompletionsProvider::new(OpenAiConfig {
        api_key: "k".into(),
        base_url: mock_url(FIXTURE_TOOL_STREAM_EOF_NO_FINISH).await,
    })
    .unwrap();

    let mut req = ChatRequest::new("m");
    req.messages = vec![Message::user_text("list files")];

    let es = provider
        .stream(req, CancellationToken::new())
        .await
        .unwrap();
    // EOF-close delivers the message through the stream's terminal.
    let msg = es.result().await.expect("clean close must complete");
    assert_eq!(msg.stop_reason, StopReason::ToolUse);
}

// The live culprit (opencode-zen free tier): reasoning deltas, one
// tool_calls delta, then bare `data: [DONE]` — no finish_reason frame ever.
const FIXTURE_DONE_WITHOUT_FINISH: &str = "\
data: {\"choices\":[{\"index\":0,\"delta\":{\"role\":\"assistant\",\"reasoning_content\":\"thinking\"},\"finish_reason\":null}]}\n\
\n\
data: {\"choices\":[{\"index\":0,\"delta\":{\"tool_calls\":[{\"index\":0,\"id\":\"call_3\",\"type\":\"function\",\"function\":{\"name\":\"glob\",\"arguments\":\"{\\\"pattern\\\":\\\"*\\\"}\"}}]},\"finish_reason\":null}]}\n\
\n\
data: [DONE]\n\
\n";

#[tokio::test]
async fn done_without_finish_reason_still_yields_tooluse() {
    let provider = OpenAiCompletionsProvider::new(OpenAiConfig {
        api_key: "k".into(),
        base_url: mock_url(FIXTURE_DONE_WITHOUT_FINISH).await,
    })
    .unwrap();

    let mut req = ChatRequest::new("m");
    req.messages = vec![Message::user_text("list files")];

    let es = provider
        .stream(req, CancellationToken::new())
        .await
        .unwrap();
    let msg = es.result().await.expect("[DONE] close must complete");
    assert_eq!(msg.stop_reason, StopReason::ToolUse);
}
