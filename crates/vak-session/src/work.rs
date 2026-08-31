use std::collections::{BTreeMap, BTreeSet, VecDeque};

use thiserror::Error;

use crate::types::{
    CriterionResult, Entry, EntryPayload, WorkContract, WorkContractStatus, WorkEvent,
    WorkEventKind, WorkItemDefinition, WorkItemState, WorkItemStatus,
};

#[derive(Debug, Clone, PartialEq, serde::Serialize)]
pub struct WorkProjection {
    pub contract: WorkContract,
    pub status: WorkContractStatus,
    pub items: BTreeMap<String, WorkItemState>,
    pub criteria: BTreeMap<String, CriterionResult>,
}

#[derive(Debug, Error, PartialEq, Eq)]
pub enum WorkError {
    #[error("work contract is empty")]
    EmptyContract,
    #[error("work contract has duplicate item '{0}'")]
    DuplicateItem(String),
    #[error("work item '{0}' has unknown dependency '{1}'")]
    UnknownDependency(String, String),
    #[error("work contract contains a dependency cycle")]
    DependencyCycle,
    #[error("work item '{0}' references unknown criterion '{1}'")]
    UnknownCriterion(String, String),
    #[error("work event belongs to contract '{event}', active contract is '{active}'")]
    WrongContract { event: String, active: String },
    #[error("work revision {actual} is not the expected revision {expected}")]
    WrongRevision { actual: u32, expected: u32 },
    #[error("work event requires an active contract")]
    MissingContract,
    #[error("invalid work status transition {from:?} -> {to:?}")]
    InvalidContractTransition {
        from: WorkContractStatus,
        to: WorkContractStatus,
    },
    #[error("invalid work item '{item}' transition {from:?} -> {to:?}")]
    InvalidItemTransition {
        item: String,
        from: WorkItemStatus,
        to: WorkItemStatus,
    },
    #[error("unknown work item '{0}'")]
    UnknownItem(String),
    #[error("work item '{0}' cannot be marked succeeded by a model event")]
    ModelCompletion(String),
    #[error("work event is not valid for the current contract state")]
    InvalidEvent,
}

pub fn validate_contract(contract: &WorkContract) -> Result<(), WorkError> {
    if contract.objective.trim().is_empty() || contract.items.is_empty() {
        return Err(WorkError::EmptyContract);
    }

    let item_ids: BTreeSet<&str> = contract
        .items
        .iter()
        .map(|item| item.item_id.as_str())
        .collect();
    if item_ids.len() != contract.items.len() {
        let mut seen = BTreeSet::new();
        let duplicate = contract
            .items
            .iter()
            .find(|item| !seen.insert(item.item_id.as_str()))
            .map(|item| item.item_id.clone())
            .unwrap_or_default();
        return Err(WorkError::DuplicateItem(duplicate));
    }

    let criterion_ids: BTreeSet<&str> = contract
        .criteria
        .iter()
        .map(|criterion| criterion.criterion_id.as_str())
        .collect();
    for item in &contract.items {
        for dep in &item.dependencies {
            if !item_ids.contains(dep.as_str()) {
                return Err(WorkError::UnknownDependency(
                    item.item_id.clone(),
                    dep.clone(),
                ));
            }
        }
        for criterion in &item.criterion_ids {
            if !criterion_ids.contains(criterion.as_str()) {
                return Err(WorkError::UnknownCriterion(
                    item.item_id.clone(),
                    criterion.clone(),
                ));
            }
        }
    }
    ensure_acyclic(&contract.items)
}

fn ensure_acyclic(items: &[WorkItemDefinition]) -> Result<(), WorkError> {
    let mut indegree: BTreeMap<&str, usize> = items
        .iter()
        .map(|item| (item.item_id.as_str(), item.dependencies.len()))
        .collect();
    let mut dependents: BTreeMap<&str, Vec<&str>> = BTreeMap::new();
    for item in items {
        for dependency in &item.dependencies {
            dependents
                .entry(dependency.as_str())
                .or_default()
                .push(item.item_id.as_str());
        }
    }
    let mut ready: VecDeque<&str> = indegree
        .iter()
        .filter_map(|(id, degree)| (*degree == 0).then_some(*id))
        .collect();
    let mut visited = 0;
    while let Some(id) = ready.pop_front() {
        visited += 1;
        if let Some(children) = dependents.get(id) {
            for child in children {
                let Some(degree) = indegree.get_mut(child) else {
                    return Err(WorkError::InvalidEvent);
                };
                *degree -= 1;
                if *degree == 0 {
                    ready.push_back(child);
                }
            }
        }
    }
    if visited != items.len() {
        return Err(WorkError::DependencyCycle);
    }
    Ok(())
}

