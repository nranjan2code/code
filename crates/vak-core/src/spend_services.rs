//! Process-owned budget settings and gate identities. A gate captures its
//! data home and budget session once; changing a Core's scope never redirects
//! an already admitted dispatch or reuses another home's mutable gate.

use crate::{Core, finops};
use std::collections::{BTreeMap, HashMap};
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

#[derive(Default)]
pub(crate) struct SpendServices {
    state: Mutex<State>,
}

#[derive(Default)]
struct State {
    // None follows configuration; Some(None) explicitly clears the cap.
    run_cap: Option<Option<f64>>,
    day_cap: Option<Option<f64>>,
    prices: Option<BTreeMap<String, vak_config::PriceEntry>>,
    // Session ids are scoped to the install whose ledger they name.
    gates: HashMap<(PathBuf, String), Arc<finops::CoreSpendGate>>,
}

impl State {
    fn settings(&self, baseline: &vak_config::FinopsResolved) -> vak_config::FinopsResolved {
        let mut settings = baseline.clone();
        if let Some(cap) = self.run_cap {
            settings.max_run_usd = cap;
        }
        if let Some(cap) = self.day_cap {
            settings.max_day_usd = cap;
        }
        if let Some(prices) = &self.prices {
            settings.price_overrides = prices.clone();
        }
        settings
    }
}

impl Core {
    /// Current budget settings, captured together under the domain owner's
    /// lock so a two-cap update cannot be observed half applied.
    pub fn effective_finops(&self) -> vak_config::FinopsResolved {
        self.inner
            .spend
            .state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .settings(&self.inner.config.finops)
    }

    pub(crate) fn spend_gate_for(&self, session_id: &str) -> Arc<finops::CoreSpendGate> {
        let scope = self.shared_scope();
        let mut state = self
            .inner
            .spend
            .state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let settings = state.settings(&self.inner.config.finops);
        let key = (scope.root().to_path_buf(), session_id.to_owned());
        if let Some(gate) = state.gates.get(&key) {
            gate.refresh_caps(&settings);
            return gate.clone();
        }
        let gate = Arc::new(finops::CoreSpendGate::with_shared_day_budget(
            scope.root(),
            &settings,
            finops::shared_day_budget(scope.root()),
        ));
        gate.bind_budget_session(session_id);
        state.gates.insert(key, gate.clone());
        gate
    }

    /// Evict cached handles for this session in every scope this Core has
    /// used. Durable spend and unresolved liabilities remain in their ledgers.
    pub fn forget_spend_gate(&self, session_id: &str) {
        self.inner
            .spend
            .state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .gates
            .retain(|(_, session), _| session != session_id);
    }

    pub fn effective_finops_max_run_usd(&self) -> Option<f64> {
        self.inner
            .spend
            .state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .run_cap
            .unwrap_or(self.inner.config.finops.max_run_usd)
    }

    pub fn effective_finops_max_day_usd(&self) -> Option<f64> {
        self.inner
            .spend
            .state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .day_cap
            .unwrap_or(self.inner.config.finops.max_day_usd)
    }

    /// Absent arguments preserve their current effective value; Some(None)
    /// explicitly clears that cap. Both supplied caps change atomically.
    pub fn apply_persisted_finops_caps(
        &self,
        max_run_usd: Option<Option<f64>>,
        max_day_usd: Option<Option<f64>>,
    ) {
        let mut state = self
            .inner
            .spend
            .state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if let Some(cap) = max_run_usd {
            state.run_cap = Some(cap);
        }
        if let Some(cap) = max_day_usd {
            state.day_cap = Some(cap);
        }
    }

    pub(crate) fn apply_persisted_finops(&self, settings: &vak_config::FinopsResolved) {
        let mut state = self
            .inner
            .spend
            .state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        state.run_cap = Some(settings.max_run_usd);
        state.day_cap = Some(settings.max_day_usd);
        state.prices = Some(settings.price_overrides.clone());
    }

    pub fn apply_persisted_finops_prices(&self, prices: BTreeMap<String, vak_config::PriceEntry>) {
        self.inner
            .spend
            .state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .prices = Some(prices);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn full_policy_refresh_publishes_prices_and_caps_together() {
        vak_config::paths::isolate_home_for_tests();
        let dir = tempfile::tempdir().unwrap();
        let core = Core::new_with_trust(dir.path().to_path_buf(), true).unwrap();
        let policy = |run| vak_config::FinopsResolved {
            max_run_usd: Some(run),
            max_day_usd: Some(run * 10.0),
            price_overrides: BTreeMap::from([(
                "fixture".into(),
                vak_config::PriceEntry {
                    input: run,
                    output: run,
                },
            )]),
            ..Default::default()
        };
        core.apply_persisted_finops(&policy(1.0));
        let writer = core.clone();
        let task = std::thread::spawn(move || {
            for n in 0..2_000 {
                writer.apply_persisted_finops(&policy(if n % 2 == 0 { 2.0 } else { 1.0 }));
            }
        });
        for _ in 0..2_000 {
            let settings = core.effective_finops();
            assert_eq!(
                settings.max_day_usd,
                settings.max_run_usd.map(|run| run * 10.0)
            );
            assert_eq!(
                settings.max_run_usd,
                Some(settings.price_overrides["fixture"].input)
            );
        }
        task.join().unwrap();
    }

    #[test]
    fn readers_never_observe_half_of_a_two_cap_update() {
        vak_config::paths::isolate_home_for_tests();
        let dir = tempfile::tempdir().unwrap();
        let core = Core::new_with_trust(dir.path().to_path_buf(), true).unwrap();
        core.apply_persisted_finops_caps(Some(Some(1.0)), Some(Some(10.0)));
        let writer = core.clone();
        let task = std::thread::spawn(move || {
            for n in 0..2_000 {
                let run = if n % 2 == 0 { 2.0 } else { 1.0 };
                writer.apply_persisted_finops_caps(Some(Some(run)), Some(Some(run * 10.0)));
            }
        });
        for _ in 0..2_000 {
            let settings = core.effective_finops();
            assert_eq!(
                settings.max_day_usd,
                settings.max_run_usd.map(|run| run * 10.0)
            );
        }
        task.join().unwrap();
    }
}
