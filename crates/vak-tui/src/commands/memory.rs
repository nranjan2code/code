//! `/memory`: durable notes across the workspace and profile tiers, served
//! by the base's memory API. Ids resolve by full id or unique 8-char prefix;
//! the destructive forget returns an armed confirm for the caller to drive.

use chrono::{DateTime, Utc};
use vak_client::types::MemoryNote;

use crate::data::ClientData;
use crate::events::trunc_one;
use crate::render::Screen;
use crate::state::ModalView;

/// Parsed `/memory [--profile] <form>` payload.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MemoryAction {
    List,
    Append(String),
    Forget(String),
    Amend(String, String),
}

impl MemoryAction {
    /// Empty arg lists; otherwise the first token selects forget/amend and
    /// anything else is new-note text.
    pub fn parse(arg: Option<&str>) -> Self {
        let Some(arg) = arg.map(str::trim).filter(|a| !a.is_empty()) else {
            return Self::List;
        };
        let mut tokens = arg.splitn(2, char::is_whitespace);
        match tokens.next().unwrap_or("") {
            "forget" | "rm" => Self::Forget(tokens.next().unwrap_or("").trim().to_string()),
            "amend" => {
                let rest = tokens.next().unwrap_or("").trim();
                match rest.split_once(char::is_whitespace) {
                    Some((id, text)) => Self::Amend(id.trim().to_string(), text.trim().to_string()),
                    None => Self::Amend(rest.to_string(), String::new()),
                }
            }
            _ => Self::Append(arg.to_string()),
        }
    }
}

/// What `/memory` leaves for the caller to do after messages are printed.
pub enum MemoryOutcome {
    Handled,
    /// A listing to open in the modal layer.
    Modal(ModalView),
    /// A `/memory forget <id>` resolved to this note; on `y` the caller
    /// invokes [`forget_confirmed`].
    ConfirmForget {
        note_id: String,
        preview: String,
    },
}

/// First 8 chars of a hex id — display form only; all stored operations run
/// on full ids.
pub fn short_hex(hex: &str) -> String {
    hex.chars().take(8).collect()
}

/// Relative timestamp for recall surfaces over an RFC3339 string:
/// now / 5m ago / 3h ago / 2d ago. Unparseable stamps render as "?" rather
/// than a wrong bucket.
pub fn rel_time(ts: &str) -> String {
    match DateTime::parse_from_rfc3339(ts) {
        Ok(t) => rel_time_instant(t.with_timezone(&Utc)),
        Err(_) => "?".to_string(),
    }
}

fn rel_time_instant(ts: DateTime<Utc>) -> String {
    let secs = (Utc::now() - ts).num_seconds().max(0) as u64;
    if secs < 60 {
        "now".to_string()
    } else if secs < 3600 {
        format!("{}m ago", secs / 60)
    } else if secs < 86_400 {
        format!("{}h ago", secs / 3600)
    } else {
        format!("{}d ago", secs / 86_400)
    }
}

/// Why a `/memory forget|amend <prefix>` id could not be resolved.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum NoteMatchError {
    NotFound(String),
    Ambiguous(String),
}

impl std::fmt::Display for NoteMatchError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::NotFound(given) => write!(f, "no note matches id '{given}'"),
            Self::Ambiguous(given) => {
                write!(f, "'{given}' matches several notes — use more of the id")
            }
        }
    }
}

/// Resolve a user-supplied note id (full 16-hex or unique prefix) to exactly
/// one note. An exact full-id hit wins even if it is also another's prefix.
pub fn find_note<'a>(
    notes: &'a [MemoryNote],
    given: &str,
) -> Result<&'a MemoryNote, NoteMatchError> {
    if let Some(note) = notes.iter().find(|n| n.id == given) {
        return Ok(note);
    }
    let matches: Vec<&MemoryNote> = notes.iter().filter(|n| n.id.starts_with(given)).collect();
    match matches.as_slice() {
        [one] => Ok(one),
        [] => Err(NoteMatchError::NotFound(given.to_string())),
        _ => Err(NoteMatchError::Ambiguous(given.to_string())),
    }
}

