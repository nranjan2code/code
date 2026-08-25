//! Bounded outbound HTTP GET tool (docs/design/29-personal-os.md phase P4).
//!
//! Permission posture (Ask in restricted modes, Allow under FullAccess or an
//! explicit rule) is applied by the registry/permission engine downstream;
//! this tool performs no permission checks itself.
//!
//! Credentials are never sent: bare requests, no cookie store, no auth
//! headers, ever.

use std::error::Error as StdError;
use std::fmt;
use std::net::{IpAddr, Ipv4Addr, ToSocketAddrs};
use std::time::Duration;

use async_trait::async_trait;
use serde_json::Value;

use crate::{ResourceClaims, Tool, ToolContext, ToolOutput};

const TOTAL_TIMEOUT_SECS: u64 = 15;
const TOTAL_TIMEOUT: Duration = Duration::from_secs(TOTAL_TIMEOUT_SECS);
const MAX_BODY_BYTES: usize = 512 * 1024;
const MAX_REDIRECTS: usize = 3;

/// Arbitrary port: `ToSocketAddrs` needs one, but only the resolved IP is
/// screened; no connection is made on it.
const SCREEN_PORT: u16 = 80;

pub struct WebFetchTool;

/// Marker threaded into a redirect-policy rejection so the typed block reason
/// survives inside the opaque `reqwest::Error` source chain.
#[derive(Debug)]
struct BlockedRange(String);

impl fmt::Display for BlockedRange {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

impl StdError for BlockedRange {}

fn blocked_message(class: &str) -> String {
    format!("blocked: {class} address range")
}

fn classify_v4(ip: Ipv4Addr) -> Option<&'static str> {
    if ip.is_loopback() {
        Some("loopback")
    } else if ip.is_unspecified() {
        Some("unspecified")
    } else if ip.is_private() {
        Some("private RFC1918")
    } else if ip.is_link_local() {
        Some("link-local")
    } else {
        None
    }
}

fn classify(ip: IpAddr) -> Option<&'static str> {
    match ip {
        IpAddr::V4(v4) => classify_v4(v4),
        IpAddr::V6(v6) => {
            if let Some(mapped) = v6.to_ipv4_mapped() {
                classify_v4(mapped)
            } else if v6.is_loopback() {
                Some("loopback")
            } else if v6.is_unspecified() {
                Some("unspecified")
            } else if v6.is_unique_local() {
                Some("unique-local")
            } else if v6.is_unicast_link_local() {
                Some("link-local")
            } else {
                None
            }
        }
    }
}

/// Pre-connect SSRF screen: resolves `host` and fails closed if ANY resolved
/// address falls in a blocked range. v1 acknowledges the TOCTOU window of
/// resolving here and letting the client resolve again (DNS rebinding);
/// mitigation roadmap is single-use-resolve-then-connect pinning.
fn ssrf_guard(host: &str) -> Result<(), GuardRejection> {
    // URL serialization brackets IPv6 literals ("[::1]"); ToSocketAddrs
    // wants the bare address.
    let bare = host
        .strip_prefix('[')
        .and_then(|rest| rest.strip_suffix(']'))
        .unwrap_or(host);
    let addrs = (bare, SCREEN_PORT).to_socket_addrs().map_err(|e| {
        GuardRejection::Unresolvable(format!("dns resolution failed for {bare}: {e}"))
    })?;
    for addr in addrs {
        if let Some(class) = classify(addr.ip()) {
            return Err(GuardRejection::Protected(class));
        }
    }
    Ok(())
}

