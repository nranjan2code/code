//! Session-related commands: sessions, resume, rewind, transcript, clear.

use std::time::SystemTime;

use chrono::{DateTime, Utc};
use vak_client::types::{Checkpoint, SessionSummary};

use crate::data::ClientData;
use crate::events::trunc_one;
use crate::render::Screen;
use crate::state::ModalView;
use crate::status;
use crate::transcript::build_transcript_rows;

const RECENT_LIMIT: usize = 15;
const CHECKPOINT_LIMIT: usize = 10;

/// Title for listings: the session's server-recorded title (first user
/// prompt) when present, else blank — callers fall back to the id prefix.
pub fn first_user_prompt(summary: &SessionSummary) -> Option<String> {
    summary
        .title
        .as_deref()
        .map(str::trim)
        .filter(|t| !t.is_empty())
        .map(str::to_string)
}

/// Relative timestamp for recall surfaces: now / 5m / 3h / 2d style labels.
pub fn age_of(mtime: SystemTime) -> String {
    let secs = mtime.elapsed().map(|d| d.as_secs()).unwrap_or(0);
    if secs < 60 {
        "now".to_string()
    } else {
        status::fmt_elapsed(secs)
    }
}

fn age_since(ts: DateTime<Utc>) -> String {
    age_of(SystemTime::from(ts))
}

fn display_title(s: &SessionSummary) -> String {
    first_user_prompt(s).unwrap_or_else(|| s.session_id.chars().take(8).collect())
}

fn summary_row(s: &SessionSummary) -> String {
    let mut row = format!(
        "{} · {} entries · {}",
        trunc_one(&display_title(s), 52),
        s.entries,
        s.updated_at.map(age_since).unwrap_or_else(|| "?".into())
    );
    if s.running {
        row.push_str(" · running");
    }
    if s.archived {
        row.push_str(" · archived");
    }
    row
}

/// Session ids ordered most-recent-first (server sort by updated_at).
/// The name is historical; recency now comes from the server, not mtimes.
pub async fn sessions_by_mtime(data: &ClientData) -> Result<Vec<String>, String> {
    Ok(data
        .list_sessions()
        .await?
        .sessions
        .into_iter()
        .map(|s| s.session_id)
        .collect())
}

/// Display rows for the caller's ChoicePicker. Row order matches
/// [`sessions_by_mtime`], so `/resume <picker index + 1>` resolves to the
/// highlighted session.
pub async fn session_choices(data: &ClientData) -> Result<Vec<String>, String> {
    let resp = data.list_sessions().await?;
    Ok(resp.sessions.iter().map(summary_row).collect())
}

pub async fn list_sessions(data: &ClientData, screen: &mut Screen) {
    match data.list_sessions().await {
        Ok(resp) if resp.sessions.is_empty() => screen.dim("no sessions yet"),
        Ok(resp) => {
            screen.dim("sessions:");
            let recent: Vec<&SessionSummary> = resp.sessions.iter().take(RECENT_LIMIT).collect();
            for s in recent.into_iter().rev() {
                screen.dim(&format!("  {}", summary_row(s)));
            }
            screen.dim("use /resume <n> or <id-prefix>");
        }
        Err(e) => screen.dim(&format!("error: {e}")),
    }
}

/// Resolve `/resume [n|id-prefix]` and attach on the server.
///
/// - `Ok(Some(canonical_id))` — attached; caller swaps its session slot.
/// - `Ok(None)` — no argument given and sessions exist; caller opens a
///   picker over [`session_choices`] rows.
/// - `Err(msg)` — nothing to show or the selection matched nothing.
pub async fn cmd_resume(data: &ClientData, arg: Option<&str>) -> Result<Option<String>, String> {
    let resp = data.list_sessions().await?;
    match arg.map(str::trim).filter(|a| !a.is_empty()) {
        None => {
            if resp.sessions.is_empty() {
                Err("no sessions yet".to_string())
            } else {
                Ok(None)
            }
        }
        Some(sel) => {
            let chosen = sel
                .parse::<usize>()
                .ok()
                .and_then(|n| n.checked_sub(1))
                .and_then(|i| resp.sessions.get(i).map(|s| s.session_id.clone()))
                .or_else(|| {
                    resp.sessions
                        .iter()
                        .find(|s| s.session_id.starts_with(sel))
                        .map(|s| s.session_id.clone())
                });
            match chosen {
                Some(id) => Ok(Some(data.attach_session(&id).await?)),
                None => Err("no such session".to_string()),
            }
        }
    }
}

/// Table rows for up to the 10 newest checkpoints (newest first).
pub fn checkpoint_rows(cps: &[Checkpoint]) -> Vec<String> {
    cps.iter()
        .rev()
        .take(CHECKPOINT_LIMIT)
        .map(|cp| {
            let when = DateTime::parse_from_rfc3339(&cp.created_at)
                .map(|t| t.with_timezone(&Utc).format("%m-%d %H:%M").to_string())
                .unwrap_or_else(|_| cp.created_at.clone());
            format!(
                "  {:>3}. {} · {}",
                cp.sequence,
                when,
                trunc_one(cp.description.as_deref().unwrap_or(""), 44)
            )
        })
        .collect()
}