pub fn project_work(entries: &[&Entry]) -> Result<Option<WorkProjection>, WorkError> {
    let mut projection = None;
    for entry in entries {
        let EntryPayload::Work(event) = &entry.payload else {
            continue;
        };
        apply_event(&mut projection, event)?;
    }
    Ok(projection)
}

fn apply_event(
    projection: &mut Option<WorkProjection>,
    event: &WorkEvent,
) -> Result<(), WorkError> {
    if let WorkEventKind::ContractCreated { contract } = &event.kind {
        if projection.is_some() {
            return Err(WorkError::InvalidEvent);
        }
        validate_contract(contract)?;
        if event.contract_id != contract.contract_id || event.revision != contract.revision {
            return Err(WorkError::WrongRevision {
                actual: event.revision,
                expected: contract.revision,
            });
        }
        let items = contract
            .items
            .iter()
            .map(|item| {
                (
                    item.item_id.clone(),
                    WorkItemState {
                        item_id: item.item_id.clone(),
                        owner: item.owner.clone(),
                        status: WorkItemStatus::Proposed,
                        attempt: 0,
                        child_session_id: None,
                        blocker: None,
                        evidence: Vec::new(),
                    },
                )
            })
            .collect();
        *projection = Some(WorkProjection {
            contract: contract.clone(),
            status: WorkContractStatus::Draft,
            items,
            criteria: BTreeMap::new(),
        });
        return Ok(());
    }

    let Some(current) = projection.as_mut() else {
        return Err(WorkError::MissingContract);
    };
    if event.contract_id != current.contract.contract_id {
        return Err(WorkError::WrongContract {
            event: event.contract_id.clone(),
            active: current.contract.contract_id.clone(),
        });
    }
    let is_revision = matches!(event.kind, WorkEventKind::ContractRevised { .. });
    let expected_revision = if is_revision {
        current.contract.revision + 1
    } else {
        current.contract.revision
    };
    if event.revision != expected_revision {
        return Err(WorkError::WrongRevision {
            actual: event.revision,
            expected: expected_revision,
        });
    }

    match &event.kind {
        WorkEventKind::ContractCreated { .. } => unreachable!(),
        WorkEventKind::ContractRevised {
            previous_revision,
            contract,
            ..
        } => {
            if *previous_revision != current.contract.revision
                || contract.revision != current.contract.revision + 1
            {
                return Err(WorkError::WrongRevision {
                    actual: contract.revision,
                    expected: current.contract.revision + 1,
                });
            }
            validate_contract(contract)?;
            current.contract = contract.clone();
            current.items = contract
                .items
                .iter()
                .map(|item| {
                    (
                        item.item_id.clone(),
                        WorkItemState {
                            item_id: item.item_id.clone(),
                            owner: item.owner.clone(),
                            status: WorkItemStatus::Proposed,
                            attempt: 0,
                            child_session_id: None,
                            blocker: None,
                            evidence: Vec::new(),
                        },
                    )
                })
                .collect();
        }
        WorkEventKind::ContractStatusChanged { from, to, .. } => {
            if current.status != *from || !valid_contract_transition(from, to) {
                return Err(WorkError::InvalidContractTransition {
                    from: current.status.clone(),
                    to: to.clone(),
                });
            }
            current.status = to.clone();
        }
        WorkEventKind::ItemStatusChanged {
            item_id,
            from,
            to,
            attempt,
            reason,
        } => {
            let current_status = current
                .items
                .get(item_id)
                .ok_or_else(|| WorkError::UnknownItem(item_id.clone()))?
                .status
                .clone();
            if current_status != *from {
                return Err(WorkError::InvalidItemTransition {
                    item: item_id.clone(),
                    from: current_status,
                    to: to.clone(),
                });
            }
            if matches!(to, WorkItemStatus::Succeeded) {
                return Err(WorkError::ModelCompletion(item_id.clone()));
            }
            if !valid_item_transition(from, to) {
                return Err(WorkError::InvalidItemTransition {
                    item: item_id.clone(),
                    from: from.clone(),
                    to: to.clone(),
                });
            }
            if *to == WorkItemStatus::Running {
                let definition = current
                    .contract
                    .items
                    .iter()
                    .find(|item| item.item_id == *item_id)
                    .ok_or_else(|| WorkError::UnknownItem(item_id.clone()))?;
                if definition.dependencies.iter().any(|dependency| {
                    !current
                        .items
                        .get(dependency)
                        .is_some_and(|dependency_state| {
                            matches!(
                                dependency_state.status,
                                WorkItemStatus::Succeeded | WorkItemStatus::Skipped
                            )
                        })
                }) {
                    return Err(WorkError::InvalidEvent);
                }
            }
            if *to == WorkItemStatus::ReadyForVerification
                && current.items[item_id].evidence.is_empty()
            {
                return Err(WorkError::InvalidEvent);
            }
            let state = current
                .items
                .get_mut(item_id)
                .ok_or_else(|| WorkError::UnknownItem(item_id.clone()))?;
            state.status = to.clone();
            state.attempt = *attempt;
            state.blocker = matches!(to, WorkItemStatus::Blocked | WorkItemStatus::Interrupted)
                .then_some(reason.clone())
                .filter(|reason| !reason.is_empty());
        }
        WorkEventKind::ItemVerified { item_id, attempt } => {
            let state = current
                .items
                .get_mut(item_id)
                .ok_or_else(|| WorkError::UnknownItem(item_id.clone()))?;
            if state.status != WorkItemStatus::ReadyForVerification || state.attempt != *attempt {
                return Err(WorkError::InvalidItemTransition {
                    item: item_id.clone(),
                    from: state.status.clone(),
                    to: WorkItemStatus::Succeeded,
                });
            }
            let definition = current
                .contract
                .items
                .iter()
                .find(|item| item.item_id == *item_id)
                .ok_or_else(|| WorkError::UnknownItem(item_id.clone()))?;
            if definition.criterion_ids.iter().any(|criterion_id| {
                let required = current
                    .contract
                    .criteria
                    .iter()
                    .find(|criterion| criterion.criterion_id == *criterion_id)
                    .is_some_and(|criterion| criterion.required);
                required
                    && !matches!(
                        current.criteria.get(criterion_id),
                        Some(CriterionResult::Passed { .. })
                    )
            }) {
                return Err(WorkError::InvalidEvent);
            }
            if state.evidence.is_empty() {
                return Err(WorkError::InvalidEvent);
            }
            state.status = WorkItemStatus::Succeeded;
        }
        WorkEventKind::ItemAssigned {
            item_id,
            owner,
            child_session_id,
            ..
        } => {
            let state = current
                .items
                .get_mut(item_id)
                .ok_or_else(|| WorkError::UnknownItem(item_id.clone()))?;
            state.child_session_id = child_session_id.clone();
            state.owner = owner.clone();
        }
        WorkEventKind::EvidenceAttached { item_id, evidence } => {
            let state = current
                .items
                .get_mut(item_id)
                .ok_or_else(|| WorkError::UnknownItem(item_id.clone()))?;
            if !state.evidence.contains(evidence) {
                state.evidence.push(evidence.clone());
            }
        }
        WorkEventKind::AssumptionResolved {
            assumption_id,
            resolution,
        } => {
            let assumption = current
                .contract
                .assumptions
                .iter_mut()
                .find(|assumption| assumption.assumption_id == *assumption_id)
                .ok_or(WorkError::InvalidEvent)?;
            assumption.resolution = Some(resolution.clone());
        }
        WorkEventKind::VerificationRecorded {
            criterion_id,
            result,
        } => {
            if !current
                .contract
                .criteria
                .iter()
                .any(|criterion| criterion.criterion_id == *criterion_id)
            {
                return Err(WorkError::InvalidEvent);
            }
            current
                .criteria
                .insert(criterion_id.clone(), result.clone());
        }
    }
    Ok(())
}

