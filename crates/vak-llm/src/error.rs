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
    #[error("provider quota exhausted: {0}")]
    QuotaExhausted(String),
    #[error("provider overloaded: {0}")]
    Overloaded(String),
    #[error("provider overloaded: {message}")]
    OverloadedWithRetryAfter {
        message: String,
        retry_after_secs: u64,
    },
    #[error("invalid request: {0}")]
    InvalidRequest(String),
    #[error("api error (status {status}): {message}")]
    Api { status: u16, message: String },
    #[error("network error: {0}")]
    Network(String),
    #[error("response parse error: {0}")]
    Parse(String),
    #[error("context budget exceeded: {0}")]
    Context(String),
    #[error("aborted before completion")]
    // Boxed: `AssistantMessage` grew past the threshold where every
    // `Result<_, LlmError>` in the crate trips `clippy::result_large_err`
    // (adding `Usage::prefill_ms`/`load_ms` and `AssistantMessage::response_id`
    // pushed it from ~104 to ~160 bytes).
    Aborted {
        partial: Option<Box<AssistantMessage>>,
    },
}

/// Provider phrasing, across every adapter, that means "the prompt does not
/// fit the model's context window" rather than some other malformed
/// request. Keyed off the provider's own wording rather than a status code
/// alone, because 400 also covers unrelated validation failures.
const OVER_LENGTH_MARKERS: [&str; 9] = [
    "exceeds the model's maximum context length", // Ollama
    "context_length_exceeded",                    // OpenAI
    "maximum context length",                     // OpenAI, vLLM, OpenRouter
    "prompt is too long",                         // Anthropic
    "exceeds the maximum number of tokens",       // Gemini
    "exceeds the available context size",         // llama.cpp server
    "input is too long",                          // Bedrock
    "reduce the length of the messages",          // OpenAI (older)
    "too large for model with",                   // Mistral
];

impl LlmError {
    /// Preserve provider-declared quota exhaustion as a terminal account
    /// condition. Unrecognized codes remain ordinary retryable throttles.
    pub fn from_rate_limit_code(
        code: Option<&str>,
        message: String,
        retry_after_secs: Option<u64>,
    ) -> Self {
        let normalized_code = code.unwrap_or_default().trim().to_ascii_lowercase();
        if matches!(
            normalized_code.as_str(),
            "insufficient_quota"
                | "billing_hard_limit_reached"
                | "billing_limit_exceeded"
                | "free-models-per-day"
                | "daily_limit_exceeded"
                | "monthly_limit_exceeded"
        ) {
            let detail = if normalized_code.is_empty() {
                message
            } else {
                format!("{normalized_code}: {message}")
            };
            Self::QuotaExhausted(detail)
        } else {
            let message = if normalized_code.is_empty() {
                message
            } else {
                format!("{normalized_code}: {message}")
            };
            Self::RateLimit {
                message,
                retry_after_secs,
            }
        }
    }

    /// Classify a 400-class rejection: over-length phrasing becomes
    /// `Context` (recoverable by re-planning the working set and retrying),
    /// everything else stays `InvalidRequest` (a permanent per-request
    /// failure). Every adapter's `map_status_error` routes its 400 branch
    /// through this so the distinction is made once, not per provider.
    pub fn classify_400(message: String) -> Self {
        let normalized = message.to_ascii_lowercase();
        if OVER_LENGTH_MARKERS
            .iter()
            .any(|marker| normalized.contains(marker))
        {
            LlmError::Context(message)
        } else {
            LlmError::InvalidRequest(message)
        }
    }

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
            LlmError::RateLimit { .. }
                | LlmError::Overloaded(_)
                | LlmError::OverloadedWithRetryAfter { .. }
                | LlmError::Network(_)
        ) && !self.is_terminal_quota()
    }

    /// Some gateways use HTTP 429 for a quota or billing ceiling that will
    /// not recover during this run. Retrying those errors burns a dispatch
    /// budget and delays a usable fallback without changing the outcome.
    pub fn is_terminal_quota(&self) -> bool {
        let LlmError::RateLimit { message, .. } = self else {
            return matches!(self, LlmError::QuotaExhausted(_));
        };
        let message = message.to_ascii_lowercase();
        [
            "free-models-per-day",
            "insufficient_quota",
            "perday",
            "per_day",
            "requestsperday",
            "tokensperday",
            "daily quota",
            "monthly quota",
            "quota exceeded",
            "spend limit",
            "credit limit",
            "insufficient credits",
            "insufficient_quota",
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
            LlmError::OverloadedWithRetryAfter {
                retry_after_secs, ..
            } => Some(*retry_after_secs),
            _ => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::LlmError;

    #[test]
    fn classify_400_detects_gemini_llamacpp_and_bedrock_phrasing() {
        for message in [
            "The input token count (1100000) exceeds the maximum number of tokens allowed (1048576).",
            "the request exceeds the available context size, try increasing it",
            "Input is too long for requested model.",
        ] {
            assert!(
                matches!(LlmError::classify_400(message.into()), LlmError::Context(_)),
                "{message}"
            );
        }
        assert!(matches!(
            LlmError::classify_400("unknown field `foo`".into()),
            LlmError::InvalidRequest(_)
        ));
    }

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
    fn provider_declared_quota_codes_are_terminal_but_throttle_codes_retry() {
        let quota = LlmError::from_rate_limit_code(
            Some("insufficient_quota"),
            "budget exhausted".into(),
            Some(30),
        );
        assert!(matches!(quota, LlmError::QuotaExhausted(_)));
        assert!(!quota.is_retryable());

        let throttle = LlmError::from_rate_limit_code(
            Some("rate_limit_exceeded"),
            "slow down".into(),
            Some(2),
        );
        assert!(matches!(throttle, LlmError::RateLimit { .. }));
        assert!(throttle.is_retryable());
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

    #[test]
    fn classify_400_detects_ollama_over_length_phrasing() {
        let error = LlmError::classify_400(
            "request exceeds the model's maximum context length (8192)".into(),
        );
        assert!(matches!(error, LlmError::Context(_)));
    }

    #[test]
    fn classify_400_detects_openai_context_length_exceeded_code() {
        let error = LlmError::classify_400(
            "This model's maximum context length is 8192 tokens. (context_length_exceeded)".into(),
        );
        assert!(matches!(error, LlmError::Context(_)));
    }

    #[test]
    fn classify_400_detects_openai_maximum_context_length_phrase() {
        let error = LlmError::classify_400(
            "your messages resulted in maximum context length exceeded".into(),
        );
        assert!(matches!(error, LlmError::Context(_)));
    }

    #[test]
    fn classify_400_detects_anthropic_prompt_too_long_phrase() {
        let error =
            LlmError::classify_400("prompt is too long: 220000 tokens > 200000 maximum".into());
        assert!(matches!(error, LlmError::Context(_)));
    }

    #[test]
    fn classify_400_leaves_unrelated_rejections_as_invalid_request() {
        let error = LlmError::classify_400("model does not exist".into());
        assert!(matches!(error, LlmError::InvalidRequest(_)));
    }
}
