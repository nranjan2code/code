//! Verdicts on a proposed call, and what a failed call means for the turn.

use std::collections::HashMap;

use vak_control::{CallVerdict, Fault, Refusal};

/// What the permission layer said of a call.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Permission {
    Allowed,
    Denied,
    /// A person, or a standing grant, must answer.
    Ask,
}

/// What the runtime observed about a proposed call before running it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CallFacts {
    /// The name is a tool this turn offers.
    pub known: bool,
    /// The arguments match the tool's schema.
    pub fits_schema: bool,
    /// The tool refuses these arguments by its own rule.
    pub tool_refuses: bool,
    /// The capability was revoked during the turn.
    pub revoked: bool,
    /// The same call was already made in this run and is not made again.
    pub repeated: bool,
    pub permission: Permission,
}

/// The verdict the facts settle, or `None` when only an approver can.
/// The order is the order the facts become available: nothing is asked of
/// a person about a call that could not run.
pub fn admit(facts: &CallFacts) -> Option<CallVerdict> {
    let refused = |by| Some(CallVerdict::Refused(by));
    if !facts.known {
        return refused(Refusal::Unknown);
    }
    if !facts.fits_schema {
        return refused(Refusal::Schema);
    }
    if facts.tool_refuses {
        return refused(Refusal::Tool);
    }
    if facts.revoked {
        return refused(Refusal::Revoked);
    }
    if facts.repeated {
        return refused(Refusal::Repeated);
    }
    match facts.permission {
        Permission::Allowed => Some(CallVerdict::Accepted),
        Permission::Denied => refused(Refusal::Policy),
        Permission::Ask => None,
    }
}

/// How a gate put to an approver ended.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Asked {
    /// A rule or a standing grant covered it; nobody was asked.
    Covered,
    Approved,
    Declined,
    /// Nobody could be asked.
    Unanswerable,
}

/// The verdict on a call whose permission was `Ask`. A capability revoked
/// while the gate was open is refused whatever the answer.
pub fn asked(answer: Asked, revoked_since: bool) -> CallVerdict {
    match answer {
        Asked::Declined => CallVerdict::Refused(Refusal::Person),
        Asked::Unanswerable => CallVerdict::Refused(Refusal::Unattended),
        Asked::Covered | Asked::Approved if revoked_since => CallVerdict::Refused(Refusal::Revoked),
        Asked::Covered | Asked::Approved => CallVerdict::Accepted,
    }
}

/// Where the fault of a refused call lies.
pub fn fault_of(refusal: Refusal) -> Fault {
    match refusal {
        Refusal::Unknown | Refusal::Schema => Fault::Shape,
        Refusal::Tool => Fault::Target,
        Refusal::Revoked
        | Refusal::Repeated
        | Refusal::Policy
        | Refusal::Person
        | Refusal::Unattended => Fault::Refused,
    }
}

/// Whether the tool's schema would help the next attempt: only a call whose
/// shape was wrong is repaired by showing what shape is taken.
pub fn repairable(fault: Fault) -> bool {
    fault == Fault::Shape
}

/// Whether a failed call is something the turn's answer must account for.
/// A call whose shape was wrong never ran and changed nothing, and a
/// cancelled one was stopped by the person; neither is a failure of the
/// work.
pub fn counts(fault: Fault) -> bool {
    !matches!(fault, Fault::Shape | Fault::Cancelled)
}

/// The failed calls of one turn that nothing later repaired. A failure is
/// repaired by a later successful call of the same tool, never by the
/// answer's wording.
#[derive(Debug, Default, Clone)]
pub struct Failures {
    names: HashMap<String, String>,
    unrecovered: HashMap<String, String>,
}

impl Failures {
    pub fn call(&mut self, call_id: &str, tool: &str) {
        self.names.insert(call_id.into(), tool.into());
    }

    pub fn failed(&mut self, call_id: &str, fault: Fault) {
        if counts(fault)
            && let Some(tool) = self.names.get(call_id)
        {
            self.unrecovered.insert(call_id.into(), tool.clone());
        }
    }

    pub fn succeeded(&mut self, call_id: &str) {
        if let Some(tool) = self.names.get(call_id) {
            self.unrecovered.retain(|_, failed| failed != tool);
        }
    }

    pub fn has_unrecovered(&self) -> bool {
        !self.unrecovered.is_empty()
    }

    pub fn clear(&mut self) {
        self.names.clear();
        self.unrecovered.clear();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn facts() -> CallFacts {
        CallFacts {
            known: true,
            fits_schema: true,
            tool_refuses: false,
            revoked: false,
            repeated: false,
            permission: Permission::Allowed,
        }
    }

    #[test]
    fn a_call_is_refused_by_the_first_fact_that_stops_it() {
        assert_eq!(admit(&facts()), Some(CallVerdict::Accepted));
        let unknown = CallFacts {
            known: false,
            fits_schema: false,
            permission: Permission::Ask,
            ..facts()
        };
        assert_eq!(
            admit(&unknown),
            Some(CallVerdict::Refused(Refusal::Unknown))
        );
        let misfit = CallFacts {
            fits_schema: false,
            permission: Permission::Denied,
            ..facts()
        };
        assert_eq!(admit(&misfit), Some(CallVerdict::Refused(Refusal::Schema)));
        let repeated = CallFacts {
            repeated: true,
            ..facts()
        };
        assert_eq!(
            admit(&repeated),
            Some(CallVerdict::Refused(Refusal::Repeated))
        );
    }

    #[test]
    fn only_an_open_gate_waits_for_an_approver() {
        let gated = CallFacts {
            permission: Permission::Ask,
            ..facts()
        };
        assert_eq!(admit(&gated), None);
        assert_eq!(asked(Asked::Approved, false), CallVerdict::Accepted);
        assert_eq!(
            asked(Asked::Approved, true),
            CallVerdict::Refused(Refusal::Revoked)
        );
        assert_eq!(
            asked(Asked::Declined, true),
            CallVerdict::Refused(Refusal::Person)
        );
        assert_eq!(
            asked(Asked::Unanswerable, false),
            CallVerdict::Refused(Refusal::Unattended)
        );
    }

    #[test]
    fn a_failure_is_cleared_only_by_the_same_tool_succeeding() {
        let mut failures = Failures::default();
        failures.call("a", "read");
        failures.call("b", "grep");
        failures.call("c", "read");
        failures.failed("a", Fault::Target);
        failures.succeeded("b");
        assert!(failures.has_unrecovered());
        failures.succeeded("c");
        assert!(!failures.has_unrecovered());
    }

    #[test]
    fn a_call_that_never_ran_or_was_cancelled_is_not_a_failure_of_the_work() {
        let mut failures = Failures::default();
        failures.call("a", "plan");
        failures.failed("a", Fault::Shape);
        failures.failed("a", Fault::Cancelled);
        assert!(!failures.has_unrecovered());
        failures.failed("a", Fault::Refused);
        assert!(failures.has_unrecovered());
        assert!(repairable(Fault::Shape) && !repairable(Fault::Tool));
        assert_eq!(fault_of(Refusal::Unknown), Fault::Shape);
        assert_eq!(fault_of(Refusal::Person), Fault::Refused);
    }
}