fn valid_contract_transition(from: &WorkContractStatus, to: &WorkContractStatus) -> bool {
    matches!(
        (from, to),
        (WorkContractStatus::Draft, WorkContractStatus::AwaitingInput)
            | (WorkContractStatus::Draft, WorkContractStatus::Active)
            | (
                WorkContractStatus::AwaitingInput,
                WorkContractStatus::Active
            )
            | (WorkContractStatus::Active, WorkContractStatus::Blocked)
            | (WorkContractStatus::Active, WorkContractStatus::Verifying)
            | (WorkContractStatus::Active, WorkContractStatus::Cancelled)
            | (WorkContractStatus::Active, WorkContractStatus::Failed)
            | (WorkContractStatus::Blocked, WorkContractStatus::Active)
            | (WorkContractStatus::Verifying, WorkContractStatus::Active)
            | (WorkContractStatus::Verifying, WorkContractStatus::Completed)
            | (
                WorkContractStatus::Verifying,
                WorkContractStatus::Unverified
            )
    )
}

fn valid_item_transition(from: &WorkItemStatus, to: &WorkItemStatus) -> bool {
    matches!(
        (from, to),
        (WorkItemStatus::Proposed, WorkItemStatus::Ready)
            | (WorkItemStatus::Ready, WorkItemStatus::Running)
            | (WorkItemStatus::Running, WorkItemStatus::Blocked)
            | (
                WorkItemStatus::Running,
                WorkItemStatus::ReadyForVerification
            )
            | (WorkItemStatus::Running, WorkItemStatus::Failed)
            | (WorkItemStatus::Running, WorkItemStatus::Interrupted)
            | (WorkItemStatus::Blocked, WorkItemStatus::Ready)
            | (WorkItemStatus::Failed, WorkItemStatus::Ready)
            | (WorkItemStatus::Interrupted, WorkItemStatus::Ready)
            | (WorkItemStatus::ReadyForVerification, WorkItemStatus::Failed)
            | (WorkItemStatus::Ready, WorkItemStatus::Cancelled)
            | (WorkItemStatus::Running, WorkItemStatus::Cancelled)
    )
}

