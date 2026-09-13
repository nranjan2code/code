//! `vak inbox` (docs/design/29-personal-os.md P6): list/show/ack/count
//! over `vak_core::inbox` (`<home>/inbox.jsonl`) without the server, the way
//! `tasks`/`memory` read their stores directly. Row/footer/count rendering
//! lives in pure helpers so tests assert strings instead of captured stdout.

use std::path::{Path, PathBuf};

use vak_core::Core;
use vak_core::inbox::{self, Entry, Kind};

const DEFAULT_LIMIT: usize = 50;
const TITLE_WIDTH: usize = 48;

pub(crate) fn run_inbox(cwd: PathBuf, action: Option<crate::cli::InboxAction>) -> i32 {
    let core = match Core::new(cwd) {
        Ok(c) => c,
        Err(e) => {
            eprintln!("error: {e}");
            return 2;
        }
    };
    run_with_home(
        &core.sessions_home(),
        action.unwrap_or(crate::cli::InboxAction::List {
            all: false,
            limit: DEFAULT_LIMIT,
        }),
    )
}

fn run_with_home(home: &Path, action: crate::cli::InboxAction) -> i32 {
    match action {
        crate::cli::InboxAction::List { all, limit } => {
            let limit = limit.clamp(1, inbox::MAX_SCAN);
            let rows = if all {
                all_rows(home, limit)
            } else {
                unread_rows(home, limit)
            };
            if rows.is_empty() {
                println!(
                    "{}",
                    if all {
                        "no inbox entries yet"
                    } else {
                        "no unread inbox entries"
                    }
                );
                return 0;
            }
            for row in rows {
                println!("{row}");
            }
            println!("{}", footer(home));
            0
        }
        crate::cli::InboxAction::Show { id_prefix } => match load_entry(home, &id_prefix) {
            Ok(entry) => {
                print_show(&entry);
                0
            }
            Err(e) => {
                eprintln!("error: {e}");
                1
            }
        },
        crate::cli::InboxAction::Ack { id_prefix } => match ack_prefix(home, &id_prefix) {
            Ok(true) => {
                println!("acked");
                0
            }
            Ok(false) => {
                println!("already acked");
                0
            }
            Err(e) => {
                eprintln!("error: {e}");
                1
            }
        },
        crate::cli::InboxAction::Count => {
            println!("{}", count_line(home));
            0
        }
    }
}

fn ledger(home: &Path) -> Vec<Entry> {
    inbox::list(home, inbox::MAX_SCAN)
}

fn all_rows(home: &Path, limit: usize) -> Vec<String> {
    ledger(home)
        .into_iter()
        .take(limit)
        .map(|e| format_row(&e))
        .collect()
}

fn unread_rows(home: &Path, limit: usize) -> Vec<String> {
    inbox::unread(home)
        .into_iter()
        .take(limit)
        .map(|e| format_row(&e))
        .collect()
}

fn footer(home: &Path) -> String {
    format!(
        "{} unread of {} total",
        inbox::unread_count(home),
        ledger(home).len()
    )
}

fn count_line(home: &Path) -> String {
    format!("{} unread", inbox::unread_count(home))
}

fn format_row(e: &Entry) -> String {
    format_row_rel(e, &rel_ts(e.ts))
}

fn format_row_rel(e: &Entry, rel: &str) -> String {
    format!(
        "{:<8}  {:<8}  {:<width$}  {}",
        rel,
        kind_chip(e.kind),
        truncate_title(&e.title),
        short_hex(&e.id),
        width = TITLE_WIDTH
    )
}

fn print_show(e: &Entry) {
    println!("kind      {}", kind_chip(e.kind));
    println!("when      {} ({})", e.ts.to_rfc3339(), rel_ts(e.ts));
    println!("id        {}", e.id);
    println!("title     {}", truncate_title(&e.title));
    if !e.body.is_empty() {
        println!();
        println!("{}", e.body);
    }
    if let Some(s) = &e.session_id {
        println!("session   {s}");
    }
    if let Some(t) = &e.task_id {
        println!("task      {t}");
    }
}

