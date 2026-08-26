//! Modal builders: help, interactive keymap, `/view` file reader, picker drawing.

use crate::data::ClientData;
use crate::keymap::Keymap;
use crate::palette::ChoicePicker;
use crate::render::Screen;
use crate::state::{ModalView, PickerKind};

/// Cap on lines rendered by `/view`; the modal scrolls, but building a
/// million rows for a huge file would stall the redraw.
const MAX_VIEW_LINES: usize = 5000;

pub fn help_modal_rows(data: &ClientData) -> Vec<String> {
    let mut rows = vec!["COMMANDS".to_string(), String::new()];
    rows.extend(crate::commands::help_rows());
    let customs = data.custom_commands();
    if !customs.is_empty() {
        rows.push(String::new());
        rows.push("CUSTOM COMMANDS".to_string());
        for (name, description) in &customs {
            rows.push(format!(
                "{name:<16} {}",
                if description.is_empty() {
                    "(no description)"
                } else {
                    description
                }
            ));
        }
    }
    rows.extend([
        String::new(),
        "PROMPT INPUT".to_string(),
        "@path             attach exact file contents".to_string(),
        "!command          run a local shell command".to_string(),
        "Alt-Enter         insert a newline".to_string(),
        "Ctrl-R            search prompt history".to_string(),
        "Ctrl-P            search every command".to_string(),
        String::new(),
        "DURING A RUN".to_string(),
        "Enter             steer the active agent (or attached subagent)".to_string(),
        "Tab               queue the next turn".to_string(),
        "Alt-S             attach to a running subagent".to_string(),
        "Alt-Y             copy last response via OSC52 when enabled".to_string(),
        "Alt-A             review pending approval".to_string(),
        "Ctrl-O            expand a stashed large paste at the cursor".to_string(),
        "Ctrl-G            edit the composer draft in $EDITOR".to_string(),
        "Ctrl-C            interrupt and preserve partial output".to_string(),
    ]);
    rows
}

/// Builds the interactive keymap modal plus the selectable-row metadata:
/// each entry maps a row index to the action name bound there.
pub fn build_keymap_modal(
    keymap: &Keymap,
    selected: Option<usize>,
) -> (ModalView, Vec<(usize, String)>) {
    let mut rows: Vec<String> = Vec::new();
    let mut meta: Vec<(usize, String)> = Vec::new();
    let mut last_ctx = "";
    for (ctx, key, action) in keymap.rows() {
        if ctx != last_ctx {
            rows.push(format!("-- {ctx}"));
            last_ctx = ctx;
        }
        meta.push((rows.len(), action.to_string()));
        rows.push(format!("{key:<16} {action}"));
    }
    let conflicts = keymap.conflicts();
    if !conflicts.is_empty() {
        rows.push(String::new());
        rows.push("!! conflicts".to_string());
        for (key, detail) in &conflicts {
            rows.push(format!("{key:<16} {detail}"));
        }
    }
    (
        ModalView {
            title: "keymap · bindings".to_string(),
            rows,
            scroll: 0,
            footer: "up/down select · r rebinds the highlighted row · Esc close".to_string(),
            selected,
            ..Default::default()
        },
        meta,
    )
}

/// Moves the modal selection through selectable rows.
pub fn step_select(
    current: Option<usize>,
    meta: &[(usize, String)],
    forward: bool,
) -> Option<usize> {
    if meta.is_empty() {
        return None;
    }
    let pos = current
        .and_then(|sel| meta.iter().position(|(row, _)| *row == sel))
        .unwrap_or(0);
    let next = if forward {
        (pos + 1) % meta.len()
    } else {
        (pos + meta.len() - 1) % meta.len()
    };
    Some(meta[next].0)
}

pub fn draw_picker(screen: &mut Screen, kind: PickerKind, picker: &ChoicePicker) {
    let items = picker.filtered().into_iter().cloned().collect::<Vec<_>>();
    let (title, allow_custom) = match kind {
        PickerKind::Provider => ("choose provider", false),
        PickerKind::Model => ("choose model · type any exact model ID", true),
        PickerKind::Theme => ("choose theme · up/down previews live", false),
        PickerKind::Sessions | PickerKind::Subagents => ("attach to a running subagent", false),
    };
    screen.redraw_picker(
        title,
        picker.query(),
        &items,
        picker.selected(),
        allow_custom,
    );
}

/// `/view <path>` — read a workspace file into a modal.
///
/// The read is server-confined (`data.read_file`), so no local path handling
/// happens here. Text is shown with line numbers; images and binaries report
/// their kind and size instead of spraying bytes at the terminal.
pub async fn view_file_modal(data: &ClientData, rel: &str) -> Result<ModalView, String> {
    let bytes = data.read_file(rel).await?;
    let kind = view_kind(rel, &bytes);
    let size = format_bytes(bytes.len() as u64);

    let rows: Vec<String> = match kind {
        ViewKind::Text => {
            let text = String::from_utf8_lossy(&bytes);
            let total = text.lines().count();
            let width = total.to_string().len().max(2);
            text.lines()
                .take(MAX_VIEW_LINES)
                .enumerate()
                .map(|(i, line)| format!("{:>width$} │ {}", i + 1, line, width = width))
                .chain(
                    (total > MAX_VIEW_LINES)
                        .then(|| format!("… {} more lines", total - MAX_VIEW_LINES)),
                )
                .collect()
        }
        ViewKind::Image => vec![
            format!("image · {size}"),
            String::new(),
            "Terminals cannot render this; open it in the desktop app".to_string(),
        ],
        ViewKind::Binary => vec![
            format!("binary · {size}"),
            String::new(),
            "Not shown: these bytes are not text.".to_string(),
        ],
    };

    Ok(ModalView {
        title: format!("{rel} · {} · {size}", kind.as_str()),
        rows,
        scroll: 0,
        footer: "↑↓ scroll · Esc close".to_string(),
        ..Default::default()
    })
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ViewKind {
    Text,
    Image,
    Binary,
}

impl ViewKind {
    fn as_str(self) -> &'static str {
        match self {
            Self::Text => "text",
            Self::Image => "image",
            Self::Binary => "binary",
        }
    }
}

