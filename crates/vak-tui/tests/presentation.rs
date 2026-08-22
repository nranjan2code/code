use crossterm::style::Color;
use vak_tui::diffview::unified;
use vak_tui::markdown::{LineStyler, highlight};
use vak_tui::status::{fmt_elapsed, fmt_tokens, frame};
use vak_tui::theme::{Theme, names};

const RESET_FG: &str = "\x1b[39m";

fn plain(s: &str) -> String {
    let mut out = String::new();
    let mut chars = s.chars();
    while let Some(c) = chars.next() {
        if c == '\x1b' {
            for terminator in chars.by_ref() {
                if terminator.is_ascii_alphabetic() {
                    break;
                }
            }
        } else {
            out.push(c);
        }
    }
    out
}

#[test]
fn theme_dark_preset_fields() {
    let dark = Theme::from_name("dark");
    assert_eq!(dark.accent, Color::Cyan);
    assert_eq!(dark.dim, Color::DarkGrey);
    assert_eq!(dark.success, Color::DarkGreen);
    assert_eq!(dark.error, Color::Red);
    assert_eq!(dark.warning, Color::Yellow);
    assert_eq!(dark.heading, Color::White);
    assert_eq!(dark.code, Color::Magenta);
    assert_eq!(dark.user, Color::White);
    assert_eq!(dark.spinner, Color::Cyan);
}

#[test]
fn theme_plain_is_all_reset() {
    let p = Theme::from_name("plain");
    let all = [
        p.accent, p.dim, p.success, p.error, p.warning, p.heading, p.code, p.user, p.spinner,
    ];
    assert!(all.iter().all(|&c| c == Color::Reset));
}

#[test]
fn theme_light_differs_from_dark_and_unknown_falls_back() {
    let dark = Theme::from_name("dark");
    let light = Theme::from_name("light");
    assert_ne!(light.accent, dark.accent);
    assert_ne!(light.dim, dark.dim);
    assert_ne!(light.heading, dark.heading);
    assert_eq!(Theme::from_name("nope"), dark);
    assert_eq!(names(), &["dark", "light", "plain"]);
}

#[test]
fn styler_tracks_open_fence_highlight_and_close() {
    let t = Theme::from_name("dark");
    let mut styler = LineStyler::new();

    assert_eq!(
        styler.line("```rust", &t),
        format!("\x1b[90m```rust{RESET_FG}")
    );

    let code = styler.line("let s = \"hi\";", &t);
    assert!(code.contains("\x1b[96mlet\x1b[39m"));
    assert!(code.contains("\x1b[32m\"hi\"\x1b[39m"));
    assert_eq!(plain(&code), "let s = \"hi\";");

    assert_eq!(styler.line("```", &t), format!("\x1b[90m```{RESET_FG}"));

    let after = styler.line("# Title", &t);
    assert!(after.starts_with("\x1b[1m"));
    assert_eq!(plain(&after), "Title");
}

#[test]
fn styler_keeps_highlighting_unclosed_fence() {
    let t = Theme::from_name("dark");
    let mut styler = LineStyler::new();

    assert_eq!(
        styler.line("```python", &t),
        format!("\x1b[90m```python{RESET_FG}")
    );

    let l1 = styler.line("# keep alive", &t);
    assert_eq!(l1, format!("\x1b[90m# keep alive{RESET_FG}"));

    let l2 = styler.line("for i in range(10):", &t);
    assert!(l2.contains("\x1b[96mfor\x1b[39m"));
    assert!(l2.contains("\x1b[93m10\x1b[39m"));

    let l3 = styler.line("plainword = 7", &t);
    assert!(l3.contains("\x1b[93m7\x1b[39m"));
    assert!(!l3.contains("\x1b[96mplainword\x1b[39m"));
}

#[test]
fn styler_styles_block_elements() {
    let t = Theme::from_name("dark");
    let mut styler = LineStyler::new();

    let h = styler.line("## Heading text", &t);
    assert!(h.starts_with("\x1b[1m\x1b[97m"));
    assert!(h.ends_with("\x1b[22m"));
    assert_eq!(plain(&h), "Heading text");

    let b = styler.line("- item body", &t);
    assert!(b.contains("\x1b[90m•\x1b[39m"));
    assert_eq!(plain(&b), "• item body");

    let n = styler.line("12. numbered entry", &t);
    assert!(n.contains("\x1b[96m12. \x1b[39m"));
    assert_eq!(plain(&n), "12. numbered entry");

    let q = styler.line("> quoted words", &t);
    assert!(q.contains("\x1b[3m"));
    assert!(q.ends_with("\x1b[23m"));
    assert!(q.contains("\x1b[90mquoted words\x1b[39m"));
    assert_eq!(plain(&q), "quoted words");

    assert_eq!(plain(&styler.line("---", &t)), "───");
    assert_eq!(plain(&styler.line("***", &t)), "───");
}

