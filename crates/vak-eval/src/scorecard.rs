//! The scorecard types the deterministic gates report through: a named
//! metric with a threshold and a direction, and the card that prints and
//! passes/fails them. The context-engine gate itself lives in
//! `context_engine_gate` (docs/design/68-context-engine.md "Verification").

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MetricDirection {
    Min,
    Max,
}

#[derive(Debug, Clone)]
pub struct QualityMetric {
    pub name: &'static str,
    pub value: f64,
    pub threshold: f64,
    pub direction: MetricDirection,
}

impl QualityMetric {
    fn passed(&self) -> bool {
        match self.direction {
            MetricDirection::Min => self.value >= self.threshold,
            MetricDirection::Max => self.value <= self.threshold,
        }
    }
}

#[derive(Debug, Clone)]
pub struct ContextScorecard {
    pub metrics: Vec<QualityMetric>,
}

impl ContextScorecard {
    pub fn passed(&self) -> bool {
        self.metrics.iter().all(|m| m.passed())
    }
}

impl std::fmt::Display for ContextScorecard {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        writeln!(f, "Context quality scorecard:")?;
        for m in &self.metrics {
            let mark = if m.passed() { "PASS" } else { "FAIL" };
            let op = match m.direction {
                MetricDirection::Min => "≥",
                MetricDirection::Max => "≤",
            };
            writeln!(
                f,
                "  [{mark}] {} = {:.3} ({} {})",
                m.name, m.value, op, m.threshold
            )?;
        }
        Ok(())
    }
}
