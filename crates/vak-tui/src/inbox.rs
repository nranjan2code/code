//! `/inbox`: the durable notification ledger — badge, table rendering,
//! two-tier id resolution, and the ack confirm flow, all over the base's
//! inbox API.

use crossterm::style::Color;
use vak_client::types::InboxEntry;

use crate::data::ClientData;
use crate::events::trunc_one;
use crate::render::Screen;
use crate::state::ModalView;
use crate::theme::{self, Theme};

/// Scan bound mirroring the base's `MAX_SCAN` so a full-ledger view and the
/// server's own reads stay in lockstep.
const INBOX_SCAN_LIMIT: u32 = 10_000;

const INBOX_TITLE_CELLS: usize = 48;

/// Idle status-row unread marker: ` · ✉ N` appended to the composer label.
/// Empty while nothing awaits attention so the row stays clean at zero.
pub fn inbox_badge(unread: u32) -> String {
    if unread == 0 {
        String::new()
    } else {
        format!(" · ✉ {unread}")
    }
}

/// Titles collapse whitespace and clip to a cell budget (CLI parity).
fn inbox_title(title: &str, max: usize) -> String {
    let flat = title.split_whitespace().collect::<Vec<_>>().join(" ");
    trunc_one(&flat, max)
}

/// Short display chip per notification kind; labels match `vakcoder inbox`.
fn kind_chip(kind: &str) -> &'static str {
    match kind {
        "task_summary" => "task",
        "approval_pending" => "approval",
        "approval_denied" => "denied",
        "budget_alert" => "budget",
        "digest" => "digest",
        "heartbeat" => "beat",
        "proposal_opened" => "proposal",
        _ => "other",
    }
}

/// Theme slot per kind — approvals and budget alerts warn, denials error,
/// digests succeed, the rest take their house accent/dim/code slots.
fn kind_color(kind: &str, theme: &Theme) -> Color {
    match kind {
        "task_summary" => theme.accent,
        "approval_pending" => theme.warning,
        "approval_denied" => theme.error,
        "budget_alert" => theme.warning,
        "digest" => theme.success,
        "heartbeat" => theme.dim,
        "proposal_opened" => theme.code,
        _ => theme.dim,
    }
}

/// One `/inbox` table row: [✉ ] rel-ts · colored kind chip · clipped title
/// · id8. The chip embeds its SGR like the budget marker does; the base
/// heading color is restored after so the rest of the row renders uniformly.
/// Entries whose id appears in `unread_ids` carry the ✉ marker.
pub fn inbox_row(entry: &InboxEntry, theme: &Theme, unread_ids: &[String]) -> String {
    let marker = if unread_ids.iter().any(|id| id == &entry.id) {
        "✉ "
    } else {
        ""
    };
    format!(
        "{marker}{} · {}{}{} · {} · {}",
        rel_time(&entry.ts),
        theme::fg(kind_color(&entry.kind, theme)),
        kind_chip(&entry.kind),
        theme::fg(theme.heading),
        inbox_title(&entry.title, INBOX_TITLE_CELLS),
        short_hex(&entry.id),
    )
}

fn inbox_rows(entries: &[InboxEntry], theme: &Theme, unread_ids: &[String]) -> Vec<String> {
    entries
        .iter()
        .map(|e| inbox_row(e, theme, unread_ids))
        .collect()
}

// `short_hex` lives beside /memory too; duplicated here because the commands
// module tree is private to its own subtree.
fn short_hex(hex: &str) -> String {
    hex.chars().take(8).collect()
}

fn rel_time(ts: &str) -> String {
    match chrono::DateTime::parse_from_rfc3339(ts) {
        Ok(t) => {
            let secs = (chrono::Utc::now() - t.with_timezone(&chrono::Utc))
                .num_seconds()
                .max(0) as u64;
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
        Err(_) => "?".to_string(),
    }
}

/// Parsed `/inbox [all|ack <id>]` payload. Err carries the usage line.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum InboxAction {
    List { all: bool },
    Ack(String),
}

