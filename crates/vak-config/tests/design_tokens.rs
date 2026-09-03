#![allow(clippy::unwrap_used, clippy::expect_used)]

//! Guards the design system's own rule: "keep the desktop and admin surfaces
//! on the same token set — the admin UI is explicitly built to match
//! vak-desktop, not to diverge stylistically."
//!
//! Both UIs declared that set independently, by copy-paste, and had already
//! drifted: `--faint` differed, `--accent-wash` differed in the third decimal,
//! the same status role was `--yellow` in one and `--amber` in the other, and
//! the admin had accumulated dead tokens (`--purple`, `--text-dim`) that
//! nothing referenced and that broke the fixed-status-palette rule.
//!
//! This lives in vak-config rather than a JS test because the workspace has no
//! frontend test runner, and a guard nobody runs is not a guard.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

// One client, two hosts (docs/design/48-web-client.md): the workspace
// client moved out of vak-desktop so the browser build could share it.
const DESKTOP: &str = "crates/vak-client-ui/src/styles.css";
const ADMIN: &str = "crates/vak-admin-ui/src/styles.css";

/// Tokens both surfaces define and must agree on. Surface-specific additions
/// (the admin's `--border-strong`, the desktop's alternate themes) are allowed
/// and simply absent here.
const SHARED: &[&str] = &[
    "bg",
    "surface",
    "surface-raised",
    "surface-hover",
    "surface-active",
    "sidebar",
    "border",
    "border-soft",
    "text",
    "text-soft",
    "muted",
    "faint",
    "accent",
    "accent-bright",
    "accent-wash",
    "green",
    "yellow",
    "red",
    "blue",
    "mono",
    "sans",
    "radius-sm",
    "radius",
    "radius-lg",
    "shadow-lg",
];

fn root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .ancestors()
        .nth(2)
        .expect("workspace root")
        .to_path_buf()
}

/// Values from the first `:root { … }` block — the light/default declaration.
/// Later theme blocks legitimately override, and are not compared.
fn base_tokens(css: &str) -> BTreeMap<String, String> {
    let start = css.find(":root").expect(":root block");
    let body = &css[start..];
    let end = body.find('}').expect("closing brace");
    body[..end]
        .lines()
        .filter_map(|line| {
            let line = line.trim().trim_end_matches(';');
            let (name, value) = line.split_once(':')?;
            let name = name.trim().strip_prefix("--")?;
            Some((name.to_string(), value.trim().to_string()))
        })
        .collect()
}

fn read(relative: &str) -> BTreeMap<String, String> {
    base_tokens(&std::fs::read_to_string(root().join(relative)).expect(relative))
}

#[test]
fn both_surfaces_agree_on_every_shared_token() {
    let desktop = read(DESKTOP);
    let admin = read(ADMIN);

    let mut drift = Vec::new();
    for token in SHARED {
        match (desktop.get(*token), admin.get(*token)) {
            (Some(d), Some(a)) if d != a => {
                drift.push(format!("--{token}: desktop {d} != admin {a}"));
            }
            (None, _) => drift.push(format!("--{token}: missing from {DESKTOP}")),
            (_, None) => drift.push(format!("--{token}: missing from {ADMIN}")),
            _ => {}
        }
    }
    assert!(
        drift.is_empty(),
        "the two surfaces have drifted apart:\n  {}",
        drift.join("\n  ")
    );
}

/// A status role must have one name. The admin called warning `--amber` while
/// the desktop called it `--yellow`, which is how a shared palette silently
/// becomes two palettes.
#[test]
fn status_roles_use_one_vocabulary() {
    for (label, path) in [("desktop", DESKTOP), ("admin", ADMIN)] {
        let css = std::fs::read_to_string(root().join(path)).expect(path);
        assert!(
            !css.contains("--amber"),
            "{label} still defines --amber; the shared name for that role is --yellow"
        );
    }
}

/// Every token defined must be referenced. `--purple` broke the "fixed status
/// set, no second accent" rule while being used by nothing at all.
#[test]
fn no_token_is_defined_and_never_used() {
    for (label, dir, css_path) in [
        ("desktop", "crates/vak-client-ui/src", DESKTOP),
        ("admin", "crates/vak-admin-ui/src", ADMIN),
    ] {
        let defined = read(css_path);
        let mut sources = String::new();
        collect_sources(&root().join(dir), &mut sources);

        let dead: Vec<&String> = defined
            .keys()
            .filter(|name| !sources.contains(&format!("var(--{name})")))
            .collect();
        assert!(
            dead.is_empty(),
            "{label} defines tokens nothing references: {dead:?} — \
             delete them or use them"
        );
    }
}

fn collect_sources(dir: &Path, out: &mut String) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            collect_sources(&path, out);
        } else if matches!(
            path.extension().and_then(|e| e.to_str()),
            Some("css" | "ts" | "tsx")
        ) && let Ok(text) = std::fs::read_to_string(&path)
        {
            out.push_str(&text);
        }
    }
}

/// DESIGN.md is the written spec; if the code and the document disagree, one
/// of them is lying to whoever reads it next.
#[test]
fn design_md_matches_the_shipped_palette() {
    let design = std::fs::read_to_string(root().join("DESIGN.md")).expect("DESIGN.md");
    let desktop = read(DESKTOP);
    // DESIGN.md names the accent by its colour rather than its role.
    let documented = [
        ("bg", "bg"),
        ("surface", "surface"),
        ("surface-raised", "surface-raised"),
        ("text", "text"),
        ("muted", "muted"),
        ("faint", "faint"),
        ("green", "green"),
        ("yellow", "yellow"),
        ("red", "red"),
        ("blue", "blue"),
    ];
    let mut wrong = Vec::new();
    for (doc_key, token) in documented {
        let shipped = desktop.get(token).expect(token);
        let needle = format!("{doc_key}: \"{shipped}\"");
        if !design.contains(&needle) {
            wrong.push(format!("DESIGN.md does not record {doc_key} as {shipped}"));
        }
    }
    assert!(wrong.is_empty(), "{}", wrong.join("\n  "));
}

/// `--faint` is mandated by DESIGN.md for *all* placeholder text, timestamps,
/// hints, badges, and tooltips, so a failing value fails systematically. It
/// must clear WCAG AA against the darkest surface it is painted on.
#[test]
fn faint_clears_wcag_aa_on_every_surface_it_sits_on() {
    let desktop = read(DESKTOP);
    let faint = desktop.get("faint").expect("--faint");
    for ground in ["bg", "surface", "surface-raised"] {
        let behind = desktop.get(ground).expect(ground);
        let ratio = contrast(faint, behind);
        assert!(
            ratio >= 4.5,
            "--faint {faint} on --{ground} {behind} is {ratio:.2}:1, below WCAG AA (4.5:1)"
        );
    }
}

fn channel(value: f64) -> f64 {
    let value = value / 255.0;
    if value <= 0.04045 {
        value / 12.92
    } else {
        ((value + 0.055) / 1.055).powf(2.4)
    }
}

fn luminance(hex: &str) -> f64 {
    let hex = hex.trim().trim_start_matches('#');
    let parse = |i: usize| u8::from_str_radix(&hex[i..i + 2], 16).expect("hex pair") as f64;
    0.2126 * channel(parse(0)) + 0.7152 * channel(parse(2)) + 0.0722 * channel(parse(4))
}

fn contrast(a: &str, b: &str) -> f64 {
    let (x, y) = (luminance(a), luminance(b));
    let (hi, lo) = if x > y { (x, y) } else { (y, x) };
    (hi + 0.05) / (lo + 0.05)
}
