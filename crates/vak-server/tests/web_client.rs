//! The browser surface, end to end (docs/design/48-web-client.md).
//!
//! These are the boundaries that decide whether serving a workspace over a
//! network is safe, so they are tested against the real secured router
//! rather than a handler in isolation — the middleware IS the security
//! property here, and a handler test would prove nothing about it.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::net::SocketAddr;

use vak_core::Core;

/// A real bound server with the full secured stack.
async fn spawn() -> (SocketAddr, String) {
    let dir = tempfile::tempdir().unwrap();
    vak_config::paths::isolate_home_for_tests();
    let core = Core::new(dir.path().to_path_buf()).unwrap();
    core.set_shared_scope(vak_config::scope::SharedScope::new(dir.path().join("home")));
    // Outlives the test body; a removed directory would fail reads midway.
    std::mem::forget(dir);

    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let (app, token) = vak_server::secured_router_with_port(core, false, addr.port());
    tokio::spawn(async move {
        axum::serve(
            listener,
            app.into_make_service_with_connect_info::<SocketAddr>(),
        )
        .await
        .unwrap();
    });
    (addr, token)
}

/// Log in and return the raw `Set-Cookie` value.
///
/// The cookie is carried by hand rather than by a cookie jar, which makes
/// the round trip the test is actually about — what the server sets, and
/// what it accepts back — visible in the test instead of hidden in a
/// client feature.
async fn login(client: &reqwest::Client, addr: SocketAddr, token: &str) -> String {
    let response = client
        .post(format!("http://{addr}/auth/login"))
        .json(&serde_json::json!({ "token": token }))
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), reqwest::StatusCode::OK);
    response
        .headers()
        .get(reqwest::header::SET_COOKIE)
        .expect("login must set a session cookie")
        .to_str()
        .unwrap()
        .to_string()
}

/// The client bundle has to load before anyone can be asked for a token —
/// the login form is part of it. So the shell is auth-exempt, and every
/// route it then calls is not.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn the_client_shell_loads_without_a_session_but_data_does_not() {
    let (addr, _token) = spawn().await;
    let client = reqwest::Client::new();

    let shell = client
        .get(format!("http://{addr}/app"))
        .send()
        .await
        .unwrap();
    assert_eq!(shell.status(), reqwest::StatusCode::OK);
    assert!(
        shell.text().await.unwrap().contains("/app/assets/"),
        "the shell must reference its own hashed assets under /app"
    );

    // Data, on the other hand, is gated.
    let sessions = client
        .get(format!("http://{addr}/sessions"))
        .send()
        .await
        .unwrap();
    assert_eq!(sessions.status(), reqwest::StatusCode::UNAUTHORIZED);
}

/// Character portraits are part of the shell, like hashed CSS and JS. They
/// must be available before authentication or Agent pickers render empty
/// silhouettes while the loopback/session exchange is still settling.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn packaged_character_assets_load_with_the_unauthenticated_shell() {
    let (addr, _token) = spawn().await;
    let response = reqwest::get(format!("http://{addr}/app/characters/mira-atlas-128.webp"))
        .await
        .unwrap();
    assert_eq!(response.status(), reqwest::StatusCode::OK);
    assert_eq!(
        response
            .headers()
            .get(reqwest::header::CONTENT_TYPE)
            .unwrap(),
        "image/webp"
    );
    assert!(!response.bytes().await.unwrap().is_empty());
    // The 1024px source atlases are brand source art, not shipped runtime:
    // the path falls through to the client shell, never to an image.
    let source = reqwest::get(format!("http://{addr}/app/characters/mira-atlas.png"))
        .await
        .unwrap();
    let kind = source
        .headers()
        .get(reqwest::header::CONTENT_TYPE)
        .and_then(|value| value.to_str().ok())
        .unwrap_or_default()
        .to_string();
    assert!(!kind.starts_with("image/"), "source atlas served as {kind}");
}

