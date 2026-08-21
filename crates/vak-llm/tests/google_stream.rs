#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use tokio_util::sync::CancellationToken;

use vak_llm::Provider;
use vak_llm::error::LlmError;
use vak_llm::google::{GoogleConfig, GoogleProvider, build_body};
use vak_llm::stream::StreamEvent;
use vak_llm::types::{ChatRequest, ContentBlock, Message, Role, StopReason, ToolDefinition};

fn sample_request() -> ChatRequest {
    let mut req = ChatRequest::new("gemini-3-pro");
    req.system = Some("You are a coding agent.".into());
    req.messages = vec![
        Message::user_text("list the files"),
        Message::assistant(vec![
            ContentBlock::text("Checking."),
            ContentBlock::ToolUse {
                id: "call_abc".into(),
                name: "bash".into(),
                input: serde_json::json!({"command": "ls"}),
            },
        ]),
        Message {
            role: Role::User,
            content: vec![ContentBlock::tool_result("call_abc", "src\nREADME.md")],
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
fn body_maps_neutral_history_to_gemini_shape() {
    let body = build_body(&sample_request()).unwrap();
    assert_eq!(
        body["systemInstruction"]["parts"][0]["text"],
        "You are a coding agent."
    );

    let contents = body["contents"].as_array().unwrap();
    // user, model(text+functionCall), user(functionResponse), user(text)
    assert_eq!(contents.len(), 4);

    assert_eq!(contents[0]["role"], "user");
    assert_eq!(contents[0]["parts"][0]["text"], "list the files");

    assert_eq!(contents[1]["role"], "model");
    let parts = contents[1]["parts"].as_array().unwrap();
    assert_eq!(parts[0]["text"], "Checking.");
    assert_eq!(parts[1]["functionCall"]["name"], "bash");
    // Gemini args are OBJECTS, not strings.
    assert_eq!(
        parts[1]["functionCall"]["args"],
        serde_json::json!({"command": "ls"})
    );

    assert_eq!(contents[2]["role"], "user");
    let resp_parts = contents[2]["parts"].as_array().unwrap();
    // functionResponse keyed by NAME resolved from the prior functionCall.
    assert_eq!(
        resp_parts[0]["functionResponse"]["name"], "bash",
        "tool_use_id must resolve to the function name via history"
    );
    assert_eq!(
        resp_parts[0]["functionResponse"]["response"]["result"],
        "src\nREADME.md"
    );

    assert_eq!(contents[3]["role"], "user");
    assert_eq!(contents[3]["parts"][0]["text"], "and now?");

    let decls = body["tools"][0]["functionDeclarations"].as_array().unwrap();
    assert_eq!(decls[0]["name"], "bash");
}

const FIXTURE_STREAM: &str = "\
data: {\"candidates\":[{\"content\":{\"parts\":[{\"text\":\"Read\"}],\"role\":\"model\"}}],\"usageMetadata\":{\"promptTokenCount\":21,\"candidatesTokenCount\":2}}\n\
\n\
data: {\"candidates\":[{\"content\":{\"parts\":[{\"text\":\"ing now.\"}],\"role\":\"model\"}}]}\n\
\n\
data: {\"candidates\":[{\"content\":{\"parts\":[{\"functionCall\":{\"name\":\"read\",\"args\":{\"path\":\"a.txt\"}}}],\"role\":\"model\"},\"finishReason\":\"STOP\"}],\"usageMetadata\":{\"promptTokenCount\":21,\"candidatesTokenCount\":9}}\n\
\n";

#[tokio::test]
async fn full_stream_accumulates_text_and_function_calls() {
    let provider = GoogleProvider::new(GoogleConfig {
        api_key: "k".into(),
        base_url: mock_url(FIXTURE_STREAM).await,
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

    assert_eq!(text, "Reading now.");
    assert_eq!(msg.stop_reason, StopReason::ToolUse);
    assert_eq!(msg.usage.input_tokens, 21);
    assert_eq!(msg.usage.output_tokens, 9);

    let calls: Vec<&ContentBlock> = msg
        .content
        .iter()
        .filter(|b| matches!(b, ContentBlock::ToolUse { .. }))
        .collect();
    assert_eq!(calls.len(), 1);
    if let ContentBlock::ToolUse { name, input, .. } = calls[0] {
        assert_eq!(name, "read");
        assert_eq!(input, &serde_json::json!({"path": "a.txt"}));
    } else {
        unreachable!()
    }
}

const FIXTURE_STOP: &str = "\
data: {\"candidates\":[{\"content\":{\"parts\":[{\"text\":\"done\"}],\"role\":\"model\"},\"finishReason\":\"STOP\"}],\"usageMetadata\":{\"promptTokenCount\":5,\"candidatesTokenCount\":1}}\n\
\n";

#[tokio::test]
async fn finish_stop_maps_to_end_turn() {
    let provider = GoogleProvider::new(GoogleConfig {
        api_key: "k".into(),
        base_url: mock_url(FIXTURE_STOP).await,
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

#[tokio::test]
async fn http_error_maps_to_typed_value() {
    let url = error_url(429, r#"{"error":{"message":"quota exceeded"}}"#).await;
    let provider = GoogleProvider::new(GoogleConfig {
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
