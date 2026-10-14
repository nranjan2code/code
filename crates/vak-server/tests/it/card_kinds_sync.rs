#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

//! The forms are one list everywhere (docs/design/92-card-forms.md): the
//! card tools, the delivery validators, the host's primitives and the forms
//! the client draws are the same set. A form one side lacks is a card the
//! model can make and nobody draws, or a drawing nothing can reach.

use std::collections::BTreeSet;
use std::path::PathBuf;

/// The keys of the client's `FORMS` table.
fn client_forms() -> BTreeSet<String> {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../vak-client-ui/src/components/cards/index.tsx");
    let source = std::fs::read_to_string(&path).unwrap();
    let start = source.find("const FORMS").unwrap();
    let end = start + source[start..].find("\n};").unwrap();
    source[start..end]
        .lines()
        .skip(1)
        .filter_map(|line| {
            line.trim()
                .split_once(':')
                .map(|(key, _)| key.trim().to_string())
        })
        .collect()
}

#[test]
fn the_card_kinds_are_exactly_the_types_the_client_draws() {
    let tools: BTreeSet<String> = vak_core::presentation_tools::conformance_cases()
        .into_iter()
        .map(|(_, form, _)| form.to_string())
        .collect();
    let delivery: BTreeSet<String> = vak_delivery::FORMS
        .iter()
        .map(|form| form.to_string())
        .collect();
    let primitives: BTreeSet<String> = vak_presentation::PRIMITIVES
        .iter()
        .filter_map(|primitive| {
            serde_json::to_value(primitive)
                .ok()?
                .as_str()
                .map(str::to_string)
        })
        .filter(|name| {
            !matches!(
                name.as_str(),
                "stack" | "row" | "group" | "section" | "text" | "metric"
            )
        })
        .collect();
    let client = client_forms();
    assert_eq!(tools, delivery, "card tools and delivery forms differ");
    assert_eq!(primitives, delivery, "host primitives and forms differ");
    assert_eq!(
        client, delivery,
        "the client draws a different set of forms"
    );
}