#[cfg(test)]
mod tests {
    #![allow(clippy::expect_used, clippy::unwrap_used)]

    use super::*;
    use crate::types::{CriterionKind, Entry, EvidenceRef, WorkCriterion, WorkOwner};

    fn contract(items: Vec<WorkItemDefinition>) -> WorkContract {
        WorkContract {
            contract_id: "work-1".into(),
            revision: 0,
            source_entry_id: "user-1".into(),
            objective: "ship the change".into(),
            constraints: Vec::new(),
            assumptions: Vec::new(),
            criteria: vec![WorkCriterion {
                criterion_id: "tests".into(),
                statement: "tests pass".into(),
                kind: CriterionKind::Semantic,
                required: true,
            }],
            items,
        }
    }

    fn item(id: &str, dependencies: Vec<&str>) -> WorkItemDefinition {
        WorkItemDefinition {
            item_id: id.into(),
            title: id.into(),
            instructions: "do the work".into(),
            dependencies: dependencies.into_iter().map(String::from).collect(),
            owner: WorkOwner::ParentAgent,
            required: true,
            readonly: false,
            path_claims: Vec::new(),
            criterion_ids: vec!["tests".into()],
        }
    }

    fn entry(event: WorkEvent) -> Entry {
        Entry::new(None, EntryPayload::Work(event))
    }

    #[test]
    fn rejects_dependency_cycles() {
        let result = validate_contract(&contract(vec![item("a", vec!["b"]), item("b", vec!["a"])]));
        assert_eq!(result, Err(WorkError::DependencyCycle));
    }

    #[test]
    fn projects_lifecycle_and_rejects_model_completion() {
        let c = contract(vec![item("implement", Vec::new())]);
        let mut entries = vec![entry(WorkEvent {
            contract_id: "work-1".into(),
            revision: 0,
            kind: WorkEventKind::ContractCreated { contract: c },
        })];
        entries.push(entry(WorkEvent {
            contract_id: "work-1".into(),
            revision: 0,
            kind: WorkEventKind::ContractStatusChanged {
                from: WorkContractStatus::Draft,
                to: WorkContractStatus::Active,
                reason: "confirmed".into(),
            },
        }));
        entries.push(entry(WorkEvent {
            contract_id: "work-1".into(),
            revision: 0,
            kind: WorkEventKind::ItemStatusChanged {
                item_id: "implement".into(),
                from: WorkItemStatus::Proposed,
                to: WorkItemStatus::Ready,
                attempt: 0,
                reason: String::new(),
            },
        }));
        let refs: Vec<&Entry> = entries.iter().collect();
        let projection = project_work(&refs).unwrap().unwrap();
        assert_eq!(projection.status, WorkContractStatus::Active);
        assert_eq!(projection.items["implement"].status, WorkItemStatus::Ready);

        entries.push(entry(WorkEvent {
            contract_id: "work-1".into(),
            revision: 0,
            kind: WorkEventKind::ItemStatusChanged {
                item_id: "implement".into(),
                from: WorkItemStatus::Ready,
                to: WorkItemStatus::Succeeded,
                attempt: 1,
                reason: "done".into(),
            },
        }));
        let refs: Vec<&Entry> = entries.iter().collect();
        assert!(matches!(
            project_work(&refs),
            Err(WorkError::ModelCompletion(item)) if item == "implement"
        ));
    }

