#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
//! The home-path ratchet (`docs/plans/data-architecture-plan.md` §4, "Now").
//!
//! Until M3a replaces raw home-path calls with typed scope accessors, this
//! holds their number where it is. A new raw call fails here; a removed one
//! must lower the ceiling in `home_path_ratchet.txt` in the same change, so
//! the counts only ever go down. M3a deletes this test when they reach zero.
//!
//! The method is blast-radius §0's: every `.rs` file under `crates/`
//! outside `tests/` and `benches/` directories, with `#[cfg(test)]` items
//! removed by brace matching and `//` comment lines skipped.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

/// What is counted, by the name the ceiling file uses.
const MEASURES: &[(&str, &[&str])] = &[
    ("sessions_home", &["sessions_home()"]),
    ("shared_data_home", &["shared_data_home()"]),
    ("vak_literal", &["\".vak/", "\".vak\""]),
    ("hash_cwd", &["hash_cwd("]),
];

const SKIPPED_DIRS: &[&str] = &["tests", "benches", "target", "node_modules", "dist"];

fn rust_sources(dir: &Path, out: &mut Vec<PathBuf>) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        let Ok(kind) = entry.file_type() else {
            continue;
        };
        if kind.is_dir() {
            let name = entry.file_name();
            if !SKIPPED_DIRS.iter().any(|skip| name == *skip) {
                rust_sources(&path, out);
            }
        } else if path.extension().is_some_and(|ext| ext == "rs") {
            out.push(path);
        }
    }
}

/// The lines of `text` outside `#[cfg(test)]` items and `//` comments.
fn production_lines(text: &str) -> Vec<&str> {
    let lines: Vec<&str> = text.lines().collect();
    let mut kept = Vec::new();
    let mut i = 0;
    while i < lines.len() {
        if lines[i].trim_start().starts_with("#[cfg(test)]") {
            let mut j = i + 1;
            let mut depth = 0i64;
            let mut started = false;
            while j < lines.len() {
                for ch in lines[j].chars() {
                    match ch {
                        '{' => {
                            depth += 1;
                            started = true;
                        }
                        '}' => depth -= 1,
                        _ => {}
                    }
                }
                if started && depth <= 0 {
                    break;
                }
                if !started && lines[j].trim_end().ends_with(';') {
                    break;
                }
                j += 1;
            }
            i = j + 1;
            continue;
        }
        if !lines[i].trim_start().starts_with("//") {
            kept.push(lines[i]);
        }
        i += 1;
    }
    kept
}

/// Per measure: the total, and the files that contribute to it.
fn measure(crates: &Path) -> BTreeMap<&'static str, (usize, BTreeMap<String, usize>)> {
    let mut files = Vec::new();
    rust_sources(crates, &mut files);
    files.sort();
    let mut counts: BTreeMap<&'static str, (usize, BTreeMap<String, usize>)> = MEASURES
        .iter()
        .map(|(name, _)| (*name, (0, BTreeMap::new())))
        .collect();
    for file in files {
        let text = std::fs::read_to_string(&file).unwrap_or_default();
        let lines = production_lines(&text);
        let shown = file
            .strip_prefix(crates)
            .unwrap_or(&file)
            .display()
            .to_string();
        for (name, needles) in MEASURES {
            let n: usize = lines
                .iter()
                .map(|line| {
                    needles
                        .iter()
                        .map(|needle| line.matches(needle).count())
                        .sum::<usize>()
                })
                .sum();
            if n > 0 {
                let entry = counts.get_mut(name).unwrap();
                entry.0 += n;
                entry.1.insert(shown.clone(), n);
            }
        }
    }
    counts
}

fn ceilings(path: &Path) -> BTreeMap<String, usize> {
    let text = std::fs::read_to_string(path).expect("home_path_ratchet.txt");
    text.lines()
        .map(str::trim)
        .filter(|line| !line.is_empty() && !line.starts_with('#'))
        .map(|line| {
            let (name, value) = line
                .split_once('=')
                .unwrap_or_else(|| panic!("`{line}` is not `name = count`"));
            let value = value
                .trim()
                .parse()
                .unwrap_or_else(|_| panic!("`{line}` has no count"));
            (name.trim().to_string(), value)
        })
        .collect()
}

#[test]
fn home_path_uses_do_not_grow() {
    let manifest = Path::new(env!("CARGO_MANIFEST_DIR"));
    let crates = manifest.parent().expect("crates/");
    let ceiling_file = manifest.join("tests/home_path_ratchet.txt");
    let ceilings = ceilings(&ceiling_file);
    let measured = measure(crates);

    let mut problems = Vec::new();
    for (name, (count, by_file)) in &measured {
        let Some(&ceiling) = ceilings.get(*name) else {
            problems.push(format!("`{name}` has no ceiling in home_path_ratchet.txt"));
            continue;
        };
        if *count > ceiling {
            let mut largest: Vec<_> = by_file.iter().collect();
            largest.sort_by(|a, b| b.1.cmp(a.1).then(a.0.cmp(b.0)));
            let largest: Vec<String> = largest
                .iter()
                .take(8)
                .map(|(file, n)| format!("{file} {n}"))
                .collect();
            problems.push(format!(
                "`{name}`: {count} raw uses, ceiling {ceiling}. Reach the home through an \
                 existing helper instead of adding a raw call; M3a replaces them all with \
                 typed scope accessors. Largest files: {}",
                largest.join(", ")
            ));
        } else if *count < ceiling {
            problems.push(format!(
                "`{name}`: {count} raw uses, below the ceiling of {ceiling}. Lower it to \
                 `{name} = {count}` in crates/vak-core/tests/home_path_ratchet.txt so the \
                 ratchet keeps the ground gained"
            ));
        }
    }
    assert!(problems.is_empty(), "{}", problems.join("\n"));
}

#[test]
fn production_lines_drop_test_items_and_comments() {
    let text = "fn a() { x.sessions_home(); }\n\
                // y.sessions_home();\n\
                #[cfg(test)]\n\
                mod tests {\n    fn b() { z.sessions_home(); }\n}\n\
                #[cfg(test)]\n\
                use something;\n\
                fn c() { w.sessions_home(); }\n";
    let kept = production_lines(text);
    let hits: usize = kept
        .iter()
        .map(|l| l.matches("sessions_home()").count())
        .sum();
    assert_eq!(hits, 2, "{kept:?}");
}
