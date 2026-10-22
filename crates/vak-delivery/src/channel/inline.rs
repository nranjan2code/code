//! Authored prose formatting is syntax, never a topic/layout classifier.
//! Flatten inline styles so provider nesting has a fixed structural depth.
use pulldown_cmark::{Event, Options, Parser, Tag, TagEnd};

#[derive(Clone, Debug, Default)]
pub(super) struct Run {
    pub text: String,
    pub bold: bool,
    pub italic: bool,
    pub strike: bool,
    pub code: bool,
    pub url: Option<String>,
}
pub(super) fn link_allowed(url: &str) -> bool {
    crate::safe_link(url)
        && !url.chars().any(char::is_control)
        && (url.starts_with("https://") || url.starts_with("http://") || url.starts_with("mailto:"))
}
pub(super) fn runs(source: &str) -> Vec<Run> {
    let mut out = Vec::new();
    let mut style = Run::default();
    let mut saved = Vec::new();
    let mut lists: Vec<Option<u64>> = Vec::new();
    for event in Parser::new_ext(source, Options::ENABLE_STRIKETHROUGH) {
        match event {
            Event::Start(tag) => {
                saved.push((style.clone(), None));
                match tag {
                    Tag::Strong | Tag::Heading { .. } => style.bold = true,
                    Tag::Emphasis => style.italic = true,
                    Tag::Strikethrough => style.strike = true,
                    Tag::CodeBlock(_) => style.code = true,
                    Tag::Link { dest_url, .. } | Tag::Image { dest_url, .. } => {
                        if link_allowed(&dest_url) {
                            style.url = Some(dest_url.into_string());
                        } else if let Some((_, literal)) = saved.last_mut() {
                            *literal = Some(dest_url.into_string());
                        }
                    }
                    Tag::Paragraph => {
                        if !out.is_empty() {
                            out.push(Run {
                                text: "\n\n".into(),
                                ..Default::default()
                            });
                        }
                    }
                    Tag::List(start) => lists.push(start),
                    Tag::Item => {
                        let marker = match lists.last_mut() {
                            Some(Some(n)) => {
                                let value = format!("{n}. ");
                                *n = n.saturating_add(1);
                                value
                            }
                            _ => "• ".into(),
                        };
                        out.push(Run {
                            text: format!("\n{marker}"),
                            ..Default::default()
                        });
                    }
                    _ => {}
                }
            }
            Event::End(tag) => {
                if tag == TagEnd::List(false) || tag == TagEnd::List(true) {
                    lists.pop();
                }
                let (prior, literal) = saved.pop().unwrap_or_default();
                style = prior;
                if let Some(url) = literal {
                    out.push(Run {
                        text: format!(" ({url})"),
                        ..Default::default()
                    });
                }
            }
            Event::Text(text) | Event::Html(text) | Event::InlineHtml(text) => out.push(Run {
                text: text.into_string(),
                ..style.clone()
            }),
            Event::Code(code) => out.push(Run {
                text: code.into_string(),
                code: true,
                ..style.clone()
            }),
            Event::SoftBreak | Event::HardBreak => out.push(Run {
                text: "\n".into(),
                ..style.clone()
            }),
            Event::Rule => out.push(Run {
                text: "\n—\n".into(),
                ..Default::default()
            }),
            Event::TaskListMarker(checked) => out.push(Run {
                text: if checked {
                    "☑ ".into()
                } else {
                    "☐ ".into()
                },
                ..style.clone()
            }),
            Event::FootnoteReference(text) | Event::InlineMath(text) | Event::DisplayMath(text) => {
                out.push(Run {
                    text: text.into_string(),
                    ..style.clone()
                })
            }
        }
    }
    out
}

/// Split text runs, carrying their explicit styles and URL on each piece.
pub(super) fn chunks(runs: Vec<Run>, limit: usize) -> Vec<Vec<Run>> {
    let mut out = Vec::new();
    let mut chunk = Vec::new();
    let mut used = 0;
    for run in runs {
        let mut start = 0;
        for (at, ch) in run.text.char_indices() {
            if used + ch.len_utf16() > limit {
                if at > start {
                    chunk.push(Run {
                        text: run.text[start..at].into(),
                        ..run.clone()
                    });
                }
                if !chunk.is_empty() {
                    out.push(std::mem::take(&mut chunk));
                }
                used = 0;
                start = at;
            }
            used += ch.len_utf16();
        }
        if start < run.text.len() {
            chunk.push(Run {
                text: run.text[start..].into(),
                ..run
            });
        }
    }
    if !chunk.is_empty() {
        out.push(chunk);
    }
    out
}
