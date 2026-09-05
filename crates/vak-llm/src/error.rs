use crate::types::AssistantMessage;

#[derive(Debug, Clone, thiserror::Error)]
pub enum LlmError {
    #[error("authentication failed: {0}")]
    Auth(String),
    #[error("rate limited: {message}")]
    RateLimit {
        message: String,
        retry_after_secs: Option<u64>,
    },
    #[error("provider overloaded: {0}")]
    Overloaded(String),
    #[error("invalid request: {0}")]
    InvalidRequest(String),
    #[error("api error (status {status}): {message}")]
    Api { status: u16, message: String },
    #[error("network error: {0}")]
    Network(String),
    #[error("response parse error: {0}")]
    Parse(String),
    #[error("aborted before completion")]
    Aborted { partial: Option<AssistantMessage> },
}

impl LlmError {
    /// Preserve the provider's rejection while adding endpoint-level guidance
    /// when it explicitly identifies an unsupported tools/reasoning pairing.
    ///
    /// This deliberately keys off the provider's response rather than a model
    /// name table: model capabilities change independently of this binary.
    pub fn invalid_request_for_endpoint(endpoint: &str, message: impl Into<String>) -> Self {
        let message = message.into();
        let normalized = message.to_ascii_lowercase();
        let tool_reasoning_conflict = normalized.contains("reasoning_effort")
            && (normalized.contains("function tool") || normalized.contains("function calling"))
            && normalized.contains("response");
        if endpoint == "/v1/chat/completions" && tool_reasoning_conflict {
            return Self::InvalidRequest(format!(
                "{message}\n\nThe selected route uses {endpoint}, which the provider reports cannot combine this model's current reasoning effort with function tools. Use a Responses-capable route for this model (vak's native OpenAI route is `openai-responses`), or select a Chat Completions-compatible model/configuration with reasoning effort disabled. This is an endpoint capability mismatch, not a transient failure."
            ));
        }
        Self::InvalidRequest(message)
    }

    pub fn is_retryable(&self) -> bool {
        matches!(
            self,
            LlmError::RateLimit { .. } | LlmError::Overloaded(_) | LlmError::Network(_)
        ) && !self.is_terminal_quota()
    }

    /// Some gateways use HTTP 429 for a quota or billing ceiling that will
    /// not recover during this run. Retrying those errors burns a dispatch
    /// budget and delays a usable fallback without changing the outcome.
    pub fn is_terminal_quota(&self) -> bool {
        let LlmError::RateLimit { message, .. } = self else {
            return false;
        };
        let message = message.to_ascii_lowercase();
        [
            "free-models-per-day",
            "daily quota",
            "monthly quota",
            "quota exceeded",
            "spend limit",
            "credit limit",
            "insufficient credits",
            "billing limit",
        ]
        .iter()
        .any(|marker| message.contains(marker))
    }

    /// Server-advised wait for RateLimit; None otherwise.
    pub fn retry_after_secs(&self) -> Option<u64> {
        match self {
            LlmError::RateLimit {
                retry_after_secs, ..
            } => *retry_after_secs,
            _ => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::LlmError;

    #[test]
    fn terminal_quota_429_is_not_retried() {
        let error = LlmError::RateLimit {
            message: "Rate limit exceeded: free-models-per-day".into(),
            retry_after_secs: None,
        };
        assert!(error.is_terminal_quota());
        assert!(!error.is_retryable());
    }

    #[test]
    fn ordinary_rate_limit_remains_retryable() {
        let error = LlmError::RateLimit {
            message: "too many requests".into(),
            retry_after_secs: Some(2),
        };
        assert!(!error.is_terminal_quota());
        assert!(error.is_retryable());
    }

    #[test]
    fn chat_completions_tool_reasoning_rejection_explains_the_route_fix() {
        let error = LlmError::invalid_request_for_endpoint(
            "/v1/chat/completions",
            "Function tools with reasoning_effort are not supported for a model in /v1/chat/completions. To use function tools, use /v1/responses or set reasoning_effort to 'none'.",
        );
        let rendered = error.to_string();
        assert!(rendered.contains("openai-responses"));
        assert!(rendered.contains("endpoint capability mismatch"));
    }

    #[test]
    fn unrelated_invalid_request_is_preserved_verbatim() {
        let error =
            LlmError::invalid_request_for_endpoint("/v1/chat/completions", "model does not exist");
        assert_eq!(error.to_string(), "invalid request: model does not exist");
    }
}