#[test]
fn styler_styles_inline_spans() {
    let t = Theme::from_name("dark");
    let mut styler = LineStyler::new();

    let code = styler.line("run `cargo test` now", &t);
    assert!(code.contains("\x1b[95mcargo test\x1b[39m"));
    assert_eq!(plain(&code), "run cargo test now");

    let bold = styler.line("**bold** tail", &t);
    assert!(bold.contains("\x1b[1mbold\x1b[22m"));
    assert_eq!(plain(&bold), "**bold** tail".replace('*', ""));

    let italic = styler.line("*ital* rest", &t);
    assert!(italic.contains("\x1b[3mital\x1b[23m"));
    assert_eq!(plain(&italic), "ital rest");

    let unmatched_star = styler.line("a * b * c", &t);
    assert!(!unmatched_star.contains('\x1b'));
    assert_eq!(unmatched_star, "a * b * c");

    let lone_backtick = styler.line("tick ` mark", &t);
    assert!(!lone_backtick.contains('\x1b'));
    assert_eq!(lone_backtick, "tick ` mark");
}

#[test]
fn highlight_rust_line_colors_keywords_strings_numbers_comments() {
    let t = Theme::from_name("dark");

    let src = "fn main() { // entry";
    let out = highlight(src, "rust", &t);
    assert!(out.contains("\x1b[96mfn\x1b[39m"));
    assert!(out.contains("\x1b[90m// entry\x1b[39m"));
    assert!(!out.contains("\x1b[96mmain\x1b[39m"));
    assert_eq!(plain(&out), src);

    let num = highlight("x = 42", "rust", &t);
    assert!(num.contains("\x1b[93m42\x1b[39m"));
    assert_eq!(plain(&num), "x = 42");
}

#[test]
fn highlight_leaves_unknown_words_unstyled() {
    let t = Theme::from_name("dark");
    assert_eq!(highlight("foobar baz_qux", "rust", &t), "foobar baz_qux");
    assert_eq!(highlight("// stays literal", "", &t), "// stays literal");
}

#[test]
fn highlight_keyword_sampling_across_sorted_list() {
    let t = Theme::from_name("dark");
    for kw in [
        "abstract",
        "crate",
        "defer",
        "enum",
        "finally",
        "goto",
        "instanceof",
        "loop",
        "namespace",
        "override",
        "protected",
        "return",
        "sizeof",
        "trait",
        "unsafe",
        "virtual",
        "where",
        "yield",
    ] {
        assert_eq!(
            highlight(kw, "rust", &t),
            format!("\x1b[96m{kw}\x1b[39m"),
            "{kw}"
        );
    }
}

#[test]
fn diffview_colors_deletions_additions_and_headers() {
    let t = Theme::from_name("dark");
    let out = unified("a\nb\nc\n", "a\nB\nc\n", &t, 50);
    assert!(out.starts_with("@@ -1,3 +1,3 @@"));
    assert!(out.contains("\x1b[91m- b\x1b[39m"));
    assert!(out.contains("\x1b[32m+ B\x1b[39m"));
    assert!(out.contains('\n'));
    assert!(!out.ends_with('\n'));
    assert!(plain(&out).contains('a'));
}

#[test]
fn diffview_truncates_with_more_notice() {
    let t = Theme::from_name("dark");
    let base: String = (1..=20)
        .map(|i| format!("line{i}\n"))
        .collect::<Vec<_>>()
        .concat();
    let extended: String = (21..=25)
        .map(|i| format!("line{i}\n"))
        .collect::<Vec<_>>()
        .concat();
    let new = format!("{base}{extended}");

    let full = unified(&base, &new, &t, 100);
    assert_eq!(full.lines().count(), 9);

    let capped = unified(&base, &new, &t, 6);
    assert_eq!(capped.lines().count(), 6);
    assert!(capped.contains("(+4 more)"));
    assert!(capped.ends_with("(+4 more)\x1b[39m"));
}

#[test]
fn spinner_frames_wrap() {
    assert_eq!(frame(0), "⠋");
    assert_eq!(frame(3), "⠸");
    assert_eq!(frame(10), "⠋");
    assert_eq!(frame(13), "⠸");
}

#[test]
fn elapsed_formatting_edges() {
    assert_eq!(fmt_elapsed(0), "0s");
    assert_eq!(fmt_elapsed(12), "12s");
    assert_eq!(fmt_elapsed(59), "59s");
    assert_eq!(fmt_elapsed(60), "1m00s");
    assert_eq!(fmt_elapsed(63), "1m03s");
    assert_eq!(fmt_elapsed(3599), "59m59s");
    assert_eq!(fmt_elapsed(3600), "1h00m");
    assert_eq!(fmt_elapsed(3720), "1h02m");
}

#[test]
fn token_formatting_edges() {
    assert_eq!(fmt_tokens(0), "0");
    assert_eq!(fmt_tokens(900), "900");
    assert_eq!(fmt_tokens(999), "999");
    assert_eq!(fmt_tokens(1_000), "1k");
    assert_eq!(fmt_tokens(1_500), "1.5k");
    assert_eq!(fmt_tokens(12_400), "12.4k");
    assert_eq!(fmt_tokens(1_000_000), "1M");
    assert_eq!(fmt_tokens(1_500_000), "1.5M");
    assert_eq!(fmt_tokens(3_200_000), "3.2M");
}
