use crate::keys::Action;
use crossterm::event::{KeyCode, KeyModifiers};

use KeyModifiers as M;

/// Interaction context for key bindings. Composer covers idle editing;
/// Running covers keys pressed while an agent run is active.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Ctx {
    Both,
    Composer,
    Running,
}

impl Ctx {
    pub fn label(self) -> &'static str {
        match self {
            Ctx::Both => "always",
            Ctx::Composer => "composer",
            Ctx::Running => "while running",
        }
    }
}

/// A physical key plus modifiers. Compared structurally, not hashed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct KeySpec {
    pub code: KeyCode,
    pub mods: KeyModifiers,
}

#[derive(Debug, Clone, Copy)]
struct Entry {
    ctx: Ctx,
    spec: KeySpec,
    action: Action,
}

/// Ordered contextual keymap: an exact `(ctx, key)` match wins over `Both`
/// entries; plain characters fall through to insert, mirroring readline
/// behavior. The default table is the dispatch truth behind
/// `crate::keys::map_key`, so overrides and conflict detection always see
/// the real binding set.
#[derive(Debug, Clone)]
pub struct Keymap {
    entries: Vec<Entry>,
}

fn spec(code: KeyCode, mods: KeyModifiers) -> KeySpec {
    KeySpec { code, mods }
}

fn default_entries() -> Vec<(Ctx, KeySpec, Action)> {
    use KeyCode as K;
    let plain = M::NONE;
    vec![
        (
            Ctx::Running,
            spec(K::Char('c'), M::CONTROL),
            Action::Interrupt,
        ),
        (
            Ctx::Composer,
            spec(K::Char('c'), M::CONTROL | M::SHIFT),
            Action::Exit,
        ),
        (
            Ctx::Composer,
            spec(K::Char('c'), M::CONTROL),
            Action::CancelOrClear,
        ),
        (Ctx::Running, spec(K::Esc, plain), Action::CancelOrClear),
        (Ctx::Composer, spec(K::Tab, plain), Action::Complete),
        (Ctx::Running, spec(K::Tab, plain), Action::Queue),
        (Ctx::Running, spec(K::BackTab, plain), Action::Queue),
        (Ctx::Both, spec(K::Char('d'), M::CONTROL), Action::Exit),
        (Ctx::Both, spec(K::Char('u'), M::CONTROL), Action::ClearLine),
        (Ctx::Both, spec(K::Char('a'), M::CONTROL), Action::Home),
        (Ctx::Both, spec(K::Char('e'), M::CONTROL), Action::End),
        (
            Ctx::Both,
            spec(K::Char('k'), M::CONTROL),
            Action::DeleteToLineEnd,
        ),
        (Ctx::Both, spec(K::Char('h'), M::CONTROL), Action::Backspace),
        (
            Ctx::Both,
            spec(K::Char('l'), M::CONTROL),
            Action::ClearViewport,
        ),
        (Ctx::Both, spec(K::Char('a'), M::ALT), Action::OpenApproval),
        (Ctx::Both, spec(K::Char('s'), M::ALT), Action::Subagents),
        (Ctx::Both, spec(K::Char('y'), M::ALT), Action::CopyResponse),
        (
            Ctx::Both,
            spec(K::Char('n'), M::CONTROL),
            Action::HistoryNext,
        ),
        (
            Ctx::Both,
            spec(K::Char('r'), M::CONTROL),
            Action::HistorySearch,
        ),
        (
            Ctx::Both,
            spec(K::Char('w'), M::CONTROL),
            Action::DeleteWordBack,
        ),
        (
            Ctx::Both,
            spec(K::Char('d'), M::ALT),
            Action::DeleteWordForward,
        ),
        (Ctx::Both, spec(K::Char('z'), M::CONTROL), Action::Undo),
        (Ctx::Both, spec(K::Char('z'), M::ALT), Action::Redo),
        (
            Ctx::Both,
            spec(K::Char('t'), M::CONTROL),
            Action::ToggleThinking,
        ),
        (
            Ctx::Both,
            spec(K::Char('p'), M::CONTROL),
            Action::CommandPalette,
        ),
        (
            Ctx::Both,
            spec(K::Char('o'), M::CONTROL),
            Action::ExpandStash,
        ),
        (
            Ctx::Both,
            spec(K::Char('g'), M::CONTROL),
            Action::ExternalEditor,
        ),
        (
            Ctx::Both,
            spec(K::Backspace, M::ALT),
            Action::DeleteWordBack,
        ),
        (Ctx::Both, spec(K::Char('b'), M::ALT), Action::WordLeft),
        (Ctx::Both, spec(K::Char('f'), M::ALT), Action::WordRight),
        (
            Ctx::Both,
            spec(K::Char('j'), M::CONTROL),
            Action::Insert('\n'),
        ),
        (Ctx::Both, spec(K::Enter, M::ALT), Action::Insert('\n')),
        (Ctx::Both, spec(K::Enter, plain), Action::Submit),
        (Ctx::Both, spec(K::Backspace, plain), Action::Backspace),
        (Ctx::Both, spec(K::Delete, plain), Action::Delete),
        (Ctx::Both, spec(K::Left, plain), Action::Left),
        (Ctx::Both, spec(K::Right, plain), Action::Right),
        (Ctx::Both, spec(K::Home, plain), Action::Home),
        (Ctx::Both, spec(K::End, plain), Action::End),
        (Ctx::Both, spec(K::Up, plain), Action::HistoryPrev),
        (Ctx::Both, spec(K::Down, plain), Action::HistoryNext),
    ]
}

