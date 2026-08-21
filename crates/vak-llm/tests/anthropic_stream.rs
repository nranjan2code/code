#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use tokio_util::sync::CancellationToken;

use vak_llm::Provider;
use vak_llm::anthropic::{AnthropicConfig, AnthropicProvider, build_body, map_status_error};
use vak_llm::error::LlmError;
use vak_llm::sse::SseDecoder;
use vak_llm::stream::StreamEvent;
use vak_llm::types::{ChatRequest, ContentBlock, Message, Role, StopReason, ToolDefinition};

fn sample_request() -> ChatRequest {
    let mut req = ChatRequest::new("claude-sonnet-4-5");
    req.system = Some("You are a coding agent.".into());
    req.messages = vec![
        Message::user_text("list the files"),
        Message::assistant(vec![ContentBlock::ToolUse {
            id: "toolu_1".into(),
            name: "bash".into(),
            input: serde_json::json!({"command": "ls"}),
        }]),
        Message {
            role: Role::User,
            content: vec![ContentBlock::tool_result("toolu_1", "src\nREADME.md")],
        },
    ];
    req.tools = vec![ToolDefinition::new(
        "bash",
        "Run a shell command",
        serde_json::json!({"type": "object", "properties": {"command": {"type": "string"}}, "required": ["command"]}),
    )];
    req
}

#[test]
fn body_serialization_matches_anthropic_shape() {
    let body = build_body(&sample_request()).unwrap();
    assert_eq!(body["model"], "claude-sonnet-4-5");
    assert_eq!(body["max_tokens"], 8192);
    assert_eq!(body["stream"], true);
    assert_eq!(body["system"], "You are a coding agent.");
    let tools = body["tools"].as_array().unwrap();
    assert_eq!(tools[0]["name"], "bash");
    assert!(tools[0]["input_schema"].is_object());
    let messages = body["messages"].as_array().unwrap();
    assert_eq!(messages.len(), 3);
    assert_eq!(messages[2]["content"][0]["type"], "tool_result");
    assert_eq!(messages[2]["content"][0]["tool_use_id"], "toolu_1");
}

#[test]
fn body_rejects_misplaced_blocks() {
    let mut req = ChatRequest::new("m");
    req.messages = vec![Message {
        role: Role::Assistant,
        content: vec![ContentBlock::tool_result("x", "y")],
    }];
    assert!(matches!(build_body(&req), Err(LlmError::InvalidRequest(_))));
}

