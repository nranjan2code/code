//! Closed registry for compiled first-party native adapters.
//!
//! Package manifests may select an identifier here, but cannot contribute an
//! executor, host, OAuth handler, credential recipient, or permission. A
//! registration with no compiled executor remains visible as gated metadata
//! and must never be dispatched.

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AdapterAvailability {
    OwnerPreview,
    Gated,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AdapterAuth {
    ApiKey,
    OAuthMember,
    OAuthUser,
    BearerToken,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CompiledExecutor {
    YoutubeOwnerSearchPreview,
    None,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct NativeAdapterRegistration {
    pub id: &'static str,
    pub plugin_id: &'static str,
    pub platform: &'static str,
    pub api_version: &'static str,
    /// Exact HTTPS API host. Callers must construct endpoint paths from
    /// reviewed code; this is never taken from a package or user input.
    pub api_host: &'static str,
    pub auth: AdapterAuth,
    /// Credential variable name, not a credential value.
    pub credential_binding: &'static str,
    /// Trusted code component that may resolve this adapter's credential.
    pub secret_recipient: &'static str,
    /// Candidate scopes are review metadata only; gated registrations cannot
    /// request or obtain them.
    pub candidate_scopes: &'static [&'static str],
    /// Tool identities that compiled dispatch currently exposes.
    pub capabilities: &'static [&'static str],
    pub max_results: u8,
    pub max_response_bytes: usize,
    pub timeout_seconds: u8,
    pub availability: AdapterAvailability,
    pub executor: CompiledExecutor,
    pub gate_reason: &'static str,
}

/// Only reviewed identifiers are registered here. The gated rows reserve
/// platform-specific contracts but provide no callable API or executor.
pub const NATIVE_ADAPTERS: &[NativeAdapterRegistration] = &[
    NativeAdapterRegistration {
        id: "reddit-data-api-v1",
        plugin_id: "social-reddit",
        platform: "Reddit",
        api_version: "v1",
        api_host: "oauth.reddit.com",
        auth: AdapterAuth::OAuthUser,
        credential_binding: "VAK_SOCIAL_REDDIT_ACCESS_TOKEN",
        secret_recipient: "vak-core/social-reddit",
        candidate_scopes: &["read"],
        capabilities: &[],
        max_results: 10,
        max_response_bytes: 1024 * 1024,
        timeout_seconds: 12,
        availability: AdapterAvailability::Gated,
        executor: CompiledExecutor::None,
        gate_reason: "Reddit eligibility and deleted-content erasure are not satisfied.",
    },
    NativeAdapterRegistration {
        id: "youtube-data-api-v3",
        plugin_id: "social-youtube",
        platform: "YouTube",
        api_version: "v3",
        api_host: "www.googleapis.com",
        auth: AdapterAuth::ApiKey,
        credential_binding: "VAK_YOUTUBE_DATA_API_KEY",
        secret_recipient: "vak-server/social-youtube-preview",
        candidate_scopes: &[],
        capabilities: &["social.youtube.owner_search_preview"],
        max_results: 10,
        max_response_bytes: 1024 * 1024,
        timeout_seconds: 12,
        availability: AdapterAvailability::OwnerPreview,
        executor: CompiledExecutor::YoutubeOwnerSearchPreview,
        gate_reason: "Owner-only preview; model-facing use is blocked by the data-retention contract.",
    },
    NativeAdapterRegistration {
        id: "x-api-v2",
        plugin_id: "social-x",
        platform: "X",
        api_version: "v2",
        api_host: "api.x.com",
        auth: AdapterAuth::BearerToken,
        credential_binding: "VAK_SOCIAL_X_BEARER_TOKEN",
        secret_recipient: "vak-core/social-x",
        candidate_scopes: &[],
        capabilities: &[],
        max_results: 10,
        max_response_bytes: 1024 * 1024,
        timeout_seconds: 12,
        availability: AdapterAvailability::Gated,
        executor: CompiledExecutor::None,
        gate_reason: "X API dispatch, metering, and a hard local spending ceiling are not implemented.",
    },
    NativeAdapterRegistration {
        id: "linkedin-api-v2",
        plugin_id: "social-linkedin",
        platform: "LinkedIn",
        api_version: "v2",
        api_host: "api.linkedin.com",
        auth: AdapterAuth::OAuthMember,
        credential_binding: "VAK_SOCIAL_LINKEDIN_ACCESS_TOKEN",
        secret_recipient: "vak-core/social-linkedin",
        candidate_scopes: &["profile", "email", "w_member_social"],
        capabilities: &[],
        max_results: 10,
        max_response_bytes: 1024 * 1024,
        timeout_seconds: 12,
        availability: AdapterAvailability::Gated,
        executor: CompiledExecutor::None,
        gate_reason: "LinkedIn OAuth, per-scope approval, and account lifecycle are not implemented.",
    },
];

pub fn native_adapter(id: &str) -> Option<&'static NativeAdapterRegistration> {
    NATIVE_ADAPTERS.iter().find(|adapter| adapter.id == id)
}

pub fn native_adapter_for_plugin(plugin_id: &str) -> Option<&'static NativeAdapterRegistration> {
    NATIVE_ADAPTERS
        .iter()
        .find(|adapter| adapter.plugin_id == plugin_id)
}

pub fn validate_native_adapter(plugin_id: &str, adapter_id: &str) -> Result<(), String> {
    match native_adapter(adapter_id) {
        None => Err(format!("native adapter {adapter_id:?} is not registered")),
        Some(adapter) if adapter.plugin_id != plugin_id => Err(format!(
            "native adapter {adapter_id:?} is registered for {}, not {plugin_id}",
            adapter.plugin_id
        )),
        Some(_) => Ok(()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_registration_is_closed_to_one_plugin_and_fixed_host() {
        let mut ids = std::collections::BTreeSet::new();
        let mut plugins = std::collections::BTreeSet::new();
        for adapter in NATIVE_ADAPTERS {
            assert!(
                ids.insert(adapter.id),
                "duplicate adapter id {}",
                adapter.id
            );
            assert!(
                plugins.insert(adapter.plugin_id),
                "multiple adapters claim plugin {}",
                adapter.plugin_id
            );
            assert!(!adapter.api_host.contains('/'));
            assert!(!adapter.api_host.contains('@'));
            assert!(adapter.max_results > 0);
            assert!(adapter.max_response_bytes > 0);
            assert!(adapter.timeout_seconds > 0);
            if adapter.executor == CompiledExecutor::None {
                assert!(adapter.capabilities.is_empty());
                assert_eq!(adapter.availability, AdapterAvailability::Gated);
            }
        }
    }

    #[test]
    fn unknown_and_cross_platform_adapter_selections_fail_closed() {
        assert!(validate_native_adapter("social-youtube", "arbitrary-adapter").is_err());
        assert!(validate_native_adapter("social-x", "youtube-data-api-v3").is_err());
        assert!(validate_native_adapter("social-youtube", "youtube-data-api-v3").is_ok());
    }
}