impl Default for Keymap {
    fn default() -> Self {
        Self {
            entries: default_entries()
                .into_iter()
                .map(|(ctx, sp, action)| Entry {
                    ctx,
                    spec: sp,
                    action,
                })
                .collect(),
        }
    }
}

impl Keymap {
    /// Binds `action` to `(ctx, key)`, replacing any binding of exactly the
    /// same key in exactly the same scope. Cross-scope shadowing is allowed
    /// and reported by [`Keymap::conflicts`].
    pub fn bind(&mut self, ctx: Ctx, key_spec: KeySpec, action: Action) {
        self.entries
            .retain(|e| !(e.spec == key_spec && e.ctx == ctx));
        self.entries.push(Entry {
            ctx,
            spec: key_spec,
            action,
        });
    }

    /// Interactive rebind: moves every binding of the named action to
    /// `key_spec` while preserving its original context, displacing any
    /// other action already bound there (same scope). Returns false when
    /// no action carries that name (or it is not rebindable, e.g. insert).
    pub fn rebind_named(&mut self, action_name: &str, key_spec: KeySpec) -> bool {
        let Some(action) = Action::parse(action_name) else {
            return false;
        };
        let Some(ctx) = self
            .entries
            .iter()
            .find(|e| e.action == action)
            .map(|e| e.ctx)
        else {
            return false;
        };
        self.entries
            .retain(|e| e.action != action && !(e.ctx == ctx && e.spec == key_spec));
        self.entries.push(Entry {
            ctx,
            spec: key_spec,
            action,
        });
        true
    }

    /// Applies `[ui.keymap]` overrides: `"Ctrl-P" = "command-palette"` or
    /// `"running|Tab" = "queue"`. Unparseable rows are skipped — unknown
    /// config keys never break startup.
    pub fn with_overrides(
        mut self,
        overrides: &std::collections::BTreeMap<String, String>,
    ) -> Self {
        for (key_str, action_str) in overrides {
            let Some(action) = Action::parse(action_str) else {
                continue;
            };
            let Some((ctx, parsed)) = parse_binding(key_str) else {
                continue;
            };
            self.bind(ctx, parsed, action);
        }
        self
    }

    pub fn action_for(&self, running: bool, code: KeyCode, mods: KeyModifiers) -> Action {
        let ctx = if running { Ctx::Running } else { Ctx::Composer };
        let mut both_hit = None;
        for e in &self.entries {
            if e.spec.code != code || e.spec.mods != mods {
                continue;
            }
            if e.ctx == ctx {
                return e.action;
            }
            if e.ctx == Ctx::Both {
                both_hit.get_or_insert(e.action);
            }
        }
        if let Some(action) = both_hit {
            return action;
        }
        match code {
            KeyCode::Char(c) if mods.is_empty() || mods == M::SHIFT => Action::Insert(c),
            _ => Action::Ignore,
        }
    }

    /// Keys bound to more than one action across overlapping scopes —
    /// usually a config mistake worth surfacing in `/keymap`.
    pub fn conflicts(&self) -> Vec<(String, String)> {
        let mut out = Vec::new();
        for i in 0..self.entries.len() {
            for j in (i + 1)..self.entries.len() {
                let (a, b) = (&self.entries[i], &self.entries[j]);
                let overlaps = a.ctx == b.ctx || a.ctx == Ctx::Both || b.ctx == Ctx::Both;
                if overlaps && a.spec == b.spec && a.action != b.action {
                    out.push((
                        label(a.spec),
                        format!("{} vs {}", a.action.name(), b.action.name()),
                    ));
                    break;
                }
            }
        }
        out
    }

