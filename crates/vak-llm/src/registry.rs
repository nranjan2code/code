use std::collections::HashMap;
use std::sync::{Arc, PoisonError, RwLock};

use tokio_util::sync::CancellationToken;

use crate::Provider;
use crate::anthropic::{AnthropicConfig, AnthropicProvider, DEFAULT_BASE_URL};
use crate::error::LlmError;
use crate::stream::EventStream;
use crate::types::ChatRequest;

#[derive(Debug, Clone, Default)]
pub struct ProviderAuth {
    pub api_key: String,
    pub base_url: Option<String>,
    /// Non-secret identity used to freeze a credential choice in a route.
    /// Providers continue to authenticate with `api_key`; this label is
    /// never sent over the wire.
    pub credential_id: Option<String>,
    /// Generic per-provider tuning knobs threaded from `vak-config`
    /// (docs/design/68-context-engine.md §8), e.g. Ollama's `keep_alive`/
    /// `num_ctx`. Keys and meaning are entirely provider-defined; the
    /// registry and this struct stay agnostic to their contents.
    pub options: std::collections::BTreeMap<String, String>,
}

type Factory = Arc<dyn Fn(&ProviderAuth) -> Result<Arc<dyn Provider>, LlmError> + Send + Sync>;

/// Cache identity: same provider name but a different key or base URL is
/// a DIFFERENT provider — returning the cached one would silently send
/// requests with stale credentials to the wrong endpoint.
fn cache_key(name: &str, auth: &ProviderAuth) -> String {
    use std::hash::{Hash, Hasher};
    let mut h = std::collections::hash_map::DefaultHasher::new();
    auth.api_key.hash(&mut h);
    auth.options.hash(&mut h);
    format!(
        "{name}\u{0}{}\u{0}{:016x}",
        auth.base_url.as_deref().unwrap_or(""),
        h.finish()
    )
}

#[derive(Default)]
pub struct ProviderRegistry {
    factories: RwLock<HashMap<String, Factory>>,
    cache: RwLock<HashMap<String, Arc<dyn Provider>>>,
}

impl ProviderRegistry {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn register(
        &self,
        name: impl Into<String>,
        factory: impl Fn(&ProviderAuth) -> Result<Arc<dyn Provider>, LlmError> + Send + Sync + 'static,
    ) {
        self.factories
            .write()
            .unwrap_or_else(PoisonError::into_inner)
            .insert(name.into(), Arc::new(factory));
    }

    pub fn get(&self, name: &str, auth: &ProviderAuth) -> Result<Arc<dyn Provider>, LlmError> {
        let key = cache_key(name, auth);
        if let Some(cached) = self
            .cache
            .read()
            .unwrap_or_else(PoisonError::into_inner)
            .get(&key)
        {
            return Ok(cached.clone());
        }
        let factory = self
            .factories
            .read()
            .unwrap_or_else(PoisonError::into_inner)
            .get(name)
            .cloned()
            .ok_or_else(|| LlmError::InvalidRequest(format!("unknown provider: {name}")))?;
        let provider = factory(auth)?;
        self.cache
            .write()
            .unwrap_or_else(PoisonError::into_inner)
            .insert(key, provider.clone());
        Ok(provider)
    }

    pub fn names(&self) -> Vec<String> {
        let mut names: Vec<String> = self
            .factories
            .read()
            .unwrap_or_else(PoisonError::into_inner)
            .keys()
            .cloned()
            .collect();
        names.sort();
        names
    }
}

