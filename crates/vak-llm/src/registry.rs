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
}

type Factory = Arc<dyn Fn(&ProviderAuth) -> Result<Arc<dyn Provider>, LlmError> + Send + Sync>;

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
        if let Some(cached) = self
            .cache
            .read()
            .unwrap_or_else(PoisonError::into_inner)
            .get(name)
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
            .insert(name.to_string(), provider.clone());
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
    registry
}

pub async fn stream_via(
    provider: &dyn Provider,
    request: ChatRequest,
    cancel: CancellationToken,
) -> Result<EventStream, LlmError> {
    provider.stream(request, cancel).await
}
