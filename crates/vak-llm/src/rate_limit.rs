//! Shared, cancelable cooldowns for routes confirmed to use one provider
//! capacity bucket. Adapters provide only an opaque account identity; the
//! agent reports typed rate-limit outcomes and successful probes.

use std::collections::{HashMap, VecDeque};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::Duration;

use tokio_util::sync::CancellationToken;

use crate::{LlmError, Provider, Usage};

#[derive(Default)]
struct State {
    until: Option<tokio::time::Instant>,
    consecutive_limits: u32,
    next_delay: Option<Duration>,
    active: usize,
    queued: usize,
    next_quota_ticket: u64,
    quota_waiters: VecDeque<u64>,
    dispatch_slots: OnceLock<Arc<tokio::sync::Semaphore>>,
    capacity_changed: OnceLock<Arc<tokio::sync::Notify>>,
    observation_serial: OnceLock<Arc<Mutex<()>>>,
    next_observation_sequence: u64,
    last_observation_sequence: u64,
    last_observed_at: Option<tokio::time::Instant>,
    daily_observed_at: Option<tokio::time::Instant>,
    tokens_remaining: Option<u64>,
    tokens_limit: Option<u64>,
    input_tokens_remaining: Option<u64>,
    input_tokens_limit: Option<u64>,
    output_tokens_remaining: Option<u64>,
    output_tokens_limit: Option<u64>,
    requests_remaining: Option<u64>,
    requests_limit: Option<u64>,
    reset_after: Option<Duration>,
    daily_remaining: Option<u64>,
    daily_limit: Option<u64>,
    daily_input_remaining: Option<u64>,
    daily_input_limit: Option<u64>,
    daily_output_remaining: Option<u64>,
    daily_output_limit: Option<u64>,
    daily_requests_remaining: Option<u64>,
    daily_requests_limit: Option<u64>,
    daily_reset_at: Option<tokio::time::Instant>,
    daily_input_reset_at: Option<tokio::time::Instant>,
    daily_output_reset_at: Option<tokio::time::Instant>,
    daily_requests_reset_at: Option<tokio::time::Instant>,
    last_used_at: Option<tokio::time::Instant>,
    model_scoped: bool,
    estimated_tokens_used: u64,
    estimated_input_tokens_used: u64,
    estimated_output_tokens_used: u64,
    estimated_requests_used: u64,
    estimated_daily_requests_used: u64,
    estimated_daily_tokens_used: u64,
    estimated_daily_input_tokens_used: u64,
    estimated_daily_output_tokens_used: u64,
    reservations: HashMap<u64, Reservation>,
    next_reservation: u64,
    reset_at: Option<tokio::time::Instant>,
    requests_reset_at: Option<tokio::time::Instant>,
    input_reset_at: Option<tokio::time::Instant>,
    output_reset_at: Option<tokio::time::Instant>,
}

struct Reservation {
    combined_tokens: u64,
    input_tokens: u64,
    output_tokens: u64,
    requests: u64,
    expires_at: tokio::time::Instant,
}

/// Removes a quota waiter if its async reservation future is cancelled.
struct QuotaQueueTicket {
    state: Arc<Mutex<State>>,
    ticket: u64,
    active: bool,
}

impl QuotaQueueTicket {
    fn disarm(&mut self) {
        self.active = false;
    }
}

impl Drop for QuotaQueueTicket {
    fn drop(&mut self) {
        if !self.active {
            return;
        }
        let changed = {
            let mut state = self
                .state
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            state.quota_waiters.retain(|queued| *queued != self.ticket);
            state.queued = state.queued.saturating_sub(1);
            state
                .capacity_changed
                .get_or_init(|| Arc::new(tokio::sync::Notify::new()))
                .clone()
        };
        changed.notify_waiters();
    }
}

/// Keeps Traffic's account queue count correct when a waiter future is
/// cancelled before its async body reaches normal cleanup.
struct DispatchQueueTicket {
    state: Arc<Mutex<State>>,
    active: bool,
}

impl DispatchQueueTicket {
    fn disarm(&mut self) {
        self.active = false;
    }
}

impl Drop for DispatchQueueTicket {
    fn drop(&mut self) {
        if !self.active {
            return;
        }
        let mut state = self
            .state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        state.queued = state.queued.saturating_sub(1);
    }
}

#[derive(Debug, Clone, Default, serde::Serialize)]
pub struct CapacityObservation {
    pub tokens_remaining: Option<u64>,
    pub tokens_limit: Option<u64>,
    pub input_tokens_remaining: Option<u64>,
    pub input_tokens_limit: Option<u64>,
    pub output_tokens_remaining: Option<u64>,
    pub output_tokens_limit: Option<u64>,
    pub input_tokens_reset_after_secs: Option<u64>,
    pub output_tokens_reset_after_secs: Option<u64>,
    pub requests_remaining: Option<u64>,
    pub requests_limit: Option<u64>,
    pub reset_after_secs: Option<u64>,
    pub requests_reset_after_secs: Option<u64>,
    pub daily_remaining: Option<u64>,
    pub daily_limit: Option<u64>,
    pub daily_input_remaining: Option<u64>,
    pub daily_input_limit: Option<u64>,
    pub daily_output_remaining: Option<u64>,
    pub daily_output_limit: Option<u64>,
    pub daily_input_reset_after_secs: Option<u64>,
    pub daily_output_reset_after_secs: Option<u64>,
    pub daily_requests_remaining: Option<u64>,
    pub daily_requests_limit: Option<u64>,
    pub daily_requests_reset_after_secs: Option<u64>,
    pub daily_reset_after_secs: Option<u64>,
}

impl CapacityObservation {
    fn is_empty(&self) -> bool {
        self.tokens_remaining.is_none()
            && self.tokens_limit.is_none()
            && self.input_tokens_remaining.is_none()
            && self.input_tokens_limit.is_none()
            && self.output_tokens_remaining.is_none()
            && self.output_tokens_limit.is_none()
            && self.input_tokens_reset_after_secs.is_none()
            && self.output_tokens_reset_after_secs.is_none()
            && self.requests_remaining.is_none()
            && self.requests_limit.is_none()
            && self.reset_after_secs.is_none()
            && self.requests_reset_after_secs.is_none()
            && self.daily_remaining.is_none()
            && self.daily_limit.is_none()
            && self.daily_input_remaining.is_none()
            && self.daily_input_limit.is_none()
            && self.daily_input_reset_after_secs.is_none()
            && self.daily_output_remaining.is_none()
            && self.daily_output_limit.is_none()
            && self.daily_output_reset_after_secs.is_none()
            && self.daily_requests_remaining.is_none()
            && self.daily_requests_limit.is_none()
            && self.daily_requests_reset_after_secs.is_none()
            && self.daily_reset_after_secs.is_none()
    }
}

