//! Preview origins (docs/design/66, §3.2).
//!
//! A preview that needs its own files (a saved draft's site, a run's output, a
//! page in the workspace) is served from an origin of its own instead of being
//! poured into a `srcdoc` frame. Each preview gets a listener on a fresh
//! loopback port: a different port is a different browser origin, so the
//! previewed page cannot script the app, and its relative stylesheets, scripts,
//! modules, images and links work as they do on any web server. The listener
//! answers only the files its scope names and only under an unguessable path
//! prefix; the app's own routes, cookies and credentials never reach it.
//!
//! This is a loopback facility. A page cannot reach a loopback port on
//! another machine, so a request that did not arrive by a loopback name is
//! refused with a reason and the client falls back to its single-page frame.

use std::{
    collections::HashMap,
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};

use axum::{
    Router,
    body::Body,
    extract::State,
    http::{HeaderMap, Method, StatusCode, Uri, header},
    response::{IntoResponse, Response},
};
use subtle::ConstantTimeEq;
use tokio_util::sync::CancellationToken;

use crate::AppState;

/// More than this and the oldest preview is closed to make room.
const MAX_PREVIEWS: usize = 16;
/// A preview nobody closed (a browser that vanished) does not live forever.
const MAX_AGE: Duration = Duration::from_secs(12 * 60 * 60);

/// What a preview may read.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum Scope {
    /// The files of one saved version of a draft.
    Candidate {
        session_id: String,
        candidate_id: String,
    },
    /// The files one run left in its scratch directory.
    Execution {
        session_id: String,
        execution_id: String,
    },
    /// A workspace below one directory (relative, no leading or trailing
    /// slash; empty means the workspace root). `root` is the workspace the
    /// conversation works in, an Agent's own folder; a file is read from
    /// there exactly and is never looked for anywhere else.
    Workspace {
        root: std::path::PathBuf,
        directory: String,
    },
}

struct Entry {
    stop: CancellationToken,
    created: Instant,
}

/// The open previews, by id. Closing one stops its listener.
#[derive(Clone, Default)]
pub(crate) struct PreviewHub {
    entries: Arc<Mutex<HashMap<String, Entry>>>,
}

/// A preview that is listening.
pub(crate) struct Opened {
    pub(crate) id: String,
    pub(crate) port: u16,
    pub(crate) token: String,
}

#[derive(Debug, PartialEq, Eq)]
pub(crate) enum OpenError {
    /// The entry file is not readable through the scope.
    NotFound,
    Unavailable(String),
}

impl PreviewHub {
    pub(crate) async fn open(
        &self,
        state: AppState,
        scope: Scope,
        entry: &str,
    ) -> Result<Opened, OpenError> {
        // Fail here, not in a blank frame: the entry must be readable.
        let relative = normalize(entry).ok_or(OpenError::NotFound)?;
        read_scoped(&state, &scope, &relative)
            .await
            .map_err(|_| OpenError::NotFound)?;

        let listener = tokio::net::TcpListener::bind((std::net::Ipv4Addr::LOCALHOST, 0))
            .await
            .map_err(|error| OpenError::Unavailable(error.to_string()))?;
        let port = listener
            .local_addr()
            .map_err(|error| OpenError::Unavailable(error.to_string()))?
            .port();
        let token = random_token().map_err(OpenError::Unavailable)?;
        let id = uuid::Uuid::now_v7().to_string();
        let stop = CancellationToken::new();

        {
            let mut entries = self
                .entries
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            entries.retain(|_, entry| {
                let keep = entry.created.elapsed() < MAX_AGE;
                if !keep {
                    entry.stop.cancel();
                }
                keep
            });
            while entries.len() >= MAX_PREVIEWS {
                let Some(oldest) = entries
                    .iter()
                    .min_by_key(|(_, entry)| entry.created)
                    .map(|(id, _)| id.clone())
                else {
                    break;
                };
                if let Some(entry) = entries.remove(&oldest) {
                    entry.stop.cancel();
                }
            }
            entries.insert(
                id.clone(),
                Entry {
                    stop: stop.clone(),
                    created: Instant::now(),
                },
            );
        }

        let served = Served {
            app: state,
            scope: Arc::new(scope),
            token: Arc::new(token.clone()),
        };
        let router = Router::new().fallback(serve).with_state(served);
        tokio::spawn(async move {
            let _ = axum::serve(listener, router)
                .with_graceful_shutdown(async move { stop.cancelled().await })
                .await;
        });
        Ok(Opened { id, port, token })
    }

