//! Headless-browser DOM render tool: drives a locally installed
//! Chromium-family browser as a child process and returns the JavaScript-
//! rendered DOM (docs/design/29-personal-os.md, sibling of webfetch P4).
//!
//! Permission posture matches webfetch exactly (Ask in restricted modes,
//! Allow under FullAccess or an explicit rule) and is applied downstream by
//! the registry/permission engine; this tool performs no permission checks
//! itself. Credentials are never sent: fresh throwaway profile, no cookie
//! reuse, no auth material.
//!
//! Sandbox posture: unlike Bash, no Seatbelt/Landlock command wrapper is
//! applied — containment comes from spawning a fixed, discovered browser
//! binary with fixed flags, a scrubbed allowlist environment, a fresh
//! profile directory that is deleted afterwards, and its own process group
//! killed on timeout/cancel.
//!
//! v1 SSRF honesty note: the guard below screens only the REQUESTED host.
//! Once navigation is handed to Chromium, redirects and subresource fetches
//! inside the browser resolve and connect on their own and are not
//! re-screenable here.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant};

use async_trait::async_trait;
use serde_json::Value;

use crate::webfetch::ssrf_guard;
use crate::{ResourceClaims, Tool, ToolContext, ToolOutput};

const DEFAULT_WAIT_MS: u64 = 4000;
const MAX_WAIT_MS: u64 = 10_000;
const TOTAL_TIMEOUT_SECS: u64 = 20;
const TOTAL_TIMEOUT: Duration = Duration::from_secs(TOTAL_TIMEOUT_SECS);
const BROWSER_ENV: &str = "VAK_BROWSER";

pub struct WebBrowseTool;

#[cfg(target_os = "macos")]
const MACOS_BUNDLES: &[&str] = &[
    "/Applications/Google Chrome.app/Contents/MacOS/Google Chrome",
    "/Applications/Chromium.app/Contents/MacOS/Chromium",
    "/Applications/Microsoft Edge.app/Contents/MacOS/Microsoft Edge",
    "/Applications/Brave Browser.app/Contents/MacOS/Brave Browser",
];

#[cfg(target_os = "linux")]
const LINUX_NAMES: &[&str] = &[
    "google-chrome",
    "chromium",
    "chromium-browser",
    "msedge",
    "brave-browser",
];

/// Ordered candidate list: an explicit `VAK_BROWSER` override walks
/// first; platform-native locations follow.
fn default_candidates() -> Vec<PathBuf> {
    let mut out: Vec<PathBuf> = std::env::var_os(BROWSER_ENV)
        .filter(|v| !v.is_empty())
        .map(PathBuf::from)
        .into_iter()
        .collect();
    #[cfg(target_os = "macos")]
    out.extend(MACOS_BUNDLES.iter().map(PathBuf::from));
    #[cfg(target_os = "linux")]
    if let Some(path_var) = std::env::var_os("PATH") {
        for dir in std::env::split_paths(&path_var) {
            for name in LINUX_NAMES {
                out.push(dir.join(name));
            }
        }
    }
    out
}

/// First usable candidate wins; exhaustion fails closed naming every path
/// tried so operators can see exactly what was searched.
fn select_browser(candidates: &[PathBuf]) -> Result<PathBuf, String> {
    if candidates.is_empty() {
        return Err(format!(
            "no {BROWSER_ENV} override set and no browser locations known for this platform"
        ));
    }
    let tried: Vec<String> = candidates.iter().map(|p| p.display().to_string()).collect();
    for path in candidates {
        if path.is_file() {
            return Ok(path.clone());
        }
    }
    Err(format!(
        "no Chromium-family browser found; tried:\n{}",
        tried
            .iter()
            .map(|p| format!("  - {p}"))
            .collect::<Vec<_>>()
            .join("\n")
    ))
}

fn discover_browser() -> Result<PathBuf, String> {
    select_browser(&default_candidates())
}

