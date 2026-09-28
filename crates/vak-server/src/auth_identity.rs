//! Portable owner identity for the headless browser surfaces (design 78).
//! WebAuthn challenge state never leaves this process; the durable record
//! contains public-key credentials and one-way recovery-code digests only.

use std::collections::HashMap;
use std::fs::{self, File, OpenOptions};
use std::io::{self, Write};
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use axum::Json;
use axum::extract::State;
use axum::http::{HeaderMap, StatusCode, header};
use axum::response::{IntoResponse, Response};
use fs2::FileExt;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use subtle::ConstantTimeEq;
use webauthn_rs::prelude::{
    Passkey, PasskeyAuthentication, PasskeyRegistration, PublicKeyCredential,
    RegisterPublicKeyCredential, Uuid, Webauthn, WebauthnBuilder,
};

use crate::{AppState, web};

const CHALLENGE_TTL: Duration = Duration::from_secs(300);
const MAX_PENDING: usize = 64;

#[derive(Clone)]
pub(crate) struct OwnerAuth {
    root: PathBuf,
    pending: Arc<Mutex<HashMap<String, Pending>>>,
}

enum Pending {
    Registration {
        state: PasskeyRegistration,
        owner_id: Uuid,
        add: bool,
        expires: Instant,
    },
    Authentication {
        state: PasskeyAuthentication,
        rotate_recovery: bool,
        expires: Instant,
    },
}

#[derive(Serialize, Deserialize)]
struct OwnerRecord {
    version: u8,
    id: Uuid,
    passkeys: Vec<Passkey>,
    recovery_hashes: Vec<String>,
}

impl OwnerAuth {
    pub(crate) fn new(data_home: PathBuf) -> Self {
        Self {
            root: data_home.join("auth"),
            pending: Arc::new(Mutex::new(HashMap::new())),
        }
    }

    fn lock(&self) -> io::Result<File> {
        fs::create_dir_all(&self.root)?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            fs::set_permissions(&self.root, fs::Permissions::from_mode(0o700))?;
        }
        let lock = OpenOptions::new()
            .create(true)
            .read(true)
            .write(true)
            .open(self.root.join("owner.lock"))?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            lock.set_permissions(fs::Permissions::from_mode(0o600))?;
        }
        lock.lock_exclusive()?;
        Ok(lock)
    }

    fn read(&self) -> io::Result<Option<OwnerRecord>> {
        let path = self.root.join("owner.json");
        match fs::read(path) {
            Ok(bytes) => {
                let owner: OwnerRecord = serde_json::from_slice(&bytes)
                    .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))?;
                if owner.version != 1 {
                    return Err(io::Error::new(
                        io::ErrorKind::InvalidData,
                        "unsupported owner record version",
                    ));
                }
                Ok(Some(owner))
            }
            Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(None),
            Err(e) => Err(e),
        }
    }

    fn write(&self, owner: &OwnerRecord) -> io::Result<()> {
        let bytes = serde_json::to_vec(owner).map_err(io::Error::other)?;
        let temp = self.root.join(format!(".owner-{}.tmp", Uuid::now_v7()));
        let mut options = OpenOptions::new();
        options.create_new(true).write(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(0o600);
        }
        let mut file = options.open(&temp)?;
        let result = (|| {
            file.write_all(&bytes)?;
            file.sync_all()?;
            fs::rename(&temp, self.root.join("owner.json"))?;
            File::open(&self.root)?.sync_all()
        })();
        if result.is_err() {
            let _ = fs::remove_file(&temp);
        }
        result
    }

    pub(crate) fn enrolled(&self) -> io::Result<bool> {
        let _lock = self.lock()?;
        Ok(self.read()?.is_some())
    }

    /// Hold through the bootstrap login exchange so first enrollment cannot
    /// race an old bearer into a new browser session.
    pub(crate) fn bootstrap_guard(&self) -> io::Result<Option<File>> {
        let lock = self.lock()?;
        if self.read()?.is_some() {
            Ok(None)
        } else {
            Ok(Some(lock))
        }
    }

    fn pending_insert(&self, challenge_id: String, pending: Pending) {
        let mut map = self
            .pending
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        map.retain(|_, value| match value {
            Pending::Registration { expires, .. } | Pending::Authentication { expires, .. } => {
                *expires > Instant::now()
            }
        });
        if map.len() >= MAX_PENDING {
            // Bound state even when a hostile client repeatedly starts flows.
            map.clear();
        }
        map.insert(challenge_id, pending);
    }

    fn pending_take(&self, challenge_id: &str) -> Option<Pending> {
        let pending = self
            .pending
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .remove(challenge_id)?;
        let expires = match &pending {
            Pending::Registration { expires, .. } | Pending::Authentication { expires, .. } => {
                expires
            }
        };
        (*expires > Instant::now()).then_some(pending)
    }
}

