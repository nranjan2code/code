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
use std::path::{Path, PathBuf};
use std::time::Duration;

use async_trait::async_trait;
use serde_json::Value;

use crate::{ResourceClaims, Tool, ToolContext, ToolOutput};

const TOTAL_TIMEOUT_SECS: u64 = 15;
const TOTAL_TIMEOUT: Duration = Duration::from_secs(TOTAL_TIMEOUT_SECS);
/// Guards memory against a hostile or endless stream; not a context budget.
const MAX_BODY_BYTES: usize = 8 * 1024 * 1024;
const MAX_REDIRECTS: usize = 3;

/// Sites such as Wikipedia and the GitHub API refuse a request with no
/// `User-Agent`; this one names the product and where to read about it.
const USER_AGENT: &str = concat!(
    "vak/",
    env!("CARGO_PKG_VERSION"),
    " (+https://vakyartha.com)"
);

/// Arbitrary port: `ToSocketAddrs` needs one, but only the resolved IP is
/// screened; no connection is made on it.
const SCREEN_PORT: u16 = 80;

/// Fetches a page; an HTML page is read as text in the tool worker at
/// `worker_exe`.
pub struct WebFetchTool {
    worker_exe: PathBuf,
}

impl WebFetchTool {
    pub fn new(worker_exe: PathBuf) -> Self {
        Self { worker_exe }
    }
}

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
pub(crate) fn ssrf_guard(host: &str) -> Result<(), GuardRejection> {
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
pub(crate) enum GuardRejection {
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

/// No cookie store and no default auth headers: the only header added to
/// every request is the `User-Agent`.
fn build_client() -> reqwest::Result<reqwest::Client> {
    reqwest::Client::builder()
        .user_agent(USER_AGENT)
        .redirect(redirect_policy())
        .timeout(TOTAL_TIMEOUT)
        .build()
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

    fn serves(&self) -> &'static [&'static str] {
        &["web", "live-data"]
    }

    fn description(&self) -> &str {
        "Fetch a known URL over HTTP(S) with GET; this does not search the web or turn a search-results page into reliable facts. To locate current sources, discover an available search tool first. Blocks loopback/private/link-local targets, follows at most 3 redirects, downloads at most 8 MiB, accepts only text/json/xml content types, and never sends credentials. Returns a status line, then the body: an HTML page as its readable text (headings, paragraphs, lists, table rows; scripts, navigation and forms left out and counted), or with format \"links\" its links, or with \"source\" its HTML. Other content types come back as they are."
    }

    fn schema(&self) -> Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "url": {"type": "string", "description": "Absolute http(s) URL to fetch"},
                "format": PageFormat::schema()
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
        let format = match PageFormat::from_args(args) {
            Ok(format) => format,
            Err(error) => return ToolOutput::error(error),
        };
        tokio::select! {
            _ = ctx.cancel.cancelled() => ToolOutput::error("fetch cancelled"),
            out = self.run(url, format) => out,
        }
    }
}

/// What a guarded GET returned.
#[derive(Debug, Clone)]
pub struct Fetched {
    pub status: u16,
    pub final_url: String,
    pub content_type: String,
    pub etag: Option<String>,
    pub last_modified: Option<String>,
    pub body: Vec<u8>,
}

/// One GET under webfetch's guard: http(s) only, every address the host
/// and each redirect resolve to screened against loopback, private and
/// link-local ranges, at most three redirects, a total timeout, no cookies
/// or credentials, and at most `max_bytes` of body. `headers` are added as
/// given (conditional-request headers, an `Accept`). A 304 comes back with
/// an empty body; any other answer must be text, JSON or XML.
pub async fn guarded_get(
    raw: &str,
    headers: &[(&str, String)],
    max_bytes: usize,
) -> Result<Fetched, String> {
    let url = parse_target(raw)?;
    let Some(host) = url.host_str().map(str::to_string) else {
        return Err("invalid url: missing host".into());
    };
    tokio::task::spawn_blocking(move || ssrf_guard(&host))
        .await
        .map_err(|e| format!("address screening failed: {e}"))?
        .map_err(|rejection| rejection.to_string())?;
    let client = build_client().map_err(|e| format!("client build failed: {e}"))?;
    let mut request = client.get(url);
    for (name, value) in headers {
        request = request.header(*name, value);
    }
    let mut resp = request
        .send()
        .await
        .map_err(|e| request_error_message(&e))?;
    let status = resp.status();
    let header = |name: reqwest::header::HeaderName| {
        resp.headers()
            .get(name)
            .and_then(|v| v.to_str().ok())
            .map(str::trim)
            .map(str::to_owned)
    };
    let content_type = header(reqwest::header::CONTENT_TYPE).unwrap_or_default();
    let etag = header(reqwest::header::ETAG);
    let last_modified = header(reqwest::header::LAST_MODIFIED);
    let final_url = resp.url().to_string();
    if status == reqwest::StatusCode::NOT_MODIFIED {
        return Ok(Fetched {
            status: status.as_u16(),
            final_url,
            content_type,
            etag,
            last_modified,
            body: Vec::new(),
        });
    }
    if !content_type_allowed(content_type.as_str()) {
        let shown = if content_type.is_empty() {
            "missing"
        } else {
            content_type.as_str()
        };
        return Err(format!("unsupported content type: {shown}"));
    }
    let mut body: Vec<u8> = Vec::new();
    while let Some(chunk) = resp.chunk().await.map_err(|e| request_error_message(&e))? {
        if body.len() + chunk.len() > max_bytes {
            return Err(format!("response body exceeds the {max_bytes} byte cap"));
        }
        body.extend_from_slice(&chunk);
    }
    Ok(Fetched {
        status: status.as_u16(),
        final_url,
        content_type,
        etag,
        last_modified,
        body,
    })
}

