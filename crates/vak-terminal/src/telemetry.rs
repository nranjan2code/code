//! Operational telemetry state: CPU/Memory gauges, token burn-rate waveform,
//! agent swarm radar, and provider latency statistics.

#[derive(Debug, Clone)]
pub struct TelemetryState {
    pub cpu_percent: f32,
    pub rss_mb: f64,
    pub token_rate_history: Vec<u32>,
    pub max_rate: u32,
    pub active_subagents: usize,
    pub pending_tasks: usize,
    pub radar_angle_deg: f32,
    pub spend_today_usd: f64,
    pub spend_budget_cap_usd: f64,
    pub circuit_breaker_healthy: bool,
    pub active_services_count: (usize, usize), // (up, total)
    pub bus_queue_depth: usize,
    pub bus_dlq_count: usize,
    pub anthropic_latency_ms: u32,
    pub ollama_latency_ms: u32,
    pub tick_count: usize,
}

impl Default for TelemetryState {
    fn default() -> Self {
        Self {
            cpu_percent: 18.4,
            rss_mb: 142.5,
            token_rate_history: vec![
                42, 65, 88, 110, 142, 130, 95, 112, 125, 145, 160, 135, 120, 142,
            ],
            max_rate: 200,
            active_subagents: 3,
            pending_tasks: 12,
            radar_angle_deg: 45.0,
            spend_today_usd: 0.142,
            spend_budget_cap_usd: 5.0,
            circuit_breaker_healthy: true,
            active_services_count: (3, 3),
            bus_queue_depth: 0,
            bus_dlq_count: 0,
            anthropic_latency_ms: 24,
            ollama_latency_ms: 12,
            tick_count: 0,
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
        // Rotate radar sweep
        self.radar_angle_deg = (self.radar_angle_deg + 6.0) % 360.0;
    }

    pub fn push_token_rate(&mut self, rate: u32) {
        if self.token_rate_history.len() >= 60 {
            self.token_rate_history.remove(0);
        }
        self.token_rate_history.push(rate);
        if rate > self.max_rate {
            self.max_rate = rate;
        }
    }
}