pub async fn cmd_rewind(
    data: &ClientData,
    arg: Option<&str>,
    session_id: &str,
    screen: &mut Screen,
) {
    let cps = match data.checkpoints(session_id).await {
        Ok(cps) => cps,
        Err(e) => {
            screen.dim(&format!("error: {e}"));
            return;
        }
    };
    match arg.map(str::trim).filter(|a| !a.is_empty()) {
        None => {
            if cps.is_empty() {
                screen.dim("no checkpoints yet");
                return;
            }
            screen.dim("checkpoints (newest first):");
            for row in checkpoint_rows(&cps) {
                screen.dim(&row);
            }
            screen.dim("use /rewind <seq> to restore that workspace snapshot");
        }
        Some(sel) => match sel.parse::<u32>() {
            Ok(seq) => match data.restore_checkpoint(session_id, seq).await {
                Ok(()) => screen.success(&format!("rewound to {seq}")),
                Err(e) => screen.dim(&format!("error: restore failed: {e}")),
            },
            Err(_) => screen.dim("usage: /rewind <seq>"),
        },
    }
}

/// Compact read-only viewer for the active session's message tree.
pub async fn transcript_modal(
    data: &ClientData,
    session_id: &str,
    arg: Option<&str>,
) -> Result<ModalView, String> {
    let limit = arg
        .and_then(|a| a.trim().parse::<usize>().ok())
        .filter(|l| *l > 0)
        .unwrap_or(40);
    let t = data.transcript(session_id).await?;
    let start = t.messages.len().saturating_sub(limit);
    let shown = t.messages.len() - start;
    let (mut rows, anchors) = build_transcript_rows(&t.messages, start);
    let scroll = rows.len().saturating_sub(1);
    rows.insert(
        0,
        format!(
            "transcript · showing {shown} of {} messages · n/p prompt jumps · e export",
            t.messages.len()
        ),
    );
    Ok(ModalView {
        title: "transcript".to_string(),
        rows,
        scroll,
        footer: "n/p prompt jumps · / search · e export · End latest · Esc close".to_string(),
        anchors: anchors.into_iter().map(|a| a + 1).collect(),
        ..Default::default()
    })
}

#[cfg(test)]
mod session_command_tests {
    #![allow(clippy::unwrap_used)]
    use super::*;
    use chrono::TimeZone;

    fn summary(title: Option<&str>) -> SessionSummary {
        SessionSummary {
            session_id: "abcdef1234567890".to_string(),
            cwd: ".".to_string(),
            created_at: None,
            updated_at: Some(Utc::now()),
            entries: 7,
            title: title.map(String::from),
            running: true,
            archived: false,
        }
    }

    #[test]
    fn first_user_prompt_prefers_nonblank_title() {
        assert_eq!(
            first_user_prompt(&summary(Some("  fix parser "))),
            Some("fix parser".into())
        );
        assert_eq!(first_user_prompt(&summary(Some("   "))), None);
        assert_eq!(first_user_prompt(&summary(None)), None);
    }

    #[test]
    fn summary_row_carries_title_entries_age_and_running_mark() {
        let row = summary_row(&summary(Some("fix parser")));
        assert!(row.contains("fix parser"), "{row}");
        assert!(row.contains("7 entries"), "{row}");
        assert!(row.contains("now"), "{row}");
        assert!(row.contains("· running"), "{row}");
        assert!(!row.contains("archived"), "{row}");
    }

    #[test]
    fn titleless_summaries_fall_back_to_short_id() {
        assert_eq!(display_title(&summary(None)), "abcdef12");
    }

    #[test]
    fn age_of_recent_is_now_and_old_is_elapsed_label() {
        assert_eq!(age_of(SystemTime::now()), "now");
        assert_ne!(age_of(SystemTime::UNIX_EPOCH), "now");
    }

    #[test]
    fn checkpoint_rows_are_newest_first_and_truncate_descriptions() {
        let cps: Vec<Checkpoint> = (0..3)
            .map(|i| Checkpoint {
                sequence: i,
                created_at: Utc
                    .timestamp_opt(1_700_000_000 + i as i64 * 60, 0)
                    .single()
                    .unwrap()
                    .to_rfc3339(),
                description: Some(format!("step {i} {}", "x".repeat(80))),
            })
            .collect();
        let rows = checkpoint_rows(&cps);
        assert_eq!(rows.len(), 3);
        assert!(rows[0].trim_start().starts_with("2."), "{rows:?}");
        assert!(rows[0].contains('…'), "long descriptions truncate");
        assert!(checkpoint_rows(&[]).is_empty());
    }
}
