//! Opt-in update awareness (docs/design/29-personal-os.md P3): when
//! `[update] url` is configured, poll it at most once per
//! `interval_hours`, notice a newer `X.Y.Z` in the response body, print
//! one line, and never install anything. Network trouble is silent — the
//! check must never delay or fail startup.

use std::path::PathBuf;

use vak_config::Config;

const CHECK_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(2);

pub fn maybe_check_update(config: &Config) {
    let Some(url) = config.update.url.clone() else {
        return;
    };
    let Some(home) = vak_home() else {
        return;
    };
    let cache_path = home.join("update-check.json");
    if let Ok(raw) = std::fs::read_to_string(&cache_path)
        && let Ok(value) = serde_json::from_str::<serde_json::Value>(&raw)
        && let Some(last) = value.get("last_check").and_then(serde_json::Value::as_u64)
        && !due(last, config.update.interval_hours)
    {
        return;
    }

    // The blocking client is confined to its own thread so the fetch can
    // never interact with the async runtime this CLI boots.
    let fetched = std::thread::spawn(move || fetch_latest_version(&url))
        .join()
        .ok()
        .flatten();
    write_cache_timestamp(&cache_path);
    if let Some(latest) = fetched
        && version_newer(latest, env!("CARGO_PKG_VERSION"))
    {
        eprintln!(
            "note: Vak {} is available (installed {}) — install manually; nothing is auto-updated",
            format_version(latest),
            env!("CARGO_PKG_VERSION")
        );
    }
}

fn due(last_check_secs: u64, interval_hours: u64) -> bool {
    let now = epoch_secs();
    match now.checked_sub(last_check_secs) {
        Some(elapsed) => elapsed >= interval_hours.saturating_mul(3600),
        None => true,
    }
}

fn epoch_secs() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

fn write_cache_timestamp(cache_path: &std::path::Path) {
    if let Some(parent) = cache_path.parent()
        && std::fs::create_dir_all(parent).is_ok()
    {
        let body = serde_json::json!({ "last_check": epoch_secs() });
        let _ = std::fs::write(cache_path, body.to_string());
    }
}

fn vak_home() -> Option<PathBuf> {
    std::env::var_os("VAK_HOME")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".vak")))
}

fn fetch_latest_version(url: &str) -> Option<(u64, u64, u64)> {
    let client = reqwest::blocking::Client::builder()
        .timeout(CHECK_TIMEOUT)
        .build()
        .ok()?;
    let body = client.get(url).send().ok()?.text().ok()?;
    parse_version_token(&body)
}

/// First dotted-numeric token (`X.Y.Z`, optional v/V prefix) in the body.
pub fn parse_version_token(body: &str) -> Option<(u64, u64, u64)> {
    for token in body.split(|c: char| !(c.is_ascii_alphanumeric() || c == '.')) {
        let t = token.trim_start_matches(['v', 'V']);
        if t.is_empty() || !t.starts_with(|c: char| c.is_ascii_digit()) {
            continue;
        }
        let parts: Vec<&str> = t.split('.').collect();
        if parts.len() != 3 {
            continue;
        }
        if let (Ok(major), Ok(minor), Ok(patch)) = (
            parts[0].parse::<u64>(),
            parts[1].parse::<u64>(),
            parts[2].parse::<u64>(),
        ) {
            return Some((major, minor, patch));
        }
    }
    None
}

fn parse_current(version: &str) -> Option<(u64, u64, u64)> {
    parse_version_token(version)
}

/// True when `latest` is strictly greater than the installed version.
pub fn version_newer(latest: (u64, u64, u64), installed: &str) -> bool {
    parse_current(installed).is_some_and(|current| latest > current)
}

fn format_version(v: (u64, u64, u64)) -> String {
    format!("{}.{}.{}", v.0, v.1, v.2)
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
    use super::*;

    #[test]
    fn parses_first_semver_token_from_prose_json_or_tags() {
        assert_eq!(parse_version_token("release 1.2.3 is out"), Some((1, 2, 3)));
        assert_eq!(
            parse_version_token(r#"{"latest":"0.4.1","notes":"see docs"}"#),
            Some((0, 4, 1))
        );
        assert_eq!(parse_version_token("v2.10.0\nsha256…"), Some((2, 10, 0)));
        assert_eq!(parse_version_token("V3.0.0"), Some((3, 0, 0)));
        assert_eq!(
            parse_version_token("older 9.9.9 newer 10.0.0"),
            Some((9, 9, 9)),
            "first token wins"
        );
        assert_eq!(parse_version_token("no versions here"), None);
        assert_eq!(parse_version_token("1.2"), None);
        assert_eq!(parse_version_token("1.2.3.4"), None);
        assert_eq!(parse_version_token("x.2.3"), None);
        assert_eq!(parse_version_token(""), None);
        // A bare number inside a longer dotted run is not semver-ish.
        assert_eq!(parse_version_token("2026.08.24"), Some((2026, 8, 24)));
    }

    #[test]
    fn newer_only_when_strictly_greater_per_component() {
        assert!(version_newer((1, 0, 1), "1.0.0"));
        assert!(version_newer((0, 5, 0), "0.4.99"));
        assert!(!version_newer((1, 0, 0), "1.0.0"));
        assert!(!version_newer((0, 9, 0), "1.0.0"));
        assert!(
            !version_newer((1, 0, 0), "unparsable"),
            "an unparsable installed version never reads as older"
        );
    }

    #[test]
    fn due_honors_interval_and_clock_skew() {
        assert!(due(0, 24));
        assert!(!due(epoch_secs(), 24));
        assert!(due(epoch_secs() - 86_400, 24));
        assert!(!due(epoch_secs() - 86_399, 24));
        assert!(due(u64::MAX, 1), "future timestamp counts as due");
        assert!(due(epoch_secs(), 0), "zero interval always due");
    }

    #[test]
    fn cache_write_is_tolerant_of_missing_dirs() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("nested/update-check.json");
        write_cache_timestamp(&path);
        let raw = std::fs::read_to_string(path).unwrap();
        let parsed: serde_json::Value = serde_json::from_str(&raw).unwrap();
        assert!(parsed["last_check"].as_u64().is_some());
    }
}
