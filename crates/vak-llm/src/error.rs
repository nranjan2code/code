use crate::types::AssistantMessage;

#[derive(Debug, thiserror::Error)]
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
    pub fn is_retryable(&self) -> bool {
        matches!(
            self,
            LlmError::RateLimit { .. } | LlmError::Overloaded(_) | LlmError::Network(_)
        )
    }
}