    #[test]
    fn verifier_can_complete_only_with_evidence_and_passed_criteria() {
        let c = contract(vec![item("implement", Vec::new())]);
        let mut entries = vec![entry(WorkEvent {
            contract_id: "work-1".into(),
            revision: 0,
            kind: WorkEventKind::ContractCreated { contract: c },
        })];
        for (from, to) in [
            (WorkItemStatus::Proposed, WorkItemStatus::Ready),
            (WorkItemStatus::Ready, WorkItemStatus::Running),
        ] {
            entries.push(entry(WorkEvent {
                contract_id: "work-1".into(),
                revision: 0,
                kind: WorkEventKind::ItemStatusChanged {
                    item_id: "implement".into(),
                    from,
                    to,
                    attempt: 1,
                    reason: String::new(),
                },
            }));
        }
        entries.push(entry(WorkEvent {
            contract_id: "work-1".into(),
            revision: 0,
            kind: WorkEventKind::EvidenceAttached {
                item_id: "implement".into(),
                evidence: EvidenceRef::LedgerEntry {
                    session_id: "s".into(),
                    entry_id: "result".into(),
                },
            },
        }));
        entries.push(entry(WorkEvent {
            contract_id: "work-1".into(),
            revision: 0,
            kind: WorkEventKind::ItemStatusChanged {
                item_id: "implement".into(),
                from: WorkItemStatus::Running,
                to: WorkItemStatus::ReadyForVerification,
                attempt: 1,
                reason: String::new(),
            },
        }));
        entries.push(entry(WorkEvent {
            contract_id: "work-1".into(),
            revision: 0,
            kind: WorkEventKind::VerificationRecorded {
                criterion_id: "tests".into(),
                result: CriterionResult::Passed {
                    evidence: "result".into(),
                },
            },
        }));
        entries.push(entry(WorkEvent {
            contract_id: "work-1".into(),
            revision: 0,
            kind: WorkEventKind::ItemVerified {
                item_id: "implement".into(),
                attempt: 1,
            },
        }));
        let refs: Vec<&Entry> = entries.iter().collect();
        let projection = project_work(&refs).unwrap().unwrap();
        assert_eq!(
            projection.items["implement"].status,
            WorkItemStatus::Succeeded
        );
    }

    #[test]
    fn assumption_resolution_is_projected_and_durable() {
        let mut c = contract(vec![item("implement", Vec::new())]);
        c.assumptions.push(crate::types::WorkAssumption {
            assumption_id: "scope".into(),
            text: "use the current workspace".into(),
            requires_confirmation: true,
            resolution: None,
        });
        let entries = [
            entry(WorkEvent {
                contract_id: "work-1".into(),
                revision: 0,
                kind: WorkEventKind::ContractCreated { contract: c },
            }),
            entry(WorkEvent {
                contract_id: "work-1".into(),
                revision: 0,
                kind: WorkEventKind::AssumptionResolved {
                    assumption_id: "scope".into(),
                    resolution: "confirmed".into(),
                },
            }),
        ];
        let refs: Vec<&Entry> = entries.iter().collect();
        assert_eq!(
            project_work(&refs).unwrap().unwrap().contract.assumptions[0].resolution,
            Some("confirmed".into())
        );
    }

    #[test]
    fn revision_requires_next_revision_and_valid_contract() {
        let c = contract(vec![item("a", Vec::new())]);
        let replacement = WorkContract {
            revision: 1,
            ..c.clone()
        };
        let entries = [
            entry(WorkEvent {
                contract_id: "work-1".into(),
                revision: 0,
                kind: WorkEventKind::ContractCreated { contract: c },
            }),
            entry(WorkEvent {
                contract_id: "work-1".into(),
                revision: 1,
                kind: WorkEventKind::ContractRevised {
                    previous_revision: 0,
                    contract: replacement,
                    reason: "discovery".into(),
                },
            }),
        ];
        let refs: Vec<&Entry> = entries.iter().collect();
        assert_eq!(project_work(&refs).unwrap().unwrap().contract.revision, 1);
    }
}