fn has_fresh_exhausted_capacity(state: &State, now: tokio::time::Instant) -> bool {
    let reservations = state
        .reservations
        .values()
        .filter(|reservation| reservation.expires_at > now);
    let mut reserved_tokens = 0u64;
    let mut reserved_input = 0u64;
    let mut reserved_output = 0u64;
    let mut reserved_requests = 0u64;
    for reservation in reservations {
        reserved_tokens = reserved_tokens.saturating_add(reservation.combined_tokens);
        reserved_input = reserved_input.saturating_add(reservation.input_tokens);
        reserved_output = reserved_output.saturating_add(reservation.output_tokens);
        reserved_requests = reserved_requests.saturating_add(reservation.requests);
    }
    let exhausted = |remaining: Option<u64>,
                     limit: Option<u64>,
                     used: u64,
                     reset_at: Option<tokio::time::Instant>,
                     reserved: u64| {
        let (remaining, used) = if reset_at.is_some_and(|reset| reset <= now) {
            (limit, 0)
        } else {
            (remaining, used)
        };
        remaining
            .is_some_and(|remaining| remaining.saturating_sub(used).saturating_sub(reserved) == 0)
    };
    let short_window_fresh = state
        .last_observed_at
        .is_some_and(|observed| now.duration_since(observed) <= Duration::from_secs(300));
    let daily_fresh = state
        .daily_observed_at
        .is_some_and(|observed| now.duration_since(observed) <= Duration::from_secs(24 * 60 * 60));
    (short_window_fresh
        && (exhausted(
            state.tokens_remaining,
            state.tokens_limit,
            state.estimated_tokens_used,
            state.reset_at,
            reserved_tokens,
        ) || exhausted(
            state.input_tokens_remaining,
            state.input_tokens_limit,
            state.estimated_input_tokens_used,
            state.input_reset_at,
            reserved_input,
        ) || exhausted(
            state.output_tokens_remaining,
            state.output_tokens_limit,
            state.estimated_output_tokens_used,
            state.output_reset_at,
            reserved_output,
        ) || exhausted(
            state.requests_remaining,
            state.requests_limit,
            state.estimated_requests_used,
            state.requests_reset_at,
            reserved_requests,
        )))
        || (daily_fresh
            && (exhausted(
                state.daily_remaining,
                state.daily_limit,
                state.estimated_daily_tokens_used,
                state.daily_reset_at,
                reserved_tokens,
            ) || exhausted(
                state.daily_input_remaining,
                state.daily_input_limit,
                state.estimated_daily_input_tokens_used,
                state.daily_input_reset_at,
                reserved_input,
            ) || exhausted(
                state.daily_output_remaining,
                state.daily_output_limit,
                state.estimated_daily_output_tokens_used,
                state.daily_output_reset_at,
                reserved_output,
            ) || exhausted(
                state.daily_requests_remaining,
                state.daily_requests_limit,
                state.estimated_daily_requests_used,
                state.daily_requests_reset_at,
                reserved_requests,
            )))
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct TrafficSnapshot {
    pub state: &'static str,
    pub active: usize,
    pub queued: usize,
    pub retry_after_secs: Option<u64>,
    pub observed_routes: usize,
    pub scope: &'static str,
}

/// Dispatch-order fence for one model and its parent account. A response that
/// arrives after a newer dispatch response cannot roll capacity evidence back.
pub struct CapacityObservationTicket {
    model_gate: RateLimitGate,
    model_sequence: u64,
    account_gate: RateLimitGate,
    account_sequence: u64,
}

impl CapacityObservationTicket {
    pub fn observe_model(&self, observation: CapacityObservation) {
        self.model_gate
            .observe_ordered(self.model_sequence, observation);
    }

    pub fn observe_account(&self, observation: CapacityObservation) {
        self.account_gate
            .observe_ordered(self.account_sequence, observation);
    }
}

pub struct DispatchPermit {
    state: Arc<Mutex<State>>,
    _slot: tokio::sync::OwnedSemaphorePermit,
}

impl Drop for DispatchPermit {
    fn drop(&mut self) {
        let mut state = self
            .state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        state.active = state.active.saturating_sub(1);
    }
}

pub struct QuotaPermit {
    state: Arc<Mutex<State>>,
    reservation_id: u64,
}

/// Shared account/model reservation for one provider dispatch outside a
/// caller that already owns admission (for example, the Agent dispatcher).
/// Dropping an unsettled value keeps dispatched quota conservative; callers
/// may explicitly release it only when dispatch definitely never started.
pub struct RequestAdmission {
    model: Option<QuotaPermit>,
    account: Option<QuotaPermit>,
    _dispatch: DispatchPermit,
    bedrock_mantle: bool,
    dispatch_cancel: CancellationToken,
    deadline_task: Option<tokio::task::JoinHandle<()>>,
}

impl RequestAdmission {
    /// Refresh published capacity, reserve estimated demand and take an
    /// account concurrency slot. `route_provider` is the configured route
    /// name, which distinguishes OpenRouter's free-model account cap and
    /// Bedrock Mantle's split input/output quota semantics.
    pub async fn acquire(
        provider: &dyn Provider,
        route_provider: &str,
        model: &str,
        estimated_input_tokens: u64,
        max_output_tokens: u64,
        cancel: &CancellationToken,
    ) -> Result<Self, LlmError> {
        Self::acquire_inner(
            provider,
            route_provider,
            model,
            estimated_input_tokens,
            max_output_tokens,
            cancel,
        )
        .await
    }

    /// Acquire admission under one total queue-plus-dispatch time budget.
    /// The returned cancellation token must be passed to `Provider::stream`;
    /// its timer remains live until settlement or drop.
    pub async fn acquire_with_timeout(
        provider: &dyn Provider,
        route_provider: &str,
        model: &str,
        estimated_input_tokens: u64,
        max_output_tokens: u64,
        timeout: Duration,
        cancel: &CancellationToken,
    ) -> Result<Self, LlmError> {
        let dispatch_cancel = cancel.child_token();
        let timer_cancel = dispatch_cancel.clone();
        let deadline_task = tokio::spawn(async move {
            tokio::time::sleep(timeout).await;
            timer_cancel.cancel();
        });
        match Self::acquire_inner(
            provider,
            route_provider,
            model,
            estimated_input_tokens,
            max_output_tokens,
            &dispatch_cancel,
        )
        .await
        {
            Ok(mut admission) => {
                admission.dispatch_cancel = dispatch_cancel;
                admission.deadline_task = Some(deadline_task);
                Ok(admission)
            }
            Err(error) => {
                deadline_task.abort();
                Err(error)
            }
        }
    }

    async fn acquire_inner(
        provider: &dyn Provider,
        route_provider: &str,
        model: &str,
        estimated_input_tokens: u64,
        max_output_tokens: u64,
        cancel: &CancellationToken,
    ) -> Result<Self, LlmError> {
        let _ = provider.refresh_capacity(cancel).await;
        let account_key = provider.rate_limit_key();
        let account_gate = RateLimitGate::for_key(account_key.clone());
        let model_gate = RateLimitGate::for_model(account_key, model);
        let bedrock_mantle = route_provider == "bedrock";
        let openrouter_free_model =
            route_provider.starts_with("openrouter") && model.ends_with(":free");
        let model_result = if bedrock_mantle {
            model_gate
                .reserve_bedrock_mantle_demand(estimated_input_tokens, max_output_tokens, cancel)
                .await
        } else {
            model_gate
                .reserve_demand(estimated_input_tokens, max_output_tokens, cancel)
                .await
        };
        let model = model_result?;
        let account_observed = account_gate.has_account_token_observation()
            || (openrouter_free_model && account_gate.has_capacity_observation());
        let account = if account_observed {
            let result = if bedrock_mantle {
                account_gate
                    .reserve_bedrock_mantle_demand(
                        estimated_input_tokens,
                        max_output_tokens,
                        cancel,
                    )
                    .await
            } else {
                account_gate
                    .reserve_demand(estimated_input_tokens, max_output_tokens, cancel)
                    .await
            };
            match result {
                Ok(permit) => Some(permit),
                Err(error) => {
                    let mut model = model;
                    model.release();
                    return Err(error);
                }
            }
        } else {
            None
        };
        let dispatch = match account_gate.admit(cancel).await {
            Ok(permit) => permit,
            Err(error) => {
                let mut model = model;
                model.release();
                if let Some(mut account) = account {
                    account.release();
                }
                return Err(error);
            }
        };
        Ok(Self {
            model: Some(model),
            account,
            _dispatch: dispatch,
            bedrock_mantle,
            dispatch_cancel: cancel.clone(),
            deadline_task: None,
        })
    }

    /// Cancellation token carrying both the caller's cancellation and the
    /// total admission/dispatch deadline, when one was configured.
    pub fn cancellation_token(&self) -> CancellationToken {
        self.dispatch_cancel.clone()
    }

    fn stop_deadline(&mut self) {
        if let Some(task) = self.deadline_task.take() {
            task.abort();
        }
        self.dispatch_cancel.cancel();
    }

    /// Settle both applicable quota scopes from provider-reported usage.
    pub fn settle(&mut self, usage: &Usage) {
        let input = if self.bedrock_mantle {
            usage.input_tokens
        } else {
            usage.prompt_tokens()
        };
        if let Some(permit) = self.model.as_mut() {
            if self.bedrock_mantle {
                permit.settle_bedrock_mantle(input, usage.output_tokens);
            } else {
                permit.settle(input, usage.output_tokens);
            }
        }
        if let Some(permit) = self.account.as_mut() {
            if self.bedrock_mantle {
                permit.settle_bedrock_mantle(input, usage.output_tokens);
            } else {
                permit.settle(input, usage.output_tokens);
            }
        }
        self.stop_deadline();
    }

    /// Undo reservations when the caller can prove no provider dispatch began.
    pub fn release(&mut self) {
        if let Some(permit) = self.model.as_mut() {
            permit.release();
        }
        if let Some(permit) = self.account.as_mut() {
            permit.release();
        }
        self.stop_deadline();
    }

    /// Mark that the caller ended before `Provider::stream` started, so its
    /// quota reservations must not be retained as uncertain provider usage.
    pub fn release_before_dispatch(&mut self) {
        self.release();
    }
}

impl Drop for RequestAdmission {
    fn drop(&mut self) {
        if let Some(task) = self.deadline_task.take() {
            task.abort();
        }
        self.dispatch_cancel.cancel();
    }
}

impl QuotaPermit {
    /// Replace the conservative reservation with observed usage. The usage is
    /// retained for the active provider window so parallel work shares it.
    pub fn settle(&mut self, input_tokens: u64, output_tokens: u64) {
        self.settle_dimensions(
            input_tokens.saturating_add(output_tokens),
            input_tokens,
            output_tokens,
        );
    }

    /// Settle the quota dimensions used by Bedrock Mantle. Mantle charges
    /// non-cached input plus generated output to its input TPM bucket, and
    /// generated output separately to its output TPM bucket.
    pub fn settle_bedrock_mantle(&mut self, input_tokens: u64, output_tokens: u64) {
        self.settle_dimensions(
            input_tokens.saturating_add(output_tokens),
            input_tokens.saturating_add(output_tokens),
            output_tokens,
        );
    }

    fn settle_dimensions(&mut self, combined_tokens: u64, input_tokens: u64, output_tokens: u64) {
        let mut state = self
            .state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if let Some(reservation) = state.reservations.remove(&self.reservation_id) {
            state.estimated_tokens_used =
                state.estimated_tokens_used.saturating_add(combined_tokens);
            state.estimated_input_tokens_used = state
                .estimated_input_tokens_used
                .saturating_add(input_tokens);
            state.estimated_output_tokens_used = state
                .estimated_output_tokens_used
                .saturating_add(output_tokens);
            state.estimated_daily_tokens_used = state
                .estimated_daily_tokens_used
                .saturating_add(combined_tokens);
            state.estimated_daily_input_tokens_used = state
                .estimated_daily_input_tokens_used
                .saturating_add(input_tokens);
            state.estimated_daily_output_tokens_used = state
                .estimated_daily_output_tokens_used
                .saturating_add(output_tokens);
            state.estimated_requests_used = state
                .estimated_requests_used
                .saturating_add(reservation.requests);
            state.estimated_daily_requests_used = state
                .estimated_daily_requests_used
                .saturating_add(reservation.requests);
        }
        drop(state);
        self.notify_capacity_changed();
    }

    fn notify_capacity_changed(&self) {
        self.state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .capacity_changed
            .get_or_init(|| Arc::new(tokio::sync::Notify::new()))
            .notify_waiters();
    }

    /// Release a reservation when dispatch never started.
    pub fn release(&mut self) {
        self.state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .reservations
            .remove(&self.reservation_id);
        self.notify_capacity_changed();
    }

    /// Settle a completed auxiliary call when its provider response has no
    /// token usage object. Keep the conservative reservation as its usage.
    pub fn settle_estimate(&mut self) {
        let mut state = self
            .state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if let Some(reservation) = state.reservations.remove(&self.reservation_id) {
            state.estimated_tokens_used = state
                .estimated_tokens_used
                .saturating_add(reservation.combined_tokens);
            state.estimated_input_tokens_used = state
                .estimated_input_tokens_used
                .saturating_add(reservation.input_tokens);
            state.estimated_output_tokens_used = state
                .estimated_output_tokens_used
                .saturating_add(reservation.output_tokens);
            state.estimated_daily_tokens_used = state
                .estimated_daily_tokens_used
                .saturating_add(reservation.combined_tokens);
            state.estimated_daily_input_tokens_used = state
                .estimated_daily_input_tokens_used
                .saturating_add(reservation.input_tokens);
            state.estimated_daily_output_tokens_used = state
                .estimated_daily_output_tokens_used
                .saturating_add(reservation.output_tokens);
            state.estimated_requests_used = state
                .estimated_requests_used
                .saturating_add(reservation.requests);
            state.estimated_daily_requests_used = state
                .estimated_daily_requests_used
                .saturating_add(reservation.requests);
        }
    }
}

impl Drop for QuotaPermit {
    fn drop(&mut self) {
        // An unsettled dispatch may have reached the provider and consumed
        // capacity. Keep its reservation until the observed window resets.
    }
}

#[derive(Clone)]
pub struct RateLimitGate {
    state: Arc<Mutex<State>>,
}

impl RateLimitGate {
    fn registry() -> &'static Mutex<HashMap<String, Arc<Mutex<State>>>> {
        static GATES: OnceLock<Mutex<HashMap<String, Arc<Mutex<State>>>>> = OnceLock::new();
        GATES.get_or_init(|| Mutex::new(HashMap::new()))
    }
    /// Return the process-shared gate for an opaque rate-limit identity.
    /// Idle records expire when the bounded registry is pruned, so a recent
    /// cooldown survives a turn boundary without retaining rotated keys forever.
    pub fn for_key(key: impl Into<String>) -> Self {
        Self::for_identity(key.into(), false)
    }

    /// A provider quota window that applies to one model within an account.
    /// Protocol aliases for the same credential and model intentionally share it.
    pub fn for_model(account_key: impl Into<String>, model: &str) -> Self {
        Self::for_identity(format!("{}\u{1f}model:{model}", account_key.into()), true)
    }

    /// Start an observation epoch at dispatch time for both shared scopes.
    pub fn capacity_observation_ticket(
        account_key: impl Into<String>,
        model: &str,
    ) -> CapacityObservationTicket {
        let account_key = account_key.into();
        let model_gate = Self::for_model(account_key.clone(), model);
        let account_gate = Self::for_key(account_key);
        let model_sequence = model_gate.next_observation_sequence();
        let account_sequence = account_gate.next_observation_sequence();
        CapacityObservationTicket {
            model_gate,
            model_sequence,
            account_gate,
            account_sequence,
        }
    }

    pub(crate) fn next_observation_sequence(&self) -> u64 {
        let mut state = self
            .state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        state.next_observation_sequence = state.next_observation_sequence.saturating_add(1);
        state.next_observation_sequence
    }

    /// Record model capacity evidence obtained outside a chat provider adapter
    /// (voice, transcription and synthesis use these protocol helpers).
    pub fn observe_model(
        account_key: impl Into<String>,
        model: &str,
        observation: CapacityObservation,
    ) {
        Self::for_model(account_key, model).observe(observation);
    }

    /// Record a provider-wide outcome from a non-chat call using the exact same
    /// opaque account identity as the corresponding chat route.
    pub fn observe_account_limit(
        account_key: impl Into<String>,
        delay: Option<Duration>,
        fallback: Duration,
    ) {
        Self::for_key(account_key).observe_limit(delay, fallback);
    }

    /// Account-level admission permit for a bounded auxiliary provider call.
    pub async fn admit_account(
        account_key: impl Into<String>,
        cancel: &CancellationToken,
    ) -> Result<DispatchPermit, LlmError> {
        Self::for_key(account_key).admit(cancel).await
    }

    /// Reserve demand against one provider/model window from an auxiliary API.
    pub async fn reserve_model(
        account_key: impl Into<String>,
        model: &str,
        estimated_tokens: u64,
        cancel: &CancellationToken,
    ) -> Result<QuotaPermit, LlmError> {
        Self::for_model(account_key, model)
            .reserve_quota(estimated_tokens, cancel)
            .await
    }

    fn for_identity(key: String, model_scoped: bool) -> Self {
        let gates = Self::registry();
        let mut gates = gates
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if gates.len() > 512 {
            let now = tokio::time::Instant::now();
            gates.retain(|_, state| {
                let s = state
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner);
                s.active > 0
                    || s.queued > 0
                    || s.until.is_some_and(|until| until > now)
                    || s.last_used_at
                        .is_some_and(|used| now.duration_since(used) < Duration::from_secs(900))
                    || s.reset_at.is_some_and(|reset| reset > now)
            });
        }
        if let Some(state) = gates.get(&key).cloned() {
            let mut state_guard = state
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            state_guard.last_used_at = Some(tokio::time::Instant::now());
            state_guard
                .dispatch_slots
                .get_or_init(|| Arc::new(tokio::sync::Semaphore::new(8)));
            state_guard
                .capacity_changed
                .get_or_init(|| Arc::new(tokio::sync::Notify::new()));
            state_guard
                .observation_serial
                .get_or_init(|| Arc::new(Mutex::new(())));
            drop(state_guard);
            return Self { state };
        }
        let state = Arc::new(Mutex::new(State::default()));
        {
            let mut state = state
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            state.last_used_at = Some(tokio::time::Instant::now());
            state.model_scoped = model_scoped;
            state
                .dispatch_slots
                .get_or_init(|| Arc::new(tokio::sync::Semaphore::new(8)));
            state
                .capacity_changed
                .get_or_init(|| Arc::new(tokio::sync::Notify::new()));
            state
                .observation_serial
                .get_or_init(|| Arc::new(Mutex::new(())));
        }
        gates.insert(key, state.clone());
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

    /// Admit a dispatch after any known cooldown. This first process-shared
    /// scheduler bounds simultaneous account work; provider observations refine
    /// rate windows as adapters report them.
    pub async fn admit(&self, cancel: &CancellationToken) -> Result<DispatchPermit, LlmError> {
        let slots = {
            let mut state = self
                .state
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            state.queued = state.queued.saturating_add(1);
            state
                .dispatch_slots
                .get_or_init(|| Arc::new(tokio::sync::Semaphore::new(8)))
                .clone()
        };
        let mut queue_ticket = DispatchQueueTicket {
            state: self.state.clone(),
            active: true,
        };
        async {
            loop {
                let until = {
                    let mut state = self
                        .state
                        .lock()
                        .unwrap_or_else(std::sync::PoisonError::into_inner);
                    let now = tokio::time::Instant::now();
                    if state.until.is_some_and(|until| until <= now) {
                        state.until = None;
                    }
                    state.until
                };
                if let Some(until) = until {
                    tokio::select! {
                        _ = cancel.cancelled() => return Err(LlmError::Aborted { partial: None }),
                        _ = tokio::time::sleep_until(until) => {}
                    }
                    continue;
                }
                let slot = tokio::select! {
                    _ = cancel.cancelled() => return Err(LlmError::Aborted { partial: None }),
                    result = slots.clone().acquire_owned() => match result {
                        Ok(permit) => permit,
                        Err(_) => return Err(LlmError::Aborted { partial: None }),
                    }
                };
                let cooldown = {
                    let mut state = self
                        .state
                        .lock()
                        .unwrap_or_else(std::sync::PoisonError::into_inner);
                    let now = tokio::time::Instant::now();
                    if let Some(until) = state.until.filter(|until| *until > now) {
                        Some(until)
                    } else {
                        if state.until.is_some_and(|until| until <= now) {
                            state.until = None;
                        }
                        state.active += 1;
                        state.queued = state.queued.saturating_sub(1);
                        queue_ticket.disarm();
                        None
                    }
                };
                if let Some(until) = cooldown {
                    drop(slot);
                    tokio::select! {
                        _ = cancel.cancelled() => return Err(LlmError::Aborted { partial: None }),
                        _ = tokio::time::sleep_until(until) => {}
                    }
                    continue;
                }
                return Ok(DispatchPermit {
                    state: self.state.clone(),
                    _slot: slot,
                });
            }
        }
        .await
    }

    /// Reserve estimated token and request demand against the latest observed
    /// model window. Unknown limits remain unbounded here; the account gate
    /// still caps concurrent dispatches and provider 429s teach its cooldown.
    pub async fn reserve_quota(
        &self,
        estimated_tokens: u64,
        cancel: &CancellationToken,
    ) -> Result<QuotaPermit, LlmError> {
        self.reserve_demand(estimated_tokens, 0, cancel).await
    }

    /// Reserve separate input and output token demand when a provider has
    /// independent bucket dimensions, while continuing to enforce combined
    /// token windows from providers that expose a single total bucket.
    pub async fn reserve_demand(
        &self,
        estimated_input_tokens: u64,
        estimated_output_tokens: u64,
        cancel: &CancellationToken,
    ) -> Result<QuotaPermit, LlmError> {
        let combined_tokens = estimated_input_tokens.saturating_add(estimated_output_tokens);
        self.reserve_dimensions(
            combined_tokens,
            estimated_input_tokens,
            estimated_output_tokens,
            cancel,
        )
        .await
    }

    /// Reserve against Mantle's evaluation rule: non-cached input plus the
    /// requested output cap is checked against input TPM; that same output
    /// cap is also reserved against output TPM. A separate combined bucket,
    /// when an adapter reports one, reserves input plus max output once.
    pub async fn reserve_bedrock_mantle_demand(
        &self,
        estimated_input_tokens: u64,
        max_output_tokens: u64,
        cancel: &CancellationToken,
    ) -> Result<QuotaPermit, LlmError> {
        let input_quota = estimated_input_tokens.saturating_add(max_output_tokens);
        let combined_tokens = input_quota;
        self.reserve_dimensions(combined_tokens, input_quota, max_output_tokens, cancel)
            .await
    }

    async fn reserve_dimensions(
        &self,
        combined_tokens: u64,
        estimated_input_tokens: u64,
        estimated_output_tokens: u64,
        cancel: &CancellationToken,
    ) -> Result<QuotaPermit, LlmError> {
        let estimated_tokens = combined_tokens;
        let ticket = {
            let mut state = self
                .state
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            state.queued = state.queued.saturating_add(1);
            state.next_quota_ticket = state.next_quota_ticket.wrapping_add(1).max(1);
            let ticket = state.next_quota_ticket;
            state.quota_waiters.push_back(ticket);
            ticket
        };
        let mut queue_ticket = QuotaQueueTicket {
            state: self.state.clone(),
            ticket,
            active: true,
        };
        let result = async {
            loop {
                let changed = self
                    .state
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner)
                    .capacity_changed
                    .get_or_init(|| Arc::new(tokio::sync::Notify::new()))
                    .clone();
                let notified = changed.notified();
                tokio::pin!(notified);
                notified.as_mut().enable();
                let wait_until = {
                    let mut state = self
                        .state
                        .lock()
                        .unwrap_or_else(std::sync::PoisonError::into_inner);
                    let now = tokio::time::Instant::now();
                    state
                        .reservations
                        .retain(|_, reservation| reservation.expires_at > now);
                    if state.last_observed_at.is_some_and(|observed| {
                        now.duration_since(observed) > Duration::from_secs(300)
                    }) {
                        state.tokens_remaining = None;
                        state.tokens_limit = None;
                        state.input_tokens_remaining = None;
                        state.input_tokens_limit = None;
                        state.output_tokens_remaining = None;
                        state.output_tokens_limit = None;
                        state.requests_remaining = None;
                        state.requests_limit = None;
                        state.requests_reset_at = None;
                        state.reset_at = None;
                        state.input_reset_at = None;
                        state.output_reset_at = None;
                        state.last_observed_at = None;
                        state.estimated_tokens_used = 0;
                        state.estimated_input_tokens_used = 0;
                        state.estimated_output_tokens_used = 0;
                        state.estimated_requests_used = 0;
                    }
                    if state.daily_observed_at.is_some_and(|observed| {
                        now.duration_since(observed) > Duration::from_secs(24 * 60 * 60)
                    }) {
                        state.daily_remaining = None;
                        state.daily_limit = None;
                        state.daily_input_remaining = None;
                        state.daily_input_limit = None;
                        state.daily_output_remaining = None;
                        state.daily_output_limit = None;
                        state.daily_requests_remaining = None;
                        state.daily_requests_limit = None;
                        state.daily_reset_at = None;
                        state.daily_input_reset_at = None;
                        state.daily_output_reset_at = None;
                        state.daily_requests_reset_at = None;
                        state.daily_observed_at = None;
                        state.estimated_daily_tokens_used = 0;
                        state.estimated_daily_input_tokens_used = 0;
                        state.estimated_daily_output_tokens_used = 0;
                        state.estimated_daily_requests_used = 0;
                    }
                    if state.reset_at.is_some_and(|reset| reset <= now) {
                        state.reset_at = None;
                        state.estimated_tokens_used = 0;
                        state.tokens_remaining = state.tokens_limit;
                    }
                    if state.requests_reset_at.is_some_and(|reset| reset <= now) {
                        state.requests_reset_at = None;
                        state.estimated_requests_used = 0;
                        state.requests_remaining = state.requests_limit;
                    }
                    if state.input_reset_at.is_some_and(|reset| reset <= now) {
                        state.input_reset_at = None;
                        state.estimated_input_tokens_used = 0;
                        state.input_tokens_remaining = state.input_tokens_limit;
                    }
                    if state.output_reset_at.is_some_and(|reset| reset <= now) {
                        state.output_reset_at = None;
                        state.estimated_output_tokens_used = 0;
                        state.output_tokens_remaining = state.output_tokens_limit;
                    }
                    if state.daily_reset_at.is_some_and(|reset| reset <= now) {
                        state.daily_reset_at = None;
                        state.estimated_daily_tokens_used = 0;
                        state.daily_remaining = state.daily_limit;
                    }
                    if state.daily_input_reset_at.is_some_and(|reset| reset <= now) {
                        state.daily_input_reset_at = None;
                        state.estimated_daily_input_tokens_used = 0;
                        state.daily_input_remaining = state.daily_input_limit;
                    }
                    if state
                        .daily_output_reset_at
                        .is_some_and(|reset| reset <= now)
                    {
                        state.daily_output_reset_at = None;
                        state.estimated_daily_output_tokens_used = 0;
                        state.daily_output_remaining = state.daily_output_limit;
                    }
                    if state
                        .daily_requests_reset_at
                        .is_some_and(|reset| reset <= now)
                    {
                        state.daily_requests_reset_at = None;
                        state.estimated_daily_requests_used = 0;
                        state.daily_requests_remaining = state.daily_requests_limit;
                    }
                    let mut too_large = Vec::new();
                    let mut check_token_limit = |label: &str, demand: u64, limit: Option<u64>| {
                        if let Some(limit) = limit
                            && demand > limit
                        {
                            too_large
                                .push(format!("{label}: demand {demand} exceeds limit {limit}"));
                        }
                    };
                    check_token_limit(
                        "short-window combined tokens",
                        estimated_tokens,
                        state.tokens_limit,
                    );
                    check_token_limit(
                        "short-window input tokens",
                        estimated_input_tokens,
                        state.input_tokens_limit,
                    );
                    check_token_limit(
                        "short-window output tokens",
                        estimated_output_tokens,
                        state.output_tokens_limit,
                    );
                    check_token_limit("daily combined tokens", estimated_tokens, state.daily_limit);
                    check_token_limit(
                        "daily input tokens",
                        estimated_input_tokens,
                        state.daily_input_limit,
                    );
                    check_token_limit(
                        "daily output tokens",
                        estimated_output_tokens,
                        state.daily_output_limit,
                    );
                    if state.requests_limit.is_some_and(|limit| limit < 1) {
                        too_large.push("short-window requests: demand 1 exceeds limit 0".into());
                    }
                    if state.daily_requests_limit.is_some_and(|limit| limit < 1) {
                        too_large.push("daily requests: demand 1 exceeds limit 0".into());
                    }
                    if !too_large.is_empty() {
                        return Err(LlmError::Context(format!(
                            "request cannot fit an observed provider quota window ({})",
                            too_large.join("; ")
                        )));
                    }
                    let reserved_tokens = state
                        .reservations
                        .values()
                        .map(|reservation| reservation.combined_tokens)
                        .sum::<u64>();
                    let reserved_input = state
                        .reservations
                        .values()
                        .map(|reservation| reservation.input_tokens)
                        .sum::<u64>();
                    let reserved_output = state
                        .reservations
                        .values()
                        .map(|reservation| reservation.output_tokens)
                        .sum::<u64>();
                    let reserved_requests = state
                        .reservations
                        .values()
                        .map(|reservation| reservation.requests)
                        .sum::<u64>();
                    let available_tokens = state.tokens_remaining.map(|remaining| {
                        remaining
                            .saturating_sub(state.estimated_tokens_used)
                            .saturating_sub(reserved_tokens)
                    });
                    let daily_tokens_available = state.daily_remaining.map(|remaining| {
                        remaining
                            .saturating_sub(state.estimated_daily_tokens_used)
                            .saturating_sub(reserved_tokens)
                    });
                    let input_available = state.input_tokens_remaining.map(|remaining| {
                        remaining
                            .saturating_sub(state.estimated_input_tokens_used)
                            .saturating_sub(reserved_input)
                    });
                    let output_available = state.output_tokens_remaining.map(|remaining| {
                        remaining
                            .saturating_sub(state.estimated_output_tokens_used)
                            .saturating_sub(reserved_output)
                    });
                    let daily_input_available = state.daily_input_remaining.map(|remaining| {
                        remaining
                            .saturating_sub(state.estimated_daily_input_tokens_used)
                            .saturating_sub(reserved_input)
                    });
                    let daily_output_available = state.daily_output_remaining.map(|remaining| {
                        remaining
                            .saturating_sub(state.estimated_daily_output_tokens_used)
                            .saturating_sub(reserved_output)
                    });
                    let available_requests = state.requests_remaining.map(|remaining| {
                        remaining
                            .saturating_sub(state.estimated_requests_used)
                            .saturating_sub(reserved_requests)
                    });
                    let daily_requests_available =
                        state.daily_requests_remaining.map(|remaining| {
                            remaining
                                .saturating_sub(state.estimated_daily_requests_used)
                                .saturating_sub(reserved_requests)
                        });
                    let tokens_fit = available_tokens
                        .is_none_or(|remaining| remaining >= estimated_tokens)
                        && daily_tokens_available
                            .is_none_or(|remaining| remaining >= estimated_tokens)
                        && input_available
                            .is_none_or(|remaining| remaining >= estimated_input_tokens)
                        && output_available
                            .is_none_or(|remaining| remaining >= estimated_output_tokens)
                        && daily_input_available
                            .is_none_or(|remaining| remaining >= estimated_input_tokens)
                        && daily_output_available
                            .is_none_or(|remaining| remaining >= estimated_output_tokens);
                    let requests_fit = available_requests.is_none_or(|remaining| remaining >= 1)
                        && daily_requests_available.is_none_or(|remaining| remaining >= 1);
                    if tokens_fit
                        && requests_fit
                        && state.quota_waiters.front().copied() == Some(ticket)
                    {
                        state.next_reservation = state.next_reservation.wrapping_add(1).max(1);
                        let reservation_id = state.next_reservation;
                        state.quota_waiters.pop_front();
                        state.queued = state.queued.saturating_sub(1);
                        state.reservations.insert(
                            reservation_id,
                            Reservation {
                                combined_tokens,
                                input_tokens: estimated_input_tokens,
                                output_tokens: estimated_output_tokens,
                                requests: 1,
                                expires_at: now + Duration::from_secs(900),
                            },
                        );
                        queue_ticket.disarm();
                        return Ok(QuotaPermit {
                            state: self.state.clone(),
                            reservation_id,
                        });
                    }
                    let provider_reset = [
                        state.reset_at,
                        state.requests_reset_at,
                        state.input_reset_at,
                        state.output_reset_at,
                        state.daily_reset_at,
                        state.daily_input_reset_at,
                        state.daily_output_reset_at,
                        state.daily_requests_reset_at,
                    ]
                    .into_iter()
                    .flatten()
                    .min();
                    let reservation_expiry = state
                        .reservations
                        .values()
                        .map(|reservation| reservation.expires_at)
                        .min();
                    match (provider_reset, reservation_expiry) {
                        (Some(reset), Some(expiry)) => Some(reset.min(expiry)),
                        (Some(reset), None) => Some(reset),
                        (None, Some(expiry)) => Some(expiry),
                        (None, None) => None,
                    }
                };
                let Some(wait_until) = wait_until else {
                    return Err(LlmError::QuotaExhausted(
                        "observed provider capacity is exhausted and no reset time was published"
                            .into(),
                    ));
                };
                tokio::select! {
                    _ = cancel.cancelled() => return Err(LlmError::Aborted { partial: None }),
                    _ = tokio::time::sleep_until(wait_until) => {},
                    _ = &mut notified => {},
                }
            }
        }
        .await;
        result
    }

    pub fn observe(&self, observation: CapacityObservation) {
        let serial = self
            .state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .observation_serial
            .get_or_init(|| Arc::new(Mutex::new(())))
            .clone();
        let _serial = serial
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        self.observe_unserialized(observation);
    }

    pub(crate) fn observe_ordered(&self, sequence: u64, observation: CapacityObservation) {
        if observation.is_empty() {
            return;
        }
        let serial = self
            .state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .observation_serial
            .get_or_init(|| Arc::new(Mutex::new(())))
            .clone();
        let _serial = serial
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        {
            let mut state = self
                .state
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            if sequence < state.last_observation_sequence {
                return;
            }
            state.last_observation_sequence = sequence;
        }
        self.observe_unserialized(observation);
    }

    fn observe_unserialized(&self, observation: CapacityObservation) {
        if observation.is_empty() {
            return;
        }
        let mut state = self
            .state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        // Provider remaining values already include requests that completed
        // before this response. Local estimates only account for dispatches
        // settled since that sample, so start a fresh local delta here.
        if observation.tokens_remaining.is_some() {
            state.estimated_tokens_used = 0;
        }
        if observation.input_tokens_remaining.is_some() {
            state.estimated_input_tokens_used = 0;
        }
        if observation.output_tokens_remaining.is_some() {
            state.estimated_output_tokens_used = 0;
        }
        if observation.daily_remaining.is_some() {
            state.estimated_daily_tokens_used = 0;
        }
        if observation.daily_input_remaining.is_some() {
            state.estimated_daily_input_tokens_used = 0;
        }
        if observation.daily_output_remaining.is_some() {
            state.estimated_daily_output_tokens_used = 0;
        }
        if observation.requests_remaining.is_some() {
            state.estimated_requests_used = 0;
        }
        state.tokens_remaining = observation.tokens_remaining.or(state.tokens_remaining);
        state.tokens_limit = observation.tokens_limit.or(state.tokens_limit);
        state.input_tokens_remaining = observation
            .input_tokens_remaining
            .or(state.input_tokens_remaining);
        state.input_tokens_limit = observation.input_tokens_limit.or(state.input_tokens_limit);
        state.output_tokens_remaining = observation
            .output_tokens_remaining
            .or(state.output_tokens_remaining);
        state.output_tokens_limit = observation
            .output_tokens_limit
            .or(state.output_tokens_limit);
        state.requests_remaining = observation.requests_remaining.or(state.requests_remaining);
        state.requests_limit = observation.requests_limit.or(state.requests_limit);
        state.reset_after = observation
            .reset_after_secs
            .map(Duration::from_secs)
            .or(state.reset_after);
        if let Some(delay) = observation.reset_after_secs {
            state.reset_at = tokio::time::Instant::now().checked_add(Duration::from_secs(delay));
        }
        if let Some(delay) = observation.requests_reset_after_secs {
            state.requests_reset_at =
                tokio::time::Instant::now().checked_add(Duration::from_secs(delay));
        }
        if let Some(delay) = observation.input_tokens_reset_after_secs {
            state.input_reset_at =
                tokio::time::Instant::now().checked_add(Duration::from_secs(delay));
        }
        if let Some(delay) = observation.output_tokens_reset_after_secs {
            state.output_reset_at =
                tokio::time::Instant::now().checked_add(Duration::from_secs(delay));
        }
        state.daily_remaining = observation.daily_remaining.or(state.daily_remaining);
        state.daily_limit = observation.daily_limit.or(state.daily_limit);
        state.daily_input_remaining = observation
            .daily_input_remaining
            .or(state.daily_input_remaining);
        state.daily_input_limit = observation.daily_input_limit.or(state.daily_input_limit);
        state.daily_output_remaining = observation
            .daily_output_remaining
            .or(state.daily_output_remaining);
        state.daily_output_limit = observation.daily_output_limit.or(state.daily_output_limit);
        state.daily_requests_remaining = observation
            .daily_requests_remaining
            .or(state.daily_requests_remaining);
        state.daily_requests_limit = observation
            .daily_requests_limit
            .or(state.daily_requests_limit);
        if observation.daily_requests_remaining.is_some() {
            state.estimated_daily_requests_used = 0;
        }
        if observation.daily_remaining.is_some()
            || observation.daily_limit.is_some()
            || observation.daily_input_remaining.is_some()
            || observation.daily_input_limit.is_some()
            || observation.daily_output_remaining.is_some()
            || observation.daily_output_limit.is_some()
            || observation.daily_input_reset_after_secs.is_some()
            || observation.daily_output_reset_after_secs.is_some()
            || observation.daily_requests_remaining.is_some()
            || observation.daily_requests_limit.is_some()
            || observation.daily_requests_reset_after_secs.is_some()
            || observation.daily_reset_after_secs.is_some()
        {
            state.daily_observed_at = Some(tokio::time::Instant::now());
        }
        if let Some(delay) = observation.daily_reset_after_secs {
            state.daily_reset_at =
                tokio::time::Instant::now().checked_add(Duration::from_secs(delay));
        }
        if let Some(delay) = observation.daily_input_reset_after_secs {
            state.daily_input_reset_at =
                tokio::time::Instant::now().checked_add(Duration::from_secs(delay));
        }
        if let Some(delay) = observation.daily_output_reset_after_secs {
            state.daily_output_reset_at =
                tokio::time::Instant::now().checked_add(Duration::from_secs(delay));
        }
        if let Some(delay) = observation.daily_requests_reset_after_secs {
            state.daily_requests_reset_at =
                tokio::time::Instant::now().checked_add(Duration::from_secs(delay));
        }
        state.last_observed_at = Some(tokio::time::Instant::now());
        drop(state);
        self.state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .capacity_changed
            .get_or_init(|| Arc::new(tokio::sync::Notify::new()))
            .notify_waiters();
    }

    /// Whether this identity has a fresh provider-published quota observation
    /// that can constrain an account-level reservation.
    pub fn has_capacity_observation(&self) -> bool {
        let state = self
            .state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        state.last_observed_at.is_some_and(|observed| {
            tokio::time::Instant::now().duration_since(observed) <= Duration::from_secs(300)
        }) && (state.tokens_remaining.is_some()
            || state.input_tokens_remaining.is_some()
            || state.output_tokens_remaining.is_some()
            || state.requests_remaining.is_some()
            || state.daily_remaining.is_some()
            || state.daily_input_remaining.is_some()
            || state.daily_output_remaining.is_some()
            || state.daily_requests_remaining.is_some())
    }

    /// Fresh account-wide token quota evidence (for example OpenAI's
    /// project-token headers), as distinct from account daily-request caps.
    pub fn has_account_token_observation(&self) -> bool {
        let state = self
            .state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        state.last_observed_at.is_some_and(|observed| {
            tokio::time::Instant::now().duration_since(observed) <= Duration::from_secs(300)
        }) && (state.tokens_remaining.is_some()
            || state.tokens_limit.is_some()
            || state.input_tokens_remaining.is_some()
            || state.input_tokens_limit.is_some()
            || state.output_tokens_remaining.is_some()
            || state.output_tokens_limit.is_some())
    }

    /// Safe aggregate: account identities and model names are deliberately omitted.
    pub fn traffic_snapshot() -> TrafficSnapshot {
        let gates = Self::registry();
        let states: Vec<Arc<Mutex<State>>> = {
            let gates = gates
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            gates.values().cloned().collect()
        };
        let mut snap = TrafficSnapshot {
            state: "unknown",
            active: 0,
            queued: 0,
            retry_after_secs: None,
            observed_routes: 0,
            scope: "this service process",
        };
        let now = tokio::time::Instant::now();
        let mut quota_limited = false;
        for shared in states {
            let s = shared
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            if !s.model_scoped {
                snap.active += s.active;
            }
            snap.queued += s.queued;
            if let Some(until) = s.until.filter(|u| *u > now) {
                let seconds = (until - now).as_secs().saturating_add(1);
                snap.retry_after_secs = Some(snap.retry_after_secs.unwrap_or(0).max(seconds));
            }
            quota_limited |= has_fresh_exhausted_capacity(&s, now);
            if s.last_observed_at
                .is_some_and(|t| now.duration_since(t) < Duration::from_secs(300))
            {
                if s.model_scoped {
                    snap.observed_routes += 1;
                }
            }
        }
        snap.state = if snap.retry_after_secs.is_some() || quota_limited {
            "limited"
        } else if snap.queued > 0 {
            "queued"
        } else if snap.active > 0 {
            "busy"
        } else if snap.observed_routes > 0 {
            "normal"
        } else {
            "unknown"
        };
        snap
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