/// `/auth/session` must distinguish "no session yet" from "unreachable",
/// and on loopback it must hand over the session rather than ask for it.
///
/// A 401 conflates the first two, and the client cannot then tell whether
/// to show the login form or an error. The second half is the whole point
/// of `[server] loopback_auto_login`: anything that can reach loopback can
/// already read the token off disk, so prompting for it to reach the
/// machine you are sitting at buys nothing and cost every local user a
/// token hunt. `host_policy.rs` covers the other side — a request arriving
/// with a real hostname is refused before this handler ever runs.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn session_status_answers_rather_than_rejecting() {
    let (addr, token) = spawn().await;
    let client = reqwest::Client::new();

    let response = client
        .get(format!("http://{addr}/auth/session"))
        .send()
        .await
        .unwrap();
    // Answered, not rejected: the distinction the client needs.
    assert_eq!(response.status(), reqwest::StatusCode::OK);
    assert!(
        response
            .headers()
            .get(reqwest::header::SET_COOKIE)
            .is_some(),
        "the loopback probe must hand over the session, not merely report it"
    );
    let before: serde_json::Value = response.json().await.unwrap();
    assert_eq!(before["authenticated"], true);
    assert_eq!(before["granted"], "loopback");

    let cookie = login(&client, addr, &token).await;

    let after: serde_json::Value = client
        .get(format!("http://{addr}/auth/session"))
        .header(reqwest::header::COOKIE, &cookie)
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(after["authenticated"], true);
}

/// The cookie must actually authenticate, because `EventSource` cannot
/// send a header and the cookie is the only channel a browser stream has.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn the_login_cookie_authenticates_subsequent_requests() {
    let (addr, token) = spawn().await;
    let client = reqwest::Client::new();

    let bad = client
        .post(format!("http://{addr}/auth/login"))
        .json(&serde_json::json!({ "token": "wrong" }))
        .send()
        .await
        .unwrap();
    assert_eq!(bad.status(), reqwest::StatusCode::UNAUTHORIZED);

    let login = client
        .post(format!("http://{addr}/auth/login"))
        .json(&serde_json::json!({ "token": token }))
        .send()
        .await
        .unwrap();
    assert_eq!(login.status(), reqwest::StatusCode::OK);
    let cookie = login
        .headers()
        .get(reqwest::header::SET_COOKIE)
        .unwrap()
        .to_str()
        .unwrap()
        .to_string();
    assert!(cookie.starts_with("vak_session="));
    assert!(
        !cookie.contains(&token),
        "the durable gateway bearer must never be a cookie"
    );
    assert!(
        cookie.contains("HttpOnly"),
        "script must not be able to read it"
    );
    assert!(cookie.contains("SameSite=Strict"));
    // Plain http: a `Secure` cookie here would be silently discarded by the
    // browser and the session would never persist.
    assert!(
        !cookie.contains("Secure"),
        "Secure over plain http makes the browser drop the cookie: {cookie}"
    );

    // No bearer header anywhere — the cookie alone carries this.
    let pair = cookie.split(';').next().unwrap();
    let host = client
        .get(format!("http://{addr}/host"))
        .header(reqwest::header::COOKIE, pair)
        .send()
        .await
        .unwrap();
    assert_eq!(host.status(), reqwest::StatusCode::OK);

    let signed_out = client
        .post(format!("http://{addr}/auth/logout"))
        .header(reqwest::header::COOKIE, pair)
        .header(reqwest::header::ORIGIN, format!("http://{addr}"))
        .send()
        .await
        .unwrap();
    assert_eq!(signed_out.status(), reqwest::StatusCode::OK);
    let revoked = client
        .get(format!("http://{addr}/host"))
        .header(reqwest::header::COOKIE, pair)
        .send()
        .await
        .unwrap();
    assert_eq!(revoked.status(), reqwest::StatusCode::UNAUTHORIZED);
}

