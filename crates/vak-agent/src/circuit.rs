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
    next_probe_token: u64,
    probe_in_flight: Option<u64>,
}

#[derive(Debug)]
pub struct CircuitBreaker {
    config: CircuitBreakerConfig,
    state: Mutex<HashMap<String, State>>,
}

/// An admitted provider leg. A half-open reservation belongs to this value,
/// so leaving the call path before it settles releases the reservation.
#[must_use = "keep the permit alive for the admitted provider leg"]
#[derive(Debug)]
pub struct CircuitPermit<'a> {
    breaker: &'a CircuitBreaker,
    key: String,
    probe_token: Option<u64>,
}

impl CircuitPermit<'_> {
    /// Record a successful provider response and close/reset the circuit.
    pub fn record_success(&mut self) {
        if let Some(token) = self.probe_token {
            self.breaker.record_probe_success_key(&self.key, token);
        } else {
            self.breaker.record_success_key(&self.key);
        }
    }

    /// Record a retryable provider failure.
    pub fn record_failure(&mut self) {
        if let Some(token) = self.probe_token {
            self.breaker.record_probe_failure_key(&self.key, token);
        } else {
            self.breaker.record_failure_key(&self.key);
        }
    }
}

impl Drop for CircuitPermit<'_> {
    fn drop(&mut self) {
        let Some(token) = self.probe_token.take() else {
            return;
        };
        let mut states = self.breaker.lock();
        if let Some(state) = states.get_mut(&self.key) {
            // Keep the expired opened_at marker. A later caller can reserve a
            // fresh probe, while dropping this permit stays neutral.
            if state.probe_in_flight == Some(token) {
                state.probe_in_flight = None;
            }
        }
    }
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
    pub fn check(&self) -> Result<CircuitPermit<'_>, CircuitOpen> {
        self.check_key("")
    }

    /// Check one provider/key circuit. Provider names are used by the agent
    /// as the stable health domain; an empty key means the process-global
    /// helper semantics for callers that do not have a route identity.
    pub fn check_key(&self, key: &str) -> Result<CircuitPermit<'_>, CircuitOpen> {
        let mut states = self.lock();
        let st = states.entry(key.to_string()).or_default();
        if st.probe_in_flight.is_some() {
            return Err(CircuitOpen {
                remaining_secs: 1,
                failures: st.consecutive_failures,
            });
        }
        let mut probe_token = None;
        if let Some(opened_at) = st.opened_at {
            let elapsed = opened_at.elapsed();
            if elapsed < self.config.cooldown {
                return Err(CircuitOpen {
                    remaining_secs: (self.config.cooldown - elapsed).as_secs().max(1),
                    failures: st.consecutive_failures,
                });
            }
            // Cooldown elapsed: reserve exactly one half-open probe. Other
            // callers remain fail-closed until that probe settles.
            let Some(token) = st.next_probe_token.checked_add(1) else {
                return Err(CircuitOpen {
                    remaining_secs: 1,
                    failures: st.consecutive_failures,
                });
            };
            st.next_probe_token = token;
            st.probe_in_flight = Some(token);
            probe_token = Some(token);
        }
        Ok(CircuitPermit {
            breaker: self,
            key: key.to_string(),
            probe_token,
        })
    }

    pub fn record_success(&self) {
        self.record_success_key("");
    }

    pub fn record_success_key(&self, key: &str) {
        let mut states = self.lock();
        let st = states.entry(key.to_string()).or_default();
        st.consecutive_failures = 0;
        st.opened_at = None;
        st.probe_in_flight = None;
    }

    /// Only retryable failures should call this.
    pub fn record_failure(&self) {
        self.record_failure_key("");
    }

    pub fn record_failure_key(&self, key: &str) {
        let mut states = self.lock();
        let st = states.entry(key.to_string()).or_default();
        let was_probe = st.probe_in_flight.is_some();
        st.consecutive_failures = st.consecutive_failures.saturating_add(1);
        if was_probe || (st.consecutive_failures >= self.config.threshold && st.opened_at.is_none())
        {
            st.opened_at = Some(Instant::now());
        }
        st.probe_in_flight = None;
    }

    fn record_probe_success_key(&self, key: &str, token: u64) {
        let mut states = self.lock();
        let Some(st) = states.get_mut(key) else {
            return;
        };
        if st.next_probe_token != token {
            return;
        }
        st.consecutive_failures = 0;
        st.opened_at = None;
        st.probe_in_flight = None;
    }

    fn record_probe_failure_key(&self, key: &str, token: u64) {
        let mut states = self.lock();
        let Some(st) = states.get_mut(key) else {
            return;
        };
        if st.next_probe_token != token {
            return;
        }
        let was_probe = st.probe_in_flight == Some(token);
        st.consecutive_failures = st.consecutive_failures.saturating_add(1);
        if was_probe || (st.consecutive_failures >= self.config.threshold && st.opened_at.is_none())
        {
            st.opened_at = Some(Instant::now());
        }
        st.probe_in_flight = None;
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, HashMap<String, State>> {
        self.state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }
}

#[cfg(test)]
#[allow(clippy::expect_used)]
mod tests {
    use super::*;
    use std::sync::{Arc, mpsc};

    fn open_breaker() -> CircuitBreaker {
        let breaker = CircuitBreaker::new(CircuitBreakerConfig {
            threshold: 1,
            cooldown: Duration::ZERO,
        });
        breaker.record_failure_key("provider");
        breaker
    }

    fn expire_cooldown(breaker: &CircuitBreaker, key: &str) {
        let opened_at = Instant::now() - breaker.config.cooldown - Duration::from_secs(1);
        let mut states = breaker.lock();
        let state = states.get_mut(key).expect("open state");
        state.opened_at = Some(opened_at);
    }

