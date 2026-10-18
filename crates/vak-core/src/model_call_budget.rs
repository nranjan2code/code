//! Durable request-wide provider-dispatch accounting, keyed by the admitted
//! root RunId and shared by every Agent and flow node in that request.

use crate::Core;
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use vak_session::ids::RunId;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct BudgetDocument {
    schema: u32,
    run: RunId,
    limit: u32,
    used: u32,
}

impl Core {
    pub(crate) fn model_call_budget(
        &self,
        run: RunId,
        requested_limit: u32,
    ) -> Result<Arc<vak_agent::ModelCallBudget>, String> {
        let path = self
            .shared_scope()
            .model_call_budgets()
            .join(run.to_string());
        let mut resolved = None;
        let mut init_failure = None;
        let initialized = vak_session::documents::update(&path, |current| {
            let is_new = current.is_none();
            let mut document = match current {
                Some(raw) => match serde_json::from_str::<BudgetDocument>(raw) {
                    Ok(document) if document.schema == 1 && document.run == run => document,
                    Ok(_) => {
                        init_failure = Some("model-call budget document identity mismatch".into());
                        return Ok(None);
                    }
                    Err(error) => {
                        init_failure = Some(format!("invalid model-call budget document: {error}"));
                        return Ok(None);
                    }
                },
                None => BudgetDocument {
                    schema: 1,
                    run,
                    limit: requested_limit,
                    used: 0,
                },
            };
            let prior_limit = document.limit;
            document.limit = document.limit.min(requested_limit);
            resolved = Some(document.clone());
            if !is_new && document.limit == prior_limit {
                return Ok(None);
            }
            let raw = serde_json::to_string(&document).map_err(|error| error.to_string())?;
            Ok(Some((raw, document)))
        })?;
        let document = initialized.or(resolved).ok_or_else(|| {
            init_failure.unwrap_or_else(|| "model-call budget initialization refused".into())
        })?;
        let limit = document.limit;
        let used = document.used;
        let claim_path = path.clone();
        let claim_run = run;
        let claim = Arc::new(move |live_limit: u32| -> Result<u32, String> {
            let mut failure = None;
            let saved = vak_session::documents::update(&claim_path, |current| {
                let document = match current {
                    Some(raw) => match serde_json::from_str::<BudgetDocument>(raw) {
                        Ok(document) if document.schema == 1 && document.run == claim_run => {
                            document
                        }
                        Ok(_) => {
                            failure =
                                Some("model-call budget document identity mismatch".to_string());
                            return Ok(None);
                        }
                        Err(error) => {
                            failure = Some(format!("invalid model-call budget document: {error}"));
                            return Ok(None);
                        }
                    },
                    None => {
                        failure = Some("model-call budget document is missing".into());
                        return Ok(None);
                    }
                };
                let effective_limit = document.limit.min(live_limit);
                let Some(next) = document.used.checked_add(1) else {
                    failure = Some("model-call budget count overflow".into());
                    return Ok(None);
                };
                if document.used >= effective_limit || next > effective_limit {
                    failure = Some("aggregate model-call limit exhausted".into());
                    return Ok(None);
                }
                let next_document = BudgetDocument {
                    used: next,
                    limit: effective_limit,
                    ..document
                };
                let raw =
                    serde_json::to_string(&next_document).map_err(|error| error.to_string())?;
                Ok(Some((raw, next)))
            })?;
            saved.ok_or_else(|| failure.unwrap_or_else(|| "model-call budget claim refused".into()))
        });
        Ok(Arc::new(vak_agent::ModelCallBudget::with_durable_claim(
            run, limit, used, claim,
        )))
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;

    #[test]
    fn claims_survive_budget_recreation_and_coordinate_independent_handles() {
        let dir = tempfile::tempdir().unwrap();
        let core = Core::new_with_trust(dir.path().to_path_buf(), true).unwrap();
        let run = RunId::new();
        let first = core.model_call_budget(run, 12).unwrap();
        let second = core.model_call_budget(run, 12).unwrap();
        let mut workers = Vec::new();
        for index in 0..24 {
            let budget = if index % 2 == 0 {
                first.clone()
            } else {
                second.clone()
            };
            workers.push(std::thread::spawn(move || budget.claim().is_ok()));
        }
        let claimed = workers
            .into_iter()
            .map(|worker| worker.join().unwrap())
            .filter(|claimed| *claimed)
            .count();
        assert_eq!(claimed, 12);

        let resumed = core.model_call_budget(run, 20).unwrap();
        assert_eq!(
            resumed.limit(),
            12,
            "a resumed RunId cannot raise its recorded cap"
        );
        assert_eq!(resumed.used(), 12);
        assert!(resumed.claim().is_err());
    }

    #[test]
    fn missing_or_corrupt_document_never_resets_a_live_budget() {
        let dir = tempfile::tempdir().unwrap();
        let core = Core::new_with_trust(dir.path().to_path_buf(), true).unwrap();
        let run = RunId::new();
        let budget = core.model_call_budget(run, 10).unwrap();
        assert_eq!(budget.claim().unwrap(), 1);
        let path = core
            .shared_scope()
            .model_call_budgets()
            .join(run.to_string());
        vak_session::documents::forget(&path).unwrap();
        assert!(budget.claim().is_err());
        assert!(vak_session::documents::read(&path).unwrap().is_none());
        let corrupt_run = RunId::new();
        let corrupt_budget = core.model_call_budget(corrupt_run, 10).unwrap();
        let corrupt_path = core
            .shared_scope()
            .model_call_budgets()
            .join(corrupt_run.to_string());
        vak_session::documents::update(&corrupt_path, |_| Ok(Some(("invalid".into(), ()))))
            .unwrap();
        assert!(corrupt_budget.claim().is_err());
        assert!(core.model_call_budget(corrupt_run, 10).is_err());
    }

    #[test]
    fn lower_and_zero_limits_remain_lower_after_recreation() {
        let dir = tempfile::tempdir().unwrap();
        let core = Core::new_with_trust(dir.path().to_path_buf(), true).unwrap();
        let run = RunId::new();
        let initial = core.model_call_budget(run, 10).unwrap();
        assert_eq!(initial.claim().unwrap(), 1);
        let narrowed = core.model_call_budget(run, 5).unwrap();
        assert_eq!(narrowed.limit(), 5);
        assert_eq!(narrowed.claim().unwrap(), 2);
        let raised = core.model_call_budget(run, 100).unwrap();
        assert_eq!(raised.limit(), 5);
        assert_eq!(raised.used(), 2);

        let zero_run = RunId::new();
        let zero = core.model_call_budget(zero_run, 0).unwrap();
        assert!(zero.claim().is_err());
        let resumed_zero = core.model_call_budget(zero_run, 100).unwrap();
        assert_eq!(resumed_zero.limit(), 0);
        assert!(resumed_zero.claim().is_err());

        let spent_run = RunId::new();
        let spend = core.model_call_budget(spent_run, 10).unwrap();
        for _ in 0..3 {
            spend.claim().unwrap();
        }
        let lowered_below_usage = core.model_call_budget(spent_run, 2).unwrap();
        assert_eq!(lowered_below_usage.limit(), 2);
        assert!(lowered_below_usage.claim().is_err());
        let raised_after_lowering = core.model_call_budget(spent_run, 100).unwrap();
        assert_eq!(raised_after_lowering.limit(), 2);
        assert_eq!(raised_after_lowering.used(), 3);
        assert!(raised_after_lowering.claim().is_err());
    }
}