impl InboxAction {
    /// Bare/blank lists unread; `all` includes acked entries;
    /// `ack <id8-prefix>` arms the inline confirm.
    pub fn parse(arg: Option<&str>) -> Result<Self, String> {
        const USAGE: &str = "usage: /inbox [all|ack <id8-prefix>]";
        let Some(arg) = arg.map(str::trim).filter(|a| !a.is_empty()) else {
            return Ok(Self::List { all: false });
        };
        let mut tokens = arg.splitn(2, char::is_whitespace);
        match tokens.next().unwrap_or("") {
            "all" => Ok(Self::List { all: true }),
            "ack" | "done" | "read" => {
                let id = tokens.next().map(str::trim).unwrap_or("");
                if id.is_empty() {
                    Err(USAGE.to_string())
                } else {
                    Ok(Self::Ack(id.to_string()))
                }
            }
            _ => Err(USAGE.to_string()),
        }
    }
}

/// Why a `/inbox ack <prefix>` id could not be resolved.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum InboxMatchError {
    NotFound(String),
    Ambiguous(String),
}

impl std::fmt::Display for InboxMatchError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::NotFound(given) => write!(f, "no inbox entry matches '{given}'"),
            Self::Ambiguous(given) => {
                write!(
                    f,
                    "'{given}' matches several inbox entries — use more of the id"
                )
            }
        }
    }
}

/// Resolve a user-supplied entry id (full 16-hex or unique prefix) against
/// UNREAD entries first, then the whole ledger; an exact full-id hit wins
/// over any prefix set. Uniqueness is judged per tier, so one unread entry
/// shadows acked entries sharing its prefix.
pub fn resolve_entry<'a>(
    unread: &'a [InboxEntry],
    all: &'a [InboxEntry],
    given: &str,
) -> Result<&'a InboxEntry, InboxMatchError> {
    enum Tier {
        NotFound,
        Ambiguous,
    }
    fn tier<'a>(entries: &'a [InboxEntry], given: &str) -> Result<&'a InboxEntry, Tier> {
        if let Some(e) = entries.iter().find(|e| e.id == given) {
            return Ok(e);
        }
        let hits: Vec<&InboxEntry> = entries.iter().filter(|e| e.id.starts_with(given)).collect();
        match hits.as_slice() {
            [one] => Ok(one),
            [] => Err(Tier::NotFound),
            _ => Err(Tier::Ambiguous),
        }
    }
    match tier(unread, given) {
        Ok(e) => Ok(e),
        Err(Tier::Ambiguous) => Err(InboxMatchError::Ambiguous(given.to_string())),
        Err(Tier::NotFound) => tier(all, given).map_err(|t| match t {
            Tier::NotFound => InboxMatchError::NotFound(given.to_string()),
            Tier::Ambiguous => InboxMatchError::Ambiguous(given.to_string()),
        }),
    }
}

/// An `/inbox ack <prefix>` armed by the command, awaiting y/n.
#[derive(Debug, Clone)]
pub struct InboxAck {
    pub id: String,
    pub preview: String,
}

/// Inline confirm card for marking an inbox entry read — same shape as the
/// forget card so destructive-op interactions read identically.
pub fn draw_ack_confirm(screen: &mut Screen, theme: &Theme, req: &InboxAck) {
    let styled = |text: String, color: Color| {
        format!("{}{text}{}", theme::fg(color), theme::fg(Color::Reset))
    };
    let lines = vec![
        styled(
            format!("╭─ confirm ack · {}", short_hex(&req.id)),
            theme.warning,
        ),
        styled(format!("│ {}", req.preview), theme.dim),
        styled("╰─ y mark read · n/Esc keep unread".to_string(), theme.dim),
    ];
    screen.redraw_block(&lines);
}

/// What `/inbox` leaves for the caller to do after messages are printed.
pub enum InboxOutcome {
    Handled,
    /// A listing to open in the modal layer; `entries` are the displayed
    /// rows (unread-only unless `all`) and `unread` is the live count for
    /// the caller's badge cache.
    Listed {
        entries: Vec<InboxEntry>,
        unread: u32,
        modal: ModalView,
    },
    /// A `/inbox ack <id>` resolved to this entry; on `y` the caller
    /// invokes [`ack_confirmed`].
    ConfirmAck {
        entry_id: String,
        preview: String,
    },
}

