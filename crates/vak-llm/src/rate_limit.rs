//! Shared, cancelable cooldowns for routes confirmed to use one provider
//! capacity bucket. Adapters provide only an opaque account identity; the
//! agent reports typed rate-limit outcomes and successful probes.

use std::collections::HashMap;
use std::sync::{Arc, Mutex, OnceLock, Weak};
use std::time::Duration;

use tokio_util::sync::CancellationToken;

use crate::LlmError;

#[derive(Default)]
struct State {
    until: Option<tokio::time::Instant>,
    consecutive_limits: u32,
    next_delay: Option<Duration>,
}

#[derive(Clone)]
pub struct RateLimitGate {
    state: Arc<Mutex<State>>,
}

impl RateLimitGate {
    /// Return the process-shared gate for an opaque rate-limit identity.
    /// The registry keeps weak references so credential rotation does not
    /// accumulate old gates for the life of the process.
    pub fn for_key(key: impl Into<String>) -> Self {
        static GATES: OnceLock<Mutex<HashMap<String, Weak<Mutex<State>>>>> = OnceLock::new();
        let key = key.into();
        let gates = GATES.get_or_init(|| Mutex::new(HashMap::new()));
        let mut gates = gates
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        gates.retain(|_, state| state.strong_count() > 0);
        if let Some(state) = gates.get(&key).and_then(Weak::upgrade) {
            return Self { state };
        }
        let state = Arc::new(Mutex::new(State::default()));
        gates.insert(key, Arc::downgrade(&state));
        Self { state }
    }

    /// Wait until the provider's last rate-limit window ends. Returns true
    /// when this request encountered a cooldown, so only a successful probe
    /// can clear a learned backoff.
    pub async fn wait(&self, cancel: &CancellationToken) -> Result<bool, LlmError> {
        let mut waited = false;
        loop {
            let until = self
                .state
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .until;
            let Some(until) = until else {
                return Ok(waited);
            };
            let now = tokio::time::Instant::now();
            if until <= now {
                return Ok(true);
            }
            waited = true;
            tokio::select! {
                _ = cancel.cancelled() => return Err(LlmError::Aborted { partial: None }),
                _ = tokio::time::sleep_until(until) => {}
            }
        }
    }

    /// Share a server-directed cooldown, or learn an increasing delay when
    /// the provider returned a rate limit without structured timing.
    pub fn observe_limit(&self, server_delay: Option<Duration>, retry_backoff: Duration) {
        let now = tokio::time::Instant::now();
        let mut state = self
            .state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let delay = if let Some(server_delay) = server_delay {
            state.next_delay = Some(server_delay);
            server_delay
        } else {
            let previous = state.next_delay.unwrap_or(retry_backoff);
            let delay = if state.consecutive_limits == 0 {
                retry_backoff.max(previous)
            } else {
                retry_backoff
                    .max(previous)
                    .saturating_mul(2)
                    .min(Duration::from_secs(30))
            };
            state.next_delay = Some(delay);
            delay
        };
        state.consecutive_limits = state.consecutive_limits.saturating_add(1);
        state.until = Some(
            now.checked_add(delay)
                .unwrap_or_else(|| now + Duration::from_secs(30)),
        );
    }

    /// A successful request that actually waited is evidence that the
    /// cooldown has cleared. An unrelated concurrent success is not.
    pub fn record_probe_success(&self, waited: bool) {
        if !waited {
            return;
        }
        let mut state = self
            .state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        state.until = None;
        state.consecutive_limits = 0;
        state.next_delay = None;
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]

    use super::RateLimitGate;
    use std::time::Duration;

    #[tokio::test]
    async fn same_capacity_key_shares_cooldown_and_cancel_ends_wait() {
        let key = test_key("capacity");
        let a = RateLimitGate::for_key(key.clone());
        let b = RateLimitGate::for_key(key);
        let cancel = tokio_util::sync::CancellationToken::new();
        a.observe_limit(Some(Duration::from_secs(5)), Duration::from_millis(1));
        let waiting_cancel = cancel.clone();
        let waiter = tokio::spawn(async move { b.wait(&waiting_cancel).await });
        cancel.cancel();
        assert!(matches!(
            waiter.await.unwrap(),
            Err(crate::LlmError::Aborted { .. })
        ));
    }

    #[tokio::test]
    async fn rate_limit_without_hint_learns_and_probe_success_resets() {
        let gate = RateLimitGate::for_key(test_key("learning"));
        gate.observe_limit(None, Duration::from_millis(1));
        assert!(
            gate.wait(&tokio_util::sync::CancellationToken::new())
                .await
                .unwrap()
        );
        gate.observe_limit(None, Duration::from_millis(1));
        assert!(
            gate.wait(&tokio_util::sync::CancellationToken::new())
                .await
                .unwrap()
        );
        gate.record_probe_success(true);
        assert!(
            !gate
                .wait(&tokio_util::sync::CancellationToken::new())
                .await
                .unwrap()
        );
    }

    fn test_key(label: &str) -> String {
        use std::sync::atomic::{AtomicU64, Ordering};
        static NEXT: AtomicU64 = AtomicU64::new(0);
        format!("test-{label}-{}", NEXT.fetch_add(1, Ordering::Relaxed))
    }
}