fn random_hex(bytes: usize) -> io::Result<String> {
    let mut data = vec![0u8; bytes];
    getrandom::fill(&mut data).map_err(io::Error::other)?;
    Ok(data.iter().map(|byte| format!("{byte:02x}")).collect())
}

fn recovery_hash(code: &str) -> String {
    let canonical: String = code
        .chars()
        .filter(|c| c.is_ascii_hexdigit())
        .flat_map(char::to_lowercase)
        .collect();
    format!(
        "{:x}",
        Sha256::digest(format!("vakyartha-recovery-v1:{canonical}").as_bytes())
    )
}

fn webauthn(state: &AppState, headers: &HeaderMap) -> Result<Webauthn, StatusCode> {
    let cfg = state.core.config();
    let host = headers
        .get(header::HOST)
        .and_then(|v| v.to_str().ok())
        .ok_or(StatusCode::BAD_REQUEST)?;
    let origin = match cfg.server.public_url.as_deref() {
        Some(url) => url::Url::parse(url).map_err(|_| StatusCode::SERVICE_UNAVAILABLE)?,
        None if crate::host_is_loopback(Some(host)) => {
            url::Url::parse(&format!("http://{host}")).map_err(|_| StatusCode::BAD_REQUEST)?
        }
        None => return Err(StatusCode::SERVICE_UNAVAILABLE),
    };
    if origin.scheme() != "https" && !crate::host_is_loopback(origin.host_str()) {
        return Err(StatusCode::SERVICE_UNAVAILABLE);
    }
    let rp_id = origin.host_str().ok_or(StatusCode::BAD_REQUEST)?;
    WebauthnBuilder::new(rp_id, &origin)
        .map_err(|_| StatusCode::SERVICE_UNAVAILABLE)?
        .rp_name("Vakyartha")
        .build()
        .map_err(|_| StatusCode::SERVICE_UNAVAILABLE)
}

fn fail(code: StatusCode, message: &'static str) -> Response {
    (code, Json(serde_json::json!({"error": message}))).into_response()
}

fn audit(state: &AppState, label: &str, failure: bool) {
    vak_core::security_events::record(
        &state.core.shared_data_home(),
        if failure {
            vak_core::security_events::EventKind::AuthFailure
        } else {
            vak_core::security_events::EventKind::OwnerIdentity
        },
        label,
        "owner identity",
        None,
    );
}

fn session_response(state: &AppState, headers: &HeaderMap, extra: serde_json::Value) -> Response {
    let token = state.browser_sessions.issue(Duration::from_secs(
        state
            .core
            .config()
            .server
            .session_ttl_hours
            .saturating_mul(3600),
    ));
    let attributes = web::cookie_attributes(state, web::forwarded_proto(headers));
    (
        [(
            header::SET_COOKIE,
            format!("{}={token}; {attributes}", crate::admin::SESSION_COOKIE),
        )],
        Json(extra),
    )
        .into_response()
}

#[derive(Deserialize)]
pub(crate) struct EnrollStart {
    token: Option<String>,
    name: Option<String>,
}

#[derive(Deserialize)]
pub(crate) struct CeremonyFinish {
    challenge_id: String,
    credential: serde_json::Value,
}

pub(crate) async fn methods(State(state): State<AppState>) -> Response {
    match state.owner_auth.enrolled() {
        Ok(enrolled) => {
            Json(serde_json::json!({"method": if enrolled { "passkey" } else { "bootstrap" }}))
                .into_response()
        }
        Err(_) => fail(
            StatusCode::SERVICE_UNAVAILABLE,
            "identity store unavailable",
        ),
    }
}