/// A server's error status is an error value, never a page: a 403 or 404
/// body is the site's own chrome, and read as a result it made the run
/// count a retrieval that found nothing, then ask the answer to "use" it
/// and to show it as a card (live: a weather card asked for after a 403).
/// The message names only the status, so its failure kind comes from the
/// status words (`Forbidden` is an access failure) and never from words
/// in a page, which could read as a correctable argument fault.
fn http_error_output(url: &str, status: u16) -> ToolOutput {
    let status = reqwest::StatusCode::from_u16(status)
        .map_or_else(|_| status.to_string(), |status| status.to_string());
    ToolOutput::error(format!(
        "[webfetch] GET {url} -> {status}: the server answered with an error instead of \
         the page, so nothing was retrieved from it."
    ))
}

impl WebFetchTool {
    async fn run(&self, raw: &str, format: PageFormat) -> ToolOutput {
        match guarded_get(raw, &[], MAX_BODY_BYTES).await {
            Ok(fetched) if fetched.status >= 400 => {
                http_error_output(&fetched.final_url, fetched.status)
            }
            Ok(fetched) => {
                let body = String::from_utf8_lossy(&fetched.body);
                let text = if is_html(&fetched.content_type) {
                    match page(&self.worker_exe, &body, &fetched.final_url, format).await {
                        Ok(text) => text,
                        Err(error) => return ToolOutput::error(error),
                    }
                } else {
                    body.into_owned()
                };
                let header = format!(
                    "[webfetch] GET {} -> {} ({}, {} bytes)",
                    fetched.final_url,
                    reqwest::StatusCode::from_u16(fetched.status)
                        .map_or_else(|_| fetched.status.to_string(), |status| status.to_string()),
                    fetched.content_type,
                    fetched.body.len()
                );
                ToolOutput::ok(format!("{header}\n{text}"))
            }
            Err(error) => ToolOutput::error(error),
        }
    }
}

/// What a fetched or rendered HTML page comes back as.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PageFormat {
    /// Its readable text, with what was left out counted.
    Text,
    /// Its links, one per line.
    Links,
    /// Its HTML, as the server sent it.
    Source,
}

impl PageFormat {
    pub fn schema() -> Value {
        serde_json::json!({
            "type": "string",
            "enum": ["text", "links", "source"],
            "description": "For an HTML page: \"text\" (default) its readable text, \"links\" its links, \"source\" its HTML"
        })
    }

    pub fn from_args(args: &Value) -> Result<Self, String> {
        match args.get("format").and_then(Value::as_str) {
            None | Some("text") => Ok(Self::Text),
            Some("links") => Ok(Self::Links),
            Some("source") => Ok(Self::Source),
            Some(other) => Err(format!(
                "format must be \"text\", \"links\" or \"source\", not \"{other}\""
            )),
        }
    }
}

fn is_html(content_type: &str) -> bool {
    let mime = content_type
        .split(';')
        .next()
        .unwrap_or_default()
        .trim()
        .to_ascii_lowercase();
    mime == "text/html" || mime == "application/xhtml+xml"
}

/// An HTML page as `format` asks, read in the tool worker (invariant 14).
/// Anything left out of the text is said in its first line.
pub async fn page(
    worker_exe: &Path,
    html: &str,
    base: &str,
    format: PageFormat,
) -> Result<String, String> {
    if format == PageFormat::Source {
        return Ok(html.to_owned());
    }
    let read = crate::broker::read_html(worker_exe, html, base)
        .await
        .map_err(|error| {
            format!("the page could not be read as text ({error}); ask with format \"source\" for its HTML")
        })?;
    Ok(render_page(&read, format))
}