/// `/inbox [all|ack <id8-prefix>]`. Refreshes the ledger over the API,
/// prints its own feedback, and returns listings or armed confirms.
pub async fn handle_inbox(
    data: &ClientData,
    arg: Option<&str>,
    theme: &Theme,
    screen: &mut Screen,
) -> InboxOutcome {
    match InboxAction::parse(arg) {
        Err(usage) => {
            screen.error(&usage);
            InboxOutcome::Handled
        }
        Ok(InboxAction::List { all }) => {
            let (unread_entries, unread_count) =
                match data.client().inbox_list(INBOX_SCAN_LIMIT, true).await {
                    Ok(resp) => (resp.entries, resp.unread_count),
                    Err(e) => {
                        screen.error(&format!("inbox unavailable: {e}"));
                        return InboxOutcome::Handled;
                    }
                };
            // The full ledger feeds the footer's total and degrades to an
            // empty ledger if its own fetch fails.
            let full = data.client().inbox_list(INBOX_SCAN_LIMIT, false).await.ok();
            let total = full.as_ref().map_or(0, |resp| resp.entries.len());
            let all_entries = full.map(|resp| resp.entries).unwrap_or_default();
            let unread_ids: Vec<String> = unread_entries.iter().map(|e| e.id.clone()).collect();
            let entries = if all { all_entries } else { unread_entries };
            let rows = if entries.is_empty() && total == 0 {
                vec![
                    "no inbox entries yet".into(),
                    "gateway pushes, task summaries, and budget alerts land here".into(),
                    "/inbox ack <id8> marks an entry read".into(),
                ]
            } else if entries.is_empty() {
                vec![
                    "no unread entries".into(),
                    format!("{total} already read — /inbox all shows them"),
                ]
            } else {
                inbox_rows(&entries, theme, &unread_ids)
            };
            let modal = ModalView {
                title: "inbox".to_string(),
                rows,
                scroll: 0,
                footer: format!(
                    "{} unread of {} · ↑↓ select · Enter opens linked session · /inbox ack <id8>",
                    unread_count, total
                ),
                selected: (!entries.is_empty()).then_some(0),
                ..Default::default()
            };
            InboxOutcome::Listed {
                entries,
                unread: unread_count,
                modal,
            }
        }
        Ok(InboxAction::Ack(given)) => {
            // Unread entries resolve first; the full ledger is the fallback
            // tier. Both sides degrade independently on fetch errors.
            let unread_list = data
                .client()
                .inbox_list(INBOX_SCAN_LIMIT, true)
                .await
                .map(|r| r.entries)
                .unwrap_or_default();
            let all_list = data
                .client()
                .inbox_list(INBOX_SCAN_LIMIT, false)
                .await
                .map(|r| r.entries)
                .unwrap_or_else(|_| unread_list.clone());
            match resolve_entry(&unread_list, &all_list, &given) {
                Err(e) => {
                    screen.error(&e.to_string());
                    InboxOutcome::Handled
                }
                Ok(entry) => InboxOutcome::ConfirmAck {
                    entry_id: entry.id.clone(),
                    preview: inbox_title(&entry.title, 80),
                },
            }
        }
    }
}

/// The `y` half of the [`InboxOutcome::ConfirmAck`] flow: acks the entry and
/// returns the refreshed unread total for the caller's badge cache.
pub async fn ack_confirmed(data: &ClientData, entry_id: &str, screen: &mut Screen) -> u32 {
    match data.inbox_ack(entry_id).await {
        Ok(_acked) => {
            screen.success(&format!("✓ acked {}", short_hex(entry_id)));
            data.inbox_unread_count().await.unwrap_or(0)
        }
        Err(e) => {
            screen.error(&format!("ack failed: {e}"));
            data.inbox_unread_count().await.unwrap_or(0)
        }
    }
}