#[test]
fn status_error_mapping() {
    assert!(matches!(
        map_status_error(401, r#"{"error":{"message":"bad key"}}"#, None),
        LlmError::Auth(_)
    ));
    match map_status_error(429, r#"{"error":{"message":"slow down"}}"#, Some(7)) {
        LlmError::RateLimit {
            retry_after_secs, ..
        } => assert_eq!(retry_after_secs, Some(7)),
        other => panic!("expected rate limit, got {other:?}"),
    }
    assert!(matches!(
        map_status_error(529, r#"{"error":{"message":"overloaded"}}"#, None),
        LlmError::Overloaded(_)
    ));
}

const FIXTURE_STREAM: &str = "\
event: message_start\n\
data: {\"type\":\"message_start\",\"message\":{\"model\":\"claude-sonnet-4-5\",\"usage\":{\"input_tokens\":10,\"output_tokens\":0}}}\n\
\n\
event: content_block_start\n\
data: {\"type\":\"content_block_start\",\"index\":0,\"content_block\":{\"type\":\"text\"}}\n\
\n\
event: content_block_delta\n\
data: {\"type\":\"content_block_delta\",\"index\":0,\"delta\":{\"type\":\"text_delta\",\"text\":\"Hel\"}}\n\
\n\
event: content_block_delta\n\
data: {\"type\":\"content_block_delta\",\"index\":0,\"delta\":{\"type\":\"text_delta\",\"text\":\"lo\"}}\n\
\n\
event: content_block_stop\n\
data: {\"type\":\"content_block_stop\",\"index\":0}\n\
\n\
event: content_block_start\n\
data: {\"type\":\"content_block_start\",\"index\":1,\"content_block\":{\"type\":\"tool_use\",\"id\":\"toolu_9\",\"name\":\"read\"}}\n\
\n\
event: content_block_delta\n\
data: {\"type\":\"content_block_delta\",\"index\":1,\"delta\":{\"type\":\"input_json_delta\",\"partial_json\":\"{\\\"path\\\":\"}}\n\
\n\
event: content_block_delta\n\
data: {\"type\":\"content_block_delta\",\"index\":1,\"delta\":{\"type\":\"input_json_delta\",\"partial_json\":\"\\\"a.txt\\\"}\"}}\n\
\n\
event: message_delta\n\
data: {\"type\":\"message_delta\",\"delta\":{\"stop_reason\":\"tool_use\"},\"usage\":{\"output_tokens\":42}}\n\
\n\
event: message_stop\n\
data: {\"type\":\"message_stop\"}\n\
\n";

#[tokio::test]
async fn full_stream_conversion_accumulates_snapshot() {
    let provider = AnthropicProvider::new(AnthropicConfig {
        api_key: "k".into(),
        base_url: mock_server_url(FIXTURE_STREAM).await,
        model: String::new(),
    })
    .unwrap();

    let cancel = CancellationToken::new();
    let mut es = provider.stream(sample_request(), cancel).await.unwrap();
    let mut text = String::new();
    let mut snapshots_consistent = true;
    while let Some(ev) = futures::StreamExt::next(&mut es).await {
        if let StreamEvent::TextDelta { delta, partial } = &ev {
            text.push_str(delta);
            let joined: String = partial
                .content
                .iter()
                .filter_map(|b| match b {
                    ContentBlock::Text { text } => Some(text.as_str()),
                    _ => None,
                })
                .collect();
            if joined != text {
                snapshots_consistent = false;
            }
        }
    }
    let final_msg = es.result().await.unwrap();
    assert!(snapshots_consistent);
    assert_eq!(text, "Hello");
    assert_eq!(final_msg.stop_reason, StopReason::ToolUse);
    assert_eq!(final_msg.usage.output_tokens, 42);
    assert_eq!(final_msg.usage.input_tokens, 10);
    let tool_calls: Vec<&ContentBlock> = final_msg
        .content
        .iter()
        .filter(|b| matches!(b, ContentBlock::ToolUse { .. }))
        .collect();
    assert_eq!(tool_calls.len(), 1);
    if let ContentBlock::ToolUse { id, name, input } = tool_calls[0] {
        assert_eq!(id, "toolu_9");
        assert_eq!(name, "read");
        assert_eq!(input, &serde_json::json!({"path": "a.txt"}));
    } else {
        unreachable!()
    }
}

#[tokio::test]
async fn abort_preserves_partial_output() {
    let part1 = "\
event: message_start\n\
data: {\"type\":\"message_start\",\"message\":{\"model\":\"m\",\"usage\":{}}}\n\
\n\
event: content_block_start\n\
data: {\"type\":\"content_block_start\",\"index\":0,\"content_block\":{\"type\":\"text\"}}\n\
\n\
event: content_block_delta\n\
data: {\"type\":\"content_block_delta\",\"index\":0,\"delta\":{\"type\":\"text_delta\",\"text\":\"partial answer\"}}\n\
\n"
        .to_string();
    let url = delayed_server_url(part1, String::new()).await;
    let provider = AnthropicProvider::new(AnthropicConfig {
        api_key: "k".into(),
        base_url: url,
        model: String::new(),
    })
    .unwrap();

    let cancel = CancellationToken::new();
    let mut es = provider
        .stream(sample_request(), cancel.clone())
        .await
        .unwrap();
    while let Some(ev) = futures::StreamExt::next(&mut es).await {
        if matches!(ev, StreamEvent::TextDelta { .. }) {
            break;
        }
    }
    cancel.cancel();
    match es.result().await {
        Err(LlmError::Aborted { partial }) => {
            let p = partial.expect("partial should be preserved");
            assert_eq!(
                p.content[0],
                ContentBlock::Text {
                    text: "partial answer".into()
                }
            );
        }
        other => panic!("expected aborted with partial, got {other:?}"),
    }
}

#[tokio::test]
async fn http_error_maps_to_typed_value() {
    let url = error_server_url(401, r#"{"error":{"message":"invalid x-api-key"}}"#).await;
    let provider = AnthropicProvider::new(AnthropicConfig {
        api_key: "bad".into(),
        base_url: url,
        model: String::new(),
    })
    .unwrap();
    let err = provider
        .stream(ChatRequest::new("m"), CancellationToken::new())
        .await
        .err()
        .expect("expected error");
    assert!(matches!(err, LlmError::Auth(_)), "got {err:?}");
}

async fn spawn_server<F>(handler: F) -> String
where
    F: FnOnce() -> Vec<u8> + Send + 'static,
{
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        let (mut sock, _) = listener.accept().await.unwrap();
        drain_headers(&mut sock).await;
        use tokio::io::AsyncWriteExt;
        let body = handler();
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

async fn mock_server_url(sse_body: &str) -> String {
    let owned = format!(
        "HTTP/1.1 200 OK\r\ncontent-type: text/event-stream\r\nconnection: close\r\n\r\n{sse_body}"
    );
    spawn_server(move || owned.into_bytes()).await
}

async fn delayed_server_url(first: String, second: String) -> String {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        let (mut sock, _) = listener.accept().await.unwrap();
        drain_headers(&mut sock).await;
        use tokio::io::AsyncWriteExt;
        let head =
            b"HTTP/1.1 200 OK\r\ncontent-type: text/event-stream\r\nconnection: close\r\n\r\n";
        let _ = sock.write_all(head).await;
        let _ = sock.write_all(first.as_bytes()).await;
        let _ = sock.flush().await;
        tokio::time::sleep(std::time::Duration::from_secs(30)).await;
        let _ = sock.write_all(second.as_bytes()).await;
        let _ = sock.shutdown().await;
    });
    format!("http://{addr}")
}

async fn error_server_url(status: u16, json: &str) -> String {
    let owned = format!(
        "HTTP/1.1 {status} Error\r\ncontent-type: application/json\r\nconnection: close\r\n\r\n{json}"
    );
    spawn_server(move || owned.into_bytes()).await
}

#[test]
fn sse_decoder_handles_split_chunks_and_crlf() {
    let mut d = SseDecoder::new();
    d.push(b"event: mess");
    assert!(d.next_frame().is_none());
    d.push(b"age_start\r\ndata: {\"a\":1}\r\n");
    d.push(b"\r\ndata: line1\ndata: line2\n\n");
    let f1 = d.next_frame().unwrap();
    assert_eq!(f1.event.as_deref(), Some("message_start"));
    assert_eq!(f1.data, "{\"a\":1}");
    let f2 = d.next_frame().unwrap();
    assert_eq!(f2.event, None);
    assert_eq!(f2.data, "line1\nline2");
    assert!(d.next_frame().is_none());
}

#[test]
fn sse_decoder_skips_comments_and_keepalives() {
    let mut d = SseDecoder::new();
    d.push(b": keepalive\n\nevent: x\ndata: y\n\n");
    let f = d.next_frame().unwrap();
    assert_eq!(f.data, "y");
    assert_eq!(f.event.as_deref(), Some("x"));
}