/// Why the SSRF screen refused a host; every variant renders as a typed,
/// self-explanatory tool error.
#[derive(Debug)]
enum GuardRejection {
    /// The lookup itself failed (fail closed); carries the rendered reason.
    Unresolvable(String),
    /// At least one resolved address fell in a protected range.
    Protected(&'static str),
}

impl fmt::Display for GuardRejection {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Unresolvable(detail) => write!(f, "{detail}"),
            Self::Protected(class) => write!(f, "{}", blocked_message(class)),
        }
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

fn follow_redirect(previous_count: usize) -> bool {
    previous_count < MAX_REDIRECTS
}

fn redirect_policy() -> reqwest::redirect::Policy {
    reqwest::redirect::Policy::custom(|attempt| {
        if !follow_redirect(attempt.previous().len()) {
            return attempt.error("too many redirects");
        }
        match attempt.url().host_str().map(ssrf_guard) {
            Some(Ok(())) => attempt.follow(),
            Some(Err(rejection)) => attempt.error(BlockedRange(rejection.to_string())),
            None => attempt.error(BlockedRange("blocked: missing-host".to_string())),
        }
    })
}

fn content_type_allowed(content_type: &str) -> bool {
    let mime = content_type
        .split(';')
        .next()
        .unwrap_or_default()
        .trim()
        .to_ascii_lowercase();
    mime == "application/json"
        || mime == "application/xhtml+xml"
        || mime == "application/xml"
        || mime.ends_with("+xml")
        || mime.starts_with("text/")
}

fn find_blocked<'a>(err: &'a (dyn StdError + 'static)) -> Option<&'a BlockedRange> {
    let mut current: Option<&(dyn StdError + 'static)> = Some(err);
    while let Some(e) = current {
        if let Some(blocked) = e.downcast_ref::<BlockedRange>() {
            return Some(blocked);
        }
        current = e.source();
    }
    None
}

fn request_error_message(e: &reqwest::Error) -> String {
    if let Some(blocked) = find_blocked(e) {
        return blocked.to_string();
    }
    if e.is_redirect() {
        return format!("too many redirects (limit {MAX_REDIRECTS})");
    }
    if e.is_timeout() {
        return format!("request exceeded its {TOTAL_TIMEOUT_SECS}s total timeout");
    }
    format!("request failed: {e}")
}

#[async_trait]
impl Tool for WebFetchTool {
    fn name(&self) -> &str {
        "webfetch"
    }

    fn description(&self) -> &str {
        "Fetch a URL over HTTP(S) with GET. Blocks loopback/private/link-local targets, follows at most 3 redirects, caps the body at 512KiB, accepts only text/json/xml content types, and returns a status header line followed by the UTF-8 body. Never sends credentials."
    }

    fn schema(&self) -> Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "url": {"type": "string", "description": "Absolute http(s) URL to fetch"}
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
        let Some(url) = args.get("url").and_then(|u| u.as_str()) else {
            return ToolOutput::error("missing required parameter: url");
        };
        tokio::select! {
            _ = ctx.cancel.cancelled() => ToolOutput::error("fetch cancelled"),
            out = self.run(url, ctx) => out,
        }
    }
}

impl WebFetchTool {
    async fn run(&self, raw: &str, ctx: &ToolContext) -> ToolOutput {
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

        let client = match reqwest::Client::builder()
            .redirect(redirect_policy())
            .timeout(TOTAL_TIMEOUT)
            .build()
        {
            Ok(c) => c,
            Err(e) => return ToolOutput::error(format!("client build failed: {e}")),
        };

        let resp = match client.get(url).send().await {
            Ok(r) => r,
            Err(e) => return ToolOutput::error(request_error_message(&e)),
        };
        let status = resp.status();
        let final_url = resp.url().clone();
        let content_type = resp
            .headers()
            .get(reqwest::header::CONTENT_TYPE)
            .and_then(|v| v.to_str().ok())
            .map(str::trim)
            .unwrap_or_default()
            .to_owned();
        if !content_type_allowed(content_type.as_str()) {
            let shown = if content_type.is_empty() {
                "missing"
            } else {
                content_type.as_str()
            };
            return ToolOutput::error(format!("unsupported content type: {shown}"));
        }

        let mut resp = resp;
        let mut body: Vec<u8> = Vec::new();
        loop {
            match resp.chunk().await {
                Ok(Some(chunk)) => {
                    if body.len() + chunk.len() > MAX_BODY_BYTES {
                        return ToolOutput::error(format!(
                            "response body exceeds the {MAX_BODY_BYTES} byte cap"
                        ));
                    }
                    body.extend_from_slice(&chunk);
                }
                Ok(None) => break,
                Err(e) => return ToolOutput::error(request_error_message(&e)),
            }
        }

        let text = String::from_utf8_lossy(&body);
        let header = format!(
            "[webfetch] GET {final_url} -> {status} ({content_type}, {count} bytes)",
            count = body.len()
        );
        ToolOutput::ok(ctx.truncate_output(format!("{header}\n{text}")))
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

    use super::*;

    use crate::context::shared_ctx;

    #[test]
    fn scheme_validation_matrix() {
        for ok in [
            "http://example.com/",
            "https://example.com/",
            "HTTPS://Example.COM/",
        ] {
            assert!(parse_target(ok).is_ok(), "expected {ok} to pass");
        }
        for bad in [
            "ftp://example.com/",
            "file:///etc/passwd",
            "javascript:alert(1)",
            "data:text/plain,hi",
            "unix:/var/run/sock",
            "//example.com/no-scheme",
            "example.com/bare",
            "",
        ] {
            assert!(parse_target(bad).is_err(), "expected {bad} to fail");
        }
        let msg = parse_target("ftp://example.com/").unwrap_err();
        assert!(msg.contains("ftp") && msg.contains("scheme"), "{msg}");
    }

    #[tokio::test]
    async fn missing_url_parameter_is_a_typed_error() {
        let ctx = shared_ctx(std::path::Path::new("."));
        let out = WebFetchTool.execute(&serde_json::json!({}), &ctx).await;
        assert!(out.is_error);
        assert!(out.content.contains("missing required parameter"));
    }

    #[tokio::test]
    async fn unsupported_scheme_is_a_typed_error_before_any_connection() {
        let ctx = shared_ctx(std::path::Path::new("."));
        let out = WebFetchTool
            .execute(&serde_json::json!({"url": "ftp://example.com/x"}), &ctx)
            .await;
        assert!(out.is_error);
        assert!(out.content.contains("ftp"));
    }

    #[tokio::test]
    async fn ssrf_matrix_blocks_dangerous_ranges_via_execute() {
        let cases = [
            ("http://127.0.0.1/x", "loopback"),
            ("http://[::1]/x", "loopback"),
            ("http://0.0.0.0/x", "unspecified"),
            ("http://[::]/x", "unspecified"),
            ("http://10.11.12.13/x", "private"),
            ("http://172.16.0.1/x", "private"),
            ("http://172.31.254.3/x", "private"),
            ("http://192.168.50.50/x", "private"),
            ("http://169.254.169.254/latest/meta-data/", "link-local"),
            ("http://[fe80::1]/x", "link-local"),
            ("http://[fd00::1]/x", "unique-local"),
            ("http://[fc00::]/x", "unique-local"),
            ("http://[::ffff:10.0.0.1]/x", "private"),
            ("http://[::ffff:127.0.0.1]/x", "loopback"),
            ("http://[::ffff:169.254.169.254]/", "link-local"),
        ];
        let ctx = shared_ctx(std::path::Path::new("."));
        for (url, class) in cases {
            let out = WebFetchTool
                .execute(&serde_json::json!({"url": url}), &ctx)
                .await;
            assert!(
                out.is_error,
                "expected {url} to be blocked, got: {}",
                out.content
            );
            assert!(
                out.content.contains("blocked") && out.content.contains(class),
                "unexpected message for {url}: {}",
                out.content
            );
        }
    }

    #[test]
    fn mapped_ipv6_addresses_are_screened_like_ipv4() {
        assert_eq!(
            classify("::ffff:10.0.0.1".parse().unwrap()),
            Some("private RFC1918")
        );
        assert_eq!(
            classify("::ffff:172.16.9.9".parse().unwrap()),
            Some("private RFC1918")
        );
        assert_eq!(
            classify("::ffff:192.168.0.1".parse().unwrap()),
            Some("private RFC1918")
        );
        assert_eq!(
            classify("::ffff:127.0.0.1".parse().unwrap()),
            Some("loopback")
        );
        assert_eq!(
            classify("::ffff:169.254.169.254".parse().unwrap()),
            Some("link-local")
        );
        assert_eq!(
            classify("::ffff:0.0.0.0".parse().unwrap()),
            Some("unspecified")
        );
    }

    #[test]
    fn public_ranges_are_not_classified_as_internal() {
        assert_eq!(classify("8.8.8.8".parse().unwrap()), None);
        assert_eq!(classify("172.32.0.1".parse().unwrap()), None);
        assert_eq!(classify("198.51.100.7".parse().unwrap()), None);
        assert_eq!(classify("2606:4700::1111".parse().unwrap()), None);
        assert_eq!(classify("::ffff:8.8.8.8".parse().unwrap()), None);
    }

    #[test]
    fn redirect_limit_allows_three_and_rejects_the_fourth() {
        assert!(follow_redirect(0));
        assert!(follow_redirect(1));
        assert!(follow_redirect(2));
        assert!(!follow_redirect(3));
        assert!(!follow_redirect(4));
    }

    #[test]
    fn content_type_filter_accepts_only_document_types() {
        for accepted in [
            "text/html; charset=utf-8",
            "text/plain",
            "Text/Markdown",
            "application/json",
            "application/xhtml+xml",
            "application/xml",
            "text/xml",
            "application/atom+xml",
        ] {
            assert!(
                content_type_allowed(accepted),
                "expected {accepted} to be accepted"
            );
        }
        for rejected in [
            "image/png",
            "image/jpeg",
            "application/octet-stream",
            "video/mp4",
            "audio/mpeg",
            "multipart/form-data",
            "",
        ] {
            assert!(
                !content_type_allowed(rejected),
                "expected {rejected:?} to be rejected"
            );
        }
    }

    #[tokio::test]
    #[ignore = "requires network access"]
    async fn live_fetch_returns_header_line_and_body() {
        let ctx = shared_ctx(std::path::Path::new("."));
        let out = WebFetchTool
            .execute(&serde_json::json!({"url": "https://example.com"}), &ctx)
            .await;
        assert!(!out.is_error, "live fetch failed: {}", out.content);
        assert!(
            out.content.starts_with("[webfetch] GET "),
            "{}",
            out.content
        );
    }
}