pub(crate) async fn enroll_start(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(body): Json<EnrollStart>,
) -> Response {
    let Ok(false) = state.owner_auth.enrolled() else {
        return fail(
            StatusCode::CONFLICT,
            "owner already enrolled or identity store unavailable",
        );
    };
    let Some(token) = body.token.as_deref() else {
        audit(&state, "bootstrap_enrollment_failed", true);
        return fail(StatusCode::UNAUTHORIZED, "bootstrap credential required");
    };
    if !bool::from(token.as_bytes().ct_eq(state.auth_token.as_bytes())) {
        audit(&state, "bootstrap_enrollment_failed", true);
        return fail(StatusCode::UNAUTHORIZED, "bootstrap credential invalid");
    }
    let Ok(webauthn) = webauthn(&state, &headers) else {
        return fail(
            StatusCode::SERVICE_UNAVAILABLE,
            "public HTTPS origin must be configured",
        );
    };
    let owner_id = Uuid::now_v7();
    let name = body
        .name
        .as_deref()
        .filter(|s| !s.trim().is_empty() && s.len() <= 100)
        .unwrap_or("Owner");
    let Ok((options, registration)) =
        webauthn.start_passkey_registration(owner_id, "owner", name, None)
    else {
        return fail(
            StatusCode::SERVICE_UNAVAILABLE,
            "passkey enrollment unavailable",
        );
    };
    let Ok(challenge_id) = random_hex(24) else {
        return fail(StatusCode::SERVICE_UNAVAILABLE, "random source unavailable");
    };
    state.owner_auth.pending_insert(
        challenge_id.clone(),
        Pending::Registration {
            state: registration,
            owner_id,
            add: false,
            expires: Instant::now() + CHALLENGE_TTL,
        },
    );
    Json(serde_json::json!({"challenge_id": challenge_id, "options": options.public_key}))
        .into_response()
}

pub(crate) async fn enroll_finish(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(body): Json<CeremonyFinish>,
) -> Response {
    let Some(Pending::Registration {
        state: registration,
        owner_id,
        add: false,
        ..
    }) = state.owner_auth.pending_take(&body.challenge_id)
    else {
        return fail(StatusCode::BAD_REQUEST, "challenge expired or already used");
    };
    let Ok(webauthn) = webauthn(&state, &headers) else {
        return fail(
            StatusCode::SERVICE_UNAVAILABLE,
            "passkey origin unavailable",
        );
    };
    let Ok(credential) = serde_json::from_value::<RegisterPublicKeyCredential>(body.credential)
    else {
        return fail(StatusCode::BAD_REQUEST, "invalid passkey response");
    };
    let Ok(passkey) = webauthn.finish_passkey_registration(&credential, &registration) else {
        audit(&state, "owner_enrollment_failed", true);
        return fail(StatusCode::UNAUTHORIZED, "passkey verification failed");
    };
    let Ok(_lock) = state.owner_auth.lock() else {
        return fail(
            StatusCode::SERVICE_UNAVAILABLE,
            "identity store unavailable",
        );
    };
    if !matches!(state.owner_auth.read(), Ok(None)) {
        return fail(
            StatusCode::CONFLICT,
            "owner already enrolled or identity store unavailable",
        );
    }
    let mut codes = Vec::with_capacity(10);
    for _ in 0..10 {
        let Ok(code) = random_hex(16) else {
            return fail(StatusCode::SERVICE_UNAVAILABLE, "random source unavailable");
        };
        codes.push(code);
    }
    let owner = OwnerRecord {
        version: 1,
        id: owner_id,
        passkeys: vec![passkey],
        recovery_hashes: codes.iter().map(|code| recovery_hash(code)).collect(),
    };
    if state.owner_auth.write(&owner).is_err() {
        return fail(
            StatusCode::SERVICE_UNAVAILABLE,
            "identity store unavailable",
        );
    }
    state.browser_sessions.revoke_all();
    audit(&state, "owner_enrolled", false);
    session_response(
        &state,
        &headers,
        serde_json::json!({"ok": true, "recovery_codes": codes}),
    )
}

