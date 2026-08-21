#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use tokio_util::sync::CancellationToken;

use vak_llm::Provider;
use vak_llm::error::LlmError;
use vak_llm::openai_responses::{OpenAiResponsesConfig, OpenAiResponsesProvider, build_body};
use vak_llm::stream::StreamEvent;
use vak_llm::types::{ChatRequest, ContentBlock, Message, Role, StopReason, ToolDefinition};

fn sample_request() -> ChatRequest {
    let mut req = ChatRequest::new("gpt-5.6");
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
    ];
    req.tools = vec![ToolDefinition::new(
        "bash",
        "Run a shell command",
        serde_json::json!({"type": "object", "properties": {"command": {"type": "string"}}, "required": ["command"]}),
    )];
    req
}

#[test]
fn body_maps_neutral_history_to_responses_shape() {
    let body = build_body(&sample_request()).unwrap();
    assert_eq!(body["model"], "gpt-5.6");
    assert_eq!(body["stream"], true);
    assert_eq!(body["instructions"], "You are a coding agent.");

    let input = body["input"].as_array().unwrap();
    // user msg, function_call item, assistant text item, function_call_output
    assert_eq!(input.len(), 4);
    assert_eq!(input[0]["role"], "user");
    assert_eq!(input[0]["content"][0]["type"], "input_text");

    assert_eq!(input[1]["type"], "function_call");
    assert_eq!(
        input[1]["call_id"], "call_abc",
        "ids survive provider switches"
    );
    assert_eq!(input[1]["name"], "bash");
    let args = input[1]["arguments"].as_str().unwrap();
    assert!(serde_json::from_str::<serde_json::Value>(args).is_ok());

    assert_eq!(input[2]["role"], "assistant");
    assert_eq!(input[2]["content"][0]["type"], "output_text");

    assert_eq!(input[3]["type"], "function_call_output");
    assert_eq!(input[3]["call_id"], "call_abc");

    let tools = body["tools"].as_array().unwrap();
    assert_eq!(tools[0]["type"], "function");
    assert_eq!(tools[0]["name"], "bash");
}

const FIXTURE_STREAM: &str = "\
event: response.output_text.delta\n\
data: {\"type\":\"response.output_text.delta\",\"delta\":\"Read\"}\n\
\n\
event: response.output_text.delta\n\
data: {\"type\":\"response.output_text.delta\",\"delta\":\"ing now.\"}\n\
\n\
event: response.output_item.added\n\
data: {\"type\":\"response.output_item.added\",\"item\":{\"type\":\"function_call\",\"id\":\"fc_1\",\"call_id\":\"call_9\",\"name\":\"read\",\"arguments\":\"\"}}\n\
\n\
event: response.function_call_arguments.delta\n\
data: {\"type\":\"response.function_call_arguments.delta\",\"item_id\":\"fc_1\",\"delta\":\"{\\\"path\\\":\"}\n\
\n\
event: response.function_call_arguments.delta\n\
data: {\"type\":\"response.function_call_arguments.delta\",\"item_id\":\"fc_1\",\"delta\":\"\\\"a.txt\\\"}\"}\n\
\n\
event: response.completed\n\
data: {\"type\":\"response.completed\",\"response\":{\"usage\":{\"input_tokens\":33,\"output_tokens\":9,\"input_tokens_details\":{\"cached_tokens\":4}}}}\n\
\n";

#[tokio::test]
async fn full_stream_accumulates_text_and_tool_calls() {
    let provider = OpenAiResponsesProvider::new(OpenAiResponsesConfig {
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
    // A function_call item means the model wants tools — the loop must
    // continue even though response.completed arrived.
    assert_eq!(msg.stop_reason, StopReason::ToolUse);
    assert_eq!(msg.usage.input_tokens, 33);
    assert_eq!(msg.usage.output_tokens, 9);
    assert_eq!(msg.usage.cache_read_input_tokens, Some(4));

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

const FIXTURE_INCOMPLETE: &str = "\
data: {\"type\":\"response.incomplete\",\"response\":{\"usage\":{\"input_tokens\":1,\"output_tokens\":1}}}\n\
\n";

#[tokio::test]
async fn incomplete_maps_to_max_tokens() {
    let provider = OpenAiResponsesProvider::new(OpenAiResponsesConfig {
        api_key: "k".into(),
        base_url: mock_url(FIXTURE_INCOMPLETE).await,
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
    assert_eq!(msg.stop_reason, StopReason::MaxTokens);
}

#[tokio::test]
async fn http_error_maps_to_typed_value() {
    let url = error_url(401, r#"{"error":{"message":"bad key"}}"#).await;
    let provider = OpenAiResponsesProvider::new(OpenAiResponsesConfig {
        api_key: "bad".into(),
        base_url: url,
    })
    .unwrap();
    let err = provider
        .stream(ChatRequest::new("m"), CancellationToken::new())
        .await
        .err()
        .expect("expected error");
    assert!(matches!(err, LlmError::Auth(_)), "got {err:?}");
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