    /// Human-readable binding rows: (context label, key, action name).
    pub fn rows(&self) -> Vec<(&'static str, String, &'static str)> {
        self.entries
            .iter()
            .map(|e| (e.ctx.label(), label(e.spec), e.action.name()))
            .collect()
    }
}

fn label(s: KeySpec) -> String {
    let mut out = String::new();
    if s.mods.contains(M::CONTROL) {
        out.push_str("Ctrl-");
    }
    if s.mods.contains(M::ALT) {
        out.push_str("Alt-");
    }
    if s.mods.contains(M::SHIFT) && !matches!(s.code, KeyCode::Char(_)) {
        out.push_str("Shift-");
    }
    match s.code {
        KeyCode::Char(c) => out.push(c),
        KeyCode::F(n) => out.push_str(&format!("F{n}")),
        other => out.push_str(&format!("{other:?}")),
    }
    out
}

/// Public alias for the binding label renderer, used by the interactive
/// rebind flow in `app`.
pub fn keymap_label(spec: KeySpec) -> String {
    label(spec)
}

/// Parses `"Ctrl-P"`, `"Alt-Enter"`, `"composer|Ctrl-U"`, `"x"`.
pub fn parse_binding(s: &str) -> Option<(Ctx, KeySpec)> {
    let s = s.trim();
    let (ctx, rest) = match s.split_once('|') {
        Some(("composer", r)) => (Ctx::Composer, r.trim()),
        Some(("running", r)) => (Ctx::Running, r.trim()),
        Some((_, _)) => return None,
        None => (Ctx::Both, s),
    };
    let mut mods = M::NONE;
    let mut remaining = rest;
    loop {
        let lowered = remaining.to_ascii_lowercase();
        if lowered.starts_with("ctrl-") {
            mods |= M::CONTROL;
            remaining = &remaining[5..];
        } else if lowered.starts_with("alt-") {
            mods |= M::ALT;
            remaining = &remaining[4..];
        } else if lowered.starts_with("shift-") {
            mods |= M::SHIFT;
            remaining = &remaining[6..];
        } else {
            break;
        }
    }
    let lowered = remaining.to_ascii_lowercase();
    let code = match lowered.as_str() {
        "enter" => KeyCode::Enter,
        "esc" | "escape" => KeyCode::Esc,
        "tab" => KeyCode::Tab,
        "backtab" => KeyCode::BackTab,
        "backspace" => KeyCode::Backspace,
        "delete" => KeyCode::Delete,
        "left" => KeyCode::Left,
        "right" => KeyCode::Right,
        "up" => KeyCode::Up,
        "down" => KeyCode::Down,
        "home" => KeyCode::Home,
        "end" => KeyCode::End,
        "pageup" => KeyCode::PageUp,
        "pagedown" => KeyCode::PageDown,
        single if matches!(single.chars().collect::<Vec<char>>()[..], [_]) => {
            let mut chars = single.chars();
            KeyCode::Char(chars.next().unwrap_or_default())
        }
        _ => return None,
    };
    Some((ctx, spec(code, mods)))
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;

    #[test]
    fn default_map_matches_legacy_map_key() {
        let km = Keymap::default();
        let cases = [
            (
                KeyCode::Char('p'),
                M::CONTROL,
                false,
                Action::CommandPalette,
            ),
            (KeyCode::Char('c'), M::CONTROL, true, Action::Interrupt),
            (KeyCode::Char('c'), M::CONTROL, false, Action::CancelOrClear),
            (
                KeyCode::Char('c'),
                M::CONTROL | M::SHIFT,
                false,
                Action::Exit,
            ),
            (KeyCode::Tab, M::NONE, true, Action::Queue),
            (KeyCode::Tab, M::NONE, false, Action::Complete),
            (KeyCode::Esc, M::NONE, true, Action::CancelOrClear),
            (KeyCode::Esc, M::NONE, false, Action::Ignore),
            (KeyCode::Enter, M::NONE, false, Action::Submit),
            (KeyCode::BackTab, M::NONE, true, Action::Queue),
            (
                KeyCode::Char('g'),
                M::CONTROL,
                false,
                Action::ExternalEditor,
            ),
            (KeyCode::Char('o'), M::CONTROL, false, Action::ExpandStash),
            (KeyCode::Char('j'), M::CONTROL, false, Action::Insert('\n')),
        ];
        for (code, mods, running, expected) in cases {
            assert_eq!(
                km.action_for(running, code, mods),
                expected,
                "{code:?} {mods:?} running={running}"
            );
        }
        assert_eq!(
            km.action_for(true, KeyCode::Char('?'), M::NONE),
            Action::Insert('?')
        );
    }

    #[test]
    fn parse_binding_understands_prefixes_and_contexts() {
        let (_, s) = parse_binding("Ctrl-p").unwrap();
        assert_eq!(s.code, KeyCode::Char('p'));
        assert_eq!(s.mods, M::CONTROL);

        let (ctx, s) = parse_binding("running|alt-enter").unwrap();
        assert_eq!(ctx, Ctx::Running);
        assert_eq!(s.code, KeyCode::Enter);
        assert_eq!(s.mods, M::ALT);

        let (ctx, _) = parse_binding("composer|Ctrl-u").unwrap();
        assert_eq!(ctx, Ctx::Composer);

        assert!(parse_binding("bogus|Ctrl-q").is_none());
        assert!(parse_binding("Ctrl-f13").is_none());
    }

    #[test]
    fn overrides_rebind_and_conflicts_detect() {
        let km = Keymap::default();
        assert!(km.conflicts().is_empty());

        // Same-scope rebind replaces.
        let mut rebound = km.clone();
        rebound.bind(
            Ctx::Both,
            spec(KeyCode::Char('p'), M::CONTROL),
            Action::Exit,
        );
        assert_eq!(
            rebound.action_for(false, KeyCode::Char('p'), M::CONTROL),
            Action::Exit
        );
        assert!(rebound.conflicts().is_empty());

        // Cross-scope shadowing is allowed but surfaced as a conflict.
        let mut shadowed = km.clone();
        shadowed.bind(
            Ctx::Composer,
            spec(KeyCode::Char('p'), M::CONTROL),
            Action::Exit,
        );
        let conflicts = shadowed.conflicts();
        assert_eq!(conflicts.len(), 1);
        assert_eq!(conflicts[0].0, "Ctrl-p");
        assert_eq!(
            shadowed.action_for(false, KeyCode::Char('p'), M::CONTROL),
            Action::Exit
        );

        // Config overrides apply; garbage rows are skipped silently.
        let mut overrides = std::collections::BTreeMap::new();
        overrides.insert("Ctrl-g".to_string(), "command-palette".to_string());
        overrides.insert("not-a-key".to_string(), "undo".to_string());
        overrides.insert("Ctrl-x".to_string(), "not-an-action".to_string());
        let applied = Keymap::default().with_overrides(&overrides);
        assert_eq!(
            applied.action_for(false, KeyCode::Char('g'), M::CONTROL),
            Action::CommandPalette
        );
        assert_eq!(
            applied.action_for(false, KeyCode::Char('x'), M::CONTROL),
            Action::Ignore
        );
    }

    #[test]
    fn rebind_named_moves_all_bindings_and_preserves_context() {
        let mut km = Keymap::default();
        // Composer-scoped action keeps its scope after a rebind.
        assert!(km.rebind_named("external-editor", spec(KeyCode::F(4), M::NONE),));
        assert_eq!(
            km.action_for(false, KeyCode::F(4), M::NONE),
            Action::ExternalEditor
        );
        assert_eq!(
            km.action_for(false, KeyCode::Char('g'), M::CONTROL),
            Action::Ignore
        );

        // Both-scoped action stays both-scoped; old key is freed.
        assert!(km.rebind_named("command-palette", spec(KeyCode::Char('z'), M::CONTROL)));
        assert_eq!(
            km.action_for(false, KeyCode::Char('z'), M::CONTROL),
            Action::CommandPalette
        );
        // The displaced Ctrl-P now falls through: modified keys never
        // become plain inserts.
        assert_eq!(
            km.action_for(false, KeyCode::Char('p'), M::CONTROL),
            Action::Ignore
        );

        // Unknown or non-rebindable names fail without mutating.
        let before = km.clone();
        assert!(!km.rebind_named("insert", spec(KeyCode::F(9), M::NONE)));
        assert!(!km.rebind_named("no-such-action", spec(KeyCode::F(9), M::NONE)));
        assert_eq!(km.entries.len(), before.entries.len());
    }

    #[test]
    fn copy_and_subagent_actions_are_bound_and_named() {
        let km = Keymap::default();
        assert_eq!(
            km.action_for(false, KeyCode::Char('y'), M::ALT),
            Action::CopyResponse
        );
        assert_eq!(
            km.action_for(true, KeyCode::Char('s'), M::ALT),
            Action::Subagents
        );
        assert_eq!(Action::CopyResponse.name(), "copy-response");
        assert_eq!(Action::parse("copy-response"), Some(Action::CopyResponse));
        assert_eq!(Action::Subagents.name(), "subagents");
        assert!(km.conflicts().is_empty());
    }
}
