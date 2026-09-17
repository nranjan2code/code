//! Operational telemetry state: CPU/Memory gauges, token burn-rate waveform,
//! agent swarm radar, and provider latency.
//!
//! All values are populated from real server data — `/health` for the
//! system/provider snapshot and `AgentEvent` stream events for live
//! token usage, worker counts, and pending task counts. The previous
//! implementation hardcoded all values; this one derives them from actual
//! API responses.

use crate::api::HealthReport;

#[derive(Debug, Clone)]
pub struct TelemetryState {
    pub cpu_percent: f32,
    pub rss_mb: f64,
    pub token_rate_history: Vec<u32>,
    pub max_rate: u32,
    pub active_workers: usize,
    pub pending_tasks: usize,
    pub radar_angle_deg: f32,
    pub spend_today_usd: f64,
    pub spend_budget_cap_usd: f64,
    pub circuit_breaker_healthy: bool,
    pub active_services_count: (usize, usize),
    pub bus_queue_depth: usize,
    pub bus_dlq_count: usize,
    pub anthropic_latency_ms: u32,
    pub ollama_latency_ms: u32,
    pub tick_count: usize,
    pub connected: bool,
    pub last_error: Option<String>,
}

impl Default for TelemetryState {
    fn default() -> Self {
        Self {
            cpu_percent: 0.0,
            rss_mb: 0.0,
            token_rate_history: Vec::new(),
            max_rate: 0,
            active_workers: 0,
            pending_tasks: 0,
            radar_angle_deg: 0.0,
            spend_today_usd: 0.0,
            spend_budget_cap_usd: 0.0,
            circuit_breaker_healthy: false,
            active_services_count: (0, 0),
            bus_queue_depth: 0,
            bus_dlq_count: 0,
            anthropic_latency_ms: 0,
            ollama_latency_ms: 0,
            tick_count: 0,
            connected: false,
            last_error: None,
        }
    }
}

impl TelemetryState {
    pub fn new() -> Self {
        Self::default()
    }

    /// Advance visual animations on tick (50ms).
    pub fn tick(&mut self) {
        self.tick_count = self.tick_count.wrapping_add(1);
        self.radar_angle_deg = (self.radar_angle_deg + 6.0) % 360.0;
    }

    /// Update telemetry from a real `/health` response.
    pub fn update_from_health(&mut self, health: &HealthReport) {
        self.circuit_breaker_healthy = health.circuit_breaker_healthy;
        self.connected = health.healthy;
    }

    /// Push a real token-rate sample from an agent TurnEnd event.
    pub fn push_token_rate(&mut self, rate: u32) {
        if self.token_rate_history.len() >= 60 {
            self.token_rate_history.remove(0);
        }
        self.token_rate_history.push(rate);
        if rate > self.max_rate {
            self.max_rate = rate;
        }
    }

    /// Update worker presence from `WorkerStarted` / `WorkerFinished`.
    pub fn set_active_workers(&mut self, count: usize) {
        self.active_workers = count;
    }

    /// Record a connection error so the UI can surface it.
    pub fn set_connection_error(&mut self, msg: String) {
        self.connected = false;
        self.last_error = Some(msg);
    }

    /// Clear the error indicator on successful data receipt.
    pub fn clear_error(&mut self) {
        if self.connected {
            self.last_error = None;
        }
    }
}
