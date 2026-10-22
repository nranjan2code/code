use super::{Element, Message, inline, split_units, table_record, units};
use serde_json::json;

const DISCORD_DESCRIPTION: usize = 4_096;
const DISCORD_TITLE: usize = 256;

pub(super) fn compile(elements: &[Element]) -> Vec<Message> {
    let mut messages = Vec::new();
    let mut title = String::new();
    let mut description = String::new();
    for element in elements {
        if let Element::Heading(heading) = element {
            flush_discord(&mut messages, &mut title, &mut description);
            let mut parts = split_units(heading, DISCORD_TITLE).into_iter();
            title = parts.next().unwrap_or_default();
            for part in parts {
                flush_discord(&mut messages, &mut title, &mut description);
                title = part;
            }
            continue;
        }
        for part in discord_parts(element) {
            if !description.is_empty()
                && units(&description) + 2 + units(&part) > DISCORD_DESCRIPTION
            {
                flush_discord(&mut messages, &mut title, &mut description);
            }
            if !description.is_empty() {
                description.push_str("\n\n");
            }
            description.push_str(&part);
        }
    }
    flush_discord(&mut messages, &mut title, &mut description);
    messages
}
fn flush_discord(messages: &mut Vec<Message>, title: &mut String, description: &mut String) {
    if title.is_empty() && description.is_empty() {
        return;
    }
    let mut embed = json!({});
    if !title.is_empty() {
        embed["title"] = json!(std::mem::take(title));
    }
    if !description.is_empty() {
        embed["description"] = json!(std::mem::take(description));
    }
    messages.push(Message {
        method: "createMessage",
        body: json!({"embeds":[embed],"allowed_mentions":{"parse":[]}}),
    });
}
fn escape_discord(text: &str) -> String {
    let mut out = String::new();
    for c in text.chars() {
        if matches!(
            c,
            '\\' | '*' | '_' | '~' | '`' | '|' | '[' | ']' | '<' | '>'
        ) {
            out.push('\\');
        }
        out.push(c);
    }
    out
}
/// Escape and split atomically, reserving the wrapper on every part.
fn discord_wrapped(text: &str, prefix: &str, suffix: &str, limit: usize) -> Vec<String> {
    let available = limit.saturating_sub(units(prefix) + units(suffix));
    let mut out = Vec::new();
    let mut current = String::new();
    let mut used = 0;
    for ch in text.chars() {
        let encoded = escape_discord(&ch.to_string());
        let size = units(&encoded);
        if used + size > available && !current.is_empty() {
            out.push(format!("{prefix}{current}{suffix}"));
            current.clear();
            used = 0;
        }
        current.push_str(&encoded);
        used += size;
    }
    if !current.is_empty() {
        out.push(format!("{prefix}{current}{suffix}"));
    }
    out
}
fn discord_parts(element: &Element) -> Vec<String> {
    match element {
        Element::Prose(source) => inline::runs(source)
            .iter()
            .flat_map(|run| {
                let mut prefix = String::new();
                let mut suffix = String::new();
                for (enabled, marker) in [
                    (run.bold, "**"),
                    (run.italic, "*"),
                    (run.strike, "~~"),
                    (run.code, "`"),
                ] {
                    if enabled {
                        prefix.push_str(marker);
                        suffix.insert_str(0, marker);
                    }
                }
                if let Some(url) = &run.url {
                    let link_suffix = format!(
                        "](<{}>)",
                        url.replace(')', "%29")
                            .replace('>', "%3E")
                            .replace('<', "%3C")
                    );
                    if units(&prefix) + units(&suffix) + units(&link_suffix) + 3
                        < DISCORD_DESCRIPTION
                    {
                        prefix.insert(0, '[');
                        suffix.push_str(&link_suffix);
                    } else {
                        return discord_parts(&Element::Text(format!("{} ({url})", run.text)));
                    }
                }
                discord_wrapped(&run.text, &prefix, &suffix, DISCORD_DESCRIPTION)
            })
            .collect(),
        Element::Field { label, value } => {
            let mut parts = discord_wrapped(label, "**", "**", DISCORD_DESCRIPTION);
            parts.extend(discord_wrapped(value, "", "", DISCORD_DESCRIPTION));
            parts
        }
        Element::Link { text, url } => {
            let suffix = format!(
                "](<{}>)",
                url.replace(')', "%29")
                    .replace('>', "%3E")
                    .replace('<', "%3C")
                    .replace('\n', "%0A")
                    .replace('\r', "%0D")
            );
            if units(&suffix) + 1 + 2 > DISCORD_DESCRIPTION {
                // A URL is indivisible when clickable. Show its exact text
                // across continuations rather than shortening the destination.
                discord_wrapped(&format!("{text} ({url})"), "", "", DISCORD_DESCRIPTION)
            } else {
                discord_wrapped(text, "[", &suffix, DISCORD_DESCRIPTION)
            }
        }
        Element::Details { summary, body } => {
            let mut parts = discord_wrapped(summary, "**", "**", DISCORD_DESCRIPTION);
            parts.extend(body.iter().flat_map(discord_parts));
            parts
        }
        Element::List { ordered, items } => items
            .iter()
            .enumerate()
            .flat_map(|(i, item)| {
                let mut parts = Vec::new();
                let mut marker = Some(if *ordered {
                    format!("{}. ", i + 1)
                } else {
                    "• ".into()
                });
                for element in item {
                    for part in discord_parts(element) {
                        if let Some(prefix) = marker.take() {
                            if units(&prefix) + units(&part) <= DISCORD_DESCRIPTION {
                                parts.push(format!("{prefix}{part}"));
                            } else {
                                parts.push(prefix);
                                parts.push(part);
                            }
                        } else {
                            parts.push(part);
                        }
                    }
                }
                parts
            })
            .collect(),
        Element::Table { header, rows } => {
            if rows.is_empty() {
                return discord_wrapped(&header.join(" | "), "", "", DISCORD_DESCRIPTION);
            }
            rows.iter()
                .flat_map(|row| {
                    table_record(header, row)
                        .iter()
                        .flat_map(discord_parts)
                        .collect::<Vec<_>>()
                })
                .collect()
        }
        Element::Code(s) => {
            let longest = s.split(|c| c != '`').map(str::len).max().unwrap_or(0);
            let fence = "`".repeat(longest.saturating_add(1).max(3));
            let overhead = 2 * fence.len() + 2;
            if overhead + 2 >= DISCORD_DESCRIPTION {
                discord_wrapped(s, "", "", DISCORD_DESCRIPTION)
            } else {
                split_units(s, DISCORD_DESCRIPTION - overhead)
                    .into_iter()
                    .map(|part| format!("{fence}\n{part}\n{fence}"))
                    .collect()
            }
        }
        Element::Text(s) | Element::Heading(s) | Element::Context(s) => {
            discord_wrapped(s, "", "", DISCORD_DESCRIPTION)
        }
    }
}
