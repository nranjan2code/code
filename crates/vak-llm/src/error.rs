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
}
