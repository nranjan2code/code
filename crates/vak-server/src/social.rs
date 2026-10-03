use std::path::PathBuf;

pub(crate) mod linkedin;
pub(crate) mod preview;

/// Where a social credential is written: `scope=user` is the Shared layer every
/// Agent inherits, anything else is the selected Agent's own scope.
pub(crate) fn credential_file(core: &vak_core::Core, scope: Option<&str>) -> PathBuf {
    if scope == Some("user") {
        core.shared_scope().env_file()
    } else {
        core.scope().env_file()
    }
}

/// The Agent's own value, else the Shared one — the same chain as every secret.
pub(crate) fn resolve_credential(core: &vak_core::Core, key: &str) -> Option<String> {
    vak_config::read_env_file_var(&core.scope().env_file(), key)
        .or_else(|| vak_config::read_env_file_var(&core.shared_scope().env_file(), key))
        .filter(|value| !value.trim().is_empty())
}

/// `configured` is the layer being viewed; `inherited` says an Agent view is
/// currently served by the Shared value.
pub(crate) fn credential_state(
    core: &vak_core::Core,
    key: &str,
    scope: Option<&str>,
) -> serde_json::Value {
    let present = |path: PathBuf| {
        vak_config::read_env_file_var(&path, key).is_some_and(|v| !v.trim().is_empty())
    };
    let configured = present(credential_file(core, scope));
    let inherited = scope != Some("user") && !configured && present(core.shared_scope().env_file());
    serde_json::json!({ "configured": configured, "inherited": inherited })
}