fn render_page(read: &vak_intake::html::Readable, format: PageFormat) -> String {
    let named = match &read.title {
        Some(title) => format!("\"{title}\""),
        None => "The page".into(),
    };
    let region = match read.region.as_str() {
        "main" => "its main content",
        "article" => "its article",
        "body" => "its body",
        _ => "the document",
    };
    if format == PageFormat::Links {
        let mut out = format!("[page] {named}: {} links in {region}.", read.links.len());
        for link in &read.links {
            if link.text.is_empty() {
                out.push_str(&format!("\n- {}", link.url));
            } else {
                out.push_str(&format!("\n- {}: {}", link.text, link.url));
            }
        }
        return out;
    }
    let mut left_out = Vec::new();
    for (count, what) in [
        (read.left_out.code, "scripts, styles and embedded media"),
        (
            read.left_out.navigation,
            "navigation and other-language links",
        ),
        (read.left_out.forms, "forms and buttons"),
        (read.left_out.hidden, "hidden elements"),
    ] {
        if count > 0 {
            left_out.push(format!("{count} {what}"));
        }
    }
    let mut head = format!(
        "[page] {named}, the readable text of {region}: {} characters.",
        read.text.chars().count()
    );
    if !left_out.is_empty() {
        head.push_str(&format!(" Left out: {}.", left_out.join(", ")));
    }
    head.push_str(&format!(
        " {} links: ask with format \"links\" for them, or \"source\" for the HTML.",
        read.links.len()
    ));
    if read.text.is_empty() {
        head.push_str(" The page has no readable text there; it may build its content with JavaScript, which browse renders.");
        return head;
    }
    format!("{head}\n\n{}", read.text)
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

    use super::*;

    use crate::context::shared_ctx;

    fn tool() -> WebFetchTool {
        WebFetchTool::new(PathBuf::from("/nonexistent/vak-tool-worker"))
    }

    #[test]
    fn a_page_says_what_it_left_out() {
        let read = vak_intake::html::readable(
            "<title>T</title><body><nav>menu</nav><p>Hello <a href='https://a.example/x'>there</a></p></body>",
            None,
        );
        assert_eq!(
            render_page(&read, PageFormat::Text),
            "[page] \"T\", the readable text of its body: 11 characters. Left out: 1 navigation and other-language links. 1 links: ask with format \"links\" for them, or \"source\" for the HTML.\n\nHello there"
        );
        assert_eq!(
            render_page(&read, PageFormat::Links),
            "[page] \"T\": 1 links in its body.\n- there: https://a.example/x"
        );
    }

    #[test]
    fn format_is_one_of_three() {
        assert_eq!(
            PageFormat::from_args(&serde_json::json!({})),
            Ok(PageFormat::Text)
        );
        assert_eq!(
            PageFormat::from_args(&serde_json::json!({"format": "source"})),
            Ok(PageFormat::Source)
        );
        assert!(PageFormat::from_args(&serde_json::json!({"format": "html"})).is_err());
        assert!(is_html("text/html; charset=UTF-8"));
        assert!(!is_html("text/plain"));
    }

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
        let out = tool().execute(&serde_json::json!({}), &ctx).await;
        assert!(out.is_error);
        assert!(out.content.contains("missing required parameter"));
    }

    #[tokio::test]
    async fn unsupported_scheme_is_a_typed_error_before_any_connection() {
        let ctx = shared_ctx(std::path::Path::new("."));
        let out = tool()
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
            let out = tool().execute(&serde_json::json!({"url": url}), &ctx).await;
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
    async fn client_sends_a_descriptive_user_agent_and_no_credentials() {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};

        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let server = tokio::spawn(async move {
            let (mut sock, _) = listener.accept().await.unwrap();
            let mut buf = Vec::new();
            let mut chunk = [0u8; 1024];
            while !buf.windows(4).any(|w| w == b"\r\n\r\n") {
                let n = sock.read(&mut chunk).await.unwrap();
                assert!(n > 0, "connection closed before headers ended");
                buf.extend_from_slice(&chunk[..n]);
            }
            sock.write_all(b"HTTP/1.1 204 No Content\r\nContent-Length: 0\r\n\r\n")
                .await
                .unwrap();
            String::from_utf8(buf).unwrap().to_ascii_lowercase()
        });

        let resp = build_client()
            .unwrap()
            .get(format!("http://{addr}/"))
            .send()
            .await
            .unwrap();
        assert_eq!(resp.status(), reqwest::StatusCode::NO_CONTENT);
        let request = server.await.unwrap();

        let expected = format!(
            "user-agent: vak/{} (+https://vakyartha.com)\r\n",
            env!("CARGO_PKG_VERSION")
        );
        assert!(request.contains(&expected), "{request}");
        assert!(!request.contains("\r\ncookie:"), "{request}");
        assert!(!request.contains("\r\nauthorization:"), "{request}");
    }

    #[tokio::test]
    #[ignore = "requires network access"]
    async fn live_fetch_returns_header_line_and_body() {
        let ctx = shared_ctx(std::path::Path::new("."));
        let out = tool()
            .execute(&serde_json::json!({"url": "https://example.com"}), &ctx)
            .await;
        assert!(!out.is_error, "live fetch failed: {}", out.content);
        assert!(
            out.content.starts_with("[webfetch] GET "),
            "{}",
            out.content
        );
    }

    #[test]
    fn an_error_status_is_an_error_that_asks_for_no_argument_fix() {
        for status in [400, 401, 403, 404, 410, 500, 503] {
            let output = super::http_error_output("https://example.com/a", status);
            assert!(output.is_error, "{status}");
            assert!(
                !output.classify().is_correctable(),
                "{status}: {}",
                output.content
            );
        }
    }
}