/// The old `/admin/login` was REMOVED, not kept alongside `/auth/login`.
/// Two endpoints against one cookie is two contracts that must agree
/// forever (AGENTS.md invariant 30).
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn the_superseded_admin_login_is_gone() {
    let (addr, token) = spawn().await;
    let client = reqwest::Client::new();
    // Authenticated, so this reaches the router rather than stopping at the
    // auth layer — an unauthenticated probe would 401 whether the route
    // existed or not, and would prove nothing about its absence.
    let response = client
        .post(format!("http://{addr}/admin/login"))
        .bearer_auth(&token)
        .header(reqwest::header::ORIGIN, format!("http://{addr}"))
        .json(&serde_json::json!({ "token": token }))
        .send()
        .await
        .unwrap();
    assert_eq!(
        response.status(),
        reqwest::StatusCode::NOT_FOUND,
        "a second login endpoint must not survive the one that replaced it"
    );
}

/// A cookie is attached by the browser to whoever asks, so a mutation
/// carrying one must ALSO prove it came from a page we serve.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_cross_origin_mutation_is_refused() {
    let (addr, token) = spawn().await;
    let client = reqwest::Client::new();
    let cookie = login(&client, addr, &token).await;
    let pair = cookie.split(';').next().unwrap().to_string();

    let evil = client
        .post(format!("http://{addr}/sessions"))
        .header(reqwest::header::COOKIE, &pair)
        .header(reqwest::header::ORIGIN, "https://attacker.example")
        .send()
        .await
        .unwrap();
    assert_eq!(evil.status(), reqwest::StatusCode::FORBIDDEN);

    // Our own origin is fine.
    let ours = client
        .post(format!("http://{addr}/sessions"))
        .header(reqwest::header::COOKIE, &pair)
        .header(reqwest::header::ORIGIN, format!("http://{addr}"))
        .send()
        .await
        .unwrap();
    assert_eq!(ours.status(), reqwest::StatusCode::OK);
}

/// Originless CLI mutations need a bearer. A cookie without Origin is
/// rejected before dispatch.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn an_originless_mutation_still_needs_a_token() {
    let (addr, token) = spawn().await;
    let client = reqwest::Client::new();

    let anonymous = client
        .post(format!("http://{addr}/sessions"))
        .send()
        .await
        .unwrap();
    assert_eq!(anonymous.status(), reqwest::StatusCode::UNAUTHORIZED);

    let cookie = login(&client, addr, &token).await;
    let cookie_only = client
        .post(format!("http://{addr}/sessions"))
        .header(reqwest::header::COOKIE, cookie.split(';').next().unwrap())
        .send()
        .await
        .unwrap();
    assert_eq!(cookie_only.status(), reqwest::StatusCode::FORBIDDEN);

    let wrong_port = client
        .post(format!("http://{addr}/sessions"))
        .header(reqwest::header::COOKIE, cookie.split(';').next().unwrap())
        .header(reqwest::header::ORIGIN, "http://127.0.0.1:1")
        .send()
        .await
        .unwrap();
    assert_eq!(wrong_port.status(), reqwest::StatusCode::FORBIDDEN);

    let authorized = client
        .post(format!("http://{addr}/sessions"))
        .bearer_auth(&token)
        .send()
        .await
        .unwrap();
    assert_eq!(authorized.status(), reqwest::StatusCode::OK);
}

/// Binding loopback does not stop a page from resolving its own domain to
/// 127.0.0.1 and becoming same-origin with this server. Pinning the `Host`
/// name is the check that does, and `trusted_hosts` is empty by default.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn an_untrusted_host_header_is_refused() {
    let (addr, token) = spawn().await;
    let client = reqwest::Client::new();
    let rebound = client
        .get(format!("http://{addr}/health"))
        .header(reqwest::header::HOST, "rebind.attacker.example")
        .bearer_auth(&token)
        .send()
        .await
        .unwrap();
    assert_eq!(rebound.status(), reqwest::StatusCode::MISDIRECTED_REQUEST);
}

/// A token in a query string can reach access logs, `Referer` headers, and
/// history. That is acceptable for an in-process loopback client with no
/// proxy in between, and not acceptable anywhere else — so the channel
/// exists only for loopback. (Here the request IS loopback, which is the
/// case that must keep working: the desktop's SSE depends on it.)
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn the_query_token_channel_works_on_loopback() {
    let (addr, token) = spawn().await;
    let client = reqwest::Client::new();
    let response = client
        .get(format!("http://{addr}/host?token={token}"))
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), reqwest::StatusCode::OK);
}