pub(crate) async fn passkey_start(State(state): State<AppState>, headers: HeaderMap) -> Response {
    let Ok(_lock) = state.owner_auth.lock() else {
        return fail(
            StatusCode::SERVICE_UNAVAILABLE,
            "identity store unavailable",
        );
    };
    let Ok(Some(owner)) = state.owner_auth.read() else {
        return fail(StatusCode::UNAUTHORIZED, "owner not enrolled");
    };
    let Ok(webauthn) = webauthn(&state, &headers) else {
        return fail(
            StatusCode::SERVICE_UNAVAILABLE,
            "passkey origin unavailable",
        );
    };
    let Ok((options, authentication)) = webauthn.start_passkey_authentication(&owner.passkeys)
    else {
        return fail(
            StatusCode::SERVICE_UNAVAILABLE,
            "passkey authentication unavailable",
        );
    };
    let Ok(challenge_id) = random_hex(24) else {
        return fail(StatusCode::SERVICE_UNAVAILABLE, "random source unavailable");
    };
    state.owner_auth.pending_insert(
        challenge_id.clone(),
        Pending::Authentication {
            state: authentication,
            rotate_recovery: false,
            expires: Instant::now() + CHALLENGE_TTL,
        },
    );
    Json(serde_json::json!({"challenge_id": challenge_id, "options": options.public_key}))
        .into_response()
}

pub(crate) async fn passkey_finish(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(body): Json<CeremonyFinish>,
) -> Response {
    let Some(Pending::Authentication {
        state: authentication,
        rotate_recovery: false,
        ..
    }) = state.owner_auth.pending_take(&body.challenge_id)
    else {
        return fail(StatusCode::BAD_REQUEST, "challenge expired or already used");
    };
    let Ok(webauthn) = webauthn(&state, &headers) else {
        return fail(
            StatusCode::SERVICE_UNAVAILABLE,
            "passkey origin unavailable",
        );
    };
    let Ok(credential) = serde_json::from_value::<PublicKeyCredential>(body.credential) else {
        return fail(StatusCode::BAD_REQUEST, "invalid passkey response");
    };
    let Ok(result) = webauthn.finish_passkey_authentication(&credential, &authentication) else {
        audit(&state, "passkey_login_failed", true);
        return fail(StatusCode::UNAUTHORIZED, "passkey verification failed");
    };
    let Ok(_lock) = state.owner_auth.lock() else {
        return fail(
            StatusCode::SERVICE_UNAVAILABLE,
            "identity store unavailable",
        );
    };
    let Ok(Some(mut owner)) = state.owner_auth.read() else {
        return fail(StatusCode::UNAUTHORIZED, "owner not enrolled");
    };
    let mut matched = false;
    for passkey in &mut owner.passkeys {
        if passkey.update_credential(&result).is_some() {
            matched = true;
        }
    }
    if !matched || state.owner_auth.write(&owner).is_err() {
        audit(&state, "passkey_login_failed", true);
        return fail(StatusCode::UNAUTHORIZED, "passkey verification failed");
    }
    audit(&state, "passkey_login", false);
    session_response(&state, &headers, serde_json::json!({"ok": true}))
}