pub fn default_registry() -> ProviderRegistry {
    let registry = ProviderRegistry::new();
    registry.register("anthropic", |auth| {
        Ok(Arc::new(AnthropicProvider::new(
            anthropic_config_from_auth(auth),
        )?))
    });

    use crate::openai::{OPENAI_DEFAULT_BASE_URL, OpenAiCompletionsProvider, OpenAiConfig};
    let openai_compat = |default_base: &'static str, cache_key: bool, openrouter: bool| {
        move |auth: &ProviderAuth| {
            Ok(Arc::new(OpenAiCompletionsProvider::new(OpenAiConfig {
                api_key: auth.api_key.clone(),
                base_url: auth.base_url.clone().unwrap_or_else(|| default_base.into()),
                cache_key,
                openrouter,
            })?) as Arc<dyn Provider>)
        }
    };
    registry.register(
        "openai",
        openai_compat(OPENAI_DEFAULT_BASE_URL, true, false),
    );

    use crate::openai_responses::{
        OPENAI_RESPONSES_DEFAULT_BASE_URL, OpenAiResponsesConfig, OpenAiResponsesProvider,
    };
    let responses = |default_base: &'static str, cache_key: bool, openrouter: bool| {
        move |auth: &ProviderAuth| {
            Ok(
                Arc::new(OpenAiResponsesProvider::new(OpenAiResponsesConfig {
                    api_key: auth.api_key.clone(),
                    base_url: auth.base_url.clone().unwrap_or_else(|| default_base.into()),
                    cache_key,
                    openrouter,
                })?) as Arc<dyn Provider>,
            )
        }
    };
    registry.register(
        "openai-responses",
        responses(OPENAI_RESPONSES_DEFAULT_BASE_URL, true, false),
    );

    use crate::google::{GOOGLE_DEFAULT_BASE_URL, GoogleConfig, GoogleProvider};
    registry.register("google", |auth| {
        Ok(Arc::new(GoogleProvider::new(GoogleConfig {
            api_key: auth.api_key.clone(),
            project_id: auth.options.get("project_id").cloned(),
            base_url: auth
                .base_url
                .clone()
                .unwrap_or_else(|| GOOGLE_DEFAULT_BASE_URL.into()),
        })?) as Arc<dyn Provider>)
    });
    registry.register(
        "openrouter",
        openai_compat("https://openrouter.ai/api/v1", true, true),
    );
    registry.register(
        "openrouter-responses",
        responses("https://openrouter.ai/api/v1", true, true),
    );
    registry.register(
        "opencode-zen",
        openai_compat("https://opencode.ai/zen/v1", true, false),
    );

    use crate::ollama::OllamaProvider;
    registry.register("ollama", |auth| {
        Ok(Arc::new(OllamaProvider::new(ollama_config_from_auth(auth))?) as Arc<dyn Provider>)
    });

    // Amazon Bedrock Mantle exposes an OpenAI-compatible API. The region is
    // part of the endpoint; callers may override it with VAK_BEDROCK_BASE_URL.
    registry.register(
        "bedrock",
        openai_compat("https://bedrock-mantle.us-east-1.api.aws/v1", true, false),
    );
    registry
}

/// Builds an `AnthropicConfig` from generic `ProviderAuth` fields
/// (docs/design/68-context-engine.md §11 "Anthropic" row). Standalone so it
/// can be unit tested without constructing a live `AnthropicProvider`.
fn anthropic_config_from_auth(auth: &ProviderAuth) -> AnthropicConfig {
    AnthropicConfig {
        api_key: auth.api_key.clone(),
        base_url: auth
            .base_url
            .clone()
            .unwrap_or_else(|| DEFAULT_BASE_URL.into()),
        model: String::new(),
        fast_mode: auth
            .options
            .get("fast_mode")
            .and_then(|v| v.parse::<bool>().ok())
            .unwrap_or(false),
    }
}

/// Builds an `OllamaConfig` from generic `ProviderAuth` fields
/// (docs/design/68-context-engine.md §8). Standalone so it can be unit
/// tested without constructing a live `OllamaProvider`.
fn ollama_config_from_auth(auth: &ProviderAuth) -> crate::ollama::OllamaConfig {
    let default_config = crate::ollama::OllamaConfig::default();
    // `auth.base_url` may carry the `/v1` OpenAI-compat suffix used for
    // discovery (models.rs::default_base_url); the native `/api/chat` wire
    // always wants the bare root.
    let base_url = auth
        .base_url
        .clone()
        .unwrap_or_else(|| crate::ollama::OLLAMA_DEFAULT_BASE_URL.into());
    let base_url = base_url
        .trim_end_matches('/')
        .trim_end_matches("/v1")
        .to_string();
    let keep_alive = auth
        .options
        .get("keep_alive")
        .cloned()
        .unwrap_or(default_config.keep_alive);
    let num_ctx = auth
        .options
        .get("num_ctx")
        .and_then(|v| v.parse::<u64>().ok())
        .or(default_config.num_ctx);
    crate::ollama::OllamaConfig {
        base_url,
        api_key: auth.api_key.clone(),
        keep_alive,
        num_ctx,
    }
}