fn clamp_wait_ms(raw: Option<i64>) -> u64 {
    match raw {
        Some(n) if n > 0 => u64::try_from(n).unwrap_or(MAX_WAIT_MS).min(MAX_WAIT_MS),
        _ => DEFAULT_WAIT_MS,
    }
}

fn assemble_args(wait_ms: u64, url: &str, profile_dir: &Path) -> Vec<String> {
    vec![
        "--headless=new".to_string(),
        "--disable-gpu".to_string(),
        "--no-first-run".to_string(),
        "--no-default-browser-check".to_string(),
        format!("--user-data-dir={}", profile_dir.display()),
        format!("--virtual-time-budget={wait_ms}"),
        "--dump-dom".to_string(),
        url.to_string(),
    ]
}

static PROFILE_SEQ: AtomicU64 = AtomicU64::new(0);

fn fresh_profile_dir() -> std::io::Result<PathBuf> {
    let n = PROFILE_SEQ.fetch_add(1, Ordering::Relaxed);
    let dir = std::env::temp_dir().join(format!(
        "vak-browse-{}-{}-{n}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or_default()
    ));
    std::fs::create_dir_all(&dir)?;
    Ok(dir)
}

struct ProfileDir(PathBuf);

impl Drop for ProfileDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn parse_target(raw: &str) -> Result<reqwest::Url, String> {
    let url = reqwest::Url::parse(raw).map_err(|e| format!("invalid url: {e}"))?;
    match url.scheme() {
        "http" | "https" => {}
        other => return Err(format!("unsupported scheme \"{other}\" (only http/https)")),
    }
    if url.host_str().is_none_or(str::is_empty) {
        return Err("invalid url: missing host".to_string());
    }
    Ok(url)
}

fn stderr_tail(err: &str) -> String {
    let trimmed = err.trim();
    if trimmed.chars().count() <= 2000 {
        return trimmed.to_string();
    }
    let skipped = trimmed.chars().count() - 2000;
    let tail: String = trimmed.chars().skip(skipped).collect();
    format!("…{tail}")
}

fn basename(path: &Path) -> String {
    path.file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| path.display().to_string())
}

#[async_trait]
impl Tool for WebBrowseTool {
    fn name(&self) -> &str {
        "browse"
    }

    fn serves(&self) -> &'static [&'static str] {
        &["web", "live-data"]
    }

    fn description(&self) -> &str {
        "Render a URL in a locally installed headless Chromium-family browser and return the JavaScript-rendered DOM. Applies the same private-range blocklist as webfetch to the requested host, runs with a fresh throwaway profile (never sends credentials), caps runtime at 20s, and returns a header line followed by the DOM. In-browser redirects are not re-screened in v1."
    }

    fn schema(&self) -> Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "url": {"type": "string", "description": "Absolute http(s) URL to render"},
                "wait_ms": {"type": "integer", "description": "Virtual-time budget in milliseconds (default 4000, capped at 10000; values <= 0 fall back to the default)"}
            },
            "required": ["url"]
        })
    }

    fn claims(&self, _args: &Value) -> ResourceClaims {
        ResourceClaims {
            exclusive: false,
            read_only: true,
            paths: Vec::new(),
        }
    }

    async fn execute(&self, args: &Value, ctx: &ToolContext) -> ToolOutput {
        if args.get("url").and_then(Value::as_str).is_none() {
            return ToolOutput::error("missing required parameter: url");
        }
        tokio::select! {
            _ = ctx.cancel.cancelled() => ToolOutput::error("browse cancelled"),
            out = self.run(args, ctx) => out,
        }
    }
}