/// A fresh passkey assertion is required to replace recovery codes; a
/// long-lived browser cookie alone cannot authorize this account action.
pub(crate) async fn rotate_recovery_start(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Response {
    if !has_owner_session(&state, &headers) {
        return fail(StatusCode::UNAUTHORIZED, "owner session required");
    }
    let Ok(_lock) = state.owner_auth.lock() else {
        return fail(
            StatusCode::SERVICE_UNAVAILABLE,
            "identity store unavailable",
        );
    };
    let Ok(Some(owner)) = state.owner_auth.read() else {
        return fail(StatusCode::UNAUTHORIZED, "owner not enrolled");
    };
    let Ok(webauthn) = webauthn(&state, &headers) else {
        return fail(
            StatusCode::SERVICE_UNAVAILABLE,
            "passkey origin unavailable",
        );
    };
    let Ok((options, authentication)) = webauthn.start_passkey_authentication(&owner.passkeys)
    else {
        return fail(
            StatusCode::SERVICE_UNAVAILABLE,
            "passkey authentication unavailable",
        );
    };
    let Ok(challenge_id) = random_hex(24) else {
        return fail(StatusCode::SERVICE_UNAVAILABLE, "random source unavailable");
    };
    state.owner_auth.pending_insert(
        challenge_id.clone(),
        Pending::Authentication {
            state: authentication,
            rotate_recovery: true,
            expires: Instant::now() + CHALLENGE_TTL,
        },
    );
    Json(serde_json::json!({"challenge_id": challenge_id, "options": options.public_key}))
        .into_response()
}

pub(crate) async fn rotate_recovery_finish(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(body): Json<CeremonyFinish>,
) -> Response {
    if !has_owner_session(&state, &headers) {
        return fail(StatusCode::UNAUTHORIZED, "owner session required");
    }
    let Some(Pending::Authentication {
        state: authentication,
        rotate_recovery: true,
        ..
    }) = state.owner_auth.pending_take(&body.challenge_id)
    else {
        return fail(StatusCode::BAD_REQUEST, "challenge expired or already used");
    };
    let Ok(webauthn) = webauthn(&state, &headers) else {
        return fail(
            StatusCode::SERVICE_UNAVAILABLE,
            "passkey origin unavailable",
        );
    };
    let Ok(credential) = serde_json::from_value::<PublicKeyCredential>(body.credential) else {
        return fail(StatusCode::BAD_REQUEST, "invalid passkey response");
    };
    let Ok(result) = webauthn.finish_passkey_authentication(&credential, &authentication) else {
        audit(&state, "recovery_rotation_failed", true);
        return fail(StatusCode::UNAUTHORIZED, "passkey verification failed");
    };
    let Ok(_lock) = state.owner_auth.lock() else {
        return fail(
            StatusCode::SERVICE_UNAVAILABLE,
            "identity store unavailable",
        );
    };
    let Ok(Some(mut owner)) = state.owner_auth.read() else {
        return fail(StatusCode::UNAUTHORIZED, "owner not enrolled");
    };
    if !owner
        .passkeys
        .iter_mut()
        .any(|passkey| passkey.update_credential(&result).is_some())
    {
        audit(&state, "recovery_rotation_failed", true);
        return fail(StatusCode::UNAUTHORIZED, "passkey verification failed");
    }
    let mut codes = Vec::with_capacity(10);
    for _ in 0..10 {
        let Ok(code) = random_hex(16) else {
            return fail(StatusCode::SERVICE_UNAVAILABLE, "random source unavailable");
        };
        codes.push(code);
    }
    owner.recovery_hashes = codes.iter().map(|code| recovery_hash(code)).collect();
    if state.owner_auth.write(&owner).is_err() {
        return fail(
            StatusCode::SERVICE_UNAVAILABLE,
            "identity store unavailable",
        );
    }
    audit(&state, "recovery_codes_rotated", false);
    Json(serde_json::json!({"recovery_codes": codes})).into_response()
}

#[derive(Deserialize)]
pub(crate) struct RecoveryBody {
    code: String,
}

pub(crate) async fn recovery(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(body): Json<RecoveryBody>,
) -> Response {
    if body.code.len() > 128 {
        return fail(StatusCode::UNAUTHORIZED, "recovery code invalid");
    }
    let hash = recovery_hash(&body.code);
    let Ok(_lock) = state.owner_auth.lock() else {
        return fail(
            StatusCode::SERVICE_UNAVAILABLE,
            "identity store unavailable",
        );
    };
    let Ok(Some(mut owner)) = state.owner_auth.read() else {
        return fail(StatusCode::UNAUTHORIZED, "recovery code invalid");
    };
    let Some(index) = owner
        .recovery_hashes
        .iter()
        .position(|saved| bool::from(saved.as_bytes().ct_eq(hash.as_bytes())))
    else {
        audit(&state, "recovery_failed", true);
        return fail(StatusCode::UNAUTHORIZED, "recovery code invalid");
    };
    owner.recovery_hashes.remove(index);
    if state.owner_auth.write(&owner).is_err() {
        return fail(
            StatusCode::SERVICE_UNAVAILABLE,
            "identity store unavailable",
        );
    }
    audit(&state, "recovery_used", false);
    session_response(
        &state,
        &headers,
        serde_json::json!({"ok": true, "recovery_codes_remaining": owner.recovery_hashes.len()}),
    )
}

fn has_owner_session(state: &AppState, headers: &HeaderMap) -> bool {
    web::session_cookie(headers).is_some_and(|token| state.browser_sessions.valid(token))
}

pub(crate) async fn add_start(State(state): State<AppState>, headers: HeaderMap) -> Response {
    if !has_owner_session(&state, &headers) {
        return fail(StatusCode::UNAUTHORIZED, "owner session required");
    }
    let Ok(_lock) = state.owner_auth.lock() else {
        return fail(
            StatusCode::SERVICE_UNAVAILABLE,
            "identity store unavailable",
        );
    };
    let Ok(Some(owner)) = state.owner_auth.read() else {
        return fail(StatusCode::UNAUTHORIZED, "owner not enrolled");
    };
    let Ok(webauthn) = webauthn(&state, &headers) else {
        return fail(
            StatusCode::SERVICE_UNAVAILABLE,
            "passkey origin unavailable",
        );
    };
    let exclude = owner
        .passkeys
        .iter()
        .map(|passkey| passkey.cred_id().clone())
        .collect();
    let Ok((options, registration)) =
        webauthn.start_passkey_registration(owner.id, "owner", "Owner", Some(exclude))
    else {
        return fail(
            StatusCode::SERVICE_UNAVAILABLE,
            "passkey enrollment unavailable",
        );
    };
    let Ok(challenge_id) = random_hex(24) else {
        return fail(StatusCode::SERVICE_UNAVAILABLE, "random source unavailable");
    };
    state.owner_auth.pending_insert(
        challenge_id.clone(),
        Pending::Registration {
            state: registration,
            owner_id: owner.id,
            add: true,
            expires: Instant::now() + CHALLENGE_TTL,
        },
    );
    Json(serde_json::json!({"challenge_id": challenge_id, "options": options.public_key}))
        .into_response()
}

pub(crate) async fn add_finish(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(body): Json<CeremonyFinish>,
) -> Response {
    if !has_owner_session(&state, &headers) {
        return fail(StatusCode::UNAUTHORIZED, "owner session required");
    }
    let Some(Pending::Registration {
        state: registration,
        owner_id,
        add: true,
        ..
    }) = state.owner_auth.pending_take(&body.challenge_id)
    else {
        return fail(StatusCode::BAD_REQUEST, "challenge expired or already used");
    };
    let Ok(webauthn) = webauthn(&state, &headers) else {
        return fail(
            StatusCode::SERVICE_UNAVAILABLE,
            "passkey origin unavailable",
        );
    };
    let Ok(credential) = serde_json::from_value::<RegisterPublicKeyCredential>(body.credential)
    else {
        return fail(StatusCode::BAD_REQUEST, "invalid passkey response");
    };
    let Ok(passkey) = webauthn.finish_passkey_registration(&credential, &registration) else {
        return fail(StatusCode::UNAUTHORIZED, "passkey verification failed");
    };
    let Ok(_lock) = state.owner_auth.lock() else {
        return fail(
            StatusCode::SERVICE_UNAVAILABLE,
            "identity store unavailable",
        );
    };
    let Ok(Some(mut owner)) = state.owner_auth.read() else {
        return fail(StatusCode::UNAUTHORIZED, "owner not enrolled");
    };
    if owner.id != owner_id
        || owner
            .passkeys
            .iter()
            .any(|existing| existing.cred_id() == passkey.cred_id())
    {
        return fail(StatusCode::CONFLICT, "passkey already enrolled");
    }
    owner.passkeys.push(passkey);
    if state.owner_auth.write(&owner).is_err() {
        return fail(
            StatusCode::SERVICE_UNAVAILABLE,
            "identity store unavailable",
        );
    }
    audit(&state, "passkey_added", false);
    Json(serde_json::json!({"ok": true, "passkeys": owner.passkeys.len()})).into_response()
}

pub(crate) async fn account(State(state): State<AppState>, headers: HeaderMap) -> Response {
    if !has_owner_session(&state, &headers) {
        return fail(StatusCode::UNAUTHORIZED, "owner session required");
    }
    let Ok(_lock) = state.owner_auth.lock() else {
        return fail(
            StatusCode::SERVICE_UNAVAILABLE,
            "identity store unavailable",
        );
    };
    let Ok(Some(owner)) = state.owner_auth.read() else {
        return fail(StatusCode::NOT_FOUND, "owner not enrolled");
    };
    Json(serde_json::json!({
        "passkeys": owner.passkeys.len(),
        "recovery_codes_remaining": owner.recovery_hashes.len(),
    }))
    .into_response()
}

pub(crate) async fn revoke_all_sessions(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Response {
    if !has_owner_session(&state, &headers) {
        return fail(StatusCode::UNAUTHORIZED, "owner session required");
    }
    state.browser_sessions.revoke_all();
    audit(&state, "sessions_revoked", false);
    (
        [(
            header::SET_COOKIE,
            format!(
                "{}=; HttpOnly; SameSite=Strict; Path=/; Max-Age=0",
                crate::admin::SESSION_COOKIE
            ),
        )],
        Json(serde_json::json!({"ok": true})),
    )
        .into_response()
}
