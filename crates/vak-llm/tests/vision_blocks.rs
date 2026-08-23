#![allow(clippy::unwrap_used, clippy::expect_used)]

use vak_llm::openai::build_body;
use vak_llm::types::ChatRequest;
use vak_llm::types::{ContentBlock, ImageSource, Message, Role};

#[test]
fn image_block_serializes_to_anthropic_native_shape() {
    let msg = Message {
        role: Role::User,
        content: vec![
            ContentBlock::text("what is this?"),
            ContentBlock::image_base64("image/png", "QUJD"),
        ],
    };
    let v = serde_json::to_value(&msg).unwrap();
    assert_eq!(v["content"][0]["type"], "text");
    assert_eq!(v["content"][1]["type"], "image");
    // Anthropic expects the source nested object verbatim.
    assert_eq!(v["content"][1]["source"]["type"], "base64");
    assert_eq!(v["content"][1]["source"]["media_type"], "image/png");
    assert_eq!(v["content"][1]["source"]["data"], "QUJD");

    // Roundtrip through the ledger format.
    let back: Message = serde_json::from_value(v).unwrap();
    assert_eq!(back, msg);
}

#[test]
fn openai_body_builds_multimodal_parts_from_images() {
    let req = ChatRequest {
        messages: vec![Message {
            role: Role::User,
            content: vec![
                ContentBlock::text("describe"),
                ContentBlock::image_base64("image/jpeg", "Zm9v"),
            ],
        }],
        ..ChatRequest::new("test-model")
    };
    let body = build_body(&req).unwrap();
    let content = body["messages"][0]["content"].as_array().unwrap();
    assert_eq!(content[0]["type"], "text");
    assert_eq!(content[1]["type"], "image_url");
    assert_eq!(
        content[1]["image_url"]["url"],
        "data:image/jpeg;base64,Zm9v"
    );
}

#[test]
fn openai_text_only_stays_a_plain_string() {
    let req = ChatRequest {
        messages: vec![Message::user_text("plain")],
        ..ChatRequest::new("test-model")
    };
    let body = build_body(&req).unwrap();
    assert_eq!(body["messages"][0]["content"], "plain");
}

#[test]
fn image_in_assistant_message_rejected_for_anthropic() {
    let req = ChatRequest {
        messages: vec![Message {
            role: Role::Assistant,
            content: vec![ContentBlock::Image {
                source: ImageSource {
                    r#type: "base64".into(),
                    media_type: "image/png".into(),
                    data: "x".into(),
                },
            }],
        }],
        ..ChatRequest::new("test-model")
    };
    assert!(vak_llm::anthropic::build_body(&req).is_err());
}