    /// Stops a preview's listener. Closing one that is gone is not an error.
    pub(crate) fn close(&self, id: &str) {
        let removed = self
            .entries
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .remove(id);
        if let Some(entry) = removed {
            entry.stop.cancel();
        }
    }

    #[cfg(test)]
    pub(crate) fn open_count(&self) -> usize {
        self.entries
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .len()
    }
}

#[derive(Clone)]
struct Served {
    app: AppState,
    scope: Arc<Scope>,
    token: Arc<String>,
}

fn random_token() -> Result<String, String> {
    let mut bytes = [0u8; 24];
    getrandom::fill(&mut bytes).map_err(|error| error.to_string())?;
    Ok(bytes.iter().map(|byte| format!("{byte:02x}")).collect())
}

/// A request path as the relative path it names, or `None` if it tries to
/// leave the tree. A trailing slash names the directory's `index.html`.
fn normalize(raw: &str) -> Option<String> {
    let decoded = percent_encoding::percent_decode_str(raw)
        .decode_utf8()
        .ok()?;
    if decoded.contains(['\0', '\\']) {
        return None;
    }
    let mut parts: Vec<&str> = Vec::new();
    for part in decoded.split('/') {
        match part {
            "" => {}
            "." | ".." => return None,
            part => parts.push(part),
        }
    }
    let directory = decoded.is_empty() || decoded.ends_with('/');
    if directory {
        parts.push("index.html");
    }
    Some(parts.join("/"))
}

/// Whether a workspace request stays below the scope's directory and away from
/// dotfiles (`.env`, `.git`, ...), which a page has no reason to fetch.
fn workspace_allows(directory: &str, relative: &str) -> bool {
    let inside = if directory.is_empty() {
        relative
    } else {
        match relative.strip_prefix(directory) {
            Some(rest) if rest.starts_with('/') => rest.trim_start_matches('/'),
            _ => return false,
        }
    };
    !inside.split('/').any(|part| part.starts_with('.'))
}

async fn read_scoped(
    state: &AppState,
    scope: &Scope,
    relative: &str,
) -> Result<Vec<u8>, StatusCode> {
    match scope {
        Scope::Candidate {
            session_id,
            candidate_id,
        } => crate::sandbox_candidate_file_bytes(state, session_id, candidate_id, relative).await,
        Scope::Execution {
            session_id,
            execution_id,
        } => {
            let path = crate::execution_artifact_path(state, session_id, execution_id, relative)?;
            read_file(&path).await
        }
        Scope::Workspace { root, directory } => {
            if !workspace_allows(directory, relative) {
                return Err(StatusCode::NOT_FOUND);
            }
            let path = crate::confined_path(root, relative).ok_or(StatusCode::NOT_FOUND)?;
            read_file(&path).await
        }
    }
}

async fn read_file(path: &std::path::Path) -> Result<Vec<u8>, StatusCode> {
    match tokio::fs::metadata(path).await {
        Ok(meta) if meta.is_file() => tokio::fs::read(path)
            .await
            .map_err(|_| StatusCode::NOT_FOUND),
        _ => Err(StatusCode::NOT_FOUND),
    }
}

