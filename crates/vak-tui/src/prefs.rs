//! Surface-owned presentation preferences (`[ui]` tables in the project
//! config). The base never sees these; they survive offline.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use crate::theme::ThemeColors;

#[derive(Debug, Clone)]
pub struct UiPrefs {
    pub theme: Option<String>,
    pub bell: bool,
    pub composer: Option<String>,
    pub osc52: bool,
    pub a11y_plain: bool,
    pub a11y_motion: bool,
    pub a11y_reader: bool,
    pub custom_themes: BTreeMap<String, ThemeColors>,
}

impl Default for UiPrefs {
    fn default() -> Self {
        Self {
            theme: None,
            bell: true,
            composer: None,
            osc52: false,
            a11y_plain: false,
            a11y_motion: false,
            a11y_reader: false,
            custom_themes: BTreeMap::new(),
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
enum Section {
    None,
    Ui,
    Theme(String),
}

impl UiPrefs {
    pub fn load_from_project(project_root: &Path) -> Self {
        let mut prefs = Self::default();
        let content =
            std::fs::read_to_string(project_config_path(project_root)).unwrap_or_default();
        prefs.apply(&content);
        prefs
    }

    pub fn save_to_project(&self, project_root: &Path) {
        let existing =
            std::fs::read_to_string(project_config_path(project_root)).unwrap_or_default();
        let next = upsert_ui_section(&existing, self);
        let path = project_config_path(project_root);
        if let Some(dir) = path.parent() {
            let _ = std::fs::create_dir_all(dir);
        }
        if let Err(e) = std::fs::write(&path, next) {
            eprintln!("warning: could not persist ui prefs — {e}");
        }
    }

    fn apply(&mut self, content: &str) {
        let mut section = Section::None;
        for raw in content.lines() {
            let trimmed = strip_comment(raw).trim().to_string();
            if trimmed.is_empty() {
                continue;
            }
            if trimmed.starts_with('[') && trimmed.ends_with(']') {
                section = classify(&trimmed);
                continue;
            }
            let Some((key, value)) = trimmed.split_once('=') else {
                continue;
            };
            let key = key.trim();
            let value = unquote(value.trim());
            match &section {
                Section::Ui => match key {
                    "theme" => self.theme = (!value.is_empty()).then_some(value),
                    "bell" => self.bell = value == "true",
                    "composer" => self.composer = (!value.is_empty()).then_some(value),
                    "osc52" => self.osc52 = value == "true",
                    "a11y_plain" => self.a11y_plain = value == "true",
                    "a11y_motion" => self.a11y_motion = value == "true",
                    "a11y_reader" => self.a11y_reader = value == "true",
                    _ => {}
                },
                Section::Theme(name) => {
                    self.custom_themes
                        .entry(name.clone())
                        .or_default()
                        .insert(key.to_string(), value);
                }
                Section::None => {}
            }
        }
    }
}

fn project_config_path(project_root: &Path) -> PathBuf {
    project_root.join(".vakcoder").join("config.toml")
}

fn classify(header: &str) -> Section {
    if header == "[ui]" {
        return Section::Ui;
    }
    if let Some(rest) = header
        .strip_prefix("[ui.themes.")
        .map(|r| r.trim_end_matches(']'))
    {
        return Section::Theme(rest.to_string());
    }
    Section::None
}

fn strip_comment(line: &str) -> &str {
    let bytes = line.as_bytes();
    let mut in_quotes = false;
    for (i, b) in bytes.iter().enumerate() {
        match b {
            b'"' | b'\'' => in_quotes = !in_quotes,
            b'#' if !in_quotes => return &line[..i],
            _ => {}
        }
    }
    line
}

fn unquote(v: &str) -> String {
    v.trim_matches('"').trim_matches('\'').to_string()
}

fn quote(v: &str) -> String {
    format!("\"{}\"", v.replace('\\', "\\\\").replace('"', "\\\""))
}

fn render_ui_table(prefs: &UiPrefs) -> String {
    let mut out = String::from("[ui]\n");
    if let Some(theme) = &prefs.theme {
        out.push_str(&format!("theme = {}\n", quote(theme)));
    }
    out.push_str(&format!("bell = {}\n", prefs.bell));
    if let Some(composer) = &prefs.composer {
        out.push_str(&format!("composer = {}\n", quote(composer)));
    }
    out.push_str(&format!("osc52 = {}\n", prefs.osc52));
    out.push_str(&format!("a11y_plain = {}\n", prefs.a11y_plain));
    out.push_str(&format!("a11y_motion = {}\n", prefs.a11y_motion));
    out.push_str(&format!("a11y_reader = {}\n", prefs.a11y_reader));
    for (name, colors) in &prefs.custom_themes {
        out.push_str(&format!("\n[ui.themes.{}]\n", quote(name)));
        for (k, v) in colors {
            out.push_str(&format!("{k} = {}\n", quote(v)));
        }
    }
    out
}

fn upsert_ui_section(content: &str, prefs: &UiPrefs) -> String {
    let fresh = render_ui_table(prefs);
    let mut out = String::new();
    let mut in_ui_block = false;
    let mut ui_replaced = false;
    for line in content.lines() {
        let trimmed = line.trim();
        if trimmed.starts_with('[') && trimmed.ends_with(']') {
            let is_plain_ui = trimmed == "[ui]";
            if is_plain_ui && ui_replaced {
                // second [ui]: drop, ours already written
                in_ui_block = false;
                continue;
            }
            if is_plain_ui && !ui_replaced {
                in_ui_block = true;
                ui_replaced = true;
                out.push_str(fresh.trim_end_matches('\n'));
                out.push('\n');
                continue;
            }
            if trimmed.starts_with("[ui.themes.") {
                in_ui_block = true;
                out.push_str(line);
                out.push('\n');
                continue;
            }
            in_ui_block = false;
            out.push_str(line);
            out.push('\n');
            continue;
        }
        if in_ui_block {
            if !trimmed.is_empty() && !trimmed.starts_with('#') {
                // keys of [ui]/[ui.themes.*] blocks are regenerated or kept?
                // Theme tables keep their lines; plain [ui] keys were replaced.
                let is_theme_block = out.contains("[ui.themes.");
                if is_theme_block {
                    out.push_str(line);
                    out.push('\n');
                }
                continue;
            }
            out.push_str(line);
            out.push('\n');
            continue;
        }
        out.push_str(line);
        out.push('\n');
    }
    if !ui_replaced {
        if !out.is_empty() && !out.ends_with("\n\n") {
            out.push('\n');
        }
        out.push_str(fresh.trim_end_matches('\n'));
        out.push('\n');
    }
    while out.ends_with("\n\n\n") {
        out.truncate(out.len() - 1);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(content: &str) -> UiPrefs {
        let mut p = UiPrefs::default();
        p.apply(content);
        p
    }

    #[test]
    fn parses_ui_section() {
        let p = parse("[ui]\ntheme = \"neo\"\nbell = false\ncomposer = \"vim\"\n");
        assert_eq!(p.theme.as_deref(), Some("neo"));
        assert!(!p.bell);
        assert_eq!(p.composer.as_deref(), Some("vim"));
    }

    #[test]
    fn ignores_other_tables() {
        let p = parse("[model]\nprovider=\"anthropic\"\n\n[ui]\nbell=false\n");
        assert!(p.theme.is_none());
        assert!(!p.bell);
    }

    #[test]
    fn parses_custom_theme_colors() {
        let p = parse("[ui.themes.solar]\naccent = \"#ff0000\"\n");
        assert_eq!(
            p.custom_themes.get("solar").and_then(|c| c.get("accent")),
            Some(&"#ff0000".to_string())
        );
    }

    #[test]
    fn upsert_appends_when_missing() {
        let out = upsert_ui_section(
            "[model]\nprovider = \"anthropic\"\n",
            &parse("[ui]\nbell = false\n"),
        );
        assert!(out.contains("[model]"));
        assert!(out.contains("[ui]\n"));
        assert!(out.contains("bell = false"));
    }

    #[test]
    fn upsert_replaces_existing_ui() {
        let out = upsert_ui_section(
            "[ui]\nbell = true\ntheme = \"dark\"\n",
            &parse("[ui]\nbell = false\n"),
        );
        assert!(!out.contains("bell = true"));
        assert!(out.contains("bell = false"));
    }

    #[test]
    fn upsert_preserves_custom_themes() {
        let original = "[ui.themes.mine]\naccent = \"#123456\"\n";
        let mut prefs = parse(original);
        prefs.bell = false;
        let out = upsert_ui_section(original, &prefs);
        assert!(out.contains("accent = \"#123456\""), "{out}");
        assert!(out.contains("bell = false"));
    }
}