/// The picker answers "which folders are there", and nothing more. It must
/// not become a way to read the filesystem, nor to walk out of the roots.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn the_directory_browser_stays_inside_its_roots() {
    let (addr, token) = spawn().await;
    let client = reqwest::Client::new();

    let escaped = client
        .get(format!("http://{addr}/fs/dirs?path=/etc"))
        .bearer_auth(&token)
        .send()
        .await
        .unwrap();
    assert_eq!(
        escaped.status(),
        reqwest::StatusCode::FORBIDDEN,
        "the picker must not be walkable outside [server] workspace_roots"
    );
}

/// The terminal is a real shell. It is off unless an operator turned it on,
/// and the refusal says which setting turns it on rather than 404ing.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn the_terminal_is_refused_until_it_is_enabled() {
    let (addr, token) = spawn().await;
    let client = reqwest::Client::new();
    // Sent as a real upgrade, because that is how the only client that
    // reaches this route asks. A plain GET is rejected earlier, by the
    // upgrade extractor, and would never exercise the config gate.
    let response = client
        .get(format!("http://{addr}/pty"))
        .bearer_auth(&token)
        .header(reqwest::header::CONNECTION, "Upgrade")
        .header(reqwest::header::UPGRADE, "websocket")
        .header(reqwest::header::SEC_WEBSOCKET_VERSION, "13")
        .header(
            reqwest::header::SEC_WEBSOCKET_KEY,
            "dGhlIHNhbXBsZSBub25jZQ==",
        )
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), reqwest::StatusCode::FORBIDDEN);
    let body: serde_json::Value = response.json().await.unwrap();
    assert!(
        body["error"]
            .as_str()
            .unwrap_or_default()
            .contains("terminal"),
        "the refusal must name what is disabled: {body}"
    );
}

/// `/host` stands in for the desktop shell's `backend_info`. It must NOT
/// carry a base URL or token: their absence is precisely how the client
/// knows it is same-origin and cookie-authenticated.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn the_host_descriptor_hands_out_no_credentials() {
    let (addr, token) = spawn().await;
    let client = reqwest::Client::new();
    let body: serde_json::Value = client
        .get(format!("http://{addr}/host"))
        .bearer_auth(&token)
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(body["ready"], true);
    assert!(body["cwd"].is_string());
    assert!(body.get("token").is_none(), "a token must never be served");
    assert!(body.get("base_url").is_none());
    assert_eq!(body["terminal"], false);
}

/// The static bundles are compressed; the API is not.
///
/// `/sessions/:id/events` is server-sent events. A compressor sits between
/// the writer and the socket, so a live transcript would arrive in
/// buffer-sized batches rather than per frame — a laggy agent in exchange
/// for a smaller JSON body nobody was waiting on. The layer is therefore
/// scoped to the three static routers rather than applied at the root, and
/// this is the test that keeps it there.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn compression_covers_the_static_bundles_and_not_the_api() {
    let (addr, _token) = spawn().await;
    let client = reqwest::Client::builder()
        // Ask for gzip without letting reqwest transparently strip the
        // header we are asserting on.
        .no_gzip()
        .build()
        .unwrap();

    let page = client
        .get(format!("http://{addr}/"))
        .header(reqwest::header::ACCEPT_ENCODING, "gzip")
        .send()
        .await
        .unwrap();
    assert_eq!(
        page.headers()
            .get(reqwest::header::CONTENT_ENCODING)
            .map(|v| v.to_str().unwrap()),
        Some("gzip"),
        "the front door is ~78 KB of inlined CSS and markup and must compress"
    );

    for path in ["/version", "/health"] {
        let api = client
            .get(format!("http://{addr}{path}"))
            .header(reqwest::header::ACCEPT_ENCODING, "gzip")
            .send()
            .await
            .unwrap();
        assert!(
            api.headers()
                .get(reqwest::header::CONTENT_ENCODING)
                .is_none(),
            "{path} must not be compressed — the layer belongs to the static bundles only, \
             because the same layer at the root would buffer server-sent events"
        );
    }
}
