//! Finding and reading the `vak` fences in an assistant's answer.
//!
//! A `vak` fence is the model writing a card envelope inline. These helpers
//! find them the way a Markdown renderer does (so an indented fence, a `~~~`
//! fence, and a `vak` block quoted inside a longer fence are all handled
//! correctly) and read `semantic_type` as a JSON field, not as a substring of
//! the text.

use pulldown_cmark::{CodeBlockKind, Event, Parser, Tag, TagEnd};

/// The body of every fenced block whose info string is `vak`, in order. An
/// unclosed fence runs to the end of the text, exactly as a renderer would
/// treat it — which is what an answer cut off mid-card looks like.
pub(crate) fn vak_fence_bodies(text: &str) -> Vec<String> {
    let mut bodies = Vec::new();
    let mut current: Option<String> = None;
    for event in Parser::new(text) {
        match event {
            Event::Start(Tag::CodeBlock(CodeBlockKind::Fenced(info)))
                if info.split_whitespace().next() == Some("vak") =>
            {
                current = Some(String::new());
            }
            Event::Text(chunk) => {
                if let Some(body) = current.as_mut() {
                    body.push_str(&chunk);
                }
            }
            Event::End(TagEnd::CodeBlock) => {
                if let Some(body) = current.take() {
                    bodies.push(body);
                }
            }
            _ => {}
        }
    }
    bodies
}

/// The parse error of the first `vak` fence whose body is not valid JSON. A
/// fence tagged `vak` is by definition an attempt at an envelope, so no
/// further guess is made about whether it "looks like" one.
pub(crate) fn find_malformed_vak_fence(text: &str) -> Option<String> {
    vak_fence_bodies(text).into_iter().find_map(|body| {
        serde_json::from_str::<serde_json::Value>(body.trim())
            .err()
            .map(|error| error.to_string())
    })
}

/// The `semantic_type` of the first `vak` fence that repeats a card already
/// emitted through a tool call this run, if any. Compares the parsed
/// `semantic_type` field; a card whose payload merely mentions another type
/// in a string does not count.
pub(crate) fn find_duplicate_card_fence(text: &str, emitted_types: &[String]) -> Option<String> {
    vak_fence_bodies(text).into_iter().find_map(|body| {
        let value: serde_json::Value = serde_json::from_str(body.trim()).ok()?;
        let declared = value.get("semantic_type")?.as_str()?;
        emitted_types
            .iter()
            .find(|emitted| emitted.as_str() == declared)
            .cloned()
    })
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    const CHART: &str = r#"{"semantic_type":"chart","chart_type":"line","points":[]}"#;

    fn fenced(body: &str) -> String {
        format!("Here you go.\n\n```vak\n{body}\n```\n")
    }

    #[test]
    fn a_valid_fence_is_found_and_not_malformed() {
        let text = fenced(CHART);
        assert_eq!(vak_fence_bodies(&text).len(), 1);
        assert_eq!(find_malformed_vak_fence(&text), None);
    }

    #[test]
    fn an_invalid_body_is_reported_with_its_parse_error() {
        let text = fenced(r#"{"semantic_type":"chart","points":[}"#);
        assert!(find_malformed_vak_fence(&text).is_some());
    }

    #[test]
    fn an_answer_cut_off_mid_card_is_malformed() {
        let text = "Result:\n\n```vak\n{\"semantic_type\":\"chart\",\"payload\":{\"ser";
        assert!(
            find_malformed_vak_fence(text).is_some(),
            "unclosed fence is a truncated card"
        );
    }

    #[test]
    fn a_vak_block_quoted_inside_a_longer_fence_is_not_a_fence() {
        let text = "Example of the format:\n\n````markdown\n```vak\n{not json\n```\n````\n";
        assert!(vak_fence_bodies(text).is_empty());
        assert_eq!(find_malformed_vak_fence(text), None);
    }

    #[test]
    fn tilde_and_indented_fences_are_found_like_a_renderer_would() {
        let tilde = format!("~~~vak\n{CHART}\n~~~\n");
        assert_eq!(vak_fence_bodies(&tilde).len(), 1);
        let indented = format!("- item\n\n  ```vak\n  {CHART}\n  ```\n");
        assert_eq!(vak_fence_bodies(&indented).len(), 1);
    }

    #[test]
    fn other_languages_are_not_vak_fences() {
        let text = "```json\n{not json\n```\n";
        assert!(vak_fence_bodies(text).is_empty());
    }

    #[test]
    fn a_duplicate_is_decided_by_the_parsed_type_not_by_a_substring() {
        let emitted = vec!["chart".to_string()];
        assert_eq!(
            find_duplicate_card_fence(&fenced(CHART), &emitted),
            Some("chart".into())
        );
        // A different card whose *text* mentions the chart envelope is not one.
        let metric = r#"{"semantic_type":"metric","label":"say \"semantic_type\":\"chart\" here","value":"1"}"#;
        assert_eq!(find_duplicate_card_fence(&fenced(metric), &emitted), None);
        assert_eq!(find_duplicate_card_fence("no fence at all", &emitted), None);
    }
}