pub async fn stream_via(
    provider: &dyn Provider,
    request: ChatRequest,
    cancel: CancellationToken,
) -> Result<EventStream, LlmError> {
    provider.stream(request, cancel).await
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_registry_names_include_ollama() {
        assert!(default_registry().names().contains(&"ollama".to_string()));
    }

    #[test]
    fn anthropic_config_defaults_fast_mode_off() {
        let auth = ProviderAuth {
            api_key: "k".into(),
            base_url: None,
            credential_id: None,
            options: Default::default(),
        };
        let config = anthropic_config_from_auth(&auth);
        assert_eq!(config.base_url, DEFAULT_BASE_URL);
        assert!(!config.fast_mode);
    }

    #[test]
    fn anthropic_config_threads_fast_mode_from_options() {
        let mut options = std::collections::BTreeMap::new();
        options.insert("fast_mode".to_string(), "true".to_string());
        let auth = ProviderAuth {
            api_key: "k".into(),
            base_url: None,
            credential_id: None,
            options,
        };
        let config = anthropic_config_from_auth(&auth);
        assert!(config.fast_mode);
    }

    #[test]
    fn anthropic_config_ignores_unparseable_fast_mode() {
        let mut options = std::collections::BTreeMap::new();
        options.insert("fast_mode".to_string(), "yes-please".to_string());
        let auth = ProviderAuth {
            api_key: "k".into(),
            base_url: None,
            credential_id: None,
            options,
        };
        let config = anthropic_config_from_auth(&auth);
        assert!(!config.fast_mode);
    }

    #[test]
    fn ollama_config_defaults_when_no_options_are_set() {
        let auth = ProviderAuth {
            api_key: "ollama".into(),
            base_url: None,
            credential_id: None,
            options: Default::default(),
        };
        let config = ollama_config_from_auth(&auth);
        assert_eq!(config.base_url, crate::ollama::OLLAMA_DEFAULT_BASE_URL);
        assert_eq!(config.keep_alive, "30m");
        assert_eq!(config.num_ctx, None);
    }

    #[test]
    fn ollama_config_threads_keep_alive_and_num_ctx_from_options() {
        let mut options = std::collections::BTreeMap::new();
        options.insert("keep_alive".to_string(), "10m".to_string());
        options.insert("num_ctx".to_string(), "8192".to_string());
        let auth = ProviderAuth {
            api_key: "ollama".into(),
            base_url: None,
            credential_id: None,
            options,
        };
        let config = ollama_config_from_auth(&auth);
        assert_eq!(config.keep_alive, "10m");
        assert_eq!(config.num_ctx, Some(8192));
    }

    #[test]
    fn ollama_config_strips_v1_compat_suffix_from_base_url() {
        let auth = ProviderAuth {
            api_key: "ollama".into(),
            base_url: Some("http://localhost:11434/v1".into()),
            credential_id: None,
            options: Default::default(),
        };
        let config = ollama_config_from_auth(&auth);
        assert_eq!(config.base_url, "http://localhost:11434");
    }

    #[test]
    fn ollama_config_ignores_unparseable_num_ctx() {
        let mut options = std::collections::BTreeMap::new();
        options.insert("num_ctx".to_string(), "not-a-number".to_string());
        let auth = ProviderAuth {
            api_key: "ollama".into(),
            base_url: None,
            credential_id: None,
            options,
        };
        let config = ollama_config_from_auth(&auth);
        assert_eq!(config.num_ctx, None);
    }
}