#[cfg(test)]
mod inbox_tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]
    use super::*;
    use crate::markdown::strip_ansi;

    fn entry(id: &str, kind: &str, title: &str) -> InboxEntry {
        InboxEntry {
            id: id.into(),
            ts: chrono::Utc::now().to_rfc3339(),
            kind: kind.into(),
            title: title.into(),
            body: String::new(),
            session_id: None,
            task_id: None,
        }
    }

    #[test]
    fn inbox_prefix_resolution_prefers_unread_then_all() {
        let unread_entry = entry("aaaa111111111111", "digest", "unread");
        let acked_a = entry("aaaa222222222222", "digest", "read one");
        let acked_b = entry("bbbb333333333333", "digest", "read two");
        let unread = vec![unread_entry.clone()];
        let all = vec![unread_entry.clone(), acked_a, acked_b.clone()];
        let id_of = |r: Result<&InboxEntry, InboxMatchError>| r.map(|e| e.id.clone());

        // Exact full-id hit wins even when it lives in the acked tier only.
        assert_eq!(
            id_of(resolve_entry(&unread, &all, "aaaa222222222222")),
            Ok("aaaa222222222222".to_string())
        );
        // Unique prefix in the unread tier shadows acked entries sharing it.
        assert_eq!(
            id_of(resolve_entry(&unread, &all, "aaaa")),
            Ok("aaaa111111111111".to_string())
        );
        // Prefix unique only among acked entries falls through to the ledger.
        assert_eq!(id_of(resolve_entry(&[], &all, "bbbb")), Ok(acked_b.id));
        // Ambiguity inside the ledger tier is refused.
        assert_eq!(
            resolve_entry(&[], &all, "aaaa").err(),
            Some(InboxMatchError::Ambiguous("aaaa".into()))
        );
        assert_eq!(
            resolve_entry(&unread, &all, "ffff").err(),
            Some(InboxMatchError::NotFound("ffff".into()))
        );
    }

    #[test]
    fn inbox_arg_grammar_splits_all_and_ack() {
        assert!(matches!(
            InboxAction::parse(None),
            Ok(InboxAction::List { all: false })
        ));
        assert!(matches!(
            InboxAction::parse(Some("   ")),
            Ok(InboxAction::List { all: false })
        ));
        assert!(matches!(
            InboxAction::parse(Some("all")),
            Ok(InboxAction::List { all: true })
        ));
        assert!(matches!(
            InboxAction::parse(Some("ack abc12345")),
            Ok(InboxAction::Ack(ref id)) if id == "abc12345"
        ));
        // Alias verbs share the ack arm.
        assert!(matches!(
            InboxAction::parse(Some("done abc12345")),
            Ok(InboxAction::Ack(ref id)) if id == "abc12345"
        ));
        // Missing id and unknown actions are usage errors, never guesses.
        assert!(InboxAction::parse(Some("ack")).is_err());
        assert!(InboxAction::parse(Some("wat")).is_err());
    }

    #[test]
    fn inbox_chips_cover_every_kind_with_theme_colors() {
        let theme = Theme::from_name("dark");
        for kind in [
            "task_summary",
            "approval_pending",
            "approval_denied",
            "budget_alert",
            "digest",
            "heartbeat",
            "proposal_opened",
        ] {
            let chip = kind_chip(kind);
            assert!(chip.chars().count() <= 8, "{chip} breaks the column");
            // Every kind carries its own theme slot, never the default fg.
            assert_ne!(kind_color(kind, &theme), Color::Reset);
        }
        // Distinct kinds land in distinct slots where semantics differ.
        assert_ne!(
            kind_color("approval_denied", &theme),
            kind_color("digest", &theme)
        );

        let e = entry("0123456789abcdef", "budget_alert", "day cap 92%");
        let row = inbox_row(&e, &theme, &[]);
        // The chip is embedded with its theme color; the base heading color
        // is restored after so the row tail stays uniform.
        assert!(row.contains(&theme::fg(theme.warning)), "{row}");
        assert!(row.contains(&theme::fg(theme.heading)));
        assert!(strip_ansi(&row).contains("budget"));
    }

    #[test]
    fn inbox_rows_render_rel_ts_chip_title_and_id8() {
        let theme = Theme::from_name("dark");
        let e = entry("0123456789abcdef", "digest", "weekly   digest\nline two");
        let plain = strip_ansi(&inbox_row(&e, &theme, &[]));
        assert_eq!(
            plain, "now · digest · weekly digest line two · 01234567",
            "rel-ts · chip · collapsed title · id8"
        );

        let long = entry(
            "ffffffffffffffff",
            "heartbeat",
            &format!("{} tail", "x".repeat(60)),
        );
        let plain = strip_ansi(&inbox_row(&long, &theme, &[]));
        assert!(plain.contains('…'), "{plain}");
        assert!(!plain.contains("tail"), "{plain}");

        // Unread ids gain the ✉ marker ahead of the timestamp.
        let marked = strip_ansi(&inbox_row(&e, &theme, std::slice::from_ref(&e.id)));
        assert!(marked.starts_with("✉ now · "), "{marked}");
    }

    #[test]
    fn inbox_badge_gates_on_positive_unread() {
        assert_eq!(inbox_badge(0), "");
        assert_eq!(inbox_badge(12), " · ✉ 12");
    }
}
