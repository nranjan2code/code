//! The renderer behind `webfetch`'s `render`: drives a locally installed
//! Chromium-family browser as a child process and returns the JavaScript-
//! rendered DOM (docs/design/29-personal-os.md P4). It is no tool of its
//! own: one reader of a URL, `webfetch`, carries webfetch's permission and
//! provenance, and the model says whether the page needs its script run
//! (docs/audits/tool-catalogue-2026-10-16.md, O7). It performs no
//! permission checks itself. Credentials are never sent: fresh throwaway profile, no cookie
//! reuse, no auth material.
//!
//! Sandbox posture: unlike Bash, no Seatbelt/Landlock command wrapper is
//! applied to the caller; the renderer itself runs under a private OS sandbox.
//! Containment includes a screened SOCKS proxy and a fixed, discovered browser
//! binary with fixed flags, a scrubbed allowlist environment, a fresh
//! profile directory that is deleted afterwards, and its own process group
//! killed on timeout/cancel.
//!
//! Every browser connection goes through a screened, pinned SOCKS proxy.
//! The OS sandbox admits only that proxy port, including for redirects and
//! subresources; unsupported network isolation refuses rendering.

use crate::Fault;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant};

use serde_json::Value;

use crate::webfetch::ssrf_guard;
use crate::{ToolContext, ToolOutput};

const DEFAULT_WAIT_MS: u64 = 4000;
const TOTAL_TIMEOUT_SECS: u64 = 20;
const TOTAL_TIMEOUT: Duration = Duration::from_secs(TOTAL_TIMEOUT_SECS);
const BROWSER_ENV: &str = "VAK_BROWSER";

/// Renders a page; its DOM is read as text in the tool worker at
/// `worker_exe`.
pub struct Renderer {
    worker_exe: std::path::PathBuf,
}

impl Renderer {
    pub fn new(worker_exe: std::path::PathBuf) -> Self {
        Self { worker_exe }
    }
}

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

impl Renderer {
    /// Render `args.url` and read it in the `format` asked for.
    pub async fn render(&self, args: &Value, ctx: &ToolContext) -> ToolOutput {
        if args.get("url").and_then(Value::as_str).is_none() {
            return ToolOutput::error(Fault::Shape, "missing required parameter: url");
        }
        if let Err(error) = crate::webfetch::PageFormat::from_args(args) {
            return ToolOutput::error(Fault::Shape, error);
        }
        tokio::select! {
            _ = ctx.cancel.cancelled() => ToolOutput::error(Fault::Cancelled, "render cancelled"),
            out = self.run(args, ctx) => out,
        }
    }
}

impl Renderer {
    async fn run(&self, args: &Value, ctx: &ToolContext) -> ToolOutput {
        // Unwrap-free re-extraction after the execute() pre-check keeps this
        // method total even when called directly in tests.
        let raw = match args.get("url").and_then(Value::as_str) {
            Some(r) => r,
            None => return ToolOutput::error(Fault::Shape, "missing required parameter: url"),
        };
        let wait_ms = DEFAULT_WAIT_MS;

        let url = match parse_target(raw) {
            Ok(u) => u,
            Err(e) => return ToolOutput::error(Fault::Shape, e),
        };
        let Some(host) = url.host_str().map(str::to_string) else {
            return ToolOutput::error(Fault::Shape, "invalid url: missing host");
        };
        let screened = match tokio::task::spawn_blocking(move || ssrf_guard(&host)).await {
            Ok(r) => r,
            Err(e) => {
                return ToolOutput::error(Fault::Tool, format!("address screening failed: {e}"));
            }
        };
        if let Err(rejection) = screened {
            return ToolOutput::error(Fault::Refused, rejection.to_string());
        }

        let browser = match discover_browser() {
            Ok(b) => b,
            Err(e) => return ToolOutput::error(Fault::Tool, e),
        };
        let profile = match fresh_profile_dir() {
            Ok(p) => ProfileDir(p),
            Err(e) => {
                return ToolOutput::error(Fault::Tool, format!("profile dir creation failed: {e}"));
            }
        };

        // Capture DOM/stderr via FILES, not pipes: Chromium helper
        // processes inherit pipe fds and never let EOF fire, while file
        // redirection makes output inspectable while Chrome still runs.
        let out_path = profile.0.join("dom.html");
        let err_path = profile.0.join("stderr.txt");
        let out_file = match std::fs::File::create(&out_path) {
            Ok(f) => f,
            Err(e) => {
                return ToolOutput::error(Fault::Tool, format!("dom capture create failed: {e}"));
            }
        };
        let err_file = match std::fs::File::create(&err_path) {
            Ok(f) => f,
            Err(e) => {
                return ToolOutput::error(
                    Fault::Tool,
                    format!("stderr capture create failed: {e}"),
                );
            }
        };

        let proxy = match crate::egress::Egress::screened_browser().await {
            Ok(proxy) => proxy,
            Err(_) => return ToolOutput::error(Fault::Tool, "browser network guard unavailable"),
        };
        let sandbox: Option<std::sync::Arc<dyn crate::sandbox::Sandbox>> = {
            #[cfg(target_os = "macos")]
            {
                Some(std::sync::Arc::new(crate::sandbox::Seatbelt::browser(
                    &profile.0, &browser,
                )))
            }
            #[cfg(target_os = "linux")]
            {
                Some(std::sync::Arc::new(crate::landlock::Landlock::new(
                    crate::sandbox::SandboxMode::WorkspaceWrite,
                    &profile.0,
                )))
            }
            #[cfg(not(any(target_os = "macos", target_os = "linux")))]
            {
                None
            }
        };
        let Some(sandbox) = sandbox
            .filter(|sandbox| sandbox.protects_workspace_control())
            .and_then(|sandbox| sandbox.proxied_variant(proxy.port()))
        else {
            return ToolOutput::error(
                Fault::Refused,
                "secure browser network isolation is unavailable on this platform",
            );
        };
        let mut browser_args = assemble_args(wait_ms, url.as_str(), &profile.0);
        browser_args.extend([
            format!("--proxy-server=socks5://127.0.0.1:{}", proxy.port()),
            "--proxy-bypass-list=<-loopback>".into(),
            "--host-resolver-rules=MAP * ~NOTFOUND".into(),
            "--disable-background-networking".into(),
            "--disable-crashpad-for-testing".into(),
            "--disable-quic".into(),
        ]);
        let command = std::iter::once(browser.to_string_lossy().into_owned())
            .chain(browser_args)
            .map(|arg| crate::bash::shell_quote(&arg))
            .collect::<Vec<_>>()
            .join(" ");
        let mut cmd = tokio::process::Command::new(crate::bash::POSIX_SHELL);
        cmd.arg("-c")
            .arg(sandbox.wrap(&command))
            .current_dir(&profile.0)
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::from(out_file))
            .stderr(std::process::Stdio::from(err_file))
            .kill_on_drop(true);
        crate::bash::scrub_environment(&mut cmd);
        // Chromium creates its singleton socket under the system temporary
        // directory by default. Keep that socket inside its private sandbox.
        cmd.env("TMPDIR", &profile.0);
        // Chromium uses its own hermetic temp override on macOS.
        cmd.env("MAC_CHROMIUM_TMPDIR", &profile.0);
        crate::bash::isolate_process_group(&mut cmd);