/// One `/memory` modal row: `id8 [tier] flat text`, newest first.
pub fn memory_rows(notes: &[MemoryNote]) -> Vec<String> {
    notes
        .iter()
        .rev()
        .map(|n| {
            format!(
                "{} [{}] {}",
                short_hex(&n.id),
                n.tier,
                n.text.replace('\n', " ")
            )
        })
        .collect()
}

/// `/memory [--profile] [<text>|forget <id>|amend <id> <text>]`. Prints its
/// own feedback; listings and armed confirms come back as the outcome.
pub async fn handle_memory(
    data: &ClientData,
    arg: Option<&str>,
    screen: &mut Screen,
) -> MemoryOutcome {
    let raw = arg.map(str::trim);
    let (profile_scope, rest) = match raw {
        Some(rest) if rest == "--profile" || rest.starts_with("--profile ") => {
            (true, Some(rest["--profile".len()..].trim().to_string()))
        }
        other => (false, other.map(String::from)),
    };
    let all_notes = match data.memory().await {
        Ok(notes) => notes,
        Err(e) => {
            screen.error(&format!("memory unavailable: {e}"));
            return MemoryOutcome::Handled;
        }
    };
    let notes: Vec<MemoryNote> = if profile_scope {
        all_notes
            .into_iter()
            .filter(|n| n.tier == "profile")
            .collect()
    } else {
        all_notes
    };
    let action = MemoryAction::parse(rest.as_deref());
    match action {
        MemoryAction::List => {
            let tier_label = if profile_scope {
                "profile"
            } else {
                "workspace · profile"
            };
            let rows = if notes.is_empty() {
                vec![
                    "no notes yet".to_string(),
                    "the model saves via its remember tool; /memory <text> saves one directly"
                        .to_string(),
                    "/memory --profile <text> saves to the profile tier that follows you across projects"
                        .to_string(),
                ]
            } else {
                memory_rows(&notes)
            };
            MemoryOutcome::Modal(ModalView {
                title: format!("memory · {tier_label} · {} note(s)", notes.len()),
                rows,
                scroll: 0,
                footer:
                    "/memory forget <id> · /memory amend <id> <text> · ids accept the 8-char prefix · Esc close"
                        .to_string(),
                ..Default::default()
            })
        }
        MemoryAction::Append(text) => {
            let tier = if profile_scope {
                "profile"
            } else {
                "workspace"
            };
            match data.append_memory(&text, tier).await {
                Ok(()) => screen.success(if profile_scope {
                    "note saved to profile memory"
                } else {
                    "note saved to durable memory"
                }),
                Err(e) => screen.error(&format!("save failed: {e}")),
            }
            MemoryOutcome::Handled
        }
        MemoryAction::Forget(given) => {
            if given.is_empty() {
                screen.error("usage: /memory forget <id> — ids show in /memory");
            } else {
                match find_note(&notes, &given) {
                    Err(e) => screen.error(&e.to_string()),
                    Ok(note) => {
                        return MemoryOutcome::ConfirmForget {
                            note_id: note.id.clone(),
                            preview: trunc_one(&note.text.replace('\n', " "), 80),
                        };
                    }
                }
            }
            MemoryOutcome::Handled
        }
        MemoryAction::Amend(given, text) => {
            if given.is_empty() || text.trim().is_empty() {
                screen.error("usage: /memory amend <id> <new text>");
            } else {
                match find_note(&notes, &given) {
                    Err(e) => screen.error(&e.to_string()),
                    Ok(note) => {
                        let full = note.id.clone();
                        match data.amend_memory(&full, &text).await {
                            Ok(()) => {
                                screen.success(&format!("✓ amended note {}", short_hex(&full),))
                            }
                            Err(e) => screen.error(&format!("amend failed: {e}")),
                        }
                    }
                }
            }
            MemoryOutcome::Handled
        }
    }
}

