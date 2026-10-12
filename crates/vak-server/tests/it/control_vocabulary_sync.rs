#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

//! The desktop client strips inline runtime hints and context blocks from text
//! it renders. It cannot import the Rust vocabulary, so this fails the build
//! if its two lists differ from `vak_intent::control` — the drift that, with
//! a marker added on one side only, once displaced every later chat turn.
//! Exact set equality, not "contains": a stale extra entry is drift too.

use std::collections::BTreeSet;
use std::path::PathBuf;

fn client_source(file: &str) -> String {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../vak-client-ui/src")
        .join(file);
    std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("read {}: {e}", path.display()))
}

/// The string literals of `export const NAME = [ ... ]` in the client source.
fn ts_string_list(source: &str, name: &str) -> BTreeSet<String> {
    let start = source
        .find(&format!("export const {name} = ["))
        .unwrap_or_else(|| panic!("structured.ts must declare `export const {name} = [...]`"));
    let body = &source[start..];
    let open = body.find('[').unwrap() + 1;
    // Not the first `]`: the marker literals themselves contain brackets.
    let close = body[open..].find("] as const").unwrap() + open;
    body[open..close]
        .split(',')
        .filter_map(|item| {
            let item = item.trim();
            item.strip_prefix('"')
                .and_then(|rest| rest.strip_suffix('"'))
                .map(str::to_string)
        })
        .collect()
}

#[test]
fn the_clients_inline_hint_list_is_exactly_the_runtimes() {
    let client = ts_string_list(&client_source("structured.ts"), "INLINE_HINT_MARKERS");
    let runtime: BTreeSet<String> = vak_intent::control::inline_markers()
        .into_iter()
        .map(str::to_string)
        .collect();
    assert_eq!(
        client, runtime,
        "structured.ts INLINE_HINT_MARKERS differs from vak_intent::control::inline_markers()"
    );
}

#[test]
fn the_clients_context_block_tags_are_exactly_the_runtimes() {
    let client = ts_string_list(&client_source("structured.ts"), "CONTEXT_BLOCK_TAGS");
    let runtime: BTreeSet<String> = vak_intent::control::CONTEXT_BLOCK_TAGS
        .into_iter()
        .map(str::to_string)
        .collect();
    assert_eq!(
        client, runtime,
        "structured.ts CONTEXT_BLOCK_TAGS differs from vak_intent::control::CONTEXT_BLOCK_TAGS"
    );
}
