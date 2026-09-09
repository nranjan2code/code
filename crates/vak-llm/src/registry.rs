use std::collections::HashMap;
use std::sync::{Arc, PoisonError, RwLock};

use tokio_util::sync::CancellationToken;

use crate::Provider;
use crate::anthropic::{AnthropicConfig, AnthropicProvider, DEFAULT_BASE_URL};
use crate::error::LlmError;
use crate::stream::EventStream;
use crate::types::ChatRequest;

#[derive(Debug, Clone)]
pub struct ProviderAuth {
    pub api_key: String,
    pub base_url: Option<String>,
    /// Non-secret identity used to freeze a credential choice in a route.
    /// Providers continue to authenticate with `api_key`; this label is
    /// never sent over the wire.
    pub credential_id: Option<String>,
}

type Factory = Arc<dyn Fn(&ProviderAuth) -> Result<Arc<dyn Provider>, LlmError> + Send + Sync>;

/// Cache identity: same provider name but a different key or base URL is
/// a DIFFERENT provider — returning the cached one would silently send
/// requests with stale credentials to the wrong endpoint.
fn cache_key(name: &str, auth: &ProviderAuth) -> String {
    use std::hash::{Hash, Hasher};
    let mut h = std::collections::hash_map::DefaultHasher::new();
    auth.api_key.hash(&mut h);
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
        Ok(Arc::new(AnthropicProvider::new(AnthropicConfig {
            api_key: auth.api_key.clone(),
            base_url: auth
                .base_url
                .clone()
                .unwrap_or_else(|| DEFAULT_BASE_URL.into()),
            model: String::new(),
        })?))
    });

    use crate::openai::{OPENAI_DEFAULT_BASE_URL, OpenAiCompletionsProvider, OpenAiConfig};
    let openai_compat = |default_base: &'static str| {
        move |auth: &ProviderAuth| {
            Ok(Arc::new(OpenAiCompletionsProvider::new(OpenAiConfig {
                api_key: auth.api_key.clone(),
                base_url: auth.base_url.clone().unwrap_or_else(|| default_base.into()),
            })?) as Arc<dyn Provider>)
        }
    };
    registry.register("openai", openai_compat(OPENAI_DEFAULT_BASE_URL));

    use crate::openai_responses::{
        OPENAI_RESPONSES_DEFAULT_BASE_URL, OpenAiResponsesConfig, OpenAiResponsesProvider,
    };
    let responses = |default_base: &'static str| {
        move |auth: &ProviderAuth| {
            Ok(
                Arc::new(OpenAiResponsesProvider::new(OpenAiResponsesConfig {
                    api_key: auth.api_key.clone(),
                    base_url: auth.base_url.clone().unwrap_or_else(|| default_base.into()),
                })?) as Arc<dyn Provider>,
            )
        }
    };
    registry.register(
        "openai-responses",
        responses(OPENAI_RESPONSES_DEFAULT_BASE_URL),
    );

    use crate::google::{GOOGLE_DEFAULT_BASE_URL, GoogleConfig, GoogleProvider};
    registry.register("google", |auth| {
        Ok(Arc::new(GoogleProvider::new(GoogleConfig {
            api_key: auth.api_key.clone(),
            base_url: auth
                .base_url
                .clone()
                .unwrap_or_else(|| GOOGLE_DEFAULT_BASE_URL.into()),
        })?) as Arc<dyn Provider>)
    });
    registry.register("openrouter", openai_compat("https://openrouter.ai/api/v1"));
    registry.register(
        "openrouter-responses",
        responses("https://openrouter.ai/api/v1"),
    );
    registry.register("opencode-zen", openai_compat("https://opencode.ai/zen/v1"));
    registry.register("ollama", openai_compat("http://localhost:11434/v1"));
    // Amazon Bedrock Mantle exposes an OpenAI-compatible API. The region is
    // part of the endpoint; callers may override it with VAK_BEDROCK_BASE_URL.
    registry.register(
        "bedrock",
        openai_compat("https://bedrock-mantle.us-east-1.api.aws/v1"),
    );
    registry
}

pub async fn stream_via(
    provider: &dyn Provider,
    request: ChatRequest,
    cancel: CancellationToken,
) -> Result<EventStream, LlmError> {
    provider.stream(request, cancel).await
}