impl WebBrowseTool {
    async fn run(&self, args: &Value, ctx: &ToolContext) -> ToolOutput {
        // Unwrap-free re-extraction after the execute() pre-check keeps this
        // method total even when called directly in tests.
        let raw = match args.get("url").and_then(Value::as_str) {
            Some(r) => r,
            None => return ToolOutput::error("missing required parameter: url"),
        };
        let wait_ms = clamp_wait_ms(args.get("wait_ms").and_then(Value::as_i64));

        let url = match parse_target(raw) {
            Ok(u) => u,
            Err(e) => return ToolOutput::error(e),
        };
        let Some(host) = url.host_str().map(str::to_string) else {
            return ToolOutput::error("invalid url: missing host");
        };
        let screened = match tokio::task::spawn_blocking(move || ssrf_guard(&host)).await {
            Ok(r) => r,
            Err(e) => return ToolOutput::error(format!("address screening failed: {e}")),
        };
        if let Err(rejection) = screened {
            return ToolOutput::error(rejection.to_string());
        }

        let browser = match discover_browser() {
            Ok(b) => b,
            Err(e) => return ToolOutput::error(e),
        };
        let profile = match fresh_profile_dir() {
            Ok(p) => ProfileDir(p),
            Err(e) => return ToolOutput::error(format!("profile dir creation failed: {e}")),
        };

        // Capture DOM/stderr via FILES, not pipes: Chromium helper
        // processes inherit pipe fds and never let EOF fire, while file
        // redirection makes output inspectable while Chrome still runs.
        let out_path = profile.0.join("dom.html");
        let err_path = profile.0.join("stderr.txt");
        let out_file = match std::fs::File::create(&out_path) {
            Ok(f) => f,
            Err(e) => return ToolOutput::error(format!("dom capture create failed: {e}")),
        };
        let err_file = match std::fs::File::create(&err_path) {
            Ok(f) => f,
            Err(e) => return ToolOutput::error(format!("stderr capture create failed: {e}")),
        };

        let mut cmd = tokio::process::Command::new(&browser);
        cmd.args(assemble_args(wait_ms, url.as_str(), &profile.0))
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::from(out_file))
            .stderr(std::process::Stdio::from(err_file))
            .kill_on_drop(true);
        crate::bash::scrub_environment(&mut cmd);
        if std::env::var_os(crate::broker::WORKER_ENV).is_none() {
            crate::bash::isolate_process_group(&mut cmd);
        }

        let started = Instant::now();
        let mut child = match cmd.spawn() {
            Ok(c) => c,
            Err(e) => {
                return ToolOutput::error(format!("failed to launch {}: {e}", basename(&browser)));
            }
        };

        // Completion is content-based: --dump-dom writes the document then
        // frequently HANGS (headless=new quirk), so poll for a closed-html
        // sentinel (or natural exit) and kill the group either way. The
        // deadline is checked against elapsed time each cycle — a fresh
        // per-iteration timer would always lose to the 250 ms tick.
        let header_base = format!("[browse] GET {url} via {}", basename(&browser));
        let started_at = Instant::now();
        loop {
            if started_at.elapsed() >= TOTAL_TIMEOUT {
                crate::bash::kill_process_group(&child.id());
                // macOS Chrome re-execs into a fresh process group, so the
                // group signal can miss the main binary entirely; a
                // direct-pid SIGKILL cannot.
                let _ = child.start_kill();
                let _ = child.wait().await;
                let elapsed_ms = started_at.elapsed().as_millis();
                return ToolOutput::error(format!(
                    "{header_base} ({elapsed_ms} ms): exceeded its {TOTAL_TIMEOUT_SECS}s total timeout"
                ));
            }
            let wait = std::cmp::min(
                std::time::Duration::from_millis(250),
                TOTAL_TIMEOUT - started_at.elapsed(),
            );
            let mut done = false;
            tokio::select! {
                _ = ctx.cancel.cancelled() => {
                    crate::bash::kill_process_group(&child.id());
                    // Direct-pid SIGKILL: see re-exec note above.
                    let _ = child.start_kill();
                    let _ = child.wait().await;
                    return ToolOutput::error(format!("{header_base}: browse cancelled"));
                }
                _ = tokio::time::sleep(wait) => {
                    if matches!(child.try_wait(), Ok(Some(_))) {
                        done = true;
                    } else if let Ok(s) = std::fs::read_to_string(&out_path)
                        && s.to_ascii_lowercase().contains("</html>")
                    {
                        done = true;
                    }
                }
            }
            if done {
                break;
            }
        }
        crate::bash::kill_process_group(&child.id());
        // Direct-pid SIGKILL: see re-exec note above.
        let _ = child.start_kill();
        let _ = child.wait().await;

