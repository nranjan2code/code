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
data: {\"candidates\":[{\"content\":{\"parts\":[{\"functionCall\":{\"name\":\"read\",\"args\":{\"path\":\"a.txt\"}},\"thoughtSignature\":\"test_sig_value\"}],\"role\":\"model\"},\"finishReason\":\"STOP\"}],\"usageMetadata\":{\"promptTokenCount\":21,\"candidatesTokenCount\":9}}\n\
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

    let sigs: Vec<Option<&str>> = msg
        .content
        .iter()
        .filter_map(|b| match b {
            ContentBlock::Thinking { signature, .. } => Some(signature.as_deref()),
            _ => None,
        })
        .collect();
    assert_eq!(sigs, vec![Some("test_sig_value")]);
}

// Realistic cache-bearing usage (docs/design/68-context-engine.md §1):
// `promptTokenCount` is the whole prompt, `cachedContentTokenCount` a
// SUBSET of it, not an addition.
const FIXTURE_CACHED_USAGE: &str = "\
data: {\"candidates\":[{\"content\":{\"parts\":[{\"text\":\"ok\"}],\"role\":\"model\"},\"finishReason\":\"STOP\"}],\"usageMetadata\":{\"promptTokenCount\":8000,\"candidatesTokenCount\":40,\"cachedContentTokenCount\":7500}}\n\
\n";

#[tokio::test]
async fn cached_tokens_are_split_out_of_prompt_token_count_not_added_on_top() {
    let provider = GoogleProvider::new(GoogleConfig {
        api_key: "k".into(),
        base_url: mock_url(FIXTURE_CACHED_USAGE).await,
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
    assert_eq!(msg.usage.input_tokens, 500);
    assert_eq!(msg.usage.cache_read_input_tokens, Some(7500));
    assert_eq!(msg.usage.prompt_tokens(), 8000);
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

#[test]
fn body_sanitizes_unsupported_schema_keywords_for_gemini() {
    let mut req = ChatRequest::new("gemini-2.5-flash");
    req.messages = vec![Message::user_text("test")];
    req.tools = vec![ToolDefinition::new(
        "complex_tool",
        "A tool with schema keywords unsupported by Gemini",
        serde_json::json!({
            "$schema": "http://json-schema.org/draft-07/schema#",
            "type": "object",
            "additionalProperties": false,
            "properties": {
                "name": {
                    "type": "string",
                    "description": "The name"
                },
                "nested": {
                    "type": "object",
                    "additionalProperties": false,
                    "properties": {
                        "inner": {"type": "string"}
                    }
                }
            },
            "patternProperties": {
                "^x-": {"type": "string"}
            },
            "definitions": {
                "something": {}
            }
        }),
    )];

    let body = build_body(&req).unwrap();
    let params = &body["tools"][0]["functionDeclarations"][0]["parameters"];
    assert!(params.get("$schema").is_none());
    assert!(params.get("additionalProperties").is_none());
    assert!(params.get("patternProperties").is_none());
    assert!(params.get("definitions").is_none());
    assert!(
        params["properties"]["nested"]
            .get("additionalProperties")
            .is_none()
    );
    assert_eq!(params["properties"]["name"]["type"], "string");
}

#[test]
fn body_serializes_thought_signature_on_assistant_function_calls() {
    let mut req = ChatRequest::new("gemini-3.8-flash");
    req.messages = vec![
        Message::user_text("what is the weather?"),
        Message::assistant(vec![
            ContentBlock::Thinking {
                text: String::new(),
                signature: Some("sig_abc_123".into()),
            },
            ContentBlock::ToolUse {
                id: "call_1".into(),
                name: "get_weather".into(),
                input: serde_json::json!({"location": "Tokyo"}),
            },
        ]),
        Message {
            role: Role::User,
            content: vec![ContentBlock::tool_result("call_1", "Sunny 22C")],
        },
    ];

    let body = build_body(&req).unwrap();
    let contents = body["contents"].as_array().unwrap();
    assert_eq!(contents.len(), 3);
    let model_parts = contents[1]["parts"].as_array().unwrap();
    assert_eq!(model_parts.len(), 1);
    assert_eq!(model_parts[0]["functionCall"]["name"], "get_weather");
    assert_eq!(model_parts[0]["thoughtSignature"], "sig_abc_123");
}