fn kind_chip(kind: Kind) -> &'static str {
    match kind {
        Kind::TaskSummary => "task",
        Kind::ApprovalPending => "approval",
        Kind::ApprovalDenied => "denied",
        Kind::BudgetAlert => "budget",
        Kind::Digest => "digest",
        Kind::Heartbeat => "beat",
        Kind::ProposalOpened => "proposal",
    }
}

fn truncate_title(title: &str) -> String {
    let flat = title.split_whitespace().collect::<Vec<_>>().join(" ");
    if flat.chars().count() <= TITLE_WIDTH {
        return flat;
    }
    let cut: String = flat.chars().take(TITLE_WIDTH - 1).collect();
    format!("{cut}…")
}

/// First 8 chars of a hex id — display form only; ack runs on the full id.
fn short_hex(hex: &str) -> String {
    hex.chars().take(8).collect()
}

/// Relative age in the house recall format: now / Nm ago / Nh ago / Nd ago.
fn rel_ts(ts: chrono::DateTime<chrono::Utc>) -> String {
    let secs = (chrono::Utc::now() - ts).num_seconds().max(0) as u64;
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

#[derive(Debug, Clone, PartialEq, Eq)]
enum MatchError {
    NotFound(String),
    Ambiguous(String),
}

impl std::fmt::Display for MatchError {
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

/// Resolve a user-supplied id (full 16-hex or unique prefix) to exactly one
/// entry. An exact full-id hit wins even if it is also another's prefix.
fn resolve<'a>(entries: &'a [Entry], given: &str) -> Result<&'a Entry, MatchError> {
    if let Some(e) = entries.iter().find(|e| e.id == given) {
        return Ok(e);
    }
    let hits: Vec<&Entry> = entries.iter().filter(|e| e.id.starts_with(given)).collect();
    match hits.as_slice() {
        [one] => Ok(one),
        [] => Err(MatchError::NotFound(given.to_string())),
        _ => Err(MatchError::Ambiguous(given.to_string())),
    }
}

fn load_entry(home: &Path, given: &str) -> Result<Entry, MatchError> {
    let ledgered = ledger(home);
    resolve(&ledgered, given).cloned()
}

