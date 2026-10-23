use std::sync::atomic::{AtomicU64, Ordering};

/// A capacity identity no other provider in this process has had or will
/// have. Capacity gates are process-wide and outlive the provider that made
/// them, so a fixed name or an address lets one test's provider inherit
/// another's cooldowns and held reservations. `Default` draws a fresh one,
/// so a mock that derives `Default` gets its own key too.
pub struct CapacityKey(pub String);

impl Default for CapacityKey {
    fn default() -> Self {
        static NEXT: AtomicU64 = AtomicU64::new(0);
        Self(format!(
            "test-provider:{}",
            NEXT.fetch_add(1, Ordering::Relaxed)
        ))
    }
}

/// A `POST /triggers` body for a script automation every `every_secs`.
#[allow(dead_code)]
pub fn script_trigger(name: &str, command: &str, every_secs: u64) -> serde_json::Value {
    serde_json::json!({
        "name": name,
        "kind": { "kind": "schedule", "schedule": {
            "kind": "interval", "every_secs": every_secs, "anchor": chrono::Utc::now(),
        }},
        "action": { "kind": "script", "command": command },
    })
}

/// A `POST /triggers` body for a prompt automation every `every_secs`.
#[allow(dead_code)]
pub fn prompt_trigger(name: &str, text: &str, every_secs: u64) -> serde_json::Value {
    serde_json::json!({
        "name": name,
        "kind": { "kind": "schedule", "schedule": {
            "kind": "interval", "every_secs": every_secs, "anchor": chrono::Utc::now(),
        }},
        "action": { "kind": "prompt", "text": text },
    })
}

/// Unrelated finite model scripts opt out of automatic organisation explicitly.
/// Topic acceptance tests use the original provider and leave the default on.
#[allow(dead_code)]
pub fn set_provider_without_automatic_topics(
    core: &vak_core::Core,
    provider: std::sync::Arc<dyn vak_llm::Provider>,
) {
    core.set_provider_instance(std::sync::Arc::new(WithoutAutomaticTopics {
        scope: core.scope(),
        provider,
    }));
}

struct WithoutAutomaticTopics {
    scope: vak_config::scope::AgentScope,
    provider: std::sync::Arc<dyn vak_llm::Provider>,
}

#[async_trait::async_trait]
impl vak_llm::Provider for WithoutAutomaticTopics {
    fn name(&self) -> &str {
        self.provider.name()
    }
    fn reach(&self) -> vak_control::ModelCapabilities {
        self.provider.reach()
    }
    fn circuit_key(&self) -> String {
        self.provider.circuit_key()
    }
    fn rate_limit_key(&self) -> String {
        self.provider.rate_limit_key()
    }
    fn rate_limit_key_for_model(&self, model: &str) -> String {
        self.provider.rate_limit_key_for_model(model)
    }
    async fn refresh_capacity(
        &self,
        cancel: &tokio_util::sync::CancellationToken,
    ) -> Result<(), vak_llm::LlmError> {
        self.provider.refresh_capacity(cancel).await
    }
    async fn stream(
        &self,
        request: vak_llm::types::ChatRequest,
        cancel: tokio_util::sync::CancellationToken,
    ) -> Result<vak_llm::EventStream, vak_llm::LlmError> {
        if request
            .output_schema
            .as_ref()
            .is_some_and(|schema| schema.name == "turn_organisation")
        {
            let mut message = vak_llm::AssistantMessage::empty(&request.model);
            message.content = vec![vak_llm::ContentBlock::text(
                r#"{"title":"Fixture answer","outcomes":[]}"#,
            )];
            let (mut sink, rx) = vak_llm::stream::channel(4);
            sink.close_message(message).await;
            return Ok(rx);
        }
        if let Some(cache) = &request.cache
            && uuid::Uuid::parse_str(&cache.session_key).is_ok()
        {
            use vak_session::trace::{self, TraceKey};
            let trace = TraceKey::root(
                trace::local::tenant(),
                trace::local::space(&std::env::temp_dir()),
                trace::local::agent("fixture"),
                trace::Cause::User {
                    request_id: uuid::Uuid::now_v7().to_string(),
                },
            )
            .in_turn(&cache.session_key, &uuid::Uuid::now_v7().to_string())
            .acting(trace::local::local_owner(), None);
            vak_session::work_threads::TopicStore::new(&self.scope, &cache.session_key)
                .and_then(|store| store.set_automatic(false, &trace))
                .map_err(|error| vak_llm::LlmError::InvalidRequest(error.to_string()))?;
        }
        self.provider.stream(request, cancel).await
    }
}
