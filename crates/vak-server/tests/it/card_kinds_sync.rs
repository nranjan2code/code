#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

//! Every card type the client draws is a kind some `answer` block produces,
//! and every kind a block produces is drawn: the two sets are equal. Types
//! the client could draw but no block produced were how 66 card types went
//! out of the model's reach unnoticed
//! (docs/audits/cards-in-answers-2026-10-11.md, F1).

use std::collections::BTreeSet;
use std::path::PathBuf;

/// The top-level keys of the client's `STRUCTURED_RENDERERS` registry.
fn client_types() -> BTreeSet<String> {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../vak-client-ui/src/components/PresentationRenderer.tsx");
    let source = std::fs::read_to_string(&path).unwrap();
    let start = source.find("const STRUCTURED_RENDERERS").unwrap();
    let end = source.find("export const structuredRendererTypes").unwrap();
    source[start..end]
        .lines()
        .filter(|line| line.starts_with("  ") && !line.starts_with("   "))
        .filter_map(|line| {
            let (key, _) = line.trim().split_once(':')?;
            let key = key.trim_matches('"');
            key.chars()
                .all(|c| c.is_ascii_lowercase() || c == '_' || c == '.')
                .then(|| key.to_string())
        })
        .filter(|key| !key.is_empty())
        .collect()
}

#[test]
fn the_card_kinds_are_exactly_the_types_the_client_draws() {
    let kinds: BTreeSet<String> = vak_core::presentation_tools::conformance_cases()
        .into_iter()
        .map(|(_, kind, _)| kind.to_string())
        .collect();
    let client = client_types();
    assert!(client.len() > 100, "parsed {} client types", client.len());
    let undrawn: Vec<_> = kinds.difference(&client).collect();
    let unreachable: Vec<_> = client.difference(&kinds).collect();
    assert!(
        undrawn.is_empty() && unreachable.is_empty(),
        "kinds the client does not draw: {undrawn:?}\ntypes no block produces: {unreachable:?}"
    );
}