/// Ack by prefix against the full ledger (acked entries stay resolvable).
fn ack_prefix(home: &Path, given: &str) -> Result<bool, String> {
    let id = load_entry(home, given).map_err(|e| e.to_string())?.id;
    inbox::ack(home, &id).map_err(|e| e.to_string())
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
    use super::*;
    use chrono::Utc;

    fn entry(id: &str, kind: Kind, title: &str) -> Entry {
        Entry {
            id: id.to_string(),
            ts: Utc::now(),
            kind,
            title: title.to_string(),
            body: String::new(),
            session_id: None,
            task_id: None,
            result_id: None,
            dedupe_key: None,
        }
    }

    #[test]
    fn prefix_resolution_exact_unique_ambiguous_notfound() {
        let entries = vec![
            entry("aaaa111111111111", Kind::Digest, "a"),
            entry("aaaa222222222222", Kind::Digest, "b"),
            entry("bbbb333333333333", Kind::Digest, "c"),
        ];
        assert_eq!(
            resolve(&entries, "aaaa111111111111").map(|e| e.id.as_str()),
            Ok("aaaa111111111111")
        );
        // Unique longer prefix disambiguates the aaaa pair.
        assert_eq!(
            resolve(&entries, "aaaa2").map(|e| e.id.as_str()),
            Ok("aaaa222222222222")
        );
        // Ambiguous prefixes are refused, never silently resolved.
        assert_eq!(
            resolve(&entries, "aaaa"),
            Err(MatchError::Ambiguous("aaaa".to_string()))
        );
        // Empty prefix matches everything, so it is ambiguous too.
        assert_eq!(
            resolve(&entries, ""),
            Err(MatchError::Ambiguous(String::new()))
        );
        assert_eq!(
            resolve(&entries, "ffff"),
            Err(MatchError::NotFound("ffff".to_string()))
        );
    }

    #[test]
    fn kind_chips_cover_every_kind_within_column_width() {
        let chips = [
            (Kind::TaskSummary, "task"),
            (Kind::ApprovalPending, "approval"),
            (Kind::ApprovalDenied, "denied"),
            (Kind::BudgetAlert, "budget"),
            (Kind::Digest, "digest"),
            (Kind::Heartbeat, "beat"),
            (Kind::ProposalOpened, "proposal"),
        ];
        for (kind, chip) in chips {
            assert_eq!(kind_chip(kind), chip);
            assert!(chip.chars().count() <= 8, "{chip} breaks the column");
        }
    }

    #[test]
    fn rows_truncate_multiline_titles_and_show_id8() {
        let long = entry(
            "0123456789abcdef",
            Kind::BudgetAlert,
            &format!("{} tail", "x".repeat(60)),
        );
        let row = format_row_rel(&long, "5m ago");
        let cells: Vec<&str> = row.split("  ").filter(|c| !c.is_empty()).collect();
        assert_eq!(cells[0], "5m ago");
        assert_eq!(cells[1], "budget");
        // Collapsed to one line, clipped under the column width, ellipsized.
        assert!(cells[2].ends_with('…'));
        assert!(cells[2].chars().count() <= TITLE_WIDTH);
        assert!(!cells[2].contains("tail"));
        assert_eq!(cells[3], "01234567");

        let short = entry("fedcba9876543210", Kind::TaskSummary, "tiny\ntwo lines");
        let row = format_row_rel(&short, "now");
        assert!(row.contains("tiny two lines"), "{row}");
        assert!(!row.contains('…'), "{row}");
        assert!(row.ends_with("fedcba98"));
    }

    #[test]
    fn unread_all_footer_and_count_math_on_seeded_store() {
        let dir = tempfile::tempdir().unwrap();
        let home = dir.path();
        let a = inbox::record(home, Kind::Digest, "alpha one", "b", None, None).unwrap();
        let b = inbox::record(home, Kind::BudgetAlert, "beta two", "b", None, None).unwrap();
        let _c = inbox::record(home, Kind::Heartbeat, "gamma three", "b", None, None).unwrap();

        assert_eq!(count_line(home), "3 unread");
        assert_eq!(footer(home), "3 unread of 3 total");

        assert!(inbox::ack(home, &b.id).unwrap());
        let unread = unread_rows(home, 50);
        assert_eq!(unread.len(), 2, "acked entry leaves the unread view");
        assert!(all_rows(home, 50).len() == 3, "--all keeps acked rows");
        assert_eq!(footer(home), "2 unread of 3 total");
        assert_eq!(count_line(home), "2 unread");

        // --limit truncates rows but never the totals.
        assert_eq!(unread_rows(home, 1).len(), 1);

        // b was already acked above; idempotent acks report false.
        assert!(inbox::ack(home, &a.id).unwrap());
        assert!(!inbox::ack(home, &b.id).unwrap());
        assert!(inbox::ack(home, &_c.id).unwrap());
        assert_eq!(footer(home), "0 unread of 3 total");
        assert_eq!(count_line(home), "0 unread");
    }

    #[test]
    fn ack_by_prefix_idempotent_and_unknown_is_typed_error() {
        let dir = tempfile::tempdir().unwrap();
        let home = dir.path();
        let e = inbox::record(
            home,
            Kind::ApprovalPending,
            "gate",
            "body",
            Some("s1"),
            Some("t1"),
        )
        .unwrap();

        let prefix = short_hex(&e.id);
        assert_eq!(ack_prefix(home, &prefix), Ok(true));
        assert_eq!(ack_prefix(home, &prefix), Ok(false));

        let err = ack_prefix(home, "ffffffff").unwrap_err();
        assert!(err.contains("no inbox entry matches 'ffffffff'"), "{err}");

        // show resolves the same prefix even after the entry was acked.
        let shown = load_entry(home, &prefix).unwrap();
        assert_eq!(shown.id, e.id);
        assert_eq!(shown.session_id.as_deref(), Some("s1"));
        assert_eq!(shown.task_id.as_deref(), Some("t1"));
    }

    #[test]
    fn empty_stores_render_clear_messages() {
        let dir = tempfile::tempdir().unwrap();
        let home = dir.path();
        assert_eq!(count_line(home), "0 unread");
        assert!(unread_rows(home, DEFAULT_LIMIT).is_empty());
        assert!(all_rows(home, DEFAULT_LIMIT).is_empty());
        assert!(load_entry(home, "deadbeef").is_err());
    }
}
