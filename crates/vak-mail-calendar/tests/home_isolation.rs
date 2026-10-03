#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

//! A test that opens an Agent vault writes under the data home. Without an
//! isolated home that is the operator's real one: three lease tests once left
//! `mailcal-lease-*` Agents in an installed service's roster, where its health
//! check counted them as verified. This reads the crate's own test sources and
//! fails any test that calls `for_agent` without isolating first. It sees only
//! a test's own body, so a helper that opens a vault must be given the home by
//! its caller.

use std::path::{Path, PathBuf};

fn sources(dir: &Path, out: &mut Vec<PathBuf>) {
    for entry in std::fs::read_dir(dir).expect("read source dir") {
        let path = entry.expect("dir entry").path();
        if path.is_dir() {
            sources(&path, out);
        } else if path.extension().is_some_and(|ext| ext == "rs") {
            out.push(path);
        }
    }
}

fn test_bodies(source: &str) -> Vec<(String, &str)> {
    let mut found = Vec::new();
    let mut from = 0;
    while let Some(at) = source[from..]
        .find("#[test]")
        .or_else(|| source[from..].find("#[tokio::test"))
    {
        let attr = from + at;
        let Some(fn_at) = source[attr..].find("fn ").map(|offset| attr + offset) else {
            break;
        };
        let name: String = source[fn_at + 3..]
            .chars()
            .take_while(|c| c.is_alphanumeric() || *c == '_')
            .collect();
        let Some(open) = source[fn_at..].find('{').map(|offset| fn_at + offset) else {
            break;
        };
        let mut depth = 0usize;
        let mut end = open;
        for (offset, c) in source[open..].char_indices() {
            match c {
                '{' => depth += 1,
                '}' => {
                    depth -= 1;
                    if depth == 0 {
                        end = open + offset + 1;
                        break;
                    }
                }
                _ => {}
            }
        }
        found.push((name, &source[open..end]));
        from = end.max(attr + 1);
    }
    found
}

#[test]
fn every_test_that_opens_a_vault_isolates_its_home_first() {
    let mut files = Vec::new();
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    sources(&root.join("src"), &mut files);
    sources(&root.join("tests"), &mut files);
    let mut leaks = Vec::new();
    let mut vault_tests = 0;
    for file in files {
        let text = std::fs::read_to_string(&file).expect("read source");
        for (name, body) in test_bodies(&text) {
            if !body.contains("for_agent(") {
                continue;
            }
            vault_tests += 1;
            if !body.contains("isolate_home_for_tests") && !body.contains("set_home_override") {
                leaks.push(format!(
                    "{}::{name}",
                    file.strip_prefix(root).unwrap_or(&file).display()
                ));
            }
        }
    }
    assert!(
        vault_tests > 10,
        "the scan found only {vault_tests} vault tests; it is broken"
    );
    assert!(
        leaks.is_empty(),
        "tests that open a vault on the real data home: {leaks:?}"
    );
}
