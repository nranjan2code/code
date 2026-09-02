//! The durable gateway bearer token.
/// Ensure a durable gateway bearer token exists, and return it.
///
/// Pinned at **activation**, which is the moment a token first has to
/// outlive a process: a bridge unit, the tray's "Open Admin Console", and
/// any saved console link all authenticate against it, and a token minted
/// per boot invalidates every one of them on each restart.
///
/// This used to happen in `vak self install`. It was removed when install
/// stopped activating anything (doc 46 D6) and the note said it moved
/// here — but it did not, so a freshly installed machine registered a
/// bridge unit that could never authenticate and crash-looped with
/// "gateway token missing". Found by installing the stack for real.
///
/// Idempotent: an existing value is never replaced, so activating again
/// does not invalidate a link already in use. An explicitly empty value is
/// left alone too — that is a deliberate "unpinned" choice.
pub fn ensure_gateway_token() -> Result<(), String> {
    const KEY: &str = "VAK_GATEWAY_TOKEN";
    let Some(path) = vak_config::user_env_path() else {
        return Ok(());
    };
    let existing = std::fs::read_to_string(&path).unwrap_or_default();
    let already_set = existing
        .lines()
        .any(|line| line.split_once('=').is_some_and(|(k, _)| k.trim() == KEY));
    if already_set {
        return Ok(());
    }
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)
            .map_err(|e| format!("creating {}: {e}", parent.display()))?;
    }
    let token = format!("vk_{}", uuid::Uuid::now_v7());
    vak_config::upsert_env_file(&path, KEY, &token)
        .map_err(|e| format!("writing {}: {e}", path.display()))?;
    // Visible to this process immediately, so a server started right after
    // activation uses the pinned value rather than minting its own.
    vak_config::set_override(KEY, token);
    Ok(())
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::*;

    /// The defect a real install exposed: with nothing pinning a token,
    /// every boot minted a new one, so a bridge unit registered at
    /// activation could never authenticate and crash-looped.
    #[test]
    fn activation_pins_a_token_and_never_replaces_an_existing_one() {
        let home = vak_config::paths::isolate_home_for_tests();
        let path = vak_config::user_env_path().expect("user env path");
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).unwrap();
        }
        let _ = std::fs::remove_file(&path);
        vak_config::clear_override("VAK_GATEWAY_TOKEN");

        ensure_gateway_token().unwrap();
        let first = std::fs::read_to_string(&path).unwrap();
        assert!(
            first.contains("VAK_GATEWAY_TOKEN=vk_"),
            "a token must be pinned: {first}"
        );

        // Re-activating must not invalidate a link already in use.
        ensure_gateway_token().unwrap();
        assert_eq!(
            std::fs::read_to_string(&path).unwrap(),
            first,
            "re-activation replaced an existing token"
        );
        assert!(home.exists());
    }
}
