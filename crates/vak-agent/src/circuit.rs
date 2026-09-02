//! Cross-run circuit breaker for provider health. Only retryable failures
//! (429/529/network) trip it; auth/config errors are the caller's problem.
//! While open, steps fail fast with the remaining cooldown instead of
//! burning their retry budget against a dead provider.

use std::collections::HashMap;
use std::sync::Mutex;
use std::time::{Duration, Instant};

#[derive(Debug, Clone)]
pub struct CircuitBreakerConfig {
    /// Consecutive retryable failures before the circuit opens.
    pub threshold: u32,
    /// How long an open circuit stays open before allowing a probe.
    pub cooldown: Duration,
}

impl Default for CircuitBreakerConfig {
    fn default() -> Self {
        CircuitBreakerConfig {
            threshold: 5,
            cooldown: Duration::from_secs(60),
        }
    }
}

#[derive(Debug, Default)]
struct State {
    consecutive_failures: u32,
    opened_at: Option<Instant>,
}

#[derive(Debug)]
pub struct CircuitBreaker {
    config: CircuitBreakerConfig,
    state: Mutex<HashMap<String, State>>,
}

#[derive(Debug, thiserror::Error)]
#[error(
    "circuit open for provider (cooling down {remaining_secs}s after {failures} consecutive failures)"
)]
pub struct CircuitOpen {
    pub remaining_secs: u64,
    pub failures: u32,
}

impl CircuitBreaker {
    pub fn new(config: CircuitBreakerConfig) -> Self {
        CircuitBreaker {
            config,
            state: Mutex::new(HashMap::new()),
        }
    }

    /// Err when the circuit is open and the cooldown has not elapsed.
    pub fn check(&self) -> Result<(), CircuitOpen> {
        self.check_key("")
    }

    /// Check one provider/key circuit. Provider names are used by the agent
    /// as the stable health domain; an empty key means the process-global
    /// helper semantics for callers that do not have a route identity.
    pub fn check_key(&self, key: &str) -> Result<(), CircuitOpen> {
        let mut states = self.lock();
        let st = states.entry(key.to_string()).or_default();
        if let Some(opened_at) = st.opened_at {
            let elapsed = opened_at.elapsed();
            if elapsed < self.config.cooldown {
                return Err(CircuitOpen {
                    remaining_secs: (self.config.cooldown - elapsed).as_secs().max(1),
                    failures: st.consecutive_failures,
                });
            }
            // Cooldown elapsed: allow one probe through.
            st.opened_at = None;
        }
        Ok(())
    }

    pub fn record_success(&self) {
        self.record_success_key("");
    }

    pub fn record_success_key(&self, key: &str) {
        let mut states = self.lock();
        let st = states.entry(key.to_string()).or_default();
        st.consecutive_failures = 0;
        st.opened_at = None;
    }

    /// Only retryable failures should call this.
    pub fn record_failure(&self) {
        self.record_failure_key("");
    }

    pub fn record_failure_key(&self, key: &str) {
        let mut states = self.lock();
        let st = states.entry(key.to_string()).or_default();
        st.consecutive_failures = st.consecutive_failures.saturating_add(1);
        if st.consecutive_failures >= self.config.threshold && st.opened_at.is_none() {
            st.opened_at = Some(Instant::now());
        }
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, HashMap<String, State>> {
        self.state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }
}