/// The `y` half of the [`MemoryOutcome::ConfirmForget`] flow.
pub async fn forget_confirmed(data: &ClientData, note_id: &str, screen: &mut Screen) {
    match data.forget_memory(note_id).await {
        Ok(()) => screen.success(&format!("✓ forgot note {}", short_hex(note_id))),
        Err(e) => screen.error(&format!("forget failed: {e}")),
    }
}

#[cfg(test)]
mod memory_tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]
    use super::*;

    fn note(id: &str, text: &str) -> MemoryNote {
        MemoryNote {
            id: id.into(),
            text: text.into(),
            created_at: Utc::now().to_rfc3339(),
            tier: "workspace".into(),
        }
    }

    #[test]
    fn memory_ids_resolve_by_full_or_unique_prefix_only() {
        let a = note("aaaaaaaa11111111", "alpha");
        let b = note("bbbbbbbb22222222", "beta");
        let c = note(
            "aaaaaaaac3333333",
            "gamma shares an 8-char prefix with alpha",
        );
        let notes = vec![a, b.clone(), c.clone()];

        assert_eq!(find_note(&notes, "bbbbbbbb22222222").unwrap().id, b.id);

        assert_eq!(find_note(&notes, "bbbbbbbb").unwrap().id, b.id);
        assert_eq!(find_note(&notes, "aaaaaaaac3").unwrap().id, c.id);

        // Ambiguous prefix is refused, not silently resolved.
        assert_eq!(
            find_note(&notes, "aaaaaaaa").err(),
            Some(NoteMatchError::Ambiguous("aaaaaaaa".into()))
        );
        // Unknown prefix.
        assert_eq!(
            find_note(&notes, "deadbeef").err(),
            Some(NoteMatchError::NotFound("deadbeef".into()))
        );
    }

    #[test]
    fn memory_arg_grammar_splits_forget_amend_append() {
        use MemoryAction::{Amend, Append, Forget, List};
        assert!(matches!(MemoryAction::parse(None), List));
        assert!(matches!(MemoryAction::parse(Some("   ")), List));
        assert!(matches!(MemoryAction::parse(Some("ship it")), Append(t) if t == "ship it"));
        assert!(
            matches!(MemoryAction::parse(Some("forget abc12345")), Forget(id) if id == "abc12345")
        );
        assert!(matches!(MemoryAction::parse(Some("rm abc12345")), Forget(id) if id == "abc12345"));
        assert!(matches!(
            MemoryAction::parse(Some("amend abc12345 new body text")),
            Amend(id, text) if id == "abc12345" && text == "new body text"
        ));
        // Missing amend body parses here; handle_memory rejects the empty text.
        assert!(matches!(
            MemoryAction::parse(Some("amend abc12345")),
            Amend(id, text) if id == "abc12345" && text.is_empty()
        ));
    }

    #[test]
    fn memory_rows_show_newest_first_with_tier_chips() {
        let old = note("1111111111111111", "older\nnote");
        let new = MemoryNote {
            tier: "profile".into(),
            ..note("2222222222222222", "newer")
        };
        let rows = memory_rows(&[old, new]);
        assert_eq!(
            rows,
            vec![
                "22222222 [profile] newer",
                "11111111 [workspace] older note"
            ]
        );
    }

    #[test]
    fn rel_time_buckets_are_compact() {
        let now = Utc::now();
        let stamp = |t: DateTime<Utc>| t.to_rfc3339();
        assert_eq!(rel_time(&stamp(now)), "now");
        assert_eq!(
            rel_time(&stamp(now - chrono::Duration::minutes(5))),
            "5m ago"
        );
        assert_eq!(rel_time(&stamp(now - chrono::Duration::hours(3))), "3h ago");
        assert_eq!(rel_time(&stamp(now - chrono::Duration::days(2))), "2d ago");
        // Future timestamps clamp instead of going negative.
        assert_eq!(rel_time(&stamp(now + chrono::Duration::hours(1))), "now");
        // Malformed stamps never invent a bucket.
        assert_eq!(rel_time("not-a-timestamp"), "?");
    }
}