/// Classify by extension first, then by content: a NUL byte or invalid UTF-8
/// means the bytes are not text whatever the name claims. SVG stays text —
/// renderable *and* meaningfully editable.
fn view_kind(path: &str, bytes: &[u8]) -> ViewKind {
    let extension = std::path::Path::new(path)
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or("")
        .to_ascii_lowercase();
    if matches!(
        extension.as_str(),
        "png" | "jpg" | "jpeg" | "gif" | "webp" | "bmp" | "ico"
    ) {
        return ViewKind::Image;
    }
    if bytes.contains(&0) || std::str::from_utf8(bytes).is_err() {
        return ViewKind::Binary;
    }
    ViewKind::Text
}

fn format_bytes(n: u64) -> String {
    if n < 1024 {
        format!("{n} B")
    } else if n < 1024 * 1024 {
        format!("{:.1} KB", n as f64 / 1024.0)
    } else {
        format!("{:.1} MB", n as f64 / (1024.0 * 1024.0))
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;
    use crate::keys::Action;

    #[test]
    fn keymap_modal_groups_contexts_and_maps_selectable_rows() {
        let keymap = Keymap::default();
        let (modal, meta) = build_keymap_modal(&keymap, Some(3));
        assert_eq!(modal.title, "keymap · bindings");
        assert_eq!(modal.selected, Some(3));
        assert!(modal.rows.first().is_some_and(|r| r.starts_with("-- ")));
        assert!(modal.rows.iter().any(|r| r.contains("Ctrl-p")));
        assert!(!meta.is_empty());
        assert!(meta.windows(2).all(|w| w[0].0 < w[1].0));
        for (row, action) in &meta {
            assert!(
                modal.rows[*row].contains(action.as_str()),
                "meta row {} ({action}) does not match `{}`",
                row,
                modal.rows[*row]
            );
        }
        assert!(modal.footer.contains("r rebinds"));
    }

    #[test]
    fn keymap_modal_lists_conflicts_as_a_trailing_section() {
        use crate::keymap::{Ctx, parse_binding};
        let mut keymap = Keymap::default();
        let (_, spec) = parse_binding("Ctrl-p").unwrap();
        keymap.bind(Ctx::Composer, spec, Action::Exit);
        let (modal, _) = build_keymap_modal(&keymap, None);
        let start = modal
            .rows
            .iter()
            .position(|r| r == "!! conflicts")
            .expect("conflict section present");
        assert!(modal.rows[start + 1..].iter().any(|r| r.contains("vs")));
    }

    #[test]
    fn step_select_walks_meta_rows_and_wraps() {
        let meta = vec![
            (2usize, "a".to_string()),
            (5, "b".to_string()),
            (9, "c".to_string()),
        ];
        assert_eq!(step_select(None, &meta, true), Some(5));
        assert_eq!(step_select(Some(5), &meta, true), Some(9));
        assert_eq!(step_select(Some(9), &meta, true), Some(2));
        assert_eq!(step_select(Some(2), &meta, false), Some(9));
        assert_eq!(step_select(None, &[], true), None);
    }

    #[test]
    fn view_helpers_classify_and_format_bytes() {
        assert_eq!(format_bytes(512), "512 B");
        assert_eq!(format_bytes(2048), "2.0 KB");
        assert_eq!(format_bytes(3 * 1024 * 1024), "3.0 MB");

        assert_eq!(view_kind("notes.md", b"hello"), ViewKind::Text);
        assert_eq!(view_kind("notes.svg", b"<svg></svg>"), ViewKind::Text);
        assert_eq!(view_kind("logo.png", b"\x89PNG"), ViewKind::Image);
        assert_eq!(view_kind("blob.bin", b"ok\0bad"), ViewKind::Binary);
        assert_eq!(view_kind("weird.txt", &[0xff, 0xfe]), ViewKind::Binary);
        assert_eq!(ViewKind::Image.as_str(), "image");
        assert_ne!(MAX_VIEW_LINES, 0);
    }

    #[test]
    fn truncated_text_reports_remaining_line_count_shape() {
        let lines: Vec<&str> = (0..MAX_VIEW_LINES + 3).map(|_| "x").collect();
        let total = lines.len();
        let extra =
            (total > MAX_VIEW_LINES).then(|| format!("… {} more lines", total - MAX_VIEW_LINES));
        assert_eq!(extra.as_deref(), Some("… 3 more lines"));
    }
}
