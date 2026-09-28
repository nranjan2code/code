//! Exercises the real HTTP boundary with a software WebAuthn authenticator.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::net::SocketAddr;
use webauthn_authenticator_rs::prelude::{Url, WebauthnAuthenticator};
use webauthn_authenticator_rs::softpasskey::SoftPasskey;
use webauthn_rs::prelude::{CreationChallengeResponse, RequestChallengeResponse};

async fn spawn() -> (SocketAddr, String) {
    let dir = tempfile::tempdir().unwrap();
    vak_config::paths::isolate_home_for_tests();
    let core = vak_core::Core::new(dir.path().to_path_buf()).unwrap();
    core.set_sessions_home(dir.path().join("home"));
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

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn owner_enrolls_signs_in_and_spends_one_recovery_code() {
    let (addr, token) = spawn().await;
    let base = format!("http://localhost:{}", addr.port());
    let client = reqwest::Client::new();
    let origin = Url::parse(&base).unwrap();
    let mut authenticator = WebauthnAuthenticator::new(SoftPasskey::new(true));

    let method: serde_json::Value = client
        .get(format!("{base}/auth/methods"))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(method["method"], "bootstrap");

    let bootstrap_login = client
        .post(format!("{base}/auth/login"))
        .json(&serde_json::json!({"token": token}))
        .send()
        .await
        .unwrap();
    assert_eq!(bootstrap_login.status(), reqwest::StatusCode::OK);
    let old_cookie = bootstrap_login
        .headers()
        .get(reqwest::header::SET_COOKIE)
        .unwrap()
        .to_str()
        .unwrap()
        .split(';')
        .next()
        .unwrap()
        .to_string();

    let start: serde_json::Value = client
        .post(format!("{base}/auth/enroll/start"))
        .json(&serde_json::json!({"token": token}))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert!(start["challenge_id"].is_string(), "{start}");
    let challenge: CreationChallengeResponse =
        serde_json::from_value(serde_json::json!({"publicKey": start["options"]})).unwrap();
    let credential = authenticator
        .do_registration(origin.clone(), challenge)
        .unwrap();
    let enrolled = client
        .post(format!("{base}/auth/enroll/finish"))
        .json(&serde_json::json!({"challenge_id": start["challenge_id"], "credential": credential}))
        .send()
        .await
        .unwrap();
    assert_eq!(enrolled.status(), reqwest::StatusCode::OK);
    let codes: serde_json::Value = enrolled.json().await.unwrap();
    assert_eq!(codes["recovery_codes"].as_array().unwrap().len(), 10);
    let old_session = client
        .get(format!("{base}/host"))
        .header(reqwest::header::COOKIE, old_cookie)
        .send()
        .await
        .unwrap();
    assert_eq!(old_session.status(), reqwest::StatusCode::UNAUTHORIZED);

    let method: serde_json::Value = client
        .get(format!("{base}/auth/methods"))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(method["method"], "passkey");
    let legacy = client
        .post(format!("{base}/auth/login"))
        .json(&serde_json::json!({"token": token}))
        .send()
        .await
        .unwrap();
    assert_eq!(legacy.status(), reqwest::StatusCode::GONE);

    let start: serde_json::Value = client
        .post(format!("{base}/auth/passkey/start"))
        .json(&serde_json::json!({}))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert!(start["challenge_id"].is_string(), "{start}");
    let challenge: RequestChallengeResponse =
        serde_json::from_value(serde_json::json!({"publicKey": start["options"]})).unwrap();
    let assertion = authenticator.do_authentication(origin, challenge).unwrap();
    let signed_in = client
        .post(format!("{base}/auth/passkey/finish"))
        .json(&serde_json::json!({"challenge_id": start["challenge_id"], "credential": assertion}))
        .send()
        .await
        .unwrap();
    assert_eq!(signed_in.status(), reqwest::StatusCode::OK);
    let owner_cookie = signed_in
        .headers()
        .get(reqwest::header::SET_COOKIE)
        .unwrap()
        .to_str()
        .unwrap()
        .split(';')
        .next()
        .unwrap()
        .to_string();
    assert!(
        signed_in
            .headers()
            .contains_key(reqwest::header::SET_COOKIE)
    );

    let account: serde_json::Value = client
        .get(format!("{base}/auth/account"))
        .header(reqwest::header::COOKIE, &owner_cookie)
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(account["passkeys"], 1);
    let rotate_start: serde_json::Value = client
        .post(format!("{base}/auth/recovery/rotate/start"))
        .header(reqwest::header::COOKIE, &owner_cookie)
        .header(reqwest::header::ORIGIN, &base)
        .json(&serde_json::json!({}))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    let challenge: RequestChallengeResponse =
        serde_json::from_value(serde_json::json!({"publicKey": rotate_start["options"]})).unwrap();
    let assertion = authenticator
        .do_authentication(Url::parse(&base).unwrap(), challenge)
        .unwrap();
    let rotated: serde_json::Value = client
        .post(format!("{base}/auth/recovery/rotate/finish"))
        .header(reqwest::header::COOKIE, &owner_cookie)
        .header(reqwest::header::ORIGIN, &base)
        .json(&serde_json::json!({"challenge_id": rotate_start["challenge_id"], "credential": assertion}))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(rotated["recovery_codes"].as_array().unwrap().len(), 10);
    let old_code = codes["recovery_codes"][0].as_str().unwrap();
    let obsolete = client
        .post(format!("{base}/auth/recovery"))
        .json(&serde_json::json!({"code": old_code}))
        .send()
        .await
        .unwrap();
    assert_eq!(obsolete.status(), reqwest::StatusCode::UNAUTHORIZED);
    let revoke = client
        .post(format!("{base}/auth/sessions/revoke-all"))
        .header(reqwest::header::COOKIE, &owner_cookie)
        .header(reqwest::header::ORIGIN, &base)
        .json(&serde_json::json!({}))
        .send()
        .await
        .unwrap();
    assert_eq!(revoke.status(), reqwest::StatusCode::OK);
    let old_session = client
        .get(format!("{base}/host"))
        .header(reqwest::header::COOKIE, &owner_cookie)
        .send()
        .await
        .unwrap();
    assert_eq!(old_session.status(), reqwest::StatusCode::UNAUTHORIZED);

    let code = rotated["recovery_codes"][0].as_str().unwrap();
    let recovered = client
        .post(format!("{base}/auth/recovery"))
        .json(&serde_json::json!({"code": code}))
        .send()
        .await
        .unwrap();
    assert_eq!(recovered.status(), reqwest::StatusCode::OK);
    let replay = client
        .post(format!("{base}/auth/recovery"))
        .json(&serde_json::json!({"code": code}))
        .send()
        .await
        .unwrap();
    assert_eq!(replay.status(), reqwest::StatusCode::UNAUTHORIZED);
}