        let dom = std::fs::read_to_string(&out_path).unwrap_or_default();
        let err = std::fs::read_to_string(&err_path).unwrap_or_default();
        let elapsed_ms = started.elapsed().as_millis();
        let header = format!("{header_base} ({elapsed_ms} ms, {} bytes)", dom.len());
        let body = if dom.is_empty() {
            let tail = stderr_tail(&err);
            if tail.is_empty() {
                "(no DOM captured)".to_string()
            } else {
                format!("(no DOM captured)\nbrowser stderr: {tail}")
            }
        } else {
            dom
        };
        ToolOutput::ok(format!("{header}\n{body}"))
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

    use super::*;
    use crate::context::shared_ctx;

    fn fake_binary(dir: &Path, name: &str) -> PathBuf {
        let p = dir.join(name);
        std::fs::write(&p, "#!/bin/sh\nexit 7\n").expect("write fake binary");
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&p, std::fs::Permissions::from_mode(0o755))
                .expect("chmod fake binary");
        }
        p
    }

    #[test]
    fn discovery_matrix_picks_first_usable_candidate_in_order() {
        let dir = tempfile::tempdir().unwrap();
        let a = fake_binary(dir.path(), "chrome-a");
        let b = fake_binary(dir.path(), "chrome-b");
        let missing = dir.path().join("does-not-exist");

        assert_eq!(select_browser(&[missing.clone(), a.clone(), b]).unwrap(), a);

        let none = select_browser(&[dir.path().join("x"), dir.path().join("y")]);
        assert!(none.is_err());
    }

    #[test]
    fn explicit_override_wins_over_existing_candidates() {
        let dir = tempfile::tempdir().unwrap();
        let pinned = fake_binary(dir.path(), "pinned-browser");
        let fallback = fake_binary(dir.path(), "fallback-browser");

        let chosen =
            select_browser(&[pinned.clone(), fallback.clone(), PathBuf::from("/nope")]).unwrap();
        assert_eq!(chosen, pinned, "override must walk first");

        // The override participates in the ordered walk like any candidate:
        // remove it and the next existing entry takes over.
        std::fs::remove_file(&pinned).unwrap();
        assert_eq!(
            select_browser(&[pinned.clone(), fallback.clone()]).unwrap(),
            fallback
        );
    }

    #[test]
    fn missing_browser_error_lists_every_tried_path_and_the_env_hint() {
        let dir = tempfile::tempdir().unwrap();
        let c1 = dir.path().join("nowhere-one");
        let c2 = dir.path().join("nowhere-two");
        let err = select_browser(&[c1.clone(), c2.clone()]).unwrap_err();
        assert!(err.contains("no Chromium-family browser found"), "{err}");
        assert!(err.contains(&c1.display().to_string()), "{err}");
        assert!(err.contains(&c2.display().to_string()), "{err}");
    }

    #[test]
    fn empty_candidate_list_fails_closed_with_reason() {
        let err = select_browser(&[]).unwrap_err();
        assert!(err.contains(BROWSER_ENV), "{err}");
    }

    #[test]
    fn wait_ms_clamping_matches_contract() {
        assert_eq!(clamp_wait_ms(None), 4000);
        assert_eq!(clamp_wait_ms(Some(0)), 4000);
        assert_eq!(clamp_wait_ms(Some(-5)), 4000);
        assert_eq!(clamp_wait_ms(Some(15000)), 10000);
        assert_eq!(clamp_wait_ms(Some(2500)), 2500);
        assert_eq!(clamp_wait_ms(Some(i64::MAX)), 10000);
    }

    #[test]
    fn arg_assembly_carries_fixed_flags_profile_budget_then_url() {
        let profile = Path::new("/tmp/fake-profile");
        let args = assemble_args(clamp_wait_ms(Some(15000)), "https://example.com/", profile);
        assert_eq!(
            args,
            vec![
                "--headless=new".to_string(),
                "--disable-gpu".to_string(),
                "--no-first-run".to_string(),
                "--no-default-browser-check".to_string(),
                format!("--user-data-dir={}", profile.display()),
                "--virtual-time-budget=10000".to_string(),
                "--dump-dom".to_string(),
                "https://example.com/".to_string(),
            ]
        );
        let default_args = assemble_args(clamp_wait_ms(None), "https://example.com/", profile);
        assert!(default_args.contains(&"--virtual-time-budget=4000".to_string()));
    }

    #[test]
    fn fresh_user_data_dir_is_unique_per_call_and_removed_by_guard() {
        let first = fresh_profile_dir().unwrap();
        let second = fresh_profile_dir().unwrap();
        assert_ne!(first, second);
        assert!(first.is_dir());
        drop(ProfileDir(first.clone()));
        assert!(!first.exists());
        let second_path = second.clone();
        drop(ProfileDir(second));
        assert!(!second_path.exists());
    }

    #[tokio::test]
    async fn ssrf_screen_blocks_dangerous_targets_before_any_spawn() {
        let cases = [
            ("http://127.0.0.1/x", "loopback"),
            ("http://169.254.169.254/latest/meta-data/", "link-local"),
            ("http://[::ffff:10.0.0.1]/x", "private"),
        ];
        let ctx = shared_ctx(std::path::Path::new("."));
        for (url, class) in cases {
            let out = WebBrowseTool
                .execute(&serde_json::json!({"url": url}), &ctx)
                .await;
            assert!(out.is_error, "expected {url} to be blocked");
            assert!(
                out.content.contains("blocked") && out.content.contains(class),
                "unexpected message for {url}: {}",
                out.content
            );
        }
    }

    #[tokio::test]
    async fn unsupported_scheme_rejected_before_any_spawn() {
        let ctx = shared_ctx(std::path::Path::new("."));
        let out = WebBrowseTool
            .execute(&serde_json::json!({"url": "file:///etc/passwd"}), &ctx)
            .await;
        assert!(out.is_error);
        assert!(out.content.contains("scheme"), "{}", out.content);

        let out = WebBrowseTool.execute(&serde_json::json!({}), &ctx).await;
        assert!(out.is_error);
        assert!(out.content.contains("missing required parameter"));
    }

    #[test]
    fn parse_target_accepts_only_http_https_with_host() {
        assert!(parse_target("https://example.com/").is_ok());
        assert!(parse_target("http://example.com/x?y=1").is_ok());
        assert!(parse_target("ftp://example.com/").is_err());
        assert!(parse_target("javascript:alert(1)").is_err());
        assert!(parse_target("not a url").is_err());
    }

    #[tokio::test]
    #[ignore = "requires a locally installed Chromium-family browser and network access"]
    async fn live_browse_returns_header_line_and_dom() {
        if discover_browser().is_err() {
            eprintln!("skipping: no local browser found");
            return;
        }
        let ctx = shared_ctx(std::path::Path::new("."));
        let out = WebBrowseTool
            .execute(
                &serde_json::json!({"url": "https://example.com/", "wait_ms": 2000}),
                &ctx,
            )
            .await;
        assert!(!out.is_error, "live browse failed: {}", out.content);
        assert!(out.content.starts_with("[browse] GET "), "{}", out.content);
    }
}
