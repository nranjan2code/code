#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
use serde_json::json;
use vak_tools::{ProgressTool, Tool, parse_progress_args};

#[test]
fn progress_schema_is_strict_flat_and_every_field_is_described() {
    let schema = ProgressTool.schema();
    assert_eq!(schema["additionalProperties"], false);
    let items = &schema["properties"]["items"];
    assert_eq!(items["type"], "array");
    assert!(
        items["description"]
            .as_str()
            .is_some_and(|text| !text.is_empty())
    );
    let step = &items["items"];
    assert_eq!(step["additionalProperties"], false);
    for (name, field) in step["properties"].as_object().unwrap() {
        assert!(field.get("type").is_some(), "{name}");
        assert!(
            field["description"]
                .as_str()
                .is_some_and(|text| !text.is_empty()),
            "{name}"
        );
        assert!(field.get("oneOf").is_none() && field.get("anyOf").is_none());
        assert!(step["required"].as_array().unwrap().contains(&json!(name)));
    }
    assert!(ProgressTool.always_loaded());
    assert!(!ProgressTool.presents_cards());
    assert!(!ProgressTool.looks_up());
    assert!(!ProgressTool.retrieves_evidence(&json!({"items":[]})));
    assert!(
        parse_progress_args(
            &json!({"items":[{"label":"Check source","state":"dropped","reason":null}]})
        )
        .is_err()
    );
    assert!(parse_progress_args(&json!({"items":[{"label":"Check source","state":"dropped","reason":"Source unavailable"}]})).is_ok());
    assert!(parse_progress_args(&json!({"items":[],"deliverable":"old"})).is_err());
}