        let started = Instant::now();
        let mut child = match cmd.spawn() {
            Ok(c) => c,
            Err(e) => {
                return ToolOutput::error(
                    Fault::Tool,
                    format!("failed to launch {}: {e}", basename(&browser)),
                );
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
                return ToolOutput::error(
                    Fault::Tool,
                    format!(
                        "{header_base} ({elapsed_ms} ms): exceeded its {TOTAL_TIMEOUT_SECS}s total timeout"
                    ),
                );
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
                    return ToolOutput::error(Fault::Cancelled, format!("{header_base}: browse cancelled"));
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
            return ToolOutput::error(
                Fault::Tool,
                if tail.is_empty() {
                    format!("{header}: secure browser launch produced no page; use ordinary fetch")
                } else {
                    format!(
                        "{header}: secure browser launch produced no page; use ordinary fetch\nbrowser stderr: {tail}"
                    )
                },
            );
        } else {
            let format = crate::webfetch::PageFormat::from_args(args)
                .unwrap_or(crate::webfetch::PageFormat::Text);
            match crate::webfetch::page(&self.worker_exe, &dom, url.as_str(), format).await {
                Ok(page) => page,
                Err(error) => return ToolOutput::error(Fault::Tool, format!("{header}: {error}")),
            }
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
    fn arg_assembly_carries_fixed_flags_profile_budget_then_url() {
        let profile = Path::new("/tmp/fake-profile");
        let args = assemble_args(10000, "https://example.com/", profile);
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
        let default_args = assemble_args(DEFAULT_WAIT_MS, "https://example.com/", profile);
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
            let out = Renderer::new(std::path::PathBuf::from("/nonexistent/vak-tool-worker"))
                .render(&serde_json::json!({"url": url}), &ctx)
                .await;
            assert!(out.is_error(), "expected {url} to be blocked");
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
        let out = Renderer::new(std::path::PathBuf::from("/nonexistent/vak-tool-worker"))
            .render(&serde_json::json!({"url": "file:///etc/passwd"}), &ctx)
            .await;
        assert!(out.is_error());
        assert!(out.content.contains("scheme"), "{}", out.content);

        let out = Renderer::new(std::path::PathBuf::from("/nonexistent/vak-tool-worker"))
            .render(&serde_json::json!({}), &ctx)
            .await;
        assert!(out.is_error());
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
    #[allow(clippy::disallowed_macros)]
    async fn live_browse_returns_header_line_and_dom() {
        if discover_browser().is_err() {
            eprintln!("skipping: no local browser found");
            return;
        }
        let ctx = shared_ctx(std::path::Path::new("."));
        let out = Renderer::new(std::path::PathBuf::from("/nonexistent/vak-tool-worker"))
            .render(
                &serde_json::json!({"url": "https://example.com/", "wait_ms": 2000}),
                &ctx,
            )
            .await;
        assert!(!out.is_error(), "live browse failed: {}", out.content);
        assert!(out.content.starts_with("[browse] GET "), "{}", out.content);
    }
}