/// What a page may do. It can load its own files and run its own script; it
/// cannot reach any other origin, so a preview stays offline as it is in the
/// Canvas's single-page frame. Only the app's own origins may embed it.
const POLICY: &str = "default-src 'self' data: blob:; \
script-src 'self' 'unsafe-inline' 'unsafe-eval' blob:; \
style-src 'self' 'unsafe-inline'; \
img-src 'self' data: blob:; \
font-src 'self' data:; \
media-src 'self' data: blob:; \
connect-src 'self'; \
frame-src 'self'; \
object-src 'none'; \
base-uri 'self'; \
form-action 'self'; \
frame-ancestors http://127.0.0.1:* http://localhost:* tauri: http://tauri.localhost https://tauri.localhost";

async fn serve(
    State(served): State<Served>,
    method: Method,
    headers: HeaderMap,
    uri: Uri,
) -> Response {
    // A page that resolves its own name to this address has not become this
    // server: only loopback names are answered (DNS rebinding).
    let host = headers
        .get(header::HOST)
        .and_then(|value| value.to_str().ok());
    if !crate::host_is_loopback(host) {
        return StatusCode::MISDIRECTED_REQUEST.into_response();
    }
    if method != Method::GET && method != Method::HEAD {
        return StatusCode::METHOD_NOT_ALLOWED.into_response();
    }
    let path = uri.path().strip_prefix('/').unwrap_or(uri.path());
    let (token, rest) = path.split_once('/').unwrap_or((path, ""));
    // A wrong or missing token is indistinguishable from a missing file.
    if !bool::from(token.as_bytes().ct_eq(served.token.as_bytes())) {
        return StatusCode::NOT_FOUND.into_response();
    }
    let Some(relative) = normalize(&format!("/{rest}")) else {
        return StatusCode::NOT_FOUND.into_response();
    };
    let bytes = match read_scoped(&served.app, &served.scope, &relative).await {
        Ok(bytes) => bytes,
        Err(status) => return status.into_response(),
    };
    let mime = crate::raw_mime_for(std::path::Path::new(&relative));
    let mut response = Response::builder()
        .status(StatusCode::OK)
        .header(header::CONTENT_TYPE, mime)
        .header(header::CONTENT_LENGTH, bytes.len())
        .header(header::X_CONTENT_TYPE_OPTIONS, "nosniff")
        .header(header::CACHE_CONTROL, "no-store")
        .header(header::REFERRER_POLICY, "no-referrer")
        .header(header::CONTENT_SECURITY_POLICY, POLICY);
    if !mime.starts_with("text/html") {
        // Only a document is framed; nothing else needs the embedding rule.
        response = response.header("Cross-Origin-Resource-Policy", "same-origin");
    }
    let body = if method == Method::HEAD {
        Body::empty()
    } else {
        Body::from(bytes)
    };
    response
        .body(body)
        .unwrap_or_else(|_| StatusCode::INTERNAL_SERVER_ERROR.into_response())
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;

    #[test]
    fn a_request_path_names_a_file_inside_the_tree_or_nothing() {
        assert_eq!(normalize("/site/app.js").as_deref(), Some("site/app.js"));
        assert_eq!(normalize("/site/").as_deref(), Some("site/index.html"));
        assert_eq!(normalize("/").as_deref(), Some("index.html"));
        assert_eq!(normalize("").as_deref(), Some("index.html"));
        assert_eq!(normalize("/a%20b/c.css").as_deref(), Some("a b/c.css"));
        for hostile in [
            "/../secret",
            "/a/../../secret",
            "/a/%2e%2e/secret",
            "/a/./b",
            "/a%5Cb",
            "/a%00b",
            "/%ff",
        ] {
            assert_eq!(normalize(hostile), None, "{hostile}");
        }
    }

    #[test]
    fn a_workspace_preview_stays_below_its_directory_and_away_from_dotfiles() {
        assert!(workspace_allows("site", "site/app.js"));
        assert!(workspace_allows("site", "site/css/app.css"));
        assert!(!workspace_allows("site", "other/app.js"));
        assert!(!workspace_allows("site", "sitemap/app.js"));
        assert!(!workspace_allows("site", "site"));
        assert!(!workspace_allows("site", "site/.env"));
        assert!(!workspace_allows("site", "site/a/.git/config"));
        assert!(workspace_allows("", "index.html"));
        assert!(!workspace_allows("", ".env"));
    }

    fn state_in(dir: &std::path::Path) -> AppState {
        vak_config::paths::isolate_home_for_tests();
        let core = vak_core::Core::new(dir.to_path_buf()).unwrap();
        core.set_sessions_home(dir.join("home"));
        AppState::new(core)
    }

    async fn get(url: &str, host: Option<&str>) -> reqwest::Response {
        let mut request = reqwest::Client::new().get(url);
        if let Some(host) = host {
            request = request.header("Host", host);
        }
        request.send().await.unwrap()
    }

    /// A raw request line, so a path reaches the server exactly as written:
    /// an HTTP client would resolve `..` before sending it.
    async fn raw_status(port: u16, target: &str) -> String {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        let mut stream = tokio::net::TcpStream::connect(("127.0.0.1", port))
            .await
            .unwrap();
        stream
            .write_all(
                format!("GET {target} HTTP/1.1\r\nHost: localhost\r\nConnection: close\r\n\r\n")
                    .as_bytes(),
            )
            .await
            .unwrap();
        let mut reply = String::new();
        stream.read_to_string(&mut reply).await.unwrap();
        reply.lines().next().unwrap_or_default().to_string()
    }

    #[tokio::test]
    async fn a_workspace_preview_serves_its_files_on_an_origin_of_its_own() {
        let dir = tempfile::tempdir().unwrap();
        let site = dir.path().join("site");
        std::fs::create_dir_all(site.join("css")).unwrap();
        std::fs::write(
            site.join("index.html"),
            "<link rel=stylesheet href=css/app.css><p>home</p>",
        )
        .unwrap();
        std::fs::write(site.join("about.html"), "<p>about</p>").unwrap();
        std::fs::write(site.join("css/app.css"), "p { color: red }").unwrap();
        std::fs::write(site.join(".env"), "SECRET=1").unwrap();
        std::fs::write(dir.path().join("outside.txt"), "not part of the site").unwrap();
        let state = state_in(dir.path());
        let scope = Scope::Workspace {
            root: dir.path().to_path_buf(),
            directory: "site".into(),
        };
        let opened = state
            .previews
            .open(state.clone(), scope, "site/index.html")
            .await
            .unwrap();
        let base = format!("http://127.0.0.1:{}/{}", opened.port, opened.token);

        let page = get(&format!("{base}/site/index.html"), None).await;
        assert_eq!(page.status(), 200);
        assert_eq!(page.headers()["content-type"], "text/html; charset=utf-8");
        let policy = page.headers()["content-security-policy"]
            .to_str()
            .unwrap()
            .to_string();
        assert!(policy.contains("connect-src 'self'"), "{policy}");
        assert!(policy.contains("frame-ancestors"), "{policy}");
        assert_eq!(page.headers()["x-content-type-options"], "nosniff");
        assert!(page.text().await.unwrap().contains("home"));

        // The page's own relative files and pages load: it is a site, not a srcdoc.
        let style = get(&format!("{base}/site/css/app.css"), None).await;
        assert_eq!(style.headers()["content-type"], "text/css; charset=utf-8");
        assert_eq!(
            get(&format!("{base}/site/about.html"), None).await.status(),
            200
        );
        // A directory is its index page.
        assert_eq!(get(&format!("{base}/site/"), None).await.status(), 200);

        // Nothing outside the page's directory, and no dotfiles.
        assert_eq!(
            get(&format!("{base}/outside.txt"), None).await.status(),
            404
        );
        assert_eq!(get(&format!("{base}/site/.env"), None).await.status(), 404);
        assert_eq!(
            get(&format!("{base}/site/missing.html"), None)
                .await
                .status(),
            404
        );
        for hostile in [
            "/../outside.txt",
            "/site/../outside.txt",
            "/site/%2e%2e/outside.txt",
            "/site/a%5cb",
        ] {
            let status = raw_status(opened.port, &format!("/{}{hostile}", opened.token)).await;
            assert!(status.contains("404"), "{hostile}: {status}");
        }

        // Without the token there is nothing here, whatever the path.
        assert_eq!(
            get(
                &format!("http://127.0.0.1:{}/site/index.html", opened.port),
                None
            )
            .await
            .status(),
            404
        );
        assert_eq!(
            get(
                &format!(
                    "http://127.0.0.1:{}/{}x/site/index.html",
                    opened.port, opened.token
                ),
                None
            )
            .await
            .status(),
            404
        );

        // A page that resolved its own name to this address is refused.
        assert_eq!(
            get(&format!("{base}/site/index.html"), Some("attacker.example"))
                .await
                .status(),
            421
        );
        // Only reading is offered.
        let posted = reqwest::Client::new()
            .post(format!("{base}/site/index.html"))
            .send()
            .await
            .unwrap();
        assert_eq!(posted.status(), 405);

        // Closing ends the origin.
        state.previews.close(&opened.id);
        tokio::time::sleep(Duration::from_millis(100)).await;
        assert!(
            reqwest::Client::new()
                .get(format!("{base}/site/index.html"))
                .send()
                .await
                .is_err()
        );
        assert_eq!(state.previews.open_count(), 0);
    }

    #[tokio::test]
    async fn opening_a_preview_needs_a_readable_entry_and_each_has_its_own_origin() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("a.html"), "<p>a</p>").unwrap();
        std::fs::write(dir.path().join("b.html"), "<p>b</p>").unwrap();
        let state = state_in(dir.path());
        let whole = || Scope::Workspace {
            root: dir.path().to_path_buf(),
            directory: String::new(),
        };
        assert_eq!(
            state
                .previews
                .open(state.clone(), whole(), "gone.html")
                .await
                .err(),
            Some(OpenError::NotFound)
        );
        assert_eq!(
            state
                .previews
                .open(state.clone(), whole(), "../a.html")
                .await
                .err(),
            Some(OpenError::NotFound)
        );
        let a = state
            .previews
            .open(state.clone(), whole(), "a.html")
            .await
            .unwrap();
        let b = state
            .previews
            .open(state.clone(), whole(), "b.html")
            .await
            .unwrap();
        assert_ne!(a.port, b.port, "two previews are two origins");
        assert_ne!(a.token, b.token);
        // One preview's token opens nothing on another's origin.
        assert_eq!(
            get(
                &format!("http://127.0.0.1:{}/{}/a.html", b.port, a.token),
                None
            )
            .await
            .status(),
            404
        );

        // Too many closes the oldest to make room.
        let mut last = None;
        for _ in 0..MAX_PREVIEWS + 2 {
            last = Some(
                state
                    .previews
                    .open(state.clone(), whole(), "a.html")
                    .await
                    .unwrap(),
            );
        }
        assert_eq!(state.previews.open_count(), MAX_PREVIEWS);
        tokio::time::sleep(Duration::from_millis(100)).await;
        assert!(
            reqwest::Client::new()
                .get(format!("http://127.0.0.1:{}/{}/a.html", a.port, a.token))
                .send()
                .await
                .is_err(),
            "the oldest preview was closed"
        );
        let last = last.unwrap();
        assert_eq!(
            get(
                &format!("http://127.0.0.1:{}/{}/a.html", last.port, last.token),
                None
            )
            .await
            .status(),
            200
        );
    }

    /// A page reads its own folder and nothing else: a file it names that is
    /// not there is missing, even when a run's scratch folder or another
    /// Agent's folder has one by that name.
    #[tokio::test]
    async fn a_workspace_preview_never_serves_a_file_found_somewhere_else() {
        let dir = tempfile::tempdir().unwrap();
        let agent = dir.path().join(".vak/agents/helper/workspace");
        std::fs::create_dir_all(agent.join("site")).unwrap();
        std::fs::write(
            agent.join("site/index.html"),
            "<script src=app.js></script>",
        )
        .unwrap();
        let scratch = dir.path().join(".vak/scratch/vak/call-1/site");
        std::fs::create_dir_all(&scratch).unwrap();
        std::fs::write(scratch.join("app.js"), "secret()").unwrap();
        std::fs::write(dir.path().join("site.js"), "not this agent's").unwrap();
        let state = state_in(dir.path());
        let opened = state
            .previews
            .open(
                state.clone(),
                Scope::Workspace {
                    root: agent.clone(),
                    directory: "site".into(),
                },
                "site/index.html",
            )
            .await
            .unwrap();
        let base = format!("http://127.0.0.1:{}/{}", opened.port, opened.token);
        assert_eq!(
            get(&format!("{base}/site/index.html"), None).await.status(),
            200
        );
        assert_eq!(
            get(&format!("{base}/site/app.js"), None).await.status(),
            404
        );
        assert_eq!(
            state
                .previews
                .open(
                    state.clone(),
                    Scope::Workspace {
                        root: agent,
                        directory: String::new(),
                    },
                    "site.js",
                )
                .await
                .err(),
            Some(OpenError::NotFound),
            "the server's own workspace is not the Agent's folder"
        );
    }

    async fn create(
        state: &AppState,
        host: &str,
        body: serde_json::Value,
    ) -> (StatusCode, serde_json::Value) {
        let mut headers = HeaderMap::new();
        headers.insert(header::HOST, host.parse().unwrap());
        let response = crate::create_preview(
            State(state.clone()),
            headers,
            axum::Json(serde_json::from_value(body).unwrap()),
        )
        .await;
        let status = response.status();
        let bytes = axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .unwrap();
        (status, serde_json::from_slice(&bytes).unwrap_or_default())
    }

    #[tokio::test]
    async fn the_app_is_told_where_to_frame_a_preview_and_only_when_it_is_local() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(dir.path().join("site")).unwrap();
        std::fs::write(dir.path().join("site/my page.html"), "<p>hi</p>").unwrap();
        let state = state_in(dir.path());
        let body = serde_json::json!({ "kind": "workspace", "path": "./site/my page.html" });

        // The page is framed from the other loopback name, so it shares no cookies with the app.
        let (status, reply) = create(&state, "127.0.0.1:8901", body.clone()).await;
        assert_eq!(status, 200, "{reply}");
        let url = reply["url"].as_str().unwrap();
        assert!(url.starts_with("http://localhost:"), "{url}");
        assert!(url.ends_with("/site/my%20page.html"), "{url}");
        assert_eq!(get(url, None).await.status(), 200);
        let (_, from_localhost) = create(&state, "localhost:8901", body.clone()).await;
        assert!(
            from_localhost["url"]
                .as_str()
                .unwrap()
                .starts_with("http://127.0.0.1:")
        );

        // Closing it by id ends its origin.
        state.previews.close(reply["id"].as_str().unwrap());
        tokio::time::sleep(Duration::from_millis(100)).await;
        assert!(reqwest::Client::new().get(url).send().await.is_err());

        // A browser that reached the server by another name cannot reach a loopback port.
        let (status, reply) = create(&state, "vak.example.com", body).await;
        assert_eq!(status, 409);
        assert_eq!(reply["reason"], "not_local");
        let (status, _) = create(
            &state,
            "127.0.0.1",
            serde_json::json!({ "kind": "workspace", "path": "site/gone.html" }),
        )
        .await;
        assert_eq!(status, 404);
    }
}
