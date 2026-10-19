//! A new defaulted Tool declaration must not silently disappear at a wrapper.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use regex::Regex;
use std::collections::BTreeMap;

fn block<'a>(source: &'a str, header: &str) -> &'a str {
    source
        .split_once(header)
        .expect("contract source header")
        .1
        .split_once("\n}\n")
        .expect("contract source block end")
        .0
}

// Rustfmt puts trait/impl members at this indentation. Inspect the source
// contract so adding a trait method fails even when Rust supplies a default.
fn methods(source: &str) -> BTreeMap<String, &str> {
    let pattern = Regex::new(r"(?m)^    (?:async )?fn ([a-zA-Z_][a-zA-Z_0-9]*)\(").unwrap();
    let matches: Vec<_> = pattern.captures_iter(source).collect();
    matches
        .iter()
        .enumerate()
        .map(|(index, captures)| {
            let start = captures.get(0).unwrap().start();
            let end = matches
                .get(index + 1)
                .map_or(source.len(), |next| next.get(0).unwrap().start());
            (captures[1].to_owned(), &source[start..end])
        })
        .collect()
}

#[test]
fn wrappers_forward_every_tool_declaration() {
    let declarations = methods(block(
        include_str!("../src/lib.rs"),
        "pub trait Tool: Send + Sync {",
    ));
    assert!(declarations.contains_key("fill_defaults"));
    assert!(declarations.contains_key("execute"));
    for (name, source) in [
        ("BrokeredTool", include_str!("../src/broker.rs")),
        (
            "ScopedWriteTool",
            include_str!("../../vak-agent/src/write_lease.rs"),
        ),
    ] {
        let members = methods(block(source, &format!("impl Tool for {name} {{")));
        assert_eq!(
            members.keys().collect::<Vec<_>>(),
            declarations.keys().collect::<Vec<_>>(),
            "{name} must explicitly implement every Tool member"
        );
        for (method, body) in &members {
            // Execution intentionally crosses the broker or checks the lease.
            // Every declaration must reach the wrapped implementation.
            if method != "execute" {
                assert!(
                    body.contains(&format!("self.inner.{method}(")),
                    "{name}::{method} must forward the wrapped declaration"
                );
            }
        }
    }
}
