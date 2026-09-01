use std::path::{Path, PathBuf};

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

fn security_events_path(home: &Path) -> PathBuf {
    home.join("security-events.jsonl")
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct SecurityEvent {
    pub ts: DateTime<Utc>,
    pub kind: EventKind,
    /// Short human-readable label (e.g. "rate_limit", "auth_failure").
    pub label: String,
    /// Structured detail — freeform JSON string.
    pub detail: String,
    /// Source IP when available.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ip: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum EventKind {
    AuthFailure,
    RateLimit,
    ChatAllowlist,
    PermissionDenial,
    ConfigChange,
    ProviderKeyChange,
    FullAccessGrant,
    FullAccessRevoke,
    /// docs/design/34-channel-onboarding.md: an unknown inbound chat key
    /// was newly recorded as pending operator review.
    ChatPending,
    /// An operator approved a pending (or unknown) allowlist entry.
    ChatApproved,
    /// An operator denied a pending (or unknown) allowlist entry.
    ChatDenied,
    /// An operator revoked a previously allowed entry.
    ChatRevoked,
    /// docs/design/22-gateway.md: an inbound turn that was admitted but failed
    /// to execute (e.g. broker protocol failure, provider unavailable, lost
    /// session lock). Recorded so a silently-failed unattended turn is
    /// auditable instead of indistinguishable from a successful completion.
    ExecutionError,
    /// docs/design/34-channel-onboarding.md: a per-channel permission-mode
    /// override asked for more than the target workspace's own configured
    /// mode allows, and was capped down to that workspace's mode. A
    /// distinct kind rather than a `ConfigChange` so a silently-reduced
    /// grant is greppable in the audit log — it means an operator believes
    /// a channel has access it does not actually have.
    PermissionCapped,
}

/// Append a security event to `<home>/security-events.jsonl`.
/// Best-effort: never panics, returns the event on success.
pub fn record(
    home: &Path,
    kind: EventKind,
    label: &str,
    detail: &str,
    ip: Option<&str>,
) -> SecurityEvent {
    let event = SecurityEvent {
        ts: Utc::now(),
        kind,
        label: label.to_string(),
        detail: detail.to_string(),
        ip: ip.map(str::to_string),
    };
    let _ = append_event(home, &event);
    event
}

fn append_event(home: &Path, event: &SecurityEvent) -> Result<(), std::io::Error> {
    let path = security_events_path(home);
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let mut line = serde_json::to_vec(event).map_err(std::io::Error::other)?;
    line.push(b'\n');
    std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)
        .and_then(|mut f| std::io::Write::write_all(&mut f, &line))
}

/// Read all security events (newest first), capped at `limit`.
pub fn list(home: &Path, limit: usize) -> Vec<SecurityEvent> {
    let path = security_events_path(home);
    let Ok(raw) = std::fs::read_to_string(&path) else {
        return Vec::new();
    };
    let mut events: Vec<SecurityEvent> = raw
        .lines()
        .filter(|l| !l.trim().is_empty())
        .filter_map(|l| serde_json::from_str(l).ok())
        .collect();
    events.reverse();
    events.truncate(limit);
    events
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;

    #[test]
    fn record_and_list_roundtrip() {
        let dir = tempfile::tempdir().unwrap();
        let home = dir.path();
        let ev = record(
            home,
            EventKind::AuthFailure,
            "bad_token",
            "wrong bearer supplied",
            Some("127.0.0.1"),
        );
        assert_eq!(ev.kind, EventKind::AuthFailure);
        let events = list(home, 10);
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].label, "bad_token");
        assert_eq!(events[0].ip.as_deref(), Some("127.0.0.1"));
    }

    #[test]
    fn list_returns_newest_first() {
        let dir = tempfile::tempdir().unwrap();
        let home = dir.path();
        record(home, EventKind::RateLimit, "first", "", None);
        std::thread::sleep(std::time::Duration::from_millis(10));
        record(home, EventKind::RateLimit, "second", "", None);
        let events = list(home, 10);
        assert_eq!(events.len(), 2);
        assert_eq!(events[0].label, "second");
        assert_eq!(events[1].label, "first");
    }

    #[test]
    fn list_respects_limit() {
        let dir = tempfile::tempdir().unwrap();
        let home = dir.path();
        for i in 0..5 {
            record(
                home,
                EventKind::ConfigChange,
                &format!("change_{i}"),
                "",
                None,
            );
        }
        assert_eq!(list(home, 3).len(), 3);
    }

    #[test]
    fn missing_file_returns_empty() {
        let dir = tempfile::tempdir().unwrap();
        let events = list(dir.path(), 10);
        assert!(events.is_empty());
    }
}