    #[test]
    fn dropping_an_abandoned_probe_allows_a_new_probe() {
        let breaker = open_breaker();
        let permit = breaker.check_key("provider").expect("probe admission");
        assert!(breaker.check_key("provider").is_err());

        drop(permit);

        assert!(breaker.check_key("provider").is_ok());
    }

    #[test]
    fn only_one_concurrent_probe_is_admitted() {
        let breaker = Arc::new(open_breaker());
        let (admitted_tx, admitted_rx) = mpsc::channel();
        let (release_tx, release_rx) = mpsc::channel();
        let worker_breaker = Arc::clone(&breaker);
        let worker = std::thread::spawn(move || {
            let permit = worker_breaker
                .check_key("provider")
                .expect("probe admission");
            admitted_tx.send(()).expect("admission signal");
            release_rx.recv().expect("release signal");
            drop(permit);
        });

        admitted_rx.recv().expect("admission signal");
        assert!(breaker.check_key("provider").is_err());
        release_tx.send(()).expect("release signal");
        worker.join().expect("probe worker");

        assert!(breaker.check_key("provider").is_ok());
    }

    #[test]
    fn failed_probe_reopens_the_cooldown() {
        let breaker = CircuitBreaker::new(CircuitBreakerConfig {
            threshold: 1,
            cooldown: Duration::from_secs(60),
        });
        breaker.record_failure_key("provider");
        {
            let mut states = breaker.lock();
            let state = states.get_mut("provider").expect("open state");
            state.opened_at = Some(Instant::now() - Duration::from_secs(61));
        }

        let mut permit = breaker.check_key("provider").expect("probe admission");
        permit.record_failure();

        assert!(breaker.check_key("provider").is_err());
    }

    #[test]
    fn success_resets_failures_and_allows_normal_admission() {
        let breaker = open_breaker();
        let mut permit = breaker.check_key("provider").expect("probe admission");
        permit.record_success();

        let permit = breaker.check_key("provider").expect("closed admission");
        drop(permit);
        let states = breaker.lock();
        let state = states.get("provider").expect("provider state");
        assert_eq!(state.consecutive_failures, 0);
        assert!(state.opened_at.is_none());
    }

    #[test]
    fn dropping_a_stale_probe_does_not_release_a_newer_probe() {
        let breaker = CircuitBreaker::new(CircuitBreakerConfig {
            threshold: 1,
            cooldown: Duration::ZERO,
        });
        let mut old_closed_permit = breaker.check_key("provider").expect("closed admission");
        breaker.record_failure_key("provider");
        expire_cooldown(&breaker, "provider");
        let old_probe = breaker.check_key("provider").expect("old probe");

        old_closed_permit.record_success();
        breaker.record_failure_key("provider");
        expire_cooldown(&breaker, "provider");
        let new_probe = breaker.check_key("provider").expect("new probe");

        drop(old_probe);

        assert!(breaker.check_key("provider").is_err());
        drop(new_probe);
    }

    #[test]
    fn stale_probe_outcomes_cannot_settle_a_newer_probe() {
        let breaker = CircuitBreaker::new(CircuitBreakerConfig {
            threshold: 1,
            cooldown: Duration::ZERO,
        });
        let mut old_closed_permit = breaker.check_key("provider").expect("closed admission");
        breaker.record_failure_key("provider");
        expire_cooldown(&breaker, "provider");
        let mut old_probe = breaker.check_key("provider").expect("old probe");

        old_closed_permit.record_success();
        breaker.record_failure_key("provider");
        expire_cooldown(&breaker, "provider");
        let new_probe = breaker.check_key("provider").expect("new probe");
        let failures_before_stale_result = breaker
            .lock()
            .get("provider")
            .expect("provider state")
            .consecutive_failures;

        old_probe.record_failure();
        old_probe.record_success();

        let states = breaker.lock();
        let state = states.get("provider").expect("provider state");
        assert_eq!(state.consecutive_failures, failures_before_stale_result);
        assert!(state.probe_in_flight.is_some());
        drop(states);
        assert!(breaker.check_key("provider").is_err());
        drop(new_probe);
    }

    #[test]
    fn old_retry_results_cannot_change_a_newer_probe() {
        let breaker = CircuitBreaker::new(CircuitBreakerConfig {
            threshold: 1,
            cooldown: Duration::from_secs(60),
        });
        breaker.record_failure_key("provider");
        expire_cooldown(&breaker, "provider");
        let mut permit = breaker.check_key("provider").expect("probe admission");

        permit.record_failure();
        expire_cooldown(&breaker, "provider");
        let new_probe = breaker.check_key("provider").expect("new probe");
        let failures_before_old_retry = breaker
            .lock()
            .get("provider")
            .expect("provider state")
            .consecutive_failures;

        permit.record_success();
        permit.record_failure();

        assert!(breaker.check_key("provider").is_err());
        let states = breaker.lock();
        let state = states.get("provider").expect("provider state");
        assert_eq!(state.consecutive_failures, failures_before_old_retry);
        assert!(state.probe_in_flight.is_some());
        drop(states);
        drop(new_probe);
    }

    #[test]
    fn retry_success_can_still_reset_its_probe_before_a_new_probe() {
        let breaker = CircuitBreaker::new(CircuitBreakerConfig {
            threshold: 1,
            cooldown: Duration::from_secs(60),
        });
        breaker.record_failure_key("provider");
        expire_cooldown(&breaker, "provider");
        let mut permit = breaker.check_key("provider").expect("probe admission");

        permit.record_failure();
        permit.record_success();

        assert!(breaker.check_key("provider").is_ok());
    }
}
